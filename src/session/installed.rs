//! PC adapter for the same installed-desktop launcher used by Linux instances.
//! Capture/input remain in LinuxHost; the launcher publishes no HTTP listener.
use super::*;
use clap::{CommandFactory, FromArgMatches};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Desktop {
    id: String,
    command: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    version: u32,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
}

pub(super) struct Scope {
    unit: String,
    runtime: PathBuf,
    login_runtime: PathBuf,
}

impl Scope {
    fn new(runtime: &Path) -> Result<Self> {
        let name = runtime
            .file_name()
            .and_then(|name| name.to_str())
            .context("virtual runtime name missing")?;
        let uid = unsafe { libc::geteuid() };
        Ok(Self {
            unit: format!("lightos-{name}.scope"),
            runtime: runtime.to_path_buf(),
            login_runtime: PathBuf::from(format!("/run/user/{uid}")),
        })
    }

    fn launcher(&self) -> Command {
        // The private systemd user manager moves processes into init.scope.
        // Give it a delegated scope containing only this desktop's processes.
        let mut command = Command::new("systemd-run");
        super::niri::isolate(&mut command);
        command
            .env("XDG_RUNTIME_DIR", &self.login_runtime)
            .args(["--user", "--scope", "--quiet"])
            .args(["--property=Delegate=yes", "--property=TimeoutStopSec=3s"])
            .arg(format!("--unit={}", self.unit))
            .arg(format!(
                "--setenv=XDG_RUNTIME_DIR={}",
                self.runtime.display()
            ))
            .args(["--", "/bin/sh"]);
        command
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let mut command = Command::new("timeout");
        super::niri::isolate(&mut command);
        let _ = command
            .env("XDG_RUNTIME_DIR", &self.login_runtime)
            .args(["5s", "systemctl", "--user", "--quiet", "stop", "--"])
            .arg(&self.unit)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        // systemd makes these directories read-only or mode 000, so TempDir
        // cannot otherwise remove the stopped desktop's private runtime.
        let uid = unsafe { libc::geteuid() };
        let mut directory = self.runtime.clone();
        for name in ["systemd", "inaccessible", "dir"] {
            directory.push(name);
            let Ok(metadata) = fs::symlink_metadata(&directory) else {
                break;
            };
            if !metadata.is_dir() || metadata.uid() != uid {
                break;
            }
            if metadata.mode() & 0o700 != 0o700 {
                let _ = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700));
            }
        }
    }
}

fn uses_private_user_manager(desktop: &Desktop) -> bool {
    let session = format!("{}:{}", desktop.id, desktop.command).to_ascii_lowercase();
    !session.contains("niri") && !session.contains("hyprland")
}

pub(crate) fn payload() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let root = executable
        .parent()
        .context("runtime directory missing")?
        .join("session-support");
    ensure!(
        root.join("session.sh").is_file(),
        "Linux virtual session support is not packaged; update the runtime"
    );
    Ok(root)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn desktop() -> Result<Desktop> {
    if platform_detect::OmarchyInstallation::detect().is_some() {
        return Ok(Desktop {
            id: "wayland:hyprland".into(),
            command: "Hyprland".into(),
        });
    }
    let mut choices = Vec::new();
    for (kind, directory) in [("wayland", "wayland-sessions"), ("x11", "xsessions")] {
        for prefix in ["/usr/local/share", "/usr/share"] {
            let Ok(entries) = fs::read_dir(Path::new(prefix).join(directory)) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path
                    .extension()
                    .is_none_or(|extension| extension != "desktop")
                {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let mut group = false;
                let mut values = BTreeMap::new();
                for line in text.lines().map(str::trim) {
                    if line.starts_with('[') {
                        group = line == "[Desktop Entry]";
                    } else if group
                        && !line.starts_with('#')
                        && let Some((key, value)) = line.split_once('=')
                    {
                        values.insert(key.trim(), value.trim());
                    }
                }
                if values.get("Hidden") == Some(&"true") || values.get("NoDisplay") == Some(&"true")
                {
                    continue;
                }
                let Some(exec) = values.get("Exec") else {
                    continue;
                };
                let Ok(args) = arguments(exec) else { continue };
                if args.is_empty()
                    || platform_detect::find_executable(Path::new(&args[0])).is_none()
                {
                    continue;
                }
                if values
                    .get("TryExec")
                    .is_some_and(|name| platform_detect::find_executable(Path::new(name)).is_none())
                {
                    continue;
                }
                let name = path
                    .file_stem()
                    .context("desktop name missing")?
                    .to_string_lossy();
                // Exec is an argument vector, not shell code. Keep host entry
                // commands but isolate protocol/session identity in the launcher.
                let mut command = format!(
                    "env {} {} {}",
                    quote(&format!("XDG_SESSION_TYPE={kind}")),
                    quote(&format!("DESKTOP_SESSION={name}")),
                    quote(&format!("XDG_SESSION_DESKTOP={name}"))
                );
                if let Some(names) = values.get("DesktopNames") {
                    command.push_str(&format!(
                        " {}",
                        quote(&format!(
                            "XDG_CURRENT_DESKTOP={}",
                            names.trim_end_matches(';').replace(';', ":")
                        ))
                    ));
                }
                for arg in args {
                    if matches!(arg.as_str(), "%f" | "%F" | "%u" | "%U" | "%i") {
                        continue;
                    }
                    let arg = if arg == "%c" {
                        values.get("Name").copied().unwrap_or(&name).to_owned()
                    } else {
                        arg.replace("%%", "%")
                    };
                    command.push_str(&format!(" {}", quote(&arg)));
                }
                let id = format!("{kind}:{name}");
                if !choices.iter().any(|choice: &Desktop| choice.id == id) {
                    choices.push(Desktop { id, command });
                }
            }
        }
    }
    choices.sort_by(|a, b| a.id.cmp(&b.id));
    let current = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    let selected_name = std::env::var("DESKTOP_SESSION").unwrap_or_default();
    let family = if current.contains("gnome") || current.contains("ubuntu") {
        "gnome"
    } else if current.contains("kde") {
        "plasma"
    } else if current.contains("xfce") {
        "xfce"
    } else if current.contains("hyprland") {
        "hyprland"
    } else if current.contains("niri") {
        "niri"
    } else {
        ""
    };
    let index = choices
        .iter()
        .position(|choice| {
            !selected_name.is_empty() && choice.id == format!("wayland:{selected_name}")
        })
        .or_else(|| {
            choices.iter().position(|choice| {
                !selected_name.is_empty() && choice.id == format!("x11:{selected_name}")
            })
        })
        .or_else(|| {
            choices.iter().position(|choice| {
                !family.is_empty()
                    && (choice.id.contains(family)
                        || family == "gnome" && choice.id.contains("ubuntu"))
            })
        })
        .unwrap_or(0);
    ensure!(
        !choices.is_empty(),
        "No supported installed desktop session was found"
    );
    Ok(choices.remove(index))
}

fn arguments(value: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for character in value.chars() {
        if escaped {
            word.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            started = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                word.push(character);
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
            started = true;
        } else if matches!(character, ' ' | '\t') {
            if started {
                args.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(character);
            started = true;
        }
    }
    ensure!(
        quote.is_none() && !escaped,
        "invalid installed desktop command"
    );
    if started {
        args.push(word);
    }
    Ok(args)
}

pub(crate) fn available() -> Result<()> {
    let payload = payload()?;
    let selected = desktop()?;
    if uses_private_user_manager(&selected) {
        ensure!(
            platform_detect::find_executable(Path::new("systemd-run")).is_some(),
            "Installed virtual desktop requires systemd-run to isolate its user manager"
        );
        if selected.id.starts_with("wayland:") {
            ensure!(
                cfg!(feature = "portal"),
                "Installed Wayland desktop capture requires the portal build feature"
            );
        }
    }
    // One dependency definition serves containers, PC diagnostics and explicit
    // installation. This mode never invokes a package manager or codec worker.
    let mut command = Command::new("timeout");
    super::niri::isolate(&mut command);
    let output = command
        .args(["10", "/bin/sh"])
        .arg(payload.join("setup-dependencies.sh"))
        .args(["--check", "--session-only"])
        .arg(&payload)
        .env("LIGHTOS_REMOTE_DESKTOP_SESSION_ID", &selected.id)
        .env("LIGHTOS_REMOTE_DESKTOP_SESSION", &selected.command)
        .output()
        .context("check shared Linux session dependencies")?;
    ensure!(
        output.status.success(),
        "Virtual desktop dependencies unavailable: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

pub(crate) fn start(config: &Config, cancelled: Arc<AtomicBool>) -> Result<(Config, Session)> {
    available()?;
    let selected = desktop()?;
    let temporary = tempfile::Builder::new()
        .prefix("lrd-owned.")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp")?;
    let runtime = temporary.path().to_owned();
    let mut owner = Session {
        socket: PathBuf::new(),
        runtime: runtime.clone(),
        signature: None,
        pulse_server: None,
        pulse_client_config: None,
        audio_source: None,
        target: None,
        owned: true,
        children: Vec::new(),
        temporary: Some(temporary),
        lock: None,
        native_lifetime: None,
        installed_scope: None,
        cancelled: cancelled.clone(),
    };
    for directory in ["config", "cache", "state"] {
        secure_directory(&runtime.join(directory))?;
    }
    let payload = payload()?;
    let mut command = if uses_private_user_manager(&selected) {
        let scope = Scope::new(&runtime)?;
        let command = scope.launcher();
        owner.installed_scope = Some(scope);
        command
    } else {
        let mut command = Command::new("/bin/sh");
        super::niri::isolate(&mut command);
        command.env("XDG_RUNTIME_DIR", &runtime);
        command
    };
    command
        .arg(payload.join("session.sh"))
        .env("LIGHTOS_REMOTE_DESKTOP_PROGRAM", std::env::current_exe()?)
        .env("LIGHTOS_REMOTE_DESKTOP_PAYLOAD_DIR", &payload)
        .env("LIGHTOS_REMOTE_DESKTOP_SESSION_ONLY", "1")
        // Native desktop-session validates Config but opens no listener. Do not
        // pass the parent controller's authentication secret to the launcher.
        .env(
            "LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD",
            format!("{:032x}", rand::random::<u128>()),
        )
        .env("LIGHTOS_REMOTE_DESKTOP_SESSION_ID", &selected.id)
        .env("LIGHTOS_REMOTE_DESKTOP_SESSION", &selected.command)
        .env("LIGHTOS_REMOTE_DESKTOP_RUNTIME_DIR", &runtime)
        .env("XDG_CACHE_HOME", runtime.join("cache"))
        .env("XDG_STATE_HOME", runtime.join("state"));
    if !selected.id.contains("hyprland") {
        command.env("XDG_CONFIG_HOME", runtime.join("config"));
    }
    let (width, height) = config.dimensions()?;
    command.env(
        "LIGHTOS_REMOTE_DESKTOP_SCREEN",
        format!("{width}x{height}x24"),
    );
    // Private display allocation must not collide with any existing X server.
    let display = (100..600)
        .find(|number| {
            // GNOME X11 also uses the following display for owned Xephyr.
            [*number, *number + 1].iter().all(|number| {
                !Path::new(&format!("/tmp/.X11-unix/X{number}")).exists()
                    && !Path::new(&format!("/tmp/.X{number}-lock")).exists()
            })
        })
        .context("No free private X display")?;
    command.env(
        "LIGHTOS_REMOTE_DESKTOP_VIRTUAL_DISPLAY",
        format!(":{display}"),
    );
    owner.spawn(command, "desktop.log")?;
    let ready = runtime.join("installed-session.json");
    let native = runtime.join("session.json");
    owner.wait_for(Duration::from_secs(45), || {
        ready.exists() || native.exists()
    })?;
    let mut settings = config.clone();
    settings.virtual_fallback = false;
    settings.wayland_socket = None;
    settings.x11_display = None;
    settings.xauthority = None;
    settings.session_runtime = None;
    settings.runtime_dir = None;
    settings.physical_wayland_socket = None;
    settings.virtual_environment.clear();
    let mut attached = if native.exists() {
        crate::handoff::Descriptor::read(&runtime)?.attach(&runtime)?
    } else {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&ready)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() }
                && metadata.is_file()
                && metadata.mode() & 0o077 == 0
                && metadata.len() < 32768,
            "invalid owned session descriptor"
        );
        let descriptor: Ready = serde_json::from_reader(file.take(32768))?;
        ensure!(
            descriptor.version == 1
                && descriptor
                    .arguments
                    .first()
                    .is_some_and(|arg| arg == "serve"),
            "unsupported owned session descriptor"
        );
        let mut args = vec!["lightos-remoted".to_owned()];
        args.extend(descriptor.arguments.into_iter().skip(1));
        // The parent still carries the physical GUI environment. Descriptor
        // parsing must not merge those physical LIGHTOS_* settings back in.
        let matches = Config::command()
            .mut_args(|arg| arg.env(None::<&'static str>))
            .try_get_matches_from(args)?;
        let parsed = Config::from_arg_matches(&matches)?;
        settings.wayland_socket = parsed.wayland_socket;
        settings.x11_display = parsed.x11_display;
        settings.xauthority = parsed.xauthority;
        settings.virtual_wayland_server_pid = parsed.virtual_wayland_server_pid;
        settings.virtual_x11_server_pid = parsed.virtual_x11_server_pid;
        settings.virtual_x11_dynamic_modes = parsed.virtual_x11_dynamic_modes;
        settings.virtual_environment = descriptor.environment;
        Session::start_in_launcher(&settings, cancelled, owner.children.first())?
    };
    attached.children.append(&mut owner.children);
    attached.installed_scope = owner.installed_scope.take();
    attached.temporary = owner.temporary.take();
    Ok((settings, attached))
}
