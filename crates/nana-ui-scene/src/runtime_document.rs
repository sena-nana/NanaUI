use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nana_ui_runtime::{AccessibilityDelta, AppContext, DocumentId, FrameworkError, SystemWork};

use crate::{SceneDelta, UiScene};

const MAX_FRAME_PASSES: usize = 8;

/// One backend-neutral retained document and its incrementally extracted scene.
///
/// Platform adapters provide text/layout work through [`Self::flush_with`];
/// this owner guarantees style, hit-test, accessibility, and render extraction
/// are drained in the same bounded frame transaction.
pub struct RuntimeDocument {
    context: AppContext,
    document: DocumentId,
    scene: Arc<UiScene>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeFrameUpdate {
    pub generation: u64,
    pub passes: usize,
    pub scene: SceneDelta,
    pub accessibility: AccessibilityDelta,
}

impl RuntimeFrameUpdate {
    pub fn is_idle(&self) -> bool {
        self.passes == 0
    }
}

impl RuntimeDocument {
    pub fn new(document: DocumentId) -> Self {
        Self {
            context: AppContext::new(),
            document,
            scene: Arc::new(UiScene::new()),
        }
    }

    pub const fn document(&self) -> DocumentId {
        self.document
    }

    pub fn context(&self) -> &AppContext {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut AppContext {
        &mut self.context
    }

    pub fn scene(&self) -> &UiScene {
        &self.scene
    }

    /// Immutable frame snapshot shared with a renderer without cloning the
    /// primitive stream. The next mutation uses copy-on-write if still in use.
    pub fn shared_scene(&self) -> Arc<UiScene> {
        Arc::clone(&self.scene)
    }

    /// Drain one frame. `run_text_and_layout` may shape dirty text and commit
    /// layout writeback through the public `AppContext` mutation boundary.
    pub fn flush_with(
        &mut self,
        mut run_text_and_layout: impl FnMut(&mut AppContext, &SystemWork) -> Result<(), FrameworkError>,
    ) -> Result<RuntimeFrameUpdate, FrameworkError> {
        let mut passes = 0;
        let mut scene_updated = 0;
        let mut scene_removed = 0;
        let mut rebuilt_primitives = 0;
        let mut order_rebuilt = false;
        let mut accessibility_updated = BTreeMap::new();
        let mut accessibility_removed = BTreeSet::new();

        loop {
            let work = self.context.take_system_work();
            if work.is_empty() {
                break;
            }
            if passes == MAX_FRAME_PASSES {
                return Err(FrameworkError::FrameDidNotSettle);
            }
            passes += 1;

            self.context.resolve_styles(&work.style)?;
            run_text_and_layout(&mut self.context, &work)?;
            if !work.input_hit_test.is_empty() || !work.layout.is_empty() {
                self.context.rebuild_hit_test(self.document);
            }

            let accessibility = self.context.world().project_accessibility_delta(&work);
            for removed in accessibility.removed {
                accessibility_updated.remove(&removed);
                accessibility_removed.insert(removed);
            }
            for node in accessibility.updated {
                accessibility_removed.remove(&node.id);
                accessibility_updated.insert(node.id, node);
            }
            let scene = Arc::make_mut(&mut self.scene).apply_delta(
                self.context.world().extract_nodes(&work.render_extraction),
                work.render_removals,
            );
            scene_updated += scene.updated_nodes;
            scene_removed += scene.removed_nodes;
            rebuilt_primitives += scene.rebuilt_primitives;
            order_rebuilt |= scene.order_rebuilt;
        }

        let generation = self.context.world().generation();
        Ok(RuntimeFrameUpdate {
            generation,
            passes,
            scene: SceneDelta {
                updated_nodes: scene_updated,
                removed_nodes: scene_removed,
                rebuilt_primitives,
                order_rebuilt,
                primitive_count: self.scene.primitives().count(),
            },
            accessibility: AccessibilityDelta {
                generation,
                updated: accessibility_updated.into_values().collect(),
                removed: accessibility_removed.into_iter().collect(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use nana_ui_runtime::{Button, LayoutBox, MutationQueue};

    use super::*;

    #[test]
    fn frame_driver_is_incremental_and_static_documents_are_idle() {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let button = runtime
            .context_mut()
            .create_component(document, Button::new("Build"))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 8.0,
                width: 120.0,
                height: 32.0,
            },
        );
        runtime.context_mut().commit_mutations(layout).unwrap();

        let first = runtime.flush_with(|_, _| Ok(())).unwrap();
        assert!(!first.is_idle());
        assert_eq!(first.scene.primitive_count, 2);
        let generation = first.generation;

        let idle = runtime.flush_with(|_, _| Ok(())).unwrap();
        assert!(idle.is_idle());
        assert_eq!(idle.generation, generation);
        assert_eq!(idle.scene.updated_nodes, 0);
    }
}
