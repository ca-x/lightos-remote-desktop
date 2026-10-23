#!/usr/bin/env bash
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
command -v python3 >/dev/null || { echo 'x11-backend: python3 is required' >&2; exit 1; }

python3 - "$repo" <<'PY'
import json
import os
from pathlib import Path
import secrets
import select
import shutil
import subprocess
import sys
import tempfile
import time

repo = Path(sys.argv[1])


def executable(value, label):
    if value:
        path = Path(value)
        if not path.is_absolute() and '/' in value:
            path = repo / path
        if '/' in value:
            path = str(path)
        else:
            path = shutil.which(value)
    else:
        path = None
    if not path or not Path(path).is_file() or not os.access(path, os.X_OK):
        raise RuntimeError(f'{label} executable is unavailable: {value or "not found in PATH"}')
    # Keep executable symlinks: cargo's mise shim dispatches by argv[0].
    return str(Path(path).absolute())


xvfb = executable(os.environ.get('LRD_TEST_XVFB') or shutil.which('Xvfb'), 'Xvfb')
xauth = executable(shutil.which('xauth'), 'xauth')
cargo_options = ['--locked']
if os.environ.get('LRD_TEST_CARGO_FEATURES'):
    cargo_options += ['--features', os.environ['LRD_TEST_CARGO_FEATURES']]
worker_setting = os.environ.get('LRD_TEST_X11_WORKER')
if worker_setting:
    worker = executable(worker_setting, 'X11 worker')
else:
    cargo = executable(shutil.which('cargo'), 'cargo')
    subprocess.run([cargo, 'build', '--bin', 'lightos-remoted', *cargo_options], cwd=repo, check=True)
    metadata = json.loads(subprocess.check_output(
        [cargo, 'metadata', '--no-deps', '--format-version', '1'], cwd=repo))
    worker = executable(str(Path(metadata['target_directory']) / 'debug/lightos-remoted'), 'X11 worker')


def selected_display(read_fd, process, log):
    deadline = time.monotonic() + 8
    data = bytearray()
    while b'\n' not in data and len(data) < 32:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([read_fd], [], [], remaining)[0]:
            raise RuntimeError(f'Xvfb -displayfd timed out: {log.read_text(errors="replace")}')
        chunk = os.read(read_fd, 32 - len(data))
        if not chunk:
            raise RuntimeError(f'Xvfb exited before -displayfd: {log.read_text(errors="replace")}')
        data.extend(chunk)
    number = bytes(data).strip().decode('ascii')
    if not number.isdecimal() or process.poll() is not None:
        raise RuntimeError(f'invalid Xvfb -displayfd response {number!r}: {log.read_text(errors="replace")}')
    return ':' + number


with tempfile.TemporaryDirectory(prefix='lrd-x11-backend-') as temporary:
    temp = Path(temporary)
    children = []
    env = os.environ.copy()
    for name in ('DISPLAY', 'XAUTHORITY', 'WAYLAND_DISPLAY'):
        env.pop(name, None)
    env['LRD_X11_TEST_BINARY'] = worker
    try:
        for suffix, size in (('', '64x48x24'), ('_2', '80x60x24')):
            authority = temp / ('authority' + suffix)
            cookie = secrets.token_hex(16)
            # Xvfb reads -auth before -displayfd reveals its chosen number.
            # Seed the server with the cookie, then add the actual display for
            # xauth's client-side lookup. The sentinel is never used as DISPLAY.
            subprocess.run([xauth, '-f', str(authority), 'add', ':65535',
                            'MIT-MAGIC-COOKIE-1', cookie], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            authority.chmod(0o600)
            read_fd, write_fd = os.pipe()
            log = temp / ('xvfb' + suffix + '.log')
            with log.open('wb') as output:
                process = subprocess.Popen(
                    [xvfb, '-displayfd', str(write_fd), '-auth', str(authority),
                     '-screen', '0', size, '-nolisten', 'tcp', '-noreset'],
                    pass_fds=(write_fd,), stdout=output, stderr=subprocess.STDOUT)
            children.append((process, log))
            os.close(write_fd)
            try:
                display = selected_display(read_fd, process, log)
            finally:
                os.close(read_fd)
            subprocess.run([xauth, '-f', str(authority), 'add', display,
                            'MIT-MAGIC-COOKIE-1', cookie], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            number = display[1:]
            deadline = time.monotonic() + 5
            while not Path(f'/tmp/.X11-unix/X{number}').exists() and process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.02)
            if process.poll() is not None or not Path(f'/tmp/.X11-unix/X{number}').exists():
                raise RuntimeError(f'Xvfb {display} did not open its socket: {log.read_text(errors="replace")}')
            env['LRD_X11_TEST_DISPLAY' + suffix] = display
            env['LRD_X11_TEST_AUTH' + suffix] = str(authority)

        result = subprocess.run(
            [executable(shutil.which('cargo'), 'cargo'), 'test', '--lib', *cargo_options,
             'backend::x11::tests',
             '--', '--ignored', '--nocapture'], cwd=repo, env=env)
        if result.returncode:
            for _, log in children:
                print(f'{log.name}:\n{log.read_text(errors="replace")}', file=sys.stderr)
        sys.exit(result.returncode)
    finally:
        for process, _ in children:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
PY
