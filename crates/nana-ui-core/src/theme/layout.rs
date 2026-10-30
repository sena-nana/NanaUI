//! Typed theme expressions retained until a theme is installed.
use super::{BorderWidth, CompiledTheme, LineRole, RadiusTier, SpacingStep, TextWeight, TypeRole};
use crate::{LayoutStyle, LengthSpec, LineHeightSpec};
use serde::{Deserialize, Serialize};

/// A typed token reference; no strings are looked up on the runtime path.
/// More tokens may come, so a `match` on one needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ThemeToken {
    Space(SpacingStep),
    Type(TypeRole),
    Weight(TextWeight),
    Line(LineRole),
    Border(BorderWidth),
    Radius(RadiusTier),
    Control(crate::ControlSize),
    ControlPadding(crate::ControlPadding),
    /// A panel's padding across ([`super::SurfacePadding::Panel`] and
    /// `PanelX`, [`super::ThemeMetrics::panel_padding_x`]): what a Card or a
    /// settings group insets its content by, so a scroll area's gutter can
    /// name its card's own padding under any metrics.
    PanelPaddingX,
}
impl ThemeToken {
    pub fn resolve(self, theme: &CompiledTheme) -> f32 {
        match self {
            Self::Space(step) => theme.spacing().resolve(step),
            Self::Type(role) => theme.typography().size(role),
            Self::Weight(role) => theme.typography().weight(role) as f32,
            Self::Line(role) => theme.typography().line_height(role),
            Self::Border(role) => theme.border().resolve(role),
            Self::Radius(tier) => tier.resolve(theme.metrics()),
            Self::Control(size) => size.height_in(theme.metrics()),
            Self::ControlPadding(padding) => padding.resolve(theme.metrics()),
            Self::PanelPaddingX => super::SurfacePadding::Panel.resolve_x(theme.metrics()),
        }
    }
}
/// A linear combination of theme tokens and a logical-pixel offset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThemeExpr {
    pub constant: f32,
    pub terms: Vec<(ThemeToken, f32)>,
}
impl ThemeExpr {
    pub fn token(token: ThemeToken) -> Self {
        Self {
            constant: 0.0,
            terms: vec![(token, 1.0)],
        }
    }
    pub fn px(value: f32) -> Self {
        Self {
            constant: value,
            terms: Vec::new(),
        }
    }
    pub fn scaled(mut self, scale: f32) -> Self {
        self.constant *= scale;
        for (_, coefficient) in &mut self.terms {
            *coefficient *= scale;
        }
        self
    }
    pub fn plus(mut self, other: Self) -> Self {
        self.constant += other.constant;
        self.terms.extend(other.terms);
        self
    }
    pub fn offset(mut self, value: f32) -> Self {
        self.constant += value;
        self
    }
    pub fn resolve(&self, theme: &CompiledTheme) -> f32 {
        self.constant
            + self
                .terms
                .iter()
                .map(|(token, coefficient)| token.resolve(theme) * coefficient)
                .sum::<f32>()
    }
}
impl From<ThemeToken> for ThemeExpr {
    fn from(token: ThemeToken) -> Self {
        Self::token(token)
    }
}
/// Layout fields accepting theme expressions, shared by Rust and compiled CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThemeProperty {
    Gap,
    RowGap,
    ColumnGap,
    Padding,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Margin,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    OffsetTop,
    OffsetRight,
    OffsetBottom,
    OffsetLeft,
    Width,
    Height,
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
    FlexBasis,
    FontSize,
    FontWeight,
    LineHeight,
    LetterSpacing,
    BorderWidth,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderRadius,
}
/// Unresolved layout expressions. Literal CSS declarations clear their matching
/// expression, preserving cascade and responsive override precedence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeLayout {
    pub gap: Option<ThemeExpr>,
    pub row_gap: Option<ThemeExpr>,
    pub column_gap: Option<ThemeExpr>,
    pub padding: Option<ThemeExpr>,
    pub padding_top: Option<ThemeExpr>,
    pub padding_right: Option<ThemeExpr>,
    pub padding_bottom: Option<ThemeExpr>,
    pub padding_left: Option<ThemeExpr>,
    pub margin: Option<ThemeExpr>,
    pub margin_top: Option<ThemeExpr>,
    pub margin_right: Option<ThemeExpr>,
    pub margin_bottom: Option<ThemeExpr>,
    pub margin_left: Option<ThemeExpr>,
    pub offset_top: Option<ThemeExpr>,
    pub offset_right: Option<ThemeExpr>,
    pub offset_bottom: Option<ThemeExpr>,
    pub offset_left: Option<ThemeExpr>,
    pub width: Option<ThemeExpr>,
    pub height: Option<ThemeExpr>,
    pub min_width: Option<ThemeExpr>,
    pub max_width: Option<ThemeExpr>,
    pub min_height: Option<ThemeExpr>,
    pub max_height: Option<ThemeExpr>,
    pub flex_basis: Option<ThemeExpr>,
    pub font_size: Option<ThemeExpr>,
    pub font_weight: Option<ThemeExpr>,
    pub line_height: Option<ThemeExpr>,
    pub letter_spacing: Option<ThemeExpr>,
    pub border_width: Option<ThemeExpr>,
    pub border_top_width: Option<ThemeExpr>,
    pub border_right_width: Option<ThemeExpr>,
    pub border_bottom_width: Option<ThemeExpr>,
    pub border_left_width: Option<ThemeExpr>,
    pub border_radius: Option<ThemeExpr>,
    /// The margin a scroll container's bar runs in
    /// ([`crate::ScrollbarSkin::room`]), resolved onto the node's skin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollbar_room: Option<ThemeExpr>,
}
impl ThemeLayout {
    /// Standard control typography selected by its size step.
    pub fn control_typography(&mut self, size: crate::ControlSize) {
        self.font_size = Some(
            ThemeToken::Type(match size {
                crate::ControlSize::Small => TypeRole::Meta,
                crate::ControlSize::Medium => TypeRole::Body,
                crate::ControlSize::Large => TypeRole::Section,
            })
            .into(),
        );
        self.line_height = Some(
            ThemeToken::Line(match size {
                crate::ControlSize::Small | crate::ControlSize::Medium => LineRole::Line,
                crate::ControlSize::Large => LineRole::Tall,
            })
            .into(),
        );
    }
    pub fn set(&mut self, property: ThemeProperty, value: Option<ThemeExpr>) {
        match property {
            ThemeProperty::Gap => self.gap = value,
            ThemeProperty::RowGap => self.row_gap = value,
            ThemeProperty::ColumnGap => self.column_gap = value,
            ThemeProperty::Padding => self.padding = value,
            ThemeProperty::PaddingTop => self.padding_top = value,
            ThemeProperty::PaddingRight => self.padding_right = value,
            ThemeProperty::PaddingBottom => self.padding_bottom = value,
            ThemeProperty::PaddingLeft => self.padding_left = value,
            ThemeProperty::Margin => self.margin = value,
            ThemeProperty::MarginTop => self.margin_top = value,
            ThemeProperty::MarginRight => self.margin_right = value,
            ThemeProperty::MarginBottom => self.margin_bottom = value,
            ThemeProperty::MarginLeft => self.margin_left = value,
            ThemeProperty::OffsetTop => self.offset_top = value,
            ThemeProperty::OffsetRight => self.offset_right = value,
            ThemeProperty::OffsetBottom => self.offset_bottom = value,
            ThemeProperty::OffsetLeft => self.offset_left = value,
            ThemeProperty::Width => self.width = value,
            ThemeProperty::Height => self.height = value,
            ThemeProperty::MinWidth => self.min_width = value,
            ThemeProperty::MaxWidth => self.max_width = value,
            ThemeProperty::MinHeight => self.min_height = value,
            ThemeProperty::MaxHeight => self.max_height = value,
            ThemeProperty::FlexBasis => self.flex_basis = value,
            ThemeProperty::FontSize => self.font_size = value,
            ThemeProperty::FontWeight => self.font_weight = value,
            ThemeProperty::LineHeight => self.line_height = value,
            ThemeProperty::LetterSpacing => self.letter_spacing = value,
            ThemeProperty::BorderWidth => self.border_width = value,
            ThemeProperty::BorderTopWidth => self.border_top_width = value,
            ThemeProperty::BorderRightWidth => self.border_right_width = value,
            ThemeProperty::BorderBottomWidth => self.border_bottom_width = value,
            ThemeProperty::BorderLeftWidth => self.border_left_width = value,
            ThemeProperty::BorderRadius => self.border_radius = value,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.gap.is_none()
            && self.row_gap.is_none()
            && self.column_gap.is_none()
            && self.padding.is_none()
            && self.padding_top.is_none()
            && self.padding_right.is_none()
            && self.padding_bottom.is_none()
            && self.padding_left.is_none()
            && self.margin.is_none()
            && self.margin_top.is_none()
            && self.margin_right.is_none()
            && self.margin_bottom.is_none()
            && self.margin_left.is_none()
            && self.offset_top.is_none()
            && self.offset_right.is_none()
            && self.offset_bottom.is_none()
            && self.offset_left.is_none()
            && self.width.is_none()
            && self.height.is_none()
            && self.min_width.is_none()
            && self.max_width.is_none()
            && self.min_height.is_none()
            && self.max_height.is_none()
            && self.flex_basis.is_none()
            && self.font_size.is_none()
            && self.font_weight.is_none()
            && self.line_height.is_none()
            && self.letter_spacing.is_none()
            && self.border_width.is_none()
            && self.border_top_width.is_none()
            && self.border_right_width.is_none()
            && self.border_bottom_width.is_none()
            && self.border_left_width.is_none()
            && self.border_radius.is_none()
            && self.scrollbar_room.is_none()
    }
    /// Resolve onto a copy of the authored layout. Expressions stay in the
    /// authored model so later theme installs can resolve them again.
    pub fn apply(&self, layout: &mut LayoutStyle, theme: &CompiledTheme) {
        if let Some(expr) = &self.gap {
            let value = expr.resolve(theme);
            layout.gap = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.row_gap {
            let value = expr.resolve(theme);
            layout.row_gap = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.column_gap {
            let value = expr.resolve(theme);
            layout.column_gap = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.padding {
            let value = expr.resolve(theme);
            layout.padding = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.padding_top {
            let value = expr.resolve(theme);
            layout.padding_top = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.padding_right {
            let value = expr.resolve(theme);
            layout.padding_right = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.padding_bottom {
            let value = expr.resolve(theme);
            layout.padding_bottom = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.padding_left {
            let value = expr.resolve(theme);
            layout.padding_left = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.margin {
            let value = expr.resolve(theme);
            layout.margin = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.margin_top {
            let value = expr.resolve(theme);
            layout.margin_top = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.margin_right {
            let value = expr.resolve(theme);
            layout.margin_right = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.margin_bottom {
            let value = expr.resolve(theme);
            layout.margin_bottom = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.margin_left {
            let value = expr.resolve(theme);
            layout.margin_left = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.offset_top {
            let value = expr.resolve(theme);
            layout.offset_top = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.offset_right {
            let value = expr.resolve(theme);
            layout.offset_right = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.offset_bottom {
            let value = expr.resolve(theme);
            layout.offset_bottom = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.offset_left {
            let value = expr.resolve(theme);
            layout.offset_left = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.width {
            let value = expr.resolve(theme);
            layout.width = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.height {
            let value = expr.resolve(theme);
            layout.height = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.min_width {
            let value = expr.resolve(theme);
            layout.min_width = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.max_width {
            let value = expr.resolve(theme);
            layout.max_width = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.min_height {
            let value = expr.resolve(theme);
            layout.min_height = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.max_height {
            let value = expr.resolve(theme);
            layout.max_height = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.flex_basis {
            let value = expr.resolve(theme);
            layout.flex_basis = Some(LengthSpec::Px(value));
        }
        if let Some(expr) = &self.font_size {
            let value = expr.resolve(theme);
            layout.font_size = Some(value);
        }
        if let Some(expr) = &self.font_weight {
            let value = expr.resolve(theme);
            layout.font_weight = Some(value.round().clamp(1.0, 1000.0) as u16);
        }
        if let Some(expr) = &self.line_height {
            let value = expr.resolve(theme);
            layout.line_height = Some(LineHeightSpec::Absolute(value));
        }
        if let Some(expr) = &self.letter_spacing {
            let value = expr.resolve(theme);
            layout.letter_spacing = Some(value);
        }
        if let Some(expr) = &self.border_width {
            let value = expr.resolve(theme);
            layout.border_width = Some(value);
        }
        if let Some(expr) = &self.border_top_width {
            let value = expr.resolve(theme);
            layout.border_top_width = Some(value);
        }
        if let Some(expr) = &self.border_right_width {
            let value = expr.resolve(theme);
            layout.border_right_width = Some(value);
        }
        if let Some(expr) = &self.border_bottom_width {
            let value = expr.resolve(theme);
            layout.border_bottom_width = Some(value);
        }
        if let Some(expr) = &self.border_left_width {
            let value = expr.resolve(theme);
            layout.border_left_width = Some(value);
        }
        if let Some(expr) = &self.border_radius {
            let value = expr.resolve(theme);
            layout.border_radius = Some(value);
            layout.paint.border_radii = Some([LengthSpec::Px(value); 4]);
        }
        if let Some(expr) = &self.scrollbar_room {
            let value = expr.resolve(theme);
            let skin = layout.paint.scrollbar.unwrap_or_default();
            layout.paint.scrollbar = Some(crate::ScrollbarSkin {
                room: Some(value),
                ..skin
            });
        }
    }
}

crate::shared::shared_default!(ThemeLayout);
