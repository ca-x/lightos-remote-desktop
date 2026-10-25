#![cfg(target_os = "linux")]
//! Explicit local integration test; creates its own virtual desktop, never restarts production.
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        let until = Instant::now() + Duration::from_secs(10);
        while self.0.try_wait().unwrap().is_none() && Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        if self.0.try_wait().unwrap().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn ipc(runtime: &Path, signature: &str, request: &str) -> String {
    let mut socket =
        UnixStream::connect(runtime.join("hypr").join(signature).join(".socket.sock")).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    socket.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    response
}
fn health(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(200),
    ) else {
        return false;
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    if stream
        .write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok()
        && response.starts_with("HTTP/1.0 200")
        && response.contains("\"native\":true")
}

fn resize_through_websocket(port: u16, width: u32, height: u32, scale: Option<f64>) {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut request = format!("ws://127.0.0.1:{port}/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "Authorization",
            format!(
                "Basic {}",
                STANDARD.encode("lightos:handoff-live-test-secret")
            )
            .parse()
            .unwrap(),
        );
        request.headers_mut().insert(
            "Origin",
            format!("http://127.0.0.1:{port}").parse().unwrap(),
        );
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), connect_async(request))
            .await
            .unwrap()
            .unwrap();
        let ready = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(ready, Message::Text(ref text) if text.contains("\"type\":\"ready\"")),
            "{ready:?}"
        );
        let Message::Text(ready) = ready else {
            unreachable!()
        };
        let ready: Value = serde_json::from_str(&ready).unwrap();
        assert_eq!(ready["resize"], true, "{ready}");
        assert_eq!(ready["resize_scale"], true, "{ready}");
        assert!(ready["resize_reason"].is_null(), "{ready}");
        let mut resize = serde_json::json!({"type":"resize", "width":width, "height":height});
        if let Some(scale) = scale {
            resize["scale"] = serde_json::json!(scale);
        }

        socket
            .send(Message::Text(resize.to_string().into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(message) = socket.next().await {
                match message.unwrap() {
                    Message::Text(text) => assert!(!text.contains("\"type\":\"error\""), "{text}"),
                    Message::Binary(bytes) if bytes.first() == Some(&1) => {
                        let frame = image::load_from_memory(&bytes[1..]).unwrap();
                        if frame.width() == width && frame.height() == height {
                            return;
                        }
                    }
                    _ => {}
                }
            }
            panic!("desktop connection closed before resize");
        })
        .await
        .expect("resized real frame from native Lua desktop");
        socket.close(None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(socket.next().await.unwrap().unwrap(), Message::Close(_)) {
                    break;
                }
            }
        })
        .await
        .expect("control was not released before closing");
    });
}

#[test]
#[ignore = "requires local GPU, Cage, Omarchy, a supported terminal and the installed private Aquamarine library"]
fn transport_restart_preserves_native_desktop_and_gui_process() {
    for name in [
        "Hyprland",
        "cage",
        "dbus-daemon",
        "quickshell",
        "hyprctl",
        "timeout",
        "wl-copy",
        "wl-paste",
    ] {
        assert!(
            platform_detect::find_executable(Path::new(name)).is_some(),
            "live test prerequisite is missing: {name}; no live result can be claimed"
        );
    }
    let (application, arguments, application_class) = [
        ("ghostty", "--class=com.lightos.handoff.test --gtk-single-instance=false", "com.lightos.handoff.test"),
        ("foot", "--app-id=lightos-handoff-test", "lightos-handoff-test"),
        ("kitty", "--class=lightos-handoff-test", "lightos-handoff-test"),
        ("alacritty", "--class=lightos-handoff-test", "lightos-handoff-test"),
    ].into_iter().find_map(|(name, arguments, class)| platform_detect::find_executable(Path::new(name)).map(|path| (path, arguments, class)))
        .expect("live UI verification requires an installed supported terminal (foot, kitty, ghostty or alacritty); no test was run");
    let runtime = tempfile::Builder::new()
        .prefix("lrd.live.")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir()
        .unwrap();
    let compat = std::env::var_os("LIGHTOS_TEST_AQUAMARINE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            fs::read_dir("/usr/local/lib/lightos-remote-desktop/compat")
                .ok()?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .find(|p| p.join("libaquamarine.so.14").is_file())
        })
        .expect("provide LIGHTOS_TEST_AQUAMARINE_DIR");
    let binary = env!("CARGO_BIN_EXE_lightos-remoted");
    let owner_log = runtime.path().join("owner.log");
    let log = fs::File::create(&owner_log).unwrap();
    let mut owner = Process(
        Command::new(binary)
            .args([
                "desktop-session",
                "--password",
                "handoff-live-test-secret",
                "--desktop-shell",
                "omarchy",
                "--audio",
                "false",
                "--runtime-dir",
            ])
            .arg(runtime.path())
            .arg("--aquamarine-dir")
            .arg(&compat)
            .env_remove("LIGHTOS_REMOTE_DESKTOP_SESSION_RUNTIME")
            .env("XDG_DATA_HOME", runtime.path().join("data"))
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(35);
    while !runtime.path().join("session.json").is_file() {
        assert!(
            owner.0.try_wait().unwrap().is_none() && Instant::now() < until,
            "owner startup failed: {}",
            fs::read_to_string(&owner_log).unwrap()
        );
        thread::sleep(Duration::from_millis(100));
    }
    let descriptor: Value =
        serde_json::from_slice(&fs::read(runtime.path().join("session.json")).unwrap()).unwrap();
    let signature = descriptor["signature"].as_str().unwrap();
    assert_eq!(ipc(runtime.path(), signature, "configerrors").trim(), "");
    let application_command = format!(
        "'{}' {} -e sh -c 'sleep 600' > '{}' 2>&1",
        application.to_string_lossy().replace('\'', "'\\''"),
        arguments,
        runtime.path().join("app.log").display()
    );
    assert!(
        ipc(
            runtime.path(),
            signature,
            &format!(
                "dispatch hl.dsp.exec_cmd({})",
                serde_json::to_string(&application_command).unwrap()
            )
        )
        .starts_with("ok")
    );
    let client_pid = || -> Option<u64> {
        let clients: Value =
            serde_json::from_str(&ipc(runtime.path(), signature, "j/clients")).unwrap();
        clients
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["class"].as_str() == Some(application_class))
            .and_then(|c| c["pid"].as_u64())
    };
    let until = Instant::now() + Duration::from_secs(8);
    let app = loop {
        if let Some(pid) = client_pid() {
            break pid;
        }
        assert!(
            Instant::now() < until,
            "GUI test window did not map: {} ; desktop: {}",
            fs::read_to_string(runtime.path().join("app.log")).unwrap_or_default(),
            fs::read_to_string(runtime.path().join("desktop.log"))
                .unwrap_or_default()
                .chars()
                .rev()
                .take(2000)
                .collect::<String>()
        );
        thread::sleep(Duration::from_millis(100));
    };
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    for generation in 1..=2 {
        let mut transport = Process(
            Command::new(binary)
                .args([
                    "serve",
                    "--password",
                    "handoff-live-test-secret",
                    "--listen",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                    "--session-runtime",
                ])
                .arg(runtime.path())
                .args(["--audio", "false"])
                .arg("--aquamarine-dir")
                .arg(&compat)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let until = Instant::now() + Duration::from_secs(15);
        while !health(port) {
            assert!(
                transport.0.try_wait().unwrap().is_none() && Instant::now() < until,
                "transport failed to attach"
            );
            thread::sleep(Duration::from_millis(100));
        }
        resize_through_websocket(port, 800 + generation * 80, 600 + generation * 40, None);
        let (width, height, scale) = if generation == 1 {
            (780, 1476, 2.0)
        } else {
            (970, 1840, 2.5)
        };
        resize_through_websocket(port, width, height, Some(scale));
        let monitors: Value =
            serde_json::from_str(&ipc(runtime.path(), signature, "j/monitors")).unwrap();
        assert_eq!(monitors[0]["width"], width);
        assert_eq!(monitors[0]["height"], height);
        assert_eq!(monitors[0]["scale"], scale);
        assert_eq!(client_pid(), Some(app));
        drop(transport);
        assert!(owner.0.try_wait().unwrap().is_none());
        assert_eq!(client_pid(), Some(app));
        println!(
            "transport generation {generation} stopped; native desktop signature {signature} and GUI PID {app} survived"
        );
    }
    assert!(ipc(runtime.path(), signature, "reload").starts_with("ok"));
    assert_eq!(ipc(runtime.path(), signature, "configerrors").trim(), "");
    assert_eq!(client_pid(), Some(app));
    let bindings: Value = serde_json::from_str(&ipc(runtime.path(), signature, "j/binds")).unwrap();
    assert!(
        bindings.as_array().unwrap().len() > 30,
        "native bindings were not loaded"
    );
    println!(
        "{} native bindings active, no configuration errors",
        bindings.as_array().unwrap().len()
    );
}
