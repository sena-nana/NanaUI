#!/usr/bin/env python3
"""Issue #101 §2 inventory: design values a built-in component still owns.

Phase 0 needs a number later phases can watch go down, not a prose list that
rots on the first refactor. The unit is the component, because that is the
unit Issue #100 migrates: every impl block a component owns — the
`impl ComponentView for X` that projects it *and* the `impl X` whose
constructor writes its defaults — is read as X's own design authority.

Reading only the trait body misses most of it. `ListItem` declares hover,
pressed, focused, selected and disabled paints in `ListItem::new`, and a scan
that skipped inherent impls reported it as styling no state at all.

Per component it records:

    color_role      a `SemanticColorRole` the component picks for itself
    color_literal   raw RGBA (`rgb8` / `rgba8` / `rgba(`) written in component code
    design_number   a bare number in a visual field (radius, padding, gap,
                    size, font, opacity, duration)
    motion          a duration or easing the component names itself
    elevation       a shadow / elevation the component picks
    states          which interaction states the component styles

`color_role` and `states` are **not** defects: "primary hover is
AccentStrong" is exactly the semantic intent Issue #100 wants components to
express. They are counted because they are the population that moves into
Component Recipes, so the migration can be sized and so a state matrix can be
read off the source instead of guessed.

`design_number`, `color_literal` and `motion` are what the migration removes.
A value already read from the token authority (`UI_METRICS`, `metrics.`,
`space::`, `type_scale::`, `ControlSize::`) is counted as `token_read`
instead — that is the denominator, not a finding.

Module-level `const NAME: f32 = …` design constants are reported per file
alongside, because a component that moved its numbers to a private const did
not move them to the Theme.

This is an inventory, not a parser. It reads text after stripping comments,
strings and test modules, so a value built across several lines is missed and
a number that is not a design value can be caught. It is written to be re-run
and diffed, which is what makes it useful.

    python3 scripts/audit-theme-hardcoding.py --format markdown
    python3 scripts/audit-theme-hardcoding.py --output <path>.json
    python3 scripts/audit-theme-hardcoding.py --check <path>.json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Iterable

REPO_ROOT = Path(__file__).resolve().parents[1]

# Built-in components live in the Runtime crate. `nana-ui-core` is excluded on
# purpose: it is the token authority, so a number there is the token.
COMPONENT_ROOT = "crates/nana-ui-runtime/src"

SKIP_PARTS = ("bin", "benches", "fixtures", "corpus")
SKIP_SUFFIXES = ("_tests.rs", "tests.rs")

NUMBER = r"-?\d+(?:\.\d+)?"

# Visual field stems. Matched with an optional prefix and suffix, so
# `border_radius`, `blur_radius`, `min_height` and `icon_size` all count —
# a design value does not stop being one because the field name is compound.
# `extent`, `min_x` and friends stay out: they are geometry maths, and
# counting them would drown the design values.
DESIGN_FIELDS = (
    "radius",
    "padding",
    "gap",
    "inset",
    "margin",
    "width",
    "height",
    "size",
    "font_size",
    "font_weight",
    "line_height",
    "letter_spacing",
    "opacity",
    "alpha",
    "duration_ms",
)

INTERACTION_STATES = (
    "hovered",
    "pressed",
    "focused",
    "disabled",
    "selected",
    "selected_hovered",
    "selected_pressed",
)

# Naming a design step *is* reading the token authority. `RadiusTier::Sm`
# defers the number to the installed theme exactly as `UI_METRICS.radius_sm`
# defers it to the constant — only later, which is the whole point. Without
# this, migrating a component from the constant to the tier would read as the
# denominator shrinking, the opposite of what happened.
# Issue #102 added token categories, and with them new ways to *name* a step:
# `MotionRole::HoverColor` defers a duration to the installed theme exactly as
# `RadiusTier::Sm` defers a radius. They belong here for the same reason —
# without them, moving a component off `motion::HOVER_COLOR` onto the role
# would read as the denominator shrinking while the numerator held, which is
# the opposite of what happened.
TOKEN_READ = re.compile(
    r"\bUI_METRICS\b|\bmetrics\.\w|\bspace::|\btype_scale::|\bControlSize::"
    r"|\bUI_BASE_TEXT_SIZE\b|\bTooltipConfig::|\bstyle_model\b|\bRadiusTier::"
    r"|\bControlHeight::|\bControlPadding::|\bSurfacePadding::|\bSquareSize::"
    r"|\bMotionRole::|\bEasingRole::|\bElevationRole::|\bSpacingStep::|\bTypeRole::"
    r"|\bLineRole::|\bTextWeight::|\bBorderWidth::|\bStateLayer::|\bSurfaceRole::"
    r"|\bComponentRecipeId::|\brecipes\(\)|\bStatusRecipe::|\bOpacityTokens::"
)

COLOR_ROLE = re.compile(r"\bSemanticColorRole::(\w+)")
COLOR_LITERAL = re.compile(r"\brgba?8?\s*\(\s*\d")
# A struct-field initializer (`border_radius: 6.0`) or an assignment of a
# literal option (`layout.border_radius = Some(6.0)`). Plain `=` is excluded:
# `let width = 0.0;` is a local, not a design decision, and including it turns
# the inventory into a census of geometry maths.
DESIGN_NUMBER = re.compile(
    r"\w*(?:" + "|".join(DESIGN_FIELDS) + r")\w*\s*"
    r"(?::\s*(?:Some\(\s*)?|=\s*Some\(\s*)" + NUMBER
)
MOTION = re.compile(r"\bfrom_millis\s*\(\s*\d+|\bEasing::\w+")
# `ComponentElevation::from_shadow` is the opposite of picking an elevation:
# it consumes an `ElevationRole` the theme resolved. Counting it here would
# make the number climb as the migration succeeds. `from_box_shadow` stays
# counted — a CSS shadow arriving from L1 is still a shadow nobody tokenized.
ELEVATION = re.compile(r"\bComponentElevation::(?!from_shadow\b)\w+")
STATE_FIELD = re.compile(
    r"\b(?:interaction|InteractionStyle)\b[^;\n]*?\b(" + "|".join(INTERACTION_STATES) + r")\b"
    r"|\b(" + "|".join(INTERACTION_STATES) + r")\s*[:.]\s*(?:SemanticPaint|\w+\s*=|background|foreground|border)"
)

# A component-view impl header, with or without generics and lifetimes.
IMPL_HEADER = re.compile(r"impl\s*(?:<[^>]*>\s*)?ComponentView\s+for\s+([\w:]+)")

# An inherent `impl X {`. The trailing `{` is what separates it from a trait
# impl: `impl ComponentView for X {` has ` for X` in between and cannot match.
INHERENT_HEADER = re.compile(r"impl\s*(?:<[^>]*>\s*)?([A-Z]\w*)(?:<[^>]*>)?\s*\{")

# Module-level named design constants.
DESIGN_CONST = re.compile(
    r"(?:pub(?:\([^)]*\))?\s+)?const\s+(\w+)\s*:\s*(f32|f64|u16|u32|u64)\s*=\s*(" + NUMBER + r")\s*;"
)

COUNT_KEYS = ("color_role", "color_literal", "design_number", "motion", "elevation")


def strip_rust(source: str) -> str:
    """Drop comments, string/char literals and `#[cfg(test)]` modules.

    Not a parser. Deliberately conservative: anything it cannot resolve stays
    in the text and is counted, because an inventory that silently drops code
    is worse than one that over-counts.
    """
    out: list[str] = []
    index = 0
    length = len(source)
    while index < length:
        char = source[index]
        if source.startswith("//", index):
            end = source.find("\n", index)
            index = length if end == -1 else end
            continue
        if source.startswith("/*", index):
            end = source.find("*/", index + 2)
            index = length if end == -1 else end + 2
            continue
        if char == '"':
            index += 1
            while index < length:
                if source[index] == "\\":
                    index += 2
                    continue
                if source[index] == '"':
                    index += 1
                    break
                index += 1
            out.append('""')
            continue
        out.append(char)
        index += 1
    return _drop_test_modules("".join(out))


def _drop_test_modules(source: str) -> str:
    pattern = re.compile(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{")
    while True:
        match = pattern.search(source)
        if match is None:
            return source
        end = _match_brace(source, match.end() - 1)
        source = source[: match.start()] + source[end + 1 :]


def _match_brace(source: str, open_index: int) -> int:
    """Index of the `}` closing the `{` at `open_index`, or the last index."""
    depth = 0
    for index in range(open_index, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return index
    return len(source) - 1


def display_path(path: Path) -> str:
    """Repo-relative when it can be, the path itself otherwise.

    The product scan always runs inside the repo; a test that points the
    scanner at a scratch directory should exercise the scan, not trip over
    the label.
    """
    try:
        return path.relative_to(REPO_ROOT).as_posix()
    except ValueError:
        return path.as_posix()



def rust_sources(root: Path) -> Iterable[Path]:
    for path in sorted(root.rglob("*.rs")):
        relative = path.relative_to(root)
        if any(part in SKIP_PARTS for part in relative.parts):
            continue
        if path.name.endswith(SKIP_SUFFIXES):
            continue
        yield path


def scan_body(body: str) -> dict[str, object]:
    counts = dict.fromkeys(COUNT_KEYS, 0)
    token_reads = 0
    roles: set[str] = set()
    states: set[str] = set()
    for line in body.splitlines():
        if not line.strip():
            continue
        tokenized = bool(TOKEN_READ.search(line))
        if tokenized:
            token_reads += 1
        for name in COLOR_ROLE.findall(line):
            roles.add(name)
            counts["color_role"] += 1
        counts["color_literal"] += len(COLOR_LITERAL.findall(line))
        counts["motion"] += len(MOTION.findall(line))
        counts["elevation"] += len(ELEVATION.findall(line))
        if not tokenized:
            counts["design_number"] += len(DESIGN_NUMBER.findall(line))
        for first, second in STATE_FIELD.findall(line):
            states.add(first or second)
    return {
        "counts": counts,
        "token_reads": token_reads,
        "roles": sorted(roles),
        "states": sorted(states),
    }


def component_names(root: Path, sources: dict[Path, str]) -> set[str]:
    """Every type with an `impl ComponentView`. That is the component set."""
    names: set[str] = set()
    for text in sources.values():
        names.update(match.group(1) for match in IMPL_HEADER.finditer(text))
    return names


def component_bodies(text: str, components: set[str]) -> list[tuple[str, str, int, int]]:
    """`(component, body, start, end)` for every impl block a component owns.

    Both kinds count. `impl ComponentView for ListItem` is where the component
    projects itself, but `impl ListItem` is where its constructor writes the
    `InteractionStyle` that decides how it looks hovered, pressed, selected and
    disabled. Reading only the trait body reports ListItem as styling no state
    at all, which is the opposite of true.
    """
    found: list[tuple[str, str, int, int]] = []
    for pattern, group in ((IMPL_HEADER, 1), (INHERENT_HEADER, 1)):
        for match in pattern.finditer(text):
            name = match.group(group)
            if name not in components:
                continue
            brace = text.find("{", match.end() - 1)
            if brace == -1:
                continue
            end = _match_brace(text, brace)
            found.append((name, text[brace : end + 1], match.start(), end + 1))
    found.sort(key=lambda entry: entry[2])
    return found


def scan_components(root: Path) -> list[dict[str, object]]:
    sources = {
        path: strip_rust(path.read_text(encoding="utf-8", errors="replace"))
        for path in rust_sources(root)
    }
    components = component_names(root, sources)
    merged: dict[str, dict[str, object]] = {}
    for path, text in sources.items():
        for name, body, _start, _end in component_bodies(text, components):
            scanned = scan_body(body)
            entry = merged.setdefault(
                name,
                {
                    "component": name,
                    "file": display_path(path),
                    "counts": dict.fromkeys(COUNT_KEYS, 0),
                    "token_reads": 0,
                    "roles": set(),
                    "states": set(),
                },
            )
            for key in COUNT_KEYS:
                entry["counts"][key] += scanned["counts"][key]  # type: ignore[index]
            entry["token_reads"] = int(entry["token_reads"]) + int(scanned["token_reads"])
            entry["roles"].update(scanned["roles"])  # type: ignore[union-attr]
            entry["states"].update(scanned["states"])  # type: ignore[union-attr]
    out: list[dict[str, object]] = []
    for entry in merged.values():
        counts = entry["counts"]
        entry["roles"] = sorted(entry["roles"])  # type: ignore[arg-type]
        entry["states"] = sorted(entry["states"])  # type: ignore[arg-type]
        entry["local_values"] = (
            counts["color_literal"]  # type: ignore[index]
            + counts["design_number"]  # type: ignore[index]
            + counts["motion"]  # type: ignore[index]
        )
        out.append(entry)
    out.sort(key=lambda entry: str(entry["component"]))
    return out


def scan_shared_authority(root: Path) -> list[dict[str, object]]:
    """Design values outside any impl block a component owns.

    A component's own impls are only half its appearance: the retained visuals
    (`StandardVisual`) are coloured in `world/extraction.rs` and sized in
    `world/geometry*`, so the paint authority for a Button is not in the
    Button. Scanning what is left after the component bodies are removed is
    how that second authority becomes visible instead of reading as zero.
    """
    sources = {
        path: strip_rust(path.read_text(encoding="utf-8", errors="replace"))
        for path in rust_sources(root)
    }
    components = component_names(root, sources)
    files: list[dict[str, object]] = []
    for path, text in sources.items():
        for _name, _body, start, end in reversed(component_bodies(text, components)):
            text = text[:start] + text[end:]
        scanned = scan_body(text)
        counts = scanned["counts"]
        local = (
            counts["color_literal"]  # type: ignore[index]
            + counts["design_number"]  # type: ignore[index]
            + counts["motion"]  # type: ignore[index]
        )
        if not local and not counts["color_role"]:  # type: ignore[index]
            continue
        files.append(
            {
                "file": display_path(path),
                "counts": counts,
                "local_values": local,
                "token_reads": scanned["token_reads"],
                "roles": scanned["roles"],
            }
        )
    files.sort(
        key=lambda entry: (
            -int(entry["local_values"]),
            -int(entry["counts"]["color_role"]),  # type: ignore[index]
            str(entry["file"]),
        )
    )
    return files



def scan_module_constants(root: Path) -> list[dict[str, object]]:
    files: list[dict[str, object]] = []
    for path in rust_sources(root):
        text = strip_rust(path.read_text(encoding="utf-8", errors="replace"))
        names = [
            {"name": name, "type": kind, "value": float(value)}
            for name, kind, value in DESIGN_CONST.findall(text)
        ]
        if names:
            files.append(
                {
                    "file": display_path(path),
                    "constants": names,
                    "count": len(names),
                }
            )
    files.sort(key=lambda entry: (-int(entry["count"]), str(entry["file"])))
    return files


def scan() -> dict[str, object]:
    root = REPO_ROOT / COMPONENT_ROOT
    components = scan_components(root)
    shared = scan_shared_authority(root)
    constants = scan_module_constants(root)
    totals = dict.fromkeys(COUNT_KEYS, 0)
    shared_totals = dict.fromkeys(COUNT_KEYS, 0)
    token_reads = 0
    for entry in components:
        for key in COUNT_KEYS:
            totals[key] += int(entry["counts"][key])  # type: ignore[index]
        token_reads += int(entry["token_reads"])
    for entry in shared:
        for key in COUNT_KEYS:
            shared_totals[key] += int(entry["counts"][key])  # type: ignore[index]
        token_reads += int(entry["token_reads"])
    return {
        "schema_version": 1,
        "issue": 101,
        "root": COMPONENT_ROOT,
        "components_scanned": len(components),
        "totals": totals,
        "shared_totals": shared_totals,
        "token_reads": token_reads,
        "local_values": totals["color_literal"] + totals["design_number"] + totals["motion"],
        "shared_local_values": (
            shared_totals["color_literal"]
            + shared_totals["design_number"]
            + shared_totals["motion"]
        ),
        "module_constants": sum(int(entry["count"]) for entry in constants),
        "components": components,
        "shared_authority": shared,
        "constant_files": constants,
        "notes": [
            "Text inventory, not a parser. Over-counts rather than silently dropping code.",
            "color_role and states are intent, not defects: they move into Component Recipes.",
            "token_reads counts lines already reading the token authority; it is the denominator.",
            "Module constants are per file: a private const is not the Theme either.",
            "shared_authority is what is left after the component bodies are removed: the retained-visual paint and geometry that decide a control's appearance outside the control.",
        ],
    }


def markdown(report: dict[str, object], top: int) -> str:
    lines: list[str] = []
    totals = report["totals"]  # type: ignore[index]
    lines.append(
        f"Scanned {report['components_scanned']} components (trait and inherent impls) "
        f"under `{report['root']}`."
    )
    lines.append("")
    lines.append("| Category | Count |")
    lines.append("| --- | ---: |")
    for key in COUNT_KEYS:
        lines.append(f"| `{key}` | {totals[key]} |")  # type: ignore[index]
    lines.append(f"| `token_read` lines | {report['token_reads']} |")
    lines.append(f"| module design constants | {report['module_constants']} |")
    lines.append(f"| **local values to migrate** | **{report['local_values']}** |")
    lines.append("")
    shared_totals = report["shared_totals"]  # type: ignore[index]
    lines.append("Outside the component bodies (retained-visual paint and geometry):")
    lines.append("")
    lines.append("| Category | Count |")
    lines.append("| --- | ---: |")
    for key in COUNT_KEYS:
        lines.append(f"| `{key}` | {shared_totals[key]} |")  # type: ignore[index]
    lines.append(f"| **local values to migrate** | **{report['shared_local_values']}** |")
    lines.append("")
    lines.append(f"Top {top} shared-authority files:")
    lines.append("")
    lines.append("| File | local values | roles |")
    lines.append("| --- | ---: | ---: |")
    for entry in report["shared_authority"][:top]:  # type: ignore[index]
        lines.append(
            f"| `{entry['file']}` | {entry['local_values']} | "
            f"{entry['counts']['color_role']} |"  # type: ignore[index]
        )
    lines.append("")
    lines.append(f"Top {top} components by local value count:")
    lines.append("")
    lines.append("| Component | local values | roles | states styled |")
    lines.append("| --- | ---: | ---: | --- |")
    ranked = sorted(
        report["components"],  # type: ignore[arg-type]
        key=lambda entry: (-int(entry["local_values"]), -len(entry["roles"]), str(entry["component"])),
    )
    for entry in ranked[:top]:
        states = ", ".join(entry["states"]) or "—"  # type: ignore[arg-type]
        lines.append(
            f"| `{entry['component']}` | {entry['local_values']} | "
            f"{len(entry['roles'])} | {states} |"  # type: ignore[arg-type]
        )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--format", choices=("json", "markdown"), default="json")
    parser.add_argument("--top", type=int, default=25, help="rows in the markdown table")
    parser.add_argument("--output", type=Path, help="write the JSON report here")
    parser.add_argument(
        "--check",
        type=Path,
        help="compare against a committed report and fail when local values grew",
    )
    args = parser.parse_args(argv)

    report = scan()
    if args.check:
        baseline = json.loads(args.check.read_text(encoding="utf-8"))
        grew = [
            (key, baseline["totals"].get(key, 0), report["totals"][key])  # type: ignore[index]
            for key in ("color_literal", "design_number", "motion")
            if report["totals"][key] > baseline["totals"].get(key, 0)  # type: ignore[index]
        ]
        for key in ("color_literal", "design_number", "motion"):
            was = (baseline.get("shared_totals") or {}).get(key, 0)
            now = report["shared_totals"][key]  # type: ignore[index]
            if now > was:
                grew.append((f"shared.{key}", was, now))
        if report["module_constants"] > baseline.get("module_constants", 0):
            grew.append(
                (
                    "module_constants",
                    baseline.get("module_constants", 0),
                    report["module_constants"],
                )
            )
        if grew:
            for key, was, now in grew:
                print(f"{key}: {was} -> {now}", file=sys.stderr)
            print(
                "Components gained design values of their own. Move them onto the "
                "Theme, or re-record the baseline with --output and say why.",
                file=sys.stderr,
            )
            return 1
        print("theme hardcoding inventory: no local-value category grew")
        return 0

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        print(args.output)
        return 0
    if args.format == "markdown":
        print(markdown(report, args.top), end="")
    else:
        print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
