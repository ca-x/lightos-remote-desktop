use super::*;
use anyhow::{Context, ensure};
use std::{
    fs,
    io::Read,
    os::fd::AsRawFd,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    process::Command,
};
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
    if options.package_managed && options.stage_only {
        return stage_package(options.test_root.as_deref());
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

fn package_file(root: &Path, relative: &str) -> Result<fs::File> {
    let path = root.join(relative);
    let parent = path.parent().context("package path has no parent")?;
    ensure!(
        fs::canonicalize(parent)? == parent,
        "package staging refuses symlinked ancestors: {relative}"
    );
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
        .open(&path)
        .with_context(|| format!("open staged package file: {relative}"))?;
    ensure!(
        file.metadata()?.is_file(),
        "staged package file must be regular: {relative}"
    );
    fs::File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .with_context(|| format!("read verified staged package inode through procfs: {relative}"))
}

fn stage_package(test_root: Option<&Path>) -> Result<()> {
    let root = fs::canonicalize(test_root.unwrap_or(Path::new("/")))?;
    ensure!(
        test_root.is_none() || root != Path::new("/"),
        "test root cannot be /"
    );
    let binary = package_file(&root, "usr/lib/lightos-remoted/lightos-remoted")?;
    let metadata = binary.metadata()?;
    ensure!(
        metadata.len() > 0 && metadata.permissions().mode() & 0o111 != 0,
        "staged package executable is empty or not executable"
    );
    for (name, expected) in [
        ("lightos-remote-desktop.service", TRANSPORT),
        ("lightos-remote-desktop-session.service", SESSION),
    ] {
        let file = package_file(&root, &format!("usr/lib/systemd/system/{name}"))?;
        let mut bytes = Vec::new();
        file.take(u64::try_from(expected.len())? + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes == expected.as_bytes(),
            "staged package unit differs from embedded definition: {name}"
        );
    }
    println!(
        "LightOS remote desktop payload staged; no accounts, credentials or services changed. Activate on the booted instance with: lightos-remoted install-service --package-managed"
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
