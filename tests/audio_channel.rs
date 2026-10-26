#![cfg(target_os = "linux")]
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

#[tokio::test]
#[ignore = "requires an isolated candidate URL and a working private PulseAudio monitor"]
async fn independent_audio_survives_video_backpressure_and_follows_controller_lifetime() {
    let base = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_URL").unwrap();
    let rest = base.strip_prefix("http://").unwrap();
    let (address, path) = rest.split_once('/').unwrap();
    let password = std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD").unwrap();
    let user =
        std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_USER").unwrap_or_else(|_| "lightos".into());
    let authorization = STANDARD.encode(format!("{user}:{password}"));
    let request = |endpoint: &str, origin: &str, authenticated: bool| {
        let mut request = format!("ws://{address}/{path}{endpoint}")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        if authenticated {
            request.headers_mut().insert(
                "Authorization",
                format!("Basic {authorization}").parse().unwrap(),
            );
        }
        request
    };
    let origin = format!("http://{address}");
    let (mut video, _) = connect_async(request("ws", &origin, true)).await.unwrap();
    let ready = timeout(Duration::from_secs(5), video.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(ready) = ready else {
        panic!("missing desktop ready")
    };
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    let session = ready["audio_session"]
        .as_u64()
        .expect("independent audio capability");
    let opus = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_AUDIO_CODEC").as_deref() == Ok("opus");
    if opus {
        assert!(
            ready["audio_codecs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|codec| codec == "opus")
        );
    }
    let endpoint = format!(
        "audio?session={session}{}",
        if opus { "&codec=opus" } else { "" }
    );
    let mut decoder = opus::Decoder::new(48_000, opus::Channels::Stereo).unwrap();
    let mut decoded = [0.0; 1920];
    let mut signal = false;
    for (authenticated, request_origin, status) in [
        (false, origin.as_str(), 401),
        (true, "https://foreign.invalid", 403),
    ] {
        let rejected = connect_async(request(&endpoint, request_origin, authenticated))
            .await
            .unwrap_err();
        assert!(
            matches!(rejected, tokio_tungstenite::tungstenite::Error::Http(response) if response.status().as_u16() == status)
        );
    }
    video
        .send(Message::Text(
            "{\"type\":\"video_flow\",\"enabled\":true,\"sequenced\":true}".into(),
        ))
        .await
        .unwrap();
    let (mut audio, _) = connect_async(request(&endpoint, &origin, true))
        .await
        .unwrap();
    let duplicate = connect_async(request(&endpoint, &origin, true))
        .await
        .unwrap_err();
    assert!(
        matches!(duplicate, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 409)
    );
    let packets = timeout(Duration::from_secs(5), async {
        let mut count = 0;
        while count < 20 {
            match audio.next().await.unwrap().unwrap() {
                Message::Binary(bytes) => {
                    if opus {
                        assert_eq!(bytes[0], 4);
                        assert_eq!(bytes.len(), 329);
                        assert_eq!(
                            decoder
                                .decode_float(&bytes[9..], &mut decoded, false)
                                .unwrap(),
                            960
                        );
                        signal |= decoded.iter().any(|sample| sample.abs() > 0.001);
                    } else {
                        assert_eq!(bytes[0], 2);
                        assert_eq!(bytes.len(), 3841);
                    }
                    count += 1;
                }
                Message::Ping(bytes) => audio.send(Message::Pong(bytes)).await.unwrap(),
                Message::Text(text) => assert!(text.contains("audio_ready"), "{text}"),
                message => panic!("unexpected audio message: {message:?}"),
            }
        }
        count
    })
    .await
    .expect("audio stalled behind unacknowledged video");
    if opus && std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_AUDIO_SIGNAL").as_deref() == Ok("1") {
        assert!(signal, "Opus stream contained no nonzero decoded signal");
    }
    println!(
        "Received {packets} independent {} packets while video render ACKs were withheld",
        if opus { "Opus" } else { "PCM" }
    );
    audio.close(None).await.unwrap();
    video
        .send(Message::Text("{\"type\":\"ping\",\"id\":42}".into()))
        .await
        .unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            match video.next().await.unwrap().unwrap() {
                Message::Text(text) if text.contains("\"type\":\"pong\"") => break,
                Message::Binary(bytes) => {
                    assert_ne!(bytes[0], 2, "PCM leaked into video transport")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("closing audio disconnected the video controller");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (mut replacement, _) = connect_async(request(&endpoint, &origin, true))
        .await
        .unwrap();
    video.close(None).await.unwrap();
    timeout(Duration::from_secs(3), async {
        while let Some(Ok(message)) = replacement.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    })
    .await
    .expect("audio survived its controller");
    let stale = connect_async(request(&endpoint, &origin, true))
        .await
        .unwrap_err();
    assert!(
        matches!(stale, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 409)
    );
}
