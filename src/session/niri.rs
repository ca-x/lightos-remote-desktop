use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::UnixStream,
            process::CommandExt,
        },
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use zbus::address::{Address, Transport, transport::UnixSocket};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    wayland_display: String,
    niri_socket: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    display: Option<String>,
}

fn private_directory(runtime: &Path) -> Result<u32> {
    let owner = unsafe { libc::geteuid() };
    ensure!(
        owner != 0,
        "Niri environment publisher must run as a non-root user"
    );
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
    ensure!(
        resolved == runtime,
        "Niri runtime must not contain symlink aliases"
    );
    ensure!(
        resolved != format!("/run/user/{owner}"),
        "Niri runtime must not be the physical login runtime"
    );
    Ok(owner)
}

fn private_runtime(runtime: &Path) -> Result<u32> {
    let owner = private_directory(runtime)?;
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
    ensure!(
        !value.contains(';'),
        "private session bus cannot have fallback addresses"
    );
    let address: Address = value
        .parse()
        .context("invalid private session bus address")?;
    if let Some(guid) = address.guid() {
        ensure!(
            guid.len() == 32 && guid.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "private session bus GUID must contain exactly 32 ASCII hexadecimal digits"
        );
    }
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
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").context("WAYLAND_DISPLAY is required")?;
    let niri_value = std::env::var("NIRI_SOCKET").context("NIRI_SOCKET is required")?;
    let niri_socket = PathBuf::from(&niri_value);
    let display = std::env::var("DISPLAY")
        .ok()
        .filter(|value| !value.is_empty());
    let environment = Environment {
        wayland_display,
        niri_socket,
        display,
    };
    environment.validate(runtime, owner)?;
    Ok(environment)
}

impl Environment {
    fn validate(&self, runtime: &Path, owner: u32) -> Result<()> {
        ensure!(
            !self.wayland_display.is_empty()
                && self.wayland_display != "."
                && self.wayland_display != ".."
                && !self.wayland_display.contains('/')
                && !self.wayland_display.chars().any(char::is_control)
                && super::is_owned_socket(&runtime.join(&self.wayland_display), owner),
            "WAYLAND_DISPLAY must name an owned socket in the private runtime"
        );
        ensure!(
            self.niri_socket.is_absolute()
                && self.niri_socket.parent() == Some(runtime)
                && self.niri_socket.to_str().is_some_and(|value| !value.chars().any(char::is_control))
                && super::is_owned_socket(&self.niri_socket, owner),
            "NIRI_SOCKET must name an owned socket in the private runtime"
        );
        if let Some(display) = &self.display {
            super::validate_local_x11_display(display)?;
        }
        Ok(())
    }
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
            format!(
                "DISPLAY={}",
                environment.display.as_deref().unwrap_or_default()
            ),
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
            ensure!(
                Instant::now() < deadline,
                "private session bus activation timed out"
            );
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
        fs::symlink_metadata(&record)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "Niri readiness record already exists or cannot be inspected"
    );
    activate(runtime, &bus, &environment)?;
    private_runtime(runtime)?;
    let mut temporary = tempfile::NamedTempFile::new_in(runtime)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut temporary, &environment)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(record)
        .map_err(|error| error.error)?;
    Ok(())
}

pub(super) fn isolate(command: &mut Command) {
    command.env_clear();
    for name in [
        "HOME", "USER", "LOGNAME", "PATH", "LANG", "LANGUAGE", "LC_ALL",
        "LC_CTYPE", "LC_MESSAGES", "TZ", "XDG_CONFIG_HOME", "XDG_CONFIG_DIRS",
        "XDG_DATA_HOME", "XDG_DATA_DIRS", "XDG_CACHE_HOME", "LIBGL_ALWAYS_SOFTWARE",
        "GALLIUM_DRIVER", "MESA_LOADER_DRIVER_OVERRIDE", "WLR_RENDERER",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

fn executable(program: &Path) -> Result<PathBuf> {
    let resolved = platform_detect::find_executable(program)
        .with_context(|| format!("missing native desktop executable: {}", program.display()))?;
    fs::canonicalize(resolved).context("resolve native desktop executable")
}

fn connect_socket(path: &Path) -> std::io::Result<UnixStream> {
    let address = socket2::SockAddr::unix(path)?;
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    socket.connect_timeout(&address, Duration::from_millis(750))?;
    Ok(socket.into())
}

fn peer(path: &Path, owner: u32) -> Result<(UnixStream, libc::ucred)> {
    ensure!(super::is_owned_socket(path, owner), "Niri endpoint is not an owned socket");
    let stream = connect_socket(path).context("connect owned Niri endpoint")?;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(), &mut length,
        )
    };
    ensure!(
        result == 0 && length as usize == std::mem::size_of_val(&credentials)
            && credentials.uid == owner && credentials.pid > 1,
        "Niri endpoint has a foreign kernel peer"
    );
    Ok((stream, credentials))
}

struct ProcessIdentity {
    parent: u32,
    group: u32,
    start: String,
}

fn process_identity(pid: u32) -> Result<ProcessIdentity> {
    let process = PathBuf::from(format!("/proc/{pid}"));
    ensure!(fs::metadata(&process)?.uid() == unsafe { libc::geteuid() }, "foreign Niri process owner");
    let stat = fs::read_to_string(process.join("stat"))?;
    let fields: Vec<_> = stat.rsplit_once(") ").context("invalid Niri process identity")?
        .1.split_whitespace().collect();
    ensure!(fields.len() > 19 && !matches!(fields[0], "Z" | "X"), "Niri process has exited");
    Ok(ProcessIdentity {
        parent: fields[1].parse()?, group: fields[2].parse()?, start: fields[19].to_owned(),
    })
}

fn read_environment(runtime: &Path) -> Result<Option<Environment>> {
    let mut file = match OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(runtime.join("niri-environment.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("open private Niri readiness"),
    };
    let metadata = file.metadata()?;
    ensure!(metadata.is_file() && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o077 == 0 && metadata.len() <= 4096,
        "Niri readiness must be a private owned regular file smaller than 4 KiB");
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(4097).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4096, "Niri readiness is too large");
    Ok(Some(serde_json::from_slice(&bytes).context("invalid Niri readiness record")?))
}

fn admit(runtime: &Path, environment: &Environment, niri: &fs::Metadata, cage: u32) -> Result<(PathBuf, File)> {
    let owner = private_directory(runtime)?;
    environment.validate(runtime, owner)?;
    let socket = runtime.join(&environment.wayland_display);
    let (_wayland, wayland_peer) = peer(&socket, owner)?;
    let (_ipc, ipc_peer) = peer(&environment.niri_socket, owner)?;
    ensure!(wayland_peer.pid == ipc_peer.pid, "Wayland and IPC must have the same Niri kernel peer");
    let pid = wayland_peer.pid as u32;
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32 };
    ensure!(raw >= 0, "could not pin selected Niri process");
    let lifetime = unsafe { File::from_raw_fd(raw) };
    let identity = process_identity(pid)?;
    let actual = fs::metadata(format!("/proc/{pid}/exe"))?;
    ensure!((actual.dev(), actual.ino()) == (niri.dev(), niri.ino()), "endpoint is not the selected Niri executable");
    ensure!(identity.group == cage, "Niri process is outside the owned Cage process group");
    let mut ancestor = identity.parent;
    for _ in 0..32 {
        if ancestor == cage || ancestor <= 1 {
            break;
        }
        ancestor = process_identity(ancestor)?.parent;
    }
    ensure!(pid != cage && ancestor == cage, "Niri process is not a controlled Cage descendant");
    ensure!(process_identity(pid)?.start == identity.start && crate::owned_wayland::alive(&lifetime), "selected Niri process changed during admission");
    tracing::info!(pid, cage, socket = %socket.display(), "owned Niri process admitted");
    Ok((socket, lifetime))
}

fn discard_record(runtime: &Path, name: &str) -> Result<()> {
    let path = runtime.join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(metadata.is_file() && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o077 == 0, "refusing foreign or nonprivate previous Niri record");
    fs::remove_file(path)?;
    Ok(())
}

pub(super) fn start(config: &crate::config::Config, cancelled: Arc<AtomicBool>) -> Result<super::Session> {
    let cage = executable(&config.cage)?;
    let niri = executable(Path::new("niri"))?;
    let niri_identity = fs::metadata(&niri)?;
    let prepared = crate::desktop::prepare(config, config.runtime_dir.as_deref().unwrap_or(Path::new("/tmp")))?;
    let temporary = if config.runtime_dir.is_none() {
        Some(tempfile::Builder::new().prefix("lrd.niri.")
            .permissions(fs::Permissions::from_mode(0o700)).tempdir()?)
    } else { None };
    let runtime = config.runtime_dir.clone().unwrap_or_else(|| temporary.as_ref().unwrap().path().to_owned());
    ensure!(runtime.is_absolute() && runtime.components().all(|component| matches!(component, Component::RootDir | Component::Normal(_))), "Niri runtime must be absolute without traversal");
    ensure!(runtime != format!("/run/user/{}", unsafe { libc::geteuid() }), "Niri runtime must not be the physical login runtime");
    let parent = runtime.parent().context("Niri runtime requires a parent")?;
    ensure!(fs::canonicalize(parent)? == parent, "Niri runtime parent must not contain symlink aliases");
    super::validate_niri_runtime_socket_paths(&runtime)?;
    super::secure_directory(&runtime)?;
    private_directory(&runtime)?;
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(runtime.join("session.lock"))?;
    let metadata = lock.metadata()?;
    ensure!(metadata.is_file() && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o077 == 0, "Niri owner lock must be an owned private regular file");
    ensure!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another desktop owns this runtime directory");
    let mut session = super::Session {
        socket: PathBuf::new(), runtime, signature: None,
        pulse_server: None, pulse_client_config: None, audio_source: None,
        target: None, owned: true, children: Vec::new(), temporary,
        lock: Some(lock), native_lifetime: None, cancelled,
    };
    ensure!(!session.cancelled.load(Ordering::Acquire), "native desktop startup cancelled");
    discard_record(&session.runtime, "niri-environment.json")?;
    discard_record(&session.runtime, "session.json")?;
    let bus_path = session.runtime.join("bus");
    if fs::symlink_metadata(&bus_path).is_ok() {
        ensure!(super::is_owned_socket(&bus_path, unsafe { libc::geteuid() }), "previous private bus is not an owned socket");
        match connect_socket(&bus_path) {
            Ok(_) => bail!("previous private session bus is still active"),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => fs::remove_file(&bus_path)?,
            Err(error) => return Err(error).context("inspect previous private session bus"),
        }
    }
    let applications = crate::applications::prepare(&session.runtime, None)?;
    let bus_address = format!("unix:path={}", bus_path.display());
    let mut bus = Command::new(executable(Path::new("dbus-daemon"))?);
    isolate(&mut bus);
    applications.apply(&mut bus);
    bus.args(["--session", "--nofork", "--nopidfile"])
        .arg(format!("--address={bus_address}"))
        .env("XDG_RUNTIME_DIR", &session.runtime).env("DBUS_SESSION_BUS_ADDRESS", &bus_address)
        .env("XDG_CURRENT_DESKTOP", "niri").env("XDG_SESSION_TYPE", "wayland");
    session.spawn(bus, "dbus.log")?;
    session.wait_for(Duration::from_secs(5), || bus_path.exists())?;
    let (_bus_stream, bus_peer) = peer(&bus_path, unsafe { libc::geteuid() })?;
    ensure!(bus_peer.pid as u32 == session.children[0].id(), "private bus is not the owned daemon");
    if config.audio {
        session.pulse_server = Some(format!("unix:{}/pulse/native", session.runtime.display()));
        session.setup_audio(crate::config::Compositor::Niri)?;
    }
    let mut command = Command::new(cage);
    isolate(&mut command);
    command.arg("--").arg(niri).arg("--config").arg(prepared.configuration)
        .arg("--").arg(std::env::current_exe()?).arg("niri-session-environment")
        .arg("--runtime").arg(&session.runtime)
        .env("XDG_RUNTIME_DIR", &session.runtime).env("DBUS_SESSION_BUS_ADDRESS", &bus_address)
        .env("WLR_BACKENDS", "headless").env("WLR_HEADLESS_OUTPUTS", "1")
        .env("WLR_RENDERER", std::env::var_os("WLR_RENDERER").unwrap_or_else(|| "pixman".into()))
        .env("XDG_CURRENT_DESKTOP", "niri").env("XDG_SESSION_DESKTOP", "niri")
        .env("XDG_SESSION_TYPE", "wayland");
    applications.apply(&mut command);
    session.audio_environment(&mut command);
    session.spawn(command, "desktop.log")?;
    let cage_pid = session.children.last().unwrap().id();
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        ensure!(!session.cancelled.load(Ordering::Acquire), "native desktop startup cancelled");
        ensure!(session.children.iter().all(|child| matches!(super::child_exited(child), Ok(false))),
            "native desktop dependency exited: {}", session.log_tail());
        if let Some(environment) = read_environment(&session.runtime)? {
            let (socket, lifetime) = admit(&session.runtime, &environment, &niri_identity, cage_pid)?;
            session.socket = socket;
            session.native_lifetime = Some(lifetime);
            break;
        }
        ensure!(Instant::now() < deadline, "native desktop readiness timed out: {}", session.log_tail());
        thread::sleep(Duration::from_millis(50));
    }
    let socket = session.socket.clone();
    let cancelled = session.cancelled.clone();
    session.wait_for(Duration::from_secs(10), || {
        crate::wayland::capabilities::probe(&socket, Duration::from_millis(250), Some(&cancelled))
            .is_ok_and(|capabilities| capabilities.capture)
    })?;
    ensure!(session.healthy(), "native desktop exited before handoff");
    if let Err(error) = crate::input_method::start(&session.runtime, &session.socket) {
        tracing::warn!(%error, "owned desktop input method unavailable");
    }
    Ok(session)
}
