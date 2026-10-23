use super::*;
use std::sync::{Mutex, mpsc as sync_mpsc};
use std::time::{Duration, Instant};

struct MockMonitor {
    locked: AtomicBool,
    fail: AtomicBool,
}

impl MockMonitor {
    fn new(locked: bool) -> Arc<Self> {
        Arc::new(Self {
            locked: AtomicBool::new(locked),
            fail: AtomicBool::new(false),
        })
    }
}

impl LockMonitor for MockMonitor {
    fn locked(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async {
            ensure!(!self.fail.load(Ordering::Acquire), "mock lock query failed");
            Ok(self.locked.load(Ordering::Acquire))
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InputEvent {
    Key(u32),
    ReleaseStarted,
    ReleaseFinished,
}

struct MockBackend {
    frames: watch::Receiver<Option<Arc<Frame>>>,
    frame_tx: watch::Sender<Option<Arc<Frame>>>,
    input: mpsc::Sender<InputCommand>,
    stopped: Arc<AtomicBool>,
    input_worker: Mutex<Option<thread::JoinHandle<()>>>,
    clipboard_started: tokio::sync::Notify,
    clipboard_finish: tokio::sync::Notify,
    image_written: AtomicBool,
}

impl MockBackend {
    fn new(release_delay: Duration) -> (Arc<Self>, sync_mpsc::Receiver<InputEvent>) {
        let frame = Arc::new(Frame::test_frame(2, 2));
        let (frame_tx, frames) = watch::channel(Some(frame));
        let (input, mut commands) = mpsc::channel::<InputCommand>(16);
        let (events, received) = sync_mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let input_worker = thread::spawn(move || {
            while !worker_stopped.load(Ordering::Acquire) {
                match commands.try_recv().map(InputCommand::into_active) {
                    Ok(Some((InputCommand::Key { code, .. }, _))) => {
                        let _ = events.send(InputEvent::Key(code));
                    }
                    Ok(Some((InputCommand::ReleaseSync { done }, _))) => {
                        let _ = events.send(InputEvent::ReleaseStarted);
                        thread::sleep(release_delay);
                        let _ = done.send(());
                        let _ = events.send(InputEvent::ReleaseFinished);
                    }
                    Ok(_) => {}
                    Err(mpsc::error::TryRecvError::Empty) => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
        });
        (
            Arc::new(Self {
                frames,
                frame_tx,
                input,
                stopped,
                input_worker: Mutex::new(Some(input_worker)),
                clipboard_started: tokio::sync::Notify::new(),
                clipboard_finish: tokio::sync::Notify::new(),
                image_written: AtomicBool::new(false),
            }),
            received,
        )
    }
}

impl DesktopBackend for MockBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Wayland
    }
    fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.clone()
    }
    fn input(&self) -> mpsc::Sender<InputCommand> {
        self.input.clone()
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.frame_tx.send_replace(None);
        if let Some(worker) = self.input_worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
    fn cursor_target(&self) -> Result<(std::path::PathBuf, i32, Option<String>)> {
        Ok(("/mock-cursor".into(), 1, None))
    }
    fn supports_image_clipboard(&self) -> bool {
        true
    }
    fn clipboard_write_png<'a>(&'a self, _data: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            self.clipboard_started.notify_one();
            self.clipboard_finish.notified().await;
            self.image_written.store(true, Ordering::Release);
            Ok(())
        })
    }
    fn clipboard_read(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async {
            self.clipboard_started.notify_one();
            self.clipboard_finish.notified().await;
            Ok("private clipboard text".into())
        })
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(Instant::now() < deadline, "guard transition timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn release_barrier_follows_queued_input_in_fifo_order() {
    let (inner, events) = MockBackend::new(Duration::from_millis(10));
    let guard = attach_with_monitor(inner, Some(MockMonitor::new(false))).unwrap();
    guard
        .input()
        .try_send(InputCommand::Key {
            code: 30,
            pressed: true,
        })
        .unwrap();
    guard
        .input()
        .try_send(InputCommand::Key {
            code: 31,
            pressed: true,
        })
        .unwrap();
    assert!(guard.release_input());
    for expected in [
        InputEvent::Key(30),
        InputEvent::Key(31),
        InputEvent::ReleaseStarted,
        InputEvent::ReleaseFinished,
    ] {
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            expected
        );
    }
    guard.stop();
}

#[test]
fn lock_clears_frame_before_slow_release_and_unlock_resumes_same_backend() {
    let (inner, events) = MockBackend::new(Duration::from_millis(300));
    let monitor = MockMonitor::new(false);
    let guard = attach_with_monitor(inner.clone(), Some(monitor.clone())).unwrap();
    assert!(!guard.paused());
    assert!(guard.frames().borrow().is_some());
    let initial_epoch = guard.privacy_epoch();

    guard
        .input()
        .try_send(InputCommand::Key {
            code: 30,
            pressed: true,
        })
        .unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        InputEvent::Key(30)
    );
    monitor.locked.store(true, Ordering::Release);
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)).unwrap(),
        InputEvent::ReleaseStarted
    );
    assert!(guard.paused());
    assert_ne!(guard.privacy_epoch(), initial_epoch);
    assert!(
        guard.frames().borrow().is_none(),
        "frame must be hidden before slow input release"
    );
    assert!(!inner.stopped.load(Ordering::Acquire));
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        InputEvent::ReleaseFinished
    );

    guard
        .input()
        .try_send(InputCommand::Key {
            code: 31,
            pressed: true,
        })
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(events.try_recv().is_err(), "input crossed a locked desktop");
    let locked_epoch = guard.privacy_epoch();
    monitor.locked.store(false, Ordering::Release);
    wait_until(|| !guard.paused() && guard.privacy_epoch() != locked_epoch);
    assert!(
        guard.frames().borrow().is_none(),
        "pre-lock capture must stay hidden after unlock"
    );
    inner
        .frame_tx
        .send_replace(Some(Arc::new(Frame::test_frame(2, 2))));
    wait_until(|| guard.frames().borrow().is_some());
    assert_ne!(guard.privacy_epoch(), initial_epoch);
    guard
        .input()
        .try_send(InputCommand::Key {
            code: 32,
            pressed: true,
        })
        .unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        InputEvent::Key(32)
    );
    guard.stop();
    assert!(inner.stopped.load(Ordering::Acquire));
    assert!(guard.is_stopped());
    assert!(guard.frames().borrow().is_none());
}

#[test]
fn lock_cancels_text_already_running_inside_the_inner_backend() {
    struct TextBackend {
        frames: watch::Receiver<Option<Arc<Frame>>>,
        input: mpsc::Sender<InputCommand>,
        stopped: Arc<AtomicBool>,
        worker: Mutex<Option<thread::JoinHandle<()>>>,
    }
    impl DesktopBackend for TextBackend {
        fn kind(&self) -> BackendKind {
            BackendKind::Wayland
        }
        fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
            self.frames.clone()
        }
        fn input(&self) -> mpsc::Sender<InputCommand> {
            self.input.clone()
        }
        fn stop(&self) {
            self.stopped.store(true, Ordering::Release);
            if let Some(worker) = self.worker.lock().unwrap().take() {
                let _ = worker.join();
            }
        }
        fn is_stopped(&self) -> bool {
            self.stopped.load(Ordering::Acquire)
        }
    }
    let (_frames_tx, frames) = watch::channel(Some(Arc::new(Frame::test_frame(2, 2))));
    let (input, mut commands) = mpsc::channel::<InputCommand>(16);
    let (first, first_seen) = sync_mpsc::sync_channel(1);
    let (resume, continued) = sync_mpsc::sync_channel(1);
    let typed = Arc::new(Mutex::new(String::new()));
    let worker_typed = typed.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = stopped.clone();
    let worker = thread::spawn(move || {
        while !worker_stopped.load(Ordering::Acquire) {
            let command = match commands.try_recv() {
                Ok(command) => command,
                Err(mpsc::error::TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            };
            let Some((command, guard)) = command.into_active() else {
                continue;
            };
            match command {
                InputCommand::Text { text } => {
                    for (index, character) in text.chars().enumerate() {
                        if guard.as_ref().is_some_and(|guard| !guard.active()) {
                            break;
                        }
                        worker_typed.lock().unwrap().push(character);
                        if index == 0 {
                            let _ = first.send(());
                            let _ = continued.recv_timeout(Duration::from_secs(3));
                        }
                    }
                    if let Some(guard) = guard {
                        guard.text_done.notify_one();
                    }
                }
                InputCommand::ReleaseSync { done } => {
                    let _ = done.send(());
                }
                _ => {}
            }
        }
    });
    let inner = Arc::new(TextBackend {
        frames,
        input,
        stopped,
        worker: Mutex::new(Some(worker)),
    });
    let monitor = MockMonitor::new(false);
    let guard = attach_with_monitor(inner, Some(monitor.clone())).unwrap();
    guard
        .input()
        .try_send(InputCommand::Text {
            text: "private text".into(),
        })
        .unwrap();
    first_seen.recv_timeout(Duration::from_secs(1)).unwrap();
    monitor.locked.store(true, Ordering::Release);
    wait_until(|| guard.paused() && guard.frames().borrow().is_none());
    resume.send(()).unwrap();
    wait_until(|| typed.lock().unwrap().as_str() == "p");
    thread::sleep(Duration::from_millis(50));
    assert_eq!(typed.lock().unwrap().as_str(), "p");
    guard.stop();
}

#[test]
fn initial_lock_or_monitor_failure_does_not_enable_backend() {
    for (locked, failed) in [(true, false), (false, true)] {
        let (inner, _) = MockBackend::new(Duration::ZERO);
        let monitor = MockMonitor::new(locked);
        monitor.fail.store(failed, Ordering::Release);
        let error = attach_with_monitor(inner.clone(), Some(monitor))
            .err()
            .unwrap();
        assert!(error.to_string().contains("attached desktop lock guard"));
        assert!(inner.stopped.load(Ordering::Acquire));
    }
}

#[test]
fn temporary_monitor_error_pauses_until_a_successful_unlock_query() {
    let (inner, events) = MockBackend::new(Duration::from_millis(10));
    let monitor = MockMonitor::new(false);
    let guard = attach_with_monitor(inner.clone(), Some(monitor.clone())).unwrap();
    assert!(guard.frames().borrow().is_some());

    monitor.fail.store(true, Ordering::Release);
    wait_until(|| guard.paused() && guard.frames().borrow().is_none());
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        InputEvent::ReleaseStarted
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        InputEvent::ReleaseFinished
    );
    guard
        .input()
        .try_send(InputCommand::Key {
            code: 33,
            pressed: true,
        })
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(events.try_recv().is_err(), "monitor error allowed input");

    let locked_epoch = guard.privacy_epoch();
    monitor.fail.store(false, Ordering::Release);
    wait_until(|| !guard.paused() && guard.privacy_epoch() != locked_epoch);
    assert!(guard.frames().borrow().is_none());
    inner
        .frame_tx
        .send_replace(Some(Arc::new(Frame::test_frame(2, 2))));
    wait_until(|| guard.frames().borrow().is_some());
    assert!(!inner.stopped.load(Ordering::Acquire));
    guard.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clipboard_read_started_before_lock_cannot_return_text_after_lock() {
    let (inner, events) = MockBackend::new(Duration::from_millis(10));
    let monitor = MockMonitor::new(false);
    let guard = attach_with_monitor(inner.clone(), Some(monitor.clone())).unwrap();
    let reading = guard.clone();
    let task = tokio::spawn(async move { reading.clipboard_read().await });
    tokio::time::timeout(Duration::from_secs(1), inner.clipboard_started.notified())
        .await
        .unwrap();
    monitor.locked.store(true, Ordering::Release);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !guard.paused() || guard.frames().borrow().is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    inner.clipboard_finish.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.unwrap_err().to_string().contains("locked"));
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        InputEvent::ReleaseStarted
    );
    guard.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lock_blocks_cursor_source_and_cancels_image_write() {
    let (inner, _) = MockBackend::new(Duration::ZERO);
    let monitor = MockMonitor::new(false);
    let guard = attach_with_monitor(inner.clone(), Some(monitor.clone())).unwrap();
    assert!(guard.cursor_target().is_ok());
    assert!(guard.supports_image_clipboard());
    let writing = guard.clone();
    let task = tokio::spawn(async move { writing.clipboard_write_png(b"png").await });
    tokio::time::timeout(Duration::from_secs(1), inner.clipboard_started.notified())
        .await
        .unwrap();
    monitor.locked.store(true, Ordering::Release);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(guard.cursor_target().is_err());
    inner.clipboard_finish.notify_one();
    assert!(!inner.image_written.load(Ordering::Acquire));
    assert!(guard.clipboard_write_png(b"png").await.is_err());
    monitor.locked.store(false, Ordering::Release);
    tokio::time::timeout(Duration::from_secs(2), async {
        while guard.paused() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(guard.cursor_target().is_ok());
    guard.clipboard_write_png(b"png").await.unwrap();
    assert!(inner.image_written.load(Ordering::Acquire));
    guard.stop();
    assert!(guard.cursor_target().is_err());
}
