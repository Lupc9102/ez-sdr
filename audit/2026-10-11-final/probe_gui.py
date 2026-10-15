#!/usr/bin/env python3
"""Launch the actual native binary on a private X server and accept JSON input.

Send {"click":[x,y],"name":"..."}, {"key":"Return"},
{"text":"..."}, or {"stop":true} on stdin.
Screenshots prove the real OpenGL/X11 surface, not physical SDR reception.
"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
AUDIT = Path(__file__).resolve().parent
os.sched_setaffinity(0, set(json.loads((AUDIT / "cpu-policy.json").read_text())["shared_audit_cpu_set"]))
os.nice(10)
directory = tempfile.TemporaryDirectory(prefix="ez-final-native-")
config = {"skip_antenna_checklists": True, "quick_start_completed": True, "mqtt_broker": "", "web_remote_enabled": False}
if os.environ.get("EZ_AUDIT_NATIVE_ADSB_REPLAY"):
    config.update(default_freq_hz=1090000000, default_sample_rate=2400000, observer_lat=52.257202, observer_lon=3.919373, source_preferences={"mode":"file","replay_file":os.environ["EZ_AUDIT_NATIVE_ADSB_REPLAY"],"replay_loop":True})
if os.environ.get("EZ_AUDIT_NATIVE_FIRST_RUN") != "1":
    Path(directory.name, "ez_sdr_config.json").write_text(json.dumps(config))
read_fd, write_fd = os.pipe()
xlog = (AUDIT / "evidence/native-xserver.log").open("w")
server = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1600x1000x24", "-nolisten", "tcp"], pass_fds=[write_fd], stdout=xlog, stderr=subprocess.STDOUT)
os.close(write_fd)
with os.fdopen(read_fd) as display_pipe:
    display = ":" + display_pipe.readline().strip()
assert display != ":", "Xvfb did not provide a display number"
env = dict(os.environ, DISPLAY=display, WINIT_UNIX_BACKEND="x11", LIBGL_ALWAYS_SOFTWARE="1", LP_NUM_THREADS="2")
env.pop("WAYLAND_DISPLAY", None)
log = (AUDIT / "evidence/native-gui.log").open("w")
binary = Path(os.environ.get("EZ_AUDIT_GUI_BINARY", ROOT / "target/debug/ez-gui"))
app = subprocess.Popen([str(binary)], cwd=directory.name, env=env, stdout=log, stderr=subprocess.STDOUT)
time.sleep(3)
x = ctypes.CDLL("libX11.so.6")
xt = ctypes.CDLL("libXtst.so.6")
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
x.XOpenDisplay.restype = ctypes.c_void_p
d = x.XOpenDisplay(display.encode())
assert d
x.XFlush.argtypes = [ctypes.c_void_p]
x.XStringToKeysym.argtypes = [ctypes.c_char_p]
x.XStringToKeysym.restype = ctypes.c_ulong
x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XKeysymToKeycode.restype = ctypes.c_uint
xt.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
xt.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xt.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
x.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
x.XResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_uint, ctypes.c_uint]
records = []
actions = []

def focus_window():
    tree = subprocess.check_output(["xwininfo", "-root", "-tree"], env=env, text=True)
    import re
    match = re.search(r"(0x[0-9a-f]+).*EZ-SDR", tree)
    if match:
        window = int(match.group(1), 16)
        x.XSetInputFocus(d, window, 2, 0)
        x.XFlush(d)
        return window
    raise RuntimeError("Native app window has not appeared yet")

def key(name):
    code = x.XKeysymToKeycode(d, x.XStringToKeysym(name.encode()))
    assert code, name
    xt.XTestFakeKeyEvent(d, code, 1, 0)
    xt.XTestFakeKeyEvent(d, code, 0, 0)
    x.XFlush(d)

def snapshot(name):
    path = AUDIT / "native" / (name + ".png")
    path.parent.mkdir(exist_ok=True)
    subprocess.run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-f", "x11grab", "-video_size", "1600x1000", "-i", display, "-frames:v", "1", "-threads", "1", str(path)], env=env, check=True, timeout=10)
    records.append({"name": name, "path": str(path.relative_to(ROOT)), "process_alive": app.poll() is None, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    print(json.dumps(records[-1]), flush=True)

try:
    snapshot(os.environ.get("EZ_AUDIT_NATIVE_STARTUP_NAME", "startup"))
    for line in sys.stdin:
        if not line.strip():
            continue
        action = json.loads(line)
        actions.append(action)
        if not action.get("stop"):
            window = focus_window()
        if "resize" in action:
            x.XResizeWindow(d, window, *action["resize"])
            x.XFlush(d)
        if action.get("stop"):
            break
        if "click" in action:
            px, py = action["click"]
            xt.XTestFakeMotionEvent(d, 0, px, py, 0)
            xt.XTestFakeButtonEvent(d, 1, 1, 0)
            xt.XTestFakeButtonEvent(d, 1, 0, 0)
            x.XFlush(d)
        if "key" in action:
            key(action["key"])
        if "chord" in action:
            codes = [x.XKeysymToKeycode(d, x.XStringToKeysym(name.encode())) for name in action["chord"]]
            for code in codes:
                xt.XTestFakeKeyEvent(d, code, 1, 0)
            for code in reversed(codes):
                xt.XTestFakeKeyEvent(d, code, 0, 0)
            x.XFlush(d)
        if "text" in action:
            for ch in action["text"]:
                code = x.XKeysymToKeycode(d, ord(ch))
                assert code, ch
                xt.XTestFakeKeyEvent(d, code, 1, 0)
                xt.XTestFakeKeyEvent(d, code, 0, 0)
                x.XFlush(d)
        time.sleep(action.get("wait", 0.5))
        if "name" in action:
            snapshot(action["name"])
finally:
    app.terminate()
    try:
        app.wait(timeout=5)
    except subprocess.TimeoutExpired:
        app.kill()
        app.wait()
    server.terminate()
    server.wait(timeout=5)
    log.close()
    xlog.close()
    directory.cleanup()
    (AUDIT / os.environ.get("EZ_AUDIT_NATIVE_MANIFEST", "native-gui.json")).write_text(json.dumps({"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "actions": actions, "surface": "native X11/OpenGL with software GL, private config", "screenshots": records}, indent=2) + "\n")
