//! What is under the pointer: the hovered item or axis position, the
//! tooltip it shows and where the axis pointer goes. Answered from a
//! finished [`ChartLayout`] without laying anything out again.

use std::sync::Arc;

use crate::layout::ChartLayout;
use crate::option::{AxisPointer, ChartOption, TimeFormatter, TooltipItem, TooltipTrigger};
use crate::scale;
use crate::smooth::Along;

/// A point of a series on the base axis, in px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisPoint {
    /// Along the base axis.
    pub base: f32,
    /// Across it, at the value.
    pub cross: f32,
    pub index: u32,
    /// The point's base-axis value (x on a vertical chart).
    pub key: f64,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisSeries {
    pub series: u32,
    pub name: Arc<str>,
    pub color: [f32; 4],
    /// Ascending (or descending, on an inverse axis) in `base`.
    pub points: Vec<AxisPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BaseAxisHit {
    Category {
        first: usize,
        count: usize,
        band: f32,
        gap: bool,
    },
    Continuous,
}

/// The cartesian axis trigger's data.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisHit {
    pub along: Along,
    pub plot: [f32; 4],
    pub base: BaseAxisHit,
    /// px of the base axis' min and max ends.
    pub base_start: f32,
    pub base_end: f32,
    /// Category names, by index.
    pub titles: Arc<[Arc<str>]>,
    pub time: bool,
    pub time_formatter: Option<TimeFormatter>,
    pub series: Vec<AxisSeries>,
    pub has_bars: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemRegion {
    Rect([f32; 4]),
    Circle {
        center: [f32; 2],
        radius: f32,
    },
    /// Angles in radians, clockwise from 3 o'clock (screen space),
    /// `start <= end`.
    Sector {
        center: [f32; 2],
        start: f32,
        end: f32,
        inner: f32,
        outer: f32,
    },
}

impl ItemRegion {
    pub fn contains(&self, p: [f32; 2]) -> bool {
        match *self {
            Self::Rect([x0, y0, x1, y1]) => p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1,
            Self::Circle { center, radius } => {
                (p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) <= radius * radius
            }
            Self::Sector {
                center,
                start,
                end,
                inner,
                outer,
            } => {
                let (dx, dy) = (p[0] - center[0], p[1] - center[1]);
                let r = (dx * dx + dy * dy).sqrt();
                if r < inner || r > outer {
                    return false;
                }
                let tau = std::f32::consts::TAU;
                let angle = dy.atan2(dx).rem_euclid(tau);
                let from = start.rem_euclid(tau);
                let span = end - start;
                if span >= tau {
                    return true;
                }
                (angle - from).rem_euclid(tau) <= span
            }
        }
    }
}

/// An item the item trigger can hover.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemHit {
    pub region: ItemRegion,
    pub series: u32,
    pub index: u32,
    /// The tooltip's title. `None`: the category, else the series name.
    pub title: Option<Arc<str>>,
    /// The row's name. `None`: the series name.
    pub row: Option<Arc<str>>,
    pub value: f64,
    pub color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HitModel {
    pub axis: Option<AxisHit>,
    /// In paint order; the last one that contains the pointer is on top.
    pub items: Vec<ItemHit>,
}

/// `series` of [`ChartHover`] that matches every series (axis trigger).
pub const ANY_SERIES: u32 = u32::MAX - 1;
/// Nothing hovered.
pub const NO_HOVER: u32 = u32::MAX;

/// One row of a tooltip.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipRow {
    pub color: Option<[f32; 4]>,
    pub name: Arc<str>,
    pub value: Arc<str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TooltipContent {
    pub title: Arc<str>,
    pub rows: Vec<TooltipRow>,
}

/// The axis pointer to draw, node-local px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerGeometry {
    Line { from: [f32; 2], to: [f32; 2] },
    Band([f32; 4]),
    Cross { center: [f32; 2], plot: [f32; 4] },
}

/// What the pointer is over.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartHover {
    /// The emphasised series ([`ANY_SERIES`] for an axis position).
    pub series: u32,
    /// The emphasised data index.
    pub index: u32,
    pub pointer: Option<PointerGeometry>,
    pub tooltip: Option<TooltipContent>,
}

/// How long hover emphasis takes to ease in or out, seconds.
pub const HOVER_DURATION: f32 = 0.15;
/// How far a hovered pie slice grows, px.
pub const EMPHASIS_GROWTH: f32 = 6.0;

/// Hover as a chart draws it: what is emphasised now and what was before,
/// since when on the animation clock, and the axis pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverState {
    /// `(series, data index)` hovered; [`NO_HOVER`] when nothing is.
    pub current: [u32; 2],
    pub previous: [u32; 2],
    /// The series focused (the others fade) now and before.
    pub focus: [u32; 2],
    pub since: std::time::Duration,
    pub pointer: Option<PointerGeometry>,
}

impl Default for HoverState {
    fn default() -> Self {
        Self {
            current: [NO_HOVER, NO_HOVER],
            previous: [NO_HOVER, NO_HOVER],
            focus: [NO_HOVER, NO_HOVER],
            since: std::time::Duration::ZERO,
            pointer: None,
        }
    }
}

impl HoverState {
    /// When the emphasis stops moving. Nothing moves until the hover has
    /// changed once.
    pub fn live_until(&self) -> std::time::Duration {
        if self.current == self.previous && self.focus[0] == self.focus[1] {
            return std::time::Duration::ZERO;
        }
        self.since + std::time::Duration::from_secs_f32(HOVER_DURATION)
    }
}

fn format_value(option: &ChartOption, value: f64) -> Arc<str> {
    match &option.tooltip.value_formatter {
        Some(formatter) => formatter.format(value),
        None if !value.is_finite() => Arc::from("-"),
        None if value.fract() == 0.0 => scale::format_number(value, 1.0).into(),
        None => {
            let magnitude = value.abs().log10().floor();
            let step = 10f64.powf((magnitude - 3.0).min(-1.0)).max(1e-6);
            let text = scale::format_number(value, step);
            // Trailing zeros of the decimals say nothing.
            let trimmed = if text.contains('.') {
                text.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                text
            };
            trimmed.into()
        }
    }
}

fn formatted(option: &ChartOption, title: Arc<str>, items: &[TooltipItem]) -> TooltipContent {
    if let Some(formatter) = &option.tooltip.formatter {
        let (title, rows) = (formatter.0)(items);
        return TooltipContent {
            title,
            rows: rows
                .into_iter()
                .map(|row| TooltipRow {
                    color: None,
                    name: row,
                    value: Arc::from(""),
                })
                .collect(),
        };
    }
    TooltipContent {
        title,
        rows: items
            .iter()
            .map(|item| TooltipRow {
                color: Some(item.color),
                name: item.series_name.clone(),
                value: format_value(option, item.value),
            })
            .collect(),
    }
}

impl ChartLayout {
    /// What `point` (node-local px) is over, or `None` when nothing is.
    pub fn hover_at(&self, option: &ChartOption, point: [f32; 2]) -> Option<ChartHover> {
        match self.trigger {
            TooltipTrigger::Axis => self.axis_hover(option, point),
            _ => self.item_hover(option, point),
        }
    }

    fn axis_hover(&self, option: &ChartOption, point: [f32; 2]) -> Option<ChartHover> {
        let axis = self.hit.axis.as_ref()?;
        let plot = axis.plot;
        if point[0] < plot[0] || point[0] > plot[2] || point[1] < plot[1] || point[1] > plot[3] {
            return None;
        }
        let coord = if axis.along == Along::X {
            point[0]
        } else {
            point[1]
        };
        let mut items = Vec::new();
        let (title, index, snapped, band) = match axis.base {
            BaseAxisHit::Category {
                first,
                count,
                band,
                gap,
            } => {
                let span = axis.base_end - axis.base_start;
                let unit = ((coord - axis.base_start) / span).clamp(0.0, 1.0);
                let local = if gap {
                    (unit * count as f32).floor().min(count as f32 - 1.0)
                } else {
                    (unit * (count as f32 - 1.0)).round()
                }
                .max(0.0);
                let category = first + local as usize;
                let title = axis
                    .titles
                    .get(category)
                    .cloned()
                    .unwrap_or_else(|| Arc::from(""));
                let unit_center = if gap {
                    (local + 0.5) / count as f32
                } else if count <= 1 {
                    0.5
                } else {
                    local / (count as f32 - 1.0)
                };
                for series in &axis.series {
                    if let Some(p) = series.points.iter().find(|p| p.index as usize == category) {
                        items.push(TooltipItem {
                            series_index: series.series as usize,
                            series_name: series.name.clone(),
                            data_index: p.index as usize,
                            name: title.clone(),
                            value: p.value,
                            color: series.color,
                        });
                    }
                }
                (
                    title,
                    category as u32,
                    axis.base_start + span * unit_center,
                    Some(band),
                )
            }
            BaseAxisHit::Continuous => {
                // The nearest point of any series sets the position; every
                // series with a point there reports it.
                let nearest = |series: &AxisSeries| -> Option<AxisPoint> {
                    let points = &series.points;
                    let ascending = points.first()?.base <= points.last()?.base;
                    let at = points.partition_point(|p| {
                        if ascending {
                            p.base < coord
                        } else {
                            p.base > coord
                        }
                    });
                    [at.checked_sub(1), Some(at)]
                        .into_iter()
                        .flatten()
                        .filter_map(|i| points.get(i))
                        .min_by(|a, b| (a.base - coord).abs().total_cmp(&(b.base - coord).abs()))
                        .copied()
                };
                let anchor = axis
                    .series
                    .iter()
                    .filter_map(nearest)
                    .min_by(|a, b| (a.base - coord).abs().total_cmp(&(b.base - coord).abs()))?;
                for series in &axis.series {
                    if let Some(p) = nearest(series)
                        && (p.base - anchor.base).abs() < 0.5
                    {
                        items.push(TooltipItem {
                            series_index: series.series as usize,
                            series_name: series.name.clone(),
                            data_index: p.index as usize,
                            name: Arc::from(""),
                            value: p.value,
                            color: series.color,
                        });
                    }
                }
                let title = if axis.time {
                    time_title(axis, anchor.key.round() as i64)
                } else {
                    format_value(option, anchor.key)
                };
                (title, anchor.index, anchor.base, None)
            }
        };
        if items.is_empty() {
            return None;
        }
        let pointer_kind = match option.tooltip.axis_pointer {
            AxisPointer::Auto if axis.has_bars && band.is_some() => AxisPointer::Shadow,
            AxisPointer::Auto => AxisPointer::Line,
            kind => kind,
        };
        let horizontal = axis.along == Along::X;
        let pointer = match (pointer_kind, band) {
            (AxisPointer::None, _) => None,
            (AxisPointer::Shadow, Some(band)) => {
                let half = band * 0.5;
                Some(PointerGeometry::Band(if horizontal {
                    [snapped - half, plot[1], snapped + half, plot[3]]
                } else {
                    [plot[0], snapped - half, plot[2], snapped + half]
                }))
            }
            (AxisPointer::Cross, _) => Some(PointerGeometry::Cross {
                center: if horizontal {
                    [snapped, point[1]]
                } else {
                    [point[0], snapped]
                },
                plot,
            }),
            _ => Some(if horizontal {
                PointerGeometry::Line {
                    from: [snapped, plot[1]],
                    to: [snapped, plot[3]],
                }
            } else {
                PointerGeometry::Line {
                    from: [plot[0], snapped],
                    to: [plot[2], snapped],
                }
            }),
        };
        Some(ChartHover {
            series: ANY_SERIES,
            index,
            pointer,
            tooltip: (option.tooltip.show != Some(false)).then(|| formatted(option, title, &items)),
        })
    }

    fn item_hover(&self, option: &ChartOption, point: [f32; 2]) -> Option<ChartHover> {
        let item = self
            .hit
            .items
            .iter()
            .rev()
            .find(|item| item.region.contains(point))?;
        let series_name = option
            .series
            .get(item.series as usize)
            .map(|series| series.name().clone())
            .unwrap_or_else(|| Arc::from(""));
        let category = self
            .hit
            .axis
            .as_ref()
            .and_then(|axis| axis.titles.get(item.index as usize).cloned());
        let title = item
            .title
            .clone()
            .or(category)
            .unwrap_or_else(|| series_name.clone());
        let row = item.row.clone().unwrap_or_else(|| series_name.clone());
        let entry = TooltipItem {
            series_index: item.series as usize,
            series_name: row,
            data_index: item.index as usize,
            name: title.clone(),
            value: item.value,
            color: item.color,
        };
        Some(ChartHover {
            series: item.series,
            index: item.index,
            pointer: None,
            tooltip: (option.tooltip.show != Some(false))
                .then(|| formatted(option, title, std::slice::from_ref(&entry))),
        })
    }

    /// The legend entry under `point`.
    pub fn legend_at(&self, point: [f32; 2]) -> Option<&Arc<str>> {
        self.legend
            .iter()
            .find(|item| ItemRegion::Rect(item.rect).contains(point))
            .map(|item| &item.name)
    }
}

/// A time axis value as a tooltip title.
fn time_title(axis: &AxisHit, value: i64) -> Arc<str> {
    let unit = crate::option::TimeUnit::Second;
    match &axis.time_formatter {
        Some(formatter) => (formatter.0)(value, unit),
        None => scale::format_time(
            scale::TimeTick {
                value,
                unit,
                boundary: false,
            },
            0,
        )
        .into(),
    }
}
