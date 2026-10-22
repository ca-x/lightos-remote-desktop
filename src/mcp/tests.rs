use std::os::unix::fs::PermissionsExt;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

use super::{Scope, TokenStore, management, tools::Desktop, transport_with_desktop};
use crate::{
    server::credentials_match,
    wayland::{Frame, InputCommand},
};

struct Harness {
    _directory: tempfile::TempDir,
    frames: watch::Sender<Option<Arc<Frame>>>,
    input: Arc<Mutex<Vec<InputCommand>>>,
    controller: Arc<AtomicBool>,
    client: Client,
    url: String,
    server: JoinHandle<()>,
    worker: JoinHandle<()>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
        self.worker.abort();
    }
}
impl Harness {
    async fn new(owned: bool) -> Self {
        Self::with_release_hold(owned, None).await
    }
    async fn with_release_hold(owned: bool, hold: Option<std::sync::mpsc::Receiver<()>>) -> Self {
        let hold = Mutex::new(hold);
        let directory = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let store = Arc::new(TokenStore::open(Some(directory.path().join("mcp.json"))));
        let (frames, receiver) = watch::channel(Some(Arc::new(Frame::test_frame(64, 48))));
        let (input, mut commands) = mpsc::channel::<InputCommand>(128);
        let observed = Arc::new(Mutex::new(Vec::new()));
        let observed_worker = observed.clone();
        let worker = tokio::spawn(async move {
            while let Some(command) = commands.recv().await {
                match command {
                    InputCommand::ReleaseSync { done } => {
                        observed_worker.lock().unwrap().push(InputCommand::Release);
                        let _ = done.send(());
                    }
                    command => observed_worker.lock().unwrap().push(command),
                }
            }
        });
        let cleanup_input = input.clone();
        let controller = Arc::new(AtomicBool::new(false));
        let desktop = Desktop {
            frames: receiver,
            input,
            controller: controller.clone(),
            owned,
            release_input: Arc::new(move || {
                if let Some(receiver) = hold.lock().unwrap().take() {
                    receiver.recv_timeout(Duration::from_secs(8)).unwrap();
                }
                let (done, completed) = std::sync::mpsc::sync_channel(1);
                cleanup_input
                    .blocking_send(InputCommand::ReleaseSync { done })
                    .unwrap();
                completed.recv_timeout(Duration::from_secs(1)).unwrap();
                true
            }),
        };
        async fn basic(request: Request<Body>, next: Next) -> Response {
            if !credentials_match(request.headers(), "lightos", "test-mcp-long-password") {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            next.run(request).await
        }
        let router = Router::new().nest(
            "/desktop",
            management(store.clone())
                .layer(middleware::from_fn(basic))
                .merge(transport_with_desktop(store.clone(), desktop)),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/desktop/", listener.local_addr().unwrap());
        let server = tokio::spawn(async { axum::serve(listener, router).await.unwrap() });
        Self {
            _directory: directory,
            frames,
            input: observed,
            controller,
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap(),
            url,
            server,
            worker,
        }
    }
    async fn manage(&self, method: Method, path: &str, body: Option<Value>) -> reqwest::Response {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.url, path))
            .basic_auth("lightos", Some("test-mcp-long-password"))
            .header("Origin", self.url.trim_end_matches("/desktop/"))
            .header("X-LightOS-MCP", "1");
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().await.unwrap()
    }
    async fn create(&self, scope: Scope) -> (String, String) {
        let response = self
            .manage(
                Method::POST,
                "api/mcp/tokens",
                Some(json!({"name":"Test agent","scope":scope})),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let result: Value = response.json().await.unwrap();
        (
            result["token"].as_str().unwrap().to_owned(),
            result["info"]["id"].as_str().unwrap().to_owned(),
        )
    }
    async fn enable(&self) {
        assert_eq!(
            self.manage(Method::PUT, "api/mcp", Some(json!({"enabled":true})))
                .await
                .status(),
            StatusCode::OK
        );
    }
    fn rpc_request(&self, bearer: &str, value: Value) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}mcp", self.url))
            .bearer_auth(bearer)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-06-18")
            .json(&value)
    }
    async fn rpc(&self, bearer: &str, method: &str, params: Value) -> Value {
        let response = self
            .rpc_request(
                bearer,
                json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key("mcp-session-id"));
        let text = response.text().await.unwrap();
        let json_text = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .next_back()
            .unwrap_or(&text);
        serde_json::from_str(json_text).unwrap()
    }
    async fn call(&self, bearer: &str, name: &str, arguments: Value) -> Value {
        self.rpc(
            bearer,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        )
        .await
    }
}

#[tokio::test]
async fn actual_http_initialize_list_capture_and_control_reuse_existing_channels() {
    let h = Harness::new(true).await;
    let status: Value = h
        .manage(Method::GET, "api/mcp", None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(status, json!({"enabled":false,"tokens":[]}));
    let (token, _) = h.create(Scope::Control).await;
    h.enable().await;
    let initialized = h.rpc(&token, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"HTTP contract test","version":"1"}})).await;
    assert_eq!(
        initialized["result"]["serverInfo"]["name"],
        "lightos-remote-desktop"
    );
    assert!(initialized["result"]["capabilities"]["tools"].is_object());
    let notification = h
        .rpc_request(
            &token,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(notification.status(), StatusCode::ACCEPTED);
    let listed = h.rpc(&token, "tools/list", json!({})).await;
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 8);
    assert!(names.contains(&"screenshot") && names.contains(&"type_text"));
    assert!(
        !names
            .iter()
            .any(|name| name.contains("exec") || name.contains("shell"))
    );
    let info = h.call(&token, "desktop_info", json!({})).await;
    let details: Value =
        serde_json::from_str(info["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(details["width"], 64);
    assert_eq!(details["coordinate_space"], "screenshot_pixels");
    let screenshot = h.call(&token, "screenshot", json!({})).await;
    let image = &screenshot["result"]["content"][1];
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/jpeg");
    let bytes = STANDARD.decode(image["data"].as_str().unwrap()).unwrap();
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (64, 48));
    for (name, args) in [
        (
            "mouse_click",
            json!({"x":63,"y":47,"button":"right","count":2}),
        ),
        ("mouse_move", json!({"x":31,"y":23})),
        ("key_press", json!({"codes":[29,46]})),
        ("scroll", json!({"x":-12,"y":30})),
        ("type_text", json!({"text":"native 中文\n"})),
        (
            "mouse_drag",
            json!({"from_x":0,"from_y":0,"to_x":63,"to_y":47,"duration_ms":50}),
        ),
    ] {
        let result = h.call(&token, name, args).await;
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert!(!h.controller.load(Ordering::Acquire));
    }
    let commands = h.input.lock().unwrap();
    assert!(matches!(commands[0], InputCommand::Move { x: 1.0, y: 1.0 }));
    assert!(matches!(
        commands[1],
        InputCommand::Button {
            button: 2,
            pressed: true
        }
    ));
    assert!(
        commands
            .iter()
            .any(|command| matches!(command,InputCommand::Text{text} if text=="native 中文\n"))
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, InputCommand::Release))
            .count(),
        6
    );
    let key_events: Vec<_> = commands
        .iter()
        .filter_map(|c| {
            if let InputCommand::Key { code, pressed } = c {
                Some((*code, *pressed))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        key_events,
        [(29, true), (46, true), (46, false), (29, false)]
    );
}

#[tokio::test]
async fn http_auth_off_revoke_and_management_origin_cannot_be_bypassed() {
    let h = Harness::new(true).await;
    let (token, id) = h.create(Scope::Control).await;
    let rpc = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
    assert_eq!(
        h.rpc_request(&token, rpc.clone())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    h.enable().await;
    for auth in [
        None,
        Some("Bearer bad".to_owned()),
        Some(format!(
            "Basic {}",
            STANDARD.encode("lightos:test-mcp-long-password")
        )),
    ] {
        let mut request = h.client.post(format!("{}mcp", h.url)).json(&rpc);
        if let Some(auth) = auth {
            request = request.header("Authorization", auth);
        }
        assert_eq!(
            request.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        h.rpc_request(&token, rpc.clone())
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.rpc_request(&token, rpc.clone())
            .header("sec-fetch-site", "cross-site")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.rpc_request(&token, rpc.clone())
            .header("Origin", h.url.trim_end_matches("/desktop/"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        h.client
            .get(format!("{}api/mcp", h.url))
            .header("Origin", h.url.trim_end_matches("/desktop/"))
            .header("X-LightOS-MCP", "1")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for origin in [None, Some("https://evil.example")] {
        let mut request = h
            .client
            .post(format!("{}api/mcp/tokens", h.url))
            .basic_auth("lightos", Some("test-mcp-long-password"))
            .header("X-LightOS-MCP", "1")
            .json(&json!({"name":"evil","scope":"control"}));
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }
    let duplicated_origin = h
        .client
        .put(format!("{}api/mcp", h.url))
        .basic_auth("lightos", Some("test-mcp-long-password"))
        .header("Origin", h.url.trim_end_matches("/desktop/"))
        .header("Origin", "https://evil.example")
        .header("X-LightOS-MCP", "1")
        .json(&json!({"enabled":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicated_origin.status(), StatusCode::FORBIDDEN);
    let fetch = h
        .client
        .get(format!("{}api/mcp", h.url))
        .basic_auth("lightos", Some("test-mcp-long-password"))
        .header("sec-fetch-site", "same-origin")
        .header("X-LightOS-MCP", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(fetch.status(), StatusCode::OK);
    let saved = fetch.text().await.unwrap();
    assert!(!saved.contains(&token) && !saved.contains("hash"));
    assert_eq!(
        h.manage(Method::DELETE, &format!("api/mcp/tokens/{id}"), None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.rpc_request(&token, rpc)
            .header("mcp-session-id", "old-session")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.manage(Method::DELETE, &format!("api/mcp/tokens/{id}"), None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn scope_physical_and_controller_boundaries_reject_input_but_allow_capture() {
    for owned in [true, false] {
        let h = Harness::new(owned).await;
        let (observe, _) = h.create(Scope::Observe).await;
        let (control, _) = h.create(Scope::Control).await;
        h.enable().await;
        let denied = h.call(&observe, "mouse_click", json!({"x":0,"y":0})).await;
        assert_eq!(denied["result"]["isError"], true);
        if owned {
            h.controller.store(true, Ordering::Release);
        }
        let denied = h.call(&control, "mouse_move", json!({"x":0,"y":0})).await;
        assert_eq!(denied["result"]["isError"], true);
        assert!(h.input.lock().unwrap().is_empty());
        let screenshot = h.call(&observe, "screenshot", json!({})).await;
        assert_eq!(screenshot["result"]["isError"], false);
    }
}

#[tokio::test]
async fn invalid_inputs_and_stale_frames_never_acquire_control_or_emit_input() {
    let h = Harness::new(true).await;
    let (token, _) = h.create(Scope::Control).await;
    h.enable().await;
    for (name, args) in [
        ("mouse_move", json!({"x":64,"y":0})),
        ("mouse_move", json!({"x":-1,"y":0})),
        ("mouse_move", json!({"x":1,"y":2,"exec":"no"})),
        ("mouse_click", json!({"x":0,"y":0,"count":3})),
        ("mouse_click", json!({"x":0,"y":0,"button":"bad"})),
        (
            "mouse_drag",
            json!({"from_x":0,"from_y":0,"to_x":63,"to_y":47,"duration_ms":1001}),
        ),
        ("key_press", json!({"codes":[]})),
        ("key_press", json!({"codes":[29,29]})),
        ("key_press", json!({"codes":[768]})),
        ("scroll", json!({"x":0,"y":1001})),
        ("type_text", json!({"text":"\u{0}"})),
        ("type_text", json!({"text":"x".repeat(16*1024+1)})),
    ] {
        let result = h.call(&token, name, args).await;
        assert!(
            result["error"].is_object() || result["result"]["isError"] == true,
            "{result}"
        );
        assert!(!h.controller.load(Ordering::Acquire));
    }
    assert!(h.input.lock().unwrap().is_empty());
    let mut frame = Frame::test_frame(64, 48);
    frame.captured_at = Instant::now() - Duration::from_secs(5);
    h.frames.send_replace(Some(Arc::new(frame)));
    let result = h.call(&token, "screenshot", json!({})).await;
    assert_eq!(result["result"]["isError"], true);
    let oversized=h.rpc_request(&token,json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"type_text","arguments":{"text":"x".repeat(65*1024)}}})).send().await.unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn revoke_and_disable_cancel_inflight_drag_and_release_controller() {
    for revoke in [true, false] {
        let h = Harness::new(true).await;
        let (token, id) = h.create(Scope::Control).await;
        h.enable().await;
        let request=h.rpc_request(&token,json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mouse_drag","arguments":{"from_x":0,"from_y":0,"to_x":63,"to_y":47,"duration_ms":1000}}}));
        let action =
            tokio::spawn(
                async move { request.send().await.unwrap().json::<Value>().await.unwrap() },
            );
        tokio::time::timeout(Duration::from_secs(2), async {
            while !h
                .input
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, InputCommand::Button { pressed: true, .. }))
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let response = if revoke {
            h.manage(Method::DELETE, &format!("api/mcp/tokens/{id}"), None)
                .await
        } else {
            h.manage(Method::PUT, "api/mcp", Some(json!({"enabled":false})))
                .await
        };
        assert!(response.status().is_success());
        let result = tokio::time::timeout(Duration::from_secs(1), action)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["result"]["isError"], true);
        tokio::time::timeout(Duration::from_secs(1), async {
            while h.controller.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            h.input.lock().unwrap().last(),
            Some(InputCommand::Release)
        ));
        let length = h.input.lock().unwrap().len();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(h.input.lock().unwrap().len(), length);
    }
}

#[tokio::test]
async fn screenshot_pixel_coordinates_follow_new_capture_dimensions() {
    let h = Harness::new(true).await;
    let (token, _) = h.create(Scope::Control).await;
    h.enable().await;
    h.frames
        .send_replace(Some(Arc::new(Frame::test_frame(128, 96))));
    let result = h.call(&token, "mouse_move", json!({"x":127,"y":95})).await;
    assert_eq!(result["result"]["isError"], false);
    assert!(matches!(
        h.input.lock().unwrap()[0],
        InputCommand::Move { x: 1.0, y: 1.0 }
    ));
    let image = h.call(&token, "screenshot", json!({})).await;
    let metadata: Value =
        serde_json::from_str(image["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        (metadata["width"].as_u64(), metadata["height"].as_u64()),
        (Some(128), Some(96))
    );
}

#[tokio::test]
async fn disconnected_http_call_releases_held_input() {
    let h = Harness::new(true).await;
    let (token, _) = h.create(Scope::Control).await;
    h.enable().await;
    let request=h.rpc_request(&token,json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mouse_drag","arguments":{"from_x":0,"from_y":0,"to_x":63,"to_y":47,"duration_ms":1000}}})).timeout(Duration::from_millis(100));
    let action = tokio::spawn(async move { request.send().await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !h
            .input
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, InputCommand::Button { pressed: true, .. }))
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(action.await.unwrap().unwrap_err().is_timeout());
    tokio::time::timeout(Duration::from_millis(400), async {
        while h.controller.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("disconnect must cancel held input before the full drag completes");
    assert!(matches!(
        h.input.lock().unwrap().last(),
        Some(InputCommand::Release)
    ));
}

#[tokio::test]
async fn tool_deadline_does_not_release_the_gate_before_worker_cleanup() {
    let (release, held) = std::sync::mpsc::channel();
    let h = Harness::with_release_hold(true, Some(held)).await;
    let (token, _) = h.create(Scope::Control).await;
    h.enable().await;
    let started = Instant::now();
    let result = h.call(&token, "mouse_move", json!({"x":1,"y":2})).await;
    assert_eq!(result["result"]["isError"], true);
    assert!(
        started.elapsed() >= Duration::from_secs(5) && started.elapsed() < Duration::from_secs(7)
    );
    assert!(h.controller.load(Ordering::Acquire));
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while h.controller.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        h.input.lock().unwrap().last(),
        Some(InputCommand::Release)
    ));
}
