//! Encoder implementations and runtime availability, independent of capture and network policy.
mod hardware;
mod jpeg;
mod openh264;
mod selection;
pub mod worker_protocol;
pub use hardware::{HardwareEncoders, WorkerDeadlines};
pub use selection::{BackendPreference, EncoderBackend, choose_h264_backend};

use anyhow::{Context, Result};
use futures_util::future::BoxFuture;
pub use jpeg::encode_jpeg;
pub use openh264::{EncodedFrame, H264Encoder, h264_packet_info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Jpeg,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct EncoderSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderRate {
    pub bitrate_bps: u32,
    pub fps: u32,
}
impl EncoderRate {
    pub fn validate(self) -> Result<()> {
        anyhow::ensure!((1..=60).contains(&self.fps), "invalid native frame rate");
        anyhow::ensure!(
            (1_000..=100_000_000).contains(&self.bitrate_bps),
            "invalid native bitrate"
        );
        Ok(())
    }
}
pub fn default_bitrate(width: u32, height: u32) -> u32 {
    (u64::from(width) * u64::from(height) * 3).clamp(600_000, 18_000_000) as u32
}

#[derive(Clone, Debug, Serialize)]
pub struct CodecCapability {
    pub codec: VideoCodec,
    pub backend: &'static str,
    pub hardware: bool,
    pub available: bool,
    pub reason: Option<String>,
    pub dynamic_quality: bool,
    pub dynamic_bitrate: bool,
    pub codec_string: Option<String>,
    pub rate_update: &'static str,
}

pub trait VideoEncoder: Send {
    fn set_rate(&mut self, _rate: EncoderRate) -> Result<()> {
        anyhow::bail!("encoder does not support dynamic bitrate changes")
    }
    fn set_quality(&mut self, _quality: u8) -> Result<()> {
        anyhow::bail!("encoder does not support dynamic quality changes")
    }
    fn dimensions(&self) -> (u32, u32);
    fn encode(&mut self, pixels: Arc<[u8]>) -> BoxFuture<'_, Result<EncodedFrame>>;
}

pub trait EncoderFactory: Send + Sync {
    fn create(&self, codec: VideoCodec, settings: EncoderSettings)
    -> Result<Box<dyn VideoEncoder>>;
    /// A real bounded test frame, rather than assuming a compiled backend works.
    fn probe(&self) -> BoxFuture<'_, Vec<CodecCapability>>;
}

pub struct SoftwareEncoders;

impl VideoEncoder for H264Encoder {
    fn set_rate(&mut self, rate: EncoderRate) -> Result<()> {
        H264Encoder::set_rate(self, rate)
    }
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn encode(&mut self, pixels: Arc<[u8]>) -> BoxFuture<'_, Result<EncodedFrame>> {
        Box::pin(self.encode_shared(pixels))
    }
}

struct JpegEncoder {
    settings: EncoderSettings,
}
impl VideoEncoder for JpegEncoder {
    fn set_quality(&mut self, quality: u8) -> Result<()> {
        anyhow::ensure!((1..=100).contains(&quality), "invalid JPEG quality");
        self.settings.quality = quality;
        Ok(())
    }
    fn dimensions(&self) -> (u32, u32) {
        (self.settings.width, self.settings.height)
    }
    fn encode(&mut self, pixels: Arc<[u8]>) -> BoxFuture<'_, Result<EncodedFrame>> {
        let settings = self.settings;
        Box::pin(async move {
            let data = tokio::task::spawn_blocking(move || {
                encode_jpeg(settings.width, settings.height, &pixels, settings.quality)
            })
            .await??;
            Ok(EncodedFrame {
                data: data.to_vec(),
                key: true,
                codec: Some("jpeg".into()),
            })
        })
    }
}

impl EncoderFactory for SoftwareEncoders {
    fn create(
        &self,
        codec: VideoCodec,
        settings: EncoderSettings,
    ) -> Result<Box<dyn VideoEncoder>> {
        match codec {
            VideoCodec::H264 => Ok(Box::new(H264Encoder::new(
                settings.width,
                settings.height,
                settings.fps,
            )?)),
            VideoCodec::Jpeg => {
                anyhow::ensure!(
                    (1..=16384).contains(&settings.width) && (1..=16384).contains(&settings.height),
                    "invalid JPEG dimensions"
                );
                anyhow::ensure!(
                    (1..=100).contains(&settings.quality),
                    "invalid JPEG quality"
                );
                Ok(Box::new(JpegEncoder { settings }))
            }
        }
    }
    fn probe(&self) -> BoxFuture<'_, Vec<CodecCapability>> {
        Box::pin(async move {
            let settings = EncoderSettings {
                width: 640,
                height: 360,
                fps: 30,
                quality: 75,
            };
            let pixels: Arc<[u8]> = vec![128; 640 * 360 * 4].into();
            let mut capabilities = Vec::new();
            for (codec, backend, dynamic_quality) in [
                (VideoCodec::H264, "openh264", false),
                (VideoCodec::Jpeg, "turbojpeg", true),
            ] {
                let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    let mut encoder = self.create(codec, settings)?;
                    let encoded = encoder.encode(pixels.clone()).await?;
                    anyhow::ensure!(
                        !encoded.data.is_empty() && encoded.key,
                        "codec test frame is not independently decodable"
                    );
                    let codec_string = encoded.codec.clone();
                    let dynamic_bitrate = if codec == VideoCodec::H264 {
                        if encoder
                            .set_rate(EncoderRate {
                                bitrate_bps: 300_000,
                                fps: 15,
                            })
                            .is_ok()
                        {
                            encoder.encode(pixels.clone()).await.is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    Ok::<_, anyhow::Error>((dynamic_bitrate, codec_string))
                })
                .await
                .context("codec probe timed out")
                .and_then(std::convert::identity);
                capabilities.push(CodecCapability {
                    codec,
                    backend,
                    hardware: false,
                    available: result.is_ok(),
                    dynamic_bitrate: result.as_ref().is_ok_and(|(supported, _)| *supported),
                    codec_string: result
                        .as_ref()
                        .ok()
                        .and_then(|(_, profile)| profile.clone()),
                    rate_update: if codec == VideoCodec::H264 {
                        "live"
                    } else {
                        "none"
                    },
                    reason: result.err().map(|e| format!("{e:#}")),
                    dynamic_quality,
                });
            }
            capabilities
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn factory_encodes_real_frames_for_both_software_backends() {
        let settings = EncoderSettings {
            width: 640,
            height: 360,
            fps: 30,
            quality: 75,
        };
        for codec in [VideoCodec::H264, VideoCodec::Jpeg] {
            let mut encoder = SoftwareEncoders.create(codec, settings).unwrap();
            assert_eq!(encoder.dimensions(), (640, 360));
            let encoded = encoder
                .encode(vec![128; 640 * 360 * 4].into())
                .await
                .unwrap();
            assert!(encoded.key);
            match codec {
                VideoCodec::Jpeg => {
                    let image = image::load_from_memory(&encoded.data).unwrap();
                    assert_eq!((image.width(), image.height()), (640, 360));
                }
                VideoCodec::H264 => assert!(encoded.codec.unwrap().starts_with("avc1.")),
            }
        }
    }
    #[tokio::test]
    async fn availability_is_backed_by_real_encoding_and_explicit_backend_metadata() {
        let capabilities = SoftwareEncoders.probe().await;
        assert_eq!(capabilities.len(), 2);
        assert!(
            capabilities
                .iter()
                .all(|c| c.available && !c.hardware && c.reason.is_none())
        );
        assert!(!capabilities[0].dynamic_quality);
        assert!(capabilities[0].dynamic_bitrate);
        assert!(!capabilities[1].dynamic_bitrate);
        assert!(capabilities[1].dynamic_quality);
    }
    #[tokio::test]
    async fn dynamic_quality_is_explicit_and_matches_backend_capability() {
        let settings = EncoderSettings {
            width: 640,
            height: 360,
            fps: 30,
            quality: 75,
        };
        let mut jpeg = SoftwareEncoders.create(VideoCodec::Jpeg, settings).unwrap();
        let pixels: Arc<[u8]> = (0..640 * 360 * 4)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>()
            .into();
        let first = jpeg.encode(pixels.clone()).await.unwrap();
        jpeg.set_quality(30).unwrap();
        let second = jpeg.encode(pixels).await.unwrap();
        assert_ne!(first.data, second.data);
        assert!(jpeg.set_quality(0).is_err());
        let mut h264 = SoftwareEncoders.create(VideoCodec::H264, settings).unwrap();
        assert!(h264.set_quality(30).is_err());
    }
    #[test]
    fn factory_rejects_unsupported_h264_dimensions_and_invalid_jpeg_quality() {
        let settings = EncoderSettings {
            width: 321,
            height: 200,
            fps: 30,
            quality: 0,
        };
        assert!(SoftwareEncoders.create(VideoCodec::H264, settings).is_err());
        assert!(SoftwareEncoders.create(VideoCodec::Jpeg, settings).is_err());
    }
}
