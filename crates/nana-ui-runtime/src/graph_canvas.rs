//! Backend-neutral graph canvas.
//!
//! Applications own [`GraphModel`], viewport, selection, and persistence.
//! This component owns transient pointer state and emits [`GraphCanvasEvent`].
//! Default paint is [`GraphCanvasVisual`]: grid lines, node frames,
//! ports, and sampled edge curves. Node interiors are optional child Regions
//! or `"nana.host-texture"` slots ([`GraphNodeContent`]); this component does
//! not rasterize mermaid, math, or formula pixels. Unslotted nodes keep the
//! default frame.
//!
//! [`CustomRenderNode`] (`renderer = "graph-canvas"`) is not projected here
//! because the default Scene painter rejects unregistered custom renderers.
//! Hosts that register a Scene GPU painter for [`GRAPH_CANVAS_RENDERER`] may
//! attach [`GraphCanvas::custom_render`]. No host Device or Queue is created here.

use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{
    GRAPH_NODE_TITLE_HEIGHT, GRAPH_PORT_INSET, GraphCanvasId, GraphEndpoint, GraphModel,
    GraphNodeId, GraphPoint, GraphPortId, GraphPortKind, GraphPortSide, GraphSelection, GraphSize,
    GraphTargetDescriptor, GraphViewport, LengthSpec, OverflowSpec, PositionSpec,
    SemanticColorRole, SemanticPalette, cubic_point, port_tangent,
};

#[cfg(test)]
use nana_ui_core::{GRAPH_MAX_ZOOM, GraphEdge, GraphNode, GraphPort};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentTextRegion, ComponentView, ComputedStyle,
    CustomRenderNode, HOST_TEXTURE_RENDERER, InteractionState, InteractionStyle, LayoutBox,
    LineCap, MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId,
    TextHorizontalAlignment, UiWorld, WidgetBatchColors, WidgetPaintCtx, WidgetPaintOp,
    WidgetPaintOutput, WidgetQuad, WidgetQuadBatch, WidgetStroke, WidgetVisual,
};

pub const GRAPH_CANVAS_RENDERER: &str = "graph-canvas";

const DEFAULT_GRID_SPACING: f32 = nana_ui_core::space::PAGE;
const KEYBOARD_PAN_STEP: f32 = nana_ui_core::UI_METRICS.control_height;
const KEYBOARD_ZOOM_FACTOR: f32 = 1.2;
const FIT_PADDING: f32 = nana_ui_core::UI_METRICS.selection_height;
const DEFAULT_CANVAS_ID: &str = "graph-canvas";
const DEFAULT_LABEL: &str = "Graph canvas";
const GRAPH_NODE_CONTENT_TAG: &str = "graph-node-content";

#[derive(Debug, Clone, PartialEq)]
pub enum GraphCanvasEvent {
    SelectionChanged(Option<GraphSelection>),
    NodePositionInput {
        node: GraphNodeId,
        position: GraphPoint,
    },
    NodePositionChanged {
        node: GraphNodeId,
        position: GraphPoint,
    },
    ConnectionRequested {
        source: GraphEndpoint,
        target: GraphEndpoint,
    },
    /// A live viewport update produced while a pointer is panning.
    ViewportInput(GraphViewport),
    /// A committed viewport update produced on pointer release, wheel or keyboard input.
    ViewportChanged(GraphViewport),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphPointerButton {
    Primary,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphCanvasAdjustment {
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    ZoomIn,
    ZoomOut,
    Fit,
    ClearSelection,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GraphScrollDelta {
    Lines { y: f32 },
    Pixels { y: f32 },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum GraphInteraction {
    #[default]
    None,
    Pan {
        pointer_id: u64,
        origin: GraphPoint,
        viewport: GraphViewport,
    },
    NodeDrag {
        pointer_id: u64,
        node: GraphNodeId,
        origin: GraphPoint,
        node_origin: GraphPoint,
        current: GraphPoint,
    },
    Connection {
        pointer_id: u64,
        source: GraphEndpoint,
        current: GraphPoint,
    },
}

impl GraphInteraction {
    fn pointer_id(&self) -> Option<u64> {
        match self {
            Self::None => None,
            Self::Pan { pointer_id, .. }
            | Self::NodeDrag { pointer_id, .. }
            | Self::Connection { pointer_id, .. } => Some(*pointer_id),
        }
    }
}

/// Resolving a window point against a graph canvas.
///
/// Right-click routing ([`crate::SecondaryPress`]) reports window coordinates,
/// while [`GraphCanvas::hit_test`] and [`GraphViewport`] work in canvas-local
/// view space. [`crate::AppContext::graph_canvas_hit_at`] bridges the two so a
/// host never re-derives the canvas transform.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphCanvasHit {
    /// The point in canvas-local view space, the space
    /// [`GraphViewport::view_to_world`] converts from.
    pub local: GraphPoint,
    /// Node, port or edge under the point; `None` on empty canvas background.
    pub selection: Option<GraphSelection>,
}

pub(crate) const PORT_RADIUS: f32 = nana_ui_core::space::XS;
pub(crate) const PORT_RADIUS_ACTIVE: f32 = PORT_RADIUS + nana_ui_core::HAIRLINE;
pub(crate) const PORT_GRAB_RADIUS: f32 = nana_ui_core::space::XL;
pub(crate) const PAN_THRESHOLD: f32 = nana_ui_core::space::XS;
pub(crate) const NODE_TITLE_SIZE_MIN: f32 = 9.0;
pub(crate) const PORT_LABEL_SIZE: f32 = 9.5;
pub(crate) const PORT_LABEL_WIDTH: f32 = 80.0;

/// View-space node rectangle for Scene quads.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNodePaint {
    pub label: Arc<str>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub title_height: f32,
    pub selected: bool,
    pub hovered: bool,
}

/// View-space port disc and label for Scene quads.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphPortPaint {
    pub label: Arc<str>,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub side: GraphPortSide,
    pub kind: GraphPortKind,
    pub selected: bool,
    pub hovered: bool,
}

/// View-space cubic Bézier for an edge or a live connection preview.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphEdgePaint {
    pub curve: [GraphPoint; 4],
    pub selected: bool,
    pub hovered: bool,
    pub connecting: bool,
    pub label: Option<Arc<str>>,
}

/// Resolved geometry and paint primitives for a [`GraphCanvas`] node.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphCanvasLayout {
    pub nodes: Vec<(LayoutBox, ComponentTextRegion, [f32; 4], Option<[f32; 4]>)>,
    pub separators: Vec<LayoutBox>,
    pub ports: Vec<(LayoutBox, [f32; 4], [f32; 4], f32)>,
    pub port_labels: Vec<(ComponentTextRegion, TextHorizontalAlignment)>,
    pub edges: Vec<(Vec<[f32; 2]>, [f32; 4])>,
    pub edge_labels: Vec<ComponentTextRegion>,
    pub grid: Vec<LayoutBox>,
    pub background: [f32; 4],
    pub grid_color: [f32; 4],
    pub separator_color: [f32; 4],
}

impl GraphCanvasLayout {
    /// Lowers this resolved layout into node-local widget paint operations.
    #[must_use]
    pub fn to_widget_paint(
        &self,
        bounds: LayoutBox,
        chrome_radii: nana_ui_core::ChromeRadii,
        style: &ComputedStyle,
    ) -> WidgetPaintOutput {
        let mut out = WidgetPaintOutput::new();
        out.owns_node_text = true;
        out.suppress_root_surface = true;

        out.push(
            10,
            WidgetPaintOp::Quad(WidgetQuad {
                rect: bounds,
                fill: Some(self.background),
                ..WidgetQuad::default()
            }),
        );
        if !self.grid.is_empty() {
            out.push(
                11,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: crate::widget_visual::union_layout_rects(&self.grid),
                    rects: Arc::from(self.grid.as_slice()),
                    colors: WidgetBatchColors::Uniform(self.grid_color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            );
        }
        for (index, (points, color)) in self.edges.iter().enumerate() {
            if points.len() < 2 {
                continue;
            }
            out.push(
                graph_slot(1, index),
                WidgetPaintOp::Stroke(WidgetStroke {
                    points: points.clone(),
                    width: 1.6,
                    color: *color,
                    cap: LineCap::Round,
                    dash: None,
                    snap_hairline: false,
                }),
            );
        }
        for (index, (node_bounds, label, fill, border)) in self.nodes.iter().enumerate() {
            out.push(
                graph_slot(2, index),
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: *node_bounds,
                    fill: Some(*fill),
                    border: border.map(|color| (color, 1.0)),
                    corner_radius: [chrome_radii.sm; 4],
                    ..WidgetQuad::default()
                }),
            );
            out.push(
                graph_slot(4, index),
                WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    label,
                    style,
                    TextHorizontalAlignment::Start,
                    true,
                )),
            );
        }
        if !self.separators.is_empty() {
            out.push(
                graph_slot(3, 0),
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: crate::widget_visual::union_layout_rects(&self.separators),
                    rects: Arc::from(self.separators.as_slice()),
                    colors: WidgetBatchColors::Uniform(self.separator_color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            );
        }
        for (index, (port, fill, border, border_width)) in self.ports.iter().enumerate() {
            out.push(
                graph_slot(5, index),
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: *port,
                    fill: Some(*fill),
                    border: Some((*border, *border_width)),
                    corner_radius: [999.0; 4],
                    ..WidgetQuad::default()
                }),
            );
        }
        for (index, (label, alignment)) in self.port_labels.iter().enumerate() {
            out.push(
                graph_slot(6, index),
                WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    label, style, *alignment, true,
                )),
            );
        }
        for (index, label) in self.edge_labels.iter().enumerate() {
            out.push(
                graph_slot(7, index),
                WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    label,
                    style,
                    TextHorizontalAlignment::Center,
                    true,
                )),
            );
        }
        out
    }
}

/// Visual payload projected by [`GraphCanvas`].
#[derive(Debug, Clone, PartialEq)]
pub struct GraphCanvasVisual {
    pub nodes: Arc<[GraphNodePaint]>,
    pub ports: Arc<[GraphPortPaint]>,
    pub edges: Arc<[GraphEdgePaint]>,
    pub connecting: Option<GraphEdgePaint>,
    pub grid_spacing: f32,
    pub viewport_offset_x: f32,
    pub viewport_offset_y: f32,
    pub viewport_zoom: f32,
}

impl GraphCanvasVisual {
    /// Resolves `bounds`-relative node, port, edge, and grid geometry against `palette`.
    #[must_use]
    pub fn layout_at(&self, bounds: LayoutBox, palette: &SemanticPalette) -> GraphCanvasLayout {
        let mut painted_edges = Vec::new();
        let mut edge_labels = Vec::new();
        for edge in self.edges.iter().chain(self.connecting.as_ref()) {
            let color = graph_edge_stroke_color(palette, edge);
            painted_edges.push((sample_curve(bounds, edge.curve), color));
            if !edge.connecting
                && self.viewport_zoom >= 0.7
                && let Some(label) = edge.label.as_ref()
            {
                let center = cubic_point(edge.curve, 0.5);
                if let Some(label_bounds) = crate::world::intersect_layout_boxes(
                    bounds,
                    LayoutBox {
                        x: bounds.x + center.x - 40.0,
                        y: bounds.y + center.y - 16.0,
                        width: 80.0,
                        height: 12.0,
                    },
                ) {
                    edge_labels.push(ComponentTextRegion {
                        bounds: label_bounds,
                        content: Arc::clone(label).into(),
                        color: Some(palette.muted.as_rgba_array()),
                        font_size: nana_ui_core::type_scale::HINT,
                        font_weight: None,
                    });
                }
            }
        }
        let mut separators = Vec::new();
        let nodes = self
            .nodes
            .iter()
            .filter_map(|node| {
                let raw = LayoutBox {
                    x: bounds.x + node.x,
                    y: bounds.y + node.y,
                    width: node.width.max(0.0),
                    height: node.height.max(0.0),
                };
                let node_bounds = crate::world::intersect_layout_boxes(bounds, raw)?;
                let title_height = node.title_height.clamp(18.0, node_bounds.height.max(18.0));
                if node_bounds.width >= 32.0
                    && node_bounds.height >= title_height
                    && let Some(separator) = crate::world::intersect_layout_boxes(
                        bounds,
                        LayoutBox {
                            x: node_bounds.x,
                            y: node_bounds.y + title_height,
                            width: node_bounds.width,
                            height: 1.0,
                        },
                    )
                {
                    separators.push(separator);
                }
                let label = ComponentTextRegion {
                    bounds: crate::world::intersect_layout_boxes(
                        bounds,
                        LayoutBox {
                            x: raw.x + 10.0,
                            y: raw.y,
                            width: (raw.width - 20.0).max(0.0),
                            height: title_height.min(raw.height),
                        },
                    )
                    .unwrap_or(LayoutBox {
                        x: node_bounds.x,
                        y: node_bounds.y,
                        width: 0.0,
                        height: 0.0,
                    }),
                    content: Arc::clone(&node.label).into(),
                    color: Some(palette.text.as_rgba_array()),
                    font_size: (nana_ui_core::type_scale::META * self.viewport_zoom)
                        .clamp(NODE_TITLE_SIZE_MIN, nana_ui_core::type_scale::BODY),
                    font_weight: Some(nana_ui_core::type_scale::MEDIUM),
                };
                let fill = if node.selected {
                    palette.selected.as_rgba_array()
                } else if node.hovered {
                    palette.hover.as_rgba_array()
                } else {
                    palette.surface.as_rgba_array()
                };
                let border = if node.selected {
                    Some(palette.border_strong.as_rgba_array())
                } else {
                    Some(palette.border.as_rgba_array())
                };
                Some((node_bounds, label, fill, border))
            })
            .collect();
        let mut port_labels = Vec::new();
        let ports = self
            .ports
            .iter()
            .filter_map(|port| {
                let radius = port.radius.max(0.0);
                let disc = crate::world::intersect_layout_boxes(
                    bounds,
                    LayoutBox {
                        x: bounds.x + port.x - radius,
                        y: bounds.y + port.y - radius,
                        width: radius * 2.0,
                        height: radius * 2.0,
                    },
                )?;
                let kind = match port.kind {
                    GraphPortKind::Input => palette.muted.as_rgba_array(),
                    GraphPortKind::Output => palette.accent.as_rgba_array(),
                    GraphPortKind::Bidirectional => palette.warning.as_rgba_array(),
                };
                if self.viewport_zoom >= 0.72 && !port.label.is_empty() {
                    let (mut label, alignment) =
                        port_label_region(bounds, port, palette.muted.as_rgba_array());
                    if let Some(clipped) =
                        crate::world::intersect_layout_boxes(bounds, label.bounds)
                    {
                        label.bounds = clipped;
                        port_labels.push((label, alignment));
                    }
                }
                Some((
                    disc,
                    palette.background.as_rgba_array(),
                    kind,
                    if port.selected { 2.4 } else { 1.6 },
                ))
            })
            .collect();
        GraphCanvasLayout {
            nodes,
            separators,
            ports,
            port_labels,
            edges: painted_edges,
            edge_labels,
            grid: graph_grid_lines(
                bounds,
                self.grid_spacing,
                self.viewport_offset_x,
                self.viewport_offset_y,
                self.viewport_zoom,
            ),
            background: palette.background.as_rgba_array(),
            grid_color: {
                let mut color = palette.border_soft.as_rgba_array();
                color[3] *= 0.72;
                color
            },
            separator_color: palette.border_soft.as_rgba_array(),
        }
    }
}

fn graph_slot(family: u64, index: usize) -> u64 {
    (family << 32) | u64::from(u32::try_from(index).expect("graph primitive family exceeds u32"))
}

impl WidgetVisual for GraphCanvasVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        self.layout_at(ctx.bounds, &ctx.style_model.palette)
            .to_widget_paint(ctx.bounds, ctx.chrome_radii, ctx.style)
    }
}

pub(crate) fn graph_edge_stroke_color(
    palette: &SemanticPalette,
    edge: &GraphEdgePaint,
) -> [f32; 4] {
    if edge.connecting {
        let mut accent = palette.accent.as_rgba_array();
        accent[3] *= 0.8;
        accent
    } else if edge.selected {
        palette.text.as_rgba_array()
    } else if edge.hovered {
        palette.muted.as_rgba_array()
    } else {
        palette.border_strong.as_rgba_array()
    }
}

pub(crate) const CURVE_FLATNESS: f32 = 0.75;

pub(crate) fn sample_curve(bounds: LayoutBox, curve: [GraphPoint; 4]) -> Vec<[f32; 2]> {
    let origin = [bounds.x, bounds.y];
    let mut points = vec![[origin[0] + curve[0].x, origin[1] + curve[0].y]];
    flatten_cubic(&mut points, curve, origin, 0);
    points
}

/// Screen-space flatness (`edge_curve` is already in view space).
fn flatten_cubic(
    points: &mut Vec<[f32; 2]>,
    [p0, p1, p2, p3]: [GraphPoint; 4],
    origin: [f32; 2],
    depth: u32,
) {
    let deviation = line_offset(p1, p0, p3).max(line_offset(p2, p0, p3));
    if depth >= 16 || deviation <= CURVE_FLATNESS {
        points.push([origin[0] + p3.x, origin[1] + p3.y]);
        return;
    }
    let mid = |a: GraphPoint, b: GraphPoint| GraphPoint::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p23 = mid(p2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let split = mid(p012, p123);
    flatten_cubic(points, [p0, p01, p012, split], origin, depth + 1);
    flatten_cubic(points, [split, p123, p23, p3], origin, depth + 1);
}

fn line_offset(point: GraphPoint, start: GraphPoint, end: GraphPoint) -> f32 {
    let abx = end.x - start.x;
    let aby = end.y - start.y;
    let length_sq = abx * abx + aby * aby;
    if length_sq <= f32::EPSILON {
        return point.distance_squared(start).sqrt();
    }
    ((point.x - start.x) * aby - (point.y - start.y) * abx).abs() / length_sq.sqrt()
}

fn graph_grid_lines(
    bounds: LayoutBox,
    base_spacing: f32,
    offset_x: f32,
    offset_y: f32,
    zoom: f32,
) -> Vec<LayoutBox> {
    if !base_spacing.is_finite() || base_spacing <= 0.0 {
        return Vec::new();
    }
    let mut spacing = base_spacing
        * if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        };
    while spacing < 16.0 {
        spacing *= 2.0;
    }
    while spacing > 96.0 {
        spacing *= 0.5;
    }
    if !spacing.is_finite() || spacing < 1.0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut x = offset_x.rem_euclid(spacing);
    while x <= bounds.width {
        lines.push(LayoutBox {
            x: bounds.x + x,
            y: bounds.y,
            width: 1.0,
            height: bounds.height,
        });
        x += spacing;
    }
    let mut y = offset_y.rem_euclid(spacing);
    while y <= bounds.height {
        lines.push(LayoutBox {
            x: bounds.x,
            y: bounds.y + y,
            width: bounds.width,
            height: 1.0,
        });
        y += spacing;
    }
    lines
}

fn port_label_region(
    bounds: LayoutBox,
    port: &GraphPortPaint,
    color: [f32; 4],
) -> (ComponentTextRegion, TextHorizontalAlignment) {
    let width = PORT_LABEL_WIDTH;
    let (x, y, height, align) = match port.side {
        GraphPortSide::Top => (
            port.x - nana_ui_core::space::PAGE_TIGHT * 2.0,
            port.y + nana_ui_core::space::MD,
            nana_ui_core::type_scale::META,
            TextHorizontalAlignment::Center,
        ),
        GraphPortSide::Right => (
            port.x - (width + nana_ui_core::space::MD),
            port.y - nana_ui_core::space::SM,
            nana_ui_core::type_scale::SECTION,
            TextHorizontalAlignment::End,
        ),
        GraphPortSide::Bottom => (
            port.x - nana_ui_core::space::PAGE_TIGHT * 2.0,
            port.y - nana_ui_core::space::PAGE_TIGHT,
            nana_ui_core::type_scale::META,
            TextHorizontalAlignment::Center,
        ),
        GraphPortSide::Left => (
            port.x + nana_ui_core::space::MD,
            port.y - nana_ui_core::space::SM,
            nana_ui_core::type_scale::SECTION,
            TextHorizontalAlignment::Start,
        ),
    };
    (
        ComponentTextRegion {
            bounds: LayoutBox {
                x: bounds.x + x,
                y: bounds.y + y,
                width,
                height,
            },
            content: Arc::clone(&port.label).into(),
            color: Some(color),
            font_size: PORT_LABEL_SIZE,
            font_weight: None,
        },
        align,
    )
}

impl UiWorld {
    /// Resolves `id`'s graph-canvas nodes, ports, edges, labels, and grid geometry.
    #[must_use]
    pub fn graph_canvas_layout(&self, id: StableNodeId) -> Option<GraphCanvasLayout> {
        let visual = self.widget_visual_as::<GraphCanvasVisual>(id)?;
        let bounds = self.component_layout_box(id)?;
        let style_model = self.node_style_model(id);
        Some(visual.layout_at(bounds, &style_model.palette))
    }
}

/// Host-drawn interior of one graph node.
///
/// Default canvas paint still draws the grid, frames, ports, and edges.
/// Interiors are first-class children sampled in document order
/// (`"nana.host-texture"`), not a HUD overlay and not a Live2D/Cubism node.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphNodeContent {
    /// Existing child Region. Layout is the node body below the title bar.
    Region(StableNodeId),
    /// HostTexture registry slot. Assemble allocates a child that samples
    /// [`HOST_TEXTURE_RENDERER`].
    HostTexture(crate::TextureSlotName),
    /// Already-built custom node, still projected onto a child (never the canvas).
    CustomRender(CustomRenderNode),
}

impl GraphNodeContent {
    pub fn region(id: StableNodeId) -> Self {
        Self::Region(id)
    }

    pub fn host_texture(slot: impl Into<crate::TextureSlotName>) -> Self {
        Self::HostTexture(slot.into())
    }

    pub fn custom_render(node: CustomRenderNode) -> Self {
        Self::CustomRender(node)
    }

    pub fn as_child(&self) -> Option<StableNodeId> {
        match self {
            Self::Region(id) => Some(*id),
            _ => None,
        }
    }

    /// HostTexture slots use [`HOST_TEXTURE_RENDERER`]. Empty identities are omitted.
    pub fn as_custom_render(&self) -> Option<CustomRenderNode> {
        match self {
            Self::HostTexture(slot) if !slot.is_empty() => Some(CustomRenderNode::new(
                HOST_TEXTURE_RENDERER,
                slot.to_resource(),
                0,
            )),
            Self::CustomRender(node)
                if !node.renderer.trim().is_empty() && !node.resource.trim().is_empty() =>
            {
                Some(node.clone())
            }
            _ => None,
        }
    }
}

impl From<StableNodeId> for GraphNodeContent {
    fn from(id: StableNodeId) -> Self {
        Self::Region(id)
    }
}

impl From<CustomRenderNode> for GraphNodeContent {
    fn from(node: CustomRenderNode) -> Self {
        Self::CustomRender(node)
    }
}

/// Controlled graph canvas. Viewport and selection live on the view; the
/// application applies model and persistence from typed events.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphCanvas {
    pub canvas_id: GraphCanvasId,
    pub model: GraphModel,
    pub viewport: GraphViewport,
    pub selection: Option<GraphSelection>,
    pub grid_spacing: f32,
    pub disabled: bool,
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
    pub interaction: GraphInteraction,
    pub hover: Option<GraphSelection>,
    pub revision: u64,
    /// Optional per-node interiors. Region children win over HostTexture.
    pub node_contents: Vec<(GraphNodeId, GraphNodeContent)>,
    /// Child identities allocated by [`AppContext::assemble_graph_canvas_contents`].
    allocated_contents: Vec<(GraphNodeId, StableNodeId)>,
}

impl GraphCanvas {
    pub fn new(canvas_id: impl Into<GraphCanvasId>, model: GraphModel) -> Self {
        Self {
            canvas_id: sanitize_canvas_id(canvas_id.into()),
            model,
            viewport: GraphViewport::default(),
            selection: None,
            grid_spacing: DEFAULT_GRID_SPACING,
            disabled: false,
            label: None,
            style: canvas_style(),
            interaction: GraphInteraction::None,
            hover: None,
            revision: 0,
            node_contents: Vec::new(),
            allocated_contents: Vec::new(),
        }
    }

    pub fn viewport(mut self, viewport: GraphViewport) -> Self {
        self.viewport = viewport;
        self
    }

    pub fn selection(mut self, selection: Option<GraphSelection>) -> Self {
        self.selection = selection;
        self
    }

    pub fn grid_spacing(mut self, spacing: f32) -> Self {
        if spacing.is_finite() && spacing >= 8.0 {
            self.grid_spacing = spacing;
        }
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Bind a host-drawn interior to `node`. Replaces any previous content for
    /// that node. Unknown node ids are kept until [`Self::set_model`] prunes them.
    pub fn node_content(mut self, node: impl Into<GraphNodeId>, content: GraphNodeContent) -> Self {
        self.set_node_content(node, content);
        self
    }

    pub fn set_node_content(
        &mut self,
        node: impl Into<GraphNodeId>,
        content: GraphNodeContent,
    ) -> bool {
        let node = node.into();
        if let Some((_, existing)) = self
            .node_contents
            .iter_mut()
            .find(|(existing, _)| *existing == node)
        {
            if *existing == content {
                return false;
            }
            *existing = content;
        } else {
            self.node_contents.push((node, content));
        }
        self.bump_revision();
        true
    }

    pub fn node_contents(&self) -> &[(GraphNodeId, GraphNodeContent)] {
        &self.node_contents
    }

    /// Canvas-local rectangle for a node's interior (below the title, inset
    /// from ports). `None` when the node is missing or the body has no area.
    pub fn node_content_rect(&self, node: &GraphNodeId) -> Option<LayoutBox> {
        let model = self.displayed_model();
        let graph_node = model.node(node)?;
        content_rect_for(graph_node, self.viewport)
    }

    fn content_for(&self, node: &GraphNodeId) -> Option<GraphNodeContent> {
        if let Some((_, content)) = self
            .node_contents
            .iter()
            .find(|(existing, _)| existing == node)
        {
            return Some(content.clone());
        }
        let slot = self
            .model
            .node(node)
            .and_then(|graph_node| graph_node.content_slot.as_deref())
            .map(str::trim)
            .filter(|slot| !slot.is_empty())?;
        Some(GraphNodeContent::host_texture(slot))
    }

    fn child_for(&self, node: &GraphNodeId, content: &GraphNodeContent) -> Option<StableNodeId> {
        content.as_child().or_else(|| {
            self.allocated_contents
                .iter()
                .find(|(existing, _)| existing == node)
                .map(|(_, id)| *id)
        })
    }

    fn prune_contents(&mut self) {
        let known: HashSet<_> = self
            .model
            .nodes()
            .iter()
            .map(|node| node.id.clone())
            .collect();
        self.node_contents.retain(|(node, _)| known.contains(node));
        self.allocated_contents
            .retain(|(node, _)| known.contains(node));
    }

    pub fn paint_nodes(&self) -> Arc<[GraphNodePaint]> {
        let model = self.displayed_model();
        let title_height = (GRAPH_NODE_TITLE_HEIGHT * self.viewport.zoom).clamp(18.0, 34.0);
        model
            .nodes()
            .iter()
            .map(|node| {
                let rect = self.viewport.world_rect_to_view(node.bounds());
                let selected =
                    self.selection.as_ref() == Some(&GraphSelection::Node(node.id.clone()));
                let hovered = self.hover.as_ref() == Some(&GraphSelection::Node(node.id.clone()));
                GraphNodePaint {
                    label: Arc::from(node.label.as_str()),
                    x: rect.origin.x,
                    y: rect.origin.y,
                    width: rect.size.width,
                    height: rect.size.height,
                    title_height,
                    selected,
                    hovered,
                }
            })
            .collect()
    }

    pub fn paint_ports(&self) -> Arc<[GraphPortPaint]> {
        let model = self.displayed_model();
        model
            .nodes()
            .iter()
            .flat_map(|node| {
                node.ports.iter().filter_map(|port| {
                    let point =
                        self.viewport
                            .world_to_view(model.port_position(&GraphEndpoint::new(
                                node.id.clone(),
                                port.id.clone(),
                            ))?);
                    let selection = GraphSelection::Port {
                        node: node.id.clone(),
                        port: port.id.clone(),
                    };
                    let selected = self.selection.as_ref() == Some(&selection);
                    let hovered = self.hover.as_ref() == Some(&selection);
                    Some(GraphPortPaint {
                        label: Arc::from(port.label.as_str()),
                        x: point.x,
                        y: point.y,
                        radius: if selected || hovered {
                            PORT_RADIUS_ACTIVE
                        } else {
                            PORT_RADIUS
                        },
                        side: port.side,
                        kind: port.kind,
                        selected,
                        hovered,
                    })
                })
            })
            .collect()
    }

    pub fn paint_edges(&self) -> Arc<[GraphEdgePaint]> {
        let model = self.displayed_model();
        model
            .edges()
            .iter()
            .filter_map(|edge| {
                Some(GraphEdgePaint {
                    curve: model.edge_curve(edge, self.viewport)?,
                    selected: self.selection.as_ref()
                        == Some(&GraphSelection::Edge(edge.id.clone())),
                    hovered: self.hover.as_ref() == Some(&GraphSelection::Edge(edge.id.clone())),
                    connecting: false,
                    label: edge.label.as_ref().map(|label| Arc::from(label.as_str())),
                })
            })
            .collect()
    }

    pub fn paint_connecting(&self) -> Option<GraphEdgePaint> {
        let GraphInteraction::Connection {
            source, current, ..
        } = &self.interaction
        else {
            return None;
        };
        let model = self.displayed_model();
        let start = self.viewport.world_to_view(model.port_position(source)?);
        let side = model
            .node(&source.node)?
            .ports
            .iter()
            .find(|port| port.id == source.port)
            .map(|port| port.side)?;
        Some(pending_connection_paint(start, *current, side))
    }

    fn displayed_model(&self) -> GraphModel {
        let mut model = self.model.clone();
        if let GraphInteraction::NodeDrag { node, current, .. } = &self.interaction {
            let _ = model.set_node_position(node, *current);
        }
        model
    }

    pub fn set_hover(&mut self, local: Option<GraphPoint>) -> bool {
        let next = local
            .filter(|point| point.is_finite())
            .and_then(|point| self.hit_test(point));
        if self.hover == next {
            return false;
        }
        self.hover = next;
        self.bump_revision();
        true
    }

    pub fn set_model(&mut self, model: GraphModel) {
        if self.model != model {
            self.model = model;
            self.prune_contents();
            self.bump_revision();
        }
    }

    pub fn set_viewport(&mut self, viewport: GraphViewport) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.bump_revision();
        }
    }

    pub fn set_selection(&mut self, selection: Option<GraphSelection>) {
        if self.selection != selection {
            self.selection = selection;
            self.bump_revision();
        }
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn target_descriptors(&self, canvas_size: GraphSize) -> Vec<GraphTargetDescriptor> {
        self.model.target_descriptors(
            &self.canvas_id,
            self.viewport,
            self.selection.as_ref(),
            canvas_size,
        )
    }

    pub fn hit_test(&self, view_position: GraphPoint) -> Option<GraphSelection> {
        if !view_position.is_finite() {
            return None;
        }
        let model = self.displayed_model();
        if let Some(port) = hit_port_disc(&model, self.viewport, view_position) {
            return Some(port);
        }
        model.hit_test(self.viewport, view_position)
    }

    pub fn pointer_press(
        &mut self,
        pointer_id: u64,
        local: GraphPoint,
        window: GraphPoint,
        button: GraphPointerButton,
    ) -> Option<GraphCanvasEvent> {
        if self.disabled || !local.is_finite() || !window.is_finite() {
            return None;
        }
        if self
            .interaction
            .pointer_id()
            .is_some_and(|active| active != pointer_id)
        {
            return None;
        }
        match button {
            GraphPointerButton::Middle => {
                self.begin_pan(pointer_id, window);
                None
            }
            GraphPointerButton::Primary => match self.hit_test(local) {
                Some(GraphSelection::Node(node)) => {
                    let Some(graph_node) = self.model.node(&node) else {
                        self.interaction = GraphInteraction::None;
                        return None;
                    };
                    self.interaction = GraphInteraction::NodeDrag {
                        pointer_id,
                        node: node.clone(),
                        origin: window,
                        node_origin: graph_node.position,
                        current: graph_node.position,
                    };
                    self.select(Some(GraphSelection::Node(node)))
                }
                Some(GraphSelection::Port { node, port }) => {
                    self.interaction = GraphInteraction::Connection {
                        pointer_id,
                        source: GraphEndpoint::new(node.clone(), port.clone()),
                        current: local,
                    };
                    self.select(Some(GraphSelection::Port { node, port }))
                }
                Some(selection) => {
                    self.interaction = GraphInteraction::None;
                    self.select(Some(selection))
                }
                None => {
                    self.begin_pan(pointer_id, window);
                    self.select(None)
                }
            },
        }
    }

    pub fn pointer_move(
        &mut self,
        pointer_id: u64,
        local: GraphPoint,
        window: GraphPoint,
    ) -> Option<GraphCanvasEvent> {
        if self.disabled || !local.is_finite() || !window.is_finite() {
            return None;
        }
        match self.interaction.clone() {
            GraphInteraction::Pan {
                pointer_id: active,
                origin,
                viewport,
            } if active == pointer_id => {
                let dx = window.x - origin.x;
                let dy = window.y - origin.y;
                if dx * dx + dy * dy < PAN_THRESHOLD * PAN_THRESHOLD {
                    return None;
                }
                let next = viewport.pan_by(dx, dy);
                self.set_viewport(next);
                Some(GraphCanvasEvent::ViewportInput(next))
            }
            GraphInteraction::NodeDrag {
                pointer_id: active,
                node,
                origin,
                node_origin,
                ..
            } if active == pointer_id => {
                let next = GraphPoint::new(
                    node_origin.x + (window.x - origin.x) / self.viewport.zoom,
                    node_origin.y + (window.y - origin.y) / self.viewport.zoom,
                );
                if let GraphInteraction::NodeDrag { current, .. } = &mut self.interaction {
                    *current = next;
                }
                self.bump_revision();
                Some(GraphCanvasEvent::NodePositionInput {
                    node,
                    position: next,
                })
            }
            GraphInteraction::Connection {
                pointer_id: active, ..
            } if active == pointer_id => {
                if let GraphInteraction::Connection { current, .. } = &mut self.interaction {
                    *current = local;
                }
                self.set_hover(Some(local));
                self.bump_revision();
                None
            }
            _ => {
                self.set_hover(Some(local));
                None
            }
        }
    }

    pub fn pointer_release(
        &mut self,
        pointer_id: u64,
        local: GraphPoint,
        cancel: bool,
    ) -> Option<GraphCanvasEvent> {
        if self.interaction.pointer_id() != Some(pointer_id) {
            return None;
        }
        let interaction = std::mem::take(&mut self.interaction);
        if self.disabled {
            return None;
        }
        if cancel {
            if let GraphInteraction::Pan { viewport, .. } = interaction {
                self.set_viewport(viewport);
            }
            return None;
        }
        match interaction {
            GraphInteraction::Pan { .. } => {
                self.bump_revision();
                Some(GraphCanvasEvent::ViewportChanged(self.viewport))
            }
            GraphInteraction::NodeDrag { node, current, .. } => {
                self.bump_revision();
                Some(GraphCanvasEvent::NodePositionChanged {
                    node,
                    position: current,
                })
            }
            GraphInteraction::Connection { source, .. } => {
                self.bump_revision();
                let target = connection_drop_target(&self.model, &source, self.hit_test(local))?;
                let (source, target) = ordered_connection(&self.model, source, target)?;
                Some(GraphCanvasEvent::ConnectionRequested { source, target })
            }
            GraphInteraction::None => None,
        }
    }

    pub fn scroll(
        &mut self,
        anchor: GraphPoint,
        delta: GraphScrollDelta,
    ) -> Option<GraphCanvasEvent> {
        self.zoom(anchor, wheel_zoom_factor(delta))
    }

    pub fn zoom(&mut self, anchor: GraphPoint, factor: f32) -> Option<GraphCanvasEvent> {
        if self.disabled {
            return None;
        }
        let next = self.viewport.zoom_at(anchor, factor);
        if next == self.viewport {
            return None;
        }
        self.set_viewport(next);
        Some(GraphCanvasEvent::ViewportChanged(next))
    }

    pub fn adjust(
        &mut self,
        adjustment: GraphCanvasAdjustment,
        canvas_size: GraphSize,
    ) -> Option<GraphCanvasEvent> {
        if self.disabled {
            return None;
        }
        if matches!(adjustment, GraphCanvasAdjustment::ClearSelection) {
            return self.select(None);
        }
        let center = GraphPoint::new(canvas_size.width * 0.5, canvas_size.height * 0.5);
        let next = match adjustment {
            GraphCanvasAdjustment::PanLeft => self.viewport.pan_by(KEYBOARD_PAN_STEP, 0.0),
            GraphCanvasAdjustment::PanRight => self.viewport.pan_by(-KEYBOARD_PAN_STEP, 0.0),
            GraphCanvasAdjustment::PanUp => self.viewport.pan_by(0.0, KEYBOARD_PAN_STEP),
            GraphCanvasAdjustment::PanDown => self.viewport.pan_by(0.0, -KEYBOARD_PAN_STEP),
            GraphCanvasAdjustment::ZoomIn => self.viewport.zoom_at(center, KEYBOARD_ZOOM_FACTOR),
            GraphCanvasAdjustment::ZoomOut => {
                self.viewport.zoom_at(center, 1.0 / KEYBOARD_ZOOM_FACTOR)
            }
            GraphCanvasAdjustment::Fit => self
                .model
                .bounds()
                .map(|bounds| GraphViewport::fit(bounds, canvas_size, FIT_PADDING))
                .unwrap_or_default(),
            GraphCanvasAdjustment::ClearSelection => self.viewport,
        };
        if next == self.viewport {
            return None;
        }
        self.set_viewport(next);
        Some(GraphCanvasEvent::ViewportChanged(next))
    }

    fn begin_pan(&mut self, pointer_id: u64, origin: GraphPoint) {
        self.interaction = GraphInteraction::Pan {
            pointer_id,
            origin,
            viewport: self.viewport,
        };
        self.bump_revision();
    }

    fn select(&mut self, selection: Option<GraphSelection>) -> Option<GraphCanvasEvent> {
        self.set_selection(selection.clone());
        Some(GraphCanvasEvent::SelectionChanged(selection))
    }

    /// Identity for a host-registered Scene GPU painter.
    ///
    /// Default paint is [`GraphCanvasVisual`] (grid, frames, edges).
    /// [`ComponentView::project`] does **not** attach this node. Hosts that
    /// register `"graph-canvas"` may call this and `set_custom_render` themselves.
    /// The default Scene painter rejects the unregistered key.
    pub fn custom_render(&self) -> CustomRenderNode {
        CustomRenderNode::new(
            GRAPH_CANVAS_RENDERER,
            self.canvas_id.as_str(),
            self.revision,
        )
    }

    fn project_node_contents(
        &self,
        canvas: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        let model = self.displayed_model();
        for node in model.nodes() {
            let Some(content) = self.content_for(&node.id) else {
                continue;
            };
            let Some(child) = self.child_for(&node.id, &content) else {
                continue;
            };
            if world.node(child).is_none() {
                continue;
            }
            mutations.insert(canvas, child, None);
            if content.as_child().is_none() {
                let custom = content.as_custom_render();
                if world.custom_render(child) != custom.as_ref() {
                    mutations.set_custom_render(child, custom);
                }
                project_allocated_interaction(child, world, mutations);
            }
            let Some(rect) = content_rect_for(node, self.viewport) else {
                hide_content_child(child, world, mutations);
                continue;
            };
            position_content_child(child, rect, world, mutations);
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        if layout.width.is_none() {
            layout.width = Some(LengthSpec::Fill);
        }
        if layout.height.is_none() {
            layout.height = Some(LengthSpec::Fill);
        }
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        layout.position = PositionSpec::Relative;
        if style.background.is_none() {
            style.background = Some(SemanticColorRole::Background);
        }
        if self.disabled {
            style.foreground = Some(SemanticColorRole::Faint);
        }
        style
    }
}

impl ComponentView for GraphCanvas {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::GRAPH_CANVAS.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(crate::AppContext::assemble_graph_canvas_contents),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Positions, hides and sets interaction on the content children the application provides.
    const ALWAYS_REPROJECT: bool = true;

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "graph-canvas".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = GraphCanvasVisual {
            nodes: self.paint_nodes(),
            ports: self.paint_ports(),
            edges: self.paint_edges(),
            connecting: self.paint_connecting(),
            grid_spacing: self.grid_spacing,
            viewport_offset_x: self.viewport.offset.x,
            viewport_offset_y: self.viewport.offset.y,
            viewport_zoom: self.viewport.zoom,
        };
        if world.widget_visual_as::<GraphCanvasVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(
                    self.label
                        .clone()
                        .unwrap_or_else(|| Arc::from(DEFAULT_LABEL)),
                ),
                value: selection_value(self.selection.as_ref()),
                disabled: self.disabled,
                selected: Some(self.selection.is_some()),
                ..AccessibilityState::default()
            },
        );
        self.project_node_contents(id, world, mutations);
    }
}

impl crate::AppContext {
    pub fn is_graph_canvas(&self, id: StableNodeId) -> bool {
        self.read(crate::Entity::<GraphCanvas>::from_stable_id(id), |_| ())
            .is_ok()
    }

    pub fn begin_graph_canvas_pointer(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
        button: GraphPointerButton,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(entity) = self.graph_canvas_entity(target) else {
            return Ok(false);
        };
        if self.read(entity, |canvas| canvas.disabled)? {
            return Ok(false);
        }
        let Some(bounds) = self.world().component_layout_box(target) else {
            return Ok(false);
        };
        if !point_in_bounds(bounds, x, y) {
            return Ok(false);
        }
        self.update_component(entity, |canvas, cx| {
            cx.mutations().request_focus(document, Some(target));
            cx.mutations().capture_pointer(pointer_id, target);
            if let Some(event) = canvas.pointer_press(
                pointer_id,
                local_point(bounds, x, y),
                GraphPoint::new(x, y),
                button,
            ) {
                cx.emit(event);
            }
            true
        })
    }

    pub fn update_graph_canvas_pointer(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(entity) = self.graph_canvas_entity(target) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().component_layout_box(target) else {
            return Ok(false);
        };
        self.update_component(entity, |canvas, cx| {
            if let Some(event) =
                canvas.pointer_move(pointer_id, local_point(bounds, x, y), GraphPoint::new(x, y))
            {
                cx.emit(event);
            }
            canvas.interaction.pointer_id() == Some(pointer_id)
        })
    }

    pub fn end_graph_canvas_pointer(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        cancel: bool,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(entity) = self.graph_canvas_entity(target) else {
            return Ok(false);
        };
        let bounds = self.world().component_layout_box(target);
        let local = bounds
            .map(|bounds| local_point(bounds, x, y))
            .unwrap_or(GraphPoint::new(x, y));
        self.update_component(entity, |canvas, cx| {
            if let Some(event) = canvas.pointer_release(pointer_id, local, cancel) {
                cx.emit(event);
            }
            cx.mutations().release_pointer(pointer_id, target);
            true
        })
    }

    pub fn hover_graph_canvas(
        &mut self,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(entity) = self.graph_canvas_entity(target) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().component_layout_box(target) else {
            return Ok(false);
        };
        if !point_in_bounds(bounds, x, y) {
            return self.update_component(entity, |canvas, _| canvas.set_hover(None));
        }
        self.update_component(entity, |canvas, _| {
            canvas.set_hover(Some(local_point(bounds, x, y)))
        })
    }

    /// Resolve a window point against a graph canvas: the canvas-local point
    /// and what sits under it. `None` when `target` is not a graph canvas, has
    /// no layout box, or the point falls outside its bounds.
    ///
    /// This is how a host turns the window coordinates carried by
    /// [`crate::SecondaryPress`] into a menu decision. Pure query: it does not
    /// select, focus or hover, and it ignores `disabled` the way
    /// [`Self::hover_graph_canvas`] does — geometry stays geometry, and what a
    /// hit means is the application's call.
    pub fn graph_canvas_hit_at(
        &self,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<GraphCanvasHit> {
        let entity = self.graph_canvas_entity(target)?;
        let bounds = self.world().component_layout_box(target)?;
        if !point_in_bounds(bounds, x, y) {
            return None;
        }
        let local = local_point(bounds, x, y);
        let selection = self.read(entity, |canvas| canvas.hit_test(local)).ok()?;
        Some(GraphCanvasHit { local, selection })
    }

    pub fn clear_graph_canvas_hover(
        &mut self,
        target: StableNodeId,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(entity) = self.graph_canvas_entity(target) else {
            return Ok(false);
        };
        self.update_component(entity, |canvas, _| canvas.set_hover(None))
    }

    pub fn scroll_graph_canvas(
        &mut self,
        document: crate::DocumentId,
        target: StableNodeId,
        x: f32,
        y: f32,
        delta: GraphScrollDelta,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(entity) = self.graph_canvas_entity(target) else {
            return Ok(false);
        };
        if self.read(entity, |canvas| canvas.disabled)? {
            return Ok(false);
        }
        let Some(bounds) = self.world().component_layout_box(target) else {
            return Ok(false);
        };
        if !point_in_bounds(bounds, x, y) {
            return Ok(false);
        }
        self.update_component(entity, |canvas, cx| {
            cx.mutations().request_focus(document, Some(target));
            if let Some(event) = canvas.scroll(local_point(bounds, x, y), delta) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn adjust_focused_graph_canvas(
        &mut self,
        document: crate::DocumentId,
        adjustment: GraphCanvasAdjustment,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        let Some(entity) = self.graph_canvas_entity(target) else {
            return Ok(false);
        };
        if self.read(entity, |canvas| canvas.disabled)? {
            return Ok(false);
        }
        let canvas_size = match self.world().component_layout_box(target) {
            Some(bounds) => GraphSize::new(bounds.width, bounds.height),
            None if matches!(
                adjustment,
                GraphCanvasAdjustment::ZoomIn
                    | GraphCanvasAdjustment::ZoomOut
                    | GraphCanvasAdjustment::Fit
            ) =>
            {
                return Ok(false);
            }
            None => GraphSize::default(),
        };
        self.update_component(entity, |canvas, cx| {
            if let Some(event) = canvas.adjust(adjustment, canvas_size) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    /// Allocate a first-class child per HostTexture / CustomRender node interior.
    ///
    /// Region children are host-owned and only reparented. Allocated children
    /// sample [`HOST_TEXTURE_RENDERER`] in document order. The canvas itself
    /// keeps [`GraphCanvasVisual`] and does not attach GPU content.
    pub fn assemble_graph_canvas_contents(
        &mut self,
        canvas: crate::Entity<GraphCanvas>,
    ) -> Result<bool, crate::FrameworkError> {
        let parent = canvas.stable_id();
        let document = self
            .world()
            .node(parent)
            .map(|node| node.document)
            .ok_or(crate::FrameworkError::MissingView(parent))?;
        let (wanted, stored) = self.read(canvas, |canvas| {
            let mut wanted = Vec::new();
            for node in canvas.displayed_model().nodes() {
                let Some(content) = canvas.content_for(&node.id) else {
                    continue;
                };
                if content.as_child().is_some() {
                    continue;
                }
                if content.as_custom_render().is_none() {
                    continue;
                }
                wanted.push(node.id.clone());
            }
            (wanted, canvas.allocated_contents.clone())
        })?;
        let mut existing: Vec<(GraphNodeId, StableNodeId)> = stored
            .into_iter()
            .filter(|(_, id)| self.world().contains(*id))
            .collect();
        let mut next = Vec::with_capacity(wanted.len());
        let mut used = HashSet::new();
        for node in wanted {
            if let Some((_, id)) = existing.iter().find(|(existing, _)| *existing == node) {
                next.push((node.clone(), *id));
                used.insert(*id);
            } else {
                let id = self
                    .create_view(
                        document,
                        NodeKind::Element {
                            tag: GRAPH_NODE_CONTENT_TAG.into(),
                        },
                        (),
                    )?
                    .stable_id();
                next.push((node, id));
                used.insert(id);
            }
        }
        for (_, id) in existing.drain(..) {
            if !used.contains(&id) {
                drop_allocated_content(self, id)?;
            }
        }
        let mut mutations = MutationQueue::new();
        for (_, id) in &next {
            mutations.insert(parent, *id, None);
        }
        if !mutations.is_empty() {
            self.commit_mutations(mutations)?;
        }
        self.update_component(canvas, |canvas, _| {
            canvas.allocated_contents = next;
        })?;
        Ok(true)
    }

    fn graph_canvas_entity(&self, id: StableNodeId) -> Option<crate::Entity<GraphCanvas>> {
        self.is_graph_canvas(id)
            .then(|| crate::Entity::from_stable_id(id))
    }
}

pub fn wheel_zoom_factor(delta: GraphScrollDelta) -> f32 {
    match delta {
        GraphScrollDelta::Lines { y } => KEYBOARD_ZOOM_FACTOR.powf(y),
        GraphScrollDelta::Pixels { y } => (y * 0.0025).exp(),
    }
}

fn hit_port_disc(
    model: &GraphModel,
    viewport: GraphViewport,
    view_position: GraphPoint,
) -> Option<GraphSelection> {
    let grab = PORT_GRAB_RADIUS * PORT_GRAB_RADIUS;
    for node in model.nodes().iter().rev() {
        for port in node.ports.iter().rev() {
            let Some(world) =
                model.port_position(&GraphEndpoint::new(node.id.clone(), port.id.clone()))
            else {
                continue;
            };
            let view = viewport.world_to_view(world);
            if view.distance_squared(view_position) <= grab {
                return Some(GraphSelection::Port {
                    node: node.id.clone(),
                    port: port.id.clone(),
                });
            }
        }
    }
    None
}

fn connection_drop_target(
    model: &GraphModel,
    source: &GraphEndpoint,
    hit: Option<GraphSelection>,
) -> Option<GraphEndpoint> {
    match hit? {
        GraphSelection::Port { node, port } => Some(GraphEndpoint::new(node, port)),
        GraphSelection::Node(node) => {
            let graph_node = model.node(&node)?;
            graph_node.ports.iter().find_map(|port| {
                let candidate = GraphEndpoint::new(node.clone(), port.id.clone());
                ordered_connection(model, source.clone(), candidate.clone()).map(|_| candidate)
            })
        }
        GraphSelection::Edge(_) => None,
    }
}

fn ordered_connection(
    model: &GraphModel,
    source: GraphEndpoint,
    target: GraphEndpoint,
) -> Option<(GraphEndpoint, GraphEndpoint)> {
    if source == target {
        return None;
    }
    let source_out = port_accepts_output(model, &source.node, &source.port);
    let source_in = port_accepts_input(model, &source.node, &source.port);
    let target_out = port_accepts_output(model, &target.node, &target.port);
    let target_in = port_accepts_input(model, &target.node, &target.port);
    if source_out && target_in {
        Some((source, target))
    } else if source_in && target_out {
        Some((target, source))
    } else {
        None
    }
}

fn canvas_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Fill),
            overflow_x: OverflowSpec::Hidden,
            overflow_y: OverflowSpec::Hidden,
            position: PositionSpec::Relative,
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(SemanticColorRole::Background),
        interaction: InteractionStyle {
            focused: SemanticPaint::FOCUS_BORDER,
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                background: Some(SemanticColorRole::Subtle),
                ..SemanticPaint::default()
            },
            ..InteractionStyle::default()
        }
        .into(),
        ..NodeStyle::default()
    }
}

fn content_rect_for(node: &nana_ui_core::GraphNode, viewport: GraphViewport) -> Option<LayoutBox> {
    let rect = viewport.world_rect_to_view(node.bounds());
    if !rect.origin.x.is_finite()
        || !rect.origin.y.is_finite()
        || !rect.size.width.is_finite()
        || !rect.size.height.is_finite()
    {
        return None;
    }
    let title_height = (GRAPH_NODE_TITLE_HEIGHT * viewport.zoom).clamp(18.0, 34.0);
    let pad = GRAPH_PORT_INSET * viewport.zoom;
    let width = (rect.size.width - pad * 2.0).max(0.0);
    let height = (rect.size.height - title_height - pad).max(0.0);
    if width < 1.0 || height < 1.0 {
        return None;
    }
    Some(LayoutBox {
        x: rect.origin.x + pad,
        y: rect.origin.y + title_height,
        width,
        height,
    })
}

fn position_content_child(
    id: StableNodeId,
    rect: LayoutBox,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    let mut style = world.node_style(id).cloned().unwrap_or_default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Absolute;
    layout.offset_left = Some(LengthSpec::Px(rect.x));
    layout.offset_top = Some(LengthSpec::Px(rect.y));
    layout.offset_right = None;
    layout.offset_bottom = None;
    layout.width = Some(LengthSpec::Px(rect.width));
    layout.height = Some(LengthSpec::Px(rect.height));
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    layout.hidden = false;
    if world.node_style(id) != Some(&style) {
        mutations.set_style(id, style);
    }
}

fn hide_content_child(id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
    let mut style = world.node_style(id).cloned().unwrap_or_default();
    let layout = Arc::make_mut(&mut style.layout);
    if layout.hidden {
        return;
    }
    layout.hidden = true;
    if world.node_style(id) != Some(&style) {
        mutations.set_style(id, style);
    }
}

fn project_allocated_interaction(id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
    let interaction = InteractionState {
        pointer_events: false,
        focusable: false,
    };
    if world.interaction(id) != Some(interaction) {
        mutations.set_interaction(id, interaction);
    }
}

fn drop_allocated_content(
    context: &mut crate::AppContext,
    id: StableNodeId,
) -> Result<(), crate::FrameworkError> {
    if context.world().contains(id) {
        let mut mutations = MutationQueue::new();
        mutations.despawn_subtree(id);
        context.commit_mutations(mutations)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn graph_node_slot_name(name: &str, model: &GraphModel) -> Option<GraphNodeId> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = GraphNodeId::new(trimmed);
    if model.node(&candidate).is_some() {
        return Some(candidate);
    }
    for prefix in [
        "node:", "node.", "node-", "content:", "content.", "content-",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let candidate = GraphNodeId::new(rest);
            if model.node(&candidate).is_some() {
                return Some(candidate);
            }
        }
    }
    None
}

fn sanitize_canvas_id(id: GraphCanvasId) -> GraphCanvasId {
    if id.as_str().is_empty() {
        GraphCanvasId::new(DEFAULT_CANVAS_ID)
    } else {
        id
    }
}

fn port_kind(
    model: &GraphModel,
    node_id: &GraphNodeId,
    port_id: &GraphPortId,
) -> Option<GraphPortKind> {
    model
        .node(node_id)?
        .ports
        .iter()
        .find(|port| &port.id == port_id)
        .map(|port| port.kind)
}

fn port_accepts_output(model: &GraphModel, node: &GraphNodeId, port: &GraphPortId) -> bool {
    port_kind(model, node, port)
        .is_some_and(|kind| matches!(kind, GraphPortKind::Output | GraphPortKind::Bidirectional))
}

fn port_accepts_input(model: &GraphModel, node: &GraphNodeId, port: &GraphPortId) -> bool {
    port_kind(model, node, port)
        .is_some_and(|kind| matches!(kind, GraphPortKind::Input | GraphPortKind::Bidirectional))
}

fn local_point(bounds: LayoutBox, x: f32, y: f32) -> GraphPoint {
    GraphPoint::new(x - bounds.x, y - bounds.y)
}

fn point_in_bounds(bounds: LayoutBox, x: f32, y: f32) -> bool {
    x >= bounds.x && y >= bounds.y && x <= bounds.x + bounds.width && y <= bounds.y + bounds.height
}

fn pending_connection_paint(
    start: GraphPoint,
    end: GraphPoint,
    source_side: GraphPortSide,
) -> GraphEdgePaint {
    let reach = ((end.x - start.x).abs().max((end.y - start.y).abs()) * 0.45).clamp(32.0, 180.0);
    let out = port_tangent(source_side);
    let incoming = GraphPoint::new(-out.x, -out.y);
    GraphEdgePaint {
        curve: [
            start,
            GraphPoint::new(start.x + out.x * reach, start.y + out.y * reach),
            GraphPoint::new(end.x + incoming.x * reach, end.y + incoming.y * reach),
            end,
        ],
        selected: false,
        hovered: false,
        connecting: true,
        label: None,
    }
}

fn selection_value(selection: Option<&GraphSelection>) -> Option<Arc<str>> {
    Some(Arc::from(match selection? {
        GraphSelection::Node(node) => format!("node {node}"),
        GraphSelection::Port { node, port } => format!("port {node}/{port}"),
        GraphSelection::Edge(edge) => format!("edge {edge}"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, MutationQueue};
    use std::sync::{Arc, Mutex};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample_graph() -> GraphModel {
        let source = GraphNode::new(
            "source",
            "Source",
            GraphPoint::new(10.0, 20.0),
            GraphSize::new(120.0, 80.0),
        )
        .with_port(GraphPort::new(
            "out",
            "Output",
            GraphPortKind::Output,
            GraphPortSide::Right,
        ));
        let target = GraphNode::new(
            "target",
            "Target",
            GraphPoint::new(250.0, 20.0),
            GraphSize::new(120.0, 80.0),
        )
        .with_port(GraphPort::new(
            "in",
            "Input",
            GraphPortKind::Input,
            GraphPortSide::Left,
        ));
        GraphModel::new(
            vec![source, target],
            vec![GraphEdge::new(
                "flow",
                GraphEndpoint::new("source", "out"),
                GraphEndpoint::new("target", "in"),
            )],
        )
        .expect("valid graph")
    }

    fn layout(context: &mut AppContext, id: crate::StableNodeId) {
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
    }

    fn collect_events(
        context: &mut AppContext,
        canvas: crate::Entity<GraphCanvas>,
    ) -> Arc<Mutex<Vec<GraphCanvasEvent>>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        context
            .on(canvas, move |_canvas, event: &GraphCanvasEvent, _cx| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();
        events
    }

    #[test]
    fn drag_node_emits_input_then_changed() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap();
        layout(&mut context, canvas.stable_id());
        let events = collect_events(&mut context, canvas);

        assert!(
            context
                .begin_graph_canvas_pointer(
                    document(),
                    1,
                    canvas.stable_id(),
                    70.0,
                    60.0,
                    GraphPointerButton::Primary,
                )
                .unwrap()
        );
        assert!(
            context
                .update_graph_canvas_pointer(document(), 1, 90.0, 70.0)
                .unwrap()
        );
        assert!(
            context
                .end_graph_canvas_pointer(document(), 1, 90.0, 70.0, false)
                .unwrap()
        );

        assert_eq!(
            *events.lock().unwrap(),
            [
                GraphCanvasEvent::SelectionChanged(Some(GraphSelection::Node("source".into()))),
                GraphCanvasEvent::NodePositionInput {
                    node: "source".into(),
                    position: GraphPoint::new(30.0, 30.0),
                },
                GraphCanvasEvent::NodePositionChanged {
                    node: "source".into(),
                    position: GraphPoint::new(30.0, 30.0),
                },
            ]
        );
        assert!(context.world().pointer_capture(document(), 1).is_none());
    }

    #[test]
    fn pan_updates_viewport() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap();
        layout(&mut context, canvas.stable_id());
        let events = collect_events(&mut context, canvas);

        assert!(
            context
                .begin_graph_canvas_pointer(
                    document(),
                    2,
                    canvas.stable_id(),
                    5.0,
                    5.0,
                    GraphPointerButton::Primary,
                )
                .unwrap()
        );
        assert!(
            context
                .update_graph_canvas_pointer(document(), 2, 25.0, 15.0)
                .unwrap()
        );
        let panned = GraphViewport::new(GraphPoint::new(20.0, 10.0), 1.0);
        assert_eq!(
            context.read(canvas, |canvas| canvas.viewport).unwrap(),
            panned
        );
        assert!(
            context
                .end_graph_canvas_pointer(document(), 2, 25.0, 15.0, false)
                .unwrap()
        );
        assert_eq!(
            context.read(canvas, |canvas| canvas.viewport).unwrap(),
            panned
        );
        assert_eq!(
            *events.lock().unwrap(),
            [
                GraphCanvasEvent::SelectionChanged(None),
                GraphCanvasEvent::ViewportInput(panned),
                GraphCanvasEvent::ViewportChanged(panned),
            ]
        );
    }

    #[test]
    fn zoom_is_clamped() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap();
        layout(&mut context, canvas.stable_id());
        assert!(
            context
                .scroll_graph_canvas(
                    document(),
                    canvas.stable_id(),
                    40.0,
                    40.0,
                    GraphScrollDelta::Lines { y: 80.0 },
                )
                .unwrap()
        );
        assert_eq!(
            context.read(canvas, |canvas| canvas.viewport.zoom).unwrap(),
            GRAPH_MAX_ZOOM
        );
        assert!(
            !context
                .scroll_graph_canvas(
                    document(),
                    canvas.stable_id(),
                    40.0,
                    40.0,
                    GraphScrollDelta::Lines { y: 80.0 },
                )
                .unwrap()
        );
    }

    #[test]
    fn connection_request_requires_valid_endpoints() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap();
        layout(&mut context, canvas.stable_id());
        let events = collect_events(&mut context, canvas);

        assert!(
            context
                .begin_graph_canvas_pointer(
                    document(),
                    3,
                    canvas.stable_id(),
                    130.0,
                    60.0,
                    GraphPointerButton::Primary,
                )
                .unwrap()
        );
        assert!(
            context
                .update_graph_canvas_pointer(document(), 3, 5.0, 5.0)
                .unwrap()
        );
        assert!(
            context
                .end_graph_canvas_pointer(document(), 3, 5.0, 5.0, false)
                .unwrap()
        );
        assert!(
            !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, GraphCanvasEvent::ConnectionRequested { .. }))
        );

        assert!(
            context
                .begin_graph_canvas_pointer(
                    document(),
                    4,
                    canvas.stable_id(),
                    250.0,
                    60.0,
                    GraphPointerButton::Primary,
                )
                .unwrap()
        );
        assert!(
            context
                .end_graph_canvas_pointer(document(), 4, 250.0, 60.0, false)
                .unwrap()
        );
        assert!(
            !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, GraphCanvasEvent::ConnectionRequested { .. }))
        );

        assert!(
            context
                .begin_graph_canvas_pointer(
                    document(),
                    5,
                    canvas.stable_id(),
                    130.0,
                    60.0,
                    GraphPointerButton::Primary,
                )
                .unwrap()
        );
        assert!(
            context
                .update_graph_canvas_pointer(document(), 5, 250.0, 60.0)
                .unwrap()
        );
        assert!(
            context
                .end_graph_canvas_pointer(document(), 5, 250.0, 60.0, false)
                .unwrap()
        );
        let requested = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                GraphCanvasEvent::ConnectionRequested { source, target } => {
                    Some((source.clone(), target.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            requested,
            [(
                GraphEndpoint::new("source", "out"),
                GraphEndpoint::new("target", "in"),
            )]
        );

        assert!(
            context
                .begin_graph_canvas_pointer(
                    document(),
                    6,
                    canvas.stable_id(),
                    250.0,
                    60.0,
                    GraphPointerButton::Primary,
                )
                .unwrap()
        );
        assert!(
            context
                .end_graph_canvas_pointer(document(), 6, 130.0, 60.0, false)
                .unwrap()
        );
        let requested = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                GraphCanvasEvent::ConnectionRequested { source, target } => {
                    Some((source.clone(), target.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            requested.last(),
            Some(&(
                GraphEndpoint::new("source", "out"),
                GraphEndpoint::new("target", "in"),
            ))
        );
    }

    #[test]
    fn keyboard_pan_requires_focus() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap();
        layout(&mut context, canvas.stable_id());
        let events = collect_events(&mut context, canvas);

        assert!(
            !context
                .adjust_focused_graph_canvas(document(), GraphCanvasAdjustment::PanLeft)
                .unwrap()
        );
        assert_eq!(
            context.read(canvas, |canvas| canvas.viewport).unwrap(),
            GraphViewport::default()
        );
        assert!(events.lock().unwrap().is_empty());

        assert!(context.focus_node(document(), canvas.stable_id()).unwrap());
        assert!(
            context
                .adjust_focused_graph_canvas(document(), GraphCanvasAdjustment::PanLeft)
                .unwrap()
        );
        assert_eq!(
            context.read(canvas, |canvas| canvas.viewport).unwrap(),
            GraphViewport::new(GraphPoint::new(KEYBOARD_PAN_STEP, 0.0), 1.0)
        );
        assert_eq!(
            *events.lock().unwrap(),
            [GraphCanvasEvent::ViewportChanged(GraphViewport::new(
                GraphPoint::new(KEYBOARD_PAN_STEP, 0.0),
                1.0
            ))]
        );
    }

    #[test]
    fn projects_fill_layout_and_accessibility_without_unregistered_gpu() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(
                document(),
                GraphCanvas::new("workspace/main", sample_graph()).label("Workflow"),
            )
            .unwrap();
        let style = context.world().node_style(canvas.stable_id()).unwrap();
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(style.layout.height, Some(LengthSpec::Fill));
        // Default Scene painter errors on unregistered custom renderers. Keep
        // layout/a11y only unless the host registers SceneGpuRenderer
        // "graph-canvas" and attaches GraphCanvas::custom_render().
        assert!(context.world().custom_render(canvas.stable_id()).is_none());
        let render = context.read(canvas, GraphCanvas::custom_render).unwrap();
        assert_eq!(render.renderer.as_ref(), GRAPH_CANVAS_RENDERER);
        assert_eq!(render.resource.as_ref(), "workspace/main");
        let descriptors = context
            .read(canvas, |canvas| {
                canvas.target_descriptors(GraphSize::new(400.0, 300.0))
            })
            .unwrap();
        assert!(
            descriptors
                .iter()
                .any(|target| target.id.as_str() == "graph.workspace%2Fmain.node.source.port.out")
        );
        let accessibility = context.world().accessibility(canvas.stable_id()).unwrap();
        assert_eq!(accessibility.label.as_deref(), Some("Workflow"));
        assert!(matches!(
            context.world().widget_visual_as::<GraphCanvasVisual>(canvas.stable_id()),
            Some(GraphCanvasVisual {
                nodes,
                ports,
                edges,
                connecting: None,
                ..
            }) if nodes.len() == 2 && ports.len() == 2 && edges.len() == 1
        ));
    }

    #[test]
    fn canvas_style_clips_overflow() {
        let mut context = AppContext::new();
        let id = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap()
            .stable_id();
        let layout = &context.world().node_style(id).unwrap().layout;
        assert_eq!(layout.overflow_x, OverflowSpec::Hidden);
        assert_eq!(layout.overflow_y, OverflowSpec::Hidden);
    }

    #[test]
    fn port_disc_is_preferred_over_the_node_body() {
        let canvas = GraphCanvas::new("gallery", sample_graph());
        let port = canvas.viewport.world_to_view(
            canvas
                .model
                .port_position(&GraphEndpoint::new("source", "out"))
                .unwrap(),
        );
        assert_eq!(
            canvas.hit_test(port),
            Some(GraphSelection::Port {
                node: "source".into(),
                port: "out".into(),
            })
        );
        assert_eq!(
            canvas.hit_test(GraphPoint::new(70.0, 60.0)),
            Some(GraphSelection::Node("source".into()))
        );
    }

    #[test]
    fn clicking_an_edge_selects_it_without_starting_a_connection() {
        let mut canvas = GraphCanvas::new("gallery", sample_graph());
        let curve = canvas
            .model
            .edge_curve(&canvas.model.edges()[0], canvas.viewport)
            .unwrap();
        let midpoint = nana_ui_core::cubic_point(curve, 0.5);
        assert_eq!(
            canvas.hit_test(midpoint),
            Some(GraphSelection::Edge("flow".into()))
        );
        let event = canvas.pointer_press(1, midpoint, midpoint, GraphPointerButton::Primary);
        assert_eq!(
            event,
            Some(GraphCanvasEvent::SelectionChanged(Some(
                GraphSelection::Edge("flow".into())
            )))
        );
        assert_eq!(canvas.interaction, GraphInteraction::None);
        assert!(canvas.paint_edges().iter().any(|edge| edge.selected));
        assert!(canvas.paint_connecting().is_none());
    }

    #[test]
    fn nodes_that_leave_the_canvas_are_clipped_in_geometry() {
        let mut context = AppContext::new();
        let mut canvas = GraphCanvas::new("gallery", sample_graph());
        canvas.viewport = GraphViewport::new(GraphPoint::new(-80.0, 0.0), 1.0);
        let id = context
            .create_component(document(), canvas)
            .unwrap()
            .stable_id();
        layout(&mut context, id);
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        context.resolve_styles(&[id]).unwrap();
        let layout = context
            .world()
            .graph_canvas_layout(id)
            .expect("graph layout");
        let nodes = layout.nodes;
        assert!(
            nodes.iter().all(|(bounds, _, _, _)| {
                bounds.x >= 0.0 && bounds.x + bounds.width <= 400.0 + 0.01
            }),
            "node quads must be intersected with the canvas, not merely culled"
        );
        assert!(
            nodes.iter().any(|(bounds, _, _, _)| bounds.x == 0.0),
            "the source node starts at x=-70 after the pan and must be cut at the canvas edge"
        );
    }

    #[test]
    fn pending_connection_leaves_output_to_the_right() {
        let start = GraphPoint::new(10.0, 20.0);
        let end = GraphPoint::new(80.0, 40.0);
        let paint = pending_connection_paint(start, end, GraphPortSide::Right);
        assert!(paint.curve[1].x > paint.curve[0].x);
        assert!(paint.curve[2].x < paint.curve[3].x);
    }

    #[test]
    fn paint_uses_curves_port_labels_and_grid_params() {
        let canvas = GraphCanvas::new("gallery", sample_graph());
        let edges = canvas.paint_edges();
        assert_eq!(edges.len(), 1);
        assert_ne!(edges[0].curve[0], edges[0].curve[1]);
        let ports = canvas.paint_ports();
        assert!(ports.iter().any(|port| port.label.as_ref() == "Output"));
        assert!(ports.iter().all(|port| port.radius <= PORT_RADIUS_ACTIVE));
        let visual = {
            let mut context = AppContext::new();
            let id = context
                .create_component(document(), canvas)
                .unwrap()
                .stable_id();
            context
                .world()
                .widget_visual_as::<GraphCanvasVisual>(id)
                .unwrap()
                .clone()
        };
        let GraphCanvasVisual {
            grid_spacing,
            viewport_zoom,
            ..
        } = visual;
        assert_eq!(grid_spacing, DEFAULT_GRID_SPACING);
        assert_eq!(viewport_zoom, 1.0);
    }

    #[test]
    fn region_content_is_positioned_inside_the_node_and_default_paint_stays() {
        let mut context = AppContext::new();
        let child = context
            .create_view(
                document(),
                NodeKind::Element {
                    tag: "nana.host-texture".into(),
                },
                (),
            )
            .unwrap();
        let canvas = context
            .create_component(
                document(),
                GraphCanvas::new("gallery", sample_graph())
                    .node_content("source", GraphNodeContent::region(child.stable_id())),
            )
            .unwrap();
        let rect = context
            .read(canvas, |canvas| canvas.node_content_rect(&"source".into()))
            .unwrap()
            .expect("source body");
        let style = context.world().node_style(child.stable_id()).unwrap();
        assert_eq!(style.layout.position, PositionSpec::Absolute);
        assert_eq!(style.layout.offset_left, Some(LengthSpec::Px(rect.x)));
        assert_eq!(style.layout.offset_top, Some(LengthSpec::Px(rect.y)));
        assert_eq!(style.layout.width, Some(LengthSpec::Px(rect.width)));
        assert_eq!(style.layout.height, Some(LengthSpec::Px(rect.height)));
        assert_eq!(
            context.world().node(canvas.stable_id()).unwrap().children,
            vec![child.stable_id()]
        );
        assert!(context.world().custom_render(canvas.stable_id()).is_none());
        assert!(matches!(
            context.world().widget_visual_as::<GraphCanvasVisual>(canvas.stable_id()),
            Some(GraphCanvasVisual {
                nodes,
                ports,
                edges,
                connecting: None,
                ..
            }) if nodes.len() == 2 && ports.len() == 2 && edges.len() == 1
        ));
    }

    #[test]
    fn host_texture_assemble_samples_in_document_order_not_on_the_canvas() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(
                document(),
                GraphCanvas::new("gallery", sample_graph())
                    .node_content("source", GraphNodeContent::host_texture("formula.source")),
            )
            .unwrap();
        assert!(
            context
                .world()
                .node(canvas.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
        assert!(context.assemble_graph_canvas_contents(canvas).unwrap());
        let children = context
            .world()
            .node(canvas.stable_id())
            .unwrap()
            .children
            .clone();
        assert_eq!(children.len(), 1);
        let custom = context.world().custom_render(children[0]).unwrap();
        assert_eq!(custom.renderer.as_ref(), HOST_TEXTURE_RENDERER);
        assert_eq!(custom.renderer.as_ref(), "nana.host-texture");
        assert_eq!(custom.resource.as_ref(), "formula.source");
        assert!(context.world().custom_render(canvas.stable_id()).is_none());
        assert!(matches!(
            context.world().widget_visual_as::<GraphCanvasVisual>(canvas.stable_id()),
            Some(GraphCanvasVisual {
                nodes,
                edges,
                ..
            }) if nodes.len() == 2 && edges.len() == 1
        ));
        let rect = context
            .read(canvas, |canvas| canvas.node_content_rect(&"source".into()))
            .unwrap()
            .expect("source body");
        let style = context.world().node_style(children[0]).unwrap();
        assert_eq!(style.layout.position, PositionSpec::Absolute);
        assert_eq!(style.layout.offset_left, Some(LengthSpec::Px(rect.x)));
        assert!(style.layout.offset_top.is_some());
        assert_eq!(
            context.world().interaction(children[0]),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
    }

    #[test]
    fn unslotted_nodes_do_not_allocate_content_children() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap();
        assert!(context.assemble_graph_canvas_contents(canvas).unwrap());
        assert!(
            context
                .world()
                .node(canvas.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
        assert!(matches!(
            context.world().widget_visual_as::<GraphCanvasVisual>(canvas.stable_id()),
            Some(GraphCanvasVisual { nodes, .. }) if nodes.len() == 2
        ));
    }

    #[test]
    fn hit_at_translates_window_points_into_canvas_local_space() {
        let mut context = AppContext::new();
        let canvas = context
            .create_component(document(), GraphCanvas::new("gallery", sample_graph()))
            .unwrap();
        // A canvas that does not start at the window origin is the case a host
        // cannot resolve on its own without copying the canvas transform.
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            canvas.stable_id(),
            LayoutBox {
                x: 40.0,
                y: 25.0,
                width: 400.0,
                height: 300.0,
            },
        );
        context.commit_mutations(mutations).unwrap();

        let on_node = context
            .graph_canvas_hit_at(canvas.stable_id(), 110.0, 85.0)
            .expect("inside the canvas");
        assert_eq!(on_node.local, GraphPoint::new(70.0, 60.0));
        assert_eq!(
            on_node.selection,
            Some(GraphSelection::Node("source".into()))
        );

        let on_background = context
            .graph_canvas_hit_at(canvas.stable_id(), 240.0, 225.0)
            .expect("inside the canvas");
        assert_eq!(on_background.local, GraphPoint::new(200.0, 200.0));
        assert_eq!(on_background.selection, None);

        assert!(
            context
                .graph_canvas_hit_at(canvas.stable_id(), 10.0, 10.0)
                .is_none()
        );
    }
}
