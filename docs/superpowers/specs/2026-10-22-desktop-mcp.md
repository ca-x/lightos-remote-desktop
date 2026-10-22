# Desktop MCP

## Objective

Expose the current remote desktop to MCP clients for visual inspection and bounded computer input. The authenticated browser settings manage whether MCP is enabled and create/revoke separate agent credentials. This is an MCP computer-use server, not an LLM agent, inference provider, or a replacement desktop.

## Scope and assumptions

- The endpoint is `<app-subfolder>/mcp`, using the official Rust `rmcp` SDK's token-bound Streamable HTTP sessions. Existing reverse proxies and TLS termination remain applicable. No additional listener or compositor starts.
- The default is disabled. Browser management endpoints require existing Basic authentication and same-origin `Origin`/fetch-site checks. MCP accepts only its own bearer tokens, with optional same-origin browser requests and no CORS allowance.
- Tokens have `observe` or `control` scope (`control` also permits observation). Creation is the only response containing the plaintext token; list/status responses expose only ID, name, scope, and creation time. The server uses OS randomness and SHA-256 from maintained crates, stores only hashes, and does not log secrets.
- Application-specific state is a private atomic JSON file under `$XDG_STATE_HOME/lightos-remote-desktop/mcp.json` (or `$HOME/.local/state/lightos-remote-desktop/mcp.json`). `--mcp-settings` / `LIGHTOS_REMOTE_DESKTOP_MCP_SETTINGS` selects an explicit absolute file. No existing service environment or desktop configuration is rewritten. Read/write errors fail closed for MCP without preventing normal browser desktop startup.
- Every HTTP request and every tool call validates current enable/revocation state. Disabling/revoking cancels outstanding actions. SDK sessions are isolated per authenticated token. Every HTTP request retains bearer validation; disable/revoke closes sessions, and re-enable requires fresh initialization.
- Capture reuses the existing fresh Wayland frame/JPEG path. Coordinates are whole pixels of that screenshot, mapped to the existing normalized pointer input. Resolution, output scale, mobile input, and IME behavior stay in the existing paths.
- Control targets only the application's owned virtual desktop. Attached physical desktops are observation-only over MCP. Each action acquires the existing controller gate; users can Disconnect the browser to hand over input. No shell, command execution, desktop selection, host settings mutation, clipboard, or resolution mutation tool is exposed.

## Interfaces

Management (JSON, authenticated and same-origin): `GET /api/mcp`, `PUT /api/mcp {enabled}`, `POST /api/mcp/tokens {name,scope}`, `DELETE /api/mcp/tokens/{id}`. Up to 32 active tokens; names are trimmed and limited to 80 characters. All responses use no-store. Failed persistence does not update runtime authorization state.

Tools: `desktop_info`, `screenshot`, `mouse_move`, `mouse_click`, `mouse_drag`, `scroll`, `key_press`, `type_text`. Observation tools return current screenshot dimensions/desktop kind/control availability; screenshot emits MCP image content with `image/jpeg`. Mouse buttons are left/middle/right; clicks and drags always release buttons. Keyboard input is a bounded chord of validated Linux evdev codes and always releases keys. Text uses the existing Unicode text injection command and its 16 KiB/control-character validation. Inputs reject unknown fields, nonfinite/out-of-bounds coordinates, oversized chords/text/scrolls and unsupported values. Tool work has a five-second deadline, and cancellation/drop always releases held input before the controller gate is available again.

## Cua boundary

The current trycua/cua repository describes Cua Driver as a computer-use daemon with CLI, MCP and typed SDK interfaces; its agent loop and inference are separate concerns. We expose standard MCP tools with documented image and input semantics. Existing Cua users can use an MCP-capable client or write a tool-name/coordinate adapter; this does not claim drop-in compatibility with Cua Driver's REST/WebSocket routes or every Cua Computer SDK method. No Cua packages or desktop images are installed.

## Acceptance and verification

1. HTTP MCP initialize, tools/list and tools/call work with a valid token; screenshot returns a decodable JPEG, and inputs use the existing frame/input paths.
2. Disabled, absent/invalid/Basic credentials, revoked tokens, wrong scopes and foreign-origin requests cannot operate tools. Disable/revoke interrupts outstanding work; stale session headers cannot bypass auth.
3. State survives a reload, contains no plaintext credentials, and is private/atomic. Corrupt, symlinked or insecure state fails closed.
4. Browser settings show enable state and token metadata, create a token once with clear copy/save guidance, revoke tokens, report failures and preserve desktop interaction. Browser requests never put credentials in URLs/localStorage.
5. The existing owned/physical boundary and controller exclusion remain effective; validation and cancellation leave no pressed input.
6. Workspace tests (normal and bundled JPEG), clippy, frontend tests/build, and HTTP integration tests pass. Any native smoke test runs only on an isolated candidate and never restarts the installed desktop owner or v2rayN.

Commands use `CARGO_HOME=/tmp/lrd-mcp-cargo` (registry symlink to the existing cache), `/home/czyt/.rustup/toolchains/1.99.1-x86_64-unknown-linux-gnu/bin/cargo`, and `mise exec node@24 -- npm ...`.

## Boundaries

Always: reuse capture/input/auth/controller validation; keep secrets out of URLs/logs; document limitations. Ask only for genuinely new scope. Never: change main/tag/version, deploy, edit host Omarchy config, restart the desktop owner or v2rayN, copy AGPL RustDesk code, or expose arbitrary execution.

Review correction: use SDK token-bound sessions so separate HTTP cancellation
notifications reach their original request. Newer initialize offers negotiate
to a supported 2025 session protocol. Internal queued input retains cancellation
and an absolute deadline; the Unicode worker checks both before batches/keys,
restores its base keymap, and acknowledges release before the gate is available.
