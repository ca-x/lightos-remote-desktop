//! Opt-in native check: starts only its own compositor and transport.
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, Method};
use serde_json::{Value, json};

struct Candidate(Child);
impl Drop for Candidate {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.0.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if self.0.try_wait().unwrap().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

async fn rpc_result(response: reqwest::Response) -> Value {
    assert_eq!(response.headers()["cache-control"], "no-store");
    let text = response.text().await.unwrap();
    let json = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .next_back()
        .unwrap_or(&text);
    serde_json::from_str(json).unwrap()
}

#[tokio::test]
#[ignore = "starts a dedicated virtual desktop; requires Hyprland/cage and private Aquamarine; never touches installed services"]
async fn native_mcp_uses_real_capture_input_and_authenticated_prefixed_management() {
    let directory = tempfile::Builder::new()
        .prefix("lrd.mcp.")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let runtime = directory.path().join("rt");
    let settings = directory.path().join("mcp.json");
    let compat = std::env::var_os("LIGHTOS_TEST_AQUAMARINE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            fs::read_dir("/usr/local/lib/lightos-remote-desktop/compat")
                .ok()?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| path.join("libaquamarine.so.14").is_file())
        })
        .expect("provide LIGHTOS_TEST_AQUAMARINE_DIR");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let log = fs::File::create(directory.path().join("candidate.log")).unwrap();
    let mut candidate = Candidate(
        Command::new(env!("CARGO_BIN_EXE_lightos-remote-desktop"))
            .args([
                "serve",
                "--listen",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--width",
                "800",
                "--height",
                "600",
                "--desktop-shell",
                "minimal",
                "--audio",
                "false",
                "--subfolder",
                "/test/desktop",
            ])
            .arg("--runtime-dir")
            .arg(&runtime)
            .arg("--mcp-settings")
            .arg(&settings)
            .arg("--aquamarine-dir")
            .arg(compat)
            .env(
                "LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD",
                "isolated-mcp-native-test",
            )
            .env_remove("LIGHTOS_REMOTE_DESKTOP_SESSION_RUNTIME")
            .env_remove("LIGHTOS_REMOTE_DESKTOP_WAYLAND_SOCKET")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap(),
    );
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();
    let origin = format!("http://127.0.0.1:{port}");
    let base = format!("{origin}/test/desktop");
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if client
            .get(format!("{origin}/healthz"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            break;
        }
        assert!(
            candidate.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "isolated candidate failed: {}",
            fs::read_to_string(directory.path().join("candidate.log")).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let manage = |method, path: &str| {
        client
            .request(method, format!("{base}/{path}"))
            .basic_auth("lightos", Some("isolated-mcp-native-test"))
            .header("Origin", &origin)
            .header("X-LightOS-MCP", "1")
    };
    assert_eq!(
        client
            .get(format!("{base}/api/mcp"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        manage(Method::GET, "api/mcp")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        manage(Method::PUT, "api/mcp")
            .header("Origin", "https://foreign.example")
            .json(&json!({"enabled":true}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let created: Value = manage(Method::POST, "api/mcp/tokens")
        .json(&json!({"name":"Native smoke test","scope":"control"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = created["token"].as_str().unwrap();
    assert!(!fs::read_to_string(&settings).unwrap().contains(token));
    assert_eq!(
        manage(Method::PUT, "api/mcp")
            .json(&json!({"enabled":true}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let init=client.post(format!("{base}/mcp")).bearer_auth(token)
        .header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2026-07-28")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"Native modern client","version":"1"}}})).send().await.unwrap();
    assert_eq!(init.status(), 200);
    let session_id = init.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        rpc_result(init).await["result"]["protocolVersion"],
        "2025-11-25"
    );
    assert_eq!(
        client
            .post(format!("{base}/mcp"))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Mcp-Session-Id", &session_id)
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    let rpc = |name: &str, args: Value| {
        client.post(format!("{base}/mcp")).bearer_auth(token).header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-11-25").header("Mcp-Session-Id",&session_id).json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}))
    };
    let response: Value = rpc_result(rpc("screenshot", json!({})).send().await.unwrap()).await;
    assert_eq!(response["result"]["isError"], false, "{response}");
    let jpeg = STANDARD
        .decode(response["result"]["content"][1]["data"].as_str().unwrap())
        .unwrap();
    let image = image::load_from_memory(&jpeg).unwrap();
    let metadata: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        (image.width() as u64, image.height() as u64),
        (
            metadata["width"].as_u64().unwrap(),
            metadata["height"].as_u64().unwrap()
        )
    );
    assert!(image.width() >= 320 && image.height() >= 200);
    let moved: Value = rpc_result(
        rpc("mouse_move", json!({"x":200,"y":150}))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(moved["result"]["isError"], false, "{moved}");
    let signature = fs::read_dir(runtime.join("hypr"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.path().join(".socket.sock").is_file()
                || entry.path().join(".socket.sock").exists()
        })
        .unwrap();
    let mut socket = UnixStream::connect(signature.path().join(".socket.sock")).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    socket.write_all(b"j/cursorpos").unwrap();
    let mut output = String::new();
    socket.read_to_string(&mut output).unwrap();
    let cursor: Value = serde_json::from_str(&output).unwrap();
    assert!(
        (cursor["x"].as_i64().unwrap() - 200).abs() <= 2
            && (cursor["y"].as_i64().unwrap() - 150).abs() <= 2,
        "{cursor}"
    );
    let typed: Value = rpc_result(
        rpc("type_text", json!({"text":"中文 native MCP\n"}))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(typed["result"]["isError"], false, "{typed}");
    assert_eq!(
        manage(
            Method::DELETE,
            &format!("api/mcp/tokens/{}", created["info"]["id"].as_str().unwrap())
        )
        .send()
        .await
        .unwrap()
        .status(),
        204
    );
    assert_eq!(
        rpc("screenshot", json!({})).send().await.unwrap().status(),
        401
    );
    assert!(
        client
            .get(format!("{origin}/healthz"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    println!(
        "isolated MCP: native JPEG/dimensions, pointer read-back, Unicode input, prefixed Basic/Origin management and revoke verified"
    );
}
