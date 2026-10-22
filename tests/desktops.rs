use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

async fn close_cleanly<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) {
    socket
        .close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "Switch desktop".into(),
        }))
        .await
        .unwrap();
    // TCP EOF alone is insufficient: the gateway otherwise keeps the browser
    // in CLOSING for 60 seconds. Require the protocol acknowledgement itself.
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match socket
                .next()
                .await
                .expect("EOF before close acknowledgement")
                .expect("unclean WebSocket close")
            {
                Message::Close(Some(frame)) => {
                    assert_eq!(frame.code, CloseCode::Normal);
                    return;
                }
                Message::Close(None) => panic!("close code was not echoed"),
                _ => {}
            }
        }
    })
    .await
    .expect("desktop close acknowledgement exceeded one second");
}

#[tokio::test]
#[ignore = "requires LIGHTOS_REMOTE_DESKTOP_TEST_WAYLAND_SOCKET pointing to an isolated candidate"]
async fn abandoned_initialization_acknowledges_close_and_releases_controller() {
    use clap::Parser;
    use lightos_remote_desktop::{
        config::Config,
        server::{AppState, router},
        session::Session,
        wayland::Backend,
    };
    use std::{io::Read, os::unix::net::UnixListener};

    let wayland = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_WAYLAND_SOCKET").unwrap();
    let mut config = Config::try_parse_from([
        "test",
        "--password",
        "isolated-test-password",
        "--wayland-socket",
        &wayland,
    ])
    .unwrap();
    let session = Session::start(&config).unwrap();
    let backend = Backend::start(session.socket.clone(), 30, 80).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let stalled_socket = directory.path().join("wayland-stalled");
    let stalled = UnixListener::bind(&stalled_socket).unwrap();
    let (accepted, pending) = tokio::sync::oneshot::channel();
    let compositor = std::thread::spawn(move || {
        let mut client = stalled.accept().unwrap().0;
        accepted.send(()).unwrap();
        client.read_to_end(&mut Vec::new()).unwrap();
    });
    config.wayland_socket = None;
    config.subfolder.clear();
    config.physical_wayland_socket = Some(stalled_socket);
    let state = AppState::new(config, backend, session);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, stop) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = stop.await;
            })
            .await
            .unwrap();
    });
    let request = |mode| {
        let mut request = format!("ws://{address}/ws?desktop={mode}")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Origin", format!("http://{address}").parse().unwrap());
        request.headers_mut().insert(
            "Authorization",
            format!(
                "Basic {}",
                STANDARD.encode("lightos:isolated-test-password")
            )
            .parse()
            .unwrap(),
        );
        request
    };
    let (mut socket, _) = connect_async(request("physical")).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap();
    let denied = connect_async(request("virtual")).await.unwrap_err();
    assert!(
        matches!(denied, tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==409)
    );
    close_cleanly(&mut socket).await;
    let mut replacement = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match connect_async(request("virtual")).await {
                Ok((socket, _)) => break socket,
                Err(tokio_tungstenite::tungstenite::Error::Http(response))
                    if response.status() == 409 =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("controller recovery failed: {error}"),
            }
        }
    })
    .await
    .expect("abandoned startup retained the controller");
    let ready = tokio::time::timeout(Duration::from_secs(1), replacement.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(ready, Message::Text(text) if text.contains("\"desktop\":\"virtual\"")));
    close_cleanly(&mut replacement).await;
    shutdown.send(()).unwrap();
    server.await.unwrap();
    compositor.join().unwrap();
}

#[tokio::test]
#[ignore = "requires a dedicated candidate configured with an existing compositor as physical target; optionally observes its cursor"]
async fn selects_existing_monitors_and_returns_to_intact_virtual_desktop() {
    let base = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_URL").unwrap();
    let rest = base.strip_prefix("http://").unwrap();
    let (address, path) = rest.split_once('/').unwrap();
    let user =
        std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_USER").unwrap_or_else(|_| "lightos".into());
    let password = std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD").unwrap();
    let request = |query: &str| {
        let mut request = format!("ws://{address}/{path}ws{query}")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "Authorization",
            format!("Basic {}", STANDARD.encode(format!("{user}:{password}")))
                .parse()
                .unwrap(),
        );
        request
            .headers_mut()
            .insert("Origin", format!("http://{address}").parse().unwrap());
        request
    };
    let connect = |query: String| {
        let request = request(&query);
        async move {
            for attempt in 0..50 {
                match connect_async(request.clone()).await {
                    Ok((socket, _)) => return socket,
                    Err(tokio_tungstenite::tungstenite::Error::Http(response))
                        if response.status() == 409 && attempt < 49 =>
                    {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(error) => panic!("desktop connection failed: {error}"),
                }
            }
            unreachable!()
        }
    };
    let mut virtual_socket = connect(String::new()).await;
    let Message::Text(ready) = virtual_socket.next().await.unwrap().unwrap() else {
        panic!("missing ready")
    };
    let virtual_ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(virtual_ready["desktop"], "virtual");
    close_cleanly(&mut virtual_socket).await;
    let mut physical_socket = connect("?desktop=physical".into()).await;
    let Message::Text(ready) =
        tokio::time::timeout(Duration::from_secs(15), physical_socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    else {
        panic!("missing physical ready")
    };
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["desktop"], "physical", "{ready}");
    assert_eq!(ready["resize"], false);
    let outputs = ready["outputs"].as_array().unwrap().clone();
    assert!(!outputs.is_empty());
    close_cleanly(&mut physical_socket).await;
    for output in outputs {
        let name = output["name"].as_str().unwrap();
        assert!(
            name.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            )
        );
        let mut socket = connect(format!("?desktop=physical&output={name}")).await;
        let Message::Text(ready) = tokio::time::timeout(Duration::from_secs(15), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        else {
            panic!("missing monitor ready")
        };
        let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
        assert_eq!(ready["output"], name);
        assert_eq!(ready["width"], output["width"]);
        assert_eq!(ready["height"], output["height"]);
        socket
            .send(Message::Text(
                "{\"type\":\"resize\",\"width\":640,\"height\":480}".into(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "error" {
                        assert!(value["message"].as_str().unwrap().contains("owned desktop"));
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        socket
            .send(Message::Text(
                "{\"type\":\"move\",\"x\":0.5,\"y\":0.5}".into(),
            ))
            .await
            .unwrap();
        if std::env::var_os("LIGHTOS_REMOTE_DESKTOP_OBSERVE_CURSOR").is_some() {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let cursor = std::process::Command::new("hyprctl")
                .args(["-j", "cursorpos"])
                .output()
                .unwrap();
            let cursor: serde_json::Value = serde_json::from_slice(&cursor.stdout).unwrap();
            let scale = output["scale"].as_f64().unwrap();
            let expected_x =
                output["x"].as_f64().unwrap() + output["width"].as_f64().unwrap() / scale / 2.0;
            let expected_y =
                output["y"].as_f64().unwrap() + output["height"].as_f64().unwrap() / scale / 2.0;
            assert!(
                (cursor["x"].as_f64().unwrap() - expected_x).abs() <= 2.0,
                "{cursor}, expected x={expected_x}"
            );
            assert!(
                (cursor["y"].as_f64().unwrap() - expected_y).abs() <= 2.0,
                "{cursor}, expected y={expected_y}"
            );
        }
        close_cleanly(&mut socket).await;
        println!(
            "verified monitor {name}: {}x{} at {},{}",
            output["width"], output["height"], output["x"], output["y"]
        );
    }
    let mut socket = connect("?desktop=physical&output=not-a-real-monitor".into()).await;
    let Message::Text(error) = socket.next().await.unwrap().unwrap() else {
        panic!("missing monitor error")
    };
    let error: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(error["type"], "error");
    // The server initiated this error close; consume its close frame.
    let _ = socket.close(None).await;
    let mut socket = connect("?desktop=virtual".into()).await;
    let Message::Text(ready) = socket.next().await.unwrap().unwrap() else {
        panic!("missing virtual ready")
    };
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["desktop"], "virtual");
    assert_eq!(ready["resize"], true);
    assert_eq!(ready["width"], virtual_ready["width"]);
    assert_eq!(ready["height"], virtual_ready["height"]);
    close_cleanly(&mut socket).await;
}
