//! Caret, hit-test and selection geometry, derived purely from a
//! [`TextLayout`]. No engine, no font access, no shaping.
//!
//! This module is the IR's own acceptance test. If a caret cannot be placed
//! from the layout alone, the layout is missing a field, and that is a Phase-0
//! finding rather than something to paper over in the engine later.

use crate::layout::{LineBox, TextLayout, TextRect};
use crate::shape::RunDirection;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Which side of a wrap boundary a caret belongs to.
///
/// At a soft wrap one byte offset has two positions: the end of the line
/// before (`Upstream`) and the start of the line after (`Downstream`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Affinity {
    Upstream,
    #[default]
    Downstream,
}

/// A caret, in source bytes plus the line it resolved onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct CaretPosition {
    pub byte: usize,
    #[serde(default)]
    pub affinity: Affinity,
    #[serde(default)]
    pub line: u32,
}

impl CaretPosition {
    pub const fn new(byte: usize, affinity: Affinity, line: u32) -> Self {
        Self {
            byte,
            affinity,
            line,
        }
    }
}

/// Where to draw a caret.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CaretGeometry {
    pub x_px: f32,
    pub top_y_px: f32,
    pub height_px: f32,
    /// Direction of the run the caret sits in — what decides which side of a
    /// BiDi boundary it paints on.
    pub direction: RunDirection,
}

/// The answer to a pointer hit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HitTestResult {
    pub caret: CaretPosition,
    /// False when the point fell outside the laid-out bounds and the caret is
    /// a clamped best effort.
    pub inside: bool,
}

/// How close two cell edges must be to count as touching, in px.
///
/// Not `f32::EPSILON`: that is the ULP at 1.0, while these are pixel
/// coordinates in the hundreds, where one ULP already exceeds it. A run's
/// `origin_x_px` and the prefix sum of the previous run's advances are computed
/// independently, so abutting runs routinely differ in the last bit or two, and
/// an exact test would split one selection rectangle into two. Any real BiDi
/// gap is whole glyphs wide, far above this.
const CELL_JOIN_TOLERANCE_PX: f32 = 0.01;

/// Which side of a line a byte of hung whitespace belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GapSide {
    /// After the line's last drawn byte.
    After,
    /// Before its first.
    Before,
}

/// One glyph's advance cell, flattened across the runs of a line.
#[derive(Clone, Copy)]
struct Cell {
    left: f32,
    right: f32,
    cluster: usize,
    cluster_end: usize,
    direction: RunDirection,
}

impl Cell {
    /// X of a caret at `byte`, which may sit strictly inside the cluster.
    ///
    /// One glyph can cover several source bytes -- a ligature, or a multi-byte
    /// character -- and a caret between them still has to land somewhere. The
    /// position is interpolated across the cell so interior offsets stay
    /// ordered and distinct; collapsing them onto the trailing edge would put
    /// the caret for "of|fice" after the whole `ffi` ligature.
    fn x_for_byte(&self, byte: usize) -> f32 {
        let span = self.cluster_end.saturating_sub(self.cluster);
        let offset = byte.saturating_sub(self.cluster);
        let fraction = if span == 0 {
            0.0
        } else {
            offset as f32 / span as f32
        };
        let width = self.right - self.left;
        match self.direction {
            RunDirection::Ltr => self.left + width * fraction,
            // RTL advances leftwards, so a later byte is further left.
            RunDirection::Rtl => self.right - width * fraction,
        }
    }

    /// How far `x_px` lies outside this cell; zero when it is inside.
    fn distance_to(&self, x_px: f32) -> f32 {
        if x_px < self.left {
            self.left - x_px
        } else if x_px >= self.right {
            x_px - self.right
        } else {
            0.0
        }
    }

    /// Source byte at the visually-left edge of this cell.
    fn leading_byte(&self) -> usize {
        match self.direction {
            RunDirection::Ltr => self.cluster,
            RunDirection::Rtl => self.cluster_end,
        }
    }

    /// Source byte at the visually-right edge of this cell.
    fn trailing_byte(&self) -> usize {
        match self.direction {
            RunDirection::Ltr => self.cluster_end,
            RunDirection::Rtl => self.cluster,
        }
    }
}

/// A line's cells in visual order, plus their order by cluster so a byte can
/// be resolved to its cell by binary search rather than a scan of the line.
struct LineCells {
    cells: Vec<Cell>,
    /// Indices into `cells`, sorted by `(cluster, cluster_end)`. Built only
    /// for many queries on one line; a single query scans.
    by_cluster: Option<Vec<u32>>,
}

impl LineCells {
    fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Every cell rendering the first cluster that satisfies `covers`, merged
    /// into one visual extent.
    ///
    /// Clusters do not overlap, so over `by_cluster` both their starts and
    /// their ends ascend: `before` (true for clusters wholly before `byte`) is
    /// a partition, and no cluster starting after `byte` can cover it.
    fn merged(
        &self,
        byte: usize,
        covers: impl Fn(&Cell) -> bool,
        before: impl Fn(&Cell) -> bool,
    ) -> Option<Cell> {
        let Some(by_cluster) = &self.by_cluster else {
            // One query on a line: a scan is cheaper than sorting an index.
            let found = *self.cells.iter().find(|cell| covers(cell))?;
            return Some(self.cells.iter().fold(found, |mut merged, cell| {
                if (cell.cluster, cell.cluster_end) == (found.cluster, found.cluster_end) {
                    merged.left = merged.left.min(cell.left);
                    merged.right = merged.right.max(cell.right);
                }
                merged
            }));
        };
        let first = by_cluster.partition_point(|index| before(&self.cells[*index as usize]));
        let found = by_cluster[first..]
            .iter()
            .map(|index| &self.cells[*index as usize])
            .take_while(|cell| cell.cluster <= byte)
            .find(|cell| covers(cell))?;
        let (cluster, cluster_end) = (found.cluster, found.cluster_end);
        let mut merged = Cell { ..*found };
        // Cells of one cluster are adjacent in `by_cluster`: combining marks
        // share their base's cluster range.
        let start = by_cluster.partition_point(|index| {
            let cell = &self.cells[*index as usize];
            (cell.cluster, cell.cluster_end) < (cluster, cluster_end)
        });
        for index in &by_cluster[start..] {
            let cell = &self.cells[*index as usize];
            if (cell.cluster, cell.cluster_end) != (cluster, cluster_end) {
                break;
            }
            merged.left = merged.left.min(cell.left);
            merged.right = merged.right.max(cell.right);
        }
        Some(merged)
    }

    /// The cluster `byte` falls in: its first byte or an interior one.
    fn containing(&self, byte: usize) -> Option<Cell> {
        self.merged(
            byte,
            |cell| byte >= cell.cluster && byte < cell.cluster_end,
            |cell| cell.cluster_end <= byte,
        )
    }

    /// The cluster that ends exactly at `byte` or runs through it.
    fn ending_at(&self, byte: usize) -> Option<Cell> {
        self.merged(
            byte,
            |cell| byte > cell.cluster && byte <= cell.cluster_end,
            |cell| cell.cluster_end < byte,
        )
    }
}

impl TextLayout {
    fn cells(&self, line: &LineBox) -> LineCells {
        LineCells {
            cells: self.visual_cells(line),
            by_cluster: None,
        }
    }

    /// [`Self::cells`] with the cluster index, for a pass that resolves every
    /// position of a line.
    fn indexed_cells(&self, line: &LineBox) -> LineCells {
        let cells = self.visual_cells(line);
        let mut by_cluster: Vec<u32> = (0..cells.len() as u32).collect();
        by_cluster.sort_by_key(|index| {
            let cell = &cells[*index as usize];
            (cell.cluster, cell.cluster_end)
        });
        LineCells {
            cells,
            by_cluster: Some(by_cluster),
        }
    }

    fn visual_cells(&self, line: &LineBox) -> Vec<Cell> {
        let mut cells = Vec::new();
        for run in self.line_runs(line) {
            for (left, glyph) in run.glyph_cells() {
                cells.push(Cell {
                    left,
                    right: left + glyph.advance_px,
                    cluster: glyph.cluster as usize,
                    cluster_end: glyph.cluster_end as usize,
                    direction: run.direction,
                });
            }
        }
        cells
    }

    fn line_at_y(&self, y_px: f32) -> Option<(&LineBox, bool)> {
        let first = self.lines.first()?;
        if y_px < first.bounds.y {
            return Some((first, false));
        }
        // Lines stack downwards, so their bottoms ascend.
        let below = self
            .lines
            .partition_point(|line| y_px >= line.bounds.bottom());
        match self.lines.get(below) {
            Some(line) => Some((line, true)),
            None => self.lines.last().map(|line| (line, false)),
        }
    }

    fn line_by_index(&self, index: u32) -> Option<&LineBox> {
        // Indices are assigned in order; fall back to a scan for a
        // hand-built layout that numbers them otherwise.
        match self.lines.get(index as usize) {
            Some(line) if line.index == index => Some(line),
            _ => self.lines.iter().find(|line| line.index == index),
        }
    }

    /// Whether `byte` falls in the gap of hung whitespace after `line`, or in
    /// the one before it.
    ///
    /// A soft wrap hangs the whitespace it broke at: those bytes are drawn by
    /// nobody and belong to no line's `source`, but they are still caret
    /// positions the user can arrow into. They resolve to the edge of whichever
    /// line they hang from.
    fn gap_side(&self, line: &LineBox, byte: usize) -> Option<GapSide> {
        let index = self
            .lines
            .iter()
            .position(|candidate| candidate.index == line.index)?;
        if byte > line.source.end {
            let next = self.lines.get(index + 1)?;
            return (byte <= next.source.start).then_some(GapSide::After);
        }
        if byte < line.source.start {
            let previous = self.lines.get(index.checked_sub(1)?)?;
            return (byte >= previous.source.end).then_some(GapSide::Before);
        }
        None
    }

    /// Byte at the visually-left / visually-right edge of a whole line.
    fn line_edge_bytes(&self, line: &LineBox) -> (usize, usize) {
        match line.base_direction {
            RunDirection::Ltr => (line.source.start, line.source.end),
            RunDirection::Rtl => (line.source.end, line.source.start),
        }
    }

    /// Resolves a point in layout space to a caret on a cluster edge.
    ///
    /// A point outside the laid-out bounds still yields the nearest caret, with
    /// `inside == false`, because callers drag-select past the edges.
    ///
    /// This is the IR-only answer the migration corpus records. An editor that
    /// has the text uses [`Self::hit_test_text`], which can also land between
    /// the graphemes of a ligature.
    pub fn hit_test(&self, x_px: f32, y_px: f32) -> HitTestResult {
        self.hit(x_px, y_px, None)
    }

    /// [`Self::hit_test`] with the source text: the caret lands on the nearest
    /// grapheme boundary even inside one glyph that draws several graphemes (a
    /// ligature), and its affinity names the cluster that was clicked, so a
    /// click on either side of a BiDi boundary draws the caret where the
    /// pointer is.
    ///
    /// `text` is the text this layout was produced from.
    pub fn hit_test_text(&self, text: &str, x_px: f32, y_px: f32) -> HitTestResult {
        self.hit(x_px, y_px, Some(text))
    }

    fn hit(&self, x_px: f32, y_px: f32, text: Option<&str>) -> HitTestResult {
        let Some((line, inside_y)) = self.line_at_y(y_px) else {
            return HitTestResult {
                caret: CaretPosition::default(),
                inside: false,
            };
        };
        let (left_byte, right_byte) = self.line_edge_bytes(line);
        let cells = self.cells(line);
        let inside_x = x_px >= line.bounds.x && x_px < line.bounds.right();

        let past_left = !cells.is_empty() && x_px < cells.cells[0].left;
        let past_right = !cells.is_empty() && x_px >= cells.cells[cells.cells.len() - 1].right;
        let caret = if cells.is_empty() {
            CaretPosition::new(line.source.start, Affinity::Downstream, line.index)
        } else if (past_left || past_right)
            && let Some(text) = text
            && let Some(stop) = {
                // With the text, the position drawn at that edge — not the
                // line's logical boundary, which a wrapped line ending in
                // opposite-direction text draws at the other side.
                let stops = self.caret_stops(line.index, text);
                if past_left {
                    stops.first().copied()
                } else {
                    stops.last().copied()
                }
            }
        {
            stop.caret
        } else if past_left {
            CaretPosition::new(left_byte, edge_affinity(line, left_byte), line.index)
        } else if past_right {
            CaretPosition::new(right_byte, edge_affinity(line, right_byte), line.index)
        } else {
            let cell = cells
                .cells
                .iter()
                .find(|cell| x_px >= cell.left && x_px < cell.right)
                .unwrap_or_else(|| {
                    // Cells are not always contiguous: alignment and trailing
                    // whitespace leave gaps between runs. A point in a gap
                    // belongs to the run beside it, not to the start of the
                    // line.
                    cells
                        .cells
                        .iter()
                        .min_by(|a, b| a.distance_to(x_px).total_cmp(&b.distance_to(x_px)))
                        .expect("the empty case returned above")
                });
            match text {
                None => {
                    let midpoint = (cell.left + cell.right) * 0.5;
                    let byte = if x_px < midpoint {
                        cell.leading_byte()
                    } else {
                        cell.trailing_byte()
                    };
                    CaretPosition::new(byte, Affinity::Downstream, line.index)
                }
                Some(text) => self.hit_cluster(line, &cells, *cell, text, x_px),
            }
        };

        HitTestResult {
            caret,
            inside: inside_y && inside_x,
        }
    }

    /// The grapheme boundary of the clicked cluster nearest `x_px`.
    fn hit_cluster(
        &self,
        line: &LineBox,
        cells: &LineCells,
        cell: Cell,
        text: &str,
        x_px: f32,
    ) -> CaretPosition {
        let cluster = cells.containing(cell.cluster).unwrap_or(cell);
        let mut best = (cluster.leading_byte(), f32::INFINITY);
        // Segmented from the line's start, not the cluster's: a grapheme rule
        // can depend on what precedes (regional indicator pairs).
        let line_start = line.source.start.min(cluster.cluster);
        if let Some(graphemes) = text.get(line_start..cluster.cluster_end) {
            let boundaries = graphemes
                .grapheme_indices(true)
                .map(|(index, _)| line_start + index)
                .skip_while(|byte| *byte < cluster.cluster)
                .chain(std::iter::once(cluster.cluster_end));
            for byte in boundaries {
                let distance = (cluster.x_for_byte(byte) - x_px).abs();
                if distance < best.1 {
                    best = (byte, distance);
                }
            }
        } else {
            // Not the text this layout came from; the cluster edges are all
            // that can be trusted.
            let midpoint = (cluster.left + cluster.right) * 0.5;
            best.0 = if x_px < midpoint {
                cluster.leading_byte()
            } else {
                cluster.trailing_byte()
            };
        }
        let byte = best.0;
        let affinity =
            if byte == line.source.end && line.break_cause == crate::layout::LineBreakCause::Wrap {
                Affinity::Upstream
            } else if byte == cluster.cluster_end && byte != cluster.cluster {
                // The clicked cluster's logical end. Downstream would draw at the
                // start of whatever follows it logically, which across a BiDi
                // boundary is somewhere else on the line.
                let upstream = cluster.x_for_byte(byte);
                let downstream = self
                    .place_caret(line, cells, byte, Affinity::Downstream)
                    .map(|(x, _)| x);
                if downstream.is_some_and(|x| (x - upstream).abs() <= CELL_JOIN_TOLERANCE_PX) {
                    Affinity::Downstream
                } else {
                    Affinity::Upstream
                }
            } else {
                Affinity::Downstream
            };
        CaretPosition::new(byte, affinity, line.index)
    }

    /// Where to draw the given caret, or `None` if it names a line this layout
    /// does not have.
    ///
    /// Affinity decides between the two clusters that meet at a byte: an
    /// upstream caret draws at the logical end of the cluster before it, a
    /// downstream one at the logical start of the cluster after it. Within one
    /// direction both are the same x; across a BiDi boundary they are the two
    /// visually distinct places that byte can be.
    pub fn caret_geometry(&self, caret: CaretPosition) -> Option<CaretGeometry> {
        let line = self.line_by_index(caret.line)?;
        let cells = self.cells(line);
        let (x_px, direction) = self.place_caret(line, &cells, caret.byte, caret.affinity)?;
        Some(CaretGeometry {
            x_px,
            top_y_px: line.metrics.top_y_px,
            height_px: line.metrics.height_px,
            direction,
        })
    }

    fn place_caret(
        &self,
        line: &LineBox,
        cells: &LineCells,
        byte: usize,
        affinity: Affinity,
    ) -> Option<(f32, RunDirection)> {
        let cluster = match affinity {
            Affinity::Upstream => cells.ending_at(byte).or_else(|| cells.containing(byte)),
            Affinity::Downstream => cells.containing(byte),
        };
        if let Some(cell) = cluster {
            return Some((cell.x_for_byte(byte), cell.direction));
        }
        // Not inside any cluster: it is one of the line's two edges.
        //
        // Both edges come from the cells, because those are what `hit_test`
        // compares against when it decides a point is past one end of the
        // line. Reading `bounds.x` for the left edge would disagree with it on
        // any line whose first glyph does not start at the line box edge -- a
        // centred or indented line -- and the caret would jump away from the
        // click that placed it. An empty line has no cells, and then the line
        // box is all there is to go on.
        let (left_byte, right_byte) = self.line_edge_bytes(line);
        let empty = line.bounds.x;
        let left_edge = cells.cells.first().map_or(empty, |cell| cell.left);
        let right_edge = cells.cells.last().map_or(empty, |cell| cell.right);
        let logical_end = match self.gap_side(line, byte) {
            // Hung whitespace: the bytes between two lines are drawn by
            // neither, and a caret in them sits at the end of the line it
            // hangs from.
            Some(GapSide::After) => true,
            Some(GapSide::Before) => false,
            None if byte == left_byte => line.base_direction.is_rtl(),
            None if byte == right_byte => !line.base_direction.is_rtl(),
            None => return None,
        };
        let edge = if logical_end == line.base_direction.is_rtl() {
            left_edge
        } else {
            right_edge
        };
        Some((edge, line.base_direction))
    }

    /// Every caret position on one line, ordered left to right, for moving a
    /// caret visually through mixed-direction text. Positions at the same x
    /// are ordered in the line's reading order.
    ///
    /// Each grapheme boundary of the line contributes its downstream position
    /// and, where it draws elsewhere, its upstream one. The logical end of a
    /// soft-wrapped line contributes only the upstream position: downstream it
    /// belongs to the next line. Positions inside whitespace the wrap hung
    /// after the line are upstream stops at its end. Empty when `text` is not the text this layout
    /// was produced from, or the line does not exist.
    pub fn caret_stops(&self, line_index: u32, text: &str) -> Vec<CaretStop> {
        let Some(line) = self.line_by_index(line_index) else {
            return Vec::new();
        };
        let Some(line_text) = text.get(line.source.clone()) else {
            return Vec::new();
        };
        let cells = self.indexed_cells(line);
        let wrapped = line.break_cause == crate::layout::LineBreakCause::Wrap;
        let mut stops = Vec::new();
        // A line starts on a grapheme boundary, so its own text segments the
        // same as the whole paragraph around it.
        let boundaries = line_text
            .grapheme_indices(true)
            .map(|(index, _)| line.source.start + index)
            .chain(std::iter::once(line.source.end));
        for byte in boundaries {
            let at_wrap = wrapped && byte == line.source.end;
            let downstream = (!at_wrap)
                .then(|| self.place_caret(line, &cells, byte, Affinity::Downstream))
                .flatten();
            if let Some((x_px, _)) = downstream {
                stops.push(CaretStop {
                    x_px,
                    caret: CaretPosition::new(byte, Affinity::Downstream, line.index),
                });
            }
            if byte > line.source.start
                && let Some((x_px, _)) = self.place_caret(line, &cells, byte, Affinity::Upstream)
                && downstream.is_none_or(|(x, _)| (x - x_px).abs() > CELL_JOIN_TOLERANCE_PX)
            {
                stops.push(CaretStop {
                    x_px,
                    caret: CaretPosition::new(byte, Affinity::Upstream, line.index),
                });
            }
        }
        // Whitespace a wrap hung after this line belongs to no line's source,
        // but every position inside it is still somewhere a caret can be; it
        // draws at this line's end.
        if wrapped
            && let Some(next) = self
                .lines
                .iter()
                .position(|candidate| candidate.index == line.index)
                .and_then(|index| self.lines.get(index + 1))
            && let Some(gap) = text.get(line.source.end..next.source.start)
        {
            let inner = gap
                .grapheme_indices(true)
                .map(|(index, _)| line.source.end + index)
                .filter(|byte| *byte > line.source.end);
            for byte in inner {
                if let Some((x_px, _)) = self.place_caret(line, &cells, byte, Affinity::Upstream) {
                    stops.push(CaretStop {
                        x_px,
                        caret: CaretPosition::new(byte, Affinity::Upstream, line.index),
                    });
                }
            }
        }
        // Two positions can draw at one x — the logical end of an LTR line
        // that ends in RTL text and the RTL run's logical start, say. They
        // are still two positions an arrow key has to be able to reach, so
        // they stay adjacent, in the line's reading order.
        let rtl = line.base_direction.is_rtl();
        stops.sort_by(|a, b| {
            let bytes = if rtl {
                b.caret.byte.cmp(&a.caret.byte)
            } else {
                a.caret.byte.cmp(&b.caret.byte)
            };
            snapped_x(a.x_px).cmp(&snapped_x(b.x_px)).then(bytes)
        });
        stops
    }

    /// Rectangles covering a source byte range, one or more per line.
    ///
    /// A mixed-BiDi line yields several disjoint rects, which is why this
    /// returns a `Vec` rather than a single rect per line.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<TextRect> {
        if range.start >= range.end {
            return Vec::new();
        }
        let mut rects = Vec::new();
        for line in &self.lines {
            // A line's clusters lie inside its source range.
            if line.source.end <= range.start || line.source.start >= range.end {
                continue;
            }
            let mut open: Option<(f32, f32)> = None;
            for cell in self.visual_cells(line) {
                let selected = cell.cluster < range.end && cell.cluster_end > range.start;
                match (selected, open) {
                    (true, None) => open = Some((cell.left, cell.right)),
                    (true, Some((start, end))) => {
                        // Merge only when the cells actually touch; a BiDi jump
                        // must stay two rects.
                        if (cell.left - end).abs() <= CELL_JOIN_TOLERANCE_PX {
                            open = Some((start, cell.right));
                        } else {
                            rects.push(line_rect(line, start, end));
                            open = Some((cell.left, cell.right));
                        }
                    }
                    (false, Some((start, end))) => {
                        rects.push(line_rect(line, start, end));
                        open = None;
                    }
                    (false, None) => {}
                }
            }
            if let Some((start, end)) = open {
                rects.push(line_rect(line, start, end));
            }
        }
        rects
    }
}

/// An x compared at the resolution two cells count as touching.
fn snapped_x(x_px: f32) -> i64 {
    (x_px / CELL_JOIN_TOLERANCE_PX).round() as i64
}

/// One place a caret can be on a line: see [`TextLayout::caret_stops`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CaretStop {
    pub x_px: f32,
    pub caret: CaretPosition,
}

/// A caret dropped at one of a line's two edges.
///
/// Only the **logical** end of a soft-wrapped line is ambiguous — that byte is
/// also the start of the next line — and it is the visually *left* edge of an
/// RTL line. Deciding by which edge was clicked instead would hand the
/// "stay on the line above" affinity to the logical start, where there is
/// nothing above.
fn edge_affinity(line: &LineBox, byte: usize) -> Affinity {
    if byte == line.source.end && line.break_cause == crate::layout::LineBreakCause::Wrap {
        Affinity::Upstream
    } else {
        Affinity::Downstream
    }
}

fn line_rect(line: &LineBox, left: f32, right: f32) -> TextRect {
    TextRect::new(
        left,
        line.metrics.top_y_px,
        right - left,
        line.metrics.height_px,
    )
}
