//! Fixed-input microbenchmark of the old worker request and the compatible byte path.
use media_codec::{
    EncoderBackend, EncoderRate, EncoderSettings,
    worker_protocol::{self as wire, RequestRef},
};
use serde::Serialize;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[derive(Serialize)]
#[allow(dead_code)] // Preserve the real v1 enum discriminants in the baseline.
enum LegacyRequest {
    Probe {
        backend: EncoderBackend,
    },
    Init {
        backend: EncoderBackend,
        settings: EncoderSettings,
    },
    Frame {
        pixels: Vec<u8>,
        rate: EncoderRate,
    },
}
fn measure(mut run: impl FnMut(), count: usize) -> Duration {
    run();
    let start = Instant::now();
    for _ in 0..count {
        run();
    }
    start.elapsed() / count as u32
}
fn main() {
    let rate = EncoderRate {
        bitrate_bps: 8_000_000,
        fps: 30,
    };
    println!("width,height,raw_bytes,wire_bytes,legacy_us,borrowed_bytes_us");
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        let pixels: Vec<u8> = (0..width * height * 4)
            .map(|n| (n * 31 % 256) as u8)
            .collect();
        let before = wire::pack(&LegacyRequest::Frame {
            pixels: pixels.clone(),
            rate,
        })
        .unwrap();
        let after = wire::pack(&RequestRef::Frame {
            pixels: &pixels,
            rate,
        })
        .unwrap();
        assert_eq!(before, after);
        let old = measure(
            || {
                black_box(
                    wire::pack(&LegacyRequest::Frame {
                        pixels: black_box(&pixels).to_vec(),
                        rate,
                    })
                    .unwrap(),
                );
            },
            20,
        );
        let new = measure(
            || {
                black_box(
                    wire::pack(&RequestRef::Frame {
                        pixels: black_box(&pixels),
                        rate,
                    })
                    .unwrap(),
                );
            },
            20,
        );
        println!(
            "{width},{height},{},{},{},{}",
            pixels.len(),
            after.len(),
            old.as_micros(),
            new.as_micros()
        );
    }
}
