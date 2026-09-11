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
    pub fn flush_scene_frame(&mut self) -> Result<(), nana_ui_runtime::FrameworkError> {
        // Read from the document rather than take it from the caller: the
        // document derives its logical size from the physical viewport and the
        // scale factor, so a caller passing its own numbers was a second source
        // that agreed only at scale 1.
        let (logical_width, logical_height) = self.document.lock().expect("vue doc").logical_size();
        {
            let mut doc = self.document.lock().expect("vue doc");
            doc.flush_host_frame();
            self.report_commit_rejections(&mut doc);
        }
        self.report_unsupported_css();
        self.flush_runtime_scene(logical_width, logical_height)?;

        // Re-recording every painted box is a full-tree walk plus one `record`
        // per node, and `record` takes three locks -- on a 2,000 node tree that
        // is 6,000 lock round trips per frame to write back the values already
        // there. `UiScene::instance_id` changes on any node update or removal
        // and on nothing else, so an unchanged scene has nothing to re-record.
        //
        // Skipping `begin_frame` with it is not a compromise, it is the
        // correction: `reapply_scroll_translations` rebuilds the view overlays
        // from scratch every time it runs (it clears them itself first), so this
        // clear was redundant -- and worse, the `record` loop then dropped each
        // overlay again through `clear_view`, leaving every geometry read
        // unscrolled until the next `resolve_layout` rebuilt them.
        let instance = {
            let document = self.document.lock().expect("vue doc");
            document.runtime_document().scene().instance_id()
        };
        if self.frame_gates_enabled && self.recorded_scene_instance == Some(instance) {
            return Ok(());
        }
        self.recorded_scene_instance = Some(instance);

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
    /// Everything a window's redraw does to this host before the Scene is painted.
    ///
    /// Lives here rather than in the hosted adapter because every step is a host
    /// method: the adapter was open-coding the host's own frame. Keeping it in
    /// one place is also what lets a benchmark measure the real production
    /// sequence instead of a replica that drifts from it.
    ///
    /// The two `sync_svg_rasters` / `flush_host_frame` rounds bracket
    /// `resolve_layout` on purpose: the first makes rasters and committed host
    /// ops available to layout, the second picks up whatever resolving produced.
    pub fn prepare_window_frame(&mut self) {
        // The GPU halves only exist with `hosted`; without it there is no device
        // to prepare, and the rest of the frame is unchanged.
        #[cfg(feature = "hosted")]
        self.prepare_canvas_gpu();
        if let Ok(mut document) = self.document.lock() {
            document.sync_svg_rasters();
        }
        #[cfg(feature = "hosted")]
        {
            self.prepare_svg_gpu();
            self.prepare_media_gpu();
        }
        // Stamp packed HostTexture generation/version onto CustomRenderNode
        // before extract; content invalidation must not leave revision at 0.
        if let Ok(mut document) = self.document.lock() {
            document.flush_host_frame();
            #[cfg(feature = "scene-view")]
            self.report_commit_rejections(&mut document);
        }
        // Borrow semantic data only when the bridge moved past the synced revision.
        let synced = self
            .document
            .lock()
            .ok()
            .and_then(|document| document.synced_semantic_revision());
        let needs_snapshot = match self.bridge.lock() {
            Ok(bridge) => synced != Some(bridge.revision()),
            Err(_) => true,
        };
        if needs_snapshot {
            #[cfg(not(feature = "benchmark"))]
            self.sync_semantics();
            #[cfg(feature = "benchmark")]
            crate::frame_profile::timed(2, || self.sync_semantics());
        }
        #[cfg(not(feature = "benchmark"))]
        self.resolve_layout();
        #[cfg(feature = "benchmark")]
        {
            let started = std::time::Instant::now();
            self.resolve_layout();
            crate::frame_profile::record(7, started.elapsed());
        }
        if let Ok(mut document) = self.document.lock() {
            document.sync_svg_rasters();
        }
        #[cfg(feature = "hosted")]
        {
            self.prepare_svg_gpu();
            self.prepare_media_gpu();
        }
        if let Ok(mut document) = self.document.lock() {
            document.flush_host_frame();
            #[cfg(feature = "scene-view")]
            self.report_commit_rejections(&mut document);
        }
    }

    pub fn resolve_layout(&mut self) {
        let key = self.layout_resolve_key();
        if self.frame_gates_enabled && self.resolved_layout_key == Some(key) {
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
            #[cfg(feature = "benchmark")]
            crate::frame_profile::count(8);
            if !self.resolve_layout_uncached() {
                break;
            }
        }
        self.resolved_layout_key = Some(self.layout_resolve_key());
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
    /// Turn every frame gate off, so each call redoes its work.
    ///
    /// Tests only, and specifically the equivalence harness: the gates claim a
    /// repeat would write what is already there, and the only way to check that
    /// claim is to run both sides and compare. Not `cfg(test)` because the
    /// harness has to be able to drive this from outside the crate.
    pub fn disable_frame_gates(&mut self) {
        self.frame_gates_enabled = false;
        self.resolved_layout_key = None;
        #[cfg(feature = "scene-view")]
        {
            self.recorded_scene_instance = None;
        }
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

#[cfg(all(test, feature = "scene-view"))]
mod equivalence {
    //! Same script down both paths, compared after every step.
    //!
    //! Every gate in this file rests on one claim: that a repeat would write
    //! what the document already holds. That claim is not provable by reading
    //! the code -- the last round shipped a gate on top of a `resolve_layout`
    //! that was not idempotent, and a hand-written test caught it only because
    //! it happened to assert the right thing. So run both, step by step, and
    //! diverge loudly at the first step that differs rather than at the end.
    //!
    //! No V8 and no bundler output: the tree is built through the host op
    //! registry, so this runs in the default `cargo test --workspace`.

    use super::*;
    use nana_js_engine::HostValue;

    struct Pair {
        gated: VueHost,
        ungated: VueHost,
    }

    impl Pair {
        fn new(rows: usize) -> Self {
            let mut ungated = build(rows);
            ungated.disable_frame_gates();
            Self {
                gated: build(rows),
                ungated,
            }
        }

        /// Run `step` on both hosts, then require the documents to agree.
        fn step(&mut self, what: &str, step: impl Fn(&mut VueHost)) {
            step(&mut self.gated);
            step(&mut self.ungated);
            let gated = snapshot(&self.gated);
            let ungated = snapshot(&self.ungated);
            assert_eq!(
                gated.document.boxes.len(),
                ungated.document.boxes.len(),
                "after {what}: the gated document has a different number of boxes"
            );
            assert_eq!(
                gated.painted, ungated.painted,
                "after {what}: skipping the frame changed the geometry JS reads back"
            );
            assert!(
                gated.document == ungated.document,
                "after {what}: skipping the frame changed the document"
            );
        }
    }

    fn build(rows: usize) -> VueHost {
        let host = VueHost::new();
        let api = host.host_api_registry();
        let body = api.call("mountRoot", &[]).expect("mount root");
        let port = api
            .call("createElement", &[HostValue::string("nana-scroll-view")])
            .expect("create port");
        api.call("insert", &[port.clone(), body, HostValue::Null])
            .expect("insert port");
        for row in 0..rows {
            let node = api
                .call("createElement", &[HostValue::string("div")])
                .expect("create row");
            api.call("insert", &[node.clone(), port.clone(), HostValue::Null])
                .expect("insert row");
            api.call(
                "setElementText",
                &[node, HostValue::string(format!("Row {row}"))],
            )
            .expect("text");
        }
        host
    }

    /// Everything a reader can observe about geometry, from both stores.
    ///
    /// `BoxSnapshot` alone is not enough and getting that wrong made the first
    /// version of this harness pass with both gates deliberately broken: it
    /// reads the *document's* layout boxes, while the scene gate decides whether
    /// to refill the *paint-box store*. What JS actually reads back is the
    /// store's view-aware value, so compare that too.
    #[derive(Debug, PartialEq)]
    struct Observable {
        document: crate::BoxSnapshot,
        painted: Vec<(u64, Option<crate::LayoutBox>)>,
    }

    fn snapshot(host: &VueHost) -> Observable {
        let store = host.layout_box_store();
        let document_slot = host.document();
        let document = document_slot.lock().expect("vue doc");
        let runtime = document.runtime_document();
        let ids = runtime.context().world().document_order(runtime.document());
        let painted = ids
            .into_iter()
            .map(|id| {
                let handle = NodeHandle(id.get());
                (id.get(), store.get(handle))
            })
            .collect();
        Observable {
            document: document.snapshot_boxes(),
            painted,
        }
    }

    fn frame(host: &mut VueHost) {
        host.prepare_window_frame();
        host.flush_scene_frame().expect("scene frame");
    }

    #[test]
    fn gated_and_ungated_frames_leave_the_same_document() {
        let mut pair = Pair::new(24);

        pair.step("mount", frame);
        // A settled tree: this is the case the gates exist for, and the case
        // where a wrong gate would freeze the document mid-convergence.
        for round in 0..4 {
            pair.step(&format!("idle frame {round}"), frame);
        }

        pair.step("scroll", |host| {
            // Straight at the document rather than through the `setScrollOffset`
            // host op: that op also writes a process-global pending-scroll
            // queue, and another test in this binary asserts that queue is
            // empty. What this harness needs is the offset, not the queue.
            let document = host.document();
            let mut document = document.lock().expect("doc");
            let port = NodeHandle(document.mount_root().0 + 1);
            document.set_scroll_offset(port, nana_ui_runtime::ScrollOffset { x: 0.0, y: 96.0 });
            drop(document);
            frame(host);
        });
        for round in 0..3 {
            pair.step(&format!("idle frame after scroll {round}"), frame);
        }

        pair.step("append a row", |host| {
            let api = host.host_api_registry();
            let body = api.call("mountRoot", &[]).expect("mount root");
            let node = api
                .call("createElement", &[HostValue::string("div")])
                .expect("create");
            api.call("insert", &[node.clone(), body, HostValue::Null])
                .expect("insert");
            api.call("setElementText", &[node, HostValue::string("appended")])
                .expect("text");
            frame(host);
        });

        pair.step("resize the viewport", |host| {
            host.set_viewport(640, 400, 1.0);
            frame(host);
        });

        pair.step("restyle a row", |host| {
            // Resolve the id before calling: the host op takes the same document
            // lock, and holding it here deadlocks into a `try_lock` failure.
            let row = host.document().lock().expect("doc").mount_root().0 + 2;
            let mut style = std::collections::BTreeMap::new();
            style.insert("height".to_owned(), HostValue::string("48px"));
            host.host_api_registry()
                .call(
                    "patchProp",
                    &[
                        HostValue::Number(row as f64),
                        HostValue::string("style"),
                        HostValue::Object(style),
                    ],
                )
                .expect("style");
            frame(host);
        });
        for round in 0..3 {
            pair.step(&format!("idle frame after restyle {round}"), frame);
        }
    }
}
