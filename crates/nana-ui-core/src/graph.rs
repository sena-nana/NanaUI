//! Application-owned graph model, viewport, hit testing, and target descriptors.
//!
//! This crate is backend-neutral. Runtime consumes these types; do not define
//! a second graph model.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const GRAPH_MIN_ZOOM: f32 = 0.1;
pub const GRAPH_MAX_ZOOM: f32 = 4.0;
pub const GRAPH_PORT_HIT_RADIUS: f32 = 8.0;
pub const GRAPH_EDGE_HIT_TOLERANCE: f32 = 6.0;
pub const GRAPH_NODE_TITLE_HEIGHT: f32 = 28.0;
pub const GRAPH_PORT_PITCH: f32 = 22.0;
pub const GRAPH_PORT_INSET: f32 = 10.0;

macro_rules! graph_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

graph_id!(GraphCanvasId);
graph_id!(GraphNodeId);
graph_id!(GraphPortId);
graph_id!(GraphEdgeId);
graph_id!(GraphTargetId);

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphPoint {
    pub x: f32,
    pub y: f32,
}

impl GraphPoint {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub fn distance_squared(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphSize {
    pub width: f32,
    pub height: f32,
}

impl GraphSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn is_valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphRect {
    pub origin: GraphPoint,
    pub size: GraphSize,
}

impl GraphRect {
    pub const fn new(origin: GraphPoint, size: GraphSize) -> Self {
        Self { origin, size }
    }

    pub fn from_points(first: GraphPoint, second: GraphPoint) -> Self {
        let min_x = first.x.min(second.x);
        let min_y = first.y.min(second.y);
        let max_x = first.x.max(second.x);
        let max_y = first.y.max(second.y);
        Self::new(
            GraphPoint::new(min_x, min_y),
            GraphSize::new(max_x - min_x, max_y - min_y),
        )
    }

    pub fn max_x(self) -> f32 {
        self.origin.x + self.size.width
    }

    pub fn max_y(self) -> f32 {
        self.origin.y + self.size.height
    }

    pub fn contains(self, point: GraphPoint) -> bool {
        point.x >= self.origin.x
            && point.y >= self.origin.y
            && point.x <= self.max_x()
            && point.y <= self.max_y()
    }

    pub fn expand(self, amount: f32) -> Self {
        let amount = amount.max(0.0);
        Self::new(
            GraphPoint::new(self.origin.x - amount, self.origin.y - amount),
            GraphSize::new(
                self.size.width + amount * 2.0,
                self.size.height + amount * 2.0,
            ),
        )
    }

    fn union(self, other: Self) -> Self {
        Self::from_points(
            GraphPoint::new(
                self.origin.x.min(other.origin.x),
                self.origin.y.min(other.origin.y),
            ),
            GraphPoint::new(
                self.max_x().max(other.max_x()),
                self.max_y().max(other.max_y()),
            ),
        )
    }
}

/// A viewport maps graph-space coordinates into local canvas pixels.
///
/// `offset` is expressed in local pixels. Keeping it independent from zoom
/// makes pointer panning and persistence deterministic across consumers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphViewport {
    pub offset: GraphPoint,
    pub zoom: f32,
}

impl Default for GraphViewport {
    fn default() -> Self {
        Self {
            offset: GraphPoint::ZERO,
            zoom: 1.0,
        }
    }
}

impl GraphViewport {
    pub fn new(offset: GraphPoint, zoom: f32) -> Self {
        Self {
            offset: if offset.is_finite() {
                offset
            } else {
                GraphPoint::ZERO
            },
            zoom: clamp_zoom(zoom),
        }
    }

    pub fn world_to_view(self, point: GraphPoint) -> GraphPoint {
        GraphPoint::new(
            point.x * self.zoom + self.offset.x,
            point.y * self.zoom + self.offset.y,
        )
    }

    pub fn view_to_world(self, point: GraphPoint) -> GraphPoint {
        GraphPoint::new(
            (point.x - self.offset.x) / self.zoom,
            (point.y - self.offset.y) / self.zoom,
        )
    }

    pub fn world_rect_to_view(self, rect: GraphRect) -> GraphRect {
        GraphRect::new(
            self.world_to_view(rect.origin),
            GraphSize::new(rect.size.width * self.zoom, rect.size.height * self.zoom),
        )
    }

    pub fn pan_by(self, delta_x: f32, delta_y: f32) -> Self {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return self;
        }
        Self {
            offset: GraphPoint::new(self.offset.x + delta_x, self.offset.y + delta_y),
            ..self
        }
    }

    /// Changes zoom while keeping the graph point under `anchor` stationary.
    pub fn zoom_at(self, anchor: GraphPoint, factor: f32) -> Self {
        if !anchor.is_finite() || !factor.is_finite() || factor <= 0.0 {
            return self;
        }
        let graph_anchor = self.view_to_world(anchor);
        let zoom = clamp_zoom(self.zoom * factor);
        Self {
            offset: GraphPoint::new(
                anchor.x - graph_anchor.x * zoom,
                anchor.y - graph_anchor.y * zoom,
            ),
            zoom,
        }
    }

    /// Fits graph bounds into a local canvas while retaining a pixel margin.
    pub fn fit(bounds: GraphRect, canvas: GraphSize, padding: f32) -> Self {
        if !bounds.size.is_valid() || !canvas.is_valid() {
            return Self::default();
        }
        let padding = padding.max(0.0);
        let available_width = (canvas.width - padding * 2.0).max(1.0);
        let available_height = (canvas.height - padding * 2.0).max(1.0);
        let zoom = clamp_zoom(
            (available_width / bounds.size.width).min(available_height / bounds.size.height),
        );
        let content_width = bounds.size.width * zoom;
        let content_height = bounds.size.height * zoom;
        Self::new(
            GraphPoint::new(
                (canvas.width - content_width) * 0.5 - bounds.origin.x * zoom,
                (canvas.height - content_height) * 0.5 - bounds.origin.y * zoom,
            ),
            zoom,
        )
    }
}

fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(GRAPH_MIN_ZOOM, GRAPH_MAX_ZOOM)
    } else {
        1.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphPortKind {
    Input,
    Output,
    Bidirectional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphPortSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPort {
    pub id: GraphPortId,
    pub label: String,
    pub kind: GraphPortKind,
    pub side: GraphPortSide,
}

impl GraphPort {
    pub fn new(
        id: impl Into<GraphPortId>,
        label: impl Into<String>,
        kind: GraphPortKind,
        side: GraphPortSide,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            side,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: GraphNodeId,
    pub label: String,
    pub position: GraphPoint,
    pub size: GraphSize,
    pub ports: Vec<GraphPort>,
    /// Optional host identity for this node's interior (`nana.host-texture`
    /// registry slot or Vue `data-nana-gpu`). Empty / omitted keeps the
    /// default frame; Runtime never paints mermaid/math/formula pixels.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "contentSlot",
        alias = "hostTexture",
        alias = "host_texture"
    )]
    pub content_slot: Option<String>,
}

impl GraphNode {
    pub fn new(
        id: impl Into<GraphNodeId>,
        label: impl Into<String>,
        position: GraphPoint,
        size: GraphSize,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            position,
            size,
            ports: Vec::new(),
            content_slot: None,
        }
    }

    pub fn with_port(mut self, port: GraphPort) -> Self {
        self.ports.push(port);
        self
    }

    pub fn with_content_slot(mut self, slot: impl Into<String>) -> Self {
        let slot = slot.into();
        self.content_slot = (!slot.trim().is_empty()).then_some(slot);
        self
    }

    pub fn fit_height_to_ports(mut self) -> Self {
        self.size.height = graph_node_fitted_height(&self.ports);
        self
    }

    pub fn bounds(&self) -> GraphRect {
        GraphRect::new(self.position, self.size)
    }
}

pub fn graph_node_fitted_height(ports: &[GraphPort]) -> f32 {
    let mut left = 0;
    let mut right = 0;
    for port in ports {
        match port.side {
            GraphPortSide::Left => left += 1,
            GraphPortSide::Right => right += 1,
            GraphPortSide::Top | GraphPortSide::Bottom => {}
        }
    }
    let rows = left.max(right).max(1);
    GRAPH_NODE_TITLE_HEIGHT + GRAPH_PORT_INSET * 2.0 + rows as f32 * GRAPH_PORT_PITCH
}

fn stacked_port_offset(side_index: usize) -> f32 {
    GRAPH_NODE_TITLE_HEIGHT + GRAPH_PORT_INSET + (side_index as f32 + 0.5) * GRAPH_PORT_PITCH
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEndpoint {
    pub node: GraphNodeId,
    pub port: GraphPortId,
}

impl GraphEndpoint {
    pub fn new(node: impl Into<GraphNodeId>, port: impl Into<GraphPortId>) -> Self {
        Self {
            node: node.into(),
            port: port.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: GraphEdgeId,
    pub label: Option<String>,
    pub source: GraphEndpoint,
    pub target: GraphEndpoint,
}

impl GraphEdge {
    pub fn new(id: impl Into<GraphEdgeId>, source: GraphEndpoint, target: GraphEndpoint) -> Self {
        Self {
            id: id.into(),
            label: None,
            source,
            target,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphModelError {
    EmptyId(&'static str),
    DuplicateNode(GraphNodeId),
    MissingNode(GraphNodeId),
    DuplicatePort {
        node: GraphNodeId,
        port: GraphPortId,
    },
    DuplicateEdge(GraphEdgeId),
    InvalidNodeGeometry(GraphNodeId),
    MissingEndpoint {
        edge: GraphEdgeId,
        endpoint: GraphEndpoint,
    },
}

impl fmt::Display for GraphModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId(kind) => write!(formatter, "{kind} identifier must not be empty"),
            Self::DuplicateNode(id) => write!(formatter, "duplicate graph node `{id}`"),
            Self::MissingNode(id) => write!(formatter, "graph node `{id}` does not exist"),
            Self::DuplicatePort { node, port } => {
                write!(formatter, "duplicate port `{port}` on node `{node}`")
            }
            Self::DuplicateEdge(id) => write!(formatter, "duplicate graph edge `{id}`"),
            Self::InvalidNodeGeometry(id) => {
                write!(formatter, "node `{id}` has invalid position or size")
            }
            Self::MissingEndpoint { edge, endpoint } => write!(
                formatter,
                "edge `{edge}` references missing endpoint `{}:{}`",
                endpoint.node, endpoint.port
            ),
        }
    }
}

impl Error for GraphModelError {}

/// An application-independent graph with stable IDs and deterministic z-order.
///
/// Nodes and edges later in their vectors are considered visually in front of
/// earlier entries during hit testing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GraphModel {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

impl GraphModel {
    pub fn new(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> Result<Self, GraphModelError> {
        validate_model(&nodes, &edges)?;
        Ok(Self { nodes, edges })
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    pub fn node(&self, id: &GraphNodeId) -> Option<&GraphNode> {
        self.nodes.iter().find(|node| &node.id == id)
    }

    pub fn set_node_position(
        &mut self,
        id: &GraphNodeId,
        position: GraphPoint,
    ) -> Result<(), GraphModelError> {
        if !position.is_finite() {
            return Err(GraphModelError::InvalidNodeGeometry(id.clone()));
        }
        let node = self
            .nodes
            .iter_mut()
            .find(|node| &node.id == id)
            .ok_or_else(|| GraphModelError::MissingNode(id.clone()))?;
        node.position = position;
        Ok(())
    }

    pub fn add_edge(&mut self, edge: GraphEdge) -> Result<(), GraphModelError> {
        self.edges.push(edge);
        if let Err(error) = validate_model(&self.nodes, &self.edges) {
            self.edges.pop();
            return Err(error);
        }
        Ok(())
    }

    pub fn edge(&self, id: &GraphEdgeId) -> Option<&GraphEdge> {
        self.edges.iter().find(|edge| &edge.id == id)
    }

    pub fn bounds(&self) -> Option<GraphRect> {
        self.nodes
            .iter()
            .map(GraphNode::bounds)
            .reduce(GraphRect::union)
    }

    pub fn port_position(&self, endpoint: &GraphEndpoint) -> Option<GraphPoint> {
        let node = self.node(&endpoint.node)?;
        let port = node.ports.iter().find(|port| port.id == endpoint.port)?;
        let side_count = node
            .ports
            .iter()
            .filter(|candidate| candidate.side == port.side)
            .count();
        let side_index = node
            .ports
            .iter()
            .filter(|candidate| candidate.side == port.side)
            .position(|candidate| candidate.id == port.id)?;
        Some(match port.side {
            GraphPortSide::Top => {
                let fraction = (side_index + 1) as f32 / (side_count + 1) as f32;
                GraphPoint::new(
                    node.position.x + node.size.width * fraction,
                    node.position.y,
                )
            }
            GraphPortSide::Right => GraphPoint::new(
                node.position.x + node.size.width,
                node.position.y + stacked_port_offset(side_index),
            ),
            GraphPortSide::Bottom => {
                let fraction = (side_index + 1) as f32 / (side_count + 1) as f32;
                GraphPoint::new(
                    node.position.x + node.size.width * fraction,
                    node.position.y + node.size.height,
                )
            }
            GraphPortSide::Left => GraphPoint::new(
                node.position.x,
                node.position.y + stacked_port_offset(side_index),
            ),
        })
    }

    pub fn edge_curve(&self, edge: &GraphEdge, viewport: GraphViewport) -> Option<[GraphPoint; 4]> {
        let source = self.endpoint_geometry(&edge.source)?;
        let target = self.endpoint_geometry(&edge.target)?;
        let start = viewport.world_to_view(source.0);
        let end = viewport.world_to_view(target.0);
        let reach =
            ((end.x - start.x).abs().max((end.y - start.y).abs()) * 0.45).clamp(32.0, 180.0);
        let source_tangent = port_tangent(source.1);
        let target_tangent = port_tangent(target.1);
        Some([
            start,
            GraphPoint::new(
                start.x + source_tangent.x * reach,
                start.y + source_tangent.y * reach,
            ),
            GraphPoint::new(
                end.x + target_tangent.x * reach,
                end.y + target_tangent.y * reach,
            ),
            end,
        ])
    }

    pub fn hit_test(
        &self,
        viewport: GraphViewport,
        view_position: GraphPoint,
    ) -> Option<GraphSelection> {
        if !view_position.is_finite() {
            return None;
        }
        let port_radius_squared = GRAPH_PORT_HIT_RADIUS * GRAPH_PORT_HIT_RADIUS;
        for node in self.nodes.iter().rev() {
            for port in node.ports.iter().rev() {
                let endpoint = GraphEndpoint::new(node.id.clone(), port.id.clone());
                let position = viewport.world_to_view(self.port_position(&endpoint)?);
                if position.distance_squared(view_position) <= port_radius_squared {
                    return Some(GraphSelection::Port {
                        node: node.id.clone(),
                        port: port.id.clone(),
                    });
                }
            }
        }
        for node in self.nodes.iter().rev() {
            if viewport
                .world_rect_to_view(node.bounds())
                .contains(view_position)
            {
                return Some(GraphSelection::Node(node.id.clone()));
            }
        }
        for edge in self.edges.iter().rev() {
            let Some(curve) = self.edge_curve(edge, viewport) else {
                continue;
            };
            if distance_to_curve(view_position, curve) <= GRAPH_EDGE_HIT_TOLERANCE {
                return Some(GraphSelection::Edge(edge.id.clone()));
            }
        }
        None
    }

    pub fn target_descriptors(
        &self,
        canvas_id: &GraphCanvasId,
        viewport: GraphViewport,
        selection: Option<&GraphSelection>,
        canvas_size: GraphSize,
    ) -> Vec<GraphTargetDescriptor> {
        let mut descriptors = Vec::with_capacity(
            1 + self.nodes.len()
                + self.edges.len()
                + self
                    .nodes
                    .iter()
                    .map(|node| node.ports.len())
                    .sum::<usize>(),
        );
        descriptors.push(GraphTargetDescriptor::new(
            GraphTarget::Canvas,
            canvas_id,
            "Graph canvas".to_owned(),
            GraphRect::new(GraphPoint::ZERO, canvas_size),
            false,
        ));
        for edge in &self.edges {
            let Some(curve) = self.edge_curve(edge, viewport) else {
                continue;
            };
            let label = edge.label.clone().unwrap_or_else(|| {
                format!(
                    "{} {} to {} {}",
                    edge.source.node, edge.source.port, edge.target.node, edge.target.port
                )
            });
            descriptors.push(GraphTargetDescriptor::new(
                GraphTarget::Edge(edge.id.clone()),
                canvas_id,
                label,
                curve_bounds(curve).expand(GRAPH_EDGE_HIT_TOLERANCE),
                selection == Some(&GraphSelection::Edge(edge.id.clone())),
            ));
        }
        for node in &self.nodes {
            let node_selection = GraphSelection::Node(node.id.clone());
            descriptors.push(GraphTargetDescriptor::new(
                GraphTarget::Node(node.id.clone()),
                canvas_id,
                node.label.clone(),
                viewport.world_rect_to_view(node.bounds()),
                selection == Some(&node_selection),
            ));
            for port in &node.ports {
                let endpoint = GraphEndpoint::new(node.id.clone(), port.id.clone());
                let Some(position) = self.port_position(&endpoint) else {
                    continue;
                };
                let view = viewport.world_to_view(position);
                let target = GraphTarget::Port {
                    node: node.id.clone(),
                    port: port.id.clone(),
                };
                let port_selection = GraphSelection::Port {
                    node: node.id.clone(),
                    port: port.id.clone(),
                };
                descriptors.push(GraphTargetDescriptor::new(
                    target,
                    canvas_id,
                    format!("{} — {}", node.label, port.label),
                    GraphRect::new(
                        GraphPoint::new(
                            view.x - GRAPH_PORT_HIT_RADIUS,
                            view.y - GRAPH_PORT_HIT_RADIUS,
                        ),
                        GraphSize::new(GRAPH_PORT_HIT_RADIUS * 2.0, GRAPH_PORT_HIT_RADIUS * 2.0),
                    ),
                    selection == Some(&port_selection),
                ));
            }
        }
        descriptors
    }

    fn endpoint_geometry(&self, endpoint: &GraphEndpoint) -> Option<(GraphPoint, GraphPortSide)> {
        let node = self.node(&endpoint.node)?;
        let port = node.ports.iter().find(|port| port.id == endpoint.port)?;
        Some((self.port_position(endpoint)?, port.side))
    }
}

fn validate_model(nodes: &[GraphNode], edges: &[GraphEdge]) -> Result<(), GraphModelError> {
    let mut node_ids = HashSet::with_capacity(nodes.len());
    for node in nodes {
        if node.id.as_str().is_empty() {
            return Err(GraphModelError::EmptyId("node"));
        }
        if !node_ids.insert(node.id.clone()) {
            return Err(GraphModelError::DuplicateNode(node.id.clone()));
        }
        if !node.position.is_finite() || !node.size.is_valid() {
            return Err(GraphModelError::InvalidNodeGeometry(node.id.clone()));
        }
        let mut port_ids = HashSet::with_capacity(node.ports.len());
        for port in &node.ports {
            if port.id.as_str().is_empty() {
                return Err(GraphModelError::EmptyId("port"));
            }
            if !port_ids.insert(port.id.clone()) {
                return Err(GraphModelError::DuplicatePort {
                    node: node.id.clone(),
                    port: port.id.clone(),
                });
            }
        }
    }
    let mut edge_ids = HashSet::with_capacity(edges.len());
    for edge in edges {
        if edge.id.as_str().is_empty() {
            return Err(GraphModelError::EmptyId("edge"));
        }
        if !edge_ids.insert(edge.id.clone()) {
            return Err(GraphModelError::DuplicateEdge(edge.id.clone()));
        }
        for endpoint in [&edge.source, &edge.target] {
            let exists = nodes.iter().any(|node| {
                node.id == endpoint.node && node.ports.iter().any(|port| port.id == endpoint.port)
            });
            if !exists {
                return Err(GraphModelError::MissingEndpoint {
                    edge: edge.id.clone(),
                    endpoint: endpoint.clone(),
                });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphSelection {
    Node(GraphNodeId),
    Port {
        node: GraphNodeId,
        port: GraphPortId,
    },
    Edge(GraphEdgeId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphTarget {
    Canvas,
    Node(GraphNodeId),
    Port {
        node: GraphNodeId,
        port: GraphPortId,
    },
    Edge(GraphEdgeId),
}

impl From<GraphSelection> for GraphTarget {
    fn from(selection: GraphSelection) -> Self {
        match selection {
            GraphSelection::Node(node) => Self::Node(node),
            GraphSelection::Port { node, port } => Self::Port { node, port },
            GraphSelection::Edge(edge) => Self::Edge(edge),
        }
    }
}

impl GraphTarget {
    pub fn stable_id(&self, canvas_id: &GraphCanvasId) -> GraphTargetId {
        let canvas = encode_target_segment(canvas_id.as_str());
        GraphTargetId::new(match self {
            Self::Canvas => format!("graph.{canvas}.canvas"),
            Self::Node(node) => {
                format!(
                    "graph.{canvas}.node.{}",
                    encode_target_segment(node.as_str())
                )
            }
            Self::Port { node, port } => format!(
                "graph.{canvas}.node.{}.port.{}",
                encode_target_segment(node.as_str()),
                encode_target_segment(port.as_str())
            ),
            Self::Edge(edge) => {
                format!(
                    "graph.{canvas}.edge.{}",
                    encode_target_segment(edge.as_str())
                )
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphTargetKind {
    Canvas,
    Node,
    Port,
    Edge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTargetDescriptor {
    pub id: GraphTargetId,
    pub target: GraphTarget,
    pub kind: GraphTargetKind,
    pub label: String,
    pub view_bounds: GraphRect,
    pub selected: bool,
}

impl GraphTargetDescriptor {
    fn new(
        target: GraphTarget,
        canvas_id: &GraphCanvasId,
        label: String,
        view_bounds: GraphRect,
        selected: bool,
    ) -> Self {
        let kind = match target {
            GraphTarget::Canvas => GraphTargetKind::Canvas,
            GraphTarget::Node(_) => GraphTargetKind::Node,
            GraphTarget::Port { .. } => GraphTargetKind::Port,
            GraphTarget::Edge(_) => GraphTargetKind::Edge,
        };
        let id = target.stable_id(canvas_id);
        Self {
            id,
            target,
            kind,
            label,
            view_bounds,
            selected,
        }
    }
}

fn encode_target_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            encoded.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

/// Outward unit tangent for a port side. Out/Right points right; In/Left
/// points left. Edge control points sit along this vector away from the node.
pub fn port_tangent(side: GraphPortSide) -> GraphPoint {
    match side {
        GraphPortSide::Top => GraphPoint::new(0.0, -1.0),
        GraphPortSide::Right => GraphPoint::new(1.0, 0.0),
        GraphPortSide::Bottom => GraphPoint::new(0.0, 1.0),
        GraphPortSide::Left => GraphPoint::new(-1.0, 0.0),
    }
}

/// Evaluates a cubic Bézier at `t` in view space.
pub fn cubic_point(curve: [GraphPoint; 4], t: f32) -> GraphPoint {
    let inverse = 1.0 - t;
    let a = inverse * inverse * inverse;
    let b = 3.0 * inverse * inverse * t;
    let c = 3.0 * inverse * t * t;
    let d = t * t * t;
    GraphPoint::new(
        curve[0].x * a + curve[1].x * b + curve[2].x * c + curve[3].x * d,
        curve[0].y * a + curve[1].y * b + curve[2].y * c + curve[3].y * d,
    )
}

fn curve_bounds(curve: [GraphPoint; 4]) -> GraphRect {
    let mut min = curve[0];
    let mut max = curve[0];
    for index in 1..=24 {
        let point = cubic_point(curve, index as f32 / 24.0);
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
    }
    GraphRect::from_points(min, max)
}

fn distance_to_curve(point: GraphPoint, curve: [GraphPoint; 4]) -> f32 {
    let mut closest = f32::INFINITY;
    let mut previous = curve[0];
    for index in 1..=24 {
        let next = cubic_point(curve, index as f32 / 24.0);
        closest = closest.min(distance_to_segment(point, previous, next));
        previous = next;
    }
    closest
}

fn distance_to_segment(point: GraphPoint, start: GraphPoint, end: GraphPoint) -> f32 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return point.distance_squared(start).sqrt();
    }
    let projection =
        (((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared).clamp(0.0, 1.0);
    point
        .distance_squared(GraphPoint::new(
            start.x + dx * projection,
            start.y + dy * projection,
        ))
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> GraphModel {
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

    #[test]
    fn viewport_round_trip_and_cursor_anchored_zoom_are_stable() {
        let viewport = GraphViewport::new(GraphPoint::new(30.0, -12.0), 1.75);
        let world = GraphPoint::new(80.0, 42.0);
        let view = viewport.world_to_view(world);
        assert_eq!(viewport.view_to_world(view), world);

        let zoomed = viewport.zoom_at(view, 1.5);
        assert!((zoomed.world_to_view(world).x - view.x).abs() < 0.001);
        assert!((zoomed.world_to_view(world).y - view.y).abs() < 0.001);
    }

    #[test]
    fn zoom_is_clamped() {
        assert_eq!(
            GraphViewport::new(GraphPoint::ZERO, 100.0).zoom,
            GRAPH_MAX_ZOOM
        );
        assert_eq!(
            GraphViewport::new(GraphPoint::ZERO, 0.001).zoom,
            GRAPH_MIN_ZOOM
        );
        assert_eq!(
            GraphViewport::default()
                .zoom_at(GraphPoint::new(40.0, 40.0), 1_000.0)
                .zoom,
            GRAPH_MAX_ZOOM
        );
        assert_eq!(
            GraphViewport::default()
                .zoom_at(GraphPoint::new(40.0, 40.0), 0.0001)
                .zoom,
            GRAPH_MIN_ZOOM
        );
    }

    #[test]
    fn model_rejects_duplicate_ids_and_missing_endpoints() {
        let node = GraphNode::new(
            "node",
            "Node",
            GraphPoint::ZERO,
            GraphSize::new(100.0, 60.0),
        );
        assert!(matches!(
            GraphModel::new(vec![node.clone(), node], Vec::new()),
            Err(GraphModelError::DuplicateNode(_))
        ));

        let node = GraphNode::new(
            "node",
            "Node",
            GraphPoint::ZERO,
            GraphSize::new(100.0, 60.0),
        );
        let edge = GraphEdge::new(
            "edge",
            GraphEndpoint::new("node", "missing"),
            GraphEndpoint::new("node", "missing"),
        );
        assert!(matches!(
            GraphModel::new(vec![node], vec![edge]),
            Err(GraphModelError::MissingEndpoint { .. })
        ));
    }

    #[test]
    fn controlled_edits_move_nodes_and_reject_invalid_edges_without_mutation() {
        let mut model = graph();
        let source: GraphNodeId = "source".into();
        model
            .set_node_position(&source, GraphPoint::new(42.0, 64.0))
            .unwrap();
        assert_eq!(
            model.node(&source).map(|node| node.position),
            Some(GraphPoint::new(42.0, 64.0))
        );

        let edge_count = model.edges().len();
        let error = model
            .add_edge(GraphEdge::new(
                "invalid",
                GraphEndpoint::new("source", "missing"),
                GraphEndpoint::new("target", "in"),
            ))
            .unwrap_err();
        assert!(matches!(error, GraphModelError::MissingEndpoint { .. }));
        assert_eq!(model.edges().len(), edge_count);
    }

    #[test]
    fn left_ports_stack_from_the_title_and_take_hit_priority() {
        let origin = GraphPoint::new(20.0, 20.0);
        let model = GraphModel::new(
            vec![
                GraphNode::new("node", "Node", origin, GraphSize::new(100.0, 60.0))
                    .with_port(GraphPort::new(
                        "first",
                        "First",
                        GraphPortKind::Input,
                        GraphPortSide::Left,
                    ))
                    .with_port(GraphPort::new(
                        "second",
                        "Second",
                        GraphPortKind::Input,
                        GraphPortSide::Left,
                    ))
                    .fit_height_to_ports(),
            ],
            Vec::new(),
        )
        .expect("valid graph");
        let first = GraphEndpoint::new("node", "first");
        let second = GraphEndpoint::new("node", "second");
        let first_y = origin.y + stacked_port_offset(0);
        let second_y = origin.y + stacked_port_offset(1);
        assert_eq!(
            model.port_position(&first),
            Some(GraphPoint::new(origin.x, first_y))
        );
        assert_eq!(
            model.port_position(&second),
            Some(GraphPoint::new(origin.x, second_y))
        );
        assert!(second_y > first_y);
        assert_eq!(
            model.hit_test(GraphViewport::default(), GraphPoint::new(origin.x, first_y)),
            Some(GraphSelection::Port {
                node: "node".into(),
                port: "first".into(),
            })
        );
    }

    #[test]
    fn shorter_side_aligns_from_the_title_not_the_node_center() {
        let origin = GraphPoint::new(20.0, 20.0);
        let node = GraphNode::new("node", "Node", origin, GraphSize::new(100.0, 60.0))
            .with_port(GraphPort::new(
                "in",
                "In",
                GraphPortKind::Input,
                GraphPortSide::Left,
            ))
            .with_port(GraphPort::new(
                "out-a",
                "A",
                GraphPortKind::Output,
                GraphPortSide::Right,
            ))
            .with_port(GraphPort::new(
                "out-b",
                "B",
                GraphPortKind::Output,
                GraphPortSide::Right,
            ))
            .with_port(GraphPort::new(
                "out-c",
                "C",
                GraphPortKind::Output,
                GraphPortSide::Right,
            ))
            .fit_height_to_ports();
        let height = node.size.height;
        let model = GraphModel::new(vec![node], Vec::new()).expect("valid graph");
        let input = model
            .port_position(&GraphEndpoint::new("node", "in"))
            .expect("input");
        let first_out = model
            .port_position(&GraphEndpoint::new("node", "out-a"))
            .expect("first output");
        let last_out = model
            .port_position(&GraphEndpoint::new("node", "out-c"))
            .expect("last output");
        assert_eq!(input.y, first_out.y);
        assert_eq!(input.y, origin.y + stacked_port_offset(0));
        let center_y = origin.y + height * 0.5;
        assert!((input.y - center_y).abs() > 1.0);
        assert!(last_out.y > first_out.y);
    }

    #[test]
    fn fitted_height_follows_the_side_with_more_ports() {
        let one = GraphNode::new("one", "One", GraphPoint::ZERO, GraphSize::new(100.0, 60.0))
            .with_port(GraphPort::new(
                "in",
                "In",
                GraphPortKind::Input,
                GraphPortSide::Left,
            ))
            .with_port(GraphPort::new(
                "out",
                "Out",
                GraphPortKind::Output,
                GraphPortSide::Right,
            ))
            .fit_height_to_ports();
        let three = GraphNode::new(
            "three",
            "Three",
            GraphPoint::ZERO,
            GraphSize::new(100.0, 60.0),
        )
        .with_port(GraphPort::new(
            "in",
            "In",
            GraphPortKind::Input,
            GraphPortSide::Left,
        ))
        .with_port(GraphPort::new(
            "out-a",
            "A",
            GraphPortKind::Output,
            GraphPortSide::Right,
        ))
        .with_port(GraphPort::new(
            "out-b",
            "B",
            GraphPortKind::Output,
            GraphPortSide::Right,
        ))
        .with_port(GraphPort::new(
            "out-c",
            "C",
            GraphPortKind::Output,
            GraphPortSide::Right,
        ))
        .fit_height_to_ports();
        assert_eq!(one.size.height, graph_node_fitted_height(&one.ports));
        assert!(three.size.height > one.size.height);
        assert_eq!(
            three.size.height,
            GRAPH_NODE_TITLE_HEIGHT + GRAPH_PORT_INSET * 2.0 + 3.0 * GRAPH_PORT_PITCH
        );
    }

    #[test]
    fn hit_test_distinguishes_node_port_and_empty() {
        let model = graph();
        let viewport = GraphViewport::default();
        let port = model
            .port_position(&GraphEndpoint::new("source", "out"))
            .expect("source port");
        assert_eq!(
            model.hit_test(viewport, GraphPoint::new(70.0, 60.0)),
            Some(GraphSelection::Node("source".into()))
        );
        assert_eq!(
            model.hit_test(viewport, port),
            Some(GraphSelection::Port {
                node: "source".into(),
                port: "out".into(),
            })
        );
        assert_eq!(model.hit_test(viewport, GraphPoint::new(5.0, 5.0)), None);
    }

    #[test]
    fn edge_curve_leaves_output_to_the_right_and_enters_input_from_the_left() {
        let model = graph();
        let curve = model
            .edge_curve(&model.edges()[0], GraphViewport::default())
            .expect("edge curve");
        assert!(
            curve[1].x > curve[0].x,
            "output control should sit to the right of the Out port: {curve:?}"
        );
        assert!(
            curve[2].x < curve[3].x,
            "input control should sit to the left of the In port: {curve:?}"
        );
    }

    #[test]
    fn edges_are_hit_tested_from_the_rendered_curve() {
        let model = graph();
        let edge = &model.edges()[0];
        let curve = model
            .edge_curve(edge, GraphViewport::default())
            .expect("edge curve");
        let midpoint = cubic_point(curve, 0.5);
        assert_eq!(
            model.hit_test(GraphViewport::default(), midpoint),
            Some(GraphSelection::Edge("flow".into()))
        );
    }

    #[test]
    fn target_descriptors_have_stable_escaped_ids_and_selection() {
        let model = graph();
        let descriptors = model.target_descriptors(
            &GraphCanvasId::new("workspace/main"),
            GraphViewport::default(),
            Some(&GraphSelection::Node("source".into())),
            GraphSize::new(640.0, 480.0),
        );
        let source = descriptors
            .iter()
            .find(|target| target.target == GraphTarget::Node("source".into()))
            .expect("node descriptor");
        assert_eq!(source.id.as_str(), "graph.workspace%2Fmain.node.source");
        assert!(source.selected);
        assert!(
            descriptors.iter().any(|target| {
                target.id.as_str() == "graph.workspace%2Fmain.node.source.port.out"
            })
        );
    }

    #[test]
    fn fit_centers_graph_bounds_inside_canvas() {
        let bounds = GraphRect::new(GraphPoint::new(100.0, 50.0), GraphSize::new(400.0, 200.0));
        let viewport = GraphViewport::fit(bounds, GraphSize::new(1000.0, 600.0), 50.0);
        let view = viewport.world_rect_to_view(bounds);
        assert!((view.origin.x - 50.0).abs() < 0.001);
        assert!((view.origin.y - 75.0).abs() < 0.001);
        assert!((view.size.width - 900.0).abs() < 0.001);
        assert!((view.size.height - 450.0).abs() < 0.001);
    }

    #[test]
    fn content_slot_round_trips_json() {
        let node = GraphNode::new(
            "source",
            "Source",
            GraphPoint::new(10.0, 20.0),
            GraphSize::new(120.0, 80.0),
        )
        .with_content_slot("formula.source");
        let encoded = serde_json::to_value(&node).expect("encode");
        assert_eq!(
            encoded
                .get("content_slot")
                .or_else(|| encoded.get("contentSlot")),
            Some(&serde_json::Value::String("formula.source".into()))
        );
        let decoded: GraphNode = serde_json::from_value(encoded).expect("decode");
        assert_eq!(decoded.content_slot.as_deref(), Some("formula.source"));
        let alias: GraphNode = serde_json::from_str(
            r#"{"id":"source","label":"Source","position":{"x":0,"y":0},"size":{"width":10,"height":10},"ports":[],"contentSlot":"preview"}"#,
        )
        .expect("alias");
        assert_eq!(alias.content_slot.as_deref(), Some("preview"));
    }
}
