#!/usr/bin/env bash
# Standalone file transaction. Service lifecycle is shared with package installs.
set -Eeuo pipefail
umask 077
fail() { printf 'lightos-remoted installer: %s\n' "$*" >&2; exit 1; }
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
binary='' root='' stage=0 dry=0 rollback=0
while (($#)); do
    case $1 in
        --binary) binary=$2; shift 2 ;;
        --test-root) root=$(realpath -e -- "$2"); [[ $root != / ]] || fail 'test root cannot be /'; shift 2 ;;
        --stage-only) stage=1; shift ;;
        --dry-run) dry=1; shift ;;
        --rollback) rollback=1; shift ;;
        *) fail "unknown argument: $1" ;;
    esac
done
bootstrap_args=()
[[ -z $root ]] || bootstrap_args=(--test-root "$root")
managed=$root/usr/lib/lightos-remoted
state=$root/var/lib/lightos-remoted-install
transport=$root/usr/lib/systemd/system/lightos-remote-desktop.service
owner=$root/usr/lib/systemd/system/lightos-remote-desktop-session.service
cli=$root/usr/bin/lightos-remoted
for directory in "$managed" "$state" "${transport%/*}" "${cli%/*}"; do
    [[ $(realpath -m -- "$directory") == "$directory" ]] || fail "symlinked installation directory: $directory"
done
for path in "$managed/lightos-remoted" "$managed/lightos-codec-ffmpeg" "$transport" "$owner"; do
    [[ ! -L $path && ( ! -e $path || -f $path ) ]] || fail "expected regular file: $path"
done
if ((dry)); then
    printf '%s\n' 'Dry run: standalone layout uses /usr/lib/lightos-remoted and creates both service units; no files or services changed.'
    exit 0
fi
# A standalone install never replaces pacman-owned files, even identical files.
for path in usr/lib/lightos-remoted/lightos-remoted usr/lib/lightos-remoted/lightos-codec-ffmpeg usr/lib/systemd/system/lightos-remote-desktop.service usr/lib/systemd/system/lightos-remote-desktop-session.service usr/bin/lightos-remoted; do
    if [[ -z $root ]]; then
        if command -v pacman >/dev/null && pacman -Qo "/$path" >/dev/null 2>&1; then fail 'package-owned installation; upgrade using the package manager'; fi
    else
        for manifest in "$root"/var/lib/pacman/local/*/files; do
            [[ -f $manifest ]] || continue
            if grep -Fxq "$path" "$manifest"; then fail 'package-owned installation; upgrade using the package manager'; fi
        done
    fi
done
# Local overrides belong to the operator. Do not delete or silently migrate them.
for name in lightos-remote-desktop lightos-remote-desktop-session; do
    [[ ! -e $root/etc/systemd/system/$name.service && ! -L $root/etc/systemd/system/$name.service ]] || fail 'legacy/custom unit found; preserve the running desktop and migrate explicitly (or use --legacy)'
    directory=$root/etc/systemd/system/$name.service.d
    if [[ -d $directory ]]; then
        for override in "$directory"/*.conf; do
            [[ -e $override ]] || continue
            [[ ${override##*/} == 50-runtime-user.conf && -f $state/units.sha256 ]] || fail "custom service override requires explicit migration: $override"
        done
    fi
done
if [[ -e $transport || -e $owner || -e $managed/lightos-remoted || -e $cli || -L $cli ]]; then
    [[ -f $state/units.sha256 && ! -L $state/units.sha256 ]] || fail 'existing unmanaged installation; migrate explicitly'
    (cd "$root/" && sha256sum --check --status "$state/units.sha256") || fail 'installed units changed locally; preserve and migrate explicitly'
    [[ -L $cli && $(readlink -- "$cli") == /usr/lib/lightos-remoted/lightos-remoted ]] || fail 'existing CLI is not managed by this installer'
fi
if ((rollback)); then
    [[ -d $state/latest && ! -L $state/latest ]] || fail 'no standalone transaction to roll back'
    if ((!stage)) && [[ ! -f $state/latest/binary ]]; then fail 'first-install rollback would remove the desktop services; save work and use --stage-only after stopping them explicitly'; fi
else
    [[ -f $binary && -x $binary ]] || fail 'candidate executable missing'
fi

[[ -n $root || $EUID == 0 ]] || fail 'requires root'
# Test roots stage only unless an explicit isolated systemctl fixture is present.
if [[ -n $root && $stage == 0 ]]; then
    [[ -x $root/test-bin/systemctl ]] || fail 'test-root activation requires isolated fixtures or --stage-only'
fi
install -d -m 0755 "$managed" "$state" "${transport%/*}" "${cli%/*}"
exec 9>"$state/install.lock"
flock -n 9 || fail 'another standalone install is running'
# The package bootstrap retains only a payload that actually passed activation.
if ((!rollback && !stage)) && [[ -f $managed/lightos-remoted ]]; then
    bash "$repository/scripts/service-bootstrap.sh" "${bootstrap_args[@]}" --snapshot
fi
restore_files() {
    local snapshot=$1 destination name
    while IFS='|' read -r name destination; do
        if [[ -e $snapshot/$name || -L $snapshot/$name ]]; then
            rm -f -- "$destination"
            cp -a -- "$snapshot/$name" "$destination"
        else
            rm -f -- "$destination"
        fi
    done <<PATHS
binary|$managed/lightos-remoted
helper|$managed/lightos-codec-ffmpeg
transport|$transport
owner|$owner
cli|$cli
units.sha256|$state/units.sha256
PATHS
}
if ((rollback)); then
    restore_files "$state/latest"
    if ((!stage)) && [[ -f $managed/lightos-remoted ]]; then
        bash "$repository/scripts/service-bootstrap.sh" "${bootstrap_args[@]}"
    fi
    rm -rf -- "$state/latest"
    printf '%s\n' 'Standalone files restored; credentials and desktop owner preserved.'
    exit 0
fi
snapshot=$(mktemp -d "$state/.transaction.XXXXXXXX")
while IFS='|' read -r name source; do
    if [[ -e $source || -L $source ]]; then cp -a -- "$source" "$snapshot/$name"; fi
done <<PATHS
binary|$managed/lightos-remoted
helper|$managed/lightos-codec-ffmpeg
transport|$transport
owner|$owner
cli|$cli
units.sha256|$state/units.sha256
PATHS
# Rejected candidates must never become the explicit rollback payload either.
recovery_state=$root/var/lib/lightos-remoted-package
if [[ -f $managed/lightos-remoted && -f $recovery_state/rejected.sha256 && -f $recovery_state/previous/lightos-remoted ]]; then
    digest=$(sha256sum "$managed/lightos-remoted"); digest=${digest%% *}
    if [[ $(<"$recovery_state/rejected.sha256") == "$digest" ]]; then
        install -m 0755 "$recovery_state/previous/lightos-remoted" "$snapshot/binary"
        if [[ -f $recovery_state/previous/lightos-codec-ffmpeg ]]; then install -m 0755 "$recovery_state/previous/lightos-codec-ffmpeg" "$snapshot/helper"; fi
    fi
fi
# Recover file-write failures. Activation failures use the shared known-good payload.
pending=1
finish() {
    local status=$?
    trap - EXIT
    if ((pending)); then restore_files "$snapshot" || printf '%s\n' 'File recovery failed; retain transaction for manual recovery.' >&2; fi
    exit "$status"
}
trap finish EXIT
candidate=$(mktemp "$managed/.candidate.XXXXXXXX")
install -m 0755 "$binary" "$candidate"
mv -Tf -- "$candidate" "$managed/lightos-remoted"
helper=${binary%/*}/lightos-codec-ffmpeg
if [[ -e $helper ]]; then
    [[ -f $helper && -x $helper && ! -L $helper ]] || fail 'codec helper must be a regular executable'
    install -m 0755 "$helper" "$managed/lightos-codec-ffmpeg"
fi
install -m 0644 "$repository/systemd/lightos-remoted-transport.service" "$transport"
install -m 0644 "$repository/systemd/lightos-remoted-session.service" "$owner"
ln -sfn /usr/lib/lightos-remoted/lightos-remoted "$cli"
(cd "$root/" && sha256sum usr/lib/systemd/system/lightos-remote-desktop.service usr/lib/systemd/system/lightos-remote-desktop-session.service) > "$state/units.sha256"
rm -rf -- "$state/latest"
mv -T -- "$snapshot" "$state/latest"
pending=0
if ((!stage)); then
    bash "$repository/scripts/service-bootstrap.sh" "${bootstrap_args[@]}"
fi
printf '%s\n' 'Standalone binary and both service units installed.'
