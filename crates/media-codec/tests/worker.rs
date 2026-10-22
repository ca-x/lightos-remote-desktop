use media_codec::worker_protocol::{self as wire, Reply};
use media_codec::{EncoderBackend, EncoderSettings, HardwareEncoders, WorkerDeadlines};
use std::time::Duration;

#[test]
fn wire_rejects_incompatible_truncated_trailing_and_oversized_data() {
    let bytes = wire::pack(&Reply::Ready).unwrap();
    assert!(matches!(
        wire::unpack::<Reply>(&bytes).unwrap(),
        Reply::Ready
    ));
    assert!(wire::unpack::<Reply>(&bytes[..bytes.len() - 1]).is_err());
    let mut bad = bytes.clone();
    bad[0] = 99;
    assert!(wire::unpack::<Reply>(&bad).is_err());
    let mut bad = bytes;
    bad.push(0);
    assert!(wire::unpack::<Reply>(&bad).is_err());
    assert!(wire::unpack::<Reply>(&vec![0; wire::MAX_MESSAGE + 1]).is_err());
}
#[tokio::test]
async fn missing_worker_never_advertises_hardware() {
    let worker = HardwareEncoders::at(std::path::PathBuf::from(
        "/does-not-exist/lightos-codec-ffmpeg",
    ));
    let caps = worker.probe().await;
    assert_eq!(caps.len(), 4);
    assert!(caps.iter().all(|c| !c.available && c.reason.is_some()));
    assert!(worker.create(EncoderBackend::Vaapi, settings()).is_err());
}
fn settings() -> EncoderSettings {
    EncoderSettings {
        width: 640,
        height: 360,
        fps: 30,
        quality: 75,
    }
}
#[cfg(unix)]
#[tokio::test]
async fn invalid_and_hanging_workers_are_bounded_and_reaped() {
    use std::os::unix::fs::PermissionsExt;
    for body in ["printf 'invalid protocol'", "exec sleep 60"] {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("worker");
        let pids = dir.path().join("pids");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' $$ >> '{}'\n{body}\n",
                pids.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let worker = HardwareEncoders::at(script).with_deadlines(WorkerDeadlines {
            init: Duration::from_millis(120),
            probe: Duration::from_millis(120),
            encode: Duration::from_millis(120),
        });
        let started = std::time::Instant::now();
        let caps = worker.probe().await;
        assert!(caps.iter().all(|c| !c.available && c.reason.is_some()));
        assert!(started.elapsed() < Duration::from_secs(3));
        #[cfg(target_os = "linux")]
        for pid in std::fs::read_to_string(&pids).unwrap().lines() {
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "worker process {pid} was not reaped"
            );
        }
    }
}
