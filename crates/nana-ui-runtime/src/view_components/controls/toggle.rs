#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, shared_label,
    text_area_footer_extent, text_field_style, tool_box, with_unit,
};
use super::super::events::*;
use crate::layout_edits::{LayoutEdit, LengthBits, StackPreset, edit_layout};
use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId, TextCodeFold,
    TextColorSwatchSpan, TextCompletion, TextContent, TextDiagnosticSpan, TextEditorRenderOptions,
    TextGitMark, TextHorizontalAlignment, TextHover, TextInlay, TextInputState, TextMatchSpan,
    TextSignatureHelp, TextVerticalAlignment, UiWorld,
};
use crate::{AppContext, Entity};

fn checkbox_style() -> NodeStyle {
    checkbox_style_for(nana_ui_core::ControlSize::Medium)
}

fn checkbox_style_for(size: nana_ui_core::ControlSize) -> NodeStyle {
    let mut layout = nana_ui_core::LayoutStyle::default();
    layout.theme.control_typography(size);
    crate::theme_intent::weight(&mut layout, nana_ui_core::TextWeight::Regular);
    NodeStyle {
        layout: Arc::new(layout),
        control_height: Some(nana_ui_core::ControlHeight::Min(size)),
        foreground: Some(nana_ui_core::SemanticColorRole::Text),
        background: Some(nana_ui_core::SemanticColorRole::Background),
        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
        interaction: crate::InteractionStyle {
            selected: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Accent),
                border: Some(nana_ui_core::SemanticColorRole::Accent),
                ..SemanticPaint::default()
            },
            selected_hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            selected_pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Active),
                border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Hover),
                border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Active),
                border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            // Both of these already draw their own ring around the indicator
            // rather than the row, which is the right target for a 16px box in a
            // 380px row — so they take the colour and keep the ring, instead of
            // washing the whole row.
            focused: SemanticPaint::FOCUS_BORDER,
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                background: Some(nana_ui_core::SemanticColorRole::Subtle),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                ..SemanticPaint::default()
            },
            ..crate::InteractionStyle::default()
        }
        .into(),
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

fn switch_style() -> NodeStyle {
    let mut layout = nana_ui_core::LayoutStyle::default();
    layout
        .theme
        .control_typography(nana_ui_core::ControlSize::Medium);
    crate::theme_intent::weight(&mut layout, nana_ui_core::TextWeight::Medium);
    NodeStyle {
        layout: Arc::new(layout),
        foreground: Some(nana_ui_core::SemanticColorRole::Text),
        radius: Some(nana_ui_core::RadiusTier::Sm),
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Hover),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Active),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint::FOCUS_BORDER,
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                ..SemanticPaint::default()
            },
            ..crate::InteractionStyle::default()
        }
        .into(),
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checkbox {
    pub label: String,
    pub checked: bool,
    /// Mixed state, for a parent checkbox over partially checked children.
    /// Wins over `checked` when painting and in accessibility.
    pub indeterminate: bool,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub invalid: bool,
    pub style: NodeStyle,
}

impl Checkbox {
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(),
            checked,
            indeterminate: false,
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            invalid: false,
            style: checkbox_style(),
        }
    }

    pub fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        self.style = checkbox_style_for(size);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Checkbox {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::CHECKBOX.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::toggle_checkbox),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "checkbox".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone().into(),
                },
            );
        }
        let visual = CheckboxVisual {
            checked: self.checked,
            indeterminate: self.indeterminate,
            size: self.size,
        };
        if world.widget_visual_as::<CheckboxVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        if self.invalid {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.hovered.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.pressed.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.focused.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.selected.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.selected_hovered.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.selected_pressed.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
        }
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Checkbox,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                checked: Some(self.checked),
                mixed: self.indeterminate,
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Visual state for [`Checkbox`] nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckboxVisual {
    pub checked: bool,
    pub indeterminate: bool,
    pub size: nana_ui_core::ControlSize,
}

impl CheckboxVisual {
    fn parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        text: crate::layout_engine::Size,
    ) -> [Option<crate::layout_engine::parts::FlexPart>; 3] {
        let node_theme = world.node_theme(id);
        crate::layout_engine::parts::checkbox_parts(
            self.size
                .indicator_size_in(node_theme.typography(), node_theme.spacing()),
            self.size.indicator_gap_in(node_theme.spacing()),
            world.text(id).is_some_and(|label| !label.is_empty()),
            text,
        )
    }
}

impl crate::WidgetVisual for CheckboxVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_SHAPE)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_FAMILY)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE)
    }

    fn is_document_select_target(&self) -> bool {
        true
    }

    fn text_visual_key(&self) -> Option<(std::any::TypeId, u64)> {
        let step = match self.size {
            nana_ui_core::ControlSize::Small => 1,
            nana_ui_core::ControlSize::Medium => 2,
            nana_ui_core::ControlSize::Large => 3,
        };
        Some((std::any::TypeId::of::<Self>(), step))
    }

    fn intrinsic_chrome_content(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> crate::WidgetVisualContent {
        let parts = self.parts(id, world, crate::layout_engine::Size::default());
        let chrome = crate::layout_engine::parts::chrome_extent(&parts);
        crate::WidgetVisualContent {
            width: chrome.width,
            height: chrome.height,
            text: 0.0,
        }
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _bounds: crate::LayoutBox,
        content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        let text = world.text_metrics(id).unwrap_or_default();
        let inputs = self.parts(
            id,
            world,
            crate::layout_engine::Size::new(text.width, text.height),
        );
        Some(crate::layout_engine::parts::start_row_parts(
            content,
            inputs,
            world.right_to_left(id),
        ))
    }

    fn node_text_max_width(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _layout: crate::LayoutBox,
        available_width: f32,
        available_height: f32,
    ) -> Option<f32> {
        let parts = self.parts(id, world, crate::layout_engine::Size::default());
        Some(crate::layout_engine::parts::text_budget(
            parts,
            crate::layout_engine::Size::new(available_width, available_height),
        ))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let indicator = ctx
            .content_part(crate::LayoutPartKind::Indicator)
            .unwrap_or(crate::LayoutBox {
                width: 0.0,
                height: 0.0,
                ..ctx.content
            });
        let label_bounds = ctx
            .content_part(crate::LayoutPartKind::TextContent)
            .unwrap_or(ctx.content);
        let extent = indicator.width;
        let fg_role = ctx.theme.recipes().foreground(
            nana_ui_core::ComponentRecipeId::Checkbox,
            self.checked || self.indeterminate,
        );
        let fg = ctx.style_model.color(fg_role).as_rgba_array();
        let mut out = crate::WidgetPaintOutput {
            suppress_root_surface: true,
            owns_node_text: false,
            node_text_bounds: Some(label_bounds),
            ..crate::WidgetPaintOutput::new()
        };
        out.push(
            3,
            crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                rect: indicator,
                fill: ctx.style.background,
                fill_paint: ctx.style.paint_colors.background,
                border: ctx
                    .style
                    .border_color
                    .map(|color| (color, ctx.theme.border().hairline)),
                border_paint: ctx.style.paint_colors.border,
                corner_radius: [ctx.chrome_radii.sm; 4],
                ..crate::WidgetQuad::default()
            }),
        );
        if self.indeterminate {
            let dash_height = (extent / 8.0).max(1.5);
            let dash_inset = extent / 4.0;
            out.push(
                4,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: crate::LayoutBox {
                        x: indicator.x + dash_inset,
                        y: indicator.y + (extent - dash_height) / 2.0,
                        width: (extent - dash_inset * 2.0).max(0.0),
                        height: dash_height,
                    },
                    fill: Some(fg),
                    fill_paint: ctx.matching_text_paint(fg),
                    corner_radius: [dash_height / 2.0; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
        } else if self.checked {
            out.push(
                4,
                crate::WidgetPaintOp::Text(crate::WidgetText {
                    bounds: indicator,
                    content: crate::TextValue::from("✓"),
                    color: Some(fg),
                    paint_color: ctx.matching_text_paint(fg),
                    font_size: extent * 0.75,
                    font_weight: Some(ctx.theme.typography().bold),
                    horizontal_alignment: crate::TextHorizontalAlignment::Center,
                    vertical_alignment: crate::TextVerticalAlignment::Center,
                    ellipsis: false,
                    wrap: false,
                    max_lines: None,
                    italic: None,
                    underline: None,
                    line_through: None,
                    suppress_spans: true,
                    preserve_lines: None,
                    unstyled_glyph: true,
                    line_height: None,
                    retained_text: crate::WidgetRetainedText::None,
                    tabular_nums: false,
                    ..Default::default()
                }),
            );
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    pub label: String,
    pub hint: Option<Arc<str>>,
    /// Spoken name in place of the visible one (`aria-label`): for a control
    /// drawn without text, or several sharing one visible word. `None` keeps
    /// the visible label.
    pub accessible_name: Option<Arc<str>>,
    /// Said after the name by assistive technology (`aria-description`),
    /// never drawn. `None` says nothing more.
    pub accessible_description: Option<Arc<str>>,
    pub checked: bool,
    pub control_position: nana_ui_core::SwitchControlPosition,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub(crate) loading_phase: f32,
    pub invalid: bool,
    pub style: NodeStyle,
}

impl Switch {
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(),
            hint: None,
            accessible_name: None,
            accessible_description: None,
            checked,
            control_position: nana_ui_core::SwitchControlPosition::End,
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
            style: switch_style(),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// See [`Self::accessible_name`](field@Self::accessible_name); an empty
    /// name is none.
    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        self.accessible_name = (!name.is_empty()).then_some(name);
        self
    }

    /// See [`Self::accessible_description`](field@Self::accessible_description);
    /// an empty description is none.
    pub fn accessible_description(mut self, description: impl Into<Arc<str>>) -> Self {
        let description = description.into();
        self.accessible_description = (!description.is_empty()).then_some(description);
        self
    }
    pub fn control_position(mut self, position: nana_ui_core::SwitchControlPosition) -> Self {
        self.control_position = position;
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        self.style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
        self.style.control_padding_x = Some(size.into());
        Arc::make_mut(&mut self.style.layout)
            .theme
            .control_typography(size);
        self
    }
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Switch {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::SWITCH.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::toggle_switch),
        animation_sample: Some(|cx, target, sample, _| {
            cx.update_component(Entity::<Self>::from_stable_id(target), |switch, _| {
                switch.loading_phase = sample.progress;
            })
            .is_ok()
        }),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "switch".into(),
        }
    }

    /// A bare switch's width is spent from the installed metrics in
    /// `project`, which no layout intent re-resolves.
    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id).is_some_and(|text| !text.is_empty()) {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        let thumb_progress = match world.widget_visual_as::<SwitchVisual>(id) {
            Some(visual)
                if visual.checked != self.checked
                    || crate::component_animation_id(
                        crate::component_animation_kinds::SWITCH,
                        id,
                    )
                    .is_some_and(|animation| world.animation_is_active(animation)) =>
            {
                visual.thumb_progress
            }
            _ => f32::from(self.checked),
        };
        let visual = SwitchVisual {
            thumb_progress,
            label: Arc::from(self.label.as_str()),
            hint: self.hint.clone(),
            checked: self.checked,
            control_position: self.control_position,
            size: self.size,
            loading: self.loading,
            loading_phase: self.loading_phase,
            invalid: self.invalid,
        };
        if world.widget_visual_as::<SwitchVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        // A switch with nothing to say beside it — a row's trailing toggle —
        // is its track. There is no text to give the node a width, so `Fill`
        // would stretch an invisible hit box across the row, and nothing to
        // inset, so the control padding would only eat the track.
        let bare = self.label.is_empty() && self.hint.is_none();
        let metrics = world.theme_metrics();
        let mut effective_style = self.style.clone();
        {
            let layout = Arc::make_mut(&mut effective_style.layout);
            if layout.width.is_none() {
                layout.width = Some(if bare {
                    nana_ui_core::LengthSpec::Px(metrics.switch.track_width)
                } else {
                    nana_ui_core::LengthSpec::Fill
                });
            }
        }
        // The track and the label over hint column are content parts (#205):
        // intrinsic measure gives the box their height, so the style only
        // holds the size step's floor. A CSS box is a spent number; don't let
        // intent write the control height back over it.
        if effective_style.layout.height.is_some() || effective_style.layout.theme.height.is_some()
        {
            effective_style.control_height = None;
        } else if effective_style.control_height.is_none() {
            effective_style.control_height = Some(nana_ui_core::ControlHeight::Min(self.size));
        }
        // Only when the caller spent no inset of its own, the way the control
        // height right above is guarded.
        //
        // Writing it unconditionally made `control_padding_x` the one intent
        // field a switch could not opt out of. It overwrites `padding_left` /
        // `padding_right`, so a switch with no label — which has no text to
        // supply its width — was left a content box `width - 2 * inset` wide,
        // and the track clamps to `min(track_width, content width)`: at the
        // default inset, a 2pt sliver with a thumb beside it. Clearing
        // `control_padding_x` was no escape either, because it starts `None`
        // and this line put it straight back.
        //
        // Authored padding is the signal, not an unset intent field: a caller
        // that spent a number on this edge said what it wanted there, and a
        // component default must not overwrite it — the same rule
        // `Button::layout` states for a caller that hands over a whole box.
        let authored_inset = effective_style.layout.padding.is_some()
            || effective_style.layout.padding_left.is_some()
            || effective_style.layout.padding_right.is_some();
        if effective_style.control_padding_x.is_none() && !authored_inset && !bare {
            effective_style.control_padding_x = Some(self.size.into());
        }
        if self.invalid {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
        }
        // A bare switch is its track, and the track already answers hover and
        // press (geometry tints its fill and border). The box wash a labelled
        // switch spreads under its text would here be a rounded square past
        // the pill's ends, wider than the control the pointer is on.
        if bare {
            effective_style.interaction.hovered.background = None;
            effective_style.interaction.pressed.background = None;
        }
        effective_style.interaction.disabled.foreground =
            Some(nana_ui_core::SemanticColorRole::Muted);
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState {
                pointer_events: !self.disabled && !self.loading,
                focusable: !self.disabled && !self.loading,
            },
            AccessibilityState {
                role: AccessibilityRole::Switch,
                label: Some(
                    self.accessible_name
                        .clone()
                        .unwrap_or_else(|| Arc::from(self.label.as_str())),
                ),
                description: self.accessible_description.clone(),
                disabled: self.disabled,
                checked: Some(self.checked),
                busy: self.loading,
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Visual state for [`Switch`] nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchVisual {
    pub thumb_progress: f32,
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub checked: bool,
    pub control_position: nana_ui_core::SwitchControlPosition,
    pub size: nana_ui_core::ControlSize,
    pub loading: bool,
    pub loading_phase: f32,
    pub invalid: bool,
}

impl SwitchVisual {
    pub(crate) fn inputs(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> crate::layout_engine::parts::SwitchInputs {
        let node_theme = world.node_theme(id);
        let track = node_theme.metrics().switch;
        let typography = node_theme.typography();
        let line = |font: f32| (font * typography.line / typography.body).max(font);
        let measure = world.chrome_text_measure(id);
        let label_width = if self.label.is_empty() {
            0.0
        } else {
            measure.width(
                &self.label,
                self.size.text_size_in(typography),
                Some(typography.medium),
            )
        };
        let hint_width = self.hint.as_ref().map_or(0.0, |hint| {
            measure.width(
                hint,
                self.size.caption_size_in(typography),
                Some(typography.regular),
            )
        });
        crate::layout_engine::parts::SwitchInputs {
            track: crate::layout_engine::Size::new(track.track_width, track.track_height),
            gap: track.label_gap,
            end: self.control_position == nana_ui_core::SwitchControlPosition::End,
            reverse: world.right_to_left(id),
            labelled: !self.label.is_empty() || self.hint.is_some(),
            text: label_width.max(hint_width),
            label_line: line(self.size.text_size_in(typography)),
            hint: self.hint.as_ref().map(|_| {
                (
                    node_theme.spacing().xxs,
                    line(self.size.caption_size_in(typography)),
                )
            }),
        }
    }
}

impl crate::WidgetVisual for SwitchVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
    }

    fn visual_parts_read_theme(&self) -> bool {
        true
    }

    fn is_document_select_target(&self) -> bool {
        true
    }

    fn content_layout_changed(&self, prev: Option<&dyn std::any::Any>) -> bool {
        prev.and_then(|p| p.downcast_ref::<Self>()).is_none_or(|a| {
            (&a.label, &a.hint, a.control_position, a.size)
                != (&self.label, &self.hint, self.control_position, self.size)
        })
    }

    fn clearing_changes_content_layout(&self) -> bool {
        true
    }

    fn advance_loading_phase(&self, phase: f32) -> Option<crate::WidgetVisualHandle> {
        Some(crate::WidgetVisualHandle::new(Self {
            loading_phase: phase,
            ..self.clone()
        }))
    }

    fn intrinsic_chrome_content(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> crate::WidgetVisualContent {
        let inputs = self.inputs(id, world);
        let extent = inputs.extent();
        crate::WidgetVisualContent {
            width: extent.width,
            height: extent.height,
            text: if inputs.labelled { inputs.text } else { 0.0 },
        }
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _bounds: crate::LayoutBox,
        content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        Some(crate::layout_engine::parts::switch_parts(
            content,
            self.inputs(id, world),
        ))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        use crate::LayoutPartKind;
        let control = ctx
            .content_part(
                if self.control_position == nana_ui_core::SwitchControlPosition::End {
                    LayoutPartKind::TrailingContent
                } else {
                    LayoutPartKind::LeadingContent
                },
            )
            .unwrap_or(crate::LayoutBox {
                width: 0.0,
                height: 0.0,
                ..ctx.content
            });
        let label_bounds = ctx
            .content_part(LayoutPartKind::Label)
            .unwrap_or(crate::LayoutBox {
                width: 0.0,
                ..ctx.content
            });
        let hint_bounds = ctx.content_part(LayoutPartKind::Hint);
        let palette = ctx.style_model;
        let (hovered, pressed, disabled) = ctx.world.widget_interaction_state(ctx.id);
        let mix = |foreground: [f32; 4], background: [f32; 4], amount: f32| {
            let amount = amount.clamp(0.0, 1.0);
            std::array::from_fn(|channel| {
                foreground[channel] * amount + background[channel] * (1.0 - amount)
            })
        };
        let fade = |mut color: [f32; 4]| {
            if disabled {
                color[3] *= 0.55;
            }
            color
        };
        let track_background = fade(if self.checked {
            if pressed {
                palette
                    .color(nana_ui_core::SemanticColorRole::AccentStrong)
                    .as_rgba_array()
            } else {
                palette
                    .color(nana_ui_core::SemanticColorRole::Accent)
                    .as_rgba_array()
            }
        } else {
            mix(
                palette
                    .color(nana_ui_core::SemanticColorRole::Hover)
                    .as_rgba_array(),
                palette
                    .color(nana_ui_core::SemanticColorRole::Background)
                    .as_rgba_array(),
                0.78,
            )
        });
        let track_border = fade(if self.invalid {
            palette
                .color(nana_ui_core::SemanticColorRole::Danger)
                .as_rgba_array()
        } else if self.checked {
            if hovered || pressed {
                palette
                    .color(nana_ui_core::SemanticColorRole::AccentStrong)
                    .as_rgba_array()
            } else {
                palette
                    .color(nana_ui_core::SemanticColorRole::Accent)
                    .as_rgba_array()
            }
        } else if hovered || pressed {
            mix(
                palette
                    .color(nana_ui_core::SemanticColorRole::Accent)
                    .as_rgba_array(),
                palette
                    .color(nana_ui_core::SemanticColorRole::BorderStrong)
                    .as_rgba_array(),
                if pressed { 0.70 } else { 0.42 },
            )
        } else {
            palette
                .color(nana_ui_core::SemanticColorRole::BorderStrong)
                .as_rgba_array()
        });
        let thumb_background = fade(if self.checked {
            palette
                .color(nana_ui_core::SemanticColorRole::AccentText)
                .as_rgba_array()
        } else {
            mix(
                palette
                    .color(nana_ui_core::SemanticColorRole::Faint)
                    .as_rgba_array(),
                palette
                    .color(nana_ui_core::SemanticColorRole::Background)
                    .as_rgba_array(),
                0.70,
            )
        });
        let border_width = ctx.theme.border().hairline;
        let inset = control.height * (3.0 / 16.0);
        let thumb_extent =
            (control.height * (10.0 / 16.0)).min((control.width - 2.0 * inset).max(0.0));
        let travel = (control.width - 2.0 * inset - thumb_extent).max(0.0);
        let rtl = ctx.style.direction == nana_ui_core::DirSpec::Rtl;
        let progress = self.thumb_progress.clamp(0.0, 1.0);
        let progress = if rtl { 1.0 - progress } else { progress };
        let thumb = crate::LayoutBox {
            x: control.x + inset + travel * progress,
            y: control.y + (control.height - thumb_extent) / 2.0,
            width: thumb_extent,
            height: thumb_extent,
        };
        let label_color = ctx.text_color();
        let mut out = crate::WidgetPaintOutput {
            owns_node_text: true,
            root_surface: Some(crate::WidgetRootSurface {
                border_color: Some(None),
                border_width: Some(0.0),
                ..Default::default()
            }),
            ..crate::WidgetPaintOutput::new()
        };
        out.push(
            2,
            crate::WidgetPaintOp::Text(crate::WidgetText {
                bounds: label_bounds,
                content: crate::TextValue::from(Arc::clone(&self.label)),
                color: Some(label_color),
                paint_color: ctx.matching_text_paint(label_color),
                font_size: self.size.text_size_in(ctx.theme.typography()),
                font_weight: Some(ctx.theme.typography().medium),
                horizontal_alignment: crate::TextHorizontalAlignment::Start,
                vertical_alignment: crate::TextVerticalAlignment::Center,
                ellipsis: true,
                wrap: false,
                max_lines: None,
                italic: None,
                underline: None,
                line_through: None,
                suppress_spans: false,
                preserve_lines: None,
                unstyled_glyph: false,
                line_height: Some(Some(nana_ui_core::LineHeightSpec::Absolute(
                    label_bounds.height,
                ))),
                retained_text: crate::WidgetRetainedText::None,
                tabular_nums: false,
                ..Default::default()
            }),
        );
        if let Some((hint, hint_bounds)) = self.hint.as_ref().zip(hint_bounds) {
            let hint_color = ctx.secondary_text_color();
            out.push(
                3,
                crate::WidgetPaintOp::Text(crate::WidgetText {
                    bounds: hint_bounds,
                    content: crate::TextValue::from(Arc::clone(hint)),
                    color: Some(hint_color),
                    paint_color: ctx.matching_text_paint(hint_color),
                    font_size: self.size.caption_size_in(ctx.theme.typography()),
                    font_weight: Some(ctx.theme.typography().regular),
                    horizontal_alignment: crate::TextHorizontalAlignment::Start,
                    vertical_alignment: crate::TextVerticalAlignment::Center,
                    ellipsis: true,
                    wrap: false,
                    max_lines: None,
                    italic: None,
                    underline: None,
                    line_through: None,
                    suppress_spans: false,
                    preserve_lines: None,
                    unstyled_glyph: false,
                    line_height: Some(Some(nana_ui_core::LineHeightSpec::Absolute(
                        hint_bounds.height,
                    ))),
                    retained_text: crate::WidgetRetainedText::None,
                    tabular_nums: false,
                    ..Default::default()
                }),
            );
        }
        out.push(
            4,
            crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                rect: control,
                fill: Some(track_background),
                fill_paint: ctx.matching_background_paint(track_background),
                border: Some((track_border, border_width)),
                border_paint: ctx.matching_border_paint(track_border),
                corner_radius: [control.height / 2.0; 4],
                ..crate::WidgetQuad::default()
            }),
        );
        out.push(
            5,
            crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                rect: thumb,
                fill: Some(thumb_background),
                fill_paint: ctx.matching_text_paint(thumb_background),
                corner_radius: [thumb_extent / 2.0; 4],
                ..crate::WidgetQuad::default()
            }),
        );
        if self.loading {
            let spinner_fg = palette
                .color(
                    ctx.theme
                        .recipes()
                        .foreground(nana_ui_core::ComponentRecipeId::Switch, self.checked),
                )
                .as_rgba_array();
            out.push(
                6,
                crate::WidgetPaintOp::Icon(crate::WidgetIcon {
                    bounds: crate::LayoutBox {
                        x: if rtl {
                            control.x + control.width - control.height * 15.0 / 16.0
                        } else {
                            control.x + control.height / 16.0
                        },
                        y: control.y + control.height / 16.0,
                        width: control.height * 14.0 / 16.0,
                        height: control.height * 14.0 / 16.0,
                    },
                    glyph: crate::WidgetGlyph::Spinner {
                        phase: (self.loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                    },
                    color: Some(spinner_fg),
                    paint_color: ctx.matching_text_paint(spinner_fg),
                    align_to_adjacent_text: false,
                }),
            );
        }
        if ctx.is_focus_visible() {
            out.push(
                7,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: crate::LayoutBox {
                        x: control.x - 4.0 * border_width,
                        y: control.y - 4.0 * border_width,
                        width: control.width + 8.0 * border_width,
                        height: control.height + 8.0 * border_width,
                    },
                    fill: None,
                    border: ctx
                        .style
                        .border_color
                        .map(|color| (color, 2.0 * border_width)),
                    border_paint: ctx.style.paint_colors.border,
                    corner_radius: [control.height / 2.0 + 4.0 * border_width; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
        }
        out
    }
}
