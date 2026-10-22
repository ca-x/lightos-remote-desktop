//! Explicit synthetic link test. It ACKs complete packets, not browser render.
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    time::{Duration, Instant},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

fn phase(seconds: f64) -> usize {
    match seconds {
        t if t < 4.0 => 0,
        t if t < 16.0 => 1,
        t if t < 24.0 => 2,
        t if t < 28.0 => 3,
        _ => 4,
    }
}
fn percentile(values: &mut [f64], percent: usize) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    (!values.is_empty()).then(|| values[(values.len() * percent).div_ceil(100) - 1])
}

#[tokio::test]
#[ignore = "requires scripts/benchmark-adaptive.py and an isolated moving desktop"]
async fn adaptive_stream_over_a_shaped_link() {
    let address = std::env::var("LIGHTOS_ADAPTIVE_ADDRESS").unwrap();
    let result_path = std::env::var("LIGHTOS_ADAPTIVE_RESULT").unwrap();
    let strategy = std::env::var("LIGHTOS_ADAPTIVE_STRATEGY").unwrap_or_else(|_| "auto".into());
    let mut request = format!("ws://{address}/ws").into_client_request().unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!(
            "Basic {}",
            STANDARD.encode("lightos:adaptive-benchmark-test")
        )
        .parse()
        .unwrap(),
    );
    request
        .headers_mut()
        .insert("Origin", format!("http://{address}").parse().unwrap());
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(request))
        .await
        .unwrap()
        .unwrap();
    let Message::Text(ready) = socket.next().await.unwrap().unwrap() else {
        panic!("missing readiness")
    };
    let ready: Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["width"], 1280);
    assert_eq!(ready["height"], 720);
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
            "{\"type\":\"video\",\"codec\":\"h264\"}".into(),
        ))
        .await
        .unwrap();
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut pending = HashMap::new();
    let mut ping_id = 0_u64;
    let mut frame_id = None;
    let mut frames = [0_u64; 5];
    let mut bytes = [0_u64; 5];
    let mut frames_per_second = [0_u64; 36];
    let mut delays: [Vec<f64>; 5] = Default::default();
    let mut metrics = Vec::new();
    let mut access_units = Vec::new();
    let mut h264 = false;
    let mut closed = false;
    let mut keyframes = 0;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(start+Duration::from_secs(36))) => break,
            _ = tick.tick() => {
                ping_id += 1;
                pending.insert(ping_id,Instant::now());
                if socket.send(Message::Text(json!({"type":"ping","id":ping_id}).to_string().into())).await.is_err() { closed=true;break; }
            }
            message = socket.next() => {
                let Some(Ok(message)) = message else { closed=true;break; };
                match message {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        match value["type"].as_str() {
                            Some("error") => panic!("server error: {value}"),
                            Some("frame_id") => frame_id = value["id"].as_u64(),
                            Some("video_config") => {
                                let codec=value["codec"].as_str().unwrap();
                                assert!(codec.starts_with("avc1."),"silent codec fallback: {value}");
                                h264=true;
                            }
                            Some("pong") => {
                                let t=start.elapsed().as_secs_f64();
                                if let Some(sent)=value["id"].as_u64().and_then(|id|pending.remove(&id)) {
                                    delays[phase(t)].push(sent.elapsed().as_secs_f64()*1000.0);
                                }
                                metrics.push(json!({"at_s":t,"metrics":value["video_metrics"]}));
                            }
                            _ => {}
                        }
                    }
                    Message::Binary(packet) => {
                        let t=start.elapsed().as_secs_f64();
                        if packet.first()==Some(&3) {
                            assert!(h264 && packet.len()>10);
                            frames[phase(t)] += 1;
                            frames_per_second[(t as usize).min(35)] += 1;
                            bytes[phase(t)] += packet.len() as u64;
                            keyframes += u32::from(packet[1]==1);
                            access_units.extend_from_slice(&packet[10..]);
                        } else if h264 && packet.first()==Some(&1) { panic!("unexpected JPEG fallback"); }
                        if let Some(id)=frame_id.take() {
                            socket.send(Message::Text(json!({"type":"frame_ack","id":id}).to_string().into())).await.unwrap();
                        }
                    }
                    Message::Ping(data) => { socket.send(Message::Pong(data)).await.unwrap(); }
                    Message::Close(_) => { closed=true;break; }
                    _ => {}
                }
            }
        }
    }
    let durations = [4.0, 12.0, 8.0, 4.0, 8.0];
    let names = [
        "clean",
        "constrained",
        "recovery",
        "interruption",
        "restored",
    ];
    let phases: Vec<_>=(0..5).map(|i| json!({"phase":names[i],"frames":frames[i],"fps":frames[i] as f64/durations[i],"bytes":bytes[i],"mbps":bytes[i] as f64*8.0/durations[i]/1e6,"ping_p50_ms":percentile(&mut delays[i],50),"ping_p95_ms":percentile(&mut delays[i],95)})).collect();
    let result = json!({"version":ready["version"],"strategy":strategy,"closed":closed,"elapsed_s":start.elapsed().as_secs_f64(),"keyframes":keyframes,"phases":phases,"frames_per_second":frames_per_second.as_slice(),
        "settled_clean_fps":frames_per_second[2..4].iter().sum::<u64>() as f64/2.0,
        "settled_recovery_fps":frames_per_second[22..24].iter().sum::<u64>() as f64/2.0,
        "settled_restored_fps":frames_per_second[33..36].iter().sum::<u64>() as f64/3.0,"samples":metrics});
    std::fs::write(&result_path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!(
        "{result_path}: {}",
        json!({"closed":closed,"phases":result["phases"]})
    );
    let mut stream = tempfile::NamedTempFile::new().unwrap();
    stream.write_all(&access_units).unwrap();
    assert!(platform_detect::find_executable(std::path::Path::new("ffmpeg")).is_some());
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-xerror", "-f", "h264", "-i"])
            .arg(stream.path())
            .args(["-f", "null", "-"])
            .status()
            .unwrap()
            .success(),
        "continuous stream must decode"
    );
    assert!(!closed, "transport closed during a transient impairment");
    assert!(frames[4] >= 20, "stream did not recover");
    let _ = socket.close(None).await;
}
