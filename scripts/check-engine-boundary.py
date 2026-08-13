#!/usr/bin/env python3
"""Enforce the one-way NanaUI -> in-tree Iced compatibility dependency."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
ENGINE = (ROOT / "engine" / "iced").resolve()


def metadata(manifest: Path) -> dict[str, object]:
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
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
    iced_packages = {"iced", "iced-wgpu", "iced-winit"}
    for package in root_metadata["packages"]:
        for dependency in package["dependencies"]:
            normalized_name = dependency["name"].replace("_", "-")
            if normalized_name not in iced_packages:
                continue

            dependency_path = dependency.get("path")
            if not dependency_path or not is_within(Path(dependency_path), ENGINE):
                failures.append(
                    f'{package["name"]} resolves {dependency["name"]} outside engine/iced'
                )

    if failures:
        print("Iced engine dependency boundary failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print("Iced engine dependency boundary: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
