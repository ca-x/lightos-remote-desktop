//! OS-approved Portal grants shared with the read-only permission command.
//! Tokens remain private to this local user; the Portal validates every restore.
use super::identity::ScopeProof;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    active: Option<Active>,
}
#[derive(Serialize, Deserialize)]
struct Active {
    id: String,
    pid: u32,
    started: String,
    scope: String,
    approved: bool,
}
fn started(pid: u32) -> Result<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // comm may contain spaces and parentheses. starttime is field 22.
    Ok(stat
        .rsplit_once(')')
        .context("invalid process identity")?
        .1
        .split_whitespace()
        .nth(19)
        .context("missing process start time")?
        .to_owned())
}
impl Active {
    fn alive(&self) -> bool {
        started(self.pid).is_ok_and(|value| value == self.started)
    }
}
fn path(identity: &ScopeProof) -> Result<PathBuf> {
    let root =
        crate::private_storage::state_directory().context("private state directory unavailable")?;
    let key = format!("{:x}", Sha256::digest(identity.restore_key.as_bytes()));
    Ok(root
        .join("portal-authorizations")
        .join(format!("{key}.json")))
}
fn read(path: &Path) -> Result<Saved> {
    let Some(bytes) = crate::private_storage::current().read(path, 16 * 1024)? else {
        return Ok(Saved::default());
    };
    let saved: Saved = serde_json::from_slice(&bytes)?;
    ensure!(
        saved
            .token
            .as_ref()
            .is_none_or(|token| !token.is_empty() && token.len() <= 4096),
        "invalid Portal restore token"
    );
    Ok(saved)
}
fn lock(path: &Path) -> Result<File> {
    let directory = path.parent().context("missing authorization directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "authorization directory must be private and owned by this user"
    );
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.with_extension("lock"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "invalid authorization lock"
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        ensure!(
            error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline,
            "authorization state is busy: {error}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn save(path: &Path, saved: &Saved) -> Result<()> {
    crate::private_storage::current().write(path, &serde_json::to_vec(saved)?)
}

pub(super) fn approved(identity: &ScopeProof) -> Result<bool> {
    let saved = read(&path(identity)?)?;
    Ok(saved.token.is_some()
        || saved.active.is_some_and(|active| {
            active.approved && active.scope == identity.active_key && active.alive()
        }))
}

pub(super) struct Grant {
    path: PathBuf,
    id: String,
    token: Option<String>,
}
impl Grant {
    pub(super) fn begin(identity: &ScopeProof) -> Result<Self> {
        let path = path(identity)?;
        let _lock = lock(&path)?;
        let mut saved = read(&path)?;
        let token = saved.token.take();
        let id = format!("{:032x}", rand::random::<u128>());
        let pid = std::process::id();
        saved.active = Some(Active {
            id: id.clone(),
            pid,
            started: started(pid)?,
            scope: identity.active_key.clone(),
            approved: token.is_some(),
        });
        // Consume the single-use token before submitting it to the Portal.
        // A crash or rejection cannot cause it to be replayed later.
        save(&path, &saved)?;
        Ok(Self { path, id, token })
    }
    pub(super) fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
    pub(super) fn confirmed(&self, token: Option<String>) -> Result<()> {
        let _lock = lock(&self.path)?;
        let mut saved = read(&self.path)?;
        if let Some(active) = &mut saved.active
            && active.id == self.id
        {
            active.approved = true;
            saved.token = token;
            save(&self.path, &saved)?;
        }
        Ok(())
    }
}
impl Drop for Grant {
    fn drop(&mut self) {
        let result = (|| -> Result<()> {
            let _lock = lock(&self.path)?;
            let mut saved = read(&self.path)?;
            if saved
                .active
                .as_ref()
                .is_some_and(|active| active.id == self.id)
            {
                saved.active = None;
                save(&self.path, &saved)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "could not finish Portal authorization status");
        }
    }
}
