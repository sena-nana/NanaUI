#!/usr/bin/env python3
"""Keep what the framework's controls say in one replaceable table.

A Chinese string literal in framework source is almost always words a
control will say to a user: those belong in
`crates/nana-ui-core/src/framework_strings.rs`, where a consumer can replace
them. This check fails on any such literal outside that file, the tests, and
the entries below (diagnostics, test content kept in non-test files, and
theme data names that the settings page localizes at display time). A test
is a test file, a module file that opens with `#![cfg(test)]` (or
`#![cfg(any(test, ...))]`), or a `tests` module under `#[cfg(test)]` or
`#[cfg(all(test, ...))]`.
"""

from __future__ import annotations

from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]

SOURCE_ROOTS = (
    "crates/nana-ui-core/src",
    "crates/nana-ui-runtime/src",
    "crates/nana-ui-scene/src",
    "crates/nana-ui-vue/src",
    "crates/nana-ui/src",
)

TABLE = "crates/nana-ui-core/src/framework_strings.rs"

# (path, literal): not interface text.
ALLOWED = {
    # Diagnostic text; documented as not for the interface.
    ("crates/nana-ui/src/presentation.rs", "当前设备没有可用的合成后端"),
    ("crates/nana-ui/src/presentation.rs", "合成目标创建失败，已改用普通窗口呈现"),
    ("crates/nana-ui/src/scene_host/browser.rs", "浏览页面已重新打开，请重新截图"),
    ("crates/nana-ui-runtime/src/paint_script.rs", '`hit` is \\"bounds\\", \\"painted\\" or { \\"path\\": 路径 }'),
    # Built-in theme data names; shown through `theme.light` / `theme.dark`.
    ("crates/nana-ui-core/src/theme/registry.rs", "浅色"),
    ("crates/nana-ui-core/src/theme/registry.rs", "深色"),
    ("crates/nana-ui-core/src/theme/definition.rs", "浅色"),
    ("crates/nana-ui-core/src/theme/definition.rs", "深色"),
    ("crates/nana-ui-core/src/theme/definition.rs", "自定义"),
}

# Whole files whose Chinese literals are test content outside a `tests` module.
ALLOWED_FILES = {
    "crates/nana-ui-runtime/src/framework/text_edit.rs",
    "crates/nana-ui-runtime/src/framework/text_history.rs",
    "crates/nana-ui/src/scene_paint/text/mod.rs",
}

# A module file compiled only for tests, as audit-theme-hardcoding.py reads it.
TEST_ONLY_FILE = re.compile(r"(?:\s*//[^\n]*\n)*\s*#!\[cfg\((?:test|any\(test\b[^\]]*\))\)\]")
TEST_MODULE = re.compile(r"#\[cfg\((?:test|all\(test\b[^\n]*\))\)\]\nmod tests\b")

LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*[一-鿿](?:[^"\\\n]|\\.)*)"')


def is_test_path(path: Path) -> bool:
    parts = path.parts
    return (
        "tests" in parts
        or "bin" in parts
        or path.name in {"tests.rs"}
        or path.name.endswith("_tests.rs")
    )


def scan() -> list[str]:
    problems = []
    for root in SOURCE_ROOTS:
        for path in sorted((ROOT / root).rglob("*.rs")):
            relative = path.relative_to(ROOT).as_posix()
            if relative == TABLE or relative in ALLOWED_FILES or is_test_path(path.relative_to(ROOT)):
                continue
            text = path.read_text(encoding="utf-8")
            if TEST_ONLY_FILE.match(text):
                continue
            cut = TEST_MODULE.search(text)
            if cut:
                text = text[: cut.start()]
            for number, line in enumerate(text.splitlines(), start=1):
                stripped = line.strip()
                if stripped.startswith("//") or "assert" in stripped:
                    continue
                for literal in LITERAL.findall(line):
                    if (relative, literal) in ALLOWED:
                        continue
                    problems.append(f"{relative}:{number}: \"{literal}\"")
    return problems


def main() -> int:
    problems = scan()
    if problems:
        print("Interface words belong in the framework strings table:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
