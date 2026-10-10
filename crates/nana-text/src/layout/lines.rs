//! Line breaking, visual ordering, metrics, alignment, truncation.
//!
//! Everything here runs on shaped advances: no width decision is taken on a
//! codepoint count, and nothing re-enters the shaper — not even the ellipsis,
//! which arrives already shaped.

use super::breaks;
use super::engine::IntrinsicWidths;
use super::ir::{LineBox, LineBreakCause, OverflowFlags, TextRect};
use crate::constraints::TextConstraints;
use crate::metrics::{LineMetrics, RunMetrics};
use crate::shape::GlyphFlags;
use crate::shape::{RunDirection, ShapedRun};
use crate::shaping::{PARAGRAPH_SEPARATORS, ShapedParagraph, ShapedText, bidi_visual_order};
use nana_ui_core::{
    DirSpec, LineBreakSpec, TextAlignSpec, TextAutospaceSpec, TextJustifySpec, TextSpacingTrimSpec,
    TextWrapBreak, TextWrapStyleSpec, WordBreakSpec,
};
use std::ops::Range;

/// Slack for a width comparison, in px.
///
/// A line's width is a sum of f32 advances while the limit is a single f32, so
/// a line that exactly fills its box can land a few ULPs either side. A real
/// overflow is orders of magnitude larger; without this, "fits exactly" would
/// wrap at random.
const WIDTH_EPSILON_PX: f32 = 0.01;

/// The font metrics every line box starts from, whatever its runs use.
///
/// This is CSS's strut, and it is what holds a baseline still: with a strut,
/// one emoji falling back to a taller face grows the reported ascent but does
/// **not** move the baseline, so `Save` and `Save 🔥` sit on the same line.
/// Without one, each line's baseline is centred on that line's own tallest run,
/// and it moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStrut {
    pub metrics: RunMetrics,
    /// Line box height the base style asks for, in physical px.
    pub line_height_px: f32,
}

/// A shaped ellipsis, ready to be placed at a truncation point.
pub(super) struct Ellipsis {
    runs: Vec<ShapedRun>,
    advance_px: f32,
}

impl Ellipsis {
    /// Takes the shaped `…` as it came out of the shaper and cache.
    ///
    /// Returns `None` for an ellipsis that shaped to nothing, which would
    /// otherwise claim a truncation happened and draw nothing.
    pub fn new(shaped: &ShapedText) -> Option<Self> {
        if shaped.runs.is_empty() {
            return None;
        }
        let advance_px = shaped.runs.iter().map(|run| run.advance_px).sum();
        Some(Self {
            runs: shaped.runs.clone(),
            advance_px,
        })
    }

    /// The runs, re-anchored to a zero-length cluster at the cut.
    ///
    /// The ellipsis is not part of the source text, so it must not claim any of
    /// its bytes: an empty range at the cut keeps hit-testing, caret placement
    /// and selection from ever resolving onto it.
    fn placed(&self, at: usize) -> Vec<ShapedRun> {
        self.runs
            .iter()
            .map(|run| {
                let mut run = run.clone();
                run.source = at..at;
                for glyph in &mut run.glyphs {
                    glyph.cluster = at as u32;
                    glyph.cluster_end = at as u32;
                }
                run
            })
            .collect()
    }
}

/// How a paragraph may break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakPolicy {
    /// No wrapping: only paragraph separators end a line.
    None,
    /// UAX #14 opportunities only. A word wider than the box overflows.
    Word,
    /// UAX #14 first, then any cluster boundary for a word that cannot fit.
    WordThenGlyph,
    /// Any cluster boundary.
    Glyph,
}

impl BreakPolicy {
    fn of(constraints: &TextConstraints) -> Self {
        let Some(wrap) = constraints.wrap else {
            return Self::None;
        };
        if wrap == TextWrapBreak::Glyph
            || constraints.word_break == WordBreakSpec::BreakAll
            || constraints.line_break == LineBreakSpec::Anywhere
        {
            return Self::Glyph;
        }
        if wrap == TextWrapBreak::WordOrGlyph || constraints.word_break == WordBreakSpec::BreakWord
        {
            return Self::WordThenGlyph;
        }
        Self::Word
    }

    fn breaks_inside_a_word(self) -> bool {
        matches!(self, Self::WordThenGlyph | Self::Glyph)
    }
}

/// One cluster of the source, in logical order, with the glyphs drawing it.
///
/// The unit of line breaking and of truncation, which is what makes both
/// cluster-safe: a cut between two cells cannot land inside a grapheme
/// cluster, a ligature or a UTF-8 sequence, because the shaper never split one
/// across cells to begin with.
#[derive(Debug, Clone)]
struct Cell {
    start: usize,
    end: usize,
    advance_px: f32,
    run: usize,
    glyphs: Range<usize>,
    whitespace: bool,
}

/// What the layout engine hands the line builder.
pub(super) struct LineInput<'a> {
    pub text: &'a str,
    pub runs: &'a [ShapedRun],
    pub paragraphs: &'a [ShapedParagraph],
    /// Resolved line box height per entry of `runs`, physical px.
    pub run_line_heights: &'a [f32],
    pub constraints: &'a TextConstraints,
    pub strut: Option<LineStrut>,
    /// Line box height for a line with no runs to measure, physical px.
    pub empty_line_height_px: f32,
    pub base_direction: RunDirection,
    pub ellipsis: Option<&'a Ellipsis>,
    /// Ruby annotations: no line breaks inside a base, and room above it.
    pub rubies: Vec<super::engine::RubyBox>,
    /// Lines run top to bottom and stack across (#59). The runs were shaped
    /// for it, so every advance is already a length along the line; what
    /// changes here is which box dimension budgets what, and where the
    /// baseline sits.
    pub vertical: bool,
    /// The envelope of each inline object that declared one, by source
    /// offset, in logical px.
    pub envelopes: Vec<(usize, crate::InlineEnvelope)>,
}

/// Work the line builder did, for the counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct LineWork {
    pub line_break_candidates: usize,
    pub lines_created: usize,
    pub runs_placed: usize,
    pub ellipsis_runs_used: usize,
    pub shape_runs_reused: usize,
    /// Break or adjustment opportunities a line decision looked at.
    pub opportunities_considered: usize,
    /// Keep-versus-break comparisons a line decision made.
    pub break_comparisons: usize,
    /// The widest beam a pretty paragraph held, and the paragraphs whose
    /// beam ran out of budget and finished greedily.
    pub beam_states: usize,
    pub budget_fallbacks: usize,
}

pub(super) struct LaidOut {
    pub runs: Vec<ShapedRun>,
    pub lines: Vec<LineBox>,
    pub overflow: OverflowFlags,
    pub work: LineWork,
}

/// A line measured but not yet placed.
struct Prepared {
    cells: Range<usize>,
    pieces: Vec<Piece>,
    /// Width **without** the trailing whitespace, which hangs.
    ///
    /// `choose_break` already ignores trailing spaces when it asks whether a
    /// line fits, so everything downstream has to ignore them too, or `"Save "`
    /// would overflow — and be cut to `"Sa…"` — in a box that fits `"Save"`. It
    /// would also sit off-centre next to `"Save"`, because alignment divides the
    /// slack the width leaves.
    width_px: f32,
    ascent_px: f32,
    descent_px: f32,
    height_px: f32,
    /// How far an inline object taller than the line's text pushes the
    /// baseline down from where the text alone would put it.
    lift_px: f32,
    /// What the line decision changed on this line's cells.
    adjust: LineAdjust,
}

/// A line already placed, and what re-placing it would have to undo.
struct PlacedLine {
    cells: Range<usize>,
    ellipsis_runs: usize,
    /// Byte the line starts at, for a line with no cells to read it from.
    anchor: usize,
}

pub(super) struct Builder<'a> {
    input: LineInput<'a>,
    cells: Vec<Cell>,
    /// `prefix[i]` is the advance of `cells[..i]`.
    prefix: Vec<f32>,
    policy: BreakPolicy,
    max_width_px: Option<f32>,
    max_height_px: Option<f32>,
    runs: Vec<ShapedRun>,
    lines: Vec<LineBox>,
    /// One record per placed line, so the last one can be re-placed with an
    /// ellipsis — and every counter it moved rolled back — when a later line
    /// turns out not to fit.
    placed_lines: Vec<PlacedLine>,
    next_top_px: f32,
    truncated: bool,
    ellipsized: bool,
    work: LineWork,
    /// The CJK line decision's view of the cells, when any of it applies.
    typo: Option<Typography>,
}

impl<'a> Builder<'a> {
    pub fn new(input: LineInput<'a>) -> Self {
        let mut cells = cells(input.text, input.runs);
        let typo = Typography::build(
            input.text,
            input.runs,
            &mut cells,
            input.constraints,
            input.vertical,
            &input.envelopes,
        );
        let mut prefix = Vec::with_capacity(cells.len() + 1);
        let mut total = 0.0;
        prefix.push(0.0);
        for cell in &cells {
            total += cell.advance_px;
            prefix.push(total);
        }
        let scale = input.constraints.scale.px_per_logical;
        // `max_width_px` / `max_height_px` below are the line budget and the
        // stacking budget: the box's width and height for horizontal lines,
        // its height and width for vertical ones.
        let max_width_px = input
            .constraints
            .inline_budget_px(input.vertical)
            .map(|width| width * scale);
        let max_height_px = input
            .constraints
            .block_budget_px(input.vertical)
            .map(|height| height * scale);
        let policy = BreakPolicy::of(input.constraints);
        Self {
            input,
            cells,
            prefix,
            policy,
            max_width_px,
            max_height_px,
            runs: Vec::new(),
            lines: Vec::new(),
            placed_lines: Vec::new(),
            next_top_px: 0.0,
            truncated: false,
            ellipsized: false,
            work: LineWork::default(),
            typo,
        }
    }

    /// Every cluster on one line: the Label fast path.
    ///
    /// No break opportunity is looked for, no paragraph structure is walked and
    /// no per-line buffer is kept. A short label costs one pass over its own
    /// runs.
    pub fn single_line(mut self) -> LaidOut {
        let count = self.cells.len();
        let start = self.cells.first().map_or(0, |cell| cell.start);
        let prepared = self.prepare(0..count);
        self.place(prepared, LineBreakCause::EndOfText, start);
        self.finish()
    }

    /// `min-content` and `max-content`, plus the break candidates it looked at.
    ///
    /// Both are measured with wrapping assumed allowed at every UAX #14
    /// opportunity, whatever these constraints say: a container asks for these
    /// numbers precisely to decide what width to then impose. Both also stop at
    /// the same segment boundaries layout breaks at, so `max-content` is the
    /// widest line the text can produce, not the width of everything joined.
    pub fn intrinsic_widths(mut self) -> (IntrinsicWidths, usize) {
        let mut widths = IntrinsicWidths::default();
        for content in self.segments() {
            let (lo, hi) = self.cell_span(&content);
            if lo == hi {
                continue;
            }
            let trimmed = self.trim(lo, hi);
            widths.max_px = widths.max_px.max(self.width(lo, trimmed));
            let stops = self.break_stops(&content, BreakPolicy::Word, lo, hi);
            let mut start = lo;
            for stop in stops.iter().copied().chain(std::iter::once(hi)) {
                let trimmed = self.trim(start, stop);
                widths.min_px = widths.min_px.max(self.width(start, trimmed));
                start = stop;
            }
        }
        (widths, self.work.line_break_candidates)
    }

    /// The full paragraph path: break opportunities, wrapping, hard breaks.
    pub fn paragraphs(mut self) -> LaidOut {
        let segments = self.segments();
        let last = segments.len() - 1;
        for (index, content) in segments.into_iter().enumerate() {
            if self.truncated {
                break;
            }
            let cause = if index == last {
                LineBreakCause::EndOfText
            } else {
                LineBreakCause::Explicit
            };
            self.segment(content, cause);
        }
        self.finish()
    }

    fn segments(&self) -> Vec<Range<usize>> {
        segments(self.input.text, self.input.paragraphs)
    }

    /// One run of text with no forced break inside it: as many lines as the
    /// width allows.
    fn segment(&mut self, content: Range<usize>, end_cause: LineBreakCause) {
        let policy = self.policy;
        let (lo, hi) = self.cell_span(&content);

        // An empty segment — two newlines in a row, a trailing one, or a
        // forced break with nothing after it — still occupies a line box, and
        // that line still starts at its own byte so a caret can land on it.
        if lo == hi {
            let prepared = self.prepare(lo..hi);
            self.place_within_budget(prepared, end_cause, content.start);
            return;
        }

        let stops = self.break_stops(&content, policy, lo, hi);
        if self.typo.is_some() {
            self.segment_typo(lo, hi, &stops, policy, end_cause);
            return;
        }
        let mut start = lo;
        let mut next_stop = 0;
        while start < hi {
            while next_stop < stops.len() && stops[next_stop] <= start {
                next_stop += 1;
            }
            let (end, soft) = self.choose_break(start, hi, &stops, next_stop, policy);
            let cause = if soft {
                LineBreakCause::Wrap
            } else {
                end_cause
            };
            // A soft wrap hangs the whitespace it broke at: it is not drawn,
            // not counted in the line width, and does not push the next line
            // along. At a hard break the whitespace is authored content and
            // stays on the line.
            let emitted = if soft { self.trim(start, end) } else { end };
            let at = self.cells[start].start;
            let prepared = self.prepare(start..emitted);
            if !self.place_within_budget(prepared, cause, at) {
                return;
            }
            start = end;
        }
    }

    /// Cell indices a line may start at, inside `content`.
    fn break_stops(
        &mut self,
        content: &Range<usize>,
        policy: BreakPolicy,
        lo: usize,
        hi: usize,
    ) -> Vec<usize> {
        match policy {
            // Nothing to scan for: the paragraph is the line.
            BreakPolicy::None => Vec::new(),
            // Every cluster boundary is a stop, so UAX #14 has nothing to add.
            BreakPolicy::Glyph => {
                self.work.line_break_candidates += hi.saturating_sub(lo + 1);
                let stops = (lo + 1..hi).collect();
                match &self.typo {
                    Some(typo) => typo.tailor(self.input.text, &self.cells, stops, lo, hi, true),
                    None => stops,
                }
            }
            BreakPolicy::Word | BreakPolicy::WordThenGlyph => {
                let offsets =
                    breaks::opportunities(&self.input.text[content.clone()], content.start);
                self.work.line_break_candidates += offsets.len();
                let stops = offsets
                    .iter()
                    .filter_map(|offset| self.cell_at(*offset))
                    .filter(|index| *index > lo && *index < hi)
                    .filter(|index| !self.inside_ruby(*index))
                    .collect();
                match &self.typo {
                    Some(typo) => typo.tailor(self.input.text, &self.cells, stops, lo, hi, false),
                    None => stops,
                }
            }
        }
    }

    /// Whether a line starting at cell `index` would split a ruby base.
    fn inside_ruby(&self, index: usize) -> bool {
        let Some(cell) = self.cells.get(index) else {
            return false;
        };
        self.input
            .rubies
            .iter()
            .any(|ruby| ruby.range.start < cell.start && cell.start < ruby.range.end)
    }

    /// Where the line starting at `start` ends, and whether that was a wrap.
    fn choose_break(
        &self,
        start: usize,
        hi: usize,
        stops: &[usize],
        next_stop: usize,
        policy: BreakPolicy,
    ) -> (usize, bool) {
        let Some(max_width) = self.max_width_px.filter(|_| policy != BreakPolicy::None) else {
            return (hi, false);
        };
        let mut best: Option<usize> = None;
        let mut probe = next_stop;
        loop {
            let stop = stops.get(probe).copied().unwrap_or(hi);
            let drawn = self.trim(start, stop);
            // A break that leaves nothing drawn is not a break. Whitespace at
            // the start of a segment has a break opportunity after it, and
            // taking it would open the paragraph with a blank line and hand the
            // whitespace nowhere to go.
            if drawn == start && stop != hi {
                probe += 1;
                continue;
            }
            let width = self.width(start, drawn);
            if width <= max_width + WIDTH_EPSILON_PX {
                if stop == hi {
                    return (hi, false);
                }
                best = Some(stop);
                probe += 1;
                continue;
            }
            return match best {
                // The last opportunity that fitted.
                Some(fit) => (fit, true),
                // Nothing fits. Either cut inside the word, or let it overflow
                // to the next opportunity: `word-break: normal` says a long
                // word sticks out rather than being sliced.
                None if policy.breaks_inside_a_word() => {
                    (self.emergency(start, stop, max_width), true)
                }
                None => (stop, stop != hi),
            };
        }
    }

    /// [`Self::segment`] under the CJK line decision: each line chosen by
    /// [`Self::decide_line`], or for a pretty paragraph by a bounded beam,
    /// and set with what that decision changed.
    fn segment_typo(
        &mut self,
        lo: usize,
        hi: usize,
        stops: &[usize],
        policy: BreakPolicy,
        end_cause: LineBreakCause,
    ) {
        let pretty = self.typo.as_ref().is_some_and(|typo| typo.pretty)
            && self.max_width_px.is_some()
            && policy != BreakPolicy::None;
        let ends: Vec<usize> = if pretty {
            self.beam_breaks(lo, hi, stops, policy)
        } else {
            Vec::new()
        };
        let mut start = lo;
        let mut next_stop = 0;
        let mut line = 0;
        while start < hi {
            while next_stop < stops.len() && stops[next_stop] <= start {
                next_stop += 1;
            }
            let first = start == lo;
            let decision = match ends.get(line) {
                Some(&end) => {
                    let (fit, _) = self.fit_of(start, end, first);
                    let fit = if matches!(fit, Fit::No) {
                        Fit::Fits { end_closed: false }
                    } else {
                        fit
                    };
                    (end, end != hi, fit)
                }
                None => self.decide_line(start, hi, stops, next_stop, policy, first),
            };
            let (end, soft, fit) = decision;
            let cause = if soft {
                LineBreakCause::Wrap
            } else {
                end_cause
            };
            let emitted = if soft { self.trim(start, end) } else { end };
            let at = self.cells[start].start;
            let adjust = self.adjust_for(start, end, soft, first, fit);
            let prepared = self.prepare_with(start..emitted, adjust);
            if !self.place_within_budget(prepared, cause, at) {
                return;
            }
            start = end;
            line += 1;
        }
    }

    /// How the line `start..stop` fits its box, and its drawn end.
    fn fit_of(&self, start: usize, stop: usize, first: bool) -> (Fit, usize) {
        let drawn = self.trim(start, stop);
        let (Some(typo), Some(max)) = (&self.typo, self.max_width_px) else {
            return (Fit::Fits { end_closed: false }, drawn);
        };
        if drawn == start {
            return (Fit::Fits { end_closed: false }, drawn);
        }
        let last = drawn - 1;
        let (always, optional) = typo.end_trims(last);
        let width = self.width(start, drawn) - typo.start_trim(start, first) - always;
        if width <= max + WIDTH_EPSILON_PX {
            return (Fit::Fits { end_closed: false }, drawn);
        }
        if optional > 0.0 && width - optional <= max + WIDTH_EPSILON_PX {
            return (Fit::Fits { end_closed: true }, drawn);
        }
        if typo.cost_fit {
            let deficit = width - optional - max;
            if deficit <= typo.interior_squeeze(start, last) + WIDTH_EPSILON_PX {
                return (Fit::Squeezed { deficit }, drawn);
            }
        }
        (Fit::No, drawn)
    }

    /// The width the line `start..stop` sets at once fitted as `fit`.
    fn fitted_width(&self, start: usize, stop: usize, first: bool, fit: Fit) -> f32 {
        let drawn = self.trim(start, stop);
        let Some(typo) = &self.typo else {
            return self.width(start, drawn);
        };
        if drawn == start {
            return 0.0;
        }
        let (always, optional) = typo.end_trims(drawn - 1);
        let mut width = self.width(start, drawn) - typo.start_trim(start, first) - always;
        match fit {
            Fit::Fits { end_closed: true } => width -= optional,
            Fit::Squeezed { deficit } => width -= optional + deficit,
            _ => {}
        }
        width
    }

    /// One line's choice: break at the last stop that fits as set, or keep
    /// later stops by closing punctuation and autospace up, whichever costs
    /// less -- the slack a break leaves, squared, against what closing up
    /// costs. A tie keeps the later stop. With nothing that fits either way,
    /// as [`Self::choose_break`].
    fn decide_line(
        &mut self,
        start: usize,
        hi: usize,
        stops: &[usize],
        next_stop: usize,
        policy: BreakPolicy,
        first: bool,
    ) -> (usize, bool, Fit) {
        let Some(max_width) = self.max_width_px.filter(|_| policy != BreakPolicy::None) else {
            return (hi, false, Fit::Fits { end_closed: false });
        };
        let mut best: Option<(usize, Fit)> = None;
        let mut squeezed: Vec<(usize, f32)> = Vec::new();
        let mut probe = next_stop;
        loop {
            let stop = stops.get(probe).copied().unwrap_or(hi);
            let drawn = self.trim(start, stop);
            if drawn == start && stop != hi {
                probe += 1;
                continue;
            }
            self.work.opportunities_considered += 1;
            match self.fit_of(start, stop, first).0 {
                fit @ Fit::Fits { .. } => {
                    if stop == hi {
                        return (hi, false, fit);
                    }
                    best = Some((stop, fit));
                }
                Fit::Squeezed { deficit } => squeezed.push((stop, deficit)),
                Fit::No => break,
            }
            if stop == hi {
                break;
            }
            probe += 1;
        }
        let em = self.typo.as_ref().map_or(16.0, |typo| typo.em[start]);
        let mut chosen: Option<(u64, usize, Fit)> = best.map(|(stop, fit)| {
            let slack = max_width - self.fitted_width(start, stop, first, fit);
            (Typography::raggedness(slack, em), stop, fit)
        });
        for (stop, deficit) in squeezed {
            self.work.break_comparisons += 1;
            let cost = Typography::squeeze_cost(deficit);
            if chosen.is_none_or(|(held, _, _)| cost <= held) {
                chosen = Some((cost, stop, Fit::Squeezed { deficit }));
            }
        }
        match chosen {
            Some((_, stop, fit)) => (stop, stop != hi, fit),
            None => {
                let (end, soft) = self.choose_break(start, hi, stops, next_stop, policy);
                (end, soft, Fit::Fits { end_closed: false })
            }
        }
    }

    /// What the line `start..end` changes on its cells: an opening mark's
    /// blank at its start, its end's autospace and (when it needs it) a
    /// closing mark's blank, the deficit a squeezed line closes across its
    /// interior shared by capacity, and a justified line's slack shared
    /// across its word gaps and CJK character boundaries.
    fn adjust_for(
        &self,
        start: usize,
        end: usize,
        soft: bool,
        first: bool,
        fit: Fit,
    ) -> LineAdjust {
        let mut adjust = LineAdjust::default();
        let (Some(typo), Some(max)) = (&self.typo, self.max_width_px) else {
            return adjust;
        };
        let drawn = self.trim(start, end);
        if drawn == start {
            return adjust;
        }
        let last = drawn - 1;
        let lead = typo.start_trim(start, first);
        if lead > 0.0 {
            adjust.deltas.push((start, -lead, -lead));
        }
        let (always, optional) = typo.end_trims(last);
        if always > 0.0 {
            adjust.deltas.push((last, -always, 0.0));
        }
        let closes_end = matches!(fit, Fit::Fits { end_closed: true } | Fit::Squeezed { .. });
        if closes_end && optional > 0.0 {
            adjust.deltas.push((last, -optional, 0.0));
        }
        if let Fit::Squeezed { deficit } = fit {
            // Every interior capacity costs the same: share the deficit by
            // capacity, in 1/64 px, the remainder one unit at a time in order.
            let items: Vec<(usize, f32, bool)> = (start + 1..last)
                .flat_map(|at| {
                    [
                        (at, typo.give_after[at], false),
                        (at, typo.give_before[at], true),
                    ]
                })
                .filter(|(_, capacity, _)| *capacity > 0.0)
                .collect();
            let units = |px: f32| nana_ui_core::dynamic_layout::LayoutUnits::from_px(px).0 as i64;
            let total: i64 = items.iter().map(|(_, capacity, _)| units(*capacity)).sum();
            let want = units(deficit).min(total);
            if total > 0 {
                let mut shares: Vec<i64> = items
                    .iter()
                    .map(|(_, capacity, _)| want * units(*capacity) / total)
                    .collect();
                let mut left = want - shares.iter().sum::<i64>();
                for (share, (_, capacity, _)) in shares.iter_mut().zip(&items) {
                    if left == 0 {
                        break;
                    }
                    if *share < units(*capacity) {
                        *share += 1;
                        left -= 1;
                    }
                }
                for ((at, _, leading), share) in items.iter().zip(shares) {
                    let px = share as f32 / 64.0;
                    adjust
                        .deltas
                        .push((*at, -px, if *leading { -px } else { 0.0 }));
                }
            }
        }
        if soft
            && let Some(justify) = typo.justify
            && justify != TextJustifySpec::None
        {
            let width = self.width(start, drawn) + adjust.width_delta(drawn);
            let slack = max - width;
            if slack > WIDTH_EPSILON_PX {
                let words = matches!(justify, TextJustifySpec::Auto | TextJustifySpec::InterWord);
                let characters = matches!(
                    justify,
                    TextJustifySpec::Auto | TextJustifySpec::InterCharacter
                );
                let gaps: Vec<usize> = (start..last)
                    .filter(|at| {
                        let cell = &self.cells[*at];
                        (words && cell.whitespace)
                            || (characters
                                && !cell.whitespace
                                && typo.cjk[*at]
                                && typo.cjk[at + 1]
                                && !self.cells[at + 1].whitespace)
                    })
                    .collect();
                if !gaps.is_empty() {
                    let share = slack / gaps.len() as f32;
                    adjust
                        .deltas
                        .extend(gaps.into_iter().map(|at| (at, share, 0.0)));
                }
            }
        }
        adjust
    }

    /// A pretty paragraph's breaks: a beam of at most `K` partial layouts,
    /// each extended by at most `C` candidate ends per line -- the last `C`
    /// stops that fit, set or squeezed -- and pruned by total cost (slack
    /// squared, plus what closing up cost). A last line costs nothing. Past
    /// its step budget the best partial layout finishes greedily, the same
    /// way every time.
    fn beam_breaks(
        &mut self,
        lo: usize,
        hi: usize,
        stops: &[usize],
        policy: BreakPolicy,
    ) -> Vec<usize> {
        const K: usize = 4;
        const C: usize = 4;
        let Some(max_width) = self.max_width_px else {
            return Vec::new();
        };
        // Arena of partial layouts: (end, parent, line index).
        let mut arena: Vec<(usize, usize, bool)> = vec![(lo, usize::MAX, true)];
        let mut beam: Vec<(u64, usize)> = vec![(0, 0)];
        let budget = 16 * stops.len() + 256;
        let mut steps = 0usize;
        let mut done: Option<(u64, usize)> = None;
        let mut fell_back = false;
        while !beam.is_empty() {
            let mut next: Vec<(u64, usize)> = Vec::new();
            for &(cost, node) in &beam {
                let (start, _, first) = arena[node];
                if start >= hi {
                    if done.is_none_or(|(held, _)| cost < held) {
                        done = Some((cost, node));
                    }
                    continue;
                }
                let next_stop = stops.partition_point(|stop| *stop <= start);
                let mut candidates: Vec<(usize, u64)> = Vec::new();
                let mut probe = next_stop;
                loop {
                    let stop = stops.get(probe).copied().unwrap_or(hi);
                    let drawn = self.trim(start, stop);
                    if drawn == start && stop != hi {
                        probe += 1;
                        continue;
                    }
                    steps += 1;
                    self.work.opportunities_considered += 1;
                    let fit = self.fit_of(start, stop, first).0;
                    if matches!(fit, Fit::No) {
                        break;
                    }
                    let line_cost = if stop == hi {
                        match fit {
                            Fit::Squeezed { deficit } => Typography::squeeze_cost(deficit),
                            _ => 0,
                        }
                    } else {
                        let em = self.typo.as_ref().map_or(16.0, |typo| typo.em[start]);
                        let slack = max_width - self.fitted_width(start, stop, first, fit);
                        match fit {
                            Fit::Squeezed { deficit } => Typography::squeeze_cost(deficit),
                            _ => Typography::raggedness(slack, em),
                        }
                    };
                    candidates.push((stop, line_cost));
                    if stop == hi {
                        break;
                    }
                    probe += 1;
                }
                if candidates.is_empty() {
                    // Nothing fits: the greedy choice (an emergency cut or an
                    // overflow) is the only end.
                    let (end, _) = self.choose_break(start, hi, stops, next_stop, policy);
                    candidates.push((end, Typography::raggedness(max_width, 16.0)));
                }
                let keep = candidates.len().saturating_sub(C);
                for (stop, line_cost) in candidates.into_iter().skip(keep) {
                    self.work.break_comparisons += 1;
                    arena.push((stop, node, false));
                    next.push((cost.saturating_add(line_cost), arena.len() - 1));
                }
            }
            // Dedupe by end, keep the K cheapest; ties keep the later end.
            next.sort_by_key(|(cost, node)| (arena[*node].0, *cost));
            next.dedup_by_key(|(_, node)| arena[*node].0);
            next.sort_by_key(|(cost, node)| (*cost, std::cmp::Reverse(arena[*node].0)));
            next.truncate(K);
            if let Some((held, _)) = done {
                next.retain(|(cost, _)| *cost < held);
            }
            self.work.beam_states = self.work.beam_states.max(next.len());
            if steps > budget {
                fell_back = true;
                if let Some(&(cost, node)) = next.first() {
                    done = Some((cost, node));
                }
                break;
            }
            beam = next;
        }
        let Some((_, mut node)) = done else {
            return Vec::new();
        };
        let mut ends = Vec::new();
        while arena[node].1 != usize::MAX {
            ends.push(arena[node].0);
            node = arena[node].1;
        }
        ends.reverse();
        if fell_back {
            self.work.budget_fallbacks += 1;
        }
        ends
    }

    /// The furthest cluster boundary in `start..limit` that still fits, and
    /// never fewer than one cluster: a box narrower than a single glyph still
    /// gets that glyph rather than an empty line and an endless loop.
    fn emergency(&self, start: usize, limit: usize, max_width: f32) -> usize {
        let mut end = start + 1;
        while end < limit && self.width(start, end + 1) <= max_width + WIDTH_EPSILON_PX {
            end += 1;
        }
        // Whitespace alone is not a line either, however narrow the box: cut
        // past it so this line draws something.
        while end < limit && self.trim(start, end) == start {
            end += 1;
        }
        end
    }

    /// True when the container allows a line to break at all.
    fn wraps(&self) -> bool {
        self.policy != BreakPolicy::None
    }

    fn width(&self, start: usize, end: usize) -> f32 {
        self.prefix[end] - self.prefix[start]
    }

    /// `end` with trailing whitespace cells removed.
    fn trim(&self, start: usize, end: usize) -> usize {
        let mut trimmed = end;
        while trimmed > start && self.cells[trimmed - 1].whitespace {
            trimmed -= 1;
        }
        trimmed
    }

    /// Cell index whose cluster starts exactly at `offset`.
    fn cell_at(&self, offset: usize) -> Option<usize> {
        self.cells
            .binary_search_by(|cell| cell.start.cmp(&offset))
            .ok()
    }

    /// Cells whose clusters lie inside `range`, as an index span.
    fn cell_span(&self, range: &Range<usize>) -> (usize, usize) {
        let lo = self.cells.partition_point(|cell| cell.start < range.start);
        let hi = self.cells.partition_point(|cell| cell.start < range.end);
        (lo, hi)
    }

    /// True when this layout has already produced every line it is allowed to.
    fn at_line_capacity(&self) -> bool {
        self.input
            .constraints
            .max_lines
            .is_some_and(|max| self.lines.len() >= usize::from(max))
    }

    /// Places a line unless it would exceed `max_lines` or `max_height_px`.
    ///
    /// Returns false when the budget stopped it, in which case the layout is
    /// finished: the line already placed becomes the truncation point and takes
    /// the ellipsis, if one was asked for.
    fn place_within_budget(
        &mut self,
        prepared: Prepared,
        cause: LineBreakCause,
        empty_at: usize,
    ) -> bool {
        let over_height = self.max_height_px.is_some_and(|max| {
            !self.lines.is_empty() && self.next_top_px + prepared.height_px > max + WIDTH_EPSILON_PX
        });
        if self.at_line_capacity() {
            self.truncate_here(LineBreakCause::MaxLines);
            return false;
        }
        if over_height {
            self.truncate_here(LineBreakCause::MaxHeight);
            return false;
        }
        self.place(prepared, cause, empty_at);
        true
    }

    /// Marks the layout truncated and re-places the last line with an ellipsis.
    ///
    /// `cause` says which budget ran out: a consumer reading `MaxLines` on a
    /// layout whose `max_lines` was never set has been told something untrue.
    fn truncate_here(&mut self, cause: LineBreakCause) {
        self.truncated = true;
        if let Some(line) = self.lines.last_mut() {
            line.break_cause = cause;
        }
        self.ellipsize_last_line();
    }

    /// Re-places the last line, cut short so a shaped ellipsis fits after it.
    fn ellipsize_last_line(&mut self) {
        let Some(ellipsis) = self.input.ellipsis else {
            return;
        };
        let (Some(line), Some(placed)) = (self.lines.pop(), self.placed_lines.pop()) else {
            return;
        };
        self.runs.truncate(line.runs.start as usize);
        self.work.runs_placed -= (line.runs.end - line.runs.start) as usize;
        self.work.ellipsis_runs_used -= placed.ellipsis_runs;
        self.work.lines_created -= 1;
        self.next_top_px = line.metrics.top_y_px;
        let cells = placed.cells;
        let cut = self.fit_with_ellipsis(cells.clone(), ellipsis.advance_px);
        let prepared = self.prepare(cells.start..cut);
        // The line's own anchor, not one re-derived from the cells: a line with
        // no cells has none to re-derive from, and falling back to byte 0 would
        // move an empty last line to the start of the text.
        self.place_ellipsized(prepared, line.break_cause, placed.anchor);
    }

    /// The largest prefix of `cells` that leaves room for the ellipsis.
    ///
    /// The cut is between cells, so it can never fall inside a grapheme
    /// cluster, a ligature or a UTF-8 sequence.
    fn fit_with_ellipsis(&self, cells: Range<usize>, ellipsis_px: f32) -> usize {
        // Always a trimmed end: the ellipsis takes the place of the text it
        // replaced, so it starts where that text's last drawn cluster ended.
        // Placing it after the hung whitespace instead would draw it beyond the
        // width the line reports, outside the box, with nothing saying so.
        let mut end = self.trim(cells.start, cells.end);
        let Some(max_width) = self.max_width_px else {
            return end;
        };
        while end > cells.start {
            if self.width(cells.start, end) + ellipsis_px <= max_width + WIDTH_EPSILON_PX {
                return end;
            }
            end = self.trim(cells.start, end - 1);
        }
        end
    }

    /// Measures a line without placing it.
    fn prepare(&self, cells: Range<usize>) -> Prepared {
        let pieces = self.pieces(cells.clone());
        let trimmed = self.trim(cells.start, cells.end);
        let width_px = self.width(cells.start, trimmed);
        let (ascent_px, descent_px, height_px, lift_px) = self.line_box(&pieces);
        Prepared {
            cells,
            pieces,
            width_px,
            ascent_px,
            descent_px,
            height_px,
            lift_px,
            adjust: LineAdjust::default(),
        }
    }

    /// [`Self::prepare`] with what the line decision changed on the line.
    fn prepare_with(&self, cells: Range<usize>, adjust: LineAdjust) -> Prepared {
        let mut prepared = self.prepare(cells.clone());
        let trimmed = self.trim(cells.start, cells.end);
        prepared.width_px += adjust.width_delta(trimmed);
        prepared.adjust = adjust;
        prepared
    }

    fn place(&mut self, prepared: Prepared, cause: LineBreakCause, empty_at: usize) {
        // A line that overflows a container which asked for an ellipsis is cut
        // here rather than left sticking out — but only where the cut text is
        // not going to be shown anywhere else. A wrapping paragraph only
        // overflows on a word no break opportunity can split, and that word is
        // already on this line: cutting it would drop bytes no line covers,
        // leaving a hole in the middle of the text that nothing reports. There
        // the line sticks out and says so with `CLIPPED_WIDTH`.
        let overflows = !self.wraps()
            && self
                .max_width_px
                .is_some_and(|max| prepared.width_px > max + WIDTH_EPSILON_PX);
        match (overflows, self.input.ellipsis) {
            (true, Some(ellipsis)) => {
                let cut = self.fit_with_ellipsis(prepared.cells.clone(), ellipsis.advance_px);
                let cut_line = self.prepare(prepared.cells.start..cut);
                self.place_ellipsized(cut_line, cause, empty_at);
            }
            _ => self.place_line(prepared, cause, empty_at, false),
        }
    }

    fn place_ellipsized(&mut self, prepared: Prepared, cause: LineBreakCause, empty_at: usize) {
        self.ellipsized = true;
        self.place_line(prepared, cause, empty_at, true);
    }

    /// Places one line: runs in visual order, metrics, alignment, bounds.
    fn place_line(
        &mut self,
        prepared: Prepared,
        break_cause: LineBreakCause,
        empty_at: usize,
        with_ellipsis: bool,
    ) {
        let Prepared {
            cells,
            pieces,
            mut width_px,
            ascent_px,
            descent_px,
            height_px,
            lift_px,
            adjust,
        } = prepared;
        let source = match cells.end.checked_sub(1) {
            Some(last) if cells.start < cells.end => {
                self.cells[cells.start].start..self.cells[last].end
            }
            _ => empty_at..empty_at,
        };

        let ellipsis = self.input.ellipsis.filter(|_| with_ellipsis);
        let mut ordered = self.visual_order(&pieces);
        if let Some(ellipsis) = ellipsis {
            width_px += ellipsis.advance_px;
        }
        let origin_x_px = self.align_offset(width_px);
        let top_y_px = self.next_top_px;
        let (strut_ascent, strut_descent) = match self.input.strut {
            Some(strut) => (strut.metrics.ascent_px, strut.metrics.descent_px),
            None => (ascent_px, descent_px),
        };
        // A vertical line's dominant baseline is the central one: upright
        // glyphs hang from the column's centre line, and sideways runs centre
        // their em box on it.
        let baseline_y_px = if self.input.vertical {
            top_y_px + height_px * 0.5
        } else {
            let half_leading = (height_px - lift_px - (strut_ascent + strut_descent)) * 0.5;
            top_y_px + lift_px + half_leading + strut_ascent
        };

        let run_start = self.runs.len() as u32;
        // Hung whitespace is drawn but not measured, and rule L1 put it at the
        // paragraph's *end* — which is the visual left in an RTL paragraph. The
        // cursor therefore starts before the aligned box by exactly what hangs
        // there, or the hung space would sit inside the box and push every real
        // glyph off the other edge.
        let hung_px = self.width(self.trim(cells.start, cells.end), cells.end);
        let mut cursor = if self.input.base_direction.is_rtl() {
            origin_x_px - hung_px
        } else {
            origin_x_px
        };
        // The ellipsis marks the visual end of the line, which is the left edge
        // in an RTL paragraph.
        let mut placed: Vec<ShapedRun> = Vec::with_capacity(ordered.len() + 1);
        let ellipsis_runs = ellipsis.map(|ellipsis| ellipsis.placed(source.end));
        if let Some(runs) = &ellipsis_runs
            && self.input.base_direction.is_rtl()
        {
            placed.extend(runs.iter().cloned());
        }
        placed.extend(
            ordered
                .drain(..)
                .map(|piece| self.placed_run(&piece, &adjust)),
        );
        if let Some(runs) = &ellipsis_runs
            && !self.input.base_direction.is_rtl()
        {
            placed.extend(runs.iter().cloned());
        }
        let ellipsis_run_count = ellipsis_runs.as_ref().map_or(0, Vec::len);
        self.work.ellipsis_runs_used += ellipsis_run_count;
        for mut run in placed {
            run.origin_x_px = cursor;
            cursor += run.advance_px;
            self.runs.push(run);
        }
        let run_end = self.runs.len() as u32;
        self.work.runs_placed += (run_end - run_start) as usize;

        self.lines.push(LineBox {
            index: self.lines.len() as u32,
            source,
            runs: run_start..run_end,
            break_cause,
            metrics: LineMetrics {
                baseline_y_px,
                top_y_px,
                height_px,
                ascent_px,
                descent_px,
                width_px,
            },
            bounds: TextRect::new(origin_x_px, top_y_px, width_px, height_px),
            base_direction: self.input.base_direction,
        });
        self.placed_lines.push(PlacedLine {
            cells,
            ellipsis_runs: ellipsis_run_count,
            anchor: empty_at,
        });
        self.work.lines_created += 1;
        self.next_top_px += height_px;
    }

    /// Maximal groups of consecutive cells from one shaped run, in logical
    /// order.
    fn pieces(&self, cells: Range<usize>) -> Vec<Piece> {
        let mut pieces: Vec<Piece> = Vec::new();
        for index in cells {
            let cell = &self.cells[index];
            match pieces.last_mut() {
                Some(piece) if piece.run == cell.run => {
                    piece.cells.end = index + 1;
                    piece.glyphs.start = piece.glyphs.start.min(cell.glyphs.start);
                    piece.glyphs.end = piece.glyphs.end.max(cell.glyphs.end);
                    piece.source.start = piece.source.start.min(cell.start);
                    piece.source.end = piece.source.end.max(cell.end);
                    piece.advance_px += cell.advance_px;
                    piece.whitespace &= cell.whitespace;
                }
                _ => pieces.push(Piece {
                    run: cell.run,
                    cells: index..index + 1,
                    glyphs: cell.glyphs.clone(),
                    source: cell.start..cell.end,
                    advance_px: cell.advance_px,
                    whitespace: cell.whitespace,
                }),
            }
        }
        pieces
    }

    /// Rule L1 then rule L2: trailing whitespace takes the paragraph level,
    /// then the pieces are reordered by level.
    fn visual_order(&self, pieces: &[Piece]) -> Vec<Piece> {
        let base_level = u8::from(self.input.base_direction.is_rtl());
        let mut levels: Vec<u8> = pieces
            .iter()
            .map(|piece| self.input.runs[piece.run].bidi_level)
            .collect();
        for (index, piece) in pieces.iter().enumerate().rev() {
            if !piece.whitespace {
                break;
            }
            levels[index] = base_level;
        }
        bidi_visual_order(&levels)
            .into_iter()
            .map(|index| pieces[index].clone())
            .collect()
    }

    /// Copies the glyphs a piece draws out of the shaped run it came from.
    ///
    /// The piece keeps the shaped run's id: the id names the shaping that
    /// produced those glyphs, and cutting a line through a run reshapes
    /// nothing.
    fn placed_run(&self, piece: &Piece, adjust: &LineAdjust) -> ShapedRun {
        let run = &self.input.runs[piece.run];
        let mut glyphs = run.glyphs[piece.glyphs.clone()].to_vec();
        // What the line decision changed: the fixed changes every line keeps
        // (already in the cells' advances) and this line's own.
        let mut adjusted = false;
        let mut line_advance = 0.0;
        if let Some(typo) = &self.typo {
            for index in piece.cells.clone() {
                let (fixed_advance, fixed_offset) = typo.fixed[index];
                let (line_a, line_o) = adjust.of(index);
                let (advance, offset) = (fixed_advance + line_a, fixed_offset + line_o);
                if advance == 0.0 && offset == 0.0 {
                    continue;
                }
                adjusted = true;
                line_advance += line_a;
                let cell = &self.cells[index];
                let first = cell.glyphs.start - piece.glyphs.start;
                let last = cell.glyphs.end - 1 - piece.glyphs.start;
                glyphs[last].advance_px += advance;
                glyphs[first].offset_x_px += offset;
            }
        }
        let whole = !adjusted && piece.glyphs.start == 0 && piece.glyphs.end == run.glyphs.len();
        ShapedRun {
            id: run.id,
            source: piece.source.clone(),
            direction: run.direction,
            bidi_level: run.bidi_level,
            script: run.script,
            orientation: run.orientation,
            font: run.font,
            font_size_px: run.font_size_px,
            glyphs,
            // A whole run keeps the advance the shaper reported. Only a run cut
            // by a line break, or adjusted, is re-summed, from its per-glyph
            // advances.
            advance_px: if whole {
                run.advance_px
            } else {
                piece.advance_px + line_advance
            },
            // Assigned by the caller once the line's visual order is known.
            origin_x_px: 0.0,
            metrics: run.metrics,
            instance: run.instance.clone(),
            ignored_axes: run.ignored_axes.clone(),
        }
    }

    /// Reported ascent and descent (the tallest run, strut included), the
    /// line box height (the tallest line-height request, strut included,
    /// grown to hold any inline object), and how far such an object moved
    /// the baseline down.
    ///
    /// An object is an atomic inline: it does not ask for a line height of
    /// its own, it is a box standing on the baseline. The line keeps the
    /// text's half-leading and grows above and below by what the object
    /// reaches past it.
    fn line_box(&self, pieces: &[Piece]) -> (f32, f32, f32, f32) {
        let mut ascent_px: f32 = 0.0;
        let mut descent_px: f32 = 0.0;
        let mut height_px = self.input.strut.map_or(0.0, |strut| strut.line_height_px);
        let mut object_above: f32 = 0.0;
        let mut object_below: f32 = 0.0;
        let mut text_runs = 0;
        let ruby_above = self
            .input
            .rubies
            .iter()
            .filter(|ruby| {
                pieces.iter().any(|piece| {
                    piece.source.start < ruby.range.end && ruby.range.start < piece.source.end
                })
            })
            .map(|ruby| ruby.height_px)
            .fold(0.0f32, f32::max);
        for piece in pieces {
            let run = &self.input.runs[piece.run];
            ascent_px = ascent_px.max(run.metrics.ascent_px);
            descent_px = descent_px.max(run.metrics.descent_px);
            if run.is_object() {
                object_above = object_above.max(run.metrics.ascent_px);
                object_below = object_below.max(run.metrics.descent_px);
                continue;
            }
            text_runs += 1;
            height_px = height_px.max(self.input.run_line_heights[piece.run]);
        }
        if pieces.is_empty() || text_runs == 0 {
            height_px = height_px.max(self.input.empty_line_height_px);
        }
        if let Some(strut) = self.input.strut {
            ascent_px = ascent_px.max(strut.metrics.ascent_px);
            descent_px = descent_px.max(strut.metrics.descent_px);
        }
        if self.input.vertical || (object_above <= 0.0 && object_below <= 0.0 && ruby_above <= 0.0)
        {
            return (ascent_px, descent_px, height_px, 0.0);
        }
        let (text_ascent, text_descent) = match self.input.strut {
            Some(strut) => (strut.metrics.ascent_px, strut.metrics.descent_px),
            None => {
                let text = pieces
                    .iter()
                    .map(|piece| &self.input.runs[piece.run])
                    .filter(|run| !run.is_object());
                text.fold((0.0f32, 0.0f32), |(a, d), run| {
                    (a.max(run.metrics.ascent_px), d.max(run.metrics.descent_px))
                })
            }
        };
        let half_leading = (height_px - (text_ascent + text_descent)) * 0.5;
        let above = half_leading + text_ascent;
        let below = height_px - above;
        // An annotation stands on the text's ascent.
        let lift = (object_above - above)
            .max(text_ascent + ruby_above - above)
            .max(0.0);
        let grown_below = (object_below - below).max(0.0);
        (ascent_px, descent_px, height_px + lift + grown_below, lift)
    }

    /// Where a line of `width_px` starts inside the container.
    ///
    /// With no `max_width_px` there is no container to align in, so every
    /// keyword lays the line out at the origin. `start` / `end` follow the
    /// paragraph direction; `left` / `right` do not.
    ///
    /// A line wider than the container gets **negative** slack rather than
    /// being pushed back to the origin: `text-align: right` on an overflowing
    /// line overflows past the start edge, so what a clipped container shows is
    /// the end of the string — the end the caller aligned to. Clamping would
    /// show the other end.
    fn align_offset(&self, width_px: f32) -> f32 {
        let Some(max_width) = self.max_width_px else {
            return 0.0;
        };
        let slack = max_width - width_px;
        // Line space runs from line-left — the left, or the top of a vertical
        // line — whatever the direction; `start` / `end` are flow-relative, so
        // an RTL line starts at line-right: the right, or the bottom of a
        // vertical line (CSS Writing Modes §2.1), as the box layout's inline
        // axis does.
        let rtl = self.input.constraints.base_direction == DirSpec::Rtl;
        match self.input.constraints.align {
            TextAlignSpec::Start => {
                if rtl {
                    slack
                } else {
                    0.0
                }
            }
            TextAlignSpec::End => {
                if rtl {
                    0.0
                } else {
                    slack
                }
            }
            TextAlignSpec::Left => 0.0,
            TextAlignSpec::Right => slack,
            TextAlignSpec::Center => slack * 0.5,
            // A justified line fills its box; a last line, or one ending at a
            // forced break, sits at the start.
            TextAlignSpec::Justify => {
                if rtl {
                    slack
                } else {
                    0.0
                }
            }
        }
    }

    fn finish(mut self) -> LaidOut {
        self.work.shape_runs_reused += self.input.runs.len();
        let mut overflow = OverflowFlags::NONE;
        if let Some(max_width) = self.max_width_px
            && self
                .lines
                .iter()
                .any(|line| line.metrics.width_px > max_width + WIDTH_EPSILON_PX)
        {
            overflow = overflow.with(OverflowFlags::CLIPPED_WIDTH);
        }
        if self.truncated {
            overflow = overflow.with(OverflowFlags::TRUNCATED_LINES);
        }
        if self.ellipsized {
            overflow = overflow.with(OverflowFlags::ELLIPSIZED);
        }
        LaidOut {
            runs: std::mem::take(&mut self.runs),
            lines: std::mem::take(&mut self.lines),
            overflow,
            work: self.work,
        }
    }
}

/// Every run of text a line may not break out of, in order.
///
/// One per BiDi paragraph, each split again at the forced breaks the paragraph
/// structure does not carry (VT, FF, U+2028). A separator falls *between* two
/// segments, so no segment covers one and nothing draws it — the treatment
/// `\n` already gets from the shaper.
///
/// Two segments exist for a caret rather than for glyphs: empty text has no
/// BiDi paragraph at all, and text ending in a separator has no paragraph after
/// it. Both still need the line the caret sits on — pressing Enter at the end
/// of a field must not make the caret vanish.
///
/// Three callers read this, and that is the point: layout breaks at these
/// boundaries, `intrinsic_widths` measures between them (so `max-content` is
/// the widest line, not two lines joined end to end), and the Label fast path
/// asks whether there is exactly one of them.
pub(super) fn segments(text: &str, paragraphs: &[ShapedParagraph]) -> Vec<Range<usize>> {
    let mut segments: Vec<Range<usize>> = Vec::with_capacity(paragraphs.len());
    for paragraph in paragraphs {
        let content = content_range(text, &paragraph.range);
        if !breaks::has_forced_break(&text[content.clone()]) {
            segments.push(content);
            continue;
        }
        let mut start = content.start;
        let mut cursor = content.start;
        while cursor < content.end {
            let character = text[cursor..]
                .chars()
                .next()
                .expect("cursor is on a character boundary");
            let width = character.len_utf8();
            if breaks::FORCED_BREAKS.contains(&character) {
                segments.push(start..cursor);
                start = cursor + width;
            }
            cursor += width;
        }
        segments.push(start..content.end);
    }
    match segments.last() {
        None => segments.push(0..0),
        Some(last) if last.end < text.len() => segments.push(text.len()..text.len()),
        Some(_) => {}
    }
    segments
}

/// A maximal run of consecutive cells belonging to one shaped run.
#[derive(Debug, Clone)]
struct Piece {
    run: usize,
    /// The cells it draws, in logical order.
    cells: Range<usize>,
    glyphs: Range<usize>,
    source: Range<usize>,
    advance_px: f32,
    whitespace: bool,
}

/// Clusters of every run, in logical order.
///
/// A run's glyphs are in visual order, which is logical order for an LTR run
/// and its reverse for an RTL one, so an RTL run's cluster groups are reversed
/// back here. Clusters are the shaper's own, never re-derived from grapheme
/// segmentation.
fn cells(text: &str, runs: &[ShapedRun]) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        let first = cells.len();
        let mut group: Option<usize> = None;
        for (offset, glyph) in run.glyphs.iter().enumerate() {
            let continues = group.is_some_and(|at| cells[at].start == glyph.cluster as usize);
            if let (true, Some(at)) = (continues, group) {
                cells[at].end = cells[at].end.max(glyph.cluster_end as usize);
                cells[at].advance_px += glyph.advance_px;
                cells[at].glyphs.end = offset + 1;
                continue;
            }
            group = Some(cells.len());
            cells.push(Cell {
                start: glyph.cluster as usize,
                end: glyph.cluster_end as usize,
                advance_px: glyph.advance_px,
                run: index,
                glyphs: offset..offset + 1,
                whitespace: false,
            });
        }
        if run.direction.is_rtl() {
            cells[first..].reverse();
        }
    }
    for cell in &mut cells {
        cell.whitespace = text
            .get(cell.start..cell.end)
            .is_some_and(|slice| !slice.is_empty() && slice.chars().all(char::is_whitespace));
    }
    cells
}

/// A paragraph's range without the separator that ended it.
///
/// `ShapedParagraph::range` includes the trailing `\n`, which is not content:
/// it produces no glyph, and asking UAX #14 about it would only rediscover the
/// break the paragraph structure already carries.
fn content_range(text: &str, range: &Range<usize>) -> Range<usize> {
    let body = &text[range.clone()];
    if body.ends_with("\r\n") {
        return range.start..range.end - "\r\n".len();
    }
    match body.chars().next_back() {
        Some(last) if PARAGRAPH_SEPARATORS.contains(&last) => {
            range.start..range.end - last.len_utf8()
        }
        _ => range.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_range_drops_only_its_own_separator() {
        assert_eq!(content_range("one\ntwo", &(0..4)), 0..3);
        assert_eq!(content_range("one\r\ntwo", &(0..5)), 0..3);
        assert_eq!(content_range("one\ntwo", &(4..7)), 4..7);
    }

    #[test]
    fn wrap_policy_reads_word_break_and_line_break_as_well_as_the_wrap_mode() {
        let mut constraints = TextConstraints::default();
        assert_eq!(BreakPolicy::of(&constraints), BreakPolicy::None);
        constraints.wrap = Some(TextWrapBreak::Word);
        assert_eq!(BreakPolicy::of(&constraints), BreakPolicy::Word);
        constraints.word_break = WordBreakSpec::BreakWord;
        assert_eq!(BreakPolicy::of(&constraints), BreakPolicy::WordThenGlyph);
        constraints.word_break = WordBreakSpec::BreakAll;
        assert_eq!(BreakPolicy::of(&constraints), BreakPolicy::Glyph);
        constraints.word_break = WordBreakSpec::Normal;
        constraints.line_break = LineBreakSpec::Anywhere;
        assert_eq!(BreakPolicy::of(&constraints), BreakPolicy::Glyph);
    }
}

/// What the CJK line decision knows about each cell (Issue #211), built
/// only when a line may adjust something: punctuation closing up, autospace,
/// justification, strict or loose breaking, or a pretty paragraph. Without
/// it every line is laid out exactly as before.
struct Typography {
    trim: TextSpacingTrimSpec,
    /// A line short by what its punctuation and autospace can give closes
    /// them up, at a cost, rather than break early (`text-spacing-trim:
    /// auto`).
    cost_fit: bool,
    justify: Option<TextJustifySpec>,
    pretty: bool,
    line_break: LineBreakSpec,
    /// Per cell, the blank that may still close at its end, and at its start.
    after: Vec<f32>,
    before: Vec<f32>,
    /// Autospace put after the cell, dropped at the end of a line.
    space_after: Vec<f32>,
    /// Advance and glyph offset changes every line keeps: adjacent marks
    /// closed up, autospace put in.
    fixed: Vec<(f32, f32)>,
    /// Whether the cell is a CJK character a justified line may space after.
    cjk: Vec<bool>,
    /// The font size each cell was shaped at, for raggedness.
    em: Vec<f32>,
    /// `squeeze[i]` is what cost fitting may close in `cells[..i]`.
    squeeze: Vec<f32>,
    /// Per cell, what cost fitting may close at its end and at its start.
    give_after: Vec<f32>,
    give_before: Vec<f32>,
    /// Breaks an inline object's envelope forbids before or after its cell.
    no_break_before: Vec<bool>,
    no_break_after: Vec<bool>,
}

impl Typography {
    fn build(
        text: &str,
        runs: &[ShapedRun],
        cells: &mut [Cell],
        constraints: &TextConstraints,
        vertical: bool,
        envelopes: &[(usize, crate::InlineEnvelope)],
    ) -> Option<Self> {
        let trim = constraints.spacing_trim;
        let autospace = constraints.autospace == TextAutospaceSpec::Normal;
        let justify = (constraints.align == TextAlignSpec::Justify).then_some(constraints.justify);
        let pretty = constraints.wrap_style == TextWrapStyleSpec::Pretty;
        let line_break = constraints.line_break;
        let tailored = matches!(line_break, LineBreakSpec::Strict | LineBreakSpec::Loose);
        if !trim.trims()
            && !autospace
            && justify.is_none()
            && !pretty
            && !tailored
            && envelopes.is_empty()
        {
            return None;
        }
        // Vertical lines keep their advances: which side of a vertical form
        // is blank is not what a horizontal measure says.
        let adjusts = !vertical;
        let n = cells.len();
        let char_of = |cell: &Cell| text.get(cell.start..).and_then(|rest| rest.chars().next());
        let flags_of = |cell: &Cell| {
            if cell.glyphs.len() == 1 {
                runs[cell.run].glyphs[cell.glyphs.start].flags
            } else {
                GlyphFlags::default()
            }
        };
        let mut after = vec![0.0f32; n];
        let mut before = vec![0.0f32; n];
        let mut space_after = vec![0.0f32; n];
        let mut fixed = vec![(0.0f32, 0.0f32); n];
        let mut cjk = vec![false; n];
        let mut em = vec![0.0f32; n];
        for (at, cell) in cells.iter().enumerate() {
            em[at] = runs[cell.run].font_size_px;
            cjk[at] = char_of(cell).is_some_and(is_cjk_letter);
            if !(adjusts && trim.trims()) {
                continue;
            }
            let flags = flags_of(cell);
            if flags.contains(GlyphFlags::PUNCT_BLANK_AFTER) {
                after[at] = cell.advance_px * 0.5;
            } else if flags.contains(GlyphFlags::PUNCT_BLANK_BEFORE) {
                before[at] = cell.advance_px * 0.5;
            }
        }
        let punctuation: Vec<bool> = (0..n)
            .map(|at| after[at] > 0.0 || before[at] > 0.0)
            .collect();
        let opening: Vec<bool> = before.iter().map(|blank| *blank > 0.0).collect();
        if adjusts && trim.trims() {
            // A closing mark before another mark, and an opening mark after
            // an opening one, close up: one blank half between them, not two.
            for at in 0..n.saturating_sub(1) {
                if after[at] > 0.0 && punctuation[at + 1] {
                    fixed[at].0 -= after[at];
                    after[at] = 0.0;
                }
                if before[at + 1] > 0.0 && opening[at] {
                    fixed[at + 1].0 -= before[at + 1];
                    fixed[at + 1].1 -= before[at + 1];
                    before[at + 1] = 0.0;
                }
            }
        }
        if adjusts && autospace {
            for at in 0..n.saturating_sub(1) {
                let (left, right) = (char_of(&cells[at]), char_of(&cells[at + 1]));
                let (Some(left), Some(right)) = (left, right) else {
                    continue;
                };
                let meets = (is_ideograph(left) && is_latin_alnum(right))
                    || (is_latin_alnum(left) && is_ideograph(right));
                if meets && !cells[at].whitespace && !cells[at + 1].whitespace {
                    let extra = em[at] / 8.0;
                    fixed[at].0 += extra;
                    space_after[at] = extra;
                }
            }
        }
        for (cell, (advance, _)) in cells.iter_mut().zip(&fixed) {
            cell.advance_px += advance;
        }
        // An object's envelope: the part of its gap that may close is dropped
        // at a line end like autospace and, inside a line, given up at a cost;
        // a break it forbids leaves the break table.
        let scale = constraints.scale.px_per_logical;
        let mut object_shrink = vec![0.0f32; n];
        let mut no_break_before = vec![false; n];
        let mut no_break_after = vec![false; n];
        for (offset, envelope) in envelopes {
            let Some(at) = cells.iter().position(|cell| cell.start == *offset) else {
                continue;
            };
            let shrink = (envelope.gap_shrink_px.min(envelope.gap_px).max(0.0)) * scale;
            if adjusts {
                object_shrink[at] = shrink;
                space_after[at] += shrink;
            }
            no_break_before[at] = !envelope.break_before;
            no_break_after[at] = !envelope.break_after;
        }
        let objects_give = object_shrink.iter().any(|shrink| *shrink > 0.0);
        let punctuation_fit = adjusts && trim == TextSpacingTrimSpec::Auto;
        let cost_fit = punctuation_fit || objects_give;
        // What cost fitting may close inside a line, at a cell's end and at
        // its start: under `auto` every blank and autospace, otherwise only
        // what objects declared.
        let (give_after, give_before): (Vec<f32>, Vec<f32>) = (0..n)
            .map(|at| match punctuation_fit {
                true => (after[at] + space_after[at], before[at]),
                false => (object_shrink[at], 0.0),
            })
            .unzip();
        let mut squeeze = Vec::with_capacity(n + 1);
        let mut total = 0.0;
        squeeze.push(0.0);
        for at in 0..n {
            total += give_after[at] + give_before[at];
            squeeze.push(total);
        }
        Some(Self {
            trim,
            cost_fit,
            justify,
            pretty,
            line_break,
            after,
            before,
            space_after,
            fixed,
            cjk,
            em,
            squeeze,
            give_after,
            give_before,
            no_break_before,
            no_break_after,
        })
    }

    /// What a line starting at `start` closes at its start: an opening mark's
    /// blank half, except on a paragraph's first line under `normal`.
    fn start_trim(&self, start: usize, first: bool) -> f32 {
        match self.trim {
            TextSpacingTrimSpec::TrimBoth => self.before[start],
            TextSpacingTrimSpec::Normal | TextSpacingTrimSpec::Auto if !first => self.before[start],
            _ => 0.0,
        }
    }

    /// What a line ending at cell `last` always drops (its autospace), and
    /// what it closes only when it would not fit otherwise (a closing mark's
    /// blank half).
    fn end_trims(&self, last: usize) -> (f32, f32) {
        let optional = if self.trim.trims() {
            self.after[last]
        } else {
            0.0
        };
        (self.space_after[last], optional)
    }

    /// What cost fitting may close strictly inside `start..last`.
    fn interior_squeeze(&self, start: usize, last: usize) -> f32 {
        if last <= start + 1 {
            return 0.0;
        }
        self.squeeze[last] - self.squeeze[start + 1]
    }

    /// Breaks the profile forbids or allows beyond UAX #14: strict forbids a
    /// break before hyphens and wave dashes, loose allows one before small
    /// kana and iteration marks, and a cluster break (`break-all`) never
    /// puts closing punctuation at a line start or opening punctuation at a
    /// line end (kinsoku).
    fn tailor(
        &self,
        text: &str,
        cells: &[Cell],
        mut stops: Vec<usize>,
        lo: usize,
        hi: usize,
        glyph_policy: bool,
    ) -> Vec<usize> {
        let ch = |at: usize| {
            text.get(cells[at].start..)
                .and_then(|rest| rest.chars().next())
        };
        if self.line_break == LineBreakSpec::Loose {
            for at in lo + 1..hi {
                let allowed = ch(at).is_some_and(loose_breaks_before)
                    && ch(at - 1).is_some_and(is_cjk_letter);
                if allowed {
                    stops.push(at);
                }
            }
            stops.sort_unstable();
            stops.dedup();
        }
        stops.retain(|at| {
            if self.no_break_before[*at] || self.no_break_after[*at - 1] {
                return false;
            }
            let before = ch(*at);
            let after_prev = ch(*at - 1);
            if self.line_break == LineBreakSpec::Strict
                && before.is_some_and(strict_no_break_before)
            {
                return false;
            }
            let kinsoku = glyph_policy && self.line_break != LineBreakSpec::Anywhere;
            !(kinsoku
                && (before.is_some_and(kinsoku_no_line_start)
                    || after_prev.is_some_and(kinsoku_no_line_end)))
        });
        stops
    }

    /// The visual price of leaving `slack_px` unused at the end of a line
    /// set at `em_px`: square in the slack, so a line a little short costs
    /// little and one far short costs much, and priced so that one em of
    /// slack costs what closing up one em of punctuation does.
    fn raggedness(slack_px: f32, em_px: f32) -> u64 {
        let per_px = u64::from(
            nana_ui_core::dynamic_layout::costs::PUNCTUATION
                .finite()
                .unwrap_or(15),
        );
        let slack = slack_px.max(0.0) as f64;
        let em = (em_px as f64).max(1.0);
        (per_px as f64 * 64.0 * slack * slack / em) as u64
    }

    /// The visual price of closing up `deficit_px`.
    fn squeeze_cost(deficit_px: f32) -> u64 {
        let per_px = u64::from(
            nana_ui_core::dynamic_layout::costs::PUNCTUATION
                .finite()
                .unwrap_or(15),
        );
        per_px
            * nana_ui_core::dynamic_layout::LayoutUnits::from_px(deficit_px)
                .0
                .max(0) as u64
    }
}

/// How a candidate line fits its box.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fit {
    /// As set, or with its end mark closed up.
    Fits {
        end_closed: bool,
    },
    /// Only by closing up `deficit` px of its punctuation and autospace.
    Squeezed {
        deficit: f32,
    },
    No,
}

/// What one line changes on the cells it sets: `(cell, advance, offset)`.
#[derive(Debug, Clone, Default)]
struct LineAdjust {
    deltas: Vec<(usize, f32, f32)>,
}

impl LineAdjust {
    fn width_delta(&self, drawn: usize) -> f32 {
        self.deltas
            .iter()
            .filter(|(cell, _, _)| *cell < drawn)
            .map(|(_, advance, _)| advance)
            .sum()
    }

    fn of(&self, cell: usize) -> (f32, f32) {
        self.deltas
            .iter()
            .filter(|(at, _, _)| *at == cell)
            .fold((0.0, 0.0), |(advance, offset), (_, a, o)| {
                (advance + a, offset + o)
            })
    }
}

fn is_ideograph(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF | 0x3040..=0x30FF)
}

fn is_cjk_letter(ch: char) -> bool {
    is_ideograph(ch)
        || matches!(ch as u32, 0x3100..=0x312F | 0x31A0..=0x31BF | 0x31F0..=0x31FF | 0xAC00..=0xD7AF)
}

fn is_latin_alnum(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || (('\u{00C0}'..='\u{024F}').contains(&ch) && ch.is_alphabetic())
}

/// CSS `line-break: strict` forbids a break before these beyond `normal`.
fn strict_no_break_before(ch: char) -> bool {
    matches!(
        ch,
        '\u{2010}'
            | '\u{2013}'
            | '\u{301C}'
            | '\u{30A0}'
            | '\u{30FC}'
            | '\u{3005}'
            | '\u{303B}'
            | '\u{309D}'
            | '\u{309E}'
            | '\u{30FD}'
            | '\u{30FE}'
    ) || is_small_kana(ch)
}

/// CSS `line-break: loose` allows a break before these after a CJK letter.
fn loose_breaks_before(ch: char) -> bool {
    is_small_kana(ch)
        || matches!(
            ch,
            '\u{30FC}'
                | '\u{3005}'
                | '\u{303B}'
                | '\u{309D}'
                | '\u{309E}'
                | '\u{30FD}'
                | '\u{30FE}'
        )
}

fn is_small_kana(ch: char) -> bool {
    matches!(
        ch,
        'ぁ' | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ゎ'
            | 'ゕ'
            | 'ゖ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | 'ヮ'
            | 'ヵ'
            | 'ヶ'
    ) || ('\u{31F0}'..='\u{31FF}').contains(&ch)
}

/// Kinsoku: never at the start of a line.
fn kinsoku_no_line_start(ch: char) -> bool {
    matches!(
        ch,
        '、' | '。'
            | '，'
            | '．'
            | '）'
            | '］'
            | '｝'
            | '」'
            | '』'
            | '】'
            | '〕'
            | '〉'
            | '》'
            | '〙'
            | '〗'
            | '’'
            | '”'
            | '！'
            | '？'
            | '：'
            | '；'
            | '・'
            | 'ー'
            | '々'
            | '…'
            | '‥'
    ) || is_small_kana(ch)
}

/// Kinsoku: never at the end of a line.
fn kinsoku_no_line_end(ch: char) -> bool {
    matches!(
        ch,
        '（' | '［' | '｛' | '「' | '『' | '【' | '〔' | '〈' | '《' | '〘' | '〖' | '‘' | '“'
    )
}
