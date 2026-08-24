#!/usr/bin/env python3
"""Forbid Iced or GPUI from re-entering Nana product crates.

The in-tree engine/iced and engine/gpui-scenario-bench trees were removed.
Workspace members must not depend on iced / iced-wgpu / iced-winit / gpui.
nana-ui-runtime and nana-ui-scene must stay backend-neutral (no Iced, WGPU,
or native GPU implementation crates).
"""

from __future__ import annotations

import json
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
            "--no-deps",
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
                f"Cargo.lock still pins {marker}; hosted windowing must use crates.io winit"
            )

    vendor_accesskit = ROOT / "vendor" / "accesskit_winit"
    if vendor_accesskit.exists():
        failures.append(
            "vendor/accesskit_winit is present; use crates.io accesskit_winit with crates.io winit"
        )

    vendor_arboard = ROOT / "vendor" / "arboard"
    if vendor_arboard.exists():
        failures.append(
            "vendor/arboard is present; Android does not compile arboard, use crates.io on desktop"
        )

    root_metadata = metadata(ROOT / "Cargo.toml")
    forbidden = ICED_PACKAGES | GPUI_PACKAGES
    for package in root_metadata["packages"]:
        for dependency in package["dependencies"]:
            normalized_name = dependency["name"].replace("_", "-")
            if (
                package["name"] in BACKEND_NEUTRAL_PACKAGES
                and normalized_name in GPU_BACKEND_PACKAGES
                and dependency.get("kind") != "dev"
            ):
                failures.append(
                    f'{package["name"]} has GPU/backend dependency '
                    f'{dependency["name"]}; Runtime and Scene must stay backend-neutral'
                )
            if normalized_name not in forbidden:
                continue
            failures.append(
                f'{package["name"]} depends on {dependency["name"]}; '
                "Iced and GPUI observation trees were removed and must not re-enter "
                "the workspace as a product path"
            )

    if failures:
        print("Engine dependency boundary failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    neutral = ", ".join(sorted(BACKEND_NEUTRAL_PACKAGES))
    print(
        f"Engine boundary: OK (Iced/GPUI trees removed; crates.io winit; "
        f"backend-neutral: {neutral})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
