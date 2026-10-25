#!/bin/sh
set -eu
lightos_application_environment="$(dirname -- "$0")/../application-environment.sh"
if [ -f "$lightos_application_environment" ]; then
    . "$lightos_application_environment"
fi
unset lightos_application_environment
browser=$(env -u BROWSER xdg-settings get default-web-browser)
if [ -z "$browser" ]; then
    printf '%s\n' 'No default browser is configured' >&2
    exit 1
fi
if [ "${1:-}" = --private ]; then
    LIGHTOS_BROWSER_PRIVATE=1
    export LIGHTOS_BROWSER_PRIVATE
    shift
fi
exec gtk-launch -- "$browser" "$@"
