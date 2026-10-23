# Standalone resize experiment

This is a separate Cargo workspace so `fast_image_resize` is not an application runtime dependency. It compares `image 0.25.10` with `fast_image_resize 6.1.0`, using single-threaded, opaque RGBA8 synthetic pixels at 1280×720, 1920×1080 and 3840×2160.

```sh
cargo run --release --locked --manifest-path tools/resize-benchmark/Cargo.toml -- 50 > resize-results.json
```

Each case has five warmups and 50 measured samples. The output contains all sample times, P50/P95 and differences from the `image` result. `image_alloc` and `fir_auto_alloc` include destination allocation. `fir_auto_reuse` and `fir_scalar_reuse` retain their destination and working buffers; `fir_scalar_reuse` disables SIMD. Filter pairs are nearest, triangle/bilinear and Lanczos3. Alpha handling is disabled because all source alpha values are 255; no linear-light conversion is performed.

The two libraries do not promise identical convolution rounding or edge behavior. Output differences are included; this does not establish equal perceived quality. Current application streaming changes the desktop resolution and uses browser drawing rather than this CPU-resize path. These numbers therefore apply to a possible future encoder-input scaling stage, not current desktop FPS.
