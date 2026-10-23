#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

fail() { printf 'installer: %s\n' "$*" >&2; exit 1; }
usage() {
    printf '%s\n' 'Usage: install.sh [--binary FILE] [--activate] [--check|--dry-run]' \
        '                  [--rollback] [--test-root EXISTING_DIRECTORY]' \
        'Default: stage only. --test-root never invokes systemd or executes a binary.'
}

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source "$repository/scripts/activation.sh"
binary=$repository/target/release/lightos-remoted
root=
activate=0
check=0
rollback=0
while (($#)); do
    case $1 in
        --binary) (($# >= 2)) || fail '--binary requires a path'; binary=$2; shift 2 ;;
        --test-root) (($# >= 2)) && [[ -n $2 ]] || fail '--test-root requires a nonempty path'; root=$2; shift 2 ;;
        --activate) activate=1; shift ;;
        --check|--dry-run) check=1; shift ;;
        --rollback) rollback=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done
if [[ -n $root ]]; then
    [[ -d $root ]] || fail 'test root must already exist'
    root=$(realpath -e -- "$root")
    [[ $root != / ]] || fail 'test root must not be /'
    ((activate == 0)) || fail 'activation is forbidden with --test-root'
else
    ((check || EUID == 0)) || fail 'installation requires root'
fi

base=$root/usr/local/lib/lightos-remote-desktop
state=$root/var/lib/lightos-remote-desktop
unit=$root/etc/systemd/system/lightos-remote-desktop.service
override=$unit.d/90-native.conf
session_override=$unit.d/95-session.conf
session_unit=$root/etc/systemd/system/lightos-remote-desktop-session.service
session_template=$repository/systemd/lightos-remote-desktop-session.service
session_override_template=$repository/systemd/95-session.conf
environment=$root/etc/lightos/remote-desktop.env
native_environment=$root/etc/lightos/remote-desktop-native.env
service=lightos-remote-desktop.service
template=$repository/systemd/90-native.conf
transaction=
pending=0
activation_attempted=0
was_active=0

exists() { [[ -e $1 || -L $1 ]]; }
snapshot() {
    local source=$1 name=$2
    if exists "$source"; then cp -a -- "$source" "$transaction/$name"; fi
}
restore_file() {
    local destination=$1 name=$2 temporary
    if exists "$transaction/$name"; then
        temporary=$(mktemp "$(dirname -- "$destination")/.restore.XXXXXX") || return 1
        rm -f -- "$temporary" || return 1
        cp -a -- "$transaction/$name" "$temporary" || return 1
        mv -Tf -- "$temporary" "$destination"
    else
        rm -f -- "$destination"
    fi
}
restore() {
    restore_file "$base/current" current || return 1
    restore_file "$override" override || return 1
    restore_file "$session_override" session-override || return 1
    restore_file "$session_unit" session-unit
}
record_recovery() {
    if ! restore_file "$state/latest" latest || ! touch "$transaction/restored"; then
        printf 'RECOVERY FAILED: could not finalize rollback metadata in %s\n' "$transaction" >&2
        return 1
    fi
}
finish() {
    local status=$?
    trap - EXIT INT TERM
    if ((pending)); then
        printf '%s\n' 'Installer failed; restoring previous binary and override.' >&2
        set +e
        if ((activation_attempted)); then
            if recover_candidate; then
                record_recovery
            else
                printf 'RECOVERY FAILED: inspect service and retry rollback using %s\n' "$transaction" >&2
            fi
        elif restore; then
            record_recovery
        else
            printf 'RECOVERY FAILED: retry rollback using %s\n' "$transaction" >&2
        fi
    fi
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for command in realpath sha256sum install mktemp cp mv flock cmp; do
    command -v "$command" >/dev/null || fail "missing command: $command"
done
[[ -f $unit && ! -L $unit ]] || fail 'existing regular system unit required; it will not be replaced'
for directory in "$base" "$base/releases" "$state" "${override%/*}" "${environment%/*}"; do
    [[ $(realpath -m -- "$directory") == "$directory" ]] || fail "symlinked deployment directory: $directory"
done
[[ ! -L $environment && ! -L $native_environment && ! -L $override && ! -L $session_override && ! -L $session_unit ]] || fail 'environment files and managed override must not be symlinks'
if exists "$native_environment"; then [[ -f $native_environment ]] || fail 'native environment must be a regular file'; fi
if exists "$base/current"; then [[ -L $base/current ]] || fail 'current must be a symlink'; fi

if ((rollback)); then
    [[ -f $state/latest && ! -L $state/latest ]] || fail 'no previous installation'
    read -r identifier < "$state/latest"
    [[ $identifier =~ ^transaction\.[a-zA-Z0-9]+$ ]] || fail 'invalid transaction identifier'
    transaction=$state/$identifier
    [[ -d $transaction && ! -L $transaction && -f $transaction/ready && ! -e $transaction/restored ]] || fail 'no prepared installation to roll back'
else
    [[ -f $binary && -x $binary ]] || fail "prebuilt executable required: $binary"
    [[ -f $session_template && -f $session_override_template ]] || fail 'missing session service templates'
    # Read only the attachment setting, never source/evaluate an environment file.
    attached=0
    environment_sources=()
    for source_file in "$environment" "$native_environment"; do
        [[ ! -f $source_file ]] || environment_sources+=("$source_file")
    done
    if ((${#environment_sources[@]})); then
        attached=$(awk '
            /^[[:space:]]*LIGHTOS_REMOTE_DESKTOP_(WAYLAND_SOCKET|X11_DISPLAY)[[:space:]]*=/ {
                key=$0; sub(/[[:space:]]*=.*/, "", key); sub(/^[[:space:]]*/, "", key)
                value=$0; sub(/^[^=]*=[[:space:]]*/, "", value); sub(/[[:space:]]*$/, "", value)
                present[key]=(value != "" && value != "\"\"" && value != "\047\047")
            }
            END { print (present["LIGHTOS_REMOTE_DESKTOP_WAYLAND_SOCKET"] || present["LIGHTOS_REMOTE_DESKTOP_X11_DISPLAY"]) ? 1 : 0 }
        ' "${environment_sources[@]}")
    fi
    if ((!activate && !attached)); then
        # Staging never contacts systemd. Include local drop-ins in lexical order.
        user_sources=("$unit")
        for source_file in "$unit.d"/*.conf; do
            [[ ! -f $source_file ]] || user_sources+=("$source_file")
        done
        runtime_user=$(awk '
            /^[[:space:]]*\[/ { section=$0; gsub(/[[:space:]]/, "", section) }
            section=="[Service]" && /^[[:space:]]*User[[:space:]]*=/ {
                user=$0; sub(/^[^=]*=[[:space:]]*/, "", user); sub(/[[:space:]]*$/, "", user)
            }
            END { print user }
        ' "${user_sources[@]}")
        [[ $runtime_user =~ ^([a-zA-Z_][a-zA-Z0-9_-]*[$]?|[1-9][0-9]*)$ && $runtime_user != root ]] || fail 'a non-root User is required in the existing service or its drop-ins'
        session_contents=$(<"$session_template")
        session_contents=${session_contents//@USER@/$runtime_user}
    fi
    [[ -f $template ]] || fail 'missing systemd override template'
    binary_digest=$(sha256sum -- "$binary"); binary_digest=${binary_digest%% *}
    digest=$binary_digest
    worker=${binary%/*}/lightos-codec-ffmpeg
    worker_digest=
    if exists "$worker"; then
        [[ -f $worker && -x $worker && ! -L $worker ]] || fail 'optional codec worker must be a regular executable'
        worker_digest=$(sha256sum -- "$worker"); worker_digest=${worker_digest%% *}
        digest=$(printf 'codec-worker-v1\n%s\n%s\n' "$binary_digest" "$worker_digest" | sha256sum); digest=${digest%% *}
    fi
    release=$base/releases/$digest
    cursor_adapter=${binary%/*}/lightos-cursor.so
    cursor_digest=
    if exists "$cursor_adapter"; then
        [[ -f $cursor_adapter && -s $cursor_adapter && ! -L $cursor_adapter ]] || fail 'optional cursor adapter must be a regular file'
        cursor_digest=$(sha256sum -- "$cursor_adapter"); cursor_digest=${cursor_digest%% *}
        digest=$(printf 'cursor-adapter-v1\n%s\n%s\n' "$digest" "$cursor_digest" | sha256sum); digest=${digest%% *}
        release=$base/releases/$digest
    fi
    [[ ! -L $release ]] || fail 'release directory must not be a symlink'
    if exists "$release"; then
        [[ -f $release/lightos-remote-desktop && ! -L $release/lightos-remote-desktop ]] || fail 'invalid existing release'
        actual=$(sha256sum -- "$release/lightos-remote-desktop"); actual=${actual%% *}
        [[ $actual == "$binary_digest" ]] || fail 'existing release checksum mismatch'
        if [[ -n $worker_digest ]]; then
            [[ -f $release/lightos-codec-ffmpeg && ! -L $release/lightos-codec-ffmpeg ]] || fail 'existing release codec worker missing'
            actual=$(sha256sum -- "$release/lightos-codec-ffmpeg"); actual=${actual%% *}
            [[ $actual == "$worker_digest" ]] || fail 'existing release codec worker checksum mismatch'
        else
            ! exists "$release/lightos-codec-ffmpeg" || fail 'unexpected worker in software-only release'
        fi
        if [[ -n $cursor_digest ]]; then
            [[ -f $release/lightos-cursor.so && ! -L $release/lightos-cursor.so ]] || fail 'existing release cursor adapter missing'
            actual=$(sha256sum -- "$release/lightos-cursor.so"); actual=${actual%% *}
            [[ $actual == "$cursor_digest" ]] || fail 'existing release cursor adapter checksum mismatch'
        else
            ! exists "$release/lightos-cursor.so" || fail 'unexpected cursor adapter in release'
        fi
    fi
fi
if ((activate && !rollback)); then
    [[ -s $environment ]] || fail 'existing environment file with authentication secret required'
    awk '
        /^[[:space:]]*LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD[[:space:]]*=/ {
            value = $0
            sub(/^[^=]*=[[:space:]]*/, "", value)
            sub(/[[:space:]]*$/, "", value)
            present = (value != "" && value != "\"\"" && value != "\047\047")
        }
        END { exit !present }
    ' "$environment" || fail 'authentication secret is absent or empty'
    for command in systemctl systemd-run timeout; do command -v "$command" >/dev/null || fail "missing command: $command"; done
fi
if ((activate)); then
    for command in systemctl timeout; do command -v "$command" >/dev/null || fail "missing command: $command"; done
fi
if ((check)); then
    printf '%s\n' 'Read-only checks passed; no files changed, no doctor or systemd commands run.'
    exit 0
fi

install -d -m 0755 -- "$base" "$base/releases" "${override%/*}"
install -d -m 0700 -- "$state"
[[ ! -L $state/lock ]] || fail 'lock must not be a symlink'
exec 9>"$state/lock"
flock -n 9 || fail 'another installer is running'

if ((rollback)); then
    if ((activate)); then
        was_active=1
        recover_candidate || fail 'rollback could not restore a stable previous service; backup retained'
    else
        restore
    fi
    touch "$transaction/restored"
    printf '%s\n' 'Previous override and binary restored. Environment and original payload unchanged.'
    exit 0
fi

if [[ ! -d $release ]]; then
    staging=$(mktemp -d "$base/releases/.release.XXXXXX")
    install -m 0755 -- "$binary" "$staging/lightos-remote-desktop"
    actual=$(sha256sum -- "$staging/lightos-remote-desktop"); actual=${actual%% *}
    [[ $actual == "$binary_digest" ]] || fail 'binary changed while copying'
    if [[ -n $worker_digest ]]; then
        install -m 0755 -- "$worker" "$staging/lightos-codec-ffmpeg"
        actual=$(sha256sum -- "$staging/lightos-codec-ffmpeg"); actual=${actual%% *}
        [[ $actual == "$worker_digest" ]] || fail 'codec worker changed while copying'
    fi
    if [[ -n $cursor_digest ]]; then
        install -m 0644 -- "$cursor_adapter" "$staging/lightos-cursor.so"
        actual=$(sha256sum -- "$staging/lightos-cursor.so"); actual=${actual%% *}
        [[ $actual == "$cursor_digest" ]] || fail 'cursor adapter changed while copying'
    fi
    chmod 0755 "$staging"
    mv -T -- "$staging" "$release"
fi

if ((activate)); then
    runtime_user=$(activation_systemctl show --property=User --value "$service")
    [[ $runtime_user =~ ^([a-zA-Z_][a-zA-Z0-9_-]*[$]?|[1-9][0-9]*)$ && $runtime_user != root ]] || fail 'service must have a non-root User'
    session_contents=$(<"$session_template")
    session_contents=${session_contents//@USER@/$runtime_user}
    activation_doctor
fi

session_files_match() {
    if ((attached)); then
        [[ ! -e $session_override && ! -e $session_unit ]]
    else
        cmp -s -- "$session_override_template" "$session_override" && cmp -s <(printf '%s\n' "$session_contents") "$session_unit"
    fi
}
if [[ -L $base/current && $(readlink -- "$base/current") == "releases/$digest" ]] && cmp -s -- "$template" "$override" && session_files_match; then
    if ((activate)); then
        [[ -f $state/latest && ! -L $state/latest ]] || fail 'missing activation rollback snapshot'
        read -r identifier < "$state/latest"
        [[ $identifier =~ ^transaction\.[a-zA-Z0-9]+$ ]] || fail 'invalid transaction identifier'
        transaction=$state/$identifier
        [[ -f $transaction/ready && ! -e $transaction/restored ]] || fail 'missing activation rollback snapshot'
        pending=1
        if activation_systemctl is-active --quiet "$service"; then was_active=1; fi
        activation_attempted=1
        activate_candidate || fail 'candidate failed live health/PID/restart gate'
        pending=0
    fi
    printf '%s\n' 'Already staged; previous rollback snapshot retained.'
    exit 0
fi

transaction=$(mktemp -d "$state/transaction.XXXXXXXXXX")
snapshot "$unit" unit
snapshot "$environment" environment
snapshot "$native_environment" native-environment
snapshot "$override" override
snapshot "$session_override" session-override
snapshot "$session_unit" session-unit
snapshot "$base/current" current
snapshot "$state/latest" latest
if [[ ! -e $state/original ]]; then
    original=$(mktemp -d "$state/.original.XXXXXX")
    cp -a -- "$transaction/." "$original/"
    mv -T -- "$original" "$state/original"
fi
pending=1
touch "$transaction/ready"
temporary=$(mktemp "$state/.latest.XXXXXX")
printf '%s\n' "${transaction##*/}" > "$temporary"
mv -Tf -- "$temporary" "$state/latest"
temporary=$(mktemp "$base/.current.XXXXXX")
rm -f -- "$temporary"
ln -s -- "releases/$digest" "$temporary"
mv -Tf -- "$temporary" "$base/current"
temporary=$(mktemp "${override%/*}/.override.XXXXXX")
install -m 0644 -- "$template" "$temporary"
mv -Tf -- "$temporary" "$override"
if ((attached)); then
    rm -f -- "$session_override" "$session_unit"
else
    temporary=$(mktemp "${session_unit%/*}/.session.XXXXXX")
    printf '%s\n' "$session_contents" > "$temporary"
    chmod 0644 "$temporary"
    mv -Tf -- "$temporary" "$session_unit"
    temporary=$(mktemp "${session_override%/*}/.session-override.XXXXXX")
    install -m 0644 -- "$session_override_template" "$temporary"
    mv -Tf -- "$temporary" "$session_override"
fi
if ((activate)); then
    if activation_systemctl is-active --quiet "$service"; then was_active=1; fi
    activation_attempted=1
    activate_candidate || fail 'candidate failed live health/PID/restart gate'
fi
touch "$transaction/complete"
pending=0
printf '%s\n' "Staged release $digest; backup: $transaction"
if ((!activate)); then printf '%s\n' 'Not activated: no systemd command was run. Review docs/install.md before cutover.'; fi
