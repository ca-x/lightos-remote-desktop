# Implementation contract

The user requests ca-x/lightos-remote-desktop and a Rust replacement for the
installed Python service. Source lives in ~/code/rust/lightos-remote-desktop.

## Boundaries and rulings

- Do not modify the installed Python service until the candidate passes real
  desktop/HTTP tests. Keep an atomic, documented rollback path.
- GitHub API authentication was restored by the user's `gh` login; the private
  repository is https://github.com/ca-x/lightos-remote-desktop. SSH works when
  the configured unavailable proxy is bypassed. Never copy tokens or runtime
  credentials into this repository.
- Authorized system cutover was performed after native desktop verification.
  Preserve the original Python payload, configuration and rollback snapshots.
- Ruling: use standard Wayland capture/input protocols and libwayshot rather
  than forking the Python-coupled pixelflux tree for the initial native backend.
  This avoids shipping a CPython runtime. Video now negotiates CPU H.264 with
  WebCodecs and JPEG fallback, but is not hardware/zero-copy performance parity. Do not present it as a
  fully equivalent production replacement until remaining features are verified.
- Native Rust means no Python runtime. Hyprland, libxkbcommon, wl-clipboard,
  FFmpeg/PulseAudio-compatible audio services remain external dependencies.
- Ruling: own a private PipeWire/Pulse/WirePlumber graph after the private D-Bus
  starts. Ambient host Pulse authentication is not a production dependency.
  Disable hardware monitoring; all audio resources die with their owned process
  groups. Audio startup failure is fatal when audio was requested.

## Modules

- src/wayland.rs: capture and virtual keyboard/pointer input; owns synchronous
  Wayland connections and publishes the latest raw frame. The capture connection
  is periodically recycled after at most eight frames / 250 ms between captures;
  JPEG is generated lazily in blocking workers, and H.264 shares the raw pixels.
- src/config.rs: CLI/environment validation; compatible LIGHTOS_REMOTE_DESKTOP_*
  variables; refuses missing/default secrets and root runtime.
- src/session.rs: launch/attach a private headless Hyprland session, bounded
  readiness, process groups and teardown without UWSM source modification.
- src/server.rs: authenticated HTTP and WebSocket, origin checks, exclusive
  controller, latest-frame delivery, input bounds, health, clipboard and audio.
- web/: same-origin browser client, canvas, reconnect, input cleanup.
- scripts/: root install/update/rollback and standalone checks.

## Browser wire contract

- GET / serves the embedded client. GET /app.js and /style.css serve assets.
- GET /healthz returns 503 until native capture produced a frame, 200 when ready.
- All desktop/asset/WebSocket endpoints require Basic Auth. Browser auth dialog
  is intentional. No credential in query strings or localStorage.
- GET /ws upgrades after same-origin validation; one controlling connection,
  later controllers rejected with 409. Close releases all pressed input.
- Server binary: byte 1 followed by a JPEG frame; byte 2 followed by little-endian
  float32 interleaved stereo PCM at 48 kHz. Frame delivery never queues unbounded.
- Server JSON: {type:"ready",width,height,audio:false|true}, {type:"clipboard",text},
  {type:"error",message}. Browser reconnects with bounded exponential backoff.
- Client JSON: {type:"move",x,y} where x/y are normalized 0..1;
  {type:"button",button:0|1|2,pressed:bool}; {type:"key",code:<evdev>,pressed:bool};
  {type:"scroll",x,y}; {type:"release"}; {type:"text",text};
  {type:"resize",width,height}; {type:"clipboard_get"};
  {type:"clipboard_set",text}; {type:"audio",enabled:bool}.
  Servers advertise `key_chords` and `image_clipboard` in ready. Negotiated
  `{type:"chord",codes:[...]}` sends at most eight distinct evdev keys as one
  input operation, releasing in reverse order. Image paste uses connection-local
  `clipboard_image_start` (`id`, `step:0`, `size`), `clipboard_image_chunk`
  (`id`, increasing `step`, base64 `data`, at most 48 KiB decoded per chunk),
  and `clipboard_image_paste` (`id`, final `step`, optional `shift`). Every request waits for a
  matching `clipboard_ack` (`id`, `step`, optional `error`). Transfers are PNG
  only, at most 8 MiB and 8192 pixels per dimension / 32 MiPixels total, with
  a 15-second idle limit. Failed or disconnected uploads are discarded; only
  a complete, valid image is offered to the selected Wayland clipboard before
  sending Ctrl+V (or Ctrl+Shift+V when the initiating gesture held Shift).
  `clipboard_paste_text` accepts UTF-8 `text` up to 64 KiB plus the same optional
  `shift`; it offers text before pasting, preserving astral Unicode and terminal
  bracketed paste. It does not synchronize clipboard contents in the background.
- Keycode is Linux evdev, without the XKB +8 offset. Browser event.code mapping
  must not forward arbitrary strings to a shell. Text uses a Unicode-aware
  virtual keymap in the backend. Bounded text and clipboard sizes.
- Readiness starts with `cursor: "embedded"` and an optional connection-scoped
  `cursor_session`. A same-origin authenticated `/cursor?session=...` socket
  carries actual shapes/positions independently of video. The optional Hyprland
  observer verifies the exact running ABI and exposes read-only cursor data.
  `frame_id.cursor_embedded` identifies the actual capture mode of a displayed
  frame, so switching local/embedded presentation cannot duplicate the cursor.
  Session generations prevent an old cursor connection from changing the new
  controller's mode. Unsupported or failed cursor channels retain video capture
  with its embedded cursor.
- While dragging the floating toolbar, all layout requests retain the current
  pointer-relative position, including React updates and resize callbacks.
  The toolbar docks and persists its position only on pointer release;
  cancellation restores the previous dock.
- Windowed control never enters fullscreen automatically. A manual fullscreen
  entry containing the desktop focuses it and requests all available keys;
  leaving fullscreen, losing focus, disconnecting, or Ctrl+Alt+Shift+Esc
  releases held input and keyboard capture. A new manual fullscreen entry
  re-enables capture after an explicit release. Clicking the focused desktop
  can retry a denied request. Captured shortcuts go to the remote desktop;
  Ctrl+V and Ctrl+Shift+V preserve browser-to-remote text/image
  paste in both windowed and captured fullscreen modes. An editable textarea
  proxy receives local IME composition; printable physical keys wait until
  native insertText confirms they are not the start of composition.
  OS shortcut suppression requires browser/compositor support and permission;
  on Wayland hosts, use native Wayland Chrome/Chromium with shortcut inhibition.

## Native backend contract

wayland.rs exports:

```text
Frame { width: u32, height: u32, jpeg: Vec<u8> }
InputCommand (serde tagged type, snake_case): Move {x:f64,y:f64},
 Button {button:u32,pressed:bool}, Key {code:u32,pressed:bool},
 Scroll {x:f64,y:f64}, Release, Text {text:String}
Backend::start(socket: PathBuf, fps:u32, quality:u8) -> anyhow::Result<Backend>
Backend::frames: tokio::sync::watch::Receiver<Option<Arc<Frame>>>
Backend::input: tokio::sync::mpsc::Sender<InputCommand>
Backend::stop(): signals its owned thread; never modifies global environment.
```

## Acceptance and verification

1. npm ci; npm run build; npm test; cargo test; cargo fmt --check; cargo clippy --all-targets -- -D warnings.
2. Installer shell syntax and sandbox tests prove backup, unchanged secrets,
   idempotence, rollback and no activation before explicit install.
3. Start candidate on a separate port/runtime directory. Capture a real frame,
   exercise keyboard/pointer, authenticate HTTP/WS and reject foreign origin.
4. Verify audio, clipboard, resizing, Unicode and disconnect cleanup separately.
   Any unverified or unsupported capability must be named, not silently omitted.
5. Only then install a system override for the existing unit, preserving its
   environment file and original payload; verify status, health and restart count.

## Progress

- Private source repository created and pushed after the user's renewed API login.
- Administrator access was provided separately; compatibility library provisioned
  privately without editing the system library or original authentication file.
- Candidate real HTTP/WebSocket tests cover JPEG decoding, auth/origin,
  exclusive controller, invalid input, text clipboard, audio, resize and reconnect.
- Per-frame capture connections contain libwayshot protocol lifetime defects;
  post-resize stability passed, including a separate 61-second FD/RSS sample.
- Live system cutover completed after browser checks and independent review.
  Installed protocol tests passed with private audio; see verification.md.
- Default shell auto-detection now starts the installed Omarchy wallpaper,
  bar and menu rather than a terminal-only desktop. Remote application launch
  uses private PATH adapters; host UWSM/environment import is not required.
- Video writes no longer block the input receiver. Render acknowledgements
  bound video in flight; persistent H.264 encodes latest raw captures only on
  demand, preserving reference-frame dependencies. FFmpeg AVI framing is parsed
  with bounded lengths, not guessed from pipe read boundaries or idle timers.
