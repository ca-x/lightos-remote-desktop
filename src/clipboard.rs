use std::{
    io::Cursor,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

const MAX_IMAGE: usize = 8 * 1024 * 1024;
const MAX_CHUNK: usize = 48 * 1024;

#[cfg(target_os = "linux")]
fn asynchronous_file(fd: std::os::fd::OwnedFd) -> Result<tokio::io::unix::AsyncFd<std::fs::File>> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::from(fd);
    // Portal/Mutter transfer pipes may already be nonblocking. Tokio File is
    // for regular blocking files and surfaces EAGAIN instead of waiting here.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    ensure!(flags >= 0, "clipboard descriptor flags unavailable");
    ensure!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == 0,
        "clipboard descriptor could not be made asynchronous"
    );
    // File owns a stable descriptor; it is never replaced or closed while the
    // registration is alive, and callers access it only for read/write.
    Ok(unsafe { tokio::io::unix::AsyncFd::register(file) }?)
}

#[cfg(target_os = "linux")]
pub(crate) async fn read_fd(fd: std::os::fd::OwnedFd, limit: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = asynchronous_file(fd)?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 16384];
    loop {
        let mut ready = file.readable().await?;
        match ready.try_io(|fd| fd.get_ref().read(&mut buffer)) {
            Ok(Ok(0)) => return Ok(bytes),
            Ok(Ok(count)) => {
                ensure!(bytes.len() + count <= limit, "clipboard content too large");
                bytes.extend_from_slice(&buffer[..count]);
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => continue,
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) async fn write_fd(fd: std::os::fd::OwnedFd, mut bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let file = asynchronous_file(fd)?;
    while !bytes.is_empty() {
        let mut ready = file.writable().await?;
        match ready.try_io(|fd| fd.get_ref().write(bytes)) {
            Ok(Ok(0)) => anyhow::bail!("clipboard pipe closed during transfer"),
            Ok(Ok(count)) => bytes = &bytes[count..],
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => continue,
        }
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct Gate {
    pub(crate) lock: tokio::sync::Mutex<()>,
    pub(crate) latest: std::sync::atomic::AtomicU64,
    pub(crate) last_selection: std::sync::Mutex<Option<u64>>,
    pub(crate) local_revision: std::sync::Mutex<Option<crate::backend::ClipboardRevision>>,
}

pub(crate) struct CopyRequest {
    pub id: u64,
    pub before: Option<crate::backend::ClipboardRevision>,
    pub epoch: u64,
}

/// Bulk clipboard traffic has its own controller-scoped connection. It must
/// never hold the video writer while waiting for a selection owner or PNG data.
pub(crate) async fn connection(
    socket: axum::extract::ws::WebSocket,
    mut lease: crate::cursor::ClipboardLease,
    state: crate::server::AppState,
) {
    use axum::extract::ws::Message;
    use futures_util::{SinkExt, StreamExt};
    use tokio::time::timeout;
    // Closing this channel cancels queued clipboard injection without closing
    // the desktop or its independent cursor channel.
    let stop = lease.cancellation().child_token();
    let gate = lease.gate();
    let (mut writer, mut reader) = socket.split();
    let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel(16);
    let receive = async {
        while let Some(Ok(message)) = reader.next().await {
            match message {
                Message::Text(text) if text.len() <= 96 * 1024 => {
                    if incoming_tx.send(text).await.is_err() {
                        break;
                    }
                }
                Message::Ping(_) | Message::Pong(_) => {}
                _ => break,
            }
        }
        stop.cancel();
    };
    let requests = lease.requests.as_mut().unwrap();
    let service = async {
        let mut upload = None;
        let mut audio = None;
        let mut observed: Option<crate::backend::ClipboardRevision> = None;
        let mut mirror: Option<(u64, crate::backend::ClipboardRevision)> = None;
        let mut passive_id = 4_000_000_000_000_000_u64;
        let mut poll = tokio::time::interval(Duration::from_millis(125));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let copy = tokio::select! {
                biased;
                copy = requests.recv() => copy,
                incoming = incoming_rx.recv() => {
                    let Some(text) = incoming else { break; };
                    let epoch = state.backend.privacy_epoch();
                    let command: serde_json::Value = serde_json::from_str(&text)?;
                    let kind=command["type"].as_str().unwrap_or_default();
                    let id=command["id"].as_u64().unwrap_or(0);
                    let order=command["order"].as_u64().unwrap_or(id);
                    let mutating=matches!(kind,"clipboard_set"|"clipboard_paste_text"|"clipboard_image_paste"|"clipboard_image_set");
                    let _write_guard=if mutating || kind=="clipboard_mirror_start" {Some(gate.lock.lock().await)}else{None};
                    if kind=="clipboard_mirror_start" {
                        // Compare against the last selection we observed, not a
                        // fresh snapshot taken after this request's network trip.
                        // Otherwise a native menu Copy can happen first and an
                        // older focus mirror will incorrectly overwrite it.
                        let own = *gate.local_revision.lock().unwrap_or_else(|e| e.into_inner());
                        let known = match (observed, own) {
                            (Some(before), Some(own)) if before.epoch == own.epoch => Some(if own.serial > before.serial { own } else { before }),
                            (before, _) => before,
                        };
                        let revision=match known { Some(revision)=>Ok(revision),None=>state.backend.clipboard_revision().await };
                        let mut ack=serde_json::json!({"type":"clipboard_ack","id":id,"step":0});
                        match revision {
                            Ok(revision)=>{mirror=Some((id,revision));},
                            Err(error)=>{ack["error"]=error.to_string().into();},
                        }
                        timeout(Duration::from_secs(15),writer.send(Message::Text(ack.to_string().into()))).await??;
                        continue;
                    }
                    if command["mirror"]==true {
                        let valid=order>=gate.latest.load(std::sync::atomic::Ordering::Acquire)
                            && if mutating { match mirror {Some((active,before)) if active==id=>state.backend.clipboard_revision().await.is_ok_and(|now|now==before),_=>false}}else{true};
                        if !valid {
                            upload=None;
                            timeout(Duration::from_secs(15),writer.send(Message::Text(serde_json::json!({"type":"clipboard_ack","id":id,"step":command["step"],"error":"Clipboard changed while local synchronization was pending."}).to_string().into()))).await??;
                            continue;
                        }
                    }
                    if mutating {gate.latest.fetch_max(order,std::sync::atomic::Ordering::AcqRel);}

                    let local_data = match command["type"].as_str() {
                        Some("clipboard_set" | "clipboard_paste_text") => command["text"].as_str().map(|text| CopyData::Text(text.to_owned())),
                        Some("clipboard_image_paste" | "clipboard_image_set") => upload.as_ref().map(|image: &ImageUpload| CopyData::Image(image.bytes.clone())),
                        _ => None,
                    };
                    let result = async {
                        let value = serde_json::from_str::<serde_json::Value>(&text)?;
                        anyhow::ensure!(value["type"].as_str().is_some_and(|kind|kind.starts_with("clipboard_")), "unsupported clipboard command");
                        crate::server::handle_command(&state, &text, &mut audio, None, &mut upload, Some(&stop)).await
                    }.await;
                    let successful = result.as_ref().is_ok_and(|message| {
                        match message {
                            Some(Message::Text(text)) => serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value.get("error").is_none()),
                            _ => true,
                        }
                    });
                    if successful && let Some(data) = local_data {
                        let hash = fingerprint(data).await?.1;
                        *gate.last_selection.lock().unwrap_or_else(|e| e.into_inner()) = Some(hash);
                    }
                    if command["mirror"]==true && mutating || kind=="clipboard_cancel" {
                        mirror=None;
                    }
                    let message = match result {
                        Ok(message) => message,
                        Err(error) => Some(Message::Text(serde_json::json!({"type":"error","message":error.to_string()}).to_string().into())),
                    };
                    // Local paste owns the selection now. Do not echo our own
                    // write back to the user's clipboard as a new remote copy.
                    if successful && mutating && state.backend.supports_clipboard_sync() {
                        observed = state.backend.clipboard_revision().await.ok();
                    }
                    if state.backend.paused() || state.backend.privacy_epoch() != epoch { break; }
                    if let Some(message) = message {
                        timeout(Duration::from_secs(15), writer.send(message)).await??;
                    }
                    continue;
                }
                _ = poll.tick(), if state.backend.supports_clipboard_sync() => {
                    // An ordered text paste runs on the input connection. Wait
                    // until its selection revision is recorded before observing
                    // it, so our own write cannot start a spurious remote copy
                    // and a new client mirror while the app is consuming paste.
                    let _gate = gate.lock.lock().await;
                    let Ok(now) = state.backend.clipboard_revision().await else { continue; };
                    let previous = observed.replace(now);
                    if *gate.local_revision.lock().unwrap_or_else(|e| e.into_inner()) == Some(now) { continue; }
                    let Some(before) = previous.filter(|before| before.epoch == now.epoch && before.serial != now.serial) else { continue; };
                    passive_id += 1;
                    timeout(Duration::from_secs(15), writer.send(Message::Text(serde_json::json!({"type":"clipboard_copy_started","id":passive_id}).to_string().into()))).await??;
                    Some(CopyRequest { id: passive_id, before: Some(before), epoch: state.backend.privacy_epoch() })
                }
            };
            let Some(copy) = copy else {
                break;
            };
            // A new remote selection while a focus mirror is pending supersedes
            // that mirror, even if the user copied the same text as last time.
            // Content-only deduplication would swallow the user's Copy and let
            // the older local value overwrite it when its upload completes.
            let supersedes_mirror = copy.id >= 4_000_000_000_000_000 && mirror.take().is_some();
            let mut changed = false;
            let result = read_copy(&state, &copy, &mut changed, &mut observed).await;
            let result = match result {
                Ok(Some(data)) => match fingerprint(data).await {
                    Ok((data, hash)) => {
                        let mut last_selection = gate
                            .last_selection
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if copy.id >= 4_000_000_000_000_000
                            && !supersedes_mirror
                            && *last_selection == Some(hash)
                        {
                            Ok(None)
                        } else {
                            *last_selection = Some(hash);
                            Ok(Some(data))
                        }
                    }
                    Err(error) => Err(error),
                },
                result => result,
            };
            if state.backend.paused() || state.backend.privacy_epoch() != copy.epoch {
                break;
            }
            let message = match result {
                Ok(Some(CopyData::Text(text))) => {
                    serde_json::json!({"type":"clipboard_copy","id":copy.id,"copied":true,"text":text})
                }
                Ok(Some(CopyData::Image(image))) => {
                    timeout(Duration::from_secs(15), writer.send(Message::Text(serde_json::json!({"type":"clipboard_copy_begin","id":copy.id,"size":image.len(),"mime":"image/png"}).to_string().into()))).await??;
                    for (index, chunk) in image.chunks(MAX_CHUNK).enumerate() {
                        if state.backend.paused() || state.backend.privacy_epoch() != copy.epoch {
                            return anyhow::Ok(());
                        }
                        let mut packet = Vec::with_capacity(chunk.len() + 13);
                        packet.push(5);
                        packet.extend_from_slice(&copy.id.to_le_bytes());
                        packet.extend_from_slice(&((index * MAX_CHUNK) as u32).to_le_bytes());
                        packet.extend_from_slice(chunk);
                        timeout(
                            Duration::from_secs(15),
                            writer.send(Message::Binary(packet.into())),
                        )
                        .await??;
                    }
                    serde_json::json!({"type":"clipboard_copy_end","id":copy.id,"copied":true})
                }
                Ok(None) => {
                    serde_json::json!({"type":"clipboard_copy","id":copy.id,"copied":false})
                }
                Err(error) => {
                    serde_json::json!({"type":"clipboard_copy","id":copy.id,"copied":changed,"error":error.to_string()})
                }
            };
            timeout(
                Duration::from_secs(15),
                writer.send(Message::Text(message.to_string().into())),
            )
            .await??;
        }
        anyhow::Ok(())
    };
    tokio::select! {biased; _ = stop.cancelled() => {}, _ = receive => {}, _ = service => {}}
    stop.cancel();
    drop(lease);
    let _ = timeout(Duration::from_millis(500), writer.close()).await;
}

pub(crate) enum CopyData {
    Text(String),
    Image(Vec<u8>),
}
// Clipboard managers can re-own the same content after serving it. That is
// not another user copy and must not overwrite a newer local clipboard.
pub(crate) async fn fingerprint(data: CopyData) -> Result<(CopyData, u64)> {
    tokio::task::spawn_blocking(move || {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        match &data {
            CopyData::Text(text) => {
                0_u8.hash(&mut hash);
                text.hash(&mut hash);
            }
            CopyData::Image(bytes) => {
                let mut reader =
                    image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(8192);
                limits.max_image_height = Some(8192);
                limits.max_alloc = Some(128 * 1024 * 1024);
                reader.limits(limits);
                let image = reader.decode()?.into_rgba8();
                ensure!(
                    u64::from(image.width()) * u64::from(image.height()) <= 33554432,
                    "clipboard image too large"
                );
                1_u8.hash(&mut hash);
                image.dimensions().hash(&mut hash);
                image.as_raw().hash(&mut hash);
            }
        }
        Ok((data, hash.finish()))
    })
    .await?
}

async fn read_copy(
    state: &crate::server::AppState,
    copy: &CopyRequest,
    changed: &mut bool,
    observed: &mut Option<crate::backend::ClipboardRevision>,
) -> Result<Option<CopyData>> {
    let before = copy
        .before
        .ok_or_else(|| anyhow::anyhow!("clipboard copy could not be observed"))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        ensure!(
            !state.backend.paused()
                && !state.backend.is_stopped()
                && state.backend.privacy_epoch() == copy.epoch,
            "desktop changed during copy"
        );
        let now = state.backend.clipboard_revision().await?;
        ensure!(
            now.epoch == before.epoch,
            "clipboard observer changed during copy"
        );
        if now.serial != before.serial {
            *changed = true;
            tokio::time::sleep(Duration::from_millis(20)).await;
            let revision = state.backend.clipboard_revision().await?;
            ensure!(
                revision.epoch == before.epoch,
                "clipboard observer changed during copy"
            );
            *observed = Some(revision);
            let data = read_selection(state).await?;
            let after = state.backend.clipboard_revision().await?;
            if after != revision {
                ensure!(
                    Instant::now() < deadline,
                    "clipboard kept changing during copy"
                );
                continue;
            }
            return Ok(Some(data));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn read_selection(state: &crate::server::AppState) -> Result<CopyData> {
    if let Some(image) = state.backend.clipboard_read_png().await? {
        ensure!(image.len() <= MAX_IMAGE, "clipboard image too large");
        let (width, height) =
            image::ImageReader::with_format(Cursor::new(&image), image::ImageFormat::Png)
                .into_dimensions()?;
        ensure!(
            width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 33554432,
            "clipboard image dimensions too large"
        );
        return Ok(CopyData::Image(image));
    }
    let text = state.backend.clipboard_read().await?;
    ensure!(
        text.len() <= crate::backend::MAX_CLIPBOARD_BYTES,
        "clipboard text too large"
    );
    Ok(CopyData::Text(text))
}

/// One bounded, ordered image upload belongs to one authenticated controller.
pub(crate) struct ImageUpload {
    id: u64,
    size: usize,
    next: u64,
    bytes: Vec<u8>,
    updated: Instant,
}

impl ImageUpload {
    pub(crate) fn start(id: u64, size: u64) -> Result<Self> {
        ensure!(
            (1..=9_007_199_254_740_991).contains(&id),
            "invalid clipboard transfer ID"
        );
        ensure!(
            (1..=MAX_IMAGE as u64).contains(&size),
            "clipboard image must be at most 8 MiB"
        );
        Ok(Self {
            id,
            size: size as usize,
            next: 1,
            bytes: Vec::new(),
            updated: Instant::now(),
        })
    }

    fn check(&self, id: u64, step: u64) -> Result<()> {
        ensure!(
            id == self.id && step == self.next,
            "clipboard transfer is out of order"
        );
        ensure!(
            self.updated.elapsed() <= Duration::from_secs(15),
            "clipboard transfer expired"
        );
        Ok(())
    }

    pub(crate) fn append(&mut self, id: u64, step: u64, data: &str) -> Result<()> {
        self.check(id, step)?;
        ensure!(
            !data.is_empty() && data.len() <= MAX_CHUNK * 4 / 3,
            "invalid clipboard chunk size"
        );
        let bytes = STANDARD.decode(data)?;
        ensure!(
            self.bytes.len() + bytes.len() <= self.size,
            "clipboard image exceeds declared size"
        );
        self.bytes.extend_from_slice(&bytes);
        self.next += 1;
        self.updated = Instant::now();
        Ok(())
    }

    pub(crate) fn finish(self, id: u64, step: u64) -> Result<Vec<u8>> {
        self.check(id, step)?;
        ensure!(
            self.bytes.len() == self.size,
            "clipboard image is incomplete"
        );
        ensure!(
            self.bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "clipboard image must be PNG"
        );
        let reader =
            image::ImageReader::with_format(Cursor::new(&self.bytes), image::ImageFormat::Png);
        let (width, height) = reader.into_dimensions()?;
        ensure!(
            width > 0
                && height > 0
                && width <= 8192
                && height <= 8192
                && u64::from(width) * u64::from(height) <= 33_554_432,
            "clipboard image dimensions are too large"
        );
        Ok(self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn nonblocking_clipboard_pipe_waits_for_data_and_handles_backpressure() {
        use std::os::fd::{FromRawFd, OwnedFd};
        let mut pipes = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(pipes.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
            0
        );
        let reader = unsafe { OwnedFd::from_raw_fd(pipes[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(pipes[1]) };
        let reading = tokio::spawn(read_fd(reader, 1024 * 1024));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !reading.is_finished(),
            "an empty nonblocking pipe must wait, not fail with EAGAIN"
        );
        let bytes: Vec<_> = (0..512 * 1024).map(|n| (n % 251) as u8).collect();
        tokio::time::timeout(Duration::from_secs(2), write_fd(writer, &bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reading.await.unwrap().unwrap(), bytes);
    }

    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn image_upload_preserves_png_and_requires_order_and_complete_size() {
        let bytes = png();
        let mut upload = ImageUpload::start(1, bytes.len() as u64).unwrap();
        assert!(upload.append(2, 1, &STANDARD.encode(&bytes)).is_err());
        assert!(upload.append(1, 2, &STANDARD.encode(&bytes)).is_err());
        for (i, part) in bytes.chunks(30).enumerate() {
            upload
                .append(1, i as u64 + 1, &STANDARD.encode(part))
                .unwrap();
        }
        let end = upload.next;
        assert_eq!(upload.finish(1, end).unwrap(), bytes);
        assert!(ImageUpload::start(1, 100).unwrap().finish(1, 1).is_err());
    }

    #[test]
    fn invalid_expired_or_oversized_images_are_rejected() {
        assert!(ImageUpload::start(1, MAX_IMAGE as u64 + 1).is_err());
        assert!(ImageUpload::start(0, 1).is_err());
        let mut upload = ImageUpload::start(1, 1).unwrap();
        assert!(upload.append(1, 1, "%%%=").is_err());
        assert!(upload.append(1, 1, &STANDARD.encode([1, 2])).is_err());
        assert!(
            upload
                .append(1, 1, &"A".repeat(MAX_CHUNK * 4 / 3 + 4))
                .is_err()
        );
        upload.updated = Instant::now() - Duration::from_secs(16);
        assert!(upload.append(1, 1, "AA==").is_err());
        let mut upload = ImageUpload::start(1, 1).unwrap();
        upload.append(1, 1, "AA==").unwrap();
        assert!(upload.finish(1, 2).is_err());
    }

    #[test]
    fn chords_are_bounded_and_cannot_leave_duplicate_key_state() {
        use crate::backend::InputCommand;
        assert!(
            InputCommand::Chord {
                codes: vec![29, 47]
            }
            .validate()
            .is_ok()
        );
        for codes in [vec![], vec![29, 29], vec![0], vec![125; 9], vec![u32::MAX]] {
            assert!(InputCommand::Chord { codes }.validate().is_err());
        }
    }
}
