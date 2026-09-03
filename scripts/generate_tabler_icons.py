#!/usr/bin/env python3
"""Generate nana-ui-core icon statics from Tabler SVG.

SVG sources are fetched at generation time and are not embedded in the crate.
"""

from __future__ import annotations

import math
import re
import ssl
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

TABLER = "https://unpkg.com/@tabler/icons@3.46.0/icons/outline/{name}.svg"
OUT = Path(__file__).resolve().parents[1] / "crates/nana-ui-core/src/icon_data.rs"

# Shell icons stay on parse_name. Catalog icons are typed constants only.
# Names are Tabler outline icon ids.
SHELL = [
    ("About", "info-circle"),
    ("Add", "plus"),
    ("Appearance", "sun"),
    ("ArrowLeft", "arrow-left"),
    ("ArrowRight", "arrow-right"),
    ("ArrowUp", "arrow-up"),
    ("Bot", "robot"),
    ("ChevronDown", "chevron-down"),
    ("ChevronRight", "chevron-right"),
    ("ChevronUp", "chevron-up"),
    ("Chart", "chart-line"),
    ("Close", "x"),
    ("Eye", "eye"),
    ("File", "file"),
    ("Folder", "folder"),
    ("GitBranch", "git-branch"),
    ("Maximize", "square"),
    ("MessageSquarePlus", "message-plus"),
    ("Minimize", "minus"),
    ("Moon", "moon"),
    ("More", "dots"),
    ("Nodes", "network"),
    ("Paperclip", "paperclip"),
    ("Restore", "copy"),
    ("Search", "search"),
    ("Settings", "settings"),
    ("ShieldCheck", "shield-check"),
    ("Sidebar", "layout-sidebar"),
    ("Sparkles", "sparkles"),
    ("Workspace", "layout-dashboard"),
]

CATALOG = [
    ("Activity", "activity"),
    ("Atom", "atom"),
    ("Blend", "contrast"),
    ("Bug", "bug"),
    ("Clapperboard", "movie"),
    ("Cpu", "cpu"),
    ("Gamepad2", "device-gamepad"),
    ("MonitorPlay", "device-tv"),
    ("Package", "package"),
    ("Palette", "palette"),
    ("Puzzle", "puzzle"),
    ("Sun", "sun"),
    ("Webcam", "device-computer-camera"),
    ("Wind", "wind"),
]

NS = {"svg": "http://www.w3.org/2000/svg"}
CTX = ssl.create_default_context()


def fetch(name: str) -> str:
    url = TABLER.format(name=name)
    with urllib.request.urlopen(url, context=CTX, timeout=30) as response:
        return response.read().decode("utf-8")


def local_tag(tag: str) -> str:
    return tag.split("}", 1)[-1].lower()


def fnum(value: float) -> str:
    text = f"{value:.4f}".rstrip("0").rstrip(".")
    if text == "-0":
        return "0.0"
    if "." not in text:
        return text + ".0"
    return text


def tokenize_path(d: str) -> list:
    tokens: list = []
    for match in re.finditer(
        r"([MmLlHhVvCcSsQqTtAaZz])|([+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)",
        d.replace(",", " "),
    ):
        cmd, num = match.group(1), match.group(2)
        tokens.append(cmd if cmd else float(num))
    return tokens


def arc_to_cubics(x1, y1, rx, ry, phi_deg, large, sweep, x2, y2):
    if rx == 0 or ry == 0:
        return [((x1, y1), (x2, y2), (x2, y2))]
    rx, ry = abs(rx), abs(ry)
    phi = math.radians(phi_deg % 360.0)
    cos_phi, sin_phi = math.cos(phi), math.sin(phi)
    dx = (x1 - x2) / 2.0
    dy = (y1 - y2) / 2.0
    x1p = cos_phi * dx + sin_phi * dy
    y1p = -sin_phi * dx + cos_phi * dy
    lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
    if lam > 1:
        scale = math.sqrt(lam)
        rx *= scale
        ry *= scale
    sq = max(
        0.0,
        (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p)
        / (rx * rx * y1p * y1p + ry * ry * x1p * x1p),
    )
    coef = math.sqrt(sq)
    if large == sweep:
        coef = -coef
    cxp = coef * (rx * y1p) / ry
    cyp = coef * -(ry * x1p) / rx
    cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) / 2.0
    cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) / 2.0

    def angle(ux, uy, vx, vy):
        dot = ux * vx + uy * vy
        det = ux * vy - uy * vx
        return math.copysign(math.acos(max(-1.0, min(1.0, dot / (math.hypot(ux, uy) * math.hypot(vx, vy) or 1.0)))), det)

    theta1 = angle(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry)
    dtheta = angle((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry)
    if not sweep and dtheta > 0:
        dtheta -= 2 * math.pi
    elif sweep and dtheta < 0:
        dtheta += 2 * math.pi

    segments = max(1, int(math.ceil(abs(dtheta) / (math.pi / 2 + 1e-8))))
    delta = dtheta / segments
    t = 4 / 3 * math.tan(delta / 4)
    cubics = []
    for i in range(segments):
        th1 = theta1 + i * delta
        th2 = th1 + delta
        e1x = rx * math.cos(th1)
        e1y = ry * math.sin(th1)
        e2x = rx * math.cos(th2)
        e2y = ry * math.sin(th2)
        p1 = (
            cos_phi * e1x - sin_phi * e1y + cx,
            sin_phi * e1x + cos_phi * e1y + cy,
        )
        p2 = (
            cos_phi * e2x - sin_phi * e2y + cx,
            sin_phi * e2x + cos_phi * e2y + cy,
        )
        c1 = (
            p1[0] - t * (cos_phi * rx * math.sin(th1) + sin_phi * ry * math.cos(th1)),
            p1[1] - t * (sin_phi * rx * math.sin(th1) - cos_phi * ry * math.cos(th1)),
        )
        c2 = (
            p2[0] + t * (cos_phi * rx * math.sin(th2) + sin_phi * ry * math.cos(th2)),
            p2[1] + t * (sin_phi * rx * math.sin(th2) - cos_phi * ry * math.cos(th2)),
        )
        cubics.append((c1, c2, p2))
    return cubics


def parse_path(d: str) -> list:
    tokens = tokenize_path(d)
    i = 0
    cmd = "M"
    cx = cy = 0.0
    sx = sy = 0.0
    lx = ly = 0.0
    out = []

    def pop(n=1):
        nonlocal i
        vals = tokens[i : i + n]
        i += n
        return vals if n > 1 else vals[0]

    while i < len(tokens):
        if isinstance(tokens[i], str):
            cmd = tokens[i]
            i += 1
            if cmd in "Zz":
                out.append(("Close",))
                cx, cy = sx, sy
                lx, ly = cx, cy
                continue
        if i >= len(tokens):
            break
        rel = cmd.islower()
        c = cmd.upper()
        if c == "M":
            x, y = pop(2)
            if rel:
                x += cx
                y += cy
            cx, cy = x, y
            sx, sy = cx, cy
            out.append(("MoveTo", cx, cy))
            cmd = "l" if rel else "L"
        elif c == "L":
            x, y = pop(2)
            if rel:
                x += cx
                y += cy
            cx, cy = x, y
            out.append(("LineTo", cx, cy))
        elif c == "H":
            x = pop()
            if rel:
                x += cx
            cx = x
            out.append(("LineTo", cx, cy))
        elif c == "V":
            y = pop()
            if rel:
                y += cy
            cy = y
            out.append(("LineTo", cx, cy))
        elif c == "C":
            x1, y1, x2, y2, x, y = pop(6)
            if rel:
                x1 += cx
                y1 += cy
                x2 += cx
                y2 += cy
                x += cx
                y += cy
            out.append(("CubicTo", x1, y1, x2, y2, x, y))
            lx, ly = x2, y2
            cx, cy = x, y
        elif c == "S":
            x2, y2, x, y = pop(4)
            if rel:
                x2 += cx
                y2 += cy
                x += cx
                y += cy
            x1 = 2 * cx - lx
            y1 = 2 * cy - ly
            out.append(("CubicTo", x1, y1, x2, y2, x, y))
            lx, ly = x2, y2
            cx, cy = x, y
        elif c == "Q":
            x1, y1, x, y = pop(4)
            if rel:
                x1 += cx
                y1 += cy
                x += cx
                y += cy
            out.append(
                (
                    "CubicTo",
                    cx + 2 / 3 * (x1 - cx),
                    cy + 2 / 3 * (y1 - cy),
                    x + 2 / 3 * (x1 - x),
                    y + 2 / 3 * (y1 - y),
                    x,
                    y,
                )
            )
            lx, ly = x1, y1
            cx, cy = x, y
        elif c == "T":
            x, y = pop(2)
            if rel:
                x += cx
                y += cy
            x1 = 2 * cx - lx
            y1 = 2 * cy - ly
            out.append(
                (
                    "CubicTo",
                    cx + 2 / 3 * (x1 - cx),
                    cy + 2 / 3 * (y1 - cy),
                    x + 2 / 3 * (x1 - x),
                    y + 2 / 3 * (y1 - y),
                    x,
                    y,
                )
            )
            lx, ly = x1, y1
            cx, cy = x, y
        elif c == "A":
            rx, ry, phi, large, sweep, x, y = pop(7)
            if rel:
                x += cx
                y += cy
            for c1, c2, p in arc_to_cubics(cx, cy, rx, ry, phi, large, sweep, x, y):
                out.append(("CubicTo", c1[0], c1[1], c2[0], c2[1], p[0], p[1]))
            cx, cy = x, y
            lx, ly = cx, cy
        else:
            raise ValueError(f"unsupported path command {cmd}")
        if c not in "CSQT":
            lx, ly = cx, cy
    return out


def rust_cmd(cmd) -> str:
    kind = cmd[0]
    if kind == "MoveTo":
        return f"IconPathCommand::MoveTo([{fnum(cmd[1])}, {fnum(cmd[2])}])"
    if kind == "LineTo":
        return f"IconPathCommand::LineTo([{fnum(cmd[1])}, {fnum(cmd[2])}])"
    if kind == "CubicTo":
        return (
            "IconPathCommand::CubicTo { control_a: ["
            f"{fnum(cmd[1])}, {fnum(cmd[2])}], control_b: ["
            f"{fnum(cmd[3])}, {fnum(cmd[4])}], to: ["
            f"{fnum(cmd[5])}, {fnum(cmd[6])}] }}"
        )
    if kind == "Close":
        return "IconPathCommand::Close"
    raise ValueError(cmd)


def parse_svg(svg: str) -> list[str]:
    root = ET.fromstring(svg)
    shapes: list[str] = []
    path_i = 0
    extra_statics: list[str] = []

    def add_path(d: str, ident: str) -> None:
        nonlocal path_i
        commands = parse_path(d)
        name = f"{ident}_PATH_{path_i}"
        path_i += 1
        body = ",\n        ".join(rust_cmd(cmd) for cmd in commands)
        extra_statics.append(
            f"static {name}: &[IconPathCommand] = &[\n        {body},\n    ];"
        )
        shapes.append(f"Path({name})")

    for node in root.iter():
        tag = local_tag(node.tag)
        # Tabler stamps an invisible full-canvas `stroke="none"` guide path on
        # every icon; it renders nothing and must not enter the geometry.
        if node.attrib.get("stroke") == "none":
            continue
        if tag == "path":
            add_path(node.attrib["d"], "TMP")
        elif tag == "circle":
            cx = float(node.attrib["cx"])
            cy = float(node.attrib["cy"])
            r = float(node.attrib["r"])
            shapes.append(
                f"Circle {{ center: [{fnum(cx)}, {fnum(cy)}], radius: {fnum(r)} }}"
            )
        elif tag == "line":
            x1, y1 = float(node.attrib["x1"]), float(node.attrib["y1"])
            x2, y2 = float(node.attrib["x2"]), float(node.attrib["y2"])
            add_path(f"M{x1} {y1}L{x2} {y2}", "TMP")
        elif tag == "polyline":
            pts = node.attrib["points"].replace(",", " ").split()
            nums = [float(p) for p in pts]
            pairs = list(zip(nums[0::2], nums[1::2]))
            d = f"M{pairs[0][0]} {pairs[0][1]}" + "".join(
                f"L{x} {y}" for x, y in pairs[1:]
            )
            add_path(d, "TMP")
        elif tag == "polygon":
            pts = node.attrib["points"].replace(",", " ").split()
            nums = [float(p) for p in pts]
            pairs = list(zip(nums[0::2], nums[1::2]))
            d = f"M{pairs[0][0]} {pairs[0][1]}" + "".join(
                f"L{x} {y}" for x, y in pairs[1:]
            ) + "Z"
            add_path(d, "TMP")
        elif tag == "rect":
            x = float(node.attrib.get("x", 0))
            y = float(node.attrib.get("y", 0))
            w = float(node.attrib["width"])
            h = float(node.attrib["height"])
            rx = float(node.attrib.get("rx", node.attrib.get("ry", 0) or 0))
            if rx:
                shapes.append(
                    "RoundedRect { origin: ["
                    f"{fnum(x)}, {fnum(y)}], size: [{fnum(w)}, {fnum(h)}], radius: {fnum(rx)} }}"
                )
            else:
                shapes.append(
                    "Rect { origin: ["
                    f"{fnum(x)}, {fnum(y)}], size: [{fnum(w)}, {fnum(h)}], filled: false }}"
                )
    return extra_statics, shapes


def snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).upper()


def compact_svg(svg: str) -> str:
    text = re.sub(r"<\?xml[^?]*\?>", "", svg).strip()
    return re.sub(r">\s+<", "><", text)


def rust_raw(text: str) -> str:
    n = 0
    while True:
        hashes = "#" * n
        if f'"{hashes}' not in text:
            return f'r{hashes}"{text}"{hashes}'
        n += 1


def emit_icon(const: str, tabler: str, svg: str) -> str:
    ident = snake(const)
    statics, shapes = parse_svg(svg)
    rewritten = []
    for block in statics:
        rewritten.append(block.replace("TMP_PATH_", f"{ident}_PATH_"))
    shapes = [s.replace("TMP_PATH_", f"{ident}_PATH_") for s in shapes]
    shape_body = ",\n        ".join(f"IconShape::{s}" for s in shapes)
    svg_raw = rust_raw(compact_svg(svg))
    parts = rewritten + [
        f"static {ident}_SHAPES: &[IconShape] = &[\n        {shape_body},\n    ];",
        f"static {ident}_SVG: &str = {svg_raw};",
        (
            f"static {ident}: IconData = IconData {{\n"
            f'        name: "{tabler}",\n'
            f"        shapes: {ident}_SHAPES,\n"
            f"        svg: {ident}_SVG,\n"
            "    };"
        ),
    ]
    return "\n".join(parts)


def main() -> None:
    chunks = [
        "// @generated by scripts/generate_tabler_icons.py from @tabler/icons 3.46.0 (MIT).",
        "// Do not edit by hand. Painters rasterize `svg`; shapes stay for tests.",
        "use crate::icon::{Icon, IconData, IconPathCommand, IconShape};",
        "",
    ]
    seen_svg: dict[str, str] = {}
    impl_lines = ["#[allow(non_upper_case_globals)]", "impl Icon {"]
    for const, tabler in SHELL + CATALOG:
        if tabler not in seen_svg:
            print(f"fetch {tabler}")
            seen_svg[tabler] = fetch(tabler)
        ident = snake(const)
        if const == "Sun":
            impl_lines.append("    pub const Sun: Self = Self(&APPEARANCE);")
            continue
        chunks.append(emit_icon(const, tabler, seen_svg[tabler]))
        chunks.append("")
        impl_lines.append(f"    pub const {const}: Self = Self(&{ident});")
    impl_lines.append("}")
    chunks.append("\n".join(impl_lines))
    OUT.write_text("\n".join(chunks) + "\n", encoding="utf-8")
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
