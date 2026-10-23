//! Explicit localhost-only browser experiment. Not compiled into the daemon.
use super::*;
use crate::{backend::Frame, wire::WireMode};
use anyhow::{Context, Result};
use axum::extract::ws::WebSocketUpgrade;
use clap::Parser;
use std::{path::PathBuf, sync::atomic::AtomicU64};

#[derive(Clone)]
struct Replay {
    directory: Arc<PathBuf>,
}
#[derive(serde::Deserialize)]
struct ReplayRequest {
    codec: String,
    #[serde(default = "default_fps")]
    fps: u32,
}
fn default_fps() -> u32 {
    30
}
fn cpu_ms() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // getrusage initializes this buffer on success; it reads only this process.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return 0.0;
    }
    let usage = unsafe { usage.assume_init() };
    (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as f64 * 1000.0
        + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1000.0
}
async fn upgrade_replay(
    State(replay): State<Replay>,
    Query(request): Query<ReplayRequest>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers)
        || !matches!(request.codec.as_str(), "jpeg" | "h264")
        || !(1..=120).contains(&request.fps)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    upgrade
        .protocols([crate::wire::PROTOBUF_PROTOCOL, crate::wire::FRAMES_PROTOCOL])
        .max_message_size(128 * 1024)
        .max_frame_size(128 * 1024)
        .on_upgrade(move |socket| async move {
            if let Err(error) = replay_connection(socket, replay, request).await {
                eprintln!("wire replay failed: {error:#}");
            }
        })
}
async fn replay_connection(
    socket: WebSocket,
    replay: Replay,
    request: ReplayRequest,
) -> Result<()> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(replay.directory.join("manifest.json"))?)?;
    let files = manifest[&request.codec]["files"]
        .as_array()
        .context("missing replay packets")?;
    let packets: Vec<Vec<u8>> = files
        .iter()
        .map(|file| std::fs::read(replay.directory.join(file.as_str().unwrap())))
        .collect::<std::io::Result<_>>()?;
    let mode = WireMode::negotiated(socket.protocol().and_then(|value| value.to_str().ok()));
    let (mut writer, reader) = socket.split();
    let (input, mut inputs) = mpsc::channel(128);
    let (controls, mut control_messages) = mpsc::channel(16);
    let commands = Arc::new(AtomicU64::new(0));
    let applied = commands.clone();
    let drain = tokio::spawn(async move {
        while let Some(command) = inputs.recv().await {
            applied.fetch_add(1, Ordering::Relaxed);
            if let InputCommand::ReleaseSync { done } = command {
                let _ = done.send(());
            }
        }
    });
    let mut flow = FrameScheduler::new(request.fps)?;
    flow.configure(true, true, SchedulingStrategy::QualityFirst);
    let (pacing, mut flow_state) = watch::channel(flow);
    let receive = tokio::spawn(receive_commands(
        reader,
        input,
        controls,
        pacing.clone(),
        mode,
    ));
    let result = async {
        let ready = json!({"type":"bench_ready","codec":request.codec,"configuration":manifest[&request.codec]["configuration"],"width":1280,"height":720,"count":packets.len()});
        writer.send(Message::Text(ready.to_string().into())).await?;
        timeout(Duration::from_secs(10), control_messages.recv()).await?.context("start required")?;
        let started = Instant::now();
        let cpu_started = cpu_ms();
        let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(request.fps)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        for (index, packet) in packets.iter().enumerate() {
            tick.tick().await;
            let id = 1000 + index as u64;
            // Single in-flight frame gives the same render-ACK contract in all modes.
            pacing.send_modify(|flow| flow.sent(id, Instant::now()));
            if !mode.inline_ids() {
                writer.send(Message::Text(json!({"type":"frame_id","id":id}).to_string().into())).await?;
            }
            let bytes = if request.codec == "jpeg" { mode.jpeg(id, packet) }
                else { mode.h264(id, manifest["h264"]["keyframes"][index].as_bool().unwrap(), index as u64 * 33333, packet) };
            writer.send(Message::Binary(bytes.into())).await?;
            timeout(Duration::from_secs(10), async {
                loop {
                    if flow_state.borrow_and_update().in_flight() == 0 { break; }
                    flow_state.changed().await?;
                }
                Ok::<_, watch::error::RecvError>(())
            }).await??;
        }
        let expected_inputs = packets.len() * 2 + packets.len().div_ceil(30) * 2;
        timeout(Duration::from_secs(2), async {
            while commands.load(Ordering::Relaxed) < expected_inputs as u64 { tokio::task::yield_now().await; }
        }).await?;
        let stats = json!({"type":"bench_done","frames":packets.len(),"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"server_cpu_ms":cpu_ms()-cpu_started,"inputs":commands.load(Ordering::Relaxed)});
        writer.send(Message::Text(stats.to_string().into())).await?;
        Ok::<_, anyhow::Error>(())
    }.await;
    receive.abort();
    drain.abort();
    let _ = receive.await;
    let _ = drain.await;
    result
}

struct SyntheticBackend {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    input: mpsc::Sender<InputCommand>,
}
impl DesktopBackend for SyntheticBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn stop(&self) {}
    fn release_input(&self) -> bool {
        true
    }
}
struct NoDisplay;
impl DisplayController for NoDisplay {
    fn outputs(&self) -> anyhow::Result<Vec<DisplayOutput>> {
        Ok(vec![])
    }
    fn capabilities(
        &self,
    ) -> futures_util::future::BoxFuture<'_, display_control::DisplayCapabilities> {
        Box::pin(async {
            display_control::DisplayCapabilities {
                resize: false,
                resize_scale: false,
                reason: Some("synthetic source".into()),
            }
        })
    }
    fn resize<'a>(
        &'a self,
        _: Option<&'a str>,
        _: DisplayMode,
        _: u32,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<DisplayMode>> {
        Box::pin(async { anyhow::bail!("synthetic source cannot resize") })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "starts a bounded localhost-only replay and synthetic application server; run scripts/benchmark-wire.mjs"]
async fn browser_wire_benchmark_server() -> Result<()> {
    let directory = PathBuf::from(std::env::var("LIGHTOS_WIRE_BENCH_DIR")?);
    let private = tempfile::tempdir()?;
    let socket = private.path().join("no-real-desktop");
    let session = Session::attached(
        private.path().to_owned(),
        socket.clone(),
        None,
        None,
        None,
        None,
    );
    let mut config = Config::try_parse_from(["test", "--password", "wire-benchmark-only"])?;
    config.audio = false;
    config.mcp_settings = Some(private.path().join("mcp.json"));
    config.rdp.rdp_settings = Some(private.path().join("rdp.json"));
    config.physical_wayland_socket = Some(socket);
    let pixels = image::load_from_memory(&std::fs::read(directory.join("frame.jpg"))?)?.to_rgba8();
    let (capture, frames) = watch::channel(Some(Arc::new(Frame::new(
        1280,
        720,
        pixels.into_raw().into(),
    ))));
    let (input, mut inputs) = mpsc::channel(128);
    let drain = tokio::spawn(async move {
        while let Some(command) = inputs.recv().await {
            if let InputCommand::ReleaseSync { done } = command {
                let _ = done.send(());
            }
        }
    });
    let mut app = AppState::new(config, SyntheticBackend { frames, input }, session);
    app.displays = Arc::new(NoDisplay);
    app.codec_capabilities
        .set(SoftwareEncoders.probe().await)
        .unwrap();
    let replay = Replay {
        directory: Arc::new(directory.clone()),
    };
    let javascript = std::fs::read_to_string(directory.join("browser.iife.js"))?;
    let routes = Router::new().route("/__wire_bench/ws",get(upgrade_replay)).with_state(replay)
        .route("/__wire_bench",get(|| async { axum::response::Html("<!doctype html><meta charset=utf-8><canvas id=video width=1280 height=720></canvas><script src=/__wire_bench.js></script>") }))
        .route("/__wire_bench.js",get(move || async move { ([(header::CONTENT_TYPE,"text/javascript")],javascript) }));
    use axum::serve::ListenerExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let nodelay = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    std::fs::write(
        directory.join("address.json"),
        json!({"address":listener.local_addr()?.to_string(),"nodelayAddress":nodelay.local_addr()?.to_string(),"pid":std::process::id()}).to_string(),
    )?;
    println!("wire benchmark server {}", listener.local_addr()?);
    let routes = router(app).merge(routes);
    let stop = tokio_util::sync::CancellationToken::new();
    let signal = stop.clone();
    tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(900), tokio::signal::ctrl_c()).await;
        signal.cancel();
    });
    let default_server = axum::serve(listener, routes.clone())
        .with_graceful_shutdown(stop.clone().cancelled_owned());
    let nodelay_server = axum::serve(
        nodelay.tap_io(|stream| {
            stream.set_nodelay(true).expect("benchmark TCP_NODELAY");
        }),
        routes,
    )
    .with_graceful_shutdown(stop.cancelled_owned());
    tokio::try_join!(async { default_server.await }, async {
        nodelay_server.await
    })?;
    drop(capture);
    drain.abort();
    Ok(())
}
