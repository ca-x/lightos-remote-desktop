//! Cursor pixels come from the same authenticated X connection as the capture.
use super::*;
use crate::backend::CursorFrame;
use x11rb::protocol::xfixes::ConnectionExt as _;

pub(super) fn supported(connection: &XConnection) -> bool {
    connection
        .xfixes_query_version(5, 0)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some()
}

pub(super) fn read(
    connection: &XConnection,
    rect: Rect,
    previous: Option<&CursorFrame>,
) -> Result<CursorFrame> {
    let image = connection.xfixes_get_cursor_image()?.reply()?;
    let width = u32::from(image.width);
    let height = u32::from(image.height);
    ensure!(
        width <= 512
            && height <= 512
            && image.cursor_image.len() == width as usize * height as usize,
        "invalid X11 cursor image"
    );
    // Zero is the wire protocol's "send the image" sentinel.
    let serial = u64::from(image.cursor_serial) + 1;
    let pixels = match previous {
        Some(old) if old.serial == serial && old.width == width && old.height == height => {
            old.pixels.clone()
        }
        _ => image
            .cursor_image
            .iter()
            .flat_map(|pixel| pixel.to_le_bytes())
            .collect::<Vec<_>>()
            .into(),
    };
    let visible = pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0);
    Ok(CursorFrame {
        serial,
        visible,
        x: (f64::from(image.x) - f64::from(rect.x)) / f64::from(rect.width),
        y: (f64::from(image.y) - f64::from(rect.y)) / f64::from(rect.height),
        width,
        height,
        hotspot_x: u32::from(image.xhot),
        hotspot_y: u32::from(image.yhot),
        units_width: u32::from(rect.width),
        units_height: u32::from(rect.height),
        pixels,
    })
}
