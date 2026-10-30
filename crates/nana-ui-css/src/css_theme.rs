//! Authoring-time parsing of typed theme expressions.
use crate::css_map::{LayoutStyleCss, split_css_space_tokens};
use nana_ui_core::theme::{BorderWidth, LineRole, RadiusTier, SpacingStep, TextWeight, TypeRole};
use nana_ui_core::{LayoutStyle, ThemeExpr, ThemeProperty, ThemeToken};

fn token(name: &str) -> Option<ThemeToken> {
    Some(match name {
        "space-xxs" => ThemeToken::Space(SpacingStep::Xxs),
        "space-xs" => ThemeToken::Space(SpacingStep::Xs),
        "space-sm" => ThemeToken::Space(SpacingStep::Sm),
        "space-md" => ThemeToken::Space(SpacingStep::Md),
        "space-lg" => ThemeToken::Space(SpacingStep::Lg),
        "space-xl" => ThemeToken::Space(SpacingStep::Xl),
        "space-xxl" => ThemeToken::Space(SpacingStep::Xxl),
        "space-xxxl" => ThemeToken::Space(SpacingStep::Xxxl),
        "space-page-tight" => ThemeToken::Space(SpacingStep::PageTight),
        "space-page" => ThemeToken::Space(SpacingStep::Page),
        "type-hint" => ThemeToken::Type(TypeRole::Hint),
        "type-meta" => ThemeToken::Type(TypeRole::Meta),
        "type-body" => ThemeToken::Type(TypeRole::Body),
        "type-section" => ThemeToken::Type(TypeRole::Section),
        "type-card-title" => ThemeToken::Type(TypeRole::CardTitle),
        "type-heading" => ThemeToken::Type(TypeRole::Heading),
        "type-title" => ThemeToken::Type(TypeRole::Title),
        "type-display" => ThemeToken::Type(TypeRole::Display),
        "weight-regular" => ThemeToken::Weight(TextWeight::Regular),
        "weight-medium" => ThemeToken::Weight(TextWeight::Medium),
        "weight-semibold" => ThemeToken::Weight(TextWeight::Semibold),
        "weight-bold" => ThemeToken::Weight(TextWeight::Bold),
        "line-line" => ThemeToken::Line(LineRole::Line),
        "line-tall" => ThemeToken::Line(LineRole::Tall),
        "border-hairline" => ThemeToken::Border(BorderWidth::Hairline),
        "radius-xs" => ThemeToken::Radius(RadiusTier::Xs),
        "radius-sm" => ThemeToken::Radius(RadiusTier::Sm),
        "radius-md" => ThemeToken::Radius(RadiusTier::Md),
        "radius-lg" => ThemeToken::Radius(RadiusTier::Lg),
        "radius-xl" => ThemeToken::Radius(RadiusTier::Xl),
        "radius-xxl" => ThemeToken::Radius(RadiusTier::Xxl),
        "control-small" => ThemeToken::Control(nana_ui_core::ControlSize::Small),
        "control-medium" => ThemeToken::Control(nana_ui_core::ControlSize::Medium),
        "control-large" => ThemeToken::Control(nana_ui_core::ControlSize::Large),
        _ => return None,
    })
}

/// Parse token arithmetic into a linear typed expression. Multiplication and
/// division are allowed only by constants; no CSS strings survive this boundary.
pub fn parse_theme_expr(value: &str) -> Option<ThemeExpr> {
    let mut parser = Parser {
        text: value.trim(),
        cursor: 0,
    };
    let result = parser.sum()?;
    parser.space();
    (parser.cursor == parser.text.len()
        && !result.terms.is_empty()
        && result.constant.is_finite()
        && result.terms.iter().all(|(_, n)| n.is_finite()))
    .then_some(result)
}
struct Parser<'a> {
    text: &'a str,
    cursor: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
    }
    fn take(&mut self, text: &str) -> bool {
        self.space();
        if self.text[self.cursor..].starts_with(text) {
            self.cursor += text.len();
            true
        } else {
            false
        }
    }
    fn sum(&mut self) -> Option<ThemeExpr> {
        let mut value = self.product()?;
        loop {
            if self.take("+") {
                value = value.plus(self.product()?);
            } else if self.take("-") {
                value = value.plus(self.product()?.scaled(-1.0));
            } else {
                return Some(value);
            }
        }
    }
    fn product(&mut self) -> Option<ThemeExpr> {
        let mut value = self.atom()?;
        loop {
            if self.take("*") {
                let right = self.atom()?;
                value = if right.terms.is_empty() {
                    value.scaled(right.constant)
                } else if value.terms.is_empty() {
                    right.scaled(value.constant)
                } else {
                    return None;
                };
            } else if self.take("/") {
                let right = self.atom()?;
                if !right.terms.is_empty() || right.constant == 0.0 {
                    return None;
                }
                value = value.scaled(1.0 / right.constant);
            } else {
                return Some(value);
            }
        }
    }
    fn atom(&mut self) -> Option<ThemeExpr> {
        if self.take("-") {
            return Some(self.atom()?.scaled(-1.0));
        }
        if self.take("+") {
            return self.atom();
        }
        if self.take("theme(") {
            let end = self.text[self.cursor..].find(')')? + self.cursor;
            let result = token(self.text[self.cursor..end].trim())?;
            self.cursor = end + 1;
            return Some(ThemeExpr::token(result));
        }
        if self.take("calc(") || self.take("(") {
            let value = self.sum()?;
            return self.take(")").then_some(value);
        }
        self.space();
        let start = self.cursor;
        while self
            .text
            .as_bytes()
            .get(self.cursor)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.cursor += 1;
        }
        if start == self.cursor {
            return None;
        }
        let value = self.text[start..self.cursor].parse::<f32>().ok()?;
        let _ = self.take("px");
        Some(ThemeExpr::px(value))
    }
}
fn property(key: &str) -> Option<ThemeProperty> {
    Some(match key {
        "gap" => ThemeProperty::Gap,
        "row-gap" => ThemeProperty::RowGap,
        "column-gap" => ThemeProperty::ColumnGap,
        "padding" => ThemeProperty::Padding,
        "padding-top" => ThemeProperty::PaddingTop,
        "padding-right" => ThemeProperty::PaddingRight,
        "padding-bottom" => ThemeProperty::PaddingBottom,
        "padding-left" => ThemeProperty::PaddingLeft,
        "margin" => ThemeProperty::Margin,
        "margin-top" => ThemeProperty::MarginTop,
        "margin-right" => ThemeProperty::MarginRight,
        "margin-bottom" => ThemeProperty::MarginBottom,
        "margin-left" => ThemeProperty::MarginLeft,
        "top" => ThemeProperty::OffsetTop,
        "right" => ThemeProperty::OffsetRight,
        "bottom" => ThemeProperty::OffsetBottom,
        "left" => ThemeProperty::OffsetLeft,
        "width" => ThemeProperty::Width,
        "height" => ThemeProperty::Height,
        "min-width" => ThemeProperty::MinWidth,
        "max-width" => ThemeProperty::MaxWidth,
        "min-height" => ThemeProperty::MinHeight,
        "max-height" => ThemeProperty::MaxHeight,
        "flex-basis" => ThemeProperty::FlexBasis,
        "font-size" => ThemeProperty::FontSize,
        "font-weight" => ThemeProperty::FontWeight,
        "line-height" => ThemeProperty::LineHeight,
        "letter-spacing" => ThemeProperty::LetterSpacing,
        "border-width" => ThemeProperty::BorderWidth,
        "border-top-width" => ThemeProperty::BorderTopWidth,
        "border-right-width" => ThemeProperty::BorderRightWidth,
        "border-bottom-width" => ThemeProperty::BorderBottomWidth,
        "border-left-width" => ThemeProperty::BorderLeftWidth,
        "border-radius" => ThemeProperty::BorderRadius,
        _ => return None,
    })
}
fn group(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "padding" => &[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
        "margin" => &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        "inset" => &["top", "right", "bottom", "left"],
        "border-width" => &[
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
        "gap" => &["row-gap", "column-gap"],
        _ => return None,
    })
}
fn clear(layout: &mut LayoutStyle, key: &str) {
    if let Some(property) = property(key) {
        layout.theme.set(property, None);
    }
    if let Some(group) = group(key) {
        for key in group {
            if let Some(property) = property(key) {
                layout.theme.set(property, None);
            }
        }
    }
}
/// Handle token declarations; literals clear the matching binding before the
/// ordinary parser writes them. Returns true only for a handled token value.
pub(crate) fn apply(layout: &mut LayoutStyle, key: &str, value: &str) -> bool {
    if !value.contains("theme(") {
        // Avoid copying the shared default group for ordinary CSS.
        if !layout.theme.is_empty() {
            clear(layout, key);
        }
        return false;
    }
    let values = split_css_space_tokens(value);
    if let Some(group) = group(key) {
        let count = values.len();
        if count == 0 || count > group.len() {
            return true;
        }
        // Validate the whole shorthand before modifying any field.
        if values
            .iter()
            .any(|v| v.contains("theme(") && parse_theme_expr(v).is_none())
        {
            return true;
        }
        // Reset shorthand residues and previous longhands through the ordinary parser.
        layout.apply_css_property(key, "0px", None, None);
        let indices: Vec<usize> = if group.len() == 2 {
            vec![0, usize::from(count > 1)]
        } else {
            match count {
                1 => vec![0, 0, 0, 0],
                2 => vec![0, 1, 0, 1],
                3 => vec![0, 1, 2, 1],
                _ => vec![0, 1, 2, 3],
            }
        };
        for (key, index) in group.iter().zip(indices) {
            layout.apply_css_property(key, &values[index], None, None);
        }
        return true;
    }
    let Some(property) = property(key) else {
        return false;
    };
    if let Some(expr) = parse_theme_expr(value) {
        let fallback = expr.resolve(nana_ui_core::builtin_theme(
            nana_ui_core::ThemeAppearance::Light,
        ));
        let literal = if key == "font-weight" {
            fallback.to_string()
        } else {
            format!("{fallback}px")
        };
        layout.apply_css_property(key, &literal, None, None);
        layout.theme.set(property, Some(expr));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::{LengthSpec, ThemeAppearance, ThemeDefinition};

    #[test]
    fn arithmetic_and_mixed_shorthands_resolve_from_the_installed_tokens() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "padding: theme(space-md) 0 calc(theme(space-xs) * 2) theme(space-xl); border-width: theme(border-hairline); border-radius: theme(radius-lg); min-height: calc(2 * theme(line-tall) + theme(space-xs) + 4px)",
            None, None,
        );
        let mut definition = ThemeDefinition::NANA_DARK;
        definition.tokens.spacing.md = 14.0;
        definition.tokens.spacing.xs = 7.0;
        definition.tokens.border.hairline = 2.0;
        definition.typography.line_tall = 25.0;
        let theme = definition.compile().unwrap();
        let bindings = layout.theme.clone();
        bindings.apply(&mut layout, &theme);
        assert_eq!(layout.padding_top, Some(LengthSpec::Px(14.0)));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(0.0)));
        assert_eq!(layout.padding_bottom, Some(LengthSpec::Px(14.0)));
        assert_eq!(layout.border_top_width, Some(2.0));
        assert_eq!(layout.border_radius, Some(theme.metrics().radius_lg));
        assert_eq!(layout.min_height, Some(LengthSpec::Px(61.0)));
    }

    #[test]
    fn literal_declarations_record_clearing_even_at_default_values() {
        let written = crate::written_layout(|layout| {
            layout.apply_css_property("padding-left", "0px", None, None);
            layout.apply_css_property("font-size", "13px", None, None);
        });
        assert_eq!(
            written.patch().get("theme.padding_left"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(
            written.patch().get("theme.font_size"),
            Some(&serde_json::Value::Null)
        );
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("gap: theme(space-md); row-gap: 0px", None, None);
        let theme = nana_ui_core::builtin_theme(ThemeAppearance::Dark);
        let bindings = layout.theme.clone();
        bindings.apply(&mut layout, theme);
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(0.0)));
        assert_eq!(layout.column_gap, Some(LengthSpec::Px(8.0)));
        layout.apply_css_text("gap: 0px", None, None);
        assert!(layout.theme.row_gap.is_none());
        assert!(layout.theme.column_gap.is_none());
    }

    #[test]
    fn invalid_or_nonlinear_expressions_are_rejected() {
        for value in [
            "theme(no-such-token)",
            "calc(theme(space-md) * theme(space-xs))",
            "calc(theme(space-md) / 0)",
            "calc(theme(space-md) +)",
        ] {
            assert!(parse_theme_expr(value).is_none(), "{value}");
        }
    }
}
