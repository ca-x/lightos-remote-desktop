#!/usr/bin/env python3
"""Package a native host built by CI without activating desktop services."""
import argparse
import hashlib
import os
import plistlib
from pathlib import Path
import shutil
import tarfile
import tomllib
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--notices", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    version = tomllib.loads((repo / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    arch = args.target.split("-")[0]
    if args.target.endswith("windows-msvc"):
        system, suffix = "windows", ".exe"
    elif args.target.endswith("apple-darwin"):
        system, suffix = "macos", ""
    else:
        parser.error("expected a Windows MSVC or macOS target")
    name = f"lightos-remoted-{system}-{arch}"
    staging = repo / "dist" / name
    staging.mkdir(parents=True, exist_ok=False)
    source = repo / "target" / args.target / "release" / f"lightos-remoted{suffix}"
    if system == "macos":
        contents = staging / "LightOS Remote Desktop.app/Contents"
        executable = contents / "MacOS/lightos-remoted"
        executable.parent.mkdir(parents=True)
        (contents / "Info.plist").write_bytes(plistlib.dumps({
            "CFBundleIdentifier": "org.lightos.remote-desktop",
            "CFBundleName": "LightOS Remote Desktop",
            "CFBundleExecutable": "lightos-remoted",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version,
            "LSMinimumSystemVersion": "13.0",
            "LSUIElement": True,
            "NSHighResolutionCapable": True,
        }))
    else:
        executable = staging / "lightos-remoted.exe"
    shutil.copy2(source, executable)
    executable.chmod(0o755)
    for filename in ["LICENSE", "README.md", "README.en.md"]:
        shutil.copy2(repo / filename, staging / filename)
    shutil.copytree(repo / "docs", staging / "docs")
    licenses = staging / "licenses"
    licenses.mkdir()
    shutil.copy2(repo / "docs/licenses/lightos-remote-desktop-mit.txt", licenses / "lightos-remote-desktop-mit.txt")
    shutil.copy2(args.notices, licenses / "native-runtime.txt")
    shutil.copy2(repo / "web/dist/NOTICE.frontend.txt", licenses / "frontend.txt")
    # These bundled C libraries include notices below the crate's top level.
    registry = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")) / "registry/src"
    sources = {
        "libjpeg-turbo.txt": "*/turbojpeg-sys-*/libjpeg-turbo/LICENSE.md",
        "openh264.txt": "*/openh264-sys2-*/upstream/LICENSE",
        "opus.txt": "*/opusic-sys-*/opus/COPYING",
        "turbojpeg.txt": "*/turbojpeg-*/LICENSE",
    }
    for name, pattern in sources.items():
        matches = list(registry.glob(pattern))
        if not matches:
            raise SystemExit(f"Missing bundled library notice: {name}")
        shutil.copy2(matches[0], licenses / name)
    shutil.copy2(repo / "docs/licenses/openh264-rust.txt", licenses / "openh264-rust.txt")
    if system == "windows":
        archive = staging.with_suffix(".zip")
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
            for path in staging.rglob("*"):
                if path.is_file():
                    output.write(path, path.relative_to(staging.parent))
    else:
        archive = staging.with_suffix(".tar.gz")
        with tarfile.open(archive, "w:gz") as output:
            output.add(staging, arcname=staging.name)
    digest = hashlib.file_digest(archive.open("rb"), "sha256").hexdigest()
    Path(str(archive) + ".sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    print(f"Packaged {archive.name} ({version})")


if __name__ == "__main__":
    main()
