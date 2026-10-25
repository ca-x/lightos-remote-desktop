pub mod capabilities;

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
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc as sync_mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use libwayshot::WayshotConnection;
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

#[derive(Debug)]
struct StartupTimeout {
    capture_ready: bool,
    pending_workers: usize,
}
impl std::fmt::Display for StartupTimeout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Wayland startup did not become ready: timed out (capture_ready={}, pending_workers={})",
            self.capture_ready, self.pending_workers
        )
    }
}
impl std::error::Error for StartupTimeout {}

fn retry_startup<T>(
    cancelled: &AtomicBool,
    total: Duration,
    windows: &[Duration],
    mut attempt: impl FnMut(Instant) -> Result<T>,
) -> Result<T> {
    ensure!(!windows.is_empty(), "missing startup attempt window");
    let deadline = Instant::now() + total;
    for (index, window) in windows.iter().enumerate() {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "Wayland startup cancelled"
        );
        let result = attempt(deadline.min(Instant::now() + *window));
        match result {
            Err(error)
                if error.downcast_ref::<StartupTimeout>().is_some()
                    && index + 1 < windows.len()
                    && Instant::now() < deadline
                    && !cancelled.load(Ordering::Acquire) =>
            {
                tracing::warn!(attempt = index + 1, %error, "recreating stalled Wayland startup connections");
            }
            result => return result,
        }
    }
    unreachable!("the final attempt returns its result")
}

struct CaptureTarget {
    socket: PathBuf,
    output: Option<String>,
}

// Kept as re-exports for integrations using the original public module.
use crate::backend::MAX_EVDEV_CODE;
#[cfg(test)]
use crate::backend::MAX_TEXT_BYTES;
#[cfg(test)]
pub(crate) use crate::backend::TextProbe;
pub use crate::backend::{Frame, InputCommand, InputGuard};

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
    cursor_owner: AtomicU64,
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
    socket_path: PathBuf,
    clipboard_owner: tokio::sync::Mutex<Option<tokio::process::Child>>,
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    input_socket: UnixStream,
    input_done: Mutex<sync_mpsc::Receiver<()>>,
    cursor_socket: PathBuf,
    cursor_output: Option<String>,
}

impl Backend {
    pub(crate) fn separate_cursor(&self, owner: u64) {
        let _ = self.shared.cursor_owner.compare_exchange(
            owner << 1,
            (owner << 1) | 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    pub(crate) fn begin_cursor(&self, owner: u64) {
        self.shared
            .cursor_owner
            .store(owner << 1, Ordering::Release);
    }
    pub(crate) fn release_cursor(&self, owner: u64) {
        let _ = self.shared.cursor_owner.compare_exchange(
            (owner << 1) | 1,
            owner << 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    pub(crate) fn cursor_target(&self) -> Result<(PathBuf, i32, Option<String>)> {
        let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&peer) as libc::socklen_t;
        ensure!(
            unsafe {
                libc::getsockopt(
                    self.input_socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut peer as *mut libc::ucred).cast(),
                    &mut length,
                )
            } == 0
                && peer.uid == unsafe { libc::geteuid() },
            "cursor compositor owner mismatch"
        );
        Ok((
            self.cursor_socket
                .parent()
                .context("cursor runtime unavailable")?
                .to_owned(),
            peer.pid,
            self.cursor_output.clone(),
        ))
    }

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
        // A selected monitor is the transient physical-desktop path. Rebuild
        // only a stalled initialization, using the same target and protocols.
        // start_once drops and joins both workers before a new attempt begins.
        let windows: &[Duration] = if output.is_some() {
            &[
                Duration::from_secs(1),
                Duration::from_secs(2),
                START_TIMEOUT,
            ]
        } else {
            &[START_TIMEOUT]
        };
        retry_startup(&cancelled, START_TIMEOUT, windows, |deadline| {
            Self::start_once(
                socket.clone(),
                fps,
                quality,
                output.clone(),
                cancelled.clone(),
                deadline,
            )
        })
    }

    fn start_once(
        socket: PathBuf,
        fps: u32,
        _quality: u8,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> Result<Self> {
        let capture_stream = connect(&socket, deadline, Some(&cancelled))?;
        let input_stream = connect(&socket, deadline, Some(&cancelled))?;
        let input_socket = input_stream.try_clone()?;
        let (frames_tx, frames) = watch::channel(None);
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            frames: frames_tx,
            capture_socket: Mutex::new(None),
            cursor_owner: AtomicU64::new(0),
        });
        let (input, input_rx) = mpsc::channel(128);
        let (ready_tx, ready_rx) = sync_mpsc::sync_channel(2);
        let (done_tx, input_done) = sync_mpsc::sync_channel(1);
        let mut backend = Self {
            socket_path: socket.clone(),
            clipboard_owner: tokio::sync::Mutex::new(None),
            frames,
            input,
            shared: shared.clone(),
            workers: Mutex::new(Vec::new()),
            input_socket,
            input_done: Mutex::new(input_done),
            cursor_socket: PathBuf::new(),
            cursor_output: None,
        };
        backend.cursor_socket = socket.clone();
        backend.cursor_output = output.clone();
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
            if remaining.is_zero() {
                let capture_ready = backend.frames.borrow().is_some();
                return Err(StartupTimeout {
                    capture_ready,
                    pending_workers: pending,
                }
                .into());
            }
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
        if let Ok(mut owner) = self.clipboard_owner.try_lock()
            && let Some(mut child) = owner.take()
        {
            let _ = child.start_kill();
        }

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

impl crate::backend::DesktopBackend for Backend {
    fn kind(&self) -> crate::backend::BackendKind {
        crate::backend::BackendKind::Wayland
    }
    fn cursor_embedded(&self) -> bool {
        true
    }
    fn begin_cursor(&self, owner: u64) {
        Backend::begin_cursor(self, owner);
    }
    fn separate_cursor(&self, owner: u64) {
        Backend::separate_cursor(self, owner);
    }
    fn release_cursor(&self, owner: u64) {
        Backend::release_cursor(self, owner);
    }
    fn cursor_target(&self) -> Result<(PathBuf, i32, Option<String>)> {
        Backend::cursor_target(self)
    }
    fn supports_image_clipboard(&self) -> bool {
        true
    }
    fn clipboard_write_png<'a>(
        &'a self,
        data: &'a [u8],
    ) -> futures_util::future::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ensure!(data.len() <= 8 * 1024 * 1024, "clipboard image too large");
            self.write_clipboard_bytes("image/png", data).await
        })
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn release_input(&self) -> bool {
        Backend::release_input(self)
    }
    fn frame_is_fresh(&self, max_age: Duration) -> bool {
        Backend::frame_is_fresh(self, max_age)
    }
    fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }
    fn stop(&self) {
        Backend::stop(self)
    }
    fn clipboard_read(&self) -> futures_util::future::BoxFuture<'_, Result<String>> {
        Box::pin(async move {
            use tokio::io::AsyncReadExt;
            let mut command = clipboard_command(&self.socket_path, "wl-paste")?;
            let mut child = command
                .args(["--no-newline", "--type", "text"])
                .stdout(std::process::Stdio::piped())
                .spawn()?;
            let mut stdout = child.stdout.take().context("clipboard pipe unavailable")?;
            let mut output = Vec::new();
            tokio::time::timeout(Duration::from_secs(2), async {
                (&mut stdout)
                    .take(crate::backend::MAX_CLIPBOARD_BYTES as u64 + 1)
                    .read_to_end(&mut output)
                    .await?;
                ensure!(
                    output.len() <= crate::backend::MAX_CLIPBOARD_BYTES,
                    "clipboard too large"
                );
                ensure!(child.wait().await?.success(), "clipboard unavailable");
                String::from_utf8(output).context("clipboard is not UTF-8 text")
            })
            .await
            .context("clipboard read timed out")?
        })
    }
    fn clipboard_write<'a>(
        &'a self,
        text: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ensure!(
                text.len() <= crate::backend::MAX_CLIPBOARD_BYTES,
                "clipboard too large"
            );
            self.write_clipboard_bytes("text/plain;charset=utf-8", text.as_bytes())
                .await
        })
    }
}

impl Backend {
    async fn write_clipboard_bytes(&self, mime: &str, data: &[u8]) -> Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut owner = self.clipboard_owner.lock().await;
        ensure!(
            !self.shared.stopped.load(Ordering::Acquire),
            "desktop stopped"
        );
        let mut command = clipboard_command(&self.socket_path, "wl-copy")?;
        let mut child = command
            .args(["--foreground", "--type", mime])
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().context("clipboard pipe unavailable")?;
        tokio::time::timeout(Duration::from_secs(2), async {
            stdin.write_all(data).await?;
            stdin.shutdown().await
        })
        .await
        .context("clipboard write timed out")??;
        drop(stdin);
        // Closing stdin only delivers the bytes; wl-copy may not own the
        // selection yet. Confirm the advertised content before a paste chord.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                ensure!(child.try_wait()?.is_none(), "clipboard owner exited");
                let mut reader = clipboard_command(&self.socket_path, "wl-paste")?
                    .args(["--no-newline", "--type", mime])
                    .stdout(std::process::Stdio::piped())
                    .spawn()?;
                let mut bytes = Vec::new();
                reader
                    .stdout
                    .take()
                    .context("clipboard pipe unavailable")?
                    .take(data.len() as u64 + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                if reader.wait().await?.success() && bytes == data {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            anyhow::Ok(())
        })
        .await
        .context("clipboard ownership confirmation timed out")??;
        if let Some(mut previous) = owner.take() {
            let _ = previous.kill().await;
        }
        *owner = Some(child);
        Ok(())
    }
}

fn clipboard_command(socket: &Path, program: &str) -> Result<tokio::process::Command> {
    let executable = platform_detect::find_executable(Path::new(program))
        .context("Wayland clipboard helper unavailable")?;
    let mut command = tokio::process::Command::new(executable);
    command
        .env("WAYLAND_DISPLAY", socket)
        .env(
            "XDG_RUNTIME_DIR",
            socket.parent().context("Wayland socket has no runtime")?,
        )
        .env_remove("DISPLAY")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    Ok(command)
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
    let mut previous_cursor = true;
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
        let cursor_embedded = shared.cursor_owner.load(Ordering::Acquire) & 1 == 0;
        let captured = if let Some(name) = &target.output {
            let output = capture
                .get_all_outputs()
                .iter()
                .find(|output| &output.name == name)
                .context("selected monitor disconnected")?;
            capture.screenshot_single_output(output, cursor_embedded)
        } else if capture.get_all_outputs().len() == 1 {
            capture.screenshot_single_output(&capture.get_all_outputs()[0], cursor_embedded)
        } else {
            capture.screenshot_all(cursor_embedded)
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
            || previous_pixels.as_ref() != rgba.as_raw().as_slice()
            || previous_cursor != cursor_embedded;
        previous_cursor = cursor_embedded;
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
            cursor_embedded,
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
            InputCommand::Chord { codes } => {
                self.release();
                for code in &codes {
                    self.key(*code, true);
                }
                for code in codes.iter().rev() {
                    self.key(*code, false);
                }
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
            InputCommand::Text { .. } | InputCommand::Guarded { .. } => unreachable!(),
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
            let received = receiver
                .try_recv()
                .map(|command| validated_input(command).and_then(InputCommand::into_active));
            match received {
                Ok(Some((InputCommand::ReleaseSync { done }, _))) => {
                    devices.release();
                    queue.roundtrip(&mut dispatch)?;
                    let _ = done.send(());
                }
                Ok(Some((InputCommand::Text { text }, guard))) => {
                    devices.release();
                    let characters: Vec<char> = text.chars().collect();
                    for batch in characters.chunks(128) {
                        if shared.stopped.load(Ordering::Acquire)
                            || guard.as_ref().is_some_and(|g| !g.active())
                        {
                            break;
                        }
                        let map = unicode_keymap(&devices.context, batch)?;
                        send_keymap(&devices.keyboard, &map)?;
                        queue.roundtrip(&mut dispatch)?;
                        for index in 0..batch.len() {
                            if shared.stopped.load(Ordering::Acquire)
                                || guard.as_ref().is_some_and(|g| !g.active())
                            {
                                break;
                            }
                            #[cfg(test)]
                            if let Some(probe) = guard.as_ref().and_then(|g| g.probe.as_ref()) {
                                probe.characters.fetch_add(1, Ordering::AcqRel);
                            }
                            devices.keyboard.key(devices.time(), index as u32 + 1, 1);
                            devices.keyboard.key(devices.time(), index as u32 + 1, 0);
                        }
                        queue.roundtrip(&mut dispatch)?;
                        #[cfg(test)]
                        if let Some(probe) = guard.as_ref().and_then(|g| g.probe.as_ref())
                            && probe.batches.fetch_add(1, Ordering::AcqRel) == 0
                        {
                            let _ = probe.first_batch.send(());
                            probe
                                .resume
                                .lock()
                                .unwrap()
                                .recv_timeout(Duration::from_secs(5))?;
                        }
                    }
                    send_keymap(&devices.keyboard, &devices.base_keymap)?;
                    devices.modifiers();
                    queue.roundtrip(&mut dispatch)?;
                    if let Some(guard) = &guard {
                        #[cfg(test)]
                        if let Some(probe) = &guard.probe {
                            probe.restored.store(true, Ordering::Release);
                        }
                        guard.text_done.notify_one();
                    }
                }
                Ok(Some((command, _))) => devices.command(command)?,
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
pub(crate) mod tests {
    use super::*;

    #[test]
    fn input_barrier_cannot_be_requested_over_the_public_protocol() {
        for payload in [
            r#"{"type":"release_sync"}"#,
            r#"{"type":"guarded","command":{"type":"key","code":30,"pressed":true}}"#,
        ] {
            assert!(serde_json::from_str::<InputCommand>(payload).is_err());
        }
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
            cursor_embedded: true,
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
            cursor_embedded: true,
        });
        publish_frame(&backend.shared, frame, false);
        assert!(!backend.frames.has_changed().unwrap());
        assert!(backend.frame_is_fresh(Duration::from_secs(1)));
    }

    #[cfg(feature = "rdp")]
    pub(crate) fn rdp_frame(width: u32, height: u32, color: [u8; 4]) -> Arc<Frame> {
        Arc::new(Frame {
            width,
            height,
            jpeg: OnceLock::new(),
            pixels: color.repeat(width as usize * height as usize).into(),
            captured_at: Instant::now(),
            cursor_embedded: true,
        })
    }

    #[cfg(feature = "rdp")]
    pub(crate) fn rdp_backend() -> (
        Backend,
        watch::Sender<Option<Arc<Frame>>>,
        mpsc::Receiver<InputCommand>,
    ) {
        let mut backend = backend_with_frame(None);
        let frames = backend.shared.frames.clone();
        let (input, receiver) = mpsc::channel(128);
        backend.input = input;
        (backend, frames, receiver)
    }

    fn backend_with_frame(captured_at: Option<Instant>) -> Backend {
        let frame = captured_at.map(|captured_at| {
            Arc::new(Frame {
                width: 1,
                height: 1,
                jpeg: OnceLock::new(),
                pixels: Arc::from([]),
                captured_at,
                cursor_embedded: true,
            })
        });
        let (sender, frames) = watch::channel(frame);
        let (input, _) = mpsc::channel(1);
        let (_, input_done) = sync_mpsc::channel();
        Backend {
            socket_path: "/nonexistent".into(),
            clipboard_owner: tokio::sync::Mutex::new(None),
            frames,
            input,
            shared: Arc::new(Shared {
                stopped: AtomicBool::new(false),
                frames: sender,
                capture_socket: Mutex::new(None),
                cursor_owner: AtomicU64::new(0),
            }),
            workers: Mutex::new(Vec::new()),
            input_socket: UnixStream::pair().unwrap().0,
            input_done: Mutex::new(input_done),
            cursor_socket: PathBuf::new(),
            cursor_output: None,
        }
    }

    #[test]
    fn stale_cursor_channels_cannot_change_a_new_connections_mode() {
        let backend = backend_with_frame(None);
        backend.begin_cursor(1);
        backend.separate_cursor(1);
        assert_eq!(backend.shared.cursor_owner.load(Ordering::Acquire), 3);
        backend.begin_cursor(2);
        backend.separate_cursor(2);
        backend.release_cursor(1);
        backend.separate_cursor(1);
        assert_eq!(backend.shared.cursor_owner.load(Ordering::Acquire), 5);
        backend.release_cursor(2);
        assert_eq!(backend.shared.cursor_owner.load(Ordering::Acquire), 4);
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
    fn stalled_startup_retries_are_bounded_and_stop_on_permission_or_cancellation() {
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let result = retry_startup(
            &cancel,
            Duration::from_secs(1),
            &[Duration::from_millis(50); 3],
            |_| {
                calls += 1;
                if calls == 1 {
                    Err(StartupTimeout {
                        capture_ready: false,
                        pending_workers: 1,
                    }
                    .into())
                } else {
                    Ok("ready")
                }
            },
        )
        .unwrap();
        assert_eq!(result, "ready");
        assert_eq!(calls, 2);
        calls = 0;
        let failed: Result<()> = retry_startup(
            &cancel,
            Duration::from_secs(1),
            &[Duration::from_millis(50); 3],
            |_| {
                calls += 1;
                bail!("capture permission denied")
            },
        );
        assert!(failed.is_err());
        assert_eq!(calls, 1);
        calls = 0;
        let failed: Result<()> = retry_startup(
            &cancel,
            Duration::from_secs(1),
            &[Duration::from_millis(50); 3],
            |_| {
                calls += 1;
                cancel.store(true, Ordering::Release);
                Err(StartupTimeout {
                    capture_ready: true,
                    pending_workers: 1,
                }
                .into())
            },
        );
        assert!(failed.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn timed_out_attempts_close_both_sockets_before_retrying() {
        use std::io::Read;
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-retry");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let (closed, completed) = sync_mpsc::channel();
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let mut capture = listener.accept().unwrap().0;
                let mut input = listener.accept().unwrap().0;
                capture
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                input
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                capture.read_to_end(&mut Vec::new()).unwrap();
                input.read_to_end(&mut Vec::new()).unwrap();
                closed.send(()).unwrap();
            }
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let mut calls = 0;
        let started = Instant::now();
        let result = retry_startup(
            &cancel,
            Duration::from_secs(2),
            &[Duration::from_millis(60); 3],
            |deadline| {
                if calls > 0 {
                    completed.recv_timeout(Duration::from_secs(1)).unwrap();
                }
                calls += 1;
                Backend::start_once(
                    socket.clone(),
                    30,
                    80,
                    Some("test-monitor".into()),
                    cancel.clone(),
                    deadline,
                )
            },
        );
        assert!(
            result
                .err()
                .unwrap()
                .downcast_ref::<StartupTimeout>()
                .is_some()
        );
        assert_eq!(calls, 3);
        completed.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
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
