//! Box model, Flexbox, Box Alignment, Positioning, Logical Edges, Container,
//! Visibility, Opacity, Cursor, Hit-Test, User-Select, Pointer-Events, and
//! Interactivity properties mapped directly from `&[ComponentValue]` to
//! `Result<Vec<StyleOp>, CssDiag>`.
//!
//! Confirmed fixes vs legacy `css_map.rs`:
//! - `display: block | inline-block | flex | inline-flex | grid | inline-grid |
//!   inline | layers | free` ALL emit `StyleOp::Hidden(false)` so `display: none`
//!   is properly undone!
//! - `min-height: 100%` emits ONLY `StyleOp::MinHeight(Some(Percent(100.0)))`
//!   and NEVER mutates `height = Fill`.
//! - Shorthands (`gap`, `padding`, `margin`, `inset`, logical edges, `flex`)
//!   split via `split_whitespace_items` so `calc()`/`min()`/`max()`/`clamp()`
//!   work cleanly without being broken on interior spaces!
//! - Negative `%` / `vh` / `em` / `rem` / `px` are preserved verbatim on
//!   signed properties (`margin*`, `top`/`right`/`bottom`/`left`/`inset*`).
//! - Invalid declarations return `Err(CssDiag)` and NEVER clobber prior values.

use nana_ui_core::{
    AlignSpec, BoxSizing, ContainerType, CursorSpec, DirSpec, DisplaySpec, FlexDirection, FlexWrap,
    HitTestBehavior, JustifySpec, LengthSpec, LogicalEdge, LogicalEdges, OverflowSpec,
    PhysicalEdge, PointerEventsSpec, PositionSpec, StyleOp, UserSelectSpec, VisibilitySpec,
    WindowResizeEdge, WritingModeSpec,
};

use crate::properties::grid::clear_grid_container_ops;
use crate::properties::theme::clear_theme_ops_for_property;
use crate::properties::{CompileContext, DeclState};
use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{ComponentValue, split_whitespace_items, trim_ws};
use crate::syntax::span::Span;
use crate::syntax::tokenizer::Token;
use crate::syntax::values::{parse_length_spec, parse_size_length_spec};

pub(crate) fn parse_layout_property(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    _ctx: &CompileContext,
    state: &mut DeclState,
) -> Option<Result<Vec<StyleOp>, CssDiag>> {
    match prop {
        "display" => Some(compile_display(values, span, state)),
        "flex-direction" => Some(compile_flex_direction(values, span, state)),
        "flex-wrap" => Some(compile_flex_wrap(values, span)),
        "flex-flow" => Some(compile_flex_flow(values, span, state)),
        "flex" => Some(compile_flex_shorthand(values, span)),
        "order" => Some(compile_order(values, span)),
        "flex-grow" => Some(compile_flex_factor(values, span, StyleOp::FlexGrow)),
        "flex-shrink" => Some(compile_flex_factor(values, span, StyleOp::FlexShrink)),
        "flex-basis" => Some(compile_size_prop(values, span, SizeProp::FlexBasis)),
        "gap" => Some(compile_gap_shorthand(values, span)),
        "row-gap" => Some(compile_single_gap(values, span, true)),
        "column-gap" => Some(compile_single_gap(values, span, false)),
        "padding" => Some(compile_box_edge_shorthand(
            values,
            span,
            EdgeKind::Padding,
            state,
        )),
        "padding-top" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Padding,
            PhysicalEdge::Top,
            state,
        )),
        "padding-right" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Padding,
            PhysicalEdge::Right,
            state,
        )),
        "padding-bottom" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Padding,
            PhysicalEdge::Bottom,
            state,
        )),
        "padding-left" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Padding,
            PhysicalEdge::Left,
            state,
        )),
        "padding-inline" => Some(compile_logical_shorthand(
            values,
            span,
            EdgeKind::Padding,
            LogicalEdge::InlineStart,
            LogicalEdge::InlineEnd,
            state,
        )),
        "padding-block" => Some(compile_logical_shorthand(
            values,
            span,
            EdgeKind::Padding,
            LogicalEdge::BlockStart,
            LogicalEdge::BlockEnd,
            state,
        )),
        "padding-inline-start" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Padding,
            LogicalEdge::InlineStart,
            state,
        )),
        "padding-inline-end" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Padding,
            LogicalEdge::InlineEnd,
            state,
        )),
        "padding-block-start" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Padding,
            LogicalEdge::BlockStart,
            state,
        )),
        "padding-block-end" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Padding,
            LogicalEdge::BlockEnd,
            state,
        )),
        "margin" => Some(compile_box_edge_shorthand(
            values,
            span,
            EdgeKind::Margin,
            state,
        )),
        "margin-top" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Margin,
            PhysicalEdge::Top,
            state,
        )),
        "margin-right" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Margin,
            PhysicalEdge::Right,
            state,
        )),
        "margin-bottom" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Margin,
            PhysicalEdge::Bottom,
            state,
        )),
        "margin-left" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Margin,
            PhysicalEdge::Left,
            state,
        )),
        "margin-inline" => Some(compile_logical_shorthand(
            values,
            span,
            EdgeKind::Margin,
            LogicalEdge::InlineStart,
            LogicalEdge::InlineEnd,
            state,
        )),
        "margin-block" => Some(compile_logical_shorthand(
            values,
            span,
            EdgeKind::Margin,
            LogicalEdge::BlockStart,
            LogicalEdge::BlockEnd,
            state,
        )),
        "margin-inline-start" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Margin,
            LogicalEdge::InlineStart,
            state,
        )),
        "margin-inline-end" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Margin,
            LogicalEdge::InlineEnd,
            state,
        )),
        "margin-block-start" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Margin,
            LogicalEdge::BlockStart,
            state,
        )),
        "margin-block-end" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Margin,
            LogicalEdge::BlockEnd,
            state,
        )),
        "inset" => Some(compile_box_edge_shorthand(
            values,
            span,
            EdgeKind::Inset,
            state,
        )),
        "top" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Inset,
            PhysicalEdge::Top,
            state,
        )),
        "right" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Inset,
            PhysicalEdge::Right,
            state,
        )),
        "bottom" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Inset,
            PhysicalEdge::Bottom,
            state,
        )),
        "left" => Some(compile_physical_edge(
            values,
            span,
            EdgeKind::Inset,
            PhysicalEdge::Left,
            state,
        )),
        "inset-inline" => Some(compile_logical_shorthand(
            values,
            span,
            EdgeKind::Inset,
            LogicalEdge::InlineStart,
            LogicalEdge::InlineEnd,
            state,
        )),
        "inset-block" => Some(compile_logical_shorthand(
            values,
            span,
            EdgeKind::Inset,
            LogicalEdge::BlockStart,
            LogicalEdge::BlockEnd,
            state,
        )),
        "inset-inline-start" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Inset,
            LogicalEdge::InlineStart,
            state,
        )),
        "inset-inline-end" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Inset,
            LogicalEdge::InlineEnd,
            state,
        )),
        "inset-block-start" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Inset,
            LogicalEdge::BlockStart,
            state,
        )),
        "inset-block-end" => Some(compile_logical_single(
            values,
            span,
            EdgeKind::Inset,
            LogicalEdge::BlockEnd,
            state,
        )),
        "width" => Some(compile_size_prop(values, span, SizeProp::Width)),
        "height" => Some(compile_size_prop(values, span, SizeProp::Height)),
        "min-width" => Some(compile_size_prop(values, span, SizeProp::MinWidth)),
        "max-width" => Some(compile_size_prop(values, span, SizeProp::MaxWidth)),
        "min-height" => Some(compile_size_prop(values, span, SizeProp::MinHeight)),
        "max-height" => Some(compile_size_prop(values, span, SizeProp::MaxHeight)),
        "align-items" => Some(compile_align_items(values, span, state)),
        "align-self" => Some(compile_align_self(values, span)),
        "align-content" => Some(compile_align_content(values, span)),
        "justify-items" => Some(compile_justify_items(values, span)),
        "justify-self" => Some(compile_justify_self(values, span)),
        "justify-content" => Some(compile_justify_content(values, span)),
        "place-items" => Some(compile_place_items(values, span, state)),
        "place-self" => Some(compile_place_self(values, span)),
        "place-content" => Some(compile_place_content(values, span)),
        "overflow" => Some(compile_overflow(values, span)),
        "overflow-x" => Some(compile_overflow_axis(values, span, true)),
        "overflow-y" => Some(compile_overflow_axis(values, span, false)),
        "box-sizing" => Some(compile_box_sizing(values, span)),
        "position" => Some(compile_position(values, span)),
        "z-index" => Some(compile_z_index(values, span)),
        "isolation" => Some(compile_isolation(values, span)),
        "aspect-ratio" => Some(compile_aspect_ratio(values, span)),
        "direction" => Some(compile_direction(values, span)),
        "writing-mode" => Some(compile_writing_mode(values, span)),
        "unicode-bidi" => Some(Ok(Vec::new())),
        "container-type" => Some(compile_container_type(values, span)),
        "container-name" => Some(compile_container_name(values, span)),
        "container" => Some(compile_container_shorthand(values, span)),
        "visibility" => Some(compile_visibility(values, span)),
        "opacity" => Some(compile_opacity(values, span)),
        "cursor" => Some(compile_cursor(values, span)),
        "hit-test" => Some(compile_hit_test(values, span)),
        "user-select" | "-webkit-user-select" => Some(compile_user_select(values, span)),
        "pointer-events" => Some(compile_pointer_events(values, span)),
        "interactivity" => Some(compile_interactivity(values, span)),
        "-webkit-app-region" | "app-region" => Some(Ok(Vec::new())),
        _ => None,
    }
}

fn compile_display(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `display`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `display`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "none" => {
            state.display = Some(DisplaySpec::None);
            Ok(vec![
                StyleOp::Display(Some(DisplaySpec::None)),
                StyleOp::Hidden(true),
            ])
        }
        "block" | "inline-block" | "flow-root" => {
            state.display = None;
            Ok(vec![StyleOp::Display(None), StyleOp::Hidden(false)])
        }
        "flex" | "inline-flex" => {
            let spec = if kw.eq_ignore_ascii_case("inline-flex") {
                DisplaySpec::InlineFlex
            } else {
                DisplaySpec::Flex
            };
            state.display = Some(spec);
            state.grid_has_columns = false;
            state.grid_has_rows = false;
            let mut ops = vec![StyleOp::Display(Some(spec)), StyleOp::Hidden(false)];
            ops.extend(clear_grid_container_ops());
            if state.direction.is_none() {
                state.direction = Some(FlexDirection::Row);
                ops.push(StyleOp::Direction(Some(FlexDirection::Row)));
            }
            if !state.saw_align_items {
                ops.push(StyleOp::AlignItems(AlignSpec::Stretch));
            }
            Ok(ops)
        }
        "grid" | "inline-grid" => {
            let spec = if kw.eq_ignore_ascii_case("inline-grid") {
                DisplaySpec::InlineGrid
            } else {
                DisplaySpec::Grid
            };
            state.display = Some(spec);
            let mut ops = vec![StyleOp::Display(Some(spec)), StyleOp::Hidden(false)];
            if state.direction.is_none() {
                state.direction = Some(FlexDirection::Row);
                ops.push(StyleOp::Direction(Some(FlexDirection::Row)));
            }
            if !state.saw_align_items {
                ops.push(StyleOp::AlignItems(AlignSpec::Stretch));
            }
            Ok(ops)
        }
        "inline" => {
            state.display = Some(DisplaySpec::Inline);
            Ok(vec![
                StyleOp::Display(Some(DisplaySpec::Inline)),
                StyleOp::Hidden(false),
            ])
        }
        "layers" => {
            state.display = Some(DisplaySpec::Layers);
            let mut ops = vec![
                StyleOp::Display(Some(DisplaySpec::Layers)),
                StyleOp::Hidden(false),
            ];
            if state.direction.is_none() {
                state.direction = Some(FlexDirection::Row);
                ops.push(StyleOp::Direction(Some(FlexDirection::Row)));
            }
            Ok(ops)
        }
        "free" => {
            state.display = Some(DisplaySpec::Free);
            Ok(vec![
                StyleOp::Display(Some(DisplaySpec::Free)),
                StyleOp::Hidden(false),
            ])
        }
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("unsupported `display: {kw}`"),
        )),
    }
}

fn compile_flex_direction(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `flex-direction`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `flex-direction`"))?;
    let (dir, rev) = match kw.to_ascii_lowercase().as_str() {
        "row" => (FlexDirection::Row, false),
        "row-reverse" => (FlexDirection::Row, true),
        "column" => (FlexDirection::Column, false),
        "column-reverse" => (FlexDirection::Column, true),
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `flex-direction: {kw}`"),
            ));
        }
    };
    state.direction = Some(dir);
    Ok(vec![
        StyleOp::Direction(Some(dir)),
        StyleOp::FlexReverse(rev),
    ])
}

fn compile_flex_wrap(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `flex-wrap`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `flex-wrap`"))?;
    let wrap = match kw.to_ascii_lowercase().as_str() {
        "wrap" => FlexWrap::Wrap,
        "wrap-reverse" => FlexWrap::WrapReverse,
        "nowrap" | "initial" | "unset" => FlexWrap::NoWrap,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `flex-wrap: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::FlexWrap(wrap)])
}

fn compile_flex_flow(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > 2 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `flex-flow`",
        ));
    }
    let mut dir: Option<(FlexDirection, bool)> = None;
    let mut wrap: Option<FlexWrap> = None;
    for it in items {
        let [cv] = it else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `flex-flow`",
            ));
        };
        let Some(kw) = cv.as_ident() else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `flex-flow`",
            ));
        };
        match kw.to_ascii_lowercase().as_str() {
            "row" if dir.is_none() => dir = Some((FlexDirection::Row, false)),
            "row-reverse" if dir.is_none() => dir = Some((FlexDirection::Row, true)),
            "column" if dir.is_none() => dir = Some((FlexDirection::Column, false)),
            "column-reverse" if dir.is_none() => dir = Some((FlexDirection::Column, true)),
            "wrap" if wrap.is_none() => wrap = Some(FlexWrap::Wrap),
            "wrap-reverse" if wrap.is_none() => wrap = Some(FlexWrap::WrapReverse),
            "nowrap" if wrap.is_none() => wrap = Some(FlexWrap::NoWrap),
            _ => {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    format!("invalid `flex-flow` token `{kw}`"),
                ));
            }
        }
    }
    let mut ops = Vec::new();
    if let Some((d, rev)) = dir {
        state.direction = Some(d);
        ops.push(StyleOp::Direction(Some(d)));
        ops.push(StyleOp::FlexReverse(rev));
    }
    if let Some(w) = wrap {
        ops.push(StyleOp::FlexWrap(w));
    }
    Ok(ops)
}

fn compile_flex_shorthand(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > 3 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `flex`",
        ));
    }
    if items.len() == 1
        && let [cv] = items[0]
        && let Some(kw) = cv.as_ident()
    {
        return match kw.to_ascii_lowercase().as_str() {
            "none" => Ok(vec![
                StyleOp::FlexGrow(Some(0.0)),
                StyleOp::FlexShrink(Some(0.0)),
                StyleOp::FlexBasis(Some(LengthSpec::Auto)),
                StyleOp::ThemeFlexBasis(None),
            ]),
            "auto" => Ok(vec![
                StyleOp::FlexGrow(Some(1.0)),
                StyleOp::FlexShrink(Some(1.0)),
                StyleOp::FlexBasis(Some(LengthSpec::Auto)),
                StyleOp::ThemeFlexBasis(None),
            ]),
            "initial" => Ok(vec![
                StyleOp::FlexGrow(Some(0.0)),
                StyleOp::FlexShrink(Some(1.0)),
                StyleOp::FlexBasis(Some(LengthSpec::Auto)),
                StyleOp::ThemeFlexBasis(None),
            ]),
            _ => {
                let basis = parse_size_or_fill(items[0]).ok_or_else(|| {
                    CssDiag::error(span, DiagCode::InvalidValue, "invalid `flex` basis")
                })?;
                Ok(vec![
                    StyleOp::FlexGrow(Some(1.0)),
                    StyleOp::FlexShrink(Some(1.0)),
                    StyleOp::FlexBasis(Some(basis)),
                    StyleOp::ThemeFlexBasis(None),
                ])
            }
        };
    }

    let (grow, shrink, basis) = match items.as_slice() {
        [one] => {
            if let Some(g) = parse_non_neg_number(one) {
                (g, 1.0, LengthSpec::Px(0.0))
            } else if let Some(b) = parse_size_or_fill(one) {
                (1.0, 1.0, b)
            } else {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `flex`",
                ));
            }
        }
        [a, b] => {
            if let Some(g) = parse_non_neg_number(a) {
                if let Some(s) = parse_non_neg_number(b) {
                    (g, s, LengthSpec::Px(0.0))
                } else if let Some(basis) = parse_size_or_fill(b) {
                    (g, 1.0, basis)
                } else {
                    return Err(CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        "invalid `flex`",
                    ));
                }
            } else if let Some(basis) = parse_size_or_fill(a)
                && let Some(g) = parse_non_neg_number(b)
            {
                (g, 1.0, basis)
            } else {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `flex`",
                ));
            }
        }
        [a, b, c] => {
            if let (Some(g), Some(s), Some(basis)) = (
                parse_non_neg_number(a),
                parse_non_neg_number(b),
                parse_size_or_fill(c),
            ) {
                (g, s, basis)
            } else if let (Some(basis), Some(g), Some(s)) = (
                parse_size_or_fill(a),
                parse_non_neg_number(b),
                parse_non_neg_number(c),
            ) {
                (g, s, basis)
            } else {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `flex`",
                ));
            }
        }
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `flex`",
            ));
        }
    };

    Ok(vec![
        StyleOp::FlexGrow(Some(grow)),
        StyleOp::FlexShrink(Some(shrink)),
        StyleOp::FlexBasis(Some(basis)),
        StyleOp::ThemeFlexBasis(None),
    ])
}

fn compile_order(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `order`",
        ));
    };
    let Some(Token::Number {
        value,
        int: Some(_),
        ..
    }) = cv.as_token()
    else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `order`",
        ));
    };
    Ok(vec![StyleOp::Order(*value as i32)])
}

fn compile_flex_factor(
    values: &[ComponentValue],
    span: Span,
    ctor: fn(Option<f32>) -> StyleOp,
) -> Result<Vec<StyleOp>, CssDiag> {
    let n = parse_non_neg_number(trim_ws(values))
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid flex factor"))?;
    Ok(vec![ctor(Some(n))])
}

fn parse_non_neg_number(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else { return None };
    match cv.as_token()? {
        Token::Number { value, .. } if value.is_finite() && *value >= 0.0 => Some(*value as f32),
        _ => None,
    }
}

fn compile_gap_shorthand(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    let mut ops = clear_theme_ops_for_property("gap");
    match items.as_slice() {
        [one] => {
            let spec = parse_gap_length(one)
                .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `gap`"))?;
            ops.push(StyleOp::Gap(Some(spec)));
            ops.push(StyleOp::RowGap(None));
            ops.push(StyleOp::ColumnGap(None));
            Ok(ops)
        }
        [row_it, col_it] => {
            let row = parse_gap_length(row_it)
                .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `gap`"))?;
            let col = parse_gap_length(col_it)
                .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `gap`"))?;
            ops.push(StyleOp::Gap(None));
            ops.push(StyleOp::RowGap(Some(row)));
            ops.push(StyleOp::ColumnGap(Some(col)));
            Ok(ops)
        }
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `gap`",
        )),
    }
}

fn compile_single_gap(
    values: &[ComponentValue],
    span: Span,
    is_row: bool,
) -> Result<Vec<StyleOp>, CssDiag> {
    let spec = parse_gap_length(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid gap length"))?;
    Ok(if is_row {
        vec![StyleOp::RowGap(Some(spec)), StyleOp::ThemeRowGap(None)]
    } else {
        vec![
            StyleOp::ColumnGap(Some(spec)),
            StyleOp::ThemeColumnGap(None),
        ]
    })
}

fn parse_gap_length(values: &[ComponentValue]) -> Option<LengthSpec> {
    let trimmed = trim_ws(values);
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("normal"))) {
        return Some(LengthSpec::Px(0.0));
    }
    let spec = parse_length_spec(trimmed)?;
    if is_negative_literal(spec) {
        return None;
    }
    Some(spec)
}

#[derive(Clone, Copy)]
enum EdgeKind {
    Padding,
    Margin,
    Inset,
}

fn parse_edge_value(values: &[ComponentValue], kind: EdgeKind) -> Option<Option<LengthSpec>> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(id) = cv.as_ident()
    {
        if id.eq_ignore_ascii_case("auto") {
            return match kind {
                EdgeKind::Padding => None,
                EdgeKind::Margin => Some(Some(LengthSpec::Auto)),
                EdgeKind::Inset => Some(None),
            };
        }
        return None;
    }
    let spec = parse_length_spec(trimmed)?;
    if matches!(kind, EdgeKind::Padding) && is_negative_literal(spec) {
        return None;
    }
    Some(Some(spec))
}

fn compile_box_edge_shorthand(
    values: &[ComponentValue],
    span: Span,
    kind: EdgeKind,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > 4 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid box edge shorthand",
        ));
    }
    let mut parsed = Vec::with_capacity(items.len());
    for it in items {
        let v = parse_edge_value(it, kind).ok_or_else(|| {
            CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid length in box shorthand",
            )
        })?;
        parsed.push(v);
    }

    let (t, r, b, l) = match parsed.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => unreachable!(),
    };

    let mut ops = match kind {
        EdgeKind::Padding => {
            let mut o = clear_theme_ops_for_property("padding");
            if parsed.len() == 1 {
                o.push(StyleOp::Padding(t));
                o.push(StyleOp::PaddingTop(None));
                o.push(StyleOp::PaddingRight(None));
                o.push(StyleOp::PaddingBottom(None));
                o.push(StyleOp::PaddingLeft(None));
            } else {
                o.push(StyleOp::Padding(None));
                o.push(StyleOp::PaddingTop(t));
                o.push(StyleOp::PaddingRight(r));
                o.push(StyleOp::PaddingBottom(b));
                o.push(StyleOp::PaddingLeft(l));
            }
            o
        }
        EdgeKind::Margin => {
            let mut o = clear_theme_ops_for_property("margin");
            if parsed.len() == 1 {
                o.push(StyleOp::Margin(t));
                o.push(StyleOp::MarginTop(None));
                o.push(StyleOp::MarginRight(None));
                o.push(StyleOp::MarginBottom(None));
                o.push(StyleOp::MarginLeft(None));
            } else {
                o.push(StyleOp::Margin(None));
                o.push(StyleOp::MarginTop(t));
                o.push(StyleOp::MarginRight(r));
                o.push(StyleOp::MarginBottom(b));
                o.push(StyleOp::MarginLeft(l));
            }
            o
        }
        EdgeKind::Inset => {
            let mut o = clear_theme_ops_for_property("inset");
            o.push(StyleOp::OffsetTop(t));
            o.push(StyleOp::OffsetRight(r));
            o.push(StyleOp::OffsetBottom(b));
            o.push(StyleOp::OffsetLeft(l));
            o
        }
    };

    let snapshot = {
        let edges = state.edges_mut(kind);
        edges.set_phys(PhysicalEdge::Top, t);
        edges.set_phys(PhysicalEdge::Right, r);
        edges.set_phys(PhysicalEdge::Bottom, b);
        edges.set_phys(PhysicalEdge::Left, l);
        *edges
    };
    state.mark_edges_dirty(kind);
    ops.extend(emit_logical_edges_ops(kind, &snapshot));
    Ok(ops)
}

fn compile_physical_edge(
    values: &[ComponentValue],
    span: Span,
    kind: EdgeKind,
    edge: PhysicalEdge,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let val = parse_edge_value(values, kind)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid box edge length"))?;
    let mut ops = match (kind, edge) {
        (EdgeKind::Padding, PhysicalEdge::Top) => {
            vec![StyleOp::PaddingTop(val), StyleOp::ThemePaddingTop(None)]
        }
        (EdgeKind::Padding, PhysicalEdge::Right) => {
            vec![StyleOp::PaddingRight(val), StyleOp::ThemePaddingRight(None)]
        }
        (EdgeKind::Padding, PhysicalEdge::Bottom) => vec![
            StyleOp::PaddingBottom(val),
            StyleOp::ThemePaddingBottom(None),
        ],
        (EdgeKind::Padding, PhysicalEdge::Left) => {
            vec![StyleOp::PaddingLeft(val), StyleOp::ThemePaddingLeft(None)]
        }
        (EdgeKind::Margin, PhysicalEdge::Top) => {
            vec![StyleOp::MarginTop(val), StyleOp::ThemeMarginTop(None)]
        }
        (EdgeKind::Margin, PhysicalEdge::Right) => {
            vec![StyleOp::MarginRight(val), StyleOp::ThemeMarginRight(None)]
        }
        (EdgeKind::Margin, PhysicalEdge::Bottom) => {
            vec![StyleOp::MarginBottom(val), StyleOp::ThemeMarginBottom(None)]
        }
        (EdgeKind::Margin, PhysicalEdge::Left) => {
            vec![StyleOp::MarginLeft(val), StyleOp::ThemeMarginLeft(None)]
        }
        (EdgeKind::Inset, PhysicalEdge::Top) => {
            vec![StyleOp::OffsetTop(val), StyleOp::ThemeOffsetTop(None)]
        }
        (EdgeKind::Inset, PhysicalEdge::Right) => {
            vec![StyleOp::OffsetRight(val), StyleOp::ThemeOffsetRight(None)]
        }
        (EdgeKind::Inset, PhysicalEdge::Bottom) => {
            vec![StyleOp::OffsetBottom(val), StyleOp::ThemeOffsetBottom(None)]
        }
        (EdgeKind::Inset, PhysicalEdge::Left) => {
            vec![StyleOp::OffsetLeft(val), StyleOp::ThemeOffsetLeft(None)]
        }
    };
    let snapshot = {
        let edges = state.edges_mut(kind);
        edges.set_phys(edge, val);
        *edges
    };
    state.mark_edges_dirty(kind);
    ops.extend(emit_logical_edges_ops(kind, &snapshot));
    Ok(ops)
}

fn compile_logical_shorthand(
    values: &[ComponentValue],
    span: Span,
    kind: EdgeKind,
    start: LogicalEdge,
    end: LogicalEdge,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    let (v_start, v_end) = match items.as_slice() {
        [one] => {
            let v = parse_edge_value(one, kind).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid logical edge length")
            })?;
            (v, v)
        }
        [a, b] => {
            let va = parse_edge_value(a, kind).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid logical edge length")
            })?;
            let vb = parse_edge_value(b, kind).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid logical edge length")
            })?;
            (va, vb)
        }
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid logical edge shorthand",
            ));
        }
    };
    let snapshot = {
        let edges = state.edges_mut(kind);
        edges.set(start, v_start);
        edges.set(end, v_end);
        *edges
    };
    state.mark_edges_dirty(kind);
    Ok(emit_logical_edges_ops(kind, &snapshot))
}

fn compile_logical_single(
    values: &[ComponentValue],
    span: Span,
    kind: EdgeKind,
    edge: LogicalEdge,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let val = parse_edge_value(values, kind).ok_or_else(|| {
        CssDiag::error(span, DiagCode::InvalidValue, "invalid logical edge length")
    })?;
    let snapshot = {
        let edges = state.edges_mut(kind);
        edges.set(edge, val);
        *edges
    };
    state.mark_edges_dirty(kind);
    Ok(emit_logical_edges_ops(kind, &snapshot))
}

fn emit_logical_edges_ops(kind: EdgeKind, e: &LogicalEdges) -> Vec<StyleOp> {
    match kind {
        EdgeKind::Padding => vec![
            StyleOp::LogicalPaddingInlineStart(e.inline_start),
            StyleOp::LogicalPaddingInlineEnd(e.inline_end),
            StyleOp::LogicalPaddingBlockStart(e.block_start),
            StyleOp::LogicalPaddingBlockEnd(e.block_end),
            StyleOp::LogicalPaddingPhysTop(e.phys_top),
            StyleOp::LogicalPaddingPhysRight(e.phys_right),
            StyleOp::LogicalPaddingPhysBottom(e.phys_bottom),
            StyleOp::LogicalPaddingPhysLeft(e.phys_left),
            StyleOp::LogicalPaddingGenerations(e.generations),
            StyleOp::LogicalPaddingNextGen(e.next_gen),
            StyleOp::LogicalPaddingDerived(e.derived),
        ],
        EdgeKind::Margin => vec![
            StyleOp::LogicalMarginInlineStart(e.inline_start),
            StyleOp::LogicalMarginInlineEnd(e.inline_end),
            StyleOp::LogicalMarginBlockStart(e.block_start),
            StyleOp::LogicalMarginBlockEnd(e.block_end),
            StyleOp::LogicalMarginPhysTop(e.phys_top),
            StyleOp::LogicalMarginPhysRight(e.phys_right),
            StyleOp::LogicalMarginPhysBottom(e.phys_bottom),
            StyleOp::LogicalMarginPhysLeft(e.phys_left),
            StyleOp::LogicalMarginGenerations(e.generations),
            StyleOp::LogicalMarginNextGen(e.next_gen),
            StyleOp::LogicalMarginDerived(e.derived),
        ],
        EdgeKind::Inset => vec![
            StyleOp::LogicalInsetInlineStart(e.inline_start),
            StyleOp::LogicalInsetInlineEnd(e.inline_end),
            StyleOp::LogicalInsetBlockStart(e.block_start),
            StyleOp::LogicalInsetBlockEnd(e.block_end),
            StyleOp::LogicalInsetPhysTop(e.phys_top),
            StyleOp::LogicalInsetPhysRight(e.phys_right),
            StyleOp::LogicalInsetPhysBottom(e.phys_bottom),
            StyleOp::LogicalInsetPhysLeft(e.phys_left),
            StyleOp::LogicalInsetGenerations(e.generations),
            StyleOp::LogicalInsetNextGen(e.next_gen),
            StyleOp::LogicalInsetDerived(e.derived),
        ],
    }
}

impl DeclState {
    fn edges_mut(&mut self, kind: EdgeKind) -> &mut LogicalEdges {
        match kind {
            EdgeKind::Padding => &mut self.logical_edges.padding,
            EdgeKind::Margin => &mut self.logical_edges.margin,
            EdgeKind::Inset => &mut self.logical_edges.inset,
        }
    }

    fn mark_edges_dirty(&mut self, kind: EdgeKind) {
        match kind {
            EdgeKind::Padding => self.dirty_padding_edges = true,
            EdgeKind::Margin => self.dirty_margin_edges = true,
            EdgeKind::Inset => self.dirty_inset_edges = true,
        }
    }
}

#[derive(Clone, Copy)]
enum SizeProp {
    Width,
    Height,
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
    FlexBasis,
}

fn compile_size_prop(
    values: &[ComponentValue],
    span: Span,
    prop: SizeProp,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty size value",
        ));
    }

    // `max-width: none` / `max-height: none`
    if matches!(prop, SizeProp::MaxWidth | SizeProp::MaxHeight)
        && matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none")))
    {
        return Ok(match prop {
            SizeProp::MaxWidth => vec![StyleOp::MaxWidth(None), StyleOp::ThemeMaxWidth(None)],
            SizeProp::MaxHeight => vec![StyleOp::MaxHeight(None), StyleOp::ThemeMaxHeight(None)],
            _ => unreachable!(),
        });
    }

    let spec = match prop {
        SizeProp::Width | SizeProp::Height | SizeProp::FlexBasis => {
            parse_size_or_fill(trimmed)
        }
        SizeProp::MaxWidth => {
            if matches!(trimmed, [cv] if matches!(cv.as_token(), Some(Token::Percentage(p)) if (*p - 100.0).abs() < 1e-6))
            {
                Some(LengthSpec::Fill)
            } else {
                parse_non_neg_size(trimmed)
            }
        }
        // `min-height: 100%` stays `Percent(100.0)` and NEVER mutates `height = Fill`!
        SizeProp::MinWidth | SizeProp::MinHeight | SizeProp::MaxHeight => {
            parse_non_neg_size(trimmed)
        }
    }
    .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid size value"))?;

    Ok(match prop {
        SizeProp::Width => vec![StyleOp::Width(Some(spec)), StyleOp::ThemeWidth(None)],
        SizeProp::Height => vec![StyleOp::Height(Some(spec)), StyleOp::ThemeHeight(None)],
        SizeProp::FlexBasis => vec![
            StyleOp::FlexBasis(Some(spec)),
            StyleOp::ThemeFlexBasis(None),
        ],
        SizeProp::MinWidth => {
            let mut ops = vec![StyleOp::MinWidth(Some(spec)), StyleOp::ThemeMinWidth(None)];
            if matches!(spec, LengthSpec::Px(0.0) | LengthSpec::Percent(0.0)) {
                ops.push(StyleOp::AllowShrink(true));
            }
            ops
        }
        SizeProp::MaxWidth => vec![StyleOp::MaxWidth(Some(spec)), StyleOp::ThemeMaxWidth(None)],
        SizeProp::MinHeight => vec![
            StyleOp::MinHeight(Some(spec)),
            StyleOp::ThemeMinHeight(None),
        ],
        SizeProp::MaxHeight => vec![
            StyleOp::MaxHeight(Some(spec)),
            StyleOp::ThemeMaxHeight(None),
        ],
    })
}

fn parse_size_or_fill(values: &[ComponentValue]) -> Option<LengthSpec> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed {
        if let Some(kw) = cv.as_ident() {
            match kw.to_ascii_lowercase().as_str() {
                "fill" => return Some(LengthSpec::Fill),
                "shrink" => return Some(LengthSpec::Shrink),
                _ => {}
            }
        }
        if let Some(Token::Percentage(p)) = cv.as_token()
            && (*p - 100.0).abs() < 1e-6
        {
            return Some(LengthSpec::Fill);
        }
    }
    parse_non_neg_size(trimmed)
}

fn parse_non_neg_size(values: &[ComponentValue]) -> Option<LengthSpec> {
    let spec = parse_size_length_spec(values)?;
    if is_negative_literal(spec) {
        return None;
    }
    Some(spec)
}

fn is_negative_literal(spec: LengthSpec) -> bool {
    match spec {
        LengthSpec::Px(v)
        | LengthSpec::Percent(v)
        | LengthSpec::Em(v)
        | LengthSpec::Rem(v)
        | LengthSpec::Viewport { value: v, .. } => v < 0.0,
        _ => false,
    }
}

// ============================================================================
// Alignment, Overflow, Positioning, Misc
// ============================================================================

fn strip_alignment_overflow_kw(items: &[&[ComponentValue]]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        if let [cv] = items[i]
            && let Some(id) = cv.as_ident()
        {
            let lower = id.to_ascii_lowercase();
            if lower == "safe" || lower == "unsafe" {
                i += 1;
                continue;
            }
            if (lower == "first" || lower == "last")
                && items.get(i + 1).is_some_and(|next| {
                    matches!(next, [c] if c.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("baseline")))
                })
            {
                out.push("baseline".to_string());
                i += 2;
                continue;
            }
            out.push(lower);
        } else {
            return Vec::new();
        }
        i += 1;
    }
    out
}

fn parse_align_kw(kw: &str) -> Option<AlignSpec> {
    Some(match kw {
        "flex-start" | "start" | "left" | "top" | "self-start" => AlignSpec::Start,
        "baseline" => AlignSpec::Baseline,
        "center" => AlignSpec::Center,
        "flex-end" | "end" | "right" | "bottom" | "self-end" => AlignSpec::End,
        "stretch" | "normal" => AlignSpec::Stretch,
        _ => return None,
    })
}

fn parse_justify_kw(kw: &str) -> Option<JustifySpec> {
    Some(match kw {
        "flex-start" | "start" | "left" | "top" => JustifySpec::Start,
        "center" => JustifySpec::Center,
        "flex-end" | "end" | "right" | "bottom" => JustifySpec::End,
        "space-between" => JustifySpec::SpaceBetween,
        "space-around" => JustifySpec::SpaceAround,
        "space-evenly" => JustifySpec::SpaceEvenly,
        "stretch" | "normal" => JustifySpec::Stretch,
        _ => return None,
    })
}

fn compile_align_items(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    let [kw] = kws.as_slice() else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `align-items`",
        ));
    };
    let a = parse_align_kw(kw)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `align-items`"))?;
    state.saw_align_items = true;
    Ok(vec![StyleOp::AlignItems(a)])
}

fn compile_align_self(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    let [kw] = kws.as_slice() else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `align-self`",
        ));
    };
    if kw == "auto" {
        return Ok(vec![StyleOp::AlignSelf(None)]);
    }
    let a = parse_align_kw(kw)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `align-self`"))?;
    Ok(vec![StyleOp::AlignSelf(Some(a))])
}

fn compile_align_content(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    let [kw] = kws.as_slice() else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `align-content`",
        ));
    };
    let j = parse_justify_kw(kw)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `align-content`"))?;
    Ok(vec![StyleOp::AlignContent(j)])
}

fn compile_justify_items(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    let [kw] = kws.as_slice() else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `justify-items`",
        ));
    };
    if kw == "auto" || kw == "legacy" {
        return Ok(vec![StyleOp::JustifyItems(None)]);
    }
    let a = parse_align_kw(kw)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `justify-items`"))?;
    Ok(vec![StyleOp::JustifyItems(Some(a))])
}

fn compile_justify_self(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    let [kw] = kws.as_slice() else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `justify-self`",
        ));
    };
    if kw == "auto" {
        return Ok(vec![StyleOp::JustifySelf(None)]);
    }
    let a = parse_align_kw(kw)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `justify-self`"))?;
    Ok(vec![StyleOp::JustifySelf(Some(a))])
}

fn compile_justify_content(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    let [kw] = kws.as_slice() else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `justify-content`",
        ));
    };
    let j = parse_justify_kw(kw)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `justify-content`"))?;
    Ok(vec![StyleOp::JustifyContent(j)])
}

fn compile_place_items(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    if kws.is_empty() || kws.len() > 2 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `place-items`",
        ));
    }
    let align = parse_align_kw(&kws[0])
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `place-items`"))?;
    let justify = parse_align_kw(kws.get(1).unwrap_or(&kws[0]))
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `place-items`"))?;
    state.saw_align_items = true;
    Ok(vec![
        StyleOp::AlignItems(align),
        StyleOp::JustifyItems(Some(justify)),
    ])
}

fn compile_place_self(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    if kws.is_empty() || kws.len() > 2 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `place-self`",
        ));
    }
    let align =
        if kws[0] == "auto" {
            None
        } else {
            Some(parse_align_kw(&kws[0]).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid `place-self`")
            })?)
        };
    let second = kws.get(1).unwrap_or(&kws[0]);
    let justify =
        if second == "auto" {
            None
        } else {
            Some(parse_align_kw(second).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid `place-self`")
            })?)
        };
    Ok(vec![
        StyleOp::AlignSelf(align),
        StyleOp::JustifySelf(justify),
    ])
}

fn compile_place_content(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let kws = strip_alignment_overflow_kw(&split_whitespace_items(values));
    if kws.is_empty() || kws.len() > 2 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `place-content`",
        ));
    }
    let align = parse_justify_kw(&kws[0])
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `place-content`"))?;
    let justify = parse_justify_kw(kws.get(1).unwrap_or(&kws[0]))
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `place-content`"))?;
    Ok(vec![
        StyleOp::AlignContent(align),
        StyleOp::JustifyContent(justify),
    ])
}

fn parse_overflow_kw(values: &[ComponentValue]) -> Option<OverflowSpec> {
    let [cv] = trim_ws(values) else { return None };
    Some(match cv.as_ident()?.to_ascii_lowercase().as_str() {
        "visible" => OverflowSpec::Visible,
        "hidden" | "clip" => OverflowSpec::Hidden,
        "auto" => OverflowSpec::Auto,
        "scroll" | "overlay" => OverflowSpec::Scroll,
        _ => return None,
    })
}

fn compile_overflow(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    match items.as_slice() {
        [one] => {
            let o = parse_overflow_kw(one).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid `overflow`")
            })?;
            Ok(vec![StyleOp::OverflowX(o), StyleOp::OverflowY(o)])
        }
        [x, y] => {
            let ox = parse_overflow_kw(x).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid `overflow`")
            })?;
            let oy = parse_overflow_kw(y).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid `overflow`")
            })?;
            Ok(vec![StyleOp::OverflowX(ox), StyleOp::OverflowY(oy)])
        }
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `overflow`",
        )),
    }
}

fn compile_overflow_axis(
    values: &[ComponentValue],
    span: Span,
    is_x: bool,
) -> Result<Vec<StyleOp>, CssDiag> {
    let o = parse_overflow_kw(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `overflow`"))?;
    Ok(vec![if is_x {
        StyleOp::OverflowX(o)
    } else {
        StyleOp::OverflowY(o)
    }])
}

fn compile_box_sizing(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `box-sizing`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `box-sizing`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "border-box" => Ok(vec![StyleOp::BoxSizing(BoxSizing::BorderBox)]),
        "content-box" => Ok(vec![StyleOp::BoxSizing(BoxSizing::ContentBox)]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `box-sizing: {kw}`"),
        )),
    }
}

fn compile_position(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `position`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `position`"))?;
    let p = match kw.to_ascii_lowercase().as_str() {
        "static" => PositionSpec::Static,
        "relative" => PositionSpec::Relative,
        "absolute" => PositionSpec::Absolute,
        "fixed" => PositionSpec::Fixed,
        "sticky" => PositionSpec::Sticky,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `position: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::Position(p)])
}

fn compile_z_index(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `z-index`",
        ));
    };
    match cv.as_token() {
        Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto") => Ok(vec![StyleOp::ZIndex(None)]),
        Some(Token::Number {
            value,
            int: Some(_),
            ..
        }) => Ok(vec![StyleOp::ZIndex(Some(*value as i32))]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `z-index`",
        )),
    }
}

fn compile_isolation(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `isolation`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `isolation`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "isolate" => Ok(vec![StyleOp::Isolation(true)]),
        "auto" | "initial" | "unset" => Ok(vec![StyleOp::Isolation(false)]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `isolation: {kw}`"),
        )),
    }
}

fn compile_aspect_ratio(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `aspect-ratio`",
        ));
    }
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("auto"))
    {
        return Ok(vec![StyleOp::AspectRatio(None)]);
    }
    // Accept `<ratio>`, `auto <ratio>`, or `<ratio> auto`.
    let mut nums = Vec::new();
    let mut saw_slash = false;
    for cv in trimmed {
        if cv.is_whitespace() {
            continue;
        }
        match cv.as_token() {
            Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto") => {}
            Some(Token::Delim('/')) if !saw_slash && nums.len() == 1 => {
                saw_slash = true;
            }
            Some(Token::Number { value, .. }) if value.is_finite() && *value > 0.0 => {
                nums.push(*value as f32);
            }
            _ => {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `aspect-ratio`",
                ));
            }
        }
    }
    let ratio = match nums.as_slice() {
        [w] if !saw_slash => *w,
        [w, h] if saw_slash => *w / *h,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `aspect-ratio`",
            ));
        }
    };
    Ok(vec![StyleOp::AspectRatio(Some(ratio))])
}

fn compile_direction(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `direction`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `direction`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "rtl" => Ok(vec![StyleOp::Dir(Some(DirSpec::Rtl))]),
        "ltr" | "initial" => Ok(vec![StyleOp::Dir(Some(DirSpec::Ltr))]),
        "inherit" | "unset" => Ok(Vec::new()),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `direction: {kw}`"),
        )),
    }
}

fn compile_writing_mode(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `writing-mode`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `writing-mode`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "horizontal-tb" | "initial" | "unset" | "inherit" | "lr-tb" | "lr" => Ok(vec![
            StyleOp::WritingMode(Some(WritingModeSpec::HorizontalTb)),
            StyleOp::UnsupportedWritingMode(false),
        ]),
        "vertical-rl" | "tb-rl" | "tb" => Ok(vec![
            StyleOp::WritingMode(Some(WritingModeSpec::VerticalRl)),
            StyleOp::UnsupportedWritingMode(false),
        ]),
        "vertical-lr" => Ok(vec![
            StyleOp::WritingMode(Some(WritingModeSpec::VerticalLr)),
            StyleOp::UnsupportedWritingMode(false),
        ]),
        "sideways-rl" | "sideways-lr" | "bt" => Ok(vec![StyleOp::UnsupportedWritingMode(true)]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `writing-mode: {kw}`"),
        )),
    }
}

fn compile_container_type(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let ct = parse_container_type_values(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `container-type`"))?;
    Ok(vec![StyleOp::ContainerType(ct)])
}

fn compile_container_name(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let names = parse_container_name_values(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `container-name`"))?;
    Ok(vec![StyleOp::ContainerName(names)])
}

fn compile_container_shorthand(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("initial") || s.eq_ignore_ascii_case("unset"))
    {
        return Ok(vec![
            StyleOp::ContainerName(Vec::new()),
            StyleOp::ContainerType(ContainerType::Normal),
        ]);
    }
    if let Some(slash) = trimmed
        .iter()
        .position(|cv| matches!(cv.as_token(), Some(Token::Delim('/'))))
    {
        let names = parse_container_name_values(&trimmed[..slash]).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `container` name")
        })?;
        let ct = parse_container_type_values(&trimmed[slash + 1..]).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `container` type")
        })?;
        Ok(vec![
            StyleOp::ContainerName(names),
            StyleOp::ContainerType(ct),
        ])
    } else {
        let names = parse_container_name_values(trimmed)
            .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `container`"))?;
        Ok(vec![
            StyleOp::ContainerName(names),
            StyleOp::ContainerType(ContainerType::Normal),
        ])
    }
}

fn parse_container_type_values(values: &[ComponentValue]) -> Option<ContainerType> {
    let items = split_whitespace_items(values);
    if items.len() == 1
        && let [cv] = items[0]
        && let Some(kw) = cv.as_ident()
    {
        match kw.to_ascii_lowercase().as_str() {
            "normal" | "initial" | "unset" => return Some(ContainerType::Normal),
            _ => {}
        }
    }
    let mut size: Option<ContainerType> = None;
    let mut scroll_state = false;
    for it in items {
        let [cv] = it else { return None };
        match cv.as_ident()?.to_ascii_lowercase().as_str() {
            "size" if size.is_none() => size = Some(ContainerType::Size),
            "inline-size" if size.is_none() => size = Some(ContainerType::InlineSize),
            "scroll-state" if !scroll_state => scroll_state = true,
            _ => return None,
        }
    }
    if size.is_none() && !scroll_state {
        return None;
    }
    Some(size.unwrap_or(ContainerType::Normal))
}

fn parse_container_name_values(values: &[ComponentValue]) -> Option<Vec<String>> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return None;
    }
    if items.len() == 1
        && let [cv] = items[0]
        && let Some(kw) = cv.as_ident()
        && matches!(
            kw.to_ascii_lowercase().as_str(),
            "none" | "initial" | "unset"
        )
    {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let [cv] = it else { return None };
        let id = cv.as_ident()?;
        if matches!(
            id.to_ascii_lowercase().as_str(),
            "none"
                | "and"
                | "or"
                | "not"
                | "initial"
                | "inherit"
                | "unset"
                | "revert"
                | "revert-layer"
                | "default"
        ) {
            return None;
        }
        out.push(id.to_string());
    }
    Some(out)
}

fn compile_visibility(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `visibility`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `visibility`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "hidden" => Ok(vec![StyleOp::Visibility(Some(VisibilitySpec::Hidden))]),
        "visible" => Ok(vec![StyleOp::Visibility(Some(VisibilitySpec::Visible))]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `visibility: {kw}`"),
        )),
    }
}

fn compile_opacity(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `opacity`",
        ));
    };
    let v = match cv.as_token() {
        Some(Token::Number { value, .. }) if value.is_finite() => (*value as f32).clamp(0.0, 1.0),
        Some(Token::Percentage(p)) if p.is_finite() => (*p as f32 / 100.0).clamp(0.0, 1.0),
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `opacity`",
            ));
        }
    };
    Ok(vec![StyleOp::Opacity(Some(v))])
}

fn compile_cursor(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `cursor`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `cursor`"))?;
    let c = match kw.to_ascii_lowercase().as_str() {
        "default" | "auto" => CursorSpec::Default,
        "pointer" => CursorSpec::Pointer,
        "text" => CursorSpec::Text,
        "move" => CursorSpec::Move,
        "grab" => CursorSpec::Grab,
        "grabbing" => CursorSpec::Grabbing,
        "not-allowed" => CursorSpec::NotAllowed,
        "crosshair" => CursorSpec::Crosshair,
        "ew-resize" | "col-resize" => CursorSpec::EwResize,
        "ns-resize" | "row-resize" => CursorSpec::NsResize,
        "help" => CursorSpec::Help,
        "wait" => CursorSpec::Wait,
        "progress" => CursorSpec::Progress,
        "zoom-in" => CursorSpec::ZoomIn,
        "zoom-out" => CursorSpec::ZoomOut,
        "none" => CursorSpec::None,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("unsupported `cursor: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::Cursor(Some(c))])
}

fn compile_hit_test(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `hit-test`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `hit-test`"))?;
    let ht = match kw.to_ascii_lowercase().as_str() {
        "auto" => HitTestBehavior::Auto,
        "client" => HitTestBehavior::Client,
        "passthrough" | "pass-through" => HitTestBehavior::PassThrough,
        "drag" => HitTestBehavior::Drag,
        "resize-north" | "resize-n" => HitTestBehavior::Resize(WindowResizeEdge::North),
        "resize-south" | "resize-s" => HitTestBehavior::Resize(WindowResizeEdge::South),
        "resize-east" | "resize-e" => HitTestBehavior::Resize(WindowResizeEdge::East),
        "resize-west" | "resize-w" => HitTestBehavior::Resize(WindowResizeEdge::West),
        "resize-north-east" | "resize-northeast" | "resize-ne" => {
            HitTestBehavior::Resize(WindowResizeEdge::NorthEast)
        }
        "resize-north-west" | "resize-northwest" | "resize-nw" => {
            HitTestBehavior::Resize(WindowResizeEdge::NorthWest)
        }
        "resize-south-east" | "resize-southeast" | "resize-se" => {
            HitTestBehavior::Resize(WindowResizeEdge::SouthEast)
        }
        "resize-south-west" | "resize-southwest" | "resize-sw" => {
            HitTestBehavior::Resize(WindowResizeEdge::SouthWest)
        }
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `hit-test: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::HitTest(Some(ht))])
}

fn compile_user_select(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `user-select`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `user-select`"))?;
    let us = match kw.to_ascii_lowercase().as_str() {
        "auto" => UserSelectSpec::Auto,
        "text" => UserSelectSpec::Text,
        "all" => UserSelectSpec::All,
        "contain" => UserSelectSpec::Contain,
        "none" => UserSelectSpec::None,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `user-select: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::UserSelect(Some(us))])
}

fn compile_pointer_events(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `pointer-events`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `pointer-events`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "inherit" | "unset" => Ok(vec![StyleOp::PointerEvents(None)]),
        "auto" => Ok(vec![StyleOp::PointerEvents(Some(PointerEventsSpec::Auto))]),
        "none" => Ok(vec![StyleOp::PointerEvents(Some(PointerEventsSpec::None))]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `pointer-events: {kw}`"),
        )),
    }
}

fn compile_interactivity(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `interactivity`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `interactivity`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "inert" => Ok(vec![StyleOp::Inert(true)]),
        "auto" | "initial" | "inherit" | "unset" => Ok(vec![StyleOp::Inert(false)]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `interactivity: {kw}`"),
        )),
    }
}
