//! A chart's pointer: hover with its tooltip and axis pointer, legend
//! clicks, item clicks, and the zoom windows' wheel, pan, selection box
//! and sliders.
use super::*;
use crate::charts::{
    ChartDrag, ChartEvent, ChartHoverVisual, ChartSpec, ChartTooltip, ChromeLabelMeasure, ZoomGrab,
    ZoomMove,
};
use crate::{Chart, ChartVisual, NodePointer, NodePointerHooks, NodeWheel};
use nana_ui_charts::hit::{ChartHover, NO_HOVER, TooltipContent};
use nana_ui_charts::{
    ChartLayout, DataZoomKind, EmphasisFocus, Series, SliderLayout, ZoomAxis, ZoomAxisLayout,
};

pub(crate) const CHART_POINTER: NodePointerHooks<Chart> = NodePointerHooks {
    moved: chart_moved,
    left: chart_left,
    pressed: chart_pressed,
    released: chart_released,
    wheel: chart_wheel,
};

/// The smallest zoom window, percent.
const MIN_ZOOM_SPAN: f64 = 1.0;
/// A press that moves less than this is a click.
const CLICK_SLOP: f32 = nana_ui_core::space::XS;
/// How near a slider end a press grabs it, px.
const HANDLE_REACH: f32 = nana_ui_core::space::MD;

/// The axis a zoom entry windows.
type ZoomTarget = (ZoomAxis, usize);

fn shown(cx: &AppContext, id: StableNodeId) -> Option<(Arc<ChartSpec>, Arc<ChartLayout>)> {
    let spec = Arc::clone(&cx.world.widget_visual_as::<ChartVisual>(id)?.spec);
    let layout = spec.shown().or_else(|| cx.world.chart_layout(id))?;
    Some((spec, layout))
}

fn focus_of(spec: &ChartSpec, series: u32) -> u32 {
    let focus = match spec.option.series.get(series as usize) {
        Some(Series::Line(line)) => line.focus,
        Some(Series::Bar(bar)) => bar.focus,
        Some(Series::Scatter(scatter)) => scatter.focus,
        Some(Series::Candlestick(candle)) => candle.focus,
        Some(Series::Boxplot(boxplot)) => boxplot.focus,
        Some(Series::Heatmap(heatmap)) => heatmap.focus,
        _ => EmphasisFocus::None,
    };
    if focus == EmphasisFocus::Series {
        series
    } else {
        NO_HOVER
    }
}

/// The shown zoomable axes some inside-plot entry drives with `wants`.
fn inside_axes(
    spec: &ChartSpec,
    layout: &ChartLayout,
    wants: impl Fn(&nana_ui_charts::DataZoom) -> bool,
) -> Vec<ZoomAxisLayout> {
    layout
        .zoom_axes
        .iter()
        .filter(|axis| {
            spec.option.data_zoom.iter().any(|zoom| {
                zoom.kind == DataZoomKind::Inside
                    && zoom.target() == (axis.axis, axis.index)
                    && wants(zoom)
            })
        })
        .copied()
        .collect()
}

fn inside(rect: [f32; 4], x: f32, y: f32) -> bool {
    x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3]
}

fn chart_moved(cx: &mut AppContext, chart: Entity<Chart>, pointer: NodePointer) -> HandledResult {
    let id = chart.stable_id();
    if pointer.captured
        && let Some(drag) = cx.read(chart, |chart| chart.drag.clone())?
        && !matches!(drag, ChartDrag::Press { .. })
    {
        return chart_drag(cx, chart, drag, pointer);
    }
    let Some((spec, layout)) = shown(cx, id) else {
        return Ok(false);
    };
    let hover = layout.hover_at(&spec.option, [pointer.x, pointer.y]);
    set_hover(cx, chart, &spec, hover.as_ref())?;
    match hover.and_then(|hover| hover.tooltip) {
        Some(content) => show_tooltip(cx, id, content)?,
        None => hide_tooltip(cx, id)?,
    }
    Ok(true)
}

fn set_hover(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    spec: &ChartSpec,
    hover: Option<&ChartHover>,
) -> Result<(), FrameworkError> {
    let key = hover.map_or([NO_HOVER, NO_HOVER], |hover| [hover.series, hover.index]);
    let pointer = hover.and_then(|hover| hover.pointer);
    let now = cx.world.animation_now();
    let focus = if key[0] == NO_HOVER {
        NO_HOVER
    } else {
        focus_of(spec, key[0])
    };
    let current = cx.read(chart, |chart| chart.hover)?;
    if current.current == key && current.pointer == pointer {
        return Ok(());
    }
    let next = if current.current == key {
        ChartHoverVisual { pointer, ..current }
    } else {
        ChartHoverVisual {
            current: key,
            previous: current.current,
            focus: [focus, current.focus[0]],
            since: now,
            pointer,
        }
    };
    cx.update_component(chart, |chart, _| chart.hover = next)
}

fn chart_left(cx: &mut AppContext, chart: Entity<Chart>) -> Result<(), FrameworkError> {
    let id = chart.stable_id();
    if let Some((spec, _)) = shown(cx, id) {
        set_hover(cx, chart, &spec, None)?;
    }
    // A cancelled selection draws nothing more.
    if cx.read(chart, |chart| chart.selection.is_some())? {
        cx.update_component(chart, |chart, _| chart.selection = None)?;
    }
    hide_tooltip(cx, id)
}

/// A slider press: which part it grabs and the axis it moves.
fn slider_press(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    spec: &ChartSpec,
    slider: SliderLayout,
    vertical: bool,
    point: [f32; 2],
) -> Result<ChartDrag, FrameworkError> {
    let target = spec.option.data_zoom[slider.zoom_index].target();
    let mut window = zoom_window(cx, chart, target)?;
    // Along the track: x, or y measured upward from the bottom.
    let (at, lo, hi, track) = if vertical {
        (
            slider.track[3] - point[1],
            slider.track[3] - slider.window[3],
            slider.track[3] - slider.window[1],
            slider.track[3] - slider.track[1],
        )
    } else {
        (
            point[0] - slider.track[0],
            slider.window[0] - slider.track[0],
            slider.window[2] - slider.track[0],
            slider.track[2] - slider.track[0],
        )
    };
    let track = track.max(1.0);
    let grab = if (at - lo).abs() <= HANDLE_REACH {
        ZoomGrab::Start
    } else if (at - hi).abs() <= HANDLE_REACH {
        ZoomGrab::End
    } else {
        ZoomGrab::Window
    };
    // The track outside the window jumps the window there.
    if grab == ZoomGrab::Window && !(lo..=hi).contains(&at) {
        let span = window.1 - window.0;
        let center = (at / track) as f64 * 100.0;
        let start = (center - span * 0.5).clamp(0.0, 100.0 - span);
        window = (start, start + span);
        set_zoom(cx, chart, target, window)?;
    }
    let per_px = 100.0 / track as f64;
    Ok(ChartDrag::Zoom {
        grab,
        from: point,
        axes: vec![ZoomMove {
            target,
            window,
            per_px: if vertical { -per_px } else { per_px },
            vertical,
        }],
    })
}

fn chart_pressed(cx: &mut AppContext, chart: Entity<Chart>, pointer: NodePointer) -> HandledResult {
    if pointer.button != 0 {
        return Ok(false);
    }
    let id = chart.stable_id();
    let Some((spec, layout)) = shown(cx, id) else {
        return Ok(false);
    };
    let (x, y) = (pointer.x, pointer.y);
    // The legend toggles its series.
    if spec
        .option
        .legend
        .as_ref()
        .is_some_and(|legend| legend.selectable)
        && let Some(name) = layout.legend_at([x, y]).cloned()
    {
        cx.update_component(chart, |chart, view| {
            let hidden = &mut chart.view.hidden;
            let selected = if hidden.remove(&name) {
                true
            } else {
                hidden.insert(name.clone());
                false
            };
            view.emit(ChartEvent::LegendSelect {
                name: name.clone(),
                selected,
            });
        })?;
        return Ok(true);
    }
    // A slider: its ends resize the window, the window moves, the track
    // jumps there.
    for (slider, vertical) in [(layout.slider, false), (layout.y_slider, true)] {
        if let Some(slider) = slider
            && inside(slider.track, x, y)
        {
            let drag = slider_press(cx, chart, &spec, slider, vertical, [x, y])?;
            cx.update_component(chart, |chart, _| chart.drag = Some(drag))?;
            return Ok(true);
        }
    }
    let hover = cx.read(chart, |chart| chart.hover.current)?;
    if let Some(plot) = layout.plot
        && inside(plot, x, y)
    {
        let selecting = inside_axes(&spec, &layout, |zoom| zoom.select_on_drag);
        if !selecting.is_empty() {
            // A double click goes back to the option's windows.
            if pointer.click_count >= 2 {
                restore_zoom(cx, chart)?;
                return Ok(true);
            }
            cx.update_component(chart, |chart, _| {
                chart.drag = Some(ChartDrag::Select {
                    from: [x, y],
                    hover,
                });
            })?;
            return Ok(true);
        }
        // Dragging inside the plot pans every axis it can the other way.
        let panning = inside_axes(&spec, &layout, |zoom| zoom.move_on_drag);
        if !panning.is_empty() {
            let mut axes = Vec::with_capacity(panning.len());
            for axis in panning {
                let target = (axis.axis, axis.index);
                axes.push(ZoomMove {
                    target,
                    window: zoom_window(cx, chart, target)?,
                    per_px: -axis.per_px(),
                    vertical: axis.vertical,
                });
            }
            cx.update_component(chart, |chart, _| {
                chart.drag = Some(ChartDrag::Zoom {
                    grab: ZoomGrab::Window,
                    from: [x, y],
                    axes,
                });
            })?;
            return Ok(true);
        }
    }
    if hover[0] == NO_HOVER {
        return Ok(false);
    }
    cx.update_component(chart, |chart, _| {
        chart.drag = Some(ChartDrag::Press { x, y, hover });
    })?;
    Ok(true)
}

/// The selection box from `from` to `to`: across the whole plot on an
/// axis nothing zooms.
fn selection_box(
    spec: &ChartSpec,
    layout: &ChartLayout,
    from: [f32; 2],
    to: [f32; 2],
) -> Option<[f32; 4]> {
    let plot = layout.plot?;
    let axes = inside_axes(spec, layout, |zoom| zoom.select_on_drag);
    let across = axes.iter().any(|axis| !axis.vertical);
    let down = axes.iter().any(|axis| axis.vertical);
    let clamp = |v: f32, lo: f32, hi: f32| v.clamp(lo, hi);
    let (x0, x1) = if across {
        let (a, b) = (
            clamp(from[0], plot[0], plot[2]),
            clamp(to[0], plot[0], plot[2]),
        );
        (a.min(b), a.max(b))
    } else {
        (plot[0], plot[2])
    };
    let (y0, y1) = if down {
        let (a, b) = (
            clamp(from[1], plot[1], plot[3]),
            clamp(to[1], plot[1], plot[3]),
        );
        (a.min(b), a.max(b))
    } else {
        (plot[1], plot[3])
    };
    Some([x0, y0, x1, y1])
}

fn chart_drag(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    drag: ChartDrag,
    pointer: NodePointer,
) -> HandledResult {
    match drag {
        ChartDrag::Zoom { grab, from, axes } => {
            for axis in axes {
                let travel = if axis.vertical {
                    pointer.y - from[1]
                } else {
                    pointer.x - from[0]
                };
                let delta = travel as f64 * axis.per_px;
                let (start, end) = axis.window;
                let next = match grab {
                    ZoomGrab::Window => {
                        let span = end - start;
                        let start = (start + delta).clamp(0.0, 100.0 - span);
                        (start, start + span)
                    }
                    ZoomGrab::Start => ((start + delta).clamp(0.0, end - MIN_ZOOM_SPAN), end),
                    ZoomGrab::End => (start, (end + delta).clamp(start + MIN_ZOOM_SPAN, 100.0)),
                };
                set_zoom(cx, chart, axis.target, next)?;
            }
        }
        ChartDrag::Select { from, .. } => {
            let id = chart.stable_id();
            let Some((spec, layout)) = shown(cx, id) else {
                return Ok(true);
            };
            let selection = selection_box(&spec, &layout, from, [pointer.x, pointer.y]);
            if cx.read(chart, |chart| chart.selection)? != selection {
                cx.update_component(chart, |chart, _| chart.selection = selection)?;
            }
        }
        ChartDrag::Press { .. } => {}
    }
    hide_tooltip(cx, chart.stable_id())?;
    Ok(true)
}

/// Zooms every axis the box spans to it. A box thinner than a click on
/// every zoomed side does nothing.
fn apply_selection(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    from: [f32; 2],
    to: [f32; 2],
) -> Result<bool, FrameworkError> {
    let Some((spec, layout)) = shown(cx, chart.stable_id()) else {
        return Ok(false);
    };
    let Some(rect) = selection_box(&spec, &layout, from, to) else {
        return Ok(false);
    };
    let axes = inside_axes(&spec, &layout, |zoom| zoom.select_on_drag);
    let spans = |axis: &ZoomAxisLayout| {
        if axis.vertical {
            (rect[1], rect[3])
        } else {
            (rect[0], rect[2])
        }
    };
    if axes.iter().all(|axis| {
        let (a, b) = spans(axis);
        b - a < CLICK_SLOP * 2.0
    }) {
        return Ok(false);
    }
    for axis in axes {
        let (a, b) = spans(&axis);
        let (pa, pb) = (axis.percent_at(a), axis.percent_at(b));
        let mut start = pa.min(pb).clamp(0.0, 100.0);
        let mut end = pa.max(pb).clamp(0.0, 100.0);
        if end - start < MIN_ZOOM_SPAN {
            let center =
                ((start + end) * 0.5).clamp(MIN_ZOOM_SPAN * 0.5, 100.0 - MIN_ZOOM_SPAN * 0.5);
            (start, end) = (center - MIN_ZOOM_SPAN * 0.5, center + MIN_ZOOM_SPAN * 0.5);
        }
        set_zoom(cx, chart, (axis.axis, axis.index), (start, end))?;
    }
    Ok(true)
}

/// Every zoom window back to the option's, saying so for each that moved.
fn restore_zoom(cx: &mut AppContext, chart: Entity<Chart>) -> Result<(), FrameworkError> {
    let changed: Vec<(usize, (f64, f64))> = cx.read(chart, |chart| {
        chart
            .option
            .data_zoom
            .iter()
            .enumerate()
            .filter_map(|(index, zoom)| {
                let initial = (zoom.start, zoom.end);
                (chart.view.zoom_window(&chart.option, index) != Some(initial))
                    .then_some((index, initial))
            })
            .collect()
    })?;
    if changed.is_empty() {
        return Ok(());
    }
    cx.update_component(chart, |chart, view| {
        chart.view.zoom.clear();
        for (index, (start, end)) in changed {
            view.emit(ChartEvent::DataZoom { index, start, end });
        }
    })
}

/// The window of the axis `target` now, in percent: its first entry's.
fn zoom_window(
    cx: &AppContext,
    chart: Entity<Chart>,
    target: ZoomTarget,
) -> Result<(f64, f64), FrameworkError> {
    cx.read(chart, |chart| {
        chart
            .option
            .data_zoom
            .iter()
            .position(|zoom| zoom.target() == target)
            .and_then(|index| chart.view.zoom_window(&chart.option, index))
            .unwrap_or((0.0, 100.0))
    })
}

/// Moves the window of the axis `target`: every entry on it, each said.
fn set_zoom(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    target: ZoomTarget,
    window: (f64, f64),
) -> Result<(), FrameworkError> {
    let entries: Vec<usize> = cx.read(chart, |chart| {
        chart
            .option
            .data_zoom
            .iter()
            .enumerate()
            .filter(|(index, zoom)| {
                zoom.target() == target
                    && chart.view.zoom_window(&chart.option, *index) != Some(window)
            })
            .map(|(index, _)| index)
            .collect()
    })?;
    if entries.is_empty() {
        return Ok(());
    }
    cx.update_component(chart, |chart, view| {
        let slots = &mut chart.view.zoom;
        for index in entries {
            if slots.len() <= index {
                slots.resize(index + 1, None);
            }
            slots[index] = Some(window);
            view.emit(ChartEvent::DataZoom {
                index,
                start: window.0,
                end: window.1,
            });
        }
    })
}

fn chart_released(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    pointer: NodePointer,
) -> HandledResult {
    let drag = cx.update_component(chart, |chart, _| {
        chart.selection = None;
        chart.drag.take()
    })?;
    let click = match drag {
        Some(ChartDrag::Press { x, y, hover }) => Some(([x, y], hover)),
        Some(ChartDrag::Select { from, hover }) => {
            if apply_selection(cx, chart, from, [pointer.x, pointer.y])? {
                None
            } else {
                Some((from, hover))
            }
        }
        _ => None,
    };
    if let Some((from, hover)) = click
        && (pointer.x - from[0]).abs() < CLICK_SLOP
        && (pointer.y - from[1]).abs() < CLICK_SLOP
        && hover[0] != NO_HOVER
    {
        cx.update_component(chart, |_, view| {
            view.emit(ChartEvent::Click {
                series: if hover[0] == nana_ui_charts::hit::ANY_SERIES {
                    0
                } else {
                    hover[0] as usize
                },
                index: hover[1] as usize,
            });
        })?;
    }
    Ok(drag.is_some())
}

fn chart_wheel(cx: &mut AppContext, chart: Entity<Chart>, wheel: NodeWheel) -> HandledResult {
    let id = chart.stable_id();
    let Some((spec, layout)) = shown(cx, id) else {
        return Ok(false);
    };
    let Some(plot) = layout.plot else {
        return Ok(false);
    };
    let axes = inside_axes(&spec, &layout, |zoom| zoom.zoom_on_wheel);
    if axes.is_empty() || !inside(plot, wheel.x, wheel.y) {
        return Ok(false);
    }
    // Turning the wheel away zooms in, around the pointer.
    let scale = (wheel.delta_y as f64 * 0.002).exp();
    for axis in axes {
        let target = (axis.axis, axis.index);
        let window = zoom_window(cx, chart, target)?;
        let span = window.1 - window.0;
        let next_span = (span * scale).clamp(MIN_ZOOM_SPAN, 100.0);
        let focus = axis
            .percent_at(axis.along([wheel.x, wheel.y]))
            .clamp(window.0, window.1);
        let at = if span > 0.0 {
            (focus - window.0) / span
        } else {
            0.5
        };
        let start = (focus - at * next_span).clamp(0.0, 100.0 - next_span);
        set_zoom(cx, chart, target, (start, start + next_span))?;
    }
    Ok(true)
}

impl AppContext {
    /// Keeps every open chart tooltip on its pointer after a layout pass.
    pub(super) fn position_chart_tooltips(
        &mut self,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        let targets: Vec<_> = self
            .component_lifecycle
            .chart_tooltips
            .iter()
            .filter(|(target, _)| {
                self.world
                    .node(**target)
                    .is_some_and(|node| node.document == document)
            })
            .map(|(target, tooltip)| (*target, *tooltip))
            .collect();
        for (target, tooltip) in targets {
            let open = self
                .world
                .node_style(tooltip)
                .is_some_and(|style| !style.layout.hidden);
            if open && self.world.is_mounted(target) {
                place_tooltip(self, target, tooltip)?;
            }
        }
        Ok(())
    }
}

fn show_tooltip(
    cx: &mut AppContext,
    target: StableNodeId,
    content: TooltipContent,
) -> Result<(), FrameworkError> {
    let document = cx
        .world
        .node(target)
        .ok_or(FrameworkError::MissingView(target))?
        .document;
    let content = Arc::new(content);
    let tooltip = match cx.component_lifecycle.chart_tooltips.get(&target).copied() {
        Some(id) => {
            let tooltip = Entity::<ChartTooltip>::from_stable_id(id);
            let same = cx.read(tooltip, |tooltip| *tooltip.content == *content)?;
            if !same {
                cx.update_component(tooltip, |tooltip, _| tooltip.content = Arc::clone(&content))?;
            }
            tooltip
        }
        None => {
            let tooltip =
                cx.create_detached_component(document, ChartTooltip::new(Arc::clone(&content)))?;
            cx.attach_child(target, tooltip.stable_id())?;
            cx.component_lifecycle
                .chart_tooltips
                .insert(target, tooltip.stable_id());
            tooltip
        }
    };
    place_tooltip(cx, target, tooltip.stable_id())?;
    let state = crate::OverlayHostState {
        active: Some(tooltip.stable_id()),
        restore_focus: None,
    };
    if cx.world.overlay_host(target) != Some(state) {
        let mut mutations = MutationQueue::new();
        mutations.set_overlay_host(target, state);
        cx.commit_mutations(mutations)?;
    }
    Ok(())
}

/// Puts the tooltip beside the pointer, flipped to stay in the viewport.
fn place_tooltip(
    cx: &mut AppContext,
    target: StableNodeId,
    tooltip: StableNodeId,
) -> Result<(), FrameworkError> {
    let Some(document) = cx.world.document_of(target) else {
        return Ok(());
    };
    let Some(viewport) = cx.world.document_viewport(document) else {
        return Ok(());
    };
    let Some((x, y)) = cx.pointer_location_on(target).or_else(|| {
        cx.component_lifecycle.pointer_positions.iter().find_map(
            |(&(owner, pointer), &position)| {
                (owner == document && cx.world.pointer_capture(owner, pointer) == Some(target))
                    .then_some(position)
            },
        )
    }) else {
        return Ok(());
    };
    let entity = Entity::<ChartTooltip>::from_stable_id(tooltip);
    let content = cx.read(entity, |tooltip| Arc::clone(&tooltip.content))?;
    let size = ChartTooltip::measure(
        &content,
        &ChromeLabelMeasure(cx.world.chrome_text_measure(tooltip)),
    );
    let gap = nana_ui_core::space::LG;
    let padding = nana_ui_core::space::SM;
    let right = x + gap;
    let left = x - gap - size[0];
    let px = if right + size[0] <= viewport.width - padding || left < padding {
        right
    } else {
        left
    };
    let above = y - gap - size[1];
    let below = y + gap;
    let py = if above >= padding || below + size[1] > viewport.height - padding {
        above
    } else {
        below
    };
    let max_x = (viewport.width - padding - size[0]).max(padding);
    let max_y = (viewport.height - padding - size[1]).max(padding);
    let (px, py) = (px.clamp(padding, max_x), py.clamp(padding, max_y));
    cx.update_component(entity, |tooltip, _| {
        let layout = Arc::make_mut(&mut tooltip.style.layout);
        layout.offset_left = Some(LengthSpec::Px(px));
        layout.offset_top = Some(LengthSpec::Px(py));
        layout.width = Some(LengthSpec::Px(size[0]));
        layout.height = Some(LengthSpec::Px(size[1]));
        layout.hidden = false;
    })
}

fn hide_tooltip(cx: &mut AppContext, target: StableNodeId) -> Result<(), FrameworkError> {
    let Some(id) = cx.component_lifecycle.chart_tooltips.get(&target).copied() else {
        return Ok(());
    };
    let entity = Entity::<ChartTooltip>::from_stable_id(id);
    if cx
        .read(entity, |tooltip| !tooltip.style.layout.hidden)
        .unwrap_or(false)
    {
        cx.update_component(entity, |tooltip, _| {
            Arc::make_mut(&mut tooltip.style.layout).hidden = true
        })?;
    }
    if cx
        .world
        .overlay_host(target)
        .is_some_and(|state| state.active.is_some())
    {
        let mut mutations = MutationQueue::new();
        mutations.set_overlay_host(target, crate::OverlayHostState::default());
        cx.commit_mutations(mutations)?;
    }
    Ok(())
}

type HandledResult = Result<bool, FrameworkError>;

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{HeadlessInput, LayoutViewport};
    use nana_ui_charts::{
        Animation, Axis, ChartOption, DataZoom, Legend, LineSeries, PieItem, PieLabelPosition,
        PieSeries,
    };
    use nana_ui_input::{InputModifiers, InputPayload, PointerPhase, WheelInput, WheelUnit};

    fn weekly() -> ChartOption {
        ChartOption::new()
            .animation(Animation::disabled())
            .legend(Legend::default())
            .x_axis(Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri"]))
            .y_axis(Axis::value())
            .series(LineSeries::new("visits", [3.0, 5.0, 4.0, 8.0, 6.0]))
    }

    fn chart_in(
        option: ChartOption,
    ) -> (
        AppContext,
        DocumentId,
        Entity<Chart>,
        Arc<Mutex<Vec<ChartEvent>>>,
    ) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut style = crate::NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(400.0));
        layout.height = Some(LengthSpec::Px(240.0));
        let chart = context
            .create_component(document, Chart::new(option).style(style))
            .unwrap();
        context
            .layout_document(document, LayoutViewport::new(600.0, 400.0))
            .unwrap();
        context.rebuild_hit_test(document);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        context
            .on(chart, move |_, event: &ChartEvent, _| {
                sink.lock().unwrap().push(event.clone())
            })
            .unwrap();
        (context, document, chart, events)
    }

    fn layout_of(context: &AppContext, chart: Entity<Chart>) -> Arc<ChartLayout> {
        context
            .world()
            .chart_layout(chart.stable_id())
            .expect("chart geometry")
    }

    #[test]
    fn hover_emphasises_the_category_and_opens_a_tooltip_until_the_pointer_leaves() {
        let (mut context, document, chart, _) = chart_in(weekly());
        let layout = layout_of(&context, chart);
        let plot = layout.plot.unwrap();
        // Over Thursday.
        let x = plot[0] + (plot[2] - plot[0]) * 0.75;
        let y = (plot[1] + plot[3]) * 0.5;
        let mut input = HeadlessInput::bind(&mut context, document);
        input
            .pointer(&mut context, PointerPhase::Move, x, y)
            .unwrap();
        let hover = context.read(chart, |chart| chart.hover).unwrap();
        assert_eq!(hover.current, [nana_ui_charts::hit::ANY_SERIES, 3]);
        assert!(matches!(
            hover.pointer,
            Some(nana_ui_charts::PointerGeometry::Line { .. })
        ));
        let id = chart.stable_id();
        let tooltip =
            Entity::<ChartTooltip>::from_stable_id(context.component_lifecycle.chart_tooltips[&id]);
        let content = context
            .read(tooltip, |tooltip| Arc::clone(&tooltip.content))
            .unwrap();
        assert_eq!(&*content.title, "Thu");
        assert_eq!(&*content.rows[0].value, "8");
        assert!(
            !context
                .read(tooltip, |tooltip| tooltip.style.layout.hidden)
                .unwrap()
        );
        let (tip_x, tip_y, tip_h) = context
            .read(tooltip, |tooltip| {
                (
                    tooltip.style.layout.offset_left,
                    tooltip.style.layout.offset_top,
                    tooltip.style.layout.height,
                )
            })
            .unwrap();
        assert!(
            matches!(tip_x, Some(LengthSpec::Px(px)) if px > x),
            "chart tooltip should prefer the pointer's right side ({x}), got {tip_x:?}"
        );
        assert!(
            matches!((tip_y, tip_h), (Some(LengthSpec::Px(py)), Some(LengthSpec::Px(h))) if py + h < y),
            "chart tooltip should prefer above the pointer ({y}), got {tip_y:?} + {tip_h:?}"
        );
        // The same category again changes nothing.
        let before = context.read(chart, |chart| chart.hover).unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, x + 1.0, y)
            .unwrap();
        assert_eq!(context.read(chart, |chart| chart.hover).unwrap(), before);
        // Leaving clears the emphasis and hides the tooltip.
        input
            .pointer(&mut context, PointerPhase::Move, 590.0, 390.0)
            .unwrap();
        let hover = context.read(chart, |chart| chart.hover).unwrap();
        assert_eq!(hover.current, [NO_HOVER, NO_HOVER]);
        assert_eq!(hover.previous, [nana_ui_charts::hit::ANY_SERIES, 3]);
        assert!(
            context
                .read(tooltip, |tooltip| tooltip.style.layout.hidden)
                .unwrap()
        );
        context.remove_view(chart).unwrap();
        assert!(context.component_lifecycle.chart_tooltips.is_empty());
        assert!(!context.world().contains(tooltip.stable_id()));
    }

    #[test]
    fn hovering_keeps_the_layout() {
        let (mut context, document, chart, _) = chart_in(weekly());
        let before = layout_of(&context, chart);
        let plot = before.plot.unwrap();
        let mut input = HeadlessInput::bind(&mut context, document);
        input
            .pointer(
                &mut context,
                PointerPhase::Move,
                plot[0] + 4.0,
                plot[1] + 4.0,
            )
            .unwrap();
        let after = layout_of(&context, chart);
        assert!(
            Arc::ptr_eq(&before, &after),
            "hover laid the chart out again"
        );
    }

    #[test]
    fn a_legend_click_switches_the_series_and_says_so() {
        let (mut context, document, chart, events) = chart_in(weekly());
        let layout = layout_of(&context, chart);
        let item = layout.legend[0].rect;
        let (x, y) = ((item[0] + item[2]) * 0.5, (item[1] + item[3]) * 0.5);
        let mut input = HeadlessInput::bind(&mut context, document);
        input
            .pointer(&mut context, PointerPhase::Down, x, y)
            .unwrap();
        input.pointer(&mut context, PointerPhase::Up, x, y).unwrap();
        assert!(
            context
                .read(chart, |chart| chart.view.is_hidden("visits"))
                .unwrap()
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [ChartEvent::LegendSelect {
                name: Arc::from("visits"),
                selected: false
            }]
        );
        // The series is gone from the new layout.
        let layout = layout_of(&context, chart);
        assert!(layout.marks.draws.iter().all(|draw| draw.series != 0));
    }

    #[test]
    fn the_wheel_zooms_around_the_pointer_and_the_page_still_scrolls_elsewhere() {
        let option = weekly().data_zoom(DataZoom::inside());
        let (mut context, document, chart, events) = chart_in(option);
        let layout = layout_of(&context, chart);
        let plot = layout.plot.unwrap();
        let mut input = HeadlessInput::bind(&mut context, document);
        let wheel = |x: f32, y: f32| {
            InputPayload::Wheel(WheelInput {
                pointer_id: nana_ui_input::PointerId(0),
                x,
                y,
                delta_x: 0.0,
                delta_y: -240.0,
                unit: WheelUnit::Pixels,
                modifiers: InputModifiers::default(),
            })
        };
        let outcome = input
            .route(&mut context, wheel(plot[0] + 10.0, plot[1] + 10.0))
            .unwrap();
        assert!(outcome.handled);
        let window = context
            .read(chart, |chart| chart.view.zoom_window(&chart.option, 0))
            .unwrap()
            .unwrap();
        assert!(window.1 - window.0 < 100.0, "{window:?}");
        // Zoomed near the left edge, the window stays there.
        assert!(window.0 < 5.0, "{window:?}");
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(ChartEvent::DataZoom { index: 0, .. })
        ));
        // Outside the plot the wheel is not the chart's.
        let outcome = input.route(&mut context, wheel(590.0, 390.0)).unwrap();
        assert!(!outcome.handled);
    }

    fn wheel_at(x: f32, y: f32, delta_y: f32) -> InputPayload {
        InputPayload::Wheel(WheelInput {
            pointer_id: nana_ui_input::PointerId(0),
            x,
            y,
            delta_x: 0.0,
            delta_y,
            unit: WheelUnit::Pixels,
            modifiers: InputModifiers::default(),
        })
    }

    fn windows(context: &AppContext, chart: Entity<Chart>) -> Vec<(f64, f64)> {
        context
            .read(chart, |chart| {
                (0..chart.option.data_zoom.len())
                    .map(|index| chart.view.zoom_window(&chart.option, index).unwrap())
                    .collect()
            })
            .unwrap()
    }

    #[test]
    fn a_selection_box_zooms_both_axes_and_a_double_click_goes_back() {
        let option = weekly()
            .data_zoom(DataZoom::select())
            .data_zoom(DataZoom::select().y_axis(0));
        let (mut context, document, chart, events) = chart_in(option);
        let plot = layout_of(&context, chart).plot.unwrap();
        let (w, h) = (plot[2] - plot[0], plot[3] - plot[1]);
        let from = (plot[0] + w * 0.25, plot[1] + h * 0.25);
        let to = (plot[0] + w * 0.75, plot[1] + h * 0.75);
        let mut input = HeadlessInput::bind(&mut context, document);
        input
            .pointer(&mut context, PointerPhase::Down, from.0, from.1)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, to.0, to.1)
            .unwrap();
        // The box is drawn while the pointer is down.
        let selection = context.read(chart, |chart| chart.selection).unwrap();
        let selection = selection.expect("selection box");
        assert!((selection[0] - from.0).abs() < 0.5 && (selection[3] - to.1).abs() < 0.5);
        let paint = context.world().extract_nodes(&[chart.stable_id()]);
        assert!(
            paint[0]
                .widget_paint
                .as_deref()
                .unwrap()
                .ops
                .iter()
                .any(|op| op.slot == 14)
        );
        input
            .pointer(&mut context, PointerPhase::Up, to.0, to.1)
            .unwrap();
        assert_eq!(context.read(chart, |chart| chart.selection).unwrap(), None);
        let [x, y] = windows(&context, chart)[..] else {
            panic!()
        };
        // Both windows narrowed to about the middle half.
        assert!(x.1 - x.0 < 80.0 && x.0 > 10.0, "{x:?}");
        assert!(
            (y.0 - 25.0).abs() < 1.0 && (y.1 - 75.0).abs() < 1.0,
            "{y:?}"
        );
        let zoomed = events.lock().unwrap().len();
        assert_eq!(zoomed, 2);
        // A double click restores the option's windows.
        let (cx, cy) = ((plot[0] + plot[2]) * 0.5, (plot[1] + plot[3]) * 0.5);
        for _ in 0..2 {
            input
                .pointer(&mut context, PointerPhase::Down, cx, cy)
                .unwrap();
            input
                .pointer(&mut context, PointerPhase::Up, cx, cy)
                .unwrap();
        }
        assert_eq!(windows(&context, chart), vec![(0.0, 100.0), (0.0, 100.0)]);
        // Each window that moved back is said once; the first press of the
        // double click is still a click on the category.
        let after: Vec<_> = events.lock().unwrap()[zoomed..].to_vec();
        let restored: Vec<_> = after
            .iter()
            .filter(|event| matches!(event, ChartEvent::DataZoom { .. }))
            .collect();
        assert_eq!(restored.len(), 2, "{after:?}");
        assert!(restored.iter().all(|event| matches!(
            event,
            ChartEvent::DataZoom {
                start: 0.0,
                end: 100.0,
                ..
            }
        )));
        assert!(matches!(after[0], ChartEvent::Click { .. }), "{after:?}");
    }

    #[test]
    fn the_wheel_zooms_a_y_axis_around_the_pointer_and_its_slider_follows() {
        let option = weekly()
            .data_zoom(DataZoom::inside().y_axis(0))
            .data_zoom(DataZoom::slider().y_axis(0));
        let (mut context, document, chart, events) = chart_in(option);
        let layout = layout_of(&context, chart);
        let plot = layout.plot.unwrap();
        assert!(layout.y_slider.is_some());
        let mut input = HeadlessInput::bind(&mut context, document);
        // Near the top: the window shrinks towards the top.
        let outcome = input
            .route(
                &mut context,
                wheel_at(plot[0] + 10.0, plot[1] + 2.0, -240.0),
            )
            .unwrap();
        assert!(outcome.handled);
        let [inside_window, slider_window] = windows(&context, chart)[..] else {
            panic!()
        };
        assert!(
            inside_window.1 - inside_window.0 < 100.0,
            "{inside_window:?}"
        );
        assert!(inside_window.1 > 95.0, "{inside_window:?}");
        // Entries on one axis share the window, each one said.
        assert_eq!(inside_window, slider_window);
        assert_eq!(events.lock().unwrap().len(), 2);
        // Dragging the slider's window down moves the window down.
        let slider = layout_of(&context, chart).y_slider.unwrap();
        let x = (slider.track[0] + slider.track[2]) * 0.5;
        let y = (slider.window[1] + slider.window[3]) * 0.5;
        input
            .pointer(&mut context, PointerPhase::Down, x, y)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, x, y + 20.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, x, y + 20.0)
            .unwrap();
        let moved = windows(&context, chart)[0];
        assert!(moved.0 < inside_window.0, "{moved:?} {inside_window:?}");
        assert!(
            ((moved.1 - moved.0) - (inside_window.1 - inside_window.0)).abs() < 1e-6,
            "{moved:?}"
        );
    }

    #[test]
    fn a_click_on_a_slice_names_it() {
        let option = ChartOption::new().animation(Animation::disabled()).series(
            PieSeries::new("share", [PieItem::new("a", 1.0), PieItem::new("b", 1.0)])
                .label(PieLabelPosition::None),
        );
        let (mut context, document, chart, events) = chart_in(option);
        let bounds = context
            .world()
            .component_layout_box(chart.stable_id())
            .unwrap();
        // `b` is the left half.
        let (x, y) = (
            bounds.x + bounds.width * 0.5 - 40.0,
            bounds.y + bounds.height * 0.5,
        );
        let mut input = HeadlessInput::bind(&mut context, document);
        input
            .pointer(&mut context, PointerPhase::Move, x, y)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Down, x, y)
            .unwrap();
        input.pointer(&mut context, PointerPhase::Up, x, y).unwrap();
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [ChartEvent::Click {
                series: 0,
                index: 1
            }]
        );
    }

    #[test]
    fn a_new_option_moves_from_what_was_shown() {
        let (mut context, _, chart, _) = chart_in(weekly().animation(Animation::default()));
        let first = layout_of(&context, chart);
        assert!(first.marks.transition.is_some_and(|motion| motion.reveal));
        context
            .update_component(chart, |chart, _| {
                chart.set_option(
                    ChartOption::new()
                        .x_axis(Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri"]))
                        .y_axis(Axis::value())
                        .series(LineSeries::new("visits", [1.0, 1.0, 1.0, 1.0, 1.0])),
                )
            })
            .unwrap();
        let next = layout_of(&context, chart);
        assert_ne!(first.marks.revision, next.marks.revision);
        let motion = next.marks.transition.expect("an update moves");
        assert!(!motion.reveal);
    }

    #[test]
    fn hovering_mark_line_and_mark_area_updates_emphasis_and_tooltip() {
        use nana_ui_charts::{MARK_LINE_INDEX, MarkArea, MarkAreaItem, MarkLine, MarkLineItem};
        let option = ChartOption::new()
            .animation(Animation::disabled())
            .x_axis(Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri"]))
            .y_axis(Axis::value().max(nana_ui_charts::AxisBound::Value(100.0)))
            .series(
                LineSeries::new("cpu", [20.0, 20.0, 20.0, 20.0, 20.0])
                    .mark_line(MarkLine::new([MarkLineItem::y(80.0).name("alert")]))
                    .mark_area(MarkArea::new([MarkAreaItem::y(40.0, 60.0).name("normal")])),
            );
        let (mut context, document, chart, _) = chart_in(option);
        let layout = layout_of(&context, chart);
        let plot = layout.plot.unwrap();
        let line_hit = layout
            .hit
            .marks
            .iter()
            .find(|m| &*m.name == "alert")
            .unwrap();
        let nana_ui_charts::hit::MarkRegion::Line { a, b } = line_hit.region else {
            panic!()
        };
        let lx = (a[0] + b[0]) * 0.5;
        let ly = (a[1] + b[1]) * 0.5;
        let mut input = HeadlessInput::bind(&mut context, document);
        input
            .pointer(&mut context, PointerPhase::Move, lx, ly)
            .unwrap();
        let hover = context.read(chart, |chart| chart.hover).unwrap();
        assert_eq!(hover.current, [0, MARK_LINE_INDEX]);
        let tooltip_id = context.component_lifecycle.chart_tooltips[&chart.stable_id()];
        let tooltip = Entity::<ChartTooltip>::from_stable_id(tooltip_id);
        let content = context.read(tooltip, |t| Arc::clone(&t.content)).unwrap();
        assert_eq!(&*content.title, "cpu");
        assert_eq!(&*content.rows[0].name, "alert");
        assert_eq!(&*content.rows[0].value, "80");

        // Moving inside the mark area (away from the mark line) appends its row to the axis tooltip.
        let area_hit = layout
            .hit
            .marks
            .iter()
            .find(|m| &*m.name == "normal")
            .unwrap();
        let nana_ui_charts::hit::MarkRegion::Area(rect) = area_hit.region else {
            panic!()
        };
        let ax = plot[0] + (plot[2] - plot[0]) * 0.5;
        let ay = (rect[1] + rect[3]) * 0.5;
        input
            .pointer(&mut context, PointerPhase::Move, ax, ay)
            .unwrap();
        let hover = context.read(chart, |chart| chart.hover).unwrap();
        assert_eq!(hover.current[0], nana_ui_charts::hit::ANY_SERIES);
        let content = context.read(tooltip, |t| Arc::clone(&t.content)).unwrap();
        assert!(content.rows.iter().any(|r| &*r.name == "normal"));
    }

    #[test]
    fn single_axis_box_select_spans_cross_axis_and_tiny_drag_emits_click() {
        let option = weekly().data_zoom(DataZoom::select());
        let (mut context, document, chart, events) = chart_in(option);
        let plot = layout_of(&context, chart).plot.unwrap();
        let (w, h) = (plot[2] - plot[0], plot[3] - plot[1]);
        let mut input = HeadlessInput::bind(&mut context, document);
        // Tiny <8px drag inside the plot acts as a click and does not zoom.
        let cx = plot[0] + w * 0.5;
        let cy = plot[1] + h * 0.5;
        input
            .pointer(&mut context, PointerPhase::Move, cx, cy)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Down, cx, cy)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, cx + 2.0, cy + 2.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, cx + 2.0, cy + 2.0)
            .unwrap();
        assert_eq!(windows(&context, chart), vec![(0.0, 100.0)]);
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [ChartEvent::Click { .. }]
        ));

        // Dragging horizontally with only x select_on_drag spans the full plot height.
        let from = (plot[0] + w * 0.25, plot[1] + h * 0.4);
        let to = (plot[0] + w * 0.75, plot[1] + h * 0.6);
        input
            .pointer(&mut context, PointerPhase::Down, from.0, from.1)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, to.0, to.1)
            .unwrap();
        let selection = context
            .read(chart, |chart| chart.selection)
            .unwrap()
            .unwrap();
        assert!((selection[1] - plot[1]).abs() < 1e-3);
        assert!((selection[3] - plot[3]).abs() < 1e-3);
        input
            .pointer(&mut context, PointerPhase::Up, to.0, to.1)
            .unwrap();
        let [x_win] = windows(&context, chart)[..] else {
            panic!()
        };
        assert!(x_win.1 - x_win.0 < 80.0 && x_win.0 > 10.0, "{x_win:?}");
    }
}
