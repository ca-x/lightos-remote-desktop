#!/usr/bin/env bash
# Shared activation module consumes the configured globals below.
# shellcheck disable=SC2034
set -Eeuo pipefail
umask 077
fail() { printf 'lightos-remoted bootstrap: %s\n' "$*" >&2; exit 1; }
root=
snapshot_only=0
remove_package=0
while (($#)); do
    case $1 in
        --snapshot) snapshot_only=1; shift ;;
        --remove) snapshot_only=1; remove_package=1; shift ;;
        --test-root) [[ $# -ge 2 && -d $2 ]] || fail 'test root must exist'; root=$(realpath -e -- "$2"); [[ $root != / ]] || fail 'test root cannot be /'; shift 2 ;;
        *) fail "unknown argument: $1" ;;
    esac
done
[[ -n $root ]] || ((EUID == 0)) || fail 'must run as root'
ctl=systemctl
lookup=getent
adduser=useradd
if [[ -n $root ]]; then
    ctl=$root/test-bin/systemctl
    lookup=$root/test-bin/getent
    adduser=$root/test-bin/useradd
    for mock in "$ctl" "$lookup" "$adduser" "$root/test-bin/systemd-run"; do [[ -x $mock ]] || fail "missing fixture command: $mock"; done
fi
managed=$root/usr/lib/lightos-remoted
state=$root/var/lib/lightos-remoted-package
service_home=$root/var/lib/lightos-remoted
environment=$root/etc/lightos/remote-desktop.env
native_environment=$root/etc/lightos/remote-desktop-native.env
recovery=$root/run/systemd/system/lightos-remote-desktop.service.d/90-aur-recovery.conf
service=lightos-remote-desktop.service
for directory in "$managed" "$state" "$service_home" "${environment%/*}" "${recovery%/*}"; do
    [[ $(realpath -m -- "$directory") == "$directory" ]] || fail "symlinked deployment directory: $directory"
done
for file in "$environment" "$native_environment" "$state/aur-lock" "$recovery"; do
    [[ ! -L $file && ( ! -e $file || -f $file ) ]] || fail "regular file required: $file"
done
[[ -f $managed/lightos-remoted && -x $managed/lightos-remoted && ! -L $managed/lightos-remoted ]] || fail 'package binary missing'
# Legacy installations own their existing port/session/config. They are left
# running and never migrated by a pacman scriptlet.
legacy=0
if [[ -e $root/etc/systemd/system/lightos-remote-desktop.service || -e $root/etc/systemd/system/lightos-remote-desktop-session.service ]]; then
    legacy=1
elif "$ctl" is-active --quiet "$service"; then
    pid=$("$ctl" show --property=ExecMainPID --value "$service")
    executable=
    if [[ $pid =~ ^[1-9][0-9]*$ ]]; then executable=$(readlink -- "$root/proc/$pid/exe" 2>/dev/null || true); fi
    executable=${executable% (deleted)}
    case $executable in
        /usr/lib/lightos-remoted/lightos-remoted|/var/lib/lightos-remoted-package/previous/lightos-remoted) ;;
        *) legacy=1 ;;
    esac
fi
if ((legacy)); then
    printf '%s\n' 'Legacy lightos-remote-desktop installation detected; left unchanged.' 'New service activation skipped to preserve the existing desktop and port. See /usr/share/doc/lightos-remoted-bin/aur.md for migration.'
    exit 0
fi
install -d -m 0755 "$state"
exec 8>"$state/aur-lock"
flock -n 8 || fail 'another bootstrap is running'
if ((snapshot_only)); then
    # Once a failed upgrade has activated a previous copy, retain that known-good
    # copy rather than snapshotting the unsuccessful package payload.
    if [[ ! -e $recovery ]]; then
        [[ ! -L $state/previous ]] || fail 'previous directory cannot be a symlink'
        staging=$(mktemp -d "$state/.previous.XXXXXXXX")
        install -m 0755 "$managed/lightos-remoted" "$staging/lightos-remoted"
        if [[ -e $managed/lightos-codec-ffmpeg ]]; then
            [[ -f $managed/lightos-codec-ffmpeg && ! -L $managed/lightos-codec-ffmpeg ]] || fail 'codec helper must be a regular file'
            install -m 0755 "$managed/lightos-codec-ffmpeg" "$staging/lightos-codec-ffmpeg"
        fi
        rm -rf -- "$state/previous"
        chmod 0755 "$staging"
        mv -T -- "$staging" "$state/previous"
    fi
    if ((remove_package)); then "$ctl" disable "$service"; fi
    exit 0
fi
install -d -m 0755 "${environment%/*}"
if [[ ! -e $environment ]]; then
    password=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
    [[ $password =~ ^[0-9a-f]{64}$ ]] || fail 'failed to generate authentication secret'
    printf 'LIGHTOS_REMOTE_DESKTOP_ENABLE=1\nLIGHTOS_REMOTE_DESKTOP_USER=lightos-remoted\nLIGHTOS_REMOTE_DESKTOP_AUTH_USER=admin\nLIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD=%s\nLIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=/usr/lib/lightos-remoted/aquamarine\nLIGHTOS_REMOTE_DESKTOP_SHELL=minimal\n' "$password" > "$environment"
    unset password
fi
[[ -s $environment ]] || fail 'existing environment is empty; refusing to replace it'
# Parse only single-line literal systemd settings; never source an env file.
setting() {
    awk -v key="$1" '
        $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
            value=$0; sub(/^[^=]*=[[:space:]]*/, "", value); sub(/[[:space:]]*$/, "", value)
            if (value ~ /^".*"$/ || value ~ /^\047.*\047$/) value=substr(value,2,length(value)-2)
        } END { print value }
    ' "$environment"
}
runtime_user=$(setting LIGHTOS_REMOTE_DESKTOP_USER)
[[ -n $runtime_user ]] || runtime_user=lightos-remoted
[[ $runtime_user =~ ^[a-zA-Z_][a-zA-Z0-9_-]*[$]?$ && $runtime_user != root ]] || fail 'configured runtime user must be a nonroot username'
if ! "$lookup" passwd "$runtime_user" > /dev/null; then
    [[ $runtime_user == lightos-remoted ]] || fail 'configured runtime account does not exist'
    "$adduser" --system --user-group --home-dir /var/lib/lightos-remoted --create-home --shell /usr/bin/nologin lightos-remoted
fi
account=$("$lookup" passwd "$runtime_user")
IFS=: read -r account_name _ uid gid _ account_home _ <<< "$account"
[[ $account_name == "$runtime_user" && $uid =~ ^[1-9][0-9]*$ && $gid =~ ^[0-9]+$ && $account_home =~ ^/[a-zA-Z0-9_./-]+$ ]] || fail 'configured account must have a nonroot UID and a supported absolute home'
if [[ $runtime_user == lightos-remoted ]]; then
    [[ $account_home == /var/lib/lightos-remoted ]] || fail 'unexpected dedicated account home'
    install -d -m 0750 "$service_home"
    [[ -n $root ]] || chown "$uid:$gid" "$service_home"
fi
# Supply the compatibility library when reusing an existing auth-only environment;
# an existing native file is never overwritten.
install -d -m 0755 "${native_environment%/*}"
if [[ ! -e $native_environment && -z $(setting LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR) ]]; then
    printf '%s\n' 'LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=/usr/lib/lightos-remoted/aquamarine' > "$native_environment"
fi
for name in lightos-remote-desktop lightos-remote-desktop-session; do
    directory=$root/etc/systemd/system/$name.service.d
    [[ $(realpath -m -- "$directory") == "$directory" ]] || fail 'symlinked runtime-user drop-in directory'
    install -d -m 0755 "$directory"
    override=$directory/50-runtime-user.conf
    [[ ! -L $override && ( ! -e $override || -f $override ) ]] || fail 'runtime-user drop-in must be a regular file'
    temporary=$(mktemp "$directory/.runtime-user.XXXXXXXX")
    printf '[Service]\nUser=%s\nGroup=%s\nEnvironment=HOME=%s\n' "$runtime_user" "$gid" "$account_home" > "$temporary"
    chmod 0644 "$temporary"
    mv -Tf -- "$temporary" "$override"
done
# Reuse the tested doctor/probe/PID/restart stability gates with the new layout.
# shellcheck source=lightos-remoted-bin/activation.sh
source "$managed/activation.sh"
if [[ -n $root ]]; then
    activation_systemctl() { "$ctl" "$@"; }
    activation_systemd_run() { "$root/test-bin/systemd-run" "$@"; }
    activation_sleep() { :; }
fi
activation_systemctl daemon-reload
runtime_user=$(activation_systemctl show --property=User --value "$service")
[[ $runtime_user =~ ^([a-zA-Z_][a-zA-Z0-9_-]*[$]?|[1-9][0-9]*)$ && $runtime_user != root ]] || fail 'transport must have a nonroot User'
release=$managed
activation_doctor || fail 'candidate doctor failed; running service left unchanged'
was_active=0
if activation_systemctl is-active --quiet "$service"; then was_active=1; fi
backup=$(mktemp -d "$state/.activation.XXXXXXXX")
trap 'rm -rf -- "$backup"' EXIT
[[ ! -f $recovery ]] || cp -p "$recovery" "$backup/recovery.conf"
restore() {
    install -d -m 0755 "${recovery%/*}" || return 1
    if [[ -f $backup/recovery.conf ]]; then
        install -m 0644 "$backup/recovery.conf" "$recovery"
    elif [[ -x $state/previous/lightos-remoted ]]; then
        printf '[Service]\nExecStart=\nExecStart=/var/lib/lightos-remoted-package/previous/lightos-remoted serve\n' > "$recovery"
        chmod 0644 "$recovery"
    else
        rm -f -- "$recovery"
    fi
}
rm -f -- "$recovery"
if ! activate_candidate; then
    printf '%s\n' 'Candidate activation failed; recovering the previous transport, preserving the desktop owner.' >&2
    recover_candidate || fail 'transport recovery failed; inspect journalctl -u lightos-remote-desktop.service'
    fail 'candidate activation failed (previous transport recovered where available)'
fi
activation_systemctl enable "$service" || fail 'activation passed but enabling boot startup failed'
printf '%s\n' 'LightOS remote desktop activated. Credentials: /etc/lightos/remote-desktop.env (root only).'
