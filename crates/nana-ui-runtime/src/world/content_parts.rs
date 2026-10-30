//! Component intent lowers into lightweight Foundation parts before publication.
use super::*;
use crate::layout_engine::{
    Size,
    parts::{
        CONTENT_PART_SLOTS, ContentParts, EmptyStateInputs, FlexPart, LeadingParts,
        NO_CONTENT_PARTS, ProgressInputs, RangeInputs, SwitchInputs, ToastInputs,
        button_content_parts, checkbox_parts, content_parts, empty_state_parts, part_bounds,
        selection_option_parts, selection_option_row, start_row_parts, status_dot_parts,
        switch_parts, toast_parts,
    },
};

/// Diameter of a status dot, as a fraction of the slot reserved for it.
///
/// `StatusBadge` and `ValidationMessage` both wrote this as
/// `indicator_slot * 10.0 / 24.0`, where nothing in either arm is 24: the
/// denominator was the icon box the ratio was originally measured against, and
/// it survived the move to a spacing-step slot. Two copies of a phantom
/// denominator is how the two badges drift apart.
pub(crate) const STATUS_DOT_FRACTION: f32 = 10.0 / 24.0;

/// Whether a visual's content parts read installed metrics that a theme can
/// move without moving any style the node resolves: a switch's track and
/// gap, a range's gaps and unit size, a progress bar's heading and gaps, a
/// status dot's slot and gap, a selection option's icon gap. (A
/// checkbox's indicator reads the same type and spacing steps its own style
/// resolves, so a theme that moves them relays it out already; a radio ring
/// keeps fixed control steps.)
pub(super) fn visual_parts_read_theme(visual: Option<&StandardVisual>) -> bool {
    matches!(
        visual,
        Some(
            StandardVisual::Switch { .. }
                | StandardVisual::Range { .. }
                | StandardVisual::Progress { .. }
                | StandardVisual::StatusBadge { .. }
                | StandardVisual::ValidationMessage { .. }
                | StandardVisual::SelectionOption { icon: Some(_), .. }
        )
    )
}

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
        let Some(StandardVisual::ModalFrame { kind, slots, .. }) = self.standard_visual_ref(id)
        else {
            return None;
        };
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
        let recipe = self.theme.recipes().dialog();
        let chrome = crate::overlay_surfaces::ModalChrome::measure_in(
            *kind,
            presentation.title,
            presentation.description,
            slots,
            recipe,
            self.theme.border().hairline,
            &self.theme,
        );
        let body_copy = presentation.body.map_or(0.0, |metrics| metrics.height);
        let body_slot = slots
            .body
            .and_then(|id| self.component_layout_box(id))
            .map_or(0.0, |region| region.height);
        let body_gap = if body_copy > 0.0 && body_slot > 0.0 {
            self.theme.spacing().md
        } else {
            0.0
        };
        let surface = crate::overlay_surfaces::modal_surface_bounds(
            bounds,
            *kind,
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
    pub(crate) fn modal_text_parts(&self, id: StableNodeId, bounds: LayoutBox) -> ContentParts {
        let Some(StandardVisual::ModalFrame {
            description,
            body_text,
            ..
        }) = self.standard_visual_ref(id)
        else {
            return NO_CONTENT_PARTS;
        };
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
                        y: y + title + self.theme.spacing().xs,
                        width,
                        height: frame.presentation.description.unwrap_or_default().height,
                    },
                )
            }),
            body_text.as_ref().and_then(|_| {
                part(
                    crate::LayoutPartKind::Note,
                    LayoutBox {
                        height: frame.presentation.body.unwrap_or_default().height,
                        ..body
                    },
                )
            }),
        ])
    }

    pub(super) fn component_content_parts(&self, id: StableNodeId) -> ContentParts {
        if let Some(result) = self.layout_result(id) {
            let mut parts = NO_CONTENT_PARTS;
            let mut published = result.parts.iter().filter(|part| {
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
                )
            });
            for (slot, part) in parts.iter_mut().zip(published.by_ref()) {
                *slot = Some(*part);
            }
            // The solver never yields more than the slots hold, so nothing
            // published is dropped here.
            debug_assert!(
                published.next().is_none(),
                "more than {CONTENT_PART_SLOTS} content parts published"
            );
            return parts;
        }
        self.component_content_box(id)
            .map_or(NO_CONTENT_PARTS, |content| {
                self.solve_content_parts(id, content)
            })
    }
    pub(super) fn solve_content_parts(&self, id: StableNodeId, content: LayoutBox) -> ContentParts {
        self.solve_content_parts_at(
            id,
            content,
            self.component_layout_box(id).unwrap_or(content),
        )
    }
    pub(crate) fn solve_content_parts_at(
        &self,
        id: StableNodeId,
        content: LayoutBox,
        bounds: LayoutBox,
    ) -> ContentParts {
        let Some(visual) = self.standard_visual_ref(id) else {
            return NO_CONTENT_PARTS;
        };
        let text = self.text_metrics(id).unwrap_or_default();
        if let StandardVisual::Select {
            label,
            hint,
            placeholder,
            size,
            options,
            fit_options,
            ..
        } = visual
        {
            let reverse = self
                .computed_style(id)
                .is_some_and(|style| style.direction == nana_ui_core::DirSpec::Rtl);
            let measure = self.chrome_text_measure(id);
            let natural_label = (*fit_options).then(|| {
                options
                    .iter()
                    .map(|option| measure.width(&option.label, size.text_size(), None))
                    .fold(0.0, f32::max)
            });
            return crate::layout_engine::parts::select_trigger_parts(
                content,
                *size,
                label,
                hint.as_ref().map(|text| text.as_ref()),
                *placeholder,
                |text, size| measure.width(text, size, None),
                natural_label.or_else(|| self.text_natural_width(id)),
                reverse,
            );
        }
        if let StandardVisual::FormField {
            label: _,
            hint,
            error,
            size,
            ..
        } = visual
        {
            let support = error.as_ref().or(hint.as_ref());
            let support_measure = support.map(|support| {
                self.chrome_text_measure(id)
                    .width(support, self.theme.typography().hint, None)
            });
            let reverse = self
                .computed_style(id)
                .is_some_and(|style| style.direction == nana_ui_core::DirSpec::Rtl);
            return crate::layout_engine::parts::form_field_parts(
                bounds,
                *size,
                support.map(|text| &**text),
                error.is_some(),
                support_measure,
                reverse,
            );
        }
        if let Some(parts) =
            button_content_parts(visual, content, Size::new(text.width, text.height))
        {
            return parts;
        }
        if let Some(inputs) = self.checkbox_parts(id, Size::new(text.width, text.height)) {
            return start_row_parts(content, inputs, self.right_to_left(id));
        }
        if let Some(inputs) = self.switch_inputs(id) {
            return switch_parts(content, inputs);
        }
        if let Some(inputs) = self.range_inputs(id) {
            return crate::layout_engine::parts::range_parts(content, inputs);
        }
        if let Some(inputs) = self.progress_inputs(id) {
            return crate::layout_engine::parts::progress_parts(content, inputs);
        }
        if let StandardVisual::ModalFrame { .. } = visual {
            return self.modal_text_parts(id, bounds);
        }
        if let Some(inputs) = self.empty_state_inputs(id) {
            let presentation = self.nodes.empty_state_text(id).copied().unwrap_or_default();
            let size = |metrics: crate::TextMetrics| Size::new(metrics.width, metrics.height);
            let message = matches!(
                visual,
                StandardVisual::EmptyState {
                    message: Some(_),
                    ..
                }
            )
            .then(|| size(presentation.message.unwrap_or_default()));
            return empty_state_parts(content, inputs, size(presentation.title), message);
        }
        if let StandardVisual::MenuSurface {
            trigger,
            trigger_icon,
            trigger_image,
            overlay,
            ..
        } = visual
        {
            // A glyph trigger centres its square in the trigger; a label runs
            // across the trigger inside its inline padding (the border lies
            // within that inset), the trigger's whole height. An avatar or a
            // child drawing the trigger lays out no part of its own.
            if trigger_icon.is_some() {
                let extent = overlay
                    .and_then(|overlay| overlay.trigger_icon_size)
                    .unwrap_or(crate::popover::TRIGGER_ICON_SIZE)
                    .min(bounds.width)
                    .min(bounds.height);
                return content_parts([Some(crate::LayoutPart::new(
                    crate::LayoutPartKind::LeadingContent,
                    LayoutBox {
                        x: bounds.x + (bounds.width - extent) / 2.0,
                        y: bounds.y + (bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    },
                ))]);
            }
            if trigger_image.is_none() && trigger.as_ref().is_some_and(|label| !label.is_empty()) {
                let padding = self.used_layout_padding(id);
                return content_parts([Some(crate::LayoutPart::new(
                    crate::LayoutPartKind::TextContent,
                    LayoutBox {
                        x: bounds.x + padding.left,
                        width: (bounds.width - padding.left - padding.right).max(0.0),
                        ..bounds
                    },
                ))]);
            }
            return NO_CONTENT_PARTS;
        }
        if let StandardVisual::Toast { assembled, .. } = visual {
            let inputs = self.toast_inputs(id).expect("toast");
            return if *assembled {
                inputs.assembled_mark(
                    content,
                    crate::toast::line_box(self.theme.recipes().toast().title.size),
                )
            } else {
                toast_parts(bounds, inputs)
            };
        }
        if let Some(inputs) = self.selection_option_parts(id, Size::new(text.width, text.height)) {
            return selection_option_row(
                content,
                inputs,
                self.selection_option_icon_line(id),
                self.right_to_left(id),
            );
        }
        if let Some(inputs) = self.status_dot_parts(id, Size::new(text.width, text.height)) {
            // The spacer before the dot is not a part of its own.
            let [_, dot, label, ..] = start_row_parts(content, inputs, self.right_to_left(id));
            return content_parts([dot, label]);
        }
        if let StandardVisual::Card { title, loading, .. } = visual {
            // The title's line at the top of the band layout keeps before
            // the body (#385).
            let x = content.x.min(bounds.x + bounds.width).max(bounds.x);
            let y = content.y.max(bounds.y).min(bounds.y + bounds.height);
            let header = LayoutBox {
                x,
                y,
                width: content.width.min((bounds.x + bounds.width - x).max(0.0)),
                height: crate::view_components::CARD_TITLE_LINE
                    .min((bounds.y + bounds.height - y).max(0.0)),
            };
            return crate::layout_engine::parts::card_content_parts(
                header,
                title.is_some(),
                *loading,
                Size::new(
                    if title.as_ref().is_some_and(|title| !title.is_empty()) {
                        text.width
                    } else {
                        0.0
                    },
                    text.height,
                ),
                self.computed_style(id)
                    .is_some_and(|style| style.direction == nana_ui_core::DirSpec::Rtl),
            );
        }
        if let StandardVisual::LabeledValue {
            label,
            value,
            value_weight,
            compact,
            action,
            ..
        } = visual
        {
            let reverse = self
                .computed_style(id)
                .is_some_and(|style| style.direction == nana_ui_core::DirSpec::Rtl);
            let gap = if *compact {
                self.theme.spacing().xs
            } else {
                self.theme.spacing().md
            };
            let action = action
                .filter(|id| {
                    self.node_style(*id)
                        .is_none_or(|style| !style.layout.omits_box())
                })
                .and_then(|id| self.component_layout_box(id));
            let mut area = content;
            if let Some(action) = action {
                if reverse {
                    area.x = (action.x + action.width + gap).min(content.x + content.width);
                    area.width = (content.x + content.width - area.x).max(0.0);
                } else {
                    area.width = (action.x - gap - content.x).max(0.0).min(content.width);
                }
            }
            let measure = self.chrome_text_measure(id);
            let label_size = self.theme.typography().hint;
            let value_size = self.theme.typography().meta;
            return crate::layout_engine::parts::paired_text_parts(
                area,
                Size::new(
                    if label.is_empty() {
                        0.0
                    } else {
                        measure.width(label, label_size, None)
                    },
                    label_size * 1.2,
                ),
                Size::new(
                    measure.width(value, value_size, Some(*value_weight)),
                    value_size * 1.2,
                ),
                gap,
                self.theme.spacing().xxxl,
                reverse,
            );
        }
        if let StandardVisual::ActionMenuItem {
            hint, icon, size, ..
        } = visual
        {
            return crate::layout_engine::parts::menu_content_parts(
                bounds,
                size.padding_x_in(self.style_model.metrics),
                self.theme.spacing().md,
                icon.map(|_| size.text_size_in(self.theme.typography())),
                hint.as_ref().map(|hint| {
                    self.chrome_text_measure(id)
                        .width(hint, self.theme.typography().hint, None)
                }),
            );
        }
        if let StandardVisual::ListItem {
            leading,
            content: None,
            trailing,
            note,
            detail,
        } = visual
        {
            let Some(style) = self.computed_style(id) else {
                return NO_CONTENT_PARTS;
            };
            let reverse = style.direction == nana_ui_core::DirSpec::Rtl;
            let gap = self.nodes.get(id).map_or(0.0, |node| {
                node.resolved_layout.main_gap_against(
                    nana_ui_core::FlexDirection::Row,
                    nana_ui_core::ParentBox::from_viewport(content.width, content.height),
                )
            });
            let mut left = content.x;
            let mut right = content.x + content.width;
            for (slot, start) in [(*leading, true), (*trailing, false)] {
                let Some(slot) = slot.filter(|slot| {
                    self.node_style(*slot)
                        .is_none_or(|style| !style.layout.omits_box())
                }) else {
                    continue;
                };
                let Some(bounds) = self.component_layout_box(slot) else {
                    continue;
                };
                if start != reverse {
                    left = left.max(bounds.x + bounds.width + gap);
                } else {
                    right = right.min(bounds.x - gap);
                }
            }
            let bounds = LayoutBox {
                x: left,
                width: (right - left).max(0.0),
                ..content
            };
            if note.as_ref().is_none_or(|text| text.is_empty())
                && detail.as_ref().is_none_or(|text| text.is_empty())
            {
                return content_parts([Some(crate::LayoutPart::new(
                    crate::LayoutPartKind::TextContent,
                    bounds,
                ))]);
            }
            let measure = self.chrome_text_measure(id);
            let size = self.theme.typography().meta;
            let height = (size * self.theme.typography().line / self.theme.typography().body)
                .min(bounds.height.max(size));
            let secondary = |text: &Option<Arc<str>>| {
                text.as_ref().filter(|text| !text.is_empty()).map(|text| {
                    Size::new(
                        measure.width(text, size, Some(self.theme.typography().regular)),
                        height,
                    )
                })
            };
            return crate::layout_engine::parts::list_text_parts(
                bounds,
                crate::layout_engine::parts::RowTextMetrics {
                    label_width: measure.width(
                        self.text(id).unwrap_or_default(),
                        style.font_size,
                        style.font_weight,
                    ),
                    note: secondary(note),
                    detail: secondary(detail),
                    note_gap: self.theme.spacing().sm,
                    detail_gap: self.theme.spacing().md,
                    min_secondary_width: self.theme.spacing().xxxl,
                },
                reverse,
            );
        }
        NO_CONTENT_PARTS
    }

    /// Whether `id` lays its content out right to left (`direction: rtl`):
    /// its parts' inline start is the right edge (#388).
    pub(crate) fn right_to_left(&self, id: StableNodeId) -> bool {
        self.computed_style(id)
            .is_some_and(|style| style.direction == nana_ui_core::DirSpec::Rtl)
    }

    /// A checkbox's indicator and label before placement, the label `text`
    /// in size: what intrinsic measure, the paragraph budget and placement
    /// all read, so none of them keeps its own copy of the indicator's room.
    pub(crate) fn checkbox_parts(
        &self,
        id: StableNodeId,
        text: Size,
    ) -> Option<[Option<FlexPart>; 3]> {
        let Some(StandardVisual::Checkbox { size, .. }) = self.standard_visual_ref(id) else {
            return None;
        };
        Some(checkbox_parts(
            size.indicator_size_in(self.theme.typography(), self.theme.spacing()),
            size.indicator_gap_in(self.theme.spacing()),
            self.text(id).is_some_and(|label| !label.is_empty()),
            text,
        ))
    }

    /// A selection option's radio ring, icon and label before placement, the
    /// label `text` in size: what intrinsic measure, the paragraph budget and
    /// placement all read, so the style's padding keeps no copy of the ring's
    /// or the icon's room. The ring and a radio row's icon keep the fixed
    /// control steps they always used; a segmented item's icon is one em of
    /// the label's resolved font, its gap the installed `xs` step.
    pub(crate) fn selection_option_parts(
        &self,
        id: StableNodeId,
        text: Size,
    ) -> Option<[Option<FlexPart>; 3]> {
        let Some(StandardVisual::SelectionOption {
            icon,
            size,
            indicator,
            ..
        }) = self.standard_visual_ref(id)
        else {
            return None;
        };
        let gap = self.theme.spacing().xs;
        let icon = icon.map(|_| {
            let extent = if *indicator {
                size.icon_size()
            } else {
                self.computed_style(id)
                    .map_or(size.text_size(), |style| style.font_size)
            };
            (extent, gap)
        });
        Some(selection_option_parts(
            indicator.then(|| (size.indicator_size(), size.indicator_gap())),
            icon,
            text,
        ))
    }

    /// The line an option's icon centres on when its label is not centred
    /// in the row: the label's first line box.
    fn selection_option_icon_line(&self, id: StableNodeId) -> Option<f32> {
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
        let Some(StandardVisual::EmptyState {
            icon,
            compact,
            action,
            ..
        }) = self.standard_visual_ref(id)
        else {
            return None;
        };
        let spacing = self.theme.spacing();
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
        match self.standard_visual_ref(id)? {
            StandardVisual::EmptyState { .. } => {
                let inputs = self.empty_state_inputs(id)?;
                let presentation = self.nodes.empty_state_text(id).copied().unwrap_or_default();
                let size = |metrics: crate::TextMetrics| Size::new(metrics.width, metrics.height);
                Some(LeadingParts {
                    axis: nana_ui_core::FlexDirection::Column,
                    extent: inputs
                        .block_height(size(presentation.title), presentation.message.map(size)),
                    breadth: 0.0,
                    end: false,
                })
            }
            StandardVisual::Card { title: Some(_), .. } => Some(LeadingParts {
                axis: nana_ui_core::FlexDirection::Column,
                extent: crate::view_components::CARD_TITLE_BAND,
                breadth: 0.0,
                end: false,
            }),
            StandardVisual::Toast {
                assembled: true, ..
            } => {
                let inputs = self.toast_inputs(id)?;
                Some(LeadingParts {
                    axis: nana_ui_core::FlexDirection::Row,
                    extent: inputs.lead(),
                    breadth: inputs.mark.unwrap_or(0.0),
                    end: inputs.reverse,
                })
            }
            _ => None,
        }
    }

    /// Where `id`'s children lay out in its `content` box: past the parts
    /// laid out before them (#385).
    pub(crate) fn children_box(&self, id: StableNodeId, content: LayoutBox) -> LayoutBox {
        let (inset, offset) = self
            .leading_parts(id)
            .map_or((Size::default(), Size::default()), |lead| {
                (lead.inset(), lead.offset())
            });
        LayoutBox {
            x: content.x + offset.width,
            y: content.y + offset.height,
            width: (content.width - inset.width).max(0.0),
            height: (content.height - inset.height).max(0.0),
        }
    }

    /// A toast's mark, copy and dismiss target before placement, from the
    /// installed toast recipe: what intrinsic measure, the title's
    /// paragraph budget, placement and an assembled toast's lead all read,
    /// so neither its style nor its paint keeps a copy of their room.
    pub(crate) fn toast_inputs(&self, id: StableNodeId) -> Option<ToastInputs> {
        let Some(StandardVisual::Toast {
            description,
            dismissible,
            assembled,
            ..
        }) = self.standard_visual_ref(id)
        else {
            return None;
        };
        let recipe = self.theme.recipes().toast();
        // An assembled toast's description and close button are children.
        let bare = !*assembled;
        let inputs = crate::layout_engine::parts::toast_inputs(
            recipe,
            recipe.control_size.height_in(self.style_model.metrics),
            bare && *dismissible,
            bare && description.is_some(),
        );
        Some(ToastInputs {
            reverse: self.right_to_left(id),
            // A bare toast's title grows by the lines it wrapped to past its
            // first (#388).
            title: inputs.title + if bare { self.toast_title_wrap(id) } else { 0.0 },
            ..inputs
        })
    }

    /// How much taller than its first line a bare toast's shaped title is:
    /// the second line, when the title wrapped.
    fn toast_title_wrap(&self, id: StableNodeId) -> f32 {
        self.text_layout(id)
            .and_then(|(_, layout)| {
                let first = layout.lines.first()?;
                (layout.lines.len() > 1)
                    .then(|| (layout.bounds.height - first.bounds.height).max(0.0))
            })
            .unwrap_or(0.0)
    }

    /// [`ToastInputs::chrome`] for a bare toast's own border.
    pub(crate) fn toast_chrome(&self, id: StableNodeId) -> Option<Size> {
        let border = self.nodes.get(id)?.resolved_layout.resolved_border_edges();
        Some(self.toast_inputs(id)?.chrome(Size::new(
            border.left + border.right,
            border.top + border.bottom,
        )))
    }

    /// A switch's track and text column before placement, from the
    /// installed theme: what intrinsic measure and placement both read, so
    /// neither the style nor the paint keeps its own copy of the track's
    /// room or the text column's height.
    pub(crate) fn switch_inputs(&self, id: StableNodeId) -> Option<SwitchInputs> {
        let Some(StandardVisual::Switch {
            label,
            hint,
            control_position,
            size,
            ..
        }) = self.standard_visual_ref(id)
        else {
            return None;
        };
        let track = self.style_model.metrics.switch;
        let typography = self.theme.typography();
        let line = |font: f32| (font * typography.line / typography.body).max(font);
        // The label and hint at the size and weight geometry paints them.
        let measure = self.chrome_text_measure(id);
        let label_width = if label.is_empty() {
            0.0
        } else {
            measure.width(
                label,
                size.text_size_in(typography),
                Some(typography.medium),
            )
        };
        let hint_width = hint.as_ref().map_or(0.0, |hint| {
            measure.width(
                hint,
                size.caption_size_in(typography),
                Some(typography.regular),
            )
        });
        Some(SwitchInputs {
            track: Size::new(track.track_width, track.track_height),
            gap: track.label_gap,
            end: *control_position == SwitchControlPosition::End,
            reverse: self.right_to_left(id),
            labelled: !label.is_empty() || hint.is_some(),
            text: label_width.max(hint_width),
            label_line: line(size.text_size_in(typography)),
            hint: hint.as_ref().map(|_| {
                (
                    self.theme.spacing().xxs,
                    line(size.caption_size_in(typography)),
                )
            }),
        })
    }

    /// A range's label, value slot and unit before placement: what
    /// intrinsic measure, placement and the track's lane all read (#384).
    /// The label and value are set in the range's own font, the unit at the
    /// caption size; the slot measures the formatted minimum and maximum in
    /// tabular figures, so it moves only with them, the step, the unit and
    /// the theme, never with the value. A rail draws no text.
    pub(crate) fn range_inputs(&self, id: StableNodeId) -> Option<RangeInputs> {
        let Some(StandardVisual::Range {
            label,
            limits,
            unit,
            size,
            rail: None,
            ..
        }) = self.standard_visual_ref(id)
        else {
            return None;
        };
        let style = self.computed_style(id)?;
        let typography = self.theme.typography();
        let measure = self.chrome_text_measure(id);
        let value = limits.as_ref().map(|limits| {
            limits
                .iter()
                .map(|limit| measure.tabular_width(limit, style.font_size, style.font_weight))
                .fold(0.0, f32::max)
        });
        Some(RangeInputs {
            label: label
                .as_ref()
                .map(|label| measure.width(label, style.font_size, style.font_weight)),
            value,
            unit: unit.as_ref().filter(|_| value.is_some()).map(|unit| {
                measure.width(
                    unit,
                    size.caption_size_in(typography),
                    Some(typography.regular),
                )
            }),
            gap: self.range_gap(*size),
        })
    }

    /// A progress bar's label, cancel target and track before placement,
    /// from the installed theme: what intrinsic measure, placement and the
    /// track all read (#384). The label's line is the meta type size it is
    /// set at, the cancel target a page step square.
    pub(crate) fn progress_inputs(&self, id: StableNodeId) -> Option<ProgressInputs> {
        let Some(StandardVisual::Progress {
            label,
            cancellable,
            thickness,
            ..
        }) = self.standard_visual_ref(id)
        else {
            return None;
        };
        let spacing = self.theme.spacing();
        Some(ProgressInputs {
            label: label.as_ref().map(|_| self.theme.typography().meta),
            cancel: cancellable.then_some(spacing.page),
            cancel_gap: spacing.md,
            gap: spacing.sm,
            thickness: if thickness.is_finite() && *thickness > 0.0 {
                *thickness
            } else {
                spacing.sm
            },
        })
    }

    /// Between a range's label and its track, and its track and readout:
    /// the spacing ladder's step for the control size.
    pub(crate) fn range_gap(&self, size: nana_ui_core::ControlSize) -> f32 {
        let spacing = self.theme.spacing();
        match size {
            nana_ui_core::ControlSize::Small => spacing.sm,
            nana_ui_core::ControlSize::Medium => spacing.md,
            nana_ui_core::ControlSize::Large => spacing.lg,
        }
    }

    /// A status badge's or validation message's dot and label before
    /// placement, the label `text` in size: what intrinsic measure, the
    /// paragraph budget and placement all read, so the style's padding keeps
    /// no copy of the dot's room.
    pub(crate) fn status_dot_parts(
        &self,
        id: StableNodeId,
        text: Size,
    ) -> Option<[Option<FlexPart>; 3]> {
        let spacing = self.theme.spacing();
        let (slot, gap) = match self.standard_visual_ref(id)? {
            StandardVisual::StatusBadge { compact, .. } => {
                if *compact {
                    (spacing.sm, spacing.xs)
                } else {
                    (spacing.md, spacing.sm)
                }
            }
            StandardVisual::ValidationMessage { compact, .. } => {
                if *compact {
                    (spacing.xl, spacing.xs)
                } else {
                    (spacing.xxl, spacing.sm)
                }
            }
            _ => return None,
        };
        Some(status_dot_parts(
            slot,
            slot * STATUS_DOT_FRACTION,
            gap,
            text,
        ))
    }

    pub(super) fn content_part_box(
        &self,
        id: StableNodeId,
        kind: crate::LayoutPartKind,
    ) -> Option<LayoutBox> {
        if let Some(result) = self.layout_result(id) {
            return result
                .parts
                .iter()
                .find(|part| part.kind == kind)
                .map(|part| part.bounds);
        }
        part_bounds(
            &self.solve_content_parts(id, self.component_content_box(id)?),
            kind,
        )
    }
}
