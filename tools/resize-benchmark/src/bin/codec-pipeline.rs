//! Synchronous, deterministic scale + convert + encode experiment.
//! OpenH264 configuration mirrors media-codec/src/openh264.rs; fixed timestamps
//! keep scaler speed from changing rate-control input. No production worker IPC.
use anyhow::{Context, Result, ensure};
use fast_image_resize as fir;
use image::{
    Rgba, RgbaImage,
    imageops::{self, FilterType},
};
use openh264::{
    OpenH264API, Timestamp,
    decoder::Decoder,
    encoder::{
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
        Profile, RateControlMode, UsageType,
    },
    formats::{RgbaSliceU8, YUVBuffer, YUVSource},
};
use serde_json::{Value, json};
use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

const METHODS: [&str; 4] = ["native", "image_triangle", "fir_bilinear", "fir_lanczos3"];
const PROFILES: [&str; 3] = ["jpeg_q75", "h264_default", "h264_3mbps"];
const FPS: u32 = 30;

fn summary(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({"p50_ms":sorted[(sorted.len()*50).div_ceil(100)-1],
        "p95_ms":sorted[(sorted.len()*95).div_ceil(100)-1],"samples_ms":values})
}

struct Scaler {
    method: &'static str,
    width: u32,
    height: u32,
    resizer: fir::Resizer,
    destination: fir::images::Image<'static>,
    allocated: RgbaImage,
}
impl Scaler {
    fn new(method: &'static str, width: u32, height: u32) -> Self {
        Self {
            method,
            width,
            height,
            resizer: fir::Resizer::new(),
            destination: fir::images::Image::new(width, height, fir::PixelType::U8x4),
            allocated: RgbaImage::new(0, 0),
        }
    }
    fn resize<'a>(&'a mut self, source: &'a RgbaImage) -> Result<&'a [u8]> {
        match self.method {
            "native" => Ok(source.as_raw()),
            "image_triangle" => {
                self.allocated =
                    imageops::resize(source, self.width, self.height, FilterType::Triangle);
                Ok(self.allocated.as_raw())
            }
            _ => {
                let view = fir::images::ImageRef::new(
                    source.width(),
                    source.height(),
                    source.as_raw(),
                    fir::PixelType::U8x4,
                )?;
                let filter = if self.method == "fir_bilinear" {
                    fir::FilterType::Bilinear
                } else {
                    fir::FilterType::Lanczos3
                };
                self.resizer.resize(
                    &view,
                    &mut self.destination,
                    &fir::ResizeOptions::new()
                        .resize_alg(fir::ResizeAlg::Convolution(filter))
                        .use_alpha(false),
                )?;
                Ok(self.destination.buffer())
            }
        }
    }
}

fn h264_encoder(bitrate: u32) -> Result<Encoder> {
    // Same source library and configuration as production, at this commit.
    Ok(Encoder::with_api_config(
        OpenH264API::from_source(),
        EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .profile(Profile::Baseline)
            .complexity(Complexity::Low)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(bitrate))
            .max_frame_rate(FrameRate::from_hz(FPS as f32))
            .skip_frames(true)
            .scene_change_detect(true)
            .adaptive_quantization(false)
            .background_detection(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(120))
            .num_threads(2),
    )?)
}

fn render(base: &RgbaImage, output: &mut RgbaImage, scene: &str, index: usize) {
    output.as_mut().copy_from_slice(base.as_raw());
    if scene == "scroll" {
        // Scroll content below the title; preserve desktop and panel geometry.
        let left = 240usize;
        let right = (base.width() - 30) as usize;
        let top = 190usize;
        let bottom = (base.height() - 70) as usize;
        let stride = base.width() as usize * 4;
        let shift = index * 3 % (bottom - top);
        for y in top..bottom {
            let from = top + (y - top + shift) % (bottom - top);
            output.as_mut()[y * stride + left * 4..y * stride + right * 4].copy_from_slice(
                &base.as_raw()[from * stride + left * 4..from * stride + right * 4],
            );
        }
        for y in 100..112 {
            let pointer_x = 280 + index as u32 * 7 % (base.width() - 310);
            for x in pointer_x..pointer_x + 12 {
                output.put_pixel(x, y, Rgba([20, 125, 180, 255]));
            }
        }
    }
}

fn crop(image: &RgbaImage) -> RgbaImage {
    imageops::crop_imm(image, 250, 190, 640, 220).to_image()
}

#[allow(clippy::too_many_arguments)]
fn run(
    base: &RgbaImage,
    target: (u32, u32),
    method: &'static str,
    profile: &str,
    scene: &str,
    frames: usize,
    artifact: Option<&Path>,
) -> Result<Value> {
    let (width, height) = if method == "native" {
        (base.width(), base.height())
    } else {
        target
    };
    let bitrate = if profile == "h264_3mbps" {
        3_000_000
    } else {
        media_codec::default_bitrate(width, height)
    };
    let h264 = profile.starts_with("h264");
    let mut encoder = if h264 {
        Some(h264_encoder(bitrate)?)
    } else {
        None
    };
    let mut yuv = YUVBuffer::new(width as usize, height as usize);
    let mut scaler = Scaler::new(method, width, height);
    let mut frame = base.clone();
    let mut scale_ms = vec![];
    let mut convert_ms = vec![];
    let mut encode_ms = vec![];
    let mut pipeline_ms = vec![];
    let mut packets = vec![];
    let mut skipped = 0;
    let mut keyframes = 0;
    for index in 0..frames {
        render(base, &mut frame, scene, index);
        let start = Instant::now();
        let rgba = black_box(scaler.resize(&frame)?);
        let scaled = Instant::now();
        let converted = if h264 {
            yuv.read_rgba8(RgbaSliceU8::new(rgba, (width as usize, height as usize)));
            Instant::now()
        } else {
            scaled
        };
        let data = if let Some(encoder) = encoder.as_mut() {
            let stream = encoder.encode_at(
                &yuv,
                Timestamp::from_millis(index as u64 * 1000 / FPS as u64),
            )?;
            let is_skip = stream.frame_type() == FrameType::Skip;
            let data = stream.to_vec();
            ensure!(
                is_skip == data.is_empty(),
                "unexpected empty H264 access unit"
            );
            if is_skip {
                skipped += 1;
            } else if media_codec::h264_packet_info(&data).0 {
                keyframes += 1;
            }
            data
        } else {
            keyframes += 1;
            media_codec::encode_jpeg(width, height, rgba, 75)?.to_vec()
        };
        let done = Instant::now();
        scale_ms.push((scaled - start).as_secs_f64() * 1000.0);
        convert_ms.push((converted - scaled).as_secs_f64() * 1000.0);
        encode_ms.push((done - converted).as_secs_f64() * 1000.0);
        pipeline_ms.push((done - start).as_secs_f64() * 1000.0);
        packets.push(data);
    }
    // Decode every nonempty access unit outside the timed region.
    let mut decoder = if h264 { Some(Decoder::new()?) } else { None };
    let mut jpeg_decoder = turbojpeg::Decompressor::new()?;
    let mut decoded_frames = 0;
    let mut last_index = 0;
    let mut last_decoded = vec![];
    for (index, packet) in packets.iter().enumerate() {
        if packet.is_empty() {
            continue;
        }
        last_decoded = if let Some(decoder) = decoder.as_mut() {
            let decoded = decoder
                .decode(packet)?
                .context("H264 nonempty access unit produced no frame")?;
            ensure!(
                decoded.dimensions() == (width as usize, height as usize),
                "H264 output dimensions"
            );
            let mut rgba = vec![0; width as usize * height as usize * 4];
            decoded.write_rgba8(&mut rgba);
            rgba
        } else {
            let header = jpeg_decoder.read_header(packet)?;
            ensure!(
                (header.width, header.height) == (width as usize, height as usize),
                "JPEG output dimensions"
            );
            let mut pixels = vec![0; width as usize * height as usize * 4];
            jpeg_decoder.decompress(
                packet,
                turbojpeg::Image {
                    pixels: pixels.as_mut_slice(),
                    width: width as usize,
                    height: height as usize,
                    pitch: width as usize * 4,
                    format: turbojpeg::PixelFormat::RGBA,
                },
            )?;
            pixels
        };
        decoded_frames += 1;
        last_index = index;
    }
    ensure!(
        decoded_frames == frames - skipped && decoded_frames > 0,
        "decoded/encoded frame accounting"
    );
    if let Some(directory) = artifact {
        fs::create_dir_all(directory)?;
        // Include stale output, if rate control skipped the final current input.
        render(base, &mut frame, scene, frames - 1);
        crop(&frame).save(directory.join("desktop-reference.png"))?;
        // Compression-only reference matches the last actually decoded input.
        render(base, &mut frame, scene, last_index);
        let reference = scaler.resize(&frame)?.to_vec();
        let reference =
            RgbaImage::from_raw(width, height, reference).context("RGBA reference dimensions")?;
        let decoded =
            RgbaImage::from_raw(width, height, last_decoded).context("RGBA decoded dimensions")?;
        let (x, y, w, h) = (
            250 * width / base.width(),
            190 * height / base.height(),
            640 * width / base.width(),
            220 * height / base.height(),
        );
        imageops::crop_imm(&reference, x, y, w, h)
            .to_image()
            .save(directory.join("compression-reference.png"))?;
        imageops::crop_imm(&decoded, x, y, w, h)
            .to_image()
            .save(directory.join("compression-decoded.png"))?;
        // Approximate a browser enlarging the stream to native desktop size.
        let displayed = if method == "native" {
            decoded
        } else {
            imageops::resize(&decoded, base.width(), base.height(), FilterType::Triangle)
        };
        crop(&displayed).save(directory.join("desktop-displayed.png"))?;
    }
    let bytes: usize = packets.iter().map(Vec::len).sum();
    let mut displayed_index = 0;
    let display_ages: Vec<_> = packets
        .iter()
        .enumerate()
        .map(|(index, data)| {
            if !data.is_empty() {
                displayed_index = index;
            }
            index - displayed_index
        })
        .collect();
    Ok(
        json!({"source":[base.width(),base.height()],"target":[width,height],"method":method,
        "profile":profile,"scene":scene,"requested_frames":frames,"encoded_frames":decoded_frames,
        "skipped_frames":skipped,"decoded_frames":decoded_frames,"keyframes":keyframes,
        "last_decoded_input_index":last_index,"bitrate_bps":if h264 {json!(bitrate)} else {Value::Null},
        "displayed_input_age_frames":display_ages,
        "encoded_fps_at_requested_30fps":decoded_frames as f64*FPS as f64/frames as f64,
        "payload_bytes":bytes,"payload_mbps_at_requested_30fps":bytes as f64*8.0/(frames as f64/FPS as f64)/1e6,
        "scale":summary(&scale_ms),"convert":summary(&convert_ms),"encode":summary(&encode_ms),
        "pipeline":summary(&pipeline_ms),"packet_bytes":packets.iter().map(Vec::len).collect::<Vec<_>>(),
        "artifacts":artifact.map(|p| p.to_string_lossy().into_owned())}),
    )
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let fixtures = PathBuf::from(
        args.next()
            .context("usage: codec-pipeline <fixtures> <output> [frames=60] [repeats=3]")?,
    );
    let output = PathBuf::from(args.next().context("output directory missing")?);
    let frames = args
        .next()
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(60usize);
    let repeats = args
        .next()
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(3usize);
    ensure!(
        (5..=300).contains(&frames) && (1..=10).contains(&repeats),
        "invalid sample count"
    );
    fs::create_dir_all(&output)?;
    let command = |program: &str, arguments: &[&str]| -> Result<String> {
        let value = std::process::Command::new(program)
            .args(arguments)
            .output()?;
        ensure!(value.status.success(), "metadata command failed: {program}");
        Ok(String::from_utf8(value.stdout)?.trim().to_string())
    };
    let metadata = json!({
        "source": command("git", &["rev-parse", "HEAD"])?,
        "source_status": command("git", &["status", "--porcelain"])?,
        "started_at": command("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"])?,
        "rustc": command("rustc", &["--version"])?,
        "platform": command("uname", &["-srmo"])?,
        "cpu": fs::read_to_string("/proc/cpuinfo")?.lines().find(|l| l.starts_with("model name")).unwrap_or("unknown CPU"),
        "libturbojpeg": command("pkg-config", &["--modversion", "libturbojpeg"])?,
        "versions": {"fast_image_resize":"6.1.0", "image":"0.25.10", "openh264":"0.9.9", "turbojpeg":"1.5.1"},
        "timing": "Synchronous scale + RGBA/I420 conversion + encode, including compressed output copy. FIR retains destination/workspace, image allocates. Frame generation, encoder creation, output decode, async ownership transfer/worker scheduling, capture, network and browser are excluded.",
        "h264": "production ScreenContentRealTime/Baseline/Low/Bitrate, skip=true, scene_change=true, AQ=false, background=false, GOP120, 2 threads; deterministic index*1000/30 ms timestamps",
        "jpeg": "production encode_jpeg, quality 75, 4:2:0",
        "static_limit": "Repeated static input for CPU comparison; production suppresses unchanged capture frames, so this is not idle-desktop bandwidth.",
        "msrv_limit": "The current compiler is recorded above. Repository-pinned OpenH264 transitive wide/safe_arch require 1.89; declared Rust 1.88 is not validated by this experiment."
    });
    let mut rows = vec![];
    let mut warmups = 0;
    for (width, height, target) in [(1920, 1080, (1280, 720)), (3840, 2160, (1920, 1080))] {
        let base =
            image::open(fixtures.join(format!("desktop-{width}x{height}.png")))?.into_rgba8();
        ensure!(base.dimensions() == (width, height), "fixture dimensions");
        ensure!(
            base.pixels().all(|p| p[3] == 255),
            "FIR experiment requires opaque source pixels"
        );
        for scene in ["static", "scroll"] {
            for profile in PROFILES {
                for repeat in 0..repeats {
                    for offset in 0..METHODS.len() {
                        let method = METHODS[(offset + repeat) % METHODS.len()];
                        run(&base, target, method, profile, scene, 10, None)?;
                        warmups += 1;
                        let artifact = (repeat == 0).then(|| {
                            output
                                .join("quality")
                                .join(format!("{width}-{scene}-{profile}-{method}"))
                        });
                        let mut row = run(
                            &base,
                            target,
                            method,
                            profile,
                            scene,
                            frames,
                            artifact.as_deref(),
                        )?;
                        row["repeat"] = json!(repeat);
                        eprintln!(
                            "{width}x{height} {scene} {profile} {method} repeat={repeat} pipeline={:.3}ms bytes={} skip={}",
                            row["pipeline"]["p50_ms"].as_f64().unwrap(),
                            row["payload_bytes"],
                            row["skipped_frames"]
                        );
                        rows.push(row);
                        fs::write(
                            output.join("results.json"),
                            serde_json::to_string_pretty(&json!({
                            "metadata":metadata,"warmups":warmups,"warmup_frames_per_run":10,"frames_per_run":frames,"repeats":repeats,
                            "fps":FPS,"fir_cpu_extensions":format!("{:?}",fir::CpuExtensions::default()),
                            "fixture_manifest":serde_json::from_str::<Value>(&fs::read_to_string(fixtures.join("manifest.json"))?)?,
                            "rows":rows}))?,
                        )?;
                    }
                }
            }
        }
    }
    println!("{}", output.join("results.json").display());
    Ok(())
}
