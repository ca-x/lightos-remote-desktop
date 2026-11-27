"""Build the non-serving replacement accepted by the PC runtime installer."""
import hashlib
import json
import os
import plistlib
import re
import subprocess
import tarfile
import tempfile
from pathlib import Path

VERSION = "0.0.0-uninstalled"


def build_removal(source, output, target, identity=None):
    system, cpu = target.split("-")
    goos = {"darwin": "darwin", "linux": "linux", "win32": "windows"}[system]
    env = dict(os.environ, GOOS=goos, GOARCH="amd64" if cpu == "x64" else cpu, CGO_ENABLED="0")
    output.mkdir(exist_ok=True, parents=True)
    archive = output / f"{target}.tar.gz"
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        binary = root / ("uninstalled.exe" if goos == "windows" else "uninstalled")
        if system == "darwin":
            app = root / "Lazycat Remote Uninstalled.app"
            binary = app / "Contents/MacOS/uninstalled"
            binary.parent.mkdir(parents=True)
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps({
                "CFBundleIdentifier": "org.lightos.remote-desktop.uninstalled",
                "CFBundleExecutable": "uninstalled", "CFBundlePackageType": "APPL",
                "CFBundleVersion": "1", "LSUIElement": True,
            }))
        subprocess.run(["go", "build", "-trimpath", "-ldflags=-s -w", "-o", binary,
                        source / "native/uninstalled/main.go"], env=env, check=True)
        if system == "darwin":
            subprocess.run(["codesign", "--force", "--sign", identity or "-", "--options",
                            "runtime", "--timestamp=none", app], check=True)
            subprocess.run(["codesign", "--verify", "--deep", "--strict", app], check=True)
        with tarfile.open(archive, "w:gz", format=tarfile.USTAR_FORMAT) as package:
            for path in sorted(root.rglob("*")):
                if path.is_file():
                    package.add(path, arcname=path.relative_to(root), recursive=False)
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        archive_digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        manifest = dict(target=target, version=VERSION, binary=binary.relative_to(root).as_posix(),
                        sha256=digest, runtime_protocol=2, archive_size=archive.stat().st_size,
                        archive_sha256=archive_digest,
                        download_path=f"/api/runtimes/{target}/removed-{archive_digest}.tar.gz")
        if system == "darwin":
            details = subprocess.run(["codesign", "-dv", "--verbose=4", app], capture_output=True, text=True, check=True)
            team = re.search(r"^TeamIdentifier=([A-Z0-9]{10})$", details.stderr, re.MULTILINE)
            if team:
                manifest["signing_team"] = team.group(1)
        (output / f"{target}.json").write_text(json.dumps(manifest))
