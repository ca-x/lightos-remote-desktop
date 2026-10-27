//! Admission for an explicitly service-owned nested compositor. Physical
//! sessions continue through the normal login/Portal authorization path.
use anyhow::{Context, Result, ensure};
use display_control::SessionTarget;
use std::{
    fs::{self, File},
    mem::size_of,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
};

pub(crate) fn verify(socket: &Path, pid: u32) -> Result<(SessionTarget, File)> {
    ensure!(pid > 1, "invalid owned compositor PID");
    let uid = unsafe { libc::geteuid() };
    let runtime = socket
        .parent()
        .context("owned compositor runtime missing")?;
    ensure!(
        runtime != PathBuf::from(format!("/run/user/{uid}")),
        "a login runtime is not an owned virtual desktop"
    );
    let directory = fs::symlink_metadata(runtime)?;
    ensure!(
        directory.is_dir()
            && !directory.file_type().is_symlink()
            && directory.uid() == uid
            && directory.mode() & 0o077 == 0,
        "owned compositor requires a private runtime"
    );
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32 };
    ensure!(raw >= 0, "could not pin owned compositor");
    let lifetime = unsafe { File::from_raw_fd(raw) };
    let process = PathBuf::from(format!("/proc/{pid}"));
    ensure!(
        fs::metadata(&process)?.uid() == uid,
        "owned compositor belongs to another user"
    );
    ensure!(
        fs::read_link(process.join("exe"))?
            .file_name()
            .is_some_and(|name| matches!(
                name.to_str(),
                Some("gnome-shell" | "kwin_wayland" | "kwin_wayland_wrapper" | "labwc")
            )),
        "unsupported owned compositor"
    );
    let own = fs::read_to_string("/proc/self/cgroup")?;
    let other = fs::read_to_string(process.join("cgroup"))?;
    let own = own
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .context("owned compositor requires a unified service cgroup")?;
    let other = other
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .context("owned compositor cgroup missing")?;
    let own = own
        .strip_suffix("/init.scope")
        .filter(|parent| {
            parent
                .rsplit('/')
                .next()
                .is_some_and(|unit| unit.ends_with(".service"))
        })
        .unwrap_or(own);
    ensure!(
        own != "/"
            && (other == own
                || other
                    .strip_prefix(own)
                    .is_some_and(|suffix| suffix.starts_with('/'))),
        "owned compositor is outside this service ({own} versus {other})"
    );
    let stream = UnixStream::connect(socket)?;
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    ensure!(
        result == 0 && peer.pid as u32 == pid && peer.uid == uid,
        "owned Wayland socket does not match its compositor"
    );
    let environment = fs::read(process.join("environ"))?;
    ensure!(
        environment.len() <= 256 * 1024,
        "owned compositor environment too large"
    );
    let bus = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"DBUS_SESSION_BUS_ADDRESS="))
        .context("owned compositor session bus missing")?;
    let bus = std::str::from_utf8(bus)?.to_owned();
    let address: zbus::Address = bus.as_str().try_into()?;
    let zbus::address::Transport::Unix(unix) = address.transport() else {
        anyhow::bail!("owned compositor requires a local Unix bus");
    };
    let zbus::address::transport::UnixSocket::File(path) = unix.path() else {
        anyhow::bail!("owned compositor requires a filesystem bus socket");
    };
    ensure!(path.is_absolute(), "owned compositor bus path is relative");
    let bus_socket = UnixStream::connect(path)?;
    let mut bus_peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            bus_socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut bus_peer as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    ensure!(
        result == 0 && bus_peer.uid == uid && bus_peer.pid > 1,
        "owned compositor bus belongs to another user"
    );
    let bus_group = fs::read_to_string(format!("/proc/{}/cgroup", bus_peer.pid))?;
    let bus_group = bus_group
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .context("owned bus cgroup missing")?;
    ensure!(
        bus_group == own
            || bus_group
                .strip_prefix(own)
                .is_some_and(|suffix| suffix.starts_with('/')),
        "owned compositor bus is outside this service"
    );
    // Older Xfce starts its own dbus-launch socket under /tmp. Admit it only
    // when the actual server credentials and service cgroup prove ownership;
    // never inherit an ambient physical-login bus by matching the username.
    ensure!(alive(&lifetime), "owned compositor exited");
    let mut target = SessionTarget::wayland(socket.to_path_buf(), runtime.to_path_buf());
    target.bus_address = Some(bus);
    Ok((target, lifetime))
}

pub(crate) fn alive(lifetime: &File) -> bool {
    let mut fd = libc::pollfd {
        fd: lifetime.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut fd, 1, 0) == 0 }
}

/// Resolve only an XWayland child of the admitted compositor. This is used for
/// clipboard and focus compatibility; capture/input stay on the Wayland session.
#[cfg(feature = "portal")]
pub(crate) fn xwayland_target(compositor: u32, runtime: &Path) -> Result<SessionTarget> {
    let uid = unsafe { libc::geteuid() };
    let tasks = fs::read_dir(format!("/proc/{compositor}/task"))?;
    for task in tasks.take(128).filter_map(Result::ok) {
        let children = fs::read_to_string(task.path().join("children")).unwrap_or_default();
        for child in children.split_whitespace().take(128) {
            let pid: u32 = child.parse()?;
            let process = PathBuf::from(format!("/proc/{pid}"));
            if !fs::read_link(process.join("exe"))
                .is_ok_and(|exe| exe.file_name().is_some_and(|name| name == "Xwayland"))
            {
                continue;
            }
            ensure!(
                fs::metadata(&process)?.uid() == uid,
                "XWayland belongs to another user"
            );
            let command = fs::read(process.join("cmdline"))?;
            ensure!(
                command.len() <= 128 * 1024,
                "XWayland command line too large"
            );
            let args: Vec<_> = command.split(|byte| *byte == 0).collect();
            let display = std::str::from_utf8(args.get(1).context("XWayland display missing")?)?;
            crate::session::validate_local_x11_display(display)?;
            let authority = args
                .windows(2)
                .find(|pair| pair[0] == b"-auth")
                .context("XWayland authorization missing")?[1];
            let authority = PathBuf::from(std::str::from_utf8(authority)?);
            ensure!(
                authority.parent() == Some(runtime),
                "XWayland authorization is outside the owned runtime"
            );
            let number = display
                .trim_start_matches(':')
                .split('.')
                .next()
                .unwrap_or_default();
            let stream = UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))?;
            let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
            let mut length = size_of::<libc::ucred>() as libc::socklen_t;
            let result = unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut peer as *mut libc::ucred).cast(),
                    &mut length,
                )
            };
            let inherited_owner = peer.pid as u32 == compositor
                || kwin_wrapper_parent(compositor).is_some_and(|parent| parent == peer.pid as u32);
            ensure!(
                result == 0
                    && peer.uid == uid
                    && (peer.pid as u32 == pid
                        || (inherited_owner
                            && args.windows(2).any(|pair| pair[0] == b"-listenfd"))),
                "XWayland socket does not match the owned compositor child"
            );
            return Ok(SessionTarget {
                runtime: runtime.to_path_buf(),
                x11_display: Some(display.to_owned()),
                xauthority: Some(authority),
                ..Default::default()
            });
        }
    }
    anyhow::bail!("owned compositor has no XWayland clipboard bridge")
}

/// The session-local GNOME adapter exports application IDs only, never titles
/// or document contents. Reject stale files and files outside its owner scope.
#[cfg(feature = "portal")]
pub(crate) fn focused_app(runtime: &Path, compositor: u32) -> Option<String> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(runtime.join("lightos-focus.json"))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 4096 || metadata.uid() != unsafe { libc::geteuid() }
    {
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value["compositor_pid"].as_u64() != Some(u64::from(compositor)) {
        return None;
    }
    let app = value["app_id"].as_str().unwrap_or_default();
    let class = value["wm_class"].as_str().unwrap_or_default();
    Some(
        if crate::backend::terminal_app(class) {
            class
        } else {
            app
        }
        .chars()
        .take(256)
        .collect(),
    )
}

// KWin's wrapper opens both sockets before exec/spawning its compositor. This
// exception only accepts that exact same-user parent in the same service.
#[cfg(feature = "portal")]
fn kwin_wrapper_parent(compositor: u32) -> Option<u32> {
    let process = PathBuf::from(format!("/proc/{compositor}"));
    if fs::read_link(process.join("exe")).ok()?.file_name()? != "kwin_wayland" {
        return None;
    }
    let status = fs::read_to_string(process.join("status")).ok()?;
    let parent: u32 = status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))?
        .trim()
        .parse()
        .ok()?;
    let wrapper = PathBuf::from(format!("/proc/{parent}"));
    (fs::read_link(wrapper.join("exe")).ok()?.file_name()? == "kwin_wayland_wrapper"
        && fs::metadata(&wrapper).ok()?.uid() == unsafe { libc::geteuid() }
        && fs::read_to_string(wrapper.join("cgroup")).ok()?
            == fs::read_to_string(process.join("cgroup")).ok()?)
    .then_some(parent)
}
