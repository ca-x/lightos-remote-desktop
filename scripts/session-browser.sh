#!/bin/sh
set -eu
browser=$(xdg-settings get default-web-browser)
if [ -z "$browser" ]; then
    printf '%s\n' 'No default browser is configured' >&2
    exit 1
fi
exec gtk-launch "$browser" "$@"
