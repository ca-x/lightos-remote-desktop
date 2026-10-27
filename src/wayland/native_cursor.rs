//! Standard Wayland cursor capture, separate from the desktop screencopy.
use super::*;
use crate::backend::CursorFrame;
use wayland_client::{
    WEnum,
    protocol::{wl_buffer, wl_output, wl_pointer, wl_shm, wl_shm_pool},
};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
pub(super) struct Cursor {
    pub frames: watch::Receiver<Option<Arc<CursorFrame>>>,
    pub focus: Arc<Mutex<String>>,
    stopped: Arc<AtomicBool>,
    socket: UnixStream,
    worker: Option<JoinHandle<()>>,
}
impl Cursor {
    pub fn start(path: &Path, output: Option<String>) -> Result<Option<Self>> {
        let socket = connect(path, Instant::now() + Duration::from_secs(2), None)?;
        let interrupt = socket.try_clone()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let (tx, frames) = watch::channel(None);
        let focus = Arc::new(Mutex::new(String::new()));
        let active = focus.clone();
        let (ready, initialized) = sync_mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("wayland-cursor-image".into())
            .spawn(move || {
                let mut ready = Some(ready);
                let result = run(socket, output, tx, active, stop.clone(), &mut ready);
                if let Some(ready) = ready {
                    let _ = ready.send(result.map(|_| false).map_err(|e| format!("{e:#}")));
                } else if let Err(error) = result
                    && !stop.load(Ordering::Acquire)
                {
                    tracing::warn!(%error,"Wayland cursor capture stopped");
                }
            })?;
        match initialized.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(true)) => Ok(Some(Self {
                frames,
                focus,
                stopped,
                socket: interrupt,
                worker: Some(worker),
            })),
            result => {
                stopped.store(true, Ordering::Release);
                let _ = interrupt.shutdown(Shutdown::Both);
                let _ = worker.join();
                match result {
                    Ok(Ok(false)) => Ok(None),
                    _ => bail!("cursor capture initialization failed: {result:?}"),
                }
            }
        }
    }
}
impl Drop for Cursor {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.socket.shutdown(Shutdown::Both);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Default)]
struct Output {
    name: String,
    width: u32,
    height: u32,
    transform: u32,
}
struct Buffer {
    buffer: wl_buffer::WlBuffer,
    map: memmap2::Mmap,
    width: u32,
    height: u32,
    format: wl_shm::Format,
}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}
struct State {
    outputs: std::collections::BTreeMap<u32, Output>,
    selected: u32,
    size: (u32, u32),
    formats: Vec<wl_shm::Format>,
    constraints: bool,
    pending: bool,
    buffer: Option<Buffer>,
    stopped: bool,
    windows: std::collections::HashMap<u32, (String, bool)>,
    focus: Arc<Mutex<String>>,
    hotspot: (i32, i32),
    transform: u32,
    cursor: CursorFrame,
    frames: watch::Sender<Option<Arc<CursorFrame>>>,
}
impl State {
    fn publish(&mut self) {
        if let Some(output) = self.outputs.get(&self.selected) {
            let (width, height) = if output.transform % 2 == 1 {
                (output.height, output.width)
            } else {
                (output.width, output.height)
            };
            if width > 0 && height > 0 {
                self.cursor.units_width = width;
                self.cursor.units_height = height;
            }
        }
        if self.cursor.width > 0 {
            self.frames
                .send_replace(Some(Arc::new(self.cursor.clone())));
        }
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
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore wl_pointer::WlPointer);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtImageCaptureSourceV1);
delegate_noop!(State: ignore ExtImageCopyCaptureManagerV1);
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
            wl_output::Event::Name { name } => info.name = name,
            wl_output::Event::Geometry {
                transform: WEnum::Value(transform),
                ..
            } => info.transform = transform as u32,
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) && width > 0 && height > 0 => {
                info.width = width as u32;
                info.height = height as u32;
            }
            _ => {}
        }
        state.publish();
    }
}
impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureCursorSessionV1,
        event: ext_image_copy_capture_cursor_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_cursor_session_v1::Event::Enter => state.cursor.visible = true,
            ext_image_copy_capture_cursor_session_v1::Event::Leave => state.cursor.visible = false,
            ext_image_copy_capture_cursor_session_v1::Event::Position { x, y } => {
                state.cursor.x = f64::from(x) / f64::from(state.cursor.units_width.max(1));
                state.cursor.y = f64::from(y) / f64::from(state.cursor.units_height.max(1));
            }
            // The hotspot and pixels must become visible atomically at Ready.
            ext_image_copy_capture_cursor_session_v1::Event::Hotspot { x, y } => {
                state.hotspot = (x, y);
                return;
            }
            _ => {}
        }
        state.publish();
    }
}
impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if state.constraints && !matches!(event, ext_image_copy_capture_session_v1::Event::Stopped)
        {
            state.formats.clear();
            state.constraints = false;
        }
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.size = (width, height);
                state.constraints = false;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => state.constraints = true,
            ext_image_copy_capture_session_v1::Event::Stopped => state.stopped = true,
            _ => {}
        }
    }
}
impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Transform {
                transform: WEnum::Value(transform),
            } => state.transform = transform as u32,
            ext_image_copy_capture_frame_v1::Event::Ready => {
                if let Some(buffer) = &state.buffer {
                    let (width, height, pixels, hotspot) = transform_image(
                        buffer.width,
                        buffer.height,
                        &buffer.map,
                        state.hotspot,
                        state.transform,
                        buffer.format,
                    );
                    if state.cursor.pixels.as_ref() != pixels.as_slice()
                        || state.cursor.width != width
                        || state.cursor.height != height
                        || state.cursor.hotspot_x != hotspot.0
                        || state.cursor.hotspot_y != hotspot.1
                    {
                        state.cursor.serial = state.cursor.serial.wrapping_add(1).max(1);
                    }
                    state.cursor.width = width;
                    state.cursor.height = height;
                    state.cursor.hotspot_x = hotspot.0;
                    state.cursor.hotspot_y = hotspot.1;
                    state.cursor.pixels = pixels.into();
                    state.publish();
                }
                frame.destroy();
                state.pending = false;
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                frame.destroy();
                state.pending = false;
                state.buffer.take();
                if reason == WEnum::Value(ext_image_copy_capture_frame_v1::FailureReason::Stopped) {
                    state.stopped = true;
                }
            }
            _ => {}
        }
    }
}
fn transform_point(x: u32, y: u32, w: u32, h: u32, t: u32) -> (u32, u32) {
    let (x, y) = match t % 4 {
        0 => (x, y),
        1 => (h - 1 - y, x),
        2 => (w - 1 - x, h - 1 - y),
        _ => (y, w - 1 - x),
    };
    let width = if t % 2 == 1 { h } else { w };
    (if t >= 4 { width - 1 - x } else { x }, y)
}

fn transform_image(
    width: u32,
    height: u32,
    bytes: &[u8],
    hotspot: (i32, i32),
    transform: u32,
    format: wl_shm::Format,
) -> (u32, u32, Vec<u8>, (u32, u32)) {
    let (w, h) = if transform % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    };
    let mut pixels = vec![0; bytes.len()];
    for y in 0..height {
        for x in 0..width {
            let (tx, ty) = transform_point(x, y, width, height, transform);
            let src = ((y * width + x) * 4) as usize;
            let dst = ((ty * w + tx) * 4) as usize;
            pixels[dst..dst + 4].copy_from_slice(&bytes[src..src + 4]);
            if format == wl_shm::Format::Xrgb8888 {
                pixels[dst + 3] = 255;
            }
        }
    }
    let hotspot = transform_point(
        (hotspot.0.max(0) as u32).min(width - 1),
        (hotspot.1.max(0) as u32).min(height - 1),
        width,
        height,
        transform,
    );
    (w, h, pixels, hotspot)
}
fn run(
    socket: UnixStream,
    selected: Option<String>,
    frames: watch::Sender<Option<Arc<CursorFrame>>>,
    focus: Arc<Mutex<String>>,
    stopped: Arc<AtomicBool>,
    ready: &mut Option<sync_mpsc::SyncSender<std::result::Result<bool, String>>>,
) -> Result<()> {
    let connection = Connection::from_socket(socket)?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let handle = queue.handle();
    let Ok(manager) = globals.bind::<ExtImageCopyCaptureManagerV1, _, _>(&handle, 1..=1, ()) else {
        return Ok(());
    };
    let sources: ExtOutputImageCaptureSourceManagerV1 = globals.bind(&handle, 1..=1, ())?;
    let shm: wl_shm::WlShm = globals.bind(&handle, 1..=1, ())?;
    let seat: wl_seat::WlSeat = globals.bind(&handle, 1..=1, ())?;
    let pointer = seat.get_pointer(&handle, ());
    let _windows: Option<ZwlrForeignToplevelManagerV1> = globals.bind(&handle, 1..=3, ()).ok();
    let outputs: Vec<wl_output::WlOutput> = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|g| g.interface == "wl_output")
        .map(|g| {
            globals
                .registry()
                .bind(g.name, g.version.min(4), &handle, ())
        })
        .collect();
    let mut state = State {
        outputs: Default::default(),
        selected: 0,
        size: (0, 0),
        formats: Vec::new(),
        constraints: false,
        pending: false,
        buffer: None,
        stopped: false,
        windows: Default::default(),
        focus,
        hotspot: (0, 0),
        transform: 0,
        cursor: CursorFrame {
            serial: 1,
            visible: false,
            x: 0.0,
            y: 0.0,
            width: 0,
            height: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            units_width: 1,
            units_height: 1,
            pixels: Arc::from([]),
        },
        frames,
    };
    queue.roundtrip(&mut state)?;
    let output = outputs
        .iter()
        .find(|output| {
            selected.as_ref().is_none_or(|name| {
                state
                    .outputs
                    .get(&output.id().protocol_id())
                    .is_some_and(|info| &info.name == name)
            })
        })
        .context("selected cursor output unavailable")?;
    state.selected = output.id().protocol_id();
    state.publish();
    let source = sources.create_source(output, &handle, ());
    let cursor = manager.create_pointer_cursor_session(&source, &pointer, &handle, ());
    let capture = cursor.get_capture_session(&handle, ());
    queue.roundtrip(&mut state)?;
    if let Some(ready) = ready.take() {
        let _ = ready.send(Ok(true));
    }
    while !stopped.load(Ordering::Acquire) {
        queue.dispatch_pending(&mut state)?;
        ensure!(!state.stopped, "cursor session stopped");
        if state.constraints && !state.pending {
            let (w, h) = state.size;
            if w == 0 || h == 0 {
                state.cursor.visible = false;
                state.publish();
                state.constraints = false;
                continue;
            }
            ensure!(w <= 512 && h <= 512, "cursor buffer exceeds supported size");
            let format = [wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888]
                .into_iter()
                .find(|f| state.formats.contains(f))
                .context("cursor has no supported shared memory format")?;
            if state
                .buffer
                .as_ref()
                .is_none_or(|b| b.width != w || b.height != h || b.format != format)
            {
                let file = tempfile::tempfile()?;
                file.set_len(u64::from(w) * u64::from(h) * 4)?;
                let pool = shm.create_pool(file.as_fd(), (w * h * 4) as i32, &handle, ());
                let buffer =
                    pool.create_buffer(0, w as i32, h as i32, (w * 4) as i32, format, &handle, ());
                pool.destroy();
                // The compositor writes only while a capture is pending. Read
                // after Ready, and copy before reusing this bounded buffer.
                let map = unsafe { memmap2::MmapOptions::new().map(&file)? };
                state.buffer = Some(Buffer {
                    buffer,
                    map,
                    width: w,
                    height: h,
                    format,
                });
            }
            let frame = capture.create_frame(&handle, ());
            frame.attach_buffer(&state.buffer.as_ref().unwrap().buffer);
            frame.damage_buffer(0, 0, w as i32, h as i32);
            frame.capture();
            state.pending = true;
        }
        connection.flush()?;
        if let Some(read) = queue.prepare_read() {
            let mut p = libc::pollfd {
                fd: connection.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut p, 1, 5) };
            if n > 0 {
                read.read()?;
            } else if n < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    capture.destroy();
    cursor.destroy();
    source.destroy();
    let _ = connection.flush();
    Ok(())
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state
                .windows
                .insert(toplevel.id().protocol_id(), Default::default());
        }
    }
    wayland_client::event_created_child!(State,ZwlrForeignToplevelManagerV1,[0=>(ZwlrForeignToplevelHandleV1,())]);
}
impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        window: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = window.id().protocol_id();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.windows.entry(id).or_default().0 = app_id.chars().take(256).collect()
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: flags } => {
                state.windows.entry(id).or_default().1 = flags
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|bytes| u32::from_ne_bytes(*bytes) == 2)
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.windows.remove(&id);
                window.destroy();
            }
            _ => {}
        }
        *state.focus.lock().unwrap_or_else(|e| e.into_inner()) = state
            .windows
            .values()
            .find(|(_, active)| *active)
            .map(|(app, _)| app.clone())
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotated_and_flipped_cursor_keeps_its_hotspot_on_the_same_pixel() {
        let pixels: Vec<u8> = (0..6).flat_map(|i| [i, 0, 0, 255]).collect();
        let expected = [
            (0, 1),
            (1, 0),
            (1, 1),
            (1, 1),
            (1, 1),
            (1, 0),
            (0, 1),
            (1, 1),
        ];
        for (transform, expected) in expected.into_iter().enumerate() {
            let (width, height, image, hotspot) = transform_image(
                2,
                3,
                &pixels,
                (0, 1),
                transform as u32,
                wl_shm::Format::Argb8888,
            );
            assert_eq!(hotspot, expected, "transform {transform}");
            assert_eq!(
                image[((hotspot.1 * width + hotspot.0) * 4) as usize],
                2,
                "hotspot pixel changed at transform {transform}"
            );
            assert_eq!(width * height, 6);
        }
    }
}
