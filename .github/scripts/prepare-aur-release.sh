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
assets = r['assets']
assert len({a['name'] for a in assets}) == len(assets), 'Duplicate assets'
assert {a['name'] for a in assets} == {base, base+'.sha256', worker, worker+'.sha256'}, 'Unexpected/missing release assets'
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
# Compare against AUR as well as tracked metadata, preventing dispatch downgrades.
curl --fail --silent --show-error --location --retry 3 --retry-all-errors --connect-timeout 15 --max-time 120 \
    'https://aur.archlinux.org/rpc/v5/info?arg[]=lightos-remoted-bin' > "$temporary/aur.json"
python3 - "$temporary/aur.json" "$version" "${FORCE_RELEASE:-false}" >> "$GITHUB_OUTPUT" <<'PY'
import json, re, sys
from pathlib import Path
wanted = tuple(map(int, sys.argv[2].split('.')))
tracked = re.search(r'^pkgver=(\d+\.\d+\.\d+)$', Path('lightos-remoted-bin/PKGBUILD').read_text(), re.M)[1]
assert wanted >= tuple(map(int, tracked.split('.'))), 'Refusing downgrade from tracked metadata'
r = json.load(open(sys.argv[1]))
assert r['type'] == 'multiinfo' and r['resultcount'] in (0, 1), 'Invalid AUR response'
same = False
if r['resultcount']:
    v = r['results'][0]['Version']
    m = re.fullmatch(r'(\d+)\.(\d+)\.(\d+)-(\d+)', v)
    assert m, 'Unrecognized AUR version; inspect before publishing'
    current = tuple(map(int, m.groups()[:3]))
    assert wanted >= current, 'Refusing AUR downgrade'
    if wanted == current:
        assert int(m[4]) == 1, 'Refusing pkgrel downgrade'
        same = True
print('publish=' + str(not same or sys.argv[3] == 'true').lower())
PY
# Download the actual core and verify the published checksum and ELF dependencies.
gh release download "$RELEASE_TAG" --repo "$GH_REPO" --pattern 'lightos-remoted-linux-x86_64.tar.gz' --pattern 'lightos-remoted-linux-x86_64.tar.gz.sha256' --dir "$temporary"
(cd "$temporary" && sha256sum --check lightos-remoted-linux-x86_64.tar.gz.sha256)
tar -xzf "$temporary/lightos-remoted-linux-x86_64.tar.gz" -C "$temporary"
binary=$temporary/lightos-remoted-linux-x86_64/lightos-remoted
readelf -h "$binary" | grep -Fq 'Advanced Micro Devices X86-64'
if readelf -d "$binary" | grep -Eq 'NEEDED.*lib(avcodec|avformat|avutil|swscale)'; then echo 'Core must not require FFmpeg' >&2; exit 1; fi
test -s "$temporary/lightos-remoted-linux-x86_64/LICENSE"
sed -i "s/^pkgver=.*/pkgver=$version/; s/^pkgrel=.*/pkgrel=1/" lightos-remoted-bin/PKGBUILD
printf 'version=%s\n' "$version" >> "$GITHUB_OUTPUT"
