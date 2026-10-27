//! Native media/input/clipboard for an explicitly admitted, service-owned GNOME
//! session. Physical GNOME sessions still use Portal consent and login guards.
use super::*;
use anyhow::Context;
use futures_util::{FutureExt, StreamExt};
use std::{
    collections::{BTreeSet, HashMap},
    os::fd::OwnedFd,
    sync::{Mutex, atomic::AtomicU64},
    thread::JoinHandle,
};
use tokio::{sync::oneshot, time::timeout};
use zbus::{
    Proxy,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const REMOTE: &str = "org.gnome.Mutter.RemoteDesktop";
const CAST: &str = "org.gnome.Mutter.ScreenCast";
const CALL: Duration = Duration::from_secs(3);
const MAX_IMAGE: usize = 8 * 1024 * 1024;

struct InputViewport {
    size: (i32, i32),
    retry_boundary: bool,
}

enum Control {
    PasteKeys(bool, oneshot::Sender<Vec<u32>>),
    Read(bool, oneshot::Sender<Result<Option<Vec<u8>>>>),
    Write(bool, Vec<u8>, oneshot::Sender<Result<()>>),
}
struct Shared {
    stopped: Arc<AtomicBool>,
    clipboard: AtomicBool,
    revision: AtomicU64,
    epoch: u64,
}
pub(crate) struct Backend {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    cursors: watch::Receiver<Option<Arc<CursorFrame>>>,
    input: mpsc::Sender<InputCommand>,
    control: mpsc::Sender<Control>,
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Backend {
    pub(crate) fn start(
        target: display_control::SessionTarget,
        pid: u32,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        let (frames_tx, frames) = watch::channel(None);
        let (cursors_tx, cursors) = watch::channel(None);
        let (input, commands) = mpsc::channel(128);
        let (control, requests) = mpsc::channel(8);
        let shared = Arc::new(Shared {
            stopped: Arc::new(AtomicBool::new(false)),
            clipboard: AtomicBool::new(false),
            revision: AtomicU64::new(0),
            epoch: rand::random(),
        });
        let state = shared.clone();
        let (ready, initialized) = sync_mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("owned-gnome".into())
            .spawn(move || {
                let mut ready = Some(ready);
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let result = runtime.block_on(run(
                        target,
                        pid,
                        frames_tx.clone(),
                        cursors_tx,
                        commands,
                        requests,
                        state.clone(),
                        cancelled,
                        &mut ready,
                    ));
                    runtime.shutdown_timeout(Duration::from_secs(1));
                    result
                })();
                state.stopped.store(true, Ordering::Release);
                frames_tx.send_replace(None);
                if let Some(ready) = ready {
                    let _ = ready.send(result.map_err(|e| format!("{e:#}")));
                } else if let Err(error) = result {
                    tracing::warn!(%error, "owned GNOME session stopped");
                }
            })?;
        match initialized.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(())) => Ok(Self {
                frames,
                cursors,
                input,
                control,
                shared,
                worker: Mutex::new(Some(worker)),
            }),
            result => {
                shared.stopped.store(true, Ordering::Release);
                let _ = worker.join();
                bail!("owned GNOME did not start: {result:?}");
            }
        }
    }
    async fn read(&self, image: bool) -> Result<Option<Vec<u8>>> {
        let (done, response) = oneshot::channel();
        timeout(CALL, self.control.send(Control::Read(image, done))).await??;
        timeout(CALL, response).await??
    }
    async fn write(&self, image: bool, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len()
                <= if image {
                    MAX_IMAGE
                } else {
                    MAX_CLIPBOARD_BYTES
                },
            "clipboard too large"
        );
        let (done, response) = oneshot::channel();
        timeout(
            CALL,
            self.control
                .send(Control::Write(image, bytes.to_vec(), done)),
        )
        .await??;
        timeout(CALL, response).await??
    }
}
impl DesktopBackend for Backend {
    fn clipboard_text_input(&self) -> bool {
        true
    }
    fn text_paste_keycodes(&self, shift: bool) -> BoxFuture<'_, Vec<u32>> {
        Box::pin(async move {
            let (done, response) = oneshot::channel();
            let request = async {
                self.control
                    .send(Control::PasteKeys(shift, done))
                    .await
                    .ok()?;
                response.await.ok()
            };
            timeout(CALL, request)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| super::linux_text_paste_keys(shift, false))
        })
    }
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
    fn frame_is_fresh(&self, _max_age: Duration) -> bool {
        // PipeWire stops publishing pixels on a static desktop. The session
        // Closed/error signals and worker lifetime establish liveness instead.
        !self.is_stopped() && self.frames.borrow().is_some()
    }
    fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }
    fn supports_image_clipboard(&self) -> bool {
        self.shared.clipboard.load(Ordering::Acquire)
    }
    fn supports_clipboard_sync(&self) -> bool {
        self.supports_image_clipboard()
    }
    fn clipboard_revision(&self) -> BoxFuture<'_, Result<ClipboardRevision>> {
        Box::pin(async {
            ensure!(
                self.supports_clipboard_sync() && !self.is_stopped(),
                "GNOME clipboard unavailable"
            );
            Ok(ClipboardRevision {
                epoch: self.shared.epoch,
                serial: self.shared.revision.load(Ordering::Acquire),
            })
        })
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async {
            String::from_utf8(self.read(false).await?.context("clipboard has no text")?)
                .context("clipboard is not UTF-8")
        })
    }
    fn clipboard_read_png(&self) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        Box::pin(self.read(true))
    }
    fn clipboard_write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.write(false, text.as_bytes()))
    }
    fn clipboard_write_png<'a>(&'a self, image: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.write(true, image))
    }
    fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn proxy(
    bus: &zbus::Connection,
    owner: &str,
    path: &str,
    interface: &str,
) -> Result<Proxy<'static>> {
    Ok(Proxy::new_owned(
        bus.clone(),
        owner.to_owned(),
        path.to_owned(),
        interface.to_owned(),
    )
    .await?)
}
async fn owner(bus: &zbus::Connection, name: &str, pid: u32) -> Result<String> {
    let api = zbus::fdo::DBusProxy::new(bus).await?;
    let owner = api.get_name_owner(name.try_into()?).await?;
    ensure!(
        api.get_connection_unix_process_id(owner.as_str().try_into()?)
            .await?
            == pid,
        "GNOME service does not belong to the admitted compositor"
    );
    Ok(owner.to_string())
}
async fn cancelled(shared: &Shared, requested: &AtomicBool) {
    while !shared.stopped.load(Ordering::Acquire) && !requested.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[allow(clippy::too_many_arguments)] // All channels are scoped to this one owned compositor.
async fn run(
    target: display_control::SessionTarget,
    pid: u32,
    frames: watch::Sender<Option<Arc<Frame>>>,
    cursors: watch::Sender<Option<Arc<CursorFrame>>>,
    mut input: mpsc::Receiver<InputCommand>,
    mut control: mpsc::Receiver<Control>,
    shared: Arc<Shared>,
    requested: Arc<AtomicBool>,
    ready: &mut Option<sync_mpsc::SyncSender<std::result::Result<(), String>>>,
) -> Result<()> {
    let cursor_position = cursors.subscribe();
    let bus = timeout(
        CALL,
        zbus::connection::Builder::address(
            target.bus_address.as_deref().context("GNOME bus missing")?,
        )?
        .build(),
    )
    .await??;
    let rd_owner = timeout(CALL, owner(&bus, REMOTE, pid)).await??;
    let cast_owner = timeout(CALL, owner(&bus, CAST, pid)).await??;
    let api = proxy(&bus, &rd_owner, "/org/gnome/Mutter/RemoteDesktop", REMOTE).await?;
    let path: OwnedObjectPath = timeout(CALL, api.call("CreateSession", &())).await??;
    let session = proxy(&bus, &rd_owner, path.as_str(), &format!("{REMOTE}.Session")).await?;
    let mut closed = session.receive_signal("Closed").await?;
    let mut capture = None;
    let mut held_keys = BTreeSet::new();
    let mut held_buttons = BTreeSet::new();
    let operation = async {
        let start = async {
            let id: String = session.get_property("SessionId").await?;
            let cast = proxy(&bus, &cast_owner, "/org/gnome/Mutter/ScreenCast", CAST).await?;
            ensure!(
                cast.get_property::<i32>("Version").await? >= 2,
                "GNOME does not support independent cursor metadata"
            );
            let props = HashMap::from([("remote-desktop-session-id", Value::from(id.as_str()))]);
            let path: OwnedObjectPath = cast.call("CreateSession", &(props,)).await?;
            let cast = proxy(&bus, &cast_owner, path.as_str(), &format!("{CAST}.Session")).await?;
            let monitor = loop {
                let selected = target.clone();
                let outputs = tokio::task::spawn_blocking(move || {
                    use display_control::DisplayController;
                    display_control::MutterDisplay::new(selected).outputs()
                })
                .await?;
                if let Ok(outputs) = outputs
                    && let Some(output) = outputs.into_iter().next()
                {
                    break output.name;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            };
            let props = HashMap::from([("cursor-mode", Value::from(2_u32))]);
            let path: OwnedObjectPath = cast.call("RecordMonitor", &(monitor, props)).await?;
            let stream = proxy(&bus, &cast_owner, path.as_str(), &format!("{CAST}.Stream")).await?;
            let parameters: HashMap<String, OwnedValue> = stream.get_property("Parameters").await?;
            let size: (i32, i32) = parameters
                .get("size")
                .context("GNOME stream size missing")?
                .try_clone()?
                .try_into()?;
            ensure!(
                size.0 > 0 && size.1 > 0 && size.0 <= 8192 && size.1 <= 8192,
                "invalid GNOME stream size"
            );
            let mut nodes = stream.receive_signal("PipeWireStreamAdded").await?;
            let owner = session.receive_signal("SelectionOwnerChanged").await?;
            let transfer = session.receive_signal("SelectionTransfer").await?;
            let enabled = session
                .call::<_, _, ()>("EnableClipboard", &(HashMap::<&str, Value>::new(),))
                .await
                .is_ok();
            tracing::info!("starting owned GNOME capture session");
            session.call::<_, _, ()>("Start", &()).await?;
            tracing::info!("owned GNOME session started; waiting for PipeWire node");
            let message = tokio::select! {
                message = timeout(CALL, nodes.next()) => message.context("GNOME did not announce a PipeWire node")?.context("GNOME stream closed")?,
                _ = closed.next() => bail!("GNOME closed capture during startup"),
            };
            let (node,): (u32,) = message.body().deserialize()?;
            tracing::info!(node, "owned GNOME PipeWire node ready");
            let connection =
                std::os::unix::net::UnixStream::connect(target.runtime.join("pipewire-0"))?;
            connection.set_nonblocking(true)?;
            let fd: OwnedFd = connection.into();
            let (failed, failure) = watch::channel(None);
            let captured = frames.clone();
            let stopped = shared.stopped.clone();
            capture = Some(
                std::thread::Builder::new()
                    .name("gnome-pipewire".into())
                    .spawn(move || {
                        if let Err(error) = super::portal::capture_pipewire(
                            fd,
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
            let mut first = frames.subscribe();
            loop {
                if first.borrow().is_some() {
                    break;
                }
                if let Some(error) = failure.borrow().as_ref() {
                    bail!("{error}");
                }
                tokio::select! {result=first.changed()=>result?,_=closed.next()=>bail!("GNOME closed capture before its first frame"),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
            }
            shared.clipboard.store(enabled, Ordering::Release);
            Ok::<_, anyhow::Error>((path, size, owner, transfer, failure))
        };
        let (stream, size, mut owners, mut transfers, failure) =
            timeout(Duration::from_secs(12), start)
                .await
                .context("GNOME startup timed out")??;
        // Drain the initial selection before admitting a client, so connecting
        // does not copy an old remote clipboard over the local one.
        let _: String = timeout(CALL, session.get_property("SessionId")).await??;
        let mut mime_types: Vec<String> = Vec::new();
        while let Some(Some(event)) = owners.next().now_or_never() {
            let (mut props,): (HashMap<String, OwnedValue>,) = event.body().deserialize()?;
            mime_types = clipboard_types(props.remove("mime-types"));
        }
        if let Some(ready) = ready.take() {
            let _ = ready.send(Ok(()));
        }
        let mut selection: Option<(bool, Arc<[u8]>)> = None;
        let mut legacy_focus: Option<super::x11::Backend> = None;
        let mut work = tokio::task::JoinSet::new();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _=tick.tick()=>{if let Some(error)=failure.borrow().as_ref(){bail!("{error}");}},
                _=closed.next()=>bail!("GNOME remote session closed"),
                Some(command)=input.recv()=>{
                    command.validate()?;
                    if let Some((command,guard))=command.into_active(){
                        let command = if let InputCommand::Text { text } = command {
                            release(&session,&mut held_keys,&mut held_buttons).await?;
                            selection=Some((false,text.into_bytes().into()));
                            let props=HashMap::from([("mime-types",Value::from(vec!["text/plain;charset=utf-8","text/plain"]))]);
                            timeout(CALL,session.call::<_,_,()>("SetSelection",&(props,))).await??;
                            InputCommand::Chord { codes: paste_keys(&mut legacy_focus,pid,&target,shared.stopped.clone(),false).await }
                        } else { command };
                        if guard.as_ref().is_some_and(|guard| !guard.active()) { if let Some(guard)=guard {guard.text_done.notify_one();} continue; }
                        // NotifyPointerMotionAbsolute consumes stream pixels; the
                        // creation-time Parameters size is logical and can also
                        // describe a transient 1px devkit output during startup.
                        let live_size=frames.borrow().as_ref().map(|frame|(frame.width as i32,frame.height as i32)).unwrap_or(size);
                        let retry_boundary=cursor_position.borrow().as_ref().is_none_or(|cursor| cursor.visible && (
                            cursor.x*f64::from(cursor.units_width)<=2.0 || cursor.y*f64::from(cursor.units_height)<=2.0 ||
                            (1.0-cursor.x)*f64::from(cursor.units_width)<=2.0 || (1.0-cursor.y)*f64::from(cursor.units_height)<=2.0));
                        let viewport=InputViewport { size: live_size, retry_boundary };
                        let result=timeout(CALL,apply_input(&session,&stream,viewport,command,guard.as_ref(),&mut held_keys,&mut held_buttons)).await;
                        if let Some(guard)=guard{guard.text_done.notify_one();}
                        result??;
                    }
                },
                Some(request)=control.recv()=>match request {
                    Control::PasteKeys(shift,reply)=>{
                        if !reply.is_closed(){let keys=paste_keys(&mut legacy_focus,pid,&target,shared.stopped.clone(),shift).await;let _=reply.send(keys);}
                    },
                    Control::Read(image,mut reply)=>{
                        if reply.is_closed(){continue;}
                        let wanted=if image {vec!["image/png"]} else {vec!["text/plain;charset=utf-8","text/plain","UTF8_STRING"]};
                        let mime=wanted.into_iter().find(|mime|mime_types.iter().any(|value|value==mime)).map(str::to_owned);
                        let session=session.clone();let own=selection.clone().filter(|(kind,_)|*kind==image);
                        work.spawn(async move {
                            let result=async {
                                if let Some((_,bytes))=own{return Ok(Some(bytes.to_vec()));}
                                let Some(mime)=mime else{return Ok(None);};
                                let fd:zbus::zvariant::OwnedFd=session.call("SelectionRead",&(mime,)).await?;
                                let limit=if image{MAX_IMAGE}else{MAX_CLIPBOARD_BYTES};
                                Ok::<_,anyhow::Error>(Some(crate::clipboard::read_fd(fd.into(),limit).await?))
                            };
                            let result=tokio::select!{_=reply.closed()=>return,result=timeout(CALL,result)=>result.context("GNOME clipboard read timed out").and_then(|r|r)};
                            let _=reply.send(result);
                        });
                    },
                    Control::Write(image,bytes,reply)=>{
                        if reply.is_closed(){continue;}
                        let types=if image{vec!["image/png"]}else{vec!["text/plain;charset=utf-8","text/plain"]};
                        let props=HashMap::from([("mime-types",Value::from(types))]);
                        selection=Some((image,bytes.into()));
                        let result=timeout(CALL,session.call::<_,_,()>("SetSelection",&(props,))).await.context("GNOME clipboard write timed out").and_then(|r|r.map_err(Into::into));
                        if result.is_err()||reply.is_closed(){selection=None;}
                        let _=reply.send(result);
                    },
                },
                event=owners.next()=>{
                    let event=event.context("GNOME clipboard observer closed")?;
                    let (mut props,):(HashMap<String,OwnedValue>,)=event.body().deserialize()?;
                    mime_types=clipboard_types(props.remove("mime-types"));
                    if !props.get("session-is-owner").is_some_and(|v|bool::try_from(v).unwrap_or(false)) {
                        selection=None;shared.revision.fetch_add(1,Ordering::AcqRel);
                    }
                },
                event=transfers.next()=>{
                    let event=event.context("GNOME clipboard transfer channel closed")?;
                    let (mime,serial):(String,u32)=event.body().deserialize()?;
                    let bytes=selection.as_ref().filter(|(image,_)|if *image{mime=="image/png"}else{matches!(mime.as_str(),"text/plain"|"text/plain;charset=utf-8")}).map(|(_,bytes)|bytes.clone());
                    let session=session.clone();work.spawn(async move {
                        let result=timeout(CALL,async {
                            let bytes=bytes.context("clipboard offer expired")?;
                            let fd:zbus::zvariant::OwnedFd=session.call("SelectionWrite",&(serial,)).await?;
                            crate::clipboard::write_fd(fd.into(),&bytes).await
                        }).await;
                        let success=matches!(result,Ok(Ok(())));
                        let _=timeout(CALL,session.call::<_,_,()>("SelectionWriteDone",&(serial,success))).await;
                    });
                },
                _=work.join_next(),if !work.is_empty()=>{},
            }
        }
    };
    let result = tokio::select! {result=operation=>result,_=cancelled(&shared,&requested)=>Ok(())};
    shared.stopped.store(true, Ordering::Release);
    let _ = timeout(CALL, release(&session, &mut held_keys, &mut held_buttons)).await;
    let _ = timeout(CALL, session.call::<_, _, ()>("Stop", &())).await;
    if let Some(capture) = capture {
        let _ = capture.join();
    }
    frames.send_replace(None);
    result
}

async fn release(
    session: &Proxy<'_>,
    keys: &mut BTreeSet<u32>,
    buttons: &mut BTreeSet<i32>,
) -> Result<()> {
    for key in std::mem::take(keys) {
        session
            .call::<_, _, ()>("NotifyKeyboardKeycode", &(key, false))
            .await?;
    }
    for button in std::mem::take(buttons) {
        session
            .call::<_, _, ()>("NotifyPointerButton", &(button, false))
            .await?;
    }
    Ok(())
}
async fn apply_input(
    session: &Proxy<'_>,
    stream: &OwnedObjectPath,
    viewport: InputViewport,
    command: InputCommand,
    guard: Option<&InputGuard>,
    keys: &mut BTreeSet<u32>,
    buttons: &mut BTreeSet<i32>,
) -> Result<()> {
    match command {
        InputCommand::Move { x, y } => {
            let coordinates = (
                stream.as_str(),
                x * f64::from(viewport.size.0 - 1),
                y * f64::from(viewport.size.1 - 1),
            );
            session
                .call::<_, _, ()>("NotifyPointerMotionAbsolute", &coordinates)
                .await?;
            if viewport.retry_boundary {
                // GNOME's edge barriers can consume the first absolute move
                // from a corner. Retry once after an input frame before any
                // queued click; ordinary in-window motion has no extra delay.
                tokio::time::sleep(Duration::from_millis(16)).await;
                if guard.is_none_or(InputGuard::active) {
                    session
                        .call::<_, _, ()>("NotifyPointerMotionAbsolute", &coordinates)
                        .await?;
                }
            }
        }
        InputCommand::Button { button, pressed } => {
            let code = [0x110, 0x112, 0x111][button as usize];
            if if pressed {
                buttons.insert(code)
            } else {
                buttons.remove(&code)
            } {
                session
                    .call::<_, _, ()>("NotifyPointerButton", &(code, pressed))
                    .await?;
            }
        }
        InputCommand::Key { code, pressed } => {
            if if pressed {
                keys.insert(code)
            } else {
                keys.remove(&code)
            } {
                session
                    .call::<_, _, ()>("NotifyKeyboardKeycode", &(code, pressed))
                    .await?;
            }
        }
        InputCommand::Scroll { x, y } => {
            session
                .call::<_, _, ()>("NotifyPointerAxis", &(x, y, 1_u32))
                .await?
        }
        InputCommand::Release => release(session, keys, buttons).await?,
        InputCommand::ReleaseSync { done } => {
            release(session, keys, buttons).await?;
            let _ = done.send(());
        }
        InputCommand::Chord { codes } => {
            release(session, keys, buttons).await?;
            for code in codes {
                session
                    .call::<_, _, ()>("NotifyKeyboardKeycode", &(code, true))
                    .await?;
                keys.insert(code);
            }
            release(session, keys, buttons).await?;
        }
        InputCommand::Text { .. } => bail!("committed text must use the prepared selection"),
        InputCommand::Guarded { .. } => unreachable!(),
    }
    Ok(())
}

// Mutter versions have emitted both `as` and `(as)` inside the variant.
// Accept the actual wire forms without treating a valid offer as empty.
fn clipboard_types(value: Option<OwnedValue>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Ok(copy) = value.try_clone()
        && let Ok(types) = Vec::<String>::try_from(copy)
    {
        return types;
    }
    <(Vec<String>,)>::try_from(value)
        .map(|(types,)| types)
        .unwrap_or_default()
}

async fn paste_keys(
    legacy: &mut Option<super::x11::Backend>,
    pid: u32,
    target: &display_control::SessionTarget,
    stopped: Arc<AtomicBool>,
    shift: bool,
) -> Vec<u32> {
    if shift {
        return super::linux_text_paste_keys(true, false);
    }
    if let Some(app) = crate::owned_wayland::focused_app(&target.runtime, pid) {
        return super::linux_text_paste_keys(false, super::terminal_app(&app));
    }
    if legacy.as_ref().is_some_and(|backend| backend.is_stopped()) {
        legacy.take();
    }
    if legacy.is_none()
        && let Ok(target) = crate::owned_wayland::xwayland_target(pid, &target.runtime)
    {
        *legacy = super::x11::Backend::clipboard_only(target, stopped).ok();
    }
    let terminal = if let Some(legacy) = legacy {
        legacy
            .active_app()
            .await
            .is_ok_and(|app| super::terminal_app(&app))
    } else {
        false
    };
    super::linux_text_paste_keys(false, terminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_clipboard_mime_variants_emitted_by_mutter() {
        let values = vec!["text/plain", "image/png"];
        let array = OwnedValue::try_from(Value::from(values.clone())).unwrap();
        let wrapped = OwnedValue::try_from(Value::from((values.clone(),))).unwrap();
        assert_eq!(clipboard_types(Some(array)), values);
        assert_eq!(clipboard_types(Some(wrapped)), values);
    }
}
