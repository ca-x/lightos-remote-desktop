//! Capability-gated X11 capture. Damage avoids readback; MIT-SHM avoids socket
//! pixel payloads. Either optimization can fail without disabling core GetImage.
use super::*;
use memmap2::{Mmap, MmapOptions};
use x11rb::protocol::{
    Event,
    damage::{ConnectionExt as _, DamageWrapper, ReportLevel},
    shm::{ConnectionExt as _, SegWrapper},
};

struct SharedImage<'a> {
    segment: SegWrapper<&'a XConnection>,
    mapping: Mmap,
    dimensions: (u16, u16),
}
impl<'a> SharedImage<'a> {
    fn new(
        connection: &'a XConnection,
        dimensions: (u16, u16),
        format: xproto::Format,
    ) -> Result<Self> {
        let length = image_length(dimensions, format)?;
        let id = connection.generate_id()?;
        let reply = connection
            .shm_create_segment(id, u32::try_from(length)?, false)?
            .reply()?;
        let segment = SegWrapper::for_seg(connection, id);
        // SAFETY: this private server-created segment is owned by this capture
        // connection. The server writes only for our SHM GetImage requests;
        // each reply is received before the mapped bytes are read or reused.
        let mapping = unsafe { MmapOptions::new().len(length).map(&reply.shm_fd)? };
        Ok(Self {
            segment,
            mapping,
            dimensions,
        })
    }
    fn capture(
        &self,
        connection: &XConnection,
        root: u32,
        rect: Rect,
        format: xproto::Format,
        visual: &xproto::Visualtype,
    ) -> Result<Arc<[u8]>> {
        let reply = connection
            .shm_get_image(
                root,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                u32::MAX,
                u8::from(ImageFormat::Z_PIXMAP),
                self.segment.seg(),
                0,
            )?
            .reply()?;
        ensure!(
            reply.depth == format.depth && reply.visual == visual.visual_id,
            "X11 shared image format changed"
        );
        ensure!(
            reply.size as usize <= self.mapping.len()
                && reply.size as usize + 3 >= self.mapping.len(),
            "invalid X11 shared image size"
        );
        image_rgba_data(
            &self.mapping,
            rect.width,
            rect.height,
            connection.setup().image_byte_order,
            format,
            visual,
        )
    }
}
fn image_length((width, height): (u16, u16), format: xproto::Format) -> Result<usize> {
    let pad = usize::from(format.scanline_pad);
    ensure!(
        width > 0 && height > 0 && pad >= 8 && pad.is_multiple_of(8),
        "invalid X11 shared image geometry"
    );
    let bits = usize::from(width)
        .checked_mul(usize::from(format.bits_per_pixel))
        .context("X11 shared row overflow")?;
    let row = bits
        .div_ceil(pad)
        .checked_mul(pad / 8)
        .context("X11 shared row overflow")?;
    let length = row
        .checked_mul(usize::from(height))
        .and_then(|size| size.checked_add(3))
        .context("X11 shared image overflow")?
        & !3;
    ensure!(
        length > 0 && length <= MAX_IMAGE_BYTES,
        "X11 shared image exceeds capture limit"
    );
    Ok(length)
}

pub(super) struct Capture<'a> {
    connection: &'a XConnection,
    target: &'a XTarget,
    output: Option<&'a str>,
    damage: Option<DamageWrapper<&'a XConnection>>,
    shm_available: bool,
    shared: Option<SharedImage<'a>>,
    previous: Option<(Rect, Arc<[u8]>)>,
}
impl<'a> Capture<'a> {
    pub(super) fn new(
        connection: &'a XConnection,
        target: &'a XTarget,
        output: Option<&'a str>,
    ) -> Self {
        let root = connection.setup().roots[target.screen].root;
        let damage = (|| -> Result<_> {
            connection.damage_query_version(1, 1)?.reply()?;
            let id = connection.generate_id()?;
            connection
                .damage_create(id, root, ReportLevel::NON_EMPTY)?
                .check()?;
            Ok(DamageWrapper::for_damage(connection, id))
        })()
        .map_err(
            |error| tracing::debug!(%error, "XDamage unavailable; polling capture remains enabled"),
        )
        .ok();
        let shm_available = connection
            .shm_query_version()
            .ok()
            .and_then(|reply| reply.reply().ok())
            .is_some_and(|reply| (reply.major_version, reply.minor_version) >= (1, 2));
        Self {
            connection,
            target,
            output,
            damage,
            shm_available,
            shared: None,
            previous: None,
        }
    }
    #[cfg(test)]
    pub(super) fn modes(&self) -> (bool, bool) {
        (self.damage.is_some(), self.shared.is_some())
    }
    pub(super) fn next(&mut self) -> Result<(Rect, Arc<[u8]>, bool)> {
        let root_rect = root_rect(self.connection, self.target.screen)?;
        let rect = match self.output {
            Some(name) => output_rect(self.connection, self.target.screen, name, root_rect)?,
            None => root_rect,
        };
        let mut dirty = self.damage.is_none()
            || self
                .previous
                .as_ref()
                .is_none_or(|(previous, _)| *previous != rect);
        while let Some(event) = self.connection.poll_for_event()? {
            if matches!(event, Event::DamageNotify(_)) {
                dirty = true;
            }
        }
        if !dirty && let Some((rect, pixels)) = &self.previous {
            return Ok((*rect, pixels.clone(), false));
        }
        if let Some(damage) = &self.damage
            && let Err(error) = (|| -> Result<()> {
                self.connection
                    .damage_subtract(damage.damage(), x11rb::NONE, x11rb::NONE)?
                    .check()?;
                Ok(())
            })()
        {
            tracing::debug!(%error, "XDamage invalidation failed; reverting to polling");
            self.damage.take();
        }
        let (visual, format) = visual_and_format(self.connection, self.target.screen)?;
        if self.shm_available
            && self
                .shared
                .as_ref()
                .is_none_or(|image| image.dimensions != (rect.width, rect.height))
        {
            self.shared.take();
            match SharedImage::new(self.connection, (rect.width, rect.height), format) {
                Ok(image) => self.shared = Some(image),
                Err(error) => {
                    tracing::debug!(%error, "MIT-SHM setup failed; reverting to GetImage");
                    self.shm_available = false;
                }
            }
        }
        let pixels = self.shared.as_ref().map(|image| {
            image.capture(
                self.connection,
                self.connection.setup().roots[self.target.screen].root,
                rect,
                format,
                &visual,
            )
        });
        let (rect, pixels) = match pixels {
            Some(Ok(pixels)) => (rect, pixels),
            result => {
                if let Some(Err(error)) = result {
                    tracing::debug!(%error, "MIT-SHM readback failed; reverting to GetImage");
                    self.shared.take();
                    self.shm_available = false;
                }
                capture_image(self.connection, self.target, self.output)?
            }
        };
        self.previous = Some((rect, pixels.clone()));
        Ok((rect, pixels, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_image_size_accounts_for_padding_and_rejects_bad_geometry() {
        let format = xproto::Format {
            depth: 24,
            bits_per_pixel: 24,
            scanline_pad: 32,
        };
        assert_eq!(image_length((1, 3), format).unwrap(), 12);
        assert_eq!(image_length((3, 2), format).unwrap(), 24);
        assert!(image_length((0, 10), format).is_err());
        assert!(image_length((u16::MAX, u16::MAX), format).is_err());
        assert!(
            image_length(
                (1, 1),
                xproto::Format {
                    scanline_pad: 0,
                    ..format
                }
            )
            .is_err()
        );
    }
}
