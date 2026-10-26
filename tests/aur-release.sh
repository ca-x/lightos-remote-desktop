#!/usr/bin/env bash
# Exercise release publication guards with isolated API/download fixtures.
set -Eeuo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT
mkdir -p "$temporary/bin" "$temporary/lightos-remoted-bin" "$temporary/lightos-codec-ffmpeg-bin"
cp "$repository/lightos-remoted-bin/PKGBUILD" "$temporary/lightos-remoted-bin/"
cp "$repository/lightos-codec-ffmpeg-bin/PKGBUILD" "$temporary/lightos-codec-ffmpeg-bin/"
cp "$repository/lightos-codec-ffmpeg-bin/validate-arch-codec.py" "$temporary/lightos-codec-ffmpeg-bin/"
export AUR_RELEASE_FIXTURE=$temporary
export GH_REPO=ca-x/lightos-remote-desktop RELEASE_TAG=v0.1.11 GITHUB_OUTPUT=$temporary/output FORCE_RELEASE=false
cat > "$temporary/bin/gh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
case $1 in
api)
    if [[ $2 == --method ]]; then shift 2; fi
    case $2 in
    */releases/tags/*) cat "$AUR_RELEASE_FIXTURE/release.json" ;;
    */commits/*) printf '%040d\n' 1 ;;
    */actions/workflows/*) cat "$AUR_RELEASE_FIXTURE/runs.json" ;;
    *) exit 2 ;;
    esac ;;
release)
    destination=
    while (($#)); do
        if [[ $1 == --dir ]]; then destination=$2; break; fi
        shift
    done
    cp "$AUR_RELEASE_FIXTURE/"*.tar.gz* "$destination/" ;;
*) exit 3 ;;
esac
MOCK
cat > "$temporary/bin/readelf" <<'MOCK'
#!/usr/bin/env bash
if [[ $1 == -d && $2 == *lightos-codec-ffmpeg-linux-x86_64-archlinux/lightos-codec-ffmpeg ]]; then
    for name in libavcodec.so.63 libavformat.so.63 libavutil.so.61 libswscale.so.10; do
        printf ' 0x0000000000000001 (NEEDED) Shared library: [%s]\n' "$name"
    done
else
    exec /usr/bin/readelf "$@"
fi
MOCK
cat > "$temporary/bin/curl" <<'MOCK'
#!/usr/bin/env bash
cat "$AUR_RELEASE_FIXTURE/aur.json"
MOCK
printf '#!/bin/sh\nexit 0\n' > "$temporary/bin/sleep"
chmod +x "$temporary/bin/"*
export PATH=$temporary/bin:$PATH
# Reuse /bin/true only to provide a real x86_64 ELF for the nonexecuting inspection.
mkdir -p "$temporary/archive/lightos-remoted-linux-x86_64"
cp /bin/true "$temporary/archive/lightos-remoted-linux-x86_64/lightos-remoted"
printf 'fixture license\n' > "$temporary/archive/lightos-remoted-linux-x86_64/LICENSE"
tar -C "$temporary/archive" -czf "$temporary/lightos-remoted-linux-x86_64.tar.gz" lightos-remoted-linux-x86_64
(cd "$temporary" && sha256sum lightos-remoted-linux-x86_64.tar.gz > lightos-remoted-linux-x86_64.tar.gz.sha256)
reset() {
    arch=lightos-codec-ffmpeg-linux-x86_64-archlinux
    mkdir -p "$temporary/archive/$arch/licenses"
    cp /bin/true "$temporary/archive/$arch/lightos-codec-ffmpeg"
    for file in LICENSE README.md dependencies.txt ffmpeg-abi.txt licenses/dependencies.txt licenses/ffmpeg-runtime.txt; do
        printf 'Arch fixture\n' > "$temporary/archive/$arch/$file"
    done
    printf '{"platform":"archlinux-x86_64","source_commit":"0000000000000000000000000000000000000001","needed":["libavcodec.so.63","libavformat.so.63","libavutil.so.61","libswscale.so.10"],"ffmpeg_sonames":["libavcodec.so.63","libavformat.so.63","libavutil.so.61","libswscale.so.10"]}\n' > "$temporary/archive/$arch/abi.json"
    tar -C "$temporary/archive" -czf "$temporary/$arch.tar.gz" "$arch"
    (cd "$temporary" && sha256sum "$arch.tar.gz" > "$arch.tar.gz.sha256" && sha256sum lightos-remoted-linux-x86_64.tar.gz > lightos-remoted-linux-x86_64.tar.gz.sha256)
    cp "$repository/lightos-remoted-bin/PKGBUILD" "$temporary/lightos-remoted-bin/"
    cp "$repository/lightos-codec-ffmpeg-bin/PKGBUILD" "$temporary/lightos-codec-ffmpeg-bin/"
    # API fixtures use a fixed historical release, independent of the tracked version.
    sed -i 's/^pkgver=.*/pkgver=0.1.11/' "$temporary/lightos-remoted-bin/PKGBUILD" "$temporary/lightos-codec-ffmpeg-bin/PKGBUILD"
    python3 - <<'PY'
import json, os
from pathlib import Path
p = Path(os.environ['AUR_RELEASE_FIXTURE'])
base = 'lightos-remoted-linux-x86_64.tar.gz'
worker = 'lightos-codec-ffmpeg-linux-x86_64-ubuntu-24.04.tar.gz'
arch = 'lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz'
(p/'release.json').write_text(json.dumps(dict(tag_name='v0.1.11', draft=False, prerelease=False, published_at='2026-10-23', assets=[dict(name=n, state='uploaded', size=123) for n in (base,base+'.sha256',worker,worker+'.sha256',arch,arch+'.sha256')])))
(p/'runs.json').write_text(json.dumps(dict(workflow_runs=[dict(head_branch='v0.1.11',head_sha=f'{1:040}',status='completed',conclusion='success',run_number=100)])))
(p/'aur.json').write_text(json.dumps(dict(type='multiinfo',resultcount=0,results=[])))
(p/'output').write_text('')
PY
    RELEASE_TAG=v0.1.11
    GH_REPO=ca-x/lightos-remote-desktop
    FORCE_RELEASE=false
}
mutate() {
    python3 - "$1" "$2" <<'PY'
import json, os, sys
from pathlib import Path
p=Path(os.environ['AUR_RELEASE_FIXTURE'])/sys.argv[1]
r=json.loads(p.read_text())
exec(sys.argv[2])
p.write_text(json.dumps(r))
PY
}
run() { (cd "$temporary" && bash "$repository/.github/scripts/prepare-aur-release.sh") > "$temporary/log" 2>&1; }
reject() { if run; then echo "FAIL: $1 accepted" >&2; exit 1; fi; checks=$((checks+1)); }
checks=0
reset
run
grep -q '^publish=true$' "$GITHUB_OUTPUT"
checks=$((checks+1))
for property in draft prerelease; do
    reset
    mutate release.json "r['$property']=True"
    reject "$property"
done
reset
mutate release.json "r['assets'].append(dict(name='unexpected',state='uploaded',size=123))"
reject 'unexpected asset'
reset
mutate release.json "r['assets'].pop()"
reject 'missing asset'
native_release() {
    reset
    RELEASE_TAG=v0.1.19
    mutate release.json "r['tag_name']='v0.1.19'; r['assets'].extend(dict(name=n, state='uploaded', size=123) for archive in ('lightos-remoted-windows-x86_64.zip','lightos-remoted-macos-aarch64.tar.gz','lightos-remoted-macos-x86_64.tar.gz') for n in (archive, archive+'.sha256'))"
    mutate runs.json "r['workflow_runs'][0]['head_branch']='v0.1.19'"
}
native_release
run
grep -q '^publish=true$' "$GITHUB_OUTPUT"
checks=$((checks+1))
native_release
mutate release.json "r['assets'].pop()"
reject 'missing native checksum'
native_release
mutate release.json "r['assets'] = r['assets'][:6]"
reject 'missing native platforms'
native_release
mutate release.json "r['assets'].append(dict(name='unexpected',state='uploaded',size=123))"
reject 'unexpected native release asset'
reset
RELEASE_TAG='v0.1.11;echo INJECTION'
reject 'malformed tag'
reset
GH_REPO=someone/else
reject 'unexpected repository'
reset
mutate runs.json "r['workflow_runs'][0]['conclusion']='failure'"
reject 'failed tag CI'
reset
mutate runs.json "r['workflow_runs'][0]['head_sha']='a'*40"
reject 'different commit CI'
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Name='lightos-remoted-bin',Version='0.2.0-1')])"
reject 'AUR downgrade'
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Name='lightos-remoted-bin',Version='0.1.11-2')])"
reject 'pkgrel downgrade'
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Name='lightos-remoted-bin',Version='0.1.11-1')])"
run
grep -q '^publish=false$' "$GITHUB_OUTPUT"
checks=$((checks+1))
FORCE_RELEASE=true
: > "$GITHUB_OUTPUT"
run
grep -q '^publish=true$' "$GITHUB_OUTPUT"
checks=$((checks+1))
reset
printf 'invalid checksum\n' > "$temporary/lightos-remoted-linux-x86_64.tar.gz.sha256"
reject 'invalid checksum'
(cd "$temporary" && sha256sum lightos-remoted-linux-x86_64.tar.gz > lightos-remoted-linux-x86_64.tar.gz.sha256)
reset
printf 'invalid checksum\n' > "$temporary/lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz.sha256"
reject 'invalid Arch checksum'
reset
python3 - "$temporary/archive/$arch/abi.json" <<'PYTHON'
import json, sys
from pathlib import Path
p=Path(sys.argv[1]); r=json.loads(p.read_text()); r['ffmpeg_sonames'][0]='libavcodec.so.60'; p.write_text(json.dumps(r))
PYTHON
tar -C "$temporary/archive" -czf "$temporary/$arch.tar.gz" "$arch"
(cd "$temporary" && sha256sum "$arch.tar.gz" > "$arch.tar.gz.sha256")
reject 'Ubuntu/incompatible worker ABI'
reset
rm "$temporary/archive/$arch/licenses/dependencies.txt"
tar -C "$temporary/archive" -czf "$temporary/$arch.tar.gz" "$arch"
(cd "$temporary" && sha256sum "$arch.tar.gz" > "$arch.tar.gz.sha256")
reject 'missing worker notices'
# Each AUR package decides independently; retries preserve exact tracked pkgrel.
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Name='lightos-remoted-bin',Version='0.1.11-1')])"
run
grep -q '^publish=false$' "$GITHUB_OUTPUT"
grep -q '^publish_helper=true$' "$GITHUB_OUTPUT"
checks=$((checks+1))
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Name='lightos-codec-ffmpeg-bin',Version='0.1.11-1')])"
run
grep -q '^publish=true$' "$GITHUB_OUTPUT"
grep -q '^publish_helper=false$' "$GITHUB_OUTPUT"
checks=$((checks+1))
reset
mutate aur.json "r.update(resultcount=2,results=[dict(Name=n,Version='0.1.11-1') for n in ('lightos-remoted-bin','lightos-codec-ffmpeg-bin')])"
run
grep -q '^publish=false$' "$GITHUB_OUTPUT"
grep -q '^publish_helper=false$' "$GITHUB_OUTPUT"
checks=$((checks+1))
FORCE_RELEASE=true
: > "$GITHUB_OUTPUT"
run
grep -q '^publish=true$' "$GITHUB_OUTPUT"
grep -q '^publish_helper=true$' "$GITHUB_OUTPUT"
checks=$((checks+1))
for package in lightos-remoted-bin lightos-codec-ffmpeg-bin; do
    reset
    sed -i 's/^pkgrel=.*/pkgrel=2/' "$temporary/$package/PKGBUILD"
    run
    grep -q '^pkgrel=2$' "$temporary/$package/PKGBUILD"
    checks=$((checks+1))
    mutate aur.json "r.update(resultcount=1,results=[dict(Name='$package',Version='0.1.11-1')])"
    run
    grep -q '^pkgrel=2$' "$temporary/$package/PKGBUILD"
    checks=$((checks+1))
    reset
    mutate aur.json "r.update(resultcount=1,results=[dict(Name='$package',Version='0.1.11-2')])"
    reject "$package pkgrel downgrade"
    FORCE_RELEASE=true
    reject "$package forced pkgrel downgrade"
    reset
    mutate aur.json "r.update(resultcount=1,results=[dict(Name='$package',Version='0.2.0-1')])"
    reject "$package version downgrade"
    reset
    sed -i 's/^pkgver=.*/pkgver=0.1.10/; s/^pkgrel=.*/pkgrel=4/' "$temporary/$package/PKGBUILD"
    run
    grep -q '^pkgver=0.1.11$' "$temporary/$package/PKGBUILD"
    grep -q '^pkgrel=1$' "$temporary/$package/PKGBUILD"
    checks=$((checks+1))
done
reset
mutate aur.json "r.update(resultcount=2,results=[dict(Name='lightos-codec-ffmpeg-bin',Version='0.1.11-1')]*2)"
reject 'duplicate AUR package'
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Name='unexpected',Version='0.1.11-1')])"
reject 'unexpected AUR package'
reset
mutate aur.json "r.update(resultcount=1)"
reject 'inconsistent AUR count'
# The deploy action runs pruning after real checksums and .SRCINFO generation.
prune_fixture=$temporary/prune
reset_prune() {
    local file
    rm -rf -- "$prune_fixture"
    mkdir -p "$prune_fixture/.git" "$prune_fixture/src" "$prune_fixture/pkg"
    printf "sha256sums=('abc123')\n" > "$prune_fixture/PKGBUILD"
    for file in .SRCINFO lightos-remoted-bin.install aquamarine-wayland.patch; do
        printf 'fixture\n' > "$prune_fixture/$file"
    done
    for file in bootstrap.sh activation.sh lightos-remote-desktop.service lightos-remote-desktop-session.service download.tar.gz upstream.env .state.json; do
        printf 'obsolete fixture\n' > "$prune_fixture/$file"
    done
    ln -s /nonexistent "$prune_fixture/dangling-download"
}
prune() { (cd "$prune_fixture" && bash "$repository/.github/scripts/prune-lightos-aur.sh"); }
reject_prune() { if prune; then echo "FAIL: pruning accepted $1" >&2; exit 1; fi; checks=$((checks+1)); }
reset_prune
prune
printf '%s\n' .SRCINFO .git PKGBUILD aquamarine-wayland.patch lightos-remoted-bin.install > "$temporary/expected-allowlist"
find "$prune_fixture" -mindepth 1 -maxdepth 1 -printf '%f\n' | LC_ALL=C sort > "$temporary/actual-allowlist"
cmp "$temporary/expected-allowlist" "$temporary/actual-allowlist"
checks=$((checks+1))
for file in PKGBUILD .SRCINFO lightos-remoted-bin.install aquamarine-wayland.patch; do
    reset_prune
    rm -- "$prune_fixture/$file"
    reject_prune "missing $file"
    reset_prune
    rm -- "$prune_fixture/$file"
    ln -s "$temporary/lightos-remoted-bin/PKGBUILD" "$prune_fixture/$file"
    reject_prune "symlink $file"
done
reset_prune
printf "sha256sums=('SKIP')\n" > "$prune_fixture/PKGBUILD"
reject_prune 'unresolved checksum'
reset_prune
printf 'fixture\n' > "$prune_fixture/lightos-codec-ffmpeg-bin.install"
printf 'fixture\n' > "$prune_fixture/validate-arch-codec.py"
(cd "$prune_fixture" && bash "$repository/.github/scripts/prune-lightos-aur.sh" lightos-codec-ffmpeg-bin)
printf '%s\n' .SRCINFO .git PKGBUILD lightos-codec-ffmpeg-bin.install validate-arch-codec.py > "$temporary/expected-allowlist"
find "$prune_fixture" -mindepth 1 -maxdepth 1 -printf '%f\n' | LC_ALL=C sort > "$temporary/actual-allowlist"
cmp "$temporary/expected-allowlist" "$temporary/actual-allowlist"
checks=$((checks+1))
for file in lightos-codec-ffmpeg-bin.install validate-arch-codec.py; do
    reset_prune
    printf 'fixture\n' > "$prune_fixture/lightos-codec-ffmpeg-bin.install"
    printf 'fixture\n' > "$prune_fixture/validate-arch-codec.py"
    rm "$prune_fixture/$file"
    if (cd "$prune_fixture" && bash "$repository/.github/scripts/prune-lightos-aur.sh" lightos-codec-ffmpeg-bin); then echo "Missing helper asset accepted" >&2; exit 1; fi
    checks=$((checks+1))
done
if (cd "$prune_fixture" && bash "$repository/.github/scripts/prune-lightos-aur.sh" unknown); then echo 'Unknown package accepted' >&2; exit 1; fi
checks=$((checks+1))
printf 'AUR release guards: %s checks passed (fixture APIs; no external publication).\n' "$checks"
