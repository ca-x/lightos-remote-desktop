# Optional native H.264 encoding

The portable service keeps its built-in OpenH264 and TurboJPEG encoders. The
optional sibling executable `lightos-codec-ffmpeg` supplies VA-API, NVENC, AMF and
VideoToolbox through native `ffmpeg-next` bindings. No FFmpeg command-line codec
pipeline is launched. Browser hardware decoding is independent of server encoding.

Capture/input and encoding are separate capabilities. The host has Linux
Wayland/X11/Portal backends and experimental Windows/macOS physical-desktop
backends; see [native host support](cross-platform-host.md). The official optional
FFmpeg worker packages currently target Linux. Windows/macOS host packages include
software H.264 and JPEG; a working hardware encoder on those systems requires a
compatible, separately built worker and actual capability validation.

## Arch Linux / Omarchy optional package

```sh
yay -S lightos-remoted-bin
yay -S lightos-codec-ffmpeg-bin  # optional GPU encoding worker
```

The helper is an optional precompiled dependency, built in Arch Linux CI from
the release source tag against Arch's FFmpeg ABI. It installs beside the managed
service executable; no worker path setting is needed. The service keeps its built-in OpenH264 and
JPEG encoders when the helper is absent or hardware probing fails. Installing,
upgrading or removing the helper refreshes an already-running managed transport
to clear encoder probes, while desktop applications remain running. Stopped,
legacy and recovery services are left alone. After a system FFmpeg ABI change,
update the helper package when its recorded SONAME dependencies require it.
GPU drivers and service-user device access are still required. See [AUR packaging](aur.md).

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
cargo build --release --locked --features bundled-jpeg,rdp
cargo build --release --locked -p media-codec-ffmpeg --features native,media-codec/bundled-jpeg
sudo ./target/release/lightos-remoted install-service
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

Native probes run in separate bounded processes in the background, so their
deadlines do not delay the initial software-ready connection. Existing viewers
receive capability updates; Auto and an explicitly requested hardware backend
can switch after validation, while Software and JPEG choices remain unchanged.
Only one hardware probe batch runs at a time. Results expire after ten minutes
when any platform-supported backend failed, otherwise after one hour; a later
connection triggers a refresh. Replacing the helper or changing detected GPU
device identities invalidates the cache on the next connection. Linux identities
include NVIDIA nodes, DRM device/driver bindings, bounded driver-version contents,
device ownership/change times and the service's device overrides. These identities
only invalidate results; they never prove encoder availability. Undetected changes
still become eligible for the TTL refresh or an explicit transport restart;
changing a running service's supplementary groups requires the usual service
restart. The persistent desktop owner does not need a restart. Missing helpers, loader
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

## Dedicated and hybrid GPU selection

By default VA-API validates all render nodes in order, including GPUs without
connected displays. NVENC retains FFmpeg's automatic device selection. Capture
and encoding use CPU RGBA frames, so their GPUs need not be the same adapter.
This is not a GPU-to-GPU or DMA-BUF zero-copy pipeline.

For an adapter that fails automatic selection or a multi-GPU machine where a
particular encoder is preferred, the host administrator can set:

| Transport environment variable | Value |
| --- | --- |
| `LIGHTOS_VAAPI_DEVICE` | Absolute UTF-8 render-device path; stable `/dev/dri/by-path/*-render` links are supported. |
| `LIGHTOS_NVENC_GPU` | Nonnegative FFmpeg/CUDA GPU index, `0` through `2147483647`; not a DRM render-node number. |

These are host-only settings, not browser parameters. For a managed Linux install,
put the desired variable in a systemd drop-in for
`lightos-remote-desktop.service`, for example:

```ini
[Service]
Environment=LIGHTOS_VAAPI_DEVICE=/dev/dri/by-path/pci-0000:03:00.0-render
```

Use the actual path/index on the host; the example is not hardware detection.
The environment is read when the transport starts. Apply configuration through
the normal transport-service lifecycle, not by restarting
`lightos-remote-desktop-session.service` and closing desktop applications.

An unset variable preserves automatic behavior; an empty, malformed, unavailable
or denied override makes only that hardware backend unavailable. A pinned VA-API
probe never tries another render node. NVENC sets and reads back FFmpeg's `gpu`
option. Probe and initialization receive the same typed selection, and rate
restarts retain it. The existing complete encode/decode probe is still required;
neither a valid index nor successful option readback proves that the GPU encodes.
Auto may use another verified backend or software; an explicitly selected backend
retains its existing software fallback policy.

Device-specific requests are distinct additions to the local worker protocol.
An older helper rejects them and the core falls back instead of silently ignoring
the pin. With no overrides, the original version-1 requests remain compatible.
Upgrade both packages to use device selection. No driver installation, device
permission change, PRIME setting or desktop switch is performed automatically.

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

## Capture and worker performance

Worker frame requests borrow the RGBA input and serialize it as bytes instead of
cloning it and encoding a general sequence. Version 1 bytes stay compatible with
older helpers in both directions. FFmpeg CPU conversion frames are reused when
writable; retained driver references cause copy-on-write before the next update.
These changes still use CPU conversion and pipe IPC. They do not introduce GPU
handle transfer or change browser video bytes. Reproducible measurements and
capture optimizations are documented in [capture/encode performance](capture-encode-performance.md).
