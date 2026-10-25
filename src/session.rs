use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{SocketAddr as UnixSocketAddr, UnixStream},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use display_control::SessionTarget;

use crate::config::Config;

pub const DEFAULT_SESSION_RUNTIME: &str = "/run/lrd-session";
pub const PREVIOUS_SESSION_RUNTIME: &str = "/run/lightos-remoted-session";

/// Reserve 64 bytes for Hyprland's commit/timestamp/random instance signature.
pub(crate) fn validate_runtime_socket_paths(runtime: &Path) -> Result<()> {
    let socket = runtime
        .join("hypr")
        .join("x".repeat(64))
        .join(".socket2.sock");
    UnixSocketAddr::from_pathname(&socket).with_context(|| {
        format!("runtime directory {} is too long for Hyprland IPC sockets; use {DEFAULT_SESSION_RUNTIME} or a shorter --runtime-dir", runtime.display())
    })?;
    Ok(())
}

pub struct Session {
    pub socket: PathBuf,
    pub runtime: PathBuf,
    pub signature: Option<String>,
    pub pulse_server: Option<String>,
    pub pulse_client_config: Option<PathBuf>,
    pub audio_source: Option<String>,
    target: Option<SessionTarget>,
    owned: bool,
    children: Vec<Child>,
    temporary: Option<tempfile::TempDir>,
    lock: Option<File>,
    cancelled: Arc<AtomicBool>,
}

impl Session {
    pub(crate) fn attached(
        runtime: PathBuf,
        socket: PathBuf,
        signature: Option<String>,
        pulse_server: Option<String>,
        pulse_client_config: Option<PathBuf>,
        audio_source: Option<String>,
    ) -> Self {
        Self {
            runtime,
            socket,
            signature,
            pulse_server,
            pulse_client_config,
            audio_source,
            target: None,
            owned: true,
            children: vec![],
            temporary: None,
            lock: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn physical_socket(config: &Config) -> Result<PathBuf> {
        physical_socket_from_environment(
            config,
            unsafe { libc::geteuid() },
            std::env::var_os("XDG_RUNTIME_DIR")
                .as_deref()
                .map(Path::new),
            std::env::var_os("WAYLAND_DISPLAY")
                .as_deref()
                .map(Path::new),
        )
    }

    pub fn physical_target(config: &Config) -> Result<SessionTarget> {
        let owner = unsafe { libc::geteuid() };
        let login_runtime = PathBuf::from(format!("/run/user/{owner}"));
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let wayland = std::env::var_os("WAYLAND_DISPLAY").map(PathBuf::from);
        let xauthority = std::env::var_os("XAUTHORITY")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        physical_target_from_environment(
            config,
            owner,
            physical_discovery_runtime(
                config,
                runtime.as_deref(),
                wayland.as_deref(),
                &login_runtime,
            ),
            wayland.as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
            xauthority.as_deref(),
            home.as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        )
    }

    pub fn target(&self) -> SessionTarget {
        self.target
            .clone()
            .unwrap_or_else(|| SessionTarget::wayland(self.socket.clone(), self.runtime.clone()))
    }

    pub fn is_owned(&self) -> bool {
        self.owned
    }

    pub fn start(config: &Config) -> Result<Self> {
        Self::start_cancellable(config, Arc::new(AtomicBool::new(false)))
    }

    pub fn start_cancellable(config: &Config, cancelled: Arc<AtomicBool>) -> Result<Self> {
        ensure!(
            unsafe { libc::geteuid() } != 0,
            "desktop service must run as a non-root user"
        );
        if config.x11_display.is_some() {
            let target = Self::physical_target(config)?;
            let (pulse_server, audio_source) = physical_audio(&target.runtime, config.audio);
            return Ok(Self {
                socket: PathBuf::new(),
                runtime: target.runtime.clone(),
                signature: None,
                pulse_server,
                pulse_client_config: None,
                audio_source,
                target: Some(target),
                owned: false,
                children: Vec::new(),
                temporary: None,
                lock: None,
                cancelled,
            });
        }
        if config.wayland_socket.is_none()
            && let Some(runtime) = &config.session_runtime
        {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                ensure!(
                    !cancelled.load(Ordering::Acquire),
                    "persistent desktop connection cancelled"
                );
                if let Some(session) = crate::handoff::attach_persistent(runtime)? {
                    // Refresh session-local launchers without stopping the owner
                    // or its applications during a transport upgrade.
                    let applications = crate::applications::prepare(
                        &session.runtime,
                        config.omarchy_path()?.as_deref(),
                    )?;
                    applications.update_activation(&session.runtime)?;
                    return Ok(session);
                }
                ensure!(
                    Instant::now() < deadline,
                    "persistent desktop owner did not become ready"
                );
                thread::sleep(Duration::from_millis(100));
            }
        }
        if let Some(socket) = &config.wayland_socket {
            let runtime = socket
                .parent()
                .context("Wayland socket has no parent")?
                .to_path_buf();
            validate_wayland_socket(socket)?;
            let target = SessionTarget::wayland(socket.clone(), runtime.clone());
            let (pulse_server, audio_source) = physical_audio(&runtime, config.audio);
            return Ok(Self {
                socket: socket.clone(),
                runtime,
                signature: None,
                pulse_server,
                pulse_client_config: None,
                audio_source,
                target: Some(target),
                owned: false,
                children: Vec::new(),
                temporary: None,
                lock: None,
                cancelled,
            });
        }
        let temporary = if config.runtime_dir.is_none() {
            Some(
                tempfile::Builder::new()
                    .prefix("lrd.")
                    .permissions(fs::Permissions::from_mode(0o700))
                    .tempdir_in("/tmp")?,
            )
        } else {
            None
        };
        let runtime = config
            .runtime_dir
            .clone()
            .unwrap_or_else(|| temporary.as_ref().unwrap().path().to_path_buf());
        validate_runtime_socket_paths(&runtime)?;
        secure_directory(&runtime)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(runtime.join("session.lock"))?;
        ensure!(
            unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&lock),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            } == 0,
            "another desktop owns this runtime directory"
        );
        let mut session = Self {
            socket: PathBuf::new(),
            pulse_server: Some(format!("unix:{}/pulse/native", runtime.display())),
            runtime,
            signature: None,
            pulse_client_config: None,
            audio_source: None,
            target: None,
            owned: true,
            children: Vec::new(),
            temporary,
            lock: Some(lock),
            cancelled,
        };
        let bus_path = session.runtime.join("bus");
        let mut bus = Command::new("dbus-daemon");
        bus.args(["--session", "--nofork", "--nopidfile"])
            .arg(format!("--address=unix:path={}", bus_path.display()))
            .env("XDG_RUNTIME_DIR", &session.runtime)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("CHROME_CONFIG_HOME")
            .env_remove("CHROME_USER_DATA_DIR");
        session.spawn(bus, "dbus.log")?;
        session.wait_for(Duration::from_secs(5), || bus_path.exists())?;
        if config.audio {
            session
                .setup_audio()
                .context("starting private desktop audio")?;
        }
        let prepared = crate::desktop::prepare(config, &session.runtime)?;
        let omarchy = prepared.omarchy;
        let applications = crate::applications::prepare(&session.runtime, omarchy.as_deref())?;
        let desktop_config = prepared.configuration;
        let lua = desktop_config
            .extension()
            .is_some_and(|extension| extension == "lua");
        let mut command = Command::new(&config.cage);
        command.arg("--");
        if let Some(directory) = &config.aquamarine_dir {
            let directory = fs::canonicalize(directory)?;
            ensure!(
                directory.join("libaquamarine.so.14").is_file(),
                "missing private libaquamarine.so.14"
            );
            command
                .arg("/usr/lib/ld-linux-x86-64.so.2")
                .arg("--library-path")
                .arg(format!("{}:/usr/lib", directory.display()));
        }
        let hyprland = std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join("Hyprland"))
                    .find(|path| path.is_file())
            })
            .context("Hyprland not found in PATH")?;
        command
            .arg(hyprland)
            .arg("--config")
            .arg(desktop_config)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env("XDG_RUNTIME_DIR", &session.runtime)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", bus_path.display()),
            )
            .env("WLR_BACKENDS", "headless")
            .env("WLR_HEADLESS_OUTPUTS", "1")
            .env("WLR_RENDERER", "gles2")
            .env("WLR_RENDER_DRM_DEVICE", "/dev/dri/renderD128")
            .env("XDG_CURRENT_DESKTOP", "Hyprland")
            .env("XDG_SESSION_TYPE", "wayland");
        command.envs(prepared.environment.iter().cloned());
        applications.apply(&mut command);
        session.audio_environment(&mut command);
        if let Some(path) = &omarchy {
            session.omarchy_environment(&mut command, path)?;
        }
        if let Some(source) = &session.audio_source {
            command.env("PULSE_SINK", source.trim_end_matches(".monitor"));
        }
        session.spawn(command, "desktop.log")?;
        let runtime = session.runtime.clone();
        session.wait_for(Duration::from_secs(20), || {
            desktop_socket(&runtime).is_some()
        })?;
        session.socket =
            desktop_socket(&session.runtime).context("nested Wayland socket disappeared")?;
        session.signature = fs::read_dir(session.runtime.join("hypr"))?
            .filter_map(Result::ok)
            .find(|entry| entry.path().join(".socket.sock").exists())
            .map(|entry| entry.file_name().to_string_lossy().into_owned());
        let display = session.prepare_application_environment(lua)?;
        if let Some(path) = &omarchy {
            let mut shell = Command::new("quickshell");
            shell
                .arg("--no-duplicate")
                .arg("--path")
                .arg(path.join("shell"));
            shell
                .env("XDG_RUNTIME_DIR", &session.runtime)
                .env(
                    "WAYLAND_DISPLAY",
                    session
                        .socket
                        .file_name()
                        .context("desktop socket has no name")?,
                )
                .env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}", bus_path.display()),
                )
                .env("DISPLAY", &display)
                .env("XDG_CURRENT_DESKTOP", "Hyprland")
                .env("QT_QPA_PLATFORM", "wayland")
                .env("QS_DISABLE_FILE_WATCHER", "1")
                .env("QS_NO_RELOAD_POPUP", "1");
            if let Some(signature) = &session.signature {
                shell.env("HYPRLAND_INSTANCE_SIGNATURE", signature);
            }
            shell.envs(prepared.environment.iter().cloned());
            applications.apply(&mut shell);
            session.audio_environment(&mut shell);
            session.omarchy_environment(&mut shell, path)?;
            session.spawn(shell, "omarchy-shell.log")?;
        }
        tracing::info!(socket = %session.socket.display(), "native desktop session ready");
        Ok(session)
    }

    fn prepare_application_environment(&mut self, lua: bool) -> Result<String> {
        let script = self.runtime.join("session-environment");
        fs::write(&script, include_str!("../scripts/session-environment.sh"))?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
        let initialization = tempfile::Builder::new()
            .prefix("application-environment.")
            .tempdir_in(&self.runtime)?;
        let output = initialization.path().join("display");
        let signature = self
            .signature
            .as_ref()
            .context("missing desktop signature")?;
        let mut socket = UnixStream::connect(
            self.runtime
                .join("hypr")
                .join(signature)
                .join(".socket.sock"),
        )?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        let command = format!("{} {}", quote(&script), quote(&output));
        socket.write_all(desktop_config::exec_dispatch(&command, lua).as_bytes())?;
        let mut response = [0_u8; 256];
        let length = socket.read(&mut response)?;
        ensure!(
            response[..length].starts_with(b"ok"),
            "compositor rejected session environment initialization: {}",
            String::from_utf8_lossy(&response[..length])
        );
        self.wait_for(Duration::from_secs(5), || output.is_file())?;
        let display = fs::read_to_string(output)?;
        Ok(display.trim().to_owned())
    }

    fn omarchy_environment(&self, command: &mut Command, path: &Path) -> Result<()> {
        let mut paths = vec![self.runtime.join("bin"), path.join("bin")];
        if let Some(inherited) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&inherited));
        }
        command
            .env("OMARCHY_PATH", path)
            .env("PATH", std::env::join_paths(paths)?);
        Ok(())
    }

    fn spawn(&mut self, mut command: Command, log: &str) -> Result<()> {
        let output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(self.runtime.join(log))?;
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output);
        self.children
            .push(command.spawn().with_context(|| format!("starting {log}"))?);
        Ok(())
    }

    fn wait_for(&mut self, timeout: Duration, ready: impl Fn() -> bool) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            ensure!(
                !self.cancelled.load(Ordering::Acquire),
                "desktop startup cancelled"
            );
            if self
                .children
                .iter()
                .any(|child| child_exited(child).unwrap_or(true))
            {
                bail!("desktop dependency exited: {}", self.log_tail());
            }
            if ready() {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "desktop startup timed out: {}",
                self.log_tail()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn log_tail(&self) -> String {
        use std::io::{Read, Seek, SeekFrom};
        ["desktop.log", "dbus.log"]
            .into_iter()
            .filter_map(|name| {
                let mut file = File::open(self.runtime.join(name)).ok()?;
                let size = file.metadata().ok()?.len();
                file.seek(SeekFrom::Start(size.saturating_sub(8192))).ok()?;
                let mut bytes = Vec::new();
                file.take(8192).read_to_end(&mut bytes).ok()?;
                Some(format!("{name}: {}", String::from_utf8_lossy(&bytes)))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn setup_audio(&mut self) -> Result<()> {
        let configuration = self.runtime.join("audio-config");
        let fragments = configuration.join("wireplumber/wireplumber.conf.d");
        fs::create_dir_all(&fragments)?;
        fs::write(
            fragments.join("90-lightos.conf"),
            include_str!("../config/90-lightos-audio.conf"),
        )?;
        for (program, arguments, log, ready_socket) in [
            (
                "pipewire",
                Vec::<&str>::new(),
                "pipewire.log",
                Some(self.runtime.join("pipewire-0")),
            ),
            (
                "pipewire-pulse",
                vec![],
                "pipewire-pulse.log",
                Some(self.runtime.join("pulse/native")),
            ),
            (
                "wireplumber",
                vec!["--profile=lightos"],
                "wireplumber.log",
                None,
            ),
        ] {
            let mut command = Command::new(program);
            command
                .args(arguments)
                .env("XDG_RUNTIME_DIR", &self.runtime)
                .env("PIPEWIRE_RUNTIME_DIR", &self.runtime)
                .env("PIPEWIRE_REMOTE", "pipewire-0")
                .env("XDG_CONFIG_HOME", &configuration)
                .env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}/bus", self.runtime.display()),
                )
                .env_remove("PULSE_SERVER")
                .env_remove("PULSE_CLIENTCONFIG");
            self.spawn(command, log)?;
            if let Some(socket) = ready_socket {
                self.wait_for(Duration::from_secs(5), || socket.exists())?;
            }
        }
        let sink = format!("lightos_remote_{}", std::process::id());
        let mut command = Command::new("timeout");
        command
            .args([
                "--kill-after=1",
                "5",
                "pactl",
                "load-module",
                "module-null-sink",
            ])
            .arg(format!("sink_name={sink}"))
            .arg("sink_properties=device.description=LightOS_Remote_Desktop");
        self.audio_environment(&mut command);
        let output = command.output()?;
        ensure!(
            output.status.success(),
            "could not create isolated audio sink: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let module = String::from_utf8(output.stdout)?.trim().to_owned();
        ensure!(
            !module.is_empty() && module.bytes().all(|byte| byte.is_ascii_digit()),
            "invalid audio module identifier"
        );
        self.audio_source = Some(format!("{sink}.monitor"));
        Ok(())
    }

    pub fn healthy(&mut self) -> bool {
        self.children
            .iter()
            .all(|child| matches!(child_exited(child), Ok(false)))
            && (self
                .target
                .as_ref()
                .is_some_and(|target| target.x11_display.is_some())
                || self.socket.exists())
    }

    pub fn audio_environment(&self, command: &mut Command) {
        if let Some(home) = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        {
            command.env("XDG_CONFIG_HOME", home);
        }
        if let Some(server) = &self.pulse_server {
            command.env("PULSE_SERVER", server);
        }
        if let Some(config) = &self.pulse_client_config {
            command.env("PULSE_CLIENTCONFIG", config);
        } else {
            command.env_remove("PULSE_CLIENTCONFIG");
        }
    }
}

fn physical_audio(runtime: &Path, enabled: bool) -> (Option<String>, Option<String>) {
    // Never inherit the transport's private Pulse server or default microphone.
    // PulseAudio and PipeWire-Pulse both expose the selected session's output
    // through this monitor alias. No host device/routing settings are changed.
    let socket = runtime.join("pulse/native");
    if enabled && is_owned_socket(&socket, unsafe { libc::geteuid() }) {
        (
            Some(format!("unix:{}", socket.display())),
            Some("@DEFAULT_MONITOR@".into()),
        )
    } else {
        (None, None)
    }
}

fn physical_socket_from_environment(
    config: &Config,
    owner: u32,
    runtime: Option<&Path>,
    display: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(socket) = config
        .wayland_socket
        .as_ref()
        .or(config.physical_wayland_socket.as_ref())
    {
        validate_wayland_socket_for_owner(socket, owner)?;
        return Ok(socket.clone());
    }
    let display = display.filter(|path| !path.as_os_str().is_empty());
    if let Some(socket) = display.filter(|path| path.is_absolute()) {
        validate_wayland_socket_for_owner(socket, owner)?;
        return Ok(socket.to_path_buf());
    }
    let directory = runtime
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{owner}")));
    validate_session_runtime(&directory, owner)?;
    // A supplied display identifies one exact target, never an arbitrary sibling socket.
    if let Some(display) = display {
        ensure!(
            display.components().count() == 1,
            "relative Wayland display must be a socket name"
        );
        let socket = directory.join(display);
        validate_wayland_socket_for_owner(&socket, owner)?;
        return Ok(socket);
    }
    let sockets = wayland_sockets(&directory, owner)?;
    match sockets.as_slice() {
        [socket] => Ok(socket.clone()),
        [] => bail!(
            "未发现当前用户的物理 Wayland 会话；请先登录本机桌面（服务仅能访问同一 UID 的会话）"
        ),
        _ => bail!("发现多个 Wayland 会话；请用 --physical-wayland-socket 指定物理桌面"),
    }
}

fn wayland_sockets(directory: &Path, owner: u32) -> Result<Vec<PathBuf>> {
    let mut sockets: Vec<_> = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("wayland-"))
                && is_owned_socket(path, owner)
        })
        .collect();
    sockets.sort();
    Ok(sockets)
}

fn physical_discovery_runtime<'a>(
    config: &Config,
    runtime: Option<&'a Path>,
    wayland_display: Option<&Path>,
    login_runtime: &'a Path,
) -> Option<&'a Path> {
    // A persistent desktop's HTTP service has its own RuntimeDirectory. That
    // directory is not the user's graphical login. Preserve an explicit
    // WAYLAND_DISPLAY/runtime pair; otherwise discover the same user's login.
    if config.session_runtime.is_some()
        && !wayland_display.is_some_and(|display| !display.as_os_str().is_empty())
    {
        Some(login_runtime)
    } else {
        runtime
    }
}

#[allow(clippy::too_many_arguments)] // One explicit snapshot keeps session discovery and its tests independent of process-global environment changes.
fn physical_target_from_environment(
    config: &Config,
    owner: u32,
    runtime: Option<&Path>,
    wayland_display: Option<&Path>,
    x11_display: Option<&str>,
    xauthority: Option<&Path>,
    home: Option<&Path>,
    session_type: Option<&str>,
) -> Result<SessionTarget> {
    if let Some(display) = config.x11_display.as_deref() {
        return x11_target(
            display,
            config.xauthority.as_deref().or(xauthority),
            home,
            owner,
        );
    }
    let x11_display = x11_display.filter(|display| !display.is_empty());
    if config.wayland_socket.is_some()
        || config.physical_wayland_socket.is_some()
        || wayland_display.is_some_and(|display| !display.as_os_str().is_empty())
    {
        let socket = physical_socket_from_environment(config, owner, runtime, wayland_display)?;
        return Ok(SessionTarget::wayland(
            socket.clone(),
            socket.parent().unwrap().to_path_buf(),
        ));
    }
    let wayland_session = session_type.is_some_and(|kind| kind.eq_ignore_ascii_case("wayland"));
    let x11_session = session_type.is_some_and(|kind| kind.eq_ignore_ascii_case("x11"));
    if x11_session && let Some(display) = x11_display {
        return x11_target(
            display,
            config.xauthority.as_deref().or(xauthority),
            home,
            owner,
        );
    }
    let directory = runtime
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{owner}")));
    if let Some(display) = x11_display.filter(|_| !wayland_session) {
        if directory.exists() {
            validate_session_runtime(&directory, owner)?;
            ensure!(
                wayland_sockets(&directory, owner)?.is_empty(),
                "both X11 and Wayland sessions are possible; select --x11-display or --physical-wayland-socket"
            );
        }
        return x11_target(
            display,
            config.xauthority.as_deref().or(xauthority),
            home,
            owner,
        );
    }
    validate_session_runtime(&directory, owner)?;
    match wayland_sockets(&directory, owner)?.as_slice() {
        [socket] => Ok(SessionTarget::wayland(socket.clone(), directory)),
        [] => bail!("no same-user Wayland or local X11 session found"),
        _ => bail!("发现多个 Wayland 会话；请用 --physical-wayland-socket 指定物理桌面"),
    }
}

pub(crate) fn validate_local_x11_display(display: &str) -> Result<()> {
    let suffix = display
        .strip_prefix(':')
        .context("X11 DISPLAY must be local (:N or :N.screen)")?;
    let (number, screen) = suffix
        .split_once('.')
        .map_or((suffix, None), |(number, screen)| (number, Some(screen)));
    ensure!(
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
        "X11 DISPLAY must be local (:N or :N.screen)"
    );
    if let Some(screen) = screen {
        ensure!(
            !screen.is_empty() && screen.bytes().all(|byte| byte.is_ascii_digit()),
            "invalid X11 screen number"
        );
    }
    Ok(())
}

fn x11_target(
    display: &str,
    authority: Option<&Path>,
    home: Option<&Path>,
    owner: u32,
) -> Result<SessionTarget> {
    validate_local_x11_display(display)?;
    let authority = authority
        .map(Path::to_path_buf)
        .or_else(|| home.map(|path| path.join(".Xauthority")))
        .context("X11 requires --xauthority, XAUTHORITY or HOME/.Xauthority")?;
    ensure!(
        authority.is_absolute(),
        "Xauthority must be an absolute path"
    );
    let metadata = fs::symlink_metadata(&authority).context("Xauthority file unavailable")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Xauthority must be a regular file, not a symlink"
    );
    ensure!(
        metadata.uid() == owner,
        "Xauthority belongs to another user"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "Xauthority must be private (0600)"
    );
    Ok(SessionTarget {
        x11_display: Some(display.to_owned()),
        xauthority: Some(authority),
        ..SessionTarget::default()
    })
}

fn validate_session_runtime(path: &Path, owner: u32) -> Result<()> {
    ensure!(path.is_absolute(), "session runtime must be absolute");
    let metadata = fs::symlink_metadata(path)
        .context("session runtime is unavailable for this service identity (same UID required)")?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "session runtime must be a directory, not a symlink"
    );
    ensure!(
        metadata.uid() == owner,
        "session runtime belongs to another user"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "session runtime must be private (0700)"
    );
    Ok(())
}

fn validate_wayland_socket_for_owner(path: &Path, owner: u32) -> Result<()> {
    ensure!(path.is_absolute(), "Wayland socket must be absolute");
    validate_session_runtime(
        path.parent().context("Wayland socket has no parent")?,
        owner,
    )?;
    ensure!(
        is_owned_socket(path, owner),
        "Wayland socket is unavailable or belongs to another user"
    );
    Ok(())
}

/// Validate an explicitly selected same-user socket without opening the display.
/// Protocol readiness is checked separately before capture starts.
pub fn validate_wayland_socket(path: &Path) -> Result<()> {
    validate_wayland_socket_for_owner(path, unsafe { libc::geteuid() })
}

pub(crate) fn is_owned_socket(path: &Path, owner: u32) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_socket() && metadata.uid() == owner)
}

fn desktop_socket(runtime: &Path) -> Option<PathBuf> {
    let signature = fs::read_dir(runtime.join("hypr"))
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.path().join(".socket.sock").exists())?;
    let _ = signature;
    fs::read_dir(runtime)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("wayland-"))
        .filter(|entry| !entry.file_name().to_string_lossy().ends_with(".lock"))
        .filter(|entry| std::os::unix::net::UnixStream::connect(entry.path()).is_ok())
        .max_by_key(|entry| entry.file_name())
        .map(|entry| entry.path())
}

pub(crate) fn secure_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "runtime directory must not be a symlink"
    );
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() },
        "runtime directory has another owner"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "runtime directory must be private (0700)"
    );
    Ok(())
}

impl Drop for Session {
    fn drop(&mut self) {
        for child in self.children.iter_mut().rev() {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while !matches!(child_exited(child), Ok(true)) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
        }
        self.lock.take();
        self.temporary.take();
    }
}

fn child_exited(child: &Child) -> Result<bool> {
    let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id(),
            &mut status,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    ensure!(
        result == 0,
        "could not inspect owned process: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { status.si_pid() } != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn private_runtime() -> tempfile::TempDir {
        tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap()
    }

    #[test]
    fn physical_audio_requires_its_own_pulse_socket_and_never_defaults_to_a_microphone() {
        let directory = private_runtime();
        let pulse = directory.path().join("pulse");
        fs::create_dir(&pulse).unwrap();
        let socket = pulse.join("native");
        assert_eq!(physical_audio(directory.path(), true), (None, None));
        fs::write(&socket, "not a socket").unwrap();
        assert_eq!(physical_audio(directory.path(), true), (None, None));
        fs::remove_file(&socket).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert_eq!(
            physical_audio(directory.path(), true),
            (
                Some(format!("unix:{}", socket.display())),
                Some("@DEFAULT_MONITOR@".into())
            )
        );
        assert_eq!(physical_audio(directory.path(), false), (None, None));
        drop(listener);
        fs::rename(&socket, pulse.join("other")).unwrap();
        symlink(pulse.join("other"), &socket).unwrap();
        assert_eq!(physical_audio(directory.path(), true), (None, None));
    }

    #[test]
    fn physical_socket_requires_an_owned_socket_not_a_file_or_symlink() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let directory = private_runtime();
        let socket = directory.path().join("wayland-test");
        let mut config =
            Config::try_parse_from(["test", "--password", "socket-test-secret"]).unwrap();
        config.physical_wayland_socket = Some(socket.clone());
        assert!(Session::physical_socket(&config).is_err());
        fs::write(&socket, "not a socket").unwrap();
        assert!(Session::physical_socket(&config).is_err());
        fs::remove_file(&socket).unwrap();
        let _listener = UnixListener::bind(&socket).unwrap();
        assert_eq!(Session::physical_socket(&config).unwrap(), socket);
        let link = directory.path().join("link");
        symlink(&socket, &link).unwrap();
        config.physical_wayland_socket = Some(link);
        assert!(Session::physical_socket(&config).is_err());
        assert!(!is_owned_socket(&socket, unsafe { libc::geteuid() } + 1));
    }

    #[test]
    fn physical_discovery_uses_exact_private_runtime_and_keeps_ambiguity() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let directory = private_runtime();
        let first = directory.path().join("wayland-1");
        let second = directory.path().join("wayland-2");
        let _first = UnixListener::bind(&first).unwrap();
        let _second = UnixListener::bind(&second).unwrap();
        let config = Config::try_parse_from(["test"]).unwrap();
        let owner = unsafe { libc::geteuid() };
        assert_eq!(
            physical_socket_from_environment(
                &config,
                owner,
                Some(directory.path()),
                Some(Path::new("wayland-2"))
            )
            .unwrap(),
            second
        );
        assert_eq!(
            physical_socket_from_environment(&config, owner, Some(directory.path()), Some(&first))
                .unwrap(),
            first
        );
        assert!(
            physical_socket_from_environment(&config, owner, Some(directory.path()), None)
                .unwrap_err()
                .to_string()
                .contains("多个")
        );
        assert!(
            physical_socket_from_environment(
                &config,
                owner + 1,
                Some(directory.path()),
                Some(&first)
            )
            .is_err()
        );
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            physical_socket_from_environment(&config, owner, Some(directory.path()), Some(&first))
                .is_err()
        );
    }

    #[test]
    fn absolute_display_uses_its_own_private_runtime_and_relative_names_cannot_escape() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let directory = private_runtime();
        let socket = directory.path().join("wayland-custom");
        let _listener = UnixListener::bind(&socket).unwrap();
        let config = Config::try_parse_from(["test"]).unwrap();
        let owner = unsafe { libc::geteuid() };

        assert_eq!(
            physical_socket_from_environment(
                &config,
                owner,
                Some(Path::new("/missing-runtime")),
                Some(&socket)
            )
            .unwrap(),
            socket
        );
        for display in [
            "../wayland-custom",
            "nested/wayland-custom",
            "./wayland-custom",
        ] {
            assert!(
                physical_socket_from_environment(
                    &config,
                    owner,
                    Some(directory.path()),
                    Some(Path::new(display))
                )
                .is_err(),
                "{display}"
            );
        }
        assert!(
            physical_socket_from_environment(
                &config,
                owner,
                Some(Path::new("relative-runtime")),
                Some(Path::new("wayland-custom"))
            )
            .is_err()
        );
        assert!(
            physical_socket_from_environment(
                &config,
                owner,
                Some(directory.path()),
                Some(Path::new("wayland-missing"))
            )
            .is_err()
        );
    }

    #[test]
    fn persistent_transport_discovers_physical_login_outside_its_private_runtime() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let service = private_runtime();
        let login = private_runtime();
        let _private_socket = UnixListener::bind(service.path().join("wayland-1")).unwrap();
        let physical = login.path().join("wayland-1");
        let _physical = UnixListener::bind(&physical).unwrap();
        let mut config = Config::try_parse_from(["test"]).unwrap();
        config.session_runtime = Some(service.path().into());
        let runtime = physical_discovery_runtime(&config, Some(service.path()), None, login.path());
        let target = physical_target_from_environment(
            &config,
            unsafe { libc::geteuid() },
            runtime,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(target.wayland_socket.as_deref(), Some(physical.as_path()));
        assert_eq!(target.runtime, login.path());
        // Multiple physical logins still require explicit selection.
        let _second = UnixListener::bind(login.path().join("wayland-2")).unwrap();
        assert!(
            physical_target_from_environment(
                &config,
                unsafe { libc::geteuid() },
                runtime,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap_err()
            .to_string()
            .contains("多个 Wayland")
        );
    }

    #[test]
    fn physical_discovery_keeps_explicit_display_runtime_and_interactive_environment() {
        use clap::Parser;
        let mut config = Config::try_parse_from(["test"]).unwrap();
        let service = Path::new("/run/remote-service");
        let login = Path::new("/run/user/1000");
        assert_eq!(
            physical_discovery_runtime(&config, Some(service), None, login),
            Some(service)
        );
        config.session_runtime = Some("/run/owned-desktop".into());
        assert_eq!(
            physical_discovery_runtime(&config, Some(service), Some(Path::new("wayland-2")), login),
            Some(service)
        );
        assert_eq!(
            physical_discovery_runtime(&config, Some(service), Some(Path::new("")), login),
            Some(login)
        );
    }

    #[test]
    fn physical_target_prefers_same_user_wayland_over_xwayland_and_rejects_ambiguous_sockets() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let directory = private_runtime();
        let first = directory.path().join("wayland-1");
        let second = directory.path().join("wayland-2");
        let _first = UnixListener::bind(&first).unwrap();
        let _second = UnixListener::bind(&second).unwrap();
        let authority = directory.path().join("Xauthority");
        fs::write(&authority, "test").unwrap();
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o600)).unwrap();
        let config = Config::try_parse_from(["test"]).unwrap();
        let owner = unsafe { libc::geteuid() };
        let target = physical_target_from_environment(
            &config,
            owner,
            Some(directory.path()),
            Some(Path::new("wayland-2")),
            Some(":42"),
            Some(&authority),
            None,
            Some("wayland"),
        )
        .unwrap();
        assert_eq!(target.wayland_socket, Some(second.clone()));
        assert_eq!(target.x11_display, None);
        let x11 = physical_target_from_environment(
            &config,
            owner,
            Some(directory.path()),
            None,
            Some(":42"),
            Some(&authority),
            None,
            Some("x11"),
        )
        .unwrap();
        assert_eq!(x11.x11_display.as_deref(), Some(":42"));
        assert!(x11.wayland_socket.is_none());
        assert!(
            physical_target_from_environment(
                &config,
                owner,
                Some(directory.path()),
                None,
                Some(":42"),
                Some(&authority),
                None,
                Some("wayland")
            )
            .is_err()
        );
        fs::remove_file(&first).unwrap();
        let target = physical_target_from_environment(
            &config,
            owner,
            Some(directory.path()),
            None,
            Some(":42"),
            Some(&authority),
            None,
            Some("wayland"),
        )
        .unwrap();
        assert_eq!(target.wayland_socket, Some(second));
    }

    #[test]
    fn attached_wayland_target_overrides_ambient_and_never_falls_back() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let directory = private_runtime();
        let ambient = directory.path().join("wayland-0");
        let attached = directory.path().join("wayland-2");
        let _ambient = UnixListener::bind(&ambient).unwrap();
        let _attached = UnixListener::bind(&attached).unwrap();
        let mut config =
            Config::try_parse_from(["test", "--wayland-socket", attached.to_str().unwrap()])
                .unwrap();
        let owner = unsafe { libc::geteuid() };
        // The service attachment is the exact target, including when the
        // service process inherited another compositor or no desktop runtime.
        for runtime in [
            Some(directory.path()),
            Some(Path::new("/missing-runtime")),
            None,
        ] {
            for display in [Some(Path::new("wayland-0")), None] {
                let target = physical_target_from_environment(
                    &config,
                    owner,
                    runtime,
                    display,
                    Some(":0"),
                    None,
                    None,
                    Some("x11"),
                )
                .unwrap();
                assert_eq!(target.wayland_socket.as_deref(), Some(attached.as_path()));
                assert!(target.x11_display.is_none());
            }
        }
        // Preserve the attachment even if a separate discovery hint is set.
        config.physical_wayland_socket = Some(ambient.clone());
        assert_eq!(
            physical_socket_from_environment(&config, owner, Some(directory.path()), None).unwrap(),
            attached,
        );
        // An invalid explicit target must fail, never switch to the ambient
        // desktop or to the physical discovery hint.
        fs::remove_file(&attached).unwrap();
        assert!(
            physical_target_from_environment(
                &config,
                owner,
                Some(directory.path()),
                Some(Path::new("wayland-0")),
                None,
                None,
                None,
                Some("wayland"),
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_x11_target_uses_exact_authority_without_wayland_fallback() {
        use clap::Parser;
        let directory = private_runtime();
        let authority = directory.path().join("Xauthority");
        fs::write(&authority, "test").unwrap();
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o600)).unwrap();
        let mut config = Config::try_parse_from(["test", "--x11-display", ":23"]).unwrap();
        config.xauthority = Some(authority.clone());
        let owner = unsafe { libc::geteuid() };
        let target = physical_target_from_environment(
            &config,
            owner,
            Some(directory.path()),
            Some(Path::new("wayland-missing")),
            Some(":0"),
            None,
            None,
            Some("wayland"),
        )
        .unwrap();
        assert_eq!(target.x11_display.as_deref(), Some(":23"));
        assert_eq!(target.xauthority.as_deref(), Some(authority.as_path()));
        assert!(target.wayland_socket.is_none());
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            physical_target_from_environment(
                &config,
                owner,
                Some(directory.path()),
                None,
                None,
                None,
                None,
                None
            )
            .is_err()
        );
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            physical_target_from_environment(
                &config,
                owner + 1,
                Some(directory.path()),
                None,
                None,
                None,
                None,
                None
            )
            .is_err()
        );
        let link = directory.path().join("linked-authority");
        symlink(&authority, &link).unwrap();
        config.xauthority = Some(link);
        assert!(
            physical_target_from_environment(
                &config,
                owner,
                Some(directory.path()),
                None,
                None,
                None,
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn ambient_local_x11_is_selected_only_when_wayland_is_absent() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let directory = private_runtime();
        let authority = directory.path().join("Xauthority");
        fs::write(&authority, "test").unwrap();
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o600)).unwrap();
        let config = Config::try_parse_from(["test"]).unwrap();
        let owner = unsafe { libc::geteuid() };
        let target = physical_target_from_environment(
            &config,
            owner,
            Some(directory.path()),
            None,
            Some(":7.1"),
            Some(&authority),
            None,
            Some("x11"),
        )
        .unwrap();
        assert_eq!(target.x11_display.as_deref(), Some(":7.1"));
        let ambient = physical_target_from_environment(
            &config,
            owner,
            Some(directory.path()),
            None,
            Some(":7.1"),
            Some(&authority),
            None,
            None,
        )
        .unwrap();
        assert_eq!(ambient.x11_display.as_deref(), Some(":7.1"));
        let socket = directory.path().join("wayland-0");
        let _listener = UnixListener::bind(&socket).unwrap();
        assert!(
            physical_target_from_environment(
                &config,
                owner,
                Some(directory.path()),
                None,
                Some(":7.1"),
                Some(&authority),
                None,
                None,
            )
            .unwrap_err()
            .to_string()
            .contains("both X11 and Wayland sessions are possible")
        );
        assert!(
            physical_target_from_environment(
                &config,
                owner,
                Some(directory.path()),
                Some(Path::new("wayland-missing")),
                Some(":7.1"),
                Some(&authority),
                None,
                Some("wayland")
            )
            .is_err()
        );
    }

    #[test]
    fn external_attachment_rejects_non_socket_and_unowned_runtime() {
        use clap::Parser;
        use std::os::unix::net::UnixListener;
        let directory = private_runtime();
        let path = directory.path().join("wayland-file");
        fs::write(&path, "not a socket").unwrap();
        let config =
            Config::try_parse_from(["test", "--wayland-socket", path.to_str().unwrap()]).unwrap();
        assert!(Session::start(&config).is_err());
        fs::remove_file(&path).unwrap();
        let _listener = UnixListener::bind(&path).unwrap();
        let external = Session::start(&config).unwrap();
        assert_eq!(external.socket, path);
        assert!(!external.is_owned());
        assert!(!directory.path().join("application-environment.sh").exists());
        assert!(!directory.path().join("bin").exists());
        let persistent = Session::attached(
            directory.path().to_path_buf(),
            path.clone(),
            None,
            None,
            None,
            None,
        );
        assert!(persistent.is_owned());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Session::start(&config).is_err());
    }

    #[test]
    fn minimal_desktop_retains_terminal_without_omarchy_dependencies() {
        let config = desktop_config::minimal(1280, 720, 20, "test-terminal");
        assert!(config.contains("exec-once = $terminal"));
        assert!(!config.contains("omarchy-menu"));
    }

    #[test]
    fn session_app_launcher_preserves_arguments_and_rejects_unsupported_options() {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/session-uwsm-app.sh");
        let output = Command::new("sh")
            .args([script, "--", "printf", "%s", "text with spaces; $(false)"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"text with spaces; $(false)");
        for arguments in [vec![script], vec![script, "--"], vec![script, "--bad"]] {
            assert!(
                !Command::new("sh")
                    .args(arguments)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
    }

    #[test]
    fn application_activation_environment_uses_private_bus_and_discovered_display() {
        let directory = tempfile::tempdir().unwrap();
        let stub = directory.path().join("dbus-update-activation-environment");
        fs::write(
            &stub,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$TEST_ARGUMENTS\"\n",
        )
        .unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).unwrap();
        let arguments = directory.path().join("arguments");
        let display = directory.path().join("display with ' quotes");
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/session-environment.sh"
        );
        let mut command = Command::new("/bin/sh");
        command
            .args([script])
            .arg(&display)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", directory.path().display()),
            )
            .env("TEST_ARGUMENTS", &arguments)
            .env("DISPLAY", ":93")
            .env("WAYLAND_DISPLAY", "wayland-7")
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/private/test/bus")
            .env("HYPRLAND_INSTANCE_SIGNATURE", "private-desktop")
            .env("XDG_STATE_HOME", "/private/test/state")
            .env("OMARCHY_PATH", "/private/test/omarchy")
            .env("PIPEWIRE_RUNTIME_DIR", "/private/test/pipewire");
        assert!(command.status().unwrap().success());
        assert_eq!(fs::read_to_string(&display).unwrap(), ":93\n");
        let updated = fs::read_to_string(&arguments).unwrap();
        assert!(updated.contains("DISPLAY=:93\n"));
        assert!(updated.contains("WAYLAND_DISPLAY=wayland-7\n"));
        assert!(updated.contains("DBUS_SESSION_BUS_ADDRESS=unix:path=/private/test/bus\n"));
        assert!(!updated.contains("--systemd"));
        assert!(updated.contains("XDG_STATE_HOME=/private/test/state\n"));
        assert!(updated.contains("OMARCHY_PATH=/private/test/omarchy\n"));
        assert!(updated.contains("PIPEWIRE_RUNTIME_DIR=/private/test/pipewire\n"));
        command.env_remove("DISPLAY");
        assert!(command.status().unwrap().success());
        assert_eq!(fs::read_to_string(&display).unwrap(), "\n");
        fs::write(&stub, "#!/bin/sh\nexit 1\n").unwrap();
        fs::remove_file(&display).unwrap();
        assert!(!command.status().unwrap().success());
        assert!(!display.exists());
    }

    #[test]
    fn runtime_socket_budget_covers_hyprland_signature_and_event_socket() {
        assert!(validate_runtime_socket_paths(Path::new(DEFAULT_SESSION_RUNTIME)).is_ok());
        let error = validate_runtime_socket_paths(Path::new(PREVIOUS_SESSION_RUNTIME)).unwrap_err();
        assert!(error.to_string().contains("shorter --runtime-dir"));
        assert!(validate_runtime_socket_paths(Path::new(&format!("/{}", "x".repeat(22)))).is_ok());
        assert!(validate_runtime_socket_paths(Path::new(&format!("/{}", "x".repeat(23)))).is_err());
        assert!(validate_runtime_socket_paths(Path::new(&format!("/{}", "界".repeat(8)))).is_err());
    }

    #[test]
    fn runtime_directory_rejects_symlink_and_public_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let link = directory.path().join("link");
        symlink(directory.path(), &link).unwrap();
        assert!(secure_directory(&link).is_err());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(secure_directory(directory.path()).is_err());
    }

    #[test]
    fn teardown_kills_descendants_even_when_leader_has_exited() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant");
        let child = Command::new("sh")
            .args([
                "-c",
                "(trap '' TERM; exec sleep 600) & echo $! > \"$1\"; exit 0",
                "sh",
            ])
            .arg(&pid_file)
            .process_group(0)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !pid_file.exists() || !child_exited(&child).unwrap() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let session = Session {
            socket: PathBuf::new(),
            runtime: directory.path().to_path_buf(),
            signature: None,
            pulse_server: None,
            pulse_client_config: None,
            audio_source: None,
            target: None,
            owned: true,
            children: vec![child],
            temporary: None,
            lock: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        drop(session);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = fs::read_to_string(format!("/proc/{pid}/stat"));
            if status.as_ref().is_err()
                || status
                    .as_ref()
                    .unwrap()
                    .split_once(") ")
                    .unwrap()
                    .1
                    .starts_with('Z')
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned descendant survived teardown"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
