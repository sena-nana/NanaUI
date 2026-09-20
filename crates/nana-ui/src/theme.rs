//! Style Model token adapter for the Nana Scene host.
//!
//! [`ThemeMetrics`] / [`ThemeMode`] / [`SemanticPalette`] live in `nana-ui-core`.
//! This module is the L3 token view used by the Scene host. It is **not** a CSS
//! / ThemeTokens factory for arbitrary L1 paint values — see
//! `nana_ui_core::style_model`.

use nana_ui_core::{AppearanceSettings, BackdropTarget};

pub use nana_ui_core::{
    HAIRLINE, SemanticColor, SemanticPalette, ThemeMetrics, ThemeMode, UI_BASE_TEXT_SIZE,
    UI_METRICS, space, type_scale,
};

/// Linear RGBA color used by L3 token adapters. Same layout as [`SemanticColor`].
pub type Color = SemanticColor;

/// Runtime token bundle: [`SemanticPalette`] + [`ThemeMetrics`] + chrome.
///
/// This is the Style Model Tokens view for the Scene host — not a dump of
/// arbitrary CSS. L1 maps known theme tiers here; unknown business colors must
/// not invent formal token fields.
///
/// The palette is the one in `nana-ui-core`, not a copy of it. There used to
/// be a `Colors` struct here with the same 24 fields and a pair of conversions
/// between the two; one design language does not get two spellings, and a
/// second spelling is where the two drift apart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeTokens {
    pub palette: SemanticPalette,
    pub metrics: ThemeMetrics,
    pub workspace_corners_enabled: bool,
    /// Title-bar / chrome strip background.
    ///
    /// Defaults to [`SemanticPalette::surface`], but stays independent so
    /// `titlebar_follows_sidebar=false` can keep the title bar opaque while the
    /// sidebar surface remains translucent.
    pub titlebar: Color,
}

impl ThemeTokens {
    pub const fn new(palette: SemanticPalette, metrics: ThemeMetrics) -> Self {
        Self {
            titlebar: palette.surface,
            palette,
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
                self.palette.surface.a = opacity;
                if titlebar_follows_sidebar {
                    self.titlebar.a = opacity;
                } else {
                    self.titlebar.a = 1.0;
                }
            }
            BackdropTarget::Main => {
                self.palette.background.a = opacity;
            }
        }
        self
    }
}

impl From<SemanticPalette> for ThemeTokens {
    fn from(palette: SemanticPalette) -> Self {
        Self::new(palette, UI_METRICS)
    }
}

/// Install Style Model tokens on a Runtime document after applying window material.
pub fn install_theme_tokens(
    context: &mut nana_ui_runtime::AppContext,
    mode: ThemeMode,
    tokens: ThemeTokens,
) -> Result<bool, nana_ui_runtime::FrameworkError> {
    context.set_style_tokens(mode, tokens.metrics, tokens.palette, tokens.titlebar)
}

/// Token helpers for [`ThemeMode`].
///
/// `colors()` is gone: it returned a second palette type with the same fields
/// as [`SemanticPalette`]. Use [`Self::palette`].
pub trait ThemeModeExt: Copy {
    fn tokens(self) -> ThemeTokens;
    fn palette(self) -> SemanticPalette;
}

impl ThemeModeExt for ThemeMode {
    fn palette(self) -> SemanticPalette {
        ThemeMode::palette(self)
    }

    fn tokens(self) -> ThemeTokens {
        ThemeTokens::new(self.palette(), self.metrics())
    }
}

// The faces themselves live in `nana-ui-core` so that the theme, the SVG
// rasterizer and Canvas2D all reference one copy — see `nana_ui_core::fonts`.
#[cfg(feature = "bundled-fonts")]
pub use nana_ui_core::fonts::{
    UI_FONT_BOLD, UI_FONT_MEDIUM, UI_FONT_REGULAR, UI_FONT_SEMIBOLD, ui_font_sources,
};

#[cfg(test)]
mod tests {
    use super::{ThemeMode, ThemeModeExt, ThemeTokens};
    use nana_ui_core::{BackdropTarget, SemanticPalette};

    /// One palette type, not two. The adapter used to expose a `Colors` struct
    /// with the same 24 fields; this asserts the token bundle now carries the
    /// core palette itself, so there is nothing to keep in sync.
    #[test]
    fn the_token_bundle_carries_the_core_palette_itself() {
        assert_eq!(ThemeMode::Dark.palette(), SemanticPalette::dark());
        assert_eq!(ThemeMode::Light.tokens().palette, SemanticPalette::light());
        assert_eq!(
            ThemeTokens::from(SemanticPalette::dark()).palette,
            SemanticPalette::dark()
        );
    }

    #[test]
    fn titlebar_follows_sidebar_controls_titlebar_alpha() {
        let base = ThemeMode::Light.tokens();
        let follows = base.with_backdrop(true, BackdropTarget::Sidebar, 0.5, true);
        assert!((follows.palette.surface.a - 0.5).abs() < f32::EPSILON);
        assert!((follows.titlebar.a - 0.5).abs() < f32::EPSILON);
        assert!((follows.palette.background.a - 1.0).abs() < f32::EPSILON);

        let independent = ThemeTokens::new(ThemeMode::Light.palette(), ThemeMode::Light.metrics())
            .with_backdrop(true, BackdropTarget::Sidebar, 0.5, false);
        assert!((independent.palette.surface.a - 0.5).abs() < f32::EPSILON);
        assert!(
            (independent.titlebar.a - 1.0).abs() < f32::EPSILON,
            "titlebar must stay opaque when follows=false"
        );

        let main = ThemeMode::Light
            .tokens()
            .with_backdrop(true, BackdropTarget::Main, 0.5, true);
        assert!((main.palette.background.a - 0.5).abs() < f32::EPSILON);
        assert!((main.palette.surface.a - 1.0).abs() < f32::EPSILON);
        assert!((main.titlebar.a - 1.0).abs() < f32::EPSILON);

        let solid =
            ThemeMode::Light
                .tokens()
                .with_backdrop(false, BackdropTarget::Sidebar, 0.5, true);
        assert!((solid.palette.surface.a - 1.0).abs() < f32::EPSILON);
        assert!((solid.titlebar.a - 1.0).abs() < f32::EPSILON);
        assert!((solid.palette.background.a - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn space_and_type_scale_are_on_the_theme_surface() {
        assert_eq!(super::space::MD, 8.0);
        assert_eq!(super::type_scale::BODY, super::UI_BASE_TEXT_SIZE);
        assert_eq!(super::type_scale::SEMIBOLD, 600);
    }
}
