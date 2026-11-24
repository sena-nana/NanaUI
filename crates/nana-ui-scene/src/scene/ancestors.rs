//! What a node inherits from its ancestors: the folded transform, opacity
//! and clip chain, and ancestor predicates.

use super::*;

/// Inherited 3D context carried along the ancestor fold:
/// - `blocks_3d`: true below a `preserve-3d` ancestor (refuses child 3D).
/// - `parent_perspective`: direct parent's `perspective` + `perspective-origin`.
/// - `backface_hidden`: true when a flattened ancestor with
///   `backface-visibility: hidden` faces away from the camera.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Ancestor3d {
    pub(super) blocks_3d: bool,
    pub(super) parent_perspective: Option<nana_ui_core::ParentPerspective>,
    pub(super) backface_hidden: bool,
}

pub(super) fn child_parent_perspective(
    style: &nana_ui_core::LayoutStyle,
    layout: LayoutBox,
    scroll: nana_ui_runtime::ScrollOffset,
    blocks_3d: bool,
) -> Option<nana_ui_core::ParentPerspective> {
    if blocks_3d || style.transforms.preserve_3d {
        return None;
    }
    let distance = style.transforms.css_perspective?;
    if !distance.is_finite() || distance <= 1e-5 {
        return None;
    }
    let [ox, oy] = style.resolved_perspective_origin(layout.width, layout.height);
    Some(nana_ui_core::ParentPerspective {
        distance,
        origin: [layout.x + ox + scroll.x, layout.y + oy + scroll.y],
    })
}

pub(super) fn node_scene_transform(
    style: &nana_ui_core::LayoutStyle,
    layout: LayoutBox,
    block_3d: bool,
    parent_perspective: Option<nana_ui_core::ParentPerspective>,
) -> AffineTransform {
    if block_3d && style.transforms.transform_3d.is_some() {
        return AffineTransform::IDENTITY;
    }
    style
        .world_scene_transform_with_perspective(
            layout.x,
            layout.y,
            layout.width,
            layout.height,
            parent_perspective,
        )
        .map(|(matrix, persp)| AffineTransform(matrix, persp))
        .unwrap_or_default()
}

impl UiScene {
    pub(super) fn ancestor_state_mut(&mut self, node: &ExtractedNode) -> AncestorState {
        if self.rebuild_scratch.active {
            let fixed = node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed;
            let visual_parent = effective_visual_parent(&self.nodes, node);
            if let Some(parent) = node.parent
                && !fixed
                && visual_parent.is_none()
                && self
                    .nodes
                    .get(&parent)
                    .is_some_and(|parent_node| !is_triggered_menu(parent_node))
                && clip_link(node) == Some(parent)
                && !is_workspace_resize_handle(node)
                && let Some(state) = self.handed_down_mut(parent, false)
            {
                return state;
            }
            let key = node
                .parent
                .map(|parent| (parent, fixed, visual_parent, clip_link(node), false));
            if let Some(key) = key
                && let Some((held, state)) = self.rebuild_scratch.ancestor_state.as_ref()
                && *held == key
            {
                return state.clone();
            }
            let (state, node_dependent) = self.compute_ancestor_state(node, false, fixed);
            if let Some(key) = key
                && !node_dependent
            {
                self.rebuild_scratch.ancestor_state = Some((key, state.clone()));
            }
            return state;
        }
        self.compute_ancestor_state(
            node,
            false,
            node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed,
        )
        .0
    }

    pub(super) fn draw_ancestor_state(&self, node: &ExtractedNode) -> AncestorState {
        self.derived_projection()
            .draw_parent_state(node.id)
            .cloned()
            .unwrap_or_else(|| {
                let fixed = node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed;
                self.compute_ancestor_state(node, true, fixed).0
            })
    }

    /// The second half of the answer is whether it read anything about `node`
    /// that its siblings do not share — a modal frame above it reads this
    /// node's focus and whether it is in the body, and a workspace resize
    /// handle drops its parent's overflow clip. Neither is cacheable by parent.
    pub(super) fn compute_ancestor_state(
        &self,
        node: &ExtractedNode,
        visual: bool,
        fixed: bool,
    ) -> (AncestorState, bool) {
        let ancestors = node
            .parent
            .map(|parent| {
                ancestor_nodes(&self.nodes, parent)
                    .map(|(_, node)| node)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let hung = clip_chain_parents(&self.nodes, node);
        self.fold_ancestor_chain(ancestors, &hung, Some(node), fixed, visual)
    }

    pub(super) fn compute_ancestor_state_with_origins(
        &self,
        node: &ExtractedNode,
        visual: bool,
        fixed: bool,
        node_index: &NodeMap<usize>,
        placed_origins: &[[f32; 2]],
    ) -> (AncestorState, bool) {
        let ancestors = node
            .parent
            .map(|parent| {
                ancestor_nodes(&self.nodes, parent)
                    .map(|(_, node)| node)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let hung = clip_chain_parents(&self.nodes, node);
        self.fold_ancestor_chain_with_origins(
            ancestors,
            &hung,
            Some(node),
            fixed,
            visual,
            node_index,
            placed_origins,
        )
    }

    pub(super) fn visual_ancestor_opacity(&self, start: VisualNode, visual: bool) -> f32 {
        let mut opacity = 1.0;
        let Some(first) = visual_parent_of(&self.nodes, start) else {
            return opacity;
        };
        for (vn, ancestor) in visual_ancestor_nodes(&self.nodes, first) {
            if matches!(vn, VisualNode::Node(_)) && !is_opacity_group(&self.nodes, ancestor) {
                opacity *= if visual {
                    self.resolved_local_opacity(ancestor)
                } else {
                    local_opacity(ancestor)
                };
            }
        }
        opacity.clamp(0.0, 1.0)
    }

    /// [`Self::compute_ancestor_state`] over an explicit chain: `ancestors`
    /// innermost first, `hung` the clip chain's links nearest first. Without a
    /// `node` the answer is the one a plain child of the innermost ancestor
    /// gets — in flow, hanging from it, neither fixed nor a resize handle.
    fn fold_ancestor_chain(
        &self,
        ancestors: Vec<&ExtractedNode>,
        hung: &[StableNodeId],
        node: Option<&ExtractedNode>,
        fixed: bool,
        visual: bool,
    ) -> (AncestorState, bool) {
        self.fold_ancestor_chain_inner(ancestors, hung, node, fixed, visual, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn fold_ancestor_chain_with_origins(
        &self,
        ancestors: Vec<&ExtractedNode>,
        hung: &[StableNodeId],
        node: Option<&ExtractedNode>,
        fixed: bool,
        visual: bool,
        node_index: &NodeMap<usize>,
        placed_origins: &[[f32; 2]],
    ) -> (AncestorState, bool) {
        self.fold_ancestor_chain_inner(
            ancestors,
            hung,
            node,
            fixed,
            visual,
            Some((node_index, placed_origins)),
        )
    }

    fn fold_ancestor_chain_inner(
        &self,
        mut ancestors: Vec<&ExtractedNode>,
        hung: &[StableNodeId],
        node: Option<&ExtractedNode>,
        fixed: bool,
        visual: bool,
        precomputed: Option<(&NodeMap<usize>, &[[f32; 2]])>,
    ) -> (AncestorState, bool) {
        let mut node_dependent = node.is_some_and(is_workspace_resize_handle);
        ancestors.reverse();
        // Layout resolves fixed boxes against the viewport even below a
        // transformed ancestor. Keep structural opacity, but begin geometry at
        // the nearest fixed boundary instead of inheriting its outer scroll,
        // transform and clip chain.
        let geometry_start = if fixed {
            ancestors.len()
        } else {
            ancestors
                .iter()
                .rposition(|ancestor| {
                    ancestor.source_style.layout.position == nana_ui_core::PositionSpec::Fixed
                })
                .unwrap_or(0)
        };
        // The items a triggered menu places hang with its surface: their
        // branch starts from the menu's presentation, not the bare viewport.
        let fixed_root = if fixed {
            node
        } else {
            ancestors.get(geometry_start).copied().filter(|ancestor| {
                ancestor.source_style.layout.position == nana_ui_core::PositionSpec::Fixed
            })
        };
        let transform = match fixed_root {
            Some(root) if visual => {
                // Whether a fixed node starts its menu's branch depends on
                // whether it is the menu's trigger content, which its
                // siblings do not share.
                node_dependent |= fixed
                    && node
                        .and_then(|node| node.parent)
                        .and_then(|parent| self.nodes.get(&parent))
                        .is_some_and(|parent| is_triggered_menu(parent));
                self.hanging_branch_transform_with(root, precomputed)
            }
            _ => AffineTransform::IDENTITY,
        };
        let opacity = if let Some(node) = node {
            if effective_visual_parent(&self.nodes, node).is_some() {
                node_dependent = true;
            }
            self.visual_ancestor_opacity(VisualNode::Node(node.id), visual)
        } else if let Some(innermost) = ancestors.last().copied() {
            let own = if !is_opacity_group(&self.nodes, innermost) {
                if visual {
                    self.resolved_local_opacity(innermost)
                } else {
                    local_opacity(innermost)
                }
            } else {
                1.0
            };
            (own * self.visual_ancestor_opacity(VisualNode::Node(innermost.id), visual))
                .clamp(0.0, 1.0)
        } else {
            1.0
        };
        let mut fold = AncestorFold {
            transform,
            opacity,
            clips: Vec::new(),
            ctx_3d: Ancestor3d::default(),
        };
        // The ancestors the clip chain hangs this node from, outermost
        // first: a subsequence of `ancestors`. The others are passed over by
        // an absolute box escaping overflow containers (#201); they lend it
        // their transforms and opacity, but none of their clips or scroll.
        let mut hung = hung.iter().rev().peekable();
        for (index, ancestor) in ancestors.into_iter().enumerate() {
            let reaches = hung.next_if(|id| **id == ancestor.id).is_some();
            if index >= geometry_start {
                let (layout, geometry_origin) = match precomputed {
                    Some((node_index, placed_origins)) => {
                        let [x, y] = node_index
                            .get(&ancestor.id)
                            .map_or([ancestor.layout.x, ancestor.layout.y], |&i| {
                                placed_origins[i]
                            });
                        let origin = if record_on_page(ancestor) {
                            [0.0, 0.0]
                        } else {
                            [x, y]
                        };
                        (
                            LayoutBox {
                                x,
                                y,
                                ..ancestor.layout
                            },
                            origin,
                        )
                    }
                    None => {
                        let origin = super::placement::placed_origin_live(
                            &self.nodes,
                            self.placement_live,
                            ancestor,
                        );
                        (
                            origin.into_layout(ancestor),
                            if record_on_page(ancestor) {
                                [0.0, 0.0]
                            } else {
                                origin
                            },
                        )
                    }
                };
                node_dependent |= self.fold_ancestor_at(
                    &mut fold,
                    ancestor,
                    node,
                    visual,
                    true,
                    reaches,
                    layout,
                    geometry_origin,
                    precomputed,
                );
            }
        }
        (fold.finish(), node_dependent)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn fold_ancestor_at(
        &self,
        fold: &mut AncestorFold,
        ancestor: &ExtractedNode,
        node: Option<&ExtractedNode>,
        visual: bool,
        geometry: bool,
        reaches: bool,
        layout: LayoutBox,
        geometry_origin: [f32; 2],
        precomputed: Option<(&NodeMap<usize>, &[[f32; 2]])>,
    ) -> bool {
        if !geometry {
            return false;
        }
        // `blocks_3d` is what the ancestors *above* this one closed, the
        // same question asked of the queried node below — an ancestor's
        // own `perspective` opens a context for its children, not against
        // itself, so it folds in afterwards. Reading it keeps a refusal
        // whole: a node whose `matrix3d` this rule takes away must not go
        // on handing it down, or it paints flat with a rotated inside.
        let local = if visual {
            self.resolved_page_transform_at(
                ancestor,
                layout,
                fold.ctx_3d.blocks_3d,
                fold.ctx_3d.parent_perspective,
                precomputed,
            )
        } else {
            self.logical_page_transform_at(
                ancestor,
                layout,
                fold.ctx_3d.blocks_3d,
                fold.ctx_3d.parent_perspective,
                precomputed,
            )
        };
        // What it hands down is on the page, and so are its clips.
        let handed = fold.transform.then(local);
        fold.transform = handed;
        let ancestor_style = ancestor.source_style.layout.as_ref();
        if ancestor_style.fails_closed_3d_context() {
            fold.ctx_3d.blocks_3d = true;
        }
        if ancestor_style.transforms.backface_visibility == nana_ui_core::BackfaceVisibility::Hidden
            && nana_ui_core::is_projective_backface(
                handed.0,
                handed.1,
                layout.x + layout.width * 0.5,
                layout.y + layout.height * 0.5,
            )
        {
            fold.ctx_3d.backface_hidden = true;
        }
        let scroll = if reaches {
            ancestor.scroll_offset
        } else {
            nana_ui_runtime::ScrollOffset::default()
        };
        fold.ctx_3d.parent_perspective =
            child_parent_perspective(ancestor_style, layout, scroll, fold.ctx_3d.blocks_3d);
        if !reaches {
            return false;
        }
        let transform = handed;
        let mut node_dependent = false;
        let clips = &mut fold.clips;
        if let Some(region) = overflow_clip_region(
            ancestor.source_style.layout.as_ref(),
            SceneRect {
                x: layout.x,
                y: layout.y,
                width: layout.width,
                height: layout.height,
            },
            transform,
        ) && !node.is_some_and(|node| {
            is_workspace_resize_handle(node) && Some(ancestor.id) == node.parent
        }) {
            clips.push(region);
        }
        if ancestor
            .widget_paint
            .as_deref()
            .is_some_and(|w| w.clips_descendants)
        {
            clips.push(ClipRegion {
                bounds: SceneRect {
                    x: layout.x,
                    y: layout.y,
                    width: layout.width,
                    height: layout.height,
                },
                transform,
                corner_radius: 0.0,
                polygon_clip: None,
            });
        }
        if let Some(modal) = ancestor
            .widget_paint
            .as_deref()
            .and_then(|w| w.modal_frame.as_ref())
        {
            node_dependent = true;
            let surface = offset_rect(scene_rect(modal.surface), geometry_origin);
            let focus_inset = if node.is_some_and(|node| node.focused) {
                3.0
            } else {
                0.0
            };
            clips.push(ClipRegion {
                bounds: SceneRect {
                    x: surface.x - focus_inset,
                    y: surface.y - focus_inset,
                    width: surface.width + focus_inset * 2.0,
                    height: surface.height + focus_inset * 2.0,
                },
                transform,
                corner_radius: 0.0,
                polygon_clip: None,
            });
            if let Some(node) = node
                && modal
                    .body_root
                    .is_some_and(|body_root| self.node_descends_from(node.id, body_root))
            {
                clips.push(ClipRegion {
                    bounds: offset_rect(scene_rect(modal.body), geometry_origin),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
            }
        }
        let ancestor_bounds = SceneRect {
            x: layout.x,
            y: layout.y,
            width: layout.width,
            height: layout.height,
        };
        if let Some(region) = clip_path_region(
            ancestor.source_style.layout.as_ref(),
            ancestor_bounds,
            transform,
        ) {
            clips.push(region);
        }
        fold.transform = handed.then(AffineTransform::from_matrix([
            1.0,
            0.0,
            0.0,
            1.0,
            -ancestor.scroll_offset.x,
            -ancestor.scroll_offset.y,
        ]));
        node_dependent
    }

    /// What `parent` hands a plain child — see [`Self::fold_ancestor_chain`]
    /// — while the rebuild pass runs, remembered per parent so a chain of
    /// only children costs one fold a node rather than one walk a node.
    /// `None` when the answer is not the parent's alone (a modal frame above
    /// reads the child's focus), or the chain leaves the scene.
    ///
    /// Each parent's answer is its own parent's answer folded once more:
    /// a fixed parent starts geometry again, keeping the opacity. A parent
    /// whose clip chain does not simply follow its parent is folded from
    /// scratch. The walk up to the nearest remembered answer is a loop, not
    /// recursion, so it does not grow with the tree's depth.
    pub(super) fn handed_down_mut(
        &mut self,
        parent: StableNodeId,
        visual: bool,
    ) -> Option<AncestorState> {
        if !self.rebuild_scratch.active {
            return None;
        }
        let mut memo = std::mem::take(&mut self.rebuild_scratch.handed_down[usize::from(visual)]);
        let answer = self.resolve_handed_down(&mut memo, parent, visual);
        self.rebuild_scratch.handed_down[usize::from(visual)] = memo;
        answer
    }

    #[cfg(test)]
    pub(super) fn handed_down_with(
        &self,
        scratch: &mut RebuildScratch,
        parent: StableNodeId,
        visual: bool,
    ) -> Option<AncestorState> {
        if !scratch.active {
            return None;
        }
        let mut memo = std::mem::take(&mut scratch.handed_down[usize::from(visual)]);
        let answer = self.resolve_handed_down(&mut memo, parent, visual);
        scratch.handed_down[usize::from(visual)] = memo;
        answer
    }

    fn resolve_handed_down(
        &self,
        memo: &mut NodeMap<Option<AncestorState>>,
        parent: StableNodeId,
        visual: bool,
    ) -> Option<AncestorState> {
        // The ancestors still to fold, innermost first.
        let mut chain = Vec::new();
        let mut cursor = parent;
        let mut handed: Option<AncestorState> = loop {
            if let Some(held) = memo.get(&cursor) {
                break held.clone();
            }
            let node = self.nodes.get(&cursor)?;
            if effective_visual_parent(&self.nodes, node).is_some() || is_triggered_menu(node) {
                break None;
            }
            let fixed = node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed;
            let follows = if fixed {
                clip_link(node).is_none()
            } else {
                clip_link(node) == node.parent
            };
            if !follows {
                let ancestors = ancestor_nodes(&self.nodes, cursor)
                    .map(|(_, node)| node)
                    .collect::<Vec<_>>();
                let mut hung = vec![cursor];
                hung.extend(clip_chain_parents(&self.nodes, node));
                let (state, node_dependent) =
                    self.fold_ancestor_chain(ancestors, &hung, None, false, visual);
                let state = (!node_dependent).then_some(state);
                memo.insert(cursor, state.clone());
                break state;
            }
            chain.push(node.as_ref());
            match node.parent {
                Some(next) => cursor = next,
                None => {
                    break Some((
                        AffineTransform::IDENTITY,
                        1.0,
                        Arc::from(Vec::new()),
                        Ancestor3d::default(),
                    ));
                }
            }
        };
        for ancestor in chain.into_iter().rev() {
            handed = handed.and_then(|state| self.fold_handed_down(state, ancestor, visual));
            memo.insert(ancestor.id, handed.clone());
        }
        handed
    }

    /// `state`, what `ancestor`'s parent hands it, folded through `ancestor`.
    fn fold_handed_down(
        &self,
        state: AncestorState,
        ancestor: &ExtractedNode,
        visual: bool,
    ) -> Option<AncestorState> {
        let (transform, opacity, inherited, ctx_3d) = state;
        let mut fold = AncestorFold {
            transform,
            opacity,
            clips: Vec::new(),
            ctx_3d,
        };
        let mut inherited = Some(inherited);
        if ancestor.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
            fold.transform = if visual {
                self.hanging_branch_transform(ancestor)
            } else {
                AffineTransform::IDENTITY
            };
            fold.ctx_3d = Ancestor3d::default();
            inherited = None;
        }
        if !is_opacity_group(&self.nodes, ancestor) {
            fold.opacity *= if visual {
                self.resolved_local_opacity(ancestor)
            } else {
                local_opacity(ancestor)
            };
        }
        let placed_origin =
            super::placement::placed_origin_live(&self.nodes, self.placement_live, ancestor);
        let layout = LayoutBox {
            x: placed_origin[0],
            y: placed_origin[1],
            ..ancestor.layout
        };
        let geometry_origin = if record_on_page(ancestor) {
            [0.0, 0.0]
        } else {
            placed_origin
        };
        if self.fold_ancestor_at(
            &mut fold,
            ancestor,
            None,
            visual,
            true,
            true,
            layout,
            geometry_origin,
            None,
        ) {
            return None;
        }
        let clips = match inherited {
            Some(inherited) if fold.clips.is_empty() => inherited,
            Some(inherited) => inherited.iter().cloned().chain(fold.clips).collect(),
            None => fold.clips.into(),
        };
        Some((fold.transform, fold.opacity, clips, fold.ctx_3d))
    }

    /// Whether a rasterized SVG or an icon glyph above `node` already paints
    /// it. While the rebuild pass runs, the walks behind both answers are
    /// remembered per ancestor.
    pub(super) fn inside_ancestor_glyph_mut(&mut self, node: &ExtractedNode) -> bool {
        let Some(parent) = node.parent else {
            return false;
        };
        if self.rebuild_scratch.active {
            let scratch = &mut self.rebuild_scratch;
            return ancestor_answer(
                &self.nodes,
                &mut scratch.in_rasterized_svg,
                &mut scratch.walk,
                parent,
                rasterized_svg_decides,
            ) || (!node.source_style.layout.is_out_of_flow()
                && ancestor_answer(
                    &self.nodes,
                    &mut scratch.in_icon_visual,
                    &mut scratch.walk,
                    parent,
                    icon_visual_decides,
                ));
        }
        is_descendant_of_rasterized_svg(&self.nodes, node)
            || is_descendant_of_icon_visual(&self.nodes, node)
    }

    pub(super) fn node_descends_from(&self, id: StableNodeId, ancestor: StableNodeId) -> bool {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.nodes.get(&candidate).and_then(|node| node.parent);
        }
        false
    }
}

pub(super) fn is_workspace_resize_handle(node: &ExtractedNode) -> bool {
    matches!(
        node.kind.as_ref(),
        NodeKind::Element { tag } if tag == "workspace-resize-handle"
    )
}

fn is_descendant_of_rasterized_svg(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    node.parent.is_some_and(|parent| {
        ancestor_nodes(nodes, parent)
            .any(|(_, ancestor)| rasterized_svg_decides(ancestor).is_some())
    })
}

/// Whether `node` is part of a glyph an ancestor already paints.
///
/// An icon bound to `IconGlyph` keeps the vector markup it came from — `path`,
/// `circle` — as children. The atlas glyph draws all of it, so those children
/// must not paint their own boxes on top of it.
///
/// They are *in flow* inside the icon, and that is the part worth naming. A
/// surface an icon only hosts is not: an `IconButton` parents its tooltip to
/// itself, fixed-positioned at `z_index` 1000, and a walk that stopped at the
/// first `Icon` ancestor swallowed it — which is why no icon-button tooltip has
/// ever reached the scene. The walk therefore ends at the first out-of-flow
/// box, and the icon's own children keep being skipped.
fn is_descendant_of_icon_visual(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    if node.source_style.layout.is_out_of_flow() {
        return false;
    }
    let Some(parent) = node.parent else {
        return false;
    };
    ancestor_nodes(nodes, parent)
        .find_map(|(_, ancestor)| icon_visual_decides(ancestor))
        .unwrap_or(false)
}

/// Whether a rasterized SVG at or above `ancestor` paints over its subtree;
/// see [`is_descendant_of_rasterized_svg`].
fn rasterized_svg_decides(ancestor: &ExtractedNode) -> Option<bool> {
    (ancestor
        .custom_render
        .as_ref()
        .is_some_and(|custom| custom.renderer.as_ref() == "nana.host-texture")
        && matches!(
            ancestor.kind.as_ref(),
            NodeKind::Element { tag } if tag.eq_ignore_ascii_case("svg")
        ))
    .then_some(true)
}

/// Whether the walk for [`is_descendant_of_icon_visual`] ends at `ancestor`,
/// and with which answer.
fn icon_visual_decides(ancestor: &ExtractedNode) -> Option<bool> {
    if ancestor
        .widget_paint
        .as_deref()
        .is_some_and(|w| w.suppresses_in_flow_descendants)
    {
        Some(true)
    } else if ancestor.source_style.layout.is_out_of_flow() {
        Some(false)
    } else {
        None
    }
}

/// The answer of a walk from `start` up its ancestors that ends at the first
/// one `decides` has an answer for (false past the root or a node the scene
/// no longer holds), remembered in `memo` for every node it passed.
///
/// Every node the walk passes over defers to its parent, so they all share
/// the one answer: a chain of only children pays one step a node, not one
/// walk a node. `passed` is the caller's buffer for the nodes walked, kept
/// across calls so a walk does not allocate.
pub(super) fn ancestor_answer(
    nodes: &SceneNodes,
    memo: &mut NodeMap<bool>,
    passed: &mut Vec<StableNodeId>,
    start: StableNodeId,
    decides: fn(&ExtractedNode) -> Option<bool>,
) -> bool {
    passed.clear();
    let mut cursor = Some(start);
    let answer = loop {
        let Some(id) = cursor else {
            break false;
        };
        if let Some(held) = memo.get(&id) {
            break *held;
        }
        let Some(node) = nodes.get(&id) else {
            break false;
        };
        passed.push(id);
        if let Some(answer) = decides(node) {
            break answer;
        }
        cursor = node.parent;
    };
    memo.extend(passed.drain(..).map(|id| (id, answer)));
    answer
}

/// A parent chain longer than this is a bug in extraction, not a deep tree.
///
/// It replaces the per-call `HashSet` these walks used to carry as a cycle
/// guard. They run once per primitive per frame — a set allocation each cost
/// more than the walk it was protecting, and a depth cap protects the same
/// thing.
pub(super) const MAX_ANCESTOR_DEPTH: usize = 4096;

/// `node` and its ancestors by id, innermost first.
///
/// Yields an id whether or not the scene still holds that node, and stops
/// after one it does not: the chain cannot continue past a node whose parent
/// nobody knows.
pub(super) fn ancestor_ids(
    nodes: &SceneNodes,
    node: StableNodeId,
) -> impl Iterator<Item = StableNodeId> + '_ {
    let mut current = Some(node);
    let mut depth = 0usize;
    std::iter::from_fn(move || {
        let id = current.take()?;
        if depth >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        depth += 1;
        current = nodes.get(&id).and_then(|node| node.parent);
        Some(id)
    })
}

/// `node` and its ancestors, innermost first, stopping at the first one the
/// scene no longer holds.
/// The node `node` takes clips and scroll from, as its layout result
/// publishes it (#201): its parent, or the clip parent an escaping absolute
/// box hangs from. A node without a result hangs from its parent.
pub(super) fn clip_link(node: &ExtractedNode) -> Option<StableNodeId> {
    match node.extracted_layout {
        Some(layout) => layout.clip_parent,
        None if node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed => None,
        None => node.parent,
    }
}

/// The ancestors whose clips and scroll reach `node`, nearest first, by its
/// clip chain's links.
pub(super) fn clip_chain_parents(nodes: &SceneNodes, node: &ExtractedNode) -> Vec<StableNodeId> {
    let mut parents = Vec::new();
    let mut cursor = clip_link(node);
    while let Some(id) = cursor
        && parents.len() < MAX_ANCESTOR_DEPTH
    {
        parents.push(id);
        cursor = nodes.get(&id).and_then(|node| clip_link(node));
    }
    parents
}

pub(super) fn ancestor_nodes(
    nodes: &SceneNodes,
    node: StableNodeId,
) -> impl Iterator<Item = (StableNodeId, &ExtractedNode)> {
    let mut current = Some(node);
    let mut depth = 0usize;
    std::iter::from_fn(move || {
        let id = current.take()?;
        if depth >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        depth += 1;
        let entry = nodes.get(&id)?;
        current = entry.parent;
        Some((id, entry.as_ref()))
    })
}

/// Which chain an [`AncestorState`] was read for: the parent it starts at,
/// whether the node breaks out of it, its visual parent override, the clip
/// parent it hangs from (#201), and whether the caller wanted the presented
/// values or the logical ones.
pub(super) type AncestorStateKey = (
    StableNodeId,
    bool,
    Option<nana_ui_runtime::scene_contract::VisualParent>,
    Option<StableNodeId>,
    bool,
);

pub(super) trait IntoLayoutOrigin {
    fn into_layout(self, base: &ExtractedNode) -> LayoutBox;
}

impl IntoLayoutOrigin for [f32; 2] {
    fn into_layout(self, base: &ExtractedNode) -> LayoutBox {
        LayoutBox {
            x: self[0],
            y: self[1],
            ..base.layout
        }
    }
}

#[allow(dead_code)]
pub(super) type DrawStamp = (u64, u64);

/// What a node inherits from the chain above it.
pub(super) type AncestorState = (AffineTransform, f32, Arc<[ClipRegion]>, Ancestor3d);

/// [`AncestorState`] while it is being folded, its clips still growing.
pub(super) struct AncestorFold {
    pub(super) transform: AffineTransform,
    pub(super) opacity: f32,
    pub(super) clips: Vec<ClipRegion>,
    pub(super) ctx_3d: Ancestor3d,
}

impl AncestorFold {
    pub(super) fn finish(self) -> AncestorState {
        (self.transform, self.opacity, self.clips.into(), self.ctx_3d)
    }
}

/// The clip an `overflow` other than `visible` puts on a node's descendants.
///
/// It is the border box with the box's own rounding, as in CSS: a child
/// cannot paint past the rounded corners of an `overflow: hidden` parent.
/// The scene carries one radius per clip, so corners of unequal radius clip
/// with the smallest — never cutting away paint a corner should keep. With
/// only one axis clipped the open axis extends far past the box and there is
/// no corner to round.
pub(super) fn overflow_clip_region(
    style: &nana_ui_core::LayoutStyle,
    bounds: SceneRect,
    transform: AffineTransform,
) -> Option<ClipRegion> {
    let (x, y, width, height) =
        style.overflow_clip_box(bounds.x, bounds.y, bounds.width, bounds.height)?;
    let corner_radius = if style.overflow_x.clips() && style.overflow_y.clips() {
        style
            .resolved_border_radii(bounds.width, bounds.height)
            .into_iter()
            .fold(f32::INFINITY, f32::min)
            .min(bounds.width.min(bounds.height) * 0.5)
    } else {
        0.0
    };
    Some(ClipRegion {
        bounds: SceneRect {
            x,
            y,
            width,
            height,
        },
        transform,
        corner_radius: if corner_radius.is_finite() {
            corner_radius.max(0.0)
        } else {
            0.0
        },
        polygon_clip: None,
    })
}

pub(super) fn clip_path_region(
    style: &nana_ui_core::LayoutStyle,
    bounds: SceneRect,
    transform: AffineTransform,
) -> Option<ClipRegion> {
    let clip_path = style.paint.clip_path.as_ref()?;
    match clip_path {
        ClipPath::Inset(inset) => {
            let [top, right, bottom, left] = inset.resolve_offsets(bounds.width, bounds.height);
            Some(ClipRegion {
                bounds: SceneRect {
                    x: bounds.x + left,
                    y: bounds.y + top,
                    width: (bounds.width - left - right).max(0.0),
                    height: (bounds.height - top - bottom).max(0.0),
                },
                transform,
                corner_radius: inset.resolve_round(bounds.width, bounds.height),
                polygon_clip: None,
            })
        }
        ClipPath::Polygon(_) => {
            let points = clip_path.resolve_polygon_points(bounds.width, bounds.height)?;
            let min_x = points
                .iter()
                .map(|point| point[0])
                .fold(f32::INFINITY, f32::min);
            let min_y = points
                .iter()
                .map(|point| point[1])
                .fold(f32::INFINITY, f32::min);
            let max_x = points
                .iter()
                .map(|point| point[0])
                .fold(f32::NEG_INFINITY, f32::max);
            let max_y = points
                .iter()
                .map(|point| point[1])
                .fold(f32::NEG_INFINITY, f32::max);
            let local_points = points
                .iter()
                .map(|point| [point[0] - min_x, point[1] - min_y])
                .collect();
            Some(ClipRegion {
                bounds: SceneRect {
                    x: bounds.x + min_x,
                    y: bounds.y + min_y,
                    width: (max_x - min_x).max(0.0),
                    height: (max_y - min_y).max(0.0),
                },
                transform,
                corner_radius: 0.0,
                polygon_clip: Some(local_points),
            })
        }
        ClipPath::Circle(_) | ClipPath::Ellipse(_) => {
            let [x, y, w, h] = clip_path.resolve_ellipse_rect(bounds.width, bounds.height)?;
            Some(ClipRegion::ellipse(
                SceneRect {
                    x: bounds.x + x,
                    y: bounds.y + y,
                    width: w.max(0.0),
                    height: h.max(0.0),
                },
                transform,
            ))
        }
    }
}
