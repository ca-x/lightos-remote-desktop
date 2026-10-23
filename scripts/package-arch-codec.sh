#!/usr/bin/env bash
# Package an already-built Arch worker. Does not build or install anything.
set -Eeuo pipefail
source_root=$(realpath "${1:?source checkout}")
destination=$(realpath -m "${2:?output directory}")
tooling_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
package=lightos-codec-ffmpeg-linux-x86_64-archlinux
staging=$(mktemp -d)
trap 'rm -rf -- "$staging"' EXIT
mkdir -p "$staging/$package/licenses" "$destination"
install -m755 "$source_root/target/release/lightos-codec-ffmpeg" "$staging/$package/"
install -m644 "$source_root/LICENSE" "$staging/$package/"
install -m644 "$source_root/docs/hardware-encoding.md" "$staging/$package/README.md"
install -m644 "$source_root/docs/licenses/ffmpeg-runtime.txt" "$staging/$package/licenses/"
python3 "$tooling_root/scripts/codec-worker-notices.py" "$source_root" "$staging/$package/licenses/dependencies.txt"
ldd "$staging/$package/lightos-codec-ffmpeg" | tee "$staging/$package/dependencies.txt"
if grep -q 'not found' "$staging/$package/dependencies.txt"; then
    echo 'Arch worker has unresolved runtime dependencies' >&2
    exit 1
fi
pkg-config --modversion libavcodec libavformat libavutil libswscale > "$staging/$package/ffmpeg-abi.txt"
python3 - "$staging/$package" "$source_root" <<'PY'
import json
from pathlib import Path
import re
import subprocess
import sys

root = Path(sys.argv[1])
dynamic = subprocess.check_output(['readelf', '-d', str(root / 'lightos-codec-ffmpeg')], text=True)
needed = re.findall(r'\(NEEDED\).*\[(.*?)\]', dynamic)
sonames = sorted(name for name in needed if re.fullmatch(r'lib(?:avcodec|avformat|avutil|swscale)\.so\.\d+', name))
required = ('libavcodec.so.', 'libavutil.so.', 'libswscale.so.')
assert all(any(name.startswith(prefix) for name in sonames) for prefix in required), f'Missing required FFmpeg libraries: {sonames}'
metadata = {
    'platform': 'archlinux-x86_64',
    'source_commit': subprocess.check_output(['git', '-C', sys.argv[2], 'rev-parse', 'HEAD'], text=True).strip(),
    'ffmpeg_package': subprocess.check_output(['pacman', '-Q', 'ffmpeg'], text=True).strip(),
    'needed': needed,
    'ffmpeg_sonames': sonames,
}
(root / 'abi.json').write_text(json.dumps(metadata, indent=2) + '\n')
PY
tar -C "$staging" -czf "$destination/$package.tar.gz" "$package"
(cd "$destination" && sha256sum "$package.tar.gz" > "$package.tar.gz.sha256")
