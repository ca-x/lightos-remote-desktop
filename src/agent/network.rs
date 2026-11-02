//! Reuse the authenticated PC client's public local API and service gateway.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::Path, time::Duration};

mod wire;

#[derive(Clone)]
pub struct Connection {
    pub id: String,
    pub identity: String,
    pub device: String,
    pub box_id: String,
    pub catalog: String,
    pub token: String,
    grpc: String,
    credential: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct ClientConfig {
    http_port: u16,
    extra_headers: HashMap<String, String>,
}

pub fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        // Device/application certificates are validated by the existing Lazycat network.
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()?)
}

impl Connection {
    pub async fn discover(path: &Path, client: &reqwest::Client) -> Result<Self> {
        let config: ClientConfig = serde_json::from_slice(&tokio::fs::read(path).await?)?;
        let local = format!("http://127.0.0.1:{}", config.http_port);
        let get = |path: String| {
            let mut request = client.get(format!("{local}{path}"));
            for (key, value) in &config.extra_headers {
                request = request.header(key, value);
            }
            request
        };
        let active: Value = get("/default-box".into())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(active["status"] == 2, "client is not connected");
        let box_id = text(&active, "boxId")?;
        let account = text(&active, "loginUser")?;
        let domain = text(&active, "boxDomain")
            .or_else(|_| text(&active, "boxName").map(|n| format!("{n}.heiyu.space")))?;
        let host = reqwest::Url::parse(&format!("https://{domain}/"))?;
        ensure!(
            host.host_str().is_some()
                && host.username().is_empty()
                && host.path() == "/"
                && host.query().is_none(),
            "invalid box domain"
        );
        let mut endpoint = reqwest::Url::parse(&format!("{local}/box-ext-info"))?;
        endpoint.query_pairs_mut().append_pair("url", host.as_str());
        let ext: Value = get(format!("/box-ext-info?{}", endpoint.query().unwrap_or("")))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let grpc = text(&ext["grpc"], "addr")?;
        let socket: std::net::SocketAddr = grpc.parse()?;
        ensure!(socket.ip().is_loopback(), "shell API must be local");
        let credential = text(&ext["grpc"], "cred")?;
        let cookie = ext["cookies"]
            .as_array()
            .and_then(|cookies| {
                cookies.iter().find(|value| {
                    value["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("Lzc-Auth-Token"))
                })
            })
            .and_then(|value| value["value"].as_str())
            .filter(|value| !value.is_empty());
        let token = if let Some(cookie) = cookie {
            cookie.to_owned()
        } else {
            // Native clients can retain the active login without an Electron cookie.
            let reply = wire::call(
                client,
                &grpc,
                &credential,
                "ShellCore/QueryLzcAuthToken",
                vec![],
            )
            .await?;
            wire::string(&reply, 1)?
        };
        let login: Value = client
            .get(host.join("sys/whoami")?)
            .header("Lzc-Auth-Token", &token)
            .header("Cookie", format!("Lzc-Auth-Token={token}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(
            login["uid"].as_str() == Some(account.as_str()),
            "active login changed"
        );
        let reply = wire::call(
            client,
            &grpc,
            &credential,
            "ShellCore/QueryShellCoreInfo",
            vec![],
        )
        .await?;
        let device = wire::string(&reply, 1)?;
        let identity = serde_json::to_string(&[&box_id, &account, &device])?;
        let id = format!("{:x}", Sha256::digest(identity.as_bytes()))[..32].to_owned();
        let query: Value = client
            .get(host.join("sys/api/queryAppDomain?appid=cloud.lazycat.lightos.remote-desktop")?)
            .header("Lzc-Auth-Token", &token)
            .header("Cookie", format!("Lzc-Auth-Token={token}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let catalog_host = text(&query, "Domain")?;
        let catalog = if catalog_host.starts_with("https://") {
            catalog_host
        } else {
            format!("https://{catalog_host}")
        };
        let parsed = reqwest::Url::parse(&catalog)?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.host_str().is_some()
                && parsed.username().is_empty()
                && parsed.query().is_none(),
            "invalid management domain"
        );
        Ok(Self {
            id,
            identity,
            device,
            box_id,
            catalog: catalog.trim_end_matches('/').into(),
            token,
            grpc,
            credential,
            client: client.clone(),
        })
    }

    pub async fn register(
        &self,
        service: &str,
        port: u16,
        header: &str,
        secret: &str,
    ) -> Result<()> {
        let mut payload = vec![8, 1]; // HTTP service
        wire::field(&mut payload, 2, service.as_bytes());
        wire::field(
            &mut payload,
            3,
            format!("http://127.0.0.1:{port}").as_bytes(),
        );
        wire::field(&mut payload, 4, secret.as_bytes());
        wire::field(&mut payload, 5, header.as_bytes());
        wire::field(&mut payload, 6, &[24, 1]); // StripPrefix
        wire::call(
            &self.client,
            &self.grpc,
            &self.credential,
            "LocalServiceGateway/RegisterLocalService",
            payload,
        )
        .await?;
        Ok(())
    }

    pub async fn publish(
        &self,
        control: &str,
        password: &str,
        desktop_service: &str,
        desktop: Value,
        status: Value,
    ) -> Result<()> {
        let platform = match std::env::consts::OS {
            "macos" => "darwin",
            "windows" => "win32",
            s => s,
        };
        let name = std::env::var("COMPUTERNAME")
            .ok()
            .or_else(|| {
                std::process::Command::new("hostname")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().into())
            })
            .unwrap_or_else(|| platform.into());
        self.client.post(format!("{}/api/hosts", self.catalog))
            .header("Lzc-Auth-Token", &self.token).header("Cookie", format!("Lzc-Auth-Token={}", self.token))
            .header("X-HC-Device-ID", &self.device)
            .json(&json!({"id":self.id,"name":name,"platform":platform,"device_id":self.device,"box_id":self.box_id,
                "control_service_name":control,"control_password":password,"desktop_service_name":desktop_service,"desktop":desktop,"agent":status}))
            .send().await?.error_for_status()?;
        Ok(())
    }
}

fn text(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .with_context(|| format!("missing {key}"))
}
