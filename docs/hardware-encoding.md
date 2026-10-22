# Optional native H.264 encoding

The portable service keeps its built-in OpenH264 and TurboJPEG encoders. The
optional sibling executable `lightos-codec-ffmpeg` supplies VA-API, NVENC, AMF and
VideoToolbox through native `ffmpeg-next` bindings. No FFmpeg command-line codec
pipeline is launched. Browser hardware decoding is independent of server encoding.

The current capture/input service remains Linux Wayland. Encoder adapters for
other operating systems do not implement Windows or macOS remote desktop sessions.

## Build and runtime dependencies

The core executable does not link FFmpeg. A missing, incompatible or broken
worker leaves software encoding available. The worker is dynamically linked to
the FFmpeg libraries installed at build time, including libavcodec, libavformat,
libavutil and libswscale, and requires their matching ABI at runtime. GPU driver
libraries are also required. `clang`/`libclang`, headers and `pkg-config` are build
dependencies, not runtime requirements. This helper is optional, not a universal
single-binary hardware encoder.

On Ubuntu, install `pkg-config clang libclang-dev libavcodec-dev libavformat-dev
libavutil-dev libswscale-dev cmake nasm` for native builds; use the repository's
bundled JPEG feature. On Arch/Omarchy, use the installed `ffmpeg`, `clang`,
`pkgconf`, `cmake` and `nasm` development environment. Drivers and render-node
permissions must already permit the service user to encode. The installer does
not install drivers or modify device permissions or user groups.

```sh
cargo build --release --locked --features bundled-jpeg
cargo build --release --locked -p media-codec-ffmpeg --features native,media-codec/bundled-jpeg
sudo ./target/release/lightos-remote-desktop install-service
```

The installer detects the optional helper beside the supplied core binary and
copies both into the same immutable release. Its identity covers both binaries;
rollback restores that release as a unit. A software-only install retains the
original binary-hash identity and does not carry forward an older helper.

CI builds and tests the optional helper on Ubuntu 24.04 with native FFmpeg
headers, Clang/libclang and the distribution's FFmpeg libraries. Its separately
named archive records `ldd` and FFmpeg versions. Use it only with matching ABI;
build locally on Arch/Omarchy. The existing portable core archive remains
separate and requires no new FFmpeg libraries. No FFmpeg shared libraries or GPU
drivers are bundled in either archive.

## Selection and failure behavior

Format (Auto/H.264/JPEG), H.264 backend (Auto/Software/VA-API/NVENC/AMF/VideoToolbox)
and streaming policy are separate saved settings. Auto tries successfully probed
hardware in NVENC, VA-API, AMF, VideoToolbox order, then OpenH264. Each backend
probe initializes a real encoder, encodes eight changing frames across two rate
configurations, normalizes Annex B with FFmpeg and decodes all frames. Presence
of an encoder name, program, device or library is insufficient. VideoToolbox
software fallback is forbidden and the option is read back. VA-API tries each render
node through frame initialization, encoder opening and the complete encode/decode
probe; an unusable earlier node does not hide a later working GPU. The validated
node is retained for that worker and its rate restarts. A new worker revalidates
devices rather than trusting a previous process's device state.

Native probes run in separate bounded processes and are cached for the service
lifetime. Restart the transport to reprobe after changing drivers or permissions;
the persistent desktop owner does not need a restart. Missing helpers, loader
failures, device errors and timeouts are reported as unavailable. The browser
also checks the actual H.264 profile. A failed or rejected backend is excluded
for that connection, with a fresh IDR on the next backend; software H.264 is
tried before JPEG. Reconnect starts a new connection-specific failure set. The
settings panel displays the active backend and fallback reason without changing
the saved user preference. Preference changes and profile retries carry request IDs;
old configurations and packets cannot reject the newly requested encoder.

Hardware encoders apply bitrate/FPS changes by explicitly reopening the native
encoder and emitting IDR/SPS. This differs from OpenH264's in-place native rate
updates and may have a keyframe cost. This is RGBA conversion/upload through a
pipe, not zero copy. Driver initialization, probe and encode calls have parent
process deadlines; a broken worker is terminated. Only raw captures are replaced
before encoding. Emitted predictive packets retain their order.

## Verification limits

The private libx264 test fixture uses in-band Annex B headers on FFmpeg 6 and 9.
A separate real avcC parameter-set and length-prefixed packet fixture verifies
filter conversion and successful decoding. Together these exercise native color conversion, Annex B,
predictive frames, rate restart and decoding without advertising CPU as hardware.
The worker integration test probes the actual runner and reports negative results
honestly. GPU-free CI does not prove working VA-API/NVENC/AMF/VideoToolbox devices.
No hardware FPS, CPU, network or perceptual-quality improvement is claimed without
positive device measurements. FFmpeg runtime licensing depends on the actual
system build; see `licenses/ffmpeg-runtime.txt` in the optional artifact.
