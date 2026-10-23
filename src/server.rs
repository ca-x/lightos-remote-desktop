use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use stream_scheduler::{FrameScheduler, LatencySamples, RateTarget, SchedulingStrategy};

use axum::{
    Router,
    body::Body,
    extract::{
        OriginalUri, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use display_control::{
    DesktopEnvironment, DisplayController, DisplayMode, DisplayOutput, SessionTarget,
    WaylandDisplay, X11Display,
};
use futures_util::{SinkExt, Stream, StreamExt, stream::SplitSink};
use media_codec::{
    BackendPreference, CodecCapability, EncoderBackend, EncoderFactory, EncoderRate,
    EncoderSettings, HardwareEncoders, RgbaScaler, SoftwareEncoders, VideoCodec, VideoEncoder,
    choose_h264_backend, fit_dimensions,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::{
    sync::{mpsc, watch},
    time::timeout,
};

use crate::{
    audio::{AudioFormat, AudioStream},
    audio_channel::{self, AudioHub},
    backend::{self, BackendKind, BackendReport, DesktopBackend, InputCommand},
    clipboard::ImageUpload,
    config::Config,
    session::Session,
};

#[cfg(test)]
mod scaling_tests;

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DesktopMode {
    Virtual,
    Physical,
}

#[derive(Default, Deserialize)]
struct DesktopRequest {
    desktop: Option<DesktopMode>,
    output: Option<String>,
}

#[derive(Deserialize)]
struct AudioRequest {
    session: u64,
    #[serde(default)]
    codec: AudioFormat,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub rdp: Arc<crate::rdp::settings::Manager>,
    pub(crate) mcp: Arc<crate::mcp::TokenStore>,
    pub backend: Arc<dyn DesktopBackend>,
    pub session: Arc<Mutex<Session>>,
    controller: Arc<AtomicBool>,
    audio_hub: Arc<AudioHub>,
    cursor_hub: Arc<crate::cursor::Hub>,
    desktop: DesktopMode,
    output: Option<String>,
    outputs: Vec<DisplayOutput>,
    transient_backend: bool,
    displays: Arc<dyn DisplayController>,
    codec_capabilities: Arc<tokio::sync::OnceCell<Vec<CodecCapability>>>,
    physical_capabilities: Arc<Mutex<Option<PhysicalCapabilities>>>,
    #[cfg(test)]
    encode_pause: Option<Arc<EncodePause>>,
}

#[cfg(test)]
struct EncodePause {
    entered: tokio::sync::Notify,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

struct PhysicalCapabilities {
    target_key: String,
    checked_at: Instant,
    result: Result<BackendReport, String>,
}

fn session_displays(config: &Config, session: &Session, owned: bool) -> Arc<dyn DisplayController> {
    let target = session.target();
    match config.display_backend.controller(
        target.clone(),
        if target.wayland_socket.is_none() && target.x11_display.is_some() {
            DesktopEnvironment::X11
        } else {
            target.matching_environment_hint()
        },
        session.signature.as_deref(),
        owned,
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

// Capture can still use the X11 root window without RandR output metadata.
fn physical_output_selection(
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

impl AppState {
    pub fn new<B: DesktopBackend + 'static>(config: Config, backend: B, session: Session) -> Self {
        Self::with_backend(config, Arc::new(backend), session)
    }
    pub fn with_backend(
        config: Config,
        backend: Arc<dyn DesktopBackend>,
        session: Session,
    ) -> Self {
        let desktop = if !session.is_owned() {
            DesktopMode::Physical
        } else {
            DesktopMode::Virtual
        };
        let displays = session_displays(&config, &session, desktop == DesktopMode::Virtual);
        let mcp = Arc::new(crate::mcp::TokenStore::open(crate::mcp::settings_path(
            &config,
        )));
        let rdp = Arc::new(crate::rdp::settings::Manager::new(&config));
        Self {
            rdp,
            mcp,
            codec_capabilities: Arc::new(tokio::sync::OnceCell::new()),
            physical_capabilities: Arc::new(Mutex::new(None)),
            displays,
            config: Arc::new(config),
            backend,
            session: Arc::new(Mutex::new(session)),
            controller: Arc::new(AtomicBool::new(false)),
            audio_hub: Arc::new(AudioHub::default()),
            cursor_hub: Arc::new(crate::cursor::Hub::default()),
            desktop,
            output: None,
            outputs: Vec::new(),
            transient_backend: false,
            #[cfg(test)]
            encode_pause: None,
        }
    }

    /// Cancel agent sessions and queued input before HTTP drains its connections.
    pub fn shutdown_mcp(&self) {
        self.mcp.shutdown();
    }

    #[cfg(feature = "rdp")]
    pub(crate) fn claim_rdp_control(&self) -> anyhow::Result<ControllerLease> {
        anyhow::ensure!(
            self.desktop == DesktopMode::Virtual,
            "RDP cannot control a physical desktop"
        );
        anyhow::ensure!(
            self.controller
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "a browser or RDP controller is already connected"
        );
        Ok(ControllerLease {
            state: self.clone(),
        })
    }

    fn physical_target(&self) -> anyhow::Result<SessionTarget> {
        Session::physical_target(&self.config)
    }

    // Blocking probes use an explicit target and never open a consent dialog.
    fn probe_physical(
        &self,
        refresh: bool,
        cancelled: Option<&AtomicBool>,
    ) -> Result<BackendReport, String> {
        let target = self.physical_target().map_err(|error| error.to_string())?;
        let target_key = format!("{target:?}");
        if !refresh {
            let cache = self
                .physical_capabilities
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(cached) = cache.as_ref()
                && cached.target_key == target_key
                && cached.checked_at.elapsed() < Duration::from_secs(5)
            {
                return cached.result.clone();
            }
        }
        let result = backend::probe(&target, true, cancelled).map_err(|error| format!("{error:#}"));
        *self
            .physical_capabilities
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(PhysicalCapabilities {
            target_key,
            checked_at: Instant::now(),
            result: result.clone(),
        });
        result
    }

    fn desktop_options(&self, physical: Result<BackendReport, String>) -> serde_json::Value {
        let reason = match &physical {
            Ok(report) => report
                .require_available()
                .err()
                .map(|error| error.to_string()),
            Err(error) => Some(error.clone()),
        };
        // A transient physical connection still belongs to the same service.
        // Its base configuration determines whether the retained virtual
        // desktop can be selected again.
        let virtual_available =
            self.config.wayland_socket.is_none() && self.config.x11_display.is_none();
        json!([
            {"mode":"virtual","label":"虚拟桌面","available":virtual_available,"reason":if virtual_available { "" } else { "服务以外部桌面模式启动" }},
            {"mode":"physical","label":"物理桌面","available":reason.is_none(),"reason":reason,"capabilities":physical.ok()}
        ])
    }

    fn select_desktop(
        self,
        request: DesktopRequest,
        cancelled: Arc<AtomicBool>,
        connection_id: u64,
        authorization: tokio::sync::oneshot::Sender<()>,
    ) -> anyhow::Result<Self> {
        use anyhow::{Context, ensure};
        let started = Instant::now();
        let mode = request.desktop.unwrap_or(self.desktop);
        if mode == self.desktop
            && request.output.is_none()
            && !self.backend.is_deferred()
            && (mode == DesktopMode::Virtual || self.backend.frame_is_fresh(Duration::from_secs(5)))
        {
            return Ok(self);
        }
        ensure!(mode == DesktopMode::Physical, "虚拟桌面不可用");
        let target = self.physical_target()?;
        let selection = self
            .probe_physical(true, Some(&cancelled))
            .map_err(anyhow::Error::msg)?;
        let selected_backend = selection.require_available()?;
        let hint = if target.wayland_socket.is_none() {
            DesktopEnvironment::X11
        } else {
            target.matching_environment_hint()
        };
        let displays = self
            .config
            .display_backend
            .controller(target.clone(), hint, None, false)?;
        // Portal chooses its granted stream; X11 can capture the root without RandR.
        let (outputs, selected) = physical_output_selection(
            selected_backend,
            request.output.as_deref(),
            displays.as_ref(),
        )?;
        ensure!(!cancelled.load(Ordering::Acquire), "桌面连接已取消");
        let mut config = (*self.config).clone();
        config.session_runtime = None;
        config.wayland_socket = target.wayland_socket.clone();
        config.x11_display = target.x11_display.clone();
        config.xauthority = target.xauthority.clone();
        let session = Session::start_cancellable(&config, cancelled.clone())?;
        if selected_backend == BackendKind::Portal {
            let _ = authorization.send(());
        }
        let backend = backend::start_selected(
            selected_backend,
            target,
            config.fps,
            config.quality,
            selected.clone(),
            cancelled.clone(),
        )
        .context("物理桌面采集或输入初始化失败")?;
        let backend = backend::guarded::attach(backend, session.target())?;
        ensure!(!cancelled.load(Ordering::Acquire), "桌面连接已取消");
        tracing::info!(connection_id, elapsed_ms = started.elapsed().as_millis() as u64, kind = ?backend.kind(), "physical capture and input ready");
        Ok(Self {
            rdp: self.rdp,
            codec_capabilities: self.codec_capabilities,
            physical_capabilities: self.physical_capabilities,
            mcp: self.mcp,
            displays,
            config: self.config,
            controller: self.controller,
            audio_hub: self.audio_hub,
            backend,
            cursor_hub: self.cursor_hub,
            session: Arc::new(Mutex::new(session)),
            desktop: mode,
            output: selected,
            outputs,
            transient_backend: true,
            #[cfg(test)]
            encode_pause: self.encode_pause,
        })
    }
}

pub fn router(state: AppState) -> Router {
    let mcp = crate::mcp::transport(
        state.mcp.clone(),
        state.backend.clone(),
        state.controller.clone(),
        state.desktop == DesktopMode::Virtual,
    );
    let desktop = Router::new()
        .route(
            "/app.js",
            get(|headers: HeaderMap| async move {
                asset(
                    &headers,
                    include_str!("../web/dist/app.js"),
                    include_bytes!("../web/dist/app.js.gz"),
                    "text/javascript; charset=utf-8",
                )
            }),
        )
        .route(
            "/style.css",
            get(|headers: HeaderMap| async move {
                asset(
                    &headers,
                    include_str!("../web/dist/style.css"),
                    include_bytes!("../web/dist/style.css.gz"),
                    "text/css; charset=utf-8",
                )
            }),
        )
        .route("/ws", get(upgrade))
        .route(
            "/audio-worker.js",
            get(|headers: HeaderMap| async move {
                asset(
                    &headers,
                    include_str!("../web/dist/audio-worker.js"),
                    include_bytes!("../web/dist/audio-worker.js.gz"),
                    "text/javascript; charset=utf-8",
                )
            }),
        )
        .route(
            "/audio-worklet.js",
            get(|headers: HeaderMap| async move {
                asset(
                    &headers,
                    include_str!("../web/dist/audio-worklet.js"),
                    include_bytes!("../web/dist/audio-worklet.js.gz"),
                    "text/javascript; charset=utf-8",
                )
            }),
        )
        .route("/audio", get(upgrade_audio))
        .route("/cursor", get(upgrade_cursor))
        .route("/desktops", get(desktops))
        .merge(crate::rdp::settings::routes())
        .merge(crate::mcp::management(state.mcp.clone()).with_state(()));
    let desktop = mount_desktop(desktop, &state.config.subfolder)
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let mcp: Router<AppState> = if state.config.subfolder.trim_matches('/').is_empty() {
        mcp.with_state(())
    } else {
        Router::new()
            .nest(
                &format!("/{}", state.config.subfolder.trim_matches('/')),
                mcp,
            )
            .with_state(())
    };
    desktop
        .merge(mcp)
        .route("/healthz", get(health))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

fn mount_desktop<State: Clone + Send + Sync + 'static>(
    desktop: Router<State>,
    prefix: &str,
) -> Router<State> {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return desktop.route("/", get(index));
    }
    let prefix = format!("/{prefix}");
    Router::new()
        .nest(&prefix, desktop)
        .route(&prefix, get(index))
        .route(&format!("{prefix}/"), get(index))
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|entry| {
                let mut parts = entry.trim().split(';');
                if !parts
                    .next()
                    .is_some_and(|name| name.trim().eq_ignore_ascii_case("gzip"))
                {
                    return false;
                }
                parts.all(|parameter| match parameter.trim().split_once('=') {
                    Some((name, quality)) if name.trim().eq_ignore_ascii_case("q") => quality
                        .trim()
                        .parse::<f32>()
                        .is_ok_and(|quality| quality > 0.0 && quality <= 1.0),
                    _ => true,
                })
            })
        })
}

fn asset(
    headers: &HeaderMap,
    content: &'static str,
    compressed: &'static [u8],
    mime: &'static str,
) -> Response {
    let compressed_response = accepts_gzip(headers);
    let mut response = if compressed_response {
        compressed.into_response()
    } else {
        content.into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, mime.parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    headers.insert(header::VARY, "Accept-Encoding".parse().unwrap());
    if compressed_response {
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
    }
    response
}

async fn index(OriginalUri(uri): OriginalUri) -> Response {
    if !uri.path().ends_with('/') {
        return axum::response::Redirect::temporary(&format!("{}/", uri.path())).into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../web/dist/index.html")
            .replace("__LIGHTOS_VERSION__", env!("CARGO_PKG_VERSION")),
    )
        .into_response()
}

pub fn credentials_match(headers: &HeaderMap, username: &str, password: &str) -> bool {
    let Some(encoded) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic "))
        .filter(|value| value.len() <= 8192)
    else {
        return false;
    };
    let Ok(decoded) = STANDARD.decode(encoded) else {
        return false;
    };
    let Some(separator) = decoded.iter().position(|byte| *byte == b':') else {
        return false;
    };
    let user_ok = decoded[..separator].ct_eq(username.as_bytes());
    let password_ok = decoded[separator + 1..].ct_eq(password.as_bytes());
    bool::from(user_ok & password_ok)
}

async fn authenticate(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !credentials_match(
        request.headers(),
        &state.config.username,
        &state.config.password,
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                "Basic realm=\"LightOS remote desktop\", charset=\"UTF-8\"",
            )],
            "Authentication required",
        )
            .into_response();
    }
    next.run(request).await
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "SAMEORIGIN"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; worker-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' blob: data:; frame-ancestors 'self'; base-uri 'none'; form-action 'none'",
        ),
    ] {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(name),
            axum::http::HeaderValue::from_static(value),
        );
    }
    response
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let ready = !state.backend.is_stopped()
        && (state.backend.is_deferred()
            || (!state.backend.paused() && state.backend.frame_is_fresh(Duration::from_secs(5))))
        && state
            .session
            .lock()
            .is_ok_and(|mut session| session.healthy());
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        axum::Json(
            json!({"native":true,"ready":ready,"requires_consent":state.backend.is_deferred(),"desktop_backend":state.backend.kind(),"pid":std::process::id(),"transport":"websocket","video_codecs":state.codec_capabilities.get().map(|caps| caps.iter().filter(|c| c.available).map(|c| c.codec).collect::<Vec<_>>()).unwrap_or_default(),"codec_probe_complete":state.codec_capabilities.initialized()}),
        ),
    )
}

async fn desktops(State(state): State<AppState>) -> impl IntoResponse {
    let current = state.desktop;
    let fallback = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Refresh is explicit user discovery; ready messages may reuse this short-lived result.
        let physical = state.probe_physical(true, None);
        let options = state.desktop_options(physical.clone());
        let outputs = (|| -> anyhow::Result<Vec<DisplayOutput>> {
            let selection = physical.map_err(anyhow::Error::msg)?;
            if selection.require_available()? == BackendKind::Portal {
                return Ok(Vec::new());
            }
            let target = state.physical_target()?;
            let hint = if target.wayland_socket.is_none() {
                DesktopEnvironment::X11
            } else {
                target.matching_environment_hint()
            };
            state
                .config
                .display_backend
                .controller(target, hint, None, false)?
                .outputs()
        })();
        (options, outputs)
    })
    .await;
    let (options, outputs, reason) = match result {
        Ok((options, Ok(outputs))) => (options, outputs, None),
        Ok((options, Err(error))) => (options, Vec::new(), Some(error.to_string())),
        Err(_) => (
            fallback.desktop_options(Err("显示器检测失败".to_owned())),
            Vec::new(),
            Some("显示器检测失败".to_owned()),
        ),
    };
    axum::Json(json!({"current":current,"desktops":options,"outputs":outputs,"reason":reason}))
}

pub fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    (origin == format!("https://{host}") || origin == format!("http://{host}"))
        && !headers
            .get("sec-fetch-site")
            .is_some_and(|value| value == "cross-site")
}

pub(crate) struct ControllerLease {
    state: AppState,
}

impl ControllerLease {
    pub(crate) async fn release(self) {
        // Worker joins must not block the async socket executor. The lease's
        // Drop keeps the controller gate until its input workers have stopped.
        let _ = tokio::task::spawn_blocking(move || drop(self)).await;
    }
}

impl Drop for ControllerLease {
    fn drop(&mut self) {
        self.state.audio_hub.stop();
        if self.state.transient_backend || !self.state.backend.release_input() {
            self.state.backend.stop();
        }
        self.state.controller.store(false, Ordering::Release);
    }
}

async fn upgrade(
    State(state): State<AppState>,
    Query(request): Query<DesktopRequest>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "Same-origin WebSocket required").into_response();
    }
    if state
        .controller
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return (StatusCode::CONFLICT, "A controller is already connected").into_response();
    }
    let mut lease = ControllerLease {
        state: state.clone(),
    };
    let connection_id = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
    let mode = request.desktop.unwrap_or(state.desktop);
    let started = Instant::now();
    tracing::info!(connection_id, desktop = ?mode, "desktop connection accepted");
    upgrade
        .max_message_size(128 * 1024)
        .max_frame_size(128 * 1024)
        .on_upgrade(move |mut socket| async move {
            let cancelled = Arc::new(AtomicBool::new(false));
            let initialize_cancelled = cancelled.clone();
            let (authorization, mut authorization_notice) = tokio::sync::oneshot::channel();
            let mut initialize = tokio::task::spawn_blocking(move || {
                state.select_desktop(request, initialize_cancelled, connection_id, authorization)
            });
            let mut awaiting_authorization = true;
            let result = loop {
                tokio::select! {
                    biased;
                    result = &mut initialize => break Some(result),
                    notice = &mut authorization_notice, if awaiting_authorization => {
                        awaiting_authorization = false;
                        if notice.is_ok() {
                            let message = Message::Text(json!({"type":"desktop_initializing", "requires_consent":true}).to_string().into());
                            if !matches!(timeout(Duration::from_secs(1), socket.send(message)).await, Ok(Ok(()))) {
                                break None;
                            }
                        }
                    },
                    _ = startup_disconnect(&mut socket) => break None,
                }
            };
            let Some(result) = result else {
                cancelled.store(true, Ordering::Release);
                let _ = timeout(Duration::from_secs(1), socket.close()).await;
                let result = initialize.await;
                let _ = tokio::task::spawn_blocking(move || drop(result)).await;
                lease.release().await;
                tracing::info!(connection_id, elapsed_ms = started.elapsed().as_millis() as u64, "desktop initialization cancelled; controller released");
                return;
            };
            match result {
                Ok(Ok(selected)) => {
                    lease.state = selected.clone();
                    let end = connection(socket, selected, connection_id, started).await;
                    lease.release().await;
                    tracing::info!(connection_id, "desktop controller released");
                    if let Some(mut end) = end {
                        if end.release_requested {
                            let acknowledgement = Message::Text(json!({"type":"released"}).to_string().into());
                            let _ = timeout(Duration::from_secs(1), end.writer.send(acknowledgement)).await;
                        }
                        let close = timeout(Duration::from_secs(1), end.writer.close()).await;
                        tracing::info!(connection_id, flushed = matches!(close, Ok(Ok(()))), "desktop close handshake");
                    }
                    return;
                }
                result => {
                    let error = match result {
                        Ok(Err(error)) => format!("{error:#}"),
                        _ => "桌面连接失败".to_owned(),
                    };
                    tracing::warn!(connection_id, %error, elapsed_ms = started.elapsed().as_millis() as u64, "desktop initialization failed");
                    let failure = Message::Text(json!({"type":"error", "message":error, "retryable":false}).to_string().into());
                    let _ = timeout(Duration::from_secs(3), socket.send(failure)).await;
                    let _ = timeout(Duration::from_secs(1), socket.close()).await;
                }
            }
            lease.release().await;
            tracing::info!(connection_id, "desktop controller released");
        })
}

async fn startup_disconnect(socket: &mut WebSocket) {
    loop {
        match socket.next().await {
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                if !matches!(
                    timeout(Duration::from_secs(1), socket.flush()).await,
                    Ok(Ok(()))
                ) {
                    return;
                }
            }
            Some(Ok(Message::Text(_) | Message::Binary(_))) => {}
            _ => return,
        }
    }
}

#[derive(Deserialize)]
struct CursorRequest {
    session: String,
}
async fn upgrade_cursor(
    State(state): State<AppState>,
    Query(request): Query<CursorRequest>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "Same-origin WebSocket required").into_response();
    }
    let Some(lease) = state.cursor_hub.attach(&request.session) else {
        return (StatusCode::CONFLICT, "Cursor session unavailable").into_response();
    };
    upgrade
        .max_message_size(1024)
        .on_upgrade(move |socket| crate::cursor::connection(socket, lease))
}

async fn upgrade_audio(
    State(state): State<AppState>,
    Query(request): Query<AudioRequest>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "Same-origin WebSocket required").into_response();
    }
    let Some(lease) = state.audio_hub.attach(request.session) else {
        return (
            StatusCode::CONFLICT,
            "Audio session unavailable or already connected",
        )
            .into_response();
    };
    let session = state.session.lock().unwrap();
    let source = session.audio_source.clone();
    let server = session.pulse_server.clone();
    drop(session);
    let Some(source) = source.filter(|_| state.config.audio) else {
        return (StatusCode::CONFLICT, "Audio unavailable").into_response();
    };
    upgrade
        .max_message_size(1024)
        .max_frame_size(1024)
        .on_upgrade(move |socket| {
            audio_channel::connection(socket, lease, server, source, request.codec)
        })
}

struct ConnectionEnd {
    writer: SplitSink<WebSocket, Message>,
    release_requested: bool,
}

async fn connection(
    socket: WebSocket,
    state: AppState,
    connection_id: u64,
    connection_started: Instant,
) -> Option<ConnectionEnd> {
    let (mut writer, reader) = socket.split();
    let (control_tx, mut controls) = mpsc::channel(16);
    let (pacing_tx, mut pacing) =
        watch::channel(FrameScheduler::new(state.config.fps).expect("validated frame rate"));
    let receive = receive_commands(reader, state.backend.input(), control_tx, pacing_tx.clone());
    let mut frames = state.backend.frames();
    let dimensions = frames
        .borrow()
        .as_ref()
        .map(|frame| (frame.width, frame.height));
    let (width, height) = dimensions?;
    let audio_source = state
        .session
        .lock()
        .ok()
        .and_then(|session| session.audio_source.clone());
    let audio_available = state.config.audio && audio_source.is_some();
    let audio_session = audio_available.then(|| state.audio_hub.start());
    let display_capabilities = state.displays.capabilities().await;
    let cursor_scope = state.cursor_hub.start(state.backend.clone());
    let codec_capabilities = state
        .codec_capabilities
        .get_or_init(|| async {
            let hardware = HardwareEncoders::discover();
            let (mut caps, native) = tokio::join!(SoftwareEncoders.probe(), hardware.probe());
            caps.extend(native);
            caps
        })
        .await;
    let video_codecs: Vec<_> = codec_capabilities
        .iter()
        .filter(|c| c.available)
        .map(|c| c.codec)
        .collect();
    let discovery = state.clone();
    let desktop_options = tokio::task::spawn_blocking(move || {
        discovery.desktop_options(discovery.probe_physical(false, None))
    })
    .await
    .ok()?;
    let ready = Message::Text(
        json!({"type":"ready","key_chords":true,"image_clipboard":state.backend.supports_image_clipboard(),"backend_selection":true,"video_request_ids":true,"video_scaling":true,"control_release":true,"resize_scale":display_capabilities.resize_scale,"resize_scales":if display_capabilities.resize_scale { DisplayMode::SUPPORTED_SCALES.as_slice() } else { &[] },"cursor_session":if state.backend.kind() == BackendKind::Wayland { Some(cursor_scope.token()) } else { None },"version":env!("CARGO_PKG_VERSION"),"width":width,"height":height,"cursor":if state.backend.cursor_embedded() { "embedded" } else { "default" },"audio":audio_available,"audio_session":audio_session,"audio_codecs":["opus","pcm"],"frame_ack":true,"frame_ids":true,"metrics":true,"video_codecs":video_codecs,"codec_capabilities":codec_capabilities,"desktop":state.desktop,"desktop_backend":state.backend.kind(),"desktops":desktop_options,"resize":display_capabilities.resize,"resize_reason":display_capabilities.reason,"outputs":state.outputs,"output":state.output})

            .to_string()
            .into(),
    );
    if !matches!(
        timeout(Duration::from_secs(3), writer.send(ready)).await,
        Ok(Ok(()))
    ) {
        return None;
    }
    tracing::info!(
        connection_id,
        elapsed_ms = connection_started.elapsed().as_millis() as u64,
        "desktop ready sent"
    );
    pacing_tx.send_modify(|flow| flow.set_quality_scalable(state.config.quality > 20));
    let mut h264_dynamic_rate = codec_capabilities
        .iter()
        .any(|c| c.codec == VideoCodec::H264 && c.available && c.dynamic_bitrate);
    let mut audio = None;
    let mut clipboard_upload = None;
    let mut audio_buffer = vec![0_u8; 3840];
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut qos_ticks = tokio::time::interval(Duration::from_millis(250));
    let mut applied_rate = None;
    let mut applied_jpeg_quality = None;
    let hardware = HardwareEncoders::discover();
    let mut backend_preference = BackendPreference::Auto;
    let mut excluded_backends = Vec::new();
    let mut selected_backend: Option<EncoderBackend> = None;
    let mut fallback_reason: Option<String> = None;
    let mut h264 = false;
    let mut video_request_id = None;
    let mut video_bounds = None;
    let mut scaler = None;
    let mut encoder: Option<Box<dyn VideoEncoder>> = None;
    let mut codec = None;
    let video_started = std::time::Instant::now();
    let mut frame_id = 0_u64;
    let mut first_video_sent = false;
    let mut encode_samples = LatencySamples::default();
    let mut send_samples = LatencySamples::default();
    let transmit = async {
        let mut capture_paused = false;
        let mut observed_epoch = state.backend.privacy_epoch();
        // An old H264 encoder can retain raw pixels even when a lock and
        // unlock complete before the next frame is published.
        macro_rules! discard_stale_frame {
            ($epoch:expr) => {
                if !frame_privacy_current(state.backend.as_ref(), $epoch) {
                    encoder.take();
                    scaler = None;
                    codec = None;
                    applied_rate = None;
                    applied_jpeg_quality = None;
                    pacing_tx.send_modify(FrameScheduler::reset);
                    continue;
                }
            };
        }
        loop {
            let current_epoch = state.backend.privacy_epoch();
            if current_epoch != observed_epoch {
                scaler = None;
                encoder.take();
                codec = None;
                applied_rate = None;
                applied_jpeg_quality = None;
                pacing_tx.send_modify(FrameScheduler::reset);
                observed_epoch = current_epoch;
            }
            let frame_deadline = pacing.borrow().next_frame_at();
            let refresh_requested = !capture_paused && pacing.borrow().refresh_requested();
            let mut outgoing_epoch = None;
            let outgoing = tokio::select! {
                changed = wait_for_video_frame(&mut frames, refresh_requested, frame_deadline), if !pacing.borrow().waiting() || state.backend.paused() => {
                    if changed.is_err() { break; }
                    let frame_epoch = state.backend.privacy_epoch();
                    let frame = frames.borrow_and_update().clone();
                    let Some(frame) = frame else {
                        if state.backend.is_stopped() { break; }
                        if !capture_paused {
                            capture_paused = true;
                            scaler = None;
                            encoder.take(); codec = None;
                            pacing_tx.send_modify(FrameScheduler::reset);
                            let paused = Message::Text(json!({"type":"desktop_state","available":false}).to_string().into());
                            if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(paused)).await, Ok(Ok(()))) { break; }
                        }
                        continue;
                    };
                    discard_stale_frame!(frame_epoch);
                    if capture_paused {
                        capture_paused = false;
                        let resumed = Message::Text(json!({"type":"desktop_state","available":true}).to_string().into());
                        if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(resumed)).await, Ok(Ok(()))) { break; }
                        discard_stale_frame!(frame_epoch);
                    }
                    let cursor_embedded = frame.cursor_embedded;
                    let source_size = (frame.width, frame.height);
                    let bounds = {
                        let flow = pacing.borrow();
                        if flow.sequenced() && flow.strategy() != SchedulingStrategy::QualityFirst { video_bounds } else { None }
                    };
                    let (width, height) = fit_dimensions(source_size, bounds);
                    pacing_tx.send_modify(|flow| flow.begin_frame((width, height)));
                    let target = pacing.borrow().target();
                    let encode_started = Instant::now();
                    pacing_tx.send_modify(|flow| flow.frame_started(encode_started));
                    let scaled_pixels = if (width, height) != source_size {
                        let mut worker = scaler.take().unwrap_or_else(RgbaScaler::default);
                        let pixels = frame.pixels.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            let pixels = worker.resize(pixels, source_size, (width, height));
                            (worker, pixels)
                        }).await;
                        let pixels = match result {
                            Ok((worker, Ok(pixels))) => { scaler = Some(worker); pixels }
                            Ok((_, Err(error))) => { tracing::warn!(%error, "video scaling failed"); break; }
                            Err(error) => { tracing::warn!(%error, "video scaling worker failed"); break; }
                        };
                        discard_stale_frame!(frame_epoch);
                        Some(pixels)
                    } else { scaler = None; None };
                    let mut packet = None;
                    if h264 {
                        let result = async {
                            if encoder.as_ref().is_none_or(|encoder| encoder.dimensions() != (width, height)) {
                                let settings = EncoderSettings {width,height,fps:state.config.fps,quality:state.config.quality};
                                let backend = selected_backend.ok_or_else(|| anyhow::anyhow!("no validated H264 backend"))?;
                                encoder = Some(if backend.is_hardware() { hardware.create(backend, settings)? } else { SoftwareEncoders.create(VideoCodec::H264, settings)? });
                                codec = None;
                            }
                            let encoder = encoder.as_mut().ok_or_else(|| anyhow::anyhow!("encoder missing"))?;
                            let rate = encoder_rate(width, height, state.config.fps, target);
                            if h264_dynamic_rate { encoder.set_rate(rate)?; }
                            let encoded = encoder.encode(scaled_pixels.as_ref().unwrap_or(&frame.pixels).clone()).await?;
                            applied_rate = h264_dynamic_rate.then_some(rate);
                            applied_jpeg_quality = None;
                            Ok::<_, anyhow::Error>(encoded)
                        }.await;
                        discard_stale_frame!(frame_epoch);
                        match result {
                            Ok(encoded) => {
                                if encoded.data.is_empty() {
                                    // OpenH264 skipped this raw input to honor its
                                    // bitrate. Retry the latest raw capture after
                                    // pacing, including when the desktop is static.
                                    pacing_tx.send_modify(FrameScheduler::frame_skipped);
                                    encode_samples.record(encode_started.elapsed());
                                    continue;
                                }
                                if let Some(configuration) = encoded.codec
                                    && codec.as_ref() != Some(&configuration) {
                                        discard_stale_frame!(frame_epoch);
                                        let message = Message::Text(json!({"type":"video_config","request_id":video_request_id,"codec":configuration,"width":width,"height":height,"backend":selected_backend.map(EncoderBackend::id),"fallback_reason":fallback_reason}).to_string().into());
                                        if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await, Ok(Ok(()))) { break; }
                                        discard_stale_frame!(frame_epoch);
                                        codec = Some(configuration);
                                }
                                let mut bytes = Vec::with_capacity(encoded.data.len() + 10);
                                bytes.extend_from_slice(&[3, u8::from(encoded.key)]);
                                bytes.extend_from_slice(&(video_started.elapsed().as_micros() as u64).to_le_bytes());
                                bytes.extend_from_slice(&encoded.data);
                                packet = Some(bytes);
                            }
                            Err(error) => {
                                tracing::warn!(%error, ?selected_backend, "H264 backend failed; selecting fallback");
                                if let Some(backend) = selected_backend.take() { excluded_backends.push(backend); }
                                fallback_reason = Some(format!("{error:#}"));
                                selected_backend = choose_h264_backend(backend_preference, codec_capabilities, &excluded_backends);
                                h264 = selected_backend.is_some();
                                h264_dynamic_rate = selected_backend.is_some_and(|backend| codec_capabilities.iter().any(|cap| cap.backend == backend.id() && cap.available && cap.dynamic_bitrate));
                                pacing_tx.send_modify(|flow| { flow.reset(); flow.set_quality_scalable(if h264 { h264_dynamic_rate } else { state.config.quality > 20 }); });
                                encoder.take(); codec = None;
                                applied_rate = None;
                                if h264 { continue; }
                                discard_stale_frame!(frame_epoch);
                                let message = Message::Text(json!({"type":"video_config","request_id":video_request_id,"codec":"jpeg","backend":"turbojpeg","fallback_reason":fallback_reason}).to_string().into());
                                if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await, Ok(Ok(()))) { break; }
                                discard_stale_frame!(frame_epoch);
                            }
                        }
                    }
                    let packet = match packet {
                        Some(packet) => packet,
                        None => {
                            let quality = jpeg_quality(state.config.quality, pacing.borrow().target().quality_percent);
                            applied_jpeg_quality = Some(quality);
                            applied_rate = None;
                            #[cfg(test)]
                            let encode_pause = state.encode_pause.clone();
                            let jpeg = match tokio::task::spawn_blocking(move || {
                                #[cfg(test)]
                                if let Some(pause) = encode_pause {
                                    let release = pause.release.lock().unwrap().take();
                                    if let Some(release) = release {
                                        pause.entered.notify_one();
                                        release.recv_timeout(Duration::from_secs(3)).expect("resume JPEG encode");
                                    }
                                }
                                match scaled_pixels {
                                    Some(pixels) => media_codec::encode_jpeg(width, height, &pixels, quality),
                                    None => frame.jpeg(quality),
                                }
                            }).await {
                                Ok(Ok(jpeg)) => jpeg,
                                result => {
                                    tracing::warn!(?result, "JPEG encoding failed");
                                    break;
                                }
                            };
                            discard_stale_frame!(frame_epoch);
                            let mut packet = Vec::with_capacity(jpeg.len() + 1);
                            packet.push(1); packet.extend_from_slice(&jpeg); packet
                        }
                    };
                    discard_stale_frame!(frame_epoch);
                    frame_id += 1;
                    pacing_tx.send_modify(|flow| flow.sent(frame_id, Instant::now()));
                    if pacing.borrow().sequenced() {
                        discard_stale_frame!(frame_epoch);
                        let metadata = Message::Text(json!({"type":"frame_id","id":frame_id,"source_width":source_size.0,"source_height":source_size.1,"cursor_embedded":if state.backend.kind() == BackendKind::Wayland { Some(cursor_embedded) } else { None }}).to_string().into());
                        if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(metadata)).await, Ok(Ok(()))) { break; }
                        discard_stale_frame!(frame_epoch);
                    }
                    encode_samples.record(encode_started.elapsed());
                    outgoing_epoch = Some(frame_epoch);
                    Some(Message::Binary(packet.into()))
                }
                _ = qos_ticks.tick() => {
                    pacing_tx.send_if_modified(|flow| {
                        let before = flow.target();
                        flow.tick(Instant::now());
                        before != flow.target()
                    });
                    None
                }
                changed = pacing.changed() => {
                    if changed.is_err() { break; }
                    None
                }
                packet = read_audio(&mut audio, &mut audio_buffer) => {
                    match packet {
                        Ok(length) if length > 0 => {
                            let mut bytes = vec![2]; bytes.extend_from_slice(&audio_buffer[..length]);
                            Some(Message::Binary(bytes.into()))
                        }
                        _ => { audio.take(); Some(error_message("Audio stream stopped")) }
                    }
                }
                _ = heartbeat.tick() => {
                    Some(Message::Ping(bytes::Bytes::new()))
                }
                incoming = controls.recv() => {
                    match incoming {
                        Some(text) => {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                                && value.get("type").and_then(|kind| kind.as_str()) == Some("ping")
                                && let Some(id) = value.get("id").and_then(|id| id.as_u64())
                            {
                                let flow = pacing.borrow();
                                let message = Message::Text(json!({"type":"pong","id":id,"video_metrics":{
                                    "ack":flow.ack_summary(),"encode":encode_samples.summary(),"send":send_samples.summary(),
                                    "in_flight":flow.in_flight(),"window":flow.window(),"strategy":flow.strategy(),
                                    "qos":flow.qos_summary(),"encoder_backend":selected_backend.map(EncoderBackend::id).unwrap_or("turbojpeg"),"fallback_reason":fallback_reason,"encoder_rate":applied_rate,"jpeg_quality":applied_jpeg_quality
                                }}).to_string().into());
                                drop(flow);
                                Some(message)
                            } else {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                                && value.get("type").and_then(|kind| kind.as_str()) == Some("video_scale")
                            {
                                match parse_video_bounds(&value) {
                                    Ok(bounds) => {
                                        if video_bounds != bounds {
                                            video_bounds = bounds;
                                            pacing_tx.send_modify(FrameScheduler::reset);
                                        }
                                    }
                                    Err(error) => {
                                        let message = error_message(&error.to_string());
                                        if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await, Ok(Ok(()))) { break; }
                                    }
                                }
                                continue;
                            }
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                                && value.get("type").and_then(|kind| kind.as_str()) == Some("video")
                            {
                                video_request_id = value.get("request_id").and_then(|id| id.as_u64()).filter(|id| (1..=9_007_199_254_740_991).contains(id));
                                fallback_reason = None;
                                if let Some(preference) = value.get("backend").and_then(|v| serde_json::from_value(v.clone()).ok()) { backend_preference = preference; }
                                if let Some(rejected) = value.get("rejected_backend").and_then(|v| v.as_str())
                                    && let Some(active) = selected_backend
                                    && active.id() == rejected {
                                        if !excluded_backends.contains(&active) { excluded_backends.push(active); }
                                        fallback_reason = Some(format!("Browser rejected {rejected} H264 profile"));
                                }
                                selected_backend = choose_h264_backend(backend_preference, codec_capabilities, &excluded_backends);
                                if let Some(requested) = backend_preference.requested_backend()
                                    && selected_backend != Some(requested) {
                                        fallback_reason = Some(codec_capabilities.iter().find(|cap| cap.backend == requested.id()).and_then(|cap| cap.reason.clone()).unwrap_or_else(|| format!("{} unavailable for this connection", requested.id())));
                                }
                                h264 = value.get("codec").and_then(|codec| codec.as_str()) == Some("h264") && pacing.borrow().sequenced() && selected_backend.is_some();
                                if !h264 { selected_backend = None; }
                                h264_dynamic_rate = selected_backend.is_some_and(|backend| codec_capabilities.iter().any(|cap| cap.backend == backend.id() && cap.available && cap.dynamic_bitrate));
                                encoder.take(); codec = None;
                                pacing_tx.send_modify(|flow| {
                                    flow.reset();
                                    flow.set_quality_scalable(if h264 { h264_dynamic_rate } else { state.config.quality > 20 });
                                });
                                applied_rate = None; applied_jpeg_quality = None;
                                if !h264 {
                                    let message = Message::Text(json!({"type":"video_config","request_id":video_request_id,"codec":"jpeg","backend":"turbojpeg","fallback_reason":fallback_reason}).to_string().into());
                                    if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await, Ok(Ok(()))) { break; }
                                }
                                continue;
                            }
                            match handle_command(&state, &text, &mut audio, audio_source.as_deref(), &mut clipboard_upload).await {
                                Ok(response) => response,
                                Err(error) => Some(error_message(&error.to_string())),
                            }
                            }
                        }
                        _ => break,
                    }
                }
            };
            if let Some(message) = outgoing {
                let video_packet = matches!(&message, Message::Binary(bytes) if matches!(bytes.first(), Some(1 | 3)));
                if let Some(epoch) = outgoing_epoch {
                    discard_stale_frame!(epoch);
                }
                if video_packet && state.backend.paused() {
                    continue;
                }
                let started = Instant::now();
                if !matches!(
                    timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await,
                    Ok(Ok(()))
                ) {
                    break;
                }
                if video_packet {
                    send_samples.record(started.elapsed());
                    if !first_video_sent {
                        first_video_sent = true;
                        tracing::info!(
                            connection_id,
                            elapsed_ms = connection_started.elapsed().as_millis() as u64,
                            "first desktop frame sent"
                        );
                    }
                }
            }
        }
    };
    let release_requested =
        tokio::select! { requested = receive => requested, _ = transmit => false };
    audio.take();
    let _ = timeout(
        Duration::from_secs(1),
        state.backend.input().send(InputCommand::Release),
    )
    .await;
    Some(ConnectionEnd {
        writer,
        release_requested,
    })
}

// A transient slow link must not tear down an otherwise live desktop. Input
// receive remains independently polled while a video write waits. A timed-out
// write still closes the transport: a partial WS frame cannot be retried.
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

fn frame_privacy_current(backend: &dyn DesktopBackend, epoch: u64) -> bool {
    !backend.paused() && !backend.is_stopped() && backend.privacy_epoch() == epoch
}

fn encoder_rate(width: u32, height: u32, cap: u32, target: RateTarget) -> EncoderRate {
    let bitrate = u64::from(media_codec::default_bitrate(width, height))
        * u64::from(target.quality_percent)
        * u64::from(target.fps)
        / (100 * u64::from(cap));
    EncoderRate {
        bitrate_bps: bitrate.max(1_000) as u32,
        fps: target.fps,
    }
}
/// null explicitly restores native encoding. Missing/malformed bounds must not
/// silently alter a running stream or trigger an unbounded allocation.
fn parse_video_bounds(value: &serde_json::Value) -> anyhow::Result<Option<(u32, u32)>> {
    #[derive(Deserialize)]
    struct Bounds {
        width: u32,
        height: u32,
    }
    let bounds: Option<Bounds> = serde_json::from_value(
        value
            .get("max_size")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing video max_size"))?,
    )?;
    bounds
        .map(|bounds| {
            anyhow::ensure!(
                (1..=8192).contains(&bounds.width)
                    && (1..=8192).contains(&bounds.height)
                    && u64::from(bounds.width) * u64::from(bounds.height) <= 33_554_432,
                "invalid video max_size"
            );
            Ok((bounds.width, bounds.height))
        })
        .transpose()
}

fn jpeg_quality(configured: u8, percent: u32) -> u8 {
    configured
        .saturating_sub(((100 - percent.min(100)) / 2) as u8)
        .max(configured.min(20))
}

// Wait before reading the latest capture so stale raw frames are discarded.
// Refreshes (policy changes, native skips) also work without a pixel notification.
async fn wait_for_video_frame<T>(
    frames: &mut watch::Receiver<T>,
    refresh: bool,
    deadline: Option<Instant>,
) -> Result<(), watch::error::RecvError> {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline.into()).await;
    }
    if refresh {
        Ok(())
    } else {
        frames.changed().await
    }
}

async fn receive_commands(
    mut reader: impl Stream<Item = Result<Message, axum::Error>> + Unpin,
    input: mpsc::Sender<InputCommand>,
    controls: mpsc::Sender<String>,
    pacing: watch::Sender<FrameScheduler>,
) -> bool {
    while let Ok(Some(Ok(message))) = timeout(Duration::from_secs(60), reader.next()).await {
        match message {
            Message::Text(text) => {
                if text.len() > 96 * 1024 {
                    break;
                }
                if let Ok(command) = serde_json::from_str::<InputCommand>(&text)
                    && command.validate().is_ok()
                {
                    if !matches!(
                        timeout(Duration::from_millis(500), input.send(command)).await,
                        Ok(Ok(()))
                    ) {
                        break;
                    }
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    match value.get("type").and_then(|kind| kind.as_str()) {
                        Some("disconnect") => return true,
                        Some("video_flow") => {
                            let strategy = match value.get("strategy") {
                                Some(value) => match serde_json::from_value::<SchedulingStrategy>(
                                    value.clone(),
                                ) {
                                    Ok(strategy) => strategy,
                                    Err(_) => {
                                        let _ = controls.send(text.to_string()).await;
                                        continue;
                                    }
                                },
                                None => SchedulingStrategy::Auto,
                            };
                            pacing.send_modify(|flow| {
                                flow.configure(
                                    value.get("enabled").and_then(|v| v.as_bool()) == Some(true),
                                    value.get("sequenced").and_then(|v| v.as_bool()) == Some(true),
                                    strategy,
                                )
                            });
                            continue;
                        }
                        Some("frame_ack") => {
                            pacing.send_modify(|flow| {
                                flow.acknowledge(
                                    value.get("id").and_then(|id| id.as_u64()),
                                    Instant::now(),
                                );
                            });
                            continue;
                        }
                        _ => {}
                    }
                }
                if controls.try_send(text.to_string()).is_err() {
                    break;
                }
            }
            Message::Ping(_) | Message::Pong(_) => {}
            _ => break,
        }
    }
    false
}

async fn read_audio(audio: &mut Option<AudioStream>, buffer: &mut [u8]) -> std::io::Result<usize> {
    match audio {
        Some(stream) => stream.read(buffer).await,
        None => std::future::pending().await,
    }
}

async fn handle_command(
    state: &AppState,
    text: &str,
    audio: &mut Option<AudioStream>,
    source: Option<&str>,
    clipboard_upload: &mut Option<ImageUpload>,
) -> anyhow::Result<Option<Message>> {
    anyhow::ensure!(text.len() <= 96 * 1024, "control message too large");
    let value: serde_json::Value = serde_json::from_str(text)?;
    let kind = value
        .get("type")
        .and_then(|kind| kind.as_str())
        .unwrap_or_default();
    match kind {
        "clipboard_paste_text" => {
            let content = value
                .get("text")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing clipboard text"))?;
            anyhow::ensure!(content.len() <= 64 * 1024, "clipboard text too large");
            state.backend.clipboard_write(content).await?;
            timeout(
                Duration::from_millis(500),
                state.backend.input().send(InputCommand::Chord {
                    codes: if value.get("shift").and_then(|value| value.as_bool()) == Some(true) {
                        vec![29, 42, 47]
                    } else {
                        vec![29, 47]
                    },
                }),
            )
            .await??;
        }
        "clipboard_image_start" | "clipboard_image_chunk" | "clipboard_image_paste" => {
            let id = value
                .get("id")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let step = value
                .get("step")
                .and_then(|value| value.as_u64())
                .unwrap_or(u64::MAX);
            let result = async {
                match kind {
                    "clipboard_image_start" => {
                        anyhow::ensure!(step == 0, "invalid clipboard start");
                        *clipboard_upload = None;
                        *clipboard_upload = Some(ImageUpload::start(
                            id,
                            value
                                .get("size")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(0),
                        )?);
                    }
                    "clipboard_image_chunk" => {
                        clipboard_upload
                            .as_mut()
                            .ok_or_else(|| anyhow::anyhow!("no clipboard image transfer"))?
                            .append(
                                id,
                                step,
                                value
                                    .get("data")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or(""),
                            )?;
                    }
                    _ => {
                        let image = clipboard_upload
                            .take()
                            .ok_or_else(|| anyhow::anyhow!("no clipboard image transfer"))?
                            .finish(id, step)?;
                        state.backend.clipboard_write_png(&image).await?;
                        timeout(
                            Duration::from_millis(500),
                            state.backend.input().send(InputCommand::Chord {
                                codes: if value.get("shift").and_then(|value| value.as_bool())
                                    == Some(true)
                                {
                                    vec![29, 42, 47]
                                } else {
                                    vec![29, 47]
                                },
                            }),
                        )
                        .await??;
                    }
                }
                anyhow::Ok(())
            }
            .await;
            let mut ack = json!({"type":"clipboard_ack", "id":id, "step":step});
            if let Err(error) = result {
                *clipboard_upload = None;
                ack["error"] = error.to_string().into();
            }
            return Ok(Some(Message::Text(ack.to_string().into())));
        }
        "ping" => {
            let id = value.get("id").and_then(|id| id.as_u64());
            anyhow::ensure!(id.is_some(), "invalid ping id");
            return Ok(Some(Message::Text(
                json!({"type":"pong","id":id}).to_string().into(),
            )));
        }
        "clipboard_get" => {
            let content = state.backend.clipboard_read().await?;
            return Ok(Some(Message::Text(
                json!({"type":"clipboard","text":content})
                    .to_string()
                    .into(),
            )));
        }
        "clipboard_set" => {
            let content = value
                .get("text")
                .and_then(|text| text.as_str())
                .unwrap_or_default();
            anyhow::ensure!(
                content.len() <= backend::MAX_CLIPBOARD_BYTES,
                "clipboard too large"
            );
            state.backend.clipboard_write(content).await?;
        }
        "resize" => {
            let width = value.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
            let height = value.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
            let resolution = DisplayMode {
                width: u32::try_from(width).map_err(|_| anyhow::anyhow!("invalid dimensions"))?,
                height: u32::try_from(height).map_err(|_| anyhow::anyhow!("invalid dimensions"))?,
                scale: match value.get("scale") {
                    None => 1.0,
                    Some(value) => value
                        .as_f64()
                        .ok_or_else(|| anyhow::anyhow!("invalid desktop scale"))?,
                },
            };
            state
                .displays
                .resize(state.output.as_deref(), resolution, state.config.fps)
                .await?;
        }
        "video_flow" => anyhow::bail!("unsupported scheduling strategy"),
        "audio" => {
            audio.take();
            if value.get("enabled").and_then(|value| value.as_bool()) == Some(true) {
                anyhow::ensure!(state.config.audio, "audio disabled");
                let source = source.ok_or_else(|| anyhow::anyhow!("audio unavailable"))?;
                let server = state.session.lock().unwrap().pulse_server.clone();
                *audio = Some(AudioStream::new(server, source.to_owned())?);
            }
        }
        _ => {
            let input: InputCommand = serde_json::from_value(value)?;
            input.validate()?;
            timeout(
                Duration::from_millis(500),
                state.backend.input().send(input),
            )
            .await??;
        }
    }
    Ok(None)
}

fn error_message(message: &str) -> Message {
    Message::Text(json!({"type":"error","message":message}).to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn physical_connection_still_advertises_the_services_virtual_desktop() {
        use clap::Parser;
        let directory = tempfile::tempdir().unwrap();
        let mut config = Config::try_parse_from(["test"]).unwrap();
        config.mcp_settings = Some(directory.path().join("mcp.json"));
        config.rdp.rdp_settings = Some(directory.path().join("rdp.json"));
        let session = Session::attached(
            directory.path().into(),
            directory.path().join("wayland-test"),
            None,
            None,
            None,
            None,
        );
        let mut state = AppState::new(
            config.clone(),
            backend::DeferredPortalBackend::new(),
            session,
        );
        state.desktop = DesktopMode::Physical;
        assert_eq!(
            state.desktop_options(Err("unavailable".into()))[0]["available"],
            true
        );
        config.wayland_socket = Some(directory.path().join("physical-wayland"));
        state.config = Arc::new(config.clone());
        assert_eq!(
            state.desktop_options(Err("unavailable".into()))[0]["available"],
            false
        );
        config.wayland_socket = None;
        config.x11_display = Some(":1".into());
        state.config = Arc::new(config);
        assert_eq!(
            state.desktop_options(Err("unavailable".into()))[0]["available"],
            false
        );
    }

    struct MissingOutputMetadata(std::sync::atomic::AtomicUsize);
    impl DisplayController for MissingOutputMetadata {
        fn outputs(&self) -> anyhow::Result<Vec<DisplayOutput>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("RandR extension unavailable")
        }
        fn capabilities(
            &self,
        ) -> futures_util::future::BoxFuture<'_, display_control::DisplayCapabilities> {
            Box::pin(async {
                display_control::DisplayCapabilities {
                    resize: false,
                    resize_scale: false,
                    reason: Some("test display".into()),
                }
            })
        }
        fn resize<'a>(
            &'a self,
            _: Option<&'a str>,
            _: DisplayMode,
            _: u32,
        ) -> futures_util::future::BoxFuture<'a, anyhow::Result<DisplayMode>> {
            Box::pin(async { anyhow::bail!("read-only test display") })
        }
    }

    #[test]
    fn x11_root_capture_survives_missing_randr_but_named_outputs_fail() {
        let displays = MissingOutputMetadata(std::sync::atomic::AtomicUsize::new(0));
        let (outputs, selected) =
            physical_output_selection(BackendKind::X11, None, &displays).unwrap();
        assert!(outputs.is_empty());
        assert!(selected.is_none());
        assert!(physical_output_selection(BackendKind::X11, Some("HDMI-1"), &displays).is_err());
        assert!(physical_output_selection(BackendKind::Wayland, None, &displays).is_err());
    }

    #[test]
    fn portal_chooser_does_not_depend_on_separate_output_metadata() {
        let displays = MissingOutputMetadata(std::sync::atomic::AtomicUsize::new(0));
        let (outputs, selected) =
            physical_output_selection(BackendKind::Portal, None, &displays).unwrap();
        assert!(outputs.is_empty());
        assert!(selected.is_none());
        assert!(physical_output_selection(BackendKind::Portal, Some("eDP-1"), &displays).is_err());
        assert_eq!(displays.0.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn deferred_portal_transport_stays_ready_after_controller_release() {
        use clap::Parser;
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket_path = directory.path().join("wayland-test");
        let _socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let mut config =
            Config::try_parse_from(["test", "--password", "test-pending-portal-secret"]).unwrap();
        config.wayland_socket = Some(socket_path);
        config.mcp_settings = Some(directory.path().join("mcp.json"));
        config.rdp.rdp_settings = Some(directory.path().join("rdp.json"));
        let session = Session::start(&config).unwrap();
        let backend = Arc::new(crate::backend::DeferredPortalBackend::new());
        let state = AppState::with_backend(config, backend.clone(), session);
        assert_eq!(state.desktop, DesktopMode::Physical);
        assert!(state.backend.frames().borrow().is_none());
        let response = health(State(state.clone())).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["requires_consent"], true);
        assert_eq!(body["ready"], true);

        drop(ControllerLease {
            state: state.clone(),
        });
        assert!(
            !backend.is_stopped(),
            "failed consent must leave the base transport available"
        );
        assert_eq!(
            health(State(state)).await.into_response().status(),
            StatusCode::OK
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lock_and_unlock_during_encode_never_publish_old_pixels() {
        use clap::Parser;
        use futures_util::StreamExt;
        use tokio_tungstenite::{
            connect_async,
            tungstenite::{Message as ClientMessage, client::IntoClientRequest},
        };

        struct Lockable {
            frames: watch::Receiver<Option<Arc<crate::backend::Frame>>>,
            input: mpsc::Sender<InputCommand>,
            locked: Arc<AtomicBool>,
            epoch: Arc<AtomicU64>,
        }
        impl DesktopBackend for Lockable {
            fn kind(&self) -> BackendKind {
                BackendKind::X11
            }
            fn frames(&self) -> watch::Receiver<Option<Arc<crate::backend::Frame>>> {
                self.frames.clone()
            }
            fn input(&self) -> mpsc::Sender<InputCommand> {
                self.input.clone()
            }
            fn stop(&self) {}
            fn paused(&self) -> bool {
                self.locked.load(Ordering::Acquire)
            }
            fn privacy_epoch(&self) -> u64 {
                self.epoch.load(Ordering::Acquire)
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let socket_path = directory.path().join("wayland-test");
        let _socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let session = Session::attached(
            directory.path().to_owned(),
            socket_path,
            None,
            None,
            None,
            None,
        );
        let mut config =
            Config::try_parse_from(["test", "--password", "test-privacy-secret"]).unwrap();
        config.mcp_settings = Some(directory.path().join("mcp.json"));
        config.rdp.rdp_settings = Some(directory.path().join("rdp.json"));
        config.physical_wayland_socket = Some(directory.path().join("no-physical-desktop"));
        let pixels = [230, 10, 10, 255].repeat(16 * 16);
        let (capture, frames) = watch::channel(Some(Arc::new(crate::backend::Frame::new(
            16,
            16,
            pixels.into(),
        ))));
        let (input, mut commands) = mpsc::channel(128);
        let input_worker = tokio::spawn(async move { while commands.recv().await.is_some() {} });
        let locked = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(0));
        let mut state = AppState::new(
            config,
            Lockable {
                frames,
                input,
                locked: locked.clone(),
                epoch: epoch.clone(),
            },
            session,
        );
        state.displays = Arc::new(MissingOutputMetadata(std::sync::atomic::AtomicUsize::new(
            0,
        )));
        state.codec_capabilities.set(Vec::new()).unwrap();
        let (resume_encode, release) = std::sync::mpsc::sync_channel(1);
        let pause = Arc::new(EncodePause {
            entered: tokio::sync::Notify::new(),
            release: Mutex::new(Some(release)),
        });
        state.encode_pause = Some(pause.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async { axum::serve(listener, router(state)).await.unwrap() });
        let mut request = format!("ws://{address}/ws").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", format!("http://{address}").parse().unwrap());
        request.headers_mut().insert(
            "Authorization",
            format!("Basic {}", STANDARD.encode("lightos:test-privacy-secret"))
                .parse()
                .unwrap(),
        );
        let (mut client, _) = connect_async(request).await.unwrap();
        timeout(Duration::from_secs(3), pause.entered.notified())
            .await
            .unwrap();

        locked.store(true, Ordering::Release);
        epoch.fetch_add(1, Ordering::AcqRel);
        capture.send_replace(None);
        locked.store(false, Ordering::Release);
        epoch.fetch_add(1, Ordering::AcqRel);
        let pixels = [10, 10, 230, 255].repeat(16 * 16);
        capture.send_replace(Some(Arc::new(crate::backend::Frame::new(
            16,
            16,
            pixels.into(),
        ))));
        resume_encode.send(()).unwrap();

        loop {
            let event = timeout(Duration::from_secs(3), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let ClientMessage::Binary(bytes) = event
                && bytes.first() == Some(&1)
            {
                let image = image::load_from_memory(&bytes[1..]).unwrap().to_rgb8();
                let pixel = image.get_pixel(8, 8);
                assert!(
                    pixel[2] > pixel[0],
                    "pre-lock pixels were published after unlock: {pixel:?}"
                );
                break;
            }
        }
        server.abort();
        input_worker.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn paused_backend_retains_websocket_and_resumes_after_unlock() {
        use clap::Parser;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{
            connect_async,
            tungstenite::{Message as ClientMessage, client::IntoClientRequest},
        };
        struct Pausable {
            frames: watch::Receiver<Option<Arc<crate::backend::Frame>>>,
            input: mpsc::Sender<InputCommand>,
            paused: Arc<AtomicBool>,
        }
        impl DesktopBackend for Pausable {
            fn kind(&self) -> BackendKind {
                BackendKind::X11
            }
            fn frames(&self) -> watch::Receiver<Option<Arc<crate::backend::Frame>>> {
                self.frames.clone()
            }
            fn input(&self) -> mpsc::Sender<InputCommand> {
                self.input.clone()
            }
            fn paused(&self) -> bool {
                self.paused.load(Ordering::Acquire)
            }
            fn stop(&self) {}
        }
        struct TestDisplays;
        impl DisplayController for TestDisplays {
            fn outputs(&self) -> anyhow::Result<Vec<DisplayOutput>> {
                Ok(Vec::new())
            }
            fn capabilities(
                &self,
            ) -> futures_util::future::BoxFuture<'_, display_control::DisplayCapabilities>
            {
                Box::pin(async {
                    display_control::DisplayCapabilities {
                        resize: false,
                        resize_scale: false,
                        reason: Some("test read-only display".into()),
                    }
                })
            }
            fn resize<'a>(
                &'a self,
                _: Option<&'a str>,
                _: DisplayMode,
                _: u32,
            ) -> futures_util::future::BoxFuture<'a, anyhow::Result<DisplayMode>> {
                Box::pin(async { anyhow::bail!("test read-only display") })
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let socket_path = directory.path().join("wayland-test");
        let _socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let session = Session::attached(
            directory.path().to_owned(),
            socket_path,
            None,
            None,
            None,
            None,
        );
        let mut config =
            Config::try_parse_from(["test", "--password", "test-pause-secret"]).unwrap();
        config.mcp_settings = Some(directory.path().join("mcp.json"));
        config.rdp.rdp_settings = Some(directory.path().join("rdp.json"));
        config.physical_wayland_socket = Some(directory.path().join("no-physical-desktop"));
        let (capture, frames) =
            watch::channel(Some(Arc::new(crate::backend::Frame::test_frame(8, 8))));
        let (input, mut commands) = mpsc::channel(128);
        let input_worker = tokio::spawn(async move {
            while let Some(command) = commands.recv().await {
                if let InputCommand::ReleaseSync { done } = command {
                    let _ = done.send(());
                }
            }
        });
        let paused = Arc::new(AtomicBool::new(false));
        let mut state = AppState::new(
            config,
            Pausable {
                frames,
                input,
                paused: paused.clone(),
            },
            session,
        );
        state.displays = Arc::new(TestDisplays);
        state.codec_capabilities.set(Vec::new()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async { axum::serve(listener, router(state)).await.unwrap() });
        let mut request = format!("ws://{address}/ws").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", format!("http://{address}").parse().unwrap());
        request.headers_mut().insert(
            "Authorization",
            format!("Basic {}", STANDARD.encode("lightos:test-pause-secret"))
                .parse()
                .unwrap(),
        );
        let (mut client, _) = connect_async(request).await.unwrap();
        async fn next_event<S>(client: &mut S) -> ClientMessage
        where
            S: futures_util::Stream<
                    Item = Result<ClientMessage, tokio_tungstenite::tungstenite::Error>,
                > + Unpin,
        {
            timeout(Duration::from_secs(3), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        }
        let mut saw_ready = false;
        loop {
            match next_event(&mut client).await {
                ClientMessage::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "ready" {
                        assert_eq!(
                            value["cursor"], "default",
                            "X11 capture excludes the pointer"
                        );
                        saw_ready = true;
                    }
                }
                ClientMessage::Binary(_) => {
                    assert!(saw_ready);
                    break;
                }
                _ => {}
            }
        }
        paused.store(true, Ordering::Release);
        capture.send_replace(None);
        loop {
            if let ClientMessage::Text(text) = next_event(&mut client).await {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "desktop_state" {
                    assert_eq!(value["available"], false);
                    break;
                }
            }
        }
        // Policy changes while locked must not refresh a cached raw frame.
        client.send(ClientMessage::Text(r#"{"type":"video_flow","enabled":true,"sequenced":true,"strategy":"quality_first"}"#.into())).await.unwrap();
        assert!(
            timeout(Duration::from_millis(350), client.next())
                .await
                .is_err()
        );
        paused.store(false, Ordering::Release);
        capture.send_replace(Some(Arc::new(crate::backend::Frame::test_frame(8, 8))));
        let mut resumed = false;
        loop {
            match next_event(&mut client).await {
                ClientMessage::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "desktop_state" {
                        assert_eq!(value["available"], true);
                        resumed = true;
                    }
                }
                ClientMessage::Binary(_) => {
                    assert!(resumed);
                    break;
                }
                ClientMessage::Close(_) => panic!("lock closed the desktop connection"),
                _ => {}
            }
        }
        client.close(None).await.unwrap();
        server.abort();
        input_worker.abort();
    }

    #[tokio::test]
    async fn static_frames_refresh_after_policy_change_or_native_skip_with_pacing() {
        let (captures, mut frames) = watch::channel(1_u32);
        let mut flow = FrameScheduler::new(30).unwrap();
        flow.configure(true, true, SchedulingStrategy::Auto);
        flow.frame_started(Instant::now());
        assert!(
            timeout(
                Duration::from_millis(15),
                wait_for_video_frame(&mut frames, flow.refresh_requested(), None,)
            )
            .await
            .is_err(),
            "a static desktop must stay idle"
        );
        flow.configure(true, true, SchedulingStrategy::QualityFirst);
        timeout(
            Duration::from_millis(100),
            wait_for_video_frame(&mut frames, flow.refresh_requested(), flow.next_frame_at()),
        )
        .await
        .expect("strategy must refresh without a new capture")
        .unwrap();
        flow.begin_frame((800, 600));
        let started = Instant::now();
        flow.frame_started(started);
        flow.frame_skipped();
        let deadline = flow.next_frame_at().unwrap();
        assert!(
            timeout(
                Duration::from_millis(15),
                wait_for_video_frame(&mut frames, flow.refresh_requested(), Some(deadline),)
            )
            .await
            .is_err(),
            "native skip retries must honor pacing"
        );
        // A newer raw capture arriving during pacing must replace the old one.
        captures.send_replace(2);
        wait_for_video_frame(&mut frames, flow.refresh_requested(), Some(deadline))
            .await
            .unwrap();
        assert!(Instant::now() >= deadline);
        assert_eq!(*frames.borrow_and_update(), 2);
        flow.frame_started(Instant::now());
        assert!(
            timeout(
                Duration::from_millis(15),
                wait_for_video_frame(&mut frames, flow.refresh_requested(), None,)
            )
            .await
            .is_err(),
            "one refresh must not repeatedly encode a static image"
        );
    }

    #[tokio::test]
    async fn input_is_delivered_while_control_writer_is_not_draining() {
        let (input, mut received) = mpsc::channel(4);
        let (controls, _blocked_writer) = mpsc::channel(1);
        let (pacing, _) = watch::channel(FrameScheduler::default());
        let messages = [
            "{\"type\":\"clipboard_get\"}",
            "{\"type\":\"move\",\"x\":0.5,\"y\":0.25}",
            "{\"type\":\"button\",\"button\":0,\"pressed\":true}",
            "{\"type\":\"button\",\"button\":0,\"pressed\":false}",
        ];
        let reader = futures_util::stream::iter(
            messages
                .into_iter()
                .map(|message| Ok(Message::Text(message.into()))),
        );
        receive_commands(reader, input, controls, pacing).await;
        assert!(matches!(
            received.recv().await,
            Some(InputCommand::Move { .. })
        ));
        assert!(matches!(
            received.recv().await,
            Some(InputCommand::Button { pressed: true, .. })
        ));
        assert!(matches!(
            received.recv().await,
            Some(InputCommand::Button { pressed: false, .. })
        ));
    }

    #[tokio::test]
    async fn frame_acknowledgement_releases_only_the_video_gate() {
        let (input, _) = mpsc::channel(1);
        let (controls, _responses) = mpsc::channel(1);
        let mut flow = FrameScheduler::default();
        flow.configure(true, false, SchedulingStrategy::Auto);
        flow.sent(1, Instant::now());
        let (pacing, state) = watch::channel(flow);
        assert!(state.borrow().waiting());
        let reader =
            futures_util::stream::iter([Ok(Message::Text("{\"type\":\"frame_ack\"}".into()))]);
        receive_commands(reader, input, controls, pacing).await;
        assert!(!state.borrow().waiting());
        assert!(state.borrow().enabled());
    }

    #[tokio::test]
    async fn stale_jpeg_ack_cannot_release_a_new_h264_keyframe() {
        let (input, _) = mpsc::channel(1);
        let (controls, _) = mpsc::channel(1);
        let mut flow = FrameScheduler::default();
        flow.configure(true, true, SchedulingStrategy::Auto);
        flow.sent(2, Instant::now());
        let (pacing, state) = watch::channel(flow);
        let reader = futures_util::stream::iter([Ok(Message::Text(
            "{\"type\":\"frame_ack\",\"id\":1}".into(),
        ))]);
        receive_commands(reader, input.clone(), controls.clone(), pacing.clone()).await;
        assert!(state.borrow().waiting());
        let reader = futures_util::stream::iter([Ok(Message::Text(
            "{\"type\":\"frame_ack\",\"id\":2}".into(),
        ))]);
        receive_commands(reader, input, controls, pacing).await;
        assert!(!state.borrow().waiting());
    }

    #[tokio::test]
    async fn prefixed_index_redirects_without_overlapping_routes() {
        let router = mount_desktop(Router::<()>::new(), "/native/test");
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/native/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], "/native/test/");
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/native/test/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = axum::body::to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))));
        assert!(!html.contains("__LIGHTOS_VERSION__"));
    }

    #[tokio::test]
    async fn embedded_logo_is_allowed_by_production_content_security_policy() {
        let desktop =
            mount_desktop(Router::<()>::new(), "/").layer(middleware::from_fn(security_headers));
        let response = desktop
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let policy = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        let sources = |directive: &str| {
            policy
                .split(';')
                .find_map(|entry| {
                    let mut words = entry.split_whitespace();
                    (words.next() == Some(directive)).then(|| words.collect::<Vec<_>>())
                })
                .unwrap()
        };
        // Vite's ?inline import embeds the About logo in app.js as a data URI.
        assert!(include_str!("../web/dist/app.js").contains("data:image/webp;base64,"));
        assert!(sources("img-src").contains(&"data:"));
        assert_eq!(sources("script-src"), vec!["'self'"]);
        assert_eq!(sources("default-src"), vec!["'none'"]);
        let html = axum::body::to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&html).unwrap().contains("./app.js"));
    }

    #[test]
    fn quality_first_rate_keeps_per_frame_budget_when_fps_falls() {
        let full = encoder_rate(
            1280,
            720,
            30,
            stream_scheduler::RateTarget {
                fps: 30,
                quality_percent: 100,
            },
        );
        let slow = encoder_rate(
            1280,
            720,
            30,
            stream_scheduler::RateTarget {
                fps: 10,
                quality_percent: 100,
            },
        );
        let reduced = encoder_rate(
            1280,
            720,
            30,
            stream_scheduler::RateTarget {
                fps: 30,
                quality_percent: 50,
            },
        );
        assert_eq!(full.bitrate_bps / 30, slow.bitrate_bps / 10);
        assert_eq!(full.bitrate_bps / 2, reduced.bitrate_bps);
        assert_eq!(jpeg_quality(75, 100), 75);
        assert_eq!(jpeg_quality(75, 20), 35);
        assert_eq!(jpeg_quality(10, 20), 10);
    }

    #[test]
    fn credentials_are_exact_and_allow_colons_in_passwords() {
        let mut headers = HeaderMap::new();
        assert!(!credentials_match(&headers, "user", "secret:one"));
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {}", STANDARD.encode("user:secret:one"))
                .parse()
                .unwrap(),
        );
        assert!(credentials_match(&headers, "user", "secret:one"));
        assert!(!credentials_match(&headers, "other", "secret:one"));
        assert!(!credentials_match(&headers, "user", "wrong"));
    }

    #[test]
    fn gzip_negotiation_respects_quality_and_raw_clients() {
        let mut headers = HeaderMap::new();
        assert!(!accepts_gzip(&headers));
        for value in ["gzip", "br, gzip;q=0.5", "GZIP; Q=1"] {
            headers.insert(header::ACCEPT_ENCODING, value.parse().unwrap());
            assert!(accepts_gzip(&headers), "{value}");
        }
        for value in [
            "br",
            "gzip;q=0",
            "gzip;Q=0",
            "gzip;q=invalid",
            "gzip;q=2",
            "gzip;q=NaN",
        ] {
            headers.insert(header::ACCEPT_ENCODING, value.parse().unwrap());
            assert!(!accepts_gzip(&headers), "{value}");
        }
    }

    #[tokio::test]
    async fn embedded_asset_selects_encoding_without_cache_mixups() {
        let mut headers = HeaderMap::new();
        let raw = asset(
            &headers,
            "raw",
            b"compressed",
            "text/javascript; charset=utf-8",
        );
        assert!(!raw.headers().contains_key(header::CONTENT_ENCODING));
        assert_eq!(raw.headers()[header::VARY], "Accept-Encoding");
        assert_eq!(raw.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(
            axum::body::to_bytes(raw.into_body(), 128)
                .await
                .unwrap()
                .as_ref(),
            b"raw"
        );
        headers.insert(header::ACCEPT_ENCODING, "gzip".parse().unwrap());
        let compressed = asset(
            &headers,
            "raw",
            b"compressed",
            "text/javascript; charset=utf-8",
        );
        assert_eq!(compressed.headers()[header::CONTENT_ENCODING], "gzip");
        assert_eq!(
            axum::body::to_bytes(compressed.into_body(), 128)
                .await
                .unwrap()
                .as_ref(),
            b"compressed"
        );
    }

    #[test]
    fn websocket_rejects_foreign_or_missing_origins() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "desktop.example".parse().unwrap());
        assert!(!origin_allowed(&headers));
        headers.insert(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(!origin_allowed(&headers));
        headers.insert(header::ORIGIN, "https://desktop.example".parse().unwrap());
        assert!(origin_allowed(&headers));
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!origin_allowed(&headers));
    }
}
