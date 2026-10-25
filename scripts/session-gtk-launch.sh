# Included after the generated real-program paths and private environment.
if [ "${1:-}" = -- ]; then shift; fi
case ${1:-} in
    */*|*..*|''|-*) exec "$LIGHTOS_REAL_PROGRAM" "$@" ;;
esac
lightos_desktop=$1
case $lightos_desktop in *.desktop) ;; *) lightos_desktop=$lightos_desktop.desktop ;; esac
case $lightos_desktop in
    google-chrome*.desktop|chromium*.desktop|brave*.desktop|microsoft-edge*.desktop|vivaldi*.desktop|opera*.desktop|helium*.desktop) ;;
    *) exec "$LIGHTOS_REAL_PROGRAM" "$@" ;;
esac
shift
lightos_entry=
lightos_data_home=${XDG_DATA_HOME:-$HOME/.local/share}
if [ -f "$lightos_data_home/applications/$lightos_desktop" ]; then
    lightos_entry=$lightos_data_home/applications/$lightos_desktop
else
    lightos_dirs=${XDG_DATA_DIRS:-/usr/local/share:/usr/share}
    while [ -n "$lightos_dirs" ]; do
        lightos_dir=${lightos_dirs%%:*}
        if [ -f "$lightos_dir/applications/$lightos_desktop" ]; then
            lightos_entry=$lightos_dir/applications/$lightos_desktop
            break
        fi
        case $lightos_dirs in *:*) lightos_dirs=${lightos_dirs#*:} ;; *) break ;; esac
    done
fi
if [ -z "$lightos_entry" ]; then exec "$LIGHTOS_REAL_PROGRAM" "$lightos_desktop" "$@"; fi
if [ -z "$LIGHTOS_REAL_GIO" ]; then printf '%s\n' 'LightOS: gio is required to launch the session browser' >&2; exit 1; fi
umask 077
lightos_copy=$LIGHTOS_DESKTOP_ENTRIES/$lightos_desktop
lightos_temporary=$(mktemp "$LIGHTOS_DESKTOP_ENTRIES/.entry.XXXXXXXX")
trap 'rm -f -- "$lightos_temporary"' EXIT HUP INT TERM
while IFS= read -r lightos_line || [ -n "$lightos_line" ]; do
    case $lightos_line in
        Exec=*) printf 'Exec=%s %s\n' "$LIGHTOS_BROWSER_EXEC" "${lightos_line#Exec=}" ;;
        DBusActivatable=*) printf '%s\n' 'DBusActivatable=false' ;;
        *) printf '%s\n' "$lightos_line" ;;
    esac
done < "$lightos_entry" > "$lightos_temporary"
mv -f -- "$lightos_temporary" "$lightos_copy"
trap - EXIT HUP INT TERM
LIGHTOS_BROWSER_DESKTOP_ID=$lightos_desktop
export LIGHTOS_BROWSER_DESKTOP_ID
# GIO retains desktop-entry argument/URI expansion and startup metadata.
exec "$LIGHTOS_REAL_GIO" launch -- "$lightos_copy" "$@"
