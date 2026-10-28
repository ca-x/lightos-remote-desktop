# Native Niri owner implementation plan

Status: resumed narrowly on 2026-10-29 after the kit and real Pond payload
initialization/frame gates passed. Implement only the native owner seam needed
by the current image goal; no codec/AUR research or unrelated refactoring.

> For agentic workers: use the executing-plans skill task by task. Do not start
> implementation before the real capture/input/text-clipboard prerequisite.

**Goal:** Add an explicit Niri persistent owner without replacing Pond's shell
or changing the LightOS control plane.

**Architecture:** Preserve owner/transport separation and descriptor v1.
Introduce a native Niri startup branch and a private Rust environment handshake.
Existing Hyprland startup stays the default and retains its regression suite.

**Tech Stack:** Rust, clap, Unix peer credentials, private D-Bus, Cage/Niri,
GitHub Actions Arch runners.

**Spec:** `docs/superpowers/specs/2026-10-28-native-niri-owner.md`

## Global constraints

- All executable validation, dependencies and artifacts stay in GitHub Actions.
- Never use the running host's desktop, accounts, services, seed/auth or devices.
- Preserve the two canonical LightOS services, config paths and descriptor v1.
- Commit/push stages over SSH. Observe behavioral RED before each fix.
- Complete the CLI/desktop extension gate before resuming OCI packaging.

## Review focus

- No config rewriting or empty-desktop fallback for a selected native config.
- No Cage/foreign socket selection, stale readiness or owner-lock/pidfd confusion.
- No authentication leakage into readiness, D-Bus activation or diagnostics.
- No host user-systemd import or owner restart on transport-only updates.
- No output-size/resize or full Pond claims from generic Niri feasibility.

### Task 1: Close the feasibility prerequisite

Files: `tests/niri_capture.rs`, `.github/workflows/niri-probe.yml`,
`docs/superpowers/plans/2026-10-28-niri-probe.md`.

- [x] Preserve exact wev receipt regressions and observe real motion/key/button
  success after establishing focus, without accepting `enter` as `motion`.
- [x] Exercise production `DesktopBackend::clipboard_write` against an
  independent `wl-paste`, then independent foreground `wl-copy` against
  `DesktopBackend::clipboard_read`, using distinct Unicode/multiline fixture
  text and the exact private Niri socket. Bound waits and kill owned helpers.
- [x] Record exact reports, full CI, independent review and SSH-pushed evidence.

### Task 2: Explicit configuration and dependency boundary

Files: `src/config.rs`, `src/diagnostics.rs`, `src/desktop/mod.rs`,
`src/session.rs`; adjacent unit tests.

Interface: `Compositor::{Hyprland, Niri}`, `Config::compositor`; preserve
`Config::desktop_config` as the native config path.

- [ ] Add tests for default compatibility, explicit Niri selection, missing
  native config, native HOME defaults, relative paths, shell/Aquamarine conflicts and non-Linux
  rejection. Observe actual remote RED.
- [ ] Implement validation and Niri doctor dependencies without traversing the
  generic/Omarchy Hyprland initializer. Native includes remain at their source.
- [ ] Remote test/fmt/clippy, review, commit/push the accepted interface.

### Task 3: Rust readiness and selected-process identity

Files: new `src/session/niri.rs`, `src/main.rs`, `src/session.rs`; new
`tests/niri_environment.rs` and adjacent identity tests.

Interface: hidden `niri-session-environment --runtime <absolute-path>`;
private readiness remains internal, not descriptor v1.

- [ ] Add process-level negative tests for mismatched bus/runtime, escaped or
  foreign sockets, stale readiness and auth-sentinel leakage. Observe RED.
- [ ] Implement allowlisted atomic readiness and private-bus activation with
  no shell, auth import or host user-systemd. Validate actual UID/PID/exe/group
  and the same Niri peer for Wayland and IPC.
- [ ] Remote tests, bounded failure/cleanup evidence, review and SSH push.

### Task 4: Persistent native owner

Files: `src/session.rs`, `src/session/niri.rs`,
`tests/persistent_session.rs`, `.github/workflows/niri-probe.yml` and docs.

- [ ] Add a real owner plus two-generation transport fixture that initially
  fails because the Niri branch does not exist. Keep actual subprocess IDs,
  PID/start times, fresh frames and GUI identity as observable assertions.
- [ ] Start Cage/Niri without `--session`, invoke Rust readiness as its startup
  command, select the verified Niri socket and preserve user-config shell
  autostart. Set no Hyprland signature and publish unchanged descriptor v1.
- [ ] Exercise startup failure/cancellation, missing readiness, foreign/stale
  socket, transport restarts, owner liveness and owned-process cleanup.
- [ ] Run full GitHub CI, independently review, SSH commit/push. Only then
  plan package/version changes and the separate real Pond shell/device gates.

## Resumption rulings

- Ruling: use only an existing native config, explicit or under the non-root
  owner's absolute HOME. The image cannot know the platform-created username
  at build time; requiring a baked /home/name path would break initialization.
  No missing-file generation or rewritten include tree is allowed.
- Ruling: persistent transport attachment does not revalidate an owner's
  startup config. A live owner must survive transport upgrades even if its
  original config is subsequently moved. Explicit supplied paths remain absolute.
- Ruling: while task 2 exposes selection/doctor only, fail native startup
  closed before spawning anything. Never silently launch Hyprland for Niri.
  Remove this temporary guard only when the real native owner is implemented.

## Evidence

- Initial task-2 run 40783292316/d002900 stops at rustfmt's requested array
  wrapping, before Linux behavior tests. Apply that exact remote formatting
  diff and correct the fixture's existing flag spelling to --desktop-shell.
  This is not the intended Niri-selection RED yet; require the actual test.

- Feasibility clipboard probe 40699256007 and later full source CI40708400112
  are accepted predecessors. The real Pond init/frame probe40738943260 passes;
  probe40782166125 additionally verifies same-peer Wayland selection and an
  actual Quickshell IPC query with exit0. These do not prove persistence.

- Read-only owner audit identifies the existing Hyprland dependencies in
  config/doctor, desktop socket discovery, environment IPC and persistent
  fixtures. Descriptor v1 already permits an absent signature.
- Baseline capture succeeded at `72f5239`/`40689971420`. Receipt predicate RED
  is `0114c31`/`40694410495`; actual focus/motion fixture RED is
  `9d1b57a`/`40695810183`. Current candidate `ab28c28` is not yet accepted.
- `ab28c28` real probe `40697001655` succeeded: actual motion, left-button and
  keyboard delivery plus four regression cases. Text clipboard is the current
  feasibility increment; owner implementation remains gated.
