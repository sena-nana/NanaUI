//! The retained primitive table: inserting, retiring and listing a node's
//! primitives.

use super::*;

impl UiScene {
    /// What a frame plan reads out of a primitive beyond its identity.
    fn custom_binding(kind: &ScenePrimitiveKind) -> Option<(&Arc<str>, &Arc<str>)> {
        match kind {
            ScenePrimitiveKind::Custom { node, .. } if node.payload.is_none() => {
                Some((&node.renderer, &node.resource))
            }
            _ => None,
        }
    }
}

impl UiScene {
    pub(super) fn remove_node_primitives(&mut self, id: StableNodeId) {
        self.glyph_live.remove(&id);
        self.gpu_payload_live.remove(&id);
        self.widget_vector_paint.remove(&id);
        self.glyph_objects.remove(&id);
        self.retire_node_primitives(id, |_| true);
    }

    /// Drop the node's primitives that `doomed` names, and their places in
    /// `ordered`.
    ///
    /// The key comes off the primitive rather than out of a fresh ancestor
    /// walk: a key computed against a node the delta has already replaced
    /// would not match the one this primitive entered `ordered` at, and would
    /// leave it there forever.
    pub(super) fn retire_node_primitives(
        &mut self,
        id: StableNodeId,
        doomed: impl Fn(&RetainedPrimitive) -> bool,
    ) {
        let slots = self
            .node_primitives(id)
            .filter(|(_, held)| doomed(held))
            .map(|(slot, _)| *slot)
            .collect::<Vec<_>>();
        for slot in slots {
            if let Some(held) = self.primitives.remove(&slot) {
                self.ordered.remove(&held.key);
                self.structure_changed = true;
                self.frame_plan_resources_changed |=
                    super::composition::primitive_has_summary_resource(&held.primitive.kind);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn primitives_for_node(
        &self,
        node: StableNodeId,
    ) -> impl Iterator<Item = &ScenePrimitive> {
        self.node_primitives(node).map(|(_, held)| &held.primitive)
    }

    pub(super) fn node_primitive_count(&self, node: StableNodeId) -> usize {
        self.node_primitives(node).count()
    }

    fn node_primitives(
        &self,
        node: StableNodeId,
    ) -> impl Iterator<Item = (&PrimitiveId, &RetainedPrimitive)> {
        self.primitives.range(
            PrimitiveId { node, slot: 0 }..=PrimitiveId {
                node,
                slot: u64::MAX,
            },
        )
    }

    pub(super) fn insert_primitive(&mut self, primitive: ScenePrimitive) {
        let key = paint_order::scene_order_key(
            &self.nodes,
            &self.node_order,
            &self.custom_paint,
            &mut self.rebuild_scratch,
            &primitive,
            self.primitives
                .get(&primitive.id)
                .map(|held| &held.key.stack),
        );
        let build = self.build;
        let origin = self.paint_origin_mut(primitive.node);
        let inherited_clips = self
            .projections
            .get(&primitive.node)
            .map_or(primitive.clips.len(), |&(_, _, count)| count)
            .min(primitive.clips.len());
        if let Some(held) = self.primitives.get_mut(&primitive.id) {
            // A rebuild usually puts the same primitive back in the same
            // place. Re-entering `ordered` at a key it already holds is the
            // work this avoids.
            let moved = held.key != key;
            // A frame plan names custom nodes by renderer and resource, so one
            // slot swapping which it draws is a structural change even though
            // the slot itself stays.
            self.structure_changed |=
                Self::custom_binding(&held.primitive.kind) != Self::custom_binding(&primitive.kind);
            self.frame_plan_resources_changed |=
                super::composition::primitive_has_summary_resource(&held.primitive.kind)
                    || super::composition::primitive_has_summary_resource(&primitive.kind);
            // A painter moving `draw_default()` to the other side of the
            // children re-keys the same slots with nothing else changing: the
            // plan has to hear it (Issue #217).
            self.structure_changed |= moved
                && !self.custom_paint.is_empty()
                && has_custom_paint(&self.nodes, primitive.node);
            held.primitive = primitive;
            held.build = build;
            held.origin = origin;
            held.inherited_clips = inherited_clips;
            if moved {
                let previous = std::mem::replace(&mut held.key, key.clone());
                self.ordered.remove(&previous);
                self.ordered.insert(key);
            }
            return;
        }
        self.structure_changed = true;
        self.frame_plan_resources_changed |=
            super::composition::primitive_has_summary_resource(&primitive.kind);
        self.primitives.insert(
            primitive.id,
            RetainedPrimitive {
                primitive,
                key: key.clone(),
                build,
                origin,
                inherited_clips,
            },
        );
        self.ordered.insert(key);
    }
}
