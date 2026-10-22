# LightOS Remote Desktop

A native Rust browser remote desktop service for Linux/LightOS. The runtime
does not embed CPython or invoke the original Selkies Python launcher.

## Current implementation

- Embedded React/TypeScript frontend with Headless UI and Lucide React icons.
  The appearance section offers Chinese/English/browser language and
  light/dark/system theme preferences, stored locally without reconnecting.
  These themes affect only the controls, not the captured desktop.
- Authenticated same-origin WebSocket client with canvas display and reconnect.
- Optional authenticated TLS RDP access to the same owned virtual desktop, off
  by default with a separate loopback listener. See [RDP setup and limits](docs/rdp.md).
- Optional MCP Streamable HTTP endpoint with separate observe/control tokens,
  one-time token creation/revocation in the tools panel, and screenshot/mouse/
  keyboard tools over the existing owned desktop. Disabled by default; see
  [MCP access and Cua compatibility](docs/mcp.md).
- Drag the floating toolbar handle to dock on any screen edge; its position is
  saved in this browser. Click to open, or focus it and use Alt+arrow keys to
  dock left/right/top/bottom. The panel stays within the viewport.
- Manually entering fullscreen requests all available keys via the browser Keyboard Lock API,
  without application-specific shortcut mappings. It requires browser support
  and permission, generally a secure context; OS/compositor-reserved shortcuts
  may still never reach the page. Focus loss, disconnect or leaving fullscreen
  releases capture and held input. Ctrl+Alt+Shift+Esc releases capture and opens
  tools; the browser's long-Escape fullscreen exit remains available.
  The keyboard section can send any supported physical key with arbitrary
  Ctrl/Alt/Shift/Super modifiers directly to the remote machine, bypassing local
  shortcut interception without changing host bindings.
  Windowed mode stays windowed. Use the page's fullscreen button; browser F11
  fullscreen does not activate this API. Each new fullscreen entry enables
  automatic capture, including when a page ancestor contains the desktop.
  Clicking the fullscreen desktop retries capture after permission denial.
  Captured Ctrl+V uses the remote clipboard; windowed paste transfers local text.
  If Super/Alt+Tab still acts on an Omarchy/Hyprland host, run Chrome/Chromium
  natively on Wayland (`--ozone-platform=wayland`), rather than XWayland, so it
  can request the compositor's shortcut inhibitor. Open the desktop as a
  top-level HTTPS/localhost page and allow the browser's keyboard permission.
  See [Chrome's Keyboard Lock requirements](https://developer.chrome.com/docs/capabilities/web-apis/keyboard-lock)
  and [Chromium's Wayland implementation](https://chromium.googlesource.com/chromium/src/+/main/ui/ozone/platform/wayland/host/wayland_keyboard.cc).
- Floating toolbar shows actual drawn FPS, codec/resolution and application
  WebSocket RTT (latest and P95 of the last 30 replies). RTT includes queueing,
  not input-to-display latency. Static desktops skip duplicate frames, so low
  idle FPS is normal. Measurements use the browser's monotonic clock.
- Native Wayland capture and virtual keyboard/pointer through libwayshot and
  standard Wayland protocols; Unicode text injection and held-input cleanup.
  Captures include the compositor's actual cursor, including text, resize,
  drag and application-defined shapes. The browser hides its local pointer
  over the desktop to avoid displaying two cursors. Cursor motion follows the
  video pipeline's frame rate and latency.
- Isolated Cage/wlroots host with nested Hyprland, a private D-Bus, and owned
  process-group cleanup. No UWSM source modification.
- Persistent encoding (Auto/H.264/JPEG) and scheduling (Auto/Low latency/Smooth)
  choices. Independent display-control, media-codec and stream-scheduler crates;
  see [runtime platform boundaries](docs/runtime-platforms.md) for desktop backends,
  real capability probes and current platform limits.
- Dynamic resolution, text clipboard and optional private PipeWire/Pulse sink
  with browser audio playback.
- Independent audio WebSocket and sender, so video encoding and render ACKs do
  not hold audio delivery. See `docs/audio-channel.md` for lifecycle and limits.
- Applications launched from the desktop receive its actual Xwayland display;
  D-Bus activation is initialized from the compositor's final environment on
  the private bus only, without importing it into the host user systemd manager.
- Supervised health/readiness checks and transactional migration/rollback of
  an existing `lightos-remote-desktop.service`.

**This is an initial native implementation, not complete Selkies parity.**
Video negotiates H.264 through in-process OpenH264 and browser WebCodecs,
with SIMD JPEG fallback when WebCodecs/encoding is unavailable. WebCodecs needs
a secure browser context (HTTPS or localhost); H.264 requires even dimensions.
The default capture cap is 30 FPS. This is CPU encoding, not DMA-BUF zero-copy
or hardware video encoding. Audio uses stereo PCM, not Opus. Microphone uplink, image
clipboard, file transfer, gamepads and multi-monitor control are not implemented.
Do not assume equivalent bandwidth or latency; benchmark your deployment.
Capture reuses a connection for at most eight frames / 250 ms between captures,
amortizing AF_UNIX and Wayland setup while still bounding libwayshot 0.9
protocol-object retention and refreshing output information. JPEG is generated
only when requested; H.264 shares the capture allocation with its worker.
Unchanged images are not repeatedly encoded or transmitted. Each browser
acknowledges rendered frames; input is processed independently of video writes.
Sequenced clients use a render-ACK-timed window of one to three frames; codec
changes and new H.264 encoders wait for their first rendered frame before expanding.
H.264 samples the latest raw frame only when video credit is available, so it
does not discard predictive frames after encoding. Reconnect and resolution
changes create a fresh encoder/keyframe. RustDesk-inspired adaptive bitrate,
GPU capture and remote cursor-shape streaming are not implemented.

## Build and test

Node.js 24.15+ (24 LTS) or 26+ is required only when building from source. Build the locked frontend
before Rust so the executable embeds its HTML, JavaScript, CSS and gzip assets:

```sh
npm ci
npm run build
npm test
cargo test --locked --features bundled-jpeg
cargo build --release --locked --features bundled-jpeg
```

The installed executable needs neither Node.js nor a frontend directory/server.
`npm run dev` previews the UI only; use the Rust server for authenticated desktop
and WebSocket integration. Assets use relative URLs under the existing prefix.

The toolbar's desktop-source selector switches between the persistent virtual
desktop and an existing same-user Wayland desktop. Physical mode lists active
outputs and captures/controls one selected monitor, without changing its resolution.
No physical session means an unavailable option, not a new physical login.
Physical audio is not yet supported. See `docs/desktop-switching.md` for discovery,
nonstandard socket configuration and protocol requirements.

GitHub Actions first runs npm ci, frontend typecheck/build/tests, then Rust and
installer checks. Both test and release-build jobs build the frontend before
compiling Rust. It then builds a Linux x86_64 archive and
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

Touch devices have a two-row shortcut bar with a pinned keyboard toggle. The
bar sits below the desktop image while the keyboard is closed and follows the
visible viewport above the open keyboard. Touch supports taps, dragging, hold
for right-click and two-finger scrolling. See [docs/mobile-input.md](docs/mobile-input.md).

The desktop fills the page. The right-hand icon opens floating controls;
icon tooltips and keyboard focus labels identify actions. Display modes are
fit-to-window, configured resolution at 1:1 with scrolling, and browser fullscreen.
Fullscreen preserves the floating controls; Escape restores the preceding window
mode. Browser-follow resolution defaults on, matching the viewport in bounded
even CSS-pixel dimensions after a 250 ms debounce. Manual resolution or 1:1 mode
disables following. With following disabled, display scaling alone does not
change remote resolution. Clipboard and resolution fields are tucked away.

Use a current stable Rust toolchain. Native linking requires Wayland,
libxkbcommon, libpulse and libturbojpeg 3+, plus a C/C++ toolchain and NASM for
source-built OpenH264 and libopus. Alternatively build with `--features bundled-jpeg`
(requires CMake and NASM), as CI does, to statically bundle libjpeg-turbo.
runtime also needs Cage, Hyprland, D-Bus, hyprctl and wl-clipboard. Audio requires
libpulse, pactl, pipewire, pipewire-pulse and WirePlumber 0.5+. A private audio graph is
created per desktop; no host PulseAudio cookies or default sink are changed.
The WirePlumber profile disables physical audio/Bluetooth/video monitors.
Video encoding and audio capture/encoding do not spawn FFmpeg. Audio uses a
cancellable native Pulse stream and negotiates 128 kbit/s Opus with compatible
browsers, decoded in a Worker and played by an AudioWorklet. PCM remains a
compatibility path. The native four-packet queue bounds backlog to 40 ms for PCM
or 80 ms for Opus; browser transfer and playback queues are also bounded. Hover
FPS/RTT for video-stage P95 timings and audio-buffer diagnostics. See
`docs/audio-channel.md` for protocol, fallbacks and measurement limits.
The current standalone
desktop backend requires access to `/dev/dri/renderD128` for GLES rendering.

```sh
npm ci
npm run build
cargo build --release --locked
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
bash tests/installer.sh
bash scripts/test-install-compat.sh
npm test
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

The default `--desktop-shell auto` detects the installed desktop independently
of the distribution. On Omarchy 4.x it loads the user's native
`~/.config/hypr/hyprland.lua`, or the installed Lua entry if the user has none.
Only when no Lua entry exists does it try the native `.conf` configuration.
Lua validation errors are reported; they do not select a different profile.
Keybindings, themes and window rules come from that native configuration, rather
than a bundled subset. The generated private entry skips the standard physical
monitor/autostart and NVIDIA modules, isolates runtime/state/audio, and restores
the session-local PATH after native settings. Host configs are never rewritten.
Custom commands embedded directly in native Lua remain user-controlled code.

Use `--desktop-shell minimal` for a generic terminal desktop, or
`--desktop-shell omarchy` to require Omarchy. The generic profile checks for an
installed terminal launcher and reports missing prerequisites. An explicit
`--desktop-config FILE` or attached Wayland socket bypasses automatic shell
startup. This is a separate desktop; theme and application data belong to the
same user. Remote app launch uses a private `uwsm-app -- command [args...]`
adapter and browser launcher, without importing the remote environment into the
host user systemd manager.

Installed services separate the long-lived
`lightos-remote-desktop-session.service` (Hyprland, Shell, D-Bus, Xwayland and
audio) from `lightos-remote-desktop.service` (HTTP, capture and input).
Transport updates and restarts reconnect to a private, versioned session
descriptor, preserving GUI windows. The first upgrade from the old combined
service rebuilds the desktop once. Explicitly restarting the session service,
a compositor crash, logout or reboot can still close applications.
See [installation and session lifecycle](docs/install.md#persistent-desktop-session).

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

Optional hardware H.264 encoders (VA-API/NVENC/AMF/VideoToolbox) use a separate
ABI-specific native worker. The portable core retains software fallback and does
not link FFmpeg; see [hardware encoding](docs/hardware-encoding.md) for build,
runtime dependencies and device-validation limits.
