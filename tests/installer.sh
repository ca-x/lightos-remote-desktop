#!/usr/bin/env bash
set -Eeuo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
installer=$repository/scripts/install.sh
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
root=$workspace/root
base=$root/usr/local/lib/lightos-remote-desktop
state=$root/var/lib/lightos-remote-desktop
unit=$root/etc/systemd/system/lightos-remote-desktop.service
override=$unit.d/90-native.conf
environment=$root/etc/lightos/remote-desktop.env
native_environment=$root/etc/lightos/remote-desktop-native.env
mkdir -p "$unit.d" "${environment%/*}" "$workspace/bin"
printf '[Service]\nSupplementaryGroups=kmem\n' > "$unit.d/10-lightos-render-groups.conf"
cp "$unit.d/10-lightos-render-groups.conf" "$workspace/legacy-groups.original"
printf '[Service]\nUser=czyt\nRuntimeDirectory=lightos-remote-desktop\nRestartSec=3\nExecStart=/original/payload\n' > "$unit"
printf 'LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD=$(touch %s/pwned)\n' "$workspace" > "$environment"
printf 'LD_LIBRARY_PATH=/private/aquamarine/lib\nNATIVE_TEST=$(touch %s/native-pwned)\n' "$workspace" > "$native_environment"
cp "$unit" "$workspace/unit.original"
cp "$environment" "$workspace/env.original"
cp "$native_environment" "$workspace/native-env.original"
printf '#!/bin/sh\ntouch "%s/executed"\nexit 1\n' "$workspace" > "$workspace/candidate"
chmod +x "$workspace/candidate"
for command in systemctl systemd-run; do
    printf '#!/bin/sh\ntouch "%s/host-systemd"\nexit 99\n' "$workspace" > "$workspace/bin/$command"
    chmod +x "$workspace/bin/$command"
done
export PATH="$workspace/bin:$PATH"
count=0
pass() { count=$((count + 1)); printf 'ok %s - %s\n' "$count" "$*"; }
run() { bash "$installer" --test-root "$root" --binary "$workspace/candidate" "$@"; }
reject() { if "$@" >"$workspace/rejected.log" 2>&1; then printf 'Unexpected success: %s\n' "$*" >&2; exit 1; fi; }

run --check
[[ ! -e $state && ! -e $base && ! -e $override ]]
pass 'check does not modify filesystem'
run --dry-run
[[ ! -e $state ]]
pass 'dry-run is read only'
reject run --activate
[[ ! -e $state ]]
pass 'test-root activation refused'
reject bash "$installer" --test-root / --binary "$workspace/candidate"
reject bash "$installer" --test-root '' --binary "$workspace/candidate"
reject run --unknown
reject bash "$installer" --binary
pass 'unsafe root and invalid arguments refused'
reject run --binary "$workspace/missing"
pass 'missing prebuilt binary refused'
run
first=$(readlink "$base/current")
first_snapshot=$(cat "$state/latest")
cmp "$unit" "$workspace/unit.original"
cmp "$environment" "$workspace/env.original"
cmp "$state/original/unit" "$unit"
cmp "$state/original/environment" "$environment"
cmp "$state/original/native-environment" "$native_environment"
cmp "$native_environment" "$workspace/native-env.original"
[[ ! -e $workspace/pwned && ! -e $workspace/executed && ! -e $workspace/host-systemd ]]
pass 'install preserves unit, secrets and payload; never sources environment or invokes systemd'
cmp "$base/current/lightos-remote-desktop" "$workspace/candidate"
cmp "$override" "$repository/systemd/90-native.conf"
[[ $(stat -c %a "$state") == 700 && $(stat -c %a "$base/current/lightos-remote-desktop") == 755 ]]
pass 'content-addressed executable, override and private backups installed'
awk '
    /^[[:space:]]*SupplementaryGroups[[:space:]]*=/ { count++ }
    END { exit count != 0 }
' "$override"
cmp "$unit.d/10-lightos-render-groups.conf" "$workspace/legacy-groups.original"
pass 'native override preserves inherited GPU groups without redefining supplementary groups'
awk '
    /^\[/ { section=$0 }
    /^Description=/ {
        count++
        if (section != "[Unit]" || $0 != "Description=LightOS Native Rust Remote Desktop") bad=1
    }
    END { exit count != 1 || bad }
' "$override"
pass 'native unit description identifies Rust instead of inherited Selkies'
run
[[ $(cat "$state/latest") == "$first_snapshot" ]]
pass 'same release is idempotent and preserves rollback snapshot'
printf '\nnew release\n' >> "$workspace/candidate"
run
[[ $(readlink "$base/current") != "$first" ]]
cmp "$state/$(cat "$state/latest")/override" "$repository/systemd/90-native.conf"
[[ -f $root/etc/systemd/system/lightos-remote-desktop-session.service ]]
grep -Fxq 'User=czyt' "$root/etc/systemd/system/lightos-remote-desktop-session.service"
grep -Fxq 'KillMode=control-group' "$root/etc/systemd/system/lightos-remote-desktop-session.service"
grep -Fxq 'Requires=lightos-remote-desktop-session.service' "$unit.d/95-session.conf"
! grep -Eq '^(PartOf|BindsTo)=' "$root/etc/systemd/system/lightos-remote-desktop-session.service"
pass 'upgrade snapshots preceding release'
run --rollback
[[ $(readlink "$base/current") == "$first" ]]
cmp "$override" "$repository/systemd/90-native.conf"
reject run --rollback
pass 'rollback restores preceding symlink and override exactly once'
cmp "$unit.d/10-lightos-render-groups.conf" "$workspace/legacy-groups.original"
printf '\nanother release\n' >> "$workspace/candidate"
previous_latest=$(cat "$state/latest")
cat > "$workspace/bin/mv" <<'EOF'
#!/usr/bin/env bash
if [[ ${*: -1} == */90-native.conf && ${*: -2:1} == */.override.* ]]; then exit 42; fi
exec /usr/bin/mv "$@"
EOF
chmod +x "$workspace/bin/mv"
reject run
[[ $(readlink "$base/current") == "$first" ]]
[[ $(cat "$state/latest") == "$previous_latest" ]]
cmp "$override" "$repository/systemd/90-native.conf"
rm "$workspace/bin/mv"
pass 'mid-transaction write failure restores preceding release and override'
run
run --rollback
cmp "$unit" "$workspace/unit.original"
cmp "$environment" "$workspace/env.original"
pass 'rollback never overwrites original unit or environment'
cmp "$native_environment" "$workspace/native-env.original"
printf 'LD_LIBRARY_PATH=/private/updated/lib\n' > "$native_environment"
cp "$native_environment" "$workspace/native-env.edited"
run
run --rollback
cmp "$native_environment" "$workspace/native-env.edited"
pass 'native env is backed up but never sourced, overwritten or reverted on rollback'
digest=$(sha256sum "$workspace/candidate"); digest=${digest%% *}
rm "$base/releases/$digest/lightos-remote-desktop"
reject run --check
pass 'corrupt existing release rejected'
root=$workspace/fresh
unit=$root/etc/systemd/system/lightos-remote-desktop.service
mkdir -p "${unit%/*}"
cp "$workspace/unit.original" "$unit"
run
run --rollback
[[ ! -e $root/usr/local/lib/lightos-remote-desktop/current && ! -L $root/usr/local/lib/lightos-remote-desktop/current ]]
[[ ! -e $unit.d/90-native.conf && ! -e $root/etc/lightos/remote-desktop.env ]]
[[ ! -e $root/etc/lightos/remote-desktop-native.env ]]
[[ ! -e $root/etc/systemd/system/lightos-remote-desktop-session.service && ! -e $unit.d/95-session.conf ]]
pass 'first-install rollback removes override, session unit and current; never creates an empty secret'
mkdir -p "$unit.d"
printf '[Service]\nEnvironment=ORIGINAL_OVERRIDE=yes\n' > "$unit.d/90-native.conf"
cp "$unit.d/90-native.conf" "$workspace/override.original"
run
run --rollback
cmp "$unit.d/90-native.conf" "$workspace/override.original"
pass 'rollback restores an existing managed override byte for byte'
(
    flock -n 8
    reject run
) 8>"$root/var/lib/lightos-remote-desktop/lock"
pass 'concurrent installer is refused'
root=$workspace/interrupted
unit=$root/etc/systemd/system/lightos-remote-desktop.service
mkdir -p "$unit.d"
cp "$workspace/unit.original" "$unit"
cp "$workspace/override.original" "$unit.d/90-native.conf"
cat > "$workspace/bin/mv" <<'EOF'
#!/usr/bin/env bash
if [[ ${*: -1} == */90-native.conf && ${*: -2:1} == */.override.* ]]; then
    kill -KILL "$PPID"
    exit 42
fi
exec /usr/bin/mv "$@"
EOF
chmod +x "$workspace/bin/mv"
reject run
rm "$workspace/bin/mv"
[[ -L $root/usr/local/lib/lightos-remote-desktop/current ]]
prepared=$(cat "$root/var/lib/lightos-remote-desktop/latest")
[[ -f $root/var/lib/lightos-remote-desktop/$prepared/ready && ! -e $root/var/lib/lightos-remote-desktop/$prepared/complete ]]
run --rollback
[[ ! -L $root/usr/local/lib/lightos-remote-desktop/current ]]
cmp "$unit.d/90-native.conf" "$workspace/override.original"
pass 'explicit rollback recovers SIGKILL between live path renames'
root=$workspace/symlink-root
mkdir -p "$root/usr/local/lib" "$workspace/elsewhere"
ln -s "$workspace/elsewhere" "$root/usr/local/lib/lightos-remote-desktop"
mkdir -p "$root/etc/systemd/system"
cp "$workspace/unit.original" "$root/etc/systemd/system/lightos-remote-desktop.service"
reject run
[[ ! -e $workspace/elsewhere/releases ]]
pass 'symlinked deployment directories cannot escape test root'
source "$repository/scripts/activation.sh"
service=lightos-remote-desktop.service
runtime_user=czyt
environment=$workspace/probe.env
native_environment=$workspace/probe-native.env
release=$workspace/release
activation_log=$workspace/activation.log
status_counter=$workspace/status-counter
scenario=healthy
recovery=0
activation_systemctl() {
    printf '%s\n' "systemctl $*" >> "$activation_log"
    case $1 in
        show)
            if [[ $* == *--value* ]]; then printf '100\n'; return; fi
            local count pid=200 restarts=0 exec=200 active=active
            count=$(cat "$status_counter"); count=$((count + 1)); printf '%s\n' "$count" > "$status_counter"
            if ((recovery)); then
                pid=300; exec=300
                if [[ $scenario == recovery-flaps ]] && ((count >= 3)); then restarts=1; fi
            else
                case $scenario in
                    same-pid) pid=100; exec=100 ;;
                    changed-pid) if ((count >= 3)); then pid=201; exec=201; fi ;;
                    restarted) if ((count >= 3)); then restarts=1; fi ;;
                    missing-exec) exec=0 ;;
                    inactive) active=failed ;;
                esac
            fi
            printf 'ActiveState=%s\nSubState=running\nMainPID=%s\nExecMainPID=%s\nNRestarts=%s\n' "$active" "$pid" "$exec" "$restarts"
            ;;
        restart) [[ $scenario != recovery-restart-failed && ( $scenario != restart-failed || $recovery == 1 ) ]] ;;
        *) return 0 ;;
    esac
}
activation_systemd_run() {
    local argument user_arguments=0
    local environment_arguments=()
    for argument in "$@"; do
        case $argument in
            --property=EnvironmentFile=*) environment_arguments+=("$argument") ;;
            --property=User=czyt) user_arguments=$((user_arguments + 1)) ;;
        esac
    done
    [[ $user_arguments == 1 && ${#environment_arguments[@]} == 2 ]] || return 1
    [[ ${environment_arguments[0]} == "--property=EnvironmentFile=$environment" &&
        ${environment_arguments[1]} == "--property=EnvironmentFile=-$native_environment" ]] || return 1
    if [[ ${*: -1} == doctor ]]; then
        printf '%s\n' "doctor $*" >> "$activation_log"
        return 0
    fi
    printf '%s\n' "probe $*" >> "$activation_log"
    [[ $* == *'--expect-pid 200'* && $* != *'--timeout-secs'* ]] || return 1
    case $scenario in
        unhealthy|wrong-native|wrong-health-pid|missing-probe) return 1 ;;
        probe-timeout) return 124 ;;
        late-unhealthy) [[ $(grep -c '^probe ' "$activation_log") == 1 ]] ;;
        *) return 0 ;;
    esac
}
activation_sleep() { printf 'sleep\n' >> "$activation_log"; }
restore() {
    printf 'restore\n' >> "$activation_log"
    [[ $scenario != restore-failed ]] || return 1
    cp "$workspace/recovery-backup" "$workspace/recovery-current" || return 1
    recovery=1
}
reset_activation() {
    scenario=$1
    recovery=0
    : > "$activation_log"
    printf '0\n' > "$status_counter"
    printf 'old binary and override\n' > "$workspace/recovery-backup"
    printf 'unhealthy candidate\n' > "$workspace/recovery-current"
}
reset_activation healthy
reject activation_systemd_run --property=User=czyt "--property=EnvironmentFile=$environment -$native_environment" -- candidate doctor
pass 'mock rejects a combined EnvironmentFile path instead of flattening argv'
activation_doctor
grep -q -- "--property=EnvironmentFile=$environment --property=EnvironmentFile=-$native_environment" "$activation_log"
grep -q -- 'RuntimeDirectory=lightos-remote-desktop-doctor' "$activation_log"
pass 'doctor shares required original and optional native env files with serve/probe'
activate_candidate
[[ $(grep -c '^sleep$' "$activation_log") == 5 ]]
[[ $(grep -c -- '--expect-pid 200' "$activation_log") == 2 ]]
grep -q -- "--property=EnvironmentFile=$environment --property=EnvironmentFile=-$native_environment" "$activation_log"
! grep -Eq '^systemctl (restart|stop) lightos-remote-desktop-session.service' "$activation_log"
pass 'activation verifies transport health and never restarts/stops the persistent session'
for failure in unhealthy wrong-native wrong-health-pid probe-timeout missing-probe late-unhealthy same-pid changed-pid restarted missing-exec inactive restart-failed; do
    reset_activation "$failure"
    reject activate_candidate
    was_active=1
    recover_candidate
    cmp "$workspace/recovery-current" "$workspace/recovery-backup"
    stop_line=$(grep -n '^systemctl stop ' "$activation_log" | cut -d: -f1)
    restore_line=$(grep -n '^restore$' "$activation_log" | cut -d: -f1)
    [[ $stop_line -lt $restore_line ]]
    [[ $(tail -1 "$activation_log") == 'systemctl show --property=ActiveState --property=SubState --property=MainPID --property=ExecMainPID --property=NRestarts lightos-remote-desktop.service' ]]
    pass "$failure fails activation and restores/restarts the previous service"
done
reset_activation unhealthy
was_active=0
recover_candidate
cmp "$workspace/recovery-current" "$workspace/recovery-backup"
! grep -q '^systemctl restart ' "$activation_log"
pass 'previously inactive service stays stopped after candidate recovery'
reset_activation restore-failed
was_active=1
reject recover_candidate
! grep -q '^systemctl restart ' "$activation_log"
pass 'failed disk restoration does not restart candidate or claim recovery'
for failure in recovery-flaps recovery-restart-failed; do
    reset_activation "$failure"
    was_active=1
    reject recover_candidate
    cmp "$workspace/recovery-current" "$workspace/recovery-backup"
    pass "$failure is reported as recovery failure, not success"
done
[[ ! -e $workspace/pwned && ! -e $workspace/native-pwned && ! -e $workspace/executed && ! -e $workspace/host-systemd ]]

root=$workspace/hardware-root
base=$root/usr/local/lib/lightos-remote-desktop
state=$root/var/lib/lightos-remote-desktop
unit=$root/etc/systemd/system/lightos-remote-desktop.service
mkdir -p "${unit%/*}"
cp "$workspace/unit.original" "$unit"
printf '#!/bin/sh\nexit 99\n' > "$workspace/lightos-codec-ffmpeg"
chmod 0755 "$workspace/lightos-codec-ffmpeg"
run
with_worker=$(readlink "$base/current")
cmp "$workspace/lightos-codec-ffmpeg" "$base/current/lightos-codec-ffmpeg"
printf '\nchanged worker\n' >> "$workspace/lightos-codec-ffmpeg"
run
[[ $(readlink "$base/current") != "$with_worker" ]]
cmp "$workspace/lightos-codec-ffmpeg" "$base/current/lightos-codec-ffmpeg"
pass 'optional sibling worker is staged and changes immutable bundle identity'
rm "$workspace/lightos-codec-ffmpeg"
run
[[ ! -e $base/current/lightos-codec-ffmpeg ]]
software=$(sha256sum "$workspace/candidate"); software=${software%% *}
[[ $(readlink "$base/current") == "releases/$software" ]]
pass 'software-only release keeps its original binary digest and no worker'
printf '#!/bin/sh\nexit 99\n' > "$workspace/lightos-codec-ffmpeg"
chmod 0600 "$workspace/lightos-codec-ffmpeg"
reject run
pass 'present but non-executable native worker is rejected before staging'

rm "$workspace/lightos-codec-ffmpeg"
printf 'cursor adapter fixture\n' > "$workspace/lightos-cursor.so"
run
with_cursor=$(readlink "$base/current")
cmp "$workspace/lightos-cursor.so" "$base/current/lightos-cursor.so"
printf 'updated adapter\n' >> "$workspace/lightos-cursor.so"
run
[[ $(readlink "$base/current") != "$with_cursor" ]]
cmp "$workspace/lightos-cursor.so" "$base/current/lightos-cursor.so"
pass 'cursor adapter is copied and participates in immutable release identity'
rm "$workspace/lightos-cursor.so"
run
[[ $(readlink "$base/current") == "releases/$software" && ! -e $base/current/lightos-cursor.so ]]
pass 'software-only rollback identity does not retain a cursor adapter'

printf 'PASS: %s installer behaviors (activation mocked); no host activation or binary execution.\n' "$count"
