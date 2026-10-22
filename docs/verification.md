# Verification on LightOS

## Runtime platform extraction — 2026-10-22

Baseline origin/main `050fbe1` preserves mobile/IME controls and high-density
resize. `display-control`, `media-codec`, and `stream-scheduler` are independent
workspace crates; version 0.1.8 adds persistent codec/scheduling choices.
Distribution/configuration initialization is separate from session environment
and actual display-protocol probing. GNOME (zbus), KDE (kscreen-doctor), wlroots
(generated Rust Wayland bindings), Hyprland IPC and X11 (x11rb-async) adapters
have explicit session coordinates. Physical-session authorization remains
read-only. See `docs/runtime-platforms.md` for backend-specific restrictions.

Current local checks: workspace tests (including installer fixtures), bundled-JPEG
workspace tests and Clippy with warnings denied passed; frontend build/typecheck,
80 Node tests and 15 Vitest tests passed. Display tests include explicit Lua
fallback, IPC deadlines, a same-pixel/different-scale change, Mutter multi-output
configuration and scale restrictions, KScreen JSON/rotation/mode-ID selection,
missing session/tool failures and physical authorization for all backends.
The isolated native Lua test exercised real decoded resize/scale frames across
two transport generations: the same Ghostty process and compositor signature
survived, 225 native bindings were active, and configerrors was empty.

GNOME, KDE, wlroots and X11 adapters have not been exercised against real desktop
sessions on this Hyprland host. Their fixture/probe tests are not real desktop
validation. No hardware codec, additional OS, WAN throughput or end-to-end latency
claim follows from this extraction. Independent review found a Mutter global-scale overrestriction and explicit
Hyprland physical-query signature restriction. Both regressions failed before
fixes and passed afterward; the reviewer confirmed no remaining blocker.
An isolated mock Mutter DisplayConfig service also passed actual zbus read-only
probe and verify/temporary-apply roundtrips. A release candidate attached to the
existing owner passed real H.264 configuration/keyframe negotiation, JPEG image
decoding and all three scheduling strategy reports over WebSocket at 2560x1440.
The candidate served JavaScript identical to the production frontend build.
These are protocol tests, not browser-render or other-desktop validation.
Local deployment completed at 2026-10-22 13:15 UTC using an independent
`systemd-run` installer task (exit 0). `main` includes implementation `3e3e20c`
and reviewed fixes `88097df`, both pushed. The installed version is 0.1.8,
SHA-256 `0fe587ae7130b372c2126aeeaf5841d1be0113577401825b1551c6cb5b872ba2`,
identical to the tested release binary. Installer backup:
`/var/lib/lightos-remote-desktop/transaction.WZEjdVfRoU`.
The transport is active/ready with PID 3303722 and NRestarts=0. The existing
session owner PID 3137346 and compositor signature remained unchanged; v2rayN
PID 3194041 survived, and compositor configerrors is empty. Final ordinary and
bundled-JPEG workspace runs each passed 98 tests; the optional real Lua/Ghostty,
private mock D-Bus and release WebSocket tests were run separately and passed.
No new browser-rendering, physical-phone or non-Hyprland desktop claim is made.


Date: 2026-10-22 (machine local date). This records a verified initial native
implementation, not complete Selkies or Omarchy feature parity.

## Mobile input and shortcuts (remote-mobile)

The mobile branch adds an explicit keyboard toggle pinned at the left of the
first shortcut row, one-shot modifiers and a two-row WebShell-inspired bar.
With the keyboard closed, fit mode lays out the bar directly below the canvas;
the open-keyboard and 1:1 layouts reserve a separate visible area for it.
VisualViewport geometry positions the bar above the keyboard without resizing
the remote desktop. Touch supports taps, dragging, hold-to-right-click and
two-finger scrolling, with cancellation and input/composition cleanup.

77 Node regression tests and 10 Vitest tests passed, including seven focused
mobile-controller cases. The frontend typecheck/build, bundled-JPEG release
build, whitespace checks and specification validation passed.

Real Chrome with a 390x844 touch viewport exercised the separately running native
candidate, then rotated to 844x390. Proxy-input text `中文mobile🙂` was typed into
the candidate's terminal and verified through its output file. A sticky Ctrl+L
chord, retained input focus, keyboard toggle, touch drag/right-click/scroll and
the keyboard-height simulation passed. The keyboard-height simulation emitted
no remote resize request. With the actual 1920x1080 HDMI-A-1 source in portrait,
the canvas bottom and bar top both measured 479.1875 CSS pixels, without page
horizontal overflow. The first keyboard button stayed at x=6 CSS pixels.

This is browser touch-viewport evidence, not a physical-phone keyboard test.
The user will manually verify OS keyboard presentation, IME and safe areas on
their phone. The validated candidate SHA-256 is
`df63ea9a660ad1b446e9e5d2fa4921a969811c0f72943bdf19e54bd68d2875e4`.
It was installed on the local service after candidate validation. The installer's
health/stability gate passed with zero automatic restarts, and the authenticated
served JavaScript matched the current build byte-for-byte. Physical-phone manual
verification remains pending.

### Mobile density and floating-entry follow-up

Automatic mobile sizing previously captured one pixel per CSS pixel, causing
upscaling on high-density screens. The new `resize_scale` capability lets the
client request higher capture density together with compositor scaling, bounded
by the existing 3840x2160 encoder limits. Legacy requests omit scale and retain
scale 1. Physical desktop resolution and scale remain protected.

The focused browser checks used the current built frontend with the identical
candidate backend. Capture/output observations were 780x1476 at scale 2 for a
390x844/DPR2 viewport; 970x1840 at scale 2.5 for a 390x844/DPR3 viewport; and
1080x1602 at scale 3 for a 360x640/DPR3 viewport. Logical dimensions stayed near
the usable CSS viewport. A desktop pointer context retained 1280x800/scale 1.
Earlier in this candidate run, Unicode terminal input, keyboard geometry and
normalized touch coordinates at scale 2 passed through the real input pipeline.

The floating entry is transparent, ignores pointer input and is removed from
tab/accessibility navigation while the bar is shown; its internal focus anchor
remains for Headless UI. The bottom menu opened interactive settings and closed
successfully for all three touch density cases. The desktop floating entry stayed
visible. Native smartphone keyboard/sharpness judgement remains a manual check.
77 Node tests, 12 Vitest tests, 57 Rust unit tests and the installer fixture passed;
clippy, release build and specification validation passed. Release
`9560d5ae22f9813b1844af8666f98cde9e5713cac11e77819475d1de41c6b240`
was installed locally and passed the health/stability gate with zero restarts.

### Rebase onto native desktop/session support

The mobile changes were rebased onto main `b57675d` (v0.1.7), retaining native
configuration loading, persistent session ownership and attached-installation
handling. Legacy monitor keywords and the Lua `hl.monitor` fallback now use the
same validated dimensions and scale, including high-density mobile requests.

The explicit native Lua session test decoded real resized frames at scale 2
and 2.5 and checked the actual output scale. The same GUI process and desktop
signature survived two transport generations; 230 native bindings remained
active with no configuration errors. This test owned its isolated session and
transport processes. Frontend tests, bundled-JPEG workspace tests, workspace
Clippy, formatting, 40 installer fixtures and 13 compatibility fixtures passed.

## Desktop-switch close-handshake regression (2026-10-22)

The actual authenticated HTTPS gateway reproduced the reported delay: the old
socket's `close()` was called at browser monotonic time 26324 ms, but the next
physical WebSocket was not constructed until 86334 ms (60010 ms later). Its ready
message arrived at 86478 ms and first canvas draw at 86620 ms. The server-side TCP
socket was already in FIN-WAIT-2 during the wait. The close-acknowledgement test
against the installed pre-fix binary failed with `ResetWithoutClosingHandshake`.

The fix explicitly flushes both media sockets' Close responses, cancels abandoned
initialization, moves worker cleanup off the async executor, and bounds browser
close waits and failed retries. The frontend has regressions for stalled closing,
late callbacks, manual recovery, initialization timeout and retained error reasons.

The separate release candidate passed ten real Chrome switches using the toolbar
and the actual HDMI-A-1 physical display. Physical first-draw times were 287.0 to
459.2 ms; virtual first-draw times were 179.7 to 601.1 ms. Close acknowledgements
arrived within 17.3 to 120.8 ms, all with code 1000. These are localhost browser
measurements, not a guarantee for external networks. The live desktop protocol and
independent PCM channel tests also passed on the isolated candidate.

Validation: 74 Node frontend/media/preference tests and 3 React tests passed;
55 Rust unit tests and the installer fixture passed, as did clippy, formatting,
the explicit ten-second compositor timeout, and specification validation.
The bundled-JPEG release build uses the existing local NASM build tool.
`spec/desktop/switching/AC.md` records the confirmed recovery contract; its scenarios
remain `Profile: draft` because this project has no Project AC Executor mapping.

The candidate also passed a real WebSocket cancellation test against an isolated
compositor and a deliberately unresponsive physical-discovery socket: cancellation
receives its Close acknowledgement within one second, concurrent controllers are
rejected, and virtual control becomes available after bounded cleanup.

After installing release SHA-256
`4e635649a9aacf0ae4c13e62d3db251cc39706a564dbad3f71975f4110f284c0`,
the actual authenticated HTTPS entry passed ten alternating toolbar switches.
Physical first-draw times were 2246.4 to 2476.7 ms; virtual first-draw times were
2073.5 to 2169.1 ms. All closes returned code 1000. The remaining approximately
2003 to 2027 ms was spent completing the old transport's close through the gateway;
server logs show the Close response and controller release finishing before this
wait. This external path does not meet the candidate's localhost one-second target;
it removes the reproduced 60-second stall while retaining a separately measured
gateway close delay. The installed service passed the installer's health/stability
gate with zero automatic restarts; its rollback transaction was preserved.

### Fast control-release follow-up

The follow-up advertises a control-release capability. The server acknowledges
release only after input/media processing ends, held virtual input has completed
a compositor roundtrip, transient workers stop and the controller gate is freed.
The client can start its next connection on that acknowledgement while the old
transport closes in the background. Legacy clients and servers retain the prior
WebSocket-close path. Release timeout, rapid source changes and stale acknowledgements
have focused frontend regressions.

77 Node tests, 3 React tests, 56 Rust unit tests, the installer fixture and clippy
passed. Three explicit candidate desktop tests passed, including ten immediate
alternating virtual/physical acquisitions while retaining all previous client
transports without sending Close. Existing clean-close and abandoned-initialization
tests also passed against the actual HDMI-A-1 target and isolated compositor.
The bundled candidate SHA-256 is
`293d44213ef463cdae7319e2351b1cc137a2863a6a7f02a6b8aa37eb1bbf3b57`.
The user confirmed fast switching in manual testing of the installed HTTPS entry.
No numerical timing was recorded for that manual check; the previous section's
gateway measurements refer to the preceding release.

## Cursor, keyboard and toolbar regression checks

The interaction fixes were checked locally on 2026-10-22 with two isolated
Hyprland sessions and native Wayland Chrome. The existing compatibility patch
was built privately against this machine's Aquamarine 0.15.0 for the candidate
sessions; the system library and installed service were not changed.

- `npm run build` and `npm test` passed (70 Node tests and 3 React tests).
  `cargo test --locked`, `cargo fmt --check` and
  `cargo clippy --locked --all-targets -- -D warnings` passed. Tests requiring
  separate audio/desktop environments remain explicitly ignored by Cargo.
- Native Chrome ran in the isolated client compositor and connected to the
  other candidate's actual desktop. Keyboard permission was granted for that
  local test origin. Super+Enter was injected through the client's virtual
  keyboard, so it passed through the compositor rather than a browser test
  API: only the remote terminal count increased (client 1, remote 1 → 2).
  Ctrl+Alt+Shift+Esc released capture; the next Super+Enter increased only the
  client terminal count (1 → 2). Clicking the windowed desktop did not enter
  fullscreen. This verifies native Wayland capture, not XWayland suppression.
- A native browser in the remote session switched between default, text,
  grabbing and hidden cursors over an otherwise unchanged page. Each change
  produced a distinct actual server JPEG. The text and grabbing captures were
  visually inspected. Cursor shape and motion are carried by video, with its
  associated latency; no independent cursor-shape channel is claimed.
- Real Chrome/React pointer drags at 1280×720 and 375×812 retained the pointer
  offset across delayed layouts, docked on release and did not open the panel
  after a drag. The narrow panel keeps its drag handle accessible above it.
- Focused regressions cover manual fullscreen events, fullscreen ancestors,
  capture retry after denial, captured Super+Enter/Ctrl+V, cursor capability
  fallback, and React layouts during a drag. Manual fullscreen-event and
  capture-retry regressions were observed failing before the keyboard fix.

After these checks, a release build with `--features bundled-jpeg` and NASM
was installed through the executable's transactional `install-service` command.
The local service remained active with zero restarts; its health PID matched
the new MainPID, its executable matched the release build, and its served
JavaScript/CSS matched the rebuilt frontend. The previous release was retained
for rollback. The user subsequently confirmed that testing passed.

## Latency instrumentation follow-up

The floating toolbar now samples actual canvas draws per second, codec and
resolution, and application WebSocket ping/pong RTT. P95 covers the last 30
completed replies. Browser monotonic timestamps avoid cross-machine clock
subtraction. Missing replies time out after five seconds; disconnect clears
the samples. Static-frame suppression can legitimately produce zero idle FPS.
RTT includes application queueing, not input-to-display latency.

The 23 frontend tests pass, including matched/stale replies, a drawn JPEG,
timeout and disconnect cleanup. Rust tests (29 unit tests plus installer),
fmt, clippy and bundled release build pass. The candidate live protocol test
passes with a real ping/pong ID check. Chrome displays H.264 and the toolbar
metrics on a real Omarchy desktop (local RTT 0..4 ms, not a WAN benchmark).
The binary self-installer successfully activates this instrumentation update.

### Delayed-ACK throughput regression

An isolated 1280x720 Omarchy candidate displaying FFplay's 30 FPS `testsrc2`
reproduced the stop-and-wait bottleneck. A WebSocket test delays each render
ACK by 120 ms without blocking its reader. Four-second runs measured:

| Codec | Previous single-frame gate | Bounded adaptive window |
| --- | --- | --- |
| JPEG, 120 ms ACK delay | 8.50 FPS | 19.74 FPS |
| H.264, 120 ms ACK delay | 8.00 FPS | 17.74 FPS |
| JPEG, no injected ACK delay | 29.99 FPS | 30.24 FPS |

This is synthetic ACK delay on localhost, not a bandwidth-limited WAN benchmark
or input-to-display latency measurement. The user's actual external path still
needs validation; this establishes a reproducible throughput defect, not that
all perceived latency is fixed.

The window starts at one, then uses send-to-render-ACK elapsed time divided by
the configured frame interval, rounded up and capped at three. Legacy clients
remain single-frame. Only an ID in the current outstanding set can advance the
cumulative ACK: this permits JPEG's latest-pending-frame replacement without
accepting stale codec-generation ACKs. Codec changes and H.264 encoder recreation
clear outstanding credit and require a first-frame ACK again. Raw frames are
sampled only when credit is available; encoded H.264 delta frames are not dropped.
The browser has one token/session-scoped decode watchdog for all outstanding
frames, retained after partial output and cleared after the last output.

Reproduce on a dedicated candidate with continuously changing content:

```sh
LIGHTOS_REMOTE_DESKTOP_TEST_URL=http://127.0.0.1:18769/ \
LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD="$CANDIDATE_PASSWORD" \
cargo test --locked --features bundled-jpeg --test performance \
  video_throughput_with_delayed_render_ack -- --ignored --nocapture
```

Set `LIGHTOS_REMOTE_DESKTOP_TEST_CODEC=h264` to exercise H.264; the test also
decodes the collected Annex-B stream with FFmpeg. Set
`LIGHTOS_REMOTE_DESKTOP_TEST_ACK_MS=0` for the no-delay control. The assertion
requires at least 15 FPS, intended for the default 120 ms and zero-delay cases.
DDIA's bounded backpressure, monotonic timing and tail-latency principles inform
diagnosis, not adding database/log infrastructure to the media hot path.

## Automated checks

v0.1.1 follow-up: 35 Rust unit tests pass (two environment-specific tests ignored
by default), 37 frontend tests pass, and the CLI installer fixture, 40 installer
behaviors, 13 compatibility behaviors, fmt, clippy and bundled release build pass.
The explicit FFmpeg encoder regression and live clipboard/audio protocol pass.
Two existing isolated outputs validate selection, per-output cursor centers,
physical resize rejection and return to the retained virtual desktop. Additional
runs cover a 90-degree rotated output, negative positions and 1.25x scaling.
Real Chrome switches virtual to attached output, changes to a 720x1280 portrait
output using H.264, then returns to virtual with resizing restored. At a 375x812
viewport, the toolbar remains in bounds without horizontal overflow. This does
not validate actual physical GPU/display hardware; this host has no physical
Wayland login socket. The unavailable physical-source state remains intentional.

Application-launch regression: production Ghostty menu activation failed with
`Gtk: Failed to open display`, while direct Wayland execution loaded OpenGL
4.6. An isolated v2rayN profile reproduced `Avalonia.X11: XOpenDisplay failed`
with no DISPLAY. These are display-environment failures, not evidence that GPU
access is unavailable. The compositor now initializes only its private D-Bus
activation environment before launching the panel; its discovered Xwayland
DISPLAY is also supplied to the panel's application launchers. No host user
systemd environment import or per-application exception is used. In the real
candidate, Ghostty launched through gtk-launch/D-Bus and v2rayN with isolated
HOME both had mapped Xwayland windows, confirmed through Hyprland IPC. A fixture
checks environment propagation, absent DISPLAY and failure before completion.

Keyboard capture follow-up: 30 frontend tests pass, including all-key lock
requests, permission denial, unsupported browsers, late permission after
disconnect, recovery after pending permission loses focus, automatic reconnect,
emergency escape, focus loss/fullscreen exit and arbitrary chord
ordering. Chrome successfully granted Keyboard Lock in fullscreen on the
candidate. Its generic composer sent Super+Digit2 and Hyprland IPC confirmed
workspace 1 changed to 2. This does not verify physical client OS shortcut
inhibition; compositor-reserved keys may still be intercepted before the page.
Rust unit tests, fmt/clippy, bundled build, installer fixtures and explicit
FFmpeg integration are release gates, not evidence of WAN latency resolution.

Toolbar docking follow-up: 25 frontend tests pass, covering four-edge drag
docking, click suppression, persisted position, viewport resize, keyboard
docking, tap versus drag and cancellation. A real Chrome pointer drag docked
the handle left without opening the panel. All four panel placements stayed
inside a 1280x633 viewport; reload restored bottom docking, and a 360x640
mobile viewport kept the panel within bounds. Rust unit tests and bundled
release build also pass. Panel size changes are observed so expanding its
settings cannot push it off screen. Position storage is browser-local only.

- Rust unit suite: 30 passed; compositor timeout and FFmpeg integration tests
  are ignored by default. Both have been run separately; the encoder produces
  an initial keyframe followed by predictive output without lookahead.
- Frontend: 30 dependency-free Node tests passed.
- Embedded CLI installer: fixture verifies dry-run, source-tree-independent
  self-staging, preserved Python unit/environment and transaction rollback.
- Migration installer: 40 behaviors passed with isolated filesystems and mocked
  systemd. Compatibility installer: 13 behaviors passed.
- `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  shell syntax checks and the release build passed.
- The explicit live protocol test passed against the installed service using
  its existing environment file, credentials and URL prefix. Credentials were
  loaded by systemd, not copied into source or command arguments.

The live test covers HTTP authentication, same-origin WebSocket enforcement,
single-controller exclusion, real JPEG decode, invalid-input rejection, Unicode
text clipboard, audio packets, real resolution change, reconnect, restoration of
initial geometry, and 15 seconds of post-resize stability.

## Browser and resource checks

- Headless Chrome/CDP rendered real frames under a nested prefix and opened the
  WebSocket at that prefix, rather than `/ws` at the origin root.
- Actual JPEG and canvas geometry matched after an 800x600 to 1024x768 change.
- A same-origin iframe rendered frames and connected successfully. Foreign
  origins remained rejected. SAMEORIGIN and CSP `frame-ancestors 'self'` verified.
- Keyboard/pointer press-release, exact Unicode typing, clipboard round-trip,
  nonzero audio scheduling and an idle connection exceeding one minute were
  exercised during browser checks.
- A 61-second post-resize candidate sample retained the same main/Hyprland PIDs
  and healthy responses. Main FDs fluctuated 13..16; Hyprland FDs 73..79. No
  monotonic growth or previously reproduced output-state/pool errors in that
  finite window. This is not a lifetime leak proof.

## Final audio isolation

The final implementation uses its own PipeWire/Pulse/WirePlumber processes,
not the browser shell's host PulseAudio authentication. The profile disables
physical ALSA, Bluetooth and video monitoring. Only the private null sink was
present in the inspected private Pulse server.

The installed service's private monitor captured a deliberately generated
440 Hz tone: 1,158,320 PCM bytes, peak absolute float32 value 0.08837890625.
The synthetic playback explicitly selected the private sink and disabled output
prebuffering; an earlier short/default-device experiment captured silence and
was not counted as a passing routing test.

Stopping only a separate candidate's owned PipeWire child caused the candidate
to exit with failure and removed its listening sockets. This fault injection
was not performed on the production service.

## Installed state

- Unit: `lightos-remote-desktop.service`, description
  `LightOS Native Rust Remote Desktop`, existing user/port preserved.
- `/healthz`: native=true, ready=true; systemd active/running and NRestarts=0 at
  verification. Health PID matched MainPID.
- Running binary SHA256:
  `b789e488d5f3403701e2bb02134cf12589080c0f7c51937c2950e76a2e6d76f3`.
  It matched the local release artifact.
- Managed override: `/etc/systemd/system/lightos-remote-desktop.service.d/90-native.conf`.
- Private Aquamarine library provisioned with license/receipt; system library
  and original distribution unit were not modified.
- Original authentication/environment file compared identical to the first
  private backup. Original Python payload remains under `/opt/lightos/remote-desktop`.
- Initial installation state is retained under
  `/var/lib/lightos-remote-desktop/original`; updates have additional private
  transaction snapshots. Latest rollback selects the previous release, which
  may itself be native after multiple updates.

## Remaining limits

Video now negotiates CPU H.264/WebCodecs with SIMD JPEG fallback; audio remains
PCM rather than Opus. Hardware video encoding, DMA-BUF zero-copy, RustDesk-style
adaptive bitrate and remote cursor-shape streaming are not implemented. Microphone uplink,
image clipboard, file transfer, gamepads, multi-monitor control and full Omarchy
desktop provisioning are not implemented. The actual external LightOS gateway
was not independently exercised; tests covered its preserved local URL prefix
and same-origin embedding behavior.

## Empty desktop startup regression

The initial default config started only the compositor, with no shell or
application. The user reported an apparently blank desktop; `hyprctl clients`
confirmed no windows and `configerrors` was empty. Launching foot produced a
mapped 1238x678 window and a captured, visible terminal. The default config now
starts foot once in minimal mode; Super+Return opens additional terminals. A
regression test failed before this change and passed afterward. This alone did
not satisfy the user's desktop expectation: the Omarchy shell was still absent.

The default auto profile now detects the installed Omarchy Shell and starts it
as an owned, monitored child on the private D-Bus and Wayland socket. Wallpaper,
bar and application menu were captured and visually inspected on a separate
candidate. The shell and its IPC callers must use the same basename spelling
of WAYLAND_DISPLAY; using an absolute socket path for the shell broke menu IPC
despite successful rendering. That mismatch was reproduced and fixed.

Candidate checks confirmed background/bar/menu layer namespaces, successful
shell IPC ping, a mapped terminal launched through the session-local
`uwsm-app -- gtk-launch foot` adapter, no compositor config errors, and passing
live protocol checks including real frames, audio, clipboard and resize
(approximately 15.8 seconds). Private adapters avoid importing the remote
environment into the host systemd manager. They deliberately do not implement
every UWSM option or Omarchy hardware/power/session service. The remote desktop
shares the user's existing wallpaper/theme and app data; it is not a separate
security boundary or a mirror of the physical display.

## Source repository

After the user renewed `gh` authentication, the private repository was created
at https://github.com/ca-x/lightos-remote-desktop. SSH authentication succeeds
when bypassing the unavailable configured local proxy.

## Latency and immersive controls

- The original connection loop awaited large video writes before processing
  input. Input now has an independent receiver; a regression test delivers
  mouse motion/press/release while the control writer is not draining.
- A real candidate switched workspace in approximately 53.9 ms with its video
  render acknowledgement deliberately withheld. This is local input-to-compositor
  observation, not browser-to-screen latency over an external network.
- A representative 1280x720 desktop JPEG benchmark (30 encodes, quality 75)
  measured legacy p50 14.55 ms versus SIMD p50 1.73 ms. Encoded sizes were
  142,788 versus 107,544 bytes. It does not measure total application latency.
- Thirty simulated small-area changes over a real desktop background produced
  3,234,719 JPEG bytes versus 311,551 H.264 bytes (10.38x smaller). The complete
  concatenated predictive stream decoded to 30 frames with FFmpeg. This is a
  bounded synthetic workload, not a general bandwidth or WAN performance claim.
- Unchanged captures refresh health without repeatedly encoding or sending
  video. Pointer moves coalesce per animation frame, but clicks/releases are
  never throttled; the final move is flushed before a click/release. Captures
  now include the remote cursor so text, resize and drag shapes are visible;
  cursor motion is subject to video latency. The local browser pointer is
  hidden when the server advertises the embedded cursor.
- Chrome rendered actual VideoFrame outputs (30 observed, no JPEG draws in the
  instrumented H.264 window), decoded 800x600 then 1280x720 after resize, and
  fell back to JPEG when VideoDecoder was deliberately unavailable.
- Full-page canvas and right floating icon controls were inspected at desktop
  and 375px widths without page overflow. Native 1:1 mode retained 1280 CSS
  pixels, fit mode scaled the canvas, and real browser fullscreen contained
  the floating toolbar. Selected mode and tooltip labels have accessible names.
- A user screenshot exposed that max-width/max-height alone did not upscale
  small desktops. The corrected CSS uses explicit viewport/aspect sizing:
  a 2400x1350 frame displays at 2800x1575 in a 2800x1600 viewport, including
  fullscreen. Both small and larger-than-source viewports must be checked.
- Browser-follow resolution defaults on, is debounced 250 ms, uses bounded even
  CSS-pixel dimensions, and turns off for manual resolution/1:1 mode. A real
  2400x1350 browser automatically received a 2400x1350 H.264 desktop. No physical
  display resolution is changed. DPR is not multiplied into encoding size.
- Independent review caught a stale JPEG ACK potentially releasing a new H.264
  keyframe. Negotiated monotonic frame IDs now bind each ACK to its video packet;
  IDs survive async decode and codec changes. Targeted regressions passed and
  the reviewer found no remaining blocking issue in that fix.

## Self-installation and automated binary builds

The installed update was activated with the binary's own `install-service`
command, not the source shell entry point. MainPID was stable, health ready
and restart count zero afterward. Original configuration/payload remain intact.

GitHub Actions run 39706988219 passed tests and the Ubuntu 22.04 Linux x86_64
build. Its archive/checksum were downloaded, verified and the built executable
ran `--version` and `install-service --dry-run` on this Arch host. libjpeg-turbo
is statically bundled; remaining native system dependencies were resolved.
Tagged `v*` builds have a release-publishing job; no tag/release is claimed to
have been created solely by this verification. Subsequent H.264 changes require
their own final CI run.


## Adaptive streaming 0.1.9 (2026-10-22)

The independently implemented QoS controller follows the RustDesk reference
principles documented in `rustdesk-reference.md`; it is not a replacement of the
whole RustDesk transport. Actual OpenH264 bitrate/FPS updates have native readback,
and every emitted predictive frame remains ordered. Quality first preserves the
configured per-frame budget and trades FPS for bandwidth. The browser persists
this choice and retries wanted connections with capped jittered backoff,
offline/online handling and visible-connection liveness detection.

Review reproduced a slowly rising delay floor (1 ms per fresh sample), a single
1200 ms spike cutting FPS, a skipped final capture remaining stale, and a static
image failing to refresh on policy changes. The fixes require a settled,
non-rising half-window minimum, confirmation of severe feedback, and a single
paced refresh request after native skips or resets. Regression checks failed
under the old behavior and passed after the fixes; independent rereview found no
remaining Critical or Important issue in those fixes.

Fresh local verification: ordinary and bundled workspaces each passed 120 tests
with 16 explicit host-dependent tests ignored; Clippy all targets with warnings
denied and formatting passed. Frontend 84 Node tests plus 15 Vitest tests and the
production asset build passed. Installer 40 and compatibility 13 mocked behaviors
passed. Explicit installed-native Lua and two-transport-generation Ghostty tests
passed: one signature and GUI PID 3556747 survived, 225 bindings and no config
errors. A separate release-binary WebSocket check negotiated all four strategies,
verified actual H264 rate metadata and decoded JPEG. Deterministic static-watch
coverage, rather than residual wire traffic, proves the static refresh behavior.

The same private 1280x720 Omarchy desktop with FFplay testsrc2 at 30 FPS was measured
through a userspace proxy limited to 98304 bytes/s downstream. Each run includes
4 s clean, 12 s constrained, 8 s recovery, 4 s paused downstream and 8 s restored. ACKs
represent complete packet reception, not actual browser decode or drawing.
Application ping is also not an input-to-photon measurement. Full receipts,
including older candidate runs, are in
`benchmarks/2026-10-22-adaptive-streaming.json`.

| Case | Constrained received FPS | Application ping P95 | Whole-run server CPU |
| --- | ---: | ---: | ---: |
| Installed 0.1.8 baseline, run 1 |8.42|347ms|26.9%|
| Installed 0.1.8 baseline, run 2 |8.25|394ms|29.1%|
| Reviewed 0.1.9 Auto |11.25|278ms|27.7%|
| Reviewed 0.1.9 Quality first |8.25|372ms|26.1%|

Every successful run kept the connection and decoded the accumulated H264 with
FFmpeg `-xerror`, without JPEG fallback. Both baseline and candidate survived the
pause, so these results do not prove a lower disconnect rate. CPU differences are
small and uncontrolled. Conservative startup/recovery lowers the 4 s clean and 8 s
recovery means: the reviewed Auto run averaged 26.25 FPS clean and 25 FPS recovery;
its final restored settled window reached 29.33 FPS. Quality first retained 100%
budget in native rate reports; perceptual image quality was not measured.

Reproduce only on an isolated desktop with installed native prerequisites:

```sh
python3 scripts/benchmark-adaptive.py --binary baseline=/path/to/0.1.8 --binary candidate=target/release/lightos-remote-desktop --output-dir /tmp/lrd-adaptive-results --strategy auto
python3 scripts/benchmark-adaptive.py --binary candidate=target/release/lightos-remote-desktop --output-dir /tmp/lrd-quality-results --strategy quality_first
```

The release still uses software OpenH264/TurboJPEG. Hardware codecs, actual WAN
and browser end-to-end latency, and complete GNOME/KDE/X11/Windows/macOS desktop
sessions have not been validated by this change.


Local upgrade receipt: an independent `systemd-run` installer exited0 and
installed the tested host-built0.1.9 binary with SHA256
`d25573508a17affb9003203265394ba02a5155c9dc5bda53011b31e6bd1dd79c`.
Transport PID3594533 is active with zero restarts; authenticated WebSocket
handshake and health confirm ready=true and completed H264/JPEG probes. Persistent
owner PID3137346, desktop signature and process start stayed unchanged. v2rayN
PID3194041 also retained its process start; compositor configerrors is empty.
The install backup is `/var/lib/lightos-remote-desktop/transaction.H5xySayyco`.
This is the tested Arch-host build, not the portable Ubuntu-built release asset.


## Optional hardware encoding (candidate, before integration)

The optional native worker was compiled against this Arch host's FFmpeg9 ABI
(libavcodec63, libavutil61, libswscale10) using temporary extracted Clang headers
and libclang; no system packages, driver permissions or user groups were changed.
The portable core does not link FFmpeg. Normal and bundled software workspace
suites passed (126 tests,16 explicitly ignored); native worker tests validate
AVCC-to-Annex-B, RGBA color channels, eight decoded frames and an IDR rate restart.
The color regression fails with BGRA substitution and passes with RGBA. Bounded
worker tests cover missing/malformed/hanging workers, version/trailing data and
process reaping. Strict all-target native Clippy and frontend tests/build passed
(86 Node,15 Vitest); installer43 and compatibility13 behaviors passed.

Actual unprivileged host probes reported NVENC initialization denied, VA-API
renderD128 initialization I/O failure, AMF initialization failure and VideoToolbox
unsupported on Linux. None was advertised as usable hardware. These are negative
device results, not GPU performance measurements.

An isolated native Omarchy owner and two disposable transport bundles passed real
WebSocket negotiation. One bundle used the actual worker; the other used the
explicit `media-codec/examples/failing-worker.rs` fixture to simulate a driver
that disappears after a successful probe. All failed hardware candidates were
excluded and the stream reached OpenH264 before JPEG; manual backend choices,
IDR refresh, actual backend/fallback metadata, profile rejection, software/JPEG
frames and four scheduling policies passed. The fixture is not distributed as
a native encoder. Logs are `/tmp/lrd-hw-{runtime,native-test,workspace-tests,
bundled-tests,native-clippy,frontend-green,installer-green,compat}.log` on this
machine. The final integrated version has a separate receipt below when verified.
