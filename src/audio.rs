#[cfg(any(target_os = "linux", test))]
use std::time::{Duration, Instant};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

#[cfg(target_os = "linux")]
use anyhow::ensure;
use anyhow::{Context as _, Result, bail};
use futures_util::future::BoxFuture;
use tokio::sync::Notify;

pub trait AudioCapture: Send {
    fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> BoxFuture<'a, std::io::Result<usize>>;
}

pub trait AudioCaptureFactory: Send + Sync {
    fn available(&self) -> bool;
    fn start(&self, format: AudioFormat) -> Result<Box<dyn AudioCapture>>;
}

/// A stream belongs to one privacy epoch, even across a fast lock/unlock cycle.
pub struct SessionAudio {
    source: Arc<dyn AudioCaptureFactory>,
    backend: Arc<dyn crate::backend::DesktopBackend>,
}
impl SessionAudio {
    pub fn new(
        source: Arc<dyn AudioCaptureFactory>,
        backend: Arc<dyn crate::backend::DesktopBackend>,
    ) -> Self {
        Self { source, backend }
    }
}
impl AudioCaptureFactory for SessionAudio {
    fn available(&self) -> bool {
        !self.backend.is_stopped() && !self.backend.paused() && self.source.available()
    }
    fn start(&self, format: AudioFormat) -> Result<Box<dyn AudioCapture>> {
        if !self.available() {
            bail!("audio unavailable while the desktop is locked or stopped");
        }
        let epoch = self.backend.privacy_epoch();
        let capture = self.source.start(format)?;
        let stream = PrivateAudio {
            capture: Some(capture),
            backend: self.backend.clone(),
            epoch,
        };
        if !stream.valid() {
            bail!("audio authorization changed during initialization");
        }
        Ok(Box::new(stream))
    }
}
struct PrivateAudio {
    capture: Option<Box<dyn AudioCapture>>,
    backend: Arc<dyn crate::backend::DesktopBackend>,
    epoch: u64,
}
impl PrivateAudio {
    fn valid(&self) -> bool {
        !self.backend.is_stopped()
            && !self.backend.paused()
            && self.backend.privacy_epoch() == self.epoch
    }
}
impl AudioCapture for PrivateAudio {
    fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> BoxFuture<'a, std::io::Result<usize>> {
        Box::pin(async move {
            loop {
                if !self.valid() {
                    buffer.fill(0);
                    self.capture.take();
                    return Ok(0);
                }
                let Some(capture) = self.capture.as_mut() else {
                    return Ok(0);
                };
                let read = tokio::select! {
                    result = capture.read(buffer) => Some(result),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => None,
                };
                if !self.valid() {
                    buffer.fill(0);
                    self.capture.take();
                    return Ok(0);
                }
                if let Some(result) = read {
                    return result;
                }
            }
        })
    }
}

pub struct UnavailableAudio;
impl AudioCaptureFactory for UnavailableAudio {
    fn available(&self) -> bool {
        false
    }
    fn start(&self, _: AudioFormat) -> Result<Box<dyn AudioCapture>> {
        bail!("system audio capture is unavailable in this session")
    }
}

#[cfg(target_os = "linux")]
mod pulse;

const PACKET_BYTES: usize = 3840;
type Packet = Arc<[u8]>;

#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    #[default]
    Pcm,
    Opus,
}

#[derive(Default)]
struct PacketQueue {
    state: Mutex<QueueState>,
    ready: Notify,
}

#[derive(Default)]
struct QueueState {
    packets: VecDeque<Packet>,
    closed: bool,
}

impl PacketQueue {
    fn push(&self, packet: Packet) {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        if state.packets.len() == 4 {
            state.packets.pop_front();
        }
        state.packets.push_back(packet);
        drop(state);
        self.ready.notify_one();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.packets.clear();
        drop(state);
        self.ready.notify_one();
    }
}

pub struct AudioStream {
    packets: Arc<PacketQueue>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AudioStream {
    /// Cancel capture and discard buffered audio before bounded worker cleanup.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.packets.close();
        if let Some(worker) = self.worker.take() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                std::thread::park_timeout(std::time::Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                tracing::error!(
                    "native audio API exceeded shutdown deadline; process supervision will reclaim it"
                );
            }
        }
    }
    #[cfg(target_os = "linux")]
    pub fn new(server: Option<String>, source: String) -> Result<Self> {
        Self::with_format(server, source, AudioFormat::Pcm)
    }

    #[cfg(target_os = "linux")]
    pub fn with_format(
        server: Option<String>,
        source: String,
        format: AudioFormat,
    ) -> Result<Self> {
        ensure!(
            !source.is_empty() && !source.contains('\0'),
            "invalid Pulse source"
        );
        ensure!(
            server.as_ref().is_none_or(|server| !server.contains('\0')),
            "invalid Pulse server"
        );
        let packets = Arc::new(PacketQueue::default());
        let output = packets.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let packetizer = Packetizer::with_format(format)?;
        let worker = std::thread::Builder::new()
            .name("pulse-capture".into())
            .spawn(move || {
                if let Err(error) =
                    pulse::capture(server.as_deref(), &source, &stopping, &output, packetizer)
                {
                    tracing::warn!(%error, "native audio capture stopped");
                }
                output.close();
            })
            .context("start native audio worker")?;
        Ok(Self {
            packets,
            stop,
            worker: Some(worker),
        })
    }

    /// Native providers feed interleaved 48 kHz stereo f32 little-endian PCM.
    pub fn capture(
        name: &str,
        format: AudioFormat,
        capture: impl FnOnce(&AtomicBool, &mut PcmWriter) -> Result<()> + Send + 'static,
    ) -> Result<Self> {
        let packets = Arc::new(PacketQueue::default());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let output = packets.clone();
        let packetizer = Packetizer::with_format(format)?;
        let worker = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let mut writer = PcmWriter {
                    packets: output.clone(),
                    packetizer,
                };
                if let Err(error) = capture(&stopping, &mut writer) {
                    tracing::warn!(%error, "native audio capture stopped");
                }
                output.close();
            })
            .context("start audio capture worker")?;
        Ok(Self {
            packets,
            stop,
            worker: Some(worker),
        })
    }

    pub async fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.len() < PACKET_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "audio buffer too small",
            ));
        }
        loop {
            {
                let mut state = self.packets.state.lock().unwrap();
                if let Some(packet) = state.packets.pop_front() {
                    buffer[..packet.len()].copy_from_slice(&packet);
                    return Ok(packet.len());
                }
                if state.closed {
                    return Ok(0);
                }
            }
            self.packets.ready.notified().await;
        }
    }
}

impl AudioCapture for AudioStream {
    fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> BoxFuture<'a, std::io::Result<usize>> {
        Box::pin(AudioStream::read(self, buffer))
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct PcmWriter {
    packets: Arc<PacketQueue>,
    packetizer: Packetizer,
}
impl PcmWriter {
    pub fn push(&mut self, pcm: &[u8]) -> Result<()> {
        self.packetizer.push(pcm, &self.packets)
    }
}

struct Packetizer {
    pending: Vec<u8>,
    encoder: Option<opus::Encoder>,
    samples: Vec<f32>,
    encoded: Vec<u8>,
}

impl Packetizer {
    fn with_format(format: AudioFormat) -> Result<Self> {
        let encoder = if format == AudioFormat::Opus {
            let mut encoder =
                opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio)?;
            encoder.set_bitrate(opus::Bitrate::Bits(128_000))?;
            encoder.set_vbr(false)?;
            encoder.set_complexity(5)?;
            Some(encoder)
        } else {
            None
        };
        Ok(Self {
            pending: Vec::with_capacity(PACKET_BYTES * 2),
            encoder,
            samples: vec![0.0; 1920],
            encoded: vec![0; 1275],
        })
    }

    #[cfg(test)]
    fn new() -> Self {
        Self::with_format(AudioFormat::Pcm).unwrap()
    }

    fn push(&mut self, mut data: &[u8], packets: &PacketQueue) -> Result<()> {
        let packet_bytes = if self.encoder.is_some() {
            PACKET_BYTES * 2
        } else {
            PACKET_BYTES
        };
        while !data.is_empty() {
            let length = (packet_bytes - self.pending.len()).min(data.len());
            self.pending.extend_from_slice(&data[..length]);
            data = &data[length..];
            if self.pending.len() == packet_bytes {
                if let Some(encoder) = self.encoder.as_mut() {
                    for (sample, bytes) in
                        self.samples.iter_mut().zip(self.pending.as_chunks::<4>().0)
                    {
                        let value = f32::from_le_bytes(*bytes);
                        *sample = if value.is_finite() {
                            value.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        };
                    }
                    let length = encoder.encode_float(&self.samples, &mut self.encoded)?;
                    packets.push(Arc::from(&self.encoded[..length]));
                    self.pending.clear();
                } else {
                    let ready =
                        std::mem::replace(&mut self.pending, Vec::with_capacity(PACKET_BYTES));
                    packets.push(ready.into());
                }
            }
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", test))]
    fn silence(&mut self, mut length: usize, packets: &PacketQueue) -> Result<()> {
        let silence = [0; PACKET_BYTES];
        while length > 0 {
            let count = length.min(silence.len());
            self.push(&silence[..count], packets)?;
            length -= count;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_audio_queue_discards_backlog_and_rejects_late_native_callbacks() {
        let queue = PacketQueue::default();
        queue.push(Arc::from([7u8; 4]));
        queue.close();
        queue.push(Arc::from([9u8; 4]));
        let state = queue.state.lock().unwrap();
        assert!(state.closed);
        assert!(state.packets.is_empty());
    }

    #[test]
    fn stalled_native_audio_does_not_block_stream_drop() {
        let (release, block) = std::sync::mpsc::channel();
        let stream = AudioStream::capture("test-stalled-audio", AudioFormat::Pcm, move |_, _| {
            let _ = block.recv();
            Ok(())
        })
        .unwrap();
        let started = Instant::now();
        drop(stream);
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
    }

    #[tokio::test]
    async fn privacy_epoch_discards_buffered_audio_even_after_fast_unlock() {
        use crate::backend::{BackendKind, DesktopBackend, Frame, InputCommand};
        use std::sync::atomic::AtomicU64;
        use tokio::sync::{mpsc, watch};
        struct Backend {
            epoch: AtomicU64,
            paused: AtomicBool,
        }
        impl DesktopBackend for Backend {
            fn kind(&self) -> BackendKind {
                BackendKind::X11
            }
            fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
                watch::channel(None).1
            }
            fn input(&self) -> mpsc::Sender<InputCommand> {
                mpsc::channel(1).0
            }
            fn stop(&self) {}
            fn paused(&self) -> bool {
                self.paused.load(Ordering::Acquire)
            }
            fn privacy_epoch(&self) -> u64 {
                self.epoch.load(Ordering::Acquire)
            }
        }
        struct Source(Arc<AtomicBool>);
        struct Capture(Arc<AtomicBool>);
        impl Drop for Capture {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        impl AudioCapture for Capture {
            fn read<'a>(
                &'a mut self,
                buffer: &'a mut [u8],
            ) -> BoxFuture<'a, std::io::Result<usize>> {
                Box::pin(async move {
                    buffer[..4].fill(7);
                    Ok(4)
                })
            }
        }
        impl AudioCaptureFactory for Source {
            fn available(&self) -> bool {
                true
            }
            fn start(&self, _: AudioFormat) -> Result<Box<dyn AudioCapture>> {
                Ok(Box::new(Capture(self.0.clone())))
            }
        }
        let backend = Arc::new(Backend {
            epoch: AtomicU64::new(0),
            paused: AtomicBool::new(false),
        });
        let dropped = Arc::new(AtomicBool::new(false));
        let factory = SessionAudio::new(Arc::new(Source(dropped.clone())), backend.clone());
        let mut stream = factory.start(AudioFormat::Pcm).unwrap();
        let mut buffer = [0; 8];
        assert_eq!(stream.read(&mut buffer).await.unwrap(), 4);
        backend.paused.store(true, Ordering::Release);
        assert!(!factory.available());
        backend.epoch.fetch_add(1, Ordering::AcqRel);
        backend.paused.store(false, Ordering::Release);
        assert_eq!(stream.read(&mut buffer).await.unwrap(), 0);
        assert!(dropped.load(Ordering::Acquire));
    }

    #[test]
    fn opus_encodes_twenty_milliseconds_at_bounded_bitrate_and_preserves_stereo() {
        let sender = PacketQueue::default();
        let mut packetizer = Packetizer::with_format(AudioFormat::Opus).unwrap();
        let mut decoder = opus::Decoder::new(48_000, opus::Channels::Stereo).unwrap();
        let samples: Vec<u8> = (0..960)
            .flat_map(|index| {
                let sample = (index as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin() * 0.25;
                [sample, -sample].into_iter().flat_map(f32::to_le_bytes)
            })
            .collect();
        for chunk in samples.chunks(13) {
            packetizer.push(chunk, &sender).unwrap();
        }
        let state = sender.state.lock().unwrap();
        assert_eq!(state.packets.len(), 1);
        let packet = state.packets.front().unwrap();
        assert_eq!(packet.len(), 320);
        let mut decoded = vec![0.0; 1920];
        assert_eq!(
            decoder.decode_float(packet, &mut decoded, false).unwrap(),
            960
        );
        assert!(decoded.iter().all(|sample| sample.is_finite()));
        assert!(decoded.iter().any(|sample| sample.abs() > 0.01));
        let difference: f32 = decoded
            .as_chunks::<2>()
            .0
            .iter()
            .map(|channels| (channels[0] + channels[1]).abs())
            .sum();
        assert!(difference / 960.0 < 0.02);
    }

    #[test]
    fn packetizer_preserves_alignment_and_fills_holes() {
        let sender = PacketQueue::default();
        let mut packetizer = Packetizer::new();
        packetizer.push(&[7; 3], &sender).unwrap();
        packetizer.push(&[8; 5], &sender).unwrap();
        packetizer.silence(PACKET_BYTES - 8, &sender).unwrap();
        let state = sender.state.lock().unwrap();
        let packet = state.packets.front().unwrap();
        assert_eq!(packet.len(), PACKET_BYTES);
        assert_eq!(&packet[..8], &[7, 7, 7, 8, 8, 8, 8, 8]);
        assert!(packet[8..].iter().all(|byte| *byte == 0));
        assert!(packetizer.pending.is_empty());
    }

    #[tokio::test]
    async fn audio_drops_only_excess_backlog_and_read_is_cancel_safe() {
        let sender = Arc::new(PacketQueue::default());
        let mut stream = AudioStream {
            packets: sender.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        let mut buffer = [0; PACKET_BYTES];
        assert!(
            tokio::time::timeout(Duration::from_millis(5), stream.read(&mut buffer))
                .await
                .is_err()
        );
        let mut packetizer = Packetizer::new();
        for byte in 0..10 {
            packetizer.push(&[byte; PACKET_BYTES], &sender).unwrap();
        }
        for byte in 6..10 {
            assert_eq!(stream.read(&mut buffer).await.unwrap(), PACKET_BYTES);
            assert!(buffer.iter().all(|value| *value == byte));
        }
        sender.close();
        assert_eq!(stream.read(&mut buffer).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_twenty_millisecond_fragment_delivers_both_packets() {
        let sender = Arc::new(PacketQueue::default());
        let mut stream = AudioStream {
            packets: sender.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        let mut packetizer = Packetizer::new();
        let mut data = vec![1; PACKET_BYTES];
        data.extend_from_slice(&[2; PACKET_BYTES]);
        packetizer.push(&data, &sender).unwrap();
        let mut buffer = [0; PACKET_BYTES];
        for byte in 1..=2 {
            assert_eq!(stream.read(&mut buffer).await.unwrap(), PACKET_BYTES);
            assert!(buffer.iter().all(|value| *value == byte));
        }
        assert!(sender.state.lock().unwrap().packets.is_empty());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn stop_reclaims_worker_even_while_connecting() {
        let started = Instant::now();
        for _ in 0..10 {
            let stream = AudioStream::new(
                Some("unix:/nonexistent-lightos-native-audio".into()),
                "monitor".into(),
            )
            .unwrap();
            drop(stream);
        }
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    #[ignore = "requires explicit test Pulse server/source playing a nonzero signal"]
    #[cfg(target_os = "linux")]
    async fn native_capture_receives_samples_and_reclaims_idle_workers() {
        let server = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_PULSE_SERVER").unwrap();
        let source = std::env::var("LIGHTOS_REMOTE_DESKTOP_TEST_PULSE_SOURCE").unwrap();
        let workers = || {
            std::fs::read_dir("/proc/self/task")
                .unwrap()
                .flatten()
                .filter(|entry| {
                    std::fs::read_to_string(entry.path().join("comm"))
                        .is_ok_and(|name| name.trim() == "pulse-capture")
                })
                .count()
        };
        let baseline = workers();
        for _ in 0..5 {
            let mut stream = AudioStream::new(Some(server.clone()), source.clone()).unwrap();
            let mut buffer = [0; PACKET_BYTES];
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    assert_eq!(stream.read(&mut buffer).await.unwrap(), PACKET_BYTES);
                    let samples: Vec<_> = buffer
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|bytes| f32::from_le_bytes(*bytes))
                        .collect();
                    assert!(samples.iter().all(|sample| sample.is_finite()));
                    if samples.iter().any(|sample| sample.abs() > 0.001) {
                        break;
                    }
                }
            })
            .await
            .expect("nonzero PCM samples");
            tokio::time::sleep(Duration::from_millis(50)).await;
            let stopped = Instant::now();
            drop(stream);
            assert!(stopped.elapsed() < Duration::from_millis(250));
            assert_eq!(workers(), baseline);
        }
    }
}
