//! Versioned descriptor shared by the desktop owner and reconnecting transports.
use crate::session::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const NAME: &str = "session.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    version: u32,
    owner_pid: u32,
    owner_start: String,
    socket: PathBuf,
    signature: Option<String>,
    pulse_server: Option<String>,
    pulse_client_config: Option<PathBuf>,
    audio_source: Option<String>,
}

fn process_start(pid: u32) -> Result<String> {
    let proc = PathBuf::from(format!("/proc/{pid}"));
    ensure!(
        fs::metadata(&proc)?.uid() == unsafe { libc::geteuid() },
        "session owner belongs to another user"
    );
    let stat = fs::read_to_string(proc.join("stat"))?;
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .map(str::to_owned)
        .context("invalid session owner identity")
}

impl Descriptor {
    pub fn publish(session: &Session) -> Result<()> {
        let owner_pid = std::process::id();
        let descriptor = Self {
            version: VERSION,
            owner_pid,
            owner_start: process_start(owner_pid)?,
            socket: session.socket.clone(),
            signature: session.signature.clone(),
            pulse_server: session.pulse_server.clone(),
            pulse_client_config: session.pulse_client_config.clone(),
            audio_source: session.audio_source.clone(),
        };
        let mut file = tempfile::NamedTempFile::new_in(&session.runtime)?;
        serde_json::to_writer(file.as_file_mut(), &descriptor)?;
        file.as_file().sync_all()?;
        file.persist(session.runtime.join(NAME))?;
        Ok(())
    }

    pub fn read(runtime: &Path) -> Result<Self> {
        crate::session::secure_directory(runtime)?;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(runtime.join(NAME))?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.len() <= 16384,
            "session descriptor must be an owned private regular file smaller than 16 KiB"
        );
        let mut bytes = Vec::new();
        file.by_ref().take(16385).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 16384, "session descriptor too large");
        let descriptor: Self = serde_json::from_slice(&bytes)?;
        ensure!(
            descriptor.version == VERSION,
            "unsupported desktop session protocol; preserve the owner and use a compatible transport"
        );
        ensure!(
            process_start(descriptor.owner_pid)? == descriptor.owner_start,
            "stale desktop session owner"
        );
        ensure!(
            descriptor.socket.parent() == Some(runtime)
                && crate::session::is_owned_socket(&descriptor.socket, unsafe { libc::geteuid() }),
            "invalid session Wayland socket"
        );
        if let Some(signature) = &descriptor.signature {
            ensure!(
                !signature.is_empty()
                    && signature
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                "invalid compositor signature"
            );
            ensure!(
                runtime
                    .join("hypr")
                    .join(signature)
                    .join(".socket.sock")
                    .exists(),
                "session compositor IPC unavailable"
            );
        }
        if let Some(server) = &descriptor.pulse_server {
            ensure!(
                server == &format!("unix:{}/pulse/native", runtime.display()),
                "unexpected session audio server"
            );
        }
        if let Some(path) = &descriptor.pulse_client_config {
            ensure!(path.starts_with(runtime), "unexpected session Pulse config");
        }
        // The owner keeps its exclusive flock for its entire lifetime.
        let lock = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(runtime.join("session.lock"))?;
        let result = unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&lock),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        ensure!(
            result == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK),
            "desktop session owner no longer holds its lock"
        );
        Ok(descriptor)
    }

    pub fn attach(self, runtime: &Path) -> Result<Session> {
        // Socket liveness is checked as well as inode ownership; stale socket files are insufficient.
        std::os::unix::net::UnixStream::connect(&self.socket)
            .context("persistent desktop unavailable")?;
        Ok(Session::attached(
            runtime.to_path_buf(),
            self.socket,
            self.signature,
            self.pulse_server,
            self.pulse_client_config,
            self.audio_source,
        ))
    }

    pub fn remove(runtime: &Path) {
        let _ = fs::remove_file(runtime.join(NAME));
    }
}

// A transport upgrade must keep attaching to an older live owner until that
// owner is explicitly restarted. The /run fallback disappears on reboot.
pub(crate) fn attach_persistent(runtime: &Path) -> Result<Option<Session>> {
    let previous = (runtime == Path::new(crate::session::DEFAULT_SESSION_RUNTIME))
        .then(|| Path::new(crate::session::PREVIOUS_SESSION_RUNTIME));
    attach_with_previous(runtime, previous)
}

fn attach_with_previous(runtime: &Path, previous: Option<&Path>) -> Result<Option<Session>> {
    if runtime.join(NAME).exists() {
        return Descriptor::read(runtime)?.attach(runtime).map(Some);
    }
    if let Some(previous) = previous
        && previous.join(NAME).exists()
    {
        match Descriptor::read(previous).and_then(|descriptor| descriptor.attach(previous)) {
            Ok(session) => {
                tracing::info!(runtime = %previous.display(), "preserving the running desktop at its previous runtime path");
                return Ok(Some(session));
            }
            Err(error) => tracing::debug!(%error, "ignoring stale previous desktop descriptor"),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::{
            fs::{PermissionsExt, symlink},
            net::UnixListener,
        },
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    #[test]
    fn reconnect_preserves_compositor_socket_and_rejects_unowned_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let socket = dir.path().join("wayland-test");
        let listener = UnixListener::bind(&socket).unwrap();
        let owner_lock = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(dir.path().join("session.lock"))
            .unwrap();
        assert_eq!(
            unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&owner_lock),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            },
            0
        );
        let owner = Session::attached(
            dir.path().to_owned(),
            socket.clone(),
            None,
            None,
            None,
            None,
        );
        Descriptor::publish(&owner).unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let accepting = running.clone();
        listener.set_nonblocking(true).unwrap();
        let acceptor = std::thread::spawn(move || {
            while accepting.load(Ordering::Acquire) {
                let _ = listener.accept();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        });
        for _ in 0..3 {
            let transport = Descriptor::read(dir.path())
                .unwrap()
                .attach(dir.path())
                .unwrap();
            drop(transport);
            assert!(socket.exists());
            assert!(dir.path().join(NAME).exists());
        }
        // A missing new runtime must preserve a live pre-upgrade owner without
        // creating a second desktop or even a new runtime directory.
        let next = dir.path().join("next");
        let transport = attach_with_previous(&next, Some(dir.path()))
            .unwrap()
            .unwrap();
        assert_eq!(transport.runtime, dir.path());
        assert!(!next.exists());
        drop(transport);
        assert!(attach_with_previous(&next, None).unwrap().is_none());
        // Prefer a published primary runtime over any fallback.
        assert!(
            attach_with_previous(dir.path(), Some(&next))
                .unwrap()
                .is_some()
        );
        let source = fs::read_to_string(dir.path().join(NAME)).unwrap();
        let mut data: serde_json::Value = serde_json::from_str(&source).unwrap();
        data["version"] = 2.into();
        fs::write(dir.path().join(NAME), data.to_string()).unwrap();
        assert!(Descriptor::read(dir.path()).is_err());
        data["version"] = 1.into();
        data["owner_start"] = "stale".into();
        fs::write(dir.path().join(NAME), data.to_string()).unwrap();
        assert!(Descriptor::read(dir.path()).is_err());
        assert!(
            attach_with_previous(&next, Some(dir.path()))
                .unwrap()
                .is_none()
        );
        assert!(attach_with_previous(dir.path(), Some(&next)).is_err());
        fs::write(dir.path().join(NAME), &source).unwrap();
        fs::set_permissions(dir.path().join(NAME), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Descriptor::read(dir.path()).is_err());
        fs::remove_file(dir.path().join(NAME)).unwrap();
        let other = dir.path().join("other");
        fs::write(&other, &source).unwrap();
        symlink(&other, dir.path().join(NAME)).unwrap();
        assert!(Descriptor::read(dir.path()).is_err());
        running.store(false, Ordering::Release);
        acceptor.join().unwrap();
        drop(owner_lock);
    }
}
