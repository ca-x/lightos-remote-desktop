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
binary=$repository/target/release/lightos-remote-desktop
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
    restore_file "$override" override
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
[[ ! -L $environment && ! -L $native_environment && ! -L $override ]] || fail 'environment files and managed override must not be symlinks'
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
    [[ -f $template ]] || fail 'missing systemd override template'
    digest=$(sha256sum -- "$binary"); digest=${digest%% *}
    release=$base/releases/$digest
    [[ ! -L $release ]] || fail 'release directory must not be a symlink'
    if exists "$release"; then
        [[ -f $release/lightos-remote-desktop && ! -L $release/lightos-remote-desktop ]] || fail 'invalid existing release'
        actual=$(sha256sum -- "$release/lightos-remote-desktop"); actual=${actual%% *}
        [[ $actual == "$digest" ]] || fail 'existing release checksum mismatch'
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
    [[ $actual == "$digest" ]] || fail 'binary changed while copying'
    chmod 0755 "$staging"
    mv -T -- "$staging" "$release"
fi

if ((activate)); then
    runtime_user=$(activation_systemctl show --property=User --value "$service")
    [[ -n $runtime_user && $runtime_user != root && $runtime_user != 0 ]] || fail 'service must have a non-root User'
    activation_doctor
fi

if [[ -L $base/current && $(readlink -- "$base/current") == "releases/$digest" ]] && cmp -s -- "$template" "$override"; then
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
if ((activate)); then
    if activation_systemctl is-active --quiet "$service"; then was_active=1; fi
    activation_attempted=1
    activate_candidate || fail 'candidate failed live health/PID/restart gate'
fi
touch "$transaction/complete"
pending=0
printf '%s\n' "Staged release $digest; backup: $transaction"
if ((!activate)); then printf '%s\n' 'Not activated: no systemd command was run. Review docs/install.md before cutover.'; fi
