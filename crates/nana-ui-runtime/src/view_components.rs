use std::fmt;
use std::sync::Arc;

use crate::{
    AccessibilityRole, AccessibilityState, InteractionState, MutationQueue, NodeKind, NodeStyle,
    OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId, StandardVisual, TextContent,
    TextHorizontalAlignment, TextInputState, TextVerticalAlignment, UiWorld,
};

fn control_layout(horizontal_padding: f32) -> Arc<nana_ui_core::LayoutStyle> {
    Arc::new(nana_ui_core::LayoutStyle {
        padding_left: Some(nana_ui_core::LengthSpec::Px(horizontal_padding)),
        padding_right: Some(nana_ui_core::LengthSpec::Px(horizontal_padding)),
        min_height: Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.control_height,
        )),
        border_width: Some(1.0),
        border_radius: Some(6.0),
        ..nana_ui_core::LayoutStyle::default()
    })
}

fn add_length_px(length: nana_ui_core::LengthSpec, offset: f32) -> nana_ui_core::LengthSpec {
    use nana_ui_core::LengthSpec;
    match length {
        LengthSpec::Px(value) => LengthSpec::Px(value + offset),
        LengthSpec::Percent(percent) => LengthSpec::CalcPercentOffset {
            percent,
            offset_px: offset,
        },
        LengthSpec::Em(em) => LengthSpec::CalcEmOffset {
            em,
            offset_px: offset,
        },
        LengthSpec::Rem(rem) => LengthSpec::CalcRemOffset {
            rem,
            offset_px: offset,
        },
        LengthSpec::Viewport { axis, value } => LengthSpec::CalcViewportOffset {
            axis,
            value,
            offset_px: offset,
        },
        LengthSpec::CalcPercentOffset { percent, offset_px } => LengthSpec::CalcPercentOffset {
            percent,
            offset_px: offset_px + offset,
        },
        LengthSpec::CalcEmOffset { em, offset_px } => LengthSpec::CalcEmOffset {
            em,
            offset_px: offset_px + offset,
        },
        LengthSpec::CalcRemOffset { rem, offset_px } => LengthSpec::CalcRemOffset {
            rem,
            offset_px: offset_px + offset,
        },
        LengthSpec::CalcViewportOffset {
            axis,
            value,
            offset_px,
        } => LengthSpec::CalcViewportOffset {
            axis,
            value,
            offset_px: offset_px + offset,
        },
        other => other,
    }
}

fn format_range_value(value: f64, step: f64) -> Arc<str> {
    let decimals = (0_i32..=6)
        .find(|decimals| {
            let scale = 10_f64.powi(*decimals);
            (step * scale - (step * scale).round()).abs() <= f64::EPSILON * scale.max(1.0)
        })
        .unwrap_or(6) as usize;
    Arc::from(format!("{value:.decimals$}"))
}

fn text_field_style(multiline: bool) -> NodeStyle {
    let mut layout = (*control_layout(nana_ui_core::UI_METRICS.field_padding_x)).clone();
    layout.width = Some(nana_ui_core::LengthSpec::Percent(100.0));
    layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
    layout.overflow_y = nana_ui_core::OverflowSpec::Hidden;
    if multiline {
        layout.padding_top = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.field_padding_x,
        ));
        layout.padding_bottom = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.field_padding_x,
        ));
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Relative(1.45));
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(96.0));
    } else {
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(
            nana_ui_core::ControlSize::Medium.line_height(),
        ));
        layout.white_space_nowrap = true;
    }
    NodeStyle {
        layout: Arc::new(layout),
        background: Some(nana_ui_core::SemanticColorRole::Background),
        border: Some(nana_ui_core::SemanticColorRole::Border),
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::BorderSoft),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                background: Some(nana_ui_core::SemanticColorRole::Subtle),
                border: Some(nana_ui_core::SemanticColorRole::Border),
            },
            ..crate::InteractionStyle::default()
        },
        text_vertical_alignment: if multiline {
            TextVerticalAlignment::Top
        } else {
            TextVerticalAlignment::Center
        },
        ..NodeStyle::default()
    }
}

struct TextFieldProjection<'a> {
    state: &'a TextInputState,
    label: &'a Option<Arc<str>>,
    disabled: bool,
    busy: bool,
    editable: bool,
    invalid: bool,
    multiline: bool,
    style: &'a NodeStyle,
}

fn project_text_field(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    field: TextFieldProjection<'_>,
) {
    if world.text_input(id) != Some(field.state) {
        mutations.set_text_input(id, Some(field.state.clone()));
    }
    if !field.editable && world.ime(id).is_some() {
        mutations.set_ime(id, None);
    }
    project_common(
        id,
        world,
        mutations,
        field.style,
        InteractionState {
            pointer_events: !field.disabled,
            focusable: !field.disabled,
        },
        AccessibilityState {
            role: AccessibilityRole::TextInput,
            label: field.label.clone(),
            disabled: field.disabled,
            busy: field.busy,
            invalid: field.invalid,
            multiline: field.multiline,
            editable: field.editable,
            ..AccessibilityState::default()
        },
    );
}

/// A Nana-native component projects its state into the retained runtime. The
/// backend consumes the resulting UiWorld/UiScene data; no renderer type is
/// part of this contract.
pub trait ComponentView: Clone + Send + 'static {
    fn node_kind(&self) -> NodeKind;
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue);
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub value: String,
    pub style: NodeStyle,
}

impl Text {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            style: NodeStyle::default(),
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Text {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Text
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let text = TextContent {
            value: self.value.clone(),
        };
        if world.text(id) != Some(text.value.as_str()) {
            mutations.set_text(id, text);
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    pub kind: nana_ui_core::ButtonKind,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub(crate) loading_phase: f32,
    pub invalid: bool,
    pub style: NodeStyle,
    pub(crate) style_override: bool,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        let mut layout = (*control_layout(nana_ui_core::UI_METRICS.control_padding_x)).clone();
        layout.font_weight = Some(500);
        Self {
            label: label.into(),
            kind: nana_ui_core::ButtonKind::Ghost,
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
            style: NodeStyle {
                layout: Arc::new(layout),
                foreground: Some(nana_ui_core::SemanticColorRole::Text),
                background: None,
                border: None,
                interaction: crate::InteractionStyle {
                    hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Hover),
                        ..SemanticPaint::default()
                    },
                    pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Active),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                        background: Some(nana_ui_core::SemanticColorRole::Subtle),
                        border: Some(nana_ui_core::SemanticColorRole::Border),
                    },
                    ..crate::InteractionStyle::default()
                },
                text_horizontal_alignment: TextHorizontalAlignment::Center,
                text_vertical_alignment: TextVerticalAlignment::Center,
            },
            style_override: false,
        }
    }

    pub fn kind(mut self, kind: nana_ui_core::ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    /// Replace host-owned outer layout without opting out of Button semantic
    /// paint. Apply [`Self::size`] afterwards when the size contract owns
    /// padding, height and typography.
    pub fn layout(mut self, layout: Arc<nana_ui_core::LayoutStyle>) -> Self {
        self.style.layout = layout;
        self
    }

    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.padding_left = Some(nana_ui_core::LengthSpec::Px(size.padding_x()));
        layout.padding_right = layout.padding_left;
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(size.height()));
        layout.font_size = Some(size.text_size());
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(size.line_height()));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
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
        self.style_override = true;
        self
    }
}

impl ComponentView for Button {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "button".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let text = TextContent {
            value: self.label.clone(),
        };
        if world.text(id) != Some(text.value.as_str()) {
            mutations.set_text(id, text);
        }
        let visual = StandardVisual::Button {
            label: Arc::from(self.label.as_str()),
            kind: self.kind,
            size: self.size,
            loading: self.loading,
            loading_phase: self.loading_phase,
            invalid: self.invalid,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        let layout = Arc::make_mut(&mut effective_style.layout);
        if self.loading {
            layout.padding_left = Some(
                layout
                    .padding_left
                    .map_or(nana_ui_core::LengthSpec::Px(10.0), |padding| {
                        add_length_px(padding, 10.0)
                    }),
            );
            layout.padding_right = Some(
                layout
                    .padding_right
                    .map_or(nana_ui_core::LengthSpec::Px(10.0), |padding| {
                        add_length_px(padding, 10.0)
                    }),
            );
        }
        if !self.style_override {
            effective_style.foreground = Some(match self.kind {
                nana_ui_core::ButtonKind::Primary => nana_ui_core::SemanticColorRole::AccentOnSoft,
                nana_ui_core::ButtonKind::Warning => nana_ui_core::SemanticColorRole::Warning,
                nana_ui_core::ButtonKind::Danger => nana_ui_core::SemanticColorRole::Danger,
                nana_ui_core::ButtonKind::Text => nana_ui_core::SemanticColorRole::Accent,
                nana_ui_core::ButtonKind::Ghost
                | nana_ui_core::ButtonKind::Subtle
                | nana_ui_core::ButtonKind::Selected => nana_ui_core::SemanticColorRole::Text,
            });
            effective_style.background = match self.kind {
                nana_ui_core::ButtonKind::Ghost
                | nana_ui_core::ButtonKind::Danger
                | nana_ui_core::ButtonKind::Text => None,
                nana_ui_core::ButtonKind::Subtle => Some(nana_ui_core::SemanticColorRole::Subtle),
                nana_ui_core::ButtonKind::Selected => {
                    Some(nana_ui_core::SemanticColorRole::Selected)
                }
                nana_ui_core::ButtonKind::Primary => {
                    Some(nana_ui_core::SemanticColorRole::AccentSoft)
                }
                nana_ui_core::ButtonKind::Warning => {
                    Some(nana_ui_core::SemanticColorRole::WarningSoft)
                }
            };
            effective_style.border = if self.kind == nana_ui_core::ButtonKind::Subtle {
                Some(nana_ui_core::SemanticColorRole::BorderSoft)
            } else {
                None
            };
            effective_style.interaction.hovered.background = Some(match self.kind {
                nana_ui_core::ButtonKind::Primary => {
                    nana_ui_core::SemanticColorRole::AccentSoftHover
                }
                nana_ui_core::ButtonKind::Warning => {
                    nana_ui_core::SemanticColorRole::WarningSoftHover
                }
                nana_ui_core::ButtonKind::Danger => {
                    nana_ui_core::SemanticColorRole::DangerSoftHover
                }
                nana_ui_core::ButtonKind::Selected => {
                    nana_ui_core::SemanticColorRole::SelectedHover
                }
                _ => nana_ui_core::SemanticColorRole::Hover,
            });
            effective_style.interaction.pressed.background = Some(match self.kind {
                nana_ui_core::ButtonKind::Primary => {
                    nana_ui_core::SemanticColorRole::AccentSoftPressed
                }
                nana_ui_core::ButtonKind::Warning => {
                    nana_ui_core::SemanticColorRole::WarningSoftPressed
                }
                nana_ui_core::ButtonKind::Danger => {
                    nana_ui_core::SemanticColorRole::DangerSoftPressed
                }
                nana_ui_core::ButtonKind::Selected => {
                    nana_ui_core::SemanticColorRole::SelectedPressed
                }
                _ => nana_ui_core::SemanticColorRole::Active,
            });
        }
        if self.invalid {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.hovered.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.pressed.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.focused.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
        }
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
                role: AccessibilityRole::Button,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled || self.loading,
                busy: self.loading,
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Compact action whose visible glyph is independent from its accessible name.
#[derive(Debug, Clone, PartialEq)]
pub struct IconButton {
    pub icon: nana_ui_core::Icon,
    pub label: Arc<str>,
    pub kind: nana_ui_core::ButtonKind,
    pub size: nana_ui_core::ControlSize,
    pub selected: bool,
    pub disabled: bool,
    pub tooltip: Option<IconButtonTooltip>,
    pub(crate) tooltip_open: bool,
    pub style: NodeStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IconButtonTooltip {
    pub label: Arc<str>,
    pub config: nana_ui_core::TooltipConfig,
}

impl IconButton {
    pub fn new(icon: nana_ui_core::Icon, label: impl Into<Arc<str>>) -> Self {
        let mut style = NodeStyle {
            foreground: Some(nana_ui_core::SemanticColorRole::Muted),
            interaction: crate::InteractionStyle {
                selected: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::AccentOnSoft),
                    background: Some(nana_ui_core::SemanticColorRole::AccentSoft),
                    ..SemanticPaint::default()
                },
                selected_hovered: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::AccentOnSoft),
                    background: Some(nana_ui_core::SemanticColorRole::AccentSoftHover),
                    ..SemanticPaint::default()
                },
                selected_pressed: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::AccentOnSoft),
                    background: Some(nana_ui_core::SemanticColorRole::AccentSoftPressed),
                    ..SemanticPaint::default()
                },
                hovered: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::Text),
                    background: Some(nana_ui_core::SemanticColorRole::Hover),
                    ..SemanticPaint::default()
                },
                pressed: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::Text),
                    background: Some(nana_ui_core::SemanticColorRole::Active),
                    ..SemanticPaint::default()
                },
                focused: SemanticPaint {
                    border: Some(nana_ui_core::SemanticColorRole::Accent),
                    ..SemanticPaint::default()
                },
                disabled: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                    ..SemanticPaint::default()
                },
            },
            text_horizontal_alignment: TextHorizontalAlignment::Center,
            text_vertical_alignment: TextVerticalAlignment::Center,
            ..NodeStyle::default()
        };
        style.layout = control_layout(nana_ui_core::UI_METRICS.compact_control_padding_x);
        let layout = Arc::make_mut(&mut style.layout);
        layout.padding_left = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.compact_control_padding_x,
        ));
        layout.padding_right = layout.padding_left;
        layout.min_width = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.icon_button_size,
        ));
        layout.min_height = layout.min_width;
        Self {
            icon,
            label: label.into(),
            kind: nana_ui_core::ButtonKind::Ghost,
            size: nana_ui_core::ControlSize::Medium,
            selected: false,
            disabled: false,
            tooltip: None,
            tooltip_open: false,
            style,
        }
    }

    pub fn kind(mut self, kind: nana_ui_core::ButtonKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.min_width = Some(nana_ui_core::LengthSpec::Px(size.height()));
        layout.min_height = layout.min_width;
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn tooltip(
        mut self,
        label: impl Into<Arc<str>>,
        config: nana_ui_core::TooltipConfig,
    ) -> Self {
        self.tooltip = Some(IconButtonTooltip {
            label: label.into(),
            config,
        });
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
}

impl ComponentView for IconButton {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "icon-button".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if self.tooltip.is_some() && world.overlay_host(id).is_none() {
            mutations.set_overlay_host(id, OverlayHostState::default());
        }
        let visual = StandardVisual::Icon {
            icon: self.icon,
            size: self.size.icon_size(),
            tooltip: self.tooltip.as_ref().map(|tooltip| crate::TooltipVisual {
                label: Arc::clone(&tooltip.label),
                config: tooltip.config,
                open: self.tooltip_open,
            }),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        effective_style.background = match self.kind {
            nana_ui_core::ButtonKind::Primary => Some(nana_ui_core::SemanticColorRole::Accent),
            nana_ui_core::ButtonKind::Subtle => Some(nana_ui_core::SemanticColorRole::Subtle),
            nana_ui_core::ButtonKind::Selected => Some(nana_ui_core::SemanticColorRole::Selected),
            _ => None,
        };
        effective_style.foreground = Some(match self.kind {
            nana_ui_core::ButtonKind::Primary => nana_ui_core::SemanticColorRole::AccentText,
            nana_ui_core::ButtonKind::Warning => nana_ui_core::SemanticColorRole::Warning,
            nana_ui_core::ButtonKind::Danger => nana_ui_core::SemanticColorRole::Danger,
            _ => nana_ui_core::SemanticColorRole::Muted,
        });
        effective_style.interaction.hovered.background = Some(match self.kind {
            nana_ui_core::ButtonKind::Primary => nana_ui_core::SemanticColorRole::AccentStrong,
            nana_ui_core::ButtonKind::Subtle
            | nana_ui_core::ButtonKind::Selected
            | nana_ui_core::ButtonKind::Ghost
            | nana_ui_core::ButtonKind::Warning
            | nana_ui_core::ButtonKind::Danger
            | nana_ui_core::ButtonKind::Text => nana_ui_core::SemanticColorRole::Hover,
        });
        effective_style.interaction.pressed.background = Some(match self.kind {
            nana_ui_core::ButtonKind::Primary => nana_ui_core::SemanticColorRole::AccentStrong,
            _ => nana_ui_core::SemanticColorRole::Active,
        });
        effective_style.interaction.selected.foreground =
            Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
        effective_style.interaction.selected.background =
            Some(nana_ui_core::SemanticColorRole::AccentSoft);
        effective_style.interaction.selected_hovered.foreground =
            Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
        effective_style.interaction.selected_hovered.background =
            Some(nana_ui_core::SemanticColorRole::AccentSoftHover);
        effective_style.interaction.selected_pressed.foreground =
            Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
        effective_style.interaction.selected_pressed.background =
            Some(nana_ui_core::SemanticColorRole::AccentSoftPressed);
        let selected = self.selected || self.kind == nana_ui_core::ButtonKind::Selected;
        if selected {
            effective_style.background = Some(nana_ui_core::SemanticColorRole::AccentSoft);
            effective_style.foreground = Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
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
                role: AccessibilityRole::Button,
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                selected: Some(selected),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Non-interactive content surface. Actions belong to explicit child controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub title: Option<Arc<str>>,
    pub kind: nana_ui_core::CardKind,
    pub loading: bool,
    pub(crate) loading_phase: f32,
    pub style: NodeStyle,
}

impl Card {
    pub fn new() -> Self {
        Self {
            title: None,
            kind: nana_ui_core::CardKind::Surface,
            loading: false,
            loading_phase: 0.0,
            style: NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    padding_left: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_x,
                    )),
                    padding_right: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_x,
                    )),
                    padding_top: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_y,
                    )),
                    padding_bottom: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_y,
                    )),
                    border_width: Some(1.0),
                    border_radius: Some(8.0),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                background: Some(nana_ui_core::SemanticColorRole::Surface),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                ..NodeStyle::default()
            },
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.title = Some(label.into());
        self
    }
    pub fn title(self, title: impl Into<Arc<str>>) -> Self {
        self.label(title)
    }
    pub fn kind(mut self, kind: nana_ui_core::CardKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    pub fn padding(mut self, padding: f32) -> Self {
        let layout = Arc::make_mut(&mut self.style.layout);
        let value = nana_ui_core::LengthSpec::Px(padding.max(0.0));
        layout.padding_left = Some(value);
        layout.padding_right = Some(value);
        layout.padding_top = Some(value);
        layout.padding_bottom = Some(value);
        self
    }
    pub fn height(mut self, height: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).height =
            Some(nana_ui_core::LengthSpec::Px(height.max(0.0)));
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for Card {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "card".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != self.title.as_deref() {
            mutations.set_text(
                id,
                TextContent {
                    value: self.title.as_deref().unwrap_or_default().to_owned(),
                },
            );
        }
        let visual = StandardVisual::Card {
            title: self.title.clone(),
            kind: self.kind,
            loading: self.loading,
            loading_phase: self.loading_phase,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        let layout = Arc::make_mut(&mut effective_style.layout);
        match self.kind {
            nana_ui_core::CardKind::Surface => {
                effective_style.background = Some(nana_ui_core::SemanticColorRole::Surface);
                effective_style.border = Some(nana_ui_core::SemanticColorRole::Border);
                layout.border_width = Some(1.0);
            }
            nana_ui_core::CardKind::Outlined => {
                effective_style.background = None;
                effective_style.border = Some(nana_ui_core::SemanticColorRole::BorderStrong);
                layout.border_width = Some(1.0);
            }
            nana_ui_core::CardKind::Raised => {
                effective_style.background = Some(nana_ui_core::SemanticColorRole::Surface);
                effective_style.border = Some(nana_ui_core::SemanticColorRole::BorderSoft);
                layout.border_width = Some(1.0);
            }
            nana_ui_core::CardKind::Flat => {
                effective_style.background = None;
                effective_style.border = None;
                layout.border_width = Some(0.0);
            }
            nana_ui_core::CardKind::Selected => {
                effective_style.background = Some(nana_ui_core::SemanticColorRole::Selected);
                effective_style.border = Some(nana_ui_core::SemanticColorRole::BorderSoft);
                layout.border_width = Some(1.0);
            }
        }
        if self.title.is_some() {
            let base =
                layout
                    .padding_top
                    .or(layout.padding)
                    .unwrap_or(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_y,
                    ));
            layout.padding_top = Some(add_length_px(base, 24.0));
        }
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState::default(),
            AccessibilityState {
                label: self.title.clone(),
                busy: self.loading,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub label: String,
    pub selected: bool,
    pub disabled: bool,
    pub(crate) slots: ListItemSlots,
    pub gap: f32,
    pub size: nana_ui_core::ControlSize,
    pub auto_height: bool,
    pub style: NodeStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListItemSlots {
    pub leading: Option<StableNodeId>,
    pub content: Option<StableNodeId>,
    pub trailing: Option<StableNodeId>,
}

impl ListItem {
    pub fn new(label: impl Into<String>) -> Self {
        let mut layout = (*control_layout(nana_ui_core::UI_METRICS.list_item_padding_x)).clone();
        layout.direction = Some(nana_ui_core::FlexDirection::Row);
        layout.align_items = nana_ui_core::AlignSpec::Center;
        layout.gap = Some(nana_ui_core::LengthSpec::Px(8.0));
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.selection_height,
        ));
        Self {
            label: label.into(),
            selected: false,
            disabled: false,
            slots: ListItemSlots::default(),
            gap: 8.0,
            size: nana_ui_core::ControlSize::Small,
            auto_height: false,
            style: NodeStyle {
                layout: Arc::new(layout),
                background: None,
                interaction: crate::InteractionStyle {
                    selected: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Selected),
                        ..SemanticPaint::default()
                    },
                    selected_hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::SelectedHover),
                        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
                        ..SemanticPaint::default()
                    },
                    selected_pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::SelectedPressed),
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                        ..SemanticPaint::default()
                    },
                    hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Hover),
                        ..SemanticPaint::default()
                    },
                    pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Active),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                        background: Some(nana_ui_core::SemanticColorRole::Subtle),
                        ..SemanticPaint::default()
                    },
                },
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            },
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
    /// Declarative projection hook for adapters that already own and validate
    /// the retained hierarchy. Direct Runtime consumers should prefer
    /// `AppContext::set_list_item_slots`, which validates and orders children
    /// atomically.
    pub fn slots(mut self, slots: ListItemSlots) -> Self {
        self.slots = slots;
        self
    }
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        Arc::make_mut(&mut self.style.layout).gap = Some(nana_ui_core::LengthSpec::Px(self.gap));
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        Arc::make_mut(&mut self.style.layout).min_height =
            Some(nana_ui_core::LengthSpec::Px(size.height()));
        self
    }
    pub fn auto_height(mut self, auto_height: bool) -> Self {
        self.auto_height = auto_height;
        if auto_height {
            Arc::make_mut(&mut self.style.layout).min_height = None;
        }
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for ListItem {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "list-item".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visible_label = if self.slots.content.is_none() {
            self.label.as_str()
        } else {
            ""
        };
        if world.text(id) != Some(visible_label) {
            mutations.set_text(
                id,
                TextContent {
                    value: visible_label.to_owned(),
                },
            );
        }
        let visual = StandardVisual::ListItem {
            leading: self.slots.leading,
            content: self.slots.content,
            trailing: self.slots.trailing,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::ListItem,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChanged {
    pub value: String,
    pub selection: crate::TextSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellFocused {
    pub row: usize,
    pub column: usize,
    pub cell: StableNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSelected {
    pub tab: StableNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleChanged {
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SliderChanged {
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxes {
    Horizontal,
    Vertical,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollChanged {
    pub offset: crate::ScrollOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayChanged {
    pub active: Option<StableNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderError {
    NonFinite,
    InvalidRange,
    OutOfRange,
}

impl fmt::Display for SliderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "slider values must be finite",
            Self::InvalidRange => "slider minimum must be less than maximum",
            Self::OutOfRange => "slider value must be within its range",
        })
    }
}

impl std::error::Error for SliderError {}

#[derive(Debug, Clone, PartialEq)]
pub struct TextInput {
    pub state: TextInputState,
    pub label: Option<Arc<str>>,
    pub placeholder: Arc<str>,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub read_only: bool,
    pub secure: bool,
    pub invalid: bool,
    pub style: NodeStyle,
    pub(crate) style_override: bool,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            state: TextInputState::new(value),
            label: None,
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            loading: false,
            read_only: false,
            secure: false,
            invalid: false,
            style: text_field_style(false),
            style_override: false,
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Replace host-owned outer layout while retaining TextInput semantic
    /// paint and interaction states.
    pub fn layout(mut self, layout: Arc<nana_ui_core::LayoutStyle>) -> Self {
        self.style.layout = layout;
        self
    }

    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.padding_left = Some(nana_ui_core::LengthSpec::Px(size.padding_x()));
        layout.padding_right = layout.padding_left;
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(size.height()));
        layout.font_size = Some(size.text_size());
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(size.line_height()));
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self.style_override = true;
        self
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        self.state.replace_selection(text)
    }
}

impl ComponentView for TextInput {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "input".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::TextInput {
            placeholder: Arc::clone(&self.placeholder),
            size: self.size,
            secure: self.secure,
            invalid: self.invalid,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        if self.invalid && !self.style_override {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.hovered.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.focused.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
        }
        project_text_field(
            id,
            world,
            mutations,
            TextFieldProjection {
                state: &self.state,
                label: &self.label,
                disabled: self.disabled || self.loading,
                busy: self.loading,
                editable: !self.disabled && !self.loading && !self.read_only,
                invalid: self.invalid,
                multiline: false,
                style: &effective_style,
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextArea {
    pub state: TextInputState,
    pub label: Option<Arc<str>>,
    pub placeholder: Arc<str>,
    pub disabled: bool,
    pub invalid: bool,
    pub scroll_offset: ScrollOffset,
    pub style: NodeStyle,
    pub(crate) style_override: bool,
}

impl TextArea {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            state: TextInputState::new(value),
            label: None,
            placeholder: Arc::from(""),
            disabled: false,
            invalid: false,
            scroll_offset: ScrollOffset::default(),
            style: text_field_style(true),
            style_override: false,
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() {
            Arc::make_mut(&mut self.style.layout).height = Some(nana_ui_core::LengthSpec::Px(
                height.max(nana_ui_core::ControlSize::Medium.height()),
            ));
        }
        self
    }

    pub fn scroll_offset(mut self, offset: ScrollOffset) -> Self {
        if offset.x.is_finite() && offset.y.is_finite() {
            self.scroll_offset = ScrollOffset {
                x: offset.x.max(0.0),
                y: offset.y.max(0.0),
            };
        }
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self.style_override = true;
        self
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        self.state.replace_selection(text)
    }
}

impl ComponentView for TextArea {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "textarea".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::TextInput {
            placeholder: Arc::clone(&self.placeholder),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: self.invalid,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        if world.scroll_offset(id) != Some(self.scroll_offset) {
            mutations.set_scroll_offset(id, self.scroll_offset);
        }
        let mut effective_style = self.style.clone();
        if self.invalid && !self.style_override {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.hovered.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
            effective_style.interaction.focused.border =
                Some(nana_ui_core::SemanticColorRole::Danger);
        }
        project_text_field(
            id,
            world,
            mutations,
            TextFieldProjection {
                state: &self.state,
                label: &self.label,
                disabled: self.disabled,
                busy: false,
                editable: !self.disabled,
                invalid: self.invalid,
                multiline: true,
                style: &effective_style,
            },
        );
        // Clear composition while this node is still focused, then release focus.
        // The world validates both transitions atomically in queue order.
        if self.disabled
            && let Some(node) = world.node(id)
            && world.focused(node.document) == Some(id)
        {
            mutations.request_focus(node.document, None);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OverlayHost {
    pub style: NodeStyle,
}

impl OverlayHost {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ComponentView for OverlayHost {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "overlay-host".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.overlay_host(id).is_none() {
            mutations.set_overlay_host(id, OverlayHostState::default());
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState::default(),
            AccessibilityState::default(),
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub title: Arc<str>,
    pub description: Option<Arc<str>>,
    pub size: nana_ui_core::DialogSize,
    pub close_policy: nana_ui_core::DialogClosePolicy,
    pub initial_focus: crate::ModalInitialFocus,
    pub slots: crate::ModalSlots,
    pub style: NodeStyle,
}

impl Dialog {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        let size = nana_ui_core::DialogSize::default();
        Self {
            title: title.into(),
            description: None,
            size,
            close_policy: nana_ui_core::DialogClosePolicy::default(),
            initial_focus: crate::ModalInitialFocus::default(),
            slots: crate::ModalSlots::default(),
            style: crate::overlay_surfaces::modal_root_style(),
        }
    }

    pub fn size(mut self, size: nana_ui_core::DialogSize) -> Self {
        self.size = size;
        self
    }

    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn close_policy(mut self, policy: nana_ui_core::DialogClosePolicy) -> Self {
        self.close_policy = policy;
        self
    }

    pub fn initial_focus(mut self, initial_focus: crate::ModalInitialFocus) -> Self {
        self.initial_focus = initial_focus;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Dialog {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dialog".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        crate::overlay_surfaces::project_modal(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::Dialog,
            &self.title,
            self.description.as_deref(),
            crate::ModalSurfaceKind::Dialog(self.size),
            false,
            false,
            &self.slots,
        );
    }
}

impl crate::ModalSurface for Dialog {
    fn slots(&self) -> &crate::ModalSlots {
        &self.slots
    }
    fn slots_mut(&mut self) -> &mut crate::ModalSlots {
        &mut self.slots
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl Menu {
    pub fn new() -> Self {
        Self {
            label: None,
            style: overlay_surface_style(320.0),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for Menu {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "menu".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Menu,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl MenuItem {
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        let button = Button::new(label.clone());
        Self {
            label,
            disabled: false,
            style: button.style,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for MenuItem {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "menuitem".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::MenuItem,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tooltip {
    pub label: Arc<str>,
    pub config: nana_ui_core::TooltipConfig,
    pub style: NodeStyle,
}

impl Tooltip {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self::with_config(label, nana_ui_core::TooltipConfig::default())
    }

    pub fn with_config(label: impl Into<Arc<str>>, config: nana_ui_core::TooltipConfig) -> Self {
        let mut style = overlay_surface_style(config.max_width.max(0.0));
        Arc::make_mut(&mut style.layout).max_width =
            Some(nana_ui_core::LengthSpec::Px(config.max_width.max(0.0)));
        Self {
            label: label.into(),
            config,
            style,
        }
    }
}

impl ComponentView for Tooltip {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "tooltip".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.to_string(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState::default(),
            AccessibilityState {
                role: AccessibilityRole::Tooltip,
                label: Some(Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

fn overlay_surface_style(max_width: f32) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            position: nana_ui_core::PositionSpec::Fixed,
            max_width: Some(nana_ui_core::LengthSpec::Px(max_width)),
            padding_left: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_x,
            )),
            padding_right: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_x,
            )),
            padding_top: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_y,
            )),
            padding_bottom: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_y,
            )),
            border_width: Some(1.0),
            border_radius: Some(nana_ui_core::UI_METRICS.radius_md),
            z_index: Some(1_000),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(nana_ui_core::SemanticColorRole::Surface),
        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
        ..NodeStyle::default()
    }
}

fn checkbox_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            min_height: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::ControlSize::Medium.height(),
            )),
            ..nana_ui_core::LayoutStyle::default()
        }),
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
            focused: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::Accent),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                background: Some(nana_ui_core::SemanticColorRole::Subtle),
                border: Some(nana_ui_core::SemanticColorRole::Border),
            },
        },
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

fn switch_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            border_radius: Some(nana_ui_core::UI_METRICS.radius_sm),
            ..nana_ui_core::LayoutStyle::default()
        }),
        foreground: Some(nana_ui_core::SemanticColorRole::Text),
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Hover),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Active),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::Accent),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                ..SemanticPaint::default()
            },
            ..crate::InteractionStyle::default()
        },
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checkbox {
    pub label: String,
    pub checked: bool,
    pub disabled: bool,
    pub invalid: bool,
    pub style: NodeStyle,
}

impl Checkbox {
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(),
            checked,
            disabled: false,
            invalid: false,
            style: checkbox_style(),
        }
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
                    value: self.label.clone(),
                },
            );
        }
        let visual = StandardVisual::Checkbox {
            checked: self.checked,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
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
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    pub label: String,
    pub hint: Option<Arc<str>>,
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
    pub fn control_position(mut self, position: nana_ui_core::SwitchControlPosition) -> Self {
        self.control_position = position;
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        Arc::make_mut(&mut self.style.layout).min_height =
            Some(nana_ui_core::LengthSpec::Px(size.height()));
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
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "switch".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id).is_some_and(|text| !text.is_empty()) {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new(),
                },
            );
        }
        let visual = StandardVisual::Switch {
            label: Arc::from(self.label.as_str()),
            hint: self.hint.clone(),
            checked: self.checked,
            control_position: self.control_position,
            size: self.size,
            loading: self.loading,
            loading_phase: self.loading_phase,
            invalid: self.invalid,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        let layout = Arc::make_mut(&mut effective_style.layout);
        if layout.width.is_none() {
            layout.width = Some(nana_ui_core::LengthSpec::Fill);
        }
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(if self.hint.is_some() {
            42.0
        } else {
            self.size.height()
        }));
        layout.padding_left = Some(nana_ui_core::LengthSpec::Px(self.size.padding_x()));
        layout.padding_right = layout.padding_left;
        if self.invalid {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
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
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                checked: Some(self.checked),
                busy: self.loading,
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slider {
    pub value: f32,
    pub minimum: f32,
    pub maximum: f32,
    pub label: Option<Arc<str>>,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl Slider {
    pub fn new(value: f32, minimum: f32, maximum: f32) -> Result<Self, SliderError> {
        if !value.is_finite() || !minimum.is_finite() || !maximum.is_finite() {
            return Err(SliderError::NonFinite);
        }
        if minimum >= maximum {
            return Err(SliderError::InvalidRange);
        }
        if !(minimum..=maximum).contains(&value) {
            return Err(SliderError::OutOfRange);
        }
        Ok(Self {
            value,
            minimum,
            maximum,
            label: None,
            disabled: false,
            style: NodeStyle {
                background: Some(nana_ui_core::SemanticColorRole::Accent),
                border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
                interaction: crate::InteractionStyle {
                    hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Faint),
                        border: Some(nana_ui_core::SemanticColorRole::Border),
                        ..SemanticPaint::default()
                    },
                    ..crate::InteractionStyle::default()
                },
                ..NodeStyle::default()
            },
        })
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
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

    pub fn ratio(&self) -> f32 {
        (self.value - self.minimum) / (self.maximum - self.minimum)
    }
}

impl ComponentView for Slider {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "slider".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::Slider {
            ratio: self.ratio(),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Slider,
                label: self.label.clone(),
                value: Some(Arc::from(self.value.to_string())),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeField {
    pub value: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub step: f64,
    pub page_step: f64,
    pub label: Option<Arc<str>>,
    pub unit: Option<Arc<str>>,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub invalid: bool,
    pub dragging: Option<RangeDragState>,
    pub style: NodeStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeDragState {
    pub pointer_id: u64,
    pub initial_value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeChanged {
    pub value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeAdjustment {
    Decrement,
    Increment,
    PageDecrement,
    PageIncrement,
    Minimum,
    Maximum,
}

impl RangeField {
    pub fn new(value: f64, minimum: f64, maximum: f64, step: f64) -> Result<Self, SliderError> {
        if !value.is_finite() || !minimum.is_finite() || !maximum.is_finite() || !step.is_finite() {
            return Err(SliderError::NonFinite);
        }
        if minimum >= maximum || step <= 0.0 {
            return Err(SliderError::InvalidRange);
        }
        if !(minimum..=maximum).contains(&value) {
            return Err(SliderError::OutOfRange);
        }
        let mut field = Self {
            value,
            minimum,
            maximum,
            step,
            page_step: step * 10.0,
            label: None,
            unit: None,
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            invalid: false,
            dragging: None,
            style: Slider::new(value as f32, minimum as f32, maximum as f32)?.style,
        };
        field.value = field.quantize(value);
        Ok(field)
    }
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn unit(mut self, unit: impl Into<Arc<str>>) -> Self {
        self.unit = Some(unit.into());
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
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
    pub fn page_step(mut self, page_step: f64) -> Result<Self, SliderError> {
        if !page_step.is_finite() {
            return Err(SliderError::NonFinite);
        }
        if page_step <= 0.0 {
            return Err(SliderError::InvalidRange);
        }
        self.page_step = page_step;
        Ok(self)
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn ratio(&self) -> f32 {
        ((self.value - self.minimum) / (self.maximum - self.minimum)) as f32
    }
    pub fn quantize(&self, value: f64) -> f64 {
        let steps = ((value.clamp(self.minimum, self.maximum) - self.minimum) / self.step).round();
        (self.minimum + steps * self.step).clamp(self.minimum, self.maximum)
    }
}

impl ComponentView for RangeField {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "range-field".into(),
        }
    }
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let value = format_range_value(self.value, self.step);
        let visual = StandardVisual::Range {
            label: self.label.clone(),
            value: Arc::clone(&value),
            unit: self.unit.clone(),
            size: self.size,
            ratio: self.ratio(),
            invalid: self.invalid,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        if world.text(id).is_some_and(|text| !text.is_empty()) {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new(),
                },
            );
        }
        let mut effective_style = self.style.clone();
        let layout = Arc::make_mut(&mut effective_style.layout);
        if layout.width.is_none() {
            layout.width = Some(nana_ui_core::LengthSpec::Fill);
        }
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(self.size.height()));
        layout.padding_left = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.field_padding_x,
        ));
        layout.padding_right = layout.padding_left;
        if self.invalid {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
        }
        effective_style.interaction.disabled.foreground =
            Some(nana_ui_core::SemanticColorRole::Muted);
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
                role: AccessibilityRole::Slider,
                label: self.label.clone(),
                value: Some(match &self.unit {
                    Some(unit) => Arc::from(format!("{} {}", value, unit)),
                    None => value,
                }),
                disabled: self.disabled,
                invalid: self.invalid,
                numeric_minimum: Some(self.minimum),
                numeric_maximum: Some(self.maximum),
                numeric_step: Some(self.step),
                numeric_value: Some(self.value),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabList {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl TabList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TabList {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "tablist".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::TabList,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub label: String,
    pub selected: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl Tab {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            selected: false,
            disabled: false,
            style: NodeStyle {
                layout: control_layout(nana_ui_core::UI_METRICS.selection_padding_x),
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                background: Some(nana_ui_core::SemanticColorRole::Surface),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                interaction: crate::InteractionStyle {
                    selected: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Text),
                        background: Some(nana_ui_core::SemanticColorRole::Selected),
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                    },
                    selected_hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::SelectedHover),
                        ..SemanticPaint::default()
                    },
                    selected_pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::SelectedPressed),
                        ..SemanticPaint::default()
                    },
                    hovered: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Text),
                        background: Some(nana_ui_core::SemanticColorRole::Hover),
                        ..SemanticPaint::default()
                    },
                    pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Active),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                        background: Some(nana_ui_core::SemanticColorRole::Subtle),
                        border: Some(nana_ui_core::SemanticColorRole::BorderSoft),
                    },
                },
                text_horizontal_alignment: TextHorizontalAlignment::Center,
                text_vertical_alignment: TextVerticalAlignment::Center,
            },
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
}

impl ComponentView for Tab {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "tab".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Tab,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollView {
    pub axes: ScrollAxes,
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl ScrollView {
    pub fn new(axes: ScrollAxes) -> Self {
        Self {
            axes,
            label: None,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn projected_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        if matches!(self.axes, ScrollAxes::Horizontal | ScrollAxes::Both) {
            layout.overflow_x = nana_ui_core::OverflowSpec::Scroll;
        }
        if matches!(self.axes, ScrollAxes::Vertical | ScrollAxes::Both) {
            layout.overflow_y = nana_ui_core::OverflowSpec::Scroll;
        }
        style
    }
}

impl ComponentView for ScrollView {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "scroll".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.projected_style(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct List {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl List {
    pub fn new() -> Self {
        Self {
            label: None,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for List {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for List {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "list".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::List,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Table {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "table".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Table,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableRow {
    pub selected: bool,
    pub style: NodeStyle,
}

impl TableRow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TableRow {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "tr".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Row,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub value: String,
    pub column_header: bool,
    pub selected: bool,
    pub style: NodeStyle,
}

impl TableCell {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            column_header: false,
            selected: false,
            style: NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    padding_left: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.list_item_padding_x,
                    )),
                    padding_right: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.list_item_padding_x,
                    )),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            },
        }
    }

    pub fn column_header(mut self, column_header: bool) -> Self {
        self.column_header = column_header;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TableCell {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: if self.column_header { "th" } else { "td" }.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.value.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.value.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: if self.column_header {
                    AccessibilityRole::ColumnHeader
                } else {
                    AccessibilityRole::Cell
                },
                label: Some(Arc::from(self.value.as_str())),
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

pub(crate) fn project_common(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    style: &NodeStyle,
    interaction: InteractionState,
    accessibility: AccessibilityState,
) {
    if world.node_style(id) != Some(style) {
        mutations.set_style(id, style.clone());
    }
    if world.interaction(id) != Some(interaction) {
        mutations.set_interaction(id, interaction);
    }
    if world.accessibility(id) != Some(&accessibility) {
        mutations.set_accessibility(id, accessibility);
    }
}
