#!/usr/bin/env python3
"""Build all native hosts from this source tree and package the LPK runtime catalog."""
import argparse
import concurrent.futures
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import tarfile
import tomllib

repo = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--config", type=Path, default=repo / ".local/runtime-build.json")
parser.add_argument("--output", type=Path, default=repo / "content/runtimes")
args = parser.parse_args()
configuration = json.loads(args.config.read_text()) if args.config.exists() else {}
cache = repo / ".local/runtime-build"
cache.mkdir(parents=True, exist_ok=True)
native = cache / "native"
native.mkdir(exist_ok=True)
version = tomllib.loads((repo / "Cargo.toml").read_text())["package"]["version"]
triples = {"linux-x64": "x86_64-unknown-linux-gnu", "win32-x64": "x86_64-pc-windows-msvc", "darwin-arm64": "aarch64-apple-darwin", "darwin-x64": "x86_64-apple-darwin"}
sources = [repo / "Cargo.toml", repo / "Cargo.lock"]
sources += [repo / "scripts" / name for name in ["build-runtime-artifacts.py", "build-linux-runtime.py", "build-macos-runtimes.py"]]
sources += [repo / ".github/scripts/prepare-spa-build-sdk.sh"]
for name in ["src", "crates", "web/dist", ".cargo"]:
    sources += [p for p in (repo / name).rglob("*") if p.is_file() and "target" not in p.relative_to(repo / name).parts]
digest = hashlib.sha256()
for path in sorted(sources):
    digest.update(str(path.relative_to(repo)).encode()); digest.update(path.read_bytes())
fingerprint = digest.hexdigest()

def run(command, *, environment=None, cwd=repo, capture=False):
    return subprocess.run(command, cwd=cwd, env=environment, text=True, stdout=subprocess.PIPE if capture else None, check=True)

def cached(target):
    try:
        record = json.loads((native / target / ".build.json").read_text())
        binary = native / target / record["binary"]
        return record["fingerprint"] == fingerprint and record.get("identity", "-") == configuration.get("macos", {}).get("identity", "-") and hashlib.sha256(binary.read_bytes()).hexdigest() == record["sha256"]
    except (OSError, KeyError, ValueError): return False

def record(target):
    directory = native / target
    entry = "Lazycat Remote.app/Contents/MacOS/lightos-remoted" if target.startswith("darwin") else "lightos-remoted" + (".exe" if target.startswith("win32") else "")
    sha = hashlib.sha256((directory / entry).read_bytes()).hexdigest()
    (directory / ".build.json").write_text(json.dumps({"fingerprint": fingerprint, "binary": entry, "sha256": sha, "identity": configuration.get("macos", {}).get("identity", "-")}))

def build_linux():
    if cached("linux-x64"): return
    options = configuration.get("linux", {})
    command = options.get("command", ["python3", str(repo / "scripts/build-linux-runtime.py")])
    # A build command must return its actual output path; never consume PC repository binaries.
    result = run(command, capture=True)
    binary = Path(json.loads(result.stdout.strip().splitlines()[-1])["binary"])
    directory = native / "linux-x64"; directory.mkdir(exist_ok=True)
    shutil.copy2(binary, directory / "lightos-remoted")
    record("linux-x64")

def build_windows():
    if cached("win32-x64"): return
    environment = dict(os.environ)
    environment.update(TURBOJPEG_SOURCE="vendor", TURBOJPEG_STATIC="1", RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/NODEFAULTLIB:msvcrt -C link-arg=/NODEFAULTLIB:vcruntime -C link-arg=/DEFAULTLIB:libcmt -C link-arg=/DEFAULTLIB:libvcruntime")
    tools = configuration.get("windows", {}).get("compiler_directory")
    if tools:
        environment["PATH"] = str(Path(tools) / "bin") + os.pathsep + environment.get("PATH", "")
        environment["LD_LIBRARY_PATH"] = str(Path(tools) / "lib") + os.pathsep + environment.get("LD_LIBRARY_PATH", "")
    environment["CARGO_TARGET_DIR"] = str(cache / "windows-target")
    command = ["cargo", "build"] if platform.system() == "Windows" else ["cargo", "xwin", "build"]
    run(command + ["--release", "--locked", "--target", triples["win32-x64"], "--features", "bundled-jpeg", "--bin", "lightos-remoted"], environment=environment)
    directory = native / "win32-x64"; directory.mkdir(exist_ok=True)
    shutil.copy2(cache / "windows-target/x86_64-pc-windows-msvc/release/lightos-remoted.exe", directory / "lightos-remoted.exe")
    record("win32-x64")

def build_macos():
    if cached("darwin-arm64") and cached("darwin-x64"): return
    options = configuration.get("macos", {})
    identity = options.get("identity", "-")
    if platform.system() == "Darwin":
        run(["python3", str(repo / "scripts/build-macos-runtimes.py"), "--source", str(repo), "--output", str(native), "--version", version, "--identity", identity])
    else:
        host = options.get("host")
        if not host: raise RuntimeError("All-platform LPK builds require macos.host in .local/runtime-build.json (Swift and the macOS SDK need a native Mac worker)")
        port = str(options.get("port", 22))
        ssh_options = options.get("ssh_options", [])
        remote = "/tmp/lazycat-remote-lpk-build/" + fingerprint[:20]
        ssh = ["ssh", *ssh_options, "-p", port, host]
        run(ssh + ["mkdir -p " + shlex.quote(remote)])
        source_archive = cache / "source.tar.gz"
        with tarfile.open(source_archive, "w:gz") as archive:
            for name in ["Cargo.toml", "Cargo.lock", "src", "crates", "web/dist", "scripts/build-macos-runtimes.py"]:
                archive.add(repo / name, arcname=name, filter=lambda item: None if "target" in Path(item.name).parts else item)
        run(["scp", *ssh_options, "-P", port, str(source_archive), host + ":" + remote + "/source.tar.gz"])
        body = "set -eu; cd " + shlex.quote(remote) + "; tar -xzf source.tar.gz; python3 scripts/build-macos-runtimes.py --source . --output artifacts --version " + shlex.quote(version) + " --identity " + shlex.quote(identity)
        run(ssh + [body])
        for target in ["darwin-arm64", "darwin-x64"]:
            if (native / target).exists(): shutil.rmtree(native / target)
            run(["scp", *ssh_options, "-r", "-P", port, host + ":" + remote + "/artifacts/" + target, str(native)])
    record("darwin-arm64"); record("darwin-x64")

with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
    futures = [pool.submit(build_linux), pool.submit(build_windows), pool.submit(build_macos)]
    for future in futures: future.result()

args.output.mkdir(parents=True, exist_ok=True)
index = {"version": version, "source_fingerprint": fingerprint, "targets": {}}
for target, triple in triples.items():
    directory = native / target
    build = json.loads((directory / ".build.json").read_text())
    artifact = {"target": target, "version": version, "binary": build["binary"], "sha256": build["sha256"], "runtime_protocol": 2, "source_fingerprint": fingerprint}
    team = directory / ".signing-team"
    if team.exists(): artifact["signing_team"] = team.read_text().strip()
    notices = cache / (target + "-NOTICE.txt")
    run(["python3", str(repo / "scripts/mcp-notices.py"), str(notices), "--target", triple, "--features", "bundled-jpeg,rdp,portal" if target.startswith("linux") else "bundled-jpeg"])
    files = {str(p.relative_to(directory)): p for p in directory.rglob("*") if p.is_file() and not p.name.startswith(".")}
    files.update({"LICENSE": repo / "LICENSE", "NOTICE.frontend.txt": repo / "web/dist/NOTICE.frontend.txt", "NOTICE.runtime.txt": notices})
    archive_path = args.output / (target + ".tar.gz")
    with archive_path.open("wb") as raw, gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0, compresslevel=6) as compressed, tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
        manifest = json.dumps(artifact, sort_keys=True).encode()
        info = tarfile.TarInfo("manifest.json"); info.size = len(manifest); info.mode = 0o644
        archive.addfile(info, io.BytesIO(manifest))
        for name, path in sorted(files.items()):
            info = tarfile.TarInfo(name); info.size = path.stat().st_size; info.mode = 0o755 if name == artifact["binary"] else 0o644
            with path.open("rb") as content: archive.addfile(info, content)
    artifact.update(archive_sha256=hashlib.sha256(archive_path.read_bytes()).hexdigest(), archive_size=archive_path.stat().st_size,
        download_path=f"/api/runtimes/{target}/{archive_path.stat().st_size}-{artifact['sha256'][:12]}.tar.gz")
    index["targets"][target] = artifact
    print(f"{target}: {version}, {artifact['archive_size']} compressed bytes", flush=True)
(args.output / "index.json").write_text(json.dumps(index, indent=2) + "\n")
