use media_codec::{EncoderBackend, EncoderSettings, HardwareEncoders};

#[tokio::test]
async fn real_worker_only_advertises_hardware_after_bounded_encode_decode_probe() {
    let worker = HardwareEncoders::at(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_lightos-codec-ffmpeg"
    )));
    let started = std::time::Instant::now();
    let caps = worker.probe().await;
    assert_eq!(caps.len(), 4);
    assert!(started.elapsed() < std::time::Duration::from_secs(12));
    for cap in caps {
        eprintln!(
            "hardware probe {}: available={} reason={:?}",
            cap.backend, cap.available, cap.reason
        );
        assert!(cap.hardware);
        if cap.available {
            assert!(cap.reason.is_none());
            assert!(cap.codec_string.unwrap().starts_with("avc1."));
            assert!(cap.dynamic_bitrate);
            assert_eq!(cap.rate_update, "restart");
            let backend = EncoderBackend::HARDWARE
                .into_iter()
                .find(|b| b.id() == cap.backend)
                .unwrap();
            let mut encoder = worker
                .create(
                    backend,
                    EncoderSettings {
                        width: 640,
                        height: 360,
                        fps: 30,
                        quality: 75,
                    },
                )
                .unwrap();
            let first = encoder
                .encode(vec![128; 640 * 360 * 4].into())
                .await
                .unwrap();
            assert!(first.key && !first.data.is_empty());
        } else {
            assert!(cap.reason.is_some_and(|r| !r.is_empty()));
            assert!(cap.codec_string.is_none());
            assert!(!cap.dynamic_bitrate);
        }
    }
}
