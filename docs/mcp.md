# MCP desktop access

The desktop serves a standard MCP **Streamable HTTP** endpoint on the same port
and application prefix as the browser. MCP is disabled by default. It reuses the
service's existing capture and virtual input workers; enabling it does not start
another compositor, replace a desktop, install Cua, or restart any service.

## Enable and connect

1. Log in to the browser desktop and open **Settings** from the floating toolbar.
2. Select the **MCP** tab, enable MCP, and create a named token. **Observe
   only** permits screenshots and display inspection. **Observe & control** also
   permits mouse and keyboard input on the owned virtual desktop.
3. Save the displayed token immediately. It appears only in the creation response
   and is cleared when you leave the MCP tab or close Settings. The server cannot
   show it again; revoke and replace a lost token.
4. Give an MCP client the displayed endpoint and an `Authorization: Bearer …`
   header. Use your client's secret/environment storage for the token. The
   desktop's browser login/password is not an MCP credential.
5. To let an agent send input, use **Disconnect** in the browser toolbar. Screenshots
   can be read while the browser is connected. Input actions report “Desktop busy”
   while another controller holds the existing controller gate.

For a desktop at `https://desktop.example/remote/`, the MCP URL is
`https://desktop.example/remote/mcp`. A client that accepts URL/header-based remote
MCP configuration can use this shape; confirm its own configuration field names:

```json
{
  "mcpServers": {
    "lightos-desktop": {
      "url": "https://desktop.example/remote/mcp",
      "headers": { "Authorization": "Bearer <new MCP token>" }
    }
  }
}
```

Use HTTPS through the existing reverse proxy outside localhost. Forward
`Authorization`, `Accept`, `Content-Type`, `MCP-Protocol-Version` and
`Mcp-Session-Id` to the same app path, and retain the `Mcp-Session-Id` response
header. There is no second port, WebSocket upgrade, legacy `/sse` endpoint, OAuth
flow, or CORS allowlist to configure. A proxy that forces HTTP Basic authentication
in front of *every* path must let the MCP path reach the application's separate
bearer check; protect browser/management paths as before.

The transport negotiates the supported MCP session protocols
`2025-03-26`, `2025-06-18` and `2025-11-25`. A client offering a newer revision
receives `2025-11-25` during standard initialization. Read the `Mcp-Session-Id`
response header, send `notifications/initialized`, then include that session ID,
the negotiated `MCP-Protocol-Version` and the bearer token on subsequent POSTs.
Tool responses use SSE (`text/event-stream`). Clients must accept both JSON and
SSE. SDK sessions route `notifications/cancelled` to the original request, even
when the notification arrives on a separate HTTP connection. Sessions belong to
one token; another token cannot use their IDs. Every HTTP request still passes
current bearer authorization. Disable/revoke closes sessions, and re-enabling
requires new initialization. There are at most eight sessions per token; DELETE
with the session ID closes one, and idle sessions expire after five minutes.
GET supports the SDK session stream. Protocol revisions requiring stateless
operation are not advertised; clients use the version returned by initialize.
Service shutdown closes active sessions and cancels queued input before waiting
for HTTP connections to drain. The saved enable setting and token hashes remain
unchanged; clients initialize a new session after the transport restarts.

## Tools

| Tool | Scope | Parameters / result |
| --- | --- | --- |
| `desktop_info` | observe | `{}`; screenshot width/height, desktop kind, coordinate space, frame age, control availability/busy state |
| `screenshot` | observe | `{}`; dimensions plus an MCP `image` content block with base64 JPEG (`image/jpeg`) |
| `mouse_move` | control | `{x, y}` whole screenshot pixels |
| `mouse_click` | control | `{x, y, button?, count?}`; button `left` (default), `middle`, or `right`; count 1 (default) or 2 |
| `mouse_drag` | control | `{from_x, from_y, to_x, to_y, button?, duration_ms?}`; duration 50–1000 ms, default 250 |
| `scroll` | control | `{x, y}`; existing Wayland axis units in −1000..1000; positive is right/down |
| `key_press` | control | `{codes}`; 1–8 distinct Linux evdev codes in 1..767, modifiers first |
| `type_text` | control | `{text}`; Unicode text up to 16 KiB UTF-8; newline, carriage return and tab allowed |

Control scope includes observation. Pointer coordinates are zero-based whole
pixels of the latest screenshot: `0 <= x < width`, `0 <= y < height`. The server
maps them to its existing normalized input space, so browser zoom and compositor
scale do not change the coordinate convention. Observe again after resizing or
an action. Captures older than three seconds are rejected. Each tool has a
five-second deadline. HTTP MCP bodies are limited to 64 KiB (management bodies to
4 KiB), and unknown parameters, out-of-range values and unsupported
control characters are rejected before input.

For chords, Ctrl+C is `[29, 46]`, Ctrl+Alt+T is `[29, 56, 20]`, Enter is `[28]`,
Escape is `[1]`, and Super is `[125]`. Keys are pressed in the supplied order and
released in reverse order. `type_text` uses the existing Unicode virtual-keyboard
path, without using a shell or clipboard. Mouse buttons and keys are released at
the end of each action, on cancellation, or when credentials are disabled/revoked.
Queued input carries token/request cancellation and an absolute deadline into the
worker. Unicode input checks these before each batch and key pair, restores the
base keymap, and acknowledges cleanup before handing over control. HTTP
disconnection and `notifications/cancelled` both stop outstanding actions.
The controller gate stays occupied until the input worker acknowledges cleanup;
an unresponsive input worker is stopped by the existing release safety path.

MCP targets the service's primary desktop, normally its owned virtual desktop.
A browser's temporary switch to a physical monitor does not redirect MCP to that
monitor. If the service starts attached to an external physical session, MCP
observation remains available but all MCP input is refused. There are no tools
for changing resolution/scale, switching desktops, reading/writing clipboard,
launching programs, arbitrary scripts, shell commands or host configuration.
Input delivery is not verification of an application's response; screenshot again
to confirm the result.

## Token administration and storage

The authenticated browser uses these prefix-relative JSON routes:

- `GET api/mcp`: enabled state and token metadata (ID, name, scope, Unix creation time).
- `PUT api/mcp` with `{ "enabled": true | false }`.
- `POST api/mcp/tokens` with `{ "name": "Agent name", "scope": "observe" | "control" }`.
- `DELETE api/mcp/tokens/{id}`: revoke one token.

Management requires the existing browser Basic credentials, `X-LightOS-MCP: 1`,
and an exact same-origin `Origin` or same-origin Fetch Metadata for GET requests
that omit Origin. Cross-site requests and unauthenticated CORS preflights are
rejected. Tokens never appear in URLs, token lists, browser localStorage, or
application logs. Responses are `Cache-Control: no-store`.

Up to 32 active tokens are allowed. Names are trimmed and limited to 80 characters.
Each credential contains 256 bits of OS randomness; storage contains only its
SHA-256 digest. Disabling MCP cancels outstanding calls and blocks every request.
Re-enabling preserves existing unrevoked credentials. Revocation removes a
credential and cancels its outstanding calls. Persistence completes before an
administration change takes effect; failed writes leave the prior state intact.

Settings live at `$XDG_STATE_HOME/lightos-remote-desktop/mcp.json`, or
`$HOME/.local/state/lightos-remote-desktop/mcp.json` when XDG state is unset.
The app creates only its own directory/file, with directory mode 0700 and file
mode 0600. An existing file/directory must be private, owned by the desktop user,
and not a symlink. Writes use a private temporary file and atomic replacement.
This does not edit `/etc/lightos/*.env`, installed service configuration or
`~/.config/hypr/`.

Select a separate absolute path per independent instance with
`--mcp-settings /private/application-directory/mcp.json` or
`LIGHTOS_REMOTE_DESKTOP_MCP_SETTINGS`. The file belongs to a single running
instance; do not share it between concurrent servers. Missing files start
disabled. Corrupt, insecure or inaccessible state fails closed for MCP while the
normal browser desktop can still start. Correct the private file and restart only
the transport process to reload it; no desktop-owner restart is required. In
persistent-session deployments, the installed owner retains its desktop/apps
while the transport reconnects through the existing handoff mechanism.

## Cua compatibility

The referenced [trycua/cua repository](https://github.com/trycua/cua) currently
includes **Cua Driver**, which serves its own MCP tools over stdio and has CLI and
typed SDK interfaces. Its [agent connection guide](https://github.com/trycua/cua/blob/main/docs/content/docs/cua-driver/guides/connect-your-agent.mdx)
and [architecture](https://github.com/trycua/cua/blob/main/docs/content/docs/cua-driver/concepts/how-cua-driver-works.mdx)
separate the driver from the harness that owns the model and agent loop. We
checked those primary sources and its [current MCP tool registry](https://github.com/trycua/cua/blob/main/.github/scripts/cua-driver-mcp-compat/expected-tools.json)
on October 23, 2026.

This server provides the observation/input part through standard MCP. An
MCP-capable harness can connect directly using its remote HTTP support. A Cua
integration that expects Cua Driver names/arguments needs an explicit adapter,
for example `get_screen_size` → `desktop_info`, `get_desktop_state` → `screenshot`,
`move_cursor` → `mouse_move`, `click`/`double_click` → `mouse_click`, and named
keyboard keys → `key_press` evdev codes. Target objects, windows/accessibility
trees, browser DOM/CDP, background delivery, action-effect verification, Cua
sessions/REST/WebSocket routes, and the full Computer SDK contract are not
implemented. The server does not run an LLM, choose actions, provide inference,
or install Cua's desktop images. Do not configure a Cua SDK driver URL as this
MCP endpoint without an adapter.

The protocol implementation uses the official
[modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk)
`rmcp` 3.1.2 Streamable HTTP service, with no custom JSON-RPC/MCP framing.

## Verification

The regular workspace suite includes real HTTP initialize/list/call, bearer/scope/
origin checks, token reload/revoke, stale captures, screenshot decoding, resized
pixel coordinates, disconnect/revocation cancellation, and the five-second limit. Session tests also cover modern-version
negotiation,
token isolation, disable/re-enable, and cancellation on a separate HTTP request.
An additional opt-in native smoke test starts its own temporary compositor and
transport, reads the real capture and compositor cursor position, sends Unicode
input, and verifies the mounted management/revoke routes:

```sh
cargo test --test mcp_live -- --ignored --nocapture
cargo test --lib mcp::tests::native_text_revocation_between_batches_restores_keymap_and_skips_queued_input -- --ignored --nocapture
```

It requires the same installed Hyprland/cage dependencies as the service and the
private Aquamarine directory (auto-detected under the installed compat directory,
or set `LIGHTOS_TEST_AQUAMARINE_DIR`). It never restarts the installed desktop
owner or any other service. Frontend tests cover settings mutations, errors,
once-only token display, clipboard copy and late responses after closing.

The native text regression pauses the real Unicode worker after its first batch,
revokes the token, and checks that subsequent keys and canceled/expired queued
commands are skipped, the original keymap is restored, release is acknowledged,
and capture and input remain available.
