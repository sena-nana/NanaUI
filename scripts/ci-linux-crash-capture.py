#!/usr/bin/env python3
"""Prepare Linux core dumps and collect GDB backtraces on CI test failures.

Used by `.github/workflows/ci.yml` on `ubuntu-latest` so any native crash
(SIGSEGV/SIGABRT) in test binaries yields actionable native stack frames in the
CI log and uploaded `linux-crash-diagnostics` artifact.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
from typing import Callable, Sequence

Runner = Callable[..., subprocess.CompletedProcess[str]]

DEFAULT_CORE_PATTERN_PATH = Path("/proc/sys/kernel/core_pattern")
GDB_TIMEOUT_SECS = 60
CORE_SKIP_SUFFIXES = (".txt", ".log", ".json", ".bt", ".md")


def _default_run(cmd: Sequence[str], *, timeout: int = 30) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        list(cmd),
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )


def _read_text_if_exists(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def prepare(
    core_dir: Path,
    *,
    runner: Runner = _default_run,
    proc_core_pattern: Path = DEFAULT_CORE_PATTERN_PATH,
) -> int:
    """Create `core_dir` and configure `kernel.core_pattern` on Linux if allowed."""
    core_dir.mkdir(parents=True, exist_ok=True)
    abs_dir = core_dir.resolve()
    previous = _read_text_if_exists(proc_core_pattern)
    print(f"crash-capture: core_dir={abs_dir}")
    if previous is not None:
        print(f"crash-capture: initial core_pattern={previous}")

    target_pattern = f"{abs_dir}/core.%e.%p.%s"
    if platform.system() == "Linux":
        try:
            res = runner(
                ["sudo", "-n", "sysctl", "-w", f"kernel.core_pattern={target_pattern}"],
                timeout=15,
            )
            if res.returncode == 0:
                print(f"crash-capture: updated core_pattern={target_pattern}")
            else:
                detail = (res.stderr or res.stdout or "").strip()
                print(f"crash-capture: core_pattern unchanged ({detail or f'exit {res.returncode}'})")
        except (OSError, subprocess.SubprocessError) as exc:
            print(f"crash-capture: skipped sysctl core_pattern update ({exc})")
    return 0


def find_core_files(core_dir: Path, extra_dirs: Sequence[Path] = ()) -> list[Path]:
    """Return discovered core dump files under `core_dir` and `extra_dirs`."""
    found: list[Path] = []
    seen: set[Path] = set()
    for base in (core_dir, *extra_dirs):
        if not base.is_dir():
            continue
        try:
            entries = sorted(base.iterdir())
        except OSError:
            continue
        for entry in entries:
            if not entry.is_file():
                continue
            name = entry.name
            if name != "core" and not name.startswith("core."):
                continue
            if name.endswith(CORE_SKIP_SUFFIXES):
                continue
            resolved = entry.resolve()
            if resolved not in seen:
                seen.add(resolved)
                found.append(entry)
    return found


def parse_executable_from_file_output(output: str) -> str | None:
    """Extract the executable path from `file <core>` output."""
    for pattern in (
        r"execfn:\s*'([^']+)'",
        r"execfn:\s*\"([^\"]+)\"",
        r"from\s+'([^']+)'",
        r"from\s+\"([^\"]+)\"",
    ):
        match = re.search(pattern, output)
        if match:
            candidate = match.group(1).strip()
            if candidate:
                return candidate
    return None


def parse_executable_from_gdb_output(output: str) -> str | None:
    """Extract the executable path from `gdb -ex 'info proc exe'` output."""
    match = re.search(r"exe\s*=\s*['\"]([^'\"]+)['\"]", output)
    if match:
        candidate = match.group(1).strip()
        if candidate:
            return candidate
    return None


def _is_candidate_binary(path: Path) -> bool:
    if not path.is_file():
        return False
    if path.suffix in {".d", ".rlib", ".rmeta", ".so", ".a", ".o", ".pdb", ".txt"}:
        return False
    return os.access(path, os.X_OK) or "-" in path.name


def _fallback_target_binary(core_path: Path, target_dir: Path | None) -> Path | None:
    if target_dir is None or not target_dir.is_dir():
        return None
    parts = core_path.name.split(".")
    if len(parts) < 2 or parts[0] != "core" or not parts[1]:
        return None
    comm = parts[1]
    search_dirs: list[Path] = [target_dir, target_dir / "deps", target_dir / "examples"]
    build_dir = target_dir / "build"
    if build_dir.is_dir():
        try:
            search_dirs.extend(sorted(build_dir.glob("*/out")))
        except OSError:
            pass
    matches: list[Path] = []
    for directory in search_dirs:
        if not directory.is_dir():
            continue
        try:
            for item in directory.iterdir():
                if item.name.startswith(comm) and _is_candidate_binary(item):
                    matches.append(item)
        except OSError:
            continue
    if not matches:
        return None
    matches.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return matches[0]


def resolve_executable(
    core_path: Path,
    target_dir: Path | None,
    *,
    runner: Runner = _default_run,
) -> Path | None:
    """Resolve the crashed executable path for `core_path`."""
    candidates: list[str] = []
    try:
        res = runner(["file", "-b", str(core_path)], timeout=15)
        if res.returncode == 0 and res.stdout:
            parsed = parse_executable_from_file_output(res.stdout)
            if parsed:
                candidates.append(parsed)
    except (OSError, subprocess.SubprocessError):
        pass

    try:
        res = runner(
            ["gdb", "-batch", "-ex", "info proc exe", "-c", str(core_path)],
            timeout=15,
        )
        combined = f"{res.stdout}\n{res.stderr}"
        parsed = parse_executable_from_gdb_output(combined)
        if parsed:
            candidates.append(parsed)
    except (OSError, subprocess.SubprocessError):
        pass

    for raw in candidates:
        path = Path(raw)
        if path.is_file():
            return path
        first_token = raw.split()[0] if raw.split() else ""
        if first_token and Path(first_token).is_file():
            return Path(first_token)

    return _fallback_target_binary(core_path, target_dir)


def summarize_gdb_backtrace(bt_text: str, *, max_frames: int = 12) -> str:
    """Return a concise summary of the terminating signal and top faulting frames."""
    lines = bt_text.splitlines()
    signals = [
        line.strip()
        for line in lines
        if "Program terminated with signal" in line or "Core was generated by" in line
    ]

    threads: list[list[str]] = []
    current: list[str] | None = None
    for line in lines:
        if re.match(r"^Thread\s+\d+", line):
            if current is not None:
                threads.append(current)
            current = [line.strip()]
        elif current is not None:
            current.append(line.rstrip())
    if current is not None:
        threads.append(current)

    chosen: list[str] = []
    if threads:
        priority_tokens = ("SIGSEGV", "libvulkan", "llvmpipe", "lavapipe", "raise", "abort")
        for block in threads:
            joined = "\n".join(block)
            if any(tok in joined for tok in priority_tokens):
                chosen = block
                break
        if not chosen:
            chosen = threads[0]

    summary_lines: list[str] = []
    summary_lines.extend(signals[:2])
    if chosen:
        summary_lines.append(chosen[0])
        frames = [line.strip() for line in chosen[1:] if re.match(r"^\s*#\d+\s+", line)]
        summary_lines.extend(frames[:max_frames])
    elif not summary_lines:
        non_empty = [line.strip() for line in lines if line.strip()]
        summary_lines.extend(non_empty[:max_frames])

    return "\n".join(summary_lines)


def _capture_env(
    out_dir: Path,
    *,
    runner: Runner,
    proc_core_pattern: Path,
) -> None:
    lines = [
        f"platform: {platform.platform()}",
        f"core_pattern: {_read_text_if_exists(proc_core_pattern) or '<unavailable>'}",
    ]
    for label, cmd in (
        ("uname", ["uname", "-a"]),
        ("rustc", ["rustc", "-V"]),
        ("vulkan_pkgs", ["dpkg", "-l", "libvulkan1", "mesa-vulkan-drivers"]),
    ):
        try:
            res = runner(cmd, timeout=15)
            body = (res.stdout or res.stderr or "").strip()
            lines.append(f"--- {label} (exit {res.returncode}) ---")
            if body:
                lines.append(body)
        except (OSError, subprocess.SubprocessError) as exc:
            lines.append(f"--- {label} (error: {exc}) ---")
    (out_dir / "env.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")


def collect(
    core_dir: Path,
    out_dir: Path,
    *,
    target_dir: Path | None = None,
    runner: Runner = _default_run,
    extra_dirs: Sequence[Path] = (),
    proc_core_pattern: Path = DEFAULT_CORE_PATTERN_PATH,
) -> int:
    """Collect GDB backtraces for any core dumps and write CI summary reports."""
    out_dir.mkdir(parents=True, exist_ok=True)
    _capture_env(out_dir, runner=runner, proc_core_pattern=proc_core_pattern)

    cores = find_core_files(core_dir, extra_dirs=extra_dirs)
    summary_sections: list[str] = []

    if not cores:
        msg = f"No core files found in {core_dir}"
        if extra_dirs:
            msg += f" (or {', '.join(str(d) for d in extra_dirs)})"
        summary_sections.append(msg)
        if shutil.which("coredumpctl"):
            try:
                res = runner(["coredumpctl", "list", "--no-pager"], timeout=15)
                body = (res.stdout or res.stderr or "").strip()
                if body:
                    (out_dir / "coredumpctl.txt").write_text(body + "\n", encoding="utf-8")
                    summary_sections.append(f"coredumpctl list (exit {res.returncode}):\n{body}")
            except (OSError, subprocess.SubprocessError) as exc:
                summary_sections.append(f"coredumpctl unavailable/failed: {exc}")
    else:
        for core_path in cores:
            exe_path = resolve_executable(core_path, target_dir, runner=runner)
            cmd = [
                "gdb",
                "-batch",
                "-ex",
                "set pagination off",
                "-ex",
                "info proc",
                "-ex",
                "info registers",
                "-ex",
                "thread apply all bt full",
            ]
            if exe_path is not None:
                cmd.extend([str(exe_path), str(core_path)])
            else:
                cmd.extend(["-c", str(core_path)])

            report_path = out_dir / f"{core_path.name}.bt.txt"
            try:
                res = runner(cmd, timeout=GDB_TIMEOUT_SECS)
                bt_output = f"{res.stdout}\n{res.stderr}".strip()
                report_path.write_text(
                    f"core: {core_path}\nexecutable: {exe_path or '<unknown>'}\nexit: {res.returncode}\n\n{bt_output}\n",
                    encoding="utf-8",
                )
                concise = summarize_gdb_backtrace(bt_output)
                section = (
                    f"=== Core: {core_path.name} (exe: {exe_path or '<unknown>'}) ===\n"
                    f"{concise or '<empty gdb output>'}"
                )
            except (OSError, subprocess.SubprocessError) as exc:
                report_path.write_text(
                    f"core: {core_path}\nexecutable: {exe_path or '<unknown>'}\ngdb error: {exc}\n",
                    encoding="utf-8",
                )
                section = f"=== Core: {core_path.name} (exe: {exe_path or '<unknown>'}) ===\ngdb failed: {exc}"
            summary_sections.append(section)

    summary_text = "\n\n".join(summary_sections) + "\n"
    (out_dir / "summary.txt").write_text(summary_text, encoding="utf-8")
    print(summary_text, end="")
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    prep_p = subparsers.add_parser("prepare", help="Prepare core dump directory and pattern")
    prep_p.add_argument("core_dir", type=Path)

    coll_p = subparsers.add_parser("collect", help="Collect GDB backtraces from core dumps")
    coll_p.add_argument("core_dir", type=Path)
    coll_p.add_argument("out_dir", type=Path)
    coll_p.add_argument("--target-dir", type=Path, default=Path("target/debug"))

    args = parser.parse_args(argv)
    if args.command == "prepare":
        return prepare(args.core_dir)
    if args.command == "collect":
        return collect(
            args.core_dir,
            args.out_dir,
            target_dir=args.target_dir,
            extra_dirs=(Path.cwd(),),
        )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
