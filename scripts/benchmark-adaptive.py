#!/usr/bin/env python3
"""Compare explicit binaries on a private desktop and a shaped TCP stream.

Requires a built Rust test (cargo test --test adaptive_stream --no-run), local
Hyprland/Omarchy, ffplay/ffmpeg and Aquamarine compatibility already installed.
No sudo, host network changes, or production-session attachment. Results are
synthetic packet ACKs, not browser render/input-to-photon measurements.
"""
import argparse
import asyncio
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_for(check, child, timeout=35):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if child.poll() is not None:
            raise RuntimeError(f"process {child.args[0]} exited with {child.returncode}")
        try:
            if check():
                return
        except (OSError, ValueError):
            pass
        time.sleep(.1)
    raise TimeoutError("private benchmark session readiness timed out")


def health(server_port):
    return urllib.request.urlopen(f"http://127.0.0.1:{server_port}/healthz", timeout=1).status == 200


def cpu_seconds(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


async def benchmark(binary, owner_runtime, compat, output, strategy, cap_bytes):
    server_port, proxy_port = port(), port()
    logs = output.with_suffix(".server.log").open("w")
    child = subprocess.Popen([str(binary), "serve", "--password", "adaptive-benchmark-test",
                            "--listen", "127.0.0.1", "--port", str(server_port),
                            "--session-runtime", str(owner_runtime), "--audio", "false",
                            "--aquamarine-dir", str(compat)], stdout=logs, stderr=logs)
    async def proxy(reader, writer):
        upstream_reader, upstream_writer = await asyncio.open_connection("127.0.0.1", server_port)
        started = time.monotonic()
        async def pump(source, target, shaped):
            try:
                while data := await source.read(2048):
                    if shaped:
                        elapsed = time.monotonic() - started
                        # Phase timing starts at the WS handshake; client t=0 is
                        # after ready, normally only a few milliseconds later.
                        if 24 <= elapsed < 28:
                            await asyncio.sleep(28 - elapsed)
                        if 4 <= elapsed < 16:
                            await asyncio.sleep(len(data) / cap_bytes)
                    target.write(data)
                    await target.drain()
            except (ConnectionError, BrokenPipeError, asyncio.CancelledError):
                pass
            finally:
                target.close()
        tasks = [asyncio.create_task(pump(reader, upstream_writer, False)),
                 asyncio.create_task(pump(upstream_reader, writer, True))]
        await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    try:
        await asyncio.to_thread(wait_for, lambda: health(server_port), child)
        server = await asyncio.start_server(proxy, "127.0.0.1", proxy_port)
        before, started = cpu_seconds(child.pid), time.monotonic()
        async with server:
            env = {**os.environ, "LIGHTOS_ADAPTIVE_ADDRESS": f"127.0.0.1:{proxy_port}",
                   "LIGHTOS_ADAPTIVE_RESULT": str(output), "LIGHTOS_ADAPTIVE_STRATEGY": strategy}
            test = await asyncio.create_subprocess_exec("cargo", "test", "--locked", "--test", "adaptive_stream", "--", "--ignored", "--nocapture", cwd=ROOT, env=env)
            await asyncio.wait_for(test.wait(), timeout=65)
            if test.returncode:
                raise RuntimeError("link test failed; inspect the result/server log")
        result = json.loads(output.read_text())
        elapsed = time.monotonic() - started
        result["server_cpu_percent"] = (cpu_seconds(child.pid) - before) / elapsed * 100
        result["binary"] = str(binary)
        result["downstream_cap_bytes_per_second"] = cap_bytes
        output.write_text(json.dumps(result, indent=2) + "\n")
        print(f"RESULT {output}: server CPU {result['server_cpu_percent']:.1f}%", flush=True)
    finally:
        child.terminate()
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
        logs.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", action="append", required=True, help="label=/absolute/binary; repeat for A/B")
    parser.add_argument("--strategy", default="auto", choices=["auto", "quality_first", "low_latency", "smooth"])
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--cap-bytes", type=int, default=96*1024)
    args = parser.parse_args()
    if args.cap_bytes <= 0:
        parser.error("--cap-bytes must be positive")
    for executable in ["Hyprland", "cage", "ghostty", "ffplay", "ffmpeg", "cargo"]:
        if not shutil.which(executable):
            raise RuntimeError(f"missing benchmark prerequisite: {executable}")
    binaries = [(label, Path(path).resolve()) for label, path in (v.split("=", 1) for v in args.binary)]
    if any(not path.is_file() or not os.access(path, os.X_OK) for _, path in binaries):
        raise RuntimeError("benchmark binary must exist and be executable")
    compat = next((p.parent for p in Path("/usr/local/lib/lightos-remote-desktop/compat").glob("*/libaquamarine.so.14")), None)
    if compat is None:
        raise RuntimeError("installed private Aquamarine compatibility library is missing")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    # Owner lifecycle is separate; both transports observe exactly this desktop.
    with tempfile.TemporaryDirectory(prefix="lrd.q.") as temp:
        runtime = Path(temp)
        owner_log = (args.output_dir / "owner.log").open("w")
        owner = subprocess.Popen([str(binaries[0][1]), "desktop-session", "--password", "adaptive-benchmark-test", "--desktop-shell", "omarchy", "--audio", "false", "--runtime-dir", str(runtime), "--width", "1280", "--height", "720", "--aquamarine-dir", str(compat)], stdout=owner_log, stderr=owner_log, start_new_session=True)
        movie = None
        try:
            wait_for(lambda: (runtime / "session.json").is_file(), owner)
            descriptor = json.loads((runtime / "session.json").read_text())
            movie_env = {**os.environ, "XDG_RUNTIME_DIR": str(runtime), "WAYLAND_DISPLAY": descriptor["socket"], "SDL_VIDEODRIVER": "wayland", "SDL_AUDIODRIVER": "dummy"}
            movie_env.pop("DISPLAY", None)
            movie_log = (args.output_dir / "movie.log").open("w")
            movie = subprocess.Popen(["ffplay", "-loglevel", "error", "-an", "-fs", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30"], env=movie_env, stdout=movie_log, stderr=movie_log)
            time.sleep(2)
            if movie.poll() is not None:
                raise RuntimeError("moving workload did not start; inspect movie.log")
            for label, binary in binaries:
                if not label.replace("-", "").replace("_", "").isalnum():
                    raise ValueError("binary labels must contain only letters/numbers/dash/underscore")
                asyncio.run(benchmark(binary, runtime, compat, args.output_dir / f"{label}-{args.strategy}.json", args.strategy, args.cap_bytes))
        finally:
            if movie:
                movie.terminate()
                try:
                    movie.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    movie.kill()
                    movie.wait()
            owner.terminate()
            try:
                owner.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(owner.pid, signal.SIGKILL)
                owner.wait()
            owner_log.close()


if __name__ == "__main__":
    main()
