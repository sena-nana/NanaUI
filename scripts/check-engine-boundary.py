#!/usr/bin/env python3
"""Forbid Iced or GPUI from re-entering Nana product crates, and keep every
product crate free of the text engines NanaUI replaced.

The in-tree engine/iced and engine/gpui-scenario-bench trees were removed.
Workspace members must not depend on iced / iced-wgpu / iced-winit / gpui.
nana-ui-runtime and nana-ui-scene must stay backend-neutral (no Iced, WGPU,
or native GPU implementation crates).

The text engines NanaUI replaced -- cosmic-text, cryoglyph, glyphon -- must not
appear in the dependency graph at all, on any edge, not even a dev one
(Issue #99). The migration is over: the cosmic reference engine and its goldens'
re-recording path were deleted with it, so nothing is left that may legitimately
reach one.

nana-text (Issue #89) additionally must not name cosmic-text or cryoglyph
anywhere under src/, and may borrow only the typography vocabulary from
nana-ui-core.

nana-text's font layer (Issue #90), shaper (Issue #91) and layout engine
(Issue #92) use fontdb, skrifa, icu_properties, harfrust, unicode-bidi and
unicode-linebreak, each from exactly one private module, so none of their types
can leak into the public text API.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
ICED_PACKAGES = {"iced", "iced-wgpu", "iced-winit"}
GPUI_PACKAGES = {"gpui"}
ICED_WINIT_MARKERS = ("iced-rs/winit",)
BACKEND_NEUTRAL_PACKAGES = {"nana-ui-runtime", "nana-ui-scene"}
# Issue #89. `nana-text` owns the text IR, and its *sources* must not name the
# engine it replaced even in a type position — the dependency graph alone cannot
# say that, because the reference engine is a legitimate dev dependency.
TEXT_NEUTRAL_PACKAGES = {"nana-text"}
# Issue #99. The replaced text engines. Forbidden on every workspace member's
# normal dependency edges *and* absent from `Cargo.lock` entirely -- a dev edge
# would mean the reference engine came back.
LEGACY_TEXT_PACKAGES = {"cosmic-text", "cryoglyph", "glyphon"}
# Migration-only crates. Nothing in the product may depend on one. These are
# Cargo *package* names, which are not always the lib target name: the crate in
# tools/css-parity is the package `nana-css-parity` with `[lib] name =
# "css-parity"`, and matching the lib name here would never fire.
REFERENCE_ONLY_PACKAGES = {"nana-css-parity"}
# `nana-text` borrows backend-neutral typography types rather than re-declaring
# them, so the UiWorld adapter stays a field-for-field move. Everything else in
# nana-ui-core -- layout, style model, semantic colour, geometry, and the
# bundled font bytes -- is a boundary violation.
NANA_TEXT_CORE_ALLOWLIST = {
    "DirSpec",
    "FontFeatureSetting",
    "FontKerningSpec",
    "FontVariationSetting",
    "LineBreakSpec",
    "LineHeightSpec",
    "TextAlignSpec",
    "TextWrapBreak",
    "WordBreakSpec",
    "WritingModeSpec",
}

# Issues #90, #91 and #92. Each mature crate behind the font layer, the shaper
# and the layout engine is named from exactly one file of
# `crates/nana-text/src`, and that module is private. `None` means no source
# file may name the crate at all (it is reached only through another).
NANA_TEXT_PRIVATE_BACKENDS = {
    "fontdb": "font/discovery.rs",
    "skrifa": "font/face.rs",
    "read_fonts": None,
    "ttf_parser": None,
    "icu_properties": "font/unicode.rs",
    "harfrust": "shaping/opentype.rs",
    "unicode_bidi": "shaping/bidi.rs",
    "unicode_linebreak": "layout/breaks.rs",
}

GPU_BACKEND_PACKAGES = {
    "ash",
    "d3d12",
    "gpui",
    "iced",
    "iced-wgpu",
    "iced-winit",
    "metal",
    "objc2-metal",
    "vulkano",
    "wgpu",
    "wgpu-core",
    "wgpu-hal",
}


def metadata(manifest: Path) -> dict[str, object]:
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--all-features",
            "--locked",
            "--manifest-path",
            str(manifest),
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        # `cargo metadata` emits UTF-8. Without this, Python decodes with the
        # locale codec, which is cp1252 on the Windows runner and blows up on
        # the non-ASCII text in this repo's manifests.
        encoding="utf-8",
    )
    return json.loads(result.stdout)


def check_dependency_graph(data: dict) -> list[str]:
    failures = []
    packages = {p["id"]: p for p in data["packages"]}
    workspace = set(data["workspace_members"])
    graph = {n["id"]: [d["pkg"] for d in n["deps"] if any(k["kind"] != "dev" for k in d["dep_kinds"])] for n in data["resolve"]["nodes"]}
    wgpu_majors = {p["version"].split(".")[0] for p in packages.values() if p["name"] == "wgpu"}
    if len(wgpu_majors) > 1:
        failures.append(f"multiple WGPU major versions: {sorted(wgpu_majors)}")
    for root in workspace:
        name = packages[root]["name"]
        pending = [(d, [name]) for d in graph.get(root, [])]
        seen = set()
        forbidden = ICED_PACKAGES | GPUI_PACKAGES | LEGACY_TEXT_PACKAGES
        if name in BACKEND_NEUTRAL_PACKAGES:
            forbidden |= GPU_BACKEND_PACKAGES
        while pending:
            dependency, path = pending.pop()
            if dependency in seen:
                continue
            seen.add(dependency)
            package = packages[dependency]
            path = path + [package["name"]]
            if package["name"].replace("_", "-") in forbidden:
                failures.append("forbidden product dependency: " + " -> ".join(path))
            pending.extend((child, path) for child in graph.get(dependency, []))
    return failures


def strip_rust_comments(text: str) -> str:
    """Blank out `//` line comments and `/* */` blocks.

    The source rules below are about the API, not the prose: `lib.rs` has to be
    able to say "this crate does not name cosmic_text" without tripping the rule
    that checks it. String literals containing `//` would be over-stripped, but
    nothing in the crates this runs over has one.
    """
    out = []
    index = 0
    length = len(text)
    while index < length:
        if text.startswith("//", index):
            end = text.find("\n", index)
            index = length if end == -1 else end
        elif text.startswith("/*", index):
            end = text.find("*/", index + 2)
            index = length if end == -1 else end + 2
        else:
            out.append(text[index])
            index += 1
    return "".join(out)


def check_text_engine_sources(crate_root: Path) -> list[str]:
    """`nana-text/src` must not name the engine it replaces, nor reach past the
    typography vocabulary in nana-ui-core.

    This is the mechanical form of "the core API contains no cosmic types": the
    dependency graph alone cannot say it, because the reference engine is a
    legitimate dev dependency.
    """
    failures = []
    source_dir = crate_root / "src"
    if not source_dir.is_dir():
        return failures
    for source in sorted(source_dir.rglob("*.rs")):
        text = strip_rust_comments(source.read_text(encoding="utf-8"))
        where = source.relative_to(ROOT) if source.is_relative_to(ROOT) else source
        for legacy in ("cosmic_text", "cryoglyph", "glyphon"):
            if re.search(rf"\b{legacy}\b", text):
                failures.append(f"{where} names {legacy}; the reference engine belongs in tests/")
        # `use nana_ui_core::{A, B}` as well as a bare `nana_ui_core::A` path.
        for group in re.findall(r"nana_ui_core::\{([^}]*)\}", text):
            for item in group.split(","):
                item = item.strip().split("::")[0].split(" as ")[0].strip()
                if item and item not in NANA_TEXT_CORE_ALLOWLIST:
                    failures.append(f"{where} imports nana_ui_core::{item}, which is off the allowlist")
        for item in re.findall(r"nana_ui_core::([A-Za-z_][A-Za-z0-9_]*)", text):
            if item not in NANA_TEXT_CORE_ALLOWLIST:
                failures.append(f"{where} names nana_ui_core::{item}, which is off the allowlist")
        relative = source.relative_to(source_dir).as_posix()
        for backend, owner in NANA_TEXT_PRIVATE_BACKENDS.items():
            if relative != owner and re.search(rf"\b{backend}\b", text):
                allowed = f"only {owner} may" if owner else "no source file may"
                failures.append(f"{where} names {backend}; {allowed} name it")
    for owner in filter(None, NANA_TEXT_PRIVATE_BACKENDS.values()):
        owner_path = source_dir / owner
        parent = owner_path.parent / "mod.rs"
        if not parent.is_file():
            continue
        declarations = strip_rust_comments(parent.read_text(encoding="utf-8"))
        if re.search(rf"\bpub\s+mod\s+{owner_path.stem}\b", declarations):
            where = parent.relative_to(ROOT) if parent.is_relative_to(ROOT) else parent
            failures.append(f"{where} makes {owner_path.stem} public; it wraps a private backend")
    return failures


def check_reference_only_packages(data: dict) -> list[str]:
    """No product crate may depend on a migration-only reference crate."""
    failures = []
    packages = {p["id"]: p for p in data["packages"]}
    workspace = set(data["workspace_members"])
    for node in data["resolve"]["nodes"]:
        if node["id"] not in workspace:
            continue
        name = packages[node["id"]]["name"]
        if name in REFERENCE_ONLY_PACKAGES:
            continue
        for dependency in node["deps"]:
            child = packages[dependency["pkg"]]["name"]
            if child in REFERENCE_ONLY_PACKAGES:
                failures.append(f"reference-only crate {child} is reachable from {name}")
    return failures


def check_cargo_commands(source: str, packages: dict, origin: str) -> list[str]:
    failures = []
    source = source.replace("\\\n", " ")
    for command in re.findall(r"\bcargo\s+(?:check|test|run|clippy|build)\b([^\n]+)", source):
        selected = re.findall(r"(?:-p|--package)\s+([\w-]+)", command)
        if not selected:
            continue  # workspace-wide commands have no package-local target
        missing = set(selected) - packages.keys()
        failures.extend(f"{origin}: unknown Cargo package {name}" for name in sorted(missing))
        available = [packages[name] for name in selected if name in packages]
        for kind, name in re.findall(r"--(bin|example|test|bench)\s+([\w-]+)", command):
            if not any(t["name"] == name and kind in t["kind"] for p in available for t in p["targets"]):
                failures.append(f"{origin}: missing {kind} {name} in {selected}")
        for raw in re.findall(r"--features[ =]+([\w,/-]+)", command):
            for feature in raw.split(","):
                if "/" in feature:
                    package, feature = feature.split("/", 1)
                    candidates = [packages[package]] if package in packages else []
                else:
                    candidates = available
                if not any(feature in p["features"] for p in candidates):
                    failures.append(f"{origin}: undeclared feature {feature} in {selected}")
    return failures


def main() -> int:
    failures: list[str] = []

    engine_dir = ROOT / "engine"
    if engine_dir.exists():
        failures.append(
            "engine/ is present; Iced and GPUI observation trees were removed from the tree"
        )

    lock_text = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    # The dependency-graph walk below only sees normal edges, so a dev edge to a
    # replaced text engine would pass it. Nothing may reach one any more, and a
    # lockfile entry is the cheapest way to say so.
    for legacy in sorted(LEGACY_TEXT_PACKAGES):
        if re.search(rf'^name = "{re.escape(legacy)}"$', lock_text, re.MULTILINE):
            failures.append(
                f"Cargo.lock still contains {legacy}; the replaced text engines are gone, "
                "including from dev dependencies"
            )
    for marker in ICED_WINIT_MARKERS:
        if marker in lock_text:
            failures.append(
                f"Cargo.lock still pins {marker}; hosted windowing must use the pinned upstream winit"
            )

    vendor_accesskit = ROOT / "vendor" / "accesskit_winit"
    if vendor_accesskit.exists():
        failures.append(
            "vendor/accesskit_winit is present; use crates.io accesskit_winit with the pinned upstream winit"
        )

    vendor_arboard = ROOT / "vendor" / "arboard"
    if vendor_arboard.exists():
        failures.append(
            "vendor/arboard is present; Android does not compile arboard, use crates.io on desktop"
        )

    root_metadata = metadata(ROOT / "Cargo.toml")
    failures.extend(check_dependency_graph(root_metadata))
    failures.extend(check_reference_only_packages(root_metadata))
    packages = {p["name"]: p for p in root_metadata["packages"] if p["id"] in root_metadata["workspace_members"]}
    for package in packages.values():
        crate_root = Path(package["manifest_path"]).parent
        if package["name"] in TEXT_NEUTRAL_PACKAGES:
            failures.extend(check_text_engine_sources(crate_root))
        features = set(package["features"])
        for source in (crate_root / "src").rglob("*.rs"):
            for feature in re.findall(r'feature\s*=\s*"([^"\n]+)"', source.read_text(encoding="utf-8")):
                if feature not in features:
                    failures.append(f"{source.relative_to(ROOT)} uses undeclared feature {feature}")
    for workflow in sorted((ROOT / ".github" / "workflows").glob("*.yml")):
        failures.extend(check_cargo_commands(workflow.read_text(encoding="utf-8"), packages, workflow.name))

    if failures:
        print("Engine dependency boundary failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    neutral = ", ".join(sorted(BACKEND_NEUTRAL_PACKAGES))
    legacy_text = ", ".join(sorted(LEGACY_TEXT_PACKAGES))
    text_sources = ", ".join(sorted(TEXT_NEUTRAL_PACKAGES))
    print(
        f"Engine boundary: OK (Iced/GPUI trees removed; the pinned upstream winit; "
        f"backend-neutral: {neutral}; no edge at all to {legacy_text}; "
        f"sources free of them: {text_sources})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
