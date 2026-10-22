# RustDesk transport reference

Read-only source investigation of `/home/czyt/code/rust/rustdesk` on 2026-10-22.
Checkout HEAD: `bd9262397208c509ea97af61f0d44621ef25611c`.
No RustDesk source is copied into this project (RustDesk is AGPL-licensed).

- `libs/scrap/src/common/codec.rs`: negotiated VP9/VP8/AV1 and hardware codec
  abstractions; quality and bitrate changes belong to persistent encoders.
- `libs/scrap/src/common/vpxcodec.rs`: real-time encoder configuration, bounded
  latency, CBR and controlled keyframe behavior. Independent JPEG images cannot
  achieve equivalent compression on changing desktop content.
- `src/server/video_qos.rs`: measures excess delay above each connection's
  observed baseline; confirmed congestion reduces bitrate before ordinary FPS
  reduction. Startup acceleration and conservative recovery avoid oscillation.
- `src/server/connection.rs`: dedicated video service queue, viewer frame
  acknowledgements and separate cursor messages. This is not evidence of a
  completely separate physical transport for every input/video packet.
- `libs/scrap/src/common/mod.rs`: unchanged pixels yield WouldBlock rather than
  repeated encoding. Static refinement is separately handled in video_service.
- The checked-in guide says the Flutter Web runtime has a separate hand-written
  JS client in a gitignored tree; do not assume the native Rust client can simply
  be compiled to WASM and dropped into our browser page.

Our independently implemented first improvements are bounded video credits,
independent input processing, unchanged-image suppression, a local browser cursor,
and SIMD JPEG. The added H.264/WebCodecs path preserves prediction dependencies:
drop unencoded capture frames, not arbitrary encoded P-frames. Resolution changes
and reconnects use a fresh encoder/keyframe. Browser WebCodecs support is
checked, with JPEG retained when it is unavailable. Version0.1.9 independently implements the relevant QoS principles: an ACK-delay
baseline with settled-floor aging, fresh feedback and consecutive congestion
confirmation, budget-before-FPS cuts, conservative startup/recovery and stall
braking. Thresholds/cooldowns and manual policies are application-specific.
OpenH264 runtime bitrate/FPS updates are implemented through its generated Rust
bindings and checked with native readback; native raw skips produce no access
unit and are never sent as an empty predictive packet. Quality first keeps the
configured per-frame budget while reducing FPS. RustDesk's native codecs,
multi-viewer controller, UDP/NAT traversal and whole transport are not replicated.
GPU zero-copy, hardware codecs and actual WAN/browser input-to-photon results
remain separate work; synthetic shaped-link evidence is in verification.md.

## RDP behavior (independent implementation)

The same read-only RustDesk checkout was checked for RDP on 2026-10-22:

- `src/ui_session_interface.rs:1989`: the RDP connection branch parses an
  existing remote RDP port (3389 by default) and invokes port forwarding.
- `src/port_forward.rs:18` and `:90`: binds a loopback TCP listener and launches
  `mstsc` pointed at that local port. This carries an existing RDP server's
  traffic; it does not convert RustDesk capture/video into RDP.
- RustDesk's official advanced-settings documentation describes `enable-tunnel`
  as enabling TCP tunneling. Its official comparison treats RustDesk and RDP as
  different protocols. Primary documentation source:
  `https://github.com/rustdesk/doc.rustdesk.com/blob/master/content/self-host/client-configuration/advanced-settings/_index.en.md`
  and `https://github.com/rustdesk/doc.rustdesk.com/blob/master/v3/src/content/post/en/rustdesk-vs-rdp.md`.
  The rendered rustdesk.com endpoint returned HTTP 403 in this environment, so
  the official documentation repository was read directly.

Our optional `rdp` feature independently implements an actual RDP server over
our existing captured desktop using IronRDP 0.13 (MIT OR Apache-2.0), whose
primary API documentation is `https://docs.rs/ironrdp-server/0.13.0/ironrdp_server/`.
No RustDesk code is copied. See [RDP setup and verified scope](rdp.md).
