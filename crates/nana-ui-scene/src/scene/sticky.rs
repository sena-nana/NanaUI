use super::*;

impl UiScene {
    pub(super) fn sticky_translation_with(
        &self,
        node: &ExtractedNode,
        placed: LayoutBox,
        precomputed: Option<(&NodeMap<usize>, &[[f32; 2]])>,
    ) -> [f32; 2] {
        if node.source_style.layout.position != nana_ui_core::PositionSpec::Sticky {
            return [0.0; 2];
        }
        let placed_of = |ancestor: &ExtractedNode| -> LayoutBox {
            let [x, y] = match precomputed {
                Some((idx_map, origins)) if self.placement_live => idx_map
                    .get(&ancestor.id)
                    .map_or([ancestor.layout.x, ancestor.layout.y], |&i| origins[i]),
                _ => {
                    super::placement::placed_origin_live(&self.nodes, self.placement_live, ancestor)
                }
            };
            LayoutBox {
                x,
                y,
                ..ancestor.layout
            }
        };
        let ancestors = node
            .parent
            .into_iter()
            .flat_map(|parent| ancestor_nodes(&self.nodes, parent))
            .map(|(_, ancestor)| nana_ui_runtime::StickyAncestor {
                style: &ancestor.source_style.layout,
                bounds: placed_of(ancestor),
                boxes: ancestor.extracted_layout.map(|layout| layout.sticky_boxes),
                scroll: ancestor.scroll_offset,
                font_size: ancestor.style.font_size,
                writing: ancestor.style.writing_context(),
            });
        nana_ui_runtime::sticky_translation(
            &node.source_style.layout,
            placed,
            node.style.font_size,
            node.style.writing_context(),
            ancestors,
        )
    }

    pub(super) fn with_sticky_translation_at(
        &self,
        node: &ExtractedNode,
        placed: LayoutBox,
        transform: AffineTransform,
        precomputed: Option<(&NodeMap<usize>, &[[f32; 2]])>,
    ) -> AffineTransform {
        let [x, y] = self.sticky_translation_with(node, placed, precomputed);
        AffineTransform::from_matrix([1.0, 0.0, 0.0, 1.0, x, y]).then(transform)
    }

    /// `node`'s own logical transform on the page, as its children are
    /// handed it: about where its box stands now.
    pub(super) fn logical_page_transform_at(
        &self,
        node: &ExtractedNode,
        placed: LayoutBox,
        blocks_3d: bool,
        parent_perspective: Option<nana_ui_core::ParentPerspective>,
        precomputed: Option<(&NodeMap<usize>, &[[f32; 2]])>,
    ) -> AffineTransform {
        self.with_sticky_translation_at(
            node,
            placed,
            node_scene_transform(
                &node.source_style.layout,
                placed,
                blocks_3d,
                parent_perspective,
            ),
            precomputed,
        )
    }

    /// [`Self::logical_page_transform`]: what `node`'s own primitives are
    /// built through, once put at its origin ([`Self::paint_origin`]).
    pub(super) fn logical_local_transform(
        &self,
        node: &ExtractedNode,
        blocks_3d: bool,
        parent_perspective: Option<nana_ui_core::ParentPerspective>,
    ) -> AffineTransform {
        let placed = super::placement::placed_origin_live(&self.nodes, self.placement_live, node)
            .into_layout(node);
        self.logical_page_transform_at(node, placed, blocks_3d, parent_perspective, None)
    }
}
