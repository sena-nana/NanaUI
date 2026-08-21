use std::sync::Arc;

use nana_ui_core::{ControlSize, LengthSpec, SemanticColorRole, SemanticPalette, UI_METRICS};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentElevation, ComponentGeometry,
    ComponentTextRegion, ComputedStyle, InteractionState, InteractionStyle, LayoutBox,
    MutationQueue, NodeKind, NodeStyle, SelectOptionData, SemanticPaint, StableNodeId,
    StandardVisual, TextContent, TextVerticalAlignment, UiWorld,
};

const HANDLE_WIDTH: f32 = 16.0;
const MENU_GAP: f32 = 0.0;
const MENU_PAD: f32 = 4.0;
const MENU_ITEM_GAP: f32 = 1.0;

/// Option identity stays application-owned. Disabled options remain visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub value: Arc<str>,
    pub label: Arc<str>,
    pub disabled: bool,
}

impl SelectOption {
    pub fn new(value: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Committed selection from pointer, keyboard or accessibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectChanged {
    pub value: Arc<str>,
}

/// Single-value field. Disabled options stay visible in the opened menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub value: Option<Arc<str>>,
    pub options: Vec<SelectOption>,
    pub placeholder: Option<Arc<str>>,
    pub size: ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub invalid: bool,
    pub opened: bool,
    pub highlighted: Option<usize>,
    pub style: NodeStyle,
}

impl Select {
    pub fn new(value: Option<impl Into<Arc<str>>>) -> Self {
        Self {
            value: value.map(Into::into),
            options: Vec::new(),
            placeholder: None,
            size: ControlSize::Medium,
            disabled: false,
            loading: false,
            invalid: false,
            opened: false,
            highlighted: None,
            style: field_style_for_size(ControlSize::Medium),
        }
    }

    pub fn options(mut self, options: impl IntoIterator<Item = SelectOption>) -> Self {
        self.options = options.into_iter().collect();
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        let placeholder = placeholder.into();
        self.placeholder = (!placeholder.is_empty()).then_some(placeholder);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self.style = field_style_for_size(size);
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

    pub fn opened(mut self, opened: bool) -> Self {
        self.opened = opened;
        if opened {
            self.highlighted = self.selected_index().or_else(|| self.first_enabled());
        } else {
            self.highlighted = None;
        }
        self
    }

    pub fn inactive(&self) -> bool {
        self.disabled || self.loading
    }

    pub fn selected_index(&self) -> Option<usize> {
        let value = self.value.as_ref()?;
        self.options
            .iter()
            .position(|option| &option.value == value)
    }

    pub fn display_label(&self) -> (Arc<str>, bool) {
        if let Some(index) = self.selected_index() {
            return (Arc::clone(&self.options[index].label), false);
        }
        (
            self.placeholder.clone().unwrap_or_else(|| Arc::from("")),
            true,
        )
    }

    pub fn toggle_open(&mut self) -> bool {
        if self.inactive() {
            return false;
        }
        self.opened = !self.opened;
        if self.opened {
            self.highlighted = self.selected_index().or_else(|| self.first_enabled());
        } else {
            self.highlighted = None;
        }
        true
    }

    pub fn close(&mut self) {
        self.opened = false;
        self.highlighted = None;
    }

    pub fn highlight_delta(&mut self, delta: i32) -> bool {
        if self.inactive() || !self.opened || self.options.is_empty() {
            return false;
        }
        let enabled = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| !option.disabled)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return false;
        }
        let current = self
            .highlighted
            .and_then(|index| enabled.iter().position(|candidate| *candidate == index));
        let next = match current {
            Some(index) => {
                let len = enabled.len() as i32;
                enabled[((index as i32 + delta).rem_euclid(len)) as usize]
            }
            None if delta >= 0 => enabled[0],
            None => *enabled.last().expect("enabled is non-empty"),
        };
        if self.highlighted == Some(next) {
            return false;
        }
        self.highlighted = Some(next);
        true
    }

    pub fn commit_highlighted(&mut self) -> Option<SelectChanged> {
        let index = self.highlighted?;
        self.select_index(index)
    }

    pub fn select_index(&mut self, index: usize) -> Option<SelectChanged> {
        let option = self.options.get(index)?;
        if option.disabled || self.inactive() {
            return None;
        }
        let changed = self.value.as_deref() != Some(&*option.value);
        self.value = Some(Arc::clone(&option.value));
        self.close();
        changed.then(|| SelectChanged {
            value: Arc::clone(self.value.as_ref().expect("selected value")),
        })
    }

    fn first_enabled(&self) -> Option<usize> {
        self.options.iter().position(|option| !option.disabled)
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(if self.inactive() || self.display_label().1 {
            SemanticColorRole::Faint
        } else {
            SemanticColorRole::Text
        });
        style.background = Some(if self.inactive() {
            SemanticColorRole::Subtle
        } else {
            SemanticColorRole::Background
        });
        style.border = Some(if self.invalid {
            SemanticColorRole::Danger
        } else if self.opened {
            SemanticColorRole::BorderSoft
        } else {
            SemanticColorRole::Border
        });
        style.interaction.hovered.border = Some(if self.invalid {
            SemanticColorRole::Danger
        } else {
            SemanticColorRole::BorderStrong
        });
        style.interaction.focused.border = Some(if self.invalid {
            SemanticColorRole::Danger
        } else {
            SemanticColorRole::BorderStrong
        });
        style.interaction.disabled = SemanticPaint {
            foreground: Some(SemanticColorRole::Faint),
            background: Some(SemanticColorRole::Subtle),
            border: Some(SemanticColorRole::Border),
        };
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(self.size.height()));
        layout.border_width = Some(if self.invalid && self.opened {
            2.0
        } else {
            1.0
        });
        layout.border_radius = Some(UI_METRICS.radius_sm);
        layout.padding_left = Some(LengthSpec::Px(self.size.padding_x()));
        layout.padding_right = Some(LengthSpec::Px(self.size.padding_x()));
        layout.white_space_nowrap = true;
        style.text_vertical_alignment = TextVerticalAlignment::Center;
        style
    }
}

impl crate::ComponentView for Select {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "select".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let (label, placeholder) = self.display_label();
        let visual = StandardVisual::Select {
            label: Arc::clone(&label),
            placeholder,
            size: self.size,
            opened: self.opened && !self.inactive(),
            invalid: self.invalid,
            loading: self.loading,
            options: self
                .options
                .iter()
                .map(|option| SelectOptionData {
                    label: Arc::clone(&option.label),
                    hint: None,
                    disabled: option.disabled,
                    checked: false,
                    icon: None,
                })
                .collect(),
            highlighted: self.highlighted,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        if world.text(id) != Some(label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: label.to_string(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.inactive(),
                focusable: !self.inactive(),
            },
            AccessibilityState {
                role: AccessibilityRole::ComboBox,
                label: Some(label),
                value: self.value.clone(),
                disabled: self.inactive(),
                busy: self.loading,
                invalid: self.invalid,
                selected: Some(self.opened && !self.inactive()),
                ..AccessibilityState::default()
            },
        );
    }
}

pub(crate) fn select_geometry(
    bounds: LayoutBox,
    label: &Arc<str>,
    placeholder: bool,
    size: ControlSize,
    opened: bool,
    options: &[SelectOptionData],
    highlighted: Option<usize>,
    style: &ComputedStyle,
    source: &NodeStyle,
    palette: &SemanticPalette,
) -> ComponentGeometry {
    let padding = source.layout.resolved_padding_against(Some(bounds.width));
    let border = source.layout.resolved_border_width();
    let content = LayoutBox {
        x: bounds.x + border + padding.left,
        y: bounds.y + border + padding.top,
        width: (bounds.width - border * 2.0 - padding.left - padding.right).max(0.0),
        height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
    };
    let handle_width = HANDLE_WIDTH.min(content.width);
    let label_width = (content.width - handle_width).max(0.0);
    let text_color = if placeholder || matches!(style.foreground, SemanticColorRole::Faint) {
        palette.faint.as_rgba_array()
    } else {
        style.color.unwrap_or_else(|| palette.text.as_rgba_array())
    };
    let handle_color = palette.muted.as_rgba_array();
    let menu = opened.then(|| select_menu_geometry(bounds, size, options, highlighted, palette));
    ComponentGeometry::Select {
        label: ComponentTextRegion {
            bounds: LayoutBox {
                x: content.x,
                y: content.y,
                width: label_width,
                height: content.height,
            },
            content: Arc::clone(label),
            color: Some(text_color),
            font_size: size.text_size(),
            font_weight: None,
        },
        handle: LayoutBox {
            x: content.x + label_width,
            y: content.y,
            width: handle_width,
            height: content.height,
        },
        handle_color,
        background: style.background,
        border: style.border_color,
        border_width: border,
        menu,
    }
}

fn select_menu_geometry(
    field: LayoutBox,
    size: ControlSize,
    options: &[SelectOptionData],
    highlighted: Option<usize>,
    palette: &SemanticPalette,
) -> crate::SelectMenuGeometry {
    let item_height = size.height();
    let count = options.len().max(1) as f32;
    let height = MENU_PAD * 2.0 + count * item_height + (count - 1.0).max(0.0) * MENU_ITEM_GAP;
    let is_light = palette.background.as_rgba_array()[0] > 0.5;
    let surface = LayoutBox {
        x: field.x,
        y: field.y + field.height + MENU_GAP,
        width: field.width,
        height,
    };
    let show_checks = options.iter().any(|option| option.checked);
    let check_reserve = if show_checks { 16.0 } else { 0.0 };
    let options = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let y = surface.y + MENU_PAD + index as f32 * (item_height + MENU_ITEM_GAP);
            let selected = highlighted == Some(index);
            let bounds = LayoutBox {
                x: surface.x + MENU_PAD,
                y,
                width: (surface.width - MENU_PAD * 2.0).max(0.0),
                height: item_height,
            };
            crate::SelectOptionGeometry {
                bounds,
                label: ComponentTextRegion {
                    bounds: LayoutBox {
                        x: bounds.x + size.padding_x() + check_reserve,
                        y: bounds.y,
                        width: (bounds.width - size.padding_x() * 2.0 - check_reserve).max(0.0),
                        height: bounds.height,
                    },
                    content: menu_option_label(option),
                    color: Some(if option.disabled {
                        palette.faint.as_rgba_array()
                    } else {
                        palette.text.as_rgba_array()
                    }),
                    font_size: size.text_size(),
                    font_weight: None,
                },
                selected,
                checked: option.checked,
                disabled: option.disabled,
                background: selected.then_some(palette.selected.as_rgba_array()),
                icon: None,
            }
        })
        .collect();
    crate::SelectMenuGeometry {
        surface,
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, if is_light { 0.24 } else { 0.48 }],
            offset_y: 8.0,
            blur_radius: 16.0,
        },
        background: palette.surface.as_rgba_array(),
        border: palette.border_soft.as_rgba_array(),
        options,
    }
}

pub(crate) fn menu_option_label(option: &SelectOptionData) -> Arc<str> {
    match option.hint.as_ref() {
        Some(hint) if !hint.is_empty() => Arc::from(format!("{}  ·  {hint}", option.label)),
        _ => Arc::clone(&option.label),
    }
}

pub(crate) fn select_option_at(menu: &crate::SelectMenuGeometry, x: f32, y: f32) -> Option<usize> {
    menu.options
        .iter()
        .position(|option| !option.disabled && option.bounds.contains(x, y))
}

pub(crate) fn field_style_for_size(size: ControlSize) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Px(size.height())),
            padding_left: Some(LengthSpec::Px(size.padding_x())),
            padding_right: Some(LengthSpec::Px(size.padding_x())),
            border_width: Some(1.0),
            border_radius: Some(UI_METRICS.radius_sm),
            white_space_nowrap: true,
            ..nana_ui_core::LayoutStyle::default()
        }),
        foreground: Some(SemanticColorRole::Text),
        background: Some(SemanticColorRole::Background),
        border: Some(SemanticColorRole::Border),
        interaction: InteractionStyle {
            hovered: SemanticPaint {
                border: Some(SemanticColorRole::BorderStrong),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint {
                border: Some(SemanticColorRole::BorderStrong),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                background: Some(SemanticColorRole::Subtle),
                border: Some(SemanticColorRole::Border),
            },
            ..InteractionStyle::default()
        },
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample() -> Select {
        Select::new(Some("code"))
            .placeholder("Choose view")
            .options([
                SelectOption::new("code", "Code"),
                SelectOption::new("split", "Split").disabled(true),
                SelectOption::new("preview", "Preview"),
            ])
    }

    #[test]
    fn select_projects_closed_field_and_keeps_disabled_options() {
        let mut context = AppContext::new();
        let select = context.create_component(document(), sample()).unwrap();
        let id = select.stable_id();
        assert!(matches!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == "select"
        ));
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::Select {
                placeholder: false,
                opened: false,
                ..
            })
        ));
        assert_eq!(context.world().text(id), Some("Code"));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Background));
        assert_eq!(style.border, Some(SemanticColorRole::Border));
        assert_eq!(style.layout.border_radius, Some(UI_METRICS.radius_sm));
        let visual = match context.world().standard_visual(id) {
            Some(StandardVisual::Select { options, .. }) => options,
            _ => panic!("select visual"),
        };
        assert_eq!(visual.len(), 3);
        assert!(visual[1].disabled);
    }

    #[test]
    fn select_toggle_and_commit_skip_disabled_options() {
        let mut select = sample();
        assert!(select.toggle_open());
        assert!(select.opened);
        assert_eq!(select.highlighted, Some(0));
        assert!(select.highlight_delta(1));
        assert_eq!(select.highlighted, Some(2));
        let changed = select.commit_highlighted().expect("commit preview");
        assert_eq!(changed.value.as_ref(), "preview");
        assert!(!select.opened);
        assert_eq!(select.value.as_deref(), Some("preview"));
        assert!(select.select_index(1).is_none());
    }

    #[test]
    fn inactive_select_does_not_open() {
        let mut select = sample().disabled(true);
        assert!(!select.toggle_open());
        let mut loading = sample().loading(true);
        assert!(!loading.toggle_open());
    }

    #[test]
    fn opened_invalid_select_uses_a_thicker_danger_border() {
        let mut context = AppContext::new();
        let select = context
            .create_component(document(), sample().invalid(true).opened(true))
            .unwrap();
        let style = context.world().node_style(select.stable_id()).unwrap();
        assert_eq!(style.border, Some(SemanticColorRole::Danger));
        assert_eq!(style.layout.border_width, Some(2.0));
        assert!(matches!(
            context.world().standard_visual(select.stable_id()),
            Some(StandardVisual::Select {
                opened: true,
                invalid: true,
                ..
            })
        ));
    }

    #[test]
    fn placeholder_is_used_when_no_value_matches() {
        let select = Select::new(None::<Arc<str>>)
            .placeholder("Choose view")
            .options([SelectOption::new("code", "Code")]);
        let (label, placeholder) = select.display_label();
        assert_eq!(label.as_ref(), "Choose view");
        assert!(placeholder);
    }
}
