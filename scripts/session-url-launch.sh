# Included after the generated real-program paths and private environment.
case ${0##*/} in
    xdg-open)
        if [ "$#" = 1 ]; then
            case $1 in http://*|https://*) exec "$(dirname -- "$0")/omarchy-launch-browser" "$@" ;; esac
        fi
        ;;
    gio)
        if [ "${1:-}" = open ] && [ "$#" -gt 1 ]; then
            shift
            lightos_urls=1
            for lightos_argument in "$@"; do
                case $lightos_argument in http://*|https://*) ;; *) lightos_urls=0 ;; esac
            done
            if [ "$lightos_urls" = 1 ]; then exec "$(dirname -- "$0")/omarchy-launch-browser" "$@"; fi
            exec "$LIGHTOS_REAL_PROGRAM" open "$@"
        fi
        ;;
esac
exec "$LIGHTOS_REAL_PROGRAM" "$@"
