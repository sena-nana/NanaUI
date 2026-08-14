//! Iced **adapter** for the Nana Style Model (Tokens slice).
//!
//! [`ThemeMetrics`] / [`ThemeMode`] / [`SemanticPalette`] live in `nana-ui-core`.
//! This module maps them to Iced [`Color`] / [`Theme`] for the sole paint path
//! (`nana-ui` widgets). It is **not** a CSS / ThemeTokens factory for arbitrary
//! L1 paint values — see `nana_ui_core::style_model`.

use iced::widget::{Row, row, text};
use iced::{Alignment, Color, Font, Theme, font};
use nana_ui_core::{AppearanceSettings, BackdropTarget};

pub use nana_ui_core::{
    SemanticColor, SemanticPalette, ThemeMetrics, ThemeMode, UI_BASE_TEXT_SIZE, UI_METRICS,
};

/// Semantic colors shared by the NanaUI shell and its widgets (Iced `Color`).
///
/// Adapter view of [`SemanticPalette`]. Prefer constructing via
/// [`Colors::from_palette`] / [`ThemeModeExt::colors`] so values stay single-sourced.
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
    pub accent_soft_hover: Color,
    pub accent_soft_pressed: Color,
    pub accent_on_soft: Color,
    pub accent_text: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

impl Colors {
    pub fn from_palette(palette: SemanticPalette) -> Self {
        Self {
            background: color_from_semantic(palette.background),
            surface: color_from_semantic(palette.surface),
            subtle: color_from_semantic(palette.subtle),
            hover: color_from_semantic(palette.hover),
            active: color_from_semantic(palette.active),
            selected: color_from_semantic(palette.selected),
            selected_hover: color_from_semantic(palette.selected_hover),
            selected_pressed: color_from_semantic(palette.selected_pressed),
            border: color_from_semantic(palette.border),
            border_soft: color_from_semantic(palette.border_soft),
            border_strong: color_from_semantic(palette.border_strong),
            text: color_from_semantic(palette.text),
            muted: color_from_semantic(palette.muted),
            faint: color_from_semantic(palette.faint),
            accent: color_from_semantic(palette.accent),
            accent_strong: color_from_semantic(palette.accent_strong),
            accent_soft: color_from_semantic(palette.accent_soft),
            accent_soft_hover: color_from_semantic(palette.accent_soft_hover),
            accent_soft_pressed: color_from_semantic(palette.accent_soft_pressed),
            accent_on_soft: color_from_semantic(palette.accent_on_soft),
            accent_text: color_from_semantic(palette.accent_text),
            success: color_from_semantic(palette.success),
            warning: color_from_semantic(palette.warning),
            danger: color_from_semantic(palette.danger),
        }
    }

    pub fn to_palette(self) -> SemanticPalette {
        SemanticPalette {
            background: semantic_from_color(self.background),
            surface: semantic_from_color(self.surface),
            subtle: semantic_from_color(self.subtle),
            hover: semantic_from_color(self.hover),
            active: semantic_from_color(self.active),
            selected: semantic_from_color(self.selected),
            selected_hover: semantic_from_color(self.selected_hover),
            selected_pressed: semantic_from_color(self.selected_pressed),
            border: semantic_from_color(self.border),
            border_soft: semantic_from_color(self.border_soft),
            border_strong: semantic_from_color(self.border_strong),
            text: semantic_from_color(self.text),
            muted: semantic_from_color(self.muted),
            faint: semantic_from_color(self.faint),
            accent: semantic_from_color(self.accent),
            accent_strong: semantic_from_color(self.accent_strong),
            accent_soft: semantic_from_color(self.accent_soft),
            accent_soft_hover: semantic_from_color(self.accent_soft_hover),
            accent_soft_pressed: semantic_from_color(self.accent_soft_pressed),
            accent_on_soft: semantic_from_color(self.accent_on_soft),
            accent_text: semantic_from_color(self.accent_text),
            success: semantic_from_color(self.success),
            warning: semantic_from_color(self.warning),
            danger: semantic_from_color(self.danger),
        }
    }
}

impl From<SemanticPalette> for Colors {
    fn from(palette: SemanticPalette) -> Self {
        Self::from_palette(palette)
    }
}

impl From<Colors> for SemanticPalette {
    fn from(colors: Colors) -> Self {
        colors.to_palette()
    }
}

fn color_from_semantic(c: SemanticColor) -> Color {
    Color::from_rgba(c.r, c.g, c.b, c.a)
}

fn semantic_from_color(c: Color) -> SemanticColor {
    SemanticColor::rgba(c.r, c.g, c.b, c.a)
}

/// Runtime token bundle for L3 widgets: semantic [`Colors`] + [`ThemeMetrics`].
///
/// This is the Style Model Tokens view on the Iced draw path — not a dump of
/// arbitrary CSS. L1 maps known theme tiers here; unknown business colors must
/// not invent formal token fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeTokens {
    pub colors: Colors,
    pub metrics: ThemeMetrics,
    pub workspace_corners_enabled: bool,
    /// Title-bar / chrome strip background.
    ///
    /// Defaults to [`Colors::surface`], but stays independent so
    /// `titlebar_follows_sidebar=false` can keep the title bar opaque while the
    /// sidebar surface remains translucent.
    pub titlebar: Color,
}

impl ThemeTokens {
    pub const fn new(colors: Colors, metrics: ThemeMetrics) -> Self {
        Self {
            titlebar: colors.surface,
            colors,
            metrics,
            workspace_corners_enabled: true,
        }
    }

    pub const fn with_workspace_corners(mut self, enabled: bool) -> Self {
        self.workspace_corners_enabled = enabled;
        self
    }

    pub const fn with_titlebar(mut self, titlebar: Color) -> Self {
        self.titlebar = titlebar;
        self
    }

    /// Apply Appearance backdrop alphas when a native window material is active.
    ///
    /// - [`BackdropTarget::Sidebar`]: translucency on `surface` (sidebar/chrome).
    ///   Title bar follows only when `titlebar_follows_sidebar` is true.
    /// - [`BackdropTarget::Main`]: translucency on `background` (primary content).
    pub fn with_backdrop(
        mut self,
        native_material: bool,
        target: BackdropTarget,
        opacity: f32,
        titlebar_follows_sidebar: bool,
    ) -> Self {
        if !native_material {
            return self;
        }
        let opacity = normalize_backdrop_opacity(opacity);
        match target {
            BackdropTarget::Sidebar => {
                self.colors.surface.a = opacity;
                if titlebar_follows_sidebar {
                    self.titlebar.a = opacity;
                } else {
                    self.titlebar.a = 1.0;
                }
            }
            BackdropTarget::Main => {
                self.colors.background.a = opacity;
            }
        }
        self
    }
}

fn normalize_backdrop_opacity(opacity: f32) -> f32 {
    if opacity.is_finite() {
        opacity.clamp(
            AppearanceSettings::MIN_BACKDROP_OPACITY,
            AppearanceSettings::MAX_BACKDROP_OPACITY,
        )
    } else {
        AppearanceSettings::DEFAULT_BACKDROP_OPACITY
    }
}

impl From<Colors> for ThemeTokens {
    fn from(colors: Colors) -> Self {
        Self::new(colors, UI_METRICS)
    }
}

/// Iced/backend helpers for [`ThemeMode`] (adapter layer only).
pub trait ThemeModeExt: Copy {
    fn colors(self) -> Colors;
    fn tokens(self) -> ThemeTokens;
    fn iced_theme(self) -> Theme;
    fn palette(self) -> SemanticPalette;
}

impl ThemeModeExt for ThemeMode {
    fn palette(self) -> SemanticPalette {
        ThemeMode::palette(self)
    }

    fn colors(self) -> Colors {
        Colors::from_palette(self.palette())
    }

    fn tokens(self) -> ThemeTokens {
        ThemeTokens::new(self.colors(), self.metrics())
    }

    fn iced_theme(self) -> Theme {
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

/// LiliaUI's regular Noto Sans SC face, converted losslessly to TTF for Iced.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf").as_slice();
/// LiliaUI's medium Noto Sans SC face, converted losslessly to TTF for Iced.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_MEDIUM: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-Medium.ttf").as_slice();
/// LiliaUI's semibold Noto Sans SC face, converted losslessly to TTF for Iced.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_SEMIBOLD: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-SemiBold.ttf").as_slice();
/// LiliaUI's bold Noto Sans SC face, converted losslessly to TTF for Iced.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Bold.ttf").as_slice();

/// Returns every bundled UI face for registration with an Iced application or
/// renderer.
#[cfg(feature = "bundled-fonts")]
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
/// Hosts using the `bundled-fonts` feature must register `ui_font_sources` once when they construct their Iced
/// application or renderer and run [`ui_font_defaults`] during application
/// startup.
pub fn ui_font(weight: font::Weight) -> Font {
    Font {
        weight,
        ..Font::new("Noto Sans SC")
    }
}

/// Applies NanaUI's shared font family and base text size to an Iced
/// application.
pub fn ui_font_defaults<Message>() -> iced::Task<Message> {
    iced::font::set_defaults(ui_font(font::Weight::Normal), UI_BASE_TEXT_SIZE)
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

#[cfg(test)]
mod tests {
    use super::{Colors, ThemeMode, ThemeModeExt, ThemeTokens};
    use nana_ui_core::{BackdropTarget, SemanticPalette};

    #[test]
    fn colors_round_trip_palette() {
        let palette = SemanticPalette::light();
        let colors = Colors::from_palette(palette);
        let back = colors.to_palette();
        assert_eq!(back.accent.r, palette.accent.r);
        assert_eq!(
            ThemeMode::Dark.colors().background,
            Colors::from_palette(SemanticPalette::dark()).background
        );
    }

    #[test]
    fn titlebar_follows_sidebar_controls_titlebar_alpha() {
        let base = ThemeMode::Light.tokens();
        let follows = base.with_backdrop(true, BackdropTarget::Sidebar, 0.5, true);
        assert!((follows.colors.surface.a - 0.5).abs() < f32::EPSILON);
        assert!((follows.titlebar.a - 0.5).abs() < f32::EPSILON);
        assert!((follows.colors.background.a - 1.0).abs() < f32::EPSILON);

        let independent = ThemeTokens::new(ThemeMode::Light.colors(), ThemeMode::Light.metrics())
            .with_backdrop(true, BackdropTarget::Sidebar, 0.5, false);
        assert!((independent.colors.surface.a - 0.5).abs() < f32::EPSILON);
        assert!(
            (independent.titlebar.a - 1.0).abs() < f32::EPSILON,
            "titlebar must stay opaque when follows=false"
        );

        let main = ThemeMode::Light
            .tokens()
            .with_backdrop(true, BackdropTarget::Main, 0.5, true);
        assert!((main.colors.background.a - 0.5).abs() < f32::EPSILON);
        assert!((main.colors.surface.a - 1.0).abs() < f32::EPSILON);
        assert!((main.titlebar.a - 1.0).abs() < f32::EPSILON);
    }
}
