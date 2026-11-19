//! Declarative component recipes for high-frequency controls (#341).
//!
//! Each recipe moves role, metric-step, width, weight, alpha and elevation
//! decisions out of runtime component bodies and into the compiled theme so
//! custom themes and `ThemeScope` overlays drive them without hardcoding.

use serde::{Deserialize, Serialize};

use super::RadiusTier;
use super::tokens::{ElevationRole, SpacingStep, TextWeight, TypographyTokens};
use crate::style_model::{SemanticColorMix, SemanticColorRole};

/// Recipe for single- and multi-line input controls (`TextInput`, `NumberInput`, `TextArea`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRecipe {
    pub background: SemanticColorRole,
    pub foreground: SemanticColorRole,
    pub border: SemanticColorRole,
    pub hover_border: SemanticColorRole,
    pub focus_border: SemanticColorRole,
    pub focus_background_mix: SemanticColorMix,
    pub disabled_foreground: SemanticColorRole,
    pub disabled_background: SemanticColorRole,
    pub disabled_border: SemanticColorRole,
    pub invalid_border: SemanticColorRole,
    pub invalid_focus_border_width: f32,
    pub selection_background: SemanticColorRole,
    pub preedit_underline: SemanticColorRole,
    pub placeholder: SemanticColorRole,
    pub multiline_height_factor: f32,
    pub multiline_line_height: f32,
    pub additional_caret_alpha: f32,
}

impl InputRecipe {
    pub const DEFAULT: Self = Self {
        background: SemanticColorRole::Background,
        foreground: SemanticColorRole::Text,
        border: SemanticColorRole::Border,
        hover_border: SemanticColorRole::BorderStrong,
        focus_border: SemanticColorRole::FocusBorder,
        focus_background_mix: SemanticColorMix::new(
            SemanticColorRole::Background,
            SemanticColorRole::Text,
            0.95,
        ),
        disabled_foreground: SemanticColorRole::Faint,
        disabled_background: SemanticColorRole::Subtle,
        disabled_border: SemanticColorRole::Border,
        invalid_border: SemanticColorRole::Danger,
        invalid_focus_border_width: 2.0,
        selection_background: SemanticColorRole::AccentSoft,
        preedit_underline: SemanticColorRole::Accent,
        placeholder: SemanticColorRole::Faint,
        multiline_height_factor: 3.0,
        multiline_line_height: 1.45,
        additional_caret_alpha: 0.55,
    };

    pub(super) const fn lengths(&self) -> [(&'static str, f32); 1] {
        [(
            "input.invalid_focus_border_width",
            self.invalid_focus_border_width,
        )]
    }
}

impl Default for InputRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Recipe for `Select`, `Dropdown`, and `SearchDropdown`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectRecipe {
    pub background: SemanticColorRole,
    pub inactive_background: SemanticColorRole,
    pub foreground: SemanticColorRole,
    pub placeholder: SemanticColorRole,
    pub hint: SemanticColorRole,
    pub handle: SemanticColorRole,
    pub border: SemanticColorRole,
    pub open_border: SemanticColorRole,
    pub hover_border: SemanticColorRole,
    pub focus_border: SemanticColorRole,
    pub select_focus_border: SemanticColorRole,
    pub disabled_foreground: SemanticColorRole,
    pub disabled_background: SemanticColorRole,
    pub disabled_border: SemanticColorRole,
    pub invalid_border: SemanticColorRole,
    pub invalid_open_border_width: f32,
    pub bare_open_background: SemanticColorRole,
    pub bare_hover_background: SemanticColorRole,
    pub bare_pressed_background: SemanticColorRole,
    pub menu_selected_background: SemanticColorRole,
    pub menu_elevation: ElevationRole,
    pub checkmark_width: f32,
    pub checkmark_weight: u16,
}

impl SelectRecipe {
    pub const DEFAULT: Self = Self {
        background: SemanticColorRole::Background,
        inactive_background: SemanticColorRole::Subtle,
        foreground: SemanticColorRole::Text,
        placeholder: SemanticColorRole::Faint,
        hint: SemanticColorRole::Muted,
        handle: SemanticColorRole::Muted,
        border: SemanticColorRole::Border,
        open_border: SemanticColorRole::BorderSoft,
        hover_border: SemanticColorRole::BorderStrong,
        focus_border: SemanticColorRole::FocusBorder,
        select_focus_border: SemanticColorRole::BorderStrong,
        disabled_foreground: SemanticColorRole::Faint,
        disabled_background: SemanticColorRole::Subtle,
        disabled_border: SemanticColorRole::Border,
        invalid_border: SemanticColorRole::Danger,
        invalid_open_border_width: 2.0,
        bare_open_background: SemanticColorRole::Hover,
        bare_hover_background: SemanticColorRole::Hover,
        bare_pressed_background: SemanticColorRole::Active,
        menu_selected_background: SemanticColorRole::Selected,
        menu_elevation: ElevationRole::Dropdown,
        checkmark_width: 16.0,
        checkmark_weight: TypographyTokens::DEFAULT.bold,
    };

    pub(super) const fn lengths(&self) -> [(&'static str, f32); 2] {
        [
            (
                "select.invalid_open_border_width",
                self.invalid_open_border_width,
            ),
            ("select.checkmark_width", self.checkmark_width),
        ]
    }
}

impl Default for SelectRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Recipe for `Tabs` strips and tab options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabsRecipe {
    pub strip_gap: SpacingStep,
    pub option_padding_extra: SpacingStep,
    pub icon_gap: SpacingStep,
    pub font_weight: TextWeight,
    pub foreground: SemanticColorRole,
    pub hover_foreground: SemanticColorRole,
    pub selected_foreground: SemanticColorRole,
    pub disabled_foreground: SemanticColorRole,
    pub hover_background: SemanticColorRole,
    pub pressed_background: SemanticColorRole,
    pub selected_background: SemanticColorRole,
    pub selected_hover_background: SemanticColorRole,
    pub selected_pressed_background: SemanticColorRole,
    pub focus_surface: SemanticColorRole,
    pub focus_border: SemanticColorRole,
    pub focus_text: SemanticColorRole,
}

impl TabsRecipe {
    pub const DEFAULT: Self = Self {
        strip_gap: SpacingStep::Xs,
        option_padding_extra: SpacingStep::Xxs,
        icon_gap: SpacingStep::Xs,
        font_weight: TextWeight::Medium,
        foreground: SemanticColorRole::Muted,
        hover_foreground: SemanticColorRole::Text,
        selected_foreground: SemanticColorRole::Text,
        disabled_foreground: SemanticColorRole::Faint,
        hover_background: SemanticColorRole::Hover,
        pressed_background: SemanticColorRole::Active,
        selected_background: SemanticColorRole::Selected,
        selected_hover_background: SemanticColorRole::SelectedHover,
        selected_pressed_background: SemanticColorRole::SelectedPressed,
        focus_surface: SemanticColorRole::FocusSurface,
        focus_border: SemanticColorRole::FocusBorder,
        focus_text: SemanticColorRole::FocusText,
    };
}

impl Default for TabsRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Recipe for `TreeView` rows, icons, toggle indicators, and drag-drop marks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeViewRecipe {
    pub selected_background: SemanticColorRole,
    pub hover_background: Option<SemanticColorRole>,
    pub foreground: SemanticColorRole,
    pub disabled_foreground: SemanticColorRole,
    pub icon: SemanticColorRole,
    pub disabled_icon: SemanticColorRole,
    pub toggle_on: SemanticColorRole,
    pub toggle_off: SemanticColorRole,
    pub selected_weight: u16,
    pub drop_indicator: SemanticColorRole,
    pub drop_line_width: f32,
    pub drop_outline_width: f32,
}

impl TreeViewRecipe {
    pub const DEFAULT: Self = Self {
        selected_background: SemanticColorRole::Selected,
        hover_background: None,
        foreground: SemanticColorRole::Text,
        disabled_foreground: SemanticColorRole::Faint,
        icon: SemanticColorRole::Muted,
        disabled_icon: SemanticColorRole::Faint,
        toggle_on: SemanticColorRole::Muted,
        toggle_off: SemanticColorRole::Faint,
        selected_weight: TypographyTokens::DEFAULT.semibold,
        drop_indicator: SemanticColorRole::Accent,
        drop_line_width: 2.0,
        drop_outline_width: 2.0,
    };

    pub(super) const fn lengths(&self) -> [(&'static str, f32); 2] {
        [
            ("tree_view.drop_line_width", self.drop_line_width),
            ("tree_view.drop_outline_width", self.drop_outline_width),
        ]
    }
}

impl Default for TreeViewRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Recipe for `ActionMenu`, `ContextMenu`, `Popover` menus, and `AppMenuBar`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuRecipe {
    pub surface_background: SemanticColorRole,
    pub surface_border: SemanticColorRole,
    pub surface_radius: RadiusTier,
    pub surface_padding: SpacingStep,
    pub item_gap: SpacingStep,
    pub surface_elevation: ElevationRole,
    pub bar_elevation: ElevationRole,
    pub separator_color: SemanticColorRole,
    pub item_foreground: SemanticColorRole,
    pub item_danger_foreground: SemanticColorRole,
    pub item_disabled_foreground: SemanticColorRole,
    pub item_icon: SemanticColorRole,
    pub item_hint: SemanticColorRole,
    pub item_hover_background: SemanticColorRole,
    pub item_pressed_background: SemanticColorRole,
    pub item_danger_hover_background: SemanticColorRole,
    pub item_danger_pressed_background: SemanticColorRole,
    pub item_label_weight: TextWeight,
    pub bar_padding_x: SpacingStep,
    pub bar_title_padding_x: SpacingStep,
    pub mark_width: f32,
}

impl MenuRecipe {
    pub const DEFAULT: Self = Self {
        surface_background: SemanticColorRole::Surface,
        surface_border: SemanticColorRole::BorderSoft,
        surface_radius: RadiusTier::Md,
        surface_padding: SpacingStep::Xs,
        item_gap: SpacingStep::Xxs,
        surface_elevation: ElevationRole::Menu,
        bar_elevation: ElevationRole::MenuBar,
        separator_color: SemanticColorRole::Border,
        item_foreground: SemanticColorRole::Text,
        item_danger_foreground: SemanticColorRole::Danger,
        item_disabled_foreground: SemanticColorRole::Faint,
        item_icon: SemanticColorRole::Muted,
        item_hint: SemanticColorRole::Muted,
        item_hover_background: SemanticColorRole::Hover,
        item_pressed_background: SemanticColorRole::Active,
        item_danger_hover_background: SemanticColorRole::AccentSoft,
        item_danger_pressed_background: SemanticColorRole::AccentSoftPressed,
        item_label_weight: TextWeight::Medium,
        bar_padding_x: SpacingStep::Xxs,
        bar_title_padding_x: SpacingStep::Sm,
        mark_width: 16.0,
    };

    pub(super) const fn lengths(&self) -> [(&'static str, f32); 1] {
        [("menu.mark_width", self.mark_width)]
    }
}

impl Default for MenuRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}
