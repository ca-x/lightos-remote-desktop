#!/usr/bin/env python3
"""Build the complete Linux host against the supported Ubuntu 22.04 ABI."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import urllib.request
from runtime_codec import sdk

repo = Path(__file__).resolve().parent.parent
subprocess.run(["cargo", "fetch", "--locked", "--target", "x86_64-unknown-linux-gnu"], cwd=repo, check=True, stdout=sys.stderr)
cache = Path(os.environ.get("REMOTE_DESKTOP_LINUX_BUILD_CACHE", Path.home() / ".cache/lazycat-remote-linux-ubuntu2204"))
cache.mkdir(parents=True, exist_ok=True)
ffmpeg = sdk("linux-x64")
archive = cache / "ubuntu-base-22.04.5-base-amd64.tar.gz"
checksum = "242cd8898b33ea806ef5f13b1076ed7c76f9f989d18384452f7166692438ff1a"
if not shutil.which("bwrap"):
    raise SystemExit("Linux distribution builds need bubblewrap and an x86_64 Linux host, or a configured Linux build command")
if not archive.exists():
    with urllib.request.urlopen("https://cdimage.ubuntu.com/ubuntu-base/releases/22.04/release/" + archive.name, timeout=60) as response, archive.open("wb") as output:
        shutil.copyfileobj(response, output)
if hashlib.sha256(archive.read_bytes()).hexdigest() != checksum:
    raise SystemExit("Ubuntu build root checksum mismatch")
root = cache / "rootfs"
root.mkdir(exist_ok=True)
if not (root / "usr/bin/apt-get").exists():
    subprocess.run(["bsdtar", "-xf", str(archive), "-C", str(root), "--no-same-owner"], check=True)
(root / "usr/sbin/policy-rc.d").write_text("#!/bin/sh\nexit 101\n")
(root / "usr/sbin/policy-rc.d").chmod(0o755)
target, logs = cache / "target", cache / "logs"
target.mkdir(exist_ok=True)
logs.mkdir(exist_ok=True)
toolchain = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
command = ["bwrap", "--unshare-user", "--uid", "0", "--gid", "0", "--unshare-pid", "--unshare-ipc", "--unshare-uts", "--die-with-parent", "--bind", str(root), "/",
    "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--ro-bind", "/etc/resolv.conf", "/etc/resolv.conf", "--ro-bind", str(repo), "/source",
    "--ro-bind", toolchain, "/toolchain", "--ro-bind", str(ffmpeg), "/codec-sdk", "--bind", str(Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))), "/cargo",
    "--bind", str(target), "/build-target", "--bind", str(logs), "/build-logs", "--chdir", "/source", "--clearenv"]
for key, value in {"PATH": "/toolchain/bin:/usr/bin:/bin", "DEBIAN_FRONTEND": "noninteractive", "CARGO_HOME": "/cargo", "RUSTC": "/toolchain/bin/rustc",
    "CARGO_TARGET_DIR": "/build-target", "CARGO_NET_OFFLINE": "true", "FFMPEG_DIR": "/codec-sdk", "LD_LIBRARY_PATH": "/codec-sdk/lib", "TURBOJPEG_SOURCE": "vendor", "TURBOJPEG_STATIC": "1"}.items():
    command += ["--setenv", key, value]
setup = ""
if not (root / "usr/bin/gcc").exists() or not (root / "usr/include/pipewire-0.3/pipewire/pipewire.h").exists() or not (root / "usr/include/X11/Xlib.h").exists() or not (root / "usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml").exists():
    setup = "apt-get -o APT::Sandbox::User=root update > /build-logs/setup.log 2>&1 && apt-get -o APT::Sandbox::User=root install -y --no-install-recommends build-essential libwayland-dev libxkbcommon-dev libpulse-dev libpipewire-0.3-dev libspa-0.2-dev libclang-dev cmake nasm pkg-config ca-certificates curl python3 libx11-dev wayland-protocols libwayland-bin >> /build-logs/setup.log 2>&1 && "
command += ["/bin/sh", "-c", setup + "GITHUB_ENV=/build-logs/spa.env bash .github/scripts/prepare-spa-build-sdk.sh /build-target/spa-sdk > /build-logs/spa.log 2>&1 && set -a && . /build-logs/spa.env && set +a && /toolchain/bin/cargo build --release --locked --features bundled-jpeg,rdp,portal --bin lightos-remoted > /build-logs/build.log 2>&1 && RUSTFLAGS='-C link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN/lib' /toolchain/bin/cargo build --release --locked -p media-codec-ffmpeg --features native,media-codec/bundled-jpeg > /build-logs/codec.log 2>&1 && python3 scripts/build-viewer-input.py --target linux-x64 --output /build-target/release/lightos-input-capture > /build-logs/input.log 2>&1"]
subprocess.run(command, check=True)
print(json.dumps({"binary": str(target / "release/lightos-remoted"), "worker": str(target / "release/lightos-codec-ffmpeg"), "ffmpeg": str(ffmpeg), "input": str(target / "release/lightos-input-capture")}))
