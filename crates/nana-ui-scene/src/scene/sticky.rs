use super::*;

impl UiScene {
    pub(super) fn sticky_translation(&self, node: &ExtractedNode) -> [f32; 2] {
        if node.source_style.layout.position != nana_ui_core::PositionSpec::Sticky {
            return [0.0; 2];
        }
        // Boxes where they stand now, not where their records were
        // extracted.
        let ancestors = node
            .parent
            .into_iter()
            .flat_map(|parent| ancestor_nodes(&self.nodes, parent))
            .map(|(_, ancestor)| nana_ui_runtime::StickyAncestor {
                style: &ancestor.source_style.layout,
                bounds: self.placed_layout(ancestor),
                boxes: ancestor.extracted_layout.map(|layout| layout.sticky_boxes),
                scroll: ancestor.scroll_offset,
                font_size: ancestor.style.font_size,
                writing: ancestor.style.writing_context(),
            });
        nana_ui_runtime::sticky_translation(
            &node.source_style.layout,
            self.placed_layout(node),
            node.style.font_size,
            node.style.writing_context(),
            ancestors,
        )
    }

    pub(super) fn with_sticky_translation(
        &self,
        node: &ExtractedNode,
        transform: AffineTransform,
    ) -> AffineTransform {
        let [x, y] = self.sticky_translation(node);
        AffineTransform::from_matrix([1.0, 0.0, 0.0, 1.0, x, y]).then(transform)
    }

    /// `node`'s own logical transform on the page, as its children are
    /// handed it: about where its box stands now.
    pub(super) fn logical_page_transform(
        &self,
        node: &ExtractedNode,
        blocks_3d: bool,
    ) -> AffineTransform {
        self.with_sticky_translation(
            node,
            node_scene_transform(
                &node.source_style.layout,
                self.placed_layout(node),
                blocks_3d,
            ),
        )
    }

    /// [`Self::logical_page_transform`]: what `node`'s own primitives are
    /// built through, once put at its origin ([`Self::paint_origin`]).
    pub(super) fn logical_local_transform(
        &self,
        node: &ExtractedNode,
        blocks_3d: bool,
    ) -> AffineTransform {
        self.logical_page_transform(node, blocks_3d)
    }
}
