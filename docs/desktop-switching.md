# Desktop switching

Owned virtual sessions advertise `resize_scale` for automatic high-density
capture. Optional `resize.scale` accepts 1, 1.5, 2, 2.5 or 3; dimensions must fit
the existing encoder limits and divide into whole logical pixels at that scale.
Missing scale retains the legacy scale-1 behavior. Physical sessions reject
resizing as before.

The toolbar selects the desktop source, independently of fit, 1:1 and fullscreen.
Page reload defaults to the service's virtual desktop. A switch releases held
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

Failed connections automatically retry twice (after 500 ms and 1 s), then retain
the failure reason for manual recovery. Successful readiness resets the retry
budget. Disconnecting during physical initialization cancels that initialization;
the controller remains reserved until its workers are reclaimed. Worker cleanup
runs outside the async socket executor. Connection IDs and elapsed stage timings
are logged for accepted connections, physical discovery/readiness, the first sent
frame, closing and controller release; credentials and pixel contents are omitted.

Physical mode attaches to an existing same-user Wayland compositor. Discovery
uses sockets in `/run/user/<uid>`; `--physical-wayland-socket` can explicitly
identify a nonstandard socket. The browser never supplies a filesystem path.
No session means an unavailable option with an explanation. Connection or capture
failures must be visible. Physical mode cannot change monitor resolution, from
either the UI or the server protocol. Physical audio is currently unavailable.
Clipboard and input target the selected compositor; no host desktop is started,
stopped or reconfigured by attaching. Physical control requires compatible
screencopy and virtual-input Wayland protocols and may be denied by the compositor.

Verification covers selector reconnect/cleanup, same-origin and controller
exclusion, missing sockets, physical resize rejection, and return to the intact
virtual session. An existing isolated compositor can validate the attach path;
that is not proof of physical hardware compatibility.

The physical source section lists the compositor's active outputs, including
name, pixel dimensions, logical position and scale. This is Wayland output
discovery, not DRM connector probing: virtual/headless outputs may also appear.
The selected output is passed to both screencopy and the virtual-pointer v2
mapping, so control cannot silently span all monitors. A disconnected output
fails explicitly; refresh the list to select a replacement. To specify an
existing compositor outside the normal runtime directory:

```sh
lightos-remote-desktop serve --physical-wayland-socket /absolute/path/to/wayland-0
```

Keep authentication configured through the normal password environment variable.
The UI never changes this path and never starts or unlocks a physical login session.
