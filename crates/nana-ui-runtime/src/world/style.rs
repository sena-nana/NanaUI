//! Style resolution and semantic palette inheritance.

use super::*;

impl UiWorld {
    pub(super) fn resolve_style(
        &mut self,
        id: StableNodeId,
        resolved: &mut HashSet<StableNodeId>,
    ) -> Result<(), UiWorldError> {
        if !self.contains(id) {
            return Err(UiWorldError::MissingNode(id));
        }
        if !resolved.insert(id) {
            return Ok(());
        }
        let parent = self.record(id).hierarchy.parent;
        if let Some(parent) = parent {
            self.resolve_style(parent, resolved)?;
        }
        let layout = Arc::clone(&self.record(id).style.layout);
        let inherited = parent
            .map(|parent| self.record(parent).resolved.0.as_ref().clone())
            .unwrap_or_default();
        let inherited_color = parent.and_then(|parent| self.record(parent).resolved.0.color);
        let (foreground, color, background, border_color) =
            self.palette_paint_colors(id, inherited_color);
        let visibility = layout.paint.visibility.unwrap_or(inherited.visibility);
        let box_visible = !layout.omits_box() && inherited.box_visible;
        let pointer_events =
            PointerEventsSpec::inherit_from(layout.pointer_events, inherited.pointer_events);
        let next = ComputedStyle {
            foreground,
            color,
            background,
            border_color,
            opacity: layout.opacity.unwrap_or(1.0) * inherited.opacity,
            visibility,
            box_visible,
            visible: box_visible
                && visibility != nana_ui_core::VisibilitySpec::Hidden
                && self.overlay_branch_active(id)
                && self.menu_branch_open(id),
            pointer_events,
            font_size: layout.font_size.unwrap_or(inherited.font_size),
            font_weight: layout.font_weight.or(inherited.font_weight),
            italic: layout.font_italic.unwrap_or(inherited.italic),
            font_family: layout
                .font_family
                .as_deref()
                .map(Arc::<str>::from)
                .or(inherited.font_family),
            line_height: layout.line_height.or(inherited.line_height),
            letter_spacing: layout.letter_spacing.unwrap_or(inherited.letter_spacing),
            font_features: layout
                .font_features
                .clone()
                .unwrap_or(inherited.font_features),
            font_variations: layout
                .font_variation_settings
                .clone()
                .unwrap_or(inherited.font_variations),
            font_kerning: layout.font_kerning.unwrap_or(inherited.font_kerning),
            word_break: layout.word_break.unwrap_or(inherited.word_break),
            line_break: layout.line_break.unwrap_or(inherited.line_break),
            direction: layout.dir.unwrap_or(inherited.direction),
            writing_mode: layout.writing_mode.unwrap_or(inherited.writing_mode),
        };
        {
            let resolved = &self.record(id).resolved;
            if resolved.0.as_ref() == &next && resolved.1 == self.palette_epoch {
                return Ok(());
            }
        }
        self.record_mut(id).resolved = ResolvedStyle(Arc::new(next), self.palette_epoch);
        Ok(())
    }
}

impl UiWorld {
    pub(super) fn palette_paint_colors(
        &self,
        id: StableNodeId,
        inherited_color: Option<[f32; 4]>,
    ) -> (
        SemanticColorRole,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
    ) {
        let local = &self.record(id).style;
        let paint = self.semantic_paint(id, local);
        let layout = local.layout.as_ref();
        let parent = self.record(id).hierarchy.parent;
        let inherited_foreground = parent
            .map(|parent| self.record(parent).resolved.0.foreground)
            .unwrap_or(SemanticColorRole::Text);
        let foreground = paint.foreground.unwrap_or(inherited_foreground);
        let color = layout.color.or_else(|| {
            paint
                .foreground
                .map(|role| self.style_model.color(role).as_rgba_array())
                .or(inherited_color)
                .or_else(|| Some(self.style_model.color(foreground).as_rgba_array()))
        });
        let background = layout.background.or_else(|| {
            paint
                .background
                .map(|role| self.style_model.color(role).as_rgba_array())
        });
        let border_color = layout.resolved_border_color().or_else(|| {
            paint
                .border
                .map(|role| self.style_model.color(role).as_rgba_array())
        });
        if let Some(transition) = self.hover_transitions.get(&id) {
            let progress = crate::Easing::EaseOutCubic.sample(
                (self
                    .animation_now
                    .saturating_sub(transition.start)
                    .as_secs_f32()
                    / nana_ui_core::motion::HOVER_COLOR.as_secs_f32())
                .clamp(0.0, 1.0),
            );
            let [color, background, border_color] = std::array::from_fn(|i| {
                interpolate_color(
                    transition.from[i],
                    [color, background, border_color][i],
                    progress,
                )
            });
            return (foreground, color, background, border_color);
        }
        (foreground, color, background, border_color)
    }
}

impl UiWorld {
    pub(super) fn inherited_palette_color(
        &self,
        mut parent: Option<StableNodeId>,
    ) -> Option<[f32; 4]> {
        while let Some(id) = parent {
            let local = &self.record(id).style;
            if let Some(color) = local.layout.color {
                return Some(color);
            }
            let paint = self.semantic_paint(id, local);
            if let Some(role) = paint.foreground {
                return Some(self.style_model.color(role).as_rgba_array());
            }
            parent = self.record(id).hierarchy.parent;
        }
        None
    }
}

impl UiWorld {
    pub(super) fn semantic_paint(
        &self,
        id: StableNodeId,
        local: &NodeStyle,
    ) -> crate::SemanticPaint {
        let mut paint = crate::SemanticPaint {
            foreground: local.foreground,
            background: local.background,
            border: local.border,
        };
        let accessibility = &self.record(id).accessibility;
        let selected = accessibility.checked == Some(true)
            || accessibility.mixed
            || accessibility.selected == Some(true);
        if selected {
            paint = paint.overlay(local.interaction.selected);
        }
        if self
            .input
            .pointer_hover
            .values()
            .any(|target| *target == id)
        {
            paint = paint.overlay(
                if selected && !local.interaction.selected_hovered.is_empty() {
                    local.interaction.selected_hovered
                } else {
                    local.interaction.hovered
                },
            );
        }
        if self
            .input
            .pointer_press
            .values()
            .any(|target| *target == id)
        {
            paint = paint.overlay(
                if selected && !local.interaction.selected_pressed.is_empty() {
                    local.interaction.selected_pressed
                } else {
                    local.interaction.pressed
                },
            );
        }
        if self.input.focused.get(&self.record(id).document) == Some(&id) {
            paint = paint.overlay(local.interaction.focused);
        }
        if accessibility.disabled && !accessibility.busy {
            paint = paint.overlay(local.interaction.disabled);
        }
        paint
    }
}

impl UiWorld {
    pub(super) fn mark_interaction_style(&mut self, id: StableNodeId) {
        self.mark(id, DirtyMask::STATE);
        if !self.record(id).style.interaction.is_empty() {
            self.mark(id, DirtyMask::STYLE | DirtyMask::RENDER);
        }
    }
}

impl UiWorld {
    /// Resolve inherited visual state for dirty nodes. Parent state is always
    /// resolved before its descendants, independent of stable ID order.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        let mut resolved = HashSet::new();
        for &id in ids {
            self.resolve_style(id, &mut resolved)?;
        }
        self.reconcile_focus(ids);
        Ok(())
    }
}

impl UiWorld {
    pub(super) fn apply_style_model(&mut self, next: StyleModelRef) {
        if self.style_model == next {
            return;
        }
        let hover_ids = self.hover_transitions.keys().copied().collect::<Vec<_>>();
        for id in hover_ids {
            self.cancel_hover_transition(id);
        }
        let previous_metrics = self.style_model.metrics;
        self.style_model = next;
        self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
        let mut bits = DirtyMask::RENDER;
        if self.style_model.metrics != previous_metrics {
            bits |= DirtyMask::LAYOUT;
        }
        let mut ids = Vec::new();
        for roots in self.live_document_roots.values() {
            for &root in roots {
                ids.extend(self.subtree_ids(root));
            }
        }
        for id in ids {
            self.mark(id, bits);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct HoverTransition {
    pub from: [Option<[f32; 4]>; 3],
    pub start: Duration,
    pub inherits_color: bool,
}

fn interpolate_color(
    from: Option<[f32; 4]>,
    to: Option<[f32; 4]>,
    progress: f32,
) -> Option<[f32; 4]> {
    if progress >= 1.0 {
        return to;
    }
    if progress <= 0.0 {
        return from;
    }
    let (mut a, mut b) = match (from, to) {
        (None, None) => return None,
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, a),
        (None, Some(b)) => (b, b),
    };
    if from.is_none() {
        a[3] = 0.0;
    }
    if to.is_none() {
        b[3] = 0.0;
    }
    Some(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * progress))
}

impl UiWorld {
    pub(super) fn hover_paint(&self, id: StableNodeId) -> [Option<[f32; 4]>; 3] {
        let inherited = self
            .record(id)
            .hierarchy
            .parent
            .and_then(|parent| self.record(parent).resolved.0.color);
        let (_, color, background, border) = self.palette_paint_colors(id, inherited);
        [color, background, border]
    }

    pub(super) fn transition_hover(&mut self, id: StableNodeId, from: [Option<[f32; 4]>; 3]) {
        if self.hover_transitions.contains_key(&id) {
            self.mark_hover_paint(id);
        }
        self.cancel_hover_transition(id);
        let to = self.hover_paint(id);
        if from != to {
            self.hover_transitions.insert(
                id,
                HoverTransition {
                    from,
                    start: self.animation_now,
                    inherits_color: from[0] != to[0],
                },
            );
            self.start_component_animation(
                id,
                crate::component_animation_kinds::HOVER,
                nana_ui_core::motion::HOVER_COLOR,
                crate::Easing::EaseOutCubic,
            );
            self.mark_hover_paint(id);
        }
    }
}

impl UiWorld {
    pub(super) fn mark_hover_paint(&mut self, target: StableNodeId) {
        let bits = DirtyMask::STYLE | DirtyMask::RENDER;
        if self
            .hover_transitions
            .get(&target)
            .is_some_and(|transition| transition.inherits_color)
        {
            self.mark_subtree(target, bits);
        } else {
            self.mark(target, bits);
        }
    }

    pub(super) fn cancel_hover_transition(&mut self, target: StableNodeId) {
        self.hover_transitions.remove(&target);
        if let Some(id) =
            crate::component_animation_id(crate::component_animation_kinds::HOVER, target)
            && let Some(animation) = self.animations.remove(&id)
        {
            self.animation_deadlines
                .remove(&(animation.next_deadline, id));
        }
    }
}
