#![cfg(target_os = "linux")]

use std::{
    fs, io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{MetadataExt, PermissionsExt, symlink},
            net::UnixListener,
            process::CommandExt,
        },
    },
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct FixtureCleanup;

fn direct_children() -> io::Result<Vec<u32>> {
    let mut children = Vec::new();
    for task in fs::read_dir("/proc/self/task")? {
        let task = task?;
        let contents = match fs::read_to_string(task.path().join("children")) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for pid in contents.split_whitespace() {
            children.push(
                pid.parse()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            );
        }
    }
    children.sort_unstable();
    children.dedup();
    Ok(children)
}

fn child_start(pid: u32) -> Option<String> {
    let process = format!("/proc/{pid}");
    if fs::metadata(&process).ok()?.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let stat = fs::read_to_string(format!("{process}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    if fields.get(1)?.parse::<u32>().ok()? != std::process::id() {
        return None;
    }
    Some(fields.get(19)?.to_string())
}

fn child_pidfd(pid: u32) -> Option<fs::File> {
    let start = child_start(pid)?;
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32 };
    if raw < 0 {
        return None;
    }
    let lifetime = unsafe { fs::File::from_raw_fd(raw) };
    (child_start(pid).as_deref() == Some(start.as_str())).then_some(lifetime)
}

impl FixtureCleanup {
    fn new() -> Self {
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
            0
        );
        Self
    }

    fn drain(&self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let children = direct_children()?;
            if children.is_empty() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("fixture children remain after cleanup: {children:?}"),
                ));
            }
            for pid in children {
                if let Some(lifetime) = child_pidfd(pid) {
                    unsafe {
                        libc::syscall(
                            libc::SYS_pidfd_send_signal,
                            lifetime.as_raw_fd(),
                            libc::SIGKILL,
                            std::ptr::null::<libc::siginfo_t>(),
                            0,
                        );
                        libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG);
                    }
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        if let Err(error) = self.drain() {
            eprintln!("independent admission fixture cleanup failed: {error}");
        }
    }
}

#[test]
fn forged_native_readiness_is_rejected_before_handoff() {
    for (case, reason) in [
        ("executable", "not the selected Niri executable"),
        ("origin", "not a controlled Cage descendant"),
        ("origin-sibling", "not a controlled Cage descendant"),
        ("peers", "same Niri kernel peer"),
        ("public", "private owned regular file"),
        ("escaped", "WAYLAND_DISPLAY"),
        ("unknown", "invalid Niri readiness record"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let cleanup = FixtureCleanup::new();
        let runtime = directory.path().join("session");
        let bin = directory.path().join("bin");
        let home = directory.path().join("home");
        for path in [&runtime, &bin, &home] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let native = directory.path().join("config.kdl");
        fs::write(&native, "").unwrap();
        symlink(
            if matches!(case, "origin" | "origin-sibling") {
                "/usr/bin/python3"
            } else {
                "/usr/bin/true"
            },
            bin.join("niri"),
        )
        .unwrap();
        let _foreign =
            (case == "peers").then(|| UnixListener::bind(runtime.join("wayland-forged")).unwrap());
        if case == "executable" {
            fs::write(
                runtime.join("niri-environment.json"),
                "stale-generation-not-json",
            )
            .unwrap();
            fs::set_permissions(
                runtime.join("niri-environment.json"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let cage = bin.join("cage");
        let fixture_pid = std::process::id();
        let source = format!(
            r#"#!/usr/bin/python3
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import time

case = {case:?}
runtime = Path(os.environ["XDG_RUNTIME_DIR"])
if case == "origin-sibling":
    cage_record = runtime / "cage.pid"
    if sys.argv[1:2] != ["foreign-peer"]:
        with tempfile.NamedTemporaryFile(mode="w", dir=runtime, delete=False) as output:
            output.write(str(os.getpid()))
            temporary = output.name
        os.replace(temporary, cage_record)
        time.sleep(30)
        raise SystemExit(0)
    deadline = time.monotonic() + 5
    while not cage_record.exists():
        if time.monotonic() >= deadline:
            raise RuntimeError("controlled Cage did not start")
        time.sleep(0.02)
    cage_pid = int(cage_record.read_text())
    assert os.getppid() == {fixture_pid}
    assert os.getpid() != cage_pid
    os.setpgid(0, cage_pid)
    executable = os.stat("/proc/self/exe")
    origin = dict(pid=os.getpid(), parent=os.getppid(), cage=cage_pid,
                  group=os.getpgrp(), uid=os.geteuid(),
                  device=executable.st_dev, inode=executable.st_ino)
    (runtime / "origin.json").write_text(json.dumps(origin))
listeners = []
for name in ("wayland-forged", "niri-forged.sock"):
    if case == "peers" and name == "wayland-forged":
        continue
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(str(runtime / name))
    listener.listen(8)
    listeners.append(listener)
document = dict(wayland_display="wayland-forged", niri_socket=str(runtime / "niri-forged.sock"))
if case == "escaped":
    document["wayland_display"] = "../foreign"
if case == "unknown":
    document["unexpected"] = "not-allowed"
with tempfile.NamedTemporaryFile(mode="w", dir=runtime, delete=False) as output:
    json.dump(document, output)
    temporary = output.name
if case == "public":
    os.chmod(temporary, 0o644)
os.replace(temporary, runtime / "niri-environment.json")
time.sleep(30)
"#
        );
        fs::write(&cage, source).unwrap();
        fs::set_permissions(&cage, fs::Permissions::from_mode(0o700)).unwrap();
        let foreign_peer = (case == "origin-sibling").then(|| {
            Command::new("/usr/bin/python3")
                .arg(&cage)
                .arg("foreign-peer")
                .env_clear()
                .env("XDG_RUNTIME_DIR", &runtime)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(fs::File::create(directory.path().join("peer.log")).unwrap())
                .spawn()
                .unwrap()
        });
        let log = directory.path().join("owner.log");
        let mut child = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
            .env_clear()
            .env("HOME", &home)
            .env(
                "PATH",
                std::env::join_paths([bin.as_path(), std::path::Path::new("/usr/bin")]).unwrap(),
            )
            .args([
                "desktop-session",
                "--password",
                "isolated-niri-admission-fixture",
                "--compositor",
                "niri",
                "--audio",
                "false",
                "--runtime-dir",
            ])
            .arg(&runtime)
            .arg("--desktop-config")
            .arg(&native)
            .arg("--cage")
            .arg(&cage)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "{case} did not reject forged readiness: {}",
                fs::read_to_string(&log).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(20));
        };
        let diagnostic = fs::read_to_string(&log).unwrap();
        if let Some(peer) = &foreign_peer {
            let origin: serde_json::Value =
                serde_json::from_slice(&fs::read(runtime.join("origin.json")).unwrap()).unwrap();
            let executable = fs::metadata(bin.join("niri")).unwrap();
            assert_eq!(origin["pid"].as_u64(), Some(u64::from(peer.id())));
            assert_eq!(origin["parent"].as_u64(), Some(u64::from(fixture_pid)));
            assert_ne!(origin["pid"], origin["cage"]);
            assert_eq!(origin["group"], origin["cage"]);
            assert_eq!(
                origin["uid"].as_u64(),
                Some(u64::from(unsafe { libc::geteuid() }))
            );
            assert_eq!(origin["device"].as_u64(), Some(executable.dev()));
            assert_eq!(origin["inode"].as_u64(), Some(executable.ino()));
        }
        let origin_ancestor_rejected =
            case == "origin-sibling" && diagnostic.contains("foreign Niri process owner");
        assert!(
            !status.success() && (diagnostic.contains(reason) || origin_ancestor_rejected),
            "{case}: {diagnostic}"
        );
        assert!(
            !runtime.join("session.json").exists(),
            "{case} published a descriptor"
        );
        cleanup.drain().unwrap();
    }
}
