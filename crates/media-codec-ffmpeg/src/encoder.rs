use anyhow::{Context, Result, bail, ensure};
use ffmpeg_next::{
    self as ffmpeg, Dictionary, Packet, codec, encoder, ffi, format::Pixel, frame,
    software::scaling,
};
use media_codec::{
    EncodedFrame, EncoderBackend, EncoderRate, EncoderSettings, default_bitrate, h264_packet_info,
    worker_protocol::{validate_pixels, validate_settings},
};
use std::{ffi::CString, ptr};

/// References attached to AVCodecContext/AVFrame remain owned by FFmpeg. These
/// wrappers hold separate references and release them exactly once.
struct Buffer(*mut ffi::AVBufferRef);
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            ffi::av_buffer_unref(&mut self.0);
        }
    }
}
struct Bitstream(*mut ffi::AVBSFContext);
impl Drop for Bitstream {
    fn drop(&mut self) {
        unsafe {
            ffi::av_bsf_free(&mut self.0);
        }
    }
}
impl Bitstream {
    fn new(encoder: &encoder::Video) -> Result<Self> {
        // SAFETY: generated ABI, live context and owned filter reference. The
        // filter copies codec parameters and owns all packet refs it receives.
        unsafe {
            let filter = ffi::av_bsf_get_by_name(c"h264_mp4toannexb".as_ptr());
            ensure!(
                !filter.is_null(),
                "FFmpeg h264_mp4toannexb filter unavailable"
            );
            let mut context = ptr::null_mut();
            check(
                ffi::av_bsf_alloc(filter, &mut context),
                "allocate H264 filter",
            )?;
            let filter = Self(context);
            check(
                ffi::avcodec_parameters_from_context((*context).par_in, encoder.as_ptr()),
                "copy encoder parameters",
            )?;
            (*context).time_base_in = (*encoder.as_ptr()).time_base;
            check(ffi::av_bsf_init(context), "initialize H264 filter")?;
            Ok(filter)
        }
    }
    fn convert(&mut self, packet: &mut Packet) -> Result<Vec<u8>> {
        use ffmpeg::packet::Mut;
        let mut result = Vec::new();
        unsafe {
            check(
                ffi::av_bsf_send_packet(self.0, packet.as_mut_ptr()),
                "filter H264 packet",
            )?;
            loop {
                let mut output = Packet::empty();
                let status = ffi::av_bsf_receive_packet(self.0, output.as_mut_ptr());
                if status == -ffmpeg::error::EAGAIN || status == ffi::AVERROR_EOF {
                    break;
                }
                check(status, "read filtered H264 packet")?;
                result.extend_from_slice(output.data().context("empty filtered packet")?);
            }
        }
        Ok(result)
    }
}
pub struct NativeEncoder {
    name: &'static str,
    backend: EncoderBackend,
    settings: EncoderSettings,
    rate: EncoderRate,
    encoder: encoder::Video,
    filter: Bitstream,
    upload: Option<Buffer>,
    scaler: scaling::Context,
    pts: i64,
}
impl NativeEncoder {
    pub fn new(
        backend: EncoderBackend,
        settings: EncoderSettings,
        rate: EncoderRate,
    ) -> Result<Self> {
        ensure!(
            backend.is_hardware() && backend.supported_platform(),
            "unsupported hardware backend/platform"
        );
        let name = match backend {
            EncoderBackend::Vaapi => "h264_vaapi",
            EncoderBackend::Nvenc => "h264_nvenc",
            EncoderBackend::Amf => "h264_amf",
            EncoderBackend::VideoToolbox => "h264_videotoolbox",
            _ => bail!("software encoding is handled by the core"),
        };
        Self::open(name, backend, settings, rate)
    }
    fn open(
        name: &'static str,
        backend: EncoderBackend,
        settings: EncoderSettings,
        rate: EncoderRate,
    ) -> Result<Self> {
        validate_settings(settings)?;
        rate.validate()?;
        let codec = encoder::find_by_name(name)
            .with_context(|| format!("FFmpeg encoder {name} is not compiled in"))?;
        let mut encoder = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        encoder.set_width(settings.width);
        encoder.set_height(settings.height);
        encoder.set_format(if backend == EncoderBackend::Vaapi {
            Pixel::VAAPI
        } else {
            Pixel::NV12
        });
        encoder.set_time_base((1, rate.fps as i32));
        encoder.set_frame_rate(Some((rate.fps as i32, 1)));
        encoder.set_bit_rate(rate.bitrate_bps as usize);
        encoder.set_max_bit_rate(rate.bitrate_bps as usize);
        encoder.set_gop(rate.fps * 2);
        encoder.set_max_b_frames(0);
        encoder.set_flags(if backend.is_hardware() {
            codec::Flags::LOW_DELAY
        } else {
            codec::Flags::LOW_DELAY | codec::Flags::GLOBAL_HEADER
        });
        // Keep VBV bounded; this is a real native bitrate, not just FPS pacing.
        unsafe {
            (*encoder.as_mut_ptr()).rc_buffer_size = rate.bitrate_bps as i32;
        }
        let upload = if backend == EncoderBackend::Vaapi {
            Some(vaapi_frames(&mut encoder, settings)?)
        } else {
            None
        };
        let mut opts = Dictionary::new();
        match backend {
            EncoderBackend::Nvenc => {
                for (k, v) in [
                    ("preset", "p4"),
                    ("tune", "ull"),
                    ("zerolatency", "1"),
                    ("rc", "cbr"),
                    ("rc-lookahead", "0"),
                ] {
                    opts.set(k, v);
                }
            }
            EncoderBackend::Amf => {
                for (k, v) in [
                    ("usage", "ultralowlatency"),
                    ("quality", "speed"),
                    ("rc", "cbr"),
                    ("preanalysis", "0"),
                    ("header_insertion_mode", "gop"),
                ] {
                    opts.set(k, v);
                }
            }
            EncoderBackend::Vaapi => {
                opts.set("rc_mode", "CBR");
                opts.set("async_depth", "1");
            }
            EncoderBackend::VideoToolbox => {
                opts.set("realtime", "1");
                opts.set("allow_sw", "0");
            }
            _ => {
                opts.set("preset", "ultrafast");
                opts.set("tune", "zerolatency");
                opts.set("x264-params", "annexb=0");
            }
        }
        let encoder = encoder
            .open_as_with(codec, opts)
            .with_context(|| format!("initialize {name}"))?;
        if backend == EncoderBackend::VideoToolbox {
            // VideoToolbox can silently fall back to software unless forbidden.
            let mut allow = -1_i64;
            unsafe {
                check(
                    ffi::av_opt_get_int(
                        (*encoder.as_ptr()).priv_data,
                        c"allow_sw".as_ptr(),
                        0,
                        &mut allow,
                    ),
                    "verify VideoToolbox hardware-only mode",
                )?;
            }
            ensure!(
                allow == 0,
                "VideoToolbox software fallback must be disabled"
            );
        }
        let filter = Bitstream::new(&encoder)?;
        let scaler = scaling::Context::get(
            Pixel::RGBA,
            settings.width,
            settings.height,
            Pixel::NV12,
            settings.width,
            settings.height,
            scaling::flag::Flags::BILINEAR,
        )?;
        Ok(Self {
            name,
            backend,
            settings,
            rate,
            encoder,
            filter,
            upload,
            scaler,
            pts: 0,
        })
    }
    pub fn encode(&mut self, pixels: &[u8], rate: EncoderRate) -> Result<EncodedFrame> {
        validate_pixels(self.settings, pixels)?;
        rate.validate()?;
        if self.rate != rate {
            // No common reliable live-rate API exists across these drivers.
            // Reopen explicitly; the next access unit must be an IDR with SPS.
            *self = Self::open(self.name, self.backend, self.settings, rate)?;
        }
        let mut rgba = frame::Video::new(Pixel::RGBA, self.settings.width, self.settings.height);
        let row = self.settings.width as usize * 4;
        let stride = rgba.stride(0);
        for (y, source) in pixels.chunks_exact(row).enumerate() {
            rgba.data_mut(0)[y * stride..y * stride + row].copy_from_slice(source);
        }
        let mut converted = frame::Video::empty();
        self.scaler.run(&rgba, &mut converted)?;
        let mut native = frame::Video::empty();
        let frame = if let Some(upload) = &self.upload {
            unsafe {
                check(
                    ffi::av_hwframe_get_buffer(upload.0, native.as_mut_ptr(), 0),
                    "allocate VA-API frame",
                )?;
                check(
                    ffi::av_hwframe_transfer_data(native.as_mut_ptr(), converted.as_ptr(), 0),
                    "upload VA-API frame",
                )?;
            }
            &mut native
        } else {
            &mut converted
        };
        frame.set_pts(Some(self.pts));
        self.pts += 1;
        self.encoder.send_frame(frame)?;
        let mut data = Vec::new();
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => data.extend_from_slice(&self.filter.convert(&mut packet)?),
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => break,
                Err(error) => return Err(error.into()),
            }
        }
        ensure!(
            !data.is_empty(),
            "hardware encoder buffers frames; low-latency output unavailable"
        );
        let (key, codec) = h264_packet_info(&data);
        if self.pts == 1 {
            ensure!(
                key && codec.is_some(),
                "hardware encoder lacks an initial IDR/SPS"
            );
        }
        Ok(EncodedFrame { data, key, codec })
    }
}
fn check(status: i32, action: &str) -> Result<()> {
    if status < 0 {
        bail!("{action}: {}", ffmpeg::Error::from(status));
    }
    Ok(())
}
fn vaapi_frames(encoder: &mut encoder::video::Video, settings: EncoderSettings) -> Result<Buffer> {
    let mut candidates: Vec<_> = std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .map(|e| e.path())
        .collect();
    candidates.sort();
    ensure!(!candidates.is_empty(), "no VA-API render nodes found");
    let mut errors = Vec::new();
    for path in candidates {
        let name = CString::new(path.to_string_lossy().as_bytes())?;
        let mut device = ptr::null_mut();
        // SAFETY: generated FFmpeg ABI, owned buffer refs, no retained CString
        // pointer. The codec gets its own ref and releases it with its context.
        unsafe {
            let status = ffi::av_hwdevice_ctx_create(
                &mut device,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                name.as_ptr(),
                ptr::null_mut(),
                0,
            );
            if status < 0 {
                errors.push(format!(
                    "{}: {}",
                    path.display(),
                    ffmpeg::Error::from(status)
                ));
                continue;
            }
            let device = Buffer(device);
            let frames = Buffer(ffi::av_hwframe_ctx_alloc(device.0));
            ensure!(!frames.0.is_null(), "allocate VA-API frame context failed");
            let context = (*frames.0).data.cast::<ffi::AVHWFramesContext>();
            (*context).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*context).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            (*context).width = settings.width as i32;
            (*context).height = settings.height as i32;
            (*context).initial_pool_size = 4;
            check(
                ffi::av_hwframe_ctx_init(frames.0),
                "initialize VA-API frames",
            )?;
            let owned = ffi::av_buffer_ref(frames.0);
            ensure!(!owned.is_null(), "reference VA-API frames failed");
            (*encoder.as_mut_ptr()).hw_frames_ctx = owned;
            return Ok(frames);
        }
    }
    bail!("VA-API device initialization failed: {}", errors.join("; "))
}
pub fn probe(backend: EncoderBackend) -> Result<String> {
    let settings = EncoderSettings {
        width: 640,
        height: 360,
        fps: 30,
        quality: 75,
    };
    let rate = EncoderRate {
        bitrate_bps: default_bitrate(settings.width, settings.height),
        fps: 30,
    };
    verify(NativeEncoder::new(backend, settings, rate)?, settings, rate)
}
fn verify(
    mut encoder: NativeEncoder,
    settings: EncoderSettings,
    rate: EncoderRate,
) -> Result<String> {
    let mut decoder = codec::context::Context::new_with_codec(
        ffmpeg::decoder::find(codec::Id::H264).context("H264 verification decoder unavailable")?,
    )
    .decoder()
    .video()?;
    let mut profile = None;
    let mut decoded = 0;
    for i in 0..8 {
        let mut pixels = vec![0; settings.width as usize * settings.height as usize * 4];
        for (index, rgba) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            rgba.copy_from_slice(&[
                (index % settings.width as usize) as u8,
                (index / settings.width as usize) as u8,
                i * 20,
                255,
            ]);
        }
        let actual = if i < 4 {
            rate
        } else {
            EncoderRate {
                bitrate_bps: 300_000,
                fps: 15,
            }
        };
        let encoded = encoder.encode(&pixels, actual)?;
        if i == 4 {
            ensure!(
                encoded.key && encoded.codec.is_some(),
                "rate restart lacks IDR/SPS"
            );
        }
        if encoded.codec.is_some() {
            profile = encoded.codec;
        }
        decoder.send_packet(&Packet::copy(&encoded.data))?;
        let mut picture = frame::Video::empty();
        while decoder.receive_frame(&mut picture).is_ok() {
            ensure!(
                picture.width() == settings.width && picture.height() == settings.height,
                "hardware probe decoded wrong dimensions"
            );
            let stride = picture.stride(0);
            let (min, max) = (0..settings.height as usize)
                .flat_map(|y| {
                    picture.data(0)[y * stride..y * stride + settings.width as usize]
                        .iter()
                        .copied()
                })
                .fold((255_u8, 0_u8), |(min, max), sample| {
                    (min.min(sample), max.max(sample))
                });
            ensure!(
                max.saturating_sub(min) > 32,
                "hardware probe decoded a blank frame"
            );
            decoded += 1;
        }
    }
    ensure!(
        decoded == 8,
        "hardware probe did not decode every frame without delay"
    );
    profile.context("hardware probe has no H264 profile")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_rgba_conversion_preserves_red_green_and_blue_channels() {
        ffmpeg::init().unwrap();
        let settings = EncoderSettings {
            width: 640,
            height: 360,
            fps: 30,
            quality: 75,
        };
        let rate = EncoderRate {
            bitrate_bps: 1_800_000,
            fps: 30,
        };
        let mut encoder =
            NativeEncoder::open("libx264", EncoderBackend::OpenH264, settings, rate).unwrap();
        let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        let mut pixels = vec![0; 640 * 360 * 4];
        for (i, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *pixel = colors[(i % 640 * 3 / 640).min(2)];
        }
        let encoded = encoder.encode(&pixels, rate).unwrap();
        let mut decoder = codec::context::Context::new_with_codec(
            ffmpeg::decoder::find(codec::Id::H264).unwrap(),
        )
        .decoder()
        .video()
        .unwrap();
        decoder.send_packet(&Packet::copy(&encoded.data)).unwrap();
        let mut picture = frame::Video::empty();
        decoder.receive_frame(&mut picture).unwrap();
        let mut scaler = scaling::Context::get(
            picture.format(),
            640,
            360,
            Pixel::RGBA,
            640,
            360,
            scaling::flag::Flags::BILINEAR,
        )
        .unwrap();
        let mut rgba = frame::Video::empty();
        scaler.run(&picture, &mut rgba).unwrap();
        for (index, x) in [80, 320, 560].into_iter().enumerate() {
            let offset = rgba.stride(0) * 180 + x * 4;
            for (channel, expected) in colors[index][..3].iter().enumerate() {
                assert!(
                    rgba.data(0)[offset + channel].abs_diff(*expected) < 40,
                    "color channel changed"
                );
            }
        }
    }
    #[test]
    fn native_scaling_annex_b_prediction_and_rate_restart_decode() {
        ffmpeg::init().unwrap();
        let settings = EncoderSettings {
            width: 640,
            height: 360,
            fps: 30,
            quality: 75,
        };
        let rate = EncoderRate {
            bitrate_bps: 900_000,
            fps: 30,
        };
        // Private fixture exercises FFmpeg without advertising CPU as hardware.
        let encoder =
            NativeEncoder::open("libx264", EncoderBackend::OpenH264, settings, rate).unwrap();
        assert!(
            verify(encoder, settings, rate)
                .unwrap()
                .starts_with("avc1.")
        );
    }
}
