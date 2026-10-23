# Linux Desktop Compatibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Make Linux desktop discovery and diagnostics reflect usable session capabilities, and fix rotated Hyprland resize without changing host services.

**Architecture:** Keep distribution/configuration initialization separate from display control and capture/input. Reuse the existing Wayland client, display-control crate and executable resolver. Capability probes target an explicit same-UID socket and remain bounded; they do not create input devices or modify the desktop.

**Tech Stack:** Rust, wayland-client, zbus, existing libwayshot, Unix socket fixtures, React capability presentation.

**Spec:** User requested comparison with local `/home/czyt/code/rust/uu-remote-linux` and improvement of Linux multi-desktop compatibility. Findings from independent session, capture/input and display audits are recorded below.

## Global Constraints

- Preserve the MIT implementation; learn from the GPL reference's design without copying its source.
- Never equate a desktop name, an installed command or a socket file with usable capture/input.
- Do not access other users' sessions or weaken ownership and runtime permissions.
- Do not alter, restart, migrate or install the production service. The user manages installation through AUR.
- Preserve read-only physical desktops; display resize is allowed only for owned sessions.
- Existing Rust crates take precedence over new shell wrappers and custom protocol implementations.
- UI/About/logo v0.1.13 is a separate completed commit and release; compatibility work uses its own branch and subsequent version.

## Review Focus

- A stale or silent socket must fail within the deadline and leave no blocked worker.
- Multiple same-user compositors must remain ambiguous unless an exact target is selected.
- GNOME/KDE display APIs do not prove Portal/PipeWire capture or input support.
- Rotated, scaled and negatively positioned outputs must retain orientation and layout during owned resize.
- Optional clipboard/audio helpers must not invalidate an otherwise usable attached desktop.

## Findings

- Existing `doctor` always requires Hyprland helpers and assumes a fixed DRM render node, even when attaching to another compositor or an existing persistent owner.
- Physical availability currently checks socket ownership only, while actual capture/input require several versioned Wayland globals.
- Native physical capture/input currently use libwayshot and virtual keyboard/pointer. Mutter/KScreen/X11 display-management backends are not complete remote-control backends.
- `physical_socket` ignores custom `XDG_RUNTIME_DIR` and attachment only checks `exists()`.
- Hyprland reads rotated dimensions correctly, but resize writes logical rotated dimensions as native dimensions and resets output placement to `auto`.
- The reference's shared Portal session and idle inhibitor are useful requirements for a future Portal backend. X11 native capture/input/clipboard and KMS are separate implementations; adding names or forcing a backend cannot substitute for them.

### Task 1: Mode-aware diagnostics and exact session coordinates

**Files:** `src/main.rs`, new diagnostic module if useful, `src/session.rs`, focused unit/integration tests.

- [ ] Introduce a testable dependency plan for owned startup, persistent attachment and external Wayland attachment.
- [ ] Require only tools actually used in each mode; report optional capability dependencies separately. Do not assume `hyprctl`, `pactl` or `renderD128` is universally required.
- [ ] Honor valid same-UID `XDG_RUNTIME_DIR` and exact `WAYLAND_DISPLAY`; validate selected socket type/owner and retain ambiguity errors.
- [ ] Use isolated PATH/runtime fixtures for missing tools, custom runtime, non-sockets, foreign owner and ambiguous sockets. Do not mutate global test process environment.
- [ ] Run focused tests and independently review.

### Task 2: Native Wayland capability gate

**Files:** focused new capability module, `src/wayland.rs`, `src/server.rs`, unit and UnixListener fixture tests.

- [ ] Define a serializable report for capture, keyboard, pointer and selected-output pointer; list missing interfaces/versions.
- [ ] Match actual libwayshot requirements, including ext capture path precedence and xdg-output v3. A registry probe is preliminary; first actual capture remains authoritative.
- [ ] Probe an explicit socket using existing connection/deadline/shutdown patterns; close the dedicated connection on timeout and join the worker.
- [ ] Apply the same gate to discovery and actual selection; avoid blocking the async runtime or repeatedly probing every frame/connection status update.
- [ ] Test complete wlr/ext paths, incomplete ext precedence, missing/old globals, pointer v1/v2 and a server accepting without responding.
- [ ] Run focused tests and independently review.

### Task 3: Correct Hyprland rotated resize and preserve layout

**Files:** `crates/display-control/src/hyprland.rs` and its Unix IPC fixtures.

- [ ] Add failing tests for odd rotation transforms in both legacy and Lua command paths.
- [ ] Invert the logical-to-native dimension swap before applying the mode.
- [ ] Preserve the selected output's x/y coordinates rather than requesting `auto` placement.
- [ ] Test all eight transform values and negative coordinates; retain final applied-state confirmation and physical authorization.
- [ ] Run display-control tests and independently review.

### Task 4: Integration, truthful support matrix and delivery

**Files:** bilingual README, focused Linux compatibility document, version/release notes when ready.

- [ ] Document distribution vs desktop vs protocol distinctions, same-UID service boundary and actual native/Portal/X11 support.
- [ ] Integrate the three tasks; run targeted tests, formatting and Clippy, plus existing frontend/installer guards when affected.
- [ ] Obtain an independent integrated review and resolve findings.
- [ ] Commit/push promptly; merge and publish only after checks, preserving tag-driven automation and user-managed production installation.

## Research checkpoint and expanded scope (2026-10-23)

Implementation is paused until the uu-remote comparison is consolidated in
`docs/research/2026-10-23-uu-remote-linux.md`. Existing unchecked implementation
remains work in progress, not shipped support.

The user additionally requires a `DesktopBackend` trait and native X11 capture,
input, text and clipboard in this iteration. Backend discovery and selection
must be automatic by default; backend splitting must not become a manual setup
step. Selection resolves a trusted target session first, then probes real
capabilities on that target. XWayland must not override the actual Wayland
session. Discovery must not prompt for Portal consent; opening a user-requested
connection may request consent. Denial/cancellation must not trigger a bypass or
repeated prompts.

Before resuming, expand the tasks to cover the selector, shared Frame/InputGuard
contracts, Wayland migration, X11 authority/library isolation validation, native
X11 backend and isolated Xvfb acceptance tests. Preserve separate initializer
and display-control traits. Portal is a separate backend capability path and
must remain explicitly unsupported until actually implemented and verified.
