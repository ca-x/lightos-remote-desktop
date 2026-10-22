# Hardware Encoding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add real optional hardware H264 backends, automatic selection and honest failure fallback while keeping software and the persistent desktop safe.

**Architecture:** media-codec owns the catalog/selection and bounded native-worker bridge. An optional media-codec-ffmpeg executable wraps native FFmpeg using ffmpeg-next. The server negotiates a backend per viewer, and the existing UI persists format, backend and scheduling choices independently.

**Tech Stack:** Existing Rust/Tokio/serde/OpenH264, ffmpeg-next9 generated bindings, mature binary framing/serialization, current React/TypeScript frontend.

**Spec:** `docs/superpowers/specs/2026-10-22-hardware-encoding.md`

## Global Constraints

- Auto defaults, manual choices persist, actual probes precede selection.
- No copied AGPL source or FFmpeg subprocess codec pipeline.
- Core must run without the optional FFmpeg worker or libraries.
- Bound IPC, driver initialization and encoding; kill/reap failed workers.
- Drop raw captures only, preserve predictive packets and restart with keyframes.
- Never restart the desktop owner for transport/codec upgrades.
- Preserve existing mobile/IME/resize and physical read-only behavior.

## Task1: Catalog, preference and bounded worker protocol

**Files:** `crates/media-codec/src/{lib,selection,hardware}.rs`, protocol module and tests; `crates/media-codec/Cargo.toml`, lockfile.

- [x] Add RED tests for validated selection, manual choice, per-connection exclusion and software fallback.
- [x] Add backend/preference types and capability metadata without breaking software-only EncoderFactory users.
- [x] Implement versioned bounded framing with initialization/probe/encode deadlines and child cleanup; test malformed/missing/failed workers.
- [x] Run focused tests and commit/push.

## Task2: Native FFmpeg hardware worker

**Files:** optional `crates/media-codec-ffmpeg/{Cargo.toml,src/main.rs,src/encoder.rs}` and tests; workspace manifest/CI native test job.

- [x] Wrap ffmpeg-next9 for NVENC/AMF/VideoToolbox and VA-API device/frame upload, low-delay H264 and RGBA conversion.
- [x] Implement actual encode/decode probe and honest native capability/failure reports.
- [x] Implement persistent input encoding and explicit rate-change/keyframe behavior, using mature bindings.
- [x] Test native conversion/stream decode with a private software fixture; run actual host probes; document missing-device limits.
- [x] Verify feature-off workspace and feature-on native worker; commit/push.

## Task3: Negotiation, runtime fallback and settings

**Files:** `src/server.rs`, `web/src/{client,preferences,App,i18n}.ts*` and corresponding tests.

- [x] Add RED tests for backend persistence/selection, profile rejection and fallback to software H264 before JPEG.
- [x] Probe/cache catalog, negotiate chosen backend, expose actual backend and fallback reasons; update controls in current UI style.
- [x] Exclude failed encoders per connection, preserve ACK/pacing and reset/keyframes correctly on backend changes.
- [x] Verify browser/protocol regressions and actual worker failure fallback; commit/push.

## Task4: Installation, review and delivery

**Files:** installer/tests/docs/verification/runtime-platforms and plan status.

- [x] Stage optional sibling worker inside the same immutable release, include it in bundle identity, preserve rollback and software-only behavior.
- [x] Verify optional worker/native FFmpeg prerequisite documentation and CI; core portable release must retain software fallback.
- [ ] Run appropriate full checks, real native/codec/protocol and Ghostty/handoff tests; request independent review and resolve blockers.
- [ ] Bump the integrated version once; commit/push, merge with separately reviewed MCP/RDP work and latest main; preserve user desktop on local update.
- [ ] Report exactly which devices/platforms were validated, source/CI/release/local state and remaining limits; save durable memory.
