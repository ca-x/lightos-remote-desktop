use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{TryRngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

const MAX_TOKENS: usize = 32;
const MAX_SETTINGS_BYTES: u64 = 32 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Observe,
    Control,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenInfo {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedToken {
    #[serde(flatten)]
    info: TokenInfo,
    hash: [u8; 32],
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    enabled: bool,
    tokens: Vec<SavedToken>,
}

struct ActiveToken {
    saved: SavedToken,
    cancellation: CancellationToken,
}

struct State {
    enabled: bool,
    tokens: Vec<ActiveToken>,
    error: bool,
}

#[derive(Clone)]
pub(super) struct Principal {
    pub id: String,
    pub scope: Scope,
    pub cancellation: CancellationToken,
}

#[derive(Serialize)]
pub(super) struct Status {
    pub enabled: bool,
    pub tokens: Vec<TokenInfo>,
}

#[derive(Serialize)]
pub(super) struct CreatedToken {
    pub token: String,
    pub info: TokenInfo,
}

pub struct TokenStore {
    path: Option<PathBuf>,
    state: Mutex<State>,
}

impl TokenStore {
    pub fn open(path: Option<PathBuf>) -> Self {
        let loaded = path
            .as_deref()
            .context("MCP state path unavailable")
            .and_then(read);
        let (settings, error) = match loaded {
            Ok(settings) => (settings, false),
            Err(error) => {
                tracing::warn!(%error, "MCP settings unavailable; MCP remains disabled");
                (Settings::default(), true)
            }
        };
        Self {
            path,
            state: Mutex::new(State {
                enabled: settings.enabled,
                tokens: settings
                    .tokens
                    .into_iter()
                    .map(|saved| ActiveToken {
                        saved,
                        cancellation: CancellationToken::new(),
                    })
                    .collect(),
                error,
            }),
        }
    }

    pub(super) fn status(&self) -> Result<Status> {
        let state = self.state.lock().unwrap();
        ensure!(
            !state.error,
            "MCP settings unavailable; check the private settings file"
        );
        Ok(Status {
            enabled: state.enabled,
            tokens: state
                .tokens
                .iter()
                .map(|token| token.saved.info.clone())
                .collect(),
        })
    }

    fn save(&self, state: &State, settings: &Settings) -> Result<()> {
        ensure!(
            !state.error,
            "MCP settings unavailable; check the private settings file"
        );
        write(
            self.path.as_deref().context("MCP state path unavailable")?,
            settings,
        )
        .context("MCP settings could not be saved")
    }

    pub(super) fn set_enabled(&self, enabled: bool) -> Result<Status> {
        let mut state = self.state.lock().unwrap();
        let settings = Settings {
            enabled,
            tokens: state
                .tokens
                .iter()
                .map(|token| token.saved.clone())
                .collect(),
        };
        self.save(&state, &settings)?;
        if state.enabled != enabled {
            for token in &mut state.tokens {
                token.cancellation.cancel();
                token.cancellation = CancellationToken::new();
            }
        }
        state.enabled = enabled;
        Ok(Status {
            enabled,
            tokens: state
                .tokens
                .iter()
                .map(|token| token.saved.info.clone())
                .collect(),
        })
    }

    pub(super) fn create(&self, name: &str, scope: Scope) -> Result<CreatedToken> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control),
            "Token name must be 1–80 characters without control characters"
        );
        let mut state = self.state.lock().unwrap();
        ensure!(
            state.tokens.len() < MAX_TOKENS,
            "Revoke a token before creating more (maximum 32)"
        );
        let mut bytes = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut bytes)
            .context("OS randomness unavailable")?;
        let token = format!("lrd_mcp_{}", URL_SAFE_NO_PAD.encode(bytes));
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let info = TokenInfo {
            id: URL_SAFE_NO_PAD.encode(&hash[..16]),
            name: name.to_owned(),
            scope,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };
        let saved = SavedToken {
            info: info.clone(),
            hash,
        };
        let mut tokens: Vec<_> = state
            .tokens
            .iter()
            .map(|token| token.saved.clone())
            .collect();
        tokens.push(saved.clone());
        self.save(
            &state,
            &Settings {
                enabled: state.enabled,
                tokens,
            },
        )?;
        state.tokens.push(ActiveToken {
            saved,
            cancellation: CancellationToken::new(),
        });
        Ok(CreatedToken { token, info })
    }

    pub(super) fn revoke(&self, id: &str) -> Result<bool> {
        let mut state = self.state.lock().unwrap();
        let Some(index) = state
            .tokens
            .iter()
            .position(|token| token.saved.info.id == id)
        else {
            return Ok(false);
        };
        self.save(
            &state,
            &Settings {
                enabled: state.enabled,
                tokens: state
                    .tokens
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != index)
                    .map(|(_, token)| token.saved.clone())
                    .collect(),
            },
        )?;
        state.tokens.remove(index).cancellation.cancel();
        Ok(true)
    }

    pub(super) fn authenticate(&self, bearer: &str) -> Option<Principal> {
        if bearer.len() != 51 || !bearer.starts_with("lrd_mcp_") {
            return None;
        }
        let hash: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        let state = self.state.lock().unwrap();
        if state.error || !state.enabled {
            return None;
        }
        state
            .tokens
            .iter()
            .find(|token| bool::from(token.saved.hash.ct_eq(&hash)))
            .map(|token| Principal {
                id: token.saved.info.id.clone(),
                scope: token.saved.info.scope,
                cancellation: token.cancellation.clone(),
            })
    }

    pub(super) fn enabled(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.enabled && !state.error
    }

    pub(super) fn authorized(&self, principal: &Principal, scope: Scope) -> bool {
        let state = self.state.lock().unwrap();
        state.enabled
            && !state.error
            && !principal.cancellation.is_cancelled()
            && (scope == Scope::Observe || principal.scope == Scope::Control)
            && state
                .tokens
                .iter()
                .any(|token| token.saved.info.id == principal.id)
    }
}

pub(super) fn default_path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".local/state"))
        })
        .map(|p| p.join("lightos-remote-desktop/mcp.json"))
}

fn private_file(file: &File) -> Result<()> {
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "MCP settings must be a private file owned by the current user"
    );
    ensure!(
        meta.len() <= MAX_SETTINGS_BYTES,
        "MCP settings file too large"
    );
    Ok(())
}

fn read(path: &Path) -> Result<Settings> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(error) => return Err(error.into()),
    };
    private_directory(
        path.parent()
            .context("MCP settings directory unavailable")?,
    )?;
    private_file(&file)?;
    let mut data = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut data)?;
    ensure!(
        data.len() <= MAX_SETTINGS_BYTES as usize,
        "MCP settings file too large"
    );
    let settings: Settings = serde_json::from_slice(&data).context("Invalid MCP settings file")?;
    ensure!(
        settings.tokens.len() <= MAX_TOKENS,
        "Too many saved MCP tokens"
    );
    for (index, token) in settings.tokens.iter().enumerate() {
        ensure!(
            token.info.id == URL_SAFE_NO_PAD.encode(&token.hash[..16])
                && !settings.tokens[..index]
                    .iter()
                    .any(|other| other.info.id == token.info.id)
                && !token.info.name.is_empty()
                && token.info.name.chars().count() <= 80
                && !token.info.name.chars().any(char::is_control),
            "Invalid saved MCP token metadata"
        );
    }
    Ok(settings)
}

fn private_directory(directory: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(directory)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "MCP settings directory must be private and owned by the current user"
    );
    Ok(())
}

fn write(path: &Path, settings: &Settings) -> Result<()> {
    let directory = path
        .parent()
        .context("MCP settings directory unavailable")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    private_directory(directory)?;
    if path.symlink_metadata().is_ok() {
        private_file(
            &OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)?,
        )?;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut temporary, settings)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_lifecycle_persists_only_hash_and_invalidates_old_principals() {
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = dir.path().join("mcp.json");
        let store = TokenStore::open(Some(path.clone()));
        let created = store.create(" agent ", Scope::Control).unwrap();
        assert!(!store.enabled());
        assert!(store.authenticate(&created.token).is_none());
        store.set_enabled(true).unwrap();
        let principal = store.authenticate(&created.token).unwrap();
        assert!(store.authorized(&principal, Scope::Control));
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains(&created.token));
        assert!(
            !serde_json::to_string(&store.status().unwrap())
                .unwrap()
                .contains("hash")
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let reloaded = TokenStore::open(Some(path.clone()));
        assert!(reloaded.authenticate(&created.token).is_some());
        store.set_enabled(false).unwrap();
        assert!(principal.cancellation.is_cancelled());
        store.set_enabled(true).unwrap();
        assert!(!store.authorized(&principal, Scope::Control));
        let current = store.authenticate(&created.token).unwrap();
        assert!(store.revoke(&created.info.id).unwrap());
        assert!(current.cancellation.is_cancelled());
        assert!(store.authenticate(&created.token).is_none());
        assert!(
            TokenStore::open(Some(path))
                .authenticate(&created.token)
                .is_none()
        );
    }

    #[test]
    fn invalid_state_and_failed_writes_fail_closed() {
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = dir.path().join("mcp.json");
        fs::write(&path, "{}").unwrap();
        let store = TokenStore::open(Some(path.clone()));
        assert!(!store.enabled());
        assert!(store.status().is_err());
        assert!(store.set_enabled(true).is_err());
        fs::remove_file(&path).unwrap();
        let store = TokenStore::open(Some(path.clone()));
        let token = store.create("read", Scope::Observe).unwrap();
        store.set_enabled(true).unwrap();
        let principal = store.authenticate(&token.token).unwrap();
        assert!(!store.authorized(&principal, Scope::Control));
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/dev/null", &path).unwrap();
        assert!(store.set_enabled(false).is_err());
        assert!(store.enabled());
        assert!(!principal.cancellation.is_cancelled());
        assert!(!TokenStore::open(Some(path)).enabled());
        assert!(store.create("\n", Scope::Control).is_err());
        assert!(store.create(&"x".repeat(81), Scope::Control).is_err());
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn token_limit_and_insecure_directory_are_enforced_on_reload() {
        let dir = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = dir.path().join("mcp.json");
        let store = TokenStore::open(Some(path.clone()));
        for index in 0..MAX_TOKENS {
            store
                .create(&format!("agent {index}"), Scope::Observe)
                .unwrap();
        }
        assert!(store.create("too many", Scope::Control).is_err());
        assert_eq!(store.status().unwrap().tokens.len(), MAX_TOKENS);
        store.set_enabled(true).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let reloaded = TokenStore::open(Some(path));
        assert!(!reloaded.enabled());
        assert!(reloaded.status().is_err());
    }
}
