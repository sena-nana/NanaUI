use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use nana_ui_core::{
    ControlSize, DrawerSide, Icon, LineHeightSpec, SwitchControlPosition, UI_METRICS,
};
use nana_ui_runtime::{
    ComponentElevation, ComponentGeometry, ComponentTextRegion, CustomRenderNode, ExtractedNode,
    LayoutBox, StableNodeId, StandardVisual, TextHorizontalAlignment, TextShaping,
    TextVerticalAlignment,
};

use crate::{
    AccessMode, CompiledRenderGraph, GraphError, PassId, RenderGraph, RenderOperation, RenderPass,
    RenderResource, ResourceAccess, ResourceId,
};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SceneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineTransform(pub [f32; 6]);

impl AffineTransform {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn then(self, rhs: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [ra, rb, rc, rd, re, rf] = rhs.0;
        Self([
            a * ra + c * rb,
            b * ra + d * rb,
            a * rc + c * rd,
            b * rc + d * rd,
            a * re + c * rf + e,
            b * re + d * rf + f,
        ])
    }
}

impl Default for AffineTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipRegion {
    pub bounds: SceneRect,
    pub transform: AffineTransform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrimitiveId {
    pub node: StableNodeId,
    pub slot: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScenePrimitiveKind {
    Quad {
        background: Option<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: f32,
        shadow: Option<ComponentElevation>,
    },
    QuadBatch {
        bounds: Vec<SceneRect>,
        background: Option<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: f32,
        shadow: Option<ComponentElevation>,
    },
    Text {
        content: String,
        color: Option<[f32; 4]>,
        size: f32,
        weight: Option<u16>,
        family: Option<String>,
        line_height: Option<LineHeightSpec>,
        letter_spacing: f32,
        wrap: bool,
        ellipsis: bool,
        shaping: TextShaping,
        horizontal_alignment: TextHorizontalAlignment,
        vertical_alignment: TextVerticalAlignment,
        /// Theme-resolved committed-text spans. Empty means solid `color`.
        spans: Vec<SceneTextSpan>,
    },
    Icon {
        icon: Icon,
        color: Option<[f32; 4]>,
    },
    Spinner {
        phase: u8,
        color: Option<[f32; 4]>,
    },
    Custom(CustomRenderNode),
}

/// One paint-ready span inside a [`ScenePrimitiveKind::Text`] primitive.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTextSpan {
    pub start: usize,
    pub end: usize,
    pub color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenePrimitive {
    pub id: PrimitiveId,
    pub node: StableNodeId,
    pub bounds: SceneRect,
    pub transform: AffineTransform,
    pub clips: Vec<ClipRegion>,
    pub opacity: f32,
    pub z_index: i32,
    pub document_order: usize,
    pub kind: ScenePrimitiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SceneOrderKey {
    z_index: i32,
    document_order: usize,
    slot: u8,
    node: StableNodeId,
}

const MAX_NODE_PRIMITIVE_SLOT: u8 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneDelta {
    pub updated_nodes: usize,
    pub removed_nodes: usize,
    pub rebuilt_primitives: usize,
    pub order_rebuilt: bool,
    pub primitive_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct UiScene {
    nodes: HashMap<StableNodeId, ExtractedNode>,
    node_order: HashMap<StableNodeId, usize>,
    primitives: BTreeMap<PrimitiveId, ScenePrimitive>,
    ordered: BTreeSet<SceneOrderKey>,
}

impl UiScene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn primitives(&self) -> impl Iterator<Item = &ScenePrimitive> {
        self.ordered.iter().filter_map(|key| {
            self.primitives.get(&PrimitiveId {
                node: key.node,
                slot: key.slot,
            })
        })
    }

    pub fn node_bounds(&self, id: StableNodeId) -> Option<SceneRect> {
        self.nodes.get(&id).map(|node| SceneRect {
            x: node.layout.x,
            y: node.layout.y,
            width: node.layout.width,
            height: node.layout.height,
        })
    }

    pub fn is_node_in_subtree(&self, root: StableNodeId, candidate: StableNodeId) -> bool {
        let mut current = Some(candidate);
        let mut visited = HashSet::new();
        while let Some(id) = current.filter(|id| visited.insert(*id)) {
            if id == root {
                return true;
            }
            current = self.nodes.get(&id).and_then(|node| node.parent);
        }
        false
    }

    /// Apply Runtime's dirty extraction and tombstone stream atomically.
    pub fn apply_delta(
        &mut self,
        extracted: impl IntoIterator<Item = ExtractedNode>,
        removals: impl IntoIterator<Item = StableNodeId>,
    ) -> SceneDelta {
        let mut removed_nodes = 0;
        let mut changed = Vec::new();
        let mut hierarchy_changed = false;
        for id in removals {
            if let Some(old) = self.nodes.remove(&id) {
                removed_nodes += 1;
                hierarchy_changed |= old.parent.is_some() || !old.children.is_empty();
                self.remove_node_primitives(id);
            }
        }
        let mut updated_nodes = 0;
        for node in extracted {
            hierarchy_changed |= self
                .nodes
                .get(&node.id)
                .is_none_or(|old| old.parent != node.parent || old.children != node.children);
            changed.push(node.id);
            self.nodes.insert(node.id, node);
            updated_nodes += 1;
        }
        let order_rebuilt = (updated_nodes != 0 || removed_nodes != 0)
            && (hierarchy_changed || self.node_order.len() != self.nodes.len());
        let mut rebuilt_primitives = 0;
        if updated_nodes != 0 || removed_nodes != 0 {
            if order_rebuilt {
                self.rebuild_document_order();
                for primitive in self.primitives.values_mut() {
                    if let Some(order) = self.node_order.get(&primitive.node) {
                        primitive.document_order = *order;
                    }
                }
            }
            for &id in &changed {
                rebuilt_primitives += self.rebuild_node_primitives(id);
            }
            if order_rebuilt {
                self.sort_primitives();
            } else {
                for id in changed {
                    self.insert_node_ordered(id);
                }
            }
        }
        SceneDelta {
            updated_nodes,
            removed_nodes,
            rebuilt_primitives,
            order_rebuilt,
            primitive_count: self.primitives.len(),
        }
    }

    /// Build the default frame pass. Custom operations remain in exact scene
    /// order and split standard draw segments, allowing a backend extension to
    /// encode a real pass between ordinary UI items. Opaque custom resources
    /// are explicit external graph inputs rather than hidden backend state.
    pub fn frame_graph(&self, target: ResourceId) -> Result<CompiledRenderGraph, GraphError> {
        let mut graph = RenderGraph::new();
        graph.add_resource(RenderResource {
            id: target,
            label: "ui-target".into(),
            external: true,
        })?;
        let mut next_resource = 1_u64;
        let mut custom_nodes: BTreeMap<Arc<str>, (PrimitiveId, CustomRenderNode)> = BTreeMap::new();
        for primitive in self.primitives() {
            let ScenePrimitiveKind::Custom(custom) = &primitive.kind else {
                continue;
            };
            if let Some((_, previous)) = custom_nodes.get(&custom.resource)
                && (previous.revision != custom.revision || previous.renderer != custom.renderer)
            {
                return Err(GraphError::ConflictingExternalResource(
                    custom.resource.to_string(),
                ));
            }
            custom_nodes
                .entry(custom.resource.clone())
                .or_insert((primitive.id, custom.clone()));
        }
        let custom_resources = custom_nodes
            .into_iter()
            .map(|(resource, (representative, _))| {
                while ResourceId(next_resource) == target {
                    next_resource += 1;
                }
                let id = ResourceId(next_resource);
                next_resource += 1;
                (resource, (id, representative))
            })
            .collect::<HashMap<_, _>>();
        for (label, (id, _)) in &custom_resources {
            graph.add_resource(RenderResource {
                id: *id,
                label: label.to_string(),
                external: true,
            })?;
        }
        let mut pass_id = 1_u64;
        let mut ordered_resources = custom_resources.iter().collect::<Vec<_>>();
        ordered_resources.sort_by_key(|(label, _)| *label);
        for (label, (resource, representative)) in ordered_resources {
            graph.add_pass(RenderPass {
                id: PassId(pass_id),
                label: format!("prepare:{label}"),
                dependencies: Vec::new(),
                resources: vec![ResourceAccess {
                    resource: *resource,
                    mode: AccessMode::Write,
                }],
                operations: vec![RenderOperation::PrepareExternal(*representative)],
            })?;
            pass_id += 1;
        }
        let mut standard = Vec::new();
        let flush_standard = |graph: &mut RenderGraph,
                              pass_id: &mut u64,
                              standard: &mut Vec<RenderOperation>|
         -> Result<(), GraphError> {
            if standard.is_empty() {
                return Ok(());
            }
            graph.add_pass(RenderPass {
                id: PassId(*pass_id),
                label: "ui-standard".into(),
                dependencies: Vec::new(),
                resources: vec![ResourceAccess {
                    resource: target,
                    mode: AccessMode::ReadWrite,
                }],
                operations: std::mem::take(standard),
            })?;
            *pass_id += 1;
            Ok(())
        };
        for primitive in self.primitives() {
            match &primitive.kind {
                ScenePrimitiveKind::Custom(custom) => {
                    flush_standard(&mut graph, &mut pass_id, &mut standard)?;
                    let resource = custom_resources[&custom.resource].0;
                    graph.add_pass(RenderPass {
                        id: PassId(pass_id),
                        label: format!("custom:{}", custom.renderer),
                        dependencies: Vec::new(),
                        resources: vec![
                            ResourceAccess {
                                resource: target,
                                mode: AccessMode::ReadWrite,
                            },
                            ResourceAccess {
                                resource,
                                mode: AccessMode::Read,
                            },
                        ],
                        operations: vec![RenderOperation::InvokeCustom(primitive.id)],
                    })?;
                    pass_id += 1;
                }
                _ => standard.push(RenderOperation::Draw(primitive.id)),
            }
        }
        flush_standard(&mut graph, &mut pass_id, &mut standard)?;
        graph.compile()
    }

    pub fn primitive(&self, id: PrimitiveId) -> Option<&ScenePrimitive> {
        self.primitives.get(&id)
    }

    fn rebuild_document_order(&mut self) {
        self.node_order.clear();
        let mut roots = self
            .nodes
            .values()
            .filter(|node| node.parent.is_none() || !self.nodes.contains_key(&node.parent.unwrap()))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        roots.sort_unstable();
        let mut visited = HashSet::new();
        let mut order = 0;
        for root in roots {
            self.visit_order(root, &mut visited, &mut order);
        }
        // Malformed/incomplete deltas must not make retained nodes disappear.
        let mut detached = self.nodes.keys().copied().collect::<Vec<_>>();
        detached.sort_unstable();
        for id in detached {
            if !visited.contains(&id) {
                self.visit_order(id, &mut visited, &mut order);
            }
        }
    }

    fn visit_order(
        &mut self,
        id: StableNodeId,
        visited: &mut HashSet<StableNodeId>,
        order: &mut usize,
    ) {
        if !visited.insert(id) {
            return;
        }
        self.node_order.insert(id, *order);
        *order += 1;
        let children = self
            .nodes
            .get(&id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child in children {
            self.visit_order(child, visited, order);
        }
    }

    fn sort_primitives(&mut self) {
        self.ordered.clear();
        for primitive in self.primitives.values() {
            self.ordered.insert(Self::order_key(primitive));
        }
    }

    fn insert_node_ordered(&mut self, node: StableNodeId) {
        for slot in 0..=MAX_NODE_PRIMITIVE_SLOT {
            if let Some(primitive) = self.primitives.get(&PrimitiveId { node, slot }) {
                self.ordered.insert(Self::order_key(primitive));
            }
        }
    }

    fn rebuild_node_primitives(&mut self, id: StableNodeId) -> usize {
        self.remove_node_primitives(id);
        let Some(node) = self.nodes.get(&id).cloned() else {
            return 0;
        };
        let before = self.primitives.len();
        let (parent_transform, parent_opacity, parent_clips) = self.ancestor_state(&node);
        let layout = node.layout;
        let bounds = SceneRect {
            x: layout.x,
            y: layout.y,
            width: layout.width,
            height: layout.height,
        };
        let local_transform = node
            .source_style
            .layout
            .transform
            .map(|transform| {
                AffineTransform(transform.around_center(
                    layout.x,
                    layout.y,
                    layout.width,
                    layout.height,
                ))
            })
            .unwrap_or_default();
        let transform = parent_transform.then(local_transform);
        let opacity = parent_opacity
            * node
                .source_style
                .layout
                .opacity
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
        let mut clips = parent_clips.to_vec();
        if node.source_style.layout.clips_overflow() {
            clips.push(ClipRegion { bounds, transform });
        }
        let empty_state_content_clips =
            if let Some(ComponentGeometry::EmptyState { content_clip, .. }) =
                node.component_geometry.as_ref()
            {
                let mut content_clips = clips.clone();
                content_clips.push(ClipRegion {
                    bounds: scene_rect(*content_clip),
                    transform,
                });
                content_clips
            } else {
                clips.clone()
            };
        let text_input_clips =
            if matches!(node.standard_visual, Some(StandardVisual::TextInput { .. })) {
                let padding = node
                    .source_style
                    .layout
                    .resolved_padding_against(Some(bounds.width));
                let border = node.source_style.layout.resolved_border_width();
                let mut text_input_clips = clips.clone();
                text_input_clips.push(ClipRegion {
                    bounds: SceneRect {
                        x: bounds.x + border + padding.left,
                        y: bounds.y + border + padding.top,
                        width: (bounds.width - border * 2.0 - padding.left - padding.right)
                            .max(0.0),
                        height: (bounds.height - border * 2.0 - padding.top - padding.bottom)
                            .max(0.0),
                    },
                    transform,
                });
                text_input_clips
            } else {
                clips.clone()
            };
        let node_order = self.node_order.get(&id).copied().unwrap_or_default();
        if node.style.visible && opacity > 0.0 {
            let style = node.source_style.layout.as_ref();
            let standard_visual_uses_root_surface = matches!(
                node.standard_visual,
                Some(
                    StandardVisual::Button { .. }
                        | StandardVisual::TextInput { .. }
                        | StandardVisual::Icon { .. }
                        | StandardVisual::Switch { .. }
                        | StandardVisual::Card { .. }
                        | StandardVisual::ListItem { .. }
                        | StandardVisual::StatusBadge { .. }
                        | StandardVisual::SelectionOption { .. }
                        | StandardVisual::ModalFrame { .. }
                        | StandardVisual::Toast { .. }
                        | StandardVisual::XYPad { .. }
                        | StandardVisual::Select { .. }
                        | StandardVisual::ActionMenuItem { .. }
                        | StandardVisual::TreeView { .. }
                        | StandardVisual::CommandPalette { .. }
                )
            );
            let component_focus_ring = node.focused
                && matches!(
                    node.standard_visual,
                    Some(StandardVisual::Icon { .. } | StandardVisual::Switch { .. })
                );
            let surface_border_color = if component_focus_ring
                || matches!(node.standard_visual, Some(StandardVisual::Switch { .. }))
            {
                None
            } else {
                node.style.border_color
            };
            let (surface_background, surface_border_color, surface_border_width) =
                match node.component_geometry.as_ref() {
                    Some(ComponentGeometry::Button {
                        background,
                        border,
                        border_width,
                        ..
                    }) => (*background, *border, *border_width),
                    Some(ComponentGeometry::TextInput {
                        background,
                        border,
                        border_width,
                        ..
                    }) => (*background, *border, *border_width),
                    Some(ComponentGeometry::StatusBadge { background, .. }) => {
                        (Some(*background), None, 0.0)
                    }
                    _ => (
                        node.style.background,
                        surface_border_color,
                        if surface_border_color.is_some() {
                            style.border_width.unwrap_or(0.0).max(0.0)
                        } else {
                            0.0
                        },
                    ),
                };
            if !matches!(
                node.standard_visual,
                Some(StandardVisual::MenuSurface { .. })
            ) && (style.has_surface_paint()
                || ((node.standard_visual.is_none() || standard_visual_uses_root_surface)
                    && (node.style.background.is_some() || node.style.border_color.is_some())))
            {
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 0 },
                    node: id,
                    bounds,
                    transform,
                    clips: parent_clips.to_vec(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Quad {
                        background: surface_background,
                        border_color: surface_border_color,
                        border_width: surface_border_width,
                        corner_radius: style.border_radius.unwrap_or(0.0).max(0.0),
                        shadow: match node.component_geometry.as_ref() {
                            Some(ComponentGeometry::Card { elevation, .. }) => *elevation,
                            _ => None,
                        },
                    },
                });
            }
            if let Some(custom) = node.custom_render.clone() {
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 1 },
                    node: id,
                    bounds,
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Custom(custom),
                });
            }
            let component_owns_text = matches!(
                node.component_geometry,
                Some(
                    ComponentGeometry::Button { .. }
                        | ComponentGeometry::TextInput { .. }
                        | ComponentGeometry::Switch { .. }
                        | ComponentGeometry::Range { .. }
                        | ComponentGeometry::Card { .. }
                        | ComponentGeometry::StatusBadge { .. }
                        | ComponentGeometry::ValidationMessage { .. }
                        | ComponentGeometry::EmptyState { .. }
                        | ComponentGeometry::LabeledValue { .. }
                        | ComponentGeometry::SelectionOption { .. }
                        | ComponentGeometry::ModalFrame { .. }
                        | ComponentGeometry::Progress { .. }
                        | ComponentGeometry::FormField { .. }
                        | ComponentGeometry::Select { .. }
                        | ComponentGeometry::ActionMenuItem { .. }
                        | ComponentGeometry::MenuSurface { .. }
                        | ComponentGeometry::TreeView { .. }
                        | ComponentGeometry::CommandPalette { .. }
                )
            );
            if let Some(text) = node
                .text
                .as_ref()
                .filter(|text| !text.value.is_empty() && !component_owns_text)
            {
                let padding = style.resolved_padding_against(Some(bounds.width));
                let border = style.resolved_border_width();
                let leading_visual = match node.standard_visual {
                    Some(StandardVisual::Checkbox { .. }) => 24.0,
                    Some(StandardVisual::Switch {
                        control_position: SwitchControlPosition::Start,
                        ..
                    }) => 38.0,
                    _ => 0.0,
                };
                let trailing_visual = match node.standard_visual {
                    Some(StandardVisual::Switch {
                        control_position: SwitchControlPosition::End,
                        ..
                    }) => 38.0,
                    _ => 0.0,
                };
                let mut text_bounds = SceneRect {
                    x: bounds.x + border + padding.left + leading_visual,
                    y: bounds.y + border + padding.top,
                    width: (bounds.width
                        - border * 2.0
                        - padding.left
                        - padding.right
                        - leading_visual
                        - trailing_visual)
                        .max(0.0),
                    height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
                };
                if let Some(ComponentGeometry::ListItem {
                    content: Some(content),
                    ..
                }) = node.component_geometry
                {
                    text_bounds = scene_rect(content);
                }
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 2 },
                    node: id,
                    bounds: text_bounds,
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Text {
                        content: text.value.clone(),
                        color: node.style.color,
                        size: node.style.font_size,
                        weight: node.style.font_weight,
                        family: node.style.font_family.as_deref().map(str::to_owned),
                        line_height: node.style.line_height,
                        letter_spacing: node.style.letter_spacing,
                        wrap: !style.white_space_nowrap,
                        ellipsis: style.text_overflow_ellipsis,
                        shaping: if node.text_input.is_some() {
                            TextShaping::Advanced
                        } else {
                            TextShaping::Auto
                        },
                        horizontal_alignment: node.source_style.text_horizontal_alignment,
                        vertical_alignment: node.source_style.text_vertical_alignment,
                        spans: scene_text_spans(&node, &text.value),
                    },
                });
            }
            match node.component_geometry.as_ref() {
                Some(ComponentGeometry::ModalFrame {
                    scrim,
                    surface,
                    body: _,
                    title,
                    description,
                    body_text,
                    background,
                    elevation,
                    ..
                }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        10,
                        scene_rect(*scrim),
                        VisualQuadStyle {
                            background: Some([0.0, 0.0, 0.0, 0.45]),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 0.0,
                        },
                    ));
                    let radius = UI_METRICS.radius_md;
                    let docked = match node.standard_visual.as_ref() {
                        Some(StandardVisual::ModalFrame {
                            kind: nana_ui_runtime::ModalSurfaceKind::Drawer(side),
                            ..
                        }) => Some(*side),
                        _ => None,
                    };
                    let mut surface_bounds = scene_rect(*surface);
                    let mut surface_clips = clips.clone();
                    if let Some(side) = docked {
                        surface_clips.push(ClipRegion {
                            bounds: scene_rect(*scrim),
                            transform,
                        });
                        match side {
                            DrawerSide::Right => surface_bounds.width += radius,
                            DrawerSide::Left => {
                                surface_bounds.x -= radius;
                                surface_bounds.width += radius;
                            }
                            DrawerSide::Bottom => surface_bounds.height += radius,
                        }
                    }
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 11 },
                        node: id,
                        bounds: surface_bounds,
                        transform,
                        clips: surface_clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Quad {
                            background: Some(*background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: radius,
                            shadow: Some(*elevation),
                        },
                    });
                    self.insert_primitive(component_text_primitive(
                        id,
                        12,
                        title,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(description) = description {
                        self.insert_primitive(component_text_primitive(
                            id,
                            13,
                            description,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some(body_text) = body_text {
                        self.insert_primitive(component_text_primitive(
                            id,
                            14,
                            body_text,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::Button { label, .. }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Center,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::TextInput {
                    text,
                    selection,
                    selection_color,
                    ..
                }) => {
                    if !selection.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &text_input_clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            1,
                            selection.iter().map(|selection| scene_rect(*selection)),
                            VisualQuadStyle {
                                background: Some(*selection_color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: 0.0,
                            },
                        ));
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        text,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        text_input_clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::Switch { label, hint, .. }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(hint) = hint {
                        self.insert_primitive(component_text_primitive(
                            id,
                            3,
                            hint,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::Range {
                    label, value, unit, ..
                }) => {
                    if let Some(label) = label {
                        self.insert_primitive(component_text_primitive(
                            id,
                            2,
                            label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        6,
                        value,
                        TextHorizontalAlignment::End,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(unit) = unit {
                        self.insert_primitive(component_text_primitive(
                            id,
                            7,
                            unit,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::Card {
                    title: Some(title), ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        title,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::StatusBadge {
                    indicator,
                    label,
                    foreground,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*indicator),
                        VisualQuadStyle {
                            background: Some(*foreground),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 999.0,
                        },
                    ));
                }
                Some(ComponentGeometry::ValidationMessage {
                    indicator,
                    label,
                    foreground,
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*indicator),
                        VisualQuadStyle {
                            background: None,
                            border_color: Some(*foreground),
                            border_width: 1.0,
                            corner_radius: 999.0,
                        },
                    ));
                }
                Some(ComponentGeometry::EmptyState {
                    icon,
                    title,
                    message,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        title,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        empty_state_content_clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some((icon, bounds, color)) = icon {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: scene_rect(*bounds),
                            transform,
                            clips: empty_state_content_clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Icon {
                                icon: *icon,
                                color: Some(*color),
                            },
                        });
                    }
                    if let Some(message) = message {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            message,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            empty_state_content_clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::LabeledValue { label, value, .. }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    self.insert_primitive(component_text_primitive(
                        id,
                        3,
                        value,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::SelectionOption {
                    icon,
                    label,
                    focus_ring,
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Center,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some((icon, icon_bounds, color)) = icon {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: scene_rect(*icon_bounds),
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Icon {
                                icon: *icon,
                                color: Some(*color),
                            },
                        });
                    }
                    if let Some(color) = focus_ring {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &parent_clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            7,
                            SceneRect {
                                x: bounds.x - 4.0,
                                y: bounds.y - 4.0,
                                width: bounds.width + 8.0,
                                height: bounds.height + 8.0,
                            },
                            VisualQuadStyle {
                                background: None,
                                border_color: Some(*color),
                                border_width: 2.0,
                                corner_radius: style.border_radius.unwrap_or(0.0) + 4.0,
                            },
                        ));
                    }
                }
                Some(ComponentGeometry::Progress {
                    track,
                    fill,
                    label,
                    cancel,
                    corner_radius,
                }) => {
                    if let Some(label) = label {
                        self.insert_primitive(component_text_primitive(
                            id,
                            2,
                            label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*track),
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: *corner_radius,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        4,
                        scene_rect(*fill),
                        VisualQuadStyle {
                            background: node.standard_visual_foreground,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: *corner_radius,
                        },
                    ));
                    if let Some(cancel) = cancel {
                        self.insert_primitive(component_text_primitive(
                            id,
                            5,
                            &ComponentTextRegion {
                                bounds: *cancel,
                                content: Arc::from("×"),
                                color: node.style.color,
                                font_size: 15.0,
                                font_weight: None,
                            },
                            TextHorizontalAlignment::Center,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::FormField {
                    label,
                    support,
                    indicator,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(support) = support {
                        self.insert_primitive(component_text_primitive(
                            id,
                            3,
                            support,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some((bounds, color)) = indicator {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            4,
                            scene_rect(*bounds),
                            VisualQuadStyle {
                                background: None,
                                border_color: Some(*color),
                                border_width: 1.0,
                                corner_radius: 999.0,
                            },
                        ));
                    }
                }
                Some(ComponentGeometry::Toast {
                    indicator,
                    title,
                    description,
                    dismiss,
                }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        1,
                        scene_rect(*indicator),
                        VisualQuadStyle {
                            background: node.standard_visual_foreground,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 999.0,
                        },
                    ));
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        title,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(description) = description {
                        self.insert_primitive(component_text_primitive(
                            id,
                            3,
                            description,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some(dismiss) = dismiss {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            &ComponentTextRegion {
                                bounds: *dismiss,
                                content: Arc::from("×"),
                                color: node.style.color,
                                font_size: 15.0,
                                font_weight: None,
                            },
                            TextHorizontalAlignment::Center,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::XYPad {
                    pad: _,
                    thumb,
                    h_axis,
                    v_axis,
                    thumb_color,
                    axis_color,
                    ..
                }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        1,
                        scene_rect(*h_axis),
                        VisualQuadStyle {
                            background: Some(*axis_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 0.0,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        2,
                        scene_rect(*v_axis),
                        VisualQuadStyle {
                            background: Some(*axis_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 0.0,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*thumb),
                        VisualQuadStyle {
                            background: Some(*thumb_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 999.0,
                        },
                    ));
                }
                Some(ComponentGeometry::QrCode { field, dark, .. }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        0,
                        scene_rect(*field),
                        VisualQuadStyle {
                            background: Some([1.0, 1.0, 1.0, 1.0]),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: UI_METRICS.radius_md,
                        },
                    ));
                    if !dark.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            1,
                            dark.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some([0.0, 0.0, 0.0, 1.0]),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: 0.0,
                            },
                        ));
                    }
                }
                Some(ComponentGeometry::Select {
                    label,
                    handle,
                    handle_color,
                    menu,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    paint_select_handle(
                        self,
                        id,
                        handle,
                        *handle_color,
                        transform,
                        &clips,
                        opacity,
                        node.z_index,
                        node_order,
                    );
                    if let Some(menu) = menu {
                        let menu_z = node.z_index.max(1_000);
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 4 },
                            node: id,
                            bounds: scene_rect(menu.surface),
                            transform,
                            clips: parent_clips.to_vec(),
                            opacity,
                            z_index: menu_z,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Quad {
                                background: Some(menu.background),
                                border_color: Some(menu.border),
                                border_width: 1.0,
                                corner_radius: UI_METRICS.radius_md,
                                shadow: Some(menu.elevation),
                            },
                        });
                        for (index, option) in menu.options.iter().enumerate() {
                            let index = u8::try_from(index).unwrap_or(u8::MAX);
                            if option.checked {
                                let mut mark = component_text_primitive(
                                    id,
                                    70u8.saturating_add(index),
                                    &nana_ui_runtime::ComponentTextRegion {
                                        bounds: nana_ui_runtime::LayoutBox {
                                            x: option.bounds.x,
                                            y: option.bounds.y,
                                            width: 16.0,
                                            height: option.bounds.height,
                                        },
                                        content: Arc::from("✓"),
                                        color: node
                                            .standard_visual_foreground
                                            .or(option.label.color),
                                        font_size: option.label.font_size,
                                        font_weight: Some(700),
                                    },
                                    TextHorizontalAlignment::Center,
                                    false,
                                    &node,
                                    transform,
                                    parent_clips.to_vec(),
                                    opacity,
                                    node_order,
                                );
                                mark.z_index = menu_z;
                                self.insert_primitive(mark);
                            }
                            if let Some(background) = option.background {
                                self.insert_primitive(visual_quad(
                                    &VisualPrimitiveContext {
                                        node: id,
                                        transform,
                                        clips: &parent_clips,
                                        opacity,
                                        z_index: menu_z,
                                        document_order: node_order,
                                    },
                                    10u8.saturating_add(index),
                                    scene_rect(option.bounds),
                                    VisualQuadStyle {
                                        background: Some(background),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: UI_METRICS.radius_sm,
                                    },
                                ));
                            }
                            let mut label = component_text_primitive(
                                id,
                                40u8.saturating_add(index),
                                &option.label,
                                TextHorizontalAlignment::Start,
                                false,
                                &node,
                                transform,
                                parent_clips.to_vec(),
                                opacity,
                                node_order,
                            );
                            label.z_index = menu_z;
                            self.insert_primitive(label);
                        }
                    }
                }
                Some(ComponentGeometry::ActionMenuItem {
                    icon, label, hint, ..
                }) => {
                    if let Some((icon, icon_bounds, color)) = icon {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: scene_rect(*icon_bounds),
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Icon {
                                icon: *icon,
                                color: Some(*color),
                            },
                        });
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(hint) = hint {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            hint,
                            TextHorizontalAlignment::End,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::TreeView { rows }) => {
                    for (index, row) in rows.iter().enumerate() {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        if let Some(background) = row.background {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                10u8.saturating_add(index),
                                scene_rect(row.bounds),
                                VisualQuadStyle {
                                    background: Some(background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: UI_METRICS.radius_sm,
                                },
                            ));
                        }
                        if let Some(disclosure) = row.disclosure {
                            self.insert_primitive(component_text_primitive(
                                id,
                                40u8.saturating_add(index),
                                &nana_ui_runtime::ComponentTextRegion {
                                    bounds: disclosure,
                                    content: Arc::from(if row.expanded { "▾" } else { "▸" }),
                                    color: row.label.color,
                                    font_size: row.label.font_size,
                                    font_weight: None,
                                },
                                TextHorizontalAlignment::Center,
                                false,
                                &node,
                                transform,
                                clips.clone(),
                                opacity,
                                node_order,
                            ));
                        }
                        if let Some((icon, icon_bounds, color)) = row.icon {
                            self.insert_primitive(ScenePrimitive {
                                id: PrimitiveId {
                                    node: id,
                                    slot: 80u8.saturating_add(index),
                                },
                                node: id,
                                bounds: scene_rect(icon_bounds),
                                transform,
                                clips: clips.clone(),
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                                kind: ScenePrimitiveKind::Icon {
                                    icon,
                                    color: Some(color),
                                },
                            });
                        }
                        self.insert_primitive(component_text_primitive(
                            id,
                            110u8.saturating_add(index),
                            &row.label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::CommandPalette {
                    scrim,
                    surface,
                    title,
                    input,
                    empty,
                    rows,
                    background,
                    input_background,
                    input_border,
                    elevation,
                }) => {
                    let overlay_z = node.z_index.max(1_000);
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &parent_clips,
                            opacity,
                            z_index: overlay_z,
                            document_order: node_order,
                        },
                        10,
                        scene_rect(*scrim),
                        VisualQuadStyle {
                            background: Some([0.0, 0.0, 0.0, 0.45]),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 0.0,
                        },
                    ));
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 11 },
                        node: id,
                        bounds: scene_rect(*surface),
                        transform,
                        clips: parent_clips.to_vec(),
                        opacity,
                        z_index: overlay_z,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Quad {
                            background: Some(*background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: UI_METRICS.radius_md,
                            shadow: Some(*elevation),
                        },
                    });
                    let mut title_text = component_text_primitive(
                        id,
                        2,
                        title,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        parent_clips.to_vec(),
                        opacity,
                        node_order,
                    );
                    title_text.z_index = overlay_z;
                    self.insert_primitive(title_text);
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &parent_clips,
                            opacity,
                            z_index: overlay_z,
                            document_order: node_order,
                        },
                        12,
                        scene_rect(input.bounds),
                        VisualQuadStyle {
                            background: Some(*input_background),
                            border_color: Some(*input_border),
                            border_width: 1.0,
                            corner_radius: UI_METRICS.radius_sm,
                        },
                    ));
                    let mut input_text = component_text_primitive(
                        id,
                        3,
                        input,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        parent_clips.to_vec(),
                        opacity,
                        node_order,
                    );
                    input_text.z_index = overlay_z;
                    self.insert_primitive(input_text);
                    if let Some(empty) = empty {
                        let mut empty_text = component_text_primitive(
                            id,
                            4,
                            empty,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            parent_clips.to_vec(),
                            opacity,
                            node_order,
                        );
                        empty_text.z_index = overlay_z;
                        self.insert_primitive(empty_text);
                    }
                    for (index, row) in rows.iter().enumerate() {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        if let Some(background) = row.background {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: overlay_z,
                                    document_order: node_order,
                                },
                                20u8.saturating_add(index),
                                scene_rect(row.bounds),
                                VisualQuadStyle {
                                    background: Some(background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: UI_METRICS.radius_sm,
                                },
                            ));
                        }
                        let mut label = component_text_primitive(
                            id,
                            40u8.saturating_add(index),
                            &row.label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            parent_clips.to_vec(),
                            opacity,
                            node_order,
                        );
                        label.z_index = overlay_z;
                        self.insert_primitive(label);
                        if let Some(category) = &row.category {
                            let mut category = component_text_primitive(
                                id,
                                70u8.saturating_add(index),
                                category,
                                TextHorizontalAlignment::Start,
                                true,
                                &node,
                                transform,
                                parent_clips.to_vec(),
                                opacity,
                                node_order,
                            );
                            category.z_index = overlay_z;
                            self.insert_primitive(category);
                        }
                        if let Some(shortcut) = &row.shortcut {
                            let mut shortcut = component_text_primitive(
                                id,
                                100u8.saturating_add(index),
                                shortcut,
                                TextHorizontalAlignment::End,
                                true,
                                &node,
                                transform,
                                parent_clips.to_vec(),
                                opacity,
                                node_order,
                            );
                            shortcut.z_index = overlay_z;
                            self.insert_primitive(shortcut);
                        }
                    }
                }
                Some(ComponentGeometry::MenuSurface {
                    trigger,
                    surface,
                    search,
                    search_field,
                    options,
                    elevation,
                    background,
                    border,
                }) => {
                    if let Some(trigger) = trigger {
                        self.insert_primitive(component_text_primitive(
                            id,
                            2,
                            trigger,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if surface.height > 1.0 && surface.width > 1.0 {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 0 },
                            node: id,
                            bounds: scene_rect(*surface),
                            transform,
                            clips: parent_clips.to_vec(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Quad {
                                background: Some(*background),
                                border_color: Some(*border),
                                border_width: 1.0,
                                corner_radius: UI_METRICS.radius_md,
                                shadow: Some(*elevation),
                            },
                        });
                    }
                    if let Some(field) = search_field {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            3,
                            scene_rect(*field),
                            VisualQuadStyle {
                                background: Some(*background),
                                border_color: Some(*border),
                                border_width: 1.0,
                                corner_radius: UI_METRICS.radius_sm,
                            },
                        ));
                    }
                    if let Some(search) = search {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            search,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    for (index, option) in options.iter().enumerate() {
                        if let Some(background) = option.background {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                10u8.saturating_add(index as u8),
                                scene_rect(option.bounds),
                                VisualQuadStyle {
                                    background: Some(background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: UI_METRICS.radius_sm,
                                },
                            ));
                        }
                        self.insert_primitive(component_text_primitive(
                            id,
                            40u8.saturating_add(index as u8),
                            &option.label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::Card { title: None, .. })
                | Some(ComponentGeometry::ListItem { .. })
                | None => {}
            }
            let visual_context = VisualPrimitiveContext {
                node: id,
                transform,
                clips: if matches!(node.standard_visual, Some(StandardVisual::TextInput { .. })) {
                    &text_input_clips
                } else {
                    &clips
                },
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            match node.standard_visual {
                Some(StandardVisual::Button { loading_phase, .. }) => {
                    if let Some(ComponentGeometry::Button {
                        spinner,
                        focus_ring,
                        ..
                    }) = node.component_geometry.as_ref()
                    {
                        if let Some(spinner) = spinner {
                            self.insert_primitive(ScenePrimitive {
                                id: PrimitiveId { node: id, slot: 3 },
                                node: id,
                                bounds: scene_rect(*spinner),
                                transform,
                                clips: clips.clone(),
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                                kind: ScenePrimitiveKind::Spinner {
                                    phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                    color: node.standard_visual_foreground.or(node.style.color),
                                },
                            });
                        }
                        if let Some(color) = focus_ring {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                7,
                                SceneRect {
                                    x: bounds.x - 3.0,
                                    y: bounds.y - 3.0,
                                    width: bounds.width + 6.0,
                                    height: bounds.height + 6.0,
                                },
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(*color),
                                    border_width: 2.0,
                                    corner_radius: style.border_radius.unwrap_or(0.0).max(0.0)
                                        + 3.0,
                                },
                            ));
                        }
                    }
                }
                Some(StandardVisual::TextInput { .. }) => {
                    if let Some(ComponentGeometry::TextInput {
                        caret,
                        preedit,
                        focus_ring,
                        caret_color,
                        preedit_color,
                        ..
                    }) = node.component_geometry.as_ref()
                    {
                        if let Some(caret) = caret {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                4,
                                scene_rect(*caret),
                                VisualQuadStyle {
                                    background: Some(*caret_color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: 0.0,
                                },
                            ));
                        }
                        if !preedit.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                5,
                                preedit.iter().map(|preedit| scene_rect(*preedit)),
                                VisualQuadStyle {
                                    background: Some(*preedit_color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: 0.0,
                                },
                            ));
                        }
                        if let Some(color) = focus_ring {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                7,
                                SceneRect {
                                    x: bounds.x - 3.0,
                                    y: bounds.y - 3.0,
                                    width: bounds.width + 6.0,
                                    height: bounds.height + 6.0,
                                },
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(*color),
                                    border_width: 2.0,
                                    corner_radius: style.border_radius.unwrap_or(0.0).max(0.0)
                                        + 3.0,
                                },
                            ));
                        }
                    }
                }
                Some(StandardVisual::Checkbox { checked }) => {
                    let extent = 16.0_f32.min(bounds.height);
                    let indicator = SceneRect {
                        x: bounds.x,
                        y: bounds.y + (bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        3,
                        indicator,
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: node.style.border_color,
                            border_width: 1.0,
                            corner_radius: 4.0,
                        },
                    ));
                    if checked {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 4 },
                            node: id,
                            bounds: indicator,
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Text {
                                content: "✓".into(),
                                color: node.standard_visual_foreground,
                                size: 12.0,
                                weight: Some(700),
                                family: None,
                                line_height: None,
                                letter_spacing: 0.0,
                                wrap: false,
                                ellipsis: false,
                                shaping: TextShaping::Auto,
                                horizontal_alignment: TextHorizontalAlignment::Center,
                                vertical_alignment: TextVerticalAlignment::Center,
                                spans: Vec::new(),
                            },
                        });
                    }
                }
                Some(StandardVisual::Icon { icon, size, .. }) => {
                    let extent = size.max(0.0).min(bounds.width).min(bounds.height);
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 3 },
                        node: id,
                        bounds: SceneRect {
                            x: bounds.x + (bounds.width - extent) / 2.0,
                            y: bounds.y + (bounds.height - extent) / 2.0,
                            width: extent,
                            height: extent,
                        },
                        transform,
                        clips: clips.clone(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Icon {
                            icon,
                            color: node.standard_visual_foreground.or(node.style.color),
                        },
                    });
                    if node.focused {
                        self.insert_primitive(visual_quad(
                            &visual_context,
                            7,
                            SceneRect {
                                x: bounds.x - 3.0,
                                y: bounds.y - 3.0,
                                width: bounds.width + 6.0,
                                height: bounds.height + 6.0,
                            },
                            VisualQuadStyle {
                                background: None,
                                border_color: node.style.border_color,
                                border_width: 2.0,
                                corner_radius: style.border_radius.unwrap_or(0.0).max(0.0) + 3.0,
                            },
                        ));
                    }
                }
                Some(StandardVisual::Switch {
                    checked,
                    loading,
                    loading_phase,
                    ..
                }) => {
                    let (track, track_background, track_border, thumb_background) =
                        match node.component_geometry.as_ref() {
                            Some(ComponentGeometry::Switch {
                                control,
                                track_background,
                                track_border,
                                thumb_background,
                                ..
                            }) => (
                                scene_rect(*control),
                                Some(*track_background),
                                Some(*track_border),
                                Some(*thumb_background),
                            ),
                            _ => (
                                SceneRect {
                                    x: bounds.x,
                                    y: bounds.y + (bounds.height - 16.0) / 2.0,
                                    width: 30.0,
                                    height: 16.0,
                                },
                                node.style.background,
                                node.style.border_color,
                                node.standard_visual_foreground,
                            ),
                        };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        4,
                        track,
                        VisualQuadStyle {
                            background: track_background,
                            border_color: track_border,
                            border_width: 1.0,
                            corner_radius: 8.0,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        5,
                        SceneRect {
                            x: track.x + if checked { 17.0 } else { 3.0 },
                            y: track.y + 3.0,
                            width: 10.0,
                            height: 10.0,
                        },
                        VisualQuadStyle {
                            background: thumb_background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 5.0,
                        },
                    ));
                    if loading {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 6 },
                            node: id,
                            bounds: SceneRect {
                                x: track.x + 1.0,
                                y: track.y + 1.0,
                                width: 14.0,
                                height: 14.0,
                            },
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Spinner {
                                phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                color: node.standard_visual_foreground.or(node.style.color),
                            },
                        });
                    }
                    if node.focused {
                        self.insert_primitive(visual_quad(
                            &visual_context,
                            7,
                            SceneRect {
                                x: track.x - 4.0,
                                y: track.y - 4.0,
                                width: track.width + 8.0,
                                height: track.height + 8.0,
                            },
                            VisualQuadStyle {
                                background: None,
                                border_color: node.style.border_color,
                                border_width: 2.0,
                                corner_radius: 12.0,
                            },
                        ));
                    }
                }
                Some(StandardVisual::Slider { ratio }) => {
                    let thumb_extent = 14.0_f32.min(bounds.width).min(bounds.height);
                    let track_inset = thumb_extent / 2.0;
                    let track = SceneRect {
                        x: bounds.x + track_inset,
                        y: bounds.y + (bounds.height - 4.0) / 2.0,
                        width: (bounds.width - thumb_extent).max(0.0),
                        height: 4.0,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        3,
                        track,
                        VisualQuadStyle {
                            background: node.style.border_color,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 2.0,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        4,
                        SceneRect {
                            width: track.width * ratio,
                            ..track
                        },
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 2.0,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        5,
                        SceneRect {
                            x: bounds.x + track.width * ratio,
                            y: bounds.y + (bounds.height - thumb_extent) / 2.0,
                            width: thumb_extent,
                            height: thumb_extent,
                        },
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: node.style.border_color,
                            border_width: 1.0,
                            corner_radius: thumb_extent / 2.0,
                        },
                    ));
                }
                Some(StandardVisual::Range { ratio, size, .. }) => {
                    let ratio = ratio.clamp(0.0, 1.0);
                    let track_band = match node.component_geometry.as_ref() {
                        Some(ComponentGeometry::Range { track, .. }) => scene_rect(*track),
                        _ => SceneRect {
                            x: bounds.x + 7.0,
                            y: bounds.y + (bounds.height - 14.0) / 2.0,
                            width: (bounds.width - 14.0).max(0.0),
                            height: 14.0,
                        },
                    };
                    let thumb_extent = match size {
                        ControlSize::Small => 12.0_f32,
                        ControlSize::Medium => 14.0_f32,
                        ControlSize::Large => 16.0_f32,
                    }
                    .min(bounds.width)
                    .min(track_band.height.max(0.0));
                    let rail = SceneRect {
                        x: track_band.x,
                        y: track_band.y + (track_band.height - 4.0) / 2.0,
                        width: track_band.width,
                        height: 4.0,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        3,
                        rail,
                        VisualQuadStyle {
                            background: node.style.border_color,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 2.0,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        4,
                        SceneRect {
                            width: rail.width * ratio,
                            ..rail
                        },
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: 2.0,
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        5,
                        SceneRect {
                            x: track_band.x + track_band.width * ratio - thumb_extent / 2.0,
                            y: track_band.y + (track_band.height - thumb_extent) / 2.0,
                            width: thumb_extent,
                            height: thumb_extent,
                        },
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: node.style.border_color,
                            border_width: 1.0,
                            corner_radius: thumb_extent / 2.0,
                        },
                    ));
                }
                Some(StandardVisual::Card {
                    loading,
                    loading_phase,
                    ..
                }) => {
                    if loading {
                        let spinner_bounds = match node.component_geometry.as_ref() {
                            Some(ComponentGeometry::Card {
                                spinner: Some(spinner),
                                ..
                            }) => scene_rect(*spinner),
                            _ => {
                                let extent = 20.0_f32.min(bounds.width).min(bounds.height);
                                SceneRect {
                                    x: bounds.x + (bounds.width - extent) / 2.0,
                                    y: bounds.y + (bounds.height - extent) / 2.0,
                                    width: extent,
                                    height: extent,
                                }
                            }
                        };
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: spinner_bounds,
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Spinner {
                                phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                color: node.standard_visual_foreground.or(node.style.color),
                            },
                        });
                    }
                }
                Some(StandardVisual::Spinner { size, phase, .. }) => {
                    let extent = size.max(0.0).min(bounds.width).min(bounds.height);
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 3 },
                        node: id,
                        bounds: SceneRect {
                            x: bounds.x,
                            y: bounds.y + (bounds.height - extent) / 2.0,
                            width: extent,
                            height: extent,
                        },
                        transform,
                        clips: clips.clone(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Spinner {
                            phase: (phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                            color: node.standard_visual_foreground.or(node.style.color),
                        },
                    });
                }
                Some(
                    StandardVisual::ListItem { .. }
                    | StandardVisual::StatusBadge { .. }
                    | StandardVisual::ValidationMessage { .. }
                    | StandardVisual::EmptyState { .. }
                    | StandardVisual::LabeledValue { .. }
                    | StandardVisual::SelectionOption { .. }
                    | StandardVisual::ModalFrame { .. }
                    | StandardVisual::Progress { .. }
                    | StandardVisual::LevelMeter { .. }
                    | StandardVisual::FormField { .. }
                    | StandardVisual::Toast { .. }
                    | StandardVisual::XYPad { .. }
                    | StandardVisual::QrCode { .. }
                    | StandardVisual::Select { .. }
                    | StandardVisual::MenuSurface { .. }
                    | StandardVisual::ActionMenuItem { .. }
                    | StandardVisual::TreeView { .. }
                    | StandardVisual::CommandPalette { .. },
                ) => {
                    // The row surface and fallback label are emitted above;
                    // typed slots remain ordinary retained child nodes.
                }
                None => {}
            }
        }
        self.primitives.len() - before
    }

    fn ancestor_state(&self, node: &ExtractedNode) -> (AffineTransform, f32, Vec<ClipRegion>) {
        let mut ancestors = Vec::new();
        let mut parent = node.parent;
        let mut visited = HashSet::new();
        while let Some(id) = parent.filter(|id| visited.insert(*id)) {
            let Some(node) = self.nodes.get(&id) else {
                break;
            };
            ancestors.push(node);
            parent = node.parent;
        }
        ancestors.reverse();
        let mut transform = AffineTransform::IDENTITY;
        let mut opacity = 1.0;
        let mut clips = Vec::new();
        for ancestor in ancestors {
            let layout = ancestor.layout;
            let local = ancestor
                .source_style
                .layout
                .transform
                .map(|value| {
                    AffineTransform(value.around_center(
                        layout.x,
                        layout.y,
                        layout.width,
                        layout.height,
                    ))
                })
                .unwrap_or_default();
            transform = transform.then(local);
            opacity *= ancestor
                .source_style
                .layout
                .opacity
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            if ancestor.source_style.layout.clips_overflow() {
                clips.push(ClipRegion {
                    bounds: SceneRect {
                        x: layout.x,
                        y: layout.y,
                        width: layout.width,
                        height: layout.height,
                    },
                    transform,
                });
            }
            if let Some(ComponentGeometry::EmptyState { root_clip, .. }) =
                ancestor.component_geometry.as_ref()
            {
                clips.push(ClipRegion {
                    bounds: scene_rect(*root_clip),
                    transform,
                });
            }
            if let Some(ComponentGeometry::ModalFrame { surface, body, .. }) =
                ancestor.component_geometry.as_ref()
            {
                let focus_inset = if node.focused { 3.0 } else { 0.0 };
                clips.push(ClipRegion {
                    bounds: SceneRect {
                        x: surface.x - focus_inset,
                        y: surface.y - focus_inset,
                        width: surface.width + focus_inset * 2.0,
                        height: surface.height + focus_inset * 2.0,
                    },
                    transform,
                });
                if let Some(StandardVisual::ModalFrame { slots, .. }) =
                    ancestor.standard_visual.as_ref()
                    && slots
                        .body
                        .is_some_and(|body_root| self.node_descends_from(node.id, body_root))
                {
                    clips.push(ClipRegion {
                        bounds: scene_rect(*body),
                        transform,
                    });
                }
            }
            transform = transform.then(AffineTransform([
                1.0,
                0.0,
                0.0,
                1.0,
                -ancestor.scroll_offset.x,
                -ancestor.scroll_offset.y,
            ]));
        }
        (transform, opacity, clips)
    }

    fn remove_node_primitives(&mut self, id: StableNodeId) {
        for slot in 0..=MAX_NODE_PRIMITIVE_SLOT {
            if let Some(primitive) = self.primitives.remove(&PrimitiveId { node: id, slot }) {
                self.ordered.remove(&Self::order_key(&primitive));
            }
        }
    }

    fn node_descends_from(&self, id: StableNodeId, ancestor: StableNodeId) -> bool {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.nodes.get(&candidate).and_then(|node| node.parent);
        }
        false
    }

    fn insert_primitive(&mut self, primitive: ScenePrimitive) {
        self.primitives.insert(primitive.id, primitive);
    }

    fn order_key(primitive: &ScenePrimitive) -> SceneOrderKey {
        SceneOrderKey {
            z_index: primitive.z_index,
            document_order: primitive.document_order,
            slot: primitive.id.slot,
            node: primitive.node,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scene_rect(bounds: LayoutBox) -> SceneRect {
    SceneRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width.max(0.0),
        height: bounds.height.max(0.0),
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_select_handle(
    scene: &mut UiScene,
    id: StableNodeId,
    handle: &LayoutBox,
    color: [f32; 4],
    transform: AffineTransform,
    clips: &[ClipRegion],
    opacity: f32,
    z_index: i32,
    document_order: usize,
) {
    let center_x = handle.x + handle.width / 2.0;
    let center_y = handle.y + handle.height / 2.0;
    let widths = [8.0, 6.0, 4.0, 2.0];
    for (index, width) in widths.iter().copied().enumerate() {
        scene.insert_primitive(visual_quad(
            &VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index,
                document_order,
            },
            3 + index as u8,
            SceneRect {
                x: center_x - width / 2.0,
                y: center_y - 1.5 + index as f32,
                width,
                height: 1.0,
            },
            VisualQuadStyle {
                background: Some(color),
                border_color: None,
                border_width: 0.0,
                corner_radius: 0.0,
            },
        ));
    }
}

fn component_text_primitive(
    id: StableNodeId,
    slot: u8,
    region: &ComponentTextRegion,
    horizontal_alignment: TextHorizontalAlignment,
    ellipsis: bool,
    node: &ExtractedNode,
    transform: AffineTransform,
    clips: Vec<ClipRegion>,
    opacity: f32,
    document_order: usize,
) -> ScenePrimitive {
    let multiline = matches!(
        node.component_geometry,
        Some(ComponentGeometry::TextInput {
            multiline: true,
            ..
        })
    );
    let intrinsic_multiline = matches!(
        node.standard_visual,
        Some(StandardVisual::EmptyState { .. } | StandardVisual::ModalFrame { .. })
    );
    ScenePrimitive {
        id: PrimitiveId { node: id, slot },
        node: id,
        bounds: scene_rect(region.bounds),
        transform,
        clips,
        opacity,
        z_index: node.z_index,
        document_order,
        kind: ScenePrimitiveKind::Text {
            content: region.content.to_string(),
            color: region.color.or(node.style.color),
            size: region.font_size,
            weight: region.font_weight,
            family: node.style.font_family.as_deref().map(str::to_owned),
            line_height: node.style.line_height,
            letter_spacing: node.style.letter_spacing,
            wrap: multiline || intrinsic_multiline,
            ellipsis,
            shaping: if node.text_input.is_some() {
                TextShaping::Advanced
            } else {
                TextShaping::Auto
            },
            horizontal_alignment,
            vertical_alignment: if multiline || intrinsic_multiline {
                TextVerticalAlignment::Top
            } else {
                TextVerticalAlignment::Center
            },
            spans: scene_text_spans(node, region.content.as_ref()),
        },
    }
}

fn scene_text_spans(node: &ExtractedNode, content: &str) -> Vec<SceneTextSpan> {
    if node.text_spans.is_empty() {
        return Vec::new();
    }
    let Some(source) = node.text.as_ref() else {
        return Vec::new();
    };
    if source.value != content {
        return Vec::new();
    }
    node.text_spans
        .iter()
        .filter(|span| {
            span.start < span.end
                && span.end <= content.len()
                && content.is_char_boundary(span.start)
                && content.is_char_boundary(span.end)
        })
        .map(|span| SceneTextSpan {
            start: span.start,
            end: span.end,
            color: span.color,
        })
        .collect()
}

struct VisualPrimitiveContext<'a> {
    node: StableNodeId,
    transform: AffineTransform,
    clips: &'a [ClipRegion],
    opacity: f32,
    z_index: i32,
    document_order: usize,
}

struct VisualQuadStyle {
    background: Option<[f32; 4]>,
    border_color: Option<[f32; 4]>,
    border_width: f32,
    corner_radius: f32,
}

fn visual_quad(
    context: &VisualPrimitiveContext<'_>,
    slot: u8,
    bounds: SceneRect,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: context.clips.to_vec(),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: ScenePrimitiveKind::Quad {
            background: style.background,
            border_color: style.border_color,
            border_width: style.border_width,
            corner_radius: style.corner_radius,
            shadow: None,
        },
    }
}

fn visual_quad_batch(
    context: &VisualPrimitiveContext<'_>,
    slot: u8,
    bounds: impl IntoIterator<Item = SceneRect>,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    let quad_bounds = bounds.into_iter().collect::<Vec<_>>();
    debug_assert!(!quad_bounds.is_empty());
    let bounds = quad_bounds
        .iter()
        .copied()
        .reduce(|left, right| {
            let x = left.x.min(right.x);
            let y = left.y.min(right.y);
            let right_edge = (left.x + left.width).max(right.x + right.width);
            let bottom_edge = (left.y + left.height).max(right.y + right.height);
            SceneRect {
                x,
                y,
                width: right_edge - x,
                height: bottom_edge - y,
            }
        })
        .unwrap_or_default();
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: context.clips.to_vec(),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: ScenePrimitiveKind::QuadBatch {
            bounds: quad_bounds,
            background: style.background,
            border_color: style.border_color,
            border_width: style.border_width,
            corner_radius: style.corner_radius,
            shadow: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nana_ui_runtime::{
        ComputedStyle, CustomRenderNode, LayoutBox, NodeKind, NodeStyle, TextContent,
    };

    use super::*;

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn node(value: u64, parent: Option<u64>, children: &[u64]) -> ExtractedNode {
        ExtractedNode {
            id: id(value),
            kind: NodeKind::Element { tag: "div".into() },
            parent: parent.map(id),
            children: children.iter().copied().map(id).collect(),
            layout: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle::default(),
            style: ComputedStyle::default(),
            text: None,
            text_metrics: None,
            z_index: 0,
            focused: false,
            ime: None,
            text_input: None,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: None,
        }
    }

    #[test]
    fn extracted_text_spans_travel_on_the_text_primitive() {
        let mut labeled = node(1, None, &[]);
        labeled.text = Some(TextContent {
            value: "fn main".into(),
        });
        labeled.text_spans = vec![nana_ui_runtime::ExtractedTextSpan {
            start: 0,
            end: 2,
            color: [0.2, 0.6, 1.0, 1.0],
        }];
        let mut scene = UiScene::new();
        scene.apply_delta([labeled], []);
        let Some(ScenePrimitiveKind::Text {
            ref content,
            ref spans,
            ..
        }) = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .map(|primitive| primitive.kind.clone())
        else {
            panic!("expected a text primitive");
        };
        assert_eq!(content, "fn main");
        assert_eq!(
            spans,
            &vec![SceneTextSpan {
                start: 0,
                end: 2,
                color: [0.2, 0.6, 1.0, 1.0],
            }]
        );
    }

    #[test]
    fn extraction_preserves_custom_interleaving_and_removals() {
        let mut root = node(1, None, &[2]);
        root.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.1, 0.2, 0.3, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.custom_render = Some(CustomRenderNode {
            renderer: Arc::from("host-texture"),
            resource: Arc::from("preview"),
            revision: 3,
        });
        child.text = Some(TextContent {
            value: "caption".into(),
        });
        let mut scene = UiScene::new();
        let delta = scene.apply_delta([root, child], []);
        assert_eq!(delta.primitive_count, 3);
        let graph = scene.frame_graph(ResourceId(7)).unwrap();
        assert_eq!(
            graph
                .passes
                .iter()
                .flat_map(|pass| pass.operations.iter().cloned())
                .collect::<Vec<_>>(),
            vec![
                RenderOperation::PrepareExternal(PrimitiveId {
                    node: id(2),
                    slot: 1
                }),
                RenderOperation::Draw(PrimitiveId {
                    node: id(1),
                    slot: 0
                }),
                RenderOperation::InvokeCustom(PrimitiveId {
                    node: id(2),
                    slot: 1
                }),
                RenderOperation::Draw(PrimitiveId {
                    node: id(2),
                    slot: 2
                }),
            ]
        );
        assert_eq!(graph.passes.len(), 4);
        assert_eq!(graph.resources.len(), 2);
        assert_eq!(graph.resources[1].label, "preview");
        assert_eq!(graph.passes[0].label, "prepare:preview");
        assert_eq!(graph.passes[2].resources.len(), 2);
        assert!(graph.passes[2].dependencies.contains(&graph.passes[0].id));
        let root_before = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .unwrap()
            .clone();
        let mut changed_child = node(2, Some(1), &[]);
        changed_child.custom_render = Some(CustomRenderNode {
            renderer: Arc::from("host-texture"),
            resource: Arc::from("preview"),
            revision: 4,
        });
        let delta = scene.apply_delta([changed_child], []);
        assert!(!delta.order_rebuilt);
        assert_eq!(delta.rebuilt_primitives, 1);
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 0,
                })
                .unwrap(),
            &root_before,
            "a local extraction must not rebuild unrelated primitives"
        );
        let delta = scene.apply_delta([], [id(2)]);
        assert_eq!(delta.removed_nodes, 1);
        assert_eq!(delta.primitive_count, 1);
    }

    #[test]
    fn selection_option_emits_surface_text_icon_and_focus_slots() {
        let mut option = node(7, None, &[]);
        option.layout = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 96.0,
            height: 26.0,
        };
        option.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.2, 0.2, 0.2, 1.0]),
                border_radius: Some(7.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        option.standard_visual = Some(StandardVisual::SelectionOption {
            label: Arc::from("Preview"),
            icon: Some(nana_ui_core::Icon::Search),
            selected: true,
            disabled: false,
            size: ControlSize::Medium,
            show_focus_ring: true,
        });
        option.component_geometry = Some(ComponentGeometry::SelectionOption {
            icon: Some((
                nana_ui_core::Icon::Search,
                LayoutBox {
                    x: 20.0,
                    y: 26.0,
                    width: 14.0,
                    height: 14.0,
                },
                [0.8, 0.8, 0.8, 1.0],
            )),
            label: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 39.0,
                    y: 20.0,
                    width: 57.0,
                    height: 26.0,
                },
                content: Arc::from("Preview"),
                color: Some([1.0, 1.0, 1.0, 1.0]),
                font_size: 13.0,
                font_weight: Some(500),
            },
            focus_ring: Some([0.2, 0.6, 1.0, 1.0]),
        });
        let mut scene = UiScene::new();
        scene.apply_delta([option], []);
        for slot in [0, 2, 3, 7] {
            assert!(scene.primitive(PrimitiveId { node: id(7), slot }).is_some());
        }
        let focus = scene
            .primitive(PrimitiveId {
                node: id(7),
                slot: 7,
            })
            .unwrap();
        assert_eq!(
            focus.bounds,
            SceneRect {
                x: 6.0,
                y: 16.0,
                width: 104.0,
                height: 34.0,
            }
        );
        assert!(matches!(
            focus.kind,
            ScenePrimitiveKind::Quad {
                border_width: 2.0,
                corner_radius: 11.0,
                ..
            }
        ));
        assert!(matches!(
            scene.primitive(PrimitiveId { node: id(7), slot: 2 }).unwrap().kind,
            ScenePrimitiveKind::Text { ref content, wrap: false, .. } if content == "Preview"
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(7),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Icon {
                icon: nana_ui_core::Icon::Search,
                ..
            }
        ));
    }

    #[test]
    fn frame_graph_rejects_conflicting_revisions_of_one_external_resource() {
        let mut first = node(1, None, &[]);
        first.custom_render = Some(CustomRenderNode {
            renderer: Arc::from("nana.host-texture"),
            resource: Arc::from("program"),
            revision: 7,
        });
        let mut second = node(2, None, &[]);
        second.custom_render = Some(CustomRenderNode {
            renderer: Arc::from("nana.host-texture"),
            resource: Arc::from("program"),
            revision: 8,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([first, second], []);

        assert_eq!(
            scene.frame_graph(ResourceId(1)),
            Err(GraphError::ConflictingExternalResource("program".into()))
        );
    }

    #[test]
    fn ancestor_clip_transform_and_opacity_are_composed() {
        let mut root = node(1, None, &[2]);
        root.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                overflow_x: nana_ui_core::OverflowSpec::Hidden,
                opacity: Some(0.5),
                transform: Some(nana_ui_core::PaintTransform {
                    e: 4.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.custom_render = Some(CustomRenderNode {
            renderer: Arc::from("test"),
            resource: Arc::from("resource"),
            revision: 0,
        });
        child.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                opacity: Some(0.5),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([root, child], []);
        let custom = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom(_)))
            .unwrap();
        assert_eq!(custom.opacity, 0.25);
        assert_eq!(custom.clips.len(), 1);
        assert_eq!(custom.transform.0[4], 4.0);
    }

    #[test]
    fn text_primitive_preserves_content_box_and_paint_semantics() {
        let mut text = node(1, None, &[]);
        text.text = Some(TextContent {
            value: "Build".into(),
        });
        text.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                padding_top: Some(nana_ui_core::LengthSpec::Px(2.0)),
                padding_right: Some(nana_ui_core::LengthSpec::Px(8.0)),
                padding_bottom: Some(nana_ui_core::LengthSpec::Px(4.0)),
                padding_left: Some(nana_ui_core::LengthSpec::Px(10.0)),
                border_width: Some(1.0),
                white_space_nowrap: true,
                text_overflow_ellipsis: true,
                ..Default::default()
            }),
            text_horizontal_alignment: TextHorizontalAlignment::Center,
            text_vertical_alignment: TextVerticalAlignment::Center,
            ..Default::default()
        };
        text.style.line_height = Some(LineHeightSpec::Absolute(18.0));

        let mut scene = UiScene::new();
        scene.apply_delta([text], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .unwrap();
        assert_eq!(
            primitive.bounds,
            SceneRect {
                x: 11.0,
                y: 3.0,
                width: 80.0,
                height: 72.0,
            }
        );
        assert!(matches!(
            primitive.kind,
            ScenePrimitiveKind::Text {
                line_height: Some(LineHeightSpec::Absolute(18.0)),
                wrap: false,
                ellipsis: true,
                horizontal_alignment: TextHorizontalAlignment::Center,
                vertical_alignment: TextVerticalAlignment::Center,
                ..
            }
        ));
    }

    #[test]
    fn text_input_geometry_paints_selection_text_caret_preedit_and_focus_in_order() {
        let mut input = node(1, None, &[]);
        input.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.1, 0.1, 0.1, 1.0]),
                border_width: Some(1.0),
                border_color: Some([0.3, 0.3, 0.3, 1.0]),
                border_radius: Some(6.0),
                overflow_x: nana_ui_core::OverflowSpec::Hidden,
                opacity: Some(0.5),
                transform: Some(nana_ui_core::PaintTransform {
                    e: 4.0,
                    f: 6.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 84.0,
                    height: 32.0,
                },
                content: Arc::from("release/next"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: (0..10)
                .map(|line| LayoutBox {
                    x: 8.0,
                    y: 8.0 + line as f32 * 16.0,
                    width: 40.0 + line as f32,
                    height: 16.0,
                })
                .collect(),
            caret: Some(LayoutBox {
                x: 48.0,
                y: 8.0,
                width: 1.0,
                height: 16.0,
            }),
            preedit: vec![
                LayoutBox {
                    x: 48.0,
                    y: 23.0,
                    width: 18.0,
                    height: 1.0,
                },
                LayoutBox {
                    x: 8.0,
                    y: 39.0,
                    width: 24.0,
                    height: 1.0,
                },
            ],
            background: Some([0.1, 0.1, 0.1, 1.0]),
            border: Some([0.3, 0.3, 0.3, 1.0]),
            border_width: 1.0,
            focus_ring: Some([0.2, 0.6, 1.0, 1.0]),
            selection_color: [0.2, 0.4, 0.7, 0.4],
            caret_color: [0.2, 0.6, 1.0, 1.0],
            preedit_color: [0.2, 0.6, 1.0, 1.0],
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        assert_eq!(scene.primitives().count(), 6);
        for slot in [0, 1, 2, 4, 5, 7] {
            assert!(scene.primitive(PrimitiveId { node: id(1), slot }).is_some());
        }
        assert!(matches!(
            scene.primitive(PrimitiveId { node: id(1), slot: 2 }).unwrap().kind,
            ScenePrimitiveKind::Text {
                ref content,
                wrap: true,
                vertical_alignment: TextVerticalAlignment::Top,
                ..
            } if content == "release/next"
        ));
        for slot in [1, 5] {
            let primitive = scene.primitive(PrimitiveId { node: id(1), slot }).unwrap();
            assert_eq!(primitive.transform.0[4..], [4.0, 6.0]);
            assert_eq!(primitive.opacity, 0.5);
            assert_eq!(primitive.clips.len(), 2);
            let expected_count = if slot == 1 { 10 } else { 2 };
            assert!(matches!(
                primitive.kind,
                ScenePrimitiveKind::QuadBatch { ref bounds, .. }
                    if bounds.len() == expected_count
            ));
        }

        let mut single_line = node(2, None, &[]);
        single_line.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
        });
        single_line.component_geometry = input_component_geometry(false);
        scene.apply_delta([single_line], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: false,
                vertical_alignment: TextVerticalAlignment::Center,
                ..
            }
        ));
    }

    fn input_component_geometry(multiline: bool) -> Option<ComponentGeometry> {
        Some(ComponentGeometry::TextInput {
            multiline,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 84.0,
                    height: 32.0,
                },
                content: Arc::from("release/next"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            preedit: Vec::new(),
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.2, 0.4, 0.7, 0.4],
            caret_color: [0.2, 0.6, 1.0, 1.0],
            preedit_color: [0.2, 0.6, 1.0, 1.0],
        })
    }

    #[test]
    fn feedback_geometry_emits_semantic_quad_text_and_icon_primitives() {
        let region = |content: &'static str, y, size, weight| ComponentTextRegion {
            bounds: LayoutBox {
                x: 20.0,
                y,
                width: 120.0,
                height: 18.0,
            },
            content: Arc::from(content),
            color: Some([0.8, 0.9, 1.0, 1.0]),
            font_size: size,
            font_weight: weight,
        };
        let mut badge = node(1, None, &[]);
        badge.source_style.layout = Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.1, 0.2, 0.3, 1.0]),
            ..Default::default()
        });
        badge.standard_visual = Some(StandardVisual::StatusBadge {
            label: Arc::from("Online"),
            tone: nana_ui_core::StatusTone::Success,
            compact: true,
        });
        badge.component_geometry = Some(ComponentGeometry::StatusBadge {
            indicator: LayoutBox {
                x: 8.0,
                y: 8.0,
                width: 3.0,
                height: 3.0,
            },
            label: region("Online", 0.0, 11.0, Some(500)),
            background: [0.1, 0.8, 0.4, 0.12],
            foreground: [0.1, 0.8, 0.4, 1.0],
        });

        let mut validation = node(2, None, &[]);
        validation.standard_visual = Some(StandardVisual::ValidationMessage {
            message: Arc::from("Required"),
            intent: nana_ui_core::ValidationIntent::Danger,
            compact: true,
        });
        validation.component_geometry = Some(ComponentGeometry::ValidationMessage {
            indicator: LayoutBox {
                x: 4.0,
                y: 7.0,
                width: 5.0,
                height: 5.0,
            },
            label: region("Required", 0.0, 11.0, None),
            foreground: [0.9, 0.2, 0.2, 1.0],
        });

        let mut empty = node(3, None, &[]);
        empty.standard_visual = Some(StandardVisual::EmptyState {
            title: Arc::from("No files"),
            message: Some(Arc::from("Create one")),
            icon: Some(nana_ui_core::Icon::Folder),
            compact: false,
            action: None,
        });
        empty.component_geometry = Some(ComponentGeometry::EmptyState {
            root_clip: empty.layout,
            content_clip: LayoutBox {
                x: 16.0,
                y: 24.0,
                width: 68.0,
                height: 52.0,
            },
            icon: Some((
                nana_ui_core::Icon::Folder,
                LayoutBox {
                    x: 40.0,
                    y: 2.0,
                    width: 22.0,
                    height: 22.0,
                },
                [0.5, 0.5, 0.5, 1.0],
            )),
            title: region("No files", 26.0, 13.0, Some(600)),
            message: Some(region("Create one", 46.0, 12.0, None)),
            action: None,
        });

        let mut labeled = node(4, None, &[]);
        labeled.standard_visual = Some(StandardVisual::LabeledValue {
            label: Arc::from("Revision"),
            value: Arc::from("42"),
            value_role: nana_ui_core::SemanticColorRole::Text,
            value_weight: 600,
            compact: true,
            action: None,
        });
        labeled.component_geometry = Some(ComponentGeometry::LabeledValue {
            label: region("Revision", 0.0, 11.0, None),
            value: region("42", 14.0, 12.0, Some(600)),
            action: None,
        });

        let mut compact_empty = node(5, None, &[]);
        compact_empty.standard_visual = Some(StandardVisual::EmptyState {
            title: Arc::from("空状态"),
            message: None,
            icon: None,
            compact: true,
            action: None,
        });
        compact_empty.component_geometry = Some(ComponentGeometry::EmptyState {
            root_clip: compact_empty.layout,
            content_clip: LayoutBox {
                x: 6.0,
                y: 8.0,
                width: 88.0,
                height: 84.0,
            },
            icon: None,
            title: region("空状态", 2.0, 12.0, Some(600)),
            message: None,
            action: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([badge, validation, empty, labeled, compact_empty], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 0
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some([_, _, _, 0.12]),
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some(_),
                border_width: 0.0,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: None,
                border_width: 1.0,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Icon {
                icon: nana_ui_core::Icon::Folder,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 4
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { size: 12.0, .. }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                vertical_alignment: TextVerticalAlignment::Top,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(5),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                vertical_alignment: TextVerticalAlignment::Top,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                size: 11.0,
                weight: None,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                size: 12.0,
                weight: Some(600),
                ..
            }
        ));
    }

    #[test]
    fn modal_frame_emits_distinct_scrim_surface_and_intrinsic_text_slots() {
        let mut modal = node(50, None, &[]);
        modal.standard_visual = Some(StandardVisual::ModalFrame {
            title: Arc::from("Delete project"),
            description: Some(Arc::from("This cannot be undone")),
            body_text: None,
            kind: nana_ui_runtime::ModalSurfaceKind::Confirm(nana_ui_core::DialogSize::Compact),
            busy: false,
            danger: false,
            slots: nana_ui_runtime::ModalSlots::default(),
        });
        let text = |content: &'static str, y, height, size, weight| ComponentTextRegion {
            bounds: LayoutBox {
                x: 206.0,
                y,
                width: 388.0,
                height,
            },
            content: Arc::from(content),
            color: Some([1.0; 4]),
            font_size: size,
            font_weight: weight,
        };
        modal.component_geometry = Some(ComponentGeometry::ModalFrame {
            scrim: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            surface: LayoutBox {
                x: 190.0,
                y: 72.0,
                width: 420.0,
                height: 180.0,
            },
            body: LayoutBox {
                x: 206.0,
                y: 130.0,
                width: 388.0,
                height: 80.0,
            },
            title: text("Delete project", 86.0, 20.0, 14.0, Some(600)),
            description: Some(text("This cannot be undone", 110.0, 18.0, 12.0, None)),
            body_text: None,
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
            elevation: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.24],
                offset_y: 8.0,
                blur_radius: 24.0,
            },
        });
        let mut scene = UiScene::default();
        scene.apply_delta([modal], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 10
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some([0.0, 0.0, 0.0, 0.45]),
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 11
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                border_color: None,
                border_width: 0.0,
                corner_radius,
                shadow: Some(_),
                ..
            } if (corner_radius - UI_METRICS.radius_md).abs() < f32::EPSILON
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 12
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                ..
            }
        ));
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 13
                })
                .is_some()
        );
    }

    #[test]
    fn docked_drawer_extends_the_flush_edge_so_clipping_squares_that_side() {
        let mut drawer = node(52, None, &[]);
        drawer.standard_visual = Some(StandardVisual::ModalFrame {
            title: Arc::from("Inspector"),
            description: None,
            body_text: None,
            kind: nana_ui_runtime::ModalSurfaceKind::Drawer(DrawerSide::Right),
            busy: false,
            danger: false,
            slots: nana_ui_runtime::ModalSlots::default(),
        });
        drawer.component_geometry = Some(ComponentGeometry::ModalFrame {
            scrim: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 420.0,
                height: 240.0,
            },
            surface: LayoutBox {
                x: 60.0,
                y: 0.0,
                width: 360.0,
                height: 240.0,
            },
            body: LayoutBox {
                x: 76.0,
                y: 64.0,
                width: 328.0,
                height: 160.0,
            },
            title: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 76.0,
                    y: 14.0,
                    width: 280.0,
                    height: 17.0,
                },
                content: Arc::from("Inspector"),
                color: Some([1.0; 4]),
                font_size: 14.0,
                font_weight: Some(600),
            },
            description: None,
            body_text: None,
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.0; 4],
            elevation: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.45],
                offset_y: 14.0,
                blur_radius: 30.0,
            },
        });
        let mut scene = UiScene::default();
        scene.apply_delta([drawer], []);
        let surface = scene
            .primitive(PrimitiveId {
                node: id(52),
                slot: 11,
            })
            .unwrap();
        assert!((surface.bounds.width - (360.0 + UI_METRICS.radius_md)).abs() < f32::EPSILON);
        assert!((surface.bounds.x - 60.0).abs() < f32::EPSILON);
        assert_eq!(surface.clips.len(), 1);
        assert!((surface.clips[0].bounds.width - 420.0).abs() < f32::EPSILON);
    }

    #[test]
    fn confirm_action_scene_restores_label_after_busy_spinner_clears() {
        let mut action = node(51, None, &[]);
        let label = ComponentTextRegion {
            bounds: LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 24.0,
            },
            content: Arc::from("Delete"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: Some(500),
        };
        action.standard_visual = Some(StandardVisual::Button {
            label: Arc::from("Delete"),
            kind: nana_ui_core::ButtonKind::Danger,
            size: nana_ui_core::ControlSize::Medium,
            loading: true,
            loading_phase: 0.5,
            invalid: false,
        });
        action.component_geometry = Some(ComponentGeometry::Button {
            label: label.clone(),
            spinner: Some(LayoutBox {
                x: 42.0,
                y: 14.0,
                width: 16.0,
                height: 16.0,
            }),
            background: Some([0.8, 0.1, 0.1, 1.0]),
            border: None,
            border_width: 0.0,
            focus_ring: None,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([action.clone()], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(51),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Spinner { .. }
        ));

        action.standard_visual = Some(StandardVisual::Button {
            label: Arc::from("Delete"),
            kind: nana_ui_core::ButtonKind::Primary,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        });
        action.component_geometry = Some(ComponentGeometry::Button {
            label,
            spinner: None,
            background: Some([0.2, 0.4, 0.8, 1.0]),
            border: None,
            border_width: 0.0,
            focus_ring: None,
        });
        scene.apply_delta([action], []);
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(51),
                    slot: 3
                })
                .is_none()
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(51),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { ref content, .. } if content == "Delete"
        ));
    }

    #[test]
    fn empty_state_separates_intrinsic_clip_from_focused_action_root_clip() {
        let content_clip = LayoutBox {
            x: 8.0,
            y: 9.0,
            width: 1.0,
            height: 2.0,
        };
        let mut empty = node(10, None, &[11]);
        empty.standard_visual = Some(StandardVisual::EmptyState {
            title: Arc::from("Title"),
            message: Some(Arc::from("Message")),
            icon: Some(nana_ui_core::Icon::Folder),
            compact: false,
            action: Some(id(11)),
        });
        empty.component_geometry = Some(ComponentGeometry::EmptyState {
            root_clip: empty.layout,
            content_clip,
            icon: Some((
                nana_ui_core::Icon::Folder,
                LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 22.0,
                    height: 22.0,
                },
                [0.5, 0.5, 0.5, 1.0],
            )),
            title: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 22.0,
                    width: 40.0,
                    height: 30.0,
                },
                content: Arc::from("Title"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: Some(600),
            },
            message: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 58.0,
                    width: 60.0,
                    height: 40.0,
                },
                content: Arc::from("Message"),
                color: Some([0.7; 4]),
                font_size: 12.0,
                font_weight: None,
            }),
            action: Some(LayoutBox {
                x: 20.0,
                y: 53.0,
                width: 60.0,
                height: 24.0,
            }),
        });
        let mut action = node(11, Some(10), &[]);
        action.layout = LayoutBox {
            x: 20.0,
            y: 53.0,
            width: 60.0,
            height: 24.0,
        };
        action.focused = true;
        action.standard_visual = Some(StandardVisual::Button {
            label: Arc::from("Action"),
            kind: nana_ui_core::ButtonKind::Primary,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        });
        action.component_geometry = Some(ComponentGeometry::Button {
            label: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 20.0,
                    y: 53.0,
                    width: 60.0,
                    height: 24.0,
                },
                content: Arc::from("Action"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            spinner: None,
            background: Some([0.2, 0.4, 0.8, 1.0]),
            border: None,
            border_width: 0.0,
            focus_ring: Some([0.3, 0.6, 1.0, 1.0]),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([empty, action], []);
        let content = ClipRegion {
            bounds: scene_rect(content_clip),
            transform: AffineTransform::IDENTITY,
        };
        for primitive in [
            PrimitiveId {
                node: id(10),
                slot: 2,
            },
            PrimitiveId {
                node: id(10),
                slot: 3,
            },
            PrimitiveId {
                node: id(10),
                slot: 4,
            },
        ] {
            assert!(scene.primitive(primitive).unwrap().clips.contains(&content));
        }
        let root = ClipRegion {
            bounds: SceneRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            transform: AffineTransform::IDENTITY,
        };
        for primitive in [
            PrimitiveId {
                node: id(11),
                slot: 2,
            },
            PrimitiveId {
                node: id(11),
                slot: 7,
            },
        ] {
            let primitive = scene.primitive(primitive).unwrap();
            assert!(primitive.clips.contains(&root));
            assert!(!primitive.clips.contains(&content));
        }
        let ring = scene
            .primitive(PrimitiveId {
                node: id(11),
                slot: 7,
            })
            .unwrap();
        assert_eq!(ring.bounds.y + ring.bounds.height, root.bounds.height);
    }

    #[test]
    fn standard_control_visuals_expand_without_backend_tag_matching() {
        let mut checkbox = node(1, None, &[]);
        checkbox.text = Some(TextContent {
            value: "Notifications".into(),
        });
        checkbox.standard_visual = Some(StandardVisual::Checkbox { checked: true });
        checkbox.style.background = Some([0.2, 0.5, 0.9, 1.0]);
        checkbox.style.border_color = Some([0.1, 0.2, 0.3, 1.0]);

        let mut slider = node(2, None, &[]);
        slider.standard_visual = Some(StandardVisual::Slider { ratio: 0.25 });
        slider.style.background = Some([0.2, 0.5, 0.9, 1.0]);
        slider.style.border_color = Some([0.4, 0.4, 0.4, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([checkbox, slider], []);
        assert_eq!(scene.primitives().count(), 6);
        let checkbox_text = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .unwrap();
        assert_eq!(checkbox_text.bounds.x, 24.0);
        assert_eq!(checkbox_text.bounds.width, 76.0);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 4,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { ref content, .. } if content == "✓"
        ));
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 4,
                })
                .unwrap()
                .bounds
                .width,
            21.5
        );
    }

    #[test]
    fn migrated_components_consume_runtime_subregion_geometry() {
        let mut icon = node(1, None, &[]);
        icon.layout = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 32.0,
            height: 32.0,
        };
        icon.standard_visual = Some(StandardVisual::Icon {
            icon: nana_ui_core::Icon::Search,
            size: 16.0,
            tooltip: None,
        });
        icon.style.background = Some([0.2, 0.3, 0.4, 1.0]);
        icon.style.border_color = Some([0.4, 0.5, 0.6, 1.0]);
        icon.style.color = Some([0.9, 0.9, 0.9, 1.0]);
        icon.standard_visual_foreground = Some([0.1, 0.6, 0.9, 1.0]);

        let mut switch = node(2, None, &[]);
        switch.standard_visual = Some(StandardVisual::Switch {
            label: Arc::from("Enabled"),
            hint: Some(Arc::from("Starts with the workspace")),
            checked: true,
            control_position: nana_ui_core::SwitchControlPosition::End,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        });
        switch.component_geometry = Some(ComponentGeometry::Switch {
            label: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 8.0,
                    width: 100.0,
                    height: 18.0,
                },
                content: Arc::from("Enabled"),
                color: Some([0.9, 0.9, 0.9, 1.0]),
                font_size: 13.0,
                font_weight: Some(500),
            },
            hint: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 28.0,
                    width: 150.0,
                    height: 16.0,
                },
                content: Arc::from("Starts with the workspace"),
                color: Some([0.6, 0.6, 0.6, 1.0]),
                font_size: 12.0,
                font_weight: Some(400),
            }),
            control: LayoutBox {
                x: 170.0,
                y: 18.0,
                width: 30.0,
                height: 16.0,
            },
            track_background: [0.2, 0.5, 0.9, 1.0],
            track_border: [0.1, 0.4, 0.8, 1.0],
            thumb_background: [1.0, 1.0, 1.0, 1.0],
        });
        switch.layout.width = 200.0;
        switch.layout.height = 52.0;
        switch.focused = true;

        let mut range = node(3, None, &[]);
        range.standard_visual = Some(StandardVisual::Range {
            label: Some(Arc::from("Opacity")),
            value: Arc::from("25"),
            unit: Some(Arc::from("%")),
            size: nana_ui_core::ControlSize::Medium,
            ratio: 0.25,
            invalid: false,
        });
        range.component_geometry = Some(ComponentGeometry::Range {
            label: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 11.0,
                    width: 52.0,
                    height: 18.0,
                },
                content: Arc::from("Opacity"),
                color: None,
                font_size: 13.0,
                font_weight: Some(500),
            }),
            value: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 210.0,
                    y: 11.0,
                    width: 20.0,
                    height: 18.0,
                },
                content: Arc::from("25"),
                color: None,
                font_size: 13.0,
                font_weight: None,
            },
            unit: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 232.0,
                    y: 11.0,
                    width: 8.0,
                    height: 18.0,
                },
                content: Arc::from("%"),
                color: None,
                font_size: 13.0,
                font_weight: None,
            }),
            track: LayoutBox {
                x: 80.0,
                y: 12.0,
                width: 120.0,
                height: 16.0,
            },
        });
        range.layout.width = 240.0;
        range.layout.height = 40.0;
        range.style.background = Some([0.2, 0.5, 0.9, 1.0]);
        range.style.border_color = Some([0.4, 0.4, 0.4, 1.0]);

        let mut card = node(4, None, &[]);
        card.standard_visual = Some(StandardVisual::Card {
            title: Some(Arc::from("Actions")),
            kind: nana_ui_core::CardKind::Surface,
            loading: true,
            loading_phase: 0.5,
        });
        card.style.background = Some([0.12, 0.12, 0.12, 1.0]);
        card.style.border_color = Some([0.3, 0.3, 0.3, 1.0]);
        card.component_geometry = Some(ComponentGeometry::Card {
            title: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 10.0,
                    y: 8.0,
                    width: 50.0,
                    height: 18.0,
                },
                content: Arc::from("Actions"),
                color: None,
                font_size: 13.0,
                font_weight: Some(600),
            }),
            content: LayoutBox {
                x: 10.0,
                y: 36.0,
                width: 80.0,
                height: 34.0,
            },
            elevation: Some(ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.25],
                offset_y: 3.0,
                blur_radius: 8.0,
            }),
            spinner: Some(LayoutBox {
                x: 68.0,
                y: 10.0,
                width: 14.0,
                height: 14.0,
            }),
        });

        let mut list_item = node(5, None, &[]);
        list_item.text = Some(TextContent {
            value: "Project".into(),
        });
        list_item.standard_visual = Some(StandardVisual::ListItem {
            leading: None,
            content: None,
            trailing: None,
        });
        list_item.component_geometry = Some(ComponentGeometry::ListItem {
            leading: None,
            content: Some(LayoutBox {
                x: 30.0,
                y: 6.0,
                width: 55.0,
                height: 22.0,
            }),
            trailing: None,
        });
        list_item.style.background = Some([0.15, 0.15, 0.15, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([icon, switch, range, card, list_item], []);

        let icon = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 3,
            })
            .unwrap();
        assert_eq!(
            icon.bounds,
            SceneRect {
                x: 18.0,
                y: 28.0,
                width: 16.0,
                height: 16.0,
            }
        );
        assert!(matches!(
            icon.kind,
            ScenePrimitiveKind::Icon {
                icon: nana_ui_core::Icon::Search,
                color: Some([0.1, 0.6, 0.9, 1.0]),
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 0,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some([0.2, 0.3, 0.4, 1.0]),
                border_color: Some([0.4, 0.5, 0.6, 1.0]),
                ..
            }
        ));

        let switch_track = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 4,
            })
            .unwrap();
        assert_eq!(switch_track.bounds.width, 30.0);
        assert_eq!(switch_track.bounds.height, 16.0);
        assert_eq!(switch_track.bounds.x, 170.0);
        let switch_thumb = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 5,
            })
            .unwrap();
        assert_eq!(switch_thumb.bounds.width, 10.0);
        assert_eq!(switch_thumb.bounds.x, 187.0);
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 2,
                })
                .unwrap()
                .bounds
                .height,
            18.0
        );
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 3,
                })
                .unwrap()
                .bounds
                .y,
            28.0
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 7,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                border_width: 2.0,
                ..
            }
        ));

        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 4,
                })
                .unwrap()
                .bounds
                .width,
            30.0
        );
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 5,
                })
                .unwrap()
                .bounds
                .x,
            103.0
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 6,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                horizontal_alignment: TextHorizontalAlignment::End,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 3,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Spinner { phase: 4, .. }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 0,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                shadow: Some(ComponentElevation {
                    offset_y: 3.0,
                    blur_radius: 8.0,
                    ..
                }),
                ..
            }
        ));
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 2,
                })
                .unwrap()
                .bounds
                .y,
            8.0
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 2,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                ellipsis: false,
                ..
            }
        ));
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(5),
                    slot: 2,
                })
                .unwrap()
                .bounds,
            SceneRect {
                x: 30.0,
                y: 6.0,
                width: 55.0,
                height: 22.0,
            }
        );
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(5),
                    slot: 0,
                })
                .is_some()
        );
    }

    #[test]
    fn subtree_membership_follows_retained_parent_links() {
        let scene = {
            let mut scene = UiScene::new();
            scene.apply_delta(
                [
                    node(1, None, &[2]),
                    node(2, Some(1), &[3]),
                    node(3, Some(2), &[]),
                ],
                [],
            );
            scene
        };
        assert!(scene.is_node_in_subtree(id(1), id(3)));
        assert!(scene.is_node_in_subtree(id(2), id(2)));
        assert!(!scene.is_node_in_subtree(id(3), id(1)));
    }

    #[test]
    fn scroll_offset_transforms_descendants_but_not_viewport_clip() {
        let mut scroller = node(1, None, &[2]);
        scroller.layout.height = 50.0;
        scroller.scroll_offset = nana_ui_runtime::ScrollOffset { x: 0.0, y: 60.0 };
        scroller.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                overflow_y: nana_ui_core::OverflowSpec::Scroll,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.layout.y = 80.0;
        child.text = Some(TextContent {
            value: "Visible".into(),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([scroller, child], []);
        let text = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2,
            })
            .unwrap();
        assert_eq!(text.transform.0[5], -60.0);
        assert_eq!(text.bounds.y, 80.0);
        assert_eq!(text.clips.len(), 1);
        assert_eq!(text.clips[0].bounds.height, 50.0);
        assert_eq!(text.clips[0].transform, AffineTransform::IDENTITY);
    }
}
