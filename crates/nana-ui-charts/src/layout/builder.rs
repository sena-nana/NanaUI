//! Collects marks into the arrays the shaders read.

use std::collections::HashMap;

use super::{DrawKey, DrawPart};
use crate::marks::{
    ChartMarks, GpuPoint, GpuShape, GpuStyle, MarkDraw, MarkPass, ShapeKind, draw_flags,
};
use crate::smooth::Along;

/// The most neighbouring segments a line fragment checks (see
/// [`MarkDraw::neighbours`]).
const MAX_NEIGHBOURS: u32 = 8;

#[derive(Default)]
pub(crate) struct MarkBuilder {
    points: Vec<GpuPoint>,
    shapes: Vec<GpuShape>,
    styles: Vec<GpuStyle>,
    style_index: HashMap<[u32; 16], u32>,
    draws: Vec<MarkDraw>,
    keys: Vec<DrawKey>,
    along: Vec<Along>,
    reach: f32,
    /// An open run of shapes: `(first, key, flags)`.
    shape_run: Option<(u32, DrawKey, u32)>,
}

/// A line's points with their bases and entry positions.
pub(crate) struct Polyline<'a> {
    pub points: &'a [[f32; 2]],
    /// The base each point fills down to. `None`: the points themselves.
    pub bases: Option<&'a [[f32; 2]]>,
    /// Entry positions of the points (and of the bases). `None`: in place.
    pub from: Option<&'a [[f32; 2]]>,
    pub from_bases: Option<&'a [[f32; 2]]>,
}

impl MarkBuilder {
    pub(crate) fn style(&mut self, style: GpuStyle) -> u32 {
        let key: [u32; 16] = bytemuck::cast(style);
        if let Some(index) = self.style_index.get(&key) {
            return *index;
        }
        let index = self.styles.len() as u32;
        self.styles.push(style);
        self.style_index.insert(key, index);
        index
    }

    pub(crate) fn note_reach(&mut self, reach: f32) {
        self.reach = self.reach.max(reach);
    }

    fn push_points(&mut self, line: &Polyline<'_>) -> Option<(u32, u32)> {
        let count = line.points.len();
        if count < 2 {
            return None;
        }
        let first = self.points.len() as u32;
        let mut distance = 0.0;
        let mut from_distance = 0.0;
        for index in 0..count {
            let to = line.points[index];
            let base = line.bases.map_or(to, |bases| bases[index]);
            let from = line.from.map_or(to, |from| from[index]);
            let from_base = line
                .from_bases
                .or(line.bases)
                .map_or(from, |bases| bases[index]);
            if index > 0 {
                distance += length(line.points[index - 1], to);
                let previous = line
                    .from
                    .map_or(line.points[index - 1], |from| from[index - 1]);
                from_distance += length(previous, from);
            }
            self.points.push(GpuPoint {
                to: [to[0], to[1], base[0], base[1]],
                from: [from[0], from[1], from_base[0], from_base[1]],
                dist: [distance, from_distance, 0.0, 0.0],
            });
        }
        Some((first, count as u32))
    }

    /// The fewest neighbours a fragment must check so that no two segments
    /// it does not compare can both reach it. Segments `k` apart are at
    /// least `k` advances apart along the axis the line runs on, so none
    /// farther than the stroke's reach over the smallest advance can touch
    /// the same pixel. A closed line has no such axis: its segment lengths
    /// stand in.
    fn neighbours(line: &Polyline<'_>, width: f32, along: Along, closed: bool) -> u32 {
        let advance = |a: [f32; 2], b: [f32; 2]| {
            if closed {
                length(a, b)
            } else if along == Along::X {
                (b[0] - a[0]).abs()
            } else {
                (b[1] - a[1]).abs()
            }
        };
        let smallest = line
            .points
            .windows(2)
            .map(|pair| advance(pair[0], pair[1]))
            .filter(|step| *step > 1e-3)
            .fold(f32::INFINITY, f32::min);
        if !smallest.is_finite() {
            return 1;
        }
        ((width + 2.0) / smallest)
            .ceil()
            .clamp(1.0, MAX_NEIGHBOURS as f32) as u32
    }

    /// A line, and the area under it when `area` is given. Both share the
    /// same points; the area is drawn first.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn polyline(
        &mut self,
        line: Polyline<'_>,
        stroke: Option<u32>,
        area: Option<u32>,
        width: f32,
        series: u32,
        run: u32,
        flags: u32,
        along: Along,
    ) {
        self.close_shapes();
        let Some((first, count)) = self.push_points(&line) else {
            return;
        };
        let guide = flags & draw_flags::GUIDE != 0;
        let part = |part| DrawKey {
            series,
            part: if guide { DrawPart::Guide } else { part },
            run,
        };
        if let Some(style) = area {
            self.draws.push(MarkDraw {
                pass: MarkPass::Area,
                first,
                count,
                style,
                series,
                flags: flags & !draw_flags::EMPHASIS,
                neighbours: 1,
            });
            self.keys.push(part(DrawPart::Area));
            self.along.push(along);
        }
        if let Some(style) = stroke {
            self.note_reach(width * 0.5 + 2.0);
            self.draws.push(MarkDraw {
                pass: MarkPass::Line,
                first,
                count,
                style,
                series,
                flags,
                neighbours: Self::neighbours(&line, width, along, flags & draw_flags::CLOSED != 0),
            });
            self.keys.push(part(DrawPart::Line));
            self.along.push(along);
        }
    }

    /// A shape. Consecutive shapes with the same key and flags share one
    /// draw.
    pub(crate) fn shape(&mut self, shape: GpuShape, key: DrawKey, flags: u32) {
        match self.shape_run {
            Some((_, open_key, open_flags)) if open_key == key && open_flags == flags => {}
            _ => {
                self.close_shapes();
                self.shape_run = Some((self.shapes.len() as u32, key, flags));
            }
        }
        if shape.kind() == ShapeKind::Symbol as u32 {
            self.note_reach(shape.extra[2].max(shape.to[2]) * 0.5 + shape.extra[0]);
        }
        self.shapes.push(shape);
    }

    pub(crate) fn close_shapes(&mut self) {
        let Some((first, key, flags)) = self.shape_run.take() else {
            return;
        };
        let count = self.shapes.len() as u32 - first;
        if count == 0 {
            return;
        }
        self.draws.push(MarkDraw {
            pass: MarkPass::Shapes,
            first,
            count,
            style: 0,
            series: key.series,
            flags,
            neighbours: 1,
        });
        self.keys.push(key);
        self.along.push(Along::X);
    }

    pub(crate) fn finish(mut self, plot: [f32; 4]) -> (ChartMarks, Vec<DrawKey>, Vec<Along>) {
        self.close_shapes();
        let mut marks = ChartMarks::empty(super::next_revision());
        marks.points = self.points.into();
        marks.shapes = self.shapes.into();
        marks.styles = self.styles.into();
        marks.draws = self.draws.into();
        marks.plot = plot;
        marks.reach = self.reach;
        marks.remeasure();
        (marks, self.keys, self.along)
    }
}

pub(crate) fn length(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

/// A style for lines and areas.
pub(crate) fn line_style(stroke: [f32; 4], width: f32, dash: [f32; 2]) -> GpuStyle {
    GpuStyle {
        fill: [0.0; 4],
        fill_end: [0.0; 4],
        stroke,
        params: [width, dash[0], dash[1], 0.0],
    }
}

pub(crate) fn fill_style(fill: [f32; 4], fill_end: [f32; 4]) -> GpuStyle {
    GpuStyle {
        fill,
        fill_end,
        stroke: [0.0; 4],
        params: [0.0; 4],
    }
}

pub(crate) fn symbol_style(fill: [f32; 4], stroke: [f32; 4], border: f32) -> GpuStyle {
    GpuStyle {
        fill,
        fill_end: fill,
        stroke,
        params: [0.0, 0.0, 0.0, border],
    }
}

pub(crate) fn with_alpha([r, g, b, a]: [f32; 4], alpha: f32) -> [f32; 4] {
    [r, g, b, a * alpha]
}
