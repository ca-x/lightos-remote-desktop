#!/bin/sh
set -eu
if [ "${1:-}" != -- ]; then
    printf '%s\n' 'Remote desktop supports only: uwsm-app -- command [args...]' >&2
    exit 2
fi
shift
if [ "$#" -eq 0 ]; then
    exit 2
fi
lightos_application_environment="$(dirname -- "$0")/../application-environment.sh"
if [ -f "$lightos_application_environment" ]; then
    . "$lightos_application_environment"
    case ${1##*/} in
        google-chrome*|chrome|chromium|chromium-browser|brave*|microsoft-edge*|vivaldi*|opera|helium)
            exec "$(dirname -- "$0")/chromium-launch" "$@" ;;
    esac
fi
unset lightos_application_environment
exec "$@"
