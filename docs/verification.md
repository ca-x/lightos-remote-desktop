# Verification on LightOS

Date: 2026-10-22 (machine local date). This records a verified initial native
implementation, not complete Selkies or Omarchy feature parity.

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
