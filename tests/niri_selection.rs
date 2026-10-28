#![cfg(target_os = "linux")]

use std::{path::Path, process::Command};

fn doctor(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"));
    command
        .env_clear()
        .env("PATH", "")
        .env("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD", "test-native-long-secret")
        .env("LIGHTOS_REMOTE_DESKTOP_COMPOSITOR", "niri")
        .current_dir(directory)
        .args(["doctor", "--audio", "false"]);
    command
}

#[test]
fn compositor_environment_and_explicit_cli_precedence_select_the_native_tools() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("config.kdl");
    std::fs::write(&native, "").unwrap();
    let native_output = doctor(directory.path())
        .arg("--desktop-config")
        .arg(&native)
        .output()
        .unwrap();
    assert!(!native_output.status.success());
    let native_error = String::from_utf8(native_output.stderr).unwrap();
    assert!(native_error.contains("niri"), "{native_error}");
    assert!(!native_error.contains("Hyprland"), "{native_error}");
    let overridden = doctor(directory.path())
        .args(["--compositor", "hyprland", "--desktop-config"])
        .arg(&native)
        .output()
        .unwrap();
    assert!(!overridden.status.success());
    let override_error = String::from_utf8(overridden.stderr).unwrap();
    assert!(override_error.contains("Hyprland"), "{override_error}");
    assert!(!override_error.contains("niri"), "{override_error}");
}

#[test]
fn persistent_attachment_needs_no_home_or_startup_config_and_creates_no_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("owner");
    let output = doctor(directory.path())
        .arg("--session-runtime")
        .arg(&runtime)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["desktop_smoke_test"], false);
    assert!(!String::from_utf8(output.stdout).unwrap().contains("test-native-long-secret"));
    assert!(!runtime.exists());
}

#[test]
fn persistent_attachment_still_rejects_an_explicit_relative_startup_path() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("owner");
    let output = doctor(directory.path())
        .arg("--session-runtime")
        .arg(&runtime)
        .args(["--desktop-config", "relative.kdl"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("absolute"));
    assert!(!runtime.exists());
}
