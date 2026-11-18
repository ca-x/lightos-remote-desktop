//! Verify an explicitly owned virtual X server without weakening physical-login guards.
use std::{
    fs::{self, File},
    mem::size_of,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    process::Child,
};

use anyhow::{Context, Result, ensure};
use display_control::SessionTarget;

use crate::config::Config;

pub(crate) fn verify(
    config: &Config,
    target: &mut SessionTarget,
    launcher: Option<&Child>,
) -> Result<Option<File>> {
    let Some(pid) = config.virtual_x11_server_pid else {
        return Ok(None);
    };
    ensure!(pid > 1, "invalid virtual X11 server PID");
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32 };
    ensure!(
        raw >= 0,
        "pin virtual X11 server: {}",
        std::io::Error::last_os_error()
    );
    let lifetime = unsafe { File::from_raw_fd(raw) };
    let owner = unsafe { libc::geteuid() };
    let authority = target
        .xauthority
        .as_ref()
        .context("virtual X11 requires Xauthority")?;
    let runtime = authority
        .parent()
        .context("virtual X11 requires a private runtime")?;
    let metadata = fs::symlink_metadata(runtime)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == owner
            && metadata.mode() & 0o077 == 0,
        "virtual X11 runtime must be an owner-only directory"
    );
    let process = PathBuf::from(format!("/proc/{pid}"));
    ensure!(
        fs::metadata(&process)?.uid() == owner,
        "virtual X11 server belongs to another user"
    );
    ensure!(
        fs::read_link(process.join("exe"))?
            .file_name()
            .is_some_and(|name| matches!(name.to_str(), Some("Xvfb" | "Xephyr"))),
        "virtual X11 requires an owned Xvfb or Xephyr server"
    );
    ensure!(
        fs::read(process.join("cgroup"))?
            == fs::read(crate::session::launcher_cgroup_path(launcher))?,
        "virtual X11 server must belong to the same service"
    );
    let display = target
        .x11_display
        .as_deref()
        .context("virtual X11 requires a local display")?;
    let command = fs::read(process.join("cmdline"))?;
    let arguments: Vec<_> = command.split(|byte| *byte == 0).collect();
    ensure!(
        arguments.get(1).copied() == Some(display.as_bytes()),
        "virtual X11 display does not match its server"
    );
    ensure!(
        arguments
            .windows(2)
            .any(|pair| pair[0] == b"-auth" && pair[1] == authority.as_os_str().as_encoded_bytes()),
        "virtual X11 server does not use the selected Xauthority"
    );
    let number = display
        .trim_start_matches(':')
        .split('.')
        .next()
        .unwrap_or_default();
    let socket = UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))?;
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    let status = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    ensure!(
        status == 0 && peer.pid as u32 == pid && peer.uid == owner,
        "selected X11 socket is not served by the owned virtual X server process"
    );
    ensure!(
        alive(&lifetime),
        "virtual X11 server exited during verification"
    );
    target.runtime = runtime.to_path_buf();
    target.bus_address = config
        .virtual_environment
        .get("DBUS_SESSION_BUS_ADDRESS")
        .cloned()
        .or_else(|| std::env::var("DBUS_SESSION_BUS_ADDRESS").ok());
    Ok(Some(lifetime))
}

pub(crate) fn alive(lifetime: &File) -> bool {
    let mut descriptor = libc::pollfd {
        fd: lifetime.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut descriptor, 1, 0) == 0 }
}

pub(crate) fn audio_source(
    config: &Config,
    target: &SessionTarget,
    server: Option<&str>,
) -> Result<Option<String>> {
    let Some(source) = config
        .virtual_environment
        .get("PULSE_SOURCE")
        .cloned()
        .or_else(|| std::env::var("PULSE_SOURCE").ok())
    else {
        return Ok(None);
    };
    let socket = target.runtime.join("pulse/native");
    ensure!(
        server == Some(format!("unix:{}", socket.display()).as_str()),
        "virtual desktop audio must use its private runtime server"
    );
    ensure!(
        crate::session::is_owned_socket(Path::new(&socket), unsafe { libc::geteuid() }),
        "virtual desktop audio socket is unavailable or owned by another user"
    );
    ensure!(
        source.ends_with(".monitor"),
        "virtual desktop audio requires a monitor source"
    );
    Ok(Some(source))
}
