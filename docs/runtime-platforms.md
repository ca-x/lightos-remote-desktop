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
change that would modify other outputs. Display management does not prove that
capture or input is available.

`DesktopBackend` owns frames, input, text clipboard, pause state and input-release
barriers. WebSocket, MCP and RDP consume this shared contract. The service first
resolves a trusted, same-user `SessionTarget`, then selects the backend by actual
capabilities; users normally do not select a capture backend.

| Capture/input backend | Requirements | Selection and verification boundary |
| --- | --- | --- |
| Native Wayland | Supported libwayshot capture globals and virtual keyboard/pointer with the required versions | Preferred on a usable selected Wayland socket; a bounded registry probe is followed by real first-frame initialization |
| Portal (`portal` build feature) | One verifiable GNOME/KDE RemoteDesktop + ScreenCast provider, matching compositor/session identities, keyboard/pointer devices, monitor source, embedded cursor, PipeWire | Used when native Wayland protocols are unavailable and the exact target can be proved. Discovery only reads capabilities; an explicit connection opens one shared authorization session and the local chooser selects the monitor |
| Native X11 | Local display, valid authority when configured, XTest, readable root window | `x11rb` core GetImage with a real pixel probe. Input/text and `arboard` clipboard run in an isolated child with explicit display/authority; the parent environment is untouched |

A selected Wayland session never falls back to its XWayland `DISPLAY`. Explicit
Wayland/X11 targets fail visibly if invalid; ambient protocol ambiguity also
requires an exact target. Portal denial/cancellation does not fall through to a
different backend or repeatedly reopen consent. Without RandR metadata, an X11
connection can still capture the root window; a specifically requested monitor
must be resolved successfully. Physical desktop resizing remains forbidden.

The owned virtual desktop remains isolated Cage/Hyprland. GNOME/KDE use the Portal
path only when their installed portal advertises and grants these capabilities,
and one provider can be tied to the selected compositor. The frontend, actual
implementation and Mutter/KWin service identities must match that target; sharing
a UID, runtime or user bus alone is insufficient.

The Portal frontend must share the service's filesystem mount namespace and use
a recognized installation prefix or an explicit descriptor directory. Its
provider registry is checked across the conservative union of descriptor paths,
including competing files with the same basename. Registry files and directories
must predate frontend startup and remain unchanged during validation. Custom or
ambiguous providers, an unknown registry scope, and unavailable identity evidence
make this backend unavailable before consent. Backend owners are pinned through
authorization and checked during the session; a restart ends the grant. Attached
Portal services wait for an explicit browser connection rather than requesting
consent at daemon startup. Portal closure ends that authorization session;
reconnect may need new consent.

Native X11 has isolated authenticated Xvfb acceptance coverage for capture, input,
Unicode, clipboard and two-display isolation. The Portal path has compile/unit
validation, not GNOME/KDE hardware or fractional-scaling acceptance. Neither
backend adds Windows/macOS sessions, KMS capture, image clipboard or DMA-BUF
zero-copy capture. X11 currently uses core image reads with a local browser
pointer and without SHM/Damage optimizations; no performance improvement over
the Wayland path is claimed.

Official Linux binaries keep the Ubuntu 22.04 glibc and PipeWire library
baseline. Their Rust SPA bindings compile against a checksum-pinned, private
SPA 1.0.5 header SDK; stock 0.3.48 headers lack required inline APIs and field
declarations. This replaces build headers only, not the system PipeWire library.
The archive records the SDK source/checksum, linked SDK version and copyright.
GNOME/KDE hardware acceptance remains separate from build compatibility.

Attached physical sessions use logind lock state where an exact local session
can be identified. Locking clears the video, releases input and blocks further
input/clipboard operations while retaining the browser connection; unlocking
resumes on the same backend. A running logind with an unidentifiable or failed
session query is not treated as an unlocked desktop. The service never unlocks a
login session or changes host lock policy. See [desktop switching](desktop-switching.md)
for explicit target options and current platform verification boundaries.

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
