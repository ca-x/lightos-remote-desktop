#!/usr/bin/env python3
"""Collect locked native worker runtime licenses, including vendored native code."""
import json
import os
from pathlib import Path
import subprocess
import sys

repository = Path(sys.argv[1]).resolve()
metadata = json.loads(subprocess.check_output([
    os.environ.get("CARGO", "cargo"), "metadata", "--locked", "--offline",
    "--format-version", "1", "--features", "media-codec-ffmpeg/native,media-codec/bundled-jpeg",
    "--filter-platform", "x86_64-unknown-linux-gnu",
], cwd=repository, text=True))
packages = {package["id"]: package for package in metadata["packages"]}
nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
by_name = {package["name"]: package for package in packages.values()}
pending = [key for key, package in packages.items() if package["name"] == "media-codec-ffmpeg"]
if len(pending) != 1:
    raise SystemExit("Expected exactly one native worker package")
visited = set()
while pending:
    key = pending.pop()
    if key in visited:
        continue
    visited.add(key)
    pending.extend(dep["pkg"] for dep in nodes[key]["deps"]
                   if any(kind["kind"] is None for kind in dep["dep_kinds"]))
notices = ["Locked Linux x86_64 native codec worker runtime dependency notices.\n"]
for package in sorted((packages[key] for key in visited), key=lambda p: (p["name"], p["version"])):
    if package["source"] is None:
        continue
    name = package["name"]
    source = Path(package["manifest_path"]).parent
    entries = sorted({*source.glob("LICENSE*"), *source.glob("LICENCE*"),
                      *source.glob("NOTICE*"), *source.glob("COPYING*")})
    files = sorted({path for entry in entries
                    for path in ([entry] if entry.is_file() else entry.rglob("*"))
                    if path.is_file()})
    if name in {"openh264", "openh264-sys2"}:
        files.append(repository / "docs/licenses/openh264-rust.txt")
        if name == "openh264-sys2":
            files.append(source / "upstream/LICENSE")
    if name == "turbojpeg-sys":
        wrapper = Path(by_name["turbojpeg"]["manifest_path"]).parent
        files.extend([wrapper / "LICENSE", wrapper / "UNLICENSE", source / "libjpeg-turbo/LICENSE.md", source / "libjpeg-turbo/README.ijg"])
    if name == "ffmpeg-sys-next" and not files:
        # Both bindings declare WTFPL; retain its full text from the sibling wrapper.
        files.append(Path(by_name["ffmpeg-next"]["manifest_path"]).parent / "LICENSE")
    if not files:
        raise SystemExit(f"Missing dependency notice: {name}-{package['version']}")
    notices.append(f"\n=== {name}-{package['version']} ({package['license']}) ===\n")
    for path in files:
        text = path.read_text()
        if not text.strip():
            raise SystemExit(f"Empty dependency notice: {path}")
        notices.append(f"\n--- {path.name} ---\n{text}\n")
output = Path(sys.argv[2])
output.write_text("".join(notices))
