use std::time::{Duration, Instant};

#[tokio::test]
#[ignore = "requires a dedicated candidate URL, credentials and its XDG_RUNTIME_DIR/HYPRLAND_INSTANCE_SIGNATURE; switches its workspace"]
async fn video_credit_does_not_hold_keyboard_input() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    };
    let base = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_URL").unwrap();
    let rest = base.strip_prefix("http://").unwrap();
    let (address, path) = rest.split_once('/').unwrap();
    let user =
        std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_USER").unwrap_or_else(|_| "lightos".into());
    let password = std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD").unwrap();
    let mut request = format!("ws://{address}/{path}ws")
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
    let observer = || {
        let output = std::process::Command::new("hyprctl")
            .args(["-j", "activeworkspace"])
            .output()
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    assert_eq!(observer(), 1, "candidate must start on workspace 1");
    let (mut socket, _) = connect_async(request).await.unwrap();
    let ready = socket.next().await.unwrap().unwrap();
    assert!(matches!(ready, Message::Text(text) if text.contains("\"frame_ack\":true")));
    socket
        .send(Message::Text(
            "{\"type\":\"video_flow\",\"enabled\":true}".into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(socket.next().await.unwrap().unwrap(), Message::Binary(bytes) if bytes.first() == Some(&1)) {}
    }).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    for (code, pressed) in [(125, true), (3, true), (3, false), (125, false)] {
        socket
            .send(Message::Text(
                serde_json::json!({"type":"key","code":code,"pressed":pressed})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    }
    while observer() != 2 {
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "input blocked behind unacknowledged video"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    println!(
        "workspace input applied with video credit withheld: {:?}",
        started.elapsed()
    );
    socket
        .send(Message::Text("{\"type\":\"frame_ack\"}".into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(socket.next().await.unwrap().unwrap(), Message::Binary(bytes) if bytes.first() == Some(&1)) {}
    }).await.unwrap();
    socket.close(None).await.unwrap();
    assert!(
        std::process::Command::new("hyprctl")
            .args(["dispatch", "workspace", "1"])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
#[ignore = "set LIGHTOS_REMOTE_DESKTOP_BENCHMARK_IMAGE to a representative desktop JPEG"]
fn jpeg_encoder_benchmark() {
    let path = std::env::var("LIGHTOS_REMOTE_DESKTOP_BENCHMARK_IMAGE").unwrap();
    let rgb = image::load_from_memory(&std::fs::read(path).unwrap())
        .unwrap()
        .to_rgb8();
    let mut times = Vec::new();
    let mut size = 0;
    for _ in 0..30 {
        let started = Instant::now();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 75)
            .encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        times.push(started.elapsed());
        size = jpeg.len();
    }
    times.sort();
    println!(
        "legacy JPEG {}x{}: p50={:?}, p95={:?}, bytes={size}",
        rgb.width(),
        rgb.height(),
        times[15],
        times[28]
    );
    assert!(times[28] < Duration::from_secs(10));
    let mut encoder = turbojpeg::Compressor::new().unwrap();
    encoder.set_quality(75).unwrap();
    encoder.set_subsamp(turbojpeg::Subsamp::Sub2x2).unwrap();
    times.clear();
    for _ in 0..30 {
        let started = Instant::now();
        let jpeg = encoder
            .compress_to_vec(turbojpeg::Image {
                pixels: rgb.as_raw().as_slice(),
                width: rgb.width() as usize,
                pitch: rgb.width() as usize * 3,
                height: rgb.height() as usize,
                format: turbojpeg::PixelFormat::RGB,
            })
            .unwrap();
        times.push(started.elapsed());
        size = jpeg.len();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (rgb.width(), rgb.height())
        );
    }
    times.sort();
    println!(
        "SIMD JPEG {}x{}: p50={:?}, p95={:?}, bytes={size}",
        rgb.width(),
        rgb.height(),
        times[15],
        times[28]
    );
}
