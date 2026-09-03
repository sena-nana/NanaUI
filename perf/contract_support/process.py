"""Explicit subprocess boundaries."""
from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import Sequence




def cargo_run(
    root: Path,
    *,
    package: str,
    binary: str,
    features: str,
    output: Path,
    extra_args: Sequence[str] | None = None,
) -> list[str]:
    command = [
        "cargo",
        "run",
        "--release",
        "--locked",
        "-p",
        package,
        "--features",
        features,
        "--bin",
        binary,
        "--",
        "--output",
        str(output),
    ]
    if extra_args:
        command.extend(extra_args)
    return command



def run_command(command: Sequence[str], cwd: Path) -> None:
    env = os.environ.copy()
    subprocess.run(command, cwd=cwd, check=True, env=env)
