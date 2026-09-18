//! Editor geometry: one retained [`TextLayout`] per paragraph, and every caret,
//! hit-test and selection query an editor asks, answered from them.
//!
//! ```text
//! display text (committed + preedit)
//!   → paragraphs split at '\n'
//!   → unchanged paragraphs keep their layout; the rest lay out again
//!   → stacked vertically
//!   → hit_test / caret_rect / selection_rects / line and visual moves
//! ```
//!
//! Splitting is what makes an edit cost its paragraph. Shaping never crosses
//! a line feed and the bidirectional algorithm starts a new paragraph at one,
//! so a paragraph laid out on its own has the same lines as it does inside the
//! whole text; [`EditorGeometry::sync`] then only has to find which
//! paragraphs' bytes changed. Constraints that act on the text as a whole —
//! `max_lines`, `max_height_px`, ellipsis — or that fold line feeds into spaces
//! keep the text as one paragraph instead.
//!
//! Queries never shape or lay out. A caret blink, a selection change or a hit
//! test reads the retained layouts and nothing else.

use super::session::EditSession;
use super::state::{Composition, EditRevisions};
use crate::constraints::TextConstraints;
use crate::counters::TextWorkCounters;
use crate::edit::{Affinity, CaretPosition, CaretStop};
use crate::engine::{NativeTextEngine, TextEngine, TextEngineEpoch};
use crate::layout::{LineBox, LineBreakCause, TextLayout, TextRect};
use crate::shape::RunDirection;
use crate::source::{CompositionSegment, TextSource, TextSpan};
use crate::style::{TextKind, TextStyle};
use std::cell::Cell;
use std::ops::Range;
use std::sync::Arc;

/// The preedit, in display bytes, as layout needs to see it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompositionMarks {
    pub range: Range<usize>,
    /// The segment the IME is converting, inside `range`.
    pub target: Option<Range<usize>>,
}

impl CompositionMarks {
    pub fn of(composition: &Composition) -> Self {
        Self {
            range: composition.display_range(),
            target: composition.display_target(),
        }
    }

    /// These marks relative to a paragraph, or `None` when they miss it.
    fn within(&self, paragraph: Range<usize>) -> Option<Self> {
        let clip = |range: &Range<usize>| {
            let start = range.start.max(paragraph.start);
            let end = range.end.min(paragraph.end);
            (start < end).then(|| start - paragraph.start..end - paragraph.start)
        };
        Some(Self {
            range: clip(&self.range)?,
            target: self.target.as_ref().and_then(clip),
        })
    }
}

/// What a [`EditorGeometry::sync`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GeometrySync {
    /// Paragraphs laid out again.
    pub paragraphs_laid_out: usize,
    /// Of those, the ones whose shaping was not answered from the cache.
    pub paragraphs_reshaped: usize,
    /// Paragraphs whose retained layout was kept as it was.
    pub paragraphs_kept: usize,
    /// Whether this was an edit of text laid out under the same style,
    /// constraints and engine epoch, rather than a first layout or a
    /// relayout of everything.
    pub incremental: bool,
    /// The layouts already laid this text out: NOTHING about the geometry
    /// changed, not even a paragraph's offset or the set of them.
    ///
    /// `paragraphs_laid_out == 0` does not mean that: a deletion can drop a
    /// paragraph without laying any out (removing the line feed that made an
    /// empty first paragraph merges it away), which moves every offset after
    /// it and changes the total height. A caller caching anything derived
    /// from the geometry has to key it on this.
    pub unchanged: bool,
}

/// A caret to draw, in geometry space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretRect {
    pub x_px: f32,
    pub y_px: f32,
    pub height_px: f32,
    /// Direction of the run the caret sits in.
    pub direction: RunDirection,
}

impl CaretRect {
    pub fn rect(&self, width_px: f32) -> TextRect {
        TextRect::new(self.x_px, self.y_px, width_px, self.height_px)
    }
}

/// A point resolved to a text position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditHit {
    /// Display byte offset, on a grapheme boundary.
    pub offset: usize,
    pub affinity: Affinity,
    /// False when the point was outside the text and this is the nearest
    /// position.
    pub inside: bool,
}

#[derive(Debug, Clone)]
struct Paragraph {
    /// Display byte offset of the paragraph's first byte.
    start: usize,
    /// The paragraph's bytes, without its line feed.
    source: TextSource,
    /// Whether a line feed follows the paragraph.
    newline: bool,
    marks: Option<CompositionMarks>,
    layout: Arc<TextLayout>,
    top_px: f32,
    height_px: f32,
}

impl Paragraph {
    fn text(&self) -> &str {
        self.source.text()
    }

    fn end(&self) -> usize {
        self.start + self.source.text().len()
    }

    /// End including the line feed.
    fn next_start(&self) -> usize {
        self.end() + usize::from(self.newline)
    }
}

/// Retained per-paragraph layouts of one editor's display text.
///
/// [`Self::sync_session`] recognises "nothing changed" by the session's
/// revisions, which carry the session's identity.
#[derive(Debug, Default)]
pub struct EditorGeometry {
    paragraphs: Vec<Paragraph>,
    style: Option<TextStyle>,
    constraints: Option<TextConstraints>,
    epoch: Option<TextEngineEpoch>,
    text_len: usize,
    /// The text and composition revisions of the session this geometry was
    /// last synced from. Selection is not part of it: the display text does
    /// not depend on it.
    revisions: Option<(u64, crate::TextRevision, u64)>,
    hit_test_queries: Cell<usize>,
    caret_geometry_queries: Cell<usize>,
}

impl Clone for EditorGeometry {
    fn clone(&self) -> Self {
        Self {
            paragraphs: self.paragraphs.clone(),
            style: self.style.clone(),
            constraints: self.constraints,
            epoch: self.epoch,
            text_len: self.text_len,
            revisions: self.revisions,
            hit_test_queries: Cell::new(0),
            caret_geometry_queries: Cell::new(0),
        }
    }
}

/// Whether `constraints` let the text be laid out one paragraph at a time
/// with the same result as all at once.
fn splits_paragraphs(constraints: &TextConstraints) -> bool {
    constraints.preserve_lines
        && constraints.max_lines.is_none()
        && constraints.max_height_px.is_none()
        && !constraints.ellipsis
}

impl EditorGeometry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Brings the layouts up to date with a session's display text.
    pub fn sync_session(
        &mut self,
        engine: &mut NativeTextEngine,
        session: &EditSession,
        style: &TextStyle,
        constraints: &TextConstraints,
        counters: &mut TextWorkCounters,
    ) -> GeometrySync {
        let revisions = session.revisions();
        let revisions = (revisions.session, revisions.text, revisions.composition);
        if self.revisions == Some(revisions) && self.is_current(engine.epoch(), style, constraints)
        {
            return GeometrySync {
                paragraphs_kept: self.paragraphs.len(),
                incremental: true,
                unchanged: true,
                ..GeometrySync::default()
            };
        }
        let display = session.display_text();
        let marks = session.composition().map(CompositionMarks::of);
        let sync = self.sync(
            engine,
            &display,
            marks.as_ref(),
            style,
            constraints,
            counters,
        );
        self.revisions = Some(revisions);
        sync
    }

    /// Whether the last sync was from a session at these text and
    /// composition revisions, under this engine epoch, style and constraints.
    pub fn is_synced_with(
        &self,
        revisions: EditRevisions,
        epoch: TextEngineEpoch,
        style: &TextStyle,
        constraints: &TextConstraints,
    ) -> bool {
        self.revisions == Some((revisions.session, revisions.text, revisions.composition))
            && self.is_current(epoch, style, constraints)
    }

    fn is_current(
        &self,
        epoch: TextEngineEpoch,
        style: &TextStyle,
        constraints: &TextConstraints,
    ) -> bool {
        self.epoch == Some(epoch)
            && self.style.as_ref() == Some(style)
            && self.constraints.as_ref() == Some(constraints)
    }

    /// Brings the layouts up to date with `text`.
    ///
    /// Paragraphs whose bytes and composition marks are unchanged keep their
    /// layout; only the ones in between are laid out again, and those are what
    /// `paragraphs_relayout_from_edit` / `paragraphs_reshaped_from_edit` count.
    /// A different style, constraint or engine epoch lays every paragraph out
    /// again — shaping is still answered from the engine's cache where it can
    /// be — and is not counted as edit work.
    pub fn sync(
        &mut self,
        engine: &mut NativeTextEngine,
        text: &str,
        composition: Option<&CompositionMarks>,
        style: &TextStyle,
        constraints: &TextConstraints,
        counters: &mut TextWorkCounters,
    ) -> GeometrySync {
        let epoch = engine.epoch();
        // The layouts already lay this text out. A caller without a session's
        // revisions -- a host answering probes -- syncs before every probe, so
        // recognising it here is what keeps an unchanged probe a comparison
        // instead of a rebuild of every paragraph. The last sync's revisions
        // still describe these layouts, so they survive.
        if self.lays_out(text, composition, epoch, style, constraints) {
            return GeometrySync {
                paragraphs_kept: self.paragraphs.len(),
                incremental: true,
                unchanged: true,
                ..GeometrySync::default()
            };
        }
        self.revisions = None;
        let current = self.is_current(epoch, style, constraints) && !self.paragraphs.is_empty();
        let split = splits_paragraphs(constraints);

        if !current {
            self.paragraphs.clear();
        }
        // The retained paragraphs stay where they are and are spliced in
        // place: an edit near the top of a long document shifts every
        // paragraph after it, and moving them all into a fresh vector costs
        // more than the one layout the edit actually owes.
        let old = &self.paragraphs;
        let old_count = old.len();
        let old_len = self.text_len;
        let marks_for = |range: Range<usize>| composition.and_then(|marks| marks.within(range));

        // Paragraphs that still start the text.
        let mut prefix = 0;
        let mut prefix_end = 0;
        while let Some(paragraph) = old.get(prefix) {
            let end = paragraph.end();
            let same = text.get(paragraph.start..end) == Some(paragraph.text())
                && if paragraph.newline {
                    text.as_bytes().get(end) == Some(&b'\n')
                } else {
                    text.len() == end
                }
                && marks_for(paragraph.start..end) == paragraph.marks;
            if !same {
                break;
            }
            prefix_end = paragraph.next_start();
            prefix += 1;
        }
        // Paragraphs that still end it, shifted by the length change.
        let mut suffix = 0;
        let mut suffix_start = text.len();
        // Unsplit text is one paragraph that has to be laid out whole: a
        // shifted tail is not a paragraph of its own.
        if split && prefix < old_count {
            while suffix < old_count - prefix {
                let paragraph = &old[old_count - 1 - suffix];
                let Some(start) = (paragraph.start + text.len()).checked_sub(old_len) else {
                    break;
                };
                let end = start + paragraph.text().len();
                let same = start >= prefix_end
                    && (start == 0 || text.as_bytes().get(start - 1) == Some(&b'\n'))
                    && text.get(start..end) == Some(paragraph.text())
                    && if paragraph.newline {
                        text.as_bytes().get(end) == Some(&b'\n')
                    } else {
                        text.len() == end
                    }
                    && marks_for(start..end) == paragraph.marks;
                if !same {
                    break;
                }
                suffix_start = start;
                suffix += 1;
            }
        }

        let finished = prefix > 0 && !old[prefix - 1].newline;

        // The bytes in between, as fresh paragraphs.
        let mut ranges = Vec::new();
        if !finished {
            let region_end = if suffix > 0 { suffix_start } else { text.len() };
            if split {
                let mut start = prefix_end;
                for (index, _) in text[prefix_end..region_end].match_indices('\n') {
                    let end = prefix_end + index;
                    ranges.push((start..end, true));
                    start = end + 1;
                }
                // Up to a kept suffix the region ends with its line feed; at
                // the end of the text the tail is a paragraph even when empty,
                // which is the line a caret after a final line feed sits on.
                if suffix == 0 {
                    ranges.push((start..region_end, false));
                }
            } else {
                ranges.push((prefix_end..region_end, false));
            }
        }
        let mut sync = GeometrySync {
            incremental: current,
            ..GeometrySync::default()
        };
        let mut fresh = Vec::with_capacity(ranges.len());
        for (range, newline) in ranges {
            let marks = marks_for(range.clone());
            let source = paragraph_source(&text[range.clone()], marks.as_ref(), style);
            let mut work = TextWorkCounters::default();
            let layout = engine.layout(TextKind::Editable, &source, style, constraints, &mut work);
            sync.paragraphs_laid_out += 1;
            if work.shape_cache_misses.unwrap_or(0) > 0 {
                sync.paragraphs_reshaped += 1;
            }
            counters.accumulate(work);
            fresh.push(Paragraph {
                start: range.start,
                source,
                newline,
                marks,
                height_px: paragraph_height(&layout),
                layout,
                top_px: 0.0,
            });
        }
        // Replace the paragraphs between the kept prefix and the kept suffix,
        // then move the suffix's starts by what the edit changed in length.
        let fresh_count = fresh.len();
        self.paragraphs.splice(prefix..old_count - suffix, fresh);
        let delta = text.len() as isize - old_len as isize;
        if delta != 0 {
            for paragraph in &mut self.paragraphs[prefix + fresh_count..] {
                paragraph.start = (paragraph.start as isize + delta) as usize;
            }
        }
        sync.paragraphs_kept = prefix + suffix;
        if current {
            counters.paragraphs_relayout_from_edit += sync.paragraphs_laid_out;
            counters.paragraphs_reshaped_from_edit += sync.paragraphs_reshaped;
        }

        let mut top = 0.0;
        for paragraph in &mut self.paragraphs {
            paragraph.top_px = top;
            top += paragraph.height_px;
        }
        self.text_len = text.len();
        self.style = Some(style.clone());
        self.constraints = Some(*constraints);
        self.epoch = Some(epoch);
        sync
    }

    /// Whether the retained paragraphs already lay `text` out with these
    /// composition marks, under this engine epoch, style and constraints.
    ///
    /// Compares the bytes: a caller that can name its text cheaply (a session,
    /// through [`Self::sync_session`]) never reaches this.
    fn lays_out(
        &self,
        text: &str,
        composition: Option<&CompositionMarks>,
        epoch: TextEngineEpoch,
        style: &TextStyle,
        constraints: &TextConstraints,
    ) -> bool {
        if self.text_len != text.len()
            || self.paragraphs.is_empty()
            || !self.is_current(epoch, style, constraints)
        {
            return false;
        }
        let mut next = 0;
        for paragraph in &self.paragraphs {
            let end = paragraph.end();
            let same = paragraph.start == next
                && text.get(paragraph.start..end) == Some(paragraph.text())
                && if paragraph.newline {
                    text.as_bytes().get(end) == Some(&b'\n')
                } else {
                    text.len() == end
                }
                && composition.and_then(|marks| marks.within(paragraph.start..end))
                    == paragraph.marks;
            if !same {
                return false;
            }
            next = paragraph.next_start();
        }
        next == text.len()
    }

    /// The text and composition revisions of the session last synced from.
    pub(super) fn synced_revisions(&self) -> Option<(u64, crate::TextRevision, u64)> {
        self.revisions
    }

    /// Queries answered since the last call: `(hit tests, caret geometries)`.
    pub fn take_query_counts(&self) -> (usize, usize) {
        (
            self.hit_test_queries.replace(0),
            self.caret_geometry_queries.replace(0),
        )
    }

    /// Adds the queries answered since the last call to `counters`.
    pub fn record_queries(&self, counters: &mut TextWorkCounters) {
        let (hits, carets) = self.take_query_counts();
        counters.hit_test_queries += hits;
        counters.caret_geometry_queries += carets;
    }

    /// Length of the display text the layouts were synced to.
    pub fn text_len(&self) -> usize {
        self.text_len
    }

    pub fn paragraph_count(&self) -> usize {
        self.paragraphs.len()
    }

    /// Each paragraph's display offset, vertical offset and layout, in order.
    /// A painter draws each layout translated by its offset.
    pub fn paragraph_layouts(&self) -> impl Iterator<Item = (usize, f32, &Arc<TextLayout>)> {
        self.paragraphs
            .iter()
            .map(|paragraph| (paragraph.start, paragraph.top_px, &paragraph.layout))
    }

    /// Widest line and total height.
    pub fn size(&self) -> (f32, f32) {
        let width = self
            .paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.layout.lines.iter())
            .map(|line| line.metrics.width_px)
            .fold(0.0, f32::max);
        let height = self
            .paragraphs
            .last()
            .map_or(0.0, |last| last.top_px + last.height_px);
        (width, height)
    }

    pub fn line_count(&self) -> usize {
        self.paragraphs
            .iter()
            .map(|paragraph| paragraph.layout.lines.len())
            .sum()
    }

    /// The paragraph a display offset belongs to. The offset of a line feed —
    /// a paragraph's end — belongs to the paragraph before it.
    fn paragraph_index(&self, offset: usize) -> Option<usize> {
        if self.paragraphs.is_empty() {
            return None;
        }
        let after = self
            .paragraphs
            .partition_point(|paragraph| paragraph.start <= offset);
        Some(after.saturating_sub(1))
    }

    /// The line of a paragraph a caret at local byte `byte` sits on.
    fn line_of(layout: &TextLayout, byte: usize, affinity: Affinity) -> Option<&LineBox> {
        let lines = &layout.lines;
        let index = lines
            .partition_point(|line| line.source.start <= byte)
            .saturating_sub(1);
        let line = lines.get(index)?;
        if affinity == Affinity::Upstream
            && index > 0
            && byte == line.source.start
            && lines[index - 1].break_cause == LineBreakCause::Wrap
            && byte >= lines[index - 1].source.end
        {
            return Some(&lines[index - 1]);
        }
        Some(line)
    }

    /// Where to draw a caret at a display offset.
    pub fn caret_rect(&self, offset: usize, affinity: Affinity) -> Option<CaretRect> {
        self.caret_geometry_queries
            .set(self.caret_geometry_queries.get() + 1);
        let paragraph = &self.paragraphs[self.paragraph_index(offset)?];
        let byte = offset
            .checked_sub(paragraph.start)?
            .min(paragraph.text().len());
        let line = Self::line_of(&paragraph.layout, byte, affinity)?;
        let geometry = paragraph
            .layout
            .caret_geometry(CaretPosition::new(byte, affinity, line.index))?;
        Some(CaretRect {
            x_px: geometry.x_px,
            y_px: paragraph.top_px + geometry.top_y_px,
            height_px: geometry.height_px,
            direction: geometry.direction,
        })
    }

    /// The display position nearest a point.
    pub fn hit_test(&self, x_px: f32, y_px: f32) -> EditHit {
        self.hit_test_queries.set(self.hit_test_queries.get() + 1);
        let Some(last) = self.paragraphs.last() else {
            return EditHit {
                offset: 0,
                affinity: Affinity::Downstream,
                inside: false,
            };
        };
        let index = self
            .paragraphs
            .partition_point(|paragraph| paragraph.top_px + paragraph.height_px <= y_px)
            .min(self.paragraphs.len() - 1);
        let paragraph = &self.paragraphs[index];
        let inside_y = y_px >= 0.0 && y_px < last.top_px + last.height_px;
        let hit = paragraph
            .layout
            .hit_test_text(paragraph.text(), x_px, y_px - paragraph.top_px);
        EditHit {
            offset: paragraph.start + hit.caret.byte,
            affinity: hit.caret.affinity,
            inside: hit.inside && inside_y,
        }
    }

    /// Rectangles covering a display byte range: one or more per line, never
    /// assumed contiguous.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<TextRect> {
        if range.start >= range.end {
            return Vec::new();
        }
        let first = self
            .paragraphs
            .partition_point(|paragraph| paragraph.end() < range.start);
        let mut rects = Vec::new();
        for paragraph in &self.paragraphs[first.min(self.paragraphs.len())..] {
            if paragraph.start >= range.end {
                break;
            }
            let local = range.start.saturating_sub(paragraph.start)
                ..(range.end - paragraph.start).min(paragraph.text().len());
            rects.extend(
                paragraph
                    .layout
                    .selection_rects(local)
                    .into_iter()
                    .map(|rect| {
                        TextRect::new(rect.x, rect.y + paragraph.top_px, rect.width, rect.height)
                    }),
            );
        }
        rects
    }

    /// The visual line a caret sits on, as `(paragraph index, line index)`.
    fn locate(&self, offset: usize, affinity: Affinity) -> Option<(usize, usize)> {
        let index = self.paragraph_index(offset)?;
        let paragraph = &self.paragraphs[index];
        let byte = offset
            .checked_sub(paragraph.start)?
            .min(paragraph.text().len());
        let line = Self::line_of(&paragraph.layout, byte, affinity)?;
        Some((index, line.index as usize))
    }

    /// Base direction of the visual line a caret sits on.
    pub fn line_direction(&self, offset: usize, affinity: Affinity) -> Option<RunDirection> {
        let (paragraph, line) = self.locate(offset, affinity)?;
        Some(
            self.paragraphs[paragraph]
                .layout
                .lines
                .get(line)?
                .base_direction,
        )
    }

    /// Start and end of the visual line a caret sits on, as carets.
    pub fn line_bounds(
        &self,
        offset: usize,
        affinity: Affinity,
    ) -> Option<((usize, Affinity), (usize, Affinity))> {
        let (paragraph_index, line_index) = self.locate(offset, affinity)?;
        let paragraph = &self.paragraphs[paragraph_index];
        let line = paragraph.layout.lines.get(line_index)?;
        let end_affinity = if line.break_cause == LineBreakCause::Wrap {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        Some((
            (paragraph.start + line.source.start, Affinity::Downstream),
            (paragraph.start + line.source.end, end_affinity),
        ))
    }

    /// The caret `lines` visual lines above (negative) or below a caret,
    /// nearest `goal_x_px`. Clamps onto the first or last line; `None` when
    /// the geometry is empty.
    pub fn vertical(
        &self,
        offset: usize,
        affinity: Affinity,
        lines: isize,
        goal_x_px: f32,
    ) -> Option<(usize, Affinity)> {
        let (mut paragraph_index, mut line_index) = self.locate(offset, affinity)?;
        let mut remaining = lines;
        while remaining < 0 {
            if line_index > 0 {
                line_index -= 1;
            } else if paragraph_index > 0 {
                paragraph_index -= 1;
                line_index = self.paragraphs[paragraph_index]
                    .layout
                    .lines
                    .len()
                    .saturating_sub(1);
            } else {
                return Some((0, Affinity::Downstream));
            }
            remaining += 1;
        }
        while remaining > 0 {
            let count = self.paragraphs[paragraph_index].layout.lines.len();
            if line_index + 1 < count {
                line_index += 1;
            } else if paragraph_index + 1 < self.paragraphs.len() {
                paragraph_index += 1;
                line_index = 0;
            } else {
                return Some((self.text_len, Affinity::Downstream));
            }
            remaining -= 1;
        }
        let paragraph = &self.paragraphs[paragraph_index];
        let line = paragraph.layout.lines.get(line_index)?;
        let y = line.metrics.top_y_px + line.metrics.height_px * 0.5;
        let hit = paragraph
            .layout
            .hit_test_text(paragraph.text(), goal_x_px, y);
        Some((paragraph.start + hit.caret.byte, hit.caret.affinity))
    }

    /// The caret one position visually left or right of a caret, crossing
    /// onto the neighbouring line at a line's edge. `None` at the edge of the
    /// text.
    ///
    /// Visual order is what arrow keys follow on platforms that move visually:
    /// in `abc ابج` a right arrow walks through the Arabic word right to left
    /// in logical terms, because that is left to right on screen.
    pub fn visual_move(
        &self,
        offset: usize,
        affinity: Affinity,
        rightwards: bool,
    ) -> Option<(usize, Affinity)> {
        let (paragraph_index, line_index) = self.locate(offset, affinity)?;
        let paragraph = &self.paragraphs[paragraph_index];
        let line = paragraph.layout.lines.get(line_index)?;
        let byte = offset - paragraph.start;
        let stops = paragraph.layout.caret_stops(line.index, paragraph.text());
        let current = paragraph
            .layout
            .caret_geometry(CaretPosition::new(byte, affinity, line.index))
            .map(|caret| caret.x_px)?;
        const SAME_X: f32 = 0.01;
        // Step through the stops in order, so positions sharing an x are
        // each one key press. A caret that is not itself a stop (an affinity
        // drawn at the same place as its twin) starts from the stop it draws
        // at.
        let index = stops
            .iter()
            .position(|stop| stop.caret.byte == byte && stop.caret.affinity == affinity)
            .or_else(|| {
                stops.iter().position(|stop| {
                    stop.caret.byte == byte && (stop.x_px - current).abs() <= SAME_X
                })
            })
            .or_else(|| {
                // Drawn where other positions are: start from the first of
                // them in the direction of travel, so none is skipped.
                let same_x = |stop: &CaretStop| (stop.x_px - current).abs() <= SAME_X;
                if rightwards {
                    stops.iter().position(same_x)
                } else {
                    stops.iter().rposition(same_x)
                }
            });
        let next = match index {
            Some(index) if rightwards => stops.get(index + 1),
            Some(index) => index.checked_sub(1).and_then(|index| stops.get(index)),
            None if rightwards => stops.iter().find(|stop| stop.x_px > current + SAME_X),
            None => stops.iter().rev().find(|stop| stop.x_px < current - SAME_X),
        };
        if let Some(stop) = next {
            return Some((paragraph.start + stop.caret.byte, stop.caret.affinity));
        }
        // Past the line's edge: onward in the paragraph's reading order.
        let forward = rightwards != line.base_direction.is_rtl();
        let (target_paragraph, target_line) = if forward {
            if line_index + 1 < paragraph.layout.lines.len() {
                (paragraph_index, line_index + 1)
            } else if paragraph_index + 1 < self.paragraphs.len() {
                (paragraph_index + 1, 0)
            } else {
                return None;
            }
        } else if line_index > 0 {
            (paragraph_index, line_index - 1)
        } else if paragraph_index > 0 {
            let previous = &self.paragraphs[paragraph_index - 1];
            (
                paragraph_index - 1,
                previous.layout.lines.len().saturating_sub(1),
            )
        } else {
            return None;
        };
        let target = &self.paragraphs[target_paragraph];
        let target_stops = target.layout.caret_stops(target_line as u32, target.text());
        // Entering a line from its reading-order start: the stop at the far
        // side in the direction of travel's opposite.
        let stop = if rightwards {
            target_stops.first()
        } else {
            target_stops.last()
        }?;
        Some((target.start + stop.caret.byte, stop.caret.affinity))
    }
}

fn paragraph_height(layout: &TextLayout) -> f32 {
    layout
        .lines
        .iter()
        .map(|line| line.metrics.height_px)
        .filter(|height| height.is_finite())
        .sum()
}

/// A paragraph's source, with the preedit marked so it shapes as its own
/// runs and a painter can decorate it.
fn paragraph_source(text: &str, marks: Option<&CompositionMarks>, style: &TextStyle) -> TextSource {
    let mut source = TextSource::new(text);
    if let Some(marks) = marks {
        let span = |range: Range<usize>, segment| TextSpan {
            range,
            style: style.clone(),
            composition: Some(segment),
        };
        let mut spans = Vec::with_capacity(3);
        match marks.target.clone() {
            Some(target) => {
                if marks.range.start < target.start {
                    spans.push(span(
                        marks.range.start..target.start,
                        CompositionSegment::Preedit,
                    ));
                }
                spans.push(span(target.clone(), CompositionSegment::PreeditTarget));
                if target.end < marks.range.end {
                    spans.push(span(
                        target.end..marks.range.end,
                        CompositionSegment::Preedit,
                    ));
                }
            }
            None => spans.push(span(marks.range.clone(), CompositionSegment::Preedit)),
        }
        source.set_composition(spans);
    }
    source
}
