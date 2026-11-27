set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH LC_ALL=C
[ -d /run/systemd/system ] || exit 1
sleep_file=/etc/systemd/sleep.conf.d/zzzz-lazycat-remote.conf
lid_file=/etc/systemd/logind.conf.d/zzzz-lazycat-remote.conf
targets='suspend.target hibernate.target hybrid-sleep.target suspend-then-hibernate.target'
config() {
    systemd-analyze cat-config "systemd/$1.conf"
}
# Resolve the final assignment in systemd's own ordered configuration listing.
setting() {
    awk -v k="$1" -v section="$2" -v def="$3" '
        /^[[:space:]]*\[/ { s=$0; gsub(/[[:space:]]/,"",s); s=substr(s,2,length(s)-2); next }
        s==section && /^[[:space:]]*[A-Za-z]+[[:space:]]*=/ {
            key=$0; sub(/=.*/,"",key); gsub(/[[:space:]]/,"",key)
            if (key==k) { sub(/^[^=]*=/,""); gsub(/^[[:space:]]+|[[:space:]]+$/,""); v=tolower($0) }
        }
        END {print v=="" ? def : v}'
}
read_state() {
    sleep_conf=$(config sleep) || return 1
    lid_conf=$(config logind) || return 1
    loads=$(systemctl show -p Id -p LoadState sleep.target $targets) || return 1
    masked() { printf '%s\n' "$loads" | awk -v target="$1" 'BEGIN{RS="";FS="\n"} {id=masked=0;for(i=1;i<=NF;i++){if($i=="Id="target) id=1;if($i=="LoadState=masked")masked=1}if(id&&masked) found=1} END{exit !found}'; }
    for key in HandleLidSwitch HandleLidSwitchExternalPower HandleLidSwitchDocked; do
        case "$(printf '%s\n' "$lid_conf" | setting "$key" Login ignore)" in
            poweroff|reboot|halt|kexec) echo disabled; return;;
        esac
    done
    if masked sleep.target; then echo enabled; return; fi
    # Hybrid and suspend-then-hibernate inherit the two primary sleep modes.
    primary=yes
    for key in AllowSuspend AllowHibernation; do
        case "$(printf '%s\n' "$sleep_conf" | setting "$key" Sleep yes)" in
            0|no|false|off) primary=no;;
        esac
    done
    for pair in 'AllowSuspend suspend.target' 'AllowHibernation hibernate.target' 'AllowHybridSleep hybrid-sleep.target' 'AllowSuspendThenHibernate suspend-then-hibernate.target'; do
        set -- $pair
        default=yes
        case "$1" in AllowHybridSleep|AllowSuspendThenHibernate) default=$primary;; esac
        case "$(printf '%s\n' "$sleep_conf" | setting "$1" Sleep "$default")" in
            0|no|false|off) continue;;
        esac
        if ! masked "$2"; then echo disabled; return; fi
    done
    echo enabled
}
action=$1
actual=$(read_state)
case "$action:$actual" in read:*|enable:enabled|restore:disabled) echo "$actual"; exit;; esac
[ "$(id -u)" = 0 ] || exit 77
# Only the two drop-ins owned by this application are written or removed.
mkdir -p /etc/systemd/sleep.conf.d /etc/systemd/logind.conf.d
for file in "$sleep_file" "$lid_file"; do
    if [ -e "$file" ] || [ -L "$file" ]; then
        [ -f "$file" ] && [ ! -L "$file" ] && [ "$(head -n 1 "$file")" = '# Lazycat Remote power policy' ] || exit 1
    fi
done
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for kind in sleep lid; do
    case "$kind" in sleep) file=$sleep_file;; lid) file=$lid_file;; esac
    if [ -f "$file" ]; then cp -p "$file" "$work/$kind"; fi
done
write_file() {
    tmp=$(mktemp "$1.XXXXXX")
    { echo '# Lazycat Remote power policy'; printf '%s\n' "$2"; } >"$tmp"
    chmod 644 "$tmp"
    mv -f "$tmp" "$1"
}
reload_lid() { systemctl kill --kill-who=main --signal=HUP systemd-logind.service; }
undo() {
    for kind in sleep lid; do
        case "$kind" in sleep) file=$sleep_file;; lid) file=$lid_file;; esac
        if [ -f "$work/$kind" ]; then cp -p "$work/$kind" "$file"; else rm -f "$file"; fi
    done
    if [ -f "$work/unmasked" ]; then
        while read -r unit; do systemctl mask "$unit" >/dev/null || :; done < "$work/unmasked"
    fi
    reload_lid || :
}
trap 'rc=$?; if [ "$rc" -ne 0 ]; then undo; fi; rm -rf "$work"' EXIT
case "$action" in
enable)
    write_file "$sleep_file" '[Sleep]
AllowSuspend=no
AllowHibernation=no
AllowHybridSleep=no
AllowSuspendThenHibernate=no'
    write_file "$lid_file" '[Login]
HandleLidSwitch=ignore
HandleLidSwitchExternalPower=ignore
HandleLidSwitchDocked=ignore'
    want=enabled;;
restore)
    # Removing our overrides reveals the administrator's original configuration.
    rm -f "$sleep_file" "$lid_file"
    underlying=$(read_state)
    if [ "$underlying" = enabled ]; then
        # The machine already disallowed sleep without our files. Apply normal
        # systemd behaviour without editing the administrator's configuration.
        write_file "$sleep_file" '[Sleep]
AllowSuspend=yes
AllowHibernation=yes'
        write_file "$lid_file" '[Login]
HandleLidSwitch=suspend
HandleLidSwitchExternalPower=suspend
HandleLidSwitchDocked=ignore'
        for unit in sleep.target $targets; do
            if [ "$(readlink "/etc/systemd/system/$unit" || :)" = /dev/null ]; then
                echo "$unit" >> "$work/unmasked"
                systemctl unmask "$unit" >/dev/null
            fi
        done
    fi
    want=disabled;;
*) exit 1;;
esac
reload_lid
actual=$(read_state)
[ "$actual" = "$want" ]
echo "$actual"
