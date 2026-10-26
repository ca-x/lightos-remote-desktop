//! Shared native host runtime. OS providers own capture/session APIs; enigo and
//! arboard own native input and clipboard integration.
use super::*;
use crate::backend::{BackendAvailability, Frame, InputCommand, MAX_CLIPBOARD_BYTES};
use anyhow::{Context, bail};
use display_control::{DisplayCapabilities, DisplayMode};
use enigo::{Axis, Button, Direction, Enigo, Keyboard, Mouse, Settings};
use futures_util::future::BoxFuture;
use std::{
    collections::BTreeSet,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

pub struct NativeHost {
    platform: Arc<dyn NativePlatform>,
}
impl NativeHost {
    pub fn new(platform: Arc<dyn NativePlatform>) -> Self {
        Self { platform }
    }
}
struct NativeSession {
    platform: Arc<dyn NativePlatform>,
    key: String,
}
impl SessionLifecycle for NativeSession {
    fn healthy(&mut self) -> bool {
        self.platform.session_key().is_ok_and(|key| key == self.key)
    }
    fn is_owned(&self) -> bool {
        false
    }
}
struct NativeDisplays(Arc<dyn NativePlatform>);
impl DisplayController for NativeDisplays {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        self.0.displays()
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            DisplayCapabilities::unavailable(
                "physical desktop resolution is controlled by the local user",
            )
        })
    }
    fn resize<'a>(
        &'a self,
        _: Option<&'a str>,
        _: DisplayMode,
        _: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        Box::pin(async { bail!("resize requires an owned virtual desktop") })
    }
}

impl HostPlatform for NativeHost {
    fn physical_displays(&self, _: &Config) -> Result<Arc<dyn DisplayController>> {
        Ok(Arc::new(NativeDisplays(self.platform.clone())))
    }
    fn physical_key(&self, _: &Config) -> Result<String> {
        self.platform.session_key()
    }
    fn supports_virtual(&self, _: &Config) -> bool {
        false
    }
    fn probe_physical(
        &self,
        config: &Config,
        cancelled: Option<&AtomicBool>,
    ) -> Result<BackendReport> {
        let result = probe_native(self.platform.clone(), config.fps, cancelled);
        Ok(BackendReport {
            selected: result.as_ref().ok().map(|_| self.platform.kind()),
            candidates: vec![BackendAvailability {
                backend: self.platform.kind(),
                available: result.is_ok(),
                requires_consent: false,
                reason: result.err().map(|e| format!("{e:#}")),
            }],
        })
    }
    fn open(&self, config: &Config, cancelled: Arc<AtomicBool>) -> Result<HostSession> {
        self.open_physical(config, None, cancelled, tokio::sync::oneshot::channel().0)
    }
    fn open_physical(
        &self,
        config: &Config,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
        _: tokio::sync::oneshot::Sender<()>,
    ) -> Result<HostSession> {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop startup cancelled"
        );
        self.platform.permissions()?.require_granted()?;
        ensure!(
            self.platform.session_active()?,
            "desktop is locked or unavailable"
        );
        let key = self.platform.session_key()?;
        let outputs = self.platform.displays()?;
        let selected = match output.as_deref() {
            Some(name) => outputs.iter().find(|o| o.name == name),
            None => outputs.first(),
        }
        .context("selected physical display unavailable")?
        .clone();
        let backend = Arc::new(NativeBackend::start(
            self.platform.clone(),
            key.clone(),
            selected.clone(),
            config.fps,
            cancelled,
        )?);
        Ok(HostSession {
            backend,
            displays: Arc::new(NativeDisplays(self.platform.clone())),
            lifecycle: Box::new(NativeSession {
                platform: self.platform.clone(),
                key,
            }),
            audio: self.platform.audio(),
            output: Some(selected.name),
            outputs,
        })
    }
}

struct Shared {
    stopped: AtomicBool,
    paused: AtomicBool,
    epoch: AtomicU64,
    frames: watch::Sender<Option<Arc<Frame>>>,
    output: Mutex<DisplayOutput>,
    publish_gate: Mutex<()>,
}
impl Shared {
    fn pause(&self) {
        let _gate = self.publish_gate.lock().unwrap_or_else(|e| e.into_inner());
        if !self.paused.swap(true, Ordering::AcqRel) {
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
        self.frames.send_replace(None);
    }
    fn stop(&self) {
        let _gate = self.publish_gate.lock().unwrap_or_else(|e| e.into_inner());
        self.stopped.store(true, Ordering::Release);
        self.paused.store(true, Ordering::Release);
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.frames.send_replace(None);
    }
    fn publish(&self, mut frame: Frame, epoch: u64) -> bool {
        let _gate = self.publish_gate.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopped.load(Ordering::Acquire) || self.epoch.load(Ordering::Acquire) != epoch {
            return false;
        }
        self.paused.store(false, Ordering::Release);
        self.frames.send_if_modified(|latest| {
            let changed = latest.as_ref().is_none_or(|previous| {
                previous.width != frame.width
                    || previous.height != frame.height
                    || previous.pixels.as_ref() != frame.pixels.as_ref()
            });
            if !changed && let Some(previous) = latest.as_ref() {
                frame.pixels = previous.pixels.clone();
                frame.jpeg = previous.jpeg.clone();
            }
            *latest = Some(Arc::new(frame));
            changed
        });
        true
    }
}
const WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(2);
const STATIC_RECAPTURE_INTERVAL: Duration = Duration::from_secs(2);

pub struct NativeBackend {
    platform: Arc<dyn NativePlatform>,
    shared: Arc<Shared>,
    input: mpsc::Sender<InputCommand>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}
impl NativeBackend {
    fn start(
        platform: Arc<dyn NativePlatform>,
        key: String,
        output: DisplayOutput,
        fps: u32,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        let (frames, _) = watch::channel(None);
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(true),
            epoch: AtomicU64::new(0),
            frames,
            output: Mutex::new(output),
            publish_gate: Mutex::new(()),
        });
        let (input, receive) = mpsc::channel(64);
        let (ready, started) = std::sync::mpsc::sync_channel(2);
        let backend = Self {
            platform: platform.clone(),
            shared: shared.clone(),
            input,
            workers: Mutex::new(Vec::new()),
        };
        let capture_platform = platform.clone();
        let capture_state = shared.clone();
        let capture_key = key.clone();
        let capture_cancelled = cancelled.clone();
        let capture_ready = ready.clone();
        let worker = std::thread::Builder::new()
            .name("native-capture".into())
            .spawn(move || {
                let result = run_capture(
                    capture_platform.as_ref(),
                    &capture_key,
                    fps,
                    &capture_state,
                    &capture_cancelled,
                    capture_ready.clone(),
                );
                if let Err(error) = result {
                    let _ = capture_ready.try_send(Err(anyhow::anyhow!("{error:#}")));
                    tracing::warn!(%error, "native capture stopped");
                }
                capture_state.stop();
            })?;
        backend.workers.lock().unwrap().push(worker);
        let worker = std::thread::Builder::new()
            .name("native-input".into())
            .spawn(move || {
                let result = run_input(
                    platform.as_ref(),
                    &key,
                    &shared,
                    receive,
                    &cancelled,
                    ready.clone(),
                );
                if let Err(error) = result {
                    let _ = ready.try_send(Err(anyhow::anyhow!("{error:#}")));
                    tracing::warn!(%error, "native input stopped");
                }
                shared.stop();
            })?;
        backend.workers.lock().unwrap().push(worker);
        let deadline = Instant::now() + Duration::from_secs(8);
        for _ in 0..2 {
            started
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .context("native desktop first frame/input initialization timed out")??;
        }
        Ok(backend)
    }
    fn clipboard_ready(&self) -> Result<()> {
        ensure!(
            !self.is_stopped() && !self.paused() && self.platform.session_active()?,
            "desktop locked or unavailable"
        );
        self.platform.permissions()?.require_granted()
    }
}
impl DesktopBackend for NativeBackend {
    fn paste_keycodes(&self, shift: bool) -> Vec<u32> {
        let modifier = if cfg!(target_os = "macos") { 125 } else { 29 };
        if shift {
            vec![modifier, 42, 47]
        } else {
            vec![modifier, 47]
        }
    }
    fn kind(&self) -> BackendKind {
        self.platform.kind()
    }
    fn cursor_embedded(&self) -> bool {
        true
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.shared.frames.subscribe()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn stop(&self) {
        self.shared.stop();
        let deadline = Instant::now() + WORKER_STOP_TIMEOUT;
        let workers = std::mem::take(&mut *self.workers.lock().unwrap_or_else(|e| e.into_inner()));
        for worker in workers {
            join_until(worker, deadline);
        }
    }
    fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }
    fn paused(&self) -> bool {
        self.shared.paused.load(Ordering::Acquire)
    }
    fn privacy_epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async {
            self.clipboard_ready()?;
            let text = tokio::task::spawn_blocking(|| -> Result<String> {
                Ok(arboard::Clipboard::new()?.get_text()?)
            })
            .await??;
            self.clipboard_ready()?;
            ensure!(
                text.len() <= MAX_CLIPBOARD_BYTES,
                "clipboard text too large"
            );
            Ok(text)
        })
    }
    fn clipboard_write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.clipboard_ready()?;
            ensure!(
                text.len() <= MAX_CLIPBOARD_BYTES,
                "clipboard text too large"
            );
            let text = text.to_owned();
            let platform = self.platform.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                ensure!(platform.session_active()?, "desktop locked");
                platform.permissions()?.require_granted()?;
                arboard::Clipboard::new()?.set_text(text)?;
                Ok(())
            })
            .await?
        })
    }
}
impl Drop for NativeBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

fn validate_frame(frame: &Frame) -> Result<()> {
    ensure!(
        frame.width > 0
            && frame.height > 0
            && frame.width <= 16384
            && frame.height <= 16384
            && frame.pixels.len() == frame.width as usize * frame.height as usize * 4,
        "invalid native RGBA frame"
    );
    Ok(())
}

static PROBE_BUSY: AtomicBool = AtomicBool::new(false);
fn probe_native(
    platform: Arc<dyn NativePlatform>,
    fps: u32,
    cancelled: Option<&AtomicBool>,
) -> Result<()> {
    ensure!(
        !cancelled.is_some_and(|c| c.load(Ordering::Acquire)),
        "probe cancelled"
    );
    ensure!(
        PROBE_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok(),
        "native capability probe is already running or awaiting OS recovery"
    );
    let (reply, response) = std::sync::mpsc::sync_channel(1);
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let worker = std::thread::Builder::new()
        .name("native-capability-probe".into())
        .spawn(move || {
            struct Release;
            impl Drop for Release {
                fn drop(&mut self) {
                    PROBE_BUSY.store(false, Ordering::Release);
                }
            }
            let _release = Release;
            let result = (|| -> Result<()> {
                platform.permissions()?.require_granted()?;
                platform.probe_input()?;
                ensure!(platform.session_active()?, "desktop locked or unavailable");
                let output = platform
                    .displays()?
                    .into_iter()
                    .next()
                    .context("no physical display")?;
                let mut capture = platform.capture(&output, fps)?;
                let deadline = Instant::now() + Duration::from_secs(3);
                while !stop.load(Ordering::Acquire) {
                    if let Some(frame) = capture.next_frame(Duration::from_millis(50))? {
                        validate_frame(&frame)?;
                        return Ok(());
                    }
                    ensure!(
                        Instant::now() < deadline,
                        "screen capture produced no validated frame"
                    );
                }
                bail!("native capability probe cancelled")
            })();
            let _ = reply.send(result);
        });
    let worker = match worker {
        Ok(worker) => worker,
        Err(error) => {
            PROBE_BUSY.store(false, Ordering::Release);
            return Err(error.into());
        }
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        match response.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => break result,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!("native capability probe worker stopped"));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if cancelled.is_some_and(|c| c.load(Ordering::Acquire)) {
            break Err(anyhow::anyhow!("probe cancelled"));
        }
        if Instant::now() >= deadline {
            break Err(anyhow::anyhow!("native capability probe timed out"));
        }
    };
    stopping.store(true, Ordering::Release);
    join_until(worker, Instant::now() + Duration::from_millis(25));
    result
}

pub(super) fn probe_input() -> Result<()> {
    let _input = Enigo::new(&Settings {
        open_prompt_to_get_permissions: false,
        ..Default::default()
    })?;
    Ok(())
}

fn join_until(worker: JoinHandle<()>, deadline: Instant) {
    while !worker.is_finished() && Instant::now() < deadline {
        std::thread::park_timeout(Duration::from_millis(5));
    }
    if worker.is_finished() {
        let _ = worker.join();
    } else {
        tracing::error!(
            "native API worker exceeded shutdown deadline; host supervision will reclaim the process"
        );
    }
}

fn session_ready(platform: &dyn NativePlatform, key: &str) -> Result<bool> {
    ensure!(
        platform.session_key()? == key,
        "interactive user session changed"
    );
    Ok(platform.session_active()? && platform.permissions()?.require_granted().is_ok())
}
fn keep_running(shared: &Shared, cancelled: &AtomicBool) -> bool {
    !shared.stopped.load(Ordering::Acquire) && !cancelled.load(Ordering::Acquire)
}

fn run_capture(
    platform: &dyn NativePlatform,
    key: &str,
    fps: u32,
    shared: &Shared,
    cancelled: &AtomicBool,
    ready: std::sync::mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    let mut capture: Option<Box<dyn NativeCapture>> = None;
    let mut ready = Some(ready);
    let mut capture_epoch = shared.epoch.load(Ordering::Acquire);
    let mut checked_display = Instant::now();
    let mut last_frame = Instant::now();
    while keep_running(shared, cancelled) {
        if !session_ready(platform, key)? {
            shared.pause();
            capture.take();
            std::thread::park_timeout(Duration::from_millis(25));
            continue;
        }
        if checked_display.elapsed() >= Duration::from_millis(500) {
            let old = shared
                .output
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let current = platform
                .displays()?
                .into_iter()
                .find(|o| o.name == old.name)
                .context("selected display disconnected")?;
            if current.width != old.width
                || current.height != old.height
                || current.x != old.x
                || current.y != old.y
                || current.scale != old.scale
            {
                shared.pause();
                *shared.output.lock().unwrap_or_else(|e| e.into_inner()) = current;
                capture.take();
            }
            checked_display = Instant::now();
        }
        if capture.is_none() || shared.epoch.load(Ordering::Acquire) != capture_epoch {
            let output = shared
                .output
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            capture.take();
            capture_epoch = shared.epoch.load(Ordering::Acquire);
            capture = Some(platform.capture(&output, fps)?);
            last_frame = Instant::now();
        }
        let epoch = capture_epoch;
        let next = capture
            .as_mut()
            .context("native capture missing")?
            .next_frame(Duration::from_millis(10))?;
        if !keep_running(shared, cancelled) {
            break;
        }
        if !session_ready(platform, key)? {
            shared.pause();
            capture.take();
            continue;
        }
        if let Some(mut frame) = next {
            validate_frame(&frame)?;
            frame.cursor_embedded = true;
            if shared.publish(frame, epoch) {
                last_frame = Instant::now();
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
            }
        } else if last_frame.elapsed() >= STATIC_RECAPTURE_INTERVAL {
            // Event-driven sources can be quiet or stalled. Reopening requests a
            // genuine fresh frame instead of inferring health from an empty queue.
            capture.take();
        }
    }
    Ok(())
}

fn discard_input(command: InputCommand) {
    match command {
        InputCommand::Guarded { guard, .. } => guard.text_done.notify_one(),
        InputCommand::ReleaseSync { done } => {
            let _ = done.send(());
        }
        _ => {}
    }
}
fn run_input(
    platform: &dyn NativePlatform,
    key: &str,
    shared: &Shared,
    mut input: mpsc::Receiver<InputCommand>,
    cancelled: &AtomicBool,
    ready: std::sync::mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    let mut enigo = Enigo::new(&Settings {
        open_prompt_to_get_permissions: false,
        ..Default::default()
    })?;
    ready
        .send(Ok(()))
        .map_err(|_| anyhow::anyhow!("native startup cancelled"))?;
    let mut held = Held::default();
    let mut was_active = false;
    let result = (|| -> Result<()> {
        while keep_running(shared, cancelled) {
            let permitted = session_ready(platform, key)?;
            if !permitted {
                shared.pause();
            }
            let active = permitted && !shared.paused.load(Ordering::Acquire);
            if !active || !was_active {
                held.release(&mut enigo)?;
                // Do not replay input queued while locked or before first capture.
                while let Ok(command) = input.try_recv() {
                    discard_input(command);
                }
            }
            was_active = active;
            if !active {
                std::thread::park_timeout(Duration::from_millis(10));
                continue;
            }
            let command = match input.try_recv() {
                Ok(command) => command,
                Err(mpsc::error::TryRecvError::Empty) => {
                    std::thread::park_timeout(Duration::from_millis(5));
                    continue;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            };
            if let InputCommand::ReleaseSync { done } = command {
                held.release(&mut enigo)?;
                let _ = done.send(());
                continue;
            }
            if matches!(command, InputCommand::Release) {
                held.release(&mut enigo)?;
                continue;
            }
            let Some((command, guard)) = command.into_active() else {
                continue;
            };
            if let Err(error) = validate_command(&command) {
                if let Some(guard) = guard {
                    guard.text_done.notify_one();
                }
                tracing::debug!(%error, "unsupported native input ignored");
                continue;
            }
            let output = shared
                .output
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let active = || {
                keep_running(shared, cancelled)
                    && !shared.paused.load(Ordering::Acquire)
                    && guard.as_ref().is_none_or(|g| g.active())
                    && session_ready(platform, key).unwrap_or(false)
            };
            let result = held.apply(platform, &mut enigo, &command, &output, active);
            let still_active = active();
            if let Some(guard) = guard {
                guard.text_done.notify_one();
            }
            result?;
            if !still_active {
                shared.pause();
                held.release(&mut enigo)?;
            }
        }
        Ok(())
    })();
    let release = held.release(&mut enigo);
    while let Ok(command) = input.try_recv() {
        discard_input(command);
    }
    result.and(release)
}

#[derive(Default)]
struct Held {
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u32>,
    scroll_x: f64,
    scroll_y: f64,
}
fn keycode(code: u32) -> Result<u16> {
    let mapping = keycode::KeyMap::from_key_mapping(keycode::KeyMapping::Evdev(code as u16))
        .map_err(|_| anyhow::anyhow!("unsupported native key"))?;
    #[cfg(target_os = "windows")]
    let raw = mapping.win;
    #[cfg(target_os = "windows")]
    ensure!(raw != 0, "key has no Windows mapping");
    #[cfg(target_os = "macos")]
    let raw = mapping.mac;
    ensure!(raw != 0xffff, "key has no native mapping");
    Ok(raw)
}
fn validate_command(command: &InputCommand) -> Result<()> {
    command.validate()?;
    match command {
        InputCommand::Key { code, .. } => {
            keycode(*code)?;
        }
        InputCommand::Chord { codes } => {
            for code in codes {
                keycode(*code)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn button(code: u32) -> Result<Button> {
    match code {
        0 => Ok(Button::Left),
        1 => Ok(Button::Middle),
        2 => Ok(Button::Right),
        _ => bail!("unsupported mouse button"),
    }
}
impl Held {
    fn release(&mut self, enigo: &mut Enigo) -> Result<()> {
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        let mut result = Ok(());
        for code in std::mem::take(&mut self.keys) {
            if let Err(error) = enigo.raw(code, Direction::Release) {
                result = Err(error.into());
            }
        }
        for code in std::mem::take(&mut self.buttons) {
            if let Err(error) = enigo.button(button(code)?, Direction::Release) {
                result = Err(error.into());
            }
        }
        result
    }
    fn apply(
        &mut self,
        platform: &dyn NativePlatform,
        enigo: &mut Enigo,
        command: &InputCommand,
        output: &DisplayOutput,
        active: impl Fn() -> bool,
    ) -> Result<()> {
        if !active() {
            return Ok(());
        }
        match command {
            InputCommand::Move { x, y } => {
                #[cfg(target_os = "macos")]
                let scale = output.scale;
                #[cfg(target_os = "windows")]
                let scale = 1.0;
                let x =
                    output.x + (x * output.width.saturating_sub(1) as f64 / scale).round() as i32;
                let y =
                    output.y + (y * output.height.saturating_sub(1) as f64 / scale).round() as i32;
                platform.move_pointer(enigo, x, y)?;
            }
            InputCommand::Key { code, pressed } => {
                let raw = keycode(*code)?;
                enigo.raw(
                    raw,
                    if *pressed {
                        Direction::Press
                    } else {
                        Direction::Release
                    },
                )?;
                if *pressed {
                    self.keys.insert(raw);
                } else {
                    self.keys.remove(&raw);
                }
            }
            InputCommand::Button {
                button: code,
                pressed,
            } => {
                enigo.button(
                    button(*code)?,
                    if *pressed {
                        Direction::Press
                    } else {
                        Direction::Release
                    },
                )?;
                if *pressed {
                    self.buttons.insert(*code);
                } else {
                    self.buttons.remove(code);
                }
            }
            InputCommand::Chord { codes } => {
                let mapped = codes
                    .iter()
                    .map(|c| keycode(*c))
                    .collect::<Result<Vec<_>>>()?;
                let newly_pressed: Vec<_> = mapped
                    .into_iter()
                    .filter(|raw| !self.keys.contains(raw))
                    .collect();
                for raw in newly_pressed.iter().copied() {
                    if !active() {
                        break;
                    }
                    enigo.raw(raw, Direction::Press)?;
                    self.keys.insert(raw);
                }
                for raw in newly_pressed.into_iter().rev() {
                    enigo.raw(raw, Direction::Release)?;
                    self.keys.remove(&raw);
                }
            }
            InputCommand::Scroll { x, y } => {
                #[cfg(target_os = "macos")]
                {
                    enigo
                        .smooth_scroll(scroll_steps(&mut self.scroll_y, *y, 1.0), Axis::Vertical)?;
                    enigo.smooth_scroll(
                        scroll_steps(&mut self.scroll_x, *x, 1.0),
                        Axis::Horizontal,
                    )?;
                }
                #[cfg(target_os = "windows")]
                {
                    enigo.scroll(scroll_steps(&mut self.scroll_y, *y, 100.0), Axis::Vertical)?;
                    enigo.scroll(
                        scroll_steps(&mut self.scroll_x, *x, 100.0),
                        Axis::Horizontal,
                    )?;
                }
            }
            InputCommand::Text { text } => {
                for character in text.chars() {
                    if !active() {
                        break;
                    }
                    let key = match character {
                        '\n' | '\r' => enigo::Key::Return,
                        '\t' => enigo::Key::Tab,
                        c => enigo::Key::Unicode(c),
                    };
                    enigo.key(key, Direction::Click)?;
                }
            }
            InputCommand::Release | InputCommand::ReleaseSync { .. } => self.release(enigo)?,
            InputCommand::Guarded { .. } => bail!("nested input authorization"),
        }
        Ok(())
    }
}

fn scroll_steps(remainder: &mut f64, pixels: f64, unit: f64) -> i32 {
    *remainder += pixels;
    let steps = (*remainder / unit).trunc() as i32;
    *remainder -= f64::from(steps) * unit;
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evdev_keys_are_translated_before_native_injection() {
        #[cfg(target_os = "windows")]
        assert_eq!(keycode(30).unwrap(), 0x1e);
        #[cfg(target_os = "macos")]
        assert_eq!(keycode(30).unwrap(), 0);
        assert!(keycode(767).is_err());
    }
    struct TestPlatform {
        permitted: bool,
        invalid_frame: bool,
        input_available: bool,
        started: std::sync::atomic::AtomicUsize,
    }
    impl PermissionProvider for TestPlatform {
        fn permissions(&self) -> Result<Permissions> {
            Ok(Permissions {
                screen: if self.permitted {
                    PermissionState::Granted
                } else {
                    PermissionState::Denied
                },
                input: PermissionState::Granted,
                reason: None,
            })
        }
    }
    impl NativePlatform for TestPlatform {
        fn kind(&self) -> BackendKind {
            #[cfg(target_os = "windows")]
            {
                BackendKind::Windows
            }
            #[cfg(target_os = "macos")]
            {
                BackendKind::Macos
            }
        }
        fn probe_input(&self) -> Result<()> {
            ensure!(self.input_available, "native input initialization failed");
            Ok(())
        }
        fn session_key(&self) -> Result<String> {
            Ok("test-session".into())
        }
        fn session_active(&self) -> Result<bool> {
            Ok(true)
        }
        fn displays(&self) -> Result<Vec<DisplayOutput>> {
            Ok(vec![DisplayOutput {
                name: "primary".into(),
                description: "Test output".into(),
                width: 2,
                height: 2,
                x: 0,
                y: 0,
                scale: 1.0,
            }])
        }
        fn capture(&self, _: &DisplayOutput, _: u32) -> Result<Box<dyn NativeCapture>> {
            self.started.fetch_add(1, Ordering::Relaxed);
            struct Capture(bool);
            impl NativeCapture for Capture {
                fn next_frame(&mut self, _: Duration) -> Result<Option<Frame>> {
                    Ok(Some(Frame::new(
                        2,
                        2,
                        vec![0; if self.0 { 12 } else { 16 }].into(),
                    )))
                }
            }
            Ok(Box::new(Capture(self.invalid_frame)))
        }
        fn audio(&self) -> Arc<dyn AudioCaptureFactory> {
            Arc::new(crate::audio::UnavailableAudio)
        }
    }
    #[test]
    fn probe_requires_real_valid_pixels_and_denial_never_opens_capture() {
        use clap::Parser;
        let config =
            Config::try_parse_from(["test", "--password", "native-test-password"]).unwrap();
        for (permitted, invalid_frame, input_available, available) in [
            (false, false, true, false),
            (true, true, true, false),
            (true, false, false, false),
            (true, false, true, true),
        ] {
            let platform = Arc::new(TestPlatform {
                permitted,
                invalid_frame,
                input_available,
                started: Default::default(),
            });
            let host = NativeHost::new(platform.clone());
            let report = host.probe_physical(&config, None).unwrap();
            assert_eq!(report.require_available().is_ok(), available);
            assert_eq!(
                platform.started.load(Ordering::Relaxed),
                usize::from(permitted && input_available)
            );
            assert!(!host.supports_virtual(&config));
        }
    }
    #[test]
    fn stale_pixels_are_not_renewed_by_empty_capture_polls_and_stop_rejects_publish() {
        let mut frame = Frame::new(2, 2, vec![0; 16].into());
        frame.captured_at = Instant::now() - Duration::from_secs(20);
        let (frames, _) = watch::channel(Some(Arc::new(frame)));
        let (input, _) = mpsc::channel(1);
        let output = TestPlatform {
            permitted: true,
            invalid_frame: false,
            input_available: true,
            started: Default::default(),
        }
        .displays()
        .unwrap()
        .remove(0);
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            frames,
            output: Mutex::new(output),
            publish_gate: Mutex::new(()),
        });
        let backend = NativeBackend {
            platform: Arc::new(TestPlatform {
                permitted: true,
                invalid_frame: false,
                input_available: true,
                started: Default::default(),
            }),
            shared: shared.clone(),
            input,
            workers: Mutex::new(Vec::new()),
        };
        assert!(!backend.frame_is_fresh(Duration::from_secs(3)));
        assert!(shared.publish(Frame::new(2, 2, vec![0; 16].into()), 0));
        assert!(backend.frame_is_fresh(Duration::from_secs(3)));
        shared.pause();
        assert!(!shared.publish(Frame::new(2, 2, vec![0; 16].into()), 0));
        assert!(backend.frames().borrow().is_none());
        backend.stop();
        assert!(!shared.publish(
            Frame::new(2, 2, vec![0; 16].into()),
            shared.epoch.load(Ordering::Acquire)
        ));
    }
    #[test]
    fn blocking_native_api_does_not_defeat_join_deadline() {
        let (release, block) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = block.recv();
        });
        let started = Instant::now();
        join_until(worker, Instant::now() + Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
    }
    #[test]
    fn scroll_converts_browser_pixels_and_keeps_fractional_motion() {
        let mut remainder = 0.0;
        assert_eq!(scroll_steps(&mut remainder, 25.0, 100.0), 0);
        assert_eq!(scroll_steps(&mut remainder, 75.0, 100.0), 1);
        assert_eq!(scroll_steps(&mut remainder, -250.0, 100.0), -2);
        assert_eq!(remainder, -50.0);
        let mut points = 0.0;
        assert_eq!(scroll_steps(&mut points, 0.6, 1.0), 0);
        assert_eq!(scroll_steps(&mut points, 0.6, 1.0), 1);
    }
    #[test]
    fn malformed_capture_is_rejected() {
        assert!(validate_frame(&Frame::new(2, 2, vec![0; 12].into())).is_err());
        assert!(validate_frame(&Frame::new(2, 2, vec![0; 16].into())).is_ok());
    }
}
