"""Windows native overlay probe: real pointer routing and screen composition.
Build desktop-overlay-probe first. This tool briefly moves the pointer onto its own windows.
"""
import ctypes as c
import json
from pathlib import Path
import queue
import subprocess
import threading
import time

u, g = c.windll.user32, c.windll.gdi32
u.SetProcessDpiAwarenessContext.argtypes = [c.c_void_p]
u.SetProcessDpiAwarenessContext(c.c_void_p(-4))
u.FindWindowW.argtypes = [c.c_wchar_p, c.c_wchar_p]
u.FindWindowW.restype = c.c_void_p
u.GetForegroundWindow.restype = c.c_void_p
u.GetDC.restype = c.c_void_p
u.ReleaseDC.argtypes = [c.c_void_p, c.c_void_p]
u.GetWindowRect.argtypes = [c.c_void_p, c.c_void_p]
u.GetWindowLongW.argtypes = [c.c_void_p, c.c_int]
u.SetForegroundWindow.argtypes = [c.c_void_p]
g.GetPixel.argtypes = [c.c_void_p, c.c_int, c.c_int]
g.GetPixel.restype = c.c_uint
class Rect(c.Structure):
    _fields_ = [(n, c.c_long) for n in ("left", "top", "right", "bottom")]
class Point(c.Structure):
    _fields_ = [(n, c.c_long) for n in ("x", "y")]
original_pointer = Point(); u.GetCursorPos(c.byref(original_pointer))
lines, inbox = [], queue.Queue()
process = subprocess.Popen(["target/debug/examples/desktop-overlay-probe.exe"], stdin=subprocess.PIPE,
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, creationflags=subprocess.CREATE_NO_WINDOW)
def read_output():
    for line in process.stdout:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        lines.append(event); inbox.put(event)
threading.Thread(target=read_output, daemon=True).start()
def wait_for(kind, timeout=35, **fields):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            line = inbox.get(timeout=.1)
            if line.get("event") == kind and all(line.get(k) == v for k, v in fields.items()): return line
        except queue.Empty: pass
    raise AssertionError(f"Timed out awaiting {kind} {fields}: {lines}")
def command(value):
    process.stdin.write(value + "\n"); process.stdin.flush()
def pixel(point):
    dc = u.GetDC(None)
    try: return g.GetPixel(dc, *point)
    finally: u.ReleaseDC(None, dc)
def click(point):
    assert u.SetCursorPos(*point)
    time.sleep(.1)
    actual = Point(); u.GetCursorPos(c.byref(actual))
    assert (actual.x, actual.y) == point, ((actual.x, actual.y), point)
    u.mouse_event(2, 0, 0, 0, 0); u.mouse_event(4, 0, 0, 0, 0)
try:
    wait_for("passthrough", window=99, success=False)
    command("fail"); wait_for("open_failed", window=2)
    time.sleep(1)
    base = u.FindWindowW(None, "NanaUI overlay probe base")
    layer = u.FindWindowW(None, "NanaUI overlay probe layer")
    assert base and layer
    initial_focus = u.GetForegroundWindow()
    assert initial_focus != layer, "Overlay stole focus on creation"
    rect = Rect(); assert u.GetWindowRect(layer, c.byref(rect))
    point = (rect.left + 160, rect.top + 100)
    clear_point = (rect.left + 200, rect.top + 200)
    before = pixel(clear_point)
    assert before == 0x00FF00, f"Transparent layer obscures green reference: {before:#x}"
    click(point); wait_for("pointer_down", 5, window=1)
    command("lock"); wait_for("passthrough", window=1, enabled=True, success=True)
    time.sleep(.3)
    assert u.GetWindowLongW(layer, -20) & 0x20, "Native WS_EX_TRANSPARENT absent"
    click(point); wait_for("pointer_down", 5, window=0)
    locked = pixel(clear_point)
    command("unlock"); wait_for("passthrough", window=1, enabled=False, success=True)
    time.sleep(.3)
    assert not u.GetWindowLongW(layer, -20) & 0x20, "Native passthrough was not cleared"
    click(point); wait_for("pointer_down", 5, window=1)
    before_resize = Rect(); assert u.GetWindowRect(layer, c.byref(before_resize))
    edge = (before_resize.right - 3, before_resize.bottom - 3)
    assert u.SetCursorPos(*edge); time.sleep(.1); u.mouse_event(2, 0, 0, 0, 0)
    time.sleep(.1)
    for delta in [15, 30, 45]:
        assert u.SetCursorPos(edge[0] + delta, edge[1] + delta); time.sleep(.15)
    u.mouse_event(4, 0, 0, 0, 0); time.sleep(.3)
    after_resize = Rect(); assert u.GetWindowRect(layer, c.byref(after_resize))
    assert after_resize.right > before_resize.right + 20 and after_resize.bottom > before_resize.bottom + 20, "Native resize did not track pointer"
    command("close"); wait_for("closed", window=1); time.sleep(.3)
    after = pixel(clear_point)
    assert before == locked == after, (before, locked, after)
    command("quit"); assert process.wait(timeout=15) == 0
    report = {"native_pointer_route": [1, 0, 1], "clear_pixels": [before, locked, after],
        "did_not_steal_focus": initial_focus != layer, "pointer_resize_delta": [after_resize.right - before_resize.right, after_resize.bottom - before_resize.bottom], "lines": lines}
    Path("target/desktop-overlay-native.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
finally:
    if process.poll() is None: process.kill()
    u.SetCursorPos(original_pointer.x, original_pointer.y)
