//! Laying a [`ChartOption`] out in a box: where the plot, axes, labels and
//! legend go, and every mark as GPU-ready arrays.
//!
//! A layout depends only on the option, the box size, the theme, label
//! measurement and the [`ChartViewState`] (legend selection, zoom). Hover
//! is not an input: it is answered from the finished layout by
//! [`ChartLayout::hover_at`] and drawn by the shaders from a uniform, so a
//! moving pointer never lays the chart out again.

mod builder;
mod cartesian;
mod legend;
mod polar;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) use builder::MarkBuilder;
pub(crate) use cartesian::sample_at;

use crate::hit::HitModel;
use crate::marks::ChartMarks;
use crate::option::{ChartOption, Series, TooltipTrigger};
use crate::smooth::Along;
use crate::theme::ChartTheme;

/// Measures a single line of label text with the host's text engine.
pub trait LabelMeasure {
    /// `[width, height]` in logical px of `text` at `size`.
    fn measure(&self, text: &str, size: f32) -> [f32; 2];
}

/// A fixed-advance measure for tests and hosts without a text engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproximateMeasure;

impl LabelMeasure for ApproximateMeasure {
    fn measure(&self, text: &str, size: f32) -> [f32; 2] {
        let width = text
            .chars()
            .map(|c| if c.is_ascii() { 0.56 } else { 1.0 })
            .sum::<f32>()
            * size;
        [width, size * 1.3]
    }
}

/// What the user has changed on a chart, which the layout follows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartViewState {
    /// Series names (pie: item names) the legend has switched off.
    pub hidden: HashSet<Arc<str>>,
    /// Per `option.data_zoom` entry, the window `(start, end)` in percent,
    /// overriding the option's initial one.
    pub zoom: Vec<Option<(f64, f64)>>,
}

impl ChartViewState {
    pub fn is_hidden(&self, name: &str) -> bool {
        self.hidden.contains(name)
    }

    /// The zoom window of `option.data_zoom[index]`.
    pub fn zoom_window(&self, option: &ChartOption, index: usize) -> Option<(f64, f64)> {
        let zoom = option.data_zoom.get(index)?;
        Some(
            self.zoom
                .get(index)
                .copied()
                .flatten()
                .unwrap_or((zoom.start, zoom.end)),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

/// One line of text the chart draws: an axis label, a legend entry, a data
/// label. `rect` is node-local and fits the measured run exactly, so the
/// text is set from its start.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartText {
    pub rect: [f32; 4],
    pub text: Arc<str>,
    pub color: [f32; 4],
    pub size: f32,
    pub weight: Option<u16>,
}

/// One legend entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendItem {
    /// The series (pie: item) name it toggles.
    pub name: Arc<str>,
    /// The whole clickable entry, symbol and text.
    pub rect: [f32; 4],
    pub selected: bool,
}

/// Where a part of the chart sits, for interaction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderLayout {
    /// The zoom entry this slider drives.
    pub zoom_index: usize,
    /// The whole track.
    pub track: [f32; 4],
    /// The selected window, between the handles.
    pub window: [f32; 4],
}

/// Identity of a mark draw across layouts, to morph one into the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawKey {
    pub series: u32,
    pub part: DrawPart,
    /// The contiguous run of a series with gaps.
    pub run: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawPart {
    Line,
    Area,
    Shapes,
    Guide,
}

/// A laid-out chart.
#[derive(Debug, Clone)]
pub struct ChartLayout {
    pub size: [f32; 2],
    /// The cartesian plot, `[x0, y0, x1, y1]`, when there is one.
    pub plot: Option<[f32; 4]>,
    pub texts: Vec<ChartText>,
    pub legend: Vec<LegendItem>,
    pub slider: Option<SliderLayout>,
    pub marks: ChartMarks,
    /// Parallel to `marks.draws`: what each draw is, to match it next time.
    pub draw_keys: Vec<DrawKey>,
    /// Parallel to `marks.draws`: the coordinate the draw advances along.
    pub draw_along: Vec<Along>,
    pub hit: HitModel,
    /// Whether the tooltip follows the axis (all series at one category)
    /// or the item under the pointer.
    pub trigger: TooltipTrigger,
    /// The axis pointer's line color and its band's fill.
    pub pointer_colors: [[f32; 4]; 2],
}

/// Two layouts are the same when they are the same build.
impl PartialEq for ChartLayout {
    fn eq(&self, other: &Self) -> bool {
        self.marks.revision == other.marks.revision
    }
}

static REVISION: AtomicU64 = AtomicU64::new(1);

/// A revision no other [`ChartMarks`] has.
pub fn next_revision() -> u64 {
    REVISION.fetch_add(1, Ordering::Relaxed)
}

/// Inputs of one layout.
pub struct LayoutInput<'a> {
    pub option: &'a ChartOption,
    pub size: [f32; 2],
    pub theme: &'a ChartTheme,
    pub measure: &'a dyn LabelMeasure,
    pub state: &'a ChartViewState,
}

/// Padding between the chart's edge and anything it draws.
pub(crate) const EDGE: f32 = nana_ui_core::space::LG;
/// Between a label and the axis or mark it names.
pub(crate) const LABEL_GAP: f32 = nana_ui_core::space::SM;

/// Lays the chart out. Every element's `from` is its entry state; see
/// [`crate::transition`] for how a layout is morphed from the previous one.
pub fn layout(input: &LayoutInput<'_>) -> ChartLayout {
    let [width, height] = input.size;
    let mut builder = MarkBuilder::default();
    let mut texts = Vec::new();
    let mut frame = [
        EDGE,
        EDGE,
        (width - EDGE).max(EDGE),
        (height - EDGE).max(EDGE),
    ];
    let legend = legend::layout(input, &mut frame, &mut builder, &mut texts);
    let mut hit = HitModel::default();
    let has_cartesian = input.option.series.iter().any(Series::is_cartesian);
    let mut plot = None;
    let mut slider = None;
    if has_cartesian {
        let cartesian = cartesian::layout(input, frame, &mut builder, &mut texts, &mut hit);
        plot = Some(cartesian.plot);
        slider = cartesian.slider;
    }
    polar::layout(input, frame, &mut builder, &mut texts, &mut hit);
    let trigger = match input.option.tooltip.trigger {
        _ if input.option.tooltip.show == Some(false) => TooltipTrigger::None,
        TooltipTrigger::Auto => {
            let axis_only = input
                .option
                .series
                .iter()
                .all(|series| matches!(series, Series::Line(_) | Series::Bar(_)));
            if axis_only && has_cartesian {
                TooltipTrigger::Axis
            } else {
                TooltipTrigger::Item
            }
        }
        trigger => trigger,
    };
    let (marks, draw_keys, draw_along) = builder.finish(plot.unwrap_or([0.0, 0.0, width, height]));
    ChartLayout {
        size: input.size,
        plot,
        texts,
        legend,
        slider,
        marks,
        draw_keys,
        draw_along,
        hit,
        trigger,
        pointer_colors: [
            input.theme.pointer,
            builder::with_alpha(input.theme.pointer, 0.18),
        ],
    }
}

/// Rounds a 1px line's centre onto a half pixel so it covers whole device
/// pixels at 1x and 2x.
pub(crate) fn crisp(value: f32) -> f32 {
    value.floor() + 0.5
}

pub(crate) fn text_rect(
    measure: &dyn LabelMeasure,
    text: &str,
    size: f32,
    anchor: [f32; 2],
    align: TextAlign,
    vertical: TextAlign,
) -> [f32; 4] {
    let [w, h] = measure.measure(text, size);
    let x = match align {
        TextAlign::Start => anchor[0],
        TextAlign::Center => anchor[0] - w * 0.5,
        TextAlign::End => anchor[0] - w,
    };
    let y = match vertical {
        TextAlign::Start => anchor[1],
        TextAlign::Center => anchor[1] - h * 0.5,
        TextAlign::End => anchor[1] - h,
    };
    // A sliver of slack so a measured width never wraps or ellipsizes.
    [x, y, x + w + 1.0, y + h]
}

pub(crate) fn rects_overlap(a: [f32; 4], b: [f32; 4], gap: f32) -> bool {
    a[0] < b[2] + gap && b[0] < a[2] + gap && a[1] < b[3] + gap && b[1] < a[3] + gap
}

#[cfg(test)]
mod tests;
