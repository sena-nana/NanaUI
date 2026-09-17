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
use crate::shape::{RunDirection, ShapedRun};
use crate::shaping::{ShapedParagraph, ShapedText, bidi_visual_order};
use nana_ui_core::{DirSpec, LineBreakSpec, TextAlignSpec, TextWrapBreak, WordBreakSpec};
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
/// Without one, each line's baseline is centred on that line's own tallest run
/// — the reference engine's rule, and it moves.
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
}

/// Work the line builder did, for the counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct LineWork {
    pub line_break_candidates: usize,
    pub lines_created: usize,
    pub runs_placed: usize,
    pub ellipsis_runs_used: usize,
    pub shape_runs_reused: usize,
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
}

impl<'a> Builder<'a> {
    pub fn new(input: LineInput<'a>) -> Self {
        let cells = cells(input.text, input.runs);
        let mut prefix = Vec::with_capacity(cells.len() + 1);
        let mut total = 0.0;
        prefix.push(0.0);
        for cell in &cells {
            total += cell.advance_px;
            prefix.push(total);
        }
        let scale = input.constraints.scale.px_per_logical;
        let max_width_px = input.constraints.max_width_px.map(|width| width * scale);
        let max_height_px = input.constraints.max_height_px.map(|height| height * scale);
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

    /// Every run of text a line may not break out of, in order.
    ///
    /// One per BiDi paragraph, each split again at the forced breaks the
    /// paragraph structure does not carry (VT, FF, U+2028). A separator falls
    /// *between* two segments, so no segment covers one and nothing draws it —
    /// the treatment `\n` already gets from the shaper.
    ///
    /// Two segments exist for a caret rather than for glyphs: empty text has no
    /// BiDi paragraph at all, and text ending in a separator has no paragraph
    /// after it. Both still need the line the caret sits on — pressing Enter at
    /// the end of a field must not make the caret vanish.
    ///
    /// This is also what `intrinsic_widths` measures over, so `max-content`
    /// cannot come back as the width of two lines joined end to end.
    fn segments(&self) -> Vec<Range<usize>> {
        let text = self.input.text;
        let mut segments: Vec<Range<usize>> = Vec::with_capacity(self.input.paragraphs.len());
        for paragraph in self.input.paragraphs {
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
                (lo + 1..hi).collect()
            }
            BreakPolicy::Word | BreakPolicy::WordThenGlyph => {
                let offsets =
                    breaks::opportunities(&self.input.text[content.clone()], content.start);
                self.work.line_break_candidates += offsets.len();
                offsets
                    .iter()
                    .filter_map(|offset| self.cell_at(*offset))
                    .filter(|index| *index > lo && *index < hi)
                    .collect()
            }
        }
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
            let width = self.width(start, self.trim(start, stop));
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

    /// The furthest cluster boundary in `start..limit` that still fits, and
    /// never fewer than one cluster: a box narrower than a single glyph still
    /// gets that glyph rather than an empty line and an endless loop.
    fn emergency(&self, start: usize, limit: usize, max_width: f32) -> usize {
        let mut end = start + 1;
        while end < limit && self.width(start, end + 1) <= max_width + WIDTH_EPSILON_PX {
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
        if self.at_line_capacity() || over_height {
            self.truncate_here();
            return false;
        }
        self.place(prepared, cause, empty_at);
        true
    }

    /// Marks the layout truncated and re-places the last line with an ellipsis.
    fn truncate_here(&mut self) {
        self.truncated = true;
        if let Some(line) = self.lines.last_mut() {
            line.break_cause = LineBreakCause::MaxLines;
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
        let Some(max_width) = self.max_width_px else {
            return cells.end;
        };
        let mut end = cells.end;
        while end > cells.start
            && self.width(cells.start, end) + ellipsis_px > max_width + WIDTH_EPSILON_PX
        {
            end -= 1;
        }
        end
    }

    /// Measures a line without placing it.
    fn prepare(&self, cells: Range<usize>) -> Prepared {
        let pieces = self.pieces(cells.clone());
        let trimmed = self.trim(cells.start, cells.end);
        let width_px = self.width(cells.start, trimmed);
        let (ascent_px, descent_px, height_px) = self.line_box(&pieces);
        Prepared {
            cells,
            pieces,
            width_px,
            ascent_px,
            descent_px,
            height_px,
        }
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
        let half_leading = (height_px - (strut_ascent + strut_descent)) * 0.5;
        let baseline_y_px = top_y_px + half_leading + strut_ascent;

        let run_start = self.runs.len() as u32;
        let mut cursor = origin_x_px;
        // The ellipsis marks the visual end of the line, which is the left edge
        // in an RTL paragraph.
        let mut placed: Vec<ShapedRun> = Vec::with_capacity(ordered.len() + 1);
        let ellipsis_runs = ellipsis.map(|ellipsis| ellipsis.placed(source.end));
        if let Some(runs) = &ellipsis_runs
            && self.input.base_direction.is_rtl()
        {
            placed.extend(runs.iter().cloned());
        }
        placed.extend(ordered.drain(..).map(|piece| self.placed_run(&piece)));
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
                    piece.glyphs.start = piece.glyphs.start.min(cell.glyphs.start);
                    piece.glyphs.end = piece.glyphs.end.max(cell.glyphs.end);
                    piece.source.start = piece.source.start.min(cell.start);
                    piece.source.end = piece.source.end.max(cell.end);
                    piece.advance_px += cell.advance_px;
                    piece.whitespace &= cell.whitespace;
                }
                _ => pieces.push(Piece {
                    run: cell.run,
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
    fn placed_run(&self, piece: &Piece) -> ShapedRun {
        let run = &self.input.runs[piece.run];
        let whole = piece.glyphs.start == 0 && piece.glyphs.end == run.glyphs.len();
        ShapedRun {
            id: run.id,
            source: piece.source.clone(),
            direction: run.direction,
            bidi_level: run.bidi_level,
            script: run.script,
            font: run.font,
            font_size_px: run.font_size_px,
            glyphs: run.glyphs[piece.glyphs.clone()].to_vec(),
            // A whole run keeps the advance the shaper reported. Only a run cut
            // by a line break is re-summed, and then from the shaper's own
            // per-glyph advances.
            advance_px: if whole {
                run.advance_px
            } else {
                piece.advance_px
            },
            // Assigned by the caller once the line's visual order is known.
            origin_x_px: 0.0,
            metrics: run.metrics,
        }
    }

    /// Reported ascent and descent (the tallest run, strut included) and the
    /// line box height (the tallest line-height request, strut included).
    fn line_box(&self, pieces: &[Piece]) -> (f32, f32, f32) {
        let mut ascent_px: f32 = 0.0;
        let mut descent_px: f32 = 0.0;
        let mut height_px = self.input.strut.map_or(0.0, |strut| strut.line_height_px);
        for piece in pieces {
            let run = &self.input.runs[piece.run];
            ascent_px = ascent_px.max(run.metrics.ascent_px);
            descent_px = descent_px.max(run.metrics.descent_px);
            height_px = height_px.max(self.input.run_line_heights[piece.run]);
        }
        if pieces.is_empty() {
            height_px = height_px.max(self.input.empty_line_height_px);
        }
        if let Some(strut) = self.input.strut {
            ascent_px = ascent_px.max(strut.metrics.ascent_px);
            descent_px = descent_px.max(strut.metrics.descent_px);
        }
        (ascent_px, descent_px, height_px)
    }

    /// Where a line of `width_px` starts inside the container.
    ///
    /// With no `max_width_px` there is no container to align in, so every
    /// keyword lays the line out at the origin. `start` / `end` follow the
    /// paragraph direction; `left` / `right` do not.
    fn align_offset(&self, width_px: f32) -> f32 {
        let Some(max_width) = self.max_width_px else {
            return 0.0;
        };
        let slack = (max_width - width_px).max(0.0);
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

/// A maximal run of consecutive cells belonging to one shaped run.
#[derive(Debug, Clone)]
struct Piece {
    run: usize,
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
    let mut end = range.end;
    for separator in ["\r\n", "\n", "\r", "\u{2029}", "\u{85}"] {
        if text[range.start..end].ends_with(separator) {
            end -= separator.len();
            break;
        }
    }
    range.start..end
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
