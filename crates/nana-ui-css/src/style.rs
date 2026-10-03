//! L1 CSS paint parsing at the Vue adapter boundary.
//!
//! Layout declarations go through [`crate::css_map::LayoutStyleCss`]. This
//! module only resolves paint values used by the Style Model and SVG adapter;
//! it does not own layout, hit-testing, or a second paint path.
//!
//! ## L1 色值策略
//!
//! | 来源 | 行为 |
//! |------|------|
//! | 已知 token / class（`accent`、`muted`、`var(--nana-*)`） | → [`SemanticColorRole`](nana_ui_core::SemanticColorRole) / 调色板字段 |
//! | 未知 `#hex` / `rgb()` | **不**写入正式 ThemeTokens；仅可作为 L1 paint hint |
//!
//! [`map_css_color_for_tokens`] 是正式 Tokens 路径的唯一入口；[`parse_css_color`]
//! 仅服务 L1 paint 解析。

use nana_ui_core::{SemanticColor, SemanticColorRole, ThemeAppearance};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static RESOLVED_PAINT_CACHE: RefCell<HashMap<String, CssPaintColor>> =
        RefCell::new(HashMap::new());
}

/// Authoring-space paint value. Semantic theme colors remain `SemanticColor`;
/// this type is only used at the CSS paint boundary so OKLCH metadata is not
/// silently confused with an already encoded sRGB tuple.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CssPaintColor {
    Srgb([f32; 4]),
    Oklch {
        l: f32,
        c: f32,
        h: Option<f32>,
        alpha: f32,
    },
    Hsv {
        h: f32,
        s: f32,
        v: f32,
        alpha: f32,
    },
    LinearScRgb {
        channels: [f32; 3],
        alpha: f32,
    },
}

impl CssPaintColor {
    pub fn to_core(self) -> nana_ui_core::PaintColor {
        match self {
            Self::Srgb(rgba) => nana_ui_core::PaintColor::Srgb { rgba },
            Self::Oklch { l, c, h, alpha } => nana_ui_core::PaintColor::Oklch { l, c, h, alpha },
            Self::Hsv { h, s, v, alpha } => nana_ui_core::PaintColor::Hsv { h, s, v, alpha },
            Self::LinearScRgb { channels, alpha } => {
                nana_ui_core::PaintColor::LinearScRgb { channels, alpha }
            }
        }
    }

    pub fn to_srgb(self) -> [f32; 4] {
        self.to_core().to_srgb()
    }

    pub fn to_linear_sc_rgb(self) -> ([f32; 3], f32) {
        self.to_core().to_linear_sc_rgb()
    }
}

/// Parse an authoring color without discarding its color-space metadata.
pub fn parse_css_paint_color(input: &str) -> Option<CssPaintColor> {
    let lower = input.trim().to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("light-dark(")
        .and_then(|r| r.strip_suffix(')'))
        && let Some((light, dark)) = split_top_level_comma_pair(rest)
    {
        let chosen = if crate::css_map::active_color_scheme_is_dark() {
            dark
        } else {
            light
        };
        // Preserve the selected branch's authoring space (OKLCH/HSV/scRGB)
        // rather than routing through parse_css_color, which yields sRGB.
        if !chosen
            .trim()
            .to_ascii_lowercase()
            .starts_with("light-dark(")
        {
            return parse_css_paint_color(chosen.trim());
        }
        return None;
    }
    if let Some(rest) = lower
        .strip_prefix("oklch(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let (body, alpha) = if let Some((lhs, rhs)) = rest.split_once('/') {
            (lhs.trim(), parse_oklch_alpha(rhs.trim())?)
        } else {
            (rest.trim(), 1.0)
        };
        let parts: Vec<_> = body.split_whitespace().collect();
        if parts.len() >= 3 {
            return Some(CssPaintColor::Oklch {
                l: parse_oklch_lightness(parts[0])?,
                c: parse_oklch_chroma(parts[1])?,
                h: parse_oklch_hue(parts[2]),
                alpha: alpha.clamp(0.0, 1.0),
            });
        }
        return None;
    }
    if let Some(rest) = lower.strip_prefix("hsv(").and_then(|r| r.strip_suffix(')')) {
        let (body, alpha) = if let Some((lhs, rhs)) = rest.split_once('/') {
            (lhs.trim(), parse_oklch_alpha(rhs.trim())?)
        } else {
            (rest.trim(), 1.0)
        };
        let parts: Vec<_> = body.split_whitespace().collect();
        if parts.len() >= 3 {
            return Some(CssPaintColor::Hsv {
                h: parse_oklch_hue(parts[0]).unwrap_or(0.0),
                s: parse_unit_interval(parts[1])?,
                v: parse_unit_interval(parts[2])?,
                alpha: alpha.clamp(0.0, 1.0),
            });
        }
        return None;
    }
    if lower.starts_with("color(srgb-linear ") || lower.starts_with("color(scrgb ") {
        return parse_linear_scrgb(&lower);
    }
    if lower.starts_with("color-mix(") {
        return parse_color_mix_paint(input);
    }
    // A malformed recognized function must fail closed. Sending it back
    // through `parse_css_color` would recurse because that parser delegates
    // OKLCH/HSV functions here (outline tokenization can expose such pieces).
    if lower.starts_with("oklch(") || lower.starts_with("hsv(") {
        return None;
    }
    parse_css_color(input).map(CssPaintColor::Srgb)
}

/// Resolve an authoring color after expanding active CSS custom properties.
/// The returned value still carries its source color space.
pub fn resolve_css_paint_color(input: &str) -> Option<CssPaintColor> {
    let expanded = crate::css_map::expand_css_var_fallback(input.trim());
    // `light-dark()` depends on the active document scheme even when its
    // source text and expanded custom properties are unchanged.
    let cache_key = format!(
        "{}\u{1f}{}",
        expanded,
        crate::css_map::active_color_scheme_is_dark() as u8
    );
    RESOLVED_PAINT_CACHE.with(|cache| {
        if let Some(color) = cache.borrow().get(&cache_key).copied() {
            return Some(color);
        }
        let color = parse_css_paint_color(&expanded)?;
        let mut cache = cache.borrow_mut();
        if cache.len() >= 4096 {
            cache.clear();
        }
        cache.insert(cache_key, color);
        Some(color)
    })
}

/// Parse `#rgb` / `#rrggbb` / `#rrggbbaa` / `rgb()` / `rgba()` / named colors.
///
/// **L1 paint only** — does not create formal ThemeTokens.
/// Prefer [`map_css_color_for_tokens`] on the Tokens path.
pub fn parse_css_color(input: &str) -> Option<[f32; 4]> {
    let s = input.trim();
    if s.is_empty() {
        return Some([0.0, 0.0, 0.0, 0.0]);
    }
    if let Some(c) = parse_light_dark_color(s) {
        return Some(c);
    }
    if let Some(c) = parse_color_mix(s) {
        return Some(c);
    }
    if let Some(c) = parse_css_named_color(s) {
        return Some(c);
    }
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("rgba(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<_> = rest.split(',').map(str::trim).collect();
        if parts.len() == 4 {
            let r = parse_rgb_channel(parts[0])?;
            let g = parse_rgb_channel(parts[1])?;
            let b = parse_rgb_channel(parts[2])?;
            let a = parse_finite_f32(parts[3])?.clamp(0.0, 1.0);
            return Some([r, g, b, a]);
        }
    }
    if let Some(rest) = lower.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<_> = rest.split(',').map(str::trim).collect();
        if parts.len() == 3 {
            let r = parse_rgb_channel(parts[0])?;
            let g = parse_rgb_channel(parts[1])?;
            let b = parse_rgb_channel(parts[2])?;
            return Some([r, g, b, 1.0]);
        }
    }
    // CSS Color 4 OKLCH, converted through OKLab and linear sRGB.
    if lower.starts_with("oklch(")
        || lower.starts_with("hsv(")
        || lower.starts_with("color(srgb-linear ")
        || lower.starts_with("color(scrgb ")
    {
        return parse_css_paint_color(s).map(CssPaintColor::to_srgb);
    }
    None
}

/// CSS2 / CSS Color 3 named keywords used by L1 paint, tests, and UI.
///
/// Single table for [`parse_css_color`] and shadow-token classification.
pub fn parse_css_named_color(input: &str) -> Option<[f32; 4]> {
    match input.trim().to_ascii_lowercase().as_str() {
        "transparent" => Some([0.0, 0.0, 0.0, 0.0]),
        // CSS 2.1 color keywords (HTML4 + orange).
        "aqua" => Some([0.0, 1.0, 1.0, 1.0]),
        "black" => Some([0.0, 0.0, 0.0, 1.0]),
        "blue" => Some([0.0, 0.0, 1.0, 1.0]),
        "fuchsia" => Some([1.0, 0.0, 1.0, 1.0]),
        "gray" | "grey" => Some([0.5, 0.5, 0.5, 1.0]),
        "green" => Some([0.0, 0.5, 0.0, 1.0]),
        "lime" => Some([0.0, 1.0, 0.0, 1.0]),
        "maroon" => Some([0.5, 0.0, 0.0, 1.0]),
        "navy" => Some([0.0, 0.0, 0.5, 1.0]),
        "olive" => Some([0.5, 0.5, 0.0, 1.0]),
        "orange" => Some([1.0, 165.0 / 255.0, 0.0, 1.0]),
        "purple" => Some([0.5, 0.0, 0.5, 1.0]),
        "red" => Some([1.0, 0.0, 0.0, 1.0]),
        "silver" => Some([192.0 / 255.0, 192.0 / 255.0, 192.0 / 255.0, 1.0]),
        "teal" => Some([0.0, 0.5, 0.5, 1.0]),
        "white" => Some([1.0, 1.0, 1.0, 1.0]),
        "yellow" => Some([1.0, 1.0, 0.0, 1.0]),
        // CSS Color 3 names already used by L1 paint.
        "coral" => Some([1.0, 0.5, 0.31, 1.0]),
        "dodgerblue" => Some([0.12, 0.56, 1.0, 1.0]),
        _ => None,
    }
}

/// CSS Color 5 `color-mix(in srgb|oklch, A P%, B)`.
fn parse_color_mix(input: &str) -> Option<[f32; 4]> {
    parse_color_mix_paint(input).map(CssPaintColor::to_srgb)
}

/// Parse a CSS Color 5 mix while retaining the requested interpolation space.
/// Direct OKLCH/HSV operands stay in that space, so high-chroma values are not
/// clipped by an intermediate sRGB conversion before interpolation.
fn parse_color_mix_paint(input: &str) -> Option<CssPaintColor> {
    let s = input.trim();
    let lower = s.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("color-mix(")?
        .trim_end()
        .strip_suffix(')')?;
    let (space, colors) = rest.split_once(',')?;
    let space = space.trim();
    if !space.starts_with("in ") {
        return None;
    }
    let (left, right) = split_top_level_comma_pair(colors.trim())?;
    let (color_a, pct_a) = split_color_and_optional_percent(left.trim())?;
    let (color_b, pct_b) = split_color_and_optional_percent(right.trim())?;
    let a = parse_css_paint_color(color_a)?;
    let b = parse_css_paint_color(color_b)?;
    let (weight_a, weight_b) = match (pct_a, pct_b) {
        (Some(a), Some(b)) => (a.max(0.0), b.max(0.0)),
        (Some(a), None) => (a.max(0.0), (100.0 - a).max(0.0)),
        (None, Some(b)) => ((100.0 - b).max(0.0), b.max(0.0)),
        (None, None) => (50.0, 50.0),
    };
    let supplied = weight_a + weight_b;
    let total = supplied.max(f32::EPSILON);
    let t = (weight_a / total).clamp(0.0, 1.0);
    // CSS Color 5 makes an under-specified mix translucent by the missing
    // weight; weights above 100% are normalized without further attenuation.
    let alpha_scale = (supplied / 100.0).min(1.0);
    if space.eq_ignore_ascii_case("in oklch") {
        let a = paint_to_oklch(a);
        let b = paint_to_oklch(b);
        return Some(CssPaintColor::Oklch {
            l: a.0 * t + b.0 * (1.0 - t),
            c: a.1 * t + b.1 * (1.0 - t),
            h: interpolate_hue(a.2, b.2, a.1, b.1, t),
            alpha: (a.3 * t + b.3 * (1.0 - t)) * alpha_scale,
        });
    }
    if space.eq_ignore_ascii_case("in hsv") {
        let a = paint_to_hsv(a);
        let b = paint_to_hsv(b);
        // A zero-saturation HSV endpoint has no meaningful hue. Borrow the
        // other endpoint's hue while interpolating so a neutral-to-chromatic
        // mix does not take an arbitrary detour through red.
        let a_powerless = a.1.abs() < 1.0e-7;
        let b_powerless = b.1.abs() < 1.0e-7;
        let ha = if a_powerless { b.0 } else { a.0 };
        let hb = if b_powerless { a.0 } else { b.0 };
        let mut delta = hb - ha;
        if delta > 180.0 {
            delta -= 360.0;
        }
        if delta < -180.0 {
            delta += 360.0;
        }
        return Some(CssPaintColor::Hsv {
            h: wrap_hue(ha + delta * (1.0 - t)),
            s: a.1 * t + b.1 * (1.0 - t),
            v: a.2 * t + b.2 * (1.0 - t),
            alpha: (a.3 * t + b.3 * (1.0 - t)) * alpha_scale,
        });
    }
    if !space.eq_ignore_ascii_case("in srgb") {
        return None;
    }
    let a = a.to_srgb();
    let b = b.to_srgb();
    Some(CssPaintColor::Srgb([
        a[0] * t + b[0] * (1.0 - t),
        a[1] * t + b[1] * (1.0 - t),
        a[2] * t + b[2] * (1.0 - t),
        (a[3] * t + b[3] * (1.0 - t)) * alpha_scale,
    ]))
}

fn paint_to_oklch(color: CssPaintColor) -> (f32, f32, Option<f32>, f32) {
    match color {
        CssPaintColor::Oklch { l, c, h, alpha } => (l, c, h, alpha),
        CssPaintColor::LinearScRgb { channels, alpha } => linear_sc_rgb_to_oklch(channels, alpha),
        other => srgb_to_oklch(other.to_srgb()),
    }
}

fn paint_to_hsv(color: CssPaintColor) -> (f32, f32, f32, f32) {
    match color {
        CssPaintColor::Hsv { h, s, v, alpha } => (h, s, v, alpha),
        other => srgb_to_hsv(other.to_srgb()),
    }
}

fn parse_linear_scrgb(input: &str) -> Option<CssPaintColor> {
    let lower = input.trim().to_ascii_lowercase();
    let prefix = if lower.starts_with("color(srgb-linear ") {
        "color(srgb-linear "
    } else {
        "color(scrgb "
    };
    let body = lower.strip_prefix(prefix)?.strip_suffix(')')?;
    let (channels, alpha) = if let Some((lhs, rhs)) = body.split_once('/') {
        (lhs.trim(), parse_oklch_alpha(rhs.trim())?)
    } else {
        (body.trim(), 1.0)
    };
    let parts: Vec<_> = channels.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let mut values = [0.0; 3];
    for (slot, raw) in values.iter_mut().zip(parts) {
        *slot = if let Some(percent) = raw.strip_suffix('%') {
            parse_finite_f32(percent)? / 100.0
        } else {
            parse_finite_f32(raw)?
        };
    }
    Some(CssPaintColor::LinearScRgb {
        channels: values,
        alpha: alpha.clamp(0.0, 1.0),
    })
}

fn split_color_and_optional_percent(input: &str) -> Option<(&str, Option<f32>)> {
    let s = input.trim();
    // Prefer trailing `N%` after the color.
    if let Some((color, pct_raw)) = s.rsplit_once(' ')
        && let Some(percent) = pct_raw.trim().strip_suffix('%')
        && let Some(percent) = parse_finite_f32(percent)
    {
        return Some((color.trim(), Some(percent)));
    }
    Some((s, None))
}

/// CSS Color 5 `light-dark(light, dark)` — pick by active document theme.
fn parse_light_dark_color(input: &str) -> Option<[f32; 4]> {
    let s = input.trim();
    // CSS function names are ASCII case-insensitive. Lowercasing the small
    // function expression also keeps nested color parsing on the same path
    // as `parse_css_paint_color`, which already normalizes function names.
    let lower = s.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("light-dark(")?
        .trim_end()
        .strip_suffix(')')?;
    let (light, dark) = split_top_level_comma_pair(rest)?;
    let prefer_dark = crate::css_map::active_color_scheme_is_dark();
    let chosen = if prefer_dark { dark } else { light };
    // Recurse so nested hex/rgb/oklch still parse (avoid re-entering light-dark).
    let chosen = chosen.trim();
    if chosen.to_ascii_lowercase().starts_with("light-dark(") {
        return None;
    }
    parse_css_color(chosen)
}

/// Split `a, b` on the first top-level comma (paren-depth aware).
fn split_top_level_comma_pair(input: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, ch) in input.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                let (a, b) = input.split_at(i);
                return Some((a.trim(), b[1..].trim()));
            }
            _ => {}
        }
    }
    None
}

fn parse_oklch_lightness(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if let Some(p) = s.strip_suffix('%') {
        return Some(parse_finite_f32(p)? / 100.0);
    }
    let v = parse_finite_f32(s)?;
    // CSS allows L as 0..1 or 0..100 without `%` in some serializations.
    Some(if v > 1.0 { v / 100.0 } else { v })
}

fn parse_oklch_alpha(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if let Some(p) = s.strip_suffix('%') {
        return Some(parse_finite_f32(p)? / 100.0);
    }
    // `var(--lilia-alpha-hover)` left unresolved — refuse.
    if s.contains("var(") {
        return None;
    }
    parse_finite_f32(s)
}

fn parse_oklch_chroma(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(p) = s.strip_suffix('%') {
        return Some(parse_finite_f32(p)? * 0.4 / 100.0);
    }
    Some(parse_finite_f32(s)?.max(0.0))
}

fn parse_unit_interval(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if let Some(p) = s.strip_suffix('%') {
        return Some((parse_finite_f32(p)? / 100.0).clamp(0.0, 1.0));
    }
    Some(parse_finite_f32(s)?.clamp(0.0, 1.0))
}

fn parse_oklch_hue(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("none") {
        return None;
    }
    let (value, scale) = if let Some(v) = s.strip_suffix("deg") {
        (v, 1.0)
    } else if let Some(v) = s.strip_suffix("grad") {
        (v, 0.9)
    } else if let Some(v) = s.strip_suffix("rad") {
        (v, 180.0 / std::f32::consts::PI)
    } else if let Some(v) = s.strip_suffix("turn") {
        (v, 360.0)
    } else {
        (s, 1.0)
    };
    Some(wrap_hue(parse_finite_f32(value)? * scale))
}

fn parse_finite_f32(raw: &str) -> Option<f32> {
    let value = raw.trim().parse::<f32>().ok()?;
    value.is_finite().then_some(value)
}

fn wrap_hue(h: f32) -> f32 {
    let h = h % 360.0;
    if h < 0.0 { h + 360.0 } else { h }
}

fn srgb_to_hsv([r, g, b, a]: [f32; 4]) -> (f32, f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-7 {
        0.0
    } else if (max - r).abs() < 1e-7 {
        wrap_hue(60.0 * ((g - b) / d))
    } else if (max - g).abs() < 1e-7 {
        wrap_hue(60.0 * ((b - r) / d + 2.0))
    } else {
        wrap_hue(60.0 * ((r - g) / d + 4.0))
    };
    (h, if max < 1e-7 { 0.0 } else { d / max }, max, a)
}

fn srgb_to_oklch(c: [f32; 4]) -> (f32, f32, Option<f32>, f32) {
    let (linear, alpha) = nana_ui_core::PaintColor::Srgb { rgba: c }.to_linear_sc_rgb();
    linear_sc_rgb_to_oklch(linear, alpha)
}

/// Convert linear scRGB directly into OKLCH without first encoding and
/// clamping to display-referred sRGB. This matters for `color-mix(in oklch)`:
/// CSS permits out-of-gamut intermediate values and only performs gamut
/// mapping when a final device space is requested.
// These constants are the published OKLab conversion matrix. Keep their full
// precision so CSS color interpolation remains stable across color spaces.
#[allow(clippy::excessive_precision)]
fn linear_sc_rgb_to_oklch([r, g, b]: [f32; 3], alpha: f32) -> (f32, f32, Option<f32>, f32) {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    let l0 = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    let chroma = (a * a + b * b).sqrt();
    let hue = if chroma < 1e-7 {
        None
    } else {
        Some(wrap_hue(b.atan2(a).to_degrees()))
    };
    (l0, chroma, hue, alpha)
}

fn interpolate_hue(a: Option<f32>, b: Option<f32>, ca: f32, cb: f32, t: f32) -> Option<f32> {
    let a_powerless = ca.abs() < 1e-7;
    let b_powerless = cb.abs() < 1e-7;
    if a_powerless && b_powerless {
        return None;
    }
    // A powerless OKLCH hue inherits the other endpoint's hue for the whole
    // interpolation. If both endpoints are powerless, the result has no hue.
    let (ha, hb) = (
        if a_powerless { b } else { a }.unwrap_or(0.0),
        if b_powerless { a } else { b }.unwrap_or(0.0),
    );
    let mut delta = hb - ha;
    if delta > 180.0 {
        delta -= 360.0;
    }
    if delta < -180.0 {
        delta += 360.0;
    }
    Some(wrap_hue(ha + delta * (1.0 - t)))
}

fn parse_hex_color(hex: &str) -> Option<[f32; 4]> {
    let h = hex.trim();
    match h.len() {
        3 => {
            let r = u8::from_str_radix(&h[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&h[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&h[2..3].repeat(2), 16).ok()?;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        6 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        8 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            let a = u8::from_str_radix(&h[6..8], 16).ok()?;
            Some([
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ])
        }
        _ => None,
    }
}

fn parse_rgb_channel(s: &str) -> Option<f32> {
    if let Some(p) = s.strip_suffix('%') {
        return Some((parse_finite_f32(p)? / 100.0).clamp(0.0, 1.0));
    }
    Some((parse_finite_f32(s)? / 255.0).clamp(0.0, 1.0))
}

/// Map a CSS color **token name** (not `#hex`) onto the active [`SemanticPalette`].
///
/// Returns `None` for arbitrary paint values so they cannot invent ThemeTokens.
pub fn map_css_color_for_tokens(
    raw: &str,
    mode: ThemeAppearance,
) -> Option<(SemanticColorRole, SemanticColor)> {
    let role = SemanticColorRole::from_css_token_name(raw)?;
    // The built-in theme for `mode`, not a bare palette: the soft warning and
    // danger roles resolve through its state-layer alphas. An L1 adapter maps a
    // CSS token name onto a role; it does not get to pick the alpha.
    let color = nana_ui_core::builtin_theme(mode).style_model().color(role);
    Some((role, color))
}

/// Whether `raw` is an arbitrary CSS paint value that must **not** enter ThemeTokens.
pub fn is_non_token_css_color(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() {
        return false;
    }
    if SemanticColorRole::from_css_token_name(s).is_some() {
        return false;
    }
    s.starts_with('#')
        || s.to_ascii_lowercase().starts_with("rgb(")
        || s.to_ascii_lowercase().starts_with("rgba(")
        || s.to_ascii_lowercase().starts_with("hsl(")
        || s.to_ascii_lowercase().starts_with("oklch(")
        || s.to_ascii_lowercase().starts_with("hsv(")
        || s.to_ascii_lowercase().starts_with("color(srgb-linear ")
        || s.to_ascii_lowercase().starts_with("color(scrgb ")
        || s.to_ascii_lowercase().starts_with("color-mix(")
        || s.to_ascii_lowercase().starts_with("light-dark(")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_and_rgb_colors() {
        assert_eq!(parse_css_color("#ff0000"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            parse_css_color("rgb(0, 128, 255)"),
            Some([0.0, 128.0 / 255.0, 1.0, 1.0])
        );
    }

    #[test]
    fn parses_named_css_colors() {
        assert_eq!(parse_css_color("yellow"), Some([1.0, 1.0, 0.0, 1.0]));
        assert_eq!(
            parse_css_color("orange"),
            Some([1.0, 165.0 / 255.0, 0.0, 1.0])
        );
        assert_eq!(parse_css_color("transparent"), Some([0.0, 0.0, 0.0, 0.0]));
        assert_eq!(parse_css_color("purple"), Some([0.5, 0.0, 0.5, 1.0]));
        assert_eq!(parse_css_color("gray"), Some([0.5, 0.5, 0.5, 1.0]));
        assert_eq!(parse_css_color("grey"), Some([0.5, 0.5, 0.5, 1.0]));
        assert_eq!(parse_css_color("aqua"), Some([0.0, 1.0, 1.0, 1.0]));
        assert_eq!(parse_css_color("fuchsia"), Some([1.0, 0.0, 1.0, 1.0]));
        assert_eq!(parse_css_color("lime"), Some([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(parse_css_color("maroon"), Some([0.5, 0.0, 0.0, 1.0]));
        assert_eq!(parse_css_color("Navy"), Some([0.0, 0.0, 0.5, 1.0]));
        assert_eq!(parse_css_color("olive"), Some([0.5, 0.5, 0.0, 1.0]));
        assert_eq!(
            parse_css_color("SILVER"),
            Some([192.0 / 255.0, 192.0 / 255.0, 192.0 / 255.0, 1.0])
        );
        assert_eq!(parse_css_color("teal"), Some([0.0, 0.5, 0.5, 1.0]));
    }

    #[test]
    fn parses_light_dark_prefers_light_by_default() {
        let c = parse_css_color("light-dark(#eff2f5, #151b23)").unwrap();
        assert!((c[0] - 0xef as f32 / 255.0).abs() < 0.01);
        assert!((c[1] - 0xf2 as f32 / 255.0).abs() < 0.01);
        assert!((c[2] - 0xf5 as f32 / 255.0).abs() < 0.01);
        let dark = crate::css_map::with_active_color_scheme_dark(true, || {
            parse_css_color("light-dark(#eff2f5, #151b23)").unwrap()
        });
        assert!((dark[0] - 0x15 as f32 / 255.0).abs() < 0.01);
        assert!(parse_css_color("LiGhT-DaRk(#fff, #000)").is_some());
    }

    #[test]
    fn parses_color_mix_in_oklch() {
        let c = parse_css_color("color-mix(in oklch, #ffffff 50%, #000000)").unwrap();
        // L=.5 is encoded from linear scRGB, yielding roughly 0.389 sRGB.
        assert!((c[0] - 0.389).abs() < 0.02);
        assert!((c[1] - 0.389).abs() < 0.02);
        assert!((c[2] - 0.389).abs() < 0.02);
        let accentish = parse_css_color("color-mix(in oklch, #61a8fa 28%, #1c1c1c)").unwrap();
        // Should be distinct from both endpoints.
        assert!(accentish[2] > 0.2 && accentish[2] < 0.9);
        let implicit = parse_css_color("color-mix(in oklch, red, blue)").unwrap();
        assert!((implicit[0] - implicit[2]).abs() < 0.08);
        let sparse = parse_css_paint_color("color-mix(in oklch, red 20%, blue 20%)").unwrap();
        assert!((sparse.to_srgb()[3] - 0.4).abs() < 1.0e-6);
    }

    #[test]
    fn parses_achromatic_oklch_tokens() {
        let light = parse_css_color("oklch(100% 0 89.9)").unwrap();
        assert!((light[0] - 1.0).abs() < 0.01);
        assert!((light[3] - 1.0).abs() < f32::EPSILON);
        let dark = parse_css_color("oklch(20.9% 0 89.9)").unwrap();
        // Neutral OKLab lightness is converted through linear scRGB, then sRGB
        // transfer encoding (it is not an sRGB-channel lightness).
        assert!((dark[0] - 0.094).abs() < 0.01);
        let translucent = parse_css_color("oklch(100% 0 89.9 / 0.06)").unwrap();
        assert!((translucent[3] - 0.06).abs() < 0.001);
    }

    #[test]
    fn parses_chromatic_oklch_with_angle_units() {
        let c = parse_css_color("oklch(62.8% 0.2577 29.23 / 50%)").unwrap();
        // CSS Color 4's canonical red sample, within the sRGB gamut tolerance.
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!(c[1] < 0.35 && c[2] < 0.25);
        assert!((c[3] - 0.5).abs() < 0.001);
        let turns = parse_css_color("oklch(62.8% 0.2577 0.081194turn)").unwrap();
        for i in 0..3 {
            assert!((turns[i] - c[i]).abs() < 0.01);
        }
    }

    #[test]
    fn authoring_oklch_keeps_extended_linear_sc_rgb() {
        let vivid = parse_css_paint_color("oklch(70% 0.5 40)").unwrap();
        let (linear, alpha) = vivid.to_linear_sc_rgb();
        assert_eq!(alpha, 1.0);
        assert!(linear.iter().any(|v| *v < 0.0));
        assert!(linear.iter().any(|v| *v > 1.0));
        assert!(matches!(vivid, CssPaintColor::Oklch { .. }));
        assert!(matches!(
            vivid.to_core(),
            nana_ui_core::PaintColor::Oklch { .. }
        ));
    }

    #[test]
    fn parses_linear_scrgb_without_clamping_authoring_channels() {
        let paint = parse_css_paint_color("color(srgb-linear 1.25 -0.1 0.5 / 25%)").unwrap();
        match paint {
            CssPaintColor::LinearScRgb { channels, alpha } => {
                assert!((channels[0] - 1.25).abs() < 1e-6);
                assert!((channels[1] + 0.1).abs() < 1e-6);
                assert!((channels[2] - 0.5).abs() < 1e-6);
                assert!((alpha - 0.25).abs() < 1e-6);
            }
            other => panic!("expected linear scRGB, got {other:?}"),
        }
        assert!(is_non_token_css_color("color(srgb-linear 0.2 0.3 0.4)"));
    }

    #[test]
    fn oklch_mix_converts_linear_scrgb_without_display_clamping() {
        let extended = parse_css_paint_color(
            "color-mix(in oklch, color(srgb-linear 1.5 0 0) 50%, color(srgb-linear 0 0 0))",
        )
        .unwrap();
        let clipped = parse_css_paint_color(
            "color-mix(in oklch, color(srgb-linear 1 0 0) 50%, color(srgb-linear 0 0 0))",
        )
        .unwrap();
        let (extended_linear, _) = extended.to_linear_sc_rgb();
        let (clipped_linear, _) = clipped.to_linear_sc_rgb();
        assert!((extended_linear[0] - clipped_linear[0]).abs() > 0.02);
    }

    #[test]
    fn malformed_explicit_color_alpha_fails_closed() {
        assert!(parse_css_paint_color("oklch(60% .1 30 / nope)").is_none());
        assert!(parse_css_paint_color("hsv(30 50% 50% / nope)").is_none());
        assert!(parse_css_paint_color("color(srgb-linear .2 .3 .4 / nope)").is_none());
    }

    #[test]
    fn css_var_and_light_dark_keep_oklch_paint_resolvable() {
        let vars =
            crate::css_map::collect_css_custom_properties(":root { --accent: oklch(65% .2 30); }");
        crate::css_map::with_active_css_vars(&vars, || {
            let resolved = crate::css_map::resolve_paint_color("var(--accent)").unwrap();
            assert!(resolved[0] > resolved[1]);
            assert!(matches!(
                resolve_css_paint_color("var(--accent)"),
                Some(CssPaintColor::Oklch { .. })
            ));
        });
        let dark = crate::css_map::with_active_color_scheme_dark(true, || {
            parse_css_color("light-dark(oklch(70% .2 30), oklch(35% .2 250))").unwrap()
        });
        assert!(dark[2] > dark[0]);
    }

    #[test]
    fn parses_hsv_and_hsv_mix() {
        let red = parse_css_paint_color("hsv(0deg 100% 100% / 50%)").unwrap();
        assert!(matches!(red, CssPaintColor::Hsv { .. }));
        let rgba = red.to_srgb();
        assert!((rgba[0] - 1.0).abs() < 0.001 && rgba[1] < 0.001);
        assert!((rgba[3] - 0.5).abs() < 0.001);
        let yellow = parse_css_color("color-mix(in hsv, red 50%, blue)").unwrap();
        assert!((yellow[0] - yellow[2]).abs() < 0.01);
        assert!(is_non_token_css_color("hsv(120 50% 50%)"));
    }

    #[test]
    fn oklch_mix_uses_shortest_hue_and_powerless_hue() {
        let wrapped =
            parse_css_color("color-mix(in oklch, oklch(60% .2 350) 50%, oklch(60% .2 10))")
                .unwrap();
        let direct = parse_css_color("oklch(60% .2 0)").unwrap();
        for i in 0..3 {
            assert!((wrapped[i] - direct[i]).abs() < 0.02);
        }
        let neutral =
            parse_css_color("color-mix(in oklch, oklch(60% 0 none) 50%, oklch(60% .2 120))")
                .unwrap();
        let inherited = parse_css_color("oklch(60% .1 120)").unwrap();
        assert!((neutral[0] - inherited[0]).abs() < 0.08);
        let both_neutral =
            parse_css_paint_color("color-mix(in oklch, oklch(60% 0 20) 50%, oklch(60% 0 220))")
                .unwrap();
        assert!(matches!(both_neutral, CssPaintColor::Oklch { h: None, .. }));
    }

    #[test]
    fn hex_is_non_token_accent_token_maps() {
        assert!(is_non_token_css_color("#e74c3c"));
        assert!(is_non_token_css_color("rgb(1, 2, 3)"));
        assert!(!is_non_token_css_color("accent"));
        let (role, color) = map_css_color_for_tokens("accent", ThemeAppearance::Light).unwrap();
        assert_eq!(role, SemanticColorRole::Accent);
        assert_eq!(color, nana_ui_core::SemanticPalette::light().accent);
        assert!(map_css_color_for_tokens("#e74c3c", ThemeAppearance::Light).is_none());
    }
}
