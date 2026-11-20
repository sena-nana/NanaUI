//! Typed defaults shared by compound controls. Numeric public setters stay literal.
use nana_ui_core::{
    LayoutStyle, LineHeightSpec, LineRole, SpacingStep, TextWeight, ThemeExpr, ThemeToken,
    TypeRole, TypographyTokens,
};

pub(crate) fn space(step: SpacingStep) -> ThemeExpr {
    ThemeToken::Space(step).into()
}

pub(crate) fn font(layout: &mut LayoutStyle, role: TypeRole) {
    layout.font_size = Some(TypographyTokens::DEFAULT.size(role));
    layout.theme.font_size = Some(ThemeToken::Type(role).into());
}

pub(crate) fn weight(layout: &mut LayoutStyle, role: TextWeight) {
    layout.font_weight = Some(TypographyTokens::DEFAULT.weight(role));
    layout.theme.font_weight = Some(ThemeToken::Weight(role).into());
}

pub(crate) fn line(layout: &mut LayoutStyle, role: LineRole) {
    layout.line_height = Some(LineHeightSpec::Absolute(
        TypographyTokens::DEFAULT.line_height(role),
    ));
    layout.theme.line_height = Some(ThemeToken::Line(role).into());
}

pub(crate) fn line_from_type(layout: &mut LayoutStyle, role: TypeRole) {
    layout.line_height = Some(LineHeightSpec::Absolute(
        TypographyTokens::DEFAULT.size(role),
    ));
    layout.theme.line_height = Some(ThemeToken::Type(role).into());
}
