# Capture and Encode Performance Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce connection startup delay and unnecessary CPU/memory traffic, then publish the verified next patch release.

**Architecture:** Keep DesktopBackend, VideoEncoder and stream-scheduler boundaries. Keep the isolated native codec worker. Optimize only after actual protocol/encoder capabilities have been established; retain software and core X11 fallbacks.

**Tech Stack:** Rust, tokio, x11rb (existing shm/damage features), pipewire, ffmpeg-next, existing bincode worker protocol and GitHub/AUR release automation.

**Spec:** User request in this session; existing docs/hardware-encoding.md, docs/encoder-scaling.md and docs/research/2026-10-23-uu-remote-linux.md define compatibility boundaries.

## Global Constraints

- Independently implement ideas from the GPL reference; LightOS remains MIT.
- No production service installation, restart, desktop change or GPU benchmark on the active desktop.
- No assumption that an executable, X11 extension, GPU, encoder or browser profile exists.
- Keep actual encode/decode capability validation, backend fallback, frame dependency order, privacy epochs and explicit user preferences.
- Do not manually dispatch or retry release/AUR workflows. Publish a new tag and verify the automatic pipeline.
- GPU handle transport, experimental native driver engines, new video codecs and 4:4:4 negotiation need separate architecture and hardware acceptance; retain the current portable CPU frame interface in this release.

## Review Focus

- A hanging/missing worker must not block initial ready or launch unlimited probes.
- Failure cache expiry and helper replacement must permit recovery without overriding manual backend selection.
- Capture/encoder buffer reuse must not mutate pixels still owned by another consumer.
- Missing SHM/Damage and unsupported X11 formats must preserve working capture; resize and stop must release resources.
- Portal backlog handling must return every dequeued buffer and preserve lock/cancellation boundaries.

## Task 1: Hardware capability lifecycle

Files: crates/media-codec/src/capabilities.rs (new), src/server.rs, crates/media-codec/src/hardware.rs.

- [x] Introduce a bounded single-flight cache with expiring hardware failures and helper identity invalidation.
- [x] Start the background probe without delaying ready; serve software capabilities immediately and publish completed capability updates.
- [x] Preserve real eight-frame encode/decode probes and manual selections; test concurrent requests, failure expiry, invalidation and startup responsiveness.

## Task 2: Native worker memory traffic

Files: crates/media-codec/src/worker_protocol.rs, crates/media-codec/src/hardware.rs, crates/media-codec-ffmpeg/src/encoder.rs.

- [x] Serialize borrowed frame input using the existing wire layout, avoiding the extra raw RGBA Vec copy.
- [x] Reuse FFmpeg CPU frames only after ensuring writability; keep the VA-API hardware-frame pool.
- [x] Verify wire compatibility, unchanged input ownership and native encode/decode colors/rate restart.

## Task 3: Capture work avoidance

Files: src/backend/portal/native.rs, src/backend/x11.rs, src/backend/x11/capture.rs (new).

- [x] Drain queued PipeWire buffers before conversion and reuse conversion storage when ownership permits.
- [x] Add automatically negotiated XDamage and MIT-SHM capture with independent fallback to core GetImage.
- [x] Validate strides, masks, byte order, extension failure, geometry changes and resource cleanup with isolated tests/Xvfb when available.

## Task 4: Verification and release

Files: docs/hardware-encoding.md, docs/runtime-platforms.md, docs/releases/vNEXT.md, Cargo.toml, Cargo.lock and packaging versions as required by the existing workflow.

- [x] Run targeted unit/integration checks, formatting and strict Clippy appropriate to changed paths.
- [x] Measure synthetic buffer/IPC work with fixed inputs; report evidence and limits without claiming unmeasured GPU/desktop gains.
- [x] Review the complete diff and recheck origin/main before integration.
- [ ] Commit changes, push main, publish the next unused patch tag and verify CI, release assets, Arch codec and both AUR packages.
