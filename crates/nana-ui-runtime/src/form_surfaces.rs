use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, CardKind, ControlSize, FlexDirection, JustifySpec, LengthSpec, SemanticColorRole,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, InteractionStyle,
    MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, TextContent, UiWorld,
};

const SUPPORT_SIZE: f32 = nana_ui_core::type_scale::HINT;

/// Line box a [`FormField`] reserves for its label at `size`.
///
/// Layout writes this into `padding_top`; the painter places the label inside
/// the same band. Both sides must read this one function.
pub(crate) fn form_field_label_line(size: ControlSize) -> f32 {
    let (label_size, _, _, _) = form_field_density(size);
    ControlSize::nearest_text(label_size).line_height()
}

pub(crate) fn form_field_density(size: ControlSize) -> (f32, f32, SemanticColorRole, u16) {
    let (gap, role, weight) = match size {
        ControlSize::Small => (
            nana_ui_core::space::XXS,
            SemanticColorRole::Muted,
            nana_ui_core::type_scale::REGULAR,
        ),
        ControlSize::Medium => (
            nana_ui_core::space::XS,
            SemanticColorRole::Text,
            nana_ui_core::type_scale::MEDIUM,
        ),
        ControlSize::Large => (
            nana_ui_core::space::SM,
            SemanticColorRole::Text,
            nana_ui_core::type_scale::MEDIUM,
        ),
    };
    (size.caption_size(), gap, role, weight)
}

/// Label/hint/error wrapper. The control is an application-owned child.
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub error: Option<Arc<str>>,
    pub size: ControlSize,
    pub control: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl FormField {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            error: None,
            size: ControlSize::Medium,
            control: None,
            style: NodeStyle::default(),
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn error(mut self, error: impl Into<Arc<str>>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn control_child(mut self, control: StableNodeId) -> Self {
        self.control = Some(control);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn support_text(&self) -> Option<&Arc<str>> {
        self.error.as_ref().or(self.hint.as_ref())
    }

    fn effective_style(&self) -> NodeStyle {
        let (label_size, gap, label_color, label_weight) = form_field_density(self.size);
        let mut style = self.style.clone();
        style.foreground = Some(label_color);
        style.background = None;
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.direction = Some(FlexDirection::Column);
        // A field holding more than one child sets its own gap; only the
        // default is zero.
        if layout.gap.is_none() {
            layout.gap = Some(LengthSpec::Px(0.0));
        }
        layout.border_width = Some(0.0);
        layout.font_size = Some(label_size);
        layout.font_weight = Some(label_weight);
        // Reserve the line box `ControlSize` declares for this text size rather
        // than guessing a leading factor: a label that wraps or a font with
        // taller metrics would otherwise overlap the control.
        let label_line = form_field_label_line(self.size);
        let support_line = ControlSize::nearest_text(SUPPORT_SIZE).line_height();
        layout.padding_top = Some(LengthSpec::Px(label_line + gap));
        layout.padding_bottom = Some(LengthSpec::Px(if self.support_text().is_some() {
            support_line + gap
        } else {
            0.0
        }));
        style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
        style
    }
}

impl ComponentView for FormField {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::FORM_FIELD.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "form-field".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        let visual = FormFieldVisual {
            label: Arc::clone(&self.label),
            hint: self.hint.clone(),
            error: self.error.clone(),
            size: self.size,
            control: self.control,
        };
        if world.widget_visual_as::<FormFieldVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.label)),
                value: self.error.clone().or_else(|| self.hint.clone()),
                invalid: self.error.is_some(),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Visual state for [`FormField`] nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct FormFieldVisual {
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub error: Option<Arc<str>>,
    pub size: ControlSize,
    pub control: Option<StableNodeId>,
}

impl crate::WidgetVisual for FormFieldVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn content_layout_changed(&self, prev: Option<&dyn std::any::Any>) -> bool {
        match prev.and_then(|any| any.downcast_ref::<Self>()) {
            Some(old) => {
                (self.size, self.hint.is_some(), self.error.is_some())
                    != (old.size, old.hint.is_some(), old.error.is_some())
            }
            None => true,
        }
    }

    fn clearing_changes_content_layout(&self) -> bool {
        true
    }

    fn invalidates_node_text_constraint(&self) -> bool {
        true
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: crate::LayoutBox,
        _content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        let support = self.error.as_ref().or(self.hint.as_ref());
        let support_measure = support.map(|support| {
            world
                .chrome_text_measure(id)
                .width(support, world.theme().typography().hint, None)
        });
        let reverse = world.right_to_left(id);
        Some(crate::layout_engine::parts::form_field_parts(
            bounds,
            self.size,
            support.map(|text| &**text),
            self.error.is_some(),
            support_measure,
            reverse,
        ))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let (label_size, _gap, label_role, label_weight) = form_field_density(self.size);
        let label_height = form_field_label_line(self.size);
        let support = self.error.as_ref().or(self.hint.as_ref());
        let support_role = if self.error.is_some() {
            SemanticColorRole::Danger
        } else {
            SemanticColorRole::Muted
        };
        let parts = ctx.content_parts();
        let label_bounds =
            crate::layout_engine::parts::part_bounds(&parts, crate::LayoutPartKind::Label)
                .unwrap_or(crate::LayoutBox {
                    x: ctx.bounds.x,
                    y: ctx.bounds.y,
                    width: ctx.bounds.width,
                    height: label_height.min(ctx.bounds.height),
                });
        let support_bounds =
            crate::layout_engine::parts::part_bounds(&parts, crate::LayoutPartKind::Hint);
        let support_height = nana_ui_core::type_scale::META.min(ctx.bounds.height);
        let support_y = (ctx.bounds.y + ctx.bounds.height - support_height).max(ctx.bounds.y);
        let indicator_lane =
            crate::layout_engine::parts::part_bounds(&parts, crate::LayoutPartKind::LeadingContent);

        let mut out = crate::WidgetPaintOutput {
            suppress_root_surface: true,
            owns_node_text: true,
            ..crate::WidgetPaintOutput::new()
        };
        out.push(
            2,
            crate::WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                &crate::ComponentTextRegion {
                    bounds: label_bounds,
                    content: crate::TextValue::from(Arc::clone(&self.label)),
                    color: Some(ctx.style_model.color(label_role).as_rgba_array()),
                    font_size: label_size,
                    font_weight: Some(label_weight),
                },
                ctx,
                crate::TextHorizontalAlignment::Start,
                true,
            )),
        );
        if let Some(message) = support {
            out.push(
                3,
                crate::WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                    &crate::ComponentTextRegion {
                        bounds: support_bounds.unwrap_or(crate::LayoutBox {
                            x: ctx.bounds.x,
                            y: support_y,
                            width: ctx.bounds.width,
                            height: support_height,
                        }),
                        content: crate::TextValue::from(Arc::clone(message)),
                        color: Some(ctx.style_model.color(support_role).as_rgba_array()),
                        font_size: SUPPORT_SIZE,
                        font_weight: None,
                    },
                    ctx,
                    crate::TextHorizontalAlignment::Start,
                    true,
                )),
            );
        }
        if let (Some(_), Some(lane)) = (&self.error, indicator_lane) {
            let diameter = (lane.width * 10.0 / 24.0).min(lane.height).max(0.0);
            let indicator_color = ctx.style_model.color(support_role).as_rgba_array();
            out.push(
                4,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: crate::LayoutBox {
                        x: lane.x + (lane.width - diameter) / 2.0,
                        y: lane.y + (lane.height - diameter) / 2.0,
                        width: diameter,
                        height: diameter,
                    },
                    border: Some((indicator_color, 1.0)),
                    border_paint: ctx
                        .matching_text_paint(indicator_color)
                        .or_else(|| ctx.matching_border_paint(indicator_color)),
                    corner_radius: [999.0; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
        }
        out
    }
}

/// Selectable card. Children are application content.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveCard {
    pub selected: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl InteractiveCard {
    pub fn new() -> Self {
        Self {
            selected: false,
            disabled: false,
            style: NodeStyle::default(),
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = Some(if self.selected {
            SemanticColorRole::Selected
        } else {
            SemanticColorRole::Surface
        });
        style.border = self.selected.then_some(SemanticColorRole::BorderSoft);
        style.interaction = InteractionStyle {
            selected: SemanticPaint {
                background: Some(SemanticColorRole::Selected),
                border: Some(SemanticColorRole::BorderSoft),
                ..SemanticPaint::default()
            },
            selected_hovered: SemanticPaint {
                background: Some(SemanticColorRole::SelectedHover),
                ..SemanticPaint::default()
            },
            selected_pressed: SemanticPaint {
                background: Some(SemanticColorRole::SelectedPressed),
                ..SemanticPaint::default()
            },
            hovered: SemanticPaint {
                background: Some(if self.selected {
                    SemanticColorRole::SelectedHover
                } else {
                    SemanticColorRole::Hover
                }),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(if self.selected {
                    SemanticColorRole::SelectedPressed
                } else {
                    SemanticColorRole::Active
                }),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                background: Some(SemanticColorRole::Subtle),
                border: self.selected.then_some(SemanticColorRole::BorderSoft),
                ..SemanticPaint::default()
            },
            ..InteractionStyle::default()
        }
        .into();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Fill);
            // A flex column: the content sits centred both ways, which a
            // block container's `auto`-width children would not.
            layout
                .display
                .get_or_insert(nana_ui_core::DisplaySpec::Flex);
            layout.direction = Some(FlexDirection::Column);
            layout.align_items = AlignSpec::Center;
            layout.justify_content = JustifySpec::Center;
            layout.border_width = Some(if self.selected {
                nana_ui_core::HAIRLINE
            } else {
                0.0
            });
        }
        style.surface_padding = Some(nana_ui_core::SurfacePadding::Panel);
        style.radius = Some(nana_ui_core::RadiusTier::Md);
        style
    }
}

impl Default for InteractiveCard {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for InteractiveCard {
    const TYPE_ID: Option<&'static str> =
        Some(crate::component_descriptors::INTERACTIVE_CARD.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "interactive-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = crate::CardVisual {
            title: None,
            kind: if self.selected {
                CardKind::Selected
            } else {
                CardKind::Surface
            },
            loading: false,
            loading_phase: 0.0,
        };
        if world.widget_visual_as::<crate::CardVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                selected: Some(self.selected),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn form_field_projects_error_over_hint() {
        let mut context = AppContext::new();
        let field = context
            .create_component(
                document(),
                FormField::new("Email")
                    .hint("Work email")
                    .error("Invalid email"),
            )
            .unwrap();
        let id = field.stable_id();

        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "form-field".into(),
            }
        );
        assert_eq!(context.world().text(id), Some(""));
        assert_eq!(
            context.world().widget_visual_as::<FormFieldVisual>(id),
            Some(&FormFieldVisual {
                label: Arc::from("Email"),
                hint: Some(Arc::from("Work email")),
                error: Some(Arc::from("Invalid email")),
                size: ControlSize::Medium,
                control: None,
            })
        );
        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::Generic);
        assert_eq!(accessibility.label.as_deref(), Some("Email"));
        assert_eq!(accessibility.value.as_deref(), Some("Invalid email"));
        assert!(accessibility.invalid);

        context
            .update_component(field, |field, _| {
                field.error = None;
            })
            .unwrap();
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.value.as_deref(), Some("Work email"));
        assert!(!accessibility.invalid);
    }

    #[test]
    fn form_field_reserves_label_padding() {
        let mut context = AppContext::new();
        let medium = context
            .create_component(document(), FormField::new("Name"))
            .unwrap();
        let layout = &context
            .world()
            .node_style(medium.stable_id())
            .unwrap()
            .layout;
        assert_eq!(layout.width, Some(LengthSpec::Percent(100.0)));
        assert_eq!(layout.direction, Some(FlexDirection::Column));
        assert_eq!(layout.gap, Some(LengthSpec::Px(0.0)));
        assert_eq!(layout.border_width, Some(0.0));
        assert_eq!(layout.font_size, Some(nana_ui_core::type_scale::META));
        assert_eq!(layout.font_weight, Some(nana_ui_core::type_scale::MEDIUM));
        // The reserve is the declared line box for this label size plus the
        // density gap, not a guessed leading factor.
        assert_eq!(
            layout.padding_top,
            Some(LengthSpec::Px(
                form_field_label_line(ControlSize::Medium) + nana_ui_core::space::XS
            ))
        );
        assert_eq!(form_field_label_line(ControlSize::Medium), 16.0);
        assert_eq!(layout.padding_bottom, Some(LengthSpec::Px(0.0)));
        assert_eq!(
            context
                .world()
                .node_style(medium.stable_id())
                .unwrap()
                .foreground,
            Some(SemanticColorRole::Text)
        );
        assert!(
            context
                .world()
                .node_style(medium.stable_id())
                .unwrap()
                .border
                .is_none()
        );

        let small = context
            .create_component(
                document(),
                FormField::new("Name")
                    .size(ControlSize::Small)
                    .hint("Hint")
                    .error("Error"),
            )
            .unwrap();
        let style = context.world().node_style(small.stable_id()).unwrap();
        assert_eq!(style.foreground, Some(SemanticColorRole::Muted));
        assert_eq!(style.layout.font_size, Some(nana_ui_core::type_scale::HINT));
        assert_eq!(
            style.layout.font_weight,
            Some(nana_ui_core::type_scale::REGULAR)
        );
        assert_eq!(
            style.layout.padding_top,
            Some(LengthSpec::Px(
                form_field_label_line(ControlSize::Small) + nana_ui_core::space::XXS
            ))
        );
        // Support text reserves the line box for its own size, not the label's.
        let support_line = ControlSize::nearest_text(SUPPORT_SIZE).line_height();
        assert_eq!(
            style.layout.padding_bottom,
            Some(LengthSpec::Px(support_line + nana_ui_core::space::XXS))
        );
    }

    #[test]
    fn interactive_card_selected_uses_selected_kind() {
        let mut context = AppContext::new();
        let selected = context
            .create_component(document(), InteractiveCard::new().selected(true))
            .unwrap();
        let id = selected.stable_id();
        assert!(matches!(
            context.world().widget_visual_as::<crate::CardVisual>(id),
            Some(crate::CardVisual {
                kind: CardKind::Selected,
                title: None,
                loading: false,
                ..
            })
        ));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Selected));
        assert_eq!(style.layout.border_width, Some(nana_ui_core::HAIRLINE));
        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::Button);
        assert_eq!(accessibility.selected, Some(true));

        let disabled = context
            .create_component(document(), InteractiveCard::new().disabled(true))
            .unwrap();
        let disabled_id = disabled.stable_id();
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<crate::CardVisual>(disabled_id),
            Some(crate::CardVisual {
                kind: CardKind::Surface,
                ..
            })
        ));
        assert_eq!(
            context.world().interaction(disabled_id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        assert!(context.world().accessibility(disabled_id).unwrap().disabled);
    }

    #[test]
    fn form_field_error_text_shares_the_indicator_row_center() {
        let mut context = AppContext::new();
        let field = context
            .create_component(document(), FormField::new("Email").error("Required"))
            .unwrap();
        let id = field.stable_id();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            id,
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 240.0,
                height: 70.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        let paint = context.world().widget_paint(id).unwrap();
        let crate::WidgetPaintOp::Text(support) = &paint
            .ops
            .iter()
            .find(|tagged| tagged.slot == 3)
            .expect("error text")
            .op
        else {
            panic!("form field support text");
        };
        let crate::WidgetPaintOp::Quad(indicator_quad) = &paint
            .ops
            .iter()
            .find(|tagged| tagged.slot == 4)
            .expect("error indicator")
            .op
        else {
            panic!("form field error indicator");
        };
        let indicator = indicator_quad.rect;
        let text_center = support.bounds.y + support.bounds.height / 2.0;
        let indicator_center = indicator.y + indicator.height / 2.0;
        assert!((text_center - indicator_center).abs() < 0.01);
        assert_eq!(support.bounds.height, 12.0);
    }

    #[test]
    fn idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let field = context
            .create_component(
                document(),
                FormField::new("Name").hint("Hint").error("Error"),
            )
            .unwrap();
        let card = context
            .create_component(
                document(),
                InteractiveCard::new().selected(true).disabled(true),
            )
            .unwrap();
        let _ = context.take_system_work();
        context.reproject_component(field).unwrap();
        context.reproject_component(card).unwrap();
        assert!(context.take_system_work().is_empty());
    }
}
