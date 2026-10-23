//! Authenticated, persistent settings for the independently replaceable RDP transport.
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[cfg(any(feature = "rdp", test))]
use std::{
    fs::File,
    io::Write,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{config::Config, server::AppState};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub port: u16,
}

#[derive(Serialize)]
pub struct Status {
    supported: bool,
    available: bool,
    unavailable_reason: Option<&'static str>,
    enabled: bool,
    port: u16,
    bind_address: String,
    endpoint: String,
    loopback: bool,
    fingerprint: Option<String>,
    error: bool,
}

struct Runtime {
    settings: Settings,
    invalid_store: bool,
    startup_error: bool,
    stopping: bool,
    #[cfg(feature = "rdp")]
    service: Option<super::RdpService>,
    fingerprint: Option<String>,
}

pub struct Manager {
    #[cfg(feature = "rdp")]
    path: Option<PathBuf>,
    config: super::RdpConfig,
    attached: bool,
    runtime: Mutex<Runtime>,
}

impl Manager {
    pub fn new(config: &Config) -> Self {
        let path = config.rdp.rdp_settings.clone().or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .filter(|p| p.is_absolute())
                        .map(|p| p.join(".local/state"))
                })
                .map(|p| p.join("lightos-remote-desktop/rdp/settings.json"))
        });
        let initial = Settings {
            enabled: config.rdp.rdp,
            port: config.rdp.rdp_listen.port(),
        };
        let loaded = path
            .as_deref()
            .context("RDP state path unavailable")
            .and_then(|path| {
                read_private(path)?
                    .map(|bytes| -> Result<Settings> {
                        let settings: Settings = serde_json::from_slice(&bytes)?;
                        ensure!(settings.port != 0, "invalid saved RDP port");
                        Ok(settings)
                    })
                    .transpose()
            });
        let (settings, invalid_store) = match loaded {
            Ok(saved) => (saved.unwrap_or(initial), false),
            Err(error) => {
                tracing::warn!(%error, "RDP settings unavailable; listener remains disabled");
                (
                    Settings {
                        enabled: false,
                        ..initial
                    },
                    true,
                )
            }
        };
        Self {
            #[cfg(feature = "rdp")]
            path,
            config: config.rdp.clone(),
            attached: config.wayland_socket.is_some(),
            runtime: Mutex::new(Runtime {
                settings,
                invalid_store,
                startup_error: false,
                stopping: false,
                #[cfg(feature = "rdp")]
                service: None,
                fingerprint: None,
            }),
        }
    }

    /// A bad optional listener must not take down the browser or desktop owner.
    pub async fn initialize(&self, state: AppState) {
        let mut runtime = self.runtime.lock().await;
        if runtime.invalid_store || !runtime.settings.enabled {
            return;
        }
        #[cfg(feature = "rdp")]
        match self.candidate(state, runtime.settings).await {
            Ok(service) => {
                service.activate();
                runtime.fingerprint = Some(service.fingerprint.clone());
                runtime.service = Some(service);
            }
            Err(error) => {
                tracing::warn!(%error, "RDP startup failed; browser remains available");
                runtime.startup_error = true;
            }
        }
        #[cfg(not(feature = "rdp"))]
        {
            let _ = state;
            runtime.startup_error = true;
        }
    }

    fn status_locked(&self, runtime: &Runtime) -> Status {
        #[cfg(feature = "rdp")]
        let enabled = runtime
            .service
            .as_ref()
            .is_some_and(|service| !service.failed());
        #[cfg(not(feature = "rdp"))]
        let enabled = false;
        let mut endpoint = self.config.rdp_listen;
        endpoint.set_port(runtime.settings.port);
        Status {
            supported: cfg!(feature = "rdp"),
            available: !self.attached && !runtime.invalid_store && !runtime.stopping,
            unavailable_reason: if self.attached {
                Some("attached")
            } else if runtime.invalid_store {
                Some("private_state")
            } else if runtime.stopping {
                Some("stopping")
            } else {
                None
            },
            enabled,
            port: runtime.settings.port,
            bind_address: endpoint.ip().to_string(),
            endpoint: endpoint.to_string(),
            loopback: endpoint.ip().is_loopback(),
            fingerprint: runtime.fingerprint.clone(),
            error: runtime.invalid_store
                || runtime.startup_error
                || (runtime.settings.enabled && !enabled),
        }
    }

    pub async fn status(&self) -> Status {
        self.status_locked(&*self.runtime.lock().await)
    }

    #[cfg(feature = "rdp")]
    async fn candidate(&self, state: AppState, settings: Settings) -> Result<super::RdpService> {
        ensure!(!self.attached, "RDP requires an owned virtual desktop");
        let mut config = self.config.clone();
        config.rdp = true;
        config.rdp_listen.set_port(settings.port);
        config.validate(self.attached)?;
        if config.rdp_tls_cert.is_none() {
            let identity = self
                .path
                .as_deref()
                .context("RDP state path unavailable")?
                .with_extension("identity.pem");
            if read_private(&identity)?.is_none() {
                let generated = rcgen::generate_simple_self_signed(vec![
                    "localhost".into(),
                    "lightos-remoted".into(),
                    config.rdp_listen.ip().to_string(),
                ])?;
                let pem = format!(
                    "{}{}",
                    generated.cert.pem(),
                    generated.signing_key.serialize_pem()
                );
                write_private(&identity, pem.as_bytes())?;
            }
            config.rdp_tls_cert = Some(identity.clone());
            config.rdp_tls_key = Some(identity);
        }
        super::service::prepare(state, &config).await
    }

    pub async fn apply(&self, state: AppState, settings: Settings) -> Result<Status> {
        ensure!(cfg!(feature = "rdp"), "RDP is not compiled in");
        ensure!(settings.port != 0, "RDP port must be 1–65535");
        let mut runtime = self.runtime.lock().await;
        ensure!(
            !runtime.invalid_store && !runtime.stopping,
            "RDP settings unavailable"
        );
        ensure!(
            !settings.enabled || !self.attached,
            "RDP requires an owned virtual desktop"
        );
        #[cfg(feature = "rdp")]
        {
            let unchanged = runtime.settings.port == settings.port
                && settings.enabled
                && runtime.service.as_ref().is_some_and(|s| !s.failed());
            let candidate = if settings.enabled && !unchanged {
                Some(self.candidate(state, settings).await?)
            } else {
                None
            };
            if let Err(error) = write_private(
                self.path.as_deref().context("RDP state path unavailable")?,
                &serde_json::to_vec(&settings)?,
            ) {
                if let Some(candidate) = candidate {
                    let _ = candidate.stop().await;
                }
                return Err(error);
            }
            if !unchanged {
                let previous = std::mem::replace(&mut runtime.service, candidate);
                if let Some(previous) = previous {
                    // stop() waits for connection deactivation and the held-input barrier.
                    if let Err(error) = previous.stop().await {
                        tracing::warn!(%error, "Previous RDP listener stopped with an error");
                    }
                }
            }
            if let Some(service) = &runtime.service {
                service.activate();
                runtime.fingerprint = Some(service.fingerprint.clone());
            }
        }
        #[cfg(not(feature = "rdp"))]
        {
            let _ = state;
        }
        runtime.settings = settings;
        runtime.startup_error = false;
        Ok(self.status_locked(&runtime))
    }

    pub async fn shutdown(&self) {
        let mut runtime = self.runtime.lock().await;
        runtime.stopping = true;
        #[cfg(feature = "rdp")]
        if let Some(service) = runtime.service.take() {
            let _ = service.stop().await;
        }
    }
}

fn private_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "RDP state directory must be private and owned by the service user"
    );
    Ok(())
}

fn read_private(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    private_directory(path.parent().context("missing RDP state directory")?)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= 65536,
        "RDP state must be a private regular file"
    );
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "RDP state too large");
    Ok(Some(bytes))
}

#[cfg(any(feature = "rdp", test))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("missing RDP state directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    private_directory(directory)?;
    read_private(path)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    // Rename is the commit point: a directory fsync failure must not leave the
    // live listener using the old settings after the new file was committed.
    if let Err(error) = File::open(directory).and_then(|file| file.sync_all()) {
        tracing::warn!(%error, "RDP settings committed but directory sync failed");
    }
    Ok(())
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/rdp", get(status).put(update))
        .layer(DefaultBodyLimit::max(1024))
        .layer(middleware::from_fn(origin))
}

async fn origin(request: Request<Body>, next: Next) -> Response {
    let headers = request.headers();
    let same = if headers.contains_key(header::ORIGIN) {
        headers.get_all(header::ORIGIN).iter().count() == 1
            && headers.get_all(header::HOST).iter().count() == 1
            && crate::server::origin_allowed(headers)
    } else {
        request.method() == axum::http::Method::GET
            && headers
                .get("sec-fetch-site")
                .is_some_and(|v| v == "same-origin")
    };
    if !same
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v == "cross-site")
        || headers.get("x-lightos-rdp").is_none_or(|v| v != "1")
    {
        return (StatusCode::FORBIDDEN, "Same-origin RDP management required").into_response();
    }
    next.run(request).await
}

async fn status(State(state): State<AppState>) -> Json<Status> {
    Json(state.rdp.status().await)
}

async fn update(State(state): State<AppState>, Json(settings): Json<Settings>) -> Response {
    if !cfg!(feature = "rdp") {
        return StatusCode::NOT_IMPLEMENTED.into_response();
    }
    if settings.port == 0 {
        return (StatusCode::BAD_REQUEST, "Port must be 1–65535").into_response();
    }
    if settings.enabled && state.config.wayland_socket.is_some() {
        return StatusCode::CONFLICT.into_response();
    }
    // Finish the transaction even if a browser closes the settings tab mid-request.
    match tokio::spawn(async move { state.rdp.clone().apply(state, settings).await }).await {
        Ok(Ok(status)) => Json(status).into_response(),
        result => {
            if let Ok(Err(error)) = result {
                tracing::warn!(%error, "RDP settings update failed");
            }
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":"RDP settings were not applied. Check the port, TLS files and private state permissions."}))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn config(directory: &Path) -> Config {
        let mut config =
            Config::try_parse_from(["test", "--password", "rdp-settings-test"]).unwrap();
        config.rdp.rdp_settings = Some(directory.join("rdp/settings.json"));
        config.mcp_settings = Some(directory.join("mcp.json"));
        config
    }

    #[tokio::test]
    async fn default_is_disabled_and_corrupt_or_public_state_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let manager = Manager::new(&config);
        assert!(!manager.status().await.enabled);
        assert_eq!(manager.status().await.supported, cfg!(feature = "rdp"));
        let path = config.rdp.rdp_settings.as_ref().unwrap();
        write_private(path, b"not json").unwrap();
        assert!(!Manager::new(&config).status().await.available);
        write_private(path, br#"{"enabled":true,"port":3389}"#).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!Manager::new(&config).status().await.available);
        assert!(write_private(path, b"replacement").is_err());
        assert_eq!(fs::read(path).unwrap(), br#"{"enabled":true,"port":3389}"#);
        fs::remove_file(path).unwrap();
        std::os::unix::fs::symlink(directory.path().join("missing"), path).unwrap();
        assert!(write_private(path, b"replacement").is_err());
    }

    #[cfg(feature = "rdp")]
    mod runtime {
        use super::*;
        use crate::{session::Session, wayland::tests::rdp_backend};
        use std::net::TcpListener;

        fn desktop(config: Config) -> AppState {
            let (backend, _frames, _input) = rdp_backend();
            AppState::new(
                config,
                backend,
                Session::attached(
                    "/tmp/rdp-settings-unit".into(),
                    "/tmp/rdp-settings-unit/wayland".into(),
                    None,
                    None,
                    None,
                    None,
                ),
            )
        }
        fn free_port() -> u16 {
            TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port()
        }

        #[tokio::test]
        async fn changes_persist_identity_and_port_without_replacing_desktop_or_http_state() {
            let dir = tempfile::tempdir().unwrap();
            let config = config(dir.path());
            let state = desktop(config.clone());
            let first = free_port();
            let enabled = state
                .rdp
                .apply(
                    state.clone(),
                    Settings {
                        enabled: true,
                        port: first,
                    },
                )
                .await
                .unwrap();
            assert!(enabled.enabled);
            assert!(enabled.loopback);
            let fingerprint = enabled.fingerprint.unwrap();
            assert_eq!(fingerprint.len(), 95);
            let path = config.rdp.rdp_settings.as_ref().unwrap();
            let identity = path.with_extension("identity.pem");
            let original_identity = fs::read(&identity).unwrap();
            assert_eq!(fs::metadata(&identity).unwrap().mode() & 0o777, 0o600);
            assert_eq!(
                fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
                0o700
            );
            assert!(
                !serde_json::to_string(&state.rdp.status().await)
                    .unwrap()
                    .contains("PRIVATE KEY")
            );
            let second = free_port();
            state
                .rdp
                .apply(
                    state.clone(),
                    Settings {
                        enabled: true,
                        port: second,
                    },
                )
                .await
                .unwrap();
            assert!(TcpListener::bind(("127.0.0.1", first)).is_ok());
            assert!(TcpListener::bind(("127.0.0.1", second)).is_err());
            state.rdp.shutdown().await;
            let restarted = desktop(config.clone());
            restarted.rdp.initialize(restarted.clone()).await;
            let status = restarted.rdp.status().await;
            assert!(status.enabled);
            assert_eq!(status.port, second);
            assert_eq!(status.fingerprint.as_deref(), Some(fingerprint.as_str()));
            assert_eq!(fs::read(&identity).unwrap(), original_identity);
            restarted
                .rdp
                .apply(
                    restarted.clone(),
                    Settings {
                        enabled: false,
                        port: second,
                    },
                )
                .await
                .unwrap();
            assert!(TcpListener::bind(("127.0.0.1", second)).is_ok());
            let mut cli_enabled = config;
            cli_enabled.rdp.rdp = true;
            assert!(
                !Manager::new(&cli_enabled)
                    .runtime
                    .lock()
                    .await
                    .settings
                    .enabled,
                "saved off must survive even a legacy --rdp startup flag"
            );
            restarted.rdp.shutdown().await;
        }

        #[tokio::test]
        async fn occupied_port_bad_tls_and_failed_save_preserve_previous_listener_and_settings() {
            let dir = tempfile::tempdir().unwrap();
            let config = config(dir.path());
            let state = desktop(config.clone());
            let first = free_port();
            state
                .rdp
                .apply(
                    state.clone(),
                    Settings {
                        enabled: true,
                        port: first,
                    },
                )
                .await
                .unwrap();
            let path = config.rdp.rdp_settings.as_ref().unwrap();
            let saved = fs::read(path).unwrap();
            let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
            assert!(
                state
                    .rdp
                    .apply(
                        state.clone(),
                        Settings {
                            enabled: true,
                            port: occupied.local_addr().unwrap().port()
                        }
                    )
                    .await
                    .is_err()
            );
            assert_eq!(fs::read(path).unwrap(), saved);
            let identity = path.with_extension("identity.pem");
            let original = fs::read(&identity).unwrap();
            fs::write(&identity, "corrupt TLS").unwrap();
            assert!(
                state
                    .rdp
                    .apply(
                        state.clone(),
                        Settings {
                            enabled: true,
                            port: free_port()
                        }
                    )
                    .await
                    .is_err()
            );
            assert_eq!(
                fs::read(&identity).unwrap(),
                b"corrupt TLS",
                "never silently rotate a broken identity"
            );
            fs::write(&identity, original).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
            let candidate_port = free_port();
            assert!(
                state
                    .rdp
                    .apply(
                        state.clone(),
                        Settings {
                            enabled: true,
                            port: candidate_port
                        }
                    )
                    .await
                    .is_err()
            );
            assert!(
                TcpListener::bind(("127.0.0.1", candidate_port)).is_ok(),
                "failed save closes candidate"
            );
            assert!(
                TcpListener::bind(("127.0.0.1", first)).is_err(),
                "old listener remains open"
            );
            assert!(state.rdp.status().await.enabled);
            assert_eq!(state.rdp.status().await.port, first);
            assert_eq!(fs::read(path).unwrap(), saved);
            assert!(
                state
                    .rdp
                    .apply(
                        state.clone(),
                        Settings {
                            enabled: true,
                            port: 0
                        }
                    )
                    .await
                    .is_err()
            );
            state.rdp.shutdown().await;
        }

        #[tokio::test]
        async fn explicit_tls_is_preserved_and_attached_desktop_is_rejected() {
            let dir = tempfile::tempdir().unwrap();
            let mut config = config(dir.path());
            let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert = dir.path().join("cert.pem");
            let key = dir.path().join("key.pem");
            fs::write(&cert, certificate.cert.pem()).unwrap();
            fs::write(&key, certificate.signing_key.serialize_pem()).unwrap();
            config.rdp.rdp_tls_cert = Some(cert.clone());
            config.rdp.rdp_tls_key = Some(key.clone());
            let original = fs::read(&key).unwrap();
            let desktop = desktop(config.clone());
            desktop
                .rdp
                .apply(
                    desktop.clone(),
                    Settings {
                        enabled: true,
                        port: free_port(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(fs::read(&key).unwrap(), original);
            assert!(
                !config
                    .rdp
                    .rdp_settings
                    .as_ref()
                    .unwrap()
                    .with_extension("identity.pem")
                    .exists()
            );
            desktop.rdp.shutdown().await;
            config.wayland_socket = Some("/tmp/physical".into());
            let attached = super::runtime::desktop(config);
            assert_eq!(
                attached.rdp.status().await.unavailable_reason,
                Some("attached")
            );
            assert!(
                attached
                    .rdp
                    .apply(
                        attached.clone(),
                        Settings {
                            enabled: true,
                            port: free_port()
                        }
                    )
                    .await
                    .is_err()
            );
        }

        #[tokio::test]
        async fn management_api_requires_basic_auth_origin_and_strict_ports_under_prefix() {
            use axum::http::Request;
            use base64::Engine;
            use tower::ServiceExt;
            let dir = tempfile::tempdir().unwrap();
            let mut config = config(dir.path());
            config.subfolder = "/private".into();
            let state = desktop(config);
            let router = crate::server::router(state.clone());
            let auth = format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("lightos:rdp-settings-test")
            );
            for (path, origin, auth_header, body, expected) in [
                ("/private/api/rdp", "http://localhost", "", "", 401),
                (
                    "/private/api/rdp",
                    "http://evil.example",
                    auth.as_str(),
                    "",
                    403,
                ),
                ("/api/rdp", "http://localhost", auth.as_str(), "", 404),
                (
                    "/private/api/rdp",
                    "http://localhost",
                    auth.as_str(),
                    r#"{"enabled":false,"port":0}"#,
                    400,
                ),
                (
                    "/private/api/rdp",
                    "http://localhost",
                    auth.as_str(),
                    r#"{"enabled":false,"port":65536}"#,
                    422,
                ),
                (
                    "/private/api/rdp",
                    "http://localhost",
                    auth.as_str(),
                    r#"{"enabled":false,"port":1.5}"#,
                    422,
                ),
                (
                    "/private/api/rdp",
                    "http://localhost",
                    auth.as_str(),
                    r#"{"enabled":false,"port":3389,"quality":50}"#,
                    422,
                ),
                (
                    "/private/api/rdp",
                    "http://localhost",
                    auth.as_str(),
                    r#"{"enabled":false,"port":65535}"#,
                    200,
                ),
            ] {
                let request = Request::builder()
                    .uri(path)
                    .method(if body.is_empty() { "GET" } else { "PUT" })
                    .header("Host", "localhost")
                    .header("Origin", origin)
                    .header("Authorization", auth_header)
                    .header("X-LightOS-RDP", "1")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body))
                    .unwrap();
                assert_eq!(
                    router
                        .clone()
                        .oneshot(request)
                        .await
                        .unwrap()
                        .status()
                        .as_u16(),
                    expected,
                    "{path} {body}"
                );
            }
            let missing_csrf = Request::builder()
                .uri("/private/api/rdp")
                .header("Host", "localhost")
                .header("Origin", "http://localhost")
                .header("Authorization", auth)
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                router.oneshot(missing_csrf).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
            state.rdp.shutdown().await;
        }
    }
}
