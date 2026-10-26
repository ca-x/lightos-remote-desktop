//! Capture, input and clipboard contracts shared by desktop backends.
//!
//! Session discovery and capability-based selection are separate from display
//! management and distribution-specific initialization.
use anyhow::{Result, bail, ensure};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::sync::Mutex;
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc as sync_mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "linux")]
pub mod guarded;
#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(target_os = "linux")]
pub mod selector;
#[cfg(target_os = "linux")]
pub mod x11;

pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_EVDEV_CODE: u32 = 767;
pub const MAX_CLIPBOARD_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Wayland,
    Portal,
    X11,
    #[cfg(target_os = "windows")]
    Windows,
    #[cfg(target_os = "macos")]
    Macos,
}

pub trait DesktopBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn paste_keycodes(&self, shift: bool) -> Vec<u32> {
        if shift {
            vec![29, 42, 47]
        } else {
            vec![29, 47]
        }
    }
    /// Whether captured pixels already include the pointer cursor.
    fn cursor_embedded(&self) -> bool {
        false
    }
    fn begin_cursor(&self, _owner: u64) {}
    fn separate_cursor(&self, _owner: u64) {}
    fn release_cursor(&self, _owner: u64) {}
    fn cursor_target(&self) -> Result<(std::path::PathBuf, i32, Option<String>)> {
        bail!("independent cursor unavailable for this desktop backend")
    }
    fn supports_image_clipboard(&self) -> bool {
        false
    }
    fn clipboard_write_png<'a>(&'a self, _data: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { bail!("image clipboard unavailable for this desktop backend") })
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>>;
    fn input(&self) -> mpsc::Sender<InputCommand>;
    fn stop(&self);
    /// A temporary privacy/permission pause keeps the transport alive while
    /// withholding frames. A stopped backend instead ends the stream.
    fn paused(&self) -> bool {
        false
    }
    fn is_stopped(&self) -> bool {
        false
    }
    /// An attached Portal session that awaits consent from a browser controller.
    fn is_deferred(&self) -> bool {
        false
    }
    /// Changes whenever previously captured physical pixels must be discarded.
    fn privacy_epoch(&self) -> u64 {
        0
    }

    /// An acknowledgement means all input held by this backend was released.
    /// A failed barrier requires stopping the backend and barring new controllers
    /// until the host process has been recovered by its supervisor.
    fn release_input(&self) -> bool {
        let (done, completed) = sync_mpsc::sync_channel(1);
        self.input()
            .try_send(InputCommand::ReleaseSync { done })
            .is_ok()
            && completed.recv_timeout(Duration::from_secs(1)).is_ok()
    }
    fn frame_is_fresh(&self, max_age: Duration) -> bool {
        self.frames()
            .borrow()
            .as_ref()
            .is_some_and(|frame| frame.captured_at.elapsed() <= max_age)
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async { bail!("clipboard read unavailable for this desktop backend") })
    }
    fn clipboard_write<'a>(&'a self, _text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { bail!("clipboard write unavailable for this desktop backend") })
    }
}

#[cfg(target_os = "linux")]
pub use selector::{probe, start_auto, start_selected};

#[derive(Clone, Debug, Serialize)]
pub struct BackendAvailability {
    pub backend: BackendKind,
    pub available: bool,
    pub requires_consent: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BackendReport {
    /// A candidate is not ready until it has produced its first validated frame.
    pub selected: Option<BackendKind>,
    pub candidates: Vec<BackendAvailability>,
}
impl BackendReport {
    pub fn require_available(&self) -> Result<BackendKind> {
        self.selected.ok_or_else(|| {
            anyhow::anyhow!(
                "desktop capture/input unavailable: {}",
                self.candidates
                    .iter()
                    .filter_map(|candidate| candidate.reason.as_deref())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
    }
    pub fn requires_consent(&self) -> bool {
        self.candidates
            .iter()
            .any(|candidate| Some(candidate.backend) == self.selected && candidate.requires_consent)
    }
}

/// A transport placeholder for physical Portal sessions. It owns no screen or
/// input device and survives failed browser authorization for a later request.
pub struct DeferredPortalBackend {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    input: mpsc::Sender<InputCommand>,
    stopped: AtomicBool,
}

impl DeferredPortalBackend {
    pub fn new() -> Self {
        let (_, frames) = watch::channel(None);
        let (input, _) = mpsc::channel(1);
        Self {
            frames,
            input,
            stopped: AtomicBool::new(false),
        }
    }
}

impl Default for DeferredPortalBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopBackend for DeferredPortalBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Portal
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    fn paused(&self) -> bool {
        true
    }
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    fn is_deferred(&self) -> bool {
        true
    }
    fn release_input(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub(crate) jpeg: OnceLock<(u8, Arc<[u8]>)>,
    pub pixels: Arc<[u8]>,
    pub captured_at: Instant,
    pub cursor_embedded: bool,
}

impl Frame {
    pub fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Self {
        Self {
            width,
            height,
            pixels,
            jpeg: OnceLock::new(),
            captured_at: Instant::now(),
            cursor_embedded: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_frame(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            jpeg: OnceLock::new(),
            pixels: [24, 64, 128, 255].repeat((width * height) as usize).into(),
            captured_at: Instant::now(),
            cursor_embedded: false,
        }
    }

    pub fn jpeg(&self, quality: u8) -> Result<Arc<[u8]>> {
        ensure!(
            self.pixels.len() == self.width as usize * self.height as usize * 4,
            "invalid JPEG pixels"
        );
        if let Some((cached_quality, data)) = self.jpeg.get()
            && *cached_quality == quality
        {
            return Ok(data.clone());
        }
        let data = media_codec::encode_jpeg(self.width, self.height, &self.pixels, quality)?;
        let _ = self.jpeg.set((quality, data.clone()));
        Ok(data)
    }
}

/// Internal input authorization survives the async queue and Unicode worker.
#[derive(Debug, Clone)]
pub struct InputGuard {
    pub(crate) cancellation: CancellationToken,
    pub(crate) lock_cancellation: Option<CancellationToken>,
    pub(crate) deadline: Instant,
    pub(crate) text_done: Arc<tokio::sync::Notify>,
    #[cfg(test)]
    pub(crate) probe: Option<Arc<TextProbe>>,
}
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct TextProbe {
    pub(crate) characters: std::sync::atomic::AtomicUsize,
    pub(crate) batches: std::sync::atomic::AtomicUsize,
    pub(crate) restored: AtomicBool,
    pub(crate) first_batch: sync_mpsc::SyncSender<()>,
    pub(crate) resume: Mutex<sync_mpsc::Receiver<()>>,
}
impl InputGuard {
    pub(crate) fn new(cancellation: CancellationToken, deadline: Instant) -> Self {
        Self {
            cancellation,
            lock_cancellation: None,
            deadline,
            text_done: Arc::new(tokio::sync::Notify::new()),
            #[cfg(test)]
            probe: None,
        }
    }
    pub(crate) fn active(&self) -> bool {
        !self.cancellation.is_cancelled()
            && !self
                .lock_cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            && Instant::now() < self.deadline
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputCommand {
    Move {
        x: f64,
        y: f64,
    },
    Button {
        button: u32,
        pressed: bool,
    },
    Key {
        code: u32,
        pressed: bool,
    },
    Chord {
        codes: Vec<u32>,
    },
    Scroll {
        x: f64,
        y: f64,
    },
    Release,
    #[serde(skip)]
    Guarded {
        command: Box<InputCommand>,
        guard: InputGuard,
    },
    #[serde(skip)]
    ReleaseSync {
        done: sync_mpsc::SyncSender<()>,
    },
    Text {
        text: String,
    },
}

impl InputCommand {
    pub(crate) fn into_active(self) -> Option<(Self, Option<InputGuard>)> {
        match self {
            Self::Guarded { command, guard } => {
                if guard.active() {
                    Some((*command, Some(guard)))
                } else {
                    guard.text_done.notify_one();
                    None
                }
            }
            command => Some((command, None)),
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Move { x, y } => ensure!(
                x.is_finite()
                    && y.is_finite()
                    && (0.0..=1.0).contains(x)
                    && (0.0..=1.0).contains(y),
                "pointer coordinates must be finite and normalized to 0..1"
            ),
            Self::Button { button, .. } => {
                ensure!(*button <= 2, "unsupported mouse button");
            }
            Self::Key { code, .. } => {
                ensure!((1..=MAX_EVDEV_CODE).contains(code), "invalid evdev keycode");
            }
            Self::Chord { codes } => {
                ensure!(!codes.is_empty() && codes.len() <= 8, "invalid key chord");
                ensure!(
                    codes.iter().all(|code| (1..=MAX_EVDEV_CODE).contains(code)),
                    "invalid chord keycode"
                );
                ensure!(
                    codes
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == codes.len(),
                    "duplicate chord keycode"
                );
            }
            Self::Scroll { x, y } => ensure!(
                x.is_finite() && y.is_finite() && x.abs() <= 1000.0 && y.abs() <= 1000.0,
                "scroll coordinates must be finite and within -1000..1000"
            ),
            Self::Text { text } => {
                ensure!(text.len() <= MAX_TEXT_BYTES, "text exceeds 16 KiB");
                ensure!(
                    text.chars().all(|character| !character.is_control()
                        || matches!(character, '\n' | '\r' | '\t')),
                    "unsupported text control character"
                );
            }
            Self::Guarded { command, .. } => command.validate()?,
            Self::Release | Self::ReleaseSync { .. } => {}
        }
        Ok(())
    }
}
