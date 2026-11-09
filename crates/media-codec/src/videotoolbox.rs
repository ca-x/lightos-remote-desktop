use crate::{
    EncodedFrame, EncoderRate, EncoderSettings, VideoEncoder, default_bitrate_for_fps,
    h264_packet_info,
};
use anyhow::{Result, ensure};
pub use apple_cf::cv::CVPixelBuffer as NativePixelBuffer;
use apple_cf::cv::CVPixelBufferLockFlags;
use futures_util::future::BoxFuture;
use std::{ffi::c_void, ptr::NonNull, sync::Arc, time::Duration};
use tokio::sync::oneshot;

unsafe extern "C" {
    fn lrd_vt_create(
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        error: *mut i32,
    ) -> *mut c_void;
    fn lrd_vt_destroy(encoder: *mut c_void);
    fn lrd_vt_rate(encoder: *mut c_void, bitrate: u32, fps: u32) -> i32;
    fn lrd_vt_submit(
        encoder: *mut c_void,
        pixels: *mut c_void,
        key: bool,
        context: *mut c_void,
        done: extern "C" fn(*mut c_void, i32, *const u8, usize),
    );
    fn lrd_vt_rgba(rgba: *const u8, width: u32, height: u32) -> *mut c_void;
    pub fn lrd_capture_age(sample: *mut c_void) -> f64;
}
type Reply = oneshot::Sender<Result<EncodedFrame>>;
extern "C" fn completed(context: *mut c_void, status: i32, bytes: *const u8, length: usize) {
    // The native callback consumes the transferred Box exactly once. It never
    // references the Rust encoder, so cancelled requests cannot leave a dangling borrow.
    let sender = unsafe { Box::from_raw(context.cast::<Reply>()) };
    let result = if status != 0 {
        Err(anyhow::anyhow!("VideoToolbox encode: OSStatus {status}"))
    } else if length > 16 * 1024 * 1024 {
        Err(anyhow::anyhow!("VideoToolbox output exceeds limit"))
    } else {
        let data = if bytes.is_null() || length == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec()
        };
        let (key, codec) = h264_packet_info(&data);
        Ok(EncodedFrame { data, key, codec })
    };
    let _ = sender.send(result);
}
pub struct VideoToolbox {
    native: NonNull<c_void>,
    settings: EncoderSettings,
    rate: EncoderRate,
    key: bool,
}
// Submission is serialized by &mut self; callback state is independently owned.
unsafe impl Send for VideoToolbox {}
impl VideoToolbox {
    pub fn new(settings: EncoderSettings) -> Result<Self> {
        crate::worker_protocol::validate_settings(settings)?;
        let rate = EncoderRate {
            fps: settings.fps,
            bitrate_bps: default_bitrate_for_fps(settings.width, settings.height, settings.fps),
        };
        let mut error = 0;
        let native = unsafe {
            lrd_vt_create(
                settings.width,
                settings.height,
                rate.fps,
                rate.bitrate_bps,
                &mut error,
            )
        };
        Ok(Self {
            native: NonNull::new(native).ok_or_else(|| {
                anyhow::anyhow!("create native VideoToolbox session: OSStatus {error}")
            })?,
            settings,
            rate,
            key: true,
        })
    }
}
impl Drop for VideoToolbox {
    fn drop(&mut self) {
        unsafe { lrd_vt_destroy(self.native.as_ptr()) }
    }
}
impl VideoEncoder for VideoToolbox {
    fn dimensions(&self) -> (u32, u32) {
        (self.settings.width, self.settings.height)
    }
    fn accepts_native(&self) -> bool {
        true
    }
    fn set_rate(&mut self, rate: EncoderRate) -> Result<()> {
        rate.validate()?;
        if rate != self.rate {
            let status = unsafe { lrd_vt_rate(self.native.as_ptr(), rate.bitrate_bps, rate.fps) };
            ensure!(status == 0, "update VideoToolbox rate: {status}");
            self.rate = rate;
        }
        Ok(())
    }
    fn encode(&mut self, pixels: Arc<[u8]>) -> BoxFuture<'_, Result<EncodedFrame>> {
        Box::pin(async move {
            crate::worker_protocol::validate_pixels(self.settings, &pixels)?;
            let raw =
                unsafe { lrd_vt_rgba(pixels.as_ptr(), self.settings.width, self.settings.height) };
            let buffer = unsafe { NativePixelBuffer::from_raw(raw) }
                .ok_or_else(|| anyhow::anyhow!("allocate VideoToolbox input"))?;
            self.encode_native(buffer).await
        })
    }
    fn encode_native(&mut self, pixels: NativePixelBuffer) -> BoxFuture<'_, Result<EncodedFrame>> {
        Box::pin(async move {
            let (sender, reply) = oneshot::channel::<Result<EncodedFrame>>();
            let context = Box::into_raw(Box::new(sender));
            unsafe {
                lrd_vt_submit(
                    self.native.as_ptr(),
                    pixels.as_ptr(),
                    self.key,
                    context.cast(),
                    completed,
                )
            };
            let result = tokio::time::timeout(Duration::from_secs(3), reply)
                .await
                .map_err(|_| anyhow::anyhow!("VideoToolbox output timed out"))?
                .map_err(|_| anyhow::anyhow!("VideoToolbox callback ended"))??;
            if !result.data.is_empty() {
                self.key = false;
            }
            Ok(result)
        })
    }
}
pub fn rgba(buffer: &NativePixelBuffer) -> Result<Arc<[u8]>> {
    let guard = buffer
        .lock(CVPixelBufferLockFlags::READ_ONLY)
        .map_err(|e| anyhow::anyhow!("lock native snapshot: {e:?}"))?;
    let (width, height, stride) = (guard.width(), guard.height(), guard.bytes_per_row());
    ensure!(
        width > 0 && height > 0 && width <= 16384 && height <= 16384,
        "invalid native snapshot dimensions"
    );
    let source = unsafe { guard.as_slice() }
        .ok_or_else(|| anyhow::anyhow!("native snapshot is not packed BGRA"))?;
    ensure!(
        stride >= width * 4 && source.len() >= stride * (height - 1) + width * 4,
        "invalid native snapshot stride"
    );
    let mut pixels = vec![0; width * height * 4];
    for y in 0..height {
        for (source, target) in source[y * stride..y * stride + width * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(
                pixels[y * width * 4..(y + 1) * width * 4]
                    .as_chunks_mut::<4>()
                    .0,
            )
        {
            target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
        }
    }
    Ok(pixels.into())
}
