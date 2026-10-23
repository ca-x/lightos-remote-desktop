use super::*;
use crate::{
    config::Config,
    session::Session,
    wayland::tests::{rdp_backend, rdp_frame},
};
use clap::Parser;
use tokio::sync::mpsc;

fn fixture() -> (
    AppState,
    watch::Sender<Option<Arc<Frame>>>,
    mpsc::Receiver<InputCommand>,
) {
    let config = Config::try_parse_from(["test", "--password", "rdp-test-password"]).unwrap();
    let (backend, frames, input) = rdp_backend();
    frames.send_replace(Some(rdp_frame(320, 200, [230, 40, 60, 255])));
    let session = Session::attached(
        "/tmp/lrd-rdp-test".into(),
        "/tmp/lrd-rdp-test/wayland".into(),
        None,
        None,
        None,
        None,
    );
    (AppState::new(config, backend, session), frames, input)
}

fn connection(state: AppState) -> Arc<Connection> {
    Arc::new(Connection {
        state,
        authenticated: AtomicBool::new(false),
        control: Mutex::new(Control::default()),
        ready: Notify::new(),
        close: Notify::new(),
        before_input_enqueue: Mutex::new(None),
    })
}

pub(super) struct InputPause {
    entered: tokio::sync::oneshot::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

impl InputPause {
    pub(super) fn wait(self) {
        self.entered.send(()).unwrap();
        self.resume.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

fn acknowledge_input_releases(
    mut input: mpsc::Receiver<InputCommand>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(command) = input.recv().await {
            if let InputCommand::ReleaseSync { done } = command {
                let _ = done.send(());
            }
        }
    })
}

#[tokio::test]
async fn capture_changes_during_initial_activation_require_a_resize() {
    let (state, frames, input) = fixture();
    let releases = acknowledge_input_releases(input);
    let connection = connection(state);
    connection.authenticated.store(true, Ordering::Release);
    let mut display = Display {
        connection: connection.clone(),
        announced: Arc::default(),
    };
    // Mirror IronRDP's capability announcement, client size request and final
    // encoder setup, with capture changing after the announcement.
    let advertised = display.size().await;
    assert_eq!((advertised.width, advertised.height), (320, 200));
    frames.send_replace(Some(rdp_frame(640, 480, [30, 80, 210, 255])));
    let requested = display.request_initial_size(advertised).await;
    assert_eq!((requested.width, requested.height), (320, 200));
    let encoder = display.size().await;
    assert_eq!((encoder.width, encoder.height), (320, 200));
    let mut updates = display.updates().await.unwrap();
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::HidePointer)
    ));
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::Resize(DesktopSize {
            width: 640,
            height: 480
        }))
    ));
    connection.deactivate().unwrap().release().await;
    releases.abort();
}

#[tokio::test]
async fn capture_changes_during_reactivation_require_another_resize() {
    let (state, frames, input) = fixture();
    let releases = acknowledge_input_releases(input);
    let connection = connection(state);
    connection.authenticated.store(true, Ordering::Release);
    let mut display = Display {
        connection: connection.clone(),
        announced: Arc::default(),
    };
    let _advertised = display.size().await;
    let mut updates = display.updates().await.unwrap();
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::HidePointer)
    ));
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::Bitmap(_))
    ));
    frames.send_replace(Some(rdp_frame(640, 480, [30, 80, 210, 255])));
    let Some(DisplayUpdate::Resize(advertised)) = updates.next_update().await.unwrap() else {
        panic!("missing first resize")
    };
    assert_eq!((advertised.width, advertised.height), (640, 480));
    // A Resize ends IronRDP's client loop and drops its update stream. Capture
    // continues changing while the replacement acceptor negotiates that size.
    drop(updates);
    frames.send_replace(Some(rdp_frame(800, 600, [1, 2, 3, 255])));
    let requested = display.request_initial_size(advertised).await;
    assert_eq!((requested.width, requested.height), (640, 480));
    frames.send_replace(Some(rdp_frame(960, 540, [4, 5, 6, 255])));
    let encoder = display.size().await;
    assert_eq!((encoder.width, encoder.height), (640, 480));
    let mut updates = display.updates().await.unwrap();
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::HidePointer)
    ));
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::Resize(DesktopSize {
            width: 960,
            height: 540
        }))
    ));
    drop(updates);
    let encoder = display.size().await;
    assert_eq!((encoder.width, encoder.height), (960, 540));
    let mut updates = display.updates().await.unwrap();
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::HidePointer)
    ));
    let Some(DisplayUpdate::Bitmap(bitmap)) = updates.next_update().await.unwrap() else {
        panic!("missing bitmap after the second reactivation")
    };
    assert_eq!((bitmap.width.get(), bitmap.height.get()), (960, 540));
    assert_eq!(bitmap.data[..4], [6, 5, 4, 255]);
    connection.deactivate().unwrap().release().await;
    releases.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_waits_for_accepted_input_before_releasing_the_controller() {
    let (state, _frames, mut commands) = fixture();
    let connection = connection(state.clone());
    connection.authenticated.store(true, Ordering::Release);
    let mut display = Display {
        connection: connection.clone(),
        announced: Arc::default(),
    };
    let _updates = display.updates().await.unwrap();
    let (recorded, mut records) = mpsc::unbounded_channel();
    let worker = tokio::spawn(async move {
        while let Some(command) = commands.recv().await {
            let command = if let InputCommand::ReleaseSync { done } = command {
                let _ = done.send(());
                InputCommand::Release
            } else {
                command
            };
            recorded.send(command).unwrap();
        }
    });
    let (entered, paused) = tokio::sync::oneshot::channel();
    let (resume, waiting) = std::sync::mpsc::channel();
    *connection.before_input_enqueue.lock().unwrap() = Some(InputPause {
        entered,
        resume: waiting,
    });
    let input = Input {
        connection: connection.clone(),
        surrogate: None,
    };
    // Model IronRDP's AInput worker stopping after accepting an event but
    // before it enters the Wayland queue.
    let callback = tokio::task::spawn_blocking(move || {
        input.send(InputCommand::Key {
            code: 30,
            pressed: true,
        });
    });
    paused.await.unwrap();
    let (started, closing) = tokio::sync::oneshot::channel();
    let mut disconnect = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        drop(connection.deactivate());
    });
    closing.await.unwrap();
    let released_while_paused = tokio::time::timeout(Duration::from_millis(100), &mut disconnect)
        .await
        .is_ok();
    let next_controller = released_while_paused.then(|| state.claim_rdp_control().unwrap());
    // Always resume/join the paused worker before asserting, including on RED.
    resume.send(()).unwrap();
    callback.await.unwrap();
    if !released_while_paused {
        disconnect.await.unwrap();
    }
    let next_controller = next_controller.unwrap_or_else(|| state.claim_rdp_control().unwrap());
    Input {
        connection: display.connection.clone(),
        surrogate: None,
    }
    .send(InputCommand::Key {
        code: 32,
        pressed: true,
    });
    state
        .backend
        .input()
        .try_send(InputCommand::Key {
            code: 31,
            pressed: true,
        })
        .unwrap();
    next_controller.release().await;
    let mut received = Vec::new();
    for _ in 0..4 {
        received.push(records.recv().await.unwrap());
    }
    worker.abort();
    assert!(
        !released_while_paused,
        "disconnect released the controller while an accepted input callback was paused"
    );
    assert!(matches!(
        received[0],
        InputCommand::Key {
            code: 30,
            pressed: true
        }
    ));
    assert!(matches!(received[1], InputCommand::Release));
    assert!(matches!(
        received[2],
        InputCommand::Key {
            code: 31,
            pressed: true
        }
    ));
    assert!(matches!(received[3], InputCommand::Release));
}

#[tokio::test]
async fn credentials_gate_pixels_input_and_shared_browser_controller() {
    let (state, _frames, mut input) = fixture();
    let connection = connection(state.clone());
    let mut display = Display {
        connection: connection.clone(),
        announced: Arc::default(),
    };
    assert!(display.updates().await.is_err());
    assert!(!connection.control.lock().unwrap().active);
    for (username, password, domain) in [
        ("lightos", "wrong-secret", None),
        ("", "", None),
        ("lightos", "rdp-test-password", Some("other")),
    ] {
        let decision = connection
            .validate(&Credentials {
                username: username.into(),
                password: password.into(),
                domain: domain.map(str::to_owned),
            })
            .await
            .unwrap();
        assert!(matches!(decision, CredentialDecision::Reject));
    }
    let decision = connection
        .validate(&Credentials {
            username: "lightos".into(),
            password: "rdp-test-password".into(),
            domain: None,
        })
        .await
        .unwrap();
    assert!(matches!(decision, CredentialDecision::Accept));
    // Even validated credentials do not reserve browser control before activation.
    let browser = state.claim_rdp_control().unwrap();
    assert!(display.updates().await.is_err());
    let (recorded, mut records) = mpsc::unbounded_channel();
    let releases = tokio::spawn(async move {
        while let Some(command) = input.recv().await {
            let command = if let InputCommand::ReleaseSync { done } = command {
                let _ = done.send(());
                InputCommand::Release
            } else {
                command
            };
            let _ = recorded.send(command);
        }
    });
    browser.release().await;
    let mut updates = display.updates().await.unwrap();
    assert!(state.claim_rdp_control().is_err());
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::HidePointer)
    ));
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::Bitmap(_))
    ));
    let mut input = Input {
        connection: connection.clone(),
        surrogate: None,
    };
    input.keyboard(KeyboardEvent::Pressed {
        code: 0x1d,
        extended: true,
    });
    input.mouse(MouseEvent::Move { x: 319, y: 199 });
    input.keyboard(KeyboardEvent::UnicodePressed('中' as u16));
    let lease = connection.deactivate().unwrap();
    lease.release().await;
    let mut commands = Vec::new();
    for _ in 0..5 {
        commands.push(
            tokio::time::timeout(Duration::from_secs(2), records.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert!(matches!(commands[0], InputCommand::Release));
    assert!(matches!(
        commands[1],
        InputCommand::Key {
            code: 97,
            pressed: true
        }
    ));
    assert!(matches!(commands[2], InputCommand::Move { x: 1.0, y: 1.0 }));
    assert!(matches!(&commands[3], InputCommand::Text { text } if text == "中"));
    assert!(matches!(commands[4], InputCommand::Release));
    state.claim_rdp_control().unwrap().release().await;
    releases.abort();
}

#[tokio::test]
async fn display_uses_latest_raw_frame_and_resizes_before_new_geometry() {
    let (_state, frames, _input) = fixture();
    let mut updates = Updates {
        frames: frames.subscribe(),
        first: true,
        hide_pointer: false,
        announced: Arc::new(Mutex::new(Some((320, 200)))),
    };
    let Some(DisplayUpdate::Bitmap(bitmap)) = updates.next_update().await.unwrap() else {
        panic!("missing initial bitmap")
    };
    assert_eq!(bitmap.data[..4], [60, 40, 230, 255]);
    frames.send_replace(Some(rdp_frame(400, 240, [20, 200, 80, 255])));
    frames.send_replace(Some(rdp_frame(640, 480, [30, 80, 210, 255])));
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::Resize(DesktopSize {
            width: 640,
            height: 480
        }))
    ));
    let Some(DisplayUpdate::Bitmap(bitmap)) = updates.next_update().await.unwrap() else {
        panic!("missing resized bitmap")
    };
    assert_eq!((bitmap.width.get(), bitmap.height.get()), (640, 480));
    assert_eq!(bitmap.data[..4], [210, 80, 30, 255]);
    let retained = frames.borrow().as_ref().unwrap().pixels.clone();
    assert_eq!(&retained[..4], [30, 80, 210, 255]);
    assert_eq!(bitmap.format, PixelFormat::BgrA32);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), updates.next_update())
            .await
            .is_err()
    );
    frames.send_replace(Some(rdp_frame(640, 480, [1, 2, 3, 255])));
    assert!(matches!(
        updates.next_update().await.unwrap(),
        Some(DisplayUpdate::Bitmap(_))
    ));
}

#[tokio::test]
async fn stopped_input_route_terminates_control_instead_of_losing_releases() {
    let (state, _frames, input) = fixture();
    drop(input);
    let connection = connection(state);
    connection.control.lock().unwrap().active = true;
    let input = Input {
        connection: connection.clone(),
        surrogate: None,
    };
    input.send(InputCommand::Key {
        code: 30,
        pressed: false,
    });
    assert!(!connection.control.lock().unwrap().active);
    tokio::time::timeout(Duration::from_millis(20), connection.close.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn default_disabled_starts_no_rdp_transport() {
    let (state, _frames, _input) = fixture();
    assert!(start(state).await.unwrap().is_none());
}

#[test]
fn scan_codes_map_modifiers_and_navigation_and_ignore_unknown_keys() {
    assert_eq!(evdev_scancode(0x1e, false), Some(30));
    assert_eq!(evdev_scancode(0x1d, false), Some(29));
    assert_eq!(evdev_scancode(0x1d, true), Some(97));
    assert_eq!(evdev_scancode(0x53, true), Some(111));
    assert_eq!(evdev_scancode(0, false), None);
    assert_eq!(evdev_scancode(0xff, true), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires LRD_TEST_FREERDP, LRD_TEST_XVFB, Python 3 and OpenSSL; uses only a private Xvfb display"]
async fn freerdp_tcp_tls_bitmap_input_resize_reconnect_and_denials() {
    let freerdp =
        std::env::var("LRD_TEST_FREERDP").expect("set LRD_TEST_FREERDP to a FreeRDP X11 client");
    let xvfb = std::env::var("LRD_TEST_XVFB").expect("set LRD_TEST_XVFB to Xvfb");
    let openssl = platform_detect::find_executable(std::path::Path::new("openssl"))
        .expect("OpenSSL is required for this opt-in check");
    let python = platform_detect::find_executable(std::path::Path::new("python3"))
        .expect("Python 3 is required for this opt-in check");
    let temp = tempfile::tempdir().unwrap();
    let cert = temp.path().join("cert.pem");
    let key = temp.path().join("key.pem");
    let generated = std::process::Command::new(&openssl)
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-sha256",
            "-days",
            "1",
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let digest = std::process::Command::new(openssl)
        .args(["x509", "-noout", "-fingerprint", "-sha256", "-in"])
        .arg(&cert)
        .output()
        .unwrap();
    assert!(digest.status.success());
    let fingerprint = String::from_utf8(digest.stdout)
        .unwrap()
        .trim()
        .split_once('=')
        .unwrap()
        .1
        .replace(':', "");
    let (mut state, frames, mut input) = fixture();
    let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    let config = Arc::make_mut(&mut state.config);
    config.rdp.rdp = true;
    config.rdp.rdp_listen.set_port(port);
    config.rdp.rdp_tls_cert = Some(cert);
    config.rdp.rdp_tls_key = Some(key);
    drop(reserved);
    let service = start(state.clone()).await.unwrap().unwrap();
    let occupied = start(state.clone()).await.err().unwrap();
    assert!(occupied.to_string().contains("binding RDP listener"));
    let received = Arc::new(Mutex::new(Vec::new()));
    let events = received.clone();
    let published = frames.clone();
    let input_worker = tokio::spawn(async move {
        while let Some(command) = input.recv().await {
            match command {
                InputCommand::Key {
                    code: 30,
                    pressed: true,
                } => {
                    published.send_replace(Some(rdp_frame(640, 480, [30, 80, 210, 255])));
                }
                InputCommand::ReleaseSync { ref done } => {
                    let _ = done.send(());
                }
                _ => {}
            }
            events.lock().unwrap().push(command);
        }
    });
    // A TCP peer sending no handshake must time out without owning browser
    // control. This uses the real production deadline, not a test-only shortcut.
    let mut idle = tokio::net::TcpStream::connect(service.address())
        .await
        .unwrap();
    state.claim_rdp_control().unwrap().release().await;
    let mut byte = [0];
    let closed = tokio::time::timeout(
        HANDSHAKE_TIMEOUT + Duration::from_secs(2),
        tokio::io::AsyncReadExt::read(&mut idle, &mut byte),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(closed, 0);
    frames.send_replace(Some(rdp_frame(320, 200, [230, 40, 60, 255])));
    let address = service.address();
    let output = tokio::process::Command::new(python)
        .arg(format!(
            "{}/scripts/rdp-interop.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .args([
            "--freerdp",
            &freerdp,
            "--xvfb",
            &xvfb,
            "--port",
            &service.address().port().to_string(),
            "--fingerprint",
            &fingerprint,
        ])
        .output()
        .await
        .unwrap();
    service.stop().await.unwrap();
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    input_worker.abort();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = received.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|command| matches!(command, InputCommand::Move { .. }))
    );
    assert!(events.iter().any(|command| matches!(
        command,
        InputCommand::Button {
            button: 0,
            pressed: true
        }
    )));
    assert!(events.iter().any(|command| matches!(
        command,
        InputCommand::Key {
            code: 30,
            pressed: true
        }
    )));
    assert!(
        events
            .iter()
            .filter(|command| matches!(command, InputCommand::ReleaseSync { .. }))
            .count()
            >= 2
    );
}

#[tokio::test]
async fn invalid_tls_files_fail_before_opening_the_rdp_listener() {
    let (mut state, _frames, _input) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let cert = temp.path().join("cert.pem");
    let key = temp.path().join("key.pem");
    let config = Arc::make_mut(&mut state.config);
    config.rdp.rdp = true;
    config.rdp.rdp_tls_cert = Some(cert.clone());
    config.rdp.rdp_tls_key = Some(key.clone());
    let error = start(state.clone()).await.err().unwrap();
    assert!(error.to_string().contains("opening RDP certificate"));
    std::fs::write(cert, b"not a certificate").unwrap();
    std::fs::write(key, b"not a private key").unwrap();
    let error = start(state).await.err().unwrap();
    assert!(error.to_string().contains("certificate chain is empty"));
}

#[tokio::test]
async fn browser_mcp_and_rdp_share_control_and_release_after_cancellation() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use futures_util::StreamExt;
    use serde_json::{Value, json};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    async fn rpc(
        client: &reqwest::Client,
        base: &str,
        token: &str,
        session: &str,
        value: Value,
    ) -> Value {
        let response = client
            .post(format!("{base}/mcp"))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Mcp-Session-Id", session)
            .json(&value)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let text = response.text().await.unwrap();
        if text.is_empty() {
            return Value::Null;
        }
        let body = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .next_back()
            .unwrap_or(&text);
        serde_json::from_str(body).unwrap()
    }
    let directory = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir()
        .unwrap();
    let (mut state, frames, mut input) = fixture();
    state.mcp = Arc::new(crate::mcp::TokenStore::open(Some(
        directory.path().join("mcp.json"),
    )));
    Arc::make_mut(&mut state.config).subfolder = "/integration".into();
    let drag_started = Arc::new(Notify::new());
    let started = drag_started.clone();
    let worker = tokio::spawn(async move {
        while let Some(command) = input.recv().await {
            let Some((command, guard)) = command.into_active() else {
                continue;
            };
            match command {
                InputCommand::ReleaseSync { done } => {
                    let _ = done.send(());
                }
                InputCommand::Button { pressed: true, .. } => started.notify_one(),
                _ => {}
            }
            if let Some(guard) = guard {
                guard.text_done.notify_one();
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = crate::server::router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{address}/integration");
    let origin = format!("http://{address}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();
    let managed = |method, suffix: &str| {
        client
            .request(method, format!("{base}/{suffix}"))
            .basic_auth("lightos", Some("rdp-test-password"))
            .header("Origin", &origin)
            .header("X-LightOS-MCP", "1")
    };
    let enabled = managed(reqwest::Method::PUT, "api/mcp")
        .json(&json!({"enabled":true}))
        .send()
        .await
        .unwrap();
    let status = enabled.status();
    let text = enabled.text().await.unwrap();
    assert!(status.is_success(), "MCP setup {status}: {text}");
    let token: Value = managed(reqwest::Method::POST, "api/mcp/tokens")
        .json(&json!({"name":"Integration", "scope":"control"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = token["token"].as_str().unwrap().to_owned();
    let response = client.post(format!("{base}/mcp")).bearer_auth(&token)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"Integrated transports","version":"1"}}}))
        .send().await.unwrap();
    assert!(response.status().is_success());
    let session = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let _ = response.text().await.unwrap();
    assert!(
        client
            .post(format!("{base}/mcp"))
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Mcp-Session-Id", &session)
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );

    let connection = connection(state.clone());
    connection.authenticated.store(true, Ordering::Release);
    let mut display = Display {
        connection: connection.clone(),
        announced: Arc::default(),
    };
    let mut websocket = format!("ws://{address}/integration/ws")
        .into_client_request()
        .unwrap();
    websocket.headers_mut().insert(
        "Authorization",
        format!("Basic {}", STANDARD.encode("lightos:rdp-test-password"))
            .parse()
            .unwrap(),
    );
    websocket
        .headers_mut()
        .insert("Origin", origin.parse().unwrap());
    let (mut browser, _) = tokio_tungstenite::connect_async(websocket).await.unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(5), browser.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(ready.to_text().unwrap().contains("\"type\":\"ready\""));
    assert!(display.updates().await.is_err(), "browser must exclude RDP");
    let move_call = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mouse_move","arguments":{"x":10,"y":10}}});
    assert_eq!(
        rpc(&client, &base, &token, &session, move_call.clone()).await["result"]["isError"],
        true,
        "browser must exclude MCP input"
    );
    browser.close(None).await.unwrap();
    let updates = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match display.updates().await {
                Ok(updates) => break updates,
                Err(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        rpc(&client, &base, &token, &session, move_call).await["result"]["isError"],
        true,
        "RDP must exclude MCP input"
    );
    frames.send_replace(Some(rdp_frame(320, 200, [230, 40, 60, 255])));
    let info = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"desktop_info","arguments":{}}});
    let info = rpc(&client, &base, &token, &session, info).await;
    assert!(
        info["result"]["content"].is_array() && info["result"]["isError"] != true,
        "observation remains available: {info}"
    );
    drop(updates);
    connection.deactivate().unwrap().release().await;

    let drag = tokio::spawn({
        let (client, base, token, session) =
            (client.clone(), base.clone(), token.clone(), session.clone());
        async move {
            rpc(&client, &base, &token, &session, json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"mouse_drag","arguments":{"from_x":0,"from_y":0,"to_x":63,"to_y":47,"duration_ms":1000}}})).await
        }
    });
    tokio::time::timeout(Duration::from_secs(3), drag_started.notified())
        .await
        .unwrap();
    assert!(
        display.updates().await.is_err(),
        "MCP drag must exclude RDP"
    );
    assert!(
        state.claim_rdp_control().is_err(),
        "MCP drag must exclude browser control"
    );
    assert!(client.post(format!("{base}/mcp")).bearer_auth(&token)
        .header("Accept", "application/json, text/event-stream").header("MCP-Protocol-Version", "2025-11-25").header("Mcp-Session-Id", &session)
        .json(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7,"reason":"Integration handover"}}))
        .send().await.unwrap().status().is_success());
    let result = drag.await.unwrap();
    assert!(
        result.is_null() || result.get("error").is_some() || result["result"]["isError"] == true,
        "canceled drag must not succeed: {result}"
    );
    state.claim_rdp_control().unwrap().release().await;
    let updates = display.updates().await.unwrap();
    drop(updates);
    connection.deactivate().unwrap().release().await;
    server.abort();
    worker.abort();
}
