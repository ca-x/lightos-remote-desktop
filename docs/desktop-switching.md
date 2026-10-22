# Desktop switching

The toolbar selects the desktop source, independently of fit, 1:1 and fullscreen.
Page reload defaults to the service's virtual desktop. A switch releases held
input and reconnects to the authenticated, same-origin WebSocket with
`?desktop=virtual` or `?desktop=physical`. Only one controller is allowed across
both targets. The virtual session remains alive while viewing the physical one.

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
