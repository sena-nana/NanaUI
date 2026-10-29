//! Component intent lowers into lightweight Foundation parts before publication.
use super::*;
use crate::layout_engine::{
    Size,
    parts::{
        CONTENT_PART_SLOTS, ContentParts, EmptyStateInputs, FlexPart, NO_CONTENT_PARTS,
        SwitchInputs, ToastInputs, button_content_parts, checkbox_parts, content_parts,
        empty_state_parts, part_bounds, selection_option_parts, selection_option_row,
        start_row_parts, status_dot_parts, switch_parts, toast_parts,
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
/// gap, a status dot's slot and gap, a selection option's icon gap. (A
/// checkbox's indicator reads the same type and spacing steps its own style
/// resolves, so a theme that moves them relays it out already; a radio ring
/// keeps fixed control steps.)
pub(super) fn visual_parts_read_theme(visual: Option<&StandardVisual>) -> bool {
    matches!(
        visual,
        Some(
            StandardVisual::Switch { .. }
                | StandardVisual::StatusBadge { .. }
                | StandardVisual::ValidationMessage { .. }
                | StandardVisual::SelectionOption { icon: Some(_), .. }
        )
    )
}

impl UiWorld {
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
            return start_row_parts(content, inputs);
        }
        if let Some(inputs) = self.switch_inputs(id) {
            return switch_parts(content, inputs);
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
            return empty_state_parts(bounds, inputs, size(presentation.title), message);
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
            return selection_option_row(content, inputs, self.selection_option_icon_line(id));
        }
        if let Some(inputs) = self.status_dot_parts(id, Size::new(text.width, text.height)) {
            // The spacer before the dot is not a part of its own.
            let [_, dot, label, ..] = start_row_parts(content, inputs);
            return content_parts([dot, label]);
        }
        if let StandardVisual::Card { title, loading, .. } = visual {
            let border = self
                .nodes
                .get(id)
                .map_or(0.0, |node| node.resolved_layout.resolved_border_edges().top);
            let x = content.x.min(bounds.x + bounds.width).max(bounds.x);
            let y = (content.y - crate::view_components::CARD_TITLE_BAND)
                .max(bounds.y + border)
                .min(bounds.y + bounds.height);
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
    /// from the installed theme: what the paragraph budget, the room the
    /// block reserves above the action, placement and the chrome clip all
    /// read.
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
        let (inset, gap) = if *compact {
            (Size::new(spacing.sm, spacing.md), spacing.xxs)
        } else {
            (Size::new(spacing.xxxl, spacing.page), spacing.sm)
        };
        Some(EmptyStateInputs {
            inset,
            icon: icon.map(|_| spacing.page_tight + spacing.xxs),
            gap,
            centred: !*compact,
            action: action.map(|_| spacing.xs),
        })
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
        Some(crate::layout_engine::parts::toast_inputs(
            recipe,
            recipe.control_size.height_in(self.style_model.metrics),
            bare && *dismissible,
            bare && description.is_some(),
        ))
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
        Some(SwitchInputs {
            track: Size::new(track.track_width, track.track_height),
            gap: track.label_gap,
            end: *control_position == SwitchControlPosition::End,
            labelled: !label.is_empty() || hint.is_some(),
            label_line: line(size.text_size_in(typography)),
            hint: hint.as_ref().map(|_| {
                (
                    self.theme.spacing().xxs,
                    line(size.caption_size_in(typography)),
                )
            }),
        })
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
