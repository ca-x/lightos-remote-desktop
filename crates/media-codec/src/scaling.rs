//! Encoding resolution is independent of the captured desktop and its input space.
use std::sync::Arc;

use anyhow::{Result, ensure};
use fast_image_resize::{
    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
    images::{Image, ImageRef},
};

/// Fit within a requested pixel box without upscaling. Preserve the encoder's
/// minimum size and round down to even pixels. Tiny captures stay native.
pub fn fit_dimensions(source: (u32, u32), bounds: Option<(u32, u32)>) -> (u32, u32) {
    let (width, height) = source;
    let Some((max_width, max_height)) = bounds else {
        return source;
    };
    if width < 320 || height < 200 || max_width == 0 || max_height == 0 {
        return source;
    }
    let ratio = (f64::from(max_width) / f64::from(width))
        .min(f64::from(max_height) / f64::from(height))
        .max(320.0 / f64::from(width))
        .max(200.0 / f64::from(height))
        .min(1.0);
    if ratio >= 1.0 {
        return source;
    }
    (
        (((f64::from(width) * ratio).floor() as u32 / 2) * 2).max(320),
        (((f64::from(height) * ratio).floor() as u32 / 2) * 2).max(200),
    )
}

#[derive(Default)]
pub struct RgbaScaler {
    resizer: Resizer,
    output: Arc<[u8]>,
}

impl RgbaScaler {
    /// Run on a blocking worker. The caller may retain the returned Arc; the
    /// next resize uses copy-on-write only if that output is still in flight.
    pub fn resize(
        &mut self,
        pixels: Arc<[u8]>,
        source: (u32, u32),
        target: (u32, u32),
    ) -> Result<Arc<[u8]>> {
        let bytes = |(width, height): (u32, u32)| -> Result<usize> {
            ensure!(
                width > 0
                    && height > 0
                    && width <= 8192
                    && height <= 8192
                    && u64::from(width) * u64::from(height) <= 33_554_432,
                "invalid scale dimensions"
            );
            Ok(width as usize * height as usize * 4)
        };
        ensure!(pixels.len() == bytes(source)?, "invalid RGBA buffer length");
        let output_len = bytes(target)?;
        ensure!(
            target.0 <= source.0 && target.1 <= source.1,
            "encoding scale must not upscale"
        );
        if source == target {
            return Ok(pixels);
        }
        if self.output.len() != output_len {
            self.output = vec![0; output_len].into();
        }
        let input = ImageRef::new(source.0, source.1, &pixels, PixelType::U8x4)?;
        let mut output = Image::from_slice_u8(
            target.0,
            target.1,
            Arc::make_mut(&mut self.output),
            PixelType::U8x4,
        )?;
        // Desktop captures are opaque RGB/RGBX. Alpha is not part of the video.
        self.resizer.resize(
            &input,
            &mut output,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
                .use_alpha(false),
        )?;
        Ok(self.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_preserves_native_or_proportional_encoder_sizes() {
        assert_eq!(
            fit_dimensions((3840, 2160), Some((1920, 1080))),
            (1920, 1080)
        );
        assert_eq!(fit_dimensions((1920, 1080), Some((1280, 800))), (1280, 720));
        assert_eq!(
            fit_dimensions((1920, 1080), Some((4000, 3000))),
            (1920, 1080)
        );
        assert_eq!(fit_dimensions((1920, 1080), None), (1920, 1080));
        for bounds in [(1, 1), (321, 201), (700, 1300)] {
            let (width, height) = fit_dimensions((3840, 2160), Some(bounds));
            assert!(
                width >= 320
                    && height >= 200
                    && width.is_multiple_of(2)
                    && height.is_multiple_of(2)
            );
            assert!((f64::from(width) / f64::from(height) - 3840.0 / 2160.0).abs() < 0.02);
        }
        assert_eq!(fit_dimensions((100, 50), Some((1, 1))), (100, 50));
    }

    #[test]
    fn native_passes_through_and_scaled_buffer_is_reused_safely() {
        let mut scaler = RgbaScaler::default();
        let pixels: Arc<[u8]> = vec![125; 640 * 360 * 4].into();
        let native = scaler
            .resize(pixels.clone(), (640, 360), (640, 360))
            .unwrap();
        assert!(Arc::ptr_eq(&pixels, &native));
        let first = scaler
            .resize(pixels.clone(), (640, 360), (320, 200))
            .unwrap();
        assert_eq!(first.len(), 320 * 200 * 4);
        let address = first.as_ptr();
        drop(first);
        let second = scaler
            .resize(pixels.clone(), (640, 360), (320, 200))
            .unwrap();
        assert_eq!(address, second.as_ptr());
        let black: Arc<[u8]> = vec![0; pixels.len()].into();
        let third = scaler.resize(black, (640, 360), (320, 200)).unwrap();
        assert!(
            second.iter().all(|&v| v == 125),
            "retained frame must remain immutable"
        );
        assert!(third.iter().all(|&v| v == 0));
        assert!(
            scaler
                .resize(pixels.clone(), (640, 360), (641, 360))
                .is_err()
        );
        assert!(scaler.resize(pixels.clone(), (640, 360), (0, 200)).is_err());
        assert!(scaler.resize(pixels, (641, 360), (320, 200)).is_err());
    }

    #[tokio::test]
    async fn scaled_frames_really_encode_and_decode() {
        use crate::{H264Encoder, encode_jpeg};
        use openh264::formats::YUVSource;
        let mut scaler = RgbaScaler::default();
        let pixels: Arc<[u8]> = (0..640 * 400 * 4)
            .map(|i| (i * 31 % 256) as u8)
            .collect::<Vec<_>>()
            .into();
        let target = fit_dimensions((640, 400), Some((320, 200)));
        let scaled = scaler.resize(pixels, (640, 400), target).unwrap();
        let jpeg = encode_jpeg(target.0, target.1, &scaled, 75).unwrap();
        let image = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((image.width(), image.height()), target);
        let mut encoder = H264Encoder::new(target.0, target.1, 30).unwrap();
        let h264 = encoder.encode_shared(scaled).await.unwrap();
        let mut decoder = openh264::decoder::Decoder::new().unwrap();
        let frame = decoder.decode(&h264.data).unwrap().unwrap();
        assert_eq!(frame.dimensions(), (target.0 as usize, target.1 as usize));
    }
}
