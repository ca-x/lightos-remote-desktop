# Adaptive streaming and weak-network resilience

## Goal and scope

Implement independently designed delay-based streaming control using RustDesk's
QoS principles as a read-only reference. Keep the extracted platform, codec and
scheduler boundaries. Preserve current capture/input, mobile gestures, IME,
resolution/scale behavior and persistent desktop ownership. Ship with evidence,
not a claim of parity with RustDesk's entire transport.

## Required behavior

- Default Auto remains selectable alongside Low latency, Smoothness and the new
  Quality first (清晰优先). Persist and resend the user's choice on reconnect.
- Render ACK time includes transport, decode and drawing. Call it ACK delay,
  never pure network RTT. Use validated frame IDs and fresh, time-spaced samples.
- Learn a connection-specific baseline; a constant high path delay is not itself
  congestion. Follow lower minima immediately, age a settled floor gradually,
  and do not follow a continually rising queue.
- Require consecutive excess-delay samples before an ordinary reduction; one
  spike cannot cut quality/FPS. Prefer lowering encoder budget for Auto/latency/
  smoothness. Lower FPS if quality cuts fail or cannot help. Quality first keeps
  the configured image-quality budget per frame and reduces FPS instead.
- Start conservatively, accelerate after clean feedback, and recover gradually
  with cooldowns. Brake outstanding *sent frames* after two seconds. Never
  mistake an idle desktop for a network stall or double-penalize its late ACK.
- Bound the existing frame-credit window to 1..3. Pace and discard only raw
  capture frames; preserve every emitted H.264 predictive frame in order.
- H.264 runtime bitrate/FPS changes must reach the existing native encoder via
  OpenH264's existing bindings, without recreating it or introducing extra IDRs.
  JPEG uses TurboJPEG's quality API. Real probes verify the advertised controls.
- Keep audio and input independent of the video ACK gate. An established socket
  tolerates a transient slow write (15s bound); abandoned writes close the
  connection, never resume a partially written video message.
- Automatic reconnect continues with a capped backoff while the user wants a
  connection. Offline waits for online. A silent visible transport is detected
  with a bounded liveness timer. Manual disconnect and desktop-switch release
  rules remain explicit; never start/restart the persistent compositor to recover
  transport.
- Reuse mature Rust crates for encoding, protocol bindings and asynchronous I/O.
  The connection policy itself is application-specific; no AGPL source is copied.

## Evidence and limitations

Deterministic tests cover steady high delay, transient spikes, rising queues,
confirmation, bitrate-before-FPS, Quality first, startup/recovery, stalls, stale
ACKs, resize/reset and configured caps. Native codec tests decode every frame
across rate changes. Frontend tests exercise persistent quality preference,
repeated reconnect, offline/online, liveness and manual cancellation.

Compare the installed 0.1.8 binary against the candidate on the same isolated
1280x720 moving desktop through a userspace rate-limited proxy. Record actual
received FPS/bytes, ACK delay, target FPS/budget, socket closures and server CPU.
Exercise clean, constrained, recovery and temporary interruption phases. This is
synthetic transport evidence, not a browser input-to-photon or WAN measurement.
Other desktop environments remain display adapters pending full capture/input
and real desktop testing.
