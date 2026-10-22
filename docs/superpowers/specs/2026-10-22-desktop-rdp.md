# Optional RDP access to the existing virtual desktop

## Objective and scope

Expose the application's existing owned Wayland virtual desktop over an optional
RDP listener, while retaining the browser WebSocket path. RustDesk's RDP action
is a tunnel to an existing RDP service; it does not turn its captured video into
RDP. This implementation instead uses the independently licensed IronRDP server
and our existing raw capture and validated input routes.

Assumptions: one controller may operate a desktop at a time; the current capture
size is authoritative; physical/external sessions are not RDP control targets.
No compositor, desktop owner, host service or v2rayN process is restarted.

## Requirements

- The `rdp` Cargo feature is optional and disabled by default. A software-only
  default build keeps its existing native prerequisites and Rust 1.88 floor.
  IronRDP 0.13 requires Rust 1.89 or newer only when this feature is selected.
- Runtime RDP is off unless explicitly enabled. Its independent listener binds
  `127.0.0.1:3389` by default, with a configurable address/port.
- Enabling requires an explicit TLS certificate/key and the already validated
  application username/password. Use TLS and credential verification on every
  connection; never offer plaintext RDP. Reject unsupported/no authentication.
- Default-off operation opens no RDP socket. Invalid TLS credentials, unavailable
  listener, attached physical session and feature-disabled requests fail clearly.
- Serve current RGBA capture through a watch receiver (latest frame, bounded
  buffering), including initial frame and size changes. Do not start another
  capture worker or resize/restart a compositor in response to an RDP client.
- Forward normalized pointer motion, three mouse buttons, scrolling, physical
  keys and Unicode text through `InputCommand::validate` and the existing input
  sender. Release held input on disconnect and before allowing another controller.
- RDP and browser share the existing exclusive controller gate. Authentication
  and idle handshakes must not reserve browser control indefinitely.
- Stop the listener and any RDP connection on HTTP shutdown; failures must be
  visible. Do not modify browser/mobile/IME/resolution/scale flows.

## Boundaries and documented limits

Linux Wayland capture/input only. This is a bitmap RDP path, not RustDesk protocol
conversion, not a transport tunnel, and not a Windows login/session service.
RDP audio, clipboard, drive redirection, multiple monitors, client-requested
compositor resizing and hardware RDP graphics are outside this change. Browser
settings remain responsible for desktop resolution/scale. Existing HTTP auth is
not a substitute for RDP TLS. Users must verify their server certificate.

## Acceptance and commands

- `cargo test --workspace` and `cargo build --release` preserve default behavior.
- `cargo test --features rdp` and `cargo clippy --workspace --features rdp -- -D warnings`
  verify enabled builds, disabled/default config, TLS/config failures, input
  translation, bounded display updates, controller release and authentication.
- A mature RDP client/library must negotiate the real protocol over a TCP/TLS
  connection, authenticate, decode a bitmap update, send input and disconnect.
  Wrong/missing credentials and plaintext downgrade must fail before any frame
  or routed input. Record actual commands/results and any unavailable GUI-client
  validation in `docs/rdp.md`.
- Existing UI checks/build remain green. No host configuration or service
  changes are part of verification.
