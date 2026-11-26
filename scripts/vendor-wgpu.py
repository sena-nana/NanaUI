#!/usr/bin/env python3
"""Vendor the wgpu crates NanaUI forks into `third_party/wgpu/`.

Copies each crate's published source for VERSION (from the local cargo
registry, or downloaded from crates.io), keeps its package name and version,
and points its dependencies on the other vendored crates at their vendored
paths, so a git dependency on this repository brings the fork with it.

NanaUI's own changes to these crates are commits on top of a vendoring commit.
To move to a new upstream version: run this script with that version, commit,
then re-apply the fork's changes (`git diff <old vendoring commit> <last fork
commit> -- third_party/wgpu`).

Usage: python scripts/vendor-wgpu.py 30.0.1
"""

from __future__ import annotations

import io
import re
import shutil
import sys
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEST = ROOT / "third_party" / "wgpu"

CRATES = [
    "naga",
    "naga-types",
    "wgpu",
    "wgpu-core",
    "wgpu-core-deps-apple",
    "wgpu-core-deps-emscripten",
    "wgpu-core-deps-windows-linux-android",
    "wgpu-hal",
    "wgpu-naga-bridge",
    "wgpu-types",
]

# Files cargo adds when publishing; they describe the upstream workspace.
PUBLISH_ONLY = ["Cargo.toml.orig", ".cargo_vcs_info.json", "Cargo.lock"]

DEPENDENCY_TABLE = re.compile(
    r"^\[(?:target\.'[^']*'\.)?(?:dev-|build-)?dependencies\.(?P<key>[A-Za-z0-9_-]+)\]\s*$"
)
# Each vendored crate is its own workspace root. NanaUI's root manifest excludes
# `third_party/wgpu`, so without this cargo keeps looking upward for a
# workspace when it loads one of these manifests on its own (as `cargo fmt
# --all` does): in a checkout nested inside another NanaUI checkout (agent
# worktrees under `.claude/worktrees/`) it finds the outer workspace, which
# does not list the crate, and fails.
STANDALONE_WORKSPACE = [
    "",
    "# Added by scripts/vendor-wgpu.py: a workspace of its own, so cargo does not",
    "# look for one in an enclosing checkout.",
    "[workspace]",
]
PACKAGE = re.compile(r'^package\s*=\s*"(?P<name>[^"]+)"\s*$')


def registry_source(name: str, version: str) -> Path | None:
    registry = Path.home() / ".cargo" / "registry" / "src"
    for index in sorted(registry.glob("index.crates.io-*")):
        candidate = index / f"{name}-{version}"
        if (candidate / "Cargo.toml").is_file():
            return candidate
    return None


def fetch(name: str, version: str, into: Path) -> None:
    source = registry_source(name, version)
    if source is not None:
        shutil.copytree(source, into)
        return
    url = f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
    with urllib.request.urlopen(url) as response:
        data = response.read()
    prefix = f"{name}-{version}/"
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive.getmembers():
            if not member.name.startswith(prefix) or not member.isfile():
                continue
            target = into / member.name[len(prefix) :]
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.extractfile(member).read())


def rewrite_manifest(manifest: Path, version: str) -> None:
    nana_version = f"{version}-nana.1"
    orig_name = manifest.parent.name
    lines = manifest.read_text(encoding="utf-8").splitlines()
    out: list[str] = [
        f"# Vendored by scripts/vendor-wgpu.py from crates.io {orig_name} {version};",
        "# NanaUI's fork (published as nana-*). Dependencies on the other vendored crates use their paths + versions.",
    ]
    i = 0
    in_pkg = False
    saw_homepage = False
    while i < len(lines):
        line = lines[i]
        if line.strip() == "[package]":
            in_pkg = True
            out.append(line)
            i += 1
            continue
        if line.startswith("[") and line.strip() != "[package]":
            if in_pkg and not saw_homepage:
                out.append('homepage = "https://github.com/sena-nana/NanaUI"')
                saw_homepage = True
            in_pkg = False

        if in_pkg:
            if line.startswith("name = "):
                out.append(f'name = "nana-{orig_name}"')
                i += 1
                continue
            if line.startswith("version = "):
                out.append(f'version = "{nana_version}"')
                i += 1
                continue
            if line.startswith("repository = "):
                out.append('repository = "https://github.com/sena-nana/NanaUI"')
                i += 1
                continue
            if line.startswith("homepage = "):
                out.append('homepage = "https://github.com/sena-nana/NanaUI"')
                saw_homepage = True
                i += 1
                continue
            if line.startswith("description = "):
                m = re.match(r'^description\s*=\s*"(.*)"\s*$', line)
                if m and f"(NanaUI fork of wgpu {version})" not in m.group(1):
                    out.append(f'description = "{m.group(1)} (NanaUI fork of wgpu {version})"')
                    i += 1
                    continue

        header = DEPENDENCY_TABLE.match(line)
        if not header:
            out.append(line)
            i += 1
            continue
        block = [line]
        i += 1
        while i < len(lines) and not lines[i].startswith("["):
            block.append(lines[i])
            i += 1
        package = header.group("key")
        for entry in block[1:]:
            explicit = PACKAGE.match(entry.strip())
            if explicit:
                package = explicit.group("name")
        if package in CRATES:
            new_block = [block[0]]
            has_pkg = False
            for entry in block[1:]:
                s = entry.strip()
                if s.startswith("package ="):
                    new_block.append(f'package = "nana-{package}"')
                    has_pkg = True
                elif s.startswith("version ="):
                    new_block.append(f'version = "{nana_version}"')
                else:
                    new_block.append(entry)
            if not has_pkg:
                new_block.insert(1, f'package = "nana-{package}"')
            new_block.insert(2, f'path = "../{package}"')
            block = new_block
        out.extend(block)
    out.extend(STANDALONE_WORKSPACE)
    manifest.write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    version = sys.argv[1]
    DEST.mkdir(parents=True, exist_ok=True)
    for name in CRATES:
        target = DEST / name
        if target.exists():
            shutil.rmtree(target)
        fetch(name, version, target)
        for extra in PUBLISH_ONLY:
            (target / extra).unlink(missing_ok=True)
        rewrite_manifest(target / "Cargo.toml", version)
        print(f"vendored {name} {version}")


if __name__ == "__main__":
    main()
