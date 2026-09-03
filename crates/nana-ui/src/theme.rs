//! Style Model token adapter for the Nana Scene host.
//!
//! [`ThemeMetrics`] / [`ThemeMode`] / [`SemanticPalette`] live in `nana-ui-core`.
//! This module is the L3 token view used by the Scene host. It is **not** a CSS
//! / ThemeTokens factory for arbitrary L1 paint values — see
//! `nana_ui_core::style_model`.

use nana_ui_core::{AppearanceSettings, BackdropTarget};

pub use nana_ui_core::{
    SemanticColor, SemanticPalette, ThemeMetrics, ThemeMode, UI_BASE_TEXT_SIZE, UI_METRICS,
};

/// Linear RGBA color used by L3 token adapters. Same layout as [`SemanticColor`].
pub type Color = SemanticColor;

/// Semantic colors shared by the NanaUI shell.
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
            background: palette.background,
            surface: palette.surface,
            subtle: palette.subtle,
            hover: palette.hover,
            active: palette.active,
            selected: palette.selected,
            selected_hover: palette.selected_hover,
            selected_pressed: palette.selected_pressed,
            border: palette.border,
            border_soft: palette.border_soft,
            border_strong: palette.border_strong,
            text: palette.text,
            muted: palette.muted,
            faint: palette.faint,
            accent: palette.accent,
            accent_strong: palette.accent_strong,
            accent_soft: palette.accent_soft,
            accent_soft_hover: palette.accent_soft_hover,
            accent_soft_pressed: palette.accent_soft_pressed,
            accent_on_soft: palette.accent_on_soft,
            accent_text: palette.accent_text,
            success: palette.success,
            warning: palette.warning,
            danger: palette.danger,
        }
    }

    pub fn to_palette(self) -> SemanticPalette {
        SemanticPalette {
            background: self.background,
            surface: self.surface,
            subtle: self.subtle,
            hover: self.hover,
            active: self.active,
            selected: self.selected,
            selected_hover: self.selected_hover,
            selected_pressed: self.selected_pressed,
            border: self.border,
            border_soft: self.border_soft,
            border_strong: self.border_strong,
            text: self.text,
            muted: self.muted,
            faint: self.faint,
            accent: self.accent,
            accent_strong: self.accent_strong,
            accent_soft: self.accent_soft,
            accent_soft_hover: self.accent_soft_hover,
            accent_soft_pressed: self.accent_soft_pressed,
            accent_on_soft: self.accent_on_soft,
            accent_text: self.accent_text,
            success: self.success,
            warning: self.warning,
            danger: self.danger,
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

/// Runtime token bundle: semantic [`Colors`] + [`ThemeMetrics`].
///
/// This is the Style Model Tokens view for the Scene host — not a dump of
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

    /// Apply Appearance backdrop alphas when the window surface is transparent.
    ///
    /// Covers plain window alpha (`Transparent`) and native blur materials
    /// (Vibrancy / Mica / Acrylic); only a chosen-solid window skips this.
    ///
    /// - [`BackdropTarget::Sidebar`]: translucency on `surface` (sidebar/chrome).
    ///   Title bar follows only when `titlebar_follows_sidebar` is true.
    /// - [`BackdropTarget::Main`]: translucency on `background` (primary content).
    pub fn with_backdrop(
        mut self,
        transparent_surface: bool,
        target: BackdropTarget,
        opacity: f32,
        titlebar_follows_sidebar: bool,
    ) -> Self {
        if !transparent_surface {
            return self;
        }
        let opacity = AppearanceSettings::clamp_backdrop_opacity(opacity);
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

impl From<Colors> for ThemeTokens {
    fn from(colors: Colors) -> Self {
        Self::new(colors, UI_METRICS)
    }
}

/// Install Style Model tokens on a Runtime document after applying window material.
pub fn install_theme_tokens(
    context: &mut nana_ui_runtime::AppContext,
    mode: ThemeMode,
    tokens: ThemeTokens,
) -> Result<bool, nana_ui_runtime::FrameworkError> {
    context.set_style_tokens(
        mode,
        tokens.metrics,
        tokens.colors.to_palette(),
        tokens.titlebar,
    )
}

/// Token helpers for [`ThemeMode`].
pub trait ThemeModeExt: Copy {
    fn colors(self) -> Colors;
    fn tokens(self) -> ThemeTokens;
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
}

/// LiliaUI's regular Noto Sans SC face.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf").as_slice();
/// LiliaUI's medium Noto Sans SC face.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_MEDIUM: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-Medium.ttf").as_slice();
/// LiliaUI's semibold Noto Sans SC face.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_SEMIBOLD: &[u8] =
    include_bytes!("../assets/fonts/NotoSansSC-SemiBold.ttf").as_slice();
/// LiliaUI's bold Noto Sans SC face.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Bold.ttf").as_slice();

/// Returns every bundled UI face for registration with the Scene text shaper.
#[cfg(feature = "bundled-fonts")]
pub const fn ui_font_sources() -> [&'static [u8]; 4] {
    [
        UI_FONT_REGULAR,
        UI_FONT_MEDIUM,
        UI_FONT_SEMIBOLD,
        UI_FONT_BOLD,
    ]
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

        let solid = ThemeMode::Light
            .tokens()
            .with_backdrop(false, BackdropTarget::Sidebar, 0.5, true);
        assert!((solid.colors.surface.a - 1.0).abs() < f32::EPSILON);
        assert!((solid.titlebar.a - 1.0).abs() < f32::EPSILON);
        assert!((solid.colors.background.a - 1.0).abs() < f32::EPSILON);
    }
}
