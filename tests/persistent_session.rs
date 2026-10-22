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

#[test]
#[ignore = "requires local GPU, Cage, Omarchy, foot and the installed private Aquamarine library"]
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
        ("foot", "--app-id=lightos-handoff-test", "lightos-handoff-test"),
        ("kitty", "--class=lightos-handoff-test", "lightos-handoff-test"),
        ("ghostty", "--class=com.lightos.handoff.test --gtk-single-instance=false", "com.lightos.handoff.test"),
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
    let binary = env!("CARGO_BIN_EXE_lightos-remote-desktop");
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
