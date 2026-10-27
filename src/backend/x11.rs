//! Native X11 capture with input and clipboard isolated in one session-bound child.
mod capture;
mod cursor;
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc as sync_mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use display_control::SessionTarget;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, watch};
use x11rb::{
    connection::Connection,
    protocol::{
        randr::{self, ConnectionExt as _},
        xfixes::{self, ConnectionExt as _},
        xproto::{self, ConnectionExt as _, ImageFormat, ImageOrder, VisualClass},
        xtest::ConnectionExt as _,
    },
    rust_connection::{DefaultStream, RustConnection},
};

use super::{
    BackendKind, ClipboardRevision, CursorFrame, DesktopBackend, Frame, InputCommand,
    MAX_CLIPBOARD_BYTES,
};

const START_TIMEOUT: Duration = Duration::from_secs(6);
const AUTH_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_AUTH_OUTPUT: usize = 16 * 1024;
const MAX_IPC_LINE: usize = 512 * 1024;
const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: i16,
    y: i16,
    width: u16,
    height: u16,
}

impl Rect {
    fn point(self, x: f64, y: f64) -> (i32, i32) {
        let px = f64::from(self.x) + (f64::from(self.width) - 1.0).max(0.0) * x;
        let py = f64::from(self.y) + (f64::from(self.height) - 1.0).max(0.0) * y;
        (px.round() as i32, py.round() as i32)
    }
}

#[derive(Clone)]
struct XTarget {
    display: String,
    authority: PathBuf,
    number: u16,
    screen: usize,
}

impl XTarget {
    fn from_session(target: &SessionTarget) -> Result<Self> {
        let display = target
            .x11_display
            .as_deref()
            .context("selected X11 display is missing")?;
        let (number, screen) = local_display(display)?;
        let authority = target
            .xauthority
            .as_ref()
            .context("selected Xauthority path is missing")?;
        ensure!(
            authority.is_absolute() && authority.is_file(),
            "selected Xauthority file is missing or relative"
        );
        Ok(Self {
            display: display.to_owned(),
            authority: authority.clone(),
            number,
            screen,
        })
    }
    fn socket(&self) -> PathBuf {
        PathBuf::from(format!("/tmp/.X11-unix/X{}", self.number))
    }
}

fn local_display(display: &str) -> Result<(u16, usize)> {
    let value = display
        .strip_prefix(':')
        .context("only explicit local X11 displays are supported")?;
    let (number, screen) = value.split_once('.').unwrap_or((value, "0"));
    ensure!(
        !number.is_empty()
            && !screen.is_empty()
            && number.bytes().all(|c| c.is_ascii_digit())
            && screen.bytes().all(|c| c.is_ascii_digit()),
        "invalid X11 display"
    );
    Ok((number.parse()?, screen.parse()?))
}

/// Read Xauthority through xauth, without consulting the process environment.
fn auth_cookie(target: &XTarget) -> Result<(Vec<u8>, Vec<u8>)> {
    use tokio::io::AsyncReadExt;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let output = runtime.block_on(async {
        tokio::time::timeout(AUTH_TIMEOUT, async {
            let mut child = tokio::process::Command::new(xauth_program())
                .arg("-f")
                .arg(&target.authority)
                .arg("list")
                .arg(format!(":{}", target.number))
                .env_remove("DISPLAY")
                .env_remove("XAUTHORITY")
                .env_remove("WAYLAND_DISPLAY")
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .context("start xauth for selected display")?;
            let stdout = child.stdout.take().context("xauth stdout unavailable")?;
            let mut output = Vec::new();
            stdout
                .take((MAX_AUTH_OUTPUT + 1) as u64)
                .read_to_end(&mut output)
                .await?;
            ensure!(output.len() <= MAX_AUTH_OUTPUT, "xauth response too large");
            ensure!(
                child.wait().await?.success(),
                "xauth rejected selected authority file"
            );
            Ok::<_, anyhow::Error>(output)
        })
        .await
        .context("xauth query timed out")?
    })?;
    let output = String::from_utf8(output).context("xauth returned non-UTF8 data")?;
    let mut entries = Vec::new();
    for line in output.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() == 3 && fields[1] == "MIT-MAGIC-COOKIE-1" {
            let cookie = decode_hex(fields[2])?;
            ensure!(!cookie.is_empty(), "empty Xauthority cookie");
            entries.push(cookie);
        }
    }
    let first = entries
        .first()
        .context("no MIT-MAGIC-COOKIE-1 for selected X11 display")?;
    ensure!(
        entries.iter().all(|entry| entry == first),
        "ambiguous Xauthority credentials"
    );
    Ok((b"MIT-MAGIC-COOKIE-1".to_vec(), first.clone()))
}

fn xauth_program() -> PathBuf {
    for path in ["/usr/bin/xauth", "/usr/sbin/xauth"] {
        if Path::new(path).is_file() {
            return PathBuf::from(path);
        }
    }
    PathBuf::from("xauth")
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len().is_multiple_of(2) && value.len() <= 1024,
        "invalid Xauthority cookie length"
    );
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            Ok(hex(pair[0]).context("invalid Xauthority cookie")? * 16
                + hex(pair[1]).context("invalid Xauthority cookie")?)
        })
        .collect()
}

type XConnection = RustConnection<DefaultStream>;

fn connect(target: &XTarget, socket_slot: &Mutex<Option<UnixStream>>) -> Result<XConnection> {
    let (name, cookie) = auth_cookie(target)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let socket = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::UnixStream::connect(target.socket()),
        )
        .await
        .context("selected X11 socket connection timed out")?
        .context("connect selected X11 socket")?
        .into_std()
        .map_err(anyhow::Error::from)
    })?;
    *socket_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(socket.try_clone()?);
    let (stream, _) = DefaultStream::from_unix_stream(socket)?;
    let connection =
        RustConnection::connect_to_stream_with_auth_info(stream, target.screen, name, cookie)
            .context("authenticate selected X11 session")?;
    Ok(connection)
}

fn visual_and_format(
    connection: &XConnection,
    screen: usize,
) -> Result<(xproto::Visualtype, xproto::Format)> {
    let root = connection
        .setup()
        .roots
        .get(screen)
        .context("invalid X11 screen")?;
    let visual = root
        .allowed_depths
        .iter()
        .flat_map(|depth| &depth.visuals)
        .find(|visual| visual.visual_id == root.root_visual)
        .cloned()
        .context("root X11 visual unavailable")?;
    ensure!(
        visual.class == VisualClass::TRUE_COLOR,
        "unsupported X11 visual class"
    );
    let format = connection
        .setup()
        .pixmap_formats
        .iter()
        .find(|format| format.depth == root.root_depth)
        .copied()
        .context("root X11 pixel format unavailable")?;
    ensure!(
        matches!(format.bits_per_pixel, 16 | 24 | 32),
        "unsupported X11 bits per pixel"
    );
    ensure!(
        matches!(format.scanline_pad, 8 | 16 | 32 | 64),
        "unsupported X11 scanline padding"
    );
    ensure!(
        visual.red_mask & visual.green_mask == 0
            && visual.red_mask & visual.blue_mask == 0
            && visual.green_mask & visual.blue_mask == 0,
        "overlapping X11 color masks"
    );
    let pixel_mask = if format.bits_per_pixel == 32 {
        u32::MAX
    } else {
        (1u32 << format.bits_per_pixel) - 1
    };
    ensure!(
        (visual.red_mask | visual.green_mask | visual.blue_mask) & !pixel_mask == 0,
        "X11 visual masks exceed pixel format"
    );
    Ok((visual, format))
}

fn root_rect(connection: &XConnection, screen: usize) -> Result<Rect> {
    let root = connection
        .setup()
        .roots
        .get(screen)
        .context("invalid X11 screen")?
        .root;
    let geometry = connection.get_geometry(root)?.reply()?;
    ensure!(
        geometry.width > 0 && geometry.height > 0,
        "empty X11 root geometry"
    );
    Ok(Rect {
        x: 0,
        y: 0,
        width: geometry.width,
        height: geometry.height,
    })
}

fn output_rect(
    connection: &XConnection,
    screen: usize,
    output_name: &str,
    root: Rect,
) -> Result<Rect> {
    let window = connection.setup().roots[screen].root;
    connection
        .randr_query_version(1, 3)?
        .reply()
        .context("RandR unavailable")?;
    let resources = connection
        .randr_get_screen_resources_current(window)?
        .reply()?;
    let mut active = Vec::new();
    for output in resources.outputs {
        let info = connection.randr_get_output_info(output, 0)?.reply()?;
        if info.connection != randr::Connection::CONNECTED || info.crtc == 0 {
            continue;
        }
        let crtc = connection.randr_get_crtc_info(info.crtc, 0)?.reply()?;
        if crtc.width == 0 || crtc.height == 0 {
            continue;
        }
        let name = String::from_utf8_lossy(&info.name).into_owned();
        active.push((name, crtc.x, crtc.y, crtc.width, crtc.height));
    }
    let (_, x, y, width, height) = active
        .iter()
        .find(|(name, ..)| name == output_name)
        .with_context(|| format!("selected X11 output {output_name} is disconnected"))?;
    // RandR CRTC positions are already in root-window coordinates. An
    // unpopulated strip at the left/top remains part of the root image.
    crop_rect((i32::from(*x), i32::from(*y), *width, *height), root)
}

fn crop_rect((x, y, width, height): (i32, i32, u16, u16), root: Rect) -> Result<Rect> {
    ensure!(
        width > 0 && height > 0 && x >= 0 && y >= 0,
        "invalid selected output rectangle"
    );
    ensure!(
        x + i32::from(width) <= i32::from(root.width)
            && y + i32::from(height) <= i32::from(root.height),
        "selected output exceeds X11 root image"
    );
    Ok(Rect {
        x: i16::try_from(x).context("output X exceeds X11 capture range")?,
        y: i16::try_from(y).context("output Y exceeds X11 capture range")?,
        width,
        height,
    })
}

fn channel(pixel: u32, mask: u32) -> Result<u8> {
    ensure!(mask != 0, "empty X11 color mask");
    let shift = mask.trailing_zeros();
    let shifted = mask >> shift;
    ensure!(
        shifted & shifted.wrapping_add(1) == 0,
        "non-contiguous X11 color mask"
    );
    let value = (pixel & mask) >> shift;
    Ok(((value as u64 * 255 + shifted as u64 / 2) / shifted as u64) as u8)
}

fn image_rgba(
    reply: xproto::GetImageReply,
    width: u16,
    height: u16,
    order: ImageOrder,
    format: xproto::Format,
    visual: &xproto::Visualtype,
) -> Result<Arc<[u8]>> {
    ensure!(
        reply.depth == format.depth && reply.visual == visual.visual_id,
        "X11 image format changed unexpectedly"
    );
    image_rgba_data(&reply.data, width, height, order, format, visual)
}

fn image_rgba_data(
    data: &[u8],
    width: u16,
    height: u16,
    order: ImageOrder,
    format: xproto::Format,
    visual: &xproto::Visualtype,
) -> Result<Arc<[u8]>> {
    let bits = usize::from(width)
        .checked_mul(usize::from(format.bits_per_pixel))
        .context("X11 row overflow")?;
    let pad = usize::from(format.scanline_pad);
    let row = bits.checked_add(pad - 1).context("X11 row overflow")? / pad * (pad / 8);
    let expected = row
        .checked_mul(usize::from(height))
        .context("X11 image overflow")?;
    let wire_size = expected.checked_add(3).context("X11 image overflow")? & !3;
    ensure!(
        expected <= MAX_IMAGE_BYTES && data.len() == wire_size,
        "invalid X11 image size"
    );
    let len = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|count| count.checked_mul(4))
        .context("X11 image overflow")?;
    ensure!(len <= MAX_IMAGE_BYTES, "X11 image exceeds capture limit");
    let bytes_per_pixel = usize::from(format.bits_per_pixel / 8);
    let mut rgba = vec![0; len];
    let byte_channel = |mask: u32| {
        let shift = mask.trailing_zeros();
        if mask.checked_shr(shift) != Some(255) || !shift.is_multiple_of(8) {
            return None;
        }
        match order {
            ImageOrder::LSB_FIRST => Some(shift as usize / 8),
            ImageOrder::MSB_FIRST => Some(3 - shift as usize / 8),
            _ => None,
        }
    };
    if format.bits_per_pixel == 32
        && let (Some(red), Some(green), Some(blue)) = (
            byte_channel(visual.red_mask),
            byte_channel(visual.green_mask),
            byte_channel(visual.blue_mask),
        )
    {
        for (source, destination) in data[..expected]
            .chunks_exact(row)
            .zip(rgba.chunks_exact_mut(usize::from(width) * 4))
        {
            for (pixel, rgba) in source[..usize::from(width) * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(destination.as_chunks_mut::<4>().0)
            {
                rgba.copy_from_slice(&[pixel[red], pixel[green], pixel[blue], 255]);
            }
        }
        return Ok(rgba.into());
    }
    for (y, line) in data[..expected].chunks_exact(row).enumerate() {
        for (x, bytes) in line[..usize::from(width) * bytes_per_pixel]
            .chunks_exact(bytes_per_pixel)
            .enumerate()
        {
            let pixel = match order {
                ImageOrder::LSB_FIRST => bytes
                    .iter()
                    .enumerate()
                    .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (i * 8)),
                ImageOrder::MSB_FIRST => {
                    bytes.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b))
                }
                _ => bail!("unsupported X11 image byte order"),
            };
            let index = (y * usize::from(width) + x) * 4;
            rgba[index] = channel(pixel, visual.red_mask)?;
            rgba[index + 1] = channel(pixel, visual.green_mask)?;
            rgba[index + 2] = channel(pixel, visual.blue_mask)?;
            rgba[index + 3] = 255;
        }
    }
    Ok(rgba.into())
}

fn capture_image(
    connection: &XConnection,
    target: &XTarget,
    output: Option<&str>,
) -> Result<(Rect, Arc<[u8]>)> {
    let root = root_rect(connection, target.screen)?;
    let rect = match output {
        Some(name) => output_rect(connection, target.screen, name, root)?,
        None => root,
    };
    let (visual, format) = visual_and_format(connection, target.screen)?;
    let image_bytes = usize::from(rect.width) * usize::from(rect.height) * 4;
    ensure!(
        image_bytes <= MAX_IMAGE_BYTES,
        "X11 image exceeds capture limit"
    );
    let drawable = connection.setup().roots[target.screen].root;
    let reply = connection
        .get_image(
            ImageFormat::Z_PIXMAP,
            drawable,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            u32::MAX,
        )?
        .reply()?;
    let pixels = image_rgba(
        reply,
        rect.width,
        rect.height,
        connection.setup().image_byte_order,
        format,
        &visual,
    )?;
    Ok((rect, pixels))
}

/// A bounded probe establishes authentication and checks capture format and XTest.
pub fn probe(target: &SessionTarget) -> Result<()> {
    let target = XTarget::from_session(target)?;
    let socket = Arc::new(Mutex::new(None));
    let worker_socket = socket.clone();
    let (sender, receiver) = sync_mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let result = (|| -> Result<()> {
            let connection = connect(&target, &worker_socket)?;
            connection
                .xtest_get_version(2, 2)?
                .reply()
                .context("XTest unavailable")?;
            root_rect(&connection, target.screen)?;
            let (visual, format) = visual_and_format(&connection, target.screen)?;
            let root = connection.setup().roots[target.screen].root;
            let pixel = connection
                .get_image(ImageFormat::Z_PIXMAP, root, 0, 0, 1, 1, u32::MAX)?
                .reply()
                .context("X11 root capture unavailable")?;
            image_rgba(
                pixel,
                1,
                1,
                connection.setup().image_byte_order,
                format,
                &visual,
            )
            .context("X11 root capture has unsupported pixels")?;
            Ok(())
        })();
        let _ = sender.send(result);
    });
    let result = receiver.recv_timeout(START_TIMEOUT);
    if result.is_err() {
        // The socket can appear just after the deadline; interrupt it until
        // the probe exits, then detach only if the server never cooperates.
        let mut exited = false;
        for _ in 0..20 {
            if let Some(socket) = socket.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = socket.shutdown(Shutdown::Both);
            }
            if receiver.recv_timeout(Duration::from_millis(50)).is_ok() {
                exited = true;
                break;
            }
        }
        if exited {
            let _ = worker.join();
        }
        bail!("X11 probe timed out");
    }
    let _ = worker.join();
    result.context("X11 probe failed")?
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireRequest {
    Input { command: WireInput },
    ClipboardRead,
    ClipboardWrite { text: String },
    ActiveApp,
    ClipboardWritePng { path: PathBuf },
    ClipboardReadPng { path: PathBuf },
    ClipboardRevision,
    Release,
    Stop,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireInput {
    Move { x: i32, y: i32 },
    Button { button: u32, pressed: bool },
    Key { code: u32, pressed: bool },
    Scroll { x: f64, y: f64 },
    Text { text: String },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireReply {
    Ready,
    Ok { text: Option<String> },
    Error { message: String },
    Revision { value: u64 },
}

fn poll_fd(fd: i32, events: i16, deadline: Instant) -> io::Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "X11 worker IPC timed out",
            ));
        }
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let timeout = remaining.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: descriptor is a valid local pollfd and the call does not retain it.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout.max(1)) };
        if result > 0 {
            return Ok(());
        }
        if result == 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn nonblocking(fd: i32) -> io::Result<()> {
    // SAFETY: fcntl only modifies flags on this owned pipe descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct LineReader {
    pipe: ChildStdout,
    pending: Vec<u8>,
}
impl LineReader {
    fn new(pipe: ChildStdout) -> Result<Self> {
        nonblocking(pipe.as_raw_fd())?;
        Ok(Self {
            pipe,
            pending: Vec::new(),
        })
    }
    fn read(&mut self, deadline: Instant) -> Result<WireReply> {
        loop {
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<_> = self.pending.drain(..=end).collect();
                return serde_json::from_slice(&line).context("invalid X11 worker response");
            }
            ensure!(
                self.pending.len() < MAX_IPC_LINE,
                "X11 worker response too large"
            );
            poll_fd(self.pipe.as_raw_fd(), libc::POLLIN, deadline)?;
            let mut chunk = [0u8; 8192];
            match self.pipe.read(&mut chunk) {
                Ok(0) => bail!("X11 worker closed response pipe"),
                Ok(count) => self.pending.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) => return Err(error.into()),
            }
            ensure!(
                self.pending.len() <= MAX_IPC_LINE,
                "X11 worker response too large"
            );
        }
    }
}

fn write_request(pipe: &mut ChildStdin, request: &WireRequest, deadline: Instant) -> Result<()> {
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    ensure!(bytes.len() <= MAX_IPC_LINE, "X11 worker request too large");
    let mut sent = 0;
    while sent < bytes.len() {
        poll_fd(pipe.as_raw_fd(), libc::POLLOUT, deadline)?;
        match pipe.write(&bytes[sent..]) {
            Ok(0) => bail!("X11 worker input pipe closed"),
            Ok(count) => sent += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

struct WorkerProcess {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    stdout: LineReader,
}
impl WorkerProcess {
    fn launch_executable(target: &XTarget, executable: &Path) -> Result<Self> {
        let mut child = Command::new(executable)
            .arg("--internal-x11-worker")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env("DISPLAY", &target.display)
            .env("XAUTHORITY", &target.authority)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("start X11 input worker")?;
        let stdin = child.stdin.take().context("X11 worker stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("X11 worker stdout unavailable")?;
        if let Err(error) = nonblocking(stdin.as_raw_fd()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let stdout = match LineReader::new(stdout) {
            Ok(stdout) => stdout,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut process = Self {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
        };
        match process.stdout.read(Instant::now() + START_TIMEOUT) {
            Ok(WireReply::Ready) => Ok(process),
            Ok(WireReply::Error { message }) => {
                process.kill();
                bail!("X11 worker startup: {message}")
            }
            Ok(_) => {
                process.kill();
                bail!("X11 worker did not report readiness")
            }
            Err(error) => {
                process.kill();
                Err(error.context("X11 worker startup"))
            }
        }
    }
    fn exchange(&mut self, request: &WireRequest, timeout: Duration) -> Result<WireReply> {
        let deadline = Instant::now() + timeout;
        write_request(&mut self.stdin, request, deadline)?;
        match self.stdout.read(deadline)? {
            response @ (WireReply::Ok { .. }
            | WireReply::Error { .. }
            | WireReply::Revision { .. }) => Ok(response),
            WireReply::Ready => bail!("unexpected X11 worker readiness response"),
        }
    }
    fn kill(&mut self) {
        let _ = self.child.lock().unwrap_or_else(|e| e.into_inner()).kill();
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        self.kill();
        let _ = self.child.lock().unwrap_or_else(|e| e.into_inner()).wait();
    }
}

enum QueueItem {
    Stop,
    Input(InputCommand),
    ClipboardRead(oneshot::Sender<Result<String>>),
    ClipboardWrite(String, oneshot::Sender<Result<()>>),
    ActiveApp(oneshot::Sender<Result<String>>),
    ClipboardWritePng(tempfile::NamedTempFile, oneshot::Sender<Result<()>>),
    ClipboardReadPng(
        tempfile::NamedTempFile,
        oneshot::Sender<Result<Option<Vec<u8>>>>,
    ),
    ClipboardRevision(oneshot::Sender<Result<u64>>),
}

struct Shared {
    stopped: AtomicBool,
    frames: watch::Sender<Option<Arc<Frame>>>,
    capture_socket: Mutex<Option<UnixStream>>,
    rect: Mutex<Option<Rect>>,
    cursors: watch::Sender<Option<Arc<CursorFrame>>>,
    cursor_supported: AtomicBool,
    clipboard_supported: AtomicBool,
    cursor_owner: AtomicU64,
}
impl Shared {
    fn finish(&self) {
        {
            let mut rect = self.rect.lock().unwrap_or_else(|e| e.into_inner());
            self.stopped.store(true, Ordering::Release);
            *rect = None;
            self.frames.send_replace(None);
            self.cursors.send_replace(None);
        }
        if let Some(socket) = self
            .capture_socket
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    fn refresh(&self, rect: Rect, pixels: Arc<[u8]>, embedded: bool) -> bool {
        let mut current_rect = self.rect.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        *current_rect = Some(rect);
        // Geometry was read successfully: static pixels are healthy, without
        // a full-screen comparison or a new encode notification.
        self.frames.send_if_modified(|slot| {
            let mut frame = Frame::new(u32::from(rect.width), u32::from(rect.height), pixels);
            frame.cursor_embedded = embedded;
            *slot = Some(Arc::new(frame));
            false
        });
        true
    }
    fn publish(&self, rect: Rect, pixels: Arc<[u8]>, embedded: bool) -> bool {
        let mut current_rect = self.rect.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        *current_rect = Some(rect);
        let changed = self.frames.borrow().as_ref().is_none_or(|previous| {
            previous.width != u32::from(rect.width)
                || previous.height != u32::from(rect.height)
                || previous.pixels.as_ref() != pixels.as_ref()
                || previous.cursor_embedded != embedded
        });
        let mut frame = Frame::new(u32::from(rect.width), u32::from(rect.height), pixels);
        frame.cursor_embedded = embedded;
        let frame = Arc::new(frame);
        self.frames.send_if_modified(|slot| {
            *slot = Some(frame);
            changed
        });
        true
    }
}

pub struct Backend {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    input: mpsc::Sender<InputCommand>,
    queue: mpsc::Sender<QueueItem>,
    shared: Arc<Shared>,
    child: Arc<Mutex<Child>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    io_done: Mutex<sync_mpsc::Receiver<()>>,
    clipboard_directory: tempfile::TempDir,
}

impl Backend {
    pub fn start(
        target: SessionTarget,
        fps: u32,
        quality: u8,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::start_with_executable(
            target,
            fps,
            quality,
            output,
            cancelled,
            &std::env::current_exe().context("find X11 worker executable")?,
        )
    }

    fn start_with_executable(
        target: SessionTarget,
        fps: u32,
        quality: u8,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
        executable: &Path,
    ) -> Result<Self> {
        Self::start_pipeline(target, fps, quality, output, cancelled, executable, true)
    }

    #[cfg(feature = "portal")]
    pub(crate) fn clipboard_only(
        target: SessionTarget,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::start_pipeline(
            target,
            1,
            80,
            None,
            cancelled,
            &std::env::current_exe()?,
            false,
        )
    }

    pub(crate) async fn active_app(&self) -> Result<String> {
        let (done, response) = oneshot::channel();
        tokio::time::timeout(REQUEST_TIMEOUT, self.queue.send(QueueItem::ActiveApp(done)))
            .await??;
        tokio::time::timeout(REQUEST_TIMEOUT, response).await??
    }

    #[allow(clippy::too_many_arguments)] // One pinned worker can serve clipboard without capturing pixels.
    fn start_pipeline(
        target: SessionTarget,
        fps: u32,
        quality: u8,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
        executable: &Path,
        capture: bool,
    ) -> Result<Self> {
        ensure!((1..=60).contains(&fps), "FPS must be between 1 and 60");
        ensure!(
            (1..=100).contains(&quality),
            "JPEG quality must be between 1 and 100"
        );
        ensure!(!cancelled.load(Ordering::Acquire), "X11 startup cancelled");
        let target = XTarget::from_session(&target)?;
        let process = WorkerProcess::launch_executable(&target, executable)?;
        let child = process.child.clone();
        let (frames_tx, frames) = watch::channel(None);
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            frames: frames_tx,
            capture_socket: Mutex::new(None),
            rect: Mutex::new(None),
            cursors: watch::channel(None).0,
            cursor_supported: AtomicBool::new(false),
            clipboard_supported: AtomicBool::new(false),
            cursor_owner: AtomicU64::new(0),
        });
        let (input, input_rx) = mpsc::channel(128);
        let (queue, queue_rx) = mpsc::channel(128);
        let (ready_tx, ready_rx) = sync_mpsc::sync_channel(1);
        let (done_tx, io_done) = sync_mpsc::sync_channel(1);
        let mut backend = Self {
            frames,
            input,
            queue: queue.clone(),
            shared: shared.clone(),
            child,
            workers: Mutex::new(Vec::new()),
            io_done: Mutex::new(io_done),
            clipboard_directory: tempfile::Builder::new()
                .prefix("lrd-x11-clipboard-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir()?,
        };

        let io_shared = shared.clone();
        backend.workers.get_mut().unwrap().push(
            thread::Builder::new()
                .name("x11-worker-ipc".into())
                .spawn(move || {
                    if let Err(error) = run_ipc(process, queue_rx, &io_shared)
                        && !io_shared.stopped.load(Ordering::Acquire)
                    {
                        tracing::error!("X11 input worker failed: {error:#}");
                        io_shared.finish();
                    }
                    let _ = done_tx.send(());
                })?,
        );
        let pump_shared = shared.clone();
        backend.workers.get_mut().unwrap().push(
            thread::Builder::new()
                .name("x11-input-queue".into())
                .spawn(move || {
                    run_input_queue(input_rx, queue, &pump_shared);
                })?,
        );
        if capture {
            let capture_shared = shared.clone();
            backend.workers.get_mut().unwrap().push(
                thread::Builder::new()
                    .name("x11-capture".into())
                    .spawn(move || {
                        if let Err(error) =
                            run_capture(&target, output.as_deref(), fps, &capture_shared, &ready_tx)
                            && !capture_shared.stopped.load(Ordering::Acquire)
                        {
                            let _ = ready_tx.try_send(Err(format!("{error:#}")));
                            tracing::error!("X11 capture failed: {error:#}");
                            capture_shared.finish();
                        }
                    })?,
            );
        } else {
            let connection = connect(&target, &Mutex::new(None))?;
            shared
                .clipboard_supported
                .store(cursor::supported(&connection), Ordering::Release);
            ready_tx
                .send(Ok(()))
                .map_err(|_| anyhow::anyhow!("clipboard worker readiness closed"))?;
        }
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            ensure!(!cancelled.load(Ordering::Acquire), "X11 startup cancelled");
            ensure!(
                !backend.shared.stopped.load(Ordering::Acquire),
                "X11 backend stopped during startup"
            );
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(!remaining.is_zero(), "X11 capture startup timed out");
            match ready_rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(Ok(())) => break,
                Ok(Err(error)) => bail!("X11 capture startup: {error}"),
                Err(sync_mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => bail!("X11 capture startup: {error}"),
            }
        }
        ensure!(
            !capture || backend.frames.borrow().is_some(),
            "X11 capture produced no frame"
        );
        Ok(backend)
    }

    pub fn stop(&self) {
        self.shared.finish();
        let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        if workers.is_empty() {
            return;
        }
        // Wake an idle IPC receiver. A full queue already has work to wake it;
        // the stop flag is checked again before any queued item is executed.
        let _ = self.queue.try_send(QueueItem::Stop);
        for worker in workers.iter() {
            worker.thread().unpark();
        }
        if self
            .io_done
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recv_timeout(Duration::from_secs(2))
            .is_err()
        {
            let _ = self.child.lock().unwrap_or_else(|e| e.into_inner()).kill();
        }
        for worker in workers.drain(..) {
            let _ = worker.join();
        }
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

impl DesktopBackend for Backend {
    fn text_paste_keycodes(&self, shift: bool) -> BoxFuture<'_, Vec<u32>> {
        Box::pin(async move {
            let terminal = self
                .active_app()
                .await
                .is_ok_and(|app| super::terminal_app(&app));
            super::linux_text_paste_keys(shift, terminal)
        })
    }
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }
    fn cursor_embedded(&self) -> bool {
        false
    }
    fn cursor_frames(&self) -> Option<watch::Receiver<Option<Arc<CursorFrame>>>> {
        self.shared
            .cursor_supported
            .load(Ordering::Acquire)
            .then(|| self.shared.cursors.subscribe())
    }
    fn begin_cursor(&self, owner: u64) {
        self.shared
            .cursor_owner
            .store(owner << 1, Ordering::Release);
    }
    fn separate_cursor(&self, owner: u64) {
        let _ = self.shared.cursor_owner.compare_exchange(
            owner << 1,
            (owner << 1) | 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    fn release_cursor(&self, owner: u64) {
        let _ = self.shared.cursor_owner.compare_exchange(
            (owner << 1) | 1,
            owner << 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    fn supports_image_clipboard(&self) -> bool {
        true
    }
    fn supports_clipboard_sync(&self) -> bool {
        self.shared.clipboard_supported.load(Ordering::Acquire)
    }
    fn clipboard_revision(&self) -> BoxFuture<'_, Result<ClipboardRevision>> {
        Box::pin(async move {
            let (done, completed) = oneshot::channel();
            self.queue
                .send(QueueItem::ClipboardRevision(done))
                .await
                .context("X11 clipboard worker closed")?;
            let serial = tokio::time::timeout(CLIPBOARD_TIMEOUT, completed)
                .await
                .context("X11 clipboard revision timed out")?
                .context("X11 clipboard worker closed")??;
            Ok(ClipboardRevision { epoch: 0, serial })
        })
    }
    fn clipboard_write_png<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ensure!(data.len() <= 8 * 1024 * 1024, "clipboard image too large");
            let mut file = tempfile::NamedTempFile::new_in(self.clipboard_directory.path())?;
            file.write_all(data)?;
            let (done, completed) = oneshot::channel();
            self.queue
                .send(QueueItem::ClipboardWritePng(file, done))
                .await
                .context("X11 clipboard worker closed")?;
            tokio::time::timeout(CLIPBOARD_TIMEOUT, completed)
                .await
                .context("X11 image clipboard write timed out")?
                .context("X11 clipboard worker closed")?
        })
    }
    fn clipboard_read_png(&self) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        Box::pin(async move {
            let file = tempfile::NamedTempFile::new_in(self.clipboard_directory.path())?;
            let (done, completed) = oneshot::channel();
            self.queue
                .send(QueueItem::ClipboardReadPng(file, done))
                .await
                .context("X11 clipboard worker closed")?;
            tokio::time::timeout(CLIPBOARD_TIMEOUT, completed)
                .await
                .context("X11 image clipboard read timed out")?
                .context("X11 clipboard worker closed")?
        })
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn stop(&self) {
        Backend::stop(self)
    }
    fn frame_is_fresh(&self, max_age: Duration) -> bool {
        !self.shared.stopped.load(Ordering::Acquire)
            && self
                .frames
                .borrow()
                .as_ref()
                .is_some_and(|frame| frame.captured_at.elapsed() <= max_age)
    }
    fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }
    fn release_input(&self) -> bool {
        let (done, completed) = sync_mpsc::sync_channel(1);
        self.input
            .try_send(InputCommand::ReleaseSync { done })
            .is_ok()
            && completed.recv_timeout(REQUEST_TIMEOUT).is_ok()
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async move {
            ensure!(
                !self.shared.stopped.load(Ordering::Acquire),
                "X11 backend stopped"
            );
            let (tx, rx) = oneshot::channel();
            tokio::time::timeout(
                CLIPBOARD_TIMEOUT,
                self.queue.send(QueueItem::ClipboardRead(tx)),
            )
            .await
            .context("X11 clipboard queue timed out")?
            .context("X11 worker queue closed")?;
            tokio::time::timeout(CLIPBOARD_TIMEOUT + Duration::from_secs(1), rx)
                .await
                .context("X11 clipboard read timed out")?
                .context("X11 worker stopped")?
        })
    }
    fn clipboard_write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ensure!(text.len() <= MAX_CLIPBOARD_BYTES, "clipboard too large");
            ensure!(
                !self.shared.stopped.load(Ordering::Acquire),
                "X11 backend stopped"
            );
            let (tx, rx) = oneshot::channel();
            tokio::time::timeout(
                CLIPBOARD_TIMEOUT,
                self.queue
                    .send(QueueItem::ClipboardWrite(text.to_owned(), tx)),
            )
            .await
            .context("X11 clipboard queue timed out")?
            .context("X11 worker queue closed")?;
            tokio::time::timeout(CLIPBOARD_TIMEOUT + Duration::from_secs(1), rx)
                .await
                .context("X11 clipboard write timed out")?
                .context("X11 worker stopped")?
        })
    }
}

fn run_capture(
    target: &XTarget,
    output: Option<&str>,
    fps: u32,
    shared: &Shared,
    ready: &sync_mpsc::SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    let connection = connect(target, &shared.capture_socket)?;
    let period = Duration::from_secs_f64(1.0 / f64::from(fps));
    let mut first = true;
    let mut capture = capture::Capture::new(&connection, target, output);
    let cursor_supported = cursor::supported(&connection);
    shared
        .clipboard_supported
        .store(cursor_supported, Ordering::Release);
    shared
        .cursor_supported
        .store(cursor_supported, Ordering::Release);
    let mut previous_cursor: Option<Arc<CursorFrame>> = None;
    let mut resize_retry: Option<Instant> = None;
    while !shared.stopped.load(Ordering::Acquire) {
        let started = Instant::now();
        let (rect, pixels, acquired) = match capture.next() {
            Ok(frame) => {
                resize_retry = None;
                frame
            }
            Err(error) if capture::resize_race(&error) => {
                let since = resize_retry.get_or_insert(started);
                if since.elapsed() >= Duration::from_secs(2) {
                    return Err(error);
                }
                thread::park_timeout(period.min(Duration::from_millis(16)));
                continue;
            }
            Err(error) => return Err(error),
        };
        let next_cursor = if cursor_supported {
            match cursor::read(&connection, rect, previous_cursor.as_deref()) {
                Ok(cursor) => Some(Arc::new(cursor)),
                // X11 can deny the current cursor image during a grab or
                // desktop startup. Keep the independent shape and retry;
                // this is not a reason to restart the desktop and its apps.
                Err(error) if error.downcast_ref::<x11rb::errors::ReplyError>()
                    .is_some_and(|error| matches!(error, x11rb::errors::ReplyError::X11Error(reply) if reply.error_kind == x11rb::protocol::ErrorKind::Access)) => previous_cursor.clone(),
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        let cursor_changed = previous_cursor != next_cursor;
        if cursor_changed {
            shared.cursors.send_replace(next_cursor.clone());
        }
        if !if acquired {
            shared.publish(rect, pixels, false)
        } else {
            shared.refresh(rect, pixels, false)
        } {
            break;
        }
        previous_cursor = next_cursor;
        if first {
            let _ = ready.send(Ok(()));
            first = false;
        }
        thread::park_timeout(period.saturating_sub(started.elapsed()));
    }
    Ok(())
}

fn run_input_queue(
    mut input: mpsc::Receiver<InputCommand>,
    queue: mpsc::Sender<QueueItem>,
    shared: &Shared,
) {
    while !shared.stopped.load(Ordering::Acquire) {
        match input.try_recv() {
            Ok(command) => {
                if let Err(error) = command.validate() {
                    tracing::warn!(%error, "ignoring invalid X11 input");
                    continue;
                }
                if queue.blocking_send(QueueItem::Input(command)).is_err() {
                    break;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                thread::park_timeout(Duration::from_millis(10))
            }
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
}

fn run_ipc(
    mut process: WorkerProcess,
    mut queue: mpsc::Receiver<QueueItem>,
    shared: &Shared,
) -> Result<()> {
    let result = (|| -> Result<()> {
        loop {
            if shared.stopped.load(Ordering::Acquire) {
                let _ = process.exchange(&WireRequest::Stop, REQUEST_TIMEOUT);
                break;
            }
            let Some(item) = queue.blocking_recv() else {
                break;
            };
            if shared.stopped.load(Ordering::Acquire) || matches!(item, QueueItem::Stop) {
                let _ = process.exchange(&WireRequest::Stop, REQUEST_TIMEOUT);
                break;
            }
            match item {
                QueueItem::Stop => unreachable!(),
                QueueItem::Input(command) => process_input(&mut process, shared, command)?,
                QueueItem::ClipboardRead(reply) => {
                    if reply.is_closed() {
                        continue;
                    }
                    let result =
                        match process.exchange(&WireRequest::ClipboardRead, CLIPBOARD_TIMEOUT)? {
                            WireReply::Ok { text: Some(text) } => {
                                if text.len() <= MAX_CLIPBOARD_BYTES {
                                    Ok(text)
                                } else {
                                    Err(anyhow::anyhow!("clipboard too large"))
                                }
                            }
                            WireReply::Ok { text: None } => {
                                Err(anyhow::anyhow!("X11 clipboard returned no text"))
                            }
                            WireReply::Error { message } => Err(anyhow::anyhow!(message)),
                            WireReply::Ready | WireReply::Revision { .. } => unreachable!(),
                        };
                    let _ = reply.send(result);
                }
                QueueItem::ClipboardWrite(text, reply) => {
                    // GuardedBackend drops the receiver on lock or cancellation.
                    // A queued write must not acquire clipboard ownership later.
                    if reply.is_closed() {
                        continue;
                    }
                    let result = match process
                        .exchange(&WireRequest::ClipboardWrite { text }, CLIPBOARD_TIMEOUT)?
                    {
                        WireReply::Ok { .. } => Ok(()),
                        WireReply::Error { message } => Err(anyhow::anyhow!(message)),
                        WireReply::Ready | WireReply::Revision { .. } => unreachable!(),
                    };
                    let _ = reply.send(result);
                }
                QueueItem::ActiveApp(reply) => {
                    if reply.is_closed() {
                        continue;
                    }
                    let result = match process.exchange(&WireRequest::ActiveApp, REQUEST_TIMEOUT)? {
                        WireReply::Ok { text } => Ok(text.unwrap_or_default()),
                        WireReply::Error { message } => Err(anyhow::anyhow!(message)),
                        _ => bail!("unexpected focus query reply"),
                    };
                    let _ = reply.send(result);
                }
                QueueItem::ClipboardRevision(reply) => {
                    if reply.is_closed() {
                        continue;
                    }
                    let result =
                        match process.exchange(&WireRequest::ClipboardRevision, REQUEST_TIMEOUT)? {
                            WireReply::Revision { value } => Ok(value),
                            WireReply::Error { message } => Err(anyhow::anyhow!(message)),
                            _ => bail!("unexpected clipboard revision reply"),
                        };
                    let _ = reply.send(result);
                }
                QueueItem::ClipboardWritePng(file, reply) => {
                    if reply.is_closed() {
                        continue;
                    }
                    let result = match process.exchange(
                        &WireRequest::ClipboardWritePng {
                            path: file.path().to_path_buf(),
                        },
                        CLIPBOARD_TIMEOUT,
                    )? {
                        WireReply::Ok { .. } => Ok(()),
                        WireReply::Error { message } => Err(anyhow::anyhow!(message)),
                        _ => bail!("unexpected image clipboard reply"),
                    };
                    let _ = reply.send(result);
                }
                QueueItem::ClipboardReadPng(file, reply) => {
                    if reply.is_closed() {
                        continue;
                    }
                    let result = match process.exchange(
                        &WireRequest::ClipboardReadPng {
                            path: file.path().to_path_buf(),
                        },
                        CLIPBOARD_TIMEOUT,
                    )? {
                        WireReply::Ok { text: Some(_) } => {
                            let bytes = std::fs::read(file.path())?;
                            ensure!(bytes.len() <= 8 * 1024 * 1024, "clipboard image too large");
                            Ok(Some(bytes))
                        }
                        WireReply::Ok { text: None } => Ok(None),
                        WireReply::Error { message } => Err(anyhow::anyhow!(message)),
                        _ => bail!("unexpected image clipboard reply"),
                    };
                    let _ = reply.send(result);
                }
            }
        }
        Ok(())
    })();
    if result.is_err() {
        process.kill();
    }
    result
}

fn process_input(
    process: &mut WorkerProcess,
    shared: &Shared,
    command: InputCommand,
) -> Result<()> {
    let (command, guard) = match command {
        InputCommand::Guarded { command, guard } if guard.active() => (*command, Some(guard)),
        InputCommand::Guarded { guard, .. } => {
            guard.text_done.notify_one();
            return Ok(());
        }
        command => (command, None),
    };
    if shared.stopped.load(Ordering::Acquire) {
        return Ok(());
    }
    match command {
        InputCommand::ReleaseSync { done } => {
            if matches!(
                process.exchange(&WireRequest::Release, REQUEST_TIMEOUT)?,
                WireReply::Ok { .. }
            ) {
                let _ = done.send(());
            }
        }
        InputCommand::Release => {
            if let WireReply::Error { message } =
                process.exchange(&WireRequest::Release, REQUEST_TIMEOUT)?
            {
                tracing::warn!("X11 input release failed: {message}");
            }
        }
        InputCommand::Chord { codes } => {
            match process.exchange(&WireRequest::Release, REQUEST_TIMEOUT)? {
                WireReply::Ok { .. } => {}
                _ => bail!("X11 input release before chord failed"),
            }
            let result = (|| -> Result<()> {
                for code in codes {
                    if shared.stopped.load(Ordering::Acquire)
                        || guard.as_ref().is_some_and(|guard| !guard.active())
                    {
                        break;
                    }
                    send_input(
                        process,
                        WireInput::Key {
                            code,
                            pressed: true,
                        },
                    )?;
                }
                Ok(())
            })();
            let released = process.exchange(&WireRequest::Release, REQUEST_TIMEOUT);
            result?;
            ensure!(
                matches!(released?, WireReply::Ok { .. }),
                "X11 chord release failed"
            );
        }
        InputCommand::Text { text } => {
            let result = (|| -> Result<()> {
                match process.exchange(&WireRequest::Release, REQUEST_TIMEOUT)? {
                    WireReply::Ok { .. } => {}
                    WireReply::Error { message } => {
                        bail!("X11 input release before text: {message}")
                    }
                    WireReply::Ready | WireReply::Revision { .. } => unreachable!(),
                }
                let chars: Vec<char> = text.chars().collect();
                // A physical-session lock cancels the guard between individual
                // characters. The worker IPC call itself cannot be interrupted.
                let batch_size = if guard.is_some() { 1 } else { 16 };
                for batch in chars.chunks(batch_size) {
                    if shared.stopped.load(Ordering::Acquire)
                        || guard.as_ref().is_some_and(|g| !g.active())
                    {
                        break;
                    }
                    let text: String = batch.iter().collect();
                    if let WireReply::Error { message } = process.exchange(
                        &WireRequest::Input {
                            command: WireInput::Text { text },
                        },
                        REQUEST_TIMEOUT,
                    )? {
                        bail!("X11 Unicode input: {message}");
                    }
                    #[cfg(test)]
                    if let Some(probe) = guard.as_ref().and_then(|g| g.probe.as_ref()) {
                        probe.characters.fetch_add(batch.len(), Ordering::AcqRel);
                        if probe.batches.fetch_add(1, Ordering::AcqRel) == 0 {
                            let _ = probe.first_batch.send(());
                            let _ = probe
                                .resume
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .recv_timeout(Duration::from_secs(5));
                        }
                    }
                }
                Ok(())
            })();
            if let Some(guard) = guard {
                #[cfg(test)]
                if let Some(probe) = &guard.probe {
                    probe.restored.store(true, Ordering::Release);
                }
                guard.text_done.notify_one();
            }
            if let Err(ref error) = result {
                tracing::warn!("X11 Unicode input failed: {error:#}");
            }
            result?;
        }
        InputCommand::Move { x, y } => {
            let rect = *shared.rect.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(rect) = rect {
                let (x, y) = rect.point(x, y);
                if let WireReply::Error { message } = process.exchange(
                    &WireRequest::Input {
                        command: WireInput::Move { x, y },
                    },
                    REQUEST_TIMEOUT,
                )? {
                    tracing::warn!("X11 pointer input failed: {message}");
                }
            }
        }
        InputCommand::Button { button, pressed } => {
            send_input(process, WireInput::Button { button, pressed })?
        }
        InputCommand::Key { code, pressed } => {
            send_input(process, WireInput::Key { code, pressed })?
        }
        InputCommand::Scroll { x, y } => send_input(process, WireInput::Scroll { x, y })?,
        InputCommand::Guarded { .. } => unreachable!(),
    }
    Ok(())
}

fn send_input(process: &mut WorkerProcess, command: WireInput) -> Result<()> {
    if let WireReply::Error { message } =
        process.exchange(&WireRequest::Input { command }, REQUEST_TIMEOUT)?
    {
        tracing::warn!("X11 input rejected: {message}");
    }
    Ok(())
}

struct WorkerState {
    settings: Settings,
    enigo: Option<Enigo>,
    clipboard: arboard::Clipboard,
    mapped: [bool; 256],
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u32>,
    clipboard_connection: XConnection,
    clipboard_selection: u32,
    clipboard_revision: u64,
}

impl WorkerState {
    fn new(display: String) -> Result<Self> {
        let settings = Settings {
            x11_display: Some(display.clone()),
            ..Settings::default()
        };
        let enigo = Enigo::new(&settings)
            .map_err(|error| anyhow::anyhow!("create X11 input connection: {error}"))?;
        let clipboard = arboard::Clipboard::new().context("create X11 clipboard connection")?;
        let (connection, screen) =
            x11rb::connect(Some(&display)).context("query X11 keyboard mapping")?;
        ensure!(
            connection.setup().roots.get(screen).is_some(),
            "invalid X11 worker screen"
        );
        let min = connection.setup().min_keycode;
        let max = connection.setup().max_keycode;
        let reply = connection
            .get_keyboard_mapping(min, max - min + 1)?
            .reply()?;
        let stride = usize::from(reply.keysyms_per_keycode);
        ensure!(stride > 0, "empty X11 keyboard mapping");
        let mut mapped = [false; 256];
        for (index, symbols) in reply.keysyms.chunks(stride).enumerate() {
            if symbols.iter().any(|symbol| *symbol != 0) {
                mapped[usize::from(min) + index] = true;
            }
        }
        let clipboard_selection = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        if cursor::supported(&connection) {
            connection
                .xfixes_select_selection_input(
                    connection.setup().roots[screen].root,
                    clipboard_selection,
                    xfixes::SelectionEventMask::SET_SELECTION_OWNER
                        | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
                        | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
                )?
                .check()?;
        }
        Ok(Self {
            settings,
            enigo: Some(enigo),
            clipboard,
            mapped,
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            clipboard_connection: connection,
            clipboard_selection,
            clipboard_revision: 0,
        })
    }
    fn enigo(&mut self) -> Result<&mut Enigo> {
        if self.enigo.is_none() {
            self.enigo = Some(
                Enigo::new(&self.settings)
                    .map_err(|error| anyhow::anyhow!("reconnect X11 input: {error}"))?,
            );
        }
        Ok(self.enigo.as_mut().unwrap())
    }
    fn release(&mut self) -> Result<()> {
        let mut first_error = None;
        if let Some(mut enigo) = self.enigo.take() {
            for code in std::mem::take(&mut self.keys) {
                if let Err(error) = enigo.raw(code, Direction::Release) {
                    first_error
                        .get_or_insert_with(|| anyhow::anyhow!("release X11 key {code}: {error}"));
                }
            }
            for button in std::mem::take(&mut self.buttons) {
                if let Err(error) = enigo.button(mouse_button(button)?, Direction::Release) {
                    first_error.get_or_insert_with(|| {
                        anyhow::anyhow!("release X11 button {button}: {error}")
                    });
                }
            }
            drop(enigo);
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }
    fn input(&mut self, command: WireInput) -> Result<()> {
        match command {
            WireInput::Move { x, y } => self
                .enigo()?
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(|error| anyhow::anyhow!("move X11 pointer: {error}")),
            WireInput::Button { button, pressed } => {
                let mapped = mouse_button(button)?;
                let changed = self.buttons.contains(&button) != pressed;
                if changed {
                    self.enigo()?
                        .button(
                            mapped,
                            if pressed {
                                Direction::Press
                            } else {
                                Direction::Release
                            },
                        )
                        .map_err(|error| anyhow::anyhow!("X11 mouse button: {error}"))?;
                    if pressed {
                        self.buttons.insert(button);
                    } else {
                        self.buttons.remove(&button);
                    }
                }
                Ok(())
            }
            WireInput::Key { code, pressed } => {
                let keycode = code
                    .checked_add(8)
                    .and_then(|code| u16::try_from(code).ok())
                    .context("evdev keycode exceeds X11 range")?;
                ensure!(
                    keycode < 256 && self.mapped[usize::from(keycode)],
                    "evdev keycode {code} has no X11 key mapping"
                );
                let changed = self.keys.contains(&keycode) != pressed;
                if changed {
                    self.enigo()?
                        .raw(
                            keycode,
                            if pressed {
                                Direction::Press
                            } else {
                                Direction::Release
                            },
                        )
                        .map_err(|error| anyhow::anyhow!("X11 physical key: {error}"))?;
                    if pressed {
                        self.keys.insert(keycode);
                    } else {
                        self.keys.remove(&keycode);
                    }
                }
                Ok(())
            }
            WireInput::Scroll { x, y } => {
                ensure!(
                    x.is_finite() && y.is_finite() && x.abs() <= 1000.0 && y.abs() <= 1000.0,
                    "invalid X11 scroll amount"
                );
                for (value, axis) in [(x, Axis::Horizontal), (y, Axis::Vertical)] {
                    if value != 0.0 {
                        let steps = value.signum() as i32
                            * (value.abs() / 100.0).round().clamp(1.0, 20.0) as i32;
                        self.enigo()?
                            .scroll(steps, axis)
                            .map_err(|error| anyhow::anyhow!("X11 scroll: {error}"))?;
                    }
                }
                Ok(())
            }
            WireInput::Text { text } => {
                ensure!(
                    text.len() <= super::MAX_TEXT_BYTES,
                    "X11 text batch too large"
                );
                ensure!(
                    text.chars()
                        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')),
                    "unsupported X11 text control character"
                );
                // Enigo may temporarily map an unused X11 keycode for Unicode.
                // Dropping this short-lived connection restores that mapping.
                let mut writer = Enigo::new(&self.settings).map_err(|error| {
                    anyhow::anyhow!("create X11 Unicode input connection: {error}")
                })?;
                let result = writer
                    .text(&text)
                    .map_err(|error| anyhow::anyhow!("X11 Unicode input: {error}"));
                drop(writer);
                result
            }
        }
    }
    fn active_app(&self) -> Result<String> {
        let mut window = self.clipboard_connection.get_input_focus()?.reply()?.focus;
        for _ in 0..8 {
            if window <= 1 {
                break;
            }
            let property = self
                .clipboard_connection
                .get_property(
                    false,
                    window,
                    xproto::AtomEnum::WM_CLASS,
                    xproto::AtomEnum::STRING,
                    0,
                    1024,
                )?
                .reply()?;
            let classes = String::from_utf8_lossy(&property.value);
            if let Some(class) = classes
                .split('\0')
                .find(|class| crate::backend::terminal_app(class))
            {
                return Ok(class.to_owned());
            }
            if !property.value.is_empty() {
                return Ok(classes
                    .split('\0')
                    .rfind(|s| !s.is_empty())
                    .unwrap_or_default()
                    .to_owned());
            }
            let tree = self.clipboard_connection.query_tree(window)?.reply()?;
            if tree.parent == window {
                break;
            }
            window = tree.parent;
        }
        Ok(String::new())
    }

    fn handle(&mut self, request: WireRequest) -> (WireReply, bool) {
        let stop = matches!(request, WireRequest::Stop);
        if matches!(request, WireRequest::ClipboardRevision) {
            // Flush server selection notifications before reporting a revision,
            // including ownership acquired by the separate arboard connection.
            if let Err(error) = self
                .clipboard_connection
                .get_input_focus()
                .map_err(anyhow::Error::from)
                .and_then(|cookie| cookie.reply().map_err(anyhow::Error::from))
            {
                return (
                    WireReply::Error {
                        message: error.to_string(),
                    },
                    false,
                );
            }
        }
        while let Ok(Some(event)) = self.clipboard_connection.poll_for_event() {
            if let x11rb::protocol::Event::XfixesSelectionNotify(event) = event
                && event.selection == self.clipboard_selection
            {
                self.clipboard_revision = self.clipboard_revision.wrapping_add(1);
            }
        }
        let result = match request {
            WireRequest::ClipboardRevision => {
                return (
                    WireReply::Revision {
                        value: self.clipboard_revision,
                    },
                    false,
                );
            }
            WireRequest::ClipboardWritePng { path } => (|| -> Result<Option<String>> {
                let file = clipboard_transfer_file(&path, false)?;
                let mut reader = image::ImageReader::with_format(
                    io::BufReader::new(file),
                    image::ImageFormat::Png,
                );
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(8192);
                limits.max_image_height = Some(8192);
                limits.max_alloc = Some(128 * 1024 * 1024);
                reader.limits(limits);
                let image = reader.decode()?.into_rgba8();
                ensure!(
                    u64::from(image.width()) * u64::from(image.height()) <= 33554432,
                    "clipboard image dimensions too large"
                );
                self.clipboard.set_image(arboard::ImageData {
                    width: image.width() as usize,
                    height: image.height() as usize,
                    bytes: std::borrow::Cow::Owned(image.into_raw()),
                })?;
                Ok(None)
            })(),
            WireRequest::ClipboardReadPng { path } => (|| -> Result<Option<String>> {
                use image::ImageEncoder;
                let image = match self.clipboard.get_image() {
                    Ok(image) => image,
                    Err(arboard::Error::ContentNotAvailable) => return Ok(None),
                    Err(error) => return Err(error.into()),
                };
                ensure!(
                    image.width <= 8192
                        && image.height <= 8192
                        && image.width * image.height <= 33554432,
                    "clipboard image dimensions too large"
                );
                let mut bytes = Vec::new();
                image::codecs::png::PngEncoder::new(&mut bytes).write_image(
                    &image.bytes,
                    image.width as u32,
                    image.height as u32,
                    image::ExtendedColorType::Rgba8,
                )?;
                ensure!(bytes.len() <= 8 * 1024 * 1024, "clipboard image too large");
                let mut file = clipboard_transfer_file(&path, true)?;
                file.set_len(0)?;
                file.write_all(&bytes)?;
                Ok(Some("image/png".into()))
            })(),
            WireRequest::Input { command } => self.input(command).map(|()| None),
            WireRequest::ClipboardRead => self
                .clipboard
                .get_text()
                .map_err(anyhow::Error::from)
                .and_then(|text| {
                    ensure!(text.len() <= MAX_CLIPBOARD_BYTES, "clipboard too large");
                    Ok(Some(text))
                }),
            WireRequest::ClipboardWrite { text } => (|| -> Result<Option<String>> {
                ensure!(text.len() <= MAX_CLIPBOARD_BYTES, "clipboard too large");
                self.clipboard.set_text(text)?;
                Ok(None)
            })(),
            WireRequest::ActiveApp => self.active_app().map(Some),
            WireRequest::Release | WireRequest::Stop => self.release().map(|()| None),
        };
        let reply = match result {
            Ok(text) => WireReply::Ok { text },
            Err(error) => WireReply::Error {
                message: format!("{error:#}").chars().take(1024).collect(),
            },
        };
        (reply, stop)
    }
}

fn clipboard_transfer_file(path: &Path, write: bool) -> Result<std::fs::File> {
    let owner = unsafe { libc::geteuid() };
    ensure!(path.is_absolute(), "clipboard transfer path is relative");
    let parent =
        std::fs::symlink_metadata(path.parent().context("clipboard transfer parent missing")?)?;
    ensure!(
        parent.is_dir()
            && !parent.file_type().is_symlink()
            && parent.uid() == owner
            && parent.mode() & 0o077 == 0,
        "clipboard transfer directory is not private"
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == owner
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 8 * 1024 * 1024,
        "invalid clipboard transfer file"
    );
    Ok(file)
}

fn mouse_button(button: u32) -> Result<Button> {
    match button {
        0 => Ok(Button::Left),
        1 => Ok(Button::Middle),
        2 => Ok(Button::Right),
        _ => bail!("unsupported X11 mouse button"),
    }
}

fn read_worker_line(reader: &mut impl io::BufRead) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            ensure!(line.is_empty(), "incomplete X11 worker request");
            return Ok(None);
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        let ended = available[count - 1] == b'\n';
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        ensure!(line.len() <= MAX_IPC_LINE, "X11 worker request too large");
        if ended {
            return Ok(Some(line));
        }
    }
}

fn write_worker_reply(writer: &mut impl Write, reply: &WireReply) -> Result<()> {
    let mut bytes = serde_json::to_vec(reply)?;
    bytes.push(b'\n');
    ensure!(bytes.len() <= MAX_IPC_LINE, "X11 worker response too large");
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Invoked only by the private `--internal-x11-worker` branch before CLI parsing.
pub fn worker_main() -> Result<()> {
    let display = std::env::var("DISPLAY").context("X11 worker DISPLAY is missing")?;
    local_display(&display)?;
    let authority = std::env::var_os("XAUTHORITY").context("X11 worker XAUTHORITY is missing")?;
    ensure!(
        Path::new(&authority).is_absolute() && Path::new(&authority).is_file(),
        "X11 worker authority file unavailable"
    );
    let mut stdout = io::stdout().lock();
    let mut state = match WorkerState::new(display) {
        Ok(state) => state,
        Err(error) => {
            let _ = write_worker_reply(
                &mut stdout,
                &WireReply::Error {
                    message: format!("{error:#}").chars().take(1024).collect(),
                },
            );
            return Err(error);
        }
    };
    write_worker_reply(&mut stdout, &WireReply::Ready)?;
    let mut stdin = io::BufReader::new(io::stdin().lock());
    let work = (|| -> Result<()> {
        while let Some(line) = read_worker_line(&mut stdin)? {
            let request: WireRequest =
                serde_json::from_slice(&line).context("invalid X11 worker request")?;
            let (reply, stop) = state.handle(request);
            write_worker_reply(&mut stdout, &reply)?;
            if stop {
                break;
            }
        }
        Ok(())
    })();
    let released = state.release();
    work?;
    released
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::InputGuard;
    use std::os::unix::net::UnixListener;
    use x11rb::protocol::xproto::ChangeWindowAttributesAux;

    fn visual(red: u32, green: u32, blue: u32) -> xproto::Visualtype {
        xproto::Visualtype {
            visual_id: 7,
            class: VisualClass::TRUE_COLOR,
            bits_per_rgb_value: 8,
            colormap_entries: 256,
            red_mask: red,
            green_mask: green,
            blue_mask: blue,
        }
    }

    fn image(data: &[u8], depth: u8) -> xproto::GetImageReply {
        xproto::GetImageReply {
            depth,
            sequence: 1,
            visual: 7,
            data: data.to_vec(),
        }
    }

    #[test]
    fn accepts_only_explicit_local_display() {
        assert_eq!(local_display(":12.1").unwrap(), (12, 1));
        for value in ["", "host:12", ":", ":12.", ":12.x", ":1.0.2"] {
            assert!(local_display(value).is_err(), "{value}");
        }
    }

    #[test]
    fn uses_raw_randr_coordinates_and_preserves_blank_root_edges() {
        let root = Rect {
            x: 0,
            y: 0,
            width: 3840,
            height: 1080,
        };
        // A lone right-hand output starts at 1920; the left half is blank.
        let crop = crop_rect((1920, 0, 1920, 1080), root).unwrap();
        assert_eq!(crop.x, 1920);
        assert_eq!(crop.point(0.0, 0.0), (1920, 0));
        assert_eq!(crop.point(1.0, 1.0), (3839, 1079));
        assert!(crop_rect((-640, 0, 640, 1080), root).is_err());
        assert!(crop_rect((1921, 0, 1920, 1080), root).is_err());
    }

    #[test]
    fn decodes_common_x11_pixel_orders_and_padded_rows() {
        let rgb = visual(0x00ff_0000, 0x0000_ff00, 0x0000_00ff);
        let format = xproto::Format {
            depth: 24,
            bits_per_pixel: 32,
            scanline_pad: 32,
        };
        let rgba = image_rgba(
            image(&[0x33, 0x22, 0x11, 0], 24),
            1,
            1,
            ImageOrder::LSB_FIRST,
            format,
            &rgb,
        )
        .unwrap();
        assert_eq!(&*rgba, &[0x11, 0x22, 0x33, 255]);

        let format = xproto::Format {
            depth: 24,
            bits_per_pixel: 24,
            scanline_pad: 8,
        };
        let rgba = image_rgba(
            image(&[0x11, 0x22, 0x33, 0], 24),
            1,
            1,
            ImageOrder::MSB_FIRST,
            format,
            &rgb,
        )
        .unwrap();
        assert_eq!(&*rgba, &[0x11, 0x22, 0x33, 255]);

        let rgb565 = visual(0xf800, 0x07e0, 0x001f);
        let format = xproto::Format {
            depth: 16,
            bits_per_pixel: 16,
            scanline_pad: 16,
        };
        let rgba = image_rgba(
            image(&[0x00, 0xf8, 0, 0], 16),
            1,
            1,
            ImageOrder::LSB_FIRST,
            format,
            &rgb565,
        )
        .unwrap();
        assert_eq!(&*rgba, &[255, 0, 0, 255]);
    }

    #[test]
    fn rejects_malformed_image_and_authority_cookie() {
        let rgb = visual(0x00ff_0000, 0x0000_ff00, 0x0000_00ff);
        let format = xproto::Format {
            depth: 24,
            bits_per_pixel: 32,
            scanline_pad: 32,
        };
        assert!(
            image_rgba(
                image(&[1, 2], 24),
                1,
                1,
                ImageOrder::LSB_FIRST,
                format,
                &rgb
            )
            .is_err()
        );
        assert!(channel(0xff, 0b1011).is_err());
        assert!(decode_hex("ab12").is_ok());
        assert!(decode_hex("zz").is_err());
        assert!(decode_hex("abc").is_err());
    }

    #[test]
    fn saturated_ipc_queue_applies_backpressure_and_stop_releases_the_input_pump() {
        let (frames, _) = watch::channel(None);
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            frames,
            capture_socket: Mutex::new(None),
            rect: Mutex::new(None),
            cursors: watch::channel(None).0,
            cursor_supported: AtomicBool::new(false),
            clipboard_supported: AtomicBool::new(false),
            cursor_owner: AtomicU64::new(0),
        });
        let (input, input_rx) = mpsc::channel(2);
        let (queue, mut queue_rx) = mpsc::channel(2);
        queue
            .try_send(QueueItem::Input(InputCommand::Release))
            .unwrap();
        queue
            .try_send(QueueItem::Input(InputCommand::Release))
            .unwrap();
        input.try_send(InputCommand::Release).unwrap();
        input.try_send(InputCommand::Release).unwrap();
        let worker_shared = shared.clone();
        let (done, completed) = sync_mpsc::sync_channel(1);
        let pump = thread::spawn(move || {
            run_input_queue(input_rx, queue, &worker_shared);
            let _ = done.send(());
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while input.capacity() != 1 {
            assert!(
                Instant::now() < deadline,
                "input pump never reached the full IPC queue"
            );
            thread::yield_now();
        }
        input.try_send(InputCommand::Release).unwrap();
        assert!(matches!(
            input.try_send(InputCommand::Release),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        assert_eq!(queue_rx.len(), 2, "IPC backlog exceeded its capacity");
        shared.finish();
        queue_rx.close();
        drop(queue_rx);
        completed.recv_timeout(Duration::from_secs(1)).unwrap();
        pump.join().unwrap();
    }

    #[test]
    fn static_frames_refresh_silently_resize_notifies_and_stop_stays_empty() {
        let (sender, mut receiver) = watch::channel(None);
        let shared = Shared {
            stopped: AtomicBool::new(false),
            frames: sender,
            capture_socket: Mutex::new(None),
            rect: Mutex::new(None),
            cursors: watch::channel(None).0,
            cursor_supported: AtomicBool::new(false),
            clipboard_supported: AtomicBool::new(false),
            cursor_owner: AtomicU64::new(0),
        };
        let small = Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        assert!(shared.publish(small, vec![255u8; 8].into(), false));
        receiver.mark_unchanged();
        assert!(shared.publish(small, vec![255u8; 8].into(), false));
        assert!(!receiver.has_changed().unwrap());
        let current = receiver.borrow().as_ref().unwrap().pixels.clone();
        assert!(shared.refresh(small, current.clone(), false));
        assert!(!receiver.has_changed().unwrap());
        assert!(Arc::ptr_eq(
            &current,
            &receiver.borrow().as_ref().unwrap().pixels
        ));
        let large = Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 1,
        };
        assert!(shared.publish(large, vec![255u8; 12].into(), false));
        assert!(receiver.has_changed().unwrap());
        assert_eq!(receiver.borrow().as_ref().unwrap().width, 3);
        shared.finish();
        assert!(!shared.publish(small, vec![0u8; 8].into(), false));
        assert!(!shared.refresh(small, current, false));
        assert!(receiver.borrow().is_none());
    }

    #[test]
    #[ignore = "waits for the six-second X11 probe deadline"]
    fn silent_x11_server_cannot_hang_probe() -> Result<()> {
        struct RemoveSocket(PathBuf);
        impl Drop for RemoveSocket {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let base = u32::from(std::process::id() as u16) % 40000 + 20000;
        let (number, listener, path) = (0..100)
            .find_map(|offset| {
                let number = base + offset;
                let path = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
                UnixListener::bind(&path)
                    .ok()
                    .map(|listener| (number, listener, path))
            })
            .context("could not reserve a private fake X11 socket")?;
        let _cleanup = RemoveSocket(path);
        let _listener = listener;
        let directory = tempfile::tempdir()?;
        let authority = directory.path().join("authority");
        let display = format!(":{number}");
        let status = Command::new(xauth_program())
            .arg("-f")
            .arg(&authority)
            .arg("add")
            .arg(&display)
            .arg("MIT-MAGIC-COOKIE-1")
            .arg("123456789abcdef0123456789abcdef0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        assert!(status.success());
        let target = SessionTarget {
            x11_display: Some(display),
            xauthority: Some(authority),
            ..SessionTarget::default()
        };
        let started = Instant::now();
        assert!(probe(&target).is_err());
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "X11 probe missed its deadline"
        );
        Ok(())
    }

    fn paint_root(connection: &XConnection, screen: usize, pixel: u32) -> Result<()> {
        let root = connection.setup().roots[screen].root;
        let geometry = root_rect(connection, screen)?;
        connection
            .change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new().background_pixel(pixel),
            )?
            .check()?;
        connection
            .clear_area(false, root, 0, 0, geometry.width, geometry.height)?
            .check()?;
        connection.flush()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires a private authenticated Xvfb display"]
    fn live_private_xvfb_acceleration_and_core_fallback() -> Result<()> {
        let session = SessionTarget {
            x11_display: Some(std::env::var("LRD_X11_TEST_DISPLAY")?),
            xauthority: Some(std::env::var("LRD_X11_TEST_AUTH")?.into()),
            ..SessionTarget::default()
        };
        let target = XTarget::from_session(&session)?;
        let connection = connect(&target, &Mutex::new(None))?;
        let (visual, _) = visual_and_format(&connection, target.screen)?;
        paint_root(&connection, target.screen, visual.red_mask)?;
        let mut capture = capture::Capture::new(&connection, &target, None);
        let (rect, red, acquired) = capture.next()?;
        assert!(acquired);
        assert_eq!(&red[..4], &[255, 0, 0, 255]);
        let accelerated = std::env::var_os("LRD_TEST_X11_DISABLE_ACCELERATION").is_none();
        assert_eq!(capture.modes(), (accelerated, accelerated));
        let (_, same, acquired) = capture.next()?;
        assert_eq!(acquired, !accelerated);
        if accelerated {
            assert!(Arc::ptr_eq(&red, &same));
        }
        paint_root(&connection, target.screen, visual.blue_mask)?;
        let (next_rect, blue, acquired) = capture.next()?;
        assert_eq!(rect, next_rect);
        assert!(acquired);
        assert_eq!(&blue[..4], &[0, 0, 255, 255]);
        assert_eq!(
            &red[..4],
            &[255, 0, 0, 255],
            "retained capture was overwritten"
        );
        drop(capture);
        connection.get_input_focus()?.reply()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires a private authenticated Xvfb display"]
    fn live_private_xvfb_cursor_shape_motion_and_disconnect_never_changes_video() -> Result<()> {
        let session = SessionTarget {
            x11_display: Some(std::env::var("LRD_X11_TEST_DISPLAY")?),
            xauthority: Some(std::env::var("LRD_X11_TEST_AUTH")?.into()),
            ..SessionTarget::default()
        };
        let target = XTarget::from_session(&session)?;
        let connection = connect(&target, &Mutex::new(None))?;
        let root = connection.setup().roots[target.screen].root;
        let (visual, _) = visual_and_format(&connection, target.screen)?;
        paint_root(&connection, target.screen, visual.blue_mask)?;
        let source = connection.generate_id()?;
        let mask = connection.generate_id()?;
        let gc = connection.generate_id()?;
        connection.create_pixmap(1, source, root, 8, 8)?.check()?;
        connection.create_pixmap(1, mask, root, 8, 8)?.check()?;
        connection
            .create_gc(gc, source, &xproto::CreateGCAux::new().foreground(1))?
            .check()?;
        let area = [xproto::Rectangle {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        }];
        connection.poly_fill_rectangle(source, gc, &area)?.check()?;
        connection.poly_fill_rectangle(mask, gc, &area)?.check()?;
        let arrow = connection.generate_id()?;
        connection
            .create_cursor(arrow, source, mask, 65535, 0, 0, 0, 0, 0, 2, 3)?
            .check()?;
        connection
            .change_window_attributes(
                root,
                &xproto::ChangeWindowAttributesAux::new().cursor(arrow),
            )?
            .check()?;
        connection
            .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 10, 20)?
            .check()?;
        let backend = Backend::start_with_executable(
            session,
            60,
            85,
            None,
            Arc::new(AtomicBool::new(false)),
            Path::new(&std::env::var("LRD_X11_TEST_BINARY")?),
        )?;
        let cursors = backend
            .cursor_frames()
            .context("XFixes cursor channel missing")?;
        let wait = |condition: &dyn Fn() -> bool| -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !condition() {
                ensure!(Instant::now() < deadline, "cursor update timed out");
                thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        };
        wait(&|| {
            cursors
                .borrow()
                .as_ref()
                .is_some_and(|c| c.visible && c.width == 8)
        })?;
        let first = cursors.borrow().as_ref().unwrap().clone();
        assert_eq!((first.hotspot_x, first.hotspot_y), (2, 3));
        assert_eq!(&first.pixels[..4], &[0, 0, 255, 255]);
        let frames = backend.frames();
        let index = (17 * first.units_width as usize + 8) * 4;
        assert!(!frames.borrow().as_ref().unwrap().cursor_embedded);
        assert_eq!(
            &frames.borrow().as_ref().unwrap().pixels[index..index + 4],
            &[0, 0, 255, 255]
        );
        backend.begin_cursor(41);
        backend.separate_cursor(41);
        wait(&|| frames.borrow().as_ref().is_some_and(|f| !f.cursor_embedded))?;
        assert_eq!(
            &frames.borrow().as_ref().unwrap().pixels[index..index + 4],
            &[0, 0, 255, 255]
        );
        connection
            .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 30, 40)?
            .check()?;
        wait(&|| {
            cursors
                .borrow()
                .as_ref()
                .is_some_and(|c| (c.x * f64::from(c.units_width) - 30.).abs() < 0.01)
        })?;
        backend.release_cursor(40);
        assert!(!frames.borrow().as_ref().unwrap().cursor_embedded);
        backend.release_cursor(41);
        assert!(!frames.borrow().as_ref().unwrap().cursor_embedded);
        let next_index = (37 * first.units_width as usize + 28) * 4;
        assert_eq!(
            &frames.borrow().as_ref().unwrap().pixels[next_index..next_index + 4],
            &[0, 0, 255, 255]
        );
        connection
            .change_gc(gc, &xproto::ChangeGCAux::new().foreground(0))?
            .check()?;
        connection.poly_fill_rectangle(mask, gc, &area)?.check()?;
        let hidden = connection.generate_id()?;
        connection
            .create_cursor(hidden, source, mask, 65535, 0, 0, 0, 0, 0, 2, 3)?
            .check()?;
        connection
            .change_window_attributes(
                root,
                &xproto::ChangeWindowAttributesAux::new().cursor(hidden),
            )?
            .check()?;
        wait(&|| cursors.borrow().as_ref().is_some_and(|c| !c.visible))?;
        wait(&|| {
            frames
                .borrow()
                .as_ref()
                .is_some_and(|f| f.pixels[next_index..next_index + 4] == [0, 0, 255, 255])
        })?;
        backend.stop();
        Ok(())
    }

    fn held_input(connection: &XConnection, screen: usize) -> Result<(bool, bool)> {
        let code = 30u8 + 8;
        let keys = connection.query_keymap()?.reply()?.keys;
        let key = keys[usize::from(code / 8)] & (1 << (code % 8)) != 0;
        let root = connection.setup().roots[screen].root;
        let button = connection
            .query_pointer(root)?
            .reply()?
            .mask
            .contains(xproto::KeyButMask::BUTTON1);
        Ok((key, button))
    }

    /// Run with two private `Xvfb -auth` displays and the built application binary.
    /// `LRD_X11_TEST_DISPLAY{,_2}`, `LRD_X11_TEST_AUTH{,_2}`, and
    /// `LRD_X11_TEST_BINARY` are supplied by an isolated test harness.
    #[test]
    #[ignore = "requires private authenticated Xvfb displays"]
    fn live_private_xvfb_capture_input_clipboard_and_stop() -> Result<()> {
        let target = SessionTarget {
            x11_display: Some(std::env::var("LRD_X11_TEST_DISPLAY")?),
            xauthority: Some(std::env::var("LRD_X11_TEST_AUTH")?.into()),
            ..SessionTarget::default()
        };
        let other = SessionTarget {
            x11_display: Some(std::env::var("LRD_X11_TEST_DISPLAY_2")?),
            xauthority: Some(std::env::var("LRD_X11_TEST_AUTH_2")?.into()),
            ..SessionTarget::default()
        };
        let binary = PathBuf::from(std::env::var("LRD_X11_TEST_BINARY")?);
        probe(&target)?;
        probe(&other)?;

        let selected = XTarget::from_session(&target)?;
        let second = XTarget::from_session(&other)?;
        let first_connection = connect(&selected, &Mutex::new(None))?;
        let second_connection = connect(&second, &Mutex::new(None))?;
        let (first_visual, _) = visual_and_format(&first_connection, selected.screen)?;
        let (second_visual, _) = visual_and_format(&second_connection, second.screen)?;
        paint_root(&first_connection, selected.screen, first_visual.red_mask)?;
        paint_root(&second_connection, second.screen, second_visual.blue_mask)?;
        let (first_rect, first_pixels) = capture_image(&first_connection, &selected, None)?;
        let (second_rect, second_pixels) = capture_image(&second_connection, &second, None)?;
        assert_eq!((first_rect.width, first_rect.height), (64, 48));
        assert_eq!((second_rect.width, second_rect.height), (80, 60));
        assert_eq!(&first_pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&second_pixels[..4], &[0, 0, 255, 255]);

        // Keep the file alive until authentication has been rejected.
        let wrong_dir = tempfile::tempdir()?;
        let wrong_file = wrong_dir.path().join("wrong.Xauthority");
        let status = Command::new(xauth_program())
            .arg("-f")
            .arg(&wrong_file)
            .arg("add")
            .arg(&selected.display)
            .arg("MIT-MAGIC-COOKIE-1")
            .arg("00000000000000000000000000000000")
            .env_remove("DISPLAY")
            .env_remove("XAUTHORITY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        assert!(status.success());
        let wrong = SessionTarget {
            xauthority: Some(wrong_file),
            ..target.clone()
        };
        assert!(
            probe(&wrong).is_err(),
            "wrong Xauthority cookie was accepted"
        );

        let backend = Backend::start_with_executable(
            target,
            30,
            75,
            None,
            Arc::new(AtomicBool::new(false)),
            &binary,
        )?;
        let mut frames = backend.frames();
        let first = frames.borrow().clone().context("no initial X11 frame")?;
        assert_eq!((first.width, first.height), (64, 48));
        frames.mark_unchanged();
        thread::sleep(Duration::from_millis(120));
        assert!(
            !frames.has_changed()?,
            "unchanged X11 frame woke subscribers"
        );
        assert!(backend.frame_is_fresh(Duration::from_millis(250)));

        let root = first_connection.setup().roots[selected.screen].root;
        if let Ok(range_cookie) = first_connection.randr_get_screen_size_range(root)
            && let Ok(range) = range_cookie.reply()
            && range.min_width <= 96
            && range.max_width >= 96
            && range.min_height <= 72
            && range.max_height >= 72
            && let Ok(resize_cookie) = first_connection.randr_set_screen_size(root, 96, 72, 25, 19)
            && resize_cookie.check().is_ok()
        {
            let deadline = Instant::now() + Duration::from_secs(2);
            while frames
                .borrow()
                .as_ref()
                .is_none_or(|frame| frame.width != 96 || frame.height != 72)
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
            let resized = frames
                .borrow()
                .clone()
                .context("X11 resize removed frame")?;
            assert_eq!((resized.width, resized.height), (96, 72));
            assert!(
                frames.has_changed()?,
                "X11 resize did not wake frame subscribers"
            );
            eprintln!("Xvfb RandR resize exercised");
        } else {
            eprintln!("Xvfb does not expose a 96x72 RandR resize; capture resize check skipped");
        }

        backend
            .input
            .try_send(InputCommand::Move { x: 0.5, y: 0.5 })?;
        backend.input.try_send(InputCommand::Key {
            code: 30,
            pressed: true,
        })?;
        backend.input.try_send(InputCommand::Button {
            button: 0,
            pressed: true,
        })?;
        let deadline = Instant::now() + Duration::from_secs(1);
        while held_input(&first_connection, selected.screen)? != (true, true)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            held_input(&first_connection, selected.screen)?,
            (true, true)
        );
        assert!(backend.release_input(), "X11 release barrier failed");
        assert_eq!(
            held_input(&first_connection, selected.screen)?,
            (false, false)
        );
        backend.input.try_send(InputCommand::Text {
            text: "Ω你🙂".into(),
        })?;
        assert!(backend.release_input(), "X11 Unicode input failed");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(backend.clipboard_write("private X11 clipboard ✓"))?;
        assert_eq!(
            runtime.block_on(backend.clipboard_read())?,
            "private X11 clipboard ✓"
        );
        // Image ownership uses the same selected X server and private IPC as
        // text. Compare decoded pixels, not merely a successful write reply.
        let before_image = runtime.block_on(backend.clipboard_revision())?;
        let pixels = image::RgbaImage::from_raw(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 30, 60, 90, 255,
            ],
        )
        .unwrap();
        let mut png = io::Cursor::new(Vec::new());
        pixels.write_to(&mut png, image::ImageFormat::Png)?;
        runtime.block_on(backend.clipboard_write_png(png.get_ref()))?;
        let restored = runtime
            .block_on(backend.clipboard_read_png())?
            .context("X11 image clipboard missing")?;
        assert_eq!(image::load_from_memory(&restored)?.to_rgba8(), pixels);
        assert_ne!(
            runtime.block_on(backend.clipboard_revision())?,
            before_image
        );
        runtime.block_on(backend.clipboard_write("private X11 clipboard ✓"))?;

        // Block the IPC worker after one text batch, then cancel a clipboard
        // write while it is queued. This is the same receiver cancellation used
        // by GuardedBackend when the physical session locks.
        let (entered, entered_rx) = sync_mpsc::sync_channel(1);
        let (resume, resume_rx) = sync_mpsc::sync_channel(1);
        let mut guard = InputGuard::new(
            tokio_util::sync::CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        );
        guard.probe = Some(Arc::new(super::super::TextProbe {
            characters: std::sync::atomic::AtomicUsize::new(0),
            batches: std::sync::atomic::AtomicUsize::new(0),
            restored: AtomicBool::new(false),
            first_batch: entered,
            resume: Mutex::new(resume_rx),
        }));
        backend.input.try_send(InputCommand::Guarded {
            command: Box::new(InputCommand::Text { text: "ab".into() }),
            guard,
        })?;
        entered_rx.recv_timeout(Duration::from_secs(2))?;
        let (reply, response) = oneshot::channel();
        backend.queue.try_send(QueueItem::ClipboardWrite(
            "cancelled clipboard write".into(),
            reply,
        ))?;
        drop(response);
        resume.send(())?;
        assert!(
            backend.release_input(),
            "cancelled clipboard queue did not drain"
        );
        assert_eq!(
            runtime.block_on(backend.clipboard_read())?,
            "private X11 clipboard ✓",
            "cancelled queued write changed the physical clipboard",
        );
        let mut observer = WorkerProcess::launch_executable(&selected, &binary)?;
        assert!(
            matches!(
                observer.exchange(
                    &WireRequest::Input {
                        command: WireInput::Text {
                            text: "Ω你🙂".into()
                        }
                    },
                    REQUEST_TIMEOUT
                )?,
                WireReply::Ok { .. }
            ),
            "X11 worker rejected Unicode input"
        );
        match observer.exchange(&WireRequest::ClipboardRead, CLIPBOARD_TIMEOUT)? {
            WireReply::Ok { text: Some(text) } => assert_eq!(text, "private X11 clipboard ✓"),
            _ => bail!("separate X11 client could not read clipboard owner"),
        }
        backend.stop();
        assert!(
            frames.borrow().is_none(),
            "stopped X11 backend retained frame"
        );
        assert!(
            matches!(
                observer.exchange(&WireRequest::ClipboardRead, CLIPBOARD_TIMEOUT)?,
                WireReply::Error { .. }
            ),
            "clipboard owner outlived stopped backend"
        );
        let _ = observer.exchange(&WireRequest::Stop, REQUEST_TIMEOUT)?;
        Ok(())
    }
}
