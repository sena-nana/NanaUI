"""Windows native overlay probe: real pointer routing and screen composition.
Build desktop-overlay-probe first. This tool briefly moves the pointer onto its own windows.
"""
import ctypes as c
import json
import platform
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time
from statistics import median

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
cursor_samples = []
try:
    import comtypes.client
except ImportError as error:
    raise SystemExit("Taskbar check needs comtypes (pip install comtypes)") from error
comtypes.client.GetModule("UIAutomationCore.dll")
from comtypes.gen.UIAutomationClient import (CUIAutomation, IUIAutomation, PropertyConditionFlags_MatchSubstring,
    TreeScope_Children, TreeScope_Descendants, UIA_ClassNamePropertyId)
automation = comtypes.client.CreateObject(CUIAutomation, interface=IUIAutomation)
u.FindWindowExW.argtypes = [c.c_void_p, c.c_void_p, c.c_wchar_p, c.c_wchar_p]
u.FindWindowExW.restype = c.c_void_p
def elements(found):
    return [found.GetElement(i) for i in range(found.Length)]
def taskbar_buttons():
    """Names of the task-list buttons only, so the clock and tray icons do not count.

    Buttons are compared as a whole rather than by window title: combined
    buttons are named after the app and window count, in the system language.
    """
    tray = u.FindWindowW("Shell_TrayWnd", None)
    assert tray, "Shell_TrayWnd not found; verify the taskbar manually"
    # Windows 11 exposes XAML task-list button peers.
    buttons = elements(automation.ElementFromHandle(tray).FindAll(TreeScope_Descendants,
        automation.CreatePropertyConditionEx(UIA_ClassNamePropertyId, "TaskListButton", PropertyConditionFlags_MatchSubstring)))
    if not buttons:
        # Windows 10: children of the MSTaskListWClass toolbar.
        bar = u.FindWindowExW(u.FindWindowExW(u.FindWindowExW(tray, None, "ReBarWindow32", None), None, "MSTaskSwWClass", None), None, "MSTaskListWClass", None)
        if bar:
            buttons = elements(automation.ElementFromHandle(bar).FindAll(TreeScope_Children, automation.CreateTrueCondition()))
    assert buttons, "No taskbar buttons found through UI Automation; verify the taskbar manually"
    return tuple(sorted(e.CurrentName or "" for e in buttons))
def settled_taskbar(expect, message, timeout=10, settle=1.5):
    """Snapshot once `expect` has held for `settle` seconds, so a re-added button is caught."""
    deadline, held_since = time.monotonic() + timeout, None
    while time.monotonic() < deadline:
        state = taskbar_buttons()
        if expect(state):
            held_since = time.monotonic() if held_since is None else held_since
            if time.monotonic() - held_since >= settle: return state
        else:
            held_since = None
        time.sleep(.1)
    raise AssertionError(f"{message}: {taskbar_buttons()}")
taskbar_before_probe = taskbar_buttons()
lines, inbox = [], queue.Queue()
transition_samples = []
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
def wait_style(layer, enabled, timeout=2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if bool(u.GetWindowLongW(layer, -20) & 0x20) == enabled:
            return
        time.sleep(.01)
    raise AssertionError(f"Timed out awaiting WS_EX_TRANSPARENT={enabled}")
def wait_for_timed(kind, timeout=35, style=None, **fields):
    started = time.perf_counter()
    event = wait_for(kind, timeout, **fields)
    if style is not None:
        wait_style(layer, style)
    transition_samples.append({"event": kind, "elapsed_ms": (time.perf_counter() - started) * 1000.0})
    return event
def percentile(values, fraction):
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int((len(ordered) - 1) * fraction + 0.999999)))
    return ordered[index]
def system_metadata():
    try:
        gpu_output = subprocess.check_output(
            ["powershell", "-NoProfile", "-Command", "(Get-CimInstance Win32_VideoController).Name"],
            text=True, stderr=subprocess.DEVNULL, timeout=5,
        )
        gpu = [line.strip() for line in gpu_output.splitlines() if line.strip()]
    except (OSError, subprocess.SubprocessError):
        gpu = []
    return {"os": platform.platform(), "python": sys.version.split()[0], "gpu": gpu}
def command(value):
    process.stdin.write(value + "\n"); process.stdin.flush()
def pixel(point):
    dc = u.GetDC(None)
    try: return g.GetPixel(dc, *point)
    finally: u.ReleaseDC(None, dc)
def click(point):
    actual = Point()
    for attempt in range(10):
        assert u.SetCursorPos(*point)
        time.sleep(.05)
        u.GetCursorPos(c.byref(actual))
        if abs(actual.x - point[0]) <= 1 and abs(actual.y - point[1]) <= 1:
            break
    delta = (actual.x - point[0], actual.y - point[1])
    cursor_samples.append({"requested": list(point), "actual": [actual.x, actual.y], "delta": list(delta), "attempts": attempt + 1})
    assert abs(delta[0]) <= 1 and abs(delta[1]) <= 1, ((actual.x, actual.y), point)
    u.mouse_event(2, 0, 0, 0, 0); u.mouse_event(4, 0, 0, 0, 0)
try:
    overlay_ready = wait_for("ready", window=1)
    wait_for("passthrough", window=99, success=False)
    wait_for("skip_taskbar", window=1, skip=True, success=True)
    command("fail"); wait_for("open_failed", window=2)
    time.sleep(1)
    base = u.FindWindowW(None, "NanaUI overlay probe base")
    layer = u.FindWindowW(None, "NanaUI overlay probe layer")
    assert base and layer
    initial_focus = u.GetForegroundWindow()
    assert initial_focus != layer, "Overlay stole focus on creation"
    # The probe's own change proves UI Automation sees its buttons.
    taskbar_base_only = settled_taskbar(lambda state: state != taskbar_before_probe,
        "Probe windows never changed the taskbar")
    rect = Rect(); assert u.GetWindowRect(layer, c.byref(rect))
    scale = float(overlay_ready["scale"])
    hit = overlay_ready["hit"]
    point = (rect.left + 160, rect.top + 100)
    clear_point = (rect.left + 200, rect.top + 200)
    opaque = (int(rect.left + (hit["x"] + hit["w"] / 2) * scale),
              int(rect.top + (hit["y"] + hit["h"] / 2) * scale))
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
    command("forward"); wait_for_timed("passthrough", window=1, enabled=True, success=True, style=True)
    click(clear_point); wait_for("pointer_down", 5, window=0)
    assert u.SetCursorPos(*opaque); time.sleep(.3)
    wait_for_timed("passthrough", window=1, enabled=False, success=True, style=False)
    assert not u.GetWindowLongW(layer, -20) & 0x20, "Forward did not recover hit-testing"
    click(opaque); wait_for("pointer_down", 5, window=1)
    assert u.SetCursorPos(*clear_point); time.sleep(.3)
    wait_for_timed("passthrough", window=1, enabled=True, success=True, style=True)
    click(clear_point); wait_for("pointer_down", 5, window=0)
    for _ in range(4):
        command("forward")
        wait_for_timed("passthrough", window=1, enabled=True, success=True, style=True)
        assert u.SetCursorPos(*opaque); time.sleep(.1)
        wait_for_timed("passthrough", window=1, enabled=False, success=True, style=False)
        assert u.SetCursorPos(*clear_point); time.sleep(.1)
        wait_for_timed("passthrough", window=1, enabled=True, success=True, style=True)
        command("forward-off")
        wait_for("passthrough", window=1, enabled=False, success=True)
    command("forward-off"); wait_for("passthrough", window=1, enabled=False, success=True)
    time.sleep(.3)
    before_resize = Rect(); assert u.GetWindowRect(layer, c.byref(before_resize))
    edge = (before_resize.right - 3, before_resize.bottom - 3)
    assert u.SetCursorPos(*edge); time.sleep(.1); u.mouse_event(2, 0, 0, 0, 0)
    time.sleep(.1)
    for delta in [15, 30, 45]:
        assert u.SetCursorPos(edge[0] + delta, edge[1] + delta); time.sleep(.15)
    u.mouse_event(4, 0, 0, 0, 0); time.sleep(.3)
    after_resize = Rect(); assert u.GetWindowRect(layer, c.byref(after_resize))
    assert after_resize.right > before_resize.right + 20 and after_resize.bottom > before_resize.bottom + 20, "Native resize did not track pointer"
    # Showing the overlay's entry must change the taskbar and hiding it must
    # restore the startup state, which proves the overlay started hidden.
    command("taskbar-show"); wait_for("skip_taskbar", window=1, skip=False, success=True)
    settled_taskbar(lambda state: state != taskbar_base_only, "Overlay entry did not appear on the taskbar")
    command("taskbar-hide"); wait_for("skip_taskbar", window=1, skip=True, success=True)
    settled_taskbar(lambda state: state == taskbar_base_only, "Overlay entry stayed on the taskbar")
    command("hide"); time.sleep(.5)
    command("show"); time.sleep(.5)
    settled_taskbar(lambda state: state == taskbar_base_only, "Overlay entry returned after the window was shown again")
    command("close"); wait_for("closed", window=1); time.sleep(.3)
    after = pixel(clear_point)
    assert before == locked == after, (before, locked, after)
    command("quit"); assert process.wait(timeout=15) == 0
    timings = [sample["elapsed_ms"] for sample in transition_samples]
    p95 = percentile(timings, .95)
    assert p95 <= 100.0, f"Forward transition p95 exceeded 100 ms: {p95:.2f}"
    report = {"native_pointer_route": [1, 0, 1], "clear_pixels": [before, locked, after],
        "did_not_steal_focus": initial_focus != layer, "pointer_resize_delta": [after_resize.right - before_resize.right, after_resize.bottom - before_resize.bottom],
        "forward_transition_ms": {"samples": transition_samples, "p50": median(timings), "p95": p95},
        "environment": {**system_metadata(), "dpi_scale": scale, "sample_count": len(timings)}, "cursor_samples": cursor_samples, "lines": lines}
    Path("target/desktop-overlay-native.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
finally:
    if process.poll() is None: process.kill()
    u.SetCursorPos(original_pointer.x, original_pointer.y)
