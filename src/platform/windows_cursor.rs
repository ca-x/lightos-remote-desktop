//! Read the interactive desktop's cursor independently of Windows Graphics Capture.

use std::{
    cell::RefCell,
    hash::{DefaultHasher, Hash, Hasher},
    mem::size_of,
    ptr::null_mut,
    slice,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use display_control::DisplayOutput;
use windows::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetObjectW, HBITMAP, HDC, HGDIOBJ,
        SelectObject,
    },
    UI::WindowsAndMessaging::{
        CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, HICON,
        ICONINFO,
    },
};

use crate::backend::CursorFrame;

struct Shape {
    icon: HICON,
    sampled: Instant,
    serial: u64,
    width: u32,
    height: u32,
    hotspot_x: u32,
    hotspot_y: u32,
    pixels: Arc<[u8]>,
}

thread_local! {
    // A native cursor worker owns this cache for its lifetime. A periodic
    // refresh also observes replaced images whose handle remains the same.
    static SHAPE: RefCell<Option<Shape>> = const { RefCell::new(None) };
}

pub(super) fn capture(output: &DisplayOutput) -> Result<CursorFrame> {
    let mut info = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetCursorInfo(&mut info) }.context("read the Windows desktop cursor")?;
    let mut frame = CursorFrame {
        serial: 0,
        visible: info.flags.0 & CURSOR_SHOWING.0 != 0 && !info.hCursor.is_invalid(),
        x: (f64::from(info.ptScreenPos.x) - f64::from(output.x)) / f64::from(output.width),
        y: (f64::from(info.ptScreenPos.y) - f64::from(output.y)) / f64::from(output.height),
        width: 0,
        height: 0,
        hotspot_x: 0,
        hotspot_y: 0,
        units_width: output.width,
        units_height: output.height,
        pixels: Arc::default(),
    };
    SHAPE.with_borrow_mut(|cached| {
        if !frame.visible {
            *cached = None;
            return Ok(frame);
        }
        let icon = HICON(info.hCursor.0);
        if cached.as_ref().is_none_or(|shape| {
            shape.icon != icon || shape.sampled.elapsed() >= Duration::from_millis(250)
        }) {
            *cached = Some(read_shape(icon)?);
        }
        let shape = cached.as_ref().expect("cursor shape has been sampled");
        frame.serial = shape.serial;
        frame.width = shape.width;
        frame.height = shape.height;
        frame.hotspot_x = shape.hotspot_x;
        frame.hotspot_y = shape.hotspot_y;
        frame.pixels = shape.pixels.clone();
        Ok(frame)
    })
}

struct IconBitmaps(ICONINFO);

impl Drop for IconBitmaps {
    fn drop(&mut self) {
        unsafe {
            if !self.0.hbmColor.is_invalid() {
                let _ = DeleteObject(self.0.hbmColor.into());
            }
            if !self.0.hbmMask.is_invalid() {
                let _ = DeleteObject(self.0.hbmMask.into());
            }
        }
    }
}

fn read_shape(icon: HICON) -> Result<Shape> {
    let mut info = IconBitmaps(ICONINFO::default());
    unsafe { GetIconInfo(icon, &mut info.0) }.context("read Windows cursor hotspot and bitmaps")?;
    let monochrome = info.0.hbmColor.is_invalid();
    let bitmap = if monochrome {
        info.0.hbmMask
    } else {
        info.0.hbmColor
    };
    let mut dimensions = BITMAP::default();
    let size = size_of::<BITMAP>() as i32;
    let read = unsafe {
        GetObjectW(
            bitmap.into(),
            size,
            Some((&mut dimensions as *mut BITMAP).cast()),
        )
    };
    ensure!(read == size, "read Windows cursor bitmap size");
    let width = dimensions.bmWidth;
    // A monochrome cursor stores two vertically stacked masks.
    let height = if monochrome {
        dimensions.bmHeight / 2
    } else {
        dimensions.bmHeight
    };
    ensure!(
        (1..=512).contains(&width) && (1..=512).contains(&height),
        "invalid Windows cursor size"
    );
    let pixels = rasterize(icon, width, height)?;
    let (width, height) = (width as u32, height as u32);
    let hotspot_x = info.0.xHotspot.min(width - 1);
    let hotspot_y = info.0.yHotspot.min(height - 1);
    let mut hash = DefaultHasher::new();
    (width, height, hotspot_x, hotspot_y, &pixels).hash(&mut hash);
    Ok(Shape {
        icon,
        sampled: Instant::now(),
        serial: hash.finish().max(1),
        width,
        height,
        hotspot_x,
        hotspot_y,
        pixels: pixels.into(),
    })
}

struct CursorRaster {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Drop for CursorRaster {
    fn drop(&mut self) {
        unsafe {
            if !self.previous.is_invalid() {
                let _ = SelectObject(self.dc, self.previous);
            }
            let _ = DeleteDC(self.dc);
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap.into());
            }
        }
    }
}

fn rasterize(icon: HICON, width: i32, height: i32) -> Result<Vec<u8>> {
    let dc = unsafe { CreateCompatibleDC(None) };
    ensure!(!dc.is_invalid(), "create Windows cursor drawing context");
    let mut raster = CursorRaster {
        dc,
        bitmap: HBITMAP::default(),
        previous: HGDIOBJ::default(),
    };
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 2 * width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = null_mut();
    raster.bitmap =
        unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    ensure!(!bits.is_null(), "Windows cursor bitmap is not mapped");
    let previous = unsafe { SelectObject(dc, raster.bitmap.into()) };
    ensure!(!previous.is_invalid(), "select Windows cursor bitmap");
    raster.previous = previous;
    let stride = width as usize * 4;
    let len = stride * height as usize * 2;
    // Drawing over black and white recovers both premultiplied color and alpha
    // using the system's rasterizer for color and legacy mask-based cursors.
    for row in
        unsafe { slice::from_raw_parts_mut(bits.cast::<u8>(), len) }.chunks_exact_mut(2 * stride)
    {
        let (dark, light) = row.split_at_mut(stride);
        dark.as_chunks_mut::<4>().0.fill([0, 0, 0, 255]);
        light.fill(255);
    }
    unsafe {
        DrawIconEx(dc, 0, 0, icon, width, height, 0, None, DI_NORMAL)?;
        DrawIconEx(dc, width, 0, icon, width, height, 0, None, DI_NORMAL)?;
        GdiFlush().ok()?;
    }
    let mut bgra = Vec::with_capacity(len / 2);
    let mut inverted = Vec::new();
    for row in unsafe { slice::from_raw_parts(bits.cast::<u8>(), len) }.chunks_exact(2 * stride) {
        let (dark, light) = row.split_at(stride);
        for (dark, light) in dark.as_chunks::<4>().0.iter().zip(light.as_chunks::<4>().0) {
            let (dark, light) = (&dark[..3], &light[..3]);
            if dark.iter().zip(light).any(|(dark, light)| dark > light) {
                // A PNG cannot invert its backdrop. Use an outlined dark stroke
                // for such pixels so the legacy I-beam remains legible.
                inverted.push(bgra.len() / 4);
                bgra.extend([0, 0, 0, 255]);
                continue;
            }
            let uncovered = dark
                .iter()
                .zip(light)
                .map(|(dark, light)| light - dark)
                .max()
                .unwrap_or(0);
            let alpha = 255 - uncovered;
            bgra.extend([
                dark[0].min(alpha),
                dark[1].min(alpha),
                dark[2].min(alpha),
                alpha,
            ]);
        }
    }
    let (width, height) = (width as usize, height as usize);
    let pixels = bgra.as_chunks_mut::<4>().0;
    for index in inverted {
        let (x, y) = (index % width, index / width);
        for (x, y) in [
            (x.saturating_sub(1), y),
            (x + 1, y),
            (x, y.saturating_sub(1)),
            (x, y + 1),
        ] {
            if x < width && y < height && pixels[y * width + x][3] == 0 {
                pixels[y * width + x] = [255; 4];
            }
        }
    }
    Ok(bgra)
}
