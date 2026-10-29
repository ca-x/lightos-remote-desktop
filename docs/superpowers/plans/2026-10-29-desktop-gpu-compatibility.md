# Desktop and GPU Compatibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate applicable desktop stability and multi-GPU improvements from the UU and RustDesk investigation without copying their differently licensed implementations.

**Architecture:** Keep existing capture backends, authorization boundaries and encoder fallback policy. Normalize reusable Wayland frames using their per-frame transform, correct Portal text keysyms, and add explicit host-only encoder device selection with fail-closed worker requests. GPU identity changes invalidate probe results but never establish hardware availability.

**Tech Stack:** Rust, Wayland image-copy capture, xdg-desktop-portal, xkbcommon, FFmpeg, existing bounded native codec workers.

**Spec:** The acceptance criteria and constraints in this document implement the user's desktop compatibility, stability and dedicated/hybrid GPU request.

## Global Constraints

- Work in the current checkout. The user subsequently authorized a release after successful fixes and verification; do not install software or restart host services.
- UU is updated to `11097b5847950f7e71221cb22a38c96ead46e406`; RustDesk is read-only at `bd9262397`.
- Do not copy GPL/AGPL implementations into the current PolyForm-licensed source.
- Do not bypass Portal consent or session identity, guess connector mappings, or reinterpret browser mouse drags as touchscreen input.
- Preserve normal-output capture performance, logical input coordinates, software readiness and all existing hardware probe/deadline/fallback behavior.
- New device overrides are service-side only and must not silently select another device or be ignored by an older worker.
- No new dependencies or claims of GPU zero-copy or dedicated/hybrid hardware validation.

## Review Focus

- All eight Wayland transforms on a non-square frame must preserve pixels, dimensions and source buffer ownership.
- Output topology changes must still trigger reconfiguration; unsupported capture must retain the screenshot fallback.
- Portal text must distinguish printable keysyms from control characters, normalize CRLF and keep press/release pairs.
- Invalid or unavailable pinned devices must fail that hardware backend, not escape to another adapter; Auto may still use another backend/software.
- Old worker compatibility must be retained with no overrides and fail closed with overrides; rate restarts must retain the selected device.

## Task 1: Reusable Wayland capture on rotated outputs

**Files:** `src/wayland/capture.rs`.
**Interfaces:** `Pixels::oriented(self, transform: u32) -> Result<Pixels>` consumes packed RGBA; callers use the image-copy frame transform, not output geometry.

- [x] Add non-square, all-eight-transform tests with exact pixel ordering, unchanged input, normal Arc reuse, singleton axes and malformed-frame rejection.
- [x] Observe the unsupported-transform regression fail before implementing orientation.
- [x] Apply orientation at capture completion and allow transformed outputs into the reusable path; preserve reconfiguration and fallback.
- [x] Run focused Wayland tests.

## Task 2: Portal text keysym correctness

**Files:** `src/backend/portal/native.rs`.
**Interfaces:** `text_keysyms(text: &str) -> Vec<u32>` returns canonical xkbcommon keysyms; CRLF is one Return, lone CR/LF are Return, Tab is Tab, other control characters are ignored.

- [x] Extract the existing mapping and add tests for multiline text, Latin-1, CJK, supplementary Unicode and controls.
- [x] Observe existing mapping fail; use xkbcommon's canonical conversion rather than handcrafted Unicode keysyms.
- [x] Keep text events in the existing authorized input queue, with paired releases and no clipboard/external-command bypass.
- [x] Run Portal input and cancellation tests.

## Task 3: Host-only GPU selection and identity invalidation

**Files:** `crates/media-codec/src/hardware.rs`, `crates/media-codec/src/worker_protocol.rs`, `crates/media-codec-ffmpeg/src/main.rs`, `crates/media-codec-ffmpeg/src/encoder.rs`, related worker tests, `docs/hardware-encoding.md`.
**Interfaces:** `LIGHTOS_VAAPI_DEVICE` is an absolute device path (including stable by-path links); `LIGHTOS_NVENC_GPU` is a nonnegative FFmpeg GPU index. Append distinct device-specific Probe/Init request variants so legacy workers reject rather than ignore pinning. Unset overrides retain version-1 bytes and automatic discovery. Pinning is backend-specific and survives rate restarts.

- [x] Observe parser and identity regressions fail before implementing; add protocol legacy-rejection and worker-selection tests.
- [x] Snapshot service-side overrides in the factory; validate before requests and include choices in cache identity.
- [x] Pass explicit typed devices in the new requests; validate device/backend compatibility in the worker and retain full encode/decode probes.
- [x] Include all NVIDIA device nodes, DRM driver identity/version, ownership and change time in Linux cache invalidation; test same-size version changes.
- [x] Run media-codec and native worker tests, including no-device failures and CPU fixtures; document operator configuration and hardware limits.

## Task 4: Provenance, review and verification

**Files:** `docs/desktop-gpu-compatibility.md`, this execution checklist.

- [x] Record adopted behavior, already-covered changes and deliberately excluded behavior against the reference revisions.
- [x] Run formatting, workspace default/Portal/RDP tests, native worker tests and strict Clippy for the touched configurations.
- [x] Review the complete diff for authorization, old worker compatibility, fallback and host side effects.
- [x] Record verification results and outstanding hardware-only checks before release.

## Task 5: Authorized release

- [x] Confirm the next version against current remote tags and follow the repository's existing release workflow.
- [ ] Commit verified changes, tag and push the authorized release; monitor automated CI, Release, Arch codec and AUR publication.
- [ ] Verify released assets and package versions without installing or restarting anything on the host.

## Execution Notes

- Baseline is clean `main` at `0414d31`; the existing installation and running services remain untouched.
- Local verification: `cargo fmt --all --check`; workspace tests with `bundled-jpeg` (308 passed, 24 opt-in ignored) and `bundled-jpeg,rdp,portal` (348 passed, 25 opt-in ignored); strict all-target Clippy for both configurations.
- Native worker: 8 unit tests and 2 real-worker tests passed with `native` and with `native,fast-yuv`; strict all-target Clippy passed for both. Tests include NVENC option readback without a GPU driver and explicit rejection of missing VA-API/invalid NVENC selections, not positive hybrid-GPU acceptance.
- AUR source contracts: package/lifecycle 49, optional codec 47 and release guards 54 checks passed using isolated fixtures; no host services changed.
- Review ruling: explicit pins are an opt-in escape from adapter discovery or an earlier device consuming the backend deadline; leave unverified zero-copy, per-adapter process isolation and generic Portal IME workarounds out of this patch.
- Prepared version `0.1.24` after confirming the latest published stable release and remote tags were `v0.1.23`. Publication checkboxes are intentionally pending in this pre-tag source snapshot; remote outcomes are verified after the immutable tag is pushed.
