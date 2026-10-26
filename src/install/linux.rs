use super::*;
use anyhow::{Context, ensure};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
const TRANSPORT: &str = include_str!("../../systemd/lightos-remoted-transport.service");
const SESSION: &str = include_str!("../../systemd/lightos-remoted-session.service");

pub(super) fn service(options: ServiceOptions, rollback: bool) -> Result<()> {
    let package = options.package_managed || options.snapshot || options.remove;
    ensure!(
        !rollback || (!package && options.print_unit.is_none()),
        "rollback does not support package lifecycle or unit export flags"
    );
    if let Some(unit) = options.print_unit {
        print!(
            "{}",
            match unit {
                ServiceUnit::Transport => TRANSPORT,
                ServiceUnit::Session => SESSION,
            }
        );
        return Ok(());
    }
    if options.legacy {
        return legacy_service(options, rollback);
    }
    ensure!(
        options.dry_run || options.test_root.is_some() || unsafe { libc::geteuid() } == 0,
        "service installation requires root; run with sudo (or use --dry-run)"
    );
    let directory = tempfile::Builder::new()
        .prefix("lightos-services.")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()?;
    for (name, source) in [
        (
            "scripts/service-install.sh",
            include_str!("../../scripts/service-install.sh"),
        ),
        (
            "scripts/service-bootstrap.sh",
            include_str!("../../scripts/service-bootstrap.sh"),
        ),
        (
            "scripts/service-activation.sh",
            include_str!("../../scripts/service-activation.sh"),
        ),
        ("systemd/lightos-remoted-transport.service", TRANSPORT),
        ("systemd/lightos-remoted-session.service", SESSION),
    ] {
        let path = directory.path().join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, source)?;
    }
    let mut command = Command::new("/bin/bash");
    command.arg(directory.path().join(if package {
        "scripts/service-bootstrap.sh"
    } else {
        "scripts/service-install.sh"
    }));
    if options.snapshot {
        command.arg("--snapshot");
    }
    if options.remove {
        command.arg("--remove");
    }
    if !package {
        if rollback {
            command.arg("--rollback");
        } else {
            command.arg("--binary").arg(std::env::current_exe()?);
        }
        if options.dry_run {
            command.arg("--dry-run");
        }
        if options.stage_only {
            command.arg("--stage-only");
        }
    }
    if let Some(root) = options.test_root {
        command.arg("--test-root").arg(root);
    }
    let status = command.status().context("run embedded service installer")?;
    ensure!(
        status.success(),
        "service installer failed: {status}; inspect its recovery output"
    );
    Ok(())
}

fn legacy_service(options: ServiceOptions, rollback: bool) -> Result<()> {
    ensure!(
        options.dry_run || options.test_root.is_some() || unsafe { libc::geteuid() } == 0,
        "service installation requires root; run with sudo (or use --dry-run)"
    );
    let directory = tempfile::Builder::new()
        .prefix("lightos-installer.")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()?;
    for (name, source) in [
        (
            "scripts/install.sh",
            include_str!("../../scripts/install.sh"),
        ),
        (
            "scripts/activation.sh",
            include_str!("../../scripts/activation.sh"),
        ),
        (
            "systemd/95-session.conf",
            include_str!("../../systemd/95-session.conf"),
        ),
        (
            "systemd/lightos-remote-desktop-session.service",
            include_str!("../../systemd/lightos-remote-desktop-session.service"),
        ),
        (
            "systemd/90-native.conf",
            include_str!("../../systemd/90-native.conf"),
        ),
    ] {
        let path = directory.path().join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, source)?;
    }
    let mut command = Command::new("/bin/bash");
    command.arg(directory.path().join("scripts/install.sh"));
    if rollback {
        command.arg("--rollback");
    } else {
        command.arg("--binary").arg(std::env::current_exe()?);
    }
    if options.dry_run {
        command.arg("--dry-run");
    } else if !options.stage_only {
        command.arg("--activate");
    }
    if let Some(root) = options.test_root {
        command.arg("--test-root").arg(root);
    }
    let status = command
        .status()
        .context("run embedded transactional installer")?;
    ensure!(
        status.success(),
        "service installer failed: {status}; inspect its recovery output"
    );
    Ok(())
}
