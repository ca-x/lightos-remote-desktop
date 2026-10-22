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
exec "$@"
