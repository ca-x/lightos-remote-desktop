# Adaptive streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add baseline-aware QoS, Quality first and weak-network recovery, then verify and deploy without disturbing the persistent desktop.

**Architecture:** Keep delay policy in stream-scheduler and native rate updates in media-codec. The server applies bounded pacing before encoding and reports diagnostics. The browser preserves user choices and manages reconnect/liveness.

**Tech Stack:** Rust std/Tokio, existing OpenH264/openh264-sys2 and TurboJPEG, existing WebSocket/TypeScript/React frontend, Rust/native and Node/Vitest tests.

**Spec:** `docs/superpowers/specs/2026-10-22-adaptive-streaming.md`

## Global Constraints

- Default Auto; manual encoding and strategy preferences persist.
- Render ACK time is not pure network RTT; only validated fresh feedback controls QoS.
- Frame credits remain 1..3; drop raw captures only, never encoded predictive frames.
- Preserve mobile controls, IME, resolution/scale and physical-session read-only rules.
- Never restart the persistent desktop owner during transport deployment.
- RustDesk source is read-only AGPL reference; use existing crates for native functionality.

## Review Focus

- A stable 500ms path must not progressively lose quality merely because delay is high.
- Queued ACKs from before a cut or stall must not trigger repeated reductions.
- Quality first must actually preserve per-frame budget on H.264 as well as JPEG.
- No ACKs on a static screen must not trigger stall control or break reconnect.
- A weak connection may outlast two retries; manual disconnect and exclusive release must still work.

### Task 1: Pure adaptive controller and credit integration

**Files:** create `crates/stream-scheduler/src/qos.rs`; modify `crates/stream-scheduler/src/lib.rs`.
**Interfaces:** produce `RateTarget { fps, quality_percent }`, `FrameScheduler::target`, `next_frame_at`, `tick`, `qos_summary`, `set_quality_scalable`; retain existing configure/ACK/window APIs. Observe sent-at/ACK-at Instants. Samples spaced at least 500ms; FPS cap validated by existing constructor. Strategies set excess-delay threshold (100/150/220/150ms), minimum auto FPS min(5, cap), Quality first min(2, cap). Encoder budget factor bounded20..100%; quality cuts cooldown2s; recovery requires consecutive clean feedback. Baseline uses20 recent fresh samples; only settled floors may rise gradually.

- [x] Write deterministic failing tests for the spec's feedback/pacing/reset scenarios.
- [x] Implement minimal policy independent of codecs and sockets, preserving old window behavior.
- [x] Run scheduler tests; commit and push.

### Task 2: Real persistent encoder rate updates

**Files:** modify `crates/media-codec/Cargo.toml`, `src/lib.rs`, `src/openh264.rs`, root `Cargo.lock`.
**Interfaces:** produce `EncoderRate { bitrate_bps, fps }`, default bitrate helper, `VideoEncoder::set_rate`; H.264 requests carry desired rate and worker applies generated SetOption bindings before encoding. Expose probed dynamic bitrate capability. Use effective bitrate = default bitrate * quality_percent/100 * target_fps/configured_fps, so Quality first retains bits per frame while reducing bandwidth.

- [x] Add failing decode-continuity and runtime-rate readback/budget tests.
- [x] Use existing OpenH264 generated API bindings; preserve cancellation safety and SPS/IDR rules.
- [x] Run media-codec tests; commit and push.

### Task 3: Server pacing, diagnostics and weak-write tolerance

**Files:** modify `src/server.rs`; create `tests/adaptive_stream.rs` and a repeatable isolated benchmark runner under `scripts/`.
**Interfaces:** consume tasks1/2; apply target rate before encoding; pace latest raw frames; expose QoS and applied codec rate in video_metrics. Tick stall controller independently of video credit. Bound established writes15s.

- [ ] Capture 0.1.8 baseline artifact and run reproducible isolated moving-desktop/network scenario.
- [ ] Write regression tests for rate mapping/transport behavior; integrate controls with no wire break for legacy clients.
- [ ] Compare clean/constrained/recovery/interruption conditions and decode actual H.264 streams.
- [ ] Run targeted protocol/input checks; commit and push.

### Task 4: Quality option and persistent reconnect

**Files:** modify `web/src/preferences.ts`, `App.tsx`, `i18n.ts`, `client.ts`; relevant frontend/UI tests.
**Interfaces:** new `quality_first` enum wire value and bilingual select entry. Retry with capped exponential backoff while wanted, pause offline/resume online; monitor visible connection inactivity. Keep decoder generation cancellation and desktop-switch release invariant.

- [ ] Add failing preference, repeated-retry, offline/online, liveness and manual-disconnect tests.
- [ ] Add option in existing UI style; implement reconnect/liveness behavior.
- [ ] Build and run frontend tests; commit and push.

### Task 5: Review, release and persistent-owner deployment

**Files:** update `Cargo.toml`, `Cargo.lock`, `docs/runtime-platforms.md`, `docs/rustdesk-reference.md`, `docs/verification.md`, plan status.

- [ ] Run normal/bundled workspace tests, clippy/fmt, frontend tests/build and installer fixtures.
- [ ] Run native Ghostty/resize/handoff, real codec/protocol checks and final A/B evidence.
- [ ] Request one independent whole-branch review; fix blockers with regression evidence.
- [ ] Bump0.1.9; commit/push; fetch latest main, reconcile/reverify if changed, merge/push main.
- [ ] Install tested binary via independent installer; confirm digest, health, unchanged owner/signature and existing GUI application PIDs. Record limitations and deployment receipt; push docs and save durable memory.
