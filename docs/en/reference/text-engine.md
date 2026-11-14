# Text engine

This page is for people changing NanaUI text. If you are writing an application, you do not need it.

Product text **measurement and drawing are both NanaUI's own now**. `nana-text` does layout. `SceneWgpuPainter::text` draws. Neither `cosmic-text` nor `cryoglyph` is in the release dependencies anymore. That was #99.

Epic #88 moves the text capabilities off the `cosmic-text` and `cryoglyph` forks.

#89 is its Phase 0. It first pins down the internal contract, a reference backend, and a correctness corpus. Every later phase can then compare against the same structured baseline, instead of losing comparability while the shaping, layout, and GPU layers all change at once.

#90 is Phase 1. `nana-text` owns its font layer. It does registration, generations, matching, and variation coordinates, plus coverage-based fallback. See the "Font layer" section.

#91 is Phase 2. It does segmentation, BiDi, HarfRust shaping, and the ShapeRun cache. See the "Shaping" section.

#92 is Phase 3. It does the single-line Label fast path, line breaking, in-line visual order, line box metrics, alignment, ellipsis, and the layout cache. See the "Layout" section.

#95 is Phase 4: retained text nodes in UiWorld. It has a graded dirty graph and a retained `TextLayout`. See the "UiWorld retained text nodes" section.

#96 is Phase 5: the Editable path. It has editable storage, caret, selection, hit-test, and IME composition, plus editor geometry invalidated per paragraph. See the "Editable path" section.

#97 is Phase 6: `SceneWgpuPainter::text`. The renderer owns the glyph IR, the rasterization boundary, the raster cache, the GPU atlas, the upload queue, and the text pipeline. With it, cryoglyph leaves the product path. See the "SceneWgpuPainter::text" section.

#99 is Phase 8: the full-path cutover. Runtime's `NanaTextShaper` and the painter both ask the same process-wide `nana-text` engine. Canvas2D switched too. `Cargo.lock` no longer contains cosmic-text, not even as a dev edge. See the "Product path after the cutover" section.

## What this is

```text
corpus/cases/TX-*.json          input: text + style + constraints + probes
        │
        ↓  nana-text (shaping + layout)
   Nana IR TextLayout
        │
        ↓  parity::compare(golden, actual, &tol) -> Vec<LayoutDelta>
corpus/golden/TX-*.layout.json  checked-in baseline (recorded by the cosmic reference engine in Phase 0)
```

`compare` is the one structured diff. Phase 0 compared "reference engine against golden". The native engine plugs into **the same function** and compares "native against golden".

**The reference engine was deleted together with cosmic-text.** See "Where the reference engine went". The goldens stayed. Two test cases, `shaping_matches_the_cosmic_reference_goldens.rs` and `layout_matches_the_cosmic_reference_goldens.rs`, now run the native engine against them.

"cosmic reference" in those file names refers to **where these goldens came from**, not to an engine that still runs.

Product text **measurement** goes through `crates/nana-ui/src/nana_text.rs`, which is now only a shell around `NanaTextEngineShaper`. **Drawing** goes through `crates/nana-ui/src/scene_paint/text/`. Both ask the single process-wide engine in `crates/nana-ui/src/text_engine.rs`.

## What nana-text owns, and what stays on mature crates

`nana-text` owns the lifecycle, the IR, the cache contract, and the orchestration. It **does not rewrite standard heavyweight algorithms**. The table shows who owns what.

| Concern | Owner | Reason |
| --- | --- | --- |
| Text IR (`TextKind` / `TextSource` / `TextStyle` / `TextConstraints` / `ShapedRun` / `LineBox` / `TextLayout`) | **nana-text** | Long-lived cross-layer ABI; cannot be a third-party internal type |
| Stable generational IDs and invalidation rules (`FontId` / `ShapeRunId` / `TextLayoutId` / `TextRevision` / `FontGeneration`) | **nana-text** | The authority for cache correctness |
| Hit testing / caret / selection rectangles | **nana-text** | Pure functions over the IR; if they cannot be written, the IR is missing a field |
| Line layout orchestration (line-breaking policy, line box merging, alignment, ellipsis, layout cache) | **nana-text** | Product semantics and cache contract; must share one vocabulary with `TextConstraints` |
| Structured diff and tolerances | **nana-text** | The migration acceptance contract; it had to outlive cosmic, and it did: the reference engine is gone, the goldens and `compare` remain |
| Typography vocabulary (variation axes / kerning / line-break / word-break / text-align / direction / writing-mode / wrap-break / line-height / feature) | **nana-ui-core** | Already backend-neutral tokens; reinventing them would only grow a lossy converter at the UiWorld seam |
| OpenType table parsing, glyph outlines, variation interpolation | **Mature crates** (skrifa / ttf-parser) | #89 explicitly lists reimplementing these as a non-goal |
| Complex script shaping (GSUB/GPOS, Arabic joining, Indic reordering) | **Mature crate** (harfrust, only in `shaping/opentype.rs`) | Same |
| Unicode BiDi algorithm (P–I rules, L2 reordering) | **Mature crate** (unicode-bidi, only in `shaping/bidi.rs`) | Same |
| UAX #14 line-break opportunities | **Mature crate** (unicode-linebreak, only in `layout/breaks.rs`) | #92 explicitly lists building our own line-break tables as a non-goal |
| Segmentation, span normalization, fallback retry, ShapeKey and ShapeRun cache, `ShapedText` | **nana-text** (`shaping` module) | The authority on when to reshape and who may reuse a shaping result |
| Unicode algorithms (BiDi, line breaking, grapheme clusters, script / emoji properties) | **Mature crates** (unicode-bidi / unicode-linebreak / unicode-segmentation / icu_properties) | Same |
| Font registration, generations, `FontId` issuance, face matching, fallback policy and candidates, coverage cache, variation coordinate resolution | **nana-text** (`font` module) | The authority for cache invalidation and for "why this font was used"; cannot be handed to a third-party query |
| System font directory scanning, name / OS/2 metadata reading | **Mature crate** (fontdb, only in `font/discovery.rs`) | Its query and fallback are not used |
| Axes, named instances, color tables, cmap reading | **Mature crate** (skrifa, only in `font/face.rs`) | Same read-fonts line as harfrust 0.12 in Phase 2 |
| Glyph placement (`TextLayout::glyph_placements`), raster keys, CPU glyph rasterization (`raster` feature) | **nana-text** (`layout/placement.rs`, `raster` module) | Everyone who draws text must use the same pen positions and rounding, and rasterization must use the font instance from layout; see the "Glyph placement and CPU rasterization" section |
| Glyph IR, raster cache, atlas policy and lifecycle, uploads, text pipeline | **SceneWgpuPainter::text** (`scene_paint/text/` in `nana-ui`) | #97: a renderer-side contract, see the next section; it does not belong in `nana-text` because it is device state |
| Glyph outline rasterization | **Mature crate** (swash, only behind `raster/outline.rs` in `nana-text`) | #97 explicitly lists rewriting a TrueType rasterizer as a non-goal |
| Rectangle packing | **Mature crate** (etagere, only behind `scene_paint/text/atlas.rs`) | Same; Nana owns the atlas policy, not the packing algorithm |

`nana-text` may import only these `nana_ui_core` items: `DirSpec`, `FontFeatureSetting`, `FontKerningSpec`, `FontVariationSetting`, `LineBreakSpec`, `LineHeightSpec`, `TextAlignSpec`, `TextWrapBreak`, `WordBreakSpec`, `WritingModeSpec`. The full list is `NANA_TEXT_CORE_ALLOWLIST` in `scripts/check-engine-boundary.py`. One entry, `nana_ui_core::sort`, is not vocabulary: it is the one sorting routine the framework's call sites share, and a second copy here would bring back the 6 KiB per call site it removes.

Anything else is a boundary violation, for example `LayoutStyle`, `ComputedStyle`, semantic colors, or geometry. See "How the boundary is enforced by machine".

There are two exceptions, defined locally. `TextRect` is one. `LogicalRect` cannot `Serialize`, and its constructor clamps width and height to `>= 0`, which would silently flatten regressions in RTL or degenerate bounds. `TextScale` is the other. Core has no fractional device scale for text.

## IDs and invalidation

Handles are all `{ index, generation }`. `generation == 0` is NULL. This follows the `MotionHandle` convention.

When a slot is recycled, it is reissued with a higher generation. Old handles are therefore rejected rather than aliased to new content.

| Handle | Issuer | When it becomes invalid | How it is detected |
| --- | --- | --- | --- |
| `FontId` | font db | `FontGeneration` bump, or the slot is recycled and reused | Slot generation mismatch |
| `ShapeRunId` | shaper | Any change to source bytes, `TextStyle`, script/direction segmentation, or `FontGeneration` | Slot generation mismatch |
| `TextLayoutId` | layout | Any change to `TextRevision` / `TextStyle` / `TextConstraints` / `FontGeneration` | Slot generation mismatch |
| `TextRevision` | `TextSource` | Monotonically increasing; every mutating method bumps it | `layout.revision != source.revision()` |
| `FontGeneration` | engine | Bumped on every change to the font collection | `layout.font_generation != engine.font_generation()` |

Two load-bearing invariants:

- `TextSource` fields are private. The only way to change the bytes is a method that bumps the revision. There is no bypass.
  That is what makes the revision worth using as a cache key.
- `TextLayout` carries the revision and generation it was produced from, so a staleness check is O(1)
  (`TextLayout::is_stale`). There is no need to re-fingerprint the whole string the way today's `ShapedLayoutKey` does.

`TextRevision` is **only comparable within a single `TextSource`**: two unrelated sources can both sit at 7. A cache key must
pair it with the caller's own node identity.

## Corpus and goldens

`crates/nana-text/corpus/cases/TX-*.json` is the input. `corpus/golden/TX-*.layout.json` is the checked-in
baseline. The id prefix is the category:

| Prefix | Covers |
| --- | --- |
| `TX-L*` | Latin, kerning |
| `TX-C*` | Mixed Han and kana |
| `TX-K*` | Hangul |
| `TX-M*` | Combining marks |
| `TX-G*` | Ligatures (`liga` on / off) |
| `TX-B*` | Arabic joining and pure RTL, mixed BiDi (base ltr / base rtl) |
| `TX-Z*` | Emoji, ZWJ sequences, variation selectors |
| `TX-F*` | Multi-font fallback, missing glyphs |
| `TX-V*` | Variable font `wdth` and the custom `BEVL` axis |
| `TX-W*` | Single line, explicit line breaks, word wrap, glyph wrap, max-lines + ellipsis |
| `TX-D*` | Fractional DPI (1.25 / 1.5) |
| `TX-E*` | IME preedit |

`status` is `pass` or `ignore`. `ignore` must also fill in `gap` and `gap_note`, or
`corpus_is_wellformed.rs` turns red. `parity::CATEGORIES` lists every category #89 requires and asserts that each category
has at least one passing case. A requirement that silently lost its last case would look exactly like a passing requirement.

**The way to re-record went away with the reference engine.** The old `NANA_TEXT_BLESS=1 cargo test -p nana-text
--test text_parity_corpus` re-recorded the goldens with cosmic. That is exactly what had to stop. The goldens are therefore **frozen
Phase 0 evidence**: they record what the replaced engine answered at the time. The native engine runs against them on every CI run.

To re-record, you first have to decide what a new baseline represents ("the native engine's current output" is no longer evidence of "consistent before and after migration").
That is a deliberate design decision, not an environment variable. `parity::write_golden` still exists. It is not wired to any engine.

### Determinism through a hermetic font library

The product path uses a process-wide `FontSystem` seeded from system fonts. Goldens recorded against it only hold on one machine.
Reconciliation therefore never touches it: `tests/support/corpus.rs` installs a hermetic font set for each case, loading only the fixtures the case declares,
in the order it declares them. Face ids are therefore deterministic, and the fallback chain is the case's font list. Missing-glyph cases come for free
(with only `nana-test-vf` installed, everything except `A` is `.notdef`).

### Font fixtures

`crates/nana-text/fonts/`. The Noto subsets are OFL. The synthetic fonts are generated by a script. About 20 KB in total:

| fixture | Source | Covers |
| --- | --- | --- |
| `noto-sans-sc` | `nana_ui_core::fonts::UI_FONT_REGULAR`, **no extra copy** | Latin, combining marks, f ligatures, kerning, kana, Han |
| `nana-test-vf.ttf` | Copy of `crates/nana-ui/src/nana_text/fixtures/nana-wdth-bevl.ttf`, 1 260 B | `wdth` + custom `BEVL` axis; cmap has only U+0041, doubling as the fallback and missing-glyph fixture |
| `noto-sans-arabic.ttf` | Noto Sans Arabic subset | Arabic joining (`init`/`medi`/`fina`/`rlig`/`mark`) |
| `noto-sans-kr.ttf` | Noto Sans KR subset | Hangul syllables |
| `noto-emoji.ttf` | Noto Emoji (**monochrome outline**) subset | Emoji and ZWJ ligatures |
| `nana-test-axes.ttf` | Synthesized by the script with fontTools, not downloaded | `wght` 100..900 / `wdth` 50..200 / `slnt` -15..0 + two named instances; range matching in the font layer and the priority of `font-weight` over `wght` |
| `nana-test-color.ttf` | Synthesized by the script with fontTools, not downloaded | COLR v0 + CPAL, covering U+2764 / U+1F525; emoji fallback prefers the color face |
| `nana-test-notdef.ttf` | Synthesized by the script with fontTools, not downloaded | cmap has `A` / `B`, but GSUB `ccmp` replaces `B` with `.notdef`: coverage says it can draw, shaping cannot; tests the #91 fallback retry specifically |

Regenerate with `python3 scripts/build-text-corpus-fonts.py`. `--check` verifies that the checked-in files match what the script
produces. This only means something because both ends are pinned: the **input** is pinned to a google/fonts commit, and each source file's
SHA-256 is recorded on its `Fixture` (`main` moves; Noto republishes). The **output** pins the `head` timestamp,
so the same input always yields the same bytes.

When a source file does not match, the script fails at the download and says explicitly that this is not "the checked-in files are stale".
Upstream changed versions. You must explicitly bump `GOOGLE_FONTS_REV` and the matching `sha256`, regenerate, and re-bless the corpus
cases that use that font in the same commit. Otherwise the Arabic / Hangul / emoji corpus silently moves to another font version, and the golden changes
look exactly like an engine regression.

Color emoji (`COLR` / `CBDT`) is **permanently** outside this IR: the IR carries glyph ids and clusters. Coloring is
the painter's job. The monochrome Noto Emoji `ccmp` carries the ZWJ ligature rules, which is enough to verify ZWJ cluster merging.
In the `TX-Z01` golden, `👩‍💻` is **one** glyph, with a cluster covering 0..11.

## Tolerances

```rust
pub const DEFAULT_TOLERANCES: Tolerances = Tolerances {
    advance_px: 0.05, offset_px: 0.05, baseline_px: 0.05,
    line_height_px: 0.05, bounds_px: 0.5, caret_x_px: 0.5,
};
```

- **0.05 px** (advance / offset / baseline / line-height): f32 noise at a 24 px font size is about 2e-6 px.
  This is four orders of magnitude higher, so it does not flake. It is also only 1/5 of the 0.25 px subpixel quantum a glyph atlas commonly uses, so a real advance
  rounding change still turns red. Round numbers like 0.1 / 0.5 start hiding half-subpixel drift.
- **0.5 px** (bounds / caret_x): both are derived quantities. Two engines can legitimately disagree on "advance width or ink width",
  so these stay a sanity check.
- **Do not reuse `DEFAULT_TOLERANCE_PX = 2.0` from `tools/css-parity`**: that is the magnitude of the CSS-vs-WKWebView box model.
  2 px can swallow a whole kern pair.

**Exact, zero-tolerance comparison**: `glyph_id`, `cluster`, `cluster_end`, `flags`, `bidi_level`, `direction`,
`script`, all counts, `LineBox::source`, `break_cause`, `CaretPosition::{byte,line}`,
`Affinity`, `OverflowFlags`.

`font` is compared by **equivalence class within a layout**, not by raw `FontId`: it asserts that runs sharing one face in expected
still share one in actual (a bijection both ways), and that the number of distinct faces is the same. Comparing raw index/generation would fail the harness whenever
font registration order changes, and that is not text behavior.

`id` / `revision` / `font_generation` are **not compared**: they say where a layout came from, not what it says.
Two engines have no reason to agree on them.

The diff is positional, so **counts are compared first, and on a mismatch it does not descend**: when line / run / glyph counts disagree,
it emits one `count` delta at that level and stops. Otherwise one extra glyph would produce hundreds of misaligned deltas and the report would be unreadable.

```text
TX-B02 FAIL (3 deltas)
  line 0 run 1 glyph 3 glyph_id: expected 74 got 7764
  line 0 run 1 advance_px: expected 41.50 got 42.10 (Δ=0.60, tol=0.05)
  line 1 metrics.baseline_y_px: expected 55.00 got 54.50 (Δ=-0.50, tol=0.05)
```

## Counters

`nana_text::TextWorkCounters` reserves the five measures #89 requires. The field convention copies
`nana_ui_core::work::WorkCounters`: a `usize` is something that pass must measure. An `Option<usize>` is not observed yet.
**It must stay `None` rather than pretend to be 0**. A fake zero reads like "this work did not happen" when it actually means "nobody looked".

```text
text_nodes_considered   usize           nodes looked at (including ones skipped as unchanged)
text_nodes_shaped       usize           of those, the ones that actually went into the shaper
shape_cache_hits/misses Option<usize>   Phase 0 has no shape cache → None
layout_cache_hits/misses Option<usize>  same
glyphs_resolved         Option<usize>   number of glyph ids resolved
```

Phase 0 has no product producers. That is by design. What kept them from becoming decoration back then was the reference engine's reconciliation case
(`counters.glyphs_resolved == Some(layout.glyph_count())`, comparing the counter with the output it claims to describe).
That case was deleted with the reference engine. The goldens still record the counters, so a counter change is still a reviewable diff.
On the product side, the measures are guarded by the counters of the UiWorld text pass (see "UiWorld retained text nodes").

Folding them into `WorkCounters` is a function at the UiWorld seam. That is the right time to do it. Adding five fields without producers now
would widen a contract that CI judges, for zero signal. From Phase 4 on they are produced by the UiWorld text pass, with added measures that explain
the cost of zero-work frames. See the counters subsection of "UiWorld retained text nodes".

## How the boundary is enforced by machine

`scripts/check-engine-boundary.py` enforces the rules below. Each has a self-test
(`scripts/tests/test_engine_boundary.py`). CI runs them every time:

1. **Product graph** (#99 §11): **no workspace member**, under any feature combination, may have a normal / build edge
   to `cosmic-text` / `cryoglyph` / `glyphon`. Packages whose names **contain** them count too. Renaming the fork to
   `nana-cryoglyph` and keeping it as the production abstraction is exactly what #99 forbids. Only two exceptions are allowed: dev edges, and
   the comparison tool packages in `REFERENCE_ONLY_PACKAGES` (and product crates may not depend on those). This gate stays after the dependency
   was removed: it guards against someone pulling the old engine back in **in the future** for a feature. `Cargo.lock`
   only shows that after it is already in.
2. **Source**: `crates/nana-text/src/**` may not contain the identifiers `cosmic_text` / `cryoglyph` /
   `glyphon`. While the reference engine was alive, this was the only mechanical way to say "the core API exposes no cosmic types"
   (the dependency graph could not say it, because back then it was a legitimate dev dependency). Now that the engine is gone, it keeps anyone from
   bringing its types back. Comments are stripped before scanning, so `lib.rs` can state the boundary plainly.
3. **allowlist**: when `crates/nana-text/src/**` references `nana_ui_core::`, it may only hit the items in the table above.
4. **Font layer backends** (#90): `fontdb` may only appear in `src/font/discovery.rs`. `skrifa` may only appear in
   `src/font/face.rs`. `icu_properties` may only appear in `src/font/unicode.rs`. `memmap2` may only appear in
   `src/font/mapped.rs`. `read_fonts` /
   `ttf_parser` may not be named anywhere. None of these three modules may be `pub mod` in `font/mod.rs`, so the public API
   cannot leak a third-party ID such as `fontdb::ID`. Same for #91: `harfrust` may only appear in
   `src/shaping/opentype.rs`, and `unicode_bidi` may only appear in `src/shaping/bidi.rs`. The module is not named
   `harfrust` precisely because module names are scanned by this rule too. Same for #92: `unicode_linebreak` may only appear in
   `src/layout/breaks.rs`.

### Where the reference engine went

The reference engine used to live in `crates/nana-text/tests/reference/` (**not** a
`#[cfg(test)] mod` under `src/`: that is invisible to `tests/*.rs`, so the corpus harness could not use it). The point was that it could be
deleted in one go without touching a line of `src/`. Once the native engine landed and the product path was switched over, it was deleted:

- `tests/reference/` (the cosmic-driven reference implementation)
- `tests/text_parity_corpus.rs` (reference vs golden, plus the `NANA_TEXT_BLESS` re-record path)
- `tests/reference_engine_counters_agree_with_the_layout_it_produced.rs`
- `[dev-dependencies] cosmic-text` in `crates/nana-text/Cargo.toml`, and the workspace fork pin

Not a line of `src/` changed. That is exactly why it was put in `tests/` in the first place. What remains are the goldens and two cases that run the **native** engine
against them. Coverage is unchanged: all 26 cases, the same `compare_golden`, the same tolerances.

## Font layer (Phase 1, #90)

`nana_text::font` outputs **deterministic font selection that Phase 2 shaping can consume**. It does not shape.

```text
TextStyle / CSS font-*
        ↓ FontQuery::from_style
FontQuery { families, weight, stretch, style, language }
        ↓ FontSystem::select (cached per query, cleared on generation change)
FontSelection { primary, fallback_chain, generation, families[] }
        ↓ FontSystem::resolve_text (coverage cache)      ↓ FontSystem::instance
FontAssignment { range, font, reason }          FontInstance { key: FontInstanceKey, ignored_axes }
```

### Sources and lifecycle

| Source | Entry point | Bytes |
| --- | --- | --- |
| Platform system fonts | `load_system_fonts` / `with_system_fonts` | The file is mapped (not read onto the heap) the first time details / coverage are needed, and the faces of one file share that mapping |
| Explicit file | `register_file` | Read and validated at registration; a bad file fails at registration |
| In-memory bytes / test fixtures | `register_bytes(font_blob(..), &FaceDescriptor)` | Shared via `Arc` |

- `FaceDescriptor` is an `@font-face` descriptor: the declared `family` **replaces** the face's own name.
  weight / stretch are closed ranges. style restricts to a single style.
- Every change (register, unload, `replace_bytes`, load system fonts, swap `FallbackPolicy`) bumps
  `FontGeneration` **exactly** once. The selection cache is cleared entirely (any new face can change the result of some family list).
  The coverage cache drops **only** unloaded faces (an unrelated registration does not change a face's cmap).
- `FontId` / `FontSourceId` are generational handles: after an unload, the slot is reissued with a higher generation and old ids are rejected.
  `FontInstanceKey` therefore cannot be confused with a replacement face.
- `face_data` returns `FontData` (`Send + Sync`, holds an `Arc`). An unload while a worker holds it does not free the bytes.
  `retired_font_data_alive` reports how many copies of bytes are unloaded but still held, so the release point is observable.
- `FontSystem` is a plain `&mut self` value. It is not global and holds no lock. When the host needs to share it, the host picks the synchronization.

### Matching

CSS Fonts 4 §5.2: first `font-stretch`, then `font-style`, then `font-weight`. Each step keeps only the faces at the best distance.
The issue draft said style → weight → stretch. This follows the CSS spec order instead, because CSS / Rust
both end up as the same `FontQuery`. An order that differs from browsers would make the same style select different faces in the two places.

- Variable faces participate as **ranges**: the `wght` axis gives a weight range, the `wdth` axis gives a stretch range, and the `ital` / `slnt`
  axes let a face support both normal and italic / oblique.
- The query's stretch comes from `TextStyle::font_stretch` (a percentage of normal width, default 100, clamped to
  50..=200). As with CSS `font-stretch`, the caller only gives a width: if the family has a condensed static face it is selected,
  and if the selected face has a `wdth` axis the coordinate comes from the query's implied value in the next section. Callers should not write
  `wdth` into `variations` themselves. That is an explicit value and overrides stretch. A rich text span's `stretch_percent` goes through this field.
  It is also part of the `ShapeKey`.
- For 400..=500, first look for heavier faces up to 500, then lighter, then above 500. Below 400: lighter first, then heavier. Above 500: heavier first, then lighter.
- On a remaining tie: explicit registration (file / memory) beats a system scan, then the **later registration** wins. The whole process never iterates
  a HashMap. The same face set and query always select the same face.

### Variation coordinates and #41

Each axis's coordinate, from lowest to highest priority: face default → query implied (`wght` ← weight, `wdth` ← stretch,
italic → `ital = 1`, oblique → `slnt = -14`, only when the face has that axis) → explicit `font-variation-settings` value.

- **`font-weight` vs explicit `"wght"`**: explicit `"wght"` wins, and it decides both face **selection** and the coordinate.
  `"wdth"` does the same for stretch. This is an existing rule on the `nana-ui` product path (`wght` folds into weight). Keeping it
  means the migration does not swap the face a page gets. No other axis (including `BEVL`) takes part in selection.
- Axes the face does not have **fail closed**: they go into `FontInstance::ignored_axes` and have no effect. They are **never** rewritten to `wght`.
- `FontInstanceKey { font, coords, synthesis }` is the key of the Phase 2 shape cache and the glyph raster cache:
  coordinates are clamped first, then default values are omitted. Two requests that render identically therefore have the same key (`wdth 1000` and `wdth 200` share a key on a face
  whose maximum is 200).
- Bold / italic that cannot be achieved is recorded in `Synthesis`. The renderer decides how to fake it. Axis animation (#85) is not in this phase.

The tests read the outlines back directly with skrifa: `BEVL 42`, `wdth 150`, and `wght 700` each actually change the outline of `A`.
That proves the coordinates reached the face, not just that they were recorded.

### Fallback

Two layers: `FallbackPolicy` (ordered family names) supplies candidates. `resolve_text` probes with cmap coverage. Clusters that are covered but
still produce `.notdef` from shaping are retried by Phase 2. The unit is the grapheme cluster. Default_Ignorable code points such as ZWJ / variation selectors
do not require coverage. When the primary already covers the text and no color is needed, it returns immediately without building a candidate list.

Candidate order:

- **Emoji presentation** (VS16, or `Emoji_Presentation` without VS15): faces in the chain with color tables →
  emoji policy → rest of the chain.
- **Everything else**: the chain in order (primary first).

Both are then followed by: the cluster's script policy (a cluster without a script of its own inherits the previous cluster that has one,
so punctuation after CJK keeps looking for a CJK face) + language hints (`ja` / `ko` / `zh-Hant` / `zh-HK`
prefer the matching glyph forms) → symbol and emoji policy when there is no script of its own → last resort.

**When no policy covers it, the font database is scanned once more** (`scan_database`): policies are an *order of preference*, not the full set of
"faces on this machine". ✓ (U+2713) is an example. On macOS it is in neither the sans face nor `Apple Symbols`,
but it is in `Arial Unicode MS`. Drawing `.notdef` because a policy could not find it, while the machine plainly has the glyph, is the worst possible answer.
Scans are memoized per code point (`scanned`), so a document full of the same missing glyph scans only once. The memo is cleared when the font collection changes.
Only when the whole database truly lacks it is `Missing` recorded and the primary's `.notdef` rendered. Only then does "missing glyph" really mean "this machine does not have it",
rather than "no list happened to name it". A scan hit records `LastResort { family }`, where family is the hit face's own name.

`FallbackPolicy::platform_default()` gives common families for Windows / macOS / others (Linux, Android).
Names that do not resolve to a face are skipped. Hermetic tests always build their own from `FallbackPolicy::empty()`.

The coverage cache stores folded code point ranges per `FontId` (a CJK face's roughly 30k mappings fold into a few hundred ranges). The default budget is
4 MiB, with LRU eviction beyond it. Each lookup is a binary search.

### Diagnostics

- `FontSelection::families`: every family name tried, which request item it came from (including generic expansion),
  and which face it resolved to.
- `FontAssignment::reason`: `Primary` / `FamilyChain { index }` / `EmojiPolicy { family }` /
  `ScriptPolicy { script, family }` / `SymbolPolicy` / `LastResort` / `Ignorable` / `Missing`.
- `describe(FontId)`: source, path, names, the weight / stretch ranges used for matching, axes, named instances, whether it has color.
- `FontCounters`: `font_faces_registered`, `font_generation`, `font_query_hits/misses`,
  `fallback_candidates_examined`, `coverage_cache_hits/misses/evictions`,
  `font_fallback_attempts`, `font_fallback_misses`.

### Tests

- `tests/font_system_is_hermetic_and_deterministic.rs`: uses only the checked-in fixtures and the Noto Sans SC variable font bundled by `nana-ui-core`
  (`wght` 400–700; all four UI weights land on the axis, no synthetic bold). Policies are built in the test. Covers preferred family, weight / style /
  stretch, variable ranges, CJK / language hints, emoji color candidates, mixed Latin + CJK, missing glyphs, `BEVL` / `wdth`,
  `wght` priority, register / unload / replace and generations, reuse for the same query, and handing off across threads.
- `tests/font_system_platform_acceptance.rs`: reads the local machine's fonts. `#[ignore]`. Run by hand on the target platform.
  2026-09-16, Windows 11: scanning 315 faces took 15 ms. `Hello, 世界。こんにちは 한국어 🔥`
  landed on Segoe UI / Microsoft YaHei UI / Malgun Gothic / Segoe UI Emoji respectively, with no `Missing`.
  Cold start (reading from disk) about 110–140 ms. Hot path about 40 µs.

The product path has sat on this layer since #99: `crates/nana-ui/src/text_engine.rs` holds the process's single
`FontSystem`. `@font-face`, `local()` aliases, and `sans-serif` overrides all land on it. The painter's rasterizer also
fetches font data by the `FontId`s it issues.

## Shaping (Phase 2, #91)

`nana_text::shaping`: character sequence → immutable `ShapedRun`. No line breaking, no layout, no carets. Those are Phase 3.

```text
ShapeRequest { source: &TextSource, style, direction, language, scale }
  → span normalization: spans override the base, composition spans override ordinary spans; boundaries snap to grapheme cluster starts
  → grapheme cluster / script / BiDi segmentation
  → font per cluster (FontSystem::resolve_text, #90 coverage selection)
  → item = longest contiguous cluster run with the same style segment × same face × same BiDi level × same script
  → HarfRust: face bytes + variation coordinates, direction, script, language, features, kern;
    text outside the item is passed as pre/post context, cluster is a byte offset into the full text
  → .notdef cluster ranges are retried against the #90 candidate list and split into fallback runs
  → Arc<ShapedText>, cached by ShapeKey
```

### Output

- `ShapedText.runs` are in **logical order**. Each run carries `bidi_level`, `direction`, `script`, `font`,
  the physical px font size, and `RunMetrics` (measured at that instance's coordinates). Glyphs within a run are in visual order (HarfBuzz convention).
- `ShapedText.paragraphs` are the UBA paragraphs and their base levels. `visual_order(range)` does L2
  reordering for one line's runs. L1 (resetting trailing whitespace) belongs to line breaking and is left to Phase 3. This is the
  run / level information that later #59 bidi / vertical layout needs.
- `cluster` / `cluster_end` are byte offsets into the source text. The source string is never rewritten: no normalization that would break the mapping is done,
  so no offset map is needed. Paragraph separators (`\n` / `\r\n` / U+2029 …) produce no glyphs, so there can be gaps between runs.
- `ShapedText` sits behind an `Arc`. Layout reads it and copies a run when it needs to write an origin. Cached values are never modified in place:
  a width change from 400 to 300 reruns only Phase 3.

### Segmentation

- **Grapheme clusters are never split**: style span boundaries snap backward to the start of their cluster. Emoji ZWJ sequences and combining marks always
  stay in one item (HarfRust's own cluster merging sits on top of that).
- **Script**: each cluster takes its own script. Common / Inherited inherit from the previous cluster with a script; leading ones inherit from the first one after them.
  Punctuation, spaces, and digits do not start runs of their own.
- **BiDi**: `unicode-bidi`. The paragraph level is fixed by `direction` (CSS semantics; no first-strong-character detection).
  Direction overrides such as `unicode-bidi: bidi-override` have no CSS input to attach to. They are not provided in this phase.
- `language` goes into HarfRust (`locl` and so on): each item selects the font's OpenType language system for this language under its own script, Latin items included. Browsers do the same when they hand `lang` to HarfBuzz (CSS `font-language-override: normal`). It also goes into #90's fallback language hints, where it only decides the order within one script's rules (the platform default orders only Han by language). Characters the primary covers use the primary directly and never enter the candidate list, so language cannot change their font. Latin letters and digits are usually in that case.

### Fallback retry

Even after coverage preselection, a glyph can still fail to draw (GSUB substitutes `.notdef`, cmap and layout tables disagree). After shaping each item:

```text
find cluster ranges containing glyph 0 → for each range take the next face from the #90 candidate list that is untried and covers that range
 → switch that range's face, reshape the remaining parts on both sides (the context changed) → until clean or candidates are exhausted
```

When exhausted, the primary's `.notdef` is kept and the glyph is marked `MISSING`. When no face in the family list resolves at all, the first
registered face steps in to draw `.notdef`. Text never silently disappears. Only when the font system has no face at all is no run produced,
and it is counted in `text_bytes_unshaped`. Limits: each range looks at no more than
`MAX_FALLBACK_CANDIDATES_PER_RANGE = 8` candidates. Each item retries at most `MAX_FALLBACK_RETRIES_PER_ITEM = 32`
times. A long string that "no font can draw" therefore does not become text length × font count.

### ShapeKey and cache

| In the key | Not in the key |
| --- | --- |
| Text content (the source's `Arc<str>` + a content hash memoized per revision) | Widget identity, `TextRevision` itself |
| Each span's range and shaping-relevant style: family, physical font size, weight, italic, letter-spacing, features, variations, kerning | line-height, composition state |
| `direction`, `language`, device scale, `FontGeneration` | max width / wrap / max lines / ellipsis and the other constraints; color, opacity, transform (`TextStyle` does not carry them anyway) |

- Lookups **do not copy text**: the key shares the source's `Arc<str>`. The hash is memoized per revision inside `TextSource`,
  so repeated lookups at the same revision hash only once. Equality compares the hash first, then the pointer, and finally bytes. A hash collision is never
  treated as a hit.
- The same text from different sources (10k identical labels) lands on the same entry.
- Two limits: entry count (default 4096) and retained bytes (default 16 MiB, including text retained by keys). LRU, with eviction counts.
  A result larger than the whole byte budget is returned as usual, not cached, and does not push anything else out.
- The font epoch in the key is "which `FontSystem` × which generation": `FontId` and generations start from zero in every system.
  A `Shaper` serving several systems in turn must not mix them. When a new epoch is seen, entries from other epochs are cleared at once (counted as
  evictions). HarfRust's per-face acceleration data is dropped at the same time.

### Counters

`Shaper::counters()` → `ShapeCounters`:

```text
shape_requests / shape_runs_created / shape_glyphs_created
shape_cache_hits / misses / evictions      shape_cache_bytes / entries (values at read time)
bidi_runs / script_runs                     level runs and script runs split out on a miss
fallback_retries / fallback_fonts_examined
text_bytes_hashed                           once per source revision
text_bytes_unshaped                         bytes that produced no run because the font system has no face (excluding paragraph separators)
```

`NativeTextEngine::layout` folds each call's shape / layout cache hits and misses into
`TextWorkCounters` (`shape_cache_hits` / `layout_cache_hits` and so on). Frame counters read that copy.
`ShapeCounters` itself stays inside `nana-text` for diagnostics and does not leave the crate.

### Reconciliation with the cosmic goldens

`tests/shaping_matches_the_cosmic_reference_goldens.rs` uses the same hermetic fonts and the same
fallback chain. It compares all 26 corpus cases against the goldens field by field, per **logical cluster**: glyph count, glyph id, `cluster_end`,
`MISSING`, `FALLBACK_FONT`, bidi level, and font size exactly. advance / offset use the 0.05 px above. Faces
are compared as a bijection. Comparing per cluster is independent of where either side splits runs or breaks lines.

Result: all 26 match. The only documented difference is allowed for line-breaking cases only: goldens are laid-out lines. Whitespace glyphs at soft wraps
and content truncated by `max_lines` are not in the goldens, while shaping without line breaking keeps them. That is Phase 3's business.
Mutation check: with kerning forced off, the test reports 45 deltas.

Reconciliation also exposed **a defect in the reference engine**: when a case did not set `font_family`, the reference engine used the fixture id
(`noto-sans-sc`) as the family name to look up "the requested face". The lookup failed, so no glyph got
`FALLBACK_FONT`, including the glyphs in `TX-B02` / `TX-B03` that really came from the Arabic fallback font. This was fixed to use the
family name and re-blessed. The only golden changes were 20 `flags: 0 → 2` in those two cases.

When the bundled font switched from four static weights to one variable font (`wght` 400–700, default instance Regular), the Regular outlines,
advances, and GSUB/GPOS stayed the same, but the glyphs in the subset were renumbered. The goldens therefore only got a mechanical glyph id renumbering:
glyphs with code points were mapped through cmap, glyphs without code points were matched uniquely by outline and advance; 5 cases, 39 `glyph_id`s,
every other field untouched. This is not a re-recording with the new engine.

### Tests

`tests/shaping_pipeline.rs`: `ffi` ligature and `liga 0`, combining marks, Arabic joining and RTL, mixed BiDi
levels and L2 reordering, levels of digits and punctuation under RTL, CJK + Latin coverage fallback and `FALLBACK_FONT`,
emoji ZWJ, the `wdth` axis, missing-glyph `.notdef`, `.notdef` retry (`nana-test-notdef`), style
spans across scripts, span boundaries falling mid-grapheme, source byte / cluster mapping. On the cache side: 10k identical labels shape only once,
repeated lookups on the same source hash only once with zero copies, width / line height / new revision with the same content do not reshape, direction / scale /
font generation do reshape and clear the old generation, entry and byte limits and LRU.

This phase guarantees that paint / transform cannot enter the ShapeKey at all. That they produce no shape requests in UiWorld
is verified on Runtime by the Phase 4 dirty graph (see "UiWorld retained text nodes").

## Layout (Phase 3, #92)

`nana_text::layout`: immutable `ShapedRun[]` + `TextConstraints` → immutable `TextLayout`.
**Shaping and layout are decoupled**: changes to width, wrap, alignment, or max-lines rerun only this layer. HarfRust is not touched.

```text
ShapedText (#91, shared via Arc) + TextConstraints
  → single-line Label fast path, or:
  → split by UBA paragraph → UAX #14 break opportunities (unicode-linebreak, only in layout/breaks.rs)
  → greedy line breaking on shaped advances (trailing whitespace hangs at soft wraps)
  → per line L1 (reset trailing whitespace) + L2 (visual order)
  → line box metrics, alignment, ellipsis
  → immutable TextLayout, cached by LayoutKey
```

### Single-line Label fast path and fallback to the paragraph path

`TextKind::Label` takes the fast path when all of these hold: no wrapping, `max_lines` is `None` or `1`,
no budget in the stacking direction (`max_height_px` for horizontal text, `max_width_px` for vertical), and **`segments()` yields only one segment**. Single-line vertical labels take the fast path too.
The fast path does one thing: accumulate the advance of its runs, compute the line box once, truncate with an ellipsis once (when needed),
and emit one `LineBox`. It builds no editor state, does not scan paragraph structure, does not look for break opportunities
(`line_break_candidates` is always 0), and does not relayout for color / transform changes (those cannot enter the
`LayoutKey` at all; see below).

The last condition is not re-derived locally. It asks `segments()` (the same function as in the "Line breaking" section below): explicit line breaks, forced breaks,
and a **trailing** line break (which produces no new paragraph but does produce an empty last line) therefore all fall back in the same way. None slips through.
If any condition fails, it falls back to the paragraph path. The same goes for any wrap, multiple or zero `max_lines`, a height budget,
or a writing mode that needs fallback. The fallback is observable: the `label_fast_paths` / `paragraph_paths`
counters count each.

### Line breaking

Break opportunities come from UAX #14 (`unicode-linebreak`). **Whether** to break at an opportunity is decided by shaped advances,
never estimated from code point counts.

Hard breaks have two sources. **Paragraph separators** (UBA class B, i.e. `shaping::PARAGRAPH_SEPARATORS`: `\n`, `\r`,
`\r\n`, U+001C–U+001E, U+0085, U+2029) come from shaping's paragraph structure. UAX #14 is not asked again.
**Forced breaks** (`breaks::FORCED_BREAKS`: VT, FF, U+2028 LINE SEPARATOR) are ignored by the paragraph structure.
Layout splits on them within a paragraph (`has_forced_break` is an ordinary character scan, not a UAX #14 pass, and it is only asked once when the
layout is **built**).

Each of the two tables has a reconciliation test, because **three places** must agree at once that a character is a separator: shaping drops it (no glyph),
layout breaks the line there, and `preserve_lines: false` folds the single-byte ones into spaces. If any one misses it, the separator is drawn
as a `.notdef` box on the line it just ended. `PARAGRAPH_SEPARATORS` is checked code point by code point against `unicode-bidi`'s
actual segmentation. `FORCED_BREAKS` is checked against `unicode-linebreak`'s mandatory breaks.
`FOLDED_SEPARATORS` asserts that it is exactly the set of single-byte members of those two tables. The two standards disagree about U+001C–U+001E
(UBA splits paragraphs there, UAX #14 does not break). The stricter UBA reading is used.
The separator itself falls between two segments. No line covers it, so it is not drawn, the same treatment as `\n`. This list is checked
code point by code point against `unicode-linebreak`'s own data by `forced_breaks_agree_with_uax14`. A new one added in a Unicode
revision cannot slip through. A Label with a forced break falls back to the paragraph path just like one with `\n`.

| Constraint | Policy |
| --- | --- |
| `wrap: None` | No wrapping, a paragraph is one line; `line_break_candidates == 0` |
| `wrap: Word` | Break only at UAX #14 opportunities; long words that do not fit overflow (`CLIPPED_WIDTH`) |
| `wrap: WordOrGlyph` or `word-break: break-word` | Break by word first, then split words that do not fit by grapheme cluster |
| `wrap: Glyph`, `word-break: break-all`, `line-break: anywhere` | Every grapheme cluster boundary is an opportunity |

- **Hanging trailing whitespace**: whitespace at a soft wrap is not drawn, and `LineBox::source` does not include it (consistent with the reference engine).
  Whitespace at a hard break and at the end of a paragraph is content the author wrote, so its glyphs stay on the line. In both cases it **does not count toward the line width**:
  `metrics.width_px` / `bounds` are the width without trailing whitespace. Overflow checks (`CLIPPED_WIDTH`, ellipsis truncation)
  and alignment both use it. Line-break decisions never counted trailing spaces in the first place, and the other stages must agree. Otherwise `"Save "` would be truncated to `"Sa…"` in a box that fits
  `"Save"`, and when centered it would sit half a space to the left of `"Save"`.
- Hanging whitespace bytes at a soft wrap belong to no line's `source`, but they are still valid caret positions:
  `caret_geometry` resolves them to the end of the line they hang from (or the start of the next line, depending on which line the caret reports).
- Hanging follows the paragraph direction: L1 assigns trailing whitespace to the **end** of the paragraph. In an RTL paragraph that is the visual left side.
  The line's start position is therefore shifted left of the alignment box by the hanging width. Drawing it inside the box would push every real glyph out the other side.
- When the container is too narrow for even one grapheme, one grapheme is still placed. Otherwise you get empty lines and an infinite loop.
- **Soft wraps never produce empty lines**: there is a break opportunity after leading whitespace in a paragraph. Breaking there would make the first line draw nothing and leave the whitespace nowhere to go.
  Such opportunities are therefore skipped (the same for emergency splitting). Empty lines only come from empty segments.
- **The segment is the unit of line breaking**: each BiDi paragraph is cut again at forced breaks. The separator between segments
  belongs to no segment and is therefore not drawn. Two kinds of segments exist only for the caret: empty text has no BiDi paragraph, and text ending
  in a paragraph separator has no paragraph after it. Both get an empty segment added, so an empty input box has a line box and the caret does not disappear after pressing Enter at the end of a line.
  `intrinsic_widths` measures **the same segments**, so `max-content` is "the widest line", not the width of two lines joined end to end.
- Break points are always between shaper clusters, so a UTF-8 sequence, grapheme cluster, or ligature can never be split.

### BiDi visual order

Each line independently does L1 (reset trailing whitespace to the paragraph level) and L2 (reorder by level), using the run levels #91 already computed.
UBA is not rerun. `LineBox::runs` is a contiguous range in **visual order**. `ShapedRun::source` is still the
**logical** byte range. Both are readable at once. Caret / selection / hit testing (`nana_text::edit`) run directly on top of them.
When a line splits a run, each piece keeps the original run's `ShapeRunId`: the id says "which shaping produced these glyphs",
and splitting lines produces no new shaping. A whole run keeps the `advance_px` the shaper reported. Only split pieces re-sum
the shaper's per-glyph advances.

### Line box metrics and the strut

```text
line_height = max(strut line height, line heights required by the styles of the line's runs)
ascent / descent (reported values) = max(strut, font metrics of the line's runs)
half_leading = (line_height - (strut.ascent + strut.descent)) / 2
baseline     = top + half_leading + strut.ascent
```

`strut` is the CSS strut: **the metrics of the base style's own face**. The engine seam
(`NativeTextEngine`) gets it from `FontSystem`. It is the mechanical implementation of "the baseline must not jitter because of a single emoji/CJK fallback".
`Search` and `Search عربي` have exactly the same baseline. A taller fallback face only raises
the reported ascent and the line box. It does not move the baseline.

**Ascent / descent in the font metrics.** Usually this is the `hhea` skrifa reads (or OS/2 typo under `USE_TYPO_METRICS`).
The exception is CJK fonts: for a face that maps Han, kana, or Hangul and whose `BASE` horizontal axis declares an `ideo` baseline
(Source Han Sans / Noto CJK and the like), `face::read_metrics` shifts ascent and descent so their midpoint lands on the midpoint of the
ideographic em box (`ideo` to `idtp`, or `ideo` plus one em if `idtp` is absent), keeping their sum. Noto Sans SC's
`hhea` is 1.16 above and 0.288 below, while the ideographic em box is 0.88 / −0.12. Splitting the leading by `hhea`, a line of Chinese sits about 0.06 em low in a box of any
height (10 px above and 6 px below in a capsule, at scale 2). After the shift, centering within a line lands on the center of the em box,
which is the central baseline of these fonts; the sum is unchanged, so `normal` line height and multi-line spacing are unchanged. Latin letters in the same face
move up with it (they are drawn in that em box anyway). Fonts without an ideographic em box do not move. The reference engine reports raw `hhea`; corpus
reconciliation lists these three items as recorded differences and checks line by line that the sum and the line center are unchanged.

Without a strut (the `LayoutRequest` default), the baseline sits on the line's tallest run, which is the reference engine's rule.
Corpus reconciliation runs in this mode; see below. An empty line has no runs, so its line height comes from the base style.

`line-height` is resolved from the style (`LineHeightSpec` → px; when unset it is `font-size × 1.2`, the same number as
`nana_ui_core::text_line_box_height_px`). When a span has its own `line-height`, it is resolved
**by the same rule the shaper uses**: `TextSource::SnappedSpans` snaps span boundaries to the start of their grapheme cluster
(the shaper cannot split a cluster), then picks "last composition span → last ordinary span → base".
Shaping and layout must resolve the same way. Otherwise a span falling mid-cluster would be shaped at the span's font size but measured into the line box at the base's line height,
and the glyph would overflow its own line box. With no spans, this path resolves nothing at all.

**Fractional scale**: `max_width_px` / `max_height_px` are logical px. They are multiplied by `scale` and compared with physical px
advances. Layout keeps floating point throughout and does **not** round to integer pixels. That is the renderer / glyph path's job.

### Alignment

`TextConstraints::align` (`nana_ui_core::TextAlignSpec`): `start` / `end` follow the paragraph direction.
`left` / `right` are physical directions. Without `max_width_px` there is no container to align in, and every keyword puts the line at the origin.
Line width is computed without trailing whitespace, so `"Save "` and `"Save"` center at the same place.

A line wider than its container is **not pushed back to the origin**: slack goes negative. `end` / `right` / `center` let it overflow past the start edge
as CSS does. A clipped container then shows the end the caller aligned to. `bounds.x` can therefore be negative. `TextRect` never
flattens negative values in the first place.

`justify` (#211): on every line except the last line of a paragraph and lines ended by a hard break, the remaining width is distributed according to `TextConstraints::justify`
between words (`inter-word`), after CJK characters (`inter-character`), or both (`auto`); with `none` the line stays at the start edge.
The distributed width is written into glyph advances, so caret, selection, and hit testing follow it. In Runtime, `text-align: justify`
inherits as in CSS: paragraphs under a justified box are all justified, whatever alignment the component asked for.

### CJK line decisions (#211)

Line breaking, punctuation compression, kinsoku, CJK–Latin spacing, and justification are decided together in `Typography` in `layout/lines.rs`.
When none of them is enabled, no `Typography` is built and every line is byte-for-byte the same as before.

| Capability | Switch | Default |
| --- | --- | --- |
| Compression of punctuation at line start/end and between adjacent punctuation (fixed amount) | `text-spacing-trim: normal` / `trim-both` | `normal` when `lang` is zh / ja (including inherited), otherwise `space-all` |
| Cost-based compression when the line does not fit, to avoid a break | `text-spacing-trim: auto` | Off |
| CJK–Latin spacing (em/8 between ideographs and Latin letters or digits, removed at line end) | `text-autospace` | Off |
| Kinsoku trimming | `line-break: strict` / `loose`; under `break-all`, closing punctuation is not allowed at line start and opening punctuation is not left at line end | `auto` |
| Justification | `text-align: justify` + `text-justify` | Off |
| Bounded lookahead | `text-wrap-style: pretty` | Off |

- **Ink decides how much can be compressed**: during shaping, each full-width punctuation mark has its horizontal ink measured once with `font/face.rs::read_glyph_ink` (skrifa lives only in this file).
  Blank space in the back half is marked `PUNCT_BLANK_AFTER`, in the front half `PUNCT_BLANK_BEFORE`. If it cannot be measured, it is
  not marked and gets no compression capacity, so glyphs never collide. Vertical text does not change advances.
- **Fixed levels do not search**: `normal` only swaps the cell width for the compressed value and still uses the original greedy line breaking, with no cost candidates.
  Opening punctuation at line start loses its front-half blank (except on a paragraph's first line), and adjacent punctuation keeps only one blank between them; the blank of closing punctuation at line end
  is only removed when the line would not fit otherwise.
- **Cost-based compression** (`auto`): when a line does not fit, it compares "compress the punctuation in the line and keep it on this line" (cost 15 per 1/64 px) with
  "break at the previous break point and leave slack" (cost `15·64·slack²/em`) and takes the cheaper one; on a tie it breaks. The compression amount is distributed across the line
  in proportion to capacity, and the remainder is assigned 1/64 px at a time in order, so the result is deterministic.
- **`pretty`**: each line keeps at most K=4 candidates, each candidate looks at no more than C=4 break points, with a total budget of `16·B + 256` steps;
  when the budget runs out it deterministically finishes greedily and counts `line_budget_fallbacks`. It does not switch strategy by paragraph length, so it does not
  oscillate between two results as the width changes continuously.
- **Atomic boxes**: `InlineObjectMetrics::envelope` (`InlineEnvelope`) declares the spacing after the object, the part of it that can be absorbed,
  and whether breaks are allowed before and after. The spacing is merged into the object's width; the absorbable part is removed at line end and yielded within a line by cost (an object that
  declares an absorbable part turns on cost-based compression); forbidden break points are removed from the break table. Decisions never reach inside an object; the `PlacedObject` rectangle
  is still the object's own width, with the spacing following it in the inline direction.
- Counts: `line_opportunities_considered`, `line_break_comparisons`,
  `line_beam_states` (max), and `line_budget_fallbacks` in `TextWorkCounters` flow into Runtime's
  `WorkCounters::dynamic` through `record_text_work`. Gate D (`tests/line_decision_gate.rs`) locks in near-linear behavior, the beam limit, and no reshaping on resize.
- Render-side fallback: `SceneTextOpenType::typography` carries the `UsedTextTypography` Runtime used. When the painter re-lays out,
  it hands it to the engine through `nana_ui_runtime::apply_used_typography`, and it goes into the `ShapeKey`.

### Ellipsis

When `constraints.ellipsis` is true and (the line is too wide **without wrapping**, or it is truncated by `max_lines` / `max_height_px`):

```text
candidate line → reserve the shaped ellipsis width → cut at a cluster-safe boundary → emit the truncated line + ellipsis run
```

- The ellipsis goes through **normal** shaping / fallback / cache: the seam hands `…` to the shaper as an ordinary `TextSource`.
  10k truncated labels in the same style shape it only once (the test asserts `shape_cache_misses == 2`: once for the body, once for the ellipsis).
- The cut unit is the shaper's cluster, so UTF-8, grapheme clusters, and ligatures are never split. A ZWJ sequence is either kept whole or cut whole.
- The cut point is always the position **after trailing whitespace is removed**: the ellipsis stands in for the text it replaces and starts right after that text's last visible cluster.
  Drawing it after the hanging whitespace would draw it beyond the line width and outside the container, and nothing would say so.
- The ellipsis run's `source` is an **empty range** at the cut point, and so is its glyph cluster. It takes no bytes of the source text,
  so caret, hit testing, and selection can never land on it.
- **With wrapping on, an over-wide line is not cut**. A wrapped paragraph only becomes too wide because of a long word that cannot be broken, and that whole word is on this line:
  cutting it would make those bytes disappear from every line. `LineBox::source` would be left with a hole in the middle that nobody can draw, select, or
  hit, and no flag could say so. Such a line is allowed to stick out, with `CLIPPED_WIDTH` set.
- **A truncated single line reports its full width to layout**. Runtime shapes at the old box width after the box has been measured, and what is drawn is the cut line.
  But `TextMetrics::width` (the intrinsic width layout reads) takes the full line width from a second layout without a width constraint. Otherwise
  a shrink-to-fit parent would keep holding the old box: changing a label to longer text would only keep getting cut short and never grow back.
  This is the only exception to "metrics and the retained layout share one source", and it only happens on single lines with `ELLIPSIZED` set.
- **Wrapped text records a separate unwrapped width**. For lines wrapped at the old box width, `TextMetrics::width` is the widest of them,
  reported to layout as is, but used only as the content width. A parent that shrinks to content (`fit-content`, `max-content`, a shrinking flex item)
  would keep measuring by the old wrapping after its limit widens, and never grow back. So when text has wrapped (some line has `break_cause == Wrap`),
  Runtime lays out the whole paragraph once more without a width constraint and stores that width beside the node (not in `TextMetrics`). When layout measures content width it
  takes `max(wrapped width, min(unwrapped width, available width))`: the box widens with the limit, and the text rewraps at the new width. Text that did not wrap is not laid out again.
  A host shaper without an engine cannot say where it broke lines, so it asks for the unwrapped width once more only when the result is more than one line tall.
  The truncation the ellipsis really exists for, `max_lines` / `max_height_px`, works as usual and carries `TRUNCATED_LINES`.
- **A box that only wraps the old text does not set the wrap width for new text**. When the previous lines did not wrap and exactly fill the box, the box width is the old text's own width.
  How wide a content-sized box can get depends on what the parent gives it, and only layout knows that. When the text is reshaped (content, font, shaping style, or language changed)
  and wraps at the old width in such a box, Runtime switches to the unwrapped full paragraph the previous rule was going to lay out anyway: both the metrics and the retained layout use it,
  no unwrapped width is recorded, it is stamped as having no width constraint, and the constraint revision lags one step. Layout gives it `min(unwrapped width, available width)`.
  The scoped shaping after layout lays it out again at the new box width; when it fits, the metrics are unchanged and layout is not rerun. So a longer label in a wide column
  runs layout only once in a frame, and the content below does not move; new text that does not fit is first laid out unwrapped, then after layout wrapped once more at the box width.
  The unwrapped height is never taller than the wrapped one, so the content below is not pushed away and then pulled back.
  A paragraph that has already wrapped fills the space it was given anyway, and keeps wrapping at the box width as before.
- In an RTL paragraph, the ellipsis goes at the visual end (the left side).
- When the text is truncated but no ellipsis was (or could be) shaped, only `TRUNCATED_LINES` is reported, not `ELLIPSIZED`: if it was not drawn, it does not claim it was.
- The `break_cause` of a truncated line says **which budget ran out**: `MaxLines` or `MaxHeight`. Writing `MaxLines` on a layout where `max_lines`
  was never set would assert to consumers a constraint that never existed.

### LayoutKey and cache

| In the key | Not in the key |
| --- | --- |
| The **identity** of the shaped runs (holds the `Arc<ShapedText>`, compared by pointer) | Widget identity |
| The requesting source's `TextRevision` (see below) | |
| The identity of the shaped ellipsis | Color, opacity, transform, z-index, background (`TextStyle` does not carry them anyway) |
| `TextKind` | |
| All `TextConstraints` fields (width/height, wrap, word-break, line-break, max-lines, ellipsis, preserve-lines, direction, align, writing-mode, tab-width, scale, spacing-trim, autospace, justify, wrap-style) | |
| Inline object metrics and envelopes (`InlineEnvelope`) | |
| Object placement (the flow and `fit_width` of `ObjectPlacement`) | |
| Each run's resolved line height, the empty-line line height, the strut | |

- The key **holds** an `Arc<ShapedText>` rather than a raw pointer: holding it is what makes pointers comparable. Otherwise the same address could be reused by different text.
- **`TextRevision` is in the key, unlike in ShapeKey**: `TextLayout` carries the revision it was produced from, and
  `is_stale` compares against it. A layout handed to a second source therefore cannot still carry the first source's revision.
  That source would read its current layout as stale on every frame, forever. Two nodes with the same text at different revisions
  therefore each get their own layout. The shaping underneath is still shared, and that is where the heavy work is.
- `tab_width` is in the key too, even though this phase does **not** implement tab stops (`\t` is shaped as an ordinary character): it is a declared
  constraint. A key that leaves one out means the cache will return the wrong layout the day it is implemented.
- `ConstraintsKey` is written by destructuring field by field. Adding a field to `TextConstraints` fails to compile here instead of
  silently producing a cache that ignores the field.
- LRU, with two limits: entry count (default 4096) and bytes (default 8 MiB). A result larger than the whole byte budget is returned as usual and not cached.
  Shaped text is charged to the shape cache. The layout cache does not count it again.
- The cache also answers a question nothing else does: was this miss **new text**, or **the same shaped text under new constraints**
  (`constraint_only_relayouts`)? That is the number to watch during resize storms.

### Not done in this phase

| Item | Status |
| --- | --- |
| Tab stops | `tab_width` is in `LayoutKey`, but no line applies tab stops; `\t` is shaped as an ordinary character |
| Runtime mixed "text + atomic box" flow | Atomic boxes take part in CJK line decisions only in nana-text's `InlineObject` (the RichText path); boxes in a Runtime inline context are still greedy at box level (#188/#190/#195 not implemented) |
| Vertical editor decorations (#59) | The line-number gutter, minimap, guides, end-of-line diagnostics, and popups are not drawn in a vertical editor; see above |
| CSS whitespace collapsing | Not done at all: consecutive spaces are kept as is, so under `preserve_lines: false` CRLF folds into **two** spaces, the same as typing two spaces; callers that want collapsing normalize the text themselves (moving offsets is then their own business) |

That "frames that only change color / transform produce no layout requests" in UiWorld is verified on Runtime by Phase 4.
This phase guarantees that they cannot enter `LayoutKey` at all. `TextStyle` carries no paint, and `LayoutRequest` has no second path.

### Intrinsic min/max content width

`Layouter::intrinsic_widths`: `min-content` is the widest unbreakable piece. `max-content` is the width without wrapping.
Both are computed from shaped advances and UAX #14 opportunities, regardless of whether the current constraints allow wrapping. A container asks for these two numbers precisely to decide its width.
Not cached: it is one pass of summing advances, and there is no width to key on.

### Writing mode and #59

`vertical-rl` / `vertical-lr` really lay out in columns, editable text included. The approach is to **lay out vertical text in line-relative
coordinates and convert coordinates only once, at the boundary**. Line breaking, alignment, truncation, and caching never know the page was turned:

- **Orientation**: `font/unicode.rs` uses ICU's `Vertical_Orientation` (UAX #50) to give each grapheme cluster an orientation.
  `U` / `Tu` / `Tr` are upright. `R` is sideways (`text-orientation: mixed`). `Tr` (brackets, the prolonged sound mark, dashes) is also treated as
  upright: what they need is the vertical **glyph**, supplied by `vert` substitution on the upright run (with HarfRust's Unicode vertical
  presentation forms as a backstop). Orientation takes part in item splitting, alongside font, bidi level, and script.
  `text-orientation` (`nana_ui_core::TextOrientationSpec`; Rust's `LayoutStyle::text_orientation` and
  CSS share one enum; it inherits with `writing-mode` and reaches `ShapeRequest` through `TextConstraints::text_orientation`)
  decides whether this table is used: `mixed` decides per cluster as above, `upright` makes everything upright, `sideways` makes everything sideways. `upright` also treats
  every character as strong LTR (bidi level always 0) and makes the box's **used value** of `direction` `ltr`
  (CSS Writing Modes §5.1; given in one place by `WritingContext::used`, so box layout, logical edges, text constraints, and the painter
  read the same value). Descendants still inherit the computed value. It has no effect on horizontal text, and the shaping cache key records it as `mixed` there.
- **Shaping**: upright runs are shaped with HarfRust `TopToBottom`. It handles the font's `vmtx` / `vhea` and `vert` / `vkna`
  substitutions. `y_advance` is negative and is converted to a positive length along the line on entering the IR. Offsets keep HarfRust's convention
  (relative to the horizontal origin, with the vertical origin `(h_advance/2, v_origin_y)` already subtracted). A sideways run is just horizontal shaping, rotated
  90° clockwise by the painter. `ShapedRun::orientation` records which one it is. `ShapedGlyph::advance_px` is the step **along the line** in all three orientations.
  `ShapedText::vertical` is layout's only source for the writing direction, and the shaping cache key includes it.
  When a font has no `vhea` / `vmtx` (including fonts pulled in by fallback, such as Noto Sans Arabic), the along-column
  step of upright glyphs is synthesized by HarfRust from the `hhea` ascender − descender. That is the font's line height, always positive,
  never 0 or negative. The text stays vertical and does not fall back to horizontal.
- **Layout**: `max_width_px` / `max_height_px` are always the **physical** box. `TextConstraints::inline_budget_px`
  / `block_budget_px` turn them into the line budget and the stacking budget. In vertical text, the height governs how long a column is and the width governs how many columns can stack.
  The vertical baseline is the column center line (`baseline_y_px = top + height/2`). The line box height is the column width. `LineBox` / run geometry
  is still line-relative: `x` runs down the column, `y` counts from the block-start column. Line space is measured from line-left (the top in vertical text) and does not change with
  `direction`. `direction: rtl` only moves `start` alignment to line-right, which in vertical text is the bottom. The conversion is given in one place by
  `nana_ui_core::WritingContext`. Box layout, the painter, and the editor all use the same one.
- **Mapping**: `TextLayout::physical_x_of_block` and `physical_size` are the only logical-to-page conversion.
  `vertical-rl` stacks columns leftward from the box's right edge. Runtime's `text_metrics_of_layout` reads `physical_size`. Vertical text
  reports no ascent (columns hang on the center line; handing that to box layout that aligns on the alphabetic baseline would only misalign).
- **Runtime constraints**: after measuring the box, vertical text always passes the content height as `max_height` (symmetrical to horizontal text always passing width).
  Width is only given when truncating. `nana_text_constraints` decides by writing direction which dimension is the truncation budget.
  Upright run steps are vertical metrics and are not written into the per-character width cache horizontal rich text reads. The single-character fast path does not answer for vertical text either.
- **Painter**: the column center line is aligned to whole pixels. Upright glyphs are placed at `(center line + offset_x, pen − offset_y)`. Sideways runs center the
  em box on the center line and rasterize with `GlyphSynthesis::ROTATE_CW` (which goes into the raster key). An entry anchors on the right edge of its own column stack,
  and any extra box width is added to the draw origin, so when the box only gets wider the instances are reused wholesale.

**Editable text** also works in line space. Every `EditorGeometry` query (`caret_rect`, `hit_test`,
`selection_rects`, line movement `vertical`, along-line movement `visual_move`) uses each paragraph layout's own coordinates. In vertical text
`x` runs down the column and `y` crosses columns. Not one line of code branches on direction. Paragraphs stack in the block direction, so a multi-paragraph editor is several groups of columns. The truncation budget also looks at the
stacking direction (`splits_paragraphs` reads `block_budget_px`). In vertical text the height is only the column length, so it does not get in the way of incremental per-paragraph layout.
Runtime converts once at the boundary, all of it in `EditorFrame`. It does not care about horizontal or vertical. Every editor has one: the content box (minus
a number input's stepper buttons), along-line / cross-line scrolling in line space, and for single-line inputs cross-line centering and line-start anchoring (horizontal LTR sticks left,
RTL right, vertical top, vertical rtl bottom; each is one scroll amount). When a single-line input overflows, it only scrolls far enough to reveal the caret. Otherwise the line start stays visible: LTR shows from the left end, RTL from the right end, vertical rtl from the bottom end. These are mirrors of the same rule, and no state beyond the caret is recorded. Every rectangle of the component geometry and every pointer hit goes through it. Horizontal text no longer
has a separate scroll computation. A single-line input's editing geometry has no line budget, so the text area takes the full line length. The painter's `start` alignment has no slack
and cannot move the glyphs. A multi-line editor's text area takes the content box's length in the line direction, the same as the editing geometry's wrap budget.

- **Geometry**: caret, selection, preedit, and multiple carets are drawn from text space to the page through `field_rect` (the caret becomes a
  horizontal bar across a column, and the preedit underline is drawn on the block-start side of the column). The text area of an editor value is given by `text_bounds`. Its right edge (`vertical-rl`)
  coincides with the caret's anchor. For multiple lines, its height is the content height used for column wrapping; the painter and the editing geometry wrap columns with the same budget. A single-line input
  centers that column horizontally in the box (a negative block-direction scroll).
- **Horizontal scrolling goes to the painter as a translation**: a multi-line editor's geometry also carries `scroll` (`EditorFrame::scroll_x`). See "Horizontal editor scrolling is also a translation".
- **Pointer**: a page point is converted back to text space through `text_point` before hit testing. It is the same conversion as drawing the geometry.
- **IME**: the cursor area the host hands to the input method is the caret the editor draws (the `caret` of `ComponentGeometry::TextInput`).
  In vertical text that is the horizontal bar across a column. The candidate window sits next to the character being
  edited, not next to the whole input box.
- **Accessibility**: an accesskit `TextRun` carries `text_direction` by inline start: horizontal LTR / RTL, vertical
  `TopToBottom`, vertical rtl `BottomToTop`. The value is the text in logical order. Per-character positions and widths (`character_positions`
  / `character_widths`) are not provided yet, for horizontal text either.
- **Arrow keys**: intents (`TextCaretIntent`) are defined in line space: left/right along the line, up/down across lines. Hosts report physical keys,
  so **before** folding modifiers into an intent, the host first asks `AppContext::focused_text_line_space_key`. In vertical text it turns the physical key
  into a line-space key: up/down move along the column, left/right cross columns (in `vertical-rl` the left key goes to the next column). What gets swapped is the key, not a finished intent,
  so the whole modifier set follows: Cmd+↑/↓ go to the start/end of the line along the column (a logical line, like horizontal Cmd+←/→). In `vertical-rl`,
  Cmd+← is the end of the document and Cmd+→ the start. Alt/Ctrl+↑/↓ move by word. Alt+←/→ move a block of lines, and
  Alt+Cmd+←/→ add a caret in the adjacent column. A page for PageUp/PageDown is the viewport width in the cross-column direction
  (`text_input_page_extent`).
- **Vertical rtl**: the line start is at the bottom. A multi-line editor's editing geometry and painter share the content height as the line budget, and each aligns
  `start` of every column to the bottom. A single-line input's editing geometry has no line budget, so the text area takes the full line length (the painter's `start` alignment
  has no slack and cannot move the glyphs). The coordinate frame then sticks it to the bottom with a negative along-column scroll. Arrow keys follow the horizontal RTL convention: paragraphs
  read bottom to top. ↑ goes forward; passing the top of a column continues into the bottom of the next column. ↓ is the reverse. Within a column it still moves visually cell by cell. A caret at the end
  of a paragraph lands at line-left (the top), for the same reason horizontal RTL draws the end on the left.
- **Scrolling**: the recorded `ScrollOffset` is a physical offset, like every scroll container. `ScrollMetrics` come live from the shaping result
  (`text_scroll_metrics`): `vertical-rl` columns start from the right and later columns overflow to the left, so the cross-column offset
  goes from 0 toward negative (CSSOM's negative `scrollLeft`). Wheel, `scroll_to`, and clamping take the same path as other containers. The recorded
  offset is a **request** (written by component field projection, the wheel, or the minimap). The painter and hit testing clamp it into the value's
  range in `editor_frame` and turn it into two line-space quantities, along-column and cross-column. A focused multi-line editor then scrolls the caret into the viewport by the same rule as horizontal text.
  `UiWorld::scroll_offset` reports this drawn position, so the wheel and the application read the position on screen: after typing,
  the wheel continues from where the caret is, and after the value gets shorter the first notch moves. Viewport pinning compares requests (`scroll_request`).
- **What is not drawn**: the line-number gutter, minimap, indent / column guides, end-of-line diagnostic text, and completion / hover / signature popups,
  code editor decorations laid out along horizontal lines, have no honest vertical version. A vertical editor draws none of them rather than drawing them sideways across the columns.

`TextLayout::unsupported_writing_mode` and `vertical_writing_fallbacks` remain, with only one possible source left: a caller
shapes horizontally with `Shaper` itself and then lays out with vertical constraints in `Layouter`. The engine shapes and lays out with the same constraints, so it never gets
here. `writing-mode: sideways-*` is rejected earlier by box layout (`unsupported_writing_mode`) and never gets here.

The selection of **static selectable text** (`user-select`) does not use editor geometry: vertical nodes read that layout directly — the one Runtime
retains and the painter also draws. Pointer points are converted into line space through `TextLayout::line_space_point` for `hit_test`.
`selection_rects` are converted back to the page through `page_rect`. The anchor is the content box's right edge (`vertical-rl`), the same as the painter.

### Counters

`Layouter::counters()` → `LayoutCounters`:

```text
layout_requests / layout_created
layout_cache_hits / misses / evictions      layout_cache_bytes / entries (values at read time)
line_break_candidates                       break opportunities examined (always 0 on the fast path)
lines_created / runs_placed / ellipsis_runs_used
constraint_only_relayouts                   same shaped text, new constraints
shape_runs_reused_for_layout                runs read by layout instead of reshaped
label_fast_paths / paragraph_paths          which path was taken (only layouts actually built are counted)
vertical_writing_fallbacks                  vertical requests served by the horizontal fallback (when shaping and layout disagree on direction)
```

`vertical_writing_fallbacks` is folded into `TextWorkCounters` (#59): the horizontal fallback itself is correct, and reporting horizontal metrics
as vertical would be worse. But it is **invisible on screen**, so it has to make noise in the counters. Through the engine it is always 0. `> 0`
means some caller bypassed the engine and shaped and laid out with different writing directions. `TextLayout::unsupported_writing_mode` is the per-node version of the same thing,
carried along with the retained layout all the way to the scene.

Reconciliation tests assert that `lines_created` / `runs_placed` equal the line and run counts of the layout they claim to describe, and that
`TextWorkCounters::glyphs_resolved` equals `layout.glyph_count()`.

### Engine seam: `NativeTextEngine`

`nana_text::NativeTextEngine` implements `TextEngine`. It puts the #90 font layer, the #91 shaper, and the #92 layouter behind one call:

- With `preserve_lines: false`, single-byte line separators (`\n` / `\r` / VT / FF) are folded into spaces **before** shaping
  (`TextSource::with_folded_newlines`). Shaping and line breaking must see the same bytes. They are all single bytes, so
  all span ranges, clusters, and caret offsets stay the same. The revision stays the same too (it is another reading of the same edit).
  The folded result is memoized on the source per revision, so per-frame relayout neither copies nor rehashes.
  `U+2028` / `U+2029` are three bytes each. Folding them would shift every later offset, so they remain line breaks whatever `preserve_lines` says.
- Shapes `…` when needed. Takes the metrics of the base style's face as the strut. Fills the five measures of `TextWorkCounters`.
- `TextEngine::layout` returns `Arc<TextLayout>`: layouts are immutable. Within one frame, the same text under the same constraints should get **the same one**,
  not a copy of it.

Runtime holds it through `NanaTextEngineShaper` (see "UiWorld retained text nodes"). Since #99, the product host's
`NanaTextShaper` **is** a shell around it. Drawing goes through `SceneWgpuPainter::text`. Both sides ask the same engine.

### Reconciliation with the cosmic goldens

`tests/layout_matches_the_cosmic_reference_goldens.rs` uses **the same** `compare_golden`, the same goldens, and
the same tolerances. It compares the native layout of the 26 corpus cases field by field with the reference layout recorded in Phase 0,
and reruns each case's own hit-test / caret probes on the **native layout** and compares those too.

Result: all 26 match. Only three kinds of delta are allowed, and all of them are about "how the two engines describe it", not "laid out differently":

| Field | Reason |
| --- | --- |
| `script` | The reference engine does not export per-run script and records `Zzzz` (a gap already documented in Phase 0) |
| `run_count` | Runs are split in different places: native splits by script / style segment, cosmic by face / level / size. The glyphs are the same, only the grouping differs |
| The `ELLIPSIZED` bit of `overflow` | cosmic cannot draw an ellipsis, so the reference sets the bit directly for "ellipsis requested and truncated"; this reconciliation run also provides no shaped ellipsis, so native reports only `TRUNCATED_LINES`. All other overflow bits are compared exactly, bit by bit |

`compare` stops descending on a count mismatch **by design**, so allowing `run_count` would leave those lines' geometry unchecked.
The same test therefore compares again per **line × glyph cell** (glyph id, cluster, cluster_end, x in visual order).
This comparison is independent of grouping and covers exactly those three cases. Another test removes the filter and asserts that the remaining fields are **only** these three,
so the filter cannot hide a fourth kind of difference.

Reconciliation runs in "no strut, no ellipsis" mode: those are exactly the two places where the native engine deliberately differs from the reference
(the strut stabilizes the baseline, the ellipsis is really drawn). Each has its own fixture.

### Tests

`tests/layout_engine.rs`: twenty-two regressions from code review (hanging whitespace on RTL lines hangs outside the start edge,
the caret at an RTL soft wrap stays on its original line, height truncation reports `MaxHeight`, leading paragraph whitespace produces no empty line,
the ellipsis is not drawn outside the container, U+001C–U+001E end a line and are not drawn, line boxes use the span's line height when a span boundary falls mid-grapheme,
a Label with a trailing line break also falls back, over-wide lines overflow past the start edge according to alignment, an over-wide line with wrapping on does not lose bytes to the ellipsis,
hanging trailing whitespace counts as neither overflow nor affects alignment, empty text produces one line on both paths, a trailing line break leaves an empty line the caret can land on,
a truncated empty line keeps its own bytes, a line cut twice records only one ellipsis run, U+2028 ends a line and is not drawn,
a layout carries the requester's revision, `max-content` is the widest line, a caret inside hanging whitespace has a position,
folding happens only once per revision, separators that cannot be folded still break lines). Plus: Label fast path (10k labels with `paragraph_paths == 0`,
`line_break_candidates == 0`; 10k labels with the same text build only one layout), explicit line breaks / wrap / max-lines trigger fallback,
a width change relayouts without reshaping, resize touches only the affected paragraph, word wrap does not split words, long words overflow or split according to `word-break`,
line breaking of Han text without spaces, explicit line breaks and empty paragraphs, mixed BiDi on a single line and per-line reordering after wrapping, the strut stabilizes the baseline (and without a
strut the baseline really does move), `line-height` and half-leading, fractional scale, six alignments × directions,
max-lines and max-height truncation, cluster-safe ellipsis cutting and zero-byte occupancy, the ellipsis shaped only once,
intrinsic min/max, vertical fail-closed, layout cache limits and LRU, font generation making old layouts stale,
counter-to-output reconciliation, caret / hit-test / selection running directly on the native layout.

## UiWorld retained text nodes (Phase 4, #95)

Runtime keeps only logical state, revisions, and handles for text. The immutable results of shaping / layout are held by `nana-text`.

```text
UiWorld node's TextNodeState (NodeStore side table, one entry per node)
  revisions { content, shape, constraint, paint, edit }
  stamp     the content / shape / constraint revision at resolve time + backend generation
  layout    TextLayoutId (this UiWorld's TextLayoutStore)
        │
        ▼
NativeTextEngine (SharedTextEngine, shared across documents / windows)
  ShapeRun cache ── Layout cache
        │
        ▼
ExtractedNode.text_layout ──► ScenePrimitiveKind::Text { layout: Option<RetainedTextLayout> }
```

### Dirty graph

`nana_ui_runtime::TextDirty` says "what changed". `TextDirty::work()` is the single source of the dependency graph.
`TextNodeState::invalidate` bumps revisions only through it:

| Class | Work | Revision bumped |
| --- | --- | --- |
| `CONTENT` / `FONT` / `SHAPE_STYLE` / `LANGUAGE` | shape + layout + scene geometry | content (only `CONTENT`), shape, constraint |
| `CONSTRAINT` | layout + scene geometry | constraint |
| `EDIT_STATE` | editor overlay (Phase 5) | edit |
| `PAINT` | re-extracts paint only | paint |
| `TRANSFORM` / `OPACITY` | compositor only | none |
| `GLYPH_PRESENTATION` | compositor only (per-glyph effects on rich text, reveal) | none |

`TextDirty` / `TextWork` are `u16` bitsets. `GLYPH_PRESENTATION` is the ninth class and `LANGUAGE` the tenth, so neither fits in `u8`. `GLYPH_PRESENTATION` touches no revision, not even paint, just like transform / opacity. A span that changes only its effect index is therefore not shaped again, not laid out again, and does not rebuild glyph instances.

Changes are classified where they happen, not by comparing afterwards:

- `SetText`: `CONTENT` only when the text really changed. `SetTextInput` / `ReplaceTextSelection` count every time (editor state itself changes every time). Any content invalidation that empties the text releases the retained layout immediately (a non-`Text` element without text no longer enters any text pass). `SetIme` / `SetTextSelection` are `EDIT_STATE`.
- When computed style lands (`world/style.rs`), `classify_computed_style_change`: font family / font size / weight / italic / letter spacing / feature / variation axis / kerning / direction → `SHAPE_STYLE`. Line height / word-break / line-break / writing-mode / CJK typography (`text_typography`) → `CONSTRAINT`. Color / foreground role / selection color → `PAINT`. opacity → `OPACITY`. The computed `language` → `LANGUAGE`. The remaining fields produce no text work.
- `SetStyle` compares only the `LayoutStyle` fields that text constraints really read (wrap / white-space / ellipsis / line-clamp / whether height and max-height are definite / border / padding / alignment). paint, transform, and opacity sit on the same `LayoutStyle` and are **deliberately not compared**. Every other constraint input schedules layout and enters the layout-scoped pass. Alignment moves no box, and only the retained layout reads it (host metrics do not). So it records `CONSTRAINT` and marks TEXT only on nodes that hold a layout. The layout-scoped pass skips invisible nodes, so a pure-text node going from invisible to visible is resolved again by the same frame's scheduling pass; no extra flush pass runs (editor / EmptyState / ModalFrame text is re-measured every pass anyway and is not in this set). If that pass fails, they are left for the retry.
- `WriteLayout` is `CONSTRAINT` only when the box size changed. A move alone (#33's head insert) is not. Padding written back by layout, and layout-type animations (width / height / padding) that write style directly, likewise record `CONSTRAINT`.
- A visual change records `CONSTRAINT` and marks TEXT only when "which path the text takes / how wide the leading indicator is" changed (`text_visual_key`; adding a Checkbox does not change the box size). Sampled values such as progress do not count.
- A font-set change does not bump per node: the resolve stamp carries the backend generation (the host's shaper type plus `TextShaper::font_generation()` — another shaper is another measurement — or the engine's font-system identity + `FontGeneration`). `RuntimeDocument::flush` calls `UiWorld::observe_text_shaper` at the start of every frame. When the generation changes, it schedules the resolved nodes that were measured (including an empty Text node, which only measured an empty line box) together with EmptyState / ModalFrame / TextInput into this frame (`FONT`) for reshaping. A static document is settled again on the new fonts too. The per-character and per-style glyph advance cache and the host path's `TextLayoutCache` are cleared at the same time (their keys do not include the measurer; the same clearing happens when the shaper changes). After reshaping, only text whose exported metrics (width, height, baseline, natural width) really changed schedules a layout seed; text whose metrics did not change under the new fonts schedules no layout at all.
- Language is not in the backend generation. The engine's `set_language` is only a fallback hint; it reaches text through computed style like any other language (see the next section).

transform / opacity bump no revision (and Runtime never needs to emit `TRANSFORM` either: transform is never an input to text). A steady-state compositor animation therefore cannot touch shaping / layout by construction. `retained_text_node.rs` verifies with real animation frames: such a frame produces no pass of Runtime work, text pass included. Revisions stay as they are. At the style level, opacity maps to `OPACITY` and color to `PAINT`, guarded by the color / opacity / transform style-change tests.

Now the text pass consumes this graph: it reads only the content / shape / constraint revisions to decide whether to redo shape / layout. Scene extraction still re-extracts the whole node on a RENDER dirty. `SCENE_PAINT` / `SCENE_GEOMETRY` / `EDITOR_OVERLAY` / `COMPOSITOR`, with the `paint` / `edit` revisions, are the contract for drawing the retained layout (#97 changed the renderer; #99 was what finally moved the product path's paragraphs onto `TextLayout`) and for editable path #96. There is no finer extraction consumer yet.

### Language scope and external metrics (#260)

Text is shaped in a language chosen in this priority order:
1. The language the node names itself: `MutationQueue::set_language`, the equivalent of HTML's `lang`.
2. The language inherited from the subtree.
3. The app's default language: `AppContext::set_default_language`.
4. The text engine's fallback hint: `NativeTextEngine::set_language`.

The resolved result is the inherited computed style field `ComputedStyle::language`. It enters nana-text's `TextStyle::language`, and through that `ShapeKey` and font fallback lookups. It also reaches the painter via `SceneTextOpenType::language`; when the painter lays text out again it uses the same language, so it never draws `locl` glyphs different from the ones measured.

A language change re-resolves the style of the affected subtree only:
- Text whose computed language really changed records `LANGUAGE` and is shaped again;
- Text shadowed by a deeper explicit language keeps its computed style and its revisions untouched;
- Text outside the scope is not touched at all.

Changing the app default language or the engine fallback affects all text that inherits the root language; changing the fallback while an app default language exists does nothing. A language change itself schedules no layout: layout is scheduled only when the metrics change after shaping.

Shaped text propagates to the parent only through the exported external metrics (width, height, baseline, natural width). An edit that leaves the metrics unchanged, such as `"99"` to `"98"`, stops at the text and does not reflow the parent.

When the parent constraint changes only the width, text breaks lines again from the existing shaped result without shaping again. Typing in a fixed-size input reflows nothing outside its box. Cursor and selection movement never enters box layout.

The counters are all on `WorkCounters`:
- `text_reflow_seeds`: layout seeds scheduled after text shaping;
- `text_external_metric_changes` / `text_external_metric_unchanged`: text whose external metrics changed or did not change after reshaping;
- `text_parent_reflows`: parents that text asked to re-measure;
- `text_constraint_relayouts`: text that broke lines again without shaping again (nana-text's `constraint_only_relayouts`);
- `text_language_scope_invalidations`: text touched by a language change;
- `text_literal_nodes_invalidated_by_language`: text reshaped by a language invalidation although its language did not change. Normally 0; it guards against anyone making language invalidation coarse again.

The gate is in `world/tests/localization.rs`, five items:
- Typing continuously into a fixed input in a 1k and in a 10k document costs the same;
- `"99"→"98"` reflows zero parents;
- 240 width changes trigger no reshaping;
- A scope language touches only the text inside the scope;
- Cursor and selection trigger zero box layout.

The #259 scale matrix has a language scenario too.

### Text scale scope (#266)

Accessibility font size and the app's content text-size level are both scoped dependencies retained in the tree, not one process-global float.
Text is laid out at a scale chosen in this priority order:
1. The scale the node sets itself: `MutationQueue::set_text_scale`. The subtree inherits it; setting it to `1.0` pins the subtree to the original font size, which is the explicit font-size override.
2. The scale inherited from the parent.
3. The window's scale: `AppContext::set_document_text_scale`.
4. The app's scale: `AppContext::set_default_text_scale`.
5. 1 when none is set.

The scale must be a finite positive number. Mutation validation rejects an invalid value; the window and app setters ignore it outright.

The resolved result is `ComputedStyle::text_scale` on the computed style:
- `font_size` is the actual scaled font size; `font_size_base` is the font size before scaling, and child nodes inherit the latter, so nested scopes never multiply the scale twice;
- Absolute line height works the same way: `line_height` is scaled, `line_height_base` is what inherits;
- A relative line height follows the font size anyway.

The scale affects text only:
- Box lengths resolve against the pre-scale font size, including `em`, `rem`, and the line box an element without text keeps for its own font size;
- Images, custom rendering, and paint-only surfaces are not affected;
- Internal labels drawn in component geometry (form field labels and hints, progress bar labels) use the component's own fixed font size and do not follow the scale.

A scale change visits only its scope: the scope root and the nodes below it that inherit this scale. It stops at a node that sets its own scale and does not enter its subtree.
Visited nodes re-resolve style; text whose computed scale really changed is laid out again and repainted.

Layout learns about the change only through the external metrics that changed:
- A box whose size follows its text propagates the metric delta to the parent;
- A fixed-size box (both width and height in px, no min/max): as long as it is not inline-level and the parent does not baseline-align it, the text change stops inside the box and nothing above it measures. A fixed-size button is like this;
- A box the parent baseline-aligns exports a new baseline, so only the line it is on realigns; lines that are not baseline-aligned do not enter measurement.

A window scale visits only that window's root; an app scale visits only windows that set no scale of their own. Setting a scope, a window, or the app to the value it already has does nothing.

The counters are all on `WorkCounters`:
- `typography_scale_scope_nodes_scanned`: scope nodes visited by a scale change;
- `typography_scale_dependents_notified`: laid-out text whose computed scale changed;
- `typography_scale_text_relayouts`: of those, the text later laid out again at the new font size;
- `typography_scale_parent_reflows`: of those, the text whose metric change made the parent re-measure;
- `typography_scale_equivalent_skips`: scale settings that set the existing value and visited nothing.

Scope visit counts and equivalent skips are counted at commit, but recorded on the frame that processes the change.

The gate is in `world/tests/localization.rs`, five items:
- In a 10k and in a 100k node document, a fixed panel with 2000 text runs scales from 1 to 1.25. Only the panel's own 3001 nodes are visited; a nested scope that pins its scale is not visited; no text, style, or measurement outside the panel changes; both sizes cost the same;
- 1000 fixed-size buttons: labels are laid out again inside the buttons, the parent reflows zero times, and the spacer elements between the buttons do not enter layout;
- Baseline: a baseline-aligned line realigns, and the button that pins its scale aligns to the new baseline; a line that is not baseline-aligned, and baseline lines outside the scope, are not measured;
- Setting the existing value does zero work;
- Two windows of 10k nodes each: one window's scale does not touch the other window's text or layout.

The #259 scale matrix has a scale scenario too: a card sets a scale on its label.

### Localized text and locale scope (#268, #269)

A node's text is either literal text or localized text:
- Literal text (`SetText`) is displayed as written, belongs to no locale, and a locale switch does not touch it;
- Localized text (`MutationQueue::set_localized_text`) is one message plus arguments: `LocalizedText { message, args }`.
  It resolves against the locale of the scope the node is in, and resolves again when the locale changes. `SetText` on a localized node returns it to literal text.

Locale scopes have three levels:
- Subtree: `MutationQueue::set_locale(node, Locale)`;
- Window: `AppContext::set_document_locale`;
- App: `AppContext::set_default_locale`, which every window without a locale of its own uses.

A `Locale` describes four things separately, each with its own generation (`UiWorld::locale_generations`):
- Which message catalog to use;
- Which language localized text is laid out in (font fallback, `locl`): by default the locale the message actually came from, and it can be named separately;
- The scope's writing direction: by default the script of the message locale (Arabic, Hebrew, and so on are right-to-left), and it can be named separately;
- Which locale numbers and dates are formatted in.

Messages come from `MessageCatalog`. It is a backend-agnostic interface: it only answers the pattern string of one message under one exact locale.
The runtime walks the fallback chain: first the prefixes of the scope locale (`zh-hant-tw` → `zh-hant` → `zh`), then the configured fallback locale (`UiWorld::set_fallback_locale`). When neither has it, the result is `MissingMessage`: show the message key, or keep what was displayed before. `MessageTable` is a ready-made table implementation. Message IDs are interned integers; the hot path does not look up strings in a table. Lookups are cached by (requested locale, message); the cache is cleared when the catalog or the fallback locale changes.

Dependency index: each localized node registers under the scope whose locale it reads (the nearest subtree scope, otherwise its window).
- A switch visits only the localized nodes registered under the switched scope; it does not scan the document, does not touch literal text, and does not touch other windows;
- A message-only change does not change language or direction, and the language and direction dependencies do not invalidate either;
- A node whose resolved result is the same is not laid out again and does not lay out boxes; only nodes whose layout language changed are laid out again: the language the locale names changed, or, when it names none, the locale the message actually came from changed;
- Direction changes the scope root's layout intent only when the scope's direction really changed (a subtree scope lands on the scope node, a window on the window root, and only a right-to-left window sets it); after that it follows the ordinary writing-direction change path, #254;
- A switch between the same direction (en-US → zh-CN) triggers no writing-direction relayout;
- A switch is one transaction: everything resolves and writes within the same call, so the next frame is either fully the new locale or fully the old one; there is no half-Chinese half-English frame;
- Rows a virtual list has not mounted are not nodes, and a switch does not resolve them; when they mount, they resolve directly at the current locale.
- When a scope is created or removed, or a node is reparented or detached (Detach, Park), the affected localized nodes are moved from the old scope's registration, and only the old scope's nodes are visited.

Text that does not live on a node (control labels, notifications, native dialogs, computed captions) uses `MessagePattern`: `parse` once, then `format(locale, args)` yields the string, with the same syntax and formatting as localized nodes; a missing argument shows `{name}` as written. It does not enter the dependency index; on a locale switch the caller formats it again.

The user's UI language is read by `nana_ui_platform::preferred_languages()` (the preferred language list on macOS/iOS, the user interface language on Windows, and on other Unix systems `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG` in that order), giving BCP 47 tags in preference order. `negotiate_language(preferences, languages the app provides)` picks one: each preference is first matched only within the same script (same region first, then no region), and only when all of them miss does it accept the same language in another script; a Chinese tag without a script is inferred from the region, with Taiwan, Hong Kong, and Macao as `Hant` and the rest as `Hans`. When to ask, whether to follow the system, and where the choice is stored are all up to the app.

The counters are on `WorkCounters::i18n` (`I18nCounters`):
- Index sizes: `localized_nodes`, `message_dependencies`, `language_dependencies`, `direction_dependencies`;
- `literal_nodes`: literal text resolved by a switch, always 0 (sentinel);
- `catalog_lookups` / `catalog_cache_hits` / `catalog_cache_misses`;
- `scope_dependents_notified`: localized nodes reached through the scope index;
- `switch_transactions` / `switch_commits`: switch transactions and commits that land as a whole;
- `nodes_resolved`, `resolved_content_changed` / `resolved_content_unchanged`;
- `virtual_rows_resolved`: localized nodes resolved in rows mounted by a virtual list, counted both at mount and at switch;
- `language_changed_nodes`, `direction_changed_scopes`, `layout_seeds` (the seeds the switch schedules itself, i.e. direction);
- Formatting counters are in the next section.

Switches are counted at commit and recorded on the frame that processes them.

The gates are in `world/tests/localization.rs` (five items) and `world/tests/localization.rs` (six items):
- A static frame does not query the catalog, visit a scope, or resolve;
- 100k nodes, about 5% localized: a message-only switch notifies exactly those localized nodes; literal nodes, language, and direction stay put;
- Two windows: switching one leaves the other window's text, style, and layout out of the next frame;
- The index changes only with registration: it returns to its original state after 10k mounts and unmounts;
- A switch does not resolve literal text;
- en-US → zh-CN: resolution is exactly the localized nodes, no direction change, layout starts only from that text and the containers it is in, and there is no whole-document fallback;
- Equivalent translation: strings that read the same are not laid out again; the next frame lays out only the ones that changed;
- 1000 fixed-size buttons switching language: labels are laid out again inside the buttons, and the buttons' positions and sizes stay unchanged;
- A subtree scope switching to right-to-left: only that subtree enters the next frame;
- A virtual list of one million rows: only mounted rows resolve, all counted as virtual rows, and logical rows are not queried;
- A switch lands as a whole before the next frame.

#259 adds a locale load: the last 40 localized labels of a 1k, a 10k, and a 100k window switch locale (same direction, equivalent translation, and switching to right-to-left once it becomes a subtree scope), at the same cost; a localized virtual list switches at 100k and at one million rows, at the same cost.

Message formatting (#270). Pattern strings are written in a common subset of ICU MessageFormat:
- `{name}`: the argument as is (text as written, numbers per locale, dates and times at medium length);
- `{name, number}`, `{name, number, integer}`, `{name, number, percent}`, `{name, number, currency}`;
- `{name, date}`, `{name, time}`, `{name, datetime}`, with an optional `short` / `medium` / `long` / `full`;
- `{name, plural, offset:1 =0 {..} one {..} other {..}}`, `{name, selectordinal, ..}`, where `#` is the number minus offset;
- `{name, select, a {..} other {..}}`;
- `''` is an apostrophe; when an apostrophe is directly followed by `{`, `}`, or `#`, it quotes up to the next lone apostrophe.

Arguments are typed `MessageArgs`: text, numbers, currency (an amount and an ISO 4217 code), dates and times.

Formatting goes through the backend-agnostic `LocaleFormatter` interface:
- The runtime installs `IcuFormatter` by default (`icu` feature, on by default), using ICU4X and the CLDR data compiled into it: plural rules, numbers, dates, and times;
- With the feature off it uses `PlainFormatter`: English plural rules, numbers with `.` as the decimal separator, ISO dates;
- `nana-ui` and `nana-ui-scene` forward `icu` explicitly and keep it in their default features; `nana-ui`'s `full` includes it too. An app can use `default-features = false` and enable only the components it uses, to avoid ICU formatting and the CLDR data. The workspace's internal runtime/scene dependencies do not inherit default features, which keeps it from being turned back on; the ICU that `nana-text` needs for Unicode properties and URL handling is not affected by this switch.
- An app can also install its own implementation: `UiWorld::set_locale_formatter`. ICU types do not cross this interface.
- Currency is currently written as "code + two decimals grouped per locale" (`USD 1,234.50`). ICU4X's currency formatting is still experimental and is not used.

Caches:
- A message is compiled once at lookup and cached by (requested locale, message). One message under one locale is compiled once, not once per node;
- The formatter for each locale and option (plural rules, numbers, dates, times) is built on first use and reused from then on;
- Formatting writes into a reused buffer.

Argument revision: changing the arguments of the same message on the same node counts as one argument revision and reformats only that node. When the output is the same as before
(absorbed by rounding, for example), neither text layout nor box layout runs.

Catalog hot update: `UiWorld::update_message_catalog(new catalog, messages that changed)` re-resolves only the nodes that speak those messages, found through the message-to-node index, without scanning the document; `set_message_catalog` re-resolves every localized node.

Diagnostics go through `nana-diagnostics`: missing messages (reported once per locale and message), patterns that are not valid patterns (shown as written),
missing or wrongly typed arguments (the argument name is written into the output). None of them panics an interface frame.

Formatting counters are on `I18nCounters` too: `format_requests`, `format_cache_hits` / `format_cache_misses` (formatter cache),
`message_patterns_compiled`, `args_revisions`, `formatted_output_changed` / `formatted_output_unchanged`,
`catalog_messages_invalidated`, `formatter_allocations` (formatters built).

The gates are in `world/tests/localization.rs` (five items) and `tests/perf_contracts/i18n_alloc.rs`:
- Among 10k localized nodes, one counter changes its number every frame: one format request per frame, no extra catalog lookups, no other node formatted, and only the counter is laid out;
- An argument change absorbed by rounding: the output is unchanged, so neither text layout nor box layout runs;
- 10k numbers under one locale: the pattern is compiled once and one formatter is built; after switching to German the formatter cache hit rate is at least 99%;
- Updating one message notifies only the ~100 nodes that speak it, and nothing else resolves;
- Switching locale: each message is compiled once, not once per node;
- For a steady single-message update, allocations per update are the same in a small document with a small catalog and in a large document with a large catalog (measured: 2, one of them the new string the node displays).

#259 adds a formatting load: in a 10k and in a 100k node window, a counter changing its number, a single-message update, and a switch to German before the formatter exists all cost the same.

How each layer writes it:
- Rust: `Text::localized`, `t(...)` in views, and `El::locale(...)` declaring a subtree's locale scope
  (see [Declarative views](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/reactive-view.md)).
- `.vue` compiles to Rust views and `view!`: `<T id="files" :count="n" />`, and `locale="ar"` on an element.

`lang` only ever governs the shaping language; messages, formatting, and direction follow the locale.

For a label that grows after a locale switch, when the box only wraps the old text, the first shaping does not wrap to the old box width (see "A box that only wraps the old text" under "Ellipsis" above): it does not first grow taller and then change back within the same frame, and when the new text fits on one line, nothing below the label moves.

### The zero-work fast path

Both text passes (`shape_text` and the layout-scoped `shape_text_for_layout*`) run
`plain_text_is_current` on every candidate first: it reads one `TextNodeState` from the side table and compares the three revisions and the backend generation.
The skip is decided **before building an editor presentation, computing constraints, cloning text, building a key, or querying any cache**.
The cost per node is independent of text length.

- The state lives in a `NodeStore` side table rather than in `NodeRecord`: a large-scope scan pays for the size of the records it visits.
  An entry of a few dozen bytes keeps the cache better than a whole record.
- A visible box with no text of its own (a container) resolves to "none" and is stamped the same way. The next pass reads the table once and skips. An empty `Text` node
  is not such a box: its line box is measured by the scheduling pass, and the layout pass does not stamp it.
- EmptyState / ModalFrame built-in text and TextInput presentations are re-measured every pass. They are not stamped.
- In debug builds the skip path recomputes constraints and compares them with the constraints recorded in the stamp. A missed `CONSTRAINT` invalidation asserts on the spot,
  instead of turning into wrong line breaks on screen.

### Two backends

`TextShaper` gains two more default methods:

- `font_generation()`: the generation of the font set the host measures with. After an `@font-face` registration, resolved text is measured again.
  Runtime's `TextLayoutCache` key carries it too, so a measurement made under old fonts is not hit under new ones.
- `take_text_work()`: the text work the host did in `shape()`. `NanaTextEngineShaper` hands over the engine's shape /
  layout caches and the count of layouts built (the pass counts nodes itself), so engine work for component text lands in the frame counters too.
  Its `font_generation()` folds the whole engine generation (font-system identity, font generation, language generation).
- `text_engine()`: when it returns `Some(SharedTextEngine)`, plain text nodes resolve through `nana-text` and keep
  the `TextLayout` whose metrics it read. **Only a host that can draw the retained layout should return an engine**; otherwise the same node
  would have two measurement authorities. `NanaTextEngineShaper` is such a host: plain text goes through `text_engine()`, and the `shape()` of the rest
  (EmptyState / Modal / editor) goes through the same engine. The product's `NanaTextShaper` has been exactly it since #99,
  so it returns the engine. `SceneWgpuPainter` uses the **same** engine and, under the same node's constraints, lays out the same `TextLayout` in logical px,
  then resolves it into `NanaGlyphRun`. The two sides therefore cannot arrive at two boxes.

Metric contract: width = the `width_px` of the widest line. Height = the sum of the `height_px` of the lines. ascent = the first line's baseline − top.
An undeclared line height reaches the engine as the 1.2em the host has always used. When only a named font family containing `mono` is given,
`monospace` is added as a fallback, matching the host.

### Handles and lifetime

- `TextLayoutStore` (`nana-text`): `{index, generation}` slots. The generation comes from a process-wide monotonic sequence,
  so a handle is accepted only by the store that issued it. Handing one to another UiWorld is rejected like handing over a released handle. The sequence is
  `u32`, so it wraps after about 4.3 billion issuances in a process. After that, a cross-store handle has to hit both the same index and the same generation
  to be accepted by mistake.
  `resolve(id, font_generation)` also rejects a layout produced under an old font generation (`StaleLayout::FontGeneration`).
- Resolve results (the stamp, keeping or releasing the layout) land only when the whole pass succeeds (for nodes that are no longer plain text — editor / EmptyState /
  ModalFrame — the release happens immediately in the loop; they are not stamped anyway). When an invalid metric on some node makes the pass fail, the nodes before it
  neither receive a stamp nor swap handles, and the retry resolves them again instead of skipping them on a stamp whose metrics were never written back.
- When a node resolves again, if the engine hands back the same `Arc<TextLayout>`, the handle is unchanged (`text_layouts_reused`).
  Otherwise the old slot is released and a new handle issued. On node deletion (including a whole subtree / document content teardown) it is released with the node;
  reparent does not release. It also releases when the text becomes empty, when the node becomes editor / EmptyState / ModalFrame, and when the host no longer provides an engine. The text copy
  built for the engine (`TextSource`) is released with it.
- `ShapedRun.instance` records the face coordinates and the synthesis (bold / italic) the glyphs were actually shaped with, so a painter does not have to
  resolve the instance from style a second time.
- `ScenePrimitiveKind::Text.layout` and `ExtractedNode.text_layout` are renderer-neutral IR
  (`RetainedTextLayout { id, layout }`). Equality is "same handle and same layout", so a relayout is a scene change.
  UiScene holds no third-party engine buffer.

### Counters

`UiWorld::last_text_work_counters()` (accumulated across passes within a frame) returns `nana_text::TextWorkCounters`.
#95 added explanatory definitions to the existing fields:

```text
text_nodes_considered        times a text node was looked at (skips included; the scheduling pass and the layout pass each count once per frame)
text_nodes_revision_skipped  judged to have no work by revision alone
text_nodes_shaped            really went into the shaper
text_source_clones           text copies made to build a source / presentation
text_bytes_hashed            bytes fed to the content hash (once per source revision)
shape_cache_lookups          shape cache lookups (hit + miss)
layout_cache_lookups         layout-level cache lookups (hit + miss; the engine layout cache plus the host path's
                             TextLayoutCache; see WorkCounters' text_layout_cache_* to tell them apart)
layouts_created              layouts really built
constraint_only_relayouts    of those, the ones that reuse the shaped runs (a constraint change, not new text)
text_layouts_reused          nodes whose handle did not change after resolving
```

The host shaper path counts the same way: `TextLayoutCache` hits / misses go into `layout_cache_hits/misses` and
`layout_cache_lookups`. Its key copies and hashes the whole text string, so every key counts both `text_source_clones`
and `text_bytes_hashed`. Only a real miss counts as `text_nodes_shaped`. A box with no text of its own that the scheduling pass measures
does not count as a text node. Engine work is recorded only here: `WorkCounters`' `text_shaped_runs` /
`text_layout_cache_*` keep the original meaning of the host `TextShaper::shape` and `TextLayoutCache`. An idle frame that runs no text pass
does not overwrite these counters, consistent with `last_work_counters`. A failed pass leaves no counters behind for the next one.

`nana-dirty-frame-benchmark` writes them, averaged per frame (fractional, so work that happens in only some frames is not flattened to 0), into each cell's
`text_work`. It also supports
`--engine measure|nana-text` (the latter resolves every label with the real engine and the built-in UI font).

### Tests

`crates/nana-ui-scene/tests/retained_text_node.rs` runs everything through `RuntimeDocument::flush`:
the label's retained layout is the same measurement that enters the scene; changing only alignment / only line-clamp also lays the retained layout out again; a non-Text
element that empties its text releases the layout; everything is released when the host no longer provides an engine; a language change makes component text re-measure under the engine host; in a frame that moves every label without changing sizes, every label is a candidate and there is zero text
work; handles stay unchanged, and re-resolving under the same constraints and getting the same layout reuses the handle; color / semantic foreground / opacity /
transform do not touch shaping or layout; a width change only lays out again (`constraint_only_relayouts == layouts_created`); a content change shapes only the node that changed;
a feature change reshapes without copying text; identical labels share one layout and the number of layouts built does not grow with the count; the #33 head-dirty workload (both backends) skips every candidate by revision and the candidate count is linear; a font-set change makes a static document
swap out the old-generation layout too (unstamped text such as EmptyState re-measures as well); an empty Text node re-measures its line height under new fonts; a label that becomes EmptyState releases its layout; another host shaper with the same font generation also measures again; a failed pass leaves no stamp and the retry resolves again; within the same pass
component text is measured through the same engine (no deadlock, and every layout the engine builds that frame lands in the frame counters); handle lifetime on delete / reparent / teardown;
transform + opacity steady-state animation touches no revision; a padding animation lays out again; a height change only to "definite" also counts as a constraint change.
The "zero text work in the frame" assertions all compare against the counters from before the frame: the counters keep the last frame that ran a text pass, so only values this frame rewrote are this frame's work.
On the `nana-text` side: `TextLayoutStore` reuse after release, cross-store, and old font generation. `ShapedRun.instance`
carries variation coordinates. A language hint change advances the engine generation. Runtime unit tests cover the dirty graph itself and style classification.

### Not done in this phase

| Item | Status |
| --- | --- |
| ~~Product drawing of the retained layout~~ **Wired** (#99) | `SceneWgpuPainter` draws directly the one `TextLayout` that `ScenePrimitiveKind::Text.layout` points at — the paragraph that measured it and the paragraph that draws it are the same paragraph, not two that are equal by construction. A node whose box does not match (trailing controls, a ListItem overriding the text box) falls back to laying out its own copy; see "The painter takes the retained layout" |
| Editable text | See Phase 5 (#96): the presentation is still re-measured every pass and is not stamped; probes on the engine host are answered by paragraph geometry instead |
| font-size / font-axis animation | Runtime has no CPU write-back path yet; once it writes back to computed style it classifies as `SHAPE_STYLE` |

## Editable path (Phase 5, #96)

Editable is an extra layer on top of Paragraph layout. It is not state that every piece of text carries: a label never holds a session or geometry.
Text that is not being edited runs none of this code.

```text
EditableText (private storage + TextRevision) ── EditState (selection / composition / goal x)
        │                                        │
        └──────── EditSession: input, delete, move, IME ─┘
                            │ display text (committed + preedit)
                            ▼
        EditorGeometry: one TextLayout per paragraph, invalidated per paragraph
        hit_test / caret_rect / selection_rects / line_bounds / vertical / visual_move
```

### Storage and revision

- The storage type of `EditableText` is private. It changes only through `replace / insert / delete / set_text`. A replacement that leaves the bytes unchanged
  returns `Ok(None)` and does not advance the revision. Callers read `&str` and never name the storage type. Switching later to a rope /
  piece table (decided by a large-document benchmark) does not change the API. `TextEdit { range, inserted_len, revision }`
  is enough to map an old offset into the new text (`map_offset`).
- Offsets are UTF-8 bytes of the committed text. When they leave a command they are always on a grapheme cluster boundary. `utf16_offset` /
  `offset_of_utf16` serve platform IME / a11y APIs that count in UTF-16. `grapheme_index` / `offset_of_grapheme`
  convert to and from grapheme ordinals.
- The three revisions in `EditRevisions { text, selection, composition }` are independent: moving the caret advances only selection.
  A composition change advances only composition. Caret blink advances nothing.

### Navigation

`editable::navigation` is the single definition of grapheme / word / logical line. Runtime's `text_editing` delegates to it directly.
Scanning is bounded to the logical line that contains the offset (UAX #29 always breaks after a newline; WB3a / GB5). So moving by word in a large document costs one line,
not the whole text. `CR LF` is one cluster, so the window includes the line's trailing newline. Unit tests compare every offset against segmenting the whole string.

`Motion`: `GraphemeBackward/Forward` (logical order), `Left/Right` (visual order; needs geometry; without geometry falls back to
logical order), `WordBackward/Forward`, `LineStart/End` (visual line; logical line without geometry), `LineUp/Down` (keeps
goal x; grapheme column without geometry), `ParagraphStart/End`, `DocumentStart/End`. A non-extending horizontal move over a
non-empty selection collapses it to the matching edge (`Left/Right` with geometry collapse to the on-screen left/right edge; in RTL text the left edge is the logical end).
`delete(motion, geometry)` can likewise delete to the start / end of a visual line. After typing, the caret takes `Upstream` and stays next to the character just typed
(typing RTL at the end of an LTR line does not jump to the other side of the RTL run; at a soft wrap it stays on the current line). Geometry is used only if it was synced from this session's current
text + composition revision. `EditRevisions` carries the session identity (unique within the process; a clone is a new
session), so another session whose revision numbers happen to match does not misuse it. Stale geometry falls back to logical movement.

### Caret affinity and hit testing

- `TextLayout::caret_geometry`: `Upstream` draws at the logical end of the previous cluster. `Downstream` draws at the logical start of the next
  cluster. In same-direction text both have the same x. At a BiDi boundary they are two different visual positions for the same byte.
- `TextLayout::hit_test` keeps IR-only semantics (the corpus goldens record it). `hit_test_text(text, x, y)` additionally
  takes the source text: when one glyph draws several graphemes (the `ffi` ligature), it lands on the nearest grapheme boundary. Affinity points at the cluster that was hit.
  Whichever side of a BiDi boundary was clicked is the side the caret draws on.
- `caret_stops(line, text)`: every caret position on a line, sorted by x (one on each side of a boundary; the end of a soft-wrapped line has only upstream).
  Different positions drawn at the same x are adjacent in the line's reading order. `EditorGeometry::visual_move` moves one stop at a time. This guarantees
  that from any caret, in either direction, every grapheme boundary is reachable. The cost is that at a BiDi boundary one key press changes only the logical position
  and does not move on screen (two logical positions are drawn in the same place; both must be reachable: when an LTR line ends in RTL text, the line's logical end is drawn only there).
  Every position in the whitespace hanging at a soft wrap is added as an upstream stop at the end of that line. At a line edge, movement crosses to the adjacent line by the paragraph's writing direction.
- When `hit_test_text` lands outside a line, it takes the visually outermost stop on that side (for equal x, the outermost in reading order; to the right of an LTR line that is the end of the text,
  so typing after the click appends), not the line's logical boundary. When the end of a soft-wrapped line is reversed text, the logical boundary is drawn on the other side.
- Single queries (caret, hit) scan cell by cell. Only `caret_stops`, which needs every position on a line, builds an index sorted by cluster and binary-searches it,
  and does a single grapheme segmentation pass over the line text. Linear in line length. Several cells of one cluster (combining marks) merge into one visual range.

### Selection

`selection_rects` is derived from a logical range + layout. It does not change shaped glyphs. One line of mixed BiDi produces several disjoint rects.
Multiple lines produce at least one per visual line. Only lines intersecting the range are visited. Selection color is paint; it does not reshape.

### IME composition

Composition is a transient overlay, not commit-then-undo: `Composition { replaced, text, selection }`
represents the committed range that the preedit replaces (the selection when composition started). Committed text does not change during composition. Display and layout use
display text (`display_text` / `display_offset` / `committed_offset`).

| Operation | Semantics |
| --- | --- |
| `set_preedit(text, selection)` | Start or update; empty text equals cancel; the IME's own cursor / target segment lies within text |
| `commit(text)` | Replaces `replaced` (the selection when not composing), ends composition, caret goes after the inserted text |
| `cancel_composition` / `blur` | Discards the preedit; committed text and selection are exactly as before it started; a focus change is a cancel |
| `delete_surrounding(before, after)` | Not composing: deletes bytes before and after the selection and the selection itself (keeps the Runtime contract). Composing: the anchor is the range the preedit replaces, **which is kept**; only the bytes before and after are deleted — otherwise canceling composition would lose text; the preedit moves with the text |
| `surrounding_text(before, after)` | A slice of committed text, with the window shrunk inward to character boundaries, carrying the selection within the slice |
| `ime_cursor` / `candidate_rect` | The preedit cursor while composing, otherwise the selection focus; the rect comes from geometry |

These two rules are pure functions in `editable::ime` (`surrounding_deletion` / `surrounding_window`). Hosts that do not hold an
`EditSession` follow the same rules. `set_text` returns `EditChange::Composition` when the bytes are unchanged but composition was canceled.

During composition, ordinary input, deletion and caret movement are all rejected (the IME owns the text near the caret). Geometry marks the preedit as
`CompositionSegment::Preedit` / `PreeditTarget` spans. Each forms its own run, so the painter can decorate them.

### EditorGeometry and incremental invalidation

Text is split into paragraphs at `\n`. Each paragraph is laid out separately by the engine (`TextKind::Editable`) and stacked vertically. Shaping does not cross newlines and the UBA starts a new paragraph
at a newline, so per-paragraph layout gives the same lines as laying out the whole text (`paragraph_geometry_matches_laying_the_whole_text_out` compares
caret x / line top / width and height). `max_lines`, `max_height_px` and ellipsis apply to the whole text. `preserve_lines == false`
folds newlines into spaces. Under these constraints the whole text is one paragraph, with no prefix/suffix reuse: any text change lays out all of it again.

`sync` compares paragraph bytes (prefix paragraphs + suffix paragraphs aligned by the length difference) and each paragraph's composition marker. It lays out again only the paragraphs that changed in the middle:

- Editing one character: 1 paragraph laid out again. The other paragraphs keep their original `Arc<TextLayout>`. Splitting a paragraph: 2. Joining: 1 (shape cache hit).
- Composition update: only the paragraph holding the preedit is laid out again.
- Selection / caret change: `sync_session` returns directly from the revisions. Zero comparisons, zero layout.
- Style / constraint / engine epoch change: everything is laid out again (shaping can still hit the cache). Not counted as editing work.

Queries (`caret_rect`, `hit_test`, `selection_rects`, `line_bounds`, `vertical`, `visual_move`) only read the retained
layouts. They never shape or lay out.

`sync` first compares bytes: if the paragraphs already lay out this text (and style / constraints / epoch are unchanged), it returns as is. The paragraphs and their
layouts are untouched and `revisions` is kept. `GeometrySync::unchanged` is set. **Anything you cache that is derived from the geometry
may rely only on this flag.** `paragraphs_laid_out == 0` does not mean the geometry did not change: deleting the newline that leaves the first paragraph empty merges one paragraph
away and shifts every later offset, yet no paragraph is laid out again. The engine host's editor measurement cache is pinned this way.

Incremental sync is equivalent, item for item, to a geometry that has only ever seen the current text (`incremental_syncs_land_where_syncing_from_scratch_does`
walks through a series of edits and compares paragraph start / top / line count, size, every caret, and a grid of hits).

### Counters

New in `TextWorkCounters`:

```text
editable_mutations              edits that change committed text
editable_bytes_inserted/deleted
caret_only_updates              caret-only moves (result is a collapsed selection), excluding caret moves that come with an edit
selection_only_updates          selection-only changes (result is non-empty)
composition_updates             preedit start / update / end
paragraphs_reshaped_from_edit   paragraphs laid out again by incremental sync whose shaping missed the cache
paragraphs_relayout_from_edit   paragraphs laid out again by incremental sync
hit_test_queries                hit queries against retained geometry
caret_geometry_queries          caret queries against retained geometry
editor_text_bytes_compared      bytes compared one by one when syncing geometry to confirm the text is unchanged (#182; 0 when a stamp is present)
```

Structural gate (`crates/nana-text/tests/editable_text.rs`): while caret blink repeatedly queries the caret, the engine's
`shape_cache_misses` / `shape_cache_hits` / `layout_created` do not change. A series of selection-only moves leaves them unchanged too.

### Runtime integration and staged gates

| Stage | Status |
| --- | --- |
| 1. Internal fixtures | `EditSession` + `EditorGeometry` cover Latin typing and deletion, Pinyin composition and commit, Japanese target segments, Korean jamo composition, emoji / skin-tone modifier deletion, moving across combining marks, caret inside a ligature, Arabic mixed-direction visual movement / affinity / selection, multi-line selection across newlines, click and drag selection, blur during composition, canceling composition, clipboard |
| 2–4. TextInput / TextArea / editor | **Semantics delegate to `nana-text`**: grapheme / word / line navigation, selection validity, IME delete-surrounding (keeping the selection the preedit replaces while composing) and the surrounding-text window reported to the host (`clip_ime_surrounding`: a selection that fits is reported whole, and the two sides share the budget) all follow `nana-text`'s rules; **storage is `EditSession`** (#182, see "Editor storage: EditSession"): Runtime's `SetTextInput` / `SetTextSelection` / `ReplaceTextSelection` / `SetIme` all land on the session, and edit counts come straight from the session (reported with the next text pass). **Geometry is staged per host**: `NanaTextEngineShaper` (the engine host that can paint retained layouts) keeps one `EditorGeometry` per editor node, which answers `text_position` / `text_caret_position` / `text_highlights` / the new `TextShaper::text_hit_at_point` and editor metrics; on hosts that support point hits, up/down movement and paging resolve with "caret position + last-line position + one point hit" (the target y is 0.5px inside the adjacent line, so differing line heights do not skip lines), instead of bisecting position probes. Geometry is retained per node: a probe batch over one text snapshot (`with_text_probes`) syncs once; a single probe outside a batch (up/down movement, left/right visual movement, click, each metrics pass) confirms by the text's **stamp** (`TextStamp`) that the geometry is still for this text, comparing no bytes (`EditorGeometry::sync_stamped`; only anonymous text without a stamp falls back to per-paragraph comparison, and `editor_text_bytes_compared` records how much was compared); probes always ask with the presentation's constraints (`text_input_presentation_constraints`) — asking with other constraints means asking about a geometry the editor did not paint, and a host that retains geometry would also lay out two layouts for one node; editor shaping does not go through Runtime's content-addressed layout cache (`retains_measurement`); `TextShaper::horizontal_offset` lays out a single line independently and does not touch editor geometry; since #99 the product `NanaTextShaper` is a shell over `NanaTextEngineShaper`, so measurement and painting share a source and editor geometry also takes this path |

Engine host guarantees (`crates/nana-ui-scene/tests/editable_text_node.rs`, through `RuntimeDocument::flush`):
caret / selection moves give `layouts_created == 0` for the whole frame and engine shape misses do not change. Typing one character in the middle of a 30-paragraph TextArea creates only
1 layout (`paragraphs_relayout_from_edit == 1`). Each preedit update creates only 1 layout. A preedit does not count toward
`editable_mutations`. A click hits through `text_hit_at_point` without layout. At the end of a soft-wrapped line (`"中" * 120`), one click inside the line and one click at the start of the next line get the same offset, and the caret draws on each line respectively.

Fixed along the way: a multi-line editor's value keeps its newlines and no longer collapses with `white-space` (previously the engine host laid a TextArea out as one paragraph).
A preedit canceled because focus moved away during composition no longer keeps painting in the original editor (`remove_ime` rederives the presentation).
Mounting an editor for the first time no longer counts as an edit.
Selection / preedit x in a single-line field now probes the presentation layout this batch already holds, instead of laying out an extra unwrapped copy of the full text.

### Runtime caret affinity

The affinity in `TextSelection { anchor, focus, affinity }` belongs to `focus`, the caret end. Its type is
`nana_text::Affinity` (re-exported by Runtime as `TextAffinity`). Probes hand it straight to the engine's caret geometry, with no extra
conversion layer. One byte offset can be two positions on screen: at a soft wrap without hanging whitespace (CJK, where the end of a line is the start of the next),
`Upstream` is the end of the previous line and `Downstream` is the start of the next. The same holds on either side of a BiDi boundary. Only pointer hits and visual movement know
which one the user meant, so they write the affinity all the way into the selection. Constructions that derive a selection purely from bytes (`TextSelection::new` /
`caret`, and all edits) take the default `Downstream`. In unwrapped text both draw in the same place.

- Affinity participates in `TextSelection` equality: the same offset on the other side is a different position on screen. `SetTextSelection`
  therefore marks dirty and repaints as usual, and records one caret update.
- Edits reset affinity: an additional caret whose offset was moved by an edit falls back to `Downstream` (the side it was originally resolved on no longer holds).
  `TextSelection::new` and merged selections do the same. Snapping to an atomic boundary that changes the landing point also resets it.
- Probe contract: `TextShaper::text_caret_position(offset, affinity, ..)` answers "where does a caret with this affinity draw".
  `text_position` is still "the origin of this boundary" (byte-derived geometry such as selection bands, run starts and bracket boxes uses it).
  `text_hit_at_point` returns `TextHit { offset, affinity }`. The default implementation ignores affinity. Hosts that do not wrap or cannot tell the two sides apart
  have only one position per offset — for example the lightweight shaper in tests — and always use `Downstream`.
- `NanaTextEngineShaper` uses `EditorGeometry::caret_rect(offset, affinity)` and `hit_test`'s affinity directly.
  The fallback from #96 that "a hit at the end of a soft-wrapped line falls back to the previous grapheme" has been removed: that end-of-line position is now clickable. The caret stays on the line
  that was clicked. Whichever side of a BiDi boundary is clicked is the side drawn.
- a11y has no affinity (AccessKit selections have only character indices). `AccessibilityAction::SetSelection` lands on `Downstream`.

### Visual-order left and right arrows

`TextCaretIntent::Left/Right` use visual order **when geometry can be queried**. When it cannot, they fall back to logical grapheme steps
(hosts without line-break and direction information; there "left" can only mean that anyway). The probe is
`TextShaper::text_caret_visual_step(offset, affinity, rightwards, ..)`. The engine host answers with
`EditorGeometry::visual_move`. There is only one definition, in `nana-text`; Runtime does not reinvent it.

One rule, no platform forks: **Left/Right are always visual order** (the behavior of browsers and Windows edit boxes; in pure LTR
text visual order equals logical order, so only BiDi and line breaks change). Two visible differences:

- In `abc ابج`, pressing Right into the Arabic word moves logically from the end of the word toward its start, because on screen that is left to right.
- At a soft wrap (a CJK-style line end without hanging whitespace), the end of the line and the start of the next line are two affinities of the same byte.
  So it takes **two** key presses: one lands at the end of the line (`Upstream`), one at the start of the next line (`Downstream`). Logical stepping
  would skip the end-of-line position and jump straight over a character.

The probe is asked only when **what is painted is the value itself**: a secure field paints dots and an empty field paints the placeholder.
These two step by grapheme (in a row of identical dots "left" can only mean that anyway). Otherwise it would answer from a layout
that was never painted, and would also lay out the value — passwords included — into the host's retained geometry and the text-keyed shape cache.
No caret move happens during composition (`focused_text_editor` returns None while composing), but the empty preedit left behind when composition
**ends** does not count as composing.

When the selection is non-empty and Shift is not held, Left/Right **collapse the selection to the edge on that side of the screen** instead of stepping one from focus.
There is only one rule, `nana_text::editable::collapse_edge`, shared by `EditSession::move_caret` and Runtime's probe path.
On the same line it is decided by the x of the carets at both ends (in RTL text the left edge is the logical end). Across lines or without geometry it uses the paragraph's
reading order. Secure / empty fields cannot query geometry and collapse to the logical edge. When the landing point is focus, focus keeps its original affinity.
When it lands on the anchor end, the end takes Upstream and the start takes Downstream, the same side the probe asked about when deciding. A selection that stops at a soft wrap
keeps its caret at the end of the line it covers after collapsing. Word/Line intents are logical by definition. Vertical movement takes
its own geometry path.

### Editor storage: EditSession (#182)

An editor's committed text, selection and IME composition are **stored once**, in the world's `EditSession`
(`NodeStore.editors`, one `EditorRecord` per editor). The three premises left by #96 are settled as follows:

1. **Multiple carets go into `nana-text`.** `EditSession` holds the primary selection + additional selections. The normalization rules (sort, merge overlapping / touching
   selections into one forward selection, a selection merged into the primary one stays primary) exist only once, in `editable::normalize_selections`. The component-side
   `TextInputState` calls it too. Composition attaches only to the primary selection. On commit the other carets shift via `remap_selection`. Hosts with their own transformation logic
   (Runtime's per-caret line transforms) hand the computed multi-point edit and landing points to `EditSession::splice`: one edit,
   one revision.
2. **Display text is derived by the world; identity comes from stamps.** A session's display text = committed + preedit (`display_text`).
   Fold summaries, inlays and secure masks are Runtime overlays and do not go into `nana-text`. Every text carries a `TextStamp`
   (unique within the process; replaced when the bytes change; kept by clones): the session's snapshot, the fold / inlay view and the composing display text are each memoized by
   "text stamp + composition revision + folds + inlays". Built once per change, and every consumer gets the same copy. The host's
   `EditorGeometry::sync_stamped` decides "unchanged" from the stamp. Probes outside a batch, caret moves and clicks compare not a single byte
   (`probing_an_unchanged_editor_compares_none_of_its_text` in `editable_text_node.rs` pins
   `editor_text_bytes_compared == 0`).
3. **Product contracts are kept and switched to shared values.** `TextValue` (`nana_text::SharedText`) is an Arc buffer + stamp:
   `TextContent.value`, `TextInputState.value`, `display_value`, the a11y `value`, the scene Text primitive and
   `TextChanged.value` all use it. Cloning is just a reference count. `UiWorld::text_input` / `ime` return
   `TextInputView` / `ImeView` that borrow the session. `TextInputState` / `ImeComposition` are the write contract (application,
   `SetTextInput` / `SetIme`) and owned snapshots.

The `state` of a component (`TextInput` / `TextArea` / …) is a **shared handle** to the session text: a component edit writes a new value, and at projection
the session adopts that buffer directly (`EditSession::assign` scans from both ends to the change and reports only the changed range; another buffer with identical bytes is
adopted too, and later compared by pointer). Both sides point at the same bytes, and projection compares by identity. Composition lives only in the session.
"IME attached but preedit empty" is a Runtime marker (not composing; it does not block the caret or display; it only makes submission wait for the IME's
commit / cancel). The undo log still stores whole-state snapshots, but the snapshots share buffers: each step's before and after states no longer copy the document separately.
The byte budget counts buffers, not snapshots.

One key press (310 KB document) used to make about 8 full-value copies (history before/after, `editor_state`, component staging, projection, validation,
world mirror, `TextChanged`, three in presentation). Now only the passes the edit itself needs remain: the component writes the new value (one
build), one diff when the session adopts it, and the prefix/suffix comparison of paragraph geometry. Editors with highlighting, bracket coloring or a minimap each
compute one more pass (highlighting is keyed by stamp and no longer hashes the full text for a key). Caret moves, clicks and up/down movement neither copy nor compare text.
When assistive technology is present, the AccessKit node still needs its own `String`.

### Not done in this phase

| Item | Status |
| --- | --- |
| ~~caret blink~~ **Wired** | The caret of a focused editor (`TextInput` / `TextArea` / `RichTextEditor`) shows and hides for 500 ms each on the animation clock (`CARET_BLINK_INTERVAL`). Typing, caret movement and gaining focus all show it immediately and restart the timer; while the window is unfocused the caret stays on and no longer wakes. Hiding removes the caret only at extraction (`UiWorld::set_hidden_carets`, marks only RENDER); caret geometry, the IME anchor and scrolling are unaffected, and no text revision advances. With no focused editor, `next_animation_deadline` does not include it |
| ~~Runtime visual-order left/right movement~~ **Wired** | See "Visual-order left and right arrows": the `text_caret_visual_step` probe + `EditorGeometry::visual_move`, falling back to logical steps without geometry; a non-empty selection collapses to the visual edge, using the same rule as `EditSession` (#182) |
| a11y composition | The current a11y contract has only value / selection / editable (caret is the selection focus); AccessKit has no composition range, so none is faked; the same goes for per-character geometry |
| Partial cluster splice | Not done; the smallest invalidation unit is a paragraph |
| ~~Switch the IME semantic backend to `EditSession`~~ **Wired** (#182) | Storage and semantics are both in the session; see "Editor storage: EditSession" |
| Large-document storage | `SharedText` (`Arc<String>` + stamp, #182); **no switch to a rope**: on a 310 KB document the memmove of one edit is 0.5% of the frame cost; see "Large-document editing benchmark" |

## Rich text spans

An application hands a styled run of text to an ordinary text node: `MutationQueue::set_rich_text(id, RichText)`, or the [`RichTextView`](https://github.com/sena-nana/NanaUI/blob/main/docs/components/rich-text-view.md) component. The value is `nana_ui_core::RichText`, owned by the application: one `Arc<str>` plus one `Arc<AttributedRanges<RichSpanStyle>>`, and a clone is two reference counts. The framework never changes it behind the application's back.

```text
RichText (application-owned)
    │  SetRichText: classify_rich_change → minimal TextDirty
    ▼
TextNodeState::source_for  shape layer over the node's computed style → TextSource::set_spans
    │
    ▼
nana-text shaping / layout      spans have their own size and line height; the line box grows with them
    │  TextLayout (one copy for measuring and painting)
    ▼
ExtractedNode.rich_text → Text { spans (fill color), rich: SceneRichPaint (the other paint layers) }
    │
    ▼
SceneWgpuPainter::text     shadows → stroke under fill → underline → fill → stroke over fill → strikethrough
```

### Three style layers

`RichSpanStyle` is sparse: every field is an `Option`, `None` inherits the node's computed style, and when the theme changes the parts that were not overridden change with it. Fields are split into three layers by the cost of the work:

| Layer | Fields | `classify_rich_change` | Work |
| --- | --- | --- | --- |
| `shape: RichShapeStyle` | `family`, `size_px`, `weight`, `italic`, `letter_spacing_px`, `features` | `SHAPE_STYLE` | Reshape, lay out, scene geometry |
| `paint: RichPaintStyle` | `color`, `decoration`, `decoration_color`, `stroke`, `shadows` (≤ 4 layers) | `PAINT` | Re-extract painting only; the painter rebuilds this span's instances |
| Presentation | `effect: Option<u16>` | `GLYPH_PRESENTATION` | Text is untouched; Runtime schedules no pass at all |

Classification compares each layer's **normalized projection**: `AttributedRanges::map` pulls one layer out, drops the empty ranges and merges equal neighbors. So the same style cut into different ranges is not a change, and a color-only change never reaches the shape revision. A change to the text itself is `CONTENT`. A later `SetText` that swaps the text drops the spans (they point at the old bytes).

`AttributedRanges<A>` is pure range algebra: ordered, non-overlapping, non-empty, equal neighbors merged. `set` / `update` / `clear_range` change one range's attributes, `splice` shifts ranges along with a text edit (inserted characters inherit the attributes of the character before the caret), and `slice` / `map` / `at` are read-only. It knows nothing about text and nothing about styles; `RichText` is responsible for aligning ranges to character boundaries. It lives in `nana-ui-core`, not `nana-text`: `nana-ui-core` cannot depend on `nana-text`, and this value has to be defined in core.

### The shape layer goes into `TextSource`

`TextNodeState::source_for(text, rich, computed)` reads a span only while the node still displays that value's text. Every span with shape fields becomes a `TextSpan`, with the node's `nana_text_style` as the base and the span's fields laid on top: the font family goes through the same `nana_font_family` as the node. A span that changes the font size keeps the node's line-height **ratio**: when the node is 16px with an absolute line height of 20px, the 32px span gets a line height of 32 × 1.25. A line that holds large text grows taller.

A source with shape spans is cached under two revisions, `(content, shape)`, and is rebuilt once when the node's computed style changes (`SHAPE_STYLE`); a node without shape spans is still cached under the content revision alone, and does not copy its text one more time. Paint-layer and presentation-layer fields never enter the source, so they never enter the shape cache key.

### The paint layer goes into the scene

`ScenePrimitiveKind::Text` gains `rich: Option<Arc<SceneRichPaint>>`:

- `base: SceneTextEffects`: the node CSS `text-decoration`, every `text-shadow` layer (`PaintStyle::text_shadows`), `-webkit-text-stroke` (`PaintStyle::text_stroke`, under the fill when `paint-order: stroke`).
- `runs`: the ranges where a span's paint layer covers `base`.
- `revision`: a hash of the above, never 0. The painter uses it to decide whether this span's instances are still painted this way, without comparing field by field.
- `reach`: logical pixels by which shadows and strokes extend past the glyph ink; the painter uses it to enlarge the ink box, so clipping and batching do not cut a shadow off.

A span's fill color is not in `rich`; it is merged into the existing `spans` (`rich_fill_spans`): the same color path as syntax highlighting and selection color, and existing spans win.

An ordinary text node no longer emits a full-box-width `Stroke` primitive for decoration lines. Markdown inline runs still use that old path.

### The painter: layered instances in one entry

The painter first resolves a paragraph into fill glyphs as usual, then arranges them according to `rich` (or, without `rich`, the component text's single-layer `text_shadow`):

1. Shadow layers, the last CSS layer at the bottom. Blurred or spread ones are `GlyphRenderMode::Blur { radius_q, spread_q }`; sharp shadows reuse the fill bitmap directly;
2. `TextStrokePlacement::Under` strokes, `GlyphRenderMode::Stroke { width_q, join }`;
3. Underline;
4. Fill;
5. `Over` strokes;
6. Strikethrough.

The low two bits of an instance's `pad[1]` record its role (fill, shadow, stroke, solid). Underlines and strikethroughs are `ROLE_SOLID` solid quads: they do not sample the atlas, so they need no fixing when the atlas moves. They walk glyph cells line by line and run by run, merging adjacent characters with the same color and decoration into one segment; position and thickness are the values `RunMetrics` reads from `post` / `OS/2`, rounded to whole pixels. A line without `decoration_color` follows the fill color; when it matches the paragraph's base color it inherits the run row, so a color change needs no rebuild. Vertical paragraphs do not draw decoration lines yet.

`Stroke` and `Blur` rasterize from the outline (swash) only, color glyphs included. A stroked glyph's fill also goes through swash with `GlyphSynthesis::OUTLINE_RASTER`: on Windows, a fill drawn by DirectWrite and a stroke drawn by swash are half a pixel apart and the edge shows. Shadows do not need this; their fill still goes through the platform rasterizer.

Blur is a separable Gaussian on 8-bit coverage on the CPU: 3σ padding on each side, a normalized kernel, total coverage preserved (the test requires deviation < 2%). Above 8 raster pixels of radius it downsamples 2×2, blurs with half the σ, then scales back up bilinearly. The logical radius is capped at 24px. The blurred result enters the raster cache and atlas like an ordinary bitmap, with the radius and spread in the key and no color.

The entry's color fingerprint combines span colors with `rich.revision`: changing a stroke, shadow or decoration line rebuilds that one entry's instances without re-laying out; changing only their colors rasterizes no new bitmap; a steady-state frame does nothing at all — no rasterization, no atlas upload, no rebuild, no instance re-upload.

### Inline objects

Stickers, emoji and editor marks are objects inside text. The text holds a U+FFFC (`OBJECT_REPLACEMENT`), and `TextSource::set_objects` pairs it with an `InlineObject { offset, id, metrics: InlineObjectMetrics { width_px, ascent_px, descent_px } }`. An object is one grapheme cluster: the caret steps over it in one move, a selection takes all of it, and hit testing needs no change.

- **Shaping does not read objects.** The shaper cuts each U+FFFC into its own placeholder run: one `GlyphFlags::OBJECT` glyph, no font, advance 0. Objects are not in the shape cache key, so changing only an object's size does not reshape.
- **Layout gives it a box.** Before line breaking, `Layouter` replaces the placeholder run's advance with the object width and its ascent / descent with the object's. An object is an atomic inline box: it does not ask for a line height of its own, and it stands on the baseline; the text's half-leading is unchanged, the line box grows above and below by the part of the object that sticks out, and the baseline shifts down accordingly. The layout cache key carries every object's position and box: two sources with the same text and the same revision but different objects are two layouts.
- **`TextLayout.objects`** records every object placed on a line: `PlacedObject { id, offset, line, rect }`, where `rect` is in line space and its bottom edge is the baseline plus descent. Objects cut off by an ellipsis are not in it.
- A placeholder run has no face, so the painter does not draw it and decoration lines skip it.

#### Block objects and width fitting

By default an object is inline and lays out at its own size. `TextSource::set_placements` gives individual objects a different placement: `ObjectPlacement { offset, flow: ObjectFlow, fit_width }`, a table beside the objects like `ObjectLabel` (set the object first, then this; the default placement and entries not on an object are dropped; `replace_range` shifts it along with the object, and deleting the object deletes it). `InlineObject` itself is unchanged, and a source with no placements pays nothing extra.

- **`ObjectFlow::Block` takes a line of its own.** `segments` cuts segments before and after a block object's U+FFFC: the text before it ends its line at its offset, the text after it starts a new line after it, and neither side gets an empty line, so a block object between two newlines, at the start or at the end, occupies only its own line. The text, the offsets and the paragraphs are unchanged, and no character represents these two line breaks, so the `break_cause` at the cut is `Wrap`: the byte at the cut has a caret position at the end of the previous line (upstream) and at the start of the next line (downstream), the same as a soft wrap. Only the line at the end of a segment is `Explicit` / `EndOfText`, and justification does not stretch the line before a block object. A Label that contains a block object and also other text does not take the single-line fast path (the fast path asks `segments` for its segment count).
- **`fit_width` scales down to the line width.** When an object is wider than the space it can get on a line, `sized_runs` scales width, ascent and descent by the same ratio, and from then on the line box, hit testing, carets, selection, `PlacedObject.rect` and the image in the scene all read this one scaled box. The space available is the line budget (`inline_budget_px`) minus the spacing the envelope leaves after it (including the part that can be collapsed at the end of a line, so it fits on a line of its own however line decisions treat spacing); this engine has no `text-indent`, so no line is narrower than the budget. Without a width budget it does not scale down, and it never scales up; the size in the source stays what the author wrote. Label objects (`Tag` / `Label`) take their size from the label and do not scale.
- **min-content is compressible.** Like an image with `max-width: 100%` in CSS: `intrinsic_widths` uses the object's own width for `max-content` and counts a scalable object as 0 wide for `min-content`, so a container can shrink below it and layout then scales it to the remaining width. Runtime flex shrinking reads the same thing: a scaled object does not leave the widest line unable to break (`widest_line_cannot_break`), so the node can narrow with its container.
- Placements enter the layout key: two sources with the same text, the same revision and different placements are two layouts. Changing a placement re-lays out without reshaping.

The application-side value is `RichObject { id, width_px, height_px, descent_px, flow, fit_width, content }`. The constructors default `flow` (`RichObjectFlow::Inline` / `Block`) and `fit_width` to inline at the original size; `.flow(..)` and `.fit_width(..)` change them. `source_for` converts them into `nana-text`'s own `ObjectPlacement` with `rich_text_placements` (`nana-text` does not know `RichObjectFlow`). There are five kinds of content:

| `RichObjectContent` | In the scene | Placeholder |
| --- | --- | --- |
| `Image { source }` | `Quad`'s `content_image`, with the same source rules as CSS `url()`, fitted into the box with contain | Its own width and height |
| `HostTexture { slot }` | `Custom`, `HOST_TEXTURE_RENDERER` draws the host texture slot (application-decoded animated images go here) | Its own width and height |
| `Chip { label, kind }` | Drawn only in the editor; the display box emits no primitive | Always 0 |
| `Tag { label, kind }` | A pill background (a fixed palette by `kind`) under the glyphs, with the label glyphs in a light ink | The label text's width plus padding |
| `Label { label, style }` | Like `Tag`, with background, ink and corner radius from `RichLabelStyle`; when `style.icon` has an image, a square is reserved before the text and the image is fitted into it with cover (`icon_round` clips it to a circle) | Label text plus padding, plus the icon square |

`Tag` and `Label` take their size from the object label (`ObjectLabel { offset, text, scale, weight, color, icon }`): the label is shaped on its own at `scale` times the surrounding text's font size (`LABEL_SCALE` 0.62 for `Tag`), and `label_box` turns the width, padding and icon square into the object's box; `PlacedLabel` records the pill's `rect`, the laid-out `runs`, the icon square `icon` and the ink `color`. Icon and ink enter the layout key (`ObjectLabel::layout_bits`), so a color-only change is a different layout too. When an object's label text or style changes, `classify_rich_change` reports `CONSTRAINT`, and the source is rebuilt. The pill and the icon follow the reveal and the effect of the object's grapheme, like the label glyphs.

`RichTextBuilder::object` / `styled_object` and `RichText::insert_object` / `set_object` maintain them; `replace_range` deletes an object when it deletes the object's characters. `TextNodeState::source_for` hands `RichObject::line_box()` to `TextSource::set_objects`. In `classify_rich_change`, an object at the same position with a different box, flow or `fit_width` is `CONSTRAINT` (re-layout only), a different content is `PAINT` (repaint only), and inserting or deleting an object changes the text, so it is `CONTENT`. The scene emits one primitive per object in text order, with slots in the `TEXT_INLINE_OBJECTS` namespace, following the node's transform, clipping, opacity and document order.

A mark chip is 0 wide, so the editor and the display box break the same `RichText` into the same lines.

### Ruby annotation

Ruby (kana, pinyin) is a line of small text above the base characters. `TextSource::set_rubies` gives the source a table: `RubySpan { range, text }`, where `range` is the base characters' byte range; the table is sorted by base and does not overlap. An edit that reaches inside the base drops that ruby; an edit after the base shifts it along. Horizontal only.

- **Shaping.** `NativeTextEngine::layout` shapes each ruby on its own with the style at the base's start and `RUBY_SCALE` (0.5) times the font size, through the same shape cache. The ruby's `ShapedText` enters the layout key by pointer: changing the ruby is a different layout, and the base itself is not reshaped.
- **The base is one unit.** Break points never fall inside the base (`break_stops` filters them), so the base does not split across two lines.
- **A wide ruby spreads the base.** When the ruby is wider than the base, each base glyph shifts right by half the difference and the last glyph's advance grows by the whole difference: the base is centered under the ruby, and the text after it shifts by the difference. A narrow ruby leaves the base alone and is centered above it.
- **The line box grows.** A line that holds the base grows upward by the ruby's height (standing on the text ascent), through the same `lift` as a tall sticker, and the baseline shifts down accordingly.
- **`TextLayout.rubies`** records every ruby placed on a line: `PlacedRuby { range, line, baseline_y_px, rect, runs }`, where `runs` are ruby runs already positioned with `origin_x_px` (clusters are byte ranges of the ruby text). Vertical layout does not place ruby, and `rubies_dropped` is true.
- **The painter** draws ruby glyphs after the line's glyphs, in the same entry. Each ruby glyph takes its color, shadow, stroke and reveal index from the base's first byte, so ruby appears and moves with its base.

On the application side is `RichText`'s ruby table: `RichTextBuilder::ruby(base, annotation)` / `styled_ruby`, `RichText::set_ruby(range, text)` replaces every ruby it touches, `clear_ruby(range)` deletes, `ruby_at(offset)` looks up. `slice` takes only rubies that fall entirely inside the range, and `replace_with` pastes them back. `source_for` hands the table to `TextSource::set_rubies`; in `classify_rich_change`, a changed ruby is `SHAPE_STYLE`. The editor's `RichEditCommand::SetRuby(Some(text))` adds ruby to the selection and `SetRuby(None)` deletes the ruby the selection touches; both can be undone.

Tests: `nana-text` `tests/ruby.rs` (centered above the base and the line grows, a wide ruby spreads the base and shifts the following text, no break inside the base, editing shifts and drops, dropped in vertical); `nana-ui-core` `rich_text::tests`; `nana-ui-scene` `tests/rich_text_editor.rs` (ruby added by the editor, the editor and the display box break at identical positions, undoable); `nana-ui` `scene_paint::text` (ruby glyphs drawn above the base and smaller than it).

### Emphasis marks

An emphasis mark (CSS `text-emphasis`) is a small mark above or below each character. `TextSource::set_emphasis` gives the source a table: `EmphasisSpan { range, mark, position }`, where `range` is the byte range that carries the mark, `mark` is the mark text and `position` is a `TextEmphasisPosition` (`over` / `under`; `right` / `left` only matter for vertical). The table is sorted by range and does not overlap. An edit inside a range grows or shrinks it (typed characters get the mark too), partial overlaps are clipped, and an empty range is dropped. Horizontal only.

- **Which character a mark is.** `TextEmphasisStyle::mark_text` maps CSS keywords to characters: `dot` • / ◦, `circle` ● / ○, `double-circle` ◉ / ◎, `triangle` ▲ / △, `sesame` ﹅ / ﹆ (filled / open); with only `filled` / `open` given, horizontal takes `circle` and vertical takes `sesame`. `<string>` is passed through as is, and the engine takes only its first grapheme cluster.
- **Which characters take a mark.** `font::unicode::takes_emphasis` skips separators (spaces), punctuation, control and format characters, unassigned code points and the object placeholder U+FFFC. So `重，点` gets only two marks, and a line of punctuation does not grow taller.
- **Shaping.** A mark is shaped on its own with its character's style, `EMPHASIS_SCALE` (0.5) times the font size and zero letter spacing, through the same shape cache. Ranges are cut at span boundaries, and adjacent runs with the same mark are merged; each run's `ShapedText` enters the layout key by pointer. Changing the mark or the position is a different layout, the base is not reshaped, and character advances do not change.
- **Placement.** One mark per character cluster that takes a mark, centered horizontally on the cluster's advance. An `over` mark stands on the run's ascent; when that character is a ruby base and the ruby is on this line, the mark stands above the ruby. An `under` mark hangs below the run's descent.
- **The line box grows.** As with ruby, only by the part of the mark that sticks out past the half-leading: above goes through `lift` and the baseline shifts down; below goes through a new `sink` and the baseline stays. `half_leading` subtracts both `lift` and `sink`, so lines that grow downward (including inline objects with a large descent) no longer push the baseline down by half.
- **`TextLayout.emphasis`** is `PlacedEmphasis { offset, line, x_px, baseline_y_px, rect, mark }`, where `mark` indexes `TextLayout.emphasis_marks`: one shaped run per mark, shared by every character with that mark. Vertical layout does not place marks, and `emphasis_dropped` is true.
- **The painter** draws mark glyphs after ruby, in the same entry, through the ordinary glyph atlas; there is no separate quad path. The color is `text-emphasis-color` (the rich span's `emphasis_color` wins, then the node's); with `currentColor` it is that character's text color. The fallback where the painter lays out on its own without a retained layout does not draw marks.

On the application side: the node style is `LayoutStyle.text_emphasis_style` / `text_emphasis_position` / `text_emphasis_color`, all three inherited; a rich span uses `RichSpanStyle::emphasis(style)`, `emphasis_position(pos)` (`RichEmphasisStyle`; `TextEmphasisStyle::None` on the span turns off the node's mark) and `emphasis_color(color)` (in the paint layer). `source_for` composes the spans over the node value into the `EmphasisSpan` table. Invalidation: a style or position change is `CONSTRAINT` (re-layout, no reshape), a color-only change is `PAINT` (no shape, no layout, only the painter fingerprint).

Tests: `nana-text` `tests/emphasis.rs` (grows above and the baseline shifts down, grows below and the baseline stays, skips punctuation and whitespace, a string mark takes only its first grapheme and is half size, a large span gets a larger mark, changing position does not reshape, a mark over ruby stands on the ruby, follows edits, dropped in vertical); `nana-ui-css` `css_paint`'s `text_emphasis_*`; `nana-ui-runtime` `text_node` (`emphasis_relays_out_and_its_colour_only_repaints`); `nana-ui-scene` `tests/rich_text_spans.rs` (a color change neither shapes nor lays out, CSS emphasis grows the line, changing position only re-lays out); `nana-ui` `scene_paint::text` (the mark is drawn above or below the character and is smaller than it); the component-gallery snapshot `component-migration/text`'s `emphasis` state (Chinese below, Japanese above).

### Rich text editor

`RichTextEditor` is not another kind of editor storage; it is an ordinary text node plus an edit state:

- **Layout.** The text it displays (the document, or the document with a preedit spliced in) goes through `SetRichText` down the rich text path above, so its `TextLayout` is the same computation as `RichTextView`'s for the same `RichText` at the same width. `tests/rich_text_editor.rs` asserts that both have the same byte range, width and line height for every line, position by position.
- **Edit state** lives in the component: the selection (anchor, focus, bytes), the preedit and its caret, the typing style at the caret, and the undo / redo snapshots (a clone of the whole `RichText`, two reference counts), capped at 200 steps, with consecutive typing coalesced into one step. All of it is kept when the application hands back an equal `value`; when it hands back a different document the selection collapses into the new text, and the preedit and typing style are cleared.
- **Range remapping.** An edit lands in `RichText::replace_range` / `replace_with`: spans go through `AttributedRanges::splice` (inserted characters inherit the character before the caret), objects shift with their own characters, and deleting characters deletes objects. Copying takes `RichText::slice`.
- **Input.** The keyboard routes to the focused rich text editor after the terminal and before ordinary editors (the application's key policy still sees keys first); composition goes through the same position as `dispatch_composition`; pointer down, drag and up are handled after the terminal. The IME context is `TextInputPurpose::Normal`, and the anchor is the editor caret.
- **Caret and selection.** The component writes byte positions into `RichEditorMarks` (`SetRichEditorMarks`, repaint only). At extraction the scene computes rectangles from the node's own retained layout: the selection is under the glyphs (the same layer as the document selection), and the caret is above. Only a focused editor draws a caret; moving focus marks a repaint. An empty text node normally retains no layout; the editor is the exception: an empty document keeps its one empty line, the caret is drawn at the start of the line, and the IME anchors there too. When the old layout cannot hold the new caret after an edit and before a new layout, the IME temporarily uses the editor's content box and does not lose the anchor.
- **Clipboard.** The system clipboard holds plain text only; it writes no HTML wrapper or process marker. In-process it remembers only the most recent copied rich text, plus the sequence number `HostServices::clipboard_change_count` gave after the write. Paste reads the text first, then the number: the same number means nothing was written after the copy, and only when the text is also the same is it pasted back as rich text; the same text written from elsewhere has a changed number and is pasted as plain text. When the host answers `Unsupported`, it falls back to comparing the text alone. `RichClipboardCommand::PastePlainText` always pastes as plain text. Cross-process rich formats are left to the host to extend.
- **Typing style.** `RichTypingStylePolicy::Inherit` (the default) makes typing inherit the character before the caret, falling back to the character after it, then to the node style; `Plain` uses the node style only. Both give precedence to the caret's own typing style (set on the caret with `SetAttrs` / `ClearAttrs`); the preedit, plain-text paste and `selection_attrs` on an empty selection read the same result.
- **Caret blink.** The edit state carries a `caret_revision`: it increments when the document, the preedit or a selection endpoint changes, and also when the application hands back a different document. It enters `TextInputKey::RichEditor`, so it reuses the same `reset_caret_blink` through `sync_text_input` as `TextInput`'s edit revision; an edit that does not change the caret box (backspace, before a new layout) still makes the caret show again. There is no second blink state.

### Per-glyph presentation

Per-glyph effects and typewriter reveals are **presentation**, not text: they enter neither shaping, layout nor glyph rasterization, and they do not enter the paragraph's already-built glyph instances.

- **Vocabulary** lives in `nana_ui_core::motion::glyph`: `GlyphEffect { kind: Shake | Wave | Jump | Rainbow | Pulse | Flicker, amplitude, frequency_hz, stagger }`, `GlyphIntro { fade, pop, rise_px, duration_s, easing }`, `RevealSchedule { start, at_s, limit, intro }`. `at_s[i]` is the time the `i`th grapheme (extended grapheme cluster) starts entering, relative to `start`; `start` is on the document's animation clock (`AppContext::animation_now()`). Graphemes past `at_s` or past `limit` are not drawn.
- **Entry** is `AppContext::set_rich_presentation(node, effects, reveal)` (`MutationQueue::set_glyph_presentation`). `effects` is a table, and `RichSpanStyle::effect` is an index into it. It marks RENDER only: the scene packs it and each grapheme's effect index into `Text { presentation: SceneGlyphPresentation }`. A changed span effect index gets only this far too (`GLYPH_PRESENTATION`).
- **Instances** record their own grapheme ordinal when the entry is built (the high 30 bits of `GlyphInstance.pad[1]`; the low 2 bits are the role). Presentation is not in the entry's fingerprint: changing the effect or the reveal does not rebuild instances.
- **GPU.** Each frame's prepare packs the presenting paragraphs into a glyph table (binding 4, `text_glyph_fx`): one header per paragraph (reveal start, limit, intro parameters, the effect table's position), two words per grapheme (effect index, reveal time). The run row's `fx` points at the header. The vertex shader reads the motion clock from `Globals.motion` (whole seconds, fraction, and an effect clock modulo one hour), computes displacement, scaling about the glyph center, opacity and rainbow color from `glyph_look`, and applies them to the instance's four corners. Shadows and strokes follow the same grapheme; rainbow tints the fill only. A decoration line appears with the first character under it. Graphemes whose turn has not come are culled. The clock writes 16 bytes only when it moved.
- **The CPU evaluator** `evaluate_glyph` is the same arithmetic: shake and flicker use the same integer hash (lowbias32), and reveal time differences subtract whole seconds first and then add the fractions. Inline objects (stickers) do not go through the text shader; on every compositor tick the scene uses it to compute an object's displacement, scale and opacity and writes them back to the object primitive.
- **Direction.** Waves, jumps and the rise of an intro go along the line's cross axis, toward the line-over side: the top of the page for `horizontal-tb`, and the right for both `vertical-rl` and `vertical-lr` (CSS Writing Modes §6.4; sideways glyphs in vertical text are rotated 90° clockwise, so their tops face right too). A page displacement `(0, dy)` becomes `(-dy, 0)` in vertical text, so a column of characters is not unevenly spaced and an intro does not run into the previous character. The writing direction comes from the node's own computed style: extraction writes it into `SceneGlyphPresentation::writing_mode`, it is packed as word 11 of the glyph table header (`glyph_over`), and the CPU side goes through `evaluate_glyph_chain_in` / `SceneGlyphPresentation::sample`. Callers pass no extra argument; `evaluate_glyph` / `evaluate_glyph_chain` still give the horizontal answer. Shake, rainbow, pulse, flicker and the pop scale have no direction and do not change.
- **Frames.** The scene records until when each presenting node still moves: looping effects forever, a reveal until the last grapheme finishes entering. `compositor_needs_tick` requests frames from that; once a reveal finishes it returns to on-demand frames. The painter does not reuse the previous frame's dest while a presentation is active.

Tests: `nana-ui-core` `motion::glyph::tests` (reveal, limit, determinism, wave crests, cross-axis displacement in vertical); `nana-ui-scene` `tests/glyph_presentation.rs` (a reveal does no text work, frames are requested only while it plays, looping effects keep going, they stop after clearing, a sticker reveals with its own grapheme, a vertical sticker waves to the right); `nana-ui` `scene_paint::text` (the shader puts glyphs where the CPU evaluator says: wave crest displacement, unrevealed glyphs not drawn, the pop scale, waves / jumps / rise going crosswise in vertical; the header carries the writing direction; 60 frames of an active presentation rebuild nothing and upload no glyph or instance).

### Font decoration-line metrics

`RunMetrics` gains `underline_offset_px` / `underline_thickness_px` (baseline to the **top edge** of the underline, positive downward) and `strikeout_offset_px` / `strikeout_thickness_px` (baseline to the top edge of the strikethrough, positive upward), read from `post` / `OS/2` in the run's axis coordinates and font size. When the font gives none or a thickness of 0 they are filled in from the font size: 1/14 em thick, the underline 0.1 em below the baseline, and the strikethrough centered at half the x-height. All four fields are `serde(default)`: they are 0 in the Phase 0 goldens, and the parity diff compares them only when the expected value carries them.

### Runtime fonts

Fonts that a skin and the like swap in are registered with `nana_ui::HostFontScope`: `add_bytes` / `add_file` / `add_face` return a `HostFontRegistration`, and `clear()` or dropping the scope withdraws that one source with `FontSystem::unregister`. Withdrawal is a new font generation, see the font-set item under "Dirty graph" above: measured nodes re-resolve on the next frame, and bitmaps from the old generation are no longer sampled.

### Tests

- `nana-ui-core` `rich_text::tests`: range algebra (splitting, merging, `update`'s gaps, `splice`, projection), character boundary alignment, the builder.
- `nana-ui-runtime` `text_node::tests`: shape spans laid over the node style, the line-height ratio, classification by layer.
- `nana-text` `tests/inline_objects.rs`: an object takes its own width and stands on the baseline; changing its size only re-lays out, no reshape; a tall object grows the line box without overflowing the line top; objects wrap with the text; objects move with edits.
- `nana-ui-runtime` `rich_text_editor::tests`: typing inherits the style and coalesces into one undo step; the style at the caret applies to the next character; the three-state summary; the preedit is underlined and does not enter the document; an object inserted at the caret is one character; read-only.
- `nana-ui-scene` `tests/rich_text_editor.rs`: the editor and the display box agree line by line; typing, IME, backspace and undo reach the document through the input route; rich text copy and paste keep the styles; caret and selection are drawn from the editor's own layout; changing color in the toolbar does not re-lay out; arrow keys, Home / End.
- `nana-text` `tests/block_objects.rs`: a block object takes a line of its own and the source bytes are unchanged, with no extra empty lines in any combination of line breaks; a block boundary is `Wrap` and the boundary byte has a caret on each of the two lines; `fit_width` scales proportionally and hit testing, carets and selection all read the scaled box, never scaling up and not scaling without a budget; an inline scalable object wraps with the text; the space available subtracts the envelope spacing; `min-content` does not count a scalable object and `max-content` counts its original width; changing a placement only re-lays out, no reshape, and the same revision with different placements does not share a layout; placements shift with edits and go with a deleted object; a source with collapsed line breaks keeps its placements; old serialized data without placements still loads.
- `nana-text` `tests/object_labels.rs`: a label object takes the label's width and puts the run inside it; an icon label reserves a square before the text, the object widens with it, and the ink enters `PlacedLabel`; labels shape at their own `scale` and typeface.
- `nana-ui-scene` `tests/rich_text_spans.rs`: a `Label` with an avatar takes the line width, draws a pill and a square image inside the pill, and re-lays out and widens as the name grows longer; a sticker takes the line width and draws as an image; a scalable block image takes a line of its own in a 400px column, shrinks to the column width and keeps its ratio; in a flex row a scalable image lets the node narrow to the row width, and a non-scalable one holds its original width; changing a sticker's size only re-lays out, no reshape; a texture object goes through the host texture renderer; a chip takes no width and the display box does not draw it; with only the paint layer changed, `text_nodes_shaped == 0` and the retained layout handle is unchanged; with only effect indices changed the whole frame is idle; a large span grows the line box; strokes, shadows and decoration lines enter the scene and no longer emit a full-box `Stroke`; CSS is the span's base.
- `nana-ui` `scene_paint::text`: changing a stroke rebuilds instances without re-laying out, and changing color rasterizes nothing new; a steady-state frame with blurred shadows neither rasterizes nor uploads; instances of one paragraph are ordered by draw order in one entry; a paragraph with a large span is drawn from the Runtime handle; a stroke bitmap is wider than the fill and concentric; blur preserves coverage.

### Not done in this phase

- Decoration lines for vertical paragraphs; `text-decoration-style` (wavy, double), `text-decoration-thickness`.
- Shadows and strokes for color bitmap glyphs (emoji without outlines): they rasterize from the outline only, so these glyphs cannot draw a shadow.
- The painter's self-laid-out fallback path (when the retained layout's alignment box does not match) does not read shape spans and lays out from the node style.

## Performance and license closeout (#99)

- **Performance matrix**: [text-release-matrix-2026-09-20](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/text-release-matrix-2026-09-20/README.md).
  The three gates static steady / paint-only / compositor-only, the #33 head-dirty
  grid (`text_nodes_shaped` / `text_bytes_hashed` /
  `layouts_created` all 0 at 2k/4k/8k nodes), and an 8000-line editor re-laying out one paragraph per edit.
  That note was written before the constraint-only gate. The 2026-09-22 closeout report is
  [text-cutover-2026-09-22](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/text-cutover-2026-09-22/README.md) (five gates, including the new
  `gpu-scene-text-constraint-resize`).
- **Third party and licenses**: [third-party.md](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/third-party.md). The release dependency graph (all-platform normal + build edges)
  has 586 external crates, all permissively licensed. Neither `cryoglyph` nor `cosmic-text` is in `Cargo.lock`.
  Copied snippets (`SubpixelBin::split`, the DirectWrite coverage correction) are credited where they are.
- **Dependency gate**: `check-engine-boundary.py` forbids any workspace member from having a non-dev edge to cosmic-text / cryoglyph /
  glyphon (including renamed forks). See "How the boundary is enforced by machine".

### Measurement forks added at closeout (2026-09-22)

After the cutover a few paths still did not measure text through `nana-text`. Or they measured, but the answer depended on cache warmth:

| Path | Before | Now |
| --- | --- | --- |
| Markdown geometry (`UiWorld::markdown_layout`) | Summed per-grapheme widths from Runtime's `GlyphCache`, estimating unseen characters at 0.6em; painting handed a whole span on one line to the engine | Hands each line of each span to the engine once (`text_engine_shaper::grapheme_advances`), taking each grapheme's advance from the difference of adjacent carets. The gap after a bold span on the gallery rich text page is gone: the title run goes from 61.20 → 44.90 px |
| SelectableRichText box height, drag selection and highlight | Box height hard-coded from the number of `\n` lines, hit testing a fixed 12 px per grapheme, highlight covering the whole content box | Box height measured by Runtime from the box width; hit testing and highlight both read the node's retained `TextLayout` (10db26c1e) |
| Single-character fast path (`NanaTextEngineShaper::shape_cached`) | Returned a cached advance and `ascent: None` for a seen character: the same "A" had ascent 13.57 with a cold cache and fell back to a 0.8em baseline with a warm one | Deleted. The engine's own layout cache already answers repeated strings; `GlyphCache` is left to the engine-less test shaper |
| `RuntimeLayoutEngine::layout_style_tree` (css-parity) | Hard-coded `MeasureTextShaper` | The caller passes the shaper |

The behavior tests are in `crates/nana-ui/tests/rich_text_measurement.rs` and `a_measurement_does_not_depend_on_what_was_measured_before` in `nana_text.rs`. Each was A/B'd against the old implementation (which reported 134.7 vs 121.0 px and ascent `None`).

**Not changed**:

- A set of control chrome still estimates width by character count: `estimated_text_width` in `world/geometry.rs` and `menus.rs`
  (shortcut badges, menu hints, the command palette, calendar tooltips), signature help overlays, line-number gutters, chart legends. They size
  **boxes**. The characters inside are still laid out and painted by the engine. The worst case is a box too wide or too narrow. Switching to real
  measurement needs the engine at the geometry stage. Separate issue.
- `PaintContext::measure_text` and `PaintRecording::record` still fall back to em estimates in a world with no engine.
  That is the "host installed no engine" case. Product hosts always install one.

## Glyph placement and CPU rasterization

Painting a laid-out run of text has to walk the same path again: the line's baseline, a pen advancing along the run, the shaper's offsets converted to page y, then rounding by the same rules. If the two places that draw glyphs each walk their own path, the same glyph comes out a pixel apart. So this pass lives in `nana-text`, and there is only one copy.

`TextLayout::glyph_placements(scale, origin)` gives every glyph to draw in paint order: first the lines, then ruby, emphasis marks and object labels. Each entry carries the font instance, the raster font size (layout px times `scale`), the glyph id, the pen position (x keeps a fraction for subpixels, y lands on a whole-pixel baseline), the byte offset it represents and its source (`GlyphSource`). A vertical paragraph gives columns; a sideways run is marked `sideways` and rasterizes with a quarter turn clockwise. A run the font layer failed to instantiate has no glyphs to draw and is skipped; its advance is still in the line, so later glyphs do not move. The renderer's resolve is this pass plus color.

The `raster` feature (`gpu` in `nana-ui` turns it on) brings `nana_text::raster`:

- **Raster key**: `GlyphRasterKey` and its parts (`GlyphFontId`, `GlyphVariationId`, `GlyphSynthesis`, `GlyphRenderMode`, the quarter-pixel `SubpixelBin`, the font size stored as bits). Color, opacity and transform are not in the key.
- **`OutlineRasterizer`**: rasterizes one key on the CPU into a `GlyphImage` — grayscale coverage, a color bitmap or subpixel coverage, plus derived coverage for strokes and blur. The face comes from the same engine's font layer, so rasterization uses the very instance that shaped the text. swash is used only in `raster/outline.rs`.
- **`rasterize_layout`**: all glyphs of a run of text, rasterized and placed (raster pixels of the bitmap's top-left corner), for hosts that composite text themselves and do not go through the renderer atlas. It re-rasterizes every time; callers that paint often cache above `OutlineRasterizer::rasterize` themselves.

There is no GPU here: upload, atlas and compositing belong to the caller. The renderer implements its own `GlyphRasterizer` boundary, raster cache and atlas on top of it; on Windows DirectWrite takes over outline glyphs and the rest still goes to `OutlineRasterizer`.

## SceneWgpuPainter::text (Phase 6, #97)

The drawing half moved from cryoglyph to NanaUI's own subsystem. The directory is
`crates/nana-ui/src/scene_paint/text/`:

```text
shaped paragraph
        │  TextLayout::glyph_placements (nana-text)
        ▼
NanaGlyphRun / PlacedGlyph                        glyph.rs
        │  GlyphRasterKey (nana_text::raster)
        ▼
GlyphRasterCache ── GlyphRasterizer               raster_cache.rs / raster.rs (OutlineRasterizer in nana-text)
        │  GlyphImage
        ▼
GlyphAtlasManager ── GlyphUploadQueue             atlas.rs / upload.rs
        │  GlyphAtlasEntryId
        ▼
TextPipeline                                      pipeline.rs / mod.rs
        ▼
      WGPU
```

No layer below resolve knows who laid out the paragraph. #99 replaced exactly the two places above resolve.
The paragraph is laid out by the `nana-text` engine. The rasterizer's face source became the engine's font layer.
Below that, the raster cache, atlas, upload queue and pipeline did not change a line.

### The painter takes the retained layout

A paragraph has two sources. The first has priority:

1. **The one Runtime retained** (`ScenePrimitiveKind::Text.layout`). The handle itself is the identity.
   `TextLayoutId` is two `u32`s. A generation slot is never reissued at the same generation. So it packs
   straight into a `u64` (top bit set; see `PARAGRAPH_IS_RETAINED`). **No string read, no hash.** wrap, ellipsis,
   max-lines, `white-space`, direction — every one of those constraints was already decided by Runtime. Take the lot.
   Do not rebuild them from the scene's description of them.
2. **The painter lays it out itself** (`lay_out` + `ShapeCache`). For text with no handle (editor presentation,
   EmptyState / Modal built-in text), and when the handle is unusable.

The only precondition is the **alignment box**: glyph placement is aligned inside `max_width_px`, and the painter
pins the paragraph to the left edge of the box. So `layout.constraints.max_width_px != Some(bounds.width)` must
fall back to the second path. A Switch with a trailing control, a ListItem that overrode the text box with content
geometry, are such nodes. Otherwise centred text is drawn off-centre. It is one float comparison, and the criterion
checks itself: whenever the two sides disagree, the painter lays out its own copy that frame and cannot draw wrong.

`text_retained_layouts_drawn` counts the paragraphs that took the first path. Read together with `shape_cache_misses`,
it tells you which path a workload really takes.

A/B (`nana-text-paint-benchmark`; alternating the two run orders, 3 rounds each, min. See
[performance-data](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/text-paint-retained-layout-2026-09-20)):

| workload | batch p50 (painter lays it out) | batch p50 (handle) | |
| --- | --- | --- | --- |
| `static-unique` 10k | 2.949 ms | 2.070 ms | −29.8% |
| `static` 10k | 2.373 ms | 1.995 ms | −15.9% |
| `mutate-1pct` 10k | 2.222 ms | 2.037 ms | −8.3% |
| `mutate-1pct` 1k | 0.144 ms | 0.128 ms | −10.8% |

The `static` row is worth explaining: on both sides `shape_cache_misses` is 0. What is saved is **one HashMap lookup
per node per frame**. The old path called `shape_cache.holds(hash)` to confirm the paragraph was still in the cache.
The scene holds the handle. There is nothing to ask. The 30% on `static-unique` is what the old path paid to keep a
cache of 10k unique paragraphs. Now it keeps none.

### Three lifetimes, deliberately different

| | Owner | Key | Who may reuse |
| --- | --- | --- | --- |
| shaped paragraph | one painter's CPU state | text + style + box + font generation | every frame of the same painter |
| glyph bitmap | one painter's CPU state | face instance + size + subpixel bucket + synthesis + font generation | every target of that painter |
| atlas placement | **device** state | the same, through a generation handle | every window on the same device |

**Colour, node opacity and scene transform are not in the raster key.** Two colours of one glyph share one bitmap.
An animated label is not rasterized again every frame.

### Handles, not coordinates

`GlyphAtlasEntryId { index, generation }`. An instance is built only after every placement in this frame is settled,
by reading the rectangle back from the handle. So the atlas may relocate (compact) or evict a glyph within the same
frame, and a run that already recorded it loses only that glyph. It never samples another glyph that now occupies
that rectangle. When a slot is recycled its generation increases. An old handle is rejected and counted in
`atlas_stale_handle_rejects`.

A draw command retained across frames (`PreparedBatch`) has another gate: when `text_placement_epoch`
(evictions + relocations) changes, it is rebuilt, because the rectangles baked into those instances are no longer
that glyph's.

### Two pipelines, one atlas

- **Axis**: pixel-aligned text under translation. One 24-byte instance per glyph, four vertices. Clipped by the
  scissor the batch already carries. Nearest sampling. Almost every glyph in the shell takes this path.
- **Affine**: text under rotation / scale / rounded clip. Six vertices per glyph. Carries the same homography and
  fragment clip as `Quad`. Linear sampling.

Both share the same atlas bind group, so the pages a rotated label brings in are hits for the upright labels.
One segment fixes one pair (mask page, colour page). When the page changes the segment **breaks** instead of
reordering, so the glyph order inside a run is always document order.

### atlas policy

Pages are 1024² (mask 1 MiB / colour 4 MiB). Total budget 48 MiB. Every glyph keeps a 1-texel transparent gutter
around it, and **that ring of 0 really is uploaded**. A rectangle reused after an eviction still holds the
previous glyph's pixels. A rotated quad samples half a texel outside its own rectangle.

Two 1×1 placeholder pages exist only to fill the bind group: a shell that never draws an emoji therefore does not
pay 4 MiB for a colour page. When something does not fit, it tries in order: open a new page → compact (`compact`;
repack the live glyphs by height; `atlas_relocations`) → evict the coldest quarter (`glyph_atlas_evict`).
**A glyph used this frame is never evicted**, so what is being drawn is not pulled out from under it.

Compaction requires the bitmap of every live glyph to still be in the raster cache. Otherwise the whole compaction
is rejected and the atlas stays as it is. Moving glyphs and then failing to restore their pixels would turn the
glyphs being drawn this frame into blanks.

### Upload only the new regions

A new glyph = rasterize → allocate a rectangle → enqueue one upload region → submit before drawing with it.
Re-uploading the whole atlas every frame is forbidden. `glyph_upload_regions` / `glyph_upload_bytes` are the
evidence for that.

In-flight safety comes from `wgpu::Queue::write_texture` itself: it copies the bytes into the queue's own staging at
call time, and orders the transfer before the next submit. A hand-written ring needs a fence the painter cannot see.
The painter owns neither submit nor the surface.

A queued bitmap is an `Arc<GlyphImage>`. That is also the lifetime contract: the raster cache may evict the entry
that produced it before the upload really happens. The queue never points at freed bytes.

### Device loss and recreation

When the host switches device (`switch_gpu`) it clears every painter. They are rebuilt on demand on the new device.
The atlas, the instance arena and the raster cache all go with the old painter. Everything needed to draw text sits
on the CPU side of that line: the paragraph is the one Runtime retained and handed down with the scene. The face is
in the process-level engine. `bump_surface_generation` only changes the scene generation and does not trigger
extract. So the first frame on the new device rasterizes and uploads each glyph it uses once. **No layout, no
shaping.**

`crates/nana-ui/tests/text_device_recreation.rs` pins this down: draw the same scene on device A, `destroy()` A,
draw again on a newly created device B with a new painter. `text_retained_layouts_drawn` equals the number of
paragraphs. The painter's own paragraph cache has 0 hits and 0 misses. The rasterize and upload counts match the
first device. The read-back pixels are byte-identical. Then the same painter switches to 2× and draws again: it
still draws those retained layouts, and only rasterizes the new size. It is a separate test binary because it needs
two devices (see the Windows NVIDIA deadlock in `test_gpu.rs`).

### Counters

`SceneWgpuPainter::text_glyph_counters()`: resolve requests, rasterization count, raster cache hits / misses /
evictions / bytes, atlas hits / misses / evictions / pages / bytes / occupancy in thousandths, upload regions and
bytes, relocations, stale-handle rejections, pipeline draws.

### Pixel difference from cryoglyph

On the day drawing changed, of component-gallery's 561 snapshots **551 are byte-identical**. 1 is
`gallery-sidebar-collapsed-dark`, which was already flaky (it had been blamed on the spinning Spinner; re-checked
on 2026-09-20, two consecutive runs, zero difference). The remaining 9 are `motion-*`, and they changed for the
**better**:

The old affine path stored the mask as an RGBA image of "RGB=255 + A=coverage", and the shader then did
`sampled.rgb * color.rgb`, so the antialiasing on a glyph's outer edge was multiplied by coverage twice. The new
path's mask page is R8Unorm. Coverage acts on alpha only, so rotated text edges are no longer darkened (by at most
36/255).

The raster size is keyed exactly, by f32 bits. There is no bucketing: a 1/64 px bucket would move a 15.6px heading
to 15.59375, and an antialiased edge by up to 11/255. This phase had no reason to change rendering. The key space is
bounded by the raster cache's byte budget and LRU, and the shaped paragraph above it was already keyed on the same
exact size.

### CPU: one extra pass, measured

Same machine (Apple M4, release), 400 lines of labels, about 10k glyphs, the batch rebuilt every frame, shaping all
cache hits:

| | batch p50 | batch p95 |
| --- | --- | --- |
| cryoglyph (Latin) | 0.272 ms | 0.288 ms |
| SceneWgpuPainter::text (Latin) | 0.316 ms | 0.344 ms |
| cryoglyph (24 unique hanzi per line) | 0.293 ms | 0.338 ms |
| SceneWgpuPainter::text (same) | 0.314 ms | 0.348 ms |

The 0.02–0.04 ms gap is **the extra pass**: cryoglyph writes vertices as it walks, while here the handles are
recorded first and instances are built from the rectangles once every placement in the frame is settled. That is
exactly what the generation handles are for: a relocation or eviction in between cannot leave an already-written UV
pointing at another glyph. So it is not an overhead that can be dropped on the way past.

Both rewrites that tried to remove it were written and measured. Both land in the noise: caching the rectangle
alongside the handle into the placement, and a fast path and a slow path chosen by epoch (0.324 ms — slightly
slower; one more branch). Shrinking the placement from 32 bytes to 20 bytes (0.317 ms) stayed. It saves memory on
its own, and colour became packed once per run instead of once per glyph. The first was deleted: the extra dual
path bought nothing. The 20-byte one also puts affine-path colour through 8-bit quantization, consistent with the
axis path and cryoglyph, at the cost of at most 1/255 of colour error on rotated text.

What it buys: no per-node GPU allocation every frame (rotated text used to need one texture, one bind group and one
vertex buffer per distinct transform), and windows on the same device no longer each hold an atlas.

### Not done in this phase

| Item | Status |
| --- | --- |
| Decouple logical `TextLayout` from raster scale | Done in #99: layout is laid out in logical px, device scale enters the raster size and the subpixel bucket only at resolve, a DPI change only rasterizes again and does not re-lay out |
| Persistent GPU instance / zero prepare | Done in #98, see the next section |
| SDF / MSDF / LCD | LCD is done (Windows ClearType), see "Crispness"; SDF / MSDF still not done |
| Share CPU bitmaps across devices | One raster cache per painter. Shared across windows on the same device (`swap_target` swaps only per-target buffers); a device switch rasterizes once again |

## Retained text (Phase 7, #98)

Phase 6 swapped the **draw** half for NanaUI's own subsystem. But every frame still rebuilt instances from zero:
walk each glyph, look up the atlas, write 24 bytes, then compare with the previous frame. As long as anything on
screen is moving, the batch is rebuilt and that bill is paid again. An interface with 48k glyphs walks
forty-eight thousand of them per frame because one spinner in the corner is turning.

Phase 7 makes it not have to.

```text
layout unchanged          ── still that one shaping result
phase unchanged           ── the bitmap was rasterized for this subpixel phase
font generation unchanged ── still the same face ids
atlas unchanged           ── the rectangles are still these glyphs'
        ↓
      reuse the TextGpuEntry's instance range
        ↓
      draw
```

The first three are compared in `prepare`. If any of them fails, **only** that entry is rebuilt. The fourth does
not rebuild but **patches**: the entry keeps an atlas handle per glyph, so after a relocation it just reads the
rectangles again. No shaping, no rasterization, no upload.

### Three tables, each with its own lifetime

`scene_paint/text/` now has `entry.rs` as well:

| | Content | Written when |
| --- | --- | --- |
| entry block | the instances a piece of text resolves to (24 B/glyph) + one atlas handle per glyph (8 B) | text / phase / font generation changed |
| run row | that piece of text's **whole-pixel origin, colour, opacity, which presentation it uses** (48 B) | it moved, changed colour, faded in or out |
| presentation row | transform and clip (160 B), deduplicated bitwise | transform or clip changed |

An instance holds **no** position, colour or transform any more. It holds only the offset from the run origin, the
atlas rectangle, and a run row number. So:

- **Moving** (whole pixels): change two floats in the run row. A subpixel move still re-resolves, because it really
  swaps the bitmap. (After #223 the fraction that translation brings in is snapped off by the painter; only a
  fractional move of the layout position itself still re-resolves.)
- **Recolouring**: change four floats in the run row. Glyphs of solid-colour text carry no colour of their own; they
  inherit the run row. Only glyphs in a rich span whose colour differs from the paragraph's carry one. A recolour
  also touches only its own block.
- **Fading**: the run row's `opacity`. The shader multiplies it into alpha. It is **no longer** folded into the
  colour, so a rich text fade does not reshape because a span colour changed. That was a real defect left over from
  Phase 6.
- **Rotation / perspective**: one row in the presentation table. The per-corner homography moved into the vertex
  shader.

### One pipeline, no longer two

Phase 6's Axis / Affine pipelines became one. The difference shrank to two flags in the run row: whether corners go
through the homography, and whether sampling is nearest or linear. Rotated text is therefore also one 24-byte
instance and four vertices per glyph, instead of six vertices each carrying a transform and a clip.

Also fixed on the way: rotated / scaled text used to ignore `clip-path: polygon()` and `circle()`. The affine shader
hard-coded the polygon count to 0. Now the polygons go to the GPU with the presentation row, and text is clipped by
the same shape as the Quad next to it.

### arena and the draw-order index table: draw order and storage order are not the same thing (#224)

Instances live in a storage buffer. Draws do not walk it directly. A draw walks a **`u32` index table ordered by
draw order** (a per-instance vertex buffer). The vertex stage reads a slot number, then fetches that slot's
instance from the storage buffer. Each table hands out positions through its own `SlotArena`. A block always
belongs to its entry:

- **arena**: where an instance lives in the storage buffer. When one piece of text changes, only its own few hundred
  bytes move. **Growing a block is the same**: it moves elsewhere in the arena, and it still writes only its own
  block. The space handed back merges with the space on either side and is cut, best-fit, for the next block; at
  the end the arena simply shrinks. So a hole left by a growing paragraph is used by a later block instead of
  pushing the arena into compaction. The arena compacts only when something does not fit or when holes exceed half.
  It has nothing to do with draw order. After compaction it leaves a third free. "Does not fit" is judged by
  trying each block this frame needs, in turn, against the existing holes (the same best-fit rule as real
  allocation, without copying the hole table). Better an extra compaction than a buffer swapped mid-frame.
- **order**: an entry's range in the index table. Its slots point at its block slot by slot (the spare slots point
  at a size-0 instance, which draws no pixels). Two adjacent ranges merge into one draw; non-adjacent ones cost one
  more draw. Once the budget is used up (one draw per 1024 index slots), or holes take half, the **index table** is
  compacted in draw order. Four bytes a slot. Not one instance moves.
- Blocks keep 1/8 slack by size class (at least 4 slots). The slack is filled with size-0 instances. Typing up to
  the twelfth character does not step up a class.
- **The range is the same size as the block. No growth slack per piece of text.** Slack would let a piece of text
  grow in place in draw order without splitting a draw. But every empty index slot is still a four-corner quad the
  vertex stage runs (about 2 ns a slot on this machine). Measured: giving every range twice the slack took draws
  from 90 to 1 and index bytes from 179 KB to 21 KB, but cost about 20% more GPU time. The break budget was
  measured too: one draw per 256 slots halves index bytes, triples draws, and costs about 2% GPU. Not worth it.
- **Where text length keeps changing, leave runs of gaps at compaction.** When compacting draw order, look at what
  made ranges move over this stretch: more than half is paragraphs growing their range (tables, logs). The new
  layout leaves 128 slots free per 512 slots. Gaps are filled with `VACANT_INDEX`. The vertex stage does not read
  an instance; it culls outright. A draw can run straight across it. Later a piece of text grows: it takes from the
  gap immediately after it; if a few pieces sit in between, they are shifted back into the gap (only their indexes
  are rewritten). It grows in place. No draw is split. Gaps are about a fifth of the index slots, far less than the
  one and a half times that per-range double slack would cost. Paragraphs coming and going (scrolling, moving into
  and out of the viewport) make ranges change places, and there no gaps are left: gaps cannot stop them from
  splitting draws, only make every compaction bigger. When a range shrinks it keeps its old range; the extra slots
  become `VACANT_INDEX`.
- **Once text stops growing, the gaps are handed back (#230).** A gap could previously disappear only at the next
  budget-triggered compaction. A table that finishes one round no longer compacts, and every frame after that runs
  quads for gaps nothing uses. Now, when one target goes `SETTLE_FRAMES` (60) drawable frames with no range
  growing, it runs one compaction that leaves no gaps. Only the index table is rewritten. The slack a shrunk range
  left is taken back too. It is the same decision as the budget compaction; at most one compaction per frame. Any
  compaction during the quiet period leaves no gaps. When text grows again, it waits for the break to use up its
  budget; the next compaction leaves gaps again. It does not oscillate every frame.
  - It is counted in frames, not time, because a gap's cost accrues per drawn frame. After 10k labels stop
    changing, quads go from 152 085 to 136 045 a frame, draws from 15 to 1, and GPU time from 2.86 to 2.72 ms
    (load < 4, 10 interleaved rounds, min;
    [data](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/text-gap-reclaim-2026-09-24/README.md)).
    That compaction rewrote 544 KB of indexes. No spike is visible in frame time; a few frames of gaps pay for it.
    60 frames is several times that. Text that updates intermittently keeps its gaps.
  - A frame that reuses the previous frame's batch (the scene did not change; a HostTexture / GPU view sits next to
    it) still draws text and still pays for gaps, but does not go through the flush. So it counts as a quiet frame
    too. On the frame it comes due the batch is not reused; it is rebuilt once. If the batch has no text, it
    neither counts nor rebuilds.
  - When device loss or font registration invalidates draw order wholesale, the "leave gaps" mark and the growth
    counter are cleared with it.
- A compaction advances each table's own generation. Indexes only know slot numbers, so a block the arena
  compaction did not move does not even need its index rewritten.

A frame where draw order is unchanged and no block moves uploads 0 bytes of index table.

Measured (tag `docs-archive`, `archive/docs-notes/performance-data/text-draw-order-2026-09-22/`; 10k labels, every
frame 1% of rows randomly switch to random-length text, 60 frames): instance uploads 1 136 190 → **40 300** B/frame.
That is the rebuilt paragraphs' own blocks (about 410 B each). The 1k-label case is 9 363 B/frame. Ten times the
labels, 4.3 times the bytes. It does not grow with the total glyph count on screen. The new index upload is
65 107 B/frame. Together about one ninth of before. Draws 87 → 21. Over 600 frames it is 50 KB and 25 KB, 36 draws.
A `transform` that turns the whole plane used to cost half a megabyte of instance compaction per frame; now it is
87 KB of index compaction, 0 instance bytes, the same draws. The remaining cells are 0 on both counts. CPU
instructions: the cells with rebuilds are 2–5% lower; the cell that writes one small block per frame is about 1%
higher (see "in-flight"). GPU time interleaved A/B is in the noise. 620 gallery frames are byte-identical.

Costs and limits:

- One more dependent read in the vertex stage (slot number → instance). The slot number is a fixed-stride fetch
  from a vertex buffer; only the instance fetch is a storage read.
- The vertex stage's storage buffers go from two to three (run table, presentation table, instances). wgpu's
  `downlevel_defaults` allows four. Reading storage in the vertex stage was already the run table's precondition
  (`DownlevelFlags::VERTEX_STORAGE`). No device became newly excluded.
- A single target's instance ceiling went from `max_buffer_size` (256 MiB) to
  `max_storage_buffer_binding_size` (128 MiB by WebGPU default, about 5.6 million glyph slots). An entry holds a
  whole paragraph's glyphs, not per line. So one very long paragraph (an entire log in one text node) really can hit
  it. When it does, it no longer fails bind group validation: the slack the arena compaction keeps stops at the
  ceiling. If one frame's glyphs do not fit, it stops drawing from the largest paragraph down until the rest is two
  thirds of the ceiling. Everything else still draws. Only if it just barely fits does the arena sit at the
  ceiling, and then the next block that moves does not fit and the whole arena is rewritten every frame. Counted as
  `text.instances.limit_frames`; the event `text.instances.limit_exceeded` (throttled to ten seconds). The index
  table is 4 B a slot with a `max_buffer_size` ceiling, more than six times roomier than instances, so it is not
  limited separately.
- VRAM: the index table is 4 B a slot. Both arena and index table leave a third free after compaction. 10k labels
  (about 100k block slots) is 3.9 MB of instances + 0.7 MB of indexes. At #98 it was 3.1 MB of instances.

The run row number is likewise **one fixed slot per entry**, not a draw-order index. A list scrolls off its first
row; the remaining eleven rows do not rewrite every instance because "they all moved up one".

### in-flight

`queue.write_buffer` copies the bytes into the queue's own staging at call time, and orders the transfer before the
next submit. But **every call has a fixed cost**: about fifty thousand instructions on Metal, far more than the
bytes it moves. Rebuilding a hundred labels in one frame is a hundred calls. So instance blocks and index ranges no
longer each call `write_buffer`: everything one frame needs to write goes into one staging ring per target (one or
two `write_buffer` calls), then `copy_buffer_to_buffer` block by block inside the host's encoder, ordered before
the pass that reads them. `mutate-1pct` at 10k labels therefore uses 5% fewer instructions.

Ordering is guaranteed by the submission: a pending write lands before the next submit and its command buffers, and
commands that copied out of the ring in the previous submit have already run by then. The ring is at least four
times what one frame needs. If the same target draws twice before one submit, the two draws get different spans of
the ring and each draws its own text. When writing block by block directly, the first draw used to draw the glyphs
the second write put there (`two_paints_of_one_target_before_one_submit_each_draw_their_own_text`). This covers
glyphs only: the run table, the presentation table and projections still call `write_buffer` directly. If position,
colour or size changed between two draws, the first draw uses the second's values. Existing hosts all draw once and
submit once (multiple windows are different targets), so those two tables were not moved into the ring either. The
cost is one more copy command on a frame that writes only one small block: about 1% more instructions on `static`. Replacing this path with persistently mapped buffers the GPU reads directly was measured and not done: it is under 1% of CPU in real scenes ([GPU](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md) §7).

A frame that needs to write more than 256 KiB (a long list's first frame, a whole-arena compaction) does not go into
the ring. It builds a temporary buffer and drops it once copied. The ring does not grow with it. Instances and
indexes **each get their own block**: in one frame each slot is written at most once, so each block is no larger
than the buffer it fills, and that buffer has already been allowed by the device. Combined into one block,
instances near the storage ceiling plus the index bytes could exceed `max_buffer_size`.
`a_frame_too_large_for_the_staging_ring_lands_whole` reads both GPU buffers back and compares them slot by slot
against a shadow rebuilt from the writes. It covers both the ring and the one-shot path.

When staging cannot be obtained (the device is lost and the host has not noticed, or VRAM is short), wgpu reports
an error and returns `None`. It then falls back to a one-shot buffer; failing that, it copies nothing. The frame
does not panic. The host's device recovery (#97) takes over as usual, and all arena and draw-order positions are
void: the next frame re-places and rewrites the blocks and ranges it draws as a whole, instead of trusting a buffer
that never received the bytes (`tests/text_lost_device.rs`).

Because blocks and ranges are copied inside the host's encoder, **that encoder must be submitted**. Drop it and the
GPU buffers fall behind the arena and draw-order ledgers (the contract is written on `SceneWgpuPainter::paint` and
[GPU](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md)).

When a buffer is replaced, wgpu itself guarantees the old one lives until the commands that reference it complete.
What this side must guarantee is that **no handle still points at the old layout**. That is gated twice: the arena
generation and the `GlyphAtlasEntryId` generation.

An atlas slot gained a reference count: a glyph an entry is still drawing is not treated as cold and evicted
because "nobody looked it up this frame". Eviction can still take it — the ceiling is hard — it just queues behind
the unreferenced ones.

### Counters

`text_glyph_counters()` adds these on top of Phase 6's batch:

```text
text_gpu_entries_active / created / destroyed / reused
text_gpu_entry_glyphs
text_instance_rebuilds        // re-resolved one paragraph of text
text_instance_patches         // only the rectangle or the run row number changed
text_instance_upload_bytes
text_index_upload_bytes       // draw-order index table (#224): only on a new range placement or a block move
text_index_slots_drawn        // index slots a draw crosses, i.e. quads the vertex stage runs; gaps and slack slots count (#230)
text_presentation_upload_bytes
text_prepare_nodes_considered / skipped / culled
```

`rebuilds` and `upload_bytes` are this phase's acceptance surface: on a still frame both must be 0.

### Benchmarks

`nana-text-paint-benchmark` (same machine, Apple M4, release). Every cell is a frame that **rebuilds the batch**.
The still cell uses one small node that changes its text every frame to knock out the batch cache, because that is
exactly the product situation: something in the corner is moving; do the labels next to it have to recompute.

**Work counts.** 10k labels (48 431 on-screen glyphs; `static-unique` is 68 892):

| workload | resolve/frame | rebuild/frame | instance bytes/frame | reshape/frame |
| --- | ---: | ---: | ---: | ---: |
| static | 58 432 → **2** | — → 1 | — → 144 | 0 → 0 |
| static-unique | 78 892 → **2** | — → 1 | — → 144 | 10 001 → **0** |
| color | 58 431 → **0** | — → 0 | — → **0** | 0 → 0 |
| opacity | 58 431 → **0** | — → 0 | — → **0** | 0 → 0 |
| transform | 58 431 → **0** | — → 0 | — → 550 509 | 0 → 0 |
| mutate 1% | 58 548 → **697** | — → 100 | — → 65 302 | 1 → 1 |

The 2 in `static` are the small node that really changes text every frame, the one that knocks out the batch cache.
None of the other 10k labels on screen re-resolves a single glyph. The three animated cells have identical
per-frame numbers at 60 / 120 / 240 Hz (listed once in the table), because nothing here is amortized by time: one
colour change is one row change.

**Time** uses the 1k-label cell (4 842 glyphs). Same machine, Apple M4, release, median of five. They differ by no
more than 3%:

| workload | batch p50 (before → after) |
| --- | --- |
| static | 0.765 → **0.322** ms |
| static-unique | 11.979 → **0.330** ms |
| color | 0.760 → **0.320** ms |
| opacity | 1.100 → **0.552** ms |
| transform | 2.054 → **0.514** ms |
| mutate 1% | 0.780 → **0.344** ms |

The **milliseconds for the 10k-label cell are not quoted**: that viewport is 4400×4400, and frame-to-frame times
are at the mercy of memory bandwidth and scheduling. The same binary run three times in a row can give
13.2 / 25.0 / 14.5 ms. The counts are stable. Judge by the counts.

How to read it:

- **`color` / `opacity` / `transform` have 0 instance bytes** (transform does not in the 10k cell; see below). All
  three write tables only. Colour writes the run row. Opacity goes through the opacity group. Rotation writes the
  presentation row.
- **`transform` still has half a megabyte in the 10k cell**: the container is turning, and dozens of labels turn
  into and out of the viewport every frame. Their blocks in the arena are no longer adjacent, so a compaction runs
  once the budget is used up. A compaction is one contiguous write, not a re-resolve. The same cell's
  `resolve/frame` and `rebuild/frame` are both 0.
- **`static-unique`'s 12 ms → 0.33 ms is not what retention bought.** It is the shaping cache's eviction rule
  becoming "anything asked about in the last two frames is not evicted": when a screen holds more unique paragraphs
  than the cache, it used to push out the paragraph it had just drawn to make room for the next, and then reshape
  the whole screen every frame.
- **The second half of the speedup is not on the text path.** Breaking `prepare` down (same machine, 10k labels,
  static, median of three):

  | Measured to | When retention landed | Now |
  | --- | --- | --- |
  | `prepare` returns directly (the painter's own per-node loop) | 6.75 ms | **4.65 ms** |
  | plus building and hashing the shaping key | 7.12 ms | 5.40 ms |
  | the whole thing | 8.74 ms | 6.80 ms |

  The painter's per-node loop used to be seven tenths of it, and three things in it were busywork every primitive
  did every frame: three candidate clip boxes built and thrown away, a cycle-protection `HashSet` built for the
  ancestor chain every time, and a walk up the parent chain on scenes with no compositor layer. All three are
  fixed. The loop itself is 6.75 → 4.65 ms. Text itself is about 2 ms left, the largest item being the
  shaping-key hash. Once #99 wires up `nana-text`'s layout handle, that item can become one generation comparison.

  The loop also did a per-primitive ancestor-chain query. `opacity_groups` was asked twice a frame (once while
  drawing, once while culling), and `compositor_paint_opacity` once inside `draw_primitive`. Both grow with **tree
  depth**, and text in a real shell sits ten to twenty elements down. Now the ancestor half is memoized per node (a
  node that is not a group shares its parent's `Arc`, so nothing is allocated):

  | Wrapping levels (`--depth`) | Before | After |
  | ---: | --- | --- |
  | 0 | 0.319 ms | 0.307 ms |
  | 4 | 0.410 ms | 0.308 ms |
  | 8 | 0.483 ms | 0.311 ms |
  | 16 | 0.631 ms | **0.303 ms** |

  It used to cost two tenths more per eight levels. Now it is flat. Every other cell in the tables is `--depth 0`,
  the flattest tree. A real interface gains more than the table shows.

- **A style change's bill is not in the painter.** `batch p50` is only the batch stage. On a frame where a container
  changes style, the time is in `RuntimeDocument::flush`. At 1k labels and sixteen wrapping levels it is 4.4 ms
  against 0.96 ms. The benchmark therefore gained a `flush p50` column.

  The money is spent like this: a container changes style once, and the primitives of every node under it are
  rebuilt. Each node walks its parent chain twice in that pass: once to compute the inherited transform / opacity /
  clip, once to work out where the chain puts it in draw order (one removal, one insertion). For siblings both are
  identical, and the pass walks the subtree in order anyway. So one slot each remembers whose it computed last:

  | Wrapping levels | color | opacity | transform |
  | ---: | --- | --- | --- |
  | 0 | 1.94 → 1.78 ms | 2.29 → 2.00 ms | 2.99 → 2.65 ms |
  | 8 | 3.04 → 2.33 ms | 3.67 → 2.87 ms | 3.16 → 2.34 ms |
  | 16 | 3.71 → **2.64** ms | 4.53 → **3.55** ms | 5.05 → **3.44** ms |

- **Scene nodes became `Arc<ExtractedNode>`.** `rebuild_node_primitives` cannot borrow the node and later needs
  `&mut self` to insert primitives, so it always cloned the whole thing — and `ExtractedNode` is 784 bytes. A
  container style change rebuilds every descendant, so that was nearly a megabyte of memmove per frame. Stored as
  an `Arc`, that clone is one refcount increment; the insertion side pays one allocation, but a frame inserts only
  the few nodes that changed and rebuilds a whole subtree. `flush p50` (five rounds each, min, run interleaved):

  | Case | Before | After |
  | --- | --- | --- |
  | 1k labels color, flat | 1.616 ms | 1.568 ms |
  | 1k labels opacity, flat | 1.737 ms | 1.706 ms |
  | 1k labels transform, flat | 1.868 ms | 1.812 ms |
  | 1k labels opacity, sixteen levels | 3.124 ms | 3.031 ms |
  | 1k labels transform, sixteen levels | 2.904 ms | 2.790 ms |
  | 10k labels opacity, flat | 29.222 ms | **26.843 ms** |

  The more nodes, the more it saves, because what is saved is the per-byte item. The 557 rendered gallery frames
  are byte-identical. This one changes storage only.

- **A fade no longer re-sorts the whole scene.** Draw order asks a node one thing: "is it translucent".
  `is_opacity_group` looks at `opacity > 0 && opacity < 1`, not at the value. But the decision whether to re-sort
  compared the bits of `local_opacity`. So a container fading from 0.35 to 0.37 recomputed the key of every
  primitive in the scene and re-sorted every frame. Comparing `paint_order_facts` instead (z-index, translucency, a
  non-identity filter, mix-blend, whether it opens a stacking context): 1k labels opacity 1.911 → 1.693 ms, at
  sixteen wrapping levels 3.338 → 2.725 ms, 10k labels 29.162 → 25.595 ms.

- **A primitive remembers its place in draw order; a rebuild overwrites in place.** Rebuilding a node's primitives
  used to do four B-tree operations per primitive: remove from `primitives` and insert again, remove from `ordered`
  and insert again — each key walking the parent chain, and the removal's work thrown away. Almost always the
  primitive and the key removed were the ones inserted again.

  Now the primitive and its `SceneOrderKey` are stored together: a removal uses the stored key, and a rebuild does
  not clear first. `insert_primitive` overwrites in place and leaves `ordered` alone when the key did not change.
  Every rebuild stamps the slots it wrote; slots not stamped are primitives this node no longer has, and are
  retired at the end. The `apply_delta` line that deleted all of an extracted node's primitives first is gone with
  it. The draw-order stack became `Arc<[(i32, usize)]>`, shared by all of a node's primitives.

  | Case | Before | After |
  | --- | --- | --- |
  | 1k labels color, flat | 1.574 ms | 1.155 ms |
  | 1k labels opacity, flat | 1.549 ms | 1.101 ms |
  | 1k labels transform, flat | 1.822 ms | 1.411 ms |
  | 1k labels opacity, sixteen levels | 2.452 ms | 1.643 ms |
  | 10k labels mutate-1pct | 0.324 ms | 0.240 ms |
  | 10k labels opacity, flat | 23.664 ms | **18.720 ms** |
  | 10k labels transform, flat | 27.532 ms | 23.178 ms |

- **A fade no longer re-extracts the whole subtree.** When a container changes opacity once, Runtime used to mark
  the whole subtree `STYLE | RENDER` (`inherited_paint_changed` counted opacity and inherited colour as one class).
  In the 100-label case `render_extraction` was 102 nodes. But a container with any inheritable descendant is an
  opacity group, and that opacity is applied once at the compositing layer; it never lands on descendant primitives
  (`compute_ancestor_state` multiplies only under `if !is_opacity_group(ancestor)`). Those hundred descendants
  re-extracted and rebuilt byte-identical primitives.

  `ComputedStyle::opacity` is read in only three places in the whole workspace (its own `Default`, the
  `resolve_style` that computes it, and the text dirty classification that files it under `TextWork::COMPOSITOR`).
  The render side reads it nowhere. So opacity now marks only `STYLE`: descendants still resolve new cumulative
  values, but are no longer extracted.

  Before this step a scene-layer hazard was closed: an ancestor's opacity is the only thing descendants **bake into
  their primitives** (transform and clip are pushed again at draw time). That used to be held up entirely by
  Runtime marking the subtree dirty. Now the scene decides for itself with `inherited_opacity` (1.0 if it is a
  group) whether that number changed, and rebuilds retained descendants when it did. The `attribute_epoch` slot was
  freed at the same time: it was advanced to make descendants push transform and clip again, and a fade that does
  not change group identity moves no geometry.

  | Case | flush before | flush after | batch before | batch after |
  | --- | --- | --- | --- | --- |
  | 1k labels opacity, flat | 1.212 ms | 0.117 ms | 0.474 ms | 0.314 ms |
  | 1k labels opacity, sixteen levels | 1.914 ms | **0.124 ms** | 0.854 ms | **0.312 ms** |
  | 10k labels opacity, flat | 20.861 ms | 3.907 ms | 9.449 ms | 6.126 ms |

- **A transform change no longer re-extracts the subtree either.** The same shape: a descendant's primitives are
  built in their own space and projected by the chain above them, and that projection is pushed again at draw time
  anyway. `projections` records the projection at the moment a primitive was built. `draw_primitive` derives a
  delta from the current projection and its inverse, and applies it to the primitive's transform and its own clips
  (that is the path retained descendants take when an ancestor scrolls). So a transform now marks the subtree
  `TRANSFORM | INPUT` only, and `RENDER` only for the node itself. The projection / singular-transform batch still
  goes into rebuild separately through `unadjustable_projections`.

  Equivalence is pinned by a test: a four-level tree (hidden overflow clip + scroll offset + opacity group + a
  descendant's own transform). Extracting only the container against rebuilding the whole tree compares each
  primitive's drawn transform, clip and opacity; the difference is within 1e-4. Pushing again fits with one
  inverse; rebuilding multiplies the chain again. Error does not accumulate: every frame's delta comes from the
  same recorded base.

  The cost moved to batch: descendants take the push-again path every frame, so that path itself got two savings.
  Ancestor state remembers one slot by parent node (the draw side has no bracket to clear it, so it is stamped by
  `(instance, attribute_epoch)`), and a primitive with no clip of its own shares the ancestor's `Arc` outright.

  | Case | flush before | flush after | batch before | batch after |
  | --- | --- | --- | --- | --- |
  | 1k labels transform | 1.191 ms | 0.388 ms | 0.446 ms | 0.544 ms |
  | 10k labels transform | 21.417 ms | **7.718 ms** | 7.541 ms | 9.487 ms |

  Frame total: 1k labels 1.64 → 0.93 ms; 10k labels 28.96 → 17.21 ms.

- **A moved subtree refreshes its own visibility index.** Sampling said 48% of paint during a transform animation
  went to `VisibilityIndex::new`: an ancestor's geometry changes, the index is dropped wholesale, and the next
  query rebuilds it from scratch — recomputing a projected bounding box for every primitive in the document,
  rehashing every node, walking every parent chain to rebuild subtree ranges. But an ancestor's transform changes
  where the subtree lands, not which primitives are in it.

  Now it refreshes by `inherited_roots`: `descendants` records the operation range each node's subtree occupies,
  and `refresh_range` walks those ranges, recomputes the leaves, and re-unions upwards. The cost is
  O(range + log n). Descending has to push the lazy translation scrolling left behind down first, or it lands on
  the freshly computed leaves (`moving_one_subtree_...` pins exactly this; fault injection turns it red).

  For this the benchmark gained a `transform-panel`: turn a panel holding eight labels while the other ten thousand
  primitives stay put. A real shell's animation has this shape, whereas `transform` turns the whole document and is
  the worst case. Frame total (flush + batch):

  | Case | Before | After |
  | --- | --- | --- |
  | transform-panel, 1k labels | 0.512 ms | 0.312 ms |
  | transform-panel, 10k labels | 9.933 ms | **5.698 ms** |
  | transform, 10k labels (worst case) | 19.733 ms | 17.224 ms |

- **Tables indexed by node switched to integer hashing.** The standard library's default SipHash is a keyed MAC:
  the right default for a table whose keys come from outside input, wasted on a counter nobody outside the process
  can choose. Drawing one primitive queried several such tables (`projections`, `draw_attributes`, `nodes`, two
  ancestor memos …), and in sampling hashing alone was 9.7% of paint. They are all `NodeMap` / `NodeSet` now; the
  mixing step is rustc's "rotate, xor, multiply by an odd number".

  This changes those tables' iteration order, so "557 gallery frames byte-identical" is load-bearing here: no
  rendering result ever depended on their order.

  The numbers are imprecise this time: machine load was near 10 while measuring, and the same binary could differ
  by 2× between rounds. Nine paired rounds, median of ratios: transform-panel 10k labels -13.3%, transform 10k
  labels -6.1%, color 1k labels -14.0%. Most of the nine rounds are below 1.0. Sampling put the ceiling at 9.7%.
  The same order.

- **An unchanged paragraph no longer rebuilds its shape key every frame.** On a steady-state frame with 10k
  on-screen labels, sampling said the largest cell was `TextPipeline::prepare` (self time 2315, whole frame about
  seven thousand): every label every frame assembled the twenty-odd fields of `ShapeKeyRef`, hashed the whole
  string with them, and compared the result in the shaping cache — a comparison whose conclusion was always "same
  as last frame".

  Now the scene adds a `revision` on `SceneDraw`: the rebuild that wrote this primitive. If that did not change,
  and neither device scale nor font set moved, the shape key cannot change, so the hash the entry remembers is used
  directly. Rich text does not take this path (its key carries colour-split spans, and the caller can override that
  colour).

  `revision` comes from a **process-wide** counter. Counting per scene does not work: the painter identifies an
  entry by (node, slot, pass), and nothing in that says which scene it belongs to. The per-scene version passed
  every test; 258 of the 557 gallery frames caught it.

  Frame total (median of ratios over nine paired rounds): static 10k labels -11.4%, mutate-1pct -9.4%,
  transform-panel -6.2%.

  Two things were fixed on the way. First, the `transform-panel` panel was using `Fill` sizing, which pushed the
  other 9992 labels out of the viewport (`text_prepare_nodes_culled` 9999.9 / 10001) — it was measuring a document
  nobody draws; after the fix 9998 are answered by their own entry. Second, the #8 text gates had never been judged
  on a real machine: `text_counters` was attached to the report after the invariants were computed, so all five
  were `not-evaluable`. Now all five are ok, `text_instance_rebuilds=1` among them.

- **Two more layers scraped off steady-state frames.** A paragraph's width and height (`measure` runs a layout run)
  are recorded on the entry, because that is the shape's size and the shape is already pinned by `revision`. The
  presentation row's "same as the previous row" first compares those four source values, instead of building a
  160-byte row and then hashing it down to forty bits. Frame total: static 10k labels -4.7%, transform-panel
  -12.3%, mutate-1pct -6.7%.

  The painter's own two hot tables (`EntryStore::index`, the shaping cache) also switched to `IdHasher`. In
  sampling, 206 of 213 painter SipHashes came from `prepare`, and the shaping cache's key is a hash already. All
  three cases agree at -5%, and every one of the nine rounds is between 0.94 and 0.97.

  **Two measured and then reverted.** Recorded here so nobody tries them again:

  - Narrowing `FragmentClip`'s `PartialEq` to compare only the vertices within `polygon_count`: sampling ranked
    `FragmentClip::eq` at 11% of paint. After the change, eleven paired rounds gave a median ratio of ±0.4%. Those
    sixteen floats were already zero and vectorized. Symbol-level self time lies at this granularity.
  - "A run whose inputs did not change is still last frame's run": recording ink, origin, flags and the cull
    conclusion on the entry. Fault injection proved six of the seven invalidating inputs load-bearing, and it
    measured at only 1–2% (the first version put the whole `FragmentClip` in the key and was 3–4% slower). A cache
    this fine for one or two points is not worth it.

- **The frame where "every node changed"** (case `color`: 10k labels each switch foreground colour — the shape
  where flush dominates). Sampling pointed at two places, neither of them about colour:

  - **Whether structure changed: reconciling afterwards became bookkeeping on the spot.** The frame plan and the
    cull index are built on "which primitives exist and what each is bound to". Whether that changed used to be
    decided by snapshotting the primitive list of every node encountered before the rebuild loop (one range scan
    plus one `Vec` allocation), then comparing snapshots after — 20 000 scans and 20 000 allocations for 10 000
    nodes, for one boolean. But the adds, deletes and rebindings happen in that very pass, and `insert_primitive`
    and `retire_node_primitives` already know. -15.6%.
  - **The visibility index's `update` refreshes by contiguous run.** Each changed primitive used to walk fourteen
    levels of `set_bound` from the root. A node's primitives are contiguous in draw order, and so are a subtree's
    nodes, so one delta changes a few contiguous ranges. Reusing the `refresh_range` from "a moved subtree
    refreshes itself" is enough. No threshold is needed: when there are many runs, each is O(range + log n), and
    when everything changed it is one O(n) run. -8.2%.

  10k labels color frame total 15.949 → 12.164 ms (**-24%**). All three places that set `structure_changed` were
  fault-injected: the one on inserting a new key already had a test watching it; retiring and rebinding did not.
  They were added in `a_rebuild_that_drops_...` (which first has to get the frame plan to compile) and
  `frame_plan_survives_...`.

  **Another one measured and reverted**: rewriting `refresh_range` from recursion to a flat two-pass (push boundary
  paths, write leaves, union bottom-up). Sampling ranked it first, but it was only -1.3%. Recursion is not the
  cost; `primitive_bounds` (that is, one `draw_primitive` per primitive) is, and both versions pay it. Two
  spellings of two functions walking the same tree for one point is not worth it.

  What is left: the integer-hash switch in `nana-ui-runtime` was tried, did not measure and was reverted (the
  hashbrown tables in `store.rs` and `layout_engine.rs` already hash with foldhash, not SipHash; see the paragraph
  after "The frame after one row of a long list changes" in `resource-budgets.md`). And the per-primitive path (`draw_primitive` → `primitive_bounds`) is now the big
  cost on both flush and batch: the delta for every descendant under one changed ancestor is mathematically the
  same (`A_new ∘ L ∘ L⁻¹ ∘ A_old⁻¹`), so one copy per ancestor would be enough — but that first requires proving
  "the chain below did not move". Further down is retaining the whole frame's draw list (damage tracking), which is
  bigger than any of the above.

### Wrap-up: defects, two policies, three kinds of gate (2026-09-21)

I went through the issue's acceptance criteria one by one, and had the changes independently reviewed as well.
These are what came up. Each has a test pinning it, and each was fault-injected: revert the fix and the matching
test turns red.

**Defects**

- **A glyph that does not fit in the atlas was dropped for good.** `build_entry` did `continue` when
  `atlas.insert` failed, but kept the entry as if complete. Nothing in the paragraph changed after that, so
  steady-state frames kept missing that glyph. Now it is marked `damaged` and re-resolved on the next frame. On a
  clean HEAD `an_eviction_recovers_the_entry_it_hit_without_reshaping_anything` was already red; the other half of
  the reason is the next item.
- **Eviction to nothing no longer skips compaction.** The freed space may be stuck on shelves of differing height
  between this frame's glyphs, and compaction had been tried once only **before** eviction. Now it tries compaction
  again when eviction really freed something; when the page is all this frame's glyphs it gives up immediately and
  does not loop.
- **Glyphs a compaction cannot place were quietly dropped.** Compaction repacks tallest first, and that order does
  not guarantee it can place the same set back. When a page is full, in seeded experiments it usually could not.
  Glyphs that did not fit used to be released outright without counting as an eviction; the placement epoch might
  not move, the entry holding them was never repaired, and afterwards it sampled a rectangle given to another
  glyph. Compaction now trial-places on a fresh allocator and commits only if **all** of them fit. Otherwise the
  atlas stays as it is.
- **Glyphs that do not fit come in two kinds.** One has no room for now, and is tried again next frame (see the
  first item). One is bigger than a whole page (a 1024 px+ display glyph, an emoji): there will never be room, and
  it used to rebuild the whole paragraph every frame. It is no longer retried.
- **Compaction moved instances already written in the same build.** An entry's atlas epoch used to be read
  **after** the build, so a rectangle moved by a mid-build compaction was stamped "current", nobody repaired it,
  and it sampled another glyph. It is read before the build now. An audit that runs only in tests at the end of
  `flush_runs` requires every instance rectangle drawn this frame to equal the rectangle its handle now points at.
  Every test that draws text exercises this invalidation rule.
- **Rebuilding miscounted atlas references.** `begin_build` released as many old handles as there were **new**
  glyphs, so a paragraph getting shorter leaked references, and closing a window released none of its entry's
  references at all. Both are fixed (the latter under multiple windows).
- **Text occasionally jumped a pixel during fractional scrolling.** A line box's top edge is aligned to a whole
  pixel, and then goes round trip through logical coordinates, translation and device scale, coming back as
  1.9999999 often enough. After `floor` the origin is one less and the phase becomes 0.9999999. The aligned whole
  pixel is now the paint origin directly. 12 labels scrolling in 0.37 px steps over 23 frames: rebuilds 16 → 0.
- **Whole-pixel horizontal scrolling rebuilt too.** The phase used to take the fractional part of `(x + e) × scale`,
  where `e` moves in whole pixels and `x` is the fraction flex left behind. Their sum rounds differently at every
  scroll position, the last bit of the phase moves with it, and every label re-resolves. The phase now takes only
  the paragraph's own position plus the **fractional part** of the translation; the whole-pixel part is added
  separately. 12 labels scrolling horizontally in 7 px steps over 39 frames: extra rebuilds 5 → 0.

**Slack and memory**

- `capacity_for`'s 1/8 slack was never used: `begin_build` required the capacity to be **exactly equal** before
  keeping a block. Capacity now comes from the size class (four classes per doubling); if it fits, it is kept. A
  block that grew shrinks only when it is more than twice what is needed **and** frees 64 slots or more. A cell
  whose length oscillates settles at the largest class it ever needed, because moving house is not just its own
  bytes: it lands elsewhere in the arena, draws get split, and enough splits compact the whole arena. (After #224
  only the draw-order index table is compacted; see "arena and the draw-order index table".)
- The CPU slab compacts when holes exceed half. The arena shrinks when a compaction uses only a quarter of it. The
  GPU buffer is swapped both ways with it (shrinking happens only inside a compaction, and a compaction rewrites
  every block anyway, so the swap does not move old bytes).

**Scene scaling is its own policy** (§7). DPI decides the raster size. A scene transform used to always draw a
DPI-sized bitmap and stretch it, so text under `scale(2)` was permanently blurry. Now upscaling steps by half an
octave (1, √2, 2, 2√2, 4). The step is recorded on the entry, and a change happens only after crossing two step
midpoints plus another quarter step. It only scales up, never down, and rotation does not count. So a press or a
fade-scale animation in 0.9–1.0 rasterizes no glyph at all, and a 1× → 2× zoom animation rasterizes once at each of
the two steps it crosses. Jittering twenty frames around a step boundary rebuilds nothing. 16 px text under
`scale(2)` is pixel-identical to 32 px upright text. A step's em is at most 256 raster px.

**Entering and leaving identity no longer rebuilds wholesale.** Translating text aligns the line box to device
pixels; projected text (rotated, scaled, pressed) used not to. So on the frame a container starts or stops turning,
the two paths computed different phases and every label under it re-resolved. The first frame of a press also
jumped a subpixel. The projected path now does the same alignment in its own raster space; at identity the two
phases are equal, and switching changes only the run row's flag. 1k labels, one pass through 0° every three frames:
rebuilds per frame 90.5 → 0.

The old precondition was that the container's translation at the moment of identity landed on a whole pixel. When a
container with a fractional translation starts or stops transforming, the labels under it still re-resolved once.
After #223 the translating path's translation is always snapped to whole pixels by the painter, and the phase comes
only from the label's own position — the same as the projected path computes. That one re-resolve is gone too
(`a_container_with_a_fractional_offset_keeps_its_glyphs_as_it_turns`). What remains is the sub-half-device-pixel
shift the snap removed. The projected path's phase must not follow the translation, or rotating about the centre
would change the translation every frame and rebuild every frame.

**A whole-pixel scroll changes one row.** A translating run's origin used to be its whole-pixel position on screen,
so scrolling one pixel rewrote the run row of every label on screen. Now the whole-pixel part of the translation
lives in the presentation row (shared by labels under the same scrolling container), and the run row stores its
origin relative to it. CPU and shader read the same `whole_translation`. 10k labels, `table-scroll`: 479 KB → 96 B
per frame.

**Fractional translation snaps to device pixels (#223).** Vertically, rebuilds were already zero, because the line
box top is aligned to a whole pixel. Horizontally there was no such step: when the scroll offset is not a whole
physical pixel (trackpad smooth scrolling, or one logical pixel at a non-integer scale such as 125% or 150%), the
paragraph's subpixel phase really changes and every scrolled label re-resolves every frame. The issue left three
questions. The decisions:

- **Which layer snaps.** The place where the painter brings the scene transform into paint space
  (`clip::paint_transform`). It is the only layer that knows the device pixel grid. What it snaps is the
  primitive's **final** pure translation, not the scroll term alone. A primitive's scissor, fragment clip and the
  extra clip wrapping dests (`clip_dests_for`) go through the same function, so quads, icons, text, path meshes,
  HostTexture and CustomRender move by the same whole pixels. Text does not jitter against its background. Frozen
  rows and columns cancel the scroll with their own translation, and after snapping the net translation is exactly
  0; snapping only the scroll term would leave the two a fraction apart, with the header jittering a pixel and
  re-resolving every frame. CSS `translate` snaps the same way: quads, icons and line boxes already rounded
  individually, only text x, path meshes and textures did not, and now they all land on the same whole pixel.
- **Snap to physical pixels, or keep the fraction.** The logical value keeps its fraction; the presented value
  snaps. Runtime's `ScrollOffset`, hit testing and accessibility bounds are unchanged: a slow trackpad scroll
  delivers increments under half a pixel, and snapping the logical value would swallow them. Hit and picture differ
  by less than half a device pixel, the same order as a quad's own edge rounding.
- **Different while animating and after it stops.** No difference: always snap. Runtime has no smooth or inertial
  scrolling — inertia is just wheel deltas the OS keeps sending — so there is no moment at which it could decide
  "stopped". Always snapping also means no rebuild to return to the exact position when it stops.

The snap is done in logical px: `round(e × scale) / scale`. It is an f32 itself and carries its own rounding error:
multiplied back by scale, 125%, 150% and 200% are exactly integers, while at 110%, 120%, 175% and so on some
integers are one ulp off. Scrolled forty thousand pixels deep, that error is thousandths of a pixel. Every place
that rounds on the grid has to guard against it:

- Text x: the phase takes only the label's own position; the translation contributes whole pixels only.
  `whole_translation` changed from `floor` to `round`; otherwise, on the frames that are one ulp off, labels land
  one pixel left of their background.
- A quad's position, the centring rounding of icons and line boxes, clip rectangles, HostTexture's two scissors
  (the frame and its own rounded clip) and the bounds handed to CustomRender: the translated position is computed
  as `local × scale + whole pixels`, then brought back to logical px (`clip::on_grid`), leaving only one or two ulps
  of error from the result itself. Rounding adds a little slack (`grid_slack`, no smaller than the quad shader's
  0.001), and ties always go up, matching the quad's edge. A 16 px icon on an odd y at 150%, a clip edge landing
  exactly on a whole pixel at 120% — both are exact ties. Without these two steps the error picks their side frame
  by frame: the icon jumps a pixel against its background, and the clipped column comes and goes.

Two places still not on the same whole pixel:

- A quad running its own GPU-evaluated transform animation (`motion.wgsl`). Its offset is not snapped, while its
  text is presented on the CPU and does snap. During the animation the two differ by up to half a device pixel. Its
  descendant quads were already snapped.
- Native content regions (`native_content_regions`) are positioned by the unsnapped transform.

10k labels, `table-scroll-x` (Runtime's real horizontal scrolling, 0.37 px a frame): rebuilds per frame 10 001 → 0.
Resolve 84 030 → 0. Instance upload 3.0 MB → 0. batch p50 8.9 → 4.8 ms (not the same run; observation only). 1.2×,
1.75× and 2× are 0 as well. The presentation row uploads on the frames where the snapped whole pixel changes: at 1×
an average of 59 B a frame (160 B × about 37% of frames). None of the 620 gallery baselines changes: no frame's
final translation carries a fraction. Data in
[performance-data/text-scroll-snap-2026-09-23](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/text-scroll-snap-2026-09-23).

**An editor's horizontal scrolling is translation too.** A multi-line editor's scrolling is its own. Runtime used
to fold it straight into the value, caret and selection page coordinates, and the painter could not tell it apart
from scrolling, so a long line scrolling horizontally by a fraction re-resolved the whole paragraph every frame.
Geometry now carries `scroll: Option<TextInputScroll>` (multi-line editors only): `offset_x` is the scroll's
displacement on page x, and `text_x` is the exact x of the value without scrolling (`text.bounds.x - offset_x`
would be one ulp off, and the phase would change). Page-coordinate geometry is unchanged; hit testing, IME and
accessibility read it as before. Scene extraction (`EditorScroll`) draws the primitives that move with the text
along x as "the box minus `offset_x`, times one more translation", and the painter snaps as usual. Value, caret,
selection and the marks above them move by the same whole pixels. The gutter, current-line bar, minimap, sticky
rows and popovers do not. Snapping text alone is not enough: a caret at `round(c − s)` and a glyph at
`c − round(s)` would differ by a pixel on some frames.

Vertically it still lands in the box (the line box top is already aligned). A single-line input has no `scroll`
(scrolling only follows the caret). The logical offset keeps its fraction. Verification:
`a_fractional_horizontal_scroll_of_an_editor_keeps_every_glyph` (1×/1.1×/1.2×/1.75×, −0.37 px a frame, the content
area shifts as a whole pixel by pixel, zero rebuilds) and
`a_vertical_rl_editor_hands_its_column_scroll_to_the_painter`. What is left: the unscrolled x of the diagnostic
end-of-line text and atom chip labels is reconstructed, so they may re-resolve on some frames — as many as there
are on screen, not the whole paragraph.

**Multi-window counters are visible.** Rebuilds, upload bytes and entry counts are stored per render target, but
`text_glyph_counters()` used to read only the one being drawn. `paint_target` put the window's state away when it
finished, so nothing the second window did could be seen from outside. Now it sums over all live targets plus
closed ones, and a closing window hands each of its entries' atlas references back one by one, so the total does
not go backwards.

The full matrix for this wrap-up round (14 workloads × 1 / 100 / 1k / 10k × 60 / 120 / 240), the real-machine
reports for the four gates, and the before/after A/B are in
[performance-data/text-retained-2026-09-21](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/text-retained-2026-09-21).
All 14 A/B cells are between −11% and +1.7%. The structural changes are in the counts: whole-pixel scrolling of a
table with 10k labels uploads 479 KB → 96 B of run/presentation per frame; when two windows share a Device the
second rasterizes no glyph at all; when 1% of rows keep swapping in new hanzi and fill the atlas, the other 9 901
labels are still answered by their entry, with 0 stale handles.

### How to run it, how to judge it

```bash
cargo run --release --locked -p nana-ui --features gpu \
    --bin nana-text-paint-benchmark -- --output target/performance/issue98/text-paint.json

# Text gates for #8 (the three #98 classes + #223 subpixel translation + #99 constraint-only)
for id in gpu-scene-text-retained gpu-scene-text-paint-color \
          gpu-scene-text-compositor-opacity gpu-scene-text-compositor-transform \
          gpu-scene-text-compositor-slide gpu-scene-text-constraint-resize; do
  python3 perf/runners/nana/run.py --scenario "$id" --output "target/performance/issue98/$id.json"
done
python3 perf/contract.py --self-test
```

| Gate | What changes each frame | Criterion |
| --- | --- | --- |
| `gpu-scene-text-retained` | One of a thousand labels changes text | `text_nodes_shaped ≤ 1`, `text_layouts_created ≤ 1`, `paint_shape_cache_misses ≤ 0`, `text_instance_rebuilds ≤ 1`, `glyph_rasterized ≤ 4`, `glyph_upload_bytes ≤ 4096`, `text_instance_upload_bytes ≤ 4096`, `text_prepare_nodes_skipped ≥ 900` |
| `gpu-scene-text-paint-color` | Every label changes its foreground color | Shaping, layout (both Runtime and painter sides), rasterization, atlas upload, instance rebuild and upload are all 0; `skipped ≥ 900`. Where the remaining CPU goes: [GPU doc §12](./gpu.md) |
| `gpu-scene-text-compositor-opacity` | The container fades in and out | Same as above |
| `gpu-scene-text-compositor-transform` | The container rotates between −1.5° / 0° / +1.5° | Same as above, except instance upload (every third frame passes through identity) |
| `gpu-scene-text-compositor-slide` (#223) | The container moves 0.37 px sideways per frame with a CSS translate, simulating a trackpad horizontal scroll | Same as paint-color (including instance upload 0, not exempt like transform) |
| `gpu-scene-text-constraint-resize` (#99) | Every label alternates between two widths | Shaping, layouts that need reshaping, painter-side layout, rasterization, atlas upload are all 0; `text_layout_lookups ≥ 900` proves the width actually reached the text. Instance rebuilds have no gate |

The scene must actually move: either `text_ticker` or `text_animation`. The extractor checks the
value echoed in the report. A scene that runs without moving does not count. If nothing moves, the
painter just reuses the previous frame's batch. Every counter is 0.
The gate never goes red. Every criterion can go red on its own (`retained_text_tests.py` generates a
value that "blows" each invariant, one at a time, to verify it).

The ticker's text is fixed-width (`tick 0007`). `tick 9 → tick 10` shifts the twenty-odd labels after
it on the same line by one digit width. Their subpixel phase really changed. That frame has to rebuild
them. That is the cost of reflow. Not of retention. But whether the sampling window happened to catch
it used to decide whether the gate went red (2.6 > 1 on real hardware).

### Pixel difference from #97

On the same machine. 17 of the gallery's 561 frames changed. All of them are **≤ 2/255 per channel,
at most 619 pixels**. And all of them are on text in an opacity or transform animation:

- Opacity is no longer folded into the color and then quantized to 8 bits. It multiplies the shader's
  alpha as f32. Text that faded in from 0 used to appear only once alpha crossed 1/255.
- The four corners for rotation / perspective are now computed in the vertex shader. Same formulas.
  The last bit of f32 may differ.

The other two are **behavior fixes**. Not noise:

- Text under rounded-rect / polygon clipping used to ignore `clip-path: polygon()` and `circle()`. The
  old affine shader hardcoded the polygon count to 0.
- Text that only translated but had rounded-rect clipping used to skip pixel alignment (it went through
  the affine path; the raster phase matched the position before the transform). Now it is a translation
  run. It aligns to whole pixels like the text beside it.

### Not done in this phase

| Item | Status |
| --- | --- |
| ~~Zero rebuild for horizontal subpixel movement (#223)~~ | Done: the painter snaps the final pure translation to device pixels, and the logical scroll offset keeps its fraction. See "Fractional translation snaps to device pixels" |
| ~~Horizontal scrolling in the editor still re-parses~~ | Done: scrolling along x is handed to the painter as a translation by `ComponentGeometry::TextInput::scroll`. See "Horizontal scrolling in the editor is a translation too" |
| ~~Churn with random lengths still reflows the whole arena (#224)~~ | Done: drawing goes through a paint-order index table, moving a block writes only that block, and a budget-triggered compaction rewrites only the index table. See "Arena and the paint-order index table" |
| Paint-order compaction is still O(total on-screen slots) (#224) | Instance bytes are already only the rebuilt paragraph's own block; compacting the index table still rewrites every interval (4 B/slot). With ten thousand labels under random churn it averages 65 KB/frame over 60 frames, 8 KB/frame with a thousand labels, and after a long run, once the gaps take effect, it falls to 25 KB. Acceptance constrains instances only; the index half holds in the amortized sense |
| Gaps are not reclaimed (#230) | After entering a layout that leaves gaps, every gap slot is a four-corner quad the vertex stage still runs (about 20% more index slots), and it can only go away at the next paint-order compaction; once the text stops growing, the UI may go a long time without compacting. A gap-free compaction should run after a few stable frames, but first an A/B has to prove those 20% of vertices are really measurable |
| Paragraphs that are off screen during arena compaction | Compaction keeps only the blocks drawn in that frame; blocks off screen lose their place and are rewritten whole when they scroll back. This is the design since #98, and `seeded_churn_keeps_every_draw_naming_its_own_glyphs` counts it into the "allowed bytes" |
| The run table never shrinks | One fixed run row per entry (48 B), slot numbers only grow, and the peak at ten thousand labels is 480 KB. Shrinking it means rewriting row numbers in instances; not worth the gain |
| Line-level clipping | Entries are independent of the viewport, so one very long unwrapped run now hands its whole instance to the scissor to clip. Paragraph-level clipping happens per ink in `prepare` |
| Per-node fixed cost | In a frame with ten thousand text nodes the painter's own per-node loop has nothing to do with text; quads / icons pay it too. See the items above |

## Sharpness: coverage correction, DirectWrite and ClearType

The feedback was "the text looks blurry". After reading the code, ordered by impact. The causes and
what was done:

### 1. Coverage is blended in linear space, with no gamma / contrast correction (main cause, all platforms)

The surface is an sRGB format. The blend unit works in linear light. The coverage the rasterizer gives
is geometric area. Used directly as linear alpha. A half-covered edge pixel of black text on white
displays as sRGB 0.74. Strokes get thinner and the edges become a ring of gray haze. Mature engines do
not do this:

| Engine | Approach |
| --- | --- |
| DirectWrite / Windows Terminal / Zed | Gamma-space blending; in the shader, enhanced contrast + alpha correction by foreground luminance, with parameters from `IDWriteRenderingParams` |
| Skia / Chrome | A gamma+contrast LUT pre-baked by text luminance, assuming the background is the inverse of the foreground |
| glyphon (cosmic-text ecosystem) | `ColorMode::Web`: blend in gamma space, imitating the browser |

Our target is linear, and the whole pipeline depends on that. We cannot switch to gamma-space
blending. (When a transparent window finally goes to the system compositor, pixels are stored as
`enc(color) × α`. See `window.md`. That is only the delivery format. Blending itself still happens in
linear light.) So `corrected_coverage` (`shader/text_atlas.wgsl`; the CPU form is in `text/gamma.rs`)
works in two steps: first compute the coverage that should be used in gamma space following
DirectWrite (enhanced contrast → alpha correction; ported from Windows Terminal. See
`third-party.md`). Then, under Skia's assumption (background = inverse of foreground), solve for the
coverage **that makes the linear blend land on the same color**. The black-on-white and white-on-black
endpoints are exact. In between it is continuous, with no jumps. Parameters: on Windows it reads the
system (which is what the ClearType tuner changes). Other platforms use DirectWrite's factory values,
gamma 1.8 / grayscale contrast 1.0 / ClearType contrast 0.5.

Effect (pinned by the `gamma.rs` unit tests): a half-covered black-on-white pixel 0.74 → 0.54.
White-on-black comes out slightly thinner than linear blending. Consistent with DirectWrite. Color
glyphs (emoji) are not corrected.

### 2. hinting: no change needed

The original plan was to switch to skrifa's "vertical only" light hinting. After checking, that is not
needed: swash 0.2.10 already uses skrifa internally. And it is fixed at
`Lcd + preserve_linear_metrics` (vertical only, keeping linear advances; CFF also goes through the
PostScript hinter). The only difference from Vello is `symmetric_rendering: true`. It only affects the
TrueType bytecode interpreter. The default NotoSansSC carries no hinting instructions. It goes through
the autohinter. Unaffected. The affected ClearType-tuned fonts such as Segoe UI / Microsoft YaHei are
already handled on Windows by DirectWrite in the next section. Vertical is aligned anyway: the
baseline rounds, the top edge of the line box is a whole pixel. The y subpixel bucket is always 0.

### 3. Windows: DirectWrite rasterization (`text/raster_dwrite.rs`)

`PlatformRasterizer` on Windows is `DWriteGlyphRasterizer`: outline glyphs go to DirectWrite. Stems
and vertical grid-fit match native system apps. The face is still the one the `nana-text` font layer
supplies. Bytes reach DirectWrite through `IDWriteInMemoryFontFileLoader`. Variation coordinates go
through `IDWriteFontResource::CreateFontFace`. Synthetic bold / oblique use
`DWRITE_FONT_SIMULATIONS`. The rendering mode comes from `GetRecommendedRenderingMode` but only
`NATURAL_SYMMETRIC` / `OUTLINE` are kept: GDI-compatible and aliased modes snap glyphs to whole pixels.
They cannot follow a quarter-pixel subpixel bucket.

Handed back to swash: color fonts (`IsColorFont`), pure bitmap fonts, sideways glyphs in vertical
layout (`ROTATE_CW`), `PIXEL_FONT`. And any step of DirectWrite failing. The face table is still owned
by the swash side. The `GlyphFontId`s the two backends emit do not drift apart.

### 4. ClearType subpixel rendering

- `GlyphRenderMode::{SubpixelRgb, SubpixelBgr}` → `GlyphImageFormat::SubpixelRgb`: three subpixel
  coverages per pixel + the maximum value. **Stored on the color page** rather than opening another
  page kind: the color page is `Rgba8UnormSrgb`. So sRGB-encode before writing. Sampling decodes back
  to the original values in hardware. Instances mark it with the `INSTANCE_SUBPIXEL` bit.
- The shader splits into a shared `shade()` and two entry points: single output (the original) and
  dual-source blending (`enable dual_source_blending`; `src0 * src1 + dst * (1 - src1)` per channel).
  Grayscale and color glyphs give all four channels the same alpha in the dual-source pipeline. That
  is exactly pass-through alpha blending. So all text in a frame goes through one pipeline.
- When subpixel is used (everything else stays grayscale; consistent with Chrome / WinUI): the system
  has ClearType on and the panel reports an RGB/BGR order (`SubpixelOrder::system()`, read once per
  process), the device has `DUAL_SOURCE_BLENDING` (the host always requests it when the adapter
  supports it), the window surface's alpha mode is `Opaque` (transparent material windows such as
  Mica / Acrylic stay grayscale; otherwise DWM compositing produces color fringes), the run is a pure
  translation, and it is not inside an offscreen group (`group_depth == 0`).
- The mode is part of an entry's validity test (`TextGpuEntry::mode`). Text that enters or leaves an
  opacity group re-resolves. Switching with `set_subpixel_text` drops retained batches.
- Who enables it: a hosted host decides every frame from the window surface's alpha mode and
  `SubpixelOrder::system()`. A host that owns its surface and embeds `SceneWgpuPainter` directly (with
  only the `gpu` feature) calls the same public `SceneWgpuPainter::set_subpixel_text`. Offscreen
  captures (devtools) do not enable it. They stay grayscale.

### Verification

- Pixels: the component-gallery snapshots (`metal-apple-m4`) first recorded a clean HEAD into a private
  baseline (byte-identical to the repository baseline, 614/614). Then the change ran against it: 552
  changed. 62 unchanged (all without text). Re-recorded from that.
- The dual-source pipeline really runs on Metal: `subpixel_text_gives_each_channel_its_own_coverage`,
  `a_bgr_panel_gets_the_rgb_coverage_mirrored` (strict per-pixel mirror),
  `subpixel_text_inside_an_offscreen_group_stays_grayscale`,
  `grayscale_text_paints_equal_channels`.
- The DirectWrite backend: `cargo xwin check/clippy --target x86_64-pc-windows-msvc` passes. The
  `raster_dwrite.rs` unit tests (grayscale single channel, ClearType three channels with BGR mirroring,
  subpixel offset taking effect, system parameter ranges) only compile on Windows. CI's
  `windows-latest` runs them.
- **Nobody has looked at the final appearance on Windows with human eyes**: the machine here is macOS.

### Not done

| Item | Status |
| --- | --- |
| Refresh after a system setting changes | The ClearType switch / panel order / gamma are read once per process (when the painter is created); after changing the ClearType tuner the app must restart |
| Per-monitor parameters | Uses the primary monitor's `CreateRenderingParams`; does not call `CreateMonitorRenderingParams` for the monitor the window is on |
| ClearType on transparent windows | Intentionally not done; the reason is above |
| DirectWrite color glyphs | Go through swash; `TranslateColorGlyphRun` is not wired |
| Blur under shrinking transforms | scale < 1 (press animation) still bilinear-samples the device-scale bitmap; not re-rasterized at the scale |
| Tuning on macOS | Uses DirectWrite factory values; never compared item by item against CoreText |

## #33 migration benchmark

The 2k / 4k / 8k cells of `nana-dirty-frame-benchmark --shape layout --position head` are Issue #33's
original workload. #89 keeps it as the text migration benchmark.

It had **never been compiled by any CI job**: `required-features = ["benchmark"]` kept it out of
`cargo test/check --workspace --all-targets`. `runtime-performance.yml` did not name it either.
Deleting it left every workflow green. There are now two lightweight guards:

- The `ci.yml` check job runs `cargo check -p nana-ui-scene --all-targets --features benchmark`. The
  file can no longer rot silently or be deleted.
- `the_migration_grid_still_spans_two_four_and_eight_thousand_nodes` asserts the default row grid still
  produces the node scales 2002 / 4002 / 8002. The grid can no longer be narrowed silently.

Reproduce (only a before/after on the same machine means anything):

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 1000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 2000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 4000 --samples 150 --warmup 30
```

The baseline after Issue #33 converged (Windows. 2026-09-13. See [Dirty frames](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/runtime-dirty-frame.md)):

| Nodes | TextShape | Layout |
| ---: | ---: | ---: |
| 2,002 | 0.176 ms | 3.68 ms |
| 4,002 | 0.557 ms | 11.47 ms |
| 8,002 | 1.811 ms | 31.75 ms |

Re-measured after Phase 0 landed (Windows. 2026-09-16. A different machine. p50. All three cells have
`text_shaped == 0`, `cache_lookups == 0`, and `skipped_unchanged` equal to the number of non-empty text
nodes):

| Nodes | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.117 ms | 3.11 ms | 3.8% |
| 4,002 | 0.593 ms | 8.55 ms | 6.9% |
| 8,002 | 0.814 ms | 18.55 ms | 4.4% |

Re-measured after Phase 2 landed (the same Windows machine. 2026-09-16. p50. All three cells still
`text_shaped == 0`. The product path does not use the shaper. What this row confirms is that there is
no regression):

| Nodes | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.190 ms | 4.42 ms | 4.3% |
| 4,002 | 0.423 ms | 9.70 ms | 4.4% |
| 8,002 | 0.835 ms | 20.05 ms | 4.2% |

Re-measured after Phase 3 landed (Linux container. 4 vCPU Xeon @ 2.80 GHz. 2026-09-16. p50. All three
cells still `text_shaped == 0` / `cache_lookups == 0`):

| Nodes | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.242 ms | 4.421 ms | 5.5% |
| 4,002 | 0.743 ms | 13.064 ms | 5.7% |
| 8,002 | 1.609 ms | 28.520 ms | 5.6% |

This round changed machines (the first three rounds were all on the same Windows box). So look at
ratios **within this round only**. Do not compare absolute values with the Windows rounds.
`nana-ui-scene`'s dependency graph does not contain `nana-text` (`cargo tree -p nana-ui-scene --edges
normal` counts 0). The benchmark binary therefore cannot change because of this phase's edits. These
three rows are the new baseline on this machine. Only a re-measurement on the same machine in the next
phase is comparable.

Phase 4 before/after on the same machine (Apple M4. 10 cores. macOS. 2026-09-17. p50. 150 samples / 30
warmup). This round changed machines. Compare within this round only:

| Nodes | Before TextShape | Before Layout | Before ratio | After TextShape | After Layout | After ratio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2,002 | 0.038 ms | 1.118 ms | 3.4% | 0.017 ms | 1.083 ms | 1.5% |
| 4,002 | 0.119 ms | 3.055 ms | 3.9% | 0.060 ms | 2.590 ms | 2.3% |
| 8,002 | 0.300 ms | 6.610 ms | 4.5% | 0.126 ms | 6.136 ms | 2.1% |

In the three "after" cells `text_work` has `text_nodes_considered == text_nodes_revision_skipped` every
frame (1000 / 2000 / 4000). `text_nodes_shaped`, `text_source_clones`, `text_bytes_hashed`,
`shape_cache_lookup`, `layout_cache_lookup`, `layouts_created` are all 0. With `--engine nana-text`
(every label retains a real `TextLayout`) the three cells are 0.016 / 0.062 / 0.131 ms. Same counters.

The remaining growth is the candidate scan itself: one side-table lookup per candidate. As nodes double
it goes slightly superlinear as the working set leaves cache, like Layout. But the ratio against Layout
is now essentially flat (2.3% / 2.1%). The extra part before the change came from reading the whole
`NodeRecord`, looking up the visual, dereferencing the computed style, and computing constraints for
every candidate before it could decide to skip.

Phase 5 before/after on the same machine (the same Apple M4. macOS. 2026-09-17. p50. 150 samples / 30
warmup). This workload has no editor. Phase 5 is not expected to change it. "Before" is the same tree
built before the change:

| Nodes | Before TextShape | Before Layout | Before ratio | After TextShape | After Layout | After ratio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2,002 | 0.050 ms | 2.891 ms | 1.7% | 0.040 ms | 1.529 ms | 2.6% |
| 4,002 | 0.078 ms | 4.951 ms | 1.6% | 0.074 ms | 4.609 ms | 1.6% |
| 8,002 | 0.162 ms | 7.942 ms | 2.0% | 0.140 ms | 7.180 ms | 2.0% |

TextShape absolute values are flat. The ratio change in the 2k cell comes from Layout's own jitter on
this machine (the same build run twice gives Layout between 1.25–1.53 ms). Not text work. The three
cells' `text_work` still skips every candidate on revision, `layouts_created == 0`.

Re-measured after the wrap-up round of editor performance changes for #96 (the same Apple M4.
2026-09-18. p50. 150 samples / 30 warmup). This cell has no editor. The change only goes through the
shared shape path (`CountingShaper` asks `retains_measurement` once more). Confirmed no regression:

| Nodes | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.014 ms | 0.963 ms | 1.4% |
| 4,002 | 0.037 ms | 2.155 ms | 1.7% |
| 8,002 | 0.122 ms | 5.595 ms | 2.2% |

(Layout's absolute values this round are much lower than last round. Load differs at different times
on the same machine. Look at ratios only.)

Rule: **every time `nana-text` lands a phase, re-run these three cells, paste the numbers back into
this table, and say which machine it was. The `TextShape` ratio against `Layout` in the same round must
not get worse.** This is not a timing gate. It is a manual comparison. Wiring it into the `perf/`
contract needs a new scenario `kind`, extractor, and fixture. Do that once there is an engine to
measure.

## Large-document editing benchmark (#96)

`nana-text-edit-benchmark`: one focused `TextArea`. It sweeps line counts (~38 B per line). Each cell
measures four timings of **the same interaction**. It is there to answer the question #96 deferred to
real numbers: should storage switch to a rope?

- `input_ms`. The event itself (`replace_focused_text` / `move_focused_text_caret` / pointer down).
  Probes outside the batch and value clones are both in here.
- `flush_ms`. The frame right after (`RuntimeDocument::flush`). Split by `FrameStage`.
- `storage_ms`. The same edit applied to a bare `EditableText`: storage only. No layout, no
  presentation, no frame. **This is the number a rope could improve**.
- `value_clone_ms`. One whole-value `String::clone`. The unit for "one O(document) pass".

Typing runs as insert/backspace pairs (only half is recorded). Otherwise a long run of samples would
keep growing the line being typed on. Later samples would then measure a document other than the one
the cell claims.

Reproduce:

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-text-edit-benchmark
./target/release/nana-text-edit-benchmark --action type  --position head --samples 80 --warmup 20
./target/release/nana-text-edit-benchmark --action caret --position head --samples 80 --warmup 20
```

First round (Apple M4. macOS. 2026-09-18. release. p50. 80 samples / 20 warmup. `--position head`):

| Lines | Bytes | type input | type flush | of which TextShape | storage | whole-value clone |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 9.4 KB | 0.008 ms | 0.035 ms | 0.026 ms | 0.0002 ms | 0.0002 ms |
| 1,000 | 37.9 KB | 0.021 ms | 0.105 ms | 0.093 ms | 0.0006 ms | 0.0005 ms |
| 4,000 | 154.9 KB | 0.081 ms | 0.388 ms | 0.363 ms | 0.0025 ms | 0.0025 ms |
| 8,000 | 310.9 KB | 0.166 ms | 0.762 ms | 0.714 ms | 0.0050 ms | 0.0055 ms |

Pure caret movement (no text change. `--action caret`), same machine, same round: flush for 250 /
1,000 / 4,000 / 8,000 lines is 0.013 / 0.035 / 0.113 / 0.222 ms. `layouts_created == 0`,
`editable_mutations == 0`.

**Conclusion: keep the storage.** Typing one character in a 310 KB document costs ~0.93 ms. The
`String` memmove in it is 0.005 ms (0.5%). Same order as one whole-value clone. Switching to a rope
would save at most that 0.5%. Meanwhile the same key press carries ~180× as much O(document) work. The
real gap is that work. The `--action caret` row says it more clearly: **a caret move that changes no
text still costs 0.22 ms in a 310 KB document**. And the work it owes is zero.

The per-frame O(document) items the profile (`/usr/bin/sample`. 8,000 lines) located, ordered by size:

| Item | Magnitude at 8,000 lines | How it was fixed |
| --- | ---: | --- |
| `metrics_of_geometry` accumulates every paragraph | ~0.05 ms / frame | The host caches this measurement per editor, and `GeometrySync::unchanged` decides whether it can stay — `paragraphs_laid_out == 0` does not mean the geometry did not move (deleting the first paragraph's newline merges a paragraph and lays out none) |
| `bracket_pair_colorization` whole-document stack scan | ~0.37 ms / edit frame | Coloring is only a function of the bracket character sequence: when there is no bracket character before or after the changed range, only offsets shift, and the cached text updates in place with the same splice; only a real bracket keystroke triggers a rescan. `text_shape_stats::bracket_rescans` pins this (the minimap line-length table still rescans the whole document and is off by default) |
| Whole-text SipHash for `TextLayoutKey` | ~0.19 ms × 4 shapes per frame | `TextShaper::retains_measurement(id)`: when the host retains that node's measurement itself (the editor's paragraph geometry), the content-addressed Runtime layout cache is pure cost for it, so it is skipped |
| Byte-by-byte prefix/suffix diff scan | ~0.14 ms / edit + 0.06 ms / frame | `changed_byte_range` first does `==` (equality is the common case), then memcmp in 64-byte blocks; the byte-wise zip does not compile to a vectorized compare, and an edit at the head of the document is the worst case for the suffix scan |
| Building a whole presentation for two booleans | ~0.09 ms / key press | The framework asks `text_shape_constraints` on every key press, but it first had to build `TextInputPresentationSource`; `text_input_kind` is now derived from the node directly |
| Resending the whole value for a caret-only move | 2 whole-value clones per frame + content dirtying | Component sync now sends `SetTextSelection` instead when neither the value nor the extra cursor changed |
| Rebuilding the paragraph table in `EditorGeometry::sync` | ~0.09 ms / edit frame | Retained paragraphs stay in place; only the paragraphs between prefix and suffix are spliced out, and the suffix's `start` gets the length delta |
| Cloning the pre-edit value even without a snippet session | 1 whole-value clone / key press | Only snippet sessions need it |

Re-measured on the same machine and round (Apple M4. macOS. 2026-09-18. release. p50. 80 samples / 20
warmup. "After" is a re-run once this round's changes had all landed and the benchmark's own sampling
defect was fixed):

| Lines | Bytes | type input | type flush | caret input | caret flush |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 9.4 KB | 0.008 → 0.003 ms | 0.035 → 0.013 ms | 0.002 → 0.009 ms | 0.013 → 0.010 ms |
| 1,000 | 37.9 KB | 0.021 → 0.009 ms | 0.105 → 0.031 ms | 0.006 → 0.013 ms | 0.035 → 0.023 ms |
| 4,000 | 154.9 KB | 0.081 → 0.027 ms | 0.388 → 0.086 ms | 0.019 → 0.030 ms | 0.113 → 0.067 ms |
| 8,000 | 310.9 KB | 0.166 → 0.051 ms | 0.762 → 0.168 ms | 0.036 → 0.051 ms | 0.222 → 0.126 ms |

One key press in a 310 KB document drops from 0.93 ms to 0.22 ms. `caret input` grew because of
**visual-order left/right movement**, landed in the same round: the arrow keys now ask one geometry
probe (see "Visual-order left and right arrows"). What it buys is a caret that no longer jumps at BiDi
and wrap boundaries.

Re-measured on the same machine after #182 (Apple M4. macOS 27. 2026-09-24. release. before =
`739defab4`. after = all #182 commits. The two binaries ran **alternately** for 8 rounds, swapping
order each round. Each cell takes the minimum of the 8 p50s. 60 samples / 15 warmup. Load 4–6):

| Cell | Lines | input before → after | flush before → after |
| --- | ---: | ---: | ---: |
| caret head | 1,000 | 0.0132 → 0.0067 ms | 0.0198 → 0.0041 ms |
| caret head | 8,000 | 0.0566 → 0.0073 ms | 0.1192 → 0.0081 ms |
| caret tail | 8,000 | 0.0538 → 0.0047 ms | 0.1180 → 0.0080 ms |
| click head | 8,000 | 0.0713 → 0.0031 ms | 0.1177 → 0.0079 ms |
| vertical head | 8,000 | 0.1198 → 0.0054 ms | 0.1248 → 0.0083 ms |
| type head | 1,000 | 0.0083 → 0.0034 ms | 0.0283 → 0.0138 ms |
| type head | 8,000 | 0.0528 → 0.0166 ms | 0.1626 → 0.0760 ms |
| type tail | 8,000 | 0.0490 → 0.0155 ms | 0.1514 → 0.0644 ms |

Caret moves, clicks, and up/down no longer grow with the document: 1,000 lines and 8,000 lines are the
same order of magnitude (geometry is decided by stamp, display text is reused by identity, a11y /
extraction do not copy). The part of typing that still grows with the document is the component writing
out the new value, the diff when the session adopts it, and the prefix/suffix comparison of paragraph
geometry. All are the one pass the edit itself needs.

The per-frame O(document) items that remained at the time. After #182:

| Item | Magnitude at 8,000 lines | Now |
| --- | ---: | --- |
| Whole-value clones in a11y / scene primitive / extraction | 3 per frame | **Gone**: all three are `TextValue` reference counts; the extraction layer no longer deep-copies `text_input` / `ime` (only the `editable` flag remains) |
| Presentation rebuilds display text every frame | 1 whole-value clone + one memcmp | **Gone**: display text is the session snapshot (when there is no overlay) or a fold / composition view memoized by stamp; presentation equality compares by identity |
| The undo log snapshots whole values | 2 whole values per step | Snapshots share buffers; the before and after states no longer each copy once. The representation is still the whole state. The byte budget cap (8 MiB) is unchanged; accounting now deduplicates by buffer |

## What Phase 0 explicitly did not do

| Item | Status | Detail |
| --- | --- | --- |
| Fractional DPI | Covers layout, **does not cover rasterization** | `TextScale` expresses a font-size scale, which is all layout can express. Physical pixel alignment of glyph origins and subpixel bucketing live in `scene_paint/text/`, entirely outside the IR. Do not read `TX-D01` as a subpixel positioning guarantee. |
| ellipsis | Records overflow, **does not insert an ellipsis glyph** | cosmic 0.19's `Buffer` has no ellipsis; the product path substitutes it itself. `TX-W05` asserts `TRUNCATED_LINES` + `ELLIPSIZED` and the truncated line count. Phase 3's native engine really shapes and places `…`. See "Layout". |
| IME preedit | Span application is real; composition state lives on the source | `TextLayout` carries geometry only; `CompositionSegment` stays in `TextSource` / `TextSpan`. `TX-E01` asserts the preedit span really does produce its own run, and that composition can be set and cleared on the source. |
| Caret inside a cluster | Interpolated by byte fraction | One glyph can cover several source bytes (a ligature, or a multi-byte character). `caret_geometry` first merges every cell that renders the same cluster into one visual range — combining marks are zero-advance and share the cluster with their base, and under RTL HarfBuzz even places them **before** the base — then interpolates by byte fraction inside that range. So the caret in `of\|fice` lands a third of the way into the `ffi` ligature, not after the whole ligature, and an Arabic cluster with marks does not collapse onto a zero-width mark (see the eight caret probes in `TX-B01`, where x strictly decreases with byte offset). A byte offset inside a grapheme is not a legal caret position to begin with, the IR has no source text to snap to, and interpolation only guarantees monotonic and distinguishable. |
| caret affinity (RTL / BiDi boundaries) | **Records behavior, not a contract** | Boundary affinity is engine-defined, not spec-defined. Phase 0 records the reference engine's answer as a golden with a `caret_x_px` tolerance. |
| Five counters | Only the reference path feeds them | By design there is no product producer; reconciliation tests keep them from idling. |
| script tagging | `ScriptTag::UNKNOWN` in the reference engine | The reference engine does not export per-run script. Phase 2's shaper fills it (see "Shaping"). |
| Vertical layout (#59) | Not done in Phase 0, and fail-closed | Phase 3's layout lays `vertical-*` out horizontally and sets `unsupported_writing_mode`. After #59 everything lays out in columns (the editor too). See "writing mode and #59". |
| Multi-font fallback | Covered in the corpus, but narrowly | The corpus fallback is VF→Noto's `A`/`B`, proving `FontId` can change mid-run and `FALLBACK_FONT` gets set. Candidate selection driven by script / language / emoji comes from the Phase 1 font layer (see "Font layer"); the reference engine does not use it. |
| #33 workload | Kept at contract level, not a perf gate | See the previous section. |

## How to run

```bash
cargo test -p nana-text --all-targets --locked
python3 scripts/check-engine-boundary.py
python3 -m unittest discover -s scripts/tests
python3 scripts/build-text-corpus-fonts.py --check   # requires fonttools
# Platform system-font acceptance (reads local fonts, not in CI)
cargo test -p nana-text --release --test text_platform font_system_platform_acceptance -- --ignored --nocapture
```

Prove it has not one edge (not even a dev edge):

```bash
grep -c '^name = "cosmic-text"$' Cargo.lock   # 0
```

Since Phase 4 `nana-ui-runtime` depends on `nana-text` (retained text nodes and handles). Since #99
both measurement and drawing in `nana-ui` go through it. The same assertion for the whole workspace:

```bash
cargo tree --workspace --locked --edges normal | grep -ci cosmic   # 0
```

### Authored rich paragraph typography

`RichText` retains optional alignment per LF paragraph, including empty and trailing paragraphs. `SetParagraphAlignment` affects the current paragraph or all paragraphs touched by an exclusive byte selection. Splitting inherits the original paragraph, and merging retains the first. `ClearFormatting` clears selected paragraph alignment; `ClearAttrs` clears only named character attributes. All changes use the editor snapshot undo history.

`RichShapeStyle::size_em` resolves against the node's inherited size and is mutually exclusive with `size_px`. `family_alias` is an opaque authored alias retained alongside the resolved `family`; the consumer resolves and refreshes it. Paragraph alignment reaches `TextSource`, the layout cache key and the shared line origins, so drawing, caret, hit testing and selection use the same geometry.
