#!/usr/bin/env bash
# Use compatible SPA inline headers without changing the system PipeWire ABI.
set -euo pipefail

sdk=${1:?Usage: prepare-spa-build-sdk.sh ABSOLUTE_SDK_DIRECTORY}
case "$sdk" in
  /*) ;;
  *) echo 'SPA SDK directory must be absolute' >&2; exit 1 ;;
esac
case "$sdk" in
  /|*[[:space:]]*) echo 'SPA SDK directory must be non-root and contain no whitespace' >&2; exit 1 ;;
esac

version=1.0.5-1ubuntu3.3
package=libspa-0.2-dev_${version}_amd64.deb
url=https://archive.ubuntu.com/ubuntu/pool/main/p/pipewire/$package
checksum=93a60e7656fb35c97a78159fd0556842c62c2d8b9aff096009f9a20c64c1f272
mkdir -p -- "$sdk"
archive=$sdk/$package
curl --fail --silent --show-error --location --retry 3 --retry-all-errors \
  --connect-timeout 15 --max-time 120 "$url" --output "$archive"
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check

if command -v dpkg-deb >/dev/null 2>&1; then
  dpkg-deb --extract "$archive" "$sdk"
elif command -v bsdtar >/dev/null 2>&1; then
  bsdtar -xOf "$archive" data.tar.zst | bsdtar -xf - -C "$sdk"
else
  echo 'SPA SDK extraction requires dpkg-deb or bsdtar' >&2
  exit 1
fi

pc_directory=$sdk/usr/lib/x86_64-linux-gnu/pkgconfig
pc=$pc_directory/libspa-0.2.pc
test -s "$pc"
test -s "$sdk/usr/include/spa-0.2/spa/buffer/meta.h"
test -s "$sdk/usr/share/doc/libspa-0.2-dev/copyright"
# Only this header-only package is relocated. libpipewire.pc, its headers and
# the linked libpipewire-0.3.so.0 remain those provided by the build OS.
python3 - "$pc" "$sdk/usr" <<'PYTHON'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
assert text.count("prefix=/usr\n") == 1, "Unexpected SPA pkg-config prefix"
path.write_text(text.replace("prefix=/usr\n", "prefix=" + sys.argv[2] + "\n", 1))
PYTHON
python3 - "$sdk" <<'PYTHON'
from pathlib import Path
import re
import sys
sdk = Path(sys.argv[1])
headers = list((sdk / "usr/include/spa-0.2").rglob("*.h"))
notices = set()
licenses = set()
for header in headers:
    text = header.read_text()
    notices.update(re.findall(r"SPDX-FileCopyrightText: ([^\r\n]+)", text))
    licenses.update(re.findall(r"SPDX-License-Identifier: ([^\s*]+)", text))
assert headers and notices and licenses == {"MIT"}, "Unexpected SPA header notices"
original = (sdk / "usr/share/doc/libspa-0.2-dev/copyright").read_text()
(sdk / "header-notices.txt").write_text(
    original + "\nAdditional SPDX copyright notices from the bundled build headers (MIT):\n"
    + "\n".join(sorted(notices)) + "\n"
)
PYTHON
export PKG_CONFIG_PATH=$pc_directory${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}
pkg-config --cflags libspa-0.2
pkg-config --cflags --libs libpipewire-0.3
cat > "$sdk/build-sdk.txt" <<EOF
SPA headers: $version
Source: $url
SHA256: $checksum
Linked PipeWire SDK: $(pkg-config --modversion libpipewire-0.3)
EOF
cat "$sdk/build-sdk.txt"
if [[ -n ${GITHUB_ENV:-} ]]; then
  printf 'PKG_CONFIG_PATH=%s\n' "$PKG_CONFIG_PATH" >> "$GITHUB_ENV"
fi
