//! The layout authority: shaped runs plus constraints in, an immutable
//! [`TextLayout`] out, cached under a [`LayoutKey`](super::key::LayoutKey).

use super::cache::{LayoutCache, LayoutCacheBudget};
use super::ir::{TextLayout, TextRect};
use super::key::LayoutKey;
use super::lines::{Builder, Ellipsis, LineInput, LineStrut, segments};
use crate::constraints::TextConstraints;
use crate::font::unicode;
use crate::id::TextLayoutId;
use crate::metrics::RunMetrics;
use crate::shape::RunDirection;
use crate::shaping::ShapedText;
use crate::source::{SnappedSpans, TextSource};
use crate::style::{TextKind, TextStyle};
use nana_ui_core::DirSpec;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Line box height when a style leaves `line-height` unset, as a multiple of
/// the font size.
///
/// The same number `nana_ui_core::text_line_box_height_px` uses, so a text node
/// measured by the box layout and one laid out here do not disagree about how
/// tall a line is.
const DEFAULT_LINE_HEIGHT_RATIO: f32 = 1.2;

/// One layout call's inputs.
///
/// The shaped text arrives from [`Shaper`](crate::shaping::Shaper) and is
/// **read**, never reshaped: changing the width, the wrap mode or the alignment
/// re-runs this module alone.
pub struct LayoutRequest<'a> {
    pub kind: TextKind,
    /// The text the runs were shaped from. Line breaking needs the characters,
    /// and the spans carry the line heights.
    pub source: &'a TextSource,
    pub shaped: &'a Arc<ShapedText>,
    /// Applies wherever [`TextSource::spans`] leaves a gap.
    pub style: &'a TextStyle,
    pub constraints: &'a TextConstraints,
    /// `…`, already shaped with the base style. Shaping it through the normal
    /// path is what keeps it out of the per-node work: every node with the same
    /// style shares one cached shaping of it.
    pub ellipsis: Option<&'a Arc<ShapedText>>,
    /// Metrics of the base style's own face. See [`LineStrut`].
    pub strut_metrics: Option<RunMetrics>,
}

impl<'a> LayoutRequest<'a> {
    pub fn new(
        kind: TextKind,
        source: &'a TextSource,
        shaped: &'a Arc<ShapedText>,
        style: &'a TextStyle,
        constraints: &'a TextConstraints,
    ) -> Self {
        Self {
            kind,
            source,
            shaped,
            style,
            constraints,
            ellipsis: None,
            strut_metrics: None,
        }
    }

    #[must_use]
    pub fn with_ellipsis(mut self, ellipsis: Option<&'a Arc<ShapedText>>) -> Self {
        self.ellipsis = ellipsis;
        self
    }

    /// Pins every line box to the base style's metrics, so a fallback face on
    /// one line cannot move that line's baseline.
    #[must_use]
    pub fn with_strut(mut self, metrics: Option<RunMetrics>) -> Self {
        self.strut_metrics = metrics;
        self
    }
}

/// Widths a container needs before it can pick one.
///
/// CSS `min-content` and `max-content`, in physical px: the widest piece that
/// cannot be broken further, and the width the text would take if it never
/// wrapped. Both are computed on shaped advances and on UAX #14 opportunities,
/// whether or not these constraints happen to allow wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct IntrinsicWidths {
    pub min_px: f32,
    pub max_px: f32,
}

/// Work counts for layout. Accumulate until [`Layouter::reset_counters`];
/// the two cache gauges are read when [`Layouter::counters`] is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LayoutCounters {
    pub layout_requests: usize,
    pub layout_created: usize,
    pub layout_cache_hits: usize,
    pub layout_cache_misses: usize,
    pub layout_cache_evictions: usize,
    pub layout_cache_bytes: usize,
    pub layout_cache_entries: usize,
    /// Break opportunities examined. Zero on the Label fast path, and zero
    /// whenever wrapping is off.
    pub line_break_candidates: usize,
    pub lines_created: usize,
    pub runs_placed: usize,
    pub ellipsis_runs_used: usize,
    /// Misses whose shaped runs were already laid out at other constraints: a
    /// resize, not new text.
    pub constraint_only_relayouts: usize,
    /// Shaped runs read by a layout instead of being shaped again.
    pub shape_runs_reused_for_layout: usize,
    /// Layouts *created* on the single-line Label path. A cache hit takes
    /// neither path, so these two and [`Self::layout_created`] agree.
    pub label_fast_paths: usize,
    /// Layouts created by walking paragraphs, break opportunities and wrapping.
    pub paragraph_paths: usize,
    /// Layouts that asked for a writing mode this engine does not implement
    /// and were laid out horizontally instead (#59).
    pub vertical_writing_fallbacks: usize,
}

/// Lays shaped text out and caches the result.
pub struct Layouter {
    cache: LayoutCache,
    counters: LayoutCounters,
    next_id: u64,
}

impl Default for Layouter {
    fn default() -> Self {
        Self::new(LayoutCacheBudget::default())
    }
}

impl Layouter {
    pub fn new(budget: LayoutCacheBudget) -> Self {
        Self {
            cache: LayoutCache::new(budget),
            counters: LayoutCounters::default(),
            next_id: 0,
        }
    }

    pub fn counters(&self) -> LayoutCounters {
        LayoutCounters {
            layout_cache_bytes: self.cache.bytes(),
            layout_cache_entries: self.cache.len(),
            ..self.counters
        }
    }

    pub fn reset_counters(&mut self) {
        self.counters = LayoutCounters::default();
    }

    pub fn set_budget(&mut self, budget: LayoutCacheBudget) {
        self.counters.layout_cache_evictions += self.cache.set_budget(budget);
    }

    /// Lays `request` out, from the cache when the same shaped runs were laid
    /// out at the same constraints.
    pub fn layout(&mut self, request: &LayoutRequest<'_>) -> Arc<TextLayout> {
        self.counters.layout_requests += 1;
        let scale = request.constraints.scale.px_per_logical;
        let base_line_height = line_height_px(request.style, scale);
        let run_line_heights = run_line_heights(request, scale);
        let strut = request.strut_metrics.map(|metrics| LineStrut {
            metrics,
            line_height_px: base_line_height,
        });
        let key = LayoutKey::new(
            request.shaped,
            request.source.revision(),
            request.ellipsis,
            request.kind,
            request.constraints,
            strut,
            &run_line_heights,
            base_line_height,
        );
        if let Some(hit) = self.cache.get(&key) {
            self.counters.layout_cache_hits += 1;
            return hit;
        }
        self.counters.layout_cache_misses += 1;
        if self.cache.knows_shaped(&key) {
            self.counters.constraint_only_relayouts += 1;
        }

        let layout = Arc::new(self.build(request, strut, &run_line_heights, base_line_height));
        self.counters.layout_created += 1;
        self.counters.layout_cache_evictions += self.cache.insert(key, Arc::clone(&layout));
        layout
    }

    /// `min-content` and `max-content` for this text, independent of the
    /// container width. Not cached: it is a pass over the shaped advances and
    /// has no width to key on.
    pub fn intrinsic_widths(&mut self, request: &LayoutRequest<'_>) -> IntrinsicWidths {
        let scale = request.constraints.scale.px_per_logical;
        let base_line_height = line_height_px(request.style, scale);
        let run_line_heights = run_line_heights(request, scale);
        let (widths, candidates) = Builder::new(line_input(
            request,
            None,
            &run_line_heights,
            base_line_height,
            None,
        ))
        .intrinsic_widths();
        self.counters.line_break_candidates += candidates;
        widths
    }

    fn build(
        &mut self,
        request: &LayoutRequest<'_>,
        strut: Option<LineStrut>,
        run_line_heights: &[f32],
        base_line_height: f32,
    ) -> TextLayout {
        let ellipsis = request
            .ellipsis
            .filter(|_| request.constraints.ellipsis)
            .and_then(|shaped| Ellipsis::new(shaped));
        let input = line_input(
            request,
            strut,
            run_line_heights,
            base_line_height,
            ellipsis.as_ref(),
        );
        let builder = Builder::new(input);
        let fast_path = uses_label_fast_path(request);
        let laid_out = if fast_path {
            self.counters.label_fast_paths += 1;
            builder.single_line()
        } else {
            self.counters.paragraph_paths += 1;
            builder.paragraphs()
        };
        let work = laid_out.work;
        self.counters.line_break_candidates += work.line_break_candidates;
        self.counters.lines_created += work.lines_created;
        self.counters.runs_placed += work.runs_placed;
        self.counters.ellipsis_runs_used += work.ellipsis_runs_used;
        self.counters.shape_runs_reused_for_layout += work.shape_runs_reused;

        let unsupported_writing_mode = request.constraints.wants_vertical_writing();
        if unsupported_writing_mode {
            self.counters.vertical_writing_fallbacks += 1;
        }

        let bounds = laid_out
            .lines
            .iter()
            .map(|line| line.bounds)
            .reduce(TextRect::union)
            .unwrap_or_default();

        TextLayout {
            id: self.issue_id(),
            kind: request.kind,
            revision: request.source.revision(),
            font_generation: request.shaped.font_generation,
            constraints: *request.constraints,
            runs: laid_out.runs,
            lines: laid_out.lines,
            bounds,
            overflow: laid_out.overflow,
            unsupported_writing_mode,
        }
    }

    /// Handles are issued, never recycled: the index counts up and the
    /// generation counts the wraps, so two layouts of one `Layouter` never
    /// share an id.
    fn issue_id(&mut self) -> TextLayoutId {
        let value = self.next_id;
        self.next_id += 1;
        TextLayoutId::from_parts(value as u32, (value >> 32) as u32 + 1)
    }
}

fn line_input<'r>(
    request: &'r LayoutRequest<'_>,
    strut: Option<LineStrut>,
    run_line_heights: &'r [f32],
    base_line_height: f32,
    ellipsis: Option<&'r Ellipsis>,
) -> LineInput<'r> {
    LineInput {
        text: request.source.text(),
        runs: &request.shaped.runs,
        paragraphs: &request.shaped.paragraphs,
        run_line_heights,
        constraints: request.constraints,
        strut,
        empty_line_height_px: base_line_height,
        base_direction: match request.constraints.base_direction {
            DirSpec::Ltr => RunDirection::Ltr,
            DirSpec::Rtl => RunDirection::Rtl,
        },
        ellipsis,
    }
}

/// When a label may skip the paragraph machinery entirely.
///
/// A label degrades to the paragraph path as soon as it stops being one line of
/// plain text: any wrap mode, a multi-line or zero `max_lines`, a height
/// budget, a writing mode this engine has to fall back on, or a source the
/// paragraph path would cut into more than one segment.
///
/// That last question is asked of [`segments`] rather than re-derived, so an
/// authored newline, a forced break and a *trailing* newline — which adds no
/// paragraph but does add an empty last line — all degrade alike. The scan is
/// not on the per-frame path: this runs when a layout is *built*, which a cache
/// hit skips.
fn uses_label_fast_path(request: &LayoutRequest<'_>) -> bool {
    request.kind == TextKind::Label
        && !request.constraints.wraps()
        && matches!(request.constraints.max_lines, None | Some(1))
        && request.constraints.max_height_px.is_none()
        && !request.constraints.wants_vertical_writing()
        && segments(request.source.text(), &request.shaped.paragraphs).len() == 1
}

/// Line box height a style asks for, in physical px.
fn line_height_px(style: &TextStyle, scale: f32) -> f32 {
    style
        .line_height_px()
        .unwrap_or(style.font_size_px * DEFAULT_LINE_HEIGHT_RATIO)
        * scale
}

/// The line box height of every shaped run, resolved through the same span rule
/// the shaper used.
///
/// [`SnappedSpans`] is what makes the two agree: a span boundary inside a
/// grapheme cluster snaps to the cluster's start for both, so a run cannot be
/// shaped at a span's size and then measured into a line box sized from the
/// base style.
///
/// Without spans there is nothing to resolve and nothing to segment the text
/// for — the common case costs one clone of a float.
fn run_line_heights(request: &LayoutRequest<'_>, scale: f32) -> Vec<f32> {
    let base = line_height_px(request.style, scale);
    let spans = request.source.spans();
    if spans.is_empty() {
        return vec![base; request.shaped.runs.len()];
    }
    let text = request.source.text();
    let starts = unicode::cluster_starts(text);
    let snapped = SnappedSpans::new(spans, text.len(), &starts);
    request
        .shaped
        .runs
        .iter()
        .map(|run| line_height_px(snapped.style_at(run.source.start, request.style), scale))
        .collect()
}
