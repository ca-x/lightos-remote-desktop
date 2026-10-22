# Desktop MCP Implementation Plan

> **For agentic workers:** Execute inline with `executing-plans`; no further agents are authorized for this implementation. Steps use checkboxes for tracking.

**Goal:** Add an opt-in, scoped, revocable MCP desktop endpoint and browser settings.

**Architecture:** A focused `src/mcp/` module owns private token persistence, HTTP management/authentication and official `rmcp` tool handling. `server::AppState` passes only its existing capture/input/controller/desktop boundary to that module. Stateless Streamable HTTP keeps token authorization on every request and avoids durable protocol sessions.

**Tech Stack:** Rust/Axum/Tokio, official rmcp 3.1.2 (MSRV 1.88), rand OS randomness, sha2, React/Headless UI/i18next.

**Spec:** `docs/superpowers/specs/2026-10-22-desktop-mcp.md`

## Global Constraints

- Default disabled, separate hashed bearer tokens; existing Basic/same-origin rules protect management.
- No compositor/service lifecycle changes, host config writes, arbitrary execution, version/tag changes or main mutations.
- Owned virtual desktop only for control; same capture pixels/input validation/controller exclusion as the browser.
- Five-second tool deadline; bounded inputs and cancellation release all held input.
- Work only in the `ad2c` worktree on `feat/desktop-mcp`.

## Review Focus

- Credential persistence failures leave authorization disabled/unchanged and never reveal tokens through list/errors.
- Disable/revoke during a queued action prevents further input and releases held state before another controller.
- Screenshot pixel coordinates remain correct across resolution/scale changes and stale capture is rejected.
- Cross-origin browser requests cannot create tokens using ambient Basic credentials.
- Active browser and attached physical sessions cannot be silently controlled by MCP.

### Task 1: Authorization, persistence and management

**Files:** `Cargo.toml`, `Cargo.lock`, `src/config.rs`, `src/lib.rs`, `src/server.rs`, new `src/mcp/mod.rs`, `src/mcp/store.rs`.

- [x] Add rmcp/hash/random/cancellation dependencies and explicit settings path override.
- [x] Implement atomic private settings, token generation/hash lookup, enable/disable/revoke and cancellation.
- [x] Mount authenticated management routes and separately authenticated MCP transport at the app prefix.
- [x] Test persistence/reload, privacy, failures, invalid credentials, same-origin boundaries and disabled/revoked access.

### Task 2: Desktop tools and HTTP contract

**Files:** new `src/mcp/tools.rs`, HTTP/tool tests alongside the module, minimal `src/wayland.rs` test helpers only if needed.

- [x] Wrap the existing frame/input/controller in bounded tools with explicit schemas and correct MCP image content.
- [x] Make each control call acquire the existing gate and release held keys/buttons on success/cancel/deadline.
- [x] Exercise real HTTP initialize/list/call, scopes, physical restriction, busy gate, validation and input/capture reuse; add isolated native smoke coverage where practical.

### Task 3: Browser settings and documentation

**Files:** new `web/src/McpSettings.tsx`, `web/src/App.tsx`, `web/src/i18n.ts`, scoped CSS, new `tests/mcp.test.tsx`, `docs/mcp.md`, `README.md`, `docs/third-party.md`.

- [x] Add enable toggle, endpoint, token list/create/revoke and once-only token display using authenticated same-origin fetch.
- [x] Test settings load/mutations/errors, once-only token handling, no credential persistence, and app integration.
- [x] Document client configuration, tool semantics, scope, persistence, TLS proxy usage and precise Cua compatibility.
- [x] Run normal/bundled workspace tests, clippy, frontend tests/build; inspect diff, commit/push feature branch and request parent review/integration.

## Execution record

- Browser same-origin GET fetches omit Origin. Management requires the custom header plus same-origin Fetch Metadata for GET; mutations require the exact Origin. A native test exposed duplicate Origin headers; a failing HTTP regression was added, then the MCP routes were changed to reject duplicates. Existing WebSocket origin rules remain shared.
- No application settings database existed, so MCP uses its own private atomic state file, with an explicit per-instance path override. No service or desktop configuration is modified.
- Input cleanup now reports an unacknowledged worker release as failure; cancellation/deadlines keep the controller gate until cleanup finishes.
- UI was checked on the actual isolated candidate at 1280 pixels and at 390 × 844, including enable, one-time creation/Done and revoke. Screenshots are temporary test artifacts; no plaintext tokens were captured.
- Normal/bundled workspace tests, frontend tests/build, native MCP smoke, clippy and notice generation are the completion gates. Parent performs the independent whole-branch review and integration; this agent does not merge or release.

Final local verification on October 23, 2026 passed: `cargo fmt --all --check`,
normal and bundled-JPEG locked workspace tests (including 12 MCP tests), bundled
all-target Clippy with `-D warnings`, `cargo test --test mcp_live -- --ignored
--nocapture`, `npm run build`, and `npm test` (84 Node and 19 Vitest tests).
`mcp-notices.py` generated a 61,753-byte artifact with every expected locked
component and the pinned upstream SDK license. The build job explicitly selects
Python 3.12 for `tomllib`; MCP adds no system library or target-machine service
dependency. The isolated browser session and candidate process were stopped and
their private temporary runtime removed.
