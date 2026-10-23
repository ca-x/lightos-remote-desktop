# Desktop switching

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

Physical mode attaches to an existing same-user Wayland or X11 session. The
resolver honors a validated `XDG_RUNTIME_DIR` and exact `WAYLAND_DISPLAY`, and
requires an explicit target when ambient protocols or multiple sockets are
ambiguous. A Wayland session is never replaced by an XWayland `DISPLAY`.
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
protocol, and physical audio is unavailable. Clipboard/input target the exact
selected session. No host desktop is started, stopped, unlocked or reconfigured.
Where logind identifies the exact target, physical locks clear video and release
input; the connection remains open for unlock and cannot return cached frames
or clipboard results across a lock transition. Failure to query a running
logind is not treated as an unlocked session.

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
