#!/usr/bin/env python3
"""Enforce the complete NanaUI <-> in-tree Iced compatibility boundary."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
ENGINE = (ROOT / "engine" / "iced").resolve()
ICED_PACKAGES = {"iced", "iced-wgpu", "iced-winit"}
COMPATIBILITY_PACKAGES = {
    "nana-android-host",
    "nana-ui",
    "nana-ui-vue",
}
BACKEND_NEUTRAL_PACKAGES = {"nana-ui-runtime", "nana-ui-scene"}
GPU_BACKEND_PACKAGES = {
    "ash",
    "d3d12",
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


def is_within(path: Path, parent: Path) -> bool:
    try:
        path.resolve().relative_to(parent)
        return True
    except ValueError:
        return False


def main() -> int:
    failures: list[str] = []

    engine_metadata = metadata(ENGINE / "Cargo.toml")
    for package in engine_metadata["packages"]:
        for dependency in package["dependencies"]:
            normalized_name = dependency["name"].replace("_", "-")
            dependency_path = dependency.get("path")

            if normalized_name == "nana" or normalized_name.startswith("nana-"):
                failures.append(
                    f'{package["name"]} depends on Nana package {dependency["name"]}'
                )

            if dependency_path and not is_within(Path(dependency_path), ENGINE):
                failures.append(
                    f'{package["name"]} has out-of-engine path dependency '
                    f'{dependency["name"]}: {dependency_path}'
                )

    root_metadata = metadata(ROOT / "Cargo.toml")
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
            if normalized_name not in ICED_PACKAGES:
                continue

            dependency_path = dependency.get("path")
            if not dependency_path or not is_within(Path(dependency_path), ENGINE):
                failures.append(
                    f'{package["name"]} resolves {dependency["name"]} outside engine/iced'
                )
            if (
                package["name"].startswith("nana-")
                and package["name"] not in COMPATIBILITY_PACKAGES
                and dependency.get("kind") != "dev"
            ):
                failures.append(
                    f'{package["name"]} has non-dev Iced dependency '
                    f'{dependency["name"]}; only compatibility adapters may depend on Iced'
                )

    if failures:
        print("Iced engine dependency boundary failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    adapters = ", ".join(sorted(COMPATIBILITY_PACKAGES))
    neutral = ", ".join(sorted(BACKEND_NEUTRAL_PACKAGES))
    print(
        f"Iced compatibility boundary: OK (adapters: {adapters}; "
        f"backend-neutral: {neutral})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
