//! Conservative Portal routing for an explicitly selected Wayland socket.
//!
//! Older Portal frontends load only the compiled descriptor directory. Newer
//! ones also load XDG data directories, where the first copy of each basename
//! takes precedence. Configuration lookup and backend priority also differ
//! across releases. Require the same route under those rules before accepting
//! a compositor identity, since the D-Bus interfaces do not expose the actual
//! backend selection.
//!
//! Scope still needs independent proof: the implementation's exact Wayland
//! environment AND the selected listening socket must match the compositor
//! services. Socket activation can retain its creator PID while the compositor
//! inherits the listener; that case requires proof of the actual kernel socket
//! descriptor. The frontend environment alone never suffices.
//! Multiple providers, custom installation prefixes/implementations, changed
//! descriptors, and unavailable identity evidence remain explicitly unavailable.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, Metadata, OpenOptions},
    io::Read,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use configparser::ini::Ini;
use display_control::SessionTarget;
use tokio::net::UnixStream;
use zbus::{Connection, fdo::DBusProxy, names::OwnedUniqueName};

const DESKTOP_PORTAL: &str = "org.freedesktop.portal.Desktop";
const REMOTE_DESKTOP: &str = "org.freedesktop.impl.portal.RemoteDesktop";
const SCREEN_CAST: &str = "org.freedesktop.impl.portal.ScreenCast";
const KWIN_WRAPPER: &str = "org.kde.KWinWrapper";
const MAX_ENVIRONMENT_BYTES: usize = 128 * 1024;
const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 256;
const MAX_DATA_DIRS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Gnome,
    Kde,
}
impl Provider {
    fn bus_name(self) -> &'static str {
        match self {
            Self::Gnome => "org.freedesktop.impl.portal.desktop.gnome",
            Self::Kde => "org.freedesktop.impl.portal.desktop.kde",
        }
    }
    fn executable(self) -> &'static str {
        match self {
            Self::Gnome => "xdg-desktop-portal-gnome",
            Self::Kde => "xdg-desktop-portal-kde",
        }
    }
    fn compositor_services(self) -> &'static [&'static str] {
        match self {
            Self::Gnome => &[
                "org.gnome.Mutter.ScreenCast",
                "org.gnome.Mutter.RemoteDesktop",
            ],
            Self::Kde => &["org.kde.KWin"],
        }
    }
}

#[derive(Debug)]
struct Peer {
    owner: OwnedUniqueName,
    pid: u32,
    uid: u32,
}

async fn peer(bus: &DBusProxy<'_>, name: &str, expected_uid: u32) -> Result<Peer> {
    let owner = bus
        .get_name_owner(name.try_into()?)
        .await
        .with_context(|| format!("Portal scope service {name} has no running owner"))?;
    let pid = bus
        .get_connection_unix_process_id(owner.as_str().try_into()?)
        .await?;
    let uid = bus
        .get_connection_unix_user(owner.as_str().try_into()?)
        .await?;
    ensure!(
        uid == expected_uid,
        "Portal scope service {name} belongs to another user"
    );
    ensure!(
        pid != 0,
        "Portal scope service {name} has no local process identity"
    );
    Ok(Peer { owner, pid, uid })
}

/// Stable owners of the one admitted capture/input route. Retain this proof
/// through consent and streaming; any owner loss/replacement invalidates it.
#[derive(Debug)]
pub(super) struct ScopeProof {
    owners: Vec<(&'static str, OwnedUniqueName)>,
    pub(super) restore_key: String,
    pub(super) active_key: String,
}
impl ScopeProof {
    /// Cheap read-only lifecycle check: no descriptor/proc scan or new consent.
    /// The caller must bound this call and withhold input/frames on failure.
    pub(super) async fn verify_unchanged(&self, connection: &Connection) -> Result<()> {
        let bus = DBusProxy::new(connection).await?;
        for (service, expected) in &self.owners {
            let owner = bus
                .get_name_owner((*service).try_into()?)
                .await
                .with_context(|| format!("Portal scope service {service} lost its owner"))?;
            ensure!(
                &owner == expected,
                "Portal scope service {service} restarted; its target identity must be validated again"
            );
        }
        Ok(())
    }
}

/// Read-only identity check: no CreateSession, devices or consent request.
/// The caller must put the complete check under its probe/startup timeout.
/// Call during discovery and again immediately before creating a session.
///
/// GNOME uses Mutter's two named D-Bus services for capture/input. KDE uses the
/// Qt Wayland connection for screen casting and legacy input, and KWin's D-Bus
/// service for EIS. Only those explicitly verified implementation routes are
/// admitted here; desktop names and matching login IDs are not evidence.
pub(super) async fn validate_target_identity(
    connection: &Connection,
    target: &SessionTarget,
) -> Result<ScopeProof> {
    let socket = target
        .wayland_socket
        .as_deref()
        .context("Portal identity requires an explicit Wayland socket")?;
    crate::session::validate_wayland_socket(socket)
        .context("Portal target Wayland socket is invalid")?;
    ensure!(
        target.runtime.is_absolute() && socket.parent() == Some(target.runtime.as_path()),
        "Portal target socket does not belong to the selected absolute runtime"
    );
    let uid = unsafe { libc::geteuid() };
    let wayland = UnixStream::connect(socket)
        .await
        .context("connect to selected Wayland socket for Portal identity")?;
    let credentials = wayland.peer_cred()?;
    let compositor_pid = u32::try_from(
        credentials
            .pid()
            .context("selected Wayland socket has no compositor process identity")?,
    )?;
    ensure!(
        credentials.uid() == uid,
        "selected Wayland compositor belongs to another user"
    );

    let bus = DBusProxy::new(connection).await?;
    let frontend = peer(&bus, DESKTOP_PORTAL, uid).await?;
    // A different mount namespace could expose a different descriptor registry.
    ensure!(
        fs::read_link(format!("/proc/{}/ns/mnt", frontend.pid))?
            == fs::read_link("/proc/self/ns/mnt")?,
        "Portal frontend has a different filesystem namespace; its provider registry cannot be verified"
    );
    let started = process_started(frontend.pid)?;
    let frontend_environment = process_environment(&frontend)?;
    exact_environment(target, &frontend_environment, "frontend")?;
    let frontend_executable = fs::read_link(format!("/proc/{}/exe", frontend.pid))?;
    let search = search_paths(&frontend_environment, &frontend_executable)?;
    let desktops = desktop_names(&frontend_environment)?;
    let mut snapshot = Snapshot::default();
    let descriptors = load_descriptors(&mut snapshot, &search.descriptor_directories(), started)?;
    let configs = load_configurations(&mut snapshot, &search, &desktops, started)?;
    let registry = ProviderRegistry::new(&descriptors, &search.system_descriptor_directory());
    let provider = registry.provider(&configs, &desktops)?;
    snapshot.verify()?;

    let implementation = peer(&bus, provider.bus_name(), uid).await?;
    let environment = process_environment(&implementation)?;
    exact_environment(target, &environment, "implementation")?;
    let executable = fs::read_link(format!("/proc/{}/exe", implementation.pid))?;
    ensure!(
        executable
            .file_name()
            .is_some_and(|name| name == provider.executable()),
        "Portal implementation {} has an unknown executable",
        provider.bus_name()
    );
    let arguments = read_bounded(
        &PathBuf::from(format!("/proc/{}/cmdline", implementation.pid)),
        MAX_DESCRIPTOR_BYTES,
    )?;
    ensure_no_display_override(&arguments)?;
    if provider == Provider::Kde {
        let platform = environment_value(&environment, b"QT_QPA_PLATFORM=")?;
        ensure!(
            platform.is_none_or(|value| matches!(value, b"" | b"wayland" | b"wayland-egl")),
            "KDE Portal uses a custom Qt platform; its Wayland target cannot be verified"
        );
    }

    let mut owners = vec![
        (DESKTOP_PORTAL, frontend.owner),
        (provider.bus_name(), implementation.owner),
    ];
    for service in provider.compositor_services() {
        let compositor = peer(&bus, service, uid).await?;
        if compositor.pid == compositor_pid {
            verify_compositor_peer(service, compositor_pid, uid, compositor.pid, compositor.uid)?;
        } else {
            // SO_PEERCRED identifies the process that created the listener,
            // which can hand it to another process. Prove that the D-Bus owner
            // holds this exact listener rather than guessing a wrapper name
            // or accepting another process on the same user bus.
            if let Err(error) =
                crate::session::wayland_endpoint::verify_listener(socket, compositor.pid, uid)
            {
                let protected = error.chain().any(|cause| {
                    cause
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
                });
                if provider == Provider::Kde && protected {
                    let wrapper = peer(&bus, KWIN_WRAPPER, uid).await?;
                    ensure!(
                        wrapper.pid == compositor_pid,
                        "KWin activation owner differs from the selected Wayland peer"
                    );
                    verify_kwin_activation(socket, wrapper.pid, compositor.pid, uid)?;
                    owners.push((KWIN_WRAPPER, wrapper.owner));
                } else {
                    return Err(error).with_context(|| format!("Portal compositor service {service} does not own the selected Wayland endpoint"));
                }
            }
        }
        owners.push((service, compositor.owner));
    }
    snapshot.verify()?;
    let socket_identity = fs::metadata(socket)?;
    let active_key = format!(
        "{}:{}:{}:{}:{owners:?}",
        connection.server_guid(),
        uid,
        socket_identity.dev(),
        socket_identity.ino()
    );
    let proof = ScopeProof {
        owners,
        // Persist across runtime upgrades and login sessions. The system Portal
        // checks the actual grant and selected screen again on every restore.
        restore_key: format!("{uid}:{}:{}", provider.bus_name(), socket.display()),
        active_key,
    };
    proof.verify_unchanged(connection).await?;
    // Keep the selected compositor connection alive through all identity checks.
    drop(wayland);
    Ok(proof)
}

fn verify_compositor_peer(
    service: &str,
    expected_pid: u32,
    expected_uid: u32,
    pid: u32,
    uid: u32,
) -> Result<()> {
    ensure!(
        pid == expected_pid && uid == expected_uid,
        "Portal compositor service {service} targets process {pid}, but the selected Wayland socket belongs to process {expected_pid}"
    );
    Ok(())
}

fn verify_kwin_activation(socket: &Path, wrapper: u32, compositor: u32, uid: u32) -> Result<()> {
    // The upstream wrapper creates the listening socket and passes its FD to
    // its direct child. CAP_SYS_NICE can protect /proc/<kwin>/{root,fd}.
    // https://github.com/KDE/kwin/blob/master/src/helpers/wayland_wrapper/kwin_wrapper.cpp
    let executable = fs::read_link(format!("/proc/{wrapper}/exe"))?;
    ensure!(
        executable
            .file_name()
            .is_some_and(|name| name == "kwin_wayland_wrapper"),
        "unrecognized KWin activation executable"
    );
    let metadata = fs::metadata(&executable)?;
    ensure!(
        metadata.is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
        "KWin activation executable is not system-owned"
    );
    let path = PathBuf::from(format!("/proc/{compositor}/cmdline"));
    let bytes = read_bounded(&path, MAX_DESCRIPTOR_BYTES)?;
    let args: Vec<_> = bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .collect();
    ensure!(
        args.first()
            .is_some_and(|arg| Path::new(std::ffi::OsStr::from_bytes(arg))
                .file_name()
                .is_some_and(|name| name == "kwin_wayland")),
        "unrecognized KWin activation child"
    );
    let option = |name: &[u8]| -> Result<&[u8]> {
        let matches: Vec<_> = args
            .windows(2)
            .filter(|pair| pair[0] == name)
            .map(|pair| pair[1])
            .collect();
        ensure!(matches.len() == 1, "ambiguous KWin activation option");
        Ok(matches[0])
    };
    let fd = std::str::from_utf8(option(b"--wayland-fd")?)?.parse::<u32>()?;
    let display = PathBuf::from(OsString::from_vec(option(b"--socket")?.to_vec()));
    let resolved = if display.is_absolute() {
        display
    } else {
        socket
            .parent()
            .context("missing Wayland runtime")?
            .join(display)
    };
    ensure!(
        resolved == socket,
        "KWin activation names a different Wayland display"
    );
    crate::session::wayland_endpoint::verify_inherited_listener(
        socket, wrapper, compositor, uid, fd,
    )?;
    ensure!(
        read_bounded(&path, MAX_DESCRIPTOR_BYTES)? == bytes,
        "KWin activation arguments changed during verification"
    );
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    // Nonblocking open prevents a substituted FIFO from blocking the probe.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .with_context(|| format!("open Portal identity evidence {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file(),
        "Portal identity evidence is not a regular file"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "Portal identity evidence exceeds the read limit"
    );
    Ok(bytes)
}

fn process_environment(peer: &Peer) -> Result<Vec<u8>> {
    let path = PathBuf::from(format!("/proc/{}/environ", peer.pid));
    ensure!(
        fs::metadata(&path)?.uid() == peer.uid,
        "Portal process identity changed"
    );
    read_bounded(&path, MAX_ENVIRONMENT_BYTES)
}

fn environment_value<'a>(environment: &'a [u8], key: &[u8]) -> Result<Option<&'a [u8]>> {
    let mut values = environment
        .split(|byte| *byte == 0)
        .filter_map(|entry| entry.strip_prefix(key));
    let value = values.next();
    ensure!(
        values.next().is_none(),
        "Portal process has ambiguous {} entries",
        String::from_utf8_lossy(key)
    );
    Ok(value)
}

fn environment_path(environment: &[u8], key: &[u8]) -> Result<Option<PathBuf>> {
    Ok(environment_value(environment, key)?
        .filter(|value| !value.is_empty())
        .map(|value| PathBuf::from(OsString::from_vec(value.to_vec()))))
}

fn exact_environment(target: &SessionTarget, environment: &[u8], role: &str) -> Result<()> {
    ensure!(
        environment_value(environment, b"WAYLAND_SOCKET=")?.is_none_or(|value| value.is_empty()),
        "Portal {role} uses an inherited WAYLAND_SOCKET; its display target cannot be verified"
    );
    let runtime = environment_path(environment, b"XDG_RUNTIME_DIR=")?
        .with_context(|| format!("Portal {role} has no runtime identity"))?;
    ensure!(
        runtime.is_absolute() && runtime == target.runtime,
        "Portal {role} runtime differs from the selected Wayland runtime"
    );
    let display = environment_path(environment, b"WAYLAND_DISPLAY=")?
        .with_context(|| format!("Portal {role} has no Wayland display identity"))?;
    let advertised = if display.is_absolute() {
        display
    } else {
        runtime.join(display)
    };
    ensure!(
        target.wayland_socket.as_ref() == Some(&advertised),
        "Portal {role} advertises {}, but the selected Wayland socket is {}",
        advertised.display(),
        target
            .wayland_socket
            .as_deref()
            .unwrap_or(Path::new("<missing>"))
            .display()
    );
    Ok(())
}

fn ensure_no_display_override(arguments: &[u8]) -> Result<()> {
    for argument in arguments.split(|byte| *byte == 0).skip(1) {
        for option in [
            b"-platform".as_slice(),
            b"--platform",
            b"-display",
            b"--display",
            b"--wayland-display",
        ] {
            ensure!(
                !(argument == option
                    || argument
                        .strip_prefix(option)
                        .is_some_and(|tail| tail.starts_with(b"="))),
                "Portal implementation has a display override argument; its Wayland target cannot be verified"
            );
        }
    }
    Ok(())
}

/// The installed compile-time layout this check can prove. Upstream always
/// appends DATADIR, SYSCONFDIR (>= 1.17 only) and the data directories even
/// with XDG overrides. Do not guess those prefixes for a custom build or a
/// deleted executable.
struct InstallLayout {
    data: &'static str,
    sysconf: &'static str,
}
fn install_layout(executable: &Path) -> Result<InstallLayout> {
    match executable.to_str() {
        Some(
            "/usr/lib/xdg-desktop-portal"
            | "/usr/libexec/xdg-desktop-portal"
            | "/usr/lib/x86_64-linux-gnu/xdg-desktop-portal"
            | "/usr/lib/aarch64-linux-gnu/xdg-desktop-portal",
        ) => Ok(InstallLayout {
            data: "/usr/share",
            sysconf: "/etc",
        }),
        Some("/usr/local/lib/xdg-desktop-portal" | "/usr/local/libexec/xdg-desktop-portal") => {
            Ok(InstallLayout {
                data: "/usr/local/share",
                sysconf: "/usr/local/etc",
            })
        }
        _ => bail!(
            "Portal frontend has a custom installation prefix; its complete provider registry cannot be verified"
        ),
    }
}

/// The complete search path the frontend consults, in upstream order.
/// `override_directory` replaces everything else when XDG_DESKTOP_PORTAL_DIR
/// is set; configuration files live in the base directories themselves.
struct SearchPath {
    override_directory: Option<PathBuf>,
    config_bases: Vec<PathBuf>,
    data_bases: Vec<PathBuf>,
}
impl SearchPath {
    /// Base directories consulted for `portals.conf` in priority order.
    /// Each directory contributes at most one configuration file.
    fn config_directories(&self) -> Vec<PathBuf> {
        match &self.override_directory {
            Some(directory) => vec![directory.clone()],
            None => self
                .config_bases
                .iter()
                .chain(&self.data_bases)
                .map(|base| base.join("xdg-desktop-portal"))
                .collect(),
        }
    }
    fn system_descriptor_directory(&self) -> PathBuf {
        self.override_directory.clone().unwrap_or_else(|| {
            self.data_bases
                .last()
                .expect("the compiled-in data directory is always present")
                .join("xdg-desktop-portal/portals")
        })
    }

    /// Directories holding `.portal` descriptors, in upstream load order;
    /// duplicate shadowed basenames stay observable.
    fn descriptor_directories(&self) -> Vec<PathBuf> {
        match &self.override_directory {
            Some(directory) => vec![directory.clone()],
            None => {
                let mut directories = Vec::new();
                for base in &self.data_bases {
                    let directory = base.join("xdg-desktop-portal/portals");
                    if !directories.contains(&directory) {
                        directories.push(directory);
                    }
                }
                directories
            }
        }
    }
}

fn search_paths(environment: &[u8], executable: &Path) -> Result<SearchPath> {
    if let Some(directory) = environment_value(environment, b"XDG_DESKTOP_PORTAL_DIR=")? {
        let directory = PathBuf::from(OsString::from_vec(directory.to_vec()));
        ensure!(
            directory.is_absolute(),
            "Portal override descriptor directory must be absolute"
        );
        return Ok(SearchPath {
            override_directory: Some(directory),
            config_bases: Vec::new(),
            data_bases: Vec::new(),
        });
    }
    let layout = install_layout(executable)?;
    let home = environment_path(environment, b"HOME=")?
        .context("Portal frontend has no home directory for its provider registry")?;
    let config_home =
        environment_path(environment, b"XDG_CONFIG_HOME=")?.unwrap_or_else(|| home.join(".config"));
    let config_dirs = environment_value(environment, b"XDG_CONFIG_DIRS=")?
        .filter(|value| !value.is_empty())
        .unwrap_or(b"/etc/xdg");
    let data_home = environment_path(environment, b"XDG_DATA_HOME=")?
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = environment_value(environment, b"XDG_DATA_DIRS=")?
        .filter(|value| !value.is_empty())
        .unwrap_or(b"/usr/local/share:/usr/share");
    let mut config_bases = vec![config_home];
    for directory in config_dirs.split(|byte| *byte == b':') {
        ensure!(
            config_bases.len() < MAX_DATA_DIRS,
            "Portal provider registry contains too many config directories"
        );
        config_bases.push(PathBuf::from(OsString::from_vec(directory.to_vec())));
    }
    // Newer frontends also consult their compile-time SYSCONFDIR. Keep the
    // base even when another entry resolves to it: the duplicated observation
    // is harmless and the conservative file set must not depend on it.
    config_bases.push(layout.sysconf.into());
    let mut data_bases = vec![data_home];
    for directory in data_dirs.split(|byte| *byte == b':') {
        ensure!(
            data_bases.len() < MAX_DATA_DIRS,
            "Portal provider registry contains too many data directories"
        );
        data_bases.push(PathBuf::from(OsString::from_vec(directory.to_vec())));
    }
    data_bases.push(layout.data.into());
    for base in config_bases.iter().chain(&data_bases) {
        ensure!(
            base.is_absolute(),
            "Portal provider search path must be absolute"
        );
    }
    Ok(SearchPath {
        override_directory: None,
        config_bases,
        data_bases,
    })
}

#[derive(Debug)]
struct Descriptor {
    path: PathBuf,
    name: String,
    /// Descriptor basename without the `.portal` suffix: `portals.conf` names
    /// backends by this source, so shadowed duplicates must stay distinguishable.
    source: String,
    interfaces: BTreeSet<String>,
    /// `None` when the descriptor has no `UseIn` list at all.
    use_in: Option<BTreeSet<String>>,
}

/// Ordered, validated desktop names: upstream splits `XDG_CURRENT_DESKTOP` on
/// ':', keeps entries of alphanumerics/'-'/'_' only, and lowercases them for
/// both configuration file names and `UseIn` matching. Empty entries are not
/// valid desktop names and are dropped, so an empty list is not one nameless
/// desktop.
fn desktop_names(environment: &[u8]) -> Result<Vec<String>> {
    let Some(value) = environment_value(environment, b"XDG_CURRENT_DESKTOP=")? else {
        return Ok(Vec::new());
    };
    value
        .split(|byte| *byte == b':')
        .filter_map(|name| {
            let valid = !name.is_empty()
                && name
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_');
            valid.then(|| {
                std::str::from_utf8(name)
                    .map(str::to_ascii_lowercase)
                    .context("XDG_CURRENT_DESKTOP is not UTF-8")
            })
        })
        .collect()
}
fn parse_descriptor(path: &Path, bytes: Vec<u8>) -> Result<Descriptor> {
    let text = String::from_utf8(bytes).context("Portal descriptor is not UTF-8")?;
    let mut defaults = Ini::new_cs().defaults();
    defaults.comment_symbols = vec!['#'];
    defaults.inline_comment_symbols = Some(Vec::new());
    defaults.delimiters = vec!['='];
    defaults.cascade_defaults = false;
    let mut ini = Ini::new_from_defaults(defaults);
    ini.read(text)
        .map_err(|error| anyhow!("invalid Portal descriptor: {error}"))?;
    let source = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .context("Portal descriptor has no usable basename")?
        .to_owned();
    let name = ini
        .get("portal", "DBusName")
        .context("Portal descriptor has no DBusName")?;
    zbus::names::WellKnownName::try_from(name.as_str())
        .context("invalid Portal descriptor bus name")?;
    let use_in = ini.get("portal", "UseIn").map(|list| {
        list.split(';')
            .filter(|name| !name.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    });
    let list = ini
        .get("portal", "Interfaces")
        .context("Portal descriptor has no Interfaces")?;
    let mut interfaces = BTreeSet::new();
    for interface in list
        .strip_suffix(';')
        .unwrap_or(&list)
        .split(';')
        .filter(|_| !list.is_empty())
    {
        zbus::names::InterfaceName::try_from(interface)
            .context("invalid Portal descriptor interface")?;
        ensure!(
            interface.starts_with("org.freedesktop.impl.portal."),
            "descriptor advertises a non-Portal interface"
        );
        interfaces.insert(interface.to_owned());
    }
    Ok(Descriptor {
        path: path.to_owned(),
        name,
        source,
        interfaces,
        use_in,
    })
}

// The configured interfaces are public on the frontend, but neither exposes
// its implementation's identity. Evaluate the registries independently so a
// descriptor visible only to newer frontends cannot hide an older route.
struct ProviderRegistry<'a> {
    system: Vec<&'a Descriptor>,
    xdg: Vec<&'a Descriptor>,
}

impl<'a> ProviderRegistry<'a> {
    fn new(descriptors: &'a [Descriptor], system_directory: &Path) -> Self {
        let mut sources = BTreeSet::new();
        Self {
            system: descriptors
                .iter()
                .filter(|descriptor| descriptor.path.parent() == Some(system_directory))
                .collect(),
            xdg: descriptors
                .iter()
                .filter(|descriptor| sources.insert(descriptor.source.as_str()))
                .collect(),
        }
    }

    fn provider(&self, configs: &[PortalConfiguration], desktops: &[String]) -> Result<Provider> {
        let current =
            |interface| select_name(&self.xdg, configs, desktops, interface, Selection::Listed);
        let remote = current(REMOTE_DESKTOP)?;
        let cast = current(SCREEN_CAST)?;
        ensure!(
            remote == cast,
            "Portal capture and input use different implementations; their common session scope cannot be verified"
        );
        let first_config = configs.first().map_or(&[][..], std::slice::from_ref);
        for (registry, configs, selection) in [
            // 1.16 ignores portals.conf and falls back to any backend.
            (&self.system, &[][..], Selection::Legacy),
            // 1.18 treats configured backends as an allowlist.
            (&self.system, first_config, Selection::Allowlist),
            // 1.20 and 1.21 honor list order and load only one config.
            (&self.system, first_config, Selection::Listed),
            (&self.xdg, first_config, Selection::Listed),
        ] {
            for interface in [REMOTE_DESKTOP, SCREEN_CAST] {
                let alternate = select_name(registry, configs, desktops, interface, selection)?;
                ensure!(
                    alternate == remote,
                    "Portal frontend versions select different providers for {interface}; the capture session cannot be verified"
                );
            }
        }
        match remote {
            "org.freedesktop.impl.portal.desktop.gnome" => Ok(Provider::Gnome),
            "org.freedesktop.impl.portal.desktop.kde" => Ok(Provider::Kde),
            _ => bail!(
                "Portal provider has no verified compositor identity adapter; supported routes are GNOME and KDE"
            ),
        }
    }
}

#[derive(Clone, Copy)]
enum Selection {
    Legacy,
    Allowlist,
    Listed,
}

fn unique_name<'a>(
    descriptors: impl IntoIterator<Item = &'a Descriptor>,
    interface: &str,
) -> Result<Option<&'a str>> {
    let names = descriptors
        .into_iter()
        .map(|descriptor| descriptor.name.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(
        names.len() <= 1,
        "Portal requires one installed provider for {interface}; found {}",
        names.len()
    );
    Ok(names.first().copied())
}

fn select_name<'a>(
    registry: &[&'a Descriptor],
    configs: &[PortalConfiguration],
    desktops: &[String],
    interface: &str,
    selection: Selection,
) -> Result<&'a str> {
    let candidates: Vec<_> = registry
        .iter()
        .copied()
        .filter(|descriptor| descriptor.interfaces.contains(interface))
        .collect();
    for config in configs {
        let preferred = config.preferred.get(interface);
        let default = config.default.as_ref();
        if let Some(backends) = preferred.or(default) {
            ensure!(
                !backends.iter().any(|backend| backend == "none"),
                "Portal configuration explicitly disables {interface}; its capture session cannot be verified"
            );
        }

        if matches!(selection, Selection::Allowlist) {
            if let Some(backends) = preferred.or(default)
                && let Some(name) = unique_name(
                    candidates.iter().copied().filter(|descriptor| {
                        backends
                            .iter()
                            .any(|backend| backend == "*" || backend == &descriptor.source)
                    }),
                    interface,
                )?
            {
                return Ok(name);
            }
        } else {
            for backends in preferred.into_iter().chain(default) {
                for backend in backends {
                    if backend == "*" {
                        if let Some(name) = unique_name(candidates.iter().copied(), interface)? {
                            return Ok(name);
                        }
                        break;
                    }
                    if let Some(name) = unique_name(
                        candidates
                            .iter()
                            .copied()
                            .filter(|descriptor| &descriptor.source == backend),
                        interface,
                    )? {
                        return Ok(name);
                    }
                }
            }
        }
    }

    // All releases check UseIn per interface. Keep all matching desktop names
    // as candidates so an ambiguous list does not require guessing the order.
    let matching = candidates.iter().copied().filter(|descriptor| {
        descriptor
            .use_in
            .as_ref()
            .is_some_and(|use_in| desktops.iter().any(|desktop| use_in.contains(desktop)))
    });
    if let Some(name) = unique_name(matching, interface)? {
        return Ok(name);
    }

    // Only the legacy frontend falls back to any backend; newer ones can fall
    // back to GTK. When the desktop is unknown, retaining the full candidate
    // set also keeps a possible GTK route in the check.
    if matches!(selection, Selection::Legacy) || desktops.is_empty() {
        return unique_name(candidates, interface)?
            .with_context(|| format!("Portal has no installed provider for {interface}"));
    }
    bail!(
        "Portal has no provider for the active desktop ({}); {} installed descriptors do not list it in UseIn or implement {interface}",
        desktops.join(":"),
        registry.len()
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp(u64, u64, u64, i64, i64, i64, i64);
fn stamp(metadata: &Metadata) -> Stamp {
    Stamp(
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}
#[derive(Debug, Default)]
struct Snapshot {
    entries: BTreeMap<PathBuf, Option<(Stamp, Stamp)>>,
    // Ancestor contents are not the provider registry. Retain their identity
    // and symlink routing, without treating unrelated data writes as changes.
    ancestors: BTreeMap<PathBuf, AncestorStamp>,
}
type NodeIdentity = (u64, u64, u32, u32);
type AncestorStamp = (Option<Stamp>, NodeIdentity);
fn node_identity(metadata: &Metadata) -> NodeIdentity {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.uid(),
        metadata.mode(),
    )
}
impl Snapshot {
    fn observe_ancestors(&mut self, path: &Path, started: SystemTime) -> Result<()> {
        for ancestor in path.ancestors().skip(1).take(64) {
            if self.entries.contains_key(ancestor) || self.ancestors.contains_key(ancestor) {
                continue;
            }
            let link = match fs::symlink_metadata(ancestor) {
                Ok(link) => link,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.entries.insert(ancestor.to_owned(), None);
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let link_stamp = if link.file_type().is_symlink() {
                let changed = UNIX_EPOCH
                    .checked_add(Duration::new(
                        u64::try_from(link.ctime())?,
                        u32::try_from(link.ctime_nsec())?,
                    ))
                    .context("invalid Portal path change time")?;
                ensure!(
                    link.modified()? <= started && changed <= started,
                    "Portal provider path {} was redirected after frontend startup",
                    ancestor.display()
                );
                Some(stamp(&link))
            } else {
                None
            };
            let target = fs::metadata(ancestor)?;
            ensure!(
                target.is_dir(),
                "Portal provider ancestor is not a directory"
            );
            self.ancestors
                .insert(ancestor.to_owned(), (link_stamp, node_identity(&target)));
        }
        ensure!(
            self.entries.len() + self.ancestors.len() <= 1024,
            "Portal provider registry has too many paths"
        );
        Ok(())
    }
    fn observe(&mut self, path: &Path, started: SystemTime) -> Result<Option<Metadata>> {
        let link = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.entries.insert(path.to_owned(), None);
                self.observe_ancestors(path, started)?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = fs::metadata(path)?;
        for entry in [&link, &metadata] {
            let changed = UNIX_EPOCH
                .checked_add(Duration::new(
                    u64::try_from(entry.ctime())?,
                    u32::try_from(entry.ctime_nsec())?,
                ))
                .context("Portal descriptor has an invalid change time")?;
            ensure!(
                entry.modified()? <= started && changed <= started,
                "Portal provider registry {} changed after frontend startup; restart the desktop Portal before connecting",
                path.display()
            );
        }
        self.entries
            .insert(path.to_owned(), Some((stamp(&link), stamp(&metadata))));
        ensure!(
            self.entries.len() + self.ancestors.len() <= 1024,
            "Portal provider registry has too many paths"
        );
        self.observe_ancestors(path, started)?;
        Ok(Some(metadata))
    }
    fn verify(&self) -> Result<()> {
        for (path, expected) in &self.entries {
            let actual = match fs::symlink_metadata(path) {
                Ok(link) => Some((stamp(&link), stamp(&fs::metadata(path)?))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            ensure!(
                &actual == expected,
                "Portal provider registry changed during target validation"
            );
        }
        for (path, expected) in &self.ancestors {
            let link = fs::symlink_metadata(path)?;
            let link_stamp = link.file_type().is_symlink().then(|| stamp(&link));
            let actual = (link_stamp, node_identity(&fs::metadata(path)?));
            ensure!(
                &actual == expected,
                "Portal provider path changed during target validation"
            );
        }
        Ok(())
    }
}

fn load_descriptors(
    snapshot: &mut Snapshot,
    directories: &[PathBuf],
    started: SystemTime,
) -> Result<Vec<Descriptor>> {
    ensure!(
        directories.len() <= MAX_DATA_DIRS + 1,
        "too many Portal descriptor directories"
    );
    let mut descriptors = Vec::new();
    let mut entries = 0;
    for directory in directories {
        let Some(metadata) = snapshot.observe(directory, started)? else {
            continue;
        };
        ensure!(
            metadata.is_dir(),
            "Portal descriptor path is not a directory"
        );
        for entry in fs::read_dir(directory)? {
            entries += 1;
            ensure!(
                entries <= MAX_DIRECTORY_ENTRIES,
                "Portal provider registry exceeds the directory entry limit"
            );
            let path = entry?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "portal")
            {
                continue;
            }
            let metadata = snapshot
                .observe(&path, started)?
                .context("Portal descriptor disappeared")?;
            ensure!(
                metadata.is_file(),
                "Portal descriptor is not a regular file"
            );
            // Count shadowed basenames too: old frontends may ignore the
            // higher-priority XDG descriptor and load the compiled one instead.
            descriptors.push(
                parse_descriptor(&path, read_bounded(&path, MAX_DESCRIPTOR_BYTES)?)
                    .with_context(|| format!("read Portal descriptor {}", path.display()))?,
            );
        }
    }
    Ok(descriptors)
}

/// A `portals.conf` file's case-sensitive `[preferred]` entries. GLib
/// merges repeated groups and reads the last value for duplicate keys.
#[derive(Debug, Default)]
struct PortalConfiguration {
    preferred: BTreeMap<String, Vec<String>>,
    default: Option<Vec<String>>,
}
fn parse_configuration(bytes: Vec<u8>) -> Result<Option<PortalConfiguration>> {
    let text = String::from_utf8(bytes).context("Portal configuration is not UTF-8")?;
    let mut preferred: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut default = None;
    let mut section_seen = false;
    let mut preferred_seen = false;
    let mut in_preferred = false;
    ensure!(
        !text.contains('\0'),
        "Portal configuration contains a NUL byte"
    );
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim_ascii_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line
            .trim_ascii_end()
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            ensure!(
                !section.is_empty() && !section.contains(['[', ']']),
                "invalid Portal configuration group on line {}",
                number + 1
            );
            section_seen = true;
            in_preferred = section == "preferred";
            preferred_seen |= in_preferred;
            continue;
        }
        let (key, value) = line.split_once('=').with_context(|| {
            format!(
                "invalid Portal configuration line {}: expected key=value",
                number + 1
            )
        })?;
        let key = key.trim_ascii_end();
        ensure!(
            section_seen && !key.is_empty(),
            "invalid Portal configuration key on line {}",
            number + 1
        );
        if !in_preferred {
            continue;
        }
        ensure!(
            !key.contains(['[', ']']) && !value.contains('\\'),
            "Portal configuration has a localized key or escaped backend list on line {}; its route cannot be verified",
            number + 1
        );
        let backends = value
            .trim_ascii_start()
            .split(';')
            .filter(|backend| !backend.is_empty())
            .map(str::to_owned)
            .collect();
        if key == "default" {
            default = Some(backends);
        } else {
            preferred.insert(key.to_owned(), backends);
        }
    }
    Ok(preferred_seen.then_some(PortalConfiguration { preferred, default }))
}

/// The configuration files a newer frontend consults, in its search order.
/// Every directory contributes at most one file: the first applicable
/// `<desktop>-portals.conf` in desktop order, else `portals.conf`. Absent
/// files and absent parent directories are still observed, so creating either
/// one later invalidates this snapshot.
fn load_configurations(
    snapshot: &mut Snapshot,
    search: &SearchPath,
    desktops: &[String],
    started: SystemTime,
) -> Result<Vec<PortalConfiguration>> {
    let directories = search.config_directories();
    ensure!(
        directories.len() <= 2 * MAX_DATA_DIRS + 3,
        "too many Portal configuration directories"
    );
    let mut configurations = Vec::new();
    for directory in directories {
        snapshot.observe(&directory, started)?;
        let mut candidates: Vec<PathBuf> = desktops
            .iter()
            .map(|desktop| directory.join(format!("{desktop}-portals.conf")))
            .collect();
        candidates.push(directory.join("portals.conf"));
        // Observe every candidate even after one is found: creating a
        // higher-priority configuration later must invalidate the snapshot.
        let mut found = None;
        for path in candidates {
            let metadata = snapshot.observe(&path, started)?;
            if found.is_some() {
                continue;
            }
            let Some(metadata) = metadata else {
                continue;
            };
            ensure!(
                metadata.is_file(),
                "Portal configuration is not a regular file"
            );
            if let Some(configuration) =
                parse_configuration(read_bounded(&path, MAX_DESCRIPTOR_BYTES)?)
                    .with_context(|| format!("read Portal configuration {}", path.display()))?
            {
                configurations.push(configuration);
                found = Some(());
            }
        }
    }
    Ok(configurations)
}

fn process_started(pid: u32) -> Result<SystemTime> {
    let process = read_bounded(&PathBuf::from(format!("/proc/{pid}/stat")), 8 * 1024)?;
    let suffix = process
        .iter()
        .rposition(|byte| *byte == b')')
        .context("invalid Portal process stat")?;
    let ticks: u64 = std::str::from_utf8(&process[suffix + 1..])?
        .split_ascii_whitespace()
        .nth(19)
        .context("Portal process stat has no start time")?
        .parse()?;
    let frequency = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })?;
    ensure!(frequency > 0, "process clock frequency is unavailable");
    let statistics = read_bounded(Path::new("/proc/stat"), 2 * 1024 * 1024)?;
    let boot: u64 = std::str::from_utf8(&statistics)?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))
        .context("system boot time is unavailable")?
        .trim()
        .parse()?;
    // btime is rounded down: using that lower bound may conservatively reject
    // a freshly changed registry, but cannot accept one newer than the process.
    UNIX_EPOCH
        .checked_add(Duration::from_secs(boot))
        .and_then(|boot| boot.checked_add(Duration::from_secs(ticks / frequency)))
        .context("invalid Portal process start time")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, FileTimes};

    fn unique_provider(
        descriptors: &[Descriptor],
        configs: &[PortalConfiguration],
        desktops: &[String],
    ) -> Result<Provider> {
        ProviderRegistry::new(descriptors, Path::new("")).provider(configs, desktops)
    }

    fn descriptor(name: &str, interfaces: &[&str]) -> Descriptor {
        descriptor_for(name, interfaces, None)
    }
    fn descriptor_for(name: &str, interfaces: &[&str], use_in: Option<&str>) -> Descriptor {
        let source = name.rsplit('.').next().unwrap();
        descriptor_from(source, name, interfaces, use_in)
    }
    fn descriptor_from(
        source: &str,
        name: &str,
        interfaces: &[&str],
        use_in: Option<&str>,
    ) -> Descriptor {
        let use_in = use_in
            .map(|list| format!("UseIn={list};\n"))
            .unwrap_or_default();
        parse_descriptor(
            Path::new(&format!("{source}.portal")),
            format!(
                "# fixture\n[portal]\nDBusName={name}\nInterfaces={};\n{use_in}",
                interfaces.join(";")
            )
            .into_bytes(),
        )
        .unwrap()
    }
    fn configuration(entries: &[(&str, &str)]) -> PortalConfiguration {
        parse_configuration(
            entries
                .iter()
                .fold("[preferred]\n".to_owned(), |text, (key, value)| {
                    format!("{text}{key}={value};\n")
                })
                .into_bytes(),
        )
        .unwrap()
        .unwrap()
    }
    fn desktops(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_ascii_lowercase()).collect()
    }
    fn target() -> SessionTarget {
        SessionTarget {
            runtime: "/run/user/1000".into(),
            wayland_socket: Some("/run/user/1000/wayland-2".into()),
            ..Default::default()
        }
    }
    const MATCHING: &[u8] = b"WAYLAND_DISPLAY=wayland-2\0XDG_RUNTIME_DIR=/run/user/1000\0";

    #[test]
    fn unique_gnome_and_kde_routes_are_admitted_with_exact_compositor_evidence() {
        for provider in [Provider::Gnome, Provider::Kde] {
            let descriptors = [
                descriptor(provider.bus_name(), &[REMOTE_DESKTOP, SCREEN_CAST]),
                descriptor(
                    "org.freedesktop.impl.portal.desktop.gtk",
                    &["org.freedesktop.impl.portal.FileChooser"],
                ),
            ];
            assert_eq!(
                unique_provider(&descriptors, &[], &desktops(&[])).unwrap(),
                provider
            );
            exact_environment(&target(), MATCHING, "implementation").unwrap();
            for service in provider.compositor_services() {
                verify_compositor_peer(service, 2345, 1000, 2345, 1000).unwrap();
            }
        }
    }

    #[test]
    fn matching_frontend_or_login_cannot_admit_a_different_compositor() {
        exact_environment(&target(), MATCHING, "frontend").unwrap();
        for provider in [Provider::Gnome, Provider::Kde] {
            for service in provider.compositor_services() {
                assert!(
                    verify_compositor_peer(service, 2345, 1000, 6789, 1000)
                        .unwrap_err()
                        .to_string()
                        .contains("selected Wayland socket belongs to process 2345")
                );
                assert!(verify_compositor_peer(service, 2345, 1000, 2345, 2000).is_err());
            }
        }
        assert!(
            exact_environment(
                &target(),
                b"WAYLAND_DISPLAY=wayland-0\0XDG_RUNTIME_DIR=/run/user/1000\0XDG_SESSION_ID=1\0",
                "implementation"
            )
            .unwrap_err()
            .to_string()
            .contains("advertises /run/user/1000/wayland-0")
        );
    }

    #[test]
    fn competing_or_split_input_and_capture_providers_are_unavailable() {
        let gnome = descriptor(Provider::Gnome.bus_name(), &[REMOTE_DESKTOP, SCREEN_CAST]);
        let kde = descriptor(Provider::Kde.bus_name(), &[REMOTE_DESKTOP, SCREEN_CAST]);
        let error = unique_provider(&[gnome, kde], &[], &desktops(&[]))
            .unwrap_err()
            .to_string();
        assert!(error.contains(REMOTE_DESKTOP) && error.contains("found 2"));
        let split = [
            descriptor(Provider::Gnome.bus_name(), &[REMOTE_DESKTOP]),
            descriptor(Provider::Kde.bus_name(), &[SCREEN_CAST]),
        ];
        assert!(
            unique_provider(&split, &[], &desktops(&[]))
                .unwrap_err()
                .to_string()
                .contains("different implementations")
        );
        let custom = descriptor("com.example.CustomPortal", &[REMOTE_DESKTOP, SCREEN_CAST]);
        assert!(
            unique_provider(&[custom], &[], &desktops(&[]))
                .unwrap_err()
                .to_string()
                .contains("no verified compositor identity adapter")
        );
    }

    #[test]
    fn custom_provider_cannot_be_hidden_behind_a_desktop_name() {
        let descriptors = [
            descriptor(Provider::Gnome.bus_name(), &[REMOTE_DESKTOP, SCREEN_CAST]),
            descriptor("com.example.CustomPortal", &[SCREEN_CAST]),
        ];
        let error = unique_provider(&descriptors, &[], &desktops(&[]))
            .unwrap_err()
            .to_string();
        assert!(error.contains(SCREEN_CAST) && error.contains("found 2"));
    }

    #[test]
    fn use_in_excludes_the_provider_this_frontend_cannot_select() {
        // Ubuntu/Debian install both backends; each one names its own desktop.
        let kde = descriptor_for(
            Provider::Kde.bus_name(),
            &[REMOTE_DESKTOP, SCREEN_CAST],
            Some("KDE"),
        );
        let gnome = descriptor_for(
            Provider::Gnome.bus_name(),
            &[REMOTE_DESKTOP, SCREEN_CAST],
            Some("gnome"),
        );
        let descriptors = [kde, gnome];
        assert_eq!(
            unique_provider(&descriptors, &[], &desktops(&["KDE"])).unwrap(),
            Provider::Kde
        );
        // Distributions such as Ubuntu add their own name to the front of the list.
        assert_eq!(
            unique_provider(
                &descriptors,
                &[],
                &desktop_names(b"XDG_CURRENT_DESKTOP=ubuntu:GNOME\0").unwrap()
            )
            .unwrap(),
            Provider::Gnome
        );
        // A session that matches both UseIn lists keeps the ambiguity refusal.
        let error = unique_provider(&descriptors, &[], &desktops(&["KDE", "GNOME"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains(REMOTE_DESKTOP) && error.contains("found 2"));
        // Without UseIn the descriptor never matches the fallback, as newer
        // frontends read the list.
        let wildcard = descriptor(Provider::Gnome.bus_name(), &[REMOTE_DESKTOP, SCREEN_CAST]);
        let error = unique_provider(&[wildcard], &[], &desktops(&["KDE"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no provider for the active desktop (kde)"));
    }

    #[test]
    fn portals_conf_selects_the_provider_use_in_would_exclude() {
        // The reported scenario: a KDE session installs both backends, and the
        // user configuration explicitly selects the GNOME one.
        let kde = descriptor_for(
            Provider::Kde.bus_name(),
            &[REMOTE_DESKTOP, SCREEN_CAST],
            Some("KDE"),
        );
        let gnome = descriptor_for(
            Provider::Gnome.bus_name(),
            &[REMOTE_DESKTOP, SCREEN_CAST],
            Some("gnome"),
        );
        let descriptors = [kde, gnome];
        let desktops = desktops(&["KDE"]);
        let config = configuration(&[("default", "gnome")]);
        assert!(
            unique_provider(&descriptors, std::slice::from_ref(&config), &desktops)
                .unwrap_err()
                .to_string()
                .contains("Portal frontend versions select different providers")
        );
        // Old frontends cannot reach the configured route either; when the
        // legacy fallback would agree, the configuration is honored.
        let gnome_only = [descriptor_from(
            "gnome",
            Provider::Gnome.bus_name(),
            &[REMOTE_DESKTOP, SCREEN_CAST],
            Some("KDE"),
        )];
        assert_eq!(
            unique_provider(&gnome_only, std::slice::from_ref(&config), &desktops).unwrap(),
            Provider::Gnome
        );
        // An interface-specific entry takes precedence over `default`, and the
        // configuration names backends regardless of their UseIn list.
        let config = configuration(&[(REMOTE_DESKTOP, "kde"), ("default", "gnome")]);
        assert!(
            unique_provider(&descriptors, std::slice::from_ref(&config), &desktops)
                .unwrap_err()
                .to_string()
                .contains("different implementations")
        );
        let config = configuration(&[(REMOTE_DESKTOP, "kde"), (SCREEN_CAST, "kde")]);
        assert_eq!(
            unique_provider(&descriptors, std::slice::from_ref(&config), &desktops).unwrap(),
            Provider::Kde
        );
        // `none` disables the interface instead of selecting a provider.
        let config = configuration(&[(SCREEN_CAST, "none")]);
        assert!(
            unique_provider(&descriptors, std::slice::from_ref(&config), &desktops)
                .unwrap_err()
                .to_string()
                .contains("explicitly disables")
        );
        // An unknown or unsupporting name is skipped, like upstream.
        let config = configuration(&[("default", "missing;gnome")]);
        assert!(
            unique_provider(&descriptors, std::slice::from_ref(&config), &desktops)
                .unwrap_err()
                .to_string()
                .contains("Portal frontend versions select different providers")
        );
        // A later configuration is consulted when an earlier one does not
        // cover the interface, mirroring the newest frontend.
        let first = configuration(&[("org.freedesktop.impl.portal.FileChooser", "gtk")]);
        let second = configuration(&[("default", "kde")]);
        assert_eq!(
            unique_provider(&descriptors, &[first, second], &desktops).unwrap(),
            Provider::Kde
        );
        // A configured wildcard must resolve to one provider.
        let config = configuration(&[("default", "*")]);
        assert!(
            unique_provider(&descriptors, std::slice::from_ref(&config), &desktops)
                .unwrap_err()
                .to_string()
                .contains("found 2")
        );
    }

    #[test]
    fn unlisted_desktop_reports_no_provider_instead_of_a_foreign_count() {
        let descriptors = [
            descriptor_for(
                Provider::Kde.bus_name(),
                &[REMOTE_DESKTOP, SCREEN_CAST],
                Some("KDE"),
            ),
            descriptor_for(
                Provider::Gnome.bus_name(),
                &[REMOTE_DESKTOP, SCREEN_CAST],
                Some("GNOME"),
            ),
        ];
        let error = unique_provider(&descriptors, &[], &desktops(&["sway"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no provider for the active desktop (sway)"));
        assert!(error.contains("2 installed descriptors"));
    }

    #[test]
    fn desktop_names_split_colon_lists_case_insensitively() {
        assert_eq!(
            desktop_names(b"XDG_CURRENT_DESKTOP=KDE:GNOME\0").unwrap(),
            desktops(&["kde", "gnome"])
        );
        assert_eq!(
            desktop_names(b"XDG_CURRENT_DESKTOP=\0").unwrap(),
            desktops(&[])
        );
        assert_eq!(desktop_names(b"HOME=/home/test\0").unwrap(), desktops(&[]));
        assert!(
            desktop_names(b"XDG_CURRENT_DESKTOP=KDE\0XDG_CURRENT_DESKTOP=GNOME\0")
                .unwrap_err()
                .to_string()
                .contains("ambiguous XDG_CURRENT_DESKTOP")
        );
    }

    #[test]
    fn descriptors_keep_case_and_semicolon_lists_without_inline_comments() {
        let parsed = descriptor(Provider::Gnome.bus_name(), &[REMOTE_DESKTOP, SCREEN_CAST]);
        assert!(parsed.interfaces.contains(SCREEN_CAST));
        assert!(parsed.interfaces.contains(REMOTE_DESKTOP));
        assert!(parse_descriptor(Path::new("test.portal"), b"[portal]\ndbusname=org.example.Test\nInterfaces=org.freedesktop.impl.portal.ScreenCast;\n".to_vec()).is_err());
        assert!(
            parse_descriptor(
                Path::new("gnome.portal"),
                format!(
                    "[portal]\nDBusName={}\nInterfaces={REMOTE_DESKTOP};;{SCREEN_CAST};\n",
                    Provider::Gnome.bus_name()
                )
                .into_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn process_environment_cannot_override_the_verified_display() {
        for environment in [
            b"WAYLAND_DISPLAY=wayland-2\0XDG_RUNTIME_DIR=/run/user/2000\0".as_slice(),
            b"WAYLAND_DISPLAY=wayland-2\0XDG_RUNTIME_DIR=/run/user/1000\0WAYLAND_SOCKET=7\0",
            b"WAYLAND_DISPLAY=wayland-2\0WAYLAND_DISPLAY=wayland-0\0XDG_RUNTIME_DIR=/run/user/1000\0",
        ] { assert!(exact_environment(&target(), environment, "implementation").is_err()); }
        exact_environment(
            &target(),
            b"WAYLAND_DISPLAY=/run/user/1000/wayland-2\0XDG_RUNTIME_DIR=/run/user/1000\0",
            "implementation",
        )
        .unwrap();
        for arguments in [
            b"portal\0-platform\0wayland:other\0".as_slice(),
            b"portal\0--display=:3\0",
        ] {
            assert!(ensure_no_display_override(arguments).is_err());
        }
        ensure_no_display_override(b"portal\0--verbose\0").unwrap();
    }

    #[test]
    fn descriptor_search_uses_frontend_environment_and_full_upstream_registry() {
        let search = search_paths(
            b"HOME=/home/test\0XDG_DATA_DIRS=/custom/share\0",
            Path::new("/usr/lib/xdg-desktop-portal"),
        )
        .unwrap();
        assert_eq!(
            search.descriptor_directories(),
            vec![
                PathBuf::from("/home/test/.local/share/xdg-desktop-portal/portals"),
                "/custom/share/xdg-desktop-portal/portals".into(),
                "/usr/share/xdg-desktop-portal/portals".into()
            ]
        );
        assert_eq!(
            search.config_directories(),
            vec![
                PathBuf::from("/home/test/.config/xdg-desktop-portal"),
                "/etc/xdg/xdg-desktop-portal".into(),
                "/etc/xdg-desktop-portal".into(),
                "/home/test/.local/share/xdg-desktop-portal".into(),
                "/custom/share/xdg-desktop-portal".into(),
                "/usr/share/xdg-desktop-portal".into()
            ]
        );
        let search = search_paths(
            b"XDG_DESKTOP_PORTAL_DIR=/override\0",
            Path::new("/custom/portal"),
        )
        .unwrap();
        assert_eq!(search.override_directory, Some(PathBuf::from("/override")));
        assert_eq!(
            search.config_directories(),
            vec![PathBuf::from("/override")]
        );
        assert!(search_paths(b"HOME=/home/test\0", Path::new("/custom/portal")).is_err());
        assert!(
            search_paths(
                b"XDG_DESKTOP_PORTAL_DIR=relative\0",
                Path::new("/usr/lib/xdg-desktop-portal")
            )
            .is_err()
        );
    }

    fn write_provider(directory: &Path, name: &str) {
        fs::write(
            directory.join("desktop.portal"),
            format!("[portal]\nDBusName={name}\nInterfaces={REMOTE_DESKTOP};{SCREEN_CAST};\n"),
        )
        .unwrap();
    }
    fn inventory(directories: &[PathBuf], desktops: &[String]) -> Result<(Provider, Snapshot)> {
        inventory_with_start(directories, SystemTime::now() + Duration::from_secs(2)).and_then(
            |(descriptors, snapshot)| {
                let provider = ProviderRegistry::new(&descriptors, directories.last().unwrap())
                    .provider(&[], desktops)?;
                Ok((provider, snapshot))
            },
        )
    }
    fn inventory_with_start(
        directories: &[PathBuf],
        started: SystemTime,
    ) -> Result<(Vec<Descriptor>, Snapshot)> {
        let mut snapshot = Snapshot::default();
        let descriptors = load_descriptors(&mut snapshot, directories, started)?;
        snapshot.verify()?;
        Ok((descriptors, snapshot))
    }

    #[test]
    fn inventory_admits_unchanged_descriptors_and_duplicate_same_provider() {
        let root = tempfile::tempdir().unwrap();
        let high = root.path().join("high");
        let low = root.path().join("low");
        fs::create_dir(&high).unwrap();
        fs::create_dir(&low).unwrap();
        write_provider(&high, Provider::Gnome.bus_name());
        write_provider(&low, Provider::Gnome.bus_name());
        let (provider, snapshot) = inventory(&[high, low], &desktops(&[])).unwrap();
        assert_eq!(provider, Provider::Gnome);
        snapshot.verify().unwrap();
    }

    #[test]
    fn user_basename_override_cannot_hide_a_system_provider_on_old_frontends() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let system = root.path().join("system");
        fs::create_dir(&user).unwrap();
        fs::create_dir(&system).unwrap();
        // A modern frontend could use user's desktop.portal (GNOME), while
        // an old frontend ignores it and uses system desktop.portal (KDE).
        write_provider(&user, Provider::Gnome.bus_name());
        write_provider(&system, Provider::Kde.bus_name());
        let error = inventory(&[user, system], &desktops(&[]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Portal frontend versions select different providers"));
    }

    #[test]
    fn post_start_descriptor_and_directory_changes_are_unavailable() {
        let root = tempfile::tempdir().unwrap();
        write_provider(root.path(), Provider::Gnome.bus_name());
        let started = SystemTime::now() + Duration::from_secs(2);
        let future = started + Duration::from_secs(1);
        let file = root.path().join("desktop.portal");
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(FileTimes::new().set_modified(future))
            .unwrap();
        assert!(
            inventory_with_start(&[root.path().to_owned()], started)
                .unwrap_err()
                .to_string()
                .contains("changed after frontend startup")
        );
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(FileTimes::new().set_modified(started - Duration::from_secs(1)))
            .unwrap();
        File::open(root.path())
            .unwrap()
            .set_times(FileTimes::new().set_modified(future))
            .unwrap();
        // A directory mtime change alone must also be refused.
        assert!(
            inventory_with_start(&[root.path().to_owned()], started)
                .unwrap_err()
                .to_string()
                .contains("changed after frontend startup")
        );
    }

    #[test]
    fn inventory_change_during_validation_is_unavailable() {
        let root = tempfile::tempdir().unwrap();
        write_provider(root.path(), Provider::Kde.bus_name());
        let started = SystemTime::now() + Duration::from_secs(2);
        let (_, snapshot) = inventory(&[root.path().to_owned()], &desktops(&[])).unwrap();
        fs::write(root.path().join("new.portal"), "new provider").unwrap();
        // Directory writes can share a filesystem clock tick.
        File::open(root.path())
            .unwrap()
            .set_times(FileTimes::new().set_modified(started + Duration::from_secs(1)))
            .unwrap();
        assert!(
            snapshot
                .verify()
                .unwrap_err()
                .to_string()
                .contains("changed during target validation")
        );
    }

    #[test]
    fn configuration_files_follow_desktop_order_and_directory_priority() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let system = root.path().join("system");
        fs::create_dir(&user).unwrap();
        fs::create_dir(&system).unwrap();
        // The loader appends the xdg-desktop-portal level to each base.
        let search = SearchPath {
            override_directory: None,
            config_bases: vec![user.clone()],
            data_bases: vec![system.clone()],
        };
        let mut snapshot = Snapshot::default();
        let started = SystemTime::now() + Duration::from_secs(2);
        let configs = load_configurations(
            &mut snapshot,
            &search,
            &desktops(&["KDE", "GNOME"]),
            started,
        )
        .unwrap();
        assert!(configs.is_empty());
        let write = |base: &Path, name: &str, text: &str| {
            let directory = base.join("xdg-desktop-portal");
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(name), text).unwrap();
        };
        write(&user, "gnome-portals.conf", "[preferred]\ndefault=gnome;\n");
        write(&user, "kde-portals.conf", "[preferred]\ndefault=kde;\n");
        write(&system, "portals.conf", "[preferred]\ndefault=kde;\n");
        let mut snapshot = Snapshot::default();
        let configs = load_configurations(
            &mut snapshot,
            &search,
            &desktops(&["KDE", "GNOME"]),
            started,
        )
        .unwrap();
        // The first desktop name wins inside a directory; the generic file in
        // the lower-priority directory is still consulted afterwards.
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].default, Some(vec!["kde".to_owned()]));
        assert_eq!(configs[1].default, Some(vec!["kde".to_owned()]));
        snapshot.verify().unwrap();
        // A file without a [preferred] group is skipped, allowing the next
        // desktop-specific configuration in the directory to take effect.
        fs::write(
            user.join("xdg-desktop-portal/kde-portals.conf"),
            "[other]\ndefault=kde;\n",
        )
        .unwrap();
        let mut snapshot = Snapshot::default();
        let configs = load_configurations(
            &mut snapshot,
            &search,
            &desktops(&["KDE", "GNOME"]),
            started,
        )
        .unwrap();
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].default, Some(vec!["gnome".to_owned()]));
        assert_eq!(configs[1].default, Some(vec!["kde".to_owned()]));
        // Appearing and disappearing configuration is caught by the snapshot.
        fs::remove_file(user.join("xdg-desktop-portal/gnome-portals.conf")).unwrap();
        assert!(
            snapshot
                .verify()
                .unwrap_err()
                .to_string()
                .contains("changed during target validation")
        );
        // GKeyFile lookups return the last value of a duplicate key.
        let config = parse_configuration(b"[preferred]\ndefault=kde;\ndefault=gnome;\n".to_vec())
            .unwrap()
            .unwrap();
        assert_eq!(config.default, Some(vec!["gnome".to_owned()]));
    }

    #[test]
    fn disk_and_process_evidence_reads_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("oversize");
        fs::write(&path, vec![b'x'; 32]).unwrap();
        assert!(read_bounded(&path, 32).is_ok());
        assert!(
            read_bounded(&path, 31)
                .unwrap_err()
                .to_string()
                .contains("read limit")
        );
        assert!(process_started(std::process::id()).unwrap() < SystemTime::now());
    }
}
