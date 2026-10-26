//! Session-scoped cursor metadata, separate from the video queue.
use crate::backend::DesktopBackend;
use anyhow::{Context, Result, ensure};
use axum::extract::ws::{Message, WebSocket};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use image::{
    ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
#[cfg(target_os = "linux")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(crate) struct Hub {
    next: AtomicU64,
    current: Mutex<Option<Arc<Session>>>,
}
struct Session {
    id: u64,
    token: String,
    backend: Arc<dyn DesktopBackend>,
    connected: AtomicBool,
    stop: CancellationToken,
}
pub(crate) struct Scope {
    hub: Arc<Hub>,
    session: Arc<Session>,
}
pub(crate) struct Lease {
    session: Arc<Session>,
}
impl Scope {
    pub(crate) fn token(&self) -> &str {
        &self.session.token
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.session.stop.cancel();
        self.session.backend.release_cursor(self.session.id);
        let mut current = self.hub.current.lock().unwrap_or_else(|e| e.into_inner());
        if current.as_ref().is_some_and(|s| s.id == self.session.id) {
            current.take();
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.session.backend.release_cursor(self.session.id);
        self.session.connected.store(false, Ordering::Release);
    }
}
impl Hub {
    pub(crate) fn start(self: &Arc<Self>, backend: Arc<dyn DesktopBackend>) -> Scope {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        backend.begin_cursor(id);
        let token = rand::random::<[u8; 16]>()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let session = Arc::new(Session {
            id,
            token,
            backend,
            connected: AtomicBool::new(false),
            stop: CancellationToken::new(),
        });
        if let Some(previous) = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(session.clone())
        {
            previous.stop.cancel();
            previous.backend.release_cursor(previous.id);
        }
        Scope {
            hub: self.clone(),
            session,
        }
    }
    pub(crate) fn attach(&self, token: &str) -> Option<Lease> {
        let current = self.current.lock().ok()?;
        let session = current.as_ref()?;
        if session.token != token
            || session.stop.is_cancelled()
            || session
                .connected
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return None;
        }
        Some(Lease {
            session: session.clone(),
        })
    }
}

#[cfg(target_os = "linux")]
async fn request(path: &Path, pid: i32, text: &str) -> Result<Vec<u8>> {
    timeout(Duration::from_millis(500), async {
        let mut stream = tokio::net::UnixStream::connect(path).await?;
        let peer = stream.peer_cred()?;
        ensure!(
            peer.pid() == Some(pid) && peer.uid() == unsafe { libc::geteuid() },
            "cursor IPC owner mismatch"
        );
        stream.write_all(text.as_bytes()).await?;
        let mut bytes = Vec::new();
        stream
            .take(2 * 1024 * 1024 + 8192)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(
            bytes.len() < 2 * 1024 * 1024 + 8192,
            "cursor response too large"
        );
        Ok(bytes)
    })
    .await
    .context("cursor metadata timed out")?
}
#[cfg(not(target_os = "linux"))]
async fn request(_: &Path, _: i32, _: &str) -> Result<Vec<u8>> {
    anyhow::bail!("independent cursor metadata unavailable; cursor is embedded in native video")
}
async fn source(backend: &dyn DesktopBackend) -> Result<(PathBuf, i32, Option<String>)> {
    let (runtime, pid, output) = backend.cursor_target()?;
    let mut entries = tokio::fs::read_dir(runtime.join("hypr")).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path().join(".socket.sock");
        if request(&path, pid, "j/version").await.is_err() {
            continue;
        }
        let reply = request(&path, pid, "j/lightoscursor 0").await?;
        if serde_json::from_slice::<serde_json::Value>(&reply)
            .ok()
            .is_some_and(|value| value["version"] == 1)
        {
            return Ok((path, pid, output));
        }
        let adapter = std::env::current_exe()?
            .parent()
            .context("missing executable directory")?
            .join("lightos-cursor.so");
        let metadata = std::fs::symlink_metadata(&adapter)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "cursor adapter must be a regular file"
        );
        let adapter = adapter.to_str().context("invalid cursor adapter path")?;
        ensure!(
            !adapter.contains(['\r', '\n']),
            "invalid cursor adapter path"
        );
        let result = request(&path, pid, &format!("/plugin load {adapter}")).await?;
        ensure!(
            std::str::from_utf8(&result)?.trim() == "ok",
            "cursor adapter rejected: {}",
            String::from_utf8_lossy(&result)
        );
        return Ok((path, pid, output));
    }
    anyhow::bail!("matching cursor observer unavailable")
}

#[derive(Deserialize)]
struct Output {
    name: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Snapshot {
    version: u32,
    available: bool,
    visible: bool,
    serial: u64,
    x: f64,
    y: f64,
    hotspot_x: f64,
    hotspot_y: f64,
    scale: f64,
    width: u32,
    height: u32,
    format: u32,
    pixels: Option<String>,
    outputs: Vec<Output>,
}
fn png(snapshot: &Snapshot) -> Result<String> {
    ensure!(
        snapshot.width > 0
            && snapshot.height > 0
            && snapshot.width <= 512
            && snapshot.height <= 512,
        "invalid cursor image size"
    );
    let hex = snapshot.pixels.as_ref().context("missing cursor image")?;
    ensure!(
        hex.len() == snapshot.width as usize * snapshot.height as usize * 8,
        "invalid cursor pixels"
    );
    let digit = |value: u8| -> Result<u8> {
        match value {
            b'0'..=b'9' => Ok(value - b'0'),
            b'a'..=b'f' => Ok(value - b'a' + 10),
            _ => anyhow::bail!("invalid cursor pixels"),
        }
    };
    let mut pixels = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().as_chunks::<2>().0 {
        pixels.push((digit(pair[0])? << 4) | digit(pair[1])?);
    }
    let bgra = snapshot.format == 0
        || snapshot.format == 1
        || matches!(
            snapshot.format.to_le_bytes(),
            [b'A' | b'X', b'R', b'2', b'4']
        );
    let rgba = matches!(
        snapshot.format.to_le_bytes(),
        [b'A' | b'X', b'B', b'2', b'4']
    );
    ensure!(bgra || rgba, "unsupported cursor pixel format");
    let opaque = snapshot.format == 1
        || matches!(
            snapshot.format.to_le_bytes(),
            [b'X', b'R' | b'B', b'2', b'4']
        );
    for pixel in pixels.as_chunks_mut::<4>().0 {
        if bgra {
            pixel.swap(0, 2);
        }
        if opaque {
            pixel[3] = 255;
        }
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = (u32::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    let mut output = Vec::new();
    PngEncoder::new_with_quality(&mut output, CompressionType::Fast, FilterType::Sub).write_image(
        &pixels,
        snapshot.width,
        snapshot.height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(STANDARD.encode(output))
}

pub(crate) async fn connection(socket: WebSocket, lease: Lease) {
    let session = &lease.session;
    let (mut writer, mut reader) = socket.split();
    let work = async {
        let epoch = session.backend.privacy_epoch();
        let check_privacy = || -> Result<()> {
            ensure!(
                !session.backend.paused()
                    && !session.backend.is_stopped()
                    && session.backend.privacy_epoch() == epoch,
                "cursor desktop locked or changed"
            );
            Ok(())
        };
        check_privacy()?;
        let (path, pid, output) = source(session.backend.as_ref()).await?;
        let mut tick = tokio::time::interval(Duration::from_millis(16));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous = 0;
        let mut last = String::new();
        let mut keepalive = tokio::time::Instant::now();
        loop {
            tick.tick().await;
            check_privacy()?;
            let bytes = request(&path, pid, &format!("j/lightoscursor {previous}")).await?;
            check_privacy()?;
            let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
            ensure!(snapshot.version == 1, "unsupported cursor observer version");
            if !snapshot.available {
                session.backend.release_cursor(session.id);
                previous = 0;
                let message = json!({"type":"cursor_unavailable"}).to_string();
                if last != message {
                    writer.send(Message::Text(message.clone().into())).await?;
                    last = message;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            let monitor = match output.as_deref() {
                Some(name) => snapshot.outputs.iter().find(|monitor| monitor.name == name),
                None if snapshot.outputs.len() == 1 => snapshot.outputs.first(),
                _ => None,
            }
            .context("cursor output unavailable")?;
            ensure!(
                monitor.width.is_finite()
                    && monitor.height.is_finite()
                    && monitor.width > 0.
                    && monitor.height > 0.
                    && snapshot.scale.is_finite()
                    && snapshot.scale > 0.
                    && snapshot.scale <= 8.
                    && [
                        snapshot.x,
                        snapshot.y,
                        snapshot.hotspot_x,
                        snapshot.hotspot_y,
                        monitor.x,
                        monitor.y
                    ]
                    .into_iter()
                    .all(f64::is_finite),
                "invalid cursor geometry"
            );
            let x = (snapshot.x - monitor.x) / monitor.width;
            let y = (snapshot.y - monitor.y) / monitor.height;
            let width = f64::from(snapshot.width) / snapshot.scale;
            let height = f64::from(snapshot.height) / snapshot.scale;
            let visible = snapshot.visible
                && snapshot.x - snapshot.hotspot_x + width > monitor.x
                && snapshot.y - snapshot.hotspot_y + height > monitor.y
                && snapshot.x - snapshot.hotspot_x < monitor.x + monitor.width
                && snapshot.y - snapshot.hotspot_y < monitor.y + monitor.height;
            let mut packet = json!({"type":"cursor","visible":visible,"id":snapshot.serial.to_string(),"x":x,"y":y,
                "width":snapshot.width,"height":snapshot.height,"display_width":width,"display_height":height,
                "hotspot_x":snapshot.hotspot_x,"hotspot_y":snapshot.hotspot_y,"units_width":monitor.width,"units_height":monitor.height});
            if snapshot.pixels.is_some() {
                packet["png"] = json!(png(&snapshot)?);
            }
            let message = packet.to_string();
            check_privacy()?;
            if last != message {
                timeout(
                    Duration::from_millis(500),
                    writer.send(Message::Text(message.clone().into())),
                )
                .await??;
                check_privacy()?;
                session.backend.separate_cursor(session.id);
                previous = snapshot.serial;
                last = message;
            } else if keepalive.elapsed() >= Duration::from_secs(10) {
                timeout(
                    Duration::from_millis(500),
                    writer.send(Message::Ping(bytes::Bytes::new())),
                )
                .await??;
                keepalive = tokio::time::Instant::now();
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    let receive =
        async { while let Some(Ok(Message::Ping(_) | Message::Pong(_))) = reader.next().await {} };
    tokio::select! {
        biased;
        _ = session.stop.cancelled() => {},
        _ = receive => {},
        result = work => { if let Err(error)=result { tracing::debug!(%error,"cursor channel ended"); } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_png_preserves_transparency_and_unpremultiplies_bgra() {
        let source = Snapshot {
            width: 2,
            height: 1,
            format: u32::from_le_bytes(*b"AR24"),
            pixels: Some("14283c8000000000".into()),
            ..Snapshot::default()
        };
        let bytes = STANDARD.decode(png(&source).unwrap()).unwrap();
        let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(image.as_raw(), &[120, 80, 40, 128, 0, 0, 0, 0]);
        let bad = Snapshot {
            width: 513,
            ..source
        };
        assert!(png(&bad).is_err());
    }
}
