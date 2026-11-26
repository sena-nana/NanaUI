#!/usr/bin/env python3
"""Package and publish NanaUI's 37 crates.io packages in topological order."""

from __future__ import annotations

import argparse
import io
import os
import re
import subprocess
import sys
import tarfile
import time
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET_DIR = ROOT / "target" / "publish-check"
MAX_CRATE_BYTES = 10 * 1024 * 1024
USER_AGENT = "nanaui-release-tool (https://github.com/sena-nana/NanaUI)"
INCLUDE_PATH_RE = re.compile(r'\binclude_(?:bytes|str)!\s*\(\s*"([^"]+)"\s*\)')


@dataclass(frozen=True)
class CrateSpec:
    name: str
    version: str
    manifest: Path
    standalone: bool
    layer: int


FORK_CRATES: tuple[CrateSpec, ...] = (
    CrateSpec("nana-naga-types", "30.0.1-nana.1", ROOT / "third_party/wgpu/naga-types/Cargo.toml", True, 0),
    CrateSpec("nana-naga", "30.0.1-nana.1", ROOT / "third_party/wgpu/naga/Cargo.toml", True, 1),
    CrateSpec("nana-wgpu-types", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu-types/Cargo.toml", True, 1),
    CrateSpec("nana-wgpu-naga-bridge", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu-naga-bridge/Cargo.toml", True, 2),
    CrateSpec("nana-wgpu-hal", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu-hal/Cargo.toml", True, 3),
    CrateSpec("nana-wgpu-core-deps-apple", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu-core-deps-apple/Cargo.toml", True, 4),
    CrateSpec("nana-wgpu-core-deps-emscripten", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu-core-deps-emscripten/Cargo.toml", True, 4),
    CrateSpec("nana-wgpu-core-deps-windows-linux-android", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu-core-deps-windows-linux-android/Cargo.toml", True, 4),
    CrateSpec("nana-wgpu-core", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu-core/Cargo.toml", True, 5),
    CrateSpec("nana-wgpu", "30.0.1-nana.1", ROOT / "third_party/wgpu/wgpu/Cargo.toml", True, 6),
    CrateSpec("nana-accesskit-windows", "0.35.1-nana.1", ROOT / "third_party/accesskit/windows/Cargo.toml", True, 0),
)

NANA_VERSION = "0.1.0-alpha.1"

NANA_CRATES: tuple[CrateSpec, ...] = (
    # Layer 0
    CrateSpec("nana-diagnostics", NANA_VERSION, ROOT / "crates/nana-diagnostics/Cargo.toml", False, 10),
    CrateSpec("nana-package", NANA_VERSION, ROOT / "crates/nana-package/Cargo.toml", False, 10),
    CrateSpec("nana-svg-raster", NANA_VERSION, ROOT / "crates/nana-svg-raster/Cargo.toml", False, 10),
    CrateSpec("nana-ui-view-schema", NANA_VERSION, ROOT / "crates/nana-ui-view-schema/Cargo.toml", False, 10),
    # Layer 1
    CrateSpec("nana-gpu", NANA_VERSION, ROOT / "crates/nana-gpu/Cargo.toml", False, 11),
    CrateSpec("nana-ui-core", NANA_VERSION, ROOT / "crates/nana-ui-core/Cargo.toml", False, 11),
    # Layer 2
    CrateSpec("nana-frame-exchange", NANA_VERSION, ROOT / "crates/nana-frame-exchange/Cargo.toml", False, 12),
    CrateSpec("nana-icons-tabler", NANA_VERSION, ROOT / "crates/nana-icons-tabler/Cargo.toml", True, 12),
    CrateSpec("nana-text", NANA_VERSION, ROOT / "crates/nana-text/Cargo.toml", False, 12),
    CrateSpec("nana-ui-charts", NANA_VERSION, ROOT / "crates/nana-ui-charts/Cargo.toml", False, 12),
    CrateSpec("nana-ui-css", NANA_VERSION, ROOT / "crates/nana-ui-css/Cargo.toml", False, 12),
    CrateSpec("nana-ui-input", NANA_VERSION, ROOT / "crates/nana-ui-input/Cargo.toml", False, 12),
    # Layer 3
    CrateSpec("nana-ui-diagram", NANA_VERSION, ROOT / "crates/nana-ui-diagram/Cargo.toml", False, 13),
    CrateSpec("nana-ui-platform", NANA_VERSION, ROOT / "crates/nana-ui-platform/Cargo.toml", False, 13),
    CrateSpec("nana-ui-view-codegen", NANA_VERSION, ROOT / "crates/nana-ui-view-codegen/Cargo.toml", False, 13),
    # Layer 4
    CrateSpec("nana-app-icon", NANA_VERSION, ROOT / "crates/nana-app-icon/Cargo.toml", False, 14),
    CrateSpec("nana-ui-sfc", NANA_VERSION, ROOT / "crates/nana-ui-sfc/Cargo.toml", False, 14),
    CrateSpec("nana-ui-view-macros", NANA_VERSION, ROOT / "crates/nana-ui-view-macros/Cargo.toml", False, 14),
    CrateSpec("nana-window", NANA_VERSION, ROOT / "crates/nana-window/Cargo.toml", False, 14),
    # Layer 5
    CrateSpec("nana-packager", NANA_VERSION, ROOT / "crates/nana-packager/Cargo.toml", False, 15),
    CrateSpec("nana-ui-runtime", NANA_VERSION, ROOT / "crates/nana-ui-runtime/Cargo.toml", False, 15),
    # Layer 6
    CrateSpec("nana-ui-scene", NANA_VERSION, ROOT / "crates/nana-ui-scene/Cargo.toml", False, 16),
    # Layer 7
    CrateSpec("nana-ui", NANA_VERSION, ROOT / "crates/nana-ui/Cargo.toml", False, 17),
    # Layer 8
    CrateSpec("nana-ui-devtools", NANA_VERSION, ROOT / "crates/nana-ui-devtools/Cargo.toml", False, 18),
    CrateSpec("nana-ui-viewport3d", NANA_VERSION, ROOT / "crates/nana-ui-viewport3d/Cargo.toml", False, 18),
    # Layer 9
    CrateSpec("nana-ui-dev", NANA_VERSION, ROOT / "crates/nana-ui-dev/Cargo.toml", False, 19),
)

ALL_CRATES: tuple[CrateSpec, ...] = FORK_CRATES + NANA_CRATES

LEAF_VERIFY_CRATES = {
    "nana-naga-types",
    "nana-accesskit-windows",
    "nana-diagnostics",
    "nana-package",
    "nana-svg-raster",
    "nana-ui-view-schema",
}


def cargo_selector(crate: CrateSpec) -> list[str]:
    if crate.standalone:
        return ["--manifest-path", str(crate.manifest)]
    return ["-p", crate.name]


def validate_manifest_deps(manifest: dict, ws_deps: dict, crate_name: str, check_workspace_ref: bool) -> None:
    problems: list[str] = []

    def scan_deps(table: dict, where: str) -> None:
        for dep, spec in table.items():
            if not isinstance(spec, dict):
                continue
            resolved = spec
            if check_workspace_ref and spec.get("workspace") is True:
                ws_key = spec.get("package", dep)
                ws_spec = ws_deps.get(ws_key)
                if isinstance(ws_spec, dict):
                    resolved = ws_spec
            if "git" in resolved:
                problems.append(f"{where}.{dep} has git dependency")
            if "path" in resolved and "version" not in resolved:
                problems.append(f"{where}.{dep} has path-only dependency without version")

    for sec in ("dependencies", "build-dependencies"):
        scan_deps(manifest.get(sec, {}), sec)
    for target_cfg, tdata in manifest.get("target", {}).items():
        for sec in ("dependencies", "build-dependencies"):
            scan_deps(tdata.get(sec, {}), f"target.{target_cfg}.{sec}")

    if problems:
        raise RuntimeError(
            f"{crate_name} Cargo.toml has invalid dependencies:\n  - "
            + "\n  - ".join(problems)
        )


def strip_cfg_test_blocks(text: str) -> str:
    """Remove `#[cfg(test)]` items from Rust source before checking non-test includes."""
    lines = text.splitlines()
    out: list[str] = []
    skip_until_brace_zero = False
    awaiting_block = False
    depth = 0
    for line in lines:
        s = line.strip()
        if not skip_until_brace_zero and not awaiting_block:
            if s.startswith("#[cfg(test)]"):
                awaiting_block = True
                continue
            out.append(line)
            continue
        if awaiting_block:
            # Skip any stacked attributes before item body
            if s.startswith("#["):
                continue
            opens = line.count("{")
            closes = line.count("}")
            if opens > 0:
                depth = opens - closes
                awaiting_block = False
                if depth > 0:
                    skip_until_brace_zero = True
            elif s.endswith(";"):
                awaiting_block = False
            continue
        if skip_until_brace_zero:
            depth += line.count("{") - line.count("}")
            if depth <= 0:
                skip_until_brace_zero = False
    return "\n".join(out)


def check_internal_includes(crate: CrateSpec) -> None:
    crate_root = crate.manifest.parent.resolve()
    src_dir = crate_root / "src"
    if not src_dir.is_dir():
        return
    for rs in sorted(src_dir.rglob("*.rs")):
        if rs.name in ("tests.rs",) or rs.name.endswith("_tests.rs"):
            continue
        text = strip_cfg_test_blocks(rs.read_text(encoding="utf-8", errors="ignore"))
        for match in INCLUDE_PATH_RE.finditer(text):
            rel_inc = match.group(1)
            resolved = (rs.parent / rel_inc).resolve()
            if not resolved.is_relative_to(crate_root):
                raise RuntimeError(
                    f"{crate.name}: {rs.relative_to(ROOT)} includes `{rel_inc}` outside crate root ({resolved})"
                )


def inspect_packaged_tarball(crate: CrateSpec) -> float:
    tar_path = TARGET_DIR / "package" / f"{crate.name}-{crate.version}.crate"
    if not tar_path.is_file():
        raise RuntimeError(f"missing packaged tarball: {tar_path}")
    size = tar_path.stat().st_size
    if size >= MAX_CRATE_BYTES:
        raise RuntimeError(
            f"{crate.name}-{crate.version}.crate is {size / (1024 * 1024):.2f} MiB (>= 10 MiB crates.io limit)"
        )
    member_name = f"{crate.name}-{crate.version}/Cargo.toml"
    with tarfile.open(tar_path, "r:gz") as tf:
        extracted = tf.extractfile(member_name)
        if extracted is None:
            raise RuntimeError(f"{tar_path} missing {member_name}")
        manifest = tomllib.loads(extracted.read().decode("utf-8"))

    validate_manifest_deps(manifest, {}, crate.name, check_workspace_ref=False)
    check_internal_includes(crate)
    return size / (1024 * 1024)


def inspect_listed_package(crate: CrateSpec, ws_deps: dict) -> float:
    cmd = ["cargo", "package", *cargo_selector(crate), "--list", "--allow-dirty"]
    res = subprocess.run(cmd, cwd=ROOT, text=True, capture_output=True)
    if res.returncode != 0:
        print(res.stdout, file=sys.stderr)
        print(res.stderr, file=sys.stderr)
        raise SystemExit(f"cargo package --list failed for {crate.name}")

    crate_root = crate.manifest.parent
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for rel in res.stdout.splitlines():
            rel = rel.strip()
            if not rel:
                continue
            fp = crate_root / rel
            if fp.is_file():
                tf.add(fp, arcname=f"{crate.name}-{crate.version}/{rel}")
    size = len(buf.getvalue())
    if size >= MAX_CRATE_BYTES:
        raise RuntimeError(
            f"{crate.name} estimated .crate size is {size / (1024 * 1024):.2f} MiB (>= 10 MiB crates.io limit)"
        )

    manifest = tomllib.loads(crate.manifest.read_text(encoding="utf-8"))
    validate_manifest_deps(manifest, ws_deps, crate.name, check_workspace_ref=not crate.standalone)
    check_internal_includes(crate)
    return size / (1024 * 1024)


def run_check() -> int:
    assert len(ALL_CRATES) == 37, f"expected 37 crates, found {len(ALL_CRATES)}"
    TARGET_DIR.mkdir(parents=True, exist_ok=True)
    root_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    ws_deps = root_manifest.get("workspace", {}).get("dependencies", {})
    sizes: list[tuple[str, str, float]] = []

    for idx, crate in enumerate(ALL_CRATES, 1):
        if crate.name in LEAF_VERIFY_CRATES:
            no_verify = ["--no-verify"] if (crate.name == "nana-accesskit-windows" and sys.platform != "win32") else []
            mode_label = "package --no-verify" if no_verify else "package (verified)"
            print(f"[{idx:2}/{len(ALL_CRATES)}] {mode_label} {crate.name} ({crate.version})...", flush=True)
            cmd = [
                "cargo",
                "package",
                *cargo_selector(crate),
                *no_verify,
                "--allow-dirty",
                "--target-dir",
                str(TARGET_DIR),
            ]
            res = subprocess.run(cmd, cwd=ROOT, text=True, capture_output=True)
            if res.returncode != 0:
                print(res.stdout, file=sys.stderr)
                print(res.stderr, file=sys.stderr)
                raise SystemExit(f"cargo package failed for leaf crate {crate.name}")
            mib = inspect_packaged_tarball(crate)
        else:
            print(f"[{idx:2}/{len(ALL_CRATES)}] Auditing {crate.name} ({crate.version}) via --list...", flush=True)
            mib = inspect_listed_package(crate, ws_deps)
        sizes.append((crate.name, crate.version, mib))
        print(f"    OK ({mib:.2f} MiB)", flush=True)

    largest = max(sizes, key=lambda x: x[2])
    print(f"\nAll {len(sizes)} crates audited (< 10 MiB; largest: {largest[0]} {largest[2]:.2f} MiB; 6 leaf crates packaged).")
    return 0


def has_cargo_auth() -> bool:
    if os.environ.get("CARGO_REGISTRY_TOKEN"):
        return True
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    for fname in ("credentials.toml", "credentials"):
        p = cargo_home / fname
        if p.is_file() and p.stat().st_size > 0:
            return True
    return False


def is_published_on_crates_io(name: str, version: str) -> bool:
    url = f"https://crates.io/api/v1/crates/{name}/{version}"
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return resp.status == 200
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return False
        raise


def wait_for_index(name: str, version: str, timeout_secs: int = 180) -> None:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        if is_published_on_crates_io(name, version):
            return
        time.sleep(3)
    raise RuntimeError(f"timed out waiting for {name} {version} on crates.io")


def parse_retry_after_seconds(stderr: str) -> int:
    m = re.search(r"after\s+([A-Za-z0-9,: +-]+?GMT)", stderr)
    if m:
        import email.utils
        dt = email.utils.parsedate_to_datetime(m.group(1))
        return max(30, int(dt.timestamp() - time.time()) + 5)
    return 60


def run_publish(allow_dirty: bool, wait_rate_limit: bool) -> int:
    if not has_cargo_auth():
        print(
            "crates.io authentication not found.\n"
            "Run `cargo login` (or set CARGO_REGISTRY_TOKEN) and then re-run:\n"
            "  python3 scripts/publish-crates.py --publish",
            file=sys.stderr,
        )
        return 2

    prev_layer = None
    published_in_prev_layer: list[CrateSpec] = []

    for idx, crate in enumerate(ALL_CRATES, 1):
        if prev_layer is not None and crate.layer != prev_layer and published_in_prev_layer:
            print(f"Waiting for layer {prev_layer} crates to appear on crates.io...", flush=True)
            for prev in published_in_prev_layer:
                wait_for_index(prev.name, prev.version)
            # Brief buffer for sparse index propagation
            time.sleep(3)
            published_in_prev_layer.clear()
        prev_layer = crate.layer

        if is_published_on_crates_io(crate.name, crate.version):
            print(f"[{idx:2}/{len(ALL_CRATES)}] Skipping {crate.name} {crate.version} (already on crates.io)", flush=True)
            continue

        cmd = [
            "cargo",
            "publish",
            *cargo_selector(crate),
            "--target-dir",
            str(TARGET_DIR),
        ]
        if allow_dirty:
            cmd.append("--allow-dirty")
        if crate.name == "nana-accesskit-windows" and sys.platform != "win32":
            cmd.append("--no-verify")

        while True:
            print(f"[{idx:2}/{len(ALL_CRATES)}] Publishing {crate.name} {crate.version}...", flush=True)
            res = subprocess.run(cmd, cwd=ROOT, text=True, capture_output=True)
            if res.returncode == 0:
                print(f"    Published {crate.name} {crate.version}", flush=True)
                published_in_prev_layer.append(crate)
                break
            combined = res.stdout + "\n" + res.stderr
            if "429" in combined or "Too Many Requests" in combined:
                delay = parse_retry_after_seconds(combined)
                if wait_rate_limit:
                    print(f"    Rate-limited by crates.io (HTTP 429); sleeping {delay}s...", flush=True)
                    time.sleep(delay)
                    continue
                print(
                    f"Rate-limited by crates.io while publishing {crate.name} (HTTP 429).\n"
                    f"Re-run `python3 scripts/publish-crates.py --publish` later (already published crates will be skipped),\n"
                    f"or pass `--wait-rate-limit` to wait automatically.\n\n{combined}",
                    file=sys.stderr,
                )
                return 3
            print(combined, file=sys.stderr)
            return 1

    print("\nAll 37 crates published successfully.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="Package all 37 crates locally and verify tarballs")
    mode.add_argument("--publish", action="store_true", help="Publish all 37 crates to crates.io in topological order")
    parser.add_argument("--allow-dirty", action="store_true", help="Pass --allow-dirty to cargo publish")
    parser.add_argument("--wait-rate-limit", action="store_true", help="Sleep and retry on crates.io HTTP 429")
    args = parser.parse_args()

    if args.check:
        return run_check()
    return run_publish(allow_dirty=args.allow_dirty, wait_rate_limit=args.wait_rate_limit)


if __name__ == "__main__":
    raise SystemExit(main())
