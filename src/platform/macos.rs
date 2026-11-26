//! Capture the logged-in macOS console, including the system lock screen.
#[path = "macos_cursor.rs"]
mod macos_cursor;
#[path = "macos_service.rs"]
mod macos_service;
pub use macos_service::install_service;

use std::{
    collections::VecDeque,
    ffi::{CStr, c_char, c_void},
    io,
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use display_control::DisplayOutput;
use futures_util::future::BoxFuture;
use screencapturekit::{cm::SCFrameStatus, prelude::*};

use super::{
    NativeCapture, NativePlatform, NativeSurface, PermissionProvider, PermissionState, Permissions,
};
use crate::{
    audio::{AudioCapture, AudioCaptureFactory, AudioFormat, AudioStream, PcmWriter},
    backend::{BackendKind, Frame},
};

pub fn provider() -> Arc<dyn NativePlatform> {
    Arc::new(MacosPlatform)
}

struct MacosPlatform;

#[derive(Clone, Copy)]
pub(super) enum InputPermission {
    Accessibility,
    DeviceControl,
    PostEvent,
}

impl InputPermission {
    pub(super) fn setting(self) -> &'static str {
        match self {
            Self::Accessibility => "accessibility",
            Self::DeviceControl => "device_control_data_access",
            Self::PostEvent => "system_app_protection_override",
        }
    }

    fn granted(self) -> bool {
        match self {
            Self::PostEvent => unsafe { CGPreflightPostEventAccess() },
            Self::Accessibility | Self::DeviceControl => unsafe { AXIsProcessTrusted() },
        }
    }
}

pub(super) fn input_permission() -> InputPermission {
    static PERMISSION: OnceLock<InputPermission> = OnceLock::new();
    *PERMISSION.get_or_init(|| {
        let version = os_release(c"kern.osproductversion").unwrap_or_default();
        let mut parts = version.split('.');
        let major = parts.next().and_then(|part| part.parse::<u32>().ok());
        let minor = parts.next().and_then(|part| part.parse::<u32>().ok());
        match (major, minor) {
            // The first 27.3 beta still had the combined permission.
            (Some(27), Some(3)) if os_release(c"kern.osversion").as_deref() == Some("26C5035r") => {
                InputPermission::DeviceControl
            }
            (Some(27), Some(3..)) | (Some(28..), _) => InputPermission::PostEvent,
            (Some(27), _) => InputPermission::DeviceControl,
            _ => InputPermission::Accessibility,
        }
    })
}

fn os_release(name: &CStr) -> Option<String> {
    let mut buffer = [0u8; 64];
    let mut len = buffer.len();
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    Some(
        CStr::from_bytes_until_nul(&buffer)
            .ok()?
            .to_str()
            .ok()?
            .to_owned(),
    )
}

pub(super) fn open_input(settings: &enigo::Settings) -> Result<enigo::Enigo> {
    Ok(match input_permission() {
        InputPermission::PostEvent => enigo::Enigo::new_with_post_event_permission(settings)?,
        InputPermission::Accessibility | InputPermission::DeviceControl => {
            enigo::Enigo::new(settings)?
        }
    })
}

/// Explicit consent entry point for desktop integrations. Capability probes and
/// the doctor command continue to call the read-only PermissionProvider method.
pub fn permissions(request: bool) -> Result<Permissions> {
    use core_foundation_sys::{
        base::{CFRelease, kCFAllocatorDefault},
        dictionary::{
            CFDictionaryCreate, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
        },
        number::kCFBooleanTrue,
    };
    let platform = MacosPlatform;
    let before = platform.permissions()?;
    if request {
        ensure!(
            GuiSession::current()?.active,
            "permission requests require an unlocked GUI session"
        );
        let post_event = matches!(input_permission(), InputPermission::PostEvent);
        let missing_input = !matches!(before.input, PermissionState::Granted);
        let input_pending = post_event && missing_input && !unsafe { CGRequestPostEventAccess() };
        // Dispatch the asynchronous legacy Accessibility prompt first.
        if !post_event && missing_input {
            // Both keys and values are retained with the CoreFoundation callbacks.
            let key = unsafe { kAXTrustedCheckOptionPrompt };
            let value = unsafe { kCFBooleanTrue } as *const c_void;
            let options = unsafe {
                CFDictionaryCreate(
                    kCFAllocatorDefault,
                    &key,
                    &value,
                    1,
                    &raw const kCFTypeDictionaryKeyCallBacks,
                    &raw const kCFTypeDictionaryValueCallBacks,
                )
            };
            ensure!(
                !options.is_null(),
                "cannot create accessibility consent options"
            );
            unsafe {
                AXIsProcessTrustedWithOptions(options.cast());
                CFRelease(options.cast());
            }
        }
        let screen_pending = !matches!(before.screen, PermissionState::Granted)
            && !unsafe { CGRequestScreenCaptureAccess() };
        if post_event && (input_pending || screen_pending) {
            // On 27.3 Beta 2, the legacy Accessibility anchor opens System & App
            // Protection Override. If both grants are pending, start with input.
            let anchor = if input_pending {
                "Privacy_Accessibility"
            } else {
                "Privacy_ScreenCapture"
            };
            let opened = Command::new("/usr/bin/open")
                .arg(format!(
                    "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?{anchor}"
                ))
                .stdout(Stdio::null())
                .status();
            if !opened.is_ok_and(|status| status.success()) {
                tracing::warn!(anchor, "could not open the macOS permission settings");
            }
        }
    }
    platform.permissions()
}

impl PermissionProvider for MacosPlatform {
    fn permissions(&self) -> Result<Permissions> {
        let screen = unsafe { CGPreflightScreenCaptureAccess() };
        let input = input_permission().granted();
        Ok(Permissions {
            screen: if screen { PermissionState::Granted } else { PermissionState::NeedsUserAction },
            input: if input { PermissionState::Granted } else { PermissionState::NeedsUserAction },
            reason: (!screen || !input).then(|| {
                "Approve the missing screen or keyboard and mouse permissions in Privacy & Security".into()
            }),
        })
    }
}

impl NativePlatform for MacosPlatform {
    fn supports_cursor(&self) -> bool {
        true
    }
    fn cursor_frame(&self, output: &DisplayOutput) -> Result<Option<crate::backend::CursorFrame>> {
        macos_cursor::capture(output)
    }
    fn kind(&self) -> BackendKind {
        BackendKind::Macos
    }

    fn session_key(&self) -> Result<String> {
        let session = GuiSession::current()?;
        Ok(format!("macos:{}:{}", session.uid, session.id))
    }

    fn session_active(&self) -> Result<bool> {
        Ok(GuiSession::current()?.active)
    }
    fn session_surface(&self, key: &str) -> Result<Option<NativeSurface>> {
        let session = GuiSession::current()?;
        ensure!(
            format!("macos:{}:{}", session.uid, session.id) == key,
            "interactive user session changed"
        );
        Ok(session.surface().filter(|_| {
            self.permissions()
                .is_ok_and(|p| p.require_granted().is_ok())
        }))
    }

    fn displays(&self) -> Result<Vec<DisplayOutput>> {
        let surface = GuiSession::current()?
            .surface()
            .context("logged-in console desktop is unavailable")?;
        self.permissions()?.require_granted()?;
        if surface == NativeSurface::LockScreen {
            return Ok(vec![main_display()?]);
        }
        let content = SCShareableContent::get().context("query ScreenCaptureKit displays")?;
        content.displays().iter().map(display_output).collect()
    }

    fn display_for_surface(
        &self,
        output: &DisplayOutput,
        surface: NativeSurface,
    ) -> Result<DisplayOutput> {
        if surface == NativeSurface::LockScreen {
            return main_display();
        }
        self.refresh_display(output)
    }

    fn lock_display_asleep(&self) -> bool {
        session_locked() && unsafe { CGDisplayIsAsleep(CGMainDisplayID()) } != 0
    }

    fn wake_lock_display(&self) -> Result<()> {
        if !self.lock_display_asleep() {
            return Ok(());
        }
        static ASSERTION: Mutex<u32> = Mutex::new(0);
        let name = unsafe {
            CFStringCreateWithCString(
                std::ptr::null(),
                c"Lazycat Remote Desktop".as_ptr(),
                0x08000100,
            )
        };
        ensure!(
            !name.is_null(),
            "could not name the remote display wake request"
        );
        let mut assertion = ASSERTION.lock().unwrap_or_else(|error| error.into_inner());
        // kIOPMUserActiveRemote requests a timed display wake for a connected viewer.
        let result = unsafe { IOPMAssertionDeclareUserActivity(name, 1, &mut *assertion) };
        unsafe { CFRelease(name) };
        ensure!(
            result == 0,
            "macOS could not wake the lock display: {result}"
        );
        Ok(())
    }

    fn refresh_display(&self, previous: &DisplayOutput) -> Result<DisplayOutput> {
        // Re-enumerating SCShareableContent synchronously can block capture on
        // WindowServer. Geometry/mode changes need only the selected CG display.
        let id = previous
            .name
            .strip_prefix("display-")
            .context("invalid macOS display identifier")?
            .parse::<u32>()?;
        ensure!(
            unsafe { CGDisplayIsActive(id) } != 0,
            "selected display disconnected"
        );
        let bounds = unsafe { CGDisplayBounds(id) };
        display_geometry(id, bounds)
    }

    fn capture(
        &self,
        output: &DisplayOutput,
        fps: u32,
        surface: NativeSurface,
    ) -> Result<Box<dyn NativeCapture>> {
        ensure!((1..=120).contains(&fps), "capture FPS must be 1..=120");
        let session = GuiSession::current()?;
        ensure!(
            session.surface() == Some(surface),
            "macOS console changed during capture initialization"
        );
        self.permissions()?.require_granted()?;
        let session_id = session.id;
        let display = selected_display(output)?;
        let current = display_output(&display)?;
        ensure!(
            current.width == output.width && current.height == output.height,
            "selected display resolution changed"
        );
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build()?;
        let config = SCStreamConfiguration::new()
            .with_width(output.width)
            .with_height(output.height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(false)
            .with_scales_to_fit(false)
            .with_queue_depth(4)
            .with_fps(fps);
        let events = Arc::new(CaptureEvents::default());
        let mut stream =
            SCStream::new_with_delegate(&filter, &config, CaptureDelegate(events.clone()))?;
        let output_events = events.clone();
        let expected_size = (output.width, output.height);
        let opened_at = Instant::now();
        stream.add_output_handler(
            move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
                if kind != SCStreamOutputType::Screen {
                    return;
                }
                if !capture_allowed(session_id, surface).unwrap_or(false) {
                    output_events.clear_frame();
                    return;
                }
                match sample.frame_status() {
                    Some(SCFrameStatus::Complete | SCFrameStatus::Started) => {
                        if let Err(error) = copy_frame(
                            &sample,
                            expected_size,
                            session_id,
                            surface,
                            opened_at,
                            &output_events,
                        ) {
                            output_events.fail(format!("ScreenCaptureKit frame: {error:#}"));
                        }
                    }
                    Some(SCFrameStatus::Stopped | SCFrameStatus::Suspended) => {
                        output_events.fail("ScreenCaptureKit stream stopped or suspended".into());
                    }
                    Some(SCFrameStatus::Idle | SCFrameStatus::Blank) => {}
                    other => output_events.fail(format!("ScreenCaptureKit frame status {other:?}")),
                }
            },
            SCStreamOutputType::Screen,
        )?;
        stream
            .start_capture()
            .context("start ScreenCaptureKit screen stream")?;
        Ok(Box::new(MacCapture {
            stream,
            events,
            session_id,
            surface,
        }))
    }

    fn audio(&self) -> Arc<dyn AudioCaptureFactory> {
        Arc::new(SckAudio {
            available: probe_audio().is_ok(),
        })
    }
}

fn display_output(display: &SCDisplay) -> Result<DisplayOutput> {
    display_geometry(display.display_id(), display.frame())
}

fn main_display() -> Result<DisplayOutput> {
    let id = unsafe { CGMainDisplayID() };
    display_geometry(id, unsafe { CGDisplayBounds(id) })
}

fn display_geometry(id: u32, frame: CGRect) -> Result<DisplayOutput> {
    // ScreenCaptureKit's display dimensions follow the mode's logical size.
    // Configure capture in backing pixels so Retina content is not first
    // downsampled to 1x and then enlarged by the viewing browser.
    let mode = unsafe { CGDisplayCopyDisplayMode(id) };
    ensure!(!mode.is_null(), "query macOS display backing scale");
    let (logical_width, logical_height, pixel_width, pixel_height) = unsafe {
        let dimensions = (
            CGDisplayModeGetWidth(mode),
            CGDisplayModeGetHeight(mode),
            CGDisplayModeGetPixelWidth(mode),
            CGDisplayModeGetPixelHeight(mode),
        );
        CGDisplayModeRelease(mode);
        dimensions
    };
    ensure!(
        logical_width > 0 && logical_height > 0,
        "invalid macOS display mode"
    );
    let backing_x = pixel_width as f64 / logical_width as f64;
    let backing_y = pixel_height as f64 / logical_height as f64;
    ensure!(
        (1.0..=4.0).contains(&backing_x) && (backing_x - backing_y).abs() < 0.05,
        "invalid macOS backing scale"
    );
    let width = (frame.size.width * backing_x).round() as u32;
    let height = (frame.size.height * backing_y).round() as u32;
    ensure!(
        width > 0 && height > 0 && frame.size.width > 0.0 && frame.size.height > 0.0,
        "invalid macOS display dimensions"
    );
    let (sx, sy) = (
        f64::from(width) / frame.size.width,
        f64::from(height) / frame.size.height,
    );
    ensure!(
        sx.is_finite() && sy.is_finite() && (sx - sy).abs() < 0.05,
        "macOS display has inconsistent pixel scale"
    );
    ensure!(
        frame.origin.x.is_finite()
            && frame.origin.y.is_finite()
            && frame.origin.x >= f64::from(i32::MIN)
            && frame.origin.x <= f64::from(i32::MAX)
            && frame.origin.y >= f64::from(i32::MIN)
            && frame.origin.y <= f64::from(i32::MAX),
        "invalid macOS display origin"
    );
    Ok(DisplayOutput {
        name: format!("display-{id}"),
        description: format!("macOS display {id}"),
        width,
        height,
        x: frame.origin.x.round() as i32,
        y: frame.origin.y.round() as i32,
        scale: sx,
    })
}

fn selected_display(output: &DisplayOutput) -> Result<SCDisplay> {
    let id: u32 = output
        .name
        .strip_prefix("display-")
        .context("invalid macOS display name")?
        .parse()?;
    SCShareableContent::get()?
        .displays()
        .into_iter()
        .find(|display| display.display_id() == id)
        .context("selected macOS display unavailable")
}

#[derive(Default)]
struct CaptureState {
    frame: Option<Frame>,
    error: Option<String>,
}
#[derive(Default)]
struct CaptureEvents {
    state: Mutex<CaptureState>,
    ready: Condvar,
}
impl CaptureEvents {
    fn clear_frame(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.frame = None;
        self.ready.notify_all();
    }

    fn fail(&self, error: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.error.get_or_insert(error);
        state.frame = None;
        self.ready.notify_all();
    }
}
struct CaptureDelegate(Arc<CaptureEvents>);
impl SCStreamDelegateTrait for CaptureDelegate {
    fn did_stop_with_error(&self, error: SCError) {
        self.0
            .fail(format!("ScreenCaptureKit stream stopped: {error}"));
    }
}

fn copy_frame(
    sample: &CMSampleBuffer,
    expected_size: (u32, u32),
    session_id: i64,
    surface: NativeSurface,
    opened_at: Instant,
    events: &CaptureEvents,
) -> Result<()> {
    let buffer = sample
        .pixel_buffer()
        .context("screen sample has no pixel buffer")?;
    let width = u32::try_from(buffer.width())?;
    let height = u32::try_from(buffer.height())?;
    ensure!(
        (width, height) == expected_size,
        "captured display resolution changed"
    );
    if !capture_allowed(session_id, surface).unwrap_or(false) {
        events.clear_frame();
        return Ok(());
    }
    let mut state = events.state.lock().unwrap_or_else(|e| e.into_inner());
    if state.error.is_none() {
        let mut frame = Frame::new(width, height, Arc::from([]));
        let age = unsafe { media_codec::videotoolbox::lrd_capture_age(sample.as_ptr()) };
        let captured_at = (0.0..=60.0)
            .contains(&age)
            .then(|| Instant::now().checked_sub(Duration::from_secs_f64(age)))
            .flatten();
        if surface == NativeSurface::LockScreen && captured_at.is_none_or(|time| time < opened_at) {
            return Ok(());
        }
        frame.native = Some(buffer);
        frame.captured_at = captured_at.unwrap_or(frame.captured_at);
        frame.cursor_embedded = false;
        state.frame = Some(frame);
        events.ready.notify_one();
    }
    Ok(())
}

#[cfg(test)]
fn padded_bgra_to_rgba(source: &[u8], width: u32, height: u32, stride: usize) -> Result<Vec<u8>> {
    ensure!(
        (1..=16384).contains(&width) && (1..=16384).contains(&height),
        "invalid captured screen size"
    );
    let row = usize::try_from(width)?
        .checked_mul(4)
        .context("screen row overflow")?;
    let height = usize::try_from(height)?;
    let required = stride
        .checked_mul(height - 1)
        .and_then(|prefix| prefix.checked_add(row))
        .context("screen buffer overflow")?;
    ensure!(
        stride >= row && source.len() >= required,
        "short or invalid padded pixel buffer"
    );
    let mut rgba = vec![0; row * height];
    for y in 0..height {
        let input = &source[y * stride..y * stride + row];
        let output = &mut rgba[y * row..(y + 1) * row];
        for (bgra, rgba) in input
            .as_chunks::<4>()
            .0
            .iter()
            .zip(output.as_chunks_mut::<4>().0.iter_mut())
        {
            rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    Ok(rgba)
}

struct MacCapture {
    stream: SCStream,
    events: Arc<CaptureEvents>,
    session_id: i64,
    surface: NativeSurface,
}
impl NativeCapture for MacCapture {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        if !capture_allowed(self.session_id, self.surface).unwrap_or(false) {
            self.events.clear_frame();
            return Ok(None);
        }
        let deadline = Instant::now() + timeout;
        let mut state = self.events.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(error) = &state.error {
                bail!("{error}");
            }
            if let Some(frame) = state.frame.take() {
                drop(state);
                if !capture_allowed(self.session_id, self.surface).unwrap_or(false) {
                    self.events.clear_frame();
                    return Ok(None);
                }
                return Ok(Some(frame));
            }
            let now = Instant::now();
            if now >= deadline {
                drop(state);
                if !capture_allowed(self.session_id, self.surface).unwrap_or(false) {
                    self.events.clear_frame();
                }
                return Ok(None);
            }
            state = self
                .events
                .ready
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
impl Drop for MacCapture {
    fn drop(&mut self) {
        let _ = self.stream.stop_capture();
    }
}

struct SckAudio {
    available: bool,
}
impl AudioCaptureFactory for SckAudio {
    fn available(&self) -> bool {
        self.available && audio_allowed().unwrap_or(false)
    }
    fn start(&self, format: AudioFormat) -> Result<Box<dyn AudioCapture>> {
        ensure!(
            self.available(),
            "ScreenCaptureKit system audio is unavailable in this GUI session"
        );
        let stream = AudioStream::capture("sck-system-audio", format, run_audio)?;
        Ok(Box::new(GuardedAudio(Some(stream))))
    }
}

fn audio_allowed() -> Result<bool> {
    let session = GuiSession::current()?;
    capture_allowed(session.id, NativeSurface::Desktop)
}

fn capture_allowed(session_id: i64, surface: NativeSurface) -> Result<bool> {
    let session = GuiSession::current()?;
    Ok(session.surface() == Some(surface)
        && session.id == session_id
        && unsafe { CGPreflightScreenCaptureAccess() }
        && input_permission().granted())
}

#[derive(Default)]
struct AudioState {
    packets: VecDeque<Vec<u8>>,
    error: Option<String>,
}
#[derive(Default)]
struct AudioEvents {
    state: Mutex<AudioState>,
    ready: Condvar,
}
impl AudioEvents {
    fn fail(&self, error: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.error.get_or_insert(error);
        state.packets.clear();
        self.ready.notify_all();
    }
}
struct AudioDelegate(Arc<AudioEvents>);
impl SCStreamDelegateTrait for AudioDelegate {
    fn did_stop_with_error(&self, error: SCError) {
        self.0
            .fail(format!("ScreenCaptureKit audio stream stopped: {error}"));
    }
}

fn audio_stream(events: Arc<AudioEvents>) -> Result<SCStream> {
    let display = SCShareableContent::get()?
        .displays()
        .into_iter()
        .next()
        .context("no display for ScreenCaptureKit system audio")?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build()?;
    let config = SCStreamConfiguration::new()
        .with_width(64)
        .with_height(64)
        .with_fps(1)
        .with_shows_cursor(false)
        .with_captures_audio(true)
        .with_sample_rate(48_000)
        .with_channel_count(2);
    let mut stream = SCStream::new_with_delegate(&filter, &config, AudioDelegate(events.clone()))?;
    stream.add_output_handler(
        |_: CMSampleBuffer, _: SCStreamOutputType| {},
        SCStreamOutputType::Screen,
    )?;
    stream.add_output_handler(
        move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
            if kind != SCStreamOutputType::Audio {
                return;
            }
            if !audio_allowed().unwrap_or(false) {
                events.fail("GUI session locked or screen permission revoked".into());
                return;
            }
            match copy_audio(&sample) {
                Ok(Some(pcm)) => {
                    let mut state = events.state.lock().unwrap_or_else(|e| e.into_inner());
                    if state.error.is_none() {
                        if state.packets.len() == 8 {
                            state.packets.pop_front();
                        }
                        state.packets.push_back(pcm);
                        events.ready.notify_one();
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    events.fail(format!("invalid ScreenCaptureKit system audio: {error:#}"))
                }
            }
        },
        SCStreamOutputType::Audio,
    )?;
    Ok(stream)
}

fn copy_audio(sample: &CMSampleBuffer) -> Result<Option<Vec<u8>>> {
    let format = sample
        .format_description()
        .context("audio sample has no format")?;
    ensure!(
        format.audio_sample_rate() == Some(48_000.0)
            && format.audio_channel_count() == Some(2)
            && format.audio_is_float()
            && !format.audio_is_big_endian()
            && format.audio_bits_per_channel() == Some(32),
        "expected 48 kHz stereo little-endian floating point audio"
    );
    let buffers = sample
        .audio_buffer_list()
        .map_err(|e| anyhow::anyhow!("audio buffer list: {e}"))?;
    match buffers.num_buffers() {
        1 => {
            let buffer = buffers.get(0).context("missing audio buffer")?;
            ensure!(
                buffer.number_channels == 2
                    && format.audio_bytes_per_frame() == Some(8)
                    && buffer.data().len() <= 1024 * 1024
                    && buffer.data().len().is_multiple_of(8),
                "invalid interleaved stereo audio buffer"
            );
            Ok((!buffer.data().is_empty()).then(|| buffer.data().to_vec()))
        }
        2 => {
            let left = buffers.get(0).context("missing left audio buffer")?;
            let right = buffers.get(1).context("missing right audio buffer")?;
            ensure!(
                left.number_channels == 1
                    && right.number_channels == 1
                    && format.audio_bytes_per_frame() == Some(4),
                "invalid planar stereo audio format"
            );
            interleave_planar(left.data(), right.data())
        }
        _ => bail!("unsupported ScreenCaptureKit audio buffer layout"),
    }
}

fn interleave_planar(left: &[u8], right: &[u8]) -> Result<Option<Vec<u8>>> {
    ensure!(
        left.len() == right.len() && left.len() <= 512 * 1024 && left.len().is_multiple_of(4),
        "invalid planar stereo audio buffer"
    );
    if left.is_empty() {
        return Ok(None);
    }
    let mut interleaved = Vec::with_capacity(left.len() * 2);
    for (left, right) in left
        .as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
    {
        interleaved.extend_from_slice(left);
        interleaved.extend_from_slice(right);
    }
    Ok(Some(interleaved))
}

static AUDIO_PROBE_BUSY: AtomicBool = AtomicBool::new(false);

fn probe_audio() -> Result<()> {
    ensure!(
        AUDIO_PROBE_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok(),
        "ScreenCaptureKit audio probe is already running or awaiting OS recovery"
    );
    let (reply, response) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("macos-audio-probe".into())
        .spawn(move || {
            struct Release;
            impl Drop for Release {
                fn drop(&mut self) {
                    AUDIO_PROBE_BUSY.store(false, Ordering::Release);
                }
            }
            let release = Release;
            let result = probe_audio_inner();
            drop(release);
            let _ = reply.send(result);
        });
    if let Err(error) = worker {
        AUDIO_PROBE_BUSY.store(false, Ordering::Release);
        return Err(error).context("spawn ScreenCaptureKit audio probe");
    }
    match response.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            bail!("ScreenCaptureKit audio probe timed out")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            bail!("ScreenCaptureKit audio probe worker stopped")
        }
    }
}

fn probe_audio_inner() -> Result<()> {
    ensure!(
        audio_allowed()?,
        "macOS GUI session or screen permission unavailable"
    );
    let events = Arc::new(AudioEvents::default());
    let stream = audio_stream(events.clone())?;
    stream
        .start_capture()
        .context("start ScreenCaptureKit audio probe")?;
    let state = events.state.lock().unwrap_or_else(|e| e.into_inner());
    let state = events
        .ready
        .wait_timeout(state, Duration::from_millis(150))
        .unwrap_or_else(|e| e.into_inner())
        .0;
    let error = state.error.clone();
    drop(state);
    stream
        .stop_capture()
        .context("stop ScreenCaptureKit audio probe")?;
    if let Some(error) = error {
        bail!("{error}");
    }
    Ok(())
}

fn run_audio(stop: &AtomicBool, writer: &mut PcmWriter) -> Result<()> {
    ensure!(
        audio_allowed()?,
        "macOS GUI session or screen permission unavailable"
    );
    let events = Arc::new(AudioEvents::default());
    let stream = audio_stream(events.clone())?;
    stream
        .start_capture()
        .context("start ScreenCaptureKit system audio")?;
    let result = (|| -> Result<()> {
        while !stop.load(Ordering::Acquire) {
            if !audio_allowed()? {
                break;
            }
            let mut state = events.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(error) = &state.error {
                bail!("{error}");
            }
            if state.packets.is_empty() {
                state = events
                    .ready
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            if let Some(error) = &state.error {
                bail!("{error}");
            }
            let packet = state.packets.pop_front();
            drop(state);
            if stop.load(Ordering::Acquire) || !audio_allowed()? {
                break;
            }
            if let Some(packet) = packet {
                writer.push(&packet)?;
            }
        }
        Ok(())
    })();
    events
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .packets
        .clear();
    let stop_result = stream
        .stop_capture()
        .context("stop ScreenCaptureKit system audio");
    result.and(stop_result)
}

struct GuardedAudio(Option<AudioStream>);
impl AudioCapture for GuardedAudio {
    fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> BoxFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            if !audio_allowed().unwrap_or(false) {
                self.0.take();
                return Ok(0);
            }
            let Some(stream) = &mut self.0 else {
                return Ok(0);
            };
            let count = stream.read(buffer).await?;
            if !audio_allowed().unwrap_or(false) {
                buffer[..count].fill(0);
                self.0.take();
                return Ok(0);
            }
            Ok(count)
        })
    }
}

struct GuiSession {
    uid: u32,
    id: i64,
    active: bool,
    locked: bool,
}

pub(super) fn session_locked() -> bool {
    GuiSession::current().is_ok_and(|session| session.locked)
}

impl GuiSession {
    fn surface(&self) -> Option<NativeSurface> {
        if self.active {
            Some(NativeSurface::Desktop)
        } else if self.locked {
            Some(NativeSurface::LockScreen)
        } else {
            None
        }
    }

    fn current() -> Result<Self> {
        let uid = unsafe { libc::geteuid() };
        ensure!(
            uid != 0,
            "macOS desktop host must run as the logged-in GUI user, not root"
        );
        // CGSession's public dictionary does not expose a session ID key.
        // AuthSession.h provides the caller's stable session ID and attributes.
        let mut id = 0_u32;
        let mut attributes = 0_u32;
        let status = unsafe { SessionGetInfo(u32::MAX, &raw mut id, &raw mut attributes) };
        ensure!(
            status == 0 && id != 0 && id != u32::MAX,
            "cannot read macOS security session: OSStatus {status}"
        );
        let mut console_uid = u32::MAX;
        let mut console_gid = u32::MAX;
        let user = unsafe {
            SCDynamicStoreCopyConsoleUser(
                std::ptr::null(),
                &raw mut console_uid,
                &raw mut console_gid,
            )
        };
        ensure!(!user.is_null(), "no logged-in macOS console user");
        unsafe { CFRelease(user) };
        let dictionary = unsafe { CGSessionCopyCurrentDictionary() };
        ensure!(!dictionary.is_null(), "no current macOS GUI session");
        let session = (|| -> Result<Self> {
            // CGSession.h macro names differ from the actual dictionary keys.
            let session_uid = dictionary_number(dictionary, "kCGSSessionUserIDKey")
                .context("macOS GUI session has no user ID")?;
            let on_console =
                dictionary_bool(dictionary, "kCGSSessionOnConsoleKey").unwrap_or(false);
            let logged_in = dictionary_bool(dictionary, "kCGSessionLoginDoneKey").unwrap_or(false);
            // Both keys are undocumented. A positive lock from either source
            // vetoes access; absence alone never proves the console unlocked.
            let quartz_lock = dictionary_bool(dictionary, "CGSSessionScreenIsLocked");
            let console_lock = io_console_locked();
            Ok(Self {
                uid,
                id: i64::from(id),
                locked: uid == console_uid
                    && i64::from(uid) == session_uid
                    && on_console
                    && logged_in
                    && attributes & 0x0010 != 0
                    && (quartz_lock == Some(true) || console_lock == Some(true)),
                // sessionHasGraphicAccess from Security/AuthSession.h.
                active: attributes & 0x0010 != 0
                    && gui_session_active(
                        uid,
                        console_uid,
                        session_uid,
                        on_console,
                        logged_in,
                        quartz_lock,
                        console_lock,
                    ),
            })
        })();
        unsafe { CFRelease(dictionary) };
        session
    }
}

fn gui_session_active(
    uid: u32,
    console_uid: u32,
    session_uid: i64,
    on_console: bool,
    logged_in: bool,
    quartz_lock: Option<bool>,
    console_lock: Option<bool>,
) -> bool {
    uid == console_uid
        && i64::from(uid) == session_uid
        && on_console
        && logged_in
        && lock_states_allow(quartz_lock, console_lock)
}

fn lock_states_allow(quartz: Option<bool>, console: Option<bool>) -> bool {
    quartz != Some(true)
        && console != Some(true)
        && (quartz == Some(false) || console == Some(false))
}

fn io_console_locked() -> Option<bool> {
    let entry = unsafe { IORegistryGetRootEntry(0) };
    if entry == 0 {
        return None;
    }
    let key = unsafe {
        CFStringCreateWithCString(std::ptr::null(), c"IOConsoleLocked".as_ptr(), 0x0800_0100)
    };
    if key.is_null() {
        unsafe { IOObjectRelease(entry) };
        return None;
    }
    let value = unsafe { IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0) };
    unsafe {
        CFRelease(key);
        IOObjectRelease(entry);
    }
    if value.is_null() {
        return None;
    }
    let result = (unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() })
        .then(|| unsafe { CFBooleanGetValue(value) != 0 });
    unsafe { CFRelease(value) };
    result
}

fn dictionary_value(dictionary: *const c_void, key: &str) -> Option<*const c_void> {
    let key = std::ffi::CString::new(key).ok()?;
    let string = unsafe { CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), 0x0800_0100) };
    if string.is_null() {
        return None;
    }
    let value = unsafe { CFDictionaryGetValue(dictionary, string) };
    unsafe { CFRelease(string) };
    (!value.is_null()).then_some(value)
}
fn dictionary_number(dictionary: *const c_void, key: &str) -> Option<i64> {
    let value = dictionary_value(dictionary, key)?;
    if unsafe { CFGetTypeID(value) != CFNumberGetTypeID() } {
        return None;
    }
    let mut number = 0_i64;
    unsafe { CFNumberGetValue(value, 4, &raw mut number as *mut c_void) }.then_some(number)
}
fn dictionary_bool(dictionary: *const c_void, key: &str) -> Option<bool> {
    let value = dictionary_value(dictionary, key)?;
    (unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() })
        .then(|| unsafe { CFBooleanGetValue(value) != 0 })
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
    fn CGDisplayIsActive(display: u32) -> u32;
    fn CGDisplayCopyDisplayMode(display: u32) -> *const c_void;
    fn CGDisplayModeGetWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeGetHeight(mode: *const c_void) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: *const c_void) -> usize;
    fn CGDisplayModeRelease(mode: *const c_void);
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGSessionCopyCurrentDictionary() -> *const c_void;
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SessionGetInfo(session: u32, id: *mut u32, attributes: *mut u32) -> i32;
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}
#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyConsoleUser(
        store: *const c_void,
        uid: *mut u32,
        gid: *mut u32,
    ) -> *const c_void;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        string: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: *const c_void, type_code: i32, value: *mut c_void) -> bool;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: *const c_void) -> u8;
}
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionDeclareUserActivity(name: *const c_void, user_type: u32, id: *mut u32) -> i32;
    fn IORegistryGetRootEntry(main_port: u32) -> u32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> *const c_void;
    fn IOObjectRelease(object: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_session_requires_affirmative_unlocked_state() {
        for (quartz, console, expected) in [
            (Some(false), None, true),
            (None, Some(false), true),
            (Some(false), Some(false), true),
            (Some(true), Some(false), false),
            (Some(false), Some(true), false),
            (Some(true), None, false),
            (None, Some(true), false),
            (None, None, false),
        ] {
            assert_eq!(
                gui_session_active(501, 501, 501, true, true, quartz, console),
                expected,
                "lock states {quartz:?}, {console:?}"
            );
        }
        assert!(!gui_session_active(
            501,
            502,
            501,
            true,
            true,
            Some(false),
            None
        ));
        assert!(!gui_session_active(
            501,
            501,
            502,
            true,
            true,
            Some(false),
            None
        ));
        assert!(!gui_session_active(
            501,
            501,
            501,
            false,
            true,
            Some(false),
            None
        ));
        assert!(!gui_session_active(
            501,
            501,
            501,
            true,
            false,
            Some(false),
            None
        ));
    }
    #[test]
    fn padded_bgra_is_packed_rgba_without_padding() {
        let source = [
            1, 2, 3, 4, 5, 6, 7, 8, 0xaa, 0xbb, 9, 10, 11, 12, 13, 14, 15, 16, 0xcc, 0xdd,
        ];
        assert_eq!(
            padded_bgra_to_rgba(&source, 2, 2, 10).unwrap(),
            [3, 2, 1, 4, 7, 6, 5, 8, 11, 10, 9, 12, 15, 14, 13, 16]
        );
        assert!(padded_bgra_to_rgba(&source[..17], 2, 2, 10).is_err());
    }
    #[test]
    fn stream_error_clears_queued_frame() {
        let events = CaptureEvents::default();
        events.state.lock().unwrap().frame = Some(Frame::new(1, 1, vec![0; 4].into()));
        events.fail("revoked".into());
        let state = events.state.lock().unwrap();
        assert!(state.frame.is_none());
        assert_eq!(state.error.as_deref(), Some("revoked"));
    }
    #[test]
    fn planar_float_samples_are_interleaved() {
        assert_eq!(
            interleave_planar(&[1, 2, 3, 4, 5, 6, 7, 8], &[9, 10, 11, 12, 13, 14, 15, 16])
                .unwrap()
                .unwrap(),
            [1, 2, 3, 4, 9, 10, 11, 12, 5, 6, 7, 8, 13, 14, 15, 16]
        );
        assert!(interleave_planar(&[1, 2, 3, 4], &[1, 2]).is_err());
    }
}
