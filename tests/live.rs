use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

async fn http(address: &str, path: &str, auth: Option<&str>) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let authorization = auth
        .map(|value| format!("Authorization: Basic {value}\r\n"))
        .unwrap_or_default();
    stream
        .write_all(
            format!(
                "GET {path} HTTP/1.0\r\nHost: {address}\r\n{authorization}Connection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    timeout(Duration::from_secs(3), stream.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    response
}

#[tokio::test]
#[ignore = "requires a separately started native desktop; sets that session's text clipboard"]
async fn browser_protocol_against_real_desktop() {
    let base = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_URL")
        .expect("set test HTTP base URL including trailing slash");
    let rest = base
        .strip_prefix("http://")
        .expect("test must use local HTTP");
    let (address, path) = rest.split_once('/').unwrap();
    let path = format!("/{path}");
    let user =
        std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_USER").unwrap_or_else(|_| "lightos".into());
    let password = std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD")
        .expect("set test password via environment");
    let authorization = STANDARD.encode(format!("{user}:{password}"));
    assert!(http(address, &path, None).await.starts_with("HTTP/1.0 401"));
    assert!(
        http(address, &path, Some(&authorization))
            .await
            .starts_with("HTTP/1.0 200")
    );
    let health = http(address, "/healthz", None).await;
    assert!(health.starts_with("HTTP/1.0 200") && health.contains("\"native\":true"));
    let url = format!("ws://{address}{path}ws");
    let request = |origin: &str| {
        let mut request = url.clone().into_client_request().unwrap();
        request.headers_mut().insert(
            "Authorization",
            format!("Basic {authorization}").parse().unwrap(),
        );
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        request
    };
    let rejected = connect_async(request("https://foreign.invalid"))
        .await
        .unwrap_err();
    assert!(
        matches!(rejected, tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==403)
    );
    let origin = format!("http://{address}");
    let (mut socket, _) = connect_async(request(&origin)).await.unwrap();
    let ready = timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(ready, Message::Text(text) if text.contains("\"type\":\"ready\"")));
    let denied = connect_async(request(&origin)).await.unwrap_err();
    assert!(
        matches!(denied, tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==409)
    );
    let frame = timeout(Duration::from_secs(10), async {
        loop {
            if let Message::Binary(frame) = socket.next().await.unwrap().unwrap()
                && frame.first() == Some(&1)
            {
                break frame;
            }
        }
    })
    .await
    .unwrap();
    let image = image::load_from_memory(&frame[1..]).unwrap();
    assert!(image.width() >= 320 && image.height() >= 200);
    let initial_size = (image.width(), image.height());
    socket
        .send(Message::Text("{\"type\":\"ping\",\"id\":42}".into()))
        .await
        .unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap()
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                && value["type"] == "pong"
            {
                assert_eq!(value["id"], 42);
                break;
            }
        }
    })
    .await
    .unwrap();
    socket
        .send(Message::Text(
            "{\"type\":\"key\",\"code\":0,\"pressed\":true}".into(),
        ))
        .await
        .unwrap();
    let error = timeout(Duration::from_secs(3), async {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap()
                && text.contains("\"type\":\"error\"")
            {
                break text;
            }
        }
    })
    .await
    .unwrap();
    assert!(!error.is_empty());
    socket
        .send(Message::Text(
            "{\"type\":\"clipboard_set\",\"text\":\"native 中文 clipboard\"}".into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text("{\"type\":\"clipboard_get\"}".into()))
        .await
        .unwrap();
    let clipboard = timeout(Duration::from_secs(5), async {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                if text.contains("\"type\":\"clipboard\"") {
                    break text;
                }
                assert!(!text.contains("\"type\":\"error\""), "{text}");
            }
        }
    })
    .await
    .unwrap();
    assert!(clipboard.contains("native 中文 clipboard"));
    socket
        .send(Message::Text(
            "{\"type\":\"audio\",\"enabled\":true}".into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            if let Message::Binary(bytes) = socket.next().await.unwrap().unwrap()
                && bytes.first() == Some(&2)
            {
                assert_eq!((bytes.len() - 1) % 8, 0);
                let samples = bytes[1..].as_chunks::<4>().0;
                assert!(
                    samples
                        .iter()
                        .all(|sample| f32::from_le_bytes(*sample).is_finite())
                );
                if std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_AUDIO_SIGNAL").as_deref() == Ok("1")
                    && !samples
                        .iter()
                        .any(|sample| f32::from_le_bytes(*sample).abs() > 0.001)
                {
                    continue;
                }
                break;
            }
        }
    })
    .await
    .expect("audio packet");
    socket
        .send(Message::Text(
            "{\"type\":\"resize\",\"width\":800,\"height\":600}".into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            if let Message::Binary(bytes) = socket.next().await.unwrap().unwrap()
                && bytes.first() == Some(&1)
            {
                let image = image::load_from_memory(&bytes[1..]).unwrap();
                if image.width() == 800 && image.height() == 600 {
                    break;
                }
            }
        }
    })
    .await
    .expect("resized real frame");
    socket
        .send(Message::Text(
            serde_json::json!({"type":"resize","width":initial_size.0,"height":initial_size.1})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            "{\"type\":\"clipboard_set\",\"text\":\"\"}".into(),
        ))
        .await
        .unwrap();
    socket.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (mut replacement, _) = connect_async(request(&origin))
        .await
        .expect("controller released on disconnect");
    replacement.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert!(
        http(address, "/healthz", None)
            .await
            .starts_with("HTTP/1.0 200")
    );
}
