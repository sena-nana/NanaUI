#!/usr/bin/env python3
"""Rebuild the `nana-text` corpus font fixtures.

The parity corpus must be reproducible on any machine, so it never touches
system fonts. Everything it shapes comes from a small set of subsetted OFL
faces committed under `crates/nana-text/fonts/`.

This script regenerates those files: it downloads the upstream variable fonts,
pins them to one named instance, subsets them to exactly the codepoints the
corpus uses, and writes the result. Run it only when the corpus needs a
codepoint that is not covered yet; the outputs are committed, so a normal build
and a normal test run never need it.

    python3 -m venv .venv && .venv/bin/pip install fonttools brotli
    .venv/bin/python scripts/build-text-corpus-fonts.py

Pass --check to verify the committed files match what this script would write
without overwriting them.

Two faces are synthesized from code rather than downloaded, because no
upstream face has exactly the property a font-system test needs and nothing
else:

- `nana-test-axes` has `wght`, `wdth` and `slnt` axes plus two named
  instances, so weight/stretch/style range matching and the
  `font-weight` versus explicit `"wght"` precedence rule are observable.
- `nana-test-color` has a COLR/CPAL colour table covering two emoji, so the
  emoji fallback policy can prefer a colour face over a monochrome one.

Two faces are *not* built here:

- `noto-sans-sc` is the product UI face. The corpus reads it from
  `nana_ui_core::fonts::UI_FONT_REGULAR`, so there is no second copy.
- `nana-test-vf` is the hand-built variable-axis fixture copied from
  `crates/nana-ui/src/nana_text/fixtures/nana-wdth-bevl.ttf`.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import shutil
import sys
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "crates" / "nana-text" / "fonts"
VF_SOURCE = ROOT / "crates" / "nana-ui" / "src" / "nana_text" / "fixtures" / "nana-wdth-bevl.ttf"
VF_TARGET = OUT_DIR / "nana-test-vf.ttf"

# Pinned to an immutable commit, not to `main`. Noto faces are reshipped
# routinely, and a moving ref would mean `--check` reports the committed
# fixtures stale whenever upstream moves -- and that regenerating silently
# rebases the Arabic, hangul and emoji corpus onto a different font version,
# with different glyph ids and advances, producing golden churn that looks
# exactly like an engine regression. Bump this deliberately, and re-bless the
# affected cases in the same commit.
GOOGLE_FONTS_REV = "1ac2012c34919f5fa2675aacf723fa98edb30b5f"
GOOGLE_FONTS = f"https://raw.githubusercontent.com/google/fonts/{GOOGLE_FONTS_REV}/ofl"

# Pinned `head.created` / `head.modified`, in the OpenType epoch (seconds since
# 1904-01-01). Any fixed value works; what matters is that it is fixed, so two
# runs of this script produce identical bytes and `--check` is meaningful.
EPOCH = 0


@dataclass
class Fixture:
    """One subsetted face."""

    name: str
    url: str
    license_url: str
    # SHA-256 of the upstream file. The pinned revision should make this
    # redundant, but it turns a silently swapped source into a loud failure at
    # fetch time instead of an unexplained diff at compare time.
    sha256: str
    # Unicode codepoints the corpus shapes with this face.
    unicodes: list[int]
    # Layout features to keep. The subsetter's default set is Latin-centric and
    # drops exactly the tables these rows exist to exercise.
    features: list[str]
    # Axis pins applied before subsetting, so the committed file is a single
    # static instance and goldens cannot drift with a default-instance change.
    pins: dict[str, float] = field(default_factory=dict)
    note: str = ""


# "العربية" plus a space, which is all the Arabic rows shape. The joining
# features are the point of the row: alef/lam/ain/ra/ba/ya/ta-marbuta exercise
# initial, medial, final and isolated forms plus the lam-alef ligature.
ARABIC = Fixture(
    name="noto-sans-arabic",
    url=f"{GOOGLE_FONTS}/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf",
    license_url=f"{GOOGLE_FONTS}/notosansarabic/OFL.txt",
    sha256="63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e",
    unicodes=[0x0020, 0x0627, 0x0628, 0x0629, 0x0631, 0x0639, 0x0644, 0x064A],
    features=["init", "medi", "fina", "isol", "rlig", "liga", "mark", "mkmk", "ccmp"],
    pins={"wght": 400, "wdth": 100},
    note="Arabic joining and RTL shaping.",
)

# Precomposed hangul syllables. NotoSansSC has no hangul at all.
KOREAN = Fixture(
    name="noto-sans-kr",
    url=f"{GOOGLE_FONTS}/notosanskr/NotoSansKR%5Bwght%5D.ttf",
    license_url=f"{GOOGLE_FONTS}/notosanskr/OFL.txt",
    sha256="194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252",
    unicodes=[0x0020, 0xD55C, 0xAE00, 0xC548, 0xB155, 0xD558, 0xC138, 0xC694],
    features=["ccmp", "liga", "kern", "mark"],
    pins={"wght": 400},
    note="Hangul syllables.",
)

# Monochrome Noto Emoji, not the colour CBDT build. The IR carries glyph ids and
# clusters, not pixels, so ZWJ cluster merging is fully observable from the
# outline font and the colour build's megabytes buy nothing here.
EMOJI = Fixture(
    name="noto-emoji",
    url=f"{GOOGLE_FONTS}/notoemoji/NotoEmoji%5Bwght%5D.ttf",
    license_url=f"{GOOGLE_FONTS}/notoemoji/OFL.txt",
    sha256="de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551",
    unicodes=[
        0x0020,
        0x200D,  # ZWJ
        0xFE0F,  # VS16
        0x2764,  # heavy black heart
        0x1F468,  # man
        0x1F469,  # woman
        0x1F4BB,  # laptop
        0x1F525,  # fire
    ],
    features=["ccmp", "liga", "rlig", "dlig"],
    pins={"wght": 400},
    note="Emoji and ZWJ sequences, monochrome outlines.",
)

FIXTURES = [ARABIC, KOREAN, EMOJI]


@dataclass
class Synthetic:
    """A face generated by `build_synthetic_*`, with no upstream source."""

    name: str
    note: str


AXES = Synthetic(
    name="nana-test-axes",
    note="Synthetic: `wght` 100..900, `wdth` 50..200, `slnt` -15..0, named instances "
    "Regular and Bold. U+0020 U+0041 U+0042",
)
COLOR = Synthetic(
    name="nana-test-color",
    note="Synthetic: COLR v0 + CPAL colour glyphs. U+0020 U+2764 U+1F525",
)

SYNTHETIC = [AXES, COLOR]


def _box(pen_cls, x0: int, y0: int, x1: int, y1: int):
    pen = pen_cls(None)
    pen.moveTo((x0, y0))
    pen.lineTo((x0, y1))
    pen.lineTo((x1, y1))
    pen.lineTo((x1, y0))
    pen.closePath()
    return pen.glyph()


def _finish(fb) -> bytes:
    font = fb.font
    font["head"].created = EPOCH
    font["head"].modified = EPOCH
    buffer = io.BytesIO()
    font.save(buffer)
    return buffer.getvalue()


def _base_builder(family: str, glyphs: dict, cmap: dict[int, str]):
    from fontTools.fontBuilder import FontBuilder

    order = list(glyphs)
    fb = FontBuilder(unitsPerEm=1000, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({name: (600, 0) for name in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    # A PostScript name is not optional in practice: fontdb rejects a face
    # without one as unnamed.
    fb.setupNameTable(
        {"familyName": family, "styleName": "Regular", "psName": f"{family}-Regular"}
    )
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost()
    # FontBuilder stamps "now" into head; `_finish` pins it back.
    fb.font["head"].created = EPOCH
    fb.font["head"].modified = EPOCH
    return fb


def build_synthetic_axes() -> bytes:
    from fontTools.pens.ttGlyphPen import TTGlyphPen
    from fontTools.ttLib.tables.TupleVariation import TupleVariation

    glyphs = {
        ".notdef": _box(TTGlyphPen, 50, 0, 550, 700),
        "space": TTGlyphPen(None).glyph(),
        "A": _box(TTGlyphPen, 100, 0, 500, 700),
        "B": _box(TTGlyphPen, 100, 0, 400, 700),
    }
    fb = _base_builder("NanaTestAxes", glyphs, {0x20: "space", 0x41: "A", 0x42: "B"})
    fb.setupFvar(
        axes=[
            ("wght", 100, 400, 900, "Weight"),
            ("wdth", 50, 100, 200, "Width"),
            ("slnt", -15, 0, 0, "Slant"),
        ],
        instances=[
            {"stylename": "Regular", "location": {"wght": 400, "wdth": 100, "slnt": 0}},
            {"stylename": "Bold", "location": {"wght": 700, "wdth": 100, "slnt": 0}},
        ],
    )
    # Four points plus four phantom points per box. `wght` thickens A to the
    # right, `wdth` widens it, `slnt` shears its top edge, so each axis moves
    # the outline in a way a test can tell apart.
    def deltas(right: int = 0, top_shift: int = 0):
        return [(0, 0), (top_shift, 0), (right + top_shift, 0), (right, 0), (0, 0), (0, 0), (0, 0), (0, 0)]

    fb.setupGvar(
        {
            "A": [
                TupleVariation({"wght": (0, 1.0, 1.0)}, deltas(right=200)),
                TupleVariation({"wdth": (0, 1.0, 1.0)}, deltas(right=300)),
                TupleVariation({"slnt": (-1.0, -1.0, 0)}, deltas(top_shift=150)),
            ]
        }
    )
    return _finish(fb)


def build_synthetic_color() -> bytes:
    from fontTools.pens.ttGlyphPen import TTGlyphPen

    glyphs = {
        ".notdef": _box(TTGlyphPen, 50, 0, 550, 700),
        "space": TTGlyphPen(None).glyph(),
        "heart": _box(TTGlyphPen, 50, 0, 550, 700),
        "fire": _box(TTGlyphPen, 50, 0, 550, 700),
        "layer_outer": _box(TTGlyphPen, 50, 0, 550, 700),
        "layer_inner": _box(TTGlyphPen, 150, 100, 450, 600),
    }
    fb = _base_builder(
        "NanaTestColor", glyphs, {0x20: "space", 0x2764: "heart", 0x1F525: "fire"}
    )
    fb.setupCOLR({"heart": [("layer_outer", 0)], "fire": [("layer_outer", 1), ("layer_inner", 0)]}, version=0)
    fb.setupCPAL([[(1.0, 0.0, 0.0, 1.0), (1.0, 0.6, 0.0, 1.0)]])
    return _finish(fb)


SYNTHETIC_BUILDERS = {AXES.name: build_synthetic_axes, COLOR.name: build_synthetic_color}


def fetch(url: str, sha256: str | None = None) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "nana-text-corpus"})
    with urllib.request.urlopen(request, timeout=120) as response:
        data = response.read()
    if sha256 is not None:
        seen = hashlib.sha256(data).hexdigest()
        if seen != sha256:
            raise SystemExit(
                f"{url}\n  expected sha256 {sha256}\n  got      sha256 {seen}\n"
                "The upstream source changed. This is not a stale fixture: bump "
                "GOOGLE_FONTS_REV and the fixture's sha256 deliberately, rebuild, "
                "and re-bless the corpus cases that use this face."
            )
    return data


def build(fixture: Fixture) -> bytes:
    from fontTools import subset
    from fontTools.ttLib import TTFont
    from fontTools.varLib import instancer

    # `recalcTimestamp=False` plus the pinned `head` dates below are what make
    # the output byte-reproducible; otherwise every rebuild stamps "now" into
    # `head.modified` and `--check` can never pass.
    font = TTFont(io.BytesIO(fetch(fixture.url, fixture.sha256)), recalcTimestamp=False)
    if fixture.pins and "fvar" in font:
        # `updateFontNames` matters: without it the instance keeps the variable
        # font's default-instance name, so pinning Noto Sans KR at wght 400
        # would still call itself "Noto Sans KR Thin" and the corpus would
        # select a face whose name contradicts its outlines.
        font = instancer.instantiateVariableFont(
            font, fixture.pins, inplace=True, updateFontNames=True
        )

    options = subset.Options()
    options.layout_features = fixture.features
    options.name_IDs = ["*"]
    options.name_legacy = True
    options.notdef_outline = True
    options.recalc_bounds = True
    options.drop_tables = ["DSIG"]
    options.hinting = False
    options.desubroutinize = True

    subsetter = subset.Subsetter(options=options)
    subsetter.populate(unicodes=fixture.unicodes)
    subsetter.subset(font)

    head = font["head"]
    head.created = EPOCH
    head.modified = EPOCH

    buffer = io.BytesIO()
    font.save(buffer)
    return buffer.getvalue()


def target_path(fixture: Fixture) -> Path:
    return OUT_DIR / f"{fixture.name}.ttf"


def write_manifest(built: dict[str, bytes], license_text: str) -> None:
    lines = [
        "# nana-text corpus font fixtures",
        "",
        "Regenerate with `python3 scripts/build-text-corpus-fonts.py`; verify with",
        "`--check`. These files exist so the parity corpus never depends on a font a",
        "developer happens to have installed.",
        "",
        "| File | Upstream | Contents |",
        "| --- | --- | --- |",
    ]
    for fixture in FIXTURES:
        codepoints = " ".join(f"U+{cp:04X}" for cp in fixture.unicodes)
        lines.append(
            f"| `{fixture.name}.ttf` | [{fixture.url}]({fixture.url}) | "
            f"{fixture.note} {codepoints} |"
        )
    for synthetic in SYNTHETIC:
        lines.append(f"| `{synthetic.name}.ttf` | generated by this script | {synthetic.note} |")
    lines += [
        "| `nana-test-vf.ttf` | `crates/nana-ui/src/nana_text/fixtures/nana-wdth-bevl.ttf` |"
        " Two glyphs, `wdth` plus a custom `BEVL` axis, cmap covers only U+0041. |",
        "",
        "`noto-sans-sc` is **not** here: the corpus reads the product UI face from",
        "`nana_ui_core::fonts::UI_FONT_REGULAR` rather than committing a second copy.",
        "",
        "## SHA-256",
        "",
        "```text",
    ]
    for name in sorted(built):
        lines.append(f"{hashlib.sha256(built[name]).hexdigest()}  {name}")
    lines += [
        "```",
        "",
        "## Upstream sources",
        "",
        f"Pinned to google/fonts `{GOOGLE_FONTS_REV}`. SHA-256 of each downloaded",
        "source, so the committed fixtures can always be reproduced:",
        "",
        "```text",
    ]
    for fixture in FIXTURES:
        lines.append(f"{fixture.sha256}  {fixture.name}")
    lines += [
        "```",
        "",
        "## Licence",
        "",
        "All Noto faces are licensed under the SIL Open Font",
        "License 1.1; see `OFL.txt`. The synthetic faces are generated here and",
        "carry no third-party outlines.",
        "",
    ]
    (OUT_DIR / "README.md").write_text("\n".join(lines), encoding="utf-8")
    (OUT_DIR / "OFL.txt").write_text(license_text, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed fixtures match this script's output",
    )
    args = parser.parse_args()

    try:
        import fontTools  # noqa: F401
    except ImportError:
        print(
            "fonttools is required: python3 -m venv .venv && "
            ".venv/bin/pip install fonttools brotli",
            file=sys.stderr,
        )
        return 2

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    built: dict[str, bytes] = {}
    failures: list[str] = []

    for fixture in FIXTURES:
        data = build(fixture)
        built[f"{fixture.name}.ttf"] = data
        path = target_path(fixture)
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                failures.append(str(path.relative_to(ROOT)))
        else:
            path.write_bytes(data)
            print(f"{path.relative_to(ROOT)}: {len(data)} bytes")

    for synthetic in SYNTHETIC:
        data = SYNTHETIC_BUILDERS[synthetic.name]()
        built[f"{synthetic.name}.ttf"] = data
        path = OUT_DIR / f"{synthetic.name}.ttf"
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                failures.append(str(path.relative_to(ROOT)))
        else:
            path.write_bytes(data)
            print(f"{path.relative_to(ROOT)}: {len(data)} bytes")

    vf = VF_SOURCE.read_bytes()
    built[VF_TARGET.name] = vf
    if args.check:
        if not VF_TARGET.exists() or VF_TARGET.read_bytes() != vf:
            failures.append(str(VF_TARGET.relative_to(ROOT)))
    else:
        shutil.copyfile(VF_SOURCE, VF_TARGET)
        print(f"{VF_TARGET.relative_to(ROOT)}: {len(vf)} bytes")

    if args.check:
        if failures:
            print("Corpus font fixtures are stale:", file=sys.stderr)
            for name in failures:
                print(f"- {name}", file=sys.stderr)
            return 1
        print(f"Corpus font fixtures: OK ({len(built)} files)")
        return 0

    write_manifest(built, fetch(ARABIC.license_url).decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
