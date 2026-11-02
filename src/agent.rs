//! An always available controller. It never opens a capture session itself.
mod instance;
mod network;
mod runtime;

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use runtime::Runtime;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, watch};

#[derive(clap::Args)]
pub struct Options {
    #[arg(long, help = "Existing Lazycat PC client core.cfg")]
    client_config: Option<PathBuf>,
    #[arg(long)]
    state_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct View {
    secret: String,
    status: Value,
}
struct Controller {
    view: watch::Receiver<View>,
    commands: mpsc::Sender<(String, String)>,
}

fn authorized(headers: &HeaderMap, view: &View) -> bool {
    headers
        .get("x-lazycat-remote-agent")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            !view.secret.is_empty() && bool::from(value.as_bytes().ct_eq(view.secret.as_bytes()))
        })
}

async fn status(
    State(controller): State<Arc<Controller>>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let view = controller.view.borrow().clone();
    if !authorized(&headers, &view) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(view.status))
}

async fn command(
    controller: Arc<Controller>,
    headers: HeaderMap,
    action: &str,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let view = controller.view.borrow().clone();
    if !authorized(&headers, &view) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    controller
        .commands
        .try_send((action.into(), view.secret))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((StatusCode::ACCEPTED, Json(json!({"accepted":true}))))
}

pub async fn run(options: Options) -> Result<()> {
    let home = dirs::home_dir().context("home directory unavailable")?;
    let config = options
        .client_config
        .unwrap_or_else(|| home.join(".config/lzc-client-desktop/core.cfg"));
    let directory = options
        .state_dir
        .unwrap_or_else(|| {
            crate::private_storage::state_directory()
                .unwrap_or_else(|| home.join(".lazycat-remote"))
        })
        .join("agent");
    let Some(_instance) = instance::acquire(&directory)? else {
        return Ok(());
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let (view_tx, view_rx) = watch::channel(View {
        secret: String::new(),
        status: json!({"status":"offline"}),
    });
    let (commands_tx, mut commands) = mpsc::channel::<(String, String)>(8);
    let controller = Arc::new(Controller {
        view: view_rx,
        commands: commands_tx,
    });
    let routes = Router::new()
        .route("/status", get(status))
        .route("/start", post(|State(c), h| command(c, h, "start")))
        .route("/stop", post(|State(c), h| command(c, h, "stop")))
        .route(
            "/permissions",
            post(|State(c), h| command(c, h, "permissions")),
        )
        .with_state(controller);
    let client = network::client()?;
    let mut runtime = Runtime::new();
    let mut connection: Option<network::Connection> = None;
    let mut password = runtime::secret();
    let mut enabled = false;
    let mut context_error = String::new();
    let mut next_heartbeat = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, routes)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
    });
    tracing::info!(
        port,
        "Lazycat Remote controller ready; desktop capture is disabled until requested"
    );
    loop {
        let step = async {
            tokio::select! {
                action = commands.recv() => {
                    let Some((action, expected_secret)) = action else { return Ok::<bool, anyhow::Error>(false); };
                    if expected_secret != password { return Ok(true); }
                    if let Some(active) = &connection {
                        match action.as_str() {
                            "start" => { enabled = true; if runtime.permissions["can_start"] != true { runtime.request_permissions().await?; } }
                            "stop" => { enabled = false; runtime.stop().await; }
                            "permissions" => { runtime.request_permissions().await?; }
                            _ => {}
                        }
                        save(&directory, &active.identity, enabled)?;
                        next_heartbeat = Instant::now();
                    }
                }
                _ = ticker.tick() => {}
            }
            if Instant::now() >= next_heartbeat {
                match network::Connection::discover(&config, &client).await {
                    Ok(active) => {
                        context_error.clear();
                        let changed = connection.as_ref().is_none_or(|old| {
                            old.identity != active.identity || old.token != active.token
                        });
                        if changed {
                            runtime.stop().await;
                            password = runtime::secret();
                            enabled = load(&directory, &active.identity).unwrap_or(false);
                        }
                        connection = Some(active);
                    }
                    Err(error) => {
                        let reason = error.to_string();
                        if reason != context_error {
                            tracing::warn!(%reason, "Active Lazycat login is unavailable");
                            context_error = reason;
                        }
                        // Never keep capture alive on a stale account or disconnected client.
                        runtime.stop().await;
                        enabled = false;
                        connection = None;
                        password = runtime::secret();
                    }
                }
                next_heartbeat = Instant::now() + Duration::from_secs(8);
            }
            if let Err(error) = runtime.tick(enabled, &directory, &client).await {
                runtime.error = Some(error.to_string());
            }
            let public = runtime.public(enabled);
            view_tx.send_replace(View {
                secret: password.clone(),
                status: public.clone(),
            });
            if let Some(active) = &connection {
                let epoch = &password[..32];
                let control = format!("cloud.lazycat.remote-desktop.control.{}.{epoch}", active.id);
                let mut desktop_service = String::new();
                let result: Result<()> = async {
                    active
                        .register(&control, port, "X-Lazycat-Remote-Agent", &password)
                        .await?;
                    if runtime.desktop["ready"] == true {
                        use base64::Engine;
                        let desktop_epoch = runtime.desktop["epoch"]
                            .as_str()
                            .context("missing runtime epoch")?;
                        desktop_service = format!(
                            "cloud.lazycat.remote-desktop.managed.{}.{desktop_epoch}",
                            active.id
                        );
                        let credential = format!(
                            "Basic {}",
                            base64::engine::general_purpose::STANDARD.encode(format!(
                                "lightos:{}",
                                runtime.desktop["password"].as_str().unwrap_or("")
                            ))
                        );
                        active
                            .register(
                                &desktop_service,
                                runtime.desktop["port"].as_u64().unwrap_or(0) as u16,
                                "Authorization",
                                &credential,
                            )
                            .await?;
                    }
                    active
                        .publish(
                            &control,
                            &password,
                            &desktop_service,
                            runtime.desktop.clone(),
                            public,
                        )
                        .await
                }
                .await;
                if result.is_err() {
                    tracing::warn!(
                        "Remote management registration unavailable; retrying with the active login"
                    );
                }
            }
            Ok::<bool, anyhow::Error>(true)
        };
        tokio::select! {
            _ = shutdown_signal() => break,
            result = step => {
                match result {
                    Ok(true) => {},
                    Ok(false) => break,
                    Err(error) => { runtime.error = Some(error.to_string()); }
                }
            }
        }
    }
    runtime.stop().await;
    let _ = stop_tx.send(());
    let _ = server.await;
    Ok(())
}

fn save(directory: &std::path::Path, identity: &str, enabled: bool) -> Result<()> {
    crate::private_storage::current().write(
        &directory.join("state.json"),
        &serde_json::to_vec(&json!({"identity":identity,"enabled":enabled}))?,
    )
}
fn load(directory: &std::path::Path, identity: &str) -> Result<bool> {
    let Some(bytes) =
        crate::private_storage::current().read(&directory.join("state.json"), 8192)?
    else {
        return Ok(false);
    };
    let state: Value = serde_json::from_slice(&bytes)?;
    ensure!(state["identity"] == identity, "account changed");
    Ok(state["enabled"] == true)
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("signal handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
