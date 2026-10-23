#!/usr/bin/env python3
"""Collect locked MCP, RDP, desktop backend and codec-protocol runtime notices."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

repository = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output([
    os.environ.get("CARGO", "cargo"), "metadata", "--locked", "--format-version", "1",
    "--features", "bundled-jpeg,rdp,portal", "--filter-platform", "x86_64-unknown-linux-gnu",
], cwd=repository, text=True))
fallback_root = repository / "docs/licenses/protocols"
fallbacks = {(entry["name"], entry["version"]): entry for entry in json.loads((fallback_root / "sources.json").read_text())}
packages = {package["id"]: package for package in metadata["packages"]}
nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
roots = {"rmcp", "schemars", "rand", "sha2", "tokio-util", "ironrdp-server", "ironrdp-pdu", "rustls-pemfile", "rcgen", "bincode", "x11rb", "enigo", "arboard", "zbus", "ashpd", "pipewire", "configparser"}
pending = [key for key, package in packages.items() if package["name"] in roots]
visited = set()
while pending:
    key = pending.pop()
    if key in visited:
        continue
    visited.add(key)
    pending.extend(dep["pkg"] for dep in nodes[key]["deps"]
                   if any(kind["kind"] is None for kind in dep["dep_kinds"]))
missing = []
notices = ["Locked Linux x86_64 MCP, RDP, desktop backend and codec-protocol dependency notices (bundled-jpeg,rdp,portal).\n"]
for package in sorted((packages[key] for key in visited), key=lambda p: (p["name"], p["version"])):
    if package["source"] is None:
        continue
    label = f'{package["name"]}-{package["version"]}'
    source = Path(package["manifest_path"]).parent
    entries = sorted({*source.glob("LICENSE*"), *source.glob("LICENCE*"), *source.glob("NOTICE*"),
                      *source.glob("COPYING*"), *source.glob("COPYRIGHT*")})
    files = sorted({path for entry in entries
                    for path in ([entry] if entry.is_file() else entry.rglob("*"))
                    if path.is_file()})
    if not files and package["name"] in {"rmcp", "rmcp-macros"} and package["version"] == "3.1.2":
        # These archives omit the license. The exact VCS root license is retained.
        files = [repository / "docs/licenses/rmcp.txt"]
    fallback = fallbacks.get((package["name"], package["version"]))
    if not files and fallback:
        vcs = json.loads((source / ".cargo_vcs_info.json").read_text())
        if vcs["git"]["sha1"] != fallback["commit"]:
            raise SystemExit(f"Upstream notice revision mismatch: {label}")
        for entry in fallback["files"]:
            path = fallback_root / entry["path"]
            if hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
                raise SystemExit(f"Upstream notice checksum mismatch: {label}/{path.name}")
            files.append(path)
    if not files:
        missing.append(label)
        continue
    notices.append(f"\n=== {label} ({package['license'] or 'see upstream notice'}) ===\n")
    for path in files:
        text = path.read_text()
        if not text.strip():
            raise SystemExit(f"Empty dependency notice: {label}/{path.name}")
        notices.append(f"\n--- {path.name} ---\n{text}\n")
if missing:
    raise SystemExit("Missing dependency notices: " + ", ".join(missing))
output = Path(sys.argv[1])
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text("".join(notices))
print(f"Collected notices for {sum(packages[key]['source'] is not None for key in visited)} locked runtime crates")
