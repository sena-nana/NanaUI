//! Nana-owned cosmic-text shaper. Layout metrics stay on Runtime.

use cosmic_text::{
    Affinity, Attrs, Buffer, Cursor, Ellipsize, EllipsizeHeightLimit, Family, FeatureTag,
    FontFeatures, FontSystem, Metrics, Shaping, Style, Weight, Wrap,
};
use nana_ui_core::LineHeightSpec;
use nana_ui_runtime::{
    ComputedStyle, GlyphCache, LayoutBox, StableNodeId, TextContent, TextMetrics,
    TextShapeConstraints, TextShaper, TextShaping,
};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use unicode_segmentation::UnicodeSegmentation;

/// Product text shaper for Runtime flush on the Nana WGPU host path.
#[derive(Debug)]
pub struct NanaTextShaper {
    font_system: SharedFontSystem,
}

impl Default for NanaTextShaper {
    fn default() -> Self {
        Self {
            font_system: nana_font_system(),
        }
    }
}

/// One font database shared by Runtime shaping and paint-time rasterization.
pub(crate) type SharedFontSystem = Arc<Mutex<FontSystem>>;

static FONT_SYSTEM: OnceLock<SharedFontSystem> = OnceLock::new();

/// The process-wide font system.
///
/// `FontSystem::new()` enumerates system fonts, and under `bundled-fonts` it
/// also parses every bundled face. Runtime shaping ([`NanaTextShaper`]) and
/// paint-time glyph rasterization (`TextPipeline`) both need the same database,
/// and on the product path they run sequentially on one thread, so a single
/// shared instance loads the font data once and the lock never contends.
pub(crate) fn nana_font_system() -> SharedFontSystem {
    Arc::clone(FONT_SYSTEM.get_or_init(|| Arc::new(Mutex::new(build_font_system()))))
}

/// Borrow the shared font system. Recovers from poisoning because a panic
/// elsewhere leaves the font database itself intact.
pub(crate) fn lock_font_system(shared: &SharedFontSystem) -> MutexGuard<'_, FontSystem> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Register a CSS `@font-face` source into the process-wide [`FontSystem`].
///
/// Loads `data` into fontdb, then aliases the CSS `font-family` (and optional
/// weight range) onto the new faces so `font-family: Display` resolves. This is
/// the host hook for L1 `@font-face` — not a CSSOM `FontFace` object.
///
/// `weight` is the CSS start (or sole) value; `weight_end` is the inclusive
/// range end (`font-weight: 200 700`). fontdb stores one `Weight` per face, so
/// a range is registered as CSS 100-step aliases covering that span — not only
/// the start.
///
/// Returns the number of faces recorded (file faces + CSS family aliases).
pub fn register_host_font_face(
    family: &str,
    data: Vec<u8>,
    weight: Option<u16>,
    weight_end: Option<u16>,
) -> usize {
    let family = family.trim();
    if family.is_empty() || data.is_empty() {
        return 0;
    }
    if data.len() as u64 > 8 * 1024 * 1024 {
        return 0;
    }
    let font_system = nana_font_system();
    let mut fonts = lock_font_system(&font_system);
    let ids = fonts
        .db_mut()
        .load_font_source(cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
            data,
        )));
    let mut aliases = 0usize;
    let snapshots: Vec<cosmic_text::fontdb::FaceInfo> = ids
        .iter()
        .filter_map(|id| fonts.db().face(*id).cloned())
        .collect();
    for info in snapshots {
        for aliased in css_family_weight_aliases(info, family, weight, weight_end) {
            fonts.db_mut().push_face_info(aliased);
            aliases += 1;
        }
    }
    ids.len() + aliases
}

/// Bind `@font-face` `local("Family")` to a face already in [`FontSystem`].
///
/// Matches fontdb family names and PostScript names (ASCII case-insensitive).
/// Does not load bytes or follow `url()`. If `weight` is set, matching-weight
/// (or in-range) faces are preferred; otherwise any family hit succeeds.
///
/// Returns the number of alias faces recorded (0 if nothing matched).
pub fn alias_host_font_face_local(
    css_family: &str,
    local_family: &str,
    weight: Option<u16>,
    weight_end: Option<u16>,
) -> usize {
    let css_family = css_family.trim();
    let local_family = local_family.trim();
    if css_family.is_empty() || local_family.is_empty() {
        return 0;
    }
    let font_system = nana_font_system();
    let mut fonts = lock_font_system(&font_system);
    let snapshots: Vec<cosmic_text::fontdb::FaceInfo> = fonts
        .db()
        .faces()
        .filter(|face| face_matches_local_name(face, local_family))
        .cloned()
        .collect();
    if snapshots.is_empty() {
        return 0;
    }
    let chosen = select_local_faces(snapshots, weight, weight_end);
    let mut aliases = 0usize;
    let mut to_push = Vec::new();
    for info in chosen {
        let already = info
            .families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(css_family));
        if already {
            aliases += 1;
            continue;
        }
        for aliased in css_family_weight_aliases(info, css_family, weight, weight_end) {
            to_push.push(aliased);
            aliases += 1;
        }
    }
    if !to_push.is_empty() {
        let db = fonts.db_mut();
        for info in to_push {
            db.push_face_info(info);
        }
    }
    aliases
}

fn css_family_weight_aliases(
    mut info: cosmic_text::fontdb::FaceInfo,
    family: &str,
    weight: Option<u16>,
    weight_end: Option<u16>,
) -> Vec<cosmic_text::fontdb::FaceInfo> {
    let already = info
        .families
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(family));
    if !already {
        info.families.insert(
            0,
            (
                family.to_string(),
                cosmic_text::fontdb::Language::English_UnitedStates,
            ),
        );
    }
    let Some(start) = weight else {
        info.id = cosmic_text::fontdb::ID::dummy();
        return vec![info];
    };
    let end = weight_end.unwrap_or(start);
    css_font_weight_alias_stops(start, end)
        .into_iter()
        .map(|w| {
            let mut face = info.clone();
            face.weight = cosmic_text::fontdb::Weight(w);
            face.id = cosmic_text::fontdb::ID::dummy();
            face
        })
        .collect()
}

/// CSS 100-step aliases covering an `@font-face` `font-weight` range.
pub(crate) fn css_font_weight_alias_stops(min: u16, max: u16) -> Vec<u16> {
    let lo = min.min(max).clamp(1, 1000);
    let hi = min.max(max).clamp(1, 1000);
    let mut stops = vec![lo];
    let mut step = lo.saturating_add(99) / 100 * 100;
    if step <= lo {
        step = step.saturating_add(100);
    }
    while step < hi {
        stops.push(step);
        step = step.saturating_add(100);
    }
    if hi != lo {
        stops.push(hi);
    }
    stops.dedup();
    stops
}

fn face_matches_local_name(face: &cosmic_text::fontdb::FaceInfo, local_family: &str) -> bool {
    face.families
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(local_family))
        || face.post_script_name.eq_ignore_ascii_case(local_family)
}

fn select_local_faces(
    snapshots: Vec<cosmic_text::fontdb::FaceInfo>,
    weight: Option<u16>,
    weight_end: Option<u16>,
) -> Vec<cosmic_text::fontdb::FaceInfo> {
    let Some(start) = weight else {
        return snapshots;
    };
    let end = weight_end.unwrap_or(start);
    let lo = start.min(end);
    let hi = start.max(end);
    let matching: Vec<_> = snapshots
        .iter()
        .filter(|face| face.weight.0 >= lo && face.weight.0 <= hi)
        .cloned()
        .collect();
    if matching.is_empty() {
        snapshots
    } else {
        matching
    }
}

fn build_font_system() -> FontSystem {
    #[cfg(not(feature = "bundled-fonts"))]
    {
        FontSystem::new()
    }
    #[cfg(feature = "bundled-fonts")]
    {
        let mut font_system = FontSystem::new();
        for source in crate::ui_font_sources() {
            let _ = font_system
                .db_mut()
                .load_font_source(cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                    source,
                )));
        }
        font_system.db_mut().set_sans_serif_family("Noto Sans SC");
        font_system
    }
}

impl TextShaper for NanaTextShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        let buffer = self.shape_buffer(&text.value, style, constraints);
        let (width, height, _) = measure(&buffer);
        TextMetrics {
            width: width.max(0.0),
            height,
        }
    }

    fn shape_cached(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        glyphs: &mut GlyphCache,
    ) -> TextMetrics {
        if !constraints.wrap
            && !constraints.ellipsis
            && let Some(ch) = single_char(&text.value)
            && let Some(advance) = glyphs.peek(ch, style)
        {
            let _ = glyphs.lookup(ch, style);
            return metrics_from_advance(advance, style, text.value.chars().count());
        }
        let buffer = self.shape_buffer(&text.value, style, constraints);
        record_shaped_glyphs(&buffer, style, glyphs);
        let (width, height, _) = measure(&buffer);
        TextMetrics {
            width: width.max(0.0),
            height,
        }
    }

    fn horizontal_offset(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
    ) -> f32 {
        if byte_offset > text.value.len()
            || !text.value.is_char_boundary(byte_offset)
            || !is_grapheme_boundary(&text.value, byte_offset)
        {
            return 0.0;
        }
        let buffer = self.shape_buffer(
            &text.value,
            style,
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        let graphemes = text.value[..byte_offset].graphemes(true).count();
        grapheme_x(&buffer, 0, graphemes).unwrap_or(0.0)
    }

    fn text_position(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> (f32, f32, f32) {
        if byte_offset > text.value.len()
            || !text.value.is_char_boundary(byte_offset)
            || !is_grapheme_boundary(&text.value, byte_offset)
        {
            return (0.0, 0.0, 0.0);
        }
        let buffer = self.shape_buffer(&text.value, style, constraints);
        let Some(cursor) = cosmic_cursor(&buffer, byte_offset, Affinity::After) else {
            return (0.0, 0.0, 0.0);
        };
        let mut position = None;
        for run in buffer.layout_runs() {
            if let Some(x) = run.cursor_position(&cursor) {
                position = Some((x, run.line_top, run.line_height));
            }
        }
        if let Some(position) = position {
            return position;
        }
        buffer
            .layout_runs()
            .find(|run| run.line_i == cursor.line && run.glyphs.is_empty())
            .map_or((0.0, 0.0, resolved_line_height(style)), |run| {
                (0.0, run.line_top, run.line_height)
            })
    }

    fn text_highlights(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        selection: (usize, usize),
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Vec<LayoutBox> {
        let (start, end) = selection;
        if start >= end
            || end > text.value.len()
            || !text.value.is_char_boundary(start)
            || !text.value.is_char_boundary(end)
            || !is_grapheme_boundary(&text.value, start)
            || !is_grapheme_boundary(&text.value, end)
        {
            return Vec::new();
        }
        let buffer = self.shape_buffer(&text.value, style, constraints);
        let Some(start) = cosmic_cursor(&buffer, start, Affinity::After) else {
            return Vec::new();
        };
        let Some(end) = cosmic_cursor(&buffer, end, Affinity::Before) else {
            return Vec::new();
        };
        let mut highlights = Vec::new();
        for run in buffer.layout_runs() {
            if run.line_i < start.line || run.line_i > end.line {
                continue;
            }
            for (x, width) in run.highlight(start, end) {
                highlights.push(LayoutBox {
                    x,
                    y: run.line_top,
                    width,
                    height: run.line_height,
                });
            }
        }
        highlights
    }
}

impl NanaTextShaper {
    fn shape_buffer(
        &mut self,
        text: &str,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Buffer {
        let font_size = style.font_size.max(f32::MIN_POSITIVE);
        let line_height = resolved_line_height(style).max(f32::MIN_POSITIVE);
        let mut fonts = lock_font_system(&self.font_system);
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(font_size, line_height));
        buffer.set_size(
            Some(constraints.max_width.unwrap_or(f32::INFINITY)),
            Some(constraints.max_height.unwrap_or(f32::INFINITY)),
        );
        buffer.set_wrap(cosmic_wrap(constraints.wrap, constraints.wrap_break));
        buffer.set_ellipsize(if constraints.ellipsis {
            let limit = constraints
                .max_lines
                .map(|n| EllipsizeHeightLimit::Lines(n.max(1) as usize))
                .unwrap_or_else(|| {
                    EllipsizeHeightLimit::Height(constraints.max_height.unwrap_or(f32::INFINITY))
                });
            Ellipsize::End(limit)
        } else {
            Ellipsize::None
        });
        let attrs = text_attrs(style);
        let shaping = match constraints.shaping {
            TextShaping::Auto | TextShaping::Advanced => Shaping::Advanced,
        };
        buffer.set_text(text, &attrs, shaping, None);
        buffer.shape_until_scroll(&mut fonts, false);

        let (min_width, min_height, has_rtl) = measure(&buffer);
        if has_rtl {
            buffer.set_size(Some(min_width), Some(min_height));
            buffer.shape_until_scroll(&mut fonts, false);
        }
        buffer
    }
}

fn text_attrs(style: &ComputedStyle) -> Attrs<'_> {
    let mut attrs = Attrs::new()
        .family(resolve_family(style.font_family.as_deref()))
        .weight(font_weight(style.font_weight));
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    if style.letter_spacing != 0.0 {
        attrs = attrs.letter_spacing(letter_spacing_em(style.letter_spacing, style.font_size));
    }
    if !style.font_features.is_empty() {
        let mut features = FontFeatures::new();
        for setting in &style.font_features {
            features.set(FeatureTag::new(&setting.tag), setting.value);
        }
        attrs = attrs.font_features(features);
    }
    attrs
}

pub(crate) fn cosmic_wrap(wrap: bool, mode: nana_ui_core::TextWrapBreak) -> Wrap {
    if !wrap {
        return Wrap::None;
    }
    match mode {
        nana_ui_core::TextWrapBreak::Word => Wrap::Word,
        nana_ui_core::TextWrapBreak::WordOrGlyph => Wrap::WordOrGlyph,
        nana_ui_core::TextWrapBreak::Glyph => Wrap::Glyph,
    }
}

pub(crate) fn letter_spacing_em(letter_spacing_px: f32, font_size: f32) -> f32 {
    if !letter_spacing_px.is_finite()
        || letter_spacing_px == 0.0
        || font_size.abs() < f32::MIN_POSITIVE
    {
        0.0
    } else {
        letter_spacing_px / font_size
    }
}

pub(crate) fn resolve_family(family: Option<&str>) -> Family<'_> {
    let trimmed = family.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return Family::SansSerif;
    }
    let lowered = trimmed.to_ascii_lowercase();
    if matches!(lowered.as_str(), "sans-serif" | "system-ui") {
        Family::SansSerif
    } else if lowered.contains("mono") {
        Family::Monospace
    } else {
        Family::Name(trimmed)
    }
}

fn font_weight(weight: Option<u16>) -> Weight {
    match weight.unwrap_or(400) {
        0..=199 => Weight::THIN,
        200..=299 => Weight::EXTRA_LIGHT,
        300..=349 => Weight::LIGHT,
        350..=449 => Weight::NORMAL,
        450..=549 => Weight::MEDIUM,
        550..=649 => Weight::SEMIBOLD,
        650..=749 => Weight::BOLD,
        750..=849 => Weight::EXTRA_BOLD,
        _ => Weight::BLACK,
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

fn metrics_from_advance(advance: f32, style: &ComputedStyle, _char_count: usize) -> TextMetrics {
    TextMetrics {
        width: advance.max(0.0),
        height: resolved_line_height(style),
    }
}

fn record_shaped_glyphs(buffer: &Buffer, style: &ComputedStyle, glyphs: &mut GlyphCache) {
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let cluster = &run.text[glyph.start..glyph.end];
            let mut chars = cluster.chars();
            let Some(ch) = chars.next() else {
                continue;
            };
            if chars.next().is_some() {
                continue;
            }
            if glyphs.lookup(ch, style).is_none() {
                glyphs.insert(ch, style, glyph.w);
            }
        }
    }
}

fn resolved_line_height(style: &ComputedStyle) -> f32 {
    match style.line_height {
        Some(LineHeightSpec::Absolute(value)) => value.max(0.0),
        Some(LineHeightSpec::Relative(value)) => style.font_size * value.max(0.0),
        None => style.font_size * 1.2,
    }
}

fn measure(buffer: &Buffer) -> (f32, f32, bool) {
    buffer
        .layout_runs()
        .fold((0.0, 0.0, false), |(width, height, has_rtl), run| {
            (
                run.line_w.max(width),
                height + run.line_height,
                has_rtl || run.rtl,
            )
        })
}

fn grapheme_x(buffer: &Buffer, line: usize, index: usize) -> Option<f32> {
    let run = buffer.layout_runs().nth(line)?;
    let mut last_start = None;
    let mut last_grapheme_count = 0;
    let mut graphemes_seen = 0;

    let glyph = run
        .glyphs
        .iter()
        .find(|glyph| {
            if Some(glyph.start) != last_start {
                last_grapheme_count = run.text[glyph.start..glyph.end].graphemes(false).count();
                last_start = Some(glyph.start);
                graphemes_seen += last_grapheme_count;
            }
            graphemes_seen >= index
        })
        .or_else(|| run.glyphs.last())?;

    let advance = if index == 0 {
        0.0
    } else {
        glyph.w
            * (1.0
                - graphemes_seen.saturating_sub(index) as f32 / last_grapheme_count.max(1) as f32)
    };

    Some(glyph.x + glyph.x_offset * glyph.font_size + advance)
}

fn is_grapheme_boundary(value: &str, offset: usize) -> bool {
    offset == value.len()
        || value
            .grapheme_indices(true)
            .any(|(boundary, _)| boundary == offset)
}

fn cosmic_cursor(buffer: &Buffer, byte_offset: usize, affinity: Affinity) -> Option<Cursor> {
    let mut base = 0;
    for (line, content) in buffer.lines.iter().enumerate() {
        let line_end = base + content.text().len();
        if byte_offset <= line_end {
            return Some(Cursor::new_with_affinity(
                line,
                byte_offset - base,
                affinity,
            ));
        }
        base = line_end + content.ending().as_str().len();
        if byte_offset < base {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> StableNodeId {
        StableNodeId::new(1).unwrap()
    }

    fn assert_positive_finite(metrics: TextMetrics) {
        assert!(metrics.width.is_finite() && metrics.width > 0.0);
        assert!(metrics.height.is_finite() && metrics.height > 0.0);
    }

    #[test]
    #[cfg(feature = "bundled-fonts")]
    fn register_host_font_face_aliases_css_family() {
        let data = include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf");
        let added = register_host_font_face("NanaCssFace", data.to_vec(), Some(400), None);
        assert!(added > 0, "bundled Regular face must load");
        let fonts = nana_font_system();
        let db = lock_font_system(&fonts);
        let found = db.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("NanaCssFace"))
        });
        assert!(found, "CSS family alias must be queryable in fontdb");
    }

    #[test]
    fn register_host_font_face_rejects_garbage() {
        assert_eq!(
            register_host_font_face("Nope", b"not-a-font".to_vec(), Some(400), None),
            0
        );
    }

    #[test]
    fn css_font_weight_alias_stops_cover_range_not_only_start() {
        assert_eq!(
            css_font_weight_alias_stops(200, 700),
            vec![200, 300, 400, 500, 600, 700]
        );
        assert_eq!(css_font_weight_alias_stops(400, 400), vec![400]);
        assert_eq!(
            css_font_weight_alias_stops(700, 200),
            vec![200, 300, 400, 500, 600, 700]
        );
    }

    #[test]
    #[cfg(feature = "bundled-fonts")]
    fn register_host_font_face_weight_range_aliases_stops() {
        let data = include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf");
        let added = register_host_font_face("NanaVfRangeFace", data.to_vec(), Some(200), Some(700));
        assert!(added > 0, "bundled Regular face must load");
        let fonts = nana_font_system();
        let db = lock_font_system(&fonts);
        let mut weights: Vec<u16> = db
            .db()
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("NanaVfRangeFace"))
            })
            .map(|face| face.weight.0)
            .collect();
        weights.sort_unstable();
        weights.dedup();
        for stop in [200u16, 400, 700] {
            assert!(
                weights.contains(&stop),
                "range 200 700 must register stop {stop}, got {weights:?}"
            );
        }
    }

    #[test]
    fn alias_host_font_face_local_unknown_family_is_zero() {
        assert_eq!(
            alias_host_font_face_local("NopeLocal", "DefinitelyNotANanaFont_xyz", None, None),
            0
        );
    }

    #[test]
    #[cfg(feature = "bundled-fonts")]
    fn alias_host_font_face_local_binds_bundled_noto() {
        let added = alias_host_font_face_local("NanaBundledLocal", "Noto Sans SC", Some(400), None);
        assert!(added > 0, "bundled Noto Sans SC must satisfy local()");
        let fonts = nana_font_system();
        let db = lock_font_system(&fonts);
        let found = db.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("NanaBundledLocal"))
        });
        assert!(found, "local() alias of bundled family must be queryable");
    }

    #[test]
    fn alias_host_font_face_local_same_family_succeeds_without_reload() {
        let fonts = nana_font_system();
        let existing = {
            let db = lock_font_system(&fonts);
            db.db()
                .faces()
                .find_map(|face| face.families.first().map(|(name, _)| name.clone()))
        };
        let Some(name) = existing else {
            return;
        };
        let added = alias_host_font_face_local(&name, &name, None, None);
        assert!(
            added > 0,
            "local() of an already-loaded family must succeed without url bytes"
        );
    }

    #[test]
    fn every_shaper_shares_one_font_database() {
        let first = NanaTextShaper::default();
        let second = NanaTextShaper::default();
        let painter_side = nana_font_system();

        // Runtime shaping and paint-time rasterization must reach the same
        // database. A second instance reparses every bundled face.
        assert!(Arc::ptr_eq(&first.font_system, &second.font_system));
        assert!(Arc::ptr_eq(&first.font_system, &painter_side));

        // Shaping through one handle must not disturb the other.
        let mut shaper = second;
        let metrics = shaper.shape(
            node(),
            &TextContent {
                value: "shared".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints::default(),
        );
        assert_positive_finite(metrics);
    }

    #[test]
    fn shapes_ascii_within_a_finite_max_width() {
        let metrics = NanaTextShaper::default().shape(
            node(),
            &TextContent {
                value: "Hello, Nana".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints {
                max_width: Some(240.0),
                wrap: true,
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        assert_positive_finite(metrics);
    }

    #[test]
    fn shapes_cjk_weekday_with_nonzero_width() {
        let metrics = NanaTextShaper::default().shape(
            node(),
            &TextContent {
                value: "周一".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints::default(),
        );
        assert!(metrics.width.is_finite() && metrics.width > 0.0);
        assert!(metrics.height.is_finite() && metrics.height > 0.0);
    }

    #[test]
    fn invalid_byte_offsets_return_zero_and_empty_highlights() {
        let mut shaper = NanaTextShaper::default();
        let text = TextContent {
            value: "周一👩‍💻".into(),
        };
        let style = ComputedStyle::default();
        let constraints = TextShapeConstraints::default();
        let mid_char = 1;
        let past_end = text.value.len() + 4;
        let mid_emoji = "周一".len() + 1;

        assert_eq!(
            shaper.horizontal_offset(node(), &text, mid_char, &style),
            0.0
        );
        assert_eq!(
            shaper.horizontal_offset(node(), &text, past_end, &style),
            0.0
        );
        assert_eq!(
            shaper.horizontal_offset(node(), &text, mid_emoji, &style),
            0.0
        );
        assert_eq!(
            shaper.text_position(node(), &text, mid_char, &style, constraints),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(
            shaper.text_position(node(), &text, past_end, &style, constraints),
            (0.0, 0.0, 0.0)
        );
        assert!(
            shaper
                .text_highlights(
                    node(),
                    &text,
                    (mid_char, text.value.len()),
                    &style,
                    constraints
                )
                .is_empty()
        );
        assert!(
            shaper
                .text_highlights(node(), &text, (0, past_end), &style, constraints)
                .is_empty()
        );
        assert!(
            shaper
                .text_highlights(node(), &text, (text.value.len(), 0), &style, constraints)
                .is_empty()
        );
    }

    #[test]
    fn highlight_rects_are_ordered_and_finite() {
        let mut shaper = NanaTextShaper::default();
        let style = ComputedStyle {
            font_size: 16.0,
            line_height: Some(LineHeightSpec::Absolute(20.0)),
            ..ComputedStyle::default()
        };
        let two_cjk = shaper.shape(
            node(),
            &TextContent {
                value: "甲乙".into(),
            },
            &style,
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        let wrapped = TextShapeConstraints {
            max_width: Some(two_cjk.width + 1.0),
            wrap: true,
            shaping: TextShaping::Advanced,
            ..TextShapeConstraints::default()
        };
        let text = TextContent {
            value: "甲乙👩‍💻丙丁戊".into(),
        };
        let highlights = shaper.text_highlights(
            node(),
            &text,
            ("甲".len(), "甲乙👩‍💻丙丁".len()),
            &style,
            wrapped,
        );

        assert!(highlights.len() >= 2);
        assert!(highlights.windows(2).all(|lines| lines[0].y < lines[1].y));
        assert!(highlights.iter().all(|line| {
            line.width.is_finite()
                && line.height.is_finite()
                && line.x.is_finite()
                && line.y.is_finite()
                && line.width > 0.0
                && line.height > 0.0
        }));

        let unknown = shaper.shape(
            node(),
            &TextContent {
                value: "Hello".into(),
            },
            &ComputedStyle {
                font_family: Some("DefinitelyNotARealFamily".into()),
                ..ComputedStyle::default()
            },
            TextShapeConstraints::default(),
        );
        assert_positive_finite(unknown);
    }

    #[test]
    fn letter_spacing_widens_shaped_metrics() {
        let text = TextContent {
            value: "标题文字".into(),
        };
        let tight = NanaTextShaper::default().shape(
            node(),
            &text,
            &ComputedStyle {
                font_size: 16.0,
                font_family: Some("Noto Sans SC".into()),
                ..ComputedStyle::default()
            },
            TextShapeConstraints::default(),
        );
        let tracked = NanaTextShaper::default().shape(
            node(),
            &text,
            &ComputedStyle {
                font_size: 16.0,
                font_family: Some("Noto Sans SC".into()),
                letter_spacing: 0.5,
                ..ComputedStyle::default()
            },
            TextShapeConstraints::default(),
        );
        assert!(tight.width.is_finite() && tight.width > 0.0);
        assert!(
            tracked.width > tight.width,
            "0.5px tracking must be visible in layout width, tight={} tracked={}",
            tight.width,
            tracked.width
        );
    }

    #[test]
    fn glyph_cache_stores_advances_and_world_counts_miss_then_hit() {
        let mut shaper = NanaTextShaper::default();
        let mut glyphs = GlyphCache::default();
        let style = ComputedStyle {
            font_size: 16.0,
            ..ComputedStyle::default()
        };
        let constraints = TextShapeConstraints {
            shaping: TextShaping::Advanced,
            ..TextShapeConstraints::default()
        };
        let first = shaper.shape_cached(
            node(),
            &TextContent { value: "ab".into() },
            &style,
            constraints,
            &mut glyphs,
        );
        assert_positive_finite(first);
        let advance_a = glyphs.peek('a', &style).expect("shaped 'a' must be cached");
        let advance_b = glyphs.peek('b', &style).expect("shaped 'b' must be cached");
        assert!(advance_a > 0.0 && advance_a.is_finite());
        assert!(advance_b > 0.0 && advance_b.is_finite());

        let reused = shaper.shape_cached(
            node(),
            &TextContent { value: "a".into() },
            &style,
            constraints,
            &mut glyphs,
        );
        assert!((reused.width - advance_a).abs() < 0.01);
        assert!(reused.height.is_finite() && reused.height > 0.0);

        let mut world = nana_ui_runtime::UiWorld::new();
        let document = nana_ui_runtime::DocumentId::new(1).unwrap();
        let id = nana_ui_runtime::StableNodeId::new(1).unwrap();
        let mut queue = nana_ui_runtime::MutationQueue::new();
        queue.create(id, document, nana_ui_runtime::NodeKind::Text);
        queue.set_text(id, TextContent { value: "ab".into() });
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let mut world_shaper = NanaTextShaper::default();
        world.shape_text(&work.text, &mut world_shaper).unwrap();
        let missed = world.last_work_counters();
        assert_eq!(missed.glyph_cache_misses, Some(2));
        assert_eq!(missed.glyph_cache_hits, Some(0));

        let mut patch = nana_ui_runtime::MutationQueue::new();
        patch.set_text(id, TextContent { value: "ba".into() });
        world.commit(patch).unwrap();
        let reused_work = world.take_system_work();
        world.resolve_styles(&reused_work.style).unwrap();
        world
            .shape_text(&reused_work.text, &mut world_shaper)
            .unwrap();
        let hit = world.last_work_counters();
        assert_eq!(hit.glyph_cache_hits, Some(2));
        assert_eq!(hit.glyph_cache_misses, Some(0));
    }
}
