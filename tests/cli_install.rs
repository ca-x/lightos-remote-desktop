use std::{fs, process::Command};

#[test]
fn embedded_installer_stages_itself_preserves_python_and_rolls_back_without_source_tree() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let unit = root.join("etc/systemd/system/lightos-remote-desktop.service");
    fs::create_dir_all(unit.parent().unwrap()).unwrap();
    let original_unit = "[Service]\nUser=desktop\nExecStart=/original/python\n";
    fs::write(&unit, original_unit).unwrap();
    let environment = root.join("etc/lightos/remote-desktop.env");
    fs::create_dir_all(environment.parent().unwrap()).unwrap();
    let original_env = "LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD=preserve-this-test-value\n";
    fs::write(&environment, original_env).unwrap();
    let binary = env!("CARGO_BIN_EXE_lightos-remoted");
    let run = |action: &str, extra: &[&str]| {
        Command::new(binary)
            .current_dir(root)
            .arg(action)
            .arg("--legacy")
            .args(["--stage-only", "--test-root"])
            .arg(root)
            .args(extra)
            .output()
            .unwrap()
    };
    let state = root.join("var/lib/lightos-remote-desktop");
    let dry_run = run("install-service", &["--dry-run"]);
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    assert!(!state.exists());
    let installed = run("install-service", &[]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let current = root.join("usr/local/lib/lightos-remote-desktop/current/lightos-remote-desktop");
    assert_eq!(fs::read(current).unwrap(), fs::read(binary).unwrap());
    assert_eq!(fs::read_to_string(&unit).unwrap(), original_unit);
    assert_eq!(fs::read_to_string(&environment).unwrap(), original_env);
    assert_eq!(
        fs::read_to_string(state.join("original/unit")).unwrap(),
        original_unit
    );
    let restored = run("rollback-service", &[]);
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    assert!(
        !root
            .join("etc/systemd/system/lightos-remote-desktop.service.d/90-native.conf")
            .exists()
    );
    assert_eq!(fs::read_to_string(environment).unwrap(), original_env);
}

#[test]
fn attached_installation_does_not_require_or_start_a_virtual_desktop() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let unit = root.join("etc/systemd/system/lightos-remote-desktop.service");
    fs::create_dir_all(unit.parent().unwrap()).unwrap();
    fs::write(
        &unit,
        "[Service]\nUser=desktop\nExecStart=/original/python\n",
    )
    .unwrap();
    let environment = root.join("etc/lightos/remote-desktop.env");
    fs::create_dir_all(environment.parent().unwrap()).unwrap();
    fs::write(
        &environment,
        "LIGHTOS_REMOTE_DESKTOP_WAYLAND_SOCKET=/run/user/1000/wayland-0\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
        .args(["install-service", "--legacy", "--stage-only", "--test-root"])
        .arg(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        !root
            .join("etc/systemd/system/lightos-remote-desktop.service.d/95-session.conf")
            .exists()
    );
    assert!(
        !root
            .join("etc/systemd/system/lightos-remote-desktop-session.service")
            .exists()
    );
}

#[test]
fn staged_session_owner_respects_user_dropin_and_numeric_uid() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let unit = root.join("etc/systemd/system/lightos-remote-desktop.service");
    fs::create_dir_all(root.join("etc/systemd/system/lightos-remote-desktop.service.d")).unwrap();
    fs::write(unit, "[Service]\nExecStart=/original/python\n").unwrap();
    fs::write(
        root.join("etc/systemd/system/lightos-remote-desktop.service.d/10-user.conf"),
        "[Service]\nUser=2000\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
        .args(["install-service", "--legacy", "--stage-only", "--test-root"])
        .arg(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let owner =
        fs::read_to_string(root.join("etc/systemd/system/lightos-remote-desktop-session.service"))
            .unwrap();
    assert!(owner.lines().any(|line| line == "User=2000"));
}

#[test]
fn public_binary_uses_the_unified_remoted_name() {
    let version = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        format!("lightos-remoted {}", env!("CARGO_PKG_VERSION"))
    );
    let help = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: lightos-remoted")
    );
}

#[test]
fn canonical_units_export_without_installation_and_flags_are_exclusive() {
    let directory = tempfile::tempdir().unwrap();
    for (unit, expected) in [
        (
            "transport",
            include_str!("../systemd/lightos-remoted-transport.service"),
        ),
        (
            "session",
            include_str!("../systemd/lightos-remoted-session.service"),
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
            .current_dir(directory.path())
            .args(["install-service", "--print-unit", unit])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    for flags in [
        ["--package-managed", "--snapshot"],
        ["--snapshot", "--remove"],
        ["--remove", "--package-managed"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
            .arg("install-service")
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}

#[test]
fn standalone_install_creates_both_units_preserves_state_and_rolls_back() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let run = |action: &str, extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
            .arg(action)
            .args(["--stage-only", "--test-root"])
            .arg(root)
            .args(extra)
            .output()
            .unwrap()
    };
    let dry = run("install-service", &["--dry-run"]);
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    let installed = run("install-service", &[]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    for name in [
        "lightos-remote-desktop.service",
        "lightos-remote-desktop-session.service",
    ] {
        assert!(root.join("usr/lib/systemd/system").join(name).is_file());
    }
    let payload = root.join("usr/lib/lightos-remoted/lightos-remoted");
    assert_eq!(
        fs::read(&payload).unwrap(),
        fs::read(env!("CARGO_BIN_EXE_lightos-remoted")).unwrap()
    );
    let environment = root.join("etc/lightos/remote-desktop.env");
    fs::create_dir_all(environment.parent().unwrap()).unwrap();
    fs::write(&environment, "private authentication\n").unwrap();
    let upgraded = run("install-service", &[]);
    assert!(
        upgraded.status.success(),
        "{}",
        String::from_utf8_lossy(&upgraded.stderr)
    );
    let rollback = run("rollback-service", &[]);
    assert!(
        rollback.status.success(),
        "{}",
        String::from_utf8_lossy(&rollback.stderr)
    );
    assert_eq!(
        fs::read_to_string(environment).unwrap(),
        "private authentication\n"
    );
    assert!(payload.is_file());
}

#[test]
fn standalone_refuses_package_files_and_legacy_units_without_modification() {
    for package in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let protected = if package {
            root.join("var/lib/pacman/local/lightos-remoted-bin-0.1.11-1/files")
        } else {
            root.join("etc/systemd/system/lightos-remote-desktop.service")
        };
        fs::create_dir_all(protected.parent().unwrap()).unwrap();
        let contents = if package {
            "%FILES%\nusr/lib/lightos-remoted/lightos-remoted\n"
        } else {
            "[Service]\nExecStart=/existing/payload\n"
        };
        fs::write(&protected, contents).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
            .args(["install-service", "--stage-only", "--test-root"])
            .arg(root)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(protected).unwrap(), contents);
        assert!(!root.join("usr/lib/lightos-remoted").exists());
    }
}
