# Standalone resize experiment

This is a separate Cargo workspace so `fast_image_resize` is not an application runtime dependency. It compares `image 0.25.10` with `fast_image_resize 6.1.0`, using single-threaded, opaque RGBA8 synthetic pixels at 1280×720, 1920×1080 and 3840×2160.

```sh
cargo run --release --locked --manifest-path tools/resize-benchmark/Cargo.toml -- 50 > resize-results.json
```

Each case has five warmups and 50 measured samples. The output contains all sample times, P50/P95 and differences from the `image` result. `image_alloc` and `fir_auto_alloc` include destination allocation. `fir_auto_reuse` and `fir_scalar_reuse` retain their destination and working buffers; `fir_scalar_reuse` disables SIMD. Filter pairs are nearest, triangle/bilinear and Lanczos3. Alpha handling is disabled because all source alpha values are 255; no linear-light conversion is performed.

The two libraries do not promise identical convolution rounding or edge behavior. Output differences are included; this does not establish equal perceived quality. Current application streaming changes the desktop resolution and uses browser drawing rather than this CPU-resize path. These numbers therefore apply to a possible future encoder-input scaling stage, not current desktop FPS.

## Encoder-input pipeline experiment

The `codec-pipeline` binary keeps desktop pixels at native size and compares no resize, `image` Triangle, FIR bilinear and FIR Lanczos3 before JPEG/H.264 encoding. Its fixtures are screenshots of the included local HTML, with no desktop capture or user content. Required benchmark tools are Rust, `pkg-config`/libturbojpeg and FFmpeg. Exact PNG inputs and their hashes are included in `fixtures/`. Regenerating them from HTML additionally requires an existing `agent-browser`/Chromium, Node and `fc-match`.

From the repository root:

```sh
cargo build --release --locked --manifest-path tools/resize-benchmark/Cargo.toml --bin codec-pipeline
tools/resize-benchmark/target/release/codec-pipeline tools/resize-benchmark/fixtures .local/encoder-scaling/formal 60 3
python3 tools/resize-benchmark/analyze-pipeline.py .local/encoder-scaling/formal
```

Regenerate the PNGs if needed with `node tools/resize-benchmark/prepare-fixtures.mjs .local/encoder-scaling/fixtures /path/to/agent-browser`. A pilot can use `5 1` in place of `60 3`, in a separate output directory. Fixture regeneration uses installed fonts, with font description, browser version and PNG hashes in `manifest.json`; retain that manifest and the source PNGs to reproduce exact input pixels. Use the included fixtures rather than a private desktop screenshot.

The formal matrix contains two resolutions, static/scrolling input, four scaling methods, JPEG quality 75 and two H.264 bitrate profiles, each repeated three times. Each measured sequence starts a fresh encoder and follows a 10-frame warmup with the same configuration. Method order rotates by repeat. OpenH264 uses the same source version and configuration as production, with fixed 30 FPS timestamps; its synchronous invocation excludes worker scheduling and ownership transfer. Production JPEG is called directly. Initialization is outside steady-state timings, but the first measured keyframe is included. All nonempty encoded frames are actually decoded and checked for dimensions; skipped inputs and display age are recorded.

`results.json` contains all timings, payload lengths and first-repeat quality data; `aggregates.json` reports medians of per-run P50/P95. The compressed-byte rate assumes 30 requested inputs per second and is not network throughput. Continuous static input measures CPU cost; the application normally suppresses unchanged capture frames. Reused FIR buffers and the allocating `image` API have different ownership/allocation costs; the earlier microbenchmark includes allocation-matched variants.

`quality.html` displays native-size text crops. The compression-only comparison uses the uncompressed scaler result at encoder resolution; the desktop comparison enlarges decoded output using `image` Triangle as a common display approximation. FFmpeg PSNR/SSIM are computed on these text crops in YUV444, not the full desktop, and are not a test of perceived legibility or actual GPU rendering. No-scale and downscaled images contain different amounts of information.

This tool uses the repository's current OpenH264 dependencies, including `wide`/`safe_arch` versions requiring Rust 1.89. The standalone tool therefore declares Rust 1.89. It is measured using Rust 1.99.1; it does not establish that the application's declared Rust 1.88 lower bound works. The FIR dependency itself declares Rust 1.87.

The formal 3 px/input scroll becomes a fractional 1.5 px/input displacement at half resolution. To isolate alignment effects, the optional `aligned-scroll-control` mode tests only 4K scrolling, default-bitrate H.264, and native/FIR bilinear/FIR Lanczos3 with 4 px/input (2 px/input at half resolution):

```sh
tools/resize-benchmark/target/release/codec-pipeline tools/resize-benchmark/fixtures .local/encoder-scaling/aligned 60 3 aligned-scroll-control
python3 tools/resize-benchmark/analyze-pipeline.py .local/encoder-scaling/aligned
```

This control changes motion and is not interchangeable with the formal workload. Compare each method to its own native baseline. A reduction here cannot be generalized to arbitrary motion, content or network conditions.
