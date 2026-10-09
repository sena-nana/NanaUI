//! What a laid-out chart hands the GPU: plain arrays the chart shaders read
//! as storage buffers, plus the ordered draws over them.
//!
//! The layouts here **are** the WGSL contract (`scene_paint/shader/chart.wgsl`
//! in `nana-ui`); a field moves in both places or neither.
//!
//! Every geometric value is in the chart node's local logical px. Each
//! element carries where it is going (`to`) and where it comes from (`from`);
//! the vertex shader blends the two by the eased [`MarkTransition`] progress
//! on the motion clock, so an entry or update animation costs no CPU work
//! after the arrays are built.

use std::sync::Arc;
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use nana_ui_core::Easing;

/// A vertex of a polyline and of the area under it.
///
/// `xy` is the curve point; `zw` is its base: the point on the axis (or on
/// the stacked series below) an area fills down to, or a radar's centre.
/// `dist` is the arc length along the line from its first point, for
/// dashes: `[to, from, _, _]`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuPoint {
    pub to: [f32; 4],
    pub from: [f32; 4],
    pub dist: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuPoint>() == 48);

/// The geometry kind of a [`GpuShape`] (low byte of `meta[0]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ShapeKind {
    /// Axis-aligned rounded rect. `to`/`from`: `[x0, y0, x1, y1]`.
    /// `extra`: corner radii `[tl, tr, br, bl]`.
    Rect = 0,
    /// Annular sector. `to`/`from`: `[start, end, inner, outer]`, angles in
    /// radians clockwise from 3 o'clock in screen space (y down).
    /// `extra`: `[cx, cy, corner_radius, pad]`.
    Sector = 1,
    /// A point symbol. `to`/`from`: `[cx, cy, size, rotation]`.
    /// `extra`: `[border_width, symbol, _, _]` (symbol is [`SymbolShape`]).
    Symbol = 2,
    /// A rotated rounded rect (gauge ticks and pointer).
    /// `to`/`from`: `[cx, cy, angle, length]`; `extra`: `[width, offset,
    /// radius, _]`: the rect runs along `angle` from `offset` to `offset +
    /// length` px out of the centre.
    Needle = 3,
}

/// Point symbol outlines (`extra[1]` of a [`ShapeKind::Symbol`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SymbolShape {
    Circle = 0,
    Rect = 1,
    RoundRect = 2,
    Triangle = 3,
    Diamond = 4,
    Pin = 5,
}

/// [`GpuShape::meta`] flag: the element sits in the cartesian plot and is
/// cut to it.
pub const SHAPE_CLIP_TO_PLOT: u32 = 1 << 8;
/// [`GpuShape::meta`] flag: the hover emphasis grows this element.
pub const SHAPE_EMPHASIS_GROW: u32 = 1 << 9;
/// [`GpuShape::meta`] flag: the line reveal also reveals this element.
pub const SHAPE_REVEAL: u32 = 1 << 10;

/// One bar, slice, symbol or needle.
///
/// `meta`: `[kind | flags, style, series, data index]`. `style` indexes
/// [`ChartMarks::styles`]; `series` and `data index` are what hover
/// emphasis matches.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuShape {
    pub to: [f32; 4],
    pub from: [f32; 4],
    pub extra: [f32; 4],
    pub meta: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuShape>() == 64);

impl GpuShape {
    pub fn new(kind: ShapeKind, flags: u32, style: u32, series: u32, index: u32) -> Self {
        Self {
            to: [0.0; 4],
            from: [0.0; 4],
            extra: [0.0; 4],
            meta: [kind as u32 | flags, style, series, index],
        }
    }

    pub fn kind(&self) -> u32 {
        self.meta[0] & 0xff
    }
}

/// Paint shared by many elements. Colors are straight-alpha **sRGB**; the
/// painter converts them to its linear instance space when it uploads, so a
/// theme change rewrites this table and not the geometry.
///
/// `fill` paints areas, bars, slices and symbol bodies; `stroke` paints
/// lines and symbol rings; `fill_end` is where an area's vertical gradient
/// ends (at its base). `params`: `[line width, dash on, dash off, symbol
/// border width]`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuStyle {
    pub fill: [f32; 4],
    pub fill_end: [f32; 4],
    pub stroke: [f32; 4],
    pub params: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuStyle>() == 64);

/// Bits of [`MarkDraw::flags`].
pub mod draw_flags {
    /// Cut to the plot rectangle.
    pub const CLIP_TO_PLOT: u32 = 1;
    /// Revealed left to right by the entry animation (lines and areas).
    pub const REVEAL: u32 = 1 << 1;
    /// Joins the last point back to the first.
    pub const CLOSED: u32 = 1 << 2;
    /// Areas: the base edge is antialiased too (stacked areas share it).
    pub const SOFT_BASE: u32 = 1 << 3;
    /// Emphasis of the hovered series widens this line.
    pub const EMPHASIS: u32 = 1 << 4;
    /// Not part of any series: guides, axes. Never faded by focus.
    pub const GUIDE: u32 = 1 << 5;
}

/// How a [`MarkDraw`] is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkPass {
    /// Segments between consecutive points: `first..first + count` points.
    Line,
    /// The area between each point and its base.
    Area,
    /// `first..first + count` shapes.
    Shapes,
}

/// One draw, in paint order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkDraw {
    pub pass: MarkPass,
    pub first: u32,
    pub count: u32,
    /// Index into [`ChartMarks::styles`] (lines and areas; shapes carry
    /// their own).
    pub style: u32,
    /// The series this draw belongs to, `u32::MAX` for guides.
    pub series: u32,
    pub flags: u32,
    /// Lines: how many neighbouring segments on each side a fragment checks
    /// before it lets this segment draw, so a translucent line is painted
    /// once where its segments overlap. At least 1.
    pub neighbours: u32,
}

/// The motion every element of one [`ChartMarks`] shares, on the document's
/// animation clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkTransition {
    pub start: Duration,
    /// Seconds.
    pub duration: f32,
    pub easing: Easing,
    /// Entry: lines and areas with [`draw_flags::REVEAL`] are uncovered left
    /// to right by the same progress.
    pub reveal: bool,
}

impl MarkTransition {
    /// Eased progress at `now`, `0.0..=1.0`.
    pub fn progress(&self, now: Duration) -> f32 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        let elapsed = now.saturating_sub(self.start).as_secs_f32();
        self.easing
            .sample((elapsed / self.duration).clamp(0.0, 1.0))
    }

    /// When the motion has nothing left to draw.
    pub fn end(&self) -> Duration {
        self.start + Duration::from_secs_f32(self.duration.max(0.0))
    }
}

/// Everything the chart shaders draw for one chart, in node-local px.
#[derive(Debug, Clone)]
pub struct ChartMarks {
    /// Unique per build: the painter keeps the uploaded arrays while this
    /// stays the same.
    pub revision: u64,
    pub points: Arc<[GpuPoint]>,
    pub shapes: Arc<[GpuShape]>,
    pub styles: Arc<[GpuStyle]>,
    pub draws: Arc<[MarkDraw]>,
    pub transition: Option<MarkTransition>,
    /// The cartesian plot, `[x0, y0, x1, y1]`; what [`draw_flags::CLIP_TO_PLOT`]
    /// cuts to and the reveal sweeps across.
    pub plot: [f32; 4],
    /// The farthest any element reaches past its geometry (line half-width,
    /// emphasis growth), for culling.
    pub reach: f32,
    /// Bounds of everything drawn, `[x0, y0, x1, y1]`, over both ends of
    /// every motion: measured when the arrays were last written.
    pub extent: Option<[f32; 4]>,
}

impl PartialEq for ChartMarks {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
    }
}

impl ChartMarks {
    pub fn empty(revision: u64) -> Self {
        Self {
            revision,
            points: Arc::from([]),
            shapes: Arc::from([]),
            styles: Arc::from([]),
            draws: Arc::from([]),
            transition: None,
            plot: [0.0; 4],
            reach: 0.0,
            extent: None,
        }
    }

    /// Measures [`Self::extent`] again after the arrays changed.
    pub fn remeasure(&mut self) {
        self.extent = self.measure();
    }

    pub fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }

    /// Bounds of everything drawn, `[x0, y0, x1, y1]`, over both ends of
    /// every motion.
    fn measure(&self) -> Option<[f32; 4]> {
        let mut bounds: Option<[f32; 4]> = None;
        let mut add = |x: f32, y: f32| {
            if !x.is_finite() || !y.is_finite() {
                return;
            }
            let b = bounds.get_or_insert([x, y, x, y]);
            b[0] = b[0].min(x);
            b[1] = b[1].min(y);
            b[2] = b[2].max(x);
            b[3] = b[3].max(y);
        };
        for point in self.points.iter() {
            for v in [point.to, point.from] {
                add(v[0], v[1]);
                add(v[2], v[3]);
            }
        }
        for shape in self.shapes.iter() {
            for v in [shape.to, shape.from] {
                match shape.kind() {
                    k if k == ShapeKind::Rect as u32 => {
                        add(v[0], v[1]);
                        add(v[2], v[3]);
                    }
                    k if k == ShapeKind::Sector as u32 => {
                        let r = v[3].max(v[2]);
                        add(shape.extra[0] - r, shape.extra[1] - r);
                        add(shape.extra[0] + r, shape.extra[1] + r);
                    }
                    k if k == ShapeKind::Symbol as u32 => {
                        let r = v[2] * 0.75;
                        add(v[0] - r, v[1] - r);
                        add(v[0] + r, v[1] + r);
                    }
                    _ => {
                        let r = shape.extra[1] + v[3] + shape.extra[0];
                        add(v[0] - r, v[1] - r);
                        add(v[0] + r, v[1] + r);
                    }
                }
            }
        }
        bounds.map(|[x0, y0, x1, y1]| {
            [
                x0 - self.reach,
                y0 - self.reach,
                x1 + self.reach,
                y1 + self.reach,
            ]
        })
    }
}
