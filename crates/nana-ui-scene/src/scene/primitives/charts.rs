//! Chart geometry to primitives: the marks as one chart primitive, the axis
//! pointer around them, the labels as text; a tooltip's dots and rows.
use super::*;

/// Chart label slots.
const CHART_TEXTS: u32 = 41;
/// Chart tooltip text slots.
const CHART_TOOLTIP_TEXTS: u32 = 42;

const POINTER_BAND_SLOT: u64 = 10;
const MARKS_SLOT: u64 = 11;
const POINTER_LINE_SLOT: u64 = 12;
const POINTER_CROSS_SLOT: u64 = 13;
const TOOLTIP_DOTS_SLOT: u64 = 10;
/// A tooltip's color dots are circles.
const DOT_RADIUS: f32 = 4.0;

pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let id = node.id;
    let visual = VisualPrimitiveContext {
        node: id,
        transform: context.transform,
        clips: context.clips,
        opacity: context.opacity,
        z_index: node.z_index,
        document_order: context.node_order,
    };
    match node.component_geometry.as_deref() {
        Some(ComponentGeometry::Chart {
            layout,
            hover,
            origin,
        }) => {
            let at = |p: [f32; 2]| [origin[0] + p[0], origin[1] + p[1]];
            let [line_color, band_color] = layout.pointer_colors;
            if let Some(nana_ui_charts::PointerGeometry::Band(band)) = hover.pointer {
                let [x0, y0] = at([band[0], band[1]]);
                let [x1, y1] = at([band[2], band[3]]);
                emit(visual_quad_batch(
                    &visual,
                    POINTER_BAND_SLOT,
                    [SceneRect {
                        x: x0,
                        y: y0,
                        width: x1 - x0,
                        height: y1 - y0,
                    }],
                    VisualQuadStyle::solid(band_color),
                ));
            }
            if !layout.marks.is_empty() {
                emit(ScenePrimitive {
                    id: PrimitiveId {
                        node: id,
                        slot: MARKS_SLOT,
                    },
                    node: id,
                    bounds: context.bounds,
                    transform: context.transform,
                    clips: Arc::clone(context.clips),
                    opacity: context.opacity,
                    z_index: node.z_index,
                    document_order: context.node_order,
                    kind: ScenePrimitiveKind::Chart {
                        marks: layout.marks.clone(),
                        origin: *origin,
                        hover: SceneChartHover {
                            current: hover.current,
                            previous: hover.previous,
                            focus: hover.focus,
                            since: hover.since,
                            duration: nana_ui_charts::hit::HOVER_DURATION,
                            growth: nana_ui_charts::hit::EMPHASIS_GROWTH,
                        },
                    },
                });
            }
            // A hairline on a half pixel stays one device pixel wide.
            let crisp = |v: f32| v.floor() + 0.5;
            match hover.pointer {
                Some(nana_ui_charts::PointerGeometry::Line { from, to }) => {
                    let (a, b) = (at(from), at(to));
                    let (a, b) = if (a[0] - b[0]).abs() < 0.01 {
                        ([crisp(a[0]), a[1]], [crisp(b[0]), b[1]])
                    } else {
                        ([a[0], crisp(a[1])], [b[0], crisp(b[1])])
                    };
                    emit(pointer_line(&visual, POINTER_LINE_SLOT, a, b, line_color));
                }
                Some(nana_ui_charts::PointerGeometry::Cross { center, plot }) => {
                    let c = at(center);
                    let (x0, y0) = (origin[0] + plot[0], origin[1] + plot[1]);
                    let (x1, y1) = (origin[0] + plot[2], origin[1] + plot[3]);
                    emit(pointer_line(
                        &visual,
                        POINTER_LINE_SLOT,
                        [crisp(c[0]), y0],
                        [crisp(c[0]), y1],
                        line_color,
                    ));
                    emit(pointer_line(
                        &visual,
                        POINTER_CROSS_SLOT,
                        [x0, crisp(c[1])],
                        [x1, crisp(c[1])],
                        line_color,
                    ));
                }
                _ => {}
            }
            for (index, text) in layout.texts.iter().enumerate() {
                let region = ComponentTextRegion {
                    bounds: LayoutBox {
                        x: origin[0] + text.rect[0],
                        y: origin[1] + text.rect[1],
                        width: (text.rect[2] - text.rect[0]).max(0.0),
                        height: (text.rect[3] - text.rect[1]).max(0.0),
                    },
                    content: text.text.clone().into(),
                    color: Some(text.color),
                    font_size: text.size,
                    font_weight: text.weight,
                };
                emit(component_text_primitive(
                    id,
                    collection_slot(CHART_TEXTS, index),
                    &region,
                    TextHorizontalAlignment::Start,
                    false,
                    node,
                    context.transform,
                    Arc::clone(context.clips),
                    context.opacity,
                    context.node_order,
                ));
            }
        }
        Some(ComponentGeometry::ChartTooltip { dots, texts }) => {
            if !dots.is_empty() {
                emit(ScenePrimitive {
                    id: PrimitiveId {
                        node: id,
                        slot: TOOLTIP_DOTS_SLOT,
                    },
                    node: id,
                    // The dots sit inside the tooltip's box.
                    bounds: context.bounds,
                    transform: context.transform,
                    clips: Arc::clone(context.clips),
                    opacity: context.opacity,
                    z_index: node.z_index,
                    document_order: context.node_order,
                    kind: ScenePrimitiveKind::QuadColorBatch {
                        bounds: dots.iter().map(|(rect, _)| scene_rect(*rect)).collect(),
                        colors: dots.iter().map(|(_, color)| *color).collect(),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: [DOT_RADIUS; 4],
                    },
                });
            }
            for (index, region) in texts.iter().enumerate() {
                emit(component_text_primitive(
                    id,
                    collection_slot(CHART_TOOLTIP_TEXTS, index),
                    region,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    context.transform,
                    Arc::clone(context.clips),
                    context.opacity,
                    context.node_order,
                ));
            }
        }
        _ => {}
    }
}

fn pointer_line(
    visual: &VisualPrimitiveContext<'_>,
    slot: u64,
    a: [f32; 2],
    b: [f32; 2],
    color: [f32; 4],
) -> ScenePrimitive {
    let bounds = SceneRect {
        x: a[0].min(b[0]) - 1.0,
        y: a[1].min(b[1]) - 1.0,
        width: (a[0] - b[0]).abs() + 2.0,
        height: (a[1] - b[1]).abs() + 2.0,
    };
    let mut line = visual_stroke(visual, slot, bounds, vec![a, b], 1.0, color);
    if let ScenePrimitiveKind::Stroke { cap, pattern, .. } = &mut line.kind {
        *cap = StrokeCap::Butt;
        *pattern = Some(Box::new(StrokePattern {
            dash: vec![4.0, 3.0],
            ..Default::default()
        }));
    }
    line
}
