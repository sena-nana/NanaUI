//! Bridge cascade state and operations.

use super::*;

#[derive(Debug, Default)]
pub(super) struct State {
    /// Root identity only; its computed font size is always read live.
    pub(super) font_root: Cell<Option<Option<WidgetId>>>,
    /// Parsed author stylesheet rules (source order across inject calls).
    /// Declaration entries are cached on each [`StyleRule`] at parse time.
    pub(super) stylesheet_rules: Vec<StyleRule>,
    /// Subject-key bucket index over [`Self::stylesheet_rules`], rebuilt with it.
    pub(super) stylesheet_rule_index: crate::css_cascade::RuleIndex,
    /// Deferred interactive (`:hover` / `:focus` / `:active`) rules.
    pub(super) interactive_rules: Vec<InteractiveStyleRule>,
    /// Deferred generated pseudo rules (`::before` / `::after`).
    pub(super) generated_pseudo_rules: Vec<GeneratedPseudoRule>,
    /// `::-webkit-scrollbar` / thumb skin rules (applied onto the originating node).
    pub(super) scrollbar_pseudo_rules: Vec<ScrollbarPseudoRule>,
    /// Parsed `@keyframes` blocks keyed by animation name.
    pub(super) keyframes: BTreeMap<String, KeyframesRule>,
    /// Static selector motion rules (`transition` / `animation` longhands).
    pub(super) motion_rules: Vec<MotionStyleRule>,
    pub(super) next_rule_order: u32,
    /// Unique simple `:has()` arguments for the current stylesheet (bitset index).
    pub(super) has_args: Vec<SimpleCompound>,
    /// Per-widget descendant-present bits matching [`Self::has_args`].
    pub(super) has_descendant_bits: HashMap<WidgetId, u64>,
    /// When true, [`Self::has_descendant_bits`] is valid for the current tree.
    pub(super) has_index_ready: bool,
    /// Last focused widget for cheap subject `:focus-within` (not a second engine).
    pub(super) cascade_focused: Option<WidgetId>,
    /// Author sheet contains subject `:focus-within`.
    pub(super) uses_focus_within: bool,
    /// Runtime pointer/focus snapshot used during the current interactive cascade.
    pub(super) interactive_runtime: Option<InteractiveRuntimeSnapshot>,
    /// Document-level custom properties (`:root` / `html` / `body` …) as inheritance base.
    /// Rebuilt from [`stylesheet_rules`] (no raw CSS re-scrape).
    pub(super) stylesheet_vars: BTreeMap<String, String>,
    /// Last synced layout viewport (`vw`/`vh` resolve during cascade).
    pub(super) layout_viewport: Option<(f32, f32)>,
    /// Document `@layer` order (first declared is weaker).
    /// Accumulated skipped-content counters across `inject_stylesheet` calls.
    pub(super) stylesheet_skips: StylesheetParseReport,
    /// Unflattened author sheets (imports already merged; `@media` kept conditional).
    pub(super) authored_sheets: Vec<ParsedStylesheet>,
    /// Shared relative forest for the current recascade pass.
    pub(super) relative_pass: Option<Arc<RelativeMatchForest>>,
    /// Test hook: how many times a relative forest was built.
    pub(super) relative_forest_builds: Cell<usize>,
    /// Test hook: identity nodes inserted into forests (must stay O(N), not N²).
    pub(super) relative_forest_nodes: Cell<usize>,
}

impl MessageBridge {
    /// Widgets whose interactive CSS result can differ from the previous pass.
    /// `None` = no previous snapshot (first pass → full recascade); empty =
    /// steady state. A hover/press change invalidates the subject plus its
    /// descendants (ancestor flags feed `.card:hover .icon` matching); a focus
    /// move additionally walks the old/new `:focus-within` ancestor chains.
    pub(super) fn interactive_dirty_ids(
        &self,
        next: &InteractiveRuntimeSnapshot,
    ) -> Option<HashSet<WidgetId>> {
        let prev = self.cascade.interactive_runtime.as_ref()?;
        let mut changed: HashSet<WidgetId> = HashSet::new();
        for (prev_map, next_map) in [
            (&prev.hovered, &next.hovered),
            (&prev.pressed, &next.pressed),
        ] {
            for key in prev_map.keys().chain(next_map.keys()) {
                if prev_map.contains_key(key) != next_map.contains_key(key) {
                    changed.insert(*key);
                }
            }
        }
        if prev.focused != next.focused {
            for focused in [prev.focused, next.focused].into_iter().flatten() {
                changed.insert(focused);
                let mut cur = self.widgets.get(&focused).and_then(|w| w.parent);
                while let Some(pid) = cur {
                    changed.insert(pid);
                    cur = self.widgets.get(&pid).and_then(|w| w.parent);
                }
            }
        }
        if changed.is_empty() {
            return Some(HashSet::new());
        }
        let mut dirty = changed.clone();
        let mut queue: Vec<WidgetId> = changed.iter().copied().collect();
        while let Some(id) = queue.pop() {
            let Some(widget) = self.widgets.get(&id) else {
                continue;
            };
            for child in &widget.children {
                if dirty.insert(*child) {
                    queue.push(*child);
                }
            }
        }
        Some(dirty)
    }
}

impl MessageBridge {
    /// Re-collect Runtime pointer/focus activation and recascade interactive CSS.
    pub(crate) fn reapply_interactive_cascade(&mut self, doc: &mut crate::tree::NanaTreeDocument) {
        if !self.has_interactive_css() {
            self.cascade.interactive_runtime = None;
            return;
        }
        let snapshot = Self::collect_interactive_runtime_snapshot(doc);
        self.cascade.cascade_focused = snapshot.focused;
        // Interactive pseudo-classes resolve at the subject and ancestor
        // positions only, so a hover/press/focus change invalidates exactly
        // [`Self::interactive_dirty_ids`]; the previous snapshot decides
        // whether anything changed at all (steady-state frames recascade
        // nothing). First pass (no snapshot) stays a full recascade.
        let dirty = self.interactive_dirty_ids(&snapshot);
        let ids: Vec<WidgetId> = match dirty {
            Some(dirty) => {
                let mut ids: Vec<WidgetId> = dirty
                    .into_iter()
                    .filter(|id| !self.is_generated_pseudo_widget(*id))
                    .collect();
                ids.sort_unstable();
                ids
            }
            None => self
                .widgets
                .keys()
                .copied()
                .filter(|id| !self.is_generated_pseudo_widget(*id))
                .collect(),
        };
        let from_snapshots: HashMap<WidgetId, CssPaintSnapshot> = ids
            .iter()
            .filter_map(|id| {
                let widget = self.widgets.get(id)?;
                if widget.props.attrs.contains_key(GENERATED_PSEUDO_ATTR) {
                    return None;
                }
                Some((
                    *id,
                    CssPaintSnapshot::from_layout_resolved(
                        &widget.props.layout,
                        widget.props.containing_block_width,
                        widget.props.containing_block_height,
                        self.cascade.layout_viewport,
                    ),
                ))
            })
            .collect();
        // Steady-state frames recascade nothing and must not bump the
        // revision — an unconditional bump here used to force a full semantic
        // resync on every hover frame.
        if !ids.is_empty() {
            self.changed_widgets(ids.iter().copied());
        }
        self.cascade.interactive_runtime = Some(snapshot);
        self.refresh_has_descendant_index();
        self.begin_relative_pass();
        for id in &ids {
            self.reapply_layout_for(*id);
        }
        let now = doc.runtime_now();
        for id in ids {
            self.sync_generated_pseudo_for(id, doc);
            let Some(motion) = self.motion.computed_motion.get(&id).cloned() else {
                continue;
            };
            if motion.animation_name.eq_ignore_ascii_case("none")
                || motion.animation_name.is_empty()
            {
                self.motion.css_keyframes_name.remove(&id);
            } else if self.cascade.keyframes.contains_key(&motion.animation_name)
                && !self.motion.css_transitions.contains_key(&id)
                && self.should_start_keyframes(id, &motion.animation_name)
                && let Some(spec) = build_keyframes_spec(id, &motion, now)
            {
                self.motion
                    .css_keyframes_name
                    .insert(id, motion.animation_name.clone());
                doc.start_css_animation(spec);
                self.queue_motion_cancel(id);
            }
            let Some(from) = from_snapshots.get(&id) else {
                continue;
            };
            if !self.widgets.contains_key(&id) {
                continue;
            }
            let to = self.cascaded_target_paint(id);
            if let Some(existing) = self.motion.css_transitions.get(&id) {
                if existing.to == to {
                    continue;
                }
                let current = self
                    .snapshot_widget(id)
                    .unwrap_or_else(|| CssPaintSnapshot::from_layout(&LayoutStyle::default()));
                if let Some(spec) = build_transition_spec(id, &motion, now) {
                    self.motion.css_transition_base.insert(id, current.clone());
                    self.motion.css_transition_progress.insert(id, 0.0);
                    self.motion.css_transitions.insert(
                        id,
                        ActiveCssTransition {
                            from: current.clone(),
                            to,
                            spec,
                        },
                    );
                    doc.start_css_animation(spec);
                    self.queue_motion_cancel(id);
                    self.pin_host_driven_transition_paint(doc, id, &current);
                    self.motion.paint_transform_overlays.remove(&id);
                    self.motion.paint_transform_releases.remove(&id);
                }
                continue;
            }
            if from == &to {
                continue;
            }
            if let Some(spec) = build_transition_spec(id, &motion, now) {
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
                self.pin_host_driven_transition_paint(doc, id, from);
                self.motion.paint_transform_overlays.remove(&id);
                self.motion.paint_transform_releases.remove(&id);
            }
        }
        self.release_pending_flip_transforms(doc);
        if doc.host_animation_epoch().is_none() {
            self.tick_css_animations(doc);
        }
        self.end_relative_pass();
    }
}

impl MessageBridge {
    pub(super) fn reapply_layout_for_inner(&mut self, id: WidgetId) {
        if self.is_generated_pseudo_widget(id) {
            return;
        }
        let Some(ancestry) = self.match_ancestry(id) else {
            return;
        };
        let is_empty = self.widget_is_empty(id);
        let Some(widget) = self.widgets.get(&id) else {
            return;
        };
        let kind = widget.kind;
        let parent_id = widget.parent;
        let class_names = widget.props.class_names.clone();
        let element_tag = widget.props.element_tag.clone();
        let element_id = widget.props.element_id.clone();
        let attrs = cascade_attrs_from_widget(widget);
        let inline_style = widget.props.inline_style.clone();
        let prop_style = widget.props.prop_style.clone();
        let hidden = widget.props.layout.hidden;
        let keep_bg = widget.props.layout.background;
        let keep_border_color = widget.props.layout.border_color;
        let keep_border_width = widget.props.layout.border_width;
        let cb_w = widget.props.containing_block_width;
        let cb_h = widget.props.containing_block_height;
        let checked = widget_checked_state(widget);

        // ancestry is [self, parent, grandparent, …] — full chain for combinators.
        let leaf_classes = class_names;
        let leaf_attrs = attrs;
        let leaf_tag = element_tag;
        let leaf_id = element_id;

        let (sibling_index, sibling_count) = self.sibling_position(id);
        let (of_type_index, of_type_count) = self.of_type_position(id);
        let prev_snaps = self.prev_sibling_snaps(id);
        self.ensure_relative_pass();
        let forest = self.cascade.relative_pass.clone();
        let sibling_snaps = if forest.is_some() {
            self.all_sibling_snaps(id)
        } else {
            Vec::new()
        };

        let ancestor_nodes: Vec<MatchNode<'_>> =
            ancestry.iter().skip(1).map(|n| n.as_node()).collect();
        let prev_nodes: Vec<MatchNode<'_>> = prev_snaps.iter().map(|n| n.as_node()).collect();
        let all_sibling_nodes: Vec<MatchNode<'_>> =
            sibling_snaps.iter().map(|n| n.as_node()).collect();
        let ctx = MatchContext {
            tag: leaf_tag.as_str(),
            id: leaf_id.as_str(),
            classes: leaf_classes.as_slice(),
            attrs: &leaf_attrs,
            ancestors: ancestor_nodes.as_slice(),
            preceding_siblings: prev_nodes.as_slice(),
            sibling_index,
            sibling_count,
            of_type_index,
            of_type_count,
            has_bits: self
                .cascade
                .has_descendant_bits
                .get(&id)
                .copied()
                .unwrap_or(0),
            has_args: self.cascade.has_args.as_slice(),
            focus_within: self.focus_within_of(id),
            is_empty,
            checked,
            media: self.media_env(),
            children: &[],
            following_siblings: &[],
            all_siblings: all_sibling_nodes.as_slice(),
            ancestor_subtrees: &[],
            owned_children: &[],
            owned_following: &[],
            owned_ancestor_trees: &[],
            relative: forest.as_deref(),
            relative_id: id,
        };

        // Layer order: kind default → stylesheet → class hints → prop → inline
        // → stylesheet !important → prop / inline !important.
        // When any author text layer or retained stylesheet exists, rebuild from
        // a clean base so prior stylesheet-computed fields do not stick after
        // selector/class changes. Document-root Fill is restored by public
        // `nana-*-root` class contracts — not by preserving computed layout
        // across a global stylesheet inject.
        //
        // Critical: do **not** seed `direction` from WidgetKind when author CSS
        // is present. `default_layout_for_kind(Column)` would set Column, then
        // `display:flex` (`if direction.is_none() → Row`) would no-op — toolbars
        // with `justify-content:space-between` stay vertical and eat the Fill
        // height, clipping siblings (Repo evidence main pane painted empty).
        let mut base = if self.cascade.stylesheet_rules.is_empty()
            && self.cascade.authored_sheets.is_empty()
            && inline_style.trim().is_empty()
            && prop_style.trim().is_empty()
        {
            // Preserve LayoutStyle fields assigned directly (scaffold /
            // createWidget / register props) when no author CSS layers exist.
            let mut layout = self
                .widgets
                .get(&id)
                .map(|w| w.props.layout.clone())
                .unwrap_or_else(|| default_layout_for_kind(kind));
            let defaults = default_layout_for_kind(kind);
            if layout.direction.is_none() {
                layout.direction = defaults.direction;
            }
            if layout.gap.is_none() {
                layout.gap = defaults.gap;
            }
            if layout.padding.is_none() {
                layout.padding = defaults.padding;
            }
            layout
        } else {
            let mut layout = LayoutStyle::default();
            let defaults = default_layout_for_kind(kind);
            // Card/SettingsCard keep kind padding seed only — never direction.
            // Gap must come from author CSS / `gap-*` hints (not kind default).
            layout.gap = defaults.gap;
            layout.padding = defaults.padding;
            layout
        };

        // CSS `direction` inherits. Seed the used parent value before cascade so
        // `padding-inline-start` on the child maps against RTL without requiring
        // the child to repeat `direction`. Do not seed flex `direction`.
        if base.dir.is_none()
            && let Some(pid) = parent_id
            && let Some(parent) = self.widgets.get(&pid)
        {
            base.dir = parent.props.layout.dir;
        }
        // HTML `dir` is a presentational hint: overrides inherited dir, loses to
        // author CSS `direction`. `auto` is fail-closed (no specified value).
        if let Some(attr_dir) = crate::widget_map::html_dir_spec_from_map(&leaf_attrs) {
            base.dir = Some(attr_dir);
        }

        // Author layers: stylesheet → class hints → prop style → class hints →
        // inline → class hints → stylesheet !important → prop !important →
        // inline !important. Layout sizing comes from those layers / public
        // class contracts — not from id / data-region-id / kind whitelists.
        let mut layout = rebuild_layout_style_indexed(
            base,
            &self.cascade.stylesheet_rules,
            &self.cascade.stylesheet_rule_index,
            &ctx,
            &prop_style,
            &inline_style,
            cb_w,
            cb_h,
        );
        if !self.cascade.scrollbar_pseudo_rules.is_empty() {
            apply_scrollbar_pseudo_skin(
                &mut layout,
                &self.cascade.scrollbar_pseudo_rules,
                &ctx,
                cb_w,
                cb_h,
            );
        }

        if let Some(runtime) = &self.cascade.interactive_runtime {
            let subject = runtime.subject_flags(id);
            let ancestors = runtime.ancestor_flags(self, id);
            let istate = InteractiveMatchState {
                subject,
                ancestors: &ancestors,
            };
            if !self.cascade.interactive_rules.is_empty() {
                apply_interactive_layers(
                    &mut layout,
                    &ctx,
                    &self.cascade.interactive_rules,
                    &istate,
                    cb_w,
                    cb_h,
                );
            }
            let interactive_motion = self.interactive_motion_for(&ctx, runtime, id);
            let computed =
                resolve_computed_motion(&self.cascade.motion_rules, interactive_motion, None, &ctx);
            self.motion.computed_motion.insert(id, computed);
        } else if !self.cascade.motion_rules.is_empty() {
            let computed = resolve_computed_motion(&self.cascade.motion_rules, None, None, &ctx);
            self.motion.computed_motion.insert(id, computed);
        } else {
            self.motion.computed_motion.remove(&id);
        }

        if let Some(transition) = self.motion.css_transitions.get(&id) {
            let progress = self
                .motion
                .css_transition_progress
                .get(&id)
                .copied()
                .unwrap_or(0.0);
            let base = self
                .motion
                .css_transition_base
                .get(&id)
                .unwrap_or(&transition.from);
            let properties = self
                .motion
                .computed_motion
                .get(&id)
                .map(|motion| parse_transition_properties(&motion.transition_property))
                .unwrap_or_default();
            let paint = lerp_paint_for_properties(base, &transition.to, progress, &properties);
            paint.apply_to_layout(&mut layout);
        }
        // Custom-element contract: tag `nana-sidebar-frame` / `nana-sidebar-row`
        // mirrors the public class hints when Vue omitted `class` (host CEs often
        // only set the tag). This is the element-name contract — not a WidgetKind
        // whitelist inventing geometry from the enum alone.
        if leaf_tag.starts_with("nana-") && !leaf_classes.iter().any(|c| c == &leaf_tag) {
            layout.apply_class_layout_hints(std::slice::from_ref(&leaf_tag));
        }
        // Preserve SVG fill/stroke paint when stylesheet didn't set them —
        // unless the author explicitly declared `fill`/`stroke` this pass and
        // resolution failed (e.g. LightningCSS `light-dark` → `initial`). Keeping
        // a prior dark `#1c1c1c` would paint black empty heatmap cells on light.
        let author_fill = css_decl_mentions(&inline_style, "fill")
            || css_decl_mentions(&prop_style, "fill")
            || leaf_attrs.contains_key("fill");
        let author_stroke = css_decl_mentions(&inline_style, "stroke")
            || css_decl_mentions(&prop_style, "stroke")
            || leaf_attrs.contains_key("stroke");
        if layout.background.is_none() && !author_fill {
            layout.background = keep_bg;
        }
        if layout.border_color.is_none() && !author_stroke {
            layout.border_color = keep_border_color;
        }
        if layout.border_width.is_none() && !author_stroke {
            layout.border_width = keep_border_width;
        }
        // CSS typography inherits when the author layers leave fields unset.
        if let Some(parent_id) = self.widgets.get(&id).and_then(|w| w.parent)
            && let Some(parent) = self.widgets.get(&parent_id)
        {
            layout.inherit_typography_from(&parent.props.layout);
        }
        if matches!(
            kind,
            WidgetKind::Input | WidgetKind::NumberInput | WidgetKind::Textarea
        ) && !self.cascade.generated_pseudo_rules.is_empty()
        {
            let matched = crate::css_interactive::matched_generated_pseudo(
                &self.cascade.generated_pseudo_rules,
                &ctx,
            );
            apply_placeholder_paint(&mut layout, &matched.placeholder, cb_w, cb_h);
        }
        // Preserve explicit hidden flag from the `hidden` attribute.
        if hidden {
            layout.hidden = true;
        }

        if leaf_tag.eq_ignore_ascii_case("img") {
            let src = leaf_attrs
                .get("src")
                .or_else(|| leaf_attrs.get("data-src"))
                .map(String::as_str)
                .unwrap_or("");
            crate::css_paint::apply_img_replaced_content(&mut layout, src);
        } else if leaf_tag.eq_ignore_ascii_case("video") {
            let poster = leaf_attrs.get("poster").map(String::as_str).unwrap_or("");
            let slotted = leaf_attrs.iter().any(|(key, value)| {
                key.eq_ignore_ascii_case("data-nana-video")
                    && value.trim().parse::<u64>().ok().is_some_and(|id| id > 0)
            });
            crate::css_paint::apply_video_poster(&mut layout, poster, slotted);
        } else if leaf_tag.eq_ignore_ascii_case("iframe") {
            crate::css_paint::apply_iframe_skip(&mut layout);
        } else if leaf_tag.eq_ignore_ascii_case("canvas") {
            let slotted = leaf_attrs.iter().any(|(key, value)| {
                !value.is_empty()
                    && (key.eq_ignore_ascii_case("data-nana-canvas")
                        || key.eq_ignore_ascii_case("data-nana-gpu"))
            });
            crate::css_paint::apply_canvas_skip(&mut layout, slotted);
        }

        crate::svg_inline::apply_inline_svg_replaced(self, id, &mut layout);

        if let Some(overlay) = self.motion.paint_transform_overlays.get(&id).copied()
            && !self.motion.paint_transform_releases.contains(&id)
        {
            layout.transform = Some(overlay);
        }

        if let Some(widget) = self.widgets.get_mut(&id) {
            if widget.props.layout != layout {
                widget.props.layout = layout;
            }
            pin_svg_chart_min_height(&mut widget.props);
        }
    }
}

impl MessageBridge {
    pub(super) fn authored_custom_properties_on(&self, id: WidgetId) -> BTreeMap<String, String> {
        let Some(ancestry) = self.match_ancestry(id) else {
            return BTreeMap::new();
        };
        let is_empty = self.widget_is_empty(id);
        let Some(widget) = self.widgets.get(&id) else {
            return BTreeMap::new();
        };
        let leaf_classes = widget.props.class_names.clone();
        let leaf_attrs = cascade_attrs_from_widget(widget);
        let leaf_tag = widget.props.element_tag.clone();
        let leaf_id = widget.props.element_id.clone();
        let prop_style = widget.props.prop_style.clone();
        let inline_style = widget.props.inline_style.clone();
        let (sibling_index, sibling_count) = self.sibling_position(id);
        let (of_type_index, of_type_count) = self.of_type_position(id);
        let prev_snaps = self.prev_sibling_snaps(id);
        let ancestor_nodes: Vec<MatchNode<'_>> =
            ancestry.iter().skip(1).map(|n| n.as_node()).collect();
        let prev_nodes: Vec<MatchNode<'_>> = prev_snaps.iter().map(|n| n.as_node()).collect();
        let ctx = MatchContext {
            tag: leaf_tag.as_str(),
            id: leaf_id.as_str(),
            classes: leaf_classes.as_slice(),
            attrs: &leaf_attrs,
            ancestors: ancestor_nodes.as_slice(),
            preceding_siblings: prev_nodes.as_slice(),
            sibling_index,
            sibling_count,
            of_type_index,
            of_type_count,
            has_bits: self
                .cascade
                .has_descendant_bits
                .get(&id)
                .copied()
                .unwrap_or(0),
            has_args: self.cascade.has_args.as_slice(),
            focus_within: self.focus_within_of(id),
            is_empty,
            checked: widget_checked_state(widget),
            media: self.media_env(),
            children: &[],
            following_siblings: &[],
            all_siblings: &[],
            ancestor_subtrees: &[],
            owned_children: &[],
            owned_following: &[],
            owned_ancestor_trees: &[],
            relative: None,
            relative_id: 0,
        };
        let mut map = crate::css_cascade::matched_custom_properties_indexed(
            &self.cascade.stylesheet_rules,
            &self.cascade.stylesheet_rule_index,
            &ctx,
        );
        for (k, v) in crate::css_map::extract_css_custom_properties_from_decls(&prop_style) {
            map.insert(k, v);
        }
        for (k, v) in crate::css_map::extract_css_custom_properties_from_decls(&inline_style) {
            map.insert(k, v);
        }
        map
    }
}

impl MessageBridge {
    /// Document vars + ancestor/self matched `--*` + inline/prop (root → leaf).
    pub(super) fn inherited_css_vars_for(&self, id: WidgetId) -> BTreeMap<String, String> {
        let mut chain = Vec::new();
        let mut cur = Some(id);
        while let Some(cid) = cur {
            chain.push(cid);
            cur = self.widgets.get(&cid).and_then(|w| w.parent);
        }
        chain.reverse();
        let mut map = self.cascade.stylesheet_vars.clone();
        for cid in chain {
            let overlay = self.authored_custom_properties_on(cid);
            if !overlay.is_empty() {
                map = crate::css_map::merge_css_custom_properties(&map, &overlay);
            }
        }
        // Ensure nested var()/simple calc in the inherited map are folded.
        crate::css_map::merge_css_custom_properties(&map, &BTreeMap::new())
    }
}

impl MessageBridge {
    /// Overlay kinds must not retain companion CSS `fixed`/`sticky`.
    /// L2 floats use Nana Overlay; anonymous CSS fixed stays on non-overlay nodes.
    pub(super) fn strip_deferred_position_on_overlay(&mut self, id: WidgetId) {
        if let Some(w) = self.widgets.get_mut(&id)
            && w.kind.is_overlay()
            && matches!(
                w.props.layout.position,
                crate::css_map::PositionSpec::Fixed | crate::css_map::PositionSpec::Sticky
            )
        {
            w.props.layout.position = crate::css_map::PositionSpec::Static;
        }
    }
}

impl MessageBridge {
    pub(super) fn reapply_layout_for(&mut self, id: WidgetId) {
        // Cascaded props/layout for this widget may change: record it for the
        // incremental semantic sync (a no-op reapply costs one set insert).
        self.changes.dirty.insert(id);
        // Refresh once per cascade pass. `bump()` clears the flag so a loop of
        // `reapply_layout_for` never rebuilds the index per node (O(n²)).
        self.ensure_has_index();
        let vars = self.inherited_css_vars_for(id);
        let fonts = self.font_context_for(id);
        let viewport = self.cascade.layout_viewport;
        let dark = matches!(self.theme, ThemeMode::Dark);
        let run = || {
            if let Some((vw, vh)) = viewport {
                crate::css_map::with_active_viewport(vw, vh, || {
                    crate::css_map::with_active_font_sizes(fonts, || {
                        crate::css_map::with_active_css_vars(&vars, || {
                            self.reapply_layout_for_inner(id);
                        })
                    })
                });
            } else {
                crate::css_map::with_active_font_sizes(fonts, || {
                    crate::css_map::with_active_css_vars(&vars, || {
                        self.reapply_layout_for_inner(id);
                    })
                });
            }
        };
        crate::css_map::with_active_color_scheme_dark(dark, run);
        self.strip_deferred_position_on_overlay(id);
    }
}

impl MessageBridge {
    /// Parent computed font-size as `em` base while applying this node's CSS.
    pub(super) fn font_context_for(&self, id: WidgetId) -> crate::css_map::FontSizeContext {
        let root_px = self.document_root_font_px();
        let parent_px = self
            .widgets
            .get(&id)
            .and_then(|w| w.parent)
            .and_then(|pid| self.widgets.get(&pid))
            .and_then(|p| p.props.layout.font_size)
            .unwrap_or(root_px);
        crate::css_map::FontSizeContext::new(root_px, parent_px)
    }
}

impl MessageBridge {
    /// Root `rem` base: html → body → CSS initial 16px.
    pub(super) fn document_root_font_px(&self) -> f32 {
        let root = self.cascade.font_root.get().unwrap_or_else(|| {
            let root = self
                .widgets
                .values()
                .find(|w| w.props.element_tag.eq_ignore_ascii_case("html"))
                .or_else(|| {
                    self.widgets
                        .values()
                        .find(|w| w.props.element_tag.eq_ignore_ascii_case("body"))
                })
                .map(|w| w.id);
            self.cascade.font_root.set(Some(root));
            root
        });
        root.and_then(|id| self.widgets.get(&id))
            .and_then(|w| w.props.layout.font_size)
            .unwrap_or(16.0)
            .max(1.0)
    }
}

impl MessageBridge {
    pub(super) fn widget_depth(&self, id: WidgetId) -> usize {
        let mut depth = 0usize;
        let mut cur = self.widgets.get(&id).and_then(|w| w.parent);
        while let Some(pid) = cur {
            depth += 1;
            cur = self.widgets.get(&pid).and_then(|w| w.parent);
        }
        depth
    }
}

impl MessageBridge {
    pub(super) fn widget_matches_rules(&self, id: WidgetId, rules: &[StyleRule]) -> bool {
        if rules.is_empty() {
            return false;
        }
        let Some(widget) = self.widgets.get(&id) else {
            return false;
        };
        let tag = if widget.props.element_tag.is_empty() {
            widget.kind.element_tag().to_string()
        } else {
            widget.props.element_tag.clone()
        };
        let element_id = widget.props.element_id.clone();
        let class_names = widget.props.class_names.clone();
        if !stylesheet_may_match_subject(rules, &tag, element_id.as_str(), &class_names) {
            return false;
        }
        let is_empty = self.widget_is_empty(id);
        let Some(ancestry) = self.match_ancestry(id) else {
            return false;
        };
        let Some(widget) = self.widgets.get(&id) else {
            return false;
        };
        let leaf_classes = widget.props.class_names.clone();
        let leaf_attrs = cascade_attrs_from_widget(widget);
        let leaf_id = widget.props.element_id.clone();
        let (sibling_index, sibling_count) = self.sibling_position(id);
        let (of_type_index, of_type_count) = self.of_type_position(id);
        let prev_snaps = self.prev_sibling_snaps(id);
        let ancestor_nodes: Vec<MatchNode<'_>> =
            ancestry.iter().skip(1).map(|n| n.as_node()).collect();
        let prev_nodes: Vec<MatchNode<'_>> = prev_snaps.iter().map(|n| n.as_node()).collect();
        let ctx = MatchContext {
            tag: tag.as_str(),
            id: leaf_id.as_str(),
            classes: leaf_classes.as_slice(),
            attrs: &leaf_attrs,
            ancestors: ancestor_nodes.as_slice(),
            preceding_siblings: prev_nodes.as_slice(),
            sibling_index,
            sibling_count,
            of_type_index,
            of_type_count,
            has_bits: self
                .cascade
                .has_descendant_bits
                .get(&id)
                .copied()
                .unwrap_or(0),
            has_args: self.cascade.has_args.as_slice(),
            focus_within: self.focus_within_of(id),
            is_empty,
            checked: widget_checked_state(widget),
            media: self.media_env(),
            children: &[],
            following_siblings: &[],
            all_siblings: &[],
            ancestor_subtrees: &[],
            owned_children: &[],
            owned_following: &[],
            owned_ancestor_trees: &[],
            relative: None,
            relative_id: 0,
        };
        stylesheet_matches(rules, &ctx)
    }
}

impl MessageBridge {
    pub(super) fn collect_subtree_ids(&self, id: WidgetId, out: &mut HashSet<WidgetId>) {
        if !out.insert(id) {
            return;
        }
        let children = self
            .widgets
            .get(&id)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        for child in children {
            self.collect_subtree_ids(child, out);
        }
    }
}

impl MessageBridge {
    pub(super) fn reapply_layout_cascade_matching(&mut self, new_rules: &[StyleRule]) {
        if new_rules.is_empty() {
            return;
        }
        self.refresh_has_descendant_index();
        let mut dirty = HashSet::new();
        let ids: Vec<WidgetId> = self.widgets.keys().copied().collect();
        for id in ids {
            if self.widget_matches_rules(id, new_rules) {
                self.collect_subtree_ids(id, &mut dirty);
            }
        }
        if dirty.is_empty() {
            return;
        }
        let mut ordered: Vec<WidgetId> = dirty.into_iter().collect();
        ordered.sort_by_cached_key(|id| self.widget_depth(*id));
        for id in &ordered {
            self.reapply_layout_for(*id);
        }
        self.changed_widgets(ordered);
    }
}

impl MessageBridge {
    pub(super) fn reapply_layout_cascade_all(&mut self) {
        let mut ids: Vec<WidgetId> = self.widgets.keys().copied().collect();
        // Parents before children so inherited typography / em font-size see
        // computed ancestor `font-size` (CSS inheritance + rem root).
        ids.sort_by_cached_key(|id| self.widget_depth(*id));
        self.refresh_has_descendant_index();
        self.begin_relative_pass();
        for id in &ids {
            self.reapply_layout_for(*id);
        }
        self.end_relative_pass();
        self.changed_all();
    }
}

impl MessageBridge {
    pub(super) fn node_self_has_bits(&self, id: WidgetId) -> u64 {
        let is_empty = self.widget_is_empty(id);
        let Some(widget) = self.widgets.get(&id) else {
            return 0;
        };
        let attrs = cascade_attrs_from_widget(widget);
        let node = MatchNode {
            tag: widget.props.element_tag.as_str(),
            id: widget.props.element_id.as_str(),
            classes: widget.props.class_names.as_slice(),
            attrs: &attrs,
            is_empty,
            checked: widget_checked_state(widget),
        };
        let mut bits = 0u64;
        for (i, arg) in self.cascade.has_args.iter().enumerate() {
            if i < 64 && simple_matches(arg, &node) {
                bits |= 1u64 << i;
            }
        }
        bits
    }
}

impl MessageBridge {
    pub(super) fn compute_has_bits_postorder(&mut self, id: WidgetId) -> u64 {
        if let Some(&bits) = self.cascade.has_descendant_bits.get(&id) {
            return bits;
        }
        let children = self
            .widgets
            .get(&id)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        let mut bits = 0u64;
        for child in children {
            bits |= self.node_self_has_bits(child);
            bits |= self.compute_has_bits_postorder(child);
        }
        self.cascade.has_descendant_bits.insert(id, bits);
        bits
    }
}

impl MessageBridge {
    /// One post-order walk: bit i means a descendant matches `has_args[i]`.
    /// Total work is O(n·k) with k unique simple `:has()` args (cap 64).
    pub(super) fn refresh_has_descendant_index(&mut self) {
        let mut args = Vec::new();
        for rule in &self.cascade.stylesheet_rules {
            for sel in &rule.selectors {
                push_has_args(&sel.subject, &mut args);
            }
        }
        for rule in &self.cascade.interactive_rules {
            push_has_args(&rule.selector.subject, &mut args);
        }
        for rule in &self.cascade.generated_pseudo_rules {
            push_has_args(&rule.originating_selector.subject, &mut args);
        }
        self.cascade.has_args = args;
        self.cascade.has_descendant_bits.clear();
        self.cascade.has_index_ready = true;
        if self.cascade.has_args.is_empty() {
            return;
        }
        let ids: Vec<WidgetId> = self.widgets.keys().copied().collect();
        for id in ids {
            if !self.cascade.has_descendant_bits.contains_key(&id) {
                self.compute_has_bits_postorder(id);
            }
        }
    }
}

impl MessageBridge {
    pub(super) fn ensure_has_index(&mut self) {
        if !self.cascade.has_index_ready {
            self.refresh_has_descendant_index();
        }
    }
}

impl MessageBridge {
    pub fn scrollbar_pseudo_rule_count(&self) -> usize {
        self.cascade.scrollbar_pseudo_rules.len()
    }
}

impl MessageBridge {
    pub fn generated_pseudo_rule_count(&self) -> usize {
        self.cascade.generated_pseudo_rules.len()
    }
}

impl MessageBridge {
    pub fn interactive_rule_count(&self) -> usize {
        self.cascade.interactive_rules.len()
    }
}

impl MessageBridge {
    pub fn stylesheet_rule_count(&self) -> usize {
        self.cascade.stylesheet_rules.len()
    }
}

impl MessageBridge {
    /// Re-collect document `--*` for the active theme from cached rule entries.
    pub(super) fn rebuild_stylesheet_vars(&mut self) {
        let theme = self.theme_label().to_string();
        self.cascade.stylesheet_vars =
            collect_document_custom_properties_from_rules(&self.cascade.stylesheet_rules, &theme);
    }
}

impl MessageBridge {
    /// Accumulated stylesheet skipped-content counters, so hosts can surface
    /// dropped rules/selectors instead of styles silently going missing.
    pub fn stylesheet_skips(&self) -> StylesheetParseReport {
        self.cascade.stylesheet_skips
    }
}

impl MessageBridge {
    pub(super) fn rebuild_active_stylesheet(&mut self) {
        let env = self.media_environment();
        let mut combined = ParsedStylesheet::default();
        for sheet in &self.cascade.authored_sheets {
            merge_parsed_stylesheet(&mut combined, sheet.flatten(&env));
        }
        self.cascade.stylesheet_rules = combined.static_rules;
        self.cascade.stylesheet_rule_index =
            crate::css_cascade::RuleIndex::build(&self.cascade.stylesheet_rules);
        self.cascade.interactive_rules = combined.interactive_rules;
        self.cascade.generated_pseudo_rules = combined.generated_pseudo_rules;
        self.cascade.scrollbar_pseudo_rules = combined.scrollbar_pseudo_rules;
        self.cascade.motion_rules = combined.motion_rules;
        self.cascade.keyframes = combined.keyframes;
        self.cascade.uses_focus_within = stylesheet_uses_focus_within(
            &self.cascade.stylesheet_rules,
            &self.cascade.interactive_rules,
            &self.cascade.generated_pseudo_rules,
        );
        self.discard_interactive_runtime_if_unused();
    }
}

impl MessageBridge {
    pub(super) fn authored_has_media(&self) -> bool {
        self.cascade
            .authored_sheets
            .iter()
            .any(|sheet| !sheet.media_rules.is_empty())
    }
}

impl MessageBridge {
    pub(super) fn media_environment(&self) -> MediaEnvironment {
        let (width, height) = self.cascade.layout_viewport.unwrap_or((960.0, 640.0));
        MediaEnvironment {
            width,
            height,
            color_scheme_dark: matches!(self.theme, ThemeMode::Dark),
        }
    }
}

impl MessageBridge {
    /// Same subset as CSS `@media` flatten (`screen`/`all` true, `print` false).
    pub fn evaluate_media_query_text(&self, query: &str) -> bool {
        evaluate_media_query_list(&parse_media_query_list(query), &self.media_environment())
    }
}

impl MessageBridge {
    /// Parse and retain stylesheet rules, then recascade matching subtrees.
    ///
    /// Empty / fully-deferred sheets are a no-op. Non-empty injects dirty nodes
    /// that match the new rules and their descendants. Unmatched subtrees stay.
    /// `@import` loads through [`FsStylesheetLoader`] into this same cascade;
    /// `@media` is stored parsed and flattened against the current viewport /
    /// theme without re-parsing CSS text.
    pub fn inject_stylesheet(&mut self, css: &str) {
        if css.trim().is_empty() {
            return;
        }
        let base = self.resources.stylesheet_base.clone();
        let attach_loader = stylesheet_base_is_set(&base);
        let loader = FsStylesheetLoader { base: &base };
        let mut cache = std::mem::take(&mut self.resources.import_cache);
        let mut options = ParseStylesheetOptions {
            media: self.media_environment(),
            loader: if attach_loader { Some(&loader) } else { None },
            base_href: None,
            import_cache: Some(&mut cache),
        };
        let (sheet, report) =
            parse_stylesheet_full_with_options(css, self.cascade.next_rule_order, &mut options);
        self.resources.import_cache = cache;
        self.cascade.stylesheet_skips = self.cascade.stylesheet_skips.combine(report);
        if sheet.is_cascade_empty() {
            return;
        }
        if let Some(last) = sheet.max_source_order() {
            self.cascade.next_rule_order = last.saturating_add(1);
        }
        let env = self.media_environment();
        let flattened = sheet.flatten(&env);
        for face in &flattened.font_faces {
            self.consider_font_face(face);
        }
        let new_static = flattened.static_rules;
        self.cascade.authored_sheets.push(sheet);
        self.rebuild_active_stylesheet();
        self.rebuild_stylesheet_vars();
        if stylesheet_needs_relative(&new_static) {
            self.reapply_layout_cascade_all();
        } else {
            self.reapply_layout_cascade_matching(&new_static);
        }
        if self.has_interactive_css() {
            self.reapply_layout_cascade_all();
        } else if self.has_focus_within_css() {
            let focused = self.focused_for_cascade();
            self.reapply_focus_within_ancestors(None, focused);
        }
    }
}

impl MessageBridge {
    pub(super) fn reapply_focus_within_ancestors(
        &mut self,
        previous: Option<WidgetId>,
        next: Option<WidgetId>,
    ) {
        if !self.cascade.uses_focus_within {
            return;
        }
        let mut dirty = HashSet::new();
        for start in [previous, next].into_iter().flatten() {
            let mut walk = Some(start);
            while let Some(id) = walk {
                if !dirty.insert(id) {
                    break;
                }
                walk = self.widgets.get(&id).and_then(|w| w.parent);
            }
        }
        let mut ordered: Vec<WidgetId> = dirty.into_iter().collect();
        ordered.sort_by_cached_key(|id| self.widget_depth(*id));
        for id in &ordered {
            self.reapply_layout_for(*id);
        }
        self.changed_widgets(ordered);
    }
}

impl MessageBridge {
    /// Focus change: `:focus-within` restyles the old/new focus ancestor chains
    /// only. Interactive `:hover`/`:focus`/`:active` still recascade through
    /// [`Self::reapply_interactive_cascade`].
    pub(crate) fn on_runtime_focus_change(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
        previous: Option<WidgetId>,
        next: Option<WidgetId>,
    ) {
        self.cascade.cascade_focused = next;
        if self.has_interactive_css() {
            self.reapply_interactive_cascade(doc);
            return;
        }
        self.discard_interactive_runtime_if_unused();
        if self.has_focus_within_css() {
            self.reapply_focus_within_ancestors(previous, next);
        }
    }
}

impl MessageBridge {
    pub(super) fn focus_within_of(&self, id: WidgetId) -> bool {
        let Some(focused) = self.focused_for_cascade() else {
            return false;
        };
        if focused == id {
            return true;
        }
        let mut cur = self.widgets.get(&focused).and_then(|w| w.parent);
        while let Some(pid) = cur {
            if pid == id {
                return true;
            }
            cur = self.widgets.get(&pid).and_then(|w| w.parent);
        }
        false
    }
}

impl MessageBridge {
    pub(super) fn discard_interactive_runtime_if_unused(&mut self) {
        if !self.has_interactive_css() {
            self.cascade.interactive_runtime = None;
        }
    }
}

impl MessageBridge {
    pub(super) fn focused_for_cascade(&self) -> Option<WidgetId> {
        // `:focus-within` follows the last written focus (`doc.focused()` via
        // [`Self::on_runtime_focus_change`] / interactive collect). The hover
        // snapshot is not a competing authority — it can go stale when media
        // flatten drops the interactive bucket.
        self.cascade.cascade_focused
    }
}

impl MessageBridge {
    pub fn has_focus_within_css(&self) -> bool {
        self.cascade.uses_focus_within
    }
}

impl MessageBridge {
    pub fn has_interactive_css(&self) -> bool {
        !self.cascade.interactive_rules.is_empty()
            || !self.cascade.generated_pseudo_rules.is_empty()
            || !self.cascade.scrollbar_pseudo_rules.is_empty()
            || !self.cascade.motion_rules.is_empty()
            || !self.cascade.keyframes.is_empty()
    }
}
