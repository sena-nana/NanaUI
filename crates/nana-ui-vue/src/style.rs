//! L1 树文档用：inline `style` / `class` → 合成盒 [`StyleIntent`]（**仅**诊断/hit-test）。
//!
//! Used by [`crate::NanaTreeDocument::resolve_now`] to size stack/row boxes.
//! Product UI layout for Iced goes through [`crate::css_map::LayoutStyle`] on
//! [`crate::WidgetProps`] → [`crate::iced_app`] (feature `iced-view`) — that is the
//! Style Model **Layout** path. This module is **not** a paint engine, not a CSS
//! cascade, and must not invent formal ThemeTokens from arbitrary CSS colors.
//!
//! ## SoT / 停扩 / 禁止第二套解析
//!
//! | 路径 | 角色 |
//! |------|------|
//! | iced `LayoutProbe` → `LayoutBoxStore` | 产品几何权威（paint 后） |
//! | [`crate::measure_layout`] | 预绘制回退 + css-parity |
//! | 本模块 + `resolve_now` | 合成 hit-test / 诊断盒 |
//!
//! **禁止**在此新增第二套声明解析（length / padding / opacity / class→几何）。
//! Inline 声明只经 [`LayoutStyleCss::apply_css_text`] → 投影 [`StyleIntent`]。
//! 产品 class 几何合同在 [`crate::shell_contract`]；勿在本模块镜像 pad/min_h/row。
//! 几何三轨（iced / measure / 合成）**勿强合**——本路径可随 iced 盒覆盖退役。
//! 新属性只进 `css_map`。色值 [`parse_css_color`] 仍留本模块（供
//! [`crate::css_map::resolve_paint_color`] 复用），但不走 ThemeTokens。
//!
//! ## L1 色值策略
//!
//! | 来源 | 行为 |
//! |------|------|
//! | 已知 token / class（`accent`、`muted`、`var(--nana-*)`） | → [`SemanticColorRole`](nana_ui_core::SemanticColorRole) / 调色板字段 |
//! | 未知 `#hex` / `rgb()` | **不**写入正式 ThemeTokens；可留在 [`StyleIntent`] 作诊断盒，或忽略 |
//!
//! [`map_css_color_for_tokens`] 是正式 Tokens 路径的唯一入口；[`parse_css_color`]
//! 仅服务诊断合成盒。

use nana_ui_core::{SemanticColor, SemanticColorRole, SemanticPalette, ThemeMode};

use crate::css_map::{FlexDirection, LayoutStyle, LayoutStyleCss};
use crate::tree::{LayoutBox, NanaTreeDocument, NodeHandle};

/// Parsed subset of CSS used for synthetic layout sizing.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleIntent {
    pub background: Option<[f32; 4]>,
    pub color: Option<[f32; 4]>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Soft minimum — grows with children, unlike fixed [`Self::height`].
    pub min_height: Option<f32>,
    pub padding: f32,
    pub margin_top: f32,
    pub font_size: f32,
    pub opacity: f32,
    pub hidden: bool,
    /// Lay element children left-to-right instead of stacking.
    pub row: bool,
    pub gap: f32,
    pub class_names: Vec<String>,
}

impl Default for StyleIntent {
    fn default() -> Self {
        Self {
            background: None,
            color: None,
            width: None,
            height: None,
            min_height: None,
            padding: 0.0,
            margin_top: 0.0,
            font_size: 14.0,
            opacity: 1.0,
            hidden: false,
            row: false,
            gap: 0.0,
            class_names: Vec::new(),
        }
    }
}

/// Parse `style` + `class` (+ `hidden` / region attrs / tag) into a simplified intent.
pub fn parse_style_intent(doc: &NanaTreeDocument, handle: NodeHandle) -> StyleIntent {
    let mut intent = StyleIntent::default();
    let mode = theme_mode_from_doc(doc);
    if doc.get_attribute(handle, "hidden").is_some() {
        intent.hidden = true;
    }
    if let Some(class) = doc.get_attribute(handle, "class") {
        intent.class_names = class
            .split_whitespace()
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .collect();
        apply_class_presets(&mut intent, mode);
    }
    if let Some(tag) = doc.element_tag(handle) {
        apply_tag_presets(&mut intent, &tag, mode);
    }
    if let Some(style) = doc.get_attribute(handle, "style") {
        apply_style_declarations(
            &mut intent,
            &style,
            doc.logical_width(),
            doc.logical_height(),
        );
    }
    intent
}

fn theme_mode_from_doc(doc: &NanaTreeDocument) -> ThemeMode {
    if doc.document_theme().eq_ignore_ascii_case("dark") {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
}

/// Apply style-driven sizing on top of the default stack layout for one box.
pub fn apply_style_to_box(box_: &mut LayoutBox, intent: &StyleIntent, viewport_w: f32) {
    if let Some(w) = intent.width {
        box_.width = w.clamp(0.0, viewport_w.max(1.0));
    }
    if let Some(h) = intent.height {
        box_.height = h.max(0.0);
    } else {
        if let Some(mh) = intent.min_height {
            box_.height = box_.height.max(mh);
        } else if intent.font_size > 0.0 && intent.background.is_some() {
            // Give styled containers a readable default height.
            box_.height = box_
                .height
                .max(intent.font_size + intent.padding * 2.0 + 8.0);
        }
        if intent.padding > 0.0 {
            box_.height = box_.height.max(intent.font_size + intent.padding * 2.0);
        }
    }
}

fn apply_class_presets(intent: &mut StyleIntent, mode: ThemeMode) {
    // Diagnostic semantic-color aliases only. Shell geometry (`card` / titlebar /
    // sidebar / `nana-root-paint` pad·min_h·row) lives in `shell_contract` →
    // `LayoutStyle` — do not mirror here (drift risk; stop expanding).
    let palette = SemanticPalette::for_mode(mode);
    let accent = palette.accent.as_rgba_array();
    let accent_text = palette.accent_text.as_rgba_array();
    let muted = palette.muted.as_rgba_array();

    for name in intent.class_names.clone() {
        match name.as_str() {
            "nana-custom-accent" | "accent" | "primary" => {
                ensure_bg(intent, accent);
                ensure_color(intent, accent_text);
            }
            "nana-custom-muted" | "muted" | "ghost" => {
                ensure_color(intent, muted);
            }
            _ => {}
        }
    }
}

fn apply_tag_presets(intent: &mut StyleIntent, tag: &str, mode: ThemeMode) {
    // Diagnostic paint for a few host tags only. Do not invent shell widths /
    // paddings / row-gap here — product path uses `widget_map` + `shell_contract`.
    let palette = SemanticPalette::for_mode(mode);
    let muted = palette.muted.as_rgba_array();
    let accent = palette.accent.as_rgba_array();
    let accent_text = palette.accent_text.as_rgba_array();

    match tag.to_ascii_lowercase().as_str() {
        "button" | "nana-button" => {
            ensure_bg(intent, accent);
            ensure_color(intent, accent_text);
        }
        "nana-text" | "nana-label" => {
            ensure_color(intent, muted);
        }
        _ => {}
    }
}

fn ensure_bg(intent: &mut StyleIntent, color: [f32; 4]) {
    if intent.background.is_none() {
        intent.background = Some(color);
    }
}

fn ensure_color(intent: &mut StyleIntent, color: [f32; 4]) {
    if intent.color.is_none() {
        intent.color = Some(color);
    }
}

fn ensure_min_h(intent: &mut StyleIntent, h: f32) {
    let next = intent.min_height.map(|v| v.max(h)).unwrap_or(h);
    intent.min_height = Some(next);
}

/// Project already-parsed [`LayoutStyle`] sizing/paint onto a diagnostic intent.
///
/// Shared box resolution uses `nana-ui-core::box_layout` helpers on `LayoutStyle`
/// (`resolve_px` / `resolved_padding_against` / …) — do not re-parse lengths here.
fn project_layout_style_onto_intent(
    intent: &mut StyleIntent,
    layout: &LayoutStyle,
    percent_w: f32,
    percent_h: f32,
) {
    if let Some(c) = layout.background {
        intent.background = Some(c);
    }
    if let Some(c) = layout.color {
        intent.color = Some(c);
    }
    if let Some(v) = layout
        .width
        .and_then(|spec| resolve_intent_length(spec, Some(percent_w)))
    {
        intent.width = Some(v);
    }
    if let Some(v) = layout
        .height
        .and_then(|spec| resolve_intent_length(spec, Some(percent_h)))
    {
        intent.height = Some(v);
    }
    if let Some(v) = layout
        .min_height
        .and_then(|spec| resolve_intent_length(spec, Some(percent_h)))
    {
        ensure_min_h(intent, v);
    }
    let pad = layout.resolved_padding_against(Some(percent_w));
    let pad_uniform = pad.top.max(pad.right).max(pad.bottom).max(pad.left);
    if pad_uniform > 0.0 {
        intent.padding = pad_uniform;
    }
    let margin = layout.resolved_margin_against(Some(percent_w));
    if margin.top != 0.0 || layout.margin_top.is_some() || layout.margin.is_some() {
        intent.margin_top = margin.top;
    }
    if let Some(fs) = layout.font_size {
        intent.font_size = fs.max(8.0);
    }
    let gap = layout.resolved_row_gap_against(Some(percent_w));
    if gap > 0.0 || layout.gap.is_some() || layout.row_gap.is_some() {
        intent.gap = gap.max(0.0);
    }
    if layout.hidden {
        intent.hidden = true;
    }
    if let Some(op) = layout.opacity {
        intent.opacity = op.clamp(0.0, 1.0);
    }
    match layout.direction {
        Some(FlexDirection::Row) => intent.row = true,
        Some(FlexDirection::Column) => intent.row = false,
        None => {}
    }
}

fn resolve_intent_length(
    spec: crate::css_map::LengthSpec,
    percent_base: Option<f32>,
) -> Option<f32> {
    match spec {
        crate::css_map::LengthSpec::Fill => percent_base,
        other => other.resolve_px(percent_base),
    }
}

/// Inline `style` → [`LayoutStyle`]（中立 parse）→ 投影 [`StyleIntent`]。
///
/// 含 `opacity`：经 `LayoutStyle` 一次扫描写入，禁止在此二次扫串。
fn apply_style_declarations(intent: &mut StyleIntent, style: &str, percent_w: f32, percent_h: f32) {
    let mut layout = LayoutStyle::default();
    layout.apply_css_text(style, Some(percent_w), Some(percent_h));
    project_layout_style_onto_intent(intent, &layout, percent_w, percent_h);
}

/// Parse `#rgb` / `#rrggbb` / `#rrggbbaa` / `rgb()` / `rgba()` / a few named colors.
///
/// **Diagnostic only** — feeds [`StyleIntent`], not formal ThemeTokens.
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
    if let Some((color, pct_raw)) = s.rsplit_once(' ') {
        let pct_raw = pct_raw.trim();
        if let Some(p) = pct_raw.strip_suffix('%') {
            if let Ok(pct) = p.parse::<f32>() {
                return Some((color.trim(), pct));
            }
        }
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
    if parts.len() < 1 {
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
    use crate::css_map::{LayoutStyle, LayoutStyleCss};
    use crate::tree::NanaTreeDocument;

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

    #[test]
    fn class_presets_use_semantic_palette_accent() {
        let mut doc = NanaTreeDocument::new(320, 180, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("div");
        doc.set_attribute(el, "class", "nana-custom-accent");
        doc.insert(el, root, None);
        doc.resolve_now();
        let intent = parse_style_intent(&doc, el);
        assert_eq!(
            intent.background,
            Some(SemanticPalette::light().accent.as_rgba_array())
        );
        assert_eq!(
            intent.color,
            Some(SemanticPalette::light().accent_text.as_rgba_array())
        );
    }

    #[test]
    fn tag_and_class_presets_follow_document_theme() {
        let mut doc = NanaTreeDocument::new(320, 180, 1.0);
        doc.set_document_theme("dark");
        let root = doc.mount_root();
        let btn = doc.create_element("button");
        doc.insert(btn, root, None);
        let accent = doc.create_element("div");
        doc.set_attribute(accent, "class", "nana-custom-accent");
        doc.insert(accent, root, None);
        doc.resolve_now();
        let btn_intent = parse_style_intent(&doc, btn);
        assert_eq!(
            btn_intent.background,
            Some(SemanticPalette::dark().accent.as_rgba_array())
        );
        assert_eq!(
            btn_intent.color,
            Some(SemanticPalette::dark().accent_text.as_rgba_array())
        );
        let accent_intent = parse_style_intent(&doc, accent);
        assert_eq!(
            accent_intent.background,
            Some(SemanticPalette::dark().accent.as_rgba_array())
        );
    }

    #[test]
    fn styled_div_gets_layout_box_from_intent() {
        let mut doc = NanaTreeDocument::new(320, 180, 1.0);
        let root = doc.mount_root();
        let card = doc.create_element("div");
        doc.set_attribute(
            card,
            "style",
            "background-color: #e74c3c; width: 200px; height: 60px; padding: 8px",
        );
        doc.insert(card, root, None);
        doc.resolve_now();
        let box_ = doc.layout_box(card).expect("layout");
        assert!((box_.width - 200.0).abs() < 0.5, "width={}", box_.width);
        assert!((box_.height - 60.0).abs() < 0.5, "height={}", box_.height);
        let intent = parse_style_intent(&doc, card);
        // Diagnostic StyleIntent may keep hex; Tokens path must not.
        assert_eq!(
            intent.background,
            Some([231.0 / 255.0, 76.0 / 255.0, 60.0 / 255.0, 1.0])
        );
        assert!(is_non_token_css_color("#e74c3c"));
    }

    #[test]
    fn vertical_margin_and_padding_percent_use_containing_block_width() {
        // Viewport 200×50 — height base would wrongly yield margin_top=5, padding=5.
        let mut doc = NanaTreeDocument::new(200, 50, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("div");
        doc.set_attribute(el, "style", "margin-top:10%;padding:10%;margin:10%");
        doc.insert(el, root, None);
        doc.resolve_now();
        let intent = parse_style_intent(&doc, el);
        assert!(
            (intent.margin_top - 20.0).abs() < 0.01,
            "margin_top={}",
            intent.margin_top
        );
        assert!(
            (intent.padding - 20.0).abs() < 0.01,
            "padding={}",
            intent.padding
        );
    }

    #[test]
    fn style_intent_resolves_lightweight_calc_width() {
        let mut doc = NanaTreeDocument::new(400, 120, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("div");
        doc.set_attribute(el, "style", "width:calc(40px + 50%);height:40px");
        doc.insert(el, root, None);
        doc.resolve_now();
        let intent = parse_style_intent(&doc, el);
        assert!(
            (intent.width.unwrap_or(0.0) - 240.0).abs() < 0.01,
            "width={:?}",
            intent.width
        );
    }

    #[test]
    fn opacity_projects_from_layout_style_without_second_scan() {
        let mut doc = NanaTreeDocument::new(200, 100, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("div");
        doc.set_attribute(el, "style", "opacity:0.4;width:80px;height:20px");
        doc.insert(el, root, None);
        doc.resolve_now();
        let intent = parse_style_intent(&doc, el);
        assert!((intent.opacity - 0.4).abs() < f32::EPSILON);
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("opacity:0.75", None, None);
        assert_eq!(layout.opacity, Some(0.75));
    }

    #[test]
    fn shell_overlapping_classes_do_not_invent_diagnostic_geometry() {
        // Product pad/min_h/row for these classes live in shell_contract.
        // StyleIntent must not re-mirror them (stop-expand / anti-drift).
        let mut doc = NanaTreeDocument::new(320, 180, 1.0);
        let root = doc.mount_root();
        for class in [
            "card",
            "nana-card",
            "titlebar",
            "nana-sidebar-frame",
            "nana-root-paint",
        ] {
            let el = doc.create_element("div");
            doc.set_attribute(el, "class", class);
            doc.insert(el, root, None);
            let intent = parse_style_intent(&doc, el);
            assert_eq!(intent.padding, 0.0, "class={class}");
            assert!(intent.min_height.is_none(), "class={class}");
            assert!(!intent.row, "class={class}");
            assert_eq!(intent.gap, 0.0, "class={class}");
            assert!(intent.background.is_none(), "class={class}");
        }
    }
}
