use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail, ensure};
use libpulse_binding::{
    context::{Context, FlagSet as ContextFlags, State as ContextState},
    def::BufferAttr,
    mainloop::standard::{IterateResult, Mainloop},
    sample::{Format, Spec},
    stream::{FlagSet as StreamFlags, PeekResult, State as StreamState, Stream},
};
use tokio::sync::Notify;

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
        if state.packets.len() == 4 {
            state.packets.pop_front();
        }
        state.packets.push_back(packet);
        drop(state);
        self.ready.notify_one();
    }

    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_one();
    }
}

pub struct AudioStream {
    packets: Arc<PacketQueue>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AudioStream {
    pub fn new(server: Option<String>, source: String) -> Result<Self> {
        Self::with_format(server, source, AudioFormat::Pcm)
    }

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
                    capture(server.as_deref(), &source, &stopping, &output, packetizer)
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

impl Drop for AudioStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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

fn step(mainloop: &mut Mainloop) -> Result<()> {
    match mainloop.iterate(false) {
        IterateResult::Success(_) => Ok(()),
        _ => bail!("Pulse mainloop stopped"),
    }
}

fn capture(
    server: Option<&str>,
    source: &str,
    stop: &AtomicBool,
    packets: &PacketQueue,
    mut packetizer: Packetizer,
) -> Result<()> {
    let mut mainloop = Mainloop::new().context("create Pulse mainloop")?;
    let mut context =
        Context::new(&mainloop, "LightOS remote audio").context("create Pulse context")?;
    context
        .connect(server, ContextFlags::NOAUTOSPAWN, None)
        .context("connect Pulse server")?;
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            step(&mut mainloop)?;
            match context.get_state() {
                ContextState::Ready => break,
                ContextState::Failed | ContextState::Terminated => bail!("Pulse context failed"),
                _ => {}
            }
            ensure!(Instant::now() < deadline, "Pulse context timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        let spec = Spec {
            format: Format::F32le,
            channels: 2,
            rate: 48_000,
        };
        let mut stream = Stream::new(&mut context, "Desktop monitor", &spec, None)
            .context("create Pulse stream")?;
        let result = (|| {
            let attr = BufferAttr {
                maxlength: (PACKET_BYTES * 10) as u32,
                tlength: u32::MAX,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize: PACKET_BYTES as u32,
            };
            stream
                .connect_record(Some(source), Some(&attr), StreamFlags::ADJUST_LATENCY)
                .context("record Pulse monitor")?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !stop.load(Ordering::Acquire) {
                step(&mut mainloop)?;
                ensure!(
                    !matches!(
                        context.get_state(),
                        ContextState::Failed | ContextState::Terminated
                    ),
                    "Pulse context lost"
                );
                match stream.get_state() {
                    StreamState::Ready => {
                        for _ in 0..32 {
                            match stream.peek().context("peek Pulse audio")? {
                                PeekResult::Empty => break,
                                PeekResult::Data(data) => {
                                    ensure!(data.len() <= 1024 * 1024, "Pulse fragment too large");
                                    packetizer.push(data, packets)?;
                                }
                                PeekResult::Hole(length) => {
                                    ensure!(length <= 1024 * 1024, "Pulse hole too large");
                                    packetizer.silence(length, packets)?;
                                }
                            }
                            stream.discard().context("discard Pulse fragment")?;
                        }
                    }
                    StreamState::Failed | StreamState::Terminated => bail!("Pulse stream failed"),
                    _ => ensure!(Instant::now() < deadline, "Pulse stream timed out"),
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        })();
        let _ = stream.disconnect();
        result
    })();
    context.disconnect();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
