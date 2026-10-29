use crate::{
    CodecCapability, EncodedFrame, EncoderBackend, EncoderRate, EncoderSettings, VideoCodec,
    VideoEncoder, default_bitrate,
    selection::valid_avc_codec,
    worker_protocol::{self as wire, Reply, RequestRef as Request},
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
                let mut worker = Worker::spawn(&self.path, None)?;
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
    pub(crate) fn identity(&self) -> Vec<(PathBuf, Option<String>)> {
        let mut paths = vec![self.path.clone()];
        #[cfg(target_os = "linux")]
        {
            paths.extend([
                PathBuf::from("/dev/nvidia0"),
                PathBuf::from("/proc/driver/nvidia/version"),
            ]);
            if let Ok(entries) = std::fs::read_dir("/dev/dri") {
                paths.extend(
                    entries
                        .filter_map(Result::ok)
                        .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
                        .map(|entry| entry.path()),
                );
            }
        }
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let signature = std::fs::metadata(&path).ok().map(|metadata| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        format!(
                            "{}:{}:{}:{}:{}:{:?}",
                            metadata.dev(),
                            metadata.ino(),
                            metadata.rdev(),
                            metadata.mode(),
                            metadata.len(),
                            metadata.modified().ok()
                        )
                    }
                    #[cfg(not(unix))]
                    {
                        format!("{}:{:?}", metadata.len(), metadata.modified().ok())
                    }
                });
                (path, signature)
            })
            .collect()
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
    #[cfg(target_os = "linux")]
    shared: Option<wire::shared_frame::SharedFrame>,
    child: Child,
    input: FramedWrite<ChildStdin, LengthDelimitedCodec>,
    output: FramedRead<ChildStdout, LengthDelimitedCodec>,
    poisoned: bool,
}
impl Worker {
    fn spawn(path: &PathBuf, frame_size: Option<usize>) -> Result<Self> {
        let mut command = Command::new(path);
        #[cfg(target_os = "linux")]
        let shared = {
            use wire::shared_frame::{CHILD_FD, ENV, NEGOTIATE_ENV, SharedFrame};
            let shared = frame_size.and_then(|length| SharedFrame::new(length).ok());
            command.env_remove(ENV);
            command.env_remove(NEGOTIATE_ENV);
            if let Some(frame) = &shared {
                let fd = frame.fd();
                command.env(ENV, CHILD_FD.to_string());
                command.env(NEGOTIATE_ENV, "1");
                // SAFETY: only async-signal-safe descriptor operations run
                // between fork and exec. The parent keeps the original file.
                unsafe {
                    command.pre_exec(move || {
                        if (fd != CHILD_FD && libc::dup2(fd, CHILD_FD) < 0)
                            || libc::fcntl(CHILD_FD, libc::F_SETFD, 0) < 0
                        {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
            }
            shared
        };
        #[cfg(not(target_os = "linux"))]
        let _ = frame_size;
        let mut child = command
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
            #[cfg(target_os = "linux")]
            shared,
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
    async fn exchange(&mut self, request: Request<'_>, deadline: Duration) -> Result<Reply> {
        if self.poisoned {
            self.stop().await;
            bail!("codec worker previous request was interrupted");
        }
        self.poisoned = true;
        let result = timeout(deadline, async {
            #[cfg(target_os = "linux")]
            let request = match request {
                Request::Frame { pixels, rate } if self.shared.is_some() => {
                    self.shared.as_mut().unwrap().write(pixels)?;
                    Request::Frame { pixels: &[], rate }
                }
                request => request,
            };
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
                let frame_size = self.settings.width as usize * self.settings.height as usize * 4;
                let mut worker = Worker::spawn(&self.factory.path, Some(frame_size))?;
                let reply = worker
                    .exchange(
                        Request::Init {
                            backend: self.backend,
                            settings: self.settings,
                        },
                        self.factory.deadlines.init,
                    )
                    .await?;
                ensure!(
                    matches!(reply, Reply::Ready | Reply::ReadySharedFrame),
                    "invalid codec initialization response"
                );
                #[cfg(target_os = "linux")]
                if matches!(reply, Reply::ReadySharedFrame) {
                    ensure!(
                        worker.shared.is_some(),
                        "worker acknowledged an unavailable shared frame"
                    );
                } else {
                    // Older workers ignore the inherited descriptor. Continue
                    // sending complete pixels over the compatible v1 protocol.
                    worker.shared = None;
                }
                #[cfg(not(target_os = "linux"))]
                ensure!(
                    matches!(reply, Reply::Ready),
                    "unexpected shared-frame acknowledgement"
                );
                self.worker = Some(worker);
            }
            let worker = self.worker.as_mut().context("missing native worker")?;
            match worker
                .exchange(
                    Request::Frame {
                        pixels: &pixels,
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
