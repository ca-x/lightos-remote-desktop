# Encoder-input scaling experiment plan

**Goal:** Continue the resize microbenchmark by measuring scale + RGBA/YUV conversion + software encoding, compressed bytes, and decoded text/detail at a fixed desktop resolution.

**Scope:** User approved continued testing on `experiment/binary-wire`. Keep HTTP, production codec behavior, installed binaries and desktop services unchanged. Use the standalone `tools/resize-benchmark` workspace; no application runtime dependency on FIR.

**Architecture:** Render a deterministic local HTML desktop fixture at 1920×1080 and 3840×2160. Feed identical static and vertically scrolling source frames into native-size, `image` triangle, FIR bilinear, and FIR Lanczos3 paths. Invoke production TurboJPEG and the same OpenH264 source/version/configuration as the production encoder, with deterministic 30 FPS timestamps. A direct synchronous OpenH264 benchmark is necessary because the production worker takes wall-clock timestamps, which would confound methods of different speed. Report that worker scheduling/IPC is excluded. Decode all produced frames and validate output dimensions.

**Stack:** `fast_image_resize 6.1.0`, `image 0.25.10`, repository `media-codec` for JPEG, `openh264 0.9.9`, headless Chromium fixtures; FFmpeg official PSNR/SSIM filters for offline quality measurements.

## Acceptance and review focus

- Keep source pixels and motion identical; fixed source resolution with target 1280×720 / 1920×1080.
- JPEG quality 75 with production 4:2:0; H.264 production profile, 30 FPS, two encoder threads and frame skipping. Compare both resolution-dependent default bitrate and fixed 3 Mbit/s to distinguish resolution savings from bitrate policy.
- Record scaling and conversion/encode wall times separately and together, P50/P95, raw samples, compressed bytes and skipped/decoded frame counts. Never call inverse processing time actual browser FPS.
- Use at least three measured repeats with rotated method order; warm up each configuration. Initialization and frame generation are outside steady-state timing. Include destination allocation for `image`; FIR retains destination/workspace. State ownership transfer to an async worker is outside this synchronous measurement.
- No-scale baseline has zero CPU resize. Downscaled results inherently sacrifice detail: compare final decoded crops enlarged to native display size, and compression quality against each method's uncompressed target. PSNR/SSIM do not prove text readability.
- Handle OpenH264 skipped inputs explicitly; validate every nonempty frame decodes with expected dimensions and record last displayed input index. Record actual library/tool versions and fixture hashes.

## Tasks

- [x] Add repeatable desktop fixtures and standalone pipeline harness; run a short pilot and validate timing/sample accounting, decode dimensions, bitrate settings and artifact generation.
- [x] Run the bounded formal matrix (60 frames × 3 repeats per case) once; inspect skipped frames and decoded text/fine-line artifacts.
- [x] Analyze compressed payload and full processing results alongside the original microbenchmark. Write a report and raw JSON under `docs/benchmarks` with limitations and reproduction commands.
- [x] Run formatting / focused harness checks, review the diff, commit and push the experiment branch. Do not merge or release.

Pilot evidence: 48 measured configurations × 5 inputs and corresponding 10-frame warmups completed; every nonempty JPEG/H.264 access unit decoded with the expected dimensions. Offline accounting and 48 PSNR/SSIM crop comparisons passed. Fixture PNG SHA256 values match the committed manifest. Release build, standalone all-target Clippy (`-D warnings`), format, JS syntax and Python syntax checks passed. Fixed the pointer trajectory to remain inside both fixtures for the full supported frame count. The new compiler's `chunks_exact` lint in the original experiment was resolved with the equivalent `as_chunks` API.

Formal matrix completed at source `0f9a76e`: 144 measured runs, 144 × 10-frame warmups, 8,640 requested / 8,601 decoded / 39 rate-control skipped measured inputs. All accounting checks and 48 offline quality comparisons passed.

Additional control: 4K/3 px-per-input H.264 scaled output unexpectedly used more bytes. Add a bounded 4 px-per-input alignment control (three methods × three repeats, 60 inputs), rather than inferring universal bandwidth savings. Default experiment pixels/configuration stay unchanged. Set only the standalone tool's declared Rust floor to 1.89 because its new repository-matched OpenH264 transitive dependencies require that version; application manifests remain unchanged.

Final evidence: aligned control at `3cc7e72` completed 9 measured runs and their warmups, with 540/540 outputs decoded and zero skips; three additional quality comparisons passed. Native/FIR-bilinear bytes changed from 508,605/536,511 at 3 px scroll to 429,605/131,784 at 4 px scroll, so the report records content/motion sensitivity rather than universal network savings. Checked both contact sheets and final payload/latency/quality tables against raw JSON. Reports and raw datasets are under `docs/benchmarks/2026-10-23-encoder-scaling*`; keep the experiment isolated from main and installed services.
