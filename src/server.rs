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
    DisplayController, DisplayMode, DisplayOutput, SessionTarget, WaylandDisplay,
};
use futures_util::{SinkExt, Stream, StreamExt, stream::SplitSink};
use media_codec::{
    CodecCapability, EncoderFactory, EncoderRate, EncoderSettings, SoftwareEncoders, VideoCodec,
    VideoEncoder,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{mpsc, watch},
    time::timeout,
};

use crate::{
    audio::{AudioFormat, AudioStream},
    audio_channel::{self, AudioHub},
    config::Config,
    session::Session,
    wayland::{Backend, InputCommand},
};

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
    pub backend: Arc<Backend>,
    pub session: Arc<Mutex<Session>>,
    controller: Arc<AtomicBool>,
    audio_hub: Arc<AudioHub>,
    desktop: DesktopMode,
    output: Option<String>,
    outputs: Vec<DisplayOutput>,
    transient_backend: bool,
    displays: Arc<dyn DisplayController>,
    codec_capabilities: Arc<tokio::sync::OnceCell<Vec<CodecCapability>>>,
}

fn session_displays(config: &Config, session: &Session, owned: bool) -> Arc<dyn DisplayController> {
    let target = SessionTarget::wayland(session.socket.clone(), session.runtime.clone());
    match config.display_backend.controller(
        target.clone(),
        target.matching_environment_hint(),
        session.signature.as_deref(),
        owned,
    ) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::warn!(%error, "display backend unavailable");
            Arc::new(WaylandDisplay::new(session.socket.clone()))
        }
    }
}

impl AppState {
    pub fn new(config: Config, backend: Backend, session: Session) -> Self {
        let desktop = if config.wayland_socket.is_some() {
            DesktopMode::Physical
        } else {
            DesktopMode::Virtual
        };
        let displays = session_displays(&config, &session, desktop == DesktopMode::Virtual);
        Self {
            codec_capabilities: Arc::new(tokio::sync::OnceCell::new()),
            displays,
            config: Arc::new(config),
            backend: Arc::new(backend),
            session: Arc::new(Mutex::new(session)),
            controller: Arc::new(AtomicBool::new(false)),
            audio_hub: Arc::new(AudioHub::default()),
            desktop,
            output: None,
            outputs: Vec::new(),
            transient_backend: false,
        }
    }

    fn physical_socket(&self) -> anyhow::Result<std::path::PathBuf> {
        if let Some(socket) = &self.config.wayland_socket {
            return Ok(socket.clone());
        }
        Session::physical_socket(&self.config)
    }

    fn desktop_options(&self) -> serde_json::Value {
        let physical = self.physical_socket();
        json!([
            {"mode":"virtual","label":"虚拟桌面","available":self.config.wayland_socket.is_none(),"reason":if self.config.wayland_socket.is_none() { "" } else { "服务以外部桌面模式启动" }},
            {"mode":"physical","label":"物理桌面","available":physical.is_ok(),"reason":physical.err().map(|error| error.to_string())}
        ])
    }

    fn select_desktop(
        self,
        request: DesktopRequest,
        cancelled: Arc<AtomicBool>,
        connection_id: u64,
    ) -> anyhow::Result<Self> {
        use anyhow::Context;

        let started = Instant::now();
        let mode = request.desktop.unwrap_or(self.desktop);
        if mode == self.desktop && request.output.is_none() {
            return Ok(self);
        }
        anyhow::ensure!(mode == DesktopMode::Physical, "虚拟桌面不可用");
        let socket = self.physical_socket()?;
        let target = SessionTarget::wayland(
            socket.clone(),
            socket.parent().context("missing session runtime")?.into(),
        );
        let displays = self.config.display_backend.controller(
            target.clone(),
            target.matching_environment_hint(),
            None,
            false,
        )?;
        let outputs = displays.outputs().context("物理显示器枚举失败")?;
        tracing::info!(
            connection_id,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "physical outputs enumerated"
        );
        anyhow::ensure!(!cancelled.load(Ordering::Acquire), "桌面连接已取消");
        let selected = match request.output {
            Some(name) => outputs.iter().find(|output| output.name == name),
            None => outputs.first(),
        }
        .ok_or_else(|| anyhow::anyhow!("所选显示器不可用，请刷新显示器列表"))?
        .name
        .clone();
        let mut config = (*self.config).clone();
        config.session_runtime = None;
        config.wayland_socket = Some(socket.clone());
        let session = Session::start(&config)?;
        let backend = Backend::start_with_output_cancellable(
            socket,
            config.fps,
            config.quality,
            Some(selected.clone()),
            cancelled.clone(),
        )
        .context("物理桌面采集或输入初始化失败")?;
        anyhow::ensure!(!cancelled.load(Ordering::Acquire), "桌面连接已取消");
        tracing::info!(
            connection_id,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "physical capture and input ready"
        );
        Ok(Self {
            codec_capabilities: self.codec_capabilities,
            displays,
            config: self.config,
            controller: self.controller,
            audio_hub: self.audio_hub,
            backend: Arc::new(backend),
            session: Arc::new(Mutex::new(session)),
            desktop: mode,
            output: Some(selected),
            outputs,
            transient_backend: true,
        })
    }
}

pub fn router(state: AppState) -> Router {
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
        .route("/desktops", get(desktops));
    mount_desktop(desktop, &state.config.subfolder)
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate))
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
            "default-src 'none'; script-src 'self'; worker-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' blob:; frame-ancestors 'self'; base-uri 'none'; form-action 'none'",
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
    let ready = state.backend.frame_is_fresh(Duration::from_secs(5))
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
            json!({"native":true,"ready":ready,"pid":std::process::id(),"transport":"websocket","video_codecs":state.codec_capabilities.get().map(|caps| caps.iter().filter(|c| c.available).map(|c| c.codec).collect::<Vec<_>>()).unwrap_or_default(),"codec_probe_complete":state.codec_capabilities.initialized()}),
        ),
    )
}

async fn desktops(State(state): State<AppState>) -> impl IntoResponse {
    let options = state.desktop_options();
    let current = state.desktop;
    let result = tokio::task::spawn_blocking(move || {
        let socket = state.physical_socket()?;
        let target = SessionTarget::wayland(
            socket.clone(),
            socket
                .parent()
                .ok_or_else(|| anyhow::anyhow!("missing session runtime"))?
                .into(),
        );
        state
            .config
            .display_backend
            .controller(
                target.clone(),
                target.matching_environment_hint(),
                None,
                false,
            )?
            .outputs()
    })
    .await;
    let (outputs, reason) = match result {
        Ok(Ok(outputs)) => (outputs, None),
        Ok(Err(error)) => (Vec::new(), Some(error.to_string())),
        Err(_) => (Vec::new(), Some("显示器检测失败".to_owned())),
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

struct ControllerLease {
    state: AppState,
}

impl ControllerLease {
    async fn release(self) {
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
            let mut initialize = tokio::task::spawn_blocking(move || {
                state.select_desktop(request, initialize_cancelled, connection_id)
            });
            let result = tokio::select! {
                biased;
                result = &mut initialize => Some(result),
                _ = startup_disconnect(&mut socket) => None,
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
                    let _ =
                        timeout(Duration::from_secs(3), socket.send(error_message(&error))).await;
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
    let receive = receive_commands(
        reader,
        state.backend.input.clone(),
        control_tx,
        pacing_tx.clone(),
    );
    let mut frames = state.backend.frames.clone();
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
    let codec_capabilities = state
        .codec_capabilities
        .get_or_init(|| SoftwareEncoders.probe())
        .await;
    let video_codecs: Vec<_> = codec_capabilities
        .iter()
        .filter(|c| c.available)
        .map(|c| c.codec)
        .collect();
    let ready = Message::Text(
        json!({"type":"ready","control_release":true,"resize_scale":display_capabilities.resize_scale,"version":env!("CARGO_PKG_VERSION"),"width":width,"height":height,"cursor":"embedded","audio":audio_available,"audio_session":audio_session,"audio_codecs":["opus","pcm"],"frame_ack":true,"frame_ids":true,"metrics":true,"video_codecs":video_codecs,"codec_capabilities":codec_capabilities,"desktop":state.desktop,"desktops":state.desktop_options(),"resize":display_capabilities.resize,"resize_reason":display_capabilities.reason,"outputs":state.outputs,"output":state.output})

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
    let h264_dynamic_rate = codec_capabilities
        .iter()
        .any(|c| c.codec == VideoCodec::H264 && c.available && c.dynamic_bitrate);
    let mut audio = None;
    let mut audio_buffer = vec![0_u8; 3840];
    let mut first_frame = true;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut qos_ticks = tokio::time::interval(Duration::from_millis(250));
    let mut applied_rate = None;
    let mut applied_jpeg_quality = None;
    let mut h264 = false;
    let mut encoder: Option<Box<dyn VideoEncoder>> = None;
    let mut codec = None;
    let video_started = std::time::Instant::now();
    let mut frame_id = 0_u64;
    let mut first_video_sent = false;
    let mut encode_samples = LatencySamples::default();
    let mut send_samples = LatencySamples::default();
    let transmit = async {
        loop {
            let frame_deadline = pacing.borrow().next_frame_at();
            let outgoing = tokio::select! {
                changed = async {
                    if let Some(deadline) = frame_deadline { tokio::time::sleep_until(deadline.into()).await; }
                    if !first_frame { frames.changed().await } else { Ok(()) }
                }, if !pacing.borrow().waiting() => {
                    first_frame = false;
                    if changed.is_err() { break; }
                    let frame = frames.borrow_and_update().clone();
                    let Some(frame) = frame else { break; };
                    pacing_tx.send_modify(|flow| flow.begin_frame((frame.width, frame.height)));
                    let target = pacing.borrow().target();
                    let encode_started = Instant::now();
                    pacing_tx.send_modify(|flow| flow.frame_started(encode_started));
                    let mut packet = None;
                    if h264 {
                        let result = async {
                            if encoder.as_ref().is_none_or(|encoder| encoder.dimensions() != (frame.width, frame.height)) {
                                encoder = Some(SoftwareEncoders.create(VideoCodec::H264, EncoderSettings {width:frame.width,height:frame.height,fps:state.config.fps,quality:state.config.quality})?);
                                codec = None;
                            }
                            let encoder = encoder.as_mut().ok_or_else(|| anyhow::anyhow!("encoder missing"))?;
                            let rate = encoder_rate(frame.width, frame.height, state.config.fps, target);
                            if h264_dynamic_rate { encoder.set_rate(rate)?; }
                            let encoded = encoder.encode(frame.pixels.clone()).await?;
                            applied_rate = h264_dynamic_rate.then_some(rate);
                            applied_jpeg_quality = None;
                            Ok::<_, anyhow::Error>(encoded)
                        }.await;
                        match result {
                            Ok(encoded) => {
                                if encoded.data.is_empty() {
                                    // OpenH264 skipped this raw input to honor its
                                    // bitrate. No predictive frame was produced.
                                    encode_samples.record(encode_started.elapsed());
                                    continue;
                                }
                                if let Some(configuration) = encoded.codec
                                    && codec.as_ref() != Some(&configuration) {
                                        let message = Message::Text(json!({"type":"video_config","codec":configuration,"width":frame.width,"height":frame.height}).to_string().into());
                                        if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await, Ok(Ok(()))) { break; }
                                        codec = Some(configuration);
                                }
                                let mut bytes = Vec::with_capacity(encoded.data.len() + 10);
                                bytes.extend_from_slice(&[3, u8::from(encoded.key)]);
                                bytes.extend_from_slice(&(video_started.elapsed().as_micros() as u64).to_le_bytes());
                                bytes.extend_from_slice(&encoded.data);
                                packet = Some(bytes);
                            }
                            Err(error) => {
                                tracing::warn!(%error, "H.264 unavailable; falling back to JPEG");
                                pacing_tx.send_modify(FrameScheduler::reset);
                                encoder.take(); h264 = false; codec = None;
                                applied_rate = None;
                                pacing_tx.send_modify(|flow| flow.set_quality_scalable(state.config.quality > 20));
                                let message = Message::Text(json!({"type":"video_config","codec":"jpeg"}).to_string().into());
                                if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await, Ok(Ok(()))) { break; }
                            }
                        }
                    }
                    let packet = match packet {
                        Some(packet) => packet,
                        None => {
                            let quality = jpeg_quality(state.config.quality, pacing.borrow().target().quality_percent);
                            applied_jpeg_quality = Some(quality);
                            applied_rate = None;
                            let jpeg = match tokio::task::spawn_blocking(move || frame.jpeg(quality)).await {
                                Ok(Ok(jpeg)) => jpeg,
                                result => {
                                    tracing::warn!(?result, "JPEG encoding failed");
                                    break;
                                }
                            };
                            let mut packet = Vec::with_capacity(jpeg.len() + 1);
                            packet.push(1); packet.extend_from_slice(&jpeg); packet
                        }
                    };
                    frame_id += 1;
                    pacing_tx.send_modify(|flow| flow.sent(frame_id, Instant::now()));
                    if pacing.borrow().sequenced() {
                        let metadata = Message::Text(json!({"type":"frame_id","id":frame_id}).to_string().into());
                        if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(metadata)).await, Ok(Ok(()))) { break; }
                    }
                    encode_samples.record(encode_started.elapsed());
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
                                    "qos":flow.qos_summary(),"encoder_rate":applied_rate,"jpeg_quality":applied_jpeg_quality
                                }}).to_string().into());
                                drop(flow);
                                Some(message)
                            } else {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                                && value.get("type").and_then(|kind| kind.as_str()) == Some("video")
                            {
                                h264 = value.get("codec").and_then(|codec| codec.as_str()) == Some("h264") && pacing.borrow().sequenced() && video_codecs.contains(&VideoCodec::H264);
                                encoder.take(); codec = None; first_frame = true;
                                pacing_tx.send_modify(|flow| {
                                    flow.reset();
                                    flow.set_quality_scalable(if h264 { h264_dynamic_rate } else { state.config.quality > 20 });
                                });
                                applied_rate = None; applied_jpeg_quality = None;
                                if !h264 {
                                    let message = Message::Text(json!({"type":"video_config","codec":"jpeg"}).to_string().into());
                                    if !matches!(timeout(STREAM_WRITE_TIMEOUT, writer.send(message)).await, Ok(Ok(()))) { break; }
                                }
                                continue;
                            }
                            match handle_command(&state, &text, &mut audio, audio_source.as_deref()).await {
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
        state.backend.input.send(InputCommand::Release),
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
fn jpeg_quality(configured: u8, percent: u32) -> u8 {
    configured
        .saturating_sub(((100 - percent.min(100)) / 2) as u8)
        .max(configured.min(20))
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
) -> anyhow::Result<Option<Message>> {
    anyhow::ensure!(text.len() <= 96 * 1024, "control message too large");
    let value: serde_json::Value = serde_json::from_str(text)?;
    let kind = value
        .get("type")
        .and_then(|kind| kind.as_str())
        .unwrap_or_default();
    match kind {
        "ping" => {
            let id = value.get("id").and_then(|id| id.as_u64());
            anyhow::ensure!(id.is_some(), "invalid ping id");
            return Ok(Some(Message::Text(
                json!({"type":"pong","id":id}).to_string().into(),
            )));
        }
        "clipboard_get" => {
            let mut command = helper(state, "wl-paste");
            command.args(["--no-newline", "--type", "text"]);
            command.stdout(std::process::Stdio::piped());
            let mut child = command.spawn()?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("clipboard pipe unavailable"))?;
            let mut output = Vec::new();
            timeout(
                Duration::from_secs(2),
                stdout.take(64 * 1024 + 1).read_to_end(&mut output),
            )
            .await??;
            anyhow::ensure!(output.len() <= 64 * 1024, "clipboard too large");
            anyhow::ensure!(
                timeout(Duration::from_secs(2), child.wait())
                    .await??
                    .success(),
                "clipboard unavailable"
            );
            return Ok(Some(Message::Text(
                json!({"type":"clipboard","text":String::from_utf8_lossy(&output)})
                    .to_string()
                    .into(),
            )));
        }
        "clipboard_set" => {
            let content = value
                .get("text")
                .and_then(|text| text.as_str())
                .unwrap_or_default();
            anyhow::ensure!(content.len() <= 64 * 1024, "clipboard too large");
            let mut command = helper(state, "wl-copy");
            command.args(["--type", "text/plain;charset=utf-8", "--", content]);
            let status = timeout(Duration::from_secs(2), command.status()).await??;
            anyhow::ensure!(status.success(), "clipboard write failed");
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
            timeout(Duration::from_millis(500), state.backend.input.send(input)).await??;
        }
    }
    Ok(None)
}

fn helper(state: &AppState, name: &str) -> Command {
    let session = state.session.lock().unwrap();
    let mut command = Command::new(name);
    command
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .env("XDG_RUNTIME_DIR", &session.runtime)
        .env("WAYLAND_DISPLAY", &session.socket)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(signature) = &session.signature {
        command.env("HYPRLAND_INSTANCE_SIGNATURE", signature);
    }
    session.audio_environment(command.as_std_mut());
    command
}

fn error_message(message: &str) -> Message {
    Message::Text(json!({"type":"error","message":message}).to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

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
