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
}

pub struct AudioLease {
    session: Arc<AudioSession>,
}

impl Drop for AudioLease {
    fn drop(&mut self) {
        self.session.connected.store(false, Ordering::Release);
    }
}

impl AudioHub {
    pub fn start(&self) -> u64 {
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

pub async fn connection(
    mut socket: WebSocket,
    lease: AudioLease,
    factory: Arc<dyn AudioCaptureFactory>,
    backend: Arc<dyn crate::backend::DesktopBackend>,
    format: AudioFormat,
) {
    let epoch = backend.privacy_epoch();
    let authorized =
        || !backend.paused() && !backend.is_stopped() && backend.privacy_epoch() == epoch;
    let mut stop = lease.session.stop.subscribe();
    if *stop.borrow() || !authorized() {
        return;
    }
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
        loop {
            let message = tokio::select! {
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
            if !authorized() {
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

    #[test]
    fn audio_requires_current_session_and_only_one_connection() {
        let hub = AudioHub::default();
        assert!(hub.attach(1).is_none());
        let id = hub.start();
        assert!(hub.attach(id + 1).is_none());
        let lease = hub.attach(id).unwrap();
        assert!(hub.attach(id).is_none());
        drop(lease);
        assert!(hub.attach(id).is_some());
    }

    #[test]
    fn controller_close_invalidates_audio_even_before_it_subscribes() {
        let hub = AudioHub::default();
        let id = hub.start();
        let lease = hub.attach(id).unwrap();
        hub.stop();
        assert!(*lease.session.stop.subscribe().borrow());
        assert!(hub.attach(id).is_none());
        let next = hub.start();
        assert_ne!(next, id);
        assert!(hub.attach(id).is_none());
        assert!(hub.attach(next).is_some());
    }

    #[test]
    fn replacing_session_stops_old_audio_without_releasing_new_lease() {
        let hub = AudioHub::default();
        let old = hub.attach(hub.start()).unwrap();
        let next = hub.start();
        let current = hub.attach(next).unwrap();
        assert!(*old.session.stop.borrow());
        drop(old);
        assert!(hub.attach(next).is_none());
        assert!(!*current.session.stop.borrow());
    }
}
