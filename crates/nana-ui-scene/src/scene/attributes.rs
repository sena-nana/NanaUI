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
fn inverse(transform: AffineTransform) -> Option<AffineTransform> {
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
