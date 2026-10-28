# Native Niri persistent owner

Status: resumed narrowly on 2026-10-29 as a direct Pond image dependency.
The earlier deferral avoided unrelated remoted research while finishing kit
and real payload gates. Those gates now include actual Pond initialization,
native sidebar/frame success in 40738943260 and display-bound IPC query success
in 40782166125. Native owner startup itself is not implemented or accepted.
Do not expand codec, cursor, AUR or unrelated platform work.

## Goal

Run the user's Niri configuration and its desktop shell inside an owned,
software-renderable remote session. Preserve that desktop and its applications
across transport reconnects and upgrades. For Pond, the existing configuration
must start `qs -c pond-shell`; an empty compositor is not the delivered desktop.

## Platform boundary

- OCI content is not a runtime bundle. LightOS owns instance materialization,
  device mounts, accounts, authentication, registration and optional VT setup.
- Platform first boot and later remote configuration are separate events.
  `/var/lib/lightos/initialized` does not imply enabled or complete remote
  settings. Preserve both canonical services and environment-file paths.
- Do not inspect real host seed/auth values, replace platform credentials,
  create another registration protocol or import into host user-systemd.
- All builds, dependencies, execution, formatting and runtime artifacts stay
  in GitHub Actions. Local work is source editing, static review and SSH Git.
- This feature does not close numeric-device-GID, full Pond shell, platform
  proxy, audio or optional physical-VT acceptance gates.

## Explicit selection

Add Linux owned-session `--compositor` /
`LIGHTOS_REMOTE_DESKTOP_COMPOSITOR` with values `hyprland` and `niri`.
Hyprland remains the default. Reuse `--desktop-config` for the selected
compositor's native configuration; do not introduce another Niri config flag.

The initial Niri owner requires an existing absolute configuration path.
An explicit desktop-config path is used unchanged. Otherwise resolve only the
ordinary HOME/.config/niri/config.kdl of the non-root owner; require an absolute
HOME and an existing file. This avoids baking a platform-created username into
the image. Preserve that file and all relative includes instead of copying it
into a different directory. Reject Hyprland shell selectors or private Aquamarine
override for Niri. Do not infer Niri from distribution labels, auto-generate a
Hyprland fallback, rewrite the user's home or silently substitute another shell.
Attaching a transport to an already-published owner never starts or replaces it.
Such attachment does not require the owner's startup config to still exist.

## Owned process graph and identity

Retain a private runtime, private D-Bus, optional private audio and one owner
lock. Cage provides a virtual parent display; nested Niri is the actual desktop
and capture target. Do not pass Niri `--session`, which removes the parent display.
This software-rendered fixture is not a headless delivery target.

Select the Niri Wayland socket by a connected socket's peer UID/PID, executable
identity and membership in this startup's process group. Do not select by
lexical socket order, assume a Hyprland signature or capture Cage instead.
Reject stale or unrelated Niri processes. Keep the existing wait-without-reap
ownership discipline until process-group termination has finished.

## Application environment handshake

Use a hidden Rust `niri-session-environment --runtime <absolute-path>` action
as Niri's startup command. It records only an allowlist of compositor-generated
display values, never arbitrary inherited environment or authentication. It
must verify the runtime is private, belongs to the effective user, and matches
`XDG_RUNTIME_DIR`; require the exact private `unix:path=<runtime>/bus` address.

Write a private atomic readiness record containing `WAYLAND_DISPLAY`,
`NIRI_SOCKET` and optional `DISPLAY`. Validate both socket paths remain in the
owned runtime. Update activation environment on that private D-Bus only, using
explicit arguments and no shell or `--systemd`. The parent verifies that both
connected socket peers belong to the same selected Niri before accepting the
record. Old records must not make a new startup ready.
Publish successful readiness only after private-bus activation succeeds; do
not publish first and complete activation asynchronously.

Application launchers retain their isolated Chromium profile and private bus.
The Niri path does not execute the Hyprland IPC/session-environment script and
does not require `HYPRLAND_INSTANCE_SIGNATURE`. Existing Hyprland behavior and
its tests remain unchanged.

## Handoff and capabilities

Reuse descriptor v1 with `signature: null`; do not add fields rejected by old
`deny_unknown_fields` readers. Keep owner PID/start time, private file and
socket checks. An owned Niri session uses its tracked children for liveness;
absence of a Hyprland signature does not turn an owner lock into a pidfd.

Report actual output dimensions and selected Niri output. Do not advertise
dynamic resizing unless a supported, tested controller implements it. The
existing read-only Wayland controller may reject unsupported resize requests
explicitly; requested dimensions are not proof of actual framebuffer size.

## Acceptance

1. Real private Niri capture, pointer motion, left press/release, evdev-key
   press/release and two-way text clipboard work in Actions. Protocol globals
   and successful enqueue operations are not delivery evidence.
2. Config parsing and doctor select Niri without requiring Hyprland,
   Aquamarine or a generated Hyprland terminal config. Invalid combinations
   fail before creating runtime processes.
3. A fake bus, escaped socket path, foreign peer, stale record or missing
   readiness record is rejected. Readiness never stores a fixture auth sentinel.
4. One real owner survives at least two real transport generations. Owner,
   Niri and a GUI application's PID/start identities stay unchanged. Each
   transport receives a fresh Niri frame; cleanup affects only fixture processes.
5. The delivered Pond configuration and actual Quickshell require a separate
   rootfs fixture. Its success cannot be inferred from a wev/empty-config test.

No feature release or desktop-image publication is justified by this document
alone. User-authorized SemVer publication follows full remote CI and review.
