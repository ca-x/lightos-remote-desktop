use anyhow::{Context, Result, ensure};
use std::{cell::RefCell, sync::Arc};

thread_local! {
    static JPEG_ENCODER: RefCell<Option<turbojpeg::Compressor>> = const { RefCell::new(None) };
}

pub fn encode_jpeg(width: u32, height: u32, pixels: &[u8], quality: u8) -> Result<Arc<[u8]>> {
    ensure!(
        width > 0 && height > 0 && width <= 16384 && height <= 16384,
        "invalid JPEG dimensions"
    );
    ensure!((1..=100).contains(&quality), "invalid JPEG quality");
    ensure!(
        pixels.len() == width as usize * height as usize * 4,
        "invalid JPEG pixels"
    );
    JPEG_ENCODER.with(|slot| -> Result<_> {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let mut encoder =
                turbojpeg::Compressor::new().context("initialize SIMD JPEG encoder")?;
            encoder.set_subsamp(turbojpeg::Subsamp::None)?;
            *slot = Some(encoder);
        }
        let encoder = slot.as_mut().unwrap();
        encoder.set_quality(i32::from(quality))?;
        Ok(encoder
            .compress_to_vec(turbojpeg::Image {
                pixels,
                width: width as usize,
                pitch: width as usize * 4,
                height: height as usize,
                format: turbojpeg::PixelFormat::RGBA,
            })?
            .into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_preserves_pixel_wide_colored_desktop_details() {
        let (width, height) = (64, 32);
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                }
            })
            .collect();
        let current = encode_jpeg(width, height, &pixels, 75).unwrap();
        let mut legacy = turbojpeg::Compressor::new().unwrap();
        legacy.set_subsamp(turbojpeg::Subsamp::Sub2x2).unwrap();
        legacy.set_quality(75).unwrap();
        let previous = legacy
            .compress_to_vec(turbojpeg::Image {
                pixels: &pixels,
                width: width as usize,
                height: height as usize,
                pitch: width as usize * 4,
                format: turbojpeg::PixelFormat::RGBA,
            })
            .unwrap();
        let error = |bytes: &[u8]| {
            let decoded = image::load_from_memory(bytes).unwrap().to_rgb8();
            decoded
                .as_raw()
                .as_chunks::<3>()
                .0
                .iter()
                .zip(pixels.as_chunks::<4>().0.iter())
                .map(|(actual, expected)| {
                    actual
                        .iter()
                        .zip(expected)
                        .map(|(a, b)| a.abs_diff(*b) as f64)
                        .sum::<f64>()
                })
                .sum::<f64>()
                / (width * height * 3) as f64
        };
        let before = error(&previous);
        let after = error(&current);
        println!(
            "Colored one-pixel edges at JPEG quality 75: 4:2:0 MAE={before:.2}, 4:4:4 MAE={after:.2}; bytes {} -> {}",
            previous.len(),
            current.len()
        );
        assert!(
            after < 15.0 && after < before / 4.0,
            "fine color detail lost: {before} -> {after}"
        );
    }
}
