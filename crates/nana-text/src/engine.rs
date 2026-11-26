//! The seam a real text engine implements, and the native engine that does.
//!
//! [`NativeTextEngine`] is the whole pipeline behind one call: the #90 font
//! layer, the #91 shaper and its cache, and the #92 layout engine and its
//! cache. It is the one engine the product holds (`nana-ui`'s
//! `SharedTextEngine`).

use crate::constraints::TextConstraints;
use crate::counters::TextWorkCounters;
use crate::font::{
    FaceOutlineSink, FaceScalerCache, FontInstanceKey, FontQuery, FontSystem, FontVariations,
    LanguageTag, canonical_f32_bits, scale_plain_outline,
};
use crate::id::FontGeneration;
use crate::layout::{
    IntrinsicWidths, LayoutCounters, LayoutRequest, Layouter, OutlineSink, ShapedEmphasis,
    TextLayout, TextRect,
};
use crate::metrics::RunMetrics;
use crate::shaping::{ShapeCounters, ShapeRequest, ShapedText, Shaper};
use crate::source::{ObjectLabel, SnappedSpans, TextSource};
use crate::style::{TextKind, TextStyle};
use std::sync::{Arc, Mutex, MutexGuard};

/// The character a truncated line ends with. One character, shaped like any
/// other text, so it picks up the same face, features and fallback.
const ELLIPSIS: &str = "…";

/// The visible hyphen drawn at the end of a line broken at `U+00AD` soft hyphen.
const HYPHEN: &str = "-";

pub trait TextEngine {
    /// Bumped by every mutation of the face set. A layout produced under a
    /// different generation is stale.
    fn font_generation(&self) -> FontGeneration;

    /// Shapes and lays out one source. `base` applies wherever
    /// [`TextSource::spans`] leaves a gap.
    ///
    /// The result is shared rather than owned: a layout is immutable, and a
    /// frame that asks for the same text at the same constraints must get the
    /// cached one rather than a copy of it.
    fn layout(
        &mut self,
        kind: TextKind,
        source: &TextSource,
        base: &TextStyle,
        constraints: &TextConstraints,
        counters: &mut TextWorkCounters,
    ) -> Arc<TextLayout>;
}

/// One engine shared by everything that shapes against the same font set:
/// every document and window of a process, and the painter that rasterizes
/// what they laid out. Sharing it is what lets one label's shaping serve
/// every other copy of it.
pub type SharedTextEngine = Arc<Mutex<NativeTextEngine>>;

/// Locks a shared engine. A panic while it was held leaves no half-applied
/// state a reader could observe — every cache insert is a single operation —
/// so a poisoned lock is recovered rather than propagated.
pub fn lock_text_engine(engine: &SharedTextEngine) -> MutexGuard<'_, NativeTextEngine> {
    engine
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a layout was produced against: the font system's identity and
/// generation, and the language hint's generation. A retained layout is
/// current only while all three match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextEngineEpoch {
    pub font_system: u64,
    pub font_generation: FontGeneration,
    /// Bumped by every change of the language hint, which reaches `locl`
    /// shaping and font fallback just as a face registration does.
    pub language_generation: u64,
}

/// Strut metrics kept, by face instance and size, before the least recently
/// used go. An animated axis or size asks with a new instance or size on every
/// frame.
const STRUT_CAP: usize = 1024;

/// Fonts, shaping and layout behind one call.
pub struct NativeTextEngine {
    fonts: FontSystem,
    shaper: Shaper,
    layouter: Layouter,
    /// The ellipsis, as a source so it shapes through the ordinary path and
    /// lands in the ordinary cache. Shaped at most once per style, not once per
    /// node.
    ellipsis: TextSource,
    /// The visible hyphen `-` inserted only when a soft hyphen (`U+00AD`) breaks.
    hyphen: TextSource,
    language: Option<LanguageTag>,
    language_generation: u64,
    /// Strut metrics by face instance and size. Reading them means parsing the
    /// face's metrics tables, and every text node of one style asks for the
    /// same answer, so it is read once per face instance rather than once per
    /// node per frame.
    struts: crate::bounded::BoundedCache<(FontInstanceKey, u32), RunMetrics>,
    /// The font generation `struts` was filled under.
    strut_generation: FontGeneration,
}

impl NativeTextEngine {
    pub fn new(fonts: FontSystem) -> Self {
        Self {
            fonts,
            shaper: Shaper::default(),
            layouter: Layouter::default(),
            ellipsis: TextSource::new(ELLIPSIS),
            hyphen: TextSource::new(HYPHEN),
            language: None,
            language_generation: 0,
            struts: crate::bounded::BoundedCache::new(STRUT_CAP),
            strut_generation: FontGeneration::default(),
        }
    }

    /// Fallback language hint passed to shaping (`locl`) and to font
    /// fallback for text whose style names no language of its own
    /// ([`TextStyle::language`]).
    pub fn set_language(&mut self, language: Option<LanguageTag>) {
        if self.language != language {
            self.language = language;
            self.language_generation += 1;
        }
    }

    /// The fallback language hint, if one is set.
    pub fn language(&self) -> Option<&LanguageTag> {
        self.language.as_ref()
    }

    /// The identity and generation every layout this engine returns is
    /// valid against.
    pub fn epoch(&self) -> TextEngineEpoch {
        TextEngineEpoch {
            font_system: self.fonts.instance_id(),
            font_generation: self.fonts.generation(),
            language_generation: self.language_generation,
        }
    }

    pub fn fonts(&self) -> &FontSystem {
        &self.fonts
    }

    pub fn fonts_mut(&mut self) -> &mut FontSystem {
        &mut self.fonts
    }

    pub fn shape_counters(&self) -> ShapeCounters {
        self.shaper.counters()
    }

    pub fn layout_counters(&self) -> LayoutCounters {
        self.layouter.counters()
    }

    pub fn reset_counters(&mut self) {
        self.shaper.reset_counters();
        self.layouter.reset_counters();
        self.fonts.reset_counters();
    }

    /// `min-content` / `max-content` for one source, for a container deciding
    /// what width to impose.
    pub fn intrinsic_widths(
        &mut self,
        source: &TextSource,
        base: &TextStyle,
        constraints: &TextConstraints,
    ) -> IntrinsicWidths {
        let source = self.fold_lines(source, constraints);
        let shaped = self.shape(source, base, constraints);
        let hyphen = (constraints.hyphens != nana_ui_core::HyphensSpec::None
            && source.text().contains('\u{00AD}'))
        .then(|| self.shape_hyphen(base, constraints));
        let request = LayoutRequest::new(TextKind::Paragraph, source, &shaped, base, constraints)
            .with_hyphen(hyphen.as_ref());
        self.layouter.intrinsic_widths(&request)
    }

    fn shape(
        &mut self,
        source: &TextSource,
        base: &TextStyle,
        constraints: &TextConstraints,
    ) -> Arc<ShapedText> {
        let language = base.language.as_ref().or(self.language.as_ref());
        let request = ShapeRequest::new(source, base, constraints).with_language(language);
        self.shaper.shape(&mut self.fonts, &request)
    }

    /// `white-space: normal` says an authored newline is a space, not a line
    /// break. Shaping and line breaking have to see the same bytes, so the fold
    /// happens here, before shaping, rather than on the laid-out lines.
    fn fold_lines<'a>(
        &self,
        source: &'a TextSource,
        constraints: &TextConstraints,
    ) -> &'a TextSource {
        if constraints.preserve_lines {
            return source;
        }
        source.with_folded_newlines().unwrap_or(source)
    }

    /// Each emphasis range's mark, shaped once per style the range crosses:
    /// a mark is half the size of the character it stands on, so a range
    /// over a larger word has larger marks there.
    fn shape_emphasis(
        &mut self,
        source: &TextSource,
        base: &TextStyle,
        constraints: &TextConstraints,
    ) -> Vec<ShapedEmphasis> {
        use unicode_segmentation::UnicodeSegmentation as _;
        if source.emphasis().is_empty() {
            return Vec::new();
        }
        let text = source.text();
        let starts;
        let snapped = if source.spans().is_empty() {
            None
        } else {
            starts = crate::font::unicode::cluster_starts(text);
            Some(SnappedSpans::new(source.spans(), text.len(), &starts))
        };
        let cuts = snapped.as_ref().map(SnappedSpans::cuts).unwrap_or_default();
        let unconstrained = TextConstraints {
            max_width_px: None,
            wrap: None,
            max_lines: None,
            ellipsis: false,
            ..*constraints
        };
        let mut shaped: Vec<ShapedEmphasis> = Vec::new();
        for span in source.emphasis() {
            let Some(mark) = span.mark.graphemes(true).next() else {
                continue;
            };
            let mark = TextSource::new(mark);
            let mut start = span.range.start;
            while start < span.range.end {
                let end = cuts
                    .iter()
                    .copied()
                    .find(|cut| *cut > start)
                    .unwrap_or(span.range.end)
                    .min(span.range.end);
                let style = snapped
                    .as_ref()
                    .map_or(base, |snapped| snapped.style_at(start, base));
                let marks = self.shape(&mark, &emphasis_style(style), &unconstrained);
                match shaped.last_mut() {
                    Some(last)
                        if last.range.end == start
                            && last.under == span.position.under
                            && Arc::ptr_eq(&last.shaped, &marks) =>
                    {
                        last.range.end = end;
                    }
                    _ => shaped.push(ShapedEmphasis {
                        range: start..end,
                        under: span.position.under,
                        shaped: marks,
                    }),
                }
                start = end;
            }
        }
        shaped
    }

    fn shape_ellipsis(
        &mut self,
        base: &TextStyle,
        constraints: &TextConstraints,
    ) -> Arc<ShapedText> {
        // Cloning the source shares its `Arc<str>` and its content-hash memo,
        // so the shape cache still answers this from one entry however many
        // nodes truncate this frame.
        let ellipsis = self.ellipsis.clone();
        self.shape(&ellipsis, base, constraints)
    }

    fn shape_hyphen(&mut self, base: &TextStyle, constraints: &TextConstraints) -> Arc<ShapedText> {
        let hyphen = self.hyphen.clone();
        self.shape(&hyphen, base, constraints)
    }

    /// Metrics of the base style's own face, which every line box then starts
    /// from. Without them a line containing one taller fallback glyph would
    /// move its own baseline. In physical px at `scale` physical px per
    /// logical px; `None` when no face serves the style.
    ///
    /// The strut of a line box laid out without this engine (an ordinary
    /// block's lines in a host's layout) reads the same answer, so both kinds
    /// of line start from one face's metrics.
    pub fn strut_metrics(&mut self, base: &TextStyle, scale: f32) -> Option<RunMetrics> {
        let language = base.language.clone().or_else(|| self.language.clone());
        let query = FontQuery::from_style(base, language);
        let selection = self.fonts.select(&query);
        let primary = selection.primary?;
        let variations = FontVariations::from_settings(&base.variations);
        let instance = self.fonts.instance(primary, &query, &variations)?;
        let size_px = base.font_size_px * scale;
        let generation = self.fonts.generation();
        if self.strut_generation != generation {
            self.struts.clear();
            self.strut_generation = generation;
        }
        let key = (instance.key.clone(), canonical_f32_bits(size_px));
        if let Some(metrics) = self.struts.get(&key) {
            return Some(*metrics);
        }
        let metrics = self.fonts.run_metrics(&instance, size_px);
        self.struts.insert(key, metrics);
        Some(metrics)
    }

    /// Emits vector glyph outlines for every glyph in `layout`, in layout
    /// coordinates (logical px, Y-down) starting at `origin`.
    pub fn emit_layout_outlines(
        &self,
        layout: &TextLayout,
        origin: [f32; 2],
        sink: &mut dyn OutlineSink,
    ) {
        /// Synthetic-oblique tangent for 14°, matching `raster::outline::OBLIQUE_DEGREES`.
        const OBLIQUE_TAN: f32 = 0.249_328; // tan(14°)

        struct GlyphAdapter<'a> {
            sink: &'a mut dyn OutlineSink,
            origin: [f32; 2],
            oblique: bool,
            sideways: bool,
            subpath_open: bool,
        }

        impl GlyphAdapter<'_> {
            #[inline]
            fn map(&self, mut x: f32, y: f32) -> [f32; 2] {
                if self.oblique {
                    x += y * OBLIQUE_TAN;
                }
                if self.sideways {
                    // Font Y-up rotated 90° CW into screen Y-down: (x, y) -> (y, x).
                    [self.origin[0] + y, self.origin[1] + x]
                } else {
                    // Font Y-up to screen Y-down.
                    [self.origin[0] + x, self.origin[1] - y]
                }
            }
        }

        impl FaceOutlineSink for GlyphAdapter<'_> {
            fn begin_layer(&mut self, _color: Option<[u8; 4]>) {}

            fn move_to(&mut self, x: f32, y: f32) {
                if self.subpath_open {
                    self.sink.close();
                }
                let [px, py] = self.map(x, y);
                self.sink.move_to(px, py);
                self.subpath_open = true;
            }

            fn line_to(&mut self, x: f32, y: f32) {
                let [px, py] = self.map(x, y);
                self.sink.line_to(px, py);
            }

            fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
                let [c0x, c0y] = self.map(cx0, cy0);
                let [px, py] = self.map(x, y);
                self.sink.quad_to(c0x, c0y, px, py);
            }

            fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
                let [c0x, c0y] = self.map(cx0, cy0);
                let [c1x, c1y] = self.map(cx1, cy1);
                let [px, py] = self.map(x, y);
                self.sink.curve_to(c0x, c0y, c1x, c1y, px, py);
            }

            fn close(&mut self) {
                if self.subpath_open {
                    self.sink.close();
                    self.subpath_open = false;
                }
            }

            fn finish_layer(&mut self) {
                if self.subpath_open {
                    self.sink.close();
                    self.subpath_open = false;
                }
            }
        }

        let mut cache = FaceScalerCache::default();
        let generation = self.fonts.generation().get();
        for placed in layout.glyph_placements(1.0, origin) {
            let Ok(glyph_id) = u16::try_from(placed.glyph_id) else {
                continue;
            };
            let Some(data) = self.fonts.face_data(placed.instance.font) else {
                continue;
            };
            let font_key = u64::from(placed.instance.font.index()) | (u64::from(generation) << 32);
            let mut adapter = GlyphAdapter {
                sink,
                origin: placed.position,
                oblique: placed.instance.synthesis.oblique,
                sideways: placed.sideways,
                subpath_open: false,
            };
            scale_plain_outline(
                data.bytes(),
                data.index(),
                font_key,
                &placed.instance.coords,
                placed.font_size_px,
                false,
                glyph_id,
                &mut cache,
                &mut adapter,
            );
        }
    }

    /// Tight bounding box of every glyph outline in `layout`, in layout
    /// coordinates (`origin = [0, 0]`). Returns `None` when no outline points
    /// were emitted (empty text or whitespace only).
    pub fn layout_ink_bounds(&self, layout: &TextLayout) -> Option<TextRect> {
        struct BoundsSink {
            min_x: f32,
            min_y: f32,
            max_x: f32,
            max_y: f32,
            any: bool,
        }

        impl BoundsSink {
            fn visit(&mut self, x: f32, y: f32) {
                if x.is_finite() && y.is_finite() {
                    self.min_x = self.min_x.min(x);
                    self.min_y = self.min_y.min(y);
                    self.max_x = self.max_x.max(x);
                    self.max_y = self.max_y.max(y);
                    self.any = true;
                }
            }
        }

        impl OutlineSink for BoundsSink {
            fn move_to(&mut self, x: f32, y: f32) {
                self.visit(x, y);
            }
            fn line_to(&mut self, x: f32, y: f32) {
                self.visit(x, y);
            }
            fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
                self.visit(cx, cy);
                self.visit(x, y);
            }
            fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
                self.visit(cx0, cy0);
                self.visit(cx1, cy1);
                self.visit(x, y);
            }
            fn close(&mut self) {}
        }

        let mut sink = BoundsSink {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
            any: false,
        };
        self.emit_layout_outlines(layout, [0.0, 0.0], &mut sink);
        sink.any.then(|| {
            TextRect::new(
                sink.min_x,
                sink.min_y,
                (sink.max_x - sink.min_x).max(0.0),
                (sink.max_y - sink.min_y).max(0.0),
            )
        })
    }
}

/// The style a ruby annotation is shaped in: its base's, at
/// [`crate::source::RUBY_SCALE`] of the size.
fn ruby_style(source: &TextSource, base: &TextStyle, at: usize) -> TextStyle {
    let spans = source.spans();
    let style = spans
        .iter()
        .rfind(|span| span.range.start <= at && at < span.range.end)
        .map_or(base, |span| &span.style);
    TextStyle {
        font_size_px: style.font_size_px * crate::source::RUBY_SCALE,
        letter_spacing_px: 0.0,
        ..style.clone()
    }
}

/// The style an emphasis mark is shaped in: the style of the character it
/// stands on, at [`crate::source::EMPHASIS_SCALE`] of the size.
fn emphasis_style(style: &TextStyle) -> TextStyle {
    TextStyle {
        font_size_px: style.font_size_px * crate::source::EMPHASIS_SCALE,
        letter_spacing_px: 0.0,
        ..style.clone()
    }
}

/// The style an object label is shaped in: the text's around it, at the
/// label's scale of the size (by default [`crate::source::LABEL_SCALE`]),
/// upright and medium weight unless the label names one.
fn label_style(source: &TextSource, base: &TextStyle, label: &ObjectLabel) -> TextStyle {
    let at = label.offset;
    let spans = source.spans();
    let style = spans
        .iter()
        .rfind(|span| span.range.start <= at && at < span.range.end)
        .map_or(base, |span| &span.style);
    let scale = if label.scale.is_finite() && label.scale > 0.0 {
        label.scale.min(4.0)
    } else {
        crate::source::LABEL_SCALE
    };
    TextStyle {
        font_size_px: (style.font_size_px * scale).max(9.0),
        font_weight: label
            .weight
            .map_or_else(|| style.font_weight.clamp(500, 700), |w| w.clamp(100, 900)),
        italic: false,
        letter_spacing_px: 0.0,
        ..style.clone()
    }
}

impl TextEngine for NativeTextEngine {
    fn font_generation(&self) -> FontGeneration {
        self.fonts.generation()
    }

    fn layout(
        &mut self,
        kind: TextKind,
        source: &TextSource,
        base: &TextStyle,
        constraints: &TextConstraints,
        counters: &mut TextWorkCounters,
    ) -> Arc<TextLayout> {
        let before = self.shaper.counters();
        let source = self.fold_lines(source, constraints);
        let shaped = self.shape(source, base, constraints);
        let ellipsis = constraints
            .ellipsis
            .then(|| self.shape_ellipsis(base, constraints));
        let hyphen = (constraints.hyphens != nana_ui_core::HyphensSpec::None
            && constraints.wraps()
            && source.text().contains('\u{00AD}'))
        .then(|| self.shape_hyphen(base, constraints));
        let strut = self.strut_metrics(base, constraints.scale.px_per_logical);

        // Each annotation shaped once, at its share of its base's size; the
        // shape cache answers the next layout of the same text.
        let rubies: Vec<Arc<ShapedText>> = if constraints.wants_vertical_writing() {
            Vec::new()
        } else {
            source
                .rubies()
                .iter()
                .map(|ruby| {
                    let style = ruby_style(source, base, ruby.range.start);
                    let annotation = TextSource::new(Arc::clone(&ruby.text));
                    let constraints = TextConstraints {
                        max_width_px: None,
                        wrap: None,
                        max_lines: None,
                        ellipsis: false,
                        ..*constraints
                    };
                    self.shape(&annotation, &style, &constraints)
                })
                .collect()
        };
        let emphasis = if constraints.wants_vertical_writing() {
            Vec::new()
        } else {
            self.shape_emphasis(source, base, constraints)
        };
        // Each object label shaped once, small, in the style around it.
        let labels: Vec<Arc<ShapedText>> = source
            .labels()
            .iter()
            .map(|label| {
                let style = label_style(source, base, label);
                let text = TextSource::new(Arc::clone(&label.text));
                let constraints = TextConstraints {
                    max_width_px: None,
                    wrap: None,
                    max_lines: None,
                    ellipsis: false,
                    ..*constraints
                };
                self.shape(&text, &style, &constraints)
            })
            .collect();
        let layouts_before = self.layouter.counters();
        let request = LayoutRequest::new(kind, source, &shaped, base, constraints)
            .with_ellipsis(ellipsis.as_ref())
            .with_hyphen(hyphen.as_ref())
            .with_strut(strut)
            .with_rubies(&rubies)
            .with_emphasis(&emphasis)
            .with_labels(&labels);
        let layout = self.layouter.layout(&request);
        let layouts = self.layouter.counters();
        let after = self.shaper.counters();

        let shaped_now = after.shape_cache_misses - before.shape_cache_misses;
        let shape_hits = after.shape_cache_hits - before.shape_cache_hits;
        let layout_hits = layouts.layout_cache_hits - layouts_before.layout_cache_hits;
        let layout_misses = layouts.layout_cache_misses - layouts_before.layout_cache_misses;
        counters.record_text_pass(1, usize::from(shaped_now > 0));
        counters.record_shape_cache(shape_hits, shaped_now);
        counters.record_layout_cache(layout_hits, layout_misses);
        counters.record_glyphs_resolved(layout.glyph_count());
        counters.shape_cache_lookups += shape_hits + shaped_now;
        counters.layout_cache_lookups += layout_hits + layout_misses;
        counters.layouts_created += layouts.layout_created - layouts_before.layout_created;
        counters.constraint_only_relayouts +=
            layouts.constraint_only_relayouts - layouts_before.constraint_only_relayouts;
        counters.line_opportunities_considered +=
            layouts.line_opportunities_considered - layouts_before.line_opportunities_considered;
        counters.line_break_comparisons +=
            layouts.line_break_comparisons - layouts_before.line_break_comparisons;
        counters.line_beam_states = counters.line_beam_states.max(layouts.line_beam_states);
        counters.line_budget_fallbacks +=
            layouts.line_budget_fallbacks - layouts_before.line_budget_fallbacks;
        // #59: the layouter reports a vertical request it fell back on; the
        // pass is what carries it out to a frame, where somebody can see it.
        counters.vertical_writing_fallbacks +=
            layouts.vertical_writing_fallbacks - layouts_before.vertical_writing_fallbacks;
        counters.text_bytes_hashed += after.text_bytes_hashed - before.text_bytes_hashed;
        layout
    }
}

#[cfg(test)]
mod outline_tests {
    use super::*;
    use crate::font::FaceDescriptor;

    #[test]
    fn emit_layout_outlines_and_ink_bounds_extract_glyph_contours() {
        let mut policy = crate::font::FallbackPolicy::empty();
        policy.set_generic(crate::font::GenericFamily::SansSerif, ["Noto Sans SC"]);
        let mut fonts = FontSystem::with_policy(policy);
        fonts
            .register_file(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../nana-ui-core/assets/fonts/NotoSansSC-Variable.ttf"
                ),
                &FaceDescriptor::default(),
            )
            .expect("bundled font registers");
        let mut engine = NativeTextEngine::new(fonts);
        let source = TextSource::new("Ag");
        let style = TextStyle {
            font_size_px: 32.0,
            ..TextStyle::default()
        };
        let mut counters = TextWorkCounters::default();
        let layout = engine.layout(
            TextKind::Label,
            &source,
            &style,
            &TextConstraints::default(),
            &mut counters,
        );

        #[derive(Default)]
        struct CounterSink {
            moves: usize,
            segments: usize,
            closes: usize,
        }
        impl OutlineSink for CounterSink {
            fn move_to(&mut self, _: f32, _: f32) {
                self.moves += 1;
            }
            fn line_to(&mut self, _: f32, _: f32) {
                self.segments += 1;
            }
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
                self.segments += 1;
            }
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
                self.segments += 1;
            }
            fn close(&mut self) {
                self.closes += 1;
            }
        }
        let mut sink = CounterSink::default();
        engine.emit_layout_outlines(&layout, [10.0, 20.0], &mut sink);
        assert!(
            sink.moves >= 2,
            "expected contours for A and g: {}",
            sink.moves
        );
        assert!(sink.segments > 4);
        assert!(sink.closes >= 2);

        let ink_ag = engine
            .layout_ink_bounds(&layout)
            .expect("ink bounds for Ag");
        assert!(ink_ag.width > 10.0 && ink_ag.height > 10.0);

        let space_layout = engine.layout(
            TextKind::Label,
            &TextSource::new("   "),
            &style,
            &TextConstraints::default(),
            &mut counters,
        );
        assert!(engine.layout_ink_bounds(&space_layout).is_none());
    }
}
