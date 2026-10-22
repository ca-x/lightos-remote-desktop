# Verification on LightOS

Date: 2026-10-22 (machine local date). This records a verified initial native
implementation, not complete Selkies or Omarchy feature parity.

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

The reported user-visible latency remains unresolved. The current single
render-ack window bounds video throughput by round-trip time; this is a code
observation, not yet confirmation of the user's bottleneck. DDIA's bounded
backpressure, monotonic timing and tail-latency principles inform diagnosis;
they do not imply adding database/log infrastructure to the media hot path.

## Automated checks

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
  never throttled; the final move is flushed before a click/release. The browser
  renders a local pointer rather than a cursor baked into delayed video.
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
  the reviewer found no remaining blocking issue in that fix. The local pointer
  deliberately does not reproduce remote text/resize/busy cursor shapes.

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
