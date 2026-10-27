//! Bounded SPA cursor metadata. No pointer pixels are added to video frames.
use super::*;

const HEADER: usize = 28;
const BITMAP: usize = 20;
const CAPACITY: usize = HEADER + BITMAP + 512 * 512 * 4;

pub(super) fn parameters() -> Result<Vec<Vec<u8>>> {
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamMeta.as_raw(),
        id: spa::param::ParamType::Meta.as_raw(),
        properties: vec![
            spa::pod::Property::new(
                spa::sys::SPA_PARAM_META_type,
                spa::pod::Value::Id(spa::utils::Id(spa::sys::SPA_META_Cursor)),
            ),
            spa::pod::Property::new(
                spa::sys::SPA_PARAM_META_size,
                spa::pod::Value::Choice(spa::pod::ChoiceValue::Int(spa::utils::Choice(
                    spa::utils::ChoiceFlags::empty(),
                    spa::utils::ChoiceEnum::Range {
                        default: (HEADER + BITMAP + 256 * 256 * 4) as i32,
                        min: (HEADER + BITMAP) as i32,
                        max: CAPACITY as i32,
                    },
                ))),
            ),
        ],
    };
    let memory_types = (1_i32 << spa::sys::SPA_DATA_MemFd) | (1_i32 << spa::sys::SPA_DATA_MemPtr);
    let buffers = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamBuffers.as_raw(),
        id: spa::param::ParamType::Buffers.as_raw(),
        properties: vec![spa::pod::Property::new(
            spa::sys::SPA_PARAM_BUFFERS_dataType,
            spa::pod::Value::Choice(spa::pod::ChoiceValue::Int(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Flags {
                    default: memory_types,
                    flags: vec![memory_types],
                },
            ))),
        )],
    };
    [object, buffers]
        .into_iter()
        .map(|value| {
            Ok(spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &spa::pod::Value::Object(value),
            )?
            .0
            .into_inner())
        })
        .collect()
}

fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_ne_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated cursor metadata")?
            .try_into()?,
    ))
}

pub(super) fn decode(
    bytes: &[u8],
    size: spa::utils::Rectangle,
    previous: Option<&CursorFrame>,
) -> Result<Option<CursorFrame>> {
    ensure!(
        bytes.len() >= HEADER && size.width > 0 && size.height > 0,
        "invalid cursor metadata"
    );
    // SPA id zero means no update, not an invisible pointer.
    if word(bytes, 0)? == 0 {
        return Ok(None);
    }
    let x = word(bytes, 8)? as i32;
    let y = word(bytes, 12)? as i32;
    let mut cursor = previous.cloned().unwrap_or(CursorFrame {
        serial: 1,
        visible: false,
        x: 0.,
        y: 0.,
        width: 0,
        height: 0,
        hotspot_x: 0,
        hotspot_y: 0,
        units_width: size.width,
        units_height: size.height,
        pixels: Arc::from([]),
    });
    cursor.x = f64::from(x) / f64::from(size.width);
    cursor.y = f64::from(y) / f64::from(size.height);
    cursor.units_width = size.width;
    cursor.units_height = size.height;
    let bitmap = word(bytes, 24)? as usize;
    if bitmap != 0 {
        ensure!(
            bitmap >= HEADER
                && bitmap
                    .checked_add(BITMAP)
                    .is_some_and(|end| end <= bytes.len()),
            "invalid cursor bitmap offset"
        );
        let format = word(bytes, bitmap)?;
        if format != 0 {
            let offset = word(bytes, bitmap + 16)? as usize;
            let (width, height, pixels) = if offset == 0 {
                (0, 0, Arc::<[u8]>::from([]))
            } else {
                let width = word(bytes, bitmap + 4)?;
                let height = word(bytes, bitmap + 8)?;
                let stride = word(bytes, bitmap + 12)? as i32;
                ensure!(
                    (1..=512).contains(&width)
                        && (1..=512).contains(&height)
                        && stride >= width as i32 * 4
                        && offset >= BITMAP,
                    "invalid cursor bitmap dimensions"
                );
                let start = bitmap
                    .checked_add(offset)
                    .context("cursor bitmap overflow")?;
                let length = (height as usize - 1)
                    .checked_mul(stride as usize)
                    .and_then(|v| v.checked_add(width as usize * 4))
                    .context("cursor bitmap overflow")?;
                let data = bytes
                    .get(
                        start
                            ..start
                                .checked_add(length)
                                .context("cursor bitmap overflow")?,
                    )
                    .context("truncated cursor bitmap")?;
                let rgba = format == VideoFormat::RGBA.as_raw();
                ensure!(
                    rgba || format == VideoFormat::BGRA.as_raw(),
                    "unsupported cursor bitmap format"
                );
                let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
                for row in data.chunks(stride as usize) {
                    for pixel in row[..width as usize * 4].as_chunks::<4>().0 {
                        if rgba {
                            pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                        } else {
                            pixels.extend_from_slice(pixel);
                        }
                    }
                }
                (width, height, pixels.into())
            };
            let hotspot_x = (word(bytes, 16)? as i32).max(0) as u32;
            let hotspot_y = (word(bytes, 20)? as i32).max(0) as u32;
            ensure!(
                width == 0 || (hotspot_x < width && hotspot_y < height),
                "invalid cursor hotspot"
            );
            if cursor.width != width
                || cursor.height != height
                || cursor.pixels != pixels
                || cursor.hotspot_x != hotspot_x
                || cursor.hotspot_y != hotspot_y
            {
                cursor.serial = cursor.serial.wrapping_add(1).max(1);
                cursor.width = width;
                cursor.height = height;
                cursor.pixels = pixels;
                cursor.hotspot_x = hotspot_x;
                cursor.hotspot_y = hotspot_y;
            }
        }
    }
    cursor.visible = x >= 0
        && y >= 0
        && x < size.width as i32
        && y < size.height as i32
        && cursor
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 0);
    Ok(Some(cursor))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> Vec<u8> {
        let mut value = vec![0; HEADER + BITMAP + 8];
        for (offset, word) in [
            (0, 1),
            (8, 10),
            (12, 20),
            (16, 1),
            (24, HEADER as u32),
            (28, VideoFormat::BGRA.as_raw()),
            (32, 2),
            (36, 1),
            (40, 8),
            (44, BITMAP as u32),
        ] {
            value[offset..offset + 4].copy_from_slice(&word.to_ne_bytes());
        }
        value[48..].copy_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255]);
        value
    }
    #[test]
    fn cursor_shape_position_visibility_and_invalid_offsets() -> Result<()> {
        let size = spa::utils::Rectangle {
            width: 100,
            height: 100,
        };
        let mut bytes = metadata();
        let first = decode(&bytes, size, None)?.unwrap();
        assert!(first.visible);
        assert_eq!(first.hotspot_x, 1);
        assert_eq!(first.x, 0.1);
        bytes[24..28].fill(0);
        bytes[8..12].copy_from_slice(&30_u32.to_ne_bytes());
        let moved = decode(&bytes, size, Some(&first))?.unwrap();
        assert_eq!(moved.serial, first.serial);
        assert!(Arc::ptr_eq(&moved.pixels, &first.pixels));
        assert_eq!(moved.x, 0.3);
        bytes[0..4].fill(0);
        assert!(decode(&bytes, size, Some(&moved))?.is_none());
        bytes = metadata();
        bytes[44..48].fill(0);
        assert!(!decode(&bytes, size, Some(&moved))?.unwrap().visible);
        bytes = metadata();
        bytes[24..28].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(decode(&bytes, size, None).is_err());
        bytes = metadata();
        bytes[44..48].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(decode(&bytes, size, None).is_err());
        Ok(())
    }
}
