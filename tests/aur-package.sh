#!/usr/bin/env bash
# Exercise package() and pacman lifecycle hooks without host installation.
# Variables below also come from the sourced makepkg recipe.
# shellcheck disable=SC2154
set -Eeuo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT
export AUR_PACKAGE_FIXTURE=$temporary
checks=0
check() { "$@" || { printf 'FAIL: %s\n' "$*" >&2; exit 1; }; checks=$((checks+1)); }
reject() { if "$@" > "$temporary/rejected.log" 2>&1; then printf 'FAIL: accepted %s\n' "$*" >&2; exit 1; fi; checks=$((checks+1)); }
# Only package() runs: no downloads, dependency builds, or makepkg installation.
# shellcheck disable=SC1091
source "$repository/lightos-remoted-bin/PKGBUILD"
export srcdir=$temporary/source pkgdir=$temporary/package
mkdir -p "$srcdir/$_bundle/"{licenses,docs} "$srcdir/aq-build" "$srcdir/aquamarine-$_aqcommit"
printf 'core license\n' > "$srcdir/$_bundle/LICENSE"
printf 'dependency notice\n' > "$srcdir/$_bundle/licenses/dependency.txt"
printf 'documentation\n' > "$srcdir/$_bundle/docs/architecture.md"
printf 'Chinese README fixture\n' > "$srcdir/$_bundle/README.md"
printf 'English README fixture\n' > "$srcdir/$_bundle/README.en.md"
printf 'Aquamarine license\n' > "$srcdir/aquamarine-$_aqcommit/LICENSE"
printf 'private library fixture\n' > "$srcdir/aq-build/libaquamarine.so.0.15.1"
cat > "$srcdir/$_bundle/lightos-remoted" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$AUR_PACKAGE_FIXTURE/export.log"
[[ $# == 3 && $1 == install-service && $2 == --print-unit ]]
[[ $3 == transport || $3 == session ]]
[[ ${FAIL_EXPORT:-} != "$3" ]] || exit 23
[[ ${EMPTY_EXPORT:-} != "$3" ]] || exit 0
cat "$AUR_PACKAGE_FIXTURE/$3.service"
MOCK
chmod +x "$srcdir/$_bundle/lightos-remoted"
for unit in transport session; do
    printf '[Unit]\nDescription=exported %s fixture\n[Service]\nExecStart=/fixture/%s\n' "$unit" "$unit" > "$temporary/$unit.service"
done
# Optional integration with the built Task 1 executable. Only rootless unit export runs.
if [[ -n ${LRD_AUR_EXPORT_BINARY:-} ]]; then
    cp -- "$LRD_AUR_EXPORT_BINARY" "$srcdir/$_bundle/lightos-remoted"
    for unit in transport session; do
        "$LRD_AUR_EXPORT_BINARY" install-service --print-unit "$unit" > "$temporary/$unit.service"
    done
fi
run_package() { (cd "$srcdir" && package); }
run_package
check cmp "$temporary/transport.service" "$pkgdir/usr/lib/systemd/system/lightos-remote-desktop.service"
check cmp "$temporary/session.service" "$pkgdir/usr/lib/systemd/system/lightos-remote-desktop-session.service"
check cmp "$srcdir/$_bundle/lightos-remoted" "$pkgdir/usr/lib/lightos-remoted/lightos-remoted"
check test -x "$pkgdir/usr/lib/lightos-remoted/lightos-remoted"
check test "$(readlink "$pkgdir/usr/bin/lightos-remoted")" = /usr/lib/lightos-remoted/lightos-remoted
check test "$(stat -c %a "$pkgdir/usr/lib/systemd/system/lightos-remote-desktop.service")" = 644
check test "$(stat -c %a "$pkgdir/usr/lib/systemd/system/lightos-remote-desktop-session.service")" = 644
check cmp "$srcdir/aq-build/libaquamarine.so.0.15.1" "$pkgdir/usr/lib/lightos-remoted/aquamarine/libaquamarine.so.14"
check test "$(find "$pkgdir/usr/lib/lightos-remoted" -type f | wc -l)" = 2
check test ! -e "$pkgdir/usr/lib/lightos-remoted/bootstrap.sh"
check test ! -e "$pkgdir/usr/lib/lightos-remoted/activation.sh"
check test ! -e "$pkgdir/etc"
check cmp "$srcdir/$_bundle/README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
check cmp "$srcdir/$_bundle/README.en.md" "$pkgdir/usr/share/doc/$pkgname/README.en.md"
if [[ -z ${LRD_AUR_EXPORT_BINARY:-} ]]; then
    printf 'install-service --print-unit transport\ninstall-service --print-unit session\n' > "$temporary/expected-exports"
    check cmp "$temporary/expected-exports" "$temporary/export.log"
    for unit in transport session; do
        export FAIL_EXPORT=$unit
        reject run_package
        unset FAIL_EXPORT
        export EMPTY_EXPORT=$unit
        reject run_package
        unset EMPTY_EXPORT
    done
fi
# Rewrite the hook's fixed production executable path to an isolated recorder.
cat > "$temporary/lightos-remoted" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$AUR_PACKAGE_FIXTURE/hooks.log"
exit "${HOOK_STATUS:-0}"
MOCK
chmod +x "$temporary/lightos-remoted"
sed "s|/usr/lib/lightos-remoted/lightos-remoted|$temporary/lightos-remoted|g" \
    "$repository/lightos-remoted-bin/lightos-remoted-bin.install" > "$temporary/install"
# shellcheck disable=SC1091
source "$temporary/install"
for hook in pre_upgrade post_install post_upgrade pre_remove; do
    "$hook" 0.1.12-1 0.1.11-1
    export HOOK_STATUS=23
    reject "$hook" 0.1.12-1 0.1.11-1
    unset HOOK_STATUS
done
cat > "$temporary/expected-hooks" <<'EXPECTED'
install-service --snapshot
install-service --snapshot
install-service --package-managed
install-service --package-managed
install-service --package-managed
install-service --package-managed
install-service --remove
install-service --remove
EXPECTED
check cmp "$temporary/expected-hooks" "$temporary/hooks.log"
# The source assets and workflow must both use only the hook and ABI patch.
for asset in "${source[@]:1}"; do
    check test -f "$repository/lightos-remoted-bin/$asset"
done
check test "${source[1]}" = aquamarine-wayland.patch
check test "${source[2]}" = lightos-remoted-bin.install
check test "${#source[@]}" = 3
check test "${#sha256sums[@]}" = 3
python3 - "$repository" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
workflow = (root / '.github/workflows/aur.yml').read_text()
assets = workflow.split('          assets: |\n', 1)[1].split('          post_process:', 1)[0]
assert assets.split() == ['./lightos-remoted-bin/lightos-remoted-bin.install', './lightos-remoted-bin/aquamarine-wayland.patch']
PY
checks=$((checks+1))
printf 'AUR package and lifecycle: %s checks passed (isolated package tree and executable hooks).\n' "$checks"
