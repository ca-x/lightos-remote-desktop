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
    fmt::Write as _,
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
    clipboard_connected: AtomicBool,
    clipboard_gate: Arc<crate::clipboard::Gate>,
    clipboard_tx: tokio::sync::mpsc::Sender<crate::clipboard::CopyRequest>,
    clipboard_rx: Mutex<Option<tokio::sync::mpsc::Receiver<crate::clipboard::CopyRequest>>>,
}
pub(crate) struct Scope {
    hub: Arc<Hub>,
    session: Arc<Session>,
}
pub(crate) struct Lease {
    session: Arc<Session>,
}
pub(crate) struct ClipboardLease {
    session: Arc<Session>,
    pub(crate) requests: Option<tokio::sync::mpsc::Receiver<crate::clipboard::CopyRequest>>,
}
impl ClipboardLease {
    pub(crate) fn gate(&self) -> Arc<crate::clipboard::Gate> {
        self.session.clipboard_gate.clone()
    }
    pub(crate) fn backend(&self) -> Arc<dyn DesktopBackend> {
        self.session.backend.clone()
    }
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.session.stop.clone()
    }
}
impl Drop for ClipboardLease {
    fn drop(&mut self) {
        if let Some(requests) = self.requests.take() {
            *self
                .session
                .clipboard_rx
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(requests);
        }
        self.session
            .clipboard_connected
            .store(false, Ordering::Release);
    }
}
impl Scope {
    pub(crate) fn clipboard_gate(&self) -> Arc<crate::clipboard::Gate> {
        self.session.clipboard_gate.clone()
    }
    pub(crate) fn token(&self) -> &str {
        &self.session.token
    }
    pub(crate) fn clipboard_sender(
        &self,
    ) -> tokio::sync::mpsc::Sender<crate::clipboard::CopyRequest> {
        self.session.clipboard_tx.clone()
    }
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.session.stop.clone()
    }
    pub(crate) fn cancel(&self) {
        self.session.stop.cancel();
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
        let (clipboard_tx, clipboard_rx) = tokio::sync::mpsc::channel(8);
        let session = Arc::new(Session {
            id,
            token,
            backend,
            connected: AtomicBool::new(false),
            stop: CancellationToken::new(),
            clipboard_connected: AtomicBool::new(false),
            clipboard_gate: Arc::default(),
            clipboard_tx,
            clipboard_rx: Mutex::new(Some(clipboard_rx)),
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
    pub(crate) fn attach_clipboard(&self, token: &str) -> Option<ClipboardLease> {
        let current = self.current.lock().ok()?;
        let session = current.as_ref()?;
        if session.token != token
            || session.stop.is_cancelled()
            || session.backend.paused()
            || session.backend.is_stopped()
            || session
                .clipboard_connected
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return None;
        }
        let requests = match session
            .clipboard_rx
            .lock()
            .ok()
            .and_then(|mut requests| requests.take())
        {
            Some(requests) => requests,
            None => {
                session.clipboard_connected.store(false, Ordering::Release);
                return None;
            }
        };
        Some(ClipboardLease {
            session: session.clone(),
            requests: Some(requests),
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
    anyhow::bail!("independent cursor metadata unavailable on this backend")
}
pub(crate) async fn focused_app(backend: &dyn DesktopBackend) -> Result<String> {
    let (runtime, pid, _) = backend.cursor_target()?;
    let mut entries = tokio::fs::read_dir(runtime.join("hypr")).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path().join(".socket.sock");
        if let Ok(bytes) = request(&path, pid, "j/activewindow").await {
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            return Ok(value["class"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(256)
                .collect());
        }
    }
    anyhow::bail!("focused application metadata unavailable")
}

async fn source(backend: &dyn DesktopBackend) -> Result<(PathBuf, i32, Option<String>)> {
    let (runtime, pid, output) = backend.cursor_target()?;
    source_for_target(runtime, pid, output).await
}

pub(crate) async fn source_for_target(
    runtime: PathBuf,
    pid: i32,
    output: Option<String>,
) -> Result<(PathBuf, i32, Option<String>)> {
    let mut entries = tokio::fs::read_dir(runtime.join("hypr")).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path().join(".socket.sock");
        if request(&path, pid, "j/version").await.is_err() {
            continue;
        }
        let reply = request(&path, pid, "j/lightosindependentcursor 0").await?;
        if serde_json::from_slice::<serde_json::Value>(&reply)
            .ok()
            .is_some_and(|value| value["version"] == 1 && value["cursor_free_capture"] == true)
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
        let reply = request(&path, pid, "j/lightosindependentcursor 0").await?;
        let capabilities: serde_json::Value = serde_json::from_slice(&reply)?;
        ensure!(
            capabilities["version"] == 1 && capabilities["cursor_free_capture"] == true,
            "cursor adapter cannot provide cursor-free capture"
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
impl Snapshot {
    fn native(frame: &crate::backend::CursorFrame, previous: u64) -> Self {
        let pixels = (frame.serial != previous && frame.width > 0 && frame.height > 0).then(|| {
            let mut hex = String::with_capacity(frame.pixels.len() * 2);
            for byte in frame.pixels.iter() {
                let _ = write!(hex, "{byte:02x}");
            }
            hex
        });
        Self {
            version: 1,
            available: true,
            visible: frame.visible,
            serial: frame.serial,
            x: frame.x * f64::from(frame.units_width),
            y: frame.y * f64::from(frame.units_height),
            hotspot_x: f64::from(frame.hotspot_x),
            hotspot_y: f64::from(frame.hotspot_y),
            scale: 1.,
            width: frame.width,
            height: frame.height,
            format: 0,
            pixels,
            outputs: vec![Output {
                name: String::new(),
                x: 0.,
                y: 0.,
                width: f64::from(frame.units_width),
                height: f64::from(frame.units_height),
            }],
        }
    }
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
        let native = session.backend.cursor_frames();
        let (path, pid, output) = if native.is_some() {
            (PathBuf::new(), 0, None)
        } else {
            source(session.backend.as_ref()).await?
        };
        let mut tick = tokio::time::interval(Duration::from_millis(16));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous = 0;
        let mut last = String::new();
        let mut keepalive = tokio::time::Instant::now();
        loop {
            tick.tick().await;
            check_privacy()?;
            let snapshot: Snapshot = if let Some(frames) = &native {
                match frames.borrow().as_deref() {
                    Some(frame) => Snapshot::native(frame, previous),
                    None => Snapshot {
                        version: 1,
                        ..Snapshot::default()
                    },
                }
            } else {
                let bytes = request(
                    &path,
                    pid,
                    &format!("j/lightosindependentcursor {previous}"),
                )
                .await?;
                serde_json::from_slice(&bytes)?
            };
            check_privacy()?;
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
    // Flush the peer's close reply (or initiate our own close). A raw TCP EOF
    // can leave a proxied browser socket CLOSING until its timeout, preventing
    // the independent cursor from reconnecting while video already fell back.
    let _ = timeout(Duration::from_millis(500), writer.close()).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn peer_close_receives_a_close_frame_without_waiting_for_tcp_eof() -> Result<()> {
        struct CursorBackend;
        impl crate::backend::DesktopBackend for CursorBackend {
            fn kind(&self) -> crate::backend::BackendKind {
                crate::backend::BackendKind::X11
            }
            fn stop(&self) {}
            fn frames(&self) -> tokio::sync::watch::Receiver<Option<Arc<crate::backend::Frame>>> {
                tokio::sync::watch::channel(None).1
            }
            fn input(&self) -> tokio::sync::mpsc::Sender<crate::backend::InputCommand> {
                tokio::sync::mpsc::channel(1).0
            }
            fn cursor_frames(
                &self,
            ) -> Option<tokio::sync::watch::Receiver<Option<Arc<crate::backend::CursorFrame>>>>
            {
                Some(
                    tokio::sync::watch::channel(Some(Arc::new(crate::backend::CursorFrame {
                        serial: 1,
                        visible: true,
                        x: 0.5,
                        y: 0.5,
                        width: 1,
                        height: 1,
                        hotspot_x: 0,
                        hotspot_y: 0,
                        units_width: 10,
                        units_height: 10,
                        pixels: vec![0, 0, 255, 255].into(),
                    })))
                    .1,
                )
            }
        }
        let hub = Arc::new(Hub::default());
        let scope = hub.start(Arc::new(CursorBackend));
        let token = scope.token().to_owned();
        let router = axum::Router::new().route(
            "/cursor",
            axum::routing::get(move |upgrade: axum::extract::ws::WebSocketUpgrade| {
                let lease = hub.attach(&token).unwrap();
                async move { upgrade.on_upgrade(move |socket| connection(socket, lease)) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(cancelled.cancelled_owned())
                .await
        });
        let (mut client, _) =
            tokio_tungstenite::connect_async(format!("ws://{address}/cursor")).await?;
        assert!(matches!(
            timeout(Duration::from_secs(1), client.next()).await?,
            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(_)))
        ));
        client.close(None).await?;
        let reply = timeout(Duration::from_secs(1), client.next()).await?;
        stop.cancel();
        server.await??;
        assert!(
            matches!(
                reply,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
            ),
            "cursor close reply: {reply:?}"
        );
        Ok(())
    }
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
