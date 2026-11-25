//! What a chart takes from the theme: the colors it paints guides and text
//! with, and a categorical palette derived from the accent.

use std::collections::HashMap;

use nana_ui_core::{SemanticColorRole, SemanticPalette, TypographyTokens, type_scale};

use crate::option::{ChartColor, ChartOption};

/// Resolved theme colors for one layout. Straight-alpha sRGB.
#[derive(Clone)]
pub struct ChartTheme {
    pub text: [f32; 4],
    pub muted: [f32; 4],
    /// Split lines inside the plot.
    pub grid: [f32; 4],
    /// Axis lines and ticks.
    pub axis: [f32; 4],
    /// What symbols are filled with and slices are cut by: the chart's
    /// own surface.
    pub surface: [f32; 4],
    /// Alternate radar rings, bar backgrounds, gauge tracks, the zoom slider.
    pub band: [f32; 4],
    /// The axis pointer and slider handles.
    pub pointer: [f32; 4],
    /// Text drawn on a series color (inside pie labels).
    pub on_series: [f32; 4],
    /// Semantic positive / rising color (e.g. bullish candlestick).
    pub positive: [f32; 4],
    /// Semantic negative / falling color (e.g. bearish candlestick).
    pub negative: [f32; 4],
    /// Semantic warning / high-end ramp color.
    pub warning: [f32; 4],
    /// Series colors, in order.
    pub palette: Vec<[f32; 4]>,
    pub font_size: f32,
    pub font_weight: u16,
    roles: HashMap<SemanticColorRole, [f32; 4]>,
}

impl std::fmt::Debug for ChartTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartTheme")
            .field("palette", &self.palette)
            .finish_non_exhaustive()
    }
}

impl ChartTheme {
    /// `resolve` is the style model's role lookup, so derived roles (alpha
    /// mixes) resolve the way every other control sees them. Only the roles
    /// `option` names are looked up.
    pub fn new(
        palette: &SemanticPalette,
        option: &ChartOption,
        resolve: impl Fn(SemanticColorRole) -> [f32; 4],
    ) -> Self {
        let roles = option
            .color_roles()
            .into_iter()
            .map(|role| (role, resolve(role)))
            .collect();
        let mix = |a: [f32; 4], b: [f32; 4], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
                a[3] + (b[3] - a[3]) * t,
            ]
        };
        let surface = palette.surface.as_rgba_array();
        let text = palette.text.as_rgba_array();
        let mut theme = Self {
            text,
            muted: palette.muted.as_rgba_array(),
            // A split line sits between the surface and the border so a
            // dense grid recedes behind the data.
            grid: mix(surface, palette.border_strong.as_rgba_array(), 0.6),
            axis: palette.border_strong.as_rgba_array(),
            surface: [surface[0], surface[1], surface[2], 1.0],
            band: mix(surface, text, 0.05),
            pointer: mix(surface, palette.muted.as_rgba_array(), 0.7),
            on_series: palette.accent_text.as_rgba_array(),
            positive: palette.success.as_rgba_array(),
            negative: palette.danger.as_rgba_array(),
            warning: palette.warning.as_rgba_array(),
            palette: palette.series_palette().to_vec(),
            font_size: type_scale::META,
            font_weight: type_scale::REGULAR,
            roles,
        };
        // An option's own palette replaces the derived one.
        if !option.color.is_empty() {
            theme.palette = option
                .color
                .iter()
                .map(|color| theme.resolve(*color))
                .collect();
        }
        theme
    }

    /// Install the host's current label typography instead of the default scale.
    pub fn with_typography(mut self, typography: TypographyTokens) -> Self {
        self.font_size = typography.meta;
        self.font_weight = typography.regular;
        self
    }

    pub fn role(&self, role: SemanticColorRole) -> [f32; 4] {
        self.roles.get(&role).copied().unwrap_or(self.text)
    }

    pub fn resolve(&self, color: ChartColor) -> [f32; 4] {
        match color {
            ChartColor::Role(role) => self.role(role),
            ChartColor::Palette(index) => self.series_color(index),
            ChartColor::Rgba(rgba) => rgba,
        }
    }

    pub fn series_color(&self, index: usize) -> [f32; 4] {
        if self.palette.is_empty() {
            return self.text;
        }
        self.palette[index % self.palette.len()]
    }
}

/// The categorical palette a chart derives from `palette` when its option
/// names none: the accent first, then hues at the accent's lightness.
pub fn series_palette(palette: &SemanticPalette) -> Vec<[f32; 4]> {
    palette.series_palette().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_round_trips_srgb() {
        for color in [
            [0.2, 0.4, 0.8, 1.0],
            [0.9, 0.3, 0.1, 1.0],
            [0.5, 0.5, 0.5, 1.0],
        ] {
            let [l, c, h] = nana_ui_core::style_model::srgb_to_oklch(color);
            let back = nana_ui_core::style_model::oklch_to_srgb(l, c, h);
            for i in 0..3 {
                assert!((back[i] - color[i]).abs() < 2e-3, "{color:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn an_option_palette_replaces_the_derived_one() {
        let option = ChartOption::new().color([
            ChartColor::Rgba([1.0, 0.0, 0.0, 1.0]),
            ChartColor::Palette(1),
        ]);
        let palette = SemanticPalette::dark();
        let derived = ChartTheme::new(&palette, &ChartOption::new(), |_| [0.0; 4]);
        let theme = ChartTheme::new(&palette, &option, |_| [0.0; 4]);
        assert_eq!(theme.series_color(0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(theme.series_color(1), derived.series_color(1));
        assert_eq!(theme.series_color(2), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn palette_starts_with_the_accent_and_stays_distinct() {
        for semantic in [SemanticPalette::dark(), SemanticPalette::light()] {
            let palette = series_palette(&semantic);
            assert_eq!(palette[0], semantic.accent.as_rgba_array());
            assert_eq!(palette.len(), 9);
            for (i, a) in palette.iter().enumerate() {
                for b in &palette[i + 1..] {
                    let distance: f32 = (0..3).map(|k| (a[k] - b[k]).abs()).sum();
                    assert!(distance > 0.12, "{a:?} vs {b:?}");
                }
            }
        }
    }
}
