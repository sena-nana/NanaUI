use super::*;

impl UiWorld {
    pub(super) fn rebind_sticky(&mut self, id: StableNodeId) {
        if self
            .nodes
            .get(id)
            .is_some_and(|node| node.resolved_layout.position == PositionSpec::Sticky)
        {
            let mut cursor = self.parent_id(id);
            let ancestors: Vec<_> = std::iter::from_fn(|| {
                let id = cursor?;
                let node = self.nodes.get(id)?;
                let style = &node.resolved_layout;
                cursor = if style.overflow_x.clips()
                    || style.overflow_y.clips()
                    || style.position == PositionSpec::Fixed
                {
                    None
                } else {
                    node.hierarchy.parent
                };
                Some(id)
            })
            .collect();
            self.sticky_nodes.bind(id, ancestors.into_iter());
        } else {
            self.sticky_nodes.remove(id);
        }
    }

    pub(super) fn rebind_sticky_subtree(&mut self, root: StableNodeId) {
        let affected = self.sticky_nodes.affected(root);
        self.rebind_sticky(root);
        for id in affected {
            self.rebind_sticky(id);
        }
    }
    pub(super) fn sticky_translation(&self, id: StableNodeId) -> [f32; 2] {
        let Some(node) = self.nodes.get(id) else {
            return [0.0; 2];
        };
        if node.resolved_layout.position != PositionSpec::Sticky {
            return [0.0; 2];
        }
        // Every box is read from its local one, measured from the node's own
        // origin: the projection reads only where they stand apart, and a
        // move above them all leaves it as it was.
        let mut chain: Vec<(
            StableNodeId,
            &NodeRecord,
            [f32; 2],
            Option<crate::StickyBoxes>,
        )> = Vec::new();
        let mut child = id;
        let mut origin = [0.0f32; 2];
        let mut cursor = node.hierarchy.parent;
        while let Some(ancestor) = cursor {
            let Some(record) = self.nodes.get(ancestor) else {
                break;
            };
            let offset = self.layout_box_from(ancestor, child).unwrap_or_default();
            origin = [origin[0] - offset.x, origin[1] - offset.y];
            let boxes = self.sticky_boxes(ancestor);
            chain.push((ancestor, record, origin, boxes));
            let style = &record.resolved_layout;
            if style.overflow_x.clips()
                || style.overflow_y.clips()
                || style.position == PositionSpec::Fixed
            {
                break;
            }
            child = ancestor;
            cursor = record.hierarchy.parent;
        }
        let ancestors =
            chain.iter().map(
                |(ancestor_id, record, [x, y], boxes)| crate::StickyAncestor {
                    style: &record.resolved_layout,
                    bounds: LayoutBox {
                        x: *x,
                        y: *y,
                        ..record.local_layout
                    },
                    boxes: *boxes,
                    scroll: self.nodes.scroll_offset(*ancestor_id),
                    font_size: record.resolved.0.font_size,
                    writing: record.resolved.0.writing_context(),
                },
            );
        crate::sticky_translation(
            &node.resolved_layout,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                ..node.local_layout
            },
            node.resolved.0.font_size,
            node.resolved.0.writing_context(),
            ancestors,
        )
    }

    pub(super) fn with_sticky_translation(
        &self,
        id: StableNodeId,
        local: ([f32; 6], [f32; 2]),
    ) -> ([f32; 6], [f32; 2]) {
        let [x, y] = self.sticky_translation(id);
        if x == 0.0 && y == 0.0 {
            local
        } else {
            then_hit(([1.0, 0.0, 0.0, 1.0, x, y], [0.0; 2]), local)
        }
    }
}
