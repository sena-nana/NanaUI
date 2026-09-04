use std::fmt;
use std::sync::Arc;

use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId,
    StandardVisual, TextCodeFold, TextColorSwatchSpan, TextCompletion, TextContent,
    TextDiagnosticSpan, TextEditorRenderOptions, TextGitMark, TextHorizontalAlignment, TextHover,
    TextInlay, TextInputState, TextMatchSpan, TextVerticalAlignment, UiWorld,
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

fn range_field_style() -> NodeStyle {
    NodeStyle {
        background: Some(nana_ui_core::SemanticColorRole::Accent),
        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::AccentStrong),
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
    }
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
                border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
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
    highlight: Option<&'a HighlightRequest>,
    /// Committed numeric value and its rules, for a spinner field.
    numeric: Option<(f64, nana_ui_core::NumberFieldSpec)>,
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
    if world.highlight_request(id) != field.highlight {
        mutations.set_highlight_request(id, field.highlight.cloned());
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
            numeric_minimum: field.numeric.and_then(|(_, spec)| spec.minimum),
            numeric_maximum: field.numeric.and_then(|(_, spec)| spec.maximum),
            numeric_step: field.numeric.map(|(_, spec)| spec.effective_step()),
            numeric_value: field.numeric.map(|(value, _)| value),
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

    /// Opt in to one reprojection when this node's child structure changes
    /// (child insert, detach or despawn under this node) even though the
    /// component's own data did not change. Components that probe the retained
    /// subtree during [`Self::project`] and memoize the result into visual or
    /// text state need this to avoid stale snapshots taken before children
    /// were attached. Defaults to `false`; existing components keep their
    /// data-change-only reprojection schedule.
    fn wants_child_reproject() -> bool
    where
        Self: Sized,
    {
        false
    }
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
                | nana_ui_core::ButtonKind::Selected
                | nana_ui_core::ButtonKind::Menu => nana_ui_core::SemanticColorRole::Text,
            });
            effective_style.background = match self.kind {
                nana_ui_core::ButtonKind::Ghost
                | nana_ui_core::ButtonKind::Danger
                | nana_ui_core::ButtonKind::Text => None,
                nana_ui_core::ButtonKind::Subtle | nana_ui_core::ButtonKind::Menu => {
                    Some(nana_ui_core::SemanticColorRole::Subtle)
                }
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
            effective_style.border = if matches!(
                self.kind,
                nana_ui_core::ButtonKind::Subtle | nana_ui_core::ButtonKind::Menu
            ) {
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
                focused: SemanticPaint::default(),
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

    /// Tooltip with [`nana_ui_core::TooltipConfig::default`].
    pub fn with_tooltip(self, label: impl Into<Arc<str>>) -> Self {
        self.tooltip(label, nana_ui_core::TooltipConfig::default())
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
            | nana_ui_core::ButtonKind::Text
            | nana_ui_core::ButtonKind::Menu => nana_ui_core::SemanticColorRole::Hover,
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

/// Non-interactive glyph for inline decoration. Hosts that need a hit target or
/// hover paint use [`IconButton`] instead.
#[derive(Debug, Clone, PartialEq)]
pub struct IconGlyph {
    pub icon: nana_ui_core::Icon,
    pub size: f32,
    pub style: NodeStyle,
}

impl IconGlyph {
    pub fn new(icon: nana_ui_core::Icon) -> Self {
        Self {
            icon,
            size: nana_ui_core::ControlSize::Small.icon_size(),
            style: NodeStyle {
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                ..NodeStyle::default()
            },
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(0.0);
        self
    }

    pub fn role(mut self, role: nana_ui_core::SemanticColorRole) -> Self {
        self.style.foreground = Some(role);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        let edge = nana_ui_core::LengthSpec::Px(self.size);
        layout.width = Some(edge);
        layout.height = Some(edge);
        layout.min_width = Some(edge);
        layout.min_height = Some(edge);
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }
}

impl ComponentView for IconGlyph {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "icon-glyph".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new(),
                },
            );
        }
        let visual = StandardVisual::Icon {
            icon: self.icon,
            size: self.size,
            tooltip: None,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
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
            AccessibilityState::default(),
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
            // 背景、边框、圆角不在此预填：project 按 kind 提供默认值，
            // 用户经 `.style(...)` 显式给出的值优先于 kind。
            style: NodeStyle::default(),
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
    /// Replace all four padding edges, including logical declarations.
    pub fn padding(self, padding: f32) -> Self {
        self.padding_xy(padding, padding)
    }

    pub fn padding_xy(mut self, x: f32, y: f32) -> Self {
        replace_padding_xy(Arc::make_mut(&mut self.style.layout), x, y);
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
        // Defaults belong to the projection, never the authored declaration.
        if layout.padding.is_none() {
            let x = nana_ui_core::LengthSpec::Px(nana_ui_core::UI_METRICS.panel_padding_x);
            let y = nana_ui_core::LengthSpec::Px(nana_ui_core::UI_METRICS.panel_padding_y);
            layout.padding_left.get_or_insert(x);
            layout.padding_right.get_or_insert(x);
            layout.padding_top.get_or_insert(y);
            layout.padding_bottom.get_or_insert(y);
            if layout.logical_padding.has_logical() {
                layout.logical_padding.phys_left.get_or_insert(x);
                layout.logical_padding.phys_right.get_or_insert(x);
            }
        }

        let (kind_background, kind_border, kind_border_width) = match self.kind {
            nana_ui_core::CardKind::Surface | nana_ui_core::CardKind::Raised => {
                (Some(nana_ui_core::SemanticColorRole::Surface), None, 0.0)
            }
            nana_ui_core::CardKind::Outlined => {
                (None, Some(nana_ui_core::SemanticColorRole::Border), 1.0)
            }
            nana_ui_core::CardKind::Flat => (None, None, 0.0),
            nana_ui_core::CardKind::Selected => (
                Some(nana_ui_core::SemanticColorRole::Selected),
                Some(nana_ui_core::SemanticColorRole::BorderSoft),
                1.0,
            ),
        };
        if effective_style.background.is_none() {
            effective_style.background = kind_background;
        }
        if effective_style.border.is_none() {
            effective_style.border = kind_border;
        }
        if layout.border_width.is_none() {
            layout.border_width = Some(kind_border_width);
        }
        if layout.border_radius.is_none() {
            layout.border_radius = Some(world.theme_metrics().radius_md);
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
    /// 单行补充信息：非空且未挂 content 槽时渲染为行尾右对齐的小字号
    /// muted 文本区，不改变行高。
    pub detail: String,
    pub selected: bool,
    pub disabled: bool,
    pub(crate) slots: ListItemSlots,
    pub gap: f32,
    pub size: nana_ui_core::ControlSize,
    pub auto_height: bool,
    /// 行 pill 相对文本线水平外扩，语义见 [`ListItem::pill_bleed`]。
    pub pill_bleed: bool,
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
        layout.font_size = Some(nana_ui_core::ControlSize::Small.text_size());
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(
            nana_ui_core::ControlSize::Small.text_size(),
        ));
        // 列表行是单行原语：过长文本截断省略，不折行撑破固定行高。
        layout.white_space_nowrap = true;
        layout.text_overflow_ellipsis = true;
        Self {
            label: label.into(),
            detail: String::new(),
            selected: false,
            disabled: false,
            slots: ListItemSlots::default(),
            gap: 8.0,
            size: nana_ui_core::ControlSize::Small,
            auto_height: false,
            pill_bleed: false,
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

    /// 设置单行补充信息；空串清除。仅在未挂 content 槽时生效。
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
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
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(size.height()));
        layout.font_size = Some(size.text_size());
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(size.text_size()));
        self
    }
    pub fn auto_height(mut self, auto_height: bool) -> Self {
        self.auto_height = auto_height;
        if auto_height {
            Arc::make_mut(&mut self.style.layout).min_height = None;
        }
        self
    }

    /// 行 pill 相对文本线水平外扩一个内边距：margin 取自身水平 padding
    /// 的负值，文本内缩不变。用于宿主面板把行文本对齐到内容线、同时
    /// pill 保留文本呼吸区的场景。
    pub fn pill_bleed(mut self, pill_bleed: bool) -> Self {
        self.pill_bleed = pill_bleed;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// 行内子节点的对齐语义：有 trailing 槽时按钮簇贴行尾（与 SidebarRow
    /// 的行工具同款规则），否则按 start 排。隐藏的槽位子节点不参与 flex 流，
    /// 不影响其余子节点的分布。
    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.justify_content = if self.slots.trailing.is_some() {
            if self.slots.leading.is_some() {
                nana_ui_core::JustifySpec::SpaceBetween
            } else {
                nana_ui_core::JustifySpec::End
            }
        } else {
            nana_ui_core::JustifySpec::Start
        };
        if self.pill_bleed {
            // 对称外扩与书写方向无关；非 `Px` padding 视为 0，不外扩。
            let bleed = |edge: &Option<nana_ui_core::LengthSpec>| match edge {
                Some(nana_ui_core::LengthSpec::Px(px)) => nana_ui_core::LengthSpec::Px(-px),
                _ => nana_ui_core::LengthSpec::Px(0.0),
            };
            layout.margin_left = Some(bleed(&layout.padding_left));
            layout.margin_right = Some(bleed(&layout.padding_right));
        }
        style
    }
}

impl ComponentView for ListItem {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "list-item".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // 行文本只承载 label；detail 由几何端生成右对齐的小字号 muted
        // 文本区，与 label 同行但不挤占同一文本。挂了 content 槽的行由槽
        // 内容自绘，label 与 detail 都不渲染。
        let has_detail = self.slots.content.is_none() && !self.detail.is_empty();
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
            detail: has_detail.then(|| Arc::from(self.detail.as_str())),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut accessible = self.label.clone();
        if has_detail {
            accessible.push_str("  ");
            accessible.push_str(&self.detail);
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
                role: AccessibilityRole::ListItem,
                label: Some(Arc::from(accessible.as_str())),
                disabled: self.disabled,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activate;

/// Secondary (right) pointer press, delivered to the nearest handler at or
/// above the hit node.
///
/// The framework opens nothing: whether a menu appears, and what is in it, is
/// the application's call. `target` is the node actually hit, which may be a
/// descendant of the handler's own node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SecondaryPress {
    pub target: StableNodeId,
    pub x: f32,
    pub y: f32,
}

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
pub struct ToggleChanged {
    pub checked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxes {
    Horizontal,
    Vertical,
    Both,
}

impl ScrollAxes {
    pub fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    pub fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    pub fn covers(self, axis: nana_ui_core::ScrollbarAxis) -> bool {
        match axis {
            nana_ui_core::ScrollbarAxis::Horizontal => self.horizontal(),
            nana_ui_core::ScrollbarAxis::Vertical => self.vertical(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollChanged {
    pub offset: crate::ScrollOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayChanged {
    pub active: Option<StableNodeId>,
}

/// Logical dismissal starts now; the host retains this root until its exit finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayClosing {
    pub root: StableNodeId,
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
    pub highlight: Option<HighlightRequest>,
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
            highlight: None,
            style_override: false,
        }
    }

    /// Color committed text with the registered `"highlight"` presenter.
    pub fn highlight(mut self, language: impl Into<Arc<str>>) -> Self {
        self.highlight = Some(HighlightRequest::highlight(language));
        self
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
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            color_swatches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            git_marks: Arc::from([]),
            editor_options: TextEditorRenderOptions::default(),
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
                highlight: self.highlight.as_ref(),
                numeric: None,
            },
        );
    }
}

/// Numeric spinner: a text field that also steps.
///
/// `value` is the committed authority; `state` carries the in-progress draft
/// while the user types. Typing never rewrites `value` — the draft is parsed on
/// commit (Enter or blur), and an unparseable draft restores the last committed
/// value instead of inventing one. Stepping and arrow keys work on `value`
/// directly, so a half-typed draft cannot leak into a stepped result.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberInput {
    pub state: TextInputState,
    pub label: Option<Arc<str>>,
    pub placeholder: Arc<str>,
    pub spec: nana_ui_core::NumberFieldSpec,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub read_only: bool,
    pub invalid: bool,
    pub style: NodeStyle,
    pub(crate) value: f64,
    pub(crate) style_override: bool,
}

impl NumberInput {
    pub fn new(value: f64) -> Self {
        let spec = nana_ui_core::NumberFieldSpec::default();
        let value = spec.snap(value);
        Self {
            state: TextInputState::new(spec.format(value)),
            label: None,
            placeholder: Arc::from(""),
            spec,
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            read_only: false,
            invalid: false,
            style: text_field_style(false),
            value,
            style_override: false,
        }
    }

    pub fn range(mut self, minimum: f64, maximum: f64) -> Self {
        self.spec.minimum = Some(minimum);
        self.spec.maximum = Some(maximum);
        self.resync();
        self
    }

    pub fn minimum(mut self, minimum: f64) -> Self {
        self.spec.minimum = Some(minimum);
        self.resync();
        self
    }

    pub fn maximum(mut self, maximum: f64) -> Self {
        self.spec.maximum = Some(maximum);
        self.resync();
        self
    }

    pub fn step(mut self, step: f64) -> Self {
        self.spec.step = step;
        self.resync();
        self
    }

    pub fn precision(mut self, precision: u8) -> Self {
        self.spec.precision = precision;
        self.resync();
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.placeholder = placeholder.into();
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

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
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

    /// Last committed value.
    pub const fn value(&self) -> f64 {
        self.value
    }

    pub(crate) fn accepts_input(&self) -> bool {
        !self.disabled && !self.read_only
    }

    /// Publish a value from the application. Rejects nothing: the value is
    /// snapped and clamped into the field's own rules.
    pub(crate) fn assign(&mut self, value: f64) -> bool {
        let next = self.spec.snap(value);
        if next == self.value && self.state.value == self.spec.format(next) {
            return false;
        }
        self.value = next;
        self.state.replace_value(self.spec.format(next));
        true
    }

    /// Move by grid positions from the committed value.
    pub(crate) fn step_value(&mut self, steps: i32) -> bool {
        self.assign(self.spec.step_by(self.value, steps))
    }

    /// Parse the draft. An unparseable draft restores the committed value.
    pub(crate) fn commit_draft(&mut self) -> bool {
        match self.spec.parse(&self.state.value) {
            Some(parsed) => self.assign(parsed),
            None => {
                let restored = self.spec.format(self.value);
                if self.state.value == restored {
                    return false;
                }
                self.state.replace_value(restored);
                true
            }
        }
    }

    fn resync(&mut self) {
        self.value = self.spec.snap(self.value);
        self.state.replace_value(self.spec.format(self.value));
    }
}

impl ComponentView for NumberInput {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "number-input".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::TextInput {
            placeholder: Arc::clone(&self.placeholder),
            size: self.size,
            secure: false,
            invalid: self.invalid,
            steppers: true,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            color_swatches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            git_marks: Arc::from([]),
            editor_options: TextEditorRenderOptions::default(),
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
                disabled: self.disabled,
                busy: false,
                editable: self.accepts_input(),
                invalid: self.invalid,
                multiline: false,
                style: &effective_style,
                highlight: None,
                numeric: Some((self.value, self.spec)),
            },
        );
    }
}

/// Reported after the committed numeric value changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberChanged {
    pub value: f64,
}

/// Code-editor behaviors enabled on a [`TextArea`].
///
/// The runtime applies bracket pairing, indentation, and line comments on the
/// raw committed text; syntax coloring stays with the `"highlight"`
/// presenter.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeEditing {
    /// Line comment token for toggle commands (for example `"//"`).
    pub comment_prefix: Arc<str>,
    /// Indentation inserted by Tab and copied by auto-indent.
    pub indent_unit: Arc<str>,
}

impl CodeEditing {
    pub fn new(comment_prefix: impl Into<Arc<str>>, indent_unit: impl Into<Arc<str>>) -> Self {
        Self {
            comment_prefix: comment_prefix.into(),
            indent_unit: indent_unit.into(),
        }
    }

    /// Default WGSL-style configuration: `//` comments, tab indentation.
    pub fn wgsl() -> Self {
        Self::new("//", "\t")
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
    pub highlight: Option<HighlightRequest>,
    /// 编译诊断 span 标记（错误/警告下划线）。宿主在文本变化后负责更新或
    /// 清除；渲染层只钳制越界偏移。
    pub diagnostics: Arc<[TextDiagnosticSpan]>,
    /// 查找匹配高亮 span（普通匹配与当前匹配）。宿主在文本变化后负责更新
    /// 或清除；渲染层只钳制越界偏移。
    pub match_spans: Arc<[TextMatchSpan]>,
    /// 颜色装饰 span（见 [`TextColorSwatchSpan`]）。宿主在文本变化后负责
    /// 更新或清除；渲染层只钳制越界偏移，纯装饰不参与命中。
    pub color_swatches: Arc<[TextColorSwatchSpan]>,
    /// 行号栏。行号绘制在节点左内边距区域，宿主需预留足够的 padding-left。
    pub line_numbers: bool,
    /// 代码编辑行为（括号配对、缩进、注释切换）。`None` 时为普通多行文本。
    pub code_editing: Option<CodeEditing>,
    /// 代码折叠区间（见 [`TextCodeFold`]）。宿主在每次文本变化后重新喂；
    /// 哪些区间处于折叠态由组件内部维护（宿主重喂时按区间匹配保留，
    /// 漂移的尽力平移匹配，失效的自动展开）。折叠是纯视图状态，不改值。
    pub code_folds: Arc<[TextCodeFold]>,
    /// 行内提示（inlay，见 [`TextInlay`]）。纯插入型显示 span：插入文本
    /// 参与布局测量与软换行，但不占缓冲 offset（光标/点击/查找经显示
    /// 视图映射自动免疫）。宿主在语义快照就绪时喂入；快照失配/偏好关
    /// 时撤空列表。世界校验后按 `(offset, label)` 排序去重；IME 组合期
    /// 与折叠隐藏区间内的条目不显示。空列表零成本。
    pub inlays: Arc<[TextInlay]>,
    /// git gutter 标记（见 [`TextGitMark`]）。宿主在 git 状态与文本变化后
    /// 重新喂：`line` 为 1 基逻辑行号，渲染为 gutter 最左侧 2px 竖条。
    /// 行号 0、超过文档逻辑行数或被折叠隐藏的标记静默跳过；空列表零成本。
    /// 宿主需预留足够的 padding-left（gutter 与行号共用左侧区域）。
    pub git_gutter: Arc<[TextGitMark]>,
    /// 补全候选（见 [`TextCompletion`]）。过滤完全由宿主负责：宿主按当前
    /// 词前缀过滤后在文本/光标变化时重新喂入，非空列表激活候选会话（弹层
    /// 锚定主光标行），空列表关闭。会话由组件内部管理：键盘选中与滚动
    /// 存放在组件内部状态里，重喂相同列表（指针或内容相等）不下发变更，
    /// 选中保持、已 Esc 关闭的弹层也不复活；重喂不同列表视为新会话
    /// （选中归零、重新打开）。
    pub completions: Arc<[TextCompletion]>,
    /// hover 文档浮窗（见 [`TextHover`]）。`Some` 显示在偏移所在行附近，
    /// `None` 隐藏；触发与生命周期完全由宿主决定，文本编辑时宿主负责撤掉
    /// （组件不自动隐藏）。正文滚动存放在组件内部状态里：重喂相同文档
    /// 不下发变更（滚动位置保留），换新文档重新显示并回到顶部。
    pub hover: Option<TextHover>,
    /// 光标处单词/选中文本的出现高亮（内部派生：聚焦时从主光标/选区
    /// 派生查询并扫描全文档，不给宿主增加喂入负担）。默认 `false`：
    /// 派生按出现次数做 shaper 探针（上限 200），成本高于同类的括号
    /// 匹配（常数次探针），由宿主按文档形态自行开启。
    pub occurrence_highlight: bool,
    /// 相对行号：光标行显示绝对行号，其余行显示与光标所在显示行的距离
    /// （Zed 惯例）。仅在 [`Self::line_numbers`] 开启时生效。默认 `false`。
    pub relative_line_numbers: bool,
    /// 空白字符显示：空格画中点、Tab 画箭头；行首缩进与行尾空白一并
    /// 可见。默认 `false`。
    pub show_whitespace: bool,
    /// wrap guide 列参考线：在给定字符列的 x 位置画全高竖线（1px）。列宽
    /// 按 `'0'` 字形宽度估算（等宽字体假设）；文档最宽行不足该列时不画。
    /// 默认为空。
    pub wrap_guides: Arc<[usize]>,
    /// 代码编辑器 minimap：内容区右缘 64px 覆盖竖条（与内容区间 1px
    /// 分隔线），每逻辑行一条 2px 行条（宽度 ∝ 行非空白长度），半透明
    /// 指示器跟随视口；点击条内滚动到对应行（点击行居中），按住拖动
    /// 连续跟随，滚轮落在条上仍按编辑器常规滚动。折叠只影响主视图，
    /// minimap 显示全部逻辑行；文本行宽计算不变——极长行会被条遮挡。
    /// 仅多行编辑器生效。默认 `false`。
    pub minimap: bool,
    /// sticky scroll：滚动视口顶部落在 [`Self::code_folds`] 喂入的区间
    /// 内部时，在内容区顶部钉住显示该区间头行（取首视觉行）；嵌套区间
    /// 钉最内层。区间头自然滚回视口时钉住行消失。折叠语义不改变钉住
    /// 逻辑；钉住行是纯装饰只读渲染。仅多行编辑器生效。默认 `false`。
    pub sticky_scroll: bool,
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
            highlight: None,
            diagnostics: Arc::from([]),
            match_spans: Arc::from([]),
            color_swatches: Arc::from([]),
            line_numbers: false,
            code_editing: None,
            code_folds: Arc::from([]),
            inlays: Arc::from([]),
            git_gutter: Arc::from([]),
            completions: Arc::from([]),
            hover: None,
            occurrence_highlight: false,
            relative_line_numbers: false,
            show_whitespace: false,
            wrap_guides: Arc::from([]),
            minimap: false,
            sticky_scroll: false,
            style_override: false,
        }
    }

    /// 启用代码编辑行为：括号配对、Enter 自动缩进、Tab/Shift+Tab 缩进与
    /// 注释切换。软折行保持组件原有样式。
    pub fn code_editor(mut self, enabled: bool) -> Self {
        self.code_editing = enabled.then(CodeEditing::wgsl);
        self
    }

    /// 设置诊断 span 标记（见 [`TextDiagnosticSpan`]）。
    pub fn diagnostics(mut self, diagnostics: Arc<[TextDiagnosticSpan]>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// 设置查找匹配高亮 span（见 [`TextMatchSpan`]）。
    pub fn match_spans(mut self, match_spans: Arc<[TextMatchSpan]>) -> Self {
        self.match_spans = match_spans;
        self
    }

    /// 设置颜色装饰 span（见 [`TextColorSwatchSpan`]）。
    pub fn color_swatches(mut self, swatches: Arc<[TextColorSwatchSpan]>) -> Self {
        self.color_swatches = swatches;
        self
    }

    /// 启用行号栏（行号绘制在节点左内边距区域）。
    pub fn line_numbers(mut self, line_numbers: bool) -> Self {
        self.line_numbers = line_numbers;
        self
    }

    /// 设置代码折叠区间（见 [`TextCodeFold`]）。宿主在文本变化后重新喂；
    /// 折叠态由组件内部维护。
    pub fn code_folds(mut self, folds: Arc<[TextCodeFold]>) -> Self {
        self.code_folds = folds;
        self
    }

    /// 设置行内提示（见 [`TextInlay`]）。快照失配/偏好关时宿主撤空列表。
    pub fn inlays(mut self, inlays: Arc<[TextInlay]>) -> Self {
        self.inlays = inlays;
        self
    }

    /// 设置 git gutter 标记（见 [`TextGitMark`]）。宿主在 git 状态与文本
    /// 变化后重新喂；行号无效或被折叠隐藏的标记静默跳过。
    pub fn git_gutter(mut self, marks: Arc<[TextGitMark]>) -> Self {
        self.git_gutter = marks;
        self
    }

    /// 设置补全候选（见 [`TextCompletion`]）。过滤由宿主负责：按当前词
    /// 前缀过滤后喂入，空列表表示关闭弹层。
    pub fn completions(mut self, items: Arc<[TextCompletion]>) -> Self {
        self.completions = items;
        self
    }

    /// 设置 hover 文档浮窗（见 [`TextHover`]）。`None` 表示隐藏。
    pub fn hover(mut self, hover: Option<TextHover>) -> Self {
        self.hover = hover;
        self
    }

    /// 开启光标处单词/选中文本的出现高亮（内部派生：聚焦时从主光标或
    /// 非空单行选区派生查询，全文档大小写敏感扫描，全词边界按词模式
    /// 生效；主光标所在出现不画）。默认关闭。
    pub fn occurrence_highlight(mut self, enabled: bool) -> Self {
        self.occurrence_highlight = enabled;
        self
    }

    /// 开启相对行号：光标行显示绝对行号，其余行显示与光标所在显示行的
    /// 距离（Zed 惯例，多光标按主光标）。仅在 [`Self::line_numbers`]
    /// 开启时生效。默认关闭。
    pub fn relative_line_numbers(mut self, enabled: bool) -> Self {
        self.relative_line_numbers = enabled;
        self
    }

    /// 开启空白字符显示：空格画中点、Tab 画箭头；行首缩进与行尾空白
    /// 一并可见。默认关闭。
    pub fn show_whitespace(mut self, enabled: bool) -> Self {
        self.show_whitespace = enabled;
        self
    }

    /// 设置 wrap guide 列参考线：在每个给定字符列（1 起）的 x 位置画
    /// 全高竖线。列宽按 `'0'` 字形宽度估算（等宽字体假设）；文档最宽行
    /// 不足该列时不画。默认为空。
    pub fn wrap_guides(mut self, columns: Arc<[usize]>) -> Self {
        self.wrap_guides = columns;
        self
    }

    /// 开启代码编辑器 minimap：内容区右缘 64px 覆盖竖条 + 视口指示器，
    /// 点击/拖动导航视口（点击行居中）。折叠只影响主视图（minimap 显示
    /// 全部逻辑行）；极长行会被条遮挡。默认关闭。
    pub fn minimap(mut self, enabled: bool) -> Self {
        self.minimap = enabled;
        self
    }

    /// 开启 sticky scroll：滚动视口顶部落在 `code_folds` 喂入的区间内部
    /// 时，在内容区顶部钉住显示该区间头行（嵌套区间钉最内层）。需要同时
    /// 喂入 `code_folds`。默认关闭。
    pub fn sticky_scroll(mut self, enabled: bool) -> Self {
        self.sticky_scroll = enabled;
        self
    }

    /// Color committed text with the registered `"highlight"` presenter.
    pub fn highlight(mut self, language: impl Into<Arc<str>>) -> Self {
        self.highlight = Some(HighlightRequest::highlight(language));
        self
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
        let normalized = crate::text_editing::normalize_newlines(text);
        self.state.replace_selection(&normalized)
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
            steppers: false,
            diagnostics: Arc::clone(&self.diagnostics),
            matches: Arc::clone(&self.match_spans),
            color_swatches: Arc::clone(&self.color_swatches),
            line_numbers: self.line_numbers,
            indent_guides: self
                .code_editing
                .as_ref()
                .map(|code| Arc::clone(&code.indent_unit)),
            folds: Arc::clone(&self.code_folds),
            git_marks: Arc::clone(&self.git_gutter),
            editor_options: TextEditorRenderOptions {
                occurrence_highlight: self.occurrence_highlight,
                relative_line_numbers: self.relative_line_numbers,
                show_whitespace: self.show_whitespace,
                wrap_guides: Arc::clone(&self.wrap_guides),
                minimap: self.minimap,
                sticky_scroll: self.sticky_scroll,
                bracket_pair_colors: true,
            },
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        if world.scroll_offset(id) != Some(self.scroll_offset) {
            mutations.set_scroll_offset(id, self.scroll_offset);
        }
        // 补全候选喂入：列表未变（指针或内容相等）时不下发变更，会话的
        // 键盘选中/滚动原样保留；空列表由世界侧移除会话（弹层关闭）。
        {
            let fed_unchanged = world
                .text_completion_items(id)
                .is_some_and(|fed| Arc::ptr_eq(fed, &self.completions) || *fed == self.completions);
            if !fed_unchanged {
                mutations.set_text_input_completions(id, Arc::clone(&self.completions));
            }
        }
        // hover 浮窗喂入：内容未变时不下发变更。
        {
            let fed_unchanged = match (world.text_hover_doc(id), self.hover.as_ref()) {
                (Some(fed), Some(hover)) => fed == hover,
                (None, None) => true,
                _ => false,
            };
            if !fed_unchanged {
                mutations.set_text_input_hover(id, self.hover.clone());
            }
        }
        // 行内提示喂入：列表未变（指针或内容相等）且空列表无条目时不下发
        // 变更；世界侧做锚点/文本校验与排序去重。
        {
            let fed_unchanged = match world.text_inlay_items(id) {
                Some(fed) => Arc::ptr_eq(fed, &self.inlays) || *fed == self.inlays,
                None => self.inlays.is_empty(),
            };
            if !fed_unchanged {
                mutations.set_text_input_inlays(id, Arc::clone(&self.inlays));
            }
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
                highlight: self.highlight.as_ref(),
                numeric: None,
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

/// Highlighted multiline editor on the same retained [`TextInputState`] as [`TextArea`].
///
/// Official syntax color is the registered `"highlight"` presenter on committed
/// text. IME preedit stays solid. Missing presenters leave the field uncolored.
#[derive(Debug, Clone, PartialEq)]
pub struct HostedTextarea {
    inner: TextArea,
}

impl HostedTextarea {
    pub fn new(value: impl Into<String>, language: impl Into<Arc<str>>) -> Self {
        Self {
            inner: TextArea::new(value).highlight(language),
        }
    }

    pub fn language(&self) -> Option<&str> {
        self.inner
            .highlight
            .as_ref()
            .map(|request| request.language.as_ref())
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.inner = self.inner.placeholder(placeholder);
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.inner = self.inner.label(label);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.inner = self.inner.disabled(disabled);
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.inner = self.inner.invalid(invalid);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    pub fn scroll_offset(mut self, offset: ScrollOffset) -> Self {
        self.inner = self.inner.scroll_offset(offset);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn into_text_area(self) -> TextArea {
        self.inner
    }
}

impl std::ops::Deref for HostedTextarea {
    type Target = TextArea;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for HostedTextarea {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl ComponentView for HostedTextarea {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "hosted-textarea".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.inner.project(id, world, mutations);
    }
}

#[cfg(test)]
mod hosted_textarea_tests {
    use super::*;
    use crate::{DocumentId, MutationQueue, UiWorld};

    #[test]
    fn hosted_textarea_always_requests_the_highlight_presenter() {
        let editor = HostedTextarea::new("fn main() {}", "rs")
            .placeholder("fn main")
            .disabled(false);
        assert_eq!(editor.language(), Some("rs"));
        assert_eq!(
            editor
                .highlight
                .as_ref()
                .map(|request| request.presenter.as_ref()),
            Some(crate::HIGHLIGHT_PRESENTER)
        );

        let mut world = UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, DocumentId::new(1).unwrap(), editor.node_kind());
        world.commit(queue).unwrap();
        let mut queue = MutationQueue::new();
        editor.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        assert_eq!(
            world
                .highlight_request(id)
                .map(|request| (request.presenter.as_ref(), request.language.as_ref())),
            Some((crate::HIGHLIGHT_PRESENTER, "rs"))
        );
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
        // Whether a host takes the pointer follows the active overlay's kind,
        // which activation resolves and writes. Projection would only see the
        // state from before that commit, so it carries the value forward and
        // starts out transparent.
        let interaction = world.interaction(id).unwrap_or(InteractionState {
            pointer_events: false,
            focusable: false,
        });
        project_common(
            id,
            world,
            mutations,
            &self.style,
            interaction,
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
            None,
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
        let mut style = tooltip_surface_style(config.max_width.max(0.0));
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

fn tooltip_surface_style(max_width: f32) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            position: nana_ui_core::PositionSpec::Fixed,
            max_width: Some(nana_ui_core::LengthSpec::Px(max_width)),
            padding_left: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::TooltipConfig::PADDING_X,
            )),
            padding_right: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::TooltipConfig::PADDING_X,
            )),
            padding_top: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::TooltipConfig::PADDING_Y,
            )),
            padding_bottom: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::TooltipConfig::PADDING_Y,
            )),
            border_width: Some(1.0),
            border_radius: Some(nana_ui_core::TooltipConfig::RADIUS),
            font_size: Some(nana_ui_core::TooltipConfig::FONT_SIZE),
            z_index: Some(1_000),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(nana_ui_core::SemanticColorRole::Surface),
        border: Some(nana_ui_core::SemanticColorRole::BorderSoft),
        foreground: Some(nana_ui_core::SemanticColorRole::Text),
        ..NodeStyle::default()
    }
}

fn checkbox_style() -> NodeStyle {
    checkbox_style_for(nana_ui_core::ControlSize::Medium)
}

fn checkbox_style_for(size: nana_ui_core::ControlSize) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            min_height: Some(nana_ui_core::LengthSpec::Px(size.height())),
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
            indeterminate: self.indeterminate,
            size: self.size,
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
                mixed: self.indeterminate,
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Hairline rule between sibling content.
///
/// A divider carries no label and no interaction: it is one themed line that
/// stretches along the cross axis of its parent. Grouping headings stay
/// ordinary `Text` siblings, so a section title never has to be a divider prop.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub orientation: crate::SelectionOrientation,
    /// Line thickness in logical pixels.
    pub thickness: f32,
    /// Inset applied at both ends along the divider's own direction.
    pub inset: f32,
    pub style: NodeStyle,
}

impl Divider {
    /// Horizontal rule: stretches across its parent's width.
    pub fn horizontal() -> Self {
        Self::new(crate::SelectionOrientation::Horizontal)
    }

    /// Vertical rule: stretches down its parent's height.
    pub fn vertical() -> Self {
        Self::new(crate::SelectionOrientation::Vertical)
    }

    fn new(orientation: crate::SelectionOrientation) -> Self {
        let mut divider = Self {
            orientation,
            thickness: 1.0,
            inset: 0.0,
            style: NodeStyle::default(),
        };
        divider.style = divider.resolved_style();
        divider
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self.style = self.resolved_style();
        self
    }

    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self.style = self.resolved_style();
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn resolved_style(&self) -> NodeStyle {
        let thickness = if self.thickness.is_finite() && self.thickness > 0.0 {
            self.thickness
        } else {
            1.0
        };
        let inset = if self.inset.is_finite() && self.inset > 0.0 {
            self.inset
        } else {
            0.0
        };
        let mut style = NodeStyle {
            background: Some(nana_ui_core::SemanticColorRole::BorderSoft),
            ..NodeStyle::default()
        };
        let layout = Arc::new(match self.orientation {
            crate::SelectionOrientation::Horizontal => nana_ui_core::LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Fill),
                height: Some(nana_ui_core::LengthSpec::Px(thickness)),
                min_height: Some(nana_ui_core::LengthSpec::Px(thickness)),
                flex_shrink: Some(0.0),
                margin_left: Some(nana_ui_core::LengthSpec::Px(inset)),
                margin_right: Some(nana_ui_core::LengthSpec::Px(inset)),
                ..nana_ui_core::LayoutStyle::default()
            },
            crate::SelectionOrientation::Vertical => nana_ui_core::LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(thickness)),
                min_width: Some(nana_ui_core::LengthSpec::Px(thickness)),
                height: Some(nana_ui_core::LengthSpec::Fill),
                flex_shrink: Some(0.0),
                margin_top: Some(nana_ui_core::LengthSpec::Px(inset)),
                margin_bottom: Some(nana_ui_core::LengthSpec::Px(inset)),
                ..nana_ui_core::LayoutStyle::default()
            },
        });
        style.layout = layout;
        style
    }
}

impl Default for Divider {
    fn default() -> Self {
        Self::horizontal()
    }
}

impl ComponentView for Divider {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "divider".into(),
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
                role: AccessibilityRole::Separator,
                orientation: Some(self.orientation),
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
        let thumb_progress = match world.standard_visual(id) {
            Some(StandardVisual::Switch {
                checked,
                thumb_progress,
                ..
            }) if checked != self.checked
                || crate::component_animation_id(crate::component_animation_kinds::SWITCH, id)
                    .is_some_and(|animation| world.animation_is_active(animation)) =>
            {
                thumb_progress
            }
            _ => f32::from(self.checked),
        };
        let visual = StandardVisual::Switch {
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
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        let layout = Arc::make_mut(&mut effective_style.layout);
        if layout.width.is_none() {
            layout.width = Some(nana_ui_core::LengthSpec::Fill);
        }
        if layout.height.is_none() {
            layout.min_height = Some(nana_ui_core::LengthSpec::Px(if self.hint.is_some() {
                42.0
            } else {
                self.size.height()
            }));
        }
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
            style: range_field_style(),
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
        if layout.height.is_none() {
            layout.min_height = Some(nana_ui_core::LengthSpec::Px(self.size.height()));
        }
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

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollView {
    pub axes: ScrollAxes,
    pub label: Option<Arc<str>>,
    pub scrollbars: nana_ui_core::ScrollbarVisibility,
    /// Pointer is inside the container, so auto-hiding bars show.
    pub hovered: bool,
    pub dragging: Option<ScrollbarDragState>,
    pub style: NodeStyle,
}

/// Transient thumb drag. The scroll offset stays authoritative in the world;
/// only the grab anchor lives here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarDragState {
    pub pointer_id: u64,
    pub axis: nana_ui_core::ScrollbarAxis,
    /// Distance from the thumb's leading edge to the grab point.
    pub grab_offset: f32,
    pub initial_offset: ScrollOffset,
}

impl ScrollView {
    pub fn new(axes: ScrollAxes) -> Self {
        Self {
            axes,
            label: None,
            scrollbars: nana_ui_core::ScrollbarVisibility::default(),
            hovered: false,
            dragging: None,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Choose overlay auto-hide, resident, or no bars at all.
    pub fn scrollbars(mut self, visibility: nana_ui_core::ScrollbarVisibility) -> Self {
        self.scrollbars = visibility;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Bars are drawn when resident, or while hovered or dragged.
    pub fn scrollbars_revealed(&self) -> bool {
        match self.scrollbars {
            nana_ui_core::ScrollbarVisibility::Always => true,
            nana_ui_core::ScrollbarVisibility::AutoHide => self.hovered || self.dragging.is_some(),
            nana_ui_core::ScrollbarVisibility::Hidden => false,
        }
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

    /// Project scrollport style, interaction, and accessibility without the
    /// scrollbar visual.
    ///
    /// Containers that lend their own node to a scrollport — [`SidebarFrame`]'s
    /// body, for one — use this so they do not stamp a scrollbar over whatever
    /// visual the node's real component owns.
    ///
    /// [`SidebarFrame`]: crate::SidebarFrame
    pub fn project_scrollport(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
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

impl ComponentView for ScrollView {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "scroll".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = crate::StandardVisual::Scrollbar {
            axes: self.axes,
            visibility: self.scrollbars,
            revealed: self.scrollbars_revealed(),
            dragging: self.dragging.map(|drag| drag.axis),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        self.project_scrollport(id, world, mutations);
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

/// 通用布局容器：预设覆盖常用 flex；完整字段走 [`nana_ui_core::LayoutStyle`]。
///
/// 预设构造器覆盖常用排版，语义与布局引擎默认值对齐过的产物一致：
/// - [`Stack::row`]：水平排列，宽度随内容（工具条、按钮组）。
/// - [`Stack::fill_row`]：水平排列并占满父容器剩余宽度。
/// - [`Stack::column`]：竖直排列，高度随内容（页面纵向结构）。
/// - [`Stack::fill_column`]：竖直排列并占满父容器剩余高度（主内容区）。
/// - [`Stack::bar`]：水平排列占满整行但不伸展（顶栏、底栏）。
///
/// Vue / CSS 路径用 [`Stack::from_layout`] 写入已解析的 [`nana_ui_core::LayoutStyle`]。
/// grid / position / overflow / paint 用 [`Self::with_layout`]，不要另造布局控件。
///
/// Rust 预设容器默认不参与命中测试；[`Self::from_layout`] 与 Vue 布局盒默认可点。
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    style: NodeStyle,
    hittable: bool,
}

impl Stack {
    fn base(
        direction: nana_ui_core::FlexDirection,
        gap: f32,
        align: nana_ui_core::AlignSpec,
        justify: nana_ui_core::JustifySpec,
    ) -> Self {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(direction);
        layout.gap = Some(nana_ui_core::LengthSpec::Px(gap.max(0.0)));
        layout.align_items = align;
        layout.justify_content = justify;
        Self {
            style,
            hittable: false,
        }
    }

    /// 承接已解析的 [`nana_ui_core::LayoutStyle`]（Vue CSS / 直写字段）。
    ///
    /// 不套用 [`Self::row`] 一类预设默认。默认可点，与 Vue 布局盒一致。
    pub fn from_layout(layout: impl Into<Arc<nana_ui_core::LayoutStyle>>) -> Self {
        let mut style = NodeStyle::default();
        style.layout = layout.into();
        Self {
            style,
            hittable: true,
        }
    }

    /// 就地改 [`nana_ui_core::LayoutStyle`] 全字段。
    pub fn with_layout(mut self, f: impl FnOnce(&mut nana_ui_core::LayoutStyle)) -> Self {
        f(Arc::make_mut(&mut self.style.layout));
        self
    }

    /// 水平排列，宽度随内容收缩，子项垂直居中。
    pub fn row(gap: f32) -> Self {
        Self::base(
            nana_ui_core::FlexDirection::Row,
            gap,
            nana_ui_core::AlignSpec::Center,
            nana_ui_core::JustifySpec::Start,
        )
        .width(nana_ui_core::LengthSpec::Shrink)
    }

    /// 水平排列并占满父容器剩余宽度，子项可收缩。
    pub fn fill_row(gap: f32) -> Self {
        Self::base(
            nana_ui_core::FlexDirection::Row,
            gap,
            nana_ui_core::AlignSpec::Center,
            nana_ui_core::JustifySpec::Start,
        )
        .width(nana_ui_core::LengthSpec::Fill)
        .grow(1.0)
        .shrink(1.0)
    }

    /// 竖直排列，高度随内容，宽度占满父容器。
    pub fn column(gap: f32) -> Self {
        Self::base(
            nana_ui_core::FlexDirection::Column,
            gap,
            nana_ui_core::AlignSpec::Stretch,
            nana_ui_core::JustifySpec::Start,
        )
        .width(nana_ui_core::LengthSpec::Fill)
        .height(nana_ui_core::LengthSpec::Shrink)
        .min_width(nana_ui_core::LengthSpec::Px(0.0))
        .grow(0.0)
        .shrink(0.0)
    }

    /// 竖直排列并占满父容器剩余高度，用于主内容区。
    pub fn fill_column(gap: f32) -> Self {
        Self::base(
            nana_ui_core::FlexDirection::Column,
            gap,
            nana_ui_core::AlignSpec::Stretch,
            nana_ui_core::JustifySpec::Start,
        )
        .width(nana_ui_core::LengthSpec::Fill)
        .height(nana_ui_core::LengthSpec::Fill)
        .min_width(nana_ui_core::LengthSpec::Px(0.0))
        .min_height(nana_ui_core::LengthSpec::Px(0.0))
        .grow(1.0)
        .shrink(1.0)
    }

    /// 水平排列占满整行但不伸展（顶栏、底栏）。
    pub fn bar(gap: f32) -> Self {
        Self::base(
            nana_ui_core::FlexDirection::Row,
            gap,
            nana_ui_core::AlignSpec::Center,
            nana_ui_core::JustifySpec::Start,
        )
        .width(nana_ui_core::LengthSpec::Fill)
        .grow(0.0)
        .shrink(0.0)
    }

    pub fn gap(mut self, gap: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).gap =
            Some(nana_ui_core::LengthSpec::Px(gap.max(0.0)));
        self
    }

    pub fn align(mut self, align: nana_ui_core::AlignSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).align_items = align;
        self
    }

    pub fn justify(mut self, justify: nana_ui_core::JustifySpec) -> Self {
        Arc::make_mut(&mut self.style.layout).justify_content = justify;
        self
    }

    pub fn width(mut self, width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).width = Some(width);
        self
    }

    pub fn height(mut self, height: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).height = Some(height);
        self
    }

    pub fn min_width(mut self, min_width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).min_width = Some(min_width);
        self
    }

    pub fn min_height(mut self, min_height: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).min_height = Some(min_height);
        self
    }

    pub fn max_width(mut self, max_width: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).max_width =
            Some(nana_ui_core::LengthSpec::Px(max_width.max(0.0)));
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).flex_grow = Some(grow);
        self
    }

    pub fn shrink(mut self, shrink: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).flex_shrink = Some(shrink);
        self
    }

    /// 四边统一内边距，覆盖先前物理及逻辑边声明。
    pub fn padding(self, padding: f32) -> Self {
        self.padding_xy(padding, padding)
    }

    /// 水平与垂直内边距，后调用者覆盖四边。
    pub fn padding_xy(mut self, x: f32, y: f32) -> Self {
        replace_padding_xy(Arc::make_mut(&mut self.style.layout), x, y);
        self
    }

    /// 取出容器样式，可复用到其他控件的 `.style(...)` 上。
    pub fn node_style(&self) -> NodeStyle {
        self.style.clone()
    }

    /// 语义背景色。
    pub fn surface(mut self, role: nana_ui_core::SemanticColorRole) -> Self {
        self.style.background = Some(role);
        self
    }

    /// 一次性写全边框：语义色角色与宽度必须同时给出，缺一边框不会绘制。
    pub fn outline(mut self, role: nana_ui_core::SemanticColorRole, width: f32) -> Self {
        self.style = self.style.outline(role, width);
        self
    }

    /// 圆角半径（物理 px）。
    pub fn radius(mut self, radius: f32) -> Self {
        self.style = self.style.radius(radius);
        self
    }

    /// Vue / CSS 布局盒：参与命中。Rust 预设默认可点关闭。
    pub fn hittable(mut self) -> Self {
        self.hittable = true;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Stack {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "stack".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let hittable = self.hittable
            && !self.style.layout.hidden
            && self.style.layout.pointer_events != Some(nana_ui_core::PointerEventsSpec::None);
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: hittable,
                focusable: false,
            },
            AccessibilityState::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;

    fn mount(component: &ListItem) -> (UiWorld, StableNodeId) {
        let mut world = UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, document, component.node_kind());
        component.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        (world, id)
    }

    #[test]
    fn detail_travels_on_the_visual_not_the_node_text() {
        let item = ListItem::new("模型").detail("3 动作");
        let (world, id) = mount(&item);
        // 行文本只承载 label，detail 由几何端独立成区。
        assert_eq!(world.text(id), Some("模型"));
        assert_eq!(
            world.standard_visual(id),
            Some(StandardVisual::ListItem {
                leading: None,
                content: None,
                trailing: None,
                detail: Some(Arc::from("3 动作")),
            })
        );
        // 节点文本不再拼接 detail，也就不再依赖 muted span。
        assert!(world.extract_nodes(&[id])[0].text_spans.is_empty());
    }

    #[test]
    fn plain_rows_stay_solid() {
        let (world, id) = mount(&ListItem::new("纯标签"));
        assert_eq!(world.text(id), Some("纯标签"));
        assert_eq!(
            world.standard_visual(id),
            Some(StandardVisual::ListItem {
                leading: None,
                content: None,
                trailing: None,
                detail: None,
            })
        );
        assert!(world.extract_nodes(&[id])[0].text_spans.is_empty());
    }

    #[test]
    fn content_slot_row_keeps_label_out_of_node_text() {
        let item = ListItem::new("模型").detail("3 动作").slots(ListItemSlots {
            content: Some(StableNodeId::new(2).unwrap()),
            ..ListItemSlots::default()
        });
        let (world, id) = mount(&item);
        assert_eq!(world.text(id), Some(""));
        assert_eq!(
            world.standard_visual(id),
            Some(StandardVisual::ListItem {
                leading: None,
                content: Some(StableNodeId::new(2).unwrap()),
                trailing: None,
                detail: None,
            })
        );
    }

    #[test]
    fn trailing_slot_aligns_row_children_to_the_end() {
        let trailing = Some(StableNodeId::new(2).unwrap());
        let leading = Some(StableNodeId::new(3).unwrap());
        let justify = |item: ListItem| item.effective_style().layout.justify_content;
        assert_eq!(
            justify(ListItem::new("行").slots(ListItemSlots {
                leading,
                content: None,
                trailing,
            })),
            nana_ui_core::JustifySpec::SpaceBetween
        );
        assert_eq!(
            justify(ListItem::new("行").slots(ListItemSlots {
                leading: None,
                content: None,
                trailing,
            })),
            nana_ui_core::JustifySpec::End
        );
        assert_eq!(
            justify(ListItem::new("行")),
            nana_ui_core::JustifySpec::Start
        );
    }

    #[test]
    fn pill_bleed_extends_the_pill_and_keeps_text_inset() {
        // 样式合同：margin 取水平 padding 负值，文本内缩不变；默认不外扩。
        let px = nana_ui_core::LengthSpec::Px;
        let inset = nana_ui_core::UI_METRICS.list_item_padding_x;
        let bled = ListItem::new("行")
            .pill_bleed(true)
            .effective_style()
            .layout;
        assert_eq!(bled.padding_left, Some(px(inset)));
        assert_eq!(bled.margin_left, Some(px(-inset)));
        assert_eq!(bled.margin_right, bled.margin_left);
        assert_eq!(
            ListItem::new("行").effective_style().layout.margin_left,
            None
        );

        // 布局行为：行盒越出列表容器一个内边距，pill 占满可用宽度。
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut list_style = Stack::column(2.0).node_style();
        std::sync::Arc::make_mut(&mut list_style.layout).width = Some(px(240.0));
        let list = context
            .create_component(document, List::new().style(list_style))
            .unwrap();
        let row = context
            .create_component(document, ListItem::new("行").pill_bleed(true))
            .unwrap();
        context.append_child(list, row).unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(240.0, 120.0))
            .unwrap();
        let list_box = context.world().layout_box(list.stable_id()).unwrap();
        let row_box = context.world().layout_box(row.stable_id()).unwrap();
        assert!((row_box.x - (list_box.x - inset)).abs() < 0.5);
        assert!((row_box.width - (list_box.width + inset * 2.0)).abs() < 0.5);
    }
}

#[cfg(test)]
mod spacing_tests {
    use super::*;
    use nana_ui_core::{DirSpec, LengthSpec, PaddingSpec};

    #[test]
    fn spacing_stack_last_padding_setter_wins_after_direction_change() {
        let stack = Stack::column(0.0)
            .padding_xy(12.0, 10.0)
            .with_layout(|layout| {
                layout.logical_padding.set_start(Some(LengthSpec::Px(30.0)));
                layout.padding_logical.block_end = Some(LengthSpec::Px(40.0));
            })
            .padding(0.0);
        let mut layout = (*stack.node_style().layout).clone();
        layout.dir = Some(DirSpec::Rtl);
        layout.bake_logical_edges();
        assert_eq!(layout.resolved_padding(), PaddingSpec::uniform(0.0));
        let layout = Stack::column(0.0)
            .padding(9.0)
            .padding_xy(2.0, 3.0)
            .node_style()
            .layout;
        assert_eq!(
            layout.resolved_padding(),
            PaddingSpec {
                left: 2.0,
                right: 2.0,
                top: 3.0,
                bottom: 3.0
            }
        );
    }

    #[test]
    fn spacing_card_defaults_restore_after_partial_style_removal() {
        let mut context = crate::AppContext::new();
        let document = crate::DocumentId::new(1).unwrap();
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).padding_top = Some(LengthSpec::Px(0.0));
        let card = context
            .create_component(document, Card::new().style(style))
            .unwrap();
        let padding = context
            .world()
            .node_style(card.stable_id())
            .unwrap()
            .layout
            .resolved_padding();
        assert_eq!(
            padding,
            PaddingSpec {
                top: 0.0,
                right: 16.0,
                bottom: 14.0,
                left: 16.0
            }
        );
        context
            .update_component(card, |card, _| {
                card.style = NodeStyle::default();
            })
            .unwrap();
        assert_eq!(
            context
                .world()
                .node_style(card.stable_id())
                .unwrap()
                .layout
                .resolved_padding(),
            PaddingSpec {
                top: 14.0,
                right: 16.0,
                bottom: 14.0,
                left: 16.0
            }
        );
        assert!(
            context
                .read(card, |card| card.style.layout.padding_left.is_none())
                .unwrap()
        );
    }
}

/// Imperative builders replace all edges; raw LayoutStyle keeps CSS precedence.
fn replace_padding_xy(layout: &mut nana_ui_core::LayoutStyle, x: f32, y: f32) {
    use nana_ui_core::LengthSpec;
    layout.padding = None;
    layout.logical_padding = Default::default();
    layout.padding_logical = Default::default();
    layout.padding_left = Some(LengthSpec::Px(x.max(0.0)));
    layout.padding_right = layout.padding_left;
    layout.padding_top = Some(LengthSpec::Px(y.max(0.0)));
    layout.padding_bottom = layout.padding_top;
}
