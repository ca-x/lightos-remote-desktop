//! Observe selection changes without reading clipboard content. Copy shortcuts
//! can then synchronize a fresh selection instead of overwriting it with stale
//! client clipboard data.
use super::*;
use crate::backend::ClipboardRevision;
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct State {
    serial: u64,
    finished: bool,
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
delegate_noop!(State: ignore ZwlrDataControlManagerV1);
delegate_noop!(State: ignore ZwlrDataControlOfferV1);
impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::Selection { id } => {
                state.serial = state.serial.wrapping_add(1);
                if let Some(offer) = id {
                    offer.destroy();
                }
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                offer.destroy();
            }
            zwlr_data_control_device_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }
    wayland_client::event_created_child!(State, ZwlrDataControlDeviceV1, [0 => (ZwlrDataControlOfferV1, ())]);
}

type Probe = tokio::sync::oneshot::Sender<ClipboardRevision>;
pub(super) struct Observer {
    probes: sync_mpsc::Sender<Probe>,
    stopped: Arc<AtomicBool>,
    socket: UnixStream,
    worker: Option<JoinHandle<()>>,
}
impl Observer {
    pub(super) fn start(path: &Path, shared: Arc<Shared>) -> Result<Self> {
        let stream = connect(
            path,
            Instant::now() + Duration::from_secs(1),
            Some(&shared.stopped),
        )?;
        let socket = stream.try_clone()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let cancelled = stopped.clone();
        let (probes, requests) = sync_mpsc::channel::<Probe>();
        let (ready, completed) = sync_mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("wayland-clipboard-events".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let connection = Connection::from_socket(stream)?;
                    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
                    let handle = queue.handle();
                    let seat: wl_seat::WlSeat = globals.bind(&handle, 1..=1, ())?;
                    let manager: ZwlrDataControlManagerV1 = globals.bind(&handle, 1..=2, ())?;
                    let device = manager.get_data_device(&seat, &handle, ());
                    let mut state = State {
                        serial: 0,
                        finished: false,
                    };
                    queue.roundtrip(&mut state)?;
                    ensure!(!state.finished, "clipboard observation was refused");
                    let epoch = NEXT.fetch_add(1, Ordering::Relaxed);
                    let _ = ready.send(Ok(()));
                    while !cancelled.load(Ordering::Acquire)
                        && !shared.stopped.load(Ordering::Acquire)
                    {
                        let probe = match requests.recv_timeout(Duration::from_millis(20)) {
                            Ok(probe) => Some(probe),
                            Err(sync_mpsc::RecvTimeoutError::Timeout) => None,
                            Err(sync_mpsc::RecvTimeoutError::Disconnected) => break,
                        };
                        queue.roundtrip(&mut state)?;
                        ensure!(!state.finished, "clipboard observer ended");
                        if let Some(probe) = probe {
                            let _ = probe.send(ClipboardRevision {
                                epoch,
                                serial: state.serial,
                            });
                        }
                    }
                    device.destroy();
                    manager.destroy();
                    let _ = connection.flush();
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = ready.send(Err(format!("{error:#}")));
                }
                cancelled.store(true, Ordering::Release);
            })?;
        let observer = Self {
            probes,
            stopped,
            socket,
            worker: Some(worker),
        };
        completed
            .recv_timeout(Duration::from_secs(1))
            .context("clipboard observer startup timed out")?
            .map_err(anyhow::Error::msg)?;
        Ok(observer)
    }
    pub(super) fn probe(&self) -> Result<tokio::sync::oneshot::Receiver<ClipboardRevision>> {
        ensure!(
            !self.stopped.load(Ordering::Acquire),
            "clipboard observer stopped"
        );
        let (done, completed) = tokio::sync::oneshot::channel();
        self.probes
            .send(done)
            .context("clipboard observer closed")?;
        Ok(completed)
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.socket.shutdown(Shutdown::Both);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
