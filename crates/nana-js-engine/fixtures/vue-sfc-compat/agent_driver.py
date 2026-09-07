"""Shared driver for the headless NanaUI Agent binary.

Locating the binary and keying replies belongs in one place: the three
acceptance scripts hardcoded a Windows-only `.exe` path and aligned replies by
list index, so a script broke on macOS/Linux and any inserted command silently
shifted every later assertion.
"""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]


def binary(name="nana-agent-session"):
    """Path to a built Agent binary, or a message saying how to build it."""
    for candidate in (ROOT / f"target/debug/{name}", ROOT / f"target/debug/{name}.exe"):
        if candidate.exists():
            return candidate
    features = "agent-bin" if name == "nana-agent-session" else "runtime-agent"
    raise SystemExit(
        f"{name} is not built. Run:\n"
        f"  cargo build -p nana-ui-devtools --features {features} --bin {name}"
    )


def run(commands, *, js=None, fixture=None, width=480, height=320, name="nana-agent-session"):
    """Send `commands` over stdio and return `{id: reply}`.

    Each command is given an `id` and the reply carries it back, so callers
    index by request rather than by position.
    """
    # A command that already names an id keeps it, so a script can key its
    # assertions to a stable name instead of a position that shifts whenever a
    # step is inserted.
    numbered = [
        command if "id" in command else dict(command, id=index)
        for index, command in enumerate(commands)
    ]
    argv = [str(binary(name))]
    if js is not None:
        argv += ["--js", str(js)]
    if fixture is not None:
        argv += ["--fixture", fixture]
    argv += ["--width", str(width), "--height", str(height), "--stdio"]
    result = subprocess.run(
        argv,
        input="\n".join(map(json.dumps, numbered)) + "\n",
        capture_output=True,
        text=True,
        check=True,
    )
    replies = [json.loads(line) for line in result.stdout.splitlines()]
    if len(replies) != len(numbered):
        raise AssertionError(
            f"expected {len(numbered)} replies, got {len(replies)}: {result.stderr}"
        )
    failed = [reply for reply in replies if not reply["ok"]]
    if failed:
        raise AssertionError(f"commands failed: {failed}")
    return result.stdout, {reply["id"]: reply for reply in replies}


def screenshot_painted(reply, minimum=8):
    """A screenshot reply's own evidence that something was drawn."""
    pixels = reply.get("pixels")
    assert pixels, f"screenshot reply carries no pixel stats: {reply}"
    assert pixels["unique_colors"] > minimum, (
        f"a flat frame painted nothing: {pixels}"
    )
    return pixels
