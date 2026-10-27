//! Clipboard access scoped to one admitted compositor. Both generations of
//! data-control are supported; PRIMARY is never modified.
use super::*;
use anyhow::Context;
use std::{
    collections::HashMap,
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    path::Path,
    sync::Mutex,
    thread::JoinHandle,
};
use tokio::sync::oneshot;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_callback, wl_registry, wl_seat},
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};
const TIMEOUT: Duration = Duration::from_secs(4);
const IMAGE_LIMIT: usize = 8 * 1024 * 1024;
type Reply<T> = oneshot::Sender<Result<T>>;
enum Request {
    Read(bool, Reply<Option<Vec<u8>>>),
    Write(bool, Vec<u8>, Reply<()>),
    Revision(Reply<ClipboardRevision>),
}
pub(crate) struct Clipboard {
    requests: sync_mpsc::SyncSender<Request>,
    stopped: Arc<AtomicBool>,
    socket: UnixStream,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Clipboard {
    pub(crate) fn start(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path)?;
        let socket = stream.try_clone()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let (requests, receiver) = sync_mpsc::sync_channel(32);
        let (ready, initialized) = sync_mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("wayland-data-control".into())
            .spawn(move || {
                let mut ready = Some(ready);
                let result = run(stream, receiver, stop.clone(), &mut ready);
                stop.store(true, Ordering::Release);
                if let Some(ready) = ready {
                    let _ = ready.send(result.map_err(|e| format!("{e:#}")));
                } else if let Err(error) = result {
                    tracing::warn!(%error,"Wayland clipboard stopped");
                }
            })?;
        match initialized.recv_timeout(TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                requests,
                stopped,
                socket,
                worker: Mutex::new(Some(worker)),
            }),
            result => {
                stopped.store(true, Ordering::Release);
                let _ = socket.shutdown(std::net::Shutdown::Both);
                let _ = worker.join();
                bail!("Wayland clipboard did not start: {result:?}");
            }
        }
    }
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) async fn read(&self, image: bool) -> Result<Option<Vec<u8>>> {
        let (done, response) = oneshot::channel();
        self.requests.try_send(Request::Read(image, done))?;
        tokio::time::timeout(TIMEOUT, response).await??
    }
    pub(crate) async fn write(&self, image: bool, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len()
                <= if image {
                    IMAGE_LIMIT
                } else {
                    MAX_CLIPBOARD_BYTES
                },
            "clipboard too large"
        );
        let (done, response) = oneshot::channel();
        self.requests
            .try_send(Request::Write(image, bytes.to_vec(), done))?;
        tokio::time::timeout(TIMEOUT, response).await??
    }
    pub(crate) async fn revision(&self) -> Result<ClipboardRevision> {
        let (done, response) = oneshot::channel();
        self.requests.try_send(Request::Revision(done))?;
        tokio::time::timeout(TIMEOUT, response).await??
    }
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        self.stop();
    }
}

enum Offer {
    Ext(ExtDataControlOfferV1),
    Wlr(ZwlrDataControlOfferV1),
}
impl Offer {
    fn id(&self) -> u32 {
        match self {
            Self::Ext(o) => o.id().protocol_id(),
            Self::Wlr(o) => o.id().protocol_id(),
        }
    }
    fn destroy(&self) {
        match self {
            Self::Ext(o) => o.destroy(),
            Self::Wlr(o) => o.destroy(),
        }
    }
    fn receive(&self, mime: String, fd: std::os::fd::BorrowedFd<'_>) {
        match self {
            Self::Ext(o) => o.receive(mime, fd),
            Self::Wlr(o) => o.receive(mime, fd),
        }
    }
}
enum Device {
    Ext(ExtDataControlManagerV1, ExtDataControlDeviceV1),
    Wlr(ZwlrDataControlManagerV1, ZwlrDataControlDeviceV1),
}
struct Content {
    image: bool,
    bytes: Arc<[u8]>,
}
struct State {
    offers: HashMap<u32, (Offer, Vec<String>)>,
    selection: Option<u32>,
    serial: u64,
    epoch: u64,
    marker: String,
    finished: bool,
    io: tokio::runtime::Runtime,
}
impl State {
    fn selection(&mut self, offer: Option<Offer>) {
        let next = offer.as_ref().map(Offer::id);
        if let Some(old) = self.selection.take()
            && Some(old) != next
            && let Some((old, _)) = self.offers.remove(&old)
        {
            old.destroy();
        }
        self.selection = next;
        let ours = next
            .and_then(|id| self.offers.get(&id))
            .is_some_and(|(_, types)| types.contains(&self.marker));
        if !ours {
            self.serial = self.serial.wrapping_add(1);
        }
    }
    fn discard(&mut self, offer: Offer) {
        self.offers.remove(&offer.id());
        offer.destroy();
    }
    fn mime(&mut self, id: u32, mime: String) {
        if let Some((_, types)) = self.offers.get_mut(&id)
            && types.len() < 256
            && mime.len() < 1024
        {
            types.push(mime);
        }
    }
    fn send(&self, fd: OwnedFd, mime: String, content: &Content) {
        if (content.image && mime != "image/png")
            || (!content.image
                && !matches!(
                    mime.as_str(),
                    "text/plain;charset=utf-8" | "text/plain" | "UTF8_STRING"
                ))
        {
            return;
        }
        let bytes = content.bytes.clone();
        self.io.spawn(async move {
            let _ = tokio::time::timeout(TIMEOUT, crate::clipboard::write_fd(fd, &bytes)).await;
        });
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
delegate_noop!(State: ignore ExtDataControlManagerV1);
delegate_noop!(State: ignore ZwlrDataControlManagerV1);
// Acknowledging a selection requires the compositor to have processed it, not
// merely putting set_selection into the client connection's output buffer.
impl Dispatch<wl_callback::WlCallback, Mutex<Option<Reply<()>>>> for State {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        reply: &Mutex<Option<Reply<()>>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some(reply) = reply.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = reply.send(Ok(()));
        }
    }
}
macro_rules! protocol {
    ($device:ty,$d:ident,$offer:ty,$o:ident,$source:ty,$s:ident,$variant:ident) => {
        impl Dispatch<$device,()> for State {
            fn event(state:&mut Self,_:&$device,event:$d::Event,_:&(),_:&Connection,_:&QueueHandle<Self>){match event {
                $d::Event::DataOffer{id}=>{let offer=Offer::$variant(id);state.offers.insert(offer.id(),(offer,Vec::new()));}
                $d::Event::Selection{id}=>state.selection(id.map(Offer::$variant)),
                $d::Event::PrimarySelection{id:Some(id)}=>state.discard(Offer::$variant(id)),
                $d::Event::Finished=>state.finished=true,
                _=>{}
            }}
            wayland_client::event_created_child!(State,$device,[0=>($offer,())]);
        }
        impl Dispatch<$offer,()> for State {
            fn event(state:&mut Self,offer:&$offer,event:$o::Event,_:&(),_:&Connection,_:&QueueHandle<Self>){if let $o::Event::Offer{mime_type}=event{state.mime(offer.id().protocol_id(),mime_type);}}
        }
        impl Dispatch<$source,Content> for State {
            fn event(state:&mut Self,source:&$source,event:$s::Event,content:&Content,_:&Connection,_:&QueueHandle<Self>){match event {
                $s::Event::Send{mime_type,fd}=>state.send(fd,mime_type,content),
                $s::Event::Cancelled=>source.destroy(),
                _=>{}
            }}
        }
    }
}
protocol!(
    ExtDataControlDeviceV1,
    ext_data_control_device_v1,
    ExtDataControlOfferV1,
    ext_data_control_offer_v1,
    ExtDataControlSourceV1,
    ext_data_control_source_v1,
    Ext
);
protocol!(
    ZwlrDataControlDeviceV1,
    zwlr_data_control_device_v1,
    ZwlrDataControlOfferV1,
    zwlr_data_control_offer_v1,
    ZwlrDataControlSourceV1,
    zwlr_data_control_source_v1,
    Wlr
);
fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    ensure!(
        unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } == 0,
        "clipboard pipe unavailable"
    );
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}
fn run(
    stream: UnixStream,
    requests: sync_mpsc::Receiver<Request>,
    stopped: Arc<AtomicBool>,
    ready: &mut Option<sync_mpsc::SyncSender<std::result::Result<(), String>>>,
) -> Result<()> {
    let connection = Connection::from_socket(stream)?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let handle = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&handle, 1..=1, ())?;
    let device = match globals.bind::<ExtDataControlManagerV1, _, _>(&handle, 1..=1, ()) {
        Ok(manager) => {
            let device = manager.get_data_device(&seat, &handle, ());
            Device::Ext(manager, device)
        }
        Err(_) => {
            let manager: ZwlrDataControlManagerV1 = globals.bind(&handle, 1..=2, ())?;
            let device = manager.get_data_device(&seat, &handle, ());
            Device::Wlr(manager, device)
        }
    };
    let mut state = State {
        offers: HashMap::new(),
        selection: None,
        serial: 0,
        epoch: rand::random(),
        marker: format!(
            "application/x-lightos-clipboard-{:016x}",
            rand::random::<u64>()
        ),
        finished: false,
        io: tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?,
    };
    queue.roundtrip(&mut state)?;
    ensure!(!state.finished, "clipboard control refused");
    if let Some(ready) = ready.take() {
        let _ = ready.send(Ok(()));
    }
    let result = (|| -> Result<()> {
        while !stopped.load(Ordering::Acquire) {
            queue.dispatch_pending(&mut state)?;
            connection.flush()?;
            if let Some(read) = queue.prepare_read() {
                let mut fd = libc::pollfd {
                    fd: connection.as_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let n = unsafe { libc::poll(&mut fd, 1, 5) };
                if n > 0 {
                    read.read()?;
                } else if n < 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
            queue.dispatch_pending(&mut state)?;
            ensure!(!state.finished, "clipboard compositor disconnected");
            for _ in 0..16 {
                let Ok(request) = requests.try_recv() else {
                    break;
                };
                match request {
                    Request::Revision(reply) => {
                        let _ = reply.send(Ok(ClipboardRevision {
                            epoch: state.epoch,
                            serial: state.serial,
                        }));
                    }
                    Request::Read(image, mut reply) => {
                        if reply.is_closed() {
                            continue;
                        }
                        let wanted = if image {
                            vec!["image/png"]
                        } else {
                            vec!["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                        };
                        let selection = state.selection.and_then(|id| state.offers.get(&id));
                        let selected = selection.and_then(|(offer, types)| {
                            wanted
                                .into_iter()
                                .find(|mime| types.iter().any(|t| t == mime))
                                .map(|mime| (offer, mime))
                        });
                        if let Some((offer, mime)) = selected {
                            let (read, write) = pipe()?;
                            offer.receive(mime.into(), write.as_fd());
                            drop(write);
                            connection.flush()?;
                            state.io.spawn(async move {let result=tokio::select!{_=reply.closed()=>return,result=tokio::time::timeout(TIMEOUT,crate::clipboard::read_fd(read,if image{IMAGE_LIMIT}else{MAX_CLIPBOARD_BYTES}))=>result.context("Wayland clipboard transfer timed out").and_then(|r|r).map(Some)};let _=reply.send(result);});
                        } else {
                            let _ = reply.send(Ok(None));
                        }
                    }
                    Request::Write(image, bytes, reply) => {
                        if reply.is_closed() {
                            continue;
                        }
                        let types = if image {
                            vec!["image/png"]
                        } else {
                            vec!["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                        };
                        let content = Content {
                            image,
                            bytes: bytes.into(),
                        };
                        match &device {
                            Device::Ext(manager, device) => {
                                let source = manager.create_data_source(&handle, content);
                                for mime in &types {
                                    source.offer((*mime).into());
                                }
                                source.offer(state.marker.clone());
                                device.set_selection(Some(&source));
                            }
                            Device::Wlr(manager, device) => {
                                let source = manager.create_data_source(&handle, content);
                                for mime in &types {
                                    source.offer((*mime).into());
                                }
                                source.offer(state.marker.clone());
                                device.set_selection(Some(&source));
                            }
                        }
                        connection.display().sync(&handle, Mutex::new(Some(reply)));
                        connection.flush()?;
                    }
                }
            }
        }
        Ok(())
    })();
    state.io.shutdown_timeout(Duration::from_secs(1));
    result
}
