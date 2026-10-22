use crate::{
    CodecCapability, EncodedFrame, EncoderBackend, EncoderRate, EncoderSettings, VideoCodec,
    VideoEncoder, default_bitrate,
    selection::valid_avc_codec,
    worker_protocol::{self as wire, Reply, Request},
};
use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, future::BoxFuture};
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

#[derive(Clone, Copy, Debug)]
pub struct WorkerDeadlines {
    pub init: Duration,
    pub probe: Duration,
    pub encode: Duration,
}
impl Default for WorkerDeadlines {
    fn default() -> Self {
        Self {
            init: Duration::from_secs(5),
            probe: Duration::from_secs(10),
            encode: Duration::from_secs(3),
        }
    }
}
#[derive(Clone)]
pub struct HardwareEncoders {
    path: PathBuf,
    deadlines: WorkerDeadlines,
}
impl HardwareEncoders {
    /// Only the service's sibling is discovered; viewer input cannot select a binary.
    pub fn discover() -> Self {
        let path = std::env::current_exe().ok().and_then(|p| {
            p.parent().map(|p| {
                p.join(if cfg!(windows) {
                    "lightos-codec-ffmpeg.exe"
                } else {
                    "lightos-codec-ffmpeg"
                })
            })
        });
        Self::at(path.unwrap_or_default())
    }
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            deadlines: WorkerDeadlines::default(),
        }
    }
    pub fn with_deadlines(mut self, deadlines: WorkerDeadlines) -> Self {
        self.deadlines = deadlines;
        self
    }
    fn check_path(&self) -> Result<()> {
        ensure!(
            self.path.is_absolute() && self.path.is_file(),
            "optional native codec worker is not installed"
        );
        Ok(())
    }
    pub async fn probe(&self) -> Vec<CodecCapability> {
        futures_util::future::join_all(EncoderBackend::HARDWARE.map(|backend| async move {
            let result = async {
                ensure!(
                    backend.supported_platform(),
                    "backend is unsupported on this operating system"
                );
                self.check_path()?;
                let mut worker = Worker::spawn(&self.path)?;
                let result = worker
                    .exchange(Request::Probe { backend }, self.deadlines.probe)
                    .await;
                worker.stop().await;
                match result? {
                    Reply::Probe { codec } if valid_avc_codec(&codec) => Ok(codec),
                    _ => bail!("invalid native probe response"),
                }
            }
            .await;
            CodecCapability {
                codec: VideoCodec::H264,
                backend: backend.id(),
                hardware: true,
                available: result.is_ok(),
                codec_string: result.as_ref().ok().cloned(),
                reason: result
                    .as_ref()
                    .err()
                    .map(|e: &anyhow::Error| format!("{e:#}")),
                dynamic_quality: false,
                dynamic_bitrate: result.is_ok(),
                rate_update: "restart",
            }
        }))
        .await
    }
    pub fn create(
        &self,
        backend: EncoderBackend,
        settings: EncoderSettings,
    ) -> Result<Box<dyn VideoEncoder>> {
        ensure!(
            backend.is_hardware() && backend.supported_platform(),
            "invalid native hardware backend"
        );
        wire::validate_settings(settings)?;
        self.check_path()?;
        Ok(Box::new(HardwareEncoder {
            factory: self.clone(),
            backend,
            settings,
            rate: EncoderRate {
                bitrate_bps: default_bitrate(settings.width, settings.height),
                fps: settings.fps,
            },
            worker: None,
        }))
    }
}

struct Worker {
    child: Child,
    input: FramedWrite<ChildStdin, LengthDelimitedCodec>,
    output: FramedRead<ChildStdout, LengthDelimitedCodec>,
    poisoned: bool,
}
impl Worker {
    fn spawn(path: &PathBuf) -> Result<Self> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("start native codec worker")?;
        let input = FramedWrite::new(
            child.stdin.take().context("missing native stdin")?,
            LengthDelimitedCodec::builder()
                .max_frame_length(wire::MAX_MESSAGE)
                .new_codec(),
        );
        let output = FramedRead::new(
            child.stdout.take().context("missing native stdout")?,
            LengthDelimitedCodec::builder()
                .max_frame_length(wire::MAX_MESSAGE)
                .new_codec(),
        );
        Ok(Self {
            child,
            input,
            output,
            poisoned: false,
        })
    }
    async fn stop(&mut self) {
        let _ = self.child.start_kill();
        let _ = timeout(Duration::from_secs(1), self.child.wait()).await;
    }
    async fn exchange(&mut self, request: Request, deadline: Duration) -> Result<Reply> {
        if self.poisoned {
            self.stop().await;
            bail!("codec worker previous request was interrupted");
        }
        self.poisoned = true;
        let result = timeout(deadline, async {
            self.input
                .send(Bytes::from(wire::pack(&request)?))
                .await
                .context("write native codec request")?;
            let bytes = self
                .output
                .next()
                .await
                .context("native codec worker exited")??;
            let reply = wire::unpack(&bytes)?;
            if let Reply::Error(reason) = reply {
                bail!("native codec: {reason}");
            }
            Ok(reply)
        })
        .await
        .context("native codec worker timed out")
        .and_then(std::convert::identity);
        if result.is_err() {
            self.stop().await;
        } else {
            self.poisoned = false;
        }
        result
    }
}
struct HardwareEncoder {
    factory: HardwareEncoders,
    backend: EncoderBackend,
    settings: EncoderSettings,
    rate: EncoderRate,
    worker: Option<Worker>,
}
impl VideoEncoder for HardwareEncoder {
    fn dimensions(&self) -> (u32, u32) {
        (self.settings.width, self.settings.height)
    }
    fn set_rate(&mut self, rate: EncoderRate) -> Result<()> {
        rate.validate()?;
        self.rate = rate;
        Ok(())
    }
    fn encode(&mut self, pixels: Arc<[u8]>) -> BoxFuture<'_, Result<EncodedFrame>> {
        Box::pin(async move {
            wire::validate_pixels(self.settings, &pixels)?;
            if self.worker.is_none() {
                let mut worker = Worker::spawn(&self.factory.path)?;
                ensure!(
                    matches!(
                        worker
                            .exchange(
                                Request::Init {
                                    backend: self.backend,
                                    settings: self.settings
                                },
                                self.factory.deadlines.init
                            )
                            .await?,
                        Reply::Ready
                    ),
                    "invalid codec initialization response"
                );
                self.worker = Some(worker);
            }
            let worker = self.worker.as_mut().context("missing native worker")?;
            match worker
                .exchange(
                    Request::Frame {
                        pixels: pixels.to_vec(),
                        rate: self.rate,
                    },
                    self.factory.deadlines.encode,
                )
                .await?
            {
                Reply::Frame { data, key, codec } => {
                    ensure!(
                        codec.as_deref().is_none_or(valid_avc_codec),
                        "invalid native codec profile"
                    );
                    Ok(EncodedFrame { data, key, codec })
                }
                _ => {
                    worker.stop().await;
                    bail!("invalid native encode response");
                }
            }
        })
    }
}
