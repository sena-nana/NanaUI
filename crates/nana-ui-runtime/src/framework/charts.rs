//! A chart's pointer: hover with its tooltip and axis pointer, legend
//! clicks, item clicks, and the zoom window's wheel, pan and slider.
use super::*;
use crate::charts::{
    ChartDrag, ChartEvent, ChartHoverVisual, ChartSpec, ChartTooltip, ChromeLabelMeasure, ZoomGrab,
};
use crate::{Chart, NodePointer, NodePointerHooks, NodeWheel, StandardVisual};
use nana_ui_charts::hit::{ChartHover, NO_HOVER, TooltipContent};
use nana_ui_charts::{ChartLayout, DataZoomKind, EmphasisFocus, Series};

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

fn shown(cx: &AppContext, id: StableNodeId) -> Option<(Arc<ChartSpec>, Arc<ChartLayout>)> {
    match cx.world.standard_visual(id)? {
        StandardVisual::Chart { spec, .. } => {
            let layout = spec.shown()?;
            Some((spec, layout))
        }
        _ => None,
    }
}

fn focus_of(spec: &ChartSpec, series: u32) -> u32 {
    let focus = match spec.option.series.get(series as usize) {
        Some(Series::Line(line)) => line.focus,
        Some(Series::Bar(bar)) => bar.focus,
        Some(Series::Scatter(scatter)) => scatter.focus,
        _ => EmphasisFocus::None,
    };
    if focus == EmphasisFocus::Series {
        series
    } else {
        NO_HOVER
    }
}

/// The first zoom entry of `kind` on the x axis.
fn zoom_entry(spec: &ChartSpec, kind: DataZoomKind) -> Option<usize> {
    spec.option
        .data_zoom
        .iter()
        .position(|zoom| zoom.x_axis_index == 0 && zoom.kind == kind)
}

fn inside(rect: [f32; 4], x: f32, y: f32) -> bool {
    x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3]
}

fn chart_moved(cx: &mut AppContext, chart: Entity<Chart>, pointer: NodePointer) -> HandledResult {
    let id = chart.stable_id();
    if pointer.captured
        && let Some(drag) = cx.read(chart, |chart| chart.drag)?
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
    hide_tooltip(cx, id)
}

fn chart_pressed(cx: &mut AppContext, chart: Entity<Chart>, pointer: NodePointer) -> HandledResult {
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
    // The slider: its ends resize the window, the window moves, the track
    // jumps there.
    if let Some(slider) = layout.slider
        && inside(slider.track, x, y)
    {
        let mut window = zoom_window(cx, chart, slider.zoom_index)?;
        let width = (slider.track[2] - slider.track[0]).max(1.0);
        let grab = if (x - slider.window[0]).abs() <= HANDLE_REACH {
            ZoomGrab::Start
        } else if (x - slider.window[2]).abs() <= HANDLE_REACH {
            ZoomGrab::End
        } else {
            ZoomGrab::Window
        };
        if grab == ZoomGrab::Window && !(slider.window[0]..=slider.window[2]).contains(&x) {
            let span = window.1 - window.0;
            let center = ((x - slider.track[0]) / width) as f64 * 100.0;
            let start = (center - span * 0.5).clamp(0.0, 100.0 - span);
            window = (start, start + span);
            set_zoom(cx, chart, slider.zoom_index, window)?;
        }
        let drag = ChartDrag::Zoom {
            zoom: slider.zoom_index,
            grab,
            x,
            window,
            per_px: 100.0 / width as f64,
        };
        cx.update_component(chart, |chart, _| chart.drag = Some(drag))?;
        return Ok(true);
    }
    // Dragging inside the plot pans the window the other way.
    if let Some(plot) = layout.plot
        && inside(plot, x, y)
        && let Some(zoom) = zoom_entry(&spec, DataZoomKind::Inside)
        && spec.option.data_zoom[zoom].move_on_drag
    {
        let window = zoom_window(cx, chart, zoom)?;
        let drag = ChartDrag::Zoom {
            zoom,
            grab: ZoomGrab::Window,
            x,
            window,
            per_px: -(window.1 - window.0) / (plot[2] - plot[0]).max(1.0) as f64,
        };
        cx.update_component(chart, |chart, _| chart.drag = Some(drag))?;
        return Ok(true);
    }
    let hover = cx.read(chart, |chart| chart.hover.current)?;
    if hover[0] == NO_HOVER {
        return Ok(false);
    }
    cx.update_component(chart, |chart, _| {
        chart.drag = Some(ChartDrag::Press { x, y, hover });
    })?;
    Ok(true)
}

fn chart_drag(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    drag: ChartDrag,
    pointer: NodePointer,
) -> HandledResult {
    let ChartDrag::Zoom {
        zoom,
        grab,
        x,
        window: (start, end),
        per_px,
    } = drag
    else {
        return Ok(true);
    };
    let delta = (pointer.x - x) as f64 * per_px;
    let next = match grab {
        ZoomGrab::Window => {
            let span = end - start;
            let start = (start + delta).clamp(0.0, 100.0 - span);
            (start, start + span)
        }
        ZoomGrab::Start => ((start + delta).clamp(0.0, end - MIN_ZOOM_SPAN), end),
        ZoomGrab::End => (start, (end + delta).clamp(start + MIN_ZOOM_SPAN, 100.0)),
    };
    set_zoom(cx, chart, zoom, next)?;
    hide_tooltip(cx, chart.stable_id())?;
    Ok(true)
}

/// The window of `option.data_zoom[zoom]` now, in percent.
fn zoom_window(
    cx: &AppContext,
    chart: Entity<Chart>,
    zoom: usize,
) -> Result<(f64, f64), FrameworkError> {
    cx.read(chart, |chart| {
        chart
            .view
            .zoom_window(&chart.option, zoom)
            .unwrap_or((0.0, 100.0))
    })
}

fn set_zoom(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    zoom: usize,
    window: (f64, f64),
) -> Result<(), FrameworkError> {
    if zoom_window(cx, chart, zoom)? == window {
        return Ok(());
    }
    cx.update_component(chart, |chart, view| {
        let slots = &mut chart.view.zoom;
        if slots.len() <= zoom {
            slots.resize(zoom + 1, None);
        }
        slots[zoom] = Some(window);
        view.emit(ChartEvent::DataZoom {
            index: zoom,
            start: window.0,
            end: window.1,
        });
    })
}

fn chart_released(
    cx: &mut AppContext,
    chart: Entity<Chart>,
    pointer: NodePointer,
) -> HandledResult {
    let drag = cx.update_component(chart, |chart, _| chart.drag.take())?;
    if let Some(ChartDrag::Press { x, y, hover }) = drag
        && (pointer.x - x).abs() < CLICK_SLOP
        && (pointer.y - y).abs() < CLICK_SLOP
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
    let Some(zoom) = zoom_entry(&spec, DataZoomKind::Inside) else {
        return Ok(false);
    };
    if !spec.option.data_zoom[zoom].zoom_on_wheel || !inside(plot, wheel.x, wheel.y) {
        return Ok(false);
    }
    let window = zoom_window(cx, chart, zoom)?;
    let span = window.1 - window.0;
    // Turning the wheel away zooms in, around the pointer.
    let scale = (wheel.delta_y as f64 * 0.002).exp();
    let next_span = (span * scale).clamp(MIN_ZOOM_SPAN, 100.0);
    let at = ((wheel.x - plot[0]) / (plot[2] - plot[0]).max(1.0)).clamp(0.0, 1.0) as f64;
    let focus = window.0 + at * span;
    let start = (focus - at * next_span).clamp(0.0, 100.0 - next_span);
    set_zoom(cx, chart, zoom, (start, start + next_span))?;
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
    let below = y + gap;
    let above = y - gap - size[1];
    let py = if below + size[1] <= viewport.height - padding || above < padding {
        below
    } else {
        above
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
        let nodes = context.world().extract_nodes(&[chart.stable_id()]);
        let Some(crate::ComponentGeometry::Chart { layout, .. }) =
            nodes[0].component_geometry.as_deref()
        else {
            panic!("chart geometry");
        };
        Arc::clone(layout)
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
}
