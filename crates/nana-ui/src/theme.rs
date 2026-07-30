use iced::widget::{Row, row, text};
use iced::{Alignment, Color, Font, Theme, font};
use serde::{Deserialize, Serialize};

/// Semantic colors shared by the NanaUI shell and its widgets.
///
/// The values follow the same hierarchy as LiliaUI's dark/light tokens. They
/// are kept as semantic fields so future renderers can consume the palette
/// without reaching into individual widget implementations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub background: Color,
    pub surface: Color,
    pub subtle: Color,
    pub hover: Color,
    pub active: Color,
    pub selected: Color,
    pub selected_hover: Color,
    pub selected_pressed: Color,
    pub border: Color,
    pub border_soft: Color,
    pub border_strong: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub accent_strong: Color,
    pub accent_soft: Color,
    pub accent_on_soft: Color,
    pub accent_text: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

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
    selection_height: 34.0,
    icon_button_size: 28.0,
    sidebar_footer_button_size: 26.0,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeTokens {
    pub colors: Colors,
    pub metrics: ThemeMetrics,
    pub workspace_corners_enabled: bool,
}

impl ThemeTokens {
    pub const fn new(colors: Colors, metrics: ThemeMetrics) -> Self {
        Self {
            colors,
            metrics,
            workspace_corners_enabled: true,
        }
    }

    pub const fn with_workspace_corners(mut self, enabled: bool) -> Self {
        self.workspace_corners_enabled = enabled;
        self
    }
}

impl From<Colors> for ThemeTokens {
    fn from(colors: Colors) -> Self {
        Self::new(colors, UI_METRICS)
    }
}

/// LiliaUI's regular Noto Sans SC face, converted losslessly to TTF for Iced.
pub const UI_FONT_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf").as_slice();
/// LiliaUI's medium Noto Sans SC face, converted losslessly to TTF for Iced.
pub const UI_FONT_MEDIUM: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-Medium.ttf").as_slice();
/// LiliaUI's semibold Noto Sans SC face, converted losslessly to TTF for Iced.
pub const UI_FONT_SEMIBOLD: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-SemiBold.ttf").as_slice();
/// LiliaUI's bold Noto Sans SC face, converted losslessly to TTF for Iced.
pub const UI_FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Bold.ttf").as_slice();

/// Returns every bundled UI face for registration with an Iced application or
/// renderer.
pub const fn ui_font_sources() -> [&'static [u8]; 4] {
    [
        UI_FONT_REGULAR,
        UI_FONT_MEDIUM,
        UI_FONT_SEMIBOLD,
        UI_FONT_BOLD,
    ]
}

/// Resolves LiliaUI's Noto Sans SC family at the requested weight.
///
/// Hosts must register [`ui_font_sources`] once when they construct their Iced
/// application or renderer.
pub fn ui_font(weight: font::Weight) -> Font {
    Font {
        weight,
        ..Font::new("Noto Sans SC")
    }
}

pub(crate) fn tracked_label<'a, Message: 'a>(
    label: &str,
    size: f32,
    weight: font::Weight,
    tracking: f32,
    color: Color,
) -> Row<'a, Message> {
    let mut content = row![].spacing(tracking).align_y(Alignment::Center);
    for character in label.chars() {
        content = content.push(
            text(character.to_string())
                .size(size)
                .font(ui_font(weight))
                .color(color),
        );
    }
    content
}

/// The two application themes currently supported by the design system.
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

    pub fn colors(self) -> Colors {
        match self {
            Self::Dark => Colors {
                background: Color::from_rgb8(24, 24, 24),
                surface: Color::from_rgb8(32, 32, 32),
                subtle: Color::from_rgb8(28, 28, 28),
                hover: Color::from_rgb8(45, 45, 45),
                active: Color::from_rgb8(53, 53, 53),
                selected: Color::from_rgb8(53, 53, 53),
                selected_hover: Color::from_rgb8(60, 60, 60),
                selected_pressed: Color::from_rgb8(47, 47, 47),
                border: Color::from_rgb8(42, 42, 42),
                border_soft: Color::from_rgb8(35, 35, 35),
                border_strong: Color::from_rgb8(58, 58, 58),
                text: Color::from_rgb8(221, 221, 221),
                muted: Color::from_rgb8(163, 163, 163),
                faint: Color::from_rgb8(90, 90, 90),
                accent: Color::from_rgb8(123, 185, 240),
                accent_strong: Color::from_rgb8(73, 145, 215),
                accent_soft: Color::from_rgba8(123, 185, 240, 0.14),
                accent_on_soft: Color::from_rgb8(123, 185, 240),
                accent_text: Color::from_rgb8(13, 22, 34),
                success: Color::from_rgb8(63, 185, 80),
                warning: Color::from_rgb8(212, 168, 91),
                danger: Color::from_rgb8(244, 113, 116),
            },
            Self::Light => Colors {
                background: Color::from_rgb8(255, 255, 255),
                surface: Color::from_rgb8(243, 244, 246),
                subtle: Color::from_rgb8(247, 248, 250),
                hover: Color::from_rgb8(235, 237, 240),
                active: Color::from_rgb8(223, 226, 231),
                selected: Color::from_rgb8(226, 226, 226),
                selected_hover: Color::from_rgb8(232, 232, 232),
                selected_pressed: Color::from_rgb8(223, 223, 223),
                border: Color::from_rgb8(227, 229, 232),
                border_soft: Color::from_rgb8(238, 240, 243),
                border_strong: Color::from_rgb8(203, 207, 213),
                text: Color::from_rgb8(26, 26, 31),
                muted: Color::from_rgb8(90, 97, 110),
                faint: Color::from_rgb8(156, 163, 175),
                accent: Color::from_rgb8(73, 145, 215),
                accent_strong: Color::from_rgb8(44, 126, 214),
                accent_soft: Color::from_rgba8(73, 145, 215, 0.10),
                accent_on_soft: Color::from_rgb8(0, 85, 159),
                accent_text: Color::WHITE,
                success: Color::from_rgb8(16, 126, 57),
                warning: Color::from_rgb8(184, 119, 28),
                danger: Color::from_rgb8(201, 60, 60),
            },
        }
    }

    pub fn metrics(self) -> ThemeMetrics {
        let _ = self;
        UI_METRICS
    }

    pub fn tokens(self) -> ThemeTokens {
        ThemeTokens::new(self.colors(), self.metrics())
    }

    /// Converts the semantic palette to Iced's application theme.
    pub fn iced_theme(self) -> Theme {
        let colors = self.colors();
        Theme::custom(
            match self {
                Self::Dark => "Nana Dark",
                Self::Light => "Nana Light",
            },
            iced::theme::palette::Seed {
                background: colors.background,
                text: colors.text,
                primary: colors.accent,
                success: colors.success,
                warning: colors.warning,
                danger: colors.danger,
            },
        )
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
