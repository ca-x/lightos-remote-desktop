//! Existing Linux sessions behind the portable host lifecycle.
use super::*;
use crate::{
    audio::{AudioCapture, AudioFormat, AudioStream},
    backend,
    session::Session,
};
use anyhow::{Context, ensure};
use display_control::{DesktopEnvironment, SessionTarget, WaylandDisplay, X11Display};
use std::sync::atomic::Ordering;

mod idle;
pub struct LinuxHost;

struct PendingLifecycle;
impl SessionLifecycle for PendingLifecycle {
    fn healthy(&mut self) -> bool {
        true
    }
    fn is_owned(&self) -> bool {
        false
    }
}

struct OwnedLifecycle {
    _idle: Option<idle::Inhibitor>,
    session: Session,
}
impl SessionLifecycle for OwnedLifecycle {
    fn healthy(&mut self) -> bool {
        self.session.healthy()
    }
    fn is_owned(&self) -> bool {
        self.session.is_owned()
    }
}

impl SessionLifecycle for Session {
    fn healthy(&mut self) -> bool {
        Session::healthy(self)
    }
    fn is_owned(&self) -> bool {
        Session::is_owned(self)
    }
}

struct PulseAudio {
    server: Option<String>,
    source: Option<String>,
}
impl AudioCaptureFactory for PulseAudio {
    fn available(&self) -> bool {
        self.source.is_some()
    }
    fn start(&self, format: AudioFormat) -> Result<Box<dyn AudioCapture>> {
        Ok(Box::new(AudioStream::with_format(
            self.server.clone(),
            self.source.clone().context("audio unavailable")?,
            format,
        )?))
    }
}

pub fn context(config: &Config, backend: Arc<dyn DesktopBackend>, session: Session) -> HostSession {
    let displays = session_displays(config, &session);
    let audio = Arc::new(PulseAudio {
        server: session.pulse_server.clone(),
        source: session.audio_source.clone(),
    });
    let idle = session
        .is_owned()
        .then(|| session.target().bus_address)
        .flatten()
        .and_then(|address| match idle::Inhibitor::start(address) {
            Ok(inhibitor) => Some(inhibitor),
            Err(error) => {
                tracing::warn!(%error,"could not start owned desktop idle guard");
                None
            }
        });
    HostSession {
        backend,
        displays,
        audio,
        lifecycle: Box::new(OwnedLifecycle {
            _idle: idle,
            session,
        }),
        output: None,
        outputs: Vec::new(),
    }
}

fn session_displays(config: &Config, session: &Session) -> Arc<dyn DisplayController> {
    let target = session.target();
    if session.is_owned() && config.virtual_x11_dynamic_modes && target.x11_display.is_some() {
        return Arc::new(X11Display::virtual_session(
            target,
            config
                .virtual_environment
                .get("LIGHTOS_REMOTE_DESKTOP_GNOME_X11")
                .is_some_and(|value| value == "1")
                || std::env::var("LIGHTOS_REMOTE_DESKTOP_GNOME_X11").as_deref() == Ok("1"),
        ));
    }
    if config.virtual_wayland_server_pid.is_some() && session.is_owned() {
        if let Some(helper) = config
            .virtual_environment
            .get("LIGHTOS_REMOTE_DESKTOP_NESTED_RESIZE_HELPER")
        {
            // The transport keeps the physical desktop's environment. Select
            // the query adapter from the verified owned compositor instead.
            let compositor = std::fs::read_link(format!(
                "/proc/{}/exe",
                config.virtual_wayland_server_pid.unwrap()
            ))
            .ok();
            let primary: Arc<dyn DisplayController> = match compositor
                .as_deref()
                .and_then(std::path::Path::file_name)
                .and_then(|name| name.to_str())
            {
                Some("gnome-shell") => {
                    Arc::new(display_control::MutterDisplay::new(target.clone()))
                }
                Some("kwin_wayland" | "kwin_wayland_wrapper") => {
                    Arc::new(display_control::KScreenDisplay::new(target.clone()))
                }
                _ => Arc::new(WaylandDisplay::new(session.socket.clone())),
            };
            return Arc::new(display_control::NestedDisplay::with_environment(
                primary,
                target,
                helper.into(),
                config.virtual_environment.clone(),
            ));
        }
    }
    if config.display_backend == display_control::DisplayBackend::Auto && session.is_native_niri() {
        return Arc::new(WaylandDisplay::new(session.socket.clone()));
    }
    match config.display_backend.controller(
        target.clone(),
        environment(&target),
        session.signature.as_deref(),
        session.is_owned(),
    ) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::warn!(%error, "display backend unavailable");
            match target.wayland_socket {
                Some(socket) => Arc::new(WaylandDisplay::new(socket)),
                None => Arc::new(X11Display::new(target)),
            }
        }
    }
}

fn environment(target: &SessionTarget) -> DesktopEnvironment {
    if target.wayland_socket.is_none() && target.x11_display.is_some() {
        DesktopEnvironment::X11
    } else {
        target.matching_environment_hint()
    }
}

// Capture can still use the X11 root window without RandR output metadata.
#[cfg(target_os = "linux")]
pub(crate) fn physical_output_selection(
    backend: BackendKind,
    requested: Option<&str>,
    displays: &dyn DisplayController,
) -> anyhow::Result<(Vec<DisplayOutput>, Option<String>)> {
    use anyhow::{Context, ensure};
    if backend == BackendKind::Portal {
        ensure!(
            requested.is_none(),
            "Portal 显示器由本机授权窗口选择，请重新连接"
        );
        return Ok((Vec::new(), None));
    }
    let outputs = match displays.outputs() {
        Ok(outputs) => outputs,
        Err(error) if backend == BackendKind::X11 && requested.is_none() => {
            tracing::debug!(%error, "X11 output metadata unavailable; capturing the root window");
            return Ok((Vec::new(), None));
        }
        Err(error) => return Err(error).context("物理显示器枚举失败"),
    };
    if backend == BackendKind::X11 && requested.is_none() && outputs.is_empty() {
        return Ok((outputs, None));
    }
    let selected = match requested {
        Some(name) => outputs.iter().find(|output| output.name == name),
        None => outputs.first(),
    }
    .ok_or_else(|| anyhow::anyhow!("所选显示器不可用，请刷新显示器列表"))?
    .name
    .clone();
    Ok((outputs, Some(selected)))
}

impl HostPlatform for LinuxHost {
    fn physical_displays(&self, config: &Config) -> Result<Arc<dyn DisplayController>> {
        let target = Session::physical_target(config)?;
        config
            .display_backend
            .controller(target.clone(), environment(&target), None, false)
    }
    fn physical_key(&self, config: &Config) -> Result<String> {
        Ok(format!("{:?}", Session::physical_target(config)?))
    }
    fn probe_physical(
        &self,
        config: &Config,
        cancelled: Option<&AtomicBool>,
    ) -> Result<BackendReport> {
        let target = Session::physical_target(config)?;
        let report = backend::probe(&target, true, cancelled)?;
        if let Some(selected) = report.selected {
            backend::guarded::require_attachable(&target, selected)?;
        }
        Ok(report)
    }
    fn supports_virtual(&self, config: &Config) -> bool {
        if config.on_demand_desktops() {
            return crate::session::installed::available().is_ok();
        }
        config.virtual_wayland_server_pid.is_some()
            || config.virtual_x11_server_pid.is_some()
            || (config.wayland_socket.is_none() && config.x11_display.is_none())
    }
    fn virtual_unavailable_reason(&self, config: &Config) -> String {
        if config.on_demand_desktops() {
            return crate::session::installed::available()
                .err()
                .map_or_else(String::new, |error| format!("{error:#}"));
        }
        "服务以外部桌面模式启动".into()
    }
    fn open_virtual(&self, config: &Config, cancelled: Arc<AtomicBool>) -> Result<HostSession> {
        ensure!(
            config.virtual_fallback,
            "virtual desktop fallback is not enabled"
        );
        let (virtual_config, session) =
            crate::session::installed::start(config, cancelled.clone())?;
        self.attach(&virtual_config, session, cancelled)
    }
    fn open(&self, config: &Config, cancelled: Arc<AtomicBool>) -> Result<HostSession> {
        if config.on_demand_desktops() {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "desktop startup cancelled"
            );
            return Ok(HostSession {
                backend: Arc::new(backend::DeferredPortalBackend::new()),
                displays: Arc::new(WaylandDisplay::new(std::path::PathBuf::new())),
                lifecycle: Box::new(PendingLifecycle),
                audio: Arc::new(PulseAudio {
                    server: None,
                    source: None,
                }),
                output: None,
                outputs: Vec::new(),
            });
        }
        let session = Session::start_cancellable(config, cancelled.clone())?;
        self.attach(config, session, cancelled)
    }
    fn open_physical(
        &self,
        config: &Config,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
        authorization: tokio::sync::oneshot::Sender<bool>,
    ) -> Result<HostSession> {
        self.attach_physical(config, output, cancelled, authorization)
    }
}

impl LinuxHost {
    fn attach(
        &self,
        config: &Config,
        session: Session,
        cancelled: Arc<AtomicBool>,
    ) -> Result<HostSession> {
        ensure!(!cancelled.load(Ordering::Acquire), "startup cancelled");
        let target = session.target();
        if let Some(pid) = config.virtual_wayland_server_pid {
            #[cfg(feature = "portal")]
            {
                let executable = std::fs::read_link(format!("/proc/{pid}/exe"))?;
                let active: Arc<dyn DesktopBackend> = if executable
                    .file_name()
                    .is_some_and(|name| name == "gnome-shell")
                {
                    Arc::new(backend::mutter::Backend::start(target, pid, cancelled)?)
                } else if executable.file_name().is_some_and(|name| name == "labwc") {
                    Arc::new(crate::wayland::Backend::start_with_output_cancellable(
                        target
                            .wayland_socket
                            .context("owned Wayland socket missing")?,
                        config.fps,
                        config.quality,
                        None,
                        cancelled,
                    )?)
                } else {
                    Arc::new(backend::kwin::Backend::start(target, pid, cancelled)?)
                };
                return Ok(context(config, active, session));
            }
            #[cfg(not(feature = "portal"))]
            anyhow::bail!(
                "owned Wayland capture requires the portal build feature (compositor PID {pid})"
            );
        }
        let selected = backend::probe(&target, false, Some(&cancelled))?.require_available()?;
        let active: Arc<dyn DesktopBackend> =
            if !session.is_owned() && selected == BackendKind::Portal {
                Arc::new(backend::DeferredPortalBackend::new())
            } else {
                let active = backend::start_selected(
                    selected,
                    target.clone(),
                    config.fps,
                    config.quality,
                    None,
                    cancelled,
                )?;
                if session.is_owned() {
                    active
                } else {
                    backend::guarded::attach(active, target)?
                }
            };
        Ok(context(config, active, session))
    }
    fn attach_physical(
        &self,
        config: &Config,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
        authorization: tokio::sync::oneshot::Sender<bool>,
    ) -> Result<HostSession> {
        let target = Session::physical_target(config)?;
        let selection = backend::probe(&target, true, Some(&cancelled))?;
        let selected = selection.require_available()?;
        backend::guarded::require_attachable(&target, selected)?;
        let displays =
            config
                .display_backend
                .controller(target.clone(), environment(&target), None, false)?;
        let (outputs, output) =
            physical_output_selection(selected, output.as_deref(), displays.as_ref())?;
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop connection cancelled"
        );
        let session = Session::attach_physical(target.clone(), config.audio, cancelled.clone());
        if selected == BackendKind::Portal {
            let needs_confirmation = !selection.candidates.iter().any(|candidate| {
                Some(candidate.backend) == selection.selected && candidate.authorized == Some(true)
            });
            let _ = authorization.send(needs_confirmation);
        }
        let active = backend::start_selected(
            selected,
            target.clone(),
            config.fps,
            config.quality,
            output.clone(),
            cancelled.clone(),
        )?;
        let active = backend::guarded::attach(active, target)?;
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop connection cancelled"
        );
        let mut host = context(config, active, session);
        host.displays = displays;
        host.output = output;
        host.outputs = outputs;
        Ok(host)
    }
}
