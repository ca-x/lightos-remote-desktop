//! Observe the system pointer separately from ScreenCaptureKit video.
use crate::backend::CursorFrame;
use anyhow::{Context, Result, ensure};
use display_control::DisplayOutput;
use objc2::rc::{Retained, autoreleasepool};
use objc2_app_kit::NSCursor;
use screencapturekit::prelude::CGRect;
use std::{
    ffi::c_void,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

pub(super) fn capture(output: &DisplayOutput) -> Result<Option<CursorFrame>> {
    autoreleasepool(|_| {
        // currentCursor is application-local; currentSystemCursor observes the
        // actual cursor set by the foreground application. Never substitute an
        // arrow when the system cannot provide a cursor image.
        #[allow(deprecated)]
        let Some(cursor) = NSCursor::currentSystemCursor() else {
            return Ok(None);
        };
        let image = cursor.image();
        let size = image.size();
        let hotspot = cursor.hotSpot();
        let display: u32 = output
            .name
            .strip_prefix("display-")
            .unwrap_or(&output.name)
            .parse()
            .context("invalid macOS cursor display")?;
        let bounds = unsafe { super::CGDisplayBounds(display) };
        ensure!(
            bounds.size.width > 0. && bounds.size.height > 0.,
            "invalid cursor display bounds"
        );
        let sx = f64::from(output.width) / bounds.size.width;
        let sy = f64::from(output.height) / bounds.size.height;
        let width = (size.width * sx).round() as u32;
        let height = (size.height * sy).round() as u32;
        ensure!(
            (1..=512).contains(&width) && (1..=512).contains(&height),
            "invalid system cursor image size"
        );
        let cg =
            unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }
                .context("system cursor has no image")?;
        let mut pixels = vec![0_u8; width as usize * height as usize * 4];
        unsafe {
            let color = CGColorSpaceCreateDeviceRGB();
            ensure!(!color.is_null(), "cursor color space unavailable");
            // Little-endian premultiplied-first is the cursor protocol's BGRA.
            let context = CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width as usize,
                height as usize,
                8,
                width as usize * 4,
                color,
                0x2002,
            );
            CGColorSpaceRelease(color);
            ensure!(!context.is_null(), "cursor bitmap unavailable");
            let rect = CGRect {
                origin: super::CGPoint { x: 0., y: 0. },
                size: super::CGSize {
                    width: f64::from(width),
                    height: f64::from(height),
                },
            };
            CGContextDrawImage(context, rect, Retained::as_ptr(&cg).cast());
            CGContextRelease(context);
        }
        let event = unsafe { CGEventCreate(std::ptr::null()) };
        ensure!(!event.is_null(), "cursor position unavailable");
        let point = unsafe { CGEventGetLocation(event) };
        unsafe { core_foundation_sys::base::CFRelease(event) };
        let hx = (hotspot.x * sx).round().max(0.) as u32;
        let hy = (hotspot.y * sy).round().max(0.) as u32;
        let mut hash = DefaultHasher::new();
        (width, height, hx, hy, &pixels).hash(&mut hash);
        Ok(Some(CursorFrame {
            serial: hash.finish().max(1),
            visible: unsafe { CGCursorIsVisible() },
            x: (point.x - bounds.origin.x) / bounds.size.width,
            y: (point.y - bounds.origin.y) / bounds.size.height,
            width,
            height,
            hotspot_x: hx,
            hotspot_y: hy,
            units_width: output.width,
            units_height: output.height,
            pixels: Arc::from(pixels),
        }))
    })
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
    fn CGColorSpaceRelease(color: *const c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits: usize,
        stride: usize,
        color: *const c_void,
        info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(context: *mut c_void, rect: CGRect, image: *const c_void);
    fn CGContextRelease(context: *mut c_void);
    fn CGEventCreate(source: *const c_void) -> *const c_void;
    fn CGEventGetLocation(event: *const c_void) -> super::CGPoint;
    fn CGCursorIsVisible() -> bool;
}
