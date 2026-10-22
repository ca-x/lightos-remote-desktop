use std::time::{Duration, Instant};

#[tokio::test]
#[ignore = "requires a dedicated candidate URL with continuously changing desktop content"]
async fn video_throughput_with_delayed_render_ack() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use futures_util::{SinkExt, StreamExt};
    use std::collections::VecDeque;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    };

    let base = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_URL").unwrap();
    let rest = base.strip_prefix("http://").unwrap();
    let (address, path) = rest.split_once('/').unwrap();
    let password = std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD").unwrap();
    let user =
        std::env::var("LIGHTOS_REMOTE_DESKTOP_AUTH_USER").unwrap_or_else(|_| "lightos".into());
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
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket.next().await.unwrap().unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"video_flow","enabled":true,"sequenced":true})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let use_h264 = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_CODEC").as_deref() == Ok("h264");
    if use_h264 {
        socket
            .send(Message::Text(
                "{\"type\":\"video\",\"codec\":\"h264\"}".into(),
            ))
            .await
            .unwrap();
    }
    let delay = Duration::from_millis(
        std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_ACK_MS")
            .unwrap_or_else(|_| "120".into())
            .parse()
            .unwrap(),
    );
    let started = tokio::time::Instant::now();
    let finish = started + Duration::from_secs(4);
    let mut pending = VecDeque::new();
    let mut frame_id = None;
    let mut count = 0;
    let mut h264_packets = Vec::new();
    let mut keyframes = 0;
    loop {
        let next_ack = pending
            .front()
            .map(|(deadline, _)| *deadline)
            .unwrap_or(finish);
        tokio::select! {
            _ = tokio::time::sleep_until(finish) => break,
            _ = tokio::time::sleep_until(next_ack), if !pending.is_empty() => {
                let (_, id) = pending.pop_front().unwrap();
                socket.send(Message::Text(serde_json::json!({"type":"frame_ack","id":id}).to_string().into())).await.unwrap();
            }
            message = socket.next() => match message.unwrap().unwrap() {
                Message::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "frame_id" { frame_id = value["id"].as_u64(); }
                }
                Message::Binary(bytes) if matches!(bytes.first(), Some(1 | 3)) => {
                    count += 1;
                    if bytes[0] == 3 {
                        keyframes += usize::from(bytes[1] != 0);
                        h264_packets.extend_from_slice(&bytes[10..]);
                    }
                    if let Some(id) = frame_id.take() { pending.push_back((tokio::time::Instant::now() + delay, id)); }
                }
                _ => {}
            }
        }
    }
    socket.close(None).await.unwrap();
    let fps = count as f64 / started.elapsed().as_secs_f64();
    println!(
        "render ACK delay={delay:?}: frames={count}, fps={fps:.2}, keyframes={keyframes}, h264_bytes={}",
        h264_packets.len()
    );
    if use_h264 {
        assert!(!h264_packets.is_empty(), "H.264 silently fell back to JPEG");
    }
    if !h264_packets.is_empty() {
        use std::io::Write;
        let mut stream = tempfile::NamedTempFile::new().unwrap();
        stream.write_all(&h264_packets).unwrap();
        assert!(
            std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-xerror", "-f", "h264", "-i"])
                .arg(stream.path())
                .args(["-f", "null", "-"])
                .status()
                .unwrap()
                .success()
        );
    }
    assert!(
        fps >= 15.0,
        "video stalled behind delayed render ACK: {fps:.2} FPS"
    );
}

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

#[tokio::test]
#[ignore = "requires representative desktop JPEG and ffmpeg as test decoder; compares simulated small desktop updates"]
async fn desktop_video_codec_bandwidth_comparison() {
    use lightos_remote_desktop::video::H264Encoder;
    let path = std::env::var("LIGHTOS_REMOTE_DESKTOP_BENCHMARK_IMAGE").unwrap();
    let mut rgba = image::load_from_memory(&std::fs::read(path).unwrap())
        .unwrap()
        .to_rgba8();
    let mut h264 = H264Encoder::new(rgba.width(), rgba.height(), 30).unwrap();
    let mut jpeg = turbojpeg::Compressor::new().unwrap();
    jpeg.set_quality(75).unwrap();
    jpeg.set_subsamp(turbojpeg::Subsamp::Sub2x2).unwrap();
    let mut stream = Vec::new();
    let mut jpeg_bytes = 0;
    for index in 0..30 {
        for row in 100..120 {
            for column in 100..180 {
                rgba.put_pixel(column, row, image::Rgba([index * 8, 150, 60, 255]));
            }
        }
        stream.extend(h264.encode(rgba.as_raw()).await.unwrap().data);
        jpeg_bytes += jpeg
            .compress_to_vec(turbojpeg::Image {
                pixels: rgba.as_raw().as_slice(),
                width: rgba.width() as usize,
                height: rgba.height() as usize,
                pitch: rgba.width() as usize * 4,
                format: turbojpeg::PixelFormat::RGBA,
            })
            .unwrap()
            .len();
    }
    println!(
        "30 simulated desktop updates: JPEG={jpeg_bytes} bytes, H.264={} bytes, ratio={:.2}",
        stream.len(),
        jpeg_bytes as f64 / stream.len() as f64
    );
    let mut decoder = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "h264",
            "-i",
            "pipe:0",
            "-f",
            "null",
            "-",
            "-progress",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        decoder.stdin.take().unwrap().write_all(&stream).unwrap();
    }
    let decoded = decoder.wait_with_output().unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert!(String::from_utf8_lossy(&decoded.stdout).contains("frame=30"));
}
