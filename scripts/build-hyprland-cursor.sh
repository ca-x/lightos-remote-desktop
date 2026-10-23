#!/usr/bin/env bash
# Build the optional read-only adapter against this host's exact Hyprland ABI.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cursor_output=${1:-"$source_root/target/release/lightos-cursor.so"}
pkg-config --exists hyprland
mkdir -p -- "$(dirname -- "$cursor_output")"
read -r -a cursor_flags <<< "$(pkg-config --cflags hyprland)"
"${CXX:-g++}" -std=c++23 -O2 -shared -fPIC -fno-gnu-unique "${cursor_flags[@]}" \
    "$source_root/native/hyprland-cursor.cpp" -o "$cursor_output"
