use std::{process::Stdio, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub key: bool,
    pub codec: Option<String>,
}

pub struct H264Encoder {
    pub width: u32,
    pub height: u32,
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    initialized: bool,
}

impl H264Encoder {
    pub fn new(width: u32, height: u32, fps: u32) -> Result<Self> {
        ensure!(
            width >= 320 && height >= 200 && width <= 3840 && height <= 2160,
            "invalid encoder dimensions"
        );
        ensure!(
            width.is_multiple_of(2) && height.is_multiple_of(2),
            "H.264 requires even dimensions; using JPEG"
        );
        let bitrate = (u64::from(width) * u64::from(height) * 3 / 1000).clamp(600, 18_000);
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
            ])
            .arg(format!("{width}x{height}"))
            .arg("-framerate")
            .arg(fps.to_string())
            .args([
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-profile:v",
                "baseline",
                "-pix_fmt",
                "yuv420p",
                "-bf",
                "0",
                "-threads",
                "2",
                "-b:v",
            ])
            .arg(format!("{bitrate}k"))
            .arg("-maxrate")
            .arg(format!("{bitrate}k"))
            .arg("-bufsize")
            .arg(format!("{}k", bitrate / 2))
            .args([
                "-x264-params",
                "repeat-headers=1:aud=1:scenecut=0:keyint=120:rc-lookahead=0:sync-lookahead=0",
                "-flush_packets",
                "1",
                "-f",
                "avi",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("start low-latency H.264 encoder")?;
        let input = child.stdin.take().context("encoder stdin missing")?;
        let output = child.stdout.take().context("encoder stdout missing")?;
        Ok(Self {
            width,
            height,
            child,
            input,
            output,
            initialized: false,
        })
    }

    pub async fn encode(&mut self, pixels: &[u8]) -> Result<EncodedFrame> {
        ensure!(
            pixels.len() == self.width as usize * self.height as usize * 4,
            "invalid RGBA frame length"
        );
        timeout(Duration::from_secs(5), async {
            self.input.write_all(pixels).await?;
            if !self.initialized {
                self.read_header().await?;
                self.initialized = true;
            }
            let data = self.read_packet().await?;
            let (key, codec) = annex_b_info(&data);
            Ok(EncodedFrame { data, key, codec })
        })
        .await
        .context("H.264 encode timed out")?
    }

    async fn read_header(&mut self) -> Result<()> {
        let mut header = [0_u8; 12];
        self.output.read_exact(&mut header).await?;
        ensure!(
            &header[..4] == b"RIFF" && &header[8..] == b"AVI ",
            "invalid encoder container"
        );
        for _ in 0..64 {
            let (kind, length) = self.chunk_header().await?;
            if &kind == b"LIST" {
                ensure!(length >= 4, "invalid AVI list");
                let mut list = [0; 4];
                self.output.read_exact(&mut list).await?;
                if &list == b"movi" {
                    return Ok(());
                }
                self.skip(length - 4 + length % 2).await?;
            } else {
                self.skip(length + length % 2).await?;
            }
        }
        bail!("encoder media list missing")
    }

    async fn chunk_header(&mut self) -> Result<([u8; 4], usize)> {
        let mut header = [0; 8];
        self.output.read_exact(&mut header).await?;
        Ok((
            header[..4].try_into()?,
            u32::from_le_bytes(header[4..].try_into()?) as usize,
        ))
    }

    async fn skip(&mut self, length: usize) -> Result<()> {
        ensure!(length <= 16 * 1024 * 1024, "encoder header too large");
        let mut remaining = length;
        let mut buffer = [0_u8; 4096];
        while remaining > 0 {
            let count = remaining.min(buffer.len());
            self.output.read_exact(&mut buffer[..count]).await?;
            remaining -= count;
        }
        Ok(())
    }

    async fn read_packet(&mut self) -> Result<Vec<u8>> {
        for _ in 0..64 {
            let (kind, length) = self.chunk_header().await?;
            ensure!(length <= 16 * 1024 * 1024, "encoded frame too large");
            if matches!(&kind, b"00dc" | b"00db") && length > 0 {
                let mut data = vec![0; length];
                self.output.read_exact(&mut data).await?;
                if length % 2 != 0 {
                    self.skip(1).await?;
                }
                return Ok(data);
            }
            self.skip(length + length % 2).await?;
        }
        bail!("encoder packet missing")
    }
}

impl Drop for H264Encoder {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn annex_b_info(data: &[u8]) -> (bool, Option<String>) {
    let mut key = false;
    let mut codec = None;
    for index in 0..data.len().saturating_sub(3) {
        if data.get(index..index + 3) != Some(&[0, 0, 1]) {
            continue;
        }
        let position = index + 3;
        let kind = data[position] & 0x1f;
        if kind == 5 {
            key = true;
        }
        if kind == 7 && position + 3 < data.len() {
            codec = Some(format!(
                "avc1.{:02x}{:02x}{:02x}",
                data[position + 1],
                data[position + 2],
                data[position + 3]
            ));
        }
    }
    (key, codec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annex_b_detects_codec_and_idr_without_assuming_start_code_length() {
        let (key, codec) = annex_b_info(&[0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x1f, 0, 0, 1, 0x65, 10]);
        assert!(key);
        assert_eq!(codec.as_deref(), Some("avc1.42c01f"));
        assert_eq!(annex_b_info(&[0, 0, 1, 0x41, 10]), (false, None));
    }

    #[tokio::test]
    #[ignore = "requires ffmpeg with libx264; validates demand-driven framed output"]
    async fn encoder_emits_an_initial_keyframe_then_predictive_frame_without_lookahead() {
        let mut encoder = H264Encoder::new(640, 360, 20).unwrap();
        let mut pixels = vec![255; 640 * 360 * 4];
        let first = encoder.encode(&pixels).await.unwrap();
        assert!(first.key);
        assert!(first.codec.is_some());
        pixels[0] = 0;
        let second = encoder.encode(&pixels).await.unwrap();
        assert!(!second.data.is_empty());
        assert!(!second.key);
        println!(
            "H.264 keyframe={} bytes, delta={} bytes",
            first.data.len(),
            second.data.len()
        );
    }
}
