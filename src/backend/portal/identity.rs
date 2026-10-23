//! Conservative Portal routing for an explicitly selected Wayland socket.
//!
//! xdg-desktop-portal's `register_portal` loads DBusName/Interfaces from .portal
//! files; `xdp_portal_config_find_impl_config_by_iface` and the legacy fallbacks
//! select only from that registry. Configuration cannot name an implementation
//! outside it. We count every descriptor in a conservative search-path superset,
//! including duplicate basenames: older frontends load only compiled DATADIR,
//! while newer ones support XDG data paths and basename shadowing. A user file
//! must not hide a competing system provider from this check. One provider for
//! both interfaces therefore proves the same route on both frontend versions.
//! See upstream desktop-portal/xdp-portal-config.c and older src/xdp-impl.c.
//!
//! Scope still needs independent proof: the implementation's exact Wayland
//! environment AND the selected socket's peer PID must match Mutter's two
//! compositor services or KWin. The frontend environment alone never suffices.
//! Multiple providers, custom installation prefixes/implementations, changed
//! descriptors, and unavailable identity evidence remain explicitly unavailable.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, Metadata, OpenOptions},
    io::Read,
    os::unix::{ffi::OsStringExt, fs::MetadataExt, fs::OpenOptionsExt},
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
    let directories = descriptor_directories(&frontend_environment, &frontend_executable)?;
    let (provider, snapshot) = installed_provider(&directories, started)?;

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
        verify_compositor_peer(service, compositor_pid, uid, compositor.pid, compositor.uid)?;
        owners.push((service, compositor.owner));
    }
    snapshot.verify()?;
    let proof = ScopeProof { owners };
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

fn descriptor_directories(environment: &[u8], executable: &Path) -> Result<Vec<PathBuf>> {
    if let Some(directory) = environment_value(environment, b"XDG_DESKTOP_PORTAL_DIR=")? {
        let directory = PathBuf::from(OsString::from_vec(directory.to_vec()));
        ensure!(
            directory.is_absolute(),
            "Portal override descriptor directory must be absolute"
        );
        return Ok(vec![directory]);
    }
    let data_home = match environment_path(environment, b"XDG_DATA_HOME=")? {
        Some(path) => path,
        None => environment_path(environment, b"HOME=")?
            .context("Portal frontend has no home directory for its provider registry")?
            .join(".local/share"),
    };
    let data_dirs = environment_value(environment, b"XDG_DATA_DIRS=")?
        .filter(|value| !value.is_empty())
        .unwrap_or(b"/usr/local/share:/usr/share");
    let mut bases = vec![data_home];
    for directory in data_dirs.split(|byte| *byte == b':') {
        ensure!(
            bases.len() < MAX_DATA_DIRS,
            "Portal provider registry contains too many data directories"
        );
        bases.push(PathBuf::from(OsString::from_vec(directory.to_vec())));
    }
    // Upstream always adds its compile-time DATADIR, even with XDG overrides.
    // Do not guess that prefix for a custom build or a deleted executable.
    let prefix = match executable.to_str() {
        Some(
            "/usr/lib/xdg-desktop-portal"
            | "/usr/libexec/xdg-desktop-portal"
            | "/usr/lib/x86_64-linux-gnu/xdg-desktop-portal"
            | "/usr/lib/aarch64-linux-gnu/xdg-desktop-portal",
        ) => "/usr/share",
        Some("/usr/local/lib/xdg-desktop-portal" | "/usr/local/libexec/xdg-desktop-portal") => {
            "/usr/local/share"
        }
        _ => bail!(
            "Portal frontend has a custom installation prefix; its complete provider registry cannot be verified"
        ),
    };
    bases.push(prefix.into());
    let mut directories = Vec::new();
    for base in bases {
        ensure!(
            base.is_absolute(),
            "Portal provider data directory must be absolute"
        );
        let directory = base.join("xdg-desktop-portal/portals");
        if !directories.contains(&directory) {
            directories.push(directory);
        }
    }
    Ok(directories)
}

#[derive(Debug)]
struct Descriptor {
    name: String,
    interfaces: BTreeSet<String>,
}
fn parse_descriptor(bytes: Vec<u8>) -> Result<Descriptor> {
    let text = String::from_utf8(bytes).context("Portal descriptor is not UTF-8")?;
    let mut defaults = Ini::new_cs().defaults();
    defaults.comment_symbols = vec!['#'];
    defaults.inline_comment_symbols = Some(Vec::new());
    defaults.delimiters = vec!['='];
    defaults.cascade_defaults = false;
    let mut ini = Ini::new_from_defaults(defaults);
    ini.read(text)
        .map_err(|error| anyhow!("invalid Portal descriptor: {error}"))?;
    let name = ini
        .get("portal", "DBusName")
        .context("Portal descriptor has no DBusName")?;
    zbus::names::WellKnownName::try_from(name.as_str())
        .context("invalid Portal descriptor bus name")?;
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
    Ok(Descriptor { name, interfaces })
}

fn unique_provider(descriptors: &[Descriptor]) -> Result<Provider> {
    let names = |interface: &str| {
        descriptors
            .iter()
            .filter(|descriptor| descriptor.interfaces.contains(interface))
            .map(|descriptor| descriptor.name.as_str())
            .collect::<BTreeSet<_>>()
    };
    let remote = names(REMOTE_DESKTOP);
    let cast = names(SCREEN_CAST);
    ensure!(
        remote.len() == 1 && cast.len() == 1,
        "Portal requires one installed provider for both RemoteDesktop and ScreenCast; found {} input and {} capture providers",
        remote.len(),
        cast.len()
    );
    ensure!(
        remote == cast,
        "Portal capture and input use different implementations; their common session scope cannot be verified"
    );
    match remote.first().copied() {
        Some("org.freedesktop.impl.portal.desktop.gnome") => Ok(Provider::Gnome),
        Some("org.freedesktop.impl.portal.desktop.kde") => Ok(Provider::Kde),
        _ => bail!(
            "Portal provider has no verified compositor identity adapter; supported routes are GNOME and KDE"
        ),
    }
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
struct Snapshot(BTreeMap<PathBuf, Option<(Stamp, Stamp)>>);
impl Snapshot {
    fn observe(&mut self, path: &Path, started: SystemTime) -> Result<Option<Metadata>> {
        let link = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.0.insert(path.to_owned(), None);
                let mut ancestor = path.parent();
                while let Some(parent) = ancestor {
                    if parent.try_exists()? {
                        self.observe(parent, started)?;
                        break;
                    }
                    ancestor = parent.parent();
                }
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
        self.0
            .insert(path.to_owned(), Some((stamp(&link), stamp(&metadata))));
        ensure!(
            self.0.len() <= 1024,
            "Portal provider registry has too many paths"
        );
        // Include ancestor symlinks so retargeting a user data directory is not
        // mistaken for the registry the running frontend originally loaded.
        for ancestor in path.ancestors().skip(1).take(64) {
            if fs::symlink_metadata(ancestor)?.file_type().is_symlink()
                && !self.0.contains_key(ancestor)
            {
                self.observe(ancestor, started)?;
            }
        }
        Ok(Some(metadata))
    }
    fn verify(&self) -> Result<()> {
        for (path, expected) in &self.0 {
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
        Ok(())
    }
}

fn installed_provider(
    directories: &[PathBuf],
    started: SystemTime,
) -> Result<(Provider, Snapshot)> {
    ensure!(
        directories.len() <= MAX_DATA_DIRS + 1,
        "too many Portal descriptor directories"
    );
    let mut snapshot = Snapshot::default();
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
                parse_descriptor(read_bounded(&path, MAX_DESCRIPTOR_BYTES)?)
                    .with_context(|| format!("read Portal descriptor {}", path.display()))?,
            );
        }
    }
    let provider = unique_provider(&descriptors)?;
    snapshot.verify()?;
    Ok((provider, snapshot))
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

    fn descriptor(name: &str, interfaces: &[&str]) -> Descriptor {
        parse_descriptor(
            format!(
                "# fixture\n[portal]\nDBusName={name}\nInterfaces={};\nUseIn=irrelevant;\n",
                interfaces.join(";")
            )
            .into_bytes(),
        )
        .unwrap()
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
            assert_eq!(unique_provider(&descriptors).unwrap(), provider);
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
        assert!(
            unique_provider(&[gnome, kde])
                .unwrap_err()
                .to_string()
                .contains("2 input and 2 capture")
        );
        let split = [
            descriptor(Provider::Gnome.bus_name(), &[REMOTE_DESKTOP]),
            descriptor(Provider::Kde.bus_name(), &[SCREEN_CAST]),
        ];
        assert!(
            unique_provider(&split)
                .unwrap_err()
                .to_string()
                .contains("different implementations")
        );
        let custom = descriptor("com.example.CustomPortal", &[REMOTE_DESKTOP, SCREEN_CAST]);
        assert!(
            unique_provider(&[custom])
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
        assert!(
            unique_provider(&descriptors)
                .unwrap_err()
                .to_string()
                .contains("1 input and 2 capture")
        );
    }

    #[test]
    fn descriptors_keep_case_and_semicolon_lists_without_inline_comments() {
        let parsed = descriptor(Provider::Gnome.bus_name(), &[REMOTE_DESKTOP, SCREEN_CAST]);
        assert!(parsed.interfaces.contains(SCREEN_CAST));
        assert!(parsed.interfaces.contains(REMOTE_DESKTOP));
        assert!(parse_descriptor(b"[portal]\ndbusname=org.example.Test\nInterfaces=org.freedesktop.impl.portal.ScreenCast;\n".to_vec()).is_err());
        assert!(
            parse_descriptor(
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
        let directories = descriptor_directories(
            b"HOME=/home/test\0XDG_DATA_DIRS=/custom/share\0",
            Path::new("/usr/lib/xdg-desktop-portal"),
        )
        .unwrap();
        assert_eq!(
            directories,
            vec![
                PathBuf::from("/home/test/.local/share/xdg-desktop-portal/portals"),
                "/custom/share/xdg-desktop-portal/portals".into(),
                "/usr/share/xdg-desktop-portal/portals".into()
            ]
        );
        assert_eq!(
            descriptor_directories(
                b"XDG_DESKTOP_PORTAL_DIR=/override\0",
                Path::new("/custom/portal")
            )
            .unwrap(),
            vec![PathBuf::from("/override")]
        );
        assert!(descriptor_directories(b"HOME=/home/test\0", Path::new("/custom/portal")).is_err());
        assert!(
            descriptor_directories(
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

    #[test]
    fn inventory_admits_unchanged_descriptors_and_duplicate_same_provider() {
        let root = tempfile::tempdir().unwrap();
        let high = root.path().join("high");
        let low = root.path().join("low");
        fs::create_dir(&high).unwrap();
        fs::create_dir(&low).unwrap();
        write_provider(&high, Provider::Gnome.bus_name());
        write_provider(&low, Provider::Gnome.bus_name());
        let started = SystemTime::now() + Duration::from_secs(2);
        let (provider, snapshot) = installed_provider(&[high, low], started).unwrap();
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
        fs::write(
            system.join("gnome.portal"),
            format!(
                "[portal]\nDBusName={}\nInterfaces={REMOTE_DESKTOP};{SCREEN_CAST};\n",
                Provider::Gnome.bus_name()
            ),
        )
        .unwrap();
        let started = SystemTime::now() + Duration::from_secs(2);
        let error = installed_provider(&[user, system], started)
            .unwrap_err()
            .to_string();
        assert!(error.contains("2 input and 2 capture providers"));
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
            installed_provider(&[root.path().to_owned()], started)
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
        assert!(
            installed_provider(&[root.path().to_owned()], started)
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
        let (_, snapshot) = installed_provider(&[root.path().to_owned()], started).unwrap();
        fs::write(root.path().join("new.portal"), "new provider").unwrap();
        assert!(
            snapshot
                .verify()
                .unwrap_err()
                .to_string()
                .contains("changed during target validation")
        );
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
