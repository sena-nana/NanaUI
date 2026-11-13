//! Audits of retained and moved primitives against a fresh projection.

use super::*;

impl UiScene {
    /// Assert that whatever [`Self::apply_delta`] chose to keep still equals
    /// what recomputing it would produce.
    ///
    /// Retaining the paint order, the frame plan or the visibility index is a
    /// judgement about which style changes can move them, and getting that
    /// wrong paints the right pixels in the wrong order — the kind of bug no
    /// still-frame snapshot catches, because the frame it is wrong on is the
    /// one *after* a mutation. Running the check inside every delta makes the
    /// whole existing suite a test of the invalidation rules, at O(scene) per
    /// delta, which is why it is bounded to the trees unit tests build.
    ///
    /// An index the scroll fast path has shifted skips the visibility half:
    /// it holds bounds moved by an offset instead of re-derived from layout,
    /// and shifts it has not pushed down to its leaves yet, so it answers a
    /// query the same as a fresh build without matching one bit for bit.
    ///
    /// That is a property of the index, and it lasts until something
    /// re-derives the bounds — not of the delta that did the shifting, which
    /// is why the index carries the flag. Order and plan are still checked.
    #[cfg(debug_assertions)]
    pub(super) fn audit_retained_projection(&self) {
        if self.nodes.len() > RETAINED_AUDIT_LIMIT {
            return;
        }
        let mut stacks: HashMap<VisualNode, GroupPrefix> = HashMap::new();
        let fresh: BTreeSet<SceneOrderKey> = self
            .primitives
            .values()
            .map(|held| {
                let primitive = &held.primitive;
                if self.splits_paint(primitive.node) {
                    return order_key(&self.nodes, &self.node_order, primitive);
                }
                let vn = primitive_visual_node(&self.nodes, primitive);
                let stack = stacks.entry(vn).or_insert_with(|| {
                    let prefix: GroupPrefix =
                        group_prefix(&self.nodes, &self.node_order, vn).into();
                    order_stack(&self.nodes, &prefix, vn, primitive)
                });
                SceneOrderKey::at(Arc::clone(stack), primitive)
            })
            .collect();
        assert!(
            self.ordered == fresh,
            "retained paint order disagrees with a fresh sort"
        );
        // Painted geometry lives only for painted nodes, and every painted
        // primitive has the geometry it was cut from (Issue #217): a node
        // painter's, or the card a dialog's surface painter recorded.
        assert!(
            self.custom_paint.keys().all(|id| self
                .nodes
                .get(id)
                .is_some_and(|node| node.custom_paint.is_some())),
            "painted geometry held for a node that no longer paints"
        );
        assert!(
            self.modal_surface_paint
                .keys()
                .all(|id| self.nodes.get(id).is_some_and(|node| matches!(
                    node.component_geometry.as_deref(),
                    Some(ComponentGeometry::ModalFrame {
                        surface_paint: Some(_),
                        ..
                    })
                ))),
            "a painted card held for a dialog that no longer paints it"
        );
        assert!(
            self.markdown_diagram_paint.keys().all(|id| self
                .nodes
                .get(id)
                .is_some_and(|node| node_has_markdown_diagram(node))),
            "diagram geometry held for a document without diagrams"
        );
        assert!(
            self.primitives.values().all(|held| !matches!(
                held.primitive.kind,
                ScenePrimitiveKind::Path { .. }
                    | ScenePrimitiveKind::LayerBegin { .. }
                    | ScenePrimitiveKind::LayerEnd { .. }
            ) || self
                .custom_paint
                .contains_key(&held.primitive.node)
                || self.modal_surface_paint.contains_key(&held.primitive.node)
                || self
                    .markdown_diagram_paint
                    .contains_key(&held.primitive.node)),
            "a painted primitive without its geometry"
        );
        assert!(
            self.primitives
                .values()
                .all(|held| self.ordered.contains(&held.key)),
            "a retained primitive is filed under a key the order does not hold"
        );
        let Some(plan) = self.frame_plan.get() else {
            return;
        };
        match self.build_frame_plan() {
            Ok(fresh) => assert!(
                plan.operations == fresh.operations
                    && plan.preparations == fresh.preparations
                    && plan.custom_nodes == fresh.custom_nodes,
                "retained frame plan disagrees with a fresh build"
            ),
            // A plan that no longer compiles is reported by `frame_plan`, not here.
            Err(_) => return,
        }
        if let Some(visibility) = self.visibility.get() {
            let fresh = VisibilityIndex::new(self, Arc::clone(plan));
            // A translated index keeps its shifts pending above the leaves:
            // compare what a query reads there.
            let mismatch = if visibility.translated() {
                visibility.placed_mismatch(&fresh)
            } else {
                visibility.mismatch(&fresh)
            };
            if let Some(mismatch) = mismatch {
                panic!("retained visibility index disagrees with a fresh build: {mismatch}");
            }
        }
    }

    /// The cold check of a move: every primitive of a node at or under a
    /// root that moved in this delta draws what a scene that built it again
    /// now draws, to the bit -- the same geometry in the node's own
    /// coordinates, put at the same origin, through the same translation
    /// and cut by the same clips. A node whose build read where it stood
    /// ([`Self::origin_bound`], [`Self::offset_bound`]) and was not built
    /// again fails here.
    ///
    /// A transform that is more than a translation is rebased through an
    /// inverse and lands an ulp off a fresh composition whether or not
    /// anything moved, so only where it is put is compared for it. Inline
    /// objects a presentation is moving are written by the presenter after
    /// they are built, and are left out.
    ///
    /// Rebuilds the whole scene, so it runs on the trees unit tests build.
    #[cfg(debug_assertions)]
    pub(super) fn audit_moved_primitives(&self, roots: &[StableNodeId]) {
        if self.nodes.len() > RETAINED_AUDIT_LIMIT {
            return;
        }
        let mut cold = self.clone();
        let ids: Vec<StableNodeId> = cold.nodes.keys().copied().collect();
        for id in ids {
            cold.rebuild_node_primitives(id);
        }
        let under_move =
            |id: StableNodeId| roots.iter().any(|&root| self.node_descends_from(id, root));
        let rect_bits =
            |rect: SceneRect| [rect.x, rect.y, rect.width, rect.height].map(f32::to_bits);
        let translation = |transform: AffineTransform| {
            transform.0[..4] == [1.0, 0.0, 0.0, 1.0] && transform.1 == [0.0, 0.0]
        };
        for (&id, held) in &self.primitives {
            let node = held.primitive.node;
            if self.glyph_objects.contains_key(&node) || !under_move(node) {
                continue;
            }
            let fresh = cold
                .primitives
                .get(&id)
                .unwrap_or_else(|| panic!("{id:?}: kept under a move, a rebuild drops it"));
            let (Some(draw), Some(cold_draw)) = (self.draw_primitive(id), cold.draw_primitive(id))
            else {
                continue;
            };
            assert_eq!(
                draw.origin.map(f32::to_bits),
                cold_draw.origin.map(f32::to_bits),
                "{id:?}: kept under a move, it is put at {:?}, a rebuild at {:?}",
                draw.origin,
                cold_draw.origin
            );
            if translation(cold_draw.transform) {
                assert!(
                    draw.transform.0.map(f32::to_bits) == cold_draw.transform.0.map(f32::to_bits)
                        && draw.transform.1.map(f32::to_bits)
                            == cold_draw.transform.1.map(f32::to_bits),
                    "{id:?}: kept under a move, it is drawn through {:?}, a rebuild through {:?}",
                    draw.transform,
                    cold_draw.transform
                );
            }
            assert!(
                draw.clips.len() == cold_draw.clips.len()
                    && draw
                        .clips
                        .iter()
                        .zip(cold_draw.clips.iter())
                        .all(|(clip, cold)| rect_bits(clip.bounds) == rect_bits(cold.bounds)),
                "{id:?}: kept under a move, it is cut by {:?}, a rebuild by {:?}",
                draw.clips,
                cold_draw.clips
            );
            let mut kept = held.primitive.clone();
            kept.clips = fresh.primitive.clips.clone();
            kept.transform = fresh.primitive.transform;
            assert!(
                kept == fresh.primitive,
                "{id:?}: kept under a move, it is not the primitive a rebuild makes:\n\
                 kept {kept:?}\nbuilt {:?}",
                fresh.primitive
            );
        }
        for (&id, fresh) in &cold.primitives {
            assert!(
                self.primitives.contains_key(&id) || !under_move(fresh.primitive.node),
                "{id:?}: under a move, a rebuild paints it and the kept scene does not"
            );
        }
    }
}

/// Scene size up to which `apply_delta` re-derives what it retained under
/// `debug_assertions` — [`UiScene::dest_group_candidates`] and, in
/// [`UiScene::audit_retained_projection`], the paint order, the frame plan and
/// the visibility index. Unit-test scenes are a handful of nodes; product
/// scenes are thousands, and auditing those on every delta would slow debug
/// builds without testing anything the small scenes do not.
pub(super) const RETAINED_AUDIT_LIMIT: usize = 512;

/// Whether a node's Markdown drawing holds a native diagram.
#[cfg(debug_assertions)]
fn node_has_markdown_diagram(node: &ExtractedNode) -> bool {
    #[cfg(feature = "rich-text")]
    if let Some(ComponentGeometry::NativeMarkdown { drawing, .. }) =
        node.component_geometry.as_deref()
    {
        return drawing
            .commands
            .iter()
            .any(|c| matches!(c, nana_ui_runtime::MarkdownDrawingCommand::Diagram { .. }));
    }
    let _ = node;
    false
}
