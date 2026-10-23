use std::{
    ffi::OsStr,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use axum::serve::{Listener, ListenerExt};
use clap::{Parser, Subcommand};
use lightos_remote_desktop::{
    backend::{self, BackendKind, DesktopBackend},
    config::Config,
    server::{AppState, router},
    session::Session,
};

#[derive(Parser)]
#[command(name = "lightos-remoted", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    #[command(about = "Install this binary and both LightOS services, or manage package lifecycle")]
    InstallService(lightos_remote_desktop::install::ServiceOptions),
    #[command(about = "Restore the previous standalone service installation")]
    RollbackService(lightos_remote_desktop::install::ServiceOptions),
    Serve(Config),
    #[command(about = "Own a persistent desktop independently of the HTTP transport")]
    DesktopSession(Config),
    Doctor(Config),
    Probe {
        #[command(flatten)]
        config: Config,
        #[arg(long)]
        expect_pid: Option<u32>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args_os().len() == 2
        && std::env::args_os().nth(1).as_deref() == Some(OsStr::new("--internal-x11-worker"))
    {
        return lightos_remote_desktop::backend::x11::worker_main();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut cli = Cli::parse();
    match &mut cli.command {
        Action::Serve(config) | Action::Doctor(config) | Action::Probe { config, .. } => {
            config.apply_service_defaults(
                std::env::var_os("LIGHTOS_REMOTE_DESKTOP_DEFAULT_SESSION_RUNTIME")
                    .map(PathBuf::from),
            );
        }
        Action::InstallService(_) | Action::RollbackService(_) | Action::DesktopSession(_) => {}
    }
    match cli.command {
        Action::InstallService(options) => lightos_remote_desktop::install::service(options, false),
        Action::RollbackService(options) => lightos_remote_desktop::install::service(options, true),
        Action::Doctor(config) => doctor(&config),
        Action::Probe { config, expect_pid } => {
            config.validate()?;
            tokio::task::spawn_blocking(move || probe(&config, expect_pid)).await?
        }
        Action::Serve(config) => serve(config).await,
        Action::DesktopSession(config) => desktop_session(config).await,
    }
}

fn doctor(config: &Config) -> Result<()> {
    config.validate()?;
    use lightos_remote_desktop::diagnostics::{SessionMode, dependencies};
    let mode = SessionMode::from_config(config);
    let omarchy = mode == SessionMode::OwnedVirtual && config.omarchy_path()?.is_some();
    let dependencies = dependencies(config, omarchy, platform_detect::find_executable);
    let missing: Vec<_> = dependencies
        .required
        .iter()
        .filter(|dependency| !dependency.available)
        .map(|dependency| dependency.program.display().to_string())
        .collect();
    ensure!(
        missing.is_empty(),
        "missing required executables: {}",
        missing.join(", ")
    );
    if let Some(socket) = &config.wayland_socket {
        lightos_remote_desktop::session::validate_wayland_socket(socket)?;
    }
    if mode == SessionMode::ExternalX11 {
        Session::physical_target(config)?;
    }
    if mode == SessionMode::OwnedVirtual
        && let Some(directory) = &config.aquamarine_dir
    {
        ensure!(
            directory.join("libaquamarine.so.14").is_file(),
            "private Aquamarine library missing"
        );
    }
    println!(
        "{}",
        serde_json::json!({"distribution": platform_detect::Distribution::detect(), "omarchy": platform_detect::OmarchyInstallation::detect(), "persistent_session":config.session_runtime, "dependencies":dependencies, "ok":true,"native":true,"transport":"websocket","video_codecs_compiled":["h264","jpeg"],"codec_probe_complete":false,"desktop_smoke_test":false})
    );
    Ok(())
}

async fn desktop_session(mut config: Config) -> Result<()> {
    config.session_runtime = None;
    ensure!(
        config.wayland_socket.is_none() && config.x11_display.is_none(),
        "desktop-session cannot own an attached compositor"
    );
    ensure!(
        config.runtime_dir.is_some(),
        "desktop-session requires an explicit persistent --runtime-dir"
    );
    doctor(&config)?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let startup_cancelled = cancelled.clone();
    let mut startup =
        tokio::task::spawn_blocking(move || Session::start_cancellable(&config, startup_cancelled));
    let mut session = tokio::select! {
        result = &mut startup => result??,
        _ = async { tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} } } => {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
            let _ = startup.await;
            return Ok(());
        }
    };
    lightos_remote_desktop::handoff::Descriptor::publish(&session)?;
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    let result = loop {
        tokio::select! {
            _ = term.recv() => break Ok(()),
            _ = interrupt.recv() => break Ok(()),
            _ = ticks.tick() => if !session.healthy() { break Err(anyhow::anyhow!("owned desktop process exited")); }
        }
    };
    lightos_remote_desktop::handoff::Descriptor::remove(&session.runtime);
    drop(session);
    result
}

async fn serve(config: Config) -> Result<()> {
    doctor(&config)?;
    ensure!(
        unsafe { libc::geteuid() } != 0,
        "refusing to run desktop as root"
    );
    if std::env::var("LIGHTOS_REMOTE_DESKTOP_ENABLE").as_deref() == Ok("0") {
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind((config.listen, config.port))
        .await
        .context("binding desktop HTTP listener")?
        .tap_io(|stream| {
            if let Err(error) = stream.set_nodelay(true) {
                tracing::warn!(%error, "could not enable TCP_NODELAY");
            }
        });
    let settings = config.clone();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let startup_cancelled = cancelled.clone();
    let mut startup = tokio::task::spawn_blocking(move || -> Result<_> {
        let session = Session::start_cancellable(&settings, startup_cancelled.clone())?;
        ensure!(
            !startup_cancelled.load(std::sync::atomic::Ordering::Acquire),
            "startup cancelled"
        );
        let target = session.target();
        let selection = backend::probe(&target, false, Some(startup_cancelled.as_ref()))?;
        let selected = selection.require_available()?;
        let backend: Arc<dyn DesktopBackend> =
            if defer_initial_consent(session.is_owned(), selected) {
                tracing::info!("Portal consent deferred until an explicit desktop connection");
                Arc::new(backend::DeferredPortalBackend::new())
            } else {
                let active = backend::start_selected(
                    selected,
                    target.clone(),
                    settings.fps,
                    settings.quality,
                    None,
                    startup_cancelled.clone(),
                )?;
                if session.is_owned() {
                    active
                } else {
                    backend::guarded::attach(active, target)?
                }
            };
        Ok((session, backend))
    });
    let (session, backend) = tokio::select! {
        result = &mut startup => result??,
        _ = async { tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} } } => {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
            if let Ok(Ok((session, backend))) = startup.await { backend.stop(); drop(session); }
            return Ok(());
        }
    };
    let state = AppState::with_backend(config, backend, session);
    state.rdp.initialize(state.clone()).await;
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor = state.clone();
    let monitor_failed = failed.clone();
    tracing::info!(address = %listener.local_addr()?, "Rust desktop HTTP listener started");
    let result = axum::serve(listener, router(state.clone())).with_graceful_shutdown(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                _ = interrupt.recv() => break,
                _ = term.recv() => break,
                _ = ticks.tick() => {
                    if !monitor.session.lock().is_ok_and(|mut session| {
                        session.healthy() && !monitor.backend.is_stopped()
                            && (monitor.backend.paused() || monitor.backend.frame_is_fresh(Duration::from_secs(5)))
                    }) {
                        tracing::error!("desktop or capture stopped; exiting for supervised recovery");
                        monitor_failed.store(true, std::sync::atomic::Ordering::Release);
                        break;
                    }
                }
            }
        }
        monitor.shutdown_mcp();
        monitor.rdp.shutdown().await;
        monitor.backend.stop();
    }).await;
    state.shutdown_mcp();
    state.rdp.shutdown().await;
    state.backend.stop();
    result?;
    ensure!(
        !failed.load(std::sync::atomic::Ordering::Acquire),
        "desktop session failed"
    );
    Ok(())
}

fn defer_initial_consent(owned: bool, selected: BackendKind) -> bool {
    !owned && selected == BackendKind::Portal
}

fn probe(config: &Config, expected: Option<u32>) -> Result<()> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.port);
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let ready = (|| -> Result<()> {
            let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
            write!(
                stream,
                "GET /healthz HTTP/1.0\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            )?;
            let mut response = String::new();
            stream.take(16 * 1024).read_to_string(&mut response)?;
            let (headers, body) = response
                .split_once("\r\n\r\n")
                .context("invalid health response")?;
            ensure!(
                headers.starts_with("HTTP/1.0 200") || headers.starts_with("HTTP/1.1 200"),
                "desktop not ready"
            );
            let health: serde_json::Value = serde_json::from_str(body)?;
            ensure!(
                health["native"] == true && health["ready"] == true,
                "not a ready native desktop"
            );
            if let Some(pid) = expected {
                ensure!(
                    health["pid"].as_u64() == Some(pid.into()),
                    "health response belongs to another process"
                );
            }
            Ok(())
        })();
        if ready.is_ok() {
            println!("native desktop ready");
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "native desktop failed its readiness probe"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_attached_portal_waits_for_browser_consent() {
        assert!(defer_initial_consent(false, BackendKind::Portal));
        for kind in [BackendKind::Wayland, BackendKind::X11] {
            assert!(!defer_initial_consent(false, kind));
        }
        assert!(!defer_initial_consent(true, BackendKind::Portal));
    }
}
