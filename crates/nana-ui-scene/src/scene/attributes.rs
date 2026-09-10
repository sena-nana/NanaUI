//! Shared projection attributes. Geometry is retained across scroll updates.
use super::*;

#[derive(Debug, Clone)]
pub(super) struct DrawAttributes {
    epoch: u64,
    delta: AffineTransform,
    parent_clips: Arc<[ClipRegion]>,
}

/// A retained primitive with the current inherited transform and clip chain.
/// Dereferencing exposes its immutable geometry; no geometry is regenerated.
pub struct SceneDraw<'a> {
    pub primitive: &'a ScenePrimitive,
    pub transform: AffineTransform,
    pub clips: Arc<[ClipRegion]>,
}
impl std::ops::Deref for SceneDraw<'_> {
    type Target = ScenePrimitive;
    fn deref(&self) -> &Self::Target {
        self.primitive
    }
}
pub(super) fn inverse(transform: AffineTransform) -> Option<AffineTransform> {
    if transform.is_projective() {
        return None;
    }
    let [a, b, c, d, e, f] = transform.0;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-8 {
        return None;
    }
    Some(AffineTransform::from_matrix([
        d / det,
        -b / det,
        -c / det,
        a / det,
        (c * f - d * e) / det,
        (b * e - a * f) / det,
    ]))
}
impl UiScene {
    /// Current projected node bounds, including ancestor scrolling. Unknown
    /// projective bounds return `None`, requiring conservative composition.
    pub fn draw_node_bounds(&self, id: StableNodeId) -> Option<SceneRect> {
        let node = self.nodes.get(&id)?;
        let (parent, _, _, blocks_3d) = self.ancestor_state(node);
        let transform = parent.then(node_scene_transform(
            &node.source_style.layout,
            node.layout,
            blocks_3d,
        ));
        super::visibility::transform(self.node_bounds(id)?, transform)
    }

    pub fn draw_primitive(&self, id: PrimitiveId) -> Option<SceneDraw<'_>> {
        let primitive = self.primitive(id)?;
        let Some(&(epoch, base_transform, parent_clip_count)) = self.projections.get(&id.node)
        else {
            return Some(SceneDraw {
                primitive,
                transform: primitive.transform,
                clips: Arc::clone(&primitive.clips),
            });
        };
        if epoch == self.attribute_epoch {
            return Some(SceneDraw {
                primitive,
                transform: primitive.transform,
                clips: Arc::clone(&primitive.clips),
            });
        }
        let cached = self
            .draw_attributes
            .lock()
            .expect("scene attributes")
            .get(&id.node)
            .filter(|entry| entry.epoch == self.attribute_epoch)
            .cloned();
        let attributes = if let Some(attributes) = cached {
            attributes
        } else {
            let node = self.nodes.get(&id.node)?;
            let (parent, _, parent_clips, blocks_3d) = self.ancestor_state(node);
            let current = parent.then(node_scene_transform(
                &node.source_style.layout,
                node.layout,
                blocks_3d,
            ));
            let delta = inverse(base_transform)
                .map_or(AffineTransform::IDENTITY, |inverse| current.then(inverse));
            let attributes = DrawAttributes {
                epoch: self.attribute_epoch,
                delta,
                parent_clips,
            };
            self.draw_attributes
                .lock()
                .expect("scene attributes")
                .insert(id.node, attributes.clone());
            attributes
        };
        let mut clips = attributes.parent_clips.to_vec();
        clips.extend(
            primitive
                .clips
                .iter()
                .skip(parent_clip_count)
                .cloned()
                .map(|mut clip| {
                    clip.transform = attributes.delta.then(clip.transform);
                    clip
                }),
        );
        Some(SceneDraw {
            primitive,
            transform: attributes.delta.then(primitive.transform),
            clips: clips.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_runtime::{
        ComponentElevation, ComponentTextRegion, ComputedStyle, NodeKind, NodeStyle,
    };

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn node(value: u64, parent: Option<u64>, children: &[u64]) -> ExtractedNode {
        ExtractedNode {
            id: id(value),
            kind: Arc::new(NodeKind::Element { tag: "div".into() }),
            parent: parent.map(id),
            children: Arc::new(children.iter().copied().map(id).collect()),
            layout: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            scroll_offset: Default::default(),
            source_style: NodeStyle::default(),
            style: Arc::new(ComputedStyle {
                background: Some([0.0, 1.0, 0.0, 1.0]),
                ..Default::default()
            }),
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

    fn title(bounds: LayoutBox) -> ComponentTextRegion {
        ComponentTextRegion {
            bounds,
            content: "".into(),
            color: None,
            font_size: 14.0,
            font_weight: None,
        }
    }

    fn set_component_clip(
        parent: &mut ExtractedNode,
        modal: bool,
        height: f32,
        body_root: Option<StableNodeId>,
    ) {
        let surface = LayoutBox {
            height,
            ..parent.layout
        };
        let body = LayoutBox {
            height: height / 2.0,
            ..parent.layout
        };
        parent.component_geometry = Some(Box::new(if modal {
            ComponentGeometry::ModalFrame {
                scrim: parent.layout,
                surface,
                body,
                title: title(parent.layout),
                description: None,
                body_text: None,
                background: [0.0; 4],
                border: [0.0; 4],
                elevation: ComponentElevation::surface_shadow(nana_ui_core::ThemeMode::Light),
            }
        } else {
            ComponentGeometry::EmptyState {
                root_clip: surface,
                content_clip: surface,
                icon: None,
                title: title(parent.layout),
                message: None,
                action: None,
            }
        }));
        parent.standard_visual = modal.then(|| StandardVisual::ModalFrame {
            title: "".into(),
            description: None,
            body_text: None,
            kind: nana_ui_runtime::ModalSurfaceKind::Dialog(Default::default()),
            busy: false,
            danger: false,
            slots: nana_ui_runtime::ModalSlots {
                body: body_root,
                ..Default::default()
            },
        });
    }

    #[test]
    fn component_clip_geometry_and_modal_body_membership_update_retained_children() {
        for modal in [false, true] {
            let mut parent = node(1, None, &[2, 3]);
            let child = node(2, Some(1), &[]);
            let other = node(3, Some(1), &[]);
            set_component_clip(&mut parent, modal, 80.0, Some(id(2)));
            let mut scene = UiScene::new();
            scene.apply_delta([parent.clone(), child.clone(), other.clone()], []);
            let primitive = scene.primitives().find(|p| p.node == id(2)).unwrap().id;
            let retained = scene.primitive(primitive).unwrap().clone();
            let plan = scene.frame_plan().unwrap();
            for (height, body_root) in [
                (40.0, Some(id(2))),
                (40.0, Some(id(3))),
                (60.0, Some(id(3))),
            ] {
                set_component_clip(&mut parent, modal, height, body_root);
                let delta = scene.apply_delta([parent.clone()], []);
                let mut fresh = UiScene::new();
                fresh.apply_delta([parent.clone(), child.clone(), other.clone()], []);
                assert_eq!(
                    scene.draw_primitive(primitive).unwrap().clips,
                    fresh.draw_primitive(primitive).unwrap().clips
                );
                assert_eq!(
                    scene.primitive(primitive).unwrap(),
                    &retained,
                    "invertible child geometry remains retained"
                );
                assert!(Arc::ptr_eq(&plan, &scene.frame_plan().unwrap()));
                assert_eq!(
                    delta.stats.rebuilt_primitives,
                    scene.primitives_for_node(id(1)).count()
                );
            }
        }
    }

    #[test]
    fn only_unadjustable_descendants_rebuild_when_the_ancestor_moves_or_scrolls() {
        for projective in [false, true] {
            let mut parent = node(1, None, &[2, 3]);
            Arc::make_mut(&mut parent.source_style.layout).transform = Some(Default::default());
            let mut child = node(2, Some(1), &[]);
            if projective {
                Arc::make_mut(&mut child.source_style.layout).transform_3d = Some(
                    nana_ui_core::PaintMat4::perspective(800.0)
                        .unwrap()
                        .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians())),
                );
            } else {
                Arc::make_mut(&mut child.source_style.layout).transform =
                    Some(nana_ui_core::PaintTransform {
                        a: 0.0,
                        ..Default::default()
                    });
            }
            let sibling = node(3, Some(1), &[]);
            let mut scene = UiScene::new();
            scene.apply_delta([parent.clone(), child.clone(), sibling.clone()], []);
            let primitive = scene.primitives().find(|p| p.node == id(2)).unwrap().id;
            let sibling_primitive = scene.primitives().find(|p| p.node == id(3)).unwrap().id;
            let retained = scene.primitive(sibling_primitive).unwrap().clone();
            assert!(scene.unadjustable_projections.contains(&id(2)));
            let plan = scene.frame_plan().unwrap();
            scene
                .visible_operations(SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 80.0,
                })
                .unwrap();
            for x in [200.0, 400.0] {
                Arc::make_mut(&mut parent.source_style.layout)
                    .transform
                    .as_mut()
                    .unwrap()
                    .e = x;
                let delta = scene.apply_delta([parent.clone()], []);
                let mut fresh = UiScene::new();
                fresh.apply_delta([parent.clone(), child.clone(), sibling.clone()], []);
                assert_eq!(
                    scene.draw_primitive(primitive).unwrap().transform,
                    fresh.draw_primitive(primitive).unwrap().transform
                );
                assert_eq!(scene.primitive(sibling_primitive).unwrap(), &retained);
                assert_eq!(
                    delta.stats.rebuilt_primitives,
                    scene.primitives_for_node(id(1)).count()
                        + scene.primitives_for_node(id(2)).count()
                );
                assert!(Arc::ptr_eq(&plan, &scene.frame_plan().unwrap()));
                let viewport = SceneRect {
                    x,
                    y: 0.0,
                    width: 100.0,
                    height: 80.0,
                };
                assert_eq!(
                    scene.visible_operations(viewport).unwrap(),
                    fresh.visible_operations(viewport).unwrap()
                );
            }
            // A plain scroll also updates a singular child while keeping its
            // invertible sibling on the retained translation path.
            parent.scroll_offset.y = 20.0;
            let delta = scene.apply_delta([parent.clone()], []);
            assert_eq!(scene.primitive(sibling_primitive).unwrap(), &retained);
            assert_eq!(
                delta.stats.rebuilt_primitives,
                scene.primitives_for_node(id(1)).count() + scene.primitives_for_node(id(2)).count()
            );
            let mut fresh = UiScene::new();
            fresh.apply_delta([parent, child, sibling], []);
            assert_eq!(
                scene.draw_primitive(primitive).unwrap().transform,
                fresh.draw_primitive(primitive).unwrap().transform
            );
        }
    }

    #[test]
    fn removing_an_ancestor_drops_its_retained_descendants_clip_and_transform() {
        let mut parent = node(1, None, &[2]);
        Arc::make_mut(&mut parent.source_style.layout).overflow_y =
            nana_ui_core::OverflowSpec::Hidden;
        Arc::make_mut(&mut parent.source_style.layout).transform =
            Some(nana_ui_core::PaintTransform {
                e: 30.0,
                ..Default::default()
            });
        let child = node(2, Some(1), &[]);
        let mut scene = UiScene::new();
        scene.apply_delta([parent, child.clone()], []);
        let primitive = scene.primitives().find(|p| p.node == id(2)).unwrap().id;
        assert!(!scene.draw_primitive(primitive).unwrap().clips.is_empty());
        scene.apply_delta([], [id(1)]);
        let mut fresh = UiScene::new();
        fresh.apply_delta([child], []);
        assert_eq!(
            scene.draw_primitive(primitive).unwrap().clips,
            fresh.draw_primitive(primitive).unwrap().clips
        );
        assert_eq!(
            scene.draw_primitive(primitive).unwrap().transform,
            fresh.draw_primitive(primitive).unwrap().transform
        );
    }
}
