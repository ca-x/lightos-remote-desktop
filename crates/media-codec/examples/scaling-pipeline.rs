//! Small production-API timing check; capture/network/browser time is excluded.
//! cargo run --release -p media-codec --example scaling-pipeline -- desktop.png
use anyhow::{Context, Result};
use media_codec::{H264Encoder, RgbaScaler, encode_jpeg, fit_dimensions};
use std::{sync::Arc, time::Instant};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("provide a desktop PNG/JPEG fixture")?;
    let image = image::open(path)?.to_rgba8();
    let source = image.dimensions();
    let target = fit_dimensions(source, Some((source.0 * 2 / 3, source.1 * 2 / 3)));
    println!("source,output,codec,round,p50_ms,p95_ms,encoded,skipped,bytes");
    for round in 0..3 {
        for codec in ["jpeg444", "h264"] {
            // Alternate order to reduce warmup/temperature ordering bias.
            for scaled in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let output = if scaled { target } else { source };
                let mut scaler = RgbaScaler::default();
                let mut encoder = if codec == "h264" {
                    Some(H264Encoder::new(output.0, output.1, 30)?)
                } else {
                    None
                };
                let mut samples = Vec::new();
                let (mut encoded, mut skipped, mut bytes) = (0, 0, 0);
                for frame in 0..48 {
                    // Three source rows become two output rows at 2/3 scale.
                    // Make a fresh changing capture before starting the timer.
                    let offset = ((frame * 3) % source.1 as usize) * source.0 as usize * 4;
                    let raw = image.as_raw();
                    let pixels: Arc<[u8]> = raw[offset..]
                        .iter()
                        .chain(&raw[..offset])
                        .copied()
                        .collect::<Vec<_>>()
                        .into();
                    let started = Instant::now();
                    let pixels = if scaled {
                        let (returned, pixels) = tokio::task::spawn_blocking(move || {
                            let pixels = scaler.resize(pixels, source, output);
                            (scaler, pixels)
                        })
                        .await?;
                        scaler = returned;
                        pixels?
                    } else {
                        pixels
                    };
                    let length = if let Some(encoder) = encoder.as_mut() {
                        encoder.encode_shared(pixels).await?.data.len()
                    } else {
                        tokio::task::spawn_blocking(move || {
                            encode_jpeg(output.0, output.1, &pixels, 75)
                        })
                        .await??
                        .len()
                    };
                    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                    if frame >= 8 {
                        samples.push(elapsed);
                        bytes += length;
                        if length == 0 {
                            skipped += 1;
                        } else {
                            encoded += 1;
                        }
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!(
                    "{}x{},{}x{},{codec},{round},{:.3},{:.3},{encoded},{skipped},{bytes}",
                    source.0,
                    source.1,
                    output.0,
                    output.1,
                    samples[samples.len() / 2],
                    samples[samples.len() * 95 / 100]
                );
            }
        }
    }
    Ok(())
}
