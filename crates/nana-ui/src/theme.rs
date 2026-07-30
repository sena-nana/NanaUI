use iced::{Color, Theme};

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeMetrics {
    pub density: f32,
    pub radius_xs: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub motion_fast_ms: u16,
    pub motion_standard_ms: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeTokens {
    pub colors: Colors,
    pub metrics: ThemeMetrics,
}

/// The two application themes currently supported by the design system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
                selected: Color::from_rgb8(223, 223, 223),
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
        ThemeMetrics {
            density: 1.0,
            radius_xs: 8.0,
            radius_sm: 12.0,
            radius_md: 16.0,
            radius_lg: 20.0,
            motion_fast_ms: 120,
            motion_standard_ms: 240,
        }
    }

    pub fn tokens(self) -> ThemeTokens {
        ThemeTokens {
            colors: self.colors(),
            metrics: self.metrics(),
        }
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
