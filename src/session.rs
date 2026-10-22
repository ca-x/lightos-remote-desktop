use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};

use crate::config::Config;

pub struct Session {
    pub socket: PathBuf,
    pub runtime: PathBuf,
    pub signature: Option<String>,
    pub pulse_server: Option<String>,
    pub pulse_client_config: Option<PathBuf>,
    pub audio_source: Option<String>,
    children: Vec<Child>,
    temporary: Option<tempfile::TempDir>,
    lock: Option<File>,
    cancelled: Arc<AtomicBool>,
}

impl Session {
    pub fn start(config: &Config) -> Result<Self> {
        Self::start_cancellable(config, Arc::new(AtomicBool::new(false)))
    }

    pub fn start_cancellable(config: &Config, cancelled: Arc<AtomicBool>) -> Result<Self> {
        ensure!(
            unsafe { libc::geteuid() } != 0,
            "desktop service must run as a non-root user"
        );
        if let Some(socket) = &config.wayland_socket {
            let runtime = socket
                .parent()
                .context("Wayland socket has no parent")?
                .to_path_buf();
            ensure!(socket.exists(), "Wayland socket does not exist");
            return Ok(Self {
                socket: socket.clone(),
                runtime,
                signature: None,
                pulse_server: std::env::var("PULSE_SERVER").ok(),
                pulse_client_config: std::env::var_os("PULSE_CLIENTCONFIG").map(PathBuf::from),
                audio_source: None,
                children: Vec::new(),
                temporary: None,
                lock: None,
                cancelled,
            });
        }
        let temporary = if config.runtime_dir.is_none() {
            Some(tempfile::Builder::new().prefix("lrd.").tempdir_in("/tmp")?)
        } else {
            None
        };
        let runtime = config
            .runtime_dir
            .clone()
            .unwrap_or_else(|| temporary.as_ref().unwrap().path().to_path_buf());
        secure_directory(&runtime)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(runtime.join("session.lock"))?;
        ensure!(
            unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&lock),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            } == 0,
            "another desktop owns this runtime directory"
        );
        let mut session = Self {
            socket: PathBuf::new(),
            pulse_server: Some(format!("unix:{}/pulse/native", runtime.display())),
            runtime,
            signature: None,
            pulse_client_config: None,
            audio_source: None,
            children: Vec::new(),
            temporary,
            lock: Some(lock),
            cancelled,
        };
        let bus_path = session.runtime.join("bus");
        let mut bus = Command::new("dbus-daemon");
        bus.args(["--session", "--nofork", "--nopidfile"])
            .arg(format!("--address=unix:path={}", bus_path.display()));
        session.spawn(bus, "dbus.log")?;
        session.wait_for(Duration::from_secs(5), || bus_path.exists())?;
        if config.audio {
            session
                .setup_audio()
                .context("starting private desktop audio")?;
        }
        let desktop_config = if let Some(path) = &config.desktop_config {
            ensure!(path.is_file(), "desktop configuration is missing");
            fs::canonicalize(path)?
        } else {
            let path = session.runtime.join("hyprland.conf");
            let (width, height) = config.dimensions()?;
            fs::write(
                &path,
                include_str!("../config/hyprland.conf").replace(
                    "1280x720@30",
                    &format!("{}x{}@{}", width, height, config.fps),
                ),
            )?;
            path
        };
        let mut command = Command::new(&config.cage);
        command.arg("--");
        if let Some(directory) = &config.aquamarine_dir {
            let directory = fs::canonicalize(directory)?;
            ensure!(
                directory.join("libaquamarine.so.14").is_file(),
                "missing private libaquamarine.so.14"
            );
            command
                .arg("/usr/lib/ld-linux-x86-64.so.2")
                .arg("--library-path")
                .arg(format!("{}:/usr/lib", directory.display()));
        }
        let hyprland = std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join("Hyprland"))
                    .find(|path| path.is_file())
            })
            .context("Hyprland not found in PATH")?;
        command
            .arg(hyprland)
            .arg("--config")
            .arg(desktop_config)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env("XDG_RUNTIME_DIR", &session.runtime)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", bus_path.display()),
            )
            .env("WLR_BACKENDS", "headless")
            .env("WLR_HEADLESS_OUTPUTS", "1")
            .env("WLR_RENDERER", "gles2")
            .env("WLR_RENDER_DRM_DEVICE", "/dev/dri/renderD128")
            .env("XDG_CURRENT_DESKTOP", "Hyprland")
            .env("XDG_SESSION_TYPE", "wayland");
        session.audio_environment(&mut command);
        if let Some(source) = &session.audio_source {
            command.env("PULSE_SINK", source.trim_end_matches(".monitor"));
        }
        session.spawn(command, "desktop.log")?;
        let runtime = session.runtime.clone();
        session.wait_for(Duration::from_secs(20), || {
            desktop_socket(&runtime).is_some()
        })?;
        session.socket =
            desktop_socket(&session.runtime).context("nested Wayland socket disappeared")?;
        session.signature = fs::read_dir(session.runtime.join("hypr"))?
            .filter_map(Result::ok)
            .find(|entry| entry.path().join(".socket.sock").exists())
            .map(|entry| entry.file_name().to_string_lossy().into_owned());
        tracing::info!(socket = %session.socket.display(), "native desktop session ready");
        Ok(session)
    }

    fn spawn(&mut self, mut command: Command, log: &str) -> Result<()> {
        let output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(self.runtime.join(log))?;
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output);
        self.children
            .push(command.spawn().with_context(|| format!("starting {log}"))?);
        Ok(())
    }

    fn wait_for(&mut self, timeout: Duration, ready: impl Fn() -> bool) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            ensure!(
                !self.cancelled.load(Ordering::Acquire),
                "desktop startup cancelled"
            );
            if self
                .children
                .iter()
                .any(|child| child_exited(child).unwrap_or(true))
            {
                bail!("desktop dependency exited: {}", self.log_tail());
            }
            if ready() {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "desktop startup timed out: {}",
                self.log_tail()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn log_tail(&self) -> String {
        use std::io::{Read, Seek, SeekFrom};
        ["desktop.log", "dbus.log"]
            .into_iter()
            .filter_map(|name| {
                let mut file = File::open(self.runtime.join(name)).ok()?;
                let size = file.metadata().ok()?.len();
                file.seek(SeekFrom::Start(size.saturating_sub(8192))).ok()?;
                let mut bytes = Vec::new();
                file.take(8192).read_to_end(&mut bytes).ok()?;
                Some(format!("{name}: {}", String::from_utf8_lossy(&bytes)))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn setup_audio(&mut self) -> Result<()> {
        let configuration = self.runtime.join("audio-config");
        let fragments = configuration.join("wireplumber/wireplumber.conf.d");
        fs::create_dir_all(&fragments)?;
        fs::write(
            fragments.join("90-lightos.conf"),
            include_str!("../config/90-lightos-audio.conf"),
        )?;
        for (program, arguments, log, ready_socket) in [
            (
                "pipewire",
                Vec::<&str>::new(),
                "pipewire.log",
                Some(self.runtime.join("pipewire-0")),
            ),
            (
                "pipewire-pulse",
                vec![],
                "pipewire-pulse.log",
                Some(self.runtime.join("pulse/native")),
            ),
            (
                "wireplumber",
                vec!["--profile=lightos"],
                "wireplumber.log",
                None,
            ),
        ] {
            let mut command = Command::new(program);
            command
                .args(arguments)
                .env("XDG_RUNTIME_DIR", &self.runtime)
                .env("PIPEWIRE_RUNTIME_DIR", &self.runtime)
                .env("PIPEWIRE_REMOTE", "pipewire-0")
                .env("XDG_CONFIG_HOME", &configuration)
                .env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}/bus", self.runtime.display()),
                )
                .env_remove("PULSE_SERVER")
                .env_remove("PULSE_CLIENTCONFIG");
            self.spawn(command, log)?;
            if let Some(socket) = ready_socket {
                self.wait_for(Duration::from_secs(5), || socket.exists())?;
            }
        }
        let sink = format!("lightos_remote_{}", std::process::id());
        let mut command = Command::new("timeout");
        command
            .args([
                "--kill-after=1",
                "5",
                "pactl",
                "load-module",
                "module-null-sink",
            ])
            .arg(format!("sink_name={sink}"))
            .arg("sink_properties=device.description=LightOS_Remote_Desktop");
        self.audio_environment(&mut command);
        let output = command.output()?;
        ensure!(
            output.status.success(),
            "could not create isolated audio sink: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let module = String::from_utf8(output.stdout)?.trim().to_owned();
        ensure!(
            !module.is_empty() && module.bytes().all(|byte| byte.is_ascii_digit()),
            "invalid audio module identifier"
        );
        self.audio_source = Some(format!("{sink}.monitor"));
        Ok(())
    }

    pub fn healthy(&mut self) -> bool {
        self.children
            .iter()
            .all(|child| matches!(child_exited(child), Ok(false)))
            && self.socket.exists()
    }

    pub fn audio_environment(&self, command: &mut Command) {
        if let Some(home) = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        {
            command.env("XDG_CONFIG_HOME", home);
        }
        if let Some(server) = &self.pulse_server {
            command.env("PULSE_SERVER", server);
        }
        if let Some(config) = &self.pulse_client_config {
            command.env("PULSE_CLIENTCONFIG", config);
        } else {
            command.env_remove("PULSE_CLIENTCONFIG");
        }
    }
}

fn desktop_socket(runtime: &Path) -> Option<PathBuf> {
    let signature = fs::read_dir(runtime.join("hypr"))
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.path().join(".socket.sock").exists())?;
    let _ = signature;
    fs::read_dir(runtime)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("wayland-"))
        .filter(|entry| !entry.file_name().to_string_lossy().ends_with(".lock"))
        .filter(|entry| std::os::unix::net::UnixStream::connect(entry.path()).is_ok())
        .max_by_key(|entry| entry.file_name())
        .map(|entry| entry.path())
}

fn secure_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "runtime directory must not be a symlink"
    );
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() },
        "runtime directory has another owner"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "runtime directory must be private (0700)"
    );
    Ok(())
}

impl Drop for Session {
    fn drop(&mut self) {
        for child in self.children.iter_mut().rev() {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while !matches!(child_exited(child), Ok(true)) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
        }
        self.lock.take();
        self.temporary.take();
    }
}

fn child_exited(child: &Child) -> Result<bool> {
    let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id(),
            &mut status,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    ensure!(
        result == 0,
        "could not inspect owned process: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { status.si_pid() } != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn default_desktop_starts_a_visible_application() {
        let config = include_str!("../config/hyprland.conf");
        assert!(config.lines().any(|line| line == "$terminal = foot"));
        assert!(config.lines().any(|line| line == "exec-once = $terminal"));
    }

    #[test]
    fn runtime_directory_rejects_symlink_and_public_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let link = directory.path().join("link");
        symlink(directory.path(), &link).unwrap();
        assert!(secure_directory(&link).is_err());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(secure_directory(directory.path()).is_err());
    }

    #[test]
    fn teardown_kills_descendants_even_when_leader_has_exited() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant");
        let child = Command::new("sh")
            .args([
                "-c",
                "(trap '' TERM; exec sleep 600) & echo $! > \"$1\"; exit 0",
                "sh",
            ])
            .arg(&pid_file)
            .process_group(0)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !pid_file.exists() || !child_exited(&child).unwrap() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let session = Session {
            socket: PathBuf::new(),
            runtime: directory.path().to_path_buf(),
            signature: None,
            pulse_server: None,
            pulse_client_config: None,
            audio_source: None,
            children: vec![child],
            temporary: None,
            lock: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        drop(session);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = fs::read_to_string(format!("/proc/{pid}/stat"));
            if status.as_ref().is_err()
                || status
                    .as_ref()
                    .unwrap()
                    .split_once(") ")
                    .unwrap()
                    .1
                    .starts_with('Z')
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned descendant survived teardown"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
