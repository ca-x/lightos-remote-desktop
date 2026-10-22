# Optional RDP access

The optional RDP server exposes the **same owned virtual Wayland desktop** as
the browser. It uses IronRDP to encode the existing raw capture and forwards
validated input through the existing Wayland backend. It does not start xrdp,
Xorg, another compositor, or another desktop. The browser transport remains
available; one browser or RDP connection may control the desktop at a time.

RustDesk's RDP action is different: its client forwards a local TCP port through
RustDesk to an existing RDP service (3389 by default), then launches an RDP
client. It is not a conversion of RustDesk's video protocol. Source and primary
reference details are in [rustdesk-reference.md](rustdesk-reference.md).

## Build and configuration

The published Linux binary includes RDP, with its listener disabled until
configured. Source builds can omit it: RDP is an optional Cargo feature requiring
Rust **1.89+** (IronRDP 0.13's minimum). Builds without it retain Rust 1.88 and their
existing prerequisites. To build with RDP:

```sh
npm ci
npm run build
cargo build --release --locked --features bundled-jpeg,rdp
```

Runtime RDP is off by default even in that binary. Enabling it requires a PEM
certificate chain and matching PEM private key. The RDP listener defaults to
`127.0.0.1:3389`; HTTP listen settings do not change it. It uses the existing
`--username` / `LIGHTOS_REMOTE_DESKTOP_AUTH_USER` and
`--password` / `LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD` credentials. The same
password rules apply. Store credentials outside shell history and keep the key
readable only by the service user.

For an existing persistent desktop owner, use its current runtime path:

```sh
export LIGHTOS_REMOTE_DESKTOP_AUTH_USER=lightos
read -rsp 'Desktop password: ' LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD
export LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD
./target/release/lightos-remote-desktop serve \
  --session-runtime /path/to/current/persistent/runtime \
  --rdp \
  --rdp-tls-cert /path/to/server-chain.pem \
  --rdp-tls-key /path/to/server-key.pem
```

This is a transport startup example, not an instruction to replace/restart the
current desktop owner or install a second service. `--wayland-socket` attaches
an external/physical desktop and is deliberately rejected with RDP enabled.
RDP supports the current Linux Wayland backend; other OS capture/input are not
implemented by this feature.

| CLI option | Environment | Default |
| --- | --- | --- |
| `--rdp` | `LIGHTOS_REMOTE_DESKTOP_RDP` | `false` |
| `--rdp-listen` | `LIGHTOS_REMOTE_DESKTOP_RDP_LISTEN` | `127.0.0.1:3389` |
| `--rdp-tls-cert` | `LIGHTOS_REMOTE_DESKTOP_RDP_TLS_CERT` | required when enabled |
| `--rdp-tls-key` | `LIGHTOS_REMOTE_DESKTOP_RDP_TLS_KEY` | required when enabled |

Use `true`/`false` for the boolean environment setting. A public listen address
must be explicitly configured and still requires TLS and credentials. TLS/key
errors and occupied ports fail startup; the server never silently falls back to
plaintext. A stalled authentication/activation attempt closes after 15 seconds
without reserving the browser controller. Input-route failure closes control and
runs the existing held-input release barrier. HTTP shutdown stops RDP as well.

## Connect

Use a client supporting RDP TLS security and submit the application username and
password with an empty domain. This first implementation does **not** implement
NLA/CredSSP, Windows account login, or a graphical login screen. A client whose
policy requires NLA cannot connect; this was verified with FreeRDP, not Windows
mstsc or Microsoft's macOS client.

For FreeRDP, verify the server identity using a CA-trusted certificate or pin a
SHA-256 fingerprint obtained through a trusted channel:

```sh
openssl x509 -in /path/to/server-chain.pem -noout -fingerprint -sha256
xfreerdp3 /v:127.0.0.1:3389 /u:lightos /from-stdin:force \
  /sec:tls /cert:fingerprint:sha256:YOUR_VERIFIED_CERTIFICATE_SHA256
```

Some installations name the executable `xfreerdp`. For a loopback listener on a
remote machine, establish an SSH forward first and connect to that local port:

```sh
ssh -N -L 13389:127.0.0.1:3389 user@desktop-host
xfreerdp3 /v:127.0.0.1:13389 /u:lightos /from-stdin:force \
  /sec:tls /cert:fingerprint:sha256:YOUR_VERIFIED_CERTIFICATE_SHA256
```

SSH forwarding carries this application's actual RDP service; it does not change
its protocol. Verify the RDP certificate even when using the tunnel.

## Behavior and limits

- Display updates consume the latest raw frame, with bounded buffering. The RDP
  adapter converts RGBA to BGRA for standard raw RDP surfaces. IronRDP negotiates
  its bitmap codecs with the client; this is not the browser H.264 pipeline.
  Raw fallback can consume substantial bandwidth and is not a WAN performance
  claim or a hardware/zero-copy RDP implementation.
- Current capture size is authoritative. Existing browser/API display settings
  still control resolution and scale; an RDP client cannot resize the compositor
  or change monitor layout. Capture-size changes trigger RDP reactivation before
  new-size bitmaps. Reconnect keeps the same desktop and applications.
- Absolute mouse motion, three buttons, wheel scrolling, common PC scancodes
  (including Ctrl/Alt/Super/navigation), and Unicode UTF-16 text are forwarded
  through validated `InputCommand`s. The compositor's existing keyboard layout
  applies to physical key events. Unknown vendor keys, relative-pointer mode,
  extra mouse buttons and client lock-state synchronization are unsupported.
- Capture includes the compositor cursor; the RDP client pointer is hidden to
  avoid duplicates. The cursor follows capture cadence, as it does in the browser.
- RDP audio, clipboard, files/drives, printer redirection, multiple monitors,
  client-requested resolution changes and Windows session management are not
  implemented. Continue using the browser path for its existing audio/clipboard
  features. There are no new browser/mobile/IME settings or host config changes.

## Verification

The opt-in interoperability test starts only a synthetic capture/input backend,
a loopback TLS RDP listener and an isolated Xvfb client display. It never attaches
to the real desktop, edits configuration or installs packages. Supply actual
available executables rather than assuming their names:

```sh
LRD_TEST_FREERDP=/absolute/path/to/xfreerdp3 \
LRD_TEST_XVFB=/absolute/path/to/Xvfb \
cargo test --features rdp freerdp_tcp_tls_bitmap_input_resize_reconnect_and_denials \
  -- --ignored --nocapture
```

Python 3, OpenSSL, the FreeRDP client's shared libraries, Xvfb, libX11 and libXtst
are test-only prerequisites. CI installs these tools, resolves the actual
FreeRDP/Xvfb executable paths and runs this protocol check alongside the optional
feature tests and Clippy. A private test certificate is generated for the run
and pinned by the client; no real user password is used.

On 2026-10-23, FreeRDP **3.32.2** decoded the expected red 320×200 bitmap, sent
real XTest mouse/key events through RDP, received the resulting blue 640×480
resize, then reconnected at 640×480. Certificate distrust, wrong/empty password
and plaintext downgrade all returned failure. The real 15-second idle handshake
deadline, occupied-listener rejection and listener closure at shutdown are also
covered. The Rust side verified actual
pointer, button, key and disconnect release commands. These checks validate the
protocol adapter and geometry/color order; they do not claim a GUI/codec speed
benchmark or certification for other RDP clients. Live desktop/browser operation
uses the unchanged capture/input backend; no live desktop was restarted for this
verification.

Regression tests also pause an accepted input callback across disconnect and
controller reuse, and change capture dimensions between the initial announcement
and activation, then again during resize reactivation. They check input ordering
around the release barrier and require Resize before a bitmap with new geometry.
Each regression failed before its corresponding fix. Input admission and closure
share a lock; announced dimensions survive update-stream replacement and advance
only on the initial announcement or a Resize.

The same worktree passed `cargo test --workspace --locked` (122 passed, 16
pre-existing live/environment checks ignored), `cargo test --workspace --locked
--features rdp` (131 passed, 17 opt-in checks ignored), and strict all-target
Clippy with RDP enabled. The explicit FreeRDP test above passed separately;
its ignored status in the general suite is not an interoperability claim by
itself. `cargo build --release --locked` preserved the default software binary,
and its CLI rejected an RDP enable request with the feature-required error.
`npm run build` and `npm test` passed (84 Node tests and 15 Vitest tests).
