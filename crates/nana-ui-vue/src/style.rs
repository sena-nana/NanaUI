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

use nana_ui_core::{SemanticColor, SemanticColorRole, SemanticPalette, ThemeMode};

/// Parse `#rgb` / `#rrggbb` / `#rrggbbaa` / `rgb()` / `rgba()` / a few named colors.
///
/// **L1 paint only** — does not create formal ThemeTokens.
/// Prefer [`map_css_color_for_tokens`] on the Tokens path.
pub fn parse_css_color(input: &str) -> Option<[f32; 4]> {
    let s = input.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("transparent") {
        return Some([0.0, 0.0, 0.0, 0.0]);
    }
    if let Some(c) = parse_light_dark_color(s) {
        return Some(c);
    }
    if let Some(c) = parse_color_mix(s) {
        return Some(c);
    }
    match s.to_ascii_lowercase().as_str() {
        "white" => return Some([1.0, 1.0, 1.0, 1.0]),
        "black" => return Some([0.0, 0.0, 0.0, 1.0]),
        "red" => return Some([1.0, 0.0, 0.0, 1.0]),
        "green" => return Some([0.0, 0.5, 0.0, 1.0]),
        "blue" => return Some([0.0, 0.0, 1.0, 1.0]),
        "coral" => return Some([1.0, 0.5, 0.31, 1.0]),
        "dodgerblue" => return Some([0.12, 0.56, 1.0, 1.0]),
        _ => {}
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
            let a = parts[3].parse::<f32>().ok()?.clamp(0.0, 1.0);
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
    // LiliaUI `@lilia/theme` tokens are oklch(). MVP: achromatic (C≈0) → gray by L;
    // chromatic values approximate L as sRGB gray (better than dropping the paint).
    if let Some(c) = parse_oklch_color(&lower) {
        return Some(c);
    }
    None
}

/// CSS Color 5 `color-mix(in srgb|oklch, A P%, B)` — sRGB lerp (oklch uses same
/// lerp as an MVP so heatmap level ramps still resolve to distinct paints).
fn parse_color_mix(input: &str) -> Option<[f32; 4]> {
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
    // Split `A 28%, B` on the top-level comma between the two colors.
    let (left, right) = split_top_level_comma_pair(colors.trim())?;
    let (color_a, pct) = split_color_and_percent(left.trim())?;
    let color_b = right.trim();
    // Optional trailing percent on B is ignored (CSS allows `B 72%`; we derive).
    let color_b = color_b
        .rsplit_once(' ')
        .and_then(|(c, p)| {
            if p.trim().ends_with('%') {
                Some(c.trim())
            } else {
                None
            }
        })
        .unwrap_or(color_b);
    let a = parse_css_color(color_a)?;
    let b = parse_css_color(color_b)?;
    let t = (pct / 100.0).clamp(0.0, 1.0);
    // `color-mix(in …, A P%, B)` → P of A and (100-P) of B.
    Some([
        a[0] * t + b[0] * (1.0 - t),
        a[1] * t + b[1] * (1.0 - t),
        a[2] * t + b[2] * (1.0 - t),
        a[3] * t + b[3] * (1.0 - t),
    ])
}

fn split_color_and_percent(input: &str) -> Option<(&str, f32)> {
    let s = input.trim();
    // Prefer trailing `N%` after the color.
    if let Some((color, pct_raw)) = s.rsplit_once(' ')
        && let Some(percent) = pct_raw.trim().strip_suffix('%')
        && let Ok(percent) = percent.parse::<f32>()
    {
        return Some((color.trim(), percent));
    }
    None
}

/// CSS Color 5 `light-dark(light, dark)` — pick by active document theme.
fn parse_light_dark_color(input: &str) -> Option<[f32; 4]> {
    let s = input.trim();
    let rest = s
        .strip_prefix("light-dark(")
        .or_else(|| s.strip_prefix("LIGHT-DARK("))?
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

/// Minimal `oklch(L% C H)` / `oklch(L% C H / A)` → sRGB.
///
/// Neutral Lilia tokens use C=0; we map L% linearly to gray. Chromatic colors also
/// fall back to gray-by-L so `background: var(--bg)` still paints under light/dark.
fn parse_oklch_color(lower: &str) -> Option<[f32; 4]> {
    let rest = lower.strip_prefix("oklch(")?.strip_suffix(')')?;
    let (body, alpha) = if let Some((lhs, rhs)) = rest.split_once('/') {
        (lhs.trim(), parse_oklch_alpha(rhs.trim()).unwrap_or(1.0))
    } else {
        (rest.trim(), 1.0)
    };
    let parts: Vec<&str> = body.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let l = parse_oklch_lightness(parts[0])?;
    let gray = l.clamp(0.0, 1.0);
    Some([gray, gray, gray, alpha.clamp(0.0, 1.0)])
}

fn parse_oklch_lightness(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if let Some(p) = s.strip_suffix('%') {
        return Some(p.parse::<f32>().ok()? / 100.0);
    }
    let v = s.parse::<f32>().ok()?;
    // CSS allows L as 0..1 or 0..100 without `%` in some serializations.
    Some(if v > 1.0 { v / 100.0 } else { v })
}

fn parse_oklch_alpha(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if let Some(p) = s.strip_suffix('%') {
        return Some(p.parse::<f32>().ok()? / 100.0);
    }
    // `var(--lilia-alpha-hover)` left unresolved — refuse.
    if s.contains("var(") {
        return None;
    }
    s.parse::<f32>().ok()
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
        return Some((p.parse::<f32>().ok()? / 100.0).clamp(0.0, 1.0));
    }
    Some((s.parse::<f32>().ok()? / 255.0).clamp(0.0, 1.0))
}

/// Map a CSS color **token name** (not `#hex`) onto the active [`SemanticPalette`].
///
/// Returns `None` for arbitrary paint values so they cannot invent ThemeTokens.
pub fn map_css_color_for_tokens(
    raw: &str,
    mode: ThemeMode,
) -> Option<(SemanticColorRole, SemanticColor)> {
    let role = SemanticColorRole::from_css_token_name(raw)?;
    let color = SemanticPalette::for_mode(mode).get(role);
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
    fn parses_light_dark_prefers_light_by_default() {
        let c = parse_css_color("light-dark(#eff2f5, #151b23)").unwrap();
        assert!((c[0] - 0xef as f32 / 255.0).abs() < 0.01);
        assert!((c[1] - 0xf2 as f32 / 255.0).abs() < 0.01);
        assert!((c[2] - 0xf5 as f32 / 255.0).abs() < 0.01);
        let dark = crate::css_map::with_active_color_scheme_dark(true, || {
            parse_css_color("light-dark(#eff2f5, #151b23)").unwrap()
        });
        assert!((dark[0] - 0x15 as f32 / 255.0).abs() < 0.01);
    }

    #[test]
    fn parses_color_mix_srgb_lerp() {
        let c = parse_css_color("color-mix(in oklch, #ffffff 50%, #000000)").unwrap();
        assert!((c[0] - 0.5).abs() < 0.02);
        assert!((c[1] - 0.5).abs() < 0.02);
        assert!((c[2] - 0.5).abs() < 0.02);
        let accentish = parse_css_color("color-mix(in oklch, #61a8fa 28%, #1c1c1c)").unwrap();
        // Should be distinct from both endpoints.
        assert!(accentish[2] > 0.2 && accentish[2] < 0.9);
    }

    #[test]
    fn parses_achromatic_oklch_tokens() {
        let light = parse_css_color("oklch(100% 0 89.9)").unwrap();
        assert!((light[0] - 1.0).abs() < 0.01);
        assert!((light[3] - 1.0).abs() < f32::EPSILON);
        let dark = parse_css_color("oklch(20.9% 0 89.9)").unwrap();
        assert!((dark[0] - 0.209).abs() < 0.01);
        let translucent = parse_css_color("oklch(100% 0 89.9 / 0.06)").unwrap();
        assert!((translucent[3] - 0.06).abs() < 0.001);
    }

    #[test]
    fn hex_is_non_token_accent_token_maps() {
        assert!(is_non_token_css_color("#e74c3c"));
        assert!(is_non_token_css_color("rgb(1, 2, 3)"));
        assert!(!is_non_token_css_color("accent"));
        let (role, color) = map_css_color_for_tokens("accent", ThemeMode::Light).unwrap();
        assert_eq!(role, SemanticColorRole::Accent);
        assert_eq!(color, SemanticPalette::light().accent);
        assert!(map_css_color_for_tokens("#e74c3c", ThemeMode::Light).is_none());
    }
}
