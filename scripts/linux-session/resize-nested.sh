#!/bin/sh
set -eu
[ "$#" = 3 ] || exit 1
width=$1; height=$2; fps=$3
case "$width:$height:$fps" in *[!0-9:]*|:*|*::*|*:) exit 1 ;; esac
[ "$width" -ge 320 ] && [ "$width" -le 3840 ]
[ "$height" -ge 200 ] && [ "$height" -le 2160 ]
kill -0 "${LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_PID:?}"
case "${LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_KIND:?}" in
  weston)
    export DISPLAY=${LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_DISPLAY:?}
    export XAUTHORITY=${LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_AUTHORITY:?}
    # The parent is the sole Weston window on this service's authenticated Xvfb.
    # Weston does not set _NET_WM_PID on its X11 output window.
    selected=""
    for window in $(xdotool search --name '.*'); do
      name=$(xdotool getwindowname "$window")
      case "$name" in
        'Weston Compositor - screen0')
          [ -z "$selected" ] || { echo 'Ambiguous private Weston output' >&2; exit 1; }
          selected=$window
          ;;
      esac
    done
    [ -n "$selected" ] || { echo 'Private Weston output is unavailable' >&2; exit 1; }
    exec xdotool windowsize --sync "$selected" "$width" "$height"
    ;;
  wlroots)
    export WAYLAND_DISPLAY=${LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_WAYLAND:?}
    exec wlr-randr --output HEADLESS-1 --custom-mode "${width}x${height}@${fps}Hz"
    ;;
  *) echo 'Unsupported owned nested display' >&2; exit 1 ;;
esac
