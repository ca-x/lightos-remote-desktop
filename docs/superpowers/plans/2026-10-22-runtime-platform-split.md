# Runtime platform, codec and scheduling extraction

Baseline: origin/main `050fbe10879b14a4305d3d9a1109a3bf5917acc2` (mobile controls and high-density rendering). User explicitly requests plan first, then execution, version bump, commit/push and local update. Execute inline in reused worktree `refactor/platform-media`. Earlier pre-sync branch/stash are reference only, not a merge base for implementation.

## Evidence and constraints

- Existing crates cover system detection and configuration initialization; runtime display IPC, codecs and scheduling still live partly in server/capture.
- Latest main adds `resize_scale`, allowed scales 1/1.5/2/2.5/3, integral logical dimensions >=320x200 and physical dimensions <=3840x2160. Preserve this validation, mobile module/gestures/IME/dock, and real scale tests.
- Read local RustDesk at `/home/czyt/code/rust/rustdesk`, HEAD `bd9262397`. `display_service`/`platform`/`scrap::Display` separate display metadata and OS operations; `EncoderApi`/factory separate codecs; `hwcodec` probes real availability; `VideoQoS` handles network policy. Reference responsibility boundaries, not AGPL code copying. RustDesk's Linux XRandR path does not implement our Hyprland/Lua support.
- No unsupported claims: current desktop capture/input is Linux Wayland, server encoding is software OpenH264/TurboJPEG, decoding is browser WebCodecs/JPEG. New OS/hardware backends are future implementations.
- Preserve independent session owner; update transport only. Do not modify host desktop configuration.

## Target boundaries

1. `display-control`: common output model and object-safe DisplayController; read-only Wayland output discovery, session-scoped Hyprland IPC control. DisplayMode carries physical dimensions AND scale. Probe is read-only; missing/unresponsive/unsupported IPC produces resize=false plus reason. Physical sessions remain read-only by policy. Target a named output and confirm applied size/scale with bounded IPC.
2. `media-codec`: EncoderFactory/VideoEncoder, existing OpenH264 worker and TurboJPEG compression, capability records (backend, hardware flag, dynamic quality, actual availability/failure). Bounded real test-frame probes; server/client use intersection of actual encoder and browser decoding support. JPEG cache remains on capture frame.
3. `stream-scheduler`: pure session-local FrameScheduler/latency statistics, explicit time inputs for deterministic tests. No WebSocket, compositor, encoder or global settings. Server parses messages, applies policy and performs IO.
4. `web/src/video-codec.ts`: browser decode capability probing; keep session/generation cancellation and decoder lifecycle in client.
5. Existing toolbar gains persistent user preferences: encoding Auto(default)/H264/JPEG; scheduling Auto(default existing ACK1..3)/Low latency(single frame)/Smoothness(up to3 after first render ACK). Capability failures transparently fall back to JPEG without overwriting preference; explain actual mode. Defaults remain compatible with older clients.

## Execution tasks

- [x] A. Record latest baseline checks: workspace tests and frontend tests/build. Save this plan before editing implementation.
- [x] B. Add display-control crate. Move enumeration/rotation tests. Preserve scaled-mode validation and matching dimensions+scale, including same physical size with changed scale. Cover explicit Lua fallback, rejection, timeout, absent output and acknowledged-but-not-applied changes. Wire actual capabilities to ready.resize and ready.resize_scale; keep physical permission errors.
- [x] C. Add media-codec crate. Move existing native encoder and its cancellation/keyframe tests; move JPEG compression while retaining lazy frame cache. Add common factory, real frame capability tests and dynamic-quality honesty. Replace direct encoder usage in server; retain public video exports for benchmarks. Doctor/health distinguish compiled codecs from runtime probe results.
- [x] D. Add stream-scheduler crate. Move ACK algorithm/regressions; encapsulate pending/window fields. Add Auto/LowLatency/Smooth policy, deterministic network-delay tests, first-frame/resize/codec-reset gates and stale cumulative ACK tests. Server retains input/control independence.
- [x] E. Add encoding/strategy preferences and localized existing-style settings; default Auto, validate persisted values, restore on reconnect, preserve fallback preference. Isolate browser decode probe and test hardware-query failure/software fallback. Preserve all mobile and HiDPI tests.
- [ ] F. Bump to 0.1.8. Update README and validation ledger. Run workspace, frontend, Clippy/fmt, installer fixtures, bundled-jpeg build/tests and explicit real native Lua/Ghostty resize/scale/reconnect tests. Independent read-only review, resolve blockers.
- [ ] G. Commit/push, sync latest remote again before merging. Merge verified branch to main and push. Build reviewed binary, independent systemd installer task, verify installed version/health/config errors and unchanged owner/signature. Verify remote SHA; do not equate CI queued with passed or claim untested hardware/mobile-device/network performance.

## Acceptance

No compositor command generation in server; no codec implementation in capture/server; no scheduler algorithm in server. Existing normal and high-density resize survives both transport generations. User codec/strategy selection is persistent and effective with automatic default. Real frame/metadata verification, not just exit statuses. Local installed version and source commit are identifiable.

## Scope extension (user, 2026-10-22)

Implement Linux desktop backends, including GNOME/Mutter DisplayConfig, KDE/KScreen, wlroots output management via wlr-randr, and X11/RandR (XFCE/MATE/Cinnamon/LXQt and other X11 desktops). Use session-scoped service/socket/environment, read-only runtime probes, explicit output/mode selection, bounded subprocess or DBus calls, preserve other outputs, verify actual applied dimensions and scale, expose missing-tool/protocol reasons. Keep physical-session resize authorization separate from backend ability. This adds display control, not GNOME/KDE capture/input support or Windows/macOS support. Include fixture tests for parsers/selection/command construction; only Hyprland can be tested against a real compositor on this host.

## Execution evidence

- A: Latest baseline Rust workspace and frontend build/tests passed before adapting implementation.
- B: Display crate includes scaled mode validation; 26 display tests pass. Real Lua resize/scale/Ghostty transport-handoff test passes. Added GNOME/KDE/wlroots/X11 adapters after explicit user scope extension; only Hyprland has real desktop evidence on this host.
- C/D/E: Workspace and bundled-JPEG tests pass; frontend 80 Node +15 Vitest tests and production build pass. Clippy with denied warnings passes. Existing mature Rust libraries implement protocols/codecs; no RustDesk source copied.
- Final review and deployment pending.
