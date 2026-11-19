//! Component intent lowers into lightweight Foundation parts before publication.
use super::*;
use crate::layout_engine::parts::{
    CONTENT_PART_SLOTS, ContentParts, EmptyStateInputs, LeadingParts, NO_CONTENT_PARTS,
    content_parts, part_bounds,
};

/// Where a modal frame's chrome puts things in its box: the chrome measured
/// from its shaped title (or the title content slot standing in for it),
/// description and body, and the card it sizes (#384).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ModalFrameLayout {
    pub chrome: crate::overlay_surfaces::ModalChrome,
    pub surface: LayoutBox,
    pub presentation: crate::components::ModalTextPresentation,
    /// The title content slot's box, when one lays out in the title's place.
    pub title_content: Option<LayoutBox>,
}

impl UiWorld {
    /// [`ModalFrameLayout`] for modal frame `id` laid out in `bounds`.
    pub(crate) fn modal_frame_layout(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
    ) -> Option<ModalFrameLayout> {
        let visual = self.widget_visual_as::<crate::ModalFrameVisual>(id)?;
        let kind = visual.kind;
        let slots = &visual.slots;
        let mut presentation = self.nodes.modal_text(id).copied().unwrap_or_default();
        // A title content slot that lays out stands in the title's place: the
        // row reads its height as it would the title's.
        let title_content = slots
            .title_content
            .filter(|slot| {
                !self
                    .node_style(*slot)
                    .is_some_and(|style| style.layout.omits_box())
            })
            .and_then(|slot| self.component_layout_box(slot));
        if let Some(content) = title_content {
            presentation.title = crate::TextMetrics {
                width: content.width,
                height: content.height,
                ascent: None,
            };
        }
        let node_theme = self.node_theme(id);
        let recipe = node_theme.recipes().dialog();
        let chrome = crate::overlay_surfaces::ModalChrome::measure_in(
            kind,
            presentation.title,
            presentation.description,
            slots,
            recipe,
            node_theme.border().hairline,
            node_theme,
        );
        let body_copy = presentation.body.map_or(0.0, |metrics| metrics.height);
        let body_slot = slots
            .body
            .and_then(|id| self.component_layout_box(id))
            .map_or(0.0, |region| region.height);
        let body_gap = if body_copy > 0.0 && body_slot > 0.0 {
            node_theme.spacing().md
        } else {
            0.0
        };
        let surface = crate::overlay_surfaces::modal_surface_bounds(
            bounds,
            kind,
            Some(chrome.chrome_height(body_copy + body_gap + body_slot)),
            recipe,
        );
        Some(ModalFrameLayout {
            chrome,
            surface,
            presentation,
            title_content,
        })
    }

    /// A modal frame's title (`Header`), description (`Detail`) and body copy
    /// (`Note`) where its chrome puts them in `bounds` (#384): the title and
    /// the description stacked after the icon, as wide as the header row
    /// lets them run, the body copy at the top of the body. Overlay layout
    /// publishes them; the text pass wraps to their widths and geometry
    /// paints in them.
    pub(crate) fn modal_text_metrics(
        &self,
        id: StableNodeId,
    ) -> crate::components::ModalTextPresentation {
        self.nodes.modal_text(id).copied().unwrap_or_default()
    }

    pub(crate) fn modal_text_parts(&self, id: StableNodeId, bounds: LayoutBox) -> ContentParts {
        let Some(visual) = self.widget_visual_as::<crate::ModalFrameVisual>(id) else {
            return NO_CONTENT_PARTS;
        };
        let description = &visual.description;
        let body_text = &visual.body_text;
        let Some(frame) = self.modal_frame_layout(id, bounds) else {
            return NO_CONTENT_PARTS;
        };
        let width = frame.chrome.text_width(frame.surface, bounds);
        let (x, y) = frame.chrome.title_origin(frame.surface);
        let title = frame.presentation.title.height;
        let body = frame.chrome.body_box(frame.surface);
        let part = |kind, bounds| Some(crate::LayoutPart::new(kind, bounds));
        content_parts([
            part(
                crate::LayoutPartKind::Header,
                LayoutBox {
                    x,
                    y,
                    width,
                    height: title,
                },
            ),
            description.as_ref().and_then(|_| {
                part(
                    crate::LayoutPartKind::Detail,
                    LayoutBox {
                        x,
                        y: y + title + self.node_theme(id).spacing().xs,
                        width,
                        height: frame.presentation.description.unwrap_or_default().height,
                    },
                )
            }),
            body_text.as_ref().and_then(|_| {
                part(
                    crate::LayoutPartKind::Note,
                    // Copy taller than the capped card stops at its body,
                    // clear of the actions below. The body is the card less
                    // its chrome, so copy that fits can come out a rounding
                    // step over it; that copy keeps its own height.
                    LayoutBox {
                        height: frame.presentation.body.map_or(0.0, |copy| {
                            if copy.height > body.height + 0.5 {
                                body.height
                            } else {
                                copy.height
                            }
                        }),
                        ..body
                    },
                )
            }),
        ])
    }

    pub(crate) fn component_content_parts(&self, id: StableNodeId) -> ContentParts {
        if let Some(meta) = self.published_layout_meta(id)
            && let Some(origin) = self.layout_origin(id)
        {
            let mut parts = NO_CONTENT_PARTS;
            if let Some(ext) = meta.ext.as_ref() {
                let mut published = ext.parts.iter().filter(|part| {
                    matches!(
                        part.kind,
                        crate::LayoutPartKind::TextContent
                            | crate::LayoutPartKind::LeadingContent
                            | crate::LayoutPartKind::TrailingContent
                            | crate::LayoutPartKind::Hint
                            | crate::LayoutPartKind::Spinner
                            | crate::LayoutPartKind::Note
                            | crate::LayoutPartKind::Detail
                            | crate::LayoutPartKind::Label
                            | crate::LayoutPartKind::Value
                            | crate::LayoutPartKind::Header
                            | crate::LayoutPartKind::Indicator
                            | crate::LayoutPartKind::Minimum
                            | crate::LayoutPartKind::Maximum
                    )
                });
                for (slot, part) in parts.iter_mut().zip(published.by_ref()) {
                    *slot = Some(crate::LayoutResult::place_part(origin, part));
                }
                // The solver never yields more than the slots hold, so nothing
                // published is dropped here.
                debug_assert!(
                    published.next().is_none(),
                    "more than {CONTENT_PART_SLOTS} content parts published"
                );
            } else if meta.single_text_source != super::TEXT_SOURCE_NONE_SENTINEL {
                parts[0] = Some(crate::LayoutResult::place_part(
                    origin,
                    &crate::LayoutPart::new(
                        crate::LayoutPartKind::TextContent,
                        meta.inline_text_box(),
                    ),
                ));
            }
            return parts;
        }
        self.component_content_box(id)
            .map_or(NO_CONTENT_PARTS, |content| {
                self.solve_content_parts(id, content)
            })
    }
    /// `id`'s parts in `content`, both on the page.
    pub(super) fn solve_content_parts(&self, id: StableNodeId, content: LayoutBox) -> ContentParts {
        self.solve_content_parts_in(
            id,
            content,
            self.component_layout_box(id).unwrap_or(content),
            false,
        )
    }
    /// [`Self::solve_content_parts`] with `content` and `bounds` measured
    /// from `id`'s own border-box origin, as layout places them and a
    /// result keeps its own geometry: the boxes of the slots a part steers
    /// around are measured from it too.
    pub(crate) fn solve_content_parts_at(
        &self,
        id: StableNodeId,
        content: LayoutBox,
        bounds: LayoutBox,
    ) -> ContentParts {
        self.solve_content_parts_in(id, content, bounds, true)
    }

    /// The parts of `id` in `content` and `bounds`, measured on the page or,
    /// `own_frame`, from `id`'s own origin, with the slots' boxes in the
    /// same space.
    fn solve_content_parts_in(
        &self,
        id: StableNodeId,
        content: LayoutBox,
        bounds: LayoutBox,
        own_frame: bool,
    ) -> ContentParts {
        if let Some(widget_visual) = self.widget_visual_ref(id) {
            let writing = self.containing_writing(id);
            if let Some(parts) =
                widget_visual.solve_content_parts(id, self, bounds, content, writing, own_frame)
            {
                return parts;
            }
        }
        NO_CONTENT_PARTS
    }

    /// Whether `id` lays its content out right to left (`direction: rtl`):
    /// its parts' inline start is the right edge (#388).
    pub(crate) fn right_to_left(&self, id: StableNodeId) -> bool {
        self.computed_style(id)
            .is_some_and(|style| style.direction == nana_ui_core::DirSpec::Rtl)
    }

    /// The line an option's icon centres on when its label is not centred
    /// in the row: the label's first line box.
    pub(crate) fn selection_option_icon_line(&self, id: StableNodeId) -> Option<f32> {
        let style = self.computed_style(id)?;
        let centred = self
            .node_style(id)
            .is_some_and(|style| style.text_vertical_alignment == TextVerticalAlignment::Center);
        (!centred)
            .then(|| nana_ui_core::text_line_box_height_px(style.font_size, style.line_height))
    }

    /// An empty state's icon, title and message block before placement,
    /// from the installed theme: what the band layout keeps above the
    /// action and placement both read.
    pub(crate) fn empty_state_inputs(&self, id: StableNodeId) -> Option<EmptyStateInputs> {
        let crate::EmptyStateVisual {
            icon,
            compact,
            action,
            ..
        } = self.widget_visual_as::<crate::EmptyStateVisual>(id)?;
        let spacing = self.node_theme(id).spacing();
        let gap = if *compact { spacing.xxs } else { spacing.sm };
        Some(EmptyStateInputs {
            icon: icon.map(|_| spacing.page_tight + spacing.xxs),
            gap,
            centred: !*compact,
            action: action.map(|_| spacing.xs),
        })
    }

    /// The parts `id` lays out before its children (#385): an empty state's
    /// icon, title and message above its action, a titled card's header
    /// above its body, an assembled toast's mark before its copy. Measure,
    /// placement and the parts' solve all read this, so no style of the node
    /// keeps a copy of their room.
    pub(crate) fn leading_parts(&self, id: StableNodeId) -> Option<LeadingParts> {
        self.widget_visual_ref(id)?.leading_parts(id, self)
    }

    /// Where `id`'s children lay out in its `content` box: past the parts
    /// laid out before them (#385).
    #[cfg(test)]
    pub(crate) fn children_box(&self, id: StableNodeId, content: LayoutBox) -> LayoutBox {
        let (inset, offset) = self.leading_parts(id).map_or(
            (
                crate::layout_engine::Size::default(),
                crate::layout_engine::Size::default(),
            ),
            |lead| (lead.inset(), lead.offset()),
        );
        LayoutBox {
            x: content.x + offset.width,
            y: content.y + offset.height,
            width: (content.width - inset.width).max(0.0),
            height: (content.height - inset.height).max(0.0),
        }
    }

    /// How much taller than its first line a bare toast's shaped title is:
    /// the second line, when the title wrapped.
    pub(crate) fn toast_title_wrap(&self, id: StableNodeId) -> f32 {
        self.text_layout(id)
            .and_then(|(_, layout)| {
                let first = layout.lines.first()?;
                (layout.lines.len() > 1)
                    .then(|| (layout.bounds.height - first.bounds.height).max(0.0))
            })
            .unwrap_or(0.0)
    }

    /// Between a range's label and its track, and its track and readout:
    /// the spacing ladder's step for the control size.
    #[cfg(test)]
    pub(crate) fn range_gap(&self, size: nana_ui_core::ControlSize) -> f32 {
        let spacing = self.theme.spacing();
        match size {
            nana_ui_core::ControlSize::Small => spacing.sm,
            nana_ui_core::ControlSize::Medium => spacing.md,
            nana_ui_core::ControlSize::Large => spacing.lg,
        }
    }

    pub(crate) fn range_gap_for(&self, id: StableNodeId, size: nana_ui_core::ControlSize) -> f32 {
        let spacing = self.node_theme(id).spacing();
        match size {
            nana_ui_core::ControlSize::Small => spacing.sm,
            nana_ui_core::ControlSize::Medium => spacing.md,
            nana_ui_core::ControlSize::Large => spacing.lg,
        }
    }

    /// Page-space bounds of the first laid-out part matching `kind` on `id`.
    pub fn content_part_box(
        &self,
        id: StableNodeId,
        kind: crate::LayoutPartKind,
    ) -> Option<LayoutBox> {
        if let Some(meta) = self.published_layout_meta(id) {
            let origin = self.layout_origin(id)?;
            if let Some(ext) = meta.ext.as_ref() {
                return ext
                    .parts
                    .iter()
                    .find(|part| part.kind == kind)
                    .map(|part| crate::LayoutResult::place_part(origin, part).bounds);
            }
            let local_box = match kind {
                crate::LayoutPartKind::ComponentContent => Some(meta.content_box),
                crate::LayoutPartKind::TextContent
                    if meta.single_text_source != super::TEXT_SOURCE_NONE_SENTINEL =>
                {
                    Some(meta.inline_text_box())
                }
                _ => None,
            };
            return local_box.map(|bounds| crate::layout_result::place_box(origin, bounds));
        }
        part_bounds(
            &self.solve_content_parts(id, self.component_content_box(id)?),
            kind,
        )
    }
}
