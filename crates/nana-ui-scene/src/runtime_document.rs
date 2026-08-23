use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nana_ui_runtime::{
    AccessibilityDelta, AppContext, DocumentId, FrameStage, FrameworkError, LayoutViewport,
    SystemWork, TextShaper,
};

use crate::{SceneDelta, UiScene};

const MAX_FRAME_PASSES: usize = 8;

/// One backend-neutral retained document and its incrementally extracted scene.
///
/// Canonical hosts call [`Self::flush`] with a viewport and text shaper; this
/// owner guarantees text, layout, hit-test, accessibility, and render
/// extraction are drained in the same bounded frame transaction.
pub struct RuntimeDocument {
    context: AppContext,
    document: DocumentId,
    scene: Arc<UiScene>,
    viewport: Option<LayoutViewport>,
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
            viewport: None,
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

    /// Drain one canonical Runtime frame with host-owned text shaping and
    /// framework-owned layout. Applications do not write widget geometry.
    pub fn flush(
        &mut self,
        viewport: LayoutViewport,
        shaper: &mut impl TextShaper,
    ) -> Result<RuntimeFrameUpdate, FrameworkError> {
        let document = self.document;
        let viewport_changed = self.viewport != Some(viewport);
        if viewport_changed && self.viewport.is_some() {
            self.context.layout_document(document, viewport)?;
        }
        let mut force_layout = viewport_changed;
        let update = self.flush_with(|context, work| {
            context.shape_text(&work.text, shaper)?;
            if force_layout || !work.layout.is_empty() {
                if force_layout {
                    context.layout_document(document, viewport)?;
                } else {
                    // `shape_text` may have marked additional LAYOUT nodes
                    // (intrinsic changes propagate) after the drain; include
                    // them so scoped layout sees the full change set.
                    let mut dirty = work.layout.clone();
                    dirty.extend(context.pending_layout_dirty());
                    context.layout_document_scoped(document, viewport, &dirty)?;
                }
                // Re-shape only the relayout scope: nodes outside it keep
                // shapes that already match their unchanged boxes.
                let mut shape_scope = context.take_last_layout_scope();
                if context.shape_text_for_layout_scoped(&shape_scope, shaper)? {
                    // Shaping re-dirtied layout (empty-state padding, modal
                    // presentations); relayout that closure plus the scope
                    // whose boxes may have shifted again.
                    let mut redirty = context.pending_layout_dirty();
                    redirty.append(&mut shape_scope);
                    redirty.sort_unstable();
                    redirty.dedup();
                    context.layout_document_scoped(document, viewport, &redirty)?;
                }
                force_layout = false;
            }
            Ok(())
        })?;
        // Publish the viewport only after the canonical frame settles. A failed
        // frame must retry layout for the same requested viewport.
        self.viewport = Some(viewport);
        Ok(update)
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
        let mut consumed = Vec::new();
        let mut scene_batches = Vec::new();

        self.context.begin_frame_profile();
        loop {
            let work = self.context.take_system_work();
            if work.is_empty() {
                break;
            }
            if passes == MAX_FRAME_PASSES {
                consumed.push(work);
                restore_work(&mut self.context, consumed);
                self.context.finish_frame_profile();
                return Err(FrameworkError::FrameDidNotSettle);
            }
            passes += 1;

            if let Err(error) = self.context.resolve_styles(&work.style) {
                consumed.push(work);
                restore_work(&mut self.context, consumed);
                self.context.finish_frame_profile();
                return Err(error);
            }
            if let Err(error) = run_text_and_layout(&mut self.context, &work) {
                consumed.push(work);
                restore_work(&mut self.context, consumed);
                self.context.finish_frame_profile();
                return Err(error);
            }
            let scroll_updates = self.context.take_scroll_hit_updates();
            if !work.input_hit_test.is_empty() || !work.layout.is_empty() {
                if work.layout.is_empty()
                    && self
                        .context
                        .hit_test_work_is_scroll_only(&work.input_hit_test, &scroll_updates)
                {
                    // Pure scrolling: patch the scrolled subtree's entry
                    // transforms in place instead of rebuilding the document.
                    for (scroller, delta) in scroll_updates {
                        self.context
                            .update_hit_test_scroll(self.document, scroller, delta);
                    }
                } else {
                    self.context.rebuild_hit_test(self.document);
                }
            }

            let started = std::time::Instant::now();
            let accessibility = self.context.world().project_accessibility_delta(&work);
            self.context
                .time_stage_duration(FrameStage::Accessibility, started.elapsed());
            for removed in accessibility.removed {
                accessibility_updated.remove(&removed);
                accessibility_removed.insert(removed);
            }
            for node in accessibility.updated {
                accessibility_removed.remove(&node.id);
                accessibility_updated.insert(node.id, node);
            }
            let started = std::time::Instant::now();
            let extracted = self.context.world().extract_nodes(&work.render_extraction);
            self.context.record_extract(&extracted);
            self.context
                .time_stage_duration(FrameStage::Extract, started.elapsed());
            scene_batches.push((extracted, work.render_removals.clone()));
            consumed.push(work);
        }
        self.context.finish_frame_profile();

        for (extracted, removals) in scene_batches {
            let scene = Arc::make_mut(&mut self.scene).apply_delta(extracted, removals);
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

fn restore_work(context: &mut AppContext, consumed: Vec<SystemWork>) {
    for work in consumed {
        context.restore_system_work(work);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nana_ui_core::{LayoutStyle, LengthSpec};
    use nana_ui_runtime::{Button, ComputedStyle, StableNodeId, TextContent, TextMetrics};

    use super::*;

    #[test]
    fn frame_driver_is_incremental_and_static_documents_are_idle() {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let button = runtime
            .context_mut()
            .create_component(document, Button::new("Build"))
            .unwrap();
        struct TestShaper;
        impl nana_ui_runtime::TextShaper for TestShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                text: &TextContent,
                _style: &ComputedStyle,
                constraints: nana_ui_runtime::TextShapeConstraints,
            ) -> TextMetrics {
                let intrinsic = text.value.len() as f32 * 8.0;
                let width = constraints.max_width.unwrap_or(intrinsic).min(intrinsic);
                TextMetrics {
                    width,
                    height: if constraints.wrap && width < intrinsic {
                        36.0
                    } else {
                        18.0
                    },
                }
            }
        }

        let first = runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut TestShaper)
            .unwrap();
        assert!(!first.is_idle());
        assert_eq!(first.scene.primitive_count, 2);
        let layout = runtime
            .context()
            .world()
            .layout_box(button.stable_id())
            .unwrap();
        assert!(layout.width > 0.0);
        assert_eq!(layout.height, 32.0);
        let generation = first.generation;
        let first_counters = runtime.context().last_work_counters();
        assert!(first_counters.entities_total >= 1);
        assert!(first_counters.entities_changed > 0);
        assert!(first_counters.render_nodes_extracted > 0);
        let first_profile = runtime.context().last_frame_profile().clone();
        assert_eq!(
            first_profile
                .stage(nana_ui_runtime::FrameStage::Style)
                .unwrap()
                .status,
            nana_ui_runtime::StageStatus::Ran
        );
        assert_eq!(
            first_profile
                .stage(nana_ui_runtime::FrameStage::TextShape)
                .unwrap()
                .status,
            nana_ui_runtime::StageStatus::Ran
        );
        assert_eq!(
            first_profile
                .stage(nana_ui_runtime::FrameStage::Extract)
                .unwrap()
                .status,
            nana_ui_runtime::StageStatus::Ran
        );
        assert_eq!(
            first_profile
                .stage(nana_ui_runtime::FrameStage::GpuUpload)
                .unwrap()
                .status,
            nana_ui_runtime::StageStatus::Unsupported
        );

        let idle = runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut TestShaper)
            .unwrap();
        assert!(idle.is_idle());
        assert_eq!(idle.generation, generation);
        assert_eq!(idle.scene.updated_nodes, 0);
        assert_eq!(runtime.context().last_work_counters(), first_counters);
        assert_eq!(runtime.context().last_frame_profile(), &first_profile);
        assert_eq!(
            runtime
                .context()
                .last_frame_profile()
                .stage(nana_ui_runtime::FrameStage::GpuUpload)
                .unwrap()
                .status,
            nana_ui_runtime::StageStatus::Unsupported
        );
    }

    #[test]
    fn switching_inspector_slot_settles_within_frame_budget() {
        use nana_ui_runtime::{DesktopShell, SidebarFrame, Text};

        struct TestShaper;
        impl nana_ui_runtime::TextShaper for TestShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                text: &TextContent,
                _style: &ComputedStyle,
                constraints: nana_ui_runtime::TextShapeConstraints,
            ) -> TextMetrics {
                let intrinsic = text.value.len() as f32 * 8.0;
                let width = constraints.max_width.unwrap_or(intrinsic).min(intrinsic);
                TextMetrics {
                    width,
                    height: if constraints.wrap && width < intrinsic {
                        36.0
                    } else {
                        18.0
                    },
                }
            }
        }

        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let navigation = runtime
            .context_mut()
            .create_detached_component(document, SidebarFrame::new())
            .unwrap();
        let primary = runtime
            .context_mut()
            .create_detached_component(document, Text::new("stage"))
            .unwrap();
        let first = runtime
            .context_mut()
            .create_detached_component(document, Text::new("inspector-a"))
            .unwrap();
        let second = runtime
            .context_mut()
            .create_detached_component(document, Text::new("inspector-b"))
            .unwrap();
        let shell = runtime
            .context_mut()
            .create_component(
                document,
                DesktopShell::new()
                    .navigation(navigation.stable_id())
                    .primary(primary.stable_id())
                    .inspector(first.stable_id()),
            )
            .unwrap();
        runtime.context_mut().assemble_desktop_shell(shell).unwrap();
        runtime
            .flush(LayoutViewport::new(1280.0, 720.0), &mut TestShaper)
            .unwrap();
        runtime
            .context_mut()
            .set_desktop_slots(shell, Some(primary.stable_id()), Some(second.stable_id()))
            .unwrap();
        let switched = runtime
            .flush(LayoutViewport::new(1280.0, 720.0), &mut TestShaper)
            .unwrap();
        assert!(switched.passes <= 8);
        assert!(!switched.is_idle());
        let idle = runtime
            .flush(LayoutViewport::new(1280.0, 720.0), &mut TestShaper)
            .unwrap();
        assert!(
            idle.is_idle(),
            "inspector switch left dirty work ({} passes)",
            idle.passes
        );
    }

    #[test]
    fn viewport_resize_and_wrapped_text_reflow_without_application_geometry() {
        let document = DocumentId::new(1).unwrap();
        let node = StableNodeId::new(7).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let mut mutations = nana_ui_runtime::MutationQueue::new();
        mutations.create(node, document, nana_ui_runtime::NodeKind::Text);
        mutations.set_text(
            node,
            TextContent {
                value: "a deliberately long line".into(),
            },
        );
        mutations.set_style(
            node,
            nana_ui_runtime::NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    ..LayoutStyle::default()
                }),
                ..nana_ui_runtime::NodeStyle::default()
            },
        );
        runtime.context_mut().commit_mutations(mutations).unwrap();

        struct WrappingShaper;
        impl nana_ui_runtime::TextShaper for WrappingShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                _text: &TextContent,
                _style: &ComputedStyle,
                constraints: nana_ui_runtime::TextShapeConstraints,
            ) -> TextMetrics {
                let width = constraints.max_width.unwrap_or(200.0).min(200.0);
                TextMetrics {
                    width,
                    height: if constraints.wrap && width < 200.0 {
                        36.0
                    } else {
                        18.0
                    },
                }
            }
        }

        runtime
            .flush(LayoutViewport::new(100.0, 80.0), &mut WrappingShaper)
            .unwrap();
        let first = runtime.context().world().layout_box(node).unwrap();
        assert_eq!((first.width, first.height), (100.0, 36.0));

        let resized = runtime
            .flush(LayoutViewport::new(60.0, 80.0), &mut WrappingShaper)
            .unwrap();
        assert!(!resized.is_idle());
        let second = runtime.context().world().layout_box(node).unwrap();
        assert_eq!((second.width, second.height), (60.0, 36.0));
    }

    #[test]
    fn failed_frame_restores_dirty_work_and_does_not_publish_a_partial_scene() {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        runtime
            .context_mut()
            .create_component(document, Button::new("Retry"))
            .unwrap();

        struct RetryShaper(bool);
        impl nana_ui_runtime::TextShaper for RetryShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                text: &TextContent,
                _style: &ComputedStyle,
                _constraints: nana_ui_runtime::TextShapeConstraints,
            ) -> TextMetrics {
                if !self.0 {
                    self.0 = true;
                    return TextMetrics {
                        width: f32::NAN,
                        height: 18.0,
                    };
                }
                TextMetrics {
                    width: text.value.len() as f32 * 8.0,
                    height: 18.0,
                }
            }
        }

        let mut shaper = RetryShaper(false);
        assert!(
            runtime
                .flush(LayoutViewport::new(320.0, 180.0), &mut shaper)
                .is_err()
        );
        assert!(runtime.scene().is_empty());

        let retried = runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut shaper)
            .unwrap();
        assert!(!retried.is_idle());
        assert_eq!(retried.scene.primitive_count, 2);
    }
}
