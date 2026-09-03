#!/usr/bin/env python3
"""Forbid Iced or GPUI from re-entering Nana product crates.

The in-tree engine/iced and engine/gpui-scenario-bench trees were removed.
Workspace members must not depend on iced / iced-wgpu / iced-winit / gpui.
nana-ui-runtime and nana-ui-scene must stay backend-neutral (no Iced, WGPU,
or native GPU implementation crates).
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
        forbidden = ICED_PACKAGES | GPUI_PACKAGES
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
    packages = {p["name"]: p for p in root_metadata["packages"] if p["id"] in root_metadata["workspace_members"]}
    for package in packages.values():
        crate_root = Path(package["manifest_path"]).parent
        features = set(package["features"])
        for source in (crate_root / "src").rglob("*.rs"):
            for feature in re.findall(r'feature\s*=\s*"([^"\n]+)"', source.read_text()):
                if feature not in features:
                    failures.append(f"{source.relative_to(ROOT)} uses undeclared feature {feature}")
    for workflow in sorted((ROOT / ".github" / "workflows").glob("*.yml")):
        failures.extend(check_cargo_commands(workflow.read_text(), packages, workflow.name))

    if failures:
        print("Engine dependency boundary failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    neutral = ", ".join(sorted(BACKEND_NEUTRAL_PACKAGES))
    print(
        f"Engine boundary: OK (Iced/GPUI trees removed; the pinned upstream winit; "
        f"backend-neutral: {neutral})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
