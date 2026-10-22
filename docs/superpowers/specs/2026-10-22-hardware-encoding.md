# Hardware encoding and automatic backend selection

## Scope

Add optional native H264 encoders for VA-API, NVENC, AMF and VideoToolbox behind
media-codec. Reference RustDesk's actual-capability catalog, per-client selection,
separate probe process and failure fallback without copying its AGPL source.
Retain the current software OpenH264/TurboJPEG and browser WebCodecs paths.

The browser still chooses H264 or JPEG and a scheduling policy. A separate
backend preference defaults to Auto and permits Software or a named hardware
backend. Persist and resend it on reconnect. Availability is the intersection of
server probe success and client decoding support; a codec name, device node or
installed executable is not evidence of usable encoding. Hardware decoding in a
browser is independent of server hardware encoding.

## Boundaries

- A persistent native worker uses ffmpeg-next bindings, never the FFmpeg CLI.
  It owns the selected native encoder, color conversion and device/frame upload.
- The optional worker is a separate executable, discovered beside the service
  executable. This isolates driver crashes/hangs and keeps the core executable
  independent of FFmpeg shared libraries. Missing/incompatible worker or drivers
  leave software encoding available.
- The worker is built against the installed FFmpeg libraries. A locally built
  helper is not automatically a portable binary across FFmpeg ABI versions.
  The existing portable core remains independently runnable. Document native
  build/runtime prerequisites and do not silently bundle system multimedia libs.
- Reuse existing crates for FFI, pixel conversion, framed IPC and serialization.
  Bound message sizes, protocol versions, initialization, probe and encode times;
  terminate/reap a failed worker. Never hold a blocking driver call in the server.
- Probe actual initialization, multiple encoded frames, configuration and decode
  validity. Cache bounded results. Report unsupported platforms/missing helpers/
  device permissions/encoder failures with reasons; do not advertise software
  fallback as successful hardware encoding.
- Auto prefers a validated hardware backend for H264, then OpenH264, then JPEG if
  needed. A per-connection failure excludes that backend and triggers a fresh
  keyframe on the next backend. Client profile rejection also tries software
  H264 before JPEG. Manual choices may fall back with an observable reason.
- Preserve configured caps, real encoder budgets, render ACK accounting and
  prediction dependencies. Native rate updates may require a fresh hardware
  encoder/keyframe if the backend lacks a reliable live rate API; state that
  behavior explicitly rather than claiming OpenH264-style in-place updates.
- Preserve mobile gestures, IME, resolution/scale, physical-session read-only
  rules, native Omarchy configuration and persistent desktop ownership.
- Default-off MCP/RDP work proceeds independently and is integrated only after
  review. This feature does not implement Windows/macOS desktop capture/input.

## Evidence

Test selection, invalid/absent capability handling, stale/failed backends,
protocol bounds, deadlines and cleanup. Exercise the native FFmpeg conversion/
encode/decode path using a test-only software codec when hardware is absent;
never advertise that fixture as a hardware success. Probe the actual host and
report negative results honestly. Verify fallback through actual WebSocket
traffic and a continuously decodable H264 stream; rerun normal/bundled Rust,
Clippy, frontend, installer fixtures and persistent-owner Ghostty checks as
relevant before merge/deployment. No hardware FPS/CPU improvement claim without
positive-device measurements.
