use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nana_ui_runtime::{
    AccessibilityDelta, AppContext, DocumentId, FrameStage, FrameworkError, LayoutViewport,
    StableNodeId, SystemWork, TextShaper,
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

    /// Drain one Runtime frame: host shaping, framework layout. A viewport
    /// change relayouts from the roots and reuses every subtree the new size
    /// did not move.
    pub fn flush(
        &mut self,
        viewport: LayoutViewport,
        shaper: &mut impl TextShaper,
    ) -> Result<RuntimeFrameUpdate, FrameworkError> {
        let document = self.document;
        let viewport_changed = self.viewport != Some(viewport);
        let mut force_layout = viewport_changed;
        let update = self.flush_loop(viewport_changed, |context, work| {
            context.world_mut().reconcile_focus(&work.focus_ime);
            context.shape_text(&work.text, shaper)?;
            if force_layout || !work.layout.is_empty() {
                if force_layout {
                    context.layout_document_for_viewport(document, viewport)?;
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
        run_text_and_layout: impl FnMut(&mut AppContext, &SystemWork) -> Result<(), FrameworkError>,
    ) -> Result<RuntimeFrameUpdate, FrameworkError> {
        self.flush_loop(false, run_text_and_layout)
    }

    fn flush_loop(
        &mut self,
        mut force_pass: bool,
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
        let mut hit_dirty: Vec<StableNodeId> = Vec::new();
        let mut hit_scroll_updates: Vec<(StableNodeId, [f32; 2])> = Vec::new();

        self.context.begin_frame_profile();
        loop {
            let work = self.context.take_system_work();
            if work.is_empty() && !force_pass {
                break;
            }
            force_pass = false;
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
            // Hit-test work is accumulated and applied once after the loop
            // settles. Text/layout feedback can re-dirty the same nodes across
            // passes, and rebuilding per pass paid for the index N times while
            // only the last result was ever observable.
            hit_scroll_updates.extend(self.context.take_scroll_hit_updates());
            hit_dirty.extend(work.input_hit_test.iter().copied());

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
        self.apply_hit_test_work(hit_dirty, hit_scroll_updates);
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

    /// Bring the hit index up to date once for the settled frame.
    ///
    /// `dirty` is the INPUT set, which layout writeback marks on exactly the
    /// nodes whose box moved. Scheduled-layout nodes are deliberately not used:
    /// layout invalidation propagates to ancestors, so patching from it would
    /// rebuild from the document root for any leaf resize.
    ///
    /// Pure scrolling patches entry transforms in place. Otherwise the changed
    /// subtrees are spliced, and only a structural change the splice cannot
    /// express falls back to a full document rebuild.
    fn apply_hit_test_work(
        &mut self,
        mut dirty: Vec<StableNodeId>,
        scroll_updates: Vec<(StableNodeId, [f32; 2])>,
    ) {
        if dirty.is_empty() {
            return;
        }
        dirty.sort_unstable();
        dirty.dedup();
        if self
            .context
            .hit_test_work_is_scroll_only(&dirty, &scroll_updates)
        {
            for (scroller, delta) in scroll_updates {
                self.context
                    .update_hit_test_scroll(self.document, scroller, delta);
            }
            return;
        }
        self.context.rebuild_hit_test_for(self.document, &dirty);
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
            .build(document, |ui| ui.child("build", Button::new("Build")))
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
                    ascent: None,
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

        let full_before_resize = runtime.context().layout_full_invocations();
        let passes_before_resize = runtime.context().layout_invocations();
        let resized = runtime
            .flush(LayoutViewport::new(640.0, 180.0), &mut TestShaper)
            .unwrap();
        assert!(!resized.is_idle());
        assert_eq!(
            runtime.context().layout_invocations() - passes_before_resize,
            1,
            "viewport change must run one layout pass, not a pre-layout plus drain layout"
        );
        assert_eq!(
            runtime.context().layout_full_invocations() - full_before_resize,
            0,
            "a document with no viewport-relative box must resize against the retained cache"
        );
    }

    #[test]
    fn a_viewport_relative_box_follows_a_resize_without_discarding_the_cache() {
        use nana_ui_core::{LayoutStyle, LengthSpec, ViewportAxis};

        struct TestShaper;
        impl nana_ui_runtime::TextShaper for TestShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                text: &TextContent,
                _style: &ComputedStyle,
                _constraints: nana_ui_runtime::TextShapeConstraints,
            ) -> TextMetrics {
                TextMetrics {
                    width: text.value.len() as f32 * 8.0,
                    height: 18.0,
                    ascent: None,
                }
            }
        }

        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let half_viewport = LayoutStyle {
            height: Some(LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: 50.0,
            }),
            ..Default::default()
        };
        let button = runtime
            .context_mut()
            .build(document, |ui| {
                ui.child(
                    "build",
                    Button::new("Build").layout(Arc::new(half_viewport)),
                )
            })
            .unwrap();
        runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut TestShaper)
            .unwrap();
        assert_eq!(
            runtime
                .context()
                .world()
                .layout_box(button.stable_id())
                .unwrap()
                .height,
            90.0
        );

        let full_before_resize = runtime.context().layout_full_invocations();
        runtime
            .flush(LayoutViewport::new(320.0, 360.0), &mut TestShaper)
            .unwrap();
        assert_eq!(
            runtime.context().layout_full_invocations() - full_before_resize,
            0,
            "a vh box dirties with the document roots and keeps the retained cache"
        );
        assert_eq!(
            runtime
                .context()
                .world()
                .layout_box(button.stable_id())
                .unwrap()
                .height,
            180.0,
            "the vh box must follow the new viewport"
        );
    }

    #[test]
    fn a_fixed_overlay_follows_a_resize_without_a_full_layout() {
        use nana_ui_core::{LayoutStyle, LengthSpec, PositionSpec};

        struct TestShaper;
        impl nana_ui_runtime::TextShaper for TestShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                text: &TextContent,
                _style: &ComputedStyle,
                _constraints: nana_ui_runtime::TextShapeConstraints,
            ) -> TextMetrics {
                TextMetrics {
                    width: text.value.len() as f32 * 8.0,
                    height: 18.0,
                }
            }
        }

        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let overlay = LayoutStyle {
            position: PositionSpec::Fixed,
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(24.0)),
            offset_left: Some(LengthSpec::Px(8.0)),
            offset_top: Some(LengthSpec::Px(8.0)),
            ..Default::default()
        };
        let button = runtime
            .context_mut()
            .build(document, |ui| {
                ui.child("overlay", Button::new("Go").layout(Arc::new(overlay)))
            })
            .unwrap();
        runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut TestShaper)
            .unwrap();
        assert!(
            runtime.context().world().uses_viewport_basis(),
            "position:fixed must register as a viewport-basis box"
        );
        let before = runtime
            .context()
            .world()
            .layout_box(button.stable_id())
            .unwrap();
        assert_eq!((before.width, before.height), (40.0, 24.0));

        let full_before_resize = runtime.context().layout_full_invocations();
        runtime
            .flush(LayoutViewport::new(640.0, 360.0), &mut TestShaper)
            .unwrap();
        assert_eq!(
            runtime.context().layout_full_invocations() - full_before_resize,
            0,
            "fixed overlay presence must not discard the retained layout cache"
        );
        let after = runtime
            .context()
            .world()
            .layout_box(button.stable_id())
            .unwrap();
        assert_eq!((after.width, after.height), (40.0, 24.0));
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
                    ascent: None,
                }
            }
        }

        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let (primary, _first, second, shell) = runtime
            .context_mut()
            .build(document, |ui| {
                let navigation = ui.leaf(SidebarFrame::new());
                let primary = ui.leaf(Text::new("stage"));
                let first = ui.leaf(Text::new("inspector-a"));
                let second = ui.leaf(Text::new("inspector-b"));
                let shell = ui.child(
                    "shell",
                    DesktopShell::new()
                        .navigation(navigation.stable_id())
                        .primary(primary.stable_id())
                        .inspector(first.stable_id()),
                );
                (primary, first, second, shell)
            })
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
    fn empty_state_keeps_its_text_block_across_reprojection() {
        use nana_ui_runtime::EmptyState;

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
                    height: 18.0,
                    ascent: None,
                }
            }
        }

        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let empty = runtime
            .context_mut()
            .build(document, |ui| {
                ui.child("empty", EmptyState::new("Nothing here yet"))
            })
            .unwrap();
        runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut TestShaper)
            .unwrap();
        let settled = runtime
            .context()
            .world()
            .layout_box(empty.stable_id())
            .unwrap()
            .height;
        assert!(
            settled > 48.0,
            "shaped title must add height on top of the 24px insets, got {settled}"
        );

        // Projection is unconditional, so even an update that changes nothing
        // rewrites the style carrying the shaped text block.
        runtime
            .context_mut()
            .update_component(empty, |_, _| {})
            .unwrap();
        runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut TestShaper)
            .unwrap();

        let after = runtime
            .context()
            .world()
            .layout_box(empty.stable_id())
            .unwrap()
            .height;
        assert_eq!(
            after, settled,
            "re-projection collapsed the empty state's text block"
        );

        let idle = runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut TestShaper)
            .unwrap();
        assert!(
            idle.is_idle(),
            "republishing the shaped block must not keep the document dirty ({} passes)",
            idle.passes
        );
    }

    #[test]
    fn multi_pass_flush_updates_the_hit_index_once() {
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
                    ascent: None,
                }
            }
        }

        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let (inspector, shell) = runtime
            .context_mut()
            .build(document, |ui| {
                let navigation = ui.leaf(SidebarFrame::new());
                let primary = ui.leaf(Text::new("stage"));
                let inspector = ui.leaf(Text::new("inspector"));
                let shell = ui.child(
                    "shell",
                    DesktopShell::new()
                        .navigation(navigation.stable_id())
                        .primary(primary.stable_id())
                        .inspector(inspector.stable_id()),
                );
                (inspector, shell)
            })
            .unwrap();
        runtime.context_mut().assemble_desktop_shell(shell).unwrap();

        let first = runtime
            .flush(LayoutViewport::new(1280.0, 720.0), &mut TestShaper)
            .unwrap();
        let built = hit_entries_built(&runtime);
        let live = runtime.context().world().len();

        // The settle loop ran several passes. Rebuilding per pass cost the index
        // once per pass while only the last result was ever observable, so the
        // frame's total must stay within a single document's worth of entries.
        assert!(first.passes > 1, "expected a multi-pass settle");
        assert!(
            built <= live,
            "{} passes built {built} hit entries for {live} nodes",
            first.passes
        );

        // Retyping one leaf must patch only what moved. Layout invalidation
        // propagates to ancestors, so a patch keyed on scheduled layout instead
        // of written boxes lands back at the document root and rebuilds the
        // whole index for a single label.
        let mut retype = nana_ui_runtime::MutationQueue::new();
        retype.set_text(
            inspector.stable_id(),
            TextContent {
                value: "inspector pane".into(),
            },
        );
        runtime.context_mut().commit_mutations(retype).unwrap();
        runtime
            .flush(LayoutViewport::new(1280.0, 720.0), &mut TestShaper)
            .unwrap();
        let edit_built = hit_entries_built(&runtime);
        assert!(
            edit_built < live,
            "one leaf edit built {edit_built} hit entries for {live} nodes; expected a patch, not a rebuild"
        );
    }

    /// Hit entries the last flush built. Frame counters are scoped to the
    /// flush, so this covers every settle pass in it.
    fn hit_entries_built(runtime: &RuntimeDocument) -> usize {
        runtime
            .context()
            .world()
            .last_work_counters()
            .hit_test_nodes_rebuilt
            .unwrap_or_default()
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
                    ascent: None,
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
            .build(document, |ui| ui.child("retry", Button::new("Retry")))
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
                        ascent: None,
                    };
                }
                TextMetrics {
                    width: text.value.len() as f32 * 8.0,
                    height: 18.0,
                    ascent: None,
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
