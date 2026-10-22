use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

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
    virtual_socket.close(None).await.unwrap();
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
    physical_socket.close(None).await.unwrap();
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
        socket.close(None).await.unwrap();
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
    socket.close(None).await.unwrap();
    let mut socket = connect("?desktop=virtual".into()).await;
    let Message::Text(ready) = socket.next().await.unwrap().unwrap() else {
        panic!("missing virtual ready")
    };
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["desktop"], "virtual");
    assert_eq!(ready["resize"], true);
    assert_eq!(ready["width"], virtual_ready["width"]);
    assert_eq!(ready["height"], virtual_ready["height"]);
    socket.close(None).await.unwrap();
}
