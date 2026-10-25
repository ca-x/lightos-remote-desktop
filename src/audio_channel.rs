use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::{sync::watch, time::timeout};

use crate::audio::{AudioCaptureFactory, AudioFormat};

#[derive(Default)]
pub struct AudioHub {
    next: AtomicU64,
    current: Mutex<Option<Arc<AudioSession>>>,
}

struct AudioSession {
    id: u64,
    stop: watch::Sender<bool>,
    connected: AtomicBool,
    target: Arc<dyn AudioCaptureFactory>,
    // A privacy epoch prevents queued audio from surviving a lock/unlock cycle.
    capture_epoch: Arc<dyn Fn() -> Option<u64> + Send + Sync>,
}

pub struct AudioLease {
    session: Arc<AudioSession>,
}

impl Drop for AudioLease {
    fn drop(&mut self) {
        self.session.connected.store(false, Ordering::Release);
    }
}

impl AudioLease {
    pub fn available(&self) -> bool {
        !*self.session.stop.borrow() && self.session.target.available() && (self.session.capture_epoch)().is_some()
    }
}

impl AudioHub {
    pub fn start(
        &self,
        target: Arc<dyn AudioCaptureFactory>,
        capture_epoch: Arc<dyn Fn() -> Option<u64> + Send + Sync>,
    ) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (stop, _) = watch::channel(false);
        let mut current = self.current.lock().unwrap();
        if let Some(previous) = current.take() {
            previous.stop.send_replace(true);
        }
        *current = Some(Arc::new(AudioSession {
            id,
            stop,
            connected: AtomicBool::new(false),
            target,
            capture_epoch,
        }));
        id
    }

    pub fn stop(&self) {
        if let Some(session) = self.current.lock().unwrap().take() {
            session.stop.send_replace(true);
        }
    }

    pub fn attach(&self, id: u64) -> Option<AudioLease> {
        let current = self.current.lock().unwrap();
        let session = current.as_ref()?;
        if session.id != id
            || session
                .connected
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return None;
        }
        Some(AudioLease {
            session: session.clone(),
        })
    }
}

pub async fn connection(mut socket: WebSocket, lease: AudioLease, format: AudioFormat) {
    let mut stop = lease.session.stop.subscribe();
    if *stop.borrow() {
        return;
    }
    let epoch = (lease.session.capture_epoch)();
    if epoch.is_none() {
        return;
    }
    let factory = lease.session.target.clone();
    let initialize = tokio::task::spawn_blocking(move || factory.start(format));
    let mut audio = tokio::select! {
        biased;
        _ = stop.changed() => return,
        result = initialize => match result {
            Ok(Ok(audio)) => audio,
            _ => {
                let message = Message::Text("{\"type\":\"error\",\"message\":\"Audio capture unavailable\"}".into());
                let _ = timeout(Duration::from_millis(500), socket.send(message)).await;
                return;
            }
        }
    };
    let (mut writer, mut reader) = socket.split();
    let receive = async {
        while let Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) =
            timeout(Duration::from_secs(45), reader.next()).await
        {}
    };
    let transmit = async {
        if (lease.session.capture_epoch)() != epoch {
            return;
        }
        let ready = Message::Text(serde_json::json!({"type":"audio_ready","codec":if format == AudioFormat::Opus { "opus" } else { "pcm" },"sample_rate":48000,"channels":2}).to_string().into());
        if !matches!(
            timeout(Duration::from_millis(500), writer.send(ready)).await,
            Ok(Ok(()))
        ) {
            return;
        }
        let mut buffer = vec![0_u8; 3840];
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        let started = std::time::Instant::now();
        let mut privacy_check = tokio::time::interval(Duration::from_millis(50));
        loop {
            let message = tokio::select! {
                _ = privacy_check.tick() => {
                    if (lease.session.capture_epoch)() != epoch { break; }
                    continue;
                },
                result = audio.read(&mut buffer) => match result {
                    Ok(length) if length > 0 => {
                        let mut bytes = Vec::with_capacity(length + 9);
                        if format == AudioFormat::Opus {
                            bytes.push(4);
                            bytes.extend_from_slice(&(started.elapsed().as_micros() as u64).to_le_bytes());
                        } else {
                            bytes.push(2);
                        }
                        bytes.extend_from_slice(&buffer[..length]);
                        Message::Binary(bytes.into())
                    }
                    _ => break,
                },
                _ = heartbeat.tick() => Message::Ping(bytes::Bytes::new()),
            };
            if (lease.session.capture_epoch)() != epoch {
                break;
            }
            if !matches!(
                timeout(Duration::from_millis(500), writer.send(message)).await,
                Ok(Ok(()))
            ) {
                break;
            }
        }
    };
    tokio::select! {
        biased;
        _ = stop.changed() => {},
        _ = receive => {},
        _ = transmit => {},
    }
    let _ = timeout(Duration::from_millis(500), writer.close()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAudio;
    impl AudioCaptureFactory for TestAudio {
        fn available(&self) -> bool { true }
        fn start(&self, _format: AudioFormat) -> anyhow::Result<Box<dyn crate::audio::AudioCapture>> { anyhow::bail!("test capture unavailable") }
    }
    fn target(_source: &str) -> Arc<dyn AudioCaptureFactory> { Arc::new(TestAudio) }

    fn start(hub: &AudioHub) -> u64 {
        hub.start(target("virtual.monitor"), Arc::new(|| Some(0)))
    }

    #[test]
    fn audio_source_follows_selected_controller_and_privacy() {
        let hub = AudioHub::default();
        let first = hub.attach(start(&hub)).unwrap();
        let permitted = Arc::new(AtomicBool::new(true));
        let check = permitted.clone();
        let selected = target("@DEFAULT_MONITOR@");
        let id = hub.start(
            selected.clone(),
            Arc::new(move || check.load(Ordering::Acquire).then_some(0)),
        );
        let second = hub.attach(id).unwrap();
        assert!(Arc::ptr_eq(&second.session.target, &selected));
        assert!(!Arc::ptr_eq(&first.session.target, &second.session.target));
        assert!(!first.available());
        assert!(second.available());
        permitted.store(false, Ordering::Release);
        assert!(!second.available());
        hub.stop();
        permitted.store(true, Ordering::Release);
        assert!(!second.available());
    }

    #[test]
    fn audio_requires_current_session_and_only_one_connection() {
        let hub = AudioHub::default();
        assert!(hub.attach(1).is_none());
        let id = start(&hub);
        assert!(hub.attach(id + 1).is_none());
        let lease = hub.attach(id).unwrap();
        assert!(hub.attach(id).is_none());
        drop(lease);
        assert!(hub.attach(id).is_some());
    }

    #[test]
    fn controller_close_invalidates_audio_even_before_it_subscribes() {
        let hub = AudioHub::default();
        let id = start(&hub);
        let lease = hub.attach(id).unwrap();
        hub.stop();
        assert!(*lease.session.stop.subscribe().borrow());
        assert!(hub.attach(id).is_none());
        let next = start(&hub);
        assert_ne!(next, id);
        assert!(hub.attach(id).is_none());
        assert!(hub.attach(next).is_some());
    }

    #[test]
    fn replacing_session_stops_old_audio_without_releasing_new_lease() {
        let hub = AudioHub::default();
        let old = hub.attach(start(&hub)).unwrap();
        let next = start(&hub);
        let current = hub.attach(next).unwrap();
        assert!(*old.session.stop.borrow());
        drop(old);
        assert!(hub.attach(next).is_none());
        assert!(!*current.session.stop.borrow());
    }
}
