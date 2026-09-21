//! The seam a real text engine implements, and the native engine that does.
//!
//! [`NativeTextEngine`] is the whole pipeline behind one call: the #90 font
//! layer, the #91 shaper and its cache, and the #92 layout engine and its
//! cache. It is what a UiWorld adapter will hold.
//!
//! The cosmic reference implementation still lives under `tests/`, where no
//! product dependency edge can reach it; it records the migration goldens and
//! is deleted with the corpus's reference column, not with this trait.

use crate::constraints::TextConstraints;
use crate::counters::TextWorkCounters;
use crate::font::{
    FontInstanceKey, FontQuery, FontSystem, FontVariations, LanguageTag, canonical_f32_bits,
};
use crate::id::FontGeneration;
use crate::layout::{IntrinsicWidths, LayoutCounters, LayoutRequest, Layouter, TextLayout};
use crate::metrics::RunMetrics;
use crate::shaping::{ShapeCounters, ShapeRequest, ShapedText, Shaper};
use crate::source::TextSource;
use crate::style::{TextKind, TextStyle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// The character a truncated line ends with. One character, shaped like any
/// other text, so it picks up the same face, features and fallback.
const ELLIPSIS: &str = "…";

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

/// Fonts, shaping and layout behind one call.
pub struct NativeTextEngine {
    fonts: FontSystem,
    shaper: Shaper,
    layouter: Layouter,
    /// The ellipsis, as a source so it shapes through the ordinary path and
    /// lands in the ordinary cache. Shaped at most once per style, not once per
    /// node.
    ellipsis: TextSource,
    language: Option<LanguageTag>,
    language_generation: u64,
    /// Strut metrics by face instance and size. Reading them means parsing the
    /// face's metrics tables, and every text node of one style asks for the
    /// same answer, so it is read once per face instance rather than once per
    /// node per frame.
    struts: HashMap<(FontInstanceKey, u32), RunMetrics>,
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
            language: None,
            language_generation: 0,
            struts: HashMap::new(),
            strut_generation: FontGeneration::default(),
        }
    }

    /// Language hint passed to shaping (`locl`) and to font fallback.
    pub fn set_language(&mut self, language: Option<LanguageTag>) {
        if self.language != language {
            self.language = language;
            self.language_generation += 1;
        }
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
        let shaped = self.shape(TextKind::Paragraph, source, base, constraints);
        self.layouter.intrinsic_widths(&LayoutRequest::new(
            TextKind::Paragraph,
            source,
            &shaped,
            base,
            constraints,
        ))
    }

    fn shape(
        &mut self,
        kind: TextKind,
        source: &TextSource,
        base: &TextStyle,
        constraints: &TextConstraints,
    ) -> Arc<ShapedText> {
        let request = ShapeRequest::new(source, base, constraints)
            .with_language(self.language.as_ref())
            .with_vertical(constraints.lays_out_vertically(kind));
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

    fn shape_ellipsis(
        &mut self,
        kind: TextKind,
        base: &TextStyle,
        constraints: &TextConstraints,
    ) -> Arc<ShapedText> {
        // Cloning the source shares its `Arc<str>` and its content-hash memo,
        // so the shape cache still answers this from one entry however many
        // nodes truncate this frame.
        let ellipsis = self.ellipsis.clone();
        self.shape(kind, &ellipsis, base, constraints)
    }

    /// Metrics of the base style's own face, which every line box then starts
    /// from. Without them a line containing one taller fallback glyph would
    /// move its own baseline.
    fn strut_metrics(&mut self, base: &TextStyle, scale: f32) -> Option<RunMetrics> {
        let query = FontQuery::from_style(base, self.language.clone());
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
        let shaped = self.shape(kind, source, base, constraints);
        let ellipsis = constraints
            .ellipsis
            .then(|| self.shape_ellipsis(kind, base, constraints));
        let strut = self.strut_metrics(base, constraints.scale.px_per_logical);

        let layouts_before = self.layouter.counters();
        let request = LayoutRequest::new(kind, source, &shaped, base, constraints)
            .with_ellipsis(ellipsis.as_ref())
            .with_strut(strut);
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
        // #59: the layouter reports a vertical request it fell back on; the
        // pass is what carries it out to a frame, where somebody can see it.
        counters.vertical_writing_fallbacks +=
            layouts.vertical_writing_fallbacks - layouts_before.vertical_writing_fallbacks;
        counters.text_bytes_hashed += after.text_bytes_hashed - before.text_bytes_hashed;
        layout
    }
}
