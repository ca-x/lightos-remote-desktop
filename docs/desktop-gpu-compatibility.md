# Desktop and GPU compatibility integration

## Reference revisions

The 2026-10-29 investigation updated UU Remote Linux to
`11097b5847950f7e71221cb22a38c96ead46e406` and consulted RustDesk at
`bd9262397208c509ea97af61f0d44621ef25611c` without changing it. Their GPL/AGPL
implementations are not copied or cherry-picked. The changes here independently
implement applicable behavior using this project's existing code and standard
Wayland, Portal, xkbcommon and FFmpeg APIs.

## Integrated changes

| Area | Behavior |
| --- | --- |
| Rotated Wayland outputs | The reusable image-copy capture path accepts rotated/flipped outputs and normalizes each frame's transform to upright RGBA. All eight transforms are covered; normal frames retain their original Arc without another image copy. Topology changes still reconfigure the capture, and failures retain the screenshot fallback. |
| Portal text input | Canonical xkbcommon keysyms replace handcrafted code-point values. Tab and Return are real keysyms, CRLF produces one Return, Latin-1 uses its canonical symbols and other control characters do not become keystrokes. Input stays in the same authorized Portal session/queue. |
| Dedicated/hybrid GPUs | Host-only VA-API render-path and NVENC GPU-index overrides apply consistently to probes and encoder initialization. Failed pins do not silently move to another adapter. Selected devices survive rate restarts; unsupported old workers fail closed. See [hardware configuration](hardware-encoding.md#dedicated-and-hybrid-gpu-selection). |
| Hardware capability refresh | Linux cache identities now include all discovered NVIDIA nodes, DRM device/driver bindings, driver-version contents, node ownership/change times and device overrides. Actual bounded encode/decode probes remain the only availability proof. |

UU's rotated-output and device-probe work motivated checking these paths.
RustDesk's separation of RAM/VRAM capabilities and adapter identity checks informed
the review of device identity and fallback boundaries. RustDesk's persisted GPU
signature comparison is platform-specific; it is not described here as an
existing Linux solution copied into this project.

## Already covered or deliberately not adopted

- X11 already uses checked x11rb SHM requests with a core-image fallback; UU's
  Xlib process-wide error-handler fix is not applicable.
- Native Wayland input already targets the selected output with virtual-pointer
  v2, and its separate printable-keycode text keyboard covers the relevant Niri
  behavior. No second inverse pointer transform is added.
- Portal already uses one stream's logical coordinates for absolute motion and
  closes authorization/capture sessions on controller teardown. It must not map
  opaque stream IDs to connectors by matching resolution or bypass consent.
- Browser touch gestures already send explicit scroll events. UU's official
  mobile-client convention of replaying mouse drags as touchscreen input does
  not apply; normal mouse selection/dragging is preserved.
- Existing focus-aware clipboard text input retains Ctrl+V/Ctrl+Shift+V. There
  is no Shift+Insert/PRIMARY selection fallback or external `wl-copy` bypass of
  Portal authorization. Arbitrary IME text delivery through Portal remains
  desktop/input-method dependent; keysym correction is not a universal IME fix.
- Unverified packed-RGB NVENC optimizations and GPU capture/encode zero-copy are
  not enabled. Display/encoder adapter affinity is not guessed from device order.

## Verification boundary

Regression tests cover exact non-square pixel ordering, all transforms, source
immutability, singleton axes, invalid frames, Portal text/control mappings,
override validation, legacy-worker rejection, pinned VA-API failures and real
worker rejection of unavailable/invalid devices. An FFmpeg context test verifies
NVENC option readback without initializing a GPU driver. CPU codec fixtures still
exercise conversion, predictive H.264, rate restart and decoding.

The local host has only an Intel iGPU, not a dedicated or hybrid GPU setup.
NVIDIA/AMD hybrid operation, suspended/disconnected adapters, driver stalls and
rotated physical outputs require separate hardware acceptance. A hung early
VA-API node can still consume the backend's overall worker deadline; explicit
pinning can bypass it, but per-adapter process/deadline isolation is not added.
No hardware throughput, CPU reduction or end-to-end latency gain is claimed.
The reference update, tests and release process do not install software or
restart/switch the host's production desktop or services.
