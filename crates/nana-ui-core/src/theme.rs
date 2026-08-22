use serde::{Deserialize, Serialize};

/// NanaUI's standard body and medium-control text size.
pub const UI_BASE_TEXT_SIZE: f32 = 13.0;

/// Non-color design tokens shared by layout and interaction primitives.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThemeMetrics {
    pub density: f32,
    pub radius_xs: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub compact_control_height: f32,
    pub control_height: f32,
    pub compact_control_padding_x: f32,
    pub control_padding_x: f32,
    pub selection_padding_x: f32,
    pub navigation_row_height: f32,
    pub selection_height: f32,
    pub icon_button_size: f32,
    pub sidebar_footer_button_size: f32,
    pub panel_padding_x: f32,
    pub panel_padding_y: f32,
    pub field_padding_x: f32,
    pub field_padding_y: f32,
    pub list_item_padding_x: f32,
    pub list_item_padding_y: f32,
    pub motion_fast_ms: u16,
    pub motion_standard_ms: u16,
}

/// Shared Lilia-style geometry used by every NanaUI component family.
pub const UI_METRICS: ThemeMetrics = ThemeMetrics {
    density: 1.0,
    radius_xs: 2.0,
    radius_sm: 6.0,
    radius_md: 10.0,
    radius_lg: 14.0,
    compact_control_height: 28.0,
    control_height: 32.0,
    compact_control_padding_x: 7.0,
    control_padding_x: 10.0,
    selection_padding_x: 12.0,
    navigation_row_height: 28.0,
    selection_height: 36.0,
    icon_button_size: 28.0,
    sidebar_footer_button_size: 28.0,
    panel_padding_x: 16.0,
    panel_padding_y: 14.0,
    field_padding_x: 9.0,
    field_padding_y: 6.0,
    list_item_padding_x: 9.0,
    list_item_padding_y: 6.0,
    motion_fast_ms: 120,
    motion_standard_ms: 240,
};

impl Default for ThemeMetrics {
    fn default() -> Self {
        UI_METRICS
    }
}

impl ThemeMetrics {
    /// Small single-line control height.
    pub const fn small_control_height(self) -> f32 {
        self.compact_control_height
    }

    /// Medium single-line control height.
    pub const fn medium_control_height(self) -> f32 {
        self.control_height
    }

    /// Large single-line control height.
    ///
    /// `selection_height` remains the serialized backing field for public
    /// compatibility, while components consume this semantic accessor.
    pub const fn large_control_height(self) -> f32 {
        self.selection_height
    }
}

/// The two application themes currently supported by the design system.
///
/// Part of the Style Model **Tokens** slice ([`crate::style_model`]).
/// Palette RGBA values live on [`crate::SemanticPalette`]; `nana-ui::theme::Colors`
/// is the paint adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    pub fn metrics(self) -> ThemeMetrics {
        let _ = self;
        UI_METRICS
    }

    pub const fn palette(self) -> crate::SemanticPalette {
        crate::SemanticPalette::for_mode(self)
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeMode;

    #[test]
    fn theme_mode_round_trips_for_host_persistence() {
        let encoded = serde_json::to_string(&ThemeMode::Light).expect("theme serializes");
        let restored: ThemeMode = serde_json::from_str(&encoded).expect("theme restores");
        assert_eq!(restored, ThemeMode::Light);
    }
}
