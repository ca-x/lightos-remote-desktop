//! Portal capture, input and clipboard tied to one explicitly selected user session.
use std::{
    collections::BTreeSet,
    os::fd::OwnedFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc as sync_mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use ashpd::desktop::{
    Session,
    clipboard::{Clipboard, RequestClipboardOptions, SetSelectionOptions},
    remote_desktop::{
        DeviceType, KeyState, NotifyKeyboardKeycodeOptions, NotifyKeyboardKeysymOptions,
        NotifyPointerAxisOptions, NotifyPointerButtonOptions, RemoteDesktop, SelectDevicesOptions,
        StartOptions,
    },
    screencast::{
        CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    },
};
use display_control::SessionTarget;
use futures_util::{StreamExt, future::BoxFuture};
use pipewire as pw;
use pw::spa::{
    self,
    param::{
        format::{MediaSubtype, MediaType},
        format_utils,
        video::{VideoFormat, VideoInfoRaw},
    },
    pod::Pod,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::{interval, timeout},
};

use super::super::{
    BackendKind, CursorFrame, DesktopBackend, Frame, InputCommand, InputGuard, MAX_CLIPBOARD_BYTES,
};

#[path = "buffer.rs"]
mod buffer;
#[path = "cursor.rs"]
mod cursor;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const START_TIMEOUT: Duration = Duration::from_secs(28);
const START_TOTAL_TIMEOUT: Duration = Duration::from_secs(40);
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(9);
const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

async fn connection(target: &SessionTarget) -> Result<zbus::Connection> {
    let address = target
        .bus_address
        .as_deref()
        .context("selected desktop has no session bus")?;
    zbus::connection::Builder::address(address)?
        .build()
        .await
        .context("connect to selected desktop session bus")
}

/// Read-only session capability check. No CreateSession, input device or consent request.
pub fn probe(target: &SessionTarget) -> Result<()> {
    ensure!(
        target.wayland_socket.is_some(),
        "Portal requires an explicit Wayland session"
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        timeout(PROBE_TIMEOUT, async {
            let bus = connection(target).await?;
            let desktop = RemoteDesktop::with_connection(bus.clone()).await?;
            let cast = Screencast::with_connection(bus.clone()).await?;
            super::identity::validate_target_identity(&bus, target).await?;
            ensure!(
                desktop.version() >= 1 && cast.version() >= 1,
                "Portal RemoteDesktop and ScreenCast are required"
            );
            let devices = desktop.available_device_types().await?;
            ensure!(
                devices.contains(DeviceType::Keyboard | DeviceType::Pointer),
                "Portal does not advertise keyboard and pointer input"
            );
            let sources = cast.available_source_types().await?;
            ensure!(
                sources.contains(SourceType::Monitor),
                "Portal does not advertise monitor capture"
            );
            let cursors = cast.available_cursor_modes().await?;
            ensure!(
                cursors.contains(CursorMode::Metadata),
                "Portal cannot provide independent cursor metadata"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("Portal capability probe timed out")?
    })
}

enum Control {
    Stop,
    Read(oneshot::Sender<Result<String>>),
    Write(String, oneshot::Sender<Result<()>>),
}

pub struct Backend {
    cursors: watch::Receiver<Option<Arc<CursorFrame>>>,
    frames: watch::Receiver<Option<Arc<Frame>>>,
    input: mpsc::Sender<InputCommand>,
    control: mpsc::UnboundedSender<Control>,
    stopped: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Backend {
    pub fn start(
        target: SessionTarget,
        fps: u32,
        _quality: u8,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        ensure!((1..=60).contains(&fps), "invalid Portal capture FPS");
        ensure!(
            target.wayland_socket.is_some(),
            "Portal requires a selected Wayland session"
        );
        // Portal stream ids are not Wayland output names. A specific output must
        // only be accepted once the portal has returned an exact matching id.
        ensure!(
            output.is_none(),
            "Portal cannot safely match a selected Wayland output; choose a screen in the consent dialog"
        );
        let (frames_tx, frames) = watch::channel(None);
        let (cursors_tx, cursors) = watch::channel(None);
        let (input, commands) = mpsc::channel(128);
        let (control, requests) = mpsc::unbounded_channel();
        let (ready, initialized) = sync_mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let worker = thread::Builder::new()
            .name("portal-desktop".into())
            .spawn(move || {
                let mut ready = Some(ready);
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let result = runtime.block_on(run(
                        target,
                        frames_tx,
                        cursors_tx,
                        commands,
                        requests,
                        cancelled,
                        worker_stopped,
                        &mut ready,
                    ));
                    runtime.shutdown_timeout(Duration::from_secs(1));
                    result
                })();
                if let Some(sender) = ready {
                    let _ = sender.send(result);
                } else if let Err(error) = result {
                    tracing::warn!(%error, "Portal desktop session stopped");
                }
            })?;
        let started = initialized.recv_timeout(START_TOTAL_TIMEOUT);
        match started {
            Ok(Ok(())) => Ok(Self {
                cursors,
                frames,
                input,
                control,
                stopped,
                worker: Mutex::new(Some(worker)),
            }),
            result => {
                stopped.store(true, Ordering::Release);
                let _ = control.send(Control::Stop);
                // Consent and first frame waits are bounded inside run. A worker
                // still running after the outer deadline owns its cleanup.
                if worker.is_finished() {
                    let _ = worker.join();
                }
                match result {
                    Ok(Err(error)) => Err(error),
                    Err(error) => Err(anyhow::anyhow!(
                        "Portal desktop startup did not complete: {error}"
                    )),
                    _ => unreachable!(),
                }
            }
        }
    }
}

impl DesktopBackend for Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::Portal
    }
    fn cursor_frames(&self) -> Option<watch::Receiver<Option<Arc<CursorFrame>>>> {
        Some(self.cursors.clone())
    }
    fn cursor_embedded(&self) -> bool {
        false
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn stop(&self) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            let _ = self.control.send(Control::Stop);
        }
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async {
            let (reply, response) = oneshot::channel();
            self.control
                .send(Control::Read(reply))
                .context("Portal clipboard is closed")?;
            timeout(CALL_TIMEOUT, response)
                .await
                .context("Portal clipboard read timed out")??
        })
    }
    fn clipboard_write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ensure!(
                text.len() <= MAX_CLIPBOARD_BYTES,
                "clipboard text exceeds 64 KiB"
            );
            let (reply, response) = oneshot::channel();
            self.control
                .send(Control::Write(text.to_owned(), reply))
                .context("Portal clipboard is closed")?;
            timeout(CALL_TIMEOUT, response)
                .await
                .context("Portal clipboard write timed out")??
        })
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn cancelled(flag: Arc<AtomicBool>, stopped: Arc<AtomicBool>) {
    let mut ticker = interval(Duration::from_millis(20));
    loop {
        ticker.tick().await;
        if flag.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)] // Capture and cursor streams share one Portal session.
async fn run(
    target: SessionTarget,
    frames: watch::Sender<Option<Arc<Frame>>>,
    cursors: watch::Sender<Option<Arc<CursorFrame>>>,
    mut input: mpsc::Receiver<InputCommand>,
    mut control: mpsc::UnboundedReceiver<Control>,
    cancelled_flag: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    ready: &mut Option<sync_mpsc::SyncSender<Result<()>>>,
) -> Result<()> {
    let bus = timeout(PROBE_TIMEOUT, connection(&target))
        .await
        .context("Portal session bus timed out")??;
    let (desktop, cast) = timeout(PROBE_TIMEOUT, async {
        Ok::<_, anyhow::Error>((
            RemoteDesktop::with_connection(bus.clone()).await?,
            Screencast::with_connection(bus.clone()).await?,
        ))
    })
    .await
    .context("Portal interface initialization timed out")??;
    let identity = timeout(
        PROBE_TIMEOUT,
        super::identity::validate_target_identity(&bus, &target),
    )
    .await
    .context("Portal target identity check timed out")??;
    let session = timeout(CALL_TIMEOUT, desktop.create_session(Default::default()))
        .await
        .context("Portal CreateSession timed out")??;
    let mut capture = None;
    let mut held_keys = BTreeSet::new();
    let mut held_buttons = BTreeSet::new();
    let result = tokio::select! {
        result = open_and_serve(bus, &identity, &desktop, &cast, &session, frames.clone(), cursors, &mut input, &mut control, stopped.clone(), &mut capture, &mut held_keys, &mut held_buttons, ready) => result,
        _ = cancelled(cancelled_flag, stopped.clone()) => Err(anyhow::anyhow!("Portal session cancelled")),
    };
    stop_capture_worker(&stopped, &mut capture, &frames);
    let _ = timeout(
        CALL_TIMEOUT,
        release_input(&desktop, &session, &mut held_keys, &mut held_buttons),
    )
    .await;
    // Close session even if consent was cancelled or video failed; never retry
    // authorization or fall back to another backend after a Portal attempt.
    let _ = timeout(CALL_TIMEOUT, session.close()).await;
    result
}

fn stop_capture_worker(
    stopped: &AtomicBool,
    capture: &mut Option<JoinHandle<()>>,
    frames: &watch::Sender<Option<Arc<Frame>>>,
) {
    stopped.store(true, Ordering::Release);
    frames.send_replace(None);
    if let Some(capture) = capture.take() {
        let _ = capture.join();
    }
    // The worker may have dequeued one last buffer after cancellation.
    frames.send_replace(None);
}

#[allow(clippy::too_many_arguments)] // All arguments belong to the one Portal session and its owner.
async fn open_and_serve(
    bus: zbus::Connection,
    identity: &super::identity::ScopeProof,
    desktop: &RemoteDesktop,
    cast: &Screencast,
    session: &Session<RemoteDesktop>,
    frames: watch::Sender<Option<Arc<Frame>>>,
    cursors: watch::Sender<Option<Arc<CursorFrame>>>,
    input: &mut mpsc::Receiver<InputCommand>,
    control: &mut mpsc::UnboundedReceiver<Control>,
    stopped: Arc<AtomicBool>,
    capture: &mut Option<JoinHandle<()>>,
    held_keys: &mut BTreeSet<u32>,
    held_buttons: &mut BTreeSet<u32>,
    ready: &mut Option<sync_mpsc::SyncSender<Result<()>>>,
) -> Result<()> {
    let mut closed = session.receive_closed().await?;
    let clipboard = timeout(CALL_TIMEOUT, Clipboard::with_connection(bus.clone()))
        .await
        .ok()
        .and_then(Result::ok);
    let mut transfer = None;
    let mut owner = None;
    let access = timeout(START_TIMEOUT, async {
        desktop
            .select_devices(
                session,
                SelectDevicesOptions::default()
                    .set_devices(DeviceType::Keyboard | DeviceType::Pointer),
            )
            .await?
            .response()?;
        cast.select_sources(
            session,
            SelectSourcesOptions::default()
                .set_sources(Some(SourceType::Monitor.into()))
                .set_cursor_mode(CursorMode::Metadata)
                .set_multiple(false),
        )
        .await?
        .response()?;
        let clipboard_requested = if let Some(api) = &clipboard {
            matches!(
                timeout(
                    CALL_TIMEOUT,
                    api.request(session, RequestClipboardOptions::default())
                )
                .await,
                Ok(Ok(()))
            )
        } else {
            false
        };
        if clipboard_requested {
            let api = clipboard.as_ref().unwrap();
            transfer = timeout(
                CALL_TIMEOUT,
                api.receive_selection_transfer::<RemoteDesktop>(),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .map(Box::pin);
            owner = timeout(
                CALL_TIMEOUT,
                api.receive_selection_owner_changed::<RemoteDesktop>(),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .map(Box::pin);
        }
        // Subscribe before Start: the portal may announce the initial clipboard
        // owner while consent completes or before the first video buffer arrives.
        let grant = desktop
            .start(session, None, StartOptions::default())
            .await?
            .response()?;
        ensure!(
            grant
                .devices()
                .contains(DeviceType::Keyboard | DeviceType::Pointer),
            "Portal permission does not include keyboard and pointer"
        );
        ensure!(
            grant.streams().len() == 1,
            "Portal must grant exactly one capture stream"
        );
        let source = &grant.streams()[0];
        let node = source.pipe_wire_node_id();
        // ScreenCast's size is measured in compositor coordinates; captured
        // pixel dimensions can differ, including on scaled GNOME monitors.
        let (width, height) = source
            .size()
            .filter(|(width, height)| *width > 0 && *height > 0)
            .context("Portal stream has no usable logical size for absolute pointer input")?;
        let logical_size = (width as u32, height as u32);
        let fd = cast
            .open_pipe_wire_remote(session, OpenPipeWireRemoteOptions::default())
            .await?;
        let (failed_tx, mut failure) = watch::channel(None);
        let capture_stopped = stopped.clone();
        let captured = frames.clone();
        *capture = Some(
            thread::Builder::new()
                .name("portal-pipewire".into())
                .spawn(move || {
                    if let Err(error) = capture_pipewire(
                        fd,
                        node,
                        captured.clone(),
                        cursors,
                        failed_tx.clone(),
                        capture_stopped,
                    ) {
                        let _ = failed_tx.send(Some(format!("{error:#}")));
                    }
                    captured.send_replace(None);
                })?,
        );
        let mut first = frames.subscribe();
        timeout(FIRST_FRAME_TIMEOUT, async {
            loop {
                if first.borrow().is_some() {
                    return Ok::<(), anyhow::Error>(());
                }
                tokio::select! {
                    change = first.changed() => {
                        change.context("PipeWire capture stopped")?;
                    },
                    change = failure.changed() => {
                        change.context("PipeWire status ended")?;
                        if let Some(reason) = failure.borrow().as_ref() { bail!("{reason}"); }
                    },
                }
            }
        })
        .await
        .context("Portal produced no initial video frame")??;
        ensure!(!stopped.load(Ordering::Acquire), "Portal session cancelled");
        Ok::<_, anyhow::Error>((
            failure,
            clipboard_requested && grant.is_clipboard_enabled(),
            node,
            logical_size,
        ))
    })
    .await
    .context("Portal permission/start timed out")??;
    let (failure, clipboard_enabled, node, logical_size) = access;
    if !clipboard_enabled {
        transfer = None;
        owner = None;
    }
    timeout(PROBE_TIMEOUT, identity.verify_unchanged(&bus))
        .await
        .context("Portal scope changed during authorization")??;
    if let Some(sender) = ready.take() {
        let _ = sender.send(Ok(()));
    }
    let mut clipboard_text: Option<String> = None;
    let mut clipboard_mime: Option<String> = None;
    let mut ticker = interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if stopped.load(Ordering::Acquire) { break Ok(()); }
                timeout(PROBE_TIMEOUT, identity.verify_unchanged(&bus))
                    .await.context("Portal session identity check timed out")??;
                if let Some(reason) = failure.borrow().as_ref() { break Err(anyhow::anyhow!("PipeWire capture failed: {reason}")); }
            },
            _ = closed.next() => break Err(anyhow::anyhow!("Portal session was closed")),
            command = input.recv() => match command {
                Some(command) => {
                    match timeout(CALL_TIMEOUT, apply_input(desktop, session, node, logical_size, &frames, command, held_keys, held_buttons)).await {
                        Ok(Ok(())) => {},
                        Ok(Err(error)) => break Err(error),
                        Err(error) => break Err(anyhow::anyhow!("Portal input timed out: {error}")),
                    }
                },
                None => break Ok(()),
            },
            request = control.recv() => match request {
                Some(Control::Stop) | None => break Ok(()),
                Some(Control::Read(mut reply)) => {
                    let result = while_requested(&mut reply, async {
                        if clipboard_enabled {
                            timeout(CALL_TIMEOUT, read_clipboard(clipboard.as_ref().unwrap(), session, clipboard_mime.as_deref()))
                                .await.context("Portal clipboard read timed out").and_then(|result| result)
                        } else { Err(anyhow::anyhow!("Portal clipboard permission unavailable")) }
                    }).await;
                    if let Some(result) = result { let _ = reply.send(result); }
                },
                Some(Control::Write(text, mut reply)) => {
                    let result = while_requested(&mut reply, async {
                        if clipboard_enabled {
                            timeout(CALL_TIMEOUT, clipboard.as_ref().unwrap().set_selection(session, SetSelectionOptions::default().set_mime_types(&["text/plain;charset=utf-8", "text/plain"])))
                                .await.context("Portal clipboard write timed out").and_then(|result| result.map_err(anyhow::Error::from))
                        } else { Err(anyhow::anyhow!("Portal clipboard permission unavailable")) }
                    }).await;
                    if let Some(result) = result {
                        if result.is_ok() && !reply.is_closed() { clipboard_text = Some(text); }
                        let _ = reply.send(result);
                    } else {
                        // Cancellation may happen after SetSelection was sent.
                        // Never serve the cancelled caller's text on a later transfer.
                        clipboard_text = None;
                    }
                }
            },
            event = async { transfer.as_mut().unwrap().as_mut().next().await }, if transfer.is_some() => {
                if let Some((event_session, mime, serial)) = event {
                    if serde_json::to_value(&event_session)? != serde_json::to_value(session)?
                        || !matches!(mime.as_str(), "text/plain;charset=utf-8" | "text/plain") { continue; }
                    if let (Some(api), Some(text)) = (&clipboard, &clipboard_text) {
                        let _ = write_clipboard(api, session, serial, text).await;
                    }
                } else {
                    transfer = None;
                }
            },
            event = async { owner.as_mut().unwrap().as_mut().next().await }, if owner.is_some() => {
                if let Some((event_session, change)) = event {
                    if serde_json::to_value(&event_session)? != serde_json::to_value(session)? { continue; }
                    clipboard_mime = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                        .iter().find(|wanted| change.mime_types().iter().any(|mime| mime == **wanted)).map(|s| (*s).to_owned());
                } else {
                    owner = None;
                }
            },
        }
    }
}

// Dropping a GuardedBackend operation closes this receiver. Prefer cancellation
// even if a queued request and a lock become ready in the same scheduler turn.
async fn while_requested<T>(
    reply: &mut oneshot::Sender<Result<T>>,
    request: impl std::future::Future<Output = Result<T>>,
) -> Option<Result<T>> {
    tokio::select! {
        biased;
        _ = reply.closed() => None,
        result = request => Some(result),
    }
}

#[allow(clippy::too_many_arguments)] // A single ordered input dispatch shares the session and held devices.
async fn apply_input(
    desktop: &RemoteDesktop,
    session: &Session<RemoteDesktop>,
    node: u32,
    logical_size: (u32, u32),
    frames: &watch::Sender<Option<Arc<Frame>>>,
    command: InputCommand,
    held_keys: &mut BTreeSet<u32>,
    held_buttons: &mut BTreeSet<u32>,
) -> Result<()> {
    let Some((command, guard)) = live_or_release(command) else {
        return Ok(());
    };
    if guard.as_ref().is_some_and(|guard| !guard.active()) {
        if matches!(command, InputCommand::Text { .. }) {
            guard.as_ref().unwrap().text_done.notify_one();
        }
        return Ok(());
    }
    match command {
        InputCommand::Move { x, y } => {
            ensure!(
                x.is_finite()
                    && y.is_finite()
                    && (0.0..=1.0).contains(&x)
                    && (0.0..=1.0).contains(&y),
                "invalid pointer coordinate"
            );
            ensure!(
                frames.borrow().is_some(),
                "Portal capture has no video frame"
            );
            let (absolute_x, absolute_y) = absolute_pointer(logical_size, x, y)?;
            desktop
                .notify_pointer_motion_absolute(
                    session,
                    node,
                    absolute_x,
                    absolute_y,
                    Default::default(),
                )
                .await?;
        }
        InputCommand::Button { button, pressed } => {
            let code = match button {
                0 => 0x110,
                1 => 0x112,
                2 => 0x111,
                _ => bail!("unsupported Portal pointer button"),
            };
            desktop
                .notify_pointer_button(
                    session,
                    code,
                    if pressed {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    },
                    NotifyPointerButtonOptions::default(),
                )
                .await?;
            if pressed {
                held_buttons.insert(button);
            } else {
                held_buttons.remove(&button);
            }
        }
        InputCommand::Key { code, pressed } => {
            ensure!((1..=767).contains(&code), "invalid evdev keycode");
            desktop
                .notify_keyboard_keycode(
                    session,
                    code as i32,
                    if pressed {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    },
                    NotifyKeyboardKeycodeOptions::default(),
                )
                .await?;
            if pressed {
                held_keys.insert(code);
            } else {
                held_keys.remove(&code);
            }
        }
        InputCommand::Chord { codes } => {
            release_input(desktop, session, held_keys, held_buttons).await?;
            let result = async {
                for code in codes {
                    if guard.as_ref().is_some_and(|guard| !guard.active()) {
                        break;
                    }
                    desktop
                        .notify_keyboard_keycode(
                            session,
                            code as i32,
                            KeyState::Pressed,
                            NotifyKeyboardKeycodeOptions::default(),
                        )
                        .await?;
                    held_keys.insert(code);
                }
                anyhow::Ok(())
            }
            .await;
            let released = release_input(desktop, session, held_keys, held_buttons).await;
            result?;
            released?;
        }
        InputCommand::Scroll { x, y } => {
            ensure!(x.is_finite() && y.is_finite(), "invalid Portal scroll");
            desktop
                .notify_pointer_axis(session, x, y, NotifyPointerAxisOptions::default())
                .await?;
        }
        InputCommand::Release => release_input(desktop, session, held_keys, held_buttons).await?,
        InputCommand::ReleaseSync { done } => {
            release_input(desktop, session, held_keys, held_buttons).await?;
            let _ = done.send(());
        }
        InputCommand::Text { text } => {
            ensure!(text.len() <= 16 * 1024, "text input too long");
            for symbol in text_keysyms(&text) {
                if guard.as_ref().is_some_and(|guard| !guard.active()) {
                    break;
                }
                desktop
                    .notify_keyboard_keysym(
                        session,
                        symbol as i32,
                        KeyState::Pressed,
                        NotifyKeyboardKeysymOptions::default(),
                    )
                    .await?;
                desktop
                    .notify_keyboard_keysym(
                        session,
                        symbol as i32,
                        KeyState::Released,
                        NotifyKeyboardKeysymOptions::default(),
                    )
                    .await?;
            }
            if let Some(guard) = guard {
                guard.text_done.notify_one();
            }
        }
        InputCommand::Guarded { .. } => unreachable!("guard was removed by into_active"),
    }
    Ok(())
}

fn text_keysyms(text: &str) -> Vec<u32> {
    use xkbcommon::xkb::{self, keysyms};
    let mut characters = text.chars().peekable();
    let mut symbols = Vec::new();
    while let Some(character) = characters.next() {
        let symbol = match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                keysyms::KEY_Return
            }
            '\n' => keysyms::KEY_Return,
            '\t' => keysyms::KEY_Tab,
            character if character.is_control() => continue,
            character => xkb::utf32_to_keysym(character as u32).raw(),
        };
        if symbol != keysyms::KEY_NoSymbol {
            symbols.push(symbol);
        }
    }
    symbols
}

// Cleanup remains valid after the input authorization expires while a key or
// button is held. GuardedBackend also forwards these releases unguarded.
fn live_or_release(command: InputCommand) -> Option<(InputCommand, Option<InputGuard>)> {
    match command {
        InputCommand::Guarded { command, .. }
            if matches!(
                &*command,
                InputCommand::Release | InputCommand::ReleaseSync { .. }
            ) =>
        {
            Some((*command, None))
        }
        InputCommand::Guarded { command, guard } if !guard.active() => {
            if matches!(&*command, InputCommand::Text { .. }) {
                guard.text_done.notify_one();
            }
            None
        }
        command => command.into_active(),
    }
}

/// Normalize client coordinates using ScreenCast's compositor-space stream size.
fn absolute_pointer((width, height): (u32, u32), x: f64, y: f64) -> Result<(f64, f64)> {
    ensure!(
        width > 0 && height > 0,
        "invalid Portal logical stream size"
    );
    ensure!(
        x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
        "invalid pointer coordinate"
    );
    Ok((
        x * f64::from(width.saturating_sub(1)),
        y * f64::from(height.saturating_sub(1)),
    ))
}

async fn release_input(
    desktop: &RemoteDesktop,
    session: &Session<RemoteDesktop>,
    held_keys: &mut BTreeSet<u32>,
    held_buttons: &mut BTreeSet<u32>,
) -> Result<()> {
    let mut error = None;
    for code in std::mem::take(held_keys) {
        if let Err(failure) = desktop
            .notify_keyboard_keycode(session, code as i32, KeyState::Released, Default::default())
            .await
        {
            error = Some(failure);
        }
    }
    for button in std::mem::take(held_buttons) {
        let code = match button {
            0 => 0x110,
            1 => 0x112,
            2 => 0x111,
            _ => continue,
        };
        if let Err(failure) = desktop
            .notify_pointer_button(session, code, KeyState::Released, Default::default())
            .await
        {
            error = Some(failure);
        }
    }
    if let Some(error) = error {
        Err(error.into())
    } else {
        Ok(())
    }
}

async fn read_clipboard(
    clipboard: &Clipboard,
    session: &Session<RemoteDesktop>,
    mime: Option<&str>,
) -> Result<String> {
    let mime = mime.context("Portal clipboard does not advertise UTF-8 text")?;
    let fd: OwnedFd = clipboard.selection_read(session, mime).await?.into();
    let bytes = timeout(
        Duration::from_secs(2),
        crate::clipboard::read_fd(fd, MAX_CLIPBOARD_BYTES),
    )
    .await??;
    String::from_utf8(bytes).context("Portal clipboard is not UTF-8")
}

async fn write_clipboard(
    clipboard: &Clipboard,
    session: &Session<RemoteDesktop>,
    serial: u32,
    text: &str,
) -> Result<()> {
    let result = timeout(CALL_TIMEOUT, async {
        let fd: OwnedFd = clipboard.selection_write(session, serial).await?.into();
        timeout(
            Duration::from_secs(2),
            crate::clipboard::write_fd(fd, text.as_bytes()),
        )
        .await??;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("Portal clipboard transfer timed out")
    .and_then(|result| result);
    let _ = timeout(
        CALL_TIMEOUT,
        clipboard.selection_write_done(session, serial, result.is_ok()),
    )
    .await;
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PixelFormat {
    Rgba,
    Rgbx,
    Bgra,
    Bgrx,
}
impl PixelFormat {
    fn from_spa(format: VideoFormat) -> Option<Self> {
        if format == VideoFormat::RGBA {
            Some(Self::Rgba)
        } else if format == VideoFormat::RGBx {
            Some(Self::Rgbx)
        } else if format == VideoFormat::BGRA {
            Some(Self::Bgra)
        } else if format == VideoFormat::BGRx {
            Some(Self::Bgrx)
        } else {
            None
        }
    }
}

/// Keep two reusable slots: the published frame may still be held by a viewer.
/// Arc copy-on-write protects any slower consumer before pixels are overwritten.
#[derive(Default)]
struct FrameConverter {
    buffers: [Arc<[u8]>; 2],
    next: usize,
}
impl FrameConverter {
    fn convert(
        &mut self,
        bytes: &[u8],
        region: (usize, usize, usize),
        dimensions: (u32, u32),
        format: PixelFormat,
    ) -> Result<Frame> {
        let (offset, size, stride) = region;
        let (width, height) = dimensions;
        let row = usize::try_from(width)?
            .checked_mul(4)
            .context("video row overflows")?;
        let length = row
            .checked_mul(usize::try_from(height)?)
            .context("video size overflows")?;
        ensure!(
            width > 0 && height > 0 && length <= MAX_FRAME_BYTES,
            "invalid Portal frame dimensions"
        );
        ensure!(
            stride >= row && stride <= MAX_FRAME_BYTES,
            "invalid Portal video stride"
        );
        let needed = stride
            .checked_mul(height as usize - 1)
            .and_then(|n| n.checked_add(row))
            .context("video stride overflows")?;
        ensure!(
            needed <= size
                && offset
                    .checked_add(size)
                    .is_some_and(|end| end <= bytes.len()),
            "truncated Portal video frame"
        );
        let storage = &mut self.buffers[self.next];
        if storage.len() != length {
            *storage = vec![0; length].into();
        }
        let pixels = Arc::make_mut(storage);
        for (row_index, destination) in pixels.chunks_exact_mut(row).enumerate() {
            let start = offset + row_index * stride;
            let source = &bytes[start..start + row];
            destination.copy_from_slice(&source[..row]);
            for pixel in destination.as_chunks_mut::<4>().0 {
                match format {
                    PixelFormat::Rgba => {}
                    PixelFormat::Rgbx => pixel[3] = 255,
                    PixelFormat::Bgra => pixel.swap(0, 2),
                    PixelFormat::Bgrx => {
                        pixel.swap(0, 2);
                        pixel[3] = 255;
                    }
                }
            }
        }
        let frame = Frame::new(width, height, storage.clone());
        self.next = (self.next + 1) % self.buffers.len();
        Ok(frame)
    }
}

#[cfg(test)]
fn rgba_frame(
    bytes: &[u8],
    offset: usize,
    size: usize,
    stride: usize,
    width: u32,
    height: u32,
    format: PixelFormat,
) -> Result<Frame> {
    FrameConverter::default().convert(bytes, (offset, size, stride), (width, height), format)
}

/// Return old PipeWire buffers before spending CPU converting the newest one.
fn newest_buffer<T>(mut dequeue: impl FnMut() -> Option<T>) -> Option<T> {
    let mut newest = dequeue()?;
    while let Some(next) = dequeue() {
        newest = next;
    }
    Some(newest)
}

pub(crate) fn capture_pipewire(
    fd: OwnedFd,
    node: u32,
    frames: watch::Sender<Option<Arc<Frame>>>,
    cursors: watch::Sender<Option<Arc<CursorFrame>>>,
    failed: watch::Sender<Option<String>>,
    stopped: Arc<AtomicBool>,
) -> Result<()> {
    pw::init();
    let loop_ = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&loop_, None)?;
    let core = context.connect_fd_rc(fd, None)?;
    let core_failed = failed.clone();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _seq, result, message| {
            core_failed.send_replace(Some(format!(
                "PipeWire core {id} error {result}: {message}"
            )));
        })
        .register();
    let stream = pw::stream::StreamRc::new(
        core.clone(),
        "lightos-portal",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;
    let state = Arc::new(Mutex::new(VideoInfoRaw::default()));
    let video_format = state.clone();
    let video_frames = frames.clone();
    let video_failed = failed.clone();
    let metadata_failed = failed.clone();
    let mut converter = FrameConverter::default();
    let _listener = stream
        .add_local_listener::<()>()
        .state_changed(move |_, _, _, new| {
            tracing::info!(?new, "PipeWire capture state");
            if let pw::stream::StreamState::Error(reason) = new {
                let _ = video_failed.send(Some(reason));
            }
        })
        .param_changed(move |stream, _, id, param| {
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else {
                return;
            };
            if format_utils::parse_format(param).ok() != Some((MediaType::Video, MediaSubtype::Raw))
            {
                return;
            }
            let mut info = VideoInfoRaw::default();
            if info.parse(param).is_ok() {
                *video_format.lock().unwrap() = info;
                let bytes = cursor::parameters();
                if let Ok(bytes) = bytes {
                    let mut params: Vec<_> = bytes
                        .iter()
                        .filter_map(|value| Pod::from_bytes(value))
                        .collect();
                    if let Err(error) = stream.update_params(&mut params) {
                        metadata_failed.send_replace(Some(format!(
                            "PipeWire cursor metadata negotiation failed: {error}"
                        )));
                    }
                }
            }
        })
        .process(move |stream, _| {
            let format = *state.lock().unwrap();
            // Shape metadata may occur only in a buffer superseded by a newer
            // video frame. Consume every cursor update before returning buffers.
            let Some(mut buffer) = newest_buffer(|| {
                loop {
                    let mut buffer = buffer::Buffer::dequeue(stream)?;
                    if let Some(metadata) = buffer.cursor() {
                        let previous = cursors.borrow().clone();
                        if let Ok(Some(cursor)) =
                            cursor::decode(metadata, format.size(), previous.as_deref())
                        {
                            cursors.send_replace(Some(Arc::new(cursor)));
                        }
                    }
                    // A cursor-only packet can follow the newest video packet.
                    // Consume its metadata without replacing the valid video with
                    // empty/corrupt pixels (KWin uses CORRUPTED for cursor-only).
                    if buffer.datas_mut().first().is_some_and(|data| {
                        data.chunk().size() > 0
                            && !data
                                .chunk()
                                .flags()
                                .contains(spa::buffer::ChunkFlags::CORRUPTED)
                    }) {
                        return Some(buffer);
                    }
                }
            }) else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            if data
                .chunk()
                .flags()
                .contains(spa::buffer::ChunkFlags::CORRUPTED)
            {
                return;
            }
            let format = *state.lock().unwrap();
            let Some(pixel_format) = PixelFormat::from_spa(format.format()) else {
                return;
            };
            let size = format.size();
            let (offset, length, stride) = (
                data.chunk().offset() as usize,
                data.chunk().size() as usize,
                data.chunk().stride(),
            );
            let Some(bytes) = data.data() else {
                return;
            };
            let Ok(stride) = usize::try_from(stride) else {
                return;
            };
            if let Ok(frame) = converter.convert(
                bytes,
                (offset, length, stride),
                (size.width, size.height),
                pixel_format,
            ) {
                video_frames.send_if_modified(|current| {
                    let changed = current.as_ref().is_none_or(|old| {
                        old.width != frame.width
                            || old.height != frame.height
                            || old.pixels != frame.pixels
                    });
                    *current = Some(Arc::new(frame));
                    changed
                });
            }
        })
        .register()?;
    let parameter = pw::spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRA,
            VideoFormat::BGRA,
            VideoFormat::BGRx,
            VideoFormat::RGBA,
            VideoFormat::RGBx
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 8192,
                height: 8192
            }
        ),
    );
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(parameter),
    )?
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).context("invalid Portal PipeWire video format")?];
    stream.connect(
        spa::utils::Direction::Input,
        Some(node),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    while !stopped.load(Ordering::Acquire) && failed.borrow().is_none() {
        if loop_
            .loop_()
            .iterate(pw::loop_::Timeout::Finite(Duration::from_millis(100)))
            < 0
        {
            bail!("Portal PipeWire event loop stopped");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_text_uses_canonical_keysyms_and_normalizes_line_endings() {
        assert_eq!(
            text_keysyms("Aé中🦀\r\nB\nC\r\t"),
            [
                0x41,
                0xe9,
                0x0100_4e2d,
                0x0101_f980,
                0xff0d,
                0x42,
                0xff0d,
                0x43,
                0xff0d,
                0xff09
            ]
        );
        assert_eq!(text_keysyms("\r\r\n\n"), [0xff0d, 0xff0d, 0xff0d]);
        assert!(text_keysyms("").is_empty());
    }

    #[test]
    fn portal_text_does_not_turn_other_control_characters_into_keys() {
        assert_eq!(text_keysyms("a\0\u{8}\u{1b}\u{7f}\u{85}b"), [0x61, 0x62]);
    }

    #[tokio::test]
    async fn cancelled_clipboard_request_skips_queued_work_and_drops_inflight_work() {
        let (mut reply, response) = oneshot::channel::<Result<()>>();
        drop(response);
        let mut changed = false;
        assert!(
            while_requested(&mut reply, async {
                changed = true;
                Ok(())
            })
            .await
            .is_none()
        );
        assert!(!changed, "cancelled queued request acquired the clipboard");

        let (mut reply, response) = oneshot::channel::<Result<()>>();
        let (started, start) = oneshot::channel();
        let (alive, dropped) = oneshot::channel::<()>();
        let work = tokio::spawn(async move {
            while_requested(&mut reply, async move {
                let _alive = alive;
                let _ = started.send(());
                std::future::pending::<Result<()>>().await
            })
            .await
        });
        timeout(Duration::from_secs(1), start)
            .await
            .unwrap()
            .unwrap();
        drop(response);
        assert!(
            timeout(Duration::from_secs(1), work)
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert!(
            dropped.await.is_err(),
            "in-flight clipboard operation survived cancellation"
        );
    }

    #[test]
    fn buffer_validates_stride_bounds_and_swaps_bgra() {
        // The last row uses only four bytes; PipeWire does not have to pad it
        // to an entire stride before declaring the chunk complete.
        let data = [99, 1, 2, 3, 4, 88, 88, 88, 88, 10, 20, 30, 40];
        let frame = rgba_frame(&data, 1, 12, 8, 1, 2, PixelFormat::Bgra).unwrap();
        assert_eq!(&*frame.pixels, &[3, 2, 1, 4, 30, 20, 10, 40]);
        assert!(rgba_frame(&data, 1, 9, 8, 1, 2, PixelFormat::Bgra).is_err());
        assert!(rgba_frame(&data, 1, 12, 3, 1, 2, PixelFormat::Bgra).is_err());
        assert!(rgba_frame(&data, 2, 12, 8, 1, 2, PixelFormat::Bgra).is_err());
        assert!(rgba_frame(&data, 1, 12, 8, 0, 2, PixelFormat::Bgra).is_err());
    }

    #[test]
    fn packed_video_converts_all_supported_channel_orders() {
        let pixel = [1, 2, 3, 4];
        for (format, expected) in [
            (PixelFormat::Rgba, [1, 2, 3, 4]),
            (PixelFormat::Rgbx, [1, 2, 3, 255]),
            (PixelFormat::Bgra, [3, 2, 1, 4]),
            (PixelFormat::Bgrx, [3, 2, 1, 255]),
        ] {
            let frame = rgba_frame(&pixel, 0, pixel.len(), 4, 1, 1, format).unwrap();
            assert_eq!(&*frame.pixels, &expected);
        }
    }

    #[test]
    fn absolute_pointer_uses_portal_logical_size() {
        let logical_size = (1280, 720);
        assert_eq!(
            absolute_pointer(logical_size, 0.0, 0.0).unwrap(),
            (0.0, 0.0)
        );
        assert_eq!(
            absolute_pointer(logical_size, 1.0, 1.0).unwrap(),
            (1279.0, 719.0)
        );
        assert_eq!(
            absolute_pointer(logical_size, 0.5, 0.5).unwrap(),
            (639.5, 359.5)
        );
        assert!(absolute_pointer((0, 720), 0.5, 0.5).is_err());
        assert!(absolute_pointer(logical_size, f64::NAN, 0.0).is_err());
        assert!(absolute_pointer(logical_size, -0.01, 0.0).is_err());
    }

    #[tokio::test]
    async fn cancellation_noticed_from_either_signal() {
        let cancelled_flag = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        assert!(
            timeout(
                Duration::from_millis(60),
                cancelled(cancelled_flag.clone(), stopped.clone())
            )
            .await
            .is_err()
        );
        stopped.store(true, Ordering::Release);
        timeout(
            Duration::from_millis(60),
            cancelled(cancelled_flag.clone(), stopped.clone()),
        )
        .await
        .unwrap();
        stopped.store(false, Ordering::Release);
        cancelled_flag.store(true, Ordering::Release);
        timeout(
            Duration::from_millis(60),
            cancelled(cancelled_flag, stopped),
        )
        .await
        .unwrap();
    }

    #[test]
    fn expired_authorization_still_allows_physical_releases() {
        let guard = InputGuard::new(
            tokio_util::sync::CancellationToken::new(),
            std::time::Instant::now() - Duration::from_secs(1),
        );
        let input = |command| InputCommand::Guarded {
            command: Box::new(command),
            guard: guard.clone(),
        };
        assert!(
            live_or_release(input(InputCommand::Key {
                code: 30,
                pressed: true
            }))
            .is_none()
        );
        assert!(matches!(
            live_or_release(input(InputCommand::Release)),
            Some((InputCommand::Release, None))
        ));
        let (done, _receiver) = sync_mpsc::sync_channel(1);
        assert!(matches!(
            live_or_release(input(InputCommand::ReleaseSync { done })),
            Some((InputCommand::ReleaseSync { .. }, None))
        ));
    }

    #[tokio::test]
    async fn expired_text_notifies_the_waiting_caller() {
        let guard = InputGuard::new(
            tokio_util::sync::CancellationToken::new(),
            std::time::Instant::now() - Duration::from_secs(1),
        );
        let completed = guard.text_done.clone();
        assert!(
            live_or_release(InputCommand::Guarded {
                command: Box::new(InputCommand::Text { text: "ab".into() }),
                guard,
            })
            .is_none()
        );
        timeout(Duration::from_millis(60), completed.notified())
            .await
            .unwrap();
    }

    #[test]
    fn cancellation_joins_capture_and_clears_a_racing_final_frame() {
        let stopped = Arc::new(AtomicBool::new(false));
        let (frames, current) = watch::channel(Some(Arc::new(Frame::test_frame(1, 1))));
        let (worker_stopped, worker_frames) = (stopped.clone(), frames.clone());
        let mut capture = Some(thread::spawn(move || {
            while !worker_stopped.load(Ordering::Acquire) {
                thread::park_timeout(Duration::from_millis(1));
            }
            worker_frames.send_replace(Some(Arc::new(Frame::test_frame(1, 1))));
        }));
        stop_capture_worker(&stopped, &mut capture, &frames);
        assert!(stopped.load(Ordering::Acquire));
        assert!(capture.is_none());
        assert!(current.borrow().is_none());
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    fn conversion_reuses_free_slots_and_keeps_retained_pixels_immutable() {
        let mut converter = FrameConverter::default();
        let a = converter
            .convert(&[1, 2, 3, 255], (0, 4, 4), (1, 1), PixelFormat::Rgba)
            .unwrap();
        let address = a.pixels.as_ptr();
        drop(a);
        let retained = converter
            .convert(&[4, 5, 6, 255], (0, 4, 4), (1, 1), PixelFormat::Rgba)
            .unwrap();
        let reused = converter
            .convert(&[7, 8, 9, 255], (0, 4, 4), (1, 1), PixelFormat::Rgba)
            .unwrap();
        assert_eq!(address, reused.pixels.as_ptr());
        let _next = converter
            .convert(&[10, 11, 12, 255], (0, 4, 4), (1, 1), PixelFormat::Rgba)
            .unwrap();
        assert_eq!(&*retained.pixels, &[4, 5, 6, 255]);
        assert_eq!(&*reused.pixels, &[7, 8, 9, 255]);
    }
    #[test]
    fn backlog_is_discarded_before_conversion_and_all_buffers_are_returned() {
        use std::{cell::RefCell, rc::Rc};
        struct Buffer(usize, Rc<RefCell<Vec<usize>>>);
        impl Drop for Buffer {
            fn drop(&mut self) {
                self.1.borrow_mut().push(self.0);
            }
        }
        let returned = Rc::new(RefCell::new(Vec::new()));
        let mut buffers = (0..5).map(|n| Buffer(n, returned.clone()));
        let newest = newest_buffer(|| buffers.next()).unwrap();
        assert_eq!(newest.0, 4);
        assert_eq!(&*returned.borrow(), &[0, 1, 2, 3]);
        drop(newest);
        assert_eq!(&*returned.borrow(), &[0, 1, 2, 3, 4]);
        assert!(newest_buffer::<u8>(|| None).is_none());
    }
}
