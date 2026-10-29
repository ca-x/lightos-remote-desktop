use anyhow::{Context, Result, bail, ensure};
use ffmpeg_next::{self as ffmpeg, Dictionary, Packet, codec, encoder, ffi, format::Pixel, frame};
use media_codec::{
    EncodedFrame, EncoderBackend, EncoderRate, EncoderSettings, default_bitrate, h264_packet_info,
    worker_protocol::{EncoderDevice, validate_pixels, validate_settings},
};
use std::{
    ffi::CString,
    path::{Path, PathBuf},
    ptr,
};

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
// The frame API can split conversion across a bounded number of workers;
// sws_scale's legacy slice API keeps RGBA conversion on one CPU thread.
struct PixelConverter(*mut ffi::SwsContext);
impl PixelConverter {
    fn new(width: u32, height: u32) -> Result<Self> {
        let converter = Self(unsafe { ffi::sws_alloc_context() });
        ensure!(!converter.0.is_null(), "allocate pixel converter");
        unsafe {
            for (option, value) in [
                (c"srcw", i64::from(width)),
                (c"srch", i64::from(height)),
                (c"dstw", i64::from(width)),
                (c"dsth", i64::from(height)),
                (c"src_format", ffi::AVPixelFormat::AV_PIX_FMT_RGBA as i64),
                (c"dst_format", ffi::AVPixelFormat::AV_PIX_FMT_NV12 as i64),
                (
                    c"sws_flags",
                    i64::from(ffmpeg::software::scaling::flag::Flags::BILINEAR.bits()),
                ),
                (c"threads", 2),
            ] {
                check(
                    ffi::av_opt_set_int(converter.0.cast(), option.as_ptr(), value, 0),
                    "configure pixel conversion",
                )?;
            }
            check(
                ffi::sws_init_context(converter.0, ptr::null_mut(), ptr::null_mut()),
                "initialize pixel converter",
            )?;
        }
        Ok(converter)
    }
    #[cfg(not(all(target_os = "linux", feature = "fast-yuv")))]
    fn convert(&mut self, source: &frame::Video, target: &mut frame::Video) -> Result<()> {
        unsafe {
            check(
                ffi::sws_scale_frame(self.0, target.as_mut_ptr(), source.as_ptr()),
                "convert RGBA to NV12",
            )
        }
    }
}
impl Drop for PixelConverter {
    fn drop(&mut self) {
        unsafe { ffi::sws_freeContext(self.0) }
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
/// Reuse CPU storage while respecting references retained by FFmpeg/drivers.
struct CpuFrames {
    #[cfg(not(all(target_os = "linux", feature = "fast-yuv")))]
    rgba: frame::Video,
    converted: frame::Video,
}
impl CpuFrames {
    fn new(width: u32, height: u32) -> Self {
        Self {
            #[cfg(not(all(target_os = "linux", feature = "fast-yuv")))]
            rgba: frame::Video::new(Pixel::RGBA, width, height),
            converted: frame::Video::new(Pixel::NV12, width, height),
        }
    }
    #[cfg(not(all(target_os = "linux", feature = "fast-yuv")))]
    fn convert(&mut self, pixels: &[u8], scaler: &mut PixelConverter) -> Result<()> {
        unsafe {
            check(
                ffi::av_frame_make_writable(self.rgba.as_mut_ptr()),
                "make RGBA frame writable",
            )?;
            check(
                ffi::av_frame_make_writable(self.converted.as_mut_ptr()),
                "make NV12 frame writable",
            )?;
        }
        let row = self.rgba.width() as usize * 4;
        let stride = self.rgba.stride(0);
        for (y, source) in pixels.chunks_exact(row).enumerate() {
            self.rgba.data_mut(0)[y * stride..y * stride + row].copy_from_slice(source);
        }
        scaler.convert(&self.rgba, &mut self.converted)?;
        Ok(())
    }
    #[cfg(all(target_os = "linux", feature = "fast-yuv"))]
    fn convert(&mut self, pixels: &[u8], _scaler: &mut PixelConverter) -> Result<()> {
        // libyuv's ABGR name describes the word order; its byte input is RGBA.
        // Conversion is synchronous, so the request's validated pixel slice can
        // be used directly without allocating and copying another RGBA frame.
        let width = self.converted.width() as i32;
        let height = self.converted.height() as i32;
        let y_stride = self.converted.stride(0) as i32;
        let uv_stride = self.converted.stride(1) as i32;
        unsafe {
            check(
                ffi::av_frame_make_writable(self.converted.as_mut_ptr()),
                "make NV12 frame writable",
            )?;
            let status = ABGRToNV12(
                pixels.as_ptr(),
                width * 4,
                self.converted.data_mut(0).as_mut_ptr(),
                y_stride,
                self.converted.data_mut(1).as_mut_ptr(),
                uv_stride,
                width,
                height,
            );
            ensure!(status == 0, "convert RGBA to NV12: libyuv status {status}");
        }
        Ok(())
    }
}

#[cfg(all(target_os = "linux", feature = "fast-yuv"))]
unsafe extern "C" {
    fn ABGRToNV12(
        source: *const u8,
        source_stride: i32,
        y: *mut u8,
        y_stride: i32,
        uv: *mut u8,
        uv_stride: i32,
        width: i32,
        height: i32,
    ) -> i32;
}
pub struct NativeEncoder {
    name: &'static str,
    backend: EncoderBackend,
    settings: EncoderSettings,
    rate: EncoderRate,
    encoder: encoder::Video,
    filter: Bitstream,
    upload: Option<Buffer>,
    scaler: PixelConverter,
    frames: CpuFrames,
    pts: i64,
    device: Option<EncoderDevice>,
}
impl NativeEncoder {
    pub fn new(
        backend: EncoderBackend,
        settings: EncoderSettings,
        rate: EncoderRate,
        device: Option<&EncoderDevice>,
    ) -> Result<Self> {
        ensure!(
            backend.is_hardware() && backend.supported_platform(),
            "unsupported hardware backend/platform"
        );
        if let Some(device) = device {
            device.validate()?;
            ensure!(
                device.backend() == backend,
                "encoder device/backend mismatch"
            );
        }
        let name = match backend {
            EncoderBackend::Vaapi => "h264_vaapi",
            EncoderBackend::Nvenc => "h264_nvenc",
            EncoderBackend::Amf => "h264_amf",
            EncoderBackend::VideoToolbox => "h264_videotoolbox",
            _ => bail!("software encoding is handled by the core"),
        };
        if backend == EncoderBackend::Vaapi {
            // Validate the complete encoder and rate-restart path per device.
            // A render node that merely creates a frame pool is not sufficient.
            return try_devices(vaapi_devices(device)?, |path| {
                let device = EncoderDevice::Vaapi(path.to_owned());
                let mut candidate =
                    Self::open_on_device(name, backend, settings, rate, Some(&device))?;
                verify(&mut candidate, settings, rate)?;
                drop(candidate);
                Self::open_on_device(name, backend, settings, rate, Some(&device))
            });
        }
        if device.is_some() {
            Self::open_on_device(name, backend, settings, rate, device)
        } else {
            Self::open(name, backend, settings, rate)
        }
    }
    fn open(
        name: &'static str,
        backend: EncoderBackend,
        settings: EncoderSettings,
        rate: EncoderRate,
    ) -> Result<Self> {
        Self::open_on_device(name, backend, settings, rate, None)
    }
    fn open_on_device(
        name: &'static str,
        backend: EncoderBackend,
        settings: EncoderSettings,
        rate: EncoderRate,
        device: Option<&EncoderDevice>,
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
        encoder.set_flags(codec::Flags::LOW_DELAY);
        // Keep VBV bounded; this is a real native bitrate, not just FPS pacing.
        unsafe {
            (*encoder.as_mut_ptr()).rc_buffer_size = rate.bitrate_bps as i32;
            #[cfg(all(target_os = "linux", feature = "fast-yuv"))]
            {
                // ABGRToNV12 uses the limited-range BT.601 matrix. Captured
                // desktop RGB has sRGB/BT.709 primaries, not a BT.709 YUV matrix.
                // Explicit VUI prevents browsers from guessing the HD matrix.
                (*encoder.as_mut_ptr()).colorspace = ffi::AVColorSpace::AVCOL_SPC_SMPTE170M;
                (*encoder.as_mut_ptr()).color_range = ffi::AVColorRange::AVCOL_RANGE_MPEG;
                (*encoder.as_mut_ptr()).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
                (*encoder.as_mut_ptr()).color_trc =
                    ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1;
            }
        }
        let upload = if backend == EncoderBackend::Vaapi {
            let Some(EncoderDevice::Vaapi(path)) = device else {
                bail!("missing VA-API device");
            };
            Some(vaapi_frames(&mut encoder, settings, path)?)
        } else {
            None
        };
        if let Some(EncoderDevice::Nvenc(index)) = device {
            select_nvenc_gpu(&mut encoder, *index)?;
        }
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
                opts.set("x264-params", "annexb=1:repeat-headers=1");
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
        let scaler = PixelConverter::new(settings.width, settings.height)?;
        Ok(Self {
            name,
            backend,
            settings,
            rate,
            encoder,
            filter,
            upload,
            scaler,
            frames: CpuFrames::new(settings.width, settings.height),
            pts: 0,
            device: device.cloned(),
        })
    }
    pub fn encode(&mut self, pixels: &[u8], rate: EncoderRate) -> Result<EncodedFrame> {
        validate_pixels(self.settings, pixels)?;
        rate.validate()?;
        if self.rate != rate {
            // No common reliable live-rate API exists across these drivers.
            // Reopen explicitly; the next access unit must be an IDR with SPS.
            *self = Self::open_on_device(
                self.name,
                self.backend,
                self.settings,
                rate,
                self.device.as_ref(),
            )?;
        }
        self.frames.convert(pixels, &mut self.scaler)?;
        let mut native = frame::Video::empty();
        let frame = if let Some(upload) = &self.upload {
            unsafe {
                check(
                    ffi::av_hwframe_get_buffer(upload.0, native.as_mut_ptr(), 0),
                    "allocate VA-API frame",
                )?;
                check(
                    ffi::av_hwframe_transfer_data(
                        native.as_mut_ptr(),
                        self.frames.converted.as_ptr(),
                        0,
                    ),
                    "upload VA-API frame",
                )?;
            }
            &mut native
        } else {
            &mut self.frames.converted
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
fn select_nvenc_gpu(encoder: &mut encoder::video::Video, index: u32) -> Result<()> {
    ensure!(index <= i32::MAX as u32, "invalid NVENC GPU index");
    unsafe {
        let private = (*encoder.as_mut_ptr()).priv_data;
        ensure!(!private.is_null(), "missing NVENC private context");
        check(
            ffi::av_opt_set_int(private, c"gpu".as_ptr(), i64::from(index), 0),
            "select NVENC GPU",
        )?;
        let mut selected = -1;
        check(
            ffi::av_opt_get_int(private, c"gpu".as_ptr(), 0, &mut selected),
            "verify NVENC GPU selection",
        )?;
        ensure!(
            selected == i64::from(index),
            "NVENC GPU selection was ignored"
        );
    }
    Ok(())
}
fn vaapi_devices(device: Option<&EncoderDevice>) -> Result<Vec<PathBuf>> {
    if let Some(device) = device {
        device.validate()?;
        let EncoderDevice::Vaapi(path) = device else {
            bail!("not a VA-API device");
        };
        return Ok(vec![path.clone()]);
    }
    let mut candidates: Vec<_> = std::fs::read_dir("/dev/dri")
        .context("read VA-API render nodes")?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .map(|e| e.path())
        .collect();
    candidates.sort();
    ensure!(!candidates.is_empty(), "no VA-API render nodes found");
    Ok(candidates)
}
fn try_devices<T>(
    candidates: Vec<PathBuf>,
    mut attempt: impl FnMut(&Path) -> Result<T>,
) -> Result<T> {
    let mut errors = Vec::new();
    for path in candidates {
        match attempt(&path) {
            Ok(encoder) => return Ok(encoder),
            Err(error) => errors.push(format!("{}: {error:#}", path.display())),
        }
    }
    bail!("no verified VA-API encoder: {}", errors.join("; "))
}
fn vaapi_frames(
    encoder: &mut encoder::video::Video,
    settings: EncoderSettings,
    path: &Path,
) -> Result<Buffer> {
    let name = CString::new(path.to_string_lossy().as_bytes())?;
    let mut device = ptr::null_mut();
    // SAFETY: generated FFmpeg ABI, owned buffer refs, no retained CString
    // pointer. The codec gets its own ref and releases it with its context.
    unsafe {
        check(
            ffi::av_hwdevice_ctx_create(
                &mut device,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                name.as_ptr(),
                ptr::null_mut(),
                0,
            ),
            "initialize VA-API device",
        )?;
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
        Ok(frames)
    }
}
pub fn probe(backend: EncoderBackend, device: Option<&EncoderDevice>) -> Result<String> {
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
    verify(
        &mut NativeEncoder::new(backend, settings, rate, device)?,
        settings,
        rate,
    )
}
fn verify(
    encoder: &mut NativeEncoder,
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
    #[cfg(target_os = "linux")]
    fn pinned_vaapi_failure_never_tries_a_different_adapter() {
        let selected =
            EncoderDevice::Vaapi(PathBuf::from("/dev/dri/by-path/pci-0000:03:00.0-render"));
        let candidates = vaapi_devices(Some(&selected)).unwrap();
        assert_eq!(
            candidates,
            [PathBuf::from("/dev/dri/by-path/pci-0000:03:00.0-render")]
        );
        let mut attempted = Vec::new();
        let result: Result<()> = try_devices(candidates, |path| {
            attempted.push(path.to_owned());
            bail!("selected adapter failed the complete frame probe")
        });
        assert!(result.is_err());
        assert_eq!(attempted.len(), 1);
        assert!(vaapi_devices(Some(&EncoderDevice::Nvenc(1))).is_err());
        assert!(vaapi_devices(Some(&EncoderDevice::Vaapi(PathBuf::from("renderD129")))).is_err());
    }

    #[test]
    fn nvenc_gpu_option_is_set_and_read_back_without_initializing_a_driver() {
        ffmpeg::init().unwrap();
        let Some(codec) = encoder::find_by_name("h264_nvenc") else {
            eprintln!("FFmpeg build has no NVENC encoder; device option test skipped");
            return;
        };
        let mut context = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .unwrap();
        for index in [0, 1, i32::MAX as u32] {
            select_nvenc_gpu(&mut context, index).unwrap();
            let mut actual = -1;
            unsafe {
                check(
                    ffi::av_opt_get_int(
                        (*context.as_mut_ptr()).priv_data,
                        c"gpu".as_ptr(),
                        0,
                        &mut actual,
                    ),
                    "read selected GPU",
                )
                .unwrap();
            }
            assert_eq!(actual, i64::from(index));
        }
        assert!(select_nvenc_gpu(&mut context, u32::MAX).is_err());
    }
    use ffmpeg::software::scaling;

    #[test]
    fn native_pixel_converter_initializes_geometry_and_formats_before_first_frame() {
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
        let encoder =
            NativeEncoder::open("libx264", EncoderBackend::OpenH264, settings, rate).unwrap();
        for (option, expected) in [
            (c"srcw", i64::from(settings.width)),
            (c"srch", i64::from(settings.height)),
            (c"dstw", i64::from(settings.width)),
            (c"dsth", i64::from(settings.height)),
            (c"src_format", ffi::AVPixelFormat::AV_PIX_FMT_RGBA as i64),
            (c"dst_format", ffi::AVPixelFormat::AV_PIX_FMT_NV12 as i64),
        ] {
            let mut configured = 0;
            unsafe {
                check(
                    ffi::av_opt_get_int(
                        encoder.scaler.0.cast(),
                        option.as_ptr(),
                        0,
                        &mut configured,
                    ),
                    "read pixel conversion option",
                )
                .unwrap();
            }
            assert_eq!(configured, expected, "{option:?}");
        }
    }

    #[test]
    fn vaapi_device_selection_continues_after_frame_or_encoding_failure() {
        let paths = ["renderD128", "renderD129", "renderD130"]
            .map(PathBuf::from)
            .to_vec();
        let mut attempted = Vec::new();
        let selected = try_devices(paths.clone(), |path| {
            attempted.push(path.to_owned());
            match path.to_str().unwrap() {
                "renderD128" => bail!("frame context initialization failed"),
                "renderD129" => bail!("frame context ready but H264 encode/decode failed"),
                _ => Ok(path.to_owned()),
            }
        })
        .unwrap();
        assert_eq!(attempted, paths);
        assert_eq!(selected, Path::new("renderD130"));
    }

    #[test]
    fn valid_avcc_extradata_and_length_prefixed_packet_convert_and_decode() {
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
        let mut encoder =
            NativeEncoder::open("libx264", EncoderBackend::OpenH264, settings, rate).unwrap();
        let pixels = [64, 128, 192, 255].repeat(640 * 360);
        let encoded = encoder.encode(&pixels, rate).unwrap();
        // Construct an actual AVCDecoderConfigurationRecord from the encoded
        // parameter sets. FFmpeg 6 libx264 annexb=0 extradata is raw NAL lengths,
        // while FFmpeg 9 packages avcC; neither fixture behavior is assumed.
        let starts: Vec<_> = encoded
            .data
            .windows(3)
            .enumerate()
            .filter(|(_, bytes)| *bytes == [0, 0, 1])
            .map(|(i, _)| {
                (
                    if i > 0 && encoded.data[i - 1] == 0 {
                        i - 1
                    } else {
                        i
                    },
                    i + 3,
                )
            })
            .collect();
        let nals: Vec<_> = starts
            .iter()
            .enumerate()
            .map(|(i, (_, start))| {
                &encoded.data[*start
                    ..starts
                        .get(i + 1)
                        .map_or(encoded.data.len(), |(offset, _)| *offset)]
            })
            .collect();
        let sps = nals.iter().find(|nal| nal[0] & 31 == 7).unwrap();
        let pps = nals.iter().find(|nal| nal[0] & 31 == 8).unwrap();
        let mut avcc = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(sps);
        avcc.push(1);
        avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(pps);
        let mut packet = Vec::new();
        for nal in nals.iter().filter(|nal| !matches!(nal[0] & 31, 7 | 8)) {
            packet.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            packet.extend_from_slice(nal);
        }
        let mut filter = unsafe {
            let mut context = ptr::null_mut();
            check(
                ffi::av_bsf_alloc(
                    ffi::av_bsf_get_by_name(c"h264_mp4toannexb".as_ptr()),
                    &mut context,
                ),
                "allocate fixture filter",
            )
            .unwrap();
            let filter = Bitstream(context);
            check(
                ffi::avcodec_parameters_from_context((*context).par_in, encoder.encoder.as_ptr()),
                "copy fixture parameters",
            )
            .unwrap();
            let parameters = &mut *(*context).par_in;
            assert!(parameters.extradata.is_null());
            parameters.extradata =
                ffi::av_mallocz(avcc.len() + ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize).cast();
            assert!(!parameters.extradata.is_null());
            ptr::copy_nonoverlapping(avcc.as_ptr(), parameters.extradata, avcc.len());
            parameters.extradata_size = avcc.len() as i32;
            check(ffi::av_bsf_init(context), "initialize valid avcC fixture").unwrap();
            filter
        };
        let converted = filter.convert(&mut Packet::copy(&packet)).unwrap();
        let (key, profile) = h264_packet_info(&converted);
        assert!(key && profile.is_some(), "filter must insert avcC SPS/PPS");
        let mut decoder = codec::context::Context::new_with_codec(
            ffmpeg::decoder::find(codec::Id::H264).unwrap(),
        )
        .decoder()
        .video()
        .unwrap();
        decoder.send_packet(&Packet::copy(&converted)).unwrap();
        let mut picture = frame::Video::empty();
        decoder.receive_frame(&mut picture).unwrap();
        assert_eq!((picture.width(), picture.height()), (640, 360));
    }

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
        let mut encoder =
            NativeEncoder::open("libx264", EncoderBackend::OpenH264, settings, rate).unwrap();
        assert!(
            verify(&mut encoder, settings, rate)
                .unwrap()
                .starts_with("avc1.")
        );
    }
}

#[cfg(test)]
mod reuse_tests {
    use super::*;
    #[test]
    fn cpu_frames_reuse_allocations_without_mutating_referenced_output() {
        ffmpeg::init().unwrap();
        let mut frames = CpuFrames::new(640, 360);
        let mut scaler = PixelConverter::new(640, 360).unwrap();
        let pixels = vec![80; 640 * 360 * 4];
        frames.convert(&pixels, &mut scaler).unwrap();
        #[cfg(not(all(target_os = "linux", feature = "fast-yuv")))]
        let rgba = frames.rgba.data(0).as_ptr();
        let output = frames.converted.data(0).as_ptr();
        frames.convert(&pixels, &mut scaler).unwrap();
        #[cfg(not(all(target_os = "linux", feature = "fast-yuv")))]
        assert_eq!(rgba, frames.rgba.data(0).as_ptr());
        assert_eq!(output, frames.converted.data(0).as_ptr());
        let retained = frames.converted.clone();
        let saved = retained.data(0).to_vec();
        frames
            .convert(&vec![200; pixels.len()], &mut scaler)
            .unwrap();
        assert_eq!(saved, retained.data(0));
        assert_ne!(saved, frames.converted.data(0));
        assert_ne!(retained.data(0).as_ptr(), frames.converted.data(0).as_ptr());
    }
}
