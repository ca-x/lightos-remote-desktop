#!/usr/bin/env bash
set -euo pipefail
package=${1:-lightos-remoted-bin}
case $package in
    lightos-remoted-bin) assets=(lightos-remoted-bin.install aquamarine-wayland.patch) ;;
    lightos-codec-ffmpeg-bin) assets=(lightos-codec-ffmpeg-bin.install validate-arch-codec.py) ;;
    *) echo 'Unknown AUR package allowlist' >&2; exit 1 ;;
esac
# Only exact package assets and action-generated metadata survive deployment.
for file in * .[^.]*; do
    [[ -e $file || -L $file ]] || continue
    keep=false
    for allowed in .git .SRCINFO PKGBUILD "${assets[@]}"; do
        [[ $file != "$allowed" ]] || keep=true
    done
    if [[ $keep == false ]]; then rm -rf -- "$file"; fi
done
for file in PKGBUILD .SRCINFO "${assets[@]}"; do
    [[ -f $file && ! -L $file ]]
done
! grep -Eq "['\"]SKIP['\"]" PKGBUILD
