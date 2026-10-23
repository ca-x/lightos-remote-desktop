#!/usr/bin/env bash
# Exercise release publication guards with isolated API/download fixtures.
set -Eeuo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT
mkdir -p "$temporary/bin" "$temporary/lightos-remoted-bin"
cp "$repository/lightos-remoted-bin/PKGBUILD" "$temporary/lightos-remoted-bin/"
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
    python3 - <<'PY'
import json, os
from pathlib import Path
p = Path(os.environ['AUR_RELEASE_FIXTURE'])
base = 'lightos-remoted-linux-x86_64.tar.gz'
worker = 'lightos-codec-ffmpeg-linux-x86_64-ubuntu-24.04.tar.gz'
(p/'release.json').write_text(json.dumps(dict(tag_name='v0.1.11', draft=False, prerelease=False, published_at='2026-10-23', assets=[dict(name=n, state='uploaded', size=123) for n in (base,base+'.sha256',worker,worker+'.sha256')])))
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
mutate aur.json "r.update(resultcount=1,results=[dict(Version='0.2.0-1')])"
reject 'AUR downgrade'
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Version='0.1.11-2')])"
reject 'pkgrel downgrade'
reset
mutate aur.json "r.update(resultcount=1,results=[dict(Version='0.1.11-1')])"
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
printf 'AUR release guards: %s checks passed (fixture APIs; no external publication).\n' "$checks"
