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
            encoder.set_subsamp(turbojpeg::Subsamp::Sub2x2)?;
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
