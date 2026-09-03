//! Bridge motion state and operations.

use super::*;

#[derive(Debug, Default)]
pub(super) struct State {
    /// Resolved motion longhands for `getComputedStyle` / Vue `<Transition>`.
    pub(super) computed_motion: HashMap<WidgetId, CssComputedMotion>,
    /// Active CSS transition timelines keyed by widget id.
    pub(super) css_transitions: HashMap<WidgetId, ActiveCssTransition>,
    /// Base layout paint captured before a CSS transition starts.
    pub(super) css_transition_base: HashMap<WidgetId, CssPaintSnapshot>,
    /// Latest eased progress for in-flight CSS transitions (0..=1).
    pub(super) css_transition_progress: HashMap<WidgetId, f32>,
    /// Finished CSS timelines waiting for host → JS `__nanaMotionComplete`.
    pub(super) pending_motion_completes: Vec<CssMotionComplete>,
    /// Widgets whose Runtime timeline just started; host should cancel the JS fallback.
    pub(super) pending_motion_cancels: Vec<WidgetId>,
    /// Last `animation-name` started per widget. Same name still playing must
    /// not `start_css_animation` again (that would reset the clock).
    pub(super) css_keyframes_name: HashMap<WidgetId, String>,
    /// TransitionGroup FLIP paint overlay. Applied after cascade; never LayoutBox.
    pub(super) paint_transform_overlays: HashMap<WidgetId, nana_ui_core::PaintTransform>,
    /// JS cleared the overlay; consume on class recascade / layout resolve.
    pub(super) paint_transform_releases: HashSet<WidgetId>,
}

impl MessageBridge {
    /// Apply Runtime animation samples without advancing the shared clock.
    pub(crate) fn apply_css_animation_samples(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
        frame: nana_ui_runtime::AnimationFrame,
    ) -> bool {
        let changed_ids = self.apply_css_animation_samples_inner(frame);
        if !changed_ids.is_empty() {
            self.sync_widget_layouts_for(doc, &changed_ids);
        }
        !changed_ids.is_empty()
    }
}

impl MessageBridge {
    /// Advance CSS transition / keyframe samples for the current host frame.
    pub(crate) fn tick_css_animations(&mut self, doc: &mut crate::tree::NanaTreeDocument) -> bool {
        let now = doc.runtime_now();
        let frame = doc.advance_css_animations(now);
        self.apply_css_animation_samples(doc, frame)
    }
}

impl MessageBridge {
    #[cfg(test)]
    pub(super) fn css_transition_target(&self, id: WidgetId) -> Option<CssPaintSnapshot> {
        self.motion
            .css_transitions
            .get(&id)
            .map(|transition| transition.to.clone())
    }
}

impl MessageBridge {
    pub(super) fn cascaded_target_paint(&mut self, id: WidgetId) -> CssPaintSnapshot {
        let overlay = if self.motion.paint_transform_releases.contains(&id) {
            self.motion.paint_transform_overlays.remove(&id)
        } else {
            None
        };
        let saved_layout = self
            .widgets
            .get(&id)
            .map(|widget| widget.props.layout.clone());
        let transition = self.motion.css_transitions.remove(&id);
        let progress = self.motion.css_transition_progress.remove(&id);
        let base = self.motion.css_transition_base.remove(&id);
        self.reapply_layout_for(id);
        let paint = self
            .snapshot_widget(id)
            .unwrap_or_else(|| CssPaintSnapshot::from_layout(&LayoutStyle::default()));
        if let Some(layout) = saved_layout
            && let Some(widget) = self.widgets.get_mut(&id)
        {
            widget.props.layout = layout;
        }
        if let Some(transition) = transition {
            self.motion.css_transitions.insert(id, transition);
        }
        if let Some(progress) = progress {
            self.motion.css_transition_progress.insert(id, progress);
        }
        if let Some(base) = base {
            self.motion.css_transition_base.insert(id, base);
        }
        if let Some(transform) = overlay {
            self.motion.paint_transform_overlays.insert(id, transform);
        }
        paint
    }
}

impl MessageBridge {
    pub(super) fn pin_host_driven_transition_paint(
        &mut self,
        doc: &crate::tree::NanaTreeDocument,
        id: WidgetId,
        from: &CssPaintSnapshot,
    ) {
        if doc.host_animation_epoch().is_some()
            && let Some(widget) = self.widgets.get_mut(&id)
        {
            from.apply_to_layout(&mut widget.props.layout);
        }
    }
}

impl MessageBridge {
    pub(super) fn release_pending_flip_transforms(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        let ids: Vec<WidgetId> = self
            .motion
            .paint_transform_releases
            .iter()
            .copied()
            .collect();
        for id in ids {
            self.maybe_release_flip_paint_transform(id, doc);
        }
    }
}

impl MessageBridge {
    /// Consume a released FLIP overlay: start a CSS transform transition when
    /// motion exists, otherwise snap to the cascaded (no leftover translate).
    pub(crate) fn maybe_release_flip_paint_transform(
        &mut self,
        id: WidgetId,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        if !self.motion.paint_transform_releases.contains(&id) {
            return;
        }
        let Some(overlay) = self.motion.paint_transform_overlays.get(&id).copied() else {
            self.motion.paint_transform_releases.remove(&id);
            return;
        };
        let mut from = self
            .snapshot_widget(id)
            .unwrap_or_else(|| CssPaintSnapshot::from_layout(&LayoutStyle::default()));
        from.transform = Some(overlay);
        self.motion.paint_transform_overlays.remove(&id);
        self.motion.paint_transform_releases.remove(&id);
        self.reapply_layout_for(id);
        let to = self
            .snapshot_widget(id)
            .unwrap_or_else(|| CssPaintSnapshot::from_layout(&LayoutStyle::default()));
        let now = doc.runtime_now();
        if from.transform != to.transform
            && let Some(motion) = self.motion.computed_motion.get(&id).cloned()
            && let Some(spec) = build_transition_spec(id, &motion, now)
        {
            self.motion.css_transition_base.insert(id, from.clone());
            self.motion.css_transition_progress.insert(id, 0.0);
            self.motion.css_transitions.insert(
                id,
                ActiveCssTransition {
                    from: from.clone(),
                    to,
                    spec,
                },
            );
            doc.start_css_animation(spec);
            self.queue_motion_cancel(id);
            if let Some(widget) = self.widgets.get_mut(&id) {
                from.apply_to_layout(&mut widget.props.layout);
            }
            self.sync_widget_layouts_for(doc, &[id]);
            return;
        }
        self.sync_widget_layouts_for(doc, &[id]);
    }
}

impl MessageBridge {
    /// Paint-only CSS `transform` (TransitionGroup FLIP). Writes `LayoutStyle.transform`
    /// without recascade so Scene extract sees the affine and CSS animations keep
    /// their clocks.
    pub fn set_paint_transform(
        &mut self,
        id: WidgetId,
        css: &str,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        if !self.widgets.contains_key(&id) {
            return;
        }
        if let Some(transform) = crate::css_map::parse_inline_paint_transform(css) {
            self.motion.paint_transform_overlays.insert(id, transform);
            self.motion.paint_transform_releases.remove(&id);
            if let Some(widget) = self.widgets.get_mut(&id) {
                widget.props.layout.transform = Some(transform);
            }
            self.sync_widget_layouts_for(doc, &[id]);
            return;
        }
        if self.motion.paint_transform_overlays.contains_key(&id) {
            self.motion.paint_transform_releases.insert(id);
            if let Some(widget) = self.widgets.get_mut(&id) {
                widget.props.layout.transform = None;
            }
            self.sync_widget_layouts_for(doc, &[id]);
            return;
        }
        if let Some(widget) = self.widgets.get_mut(&id) {
            widget.props.layout.transform = None;
        }
        self.sync_widget_layouts_for(doc, &[id]);
    }
}

impl MessageBridge {
    pub(super) fn snapshot_widget(&self, id: WidgetId) -> Option<CssPaintSnapshot> {
        let widget = self.widgets.get(&id)?;
        Some(CssPaintSnapshot::from_layout_resolved(
            &widget.props.layout,
            widget.props.containing_block_width,
            widget.props.containing_block_height,
            self.cascade.layout_viewport,
        ))
    }
}

impl MessageBridge {
    pub(super) fn should_start_keyframes(&self, id: WidgetId, name: &str) -> bool {
        if name.is_empty() || name.eq_ignore_ascii_case("none") {
            return false;
        }
        !self
            .motion
            .css_keyframes_name
            .get(&id)
            .is_some_and(|started| started.eq_ignore_ascii_case(name))
    }
}

impl MessageBridge {
    pub(super) fn queue_motion_cancel(&mut self, id: WidgetId) {
        if !self.motion.pending_motion_cancels.contains(&id) {
            self.motion.pending_motion_cancels.push(id);
        }
    }
}

impl MessageBridge {
    pub(crate) fn take_motion_cancels(&mut self) -> Vec<WidgetId> {
        std::mem::take(&mut self.motion.pending_motion_cancels)
    }
}

impl MessageBridge {
    pub(crate) fn take_motion_completes(&mut self) -> Vec<CssMotionComplete> {
        std::mem::take(&mut self.motion.pending_motion_completes)
    }
}

impl MessageBridge {
    pub fn computed_motion_for(&self, id: WidgetId) -> Option<&CssComputedMotion> {
        self.motion.computed_motion.get(&id)
    }
}
