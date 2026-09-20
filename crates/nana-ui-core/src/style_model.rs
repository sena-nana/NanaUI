//! Nana **Style Model** — the single styling contract shared by L1 / L2 / L3.
//!
//! ```text
//! L1 CSS 子集 ──► Nana Style Model（Tokens + Semantics + Layout）
//! L2 Vue props ─► 同一套 Model
//! L3 Rust API ──► 同一套 Model
//!                  ▼
//!            唯一绘制：Runtime / UiScene → SceneWgpuPainter
//! ```
//!
//! ## Parts
//!
//! | Part | Meaning | Lives today |
//! |------|---------|-------------|
//! | **Tokens** | Theme spacing / radius / control metrics; semantic palette roles | [`ThemeMetrics`](crate::ThemeMetrics), [`ThemeMode`](crate::ThemeMode), [`SemanticPalette`] |
//! | **Semantics** | Widget kind + control intent (`ButtonKind`, `ControlSize`, …) | [`crate::semantics`] |
//! | **Layout** | Flex/gap/padding/size intent | Workspace regions: [`crate::layout`]; box flex: [`crate::LayoutStyle`] / [`crate::LengthSpec`] / [`crate::ParentBox`]（CSS parse 在 `nana-ui-vue::css_map`） |
//!
//! ## Mapping rules (do not distort)
//!
//! - Theme color / spacing / radius **tiers** → Tokens / semantic palette roles
//! - Known classes (e.g. `nana-btn--primary`) → Semantics (`WidgetKind` + props), not new tokens
//! - flex / gap / padding / sizes → Layout (`LayoutStyle` …)
//! - Arbitrary business CSS color values must **not** invent formal token roles
//!
//! ## L1 color policy
//!
//! - Known token / class names → [`SemanticColorRole`] / [`SemanticPalette`] field
//! - Unknown `#hex` / `rgb()` → **do not** write into formal ThemeTokens; L1 may keep a
//!   restricted paint hint on the L1 bridge or drop it
//!
//! ## Restricted paint hints (no second paint path)
//!
//! Arbitrary business colors are **bridge diagnostics only**. They must not:
//! - invent new ThemeTokens / palette roles
//! - open a parallel paint pipeline beside L3 NanaUI widgets
//! - enter `nana-ui` public core as CSSOM / free-form color maps
//!
//! If a value cannot map to [`SemanticColorRole`], keep it as a restricted L1
//! paint hint or drop it. Formal appearance always goes through Tokens +
//! Semantics + Layout → L3.
//!
//! ## Non-goals
//!
//! - CSS parsing / CSSOM must not enter `nana-ui` or this crate
//! - L1 adapters may map a CSS **subset** into this model; that adapter stays in `nana-ui-vue`

use serde::{Deserialize, Serialize};

use crate::semantics::{ButtonKind, CardKind, ControlSize, StatusTone};
use crate::theme::tokens::{AccentRamp, OpacityTokens, StateLayer};
use crate::theme::{ThemeMetrics, ThemeMode, UI_BASE_TEXT_SIZE, UI_METRICS};

/// Backend-neutral RGBA in 0..=1.
///
/// RGBA conversion stays in `nana-ui` (adapter layer). Arbitrary CSS
/// hex from L1 must map to an existing [`SemanticPalette`] role or stay as a
/// one-off paint hint on the bridge — never as a new formal token.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SemanticColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl SemanticColor {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }

    pub const fn rgba8(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a,
        }
    }

    pub const fn as_rgba_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn to_u8_rgba(self) -> (u8, u8, u8, u8) {
        (
            (self.r * 255.0 + 0.5) as u8,
            (self.g * 255.0 + 0.5) as u8,
            (self.b * 255.0 + 0.5) as u8,
            (self.a * 255.0 + 0.5) as u8,
        )
    }
}

/// Named roles inside [`SemanticPalette`] (Tokens slice).
///
/// L1 may map known CSS token / class names onto these roles. Unknown hex
/// must not create new roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemanticColorRole {
    Background,
    Surface,
    Subtle,
    Hover,
    Active,
    Selected,
    SelectedHover,
    SelectedPressed,
    Border,
    BorderSoft,
    BorderStrong,
    Text,
    Muted,
    Faint,
    Accent,
    AccentStrong,
    AccentSoft,
    AccentSoftHover,
    AccentSoftPressed,
    AccentOnSoft,
    AccentText,
    Success,
    Warning,
    WarningSoft,
    WarningSoftHover,
    WarningSoftPressed,
    Danger,
    DangerSoftHover,
    DangerSoftPressed,
    /// Chrome strip. Defaults to Surface; backdrop can keep it opaque while
    /// sidebar Surface is translucent.
    Titlebar,
    // ---- 代码 token 角色（语义 overlay 通道；默认值见 `SemanticPalette::get`，
    // Muted/Faint 系保守起步、逐主题可覆盖）----
    /// 函数名（声明与调用点）。
    Function,
    /// 内置函数/语言预置名。
    Builtin,
    /// 类型名（struct 与类型别名并档同色）。
    Type,
    /// 变量绑定。
    Variable,
    /// 函数参数。
    Parameter,
    /// 常量声明。
    Const,
    /// 纹理绑定（一等产品概念）。
    Texture,
    /// 成员属性/字段访问。
    Property,
    /// 关键字。
    Keyword,
}

impl SemanticColorRole {
    /// Map a known CSS / design-token name (not `#hex`) onto a palette role.
    pub fn from_css_token_name(raw: &str) -> Option<Self> {
        let s = raw.trim().to_ascii_lowercase();
        let s = s
            .strip_prefix("var(")
            .and_then(|rest| rest.strip_suffix(')'))
            .map(str::trim)
            .unwrap_or(s.as_str());
        let s = s.strip_prefix("--nana-").unwrap_or(s);
        let s = s.strip_prefix("--").unwrap_or(s);
        Some(match s {
            "background" | "bg" => Self::Background,
            "surface" | "panel" => Self::Surface,
            "subtle" => Self::Subtle,
            "hover" => Self::Hover,
            "active" | "pressed" => Self::Active,
            "selected" => Self::Selected,
            "selected-hover" => Self::SelectedHover,
            "selected-pressed" => Self::SelectedPressed,
            "border" => Self::Border,
            "border-soft" => Self::BorderSoft,
            "border-strong" => Self::BorderStrong,
            "text" | "foreground" | "fg" => Self::Text,
            "muted" | "secondary" | "ghost" => Self::Muted,
            "faint" => Self::Faint,
            "accent" | "primary" | "nana-custom-accent" => Self::Accent,
            "accent-strong" => Self::AccentStrong,
            "accent-soft" => Self::AccentSoft,
            "accent-soft-hover" => Self::AccentSoftHover,
            "accent-soft-pressed" => Self::AccentSoftPressed,
            "accent-on-soft" => Self::AccentOnSoft,
            "accent-text" | "on-accent" => Self::AccentText,
            "success" => Self::Success,
            "warning" => Self::Warning,
            "warning-soft" => Self::WarningSoft,
            "warning-soft-hover" => Self::WarningSoftHover,
            "warning-soft-pressed" => Self::WarningSoftPressed,
            "danger" | "error" => Self::Danger,
            "danger-soft-hover" => Self::DangerSoftHover,
            "danger-soft-pressed" => Self::DangerSoftPressed,
            "titlebar" | "title-bar" => Self::Titlebar,
            _ => return None,
        })
    }
}

/// A theme-relative, premultiplied-alpha sRGB mix. Weights use basis points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SemanticColorMix {
    first: SemanticColorRole,
    second: Option<SemanticColorRole>,
    weight: u16,
}
impl SemanticColorMix {
    pub fn new(first: SemanticColorRole, second: SemanticColorRole, ratio: f32) -> Self {
        Self {
            first,
            second: Some(second),
            weight: Self::weight(ratio),
        }
    }
    /// Mix a palette role with transparent without fading descendants.
    pub fn alpha(role: SemanticColorRole, alpha: f32) -> Self {
        Self {
            first: role,
            second: None,
            weight: Self::weight(alpha),
        }
    }
    fn weight(ratio: f32) -> u16 {
        if ratio.is_finite() {
            (ratio.clamp(0.0, 1.0) * 10000.0).round() as u16
        } else {
            0
        }
    }
    pub fn resolve(self, model: StyleModelRef) -> SemanticColor {
        let first = model.color(self.first);
        let second = self
            .second
            .map(|role| model.color(role))
            .unwrap_or(SemanticColor::rgba(0.0, 0.0, 0.0, 0.0));
        let weight = f32::from(self.weight.min(10000)) / 10000.0;
        let a = first.a * weight + second.a * (1.0 - weight);
        if a <= 0.0 {
            return SemanticColor::rgba(0.0, 0.0, 0.0, 0.0);
        }
        let channel = |x: f32, y: f32| (x * first.a * weight + y * second.a * (1.0 - weight)) / a;
        SemanticColor::rgba(
            channel(first.r, second.r),
            channel(first.g, second.g),
            channel(first.b, second.b),
            a,
        )
    }
}

/// Semantic palette roles shared across backends.
///
/// Field set mirrors the Lilia hierarchy used by `nana-ui::theme::Colors`.
/// Concrete dark/light values live here; paint maps them through Tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SemanticPalette {
    pub background: SemanticColor,
    pub surface: SemanticColor,
    pub subtle: SemanticColor,
    pub hover: SemanticColor,
    pub active: SemanticColor,
    pub selected: SemanticColor,
    pub selected_hover: SemanticColor,
    pub selected_pressed: SemanticColor,
    pub border: SemanticColor,
    pub border_soft: SemanticColor,
    pub border_strong: SemanticColor,
    pub text: SemanticColor,
    pub muted: SemanticColor,
    pub faint: SemanticColor,
    pub accent: SemanticColor,
    pub accent_strong: SemanticColor,
    pub accent_soft: SemanticColor,
    pub accent_soft_hover: SemanticColor,
    pub accent_soft_pressed: SemanticColor,
    pub accent_on_soft: SemanticColor,
    pub accent_text: SemanticColor,
    pub success: SemanticColor,
    pub warning: SemanticColor,
    pub danger: SemanticColor,
}

/// The accent family the dark palette is built from. Naming it once is what
/// keeps `accent_soft` and friends from drifting off `accent`.
const ACCENT_DARK: AccentRamp = AccentRamp::DARK;
/// The light palette's accent family.
const ACCENT_LIGHT: AccentRamp = AccentRamp::LIGHT;

impl SemanticPalette {
    pub const fn dark() -> Self {
        Self {
            background: SemanticColor::rgb8(24, 24, 24),
            surface: SemanticColor::rgb8(32, 32, 32),
            subtle: SemanticColor::rgb8(28, 28, 28),
            hover: SemanticColor::rgb8(45, 45, 45),
            active: SemanticColor::rgb8(53, 53, 53),
            selected: SemanticColor::rgb8(53, 53, 53),
            selected_hover: SemanticColor::rgb8(60, 60, 60),
            selected_pressed: SemanticColor::rgb8(47, 47, 47),
            border: SemanticColor::rgb8(42, 42, 42),
            border_soft: SemanticColor::rgb8(35, 35, 35),
            border_strong: SemanticColor::rgb8(58, 58, 58),
            text: SemanticColor::rgb8(221, 221, 221),
            muted: SemanticColor::rgb8(163, 163, 163),
            faint: SemanticColor::rgb8(90, 90, 90),
            accent: ACCENT_DARK.base,
            accent_strong: ACCENT_DARK.strong,
            accent_soft: ACCENT_DARK.soft(),
            accent_soft_hover: ACCENT_DARK.soft_hover(),
            accent_soft_pressed: ACCENT_DARK.soft_pressed(),
            accent_on_soft: ACCENT_DARK.on_soft,
            accent_text: ACCENT_DARK.text,
            success: SemanticColor::rgb8(63, 185, 80),
            warning: SemanticColor::rgb8(212, 168, 91),
            danger: SemanticColor::rgb8(244, 113, 116),
        }
    }

    pub const fn light() -> Self {
        Self {
            background: SemanticColor::rgb8(255, 255, 255),
            surface: SemanticColor::rgb8(243, 244, 246),
            subtle: SemanticColor::rgb8(247, 248, 250),
            hover: SemanticColor::rgb8(235, 237, 240),
            active: SemanticColor::rgb8(223, 226, 231),
            selected: SemanticColor::rgb8(226, 226, 226),
            selected_hover: SemanticColor::rgb8(232, 232, 232),
            selected_pressed: SemanticColor::rgb8(223, 223, 223),
            border: SemanticColor::rgb8(227, 229, 232),
            border_soft: SemanticColor::rgb8(238, 240, 243),
            border_strong: SemanticColor::rgb8(203, 207, 213),
            text: SemanticColor::rgb8(26, 26, 31),
            muted: SemanticColor::rgb8(90, 97, 110),
            faint: SemanticColor::rgb8(156, 163, 175),
            accent: ACCENT_LIGHT.base,
            accent_strong: ACCENT_LIGHT.strong,
            accent_soft: ACCENT_LIGHT.soft(),
            accent_soft_hover: ACCENT_LIGHT.soft_hover(),
            accent_soft_pressed: ACCENT_LIGHT.soft_pressed(),
            accent_on_soft: ACCENT_LIGHT.on_soft,
            accent_text: ACCENT_LIGHT.text,
            success: SemanticColor::rgb8(16, 126, 57),
            warning: SemanticColor::rgb8(184, 119, 28),
            danger: SemanticColor::rgb8(201, 60, 60),
        }
    }

    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    /// Re-derive the whole accent family from one ramp.
    ///
    /// Setting `accent` alone leaves `accent_soft` / `accent_soft_hover` /
    /// `accent_soft_pressed` on the previous hue, because those three are the
    /// base accent at three alphas. Nothing enforced that before this method
    /// existed, which is why a custom accent used to produce a button whose
    /// fill and label disagreed about what the accent was.
    pub const fn with_accent_ramp(mut self, ramp: AccentRamp) -> Self {
        self.accent = ramp.base;
        self.accent_strong = ramp.strong;
        self.accent_soft = ramp.soft();
        self.accent_soft_hover = ramp.soft_hover();
        self.accent_soft_pressed = ramp.soft_pressed();
        self.accent_on_soft = ramp.on_soft;
        self.accent_text = ramp.text;
        self
    }

    /// The alpha channel of the field a role names, if it names one.
    ///
    /// `None` for the derived roles (`WarningSoft`, `Titlebar`, the code-token
    /// roles): those have no field of their own, so there is nothing to write.
    /// Returning `None` rather than silently doing nothing is what lets a
    /// caller — the window backdrop, today — notice that it aimed at a role
    /// that cannot hold an alpha.
    pub fn alpha_mut(&mut self, role: SemanticColorRole) -> Option<&mut f32> {
        Some(match role {
            SemanticColorRole::Background => &mut self.background.a,
            SemanticColorRole::Surface => &mut self.surface.a,
            SemanticColorRole::Subtle => &mut self.subtle.a,
            SemanticColorRole::Hover => &mut self.hover.a,
            SemanticColorRole::Active => &mut self.active.a,
            SemanticColorRole::Selected => &mut self.selected.a,
            SemanticColorRole::SelectedHover => &mut self.selected_hover.a,
            SemanticColorRole::SelectedPressed => &mut self.selected_pressed.a,
            SemanticColorRole::Border => &mut self.border.a,
            SemanticColorRole::BorderSoft => &mut self.border_soft.a,
            SemanticColorRole::BorderStrong => &mut self.border_strong.a,
            SemanticColorRole::Text => &mut self.text.a,
            SemanticColorRole::Muted => &mut self.muted.a,
            SemanticColorRole::Faint => &mut self.faint.a,
            SemanticColorRole::Accent => &mut self.accent.a,
            SemanticColorRole::AccentStrong => &mut self.accent_strong.a,
            SemanticColorRole::AccentSoft => &mut self.accent_soft.a,
            SemanticColorRole::AccentSoftHover => &mut self.accent_soft_hover.a,
            SemanticColorRole::AccentSoftPressed => &mut self.accent_soft_pressed.a,
            SemanticColorRole::AccentOnSoft => &mut self.accent_on_soft.a,
            SemanticColorRole::AccentText => &mut self.accent_text.a,
            SemanticColorRole::Success => &mut self.success.a,
            SemanticColorRole::Warning => &mut self.warning.a,
            SemanticColorRole::Danger => &mut self.danger.a,
            _ => return None,
        })
    }

    /// Resolve a role against this palette and the theme's state-layer alphas.
    ///
    /// The `opacity` argument is not ceremony. The soft warning and danger
    /// fills are the base colour at a theme-chosen alpha, and that alpha used
    /// to be a literal in this function — with the light/dark difference
    /// decided by sniffing `background.r > 0.5`. A theme with a mid-grey
    /// background got the wrong branch and no theme could move the number.
    /// Taking the alphas as an argument is what makes them a token.
    pub const fn get_in(self, role: SemanticColorRole, opacity: OpacityTokens) -> SemanticColor {
        match role {
            SemanticColorRole::Background => self.background,
            SemanticColorRole::Surface => self.surface,
            SemanticColorRole::Subtle => self.subtle,
            SemanticColorRole::Hover => self.hover,
            SemanticColorRole::Active => self.active,
            SemanticColorRole::Selected => self.selected,
            SemanticColorRole::SelectedHover => self.selected_hover,
            SemanticColorRole::SelectedPressed => self.selected_pressed,
            SemanticColorRole::Border => self.border,
            SemanticColorRole::BorderSoft => self.border_soft,
            SemanticColorRole::BorderStrong => self.border_strong,
            SemanticColorRole::Text => self.text,
            SemanticColorRole::Muted => self.muted,
            SemanticColorRole::Faint => self.faint,
            SemanticColorRole::Accent => self.accent,
            SemanticColorRole::AccentStrong => self.accent_strong,
            SemanticColorRole::AccentSoft => self.accent_soft,
            SemanticColorRole::AccentSoftHover => self.accent_soft_hover,
            SemanticColorRole::AccentSoftPressed => self.accent_soft_pressed,
            SemanticColorRole::AccentOnSoft => self.accent_on_soft,
            SemanticColorRole::AccentText => self.accent_text,
            SemanticColorRole::Success => self.success,
            SemanticColorRole::Warning => self.warning,
            SemanticColorRole::WarningSoft => SemanticColor {
                a: opacity.resolve(StateLayer::WarningSoft),
                ..self.warning
            },
            SemanticColorRole::WarningSoftHover => SemanticColor {
                a: opacity.resolve(StateLayer::WarningSoftHover),
                ..self.warning
            },
            SemanticColorRole::WarningSoftPressed => SemanticColor {
                a: opacity.resolve(StateLayer::WarningSoftPressed),
                ..self.warning
            },
            SemanticColorRole::Danger => self.danger,
            SemanticColorRole::DangerSoftHover => SemanticColor {
                a: opacity.resolve(StateLayer::DangerSoftHover),
                ..self.danger
            },
            SemanticColorRole::DangerSoftPressed => SemanticColor {
                a: opacity.resolve(StateLayer::DangerSoftPressed),
                ..self.danger
            },
            SemanticColorRole::Titlebar => self.surface,
            // 代码 token 角色默认值：Keyword/Function/Type 与 syntect
            // 基础层已有视觉档位对齐，避免开启语义高亮后反而褪色；
            // Builtin 在 syntect 无对应档位（现状不着色），accent 属
            // 语义层的增强；结构性角色里 Variable 用正文前景（变量
            // 本就是主体文本），Parameter/Const/Texture/Property 从
            // Muted 系保守起步。主题层经派生入口覆盖对应基础色即
            // 生效（如覆盖 accent 同时改 Keyword/Function/Builtin）。
            SemanticColorRole::Keyword => self.accent_strong,
            SemanticColorRole::Function => self.accent,
            SemanticColorRole::Builtin => self.accent,
            SemanticColorRole::Type => self.accent_on_soft,
            SemanticColorRole::Variable => self.text,
            SemanticColorRole::Parameter => self.muted,
            SemanticColorRole::Const => self.muted,
            SemanticColorRole::Texture => self.muted,
            SemanticColorRole::Property => self.muted,
        }
    }
}

/// Index of Style Model pieces that already live in this crate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleModelRef {
    pub theme_mode: ThemeMode,
    pub metrics: ThemeMetrics,
    pub palette: SemanticPalette,
    pub titlebar: SemanticColor,
    /// State-layer alphas for the derived soft roles. Small on purpose: this
    /// struct is the per-node read handle, and Issue #101 §1.5 measured what
    /// putting a large value on a read path costs.
    pub opacity: OpacityTokens,
}

impl StyleModelRef {
    pub const fn new(theme_mode: ThemeMode) -> Self {
        let palette = SemanticPalette::for_mode(theme_mode);
        Self {
            theme_mode,
            metrics: UI_METRICS,
            titlebar: palette.surface,
            palette,
            opacity: OpacityTokens::for_mode(theme_mode),
        }
    }

    pub const fn with_tokens(
        theme_mode: ThemeMode,
        metrics: ThemeMetrics,
        palette: SemanticPalette,
        titlebar: SemanticColor,
        opacity: OpacityTokens,
    ) -> Self {
        Self {
            theme_mode,
            metrics,
            palette,
            titlebar,
            opacity,
        }
    }

    pub const fn color(self, role: SemanticColorRole) -> SemanticColor {
        match role {
            SemanticColorRole::Titlebar => self.titlebar,
            other => self.palette.get_in(other, self.opacity),
        }
    }

    pub const fn base_text_size(self) -> f32 {
        let _ = self;
        UI_BASE_TEXT_SIZE
    }
}

impl Default for StyleModelRef {
    fn default() -> Self {
        Self::new(ThemeMode::default())
    }
}

/// Control-facing semantic slice of the Style Model (L2 props / L3 builders).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlSemantics {
    pub size: ControlSize,
    pub button_kind: Option<ButtonKind>,
    pub card_kind: Option<CardKind>,
    pub status: Option<StatusTone>,
}

impl Default for ControlSemantics {
    fn default() -> Self {
        Self {
            size: ControlSize::Medium,
            button_kind: None,
            card_kind: None,
            status: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OpacityTokens, SemanticColor, SemanticColorRole, SemanticPalette, StyleModelRef};
    use crate::theme::ThemeMode;

    #[test]
    fn semantic_color_rgb8_normalizes() {
        let c = SemanticColor::rgb8(255, 0, 128);
        assert!((c.r - 1.0).abs() < f32::EPSILON);
        assert_eq!(c.g, 0.0);
        assert!((c.b - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn style_model_ref_defaults_to_shared_metrics() {
        let model = StyleModelRef::new(ThemeMode::Light);
        assert_eq!(model.metrics.radius_md, 10.0);
        assert_eq!(model.base_text_size(), 13.0);
        assert_eq!(model.palette.accent, SemanticPalette::light().accent);
    }

    #[test]
    fn style_model_ref_titlebar_stays_independent_of_surface_alpha() {
        let mut palette = SemanticPalette::dark();
        palette.surface.a = 0.5;
        let mut titlebar = palette.surface;
        titlebar.a = 1.0;
        let model = StyleModelRef::with_tokens(
            ThemeMode::Dark,
            crate::UI_METRICS,
            palette,
            titlebar,
            OpacityTokens::DARK,
        );
        assert!((model.color(SemanticColorRole::Surface).a - 0.5).abs() < f32::EPSILON);
        assert!((model.color(SemanticColorRole::Titlebar).a - 1.0).abs() < f32::EPSILON);
        assert!(
            (model
                .palette
                .get_in(SemanticColorRole::Titlebar, OpacityTokens::DARK)
                .a
                - 0.5)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn known_css_token_maps_to_role_hex_does_not() {
        assert_eq!(
            SemanticColorRole::from_css_token_name("accent"),
            Some(SemanticColorRole::Accent)
        );
        assert_eq!(
            SemanticColorRole::from_css_token_name("var(--nana-muted)"),
            Some(SemanticColorRole::Muted)
        );
        assert_eq!(SemanticColorRole::from_css_token_name("#e74c3c"), None);
        assert_eq!(SemanticColorRole::from_css_token_name("rgb(1,2,3)"), None);
        assert_eq!(
            SemanticColorRole::from_css_token_name("titlebar"),
            Some(SemanticColorRole::Titlebar)
        );
    }

    #[test]
    fn dark_palette_accent_matches_legacy_rgb() {
        let p = SemanticPalette::dark();
        assert!((p.accent.r - 123.0 / 255.0).abs() < 1e-5);
    }

    /// 代码 token 角色默认值：与 syntect 基础层档位对齐的四角色不褪色，
    /// 结构性新角色从 Muted/Faint 系起步；dark/light 同一套派生规则。
    #[test]
    fn code_token_roles_default_to_conservative_theme_colors() {
        for palette in [SemanticPalette::dark(), SemanticPalette::light()] {
            assert_eq!(
                palette.get_in(SemanticColorRole::Keyword, OpacityTokens::DARK),
                palette.accent_strong
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Function, OpacityTokens::DARK),
                palette.accent
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Builtin, OpacityTokens::DARK),
                palette.accent
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Type, OpacityTokens::DARK),
                palette.accent_on_soft
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Variable, OpacityTokens::DARK),
                palette.text
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Parameter, OpacityTokens::DARK),
                palette.muted
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Const, OpacityTokens::DARK),
                palette.muted
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Texture, OpacityTokens::DARK),
                palette.muted
            );
            assert_eq!(
                palette.get_in(SemanticColorRole::Property, OpacityTokens::DARK),
                palette.muted
            );
            assert_ne!(
                palette.get_in(SemanticColorRole::Keyword, OpacityTokens::DARK),
                palette.text
            );
        }
    }
}
