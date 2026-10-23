use std::{
    io::Cursor,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

const MAX_IMAGE: usize = 8 * 1024 * 1024;
const MAX_CHUNK: usize = 48 * 1024;

/// One bounded, ordered image upload belongs to one authenticated controller.
pub(crate) struct ImageUpload {
    id: u64,
    size: usize,
    next: u64,
    bytes: Vec<u8>,
    updated: Instant,
}

impl ImageUpload {
    pub(crate) fn start(id: u64, size: u64) -> Result<Self> {
        ensure!(
            (1..=9_007_199_254_740_991).contains(&id),
            "invalid clipboard transfer ID"
        );
        ensure!(
            (1..=MAX_IMAGE as u64).contains(&size),
            "clipboard image must be at most 8 MiB"
        );
        Ok(Self {
            id,
            size: size as usize,
            next: 1,
            bytes: Vec::new(),
            updated: Instant::now(),
        })
    }

    fn check(&self, id: u64, step: u64) -> Result<()> {
        ensure!(
            id == self.id && step == self.next,
            "clipboard transfer is out of order"
        );
        ensure!(
            self.updated.elapsed() <= Duration::from_secs(15),
            "clipboard transfer expired"
        );
        Ok(())
    }

    pub(crate) fn append(&mut self, id: u64, step: u64, data: &str) -> Result<()> {
        self.check(id, step)?;
        ensure!(
            !data.is_empty() && data.len() <= MAX_CHUNK * 4 / 3,
            "invalid clipboard chunk size"
        );
        let bytes = STANDARD.decode(data)?;
        ensure!(
            self.bytes.len() + bytes.len() <= self.size,
            "clipboard image exceeds declared size"
        );
        self.bytes.extend_from_slice(&bytes);
        self.next += 1;
        self.updated = Instant::now();
        Ok(())
    }

    pub(crate) fn finish(self, id: u64, step: u64) -> Result<Vec<u8>> {
        self.check(id, step)?;
        ensure!(
            self.bytes.len() == self.size,
            "clipboard image is incomplete"
        );
        ensure!(
            self.bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "clipboard image must be PNG"
        );
        let reader =
            image::ImageReader::with_format(Cursor::new(&self.bytes), image::ImageFormat::Png);
        let (width, height) = reader.into_dimensions()?;
        ensure!(
            width > 0
                && height > 0
                && width <= 8192
                && height <= 8192
                && u64::from(width) * u64::from(height) <= 33_554_432,
            "clipboard image dimensions are too large"
        );
        Ok(self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn image_upload_preserves_png_and_requires_order_and_complete_size() {
        let bytes = png();
        let mut upload = ImageUpload::start(1, bytes.len() as u64).unwrap();
        assert!(upload.append(2, 1, &STANDARD.encode(&bytes)).is_err());
        assert!(upload.append(1, 2, &STANDARD.encode(&bytes)).is_err());
        for (i, part) in bytes.chunks(30).enumerate() {
            upload
                .append(1, i as u64 + 1, &STANDARD.encode(part))
                .unwrap();
        }
        let end = upload.next;
        assert_eq!(upload.finish(1, end).unwrap(), bytes);
        assert!(ImageUpload::start(1, 100).unwrap().finish(1, 1).is_err());
    }

    #[test]
    fn invalid_expired_or_oversized_images_are_rejected() {
        assert!(ImageUpload::start(1, MAX_IMAGE as u64 + 1).is_err());
        assert!(ImageUpload::start(0, 1).is_err());
        let mut upload = ImageUpload::start(1, 1).unwrap();
        assert!(upload.append(1, 1, "%%%=").is_err());
        assert!(upload.append(1, 1, &STANDARD.encode([1, 2])).is_err());
        assert!(
            upload
                .append(1, 1, &"A".repeat(MAX_CHUNK * 4 / 3 + 4))
                .is_err()
        );
        upload.updated = Instant::now() - Duration::from_secs(16);
        assert!(upload.append(1, 1, "AA==").is_err());
        let mut upload = ImageUpload::start(1, 1).unwrap();
        upload.append(1, 1, "AA==").unwrap();
        assert!(upload.finish(1, 2).is_err());
    }

    #[test]
    fn chords_are_bounded_and_cannot_leave_duplicate_key_state() {
        use crate::wayland::InputCommand;
        assert!(
            InputCommand::Chord {
                codes: vec![29, 47]
            }
            .validate()
            .is_ok()
        );
        for codes in [vec![], vec![29, 29], vec![0], vec![125; 9], vec![u32::MAX]] {
            assert!(InputCommand::Chord { codes }.validate().is_err());
        }
    }
}
