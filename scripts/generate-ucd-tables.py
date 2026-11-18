#!/usr/bin/env python3
"""Generate `crates/nana-text/src/font/ucd_tables.rs` from Unicode Character Database files.

NanaUI compiles compact binary-search range tables directly into `nana-text`
instead of linking `icu_properties` for character properties. Sources are
fetched once per Unicode release from unicode.org, verified against pinned
SHA-256 digests, and cached under `target/ucd-cache/17.0.0/`.

Regenerate (downloads once if the cache is empty):
    python3 scripts/generate-ucd-tables.py

Regenerate from a local offline UCD mirror:
    python3 scripts/generate-ucd-tables.py --ucd-dir /path/to/ucd-17.0.0

Verify committed `ucd_tables.rs`:
    python3 scripts/generate-ucd-tables.py --check
    python3 scripts/generate-ucd-tables.py --check --ucd-dir /path/to/ucd-17.0.0
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import re
import ssl
import sys
import urllib.request
from pathlib import Path
from typing import Iterable

ROOT = Path(__file__).resolve().parents[1]
UCD_VERSION = "17.0.0"
UCD_BASE_URL = f"https://www.unicode.org/Public/{UCD_VERSION}/ucd/"
DEFAULT_OUTPUT = ROOT / "crates" / "nana-text" / "src" / "font" / "ucd_tables.rs"
DEFAULT_LOCKFILE = ROOT / "Cargo.lock"
DEFAULT_CACHE_DIR = ROOT / "target" / "ucd-cache" / UCD_VERSION

# Pinned Unicode crate versions in Cargo.lock whose character/line/bidi data
# must be reviewed against `ucd_tables.rs` (Unicode 17.0.0) on upgrade.
PINNED_UNICODE_CRATES: tuple[tuple[str, str, str], ...] = (
    ("harfrust", "0.12.0", "Unicode 17.0.0"),
    ("unicode-segmentation", "1.13.3", "Unicode 17.0.0"),
    ("unicode-bidi", "0.3.18", "Unicode 16.0.0"),
    ("unicode-linebreak", "0.1.5", "Unicode 15.0.0"),
)

# Pinned Unicode 17.0.0 UCD files and their SHA-256 digests.
UCD_FILES: tuple[tuple[str, str], ...] = (
    (
        "extracted/DerivedGeneralCategory.txt",
        "d62e5bab70ca74f099343f71224fa051cb1fdd61a1ab45c0488c44cfc0b6102e",
    ),
    (
        "DerivedCoreProperties.txt",
        "24c7fed1195c482faaefd5c1e7eb821c5ee1fb6de07ecdbaa64b56a99da22c08",
    ),
    (
        "Scripts.txt",
        "9f5e50d3abaee7d6ce09480f325c706f485ae3240912527e651954d2d6b035bf",
    ),
    (
        "PropertyValueAliases.txt",
        "64e9a5f76f7a1e8b5a47d6a1f9a26522a251208f5276bdfa1559dac7cf2e827a",
    ),
    (
        "VerticalOrientation.txt",
        "dcef09c3fb24d356b042569c328ec341efc5b53447700d799f2fb4834c3cd3cd",
    ),
    (
        "emoji/emoji-data.txt",
        "2cb2bb9455cda83e8481541ecf5b6dfda66a3bb89efa3fa7c5297eccf607b72b",
    ),
)

MAX_CODEPOINT = 0x10FFFF
SURROGATE_START = 0xD800
SURROGATE_END = 0xDFFF
OBJECT_REPLACEMENT = 0xFFFC

EXCLUDED_EMPHASIS_GCS = frozenset(
    {"Zs", "Zl", "Zp", "Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po", "Cc", "Cf", "Cn", "Cs"}
)
EXCLUDED_SCRIPTS = frozenset({"Zyyy", "Zinh", "Zzzz"})

_VERSION_FILE_RE = re.compile(r"^#\s*[A-Za-z0-9_]+-(\d+)\.(\d+)\.(\d+)\.txt\s*$")
_EMOJI_VERSION_RE = re.compile(r"^#\s*Version:\s*(\d+)\.(\d+)(?:\.(\d+))?\s*$")
_MISSING_RE = re.compile(r"^#\s*@missing:\s*([0-9A-Fa-f.]+)\s*;\s*([A-Za-z0-9_<>-]+)\s*$")


def is_surrogate(cp: int) -> bool:
    return SURROGATE_START <= cp <= SURROGATE_END


def parse_codepoint_range(token: str) -> tuple[int, int]:
    if ".." in token:
        start_s, end_s = token.split("..", 1)
        start, end = int(start_s, 16), int(end_s, 16)
    else:
        start = end = int(token, 16)
    if not (0 <= start <= end <= MAX_CODEPOINT):
        raise ValueError(f"invalid Unicode codepoint range: {token!r}")
    return start, end


def merge_codepoints_to_ranges(codepoints: Iterable[int]) -> list[tuple[int, int]]:
    ordered = sorted(cp for cp in set(codepoints) if 0 <= cp <= MAX_CODEPOINT and not is_surrogate(cp))
    if not ordered:
        return []
    ranges: list[tuple[int, int]] = []
    start = end = ordered[0]
    for cp in ordered[1:]:
        if cp == end + 1:
            end = cp
        else:
            ranges.append((start, end))
            start = end = cp
    ranges.append((start, end))
    return ranges


def pack_bool_ranges(ranges: Iterable[tuple[int, int]]) -> list[int]:
    packed: list[int] = []
    for start, end in ranges:
        if not (0 <= start <= end <= MAX_CODEPOINT) or is_surrogate(start) or is_surrogate(end):
            raise ValueError(f"invalid scalar range for packing: {start:#x}..={end:#x}")
        packed.append((start << 21) | end)
    return packed


def parse_ucd_version(texts: dict[str, str]) -> tuple[int, int, int]:
    """Derive and cross-check `(major, minor, patch)` from the UCD file headers."""
    core_versions: dict[str, tuple[int, int, int]] = {}
    for rel_path in (
        "extracted/DerivedGeneralCategory.txt",
        "DerivedCoreProperties.txt",
        "Scripts.txt",
        "PropertyValueAliases.txt",
        "VerticalOrientation.txt",
    ):
        for line in texts[rel_path].splitlines()[:10]:
            match = _VERSION_FILE_RE.match(line.strip())
            if match:
                core_versions[rel_path] = tuple(int(g) for g in match.groups())  # type: ignore[assignment]
                break

    if not core_versions:
        raise ValueError("could not find a `<Name>-X.Y.Z.txt` version header in UCD files")

    unique_versions = set(core_versions.values())
    if len(unique_versions) != 1:
        raise ValueError(f"mismatched UCD file versions: {core_versions}")
    version = next(iter(unique_versions))

    emoji_version: tuple[int, int] | None = None
    for line in texts["emoji/emoji-data.txt"].splitlines()[:25]:
        match = _EMOJI_VERSION_RE.match(line.strip())
        if match:
            emoji_version = (int(match.group(1)), int(match.group(2)))
            break
    if emoji_version is None:
        raise ValueError("could not find `# Version: X.Y` in emoji/emoji-data.txt")
    if emoji_version != version[:2]:
        raise ValueError(
            f"emoji/emoji-data.txt version {emoji_version} disagrees with UCD {version}"
        )

    return version


def parse_property_ranges(text: str, target_props: set[str]) -> set[int]:
    codepoints: set[int] = set()
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [part.strip() for part in line.split(";")]
        if len(parts) < 2 or parts[1] not in target_props:
            continue
        start, end = parse_codepoint_range(parts[0])
        for cp in range(start, end + 1):
            if not is_surrogate(cp):
                codepoints.add(cp)
    return codepoints


def parse_vertical_rotated(text: str) -> list[tuple[int, int]]:
    """Extract UAX #50 `Vo == R` ranges, respecting `# @missing` defaults."""
    default_is_r = True
    explicit_r: set[int] = set()
    explicit_non_r: set[int] = set()

    for raw in text.splitlines():
        missing = _MISSING_RE.match(raw.strip())
        if missing:
            default_is_r = missing.group(2) == "R"
            continue
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [part.strip() for part in line.split(";")]
        if len(parts) < 2:
            continue
        start, end = parse_codepoint_range(parts[0])
        target = explicit_r if parts[1] == "R" else explicit_non_r
        for cp in range(start, end + 1):
            if not is_surrogate(cp):
                target.add(cp)

    if default_is_r:
        rotated = {
            cp
            for cp in range(0, MAX_CODEPOINT + 1)
            if not is_surrogate(cp) and cp not in explicit_non_r
        }
    else:
        rotated = explicit_r - explicit_non_r

    return merge_codepoints_to_ranges(rotated)


def parse_emphasis_allowed(general_category_text: str) -> list[tuple[int, int]]:
    """Scalar codepoints that accept emphasis marks (excluding U+FFFC and Z*/P*/Cc/Cf/Cn/Cs)."""
    allowed: set[int] = set()
    for raw in general_category_text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [part.strip() for part in line.split(";")]
        if len(parts) < 2:
            continue
        gc = parts[1]
        if gc in EXCLUDED_EMPHASIS_GCS:
            continue
        start, end = parse_codepoint_range(parts[0])
        for cp in range(start, end + 1):
            if cp != OBJECT_REPLACEMENT and not is_surrogate(cp):
                allowed.add(cp)
    return merge_codepoints_to_ranges(allowed)


def parse_scripts(scripts_text: str, aliases_text: str) -> tuple[list[str], list[int]]:
    """Return `(script_tags, packed_script_ranges)` excluding Zyyy/Zinh/Zzzz."""
    aliases: dict[str, str] = {}
    for raw in aliases_text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [part.strip() for part in line.split(";")]
        if len(parts) >= 3 and parts[0] == "sc":
            short = parts[1]
            if not re.fullmatch(r"[A-Za-z]{4}", short):
                raise ValueError(f"invalid ISO 15924 script tag {short!r}")
            aliases[short] = short
            for name in parts[2:]:
                aliases[name] = short

    by_cp: dict[int, str] = {}
    for raw in scripts_text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [part.strip() for part in line.split(";")]
        if len(parts) < 2:
            continue
        script_name = parts[1]
        if script_name not in aliases:
            raise ValueError(f"unknown script {script_name!r} in Scripts.txt")
        short = aliases[script_name]
        if short in EXCLUDED_SCRIPTS:
            continue
        start, end = parse_codepoint_range(parts[0])
        for cp in range(start, end + 1):
            if not is_surrogate(cp):
                by_cp[cp] = short

    if not by_cp:
        return [], []

    ordered = sorted(by_cp.items())
    merged: list[tuple[int, int, str]] = []
    start, prev_cp, prev_tag = ordered[0][0], ordered[0][0], ordered[0][1]
    for cp, tag in ordered[1:]:
        if cp == prev_cp + 1 and tag == prev_tag:
            prev_cp = cp
        else:
            merged.append((start, prev_cp, prev_tag))
            start = prev_cp = cp
            prev_tag = tag
    merged.append((start, prev_cp, prev_tag))

    tags: list[str] = []
    tag_to_idx: dict[str, int] = {}
    packed_ranges: list[int] = []
    for start, end, tag in merged:
        idx = tag_to_idx.get(tag)
        if idx is None:
            idx = len(tags)
            if idx >= 256:
                raise ValueError("SCRIPT_TAGS exceeds 256 entries; u8 tag_idx overflows")
            tag_to_idx[tag] = idx
            tags.append(tag)
        packed_ranges.append((start << 32) | (end << 8) | idx)

    return tags, packed_ranges


def build_tables(texts: dict[str, str]) -> dict[str, object]:
    unicode_version = parse_ucd_version(texts)
    default_ignorable = pack_bool_ranges(
        merge_codepoints_to_ranges(
            parse_property_ranges(texts["DerivedCoreProperties.txt"], {"Default_Ignorable_Code_Point"})
        )
    )
    vertical_rotated = pack_bool_ranges(parse_vertical_rotated(texts["VerticalOrientation.txt"]))
    emphasis_allowed = pack_bool_ranges(parse_emphasis_allowed(texts["extracted/DerivedGeneralCategory.txt"]))
    emoji_presentation = pack_bool_ranges(
        merge_codepoints_to_ranges(
            parse_property_ranges(texts["emoji/emoji-data.txt"], {"Emoji_Presentation"})
        )
    )
    script_tags, script_ranges = parse_scripts(texts["Scripts.txt"], texts["PropertyValueAliases.txt"])
    return {
        "unicode_version": unicode_version,
        "default_ignorable": default_ignorable,
        "vertical_rotated": vertical_rotated,
        "emphasis_allowed": emphasis_allowed,
        "emoji_presentation": emoji_presentation,
        "script_tags": script_tags,
        "script_ranges": script_ranges,
    }


def _format_u64_const(name: str, values: list[int], width: int) -> list[str]:
    lines = [f"pub(super) const {name}: &[u64] = &["]
    for value in values:
        lines.append(f"    0x{value:0{width}X},")
    lines.append("];")
    return lines


def _format_script_tags(tags: list[str]) -> list[str]:
    lines = ["pub(super) const SCRIPT_TAGS: &[[u8; 4]] = &["]
    per_line = 9
    for offset in range(0, len(tags), per_line):
        chunk = ", ".join(f'*b"{tag}"' for tag in tags[offset : offset + per_line])
        lines.append(f"    {chunk},")
    lines.append("];")
    return lines


# Exact commands recorded in the generated header. Downloads into (or reuses)
# `target/ucd-cache`; pass `--ucd-dir <dir>` to read an existing UCD tree instead.
REGENERATE_COMMAND = "python3 scripts/generate-ucd-tables.py"
CHECK_COMMAND = "python3 scripts/generate-ucd-tables.py --check"


def render_rust(
    tables: dict[str, object],
    digests: dict[str, str],
    source_base_url: str | None = None,
) -> str:
    major, minor, patch = tables["unicode_version"]  # type: ignore[misc]
    version_str = f"{major}.{minor}.{patch}"
    base_url = source_base_url or f"https://www.unicode.org/Public/{version_str}/ucd/"
    pad = max(len(rel_path) for rel_path, _ in UCD_FILES)

    lines = [
        f"// @generated by scripts/generate-ucd-tables.py from Unicode {version_str} UCD (Unicode-3.0).",
        "// Do not edit by hand.",
        "//",
        f"// Regenerate: {REGENERATE_COMMAND}",
        f"// Verify:     {CHECK_COMMAND}",
        "//",
        f"// Sources ({base_url}):",
    ]
    for rel_path, _ in UCD_FILES:
        lines.append(f"//   {rel_path:<{pad}} sha256:{digests[rel_path]}")
    lines += [
        "//",
        "// Each u64 packs `(start << 21) | end` for boolean sets,",
        "// and `((start as u64) << 32) | ((end as u64) << 8) | tag_idx` for script ranges.",
        "",
        "#[cfg_attr(not(test), allow(dead_code))]",
        f"pub(super) const UNICODE_VERSION: (u8, u8, u8) = ({major}, {minor}, {patch});",
        "pub(super) const VERTICAL_INVERTED: bool = true;",
    ]
    lines += _format_u64_const("DEFAULT_IGNORABLE", tables["default_ignorable"], 11)  # type: ignore[arg-type]
    lines.append("")
    lines += _format_u64_const("VERTICAL_ROTATED", tables["vertical_rotated"], 11)  # type: ignore[arg-type]
    lines.append("")
    lines += _format_u64_const("EMPHASIS_ALLOWED", tables["emphasis_allowed"], 11)  # type: ignore[arg-type]
    lines.append("")
    lines += _format_u64_const("EMOJI_PRESENTATION", tables["emoji_presentation"], 11)  # type: ignore[arg-type]
    lines.append("")
    lines += _format_script_tags(tables["script_tags"])  # type: ignore[arg-type]
    lines.append("")
    lines += _format_u64_const("SCRIPT_RANGES", tables["script_ranges"], 14)  # type: ignore[arg-type]
    return "\n".join(lines) + "\n"


def _resolve_local_ucd_path(ucd_dir: Path, rel_path: str) -> Path:
    candidate = ucd_dir / rel_path
    if candidate.is_file():
        return candidate
    flat = ucd_dir / Path(rel_path).name
    if flat.is_file():
        return flat
    return candidate


def cache_is_complete(cache_dir: Path) -> bool:
    return all((cache_dir / rel_path).is_file() for rel_path, _ in UCD_FILES)


def load_ucd_files(
    ucd_dir: Path | None,
    cache_dir: Path,
    *,
    allow_download: bool,
    verify_sha: bool,
) -> tuple[dict[str, str], dict[str, str]]:
    texts: dict[str, str] = {}
    digests: dict[str, str] = {}
    ctx = ssl.create_default_context() if allow_download else None

    for rel_path, expected_sha in UCD_FILES:
        if ucd_dir is not None:
            path = _resolve_local_ucd_path(ucd_dir, rel_path)
            if not path.is_file():
                raise SystemExit(f"missing UCD file under --ucd-dir: {path}")
            data = path.read_bytes()
            where = str(path)
        else:
            path = cache_dir / rel_path
            if path.is_file():
                data = path.read_bytes()
                where = str(path)
            elif allow_download:
                url = UCD_BASE_URL + rel_path
                request = urllib.request.Request(url, headers={"User-Agent": "nana-ucd-tables"})
                with urllib.request.urlopen(request, context=ctx, timeout=120) as response:
                    data = response.read()
                seen = hashlib.sha256(data).hexdigest()
                if verify_sha and seen != expected_sha:
                    raise SystemExit(
                        f"{url}\n  expected sha256 {expected_sha}\n  got      sha256 {seen}"
                    )
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                where = url
            else:
                raise SystemExit(f"missing cached UCD file: {path}")

        seen = hashlib.sha256(data).hexdigest()
        if verify_sha and seen != expected_sha:
            raise SystemExit(
                f"{where}\n  expected sha256 {expected_sha}\n  got      sha256 {seen}"
            )
        texts[rel_path] = data.decode("utf-8")
        digests[rel_path] = seen

    return texts, digests


def _extract_u64_array(source: str, name: str) -> list[int]:
    match = re.search(rf"pub\(super\) const {name}: &\[u64\] = &\[(.*?)\];", source, re.S)
    if not match:
        raise ValueError(f"missing `pub(super) const {name}: &[u64]`")
    return [int(token, 16) for token in re.findall(r"0x[0-9A-Fa-f]+", match.group(1))]


def verify_table_invariants(source: str, expected_version: tuple[int, int, int] | None = None) -> list[str]:
    """Check structural, packing, sorting, merging, and surrogate invariants of `ucd_tables.rs`."""
    errors: list[str] = []

    ver_match = re.search(
        r"pub\(super\) const UNICODE_VERSION:\s*\(u8,\s*u8,\s*u8\)\s*=\s*\((\d+),\s*(\d+),\s*(\d+)\);",
        source,
    )
    if not ver_match:
        errors.append("missing `pub(super) const UNICODE_VERSION: (u8, u8, u8)`")
    else:
        parsed_ver = tuple(int(g) for g in ver_match.groups())
        if expected_version is not None and parsed_ver != expected_version:
            errors.append(f"UNICODE_VERSION is {parsed_ver}, expected {expected_version}")

    if not re.search(r"pub\(super\) const VERTICAL_INVERTED:\s*bool\s*=\s*true;", source):
        errors.append("missing `pub(super) const VERTICAL_INVERTED: bool = true;`")

    for name in ("DEFAULT_IGNORABLE", "VERTICAL_ROTATED", "EMPHASIS_ALLOWED", "EMOJI_PRESENTATION"):
        try:
            entries = _extract_u64_array(source, name)
        except ValueError as exc:
            errors.append(str(exc))
            continue
        if not entries:
            errors.append(f"{name} must not be empty")
            continue
        prev_end: int | None = None
        for entry in entries:
            start = entry >> 21
            end = entry & ((1 << 21) - 1)
            if not (0 <= start <= end <= MAX_CODEPOINT):
                errors.append(f"{name} entry {entry:#x} has out-of-range bounds {start:#x}..={end:#x}")
            elif start <= SURROGATE_END and end >= SURROGATE_START:
                errors.append(f"{name} range {start:#x}..={end:#x} intersects UTF-16 surrogates")
            if prev_end is not None and prev_end + 1 >= start:
                errors.append(
                    f"{name} ranges not strictly sorted and maximally merged: {prev_end:#x} before {start:#x}"
                )
            prev_end = end

    tags_match = re.search(r"pub\(super\) const SCRIPT_TAGS:\s*&\[\[u8;\s*4\]\]\s*=\s*&\[(.*?)\];", source, re.S)
    tags: list[str] = []
    if not tags_match:
        errors.append("missing `pub(super) const SCRIPT_TAGS: &[[u8; 4]]`")
    else:
        tags = re.findall(r'\*b"([A-Za-z]{4})"', tags_match.group(1))
        if not tags:
            errors.append("SCRIPT_TAGS must not be empty")
        if len(tags) > 256:
            errors.append(f"SCRIPT_TAGS length {len(tags)} exceeds u8 index capacity")
        if len(tags) != len(set(tags)):
            errors.append("SCRIPT_TAGS contains duplicate script tags")
        bad_scripts = EXCLUDED_SCRIPTS & set(tags)
        if bad_scripts:
            errors.append(f"SCRIPT_TAGS contains excluded generic scripts: {sorted(bad_scripts)}")

    try:
        script_ranges = _extract_u64_array(source, "SCRIPT_RANGES")
    except ValueError as exc:
        errors.append(str(exc))
    else:
        if not script_ranges:
            errors.append("SCRIPT_RANGES must not be empty")
        prev_end = None
        prev_tag: int | None = None
        seen_order: list[int] = []
        seen_set: set[int] = set()
        for entry in script_ranges:
            start = entry >> 32
            end = (entry >> 8) & 0xFF_FFFF
            tag_idx = entry & 0xFF
            if not (0 <= start <= end <= MAX_CODEPOINT):
                errors.append(f"SCRIPT_RANGES entry {entry:#x} has invalid bounds {start:#x}..={end:#x}")
            elif start <= SURROGATE_END and end >= SURROGATE_START:
                errors.append(f"SCRIPT_RANGES range {start:#x}..={end:#x} intersects UTF-16 surrogates")
            if tags and tag_idx >= len(tags):
                errors.append(f"SCRIPT_RANGES entry {entry:#x} tag_idx {tag_idx} >= len(SCRIPT_TAGS)")
            if prev_end is not None:
                if prev_end >= start:
                    errors.append(f"SCRIPT_RANGES not strictly sorted: {prev_end:#x} >= {start:#x}")
                elif prev_end + 1 == start and prev_tag == tag_idx:
                    errors.append(f"SCRIPT_RANGES adjacent ranges share tag_idx {tag_idx} at {start:#x}")
            if tag_idx not in seen_set:
                seen_set.add(tag_idx)
                seen_order.append(tag_idx)
            prev_end = end
            prev_tag = tag_idx
        if tags and seen_order != list(range(len(tags))):
            errors.append("SCRIPT_TAGS order does not match first appearance in SCRIPT_RANGES")

    return errors


def verify_output_offline(output_path: Path) -> list[str]:
    """Validate provenance header, pinned SHA-256 comments, version, and invariants offline."""
    if not output_path.is_file():
        return [f"missing generated file: {output_path}"]
    source = output_path.read_text(encoding="utf-8")
    errors: list[str] = []

    expected_header = (
        f"// @generated by scripts/generate-ucd-tables.py from Unicode {UCD_VERSION} UCD (Unicode-3.0)."
    )
    if not source.startswith(expected_header + "\n// Do not edit by hand.\n"):
        errors.append(f"missing expected `@generated` header for Unicode {UCD_VERSION}")
    if f"\n// Regenerate: {REGENERATE_COMMAND}\n" not in source:
        errors.append(f"missing `// Regenerate: {REGENERATE_COMMAND}` header line")

    for rel_path, expected_sha in UCD_FILES:
        pattern = re.compile(rf"^//\s+{re.escape(rel_path)}\s+sha256:{expected_sha}$", re.M)
        if not pattern.search(source):
            errors.append(f"missing or mismatched provenance digest for {rel_path} ({expected_sha})")

    expected_ver = tuple(int(part) for part in UCD_VERSION.split("."))
    errors.extend(verify_table_invariants(source, expected_version=expected_ver))  # type: ignore[arg-type]
    return errors


_LOCK_PACKAGE_RE = re.compile(
    r'^\[\[package\]\]\s*\nname\s*=\s*"([^"]+)"\s*\nversion\s*=\s*"([^"]+)"',
    re.M,
)


def verify_unicode_crate_alignment(lockfile_path: Path) -> list[str]:
    """Verify Cargo.lock keeps expected versions of companion Unicode crates."""
    if not lockfile_path.is_file():
        return [f"missing Cargo.lock for Unicode alignment check: {lockfile_path}"]
    text = lockfile_path.read_text(encoding="utf-8")
    versions: dict[str, list[str]] = {}
    for name, version in _LOCK_PACKAGE_RE.findall(text):
        versions.setdefault(name, []).append(version)

    errors: list[str] = []
    for crate, expected_version, unicode_ver in PINNED_UNICODE_CRATES:
        found = versions.get(crate, [])
        if found != [expected_version]:
            errors.append(
                f"Cargo.lock `{crate}` resolved {found or 'missing'}, expected "
                f"['{expected_version}'] ({unicode_ver}): re-verify Unicode property "
                "alignment across nana-text and update PINNED_UNICODE_CRATES in "
                "scripts/generate-ucd-tables.py plus docs/reference/third-party.md and "
                "docs/en/reference/third-party.md"
            )
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed file (regenerates when --ucd-dir or local cache is available; validates provenance and invariants otherwise)",
    )
    parser.add_argument(
        "--ucd-dir",
        type=Path,
        help="read UCD source files from a local directory instead of downloading",
    )
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=DEFAULT_CACHE_DIR,
        help=f"cache directory for downloaded UCD files (default: {DEFAULT_CACHE_DIR})",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help=f"output Rust file path (default: {DEFAULT_OUTPUT})",
    )
    parser.add_argument(
        "--lockfile",
        type=Path,
        default=DEFAULT_LOCKFILE,
        help=f"Cargo.lock path to verify companion Unicode crate versions (default: {DEFAULT_LOCKFILE})",
    )
    parser.add_argument(
        "--allow-unverified",
        action="store_true",
        help="skip pinned SHA-256 verification (for synthetic test fixtures or alternate UCD versions)",
    )
    args = parser.parse_args(argv)

    verify_sha = not args.allow_unverified

    if args.check:
        lock_errors = verify_unicode_crate_alignment(args.lockfile)
        if lock_errors:
            for error in lock_errors:
                print(f"invalid: {error}", file=sys.stderr)
            return 1
        if args.ucd_dir is not None or cache_is_complete(args.cache_dir):
            texts, digests = load_ucd_files(
                args.ucd_dir,
                args.cache_dir,
                allow_download=False,
                verify_sha=verify_sha,
            )
            tables = build_tables(texts)
            expected = render_rust(tables, digests)
            if not args.output.is_file():
                print(f"missing: {args.output}", file=sys.stderr)
                return 1
            actual = args.output.read_text(encoding="utf-8")
            if actual != expected:
                diff = "".join(
                    difflib.unified_diff(
                        actual.splitlines(keepends=True),
                        expected.splitlines(keepends=True),
                        fromfile=str(args.output),
                        tofile="generated",
                        n=2,
                    )
                )
                print(f"stale: {args.output}\n{diff[:4000]}", file=sys.stderr)
                return 1
            errors = verify_table_invariants(actual, expected_version=tables["unicode_version"])  # type: ignore[arg-type]
            if errors:
                for error in errors:
                    print(f"invalid: {error}", file=sys.stderr)
                return 1
            source_desc = str(args.ucd_dir) if args.ucd_dir is not None else str(args.cache_dir)
            print(f"ucd_tables OK (regenerated & diffed against {source_desc})")
            return 0

        errors = verify_output_offline(args.output)
        if errors:
            for error in errors:
                print(f"invalid: {error}", file=sys.stderr)
            return 1
        print(
            f"ucd_tables OK (offline provenance & invariants verified; cache absent at {args.cache_dir})"
        )
        return 0

    texts, digests = load_ucd_files(
        args.ucd_dir,
        args.cache_dir,
        allow_download=True,
        verify_sha=verify_sha,
    )
    tables = build_tables(texts)
    rendered = render_rust(tables, digests)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(rendered, encoding="utf-8")
    ver = tables["unicode_version"]
    print(
        f"{args.output}: Unicode {ver[0]}.{ver[1]}.{ver[2]} "
        f"(ignorable={len(tables['default_ignorable'])}, "
        f"vert_r={len(tables['vertical_rotated'])}, "
        f"emphasis={len(tables['emphasis_allowed'])}, "
        f"emoji={len(tables['emoji_presentation'])}, "
        f"scripts={len(tables['script_tags'])}/{len(tables['script_ranges'])})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
