use super::*;
use crate::backend::Frame;
use crate::{backend::BackendKind, session::Session};
use clap::Parser;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message as ClientMessage, client::IntoClientRequest},
};

type Client = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct StaticDesktop {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    input: mpsc::Sender<InputCommand>,
    privacy: Arc<Privacy>,
}
#[derive(Default)]
struct Privacy {
    locked: AtomicBool,
    epoch: AtomicU64,
}
impl DesktopBackend for StaticDesktop {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn paused(&self) -> bool {
        self.privacy.locked.load(Ordering::Acquire)
    }
    fn privacy_epoch(&self) -> u64 {
        self.privacy.epoch.load(Ordering::Acquire)
    }
    fn stop(&self) {}
}
struct ReadOnlyDisplay;
impl DisplayController for ReadOnlyDisplay {
    fn outputs(&self) -> anyhow::Result<Vec<DisplayOutput>> {
        Ok(Vec::new())
    }
    fn capabilities(
        &self,
    ) -> futures_util::future::BoxFuture<'_, display_control::DisplayCapabilities> {
        Box::pin(async {
            display_control::DisplayCapabilities {
                resize: false,
                resize_scale: false,
                reason: None,
            }
        })
    }
    fn resize<'a>(
        &'a self,
        _: Option<&'a str>,
        _: DisplayMode,
        _: u32,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<DisplayMode>> {
        Box::pin(async { anyhow::bail!("video scaling must not change the display mode") })
    }
}
struct Connection {
    client: Client,
    capture: watch::Sender<Option<Arc<Frame>>>,
    privacy: Arc<Privacy>,
    pause: Arc<EncodePause>,
    server: tokio::task::JoinHandle<()>,
    input: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.server.abort();
        self.input.abort();
    }
}
impl Connection {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket_path = directory.path().join("wayland-test");
        let session = Session::attached(
            directory.path().to_owned(),
            socket_path,
            None,
            None,
            None,
            None,
        );
        let mut config =
            Config::try_parse_from(["test", "--password", "test-scaling-secret"]).unwrap();
        config.mcp_settings = Some(directory.path().join("mcp.json"));
        config.rdp.rdp_settings = Some(directory.path().join("rdp.json"));
        config.physical_wayland_socket = Some(directory.path().join("no-physical-desktop"));
        let (capture, frames) = watch::channel(Some(Arc::new(Frame::test_frame(1280, 800))));
        let (input_sender, mut commands) = mpsc::channel(128);
        let input = tokio::spawn(async move {
            while let Some(command) = commands.recv().await {
                if let InputCommand::ReleaseSync { done } = command {
                    let _ = done.send(());
                }
            }
        });
        let privacy = Arc::new(Privacy::default());
        let pause = Arc::new(EncodePause {
            entered: tokio::sync::Notify::new(),
            release: Mutex::new(None),
        });
        let mut state = AppState::new(
            config,
            StaticDesktop {
                frames,
                input: input_sender,
                privacy: privacy.clone(),
            },
            session,
        );
        state.encode_pause = Some(pause.clone());
        state.displays = Arc::new(ReadOnlyDisplay);
        state
            .codec_capabilities
            .capabilities(HardwareEncoders::discover())
            .await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async { axum::serve(listener, router(state)).await.unwrap() });
        let mut request = format!("ws://{address}/ws").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", format!("http://{address}").parse().unwrap());
        request.headers_mut().insert(
            "Authorization",
            format!("Basic {}", STANDARD.encode("lightos:test-scaling-secret"))
                .parse()
                .unwrap(),
        );
        let (client, _) = connect_async(request).await.unwrap();
        Self {
            client,
            capture,
            privacy,
            pause,
            server,
            input,
            _directory: directory,
        }
    }
    async fn send(&mut self, value: serde_json::Value) {
        self.client
            .send(ClientMessage::Text(value.to_string().into()))
            .await
            .unwrap();
    }
    async fn next(&mut self) -> ClientMessage {
        timeout(Duration::from_secs(5), self.client.next())
            .await
            .expect("stream stalled on a static desktop")
            .unwrap()
            .unwrap()
    }
    async fn jpeg(&mut self, size: (u32, u32)) -> image::DynamicImage {
        let mut metadata = None;
        loop {
            match self.next().await {
                ClientMessage::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "ready" {
                        assert_eq!(value["video_scaling"], true);
                    }
                    if value["type"] == "frame_id" {
                        metadata = Some(value);
                    }
                }
                ClientMessage::Binary(bytes) if bytes[0] == 1 => {
                    let image = image::load_from_memory(&bytes[1..]).unwrap();
                    if let Some(ref value) = metadata {
                        self.send(json!({"type":"frame_ack","id":value["id"]}))
                            .await;
                    }
                    if (image.width(), image.height()) == size {
                        if size != (1280, 800) {
                            let value = metadata.unwrap();
                            assert_eq!(
                                (
                                    value["source_width"].as_u64(),
                                    value["source_height"].as_u64()
                                ),
                                (Some(1280), Some(800))
                            );
                        }
                        return image;
                    }
                    metadata = None;
                }
                ClientMessage::Close(_) => panic!("video scaling closed the connection"),
                _ => {}
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_jpeg_scales_and_restores_native_without_changing_capture() {
    let mut connection = Connection::new().await;
    connection.jpeg((1280, 800)).await; // Older clients keep native resolution.
    connection
        .send(json!({"type":"video_flow","enabled":true,"sequenced":true}))
        .await;
    connection
        .send(json!({"type":"video_scale","max_size":{"width":640,"height":400}}))
        .await;
    connection.jpeg((640, 400)).await;
    connection
        .send(
            json!({"type":"video_flow","enabled":true,"sequenced":true,"strategy":"quality_first"}),
        )
        .await;
    connection.jpeg((1280, 800)).await;
    connection
        .send(json!({"type":"video_flow","enabled":true,"sequenced":true,"strategy":"auto"}))
        .await;
    connection.jpeg((640, 400)).await;
    connection
        .send(json!({"type":"video_scale","max_size":null}))
        .await;
    connection.jpeg((1280, 800)).await;
    let capture = connection.capture.borrow();
    let frame = capture.as_ref().unwrap();
    assert_eq!((frame.width, frame.height), (1280, 800));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h264_reconfigures_and_decodes_after_static_scale_change() {
    use openh264::formats::YUVSource;
    let mut connection = Connection::new().await;
    connection.jpeg((1280, 800)).await;
    connection
        .send(json!({"type":"video_flow","enabled":true,"sequenced":true}))
        .await;
    connection
        .send(json!({"type":"video_scale","max_size":{"width":640,"height":400}}))
        .await;
    connection
        .send(json!({"type":"video","codec":"h264","backend":"software","request_id":1}))
        .await;
    let mut decoder = openh264::decoder::Decoder::new().unwrap();
    for size in [(640, 400), (1280, 800)] {
        if size == (1280, 800) {
            connection
                .send(json!({"type":"video_scale","max_size":null}))
                .await;
        }
        let mut configured = false;
        let mut metadata = None;
        loop {
            match connection.next().await {
                ClientMessage::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "video_config"
                        && value["width"] == size.0
                        && value["height"] == size.1
                    {
                        assert_eq!(value["request_id"], 1);
                        assert_eq!(value["backend"], "openh264");
                        decoder = openh264::decoder::Decoder::new().unwrap();
                        configured = true;
                    }
                    if value["type"] == "frame_id" {
                        metadata = Some(value);
                    }
                }
                ClientMessage::Binary(bytes) => {
                    if let Some(ref value) = metadata {
                        connection
                            .send(json!({"type":"frame_ack","id":value["id"]}))
                            .await;
                    }
                    if bytes[0] == 3 && configured {
                        assert_eq!(bytes[1], 1, "dimension change starts a keyframe");
                        let frame = decoder.decode(&bytes[10..]).unwrap().unwrap();
                        assert_eq!(frame.dimensions(), (size.0 as usize, size.1 as usize));
                        let value = metadata.unwrap();
                        assert_eq!(value["source_width"], 1280);
                        assert_eq!(value["source_height"], 800);
                        break;
                    }
                    metadata = None;
                }
                ClientMessage::Close(_) => panic!("H264 scaling closed the connection"),
                _ => {}
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scaled_buffer_is_discarded_when_lock_epoch_changes_during_encode() {
    let mut connection = Connection::new().await;
    connection.jpeg((1280, 800)).await;
    connection
        .send(json!({"type":"video_flow","enabled":true,"sequenced":true}))
        .await;
    connection
        .send(json!({"type":"video_scale","max_size":{"width":640,"height":400}}))
        .await;
    connection.jpeg((640, 400)).await;
    let (release, receiver) = std::sync::mpsc::sync_channel(1);
    *connection.pause.release.lock().unwrap() = Some(receiver);
    connection
        .send(json!({"type":"video_scale","max_size":{"width":800,"height":500}}))
        .await;
    timeout(Duration::from_secs(3), connection.pause.entered.notified())
        .await
        .unwrap();
    connection.privacy.locked.store(true, Ordering::Release);
    connection.privacy.epoch.fetch_add(1, Ordering::AcqRel);
    connection.capture.send_replace(None);
    connection.privacy.locked.store(false, Ordering::Release);
    connection.privacy.epoch.fetch_add(1, Ordering::AcqRel);
    let pixels = [230, 10, 10, 255].repeat(1280 * 800);
    connection
        .capture
        .send_replace(Some(Arc::new(Frame::new(1280, 800, pixels.into()))));
    release.send(()).unwrap();
    let frame = connection.jpeg((800, 500)).await.to_rgb8();
    let pixel = frame.get_pixel(400, 250);
    assert!(
        pixel[0] > pixel[2],
        "pre-lock scaled pixels leaked: {pixel:?}"
    );
}

#[test]
fn bounds_reject_invalid_requests() {
    for value in [
        json!({}),
        json!({"max_size":{}}),
        json!({"max_size":{"width":0,"height":400}}),
        json!({"max_size":{"width":640.5,"height":400}}),
        json!({"max_size":{"width":8192,"height":8192}}),
    ] {
        assert!(parse_video_bounds(&value).is_err(), "{value}");
    }
    assert_eq!(parse_video_bounds(&json!({"max_size":null})).unwrap(), None);
    assert_eq!(
        parse_video_bounds(&json!({"max_size":{"width":640,"height":400}})).unwrap(),
        Some((640, 400))
    );
}
