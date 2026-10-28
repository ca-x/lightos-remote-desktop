#!/usr/bin/env bash
# Binary helper packaging/ABI/lifecycle, without compiling or changing services.
# shellcheck disable=SC2154
set -Eeuo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT
checks=0
check() { "$@" || { printf 'FAIL: %s\n' "$*" >&2; exit 1; }; checks=$((checks+1)); }
reject() { if "$@" > "$temporary/rejected.log" 2>&1; then printf 'FAIL: accepted %s\n' "$*" >&2; exit 1; fi; checks=$((checks+1)); }
# shellcheck disable=SC1091
source "$repository/lightos-codec-ffmpeg-bin/PKGBUILD"
srcdir=$temporary/source
pkgdir=$temporary/package
export srcdir pkgdir
mkdir -p "$srcdir/$_bundle/licenses" "$temporary/bin"
cp /bin/true "$srcdir/$_bundle/lightos-codec-ffmpeg"
for file in LICENSE README.md dependencies.txt ffmpeg-abi.txt licenses/dependencies.txt licenses/ffmpeg-runtime.txt; do
    printf 'Arch fixture\n' > "$srcdir/$_bundle/$file"
done
printf '{"platform":"archlinux-x86_64","source_commit":"0000000000000000000000000000000000000001","needed":["libavcodec.so.63","libavformat.so.63","libavutil.so.61","libswscale.so.10"],"ffmpeg_sonames":["libavcodec.so.63","libavformat.so.63","libavutil.so.61","libswscale.so.10"]}\n' > "$srcdir/$_bundle/abi.json"
cp "$repository/lightos-codec-ffmpeg-bin/validate-arch-codec.py" "$srcdir/"
# Real ELF header inspection, with explicit fake dynamic linkage for /bin/true.
cat > "$temporary/bin/readelf" <<'MOCK'
#!/usr/bin/env bash
if [[ $1 == -d ]]; then
    for name in libavcodec.so.63 libavformat.so.63 libavutil.so.61 libswscale.so.10; do
        [[ ${AUR_CODEC_WITHOUT_FORMAT:-} != 1 || $name != libavformat.so.63 ]] || continue
        printf ' 0x0000000000000001 (NEEDED) Shared library: [%s]\n' "$name"
    done
else
    exec /usr/bin/readelf "$@"
fi
MOCK
chmod +x "$temporary/bin/readelf"
export PATH=$temporary/bin:$PATH
tar -C "$srcdir" -czf "$srcdir/${_bundle}-${pkgver}.tar.gz" "$_bundle"
digest=$(sha256sum "$srcdir/${_bundle}-${pkgver}.tar.gz")
printf '%s  %s.tar.gz\n' "${digest%% *}" "$_bundle" > "$srcdir/${_bundle}-${pkgver}.tar.gz.sha256"
(cd "$srcdir" && prepare && package)
check cmp "$srcdir/$_bundle/lightos-codec-ffmpeg" "$pkgdir/usr/lib/lightos-remoted/lightos-codec-ffmpeg"
check test -x "$pkgdir/usr/lib/lightos-remoted/lightos-codec-ffmpeg"
check test "$(readlink "$pkgdir/usr/bin/lightos-codec-ffmpeg")" = /usr/lib/lightos-remoted/lightos-codec-ffmpeg
check cmp "$srcdir/$_bundle/licenses/dependencies.txt" "$pkgdir/usr/share/licenses/$pkgname/dependencies.txt"
check cmp "$srcdir/$_bundle/abi.json" "$pkgdir/usr/share/doc/$pkgname/abi.json"
check test ! -e "$pkgdir/etc"
check test ! -e "$pkgdir/usr/lib/systemd"
check test "${source[0]}" = lightos-codec-ffmpeg-bin.install
check test "${source[1]}" = validate-arch-codec.py
check test "${#source[@]}" = "${#sha256sums[@]}"
check test "${#source_x86_64[@]}" = "${#sha256sums_x86_64[@]}"
check test "${provides[0]}" = "lightos-codec-ffmpeg=$pkgver"
check test "${conflicts[0]}" = lightos-codec-ffmpeg
check test "${source_x86_64[0]}" = "${_bundle}-${pkgver}.tar.gz::https://github.com/ca-x/lightos-remote-desktop/releases/download/v${pkgver}/${_bundle}.tar.gz"
check test "$(declare -F build || true)" = ''
check python "$srcdir/validate-arch-codec.py" "$srcdir/$_bundle" 0000000000000000000000000000000000000001
reject python "$srcdir/validate-arch-codec.py" "$srcdir/$_bundle" 1111111111111111111111111111111111111111
cp "$srcdir/$_bundle/abi.json" "$temporary/good-abi.json"
sed -i s/libavcodec.so.63/libavcodec.so.60/g "$srcdir/$_bundle/abi.json"
reject python "$srcdir/validate-arch-codec.py" "$srcdir/$_bundle"
cp "$temporary/good-abi.json" "$srcdir/$_bundle/abi.json"
python - "$srcdir/$_bundle/abi.json" <<'PYTHON'
import json, sys
from pathlib import Path
p=Path(sys.argv[1]); data=json.loads(p.read_text())
for key in ('needed','ffmpeg_sonames'):
    data[key].remove('libavformat.so.63')
p.write_text(json.dumps(data))
PYTHON
export AUR_CODEC_WITHOUT_FORMAT=1
check python "$srcdir/validate-arch-codec.py" "$srcdir/$_bundle"
unset AUR_CODEC_WITHOUT_FORMAT
cp "$temporary/good-abi.json" "$srcdir/$_bundle/abi.json"
rm "$srcdir/$_bundle/licenses/dependencies.txt"
reject python "$srcdir/validate-arch-codec.py" "$srcdir/$_bundle"
printf 'invalid checksum\n' > "$srcdir/${_bundle}-${pkgver}.tar.gz.sha256"
run_prepare() { (cd "$srcdir" && prepare); }
reject run_prepare
# Hook paths are rewritten into each fixture, and systemctl is always a mock.
cat > "$temporary/bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$AUR_CODEC_ROOT/events"
case $1 in
is-active) [[ -f $AUR_CODEC_ROOT/active ]] ;;
show)
    if [[ $2 == --property=ExecMainPID ]]; then cat "$AUR_CODEC_ROOT/pid"; else cat "$AUR_CODEC_ROOT/fragment"; fi ;;
try-restart) [[ ! -f $AUR_CODEC_ROOT/fail-restart ]] ;;
*) echo 'Unexpected lifecycle operation' >&2; exit 99 ;;
esac
MOCK
chmod +x "$temporary/bin/systemctl"
fixture() {
    export AUR_CODEC_ROOT=$temporary/$1
    mkdir -p "$AUR_CODEC_ROOT/usr/lib/lightos-remoted" "$AUR_CODEC_ROOT/proc/123" "$AUR_CODEC_ROOT/etc/systemd/system"
    touch "$AUR_CODEC_ROOT/events" "$AUR_CODEC_ROOT/active"
    printf '#!/bin/sh\nexit 0\n' > "$AUR_CODEC_ROOT/usr/lib/lightos-remoted/lightos-remoted"
    chmod +x "$AUR_CODEC_ROOT/usr/lib/lightos-remoted/lightos-remoted"
    printf '123\n' > "$AUR_CODEC_ROOT/pid"
    printf '%s\n' "$AUR_CODEC_ROOT/usr/lib/systemd/system/lightos-remote-desktop.service" > "$AUR_CODEC_ROOT/fragment"
    ln -s "$AUR_CODEC_ROOT/usr/lib/lightos-remoted/lightos-remoted" "$AUR_CODEC_ROOT/proc/123/exe"
    sed "s|/usr/lib/|$AUR_CODEC_ROOT/usr/lib/|g; s|/etc/systemd/|$AUR_CODEC_ROOT/etc/systemd/|g; s|/proc/|$AUR_CODEC_ROOT/proc/|g" \
        "$repository/lightos-codec-ffmpeg-bin/lightos-codec-ffmpeg-bin.install" > "$AUR_CODEC_ROOT/install"
    # shellcheck disable=SC1091
    source "$AUR_CODEC_ROOT/install"
}
fixture managed
for hook in post_install post_upgrade post_remove; do "$hook"; done
check test "$(grep -c '^try-restart lightos-remote-desktop.service$' "$AUR_CODEC_ROOT/events")" = 3
check test "$(grep -Ec '^(start|restart|stop|enable|disable) ' "$AUR_CODEC_ROOT/events" || true)" = 0
check test "$(grep -c 'session.service' "$AUR_CODEC_ROOT/events" || true)" = 0
for scenario in no-main stopped legacy-transport legacy-owner foreign-binary recovery custom-unit invalid-pid; do
    fixture "$scenario"
    case $scenario in
        no-main) rm "$AUR_CODEC_ROOT/usr/lib/lightos-remoted/lightos-remoted" ;;
        stopped) rm "$AUR_CODEC_ROOT/active" ;;
        legacy-transport) touch "$AUR_CODEC_ROOT/etc/systemd/system/lightos-remote-desktop.service" ;;
        legacy-owner) ln -s /missing "$AUR_CODEC_ROOT/etc/systemd/system/lightos-remote-desktop-session.service" ;;
        foreign-binary) ln -sfn /legacy/lightos-remoted "$AUR_CODEC_ROOT/proc/123/exe" ;;
        recovery) ln -sfn /var/lib/lightos-remoted-package/previous/lightos-remoted "$AUR_CODEC_ROOT/proc/123/exe" ;;
        custom-unit) printf '/other/custom.service\n' > "$AUR_CODEC_ROOT/fragment" ;;
        invalid-pid) printf 'invalid\n' > "$AUR_CODEC_ROOT/pid" ;;
    esac
    for hook in post_install post_upgrade post_remove; do "$hook"; done
    check test "$(grep -c '^try-restart ' "$AUR_CODEC_ROOT/events" || true)" = 0
done
fixture deleted-executable
ln -sfn "$AUR_CODEC_ROOT/usr/lib/lightos-remoted/lightos-remoted (deleted)" "$AUR_CODEC_ROOT/proc/123/exe"
post_upgrade
check grep -Fxq 'try-restart lightos-remote-desktop.service' "$AUR_CODEC_ROOT/events"
fixture restart-fails
touch "$AUR_CODEC_ROOT/fail-restart"
check post_upgrade
fixture offline
export LIGHTOS_REMOTED_PACKAGE_STAGE_ONLY=1
for hook in post_install post_upgrade post_remove; do check "$hook"; done
check test ! -s "$AUR_CODEC_ROOT/events"
for staging_mode in invalid ""; do
    export LIGHTOS_REMOTED_PACKAGE_STAGE_ONLY=$staging_mode
    for hook in post_install post_upgrade post_remove; do reject "$hook"; done
    check test ! -s "$AUR_CODEC_ROOT/events"
done
unset LIGHTOS_REMOTED_PACKAGE_STAGE_ONLY
python3 - "$repository" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
workflow = (root/'.github/workflows/aur.yml').read_text()
helper = workflow.split('          pkgname: lightos-codec-ffmpeg-bin\n', 1)[1]
assets = helper.split('          assets: |\n', 1)[1].split('          post_process:', 1)[0]
assert assets.split() == ['./lightos-codec-ffmpeg-bin/lightos-codec-ffmpeg-bin.install', './lightos-codec-ffmpeg-bin/validate-arch-codec.py']
assert 'needs: [prepare, helper]' in workflow
assert "needs.helper.result == 'success' || needs.helper.result == 'skipped'" in workflow
assert "optdepends=('lightos-codec-ffmpeg-bin:" in (root/'lightos-remoted-bin/PKGBUILD').read_text()
PY
checks=$((checks+1))
printf 'AUR optional codec package: %s checks passed (binary ABI fixtures and managed transport refresh).\n' "$checks"
