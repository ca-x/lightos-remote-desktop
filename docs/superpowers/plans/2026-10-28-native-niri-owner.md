# Native Niri owner implementation plan

Status: resumed narrowly on 2026-10-29 after the kit and real Pond payload
initialization/frame gates passed. Implement only the native owner seam needed
by the current image goal; no codec/AUR research or unrelated refactoring.

2026-10-29 owner evidence: `c8f317f` / probe `40798343643` started the
actual Niri owner and included foot application, then established an authenticated
transport. Its first ready message incorrectly advertised resize/scale through
automatic wlroots controls, failing the existing behavioral assertion before
colored-frame or two-generation acceptance. Native descriptor-v1 owners and their
attachments now default to query-only Wayland output discovery; no native resize
claim is made without its own implementation and behavioral acceptance. The
probe explicitly requests JPEG before checking fresh red/blue application pixels.
Full owner acceptance and production-image publication remain pending.

Actual owner GREEN: `0fad9c9` / probe `40800711240` passes both authenticated
transport generations with fresh JPEG pixels: red 302772 and blue 302756 at
1272x688. Owner, Niri and foot PID-start identities remain unchanged, native
relative includes remain unchanged, and normal owner stop leaves no live fixture
processes before independent fallback cleanup. The probe also checks the actual
Niri/private-bus PATH. This does not test Pond shell or the platform proxy.
Cancellation and log-alias regressions added at `1be94be` remain independent
production-hardening gates, not grounds to discard the successful owner evidence.

Behavioral RED: `7207f46` / CI `40800945917`, Linux job `123208642450`,
executes both regressions despite the separate formatting failure. Abruptly
killing the publisher leaves the direct activation helper alive; opening the
preplaced log symlink does not fail before truncation. The production fix ties
the direct activator to publisher death with race-checked Linux PDEATHSIG while
retaining group cleanup on normal completion/deadline. Log creation opens without
following symlinks or truncating, validates a private owned single-link regular
file, then truncates the pinned descriptor. These fixes await GitHub GREEN;
PDEATHSIG is not a claim about arbitrary malicious activator grandchildren.

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

- [x] Add tests for default compatibility, explicit Niri selection, missing
  native config, native HOME defaults, relative paths, shell/Aquamarine conflicts and non-Linux
  rejection. Observe actual remote RED.
- [x] Implement validation and Niri doctor dependencies without traversing the
  generic/Omarchy Hyprland initializer. Native includes remain at their source.
- [x] Remote test/fmt/clippy, review, commit/push the accepted interface.

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

- After integrating concurrent upstream performance/resize work without
  overwriting it, 396b6ad/40796928186 reaches the intended actual-owner RED:
  doctor prerequisites pass, then native startup refuses the unimplemented
  owner. Implement the real branch, not a generated Hyprland fallback.
- Strengthen the actual fixture after independent review: subreaper fallback
  runs only in its isolated test process; normal stop signals the main PID
  alone and checks original live identities before fallback. Put required foot
  startup inside the relative include, verify Niri's original --config argv,
  verify foot UID/executable inode and decode only JPEG. Accept no cleanup or
  include-preservation claim based only on a file-content comparison.
- Add actual CLI admission regressions with a private real bus and controlled
  forged readiness: wrong executable, wrong process origin, split kernel
  peers, public record, escaped socket and unknown fields. Native code must
  reject these before publishing descriptor v1; old records are cleared only
  under the exclusive owner lock. These are not real compositor positives.

- Initial actual-owner probe e9c7e22/40795985718 is a real RED, but not the
  intended missing-owner RED: Config incorrectly reserves a 64-byte Hyprland
  signature for a Niri runtime before startup. Fix the compositor-specific
  socket budget rather than shortening the fixture to hide the defect.
  Remote rustfmt output is applied as a source patch, never executed locally.
- Native feasibility remains green in that same run. Numeric origin reports
  Niri PID7609, Cage parent7608, process group7606, session7586: nested Niri
  retains the explicitly launched dbus-run-session root's group. Native startup
  can require its own Cage group's identity, not merely a same-UID process.
  The future direct Cage launch must still verify that relationship at runtime.

- Start the actual owner increment with a GitHub-only native fixture: the
  unchanged publisher is not enough. Require preserved native includes,
  same Wayland/IPC Niri peer, unchanged owner/Niri/application PID-start
  identities and two real transport generations delivering a red then blue
  terminal background, not just changed screenshot hashes or health checks.
  Re-run native feasibility first with numeric process-origin diagnostics to
  learn Cage/Niri process-group behavior before choosing ownership cleanup.
  The intended RED is the existing explicit unimplemented-owner guard.

- Publisher-only GREEN is edf216e/40794000163, Linux test job123185863015:
  both bundled-jpeg and bundled-jpeg/rdp/portal run all twelve process cases
  successfully, including all three malformed UUID spellings; both all-target
  clippy runs, fmt, private FreeRDP/X11 and package/installer regressions pass.
  Independent narrow re-review closes the GUID P2 without a new P1/P2.
  This is not full repository CI: aur-native-cursor still fails against the
  mutable Arch Hyprland ABI; other build/native-host jobs were still running.
- Task 3 is not complete: the record consumer, same-Niri UID/PID/executable/
  process-origin admission, previous-generation cleanup and real private-bus/
  compositor acceptance remain to implement with the actual owner branch.
  The native startup guard still refuses to launch an incomplete owner.
  Stub socket/activator success cannot prove a real bus, capture or persistence.
- Review coverage follow-ups: test owned activator descendants and unrelated
  process-group isolation, concurrent destination creation, and startup
  cancellation when the publisher has its activator in a separate group.
  These are not claimed as covered by the current exec-sleep timeout fixture.

- GUID behavior RED is 68e4abb/40793321370: eleven publisher cases pass and
  the added subprocess case rejects neither the hyphenated UUID nor activation,
  failing with non-D-Bus GUID accepted. Narrow the parsed optional GUID to
  exactly 32 ASCII hexadecimal digits before invoking the activator.
- The preceding 3960f83/40793241737 runs all eleven publisher cases green,
  then clippy reports only cmp_owned at the login-runtime comparison. Apply
  its specific comparison suggestion; this is not full CI acceptance yet.

- Independent publisher review finds a P2: zbus 5.19 delegates GUID validation
  to a UUID parser accepting hyphenated/braced/URN values and preserves their
  spelling, so address round-tripping alone does not enforce the D-Bus
  32-ASCII-hex contract. Add process rejection cases first; observe actual RED
  before narrowing GUID validation. No kernel-peer or native-owner claim.

- Publisher candidate 0ab363e/40792935194 stops at rustfmt before behavior.
  Apply only the reported GitHub formatting diff; no local formatter or test
  runs. Keep the publisher unaccepted until the subprocess suite actually runs.

- Task-3 publisher behavior RED is 52e2030/40790727648: all three process
  cases run and fail for the missing niri-session-environment action, not
  formatting. Implement only the private publisher first. Its controlled
  socket-node/stub-activator fixture does not verify kernel peers or a native
  owner. Preserve the fail-closed owner guard until the consumer is integrated.
- Ruling: refuse pre-existing readiness rather than overwriting it. A future
  lock-owning startup must clear the previous generation before launching Niri
  and independently admit both peers; the publisher cannot prove freshness.
- Sidecar review distinguishes sidebar.toggle from the actual launcher:
  it expands/collapses the sidebar, whereas the audited native Mod+Space
  binding invokes vicinae toggle. Neither ipc-show success nor a changed
  screenshot proves launcher selection or application start. Future real Pond
  acceptance must observe a new verified application window and process.

- Task-3 fixture 5ad6ad6/40789753777 stops at three rustfmt changes before
  behavior runs. Apply the exact GitHub output and observe the publisher's
  actual missing-action RED, not a formatting failure.

- Task 2 interface GREEN 40787218608/2f43e31: Linux test job succeeds including
  169 default unit cases, real CLI subprocess environment/precedence cases,
  bundled-jpeg and bundled-jpeg/rdp/portal tests and all-target clippy. Windows
  and macOS arm64 jobs also succeed; Intel macOS was still running at inspection.
  Independent review has no P1/P2 and its main coverage gaps are addressed.
  Full repository CI is not green because the separate cursor ABI job fails.
- Start task 3 with only the hidden publisher contract. Positive fixtures use
  controlled socket nodes and a stub activator, not a real compositor or bus.
  Its success must not imply peer admission or a persistent owner. Kernel peer
  identity and the consumer are implemented with the actual startup branch,
  avoiding unused production admission helpers. Startup remains fail-closed.

- 43901e3/40786446538 reports only three formatting changes in the new
  subprocess test. Apply the exact runner rustfmt output; no local formatter
  or tests run. No configuration-interface GREEN is claimed before execution.

- Candidate 8d5f19e/40785414507 stops at one remotely reported rustfmt
  assertion collapse; apply the exact diff. Readonly task-2 review finds no
  confirmed P1/P2, but identifies missing boundary coverage. Add default-native
  dependency checks, physical-socket conflicts, both audio choices and actual
  subprocess environment/CLI-precedence plus HOME-less attachment tests.
  Process-stub zero-start and real cancellation/cleanup stay in tasks 3/4.

- Intended task-2 RED is 40784412081/31f3beb: Linux workspace tests report
  exactly five Niri-selection failures for unknown --compositor, with 160
  existing cases passing and seven ignored. Implement selection, native path
  validation/preparation and doctor dependency isolation; preserve Hyprland.
  Add HOME, persistent-attachment, non-Linux and fail-closed startup cases.
  This increment still does not implement native process ownership.

- Initial task-2 run 40783292316/d002900 stops at rustfmt's requested array
  wrapping, before Linux behavior tests. Apply that exact remote formatting
  diff and correct the fixture's existing flag spelling to --desktop-shell.
  This is not the intended Niri-selection RED yet; require the actual test.
- Separately, the unchanged Hyprland cursor companion fails against mutable
  Arch headers (SHyprCtlCommand/render-context API changes). Do not count that
  as the Niri-selection RED or mask the job. Keep it separate from this native
  owner increment and do not claim full repository/release CI is green.

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
