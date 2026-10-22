#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

fail() { printf 'compat installer: %s\n' "$*" >&2; exit 1; }
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
build=$repository/.local/aquamarine/build
license=
root=
check=0
rollback=0
while (($#)); do
    case $1 in
        --build-dir) (($# >= 2)) || fail '--build-dir requires a directory'; build=$2; shift 2 ;;
        --license) (($# >= 2)) && [[ -n $2 ]] || fail '--license requires a file'; license=$2; shift 2 ;;
        --test-root) (($# >= 2)) && [[ -n $2 ]] || fail '--test-root requires a nonempty directory'; root=$2; shift 2 ;;
        --check|--dry-run) check=1; shift ;;
        --rollback) rollback=1; shift ;;
        --help|-h) printf '%s\n' 'Usage: install-compat.sh [--build-dir DIR] [--license FILE] [--check|--dry-run] [--rollback] [--test-root DIR]'; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done
for command in realpath sha256sum pkg-config readelf awk install mktemp flock cmp stat; do
    command -v "$command" >/dev/null || fail "missing command: $command"
done
if [[ -n $root ]]; then
    [[ -d $root ]] || fail 'test root must exist'
    root=$(realpath -e -- "$root")
    [[ $root != / ]] || fail 'test root must not be /'
else
    ((check || EUID == 0)) || fail 'installation requires root'
fi
base=$root/usr/local/lib/lightos-remote-desktop
state=$root/var/lib/lightos-remote-desktop
environment=$root/etc/lightos/remote-desktop-native.env
latest=$state/compat-latest
for directory in "$base" "$base/compat" "$state" "${environment%/*}"; do
    [[ $(realpath -m -- "$directory") == "$directory" ]] || fail "symlinked deployment directory: $directory"
done
for file in "$environment" "$latest" "$state/lock"; do
    [[ ! -L $file ]] || fail "symlink not allowed: $file"
    [[ ! -e $file || -f $file ]] || fail "regular file required: $file"
done
transaction=
temporary=
staging=
pending=0
restore_file() {
    local destination=$1 name=$2 restore_temp
    if [[ -f $transaction/$name ]]; then
        restore_temp=$(mktemp "${destination%/*}/.compat-restore.XXXXXX") || return 1
        cp -p -- "$transaction/$name" "$restore_temp" || return 1
        mv -Tf -- "$restore_temp" "$destination"
    else
        rm -f -- "$destination"
    fi
}
finish() {
    local status=$?
    trap - EXIT INT TERM
    if ((pending)); then
        if restore_file "$environment" native-environment && restore_file "$latest" previous-latest; then
            printf '%s\n' 'Failed compat installation: previous native env restored.' >&2
        else
            printf 'RECOVERY FAILED: retain backup %s\n' "$transaction" >&2
        fi
    fi
    [[ -z $temporary ]] || rm -f -- "$temporary"
    [[ -z $staging ]] || rm -rf -- "$staging"
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

render_receipt() {
    printf 'upstream=https://github.com/hyprwm/aquamarine.git\nversion=0.15.1\ncommit=f31c47a1b9d300847d8fc3108ad959448103dfc1\nsoname=libaquamarine.so.14\nsha256=%s\npatch_sha256=%s\nlicense_sha256=%s\n' "$digest" "$patch_digest" "$license_digest"
}

render_native_env() {
    if [[ -f $environment ]]; then
        awk '
            /^[[:space:]]*([#;]|$)/ { print; next }
            {
                if ($0 !~ /^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=/) exit 1
                value=$0; sub(/^[^=]*=/, "", value)
                quote=""; escaped=0
                for (position=1; position<=length(value); position++) {
                    character=substr(value, position, 1)
                    if (escaped) { escaped=0; continue }
                    if (character == "\\" && quote != "\047") { escaped=1; continue }
                    if (quote != "") { if (character == quote) quote="" }
                    else if (character == "\047" || character == "\"") quote=character
                }
                if (quote != "" || escaped) exit 1
                if ($0 !~ /^[[:space:]]*LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR[[:space:]]*=/) print
            }
        ' "$environment" || return 1
    fi
    printf 'LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=%s\n' "$stable"
}

read_transaction() {
    local identifier
    [[ -f $latest ]] || fail 'no compat installation to roll back'
    read -r identifier < "$latest"
    [[ $identifier =~ ^compat-transaction\.[a-zA-Z0-9]+$ ]] || fail 'invalid compat transaction'
    transaction=$state/$identifier
    [[ -d $transaction && ! -L $transaction && -f $transaction/ready && ! -e $transaction/restored ]] || fail 'no prepared compat transaction'
}

if ((rollback)); then
    read_transaction
else
    [[ -d $build ]] || fail 'build directory missing; run scripts/build-aquamarine.sh first'
    build=$(realpath -e -- "$build")
    if [[ -z $license ]]; then license=$build/../LICENSE; fi
    [[ -f $license && -s $license ]] || fail 'nonempty source LICENSE required; use --license FILE if not beside the build directory'
    library=$build/libaquamarine.so.0.15.1
    receipt=$build/compatibility.txt
    patch=$repository/patches/aquamarine-wayland.patch
    [[ -f $library && -f $build/aquamarine.pc && -f $receipt && -f $patch ]] || fail 'library, pkg-config metadata, patch and build receipt required'
    [[ $(pkg-config --modversion "$build/aquamarine.pc") == 0.15.1 ]] || fail 'pkg-config version must be 0.15.1'
    sonames=$(LC_ALL=C readelf -d -- "$library" | awk '/\(SONAME\)/ { print $NF }')
    [[ $sonames == '[libaquamarine.so.14]' ]] || fail 'ELF SONAME must be libaquamarine.so.14'
    for metadata in 'upstream=https://github.com/hyprwm/aquamarine.git' 'version=0.15.1' \
        'commit=f31c47a1b9d300847d8fc3108ad959448103dfc1' 'soname=libaquamarine.so.14'; do
        awk -F= -v key="${metadata%%=*}" -v expected="$metadata" \
            '$1 == key { count++; if ($0 != expected) bad=1 } END { exit count != 1 || bad }' \
            "$receipt" || fail 'unexpected pinned build receipt'
    done
    digest=$(sha256sum -- "$library"); digest=${digest%% *}
    patch_digest=$(sha256sum -- "$patch"); patch_digest=${patch_digest%% *}
    license_digest=$(sha256sum -- "$license"); license_digest=${license_digest%% *}
    for source in "$library" "$patch"; do
        expected=$(sha256sum -- "$source"); expected=${expected%% *}
        awk -v path="$source" -v expected="$expected" '
            substr($0, 67) == path { count++; if (substr($0, 1, 64) != expected) bad=1 }
            END { exit count != 1 || bad }
        ' "$receipt" || fail 'receipt checksum mismatch; rebuild the pinned compatibility library'
    done
    stable=/usr/local/lib/lightos-remote-desktop/compat/$digest
    destination=$root$stable
    [[ ! -L $destination ]] || fail 'compat release must not be a symlink'
    if [[ -e $destination ]]; then
        [[ -d $destination && -f $destination/libaquamarine.so.14 && ! -L $destination/libaquamarine.so.14 ]] || fail 'invalid existing compat release'
        actual=$(sha256sum -- "$destination/libaquamarine.so.14"); actual=${actual%% *}
        [[ $actual == "$digest" ]] || fail 'existing compat release checksum mismatch'
        [[ -f $destination/LICENSE && ! -L $destination/LICENSE && -f $destination/receipt.txt && ! -L $destination/receipt.txt ]] || fail 'existing compat release requires regular LICENSE and receipt'
        actual=$(sha256sum -- "$destination/LICENSE"); actual=${actual%% *}
        [[ $actual == "$license_digest" ]] || fail 'existing compat release LICENSE mismatch'
        cmp -s "$destination/receipt.txt" <(render_receipt) || fail 'existing compat release receipt mismatch'
        if [[ -z $root ]]; then
            [[ $(stat -c %u "$destination") == 0 && $(stat -c %u "$destination/libaquamarine.so.14") == 0 \
                && $(stat -c %a "$destination") == 755 && $(stat -c %a "$destination/libaquamarine.so.14") == 755 ]] || fail 'existing compat release must be root-owned and mode 0755'
            for file in "$destination/LICENSE" "$destination/receipt.txt"; do
                [[ $(stat -c %u "$file") == 0 && $(stat -c %a "$file") == 644 ]] || fail 'existing release notices must be root-owned and mode 0644'
            done
        fi
    fi
fi
if ((check)); then
    if ((!rollback)); then render_native_env > /dev/null || fail 'native env requires single-line assignments'; fi
    printf '%s\n' 'Compat read-only checks passed; no files changed or service/library executed.'
    exit 0
fi
ownership=()
if [[ -z $root ]]; then ownership=(-o 0 -g 0); fi
install -d "${ownership[@]}" -m 0755 -- "$base" "$base/compat" "${environment%/*}"
install -d "${ownership[@]}" -m 0700 -- "$state"
exec 9>"$state/lock"
flock -n 9 || fail 'another installer is running'
if ((rollback)); then
    read_transaction
    restore_file "$environment" native-environment
    touch "$transaction/restored"
    printf '%s\n' 'Previous native env restored; no service changes. Compat releases retained.'
    exit 0
fi
if [[ ! -d $destination ]]; then
    staging=$(mktemp -d "$base/compat/.release.XXXXXX")
    install -m 0755 -- "$library" "$staging/libaquamarine.so.14"
    actual=$(sha256sum -- "$staging/libaquamarine.so.14"); actual=${actual%% *}
    [[ $actual == "$digest" ]] || fail 'library changed during copy'
    install -m 0644 -- "$license" "$staging/LICENSE"
    actual=$(sha256sum -- "$staging/LICENSE"); actual=${actual%% *}
    [[ $actual == "$license_digest" ]] || fail 'LICENSE changed during copy'
    render_receipt > "$staging/receipt.txt"
    chmod 0644 "$staging/receipt.txt"
    chmod 0755 "$staging"
    mv -T -- "$staging" "$destination"
    staging=
fi
temporary=$(mktemp "${environment%/*}/.native-env.XXXXXX")
render_native_env > "$temporary" || fail 'native env must use single-line assignments; multiline/unsupported syntax left unchanged'
if [[ -f $environment ]] && cmp -s -- "$environment" "$temporary" && [[ $(stat -c %a "$environment") == 600 ]] \
    && { [[ -n $root ]] || [[ $(stat -c %u "$environment") == 0 ]]; }; then
    printf '%s\n' 'Compat library/config already installed; rollback backup retained.'
    exit 0
fi
transaction=$(mktemp -d "$state/compat-transaction.XXXXXXXXXX")
[[ ! -f $environment ]] || cp -p -- "$environment" "$transaction/native-environment"
[[ ! -f $latest ]] || cp -p -- "$latest" "$transaction/previous-latest"
touch "$transaction/ready"
pending=1
pointer=$(mktemp "$state/.compat-latest.XXXXXX")
printf '%s\n' "${transaction##*/}" > "$pointer"
mv -Tf -- "$pointer" "$latest"
mv -Tf -- "$temporary" "$environment"
temporary=
touch "$transaction/complete"
pending=0
printf 'Compat installed: %s\nNative env backup: %s\nNo service changes.\n' "$stable" "$transaction"
