//! Style, theme, text, and localization mutation application.

use super::super::*;

impl UiWorld {
    pub(super) fn apply_restyle_or_text_mutation(&mut self, mutation: &UiMutation) -> bool {
        match mutation {
            UiMutation::SetStyle { id, style } => {
                self.restyle(*id, Some(style));
            }
            UiMutation::SetPresetTheme { mode } => {
                self.install_theme(nana_ui_core::builtin_theme_arc(*mode));
            }
            UiMutation::SetThemeTokens { theme } => {
                self.install_theme(Arc::clone(theme));
            }
            UiMutation::SetText { id, text } => {
                // Written text is literal: a locale no longer rewrites it.
                self.set_localized_text(*id, None);
                // Re-setting the same text is not a content change: nothing
                // about the node's shaping or layout moved.
                if *self.nodes.text_content(*id) != *text
                    || self.nodes.rich_text(*id).is_some()
                    || self.inline_record(*id).is_some()
                {
                    self.replace_text_content(*id, text.clone());
                }
                self.mark(
                    *id,
                    DirtyMask::TEXT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetLocale { id, locale } => {
                self.set_scope_locale(*id, locale.as_deref().cloned());
            }
            // A localized text, language, typography scale or responsive rule
            // equal to the one in place never gets here: the commit skips it
            // as a no-op.
            UiMutation::SetLocalizedText { id, text } => {
                self.set_localized_text(*id, text.as_deref().cloned());
            }
            UiMutation::SetRichText { id, rich } => {
                self.nodes.set_inline_atoms(*id, Arc::from([]));
                self.apply_rich_text(*id, rich.as_deref());
            }
            UiMutation::SetInlineContent { id, content } => self.apply_inline_content(*id, content),
            UiMutation::SetLanguage { id, language } => {
                self.nodes.set_language(*id, language.as_deref().cloned());
                // The subtree resolves its styles again and its text is
                // considered again; text whose computed language moved is
                // shaped again, the rest keeps its revision. Layout hears
                // of it only from metrics that moved.
                self.mark_subtree(*id, DirtyMask::STYLE | DirtyMask::TEXT);
            }
            UiMutation::SetTextScale { id, scale } => {
                self.nodes.set_text_scale(*id, *scale);
                self.mark_text_scale_scope(*id);
            }
            UiMutation::SetResponsive { id, rule } => {
                self.set_responsive_rule(*id, rule.clone());
            }
            UiMutation::SetReplacedMetadata { resource, metadata } => {
                self.apply_replaced_metadata(resource, metadata.as_deref().copied());
            }
            UiMutation::SetGlyphPresentation { id, presentation } => {
                if self.nodes.glyph_presentation(*id) != presentation.as_deref() {
                    self.nodes
                        .set_glyph_presentation(*id, presentation.as_deref().cloned());
                    // The scene carries it to the painter; no text pass reads it.
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetRichEditorMarks { id, marks } => {
                if self.nodes.rich_editor_marks(*id) != marks.as_deref() {
                    self.nodes.set_text_viewport_pin(*id, None);
                    self.nodes
                        .set_rich_editor_marks(*id, marks.as_deref().cloned());
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            _ => return false,
        }
        true
    }
}

pub(super) fn markdown_blocks_changed(
    previous: Option<&crate::WidgetVisualHandle>,
    next: Option<&crate::WidgetVisualHandle>,
) -> bool {
    #[cfg(feature = "rich-text")]
    {
        let blocks = |visual: Option<&crate::WidgetVisualHandle>| {
            visual
                .and_then(|v| v.downcast_ref::<crate::NativeMarkdownVisual>())
                .map(|v| Arc::clone(&v.blocks))
        };
        blocks(previous) != blocks(next)
    }
    #[cfg(not(feature = "rich-text"))]
    {
        let _ = (previous, next);
        false
    }
}

impl UiWorld {
    /// A node's style moves: its authored style to `authored`, a style write,
    /// or -- with `None` -- only what resolves from it, its responsive variant
    /// (Issue #265). Every comparison below reads the style the pipeline
    /// reads, the effective one over its resolved layout, so a variant is
    /// classified, marked and seeded exactly as the same change written as a
    /// style.
    pub(in crate::world) fn restyle(&mut self, id: StableNodeId, authored: Option<&NodeStyle>) {
        // Only the authored layouts are compared below.
        let previous_authored = Arc::clone(&self.authored_style(id).layout);
        let previous_effective = Arc::clone(&self.record(id).style.layout);
        let previous_presentation = self.record(id).style.clone();
        let previous_resolved = Arc::clone(&self.record(id).resolved_layout);
        self.write_node_style(
            id,
            authored
                .cloned()
                .unwrap_or_else(|| self.authored_style(id).clone()),
        );
        let next_authored = Arc::clone(&self.authored_style(id).layout);
        let next_presentation = self.record(id).style.clone();
        if previous_presentation.theme_patch != next_presentation.theme_patch {
            self.sync_node_theme_scope(id);
        }
        let resolved = Arc::clone(&self.record(id).resolved_layout);
        // Every comparison of the two resolved layouts below, in one pass,
        // remembered for the pair: nodes built alike restyle from the
        // shared default to one interned layout.
        let facts = self
            .invalidation
            .layout_pairs
            .facts(&previous_resolved, &resolved);
        if facts.sticky_inputs {
            self.rebind_sticky_subtree(id);
        }
        self.track_scroll_style(id, &previous_effective, &next_presentation.layout);
        let previous = NodeStyle {
            layout: Arc::clone(&previous_resolved),
            ..NodeStyle::clone(&previous_presentation)
        };
        let style = &NodeStyle {
            layout: Arc::clone(&resolved),
            ..NodeStyle::clone(&next_presentation)
        };
        // Compared once; three classifications below read it.
        let presentation_eq = super::node_presentation_eq(&previous, style);
        let inherited_text_changed = facts.inherited_text;
        // A colour scope reaches every descendant's colours, as an inherited
        // foreground does.
        let inherited_paint_changed = previous.foreground != style.foreground
            || previous.color_overrides != style.color_overrides
            || facts.inherited_paint;
        let inherited_opacity_changed = facts.opacity;
        let paint_visibility_changed = facts.paint_visibility;
        let pointer_events_changed = facts.pointer_events;
        let cursor_changed = facts.cursor;
        let user_select_changed = facts.user_select;
        let omits_box_changed = facts.omits_box;
        if !facts.same_text_constraints {
            self.nodes
                .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
        }
        if previous.text_horizontal_alignment != style.text_horizontal_alignment
            && self
                .nodes
                .text_node(id)
                .is_some_and(|text| !text.layout.is_null())
        {
            // Only a retained layout places lines by alignment; host
            // metrics do not read it. It moves no box, so no layout
            // scope reaches the text: schedule it explicitly.
            self.nodes
                .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
            self.mark(id, DirtyMask::TEXT);
        }
        // One pass over the resolved layout -- the authored one with
        // its design intent applied, which layout and paint read --
        // read by every classification below. A write of intent
        // alone (a control's padding step, its radius tier) moves it
        // and nothing authored; a write that moves neither is
        // equivalent, however it was spelled.
        let changed = facts.changed;
        let transform_changed = changed.intersects(nana_ui_core::LayoutStyleChange::TRANSFORM);
        if changed.intersects(nana_ui_core::LayoutStyleChange::CONTAINER) {
            self.find_query_containers_again(id);
        }
        let stacking_changed = changed.intersects(nana_ui_core::LayoutStyleChange::STACKING);
        let style_layout_invalidation = self.classify_layout_change(id, changed);
        if facts.content_image {
            self.reindex_replaced(id);
        }
        // A write's own counts; a variant counts as a responsive result.
        if authored.is_some() {
            if changed.is_empty() && presentation_eq {
                self.invalidation
                    .counters
                    .pending_layout_invalidations_counted
                    .layout_equivalent_mutations_skipped += 1;
                self.invalidation
                    .counters
                    .pending_drain_counts
                    .equivalent_style_layout_skips += 1;
            }
            if !style_layout_invalidation.is_empty() {
                self.invalidation
                    .counters
                    .pending_drain_counts
                    .style_to_layout_seeds += 1;
            }
            self.release_rewritten_holds(id, &previous_authored, &next_authored);
        }
        self.sync_node_presence(id);

        // An empty classification is counted there and queues nothing.
        self.record_layout_invalidation(id, style_layout_invalidation);
        // Flow membership is an input of the parent, even when the child's
        // exported size is unchanged. Once it becomes positioned, its new
        // dependency graph deliberately has no child-metrics edge back to
        // the flow: seed that context explicitly, as for a child-list edit.
        if let Some(parent) = self.parent_id(id) {
            let context = self
                .established_layout_context(parent)
                .unwrap_or(crate::FormattingContextKind::Flex);
            let content = self
                .layout_content_kind(id)
                .unwrap_or(crate::LayoutContentKind::Empty);
            let participation = |style: &nana_ui_core::LayoutStyle| {
                crate::ParticipationKind::classify(context, style, content)
            };
            if participation(&previous_resolved) != participation(&resolved) {
                self.record_child_list_invalidation(parent);
            }
        }
        if omits_box_changed {
            self.record_layout_invalidation(
                id,
                LayoutInvalidation::new(
                    LayoutInvalidationSource::Structure,
                    InvalidationReason::TOPOLOGY,
                    InvalidationKind::TOPOLOGY
                        .union(InvalidationKind::MEASURE)
                        .union(InvalidationKind::PLACEMENT),
                    LayoutStyleChange::FLOW.union(LayoutStyleChange::VISIBILITY),
                    LayoutDependencyFootprint::ALL,
                ),
            );
            // Shown or hidden, a child changes what its parent holds, as an
            // insert or a removal would.
            if let Some(parent) = self.parent_id(id) {
                self.note_structural_change(parent);
            }
        }

        if !style_excluding_transform_and_cursor_eq(presentation_eq, changed) {
            self.mark(id, DirtyMask::STYLE);
            if !style_change_is_layout_geometry_only(presentation_eq, changed) {
                self.mark(id, DirtyMask::RENDER);
            }
        }
        if previous.painter != style.painter {
            // The hit index entry holds the painter (Issue #217).
            self.mark(id, DirtyMask::INPUT);
            if style.painter.is_none() && !self.invalidation.painter_overrides.contains_key(&id) {
                self.invalidation.paint_recordings.get_mut().remove(&id);
            }
        }
        if inherited_paint_changed {
            self.mark_subtree(id, DirtyMask::STYLE | DirtyMask::RENDER);
        } else if inherited_opacity_changed {
            // Descendants resolve a new accumulated opacity, but they
            // do not paint differently for it: nothing downstream of
            // extraction reads `ComputedStyle::opacity`. The renderer
            // walks the ancestor chain itself, and a container with
            // descendants to fade is an opacity group, whose opacity
            // composites once at the group instead of reaching each
            // descendant's primitive. Re-extracting the subtree every
            // frame of a fade would rebuild primitives byte for byte
            // the same.
            self.mark_subtree(id, DirtyMask::STYLE);
        }
        if inherited_text_changed || omits_box_changed {
            // Inherited text and a box that appears or goes change
            // what descendants resolve, so their results wait for the
            // next pass. A layout field of this node does not: the
            // typed seed above is the whole classification, and the
            // frontier reaches the descendants that depend on it.
            // Keep the Arc so a bit-equivalent recompute can reuse it.
            self.suppress_layout_results_subtree(id);
            self.suppress_layout_result_chain(id, true);
        }
        if inherited_text_changed {
            self.mark_subtree(id, super::motion::INHERITED_TEXT_DIRTY);
        }
        if omits_box_changed || paint_visibility_changed {
            self.mark_subtree(
                id,
                DirtyMask::STYLE
                    | DirtyMask::INPUT
                    | DirtyMask::FOCUS_IME
                    | DirtyMask::ACCESSIBILITY
                    | DirtyMask::RENDER,
            );
            if let Some(parent) = self.parent_id(id) {
                self.mark(parent, DirtyMask::ACCESSIBILITY);
            }
        }
        if pointer_events_changed {
            // Inherited: unspecified descendants pick up the new used
            // value. Not a layout dirty.
            self.mark_subtree(id, DirtyMask::STYLE | DirtyMask::INPUT);
            self.clear_hover_for_pointer_events_none(id);
            self.withdraw_pointer_events_none(id);
        }
        if changed.intersects(nana_ui_core::LayoutStyleChange::INERT) {
            // Inherited and not undone below: every descendant resolves a
            // new used value. Hit testing, focus and accessibility read it;
            // nothing moves and nothing paints differently.
            self.mark_subtree(
                id,
                DirtyMask::STYLE
                    | DirtyMask::INPUT
                    | DirtyMask::FOCUS_IME
                    | DirtyMask::ACCESSIBILITY,
            );
            if self.is_inert(id) {
                self.release_inert_subtree(id);
            }
        }
        if cursor_changed {
            self.invalidation.cursor_style_dirty = true;
            // Cursor is inherited and consumed by the host from the
            // resolved style; descendants need fresh computed values,
            // but no layout, hit-test, or render extraction is required.
            self.mark_subtree(id, DirtyMask::STYLE);
        }
        if user_select_changed {
            self.mark_subtree(id, DirtyMask::STYLE | DirtyMask::RENDER);
        }
        if transform_changed {
            // Scene extract and hit-test read `layout.transform`; LAYOUT
            // does not, so paint-transform is not a layout dirty.
            //
            // Only this node is marked. Extraction: a descendant's
            // primitives are built in its own space and projected by the
            // chain above it, and the renderer re-projects a retained
            // descendant from the ancestor's new transform rather than
            // rebuilding it; extracting the subtree every frame of an
            // animation would hand back the same geometry. Hit testing: an
            // entry that hands its children its own layout coordinates
            // (`HitEntry::local_children`) is built again on its own, any
            // other patches its whole branch, and the pinned entries below
            // are built again either way. Accessibility: a transform that
            // gives what the box holds the same space projects the box and
            // the boxes placed from the viewport under it
            // (`SystemWork::transform_kept`); any other projects the
            // subtree from `SystemWork::transform`.
            if !stacking_changed && facts.keeps_space {
                self.mark_transform_kept(id);
            } else {
                self.mark(
                    id,
                    DirtyMask::TRANSFORM | DirtyMask::INPUT | DirtyMask::RENDER,
                );
            }
        } else if stacking_changed {
            self.mark_subtree(id, DirtyMask::INPUT | DirtyMask::RENDER);
        }
        if (inherited_text_changed || omits_box_changed)
            && let Some(parent) = self.parent_id(id)
        {
            self.mark_ancestors(parent, DirtyMask::RENDER);
        }
    }
}

impl UiWorld {
    /// Replace `id`'s text with `text`: its spans go, its shaped text is
    /// stale, and a document selection in it is dropped. The caller marks.
    pub(in crate::world) fn replace_text_content(&mut self, id: StableNodeId, text: TextContent) {
        self.mark_inline_text_owners(id);
        self.nodes.set_inline_atoms(id, Arc::from([]));
        self.nodes.set_text_content(id, text);
        // Spans over the old text would style the wrong bytes.
        self.nodes.set_rich_text(id, None);
        self.invalidate_text_content(id);
        if let Some(document) = self.nodes.get(id).map(|node| node.document)
            && self
                .text
                .document_text_selections
                .get(&document)
                .is_some_and(|selection| selection.node == id)
        {
            self.set_document_text_selection(document, None);
        }
        self.recompute_node_theme_deps(id);
    }
}
