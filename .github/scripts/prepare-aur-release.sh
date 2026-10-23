#!/usr/bin/env bash
# Read-only upstream verification; mutates only the checked-out PKGBUILD.
set -Eeuo pipefail
: "${GH_REPO:?}" "${RELEASE_TAG:?}" "${GITHUB_OUTPUT:?}"
[[ $GH_REPO == ca-x/lightos-remote-desktop ]] || { echo 'Unexpected upstream repository' >&2; exit 1; }
[[ $RELEASE_TAG =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo 'Expected stable vX.Y.Z tag' >&2; exit 1; }
version=${RELEASE_TAG#v}
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT
gh api "repos/$GH_REPO/releases/tags/$RELEASE_TAG" > "$temporary/release.json"
python3 - "$temporary/release.json" "$RELEASE_TAG" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
assert r['tag_name'] == sys.argv[2] and not r['draft'] and not r['prerelease'] and r['published_at'], 'Release must be published and stable'
base = 'lightos-remoted-linux-x86_64.tar.gz'
worker = 'lightos-codec-ffmpeg-linux-x86_64-ubuntu-24.04.tar.gz'
arch = 'lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz'
assets = r['assets']
assert len({a['name'] for a in assets}) == len(assets), 'Duplicate assets'
assert {a['name'] for a in assets} == {base, base+'.sha256', worker, worker+'.sha256', arch, arch+'.sha256'}, 'Unexpected/missing release assets'
assert all(a['state'] == 'uploaded' and a['size'] > 0 for a in assets), 'Incomplete assets'
PY
# Resolve annotated tags through the commits API and require successful CI for
# exactly that tag/commit. A release event can arrive before CI finishes.
commit=$(gh api "repos/$GH_REPO/commits/$RELEASE_TAG" --jq .sha)
[[ $commit =~ ^[0-9a-f]{40}$ ]]
ready=false
for _attempt in {1..60}; do
    gh api --method GET "repos/$GH_REPO/actions/workflows/ci.yml/runs" -f head_sha="$commit" -f event=push -f per_page=100 > "$temporary/runs.json"
    if python3 - "$temporary/runs.json" "$RELEASE_TAG" "$commit" <<'PY'
import json, sys
runs = [r for r in json.load(open(sys.argv[1]))['workflow_runs'] if r['head_branch'] == sys.argv[2] and r['head_sha'] == sys.argv[3]]
assert runs, 'No CI run for release tag'
latest = max(runs, key=lambda r: r['run_number'])
assert latest['status'] == 'completed' and latest['conclusion'] == 'success', 'Release CI is not successful'
PY
    then ready=true; break; fi
    sleep 15
done
[[ $ready == true ]] || { echo 'Release tag CI did not pass' >&2; exit 1; }
# Check both packages independently. Same-version retries preserve tracked pkgrel;
# force only republishes identical versions, never a lower version or pkgrel.
curl --globoff --fail --silent --show-error --location --retry 3 --retry-all-errors --connect-timeout 15 --max-time 120 \
    'https://aur.archlinux.org/rpc/v5/info?arg[]=lightos-remoted-bin&arg[]=lightos-codec-ffmpeg-bin' > "$temporary/aur.json"
python3 - "$temporary/aur.json" "$version" "${FORCE_RELEASE:-false}" "$temporary/decisions.json" <<'PYTHON'
import json, re, sys
from pathlib import Path
wanted = tuple(map(int, sys.argv[2].split('.')))
assert sys.argv[3] in ('true', 'false'), 'Invalid force input'
r = json.load(open(sys.argv[1]))
names = ('lightos-remoted-bin', 'lightos-codec-ffmpeg-bin')
assert r['type'] == 'multiinfo' and r['resultcount'] == len(r['results']) <= 2, 'Invalid AUR response'
assert all(p['Name'] in names for p in r['results']), 'Unexpected AUR package'
current = {p['Name']: p['Version'] for p in r['results']}
assert len(current) == len(r['results']), 'Duplicate AUR package'
decisions = {}
for name in names:
    text = Path(name, 'PKGBUILD').read_text()
    tracked = re.search(r'^pkgver=(\d+\.\d+\.\d+)$', text, re.M)
    rel = re.search(r'^pkgrel=([1-9]\d*)$', text, re.M)
    assert tracked and rel, 'Unrecognized tracked package version'
    tracked_version = tuple(map(int, tracked[1].split('.')))
    assert wanted >= tracked_version, f'Refusing downgrade from tracked {name}'
    pkgrel = int(rel[1]) if wanted == tracked_version else 1
    same = False
    if name in current:
        match = re.fullmatch(r'(\d+)\.(\d+)\.(\d+)-([1-9]\d*)', current[name])
        assert match, f'Unrecognized AUR version for {name}'
        published = tuple(map(int, match.groups()[:3]))
        assert wanted >= published, f'Refusing AUR downgrade for {name}'
        if wanted == published:
            assert pkgrel >= int(match[4]), f'Refusing pkgrel downgrade for {name}'
            same = pkgrel == int(match[4])
    decisions[name] = dict(pkgrel=pkgrel, publish=not same or sys.argv[3] == 'true')
Path(sys.argv[4]).write_text(json.dumps(decisions))
PYTHON
# Download the actual core and verify the published checksum and ELF dependencies.
gh release download "$RELEASE_TAG" --repo "$GH_REPO" --pattern 'lightos-remoted-linux-x86_64.tar.gz' --pattern 'lightos-remoted-linux-x86_64.tar.gz.sha256' --pattern 'lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz' --pattern 'lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz.sha256' --dir "$temporary"
(cd "$temporary" && sha256sum --check lightos-remoted-linux-x86_64.tar.gz.sha256 && sha256sum --check lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz.sha256)
python3 - "$temporary/lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz" <<'PYTHON'
from pathlib import PurePosixPath
import sys, tarfile
with tarfile.open(sys.argv[1]) as archive:
    for member in archive:
        path = PurePosixPath(member.name)
        assert not path.is_absolute() and '..' not in path.parts
        assert path.parts[0] == 'lightos-codec-ffmpeg-linux-x86_64-archlinux'
        assert member.isdir() or member.isfile(), 'Unexpected worker archive link/device'
PYTHON
tar -xzf "$temporary/lightos-codec-ffmpeg-linux-x86_64-archlinux.tar.gz" -C "$temporary"
python3 lightos-codec-ffmpeg-bin/validate-arch-codec.py "$temporary/lightos-codec-ffmpeg-linux-x86_64-archlinux" "$commit"
tar -xzf "$temporary/lightos-remoted-linux-x86_64.tar.gz" -C "$temporary"
binary=$temporary/lightos-remoted-linux-x86_64/lightos-remoted
readelf -h "$binary" | grep -Fq 'Advanced Micro Devices X86-64'
if readelf -d "$binary" | grep -Eq 'NEEDED.*lib(avcodec|avformat|avutil|swscale)'; then echo 'Core must not require FFmpeg' >&2; exit 1; fi
test -s "$temporary/lightos-remoted-linux-x86_64/LICENSE"
python3 - "$temporary/decisions.json" "$version" >> "$GITHUB_OUTPUT" <<'PYTHON'
import json, re, sys
from pathlib import Path
for name, suffix in [('lightos-remoted-bin', ''), ('lightos-codec-ffmpeg-bin', '_helper')]:
    decision = json.load(open(sys.argv[1]))[name]
    path = Path(name, 'PKGBUILD')
    text = re.sub(r'^pkgver=.*$', 'pkgver=' + sys.argv[2], path.read_text(), flags=re.M)
    text = re.sub(r'^pkgrel=.*$', 'pkgrel=' + str(decision['pkgrel']), text, flags=re.M)
    path.write_text(text)
    print('publish' + suffix + '=' + str(decision['publish']).lower())
    print('pkgrel' + suffix + '=' + str(decision['pkgrel']))
print('version=' + sys.argv[2])
PYTHON
