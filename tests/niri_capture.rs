#![cfg(target_os = "linux")]

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lightos_remote_desktop::wayland::{Backend, capabilities};

struct Candidate(Child);

impl Drop for Candidate {
    fn drop(&mut self) {
        let group = -i32::try_from(self.0.id()).unwrap();
        unsafe { libc::kill(group, libc::SIGTERM) };
        std::thread::sleep(Duration::from_millis(100));
        unsafe { libc::kill(group, libc::SIGKILL) };
        let _ = self.0.wait();
    }
}

fn niri_socket(runtime: &Path) -> Option<PathBuf> {
    fs::read_dir(runtime).ok()?.filter_map(Result::ok).find_map(|entry| {
        let path = entry.path();
        if !entry.file_name().to_str()?.starts_with("wayland-")
            || !entry.file_type().ok()?.is_socket()
        {
            return None;
        }
        let socket = UnixStream::connect(&path).ok()?;
        let mut credentials = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut size = std::mem::size_of_val(&credentials).try_into().ok()?;
        let status = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut size,
            )
        };
        if status != 0 || credentials.uid != unsafe { libc::geteuid() } {
            return None;
        }
        let executable = fs::read_link(format!("/proc/{}/exe", credentials.pid)).ok()?;
        (executable.file_name()? == "niri").then_some(path)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated non-root GitHub Niri/Cage software-rendered candidate"]
async fn niri_native_capabilities_and_first_frame() {
    assert_ne!(unsafe { libc::geteuid() }, 0, "probe must run as a fixture user");
    let report = PathBuf::from(std::env::var_os("LRD_NIRI_PROBE_REPORT").unwrap());
    fs::create_dir_all(&report).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("lrd-niri.")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let runtime = directory.path();
    let home = runtime.join("home");
    fs::create_dir(&home).unwrap();
    let config = runtime.join("config.kdl");
    fs::write(&config, "").unwrap();
    let log = report.join("candidate.log");
    let output = fs::File::create(&log).unwrap();
    let _candidate = Candidate(
        Command::new("/usr/bin/dbus-run-session")
            .args(["--", "/usr/bin/cage", "--", "/usr/bin/niri", "--config"])
            .arg(&config)
            .args(["--", "/usr/bin/wev"])
            .env_clear()
            .env("PATH", "/usr/bin")
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", runtime)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "niri")
            .env("WLR_BACKENDS", "headless")
            .env("WLR_HEADLESS_OUTPUTS", "1")
            .env("WLR_RENDERER", "pixman")
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env("GALLIUM_DRIVER", "llvmpipe")
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let socket = loop {
        if let Some(socket) = niri_socket(runtime) {
            break socket;
        }
        assert!(
            Instant::now() < deadline,
            "Niri did not expose its own socket: {}",
            fs::read_to_string(&log).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let protocols = capabilities::probe(&socket, Duration::from_secs(5), None).unwrap();
    protocols.require_selected_output().unwrap();
    let backend = Backend::start_with_output(socket.clone(), 5, 75, Some("winit".into()))
        .unwrap_or_else(|error| {
            panic!(
                "native Niri capture failed: {error:#}; {}",
                fs::read_to_string(&log).unwrap_or_default()
            )
        });
    let mut frames = backend.frames.clone();
    let frame = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let latest = frames.borrow().clone();
            if let Some(frame) = latest
                && frame.pixels.len() >= 4
                && frame.pixels.chunks_exact(4).any(|pixel| pixel[..3] != frame.pixels[..3])
            {
                break frame;
            }
            frames.changed().await.unwrap();
        }
    })
    .await
    .expect("Niri did not produce a nonuniform frame containing its client");
    assert!(frame.width >= 320 && frame.height >= 200);
    assert!(backend.frame_is_fresh(Duration::from_secs(5)));
    image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels.to_vec())
        .unwrap()
        .save(report.join("niri-first-frame.png"))
        .unwrap();
    fs::write(
        report.join("capture.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "socket": socket,
            "protocols": protocols,
            "width": frame.width,
            "height": frame.height,
            "input_delivery_tested": false,
            "pond_shell_tested": false,
            "platform_proxy_tested": false,
            "physical_vt_tested": false,
        }))
        .unwrap(),
    )
    .unwrap();
    backend.stop();
}
