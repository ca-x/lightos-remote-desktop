# Native Media Implementation Plan

**Goal:** Remove external FFmpeg processes from runtime video encoding and audio capture without changing the browser protocol.

**Architecture:** A dedicated worker owns the OpenH264 encoder and reusable I420 conversion buffer. A cancellable PulseAudio worker owns its context, stream and nonblocking mainloop; a four-packet queue bounds audio latency.

**Tech Stack:** Rust, openh264 (source-built), libpulse-binding, existing WebCodecs/PCM WebSocket protocol.

## Constraints / Spec
- Retain H.264 Annex-B packets, codec identification, keyframe handling, render ACK flow and JPEG fallback.
- Retain stereo 48 kHz float32 little-endian audio, isolated Pulse server selection and physical-session audio restriction.
- Do not change process-wide Pulse environment or copy RustDesk code. Initial candidate isolation is complete; the user subsequently authorized local deployment and a release.
- Software encoding first; hardware encoding, VP8/AV1 and Opus are separate work.
- Source-built OpenH264 does not imply coverage by Cisco's binary patent arrangement; document before any release.

## Review Focus
- Encoder initialization, invalid dimensions, resize, canceled encode and worker shutdown.
- No frame skipping that silently consumes video credit; startup includes SPS/PPS/IDR.
- Audio connection failures and stop while connecting/idle; bounded memory and no old-audio backlog.
- Preserve PCM frame alignment across native peek boundaries and silence holes.
- Native dependency installation, CI, third-party notices; FFmpeg permitted only as a test decoder.

## Tasks
- [x] Replace src/video.rs FFmpeg pipes with a bounded encoder worker; test key/delta, dimensions, repeated create/drop and decoding.
- [x] Add src/audio.rs native capture, integrate src/server.rs and doctor; test packetization, cancellation and bounded audio delivery.
- [x] Update Cargo, CI and runtime/third-party docs; run fmt, Rust/frontend/installer tests, clippy and release build.
- [x] Validate an isolated candidate (video, audio, worker teardown); record evidence and remaining performance limits in the release notes.

## Execution Notes
- Baseline: main 917f8481, version 0.1.1, clean worktree. The subsequently authorized v0.1.2 transactional deployment is active, with authenticated version/health checks passing and zero restarts.
- Review ruling: replace a single latest-only audio packet with a bounded 40 ms queue; otherwise ordinary 20 ms Pulse fragments lose half their samples without network congestion. A regression test covers both packets.

## Local performance/deployment follow-up
- [x] Pass captured Arc pixels directly to the encoder instead of copying full RGBA frames into the request queue.
- [x] Generate/cache JPEG on demand in blocking workers, not on every changed capture while H.264 is active.
- [x] Amortize Linux AF_UNIX/Wayland setup over bounded batches (at most eight frames, checking 250 ms expiry before the next capture), retaining periodic refresh to bound libwayshot protocol-object retention and discover output changes.
- [x] Set TCP_NODELAY on accepted connections; keep existing input/video flow control.
- [x] Verify resize, JPEG/H.264, nonzero audio, teardown, CPU/syscall and throughput samples before local transactional deployment. Preserve v0.1.1 rollback. RSS observations are not a controlled memory improvement claim.
- Linux DMA-BUF/hardware encode is not part of this patch: the current software encoder still needs CPU pixels. Do not label the shared-Arc path GPU zero-copy.
