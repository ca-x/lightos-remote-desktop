//! The shipped PC controller can invoke the installed program on demand but
//! cannot forward new verbs. Redeem a device-bound, one-use instruction on that
//! invocation. Ordinary background permission probes never contact this endpoint.
use super::network::Connection;
use anyhow::{Context, Result, ensure};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

fn credential() -> Result<Option<String>> {
    let executable = std::env::current_exe()?;
    // The version directory is outside the signed application bundle and
    // is removed together with its credential during an update or uninstall.
    let Some(version) = executable.ancestors().find(|dir| {
        dir.parent()
            .is_some_and(|p| p.ends_with("remote-desktop/managed/versions"))
    }) else {
        return Ok(None);
    };
    let path = version.join(".power-channel").join("credential");
    let storage = crate::private_storage::current();
    let key = match storage.read(&path, 64)? {
        Some(bytes) => String::from_utf8(bytes)?,
        None => {
            let key = format!("{:064x}", Sha256::digest(rand::random::<[u8; 32]>()));
            storage.write(&path, key.as_bytes())?;
            key
        }
    };
    ensure!(
        key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid device credential"
    );
    Ok(Some(key))
}

pub fn verifier() -> Option<String> {
    credential()
        .ok()
        .flatten()
        .map(|key| format!("{:x}", Sha256::digest(key.as_bytes())))
}

#[derive(Deserialize)]
struct Work {
    action: String,
    #[serde(default)]
    nonce: String,
}

/// True only if this invocation should request the ordinary desktop permissions.
pub async fn dispatch() -> Result<bool> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return Ok(true);
    }
    let Some(key) = credential()? else {
        return Ok(true);
    };
    let path = dirs::home_dir()
        .context("home directory unavailable")?
        .join(".config/lzc-client-desktop/core.cfg");
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(8))
        .build()?;
    let conn = Connection::discover(&path, &client).await?;
    let url = format!("{}/api/power-worker/{}", conn.catalog, conn.id);
    let post = |suffix: &str| {
        client
            .post(format!("{url}/{suffix}"))
            .header("Lzc-Auth-Token", &conn.token)
            .header("Cookie", format!("Lzc-Auth-Token={}", conn.token))
            .header("X-HC-Device-ID", &conn.device)
            .header("X-Lazycat-Power-Key", &key)
    };
    let response = post("claim").json(&json!({})).send().await?;
    // Old management packages have no command endpoint.
    if matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) {
        return Ok(true);
    }
    let work: Work = response.error_for_status()?.json().await?;
    use crate::power::Operation;
    let op = match work.action.as_str() {
        "permissions" => return Ok(true),
        "idle" => return Ok(false),
        "read" => Operation::Read,
        "enable" => Operation::Enable,
        "restore" => Operation::Restore,
        _ => anyhow::bail!("unrecognized device instruction"),
    };
    ensure!(work.nonce.len() == 64, "invalid power request");
    let result = tokio::task::spawn_blocking(move || crate::power::execute(op)).await?;
    post("result")
        .json(&json!({"nonce":work.nonce,"enabled":result.ok()}))
        .send()
        .await?
        .error_for_status()?;
    Ok(false)
}
