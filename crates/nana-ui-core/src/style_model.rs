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
            _ => return None,
        })
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
            accent: SemanticColor::rgb8(123, 185, 240),
            accent_strong: SemanticColor::rgb8(73, 145, 215),
            accent_soft: SemanticColor::rgba8(123, 185, 240, 0.14),
            accent_soft_hover: SemanticColor::rgba8(123, 185, 240, 0.20),
            accent_soft_pressed: SemanticColor::rgba8(123, 185, 240, 0.23),
            accent_on_soft: SemanticColor::rgb8(123, 185, 240),
            accent_text: SemanticColor::rgb8(13, 22, 34),
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
            accent: SemanticColor::rgb8(73, 145, 215),
            accent_strong: SemanticColor::rgb8(44, 126, 214),
            accent_soft: SemanticColor::rgba8(73, 145, 215, 0.10),
            accent_soft_hover: SemanticColor::rgba8(73, 145, 215, 0.20),
            accent_soft_pressed: SemanticColor::rgba8(73, 145, 215, 0.23),
            accent_on_soft: SemanticColor::rgb8(0, 85, 159),
            accent_text: SemanticColor::rgba(1.0, 1.0, 1.0, 1.0),
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

    pub const fn get(self, role: SemanticColorRole) -> SemanticColor {
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
                a: if self.background.r > 0.5 { 0.12 } else { 0.16 },
                ..self.warning
            },
            SemanticColorRole::WarningSoftHover => SemanticColor {
                a: 0.20,
                ..self.warning
            },
            SemanticColorRole::WarningSoftPressed => SemanticColor {
                a: 0.24,
                ..self.warning
            },
            SemanticColorRole::Danger => self.danger,
            SemanticColorRole::DangerSoftHover => SemanticColor {
                a: 0.18,
                ..self.danger
            },
            SemanticColorRole::DangerSoftPressed => SemanticColor {
                a: 0.22,
                ..self.danger
            },
        }
    }
}

/// Index of Style Model pieces that already live in this crate.
#[derive(Debug, Clone, Copy)]
pub struct StyleModelRef {
    pub theme_mode: ThemeMode,
    pub metrics: ThemeMetrics,
    pub palette: SemanticPalette,
}

impl StyleModelRef {
    pub const fn new(theme_mode: ThemeMode) -> Self {
        Self {
            theme_mode,
            metrics: UI_METRICS,
            palette: SemanticPalette::for_mode(theme_mode),
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
    use super::{SemanticColor, SemanticColorRole, SemanticPalette, StyleModelRef};
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
    }

    #[test]
    fn dark_palette_accent_matches_legacy_rgb() {
        let p = SemanticPalette::dark();
        assert!((p.accent.r - 123.0 / 255.0).abs() < 1e-5);
    }
}
