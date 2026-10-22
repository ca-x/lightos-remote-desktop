# LightOS Remote Desktop

A native Rust browser remote desktop service for Linux/LightOS. The runtime
does not embed CPython or invoke the original Selkies Python launcher.

## Current implementation

- Authenticated same-origin WebSocket client with canvas display and reconnect.
- Drag the floating toolbar handle to dock on any screen edge; its position is
  saved in this browser. Click to open, or focus it and use Alt+arrow keys to
  dock left/right/top/bottom. The panel stays within the viewport.
- Fullscreen requests all available keys via the browser Keyboard Lock API,
  without application-specific shortcut mappings. It requires browser support
  and permission, generally a secure context; OS/compositor-reserved shortcuts
  may still never reach the page. Focus loss, disconnect or leaving fullscreen
  releases capture and held input. Ctrl+Alt+Shift+Esc releases capture and opens
  tools; the browser's long-Escape fullscreen exit remains available.
  The keyboard section can send any supported physical key with arbitrary
  Ctrl/Alt/Shift/Super modifiers directly to the remote machine, bypassing local
  shortcut interception without changing host bindings.
- Floating toolbar shows actual drawn FPS, codec/resolution and application
  WebSocket RTT (latest and P95 of the last 30 replies). RTT includes queueing,
  not input-to-display latency. Static desktops skip duplicate frames, so low
  idle FPS is normal. Measurements use the browser's monotonic clock.
- Native Wayland capture and virtual keyboard/pointer through libwayshot and
  standard Wayland protocols; Unicode text injection and held-input cleanup.
- Isolated Cage/wlroots host with nested Hyprland, a private D-Bus, and owned
  process-group cleanup. No UWSM source modification.
- Dynamic resolution, text clipboard and optional private PipeWire/Pulse sink
  with browser audio playback.
- Applications launched from the desktop receive its actual Xwayland display;
  D-Bus activation is initialized from the compositor's final environment on
  the private bus only, without importing it into the host user systemd manager.
- Supervised health/readiness checks and transactional migration/rollback of
  an existing `lightos-remote-desktop.service`.

**This is an initial native implementation, not complete Selkies parity.**
Video negotiates low-latency H.264 through FFmpeg/libx264 and browser WebCodecs,
with SIMD JPEG fallback when WebCodecs/encoding is unavailable. WebCodecs needs
a secure browser context (HTTPS or localhost); H.264 requires even dimensions.
The default capture cap is 30 FPS. This is CPU encoding, not DMA-BUF zero-copy
or hardware video encoding. Audio uses stereo PCM, not Opus. Microphone uplink, image
clipboard, file transfer, gamepads and multi-monitor control are not implemented.
Do not assume equivalent bandwidth or latency; benchmark your deployment.
Capture currently uses one short-lived connection per frame to contain
libwayshot 0.9 protocol-object lifetime defects observed during real resize
tests. This bounds resources but adds per-frame protocol setup overhead.
Unchanged images are not repeatedly encoded or transmitted. Each browser
acknowledges rendered frames; input is processed independently of video writes.
H.264 samples the latest raw frame only when video credit is available, so it
does not discard predictive frames after encoding. Reconnect and resolution
changes create a fresh encoder/keyframe. RustDesk-inspired adaptive bitrate,
GPU capture and remote cursor-shape streaming are not implemented.

## Build and test

GitHub Actions tests every push/PR, then builds a Linux x86_64 archive and
SHA256 checksum (also available through `workflow_dispatch`). A `v*` tag
publishes these files to GitHub Releases. Release builds use Ubuntu 22.04 and
bundle the SIMD JPEG library; Wayland, xkbcommon and glibc remain system libraries.

### Install the downloaded executable

```sh
./lightos-remote-desktop install-service --dry-run
sudo ./lightos-remote-desktop install-service
sudo ./lightos-remote-desktop rollback-service
```

`install-service` installs this executable and activates the existing LightOS
service, preserving its original unit, Python payload, credentials and URL prefix.
Failure restores the previous transaction. `--stage-only` skips activation.
Rollback restores the previous version, which may already be Rust. The installer
and unit template are embedded; no source checkout is required. Existing native
desktop dependencies and any required private Aquamarine compatibility library
must already be provisioned; installation does not download libraries or packages.

### Desktop controls

The desktop fills the page. The right-hand icon opens floating controls;
icon tooltips and keyboard focus labels identify actions. Display modes are
fit-to-window, configured resolution at 1:1 with scrolling, and browser fullscreen.
Fullscreen preserves the floating controls; Escape restores the preceding window
mode. Browser-follow resolution defaults on, matching the viewport in bounded
even CSS-pixel dimensions after a 250 ms debounce. Manual resolution or 1:1 mode
disables following. With following disabled, display scaling alone does not
change remote resolution. Clipboard and resolution fields are tucked away.

Use a current stable Rust toolchain. Native linking requires Wayland,
libxkbcommon and libturbojpeg 3+. Alternatively build with `--features bundled-jpeg`
(requires CMake and NASM), as CI does, to statically bundle libjpeg-turbo.
runtime also needs Cage, Hyprland, D-Bus, hyprctl and wl-clipboard. Audio requires
FFmpeg, pactl, pipewire, pipewire-pulse and WirePlumber 0.5+. A private audio graph is
created per desktop; no host PulseAudio cookies or default sink are changed.
The WirePlumber profile disables physical audio/Bluetooth/video monitors.
The current standalone
desktop backend requires access to `/dev/dri/renderD128` for GLES rendering.

```sh
cargo build --release --locked
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
bash tests/installer.sh
bash scripts/test-install-compat.sh
node --test tests/frontend.mjs
```

The ten-second startup timeout test is explicit:

```sh
cargo test --locked unresponsive_compositor -- --ignored
```

## Run a separate candidate

Set `LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD` privately to a non-placeholder secret
of at least 12 bytes. Never commit an environment file or pass production
credentials in command arguments.

```sh
target/release/lightos-remote-desktop doctor
target/release/lightos-remote-desktop serve --listen 127.0.0.1 --port 18766
```

For an already-running compositor, use `--wayland-socket /absolute/socket`.
It must expose the required capture, seat and virtual-input protocols. Attach
mode does not own that desktop and cannot resize it.

The default `--desktop-shell auto` starts the installed Omarchy Shell when
available, including the user's wallpaper, top bar and application menu.
Super+Space opens applications, Super+Escape opens the root menu, Super+Return
opens a terminal, and Super+1 through Super+5 switch workspaces. No terminal
covers the wallpaper on startup. This is a separate desktop, not a mirror of
the physical display. Theme and application data still belong to the same user.

Use `--desktop-shell minimal` for the terminal-only fallback, or
`--desktop-shell omarchy` to require Omarchy. An explicit `--desktop-config FILE`
or attached Wayland socket bypasses automatic shell startup. Remote app launch
uses private session-local adapters, not UWSM or the host user systemd manager;
only the `uwsm-app -- command [args...]` form is supported. We do not run
Omarchy's host-session autostart, hardware provisioning or import the remote
environment into the host session. Host network/polkit/power integrations are
not guaranteed in this seatless session.

### Aquamarine compatibility

The tested LightOS image uses Aquamarine 0.15.1. Its nested Wayland backend
overbinds protocol versions and does not flush its initial output commit.
The pinned, reviewable private build fixes these issues without replacing
`/usr/lib/libaquamarine.so.14`:

```sh
bash scripts/build-aquamarine.sh
target/release/lightos-remote-desktop serve --port 18766 \
  --aquamarine-dir "$PWD/.local/aquamarine/build"
```

The build recipe intentionally refuses other installed Aquamarine versions.
Revalidate rather than bypassing this check after a system update. Build tools
include cmake, ninja and the matching native development dependencies.

## Installation and rollback

Read [docs/install.md](docs/install.md) before touching the live service. The
installer preserves `/etc/lightos/remote-desktop.env`, the original unit and
Python payload. Native-only settings live in a separate optional env file.
The migration installer requires an existing service; it is not a distro
provisioner. It does not install packages or download executable dependencies.

```sh
sudo bash scripts/install-compat.sh --check
sudo bash scripts/install-compat.sh
sudo bash scripts/install.sh --check
sudo bash scripts/install.sh --activate
sudo bash scripts/install.sh --rollback --activate
```

Do not activate until real browser capture, input and required features pass on
a separate port. Activation checks a native health response matching the systemd
MainPID and verifies stable restart counts; failure restores the prior release.

## Configuration

Run `serve --help` for the exact CLI/env interface. Existing authentication,
port, enable, screen and URL subfolder variables are consumed without evaluating
shell code. `LIGHTOS_REMOTE_DESKTOP_SESSION` and `SESSION_ID` are not arbitrary
shell execution hooks; this backend explicitly launches Hyprland.

`GET /healthz` is public and returns readiness, PID and transport without
credentials. All desktop, static assets and WebSocket routes require Basic Auth.
Use HTTPS/WSS at the trusted gateway; raw HTTP is for localhost or protected
networks. Preserve Host and the configured URL prefix through the proxy. No
cross-origin WebSocket connections are accepted.

## Provenance

This implementation uses libwayshot, Wayland client protocols, xkbcommon, Tokio
and Axum. The separate Aquamarine patch builds exact official commit
`f31c47a1b9d300847d8fc3108ad959448103dfc1` (v0.15.1). Its upstream license applies
to that library; see `docs/third-party.md`. It is not bundled into the Rust
executable. No pixelflux/pcmflux source or Python wheel is copied into this repo.
