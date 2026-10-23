# Linux Desktop Compatibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Make Linux desktop discovery and diagnostics reflect usable session capabilities, and fix rotated Hyprland resize without changing host services.

**Architecture:** Keep distribution/configuration initialization separate from display control and capture/input. Reuse the existing Wayland client, display-control crate and executable resolver. Capability probes target an explicit same-UID socket and remain bounded; they do not create input devices or modify the desktop.

**Tech Stack:** Rust, wayland-client, zbus, libwayshot, x11rb, enigo, arboard, optional ashpd/PipeWire, Unix socket fixtures and React.

**Spec:** User requested comparison with local `/home/czyt/code/rust/uu-remote-linux` and improvement of Linux multi-desktop compatibility. Findings from independent session, capture/input and display audits are recorded below.

## Global Constraints

- Preserve the MIT implementation; learn from the GPL reference's design without copying its source.
- Never equate a desktop name, an installed command or a socket file with usable capture/input.
- Do not access other users' sessions or weaken ownership and runtime permissions.
- Do not alter, restart, migrate or install the production service. The user manages installation through AUR.
- Preserve physical monitor configuration; display resize is allowed only for owned sessions.
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

- [x] Introduce a testable dependency plan for owned startup, persistent attachment and external Wayland attachment.
- [x] Require only tools actually used in each mode; report optional capability dependencies separately. Do not assume `hyprctl`, `pactl` or `renderD128` is universally required.
- [x] Honor valid same-UID `XDG_RUNTIME_DIR` and exact `WAYLAND_DISPLAY`; validate selected socket type/owner and retain ambiguity errors.
- [x] Use isolated PATH/runtime fixtures for missing tools, custom runtime, non-sockets, foreign owner and ambiguous sockets. Do not mutate global test process environment.
- [x] Run focused tests and independently review.

### Task 2: Native Wayland capability gate

**Files:** focused new capability module, `src/wayland.rs`, `src/server.rs`, unit and UnixListener fixture tests.

- [x] Define a serializable report for capture, keyboard, pointer and selected-output pointer; list missing interfaces/versions.
- [x] Match actual libwayshot requirements, including ext capture path precedence and xdg-output v3. A registry probe is preliminary; first actual capture remains authoritative.
- [x] Probe an explicit socket using existing connection/deadline/shutdown patterns; close the dedicated connection on timeout and join the worker.
- [x] Apply the same gate to discovery and actual selection; avoid blocking the async runtime or repeatedly probing every frame/connection status update.
- [x] Test complete wlr/ext paths, incomplete ext precedence, missing/old globals, pointer v1/v2 and a server accepting without responding.
- [x] Run focused tests and independently review.

### Task 3: Correct Hyprland rotated resize and preserve layout

**Files:** `crates/display-control/src/hyprland.rs` and its Unix IPC fixtures.

- [x] Add failing tests for odd rotation transforms in both legacy and Lua command paths.
- [x] Invert the logical-to-native dimension swap before applying the mode.
- [x] Preserve the selected output's x/y coordinates rather than requesting `auto` placement.
- [x] Test all eight transform values and negative coordinates; retain final applied-state confirmation and physical authorization.
- [x] Run display-control tests and independently review.

### Task 4: Integration, truthful support matrix and delivery

**Files:** bilingual README, focused Linux compatibility document, version/release notes when ready.

- [x] Document distribution vs desktop vs protocol distinctions, same-UID service boundary and actual native/Portal/X11 support.
- [x] Integrate the three tasks; run targeted tests, formatting and Clippy, plus existing frontend/installer guards when affected.
- [x] Obtain an independent integrated review and resolve findings.
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

## Execution tasks after research approval

The user resumed implementation after the research checkpoint. Automatic backend
selection is required, while the existing default owned virtual desktop remains
the deployment contract. Selecting a physical desktop automatically resolves its
protocol; explicit attach targets remain supported for service configuration.

- [x] Move Frame/InputCommand/InputGuard and release barriers into the common backend contract; migrate Web/MCP/RDP consumers and Wayland implementation without changing input cancellation semantics.
- [x] Resolve exact Wayland/X11 session targets; validate same UID, local display and private authority, preserving owned-session defaults and XWayland precedence.
- [x] Implement capability-based automatic selector with non-interactive probes, explicit consent-required state and no fallback after consent denial/cancellation.
- [x] Implement native X11 capture, input/text and clipboard using x11rb plus mature Rust input/clipboard libraries in a session-scoped child; validate under isolated Xvfb.
- [x] Implement shared Portal RemoteDesktop/ScreenCast session, PipeWire capture, input, first-frame readiness, cancellation and cleanup; probe without prompting and keep libraries optional where needed for deployment compatibility.
- [x] Complete doctor dependency classification, native Wayland registry gates and owned Hyprland rotated resize/layout fixes.
- [x] Add physical lock-state input/capture gating and lifecycle recovery without automatic unlocking or host policy changes.
- [x] Run focused Rust tests, isolated backend acceptance tests, formatting/Clippy, and an independent integrated review; update the bilingual support matrix with actual boundaries.

## Implementation receipt (2026-10-23, before final delivery)

- The research checkpoint was completed in `c74a5ab`; subsequent scope includes
  native X11 and optional Portal backends rather than desktop-name placeholders.
- Exact session/authority selection and mode-aware diagnostics are implemented.
  The configured Wayland attachment takes precedence over ambient displays and
  never falls through after validation fails; its regression was red before the
  fix, then all 14 session tests passed.
- Native Wayland registry checks use an explicit socket and bounded shutdown.
  Hyprland resize/layout fixes were independently reviewed, passed display tests
  and were committed/pushed as `54f91e1`.
- Two private authenticated Xvfb servers passed capture, input, Unicode,
  clipboard ownership, cancelled queued clipboard writes and display isolation.
  The silent-server probe met its deadline. That Xvfb cannot resize to 96×72;
  the live RandR resize check remains explicitly skipped.
- Portal unit tests cover pixel layouts, logical pointer coordinates, input and
  initialization cancellation. Clipboard subscriptions are established before
  Start, and cancelled requests cannot execute queued work or retain their text.
  GNOME/KDE hardware and fractional-scaling acceptance have not been performed.
- The production Rust router/browser check confirms the About logo loads at
  48×48 under CSP. Transparent directional triggers passed four-edge dragging,
  menu placement, fullscreen and focus checks; 97 Node and 37 component tests
  passed after the latest frontend build.
- The final integration review has no remaining P1/P2 findings. The privacy,
  authorization, queued-input and selection regressions have been addressed.
  Full-feature workspace tests passed (240 tests, 21 ignored); full-feature and
  feature-off Clippy passed with warnings denied. Feature-off workspace tests,
  formatting and installer/AUR guards also passed.
- An isolated FreeRDP client passed TLS pinning, authentication rejection, bitmap,
  input, resize and reconnect checks against the current implementation.
- Commit/push, remote CI, merge and tag-driven GitHub/AUR publication remain
  delivery steps. Production installation remains the user's responsibility.
