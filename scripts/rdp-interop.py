#!/usr/bin/env python3
"""Opt-in FreeRDP TCP/TLS interoperability check on a private Xvfb display.

Called by the ignored Rust test with an ephemeral certificate and synthetic
frame/input backend. No installed packages, live desktop, or host settings change.
"""
import argparse
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def until(fn, seconds=12):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = fn()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError("timed out waiting for FreeRDP/X11 result")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--freerdp", required=True)
    parser.add_argument("--xvfb", required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--fingerprint", required=True)
    args = parser.parse_args()
    for executable in (args.freerdp, args.xvfb):
        assert Path(executable).is_file(), executable
    x11 = ctypes.CDLL(ctypes.util.find_library("X11"))
    xtest = ctypes.CDLL(ctypes.util.find_library("Xtst"))
    ptr, ul, ui, integer = ctypes.c_void_p, ctypes.c_ulong, ctypes.c_uint, ctypes.c_int
    def bind(lib, name, restype, *types):
        f = getattr(lib, name)
        f.restype, f.argtypes = restype, list(types)
        return f
    open_display = bind(x11, "XOpenDisplay", ptr, ctypes.c_char_p)
    close_display = bind(x11, "XCloseDisplay", integer, ptr)
    root_window = bind(x11, "XDefaultRootWindow", ul, ptr)
    query_tree = bind(x11, "XQueryTree", integer, ptr, ul, ctypes.POINTER(ul), ctypes.POINTER(ul), ctypes.POINTER(ctypes.POINTER(ul)), ctypes.POINTER(ui))
    fetch_name = bind(x11, "XFetchName", integer, ptr, ul, ctypes.POINTER(ctypes.c_char_p))
    free = bind(x11, "XFree", integer, ptr)
    geometry = bind(x11, "XGetGeometry", integer, ptr, ul, ctypes.POINTER(ul), ctypes.POINTER(integer), ctypes.POINTER(integer), ctypes.POINTER(ui), ctypes.POINTER(ui), ctypes.POINTER(ui), ctypes.POINTER(ui))
    get_image = bind(x11, "XGetImage", ptr, ptr, ul, integer, integer, ui, ui, ul, integer)
    get_pixel = bind(x11, "XGetPixel", ul, ptr, integer, integer)
    destroy_image = bind(x11, "XDestroyImage", integer, ptr)
    focus = bind(x11, "XSetInputFocus", integer, ptr, ul, integer, ul)
    keysym = bind(x11, "XKeysymToKeycode", ctypes.c_ubyte, ptr, ul)
    sync = bind(x11, "XSync", integer, ptr, integer)
    key = bind(xtest, "XTestFakeKeyEvent", integer, ptr, ui, integer, ul)
    button = bind(xtest, "XTestFakeButtonEvent", integer, ptr, ui, integer, ul)
    move = bind(xtest, "XTestFakeMotionEvent", integer, ptr, integer, integer, integer, ul)

    with tempfile.TemporaryDirectory(prefix="lrd-rdp-client-") as temp:
        env = os.environ.copy()
        read_fd, write_fd = os.pipe()
        log_path = Path(temp) / "xvfb.log"
        clients = []
        display = None
        with log_path.open("wb") as xvfb_log:
            xvfb = subprocess.Popen([args.xvfb, "-displayfd", str(write_fd), "-screen", "0", "1024x768x24", "-nolisten", "tcp", "-ac", "-noreset"], pass_fds=(write_fd,), stdout=xvfb_log, stderr=xvfb_log)
        os.close(write_fd)
        try:
            # Xvfb writes the selected, unoccupied display number when ready.
            import select
            assert select.select([read_fd], [], [], 8)[0], log_path.read_text()
            number = os.read(read_fd, 32).decode().strip()
            assert number.isdigit(), number
            env["DISPLAY"] = ":" + number
            env.pop("WAYLAND_DISPLAY", None)
            display = until(lambda: open_display(env["DISPLAY"].encode()), 5)
            root = root_window(display)
            common = [args.freerdp, f"/v:127.0.0.1:{args.port}", "/u:lightos", "/sec:tls", "/size:320x200", "/bpp:32", "/t:lrd-rdp-interop", "-decorations", "-clipboard", "-auto-reconnect", "/log-level:WARN"]
            certificate = "/cert:fingerprint:sha256:" + args.fingerprint
            def denied(extra):
                result = subprocess.run(common + extra, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10)
                assert result.returncode != 0, result.stdout.decode(errors="replace")
                return result.returncode
            failures = {
                "untrusted_certificate": denied(["/p:rdp-test-password", "/cert:deny"]),
                "wrong_password": denied(["/p:incorrect-password", certificate]),
                "empty_password": denied(["/p:", certificate]),
                "plaintext_downgrade": denied(["/p:rdp-test-password", certificate, "/sec:rdp"]),
            }
            def windows():
                r, parent, children, count = ul(), ul(), ctypes.POINTER(ul)(), ui()
                query_tree(display, root, ctypes.byref(r), ctypes.byref(parent), ctypes.byref(children), ctypes.byref(count))
                found = []
                try:
                    for i in range(count.value):
                        name = ctypes.c_char_p()
                        if fetch_name(display, children[i], ctypes.byref(name)) and name.value:
                            if b"lrd-rdp-interop" in name.value:
                                found.append(children[i])
                            free(ctypes.cast(name, ptr))
                finally:
                    if children:
                        free(children)
                return found
            last_sample = [None]
            def sample(window, expected):
                r, x, y, w, h, border, depth = ul(), integer(), integer(), ui(), ui(), ui(), ui()
                if not geometry(display, window, ctypes.byref(r), ctypes.byref(x), ctypes.byref(y), ctypes.byref(w), ctypes.byref(h), ctypes.byref(border), ctypes.byref(depth)):
                    return False
                image = get_image(display, window, 0, 0, w.value, h.value, ul(-1).value, 2)
                if not image:
                    return False
                try:
                    pixel = get_pixel(image, min(100, w.value - 1), min(80, h.value - 1)) & 0xffffff
                finally:
                    destroy_image(image)
                observed = (w.value, h.value, hex(pixel))
                if observed != last_sample[0]:
                    print("observed", observed, "expected", hex(expected), flush=True)
                    last_sample[0] = observed
                if pixel != expected:
                    return False
                return (w.value, h.value)
            def connect():
                output = open(Path(temp) / f"client-{len(clients)}.log", "wb")
                process = subprocess.Popen(common + ["/p:rdp-test-password", certificate], env=env, stdout=output, stderr=output)
                output.close()
                clients.append(process)
                window = until(lambda: windows())[-1]
                assert process.poll() is None
                return process, window
            client, window = connect()
            initial = until(lambda: sample(window, 0xe6283c))
            assert initial == (320, 200), initial
            focus(display, window, 1, 0)
            move(display, -1, 100, 50, 0)
            button(display, 1, 1, 0)
            button(display, 1, 0, 0)
            a = keysym(display, ord("a"))
            key(display, a, 1, 0)
            key(display, a, 0, 0)
            sync(display, 0)
            resized = until(lambda: sample(window, 0x1e50d2))
            assert resized == (640, 480), resized
            client.terminate()
            client.wait(timeout=5)
            until(lambda: not windows(), 5)
            client, window = connect()
            reconnect = until(lambda: sample(window, 0x1e50d2))
            assert reconnect == (640, 480), reconnect
            client.terminate()
            client.wait(timeout=5)
            print(json.dumps({"client": "FreeRDP", "tls_certificate_pinned": True, "rejected": failures, "bitmap": initial, "input_and_resize": resized, "reconnect": reconnect}))
        except BaseException:
            for log in Path(temp).glob("client-*.log"):
                print(log.name, log.read_text(errors="replace"), flush=True)
            raise
        finally:
            if display:
                close_display(display)
            for process in clients:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)
            xvfb.terminate()
            xvfb.wait(timeout=5)
            os.close(read_fd)


if __name__ == "__main__":
    main()
