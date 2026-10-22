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
support, including a separately verified dynamic-bitrate flag. Software OpenH264
supports runtime bitrate/FPS updates with native option readback and preserves
its prediction chain; it does not claim a CRF-quality control. Optional native
VA-API/NVENC/AMF/VideoToolbox adapters use a separate FFmpeg worker; see
[hardware encoding](hardware-encoding.md) for ABI/runtime dependencies, real
probe requirements and device-validation limits.
Its native rate controller may skip a raw input before generating an access unit.
The capture frame retains its lazy shared JPEG cache. `web/src/video-codec.ts`
probes browser WebCodecs decoding support; decoder cancellation/generation and
rendering remain in the client. An encoder being compiled does not prove it
works, and server support alone does not prove browser decoding support.

`stream-scheduler` owns per-connection render ACK accounting, credit windows and
an independent adaptive rate policy. It has no socket/compositor/codec dependency.
The existing Auto window remains1–3 frames; Low latency holds one, Smooth permits
three, and Quality first uses the Auto window. All begin at one credit and reset
on codec/dimension changes. Legacy clients stay at one in flight and fixed rates.

For sequenced clients the new QoS controller learns an ACK-delay floor, confirms
excess delay using spaced fresh samples, cuts encoder budget before ordinary FPS
cuts, and recovers with cooldowns. A sent-frame stall brakes FPS; an idle screen
does not. Capture encoding is paced before encoding, including native skips.
OpenH264 receives a real bitrate/FPS update without a forced encoder restart;
optional hardware encoders explicitly reopen with an IDR on rate changes. JPEG
uses the configured TurboJPEG quality. Quality first preserves the configured
per-frame budget and reduces FPS, scaling H.264 total bitrate proportionally.
This is a budget policy, not a lossless or fixed perceptual-quality guarantee.

The toolbar has persistent encoding (Auto/H.264/JPEG) and scheduling
(Auto/Low latency/Smooth/Quality first) preferences. Auto is the default.
A separate H.264 backend preference defaults to Auto. Validated hardware is
tried before software H.264, and runtime or profile failure excludes that backend
for the connection. H.264 exhaustion falls back to JPEG while retaining the
user's preference for reconnect. Reconnect continues with a jittered capped backoff while wanted, waits
when offline and resumes online; a silent visible connection is replaced after
30s. Manual disconnect cancels retries; explicit desktop-switch release still
has its original failure boundary. See the adaptive-streaming spec/plan and
verification receipt for measured conditions and limitations.
Mobile gestures, IME controls, high-density sizing and logical-dimension limits
continue to use the latest mobile baseline.

RustDesk was read for responsibility boundaries (display service/platform,
encoder factory/runtime probing and VideoQoS). No AGPL source was copied.
Existing Rust libraries supply protocol, codec and browser bindings; project
code contains session policy and adapters rather than new protocol encodings.
