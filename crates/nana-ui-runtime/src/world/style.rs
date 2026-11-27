//! Style resolution and semantic palette inheritance.

use super::*;
use crate::InheritedStyle;

impl UiWorld {
    /// A node with no local computed overrides publishes its parent's style.
    ///
    /// Building a `ComputedStyle` reads a 4.8 KB layout just to discover the
    /// result already lives on the parent. Sharing that `Arc` is the same
    /// answer the full resolver reaches when every authored field inherits.
    /// Returns `Ok(true)` when this node is finished.
    fn publish_inherited_computed_style(
        &mut self,
        id: StableNodeId,
        parent: Option<StableNodeId>,
        propagated_from_theme: bool,
        queue: &mut std::collections::VecDeque<StableNodeId>,
        propagated_theme: &mut HashSet<StableNodeId>,
        work: &mut ThemeWorkCounters,
    ) -> Result<bool, UiWorldError> {
        let Some(parent) = parent else {
            return Ok(false);
        };
        if !self.invalidation.theme_state.theme_scopes.is_empty()
            && self
                .invalidation
                .theme_state
                .theme_scopes
                .scope_for_owner(id)
                .is_some()
        {
            return Ok(false);
        }
        let parent_scope = self.nodes.node_scope(parent);
        if !self.overlay_branch_active(id)
            || !self.menu_branch_open(id)
            || (!self.motion.presentation.is_empty()
                && (!self.font_axis_overlay(id).is_empty()
                    || self
                        .presentation_color_overlay(id, crate::AnimatableProperty::Background)
                        .is_some()
                    || self
                        .presentation_color_overlay(id, crate::AnimatableProperty::Color)
                        .is_some()))
            || self.motion.hover_transitions.contains_key(&id)
        {
            return Ok(false);
        }
        let (parent_style, parent_scope_gen) = {
            let parent_record = self.record(parent);
            if parent_record.resolved.1 != self.resolved_palette_epoch()
                || parent_record.resolved.3.ext.theme_scope != parent_scope
            {
                return Ok(false);
            }
            (
                Arc::clone(&parent_record.resolved.3),
                parent_record.resolved.2,
            )
        };
        let writing_mode = parent_style.writing_mode;
        let direction = parent_style.direction;
        let text_orientation = parent_style.text_orientation;
        let has_visual = self.widget_visual_ref(id).is_some();
        let (
            inherited,
            same_identity,
            has_text,
            show_text,
            prev_computed,
            prev_epoch,
            prev_unpainted,
        ) = {
            let record = self.record(id);
            let inherited = record.style.computed_style_is_inherited()
                && (crate::components::layout_style_is_default(&record.resolved_layout)
                    || !record.resolved_layout.overrides_computed_style());
            (
                inherited,
                inherited
                    && Arc::ptr_eq(&record.resolved.0, &parent_style)
                    && Arc::ptr_eq(&record.resolved.3, &parent_style)
                    && record.resolved.1 == self.resolved_palette_epoch()
                    && record.resolved.2 == parent_scope_gen,
                has_visual || record.has_text || matches!(record.kind.as_ref(), NodeKind::Text),
                !record.resolved.0.visible && parent_style.visible,
                Arc::clone(&record.resolved.0),
                record.resolved.1,
                Arc::clone(&record.resolved.3),
            )
        };
        // A node that names its own language or typography scale, or follows
        // a responsive rule, resolves a style of its own.
        if !inherited
            || self.nodes.language(id).is_some()
            || self.nodes.text_scale(id).is_some()
            || self.overlays_a11y.responsive.follows(id)
            || self.localized_text(id).is_some()
            || self.locale_direction(id).is_some()
        {
            return Ok(false);
        }
        self.nodes.set_node_scope(id, parent_scope);
        let theme_change = self
            .invalidation
            .theme_state
            .pending_theme_eval
            .remove(&id)
            .unwrap_or(nana_ui_core::ThemeDependencyMask::NONE);
        let theme_triggered = propagated_from_theme || !theme_change.is_empty();
        {
            let record = self.record_mut(id);
            record.inherited_writing = nana_ui_core::WritingContext::new(writing_mode, direction);
            record.inherited_orientation = text_orientation;
        }
        let epoch = self.resolved_palette_epoch();
        if same_identity {
            let visual_needs_repaint = self.theme_change_requires_visual_repaint(id, theme_change);
            if visual_needs_repaint {
                work.resolved_style_changed += 1;
                work.record_paint_invalidation(1);
                self.mark(id, DirtyMask::RENDER);
                if self.node_painter(id).is_some() {
                    self.mark_repaint(id);
                }
            } else {
                work.resolved_style_unchanged += 1;
                work.record_skipped();
            }
            return Ok(true);
        }
        let was_uninitialized = prev_epoch == 0;
        let style_changed = was_uninitialized
            || (!Arc::ptr_eq(&prev_computed, &parent_style)
                && !computed_style_eq(&prev_computed, &parent_style));
        if !style_changed {
            self.record_mut(id).resolved = ResolvedStyle(
                Arc::clone(&parent_style),
                epoch,
                parent_scope_gen,
                Arc::clone(&parent_style),
            );
            let visual_needs_repaint = self.theme_change_requires_visual_repaint(id, theme_change);
            if visual_needs_repaint {
                work.resolved_style_changed += 1;
                work.record_paint_invalidation(1);
                self.mark(id, DirtyMask::RENDER);
                if self.node_painter(id).is_some() {
                    self.mark_repaint(id);
                }
            } else {
                work.resolved_style_unchanged += 1;
                work.record_skipped();
            }
            return Ok(true);
        }
        if has_text {
            self.invalidate_text_for_style(
                id,
                &parent_style,
                theme_triggered,
                was_uninitialized,
                work,
            );
        }
        if show_text {
            self.text.text_shown.push(id);
        }
        if hit_relevant_style_changed(was_uninitialized, &prev_computed, &parent_style) {
            self.invalidation.style_hit_changed = true;
        }
        if (!was_uninitialized && theme_triggered)
            || self.theme_change_requires_visual_repaint(id, theme_change)
        {
            self.mark(id, DirtyMask::RENDER);
            work.record_paint_invalidation(1);
            if self.node_painter(id).is_some() {
                self.mark_repaint(id);
            }
        }
        let inherited_to_children_changed = !was_uninitialized
            && !Arc::ptr_eq(&prev_unpainted, &parent_style)
            && !computed_style_eq(&prev_unpainted, &parent_style);
        work.resolved_style_changed += 1;
        work.record_resolved();
        self.record_mut(id).resolved = ResolvedStyle(
            Arc::clone(&parent_style),
            epoch,
            parent_scope_gen,
            Arc::clone(&parent_style),
        );
        if inherited_to_children_changed {
            let children = Arc::clone(&self.record(id).hierarchy.children);
            for &child in children.iter() {
                if theme_triggered {
                    propagated_theme.insert(child);
                }
                queue.push_back(child);
            }
        }
        Ok(true)
    }

    /// Resolves `id` and every ancestor not resolved yet, outermost first: a
    /// node inherits from its parent's resolved style. The ancestors wait on
    /// the heap, so any depth resolves on any stack.
    fn resolve_style<const SHARE: bool>(
        &mut self,
        id: StableNodeId,
        propagated_from_theme: bool,
        resolved: &mut HashSet<StableNodeId>,
        queue: &mut std::collections::VecDeque<StableNodeId>,
        propagated_theme: &mut HashSet<StableNodeId>,
        work: &mut ThemeWorkCounters,
    ) -> Result<(), UiWorldError> {
        let mut chain = Vec::new();
        let mut cursor = Some(id);
        while let Some(node) = cursor {
            if !self.contains(node) {
                return Err(UiWorldError::MissingNode(node));
            }
            if !resolved.insert(node) {
                break;
            }
            chain.push(node);
            cursor = self.record(node).hierarchy.parent;
        }
        for node in chain.into_iter().rev() {
            let node_propagated =
                (node == id && propagated_from_theme) || propagated_theme.contains(&node);
            self.resolve_one_style::<SHARE>(node, node_propagated, queue, propagated_theme, work)?;
        }
        Ok(())
    }

    /// Resolves `id` against its parent's resolved style.
    fn resolve_one_style<const SHARE: bool>(
        &mut self,
        id: StableNodeId,
        propagated_from_theme: bool,
        queue: &mut std::collections::VecDeque<StableNodeId>,
        propagated_theme: &mut HashSet<StableNodeId>,
        work: &mut ThemeWorkCounters,
    ) -> Result<(), UiWorldError> {
        let parent = self.record(id).hierarchy.parent;
        if SHARE
            && self.publish_inherited_computed_style(
                id,
                parent,
                propagated_from_theme,
                queue,
                propagated_theme,
                work,
            )?
        {
            return Ok(());
        }
        // The layout the pipeline reads: design intent and a responsive
        // variant applied. Text, paint and visibility follow the variant.
        let layout = Arc::clone(&self.record(id).resolved_layout);
        // Only a handful of fields are read out of the parent, so share its Arc
        // instead of cloning the whole `ComputedStyle` (and its three heap
        // fields) once per node. A borrow would pin `&mut self` to the end.
        let inherited_style = parent
            .map(|parent| Arc::clone(&self.record(parent).resolved.0))
            .unwrap_or_else(crate::store::interned_default_style);
        let inherited = inherited_style.as_ref();
        let inherited_color = parent.and(inherited.color);
        let doc = self.record(id).document;
        let parent_scope = parent
            .map(|p| self.nodes.node_scope(p))
            .unwrap_or(nana_ui_core::ThemeScopeId::ROOT);
        let effective_scope = if self.invalidation.theme_state.theme_scopes.is_empty() {
            nana_ui_core::ThemeScopeId::ROOT
        } else {
            self.invalidation
                .theme_state
                .theme_scopes
                .scope_for_owner(id)
                .unwrap_or(parent_scope)
        };
        let scope_gen = self
            .invalidation
            .theme_state
            .theme_scopes
            .generation(effective_scope)
            .0;
        self.nodes.set_node_scope(id, effective_scope);
        // The colour scope: this node's own overrides written over its
        // parent's. Nested scopes are flattened here, once per resolve, so
        // nothing downstream walks the chain.
        let color_scope = {
            let outer = parent.and(inherited.color_scope.as_ref());
            match (self.record(id).style.color_overrides.as_ref(), outer) {
                (None, outer) => outer.cloned(),
                (Some(own), None) => Some(Arc::clone(own)),
                (Some(own), Some(outer)) => Some(Arc::new(own.over(outer))),
            }
        };
        let model = self.style_model_for_scope(doc, effective_scope, color_scope.as_deref());
        let (foreground, color, background, border_color) =
            self.palette_paint_colors(id, inherited_color, model);
        let local_paint = self.semantic_paint(id, &self.record(id).style);
        let color_override = layout.paint_colors.color.is_some()
            || local_paint.foreground.is_some()
            || local_paint.foreground_mix.is_some();
        let visibility = layout.paint.visibility.unwrap_or(inherited.visibility);
        // Overlay/menu presence is structural: descendants cannot make a closed
        // branch paintable again by resolving their own local visibility.
        // An inactive retained branch therefore omits its whole subtree just
        // like display:none. Keeping this only in `visible` would let
        // descendants inherit a live box and leave their paint primitives
        // behind on close.
        let box_visible = !layout.omits_box()
            && inherited.box_visible
            && self.overlay_branch_active(id)
            && self.menu_branch_open(id);
        // An inert subtree takes no pointer whatever its nodes specify.
        let inert = layout.inert || inherited.inert;
        let pointer_events = if inert {
            PointerEventsSpec::None
        } else {
            PointerEventsSpec::inherit_from(layout.pointer_events, inherited.pointer_events)
        };
        let text_scale = match self.nodes.text_scale(id) {
            Some(own) => *own,
            None if parent.is_some() => inherited.text_scale,
            None => self.root_text_scale(doc),
        };
        // Roots inherit typography from the active scope/document theme. Descendants still
        // inherit their parent, including explicit application font overrides.
        let typography = self.theme_for_scope(doc, effective_scope).typography();
        let font_size_base = layout.font_size.unwrap_or_else(|| {
            if parent.is_some() {
                inherited.font_size_base
            } else {
                typography.body
            }
        });
        let font_weight = layout.font_weight.or_else(|| {
            if parent.is_some() {
                inherited.font_weight
            } else {
                Some(typography.regular)
            }
        });
        let line_height_base = layout.line_height.or_else(|| {
            if parent.is_some() {
                inherited.line_height_base
            } else {
                // A unitless root line follows larger descendant fonts too;
                // at Body it is exactly the theme's Line box.
                Some(nana_ui_core::LineHeightSpec::Relative(
                    typography.line / typography.body,
                ))
            }
        });
        let has_text_ext = !layout.text_ext.is_shared_default();
        let text_ext = &*layout.text_ext;
        let overlay_bg = self.presentation_color_overlay(id, crate::AnimatableProperty::Background);
        let overlay_fg = self.presentation_color_overlay(id, crate::AnimatableProperty::Color);
        let paint_colors = if layout.paint_colors.is_shared_default()
            && inherited.paint_colors.is_shared_default()
            && overlay_bg.is_none()
            && overlay_fg.is_none()
        {
            nana_ui_core::shared::Shared::default()
        } else {
            let mut slots = nana_ui_core::PaintColorSlots {
                color: overlay_fg.map(nana_ui_core::PaintColor::srgb).or_else(|| {
                    layout.paint_colors.color.or_else(|| {
                        (!color_override)
                            .then_some(inherited.paint_colors.color)
                            .flatten()
                    })
                }),
                // Backgrounds, borders, and outlines are non-inherited CSS
                // properties. Keep their authoring-space values local to the
                // node while the foreground and selection colors follow the
                // existing inherited compatibility fields below.
                background: overlay_bg
                    .map(nana_ui_core::PaintColor::srgb)
                    .or(layout.paint_colors.background),
                border: layout.paint_colors.border,
                border_top: layout.paint_colors.border_top,
                border_right: layout.paint_colors.border_right,
                border_bottom: layout.paint_colors.border_bottom,
                border_left: layout.paint_colors.border_left,
                outline: layout.paint_colors.outline,
                selection_background: layout
                    .paint_colors
                    .selection_background
                    .or(inherited.paint_colors.selection_background),
                selection_color: layout
                    .paint_colors
                    .selection_color
                    .or(inherited.paint_colors.selection_color),
            };
            if slots.has_dynamic_theme() {
                slots.resolve_theme(model);
            }
            if slots == nana_ui_core::PaintColorSlots::default() {
                nana_ui_core::shared::Shared::default()
            } else {
                slots.into()
            }
        };
        // Its own language; else, for localized text, the language its
        // locale shapes in; else its parent's.
        let language = match self.nodes.language(id) {
            Some(own) => Some(own.clone()),
            None => match self.localized_language(id) {
                Some(localized) => Some(localized),
                None if parent.is_some() => inherited.ext.language.clone(),
                None => self.root_language(),
            },
        };
        let selection_background = paint_colors
            .selection_background
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(inherited.ext.selection_background);
        let selection_color = paint_colors
            .selection_color
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(inherited.ext.selection_color);
        let text_typography = text_ext
            .text_typography
            .inherit_from(inherited.ext.text_typography);
        let text_emphasis = if has_text_ext {
            nana_ui_core::TextEmphasis {
                style: text_ext
                    .text_emphasis_style
                    .clone()
                    .unwrap_or_else(|| inherited.ext.text_emphasis.style.clone()),
                position: text_ext
                    .text_emphasis_position
                    .unwrap_or(inherited.ext.text_emphasis.position),
                color: text_ext
                    .text_emphasis_color
                    .unwrap_or(inherited.ext.text_emphasis.color),
            }
        } else {
            inherited.ext.text_emphasis.clone()
        };
        let ext_same_base = inherited.ext.language == language
            && inherited.ext.color_scope == color_scope
            && inherited.ext.theme_scope == effective_scope
            && inherited.ext.selection_background == selection_background
            && inherited.ext.selection_color == selection_color
            && (self.motion.presentation.is_empty() || self.font_axis_overlay(id).is_empty())
            && (!has_text_ext
                || (inherited.ext.text_typography == text_typography
                    && inherited.ext.text_emphasis == text_emphasis
                    && (text_ext.font_features.is_none()
                        || text_ext.font_features.as_deref()
                            == Some(inherited.ext.font_features.as_slice()))
                    && (text_ext.font_variation_settings.is_none()
                        || text_ext.font_variation_settings.as_deref()
                            == Some(inherited.ext.font_variations.as_slice()))));
        let ext = if ext_same_base {
            inherited.ext.clone()
        } else {
            crate::InheritedStyleExt {
                selection_background,
                selection_color,
                font_features: text_ext
                    .font_features
                    .clone()
                    .unwrap_or_else(|| inherited.ext.font_features.clone()),
                font_variations: {
                    let mut axes = text_ext
                        .font_variation_settings
                        .clone()
                        .unwrap_or_else(|| inherited.ext.font_variations.clone());
                    for (tag, value) in self.font_axis_overlay(id) {
                        match value {
                            Some(value) => {
                                nana_ui_core::FontVariationSetting::set_axis(&mut axes, tag, value)
                            }
                            None => axes.retain(|axis| axis.tag != tag),
                        }
                    }
                    axes
                },
                text_typography,
                text_emphasis,
                language,
                color_scope,
                theme_scope: effective_scope,
            }
            .into()
        };
        let next_inherited = crate::InheritedStyle {
            foreground,
            color,
            opacity: layout.opacity.unwrap_or(1.0) * inherited.opacity,
            visibility,
            box_visible,
            visible: box_visible && visibility != nana_ui_core::VisibilitySpec::Hidden,
            pointer_events,
            inert,
            cursor: match &layout.cursor {
                Some(spec) => Some(crate::components::shared_cursor(spec)),
                None => inherited.cursor.clone(),
            },
            user_select: layout.user_select.unwrap_or(inherited.user_select),
            font_size: font_size_base * text_scale,
            font_weight,
            italic: layout.font_italic.unwrap_or(inherited.italic),
            font_family: text_ext
                .font_family
                .as_deref()
                .map(Arc::<str>::from)
                .or_else(|| inherited.font_family.clone()),
            line_height: line_height_base.map(|spec| scaled_line_height(spec, text_scale)),
            letter_spacing: text_ext.letter_spacing.unwrap_or(inherited.letter_spacing),
            font_kerning: text_ext.font_kerning.unwrap_or(inherited.font_kerning),
            word_break: text_ext.word_break.unwrap_or(inherited.word_break),
            line_break: text_ext.line_break.unwrap_or(inherited.line_break),
            hyphens: text_ext.hyphens.unwrap_or(inherited.hyphens),
            direction: layout.dir.unwrap_or(inherited.direction),
            writing_mode: layout.writing_mode.unwrap_or(inherited.writing_mode),
            text_orientation: text_ext
                .text_orientation
                .unwrap_or(inherited.text_orientation),
            dominant_baseline: text_ext
                .dominant_baseline
                .unwrap_or(inherited.dominant_baseline),
            text_scale,
            font_size_base,
            line_height_base,
            ext,
        };
        let ring_color = self.palette_ring_color(id, model);
        // Written before the early return: a node that declares its own
        // writing mode resolves to the same style when its parent's changes,
        // but its containing block is still the parent.
        // The computed values, not the used context: `upright` makes this
        // node's used direction `ltr`, but its children inherit `direction`.
        let record = self.record_mut(id);
        (record.inherited_writing, record.inherited_orientation) = if parent.is_some() {
            (
                nana_ui_core::WritingContext::new(inherited.writing_mode, inherited.direction),
                inherited.text_orientation,
            )
        } else {
            Default::default()
        };
        let theme_change = self
            .invalidation
            .theme_state
            .pending_theme_eval
            .remove(&id)
            .unwrap_or(nana_ui_core::ThemeDependencyMask::NONE);
        let theme_triggered = propagated_from_theme || !theme_change.is_empty();
        let epoch = self.resolved_palette_epoch();
        let (prev_computed, prev_epoch, prev_scope_gen, prev_unpainted) = {
            let resolved = &self.record(id).resolved;
            (
                Arc::clone(&resolved.0),
                resolved.1,
                resolved.2,
                Arc::clone(&resolved.3),
            )
        };
        let was_uninitialized = prev_epoch == 0;
        let same_as_prev = !was_uninitialized
            && prev_computed.background == background
            && prev_computed.border_color == border_color
            && prev_computed.ring_color == ring_color
            && prev_computed.paint_colors == paint_colors
            && *prev_computed.inherited == next_inherited;
        if same_as_prev {
            if prev_epoch != epoch || prev_scope_gen != scope_gen {
                let resolved = &mut self.record_mut(id).resolved;
                resolved.1 = epoch;
                resolved.2 = scope_gen;
            }
            work.record_skipped();
            let visual_needs_repaint = self.theme_change_requires_visual_repaint(id, theme_change);
            if visual_needs_repaint {
                work.resolved_style_changed += 1;
                work.record_paint_invalidation(1);
                self.mark(id, DirtyMask::RENDER);
                if self.node_painter(id).is_some() {
                    self.mark_repaint(id);
                }
            } else {
                work.resolved_style_unchanged += 1;
            }
            return Ok(());
        }
        work.record_resolved();
        work.resolved_style_changed += 1;
        let inherited_arc = if SHARE && *inherited_style.inherited == next_inherited {
            Arc::clone(&inherited_style.inherited)
        } else if SHARE {
            self.invalidation
                .inherited_styles
                .intern(next_inherited, work)
        } else {
            work.record_allocation(1, size_of::<InheritedStyle>());
            Arc::new(next_inherited)
        };
        let next = ComputedStyle {
            inherited: inherited_arc,
            paint_colors,
            background,
            border_color,
            ring_color,
        };
        // Identical inherited results can share immutable paint state. Painted
        // boxes publish an unpainted companion Arc (`resolved.3`) so descendants
        // inside them still share via `Arc::clone`.
        let (next_arc, next_unpainted) = if !next.has_local_paint() {
            let shared = if SHARE {
                parent
                    .map(|parent| &self.record(parent).resolved.3)
                    .filter(|inherited| inherited.as_ref() == &next)
                    .map(Arc::clone)
            } else {
                None
            };
            let arc = shared.unwrap_or_else(|| {
                if SHARE {
                    self.invalidation.computed_styles.intern(next, work)
                } else {
                    work.record_allocation(1, size_of::<ComputedStyle>());
                    Arc::new(next)
                }
            });
            (Arc::clone(&arc), arc)
        } else {
            let arc = if SHARE {
                self.invalidation.computed_styles.intern(next, work)
            } else {
                work.record_allocation(1, size_of::<ComputedStyle>());
                Arc::new(next)
            };
            let unpainted = if SHARE {
                let clean = arc.without_local_paint();
                parent
                    .map(|parent| &self.record(parent).resolved.3)
                    .filter(|parent_unpainted| parent_unpainted.as_ref() == &clean)
                    .map(Arc::clone)
                    .unwrap_or_else(|| self.invalidation.computed_styles.intern(clean, work))
            } else {
                Arc::clone(&arc)
            };
            (arc, unpainted)
        };
        self.invalidate_text_for_style(id, &next_arc, theme_triggered, was_uninitialized, work);
        if !prev_computed.visible && next_arc.visible {
            // Text passes skip hidden nodes without resolving them, so a node
            // whose box changed while hidden comes back stale. The scheduled
            // text pass of this same frame picks it up; marking it dirty here,
            // after the drain, would cost the frame another pass.
            self.text.text_shown.push(id);
        }
        if hit_relevant_style_changed(was_uninitialized, &prev_computed, &next_arc) {
            self.invalidation.style_hit_changed = true;
        }
        if (!was_uninitialized && theme_triggered)
            || self.theme_change_requires_visual_repaint(id, theme_change)
        {
            self.mark(id, DirtyMask::RENDER);
            work.record_paint_invalidation(1);
            if self.node_painter(id).is_some() {
                self.mark_repaint(id);
            }
        }
        let inherited_to_children_changed = !was_uninitialized
            && !Arc::ptr_eq(&prev_unpainted, &next_unpainted)
            && !computed_style_eq(&prev_unpainted, &next_unpainted);
        self.record_mut(id).resolved = ResolvedStyle(next_arc, epoch, scope_gen, next_unpainted);
        if inherited_to_children_changed {
            let children = Arc::clone(&self.record(id).hierarchy.children);
            for &child in children.iter() {
                if theme_triggered {
                    propagated_theme.insert(child);
                }
                queue.push_back(child);
            }
        }
        Ok(())
    }
}

impl UiWorld {
    /// Resolve a node's design intent into the layout box the pipeline reads.
    ///
    /// Returns the resolved layout and whether producing it had to copy the
    /// authored one. A node whose intent already matches what it authored —
    /// and every node with no intent at all — keeps sharing the same `Arc`.
    pub(crate) fn resolve_layout_intent(
        style: &NodeStyle,
        tokens: IntentTokens,
    ) -> (Arc<nana_ui_core::LayoutStyle>, bool) {
        let IntentTokens {
            metrics,
            typography,
            spacing,
            shape,
        } = tokens;
        let font_size = style.type_role.map(|role| typography.size(role));
        let font_weight = style.type_weight.map(|weight| typography.weight(weight));
        let gap = style
            .gap_step
            .map(|step| nana_ui_core::LengthSpec::Px(spacing.resolve(step)));
        let padding = style.padding_step.map(|[x, y]| {
            (
                nana_ui_core::LengthSpec::Px(spacing.resolve(x)),
                nana_ui_core::LengthSpec::Px(spacing.resolve(y)),
            )
        });
        // A step the node names wins over the one its component's shape
        // role takes from the installed recipe.
        let radius = style
            .radius
            .or_else(|| style.shape.map(|role| shape.tier(role)))
            .map(|tier| tier.resolve(metrics));
        let corners = style.corner_radii.map(|tiers| {
            tiers.map(|tier| {
                nana_ui_core::LengthSpec::Px(tier.map_or(0.0, |tier| tier.resolve(metrics)))
            })
        });
        let control = style.control_height.map(|height| {
            (
                matches!(height, nana_ui_core::ControlHeight::Exact(_)),
                nana_ui_core::LengthSpec::Px(height.resolve(metrics)),
            )
        });
        let padding_x = style
            .control_padding_x
            .map(|padding| nana_ui_core::LengthSpec::Px(padding.resolve(metrics)));
        let padding_y = style
            .control_padding_y
            .map(|padding| nana_ui_core::LengthSpec::Px(padding.resolve(metrics)));
        let surface = style.surface_padding.map(|padding| {
            (
                nana_ui_core::LengthSpec::Px(padding.resolve_x(metrics)),
                padding.resolve_y(metrics).map(nana_ui_core::LengthSpec::Px),
            )
        });
        // A square is the box a control has when nothing else says. An axis
        // whose length the author wrote is the author's: holding it open at
        // the square's minimum turned a 22px icon button back into a 28px
        // one (a CSS box keeps its size over `Select`'s and `Switch`'s size
        // steps the same way).
        let square = style
            .square
            .map(|size| nana_ui_core::LengthSpec::Px(size.resolve(metrics)));
        let authored = |extent: Option<nana_ui_core::LengthSpec>| {
            extent.is_some_and(nana_ui_core::LengthSpec::is_definite_declared)
        };
        let square_width = square.filter(|_| !authored(style.layout.width));
        let square_height = square.filter(|_| !authored(style.layout.height));
        // CSS `aspect-ratio` fills an automatic height from a definite width;
        // it does not shrink `width:auto` from a definite height. A control
        // that names Exact height and a ratio still has to resolve the inline
        // size here, or a spent `width` from the last project would stick.
        let aspect_width = style.control_height.and_then(|height| {
            if !matches!(height, nana_ui_core::ControlHeight::Exact(_))
                || style.layout.width.is_some()
            {
                return None;
            }
            let ratio = style.layout.aspect_ratio?;
            if !ratio.is_finite() || ratio <= 0.0 {
                return None;
            }
            Some(nana_ui_core::LengthSpec::Px(
                height.resolve(metrics) * ratio,
            ))
        });
        let radius_settled = radius.is_none_or(|value| style.layout.border_radius == Some(value));
        let corners_settled =
            corners.is_none_or(|value| style.layout.paint.border_radii == Some(value));
        let padding_x_settled = padding_x.is_none_or(|length| {
            style.layout.padding_left == Some(length) && style.layout.padding_right == Some(length)
        });
        let padding_y_settled = padding_y.is_none_or(|length| {
            style.layout.padding_top == Some(length) && style.layout.padding_bottom == Some(length)
        });
        let surface_settled = surface.is_none_or(|(x, y)| {
            let horizontal = style.layout.padding.is_some()
                || (style.layout.padding_left == Some(x) && style.layout.padding_right == Some(x));
            let vertical = y.is_none_or(|y| {
                style.layout.padding.is_some()
                    || (style.layout.padding_top == Some(y)
                        && style.layout.padding_bottom == Some(y))
            });
            horizontal && vertical
        });
        let square_settled = square_width
            .is_none_or(|length| style.layout.min_width == Some(length))
            && square_height.is_none_or(|length| style.layout.min_height == Some(length));
        let control_settled = control.is_none_or(|(exact, length)| {
            if exact {
                style.layout.height == Some(length)
            } else {
                style.layout.min_height == Some(length)
            }
        });
        let aspect_width_settled =
            aspect_width.is_none_or(|length| style.layout.width == Some(length));
        let type_settled = font_size.is_none_or(|size| style.layout.font_size == Some(size))
            && font_weight.is_none_or(|weight| style.layout.font_weight == Some(weight));
        let gap_settled = gap.is_none_or(|length| style.layout.gap == Some(length));
        let padding_settled = padding.is_none_or(|(x, y)| {
            style.layout.padding_left == Some(x)
                && style.layout.padding_right == Some(x)
                && style.layout.padding_top == Some(y)
                && style.layout.padding_bottom == Some(y)
        });
        if radius_settled
            && corners_settled
            && control_settled
            && padding_x_settled
            && padding_y_settled
            && surface_settled
            && square_settled
            && aspect_width_settled
            && type_settled
            && gap_settled
            && padding_settled
        {
            return (Arc::clone(&style.layout), false);
        }
        let mut layout = Arc::clone(&style.layout);
        let target = Arc::make_mut(&mut layout);
        if let Some(value) = radius {
            target.border_radius = Some(value);
        }
        if let Some(value) = corners {
            target.paint.border_radii = Some(value);
        }
        if let Some((exact, length)) = control {
            if exact {
                target.height = Some(length);
            } else {
                target.min_height = Some(length);
            }
        }
        if let Some(length) = padding_x {
            target.padding_left = Some(length);
            target.padding_right = Some(length);
        }
        if let Some(length) = padding_y {
            target.padding_top = Some(length);
            target.padding_bottom = Some(length);
        }
        if let Some((x, y)) = surface
            && target.padding.is_none()
        {
            target.padding_left.get_or_insert(x);
            target.padding_right.get_or_insert(x);
            if let Some(y) = y {
                target.padding_top.get_or_insert(y);
                target.padding_bottom.get_or_insert(y);
            }
        }
        if let Some(length) = square_width {
            target.min_width = Some(length);
        }
        if let Some(length) = square_height {
            target.min_height = Some(length);
        }
        if let Some(length) = aspect_width {
            target.width = Some(length);
        }
        if let Some(size) = font_size {
            target.font_size = Some(size);
        }
        if let Some(weight) = font_weight {
            target.font_weight = Some(weight);
        }
        if let Some(length) = gap {
            target.gap = Some(length);
        }
        if let Some((x, y)) = padding {
            target.padding = None;
            target.padding_left = Some(x);
            target.padding_right = Some(x);
            target.padding_top = Some(y);
            target.padding_bottom = Some(y);
        }
        (layout, true)
    }

    /// Replace `layout` with an equal one written recently, if any.
    pub(crate) fn share_layout(&mut self, layout: &mut Arc<nana_ui_core::LayoutStyle>) {
        self.invalidation.layouts.intern(layout);
    }

    /// Replace `style` with an equal one recently written, or remember it:
    /// a component that holds its style as an `Arc` and the record written
    /// from it then hold one between them.
    pub(crate) fn share_node_style(&mut self, style: &mut Arc<NodeStyle>) {
        self.invalidation.node_styles.intern(style);
    }

    /// Write a node's authored style and keep its effective style and
    /// resolved layout in step.
    ///
    /// They have to move together: projection diffs against the authored
    /// style, while layout, paint and extraction read the effective one --
    /// the authored style with its responsive variant written over it
    /// (Issue #265) -- and its resolved layout. Every path that writes
    /// `record.style` goes through here so they cannot drift.
    pub(crate) fn write_node_style(&mut self, id: StableNodeId, mut style: NodeStyle) {
        let current = Arc::clone(&self.authored_style(id).layout);
        if !Arc::ptr_eq(&current, &style.layout)
            && self
                .invalidation
                .layout_pairs
                .equal(&current, &style.layout)
        {
            style.layout = current;
        } else {
            self.invalidation.layouts.intern(&mut style.layout);
        }
        let style = match self.responsive_variant(id) {
            Some(variant) => {
                let mut effective = style.clone();
                variant.apply(Arc::make_mut(&mut effective.layout));
                self.invalidation.layouts.intern(&mut effective.layout);
                self.overlays_a11y.responsive.keep_authored(id, style);
                effective
            }
            None => {
                self.overlays_a11y.responsive.drop_authored(id);
                style
            }
        };
        let resolved = self.resolve_node_layout(id, &style);
        let depends_on_viewport = resolved.depends_on_viewport();
        if style_declares_intent(&style) {
            self.invalidation.intent_nodes.insert(id);
        } else {
            self.invalidation.intent_nodes.remove(&id);
        }
        let style = self.invalidation.node_styles.share(style);
        let record = self.record_mut(id);
        record.style = style;
        let previous = std::mem::replace(&mut record.resolved_layout, resolved);
        record.layout_depends_on_viewport = depends_on_viewport;
        let next = Arc::clone(&self.record(id).resolved_layout);
        self.note_layout_anchor_change(id, &previous, &next);
        self.refresh_block_extent_reads(id);
        self.recompute_node_theme_deps(id);
        self.note_layout_source_change();
    }

    /// The layout the pipeline reads for `id` with effective style `style`,
    /// and whether it is a copy: its design intent resolved against the
    /// installed metrics, then a locale's direction. A node with neither
    /// keeps sharing its effective `Arc`.
    fn node_layout(
        &self,
        id: StableNodeId,
        style: &NodeStyle,
    ) -> (Arc<nana_ui_core::LayoutStyle>, bool) {
        let (mut resolved, mut copied) =
            Self::resolve_layout_intent(style, self.intent_tokens_for(id));
        if !style.layout.theme.is_empty() {
            let before = Arc::clone(&resolved);
            style
                .layout
                .theme
                .apply(Arc::make_mut(&mut resolved), self.node_theme(id));
            if *before == *resolved {
                resolved = before;
            } else {
                copied = true;
            }
        }
        // A locale's direction, where the node names none (Issue #269).
        if resolved.dir.is_none()
            && let Some(direction) = self.locale_direction(id)
        {
            Arc::make_mut(&mut resolved).dir = Some(direction);
            copied = true;
        }
        (resolved, copied)
    }

    /// [`Self::node_layout`], shared with an equal layout written recently.
    pub(super) fn resolve_node_layout(
        &mut self,
        id: StableNodeId,
        style: &NodeStyle,
    ) -> Arc<nana_ui_core::LayoutStyle> {
        let (mut resolved, copied) = self.node_layout(id, style);
        if copied {
            self.record_resolved_layout_copy();
            self.invalidation.layouts.intern(&mut resolved);
        }
        resolved
    }

    /// Re-resolve the layout intent of every node that declares some, after
    /// a metrics install, and seed each live one whose resolved layout moved
    /// by what moved: a padding step moves the boxes that use it and no
    /// other. Returns how many moved.
    pub(super) fn reresolve_layout_intent(&mut self) -> usize {
        let mut ids: Vec<StableNodeId> = self.invalidation.intent_nodes.iter().copied().collect();
        ids.sort_unstable();
        self.reresolve_layout_intent_ids(ids)
    }

    pub(super) fn reresolve_layout_intent_for(&mut self, candidates: &[StableNodeId]) -> usize {
        if self.invalidation.intent_nodes.is_empty() || candidates.is_empty() {
            return 0;
        }
        let mut ids: Vec<StableNodeId> = candidates
            .iter()
            .copied()
            .filter(|id| self.invalidation.intent_nodes.contains(id))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        self.reresolve_layout_intent_ids(ids)
    }

    fn reresolve_layout_intent_ids(&mut self, ids: Vec<StableNodeId>) -> usize {
        let mut moved = 0;
        for id in ids {
            let (mut resolved, copied) = self.node_layout(id, &self.record(id).style);
            let previous = Arc::clone(&self.record(id).resolved_layout);
            let changed = if Arc::ptr_eq(&previous, &resolved) {
                nana_ui_core::LayoutStyleChange::NONE
            } else {
                previous.changed_fields(resolved.as_ref())
            };
            if changed.is_empty() {
                continue;
            }
            if copied {
                self.record_resolved_layout_copy();
                self.invalidation.layouts.intern(&mut resolved);
            }
            self.record_mut(id).resolved_layout = resolved;
            self.note_layout_anchor_change(id, &previous, &self.record(id).resolved_layout.clone());
            self.refresh_block_extent_reads(id);
            self.note_layout_source_change();
            moved += 1;
            if !self.presence_live(id) {
                continue;
            }
            if changed.intersects(nana_ui_core::LayoutStyleChange::FONT) {
                // Style resolution changes the font, but the pre-layout text
                // pass only visits scheduled text. Keep its intrinsic metrics
                // in the same frame as the new typography, including descendants
                // that inherit the font from this node.
                self.mark_subtree(id, DirtyMask::STYLE | DirtyMask::TEXT);
            }
            let classified = self.classify_layout_change(id, changed);
            self.record_layout_invalidation(
                id,
                nana_ui_core::LayoutInvalidation {
                    source: nana_ui_core::LayoutInvalidationSource::Resource,
                    reason: nana_ui_core::InvalidationReason::RESOURCE,
                    ..classified
                },
            );
        }
        moved
    }

    /// Read one role out of `model` (the token authority under a node's
    /// colour scope), counted for Issue #101 §4.
    fn theme_color(&self, model: StyleModelRef, role: SemanticColorRole) -> [f32; 4] {
        self.record_theme_read();
        model.color(role).as_rgba_array()
    }

    /// Resolve a palette mix. One question asked of the theme, so one read —
    /// the roles it blends are the mix's own business.
    fn theme_mix(&self, model: StyleModelRef, mix: nana_ui_core::SemanticColorMix) -> [f32; 4] {
        self.record_theme_read();
        mix.resolve(model).as_rgba_array()
    }

    /// The installed style model under `scope`: what a node in that colour
    /// scope reads every role from. No scope is the installed model itself.
    pub(crate) fn scoped_style_model(
        &self,
        scope: Option<&nana_ui_core::ColorOverrides>,
    ) -> StyleModelRef {
        match scope {
            Some(scope) => self.invalidation.style_model.with_color_overrides(scope),
            None => self.invalidation.style_model,
        }
    }

    pub(super) fn logical_palette_paint_colors(
        &self,
        id: StableNodeId,
        inherited_color: Option<[f32; 4]>,
        model: StyleModelRef,
    ) -> (
        SemanticColorRole,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
    ) {
        let record = self.record(id);
        let local = &record.style;
        let paint = self.semantic_paint(id, local);
        let layout = record.resolved_layout.as_ref();
        let parent = record.hierarchy.parent;
        let inherited_foreground = parent
            .map(|parent| self.record(parent).resolved.0.foreground)
            .unwrap_or(SemanticColorRole::Text);
        let foreground = paint.foreground.unwrap_or(inherited_foreground);
        let color = layout
            .paint_colors
            .color
            .map(|c| c.resolve_srgb(model))
            .or_else(|| paint.foreground_mix.map(|mix| self.theme_mix(model, mix)))
            .or_else(|| {
                paint
                    .foreground
                    .map(|role| self.theme_color(model, self.effective_foreground_role(id, role)))
                    .or(inherited_color)
                    .or_else(|| Some(self.theme_color(model, foreground)))
            });
        let background = layout
            .paint_colors
            .background
            .map(|c| c.resolve_srgb(model))
            .or_else(|| paint.background_mix.map(|mix| self.theme_mix(model, mix)))
            .or_else(|| paint.background.map(|role| self.theme_color(model, role)));
        let border_color = layout
            .paint_colors
            .border
            .map(|c| c.resolve_srgb(model))
            .or_else(|| layout.resolved_border_color())
            .or_else(|| paint.border_mix.map(|mix| self.theme_mix(model, mix)))
            .or_else(|| paint.border.map(|role| self.theme_color(model, role)));
        (foreground, color, background, border_color)
    }

    pub(super) fn palette_paint_colors(
        &self,
        id: StableNodeId,
        inherited_color: Option<[f32; 4]>,
        model: StyleModelRef,
    ) -> (
        SemanticColorRole,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
    ) {
        let (foreground, mut color, mut background, mut border_color) =
            self.logical_palette_paint_colors(id, inherited_color, model);
        if let Some((from, progress)) = self
            .motion
            .hover_transitions
            .get(&id)
            .and_then(|transition| transition.border)
        {
            border_color = interpolate_color(from, border_color, progress);
        }
        if let Some(overlay_fg) =
            self.presentation_color_overlay(id, crate::AnimatableProperty::Color)
        {
            color = Some(overlay_fg);
        }
        if let Some(overlay_bg) =
            self.presentation_color_overlay(id, crate::AnimatableProperty::Background)
        {
            let suppress_transparent = self
                .motion
                .hover_transitions
                .get(&id)
                .is_some_and(|transition| transition.bg_has_none && overlay_bg[3] <= 1e-5);
            background = if suppress_transparent {
                None
            } else {
                Some(overlay_bg)
            };
        }
        (foreground, color, background, border_color)
    }
}

impl UiWorld {
    /// `id`'s [`crate::NodeStyle::ring`] in the installed palette.
    pub(super) fn palette_ring_color(
        &self,
        id: StableNodeId,
        model: StyleModelRef,
    ) -> Option<[f32; 4]> {
        self.record(id)
            .style
            .ring
            .map(|role| self.theme_color(model, role))
    }

    pub(super) fn inherited_palette_color(
        &self,
        mut parent: Option<StableNodeId>,
    ) -> Option<[f32; 4]> {
        while let Some(id) = parent {
            if let Some(overlay) =
                self.presentation_color_overlay(id, crate::AnimatableProperty::Color)
            {
                return Some(overlay);
            }
            let record = self.record(id);
            let local = &record.style;
            let resolved_layout = record.resolved_layout.as_ref();
            let model = self.node_style_model(id);
            if let Some(pc) = resolved_layout
                .paint_colors
                .color
                .or(local.layout.paint_colors.color)
            {
                return Some(pc.resolve_srgb(model));
            }
            let paint = self.semantic_paint(id, local);
            if let Some(mix) = paint.foreground_mix {
                return Some(self.theme_mix(model, mix));
            }
            if let Some(role) = paint.foreground {
                return Some(self.theme_color(model, role));
            }
            parent = record.hierarchy.parent;
        }
        None
    }

    /// A `Muted` child inside an interactive row whose current state names its
    /// own [`crate::SemanticPaint::foreground_secondary`] (such as
    /// [`crate::SemanticPaint::FOCUS_SURFACE`] on keyboard focus) follows that
    /// secondary role until an intermediate box paints a surface or installs
    /// colour overrides of its own.
    fn effective_foreground_role(
        &self,
        id: StableNodeId,
        role: SemanticColorRole,
    ) -> SemanticColorRole {
        if role != SemanticColorRole::Muted {
            return role;
        }
        let own = &self.record(id).style;
        let own_paint = self.semantic_paint(id, own);
        if let Some(secondary) = own_paint.foreground_secondary {
            return secondary;
        }
        if own.color_overrides.is_some()
            || own
                .theme_patch
                .as_ref()
                .is_some_and(|p| p.colors.is_some() || p.accent.is_some() || p.appearance.is_some())
            || own.layout.paint_colors.background.is_some()
            || own_paint.background.is_some()
            || own_paint.background_mix.is_some()
        {
            return role;
        }
        let mut cursor = self.record(id).hierarchy.parent;
        while let Some(anc) = cursor {
            let local = &self.record(anc).style;
            let paint = self.semantic_paint(anc, local);
            if let Some(secondary) = paint.foreground_secondary {
                return secondary;
            }
            if local.color_overrides.is_some()
                || local.theme_patch.as_ref().is_some_and(|p| {
                    p.colors.is_some() || p.accent.is_some() || p.appearance.is_some()
                })
                || local.layout.paint_colors.background.is_some()
                || paint.background.is_some()
                || paint.background_mix.is_some()
            {
                break;
            }
            cursor = self.record(anc).hierarchy.parent;
        }
        role
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
            ..crate::SemanticPaint::default()
        }
        .overlay(local.interaction.base);
        let accessibility = self.nodes.accessibility(id);
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
        // `focus_visible`, not `focused`: a control that was clicked is focused
        // and does not say so, unless it is a text field taking typing.
        if self.focus_visible(self.record(id).document) == Some(id) {
            paint = paint.overlay(local.interaction.focused);
        }
        if accessibility.disabled && !accessibility.busy {
            paint = paint.overlay(local.interaction.disabled);
        }
        paint
    }

    /// The colour a component's quieter text resolves to for `id`, in its
    /// current interaction state.
    ///
    /// Component geometry used to read `palette.muted` straight off the
    /// palette for every detail line, hint, placeholder and unit. That skips
    /// the state the node is actually in: a focused row fills with
    /// `focus_surface` and its label follows to `focus_text`, while the pinned
    /// grey stays resolved against the surface the row no longer has. The
    /// role still defaults to `Muted`, so a component that says nothing paints
    /// exactly what it painted before.
    pub(crate) fn secondary_text_color(&self, id: StableNodeId) -> [f32; 4] {
        let local = &self.record(id).style;
        let role = self
            .semantic_paint(id, local)
            .foreground_secondary
            .unwrap_or(SemanticColorRole::Muted);
        self.theme_color(self.node_style_model(id), role)
    }

    /// The colour a list row's note and detail resolve to: the row's own
    /// `primary` text colour at the theme's
    /// [`nana_ui_core::OpacityTokens::secondary_text`] alpha, one step back
    /// on whatever fill the row has now.
    ///
    /// A state that names its own [`crate::SemanticPaint::foreground_secondary`]
    /// still wins. Focus does: light mode's focus fill is nearly solid and its
    /// label is already the weakest text on it, so the detail keeps the
    /// label's full colour there rather than fading under it.
    pub(crate) fn row_secondary_text_color(&self, id: StableNodeId, primary: [f32; 4]) -> [f32; 4] {
        let local = &self.record(id).style;
        if let Some(role) = self.semantic_paint(id, local).foreground_secondary {
            return self.theme_color(self.node_style_model(id), role);
        }
        self.record_theme_read();
        let [r, g, b, a] = primary;
        [
            r,
            g,
            b,
            a * self.node_style_model(id).opacity.secondary_text,
        ]
    }
}

impl UiWorld {
    /// Re-record a painted node. A painter whose outline is its hit shape
    /// takes its hit index entry with it: the entry holds the recording.
    /// A node gained or lost focus: its focus ring, IME and accessibility
    /// state, and a painter that paints focus.
    pub(super) fn mark_focus_changed(&mut self, id: StableNodeId) {
        self.mark(
            id,
            DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
        );
        if self.node_painter(id).is_some() {
            self.mark_repaint(id);
        }
    }

    pub(super) fn mark_repaint(&mut self, id: StableNodeId) {
        let outline = self
            .node_painter(id)
            .is_some_and(|painter| painter.painter().hit_painted_outline());
        self.mark(
            id,
            if outline {
                DirtyMask::RENDER | DirtyMask::INPUT
            } else {
                DirtyMask::RENDER
            },
        );
    }

    pub(super) fn mark_interaction_style(&mut self, id: StableNodeId) {
        self.mark(id, DirtyMask::STATE);
        let record = self.record(id);
        // A painter reads the state it is in, so it re-records on it.
        let painted = record.style.painter.is_some()
            || (!self.invalidation.painter_overrides.is_empty()
                && self.invalidation.painter_overrides.contains_key(&id));
        let styled =
            !record.style.interaction.is_shared_default() && !record.style.interaction.is_empty();
        if painted {
            self.mark_repaint(id);
        }
        if styled {
            let mut work = ThemeWorkCounters::default();
            work.record_paint_invalidation(1);
            self.record_theme_work(work);
            if self.record(id).hierarchy.children.is_empty() {
                self.mark(id, DirtyMask::STYLE | DirtyMask::RENDER);
            } else {
                self.mark_subtree(id, DirtyMask::STYLE | DirtyMask::RENDER);
            }
        }
    }
}

impl UiWorld {
    /// Resolve inherited visual state for dirty nodes. Parent state is always
    /// resolved before its descendants, independent of stable ID order.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        let mut resolved = HashSet::with_capacity(ids.len());
        let mut queue: std::collections::VecDeque<StableNodeId> = ids.iter().copied().collect();
        let mut propagated_theme = HashSet::new();
        let mut work = ThemeWorkCounters::default();
        let pending_theme_before = self.invalidation.theme_state.pending_theme_eval.len();
        while let Some(id) = queue.pop_front() {
            if !self.contains(id) {
                return Err(UiWorldError::MissingNode(id));
            }
            if resolved.contains(&id) {
                continue;
            }
            let from_parent = propagated_theme.contains(&id);
            self.resolve_style::<true>(
                id,
                from_parent,
                &mut resolved,
                &mut queue,
                &mut propagated_theme,
                &mut work,
            )?;
        }
        if self.invalidation.theme_state.pending_theme_eval.len() < pending_theme_before {
            self.fold_theme_invalidation_resolution(&work);
        }
        self.record_theme_work(work);
        self.reconcile_focus(ids);
        self.drop_invalid_document_text_selections();
        Ok(())
    }
}

impl UiWorld {
    /// Diagnostic control path for paired sharing measurements; product resolution shares.
    #[cfg(feature = "benchmark")]
    pub fn benchmark_resolve_styles_unshared(
        &mut self,
        ids: &[StableNodeId],
    ) -> Result<(), UiWorldError> {
        let mut resolved = HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        let mut propagated_theme = HashSet::new();
        let mut work = ThemeWorkCounters::default();
        for &id in ids {
            self.resolve_style::<false>(
                id,
                false,
                &mut resolved,
                &mut queue,
                &mut propagated_theme,
                &mut work,
            )?;
        }
        self.record_theme_work(work);
        self.reconcile_focus(ids);
        Ok(())
    }

    /// Install `theme` as the design system; what paints is it, or its
    /// high-contrast rendition while the system asks for one.
    pub(super) fn install_theme(&mut self, theme: Arc<nana_ui_core::CompiledTheme>) {
        self.invalidation.installed_theme = theme;
        self.apply_compiled_theme(self.contrast_rendition());
    }

    /// Follow the system high-contrast setting. A presentation overlay: the
    /// installed theme stays as it was and comes back when the setting turns
    /// off. Whether anything changed.
    pub fn set_high_contrast(&mut self, high_contrast: bool) -> bool {
        if self.invalidation.high_contrast == high_contrast {
            return false;
        }
        self.invalidation.high_contrast = high_contrast;
        self.apply_compiled_theme(self.contrast_rendition());
        if !self
            .invalidation
            .theme_state
            .installed_document_themes
            .is_empty()
        {
            let doc_themes: Vec<_> = self
                .invalidation
                .theme_state
                .installed_document_themes
                .iter()
                .map(|(&doc, theme)| (doc, Arc::clone(theme)))
                .collect();
            for (doc, installed) in doc_themes {
                self.set_document_theme(doc, Some(installed));
            }
        }
        true
    }

    /// The installed theme with the high-contrast palette when it is on: the
    /// palette of the same lightness, opaque, and a title bar that matches
    /// its background.
    fn contrast_rendition(&self) -> Arc<nana_ui_core::CompiledTheme> {
        self.contrast_rendition_for(&self.invalidation.installed_theme)
    }

    pub(super) fn apply_compiled_theme(&mut self, next: Arc<nana_ui_core::CompiledTheme>) {
        // Values, not identity. `CompiledTheme::is_same_revision` is the cheap
        // question and it trusts the author's generation bump; an install has
        // to be right even for a theme that forgot to bump.
        if *self.invalidation.theme == *next {
            return;
        }
        let changed =
            nana_ui_core::ThemeCategoryGenerations::diff_themes(&self.invalidation.theme, &next);
        if changed.is_empty() {
            self.invalidation.theme = next;
            return;
        }
        self.invalidation
            .theme_state
            .theme_generations
            .advance(changed);
        if changed == nana_ui_core::ThemeDependencyMask::MOTION_POLICY
            && self.invalidation.theme_state.theme_scopes.is_empty()
            && self.invalidation.theme_state.document_themes.is_empty()
        {
            self.invalidation.theme = next;
            let mut work = ThemeWorkCounters::default();
            work.style_motion_policy_invalidations = 1;
            let identity = self.invalidation.theme.identity();
            self.record_theme_invalidation(
                crate::world::theme_inspect::ThemeInvalidationTrigger::RootTheme {
                    theme_id: identity.id,
                    generation: identity.generation,
                },
                changed,
                &work,
                0,
                true,
            );
            self.record_theme_work(work);
            return;
        }
        if changed != nana_ui_core::ThemeDependencyMask::MOTION_POLICY {
            let has_doc_overrides = !self.invalidation.theme_state.document_themes.is_empty();
            let hover_ids = self
                .motion
                .hover_transitions
                .keys()
                .copied()
                .filter(|&id| {
                    !has_doc_overrides
                        || !self.contains(id)
                        || !self
                            .invalidation
                            .theme_state
                            .document_themes
                            .contains_key(&self.record(id).document)
                })
                .collect::<Vec<_>>();
            for id in hover_ids {
                self.cancel_hover_transition(id);
            }
        }
        if !self.invalidation.theme_state.theme_scopes.is_empty()
            || !self.invalidation.theme_state.document_themes.is_empty()
        {
            let old_root = Arc::clone(&self.invalidation.theme);
            self.invalidation.style_model = next.style_model();
            self.invalidation.theme = Arc::clone(&next);
            if changed != nana_ui_core::ThemeDependencyMask::MOTION_POLICY {
                self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
            }
            self.apply_root_theme_with_scopes(&old_root, &next, changed);
            return;
        }
        let previous_metrics = self.invalidation.style_model.metrics;
        let body_changed = self.invalidation.theme.typography().body != next.typography().body;
        let regular_changed =
            self.invalidation.theme.typography().regular != next.typography().regular;
        let line_ratio_changed = self.invalidation.theme.typography().line
            / self.invalidation.theme.typography().body
            != next.typography().line / next.typography().body;
        let token_layout_changed = self.invalidation.theme.spacing() != next.spacing()
            || self.invalidation.theme.typography() != next.typography()
            || self.invalidation.theme.border() != next.border()
            || self.invalidation.theme.metrics() != next.metrics();
        let previous_scales = (
            self.invalidation.theme.typography(),
            self.invalidation.theme.spacing(),
        );
        let dialog_changed = changed.intersects(nana_ui_core::ThemeDependencyMask::RECIPE_DIALOG)
            || token_layout_changed;
        let toast_changed = changed.intersects(nana_ui_core::ThemeDependencyMask::RECIPE_TOAST);
        let shape_changed = changed.intersects(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE);
        let menu_changed = changed.intersects(nana_ui_core::ThemeDependencyMask::RECIPE_MENU);
        self.invalidation.style_model = next.style_model();
        self.invalidation.theme = next;
        self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);

        let painted = self
            .invalidation
            .paint_recordings
            .get_mut()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in painted {
            if self.contains(id) && self.nodes.theme_dependency(id).intersects(changed) {
                self.mark_repaint(id);
            }
        }
        if changed.intersects(
            nana_ui_core::ThemeDependencyMask::PALETTE
                .union(nana_ui_core::ThemeDependencyMask::ACCENT)
                .union(nana_ui_core::ThemeDependencyMask::EFFECT)
                .union(nana_ui_core::ThemeDependencyMask::RECIPE_DIALOG)
                .union(nana_ui_core::ThemeDependencyMask::METRICS)
                .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE),
        ) {
            let modal_painted = self
                .invalidation
                .modal_surface_recordings
                .get_mut()
                .keys()
                .copied()
                .collect::<Vec<_>>();
            for id in modal_painted {
                self.mark_repaint(id);
            }
        }
        let metrics_changed = !self
            .invalidation
            .style_model
            .metrics
            .same_layout(&previous_metrics)
            || previous_scales
                != (
                    self.invalidation.theme.typography(),
                    self.invalidation.theme.spacing(),
                );
        let seeds_before = self.invalidation.layout_seeds_created;
        let mut ids = Vec::new();
        for roots in self.tree.live_document_roots.values() {
            for &root in roots {
                ids.extend(self.subtree_ids(root));
            }
        }
        let mut work = ThemeWorkCounters::default();
        let mut modal_frames = Vec::new();
        let mut part_readers = Vec::new();
        for &id in &ids {
            let dep = self.nodes.theme_dependency(id);
            if dep.intersects(changed) {
                let hit = dep.intersection(changed);
                work.theme_revision_candidates += 1;
                if hit.intersects(nana_ui_core::ThemeDependencyMask::RECIPE_ANY) {
                    work.component_recipe_nodes_invalidated += 1;
                }
                self.invalidation
                    .theme_state
                    .pending_theme_eval
                    .entry(id)
                    .and_modify(|prev| *prev = prev.union(hit))
                    .or_insert(hit);
                self.mark(id, DirtyMask::STYLE);
            } else {
                work.theme_dependency_skipped += 1;
            }
            if dialog_changed
                && (self
                    .widget_visual_as::<crate::ModalFrameVisual>(id)
                    .is_some()
                    || self
                        .widget_visual_as::<crate::EmptyStateVisual>(id)
                        .is_some())
            {
                modal_frames.push(id);
            }
            if token_layout_changed || toast_changed {
                let widget_visual = self.nodes.widget_visual(id);
                if (token_layout_changed
                    && widget_visual.is_some_and(|w| w.visual_parts_read_theme()))
                    || (toast_changed && widget_visual.is_some_and(|w| w.reads_toast_recipe()))
                {
                    part_readers.push(id);
                }
            }
        }
        if !ids.is_empty() {
            work.record_allocation(1, ids.len().saturating_mul(size_of::<StableNodeId>()));
        }
        let mut dependents = 0;
        let intent_relevant = metrics_changed
            || token_layout_changed
            || shape_changed
            || menu_changed
            || changed.intersects(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY);
        if intent_relevant {
            dependents = self.reresolve_layout_intent();
            if body_changed || regular_changed || line_ratio_changed {
                let default_roots = self
                    .tree
                    .live_document_roots
                    .values()
                    .flat_map(|roots| roots.iter().copied())
                    .filter(|root| {
                        let layout = &self.record(*root).resolved_layout;
                        (body_changed && layout.font_size.is_none())
                            || (regular_changed && layout.font_weight.is_none())
                            || (line_ratio_changed && layout.line_height.is_none())
                    })
                    .collect::<Vec<_>>();
                for root in default_roots {
                    self.mark(root, DirtyMask::STYLE | DirtyMask::TEXT);
                    let classified =
                        self.classify_layout_change(root, nana_ui_core::LayoutStyleChange::FONT);
                    self.record_layout_invalidation(
                        root,
                        nana_ui_core::LayoutInvalidation {
                            source: nana_ui_core::LayoutInvalidationSource::Resource,
                            reason: nana_ui_core::InvalidationReason::RESOURCE,
                            ..classified
                        },
                    );
                    dependents += 1;
                }
            }
            work.record_layout_invalidation(dependents);
        }
        work.record_layout_invalidation(modal_frames.len() + part_readers.len());
        self.record_theme_work(work);
        for id in modal_frames {
            self.nodes
                .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
            self.mark(id, DirtyMask::TEXT);
            self.record_layout_invalidation(
                id,
                nana_ui_core::LayoutInvalidation::new(
                    nana_ui_core::LayoutInvalidationSource::Resource,
                    nana_ui_core::InvalidationReason::RESOURCE,
                    nana_ui_core::InvalidationKind::MEASURE
                        .union(nana_ui_core::InvalidationKind::PLACEMENT),
                    nana_ui_core::LayoutStyleChange::ALL,
                    nana_ui_core::LayoutDependencyFootprint::ALL,
                ),
            );
        }
        for id in part_readers {
            if self.text(id).is_some_and(|text| !text.is_empty()) {
                self.nodes
                    .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
                self.mark(id, DirtyMask::TEXT);
            }
            self.record_layout_invalidation(
                id,
                nana_ui_core::LayoutInvalidation::new(
                    nana_ui_core::LayoutInvalidationSource::Resource,
                    nana_ui_core::InvalidationReason::RESOURCE,
                    nana_ui_core::InvalidationKind::MEASURE
                        .union(nana_ui_core::InvalidationKind::PLACEMENT),
                    nana_ui_core::LayoutStyleChange::INTRINSIC
                        .union(nana_ui_core::LayoutStyleChange::FLOW),
                    nana_ui_core::LayoutDependencyFootprint::intrinsic_container(),
                ),
            );
        }
        let seeds = (self.invalidation.layout_seeds_created - seeds_before) as usize;
        let identity = self.invalidation.theme.identity();
        self.record_theme_invalidation(
            crate::world::theme_inspect::ThemeInvalidationTrigger::RootTheme {
                theme_id: identity.id,
                generation: identity.generation,
            },
            changed,
            &work,
            seeds,
            false,
        );
        let counts = &mut self.invalidation.counters.pending_drain_counts;
        if intent_relevant {
            counts.theme_to_layout_seeds += seeds;
            counts.theme_metric_dependents_invalidated += dependents;
        } else {
            counts.theme_palette_layout_invalidations += seeds;
        }
    }

    pub(crate) fn recompute_node_theme_deps(&mut self, id: StableNodeId) {
        use nana_ui_core::ThemeDependencyMask;
        let Some(record) = self.nodes.get(id) else {
            return;
        };
        let mut mask = ThemeDependencyMask::NONE;
        if record.hierarchy.parent.is_none() {
            if record.style.foreground.is_none()
                && record.style.interaction.base.foreground.is_none()
                && record.style.interaction.base.foreground_mix.is_none()
                && record.resolved_layout.paint_colors.color.is_none()
            {
                mask = mask.union(ThemeDependencyMask::PALETTE);
            }
            if record.resolved_layout.font_size.is_none()
                || record.resolved_layout.line_height.is_none()
            {
                mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_LAYOUT);
            }
            if record.resolved_layout.font_weight.is_none() {
                mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_SHAPE);
            }
        }
        for role in [
            record.style.foreground,
            record.style.background,
            record.style.border,
            record.style.ring,
        ]
        .into_iter()
        .flatten()
        {
            mask = mask.union(ThemeDependencyMask::for_color_role(role));
        }
        if record.style.ring.is_some()
            || record.style.layout.paint.elevation.is_some()
            || record.resolved_layout.paint.elevation.is_some()
            || record.interaction.focusable
        {
            mask = mask.union(ThemeDependencyMask::EFFECT);
        }
        if !record.style.interaction.is_shared_default() {
            let inter = &*record.style.interaction;
            for paint in [
                &inter.base,
                &inter.selected,
                &inter.selected_hovered,
                &inter.selected_pressed,
                &inter.hovered,
                &inter.pressed,
                &inter.focused,
                &inter.disabled,
            ] {
                mask = mask.union(semantic_paint_dependency_mask(paint));
            }
        }
        mask = mask.union(record.style.layout.paint_colors.dependency_mask());
        mask = mask.union(record.resolved_layout.paint_colors.dependency_mask());
        if record.style.layout.has_dynamic_paint() {
            let paint_mask = paint_theme_dependency_mask(&record.style.layout.paint);
            mask = mask.union(if paint_mask.is_empty() {
                ThemeDependencyMask::COLOR_ANY
            } else {
                paint_mask
            });
        }
        if record.resolved_layout.has_dynamic_paint() {
            let paint_mask = paint_theme_dependency_mask(&record.resolved_layout.paint);
            mask = mask.union(if paint_mask.is_empty() {
                ThemeDependencyMask::COLOR_ANY
            } else {
                paint_mask
            });
        }
        mask = mask.union(record.style.layout.theme.dependency_mask());
        if record.style.type_role.is_some() {
            mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_LAYOUT);
        }
        if record.style.type_weight.is_some() {
            mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_SHAPE);
        }
        if record.style.gap_step.is_some()
            || record.style.padding_step.is_some()
            || record.style.radius.is_some()
            || record.style.corner_radii.is_some()
            || record.style.control_height.is_some()
            || record.style.control_padding_x.is_some()
            || record.style.control_padding_y.is_some()
            || record.style.surface_padding.is_some()
            || record.style.square.is_some()
        {
            mask = mask.union(ThemeDependencyMask::METRICS);
        }
        if record.style.shape.is_some() {
            mask =
                mask.union(ThemeDependencyMask::METRICS.union(ThemeDependencyMask::RECIPE_SHAPE));
        }
        if record.style.radius.is_some()
            || record.style.corner_radii.is_some()
            || record.style.shape.is_some()
            || record
                .resolved_layout
                .border_radius
                .is_some_and(|radius| radius > 0.0)
            || record.resolved_layout.paint.border_radii.is_some()
        {
            mask = mask.union(ThemeDependencyMask::EFFECT);
        }
        let has_rich = self.nodes.rich_text(id).is_some()
            || self.nodes.highlight(id).is_some()
            || self.nodes.editor(id).is_some();
        if has_rich {
            mask = mask.union(
                ThemeDependencyMask::COLOR_ANY
                    .union(ThemeDependencyMask::TYPOGRAPHY_ANY)
                    .union(ThemeDependencyMask::INHERITED_TYPOGRAPHY),
            );
        } else if matches!(record.kind.as_ref(), NodeKind::Text)
            || record.has_text
            || !self.nodes.text_content(id).value.is_empty()
        {
            mask = mask.union(ThemeDependencyMask::INHERITED_TYPOGRAPHY);
        }
        if self
            .text
            .document_text_selections
            .get(&record.document)
            .is_some_and(|sel| sel.node == id)
        {
            mask = mask.union(ThemeDependencyMask::ACCENT);
        }
        if let Some(visual) = self.nodes.widget_visual(id) {
            mask = mask.union(visual.theme_dependency_mask());
        }
        if self
            .nodes
            .custom_render(id)
            .is_some_and(|c| c.renderer.as_ref() == crate::CHART_RENDERER)
        {
            mask = mask.union(
                ThemeDependencyMask::CHART
                    .union(ThemeDependencyMask::PALETTE)
                    .union(ThemeDependencyMask::ACCENT)
                    .union(ThemeDependencyMask::TYPOGRAPHY_LAYOUT)
                    .union(ThemeDependencyMask::TYPOGRAPHY_SHAPE),
            );
        }
        if record.style.painter.is_some()
            || (!self.invalidation.painter_overrides.is_empty()
                && self.invalidation.painter_overrides.contains_key(&id))
        {
            mask = mask.union(
                ThemeDependencyMask::CUSTOM_PAINT
                    .union(ThemeDependencyMask::COLOR_ANY)
                    .union(ThemeDependencyMask::METRICS)
                    .union(ThemeDependencyMask::EFFECT),
            );
        }
        self.nodes.set_theme_dependency(id, mask);
    }

    fn theme_change_requires_visual_repaint(
        &self,
        id: StableNodeId,
        theme_change: nana_ui_core::ThemeDependencyMask,
    ) -> bool {
        if theme_change.is_empty() {
            return false;
        }
        if let Some(visual) = self.nodes.widget_visual(id)
            && visual.theme_dependency_mask().intersects(theme_change)
        {
            return true;
        }
        if self.node_painter(id).is_some() {
            return true;
        }
        if theme_change.intersects(
            nana_ui_core::ThemeDependencyMask::CHART
                .union(nana_ui_core::ThemeDependencyMask::PALETTE)
                .union(nana_ui_core::ThemeDependencyMask::ACCENT)
                .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY),
        ) && self
            .nodes
            .custom_render(id)
            .is_some_and(|c| c.renderer.as_ref() == crate::CHART_RENDERER)
        {
            return true;
        }
        let record = self.record(id);
        if record.style.layout.has_dynamic_paint() || record.resolved_layout.has_dynamic_paint() {
            let mut paint_mask = nana_ui_core::ThemeDependencyMask::NONE;
            if record.style.layout.has_dynamic_paint() {
                paint_mask =
                    paint_mask.union(paint_theme_dependency_mask(&record.style.layout.paint));
            }
            if record.resolved_layout.has_dynamic_paint() {
                paint_mask =
                    paint_mask.union(paint_theme_dependency_mask(&record.resolved_layout.paint));
            }
            if paint_mask.is_empty() {
                paint_mask = nana_ui_core::ThemeDependencyMask::COLOR_ANY;
            }
            if paint_mask.intersects(theme_change) {
                return true;
            }
        }
        let doc = record.document;
        if theme_change.intersects(nana_ui_core::ThemeDependencyMask::EFFECT) {
            let rounded = record.style.radius.is_some()
                || record.style.corner_radii.is_some()
                || record.style.shape.is_some()
                || record
                    .resolved_layout
                    .border_radius
                    .is_some_and(|radius| radius > 0.0)
                || record.resolved_layout.paint.border_radii.is_some();
            if rounded || self.focus_visible(doc) == Some(id) {
                return true;
            }
        }
        if theme_change.intersects(nana_ui_core::ThemeDependencyMask::ACCENT)
            && self.overlays_a11y.drop_hover.map(|(h, _)| h) == Some(id)
        {
            return true;
        }
        false
    }
}

fn paint_theme_dependency_mask(
    paint: &nana_ui_core::PaintStyle,
) -> nana_ui_core::ThemeDependencyMask {
    use nana_ui_core::{BackgroundImage, CssGradient, MaskImage, ThemeDependencyMask};

    let mut mask = ThemeDependencyMask::NONE;
    let gradient_mask = |g: &CssGradient| {
        g.stops().iter().fold(ThemeDependencyMask::NONE, |acc, s| {
            acc.union(s.color.dependency_mask())
        })
    };
    for shadow in &paint.box_shadows {
        mask = mask.union(shadow.color.dependency_mask());
    }
    if paint.elevation.is_some() {
        mask = mask.union(ThemeDependencyMask::EFFECT);
    }
    for shadow in &paint.text_shadows {
        mask = mask.union(shadow.color.dependency_mask());
    }
    if let Some(stroke) = paint.text_stroke
        && let Some(color) = stroke.color
    {
        mask = mask.union(color.dependency_mask());
    }
    if let Some(filter) = paint.filter
        && let Some(shadow) = filter.drop_shadow
    {
        mask = mask.union(shadow.color.dependency_mask());
    }
    for layer in &paint.background_layers {
        if let BackgroundImage::Gradient(g) = layer {
            mask = mask.union(gradient_mask(g));
        }
    }
    if let Some(MaskImage::Gradient(g)) = &paint.mask {
        mask = mask.union(gradient_mask(g));
    }
    mask
}

#[derive(Clone, Copy, Default)]
pub(super) struct HoverTransition {
    pub border: Option<(Option<[f32; 4]>, f32)>,
    pub bg_has_none: bool,
    pub inherits_color: bool,
}

fn hover_color_endpoints(
    from: Option<[f32; 4]>,
    to: Option<[f32; 4]>,
) -> Option<([f32; 4], [f32; 4])> {
    if from == to {
        return None;
    }
    match (from, to) {
        (Some(a), Some(b)) => Some((a, b)),
        (Some(a), None) => Some((a, [a[0], a[1], a[2], 0.0])),
        (None, Some(b)) => Some(([b[0], b[1], b[2], 0.0], b)),
        (None, None) => None,
    }
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
        let (_, color, background, border) =
            self.palette_paint_colors(id, inherited, self.node_style_model(id));
        [color, background, border]
    }

    pub(super) fn logical_hover_paint(&self, id: StableNodeId) -> [Option<[f32; 4]>; 3] {
        let inherited = self
            .record(id)
            .hierarchy
            .parent
            .and_then(|parent| self.record(parent).resolved.0.color);
        let (_, color, background, border) =
            self.logical_palette_paint_colors(id, inherited, self.node_style_model(id));
        [color, background, border]
    }

    pub(super) fn has_active_hover_track(&self, target: StableNodeId) -> bool {
        [
            crate::component_animation_kinds::HOVER,
            crate::component_animation_kinds::HOVER_FOREGROUND,
            crate::component_animation_kinds::HOVER_BORDER,
        ]
        .into_iter()
        .any(|kind| {
            crate::component_animation_id(kind, target)
                .is_some_and(|id| self.motion.animations.contains_key(&id))
        })
    }

    fn cancel_hover_kind(&mut self, kind: u64, target: StableNodeId) {
        if let Some(id) = crate::component_animation_id(kind, target) {
            if let Some(animation) = self.motion.animations.remove(&id) {
                self.motion
                    .animation_deadlines
                    .remove(&(animation.next_deadline, id));
            }
            self.unbind_presentation(id.track_id());
        }
    }

    pub(super) fn transition_hover(&mut self, id: StableNodeId, from: [Option<[f32; 4]>; 3]) {
        if self.motion.hover_transitions.contains_key(&id) {
            self.mark_hover_paint(id);
        }
        let to = self.logical_hover_paint(id);
        if from == to {
            self.cancel_hover_transition(id);
            return;
        }
        let node_theme = self.node_theme(id);
        let hover_duration = node_theme.duration(nana_ui_core::MotionRole::HoverColor);
        let hover_easing = node_theme
            .motion()
            .easing(nana_ui_core::EasingRole::Standard);
        let mut active = false;
        if let Some((from_c, to_c)) = hover_color_endpoints(from[0], to[0]) {
            self.start_component_track(
                id,
                crate::component_animation_kinds::HOVER_FOREGROUND,
                hover_duration,
                hover_easing,
                crate::AnimatableProperty::Color,
                crate::MotionValue::Color(from_c),
                crate::MotionValue::Color(to_c),
                crate::MotionInterrupt::Retarget,
                None,
            );
            active = true;
        } else {
            self.cancel_hover_kind(crate::component_animation_kinds::HOVER_FOREGROUND, id);
        }
        if let Some((from_bg, to_bg)) = hover_color_endpoints(from[1], to[1]) {
            self.start_component_track(
                id,
                crate::component_animation_kinds::HOVER,
                hover_duration,
                hover_easing,
                crate::AnimatableProperty::Background,
                crate::MotionValue::Color(from_bg),
                crate::MotionValue::Color(to_bg),
                crate::MotionInterrupt::Retarget,
                None,
            );
            active = true;
        } else {
            self.cancel_hover_kind(crate::component_animation_kinds::HOVER, id);
        }
        let border = if from[2] != to[2] {
            self.start_component_track(
                id,
                crate::component_animation_kinds::HOVER_BORDER,
                hover_duration,
                hover_easing,
                crate::AnimatableProperty::Progress,
                crate::MotionValue::Scalar(0.0),
                crate::MotionValue::Scalar(1.0),
                crate::MotionInterrupt::Retarget,
                None,
            );
            active = true;
            Some((from[2], 0.0))
        } else {
            self.cancel_hover_kind(crate::component_animation_kinds::HOVER_BORDER, id);
            None
        };
        if active {
            self.motion.hover_transitions.insert(
                id,
                HoverTransition {
                    border,
                    bg_has_none: from[1].is_none() || to[1].is_none(),
                    inherits_color: from[0] != to[0],
                },
            );
            self.mark_hover_paint(id);
        } else {
            self.motion.hover_transitions.remove(&id);
        }
    }
}

impl UiWorld {
    pub(super) fn mark_hover_paint(&mut self, target: StableNodeId) {
        let bits = DirtyMask::STYLE | DirtyMask::RENDER;
        if self
            .motion
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
        self.motion.hover_transitions.remove(&target);
        for kind in [
            crate::component_animation_kinds::HOVER,
            crate::component_animation_kinds::HOVER_FOREGROUND,
            crate::component_animation_kinds::HOVER_BORDER,
        ] {
            self.cancel_hover_kind(kind, target);
        }
    }
}

#[cfg(test)]
#[path = "style_sharing_tests.rs"]
mod sharing_tests;

/// How many recent node styles a world keeps to share.
const RECENT_STYLES: usize = 32;

/// Node styles written recently, so nodes whose styles are equal hold one
/// `Arc` instead of a copy each in their record. Only a style with the very
/// same layout `Arc` is compared further (layouts are interned already), so a
/// write that matches none costs a pointer compare per entry.
#[derive(Default)]
pub(crate) struct NodeStyleInterner {
    recent: [Option<Arc<NodeStyle>>; RECENT_STYLES],
    next: usize,
}

impl NodeStyleInterner {
    /// `style` as a record holds it: an equal recent one, or a new `Arc`
    /// remembered.
    pub(crate) fn share(&mut self, style: NodeStyle) -> Arc<NodeStyle> {
        if let Some(kept) = self.find(&style) {
            return kept;
        }
        let style = Arc::new(style);
        self.remember(&style);
        style
    }

    /// [`Self::share`] for a style already in an `Arc`.
    pub(crate) fn intern(&mut self, style: &mut Arc<NodeStyle>) {
        if self
            .recent
            .iter()
            .flatten()
            .any(|kept| Arc::ptr_eq(kept, style))
        {
            return;
        }
        match self.find(style) {
            Some(kept) => *style = kept,
            None => self.remember(style),
        }
    }

    fn find(&self, style: &NodeStyle) -> Option<Arc<NodeStyle>> {
        self.recent
            .iter()
            .flatten()
            .find(|kept| same_style_on_one_layout(kept, style))
            .cloned()
    }

    fn remember(&mut self, style: &Arc<NodeStyle>) {
        self.recent[self.next] = Some(Arc::clone(style));
        self.next = (self.next + 1) % RECENT_STYLES;
    }
}

/// Whether `kept` and `style` are equal, given that they hold the very same
/// layout `Arc`: the layout is not compared again (`Arc<LayoutStyle>`'s `==`
/// compares what it points to), the rest field by field. The pattern names
/// every field, so a field added to [`NodeStyle`] has to be added here.
fn same_style_on_one_layout(kept: &NodeStyle, style: &NodeStyle) -> bool {
    let NodeStyle {
        layout,
        foreground,
        background,
        border,
        ring,
        radius,
        corner_radii,
        shape,
        control_height,
        control_padding_x,
        control_padding_y,
        surface_padding,
        square,
        type_role,
        type_weight,
        gap_step,
        padding_step,
        interaction,
        text_horizontal_alignment,
        text_vertical_alignment,
        painter,
        color_overrides,
        theme_patch,
    } = kept;
    Arc::ptr_eq(layout, &style.layout)
        && *foreground == style.foreground
        && *background == style.background
        && *border == style.border
        && *ring == style.ring
        && *radius == style.radius
        && *corner_radii == style.corner_radii
        && *shape == style.shape
        && *control_height == style.control_height
        && *control_padding_x == style.control_padding_x
        && *control_padding_y == style.control_padding_y
        && *surface_padding == style.surface_padding
        && *square == style.square
        && *type_role == style.type_role
        && *type_weight == style.type_weight
        && *gap_step == style.gap_step
        && *padding_step == style.padding_step
        && *interaction == style.interaction
        && *text_horizontal_alignment == style.text_horizontal_alignment
        && *text_vertical_alignment == style.text_vertical_alignment
        && *painter == style.painter
        && *color_overrides == style.color_overrides
        && *theme_patch == style.theme_patch
}

/// How many recent layouts a world keeps to share. Siblings built alike
/// (list rows, toolbars) repeat within a few writes.
const RECENT_LAYOUTS: usize = 32;

/// Layouts written recently, so nodes and resolutions whose layouts are
/// equal share one allocation instead of one each. Comparing is cheap: the
/// large groups of a layout are shared and compare by pointer first.
#[derive(Default)]
pub(crate) struct LayoutInterner {
    recent: [Option<Arc<nana_ui_core::LayoutStyle>>; RECENT_LAYOUTS],
    next: usize,
}

impl LayoutInterner {
    /// Replace `layout` with an equal recent one, or remember it.
    pub(crate) fn intern(&mut self, layout: &mut Arc<nana_ui_core::LayoutStyle>) {
        let recent = self.recent.iter().flatten();
        if recent.clone().any(|kept| Arc::ptr_eq(kept, layout)) {
            return;
        }
        if let Some(kept) = recent.clone().find(|kept| ***kept == **layout) {
            *layout = Arc::clone(kept);
            return;
        }
        self.recent[self.next] = Some(Arc::clone(layout));
        self.next = (self.next + 1) % RECENT_LAYOUTS;
    }
}

const RECENT_INHERITED_STYLES: usize = 16;

/// Recently resolved inherited styles, so siblings that diverge from a shared
/// parent share one `Arc<InheritedStyle>` instead of allocating one each.
#[derive(Default)]
pub(crate) struct InheritedStyleInterner {
    recent: [Option<Arc<InheritedStyle>>; RECENT_INHERITED_STYLES],
    next: usize,
}

impl InheritedStyleInterner {
    pub(crate) fn intern(
        &mut self,
        value: InheritedStyle,
        work: &mut ThemeWorkCounters,
    ) -> Arc<InheritedStyle> {
        if let Some(kept) = self.recent.iter().flatten().find(|kept| ***kept == value) {
            return Arc::clone(kept);
        }
        work.record_allocation(1, size_of::<InheritedStyle>());
        let arc = Arc::new(value);
        self.recent[self.next] = Some(Arc::clone(&arc));
        self.next = (self.next + 1) % RECENT_INHERITED_STYLES;
        arc
    }
}

const RECENT_COMPUTED_STYLES: usize = 32;

/// Recently resolved computed styles, so painted controls (`Button`,
/// `ListItem`, badges) share one `Arc<ComputedStyle>` instead of allocating
/// per node.
#[derive(Default)]
pub(crate) struct ComputedStyleInterner {
    recent: [Option<Arc<ComputedStyle>>; RECENT_COMPUTED_STYLES],
    next: usize,
}

impl ComputedStyleInterner {
    pub(crate) fn intern(
        &mut self,
        value: ComputedStyle,
        work: &mut ThemeWorkCounters,
    ) -> Arc<ComputedStyle> {
        if let Some(kept) = self.recent.iter().flatten().find(|kept| {
            (Arc::ptr_eq(&kept.inherited, &value.inherited) || *kept.inherited == *value.inherited)
                && kept.background == value.background
                && kept.border_color == value.border_color
                && kept.ring_color == value.ring_color
                && kept.paint_colors == value.paint_colors
        }) {
            return Arc::clone(kept);
        }
        work.record_allocation(1, size_of::<ComputedStyle>());
        let arc = Arc::new(value);
        self.recent[self.next] = Some(Arc::clone(&arc));
        self.next = (self.next + 1) % RECENT_COMPUTED_STYLES;
        arc
    }
}

impl UiWorld {
    /// The language a root inherits: the application's, else the text
    /// engine's fallback.
    fn root_language(&self) -> Option<nana_text::font::LanguageTag> {
        self.text
            .default_language
            .clone()
            .or_else(|| self.text.engine_fallback_language.clone())
    }

    /// Set the application's language, which every node without a language
    /// of its own or above it inherits. Text whose language moves shapes
    /// again; the rest only resolves its style again.
    pub fn set_default_language(&mut self, language: Option<nana_text::font::LanguageTag>) {
        let before = self.root_language();
        self.text.default_language = language;
        if self.root_language() != before {
            self.mark_root_language_dependents();
        }
    }

    /// The language `id` names for itself, if any; see
    /// [`crate::MutationQueue::set_language`].
    pub fn node_language(&self, id: StableNodeId) -> Option<&nana_text::font::LanguageTag> {
        self.nodes.language(id)
    }

    /// The text engine's fallback language moved. It only reaches text when
    /// the application names no language of its own.
    pub(crate) fn observe_engine_fallback_language(
        &mut self,
        language: Option<nana_text::font::LanguageTag>,
    ) {
        if self.text.engine_fallback_language == language {
            return;
        }
        let before = self.root_language();
        self.text.engine_fallback_language = language;
        if self.root_language() != before {
            self.mark_root_language_dependents();
        }
    }

    /// `id`'s computed style is going from the one it holds to `next`: what
    /// that costs its text, by the class of change.
    fn invalidate_text_for_style(
        &mut self,
        id: StableNodeId,
        next: &ComputedStyle,
        theme_triggered: bool,
        was_uninitialized: bool,
        work: &mut ThemeWorkCounters,
    ) {
        let previous = Arc::clone(&self.record(id).resolved.0);
        let dirty = crate::text_node::classify_computed_style_change(&previous, next);
        let text_work = dirty.work();
        let is_text_carrier = self.widget_visual_ref(id).is_some()
            || self.record(id).has_text
            || matches!(self.record(id).kind.as_ref(), NodeKind::Text);
        if is_text_carrier && !was_uninitialized {
            if text_work.intersects(crate::text_node::TextWork::SHAPE) {
                work.style_text_shape_invalidations += 1;
            }
            if dirty.intersects(crate::text_node::TextDirty::CONSTRAINT) {
                work.style_text_layout_invalidations += 1;
            }
        }
        if is_text_carrier
            && (text_work.intersects(crate::text_node::TextWork::SHAPE)
                || text_work.intersects(crate::text_node::TextWork::LAYOUT))
        {
            work.record_text_invalidation(1);
            if theme_triggered && !was_uninitialized && self.shows_text(id) {
                self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
        }
        self.note_language_invalidation(id, dirty);
        self.note_text_scale_change(id, previous.text_scale, next.text_scale);
        self.nodes.invalidate_text(id, dirty);
    }

    /// Text whose computed language moved: a language change reached it.
    /// It shapes again, and the scene takes its new layout and language.
    fn note_language_invalidation(&mut self, id: StableNodeId, dirty: crate::text_node::TextDirty) {
        if dirty.intersects(crate::text_node::TextDirty::LANGUAGE) && self.shows_text(id) {
            self.bump_last_counters(|counters| counters.record_text_language(1, 0));
            self.mark(id, DirtyMask::RENDER);
        }
    }

    /// The typography scale a node with no parent starts from: its window's,
    /// else the application's.
    fn root_text_scale(&self, document: DocumentId) -> f32 {
        self.text
            .document_text_scales
            .get(&document)
            .copied()
            .unwrap_or(self.text.default_text_scale)
    }

    /// Set the application's typography scale, which text inherits unless a
    /// window or a scope above it sets its own: an accessibility text size or
    /// an application's content size level. Only the windows that inherit it
    /// are visited, and of them only the scopes that do; see
    /// [`crate::MutationQueue::set_text_scale`]. A scale that is not a
    /// positive finite number is ignored.
    pub fn set_default_text_scale(&mut self, scale: f32) {
        if !valid_text_scale(scale) {
            return;
        }
        if self.text.default_text_scale == scale {
            self.note_equivalent_text_scale();
            return;
        }
        self.text.default_text_scale = scale;
        let mut documents: Vec<DocumentId> = self
            .tree
            .live_document_roots
            .keys()
            .copied()
            .filter(|document| !self.text.document_text_scales.contains_key(document))
            .collect();
        documents.sort_unstable();
        for document in documents {
            self.mark_document_text_scale(document);
        }
    }

    /// Set one window's typography scale over the application's; `None`
    /// follows the application's again. Other windows are not visited. A
    /// scale that is not a positive finite number is ignored.
    pub fn set_document_text_scale(&mut self, document: DocumentId, scale: Option<f32>) {
        if scale.is_some_and(|scale| !valid_text_scale(scale)) {
            return;
        }
        let before = self.root_text_scale(document);
        match scale {
            Some(scale) => {
                self.text.document_text_scales.insert(document, scale);
            }
            None => {
                self.text.document_text_scales.remove(&document);
            }
        }
        if self.root_text_scale(document) == before {
            self.note_equivalent_text_scale();
            return;
        }
        self.mark_document_text_scale(document);
    }

    /// The typography scale `id` sets for its subtree, if it sets one.
    pub fn node_text_scale(&self, id: StableNodeId) -> Option<f32> {
        self.nodes.text_scale(id).copied()
    }

    /// A window's scale moved: visit the scope of each of its roots that
    /// does not set its own.
    fn mark_document_text_scale(&mut self, document: DocumentId) {
        for root in self.document_roots(document) {
            if self.nodes.text_scale(root).is_none() {
                self.mark_text_scale_scope(root);
            }
        }
    }

    /// Visit the scope of a typography scale `root` sets or inherits: `root`
    /// and every node under it that inherits it, stopping at the nodes that
    /// set their own, whose subtrees keep their scale. Their styles resolve
    /// again and their text is considered again; text whose size moved is
    /// laid out again, and layout hears of it only from metrics that moved.
    pub(super) fn mark_text_scale_scope(&mut self, root: StableNodeId) {
        let mut stack = vec![root];
        let mut scanned = 0usize;
        while let Some(id) = stack.pop() {
            scanned += 1;
            let record = self.record(id);
            stack.extend(
                record
                    .hierarchy
                    .children
                    .iter()
                    .rev()
                    .copied()
                    .filter(|child| self.nodes.text_scale(*child).is_none()),
            );
            let _ = self.mark(id, DirtyMask::STYLE | DirtyMask::TEXT);
        }
        self.invalidation
            .counters
            .pending_drain_counts
            .typography_scale_scope_nodes_scanned += scanned;
    }

    /// A typography scale was set to the one it already was: nothing visited.
    pub(super) fn note_equivalent_text_scale(&mut self) {
        self.invalidation
            .counters
            .pending_drain_counts
            .typography_scale_equivalent_skips += 1;
    }

    /// `id`'s computed typography scale is going from `previous` to `next`.
    /// What it paints from its font size paints again; text of it that had
    /// resolved counts as reached, and its next layout as the scale's.
    fn note_text_scale_change(&mut self, id: StableNodeId, previous: f32, next: f32) {
        if previous == next {
            return;
        }
        if !self.shows_text(id) && self.widget_visual_ref(id).is_none() {
            return;
        }
        let _ = self.mark(id, DirtyMask::RENDER);
        if self.nodes.note_text_scale(id) {
            self.bump_last_counters(|counters| counters.record_typography_scale_text(1, 0, 0));
        }
    }

    /// Every live root inherits the root language: resolve the styles under
    /// them again and consider their text. Text that names its own language
    /// keeps its computed style, and so its shape.
    fn mark_root_language_dependents(&mut self) {
        let roots: Vec<StableNodeId> = self
            .tree
            .live_document_roots
            .values()
            .flat_map(|roots| roots.iter().copied())
            .collect();
        for root in roots {
            self.mark_subtree(root, DirtyMask::STYLE | DirtyMask::TEXT);
        }
    }
}

impl UiWorld {
    /// A node with text of its own to shape: a text node, or an element
    /// that holds text.
    fn shows_text(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|record| record.has_text || matches!(record.kind.as_ref(), NodeKind::Text))
    }
}

/// `spec` at typography scale `scale`: an absolute line height scales with the
/// font; a relative one already follows the scaled font size.
fn scaled_line_height(
    spec: nana_ui_core::LineHeightSpec,
    scale: f32,
) -> nana_ui_core::LineHeightSpec {
    match spec {
        nana_ui_core::LineHeightSpec::Absolute(value) => {
            nana_ui_core::LineHeightSpec::Absolute(value * scale)
        }
        relative @ nana_ui_core::LineHeightSpec::Relative(_) => relative,
    }
}

/// Whether `style` declares design intent the installed metrics resolve: a
/// radius tier, a control height or padding step, a surface inset, a square.
/// What design intent resolves against: the installed metrics, type scale,
/// spacing scale and shape recipe.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IntentTokens {
    pub(crate) metrics: nana_ui_core::ThemeMetrics,
    pub(crate) typography: nana_ui_core::TypographyTokens,
    pub(crate) spacing: nana_ui_core::SpacingTokens,
    pub(crate) shape: nana_ui_core::ShapeRecipe,
}

impl UiWorld {
    pub(crate) fn intent_tokens_for(&self, id: StableNodeId) -> IntentTokens {
        let theme = self.node_theme(id);
        IntentTokens {
            metrics: theme.metrics(),
            typography: theme.typography(),
            spacing: theme.spacing(),
            shape: theme.recipes().shape(),
        }
    }
}

fn semantic_paint_dependency_mask(
    paint: &crate::SemanticPaint,
) -> nana_ui_core::ThemeDependencyMask {
    let mut mask = nana_ui_core::ThemeDependencyMask::NONE;
    for role in [
        paint.foreground,
        paint.foreground_secondary,
        paint.background,
        paint.border,
    ]
    .into_iter()
    .flatten()
    {
        mask = mask.union(nana_ui_core::ThemeDependencyMask::for_color_role(role));
    }
    for mix in [paint.foreground_mix, paint.background_mix, paint.border_mix]
        .into_iter()
        .flatten()
    {
        mask = mask.union(nana_ui_core::ThemeDependencyMask::for_color_mix(mix));
    }
    mask
}

fn computed_style_eq(a: &ComputedStyle, b: &ComputedStyle) -> bool {
    a.background == b.background
        && a.border_color == b.border_color
        && a.ring_color == b.ring_color
        && a.paint_colors == b.paint_colors
        && (Arc::ptr_eq(&a.inherited, &b.inherited) || *a.inherited == *b.inherited)
}

fn hit_relevant_style_changed(
    was_uninitialized: bool,
    previous: &ComputedStyle,
    next: &ComputedStyle,
) -> bool {
    was_uninitialized
        || previous.visible != next.visible
        || previous.box_visible != next.box_visible
        || previous.pointer_events != next.pointer_events
        || previous.inert != next.inert
        || (previous.opacity <= 0.0) != (next.opacity <= 0.0)
        || previous.writing_mode != next.writing_mode
        || previous.direction != next.direction
        || previous.text_orientation != next.text_orientation
}

fn style_declares_intent(style: &NodeStyle) -> bool {
    !style.layout.theme.is_empty()
        || style.type_role.is_some()
        || style.type_weight.is_some()
        || style.gap_step.is_some()
        || style.padding_step.is_some()
        || style.radius.is_some()
        || style.corner_radii.is_some()
        || style.shape.is_some()
        || style.control_height.is_some()
        || style.control_padding_x.is_some()
        || style.control_padding_y.is_some()
        || style.surface_padding.is_some()
        || style.square.is_some()
}
