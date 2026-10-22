use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use openh264::{
    OpenH264API, Timestamp,
    encoder::{
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile,
        RateControlMode, UsageType,
    },
    formats::{RgbaSliceU8, YUVBuffer},
};
use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};

pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub key: bool,
    pub codec: Option<String>,
}

struct EncodeRequest {
    pixels: Arc<[u8]>,
    reply: oneshot::Sender<Result<EncodedFrame>>,
}

pub struct H264Encoder {
    pub width: u32,
    pub height: u32,
    requests: mpsc::Sender<EncodeRequest>,
    usable: bool,
}

impl H264Encoder {
    pub fn new(width: u32, height: u32, fps: u32) -> Result<Self> {
        ensure!(
            (320..=3840).contains(&width) && (200..=2160).contains(&height),
            "invalid encoder dimensions"
        );
        ensure!(
            width.is_multiple_of(2) && height.is_multiple_of(2),
            "H.264 requires even dimensions; using JPEG"
        );
        ensure!((1..=120).contains(&fps), "invalid encoder frame rate");
        let (requests, mut receiver) = mpsc::channel::<EncodeRequest>(1);
        std::thread::Builder::new()
            .name("h264-encoder".into())
            .spawn(move || {
                let result = native_encoder(width, height, fps);
                let mut encoder = match result {
                    Ok(encoder) => encoder,
                    Err(error) => {
                        if let Some(request) = receiver.blocking_recv() {
                            let _ = request.reply.send(Err(error));
                        }
                        return;
                    }
                };
                let mut yuv = YUVBuffer::new(width as usize, height as usize);
                let started = Instant::now();
                while let Some(request) = receiver.blocking_recv() {
                    if request.reply.is_closed() {
                        continue;
                    }
                    yuv.read_rgba8(RgbaSliceU8::new(
                        &request.pixels,
                        (width as usize, height as usize),
                    ));
                    let result = encoder
                        .encode_at(
                            &yuv,
                            Timestamp::from_millis(started.elapsed().as_millis() as u64),
                        )
                        .context("native H.264 encode")
                        .and_then(|stream| {
                            let data = stream.to_vec();
                            ensure!(!data.is_empty(), "native encoder skipped a frame");
                            let (key, codec) = annex_b_info(&data);
                            Ok(EncodedFrame { data, key, codec })
                        });
                    let failed = result.is_err();
                    let abandoned = request.reply.send(result).is_err();
                    if failed || abandoned {
                        break;
                    }
                }
            })
            .context("start native H.264 worker")?;
        Ok(Self {
            width,
            height,
            requests,
            usable: true,
        })
    }

    pub async fn encode(&mut self, pixels: &[u8]) -> Result<EncodedFrame> {
        ensure!(
            pixels.len() == self.width as usize * self.height as usize * 4,
            "invalid RGBA frame length"
        );
        self.encode_shared(Arc::from(pixels)).await
    }

    pub async fn encode_shared(&mut self, pixels: Arc<[u8]>) -> Result<EncodedFrame> {
        ensure!(
            self.usable,
            "native encoder cannot resume after canceled or failed output"
        );
        ensure!(
            pixels.len() == self.width as usize * self.height as usize * 4,
            "invalid RGBA frame length"
        );
        let (reply, received) = oneshot::channel();
        self.usable = false;
        let result = timeout(Duration::from_secs(5), async {
            self.requests
                .send(EncodeRequest { pixels, reply })
                .await
                .map_err(|_| anyhow::anyhow!("native H.264 worker stopped"))?;
            received.await.context("native H.264 worker stopped")?
        })
        .await
        .context("native H.264 encode timed out")?;
        self.usable = result.is_ok();
        result
    }
}

fn native_encoder(width: u32, height: u32, fps: u32) -> Result<Encoder> {
    let bitrate = (u64::from(width) * u64::from(height) * 3).clamp(600_000, 18_000_000);
    let config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .profile(Profile::Baseline)
        .complexity(Complexity::Low)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps(bitrate as u32))
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .skip_frames(false)
        .scene_change_detect(true)
        .adaptive_quantization(false)
        .background_detection(false)
        .intra_frame_period(IntraFramePeriod::from_num_frames(120))
        .num_threads(2);
    Encoder::with_api_config(OpenH264API::from_source(), config)
        .context("initialize native OpenH264 encoder")
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

    #[tokio::test]
    async fn rejects_invalid_dimensions_and_pixels() {
        assert!(H264Encoder::new(321, 200, 30).is_err());
        assert!(H264Encoder::new(3842, 2160, 30).is_err());
        assert!(H264Encoder::new(640, 360, 0).is_err());
        let mut encoder = H264Encoder::new(640, 360, 30).unwrap();
        assert!(encoder.encode(&[0; 4]).await.is_err());
    }

    #[tokio::test]
    async fn recreated_encoder_includes_parameter_sets_and_idr() {
        for (width, height) in [(640, 360), (800, 600), (640, 360)] {
            let mut encoder = H264Encoder::new(width, height, 30).unwrap();
            let frame = encoder
                .encode(&vec![127; width as usize * height as usize * 4])
                .await
                .unwrap();
            assert!(frame.key && frame.codec.is_some());
        }
    }

    #[tokio::test]
    async fn native_stream_decodes_every_access_unit_across_keyframe_period() {
        use openh264::{decoder::Decoder, formats::YUVSource};
        let mut decoder = Decoder::new().unwrap();
        let mut encoder = H264Encoder::new(640, 360, 30).unwrap();
        let mut pixels = vec![255; 640 * 360 * 4];
        for index in 0..130 {
            for row in 30..80 {
                for column in 30..120 {
                    let offset = (row * 640 + column) * 4;
                    pixels[offset] = (index * 7) as u8;
                    pixels[offset + 1] = (index * 13) as u8;
                }
            }
            let frame = encoder.encode(&pixels).await.unwrap();
            let decoded = decoder
                .decode(&frame.data)
                .unwrap()
                .expect("one output per frame");
            assert_eq!(decoded.dimensions(), (640, 360));
        }
    }

    #[tokio::test]
    async fn canceled_output_cannot_be_followed_by_a_dependent_frame() {
        let mut encoder = H264Encoder::new(640, 360, 30).unwrap();
        let pixels = vec![127; 640 * 360 * 4];
        let mut encoding = Box::pin(encoder.encode(&pixels));
        assert!(futures_util::poll!(&mut encoding).is_pending());
        drop(encoding);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(encoder.encode(&pixels).await.is_err());
    }

    #[tokio::test]
    async fn shared_encode_passes_the_capture_allocation_without_copying_pixels() {
        let (requests, mut receiver) = mpsc::channel(1);
        let mut encoder = H264Encoder {
            width: 640,
            height: 360,
            requests,
            usable: true,
        };
        let pixels: Arc<[u8]> = vec![255; 640 * 360 * 4].into();
        let mut encoding = Box::pin(encoder.encode_shared(pixels.clone()));
        assert!(futures_util::poll!(&mut encoding).is_pending());
        let request = receiver.recv().await.unwrap();
        assert!(Arc::ptr_eq(&pixels, &request.pixels));
        assert!(
            request
                .reply
                .send(Ok(EncodedFrame {
                    data: vec![1],
                    key: true,
                    codec: None
                }))
                .is_ok()
        );
        assert_eq!(encoding.await.unwrap().data, [1]);
    }
}
