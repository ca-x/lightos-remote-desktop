# Cross-platform desktop host

## Approved scope

Add Windows and macOS hosts for the current logged-in desktop. Both foreground
`serve` and supervised service execution are required, using the existing
`install-service` command for one-command installation. Keep browser/HTTP,
WebSocket framing, codec selection, scheduler, authentication and Linux behavior.
The user explicitly requested responsibility-based traits with automatic native
implementation selection and existing Rust libraries wherever suitable.

## Contracts

- Keep `DesktopBackend`, `DisplayController`, `VideoEncoder` and Linux
  `DesktopInitializer` as their current responsibility boundaries.
- Add `SessionLifecycle` and `HostPlatform` for host discovery/opening and lifetime;
  a `SessionContext` carries the backend, display controller, audio source factory,
  desktop mode and lifecycle together. Generic callers receive no Wayland socket,
  PulseAudio endpoint, Linux PID token or systemd unit path.
- Add `AudioCapture`/`AudioCaptureFactory`, `PermissionProvider`, `ServiceInstaller`
  and `PrivateStorage` for the remaining native operations. Do not add traits
  with no real caller. Software probes and native operations must report failures
  honestly; the OS name is never evidence that screen/input/audio is usable.
- Keep screen and input session identities aligned. Lock, permission revocation,
  cancellation and user switching clear/pause video and release held input.
- Existing evdev wire key numbers remain compatible and are translated at native
  input boundaries; never pass them directly as Windows/macOS native key codes.
- Physical displays remain read-only; browser fit continues to scale encoded
  pixels. Independent virtual displays, secure/logon desktop control and drivers
  are outside this first host implementation.

## Native execution

Windows foreground capture/input runs in the user's interactive session.
SCM supervision must launch the host in the selected logged-in user session,
not capture from Session 0; child stop is bounded and affects only the host and
its codec children. Installation stores credentials privately and never in a
world-readable service command line. macOS uses the logged-in user's launchd
LaunchAgent; screen recording and Accessibility grants remain OS-controlled.
Permission probes do not open prompts. The existing installation command supports
dry-run and stage-only behavior and retains Linux's established service names.

## Dependencies and reference use

Use the existing enigo/arboard input and clipboard libraries and maintained
capture/native bindings after checking their actual published APIs and licenses.
RustDesk and uu-remote-linux are design references, not sources copied wholesale
into this MIT project. The latter is a Linux host with non-Linux stubs, not a
ready Windows/macOS host implementation.

## Verification

Run Linux unit/frontend checks and isolated Xvfb regression checks for changed
paths; never restart/install/modify the production desktop. Add Windows and macOS
native build/test CI, portable contract tests, denied/missing permission tests,
installer dry-run tests and cancellation/release tests. Foreground/service real
screen and input acceptance requires actual Windows/macOS sessions; compilation
or a stub cannot be reported as a working native desktop. Record any missing
native runtime evidence explicitly. No release tag until required checks pass.
