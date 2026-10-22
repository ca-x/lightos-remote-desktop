#!/usr/bin/env bash
set -Eeuo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
build=$workspace/build
root=$workspace/root
mkdir -p "$build" "$root/etc/lightos" "$workspace/bin"
printf 'Public installer fixture license notice.\n' > "$workspace/LICENSE"
library=$build/libaquamarine.so.0.15.1
printf 'int compatibility_fixture(void) { return 1; }\n' | cc -x c -shared -fPIC -Wl,-soname,libaquamarine.so.14 -o "$library" -
printf 'Name: aquamarine\nDescription: installer fixture\nVersion: 0.15.1\n' > "$build/aquamarine.pc"
receipt() {
    printf 'upstream=https://github.com/hyprwm/aquamarine.git\nversion=0.15.1\ncommit=f31c47a1b9d300847d8fc3108ad959448103dfc1\nsoname=libaquamarine.so.14\n'
    sha256sum "$repository/patches/aquamarine-wayland.patch" "$library"
}
receipt > "$build/compatibility.txt"
environment=$root/etc/lightos/remote-desktop-native.env
secret=$root/etc/lightos/remote-desktop.env
state=$root/var/lib/lightos-remote-desktop
printf 'original-secret\n' > "$secret"
printf '# preserved\nOTHER="value with spaces"\nLITERAL=$(touch %s/pwned)\nLIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=/old\n LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR = /duplicate\n' "$workspace" > "$environment"
cp "$environment" "$workspace/original"
for command in systemctl systemd-run; do
    printf '#!/bin/sh\ntouch "%s/host-called"\nexit 99\n' "$workspace" > "$workspace/bin/$command"
    chmod +x "$workspace/bin/$command"
done
export PATH="$workspace/bin:$PATH"
run() { bash "$repository/scripts/install-compat.sh" --build-dir "$build" --test-root "$root" "$@"; }
reject() { if "$@" > "$workspace/rejected" 2>&1; then printf 'unexpected success: %s\n' "$*" >&2; exit 1; fi; }
count=0
pass() { count=$((count + 1)); printf 'ok %s - %s\n' "$count" "$*"; }
run --check
[[ ! -e $state && ! -e $root/usr ]]
cmp "$environment" "$workspace/original"
pass 'check validates real ELF/pkg-config/receipt without writes'
run
digest=$(sha256sum "$library"); digest=${digest%% *}
stable=/usr/local/lib/lightos-remote-desktop/compat/$digest
cmp "$root$stable/libaquamarine.so.14" "$library"
cmp "$root$stable/LICENSE" "$workspace/LICENSE"
license_digest=$(sha256sum "$workspace/LICENSE"); license_digest=${license_digest%% *}
grep -Fxq "license_sha256=$license_digest" "$root$stable/receipt.txt"
[[ $(stat -c %a "$environment") == 600 ]]
[[ $(grep -c 'LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR' "$environment") == 1 ]]
grep -Fxq "LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR=$stable" "$environment"
grep -Fxq 'OTHER="value with spaces"' "$environment"
grep -Fq 'LITERAL=$(touch ' "$environment"
snapshot=$(cat "$state/compat-latest")
cmp "$state/$snapshot/native-environment" "$workspace/original"
[[ $(cat "$secret") == original-secret && ! -e $workspace/pwned && ! -e $workspace/host-called ]]
pass 'stable library, 0600 env, deduplicated key and original backup; no evaluation/service calls'
run
[[ $(cat "$state/compat-latest") == "$snapshot" ]]
pass 'identical provisioning retains rollback backup'
mv "$root$stable/LICENSE" "$workspace/installed-license"
reject run --check
mv "$workspace/installed-license" "$root$stable/LICENSE"
cp "$root$stable/receipt.txt" "$workspace/installed-receipt"
printf 'tampered\n' >> "$root$stable/receipt.txt"
reject run --check
cp "$workspace/installed-receipt" "$root$stable/receipt.txt"
pass 'existing release cannot omit LICENSE or alter its checksum receipt'
run --rollback --check
run --rollback
cmp "$environment" "$workspace/original"
reject run --rollback
pass 'rollback restores previous native env exactly once'
printf '\ncorruption\n' >> "$library"
reject run --check
pass 'receipt checksum mismatch rejected'
receipt > "$build/compatibility.txt"
printf 'Name: aquamarine\nDescription: fixture\nVersion: 9.0\n' > "$build/aquamarine.pc"
reject run --check
printf 'Name: aquamarine\nDescription: fixture\nVersion: 0.15.1\n' > "$build/aquamarine.pc"
pass 'wrong pkg-config version rejected'
printf 'int compatibility_fixture(void) { return 1; }\n' | cc -x c -shared -fPIC -Wl,-soname,libaquamarine.so.99 -o "$library" -
receipt > "$build/compatibility.txt"
reject run --check
pass 'wrong SONAME rejected despite a matching receipt checksum'
printf 'int compatibility_fixture(void) { return 1; }\n' | cc -x c -shared -fPIC -Wl,-soname,libaquamarine.so.14 -o "$library" -
receipt > "$build/compatibility.txt"
mv "$workspace/LICENSE" "$workspace/saved-license"
reject run --check
reject run
run --check --license "$workspace/saved-license"
mv "$workspace/saved-license" "$workspace/LICENSE"
pass 'missing license refuses installation; explicit license path is supported'
printf 'OTHER="multiline\nvalue"\n' > "$environment"
cp "$environment" "$workspace/multiline"
reject run --check
reject run
cmp "$environment" "$workspace/multiline"
pass 'multiline env fails without changing native config'
rm "$environment"
run
run --rollback
[[ ! -e $environment ]]
pass 'absent native env is created safely and rollback removes it'
ln -s "$secret" "$environment"
reject run --check
rm "$environment"
reject bash "$repository/scripts/install-compat.sh" --test-root / --check
reject bash "$repository/scripts/install-compat.sh" --test-root '' --check
pass 'unsafe native symlink and test roots rejected'
run
cat > "$workspace/bin/mv" <<'EOF'
#!/usr/bin/env bash
if [[ ${*: -1} == */remote-desktop-native.env && ${*: -2:1} == */.native-env.* ]]; then exit 42; fi
exec /usr/bin/mv "$@"
EOF
chmod +x "$workspace/bin/mv"
printf 'OTHER=operator-edit\n' >> "$environment"
cp "$environment" "$workspace/before-failure"
latest=$(cat "$state/compat-latest")
reject run
cmp "$environment" "$workspace/before-failure"
[[ $(cat "$state/compat-latest") == "$latest" ]]
pass 'failed native env rename restores config and previous transaction pointer'
[[ $(cat "$secret") == original-secret && ! -e $workspace/pwned && ! -e $workspace/host-called ]]
printf 'PASS: %s compatibility installer behaviors; no host or library execution.\n' "$count"
