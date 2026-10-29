#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::{
        fs::{MetadataExt, PermissionsExt, symlink},
        net::UnixListener,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    _directory: tempfile::TempDir,
    runtime: PathBuf,
    bin: PathBuf,
    calls: PathBuf,
    _sockets: Vec<UnixListener>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path().join("runtime");
        let bin = directory.path().join("bin");
        let calls = directory.path().join("activation.args");
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(&bin).unwrap();
        let sockets = ["bus", "wayland-7", "niri.control.sock"]
            .map(|name| UnixListener::bind(runtime.join(name)).unwrap());
        let activator = bin.join("dbus-update-activation-environment");
        let quoted = calls.to_str().unwrap().replace('\'', "'\\''");
        fs::write(
            &activator,
            format!(
                "#!/bin/sh\nif /usr/bin/printenv LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD >/dev/null; then exit 92; fi\nprintf '%s\\n' \"$@\" > '{quoted}'\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&activator, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _directory: directory,
            runtime,
            bin,
            calls,
            _sockets: sockets.into(),
        }
    }

    fn record(&self) -> PathBuf {
        self.runtime.join("niri-environment.json")
    }

    fn command(&self) -> Command {
        self.command_for(&self.runtime)
    }

    fn command_for(&self, runtime: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"));
        let bus = format!("unix:path={}", self.runtime.join("bus").display());
        command
            .env_clear()
            .env("PATH", &self.bin)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("DBUS_SESSION_BUS_ADDRESS", bus)
            .env("WAYLAND_DISPLAY", "wayland-7")
            .env("NIRI_SOCKET", self.runtime.join("niri.control.sock"))
            .env("DISPLAY", ":93")
            .env(
                "LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD",
                "fixture-auth-sentinel-not-for-readiness",
            )
            .args(["niri-session-environment", "--runtime"])
            .arg(runtime);
        command
    }
}

fn run(mut command: Command) -> Output {
    let mut child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            panic!("owned environment helper exceeded its fixture deadline");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn publishes_only_private_display_values_after_activation_without_importing_auth() {
    let fixture = Fixture::new();
    let output = run(fixture.command());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = fs::symlink_metadata(fixture.record()).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        metadata.uid(),
        fs::metadata(&fixture.runtime).unwrap().uid()
    );
    let bytes = fs::read(fixture.record()).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document.as_object().unwrap().len(), 3);
    assert_eq!(document["wayland_display"], "wayland-7");
    assert_eq!(
        document["niri_socket"],
        fixture.runtime.join("niri.control.sock").to_str().unwrap()
    );
    assert_eq!(document["display"], ":93");
    let content = String::from_utf8(bytes).unwrap();
    assert!(!content.contains("fixture-auth-sentinel"));
    let arguments = fs::read_to_string(&fixture.calls).unwrap();
    assert!(arguments.contains("WAYLAND_DISPLAY=wayland-7"));
    assert!(arguments.contains("DISPLAY=:93"));
    assert!(!arguments.contains("AUTH"));
    assert!(!arguments.contains("fixture-auth-sentinel"));
    assert!(!arguments.contains("--systemd"));
    assert_eq!(arguments.lines().count(), 7);
    assert!(arguments.contains("XDG_CURRENT_DESKTOP=niri"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-auth-sentinel"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture-auth-sentinel"));
}

#[test]
fn rejects_a_foreign_bus_before_activating_or_writing_readiness() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command.env(
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:path=/tmp/not-this-runtime",
    );
    let output = run(command);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("private session bus"));
    assert!(!fixture.calls.exists());
    assert!(!fixture.record().exists());
}

#[test]
fn rejects_a_mismatched_runtime_before_activating_or_writing_readiness() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command.env("XDG_RUNTIME_DIR", "/tmp/not-this-runtime");
    let output = run(command);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("XDG_RUNTIME_DIR"));
    assert!(!fixture.calls.exists());
    assert!(!fixture.record().exists());
}

#[test]
fn accepts_the_private_bus_guid_without_importing_into_user_systemd() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command.env(
        "DBUS_SESSION_BUS_ADDRESS",
        format!(
            "unix:path={},guid=0123456789abcdef0123456789abcdef",
            fixture.runtime.join("bus").display()
        ),
    );
    let output = run(command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fixture.record().exists());
    assert!(
        !fs::read_to_string(&fixture.calls)
            .unwrap()
            .contains("--systemd")
    );
}

#[test]
fn publishes_without_an_x11_display() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command.env_remove("DISPLAY");
    let output = run(command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.record()).unwrap()).unwrap();
    assert_eq!(document.as_object().unwrap().len(), 2);
    assert!(document.get("display").is_none());
    assert!(
        fs::read_to_string(&fixture.calls)
            .unwrap()
            .lines()
            .any(|line| line == "DISPLAY=")
    );
}

#[test]
fn rejects_ambiguous_or_non_filesystem_bus_addresses() {
    for suffix in [
        ";unix:path=/tmp/foreign",
        ",path=/tmp/foreign",
        ",unused=yes",
    ] {
        let fixture = Fixture::new();
        let mut command = fixture.command();
        command.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!(
                "unix:path={}{suffix}",
                fixture.runtime.join("bus").display()
            ),
        );
        let output = run(command);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("private session bus"));
        assert!(!fixture.calls.exists());
        assert!(!fixture.record().exists());
    }
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command.env(
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:abstract=not-the-private-bus",
    );
    let output = run(command);
    assert!(!output.status.success());
    assert!(!fixture.calls.exists());
    assert!(!fixture.record().exists());
}

#[test]
fn rejects_socket_symlinks_and_nonprivate_runtime_before_activation() {
    for name in ["bus", "wayland-7", "niri.control.sock"] {
        let fixture = Fixture::new();
        let socket = fixture.runtime.join(name);
        let other = fixture.runtime.join(format!("other-{name}"));
        fs::rename(&socket, &other).unwrap();
        symlink(other, socket).unwrap();
        let output = run(fixture.command());
        assert!(!output.status.success());
        assert!(!fixture.calls.exists());
        assert!(!fixture.record().exists());
    }
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o755)).unwrap();
    let output = run(fixture.command());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("private directory"));
    assert!(!fixture.calls.exists());
    assert!(!fixture.record().exists());
}

#[test]
fn rejects_runtime_aliases_and_socket_escapes_before_activation() {
    for (name, value) in [
        ("WAYLAND_DISPLAY", "../wayland-7"),
        ("WAYLAND_DISPLAY", "/tmp/wayland-7"),
        ("NIRI_SOCKET", "/tmp/foreign-niri.sock"),
        ("DISPLAY", "remote.example:0"),
    ] {
        let fixture = Fixture::new();
        let mut command = fixture.command();
        command.env(name, value);
        let output = run(command);
        assert!(!output.status.success());
        assert!(!fixture.calls.exists());
        assert!(!fixture.record().exists());
    }
    let fixture = Fixture::new();
    let alias = fixture._directory.path().join("runtime-alias");
    symlink(&fixture.runtime, &alias).unwrap();
    let mut command = fixture.command_for(&alias);
    command.env("XDG_RUNTIME_DIR", alias);
    let output = run(command);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("private directory"));
    assert!(!fixture.calls.exists());
    assert!(!fixture.record().exists());
}

#[test]
fn never_replaces_existing_readiness_or_activates_for_a_stale_record() {
    let fixture = Fixture::new();
    fs::write(fixture.record(), b"old-generation\n").unwrap();
    let output = run(fixture.command());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
    assert!(!fixture.calls.exists());
    assert_eq!(fs::read(fixture.record()).unwrap(), b"old-generation\n");
}

#[test]
fn activation_failure_never_publishes_readiness() {
    let fixture = Fixture::new();
    fs::write(
        fixture.bin.join("dbus-update-activation-environment"),
        "#!/bin/sh\nexit 42\n",
    )
    .unwrap();
    let output = run(fixture.command());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("activation failed"));
    assert!(!fixture.record().exists());
}

#[test]
fn activation_deadline_kills_and_reaps_its_owned_helper_without_publishing() {
    let fixture = Fixture::new();
    let pid_file = fixture._directory.path().join("activator.pid");
    fs::write(
        fixture.bin.join("dbus-update-activation-environment"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec /bin/sleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();
    let start = Instant::now();
    let output = run(fixture.command());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(8));
    assert!(!fixture.record().exists());
    let pid: i32 = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
