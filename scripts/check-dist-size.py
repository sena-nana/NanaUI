#!/usr/bin/env python3
"""Fail when a `[profile.dist]` artifact outgrows its recorded size (Issue #152).

Build first, then check from the repository root:

    cargo build --profile dist -p reactive-sfc --bin reactive-sfc --locked
    python3 scripts/check-dist-size.py

Sizes are recorded per platform (`<sys.platform>-<machine>`), since the same
source links to a different size on each. `--update` re-records the current
platform's sizes; commit that with the reason in `scripts/baselines/README.md`.
"""
from __future__ import annotations

import argparse
import json
import math
import platform
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts/baselines/dist-size.json"


def platform_key() -> str:
    machine = platform.machine().lower()
    machine = {"amd64": "x86_64", "aarch64": "arm64"}.get(machine, machine)
    return f"{sys.platform}-{machine}"


def limit(recorded: int, tolerance: float) -> int:
    return math.floor(recorded * (1 + tolerance))


def judge(size: int, recorded: int, tolerance: float) -> tuple[bool, str]:
    """Whether `size` is within the limit, and a line saying where it stands."""
    delta = (size - recorded) / recorded
    line = f"{size:,} B ({size / 1048576:.2f} MiB), recorded {recorded:,} B, {delta:+.1%}"
    if size > limit(recorded, tolerance):
        return False, f"{line}: over the {tolerance:.0%} limit of {limit(recorded, tolerance):,} B"
    if size < recorded * (1 - tolerance):
        return True, f"{line}: smaller by more than {tolerance:.0%}; re-record with --update to keep the gain"
    return True, line


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--baseline", type=Path, default=BASELINE)
    parser.add_argument("--platform", default=platform_key())
    parser.add_argument("--update", action="store_true", help="re-record this platform's sizes")
    args = parser.parse_args(argv)

    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    tolerance = baseline["tolerance"]
    recorded = baseline["platforms"].get(args.platform)
    if recorded is None:
        print(f"check-dist-size: no sizes recorded for {args.platform}", file=sys.stderr)
        return 2

    failed = False
    for artifact, bytes_recorded in sorted(recorded.items()):
        path = ROOT / artifact
        if not path.is_file():
            print(f"check-dist-size: {artifact} is not built (cargo build --profile dist …)", file=sys.stderr)
            return 2
        size = path.stat().st_size
        if args.update:
            recorded[artifact] = size
            print(f"{artifact}: recorded {size:,} B (was {bytes_recorded:,} B)")
            continue
        ok, line = judge(size, bytes_recorded, tolerance)
        failed |= not ok
        print(f"{artifact}: {line}")

    if args.update:
        args.baseline.write_text(json.dumps(baseline, indent=2) + "\n", encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
