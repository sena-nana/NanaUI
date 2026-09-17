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

impl TextLayout {
    fn cells(&self, line: &LineBox) -> Vec<Cell> {
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
        for line in &self.lines {
            if y_px < line.bounds.bottom() {
                return Some((line, true));
            }
        }
        self.lines.last().map(|line| (line, false))
    }

    fn line_by_index(&self, index: u32) -> Option<&LineBox> {
        self.lines.iter().find(|line| line.index == index)
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

    /// Resolves a point in layout space to a caret.
    ///
    /// A point outside the laid-out bounds still yields the nearest caret, with
    /// `inside == false`, because callers drag-select past the edges.
    pub fn hit_test(&self, x_px: f32, y_px: f32) -> HitTestResult {
        let Some((line, inside_y)) = self.line_at_y(y_px) else {
            return HitTestResult {
                caret: CaretPosition::default(),
                inside: false,
            };
        };
        let (left_byte, right_byte) = self.line_edge_bytes(line);
        let cells = self.cells(line);
        let inside_x = x_px >= line.bounds.x && x_px < line.bounds.right();

        let caret = if cells.is_empty() {
            CaretPosition::new(line.source.start, Affinity::Downstream, line.index)
        } else if x_px < cells[0].left {
            CaretPosition::new(left_byte, edge_affinity(line, left_byte), line.index)
        } else if x_px >= cells[cells.len() - 1].right {
            CaretPosition::new(right_byte, edge_affinity(line, right_byte), line.index)
        } else {
            let cell = cells
                .iter()
                .find(|cell| x_px >= cell.left && x_px < cell.right)
                .unwrap_or_else(|| {
                    // Cells are not always contiguous: alignment and trailing
                    // whitespace leave gaps between runs. A point in a gap
                    // belongs to the run beside it, not to the start of the
                    // line.
                    cells
                        .iter()
                        .min_by(|a, b| a.distance_to(x_px).total_cmp(&b.distance_to(x_px)))
                        .expect("the empty case returned above")
                });
            let midpoint = (cell.left + cell.right) * 0.5;
            let byte = if x_px < midpoint {
                cell.leading_byte()
            } else {
                cell.trailing_byte()
            };
            CaretPosition::new(byte, Affinity::Downstream, line.index)
        };

        HitTestResult {
            caret,
            inside: inside_y && inside_x,
        }
    }

    /// Where to draw the given caret, or `None` if it names a line this layout
    /// does not have.
    pub fn caret_geometry(&self, caret: CaretPosition) -> Option<CaretGeometry> {
        let line = self.line_by_index(caret.line)?;
        let (left_byte, right_byte) = self.line_edge_bytes(line);
        let cells = self.cells(line);

        let placed = cluster_cell_at(&cells, caret.byte)
            .map(|cell| (cell.x_for_byte(caret.byte), cell.direction))
            .or_else(|| {
                // Not inside any cluster: it is one of the line's two edges.
                //
                // Both edges come from the cells, because those are what
                // `hit_test` compares against when it decides a point is past
                // one end of the line. Reading `bounds.x` for the left edge
                // would disagree with it on any line whose first glyph does not
                // start at the line box edge -- a centred or indented line --
                // and the caret would jump away from the click that placed it.
                // An empty line has no cells, and then the line box is all
                // there is to go on.
                let empty = line.bounds.x;
                let left_edge = cells.first().map_or(empty, |cell| cell.left);
                let right_edge = cells.last().map_or(empty, |cell| cell.right);
                let logical_end = match self.gap_side(line, caret.byte) {
                    // Hung whitespace: the bytes between two lines are drawn by
                    // neither, and a caret in them sits at the end of the line
                    // it hangs from.
                    Some(GapSide::After) => true,
                    Some(GapSide::Before) => false,
                    None if caret.byte == left_byte => line.base_direction.is_rtl(),
                    None if caret.byte == right_byte => !line.base_direction.is_rtl(),
                    None => return None,
                };
                let edge = if logical_end == line.base_direction.is_rtl() {
                    left_edge
                } else {
                    right_edge
                };
                Some((edge, line.base_direction))
            })?;

        Some(CaretGeometry {
            x_px: placed.0,
            top_y_px: line.metrics.top_y_px,
            height_px: line.metrics.height_px,
            direction: placed.1,
        })
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
            let mut open: Option<(f32, f32)> = None;
            for cell in self.cells(line) {
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

/// The cell covering `byte`, merged across every cell rendering the same
/// cluster.
///
/// One cluster is not always one cell. A combining mark carries no advance and
/// shares its base's cluster, and in an RTL run HarfBuzz emits the mark
/// *before* its base, so taking whichever cell comes first would resolve the
/// caret against a zero-width cell and drop it a whole base glyph to the left.
/// Merging first means the interpolation in [`Cell::x_for_byte`] runs over the
/// extent the cluster actually draws in.
fn cluster_cell_at(cells: &[Cell], byte: usize) -> Option<Cell> {
    let first = cells
        .iter()
        .find(|cell| byte >= cell.cluster && byte < cell.cluster_end)?;
    let mut merged = Cell {
        left: first.left,
        right: first.right,
        cluster: first.cluster,
        cluster_end: first.cluster_end,
        direction: first.direction,
    };
    for cell in cells {
        if cell.cluster == merged.cluster && cell.cluster_end == merged.cluster_end {
            merged.left = merged.left.min(cell.left);
            merged.right = merged.right.max(cell.right);
        }
    }
    Some(merged)
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
