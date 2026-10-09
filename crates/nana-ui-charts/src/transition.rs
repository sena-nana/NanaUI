//! Entry and update motion: where each element of a new layout starts.
//!
//! A fresh layout carries every element's entry state in `from`. On the
//! first layout that is the motion (bars grow, slices sweep, lines are
//! revealed). On a data update each element instead starts where the
//! previous layout *shows* it at that instant (mid-motion included), and
//! an element that did not exist enters from its entry state. Lines are
//! matched by series and run and resampled along their axis, since their
//! point counts change with the data.

use std::collections::HashMap;
use std::time::Duration;

use crate::layout::{ChartLayout, DrawPart};
use crate::marks::{
    GpuPoint, GpuShape, MarkPass, MarkTransition, SHAPE_REVEAL, ShapeKind, draw_flags,
};
use crate::option::{ChartOption, Series};
use crate::smooth::Along;

/// The most data items any series has.
fn largest_series(option: &ChartOption) -> usize {
    option
        .series
        .iter()
        .map(|series| match series {
            Series::Line(line) => line.data.len(),
            Series::Bar(bar) => bar.data.len(),
            Series::Scatter(scatter) => scatter.data.len(),
            Series::Pie(pie) => pie.data.len(),
            Series::Radar(radar) => radar.data.len(),
            Series::Gauge(_) => 1,
        })
        .max()
        .unwrap_or(0)
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Puts every element at rest where it is going: no motion.
pub fn settle(layout: &mut ChartLayout) {
    let points: Vec<GpuPoint> = layout
        .marks
        .points
        .iter()
        .map(|p| GpuPoint {
            from: p.to,
            dist: [p.dist[0], p.dist[0], 0.0, 0.0],
            ..*p
        })
        .collect();
    let shapes: Vec<GpuShape> = layout
        .marks
        .shapes
        .iter()
        .map(|s| GpuShape { from: s.to, ..*s })
        .collect();
    layout.marks.points = points.into();
    layout.marks.shapes = shapes.into();
    layout.marks.transition = None;
    layout.marks.remeasure();
}

/// Sets `layout` in motion from `previous` (what was shown before) at
/// `now` on the animation clock.
pub fn begin(
    layout: &mut ChartLayout,
    previous: Option<&ChartLayout>,
    option: &ChartOption,
    now: Duration,
) {
    let animation = option.animation;
    if !animation.enabled || largest_series(option) > animation.threshold {
        settle(layout);
        return;
    }
    match previous {
        None => {
            // Entry: lines are revealed in place; everything else starts
            // from its entry state.
            let mut points: Vec<GpuPoint> = layout.marks.points.to_vec();
            for draw in layout.marks.draws.iter() {
                if draw.flags & draw_flags::REVEAL != 0
                    && matches!(draw.pass, MarkPass::Line | MarkPass::Area)
                {
                    for p in &mut points[draw.first as usize..(draw.first + draw.count) as usize] {
                        p.from = p.to;
                        p.dist[1] = p.dist[0];
                    }
                }
            }
            let shapes: Vec<GpuShape> = layout
                .marks
                .shapes
                .iter()
                .map(|s| {
                    if s.meta[0] & SHAPE_REVEAL != 0 {
                        GpuShape { from: s.to, ..*s }
                    } else {
                        *s
                    }
                })
                .collect();
            layout.marks.points = points.into();
            layout.marks.shapes = shapes.into();
            layout.marks.transition = Some(MarkTransition {
                start: now,
                duration: animation.duration,
                easing: animation.easing,
                reveal: true,
            });
            layout.marks.remeasure();
        }
        Some(previous) => {
            let progress = previous
                .marks
                .transition
                .map_or(1.0, |transition| transition.progress(now));
            let points = morph_points(layout, previous, progress);
            let shapes = morph_shapes(layout, previous, progress);
            layout.marks.points = points.into();
            layout.marks.shapes = shapes.into();
            layout.marks.transition = Some(MarkTransition {
                start: now,
                duration: animation.duration_update,
                easing: animation.easing_update,
                reveal: false,
            });
            layout.marks.remeasure();
        }
    }
}

/// What `previous` shows of each point range keyed by series and run:
/// `(curve, bases)` at `progress`.
/// A line as shown: its curve and its bases.
type Shown = (Vec<[f32; 2]>, Vec<[f32; 2]>);

fn shown_lines(previous: &ChartLayout, progress: f32) -> HashMap<(u32, u32), Shown> {
    let mut shown = HashMap::new();
    for (draw, key) in previous.marks.draws.iter().zip(&previous.draw_keys) {
        if !matches!(draw.pass, MarkPass::Line | MarkPass::Area) || key.part == DrawPart::Guide {
            continue;
        }
        shown.entry((key.series, key.run)).or_insert_with(|| {
            let range = draw.first as usize..(draw.first + draw.count) as usize;
            let (mut curve, mut bases) = (Vec::new(), Vec::new());
            for p in &previous.marks.points[range] {
                let v = lerp4(p.from, p.to, progress);
                curve.push([v[0], v[1]]);
                bases.push([v[2], v[3]]);
            }
            (curve, bases)
        });
    }
    shown
}

fn morph_points(layout: &ChartLayout, previous: &ChartLayout, progress: f32) -> Vec<GpuPoint> {
    let shown = shown_lines(previous, progress);
    let mut points: Vec<GpuPoint> = layout.marks.points.to_vec();
    let mut done = Vec::new();
    for ((draw, key), along) in layout
        .marks
        .draws
        .iter()
        .zip(&layout.draw_keys)
        .zip(&layout.draw_along)
    {
        if !matches!(draw.pass, MarkPass::Line | MarkPass::Area)
            || key.part == DrawPart::Guide
            || done.contains(&draw.first)
        {
            continue;
        }
        done.push(draw.first);
        let Some((curve, bases)) = shown.get(&(key.series, key.run)) else {
            continue;
        };
        if curve.len() < 2 {
            continue;
        }
        let closed = draw.flags & draw_flags::CLOSED != 0;
        let range = draw.first as usize..(draw.first + draw.count) as usize;
        let same_count = curve.len() == draw.count as usize;
        let mut distance = 0.0;
        let mut last: Option<[f32; 2]> = None;
        for (offset, p) in points[range].iter_mut().enumerate() {
            // A closed shape (radar) keeps its vertex order; an open line
            // is resampled along its axis.
            let (from, from_base) = if closed || same_count {
                if offset < curve.len() {
                    (curve[offset], bases[offset])
                } else {
                    (curve[curve.len() - 1], bases[bases.len() - 1])
                }
            } else {
                let coord = if *along == Along::X { p.to[0] } else { p.to[1] };
                (
                    crate::layout::sample_at(curve, *along, coord),
                    crate::layout::sample_at(bases, *along, coord),
                )
            };
            if let Some(last) = last {
                distance += ((from[0] - last[0]).powi(2) + (from[1] - last[1]).powi(2)).sqrt();
            }
            last = Some(from);
            p.from = [from[0], from[1], from_base[0], from_base[1]];
            p.dist[1] = distance;
        }
    }
    points
}

/// Shapes are matched by kind, series and data index.
fn morph_shapes(layout: &ChartLayout, previous: &ChartLayout, progress: f32) -> Vec<GpuShape> {
    // Several shapes can share a data index (a radar entry's vertices):
    // the n-th of each key matches the n-th.
    let mut shown: HashMap<(u32, u32, u32, u32), [f32; 4]> = HashMap::new();
    let mut seen: HashMap<(u32, u32, u32), u32> = HashMap::new();
    for shape in previous.marks.shapes.iter() {
        if shape.meta[2] == u32::MAX {
            continue;
        }
        let key = (shape.kind(), shape.meta[2], shape.meta[3]);
        let nth = seen.entry(key).or_default();
        shown.insert(
            (key.0, key.1, key.2, *nth),
            lerp4(shape.from, shape.to, progress),
        );
        *nth += 1;
    }
    seen.clear();
    let tau = std::f32::consts::TAU;
    layout
        .marks
        .shapes
        .iter()
        .map(|shape| {
            let mut shape = *shape;
            if shape.meta[2] == u32::MAX {
                shape.from = shape.to;
                return shape;
            }
            let key = (shape.kind(), shape.meta[2], shape.meta[3]);
            let nth = seen.entry(key).or_default();
            let found = shown.get(&(key.0, key.1, key.2, *nth));
            *nth += 1;
            if let Some(from) = found {
                shape.from = *from;
                if shape.kind() == ShapeKind::Sector as u32 {
                    // Turn the short way round.
                    let shift = ((shape.to[0] - from[0]) / tau).round() * tau;
                    shape.from[0] += shift;
                    shape.from[1] += shift;
                }
            } else if shape.kind() == ShapeKind::Sector as u32 {
                // A new slice opens where it ends up.
                shape.from = [shape.to[0], shape.to[0], shape.to[2], shape.to[3]];
            }
            shape
        })
        .collect()
}
