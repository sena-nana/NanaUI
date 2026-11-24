//! Logical vs presentation query. Overlay storage is Motion IR; UiWorld style
//! remains the logical authority.

use super::*;
use crate::PresentationPlayback;
use nana_ui_core::{
    DisplaySpec, LengthSpec, PaintTransform, ParentPerspective,
    motion::{
        AnimatableProperty, AnimationClass, MotionEvaluatorBackend, MotionInspectorEntry,
        MotionTargetId, MotionValue, MotionWorkCounters, PresentationPair, cpu_fallback_reason,
    },
};

/// Resolves direct-child `perspective` + `perspective-origin` in this parent's
/// layout coordinates (with `scroll` added so child coordinates match).
pub(super) fn child_parent_perspective(
    style: &nana_ui_core::LayoutStyle,
    layout: LayoutBox,
    scroll: ScrollOffset,
    blocks_3d: bool,
) -> Option<ParentPerspective> {
    if blocks_3d || style.transforms.preserve_3d {
        return None;
    }
    let distance = style.transforms.css_perspective?;
    if !distance.is_finite() || distance <= 1e-5 {
        return None;
    }
    let [ox, oy] = style.resolved_perspective_origin(layout.width, layout.height);
    Some(ParentPerspective {
        distance,
        origin: [layout.x + ox + scroll.x, layout.y + oy + scroll.y],
    })
}

impl UiWorld {
    pub fn presentation_store(&self) -> &nana_ui_core::motion::PresentationStore {
        &self.motion.presentation
    }

    pub fn animation_now(&self) -> Duration {
        self.motion.animation_now
    }

    /// Host compositor present clock. Does not sample CPU/Layout tracks or
    /// mark dirty; GPU evaluate and layer hysteresis read this timestamp.
    ///
    /// `now` is host time. It moves the document's time only while the
    /// document plays (see [`Self::set_presentation_playback`]): a paused
    /// document presents the time it holds, so a compositor-only frame cannot
    /// pull compositor samples away from the layout samples a seek wrote.
    pub fn sync_presentation_clock(&mut self, now: Duration) {
        self.observe_host_time(now);
    }

    /// Whether the document's time plays or holds.
    pub fn presentation_playback(&self) -> PresentationPlayback {
        self.motion.presentation_clock.playback()
    }

    /// Play at a rate or hold the document's time, from the time it presents
    /// now. Only this document is affected; another keeps its own time.
    /// Playing again does not count the time spent paused: the next host time
    /// observed pairs with the held time. A rate that is not finite and
    /// positive pauses.
    pub fn set_presentation_playback(&mut self, playback: PresentationPlayback) {
        self.motion
            .presentation_clock
            .set_playback(playback, self.motion.animation_now);
    }

    /// Whether host time currently moves the document's time. A paused
    /// document asks for no compositor frames of its own.
    pub fn presentation_advancing(&self) -> bool {
        self.motion.presentation_clock.playback().is_playing()
    }

    /// The interval the current frame stands for: the display interval while
    /// the document plays, or the interval a caller last stepped by. Shutter
    /// and sampling tolerances read this.
    pub fn presentation_frame_interval(&self) -> Duration {
        self.motion.presentation_clock.frame_interval()
    }

    /// The host reports its display interval; it is the frame interval while
    /// the document plays.
    pub fn set_presentation_display_interval(&mut self, interval: Duration) {
        self.motion
            .presentation_clock
            .set_display_interval(interval);
    }

    /// The latest host time this document observed. Input, debounce and
    /// other wall-clock work read this rather than the presentation time.
    pub fn presentation_host_now(&self) -> Duration {
        self.motion.presentation_clock.host_now()
    }

    /// The host time the next animation wake falls due at:
    /// [`Self::next_animation_deadline`] mapped back through the document's
    /// clock. A paused document wakes only to deliver what is already due.
    pub fn next_presentation_wakeup(&self) -> Option<Duration> {
        let deadline = self
            .next_animation_deadline()
            .into_iter()
            .chain(self.next_media_motion())
            .min()?;
        self.motion
            .presentation_clock
            .host_deadline(deadline, self.motion.animation_now)
    }

    /// The presentation-time mapping of a time-sampled host-texture node
    /// (Issue #303).
    pub fn media_time_mapping(&self, id: StableNodeId) -> Option<&crate::MediaTimeMapping> {
        if self.invalidation.media_time_mappings.is_empty() {
            return None;
        }
        self.invalidation.media_time_mappings.get(&id)
    }

    /// The resource time, in seconds, a time-sampled node shows at the
    /// document's current presentation time.
    pub fn media_time(&self, id: StableNodeId) -> Option<f64> {
        self.media_time_mapping(id)
            .map(|mapping| mapping.media_time(self.motion.animation_now))
    }

    /// Every visible time-sampled host-texture node with the slot it samples
    /// and the resource time this frame shows, in node order. Read it before
    /// the frame is flushed (`prepare_window_frame`). Empty, and free, in a
    /// tree without such nodes. `document` limits it to one document.
    pub fn media_sample_requests(
        &self,
        document: Option<DocumentId>,
    ) -> Vec<crate::MediaSampleRequest> {
        if self.invalidation.media_time_mappings.is_empty() {
            return Vec::new();
        }
        let frame_interval = self.presentation_frame_interval();
        let pacing = if self.presentation_advancing() {
            crate::MediaPacing::Realtime
        } else {
            crate::MediaPacing::Stepped
        };
        let mut requests = self
            .invalidation
            .media_time_mappings
            .iter()
            .filter(|(id, _)| {
                document.is_none_or(|document| self.document_of(**id) == Some(document))
            })
            .filter_map(|(&node, mapping)| {
                let slot = self.media_sampled_slot(node)?;
                Some(crate::MediaSampleRequest {
                    node,
                    slot: crate::TextureSlotName::new(slot),
                    media_time: mapping.media_time(self.motion.animation_now),
                    frame_interval,
                    pacing,
                })
            })
            .collect::<Vec<_>>();
        requests.sort_unstable_by_key(|request| request.node);
        requests
    }

    /// The earliest document time a visible time-sampled node's resource time
    /// moves at; `None` while every one holds. While the document is paused
    /// nothing moves, so nothing wakes the host.
    pub(crate) fn next_media_motion(&self) -> Option<Duration> {
        if self.invalidation.media_time_mappings.is_empty() || !self.presentation_advancing() {
            return None;
        }
        self.invalidation
            .media_time_mappings
            .iter()
            .filter_map(|(&node, mapping)| {
                let next = mapping.next_motion(self.motion.animation_now)?;
                self.media_sampled_slot(node)?;
                Some(next)
            })
            .min()
    }

    /// The host-texture slot `id` samples while it is part of a live,
    /// visible document; `None` for a detached, parked, hidden or slotless
    /// node.
    fn media_sampled_slot(&self, id: StableNodeId) -> Option<Arc<str>> {
        let custom = self.custom_render(id)?;
        if custom.renderer.as_ref() != crate::HOST_TEXTURE_RENDERER || !self.is_mounted(id) {
            return None;
        }
        let mut root = id;
        while let Some(parent) = self.parent_id(root) {
            root = parent;
        }
        if !self.is_document_node(root) || !self.is_overlay_reachable(id) {
            return None;
        }
        Some(Arc::clone(&custom.resource))
    }

    /// Bring the document to host time `now`.
    pub(crate) fn observe_host_time(&mut self, now: Duration) {
        self.motion.animation_now = self.motion.presentation_clock.observe(now);
    }

    /// Bring the document to host time `now` unless it already saw a later
    /// one (an input stamped before the last frame).
    pub(crate) fn note_host_time(&mut self, now: Duration) {
        if now > self.motion.presentation_clock.host_now() {
            self.observe_host_time(now);
        }
    }

    pub fn motion_descriptor_store(&self) -> &nana_ui_core::motion::MotionDescriptorStore {
        &self.motion.motion_descriptors
    }

    pub fn motion_handle(
        &self,
        id: crate::AnimationId,
    ) -> Option<nana_ui_core::motion::MotionHandle> {
        self.motion.motion_descriptors.handle_for(id.track_id())
    }

    /// Logical / base value from retained style. Not the in-flight overlay.
    pub fn logical_motion_value(
        &self,
        id: StableNodeId,
        property: AnimatableProperty,
    ) -> Option<MotionValue> {
        let style = self.node_style(id)?;
        let layout = style.layout.as_ref();
        match property {
            AnimatableProperty::Opacity => Some(MotionValue::Scalar(layout.opacity.unwrap_or(1.0))),
            AnimatableProperty::Transform => Some(MotionValue::Transform(
                layout.transforms.transform.unwrap_or_default(),
            )),
            AnimatableProperty::Background => {
                let model = self.node_style_model(id);
                let resolved_layout = &self.record(id).resolved_layout;
                resolved_layout
                    .paint_colors
                    .background
                    .filter(|c| c.is_theme_dynamic())
                    .map(|c| c.resolve_srgb(model))
                    .or(resolved_layout.background)
                    .or(layout.background)
                    .or_else(|| self.computed_style(id).and_then(|s| s.background))
                    .map(MotionValue::Color)
            }
            AnimatableProperty::Width => length_px(layout.width),
            AnimatableProperty::Height => length_px(layout.height),
            AnimatableProperty::Padding => length_px(layout.padding),
            AnimatableProperty::Margin => length_px(layout.margin),
            AnimatableProperty::Display => Some(MotionValue::Discrete(display_tag(
                layout.display.unwrap_or(DisplaySpec::Flex),
            ))),
            AnimatableProperty::FontAxis(tag) => nana_ui_core::FontVariationSetting::axis_value(
                &self.logical_font_variations(id),
                tag,
            )
            .map(MotionValue::Scalar),
            AnimatableProperty::Color => {
                let model = self.node_style_model(id);
                let resolved_layout = &self.record(id).resolved_layout;
                resolved_layout
                    .paint_colors
                    .color
                    .filter(|c| c.is_theme_dynamic())
                    .map(|c| c.resolve_srgb(model))
                    .or(resolved_layout.color)
                    .or(layout.color)
                    .or_else(|| self.computed_style(id).and_then(|s| s.color))
                    .map(MotionValue::Color)
            }
            AnimatableProperty::Filter => {
                // No filter is the identity filter, so `none` and a filter
                // interpolate. A theme-tracked shadow colour starts from what
                // the theme resolves it to now.
                let mut filter = layout.paint.filter.unwrap_or_default();
                if let Some(shadow) = filter.drop_shadow.as_mut() {
                    shadow.color = shadow.color.resolve(self.node_style_model(id));
                }
                Some(MotionValue::Filter(filter.into()))
            }
            AnimatableProperty::Clip
            | AnimatableProperty::Blur
            | AnimatableProperty::Shadow
            | AnimatableProperty::ShaderParameter
            | AnimatableProperty::FontSize
            | AnimatableProperty::Progress => None,
        }
    }

    /// On-demand presentation at `now`. Hit-test / pointer mapping / focus /
    /// a11y sample compositor properties here; they do not write UiWorld.
    pub fn presentation_motion_value(
        &self,
        id: StableNodeId,
        property: AnimatableProperty,
        now: Duration,
    ) -> Option<MotionValue> {
        Some(self.presentation_pair(id, property, now)?.presentation)
    }

    /// Query-time compositor samples since this world was created. Idle
    /// [`Self::advance_animations`] frames do not increment this.
    pub fn presentation_values_cpu_sampled(&self) -> usize {
        self.motion.presentation_query_samples.get()
    }

    /// Track classification plus last-frame animation attribution.
    /// GPU upload counters stay zero until a renderer observes them.
    /// `presentation_values_cpu_sampled` is the **cumulative** query count so
    /// existing hit-test tests can diff it; last-frame delta is
    /// [`Self::last_motion_frame_counters`].
    pub fn motion_work_counters(&self) -> MotionWorkCounters {
        let mut counters = MotionWorkCounters::from_tracks(
            self.motion
                .presentation
                .overlays()
                .map(|overlay| &overlay.track),
        );
        counters.presentation_values_cpu_sampled = self.motion.presentation_query_samples.get();
        counters.uiworld_mutations_from_animation = self
            .motion
            .last_motion_frame
            .uiworld_mutations_from_animation;
        counters.layout_nodes_from_animation =
            self.motion.last_motion_frame.layout_nodes_from_animation;
        counters.style_processed_from_animation =
            self.motion.last_motion_frame.style_processed_from_animation;
        counters.render_nodes_reextracted_from_animation = self
            .motion
            .last_motion_frame
            .render_nodes_reextracted_from_animation;
        counters
    }

    /// Last `advance_animations` work. CPU samples are the delta since that
    /// call started (no hit-test/query → 0).
    pub fn last_motion_frame_counters(&self) -> MotionWorkCounters {
        let mut counters = self.motion_work_counters();
        counters.presentation_values_cpu_sampled = self.last_motion_frame_cpu_samples();
        counters
    }

    pub fn last_motion_frame_cpu_samples(&self) -> usize {
        self.motion
            .presentation_query_samples
            .get()
            .saturating_sub(self.motion.presentation_samples_at_advance.get())
    }

    /// Issue #87 §14 inspector rows. Scene layer fields stay `None` until
    /// [`nana_ui_scene::UiScene::annotate_motion_inspector`] fills them.
    pub fn inspect_motion(&self) -> Vec<MotionInspectorEntry> {
        let samples = self.last_motion_frame_cpu_samples();
        let layout = self.motion.last_motion_frame.layout_nodes_from_animation;
        let style = self.motion.last_motion_frame.style_processed_from_animation;
        let extract = self
            .motion
            .last_motion_frame
            .render_nodes_reextracted_from_animation;
        let now = self.motion.animation_now;
        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for overlay in self.motion.presentation.overlays() {
            let track = &overlay.track;
            seen.insert(track.id);
            let node = track.target.get();
            let handle = self.motion.motion_descriptors.handle_for(track.id);
            let has_gpu = handle.is_some_and(|handle| !handle.is_null());
            let evaluator =
                if track.property.animation_class() == AnimationClass::Compositor && has_gpu {
                    MotionEvaluatorBackend::Gpu
                } else {
                    MotionEvaluatorBackend::Cpu
                };
            let id = crate::AnimationId::new(track.id.get());
            let next_deadline = id.and_then(|id| {
                self.motion
                    .animations
                    .get(&id)
                    .map(|animation| animation.next_deadline)
            });
            let pair = StableNodeId::new(node)
                .and_then(|id| self.presentation_pair(id, track.property, now));
            entries.push(MotionInspectorEntry {
                node,
                property: track.property,
                class: track.property.animation_class(),
                evaluator,
                layer: None,
                layer_promotion_reason: None,
                cpu_fallback_reason: cpu_fallback_reason(track.property, has_gpu),
                ineffective_reason: StableNodeId::new(node)
                    .and_then(|id| self.ineffective_motion_reason(id, track.property)),
                base: pair.as_ref().map(|pair| pair.logical),
                presentation: pair.map(|pair| pair.presentation),
                next_deadline,
                gpu_handle: handle,
                runtime_samples_this_frame: if track.property.animation_class()
                    == AnimationClass::Compositor
                {
                    samples
                } else {
                    1
                },
                layout_nodes_from_animation: layout,
                style_processed_from_animation: style,
                render_nodes_reextracted_from_animation: extract,
            });
        }
        for (id, animation) in &self.motion.animations {
            if seen.contains(&id.track_id()) {
                continue;
            }
            let spec = &animation.spec;
            let has_gpu = false;
            entries.push(MotionInspectorEntry {
                node: spec.target.get(),
                property: spec.property,
                class: spec.property.animation_class(),
                evaluator: MotionEvaluatorBackend::Cpu,
                layer: None,
                layer_promotion_reason: None,
                cpu_fallback_reason: cpu_fallback_reason(spec.property, has_gpu),
                ineffective_reason: self.ineffective_motion_reason(spec.target, spec.property),
                base: self.logical_motion_value(spec.target, spec.property),
                presentation: None,
                next_deadline: Some(animation.next_deadline),
                gpu_handle: None,
                runtime_samples_this_frame: 1,
                layout_nodes_from_animation: layout,
                style_processed_from_animation: style,
                render_nodes_reextracted_from_animation: extract,
            });
        }
        entries.compact_sort_by_key(|entry| (entry.node, entry.property));
        entries
    }

    pub(super) fn begin_motion_frame(&mut self) {
        self.motion
            .presentation_samples_at_advance
            .set(self.motion.presentation_query_samples.get());
        self.motion.last_motion_frame = MotionWorkCounters::default();
    }

    pub(super) fn account_animation_dirty(&mut self, bits: u16) {
        self.motion
            .last_motion_frame
            .uiworld_mutations_from_animation = self
            .motion
            .last_motion_frame
            .uiworld_mutations_from_animation
            .saturating_add(1);
        if bits & crate::schedule::DirtyMask::LAYOUT != 0 {
            self.motion.last_motion_frame.layout_nodes_from_animation = self
                .motion
                .last_motion_frame
                .layout_nodes_from_animation
                .saturating_add(1);
        }
        if bits & crate::schedule::DirtyMask::STYLE != 0 {
            self.motion.last_motion_frame.style_processed_from_animation = self
                .motion
                .last_motion_frame
                .style_processed_from_animation
                .saturating_add(1);
        }
        if bits & crate::schedule::DirtyMask::RENDER != 0 {
            self.motion
                .last_motion_frame
                .render_nodes_reextracted_from_animation = self
                .motion
                .last_motion_frame
                .render_nodes_reextracted_from_animation
                .saturating_add(1);
        }
    }

    pub(super) fn has_compositor_transform_overlay_for(&self, document: DocumentId) -> bool {
        let revision = (
            self.motion.presentation.revision(),
            self.invalidation.generation,
        );
        if let Some((presentation_revision, world_revision, documents)) = self
            .motion
            .presentation_transform_documents_cache
            .borrow()
            .as_ref()
            && (*presentation_revision, *world_revision) == revision
        {
            return documents.contains(&document);
        }
        let documents: std::collections::HashSet<DocumentId> = self
            .motion
            .presentation
            .overlays()
            .filter(|overlay| overlay.track.property == AnimatableProperty::Transform)
            .filter_map(|overlay| {
                StableNodeId::new(overlay.track.target.get()).and_then(|id| self.document_of(id))
            })
            .collect();
        let contains = documents.contains(&document);
        *self
            .motion
            .presentation_transform_documents_cache
            .borrow_mut() = Some((revision.0, revision.1, documents));
        contains
    }

    pub(super) fn sampled_compositor_transform(&self, id: StableNodeId) -> Option<PaintTransform> {
        match self.sample_compositor_presentation(id, AnimatableProperty::Transform)? {
            MotionValue::Transform(transform) => Some(transform),
            _ => None,
        }
    }

    pub(super) fn sampled_compositor_opacity(&self, id: StableNodeId) -> Option<f32> {
        match self.sample_compositor_presentation(id, AnimatableProperty::Opacity)? {
            MotionValue::Scalar(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn presentation_color_overlay(
        &self,
        id: StableNodeId,
        property: AnimatableProperty,
    ) -> Option<[f32; 4]> {
        if self.motion.presentation.is_empty() {
            return None;
        }
        let target = MotionTargetId::new(id.get())?;
        match self
            .motion
            .presentation
            .applied_value(target, property, self.motion.animation_now)?
        {
            MotionValue::Color(rgba) => Some(rgba),
            _ => None,
        }
    }

    fn sample_compositor_presentation(
        &self,
        id: StableNodeId,
        property: AnimatableProperty,
    ) -> Option<MotionValue> {
        if property.animation_class() != AnimationClass::Compositor {
            return None;
        }
        let target = MotionTargetId::new(id.get())?;
        if !self.motion.presentation.has_property(target, property) {
            return None;
        }
        self.motion.presentation_query_samples.set(
            self.motion
                .presentation_query_samples
                .get()
                .saturating_add(1),
        );
        self.motion
            .presentation
            .applied_value(target, property, self.motion.animation_now)
    }

    /// Local scene transform for hit-test / a11y / focus. Compositor
    /// `transform` overlays replace the logical matrix; Layout-class overlays
    /// are ignored so width/height animation cannot fake a scale.
    pub(super) fn input_local_scene_transform(
        &self,
        id: StableNodeId,
        style: &nana_ui_core::LayoutStyle,
        bounds: LayoutBox,
        blocks_3d: bool,
        parent_perspective: Option<ParentPerspective>,
    ) -> ([f32; 6], [f32; 2]) {
        // The refusal comes first: an overlay is not a way around a closed 3D
        // context, or the pointer keeps a projection paint gave up and a click
        // lands where the node is not drawn.
        if blocks_3d && style.transforms.transform_3d.is_some() {
            return (IDENTITY_AFFINE, [0.0, 0.0]);
        }
        // A triggered menu's overlay moves its hanging surface, not the
        // trigger: see `hanging_branch_transform`. Keep suppressing the
        // node's own compositor transform once closed while FillMode::Forwards
        // still retains the closing pop sample on `id`.
        if let Some(transform) = self.sampled_compositor_transform(id)
            && !self.has_triggered_menu_chrome(id)
        {
            let [ox, oy] = self
                .modal_pivot(id, bounds)
                .unwrap_or_else(|| style.resolved_transform_origin(bounds.width, bounds.height));
            return (
                transform.around_origin(bounds.x, bounds.y, ox, oy),
                [0.0, 0.0],
            );
        }
        style
            .world_scene_transform_with_perspective(
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                parent_perspective,
            )
            .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
    }

    /// A modal frame's presentation pivot: its card's centre. Only a modal
    /// frame is asked for its geometry here, and only while it moves.
    fn modal_pivot(&self, id: StableNodeId, bounds: LayoutBox) -> Option<[f32; 2]> {
        crate::WidgetVisual::presentation_pivot(
            self.widget_visual_as::<crate::ModalFrameVisual>(id)?,
            id,
            self,
            bounds,
        )
    }

    fn is_triggered_menu(&self, id: StableNodeId) -> bool {
        self.widget_visual_ref(id)
            .is_some_and(|visual| visual.triggered_menu_overlay().is_some())
    }

    fn has_triggered_menu_chrome(&self, id: StableNodeId) -> bool {
        self.widget_visual_as::<crate::MenuSurfaceVisual>(id)
            .is_some_and(|visual| visual.overlay.is_some() && visual.has_trigger())
    }

    /// Where the viewport-fixed branch rooted at `fixed` starts: the identity,
    /// unless `fixed` is an item a triggered menu places. Those, and the
    /// surface hanging behind them, take the menu's presentation transform
    /// about the surface's pivot; the trigger keeps its logical one.
    pub(super) fn hanging_branch_transform(&self, fixed: StableNodeId) -> ([f32; 6], [f32; 2]) {
        self.nodes
            .get(fixed)
            .and_then(|node| node.hierarchy.parent)
            .and_then(|menu| {
                let overlay = self.widget_visual_ref(menu)?.triggered_menu_overlay()?;
                (overlay.trigger_content != Some(fixed)).then_some(menu)
            })
            .and_then(|menu| self.hanging_surface_transform(menu))
            .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
    }

    /// A triggered menu's presentation transform where its surface hangs:
    /// `None` when it has no transform overlay or is not a triggered menu.
    pub(super) fn hanging_surface_transform(
        &self,
        menu: StableNodeId,
    ) -> Option<([f32; 6], [f32; 2])> {
        if !self.is_triggered_menu(menu) {
            return None;
        }
        let transform = self.sampled_compositor_transform(menu)?;
        let bounds = self.component_layout_box(menu)?;
        let [ox, oy] = self
            .widget_visual_ref(menu)?
            .presentation_pivot(menu, self, bounds)?;
        Some((
            transform.around_origin(bounds.x, bounds.y, ox, oy),
            [0.0, 0.0],
        ))
    }

    /// Viewport AABB of the logical layout box after compositor presentation
    /// transform. Layout-class overlays do not change the box.
    pub fn presentation_input_bounds(&self, id: StableNodeId) -> Option<LayoutBox> {
        self.project_input_bounds(id, self.component_layout_box(id)?)
    }

    /// Where the selected text of a `NativeMarkdown` or `SelectableRichText`
    /// shows in the viewport: the smallest box around every selected glyph,
    /// through the scroll offsets and transforms above the node. `None`
    /// without a selection, or for any other node.
    ///
    /// For anchoring a floating toolbar to a selection. Ask after the update
    /// that changed the selection returns, and again after a scroll moves it.
    pub fn text_selection_bounds(&self, id: StableNodeId) -> Option<LayoutBox> {
        #[cfg(not(feature = "rich-text"))]
        {
            let _ = id;
            None
        }
        #[cfg(feature = "rich-text")]
        {
            let selection = if let Some(layout) = self.native_markdown_layout(id) {
                layout.selection
            } else {
                let layout = self.selectable_rich_text_paint_layout(id)?;
                layout.selection
            };
            let enclosing = selection.into_iter().reduce(|a, b| {
                let left = a.x.min(b.x);
                let top = a.y.min(b.y);
                LayoutBox {
                    x: left,
                    y: top,
                    width: (a.x + a.width).max(b.x + b.width) - left,
                    height: (a.y + a.height).max(b.y + b.height) - top,
                }
            })?;
            self.project_input_bounds(id, enclosing)
        }
    }

    /// Focus geometry follows compositor presentation, not the logical rest
    /// transform.
    pub fn focused_geometry(&self, document: DocumentId) -> Option<LayoutBox> {
        self.presentation_input_bounds(self.focused(document)?)
    }

    /// Where `bounds`, in `id`'s layout coordinates, sits in the viewport:
    /// through the scroll offsets and transforms above `id`, up to its nearest
    /// viewport-fixed ancestor. Layout anchors fixed surfaces on this.
    pub(crate) fn project_input_bounds(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
    ) -> Option<LayoutBox> {
        let ([a, by, c, d, e, f], [g, h]) = self.layout_projection_transform(id)?;
        project_transformed_box(bounds, [a, by, c, d, e, f], [g, h])
    }

    /// `id`'s layout box where it sits in the window: through the scroll
    /// offsets and transforms above it, but not its own transform.
    pub(crate) fn project_placement_bounds(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
    ) -> Option<LayoutBox> {
        let ([a, by, c, d, e, f], [g, h]) = self.layout_projection_transform_from(id, false)?;
        project_transformed_box(bounds, [a, by, c, d, e, f], [g, h])
    }

    pub fn presentation_pair(
        &self,
        id: StableNodeId,
        property: AnimatableProperty,
        now: Duration,
    ) -> Option<PresentationPair> {
        let target = MotionTargetId::new(id.get())?;
        let logical = self.logical_motion_value(id, property).or_else(|| {
            (property == AnimatableProperty::Background
                && self.motion.presentation.has_property(target, property))
            .then_some(MotionValue::Color([0.0, 0.0, 0.0, 0.0]))
        })?;
        Some(
            self.motion
                .presentation
                .pair(target, property, logical, now),
        )
    }

    pub fn presentation_applied_value(
        &self,
        id: StableNodeId,
        property: AnimatableProperty,
        now: Duration,
    ) -> Option<MotionValue> {
        let target = MotionTargetId::new(id.get())?;
        self.motion
            .presentation
            .applied_value(target, property, now)
    }

    /// Enter/exit invariant: unfinished overlays survive park/logical unmount.
    pub fn presentation_retains_unmounted(&self, id: StableNodeId, now: Duration) -> bool {
        MotionTargetId::new(id.get())
            .is_some_and(|target| self.motion.presentation.retains_unmounted(target, now))
    }

    pub fn motion_descriptors(&self) -> &nana_ui_core::motion::MotionDescriptorStore {
        &self.motion.motion_descriptors
    }

    /// Advanced compositor layer request. Extract copies this onto
    /// [`crate::ExtractedCompositor::request_layer`].
    pub fn request_compositor_layer(&mut self, id: StableNodeId) {
        self.motion.compositor_layer_requests.insert(id);
        self.mark(id, crate::schedule::DirtyMask::RENDER);
    }

    pub fn clear_compositor_layer_request(&mut self, id: StableNodeId) {
        self.motion.compositor_layer_requests.remove(&id);
        self.mark(id, crate::schedule::DirtyMask::RENDER);
    }

    pub(super) fn extracted_compositor(&self, id: StableNodeId) -> crate::ExtractedCompositor {
        if self.motion.presentation.is_empty() && self.motion.compositor_layer_requests.is_empty() {
            return crate::ExtractedCompositor {
                bindings: Vec::new(),
                request_layer: false,
            };
        }
        let mut bindings = Vec::new();
        if let Some(target) = MotionTargetId::new(id.get()) {
            for property in [
                AnimatableProperty::Transform,
                AnimatableProperty::Opacity,
                AnimatableProperty::Clip,
                AnimatableProperty::ShaderParameter,
            ] {
                if let Some(track) = self.motion.presentation.winning_track_id(
                    target,
                    property,
                    self.motion.animation_now,
                ) && !bindings.contains(&track)
                {
                    bindings.push(track);
                }
            }
        }
        crate::ExtractedCompositor {
            bindings,
            request_layer: self.motion.compositor_layer_requests.contains(&id),
        }
    }
}

fn length_px(spec: Option<LengthSpec>) -> Option<MotionValue> {
    match spec {
        Some(LengthSpec::Px(px)) if px.is_finite() => Some(MotionValue::Scalar(px)),
        _ => None,
    }
}

fn display_tag(display: DisplaySpec) -> u32 {
    display as u32
}
