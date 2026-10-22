use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

use anyhow::{Context, Result, ensure};
use clap::Args;

#[derive(Args)]
pub struct ServiceOptions {
    #[arg(
        long,
        help = "Check prerequisites without changing files or restarting the service"
    )]
    pub dry_run: bool,
    #[arg(
        long,
        help = "Stage the binary and override without restarting the service"
    )]
    pub stage_only: bool,
    #[arg(long, hide = true, requires = "stage_only")]
    pub test_root: Option<PathBuf>,
}

pub fn service(options: ServiceOptions, rollback: bool) -> Result<()> {
    ensure!(
        options.dry_run || options.test_root.is_some() || unsafe { libc::geteuid() } == 0,
        "service installation requires root; run with sudo (or use --dry-run)"
    );
    let directory = tempfile::Builder::new()
        .prefix("lightos-installer.")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()?;
    for (name, source) in [
        ("scripts/install.sh", include_str!("../scripts/install.sh")),
        (
            "scripts/activation.sh",
            include_str!("../scripts/activation.sh"),
        ),
        (
            "systemd/95-session.conf",
            include_str!("../systemd/95-session.conf"),
        ),
        (
            "systemd/lightos-remote-desktop-session.service",
            include_str!("../systemd/lightos-remote-desktop-session.service"),
        ),
        (
            "systemd/90-native.conf",
            include_str!("../systemd/90-native.conf"),
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
