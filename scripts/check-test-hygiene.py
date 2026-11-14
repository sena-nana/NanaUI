#!/usr/bin/env python3
"""Enforce NanaUI's test target count, naming, and anti-hang hygiene rules.

Prevents the test suite from freezing developer machines or bloated target lists
from creeping back in:
1. Cargo target hygiene (`cargo metadata --no-deps --format-version=1`):
   - Non-allowlisted `[[bin]]` targets must set `test = false`.
   - Non-allowlisted `[[example]]` targets must set `test = false`.
   - Integration `[[test]]` harnesses are capped at <= 4 per package and <= 30
     across the workspace.
2. Naming hygiene:
   - No per-issue `issue<digits>*.rs` files under any `tests/` directory or
     `crates/nana-ui-runtime/src/world/`.
3. Anti-hang & isolation hygiene on test code:
   - No `PollType::wait_indefinitely()` or `PollType::Wait { ... }` without
     `timeout: Some(...)`.
   - No `sleep(Duration::from_secs(N))` with `N >= 2` unless the enclosing
     `#[test]` is marked `#[ignore]`.
   - No `env::temp_dir()` + `process::id()` scratch paths without a monotonic
     atomic sequence counter (`AtomicUsize` / `AtomicU64` / `AtomicU32`).
   - No nested `Command::new("cargo")` / `Command::new(env!("CARGO"))` running
     `build`, `test`, or `run` inside Rust tests.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

MAX_TESTS_PER_PACKAGE = 4
MAX_TESTS_WORKSPACE = 30

ALLOWED_TEST_BINS = {
    ("component-gallery", "ui-benchmark"),
    ("component-gallery", "ui-snapshots"),
    ("nana-ui", "nana-gpu-scene-benchmark"),
    ("nana-ui-scene", "nana-dirty-frame-benchmark"),
    ("nana-ui-scene", "nana-text-edit-benchmark"),
    ("native-rhi-probe", "native-rhi-probe"),
}

ALLOWED_TEST_EXAMPLES = {
    ("nana-ui", "openvr-output-probe"),
    ("nana-ui", "presentation-output-probe"),
    ("nana-ui", "web-output-producer"),
}

SCAN_ROOTS = ("crates", "examples", "tools", "third_party")


def load_cargo_metadata(root: Path = ROOT) -> dict:
    raw = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version=1"],
        cwd=root,
        text=True,
    )
    return json.loads(raw)


def check_metadata_targets(metadata: dict) -> list[str]:
    errors: list[str] = []
    workspace_tests = 0

    for pkg in sorted(metadata.get("packages", []), key=lambda p: p["name"]):
        pkg_name = pkg["name"]
        pkg_tests = 0
        for target in pkg.get("targets", []):
            kinds = set(target.get("kind", []))
            tname = target["name"]
            if "test" in kinds:
                pkg_tests += 1
                workspace_tests += 1
            if "bin" in kinds and target.get("test", True):
                if (pkg_name, tname) not in ALLOWED_TEST_BINS:
                    errors.append(
                        f"{pkg_name}: [[bin]] `{tname}` must set `test = false` "
                        "(or belong to ALLOWED_TEST_BINS)"
                    )
            if "example" in kinds and target.get("test", True):
                if (pkg_name, tname) not in ALLOWED_TEST_EXAMPLES:
                    errors.append(
                        f"{pkg_name}: [[example]] `{tname}` must set `test = false` "
                        "(or belong to ALLOWED_TEST_EXAMPLES)"
                    )

        if pkg_tests > MAX_TESTS_PER_PACKAGE:
            errors.append(
                f"{pkg_name}: has {pkg_tests} integration test targets "
                f"(cap is {MAX_TESTS_PER_PACKAGE})"
            )

    if workspace_tests > MAX_TESTS_WORKSPACE:
        errors.append(
            f"workspace has {workspace_tests} integration test targets "
            f"(cap is {MAX_TESTS_WORKSPACE})"
        )

    return errors


def _iter_rust_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for sub in SCAN_ROOTS:
        base = root / sub
        if not base.exists():
            continue
        for path in sorted(base.rglob("*.rs")):
            if "target" in path.parts:
                continue
            files.append(path)
    return files


def _extract_test_slice(path: Path, root: Path) -> tuple[str, int, list[str], str] | None:
    rel = path.relative_to(root).as_posix()
    text = path.read_text(encoding="utf-8", errors="ignore")
    lines = text.splitlines()
    is_dedicated = (
        "/tests/" in f"/{rel}"
        or rel.endswith("/tests.rs")
        or rel.endswith("_tests.rs")
        or text.lstrip().startswith("#![cfg(test)]")
    )
    if is_dedicated:
        return rel, 1, lines, text
    for idx, line in enumerate(lines):
        if "#[cfg(test)]" in line:
            return rel, idx + 1, lines[idx:], "\n".join(lines[idx:])
    return None


def _enclosing_fn_has_ignore(lines: list[str], rel_idx: int) -> bool:
    for back in range(rel_idx, -1, -1):
        stripped = lines[back].strip()
        if re.match(r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+", stripped):
            attr_start = max(0, back - 8)
            attrs = "\n".join(lines[attr_start:back])
            if "#[ignore" in attrs:
                return True
            if "#[test]" in attrs:
                return False
            # Helper fn without #[test]; keep scanning? Treat un-ignored long sleep inside helper as unsafe too.
            return False
    return False


def check_source_hygiene(root: Path) -> list[str]:
    errors: list[str] = []

    for path in _iter_rust_files(root):
        rel = path.relative_to(root).as_posix()
        if re.match(r"^issue\d+", path.name):
            if "/tests/" in f"/{rel}" or rel.startswith("crates/nana-ui-runtime/src/world/"):
                errors.append(
                    f"{rel}: per-issue test filename `issue<N>*.rs` is forbidden; "
                    "consolidate into a domain test module"
                )

        test_slice = _extract_test_slice(path, root)
        if test_slice is None:
            continue
        rel, start_ln, lines, slice_text = test_slice

        for offset, line in enumerate(lines):
            lineno = start_ln + offset
            if "PollType::wait_indefinitely" in line:
                errors.append(f"{rel}:{lineno}: unbounded PollType::wait_indefinitely() in test code")
            elif "PollType::Wait" in line and "timeout: Some" not in line:
                window = "\n".join(lines[offset : offset + 8])
                if "timeout: Some" not in window:
                    errors.append(
                        f"{rel}:{lineno}: PollType::Wait without `timeout: Some(...)` in test code"
                    )

            m_sleep = re.search(
                r"sleep\s*\(\s*(?:std::time::)?Duration::from_secs\s*\(\s*(\d+)", line
            )
            if m_sleep and int(m_sleep.group(1)) >= 2:
                if not _enclosing_fn_has_ignore(lines, offset):
                    errors.append(
                        f"{rel}:{lineno}: blocking sleep >= 2s in non-#[ignore] test code"
                    )

            if re.search(
                r'Command::new\s*\(\s*(?:"cargo"|env!\s*\(\s*"CARGO"\s*\))\s*\)',
                line,
            ):
                window = "\n".join(lines[offset : offset + 8])
                if re.search(r'"(?:build|test|run)"', window):
                    errors.append(
                        f"{rel}:{lineno}: nested `cargo build/test/run` inside test code"
                    )

        if "temp_dir()" in slice_text and "process::id()" in slice_text:
            if not any(
                tok in slice_text
                for tok in ("AtomicUsize", "AtomicU64", "AtomicU32", "unique_test_suffix(")
            ):
                errors.append(
                    f"{rel}: `temp_dir()` + `process::id()` requires an atomic sequence counter"
                )

    return errors


def evaluate(root: Path = ROOT, metadata: dict | None = None) -> list[str]:
    if metadata is None:
        metadata = load_cargo_metadata(root)
    return check_metadata_targets(metadata) + check_source_hygiene(root)


def main() -> int:
    errors = evaluate(ROOT)
    if errors:
        for err in errors:
            print(f"test-hygiene error: {err}", file=sys.stderr)
        return 1
    print("test hygiene check: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
