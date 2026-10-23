# Binary wire experiment implementation plan

**Goal:** Measure the previously proposed frame-ID, ACK and frequent-input changes on an isolated branch, retaining HTTP APIs and the legacy WebSocket protocol.

**Architecture:** Three negotiated WebSocket modes: legacy (default), `lightos.frames.v1` (inline frame ID), and `lightos.protobuf.v1` (inline frame ID plus Protobuf ACK / move / button / key / scroll / release). Encoded JPEG / Annex-B H.264 payloads and scheduling semantics remain the same. Use `prost` and Protobuf-ES rather than a custom general-purpose serializer.

**Scope:** User requested a new experimental branch and tests, mainly the recommended hot-path changes. No production install, service restart, main merge, release, Connect RPC migration, X11 helper IPC rewrite, or codec-worker shared-memory change.

## Constraints and acceptance

- Keep legacy clients usable; experimental modes require explicit WebSocket subprotocol negotiation (`?wire=frames` or `?wire=protobuf` in the browser).
- Binary input must pass the same `InputCommand::validate` and enter the same bounded input channel. Reject malformed / oversized / unsupported messages. Keep the same controller, privacy epoch, cancellation and ACK checks.
- Encode IDs as uint64 but reject values outside the browser safe-integer range.
- Keep low-frequency setup, clipboard, resize and metrics messages as JSON.
- Report payload bytes, WebSocket envelope bytes, browser decode/draw durations, frame rate and CPU separately. Never call envelope accounting TCP/IP/TLS or loopback tests Internet performance.
- Deterministic replay must use identical compressed payloads in every mode. Exercise real Rust WebSocket I/O and a real browser decoder / canvas, not a Rust or Python client impersonating a browser.
- Record runtime/library versions, source commit, workload, warmup, repetitions and measurement limits. Include current application route smoke tests as well as isolated replay.

## Tasks

- [x] Add shared protocol schema, Rust and browser wire helpers, golden vectors and malformed-input / ID validation tests.
- [x] Integrate explicit subprotocol negotiation, inline IDs, and Protobuf frequent commands without changing legacy defaults. Run focused Rust and frontend tests.
- [x] Add a bounded, loopback-only replay harness using the production wire helpers and command receiver; run identical JPEG/H.264 streams in a real browser, across repeated runs. Check the application browser route too.
- [x] Write the measured result and reproducible commands under `docs/benchmarks`, review the diff, commit and push the experiment branch.

## Review focus

The production listener already enables TCP_NODELAY (src/main.rs); keep that setting in the primary comparison. Unnegotiated binary messages; stale ACKs; key/button release validation; locked/epoch-invalid video; reconnect to a legacy server; payload copies and browser parsing differences that confound the comparison; H.264 keyframe/decode failures; distinguishing draw submission from screen presentation.

## Additional requested scaling evaluation

- [x] Inspect application pixel paths: display resolution changes plus browser layout; no generic per-frame CPU resizer today.
- [x] Check existing crates and MSRV. Benchmark `fast_image_resize 6.1.0` (Rust 1.87) against `image 0.25.10` outside runtime dependencies; retain opaque-alpha / allocation / filter differences in results.
- [x] Include the scaling microbenchmark and its limits in the result report.

Pilot validation: 120 Rust unit tests passed (6 intentionally ignored); 100 Node tests plus 40 Vitest tests passed; frontend build and production-library Clippy passed. A 30-frame pilot completed all 18 measured cases, 18 warmups and all three complete application client smoke checks. Formal repeats follow separately.

Formal result: 90 measured replays, 18 warmups and all three application smoke checks completed. See `docs/benchmarks/2026-10-23-binary-wire.md` and its raw JSON datasets.
