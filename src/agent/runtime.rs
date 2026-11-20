use anyhow::Result;
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::process::{Child, Command};

pub fn secret() -> String {
    format!(
        "{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    )
}

pub struct Runtime {
    child: Option<Child>,
    permission: Option<(Child, Instant, bool)>,
    #[cfg(target_os = "macos")]
    permission_requests: Vec<(Child, Instant)>,
    #[cfg(target_os = "macos")]
    checked_for_start: bool,
    pub permissions: Value,
    pub desktop: Value,
    pub error: Option<String>,
    probe_at: Instant,
    start_at: Instant,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            child: None,
            permission: None,
            #[cfg(target_os = "macos")]
            permission_requests: Vec::new(),
            #[cfg(target_os = "macos")]
            checked_for_start: false,
            permissions: json!({"screen":"unknown","input":"unknown","can_start":false,"requesting":false}),
            desktop: json!({"enabled":false,"ready":false}),
            error: None,
            probe_at: Instant::now(),
            start_at: Instant::now(),
        }
    }

    pub async fn request_permissions(&mut self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let child = Self::permission_command(true)?
                .stdout(Stdio::null())
                .spawn()?;
            self.permission_requests.push((child, Instant::now()));
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some((mut child, _, _)) = self.permission.take() {
                let _ = child.kill().await;
            }
            self.probe(true)?;
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn refresh_permissions(&mut self) -> Result<()> {
        if self.permission.is_none() {
            self.probe(false)?;
        }
        Ok(())
    }

    fn permission_command(request: bool) -> Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        command.arg("permissions");
        if request {
            command.arg("--request");
        }
        command.stderr(Stdio::null()).kill_on_drop(true);
        Ok(command)
    }

    fn probe(&mut self, request: bool) -> Result<()> {
        let child = Self::permission_command(request)?
            .stdout(Stdio::piped())
            .spawn()?;
        self.permission = Some((child, Instant::now(), request));
        self.permissions["requesting"] = json!(request);
        self.probe_at = Instant::now() + Duration::from_secs(5);
        Ok(())
    }

    pub async fn tick(
        &mut self,
        enabled: bool,
        directory: &Path,
        client: &reqwest::Client,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        self.permission_requests.retain_mut(|(child, started)| {
            started.elapsed() < Duration::from_secs(120) && matches!(child.try_wait(), Ok(None))
        });
        if let Some((child, started, request)) = self.permission.as_mut() {
            let done = child.try_wait()?.is_some();
            if done || started.elapsed() > Duration::from_secs(if *request { 120 } else { 8 }) {
                let (mut child, _, _) = self.permission.take().unwrap();
                if !done {
                    let _ = child.kill().await;
                }
                let output = child.wait_with_output().await?;
                self.permissions["requesting"] = json!(false);
                if output.status.success() && output.stdout.len() < 16384 {
                    self.permissions = serde_json::from_slice(&output.stdout)?;
                    self.permissions["requesting"] = json!(false);
                    if cfg!(target_os = "macos") && self.permissions["can_start"] != true {
                        self.desktop["ready"] = json!(false);
                    }
                }
            }
        } else if !cfg!(target_os = "macos") && Instant::now() >= self.probe_at {
            self.probe(false)?;
        }
        #[cfg(target_os = "macos")]
        if enabled && !self.checked_for_start {
            self.refresh_permissions()?;
            self.checked_for_start = true;
        }

        if let Some(child) = &mut self.child
            && child.try_wait()?.is_some()
        {
            self.child = None;
            self.desktop["ready"] = json!(false);
            self.error = Some("desktop_process_stopped".into());
            self.start_at = Instant::now() + Duration::from_secs(15);
        }
        if !enabled {
            self.desktop["enabled"] = json!(false);
            return Ok(());
        }
        self.desktop["enabled"] = json!(true);
        if self.child.is_none()
            && self.permissions["can_start"] == true
            && (!cfg!(target_os = "macos") || self.permission.is_none())
            && Instant::now() >= self.start_at
        {
            self.start(directory).await?;
        }
        if let Some(child) = &self.child {
            let port = self.desktop["port"].as_u64().unwrap_or(0);
            let password = self.desktop["password"].as_str().unwrap_or("");
            let response = client
                .get(format!("http://127.0.0.1:{port}/healthz"))
                .basic_auth("lightos", Some(password))
                .timeout(Duration::from_secs(1))
                .send()
                .await;
            let health = match response {
                Ok(r) if r.status().is_success() => r.json::<Value>().await.ok(),
                _ => None,
            };
            let ready = (!cfg!(target_os = "macos") || self.permissions["can_start"] == true)
                && health.as_ref().is_some_and(|v| {
                    v["native"] == true
                        && v["ready"] == true
                        && v["pid"].as_u64() == child.id().map(u64::from)
                });
            self.desktop["ready"] = json!(ready);
            self.desktop["requires_consent"] = json!(
                health
                    .as_ref()
                    .is_some_and(|v| v["requires_consent"] == true)
            );
            if ready {
                self.error = None;
            }
        }
        Ok(())
    }

    async fn start(&mut self, directory: &Path) -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let password = secret();
        let epoch = secret()[..32].to_owned();
        let mut command = Command::new(std::env::current_exe()?);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        command
            .arg("serve")
            .env("LIGHTOS_REMOTE_DESKTOP_ADDR", "127.0.0.1")
            .env("LIGHTOS_REMOTE_DESKTOP_PORT", port.to_string())
            .env("LIGHTOS_REMOTE_DESKTOP_AUTH_USER", "lightos")
            .env("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD", &password)
            .env("LIGHTOS_REMOTE_DESKTOP_SUBFOLDER", "")
            .env(
                "LIGHTOS_REMOTE_DESKTOP_MCP_SETTINGS",
                directory.join("mcp.json"),
            )
            .env(
                "LIGHTOS_REMOTE_DESKTOP_RDP_SETTINGS",
                directory.join("rdp.json"),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        self.child = Some(command.spawn()?);
        self.desktop =
            json!({"enabled":true,"ready":false,"port":port,"password":password,"epoch":epoch});
        self.error = None;
        Ok(())
    }

    pub async fn stop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.permission_requests.clear();
            self.checked_for_start = false;
        }
        if let Some((mut child, _, _)) = self.permission.take() {
            let _ = child.kill().await;
        }
        self.permissions["requesting"] = json!(false);
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGTERM);
                }
            }
            #[cfg(windows)]
            if let Some(pid) = child.id() {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output()
                    .await;
            }
            if tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .is_err()
            {
                #[cfg(unix)]
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
            }
        }
        self.desktop = json!({"enabled":false,"ready":false});
        self.error = None;
        self.start_at = Instant::now();
    }

    pub fn public(&self, enabled: bool) -> Value {
        let status = if !enabled {
            "disabled"
        } else if self.permissions["can_start"] != true {
            "permission_required"
        } else if self.desktop["ready"] == true {
            "running"
        } else if self.error.is_some() {
            "error"
        } else {
            "starting"
        };
        json!({"version":env!("CARGO_PKG_VERSION"),"enabled":enabled,"ready":self.desktop["ready"] == true,"status":status,"permissions":self.permissions,"permission_check":cfg!(target_os = "macos"),"error":self.error})
    }
}
