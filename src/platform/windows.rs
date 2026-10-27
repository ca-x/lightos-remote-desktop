//! Interactive Windows desktop host. The SCM process never constructs this provider.

#[path = "windows_service.rs"]
mod windows_service;
pub use windows_service::{install_service, run_service, wait_for_stop_event};

use std::{
    ffi::c_void,
    mem::{size_of, size_of_val},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use anyhow::{Context as _, Result, bail, ensure};
use display_control::DisplayOutput;
use enigo::Enigo;
use wasapi::{
    AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle as WasapiEvent,
    SampleType, StreamMode, WaveFormat,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree},
        Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
        Security::Authorization::ConvertSidToStringSidW,
        Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser},
        System::{
            RemoteDesktop::{
                ProcessIdToSessionId, WTS_SESSIONSTATE_UNLOCK, WTSActive, WTSFreeMemory,
                WTSGetActiveConsoleSessionId, WTSINFOEXW, WTSQuerySessionInformationW,
                WTSSessionInfoEx,
            },
            StationsAndDesktops::{
                CloseDesktop, DESKTOP_READOBJECTS, GetUserObjectInformationW, OpenInputDesktop,
                UOI_NAME,
            },
            Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken},
        },
        UI::HiDpi::{
            AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            GetDpiAwarenessContextForProcess, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
            SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
        },
        UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
            MOUSEINPUT, SendInput,
        },
        UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        },
    },
    core::PWSTR,
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame as CaptureFrame,
    graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl},
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};

use super::{NativeCapture, NativePlatform, PermissionProvider, PermissionState, Permissions};
use crate::{
    audio::{AudioCapture, AudioCaptureFactory, AudioFormat, AudioStream, PcmWriter},
    backend::{BackendKind, Frame},
};

pub fn provider() -> Arc<dyn NativePlatform> {
    if let Err(error) = process_dpi_ready() {
        tracing::warn!(%error, "Windows physical-pixel DPI setup failed");
    }
    Arc::new(WindowsPlatform)
}

struct WindowsPlatform;

pub(super) fn permission_report(request: bool) -> Result<super::permissions::PermissionReport> {
    use windows::{
        Foundation::Metadata::ApiInformation,
        Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind},
        Security::Authorization::AppCapabilityAccess::{AppCapability, AppCapabilityAccessStatus},
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        core::HSTRING,
    };
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { RoUninitialize() };
            }
        }
    }
    let _apartment = Apartment(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok());
    let platform = WindowsPlatform;
    let permissions = match platform.permissions() {
        Ok(permissions) => permissions,
        Err(error) => {
            return Ok(super::permissions::PermissionReport::unavailable(
                error.to_string(),
                "permission_check_failed",
            ));
        }
    };
    if !matches!(permissions.input, PermissionState::Granted) {
        return Ok(super::permissions::PermissionReport::unavailable(
            permissions.reason.unwrap_or_default(),
            "session_unavailable",
        ));
    }
    if !matches!(permissions.screen, PermissionState::Granted) {
        let code = if permissions
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("display"))
        {
            "display_unavailable"
        } else {
            "capture_unsupported"
        };
        return Ok(super::permissions::PermissionReport::unavailable(
            permissions.reason.unwrap_or_default(),
            code,
        ));
    }
    let mut access = AppCapability::Create(&HSTRING::from("graphicsCaptureProgrammatic"))
        .and_then(|capability| capability.CheckAccess())
        .ok();
    if request
        && ApiInformation::IsTypePresent(&HSTRING::from(
            "Windows.Graphics.Capture.GraphicsCaptureAccess",
        ))
        .unwrap_or(false)
    {
        access = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Programmatic)
            .and_then(|operation| operation.get())
            .ok()
            .or(access);
    }
    // Traditional unpackaged Win32 hosts use CreateForMonitor; NotDeclared is
    // not a denial of that interop API. Real policy/user denials remain visible.
    match access {
        Some(AppCapabilityAccessStatus::UserPromptRequired) => {
            Ok(super::permissions::PermissionReport::windows(
                Permissions {
                    screen: PermissionState::NeedsUserAction,
                    input: PermissionState::Granted,
                    reason: Some("Windows requires approval for screen capture".into()),
                },
                "system",
                "permission_required",
            ))
        }
        Some(AppCapabilityAccessStatus::DeniedByUser) => {
            Ok(super::permissions::PermissionReport::windows(
                Permissions {
                    screen: PermissionState::Denied,
                    input: PermissionState::Granted,
                    reason: Some("Screen capture is disabled in Windows privacy settings".into()),
                },
                "settings",
                "permission_required",
            ))
        }
        Some(AppCapabilityAccessStatus::DeniedBySystem) => {
            Ok(super::permissions::PermissionReport::windows(
                Permissions {
                    screen: PermissionState::Denied,
                    input: PermissionState::Granted,
                    reason: Some("Screen capture is disabled by system policy".into()),
                },
                "none",
                "system_policy_denied",
            ))
        }
        _ => Ok(super::permissions::PermissionReport::windows(
            permissions,
            "none",
            "ready",
        )),
    }
}

impl PermissionProvider for WindowsPlatform {
    fn permissions(&self) -> Result<Permissions> {
        process_dpi_ready()?;
        let active = session_active()?;
        let supported =
            GraphicsCaptureApi::is_supported().context("query Windows capture support")?;
        let cursor = supported
            && GraphicsCaptureApi::is_cursor_settings_supported()
                .context("query Windows cursor capture support")?;
        let has_display = !Monitor::enumerate()
            .context("enumerate Windows monitors")?
            .is_empty();
        let granted = active && supported && cursor && has_display;
        Ok(Permissions {
            screen: if granted {
                PermissionState::Granted
            } else {
                PermissionState::Denied
            },
            input: if active {
                PermissionState::Granted
            } else {
                PermissionState::Denied
            },
            reason: (!granted).then(|| {
                if !active {
                    "the interactive desktop is locked, secure, or inactive"
                } else if !supported {
                    "Windows Graphics Capture is unavailable"
                } else if !cursor {
                    "Windows cursor capture is unavailable"
                } else {
                    "no active display is available"
                }
                .to_owned()
            }),
        })
    }
}

impl NativePlatform for WindowsPlatform {
    fn move_pointer(&self, _: &mut Enigo, x: i32, y: i32) -> Result<()> {
        let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
        let (dx, dy) = virtual_pointer(x, y, left, top, width, height)?;
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    ..Default::default()
                },
            },
        };
        ensure!(
            unsafe { SendInput(&[input], size_of::<INPUT>() as i32) } == 1,
            "Windows virtual-desktop pointer injection failed: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Windows
    }

    fn session_key(&self) -> Result<String> {
        let id = current_session_id()?;
        let details = session_details(id)?;
        let sid = current_user_sid()?;
        Ok(format!("windows:{id}:{sid}:{}", details.logon_time))
    }

    fn session_active(&self) -> Result<bool> {
        session_active()
    }

    fn displays(&self) -> Result<Vec<DisplayOutput>> {
        let _dpi = PhysicalPixelContext::enter()?;
        let mut displays = Vec::new();
        for monitor in Monitor::enumerate().context("enumerate Windows monitors")? {
            let name = monitor.device_name().context("read Windows display name")?;
            let width = monitor.width().context("read Windows display width")?;
            let height = monitor.height().context("read Windows display height")?;
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                rcMonitor: Default::default(),
                rcWork: Default::default(),
                dwFlags: 0,
            };
            unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &mut info).ok()? };
            let mut xdpi = 96;
            let mut ydpi = 96;
            unsafe {
                GetDpiForMonitor(
                    HMONITOR(monitor.as_raw_hmonitor()),
                    MDT_EFFECTIVE_DPI,
                    &mut xdpi,
                    &mut ydpi,
                )?
            };
            displays.push(DisplayOutput {
                name,
                description: monitor
                    .name()
                    .or_else(|_| monitor.device_string())
                    .unwrap_or_default(),
                width,
                height,
                x: info.rcMonitor.left,
                y: info.rcMonitor.top,
                scale: f64::from(xdpi) / 96.0,
            });
        }
        Ok(displays)
    }

    fn capture(&self, output: &DisplayOutput, fps: u32) -> Result<Box<dyn NativeCapture>> {
        let _dpi = PhysicalPixelContext::enter()?;
        ensure!((1..=120).contains(&fps), "capture FPS must be 1..=120");
        ensure!(
            session_active()?,
            "interactive Windows desktop is unavailable"
        );
        ensure!(
            GraphicsCaptureApi::is_supported()?,
            "Windows Graphics Capture is unavailable"
        );
        ensure!(
            GraphicsCaptureApi::is_cursor_settings_supported()?,
            "cursor capture is unavailable"
        );
        let monitor = Monitor::enumerate()?
            .into_iter()
            .find(|monitor| monitor.device_name().ok().as_deref() == Some(output.name.as_str()))
            .context("selected Windows display is unavailable")?;
        let border = if GraphicsCaptureApi::is_border_settings_supported()? {
            DrawBorderSettings::WithoutBorder
        } else {
            DrawBorderSettings::Default
        };
        let update_interval = if GraphicsCaptureApi::is_minimum_update_interval_supported()? {
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / f64::from(fps)))
        } else {
            MinimumUpdateIntervalSettings::Default
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::WithoutCursor,
            border,
            SecondaryWindowSettings::Default,
            update_interval,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            sender,
        );
        let control = CaptureHandler::start_free_threaded(settings)
            .context("start Windows screen capture")?;
        Ok(Box::new(WindowsCapture {
            receiver,
            control: Some(control),
        }))
    }

    fn audio(&self) -> Arc<dyn AudioCaptureFactory> {
        Arc::new(WasapiAudio {
            available: probe_audio().is_ok(),
        })
    }
}

fn virtual_pointer(
    x: i32,
    y: i32,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
) -> Result<(i32, i32)> {
    ensure!(
        width > 0 && height > 0,
        "virtual desktop has invalid dimensions"
    );
    let relative_x = i64::from(x) - i64::from(left);
    let relative_y = i64::from(y) - i64::from(top);
    ensure!(
        (0..i64::from(width)).contains(&relative_x) && (0..i64::from(height)).contains(&relative_y),
        "selected pointer position lies outside the current virtual desktop"
    );
    let normalize = |position: i64, extent: i32| -> i32 {
        if extent == 1 {
            0
        } else {
            ((position * 65_535 + i64::from(extent - 1) / 2) / i64::from(extent - 1)) as i32
        }
    };
    Ok((normalize(relative_x, width), normalize(relative_y, height)))
}

#[cfg(test)]
mod pointer_tests {
    use super::virtual_pointer;

    #[test]
    fn two_monitor_virtual_desktop_maps_negative_origin_and_all_edges() {
        assert_eq!(
            virtual_pointer(-1920, 0, -1920, 0, 3840, 1080).unwrap(),
            (0, 0)
        );
        assert_eq!(
            virtual_pointer(-1, 1079, -1920, 0, 3840, 1080).unwrap().1,
            65_535
        );
        assert_eq!(
            virtual_pointer(0, 0, -1920, 0, 3840, 1080).unwrap().0,
            32_776
        );
        assert_eq!(
            virtual_pointer(1919, 1079, -1920, 0, 3840, 1080).unwrap(),
            (65_535, 65_535)
        );
        assert!(virtual_pointer(-1921, 0, -1920, 0, 3840, 1080).is_err());
        assert!(virtual_pointer(1920, 0, -1920, 0, 3840, 1080).is_err());
        assert_eq!(
            virtual_pointer(0, -1080, 0, -1080, 1920, 2160).unwrap().1,
            0
        );
        assert_eq!(
            virtual_pointer(0, 1079, 0, -1080, 1920, 2160).unwrap().1,
            65_535
        );
        assert_eq!(virtual_pointer(-5, -8, -5, -8, 1, 1).unwrap(), (0, 0));
    }
}

struct WasapiAudio {
    available: bool,
}

impl AudioCaptureFactory for WasapiAudio {
    fn available(&self) -> bool {
        self.available
    }

    fn start(&self, format: AudioFormat) -> Result<Box<dyn AudioCapture>> {
        ensure!(
            self.available,
            "WASAPI loopback is unavailable in this session"
        );
        probe_audio().context("initialize Windows system audio loopback")?;
        Ok(Box::new(AudioStream::capture(
            "wasapi-loopback",
            format,
            run_audio,
        )?))
    }
}

struct ComApartment;
impl ComApartment {
    fn enter() -> Result<Self> {
        wasapi::initialize_mta()
            .ok()
            .context("initialize WASAPI COM apartment")?;
        Ok(Self)
    }
}
impl Drop for ComApartment {
    fn drop(&mut self) {
        wasapi::deinitialize();
    }
}

struct Loopback {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: WasapiEvent,
}

impl Loopback {
    fn open() -> Result<Self> {
        let enumerator = DeviceEnumerator::new()?;
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut client = device.get_iaudioclient()?;
        let format = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
        let (period, _) = client.get_device_period()?;
        client.initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: period,
            },
        )?;
        let event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        Ok(Self {
            client,
            capture,
            event,
        })
    }
}

fn probe_audio() -> Result<()> {
    ensure!(
        session_active()?,
        "interactive desktop is unavailable for audio"
    );
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("wasapi-probe".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let _com = ComApartment::enter()?;
                let loopback = Loopback::open()?;
                loopback.client.start_stream()?;
                loopback.client.stop_stream()?;
                Ok(())
            })();
            let _ = sender.send(result);
        })
        .context("start WASAPI probe")?;
    receiver
        .recv_timeout(Duration::from_secs(2))
        .context("WASAPI probe timed out or stopped")?
}

fn run_audio(stop: &AtomicBool, writer: &mut PcmWriter) -> Result<()> {
    let _com = ComApartment::enter()?;
    let loopback = Loopback::open()?;
    loopback.client.start_stream()?;
    let mut buffer = Vec::new();
    let result = (|| -> Result<()> {
        while !stop.load(Ordering::Acquire) {
            ensure!(
                session_active()?,
                "interactive desktop locked during audio capture"
            );
            while let Some(frames) = loopback.capture.get_next_packet_size()? {
                if frames == 0 {
                    break;
                }
                ensure!(
                    session_active()?,
                    "interactive desktop locked during audio capture"
                );
                let bytes = usize::try_from(frames)?
                    .checked_mul(8)
                    .context("WASAPI packet overflow")?;
                buffer.resize(bytes, 0);
                let (read, info) = loopback.capture.read_from_device(&mut buffer)?;
                let size = usize::try_from(read)?
                    .checked_mul(8)
                    .context("WASAPI packet overflow")?;
                if info.flags.silent {
                    buffer[..size].fill(0);
                }
                ensure!(
                    session_active()?,
                    "interactive desktop locked during audio capture"
                );
                writer.push(&buffer[..size])?;
            }
            match loopback.event.wait_for_event(100) {
                Ok(()) | Err(wasapi::WasapiError::EventTimeout) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    })();
    let stopped = loopback.client.stop_stream();
    result?;
    stopped?;
    Ok(())
}

struct CaptureHandler {
    frames: mpsc::SyncSender<std::result::Result<Frame, String>>,
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = mpsc::SyncSender<std::result::Result<Frame, String>>;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        Ok(Self { frames: ctx.flags })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut CaptureFrame,
        _control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        if !session_active()? {
            return Ok(());
        }
        let width = frame.width();
        let height = frame.height();
        let mut buffer = frame.buffer()?;
        if buffer.color_format() != ColorFormat::Rgba8 {
            return Err("unexpected Windows capture pixel format".into());
        }
        let stride = buffer.row_pitch();
        let pixels = pack_rgba(buffer.as_raw_buffer(), width, height, stride)?;
        let mut frame = Frame::new(width, height, pixels.into());
        frame.cursor_embedded = false;
        match self.frames.try_send(Ok(frame)) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => Err("capture receiver closed".into()),
        }
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        let _ = self
            .frames
            .try_send(Err("Windows capture item closed".into()));
        Ok(())
    }
}

struct WindowsCapture {
    receiver: mpsc::Receiver<std::result::Result<Frame, String>>,
    control: Option<CaptureControl<CaptureHandler, Box<dyn std::error::Error + Send + Sync>>>,
}

impl NativeCapture for WindowsCapture {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        if !session_active()? {
            return Ok(None);
        }
        match self
            .receiver
            .recv_timeout(timeout.min(Duration::from_secs(2)))
        {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(error)) => bail!("{error}"),
            Err(mpsc::RecvTimeoutError::Timeout)
                if self
                    .control
                    .as_ref()
                    .is_some_and(|control| control.is_finished()) =>
            {
                bail!("Windows capture stopped")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("Windows capture disconnected"),
        }
    }
}

impl Drop for WindowsCapture {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            // The crate's stop joins its capture thread without a deadline.
            control.halt_handle().store(true, Ordering::Release);
            if let Err(error) = std::thread::Builder::new()
                .name("windows-capture-stop".into())
                .spawn(move || {
                    let _ = control.stop();
                })
            {
                tracing::warn!(%error, "unable to start Windows capture cleanup");
            }
        }
    }
}

fn pack_rgba(source: &[u8], width: u32, height: u32, stride: u32) -> Result<Vec<u8>> {
    let row = usize::try_from(width)?
        .checked_mul(4)
        .context("frame row overflow")?;
    let height = usize::try_from(height)?;
    let stride = usize::try_from(stride)?;
    ensure!(stride >= row, "capture stride is shorter than a pixel row");
    let required = height.checked_mul(stride).context("frame size overflow")?;
    ensure!(
        source.len() >= required,
        "capture buffer is shorter than its stride"
    );
    let mut pixels = Vec::with_capacity(row.checked_mul(height).context("frame size overflow")?);
    for y in 0..height {
        pixels.extend_from_slice(&source[y * stride..y * stride + row]);
    }
    Ok(pixels)
}

pub(crate) fn current_session_id() -> Result<u32> {
    let mut id = 0;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut id)? };
    Ok(id)
}

pub(crate) fn active_console_session_id() -> Option<u32> {
    let id = unsafe { WTSGetActiveConsoleSessionId() };
    (id != u32::MAX).then_some(id)
}

struct SessionDetails {
    active: bool,
    unlocked: bool,
    logon_time: i64,
}

struct PhysicalPixelContext(DPI_AWARENESS_CONTEXT);

impl PhysicalPixelContext {
    fn enter() -> Result<Self> {
        process_dpi_ready()?;
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        ensure!(
            !previous.0.is_null(),
            "per-monitor DPI awareness is unavailable"
        );
        Ok(Self(previous))
    }
}

fn process_dpi_ready() -> Result<()> {
    static DPI: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    DPI.get_or_init(|| {
        let aware = |context| unsafe {
            AreDpiAwarenessContextsEqual(context, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE).as_bool()
                || AreDpiAwarenessContextsEqual(context, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                    .as_bool()
        };
        if aware(unsafe { GetDpiAwarenessContextForProcess(GetCurrentProcess()) }) {
            return Ok(());
        }
        let set =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if aware(unsafe { GetDpiAwarenessContextForProcess(GetCurrentProcess()) }) {
            Ok(())
        } else {
            Err(format!(
                "per-monitor process DPI awareness is unavailable: {:?}",
                set.err()
            ))
        }
    })
    .as_ref()
    .map_err(|error| anyhow::anyhow!("{error}"))
    .copied()
}

impl Drop for PhysicalPixelContext {
    fn drop(&mut self) {
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

fn session_details(id: u32) -> Result<SessionDetails> {
    let mut ptr = PWSTR::null();
    let mut bytes = 0;
    unsafe { WTSQuerySessionInformationW(None, id, WTSSessionInfoEx, &mut ptr, &mut bytes)? };
    let result = (|| {
        ensure!(
            bytes as usize >= size_of::<WTSINFOEXW>(),
            "short Windows session information"
        );
        let info = unsafe { &*(ptr.0.cast::<WTSINFOEXW>()) };
        ensure!(
            info.Level == 1,
            "unsupported Windows session information level"
        );
        let detail = unsafe { info.Data.WTSInfoExLevel1 };
        Ok(SessionDetails {
            active: detail.SessionState == WTSActive,
            unlocked: detail.SessionFlags == WTS_SESSIONSTATE_UNLOCK as i32,
            logon_time: detail.LogonTime,
        })
    })();
    unsafe { WTSFreeMemory(ptr.0.cast::<c_void>()) };
    result
}

pub(crate) fn console_session_ready(id: u32) -> Result<bool> {
    let details = session_details(id)?;
    Ok(details.active && details.unlocked)
}

fn input_desktop_is_default() -> Result<bool> {
    let desktop = match unsafe { OpenInputDesktop(Default::default(), false, DESKTOP_READOBJECTS) }
    {
        Ok(desktop) => desktop,
        Err(_) => return Ok(false),
    };
    let result = (|| {
        let mut name = [0u16; 64];
        let mut needed = 0;
        unsafe {
            GetUserObjectInformationW(
                HANDLE(desktop.0),
                UOI_NAME,
                Some(name.as_mut_ptr().cast()),
                size_of_val(&name) as u32,
                Some(&mut needed),
            )?;
        }
        let len = name.iter().position(|ch| *ch == 0).unwrap_or(name.len());
        Ok(String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case("Default"))
    })();
    unsafe { CloseDesktop(desktop)? };
    result
}

fn session_active() -> Result<bool> {
    let id = current_session_id()?;
    if id == 0 {
        return Ok(false);
    }
    let details = session_details(id)?;
    if !details.active || !details.unlocked {
        return Ok(false);
    }
    input_desktop_is_default()
}

pub(crate) fn current_user_sid() -> Result<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    let result = token_sid(token);
    unsafe { CloseHandle(token)? };
    result
}

pub(crate) fn token_sid(token: HANDLE) -> Result<String> {
    let mut needed = 0;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    ensure!(
        needed as usize >= size_of::<TOKEN_USER>(),
        "invalid Windows token user"
    );
    let words = (needed as usize).div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; words];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(storage.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )?
    };
    let user = unsafe { &*(storage.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid)? };
    let result = (|| {
        let mut len = 0;
        while len < 256 && unsafe { *sid.0.add(len) } != 0 {
            len += 1;
        }
        ensure!(len < 256, "Windows SID string is unexpectedly long");
        Ok(String::from_utf16(unsafe {
            std::slice::from_raw_parts(sid.0, len)
        })?)
    })();
    unsafe { LocalFree(Some(HLOCAL(sid.0.cast()))) };
    result
}

#[cfg(test)]
mod tests {
    use super::pack_rgba;

    #[test]
    fn packed_rgba_skips_row_padding() {
        let pixels = pack_rgba(&[1, 2, 3, 4, 99, 99, 5, 6, 7, 8, 88, 88], 1, 2, 6).unwrap();
        assert_eq!(pixels, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(pack_rgba(&[1, 2, 3, 4], 1, 2, 4).is_err());
    }
}
