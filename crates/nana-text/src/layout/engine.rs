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
use crate::shape::{RunDirection, ShapedRun};
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
    /// The shaped annotation of each of [`TextSource::rubies`], in order;
    /// empty when they are not laid out (vertical text).
    pub rubies: &'a [Arc<ShapedText>],
    /// The shaped text of each of [`TextSource::labels`], in order.
    pub labels: &'a [Arc<ShapedText>],
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
            rubies: &[],
            labels: &[],
        }
    }

    #[must_use]
    pub fn with_rubies(mut self, rubies: &'a [Arc<ShapedText>]) -> Self {
        self.rubies = rubies;
        self
    }

    #[must_use]
    pub fn with_labels(mut self, labels: &'a [Arc<ShapedText>]) -> Self {
        self.labels = labels;
        self
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
    /// Layouts that asked for a vertical writing mode and were laid out
    /// horizontally instead: editable text, which does not yet edit in
    /// columns (#59).
    pub vertical_writing_fallbacks: usize,
    /// Break or adjustment opportunities the CJK line decision looked at
    /// (Issue #211), and the keep-versus-break comparisons it made.
    pub line_opportunities_considered: usize,
    pub line_break_comparisons: usize,
    /// The widest beam a pretty paragraph held, and paragraphs whose beam
    /// ran out of budget and finished greedily.
    pub line_beam_states: usize,
    pub line_budget_fallbacks: usize,
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
            request.source.objects(),
            request
                .source
                .rubies()
                .iter()
                .zip(request.rubies)
                .map(|(ruby, shaped)| (ruby.range.clone(), Arc::clone(shaped)))
                .collect(),
            request
                .source
                .labels()
                .iter()
                .zip(request.labels)
                .map(|(label, shaped)| (label.offset, Arc::clone(shaped)))
                .collect(),
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
        let sized = sized_runs(request, scale);
        let (widths, candidates) = Builder::new(line_input(
            request,
            sized.as_deref(),
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
        let sized = sized_runs(request, request.constraints.scale.px_per_logical);
        let input = line_input(
            request,
            sized.as_deref(),
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
        self.counters.line_opportunities_considered += work.opportunities_considered;
        self.counters.line_break_comparisons += work.break_comparisons;
        self.counters.line_beam_states = self.counters.line_beam_states.max(work.beam_states);
        self.counters.line_budget_fallbacks += work.budget_fallbacks;

        let unsupported_writing_mode =
            request.constraints.wants_vertical_writing() && !request.shaped.vertical;
        if unsupported_writing_mode {
            self.counters.vertical_writing_fallbacks += 1;
        }

        let bounds = laid_out
            .lines
            .iter()
            .map(|line| line.bounds)
            .reduce(TextRect::union)
            .unwrap_or_default();
        let objects = placed_objects(request, &laid_out.lines, &laid_out.runs);
        let rubies = placed_rubies(request, &laid_out.lines, &laid_out.runs);
        let rubies_dropped = !request.source.rubies().is_empty()
            && (request.rubies.len() != request.source.rubies().len() || request.shaped.vertical);
        let labels = placed_labels(request, &objects, &laid_out.lines);

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
            objects,
            rubies,
            rubies_dropped,
            labels,
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

/// The shaped runs with every inline object's placeholder given its box,
/// or `None` when the source has no objects (the runs are read as shaped).
///
/// This is where an object's size enters: after shaping and inside the
/// layout cache's key (the source revision moves with the objects), so
/// resizing a sticker relays the paragraph out from the runs already shaped.
fn sized_runs(request: &LayoutRequest<'_>, scale: f32) -> Option<Vec<ShapedRun>> {
    let rubies = ruby_boxes(request);
    if request.source.objects().is_empty() && rubies.is_empty() {
        return None;
    }
    let mut runs = request.shaped.runs.clone();
    // A base narrower than its annotation is spaced out to it: its glyphs
    // move right by half the difference and the last of them takes the rest
    // of it, so the base stays centred under the annotation and the text
    // after it moves on by exactly the difference.
    for ruby in &rubies {
        let base: f32 = runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .filter(|glyph| ruby.range.contains(&(glyph.cluster as usize)))
            .map(|glyph| glyph.advance_px)
            .sum();
        let extra = ruby.width_px - base;
        if extra <= 0.0 {
            continue;
        }
        let last = runs
            .iter()
            .enumerate()
            .flat_map(|(run, item)| {
                item.glyphs
                    .iter()
                    .enumerate()
                    .map(move |(glyph, shaped)| (run, glyph, shaped.cluster))
            })
            .filter(|(_, _, cluster)| ruby.range.contains(&(*cluster as usize)))
            .max_by_key(|(_, _, cluster)| *cluster);
        for run in runs.iter_mut() {
            for glyph in run.glyphs.iter_mut() {
                if ruby.range.contains(&(glyph.cluster as usize)) {
                    glyph.offset_x_px += extra * 0.5;
                }
            }
        }
        if let Some((run, glyph, _)) = last {
            runs[run].glyphs[glyph].advance_px += extra;
            runs[run].advance_px += extra;
        }
    }
    for run in runs.iter_mut().filter(|run| run.is_object()) {
        let metrics = request
            .source
            .object_at(run.source.start)
            .map(|object| object.metrics)
            .unwrap_or_default();
        let [width, ascent, descent] = match label_box(request, run.source.start) {
            Some(label) => [label.width_px, label.ascent_px, label.descent_px],
            None => [
                metrics.width_px.max(0.0) * scale,
                metrics.ascent_px.max(0.0) * scale,
                metrics.descent_px.max(0.0) * scale,
            ],
        };
        // The envelope's gap travels with the object: the line decision sees
        // one cell, and may close part of it up or drop it at a line end.
        let width = width
            + metrics
                .envelope
                .map_or(0.0, |envelope| envelope.gap_px.max(0.0) * scale);
        if let Some(glyph) = run.glyphs.first_mut() {
            glyph.advance_px = width;
        }
        run.advance_px = width;
        run.metrics.ascent_px = ascent;
        run.metrics.descent_px = descent;
    }
    Some(runs)
}

/// A labelled object's box, physical px: the label and its padding.
struct LabelBox {
    width_px: f32,
    ascent_px: f32,
    descent_px: f32,
    /// From the object's left edge to the label's pen.
    inset_px: f32,
    /// Room kept clear either side of the tag.
    gap_px: f32,
}

fn label_box(request: &LayoutRequest<'_>, offset: usize) -> Option<LabelBox> {
    let shaped = request.labels.get(request.source.label_index(offset)?)?;
    let width: f32 = shaped.runs.iter().map(|run| run.advance_px).sum();
    let size = shaped
        .runs
        .iter()
        .map(|run| run.font_size_px * request.constraints.scale.px_per_logical)
        .fold(0.0f32, f32::max);
    let ascent = shaped
        .runs
        .iter()
        .map(|run| run.metrics.ascent_px)
        .fold(0.0f32, f32::max);
    let descent = shaped
        .runs
        .iter()
        .map(|run| run.metrics.descent_px)
        .fold(0.0f32, f32::max);
    let pad_x = (size * 0.45).round();
    let pad_y = (size * 0.18).round();
    // A small gap either side keeps neighbouring text off the tag.
    let gap = (size * 0.2).round();
    Some(LabelBox {
        width_px: width + 2.0 * (pad_x + gap),
        ascent_px: ascent + pad_y,
        descent_px: descent + pad_y,
        inset_px: pad_x + gap,
        gap_px: gap,
    })
}

/// Each object label placed inside its object, on the object's baseline.
fn placed_labels(
    request: &LayoutRequest<'_>,
    objects: &[super::ir::PlacedObject],
    lines: &[super::ir::LineBox],
) -> Vec<super::ir::PlacedLabel> {
    if request.labels.is_empty() {
        return Vec::new();
    }
    let mut placed = Vec::new();
    for (label, shaped) in request.source.labels().iter().zip(request.labels) {
        let Some(object) = objects.iter().find(|object| object.offset == label.offset) else {
            continue;
        };
        let Some(boxed) = label_box(request, label.offset) else {
            continue;
        };
        let Some(line) = lines.get(object.line as usize) else {
            continue;
        };
        let mut cursor = object.rect.x + boxed.inset_px;
        let runs = shaped
            .runs
            .iter()
            .map(|run| {
                let mut run = run.clone();
                run.origin_x_px = cursor;
                cursor += run.advance_px;
                run
            })
            .collect();
        placed.push(super::ir::PlacedLabel {
            offset: label.offset,
            line: object.line,
            baseline_y_px: line.metrics.baseline_y_px,
            rect: TextRect::new(
                object.rect.x + boxed.gap_px,
                object.rect.y,
                (object.rect.width - 2.0 * boxed.gap_px).max(0.0),
                object.rect.height,
            ),
            runs,
        });
    }
    placed
}

/// An annotation's size: how wide it is and how tall it stands, physical px.
pub(super) struct RubyBox {
    pub range: std::ops::Range<usize>,
    pub width_px: f32,
    pub height_px: f32,
}

/// The box of each annotation the request carries.
pub(super) fn ruby_boxes(request: &LayoutRequest<'_>) -> Vec<RubyBox> {
    request
        .source
        .rubies()
        .iter()
        .zip(request.rubies)
        .map(|(ruby, shaped)| {
            let width_px = shaped.runs.iter().map(|run| run.advance_px).sum();
            let height_px = shaped
                .runs
                .iter()
                .map(|run| run.metrics.ascent_px + run.metrics.descent_px)
                .fold(0.0f32, f32::max);
            RubyBox {
                range: ruby.range.clone(),
                width_px,
                height_px,
            }
        })
        .collect()
}

/// Each annotation placed centred above its base, on the line the base
/// starts on, sitting on the base's ascent.
fn placed_rubies(
    request: &LayoutRequest<'_>,
    lines: &[super::ir::LineBox],
    runs: &[ShapedRun],
) -> Vec<super::ir::PlacedRuby> {
    if request.rubies.is_empty() || request.shaped.vertical {
        return Vec::new();
    }
    let mut placed = Vec::new();
    for (ruby, shaped) in request.source.rubies().iter().zip(request.rubies) {
        let Some(line) = lines.iter().find(|line| {
            line.source.start <= ruby.range.start && ruby.range.start < line.source.end
        }) else {
            continue;
        };
        let line_runs = &runs[line.runs.start as usize..line.runs.end as usize];
        let mut left = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut ascent: f32 = 0.0;
        for run in line_runs {
            let mut pen = run.origin_x_px;
            let mut touched = false;
            for glyph in &run.glyphs {
                if ruby.range.contains(&(glyph.cluster as usize)) {
                    left = left.min(pen);
                    right = right.max(pen + glyph.advance_px);
                    touched = true;
                }
                pen += glyph.advance_px;
            }
            if touched {
                ascent = ascent.max(run.metrics.ascent_px);
            }
        }
        if !left.is_finite() {
            continue;
        }
        let width: f32 = shaped.runs.iter().map(|run| run.advance_px).sum();
        let descent = shaped
            .runs
            .iter()
            .map(|run| run.metrics.descent_px)
            .fold(0.0f32, f32::max);
        let height = shaped
            .runs
            .iter()
            .map(|run| run.metrics.ascent_px + run.metrics.descent_px)
            .fold(0.0f32, f32::max);
        let baseline = line.metrics.baseline_y_px - ascent - descent;
        let x = (left + right) * 0.5 - width * 0.5;
        let mut cursor = x;
        let placed_runs = shaped
            .runs
            .iter()
            .map(|run| {
                let mut run = run.clone();
                run.origin_x_px = cursor;
                cursor += run.advance_px;
                run
            })
            .collect();
        placed.push(super::ir::PlacedRuby {
            range: ruby.range.clone(),
            line: line.index,
            baseline_y_px: baseline,
            rect: TextRect::new(x, baseline - (height - descent), width, height),
            runs: placed_runs,
        });
    }
    placed
}

/// Each object placeholder that was placed, as the box it takes on its line.
fn placed_objects(
    request: &LayoutRequest<'_>,
    lines: &[super::ir::LineBox],
    runs: &[ShapedRun],
) -> Vec<super::ir::PlacedObject> {
    let source = request.source;
    if source.objects().is_empty() {
        return Vec::new();
    }
    let mut placed = Vec::new();
    for line in lines {
        for run in &runs[line.runs.start as usize..line.runs.end as usize] {
            if !run.is_object() {
                continue;
            }
            let Some(object) = source.object_at(run.source.start) else {
                continue;
            };
            let ascent = run.metrics.ascent_px;
            // An envelope's gap is room after the object, not the object: the
            // box keeps its own width and the gap, closed up or not, follows
            // it in the line's direction.
            let (x, width) = match object.metrics.envelope {
                Some(_) if label_box(request, object.offset).is_none() => {
                    let own = (object.metrics.width_px.max(0.0)
                        * request.constraints.scale.px_per_logical)
                        .min(run.advance_px);
                    let x = match run.direction {
                        RunDirection::Rtl => run.origin_x_px + run.advance_px - own,
                        _ => run.origin_x_px,
                    };
                    (x, own)
                }
                _ => (run.origin_x_px, run.advance_px),
            };
            placed.push(super::ir::PlacedObject {
                id: object.id,
                offset: object.offset,
                line: line.index,
                rect: TextRect::new(
                    x,
                    line.metrics.baseline_y_px - ascent,
                    width,
                    ascent + run.metrics.descent_px,
                ),
            });
        }
    }
    placed.sort_by_key(|object| object.offset);
    placed
}

fn line_input<'r>(
    request: &'r LayoutRequest<'_>,
    sized: Option<&'r [ShapedRun]>,
    strut: Option<LineStrut>,
    run_line_heights: &'r [f32],
    base_line_height: f32,
    ellipsis: Option<&'r Ellipsis>,
) -> LineInput<'r> {
    LineInput {
        text: request.source.text(),
        runs: sized.unwrap_or(&request.shaped.runs),
        rubies: ruby_boxes(request),
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
        vertical: request.shaped.vertical,
        envelopes: request
            .source
            .objects()
            .iter()
            .filter_map(|object| {
                object
                    .metrics
                    .envelope
                    .map(|envelope| (object.offset, envelope))
            })
            .collect(),
    }
}

/// When a label may skip the paragraph machinery entirely.
///
/// A label degrades to the paragraph path as soon as it stops being one line of
/// plain text: any wrap mode, a multi-line or zero `max_lines`, a stacking
/// budget (the box's height, or its width for a vertical label), or a source
/// the paragraph path would cut into more than one segment.
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
        && request
            .constraints
            .block_budget_px(request.shaped.vertical)
            .is_none()
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
