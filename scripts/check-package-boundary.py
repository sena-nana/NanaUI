#!/usr/bin/env python3
"""Keep the Issue #226 packaging split honest.

- `nana-package` ships inside applications (behind `nana-ui/packaged-resources`),
  so it stays pure Rust and RNG-free: no zstd C library, no ring / rustls /
  OpenSSL, no system RNG crates, and `blake3` only with its `pure` backend.
- The embeddable runtime crates never reach `nana-package`: a host that does
  its own packaging links none of it.
- `nana-packager` is a build tool: no workspace member may depend on it
  outside `[dev-dependencies]`.

Checks the resolved graph with every feature enabled, so an optional edge
cannot hide.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUNTIME_READER = "nana-package"
BUILD_TOOL = "nana-packager"
FORBIDDEN_IN_READER = {
    "zstd",
    "zstd-sys",
    "zstd-safe",
    "ring",
    "rustls",
    "ureq",
    "openssl",
    "openssl-sys",
    "getrandom",
    "rand",
    "aws-lc-rs",
    "aws-lc-sys",
}
EMBEDDABLE = {"nana-ui-core", "nana-ui-runtime", "nana-ui-scene", "nana-text", "nana-ui-platform"}


def metadata() -> dict:
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        # cargo metadata is UTF-8; Windows' locale default (cp1252) is not.
        encoding="utf-8",
    ).stdout
    return json.loads(output)


def product_edges(node: dict, include_build: bool) -> list[str]:
    kinds = {None, "build"} if include_build else {None}
    return [
        dep["pkg"]
        for dep in node.get("deps", [])
        if any(kind.get("kind") in kinds for kind in dep.get("dep_kinds", []))
    ]


def reachable(data: dict, start: str, include_build: bool = False) -> dict[str, list[str]]:
    """Package id -> path of names from `start`, over product edges."""
    nodes = {node["id"]: node for node in data["resolve"]["nodes"]}
    names = {package["id"]: package["name"] for package in data["packages"]}
    paths = {start: [names[start]]}
    stack = [start]
    while stack:
        current = stack.pop()
        for child in product_edges(nodes[current], include_build):
            if child not in paths:
                paths[child] = paths[current] + [names[child]]
                stack.append(child)
    return paths


def check(data: dict) -> list[str]:
    failures: list[str] = []
    names = {package["id"]: package["name"] for package in data["packages"]}
    members = {member for member in data["workspace_members"]}
    by_name = {names[member]: member for member in members}
    nodes = {node["id"]: node for node in data["resolve"]["nodes"]}

    reader = by_name.get(RUNTIME_READER)
    if reader is None:
        failures.append(f"{RUNTIME_READER} is not a workspace member")
    else:
        for package_id, path in reachable(data, reader).items():
            if names[package_id] in FORBIDDEN_IN_READER:
                failures.append(
                    f"{' -> '.join(path)}: the shipped pack reader must stay pure Rust and RNG-free"
                )
            if names[package_id] == "blake3" and "pure" not in nodes[package_id].get("features", []):
                failures.append(f"{' -> '.join(path)}: blake3 must use its `pure` backend (no C/asm)")

    for name in sorted(EMBEDDABLE):
        member = by_name.get(name)
        if member is None:
            continue
        for package_id, path in reachable(data, member).items():
            if names[package_id] in {RUNTIME_READER, BUILD_TOOL}:
                failures.append(
                    f"{' -> '.join(path)}: embeddable crates must not depend on packaging"
                )

    tool = by_name.get(BUILD_TOOL)
    for member in sorted(members):
        if member == tool:
            continue
        for child in product_edges(nodes[member], include_build=True):
            if child == tool:
                failures.append(
                    f"{names[member]} -> {BUILD_TOOL}: the packager is a build tool, never a dependency"
                )
    return failures


def main() -> int:
    failures = check(metadata())
    for failure in failures:
        print(f"package boundary: {failure}", file=sys.stderr)
    if not failures:
        print("package boundary: ok")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
