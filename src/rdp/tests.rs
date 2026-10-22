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
        active: AtomicBool::new(false),
        lease: Mutex::new(None),
        ready: Notify::new(),
        close: Notify::new(),
    })
}

#[tokio::test]
async fn credentials_gate_pixels_input_and_shared_browser_controller() {
    let (state, _frames, mut input) = fixture();
    let connection = connection(state.clone());
    let mut display = Display {
        connection: connection.clone(),
        negotiated: None,
    };
    assert!(display.updates().await.is_err());
    assert!(!connection.active.load(Ordering::Acquire));
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
    connection.active.store(false, Ordering::Release);
    let lease = connection.lease.lock().unwrap().take().unwrap();
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
        size: Some((320, 200)),
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
    connection.active.store(true, Ordering::Release);
    let input = Input {
        connection: connection.clone(),
        surrogate: None,
    };
    input.send(InputCommand::Key {
        code: 30,
        pressed: false,
    });
    assert!(!connection.active.load(Ordering::Acquire));
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
