//! Plain text constraint resolution, layout caching via `nana-text`, and block struts.

use nana_text::TextEngine as _;

use super::*;
use crate::text_node::TextBackendEpoch;
use crate::world::*;

impl UiWorld {
    /// Shape against the last published content box when it exists so wrap
    /// height can stop or propagate LAYOUT. Unmeasured nodes stay unconstrained.
    pub(crate) fn text_shape_constraints(&self, id: StableNodeId) -> crate::TextShapeConstraints {
        self.text_shape_constraints_for(id, self.text_input_kind(id))
    }

    /// Whether an authored newline is a line break for this node.
    ///
    /// A multiline editor's value keeps its line breaks whatever
    /// `white-space` says: they are the text being edited, not authored markup
    /// whitespace. Everywhere else CSS decides.
    ///
    /// One function because two consumers answer it: the constraints this node
    /// is *measured* with, and the scene the renderer *paints* from. A second
    /// spelling would measure one line and paint two.
    fn preserves_lines(style: &NodeStyle, text_input_multiline: bool) -> bool {
        style.layout.white_space.preserve_newlines() || text_input_multiline
    }

    /// [`Self::preserves_lines`] for one node, as the scene carries it.
    pub(crate) fn text_preserves_lines(&self, id: StableNodeId) -> bool {
        Self::preserves_lines(
            &self.record(id).style,
            self.text_input_kind(id).unwrap_or(false) || self.nodes.rich_editor_marks(id).is_some(),
        )
    }

    /// Whether the node is an editor, and whether it is multiline: the only
    /// two things [`Self::text_shape_constraints_for`] needs to know about its
    /// presentation.
    ///
    /// Derived from the node rather than from a built presentation source: a
    /// caret move asks for the constraints it should probe with, and building
    /// a whole presentation (display text, decorations and all) to learn two
    /// booleans is O(document) per key press.
    pub(in crate::world) fn text_input_kind(&self, id: StableNodeId) -> Option<bool> {
        if !self.nodes.has_widget_visuals() {
            return None;
        }
        if self.text_input_visual_ref(id).is_none() || self.nodes.text_input(id).is_none() {
            return None;
        }
        Some(self.nodes.contains(id) && self.nodes.accessibility(id).multiline)
    }

    /// Whether the editor draws bullets instead of its value.
    pub(crate) fn text_input_is_secure(&self, id: StableNodeId) -> bool {
        self.text_input_visual_ref(id).is_some_and(|v| v.secure)
    }

    /// [`Self::text_shape_constraints`] for a caller that already knows
    /// whether the node is an editor and whether it is multiline (the built
    /// presentation source's `multiline`).
    pub(in crate::world) fn text_shape_constraints_for(
        &self,
        id: StableNodeId,
        editor_multiline: Option<bool>,
    ) -> crate::TextShapeConstraints {
        let source = &self.record(id).style;
        let layout = self
            .component_layout_box(id)
            .expect("text node layout is available");
        let text_input_multiline =
            editor_multiline.unwrap_or(false) || self.nodes.rich_editor_marks(id).is_some();
        let is_text_input =
            editor_multiline.is_some() || self.nodes.rich_editor_marks(id).is_some();
        // A card's title is one line and a bare toast's title at most two
        // (#388: one line loses a medium title's meaning, two keep the
        // toast's height bounded), each cut to the lane it paints in. The
        // recipe's `title_max_lines` is the assembled notice's.
        let card_title = self
            .widget_visual_ref(id)
            .is_some_and(crate::WidgetVisualHandle::uses_card_title_typography);
        let clamped = if card_title {
            Some(1)
        } else {
            self.widget_visual_ref(id)
                .and_then(crate::WidgetVisualHandle::clamped_node_text_lines)
        };
        let wrap = if let Some(lines) = clamped {
            lines > 1
        } else if is_text_input {
            text_input_multiline && source.layout.text_wraps()
        } else {
            source.layout.text_wraps()
        };
        let preserve_lines =
            clamped.is_none() && Self::preserves_lines(source, text_input_multiline);
        let wrap_break = source.layout.text_wrap_break();
        let ellipsis = clamped.is_some() || (!is_text_input && source.layout.uses_text_ellipsis());
        let max_lines = if let Some(lines) = clamped {
            Some(u16::try_from(lines).unwrap_or(u16::MAX))
        } else {
            (!is_text_input)
                .then(|| source.layout.resolved_line_clamp())
                .flatten()
        };
        let measured = layout.width > 0.0 || layout.height > 0.0;
        let vertical = self
            .nodes
            .get(id)
            .is_some_and(|node| node.resolved.0.writing_mode.is_vertical());
        if !measured {
            return crate::TextShapeConstraints {
                wrap,
                ellipsis,
                max_lines,
                shaping: self.text_shaping(id),
                preserve_lines,
                wrap_break,
                ..crate::TextShapeConstraints::default()
            };
        }
        let padding = self.used_layout_padding(id);
        let border = source.layout.resolved_border_edges();
        let counter_reserved = self
            .text_input_character_counter(id)
            .map(|counter| {
                self.component_content_box(id)
                    .map_or(0.0, |content| content.width - counter.editor_bounds.width)
            })
            .unwrap_or(0.0);
        crate::TextShapeConstraints {
            max_width: if is_text_input && !text_input_multiline {
                None
            } else {
                let available = (layout.width
                    - padding.left
                    - padding.right
                    - border.left
                    - border.right
                    - counter_reserved)
                    .max(0.0);
                let content = crate::layout_engine::Size::new(
                    available,
                    (layout.height - padding.top - padding.bottom - border.top - border.bottom)
                        .max(0.0),
                );
                Some(
                    self.widget_visual_ref(id)
                        .and_then(|v| {
                            v.node_text_max_width(id, self, layout, available, content.height)
                        })
                        .unwrap_or(available),
                )
            },
            // A vertical paragraph's lines run down the box, so its height is
            // the line budget the way a horizontal one's width is: always
            // given once the box is measured. A vertical editor wraps down its
            // height when it wraps at all — a multiline one, as a horizontal
            // editor wraps across its width (#59).
            max_height: if card_title {
                self.content_part_box(id, crate::LayoutPartKind::Header)
                    .map(|part| part.height.max(0.0))
            } else {
                ((vertical && (!is_text_input || text_input_multiline))
                    || (!is_text_input
                        && (self
                            .presentation_length_overlay(id)
                            .and_then(|overlay| overlay.get(crate::AnimatableProperty::Height))
                            .is_some()
                            || source
                                .layout
                                .height
                                .is_some_and(nana_ui_core::LengthSpec::is_definite_declared)
                            || source
                                .layout
                                .max_height
                                .is_some_and(nana_ui_core::LengthSpec::is_definite_declared))))
                .then(|| {
                    (layout.height - padding.top - padding.bottom - border.top - border.bottom)
                        .max(0.0)
                })
            },
            wrap,
            ellipsis,
            max_lines,
            shaping: self.text_shaping(id),
            preserve_lines,
            wrap_break,
        }
    }
}

/// Ellipsized text paints within its measured box, but asks layout for its
/// unclipped natural width. This includes line-clamp: it enables wrapping,
/// then substitutes an ellipsis on the last visible line. Exporting that
/// shortened line's width would pin a shrink-to-fit parent to its old size.
pub(super) fn cut_to_its_box(constraints: &crate::TextShapeConstraints) -> bool {
    constraints.ellipsis
        && (constraints.max_lines.is_some() || (!constraints.wrap && !constraints.preserve_lines))
        && constraints.max_width.is_some()
}

/// Text that wraps to its measured box: its lines may be narrower than the
/// text is on its own.
pub(super) fn wraps_to_its_box(constraints: &crate::TextShapeConstraints) -> bool {
    constraints.wrap && constraints.max_width.is_some()
}

/// One plain text node's resolution: [`UiWorld::resolve_plain_text`].
pub(in crate::world) struct PlainText {
    pub(super) metrics: TextMetrics,
    /// Its unwrapped width, when it wrapped narrower than that.
    pub(super) natural: Option<f32>,
    /// The `nana-text` layout it retains, with an engine.
    pub(super) layout: Option<Arc<nana_text::TextLayout>>,
    /// Resolved to its unwrapped lines, not to its box's width, which only
    /// held the text it replaced.
    pub(super) provisional: bool,
    /// Through the host shaper, whose widest line cannot break
    /// ([`UiWorld::host_widest_line_cannot_break`]). An engine's layout says
    /// so itself.
    pub(super) host_unbroken: bool,
}

/// The same constraints without the box width: the line as long as it runs.
pub(super) fn unbounded(constraints: crate::TextShapeConstraints) -> crate::TextShapeConstraints {
    crate::TextShapeConstraints {
        max_width: None,
        ..constraints
    }
}

/// True when two authored styles give a plain text node the same
/// constraints for the same box: everything [`UiWorld::text_shape_constraints`]
/// reads from [`NodeStyle`]. Alignment only matters to a retained layout and is
/// handled where the style is set. Paint, transform and
/// opacity live on the same `LayoutStyle` and are deliberately not compared;
/// fonts are classified when the computed style resolves.
pub(in crate::world) fn same_text_constraint_layouts(
    previous: &Arc<nana_ui_core::LayoutStyle>,
    next: &Arc<nana_ui_core::LayoutStyle>,
) -> bool {
    if Arc::ptr_eq(previous, next) {
        return true;
    }
    let (a, b) = (previous.as_ref(), next.as_ref());
    let definite = |length: Option<nana_ui_core::LengthSpec>| {
        length.is_some_and(nana_ui_core::LengthSpec::is_definite_declared)
    };
    a.text_wraps() == b.text_wraps()
        && a.white_space.preserve_newlines() == b.white_space.preserve_newlines()
        && a.text_wrap_break() == b.text_wrap_break()
        && a.uses_text_ellipsis() == b.uses_text_ellipsis()
        && a.resolved_line_clamp() == b.resolved_line_clamp()
        && definite(a.height) == definite(b.height)
        && definite(a.max_height) == definite(b.max_height)
        && a.resolved_border_edges() == b.resolved_border_edges()
        && a.padding == b.padding
        && a.padding_top == b.padding_top
        && a.padding_right == b.padding_right
        && a.padding_bottom == b.padding_bottom
        && a.padding_left == b.padding_left
        && a.font_size.map(f32::to_bits) == b.font_size.map(f32::to_bits)
}

impl UiWorld {
    /// True, and counted as a revision skip, when `id` is plain text already
    /// resolved by `backend` at its current revisions. Reads one small entry of
    /// the text side table and no text.
    pub(super) fn plain_text_is_current(
        &self,
        id: StableNodeId,
        backend: TextBackendEpoch,
        work: &mut nana_text::TextWorkCounters,
    ) -> bool {
        let Some(text) = self.nodes.text_node(id) else {
            return self.nodes.is_empty_text_current(id, backend);
        };
        if !text.is_current(backend) {
            return false;
        }
        let stamp = text.stamp().expect("a current node is stamped");
        // A resolved box without text is not a text node; only text nodes count.
        if stamp.text_node {
            work.text_nodes_considered += 1;
            work.text_nodes_revision_skipped += 1;
            #[cfg(any(test, feature = "benchmark"))]
            {
                crate::text_shape_stats::note_nonempty();
                crate::text_shape_stats::note_skipped_unchanged();
            }
        }
        #[cfg(debug_assertions)]
        if let Some(constraints) = stamp.constraints {
            // A revision that should have moved and did not would leave this
            // node on stale constraints. Catch the missing invalidation where
            // it is cheap to explain rather than as a wrong wrap on screen.
            let (recorded, alignment) = constraints;
            if !text.layout.is_null() {
                debug_assert_eq!(
                    alignment,
                    self.record(id).style.text_horizontal_alignment,
                    "text node {id:?} retains a layout at a stale alignment"
                );
            }
            debug_assert_eq!(
                recorded,
                self.text_shape_constraints(id),
                "text node {id:?} is current by revision but its constraints changed: \
                 a CONSTRAINT invalidation is missing"
            );
        }
        true
    }

    /// Resolves one plain (non-editor) text node at its current revisions; the
    /// caller stamps it once the metrics validate. With an engine a node with
    /// text retains the `nana-text` layout its metrics were read from; without
    /// one the host shaper measures it. Text that wrapped to its box also
    /// reports its unwrapped width when that is wider (see
    /// [`UiWorld::text_natural_width`]).
    ///
    /// Text shaped anew in a box only as wide as the text it replaces (see
    /// [`Self::shapes_anew_in_a_hugging_box`]) is not held to that width:
    /// when it wraps there, it resolves to its unwrapped lines instead,
    /// provisionally, and layout gives it the room the box really has. A
    /// longer label in a wide column lays out once, on the line it fits,
    /// rather than tall for a pass that moves everything after it down and
    /// back.
    pub(super) fn resolve_plain_text<S: TextShaper>(
        &mut self,
        id: StableNodeId,
        constraints: crate::TextShapeConstraints,
        engine: Option<&nana_text::SharedTextEngine>,
        shaper: &mut CountingShaper<'_, S>,
        work: &mut nana_text::TextWorkCounters,
    ) -> PlainText {
        let base_style = Arc::clone(&self.record(id).resolved.0);
        // The title has component-owned typography, while the Card's base
        // style continues to inherit normally into its body children.
        let title_style = self
            .widget_visual_ref(id)
            .is_some_and(crate::WidgetVisualHandle::uses_card_title_typography)
            .then(|| {
                let typography = self.node_theme(id).typography();
                let mut style = base_style.as_ref().clone();
                style.font_size = typography.body;
                style.font_weight = Some(typography.semibold);
                style.line_height =
                    Some(nana_ui_core::LineHeightSpec::Absolute(typography.line_tall));
                style
            });
        let style = title_style.as_ref().unwrap_or(base_style.as_ref());
        let text_bytes = self.nodes.text_content(id).value.len();
        // A box without text that a scheduled pass still measures is not a
        // text node: its measurement is counted, but not as a node.
        let is_text_node =
            text_bytes > 0 || matches!(self.record(id).kind.as_ref(), NodeKind::Text);
        // Read before the engine borrows the node's source.
        let hugged = self.shapes_anew_in_a_hugging_box(id, &constraints);
        let mut node_work = nana_text::TextWorkCounters::default();
        let mut resolved = match engine {
            Some(engine) => {
                let alignment = self.record(id).style.text_horizontal_alignment;
                let (source, copied) = self
                    .nodes
                    .text_source(id, style)
                    .expect("a resolved node exists");
                // Locked per resolution, never across the pass: the same pass
                // measures component text through the host's `shape`, which
                // may lay out through this very engine.
                let kind = crate::text_node::text_kind(&constraints);
                let layout = nana_text::lock_text_engine(engine).layout(
                    kind,
                    source,
                    &crate::text_node::nana_text_style(style),
                    &crate::text_node::nana_text_constraints(style, &constraints, alignment),
                    &mut node_work,
                );
                let mut metrics = crate::text_node::text_metrics_of_layout(&layout);
                // A capped paragraph's last line can be ellipsized instead of
                // reporting a soft wrap. Keep its natural width separately
                // from the measured visible lines, just as for ordinary wrap.
                let unwrapped = (!layout.is_vertical()
                    && ((cut_to_its_box(&constraints)
                        && layout
                            .overflow
                            .contains(nana_text::OverflowFlags::ELLIPSIZED))
                        || (wraps_to_its_box(&constraints)
                            && layout
                                .lines
                                .iter()
                                .any(|line| line.break_cause == nana_text::LineBreakCause::Wrap))))
                .then(|| {
                    let natural = nana_text::lock_text_engine(engine).layout(
                        kind,
                        source,
                        &crate::text_node::nana_text_style(style),
                        &crate::text_node::nana_text_constraints(
                            style,
                            &unbounded(constraints),
                            alignment,
                        ),
                        &mut node_work,
                    );
                    (crate::text_node::text_metrics_of_layout(&natural), natural)
                })
                .filter(|(natural, _)| !constraints.wrap || natural.width > metrics.width);
                let unwrapped = if !constraints.wrap {
                    if let Some((natural, _)) = unwrapped {
                        metrics.width = natural.width;
                    }
                    None
                } else {
                    unwrapped
                };
                if copied {
                    node_work.text_source_clones += 1;
                    self.record_string_clone(text_bytes);
                }
                let (metrics, natural, layout, provisional) = match unwrapped {
                    Some((natural, unwrapped)) if hugged => (natural, None, unwrapped, true),
                    unwrapped => (
                        metrics,
                        unwrapped.map(|(natural, _)| natural.width),
                        layout,
                        false,
                    ),
                };
                // An empty Text node still has a line box to measure, but
                // nothing to draw: it retains no layout. An empty rich editor
                // keeps its one empty line: the caret is drawn on it and the
                // IME anchors to it.
                let retained = text_bytes > 0 || self.nodes.rich_editor_marks(id).is_some();
                PlainText {
                    metrics,
                    natural,
                    layout: retained.then_some(layout),
                    provisional,
                    host_unbroken: false,
                }
            }
            None => {
                let runs = shaper.runs;
                let constraint_only = self.nodes.text_node(id).is_some_and(
                    crate::text_node::TextNodeState::constraint_moved_without_reshaping,
                );
                let mut metrics = shaper.shape(id, self.nodes.text_content(id), style, constraints);
                let lines_tall = crate::components::resolved_text_line_height(style) * 1.5;
                // Host-only shapers expose no overflow flags. A clamped run
                // can end short of its budget because the ellipsis has width.
                // Some size-only host shapers cap even a no-wrap run. When
                // chrome takes space from a control's old box, preserve the
                // label's intrinsic width so auto sizing can grow that box.
                let control_capped = !constraints.wrap
                    && self
                        .widget_visual_ref(id)
                        .is_some_and(crate::WidgetVisualHandle::caps_control_text)
                    && constraints
                        .max_width
                        .is_some_and(|max| metrics.width >= max - 0.5);
                let needs_natural = control_capped
                    || (cut_to_its_box(&constraints)
                        && (constraints.max_lines.is_some()
                            || constraints
                                .max_width
                                .is_some_and(|max| metrics.width >= max - 0.5)))
                    || (wraps_to_its_box(&constraints) && metrics.height > lines_tall);
                let unwrapped = needs_natural
                    .then(|| {
                        shaper.shape(
                            id,
                            self.nodes.text_content(id),
                            style,
                            unbounded(constraints),
                        )
                    })
                    .filter(|natural| !constraints.wrap || natural.width > metrics.width);
                let unwrapped = if !constraints.wrap {
                    if let Some(natural) = unwrapped {
                        metrics.width = natural.width;
                    }
                    None
                } else {
                    unwrapped
                };
                if constraint_only {
                    // `shape` still runs; a width constraint is not a new shaped run.
                    shaper.runs = runs;
                }
                node_work.record_text_pass(1, usize::from(shaper.runs > runs));
                let (metrics, natural, provisional) = match unwrapped {
                    Some(natural) if hugged => (natural, None, true),
                    unwrapped => (metrics, unwrapped.map(|natural| natural.width), false),
                };
                let host_unbroken = !provisional
                    && self.host_widest_line_cannot_break(id, style, constraints, metrics, shaper);
                // No layout: one from an engine this host no longer offers is
                // not what the host measures now.
                PlainText {
                    metrics,
                    natural,
                    layout: None,
                    provisional,
                    host_unbroken,
                }
            }
        };
        // A select that sizes to its options asks layout for the widest one,
        // whichever it shows.
        if let Some(widest) = self.widest_fitted_option(id, style, constraints, shaper)
            && widest > resolved.metrics.width
        {
            resolved.natural = Some(
                resolved
                    .natural
                    .map_or(widest, |natural| natural.max(widest)),
            );
        }
        if !is_text_node {
            node_work.text_nodes_considered = 0;
            node_work.text_nodes_shaped = 0;
        }
        work.accumulate(node_work);
        resolved
    }

    /// The widest option label of a select that sizes to its options
    /// ([`crate::Select::fit_options`]), measured on one line in the field's
    /// own style, the way its label is; an option with a hint holds the hint
    /// after it too, which the field draws a step smaller, so it never comes
    /// short. `None` for any other node.
    fn widest_fitted_option<S: TextShaper>(
        &self,
        id: StableNodeId,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
        shaper: &mut CountingShaper<'_, S>,
    ) -> Option<f32> {
        let Some(crate::SelectVisual {
            fit_options: true,
            options,
            ..
        }) = self.widget_visual_as::<crate::SelectVisual>(id)
        else {
            return None;
        };
        let line = crate::TextShapeConstraints {
            max_width: None,
            wrap: false,
            ellipsis: false,
            max_lines: None,
            ..constraints
        };
        options
            .iter()
            .map(|option| {
                let label = TextContent::new(Arc::clone(&option.label));
                let width = shaper.shape(id, &label, style, line).width;
                match option.hint.as_ref().filter(|hint| !hint.is_empty()) {
                    Some(hint) => {
                        let hint = TextContent::new(Arc::clone(hint));
                        width + crate::select::HINT_GAP + shaper.shape(id, &hint, style, line).width
                    }
                    None => width,
                }
            })
            .reduce(f32::max)
    }

    /// Whether `id` shapes anew (new content, font, shaping style or
    /// language) in a wrapping box only as wide as the text it held: lines
    /// that did not wrap and filled it. That width was the old text's, not
    /// room the box was given -- a box that shrinks to its content has what
    /// its parent offers, which only layout knows -- so it is no width to
    /// wrap new lines to.
    ///
    /// A paragraph that wrapped fills the room it has: its box's width is a
    /// constraint, and it keeps wrapping to it.
    fn shapes_anew_in_a_hugging_box(
        &self,
        id: StableNodeId,
        constraints: &crate::TextShapeConstraints,
    ) -> bool {
        let Some(width) = constraints.max_width.filter(|_| constraints.wrap) else {
            return false;
        };
        self.nodes
            .text_node(id)
            .is_some_and(crate::text_node::TextNodeState::reshapes)
            && self.text_natural_width(id).is_none()
            && (self.nodes.text_metrics(id).width - width).abs() <= 0.5
    }

    /// Applies a successful pass's plain text resolutions: retains or releases
    /// each node's layout, then stamps it at the revisions it was resolved at.
    /// Nothing is applied for a pass that failed, so a retry resolves the same
    /// nodes again rather than skipping them with metrics never written.
    /// Returns the text among them a typography scale change had reached, and
    /// the text whose widest line was found not to break, or to break after
    /// all: the width it asks of layout moved, whether or not its metrics did.
    pub(super) fn apply_plain_resolutions(
        &mut self,
        resolutions: Vec<PlainResolution>,
        backend: TextBackendEpoch,
        work: &mut nana_text::TextWorkCounters,
    ) -> (HashSet<StableNodeId>, HashSet<StableNodeId>, bool) {
        let mut scaled = HashSet::new();
        let mut unbreakable_moved = HashSet::new();
        let mut fragments_changed = Vec::new();
        let mut inline_placement_changed = false;
        let mut max_text_id: Option<StableNodeId> = None;
        let mut text_count = 0usize;
        for resolution in &resolutions {
            let needs_text_node = resolution.layout.is_some()
                || self.nodes.get(resolution.id).is_some_and(|record| {
                    record.has_text || matches!(record.kind.as_ref(), NodeKind::Text)
                });
            if needs_text_node {
                text_count += 1;
                max_text_id =
                    Some(max_text_id.map_or(resolution.id, |current| current.max(resolution.id)));
            }
        }
        if let Some(highest) = max_text_id {
            self.nodes
                .reserve_text_nodes_through(highest, self.nodes.text_nodes_len() + text_count);
        }
        for resolution in resolutions {
            fragments_changed.push(resolution.id);
            // What the metrics this resolution wrote were wrapped against:
            // the box width, or none yet. Layout reads it to know whether the
            // width of those lines is one this node keeps in a given box. A
            // widest line that cannot break is that wide in every box, so it
            // counts as wrapped against less than any.
            let unbreakable = |limit: Option<f32>| limit == Some(f32::NEG_INFINITY);
            let previous = self.text_wrap_limit(resolution.id);
            let wrap_limit = resolution
                .constraints
                .filter(|constraints| constraints.wrap)
                .map(|constraints| {
                    if resolution.host_unbroken
                        || self.widest_line_cannot_break(
                            resolution.id,
                            &constraints,
                            resolution.layout.as_deref(),
                            unbreakable(previous),
                            work,
                        )
                    {
                        f32::NEG_INFINITY
                    } else {
                        constraints.max_width.unwrap_or(f32::INFINITY)
                    }
                });
            if unbreakable(previous) != unbreakable(wrap_limit) {
                unbreakable_moved.insert(resolution.id);
            }
            let objects_moved = self.inline_record(resolution.id).is_some()
                && self
                    .text_layout(resolution.id)
                    .zip(resolution.layout.as_ref())
                    .is_some_and(|((_, old), new)| old.objects != new.objects);
            if objects_moved {
                self.record_layout_invalidation(
                    resolution.id,
                    nana_ui_core::LayoutInvalidation::new(
                        nana_ui_core::LayoutInvalidationSource::Text,
                        nana_ui_core::InvalidationReason::TEXT,
                        nana_ui_core::InvalidationKind::PLACEMENT,
                        nana_ui_core::LayoutStyleChange::FLOW,
                        nana_ui_core::LayoutDependencyFootprint::NONE,
                    ),
                );
                inline_placement_changed = true;
            }
            match resolution.layout {
                Some(layout) => {
                    if self.nodes.retain_text_layout(resolution.id, layout) {
                        work.text_layouts_reused += 1;
                    }
                }
                None => self.nodes.release_text_layout(resolution.id),
            }
            self.nodes.set_text_wrap_limit(resolution.id, wrap_limit);
            if self
                .nodes
                .mark_text_resolved(resolution.id, backend, resolution.constraints)
            {
                self.bump_last_counters(|counters| counters.record_text_language(0, 1));
            }
            if resolution.provisional {
                // Not resolved at its box: stamped one constraint revision
                // behind, so the pass after layout, which its new metrics
                // reach, lays it out again at the box layout gave it.
                self.nodes
                    .invalidate_text(resolution.id, crate::text_node::TextDirty::CONSTRAINT);
            }
            if self.nodes.take_text_scale_pending(resolution.id) {
                scaled.insert(resolution.id);
            }
        }
        self.publish_layout_results(&fragments_changed, crate::LayoutResultSource::RuntimeLayout);
        (scaled, unbreakable_moved, inline_placement_changed)
    }

    /// Whether the widest line `id` was just laid out to cannot break, so the
    /// text is that wide in a box of any width (see
    /// [`UiWorld::text_wrap_limit`]).
    ///
    /// A line wider than the box it wrapped to cannot: it would have broken.
    /// Once its box has grown to hold that line, the line fits and shows
    /// nothing; laid out half a pixel narrower, a line that cannot break
    /// sticks out again just as wide. Only text whose widest line could not
    /// break when it last resolved (`unbroken_before`) is laid out that
    /// second time, so a label as wide as its line costs nothing more. Only
    /// an engine's layout says where its lines stuck out; a host shaper's
    /// metrics say it of the widest line
    /// ([`Self::host_widest_line_cannot_break`]).
    fn widest_line_cannot_break(
        &mut self,
        id: StableNodeId,
        constraints: &crate::TextShapeConstraints,
        layout: Option<&nana_text::TextLayout>,
        unbroken_before: bool,
        work: &mut nana_text::TextWorkCounters,
    ) -> bool {
        let Some(layout) = layout.filter(|layout| !layout.is_vertical()) else {
            return false;
        };
        if constraints.max_width.is_none() {
            return false;
        }
        if layout
            .overflow
            .contains(nana_text::OverflowFlags::CLIPPED_WIDTH)
        {
            return true;
        }
        let engine = match &self.paint_text_engine {
            Some(engine) if unbroken_before => Arc::clone(engine),
            _ => return false,
        };
        let widest = crate::text_node::text_metrics_of_layout(layout).width;
        let inside = crate::TextShapeConstraints {
            max_width: Some((widest - 0.5).max(0.0)),
            ..*constraints
        };
        let style = Arc::clone(&self.record(id).resolved.0);
        let alignment = self.record(id).style.text_horizontal_alignment;
        let Some((source, _)) = self.nodes.text_source(id, &style) else {
            return false;
        };
        let narrower = nana_text::lock_text_engine(&engine).layout(
            crate::text_node::text_kind(&inside),
            source,
            &crate::text_node::nana_text_style(&style),
            &crate::text_node::nana_text_constraints(&style, &inside, alignment),
            work,
        );
        crate::text_node::text_metrics_of_layout(&narrower).width >= widest - 0.01
    }

    /// [`Self::widest_line_cannot_break`] for text the host shaper measured
    /// at `constraints`, from what it measured (`metrics`): a host reports
    /// no overflow, but a widest line wider than the box it wrapped to
    /// stuck out of it. Text whose widest line could not break when it last
    /// resolved is measured once more, half a pixel narrower than that
    /// line, and still cannot break when it comes out as wide.
    ///
    /// Without this, text a host measured wider than its box asks layout
    /// for the box's width before it is shaped there and for its own width
    /// after (`text_inline_size` counts lines wrapped no wider than the
    /// box), with the same metrics: no seed reaches layout, the frame keeps
    /// the narrower box, and a later pass that measures the text again
    /// widens it while one that reuses a retained measurement does not.
    fn host_widest_line_cannot_break<S: TextShaper>(
        &self,
        id: StableNodeId,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
        metrics: TextMetrics,
        shaper: &mut CountingShaper<'_, S>,
    ) -> bool {
        if style.writing_mode.is_vertical() || !constraints.wrap {
            return false;
        }
        let Some(max_width) = constraints.max_width else {
            return false;
        };
        if metrics.width > max_width + 0.5 {
            return true;
        }
        if self.text_wrap_limit(id) != Some(f32::NEG_INFINITY) {
            return false;
        }
        let inside = crate::TextShapeConstraints {
            max_width: Some((metrics.width - 0.5).max(0.0)),
            ..constraints
        };
        shaper
            .shape(id, self.nodes.text_content(id), style, inside)
            .width
            >= metrics.width - 0.01
    }
}

pub(in crate::world) struct PlainResolution {
    id: StableNodeId,
    /// The constraints a node with text was resolved at. `None` for a box with
    /// no text, whose constraints nothing reads.
    constraints: Option<crate::TextShapeConstraints>,
    /// The layout to retain; `None` releases whatever the node held.
    layout: Option<Arc<nana_text::TextLayout>>,
    /// Resolved without its box's width ([`PlainText::provisional`]).
    provisional: bool,
    /// [`PlainText::host_unbroken`].
    host_unbroken: bool,
}

impl PlainResolution {
    /// A node with text, resolved in a box that gives it `constraints`, or
    /// `provisional`ly without that box's width.
    pub(super) fn text(
        id: StableNodeId,
        constraints: crate::TextShapeConstraints,
        layout: Option<Arc<nana_text::TextLayout>>,
        provisional: bool,
        host_unbroken: bool,
    ) -> Self {
        Self {
            id,
            constraints: Some(if provisional {
                unbounded(constraints)
            } else {
                constraints
            }),
            layout,
            provisional,
            host_unbroken,
        }
    }

    pub(super) fn nothing(id: StableNodeId) -> Self {
        Self {
            id,
            constraints: None,
            layout: None,
            provisional: false,
            host_unbroken: false,
        }
    }
}

/// How plain text resolves for one pass, read once from the host.
pub(in crate::world) struct PlainTextBackend {
    pub epoch: TextBackendEpoch,
    pub engine: Option<nana_text::SharedTextEngine>,
    /// The engine's fallback language. Text depends on it through its
    /// computed style, like on any other language, not through the epoch.
    pub fallback_language: Option<nana_text::font::LanguageTag>,
}

impl PlainTextBackend {
    pub(in crate::world) fn of<H: TextShaper + ?Sized>(host: &H) -> Self {
        match host.text_engine() {
            Some(engine) => {
                let (epoch, fallback_language) = {
                    let engine = nana_text::lock_text_engine(&engine);
                    (engine.epoch(), engine.language().cloned())
                };
                // The fallback language reaches text as the root of the
                // language its style inherits, so a change of it re-resolves
                // the text that inherits it and nothing else. Kept in the
                // epoch, it would make every resolved node stale.
                let epoch = nana_text::TextEngineEpoch {
                    language_generation: 0,
                    ..epoch
                };
                Self {
                    epoch: TextBackendEpoch::Engine(epoch),
                    engine: Some(engine),
                    fallback_language,
                }
            }
            None => Self {
                epoch: TextBackendEpoch::Host {
                    shaper: {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::hash::DefaultHasher::new();
                        std::any::type_name::<H>().hash(&mut hasher);
                        hasher.finish()
                    },
                    font_generation: host.font_generation(),
                },
                engine: None,
                fallback_language: None,
            },
        }
    }
}

impl UiWorld {
    /// The strut `id`'s line boxes start from, per px of its font size: its
    /// own face's ascent and descent, as the shaper reported them. `None`
    /// keeps layout's approximation.
    pub(crate) fn block_strut(&self, id: StableNodeId) -> Option<crate::FontStrut> {
        self.nodes.block_strut(id).copied()
    }

    /// Whether `id` lays its children's lines out with a strut of its own: an
    /// ordinary block (not a flex or grid container) with no line box of its
    /// own whose children include an inline-level one in flow (CSS 2.1
    /// §10.8.1). It reads `id`'s kind, text and display, and its children's
    /// display, position and float; each of those changing makes `id` a
    /// strut candidate.
    pub(crate) fn wants_block_strut(&self, id: StableNodeId) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let style = &record.resolved_layout;
        if matches!(record.kind.as_ref(), NodeKind::Text)
            || self.lays_out_own_line(id)
            || style.omits_box()
            || style
                .display
                .is_some_and(|display| display.is_flex_container() || display.is_grid_container())
        {
            return false;
        }
        record.hierarchy.children.iter().any(|&child| {
            self.nodes.get(child).is_some_and(|child| {
                let child = &child.resolved_layout;
                child.is_inline_level() && !child.position.is_out_of_flow()
            })
        })
    }

    /// The strut `id` holds when it [wants one](Self::wants_block_strut):
    /// the shaper's metrics of its computed font's own face, per px of its
    /// font size. A size change alone then lays the block out as before
    /// without asking the shaper again.
    fn fresh_block_strut(
        &self,
        id: StableNodeId,
        shaper: &mut impl TextShaper,
    ) -> Option<crate::FontStrut> {
        if !self.wants_block_strut(id) {
            return None;
        }
        let style = &self.nodes.get(id)?.resolved.0;
        let size = style.font_size;
        if !(size.is_finite() && size > 0.0) {
            return None;
        }
        let metrics = shaper.strut_metrics(style)?;
        let per_px = crate::FontStrut {
            ascent: metrics.ascent / size,
            descent: metrics.descent / size,
        };
        (per_px.ascent.is_finite() && per_px.descent.is_finite()).then_some(per_px)
    }

    /// Considers again the strut of every block that may have gained, lost
    /// or changed one: the scheduled text (a box whose text came or went),
    /// and the candidates since the last pass (a block whose font or child
    /// list changed, or a child whose display did; every holder after a
    /// font set change). A strut that moved lays its block out again, the
    /// way moved text metrics do.
    pub(super) fn resolve_block_struts(
        &mut self,
        scheduled: &[StableNodeId],
        shaper: &mut impl TextShaper,
    ) {
        if self.text.strut_candidates.is_empty() && scheduled.is_empty() {
            return;
        }
        let mut candidates = std::mem::take(&mut self.text.strut_candidates);
        candidates.extend_from_slice(scheduled);
        candidates.sort_unstable();
        candidates.dedup();
        for id in candidates {
            if !self.contains(id) {
                continue;
            }
            let next = self.fresh_block_strut(id, shaper);
            if self.nodes.block_strut(id).copied() == next {
                continue;
            }
            self.nodes.set_block_strut(id, next);
            self.note_layout_source_change();
            if self.presence_live(id) {
                self.propagate_layout_from_node(id);
            }
        }
        #[cfg(any(test, feature = "layout-verify"))]
        self.audit_block_struts(shaper);
    }

    /// Every block's retained strut is the one it would read fresh: what
    /// the candidates missed would lay a block out with a stale strut that
    /// no frame corrects. Checked on worlds small enough to scan.
    #[cfg(any(test, feature = "layout-verify"))]
    fn audit_block_struts(&self, shaper: &mut impl TextShaper) {
        const AUDIT_LIMIT: usize = 512;
        if self.nodes.len() > AUDIT_LIMIT {
            return;
        }
        let ids = self.nodes.keys().collect::<Vec<_>>();
        for id in ids {
            assert_eq!(
                self.nodes.block_strut(id).copied(),
                self.fresh_block_strut(id, shaper),
                "{id:?}: the retained strut disagrees with a fresh read"
            );
        }
    }
}
