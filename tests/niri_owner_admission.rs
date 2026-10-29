#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::{fs::{PermissionsExt, symlink}, net::UnixListener, process::CommandExt},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn forged_native_readiness_is_rejected_before_handoff() {
    for (case, reason) in [
        ("executable", "not the selected Niri executable"),
        ("origin", "not a controlled Cage descendant"),
        ("peers", "same Niri kernel peer"),
        ("public", "private owned regular file"),
        ("escaped", "WAYLAND_DISPLAY"),
        ("unknown", "invalid Niri readiness record"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path().join("session");
        let bin = directory.path().join("bin");
        let home = directory.path().join("home");
        for path in [&runtime, &bin, &home] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let native = directory.path().join("config.kdl");
        fs::write(&native, "").unwrap();
        symlink(if case == "origin" { "/usr/bin/python3" } else { "/usr/bin/true" }, bin.join("niri")).unwrap();
        let _foreign = (case == "peers").then(|| UnixListener::bind(runtime.join("wayland-forged")).unwrap());
        if case == "executable" {
            fs::write(runtime.join("niri-environment.json"), "stale-generation-not-json").unwrap();
            fs::set_permissions(runtime.join("niri-environment.json"), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let cage = bin.join("cage");
        let source = format!(r#"#!/usr/bin/python3
import json
import os
from pathlib import Path
import socket
import tempfile
import time

case = {case:?}
runtime = Path(os.environ["XDG_RUNTIME_DIR"])
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
"#);
        fs::write(&cage, source).unwrap();
        fs::set_permissions(&cage, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
            .env_clear().env("HOME", &home)
            .env("PATH", std::env::join_paths([bin.as_path(), std::path::Path::new("/usr/bin")]).unwrap())
            .args(["desktop-session", "--password", "isolated-niri-admission-fixture", "--compositor", "niri", "--audio", "false", "--runtime-dir"])
            .arg(&runtime).arg("--desktop-config").arg(&native).arg("--cage").arg(&cage)
            .process_group(0).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if child.try_wait().unwrap().is_none() {
            unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            let cleanup = Instant::now() + Duration::from_secs(2);
            while child.try_wait().unwrap().is_none() && Instant::now() < cleanup {
                thread::sleep(Duration::from_millis(20));
            }
            if child.try_wait().unwrap().is_none() {
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            }
        }
        let output = child.wait_with_output().unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success() && diagnostic.contains(reason), "{case}: {diagnostic}");
        assert!(!runtime.join("session.json").exists(), "{case} published a descriptor");
    }
}
