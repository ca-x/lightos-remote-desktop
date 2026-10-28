#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::{Command, Output};

const UNITS: [(&str, &str); 2] = [
    (
        "lightos-remote-desktop.service",
        include_str!("../systemd/lightos-remoted-transport.service"),
    ),
    (
        "lightos-remote-desktop-session.service",
        include_str!("../systemd/lightos-remoted-session.service"),
    ),
];

fn fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("usr/lib/lightos-remoted")).unwrap();
    fs::create_dir_all(root.join("usr/lib/systemd/system")).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_lightos-remoted"),
        root.join("usr/lib/lightos-remoted/lightos-remoted"),
    )
    .unwrap();
    for (name, source) in UNITS {
        fs::write(root.join("usr/lib/systemd/system").join(name), source).unwrap();
    }
    directory
}

fn stage(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
        .args([
            "install-service",
            "--package-managed",
            "--stage-only",
            "--test-root",
        ])
        .arg(root)
        .env("PATH", "/no-commands-in-offline-stage")
        .current_dir(root)
        .output()
        .unwrap()
}

#[test]
fn package_staging_is_read_only_and_needs_no_installer_commands() {
    let directory = fixture();
    let root = directory.path();
    fs::create_dir_all(root.join("etc/lightos")).unwrap();
    let environment = root.join("etc/lightos/remote-desktop.env");
    fs::write(&environment, "opaque fixture: must not be parsed or replaced\n").unwrap();
    for _attempt in 0..2 {
        let output = stage(root);
        assert!(output.status.success(), "{:?}", output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("staged"));
        assert_eq!(
            fs::read_to_string(&environment).unwrap(),
            "opaque fixture: must not be parsed or replaced\n"
        );
        for absent in ["var", "run", "etc/systemd", "etc/passwd", "etc/group"] {
            assert!(!root.join(absent).exists(), "unexpected mutation: {absent}");
        }
        for (name, source) in UNITS {
            assert_eq!(
                fs::read_to_string(root.join("usr/lib/systemd/system").join(name)).unwrap(),
                source
            );
        }
    }
}

#[test]
fn package_staging_rejects_missing_modified_and_symlinked_units() {
    for (name, _) in UNITS {
        for condition in ["missing", "modified", "symlink"] {
            let directory = fixture();
            let root = directory.path();
            let unit = root.join("usr/lib/systemd/system").join(name);
            fs::remove_file(&unit).unwrap();
            match condition {
                "modified" => fs::write(&unit, "[Service]\nExecStart=/different\n").unwrap(),
                "symlink" => {
                    let outside = root.join("outside.service");
                    fs::write(&outside, UNITS.iter().find(|entry| entry.0 == name).unwrap().1)
                        .unwrap();
                    symlink(outside, &unit).unwrap();
                }
                _ => {}
            }
            let output = stage(root);
            assert!(!output.status.success(), "accepted {condition} {name}");
            assert!(!root.join("var").exists());
            assert!(!root.join("etc").exists());
        }
    }
}

#[test]
fn package_staging_rejects_an_absent_nonexecutable_or_symlinked_payload() {
    for condition in ["missing", "nonexecutable", "symlink"] {
        let directory = fixture();
        let root = directory.path();
        let binary = root.join("usr/lib/lightos-remoted/lightos-remoted");
        if condition == "nonexecutable" {
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o644)).unwrap();
        } else {
            fs::remove_file(&binary).unwrap();
            if condition == "symlink" {
                symlink(env!("CARGO_BIN_EXE_lightos-remoted"), &binary).unwrap();
            }
        }
        let output = stage(root);
        assert!(!output.status.success(), "accepted {condition} payload");
        assert!(!root.join("var").exists());
        assert!(!root.join("etc").exists());
    }
}
