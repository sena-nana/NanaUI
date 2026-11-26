//! Typed sparse theme patches shared by `ThemeScope` (#104) and `ThemePackage` (#109).

use serde::{Deserialize, Serialize};

use super::controls::{InputRecipe, MenuRecipe, SelectRecipe, TabsRecipe, TreeViewRecipe};
use super::definition::validate_compiled_theme;
use super::dependency::ThemeDependencyMask;
use super::recipe::{
    ButtonRecipe, ButtonVariantDraft, ButtonVariantRecipe, ComponentRecipe, ComponentRecipeDraft,
    ComponentRecipeId, StatusRecipe, button_index,
};
use super::{
    AccentRamp, BorderTokens, CompiledTheme, CornerShape, DialogRecipe, EffectTokens,
    FocusRingMetrics, KeycapMetrics, MotionTokens, OpacityTokens, ScrollbarRecipe, ShadowToken,
    ShapeRecipe, SpacingTokens, SurfaceSpec, SurfaceTokens, SwitchMetrics, ThemeAppearance,
    ThemeCompileError, ThemeDefinition, ThemeMetrics, ToastRecipe, TreeMetrics, TypographyTokens,
};
use crate::ScrollbarMetrics;
use crate::motion::Easing;
use crate::semantics::ButtonKind;
use crate::style_model::{ColorOverrides, SemanticColor, SemanticColorRole};

/// Sparse patch over [`ThemeMetrics`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeMetricsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_xs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_sm: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_md: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_lg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_xl: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_xxl: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_control_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_control_padding_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_padding_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_button_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_padding_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_padding_y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_padding_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_item_padding_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large_control_padding_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollbar: Option<ScrollbarMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch: Option<SwitchMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<TreeMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycap: Option<KeycapMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_shape: Option<CornerShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_ring: Option<FocusRingMetrics>,
}

impl ThemeMetricsPatch {
    pub fn from_metrics(m: ThemeMetrics) -> Self {
        Self {
            radius_xs: Some(m.radius_xs),
            radius_sm: Some(m.radius_sm),
            radius_md: Some(m.radius_md),
            radius_lg: Some(m.radius_lg),
            radius_xl: Some(m.radius_xl),
            radius_xxl: Some(m.radius_xxl),
            compact_control_height: Some(m.compact_control_height),
            control_height: Some(m.control_height),
            compact_control_padding_x: Some(m.compact_control_padding_x),
            control_padding_x: Some(m.control_padding_x),
            selection_height: Some(m.selection_height),
            icon_button_size: Some(m.icon_button_size),
            panel_padding_x: Some(m.panel_padding_x),
            panel_padding_y: Some(m.panel_padding_y),
            field_padding_x: Some(m.field_padding_x),
            list_item_padding_x: Some(m.list_item_padding_x),
            large_control_padding_x: Some(m.large_control_padding_x),
            scrollbar: Some(m.scrollbar),
            switch: Some(m.switch),
            tree: Some(m.tree),
            keycap: Some(m.keycap),
            corner_shape: Some(m.corner_shape),
            focus_ring: Some(m.focus_ring),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(&self, mut target: ThemeMetrics) -> ThemeMetrics {
        macro_rules! set_opt {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(v) = self.$field {
                    target.$field = v;
                })+
            };
        }
        set_opt!(
            radius_xs,
            radius_sm,
            radius_md,
            radius_lg,
            radius_xl,
            radius_xxl,
            compact_control_height,
            control_height,
            compact_control_padding_x,
            control_padding_x,
            selection_height,
            icon_button_size,
            panel_padding_x,
            panel_padding_y,
            field_padding_x,
            list_item_padding_x,
            large_control_padding_x,
            scrollbar,
            switch,
            tree,
            keycap,
            corner_shape,
            focus_ring,
        );
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            radius_xs: self.radius_xs.or(outer.radius_xs),
            radius_sm: self.radius_sm.or(outer.radius_sm),
            radius_md: self.radius_md.or(outer.radius_md),
            radius_lg: self.radius_lg.or(outer.radius_lg),
            radius_xl: self.radius_xl.or(outer.radius_xl),
            radius_xxl: self.radius_xxl.or(outer.radius_xxl),
            compact_control_height: self.compact_control_height.or(outer.compact_control_height),
            control_height: self.control_height.or(outer.control_height),
            compact_control_padding_x: self
                .compact_control_padding_x
                .or(outer.compact_control_padding_x),
            control_padding_x: self.control_padding_x.or(outer.control_padding_x),
            selection_height: self.selection_height.or(outer.selection_height),
            icon_button_size: self.icon_button_size.or(outer.icon_button_size),
            panel_padding_x: self.panel_padding_x.or(outer.panel_padding_x),
            panel_padding_y: self.panel_padding_y.or(outer.panel_padding_y),
            field_padding_x: self.field_padding_x.or(outer.field_padding_x),
            list_item_padding_x: self.list_item_padding_x.or(outer.list_item_padding_x),
            large_control_padding_x: self
                .large_control_padding_x
                .or(outer.large_control_padding_x),
            scrollbar: self.scrollbar.or(outer.scrollbar),
            switch: self.switch.or(outer.switch),
            tree: self.tree.or(outer.tree),
            keycap: self.keycap.or(outer.keycap),
            corner_shape: self.corner_shape.or(outer.corner_shape),
            focus_ring: self.focus_ring.or(outer.focus_ring),
        }
    }

    pub fn affected_mask(&self) -> ThemeDependencyMask {
        let mut mask = ThemeDependencyMask::NONE;
        if self.radius_xs.is_some()
            || self.radius_sm.is_some()
            || self.radius_md.is_some()
            || self.radius_lg.is_some()
            || self.radius_xl.is_some()
            || self.radius_xxl.is_some()
            || self.compact_control_height.is_some()
            || self.control_height.is_some()
            || self.compact_control_padding_x.is_some()
            || self.control_padding_x.is_some()
            || self.selection_height.is_some()
            || self.icon_button_size.is_some()
            || self.panel_padding_x.is_some()
            || self.panel_padding_y.is_some()
            || self.field_padding_x.is_some()
            || self.list_item_padding_x.is_some()
            || self.large_control_padding_x.is_some()
            || self.switch.is_some()
            || self.tree.is_some()
            || self.keycap.is_some()
        {
            mask = mask.union(ThemeDependencyMask::METRICS);
        }
        if self.scrollbar.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_SCROLLBAR);
        }
        if self.corner_shape.is_some() || self.focus_ring.is_some() {
            mask = mask.union(ThemeDependencyMask::EFFECT);
        }
        mask
    }
}

/// Sparse patch over [`SpacingTokens`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpacingPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xxs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sm: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xl: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xxl: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xxxl: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_tight: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<f32>,
}

impl SpacingPatch {
    pub fn from_tokens(s: SpacingTokens) -> Self {
        Self {
            xxs: Some(s.xxs),
            xs: Some(s.xs),
            sm: Some(s.sm),
            md: Some(s.md),
            lg: Some(s.lg),
            xl: Some(s.xl),
            xxl: Some(s.xxl),
            xxxl: Some(s.xxxl),
            page_tight: Some(s.page_tight),
            page: Some(s.page),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(&self, mut target: SpacingTokens) -> SpacingTokens {
        if let Some(v) = self.xxs {
            target.xxs = v;
        }
        if let Some(v) = self.xs {
            target.xs = v;
        }
        if let Some(v) = self.sm {
            target.sm = v;
        }
        if let Some(v) = self.md {
            target.md = v;
        }
        if let Some(v) = self.lg {
            target.lg = v;
        }
        if let Some(v) = self.xl {
            target.xl = v;
        }
        if let Some(v) = self.xxl {
            target.xxl = v;
        }
        if let Some(v) = self.xxxl {
            target.xxxl = v;
        }
        if let Some(v) = self.page_tight {
            target.page_tight = v;
        }
        if let Some(v) = self.page {
            target.page = v;
        }
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            xxs: self.xxs.or(outer.xxs),
            xs: self.xs.or(outer.xs),
            sm: self.sm.or(outer.sm),
            md: self.md.or(outer.md),
            lg: self.lg.or(outer.lg),
            xl: self.xl.or(outer.xl),
            xxl: self.xxl.or(outer.xxl),
            xxxl: self.xxxl.or(outer.xxxl),
            page_tight: self.page_tight.or(outer.page_tight),
            page: self.page.or(outer.page),
        }
    }
}

/// Sparse patch over [`BorderTokens`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorderPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hairline: Option<f32>,
}

impl BorderPatch {
    pub fn is_empty(&self) -> bool {
        self.hairline.is_none()
    }

    pub fn apply(&self, mut target: BorderTokens) -> BorderTokens {
        if let Some(v) = self.hairline {
            target.hairline = v;
        }
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            hairline: self.hairline.or(outer.hairline),
        }
    }
}

/// Sparse patch over [`OpacityTokens`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpacityPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_soft: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_soft_hover: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_soft_pressed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub danger_soft_hover: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub danger_soft_pressed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_text: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice_tint: Option<f32>,
}

impl OpacityPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(&self, mut target: OpacityTokens) -> OpacityTokens {
        if let Some(v) = self.warning_soft {
            target.warning_soft = v;
        }
        if let Some(v) = self.warning_soft_hover {
            target.warning_soft_hover = v;
        }
        if let Some(v) = self.warning_soft_pressed {
            target.warning_soft_pressed = v;
        }
        if let Some(v) = self.danger_soft_hover {
            target.danger_soft_hover = v;
        }
        if let Some(v) = self.danger_soft_pressed {
            target.danger_soft_pressed = v;
        }
        if let Some(v) = self.secondary_text {
            target.secondary_text = v;
        }
        if let Some(v) = self.disabled {
            target.disabled = v;
        }
        if let Some(v) = self.notice_tint {
            target.notice_tint = v;
        }
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            warning_soft: self.warning_soft.or(outer.warning_soft),
            warning_soft_hover: self.warning_soft_hover.or(outer.warning_soft_hover),
            warning_soft_pressed: self.warning_soft_pressed.or(outer.warning_soft_pressed),
            danger_soft_hover: self.danger_soft_hover.or(outer.danger_soft_hover),
            danger_soft_pressed: self.danger_soft_pressed.or(outer.danger_soft_pressed),
            secondary_text: self.secondary_text.or(outer.secondary_text),
            disabled: self.disabled.or(outer.disabled),
            notice_tint: self.notice_tint.or(outer.notice_tint),
        }
    }

    pub fn affected_mask(&self) -> ThemeDependencyMask {
        let mut mask = ThemeDependencyMask::NONE;
        if self.warning_soft.is_some()
            || self.warning_soft_hover.is_some()
            || self.warning_soft_pressed.is_some()
            || self.danger_soft_hover.is_some()
            || self.danger_soft_pressed.is_some()
        {
            mask = mask.union(ThemeDependencyMask::PALETTE);
        }
        if self.secondary_text.is_some() || self.disabled.is_some() || self.notice_tint.is_some() {
            mask = mask.union(ThemeDependencyMask::EFFECT);
        }
        mask
    }
}

/// Sparse patch over [`TypographyTokens`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypographyPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_title: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_tall: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regular: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semibold: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<u16>,
}

impl TypographyPatch {
    pub fn from_tokens(t: TypographyTokens) -> Self {
        Self {
            hint: Some(t.hint),
            meta: Some(t.meta),
            body: Some(t.body),
            section: Some(t.section),
            card_title: Some(t.card_title),
            heading: Some(t.heading),
            title: Some(t.title),
            display: Some(t.display),
            line: Some(t.line),
            line_tall: Some(t.line_tall),
            regular: Some(t.regular),
            medium: Some(t.medium),
            semibold: Some(t.semibold),
            bold: Some(t.bold),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(&self, mut target: TypographyTokens) -> TypographyTokens {
        macro_rules! set_opt {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(v) = self.$field {
                    target.$field = v;
                })+
            };
        }
        set_opt!(
            hint, meta, body, section, card_title, heading, title, display, line, line_tall,
            regular, medium, semibold, bold,
        );
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            hint: self.hint.or(outer.hint),
            meta: self.meta.or(outer.meta),
            body: self.body.or(outer.body),
            section: self.section.or(outer.section),
            card_title: self.card_title.or(outer.card_title),
            heading: self.heading.or(outer.heading),
            title: self.title.or(outer.title),
            display: self.display.or(outer.display),
            line: self.line.or(outer.line),
            line_tall: self.line_tall.or(outer.line_tall),
            regular: self.regular.or(outer.regular),
            medium: self.medium.or(outer.medium),
            semibold: self.semibold.or(outer.semibold),
            bold: self.bold.or(outer.bold),
        }
    }

    pub fn affected_mask(&self) -> ThemeDependencyMask {
        let mut mask = ThemeDependencyMask::NONE;
        if self.hint.is_some()
            || self.meta.is_some()
            || self.body.is_some()
            || self.section.is_some()
            || self.card_title.is_some()
            || self.heading.is_some()
            || self.title.is_some()
            || self.display.is_some()
            || self.line.is_some()
            || self.line_tall.is_some()
        {
            mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_LAYOUT);
        }
        if self.regular.is_some()
            || self.medium.is_some()
            || self.semibold.is_some()
            || self.bold.is_some()
        {
            mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_SHAPE);
        }
        mask
    }
}

/// Sparse patch over [`MotionTokens`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover_color_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_fade_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_opacity_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_pop_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_collapse_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_pulse_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spinner_rotation_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_spin_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_hold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_easing: Option<Easing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard_easing: Option<Easing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expand_easing: Option<Easing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emphasized_easing: Option<Easing>,
}

impl MotionPatch {
    pub fn from_tokens(m: MotionTokens) -> Self {
        Self {
            hover_color_ms: Some(m.hover_color_ms),
            overlay_fade_ms: Some(m.overlay_fade_ms),
            menu_opacity_ms: Some(m.menu_opacity_ms),
            menu_pop_ms: Some(m.menu_pop_ms),
            sidebar_collapse_ms: Some(m.sidebar_collapse_ms),
            skeleton_pulse_ms: Some(m.skeleton_pulse_ms),
            spinner_rotation_ms: Some(m.spinner_rotation_ms),
            loading_spin_ms: Some(m.loading_spin_ms),
            attention_ms: Some(m.attention_ms),
            attention_hold: Some(m.attention_hold),
            linear_easing: Some(m.linear_easing),
            standard_easing: Some(m.standard_easing),
            expand_easing: Some(m.expand_easing),
            emphasized_easing: Some(m.emphasized_easing),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(&self, mut target: MotionTokens) -> MotionTokens {
        macro_rules! set_opt {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(v) = self.$field {
                    target.$field = v;
                })+
            };
        }
        set_opt!(
            hover_color_ms,
            overlay_fade_ms,
            menu_opacity_ms,
            menu_pop_ms,
            sidebar_collapse_ms,
            skeleton_pulse_ms,
            spinner_rotation_ms,
            loading_spin_ms,
            attention_ms,
            attention_hold,
            linear_easing,
            standard_easing,
            expand_easing,
            emphasized_easing,
        );
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            hover_color_ms: self.hover_color_ms.or(outer.hover_color_ms),
            overlay_fade_ms: self.overlay_fade_ms.or(outer.overlay_fade_ms),
            menu_opacity_ms: self.menu_opacity_ms.or(outer.menu_opacity_ms),
            menu_pop_ms: self.menu_pop_ms.or(outer.menu_pop_ms),
            sidebar_collapse_ms: self.sidebar_collapse_ms.or(outer.sidebar_collapse_ms),
            skeleton_pulse_ms: self.skeleton_pulse_ms.or(outer.skeleton_pulse_ms),
            spinner_rotation_ms: self.spinner_rotation_ms.or(outer.spinner_rotation_ms),
            loading_spin_ms: self.loading_spin_ms.or(outer.loading_spin_ms),
            attention_ms: self.attention_ms.or(outer.attention_ms),
            attention_hold: self.attention_hold.or(outer.attention_hold),
            linear_easing: self.linear_easing.or(outer.linear_easing),
            standard_easing: self.standard_easing.or(outer.standard_easing),
            expand_easing: self.expand_easing.or(outer.expand_easing),
            emphasized_easing: self.emphasized_easing.or(outer.emphasized_easing),
        }
    }
}

/// Sparse patch over [`EffectTokens`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<ShadowToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<ShadowToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<ShadowToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropdown: Option<ShadowToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_bar: Option<ShadowToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_scrim: Option<SemanticColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_foreground: Option<SemanticColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_hover_alpha: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_pressed_alpha: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_danger_pressed_alpha: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modal_scrim: Option<SemanticColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modal_scrim_blur: Option<f32>,
}

impl EffectPatch {
    pub fn from_tokens(e: EffectTokens) -> Self {
        Self {
            surface: Some(e.surface),
            overlay: Some(e.overlay),
            menu: Some(e.menu),
            dropdown: Some(e.dropdown),
            menu_bar: Some(e.menu_bar),
            media_scrim: Some(e.media_scrim),
            media_foreground: Some(e.media_foreground),
            media_hover_alpha: Some(e.media_hover_alpha),
            media_pressed_alpha: Some(e.media_pressed_alpha),
            media_danger_pressed_alpha: Some(e.media_danger_pressed_alpha),
            modal_scrim: Some(e.modal_scrim),
            modal_scrim_blur: Some(e.modal_scrim_blur),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(&self, mut target: EffectTokens) -> EffectTokens {
        macro_rules! set_opt {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(v) = self.$field {
                    target.$field = v;
                })+
            };
        }
        set_opt!(
            surface,
            overlay,
            menu,
            dropdown,
            menu_bar,
            media_scrim,
            media_foreground,
            media_hover_alpha,
            media_pressed_alpha,
            media_danger_pressed_alpha,
            modal_scrim,
            modal_scrim_blur,
        );
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            surface: self.surface.or(outer.surface),
            overlay: self.overlay.or(outer.overlay),
            menu: self.menu.or(outer.menu),
            dropdown: self.dropdown.or(outer.dropdown),
            menu_bar: self.menu_bar.or(outer.menu_bar),
            media_scrim: self.media_scrim.or(outer.media_scrim),
            media_foreground: self.media_foreground.or(outer.media_foreground),
            media_hover_alpha: self.media_hover_alpha.or(outer.media_hover_alpha),
            media_pressed_alpha: self.media_pressed_alpha.or(outer.media_pressed_alpha),
            media_danger_pressed_alpha: self
                .media_danger_pressed_alpha
                .or(outer.media_danger_pressed_alpha),
            modal_scrim: self.modal_scrim.or(outer.modal_scrim),
            modal_scrim_blur: self.modal_scrim_blur.or(outer.modal_scrim_blur),
        }
    }
}

/// Sparse patch over [`SurfaceTokens`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfacePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SurfaceSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome: Option<SurfaceSpec>,
}

impl SurfacePatch {
    pub fn is_empty(&self) -> bool {
        self.window.is_none() && self.chrome.is_none()
    }

    pub fn apply(&self, mut target: SurfaceTokens) -> SurfaceTokens {
        if let Some(v) = self.window {
            target.window = v;
        }
        if let Some(v) = self.chrome {
            target.chrome = v;
        }
        target
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        Self {
            window: self.window.or(outer.window),
            chrome: self.chrome.or(outer.chrome),
        }
    }
}

/// Sparse patch over [`ButtonRecipe`]: overrides individual [`ButtonKind`] variants
/// and/or `invalid_border`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonRecipePatch {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<(ButtonKind, ButtonVariantRecipe)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_border: Option<SemanticColorRole>,
}

impl ButtonRecipePatch {
    pub fn is_empty(&self) -> bool {
        self.variants.is_empty() && self.invalid_border.is_none()
    }

    #[must_use]
    pub fn with_variant(mut self, kind: ButtonKind, variant: ButtonVariantRecipe) -> Self {
        match self.variants.iter_mut().find(|(k, _)| *k == kind) {
            Some(entry) => entry.1 = variant,
            None => self.variants.push((kind, variant)),
        }
        self
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        let mut merged = outer.clone();
        for &(kind, variant) in &self.variants {
            merged = merged.with_variant(kind, variant);
        }
        if let Some(invalid_border) = self.invalid_border {
            merged.invalid_border = Some(invalid_border);
        }
        merged
    }

    pub fn apply_to_compiled(&self, base: &ButtonRecipe) -> ButtonRecipe {
        let mut variants = [
            base.variant(ButtonKind::Ghost),
            base.variant(ButtonKind::Subtle),
            base.variant(ButtonKind::Selected),
            base.variant(ButtonKind::Primary),
            base.variant(ButtonKind::Warning),
            base.variant(ButtonKind::Danger),
            base.variant(ButtonKind::Text),
            base.variant(ButtonKind::Menu),
        ];
        for &(kind, variant) in &self.variants {
            variants[button_index(kind)] = variant;
        }
        ButtonRecipe::from_variants(variants, self.invalid_border.unwrap_or(base.invalid_border))
    }

    pub fn apply_to_definition(&self, target: &mut super::ButtonRecipeDraft) {
        for &(kind, variant) in &self.variants {
            target.variants[button_index(kind)] = ButtonVariantDraft {
                foreground: Some(variant.foreground),
                background: Some(variant.background),
                border: Some(variant.border),
                hovered_background: Some(variant.hovered_background),
                pressed_background: Some(variant.pressed_background),
            };
        }
        if let Some(invalid_border) = self.invalid_border {
            target.invalid_border = Some(invalid_border);
        }
    }
}

/// Sparse patch over component recipes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipePatch {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub families: Vec<(ComponentRecipeId, ComponentRecipe)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<ButtonRecipePatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialog: Option<DialogRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollbar: Option<ScrollbarRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toast: Option<ToastRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<SelectRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<TabsRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_view: Option<TreeViewRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<MenuRecipe>,
}

impl RecipePatch {
    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
            && self.button.as_ref().is_none_or(ButtonRecipePatch::is_empty)
            && self.status.is_none()
            && self.dialog.is_none()
            && self.scrollbar.is_none()
            && self.toast.is_none()
            && self.shape.is_none()
            && self.input.is_none()
            && self.select.is_none()
            && self.tabs.is_none()
            && self.tree_view.is_none()
            && self.menu.is_none()
    }

    #[must_use]
    pub fn with_family(mut self, id: ComponentRecipeId, recipe: ComponentRecipe) -> Self {
        match self.families.iter_mut().find(|(k, _)| *k == id) {
            Some(entry) => entry.1 = recipe,
            None => self.families.push((id, recipe)),
        }
        self
    }

    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        let mut merged = outer.clone();
        for &(id, recipe) in &self.families {
            merged = merged.with_family(id, recipe);
        }
        merged.button = match (&self.button, &outer.button) {
            (Some(inner), Some(out)) => Some(inner.over(out)),
            (Some(inner), None) => Some(inner.clone()),
            (None, Some(out)) => Some(out.clone()),
            (None, None) => None,
        };
        merged.status = self.status.or(outer.status);
        merged.dialog = self.dialog.or(outer.dialog);
        merged.scrollbar = self.scrollbar.or(outer.scrollbar);
        merged.toast = self.toast.or(outer.toast);
        merged.shape = self.shape.or(outer.shape);
        merged.input = self.input.or(outer.input);
        merged.select = self.select.or(outer.select);
        merged.tabs = self.tabs.or(outer.tabs);
        merged.tree_view = self.tree_view.or(outer.tree_view);
        merged.menu = self.menu.or(outer.menu);
        merged
    }

    pub fn affected_mask(&self) -> ThemeDependencyMask {
        let mut mask = ThemeDependencyMask::NONE;
        if !self.families.is_empty() {
            mask = mask.union(ThemeDependencyMask::RECIPE_FAMILY);
        }
        if self.button.as_ref().is_some_and(|b| !b.is_empty()) {
            mask = mask.union(ThemeDependencyMask::RECIPE_BUTTON);
        }
        if self.status.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_STATUS);
        }
        if self.dialog.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_DIALOG);
        }
        if self.scrollbar.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_SCROLLBAR);
        }
        if self.toast.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_TOAST);
        }
        if self.shape.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_SHAPE);
        }
        if self.input.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_INPUT);
        }
        if self.select.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_SELECT);
        }
        if self.tabs.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_TABS);
        }
        if self.tree_view.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_TREE_VIEW);
        }
        if self.menu.is_some() {
            mask = mask.union(ThemeDependencyMask::RECIPE_MENU);
        }
        mask
    }
}

/// Unified typed delta over a theme, consumed by both subtree `ThemeScope`
/// (#104) and file/wire `ThemePackage` (#109).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<ThemeAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<AccentRamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<ColorOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub titlebar: Option<Option<SemanticColor>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<ThemeMetricsPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<SpacingPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<BorderPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<OpacityPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typography: Option<TypographyPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<EffectPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surfaces: Option<SurfacePatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipes: Option<RecipePatch>,
}

impl ThemePatch {
    pub const fn new() -> Self {
        Self {
            appearance: None,
            accent: None,
            colors: None,
            titlebar: None,
            metrics: None,
            spacing: None,
            border: None,
            opacity: None,
            typography: None,
            motion: None,
            effects: None,
            surfaces: None,
            recipes: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.appearance.is_none()
            && self.accent.is_none()
            && self.colors.as_ref().is_none_or(ColorOverrides::is_empty)
            && self.titlebar.is_none()
            && self
                .metrics
                .as_ref()
                .is_none_or(ThemeMetricsPatch::is_empty)
            && self.spacing.as_ref().is_none_or(SpacingPatch::is_empty)
            && self.border.as_ref().is_none_or(BorderPatch::is_empty)
            && self.opacity.as_ref().is_none_or(OpacityPatch::is_empty)
            && self
                .typography
                .as_ref()
                .is_none_or(TypographyPatch::is_empty)
            && self.motion.as_ref().is_none_or(MotionPatch::is_empty)
            && self.effects.as_ref().is_none_or(EffectPatch::is_empty)
            && self.surfaces.as_ref().is_none_or(SurfacePatch::is_empty)
            && self.recipes.as_ref().is_none_or(RecipePatch::is_empty)
    }

    #[must_use]
    pub fn with_appearance(mut self, appearance: ThemeAppearance) -> Self {
        self.appearance = Some(appearance);
        self
    }

    #[must_use]
    pub fn with_accent(mut self, accent: AccentRamp) -> Self {
        self.accent = Some(accent);
        self
    }

    #[must_use]
    pub fn with_color(mut self, role: SemanticColorRole, color: SemanticColor) -> Self {
        let mut colors = self.colors.take().unwrap_or_default();
        colors.set(role, color);
        self.colors = Some(colors);
        self
    }

    #[must_use]
    pub fn with_colors(mut self, colors: ColorOverrides) -> Self {
        if !colors.is_empty() {
            self.colors = Some(match self.colors.take() {
                Some(prev) => colors.over(&prev),
                None => colors,
            });
        }
        self
    }

    #[must_use]
    pub fn with_titlebar(mut self, titlebar: Option<SemanticColor>) -> Self {
        self.titlebar = Some(titlebar);
        self
    }

    #[must_use]
    pub fn with_metrics(mut self, metrics: ThemeMetrics) -> Self {
        self.metrics = Some(ThemeMetricsPatch::from_metrics(metrics));
        self
    }

    #[must_use]
    pub fn with_metrics_patch(mut self, patch: ThemeMetricsPatch) -> Self {
        self.metrics = Some(match self.metrics.take() {
            Some(prev) => patch.over(&prev),
            None => patch,
        });
        self
    }

    #[must_use]
    pub fn with_spacing(mut self, spacing: SpacingTokens) -> Self {
        self.spacing = Some(SpacingPatch::from_tokens(spacing));
        self
    }

    #[must_use]
    pub fn with_spacing_patch(mut self, patch: SpacingPatch) -> Self {
        self.spacing = Some(match self.spacing.take() {
            Some(prev) => patch.over(&prev),
            None => patch,
        });
        self
    }

    #[must_use]
    pub fn with_border(mut self, border: BorderTokens) -> Self {
        self.border = Some(BorderPatch {
            hairline: Some(border.hairline),
        });
        self
    }

    #[must_use]
    pub fn with_opacity_patch(mut self, patch: OpacityPatch) -> Self {
        self.opacity = Some(match self.opacity.take() {
            Some(prev) => patch.over(&prev),
            None => patch,
        });
        self
    }

    #[must_use]
    pub fn with_typography(mut self, typography: TypographyTokens) -> Self {
        self.typography = Some(TypographyPatch::from_tokens(typography));
        self
    }

    #[must_use]
    pub fn with_typography_patch(mut self, patch: TypographyPatch) -> Self {
        self.typography = Some(match self.typography.take() {
            Some(prev) => patch.over(&prev),
            None => patch,
        });
        self
    }

    #[must_use]
    pub fn with_motion(mut self, motion: MotionTokens) -> Self {
        self.motion = Some(MotionPatch::from_tokens(motion));
        self
    }

    #[must_use]
    pub fn with_motion_patch(mut self, patch: MotionPatch) -> Self {
        self.motion = Some(match self.motion.take() {
            Some(prev) => patch.over(&prev),
            None => patch,
        });
        self
    }

    #[must_use]
    pub fn with_effects(mut self, effects: EffectTokens) -> Self {
        self.effects = Some(EffectPatch::from_tokens(effects));
        self
    }

    #[must_use]
    pub fn with_effect_patch(mut self, patch: EffectPatch) -> Self {
        self.effects = Some(match self.effects.take() {
            Some(prev) => patch.over(&prev),
            None => patch,
        });
        self
    }

    #[must_use]
    pub fn with_surface_patch(mut self, patch: SurfacePatch) -> Self {
        self.surfaces = Some(match self.surfaces.take() {
            Some(prev) => patch.over(&prev),
            None => patch,
        });
        self
    }

    #[must_use]
    pub fn with_recipes(mut self, recipes: RecipePatch) -> Self {
        self.recipes = Some(match self.recipes.take() {
            Some(prev) => recipes.over(&prev),
            None => recipes,
        });
        self
    }

    #[must_use]
    pub fn with_button_variant(self, kind: ButtonKind, variant: ButtonVariantRecipe) -> Self {
        self.with_recipes(RecipePatch {
            button: Some(ButtonRecipePatch::default().with_variant(kind, variant)),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_button_patch(self, button: ButtonRecipePatch) -> Self {
        self.with_recipes(RecipePatch {
            button: Some(button),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_shape_recipe(self, shape: ShapeRecipe) -> Self {
        self.with_recipes(RecipePatch {
            shape: Some(shape),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_input_recipe(self, input: InputRecipe) -> Self {
        self.with_recipes(RecipePatch {
            input: Some(input),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_select_recipe(self, select: SelectRecipe) -> Self {
        self.with_recipes(RecipePatch {
            select: Some(select),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_tabs_recipe(self, tabs: TabsRecipe) -> Self {
        self.with_recipes(RecipePatch {
            tabs: Some(tabs),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_tree_view_recipe(self, tree_view: TreeViewRecipe) -> Self {
        self.with_recipes(RecipePatch {
            tree_view: Some(tree_view),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_menu_recipe(self, menu: MenuRecipe) -> Self {
        self.with_recipes(RecipePatch {
            menu: Some(menu),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_dialog_recipe(self, dialog: DialogRecipe) -> Self {
        self.with_recipes(RecipePatch {
            dialog: Some(dialog),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_scrollbar_recipe(self, scrollbar: ScrollbarRecipe) -> Self {
        self.with_recipes(RecipePatch {
            scrollbar: Some(scrollbar),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_toast_recipe(self, toast: ToastRecipe) -> Self {
        self.with_recipes(RecipePatch {
            toast: Some(toast),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_status_recipe(self, status: StatusRecipe) -> Self {
        self.with_recipes(RecipePatch {
            status: Some(status),
            ..RecipePatch::default()
        })
    }

    #[must_use]
    pub fn with_family_recipe(self, id: ComponentRecipeId, recipe: ComponentRecipe) -> Self {
        self.with_recipes(RecipePatch::default().with_family(id, recipe))
    }

    /// Merge `self` over `outer`: fields set in `self` replace `outer`.
    #[must_use]
    pub fn over(&self, outer: &Self) -> Self {
        fn merge_opt<T: Clone>(
            inner: Option<&T>,
            outer: Option<&T>,
            combine: impl FnOnce(&T, &T) -> T,
        ) -> Option<T> {
            match (inner, outer) {
                (Some(i), Some(o)) => Some(combine(i, o)),
                (Some(i), None) => Some(i.clone()),
                (None, Some(o)) => Some(o.clone()),
                (None, None) => None,
            }
        }
        Self {
            appearance: self.appearance.or(outer.appearance),
            accent: self.accent.or(outer.accent),
            colors: merge_opt(self.colors.as_ref(), outer.colors.as_ref(), |i, o| {
                i.over(o)
            }),
            titlebar: self.titlebar.or(outer.titlebar),
            metrics: merge_opt(self.metrics.as_ref(), outer.metrics.as_ref(), |i, o| {
                i.over(o)
            }),
            spacing: merge_opt(self.spacing.as_ref(), outer.spacing.as_ref(), |i, o| {
                i.over(o)
            }),
            border: merge_opt(self.border.as_ref(), outer.border.as_ref(), |i, o| {
                i.over(o)
            }),
            opacity: merge_opt(self.opacity.as_ref(), outer.opacity.as_ref(), |i, o| {
                i.over(o)
            }),
            typography: merge_opt(
                self.typography.as_ref(),
                outer.typography.as_ref(),
                |i, o| i.over(o),
            ),
            motion: merge_opt(self.motion.as_ref(), outer.motion.as_ref(), |i, o| {
                i.over(o)
            }),
            effects: merge_opt(self.effects.as_ref(), outer.effects.as_ref(), |i, o| {
                i.over(o)
            }),
            surfaces: merge_opt(self.surfaces.as_ref(), outer.surfaces.as_ref(), |i, o| {
                i.over(o)
            }),
            recipes: merge_opt(self.recipes.as_ref(), outer.recipes.as_ref(), |i, o| {
                i.over(o)
            }),
        }
    }

    pub fn affected_mask(&self) -> ThemeDependencyMask {
        let mut mask = ThemeDependencyMask::NONE;
        if self.appearance.is_some() {
            mask = mask.union(ThemeDependencyMask::PALETTE);
        }
        if self.accent.is_some() {
            mask = mask.union(ThemeDependencyMask::ACCENT);
        }
        if let Some(colors) = &self.colors {
            for (role, _) in colors.iter() {
                mask = mask.union(ThemeDependencyMask::for_color_role(role));
            }
        }
        if self.titlebar.is_some() {
            mask = mask.union(ThemeDependencyMask::PALETTE);
        }
        if let Some(metrics) = &self.metrics {
            mask = mask.union(metrics.affected_mask());
        }
        if self.spacing.as_ref().is_some_and(|s| !s.is_empty())
            || self.border.as_ref().is_some_and(|b| !b.is_empty())
        {
            mask = mask.union(ThemeDependencyMask::METRICS);
        }
        if let Some(opacity) = &self.opacity {
            mask = mask.union(opacity.affected_mask());
        }
        if let Some(typography) = &self.typography {
            mask = mask.union(typography.affected_mask());
        }
        if self.motion.as_ref().is_some_and(|m| !m.is_empty()) {
            mask = mask.union(ThemeDependencyMask::MOTION_POLICY);
        }
        if let Some(effects) = &self.effects {
            if !effects.is_empty() {
                mask = mask.union(ThemeDependencyMask::EFFECT);
            }
            if effects.media_foreground.is_some() {
                mask = mask.union(ThemeDependencyMask::PALETTE);
            }
        }
        if self.surfaces.as_ref().is_some_and(|s| !s.is_empty()) {
            mask = mask.union(ThemeDependencyMask::EFFECT);
        }
        if let Some(recipes) = &self.recipes {
            mask = mask.union(recipes.affected_mask());
        }
        mask
    }

    /// Apply onto an authoring [`ThemeDefinition`] (used by `ThemePackage`).
    #[must_use]
    pub fn apply_to_definition(&self, base: &ThemeDefinition) -> ThemeDefinition {
        let mut out = base.clone();
        if let Some(appearance) = self.appearance {
            out.appearance = appearance;
        }
        if let Some(accent) = self.accent {
            out.tokens.foundation.accent = accent;
            out.tokens.palette = out.tokens.palette.with_accent_ramp(accent);
        }
        if let Some(colors) = &self.colors {
            for (role, color) in colors.iter() {
                match role {
                    SemanticColorRole::Titlebar => out.tokens.titlebar = Some(color),
                    SemanticColorRole::OnMedia => out.effects.media_foreground = color,
                    SemanticColorRole::ModalScrim => out.effects.modal_scrim = color,
                    other => {
                        let _ = out.tokens.palette.set(other, color);
                    }
                }
            }
            out.tokens.foundation.accent = AccentRamp::from_palette(&out.tokens.palette);
        }
        if let Some(titlebar) = self.titlebar {
            out.tokens.titlebar = titlebar;
        }
        if let Some(metrics) = &self.metrics {
            out.tokens.metrics = metrics.apply(out.tokens.metrics);
        }
        if let Some(spacing) = &self.spacing {
            out.tokens.spacing = spacing.apply(out.tokens.spacing);
        }
        if let Some(border) = &self.border {
            out.tokens.border = border.apply(out.tokens.border);
        }
        if let Some(opacity) = &self.opacity {
            out.tokens.opacity = opacity.apply(out.tokens.opacity);
        }
        if let Some(typography) = &self.typography {
            out.typography = typography.apply(out.typography);
        }
        if let Some(motion) = &self.motion {
            out.motion = motion.apply(out.motion);
        }
        if let Some(effects) = &self.effects {
            out.effects = effects.apply(out.effects);
        }
        if let Some(surfaces) = &self.surfaces {
            out.surfaces = surfaces.apply(out.surfaces);
        }
        if let Some(recipes) = &self.recipes {
            for &(id, recipe) in &recipes.families {
                out.components.families[id.index()] = ComponentRecipeDraft {
                    foreground: Some(recipe.foreground),
                    foreground_checked: recipe.foreground_checked,
                };
            }
            if let Some(button) = &recipes.button {
                button.apply_to_definition(&mut out.components.button);
            }
            if let Some(status) = recipes.status {
                out.components.status = Some(status);
            }
            if let Some(dialog) = recipes.dialog {
                out.components.dialog = Some(dialog);
            }
            if let Some(scrollbar) = recipes.scrollbar {
                out.components.scrollbar = Some(scrollbar);
            }
            if let Some(toast) = recipes.toast {
                out.components.toast = Some(toast);
            }
            if let Some(shape) = recipes.shape {
                out.components.shape = Some(shape);
            }
            if let Some(input) = recipes.input {
                out.components.input = Some(input);
            }
            if let Some(select) = recipes.select {
                out.components.select = Some(select);
            }
            if let Some(tabs) = recipes.tabs {
                out.components.tabs = Some(tabs);
            }
            if let Some(tree_view) = recipes.tree_view {
                out.components.tree_view = Some(tree_view);
            }
            if let Some(menu) = recipes.menu {
                out.components.menu = Some(menu);
            }
        }
        if !self.is_empty() {
            out.generation = out.generation.next();
        }
        out
    }

    /// Apply directly onto a [`CompiledTheme`] and validate with the exact same
    /// fail-closed rules as [`ThemeDefinition::compile`]. Used by `ThemeScope`.
    pub fn apply_to_compiled(
        &self,
        base: &CompiledTheme,
    ) -> Result<CompiledTheme, ThemeCompileError> {
        if self.is_empty() {
            return Ok(base.clone());
        }
        let mut out = base.clone();
        if let Some(appearance) = self.appearance {
            out.appearance = appearance;
            out.style_model.theme_appearance = appearance;
        }
        if let Some(accent) = self.accent {
            out.style_model.palette = out.style_model.palette.with_accent_ramp(accent);
        }
        if let Some(colors) = &self.colors {
            out.style_model = out.style_model.with_color_overrides(colors);
            if let Some(on_media) = colors.get(SemanticColorRole::OnMedia) {
                out.effects.media_foreground = on_media;
            }
            if let Some(modal_scrim) = colors.get(SemanticColorRole::ModalScrim) {
                out.effects.modal_scrim = modal_scrim;
            }
        }
        if let Some(titlebar) = self.titlebar {
            out.style_model.titlebar = titlebar.unwrap_or(out.style_model.palette.surface);
        }
        if let Some(metrics) = &self.metrics {
            out.style_model.metrics = metrics.apply(out.style_model.metrics);
        }
        if let Some(spacing) = &self.spacing {
            out.spacing = spacing.apply(out.spacing);
        }
        if let Some(border) = &self.border {
            out.border = border.apply(out.border);
        }
        if let Some(opacity) = &self.opacity {
            out.style_model.opacity = opacity.apply(out.style_model.opacity);
        }
        if let Some(typography) = &self.typography {
            out.typography = typography.apply(out.typography);
        }
        if let Some(motion) = &self.motion {
            out.motion = motion.apply(out.motion);
        }
        if let Some(effects) = &self.effects {
            out.effects = effects.apply(out.effects);
            out.style_model.on_media = out.effects.media_foreground;
            out.style_model.modal_scrim = out.effects.modal_scrim;
        }
        if let Some(surfaces) = &self.surfaces {
            out.surfaces = surfaces.apply(out.surfaces);
        }
        if let Some(recipes) = &self.recipes {
            for &(id, recipe) in &recipes.families {
                out.recipes.set_family(id, recipe);
            }
            if let Some(button) = &recipes.button {
                let next = button.apply_to_compiled(out.recipes.button());
                out.recipes.set_button(next);
            }
            if let Some(status) = recipes.status {
                out.recipes.set_status(status);
            }
            if let Some(dialog) = recipes.dialog {
                out.recipes.set_dialog(dialog);
            }
            if let Some(scrollbar) = recipes.scrollbar {
                out.recipes.set_scrollbar(scrollbar);
            }
            if let Some(toast) = recipes.toast {
                out.recipes.set_toast(toast);
            }
            if let Some(shape) = recipes.shape {
                out.recipes.set_shape(shape);
            }
            if let Some(input) = recipes.input {
                out.recipes.set_input(input);
            }
            if let Some(select) = recipes.select {
                out.recipes.set_select(select);
            }
            if let Some(tabs) = recipes.tabs {
                out.recipes.set_tabs(tabs);
            }
            if let Some(tree_view) = recipes.tree_view {
                out.recipes.set_tree_view(tree_view);
            }
            if let Some(menu) = recipes.menu {
                out.recipes.set_menu(menu);
            }
        }
        out.recipes.resolve_typography(out.typography);
        out.refresh_chrome_radii();
        out.identity.generation = out.identity.generation.next();
        validate_compiled_theme(&out)?;
        Ok(out)
    }

    /// Human-readable list of overridden sections/keys for DevTools diagnostics.
    pub fn describe_overrides(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(appearance) = self.appearance {
            out.push(("appearance".into(), format!("{appearance:?}")));
        }
        if let Some(accent) = self.accent {
            out.push(("accent".into(), format!("{:?}", accent.base)));
        }
        if let Some(colors) = &self.colors {
            for (role, color) in colors.iter() {
                out.push((
                    format!("colors.{role:?}"),
                    format!(
                        "rgba({:.2}, {:.2}, {:.2}, {:.2})",
                        color.r, color.g, color.b, color.a
                    ),
                ));
            }
        }
        if let Some(titlebar) = self.titlebar {
            out.push(("titlebar".into(), format!("{titlebar:?}")));
        }
        if let Some(metrics) = &self.metrics
            && !metrics.is_empty()
        {
            out.push(("metrics".into(), "metrics patch".into()));
        }
        if let Some(spacing) = &self.spacing
            && !spacing.is_empty()
        {
            out.push(("spacing".into(), "spacing patch".into()));
        }
        if let Some(border) = &self.border
            && !border.is_empty()
        {
            out.push(("border".into(), "border patch".into()));
        }
        if let Some(opacity) = &self.opacity
            && !opacity.is_empty()
        {
            out.push(("opacity".into(), "opacity patch".into()));
        }
        if let Some(typography) = &self.typography
            && !typography.is_empty()
        {
            out.push(("typography".into(), "typography patch".into()));
        }
        if let Some(motion) = &self.motion
            && !motion.is_empty()
        {
            out.push(("motion".into(), "motion patch".into()));
        }
        if let Some(effects) = &self.effects
            && !effects.is_empty()
        {
            out.push(("effects".into(), "effects patch".into()));
        }
        if let Some(surfaces) = &self.surfaces
            && !surfaces.is_empty()
        {
            out.push(("surfaces".into(), "surfaces patch".into()));
        }
        if let Some(recipes) = &self.recipes {
            for &(id, _) in &recipes.families {
                out.push((
                    format!("recipes.families.{}", id.name()),
                    "overridden".into(),
                ));
            }
            if let Some(btn) = &recipes.button {
                for &(kind, _) in &btn.variants {
                    out.push((format!("recipes.button.{kind:?}"), "overridden".into()));
                }
                if let Some(inv) = btn.invalid_border {
                    out.push(("recipes.button.invalid_border".into(), format!("{inv:?}")));
                }
            }
            if recipes.status.is_some() {
                out.push(("recipes.status".into(), "overridden".into()));
            }
            if recipes.dialog.is_some() {
                out.push(("recipes.dialog".into(), "overridden".into()));
            }
            if recipes.scrollbar.is_some() {
                out.push(("recipes.scrollbar".into(), "overridden".into()));
            }
            if recipes.toast.is_some() {
                out.push(("recipes.toast".into(), "overridden".into()));
            }
            if recipes.shape.is_some() {
                out.push(("recipes.shape".into(), "overridden".into()));
            }
            if recipes.input.is_some() {
                out.push(("recipes.input".into(), "overridden".into()));
            }
            if recipes.select.is_some() {
                out.push(("recipes.select".into(), "overridden".into()));
            }
            if recipes.tabs.is_some() {
                out.push(("recipes.tabs".into(), "overridden".into()));
            }
            if recipes.tree_view.is_some() {
                out.push(("recipes.tree_view".into(), "overridden".into()));
            }
            if recipes.menu.is_some() {
                out.push(("recipes.menu".into(), "overridden".into()));
            }
        }
        out
    }
}
