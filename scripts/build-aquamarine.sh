#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
SOURCE="$ROOT/.local/aquamarine"
BUILD="$SOURCE/build"
PATCH="$ROOT/patches/aquamarine-wayland.patch"
UPSTREAM=https://github.com/hyprwm/aquamarine.git
VERSION=0.15.1
COMMIT=f31c47a1b9d300847d8fc3108ad959448103dfc1
JOBS=${CMAKE_BUILD_PARALLEL_LEVEL:-2}

fail() { printf '%s\n' "$*" >&2; exit 1; }

[[ $# == 0 ]] || fail "Usage: $0 (optional CMAKE_BUILD_PARALLEL_LEVEL)"
[[ $JOBS =~ ^[1-9][0-9]*$ ]] || fail "CMAKE_BUILD_PARALLEL_LEVEL must be a positive integer"
[[ $(id -u) != 0 ]] || fail "Build the private library as an unprivileged user"
for command in git cmake ninja pkg-config readelf sha256sum; do
    command -v "$command" >/dev/null || fail "Missing build tool: $command"
done
[[ $(pkg-config --modversion aquamarine) == "$VERSION" ]] || fail "This patch is only verified against installed Aquamarine $VERSION (SONAME 14)"
[[ -f $PATCH ]] || fail "Missing compatibility patch: $PATCH"
if [[ ! -e $SOURCE ]]; then
    mkdir -p -- "$ROOT/.local"
    git clone --branch "v$VERSION" --depth 1 -- "$UPSTREAM" "$SOURCE"
fi
[[ $(git -C "$SOURCE" remote get-url origin) == "$UPSTREAM" ]] || fail "Unexpected Aquamarine source origin"
[[ $(git -C "$SOURCE" rev-parse HEAD) == "$COMMIT" ]] || fail "Unexpected Aquamarine source commit"
[[ $(tr -d '\n\r' < "$SOURCE/VERSION") == "$VERSION" ]] || fail "Unexpected source VERSION"
git -C "$SOURCE" diff --cached --quiet || fail "Refusing staged Aquamarine changes"
[[ -z $(git -C "$SOURCE" ls-files --others --exclude-standard) ]] || fail "Refusing untracked Aquamarine source files"
if git -C "$SOURCE" diff --quiet; then
    git -C "$SOURCE" apply --check "$PATCH"
    git -C "$SOURCE" apply "$PATCH"
fi
[[ $(git -C "$SOURCE" diff --name-only) == src/backend/Wayland.cpp ]] || fail "Refusing unrelated Aquamarine changes"
EXPECTED_PATCH=$(git patch-id --stable < "$PATCH" | cut -d ' ' -f 1)
ACTUAL_PATCH=$(git -C "$SOURCE" diff --src-prefix=a/ --dst-prefix=b/ | git patch-id --stable | cut -d ' ' -f 1)
[[ -n $EXPECTED_PATCH && $ACTUAL_PATCH == "$EXPECTED_PATCH" ]] || fail "Aquamarine changes do not match the recorded patch"
git -C "$SOURCE" apply --reverse --check "$PATCH"

cmake -S "$SOURCE" -B "$BUILD" -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=OFF
cmake --build "$BUILD" --target aquamarine --parallel "$JOBS"
LIBRARY="$BUILD/libaquamarine.so.$VERSION"
readelf -d "$LIBRARY" | grep -Fq 'Library soname: [libaquamarine.so.14]' || fail "Unexpected private library SONAME"
[[ -e $BUILD/libaquamarine.so.14 ]] || fail "Missing private SONAME symlink"
{
    printf 'upstream=%s\nversion=%s\ncommit=%s\nsoname=libaquamarine.so.14\n' "$UPSTREAM" "$VERSION" "$COMMIT"
    sha256sum "$PATCH" "$LIBRARY"
} > "$BUILD/compatibility.txt"
printf '\nPrivate Aquamarine library directory: %s\nReceipt: %s\n' "$BUILD" "$BUILD/compatibility.txt"
