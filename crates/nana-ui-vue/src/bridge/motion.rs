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
    /// Widgets whose Runtime timeline just started; host should cancel any leftover JS arm.
    pub(super) pending_motion_cancels: Vec<WidgetId>,
    /// Last `animation-name` started per widget. Same name still playing must
    /// not `start_css_animation` again (that would reset the clock).
    pub(super) css_keyframes_name: HashMap<WidgetId, String>,
    /// Compositor overlay IDs compiled from `@keyframes`.
    pub(super) css_keyframes_overlays: HashMap<WidgetId, Vec<nana_ui_runtime::AnimationId>>,
    /// Widgets whose `@keyframes` still have a CPU Progress spec.
    pub(super) css_keyframes_cpu: HashSet<WidgetId>,
    /// Overlay tracks of the widget's current `@keyframes`, kept after they
    /// finish: one that fills forwards still holds its property in Runtime
    /// until the animation is replaced or removed.
    pub(super) css_keyframes_tracks: HashMap<WidgetId, Vec<nana_ui_runtime::AnimationId>>,
    /// Axes each widget's CSS transitions and animations move, for the
    /// diagnostic that reports an axis no face of the text has.
    pub(super) css_font_axes: HashMap<WidgetId, Vec<[u8; 4]>>,
    /// Widgets such a track currently runs on, as of the last refresh.
    pub(super) ineffective_font_axes: usize,
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

    #[cfg(test)]
    pub(super) fn css_transition_from(&self, id: WidgetId) -> Option<CssPaintSnapshot> {
        self.motion
            .css_transitions
            .get(&id)
            .map(|transition| transition.from.clone())
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
            from.apply_cpu_to_layout(&mut widget.props.layout);
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
    /// Consume a released FLIP overlay: play compositor invert → identity on
    /// the same FLIP track. Duration/easing come from the CSS move class.
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
        let motion = self.motion.computed_motion.get(&id).cloned();
        let duration_ms = motion
            .as_ref()
            .and_then(|motion| {
                crate::css_interactive_apply::parse_css_time_ms(&motion.transition_duration)
            })
            .unwrap_or(0.0);
        let includes_transform = motion
            .as_ref()
            .map(css_transition_includes_transform)
            .unwrap_or(false);
        if overlay != nana_ui_core::PaintTransform::default()
            && !css_flip_move_ready(motion.as_ref(), duration_ms, includes_transform)
        {
            // Move class not applied yet. Keep Invert hold for the next recascade.
            return;
        }
        self.motion.paint_transform_overlays.remove(&id);
        self.motion.paint_transform_releases.remove(&id);
        let Some(node) = nana_ui_runtime::StableNodeId::new(id) else {
            return;
        };
        let now = doc.runtime_now();
        if overlay != nana_ui_core::PaintTransform::default()
            && duration_ms > 0.0
            && let Some(motion) = motion.as_ref()
        {
            let easing =
                crate::css_interactive_apply::easing_from_css(&motion.transition_timing_function);
            let duration = crate::css_interactive_apply::css_ms_duration(duration_ms);
            if let Some(spec) =
                nana_ui_runtime::layout_flip_play_spec(node, overlay, now, duration, easing)
            {
                let mut from = self
                    .snapshot_widget(id)
                    .unwrap_or_else(|| CssPaintSnapshot::from_layout(&LayoutStyle::default()));
                from.transform = Some(overlay);
                let to = CssPaintSnapshot::from_layout(
                    &self
                        .widgets
                        .get(&id)
                        .map(|widget| widget.props.layout.clone())
                        .unwrap_or_default(),
                );
                self.start_compiled_transition(
                    doc,
                    id,
                    from,
                    to,
                    crate::css_interactive_apply::CompiledCssMotion {
                        cpu: None,
                        overlays: vec![spec],
                    },
                    false,
                );
                return;
            }
        }
        if let Some(flip_id) = nana_ui_runtime::component_animation_id(
            nana_ui_runtime::component_animation_kinds::FLIP,
            node,
        ) {
            doc.stop_css_animation(flip_id);
        }
    }
}

impl MessageBridge {
    /// Paint-only CSS `transform` (TransitionGroup FLIP). Invert is a compositor
    /// overlay; Last layout is already committed. Never writes LayoutBox.
    pub fn set_paint_transform(
        &mut self,
        id: WidgetId,
        css: &str,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        if !self.widgets.contains_key(&id) {
            return;
        }
        let Some(node) = nana_ui_runtime::StableNodeId::new(id) else {
            return;
        };
        let now = doc.runtime_now();
        if let Some(transform) = crate::css_map::parse_inline_paint_transform(css) {
            self.motion.paint_transform_overlays.insert(id, transform);
            self.motion.paint_transform_releases.remove(&id);
            if let Some(spec) = nana_ui_runtime::layout_flip_hold_spec(node, transform, now) {
                doc.start_css_animation(spec);
            }
            return;
        }
        if self.motion.paint_transform_overlays.contains_key(&id) {
            self.motion.paint_transform_releases.insert(id);
            self.maybe_release_flip_paint_transform(id, doc);
            return;
        }
        if let Some(id) = nana_ui_runtime::component_animation_id(
            nana_ui_runtime::component_animation_kinds::FLIP,
            node,
        ) {
            doc.stop_css_animation(id);
        }
    }
}

impl MessageBridge {
    /// The widget's cascaded paint, with its axes as the cascade gives them
    /// (inherited ones included).
    pub(super) fn snapshot_widget(&self, id: WidgetId) -> Option<CssPaintSnapshot> {
        let mut snapshot = self.widget_paint(id)?;
        snapshot.font_variations = self.cascaded_font_variations(id);
        Some(snapshot)
    }

    /// The widget's own layout as a snapshot; its axes are the caller's to fill.
    fn widget_paint(&self, id: WidgetId) -> Option<CssPaintSnapshot> {
        let widget = self.widgets.get(&id)?;
        Some(CssPaintSnapshot::from_layout_resolved(
            &widget.props.layout,
            widget.props.containing_block_width,
            widget.props.containing_block_height,
            self.cascade.layout_viewport,
        ))
    }

    /// The axes the cascade gives `id`: its own declaration, else the nearest
    /// ancestor's. Widgets do not copy inherited axes (Runtime inherits them),
    /// so an inherited value is found here, not on the widget.
    pub(super) fn cascaded_font_variations(
        &self,
        id: WidgetId,
    ) -> Option<Vec<nana_ui_core::FontVariationSetting>> {
        let mut current = Some(id);
        while let Some(id) = current {
            let widget = self.widgets.get(&id)?;
            if let Some(axes) = &widget.props.layout.font_variation_settings {
                return (!axes.is_empty()).then(|| axes.clone());
            }
            current = widget.parent;
        }
        None
    }

    /// The axes `id` shows right now in Runtime: a transition that starts or
    /// retargets mid-flight starts from these, not from the cascade, which
    /// already moved to the new destination.
    pub(super) fn presented_font_variations(
        doc: &crate::tree::NanaTreeDocument,
        id: WidgetId,
    ) -> Option<Vec<nana_ui_core::FontVariationSetting>> {
        let style = doc
            .world()
            .computed_style(nana_ui_runtime::StableNodeId::new(id)?)?;
        (!style.font_variations.is_empty()).then(|| style.font_variations.clone())
    }

    /// Interruption from is the current visual, not logical. Compositor
    /// opacity/transform live on the overlay; layout is already the retarget
    /// destination, so a layout snapshot makes from==to and `compile` drops.
    pub(super) fn snapshot_from_presentation(
        &self,
        doc: &crate::tree::NanaTreeDocument,
        id: WidgetId,
    ) -> Option<CssPaintSnapshot> {
        let mut snapshot = self.widget_paint(id)?;
        snapshot.font_variations = Self::presented_font_variations(doc, id);
        let Some(node) = nana_ui_runtime::StableNodeId::new(id) else {
            return Some(snapshot);
        };
        let now = doc.runtime_now();
        if let Some(nana_ui_runtime::MotionValue::Scalar(opacity)) = doc
            .world()
            .presentation_applied_value(node, nana_ui_runtime::AnimatableProperty::Opacity, now)
        {
            snapshot.opacity = Some(opacity);
        }
        if let Some(nana_ui_runtime::MotionValue::Transform(transform)) = doc
            .world()
            .presentation_applied_value(node, nana_ui_runtime::AnimatableProperty::Transform, now)
        {
            snapshot.transform = Some(transform);
        }
        Some(snapshot)
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
    pub(super) fn start_compiled_transition(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
        id: WidgetId,
        from: CssPaintSnapshot,
        to: CssPaintSnapshot,
        compiled: crate::css_interactive_apply::CompiledCssMotion,
        retarget: bool,
    ) {
        if let Some(previous) = self.motion.css_transitions.get(&id) {
            let keep: std::collections::HashSet<_> = compiled
                .overlays
                .iter()
                .map(|spec| spec.id)
                .chain(compiled.cpu.as_ref().map(|spec| spec.id))
                .collect();
            if let Some(cpu_id) = previous.cpu_id
                && !keep.contains(&cpu_id)
            {
                doc.stop_css_animation(cpu_id);
            }
            for overlay in &previous.overlay_ids {
                if !keep.contains(overlay) {
                    doc.stop_css_animation(*overlay);
                }
            }
        }
        self.sync_widget_layouts_for(doc, &[id]);
        self.note_css_font_axes(id, &compiled.overlays);
        for spec in &compiled.overlays {
            let spec = if retarget {
                spec.clone()
                    .with_interrupt(nana_ui_runtime::MotionInterrupt::Retarget)
            } else {
                spec.clone()
            };
            doc.start_css_animation(spec);
        }
        if let Some(spec) = &compiled.cpu {
            doc.start_css_animation(spec.clone());
        }
        let Some(primary) = compiled.primary_spec() else {
            return;
        };
        let cpu_properties = cpu_transition_properties(&parse_transition_properties(
            &self
                .motion
                .computed_motion
                .get(&id)
                .map(|motion| motion.transition_property.clone())
                .unwrap_or_default(),
        ));
        self.motion.css_transition_base.insert(id, from.clone());
        self.motion.css_transition_progress.insert(id, 0.0);
        self.motion.css_transitions.insert(
            id,
            ActiveCssTransition {
                from: from.clone(),
                to,
                spec: primary,
                overlay_ids: compiled.overlay_ids(),
                cpu_id: compiled.cpu.as_ref().map(|spec| spec.id),
                cpu_properties,
            },
        );
        self.queue_motion_cancel(id);
        // Pinning holds `from` until the first CPU sample rewrites it. Font
        // axes have no such sample — Runtime overlays them on the computed
        // style — so they must not pin, or an unlisted property that changed
        // alongside them would stay at its old value.
        if compiled.cpu.is_some()
            || compiled.overlays.iter().any(|spec| {
                matches!(
                    spec.property,
                    nana_ui_runtime::AnimatableProperty::Width
                        | nana_ui_runtime::AnimatableProperty::Height
                )
            })
        {
            self.pin_host_driven_transition_paint(doc, id, &from);
        }
        self.motion.paint_transform_overlays.remove(&id);
        self.motion.paint_transform_releases.remove(&id);
    }

    pub(super) fn start_compiled_keyframes(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
        id: WidgetId,
        name: String,
        compiled: crate::css_interactive_apply::CompiledCssMotion,
    ) {
        self.sync_widget_layouts_for(doc, &[id]);
        self.note_css_font_axes(id, &compiled.overlays);
        let tracks = compiled.overlay_ids();
        for previous in self
            .motion
            .css_keyframes_tracks
            .insert(id, tracks.clone())
            .unwrap_or_default()
        {
            if !tracks.contains(&previous) {
                stop_live_track(doc, previous);
            }
        }
        for spec in &compiled.overlays {
            doc.start_css_animation(spec.clone());
        }
        if let Some(spec) = &compiled.cpu {
            doc.start_css_animation(spec.clone());
        }
        self.motion.css_keyframes_name.insert(id, name);
        if compiled.overlays.is_empty() {
            self.motion.css_keyframes_overlays.remove(&id);
        } else {
            self.motion.css_keyframes_overlays.insert(id, tracks);
        }
        if compiled.cpu.is_some() {
            self.motion.css_keyframes_cpu.insert(id);
        } else {
            self.motion.css_keyframes_cpu.remove(&id);
        }
        self.queue_motion_cancel(id);
    }

    /// `animation-name` no longer names the running animation: its tracks
    /// stop, and what the finished ones held goes back to the cascade — a
    /// fill lasts only as long as its animation applies.
    pub(super) fn remove_css_keyframes(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
        id: WidgetId,
    ) {
        for track in self
            .motion
            .css_keyframes_tracks
            .remove(&id)
            .unwrap_or_default()
        {
            stop_live_track(doc, track);
        }
        self.clear_css_keyframes(id);
    }

    /// Stops `id`'s running font-axis transition tracks, so its text shows
    /// the cascaded axes at once: the change they were heading for is gone
    /// and the new one does not interpolate.
    pub(super) fn stop_css_font_axis_transition(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
        id: WidgetId,
    ) {
        let Some(mut transition) = self.motion.css_transitions.get(&id).cloned() else {
            return;
        };
        let tags = self
            .motion
            .css_font_axes
            .get(&id)
            .cloned()
            .unwrap_or_default();
        for tag in tags {
            let track = crate::css_interactive_apply::css_transition_font_axis_id(id, tag);
            if transition.tracks_sample(track) {
                stop_live_track(doc, track);
                transition.note_finished(track);
            }
        }
        if transition.all_tracks_finished() {
            self.motion.css_transitions.remove(&id);
            self.motion.css_transition_base.remove(&id);
            self.motion.css_transition_progress.remove(&id);
        } else {
            self.motion.css_transitions.insert(id, transition);
        }
    }

    /// Remembers which axes `specs` move on `id`.
    pub(super) fn note_css_font_axes(
        &mut self,
        id: WidgetId,
        specs: &[nana_ui_runtime::AnimationSpec],
    ) {
        let axes = self.motion.css_font_axes.entry(id).or_default();
        for spec in specs {
            if let nana_ui_runtime::AnimatableProperty::FontAxis(tag) = spec.property
                && !axes.contains(&tag)
            {
                axes.push(tag);
            }
        }
    }

    /// Counts widgets with a live CSS font-axis track that changes nothing,
    /// for [`crate::css_cascade::UnsupportedCssReport::font_axis_animations`].
    /// Read after the frame shaped its text: which faces a text uses is only
    /// known then.
    #[cfg_attr(not(feature = "scene-view"), allow(dead_code))]
    pub(crate) fn refresh_font_axis_diagnostics(&mut self, doc: &crate::tree::NanaTreeDocument) {
        if self.motion.css_font_axes.is_empty() {
            self.motion.ineffective_font_axes = 0;
            return;
        }
        let world = doc.world();
        self.motion.css_font_axes.retain(|widget, axes| {
            axes.retain(|tag| {
                world.animation_is_live(crate::css_interactive_apply::css_transition_font_axis_id(
                    *widget, *tag,
                )) || world.animation_is_live(
                    crate::css_interactive_apply::css_keyframes_font_axis_id(*widget, *tag),
                )
            });
            !axes.is_empty()
        });
        self.motion.ineffective_font_axes = self
            .motion
            .css_font_axes
            .iter()
            .filter(|(widget, axes)| {
                nana_ui_runtime::StableNodeId::new(**widget).is_some_and(|node| {
                    axes.iter()
                        .any(|tag| world.font_axis_is_ineffective(node, *tag))
                })
            })
            .count();
    }

    pub(super) fn clear_css_keyframes(&mut self, id: WidgetId) {
        self.motion.css_keyframes_name.remove(&id);
        self.motion.css_keyframes_overlays.remove(&id);
        self.motion.css_keyframes_cpu.remove(&id);
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

/// Stops a track that is still running or still holds its property.
/// Anything else is already gone, and asking Runtime to stop it would fail
/// the batch it is committed with.
fn stop_live_track(doc: &mut crate::tree::NanaTreeDocument, id: nana_ui_runtime::AnimationId) {
    if doc.world().animation_is_live(id) {
        doc.stop_css_animation(id);
    }
}

fn css_transition_includes_transform(motion: &CssComputedMotion) -> bool {
    if motion.transition_property.eq_ignore_ascii_case("none")
        || motion.transition_property.is_empty()
    {
        return false;
    }
    let listed = parse_transition_properties(&motion.transition_property);
    listed.is_empty()
        || listed.iter().any(|property| {
            property.eq_ignore_ascii_case("all") || property.eq_ignore_ascii_case("transform")
        })
}

fn css_flip_move_ready(
    motion: Option<&CssComputedMotion>,
    duration_ms: f32,
    includes_transform: bool,
) -> bool {
    if duration_ms > 0.0 && includes_transform {
        return true;
    }
    let Some(motion) = motion else {
        return false;
    };
    let listed = parse_transition_properties(&motion.transition_property);
    duration_ms <= 0.0
        && listed
            .iter()
            .any(|property| property.eq_ignore_ascii_case("transform"))
}
