//! Native KWin capture for explicitly admitted service-owned sessions. Pixels
//! never contain the pointer; PipeWire cursor metadata uses the normal channel.
use super::*;
use anyhow::Context;
use display_control::SessionTarget;
use std::{
    collections::{BTreeSet, HashMap},
    os::{
        fd::{AsFd, AsRawFd},
        unix::net::UnixStream,
    },
    sync::Mutex,
    thread::JoinHandle,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};
use wayland_protocols_plasma::{
    fake_input::client::org_kde_kwin_fake_input::OrgKdeKwinFakeInput,
    plasma_window_management::client::{
        org_kde_plasma_window::{self, OrgKdePlasmaWindow},
        org_kde_plasma_window_management::{self, OrgKdePlasmaWindowManagement},
    },
    screencast::v1::client::{
        zkde_screencast_stream_unstable_v1::{self, ZkdeScreencastStreamUnstableV1},
        zkde_screencast_unstable_v1::ZkdeScreencastUnstableV1,
    },
};

pub(crate) struct Backend {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    cursors: watch::Receiver<Option<Arc<CursorFrame>>>,
    input: mpsc::Sender<InputCommand>,
    stopped: Arc<AtomicBool>,
    focus: Arc<Mutex<String>>,
    clipboard: Arc<super::wayland_clipboard::Clipboard>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Backend {
    pub(crate) fn start(
        target: SessionTarget,
        peer: u32,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let _pid = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), compositor_pid(&target, peer)).await
        })??;
        drop(runtime);
        let clipboard = Arc::new(super::wayland_clipboard::Clipboard::start(
            target
                .wayland_socket
                .as_deref()
                .context("KDE socket missing")?,
        )?);
        let socket = UnixStream::connect(
            target
                .wayland_socket
                .as_deref()
                .context("KDE socket missing")?,
        )?;
        let interrupt = socket.try_clone()?;
        let worker_clipboard = clipboard.clone();
        let (frames_tx, frames) = watch::channel(None);
        let (cursors_tx, cursors) = watch::channel(None);
        let (input, commands) = mpsc::channel(128);
        let stopped = Arc::new(AtomicBool::new(false));
        let focus = Arc::new(Mutex::new(String::new()));
        let stop = stopped.clone();
        let active = focus.clone();
        let (ready, initialized) = sync_mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("owned-kwin".into())
            .spawn(move || {
                let mut ready = Some(ready);
                let result = run(
                    target,
                    socket,
                    worker_clipboard,
                    frames_tx.clone(),
                    cursors_tx,
                    commands,
                    stop.clone(),
                    cancelled,
                    active,
                    &mut ready,
                );
                stop.store(true, Ordering::Release);
                frames_tx.send_replace(None);
                if let Some(ready) = ready {
                    let _ = ready.send(result.map_err(|e| format!("{e:#}")));
                } else if let Err(error) = result {
                    tracing::warn!(%error, "owned KDE session stopped");
                }
            })?;
        match initialized.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(())) => Ok(Self {
                frames,
                cursors,
                input,
                stopped,
                focus,
                clipboard,
                worker: Mutex::new(Some(worker)),
            }),
            result => {
                stopped.store(true, Ordering::Release);
                let _ = interrupt.shutdown(std::net::Shutdown::Both);
                let _ = worker.join();
                bail!("owned KDE did not start: {result:?}");
            }
        }
    }
}
impl DesktopBackend for Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::Portal
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn cursor_frames(&self) -> Option<watch::Receiver<Option<Arc<CursorFrame>>>> {
        Some(self.cursors.clone())
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn frame_is_fresh(&self, _: Duration) -> bool {
        !self.is_stopped() && self.frames.borrow().is_some()
    }
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.clipboard.stopped()
    }
    fn clipboard_text_input(&self) -> bool {
        true
    }
    fn text_paste_keycodes(&self, shift: bool) -> BoxFuture<'_, Vec<u32>> {
        Box::pin(async move {
            let app = self.focus.lock().unwrap_or_else(|e| e.into_inner()).clone();
            super::linux_text_paste_keys(shift, super::terminal_app(&app))
        })
    }
    fn supports_image_clipboard(&self) -> bool {
        !self.clipboard.stopped()
    }
    fn supports_clipboard_sync(&self) -> bool {
        !self.clipboard.stopped()
    }
    fn clipboard_revision(&self) -> BoxFuture<'_, Result<ClipboardRevision>> {
        Box::pin(self.clipboard.revision())
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async {
            String::from_utf8(
                self.clipboard
                    .read(false)
                    .await?
                    .context("clipboard has no text")?,
            )
            .context("clipboard is not UTF-8")
        })
    }
    fn clipboard_read_png(&self) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        Box::pin(self.clipboard.read(true))
    }
    fn clipboard_write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.clipboard.write(false, text.as_bytes()))
    }
    fn clipboard_write_png<'a>(&'a self, image: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.clipboard.write(true, image))
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
        self.clipboard.stop();
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

// The socket can be inherited from kwin_wayland_wrapper. Accept only the
// actual KWin process belonging to that admitted parent, on its private bus.
async fn compositor_pid(target: &SessionTarget, peer: u32) -> Result<u32> {
    let bus = zbus::connection::Builder::address(
        target.bus_address.as_deref().context("KDE bus missing")?,
    )?
    .build()
    .await?;
    let api = zbus::fdo::DBusProxy::new(&bus).await?;
    let owner = api.get_name_owner("org.kde.KWin".try_into()?).await?;
    let pid = api
        .get_connection_unix_process_id(owner.as_str().try_into()?)
        .await?;
    let path = format!("/proc/{pid}");
    ensure!(
        std::fs::read_link(format!("{path}/exe"))?
            .file_name()
            .is_some_and(|name| name == "kwin_wayland"),
        "KDE bus owner is not KWin"
    );
    let status = std::fs::read_to_string(format!("{path}/status"))?;
    let parent: u32 = status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))
        .context("KWin parent missing")?
        .trim()
        .parse()?;
    ensure!(
        pid == peer || parent == peer,
        "KWin is not the admitted compositor child"
    );
    ensure!(
        std::fs::read_to_string(format!("{path}/cgroup"))?
            == std::fs::read_to_string(format!("/proc/{peer}/cgroup"))?,
        "KWin left its owned service"
    );
    Ok(pid)
}

#[derive(Default)]
struct Output {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: i32,
    rotated: bool,
}
#[derive(Default)]
struct Window {
    app: String,
    active: bool,
}
struct State {
    outputs: HashMap<u32, Output>,
    node: Option<u32>,
    error: Option<String>,
    windows: HashMap<u32, Window>,
    focus: Arc<Mutex<String>>,
}
impl State {
    fn update_focus(&self) {
        *self.focus.lock().unwrap_or_else(|e| e.into_inner()) = self
            .windows
            .values()
            .find(|w| w.active)
            .map(|w| w.app.clone())
            .unwrap_or_default();
    }
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(State: ignore OrgKdeKwinFakeInput);
delegate_noop!(State: ignore ZkdeScreencastUnstableV1);
impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let info = state.outputs.entry(output.id().protocol_id()).or_default();
        match event {
            wl_output::Event::Geometry {
                x, y, transform, ..
            } => {
                info.x = x;
                info.y = y;
                info.rotated = matches!(
                    transform,
                    WEnum::Value(
                        wl_output::Transform::_90
                            | wl_output::Transform::_270
                            | wl_output::Transform::Flipped90
                            | wl_output::Transform::Flipped270
                    )
                );
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                info.width = width;
                info.height = height;
            }
            wl_output::Event::Scale { factor } => info.scale = factor,
            _ => {}
        }
    }
}
impl Dispatch<ZkdeScreencastStreamUnstableV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZkdeScreencastStreamUnstableV1,
        event: zkde_screencast_stream_unstable_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zkde_screencast_stream_unstable_v1::Event::Created { node } => state.node = Some(node),
            zkde_screencast_stream_unstable_v1::Event::Closed => {
                state.error = Some("KDE closed the capture stream".into())
            }
            zkde_screencast_stream_unstable_v1::Event::Failed { error } => {
                state.error = Some(error)
            }
            _ => {}
        }
    }
}
impl Dispatch<OrgKdePlasmaWindowManagement, ()> for State {
    fn event(
        state: &mut Self,
        manager: &OrgKdePlasmaWindowManagement,
        event: org_kde_plasma_window_management::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let org_kde_plasma_window_management::Event::WindowWithUuid { uuid, .. } = event {
            let window = manager.get_window_by_uuid(uuid, handle, ());
            state.windows.entry(window.id().protocol_id()).or_default();
        }
    }
}
impl Dispatch<OrgKdePlasmaWindow, ()> for State {
    fn event(
        state: &mut Self,
        window: &OrgKdePlasmaWindow,
        event: org_kde_plasma_window::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = window.id().protocol_id();
        let info = state.windows.entry(id).or_default();
        match event {
            org_kde_plasma_window::Event::AppIdChanged { app_id } => {
                info.app = app_id.chars().take(256).collect()
            }
            org_kde_plasma_window::Event::StateChanged { flags } => info.active = flags & 1 != 0,
            org_kde_plasma_window::Event::Unmapped => {
                state.windows.remove(&id);
                window.destroy();
            }
            _ => {}
        }
        state.update_focus();
    }
}

fn dispatch(
    connection: &Connection,
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
) -> Result<()> {
    queue.dispatch_pending(state)?;
    connection.flush()?;
    if let Some(read) = queue.prepare_read() {
        let mut poll = libc::pollfd {
            fd: connection.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll, 1, 2) };
        if result > 0 {
            read.read()?;
        } else if result < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    queue.dispatch_pending(state)?;
    if let Some(error) = state.error.take() {
        bail!("{error}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run(
    target: SessionTarget,
    socket: UnixStream,
    clipboard: Arc<super::wayland_clipboard::Clipboard>,
    frames: watch::Sender<Option<Arc<Frame>>>,
    cursors: watch::Sender<Option<Arc<CursorFrame>>>,
    mut input: mpsc::Receiver<InputCommand>,
    stopped: Arc<AtomicBool>,
    requested: Arc<AtomicBool>,
    focus: Arc<Mutex<String>>,
    ready: &mut Option<sync_mpsc::SyncSender<std::result::Result<(), String>>>,
) -> Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.set_write_timeout(Some(Duration::from_secs(3)))?;
    let connection = Connection::from_socket(socket)?;
    let io = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let handle = queue.handle();
    let device: OrgKdeKwinFakeInput = globals
        .bind(&handle, 4..=5, ())
        .context("KDE input protocol unavailable")?;
    device.authenticate(
        "LightOS Remote Desktop".into(),
        "Control this service-owned desktop".into(),
    );
    let cast: ZkdeScreencastUnstableV1 = globals
        .bind(&handle, 1..=4, ())
        .context("KDE screencast protocol unavailable")?;
    let output: wl_output::WlOutput = globals.bind(&handle, 2..=4, ())?;
    let _windows: OrgKdePlasmaWindowManagement = globals
        .bind(&handle, 13..=17, ())
        .context("KDE window metadata protocol unavailable")?;
    let mut state = State {
        outputs: HashMap::new(),
        node: None,
        error: None,
        windows: HashMap::new(),
        focus,
    };
    let stream = cast.stream_output(&output, 4, &handle, ());
    let deadline = Instant::now() + Duration::from_secs(12);
    let (failed, failure) = watch::channel(None);
    let mut capture = None;
    let mut keys = BTreeSet::new();
    let mut buttons = BTreeSet::new();
    let result = (|| -> Result<()> {
        while !stopped.load(Ordering::Acquire) && !requested.load(Ordering::Acquire) {
            dispatch(&connection, &mut queue, &mut state)?;
            if let Some(error) = failure.borrow().as_ref() {
                bail!("{error}");
            }
            if capture.is_none()
                && let Some(node) = state.node
            {
                let pipewire = UnixStream::connect(target.runtime.join("pipewire-0"))?;
                pipewire.set_nonblocking(true)?;
                let captured = frames.clone();
                let cursors = cursors.clone();
                let stopped = stopped.clone();
                let failed = failed.clone();
                capture = Some(
                    std::thread::Builder::new()
                        .name("kwin-pipewire".into())
                        .spawn(move || {
                            if let Err(error) = super::portal::capture_pipewire(
                                pipewire.into(),
                                node,
                                captured,
                                cursors,
                                failed.clone(),
                                stopped,
                            ) {
                                failed.send_replace(Some(format!("{error:#}")));
                            }
                        })?,
                );
            }
            if ready.is_some() {
                ensure!(Instant::now() < deadline, "KDE capture startup timed out");
                if frames.borrow().is_some()
                    && let Some(ready) = ready.take()
                {
                    let _ = ready.send(Ok(()));
                }
            }
            for _ in 0..128 {
                let Ok(command) = input.try_recv() else {
                    break;
                };
                if command.validate().is_err() {
                    continue;
                }
                let Some((command, guard)) = command.into_active() else {
                    continue;
                };
                match command {
                    InputCommand::Move { x, y } => {
                        let o = state
                            .outputs
                            .get(&output.id().protocol_id())
                            .context("KDE output geometry missing")?;
                        let (width, height) = if o.rotated {
                            (o.height, o.width)
                        } else {
                            (o.width, o.height)
                        };
                        let scale = f64::from(o.scale.max(1));
                        device.pointer_motion_absolute(
                            f64::from(o.x) + x * (f64::from(width) / scale - 1.0).max(0.0),
                            f64::from(o.y) + y * (f64::from(height) / scale - 1.0).max(0.0),
                        );
                    }
                    InputCommand::Key { code, pressed } => {
                        device.keyboard_key(code, u32::from(pressed));
                        if pressed {
                            keys.insert(code);
                        } else {
                            keys.remove(&code);
                        }
                    }
                    InputCommand::Button { button, pressed } => {
                        let code = [272, 274, 273][button as usize];
                        device.button(code, u32::from(pressed));
                        if pressed {
                            buttons.insert(code);
                        } else {
                            buttons.remove(&code);
                        }
                    }
                    InputCommand::Scroll { x, y } => {
                        if y != 0.0 {
                            device.axis(0, y);
                        }
                        if x != 0.0 {
                            device.axis(1, x);
                        }
                    }
                    InputCommand::Release => release(&device, &mut keys, &mut buttons),
                    InputCommand::ReleaseSync { done } => {
                        release(&device, &mut keys, &mut buttons);
                        connection.flush()?;
                        let _ = done.send(());
                    }
                    InputCommand::Chord { codes } => {
                        release(&device, &mut keys, &mut buttons);
                        for code in &codes {
                            if guard.as_ref().is_some_and(|g| !g.active()) {
                                break;
                            }
                            device.keyboard_key(*code, 1);
                            keys.insert(*code);
                        }
                        release(&device, &mut keys, &mut buttons);
                    }
                    InputCommand::Text { text } => {
                        release(&device, &mut keys, &mut buttons);
                        io.block_on(clipboard.write(false, text.as_bytes()))?;
                        if guard.as_ref().is_none_or(|guard| guard.active()) {
                            let app = state
                                .focus
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .clone();
                            for code in
                                super::linux_text_paste_keys(false, super::terminal_app(&app))
                            {
                                device.keyboard_key(code, 1);
                                keys.insert(code);
                            }
                            release(&device, &mut keys, &mut buttons);
                        }
                    }
                    InputCommand::Guarded { .. } => unreachable!(),
                }
                if let Some(guard) = guard {
                    guard.text_done.notify_one();
                }
            }
        }
        Ok(())
    })();
    release(&device, &mut keys, &mut buttons);
    stream.close();
    cast.destroy();
    if device.version() >= 5 {
        device.destroy();
    }
    let _ = connection.flush();
    stopped.store(true, Ordering::Release);
    if let Some(capture) = capture {
        let _ = capture.join();
    }
    result
}
fn release(device: &OrgKdeKwinFakeInput, keys: &mut BTreeSet<u32>, buttons: &mut BTreeSet<u32>) {
    for code in std::mem::take(keys).into_iter().rev() {
        device.keyboard_key(code, 0);
    }
    for button in std::mem::take(buttons) {
        device.button(button, 0);
    }
}
