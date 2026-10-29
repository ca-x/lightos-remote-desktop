//! A single output capture session with one reusable SHM buffer. Unlike the
//! screenshot path, each frame needs no new source, session, file or mapping.
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs::File,
    os::fd::{AsFd, FromRawFd},
    sync::Arc,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

pub(super) struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Arc<[u8]>,
}
#[derive(Debug)]
pub(super) struct Reconfigure;
impl std::fmt::Display for Reconfigure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("capture output changed")
    }
}
impl std::error::Error for Reconfigure {}

#[derive(Default)]
struct Output {
    name: String,
    transform: u32,
}
#[derive(Default)]
struct State {
    outputs: BTreeMap<u32, Output>,
    initialized: bool,
    topology_changed: bool,
    size: (u32, u32),
    formats: Vec<wl_shm::Format>,
    constraints: bool,
    stopped: bool,
    ready: bool,
    failed: bool,
    transform: u32,
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        s: &mut Self,
        _: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if s.initialized
            && match e {
                wl_registry::Event::Global { interface, .. } => interface == "wl_output",
                wl_registry::Event::GlobalRemove { name } => s.outputs.contains_key(&name),
                _ => false,
            }
        {
            s.topology_changed = true;
        }
    }
}
impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        s: &mut Self,
        _: &wl_output::WlOutput,
        e: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let output = s.outputs.entry(*id).or_default();
        match e {
            wl_output::Event::Name { name } => {
                if s.initialized && output.name != name {
                    s.topology_changed = true;
                }
                output.name = name;
            }
            wl_output::Event::Geometry {
                transform: WEnum::Value(value),
                ..
            } => {
                if s.initialized && output.transform != value as u32 {
                    s.topology_changed = true;
                }
                output.transform = value as u32;
            }
            _ => {}
        }
    }
}
impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        e: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if s.constraints && !matches!(e, ext_image_copy_capture_session_v1::Event::Stopped) {
            s.formats.clear();
            s.constraints = false;
        }
        match e {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                s.size = (width, height)
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => s.formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => s.constraints = true,
            ext_image_copy_capture_session_v1::Event::Stopped => s.stopped = true,
            _ => {}
        }
    }
}
impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        e: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            ext_image_copy_capture_frame_v1::Event::Ready => s.ready = true,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => s.failed = true,
            ext_image_copy_capture_frame_v1::Event::Transform {
                transform: WEnum::Value(value),
            } => s.transform = value as u32,
            _ => {}
        }
    }
}
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtImageCaptureSourceV1);
delegate_noop!(State: ignore ExtImageCopyCaptureManagerV1);

struct Buffer {
    handle: wl_buffer::WlBuffer,
    map: memmap2::Mmap,
    width: u32,
    height: u32,
    format: wl_shm::Format,
}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.handle.destroy();
    }
}
impl Buffer {
    fn new(
        shm: &wl_shm::WlShm,
        queue: &QueueHandle<State>,
        width: u32,
        height: u32,
        format: wl_shm::Format,
    ) -> Result<Self> {
        ensure!(
            width > 0
                && height > 0
                && width <= 8192
                && height <= 8192
                && u64::from(width) * u64::from(height) <= 33_554_432,
            "invalid capture buffer dimensions"
        );
        let length = u64::from(width) * u64::from(height) * 4;
        let fd = unsafe {
            libc::memfd_create(
                c"lightos-wayland-frame".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        ensure!(
            fd >= 0,
            "create capture buffer: {}",
            std::io::Error::last_os_error()
        );
        // This descriptor has one owner. Both peers retain mappings, and size
        // seals prevent truncation while the compositor writes the buffer.
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(length)?;
        ensure!(
            unsafe {
                libc::fcntl(
                    fd,
                    libc::F_ADD_SEALS,
                    libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL,
                )
            } >= 0,
            "seal capture buffer: {}",
            std::io::Error::last_os_error()
        );
        let map = unsafe { memmap2::Mmap::map(&file)? };
        let pool = shm.create_pool(file.as_fd(), length as i32, queue, ());
        let handle = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            (width * 4) as i32,
            format,
            queue,
            (),
        );
        pool.destroy();
        Ok(Self {
            handle,
            map,
            width,
            height,
            format,
        })
    }
    fn pixels(&self) -> Arc<[u8]> {
        let mut pixels = Arc::<[u8]>::new_uninit_slice(self.map.len());
        let target = Arc::get_mut(&mut pixels).unwrap().as_mut_ptr().cast::<u8>();
        // The compositor has sent Ready and cannot write again until the next
        // capture. Initialize every byte of this exclusively owned allocation.
        unsafe {
            std::ptr::copy_nonoverlapping(self.map.as_ptr(), target, self.map.len());
            if matches!(
                self.format,
                wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888
            ) {
                for pixel in std::slice::from_raw_parts_mut(target, self.map.len())
                    .as_chunks_mut::<4>()
                    .0
                {
                    pixel.swap(0, 2);
                }
            }
            if matches!(
                self.format,
                wl_shm::Format::Xrgb8888 | wl_shm::Format::Xbgr8888
            ) {
                for pixel in std::slice::from_raw_parts_mut(target, self.map.len())
                    .as_chunks_mut::<4>()
                    .0
                {
                    pixel[3] = 255;
                }
            }
            pixels.assume_init()
        }
    }
}
struct Pending(ExtImageCopyCaptureFrameV1);
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

pub(super) struct Reusable {
    queue: EventQueue<State>,
    state: State,
    shm: wl_shm::WlShm,
    session: ExtImageCopyCaptureSessionV1,
    source: ExtImageCaptureSourceV1,
    outputs: Vec<wl_output::WlOutput>,
    buffer: Option<Buffer>,
}
impl Drop for Reusable {
    fn drop(&mut self) {
        self.buffer.take();
        self.session.destroy();
        self.source.destroy();
        for output in &self.outputs {
            output.release();
        }
    }
}
impl Reusable {
    pub fn try_new(connection: Connection, selected: Option<&str>) -> Result<Option<Self>> {
        let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
        let q = queue.handle();
        let Ok(manager): Result<ExtImageCopyCaptureManagerV1, _> = globals.bind(&q, 1..=1, ())
        else {
            return Ok(None);
        };
        let Ok(sources): Result<ExtOutputImageCaptureSourceManagerV1, _> =
            globals.bind(&q, 1..=1, ())
        else {
            return Ok(None);
        };
        let Ok(shm): Result<wl_shm::WlShm, _> = globals.bind(&q, 1..=1, ()) else {
            return Ok(None);
        };
        let entries: Vec<_> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == "wl_output")
                .map(|g| (g.name, g.version))
                .collect()
        });
        if entries.is_empty()
            || (selected.is_none() && entries.len() != 1)
            || entries.iter().any(|(_, v)| *v < 4)
        {
            return Ok(None);
        }
        let outputs: Vec<wl_output::WlOutput> = entries
            .iter()
            .map(|(id, _)| globals.registry().bind(*id, 4, &q, *id))
            .collect();
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        let Some((index, _)) = entries.iter().enumerate().find(|(_, (id, _))| {
            state
                .outputs
                .get(id)
                .is_some_and(|o| o.transform == 0 && selected.is_none_or(|name| o.name == name))
        }) else {
            return Ok(None);
        };
        let source = sources.create_source(&outputs[index], &q, ());
        let session = manager.create_session(&source, Options::empty(), &q, ());
        while !state.constraints && !state.stopped {
            queue.blocking_dispatch(&mut state)?;
        }
        let mut capture = Self {
            queue,
            state,
            shm,
            session,
            source,
            outputs,
            buffer: None,
        };
        if capture.state.stopped || capture.format().is_none() {
            return Ok(None);
        }
        capture.state.initialized = true;
        tracing::debug!("using reusable Wayland capture session");
        Ok(Some(capture))
    }
    fn format(&self) -> Option<wl_shm::Format> {
        [
            wl_shm::Format::Xbgr8888,
            wl_shm::Format::Abgr8888,
            wl_shm::Format::Xrgb8888,
            wl_shm::Format::Argb8888,
        ]
        .into_iter()
        .find(|f| self.state.formats.contains(f))
    }
    fn check_output(&self) -> Result<()> {
        if self.state.stopped || self.state.topology_changed {
            return Err(Reconfigure.into());
        }
        Ok(())
    }
    pub fn capture(&mut self) -> Result<Pixels> {
        for _ in 0..3 {
            self.queue.dispatch_pending(&mut self.state)?;
            self.check_output()?;
            while !self.state.constraints {
                self.queue.blocking_dispatch(&mut self.state)?;
                self.check_output()?;
            }
            let (width, height) = self.state.size;
            let format = self
                .format()
                .context("capture has no supported SHM format")?;
            let fresh = self
                .buffer
                .as_ref()
                .is_none_or(|b| b.width != width || b.height != height || b.format != format);
            if fresh {
                self.buffer = Some(Buffer::new(
                    &self.shm,
                    &self.queue.handle(),
                    width,
                    height,
                    format,
                )?);
            }
            self.state.ready = false;
            self.state.failed = false;
            self.state.transform = 0;
            let frame = Pending(self.session.create_frame(&self.queue.handle(), ()));
            frame.0.attach_buffer(&self.buffer.as_ref().unwrap().handle);
            // This buffer is never modified by the client. Only its first
            // capture needs client damage; the compositor tracks source damage.
            if fresh {
                frame.0.damage_buffer(0, 0, width as i32, height as i32);
            }
            frame.0.capture();
            while !self.state.ready && !self.state.failed {
                self.queue.blocking_dispatch(&mut self.state)?;
                self.check_output()?;
            }
            if self.state.failed {
                continue;
            }
            ensure!(
                self.state.transform == 0,
                "capture requires transformed screenshot path"
            );
            let buffer = self.buffer.as_ref().unwrap();
            return Ok(Pixels {
                width: buffer.width,
                height: buffer.height,
                data: buffer.pixels(),
            });
        }
        bail!("capture buffer constraints did not settle")
    }
}
