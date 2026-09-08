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
    /// Everything [`VueHost::resolve_layout`] reads. Equal keys mean it would
    /// write exactly what it wrote last time.
    fn layout_resolve_key(&self) -> LayoutResolveKey {
        let (bridge, logical) = {
            let bridge = self.bridge.lock().expect("vue bridge");
            let doc = self.document.lock().expect("vue doc");
            (bridge.revision(), doc.logical_size())
        };
        LayoutResolveKey {
            boxes: self.layout_boxes.revision(),
            bridge,
            logical: (logical.0.to_bits(), logical.1.to_bits()),
        }
    }

    /// Project painted geometry back into the document, unless nothing moved.
    ///
    /// `pump_frame` calls this once per host frame, and a pointer move is a host
    /// frame, so on a hover-heavy surface this runs at input rate. The body is
    /// five full-tree passes -- `reparent_orphans`, the sidebar/footer sync, the
    /// containing-block sync, the cascade sync and `flush_host_frame`, plus the
    /// store retain/snapshot/apply around them -- and every one of them is
    /// deterministic in the key below. A pointer move changes no box, no widget
    /// and no viewport, so re-running them rebuilds the identical document at
    /// O(nodes) per event: measured at 1.04 ms of a 1.44 ms pointer event on a
    /// 2,000-node tree, against 0.047 ms for the same tree under Rust L3.
    ///
    /// `LayoutBoxStore::revision` is the load-bearing half of the key: a Scene
    /// frame re-records every visible node whether or not it moved, so the store
    /// counts writes that changed something rather than writes.
    pub fn resolve_layout(&mut self) {
        let mut key = self.layout_resolve_key();
        if self.resolved_layout_key == Some(key) {
            return;
        }
        // One pass is not a fixed point. The cascade sync and `flush_host_frame`
        // both write, and they carry state across calls, so the first resolve
        // after a mount leaves boxes a second pass still moves -- before this
        // loop existed, callers reached the settled geometry only by virtue of
        // running every frame forever, and a gate on top of that froze the
        // document mid-convergence. So converge here, the way
        // `RuntimeDocument::flush` already converges its own systems, and only
        // then record the state that is allowed to skip.
        //
        // The cap matches `nana_ui_scene`'s `MAX_FRAME_PASSES`: a tree that has
        // not settled in this many passes is oscillating, and spinning would
        // turn that into a hang instead of a stale frame.
        const MAX_RESOLVE_PASSES: usize = 8;
        for _ in 0..MAX_RESOLVE_PASSES {
            if !self.resolve_layout_uncached() {
                break;
            }
        }
        key = self.layout_resolve_key();
        self.resolved_layout_key = Some(key);
    }

    /// One projection pass. Returns whether it changed any node's layout, which
    /// is what [`Self::resolve_layout`] loops on.
    fn resolve_layout_uncached(&mut self) -> bool {
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
            // Nothing painted yet, so there is no projection to converge on.
            return false;
        }
        let mut bridge = self.bridge.lock().expect("vue bridge");
        let mut doc = self.document.lock().expect("vue doc");
        let moved = doc.apply_layout_boxes(&painted);
        reapply_scroll_translations(&mut doc, &bridge, &self.layout_boxes);
        bridge.resolve_missing_document_layout(&mut doc);
        moved
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
    /// Force the next [`Self::resolve_layout`] to do its work.
    ///
    /// For callers that change something the key cannot see. Nothing needs this
    /// today; it exists so that adding such a caller is a one-line fix rather
    /// than a silent stale layout.
    pub fn invalidate_resolved_layout(&mut self) {
        self.resolved_layout_key = None;
    }

    /// Per-window Scene layout writeback buffer (same as probes / `layoutBox`).
    pub fn layout_box_store(&self) -> Arc<LayoutBoxStore> {
        Arc::clone(&self.layout_boxes)
    }
}

/// Identifies one `resolve_layout` input state. See [`VueHost::resolve_layout`].
///
/// Floats are compared as bits rather than by `PartialEq` so the key is `Eq`:
/// a NaN viewport would make every frame differ, which costs a repeat rather
/// than skipping a needed one -- the safe direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayoutResolveKey {
    boxes: u64,
    bridge: u64,
    logical: (u32, u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_js_engine::HostValue;

    /// Build a host with one element in the body and return `(host, node)`.
    fn host_with_one_node() -> (VueHost, NodeHandle) {
        let host = VueHost::new();
        let api = host.host_api_registry();
        let body = api.call("mountRoot", &[]).expect("mount root");
        let node = api
            .call("createElement", &[HostValue::string("div")])
            .expect("create");
        api.call("insert", &[node.clone(), body, HostValue::Null])
            .expect("insert");
        let HostValue::Number(id) = node else {
            panic!("createElement returns a node handle");
        };
        (host, NodeHandle(id as u64))
    }

    fn document_box(host: &VueHost, node: NodeHandle) -> Option<LayoutBox> {
        host.document().lock().expect("vue doc").layout_box(node)
    }

    /// The skip must not stick: once a node actually moves, the next resolve has
    /// to project it. A gate that latched here would freeze every `layoutBox`
    /// and `getBoundingClientRect` at whatever the first frame painted, and the
    /// symptom -- menus anchored to stale coordinates -- would show up far from
    /// the cause.
    /// The skip must not stick. Once paint reports an extent the document does
    /// not have, the next resolve has to project it -- a gate that latched here
    /// would freeze `layoutBox` and `getBoundingClientRect` at whatever the
    /// first settled frame held, and the symptom (menus anchored to stale
    /// coordinates, wheel metrics short by an overflow) would surface far from
    /// the cause.
    ///
    /// An expanded extent rather than a moved box on purpose:
    /// `write_layout_boxes` only fills boxes the engine has not produced and
    /// grows ones paint reports larger. Repositioning from paint is deliberately
    /// not a thing, so a moved box would assert a contract that does not exist.
    #[test]
    fn a_grown_paint_extent_still_reaches_the_document_after_an_earlier_resolve() {
        let (mut host, node) = host_with_one_node();
        let store = host.layout_box_store();

        store.record(node, 0.0, 0.0, 100.0, 20.0);
        host.resolve_layout();
        assert_eq!(
            document_box(&host, node).map(|box_| (box_.width, box_.height)),
            Some((100.0, 20.0))
        );

        store.record(node, 0.0, 0.0, 100.0, 200.0);
        host.resolve_layout();
        assert_eq!(
            document_box(&host, node).map(|box_| (box_.width, box_.height)),
            Some((100.0, 200.0)),
            "a resolve after paint grew must not be skipped"
        );
    }

    #[test]
    fn an_identical_repaint_leaves_the_document_alone() {
        let (mut host, node) = host_with_one_node();
        let store = host.layout_box_store();
        store.record(node, 8.0, 12.0, 64.0, 24.0);
        host.resolve_layout();

        for _ in 0..8 {
            store.record(node, 8.0, 12.0, 64.0, 24.0);
            host.resolve_layout();
        }
        // Named explicitly rather than captured from the first pass: a gate that
        // froze the document at a pre-projection zero box would also be
        // "stable", and asserting stability alone would call that a pass.
        assert_eq!(
            document_box(&host, node).map(|box_| (box_.x, box_.y, box_.width, box_.height)),
            Some((8.0, 12.0, 64.0, 24.0)),
            "repeats must settle on the painted geometry, not on whatever the \
             first pass happened to leave"
        );
    }

    /// A node appearing changes the tree without moving any existing box, so the
    /// bridge revision is the half of the key that has to catch it.
    #[test]
    fn a_new_node_invalidates_the_skip() {
        let (mut host, node) = host_with_one_node();
        let store = host.layout_box_store();
        store.record(node, 0.0, 0.0, 100.0, 20.0);
        host.resolve_layout();
        let before = host.layout_resolve_key();

        let api = host.host_api_registry();
        let added = api
            .call("createElement", &[HostValue::string("span")])
            .expect("create");
        api.call("insert", &[added, HostValue::Number(1.0), HostValue::Null])
            .ok();

        assert_ne!(
            host.layout_resolve_key(),
            before,
            "a tree that grew must not reuse the previous resolve"
        );
    }
}
