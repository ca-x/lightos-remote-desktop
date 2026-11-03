//! A short-lived capture lease on the viewing device. The manager binds this
//! endpoint to X-HC-Device-ID; target desktops never capture a viewer's keys.
use axum::{
    Json,
    extract::{
        Query, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::timeout,
};

#[derive(Deserialize)]
pub(crate) struct Options {
    #[serde(default)]
    capabilities: bool,
}
fn helper() -> Option<std::path::PathBuf> {
    let path = std::env::current_exe()
        .ok()?
        .parent()?
        .join(if cfg!(windows) {
            "lightos-input-capture.exe"
        } else {
            "lightos-input-capture"
        });
    path.is_file().then_some(path)
}
pub(crate) async fn route(
    headers: HeaderMap,
    Query(options): Query<Options>,
    upgrade: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let authorized = headers
        .get("x-lightos-viewer-input")
        .is_some_and(|v| v == "1");
    if options.capabilities || upgrade.is_err() {
        return Json(json!({"available":authorized && helper().is_some()})).into_response();
    }
    if !authorized {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    upgrade.unwrap().on_upgrade(capture).into_response()
}
async fn capture(mut socket: WebSocket) {
    static LEASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Some(path) = helper() else {
        return;
    };
    let Ok(Some(Ok(Message::Text(first)))) = timeout(Duration::from_secs(3), socket.recv()).await
    else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&first) else {
        return;
    };
    if value["type"] != "capture" {
        return;
    }
    let title = value["title"].as_str().unwrap_or("");
    if title.is_empty() || title.len() > 512 {
        return;
    }
    let Ok(_lease) = LEASE.try_lock() else {
        let _ = socket
            .send(Message::Text(
                json!({"type":"error","message":"Viewing keyboard is already captured"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    };
    let Ok(mut child) = Command::new(path)
        .arg(title)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    else {
        return;
    };
    let Some(mut input) = child.stdin.take() else {
        return;
    };
    let Some(output) = child.stdout.take() else {
        return;
    };
    let mut lines = BufReader::new(output).lines();
    let mut last = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let _ = input.write_all(b".").await;
    loop {
        tokio::select! {
            _ = tick.tick() => { if last.elapsed() > Duration::from_millis(1500) { break; } }
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Text(text))) if text.as_str() == "heartbeat" => {last = Instant::now();if input.write_all(b".").await.is_err() {break;}}
                    _ => break,
                }
            }
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break; };
                if line.len() > 2048 { break; }
                if !matches!(timeout(Duration::from_millis(500),socket.send(Message::Text(line.into()))).await,Ok(Ok(()))) {break;}
            }
        }
    }
    let _ = child.kill().await;
    let _ = timeout(
        Duration::from_millis(500),
        socket.send(Message::Close(None)),
    )
    .await;
}
