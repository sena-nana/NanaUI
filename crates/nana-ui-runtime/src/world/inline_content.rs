use super::*;
use crate::{InlineContent, inline_content::InlineRecord};

impl UiWorld {
    /// A single plain line keeps the original two box fragments. Its native
    /// layout remains the glyph/caret authority; it need not republish layout
    /// boxes when only the string's advance changes inside the same box.
    pub(super) fn needs_native_text_fragments(&self, id: StableNodeId) -> bool {
        self.text_layout(id).is_some_and(|(_, layout)| {
            self.inline_record(id).is_some()
                || self.nodes.rich_text(id).is_some_and(|rich| {
                    !rich.objects().is_empty()
                        || !rich.rubies().is_empty()
                        || rich.paragraph_alignments().iter().any(Option::is_some)
                        || rich
                            .spans()
                            .iter()
                            .any(|(_, style)| !style.shape.is_empty() || !style.emphasis.is_empty())
                })
                || self.nodes.text_input(id).is_some()
                || self.nodes.rich_editor_marks(id).is_some()
                || layout.lines.len() != 1
                || layout.is_vertical()
                || layout.constraints.base_direction.is_rtl()
                || layout.constraints.ellipsis
                || layout.constraints.max_lines.is_some()
        })
    }
    pub(crate) fn single_text_baseline(&self, id: StableNodeId, bounds: LayoutBox) -> Option<f32> {
        let node = self.nodes.get(id)?;
        let metrics = self.nodes.text_metrics(id);
        if self.text_layout(id).is_none() {
            return metrics.ascent.map(|ascent| bounds.y + ascent.max(0.0));
        }
        let origin =
            crate::text_layout_origin(bounds, node.style.text_vertical_alignment, metrics.height);
        metrics.ascent.map(|ascent| origin[1] + ascent.max(0.0))
    }
    pub(super) fn native_text_fragments(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
    ) -> Option<(Vec<crate::LayoutFragment>, Option<f32>, Option<f32>)> {
        let (_, layout) = self.text_layout(id)?;
        let origin = self.native_text_origin(id, bounds, layout);
        let mut fragments = Vec::with_capacity(layout.lines.len() + layout.runs.len());
        let to_box = |rect: nana_text::TextRect| {
            let rect = layout.page_rect(rect, bounds.width);
            LayoutBox {
                x: origin[0] + rect.x,
                y: origin[1] + rect.y,
                width: rect.width,
                height: rect.height,
            }
        };
        let mut first = None;
        let mut last = None;
        for line in &layout.lines {
            let baseline =
                (!layout.is_vertical()).then_some(origin[1] + line.metrics.baseline_y_px);
            first = first.or(baseline);
            last = baseline;
            let mut fragment = crate::LayoutFragment::new(
                crate::LayoutFragmentKind::TextLine,
                to_box(line.bounds),
            );
            fragment.index = line.index as usize;
            fragment.source = Some(line.source.clone());
            fragment.first_baseline = baseline;
            fragment.last_baseline = baseline;
            fragments.push(fragment);
            for index in line.runs.clone() {
                let run = &layout.runs[index as usize];
                if run.is_object() {
                    continue;
                }
                let rect = nana_text::TextRect::new(
                    run.origin_x_px,
                    line.metrics.baseline_y_px - run.metrics.ascent_px,
                    run.advance_px,
                    run.metrics.ascent_px + run.metrics.descent_px,
                );
                let mut fragment =
                    crate::LayoutFragment::new(crate::LayoutFragmentKind::TextRun, to_box(rect));
                fragment.index = index as usize;
                fragment.source = Some(run.source.clone());
                fragment.first_baseline = baseline;
                fragment.last_baseline = baseline;
                fragments.push(fragment);
            }
        }
        Some((fragments, first, last))
    }
    /// Controls position a tight content group before painting the paragraph.
    /// Native line coordinates may still carry alignment in the wider original
    /// constraint. Remove that shared offset once, for text and atomic boxes.
    pub(crate) fn native_text_origin(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
        layout: &nana_text::TextLayout,
    ) -> [f32; 2] {
        let mut origin = if layout.is_vertical() {
            [bounds.x, bounds.y]
        } else {
            crate::text_layout_origin(
                bounds,
                self.record(id).style.text_vertical_alignment,
                layout.physical_size().1,
            )
        };
        if !layout.is_vertical()
            && layout.constraints.max_width_px != Some(bounds.width)
            && self
                .widget_visual_ref(id)
                .is_some_and(crate::WidgetVisualHandle::offsets_overflowing_inline_text)
        {
            origin[0] -= layout.bounds.x;
        }
        origin
    }
    pub(crate) fn inline_object_margin(
        &self,
        child: StableNodeId,
        inline_size: f32,
    ) -> nana_ui_core::PaddingSpec {
        self.layout_style(child)
            .map_or_else(Default::default, |style| {
                style.resolved_margin_against_fonts(
                    Some(inline_size),
                    nana_ui_core::FontSizeContext::new(
                        nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX,
                        self.computed_style(child)
                            .map_or(nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX, |style| {
                                style.font_size
                            }),
                    ),
                )
            })
    }
    /// Whether `id` aligns by its border-box bottom rather than a text
    /// baseline: a custom renderer, a content image, a skipped replaced
    /// element or an image viewer. The one rule every context's baseline
    /// reads, so inline, flex and grid alignment cannot disagree on it.
    pub(crate) fn aligns_by_bottom_edge(
        &self,
        id: StableNodeId,
        style: &nana_ui_core::LayoutStyle,
    ) -> bool {
        self.custom_render(id).is_some()
            || style.paint.content_image.is_some()
            || style.paint.skipped_replaced.is_some()
            || self
                .widget_visual_ref(id)
                .is_some_and(crate::WidgetVisualHandle::aligns_by_bottom_edge)
    }

    /// Whether `id` is a non-subgrid flex or grid container that holds at least
    /// one in-flow item (synthesizing a first baseline even if the item has no
    /// line box, CSS Flexbox §8.5 / CSS Grid §11.8).
    pub(crate) fn has_in_flow_flex_or_grid_item(
        &self,
        id: StableNodeId,
        style: &nana_ui_core::LayoutStyle,
    ) -> bool {
        style
            .display
            .is_some_and(|d| d.is_flex_container() || d.is_grid_container())
            && !style.is_subgrid_columns()
            && !style.is_subgrid_rows()
            && !self.lays_out_own_line(id)
            && !self.aligns_by_bottom_edge(id, style)
            && self.child_ids(id).iter().any(|&child| {
                self.layout_style(child)
                    .is_some_and(|cs| !cs.omits_box() && !cs.position.is_out_of_flow())
            })
    }

    /// Whether atomic inline `id` sits on its line by a content baseline
    /// (`inline-flex` / `inline-grid` first baseline per CSS Flexbox §8.5 and
    /// CSS Grid §11.8; overflow does not suppress their baseline). Empty flex/grid
    /// containers without in-flow items and replaced elements sit on their bottom
    /// margin edge.
    pub(crate) fn atom_sits_on_line_box(&self, id: StableNodeId) -> bool {
        self.atom_sits_on_line_box_in(id, &mut NodeMap::default())
    }

    /// [`Self::atom_sits_on_line_box`], reading and filling `memo` (see
    /// [`Self::holds_line_box_in`]).
    pub(crate) fn atom_sits_on_line_box_in(
        &self,
        id: StableNodeId,
        memo: &mut NodeMap<bool>,
    ) -> bool {
        self.layout_style(id).is_some_and(|style| {
            let flex_or_grid = style
                .display
                .is_some_and(|d| d.is_flex_container() || d.is_grid_container());
            style.resolved_vertical_align().uses_own_baseline()
                && !self.aligns_by_bottom_edge(id, &style)
                && (flex_or_grid || (!style.overflow_x.clips() && !style.overflow_y.clips()))
                && (self.holds_line_box_in(id, memo)
                    || self.has_in_flow_flex_or_grid_item(id, &style))
        })
    }

    /// Whether what `id` holds can move the baseline an atom sits on its
    /// line by: an atom on a line of an ordinary block whose last line box
    /// is its own, an item a flex line aligns by its first baseline, or an
    /// in-flow block, flex item or grid item of such an atom's or item's
    /// flow (and so on down), where that line box is looked for (CSS 2.1 §10.8.1, CSS
    /// Align 3 §9).
    /// A replaced one and an atom whose overflow is not visible sit by their
    /// bottom margin edge, whatever they hold. Native paragraph atoms are
    /// not counted: the paragraph reads their metrics after layout.
    pub(crate) fn atom_baseline_reads_content(&self, id: StableNodeId) -> bool {
        // Up the ancestors, one at a time: each step either answers or asks
        // the same of the parent.
        let mut id = id;
        loop {
            let Some(style) = self.layout_style(id) else {
                return false;
            };
            if self.inline_parent(id).is_some() || self.aligns_by_bottom_edge(id, &style) {
                return false;
            }
            if self.aligns_by_first_baseline(id) {
                return true;
            }
            let Some(parent) = self.parent_id(id) else {
                return false;
            };
            let Some(parent_style) = self.layout_style(parent) else {
                return false;
            };
            // Any in-flow item of a flex or grid container can be the one it
            // takes its baseline from (#394).
            let reads_parent = if parent_style
                .display
                .is_some_and(|display| display.is_flex_container() || display.is_grid_container())
            {
                !style.position.is_out_of_flow() && !style.omits_box()
            } else if style.is_inline_level() {
                if !style.resolved_vertical_align().uses_own_baseline() {
                    return false;
                }
                let flex_or_grid = style
                    .display
                    .is_some_and(|d| d.is_flex_container() || d.is_grid_container());
                return flex_or_grid || (!style.overflow_x.clips() && !style.overflow_y.clips());
            } else {
                !style.position.is_out_of_flow()
            };
            if !reads_parent {
                return false;
            }
            id = parent;
        }
    }

    /// Whether `id` is an in-flow item of a flex or grid container that
    /// aligns it by its first baseline (`align-self: baseline`, or
    /// `align-items: baseline` it does not override). A grid's share one
    /// baseline per start row (#394).
    pub(crate) fn aligns_by_first_baseline(&self, id: StableNodeId) -> bool {
        let Some(parent) = self
            .parent_id(id)
            .and_then(|parent| self.layout_style(parent))
        else {
            return false;
        };
        parent
            .display
            .is_some_and(|display| display.is_flex_container() || display.is_grid_container())
            && self.layout_style(id).is_some_and(|style| {
                !style.position.is_out_of_flow()
                    && !style.omits_box()
                    && style.resolved_align_self(parent.align_items)
                        == nana_ui_core::AlignSpec::Baseline
            })
    }

    /// Whether `id` lays out a line box of its own: text of its own, or a
    /// control or paragraph that lays text out. Its baselines are then that
    /// text's.
    pub(crate) fn lays_out_own_line(&self, id: StableNodeId) -> bool {
        self.widget_visual_ref(id).is_some()
            || self.text(id).is_some_and(|text| !text.is_empty())
            || self.text_layout(id).is_some()
            || self.inline_record(id).is_some()
    }

    /// Whether every line box `id` holds is a descendant's: it lays out none
    /// of its own, is not replaced, and holds one in its flow. Its last
    /// baseline is then that of the descendant holding its last line box
    /// (CSS 2.1 §10.8.1), at that descendant's offset inside it.
    pub(crate) fn line_boxes_are_descendants_in(
        &self,
        id: StableNodeId,
        memo: &mut NodeMap<bool>,
    ) -> bool {
        !self.lays_out_own_line(id)
            && self.layout_style(id).is_some_and(|style| {
                !self.aligns_by_bottom_edge(id, &style)
                    && (self.holds_line_box_in(id, memo)
                        || self.has_in_flow_flex_or_grid_item(id, &style))
            })
    }

    /// Whether `id` holds an in-flow line box: text of its own, a control
    /// or paragraph that lays text out, an inline-level box in its flow, or
    /// a block in its flow that holds one.
    ///
    /// Answers from `memo` and records there every box it settles. A layout
    /// pass keeps one memo for the pass (the world cannot change under it),
    /// so a chain of boxes asking level by level settles each box once
    /// instead of walking its subtree again at every level. The walk keeps
    /// its open boxes on the heap.
    pub(crate) fn holds_line_box_in(&self, id: StableNodeId, memo: &mut NodeMap<bool>) -> bool {
        /// How an in-flow child bears on whether its parent holds a line
        /// box: not at all, certainly, or by holding one itself.
        enum Bears {
            No,
            Holds,
            IfItHolds,
        }
        let creates_ifc = |node: StableNodeId| {
            self.layout_style(node).is_none_or(|style| {
                !style
                    .display
                    .is_some_and(|d| d.is_flex_container() || d.is_grid_container())
            })
        };
        let bears = |parent_ifc: bool, child: StableNodeId| {
            let Some(style) = self.layout_style(child) else {
                return Bears::No;
            };
            if style.omits_box() || style.position.is_out_of_flow() {
                Bears::No
            } else if parent_ifc && style.is_inline_level() {
                Bears::Holds
            } else if self.aligns_by_bottom_edge(child, &style) {
                Bears::No
            } else {
                Bears::IfItHolds
            }
        };
        if let Some(&known) = memo.get(&id) {
            return known;
        }
        if self.lays_out_own_line(id) {
            memo.insert(id, true);
            return true;
        }
        // Measured bottom up, a box usually finds every child it waits on
        // settled already, or a leaf, which holds a line box exactly when it
        // lays one out itself: then it settles here, with no walk to set up.
        let mut deeper = false;
        let id_ifc = creates_ifc(id);
        for &child in self.child_ids(id) {
            let holds = match bears(id_ifc, child) {
                Bears::No => false,
                Bears::Holds => true,
                Bears::IfItHolds => match memo.get(&child) {
                    Some(&known) => known,
                    None if self.child_ids(child).is_empty() => {
                        let holds = self.lays_out_own_line(child);
                        memo.insert(child, holds);
                        holds
                    }
                    None => {
                        deeper = true;
                        false
                    }
                },
            };
            if holds {
                memo.insert(id, true);
                return true;
            }
        }
        if !deeper {
            memo.insert(id, false);
            return false;
        }
        // Post-order: a box is settled once every child it depends on is.
        // Each box reads its children once: a child that holds a line box,
        // or one already known to hold one, settles it; otherwise it waits
        // for the blocks not yet known (`unknown[from..to]`), and the ones
        // known not to hold any are not read again. A list of rows under an
        // edited one asks its rows once, not twice.
        let mut unknown = Vec::new();
        let mut open = vec![(id, None::<(usize, usize)>)];
        while let Some((node, waiting)) = open.pop() {
            if memo.contains_key(&node) {
                continue;
            }
            if let Some((from, to)) = waiting {
                let holds = unknown[from..to]
                    .iter()
                    .any(|child| memo.get(child) == Some(&true));
                memo.insert(node, holds);
                continue;
            }
            if self.lays_out_own_line(node) {
                memo.insert(node, true);
                continue;
            }
            let from = unknown.len();
            let mut holds = false;
            let node_ifc = creates_ifc(node);
            for &child in self.child_ids(node).iter() {
                match bears(node_ifc, child) {
                    Bears::No => {}
                    Bears::Holds => holds = true,
                    Bears::IfItHolds => match memo.get(&child) {
                        Some(&known) => holds |= known,
                        None => unknown.push(child),
                    },
                }
                if holds {
                    break;
                }
            }
            if holds || unknown.len() == from {
                unknown.truncate(from);
                memo.insert(node, holds);
                continue;
            }
            open.push((node, Some((from, unknown.len()))));
            open.extend(unknown[from..].iter().map(|&child| (child, None)));
        }
        memo[&id]
    }

    pub(super) fn inline_box_omitted(&self, id: StableNodeId) -> bool {
        self.inline_parent(id).is_some()
            && self
                .component_layout_box(id)
                .is_some_and(|bounds| bounds.width <= 0.0 || bounds.height <= 0.0)
    }
    pub(super) fn inline_presentation_visible_memo(
        &self,
        id: StableNodeId,
        memo: &mut AncestorMemo,
    ) -> bool {
        if !self.nodes.has_inline_content() {
            return true;
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut visible = true;
        while let Some(node) = current {
            if let Some(known) = memo.inline_visible.get(&node) {
                visible = *known;
                break;
            }
            memo.chain.push(node);
            if self.inline_box_omitted(node) {
                visible = false;
                break;
            }
            current = self.parent_id(node);
        }
        for node in memo.chain.drain(..) {
            memo.inline_visible.insert(node, visible);
        }
        visible
    }
    /// One canonical view of plain text, styled text and ordinary atomic
    /// children. No builder/Vue/range-edit history is retained.
    pub fn inline_content(&self, id: StableNodeId) -> Option<InlineContent> {
        if !self.nodes.contains(id) {
            return None;
        }
        let value = &self.nodes.text_content(id).value;
        if self.nodes.rich_text(id).is_none() && self.inline_record(id).is_none() {
            return Some(InlineContent::PlainText(value.to_shared_str()));
        }
        let text = self
            .nodes
            .rich_text(id)
            .cloned()
            .unwrap_or_else(|| nana_ui_core::RichText::new(value.to_shared_str()));
        Some(InlineContent::from_parts(
            text,
            self.nodes
                .inline_record(id)
                .map_or_else(|| Arc::from([]), |inline| Arc::clone(&inline.atoms)),
        ))
    }
    pub(crate) fn inline_record(&self, id: StableNodeId) -> Option<&InlineRecord> {
        self.nodes.inline_record(id)
    }
    pub(crate) fn inline_parent(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.nodes.inline_parent(id)
    }
    pub(super) fn apply_inline_content(&mut self, id: StableNodeId, content: &InlineContent) {
        self.set_localized_text(id, None);
        let rich = content.to_rich_text();
        self.apply_rich_text(id, Some(&rich));
        if self.nodes.set_inline_atoms(id, content.atoms().into()) {
            self.nodes.invalidate_text(id, crate::TextDirty::CONSTRAINT);
            self.mark(
                id,
                DirtyMask::TEXT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
            );
            self.record_child_list_invalidation(id);
        }
        self.mark(id, DirtyMask::ACCESSIBILITY);
        self.recompute_node_theme_deps(id);
    }
    /// Resolves the inline parent's font strut and x-height for `vertical-align`
    /// keywords (`text-top`, `text-bottom`, `middle`, `sub`, `super`).
    pub(crate) fn inline_parent_strut(
        &self,
        parent: StableNodeId,
        parent_font_px: f32,
    ) -> crate::inline_content::ParentLineStrut {
        let font = self
            .computed_style(parent)
            .map_or(parent_font_px, |style| style.font_size)
            .max(0.0);
        let (font_ascent, font_descent, x_height) = self
            .inline_strut(parent)
            .or_else(|| {
                self.block_strut(parent)
                    .map(|strut| (strut, self.block_x_height(parent)))
            })
            .map(|(strut, x_h)| {
                (
                    font * strut.ascent,
                    font * strut.descent,
                    x_h.map_or(font * 0.5, |ratio| font * ratio),
                )
            })
            .or_else(|| {
                self.text_layout(parent).and_then(|(_, layout)| {
                    layout.runs.iter().find(|run| !run.is_object()).map(|run| {
                        let xh = if run.metrics.x_height_px.is_finite()
                            && run.metrics.x_height_px > 0.0
                        {
                            run.metrics.x_height_px
                        } else {
                            font * 0.5
                        };
                        (run.metrics.ascent_px, run.metrics.descent_px, xh)
                    })
                })
            })
            .unwrap_or_else(|| {
                let ascent = font * nana_ui_core::TEXT_APPROX_ASCENT_EM;
                (ascent, font - ascent, font * 0.5)
            });
        crate::inline_content::ParentLineStrut {
            font_ascent,
            font_descent,
            x_height,
            font_px: font,
        }
    }

    /// Update only paragraphs reached by this layout scope. A pixel/hover
    /// mutation never calls this and equal exported metrics queue no work.
    pub(crate) fn sync_inline_metrics(
        &mut self,
        scope: &[StableNodeId],
        baseline_of: impl Fn(StableNodeId, bool) -> Option<f32>,
        mut solved: hashbrown::HashMap<StableNodeId, Arc<[nana_text::InlineObject]>>,
        viewport: crate::LayoutViewport,
    ) {
        let mut parents = NodeSet::default();
        for &id in scope {
            if self.inline_record(id).is_some() {
                parents.insert(id);
            }
            if let Some(parent) = self.inline_parent(id) {
                parents.insert(parent);
            }
        }
        for parent in parents {
            if let Some(objects) = solved.remove(&parent) {
                if self.nodes.set_inline_objects(parent, objects) {
                    self.nodes
                        .invalidate_text(parent, crate::TextDirty::CONSTRAINT);
                    self.mark(parent, DirtyMask::TEXT | DirtyMask::RENDER);
                }
                continue;
            }
            let Some(inline) = self.inline_record(parent) else {
                continue;
            };
            let writing = self.layout_writing(parent);
            let objects: Arc<[_]> = inline
                .atoms
                .iter()
                .zip(inline.objects.iter())
                .map(|(atom, previous)| {
                    let bounds = self.component_layout_box(atom.node).unwrap_or_default();
                    let visible = self
                        .layout_style(atom.node)
                        .is_some_and(|style| !style.omits_box());
                    if visible && (bounds.width <= 0.0 || bounds.height <= 0.0) {
                        return *previous;
                    }
                    let available = self.component_content_box(parent).unwrap_or_default();
                    let margin = self.inline_object_margin(
                        atom.node,
                        writing.inline_size(available.width, available.height),
                    );
                    let block_size = writing.logical_size(bounds.width, bounds.height).1;
                    let retained =
                        (!writing.is_vertical()).then_some(previous.metrics.ascent_px - margin.top);
                    let first_only = self.layout_style(atom.node).is_some_and(|style| {
                        style.display.is_some_and(|display| {
                            display.is_flex_container() || display.is_grid_container()
                        })
                    });
                    let baseline = self
                        .atom_sits_on_line_box(atom.node)
                        .then(|| baseline_of(atom.node, first_only).or(retained))
                        .flatten()
                        .map(|baseline| baseline.clamp(0.0, block_size.max(0.0)));
                    let default_style = nana_ui_core::LayoutStyle::default();
                    let child_style = self.layout_style(atom.node);
                    let parent_font_px = self
                        .computed_style(parent)
                        .map_or(nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX, |style| {
                            style.font_size
                        });
                    let parent_strut = self.inline_parent_strut(parent, parent_font_px);
                    let (metrics, _) = crate::inline_content::resolve_atom_vertical_align(
                        child_style.as_deref().unwrap_or(&default_style),
                        writing,
                        crate::layout_engine::Size::new(bounds.width, bounds.height),
                        margin,
                        baseline,
                        parent_strut,
                        parent_font_px,
                        viewport,
                    );
                    nana_text::InlineObject {
                        offset: atom.offset,
                        id: atom.node.get(),
                        metrics: if visible {
                            metrics
                        } else {
                            nana_text::InlineObjectMetrics {
                                width_px: 0.0,
                                ascent_px: 0.0,
                                descent_px: 0.0,
                                align: nana_text::InlineObjectAlign::Baseline,
                                baseline_shift_px: 0.0,
                                envelope: None,
                            }
                        },
                    }
                })
                .collect::<Vec<_>>()
                .into();
            if self.nodes.set_inline_objects(parent, objects) {
                self.nodes
                    .invalidate_text(parent, crate::TextDirty::CONSTRAINT);
                self.mark(parent, DirtyMask::TEXT | DirtyMask::RENDER);
            }
        }
    }
    pub(super) fn mark_inline_text_owners(&mut self, id: StableNodeId) {
        if !self.nodes.has_inline_content() {
            return;
        }
        let mut current = Some(id);
        while let Some(child) = current {
            if let Some(parent) = self.inline_parent(child) {
                self.mark(parent, DirtyMask::ACCESSIBILITY);
            }
            current = self.parent_id(child);
        }
    }
    /// Visible text uses ordinary child semantics, never a hidden pre-edit
    /// source. Accessibility names and copy values are independent projections.
    pub fn inline_text(&self, id: StableNodeId, accessible: bool) -> Option<String> {
        self.inline_text_range(id, 0..self.text(id)?.len(), accessible)
    }
    pub fn inline_text_range(
        &self,
        id: StableNodeId,
        range: std::ops::Range<usize>,
        accessible: bool,
    ) -> Option<String> {
        if !self.nodes.contains(id) {
            return None;
        }
        if accessible && let Some(label) = &self.nodes.accessibility(id).label {
            return Some(label.to_string());
        }
        if self.text_input_is_secure(id) {
            return Some(String::new());
        }
        let value = &self.nodes.text_content(id).value;
        if range.start > range.end
            || range.end > value.len()
            || !value.is_char_boundary(range.start)
            || !value.is_char_boundary(range.end)
        {
            return None;
        }
        let Some(inline) = self.inline_record(id) else {
            return Some(value[range].to_owned());
        };
        let mut result = String::with_capacity(value.len());
        let mut start = range.start;
        for atom in inline
            .atoms
            .iter()
            .filter(|atom| range.contains(&atom.offset))
        {
            result.push_str(&value[start..atom.offset]);
            let explicit = if accessible {
                &atom.text.accessible_text
            } else {
                &atom.text.copy_text
            };
            if let Some(text) = explicit {
                result.push_str(text);
            } else if self.nodes.contains(atom.node)
                && (!accessible || !self.nodes.accessibility(atom.node).hidden)
                && let Some(text) = self.inline_text(atom.node, accessible)
            {
                result.push_str(&text);
            }
            start = atom.offset + '\u{fffc}'.len_utf8();
        }
        result.push_str(&value[start..range.end]);
        Some(result)
    }
}

/// Shared physical text anchor. Layout parts and the painter consume the same
/// vertical alignment; they do not independently adjust labels or atomic nodes.
pub fn text_layout_origin(
    bounds: LayoutBox,
    vertical: crate::TextVerticalAlignment,
    height: f32,
) -> [f32; 2] {
    use crate::TextVerticalAlignment::*;
    [
        bounds.x,
        match vertical {
            Top => bounds.y,
            Center => bounds.y + (bounds.height - height) * 0.5,
            Bottom => bounds.y + bounds.height - height,
        },
    ]
}

pub(crate) fn project_inline_content(
    id: StableNodeId,
    world: &UiWorld,
    content: &InlineContent,
    queue: &mut MutationQueue,
) {
    if world.inline_content(id).as_ref() == Some(content) && world.localized_text(id).is_none() {
        return;
    }
    let previous = world
        .inline_record(id)
        .map(|inline| Arc::clone(&inline.atoms))
        .unwrap_or_default();
    let next: NodeSet = content.atoms().iter().map(|atom| atom.node).collect();
    queue.set_inline_content(id, content.clone());
    for atom in previous.iter().filter(|atom| !next.contains(&atom.node)) {
        queue.despawn_subtree(atom.node);
    }
    let wanted: Vec<_> = content.atoms().iter().map(|atom| atom.node).collect();
    if previous.iter().map(|atom| atom.node).collect::<Vec<_>>() != wanted {
        for child in wanted {
            queue.insert(id, child, None);
        }
    }
}

impl super::mutation::ValidationPlan<'_> {
    pub(super) fn validate_inline_content(
        &mut self,
        mutations: &[UiMutation],
    ) -> Result<(), UiWorldError> {
        if !self.source.nodes.has_inline_content()
            && !mutations
                .iter()
                .any(|mutation| matches!(mutation, UiMutation::SetInlineContent { .. }))
        {
            return Ok(());
        }
        let mut pending: NodeMap<Option<InlineContent>> = NodeMap::default();
        let mut affected = NodeSet::default();
        let mut roles = NodeMap::default();
        let mut visited_ancestors = NodeSet::default();
        for mutation in mutations {
            let id = match mutation {
                UiMutation::SetInlineContent { id, content } => {
                    pending.insert(*id, Some((**content).clone()));
                    Some(*id)
                }
                UiMutation::SetText { id, .. } | UiMutation::SetRichText { id, .. } => {
                    pending.insert(*id, None);
                    Some(*id)
                }
                UiMutation::Insert { child, .. } => Some(*child),
                UiMutation::Detach { id }
                | UiMutation::ParkSubtree { root: id }
                | UiMutation::DespawnSubtree { root: id }
                | UiMutation::SetStyle { id, .. } => Some(*id),
                UiMutation::SetInteraction { id, .. } => Some(*id),
                UiMutation::SetAccessibility { id, accessibility } => {
                    roles.insert(*id, accessibility.role);
                    Some(*id)
                }
                _ => None,
            };
            if let Some(id) = id {
                if self.source.inline_record(id).is_some() {
                    affected.insert(id);
                }
                if let Some(parent) = self.source.inline_parent(id) {
                    affected.insert(parent);
                }
                let mut ancestor = self.node(id).ok().and_then(|node| node.parent);
                while let Some(parent) = ancestor {
                    if !visited_ancestors.insert(parent) {
                        break;
                    }
                    if self.source.inline_record(parent).is_some() || pending.contains_key(&parent)
                    {
                        affected.insert(parent);
                    }
                    ancestor = self.node(parent).ok().and_then(|node| node.parent);
                }
            }
        }
        affected.extend(pending.keys().copied());
        let role = |id, source: &UiWorld| {
            roles
                .get(&id)
                .copied()
                .or_else(|| source.accessibility(id).map(|state| state.role))
                .unwrap_or_default()
        };
        let interactions = self.interactions.clone();
        let focusable = |id, source: &UiWorld| {
            interactions
                .get(&id)
                .copied()
                .or_else(|| source.interaction(id))
                .is_some_and(|state| state.focusable)
        };
        for id in affected {
            if !self.exists(id) {
                continue;
            }
            let content = match pending.get(&id) {
                Some(Some(content)) => content.clone(),
                Some(None) => continue,
                None => self
                    .source
                    .inline_content(id)
                    .ok_or(UiWorldError::InvalidInlineContent(id))?,
            };
            let parent_document = self.node(id)?.document;
            let mut seen = NodeSet::default();
            let referenced: NodeSet = content.atoms().iter().map(|atom| atom.node).collect();
            if !referenced.is_empty() {
                for child in self.node(id)?.children.clone().iter() {
                    let style = self
                        .styles
                        .get(child)
                        .map(|style| style.layout.as_ref())
                        .or_else(|| {
                            self.source
                                .node_style(*child)
                                .map(|style| style.layout.as_ref())
                        });
                    if !referenced.contains(child)
                        && !style.is_some_and(|style| {
                            style.position.is_out_of_flow() || style.omits_box()
                        })
                    {
                        return Err(UiWorldError::InvalidInlineContent(id));
                    }
                }
            }
            for atom in content.atoms() {
                if !seen.insert(atom.node)
                    || !content
                        .text()
                        .get(atom.offset..)
                        .is_some_and(|text| text.starts_with('\u{fffc}'))
                {
                    return Err(UiWorldError::InvalidInlineContent(id));
                }
                let child = self.node(atom.node)?;
                if child.parent != Some(id) || child.document != parent_document {
                    return Err(UiWorldError::InvalidInlineContent(id));
                }
                let style = self
                    .styles
                    .get(&atom.node)
                    .map(|style| style.layout.as_ref())
                    .or_else(|| {
                        self.source
                            .node_style(atom.node)
                            .map(|style| style.layout.as_ref())
                    });
                if style.is_some_and(|style| style.position.is_out_of_flow()) {
                    return Err(UiWorldError::InvalidInlineContent(id));
                }
                if inline_control(role(id, self.source)) || focusable(id, self.source) {
                    let mut stack = vec![atom.node];
                    while let Some(child) = stack.pop() {
                        if inline_control(role(child, self.source)) || focusable(child, self.source)
                        {
                            return Err(UiWorldError::InvalidInlineContent(id));
                        }
                        stack.extend(self.node(child)?.children.iter().copied());
                    }
                }
            }
        }
        Ok(())
    }
}

fn inline_control(role: crate::AccessibilityRole) -> bool {
    use crate::AccessibilityRole::*;
    matches!(
        role,
        Button | TextInput | Checkbox | Switch | Slider | ComboBox | Tab | Radio | MenuItem
    )
}

impl crate::AppContext {
    /// Reconcile a content sequence without recreating unchanged children.
    /// Removed atomic subtrees leave the retained world normally.
    pub fn set_inline_content(
        &mut self,
        id: StableNodeId,
        content: impl Into<InlineContent>,
    ) -> Result<crate::CommitReport, crate::FrameworkError> {
        let content = content.into().normalized();
        let mut queue = MutationQueue::new();
        project_inline_content(id, self.world(), &content, &mut queue);
        self.commit_mutations(queue)
    }
}

/// Plain controls stay on their existing fast path; only styled/mixed content
/// or a transition out of it needs reconciliation.
pub(crate) fn project_control_content(
    id: StableNodeId,
    world: &UiWorld,
    label: &str,
    content: Option<&InlineContent>,
    queue: &mut MutationQueue,
) -> bool {
    if let Some(content) = content {
        project_inline_content(id, world, content, queue);
        true
    } else if world.inline_record(id).is_some() || world.rich_text(id).is_some() {
        project_inline_content(id, world, &InlineContent::from(label), queue);
        true
    } else {
        false
    }
}
