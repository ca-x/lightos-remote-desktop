use std::{
    fs,
    io::Write,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use zbus::address::{Address, Transport, transport::UnixSocket};

#[derive(Serialize)]
struct Environment {
    wayland_display: String,
    niri_socket: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    display: Option<String>,
}

fn private_runtime(runtime: &Path) -> Result<u32> {
    let owner = unsafe { libc::geteuid() };
    ensure!(owner != 0, "Niri environment publisher must run as a non-root user");
    ensure!(
        runtime.is_absolute()
            && runtime.components().all(|component| {
                matches!(component, Component::RootDir | Component::Normal(_))
            }),
        "Niri runtime must be an absolute path without traversal"
    );
    let metadata = fs::symlink_metadata(runtime).context("inspect private Niri runtime")?;
    ensure!(
        metadata.is_dir() && metadata.uid() == owner && metadata.mode() & 0o777 == 0o700,
        "Niri runtime must be a private directory (0700) owned by the current user"
    );
    let resolved = fs::canonicalize(runtime).context("resolve private Niri runtime")?;
    ensure!(resolved == runtime, "Niri runtime must not contain symlink aliases");
    ensure!(
        resolved != PathBuf::from(format!("/run/user/{owner}")),
        "Niri runtime must not be the physical login runtime"
    );
    let inherited = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?;
    ensure!(
        Path::new(&inherited) == runtime,
        "XDG_RUNTIME_DIR must match the private Niri runtime"
    );
    Ok(owner)
}

fn private_bus(runtime: &Path, owner: u32) -> Result<String> {
    let value = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .context("private session bus address is required")?;
    ensure!(!value.contains(';'), "private session bus cannot have fallback addresses");
    let address: Address = value.parse().context("invalid private session bus address")?;
    let expected = runtime.join("bus");
    ensure!(
        matches!(address.transport(), Transport::Unix(unix) if matches!(unix.path(), UnixSocket::File(path) if path == &expected))
            && value == address.to_string()
            && super::is_owned_socket(&expected, owner),
        "private session bus must use the owned runtime bus socket"
    );
    Ok(value)
}

fn environment(runtime: &Path, owner: u32) -> Result<Environment> {
    let wayland_display = std::env::var("WAYLAND_DISPLAY").context("WAYLAND_DISPLAY is required")?;
    ensure!(
        !wayland_display.is_empty()
            && wayland_display != "."
            && wayland_display != ".."
            && !wayland_display.contains('/')
            && !wayland_display.chars().any(char::is_control)
            && super::is_owned_socket(&runtime.join(&wayland_display), owner),
        "WAYLAND_DISPLAY must name an owned socket in the private runtime"
    );
    let niri_value = std::env::var("NIRI_SOCKET").context("NIRI_SOCKET is required")?;
    let niri_socket = PathBuf::from(&niri_value);
    ensure!(
        niri_socket.is_absolute()
            && niri_socket.parent() == Some(runtime)
            && !niri_value.chars().any(char::is_control)
            && super::is_owned_socket(&niri_socket, owner),
        "NIRI_SOCKET must name an owned socket in the private runtime"
    );
    let display = std::env::var("DISPLAY").ok().filter(|value| !value.is_empty());
    if let Some(display) = &display {
        super::validate_local_x11_display(display)?;
    }
    Ok(Environment {
        wayland_display,
        niri_socket,
        display,
    })
}

fn activate(runtime: &Path, bus: &str, environment: &Environment) -> Result<()> {
    let program = platform_detect::find_executable(Path::new("dbus-update-activation-environment"))
        .context("private session bus activator is unavailable")?;
    let program = fs::canonicalize(program).context("resolve private session bus activator")?;
    let mut child = Command::new(program)
        .env_clear()
        .env("XDG_RUNTIME_DIR", runtime)
        .env("DBUS_SESSION_BUS_ADDRESS", bus)
        .args([
            format!("WAYLAND_DISPLAY={}", environment.wayland_display),
            format!("NIRI_SOCKET={}", environment.niri_socket.display()),
            format!("DISPLAY={}", environment.display.as_deref().unwrap_or_default()),
            format!("XDG_RUNTIME_DIR={}", runtime.display()),
            "XDG_CURRENT_DESKTOP=niri".into(),
            "XDG_SESSION_DESKTOP=niri".into(),
            "XDG_SESSION_TYPE=wayland".into(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .context("start private session bus activation")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let completion = (|| -> Result<()> {
        while !super::child_exited(&child)? {
            ensure!(Instant::now() < deadline, "private session bus activation timed out");
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    })();
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let status = child.wait().context("reap private session bus activator")?;
    completion?;
    ensure!(status.success(), "private session bus activation failed");
    Ok(())
}

pub fn publish_environment(runtime: &Path) -> Result<()> {
    let owner = private_runtime(runtime)?;
    let bus = private_bus(runtime, owner)?;
    let environment = environment(runtime, owner)?;
    let record = runtime.join("niri-environment.json");
    ensure!(
        fs::symlink_metadata(&record).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "Niri readiness record already exists or cannot be inspected"
    );
    activate(runtime, &bus, &environment)?;
    private_runtime(runtime)?;
    let mut temporary = tempfile::NamedTempFile::new_in(runtime)?;
    temporary.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut temporary, &environment)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(record).map_err(|error| error.error)?;
    Ok(())
}
