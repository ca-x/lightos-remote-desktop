# Desktop switching

The virtual/physical session discovery below describes Linux hosts. Windows and
macOS share the current logged-in physical desktop and do not create the Linux
virtual session; see [native host support](cross-platform-host.md).

Owned virtual sessions advertise `resize_scale` for automatic high-density
capture. Optional `resize.scale` accepts 1, 1.5, 2, 2.5 or 3; dimensions must fit
the existing encoder limits and divide into whole logical pixels at that scale.
Missing scale retains the legacy scale-1 behavior. Physical sessions reject
resizing as before.

The toolbar selects the desktop source, independently of fit, 1:1 and fullscreen.
Page reload uses the service's configured base desktop: normally the virtual
desktop, or the selected physical desktop for an explicitly attached service. A switch releases held
input and reconnects to the authenticated, same-origin WebSocket with
`?desktop=virtual` or `?desktop=physical`. Only one controller is allowed across
both targets. The virtual session remains alive while viewing the physical one.

New servers advertise `control_release: true` in `ready`. To switch, the owner
sends `{"type":"disconnect"}`. The server stops reading input and sending media,
releases held input (including a compositor roundtrip for the retained virtual
desktop), stops transient workers and frees the controller before replying with
`{"type":"released"}`. The browser may then connect the latest selected target
immediately, while the old WebSocket completes its transport close in the
background. Only the pending switch on that exact socket accepts this reply;
unsolicited or stale replies cannot start another connection. A missing reply
retains the three-second recovery bound. Older servers retain the Close-wait path.

Both media sockets finish the WebSocket close handshake before dropping their
transport. A TCP disconnect without the Close acknowledgement can leave a
gateway holding the browser in CLOSING for 60 seconds, before the next desktop
request is even sent. The client waits at most three seconds for closing and
then shows a recoverable failure. It does not automatically open a replacement
transport after this timeout; an explicit reconnect or another source selection
can recover, with the server still enforcing one controller. Late callbacks from
the abandoned socket cannot affect the replacement connection.

Wanted connections retry with jittered capped backoff, pause while offline and
resume online. Explicit disconnect cancels retries; successful readiness resets
the delay. Disconnecting during physical initialization cancels that initialization;
the controller remains reserved until its workers are reclaimed. Worker cleanup
runs outside the async socket executor. Connection IDs and elapsed stage timings
are logged for accepted connections, physical discovery/readiness, the first sent
frame, closing and controller release; credentials and pixel contents are omitted.

When a selected native Wayland monitor stalls during initialization, the service
can rebuild the same capture/input connections up to twice within the original
ten-second budget. The first two waits are bounded to one and two seconds;
the last attempt uses the remaining time. Each attempt closes and joins its
workers before retrying. Explicit errors, permission denial and cancellation
are not retried or redirected to another backend. Timeout diagnostics record
whether a captured frame existed and how many workers had not become ready.
The mobile tools menu remains available before readiness and after failure,
so the user can reconnect or return to the retained virtual desktop. Remote
input keys stay disabled until readiness; a disconnected view shows a recovery hint.

Physical mode attaches to an existing same-user Wayland or X11 session. The
resolver honors a validated `XDG_RUNTIME_DIR` and exact `WAYLAND_DISPLAY`, and
requires an explicit target when ambient protocols or multiple sockets are
ambiguous. A Wayland session is never replaced by an XWayland `DISPLAY`.
When transport is attached to an owned persistent desktop and has no explicit
`WAYLAND_DISPLAY`, physical discovery uses `/run/user/<uid>`: the transport's
private systemd runtime directory does not identify the graphical login.
`--physical-wayland-socket` can identify a nonstandard physical socket while the
service retains its owned virtual desktop. The browser never supplies a socket,
display or authority path. No session means an unavailable option with a reason.

The automatically selected backend may be native Wayland, Portal or X11.
Discovery reads bounded protocol capabilities without opening Portal consent;
only a connection requests authorization. Native Wayland requires the capture
and virtual-input versions actually used by libwayshot and the input adapter.
Portal requires one verified GNOME/KDE provider whose actual implementation and
Mutter/KWin compositor match the selected socket. A shared user bus is not proof
of desktop identity; ambiguous providers are unavailable without opening
consent. Portal shares RemoteDesktop, ScreenCast and clipboard under one
authorization session; its local chooser selects a monitor. A directly attached
Portal service waits for this explicit browser connection before requesting
consent. Provider restarts terminate the authorization session. Denied/cancelled authorization
ends that attempt. Closing the Portal ends the grant and a reconnect may require
new consent. The server announces pending Portal initialization so the browser
allows its bounded authorization wait. Denial, an initialization error or an
interrupted pending authorization requires explicit reconnect; network recovery
after a ready session retains the normal reconnect policy. Portal does not use
unrelated compositor output names to identify a granted PipeWire stream.

Physical mode cannot change monitor resolution, from either UI or server
protocol. Physical audio uses the selected session’s PulseAudio/PipeWire-Pulse
output monitor when available; it never falls back to a microphone or the
virtual desktop’s private sink. Clipboard/input target the exact
selected session. No host desktop is started, stopped, unlocked or reconfigured.
Where logind identifies the exact target, physical locks clear video and release
input; the connection remains open for unlock and cannot return cached frames
or clipboard results across a lock transition. Failure to query a running
logind is not treated as an unlocked session.

When a user-service compositor has no inherited login ID and logind cannot map
its PID directly, lock monitoring requires one local graphical login, logind's
matching user display/runtime, the socket peer's membership in the registered
user service, and exactly one owned Wayland socket in that runtime. Ambiguous
or unavailable evidence fails explicitly; the virtual transport's environment
does not identify the physical login and never bypasses the lock guard.

The physical source lists active outputs when native output discovery is
available, including pixel dimensions, position and scale. Wayland selected
outputs also constrain virtual-pointer v2 mapping. X11 can fall back to the
whole root window when RandR metadata is absent, but a named output must resolve
and its crop must fit the root geometry. Portal selection is performed in the
local consent chooser. Disconnected or invalid named outputs fail explicitly.

To select a nonstandard existing compositor while retaining the virtual desktop:

```sh
lightos-remoted serve --physical-wayland-socket /absolute/path/to/wayland-0
```

To run the service attached directly to an existing X11 desktop:

```sh
lightos-remoted serve --x11-display :0 --xauthority /absolute/path/to/Xauthority
```

Only local `:N[.screen]` displays are accepted. Explicit authority files must be
regular private files belonging to the desktop user; symlinks and wrong-owner
files are rejected. Capture, input, clipboard and RandR use that display and
authority. Existing desktop attachment does not create an RDP virtual session;
RDP remains restricted to the owned virtual desktop.

Verification covers synthetic Wayland registry/socket failures, isolated
same-user authenticated Xvfb capture/input/Unicode/clipboard and two-display
isolation, plus browser pause/resume on the same WebSocket. This does not prove
GNOME/KDE hardware or fractional-scaling compatibility; Portal and platform
limits are tracked in [runtime platforms](runtime-platforms.md).

Keep authentication configured through the normal password environment variable.
The UI never changes this path and never starts or unlocks a physical login session.

## Application launch ownership

Owned virtual desktops keep a private display and D-Bus activation environment.
Chromium-family browsers additionally receive a persistent, session-specific
`CHROME_CONFIG_HOME` under the user's XDG data directory. This prevents Chrome's
profile singleton from forwarding web-app launches to a physical browser.
Physical sessions keep their normal profile. Website logins in the virtual
browser are independent; existing physical profiles and cookies are not copied.

Session-local launchers cover Omarchy web apps, the default-browser entry,
GTK/URL activation, and installed Chromium-family command names. They preserve
arguments and reject a different runtime, bus or display endpoint. Native virtual
browser launches select Wayland even if the host Chrome flags force X11; an
explicit platform argument remains a caller override. Browser desktop entries
are adapted in the private runtime through GIO, leaving installed entries intact. New virtual
processes and private D-Bus activations inherit the same profile root. A transport
upgrade refreshes the running owned session's launchers without restarting its
compositor or applications. Existing windows stay on their original desktop;
long-running applications that launch a browser directly with an old inherited
environment may need to be reopened. Explicit `--user-data-dir` arguments remain
an advanced user override. This is application routing, not a same-user security
sandbox for every third-party application's singleton protocol.

These adapters are limited to tool-owned Wayland sessions. Attached physical
Wayland/X11 sessions are not given these scripts, and non-Chromium GTK entries
retain their original launcher. The service does not edit host Chrome flag files.
If Chrome itself is forced to X11 while the host X server is unavailable, repair
the host X11 setup or explicitly configure Chrome for the working Wayland session;
that host configuration issue is distinct from virtual-session routing.
