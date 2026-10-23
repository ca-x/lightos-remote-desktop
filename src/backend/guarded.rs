//! Lock-state gating for attached graphical sessions. Never unlocks the desktop.
use super::{BackendKind, DesktopBackend, Frame, InputCommand, InputGuard};
use anyhow::{Context, Result, bail, ensure};
use display_control::SessionTarget;
use futures_util::future::BoxFuture;
use std::{
    fs::File,
    io::Read,
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use zbus::zvariant::OwnedObjectPath;

#[derive(Debug)]
struct SessionLock {
    connection: zbus::Connection,
    path: OwnedObjectPath,
}
impl SessionLock {
    async fn discover(target: &SessionTarget) -> Result<Option<Self>> {
        if !std::path::Path::new("/run/dbus/system_bus_socket").exists()
            && std::env::var_os("DBUS_SYSTEM_BUS_ADDRESS").is_none()
        {
            return Ok(None);
        }
        let connection = zbus::Connection::system().await?;
        let bus = zbus::Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await?;
        let running: bool = bus
            .call("NameHasOwner", &("org.freedesktop.login1",))
            .await?;
        if !running {
            let activatable: Vec<String> = bus.call("ListActivatableNames", &()).await?;
            if !activatable
                .iter()
                .any(|name| name == "org.freedesktop.login1")
            {
                return Ok(None);
            }
        }
        let manager = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await?;
        // The selected Wayland socket's peer PID is stronger evidence than a
        // user manager's inherited DISPLAY. Keep the connection open while
        // asking logind which login owns that compositor process.
        let peer = if let Some(socket) = target.wayland_socket.as_ref() {
            Some(selected_wayland_peer(socket).await?)
        } else {
            None
        };
        let peer_session: Option<OwnedObjectPath> = if let Some((pid, _)) = peer.as_ref() {
            manager.call("GetSessionByPID", &(*pid,)).await.ok()
        } else {
            None
        };
        let wayland_login = if peer.is_some() && peer_session.is_none() {
            let peer_login = peer.as_ref().and_then(|(pid, _)| {
                peer_wayland_environment(*pid)
                    .ok()
                    .and_then(|(display, runtime, id)| {
                        exact_wayland_login(target, display, runtime, id)
                            .ok()
                            .flatten()
                    })
            });
            Some(match peer_login {
                Some(id) => id,
                None => exact_wayland_login(
                    target,
                    std::env::var_os("WAYLAND_DISPLAY").map(std::path::PathBuf::from),
                    std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from),
                    std::env::var("XDG_SESSION_ID").ok(),
                )?
                .context("selected Wayland socket has no exact login session")?,
            })
        } else {
            None
        };
        let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> =
            manager.call("ListSessions", &()).await?;
        let uid = unsafe { libc::geteuid() };
        let mut matches = Vec::new();
        for (id, owner, _, _, path) in sessions {
            if owner != uid
                || peer_session
                    .as_ref()
                    .is_some_and(|expected| expected != &path)
                || wayland_login
                    .as_ref()
                    .is_some_and(|expected| expected != &id)
            {
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
            if remote {
                continue;
            }
            let matches_target = if target.wayland_socket.is_some() {
                kind == "wayland"
            } else if let Some(display) = &target.x11_display {
                let selected: String = session.get_property("Display").await?;
                kind == "x11" && display_number(&selected) == display_number(display)
            } else {
                false
            };
            if matches_target {
                matches.push(path);
            }
        }
        // A logind user's default display may be another session. Never use it
        // when there is more than one possible owner for this selected target.
        if matches.len() > 1 {
            bail!(
                "multiple graphical login sessions match the attached desktop; lock-state identity is ambiguous"
            );
        }
        ensure!(
            !matches.is_empty(),
            "no graphical login session matches the selected desktop"
        );
        Ok(matches.pop().map(|path| Self { connection, path }))
    }
    async fn locked(&self) -> Result<bool> {
        let session = zbus::Proxy::new(
            &self.connection,
            "org.freedesktop.login1",
            self.path.as_str(),
            "org.freedesktop.login1.Session",
        )
        .await?;
        Ok(session.get_property("LockedHint").await?)
    }
}
async fn selected_wayland_peer(socket: &std::path::Path) -> Result<(u32, tokio::net::UnixStream)> {
    let stream = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::UnixStream::connect(socket),
    )
    .await
    .context("selected Wayland peer connection timed out")??;
    let credential = stream.peer_cred()?;
    ensure!(
        credential.uid() == unsafe { libc::geteuid() },
        "selected Wayland peer belongs to another user"
    );
    let pid = u32::try_from(
        credential
            .pid()
            .context("selected Wayland peer has no process ID")?,
    )
    .context("selected Wayland peer has an invalid process ID")?;
    Ok((pid, stream))
}

fn peer_wayland_environment(
    pid: u32,
) -> Result<(
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
    Option<String>,
)> {
    let path = std::path::PathBuf::from(format!("/proc/{pid}/environ"));
    let file = File::open(&path)?;
    ensure!(
        file.metadata()?.uid() == unsafe { libc::geteuid() },
        "Wayland peer environment belongs to another user"
    );
    let mut bytes = Vec::new();
    file.take(128 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 128 * 1024,
        "Wayland peer environment is too large"
    );
    let value = |key: &[u8]| {
        bytes
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(key).map(|value| value.to_vec()))
    };
    let display = value(b"WAYLAND_DISPLAY=")
        .map(std::ffi::OsString::from_vec)
        .map(std::path::PathBuf::from);
    let runtime = value(b"XDG_RUNTIME_DIR=")
        .map(std::ffi::OsString::from_vec)
        .map(std::path::PathBuf::from);
    let id = value(b"XDG_SESSION_ID=").and_then(|value| String::from_utf8(value).ok());
    Ok((display, runtime, id))
}
fn exact_wayland_login(
    target: &SessionTarget,
    display: Option<std::path::PathBuf>,
    runtime: Option<std::path::PathBuf>,
    session_id: Option<String>,
) -> Result<Option<String>> {
    let Some(selected) = target.wayland_socket.as_ref() else {
        return Ok(None);
    };
    let socket = display.map(|path| {
        if path.is_absolute() {
            path
        } else {
            runtime.unwrap_or_default().join(path)
        }
    });
    ensure!(
        socket.as_ref() == Some(selected),
        "selected Wayland socket is not associated with this login session"
    );
    let id = session_id
        .filter(|id| !id.is_empty())
        .context("selected Wayland socket has no exact login session ID")?;
    Ok(Some(id))
}
fn display_number(display: &str) -> &str {
    display.split('.').next().unwrap_or(display)
}

trait LockMonitor: Send + Sync {
    fn locked(&self) -> BoxFuture<'_, Result<bool>>;
}

impl LockMonitor for SessionLock {
    fn locked(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(SessionLock::locked(self))
    }
}

enum LockSource {
    Discover(SessionTarget),
    #[cfg(test)]
    Known(Option<Arc<dyn LockMonitor>>),
}

pub struct GuardedBackend {
    inner: Arc<dyn DesktopBackend>,
    frames: watch::Receiver<Option<Arc<Frame>>>,
    input: mpsc::Sender<InputCommand>,
    blocked: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    worker: std::sync::Mutex<Option<thread::JoinHandle<()>>>,
}

/// Blocking setup. On systems without logind no lock authority is invented;
/// native desktop authorization remains authoritative. Once a matching login
/// session has been found, loss of its lock state blocks both capture and input.
pub fn attach(
    inner: Arc<dyn DesktopBackend>,
    target: SessionTarget,
) -> Result<Arc<dyn DesktopBackend>> {
    attach_with_source(inner, LockSource::Discover(target))
}

#[cfg(test)]
fn attach_with_monitor(
    inner: Arc<dyn DesktopBackend>,
    monitor: Option<Arc<dyn LockMonitor>>,
) -> Result<Arc<dyn DesktopBackend>> {
    attach_with_source(inner, LockSource::Known(monitor))
}

fn attach_with_source(
    inner: Arc<dyn DesktopBackend>,
    source: LockSource,
) -> Result<Arc<dyn DesktopBackend>> {
    let (frames_tx, frames) = watch::channel(None);
    let (input, mut commands) = mpsc::channel::<InputCommand>(128);
    let blocked = Arc::new(AtomicBool::new(true));
    let stopped = Arc::new(AtomicBool::new(false));
    let generation = Arc::new(AtomicU64::new(0));
    let worker_generation = generation.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let worker_inner = inner.clone();
    let worker_blocked = blocked.clone();
    let worker_stopped = stopped.clone();
    let worker = thread::Builder::new().name("desktop-lock-guard".into()).spawn(move || {
        let result = (|| -> Result<()> {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
            runtime.block_on(async {
                let authority: Option<Arc<dyn LockMonitor>> = match source {
                    LockSource::Discover(target) => tokio::time::timeout(Duration::from_secs(2), SessionLock::discover(&target))
                        .await.context("lock-state discovery timed out")??
                        .map(|session| Arc::new(session) as Arc<dyn LockMonitor>),
                    #[cfg(test)]
                    LockSource::Known(monitor) => monitor,
                };
                let initial = match &authority {
                    Some(authority) => tokio::time::timeout(Duration::from_secs(1), authority.locked()).await.context("lock-state query timed out")??,
                    None => false,
                };
                ensure!(!initial, "selected desktop is locked");
                worker_blocked.store(false, Ordering::Release);
                let mut source = worker_inner.frames();
                frames_tx.send_replace(source.borrow().clone());
                let _ = ready_tx.send(Ok(()));
                let mut ticks = tokio::time::interval(Duration::from_millis(250));
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut last_blocked = false;
                let mut input_gate = CancellationToken::new();
                let mut fresh_after = None;
                let monitored: Result<()> = async {
                loop {
                    if worker_stopped.load(Ordering::Acquire) { break; }
                    tokio::select! {
                        command = commands.recv() => {
                            let Some(command) = command else { break; };
                            if release_command(&command) {
                                tokio::time::timeout(Duration::from_millis(500), worker_inner.input().send(unwrapped_release(command))).await.context("input queue timed out")??;
                            } else if !worker_blocked.load(Ordering::Acquire) {
                                let command = match command {
                                    InputCommand::Guarded { command, mut guard } => {
                                        guard.lock_cancellation = Some(input_gate.clone());
                                        InputCommand::Guarded { command, guard }
                                    }
                                    command => InputCommand::Guarded {
                                        command: Box::new(command),
                                        guard: InputGuard::new(input_gate.clone(), Instant::now() + Duration::from_secs(3600)),
                                    },
                                };
                                tokio::time::timeout(Duration::from_millis(500), worker_inner.input().send(command)).await.context("input queue timed out")??;
                            } else if let InputCommand::Guarded { guard, .. } = command {
                                guard.text_done.notify_one();
                            }
                        },
                        changed = source.changed() => { if changed.is_err() { break; } },
                        _ = ticks.tick() => {
                            if let Some(authority) = &authority {
                                let locked = !matches!(tokio::time::timeout(Duration::from_secs(1), authority.locked()).await, Ok(Ok(false)));
                                if locked {
                                    input_gate.cancel();
                                } else if last_blocked {
                                    // Advance the epoch and require a new capture before
                                    // reporting the desktop as unlocked to readers.
                                    worker_generation.fetch_add(1, Ordering::AcqRel);
                                    input_gate = CancellationToken::new();
                                    fresh_after = Some(Instant::now());
                                }
                                worker_blocked.store(locked, Ordering::Release);
                            }
                        }
                    }
                    let locked = worker_blocked.load(Ordering::Acquire);
                    if locked && !last_blocked {
                        worker_generation.fetch_add(1, Ordering::AcqRel);
                    }
                    if locked && !last_blocked {
                        // Hide the last captured frame before waiting for input
                        // cleanup; refresh requests must not expose it meanwhile.
                        frames_tx.send_replace(None);
                        let backend = worker_inner.clone();
                        let released = tokio::task::spawn_blocking(move || backend.release_input()).await.unwrap_or(false);
                        if !released { worker_inner.stop(); bail!("could not release input on desktop lock"); }
                    }
                    last_blocked = locked;
                    let next = if locked {
                        None
                    } else {
                        source.borrow().clone().filter(|frame| {
                            fresh_after.is_none_or(|after| frame.captured_at > after)
                        })
                    };
                    frames_tx.send_if_modified(|current| {
                        let changed = match (&*current, &next) {
                            (Some(old), Some(new)) => !Arc::ptr_eq(&old.pixels, &new.pixels) || old.width != new.width || old.height != new.height,
                            (None, None) => false,
                            _ => true,
                        };
                        *current = next;
                        changed
                    });
                }
                Ok(())
                }.await;
                input_gate.cancel();
                monitored
            })
        })();
        if let Err(error) = result { let _ = ready_tx.send(Err(format!("{error:#}"))); }
        worker_stopped.store(true, Ordering::Release);
        worker_blocked.store(true, Ordering::Release);
        frames_tx.send_replace(None);
        worker_inner.stop();
    })?;
    match ready_rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(())) => Ok(Arc::new(GuardedBackend {
            inner,
            frames,
            input,
            blocked,
            stopped,
            generation,
            worker: std::sync::Mutex::new(Some(worker)),
        })),
        result => {
            stopped.store(true, Ordering::Release);
            inner.stop();
            let _ = worker.join();
            bail!(
                "attached desktop lock guard: {}",
                match result {
                    Ok(Err(message)) => message,
                    _ => "initialization timed out".into(),
                }
            )
        }
    }
}
fn release_command(command: &InputCommand) -> bool {
    match command {
        InputCommand::Release | InputCommand::ReleaseSync { .. } => true,
        InputCommand::Guarded { command, .. } => release_command(command),
        _ => false,
    }
}
fn unwrapped_release(command: InputCommand) -> InputCommand {
    match command {
        InputCommand::Guarded { command, .. } => unwrapped_release(*command),
        command => command,
    }
}
impl DesktopBackend for GuardedBackend {
    fn kind(&self) -> BackendKind {
        self.inner.kind()
    }
    fn cursor_embedded(&self) -> bool {
        self.inner.cursor_embedded()
    }
    fn begin_cursor(&self, owner: u64) {
        self.inner.begin_cursor(owner);
    }
    fn release_cursor(&self, owner: u64) {
        self.inner.release_cursor(owner);
    }
    fn separate_cursor(&self, owner: u64) {
        if !self.paused() && !self.is_stopped() {
            self.inner.separate_cursor(owner);
        }
    }
    fn cursor_target(&self) -> Result<(std::path::PathBuf, i32, Option<String>)> {
        ensure!(
            !self.paused() && !self.is_stopped(),
            "desktop locked or unavailable"
        );
        self.inner.cursor_target()
    }
    fn supports_image_clipboard(&self) -> bool {
        self.inner.supports_image_clipboard()
    }
    fn clipboard_write_png<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let generation = self.generation.load(Ordering::Acquire);
            ensure!(!self.paused(), "desktop locked or unavailable");
            tokio::select! {
                biased;
                _ = self.wait_for_lock(generation) => bail!("desktop locked during clipboard write"),
                result = self.inner.clipboard_write_png(data) => result?,
            }
            ensure!(
                self.generation.load(Ordering::Acquire) == generation && !self.paused(),
                "desktop locked during clipboard write"
            );
            Ok(())
        })
    }
    fn paused(&self) -> bool {
        self.blocked.load(Ordering::Acquire) || self.inner.paused()
    }
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.inner.is_stopped()
    }
    fn privacy_epoch(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.blocked.store(true, Ordering::Release);
        self.inner.stop();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async {
            let generation = self.generation.load(Ordering::Acquire);
            ensure!(!self.paused(), "desktop locked or unavailable");
            let text = tokio::select! {
                result = self.inner.clipboard_read() => result?,
                _ = self.wait_for_lock(generation) => bail!("desktop locked during clipboard read"),
            };
            ensure!(
                self.generation.load(Ordering::Acquire) == generation && !self.paused(),
                "desktop locked during clipboard read"
            );
            Ok(text)
        })
    }
    fn clipboard_write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let generation = self.generation.load(Ordering::Acquire);
            ensure!(!self.paused(), "desktop locked or unavailable");
            tokio::select! {
                result = self.inner.clipboard_write(text) => result?,
                _ = self.wait_for_lock(generation) => bail!("desktop locked during clipboard write"),
            }
            ensure!(
                self.generation.load(Ordering::Acquire) == generation && !self.paused(),
                "desktop locked during clipboard write"
            );
            Ok(())
        })
    }
}
impl GuardedBackend {
    async fn wait_for_lock(&self, generation: u64) {
        let mut updates = self.frames.clone();
        loop {
            if self.paused() || self.generation.load(Ordering::Acquire) != generation {
                return;
            }
            if updates.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for GuardedBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "guarded/tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_cleanup_commands_cross_a_lock_boundary() {
        assert!(release_command(&InputCommand::Release));
        let (done, _) = std::sync::mpsc::sync_channel(1);
        assert!(release_command(&InputCommand::ReleaseSync { done }));
        assert!(!release_command(&InputCommand::Key {
            code: 30,
            pressed: true
        }));
        assert!(!release_command(&InputCommand::Text {
            text: "secret".into()
        }));
    }
    #[test]
    fn wayland_lock_authority_requires_exact_inherited_session_identity() {
        let target =
            SessionTarget::wayland("/run/user/1000/wayland-2".into(), "/run/user/1000".into());
        let runtime = Some(std::path::PathBuf::from("/run/user/1000"));
        assert_eq!(
            exact_wayland_login(
                &target,
                Some("wayland-2".into()),
                runtime.clone(),
                Some("42".into())
            )
            .unwrap(),
            Some("42".into())
        );
        assert!(exact_wayland_login(&target, None, runtime.clone(), Some("42".into())).is_err());
        assert!(
            exact_wayland_login(
                &target,
                Some("wayland-1".into()),
                runtime.clone(),
                Some("42".into())
            )
            .is_err()
        );
        assert!(exact_wayland_login(&target, Some("wayland-2".into()), runtime, None).is_err());
    }
    #[tokio::test]
    async fn wayland_socket_peer_identifies_the_server_process() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-peer");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let accepted = std::thread::spawn(move || listener.accept().unwrap());
        let (pid, stream) = selected_wayland_peer(&socket).await.unwrap();
        assert_eq!(pid, std::process::id());
        drop(stream);
        accepted.join().unwrap();
    }
    #[test]
    fn x11_screens_share_the_same_login_display() {
        assert_eq!(display_number(":1.0"), display_number(":1.2"));
        assert_ne!(display_number(":1"), display_number(":2"));
    }
}
