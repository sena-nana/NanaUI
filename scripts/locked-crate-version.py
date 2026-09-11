#!/usr/bin/env python3
"""Print the version Cargo.lock pins for one crate.

Used by CI so a workflow cannot name a version the workspace no longer builds:
`v8-package.yml` hard-coded 150.4.0 long after the workspace moved to 152.2.0,
and tagged its release with a version it had not built.
"""

import re
import sys
from pathlib import Path

def locked_version(lock_text: str, crate: str) -> str:
    # Cargo.lock packages are `[[package]]` blocks with `name` before `version`.
    for block in lock_text.split("[[package]]"):
        name = re.search(r'^name = "([^"]+)"', block, re.M)
        if name and name.group(1) == crate:
            version = re.search(r'^version = "([^"]+)"', block, re.M)
            if version:
                return version.group(1)
    raise SystemExit(f"locked-crate-version: {crate} is not in Cargo.lock")

def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: locked-crate-version.py <crate>")
    root = Path(__file__).resolve().parent.parent
    print(locked_version((root / "Cargo.lock").read_text(encoding="utf-8"), sys.argv[1]))

if __name__ == "__main__":
    main()
