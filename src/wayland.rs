use std::{
    collections::BTreeSet,
    io::{Seek, SeekFrom, Write},
    net::Shutdown,
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc as sync_mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use libwayshot::WayshotConnection;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_pointer, wl_registry, wl_seat},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xkbcommon::xkb;

const START_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_EVDEV_CODE: u32 = 767;

struct CaptureTarget {
    socket: PathBuf,
    output: Option<String>,
}

#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    jpeg: OnceLock<(u8, Arc<[u8]>)>,
    pub pixels: Arc<[u8]>,
    pub captured_at: Instant,
}

impl Frame {
    pub fn jpeg(&self, quality: u8) -> Result<Arc<[u8]>> {
        ensure!(
            self.pixels.len() == self.width as usize * self.height as usize * 4,
            "invalid JPEG pixels"
        );
        if let Some((cached_quality, data)) = self.jpeg.get()
            && *cached_quality == quality
        {
            return Ok(data.clone());
        }
        let data = media_codec::encode_jpeg(self.width, self.height, &self.pixels, quality)?;
        let _ = self.jpeg.set((quality, data.clone()));
        Ok(data)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputCommand {
    Move {
        x: f64,
        y: f64,
    },
    Button {
        button: u32,
        pressed: bool,
    },
    Key {
        code: u32,
        pressed: bool,
    },
    Scroll {
        x: f64,
        y: f64,
    },
    Release,
    #[serde(skip)]
    ReleaseSync {
        done: sync_mpsc::SyncSender<()>,
    },
    Text {
        text: String,
    },
}

impl InputCommand {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Move { x, y } => ensure!(
                x.is_finite()
                    && y.is_finite()
                    && (0.0..=1.0).contains(x)
                    && (0.0..=1.0).contains(y),
                "pointer coordinates must be finite and normalized to 0..1"
            ),
            Self::Button { button, .. } => {
                ensure!(*button <= 2, "unsupported mouse button");
            }
            Self::Key { code, .. } => {
                ensure!((1..=MAX_EVDEV_CODE).contains(code), "invalid evdev keycode");
            }
            Self::Scroll { x, y } => ensure!(
                x.is_finite() && y.is_finite() && x.abs() <= 1000.0 && y.abs() <= 1000.0,
                "scroll coordinates must be finite and within -1000..1000"
            ),
            Self::Text { text } => {
                ensure!(text.len() <= MAX_TEXT_BYTES, "text exceeds 16 KiB");
                ensure!(
                    text.chars().all(|character| !character.is_control()
                        || matches!(character, '\n' | '\r' | '\t')),
                    "unsupported text control character"
                );
            }
            Self::Release | Self::ReleaseSync { .. } => {}
        }
        Ok(())
    }
}

fn validated_input(command: InputCommand) -> Option<InputCommand> {
    if let Err(error) = command.validate() {
        tracing::warn!(%error, "ignoring invalid client input");
        return None;
    }
    Some(command)
}

struct Shared {
    stopped: AtomicBool,
    frames: watch::Sender<Option<Arc<Frame>>>,
    capture_socket: Mutex<Option<UnixStream>>,
}

impl Shared {
    fn finish(&self) {
        self.stopped.store(true, Ordering::Release);
        self.frames.send_replace(None);
    }
}

pub struct Backend {
    pub frames: watch::Receiver<Option<Arc<Frame>>>,
    pub input: mpsc::Sender<InputCommand>,
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    input_socket: UnixStream,
    input_done: Mutex<sync_mpsc::Receiver<()>>,
}

impl Backend {
    pub fn release_input(&self) -> bool {
        let (done, completed) = sync_mpsc::sync_channel(1);
        self.input
            .try_send(InputCommand::ReleaseSync { done })
            .is_ok()
            && completed.recv_timeout(Duration::from_secs(1)).is_ok()
    }

    pub fn frame_is_fresh(&self, max_age: Duration) -> bool {
        !self.shared.stopped.load(Ordering::Acquire)
            && self
                .frames
                .borrow()
                .as_ref()
                .is_some_and(|frame| frame.captured_at.elapsed() <= max_age)
    }

    pub fn start(socket: PathBuf, fps: u32, quality: u8) -> Result<Self> {
        Self::start_with_output(socket, fps, quality, None)
    }

    pub fn start_with_output(
        socket: PathBuf,
        fps: u32,
        quality: u8,
        output: Option<String>,
    ) -> Result<Self> {
        Self::start_with_output_cancellable(
            socket,
            fps,
            quality,
            output,
            Arc::new(AtomicBool::new(false)),
        )
    }

    pub fn start_with_output_cancellable(
        socket: PathBuf,
        fps: u32,
        quality: u8,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        ensure!((1..=60).contains(&fps), "FPS must be between 1 and 60");
        ensure!(
            (1..=100).contains(&quality),
            "JPEG quality must be between 1 and 100"
        );
        let deadline = Instant::now() + START_TIMEOUT;
        let capture_stream = connect(&socket, deadline, Some(&cancelled))?;
        let input_stream = connect(&socket, deadline, Some(&cancelled))?;
        let input_socket = input_stream.try_clone()?;
        let (frames_tx, frames) = watch::channel(None);
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            frames: frames_tx,
            capture_socket: Mutex::new(None),
        });
        let (input, input_rx) = mpsc::channel(128);
        let (ready_tx, ready_rx) = sync_mpsc::sync_channel(2);
        let (done_tx, input_done) = sync_mpsc::sync_channel(1);
        let mut backend = Self {
            frames,
            input,
            shared: shared.clone(),
            workers: Mutex::new(Vec::new()),
            input_socket,
            input_done: Mutex::new(input_done),
        };
        let input_shared = shared.clone();
        let input_ready = ready_tx.clone();
        let input_output = output.clone();
        let input_worker =
            thread::Builder::new()
                .name("wayland-input".into())
                .spawn(move || {
                    let result = run_input(
                        input_stream,
                        input_rx,
                        &input_shared,
                        &input_ready,
                        input_output.as_deref(),
                    );
                    if let Err(error) = result
                        && !input_shared.stopped.load(Ordering::Acquire)
                    {
                        let _ = input_ready.try_send(Err(format!("Wayland input: {error:#}")));
                        tracing::error!("Wayland input failed: {error:#}");
                    }
                    input_shared.finish();
                    let _ = done_tx.send(());
                })?;
        backend.workers.get_mut().unwrap().push(input_worker);
        let capture_worker = thread::Builder::new()
            .name("wayland-capture".into())
            .spawn(move || {
                let result = run_capture(
                    capture_stream,
                    &CaptureTarget { socket, output },
                    deadline,
                    fps,
                    &shared,
                    &ready_tx,
                );
                if let Err(error) = result
                    && !shared.stopped.load(Ordering::Acquire)
                {
                    let _ = ready_tx.try_send(Err(format!("Wayland capture: {error:#}")));
                    tracing::error!("Wayland capture failed: {error:#}");
                }
                shared.finish();
            })?;
        backend.workers.get_mut().unwrap().push(capture_worker);
        let mut pending = 2;
        while pending > 0 {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "Wayland startup cancelled"
            );
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(
                !remaining.is_zero(),
                "Wayland startup did not become ready: timed out"
            );
            match ready_rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(Ok(())) => pending -= 1,
                Ok(Err(error)) => bail!(error),
                Err(sync_mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => bail!("Wayland startup did not become ready: {error}"),
            }
        }
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "Wayland startup cancelled"
        );
        ensure!(
            !backend.shared.stopped.load(Ordering::Acquire) && backend.frames.borrow().is_some(),
            "Wayland workers stopped during startup"
        );
        Ok(backend)
    }

    pub fn stop(&self) {
        self.shared.finish();
        if let Some(socket) = self
            .shared
            .capture_socket
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = socket.shutdown(Shutdown::Both);
        }
        let mut workers = self
            .workers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if workers.is_empty() {
            return;
        }
        let _ = self
            .input_done
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recv_timeout(Duration::from_millis(100));
        let _ = self.input_socket.shutdown(Shutdown::Both);
        for worker in workers.drain(..) {
            worker.thread().unpark();
            let _ = worker.join();
        }
        self.shared.frames.send_replace(None);
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

fn connect(socket: &Path, deadline: Instant, stopped: Option<&AtomicBool>) -> Result<UnixStream> {
    let bytes = socket.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    ensure!(
        !bytes.is_empty() && bytes.len() < address.sun_path.len() && !bytes.contains(&0),
        "invalid Wayland socket path"
    );
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (destination, source) in address.sun_path.iter_mut().zip(bytes) {
        *destination = *source as libc::c_char;
    }
    let descriptor = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    ensure!(
        descriptor >= 0,
        "create Wayland socket: {}",
        std::io::Error::last_os_error()
    );
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let stream = UnixStream::from(descriptor);
    loop {
        ensure!(
            Instant::now() < deadline,
            "Wayland socket connection timed out"
        );
        ensure!(
            !stopped.is_some_and(|stopped| stopped.load(Ordering::Acquire)),
            "Wayland connection cancelled"
        );
        let result = unsafe {
            libc::connect(
                stream.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1)
                    as libc::socklen_t,
            )
        };
        if result == 0 {
            stream.set_nonblocking(false)?;
            return Ok(stream);
        }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EAGAIN | libc::EINTR) => thread::park_timeout(Duration::from_millis(5)),
            Some(libc::EINPROGRESS) => loop {
                ensure!(
                    Instant::now() < deadline,
                    "Wayland socket connection timed out"
                );
                ensure!(
                    !stopped.is_some_and(|stopped| stopped.load(Ordering::Acquire)),
                    "Wayland connection cancelled"
                );
                let mut poll = libc::pollfd {
                    fd: stream.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let result = unsafe { libc::poll(&mut poll, 1, 5) };
                if result > 0 {
                    if let Some(error) = stream.take_error()? {
                        return Err(error).with_context(|| {
                            format!("connect to Wayland socket {}", socket.display())
                        });
                    }
                    stream.set_nonblocking(false)?;
                    return Ok(stream);
                }
                if result < 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                {
                    return Err(std::io::Error::last_os_error()).context("poll Wayland connection");
                }
            },
            _ => {
                return Err(error)
                    .with_context(|| format!("connect to Wayland socket {}", socket.display()));
            }
        }
    }
}

struct CaptureBatch {
    connection: WayshotConnection,
    started: Instant,
    frames: usize,
}

fn capture_batch_expired(frames: usize, elapsed: Duration) -> bool {
    frames >= 8 || elapsed >= Duration::from_millis(250)
}

fn run_capture(
    stream: UnixStream,
    target: &CaptureTarget,
    startup_deadline: Instant,
    fps: u32,
    shared: &Shared,
    ready: &sync_mpsc::SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    let mut initial_stream = Some(stream);
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let mut first = true;
    let mut previous_pixels: Arc<[u8]> = Arc::from([]);
    let mut previous_dimensions = (0, 0);
    let mut batch: Option<CaptureBatch> = None;
    let mut capture_failures = 0;
    while !shared.stopped.load(Ordering::Acquire) {
        let started = Instant::now();
        if batch
            .as_ref()
            .is_none_or(|batch| capture_batch_expired(batch.frames, batch.started.elapsed()))
        {
            batch.take();
            if let Some(socket) = shared
                .capture_socket
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
            {
                let _ = socket.shutdown(Shutdown::Both);
            }
            let stream = match initial_stream.take() {
                Some(stream) => stream,
                None => connect(
                    &target.socket,
                    if first {
                        startup_deadline
                    } else {
                        Instant::now() + START_TIMEOUT
                    },
                    Some(&shared.stopped),
                )?,
            };
            if !replace_capture_socket(shared, &stream)? {
                break;
            }
            let connection = Connection::from_socket(stream)?;
            let capture = WayshotConnection::from_connection(connection)
                .context("initialize native screencopy (compositor protocols required)")?;
            batch = Some(CaptureBatch {
                connection: capture,
                started: Instant::now(),
                frames: 0,
            });
        }
        let current = batch.as_mut().context("missing capture connection")?;
        let capture = &current.connection;
        // Preserve compositor/application cursor shapes in both video codecs.
        // Readiness advertises this so the browser can hide its local cursor.
        let captured = if let Some(name) = &target.output {
            let output = capture
                .get_all_outputs()
                .iter()
                .find(|output| &output.name == name)
                .context("selected monitor disconnected")?;
            capture.screenshot_single_output(output, true)
        } else if capture.get_all_outputs().len() == 1 {
            capture.screenshot_single_output(&capture.get_all_outputs()[0], true)
        } else {
            capture.screenshot_all(true)
        };
        let image = match captured {
            Ok(image) => {
                capture_failures = 0;
                image
            }
            Err(error) => {
                if shared.stopped.load(Ordering::Acquire) {
                    break;
                }
                capture_failures += 1;
                if capture_failures >= 3 {
                    return Err(error).context("capture desktop after reconnect");
                }
                tracing::warn!(%error, capture_failures, "refreshing capture connection after failed frame");
                batch.take();
                thread::park_timeout(interval.min(Duration::from_millis(100)));
                continue;
            }
        };
        let captured_at = Instant::now();
        current.frames += 1;
        let rgba = image.into_rgba8();
        let dimensions = (rgba.width(), rgba.height());
        let changed = previous_dimensions != dimensions
            || previous_pixels.as_ref() != rgba.as_raw().as_slice();
        if shared.stopped.load(Ordering::Acquire) {
            break;
        }
        if changed {
            previous_pixels = rgba.into_raw().into();
            previous_dimensions = dimensions;
        }
        let frame = Arc::new(Frame {
            width: dimensions.0,
            height: dimensions.1,
            jpeg: OnceLock::new(),
            pixels: previous_pixels.clone(),
            captured_at,
        });
        publish_frame(shared, frame, changed);
        if first {
            ready.send(Ok(()))?;
            first = false;
        }
        thread::park_timeout(interval.saturating_sub(started.elapsed()));
    }
    Ok(())
}

fn publish_frame(shared: &Shared, frame: Arc<Frame>, changed: bool) {
    shared.frames.send_if_modified(|latest| {
        if shared.stopped.load(Ordering::Acquire) {
            return false;
        }
        *latest = Some(frame);
        changed
    });
}

fn replace_capture_socket(shared: &Shared, stream: &UnixStream) -> Result<bool> {
    let mut active = shared
        .capture_socket
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if shared.stopped.load(Ordering::Acquire) {
        let _ = stream.shutdown(Shutdown::Both);
        return Ok(false);
    }
    if let Some(previous) = active.replace(stream.try_clone()?) {
        let _ = previous.shutdown(Shutdown::Both);
    }
    Ok(true)
}

struct InputState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for InputState {
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

delegate_noop!(InputState: ignore wl_seat::WlSeat);
delegate_noop!(InputState: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(InputState: ignore ZwpVirtualKeyboardV1);
delegate_noop!(InputState: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(InputState: ignore ZwlrVirtualPointerV1);

struct Devices {
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    context: xkb::Context,
    base_keymap: xkb::Keymap,
    state: xkb::State,
    keys: BTreeSet<u32>,
    buttons: BTreeSet<u32>,
    started: Instant,
}

impl Devices {
    fn time(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn modifiers(&self) {
        self.keyboard.modifiers(
            self.state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            self.state.serialize_mods(xkb::STATE_MODS_LATCHED),
            self.state.serialize_mods(xkb::STATE_MODS_LOCKED),
            self.state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        );
    }

    fn key(&mut self, code: u32, pressed: bool) {
        let changed = if pressed {
            self.keys.insert(code)
        } else {
            self.keys.remove(&code)
        };
        if changed {
            self.keyboard.key(self.time(), code, u32::from(pressed));
            self.state.update_key(
                (code + 8).into(),
                if pressed {
                    xkb::KeyDirection::Down
                } else {
                    xkb::KeyDirection::Up
                },
            );
            self.modifiers();
        }
    }

    fn release(&mut self) {
        for code in std::mem::take(&mut self.keys) {
            self.keyboard.key(self.time(), code, 0);
        }
        for button in std::mem::take(&mut self.buttons) {
            self.pointer
                .button(self.time(), button, wl_pointer::ButtonState::Released);
        }
        self.state = xkb::State::new(&self.base_keymap);
        self.modifiers();
        self.pointer.frame();
    }

    fn command(&mut self, command: InputCommand) -> Result<()> {
        match command {
            InputCommand::ReleaseSync { .. } => {
                bail!("input release barrier requires a compositor roundtrip")
            }
            InputCommand::Move { x, y } => {
                ensure!(
                    x.is_finite() && y.is_finite(),
                    "non-finite pointer coordinates"
                );
                self.pointer.motion_absolute(
                    self.time(),
                    (x.clamp(0.0, 1.0) * 65535.0).round() as u32,
                    (y.clamp(0.0, 1.0) * 65535.0).round() as u32,
                    65535,
                    65535,
                );
                self.pointer.frame();
            }
            InputCommand::Button { button, pressed } => {
                let button = linux_button(button).context("unsupported mouse button")?;
                let changed = if pressed {
                    self.buttons.insert(button)
                } else {
                    self.buttons.remove(&button)
                };
                if changed {
                    self.pointer.button(
                        self.time(),
                        button,
                        if pressed {
                            wl_pointer::ButtonState::Pressed
                        } else {
                            wl_pointer::ButtonState::Released
                        },
                    );
                    self.pointer.frame();
                }
            }
            InputCommand::Key { code, pressed } => {
                ensure!(
                    (1..=MAX_EVDEV_CODE).contains(&code),
                    "invalid evdev keycode"
                );
                self.key(code, pressed);
            }
            InputCommand::Scroll { x, y } => {
                ensure!(x.is_finite() && y.is_finite(), "non-finite scroll");
                self.pointer.axis_source(wl_pointer::AxisSource::Continuous);
                if x != 0.0 {
                    self.pointer.axis(
                        self.time(),
                        wl_pointer::Axis::HorizontalScroll,
                        x.clamp(-1000.0, 1000.0),
                    );
                }
                if y != 0.0 {
                    self.pointer.axis(
                        self.time(),
                        wl_pointer::Axis::VerticalScroll,
                        y.clamp(-1000.0, 1000.0),
                    );
                }
                self.pointer.frame();
            }
            InputCommand::Release => self.release(),
            InputCommand::Text { .. } => unreachable!(),
        }
        Ok(())
    }
}

fn linux_button(button: u32) -> Option<u32> {
    match button {
        0 => Some(0x110),
        1 => Some(0x112),
        2 => Some(0x111),
        _ => None,
    }
}

fn send_keymap(keyboard: &ZwpVirtualKeyboardV1, keymap: &xkb::Keymap) -> Result<()> {
    let map = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    let mut file = tempfile::tempfile().context("create virtual keyboard keymap fd")?;
    file.write_all(map.as_bytes())?;
    file.write_all(&[0])?;
    file.seek(SeekFrom::Start(0))?;
    keyboard.keymap(1, file.as_fd(), u32::try_from(map.len() + 1)?);
    Ok(())
}

fn unicode_keymap(context: &xkb::Context, characters: &[char]) -> Result<xkb::Keymap> {
    ensure!(
        !characters.is_empty() && characters.len() <= 128,
        "invalid text batch"
    );
    let mut codes = String::new();
    let mut symbols = String::new();
    for (index, character) in characters.iter().enumerate() {
        codes.push_str(&format!("<U{index:03}> = {};\n", index + 9));
        let symbol = match character {
            '\n' | '\r' => "Return".into(),
            '\t' => "Tab".into(),
            character => format!("U{:04X}", *character as u32),
        };
        symbols.push_str(&format!(
            "key <U{index:03}> {{ type=\"ONE_LEVEL\", [ {symbol} ] }};\n"
        ));
    }
    xkb::Keymap::new_from_string(
        context,
        format!(
            "xkb_keymap {{ xkb_keycodes {{ minimum=8; maximum=255; {codes} }}; \
             xkb_types {{ include \"complete\" }}; \
             xkb_compatibility {{ include \"complete\" }}; \
             xkb_symbols {{ {symbols} }}; }};"
        ),
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .context("compile Unicode virtual keymap")
}

fn run_input(
    stream: UnixStream,
    mut receiver: mpsc::Receiver<InputCommand>,
    shared: &Shared,
    ready: &sync_mpsc::SyncSender<std::result::Result<(), String>>,
    output: Option<&str>,
) -> Result<()> {
    let connection = Connection::from_socket(stream)?;
    let (globals, mut queue) = registry_queue_init::<InputState>(&connection)?;
    let handle = queue.handle();
    let seat: wl_seat::WlSeat = globals
        .bind(&handle, 1..=1, ())
        .context("compositor does not advertise wl_seat")?;
    let keyboard_manager: ZwpVirtualKeyboardManagerV1 = globals
        .bind(&handle, 1..=1, ())
        .context("compositor does not support zwp_virtual_keyboard_manager_v1")?;
    let pointer_manager: ZwlrVirtualPointerManagerV1 = globals
        .bind(&handle, 1..=2, ())
        .context("compositor does not support zwlr_virtual_pointer_manager_v1")?;
    let keyboard = keyboard_manager.create_virtual_keyboard(&seat, &handle, ());
    let output_connection = if output.is_some() {
        Some(WayshotConnection::from_connection(connection.clone())?)
    } else {
        None
    };
    let pointer = if let Some(name) = output {
        ensure!(
            pointer_manager.version() >= 2,
            "compositor cannot bind input to a selected monitor"
        );
        let selected = output_connection
            .as_ref()
            .unwrap()
            .get_all_outputs()
            .iter()
            .find(|candidate| candidate.name == name)
            .context("selected input monitor disconnected")?;
        pointer_manager.create_virtual_pointer_with_output(
            Some(&seat),
            Some(&selected.wl_output),
            &handle,
            (),
        )
    } else {
        pointer_manager.create_virtual_pointer(Some(&seat), &handle, ())
    };
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let base_keymap = xkb::Keymap::new_from_names(
        &context,
        "evdev",
        "pc105",
        "us",
        "",
        Some(String::new()),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .context("compile full evdev keyboard keymap")?;
    send_keymap(&keyboard, &base_keymap)?;
    let state = xkb::State::new(&base_keymap);
    let mut devices = Devices {
        keyboard,
        pointer,
        context,
        base_keymap,
        state,
        keys: BTreeSet::new(),
        buttons: BTreeSet::new(),
        started: Instant::now(),
    };
    let mut dispatch = InputState;
    queue
        .roundtrip(&mut dispatch)
        .context("initialize virtual input devices")?;
    ready.send(Ok(()))?;
    let result = (|| -> Result<()> {
        let mut last_sync = Instant::now();
        while !shared.stopped.load(Ordering::Acquire) {
            match receiver.try_recv().map(validated_input) {
                Ok(Some(InputCommand::ReleaseSync { done })) => {
                    devices.release();
                    queue.roundtrip(&mut dispatch)?;
                    let _ = done.send(());
                }
                Ok(Some(InputCommand::Text { text })) => {
                    devices.release();
                    let characters: Vec<char> = text.chars().collect();
                    for batch in characters.chunks(128) {
                        if shared.stopped.load(Ordering::Acquire) {
                            break;
                        }
                        let map = unicode_keymap(&devices.context, batch)?;
                        send_keymap(&devices.keyboard, &map)?;
                        queue.roundtrip(&mut dispatch)?;
                        for index in 0..batch.len() {
                            devices.keyboard.key(devices.time(), index as u32 + 1, 1);
                            devices.keyboard.key(devices.time(), index as u32 + 1, 0);
                        }
                        queue.roundtrip(&mut dispatch)?;
                    }
                    send_keymap(&devices.keyboard, &devices.base_keymap)?;
                    devices.modifiers();
                    queue.roundtrip(&mut dispatch)?;
                }
                Ok(Some(command)) => devices.command(command)?,
                Ok(None) => {}
                Err(mpsc::error::TryRecvError::Empty) => {
                    thread::park_timeout(Duration::from_millis(5))
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
            connection.flush()?;
            if last_sync.elapsed() >= Duration::from_millis(250) {
                queue.roundtrip(&mut dispatch)?;
                last_sync = Instant::now();
            }
        }
        Ok(())
    })();
    devices.release();
    devices.keyboard.destroy();
    devices.pointer.destroy();
    pointer_manager.destroy();
    let _ = queue.roundtrip(&mut dispatch);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_barrier_cannot_be_requested_over_the_public_protocol() {
        assert!(serde_json::from_str::<InputCommand>(r#"{"type":"release_sync"}"#).is_err());
    }

    #[test]
    fn capture_batches_bound_protocol_retention_and_output_refresh_delay() {
        assert!(!capture_batch_expired(7, Duration::from_millis(249)));
        assert!(capture_batch_expired(8, Duration::ZERO));
        assert!(capture_batch_expired(1, Duration::from_millis(250)));
    }

    #[test]
    fn jpeg_is_lazy_cached_and_decodable() {
        let frame = Frame {
            width: 640,
            height: 360,
            pixels: vec![127; 640 * 360 * 4].into(),
            jpeg: OnceLock::new(),
            captured_at: Instant::now(),
        };
        assert!(frame.jpeg.get().is_none());
        let jpeg = frame.jpeg(75).unwrap();
        assert!(Arc::ptr_eq(&jpeg, &frame.jpeg(75).unwrap()));
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (640, 360));
        assert!(!Arc::ptr_eq(&jpeg, &frame.jpeg(50).unwrap()));
        assert!(Arc::ptr_eq(&jpeg, &frame.jpeg(75).unwrap()));
    }

    #[test]
    fn unchanged_frame_refreshes_health_without_waking_video_subscribers() {
        let backend = backend_with_frame(Some(Instant::now() - Duration::from_secs(10)));
        let frame = Arc::new(Frame {
            width: 1,
            height: 1,
            jpeg: OnceLock::new(),
            pixels: Arc::from([]),
            captured_at: Instant::now(),
        });
        publish_frame(&backend.shared, frame, false);
        assert!(!backend.frames.has_changed().unwrap());
        assert!(backend.frame_is_fresh(Duration::from_secs(1)));
    }

    fn backend_with_frame(captured_at: Option<Instant>) -> Backend {
        let frame = captured_at.map(|captured_at| {
            Arc::new(Frame {
                width: 1,
                height: 1,
                jpeg: OnceLock::new(),
                pixels: Arc::from([]),
                captured_at,
            })
        });
        let (sender, frames) = watch::channel(frame);
        let (input, _) = mpsc::channel(1);
        let (_, input_done) = sync_mpsc::channel();
        Backend {
            frames,
            input,
            shared: Arc::new(Shared {
                stopped: AtomicBool::new(false),
                frames: sender,
                capture_socket: Mutex::new(None),
            }),
            workers: Mutex::new(Vec::new()),
            input_socket: UnixStream::pair().unwrap().0,
            input_done: Mutex::new(input_done),
        }
    }

    #[test]
    fn replacement_capture_socket_is_closed_on_stop() {
        use std::io::Read;

        let backend = backend_with_frame(None);
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        let (second, mut second_peer) = UnixStream::pair().unwrap();
        first_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        second_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert!(replace_capture_socket(&backend.shared, &first).unwrap());
        assert!(replace_capture_socket(&backend.shared, &second).unwrap());
        assert_eq!(first_peer.read(&mut [0]).unwrap(), 0);
        backend.stop();
        assert_eq!(second_peer.read(&mut [0]).unwrap(), 0);
        let (third, mut third_peer) = UnixStream::pair().unwrap();
        third_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert!(!replace_capture_socket(&backend.shared, &third).unwrap());
        assert_eq!(third_peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn socket_connection_obeys_deadline_and_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-test");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
        let queued = connect(&socket, Instant::now() + Duration::from_secs(1), None).unwrap();
        let started = Instant::now();
        assert!(connect(&socket, started + Duration::from_millis(50), None).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        let stopped = AtomicBool::new(true);
        assert!(
            connect(
                &socket,
                Instant::now() + Duration::from_secs(1),
                Some(&stopped)
            )
            .is_err()
        );
        drop(queued);
    }

    #[test]
    fn frame_freshness_rejects_missing_stale_and_stopped_frames() {
        let max_age = Duration::from_secs(5);
        let missing = backend_with_frame(None);
        assert!(!missing.frame_is_fresh(max_age));
        let stale = backend_with_frame(Some(Instant::now() - Duration::from_secs(10)));
        assert!(!stale.frame_is_fresh(max_age));
        let current = backend_with_frame(Some(Instant::now()));
        assert!(current.frame_is_fresh(max_age));
        current.shared.stopped.store(true, Ordering::Release);
        assert!(!current.frame_is_fresh(max_age));
        current.shared.frames.send_replace(None);
        assert!(!current.frame_is_fresh(max_age));
    }

    #[test]
    fn input_validation_accepts_supported_boundaries() {
        for command in [
            InputCommand::Move { x: 0.0, y: 1.0 },
            InputCommand::Move { x: 1.0, y: 0.0 },
            InputCommand::Button {
                button: 0,
                pressed: true,
            },
            InputCommand::Button {
                button: 2,
                pressed: false,
            },
            InputCommand::Key {
                code: 1,
                pressed: true,
            },
            InputCommand::Key {
                code: MAX_EVDEV_CODE,
                pressed: false,
            },
            InputCommand::Scroll {
                x: -1000.0,
                y: 1000.0,
            },
            InputCommand::Text {
                text: "é中😀\r\n\t".into(),
            },
            InputCommand::Text {
                text: "é".repeat(MAX_TEXT_BYTES / 2),
            },
            InputCommand::Release,
        ] {
            assert!(command.validate().is_ok(), "{command:?}");
        }
    }

    #[test]
    fn input_validation_rejects_out_of_range_and_nonfinite_values() {
        for command in [
            InputCommand::Move { x: -0.01, y: 0.0 },
            InputCommand::Move { x: 0.0, y: 1.01 },
            InputCommand::Move {
                x: f64::NAN,
                y: 0.0,
            },
            InputCommand::Move {
                x: 0.0,
                y: f64::INFINITY,
            },
            InputCommand::Button {
                button: 3,
                pressed: true,
            },
            InputCommand::Key {
                code: 0,
                pressed: true,
            },
            InputCommand::Key {
                code: MAX_EVDEV_CODE + 1,
                pressed: false,
            },
            InputCommand::Scroll {
                x: -1000.01,
                y: 0.0,
            },
            InputCommand::Scroll { x: 0.0, y: 1000.01 },
            InputCommand::Scroll {
                x: f64::NAN,
                y: 0.0,
            },
            InputCommand::Scroll {
                x: 0.0,
                y: f64::NEG_INFINITY,
            },
            InputCommand::Text {
                text: "a".repeat(MAX_TEXT_BYTES + 1),
            },
            InputCommand::Text {
                text: "é".repeat(MAX_TEXT_BYTES / 2 + 1),
            },
            InputCommand::Text { text: "\0".into() },
            InputCommand::Text {
                text: "\u{7f}".into(),
            },
            InputCommand::Text {
                text: "\u{85}".into(),
            },
        ] {
            assert!(command.validate().is_err(), "invalid command accepted");
        }
    }

    #[test]
    fn rejected_input_does_not_block_the_next_release() {
        let (sender, mut receiver) = mpsc::channel(2);
        sender
            .try_send(InputCommand::Key {
                code: 0,
                pressed: true,
            })
            .unwrap();
        sender.try_send(InputCommand::Release).unwrap();
        assert!(receiver.try_recv().map(validated_input).unwrap().is_none());
        assert!(matches!(
            receiver.try_recv().map(validated_input).unwrap(),
            Some(InputCommand::Release)
        ));
    }

    #[test]
    fn wire_commands_are_tagged_and_strict() {
        assert!(matches!(
            serde_json::from_str::<InputCommand>(r#"{"type":"key","code":30,"pressed":true}"#)
                .unwrap(),
            InputCommand::Key {
                code: 30,
                pressed: true
            }
        ));
        assert!(
            serde_json::from_str::<InputCommand>(r#"{"type":"move","x":0.5,"y":0.5,"extra":1}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<InputCommand>(r#"{"type":"key","code":"KeyA","pressed":true}"#)
                .is_err()
        );
    }

    #[test]
    fn browser_buttons_map_to_evdev() {
        assert_eq!(linux_button(0), Some(0x110));
        assert_eq!(linux_button(1), Some(0x112));
        assert_eq!(linux_button(2), Some(0x111));
        assert_eq!(linux_button(3), None);
    }

    #[test]
    fn unicode_map_preserves_scalars() {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let characters = ['a', 'é', '中', '😀', '\n', '\t'];
        let keymap = unicode_keymap(&context, &characters).unwrap();
        let state = xkb::State::new(&keymap);
        for (index, character) in characters[..4].iter().enumerate() {
            assert_eq!(
                state.key_get_utf32((index as u32 + 9).into()),
                *character as u32
            );
        }
        let serialized = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
        assert!(serialized.contains("xkb_keymap"));
    }

    #[test]
    fn invalid_capture_parameters_fail_before_connecting() {
        assert!(Backend::start(PathBuf::from("/nonexistent"), 0, 80).is_err());
        assert!(Backend::start(PathBuf::from("/nonexistent"), 61, 80).is_err());
        assert!(Backend::start(PathBuf::from("/nonexistent"), 30, 0).is_err());
    }

    #[test]
    fn disconnected_compositor_fails_startup() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-test");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let first = listener.accept().unwrap().0;
            let second = listener.accept().unwrap().0;
            drop((first, second));
        });
        assert!(Backend::start(socket, 30, 80).is_err());
        server.join().unwrap();
    }

    #[test]
    fn cancelled_startup_interrupts_an_unresponsive_compositor() {
        use std::io::Read;

        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-cancel");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let server = thread::spawn(move || {
            let mut first = listener.accept().unwrap().0;
            let mut second = listener.accept().unwrap().0;
            cancel.store(true, Ordering::Release);
            first.read_to_end(&mut Vec::new()).unwrap();
            second.read_to_end(&mut Vec::new()).unwrap();
        });
        let started = Instant::now();
        let error = Backend::start_with_output_cancellable(socket, 30, 80, None, cancelled)
            .err()
            .expect("startup must be cancelled");
        assert!(error.to_string().contains("cancelled"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[test]
    #[ignore = "waits for the ten-second compositor startup deadline"]
    fn unresponsive_compositor_is_interrupted_on_startup_timeout() {
        use std::io::Read;

        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-test");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let mut first = listener.accept().unwrap().0;
            let mut second = listener.accept().unwrap().0;
            let _ = first.read_to_end(&mut Vec::new());
            let _ = second.read_to_end(&mut Vec::new());
        });
        let started = Instant::now();
        assert!(Backend::start(socket, 30, 80).is_err());
        assert!(started.elapsed() < START_TIMEOUT + Duration::from_secs(5));
        server.join().unwrap();
    }
}
