<p align="center">
  <img src="docs/assets/logo.png" width="160" height="160" alt="LightOS Remote Desktop logo" />
</p>

# LightOS Remote Desktop

[简体中文](README.md) | English

A cross-platform remote desktop host written in Rust. Connect to and control
Linux, Windows and macOS computers from a browser.

Experimental Windows/macOS physical-desktop hosts share the browser protocol and
provide foreground `serve` and one-command `install-service` execution. Linux
retains its existing virtual desktops and systemd services. See
[native host installation, permissions and acceptance limits](docs/cross-platform-host.md).

## Features

- A compact action toolbar opens contextual display, keyboard, clipboard and
  connection menus; a separate tabbed Settings dialog holds resolution,
  encoding, appearance, MCP and RDP preferences. Its final About tab shows the
  application version, license and project links. The appearance tab offers Chinese/English/browser language and
  light/dark/system theme preferences, stored locally without reconnecting.
  These themes affect only the controls, not the captured desktop.
- View and control the desktop from a browser, with authentication and automatic reconnect.
- Enable/disable TLS RDP and set its port in Settings → RDP, using the same
  owned virtual desktop as the browser. It is off by default and retains the
  loopback bind policy; use SSH forwarding for remote access. Settings shows the
  actual endpoint and TLS fingerprint. A private persistent TLS identity is
  generated when no certificate is configured; explicit certificates remain
  supported. Listener changes preserve HTTP and the desktop owner. RDP quality
  is negotiated with the client, independently of browser encoding/quality
  preferences. See [RDP setup and limits](docs/rdp.md).
- Optional MCP Streamable HTTP endpoint with separate observe/control tokens,
  one-time token creation/revocation in Settings → MCP, and screenshot/mouse/
  keyboard tools over the existing owned desktop. Disabled by default; see
  [MCP access and Cua compatibility](docs/mcp.md).
- Drag the floating toolbar handle to dock on any screen edge; its position is
  saved in this browser. Click to open, or focus it and use Alt+arrow keys to
  dock left/right/top/bottom. Menus open inward, adjust at screen edges and
  remain scrollable in small windows, with hover switching, arrow-key navigation
  and nested choices. Fullscreen is directly available at the right of the toolbar. Settings and menus also work in fullscreen.
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
- The toolbar’s Connection information menu shows actual drawn FPS, codec/resolution and application
  connection RTT (latest and P95 of the last 30 replies). RTT is not
  input-to-display latency.
- Responsive remote cursor with actual shapes, hotspots, scaling and visibility,
  including text fields, links, window edges, dragging and custom cursor images.
- Persistent encoding (Auto/H.264/JPEG) and scheduling (Auto/Low latency/Smooth/Quality first)
  choices. Browser H.264 requires a secure context such as HTTPS or localhost;
  JPEG remains available when H.264 cannot be used.
- Fit-to-window and fullscreen adjust the picture size automatically;
  quality-first and actual-size modes keep native dimensions.
- Dynamic resolution, text/image clipboard and optional private PipeWire/Pulse sink
  with browser audio playback.
- Supervised health/readiness checks and transactional migration/rollback of
  an existing `lightos-remote-desktop.service`.

Microphone uplink, file transfer, gamepads and simultaneous multi-monitor control
are not yet supported.

For rotated outputs, Portal input and dedicated/hybrid GPU limits, see
[desktop and GPU compatibility](docs/desktop-gpu-compatibility.md) and
[encoder device selection](docs/hardware-encoding.md#dedicated-and-hybrid-gpu-selection).

## Linux: AUR installation and updates

The package is `lightos-remoted-bin`; the command is `lightos-remoted` and the
installed executable is `/usr/lib/lightos-remoted/lightos-remoted`. GitHub
binary archives use `lightos-remoted-linux-x86_64.tar.gz`.
The optional AUR package `lightos-codec-ffmpeg-bin` provides a native FFmpeg encoding
worker with hardware H.264 probing. It is prebuilt in Arch from the matching source
tag against the corresponding system FFmpeg ABI and installs beside the main executable at
`/usr/lib/lightos-remoted/lightos-codec-ffmpeg`, where it is discovered automatically.
Built-in software H.264 and JPEG work without this package. Hardware encoding
availability depends on the GPU, driver and FFmpeg capabilities.

```sh
yay -S lightos-remoted-bin
yay -Syu
```

For optional hardware encoding, install the worker:

```sh
yay -S lightos-codec-ffmpeg-bin
```

A fresh installation initializes authentication and starts the services without
manual unit installation or `enable` commands. Configuration and credentials
remain in `/etc/lightos/remote-desktop.env`; existing values are preserved.
The service names remain `lightos-remote-desktop.service` and
`lightos-remote-desktop-session.service` because LightOS setup scripts depend
on them. The binary is authoritative for service definitions and initialization: AUR
exports both units from the executable and calls its lifecycle commands, while
pacman owns the installed files. Package upgrades restart only the HTTP
transport and preserve the persistent desktop owner. See [AUR service and package details](docs/aur.md).

### Migration and cleanup for legacy manual installations

When AUR detects a legacy manual installation, it preserves the current services
and desktop without switching automatically. The following steps are for a
manual migration when you choose to proceed; fresh AUR installations do not need
them.

**Save your work in desktop applications first. Stopping
`lightos-remote-desktop-session.service` ends the desktop applications it manages,
including terminals and v2rayN.**

1. Back up the old configuration, units and rollback records. The backup directory
   is readable only by root; do not paste its authentication data into logs or
   issues.

   ```sh
   backup="/root/lightos-remote-desktop-backup-$(date +%Y%m%d-%H%M%S)"
   sudo install -d -m 0700 "$backup"
   for path in /etc/lightos \
     /etc/systemd/system/lightos-remote-desktop.service \
     /etc/systemd/system/lightos-remote-desktop.service.d \
     /etc/systemd/system/lightos-remote-desktop-session.service \
     /var/lib/lightos-remote-desktop; do
     if sudo test -e "$path"; then
       sudo cp -a --parents "$path" "$backup/"
     fi
   done
   ```

   Back up any custom MCP settings files, certificates or other state directories
   separately before proceeding.

2. After saving your work, stop the old desktop and remove the old local unit
   overrides so that the AUR units with the same names take effect.

   ```sh
   sudo systemctl disable --now lightos-remote-desktop.service
   sudo systemctl stop lightos-remote-desktop-session.service
   sudo rm -f -- /etc/systemd/system/lightos-remote-desktop.service \
     /etc/systemd/system/lightos-remote-desktop-session.service
   sudo rm -rf -- /etc/systemd/system/lightos-remote-desktop.service.d
   sudo systemctl daemon-reload
   sudo lightos-remoted install-service --package-managed
   ```

   The old `90-native.conf` and `95-session.conf` are removed along with the old
   drop-in directory above. The complete AUR units already include the executable
   path and session dependency, so these two files do not need to be reinstalled.
   pacman does not automatically remove these files from a manual installation.
   If the directory contains other custom settings, back them up and migrate
   them individually.

   Keep `/etc/lightos/remote-desktop.env`: LightOS still uses it to manage
   authentication and the enable switch. The new services retain the valid
   nonroot user and authentication configuration in this file.

3. Sign in through the browser to confirm that display, input and application
   launching work, then check the services.

   ```sh
   lightos-remoted --version
   systemctl status lightos-remote-desktop.service --no-pager
   systemctl status lightos-remote-desktop-session.service --no-pager
   ```

4. Remove the old Rust installation directories only after confirming that the
   new version works and rollback is no longer needed. First confirm that the
   service uses `/usr/lib/lightos-remoted/lightos-remoted`, then inspect the
   preserved `/etc/lightos/remote-desktop-native.env`. If it still references a
   compatibility library in the old installation directory, follow the
   [AUR documentation](docs/aur.md) to switch to the package-provided library and
   verify that restarting works before deleting anything.

   ```sh
   sudo rm -rf -- /usr/local/lib/lightos-remote-desktop
   sudo rm -rf -- /var/lib/lightos-remote-desktop
   ```

   **Do not delete** `/etc/lightos`, `/opt/lightos/remote-desktop` or
   `/usr/local/bin/lightos-remote-desktop-*.sh`: they are still part of the LightOS
   platform integration. You can remove the backup directory after confirming
   that rollback is no longer needed.

## Build and test

Node.js 24.15+ (24 LTS) or 26+ is required only when building from source. Build the locked frontend
before Rust so the executable embeds its HTML, JavaScript, CSS and gzip assets:

```sh
npm ci
npm run build
npm test
cargo test --locked --features bundled-jpeg,rdp,portal
cargo build --release --locked --features bundled-jpeg,rdp,portal
```

The installed executable needs neither Node.js nor a frontend directory/server.
`npm run dev` previews the UI only; use the Rust server for authenticated desktop
and WebSocket integration. Assets use relative URLs under the existing prefix.

On Linux, the toolbar's desktop-source selector switches between the persistent virtual
desktop and an existing same-user Wayland desktop. Physical mode lists active
outputs and captures/controls one selected monitor, without changing its resolution.
No physical session means an unavailable option, not a new physical login.
Linux physical-desktop audio is not yet supported. See `docs/desktop-switching.md` for discovery,
nonstandard socket configuration and protocol requirements.

GitHub Actions automatically checks the frontend, Rust and installers, then builds
Linux x86_64, Windows x86_64, macOS ARM64 and macOS Intel x86_64 archives with
SHA256 checksums. ARM64 macOS builds run natively on `macos-15`; Intel builds use
`macos-15-intel`. Each job verifies the Rust host architecture before building.
Pushing a `v*` tag publishes to GitHub Releases only after all platform jobs pass;
`workflow_dispatch` can also run verification. Linux release builds use Ubuntu
22.04 and bundle SIMD JPEG; Wayland, xkbcommon and glibc come from the Linux host.

### Linux: install the downloaded executable

```sh
./lightos-remoted install-service --dry-run
sudo ./lightos-remoted install-service
sudo ./lightos-remoted rollback-service
```

`install-service` uses the new layout by default: it installs the executable,
command entry and both complete systemd units, initializes a nonroot account and
authentication, and enables the services. It preserves existing values in
`/etc/lightos/remote-desktop.env`. No source checkout is required.
Desktop dependencies and any required private Aquamarine
library must already be installed; it does not download packages.

`--dry-run` only shows the installation plan. `--stage-only` installs files
without creating accounts/authentication or starting services. Normal updates
restart only the transport and retain the desktop owner. `rollback-service`
restores the previous standalone file transaction while preserving credentials.
Rolling back the first installation removes service files, so save work and
explicitly stop both services before using `--stage-only` for that rollback.
Standalone installation refuses package-owned files; update AUR installations
through the package manager.

Legacy or custom units under `/etc/systemd/system` are preserved rather than
overwritten or stopped. Follow the migration steps above. To retain the old
transactional layout explicitly, use `install-service --legacy` and
`rollback-service --legacy`.

### Desktop controls

Touch devices have a compact two-row shortcut bar matching WebShell, with pinned
keyboard and LightOS home icons. The bar stays at the bottom of the visible page;
the picture fits proportionally in the area above it. The bar follows the open
keyboard and respects Lazycat client immersion and automatic safe areas. Touch supports taps, dragging, hold
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
source-built OpenH264 and libopus. `bundled-jpeg` statically bundles libjpeg-turbo
and needs CMake/NASM. Official binaries enable `bundled-jpeg,rdp,portal`; building
`portal` additionally requires PipeWire/SPA development headers, pkg-config and
libclang, and the binary links `libpipewire-0.3.so.0` at runtime. Custom builds
without that feature report Portal as unavailable.

For Ubuntu 22.04, the release build uses a checksum-pinned SPA 1.0.5 header SDK
in a private directory, while linking the distribution's PipeWire library.
The stock SPA 0.3.48 headers cannot compile the current Rust bindings. See
[the build helper](.github/scripts/prepare-spa-build-sdk.sh); its exported
`PKG_CONFIG_PATH` selects headers without installing a different runtime.
Release archives include the SDK source/checksum and copyright notice.

Owned virtual sessions need Cage, Hyprland and D-Bus; attaching to an existing
desktop does not start these components. Native Wayland text clipboard uses
optional wl-clipboard helpers; X11 requires xauth and usable XTest input.
`doctor` distinguishes required and optional dependencies for the selected
startup mode; installed programs do not prove capture/input availability.
Audio requires libpulse, pactl, pipewire, pipewire-pulse and WirePlumber 0.5+.
Standalone desktops need access to `/dev/dri/renderD128`.
See [audio configuration](docs/audio-channel.md) for details.

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
target/release/lightos-remoted doctor
target/release/lightos-remoted serve --listen 127.0.0.1 --port 18766
```

For an already-running compositor, use `--wayland-socket /absolute/socket`.
It must expose the required capture, seat and virtual-input protocols. Attach
mode does not own that desktop and cannot resize it.

The default `--desktop-shell auto` detects the installed desktop. On Omarchy 4.x,
it uses the user's or system's Hyprland configuration, including keybindings,
themes and window rules. Configuration errors are reported directly.

Use `--desktop-shell minimal` for a generic terminal desktop, or
`--desktop-shell omarchy` to require Omarchy. The generic profile checks for an
installed terminal launcher and reports missing prerequisites. An explicit
`--desktop-config FILE` or attached Wayland socket bypasses automatic shell
startup. This is a separate desktop; theme and application data belong to the
same user.

Updating or restarting `lightos-remote-desktop.service` preserves the desktop
and its applications. The first upgrade from an older version rebuilds the
desktop once. Restarting `lightos-remote-desktop-session.service`, a compositor
crash, logout or reboot can still close applications.
See [installation and session lifecycle](docs/install.md#persistent-desktop-session).

### Aquamarine compatibility

For environments using Aquamarine 0.15.1, build and use the compatibility library:

```sh
bash scripts/build-aquamarine.sh
target/release/lightos-remoted serve --port 18766 \
  --aquamarine-dir "$PWD/.local/aquamarine/build"
```

The build recipe intentionally refuses other installed Aquamarine versions.
Revalidate rather than bypassing this check after a system update. Build tools
include cmake, ninja and the matching native development dependencies.

## Installation and rollback

Read [docs/install.md](docs/install.md) before touching the live service. The
installer preserves `/etc/lightos/remote-desktop.env`, the original unit and
existing platform files. Native-only settings live in a separate optional env file.
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

## License

LightOS Remote Desktop uses the [PolyForm Noncommercial License 1.0.0](LICENSE)
(`PolyForm-Noncommercial-1.0.0`) for the noncommercial uses permitted by its terms. Third-party components
retain their own licenses; see [third-party notices](docs/third-party.md).
