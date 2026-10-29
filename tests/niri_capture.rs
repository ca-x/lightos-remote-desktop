#![cfg(target_os = "linux")]

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lightos_remote_desktop::{
    backend::{DesktopBackend, InputCommand},
    wayland::{Backend, capabilities},
};
use tokio::io::AsyncWriteExt;

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
    fs::read_dir(runtime)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
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
            if executable.file_name()? != "niri" {
                return None;
            }
            let stat = fs::read_to_string(format!("/proc/{}/stat", credentials.pid)).ok()?;
            let origin: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().take(4).collect();
            println!("native Niri peer PID={} origin(state/parent/group/session)={origin:?}", credentials.pid);
            Some(path)
        })
}

fn wev_event<'a>(line: &'a str, interface: &str, event: &str) -> Option<&'a str> {
    let (header, payload) = line.strip_prefix('[')?.split_once("] ")?;
    let (object, class) = header.split_once(':')?;
    object.trim().parse::<u32>().ok()?;
    let (name, fields) = payload.split_once(": ")?;
    (class.trim() == interface && name == event).then_some(fields)
}

fn wev_number(fields: &str, name: &str) -> Option<u32> {
    fields.split([';', ',']).find_map(|field| {
        let (field_name, value) = field.trim().split_once(": ")?;
        if field_name != name {
            return None;
        }
        value.split_whitespace().next()?.parse().ok()
    })
}

fn wev_reports_input(log: &str, offset: usize) -> bool {
    let Some(observed) = log.get(offset..) else {
        return false;
    };
    let delivered = [
        ("wl_keyboard", "key", 38, 1),
        ("wl_keyboard", "key", 38, 0),
        ("wl_pointer", "button", 272, 1),
        ("wl_pointer", "button", 272, 0),
    ]
    .iter()
    .all(|(interface, event, code, state)| {
        observed.lines().any(|line| {
            wev_event(line, interface, event).is_some_and(|fields| {
                wev_number(fields, event) == Some(*code)
                    && wev_number(fields, "state") == Some(*state)
            })
        })
    });
    delivered
        && observed
            .lines()
            .any(|line| wev_event(line, "wl_pointer", "motion").is_some())
}

fn clipboard_client(socket: &Path, program: &str) -> tokio::process::Command {
    let runtime = socket.parent().unwrap();
    let mut command = tokio::process::Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/bin")
        .env("HOME", runtime.join("home"))
        .env("XDG_RUNTIME_DIR", runtime)
        .env("WAYLAND_DISPLAY", socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn await_clipboard_write(
    write: impl std::future::Future<Output = anyhow::Result<()>>,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(3), write)
        .await
        .map_err(|_| anyhow::anyhow!("initial Niri clipboard write timed out"))?
}

#[tokio::test]
async fn clipboard_probe_bounds_a_stalled_write() {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        await_clipboard_write(std::future::pending()),
    )
    .await
    .expect("clipboard probe writes must have their own deadline");
    assert!(result.is_err());
}

async fn verify_text_clipboard(backend: &Backend, socket: &Path) {
    let from_transport = "From remoted: 猫与 Pond 🦋\nsecond line\n";
    await_clipboard_write(backend.clipboard_write(from_transport))
        .await
        .unwrap();
    let mut paste = clipboard_client(socket, "/usr/bin/wl-paste");
    paste
        .args(["--no-newline", "--type", "text"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let observed = tokio::time::timeout(Duration::from_secs(3), paste.output())
        .await
        .expect("independent clipboard reader timed out")
        .unwrap();
    assert!(
        observed.status.success(),
        "independent clipboard reader failed: {}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert_eq!(observed.stdout, from_transport.as_bytes());

    let from_desktop = "From Niri client: 蝴蝶 🦋\nnew selection\n";
    let mut copy = clipboard_client(socket, "/usr/bin/wl-copy");
    let mut owner = copy
        .args(["--foreground", "--type", "text/plain;charset=utf-8"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = owner.stdin.take().unwrap();
    input.write_all(from_desktop.as_bytes()).await.unwrap();
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observed = tokio::time::timeout(Duration::from_secs(2), backend.clipboard_read()).await;
        if observed
            .as_ref()
            .is_ok_and(|result| result.as_ref().is_ok_and(|text| text == from_desktop))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "remoted did not read the independent clipboard selection: {observed:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    owner.start_kill().unwrap();
    tokio::time::timeout(Duration::from_secs(2), owner.wait())
        .await
        .expect("independent clipboard owner did not stop")
        .unwrap();
}

const WEV_INPUT: &str = "[        17:      wl_pointer] motion: time: 100; x, y: 100.0, 200.0\n\
[        17:      wl_pointer] button: serial: 4; time: 101; button: 272 (left), state: 1 (pressed)\n\
[        17:      wl_pointer] button: serial: 5; time: 102; button: 272 (left), state: 0 (released)\n\
[        18:     wl_keyboard] key: serial: 6; time: 103; key: 38; state: 1 (pressed)\n\
[        18:     wl_keyboard] key: serial: 7; time: 104; key: 38; state: 0 (released)\n";

#[test]
fn wev_receipt_accepts_xkb_keycode_for_evdev_30() {
    assert!(wev_reports_input(WEV_INPUT, 0));
}

#[test]
fn wev_receipt_rejects_numeric_prefix_collisions() {
    for invalid in [
        WEV_INPUT.replace("key: 38", "key: 300"),
        WEV_INPUT.replace("key: 38", "key: 380"),
        WEV_INPUT.replace("button: 272", "button: 2720"),
        WEV_INPUT
            .replace("key: 38", "key: 30")
            .replace("button: 272", "button: 2720"),
        WEV_INPUT.replace("state: 1", "state: 10"),
    ] {
        assert!(!wev_reports_input(&invalid, 0), "{invalid}");
    }
}

#[test]
fn wev_receipt_rejects_incomplete_or_stale_events() {
    for omitted in ["motion:", "state: 0 (released)", "state: 1 (pressed)"] {
        let incomplete = WEV_INPUT
            .lines()
            .filter(|line| !line.contains(omitted))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!wev_reports_input(&incomplete, 0));
    }
    assert!(!wev_reports_input(WEV_INPUT, WEV_INPUT.len()));
    assert!(!wev_reports_input(WEV_INPUT, WEV_INPUT.len() + 1));
    let old_then_new = format!("{WEV_INPUT}unrelated new line\n");
    assert!(!wev_reports_input(&old_then_new, WEV_INPUT.len()));
}

#[test]
fn wev_receipt_rejects_lookalike_interfaces_and_events() {
    for invalid in [
        WEV_INPUT.replace("wl_keyboard", "not_wl_keyboard"),
        WEV_INPUT.replace("] key:", "] not_key:"),
        WEV_INPUT.replace("] motion:", "] not_motion:"),
    ] {
        assert!(!wev_reports_input(&invalid, 0));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated non-root GitHub Niri/Cage software-rendered candidate"]
async fn niri_native_capabilities_and_first_frame() {
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "probe must run as a fixture user"
    );
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
    let wev_log = report.join("wev.log");
    let output = fs::File::create(&log).unwrap();
    let _candidate = Candidate(
        Command::new("/usr/bin/dbus-run-session")
            .args(["--", "/usr/bin/cage", "--", "/usr/bin/niri", "--config"])
            .arg(&config)
            .args([
                "--",
                "/usr/bin/sh",
                "-c",
                "exec /usr/bin/stdbuf -oL /usr/bin/wev >\"$1\" 2>&1",
                "wev-log",
            ])
            .arg(&wev_log)
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
                && frame
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[..3] != frame.pixels[..3])
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
    backend
        .input
        .send(InputCommand::Move { x: 0.25, y: 0.5 })
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let offset = loop {
        let observed = fs::read_to_string(&wev_log).unwrap_or_default();
        if observed
            .lines()
            .any(|line| wev_event(line, "wl_keyboard", "enter").is_some())
            && observed
                .lines()
                .any(|line| wev_event(line, "wl_pointer", "enter").is_some())
        {
            break observed.len();
        }
        assert!(
            Instant::now() < deadline,
            "wev did not acquire keyboard and pointer focus: {observed}; {}",
            fs::read_to_string(&log).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    for command in [
        InputCommand::Move { x: 0.35, y: 0.5 },
        InputCommand::Button {
            button: 0,
            pressed: true,
        },
        InputCommand::Button {
            button: 0,
            pressed: false,
        },
        InputCommand::Key {
            code: 30,
            pressed: true,
        },
        InputCommand::Key {
            code: 30,
            pressed: false,
        },
    ] {
        backend.input.send(command).await.unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let observed = fs::read_to_string(&wev_log).unwrap();
        if wev_reports_input(&observed, offset) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "wev did not receive the injected pointer and key transitions: {}",
            observed.get(offset..).unwrap_or("wev log was truncated")
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    verify_text_clipboard(&backend, &socket).await;
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
            "input_delivery_tested": true,
            "keyboard_delivery_tested": true,
            "pointer_delivery_tested": true,
            "clipboard_text_delivery_tested": true,
            "clipboard_image_delivery_tested": false,
            "clipboard_notifications_tested": false,
            "pond_shell_tested": false,
            "platform_proxy_tested": false,
            "physical_vt_tested": false,
        }))
        .unwrap(),
    )
    .unwrap();
    backend.stop();
}
