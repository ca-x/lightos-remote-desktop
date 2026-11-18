#!/usr/bin/env python3
"""Publish the admitted private compositor to application activation services."""
import os
from pathlib import Path
import re
import socket
import stat
import struct
import subprocess
import sys

runtime, display = map(Path, sys.argv[1:3])
compositor = int(sys.argv[3])
uid = os.geteuid()
directory = runtime.lstat()
if (display.parent != runtime or not stat.S_ISDIR(directory.st_mode)
        or directory.st_uid != uid or directory.st_mode & 0o077):
    sys.exit("Wayland activation requires a private, same-user runtime directory")


def cgroup(pid):
    return Path(next(line[3:] for line in Path(f'/proc/{pid}/cgroup').read_text().splitlines() if line.startswith('0::')))


group = cgroup('self')
# A nested user manager moves its launcher into init.scope and starts services
# in sibling cgroups. PC launches can start inside a scope rather than a service.
if group.name == 'init.scope' and group.parent.name.endswith(('.service', '.scope')):
    group = group.parent
if group == Path('/'):
    sys.exit("Wayland activation requires an owned cgroup")


def owned_group(other):
    return other == group or group in other.parents


def peer(path):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(str(path))
        pid, owner, _ = struct.unpack('3i', connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
    if owner != uid or pid <= 1:
        sys.exit(f"Wayland activation socket {path} is not served by the session user")
    other = cgroup(pid)
    if not owned_group(other):
        sys.exit(f"Wayland activation socket {path} is outside the owned cgroup ({group} versus {other})")
    return pid


if peer(display) != compositor:
    sys.exit(f"Wayland activation socket {display} no longer matches the selected compositor")
environment = dict(item.split(b'=', 1) for item in Path(f'/proc/{compositor}/environ').read_bytes().split(b'\0') if b'=' in item)
address = environment[b'DBUS_SESSION_BUS_ADDRESS'].decode()
assert address.startswith('unix:path=') and ';' not in address and '\n' not in address
bus_path = Path(address[len('unix:path='):].split(',', 1)[0])
assert bus_path.is_absolute()
peer(bus_path)
values = {'WAYLAND_DISPLAY': str(display), 'DBUS_SESSION_BUS_ADDRESS': address, 'XDG_SESSION_TYPE': 'wayland', 'DISPLAY': '', 'XAUTHORITY': ''}
# Prefer a compositor-owned XWayland when it has already been started. Never
# republish the outer Xvfb DISPLAY as an inner desktop's application target.
children = set()
for task in Path(f'/proc/{compositor}/task').iterdir():
    try:
        children.update((task / 'children').read_text().split())
    except FileNotFoundError:
        pass
for child in children:
    process = Path('/proc') / child
    try:
        if (process / 'exe').readlink().name != 'Xwayland':
            continue
        assert process.stat().st_uid == uid and owned_group(cgroup(child))
        args = (process / 'cmdline').read_bytes().decode().split('\0')
        if len(args) < 2 or not re.fullmatch(r':\d+(?:\.\d+)?', args[1]):
            continue
        server = peer('/tmp/.X11-unix/X' + args[1][1:].split('.')[0])
        assert server in (compositor, int(child))
        authority = args[args.index('-auth') + 1] if '-auth' in args else ''
        if authority:
            assert Path(authority).parent == runtime and Path(authority).stat().st_uid == uid
        values.update(DISPLAY=args[1], XAUTHORITY=authority)
        break
    except (FileNotFoundError, PermissionError):
        # A setuid helper (for example a portal's mount helper) can be a child
        # of the compositor without exposing /proc/PID/exe to this user. It is
        # not an admitted XWayland target and must not abort the Wayland session.
        continue
entries = [key + '=' + value for key, value in values.items()]
subprocess.run(['/usr/bin/dbus-update-activation-environment', *entries], env={**os.environ, 'DBUS_SESSION_BUS_ADDRESS': address}, check=True)
subprocess.run(['/usr/bin/systemctl', '--user', 'set-environment', *entries], env={**os.environ, 'DBUS_SESSION_BUS_ADDRESS': 'unix:path=' + str(runtime / 'bus')}, stdout=subprocess.DEVNULL, check=True)
print(address)
