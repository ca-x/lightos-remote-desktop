# Audio Latency Implementation Plan

**Goal:** Reduce audio/video contention without lowering desktop image quality.

**Architecture:** Negotiate Opus on the authenticated audio socket, retaining PCM
for legacy clients. Encode on the native capture thread. Decode in a dedicated
browser Worker and play through an AudioWorklet with bounded buffering. Keep
controller lifetime, Origin checks, URL prefixes and embedded assets intact.

**Tech Stack:** Rust/Tokio, libopus, WebCodecs, AudioWorklet, existing React UI.

- [x] Add bounded playback tests and protocol negotiation tests.
- [x] Add 20 ms, 128 kbps Opus capture and timestamped audio packets.
- [x] Add browser Worker decoding and bounded AudioWorklet playback with PCM fallback.
- [x] Add ACK/encoding/send diagnostic samples, without changing video pacing.
- [x] Verify frontend, Rust, packaging and an isolated live candidate.

Acceptance: Opus round-trip produces nonzero stereo samples; playback cannot
accumulate stale audio; audio failure cannot disconnect video; new assets are
embedded/authenticated; video resolution and quality are unchanged. Local
functional tests alone do not establish a WAN latency improvement. No release
or push is authorized. Restarting the production virtual desktop requires notice.

Validation: 65 Node tests and 3 React tests; Rust 54 library tests passed with 2
environment-dependent tests ignored, plus normal integration tests. Explicit live
PCM and Opus tests passed, including nonzero Opus signal, authentication/Origin,
video-ACK isolation and controller teardown. Installer 40 / compatibility 13
checks, fmt, clippy and release build passed. Opus uses bundled static code;
DT_NEEDED does not add libopus (system Pulse may load its own codecs transitively).

An isolated animated desktop with a continuous tone held about 30 FPS with audio
off and Opus on. Browser-observed Opus transport was about 131.6 kbit/s / 50
packets per second. Fullscreen and injected audio failure preserved video. These
are localhost functional measurements, not proof of the user's WAN latency fix.
The isolated browser and candidate were stopped. After advance restart notice,
the local service was updated to the new 0.1.5-dev build, SHA256
`1cc380d124a89184fb08db1c5a965c6c7a3e5840bd89262e456b95a65d078a6d`;
authenticated embedded assets match the build and health is ready. No release,
commit or push was performed.
