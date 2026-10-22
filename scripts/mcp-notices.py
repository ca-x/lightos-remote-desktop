#!/usr/bin/env python3
"""Collect notices for MCP's direct runtime dependencies from the locked graph."""
import os
from pathlib import Path
import sys
import tomllib

repository = Path(__file__).resolve().parent.parent
registry = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")) / "registry/src"
packages = tomllib.loads((repository / "Cargo.lock").read_text())["package"]
names = {"rmcp", "rmcp-macros", "schemars", "schemars_derive", "sha2", "rand", "tokio-util"}
notices = []
for package in packages:
    if package["name"] not in names:
        continue
    label = f'{package["name"]}-{package["version"]}'
    sources = sorted(registry.glob(f"*/{label}"))
    if not sources:
        raise SystemExit(f"Missing locked crate source: {label}")
    source = sources[0]
    files = sorted({*source.glob("LICENSE*"), *source.glob("NOTICE*"), *source.glob("COPYING*")})
    if not files and package["name"] in {"rmcp", "rmcp-macros"} and package["version"] == "3.1.2":
        # rmcp 3.1.2's crate archives omit the repository license. Retain the
        # upstream root license from their exact .cargo_vcs_info.json commit.
        files = [repository / "docs/licenses/rmcp.txt"]
    if not files:
        raise SystemExit(f"Missing dependency notice: {label}")
    notices.append(f"\n=== {label} ===\n")
    for path in files:
        text = path.read_text()
        if not text.strip():
            raise SystemExit(f"Empty dependency notice: {label}/{path.name}")
        notices.append(f"\n--- {path.name} ---\n{text}\n")
output = Path(sys.argv[1])
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text("".join(notices))
