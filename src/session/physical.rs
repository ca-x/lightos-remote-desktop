//! Resolve a same-user physical login without consulting virtual-session settings.
use super::{
    is_owned_socket, validate_session_runtime, validate_wayland_socket_for_owner, wayland_sockets,
    x11_target,
};
use anyhow::{Context, Result, ensure};
use display_control::SessionTarget;
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
    time::Duration,
};
use zbus::zvariant::OwnedObjectPath;

pub(super) fn target(runtime: &Path, socket: Option<&Path>) -> Result<SessionTarget> {
    let owner = unsafe { libc::geteuid() };
    if let Some(socket) = socket {
        validate_wayland_socket_for_owner(socket, owner)?;
        return Ok(SessionTarget::wayland(
            socket.into(),
            socket.parent().unwrap().into(),
        ));
    }
    validate_session_runtime(runtime, owner)?;
    let sockets = wayland_sockets(runtime, owner)?;
    let x11 = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(3), x11_logins(owner))
                .await
                .context("physical login discovery timed out")?
        })?;
    ensure!(
        sockets.len() + x11.len() == 1,
        "物理桌面会话不可用或不唯一：发现 {} 个 Wayland 会话、{} 个 X11 登录会话；请先登录物理桌面，多个会话时显式指定物理目标",
        sockets.len(),
        x11.len()
    );
    if let Some(socket) = sockets.into_iter().next() {
        return Ok(SessionTarget::wayland(socket, runtime.into()));
    }
    let login = &x11[0];
    let authority = login_authority(login, owner)?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut target = x11_target(&login.display, authority.as_deref(), home.as_deref(), owner)?;
    target.runtime = runtime.into();
    if is_owned_socket(&runtime.join("bus"), owner) {
        target.bus_address = Some(
            zbus::address::transport::Unix::new(zbus::address::transport::UnixSocket::File(
                runtime.join("bus"),
            ))
            .to_string(),
        );
    }
    Ok(target)
}

struct X11Login {
    id: String,
    display: String,
    scope: String,
}

async fn x11_logins(owner: u32) -> Result<Vec<X11Login>> {
    if !Path::new("/run/dbus/system_bus_socket").exists()
        && std::env::var_os("DBUS_SYSTEM_BUS_ADDRESS").is_none()
    {
        return Ok(Vec::new());
    }
    let connection = zbus::Connection::system().await?;
    let manager = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> =
        manager.call("ListSessions", &()).await?;
    let mut logins = Vec::new();
    for (id, uid, _, _, path) in sessions {
        if uid != owner {
            continue;
        }
        let session = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            path.as_str(),
            "org.freedesktop.login1.Session",
        )
        .await?;
        let kind: String = session.get_property("Type").await?;
        let remote: bool = session.get_property("Remote").await?;
        let state: String = session.get_property("State").await?;
        if kind != "x11" || remote || state == "closing" {
            continue;
        }
        let display: String = session.get_property("Display").await?;
        super::validate_local_x11_display(&display)?;
        logins.push(X11Login {
            id,
            display,
            scope: session.get_property("Scope").await?,
        });
    }
    Ok(logins)
}

// LightDM/SDDM/GDM can put the authority outside ~/.Xauthority. Read only
// same-user processes belonging to the selected login, never the transport env.
fn login_authority(login: &X11Login, owner: u32) -> Result<Option<PathBuf>> {
    let mut authorities = BTreeSet::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let process = entry.path();
        let read = || -> Result<Option<PathBuf>> {
            let file = fs::File::open(process.join("environ"))?;
            if file.metadata()?.uid() != owner {
                return Ok(None);
            }
            let mut environment = Vec::new();
            file.take(128 * 1024 + 1).read_to_end(&mut environment)?;
            ensure!(
                environment.len() <= 128 * 1024,
                "login process environment is too large"
            );
            let value = |key: &[u8]| {
                environment
                    .split(|byte| *byte == 0)
                    .find_map(|entry| entry.strip_prefix(key))
            };
            let Some(display) =
                value(b"DISPLAY=").and_then(|bytes| std::str::from_utf8(bytes).ok())
            else {
                return Ok(None);
            };
            if display.split('.').next() != login.display.split('.').next() {
                return Ok(None);
            }
            let exact_id = value(b"XDG_SESSION_ID=") == Some(login.id.as_bytes());
            let in_scope = !login.scope.is_empty()
                && fs::read_to_string(process.join("cgroup"))?
                    .lines()
                    .any(|line| {
                        line.split(':')
                            .nth(2)
                            .is_some_and(|group| group.split('/').any(|part| part == login.scope))
                    });
            if !exact_id && !in_scope {
                return Ok(None);
            }
            Ok(value(b"XAUTHORITY=")
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(std::ffi::OsString::from_vec(value.into()))))
        };
        // Processes can exit during discovery, and other users' environments
        // are intentionally unreadable. Selected authority files are validated below.
        if let Ok(Some(authority)) = read() {
            x11_target(&login.display, Some(&authority), None, owner)?;
            authorities.insert(authority);
        }
    }
    ensure!(
        authorities.len() <= 1,
        "物理 X11 登录会话存在多个授权文件，无法确定桌面目标"
    );
    Ok(authorities.into_iter().next())
}
