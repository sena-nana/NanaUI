#!/usr/bin/env python3
"""Rebuild the bundled UI face, `crates/nana-ui/assets/fonts/NotoSansSC-Variable.ttf`.

NanaUI ships one Noto Sans SC file: the upstream variable font with its `wght`
axis narrowed to 400..700 (default 400, the Regular instance), subsetted to the
characters the framework and its consumers draw. One file covers every UI
weight on a real axis, so `nana-text` never synthesizes bold and a host pays
for one copy of the outlines instead of four.

The character set is the union of

- `scripts/ui-font/base-codepoints.txt`: the coverage the four static weights
  had before this file replaced them, frozen;
- JIS X 0208 level 1, so Japanese interface text has the everyday kanji the
  simplified-Chinese coverage lacks (computed from Python's `euc_jp` codec);
- `scripts/ui-font/ui-text-codepoints.txt`: every character the interface text
  of the framework and its consumers uses. Refresh it with `--scan`, passing the
  consumers' locale directories; the framework's own built-in strings
  (`crates/nana-ui-core/src/framework_strings.rs`) are always scanned.

Codepoints the upstream face does not have are reported and skipped.

    python3 -m venv .venv && .venv/bin/pip install fonttools
    .venv/bin/python scripts/build-ui-font.py
    .venv/bin/python scripts/build-ui-font.py --scan ../NanaLive/crates/nanalive-control/locales

Pass `--check` to verify the committed face matches what this script would
write. `--source PATH` reads a local copy of the upstream file instead of
downloading it; its SHA-256 is still verified.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA_DIR = ROOT / "scripts" / "ui-font"
BASE_CODEPOINTS = DATA_DIR / "base-codepoints.txt"
UI_TEXT_CODEPOINTS = DATA_DIR / "ui-text-codepoints.txt"
FRAMEWORK_STRINGS = ROOT / "crates" / "nana-ui-core" / "src" / "framework_strings.rs"
OUT_DIR = ROOT / "crates" / "nana-ui" / "assets" / "fonts"
OUT_FONT = OUT_DIR / "NotoSansSC-Variable.ttf"
OUT_LICENSE = OUT_DIR / "OFL.txt"
OUT_MANIFEST = OUT_DIR / "README.md"

# Pinned to an immutable google/fonts commit (the same one the nana-text corpus
# fixtures use). The file there is Noto Sans SC v2.004, `wght` 100..900.
GOOGLE_FONTS_REV = "1ac2012c34919f5fa2675aacf723fa98edb30b5f"
SOURCE_URL = (
    f"https://raw.githubusercontent.com/google/fonts/{GOOGLE_FONTS_REV}"
    "/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf"
)
SOURCE_SHA256 = "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da"
LICENSE_URL = (
    f"https://raw.githubusercontent.com/google/fonts/{GOOGLE_FONTS_REV}/ofl/notosanssc/OFL.txt"
)
LICENSE_SHA256 = "1c05c68c34f9708415aada51f17e1b0092d2cea709bf4a94cd38114f9e73d7d9"

# The UI weights. 400 is the default instance so that a reader that ignores
# variations (the SVG rasterizer's fontdb, `usWeightClass` matching) still gets
# Regular, and the named instances stay what the old static files were.
WGHT_MIN, WGHT_DEFAULT, WGHT_MAX = 400, 400, 700

# The layout features the static faces kept. The subsetter's default list is
# Latin-centric and would drop the CJK vertical and proportional features.
LAYOUT_FEATURES = [
    # GSUB
    "ccmp",
    "liga",
    "locl",
    "vert",
    "vrt2",
    # GPOS
    "halt",
    "kern",
    "mark",
    "palt",
    "vhal",
    "vkrn",
    "vpal",
]

_RANGE = re.compile(r"^U\+([0-9A-F]{4,6})(?:\.\.U\+([0-9A-F]{4,6}))?$")


def read_codepoints(path: Path) -> set[int]:
    codepoints: set[int] = set()
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        match = _RANGE.match(line)
        if match is None:
            raise SystemExit(f"{path}:{number}: expected U+XXXX or U+XXXX..U+YYYY, got {raw!r}")
        start = int(match.group(1), 16)
        end = int(match.group(2) or match.group(1), 16)
        codepoints.update(range(start, end + 1))
    return codepoints


def format_codepoints(codepoints: set[int]) -> list[str]:
    lines: list[str] = []
    ordered = sorted(codepoints)
    start = end = ordered[0]
    for cp in ordered[1:]:
        if cp == end + 1:
            end = cp
            continue
        lines.append(f"U+{start:04X}" if start == end else f"U+{start:04X}..U+{end:04X}")
        start = end = cp
    lines.append(f"U+{start:04X}" if start == end else f"U+{start:04X}..U+{end:04X}")
    return lines


def jis_x0208_level1() -> set[int]:
    """Kanji rows 16..47 of JIS X 0208 (the level-1 set), via EUC-JP."""

    kanji: set[int] = set()
    for row in range(16, 48):
        for cell in range(1, 95):
            try:
                kanji.add(ord(bytes([row + 0xA0, cell + 0xA0]).decode("euc_jp")))
            except UnicodeDecodeError:
                pass
    return kanji


def text_codepoints(text: str) -> set[int]:
    # Printable characters only: the font has no business with controls, and
    # a stray BOM or tab in a locale file must not pull glyphs in.
    return {ord(ch) for ch in text if ord(ch) >= 0x20 and ord(ch) not in (0x7F, 0xFEFF)}


def scan(paths: list[Path]) -> set[int]:
    found = text_codepoints(FRAMEWORK_STRINGS.read_text(encoding="utf-8"))
    for path in paths:
        files = sorted(path.rglob("*.ftl")) if path.is_dir() else [path]
        if not files:
            raise SystemExit(f"{path}: no .ftl files")
        for file in files:
            found |= text_codepoints(file.read_text(encoding="utf-8"))
    return found


def write_ui_text_codepoints(codepoints: set[int], sources: list[Path]) -> None:
    header = [
        "# Every character the interface text of NanaUI and its consumers uses.",
        "# Generated by `scripts/build-ui-font.py --scan`; the scan unions what is",
        "# already here, so a consumer's characters stay covered when another",
        "# consumer refreshes the list. Sources scanned so far:",
        "#   crates/nana-ui-core/src/framework_strings.rs",
    ]
    known = []
    if UI_TEXT_CODEPOINTS.exists():
        known = [
            line[4:]
            for line in UI_TEXT_CODEPOINTS.read_text(encoding="utf-8").splitlines()
            if line.startswith("#   ") and "framework_strings.rs" not in line
        ]
    for source in sources:
        label = source.resolve().as_posix()
        # Record consumer sources by repository-relative spelling where
        # possible; absolute machine paths would make the file churn.
        for anchor in ("NanaLive/", "NanaType/", "NanaUI/"):
            if anchor in label:
                label = anchor + label.split(anchor, 1)[1]
                break
        if label not in known:
            known.append(label)
    header += [f"#   {label}" for label in sorted(known)]
    UI_TEXT_CODEPOINTS.write_text("\n".join(header + format_codepoints(codepoints)) + "\n", encoding="utf-8")


def fetch(url: str, sha256: str, local: Path | None = None) -> bytes:
    if local is not None:
        data = local.read_bytes()
    else:
        request = urllib.request.Request(url, headers={"User-Agent": "nana-ui-font"})
        with urllib.request.urlopen(request, timeout=300) as response:
            data = response.read()
    seen = hashlib.sha256(data).hexdigest()
    if seen != sha256:
        raise SystemExit(
            f"{local or url}\n  expected sha256 {sha256}\n  got      sha256 {seen}\n"
            "The upstream source changed. Bump GOOGLE_FONTS_REV and the sha256 "
            "deliberately, rebuild, and re-record the ui-snapshots baselines."
        )
    return data


def build(source: bytes, wanted: set[int]) -> tuple[bytes, set[int], set[int]]:
    from fontTools import subset
    from fontTools.ttLib import TTFont
    from fontTools.varLib import instancer

    font = TTFont(io.BytesIO(source), recalcTimestamp=False)
    cmap = set(font.getBestCmap())
    kept = wanted & cmap
    missing = wanted - cmap

    options = subset.Options()
    options.layout_features = LAYOUT_FEATURES
    options.name_IDs = ["*"]
    options.name_legacy = True
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.recalc_bounds = True
    options.drop_tables = ["DSIG"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(unicodes=kept)
    subsetter.subset(font)

    # Narrow the axis and move its default from Thin (100) to Regular, after
    # subsetting (the instancer is far faster on the subset, and it leaves
    # lazily loaded `gvar` entries the subsetter then cannot find). With
    # `updateFontNames` the default instance is named "Noto Sans SC Regular"
    # rather than keeping the upstream "Thin" names.
    font = instancer.instantiateVariableFont(
        font,
        {"wght": instancer.AxisTriple(WGHT_MIN, WGHT_DEFAULT, WGHT_MAX)},
        inplace=True,
        updateFontNames=True,
    )

    font["OS/2"].usWeightClass = WGHT_DEFAULT

    buffer = io.BytesIO()
    font.save(buffer)
    return buffer.getvalue(), kept, missing


def manifest(font: bytes, kept: set[int], missing: set[int], base: set[int]) -> str:
    from fontTools.ttLib import TTFont

    face = TTFont(io.BytesIO(font), lazy=True)
    added = sorted(kept - base)
    added_cjk = [cp for cp in added if 0x3400 <= cp <= 0x9FFF or 0xF900 <= cp <= 0xFAFF]
    added_other = [cp for cp in added if cp not in set(added_cjk)]
    lines = [
        "# Bundled UI font",
        "",
        "`NotoSansSC-Variable.ttf` is the one face `nana_ui_core::fonts` embeds. Regenerate",
        "it with `python3 scripts/build-ui-font.py`; verify with `--check`.",
        "",
        f"- Upstream: Noto Sans SC v2.004, google/fonts `{GOOGLE_FONTS_REV}`,",
        f"  `{SOURCE_URL}`",
        f"- Upstream SHA-256: `{SOURCE_SHA256}`",
        f"- Axis: `wght` {WGHT_MIN}..{WGHT_MAX}, default {WGHT_DEFAULT}; named instances "
        + ", ".join(
            f"{face['name'].getDebugName(inst.subfamilyNameID)} ({inst.coordinates['wght']:g})"
            for inst in face["fvar"].instances
        ),
        f"- Glyphs: {face['maxp'].numGlyphs}; codepoints: {len(kept)}",
        f"- Layout features kept: {', '.join(f'`{tag}`' for tag in LAYOUT_FEATURES)}",
        f"- SHA-256: `{hashlib.sha256(font).hexdigest()}`",
        "",
        "Character set: `scripts/ui-font/base-codepoints.txt` (the coverage of the",
        "former four static weights), JIS X 0208 level 1, and",
        "`scripts/ui-font/ui-text-codepoints.txt` (interface text of NanaUI and its",
        "consumers).",
        "",
        f"Added over the former static coverage: {len(added_cjk)} ideographs and "
        f"{len(added_other)} other characters:",
        "",
        "```text",
        "".join(chr(cp) for cp in added_other),
        "```",
        "",
    ]
    if missing:
        lines += [
            "Requested but absent from the upstream face, so not covered (a fallback",
            "face draws them):",
            "",
            "```text",
            " ".join(f"U+{cp:04X}" for cp in sorted(missing)),
            "```",
            "",
        ]
    lines += [
        "## Licence",
        "",
        "SIL Open Font License 1.1; `OFL.txt` is the upstream licence file.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--check", action="store_true", help="verify the committed files only")
    parser.add_argument("--source", type=Path, help="local copy of the upstream variable font")
    parser.add_argument(
        "--scan",
        type=Path,
        nargs="+",
        default=[],
        help="locale directories or .ftl files whose characters to add to ui-text-codepoints.txt",
    )
    args = parser.parse_args()

    if args.scan:
        if args.check:
            raise SystemExit("--scan rewrites ui-text-codepoints.txt; it cannot be combined with --check")
        known = read_codepoints(UI_TEXT_CODEPOINTS) if UI_TEXT_CODEPOINTS.exists() else set()
        write_ui_text_codepoints(known | scan(args.scan), args.scan)

    base = read_codepoints(BASE_CODEPOINTS)
    ui_text = read_codepoints(UI_TEXT_CODEPOINTS)
    framework = scan([])
    if not framework <= ui_text:
        raise SystemExit(
            "framework_strings.rs uses characters missing from ui-text-codepoints.txt: "
            + "".join(chr(cp) for cp in sorted(framework - ui_text))
            + "\nRun with --scan (and the consumers' locale directories) to refresh it."
        )
    wanted = base | jis_x0208_level1() | ui_text

    source = fetch(SOURCE_URL, SOURCE_SHA256, args.source)
    font, kept, missing = build(source, wanted)
    if not base <= kept:
        raise SystemExit("the upstream face lost coverage the bundled face had")
    license_text = fetch(LICENSE_URL, LICENSE_SHA256)
    readme = manifest(font, kept, missing, base).encode("utf-8")

    outputs = {OUT_FONT: font, OUT_LICENSE: license_text, OUT_MANIFEST: readme}
    if args.check:
        stale = [path for path, data in outputs.items() if not path.exists() or path.read_bytes() != data]
        for path in stale:
            print(f"stale: {path.relative_to(ROOT)}", file=sys.stderr)
        return 1 if stale else 0
    for path, data in outputs.items():
        path.write_bytes(data)
    print(f"{OUT_FONT.relative_to(ROOT)}: {len(font)} bytes, {len(kept)} codepoints")
    if missing:
        print("not in upstream: " + " ".join(f"U+{cp:04X}" for cp in sorted(missing)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
