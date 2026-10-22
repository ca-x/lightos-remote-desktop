# Verification on LightOS

Date: 2026-10-22 (machine local date). This records a verified initial native
implementation, not complete Selkies or Omarchy feature parity.

## Automated checks

- Rust unit suite: 21 passed; one ten-second timeout test is explicitly ignored
  by default and was also run separately, passing in approximately 10.1 seconds.
- Frontend: 16 dependency-free Node tests passed.
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
  `b58876b5a1ebd1568d0296cb0376f806d81fe370f36f13170a8621a1387cf0e2`.
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

JPEG/PCM bandwidth and CPU behavior differ from H.264/Opus. Microphone uplink,
image clipboard, file transfer, gamepads, multi-monitor control and full Omarchy
desktop provisioning are not implemented. The actual external LightOS gateway
was not independently exercised; tests covered its preserved local URL prefix
and same-origin embedding behavior.

## Empty desktop startup regression

The initial default config started only the compositor, with no shell or
application. The user reported an apparently blank desktop; `hyprctl clients`
confirmed no windows and `configerrors` was empty. Launching foot produced a
mapped 1238x678 window and a captured, visible terminal. The default config now
starts foot once; Super+Return opens additional terminals. A regression test
failed before this change and passed afterward. This isolated desktop does not
mirror the physical desktop and still does not provision the full Omarchy shell.

## Source repository

After the user renewed `gh` authentication, the private repository was created
at https://github.com/ca-x/lightos-remote-desktop. SSH authentication succeeds
when bypassing the unavailable configured local proxy.
