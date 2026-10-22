# Runtime display, media and scheduling boundaries

Linux distribution, desktop session and display protocol are independent:
`platform-detect::Distribution` reads os-release; `OmarchyInstallation` discovers
configuration installed on disk; `desktop-config` initializes the owned session.
Neither decides which runtime display protocol is available. Arch, Ubuntu,
Fedora, Manjaro and Omarchy are not desktop protocols.

`display-control` owns output metadata and the object-safe `DisplayController`
interface. `DisplayMode` includes physical pixels and compositor scale. Session
coordinates are explicit; helpers never inherit the transport process's display,
authority or D-Bus connection. Desktop environment strings are hints only, and
an environment hint is used only if it names the exact selected Wayland socket.
Every backend still performs a read-only protocol query. `--display-backend`
(or `LIGHTOS_REMOTE_DESKTOP_DISPLAY_BACKEND`) overrides the protocol selection:

| Backend | Desktop sessions | Runtime dependency and behavior |
| --- | --- | --- |
| `hyprland` | Hyprland, including Omarchy | Native session-scoped Unix IPC; named output; legacy keyword with explicit Lua fallback; arbitrary valid dimensions/scale; applied mode verified |
| `mutter` | GNOME | `zbus` DisplayConfig calls on the selected bus; checks ApplyMonitorsConfigAllowed; selects advertised mode/scale; verifies before temporary apply, preserves other logical outputs |
| `kscreen` | KDE Plasma Wayland | Executable `kscreen-doctor`, functional `--json` query; uses native output/mode IDs; changes one output and confirms dimensions/scale |
| `wlroots` | Sway, River, Wayfire, labwc when they expose output management | `wayland-client` and `wayland-protocols-wlr`; actual wlr-output-management global required; chooses an advertised mode, tests configuration before applying |
| `x11` | XFCE, MATE, Cinnamon, LXQt and other X11 sessions | `x11rb-async` RandR; explicit local display and optional explicit Xauthority (`xauth` helper); advertised modes, preserved rotation/layout; scale 1 only; transformed and cloned outputs rejected |
| `wayland` | Sessions exposing the existing Wayland discovery protocols | Read-only output discovery |
| `auto` | Any supported session | Owned Hyprland signature first; matching desktop hint next; otherwise probes wlroots. Wayland enumeration fallback remains available when management is absent |

Resize permission is separate from protocol availability: attached physical
desktops remain read-only. A backend does not grant permission to alter them.
A query or update failure includes a reason, and helpers/IPC have deadlines.
Ambiguous output selection requires an explicit name. No backend writes host
configuration files. Mutter refuses mirrored-group changes or a global-scale
change that would modify other outputs. GNOME/KDE/wlroots/X11 backend adapters
exist, but the service's capture/input path is still Linux Wayland with the
required capture and virtual-input protocols; this change does not implement
full GNOME/KDE capture, X11 capture, Windows or macOS remote sessions.

The service's owned compositor remains Hyprland. GNOME/KDE adapters can be
selected for existing Wayland output discovery; their mutation methods are
available to callers supplying an authorized session. The X11 adapter is a
library backend for an explicit `SessionTarget`; an X11 capture service is not
available through the Wayland-only service CLI. New backends implement this
same trait without distribution checks or edits to the WebSocket server.

`media-codec` owns `EncoderFactory` and `VideoEncoder`, current OpenH264 and
TurboJPEG implementations and real test-frame availability probes. Metadata
states backend, hardware usage, availability/failure and dynamic-quality
support. Software OpenH264 does not claim dynamic quality or hardware encoding.
The capture frame retains its lazy shared JPEG cache. `web/src/video-codec.ts`
probes browser WebCodecs decoding support; decoder cancellation/generation and
rendering remain in the client. An encoder being compiled does not prove it
works, and server support alone does not prove browser decoding support.

`stream-scheduler` owns per-connection render ACK accounting and deterministic
latency-based window policy. It has no socket, compositor or encoder dependency.
Auto preserves the existing 1–3 frame algorithm. Low latency holds one frame;
Smooth permits three after the first render ACK. All strategies begin with one
frame and reset on codec/dimension changes. Legacy clients stay at one in flight.
This is scheduling, not adaptive bitrate or measured WAN optimization.

The existing toolbar has persistent encoding (Auto/H.264/JPEG) and scheduling
(Auto/Low latency/Smooth) preferences. Auto is the default. Unsupported H.264
falls back to JPEG while retaining the user's preference for a later reconnect.
Mobile gestures, IME controls, high-density sizing and logical-dimension limits
continue to use the latest mobile baseline.

RustDesk was read for responsibility boundaries (display service/platform,
encoder factory/runtime probing and VideoQoS). No AGPL source was copied.
Existing Rust libraries supply protocol, codec and browser bindings; project
code contains session policy and adapters rather than new protocol encodings.
