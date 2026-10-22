# Native media (v0.1.2)

The runtime does not invoke FFmpeg. The browser protocol stays unchanged:
Annex-B H.264/WebCodecs with JPEG fallback, and 48 kHz stereo f32le PCM audio.

## Video

A dedicated thread owns a source-built OpenH264 encoder. RGBA is converted into
a reusable I420 buffer. Encoding uses real-time screen-content mode, baseline
profile, two threads, no frame skipping and an intra period of 120 frames.
The bitrate target retains the previous pixel-based range (0.6–18 Mbps), not a
hard cap: OpenH264 warns that rate control cannot guarantee this target with
frame skipping disabled. This tradeoff preserves demand-driven frame/ACK flow.
Each request produces one complete access unit; there is no pipe/AVI framing.
The request queue has capacity one, and the existing render-ACK window remains
authoritative. Resize recreates the encoder, producing fresh SPS/PPS/IDR.
Dropping the sender releases the worker after its current native call returns;
native calls cannot be force-canceled. Canceled/failed encoding invalidates the
encoder, preventing a dependent P frame after discarded output; the worker exits
when it detects abandoned output or the sender is dropped.

## Linux / hot-path follow-up

- The capture worker reuses each AF_UNIX/Wayland connection for at most eight
  frames and checks a 250 ms expiry before the next capture. This is not a hard
  connection lifetime bound when capture blocks or runs at a low frame rate.
  Periodic rebuilding still bounds retained
  libwayshot protocol objects and refreshes output information. External monitor
  layout changes can take one batch to appear. Single-output capture avoids
  composing an unnecessary multi-output mosaic; multi-output capture is retained.
  A failed screenshot discards the batch and retries with fresh outputs up to
  three times; a missing explicitly selected output remains an error.
- JPEG compression runs only when a JPEG packet is actually requested, on the
  blocking pool with a thread-local compressor. H.264 capture no longer also
  compresses every frame as JPEG. Frame-local caching reuses a requested JPEG.
- H.264 requests hold the capture's `Arc<[u8]>`; they no longer copy a full RGBA
  allocation into the encoding worker. This is shared CPU memory, not DMA-BUF
  or GPU zero-copy. CPU color conversion and SHM capture allocations remain.
- Accepted TCP connections enable `TCP_NODELAY`. Proxy/network/browser buffering
  remains outside this setting's scope. No global kernel, priority or affinity
  settings are changed.

## Audio

A dedicated thread owns a libpulse context, recording stream and nonblocking
mainloop. It connects to the session's explicit Pulse server/source without
changing process-global environment. Context/stream startup has a five-second
deadline. Stop/disconnect sets a cancellation flag and joins the worker; idle
polling sleeps at most two milliseconds between checks.
The stream requests ten-millisecond fragments. Data is packetized in 3840-byte
chunks, preserving stereo sample alignment across peek boundaries and filling
Pulse holes with silence. A four-packet (40 ms) queue preserves normal batched
fragments and replaces the oldest packets only when that bound is exceeded.
Physical-session audio remains unsupported.

The current frontend uses a session-bound, independently authenticated audio
WebSocket. Its async sender and blocking-pool initialization are separate from
the video transmit loop. Legacy clients retain the original multiplexed path.
See `audio-channel.md` for cancellation, connection limits and bandwidth scope.

Audio now negotiates source-built libopus (20 ms / 128 kbit/s stereo) when the
browser's Worker supports WebCodecs audio decoding. Decoded audio is transferred
directly to a bounded AudioWorklet ring. PCM and the legacy main-thread player
remain compatibility paths. See `audio-channel.md` for protocol and diagnostics.

## Dependencies and scope

Builds require a native C/C++ toolchain, NASM and libpulse development headers.
OpenH264 is compiled from pinned crate source, not loaded from an arbitrary
system blob. libpulse is dynamically linked. PipeWire/Pulse, WirePlumber and
pactl still provide the isolated desktop audio graph.

This is software encoding, not hwcodec/VA-API/NVENC. Removing a subprocess is
not proof of lower input-to-display latency. Compare the same desktop load,
resolution, frame rate and network/ACK delay before making performance claims.
Distribution uses source-built OpenH264 with upstream and wrapper notices;
it does not claim Cisco binary patent coverage. See `third-party.md`.
The local v0.1.2 installation preserves the previous v0.1.1 transaction for rollback.
