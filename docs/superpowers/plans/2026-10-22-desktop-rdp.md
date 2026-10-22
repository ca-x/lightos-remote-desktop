# Optional Desktop RDP Implementation Plan

> Execution: implement inline in the delegated feature worktree, then request an
> independent review from the parent before integration. The user authorized
> implementation, verification, commit and push; no host rollout is authorized.

**Goal:** Offer authenticated TLS RDP access to the same captured virtual desktop.

**Architecture:** Add an optional IronRDP server adapter driven by the existing
latest raw-frame watch receiver and `InputCommand` sender. Keep an independent
listener, share browser controller ownership, and stop it with HTTP transport.

**Tech Stack:** Rust, Tokio, IronRDP 0.13, rustls; existing Wayland capture/input.

**Spec:** `docs/superpowers/specs/2026-10-22-desktop-rdp.md`.

## Global constraints

Default off; loopback by default; mandatory TLS and authentication; optional
feature requires Rust 1.89, default preserves Rust 1.88. No new compositor/capture,
physical-session input, native config edits, version changes, deployment or tags.

## Review focus

- Missing/wrong credentials and attempted plaintext negotiation produce no data.
- Failed/slow handshakes must not retain browser ownership.
- Disconnect/overfull input queue must release held keys/buttons before reuse.
- Frame geometry changes must not send mismatched RDP bitmap dimensions.
- Default builds must not acquire new RDP runtime/build requirements.

## Tasks

- [x] Add `src/rdp.rs` configuration and focused default/config-failure tests;
  flatten it in `Config`; optional dependencies in `Cargo.toml`; export module.
- [x] Add feature-gated `src/rdp/service.rs` with TLS/authenticated listener,
  protocol timeout/shutdown and display/input adapters. Add narrowly scoped
  shared-controller access in `src/server.rs` and start/stop in `src/main.rs`.
- [x] Exercise real TCP/TLS RDP with a mature client/library; verify bitmap and
  routed inputs, failure/auth, geometry change, release and disabled operation.
- [x] Document prerequisites, connection commands, scope and measured checks in
  `docs/rdp.md`; link from README; update RustDesk reference with source evidence.
- [ ] Run default and enabled verification and relevant frontend checks; inspect
  diff; commit and push branch; ask parent for independent review and integration.
