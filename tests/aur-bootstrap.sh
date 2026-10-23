#!/usr/bin/env bash
# Isolated account/systemd fixtures. No host services/users/files are changed.
set -Eeuo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT
checks=0
check() { "$@" || { printf 'FAIL: %s\n' "$*" >&2; exit 1; }; checks=$((checks + 1)); }
fixture() {
    root=$temporary/$1
    mkdir -p "$root/usr/lib/lightos-remoted/aquamarine" "$root/usr/lib/systemd/system" "$root/test-bin"
    touch "$root/events"
    cp "$repository/lightos-remoted-bin/"*.service "$root/usr/lib/systemd/system/"
    cp "$repository/lightos-remoted-bin/activation.sh" "$root/usr/lib/lightos-remoted/"
    printf '#!/bin/sh\nexit 0\n' > "$root/usr/lib/lightos-remoted/lightos-remoted"
    chmod +x "$root/usr/lib/lightos-remoted/lightos-remoted"
    cat > "$root/test-bin/getent" <<'MOCK'
#!/usr/bin/env bash
root=${0%/test-bin/getent}
if [[ -f $root/nss-$2 ]]; then cat "$root/nss-$2"; exit; fi
[[ -f $root/account ]] || exit 2
awk -F: -v key="$2" '$1==key || $3==key { print; found=1 } END { exit !found }' "$root/account"
MOCK
    cat > "$root/test-bin/useradd" <<'MOCK'
#!/usr/bin/env bash
root=${0%/test-bin/useradd}
printf '%s\n' 'lightos-remoted:x:999:999::/var/lib/lightos-remoted:/usr/bin/nologin' > "$root/account"
printf '%s\n' 'useradd' >> "$root/events"
MOCK
    cat > "$root/test-bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
root=${0%/test-bin/systemctl}
printf 'systemctl %s\n' "$*" >> "$root/events"
if [[ ${*: -1} == lightos-remote-desktop-session.service ]]; then
    [[ $1 == show && $* == *--property=User* ]] || exit 99
fi
case $1 in
is-active)
    [[ -f $root/legacy-active || -f $root/active ]] ;;
show)
    if [[ $* == *--property=User* ]]; then
        kind=transport
        [[ ${*: -1} != lightos-remote-desktop-session.service ]] || kind=owner
        if [[ -f $root/$kind-user ]]; then cat "$root/$kind-user"; else
            awk -F= '$1=="User" {print $2}' "$root/etc/systemd/system/${*: -1}.d/50-runtime-user.conf"
        fi
    elif [[ $* == *--value* ]]; then cat "$root/pid" 2>/dev/null || printf '0\n'
    else printf 'ActiveState=active\nSubState=running\nMainPID=%s\nExecMainPID=%s\nNRestarts=0\n' "$(cat "$root/pid")" "$(cat "$root/pid")"; fi ;;
restart)
    pid=$(cat "$root/pid" 2>/dev/null || echo 100)
    echo "$((pid+1))" > "$root/pid"
    touch "$root/active"
    mkdir -p "$root/proc/$((pid+1))"
    ln -s /usr/lib/lightos-remoted/lightos-remoted "$root/proc/$((pid+1))/exe" ;;
stop) rm -f "$root/active" ;;
*) : ;;
esac
MOCK
    cat > "$root/test-bin/systemd-run" <<'MOCK'
#!/usr/bin/env bash
root=${0%/test-bin/systemd-run}
printf 'systemd-run %s\n' "$*" >> "$root/events"
if [[ -f $root/fail-doctor && $* == *doctor ]]; then exit 31; fi
if [[ -f $root/fail-probe && $* == *probe* ]]; then exit 32; fi
MOCK
    chmod +x "$root/test-bin/"*
}
bootstrap() { bash "$repository/lightos-remoted-bin/bootstrap.sh" --test-root "$root" "$@" > "$root/output" 2>&1; }
fixture fresh
bootstrap
check test -s "$root/etc/lightos/remote-desktop.env"
check grep -q '^LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD=[0-9a-f]\{64\}$' "$root/etc/lightos/remote-desktop.env"
check test "$(stat -c %a "$root/etc/lightos/remote-desktop.env")" = 600
check grep -q "^LIGHTOS_REMOTE_DESKTOP_ENABLE=1$" "$root/etc/lightos/remote-desktop.env"
check grep -q "^LIGHTOS_REMOTE_DESKTOP_USER=lightos-remoted$" "$root/etc/lightos/remote-desktop.env"
check grep -q '^systemctl enable lightos-remote-desktop.service$' "$root/events"
check grep -q '^systemctl restart lightos-remote-desktop.service$' "$root/events"
check test "$(grep -c 'systemd-run .* probe ' "$root/events")" = 2
password=$(cat "$root/etc/lightos/remote-desktop.env")
check test "$(grep -Fc "$password" "$root/output" || true)" = 0
bootstrap
check test "$(cat "$root/etc/lightos/remote-desktop.env")" = "$password"
check test "$(grep -c useradd "$root/events")" = 1
fixture fresh2
bootstrap
check test "$(cat "$root/etc/lightos/remote-desktop.env")" != "$password"
# New-layout upgrades snapshot old binary/helper and restart transport only.
fixture upgrade
bootstrap
printf 'validated helper' > "$root/usr/lib/lightos-remoted/lightos-codec-ffmpeg"
chmod +x "$root/usr/lib/lightos-remoted/lightos-codec-ffmpeg"
bootstrap --snapshot
check cmp "$root/usr/lib/lightos-remoted/lightos-remoted" "$root/var/lib/lightos-remoted-package/previous/lightos-remoted"
printf '\n# upgraded\n' >> "$root/usr/lib/lightos-remoted/lightos-remoted"
bootstrap
check test "$(grep -c 'systemctl restart lightos-remote-desktop.service' "$root/events")" = 2
check test "$(cat "$root/usr/lib/lightos-remoted/lightos-codec-ffmpeg")" = 'validated helper'
# Failure propagates and selects the previous payload without changing owner.
touch "$root/fail-probe"
if bootstrap; then echo 'Probe failure swallowed' >&2; exit 1; fi
check grep -q 'ExecStart=/var/lib/lightos-remoted-package/previous/lightos-remoted serve' "$root/run/systemd/system/lightos-remote-desktop.service.d/90-aur-recovery.conf"
check test -f "$root/active"
check grep -q 'previous transport recovered' "$root/output"
# Repeated snapshots during recovery retain the known-good binary.
cp "$root/var/lib/lightos-remoted-package/previous/lightos-remoted" "$root/known-good"
bootstrap --snapshot
check cmp "$root/known-good" "$root/var/lib/lightos-remoted-package/previous/lightos-remoted"
rm "$root/fail-probe"
bootstrap
check test ! -e "$root/run/systemd/system/lightos-remote-desktop.service.d/90-aur-recovery.conf"
# Doctor failure cannot restart a running transport or enable a fresh instance.
fixture doctor
 touch "$root/fail-doctor"
if bootstrap; then echo 'Doctor failure swallowed' >&2; exit 1; fi
check test "$(grep -c 'systemctl restart\|systemctl enable' "$root/events" || true)" = 0
# A doctor-rejected B must never displace successfully activated A when C arrives.
fixture doctor-upgrades
bootstrap
cp "$root/usr/lib/lightos-remoted/lightos-remoted" "$root/payload-a"
a_digest=$(sha256sum "$root/payload-a"); a_digest=${a_digest%% *}
bootstrap --snapshot
printf '\n# candidate B\n' >> "$root/usr/lib/lightos-remoted/lightos-remoted"
touch "$root/fail-doctor"
if bootstrap; then echo 'Candidate B doctor failure swallowed' >&2; exit 1; fi
check test "$(cat "$root/var/lib/lightos-remoted-package/activated.sha256")" = "$a_digest"
b_digest=$(sha256sum "$root/usr/lib/lightos-remoted/lightos-remoted"); b_digest=${b_digest%% *}
check test "$(cat "$root/var/lib/lightos-remoted-package/rejected.sha256")" = "$b_digest"
bootstrap --snapshot
check cmp "$root/payload-a" "$root/var/lib/lightos-remoted-package/previous/lightos-remoted"
printf '\n# candidate C\n' >> "$root/usr/lib/lightos-remoted/lightos-remoted"
rm "$root/fail-doctor"
touch "$root/fail-probe"
if bootstrap; then echo 'Candidate C probe failure swallowed' >&2; exit 1; fi
check cmp "$root/payload-a" "$root/var/lib/lightos-remoted-package/previous/lightos-remoted"
check grep -q 'previous/lightos-remoted serve' "$root/run/systemd/system/lightos-remote-desktop.service.d/90-aur-recovery.conf"
check test -f "$root/active"
# Both effective unit users must resolve to the configured nonroot UID.
for condition in transport-alias-zero owner-root owner-wrong-uid; do
    fixture "effective-$condition"
    case $condition in
        transport-alias-zero)
            printf 'superuser\n' > "$root/transport-user"
            printf 'superuser:x:0:0::/root:/bin/bash\n' > "$root/nss-superuser" ;;
        owner-root)
            printf 'root\n' > "$root/owner-user"
            printf 'root:x:0:0::/root:/bin/bash\n' > "$root/nss-root" ;;
        owner-wrong-uid)
            printf 'other\n' > "$root/owner-user"
            printf 'other:x:1000:1000::/home/other:/bin/bash\n' > "$root/nss-other" ;;
    esac
    if bootstrap; then echo "Invalid effective user accepted: $condition" >&2; exit 1; fi
    check test "$(grep -c 'systemd-run\|systemctl restart\|systemctl enable' "$root/events" || true)" = 0
    check test -s "$root/var/lib/lightos-remoted-package/rejected.sha256"
done
# Existing secrets/config (including custom library/shell settings) are byte exact.
fixture config
mkdir -p "$root/etc/lightos"
printf 'LIGHTOS_REMOTE_DESKTOP_USER=czyt\nLIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD="existing secret"\nLIGHTOS_REMOTE_DESKTOP_SHELL=auto\n' > "$root/etc/lightos/remote-desktop.env"
printf "czyt:x:1000:1000::/home/czyt:/bin/bash\n" > "$root/account"
cp "$root/etc/lightos/remote-desktop.env" "$root/config.before"
bootstrap
check cmp "$root/config.before" "$root/etc/lightos/remote-desktop.env"
check grep -q "^User=czyt$" "$root/etc/systemd/system/lightos-remote-desktop.service.d/50-runtime-user.conf"
check grep -q "^User=czyt$" "$root/etc/systemd/system/lightos-remote-desktop-session.service.d/50-runtime-user.conf"
check grep -q "^Environment=HOME=/home/czyt$" "$root/etc/systemd/system/lightos-remote-desktop-session.service.d/50-runtime-user.conf"
check test "$(grep -c useradd "$root/events" || true)" = 0
printf 'LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=/existing/private/lib\n' > "$root/etc/lightos/remote-desktop-native.env"
cp "$root/etc/lightos/remote-desktop-native.env" "$root/native.before"
bootstrap
check cmp "$root/native.before" "$root/etc/lightos/remote-desktop-native.env"
# Invalid or missing configured accounts never turn into a root desktop.
for user in root absent; do
    fixture "invalid-$user"
    mkdir -p "$root/etc/lightos"
    printf 'LIGHTOS_REMOTE_DESKTOP_USER=%s\nLIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD=preserve\n' "$user" > "$root/etc/lightos/remote-desktop.env"
    if bootstrap; then echo "Invalid account accepted: $user" >&2; exit 1; fi
    check test "$(grep -c 'systemctl restart\|systemctl enable' "$root/events" || true)" = 0
done
# Legacy service presence OR active state skips all initialization and activation.
for condition in unit owner active; do
    fixture "legacy-$condition"
    if [[ $condition == unit || $condition == owner ]]; then
        mkdir -p "$root/etc/systemd/system"
        name=lightos-remote-desktop
        [[ $condition != owner ]] || name=lightos-remote-desktop-session
        printf '[Service]\nUser=czyt\n' > "$root/etc/systemd/system/$name.service"
    else touch "$root/legacy-active"; echo 91 > "$root/pid"; fi
    bootstrap
    check test ! -e "$root/account"
    check test ! -e "$root/etc/lightos/remote-desktop.env"
    check test ! -e "$root/var/lib/lightos-remoted-package"
    check grep -q 'left unchanged' "$root/output"
    check test "$(grep -c 'systemctl restart\|systemctl enable' "$root/events" 2>/dev/null || true)" = 0
    bootstrap --remove
    check test "$(grep -c 'systemctl disable' "$root/events" || true)" = 0
 done
fixture empty
mkdir -p "$root/etc/lightos"
touch "$root/etc/lightos/remote-desktop.env"
if bootstrap; then echo 'Empty existing environment overwritten' >&2; exit 1; fi
check test ! -s "$root/etc/lightos/remote-desktop.env"
fixture symlink
mkdir -p "$root/etc/lightos"
printf preserve > "$root/target"
ln -s "$root/target" "$root/etc/lightos/remote-desktop.env"
if bootstrap; then echo 'Symlink accepted' >&2; exit 1; fi
check test "$(cat "$root/target")" = preserve
# Removal disables only the transport and retains account/config/session/payload.
fixture removal
bootstrap
bootstrap --snapshot
PATH="$root/test-bin:$PATH"
source "$repository/lightos-remoted-bin/lightos-remoted-bin.install"
# Exercise removal with snapshot dispatch redirected into the isolated fixture.
pre_remove_body=$(declare -f pre_remove)
pre_remove_body=${pre_remove_body//\/usr\/lib\/lightos-remoted\/bootstrap/bash "$repository/lightos-remoted-bin/bootstrap.sh" --test-root "$root"}
eval "$pre_remove_body"
pre_remove
check test -f "$root/active"
check test -s "$root/etc/lightos/remote-desktop.env"
check grep -q '^systemctl disable lightos-remote-desktop.service$' "$root/events"
check test -x "$root/var/lib/lightos-remoted-package/previous/lightos-remoted"
printf 'AUR bootstrap: %s checks passed (mock activation; no host service changes).\n' "$checks"
