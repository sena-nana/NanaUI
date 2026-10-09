//! What a chart takes from the theme: the colors it paints guides and text
//! with, and a categorical palette derived from the accent.

use std::collections::HashMap;

use nana_ui_core::{SemanticColorRole, SemanticPalette, type_scale};

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
    /// Series colors, in order.
    pub palette: Vec<[f32; 4]>,
    pub font_size: f32,
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
            palette: categorical(palette.accent.as_rgba_array(), surface),
            font_size: type_scale::META,
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

/// Hue steps from the accent, in degrees. Neighbours stay far apart so the
/// first few series read as distinct; later entries fill the gaps.
const HUE_STEPS: [f32; 9] = [0.0, 140.0, 75.0, 300.0, 200.0, 35.0, 250.0, 105.0, 335.0];

/// A categorical palette with the accent first and every entry at the
/// accent's perceived lightness and a shared chroma, so no series shouts
/// over another. Lightness follows the surface: lighter on dark themes.
fn categorical(accent: [f32; 4], surface: [f32; 4]) -> Vec<[f32; 4]> {
    let [l, c, h] = srgb_to_oklch(accent);
    let dark_surface = srgb_to_oklch(surface)[0] < 0.5;
    let lightness = if dark_surface {
        l.clamp(0.68, 0.76)
    } else {
        l.clamp(0.58, 0.68)
    };
    let chroma = c.clamp(0.10, 0.15);
    HUE_STEPS
        .iter()
        .enumerate()
        .map(|(index, step)| {
            if index == 0 {
                accent
            } else {
                oklch_to_srgb(lightness, chroma, h + step)
            }
        })
        .collect()
}

fn decode(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(channel: f32) -> f32 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// `[L, C, h°]` of an sRGB color.
pub(crate) fn srgb_to_oklch([r, g, b, _]: [f32; 4]) -> [f32; 3] {
    let (r, g, b) = (decode(r), decode(g), decode(b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    let chroma = (a * a + bb * bb).sqrt();
    let hue = bb.atan2(a).to_degrees().rem_euclid(360.0);
    [lightness, chroma, hue]
}

pub(crate) fn oklch_to_srgb(lightness: f32, chroma: f32, hue: f32) -> [f32; 4] {
    // Reduce chroma until the color fits sRGB, so a hue never clips into a
    // different-looking color.
    let mut chroma = chroma;
    loop {
        let (sin, cos) = hue.to_radians().sin_cos();
        let (a, b) = (chroma * cos, chroma * sin);
        let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
        let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
        let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
        let rgb = [
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        ];
        let fits = rgb.iter().all(|c| (-1e-4..=1.0 + 1e-4).contains(c));
        if fits || chroma <= 0.005 {
            return [encode(rgb[0]), encode(rgb[1]), encode(rgb[2]), 1.0];
        }
        chroma -= 0.005;
    }
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
            let [l, c, h] = srgb_to_oklch(color);
            let back = oklch_to_srgb(l, c, h);
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
        let accent = [0.25, 0.45, 0.95, 1.0];
        let palette = categorical(accent, [0.12, 0.12, 0.12, 1.0]);
        assert_eq!(palette[0], accent);
        assert_eq!(palette.len(), HUE_STEPS.len());
        for (i, a) in palette.iter().enumerate() {
            for b in &palette[i + 1..] {
                let distance: f32 = (0..3).map(|k| (a[k] - b[k]).abs()).sum();
                assert!(distance > 0.12, "{a:?} vs {b:?}");
            }
        }
    }
}
