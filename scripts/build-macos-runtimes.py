#!/usr/bin/env python3
"""Native macOS worker: compile both CPUs and sign stable application bundles."""
import argparse
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--source", required=True, type=Path)
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--version", required=True)
parser.add_argument("--identity", default="-")
args = parser.parse_args()
args.source = args.source.resolve()
args.output = args.output.resolve()
environment = dict(os.environ)
target_directory = args.source.parent / "target-cache"
environment["CARGO_TARGET_DIR"] = str(target_directory)
environment.update(PATH=str(Path.home() / ".cargo/bin") + ":/opt/homebrew/bin:" + environment.get("PATH", ""), TURBOJPEG_SOURCE="vendor", TURBOJPEG_STATIC="1", MACOSX_DEPLOYMENT_TARGET="13.0", CARGO_PROFILE_RELEASE_STRIP="none", CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_DEBUG="2")
for target, triple in [("darwin-arm64", "aarch64-apple-darwin"), ("darwin-x64", "x86_64-apple-darwin")]:
    subprocess.run(["cargo", "build", "--release", "--locked", "--target", triple, "--features", "bundled-jpeg", "--bin", "lightos-remoted"], cwd=args.source, env=environment, check=True)
    directory = args.output / target
    if directory.exists(): shutil.rmtree(directory)
    app = directory / "Lazycat Remote.app"
    contents = app / "Contents"
    binary = contents / "MacOS/lightos-remoted"
    binary.parent.mkdir(parents=True)
    shutil.copy2(target_directory / triple / "release/lightos-remoted", binary)
    subprocess.run(["python3", str(args.source / "scripts/build-viewer-input.py"), "--target", target, "--output", str(binary.with_name("lightos-input-capture")), "--identity", args.identity], cwd=args.source, env=environment, check=True)
    shutil.move(str(binary.with_name("NOTICE.input.txt")), str(directory / "NOTICE.input.txt"))
    binary.chmod(0o755)
    (contents / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "org.lightos.remote-desktop", "CFBundleName": "Lazycat Remote", "CFBundleDisplayName": "懒猫远程", "CFBundleExecutable": "lightos-remoted", "CFBundlePackageType": "APPL", "CFBundleShortVersionString": args.version, "CFBundleVersion": args.version, "LSMinimumSystemVersion": "13.0", "LSUIElement": True, "NSHighResolutionCapable": True}))
    subprocess.run(["python3", str(args.source / "scripts/build-macos-codec.py"), "--source", str(args.source), "--output", str(directory), "--target", triple, "--identity", args.identity], cwd=args.source, env=environment, check=True)
    subprocess.run(["codesign", "--force", "--sign", args.identity, "--options", "runtime", "--timestamp=none", str(app)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    details = subprocess.run(["codesign", "-dv", "--verbose=4", str(app)], capture_output=True, text=True, check=True).stderr
    team = re.search(r"^TeamIdentifier=([A-Z0-9]{10})$", details, re.MULTILINE)
    if team: (directory / ".signing-team").write_text(team.group(1))
