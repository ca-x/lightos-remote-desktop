use std::{
    fs::File,
    io::BufReader,
    net::{SocketAddr, TcpListener},
    num::{NonZeroU16, NonZeroUsize},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use bytes::Bytes;
use ironrdp_server::{
    BitmapUpdate, CredentialDecision, CredentialValidationError, CredentialValidator, Credentials,
    DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
    tokio_rustls::{TlsAcceptor, rustls},
};
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, watch};

use crate::{
    backend::{Frame, InputCommand},
    server::{AppState, ControllerLease},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Owns only the RDP transport thread. It never owns/restarts the desktop.
pub struct RdpService {
    shutdown: watch::Sender<bool>,
    activate: watch::Sender<bool>,
    worker: Option<JoinHandle<Result<()>>>,
    failed: Arc<AtomicBool>,
    address: SocketAddr,
    pub(super) fingerprint: String,
}

impl RdpService {
    pub(super) fn activate(&self) {
        self.activate.send_replace(true);
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub fn failure_handle(&self) -> Arc<AtomicBool> {
        self.failed.clone()
    }

    pub async fn stop(mut self) -> Result<()> {
        self.shutdown.send_replace(true);
        if let Some(worker) = self.worker.take() {
            tokio::task::spawn_blocking(move || {
                worker
                    .join()
                    .map_err(|_| anyhow::anyhow!("RDP worker panicked"))?
            })
            .await??;
        }
        Ok(())
    }
}

impl Drop for RdpService {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}

pub async fn start(state: AppState) -> Result<Option<RdpService>> {
    if !state.config.rdp.rdp {
        return Ok(None);
    }
    start_with(state.clone(), &state.config.rdp).await.map(Some)
}

pub(super) async fn start_with(state: AppState, settings: &super::RdpConfig) -> Result<RdpService> {
    let service = prepare(state, settings).await?;
    service.activate();
    Ok(service)
}

pub(super) async fn prepare(state: AppState, settings: &super::RdpConfig) -> Result<RdpService> {
    state.config.validate()?;
    settings.validate(
        !cfg!(target_os = "linux")
            || state.config.wayland_socket.is_some()
            || state.config.x11_display.is_some(),
    )?;
    let mut cert_reader = BufReader::new(
        File::open(
            settings
                .rdp_tls_cert
                .as_ref()
                .context("missing RDP certificate")?,
        )
        .context("opening RDP certificate")?,
    );
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading RDP PEM certificate chain")?;
    ensure!(!certs.is_empty(), "RDP certificate chain is empty");
    let mut key_reader = BufReader::new(
        File::open(
            settings
                .rdp_tls_key
                .as_ref()
                .context("missing RDP private key")?,
        )
        .context("opening RDP private key")?,
    );
    let key = rustls_pemfile::private_key(&mut key_reader)
        .context("reading RDP PEM private key")?
        .context("RDP private key is missing")?;
    let fingerprint = {
        use sha2::{Digest, Sha256};
        Sha256::digest(certs[0].as_ref())
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    };
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .context("invalid RDP TLS certificate/key")?;
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind(settings.rdp_listen).context("binding RDP listener")?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let (shutdown, mut stop) = watch::channel(false);
    let (activate, mut activation) = watch::channel(false);
    let failed = Arc::new(AtomicBool::new(false));
    let worker_failed = failed.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let worker = std::thread::Builder::new()
        .name("rdp-transport".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let _ = ready_tx.send(());
                runtime.block_on(async move {
                    tokio::select! {
                        biased;
                        _ = stop.changed() => return Ok(()),
                        _ = activation.changed() => {},
                    }
                    if !*activation.borrow() {
                        return Ok(());
                    }
                    accept_connections(listener, acceptor, state, stop).await
                })
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("RDP worker panicked")));
            if let Err(error) = &result {
                tracing::error!(%error, "RDP transport failed");
            }
            worker_failed.store(true, Ordering::Release);
            result
        })?;
    tracing::info!(%address, "authenticated TLS RDP listener started");
    let service = RdpService {
        shutdown,
        activate,
        worker: Some(worker),
        failed,
        address,
        fingerprint,
    };
    if let Err(error) = ready_rx.await {
        let _ = service.stop().await;
        return Err(error).context("starting RDP worker");
    }
    Ok(service)
}

async fn accept_connections(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    state: AppState,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    loop {
        let (stream, peer) = tokio::select! {
            biased;
            _ = stop.changed() => return Ok(()),
            accepted = listener.accept() => accepted?,
        };
        stream.set_nodelay(true)?;
        let connection = Arc::new(Connection {
            state: state.clone(),
            authenticated: AtomicBool::new(false),
            control: Mutex::new(Control::default()),
            ready: Notify::new(),
            close: Notify::new(),
            #[cfg(test)]
            before_input_enqueue: Mutex::new(None),
        });
        let mut server = RdpServer::builder()
            .with_addr(listener.local_addr()?)
            .with_tls(acceptor.clone())
            .with_input_handler(Input {
                connection: connection.clone(),
                surrogate: None,
            })
            .with_display_handler(Display {
                connection: connection.clone(),
                announced: Arc::default(),
            })
            .with_credential_validator(Some(connection.clone()))
            .with_max_request_size(1024 * 1024)
            .build();
        // A new server per TCP connection keeps protocol, framebuffer and input
        // state isolated. Only an authenticated activated session claims control.
        let result = tokio::select! {
            result = server.run_connection(stream) => result,
            _ = stop.changed() => Ok(()),
            _ = connection.close.notified() => Err(anyhow::anyhow!("RDP input route unavailable")),
            _ = async {
                tokio::select! {
                    _ = connection.ready.notified() => std::future::pending::<()>().await,
                    _ = tokio::time::sleep(HANDSHAKE_TIMEOUT) => {},
                }
            } => Err(anyhow::anyhow!("RDP authentication/activation timed out")),
        };
        let lease = connection.deactivate();
        drop(server);
        if let Some(lease) = lease {
            lease.release().await;
        }
        if let Err(error) = result {
            // Do not log protocol errors: a peer may place credentials inside
            // decoder diagnostics. The transport closes and releases its lease.
            let _ = error;
            tracing::warn!(%peer, "RDP connection ended before graceful disconnect");
        }
        if *stop.borrow() {
            return Ok(());
        }
    }
}

struct Connection {
    state: AppState,
    authenticated: AtomicBool,
    control: Mutex<Control>,
    ready: Notify,
    close: Notify,
    #[cfg(test)]
    before_input_enqueue: Mutex<Option<tests::InputPause>>,
}

#[derive(Default)]
struct Control {
    active: bool,
    lease: Option<ControllerLease>,
}

impl Connection {
    fn deactivate(&self) -> Option<ControllerLease> {
        // IronRDP also dispatches input on blocking workers. Closing admission
        // under the enqueue lock puts every accepted event before ReleaseSync.
        let mut control = self.control.lock().unwrap();
        control.active = false;
        control.lease.take()
    }
}

#[async_trait]
impl CredentialValidator for Connection {
    async fn validate(
        &self,
        credentials: &Credentials,
    ) -> std::result::Result<CredentialDecision, CredentialValidationError> {
        let accepted = credentials
            .username
            .as_bytes()
            .ct_eq(self.state.config.username.as_bytes())
            & credentials
                .password
                .as_bytes()
                .ct_eq(self.state.config.password.as_bytes());
        if bool::from(accepted) && credentials.domain.as_deref().is_none_or(str::is_empty) {
            self.authenticated.store(true, Ordering::Release);
            Ok(CredentialDecision::Accept)
        } else {
            Ok(CredentialDecision::Reject)
        }
    }
}

struct Display {
    connection: Arc<Connection>,
    announced: Arc<Mutex<Option<(u32, u32)>>>,
}

#[async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        // IronRDP calls size() again when preparing the activation encoder.
        // Only the initial announcement or a Resize may advance this size;
        // capture can keep changing throughout activation/reactivation.
        let size = *self.announced.lock().unwrap().get_or_insert_with(|| {
            self.connection
                .state
                .backend
                .frames()
                .borrow()
                .as_ref()
                .map(|frame| (frame.width, frame.height))
                .unwrap_or_else(|| self.connection.state.config.dimensions().unwrap())
        });
        DesktopSize {
            width: size.0 as u16,
            height: size.1 as u16,
        }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        ensure!(
            self.connection.authenticated.load(Ordering::Acquire),
            "RDP credentials are required"
        );
        let mut control = self.connection.control.lock().unwrap();
        if control.lease.is_none() {
            control.lease = Some(self.connection.state.claim_rdp_control()?);
        }
        control.active = true;
        self.connection.ready.notify_one();
        Ok(Box::new(Updates {
            frames: self.connection.state.backend.frames(),
            first: true,
            hide_pointer: true,
            announced: self.announced.clone(),
        }))
    }
}

struct Updates {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    first: bool,
    hide_pointer: bool,
    announced: Arc<Mutex<Option<(u32, u32)>>>,
}

#[async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        if std::mem::take(&mut self.hide_pointer) {
            // Capture already contains the compositor cursor, including its
            // application-defined shape; avoid a second client-side pointer.
            return Ok(Some(DisplayUpdate::HidePointer));
        }
        loop {
            if !self.first && self.frames.changed().await.is_err() {
                return Ok(None);
            }
            let Some(frame) = self.frames.borrow_and_update().clone() else {
                self.first = false;
                continue;
            };
            ensure!(
                frame.captured_at.elapsed() <= Duration::from_secs(5),
                "RDP capture is stale"
            );
            let size = (frame.width, frame.height);
            let resized = {
                let mut announced = self.announced.lock().unwrap();
                let resized = announced.is_some_and(|old| old != size);
                *announced = Some(size);
                resized
            };
            if resized {
                self.first = true;
                return Ok(Some(DisplayUpdate::Resize(DesktopSize {
                    width: u16::try_from(frame.width)?,
                    height: u16::try_from(frame.height)?,
                })));
            }
            self.first = false;
            return Ok(Some(DisplayUpdate::Bitmap(bitmap(frame)?)));
        }
    }
}

fn bitmap(frame: Arc<Frame>) -> Result<BitmapUpdate> {
    let width = NonZeroU16::new(u16::try_from(frame.width)?).context("empty RDP frame")?;
    let height = NonZeroU16::new(u16::try_from(frame.height)?).context("empty RDP frame")?;
    ensure!(
        frame.pixels.len() == usize::from(width.get()) * usize::from(height.get()) * 4,
        "invalid RDP pixel length"
    );
    // Standard uncompressed RDP surface data is BGR(A). IronRDP's raw
    // surface encoder forwards bytes directly, so normalize here rather than
    // relying on format metadata to swizzle our RGBA capture for every codec.
    let mut pixels = frame.pixels.to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(BitmapUpdate {
        x: 0,
        y: 0,
        width,
        height,
        format: PixelFormat::BgrA32,
        data: Bytes::from(pixels),
        stride: NonZeroUsize::new(usize::from(width.get()) * 4).unwrap(),
    })
}

struct Input {
    connection: Arc<Connection>,
    surrogate: Option<u16>,
}

impl Input {
    fn send(&self, command: InputCommand) {
        if command.validate().is_err() {
            return;
        }
        let mut control = self.connection.control.lock().unwrap();
        if !control.active {
            return;
        }
        #[cfg(test)]
        if let Some(pause) = self.connection.before_input_enqueue.lock().unwrap().take() {
            pause.wait();
        }
        if self
            .connection
            .state
            .backend
            .input()
            .try_send(command)
            .is_err()
        {
            // Losing a key release would leave the compositor in a stuck state.
            // Close and run the same release barrier as the browser path.
            control.active = false;
            self.connection.close.notify_one();
        }
    }
}

impl RdpServerInputHandler for Input {
    fn keyboard(&mut self, event: KeyboardEvent) {
        match event {
            KeyboardEvent::Pressed { code, extended }
            | KeyboardEvent::Released { code, extended } => {
                let pressed = matches!(event, KeyboardEvent::Pressed { .. });
                if let Some(code) = evdev_scancode(code, extended) {
                    self.send(InputCommand::Key { code, pressed });
                }
            }
            KeyboardEvent::UnicodePressed(unit) => {
                if (0xd800..=0xdbff).contains(&unit) {
                    self.surrogate = Some(unit);
                    return;
                }
                let text = if let Some(high) = self.surrogate.take() {
                    char::decode_utf16([high, unit])
                        .collect::<std::result::Result<String, _>>()
                        .ok()
                } else {
                    char::from_u32(u32::from(unit)).map(|value| value.to_string())
                };
                if let Some(text) = text {
                    self.send(InputCommand::Text { text });
                }
            }
            KeyboardEvent::UnicodeReleased(_) | KeyboardEvent::Synchronize(_) => {}
        }
    }

    fn mouse(&mut self, event: MouseEvent) {
        let command = match event {
            MouseEvent::Move { x, y } => {
                let frames = self.connection.state.backend.frames();
                let frame = frames.borrow();
                let Some(frame) = frame.as_ref() else { return };
                InputCommand::Move {
                    x: (f64::from(x) / f64::from(frame.width.saturating_sub(1).max(1)))
                        .clamp(0.0, 1.0),
                    y: (f64::from(y) / f64::from(frame.height.saturating_sub(1).max(1)))
                        .clamp(0.0, 1.0),
                }
            }
            MouseEvent::LeftPressed => InputCommand::Button {
                button: 0,
                pressed: true,
            },
            MouseEvent::LeftReleased => InputCommand::Button {
                button: 0,
                pressed: false,
            },
            MouseEvent::MiddlePressed => InputCommand::Button {
                button: 1,
                pressed: true,
            },
            MouseEvent::MiddleReleased => InputCommand::Button {
                button: 1,
                pressed: false,
            },
            MouseEvent::RightPressed => InputCommand::Button {
                button: 2,
                pressed: true,
            },
            MouseEvent::RightReleased => InputCommand::Button {
                button: 2,
                pressed: false,
            },
            MouseEvent::VerticalScroll { value } => InputCommand::Scroll {
                x: 0.0,
                y: (-f64::from(value) / 8.0).clamp(-1000.0, 1000.0),
            },
            MouseEvent::Scroll { x, y } => InputCommand::Scroll {
                x: (f64::from(x) / 8.0).clamp(-1000.0, 1000.0),
                y: (-f64::from(y) / 8.0).clamp(-1000.0, 1000.0),
            },
            _ => return,
        };
        self.send(command);
    }
}

// PC/AT set-1 scancodes share the base evdev range. E0 codes need an
// explicit Linux input-event mapping; unknown vendor/media codes are ignored.
fn evdev_scancode(code: u8, extended: bool) -> Option<u32> {
    if !extended {
        return match code {
            1..=83 | 86..=88 => Some(u32::from(code)),
            0x64..=0x6f => Some(u32::from(code) - 0x64 + 183),
            _ => None,
        };
    }
    Some(match code {
        0x1c => 96,
        0x1d => 97,
        0x35 => 98,
        0x37 => 99,
        0x38 => 100,
        0x47 => 102,
        0x48 => 103,
        0x49 => 104,
        0x4b => 105,
        0x4d => 106,
        0x4f => 107,
        0x50 => 108,
        0x51 => 109,
        0x52 => 110,
        0x53 => 111,
        0x5b => 125,
        0x5c => 126,
        0x5d => 127,
        _ => return None,
    })
}

#[cfg(all(test, target_os = "linux"))]
#[path = "tests.rs"]
mod tests;
