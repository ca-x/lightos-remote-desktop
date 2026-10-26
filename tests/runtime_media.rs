#![cfg(target_os = "linux")]
//! Explicit wire check against an isolated candidate with the test-only password below.
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

#[tokio::test]
#[ignore = "requires an isolated 127.0.0.1 candidate with LIGHTOS_RUNTIME_TEST_PORT and the runtime-candidate-check test password"]
async fn real_encoders_and_scheduling_choices_negotiate_over_websocket() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let port = std::env::var("LIGHTOS_RUNTIME_TEST_PORT")
        .expect("explicit isolated candidate port required")
        .parse::<u16>()
        .unwrap();
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!(
            "Basic {}",
            STANDARD.encode("lightos:runtime-candidate-check")
        )
        .parse()
        .unwrap(),
    );
    request.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{port}").parse().unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
    let mut last_id = None;
    let mut video_packets = 0_u64;
    macro_rules! receive_until {
        ($predicate:expr) => {{
            tokio::time::timeout(Duration::from_secs(15),async {
                loop {
                    let message=socket.next().await.unwrap().unwrap();
                    if let Message::Text(text)=&message {
                        let value:Value=serde_json::from_str(text).unwrap();
                        assert_ne!(value["type"],"error","{value}");
                        if value["type"]=="frame_id" {last_id=value["id"].as_u64();}
                    }
                    if matches!(&message,Message::Binary(bytes) if matches!(bytes.first(),Some(1 | 3))) {video_packets += 1;}
                    if matches!(message,Message::Binary(_)) && let Some(id)=last_id.take() {
                        socket.send(Message::Text(json!({"type":"frame_ack","id":id}).to_string().into())).await.unwrap();
                    }
                    if ($predicate)(&message) {break message;}
                }
            }).await.expect("candidate response deadline")
        }};
    }
    let ready = receive_until!(
        |m: &Message| matches!(m,Message::Text(text) if serde_json::from_str::<Value>(text).unwrap()["type"]=="ready")
    );
    let Message::Text(ready) = ready else {
        unreachable!()
    };
    let ready: Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(ready["resize_scale"], true);
    assert!(
        ready["codec_capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["hardware"] == false)
            .all(|c| c["available"] == true)
    );
    for (index, strategy) in ["auto", "low_latency", "smooth", "quality_first"]
        .into_iter()
        .enumerate()
    {
        let before = video_packets;
        socket
            .send(Message::Text(
                json!({"type":"video_flow","enabled":true,"sequenced":true,"strategy":strategy})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                json!({"type":"ping","id":index+1}).to_string().into(),
            ))
            .await
            .unwrap();
        let pong = receive_until!(
            |m: &Message| matches!(m,Message::Text(text) if serde_json::from_str::<Value>(text).unwrap()["type"]=="pong")
        );
        let Message::Text(pong) = pong else {
            unreachable!()
        };
        let pong: Value = serde_json::from_str(&pong).unwrap();
        assert_eq!(pong["video_metrics"]["strategy"], strategy);
        if video_packets == before {
            receive_until!(
                |m: &Message| matches!(m,Message::Binary(bytes) if matches!(bytes.first(),Some(1 | 3)))
            );
        }
        assert!(
            video_packets > before,
            "selecting a strategy must refresh a static frame"
        );
    }
    socket
        .send(Message::Text(
            json!({"type":"video","codec":"h264"}).to_string().into(),
        ))
        .await
        .unwrap();
    let config = receive_until!(
        |m: &Message| matches!(m,Message::Text(text) if serde_json::from_str::<Value>(text).unwrap()["codec"].as_str().is_some_and(|c|c.starts_with("avc1")))
    );
    let h264 =
        receive_until!(|m: &Message| matches!(m,Message::Binary(bytes) if bytes.first()==Some(&3)));
    let Message::Binary(h264) = h264 else {
        unreachable!()
    };
    assert_eq!(h264[1], 1);
    assert!(
        ready["codec_capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["codec"] == "h264" && c["dynamic_bitrate"] == true)
    );
    socket
        .send(Message::Text(
            json!({"type":"ping","id":99}).to_string().into(),
        ))
        .await
        .unwrap();
    let actual = receive_until!(
        |m: &Message| matches!(m,Message::Text(text) if serde_json::from_str::<Value>(text).unwrap()["type"]=="pong")
    );
    let Message::Text(actual) = actual else {
        unreachable!()
    };
    let actual: Value = serde_json::from_str(&actual).unwrap();
    assert_eq!(actual["video_metrics"]["strategy"], "quality_first");
    assert_eq!(
        actual["video_metrics"]["qos"]["target"]["quality_percent"],
        100
    );
    assert!(
        actual["video_metrics"]["encoder_rate"]["bitrate_bps"]
            .as_u64()
            .unwrap()
            > 0
    );
    for backend in ["software", "nvenc", "vaapi", "amf", "videotoolbox"] {
        socket
            .send(Message::Text(
                json!({"type":"video","codec":"h264","backend":backend})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let config = receive_until!(
            |m: &Message| matches!(m, Message::Text(text) if serde_json::from_str::<Value>(text).unwrap()["codec"].as_str().is_some_and(|c| c.starts_with("avc1.")))
        );
        let Message::Text(config) = config else {
            unreachable!()
        };
        let config: Value = serde_json::from_str(&config).unwrap();
        assert!(
            config["backend"].is_string(),
            "actual encoder must be observable: {config}"
        );
        if backend == "software" {
            assert_eq!(config["backend"], "openh264");
        }
        let packet = receive_until!(
            |m: &Message| matches!(m,Message::Binary(bytes) if bytes.first()==Some(&3))
        );
        let Message::Binary(packet) = packet else {
            unreachable!()
        };
        assert_eq!(packet[1], 1, "backend changes must start with IDR");
    }
    // The active software profile is explicitly rejected. The exhausted
    // connection must move to JPEG rather than retrying OpenH264 forever.
    socket.send(Message::Text(json!({"type":"video","codec":"h264","backend":"software","rejected_backend":"openh264"}).to_string().into())).await.unwrap();
    let fallback = receive_until!(
        |m: &Message| matches!(m,Message::Text(text) if serde_json::from_str::<Value>(text).unwrap()["codec"]=="jpeg")
    );
    let Message::Text(fallback) = fallback else {
        unreachable!()
    };
    let fallback: Value = serde_json::from_str(&fallback).unwrap();
    assert_eq!(fallback["backend"], "turbojpeg");
    assert!(fallback["fallback_reason"].is_string());
    socket
        .send(Message::Text(
            json!({"type":"video","codec":"jpeg"}).to_string().into(),
        ))
        .await
        .unwrap();
    receive_until!(
        |m: &Message| matches!(m,Message::Text(text) if serde_json::from_str::<Value>(text).unwrap()["codec"]=="jpeg")
    );
    let jpeg =
        receive_until!(|m: &Message| matches!(m,Message::Binary(bytes) if bytes.first()==Some(&1)));
    let Message::Binary(jpeg) = jpeg else {
        unreachable!()
    };
    let frame = image::load_from_memory(&jpeg[1..]).unwrap();
    assert_eq!(frame.width(), ready["width"].as_u64().unwrap() as u32);
    assert_eq!(frame.height(), ready["height"].as_u64().unwrap() as u32);
    eprintln!(
        "real H.264 configuration {config:?}; JPEG decoded; Auto/LowLatency/Smooth metrics confirmed"
    );
    socket.close(None).await.unwrap();
}
