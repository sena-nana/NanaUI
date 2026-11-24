//! Opacity, filter, edge-fade and inset-shadow groups: which nodes isolate
//! their paint and what each primitive is drawn inside.

use super::*;

/// Isolating ancestor whose subtree is composited as one layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpacityGroup {
    pub node: StableNodeId,
    pub opacity: f32,
    pub filter: ColorFilter,
    pub mix_blend: MixBlendMode,
    /// Inset `box-shadow` overlay recorded on this dest group. `None` when none.
    pub inset_shadow: Option<InsetShadowOverlay>,
    /// The scroll edge fade the group's alpha ramps across, for a scroll
    /// container styled with one ([`nana_ui_core::PaintStyle::scroll_edge_fade`]).
    pub edge_fade: Option<EdgeFadeMask>,
    /// The directional streak an open shutter draws the group along, in
    /// logical pixels on the page: where its content was at the start of the
    /// exposure, relative to now (Issue #323). Zero draws none.
    pub motion_streak: [f32; 2],
}

impl OpacityGroup {
    /// Whether compositing this group changes nothing: it is kept only as a
    /// stacking and culling boundary.
    pub fn is_identity(&self) -> bool {
        self.opacity == 1.0
            && self.filter.is_identity()
            && self.mix_blend.is_normal()
            && self.inset_shadow.is_none()
            && self.edge_fade.is_none_or(|fade| !fade.is_visible())
            && self.motion_streak == [0.0, 0.0]
    }

    /// Whether the group only fades its scroll edges: compositing it moves
    /// no pixel and lightens none outside the bands, so what it holds can be
    /// culled like anything else.
    pub fn is_edge_fade_only(&self) -> bool {
        self.edge_fade.is_some()
            && Self {
                edge_fade: None,
                ..*self
            }
            .is_identity()
    }
}

/// A scroll container's edge fade on its dest group: the alpha ramps from 0
/// at the clip box's top edge to 1 `top` below it, and from 1 `bottom` above
/// its bottom edge to 0 there. Outside the bands the group is untouched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeFadeMask {
    /// The scrollport's overflow clip box, in the node's layout space.
    pub bounds: SceneRect,
    /// The bands' lengths for the current scroll offset, in logical pixels
    /// ([`nana_ui_core::ScrollEdgeFade::resolve`]).
    pub top: f32,
    pub bottom: f32,
}

impl EdgeFadeMask {
    /// Whether either band has any length right now.
    pub fn is_visible(&self) -> bool {
        self.top > 0.0 || self.bottom > 0.0
    }
}

/// Inset shadow painted onto a dest group after its descendants composite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InsetShadowOverlay {
    pub elevation: ComponentElevation,
    pub bounds: SceneRect,
    pub corner_radius: [f32; 4],
}

/// Isolating ancestor with a non-identity CSS `filter`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterGroup {
    pub node: StableNodeId,
    pub filter: ColorFilter,
}

impl UiScene {
    /// Isolating opacity groups from outermost to innermost that contain `node`.
    /// Dest groups containing `node`, outermost first.
    ///
    /// Asked once per primitive per frame, and again by culling, so the
    /// *ancestor* half of the answer is memoized: a container's list is its
    /// parent's list plus itself when it is a group. The queried node's own
    /// list is not stored — every primitive names a different node, so an
    /// entry for it would be written once and read never.
    ///
    /// A node that is not a group **shares** its parent's list rather than
    /// copying it, so a shell whose containers are all opaque allocates
    /// nothing here at all.
    ///
    /// The stamp covers scene changes and presentation samples: groups retain
    /// their membership across a track while their compositing alpha changes.
    pub fn opacity_groups(&self, node: StableNodeId) -> Arc<[OpacityGroup]> {
        self.visual_opacity_groups(VisualNode::Node(node))
    }

    pub(super) fn visual_opacity_groups(&self, vn: VisualNode) -> Arc<[OpacityGroup]> {
        if self.dest_group_candidates == 0
            && self.nodes.opacity_motion.is_empty()
            && self.nodes.filter_motion.is_empty()
        {
            return empty_opacity_groups();
        }
        if !self.nodes.contains_key(&vn.node_id()) {
            return empty_opacity_groups();
        }
        self.derived_projection().opacity_groups(vn)
    }

    /// `own` as it composites below `base`: a non-identity filter past
    /// [`ColorFilter::MAX_CHAIN_DEPTH`] filtering groups stays the identity
    /// (Issue #324). The group itself stays, so its opacity, blend and the
    /// subtree it holds are untouched.
    pub(super) fn within_filter_depth(
        &self,
        base: &[OpacityGroup],
        mut own: OpacityGroup,
    ) -> OpacityGroup {
        if own.filter.is_identity() {
            return own;
        }
        let depth = filtering_groups(base);
        if depth < ColorFilter::MAX_CHAIN_DEPTH {
            return own;
        }
        own.filter = ColorFilter::default();
        self.report_filter_depth(own.node, depth);
        own
    }

    /// One [`PAINT_FILTER_DEPTH_EXCEEDED`] per node, however often its
    /// groups are asked for.
    ///
    /// [`PAINT_FILTER_DEPTH_EXCEEDED`]: nana_diagnostics::framework::runtime::PAINT_FILTER_DEPTH_EXCEEDED
    pub(super) fn report_filter_depth(&self, node: StableNodeId, depth: usize) {
        let first = self
            .filter_depth_reported
            .lock()
            .map(|mut reported| reported.insert(node))
            .unwrap_or(false);
        if first {
            nana_diagnostics::event!(
                nana_diagnostics::framework::runtime::PAINT_FILTER_DEPTH_EXCEEDED,
                node = node.get(),
                depth = depth as u64,
            );
        }
    }

    /// Destination groups for this draw. A modal scrim inherits outer groups,
    /// but never the card's own opacity: it has an independent presence track.
    /// Triggered overlay surfaces (`VisualNode::Overlay`) strip bypassed
    /// scroll edge fades; scrollbars strip their own scroll container's edge fade.
    pub fn primitive_opacity_groups(&self, primitive: &ScenePrimitive) -> Arc<[OpacityGroup]> {
        if self.dest_group_candidates == 0
            && self.nodes.opacity_motion.is_empty()
            && self.nodes.filter_motion.is_empty()
        {
            return empty_opacity_groups();
        }
        if self.is_modal_scrim(primitive) {
            return self
                .derived_projection()
                .inherited_node_opacity_groups(primitive.node);
        }
        let vn = primitive_visual_node(&self.nodes, primitive);
        let groups = self.derived_projection().opacity_groups(vn);
        if is_scrollbar_chrome_slot(primitive.id.slot) {
            return without_edge_fade(primitive.node, groups);
        }
        groups
    }

    pub(super) fn visual_node_overlay_opacity_group(
        &self,
        id: StableNodeId,
    ) -> Option<OpacityGroup> {
        if !visual_is_opacity_group(&self.nodes, VisualNode::Overlay(id)) {
            return None;
        }
        let menu = self.nodes.get(&id)?;
        let fade = match self.compositor.opacity(id) {
            Some(presented) => {
                let logical = local_opacity(menu);
                if logical > 1e-6 {
                    (presented / logical).clamp(0.0, 1.0)
                } else {
                    presented.clamp(0.0, 1.0)
                }
            }
            None => 1.0,
        };
        // A triggered menu's presented filter, like its fade, is
        // its hanging surface's: the trigger keeps its own paint.
        let filter = self.compositor.effect(id).unwrap_or_default();
        if (fade - 1.0).abs() < 1e-6 && filter.is_identity() {
            return None;
        }
        Some(OpacityGroup {
            node: id,
            opacity: fade,
            filter: filter.with_blur_cap(ColorFilter::blur_cap(
                menu.source_style.layout.paint.wide_filter_blur,
            )),
            mix_blend: MixBlendMode::Normal,
            inset_shadow: None,
            edge_fade: None,
            motion_streak: [0.0, 0.0],
        })
    }

    /// Isolating filter groups from outermost to innermost that contain `node`.
    pub fn filter_groups(&self, node: StableNodeId) -> Vec<FilterGroup> {
        filter_groups_from(&self.nodes, node)
    }
}

/// How many of `groups` composite a non-identity filter.
pub(super) fn filtering_groups(groups: &[OpacityGroup]) -> usize {
    groups
        .iter()
        .filter(|group| !group.filter.is_identity())
        .count()
}

pub(super) fn local_opacity(node: &ExtractedNode) -> f32 {
    node.source_style
        .layout
        .opacity
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

pub(super) fn has_extracted_child(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    node.children.iter().any(|child| nodes.contains_key(child))
}

pub(super) fn dest_filter_applies(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let Some(filter) = node
        .source_style
        .layout
        .paint
        .filter
        .filter(|filter| !filter.is_identity())
    else {
        return false;
    };
    filter.blur_radius > 0.0
        || filter.drop_shadow.is_some()
        || has_extracted_child(nodes, node)
        || node
            .text
            .as_ref()
            .is_some_and(|text| !text.value.is_empty())
        || node.custom_render.is_some()
        || node.widget_paint.is_some()
}

/// The only thing [`is_opacity_group`] — and so every descendant's paint-order
/// key — asks of a node's opacity. Keep the readings together: a group test
/// that started caring about the value itself would need `stacking_changed` and
/// [`may_be_dest_group`] to care again too.
pub(super) fn is_translucent(node: &ExtractedNode) -> bool {
    let opacity = local_opacity(node);
    opacity > 0.0 && opacity < 1.0
}

/// Node-local necessary condition for [`is_opacity_group`].
///
/// Every disjunct there needs one of these to hold, and none of them can be
/// turned on by a *different* node, so a scene where no node passes this has no
/// opacity group whatever its shape. [`UiScene::dest_group_candidates`] counts
/// these as nodes are inserted and removed, which is why this must stay a
/// superset: a term added to [`is_opacity_group`] needs its own term here.
pub(super) fn may_be_dest_group(node: &ExtractedNode) -> bool {
    is_translucent(node)
        || node
            .source_style
            .layout
            .paint
            .filter
            .is_some_and(|filter| !filter.is_identity())
        || !node.source_style.layout.paint.mix_blend.is_normal()
        || has_scroll_edge_fade(node)
}

pub(super) fn is_opacity_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let translucent = is_translucent(node) && has_extracted_child(nodes, node);
    ((nodes.opacity_motion.contains(&node.id) || nodes.filter_motion.contains(&node.id))
        && !is_triggered_menu(node))
        || translucent
        || dest_filter_applies(nodes, node)
        || !node.source_style.layout.paint.mix_blend.is_normal()
        || (has_scroll_edge_fade(node) && has_extracted_child(nodes, node))
}

pub(super) fn visual_is_opacity_group(nodes: &SceneNodes, vn: VisualNode) -> bool {
    match vn {
        VisualNode::Node(id) => nodes
            .get(&id)
            .is_some_and(|node| is_opacity_group(nodes, node)),
        VisualNode::Overlay(id) => {
            (nodes.opacity_motion.contains(&id) || nodes.filter_motion.contains(&id))
                && nodes.get(&id).is_some_and(|node| is_triggered_menu(node))
        }
    }
}

/// A vertically clipping box styled with an edge fade. It is a group however
/// far it is scrolled, so scrolling to either end never re-sorts the scene:
/// the painter skips compositing it while neither band shows.
pub(super) fn has_scroll_edge_fade(node: &ExtractedNode) -> bool {
    let style = &node.source_style.layout;
    style.overflow_y.clips()
        && style
            .paint
            .scroll_edge_fade
            .is_some_and(nana_ui_core::ScrollEdgeFade::is_active)
}

/// The edge fade `node`'s group composites with at its current offset.
fn edge_fade_mask(node: &ExtractedNode, placed: LayoutBox) -> Option<EdgeFadeMask> {
    if !has_scroll_edge_fade(node) {
        return None;
    }
    let style = &node.source_style.layout;
    let fade = style.paint.scroll_edge_fade?;
    let (x, y, width, height) =
        style.overflow_clip_box(placed.x, placed.y, placed.width, placed.height)?;
    // Physical offsets run from the topmost reachable one to the lowest,
    // whichever edge the axis starts at: the content above the scrollport
    // is the distance from the first, and below it the distance to the last.
    let [top, bottom] = node
        .extracted_layout
        .and_then(|layout| layout.scroll_metrics)
        .map_or([0.0, 0.0], |metrics| {
            let offset = node.scroll_offset.y;
            fade.resolve(
                offset - metrics.min_offset().y,
                metrics.max_offset().y - offset,
            )
        });
    Some(EdgeFadeMask {
        bounds: SceneRect {
            x,
            y,
            width,
            height,
        },
        top,
        bottom,
    })
}

/// `groups` with `node`'s own group, when it is the last, not fading its
/// edges: the bars a scroll view draws over its content stay whole. Dropped
/// when that leaves nothing to composite.
fn without_edge_fade(node: StableNodeId, groups: Arc<[OpacityGroup]>) -> Arc<[OpacityGroup]> {
    let Some((own, outer)) = groups
        .split_last()
        .filter(|(own, _)| own.node == node && own.edge_fade.is_some())
    else {
        return groups;
    };
    let own = OpacityGroup {
        edge_fade: None,
        ..*own
    };
    let mut groups = outer.to_vec();
    if !own.is_identity() {
        groups.push(own);
    }
    Arc::from(groups)
}

/// `groups` with `edge_fade` cleared on every group whose node `keep` rejects,
/// dropping any group that had nothing else to composite. Shares `groups`
/// untouched when no group loses a fade.
fn without_edge_fades(
    groups: Arc<[OpacityGroup]>,
    mut keep: impl FnMut(StableNodeId) -> bool,
) -> Arc<[OpacityGroup]> {
    if !groups
        .iter()
        .any(|group| group.edge_fade.is_some() && !keep(group.node))
    {
        return groups;
    }
    let mut kept = Vec::with_capacity(groups.len());
    for group in groups.iter().copied() {
        if group.edge_fade.is_some() && !keep(group.node) {
            let stripped = OpacityGroup {
                edge_fade: None,
                ..group
            };
            if !stripped.is_identity() {
                kept.push(stripped);
            }
        } else {
            kept.push(group);
        }
    }
    Arc::from(kept)
}

pub(super) fn is_filter_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    dest_filter_applies(nodes, node)
}

/// Whether `node`'s group composites a presented filter (Issue #312), which
/// stands for its whole filter, or an open shutter's streak (Issue #323):
/// its own quad then draws unfiltered.
pub(super) fn presents_filter(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    nodes.filter_motion.contains(&node.id) && !is_triggered_menu(node)
}

/// `groups`, inherited by `child_vn` from `parent_vn`: floating overlay
/// surfaces (`VisualNode::Overlay`) and `position: fixed` nodes strip
/// bypassed scroll edge fades, and escaping absolute boxes strip scroll
/// edge fades of containers they are not clipped by.
pub(super) fn inherited_visual_opacity_groups(
    nodes: &SceneNodes,
    parent_vn: VisualNode,
    child_vn: VisualNode,
    groups: Arc<[OpacityGroup]>,
) -> Arc<[OpacityGroup]> {
    if matches!(child_vn, VisualNode::Overlay(_)) {
        return without_edge_fades(groups, |_| false);
    }
    let VisualNode::Node(child_id) = child_vn else {
        return groups;
    };
    let Some(child) = nodes.get(&child_id) else {
        return empty_opacity_groups();
    };
    if child.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
        return without_edge_fades(groups, |_| false);
    }
    if let VisualNode::Node(parent_id) = parent_vn
        && clip_link(child) == Some(parent_id)
    {
        return groups;
    }
    if !groups.iter().any(|group| group.edge_fade.is_some()) {
        return groups;
    }
    let hung = clip_chain_parents(nodes, child);
    without_edge_fades(groups, |node| hung.contains(&node))
}

/// The list every node that is not itself a group shares with its parent.
pub(super) fn empty_opacity_groups() -> Arc<[OpacityGroup]> {
    static EMPTY: std::sync::OnceLock<Arc<[OpacityGroup]>> = std::sync::OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::new())))
}

fn inset_shadow_overlay(node: &ExtractedNode, placed: LayoutBox) -> Option<InsetShadowOverlay> {
    let shadow = node
        .source_style
        .layout
        .paint
        .box_shadows
        .iter()
        .copied()
        .find(|shadow| shadow.inset)?;
    Some(InsetShadowOverlay {
        elevation: ComponentElevation::from_box_shadow(shadow),
        bounds: scene_rect(placed),
        corner_radius: surface_corner_radii(
            node.source_style.layout.as_ref(),
            node.layout.width,
            node.layout.height,
        ),
    })
}

/// `candidate`'s group, its box where it stands now at `placed`.
pub(super) fn dest_group(
    nodes: &SceneNodes,
    id: StableNodeId,
    candidate: &ExtractedNode,
    placed: LayoutBox,
) -> OpacityGroup {
    OpacityGroup {
        node: id,
        opacity: local_opacity(candidate),
        filter: if dest_filter_applies(nodes, candidate) {
            candidate
                .source_style
                .layout
                .paint
                .filter
                .unwrap_or_default()
        } else {
            ColorFilter::default()
        },
        mix_blend: candidate.source_style.layout.paint.mix_blend,
        inset_shadow: inset_shadow_overlay(candidate, placed),
        edge_fade: edge_fade_mask(candidate, placed),
        motion_streak: [0.0, 0.0],
    }
}

fn filter_groups_from(nodes: &SceneNodes, node: StableNodeId) -> Vec<FilterGroup> {
    let mut groups = Vec::new();
    for (vn, candidate) in visual_ancestor_nodes(nodes, VisualNode::Node(node)) {
        if matches!(vn, VisualNode::Node(_)) && is_filter_group(nodes, candidate) {
            groups.push(FilterGroup {
                node: vn.node_id(),
                filter: candidate
                    .source_style
                    .layout
                    .paint
                    .filter
                    .unwrap_or_default(),
            });
        }
    }
    groups.reverse();
    groups
}
