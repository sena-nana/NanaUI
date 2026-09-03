//! Vue host frame boundary.

use crate::*;

impl VueHost {
    pub fn set_viewport(&mut self, physical_width: u32, physical_height: u32, scale_factor: f32) {
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.set_viewport(physical_width, physical_height, scale_factor);
        bridge.resolve_document_layout(&mut doc);
    }
    /// Commit host ops and flush Runtime layout/extract like a window Scene host.
    /// Headless sessions call this after [`Self::semantic_snapshot`].
    #[cfg(feature = "scene-view")]
    pub fn flush_scene_frame(
        &mut self,
        logical_width: f32,
        logical_height: f32,
    ) -> Result<(), nana_ui_runtime::FrameworkError> {
        {
            let mut doc = self.document.lock().expect("vue doc");
            doc.flush_host_frame();
            self.report_commit_rejections(&mut doc);
        }
        self.flush_runtime_scene(logical_width, logical_height)?;

        let records: Vec<(u64, nana_ui_scene::SceneRect)> = {
            let document = self.document.lock().expect("vue doc");
            let runtime = document.runtime_document();
            let document_id = runtime.document();
            let scene = runtime.scene();
            runtime
                .context()
                .world()
                .document_order(document_id)
                .into_iter()
                .filter_map(|id| scene.node_bounds(id).map(|rect| (id.get(), rect)))
                .collect()
        };
        self.layout_boxes.begin_frame();
        for (id, rect) in records {
            self.layout_boxes
                .record(NodeHandle(id), rect.x, rect.y, rect.width, rect.height);
        }
        Ok(())
    }
    #[cfg(feature = "scene-view")]
    pub(crate) fn flush_runtime_scene(
        &mut self,
        logical_width: f32,
        logical_height: f32,
    ) -> Result<(), nana_ui_runtime::FrameworkError> {
        self.document
            .lock()
            .expect("vue doc")
            .runtime_document_mut()
            .flush(
                nana_ui_runtime::LayoutViewport::new(logical_width, logical_height),
                &mut nana_ui::NanaTextShaper::default(),
            )?;
        Ok(())
    }
    pub fn resolve_layout(&mut self) {
        let painted = {
            let doc = self.document.lock().expect("vue doc");
            self.layout_boxes
                .retain(|id| doc.contains_handle(NodeHandle(id)));
            self.layout_boxes.snapshot()
        };
        if painted.is_empty() {
            // Empty paint cache: keep Runtime boxes; CSS auto-height 0 must not overwrite them.
            let mut bridge = self.bridge.lock().expect("vue bridge");
            let mut doc = self.document.lock().expect("vue doc");
            doc.flush_host_frame();
            #[cfg(feature = "scene-view")]
            self.report_commit_rejections(&mut doc);
            bridge.resolve_missing_document_layout(&mut doc);
            return;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.apply_layout_boxes(&painted);
        reapply_scroll_translations(&mut doc, &bridge, &self.layout_boxes);
        bridge.resolve_missing_document_layout(&mut doc);
    }
    /// Copy Scene paint boxes into the document cache (call after a frame draws).
    ///
    /// `layoutBox` / `getBoundingClientRect` already prefer the live store; this
    /// keeps hit-tests and `snapshot_boxes` aligned with paint.
    pub fn sync_scene_layout_boxes(&mut self) {
        let painted = {
            let doc = self.document.lock().expect("vue doc");
            self.layout_boxes
                .retain(|id| doc.contains_handle(NodeHandle(id)));
            self.layout_boxes.snapshot()
        };
        if painted.is_empty() {
            return;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        doc.apply_layout_boxes(&painted);
        reapply_scroll_translations(&mut doc, &bridge, &self.layout_boxes);
        bridge.resolve_missing_document_layout(&mut doc);
    }
    /// Per-window Scene layout writeback buffer (same as probes / `layoutBox`).
    pub fn layout_box_store(&self) -> Arc<LayoutBoxStore> {
        Arc::clone(&self.layout_boxes)
    }
}
