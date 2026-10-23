use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use axum::serve::{Listener, ListenerExt};
use clap::{Parser, Subcommand};
use lightos_remote_desktop::{
    config::Config,
    server::{AppState, router},
    session::Session,
    wayland::Backend,
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    match Cli::parse().command {
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
    if config.omarchy_path()?.is_some() {
        ensure!(
            executable_exists(Path::new("quickshell")),
            "missing executable: quickshell"
        );
        ensure!(
            executable_exists(Path::new("gtk-launch")),
            "missing executable: gtk-launch"
        );
    }
    if config.wayland_socket.is_none() {
        for command in [
            config.cage.as_path(),
            Path::new("Hyprland"),
            Path::new("dbus-daemon"),
        ] {
            ensure!(
                executable_exists(command),
                "missing executable: {}",
                command.display()
            );
        }
        ensure!(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/dri/renderD128")
                .is_ok(),
            "renderD128 must be accessible for headless GLES rendering"
        );
    }
    for command in ["hyprctl", "wl-copy", "wl-paste", "timeout"] {
        ensure!(
            executable_exists(Path::new(command)),
            "missing executable: {command}"
        );
    }
    if config.audio {
        ensure!(
            executable_exists(Path::new("pactl")),
            "missing executable: pactl"
        );
        if config.wayland_socket.is_none() {
            for command in ["pipewire", "pipewire-pulse", "wireplumber"] {
                ensure!(
                    executable_exists(Path::new(command)),
                    "missing executable: {command}"
                );
            }
        }
    }
    if let Some(directory) = &config.aquamarine_dir {
        ensure!(
            directory.join("libaquamarine.so.14").is_file(),
            "private Aquamarine library missing"
        );
    }
    println!(
        "{}",
        serde_json::json!({"distribution": platform_detect::Distribution::detect(), "omarchy": platform_detect::OmarchyInstallation::detect(), "persistent_session":config.session_runtime, "ok":true,"native":true,"transport":"websocket","video_codecs_compiled":["h264","jpeg"],"codec_probe_complete":false,"desktop_smoke_test":false})
    );
    Ok(())
}

fn executable_exists(path: &Path) -> bool {
    platform_detect::find_executable(path).is_some()
}

async fn desktop_session(mut config: Config) -> Result<()> {
    config.session_runtime = None;
    ensure!(
        config.wayland_socket.is_none(),
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
        let backend = Backend::start(session.socket.clone(), settings.fps, settings.quality)?;
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
    let state = AppState::new(config, backend, session);
    #[cfg(feature = "rdp")]
    let rdp = lightos_remote_desktop::rdp::start(state.clone()).await?;
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor = state.clone();
    let monitor_failed = failed.clone();
    tracing::info!(address = %listener.local_addr()?, "Rust desktop HTTP listener started");
    #[cfg(feature = "rdp")]
    let rdp_failure = rdp.as_ref().map(|service| service.failure_handle());
    let result = axum::serve(listener, router(state.clone())).with_graceful_shutdown(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                _ = interrupt.recv() => break,
                _ = term.recv() => break,
                _ = ticks.tick() => {
                    #[cfg(feature = "rdp")]
                    if rdp_failure.as_ref().is_some_and(|failed| failed.load(std::sync::atomic::Ordering::Acquire)) {
                        tracing::error!("RDP transport stopped; exiting for supervised recovery");
                        monitor_failed.store(true, std::sync::atomic::Ordering::Release);
                        break;
                    }
                    if !monitor.backend.frame_is_fresh(Duration::from_secs(5)) || !monitor.session.lock().is_ok_and(|mut session| session.healthy()) {
                        tracing::error!("desktop or capture stopped; exiting for supervised recovery");
                        monitor_failed.store(true, std::sync::atomic::Ordering::Release);
                        break;
                    }
                }
            }
        }
        monitor.shutdown_mcp();
        #[cfg(feature = "rdp")]
        if let Some(rdp) = rdp
            && let Err(error) = rdp.stop().await
        {
            tracing::error!(%error, "RDP transport did not stop cleanly");
            monitor_failed.store(true, std::sync::atomic::Ordering::Release);
        }
        monitor.backend.stop();
    }).await;
    state.shutdown_mcp();
    state.backend.stop();
    result?;
    ensure!(
        !failed.load(std::sync::atomic::Ordering::Acquire),
        "desktop session failed"
    );
    Ok(())
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
