//! CSS Grid container and item properties parsed directly from `&[ComponentValue]`.

use nana_ui_core::shared::Shared;
use nana_ui_core::{
    DisplaySpec, FlexDirection, GridAutoFlow, GridLine, GridRepeatAuto, GridTemplateAreas,
    GridTrack, GridTrackListUnsupported, StyleOp,
};

use crate::properties::{CompileContext, DeclState};
use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{
    BlockKind, ComponentValue, split_comma_groups, split_whitespace_items, trim_ws,
};
use crate::syntax::span::Span;
use crate::syntax::tokenizer::{SpannedToken, Token};
use crate::syntax::values::parse_length_spec;

/// Result of parsing a CSS grid track list (`grid-template-columns` / `rows` /
/// `grid-auto-*`).
#[derive(Debug, Clone, PartialEq)]
pub enum GridTrackListParse {
    Tracks(Vec<GridTrack>),
    RepeatAuto(GridRepeatAuto),
    Subgrid,
    Unsupported(GridTrackListUnsupported),
    Invalid,
}

/// Returns `true` for CSS properties whose values are grid track lists
/// (where an unresolved `var(--x)` degrades to `auto` instead of dropping a slot).
pub fn is_grid_track_property(prop: &str) -> bool {
    matches!(
        prop,
        "grid-template-columns" | "grid-template-rows" | "grid-auto-columns" | "grid-auto-rows"
    )
}

pub(crate) fn parse_grid_property(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Option<Result<Vec<StyleOp>, CssDiag>> {
    match prop {
        "grid-template-columns" => Some(compile_grid_template(values, span, ctx, state, true)),
        "grid-template-rows" => Some(compile_grid_template(values, span, ctx, state, false)),
        "grid-auto-columns" => Some(compile_grid_auto(values, span, ctx, state, true)),
        "grid-auto-rows" => Some(compile_grid_auto(values, span, ctx, state, false)),
        "grid-auto-flow" => Some(compile_grid_auto_flow(values, span, state)),
        "grid-template-areas" => Some(compile_grid_template_areas(values, span, state)),
        "grid-column" => Some(compile_grid_axis_placement(values, span, true)),
        "grid-row" => Some(compile_grid_axis_placement(values, span, false)),
        "grid-column-start" => Some(compile_single_grid_line(
            values,
            span,
            StyleOp::GridColumnStart,
        )),
        "grid-column-end" => Some(compile_single_grid_line(
            values,
            span,
            StyleOp::GridColumnEnd,
        )),
        "grid-row-start" => Some(compile_single_grid_line(
            values,
            span,
            StyleOp::GridRowStart,
        )),
        "grid-row-end" => Some(compile_single_grid_line(values, span, StyleOp::GridRowEnd)),
        "grid-area" => Some(compile_grid_area(values, span)),
        _ => None,
    }
}

fn compile_grid_template(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
    columns: bool,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty grid template track list",
        ));
    }

    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|id| id.eq_ignore_ascii_case("none"))
    {
        if columns {
            state.grid_has_columns = false;
        } else {
            state.grid_has_rows = false;
        }
        let mut ops = clear_axis_ops(columns);
        if let Some(dir) = recompute_grid_direction(state) {
            ops.push(StyleOp::Direction(Some(dir)));
        }
        return Ok(ops);
    }

    if state.display.is_some_and(DisplaySpec::is_flex_container) {
        // Inert under `display: flex`.
        return Ok(Vec::new());
    }

    match parse_grid_track_list(trimmed, ctx) {
        GridTrackListParse::Tracks(tracks) => {
            let line_names = parse_grid_line_names(trimmed);
            if columns {
                state.grid_has_columns = !tracks.is_empty();
            } else {
                state.grid_has_rows = !tracks.is_empty();
            }
            let mut ops = set_axis_ops(columns, Some(tracks), None, None, false, line_names);
            ensure_grid_display_and_dir(state, &mut ops);
            Ok(ops)
        }
        GridTrackListParse::RepeatAuto(mut rep) => {
            attach_repeat_line_name_patterns(&mut rep, trimmed);
            let line_names = parse_grid_line_names(trimmed);
            if columns {
                state.grid_has_columns = true;
            } else {
                state.grid_has_rows = true;
            }
            let mut ops = set_axis_ops(
                columns,
                None,
                None,
                Some(Shared::new(rep)),
                false,
                line_names,
            );
            ensure_grid_display_and_dir(state, &mut ops);
            Ok(ops)
        }
        GridTrackListParse::Subgrid => {
            let line_names = parse_subgrid_line_names(trimmed);
            if columns {
                state.grid_has_columns = true;
            } else {
                state.grid_has_rows = true;
            }
            let mut ops = set_axis_ops(columns, None, None, None, true, line_names);
            ensure_grid_display_and_dir(state, &mut ops);
            Ok(ops)
        }
        GridTrackListParse::Unsupported(unsup) => {
            if columns {
                state.grid_has_columns = false;
            } else {
                state.grid_has_rows = false;
            }
            let mut ops = set_axis_ops(columns, None, Some(unsup), None, false, None);
            ensure_grid_display_and_dir(state, &mut ops);
            Ok(ops)
        }
        GridTrackListParse::Invalid => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid grid template track list",
        )),
    }
}

fn ensure_grid_display_and_dir(state: &mut DeclState, ops: &mut Vec<StyleOp>) {
    if state.display.is_none() {
        state.display = Some(DisplaySpec::Grid);
        ops.push(StyleOp::Display(Some(DisplaySpec::Grid)));
        ops.push(StyleOp::Hidden(false));
    }
    if let Some(dir) = recompute_grid_direction(state) {
        state.direction = Some(dir);
        ops.push(StyleOp::Direction(Some(dir)));
    }
}

fn recompute_grid_direction(state: &DeclState) -> Option<FlexDirection> {
    if state.grid_has_columns {
        Some(FlexDirection::Row)
    } else if state.grid_has_rows {
        Some(FlexDirection::Column)
    } else if state.display.is_some_and(DisplaySpec::is_grid_container) {
        Some(FlexDirection::Row)
    } else {
        None
    }
}

fn clear_axis_ops(columns: bool) -> Vec<StyleOp> {
    set_axis_ops(columns, None, None, None, false, None)
}

fn set_axis_ops(
    columns: bool,
    tracks: Option<Vec<GridTrack>>,
    unsupported: Option<GridTrackListUnsupported>,
    repeat: Option<Shared<GridRepeatAuto>>,
    subgrid: bool,
    line_names: Option<Vec<Vec<String>>>,
) -> Vec<StyleOp> {
    if columns {
        vec![
            StyleOp::GridColumns(tracks),
            StyleOp::GridColumnsUnsupported(unsupported),
            StyleOp::GridColumnsRepeat(repeat),
            StyleOp::GridColumnsSubgrid(subgrid),
            StyleOp::GridColumnLineNames(line_names),
        ]
    } else {
        vec![
            StyleOp::GridRows(tracks),
            StyleOp::GridRowsUnsupported(unsupported),
            StyleOp::GridRowsRepeat(repeat),
            StyleOp::GridRowsSubgrid(subgrid),
            StyleOp::GridRowLineNames(line_names),
        ]
    }
}

/// Emit ops to clear all grid container properties when `display: flex` overrides.
pub(crate) fn clear_grid_container_ops() -> Vec<StyleOp> {
    vec![
        StyleOp::GridColumns(None),
        StyleOp::GridRows(None),
        StyleOp::GridColumnsUnsupported(None),
        StyleOp::GridRowsUnsupported(None),
        StyleOp::GridAutoColumns(None),
        StyleOp::GridAutoRows(None),
        StyleOp::GridAutoFlow(None),
        StyleOp::GridColumnsRepeat(None),
        StyleOp::GridRowsRepeat(None),
        StyleOp::GridColumnsSubgrid(false),
        StyleOp::GridRowsSubgrid(false),
        StyleOp::GridTemplateAreas(None),
        StyleOp::GridColumnLineNames(None),
        StyleOp::GridRowLineNames(None),
    ]
}

fn compile_grid_auto(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &DeclState,
    columns: bool,
) -> Result<Vec<StyleOp>, CssDiag> {
    if state.display.is_some_and(DisplaySpec::is_flex_container) {
        return Ok(Vec::new());
    }
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(id) = cv.as_ident()
    {
        if id.eq_ignore_ascii_case("auto") {
            return Ok(vec![if columns {
                StyleOp::GridAutoColumns(Some(vec![GridTrack::Auto]))
            } else {
                StyleOp::GridAutoRows(Some(vec![GridTrack::Auto]))
            }]);
        }
        if id.eq_ignore_ascii_case("none") {
            return Ok(vec![if columns {
                StyleOp::GridAutoColumns(None)
            } else {
                StyleOp::GridAutoRows(None)
            }]);
        }
    }
    match parse_grid_track_list(trimmed, ctx) {
        GridTrackListParse::Tracks(tracks) => Ok(vec![if columns {
            StyleOp::GridAutoColumns(Some(tracks))
        } else {
            StyleOp::GridAutoRows(Some(tracks))
        }]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `grid-auto-*` track list",
        )),
    }
}

fn compile_grid_auto_flow(
    values: &[ComponentValue],
    span: Span,
    state: &DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    if state.display.is_some_and(DisplaySpec::is_flex_container) {
        return Ok(Vec::new());
    }
    let items = split_whitespace_items(values);
    let mut idents = Vec::with_capacity(items.len());
    for item in items {
        let [cv] = item else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `grid-auto-flow`",
            ));
        };
        let Some(id) = cv.as_ident() else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `grid-auto-flow`",
            ));
        };
        idents.push(id.to_ascii_lowercase());
    }
    let flow = match idents.as_slice() {
        [a] if a == "row" => GridAutoFlow::Row,
        [a] if a == "column" => GridAutoFlow::Column,
        [a] if a == "dense" => GridAutoFlow::RowDense,
        [a, b] if (a == "row" && b == "dense") || (a == "dense" && b == "row") => {
            GridAutoFlow::RowDense
        }
        [a, b] if (a == "column" && b == "dense") || (a == "dense" && b == "column") => {
            GridAutoFlow::ColumnDense
        }
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `grid-auto-flow` value",
            ));
        }
    };
    Ok(vec![StyleOp::GridAutoFlow(Some(flow))])
}

fn compile_grid_template_areas(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    if state.display.is_some_and(DisplaySpec::is_flex_container) {
        return Ok(Vec::new());
    }
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|id| id.eq_ignore_ascii_case("none"))
    {
        return Ok(vec![StyleOp::GridTemplateAreas(None)]);
    }
    let mut cells: Vec<Vec<String>> = Vec::new();
    for cv in trimmed {
        if cv.is_whitespace() {
            continue;
        }
        let Some(Token::String(s)) = cv.as_token() else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "expected quoted row string in `grid-template-areas`",
            ));
        };
        let row: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
        if row.is_empty() {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "empty row in `grid-template-areas`",
            ));
        }
        if let Some(first) = cells.first()
            && row.len() != first.len()
        {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "inconsistent column count in `grid-template-areas`",
            ));
        }
        cells.push(row);
    }
    if cells.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `grid-template-areas`",
        ));
    }
    let mut ops = vec![StyleOp::GridTemplateAreas(Some(GridTemplateAreas {
        cells,
    }))];
    if state.display.is_none() {
        state.display = Some(DisplaySpec::Grid);
        ops.push(StyleOp::Display(Some(DisplaySpec::Grid)));
        ops.push(StyleOp::Hidden(false));
    }
    Ok(ops)
}

fn compile_single_grid_line(
    values: &[ComponentValue],
    span: Span,
    ctor: fn(GridLine) -> StyleOp,
) -> Result<Vec<StyleOp>, CssDiag> {
    let line = parse_grid_line(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid grid line"))?;
    Ok(vec![ctor(line)])
}

fn compile_grid_axis_placement(
    values: &[ComponentValue],
    span: Span,
    columns: bool,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty grid placement",
        ));
    }
    let parts = split_slash_groups(trimmed);
    let (start, end) = match parts.as_slice() {
        [a] => {
            let s = parse_grid_line(a).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid grid placement")
            })?;
            (s, GridLine::Auto)
        }
        [a, b] => {
            let s = parse_grid_line(a).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid grid placement start")
            })?;
            let e = parse_grid_line(b).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid grid placement end")
            })?;
            (s, e)
        }
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid grid placement",
            ));
        }
    };
    Ok(if columns {
        vec![StyleOp::GridColumnStart(start), StyleOp::GridColumnEnd(end)]
    } else {
        vec![StyleOp::GridRowStart(start), StyleOp::GridRowEnd(end)]
    })
}

fn compile_grid_area(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `grid-area`",
        ));
    }
    let parts = split_slash_groups(trimmed);
    if parts.len() == 1
        && let [cv] = parts[0]
        && let Some(ident) = cv.as_ident()
        && !ident.eq_ignore_ascii_case("auto")
        && !ident.eq_ignore_ascii_case("span")
        && !ident.eq_ignore_ascii_case("none")
    {
        return Ok(vec![
            StyleOp::GridArea(Some(ident.to_string())),
            StyleOp::GridRowStart(GridLine::Auto),
            StyleOp::GridColumnStart(GridLine::Auto),
            StyleOp::GridRowEnd(GridLine::Auto),
            StyleOp::GridColumnEnd(GridLine::Auto),
        ]);
    }
    if parts.is_empty() || parts.len() > 4 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `grid-area`",
        ));
    }
    let mut lines = Vec::with_capacity(parts.len());
    for p in parts {
        let l = parse_grid_line(p)
            .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `grid-area`"))?;
        lines.push(l);
    }
    let row_start = lines.first().cloned().unwrap_or(GridLine::Auto);
    let col_start = lines.get(1).cloned().unwrap_or(GridLine::Auto);
    let row_end = lines.get(2).cloned().unwrap_or(GridLine::Auto);
    let col_end = lines.get(3).cloned().unwrap_or(GridLine::Auto);
    Ok(vec![
        StyleOp::GridArea(None),
        StyleOp::GridRowStart(row_start),
        StyleOp::GridColumnStart(col_start),
        StyleOp::GridRowEnd(row_end),
        StyleOp::GridColumnEnd(col_end),
    ])
}

fn split_slash_groups(values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, cv) in values.iter().enumerate() {
        if matches!(cv.as_token(), Some(Token::Delim('/'))) {
            out.push(trim_ws(&values[start..i]));
            start = i + 1;
        }
    }
    out.push(trim_ws(&values[start..]));
    out
}

fn parse_grid_line(values: &[ComponentValue]) -> Option<GridLine> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return None;
    }
    if items.len() == 1 {
        let [cv] = items[0] else { return None };
        return match cv.as_token()? {
            Token::Ident(s) if s.eq_ignore_ascii_case("auto") => Some(GridLine::Auto),
            Token::Ident(s) if s.eq_ignore_ascii_case("span") => Some(GridLine::Span(1)),
            Token::Ident(s) if !s.eq_ignore_ascii_case("none") => Some(GridLine::Name(s.clone())),
            Token::Number {
                value,
                int: Some(_),
                ..
            } => {
                let n = *value as i32;
                (n != 0).then_some(GridLine::Index(n))
            }
            _ => None,
        };
    }

    // Check for `span ...`
    let has_span = items.iter().any(
        |it| matches!(it, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("span"))),
    );
    if has_span {
        let mut count: Option<u16> = None;
        let mut name: Option<String> = None;
        for it in items {
            let [cv] = it else { return None };
            if cv
                .as_ident()
                .is_some_and(|s| s.eq_ignore_ascii_case("span"))
            {
                continue;
            }
            match cv.as_token()? {
                Token::Number {
                    value,
                    int: Some(_),
                    ..
                } if *value >= 1.0 && *value <= f64::from(u16::MAX) => {
                    if count.is_some() {
                        return None;
                    }
                    count = Some(*value as u16);
                }
                Token::Ident(s) => {
                    if name.is_some() {
                        return None;
                    }
                    name = Some(s.clone());
                }
                _ => return None,
            }
        }
        if let Some(n) = count {
            return Some(GridLine::Span(n));
        }
        if let Some(ident) = name {
            return Some(GridLine::Name(ident));
        }
        return Some(GridLine::Span(1));
    }

    // `<custom-ident> <integer>` or `<integer> <custom-ident>`
    if items.len() == 2 {
        let ([a], [b]) = (items[0], items[1]) else {
            return None;
        };
        let (ident, occ) = match (a.as_token()?, b.as_token()?) {
            (
                Token::Ident(s),
                Token::Number {
                    value,
                    int: Some(_),
                    ..
                },
            )
            | (
                Token::Number {
                    value,
                    int: Some(_),
                    ..
                },
                Token::Ident(s),
            ) if *value >= 1.0 && *value <= f64::from(u16::MAX) => (s.clone(), *value as u16),
            _ => return None,
        };
        return Some(if occ == 1 {
            GridLine::Name(ident)
        } else {
            GridLine::NthName(ident, occ)
        });
    }
    None
}

// ============================================================================
// Track list parsing directly on &[ComponentValue]
// ============================================================================

/// Parse a CSS grid track list directly from `&[ComponentValue]`.
pub fn parse_grid_track_list(
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> GridTrackListParse {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return GridTrackListParse::Invalid;
    }

    // Collect top-level non-whitespace items.
    let items: Vec<&ComponentValue> = trimmed.iter().filter(|cv| !cv.is_whitespace()).collect();
    if items.is_empty() {
        return GridTrackListParse::Invalid;
    }

    // Check `subgrid`
    if items[0]
        .as_ident()
        .is_some_and(|id| id.eq_ignore_ascii_case("subgrid"))
    {
        let only_line_names = items[1..].iter().all(|cv| {
            matches!(
                cv,
                ComponentValue::SimpleBlock {
                    kind: BlockKind::Square,
                    ..
                }
            )
        });
        return if only_line_names {
            GridTrackListParse::Subgrid
        } else {
            GridTrackListParse::Unsupported(GridTrackListUnsupported::Subgrid)
        };
    }

    let mut tracks = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let cv = items[i];
        i += 1;

        // Skip `[line-name ...]` bracket blocks (line names parsed separately).
        if matches!(
            cv,
            ComponentValue::SimpleBlock {
                kind: BlockKind::Square,
                ..
            }
        ) {
            continue;
        }

        if cv
            .as_ident()
            .is_some_and(|id| id.eq_ignore_ascii_case("subgrid"))
        {
            return GridTrackListParse::Unsupported(GridTrackListUnsupported::Subgrid);
        }

        if let ComponentValue::Function { name, args, .. } = cv {
            let lower = name.to_ascii_lowercase();
            if lower == "repeat" {
                let groups = split_comma_groups(args);
                if groups.len() != 2 {
                    return GridTrackListParse::Invalid;
                }
                let count_slice = groups[0];
                let pattern_slice = groups[1];
                if contains_nested_repeat(pattern_slice) {
                    return GridTrackListParse::Unsupported(GridTrackListUnsupported::NestedRepeat);
                }

                if let [count_cv] = count_slice
                    && let Some(id) = count_cv.as_ident()
                    && (id.eq_ignore_ascii_case("auto-fit") || id.eq_ignore_ascii_case("auto-fill"))
                {
                    let kind = if id.eq_ignore_ascii_case("auto-fit") {
                        GridTrackListUnsupported::RepeatAutoFit
                    } else {
                        GridTrackListUnsupported::RepeatAutoFill
                    };
                    let unit = match parse_grid_track_list(pattern_slice, ctx) {
                        GridTrackListParse::Tracks(t) if !t.is_empty() => t,
                        GridTrackListParse::RepeatAuto(_) | GridTrackListParse::Unsupported(_) => {
                            return GridTrackListParse::Unsupported(kind);
                        }
                        _ => return GridTrackListParse::Invalid,
                    };
                    // Parse trailing suffix tracks (if any).
                    let suffix_cvs: Vec<ComponentValue> =
                        items[i..].iter().map(|c| (*c).clone()).collect();
                    let suffix = if suffix_cvs.is_empty()
                        || suffix_cvs.iter().all(|c| {
                            matches!(
                                c,
                                ComponentValue::SimpleBlock {
                                    kind: BlockKind::Square,
                                    ..
                                }
                            )
                        }) {
                        Vec::new()
                    } else {
                        match parse_grid_track_list(&suffix_cvs, ctx) {
                            GridTrackListParse::Tracks(t) => t,
                            GridTrackListParse::RepeatAuto(_)
                            | GridTrackListParse::Unsupported(_) => {
                                return GridTrackListParse::Unsupported(kind);
                            }
                            _ => return GridTrackListParse::Invalid,
                        }
                    };
                    return GridTrackListParse::RepeatAuto(GridRepeatAuto {
                        kind,
                        tracks: unit,
                        prefix: tracks,
                        suffix,
                        ..Default::default()
                    });
                }

                let [
                    ComponentValue::Token(SpannedToken {
                        token:
                            Token::Number {
                                value,
                                int: Some(_),
                                ..
                            },
                        ..
                    }),
                ] = count_slice
                else {
                    return GridTrackListParse::Invalid;
                };
                let count = *value as i64;
                if !(1..=64).contains(&count) {
                    return GridTrackListParse::Invalid;
                }
                match parse_grid_track_list(pattern_slice, ctx) {
                    GridTrackListParse::Tracks(unit) if !unit.is_empty() => {
                        for _ in 0..count {
                            tracks.extend(unit.iter().copied());
                        }
                    }
                    other @ (GridTrackListParse::Unsupported(_) | GridTrackListParse::Invalid) => {
                        return other;
                    }
                    GridTrackListParse::RepeatAuto(rep) => {
                        return GridTrackListParse::Unsupported(rep.kind);
                    }
                    _ => return GridTrackListParse::Invalid,
                }
                continue;
            }

            if lower == "fit-content" {
                let inner = trim_ws(args);
                let track = if inner.is_empty()
                    || matches!(inner, [c] if c.as_ident().is_some_and(is_intrinsic_kw))
                {
                    GridTrack::Auto
                } else if let Some(px) = parse_track_px(inner, ctx) {
                    GridTrack::FitContent {
                        limit_px: px.max(0.0),
                    }
                } else {
                    return GridTrackListParse::Invalid;
                };
                tracks.push(track);
                continue;
            }

            if lower == "minmax" {
                let groups = split_comma_groups(args);
                if groups.len() != 2 {
                    return GridTrackListParse::Invalid;
                }
                let min_px = parse_minmax_min(groups[0], ctx);
                let Some(track) = parse_minmax_max(min_px, groups[1], ctx) else {
                    return GridTrackListParse::Invalid;
                };
                tracks.push(track);
                continue;
            }

            if lower == "var" {
                // Unresolved var(--x) in a grid track degrades to Auto so slot count stays intact.
                tracks.push(GridTrack::Auto);
                continue;
            }
        }

        let Some(track) = parse_single_track(cv, ctx) else {
            return GridTrackListParse::Invalid;
        };
        tracks.push(track);
    }

    if tracks.is_empty() {
        GridTrackListParse::Invalid
    } else {
        GridTrackListParse::Tracks(tracks)
    }
}

fn contains_nested_repeat(values: &[ComponentValue]) -> bool {
    values.iter().any(|cv| match cv {
        ComponentValue::Function { name, args, .. } => {
            name.eq_ignore_ascii_case("repeat") || contains_nested_repeat(args)
        }
        _ => false,
    })
}

fn is_intrinsic_kw(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "auto" | "min-content" | "max-content" | "fit-content"
    )
}

fn parse_fr_cv(cv: &ComponentValue) -> Option<f32> {
    if let Some(Token::Dimension { value, unit }) = cv.as_token()
        && unit.eq_ignore_ascii_case("fr")
        && value.is_finite()
        && *value >= 0.0
    {
        return Some(*value as f32);
    }
    None
}

fn parse_track_px(values: &[ComponentValue], ctx: &CompileContext) -> Option<f32> {
    let spec = parse_length_spec(values)?;
    ctx.resolve_length_px(spec).filter(|v| *v >= 0.0)
}

fn parse_minmax_min(values: &[ComponentValue], ctx: &CompileContext) -> f32 {
    let trimmed = trim_ws(values);
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(is_intrinsic_kw)) {
        return 0.0;
    }
    parse_track_px(trimmed, ctx).unwrap_or(0.0)
}

fn parse_minmax_max(
    min_px: f32,
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> Option<GridTrack> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed {
        if let Some(fr) = parse_fr_cv(cv) {
            return Some(GridTrack::MinMax {
                min_px,
                fr,
                max_px: None,
            });
        }
        if cv.as_ident().is_some_and(is_intrinsic_kw) {
            return Some(GridTrack::MinMax {
                min_px,
                fr: 1.0,
                max_px: None,
            });
        }
    }
    if let Some(max_px) = parse_track_px(trimmed, ctx) {
        return Some(GridTrack::MinMax {
            min_px,
            fr: 1.0,
            max_px: Some(max_px.max(min_px)),
        });
    }
    None
}

fn parse_single_track(cv: &ComponentValue, ctx: &CompileContext) -> Option<GridTrack> {
    if let Some(ident) = cv.as_ident()
        && is_intrinsic_kw(ident)
    {
        return Some(GridTrack::Auto);
    }
    if let Some(fr) = parse_fr_cv(cv) {
        return Some(GridTrack::Fr(fr));
    }
    if let Some(Token::Percentage(p)) = cv.as_token()
        && p.is_finite()
        && *p >= 0.0
    {
        return Some(GridTrack::Percent((*p as f32).clamp(0.0, 100.0)));
    }
    let px = parse_track_px(std::slice::from_ref(cv), ctx)?;
    Some(GridTrack::Px(px))
}

// ============================================================================
// Grid line names `[name1 name2]` directly from ComponentValue::SimpleBlock
// ============================================================================

fn extract_bracket_names(inner: &[ComponentValue]) -> Vec<String> {
    inner
        .iter()
        .filter_map(|cv| cv.as_ident().map(str::to_owned))
        .collect()
}

fn parse_grid_line_names(values: &[ComponentValue]) -> Option<Vec<Vec<String>>> {
    let mut names = vec![Vec::new()];
    let mut saw = false;
    for cv in values {
        if cv.is_whitespace() {
            continue;
        }
        if let ComponentValue::SimpleBlock {
            kind: BlockKind::Square,
            values: inner,
            ..
        } = cv
        {
            names
                .last_mut()
                .expect("line table starts with line 1")
                .extend(extract_bracket_names(inner));
            saw = true;
            continue;
        }
        if let ComponentValue::Function { name, args, .. } = cv
            && name.eq_ignore_ascii_case("repeat")
        {
            let groups = split_comma_groups(args);
            if groups.len() == 2
                && let Some(inner_names) = parse_grid_line_names(groups[1])
            {
                let reps = match groups[0] {
                    [
                        ComponentValue::Token(SpannedToken {
                            token:
                                Token::Number {
                                    value,
                                    int: Some(_),
                                    ..
                                },
                            ..
                        }),
                    ] if (*value as i64) >= 1 && (*value as i64) <= 64 => *value as usize,
                    _ => 1,
                };
                let merged = GridRepeatAuto::merge_line_name_pattern(&inner_names, reps);
                names = GridRepeatAuto::join_line_name_lists(&names, &merged);
                saw = true;
            }
            continue;
        }
        names.push(Vec::new());
    }
    saw.then_some(names)
}

fn parse_subgrid_line_names(values: &[ComponentValue]) -> Option<Vec<Vec<String>>> {
    let mut names = Vec::new();
    for cv in values {
        if let ComponentValue::SimpleBlock {
            kind: BlockKind::Square,
            values: inner,
            ..
        } = cv
        {
            names.push(extract_bracket_names(inner));
        }
    }
    names.iter().any(|line| !line.is_empty()).then_some(names)
}

fn attach_repeat_line_name_patterns(rep: &mut GridRepeatAuto, values: &[ComponentValue]) {
    let items: Vec<&ComponentValue> = values.iter().filter(|cv| !cv.is_whitespace()).collect();
    for (idx, cv) in items.iter().enumerate() {
        if let ComponentValue::Function { name, args, .. } = cv
            && name.eq_ignore_ascii_case("repeat")
        {
            let groups = split_comma_groups(args);
            if groups.len() == 2
                && let [count_cv] = groups[0]
                && count_cv.as_ident().is_some_and(|id| {
                    id.eq_ignore_ascii_case("auto-fit") || id.eq_ignore_ascii_case("auto-fill")
                })
            {
                let prefix_cvs: Vec<ComponentValue> =
                    items[..idx].iter().map(|c| (*c).clone()).collect();
                let suffix_cvs: Vec<ComponentValue> =
                    items[idx + 1..].iter().map(|c| (*c).clone()).collect();
                if let Some(names) = parse_grid_line_names(&prefix_cvs) {
                    rep.prefix_line_names = names;
                }
                if let Some(names) = parse_grid_line_names(groups[1]) {
                    rep.pattern_line_names = names;
                }
                if let Some(names) = parse_grid_line_names(&suffix_cvs) {
                    rep.suffix_line_names = names;
                }
                return;
            }
        }
    }
}
