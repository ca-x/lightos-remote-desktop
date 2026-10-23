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
activated=$state/activated.sha256
rejected=$state/rejected.sha256
for directory in "$managed" "$state" "$service_home" "${environment%/*}" "${recovery%/*}"; do
    [[ $(realpath -m -- "$directory") == "$directory" ]] || fail "symlinked deployment directory: $directory"
done
for file in "$environment" "$native_environment" "$state/aur-lock" "$recovery" "$activated" "$rejected"; do
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
payload_digest=$(sha256sum "$managed/lightos-remoted"); payload_digest=${payload_digest%% *}
if ((snapshot_only)); then
    # Only a payload that completed activation may replace the recovery copy.
    # Doctor/configuration failures happen before a recovery override exists.
    if [[ ! -e $recovery && -f $activated && $(<"$activated") == "$payload_digest" &&
        ( ! -f $rejected || $(<"$rejected") != "$payload_digest" ) ]]; then
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
    if ((remove_package)); then
        "$ctl" disable "$service"
        "$ctl" disable lightos-remote-desktop-session.service
    fi
    exit 0
fi
write_marker() {
    local destination=$1 temporary
    temporary=$(mktemp "$state/.activation-marker.XXXXXXXX") || return 1
    printf '%s\n' "$payload_digest" > "$temporary" || return 1
    mv -Tf -- "$temporary" "$destination"
}
backup=
finish() {
    local status=$?
    trap - EXIT
    if ((status)); then
        write_marker "$rejected" || printf '%s\n' 'Failed to record rejected candidate; retain the previous payload.' >&2
    fi
    [[ -z $backup ]] || rm -rf -- "$backup"
    exit "$status"
}
trap finish EXIT
install -d -m 0755 "${environment%/*}"
if [[ ! -e $environment ]]; then
    password=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
    [[ $password =~ ^[0-9a-f]{64}$ ]] || fail 'failed to generate authentication secret'
    printf 'LIGHTOS_REMOTE_DESKTOP_ENABLE=1\nLIGHTOS_REMOTE_DESKTOP_USER=lightos-remoted\nLIGHTOS_REMOTE_DESKTOP_AUTH_USER=admin\nLIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD=%s\nLIGHTOS_REMOTE_DESKTOP_SHELL=minimal\n' "$password" > "$environment"
    unset password
fi
[[ -s $environment ]] || fail 'existing environment is empty; refusing to replace it'
# Parse only single-line literal systemd settings; never source an env file.
setting() {
    local inputs=("$environment")
    [[ ! -f $native_environment ]] || inputs+=("$native_environment")
    awk -v key="$1" '
        $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
            value=$0; sub(/^[^=]*=[[:space:]]*/, "", value); sub(/[[:space:]]*$/, "", value)
            if (value ~ /^".*"$/ || value ~ /^\047.*\047$/) value=substr(value,2,length(value)-2)
        } END { print value }
    ' "${inputs[@]}"
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
if [[ -f $managed/aquamarine/libaquamarine.so.14 && ! -e $native_environment && -z $(setting LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR) ]]; then
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
# Remove only the exact paired workaround installed for v0.1.11. Modified,
# partial or symlinked administrator overrides remain administrator-owned.
short_owner=$root/etc/systemd/system/lightos-remote-desktop-session.service.d/60-short-runtime.conf
short_transport=$root/etc/systemd/system/lightos-remote-desktop.service.d/60-short-runtime.conf
if [[ -f $short_owner && ! -L $short_owner && -f $short_transport && ! -L $short_transport ]] &&
    cmp -s "$short_owner" <(printf '%s\n' '[Service]' 'Environment=XDG_RUNTIME_DIR=/run/lrd-session' 'RuntimeDirectory=' 'RuntimeDirectory=lrd-session' 'ExecStart=' 'ExecStart=/usr/lib/lightos-remoted/lightos-remoted desktop-session --runtime-dir /run/lrd-session' 'RuntimeDirectoryMode=0700') &&
    cmp -s "$short_transport" <(printf '%s\n' '[Service]' 'Environment=LIGHTOS_REMOTE_DESKTOP_SESSION_RUNTIME=/run/lrd-session'); then
    unlink -- "$short_owner"
    unlink -- "$short_transport"
fi
# Reuse the tested doctor/probe/PID/restart stability gates with the new layout.
# shellcheck source=scripts/service-activation.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/service-activation.sh"
if [[ -n $root ]]; then
    activation_systemctl() { "$ctl" "$@"; }
    activation_systemd_run() { "$root/test-bin/systemd-run" "$@"; }
    activation_sleep() { :; }
fi
activation_systemctl daemon-reload
# Later administrator drop-ins can override either unit. Resolve both effective
# identities through NSS; a username other than "root" may still have UID 0.
configured_uid=$uid
for checked_service in "$service" lightos-remote-desktop-session.service; do
    effective_user=$(activation_systemctl show --property=User --value "$checked_service")
    [[ $effective_user =~ ^([a-zA-Z_][a-zA-Z0-9_-]*[$]?|[0-9]+)$ ]] || fail "invalid effective User for $checked_service"
    effective_account=$("$lookup" passwd "$effective_user") || fail "effective User cannot be resolved for $checked_service"
    IFS=: read -r _ _ effective_uid _ <<< "$effective_account"
    [[ $effective_uid =~ ^[1-9][0-9]*$ && $effective_uid == "$configured_uid" ]] || fail "effective User for $checked_service must match configured nonroot UID"
    [[ $checked_service != "$service" ]] || runtime_user=$effective_user
done
release=$managed
activation_doctor || fail 'candidate doctor failed; running service left unchanged'
attached=$(setting LIGHTOS_REMOTE_DESKTOP_WAYLAND_SOCKET)
attached_x11=$(setting LIGHTOS_REMOTE_DESKTOP_X11_DISPLAY)
if [[ -n $attached || -n $attached_x11 ]]; then
    # A previously started owner stays alive until an explicit user cutover.
    activation_systemctl disable lightos-remote-desktop-session.service
else
    # Upgrades preserve the already-running owner and its applications.
    if ! activation_systemctl is-active --quiet lightos-remote-desktop-session.service; then
        activation_systemctl start lightos-remote-desktop-session.service
    fi
    activation_systemctl enable lightos-remote-desktop-session.service
fi
was_active=0
if activation_systemctl is-active --quiet "$service"; then was_active=1; fi
backup=$(mktemp -d "$state/.activation.XXXXXXXX")
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
    # Previous transport binaries predate the lower-priority service default.
    # Keep their persistent owner handoff when recovering an owned deployment.
    if [[ -f $recovery && -z $attached && -z $attached_x11 ]]; then
        printf '\n[Service]\nEnvironment=LIGHTOS_REMOTE_DESKTOP_SESSION_RUNTIME=/run/lrd-session\n' >> "$recovery"
    fi
}
rm -f -- "$recovery"
if ! activate_candidate; then
    printf '%s\n' 'Candidate activation failed; recovering the previous transport, preserving the desktop owner.' >&2
    recover_candidate || fail 'transport recovery failed; inspect journalctl -u lightos-remote-desktop.service'
    fail 'candidate activation failed (previous transport recovered where available)'
fi
activation_systemctl enable "$service" || fail 'activation passed but enabling boot startup failed'
write_marker "$activated"
rm -f -- "$rejected"
printf '%s\n' 'LightOS remote desktop activated. Credentials: /etc/lightos/remote-desktop.env (root only).'
