use media_codec::{EncoderBackend, EncoderSettings, HardwareEncoders};

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pinned_requests_fail_explicitly_in_the_real_worker() {
    use futures_util::{SinkExt, StreamExt};
    use media_codec::worker_protocol::{self as wire, EncoderDevice, Reply, RequestRef};
    use std::{path::PathBuf, process::Stdio, time::Duration};
    use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

    let missing = PathBuf::from("/dev/dri/lightos-missing-render-node");
    assert!(!missing.exists());
    let settings = EncoderSettings {
        width: 640,
        height: 360,
        fps: 30,
        quality: 75,
    };
    for request in [
        RequestRef::ProbeDevice {
            device: EncoderDevice::Vaapi(missing.clone()),
        },
        RequestRef::InitDevice {
            device: EncoderDevice::Vaapi(missing),
            settings,
        },
        RequestRef::ProbeDevice {
            device: EncoderDevice::Nvenc(u32::MAX),
        },
        RequestRef::InitDevice {
            device: EncoderDevice::Nvenc(u32::MAX),
            settings,
        },
    ] {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_lightos-codec-ffmpeg"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut input = FramedWrite::new(child.stdin.take().unwrap(), LengthDelimitedCodec::new());
        let mut output = FramedRead::new(child.stdout.take().unwrap(), LengthDelimitedCodec::new());
        input
            .send(bytes::Bytes::from(wire::pack(&request).unwrap()))
            .await
            .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), output.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Reply::Error(reason) = wire::unpack(&response).unwrap() else {
            panic!("invalid pinned device was silently replaced")
        };
        assert!(
            reason.contains("lightos-missing-render-node") || reason.contains("LIGHTOS_NVENC_GPU"),
            "{reason}"
        );
        child.start_kill().ok();
        tokio::time::timeout(Duration::from_secs(1), child.wait())
            .await
            .unwrap()
            .unwrap();
    }
}

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
