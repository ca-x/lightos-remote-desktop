use fast_image_resize as fir;
use image::{
    RgbaImage,
    imageops::{self, FilterType},
};
use serde_json::json;
use std::{hint::black_box, time::Instant};

fn p(times: &[f64], percentile: usize) -> f64 {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[(sorted.len() * percentile).div_ceil(100) - 1]
}
fn source(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        // Opaque desktop-like edges / fine lines plus deterministic texture.
        let texture = (x.wrapping_mul(1664525) ^ y.wrapping_mul(1013904223)) as u8;
        let line = if x % 80 < 2 || y % 24 < 2 { 255 } else { 32 };
        image::Rgba([line, texture, (x / 8 + y / 8) as u8, 255])
    })
}
fn main() {
    let count = std::env::args()
        .nth(1)
        .map(|v| v.parse::<usize>().unwrap())
        .unwrap_or(50);
    assert!((10..=500).contains(&count));
    let mut rows = vec![];
    for (width, height, target_width, target_height) in [
        (1280, 720, 640, 360),
        (1920, 1080, 1280, 720),
        (3840, 2160, 1920, 1080),
    ] {
        let input = source(width, height);
        let source = fir::images::Image::from_vec_u8(
            width,
            height,
            input.as_raw().clone(),
            fir::PixelType::U8x4,
        )
        .unwrap();
        for (filter, image_filter, fir_filter) in [
            ("nearest", FilterType::Nearest, fir::ResizeAlg::Nearest),
            (
                "bilinear",
                FilterType::Triangle,
                fir::ResizeAlg::Convolution(fir::FilterType::Bilinear),
            ),
            (
                "lanczos3",
                FilterType::Lanczos3,
                fir::ResizeAlg::Convolution(fir::FilterType::Lanczos3),
            ),
        ] {
            let options = fir::ResizeOptions::new()
                .resize_alg(fir_filter)
                .use_alpha(false);
            let reference =
                imageops::resize(&input, target_width, target_height, image_filter).into_raw();
            for library in [
                "image_alloc",
                "fir_auto_alloc",
                "fir_auto_reuse",
                "fir_scalar_reuse",
            ] {
                let mut resizer = fir::Resizer::new();
                if library == "fir_scalar_reuse" {
                    // The scalar implementation is supported on every target.
                    unsafe {
                        resizer.set_cpu_extensions(fir::CpuExtensions::None);
                    }
                }
                let mut destination =
                    fir::images::Image::new(target_width, target_height, fir::PixelType::U8x4);
                let mut times = vec![];
                let mut output = vec![];
                for index in 0..(count + 5) {
                    let started = Instant::now();
                    if library == "image_alloc" {
                        let resized = imageops::resize(
                            black_box(&input),
                            target_width,
                            target_height,
                            image_filter,
                        );
                        black_box(&resized);
                        if index >= 5 {
                            times.push(started.elapsed().as_secs_f64() * 1000.0);
                        }
                        output = resized.into_raw();
                    } else if library == "fir_auto_alloc" {
                        let mut destination = fir::images::Image::new(
                            target_width,
                            target_height,
                            fir::PixelType::U8x4,
                        );
                        resizer
                            .resize(black_box(&source), &mut destination, &options)
                            .unwrap();
                        black_box(destination.buffer());
                        if index >= 5 {
                            times.push(started.elapsed().as_secs_f64() * 1000.0);
                        }
                        output = destination.into_vec();
                    } else {
                        resizer
                            .resize(black_box(&source), &mut destination, &options)
                            .unwrap();
                        black_box(destination.buffer());
                        if index >= 5 {
                            times.push(started.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                }
                if output.is_empty() {
                    output = destination.into_vec();
                }
                assert_eq!(output.len(), reference.len());
                assert!(output.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
                let mse = output
                    .iter()
                    .zip(&reference)
                    .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                    .sum::<f64>()
                    / output.len() as f64;
                let max_diff = output
                    .iter()
                    .zip(&reference)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                rows.push(json!({"source":[width,height],"target":[target_width,target_height],"filter":filter,"library":library,
                    "p50_ms":p(&times,50),"p95_ms":p(&times,95),"samples_ms":times,"mse_vs_image":mse,"max_abs_diff_vs_image":max_diff}));
            }
        }
    }
    println!("{}",serde_json::to_string_pretty(&json!({"samples_per_case":count,"warmup_per_case":5,"fir_cpu_extensions":format!("{:?}",fir::CpuExtensions::default()),
        "limits":["Standalone pixel resize microbenchmark, not current application FPS.","Opaque RGBA8 synthetic image, gamma-encoded channels, alpha handling disabled; no linear-light conversion.","image_alloc and fir_auto_alloc include output allocation; reuse variants retain their destination and working buffers.","Single-threaded; no frame capture, JPEG/H264 encoding, network or browser cost."],"rows":rows})).unwrap());
}
