//! Style resolution and semantic palette inheritance.

use super::*;

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
        work: &mut ThemeWorkCounters,
    ) -> Result<bool, UiWorldError> {
        let Some(parent) = parent else {
            return Ok(false);
        };
        if !self.layout_length_tracks.is_empty()
            || !self.overlay_host_nodes.is_empty()
            || self.nodes.has_visuals()
            || !self.presentation.is_empty()
            || self.hover_transitions.contains_key(&id)
        {
            return Ok(false);
        }
        let parent_style = {
            let parent_record = self.record(parent);
            if parent_record.resolved.1 != self.palette_epoch {
                return Ok(false);
            }
            Arc::clone(&parent_record.resolved.0)
        };
        // Sharing hands the child every field of the parent's style. That is
        // the child's own answer only while the parent sets none of the
        // non-inherited ones; a child of a painted box resolves its own.
        if carries_non_inherited_paint(&parent_style) {
            return Ok(false);
        }
        let writing_mode = parent_style.writing_mode;
        let direction = parent_style.direction;
        let text_orientation = parent_style.text_orientation;
        let (inherited, same, has_text, show_text) = {
            let record = self.record(id);
            let inherited = record.style.computed_style_is_inherited();
            (
                inherited,
                inherited
                    && Arc::ptr_eq(&record.resolved.0, &parent_style)
                    && record.resolved.1 == self.palette_epoch,
                !record.text.value.is_empty() || matches!(record.kind.as_ref(), NodeKind::Text),
                !record.resolved.0.visible && parent_style.visible,
            )
        };
        // A node that names its own language or typography scale, or follows
        // a responsive rule, resolves a style of its own.
        if !inherited
            || self.nodes.language(id).is_some()
            || self.nodes.text_scale(id).is_some()
            || self.responsive.follows(id)
            || self.localized_text(id).is_some()
        {
            return Ok(false);
        }
        {
            let record = self.record_mut(id);
            record.inherited_writing = nana_ui_core::WritingContext::new(writing_mode, direction);
            record.inherited_orientation = text_orientation;
        }
        if same {
            work.record_skipped();
            return Ok(true);
        }
        if has_text {
            self.invalidate_text_for_style(id, &parent_style, work);
        }
        if show_text {
            self.text_shown.push(id);
        }
        work.record_resolved();
        self.record_mut(id).resolved = ResolvedStyle(parent_style, self.palette_epoch);
        Ok(true)
    }

    fn resolve_style<const SHARE: bool>(
        &mut self,
        id: StableNodeId,
        resolved: &mut HashSet<StableNodeId>,
        work: &mut ThemeWorkCounters,
    ) -> Result<(), UiWorldError> {
        if !self.contains(id) {
            return Err(UiWorldError::MissingNode(id));
        }
        if !resolved.insert(id) {
            return Ok(());
        }
        let parent = self.record(id).hierarchy.parent;
        if let Some(parent) = parent {
            self.resolve_style::<SHARE>(parent, resolved, work)?;
        }
        if SHARE && self.publish_inherited_computed_style(id, parent, work)? {
            return Ok(());
        }
        // The layout the pipeline reads: design intent and a responsive
        // variant applied. Text, paint and visibility follow the variant.
        let layout = self.motion_layout(id, &self.record(id).resolved_layout);
        // Only a handful of fields are read out of the parent, so share its Arc
        // instead of cloning the whole `ComputedStyle` (and its three heap
        // fields) once per node. A borrow would pin `&mut self` to the end.
        let inherited_style = parent
            .map(|parent| Arc::clone(&self.record(parent).resolved.0))
            .unwrap_or_else(crate::store::interned_default_style);
        let inherited = inherited_style.as_ref();
        let inherited_color = parent.and(inherited.color);
        let (foreground, color, background, border_color) =
            self.palette_paint_colors(id, inherited_color);
        let local_paint = self.semantic_paint(id, &self.record(id).style);
        let color_override = layout.color.is_some()
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
        let pointer_events =
            PointerEventsSpec::inherit_from(layout.pointer_events, inherited.pointer_events);
        let text_scale = match self.nodes.text_scale(id) {
            Some(own) => *own,
            None if parent.is_some() => inherited.text_scale,
            None => self.root_text_scale(self.record(id).document),
        };
        let font_size_base = layout.font_size.unwrap_or(inherited.font_size_base);
        let line_height_base = layout.line_height.or(inherited.line_height_base);
        let next = ComputedStyle {
            paint_colors: nana_ui_core::PaintColorSlots {
                color: layout.paint_colors.color.or_else(|| {
                    (!color_override)
                        .then_some(inherited.paint_colors.color)
                        .flatten()
                }),
                // Backgrounds, borders, and outlines are non-inherited CSS
                // properties. Keep their authoring-space values local to the
                // node while the foreground and selection colors follow the
                // existing inherited compatibility fields below.
                background: layout.paint_colors.background,
                border: layout.paint_colors.border,
                border_top: layout.paint_colors.border_top,
                border_right: layout.paint_colors.border_right,
                border_bottom: layout.paint_colors.border_bottom,
                border_left: layout.paint_colors.border_left,
                outline: layout.paint_colors.outline,
                selection_background: layout.paint_colors.selection_background.or_else(|| {
                    layout
                        .selection_background
                        .is_none()
                        .then_some(inherited.paint_colors.selection_background)
                        .flatten()
                }),
                selection_color: layout.paint_colors.selection_color.or_else(|| {
                    layout
                        .selection_color
                        .is_none()
                        .then_some(inherited.paint_colors.selection_color)
                        .flatten()
                }),
            },
            foreground,
            color,
            background,
            border_color,
            opacity: layout.opacity.unwrap_or(1.0) * inherited.opacity,
            visibility,
            box_visible,
            visible: box_visible && visibility != nana_ui_core::VisibilitySpec::Hidden,
            pointer_events,
            cursor: layout.cursor.unwrap_or(inherited.cursor),
            cursor_specified: layout.cursor.is_some() || inherited.cursor_specified,
            user_select: layout.user_select.unwrap_or(inherited.user_select),
            selection_background: layout
                .selection_background
                .or(inherited.selection_background),
            selection_color: layout.selection_color.or(inherited.selection_color),
            font_size: font_size_base * text_scale,
            font_weight: layout.font_weight.or(inherited.font_weight),
            italic: layout.font_italic.unwrap_or(inherited.italic),
            font_family: layout
                .font_family
                .as_deref()
                .map(Arc::<str>::from)
                .or_else(|| inherited.font_family.clone()),
            line_height: line_height_base.map(|spec| scaled_line_height(spec, text_scale)),
            letter_spacing: layout.letter_spacing.unwrap_or(inherited.letter_spacing),
            font_features: layout
                .font_features
                .clone()
                .unwrap_or_else(|| inherited.font_features.clone()),
            font_variations: {
                let mut axes = layout
                    .font_variation_settings
                    .clone()
                    .unwrap_or_else(|| inherited.font_variations.clone());
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
            font_kerning: layout.font_kerning.unwrap_or(inherited.font_kerning),
            word_break: layout.word_break.unwrap_or(inherited.word_break),
            line_break: layout.line_break.unwrap_or(inherited.line_break),
            direction: layout.dir.unwrap_or(inherited.direction),
            writing_mode: layout.writing_mode.unwrap_or(inherited.writing_mode),
            text_orientation: layout
                .text_orientation
                .unwrap_or(inherited.text_orientation),
            // Its own language; else, for localized text, the language its
            // locale shapes in; else its parent's.
            language: match self.nodes.language(id) {
                Some(own) => Some(own.clone()),
                None => match self.localized_language(id) {
                    Some(localized) => Some(localized),
                    None if parent.is_some() => inherited.language.clone(),
                    None => self.root_language(),
                },
            },
            text_scale,
            font_size_base,
            line_height_base,
        };
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
        {
            let resolved = &self.record(id).resolved;
            if resolved.0.as_ref() == &next && resolved.1 == self.palette_epoch {
                work.record_skipped();
                return Ok(());
            }
        }
        work.record_resolved();
        // Identical inherited results can share immutable paint state. Local
        // authored style remains on the node; future changes publish a new Arc.
        let shared = if SHARE {
            parent
                .map(|parent| &self.record(parent).resolved.0)
                .filter(|inherited| inherited.as_ref() == &next)
                .map(Arc::clone)
        } else {
            None
        };
        let next = match shared {
            Some(shared) => shared,
            None => {
                // One `ComputedStyle` behind one `Arc`. A node that inherits
                // its parent's result verbatim borrows that allocation instead.
                work.record_allocation(1, size_of::<ComputedStyle>());
                Arc::new(next)
            }
        };
        self.invalidate_text_for_style(id, &next, work);
        if !self.record(id).resolved.0.visible && next.visible {
            // Text passes skip hidden nodes without resolving them, so a node
            // whose box changed while hidden comes back stale. The scheduled
            // text pass of this same frame picks it up; marking it dirty here,
            // after the drain, would cost the frame another pass.
            self.text_shown.push(id);
        }
        self.record_mut(id).resolved = ResolvedStyle(next, self.palette_epoch);
        Ok(())
    }
}

impl UiWorld {
    /// Apply a node's design intent to its layout, against the installed
    /// metrics. Returns the authored `Arc` untouched when there is nothing to
    /// resolve, so a node without intent costs nothing.
    /// Resolve a node's design intent into the layout box the pipeline reads.
    ///
    /// Returns the resolved layout and whether producing it had to copy the
    /// authored one. A node whose intent already matches what it authored —
    /// and every node with no intent at all — keeps sharing the same `Arc`.
    pub(crate) fn resolve_layout_intent(
        style: &NodeStyle,
        metrics: nana_ui_core::ThemeMetrics,
    ) -> (Arc<nana_ui_core::LayoutStyle>, bool) {
        let radius = style.radius.map(|tier| tier.resolve(metrics));
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
        if radius_settled
            && corners_settled
            && control_settled
            && padding_x_settled
            && padding_y_settled
            && surface_settled
            && square_settled
            && aspect_width_settled
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
        (layout, true)
    }

    /// Replace `layout` with an equal one written recently, if any.
    pub(crate) fn share_layout(&mut self, layout: &mut Arc<nana_ui_core::LayoutStyle>) {
        self.layouts.intern(layout);
    }

    /// Write a node's authored style and keep its resolved layout in step.
    ///
    /// The two have to move together: projection diffs against the authored
    /// style, while layout and extraction read the resolved one. Every path
    /// that writes `record.style` goes through here so the pair cannot drift.
    pub(crate) fn write_node_style(&mut self, id: StableNodeId, mut style: NodeStyle) {
        let current = &self.record(id).style.layout;
        if !Arc::ptr_eq(current, &style.layout) && **current == *style.layout {
            style.layout = Arc::clone(current);
        } else {
            self.layouts.intern(&mut style.layout);
        }
        let resolved = self.resolve_node_layout(id, &style);
        let depends_on_viewport = resolved.depends_on_viewport();
        if style_declares_intent(&style) {
            self.intent_nodes.insert(id);
        } else {
            self.intent_nodes.remove(&id);
        }
        let record = self.record_mut(id);
        record.style = style;
        record.resolved_layout = resolved;
        record.layout_depends_on_viewport = depends_on_viewport;
        self.note_layout_source_change();
    }

    /// Re-resolve one node's layout after its authored layout was mutated in
    /// place. A node without design intent keeps sharing the same `Arc`. The
    /// caller records the layout cause, which moves the input epoch.
    pub(crate) fn refresh_resolved_layout(&mut self, id: StableNodeId) {
        let style = self.record(id).style.clone();
        let resolved = self.resolve_node_layout(id, &style);
        self.record_mut(id).resolved_layout = resolved;
    }

    /// The layout the pipeline reads for `id` authored as `style`, and
    /// whether it is a copy: its design intent resolved against the installed
    /// metrics, then the variant its responsive rule's bucket picks (Issue
    /// #265). A node with neither keeps sharing its authored `Arc`.
    fn node_layout(
        &self,
        id: StableNodeId,
        style: &NodeStyle,
    ) -> (Arc<nana_ui_core::LayoutStyle>, bool) {
        let (mut resolved, mut copied) =
            Self::resolve_layout_intent(style, self.style_model.metrics);
        if let Some(variant) = self.responsive_variant(id) {
            variant.apply(Arc::make_mut(&mut resolved));
            copied = true;
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
            self.layouts.intern(&mut resolved);
        }
        resolved
    }

    /// Re-resolve the layout intent of every node that declares some, after
    /// a metrics install, and seed each live one whose resolved layout moved
    /// by what moved: a padding step moves the boxes that use it and no
    /// other. Returns how many moved.
    fn reresolve_layout_intent(&mut self) -> usize {
        let mut ids: Vec<StableNodeId> = self.intent_nodes.iter().copied().collect();
        ids.sort_unstable();
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
                self.layouts.intern(&mut resolved);
            }
            self.record_mut(id).resolved_layout = resolved;
            self.note_layout_source_change();
            moved += 1;
            if !self.presence_live(id) {
                continue;
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

    /// Read one role out of the token authority, counted for Issue #101 §4.
    fn theme_color(&self, role: SemanticColorRole) -> [f32; 4] {
        self.record_theme_read();
        self.style_model.color(role).as_rgba_array()
    }

    /// Resolve a palette mix. One question asked of the theme, so one read —
    /// the roles it blends are the mix's own business.
    fn theme_mix(&self, mix: nana_ui_core::SemanticColorMix) -> [f32; 4] {
        self.record_theme_read();
        mix.resolve(self.style_model).as_rgba_array()
    }

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
        let color = layout
            .color
            .or_else(|| paint.foreground_mix.map(|mix| self.theme_mix(mix)))
            .or_else(|| {
                paint
                    .foreground
                    .map(|role| self.theme_color(role))
                    .or(inherited_color)
                    .or_else(|| Some(self.theme_color(foreground)))
            });
        let background = layout
            .background
            .or_else(|| paint.background_mix.map(|mix| self.theme_mix(mix)))
            .or_else(|| paint.background.map(|role| self.theme_color(role)));
        let border_color = layout
            .resolved_border_color()
            .or_else(|| paint.border_mix.map(|mix| self.theme_mix(mix)))
            .or_else(|| paint.border.map(|role| self.theme_color(role)));
        if let Some(transition) = self.hover_transitions.get(&id) {
            let [color, background, border_color] = std::array::from_fn(|i| {
                interpolate_color(
                    transition.from[i],
                    [color, background, border_color][i],
                    transition.progress,
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
            if let Some(mix) = paint.foreground_mix {
                return Some(self.theme_mix(mix));
            }
            if let Some(role) = paint.foreground {
                return Some(self.theme_color(role));
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
            ..crate::SemanticPaint::default()
        }
        .overlay(local.interaction.base);
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
        // `focus_visible`, not `focused`: a control that was clicked is focused
        // and does not say so.
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
    pub(super) fn secondary_text_color(&self, id: StableNodeId) -> [f32; 4] {
        let local = &self.record(id).style;
        let role = self
            .semantic_paint(id, local)
            .foreground_secondary
            .unwrap_or(SemanticColorRole::Muted);
        self.theme_color(role)
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
            || (!self.painter_overrides.is_empty() && self.painter_overrides.contains_key(&id));
        let styled = !record.style.interaction.is_empty();
        if painted {
            self.mark_repaint(id);
        }
        if styled {
            let mut work = ThemeWorkCounters::default();
            work.record_paint_invalidation(1);
            self.record_theme_work(work);
            self.mark(id, DirtyMask::STYLE | DirtyMask::RENDER);
        }
    }
}

impl UiWorld {
    /// Resolve inherited visual state for dirty nodes. Parent state is always
    /// resolved before its descendants, independent of stable ID order.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        // Most frame batches already contain the full dirty frontier. Reserve
        // once so parent-chain deduplication does not repeatedly rehash large
        // initial documents.
        let mut resolved = HashSet::with_capacity(ids.len());
        let mut work = ThemeWorkCounters::default();
        for &id in ids {
            self.resolve_style::<true>(id, &mut resolved, &mut work)?;
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
        let mut work = ThemeWorkCounters::default();
        for &id in ids {
            self.resolve_style::<false>(id, &mut resolved, &mut work)?;
        }
        self.record_theme_work(work);
        self.reconcile_focus(ids);
        Ok(())
    }

    /// Install `theme` as the design system; what paints is it, or its
    /// high-contrast rendition while the system asks for one.
    pub(super) fn install_theme(&mut self, theme: Arc<nana_ui_core::CompiledTheme>) {
        self.installed_theme = theme;
        self.apply_compiled_theme(self.contrast_rendition());
    }

    /// Follow the system high-contrast setting. A presentation overlay: the
    /// installed theme stays as it was and comes back when the setting turns
    /// off. Whether anything changed.
    pub fn set_high_contrast(&mut self, high_contrast: bool) -> bool {
        if self.high_contrast == high_contrast {
            return false;
        }
        self.high_contrast = high_contrast;
        self.apply_compiled_theme(self.contrast_rendition());
        true
    }

    /// The installed theme with the high-contrast palette when it is on: the
    /// palette of the same lightness, opaque, and a title bar that matches
    /// its background.
    fn contrast_rendition(&self) -> Arc<nana_ui_core::CompiledTheme> {
        if !self.high_contrast {
            return Arc::clone(&self.installed_theme);
        }
        let mut model = self.installed_theme.style_model();
        model.palette = model.palette.for_system_contrast(true);
        model.titlebar = model.palette.background;
        Arc::new((*self.installed_theme).clone().with_style_model(model))
    }

    pub(super) fn apply_compiled_theme(&mut self, next: Arc<nana_ui_core::CompiledTheme>) {
        // Values, not identity. `CompiledTheme::is_same_revision` is the cheap
        // question and it trusts the author's generation bump; an install has
        // to be right even for a theme that forgot to bump.
        if *self.theme == *next {
            return;
        }
        let hover_ids = self.hover_transitions.keys().copied().collect::<Vec<_>>();
        for id in hover_ids {
            self.cancel_hover_transition(id);
        }
        let previous_metrics = self.style_model.metrics;
        // A dialog card is placed from its recipe, which no layout input
        // names; a change to it reaches only the modal frames.
        let dialog_changed = self.theme.recipes().dialog() != next.recipes().dialog();
        self.style_model = next.style_model();
        self.theme = next;
        self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
        // Painters re-record against the new palette and radius tiers.
        let painted = self
            .paint_recordings
            .get_mut()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in painted {
            self.mark_repaint(id);
        }
        // A corner shape only paints: it moves no box.
        let metrics_changed = !self.style_model.metrics.same_layout(&previous_metrics);
        let seeds_before = self.layout_seeds_created;
        let mut ids = Vec::new();
        for roots in self.live_document_roots.values() {
            for &root in roots {
                ids.extend(self.subtree_ids(root));
            }
        }
        // Every live node paints against the new palette. Layout hears only
        // of the boxes whose design intent resolves differently against new
        // metrics, each by what moved, and -- when the dialog recipe moved --
        // of the modal frames, whose cards it places; the rest of a document
        // lays nothing out again.
        let modal_frames = if dialog_changed {
            ids.iter()
                .copied()
                .filter(|id| {
                    matches!(
                        self.nodes.visual(*id),
                        Some(StandardVisual::ModalFrame { .. })
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut work = ThemeWorkCounters::default();
        work.record_paint_invalidation(ids.len());
        if !ids.is_empty() {
            work.record_allocation(1, ids.len().saturating_mul(size_of::<StableNodeId>()));
        }
        let mut dependents = 0;
        if metrics_changed {
            // Design intent resolves against the metrics, so a metrics install
            // is the one event that has to re-run it. Doing it here, once, is
            // what keeps it off every frame's read path.
            dependents = self.reresolve_layout_intent();
            work.record_layout_invalidation(dependents);
        }
        work.record_layout_invalidation(modal_frames.len());
        self.record_theme_work(work);
        for id in modal_frames {
            // The card's width wraps its title, and its slots are placed in
            // it: the frame measures and places again by its recipe.
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
                    nana_ui_core::LayoutFieldMask::ALL,
                    nana_ui_core::LayoutDependencyFootprint::ALL,
                ),
            );
        }
        for id in ids {
            self.mark(id, DirtyMask::RENDER);
        }
        let seeds = (self.layout_seeds_created - seeds_before) as usize;
        let counts = &mut self.pending_drain_counts;
        if metrics_changed {
            counts.theme_to_layout_seeds += seeds;
            counts.theme_metric_dependents_invalidated += dependents;
        } else {
            counts.theme_palette_layout_invalidations += seeds;
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct HoverTransition {
    pub from: [Option<[f32; 4]>; 3],
    /// Motion IR sample progress. Style interpolation reads this; it does not
    /// sample `animation_now` on its own clock.
    pub progress: f32,
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
                    progress: 0.0,
                    inherits_color: from[0] != to[0],
                },
            );
            self.start_component_track(
                id,
                crate::component_animation_kinds::HOVER,
                self.theme.duration(nana_ui_core::MotionRole::HoverColor),
                self.theme
                    .motion()
                    .easing(nana_ui_core::EasingRole::Standard),
                crate::AnimatableProperty::Progress,
                crate::MotionValue::Scalar(0.0),
                crate::MotionValue::Scalar(1.0),
                crate::MotionInterrupt::Retarget,
                None,
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

#[cfg(test)]
#[path = "style_sharing_tests.rs"]
mod sharing_tests;

/// How many recent layouts a world keeps to share. Siblings built alike
/// (list rows, toolbars) repeat within a few writes.
const RECENT_LAYOUTS: usize = 8;

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

/// Whether `style` sets a property children do not inherit: a background,
/// a border or an outline color.
fn carries_non_inherited_paint(style: &ComputedStyle) -> bool {
    let slots = &style.paint_colors;
    style.background.is_some()
        || style.border_color.is_some()
        || slots.background.is_some()
        || slots.border.is_some()
        || slots.border_top.is_some()
        || slots.border_right.is_some()
        || slots.border_bottom.is_some()
        || slots.border_left.is_some()
        || slots.outline.is_some()
}

impl UiWorld {
    /// The language a root inherits: the application's, else the text
    /// engine's fallback.
    fn root_language(&self) -> Option<nana_text::font::LanguageTag> {
        self.default_language
            .clone()
            .or_else(|| self.engine_fallback_language.clone())
    }

    /// Set the application's language, which every node without a language
    /// of its own or above it inherits. Text whose language moves shapes
    /// again; the rest only resolves its style again.
    pub fn set_default_language(&mut self, language: Option<nana_text::font::LanguageTag>) {
        let before = self.root_language();
        self.default_language = language;
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
        if self.engine_fallback_language == language {
            return;
        }
        let before = self.root_language();
        self.engine_fallback_language = language;
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
        work: &mut ThemeWorkCounters,
    ) {
        let previous = Arc::clone(&self.record(id).resolved.0);
        let dirty = crate::text_node::classify_computed_style_change(&previous, next);
        // What this class costs the text pipeline is `TextDirty::work`'s
        // answer, not a second copy of that mapping here. A colour-only change
        // implies SCENE_PAINT, so a palette switch stays paint work.
        let text_work = dirty.work();
        if text_work.intersects(crate::text_node::TextWork::SHAPE)
            || text_work.intersects(crate::text_node::TextWork::LAYOUT)
        {
            work.record_text_invalidation(1);
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
        self.document_text_scales
            .get(&document)
            .copied()
            .unwrap_or(self.default_text_scale)
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
        if self.default_text_scale == scale {
            self.note_equivalent_text_scale();
            return;
        }
        self.default_text_scale = scale;
        let mut documents: Vec<DocumentId> = self
            .live_document_roots
            .keys()
            .copied()
            .filter(|document| !self.document_text_scales.contains_key(document))
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
                self.document_text_scales.insert(document, scale);
            }
            None => {
                self.document_text_scales.remove(&document);
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
        let mut scanned = 0usize;
        let mut stack = vec![root];
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
        self.pending_drain_counts
            .typography_scale_scope_nodes_scanned += scanned;
    }

    /// A typography scale was set to the one it already was: nothing visited.
    pub(super) fn note_equivalent_text_scale(&mut self) {
        self.pending_drain_counts.typography_scale_equivalent_skips += 1;
    }

    /// `id`'s computed typography scale is going from `previous` to `next`.
    /// What it paints from its font size paints again; text of it that had
    /// resolved counts as reached, and its next layout as the scale's.
    fn note_text_scale_change(&mut self, id: StableNodeId, previous: f32, next: f32) {
        if previous == next {
            return;
        }
        if !self.shows_text(id) && self.standard_visual_ref(id).is_none() {
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
        self.nodes.get(id).is_some_and(|record| {
            !record.text.value.is_empty() || matches!(record.kind.as_ref(), NodeKind::Text)
        })
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
fn style_declares_intent(style: &NodeStyle) -> bool {
    style.radius.is_some()
        || style.corner_radii.is_some()
        || style.control_height.is_some()
        || style.control_padding_x.is_some()
        || style.control_padding_y.is_some()
        || style.surface_padding.is_some()
        || style.square.is_some()
}
