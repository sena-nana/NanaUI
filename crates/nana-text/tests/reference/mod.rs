//! The cosmic-text reference engine. Temporary, and test-only by construction.
//!
//! This is the only place in the crate that names `cosmic_text`, and it lives
//! under `tests/` so the dependency is a dev edge that no product crate can
//! reach. When a native engine lands, this directory and the `cosmic-text`
//! dev-dependency are deleted together; nothing under `src/` changes.
//!
//! It exists to record goldens, not to be fast or complete. Where cosmic has no
//! equivalent for something the IR expresses, the translation says so in a
//! comment rather than inventing a number.

#![allow(dead_code)]

pub mod font_set;

use cosmic_text::{
    Attrs, Buffer, Family, FontFeatures, FontSystem, FontVariations, Metrics, Shaping, Stretch,
    Style, VariationTag, Weight, Wrap, fontdb,
};
use nana_text::parity::CorpusCase;
use nana_text::{
    FontGeneration, FontId, GlyphFlags, LineBox, LineBreakCause, LineMetrics, OverflowFlags,
    RunDirection, RunMetrics, ScriptTag, ShapeRunId, ShapedGlyph, ShapedRun, TextConstraints,
    TextLayout, TextLayoutId, TextRect, TextSource, TextStyle, TextWorkCounters,
};
use nana_ui_core::{DirSpec, FontKerningSpec, FontVariationSetting, TextWrapBreak};

/// Unicode directional isolates. Both LRI and RLI are three bytes in UTF-8.
const LRI: char = '\u{2066}';
const RLI: char = '\u{2067}';
const PDI: char = '\u{2069}';

/// How much the isolate inflates the embedding levels cosmic reports.
///
/// cosmic always treats the paragraph as level 0, so RLI establishes level 1 --
/// exactly the paragraph level an RTL paragraph should have -- while LRI
/// establishes level 2 where an LTR paragraph should be at 0. Without this the
/// isolate would leak into `ShapedRun::bidi_level`, and plain Latin text in an
/// LTR paragraph would claim level 2.
const fn level_bias(base: DirSpec) -> u8 {
    match base {
        DirSpec::Ltr => 2,
        DirSpec::Rtl => 0,
    }
}

/// Where one buffer line's text sits in the source, and how many bytes of
/// isolate were prepended to it in the shaped string.
pub struct SourceLine {
    /// Byte offset of this line's text within the source.
    source_start: usize,
    /// Length of this line's text in the source, isolate excluded.
    source_len: usize,
    /// Byte offset of this buffer line within the shaped string.
    shaped_start: usize,
    /// Isolate bytes prepended to this line in the shaped text.
    prefix: usize,
}

impl SourceLine {
    /// Maps a byte offset within the shaped buffer line back to the source, or
    /// `None` when it falls on the isolate rather than on the text.
    fn to_source(&self, local: usize) -> Option<usize> {
        let offset = local.checked_sub(self.prefix)?;
        (offset <= self.source_len).then_some(self.source_start + offset)
    }

    /// Maps a *caret* offset within the shaped buffer line back to the source,
    /// clamping either isolate onto the text it wraps.
    ///
    /// Unlike [`Self::to_source`], which has to reject isolate positions so the
    /// glyphs belonging to them are dropped, a caret inside an isolate is a
    /// real position: a click past the right end of the line lands after the
    /// trailing PDI, and it means the end of the text, not "nowhere".
    fn clamp_to_source(&self, local: usize) -> usize {
        let offset = local.saturating_sub(self.prefix).min(self.source_len);
        self.source_start + offset
    }

    /// Whether a source byte lies on this line. The end boundary is inclusive
    /// so a span or range ending at the line's last byte still resolves here.
    fn contains_source(&self, byte: usize) -> bool {
        byte >= self.source_start && byte <= self.source_start + self.source_len
    }

    /// Maps a source byte on this line to its offset in the shaped string.
    fn to_shaped(&self, byte: usize) -> usize {
        self.shaped_start + self.prefix + (byte - self.source_start)
    }
}

/// Builds the string handed to cosmic, isolating each line at the declared base
/// direction, plus the map back to source offsets.
///
/// Both directions are isolated, not just RTL. cosmic has no paragraph
/// direction switch and otherwise falls back to first-strong detection, which
/// would quietly lay a `direction: ltr` paragraph out right-to-left whenever its
/// text happened to begin with RTL script. CSS fixes the paragraph embedding
/// level from the declared direction, so the isolate has to be symmetric.
fn shaped_lines(text: &str, base: DirSpec) -> (String, Vec<SourceLine>) {
    let isolate = match base {
        DirSpec::Ltr => LRI,
        DirSpec::Rtl => RLI,
    };
    let mut shaped = String::new();
    let mut lines = Vec::new();
    let mut source_start = 0usize;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            shaped.push('\n');
            source_start += '\n'.len_utf8();
        }
        let shaped_start = shaped.len();
        shaped.push(isolate);
        shaped.push_str(line);
        shaped.push(PDI);
        lines.push(SourceLine {
            source_start,
            source_len: line.len(),
            shaped_start,
            prefix: isolate.len_utf8(),
        });
        source_start += line.len();
    }
    (shaped, lines)
}

/// One shaped corpus case, plus the cosmic buffer it came from so a test can
/// cross-check nana-text's own derivations against the reference's.
pub struct ReferenceRun {
    pub layout: TextLayout,
    pub counters: TextWorkCounters,
    pub buffer: Buffer,
    pub font_system: FontSystem,
    source_lines: Vec<SourceLine>,
}

impl ReferenceRun {
    /// Maps a cosmic `Cursor` back to a source byte.
    ///
    /// cosmic reports `index` relative to its own buffer line, which carries
    /// the directional isolate this adapter added, so a caller comparing
    /// against the IR has to come back through the same map. A cursor sitting
    /// on an isolate clamps onto the text beside it; `None` means only that the
    /// cursor names a line this run does not have.
    pub fn source_byte(&self, cursor: cosmic_text::Cursor) -> Option<usize> {
        self.source_lines
            .get(cursor.line)
            .map(|line| line.clamp_to_source(cursor.index))
    }
}

/// Shapes and lays out one corpus case against a hermetic font database.
pub fn run_case(case: &CorpusCase) -> ReferenceRun {
    let (mut font_system, faces) = font_set::hermetic_font_system(&case.fonts);
    let mut counters = TextWorkCounters::default();

    let scale = case.constraints.scale.px_per_logical;
    let font_size = case.style.font_size_px * scale;
    let line_height = case
        .style
        .line_height_px()
        .map(|px| px * scale)
        .unwrap_or(font_size * 1.2);

    let mut buffer = Buffer::new(&mut font_system, Metrics::new(font_size, line_height));
    buffer.set_wrap(wrap_of(&case.constraints));
    buffer.set_tab_width(u16::from(case.constraints.tab_width));
    buffer.set_size(
        case.constraints.max_width_px.map(|px| px * scale),
        case.constraints.max_height_px.map(|px| px * scale),
    );

    // cosmic always splits on line endings. `preserve_lines: false` is the
    // caller saying an authored newline is not a line break, so it is folded to
    // a space before shaping rather than after.
    let text = if case.constraints.preserve_lines {
        case.text.clone()
    } else {
        case.text.replace('\n', " ")
    };

    // The declared direction is authoritative, so each buffer line is isolated
    // at it. cosmic resolves BiDi per buffer line, which is why the isolate
    // goes around *each* line rather than the whole string.
    let (shaped_text, source_lines) = shaped_lines(&text, case.constraints.base_direction);

    // The family *name* of the first declared fixture. Its fixture id is not a
    // family: querying by it resolves to no face, which silently marked every
    // glyph of a case without `font_family` as not-fallback.
    let default_family = case.fonts.first().and_then(|id| font_set::font_family(id));
    let attrs = attrs_for(&case.style, scale, default_family);
    if case.spans.is_empty() {
        buffer.set_text(&shaped_text, &attrs, Shaping::Advanced, None);
    } else {
        let spans = rich_spans(case, &shaped_text, &source_lines, scale, default_family);
        buffer.set_rich_text(
            spans
                .iter()
                .map(|(text, attrs)| (text.as_str(), attrs.clone())),
            &attrs,
            Shaping::Advanced,
            None,
        );
    }
    buffer.shape_until_scroll(&mut font_system, false);

    // The face the case asked for. Anything else in the output is fallback.
    let requested = requested_face(&font_system, &attrs);
    let source = TextSource::new(text.clone());
    let layout = to_layout(
        case,
        &source_lines,
        &buffer,
        &mut font_system,
        &faces,
        requested,
        &source,
    );

    counters.record_text_pass(1, 1);
    counters.record_glyphs_resolved(layout.glyph_count());

    ReferenceRun {
        layout,
        counters,
        buffer,
        font_system,
        source_lines,
    }
}

/// Slices the shaped text into `(text, attrs)` spans.
///
/// Byte ranges in a corpus case address the *source*, so they are shifted by
/// the isolate prefix before being used against the shaped string.
fn rich_spans<'a>(
    case: &CorpusCase,
    shaped_text: &str,
    source_lines: &[SourceLine],
    scale: f32,
    default_family: Option<&'a str>,
) -> Vec<(String, Attrs<'a>)> {
    // A span's range is in source bytes, and every buffer line carries its own
    // isolate, so each end has to be mapped through the line it falls on.
    // Shifting everything by the first line's prefix would land a span on line
    // 1 or later inside an isolate -- not a char boundary -- and panic.
    let shaped_at = |byte: usize| {
        source_lines
            .iter()
            .find(|line| line.contains_source(byte))
            .map_or(byte, |line| line.to_shaped(byte))
    };
    let base = attrs_for(&case.style, scale, default_family);
    let mut spans: Vec<(String, Attrs<'a>)> = Vec::new();
    let mut cursor = 0usize;
    let mut ordered = case.spans.clone();
    ordered.sort_by_key(|span| span.range.start);
    for span in &ordered {
        let start = shaped_at(span.range.start);
        let end = shaped_at(span.range.end);
        if start > cursor {
            spans.push((shaped_text[cursor..start].to_string(), base.clone()));
        }
        spans.push((
            shaped_text[start..end].to_string(),
            attrs_for(&span.style, scale, default_family),
        ));
        cursor = end;
    }
    if cursor < shaped_text.len() {
        spans.push((shaped_text[cursor..].to_string(), base.clone()));
    }
    spans
}

fn wrap_of(constraints: &TextConstraints) -> Wrap {
    match constraints.wrap {
        None => Wrap::None,
        Some(TextWrapBreak::Word) => Wrap::Word,
        Some(TextWrapBreak::WordOrGlyph) => Wrap::WordOrGlyph,
        Some(TextWrapBreak::Glyph) => Wrap::Glyph,
    }
}

fn attrs_for<'a>(style: &TextStyle, scale: f32, default_family: Option<&'a str>) -> Attrs<'a> {
    let family = match style.font_family.as_deref() {
        Some(name) => Family::Name(leak_family(name)),
        None => match default_family {
            Some(name) => Family::Name(name),
            None => Family::SansSerif,
        },
    };

    let mut features = FontFeatures::new();
    for feature in &style.features {
        features.set(cosmic_text::FeatureTag::new(&feature.tag), feature.value);
    }
    if matches!(style.kerning, FontKerningSpec::None) {
        features.disable(cosmic_text::FeatureTag::new(b"kern"));
    }

    let mut variations = FontVariations::new();
    for axis in &style.variations {
        variations.set(VariationTag::new(&axis.tag), axis.value);
    }

    // Per-span metrics, not just the buffer's: without these a span that only
    // differs in size would silently shape at the base size and the fixture
    // would assert nothing.
    let size = style.font_size_px * scale;
    let line_height = style
        .line_height_px()
        .map(|px| px * scale)
        .unwrap_or(size * 1.2);

    let mut attrs = Attrs::new()
        .family(family)
        .weight(Weight(style.font_weight))
        .stretch(stretch_for(&style.variations))
        .metrics(Metrics::new(size, line_height))
        .font_features(features)
        .font_variations(variations);
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    if style.letter_spacing_px != 0.0 {
        // cosmic tracks letter spacing in EM; the IR carries px.
        let em = style.letter_spacing_px * scale / size;
        attrs = attrs.letter_spacing(em);
    }
    attrs
}

/// `wdth` maps onto `font-stretch`, mirroring what the product shaper does.
fn stretch_for(variations: &[FontVariationSetting]) -> Stretch {
    match FontVariationSetting::wdth_value(variations) {
        None => Stretch::Normal,
        Some(value) if value <= 56.25 => Stretch::UltraCondensed,
        Some(value) if value <= 68.75 => Stretch::ExtraCondensed,
        Some(value) if value <= 81.25 => Stretch::Condensed,
        Some(value) if value <= 93.75 => Stretch::SemiCondensed,
        Some(value) if value < 106.25 => Stretch::Normal,
        Some(value) if value < 118.75 => Stretch::SemiExpanded,
        Some(value) if value < 137.5 => Stretch::Expanded,
        Some(value) if value < 175.0 => Stretch::ExtraExpanded,
        Some(_) => Stretch::UltraExpanded,
    }
}

/// `Family::Name` borrows, and a corpus case owns its family string. The set of
/// distinct family names in a corpus run is tiny and bounded, so interning them
/// for the process is cheaper than threading a lifetime through the engine.
fn leak_family(name: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let names = NAMES.get_or_init(|| Mutex::new(HashSet::new()));
    let mut names = names.lock().expect("family intern set is not poisoned");
    if let Some(existing) = names.get(name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.insert(leaked);
    leaked
}

fn requested_face(font_system: &FontSystem, attrs: &Attrs) -> Option<fontdb::ID> {
    let family = attrs.family;
    font_system.db().query(&fontdb::Query {
        families: &[family],
        weight: attrs.weight,
        stretch: attrs.stretch,
        style: attrs.style,
    })
}

#[allow(clippy::too_many_arguments)]
fn to_layout(
    case: &CorpusCase,
    source_lines: &[SourceLine],
    buffer: &Buffer,
    font_system: &mut FontSystem,
    faces: &[fontdb::ID],
    requested: Option<fontdb::ID>,
    source: &TextSource,
) -> TextLayout {
    let bias = level_bias(case.constraints.base_direction);
    let mut runs: Vec<ShapedRun> = Vec::new();
    let mut lines: Vec<LineBox> = Vec::new();

    let visual: Vec<_> = buffer.layout_runs().collect();
    for (index, run) in visual.iter().enumerate() {
        let Some(source_line) = source_lines.get(run.line_i) else {
            continue;
        };
        let run_start = runs.len() as u32;

        // Split the visual glyph list wherever the face, the embedding level or
        // the size changes: that is exactly a shaped run.
        let mut group: Vec<&cosmic_text::LayoutGlyph> = Vec::new();
        let mut flush = |group: &mut Vec<&cosmic_text::LayoutGlyph>, runs: &mut Vec<ShapedRun>| {
            if group.is_empty() {
                return;
            }
            runs.push(shaped_run(
                group,
                source_line,
                bias,
                font_system,
                faces,
                requested,
                runs.len(),
            ));
            group.clear();
        };
        for glyph in run.glyphs {
            // Glyphs belonging to the RLI/PDI isolate are not part of the
            // source and must not appear in the IR.
            if source_line.to_source(glyph.start).is_none()
                || glyph.start >= source_line.prefix + source_line.source_len
            {
                continue;
            }
            let split = group.last().is_some_and(|last| {
                last.font_id != glyph.font_id
                    || last.level != glyph.level
                    || last.font_size != glyph.font_size
            });
            if split {
                flush(&mut group, &mut runs);
            }
            group.push(glyph);
        }
        flush(&mut group, &mut runs);

        let run_end = runs.len() as u32;

        // `LineBox::runs` is a **visual**-order range and `TextLayout::cells`
        // depends on it, but cosmic's glyph vector is not guaranteed to run
        // left to right across a mixed-direction line. Sort what this line
        // produced, then renumber so a run's id still matches its position.
        let line_runs = &mut runs[run_start as usize..run_end as usize];
        line_runs.sort_by(|a, b| a.origin_x_px.total_cmp(&b.origin_x_px));
        for (offset, run) in line_runs.iter_mut().enumerate() {
            run.id = ShapeRunId::from_parts(run_start + offset as u32, 1);
        }

        let slice = &runs[run_start as usize..run_end as usize];
        // A line with no glyphs still starts where it starts: falling back to
        // 0..0 would make every blank line claim the first line's bytes.
        let empty_at = source_line.source_start;
        let source_range = slice
            .iter()
            .map(|run| run.source.clone())
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))
            .unwrap_or(empty_at..empty_at);

        let next_line_i = visual.get(index + 1).map(|next| next.line_i);
        let break_cause = match next_line_i {
            // Still inside the same buffer line: the break was a soft wrap.
            Some(next) if next == run.line_i => LineBreakCause::Wrap,
            // A new buffer line: the break was an authored newline.
            Some(_) => LineBreakCause::Explicit,
            None => LineBreakCause::EndOfText,
        };

        let ascent = slice
            .iter()
            .map(|run| run.metrics.ascent_px)
            .fold(0.0_f32, f32::max);
        let descent = slice
            .iter()
            .map(|run| run.metrics.descent_px)
            .fold(0.0_f32, f32::max);

        lines.push(LineBox {
            index: index as u32,
            source: source_range,
            runs: run_start..run_end,
            break_cause,
            // The case declared it and every line is isolated at it, so it is
            // simply what the caller asked for. `LayoutRun::rtl` is cosmic's
            // first-strong guess, which cannot see through the isolate.
            base_direction: match case.constraints.base_direction {
                DirSpec::Ltr => RunDirection::Ltr,
                DirSpec::Rtl => RunDirection::Rtl,
            },
            metrics: LineMetrics {
                baseline_y_px: run.line_y,
                top_y_px: run.line_top,
                height_px: run.line_height,
                ascent_px: ascent,
                descent_px: descent,
                width_px: run.line_w,
            },
            bounds: TextRect::new(0.0, run.line_top, run.line_w, run.line_height),
        });
    }

    let mut overflow = OverflowFlags::NONE;
    if let Some(max_lines) = case.constraints.max_lines
        && lines.len() > usize::from(max_lines)
    {
        let keep = usize::from(max_lines);
        // `max_lines: 0` is legal and means "keep nothing", so there is no
        // last retained line to read a run bound from.
        let run_end = keep.checked_sub(1).map_or(0, |last| lines[last].runs.end);
        lines.truncate(keep);
        runs.truncate(run_end as usize);
        if let Some(last) = lines.last_mut() {
            last.break_cause = LineBreakCause::MaxLines;
        }
        overflow = overflow.with(OverflowFlags::TRUNCATED_LINES);
        if case.constraints.ellipsis {
            // cosmic 0.19 has no ellipsis in `Buffer`; the product path
            // substitutes one itself. The reference records that truncation
            // happened and does not invent an ellipsis glyph.
            overflow = overflow.with(OverflowFlags::ELLIPSIZED);
        }
    }

    let scale = case.constraints.scale.px_per_logical;
    if let Some(max_width) = case.constraints.max_width_px
        && lines
            .iter()
            .any(|line| line.metrics.width_px > max_width * scale + 0.01)
    {
        overflow = overflow.with(OverflowFlags::CLIPPED_WIDTH);
    }

    let bounds = lines
        .iter()
        .map(|line| line.bounds)
        .reduce(TextRect::union)
        .unwrap_or_default();

    TextLayout {
        id: TextLayoutId::from_parts(0, 1),
        kind: case.kind,
        revision: source.revision(),
        font_generation: FontGeneration::new(1),
        constraints: case.constraints,
        runs,
        lines,
        bounds,
        overflow,
        // The reference lays every case out horizontally; no corpus case asks
        // for a vertical writing mode, and cosmic could not honour one anyway.
        unsupported_writing_mode: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn shaped_run(
    group: &[&cosmic_text::LayoutGlyph],
    source_line: &SourceLine,
    level_bias: u8,
    font_system: &mut FontSystem,
    faces: &[fontdb::ID],
    requested: Option<fontdb::ID>,
    index: usize,
) -> ShapedRun {
    let first = group[0];
    // Report the level the text has in the paragraph, not the one the isolate
    // pushed it to; direction follows from the corrected level.
    let level = first.level.number().saturating_sub(level_bias);
    let direction = RunDirection::from_bidi_level(level);
    let font_index = faces
        .iter()
        .position(|id| *id == first.font_id)
        .unwrap_or(usize::MAX);
    let is_fallback = requested.is_some_and(|id| id != first.font_id);

    let glyphs: Vec<ShapedGlyph> = group
        .iter()
        .map(|glyph| {
            let mut flags = GlyphFlags::NONE;
            if glyph.glyph_id == 0 {
                flags = flags.with(GlyphFlags::MISSING);
            }
            if is_fallback {
                flags = flags.with(GlyphFlags::FALLBACK_FONT);
            }
            ShapedGlyph {
                glyph_id: u32::from(glyph.glyph_id),
                cluster: source_line.to_source(glyph.start).unwrap_or(0) as u32,
                cluster_end: source_line.to_source(glyph.end).unwrap_or(0) as u32,
                advance_px: glyph.w,
                advance_y_px: 0.0,
                // cosmic reports these in EM; the IR is in px throughout.
                offset_x_px: glyph.x_offset * glyph.font_size,
                offset_y_px: glyph.y_offset * glyph.font_size,
                flags,
            }
        })
        .collect();

    let source_start = group
        .iter()
        .filter_map(|glyph| source_line.to_source(glyph.start))
        .min()
        .unwrap_or(source_line.source_start);
    let source_end = group
        .iter()
        .filter_map(|glyph| source_line.to_source(glyph.end))
        .max()
        .unwrap_or(source_line.source_start);

    let (ascent_px, descent_px, line_gap_px) = face_metrics(
        font_system,
        first.font_id,
        first.font_weight,
        first.font_size,
    );

    ShapedRun {
        id: ShapeRunId::from_parts(index as u32, 1),
        source: source_start..source_end,
        direction,
        bidi_level: level,
        script: ScriptTag::UNKNOWN,
        font: FontId::from_parts(font_index as u32, 1),
        font_size_px: first.font_size,
        advance_px: group.iter().map(|glyph| glyph.w).sum(),
        origin_x_px: group
            .iter()
            .map(|glyph| glyph.x)
            .fold(f32::INFINITY, f32::min),
        glyphs,
        metrics: RunMetrics {
            ascent_px,
            descent_px,
            line_gap_px,
        },
    }
}

/// Per-run vertical metrics, scaled from the face rather than copied off the
/// line: a line's ascent is the max over its runs, so taking the line's number
/// would make every run in a mixed-font line claim the same metrics.
fn face_metrics(
    font_system: &mut FontSystem,
    font_id: fontdb::ID,
    weight: fontdb::Weight,
    font_size: f32,
) -> (f32, f32, f32) {
    let Some(font) = font_system.get_font(font_id, weight) else {
        return (0.0, 0.0, 0.0);
    };
    let metrics = font.metrics();
    let upem = f32::from(metrics.units_per_em);
    if upem == 0.0 {
        return (0.0, 0.0, 0.0);
    }
    (
        metrics.ascent / upem * font_size,
        metrics.descent.abs() / upem * font_size,
        metrics.leading / upem * font_size,
    )
}
