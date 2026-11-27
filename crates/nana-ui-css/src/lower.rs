//! Pure AST lowering from `crate::syntax::at_rule::*` to backend-neutral
//! `nana_ui_core` contracts (`MediaQueryList`, `FontFaceRule`) and compiled
//! `@container` query intervals.

use std::fmt;

use nana_ui_core::{
    FontFaceRule as CoreFontFaceRule, FontFaceSource as CoreFontFaceSource, FontStyleSpec,
    HotSheetAxis, HotSheetQuery, LengthSpec, MediaColorScheme, MediaComparison,
    MediaCondition as CoreMediaCondition, MediaContrast, MediaFeature as CoreMediaFeature,
    MediaHover, MediaOrientation, MediaPointer, MediaQuery as CoreMediaQuery,
    MediaQueryList as CoreMediaQueryList, MediaReducedMotion, MediaType as CoreMediaType,
};

use crate::syntax::at_rule::{
    CompareOp, ContainerCondition, ContainerFeatureName, ContainerSizeQuery, ContainerValue,
    FontFaceRule as SyntaxFontFaceRule, FontFaceSrcKind, FontFaceStyle,
    MediaCondition as SyntaxMediaCondition, MediaFeature as SyntaxMediaFeature, MediaModifier,
    MediaQuery as SyntaxMediaQuery, MediaQueryList as SyntaxMediaQueryList,
    MediaType as SyntaxMediaType,
};
use crate::syntax::diag::CssDiag;
use crate::syntax::parser::{ComponentValue, trim_ws};
use crate::syntax::tokenizer::{SpannedToken, Token};
use crate::syntax::values::parse_length_spec;

// ============================================================================
// 1. @media -> nana_ui_core::MediaQueryList (strict Kleene 3-valued logic)
// ============================================================================

/// Lower a parsed [`SyntaxMediaQueryList`] into [`nana_ui_core::MediaQueryList`].
pub fn lower_media_query_list(
    list: &SyntaxMediaQueryList,
    diags: &mut Vec<CssDiag>,
) -> CoreMediaQueryList {
    let queries = list.0.iter().map(|q| lower_media_query(q, diags)).collect();
    CoreMediaQueryList { queries }
}

/// Lower one parsed [`SyntaxMediaQuery`] into [`nana_ui_core::MediaQuery`].
pub fn lower_media_query(query: &SyntaxMediaQuery, diags: &mut Vec<CssDiag>) -> CoreMediaQuery {
    match query {
        SyntaxMediaQuery::Invalid => {
            // Per CSS Media Queries 4: a malformed query acts as `not all`
            // (evaluates to `Some(false)`, never matches, never poisons siblings).
            CoreMediaQuery::with_type(false, CoreMediaType::Other, None)
        }
        SyntaxMediaQuery::ConditionOnly(cond) => {
            CoreMediaQuery::from_condition(lower_media_condition(cond, diags))
        }
        SyntaxMediaQuery::TypeAndCondition {
            modifier,
            media_type,
            condition,
        } => {
            let negated = matches!(modifier, Some(MediaModifier::Not));
            let core_type = lower_media_type(media_type);
            let core_cond = condition.as_ref().map(|c| lower_media_condition(c, diags));
            CoreMediaQuery::with_type(negated, core_type, core_cond)
        }
    }
}

fn lower_media_type(mt: &SyntaxMediaType) -> CoreMediaType {
    match mt {
        SyntaxMediaType::All => CoreMediaType::All,
        SyntaxMediaType::Screen => CoreMediaType::Screen,
        SyntaxMediaType::Print => CoreMediaType::Print,
        SyntaxMediaType::Speech | SyntaxMediaType::Unknown(_) => CoreMediaType::Other,
    }
}

/// Lower a parsed [`SyntaxMediaCondition`] into [`nana_ui_core::MediaCondition`].
pub fn lower_media_condition(
    cond: &SyntaxMediaCondition,
    diags: &mut Vec<CssDiag>,
) -> CoreMediaCondition {
    match cond {
        SyntaxMediaCondition::Feature(feat) => lower_media_feature(feat),
        SyntaxMediaCondition::Not(inner) => {
            CoreMediaCondition::negate(lower_media_condition(inner, diags))
        }
        SyntaxMediaCondition::And(items) => CoreMediaCondition::and(
            items
                .iter()
                .map(|c| lower_media_condition(c, diags))
                .collect(),
        ),
        SyntaxMediaCondition::Or(items) => CoreMediaCondition::or(
            items
                .iter()
                .map(|c| lower_media_condition(c, diags))
                .collect(),
        ),
        SyntaxMediaCondition::Unknown(_) => CoreMediaCondition::Unknown,
    }
}

fn lower_compare_op(op: CompareOp) -> MediaComparison {
    match op {
        CompareOp::Lt => MediaComparison::Lt,
        CompareOp::Le => MediaComparison::Le,
        CompareOp::Eq => MediaComparison::Eq,
        CompareOp::Gt => MediaComparison::Gt,
        CompareOp::Ge => MediaComparison::Ge,
    }
}

fn lower_media_feature(feat: &SyntaxMediaFeature) -> CoreMediaCondition {
    match feat {
        SyntaxMediaFeature::Unknown(_) => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
        SyntaxMediaFeature::Plain { name, value } => {
            lower_plain_media_feature(name, value.as_deref())
        }
        SyntaxMediaFeature::Range { name, bounds } => {
            if bounds.is_empty() {
                return CoreMediaCondition::feature(CoreMediaFeature::Unknown);
            }
            let mut conditions = Vec::with_capacity(bounds.len());
            for (op, val) in bounds {
                let cmp = lower_compare_op(*op);
                let lowered = lower_range_media_bound(name, cmp, val);
                conditions.push(CoreMediaCondition::feature(lowered));
            }
            if conditions.len() == 1 {
                conditions.pop().unwrap()
            } else {
                CoreMediaCondition::and(conditions)
            }
        }
    }
}

fn lower_plain_media_feature(name: &str, value: Option<&[ComponentValue]>) -> CoreMediaCondition {
    let lower = name.to_ascii_lowercase();
    match (lower.as_str(), value) {
        // Boolean context (no value)
        ("width", None) => {
            CoreMediaCondition::feature(CoreMediaFeature::Width(MediaComparison::Gt, 0.0))
        }
        ("height", None) => {
            CoreMediaCondition::feature(CoreMediaFeature::Height(MediaComparison::Gt, 0.0))
        }
        ("hover", None) => CoreMediaCondition::feature(CoreMediaFeature::Hover(MediaHover::Hover)),
        ("pointer", None) => CoreMediaCondition::or(vec![
            CoreMediaCondition::feature(CoreMediaFeature::Pointer(MediaPointer::Fine)),
            CoreMediaCondition::feature(CoreMediaFeature::Pointer(MediaPointer::Coarse)),
        ]),
        ("prefers-reduced-motion", None) => CoreMediaCondition::feature(
            CoreMediaFeature::PrefersReducedMotion(MediaReducedMotion::Reduce),
        ),
        ("prefers-contrast", None) => CoreMediaCondition::or(vec![
            CoreMediaCondition::feature(CoreMediaFeature::PrefersContrast(MediaContrast::More)),
            CoreMediaCondition::feature(CoreMediaFeature::PrefersContrast(MediaContrast::Less)),
            CoreMediaCondition::feature(CoreMediaFeature::PrefersContrast(MediaContrast::Custom)),
        ]),

        // Width / height
        ("width", Some(v)) => parse_media_length_px(v)
            .map(|px| CoreMediaCondition::feature(CoreMediaFeature::width(px)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("min-width", Some(v)) => parse_media_length_px(v)
            .map(|px| CoreMediaCondition::feature(CoreMediaFeature::min_width(px)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("max-width", Some(v)) => parse_media_length_px(v)
            .map(|px| CoreMediaCondition::feature(CoreMediaFeature::max_width(px)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("height", Some(v)) => parse_media_length_px(v)
            .map(|px| CoreMediaCondition::feature(CoreMediaFeature::height(px)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("min-height", Some(v)) => parse_media_length_px(v)
            .map(|px| CoreMediaCondition::feature(CoreMediaFeature::min_height(px)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("max-height", Some(v)) => parse_media_length_px(v)
            .map(|px| CoreMediaCondition::feature(CoreMediaFeature::max_height(px)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),

        // Aspect ratio
        ("aspect-ratio", Some(v)) => parse_media_ratio(v)
            .map(|(w, h)| CoreMediaCondition::feature(CoreMediaFeature::aspect_ratio(w, h)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("min-aspect-ratio", Some(v)) => parse_media_ratio(v)
            .map(|(w, h)| CoreMediaCondition::feature(CoreMediaFeature::min_aspect_ratio(w, h)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("max-aspect-ratio", Some(v)) => parse_media_ratio(v)
            .map(|(w, h)| CoreMediaCondition::feature(CoreMediaFeature::max_aspect_ratio(w, h)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),

        // Resolution
        ("resolution", Some(v)) => parse_media_resolution_dppx(v)
            .map(|dppx| CoreMediaCondition::feature(CoreMediaFeature::resolution(dppx)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("min-resolution", Some(v)) => parse_media_resolution_dppx(v)
            .map(|dppx| CoreMediaCondition::feature(CoreMediaFeature::min_resolution(dppx)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),
        ("max-resolution", Some(v)) => parse_media_resolution_dppx(v)
            .map(|dppx| CoreMediaCondition::feature(CoreMediaFeature::max_resolution(dppx)))
            .unwrap_or_else(|| CoreMediaCondition::feature(CoreMediaFeature::Unknown)),

        // Discrete keyword features
        ("orientation", Some(v)) => match single_ident_lower(v).as_deref() {
            Some("portrait") => CoreMediaCondition::feature(CoreMediaFeature::Orientation(
                MediaOrientation::Portrait,
            )),
            Some("landscape") => CoreMediaCondition::feature(CoreMediaFeature::Orientation(
                MediaOrientation::Landscape,
            )),
            _ => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
        },
        ("prefers-color-scheme", Some(v)) => match single_ident_lower(v).as_deref() {
            Some("light") => CoreMediaCondition::feature(CoreMediaFeature::PrefersColorScheme(
                MediaColorScheme::Light,
            )),
            Some("dark") => CoreMediaCondition::feature(CoreMediaFeature::PrefersColorScheme(
                MediaColorScheme::Dark,
            )),
            _ => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
        },
        ("prefers-reduced-motion", Some(v)) => match single_ident_lower(v).as_deref() {
            Some("no-preference") => CoreMediaCondition::feature(
                CoreMediaFeature::PrefersReducedMotion(MediaReducedMotion::NoPreference),
            ),
            Some("reduce") => CoreMediaCondition::feature(CoreMediaFeature::PrefersReducedMotion(
                MediaReducedMotion::Reduce,
            )),
            _ => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
        },
        ("prefers-contrast", Some(v)) => match single_ident_lower(v).as_deref() {
            Some("no-preference") => CoreMediaCondition::feature(
                CoreMediaFeature::PrefersContrast(MediaContrast::NoPreference),
            ),
            Some("more") => {
                CoreMediaCondition::feature(CoreMediaFeature::PrefersContrast(MediaContrast::More))
            }
            Some("less") => {
                CoreMediaCondition::feature(CoreMediaFeature::PrefersContrast(MediaContrast::Less))
            }
            Some("custom") => CoreMediaCondition::feature(CoreMediaFeature::PrefersContrast(
                MediaContrast::Custom,
            )),
            _ => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
        },
        ("hover", Some(v)) => match single_ident_lower(v).as_deref() {
            Some("none") => CoreMediaCondition::feature(CoreMediaFeature::Hover(MediaHover::None)),
            Some("hover") => {
                CoreMediaCondition::feature(CoreMediaFeature::Hover(MediaHover::Hover))
            }
            _ => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
        },
        ("pointer", Some(v)) => match single_ident_lower(v).as_deref() {
            Some("none") => {
                CoreMediaCondition::feature(CoreMediaFeature::Pointer(MediaPointer::None))
            }
            Some("coarse") => {
                CoreMediaCondition::feature(CoreMediaFeature::Pointer(MediaPointer::Coarse))
            }
            Some("fine") => {
                CoreMediaCondition::feature(CoreMediaFeature::Pointer(MediaPointer::Fine))
            }
            _ => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
        },
        _ => CoreMediaCondition::feature(CoreMediaFeature::Unknown),
    }
}

fn lower_range_media_bound(
    name: &str,
    cmp: MediaComparison,
    value: &[ComponentValue],
) -> CoreMediaFeature {
    match name.to_ascii_lowercase().as_str() {
        "width" => parse_media_length_px(value)
            .map(|px| CoreMediaFeature::Width(cmp, px))
            .unwrap_or(CoreMediaFeature::Unknown),
        "height" => parse_media_length_px(value)
            .map(|px| CoreMediaFeature::Height(cmp, px))
            .unwrap_or(CoreMediaFeature::Unknown),
        "aspect-ratio" => parse_media_ratio(value)
            .map(|(w, h)| CoreMediaFeature::AspectRatio(cmp, w, h))
            .unwrap_or(CoreMediaFeature::Unknown),
        "resolution" => parse_media_resolution_dppx(value)
            .map(|dppx| CoreMediaFeature::Resolution(cmp, dppx))
            .unwrap_or(CoreMediaFeature::Unknown),
        _ => CoreMediaFeature::Unknown,
    }
}

fn single_ident_lower(values: &[ComponentValue]) -> Option<String> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    cv.as_ident().map(str::to_ascii_lowercase)
}

/// Resolve CSS media query `<length>` to logical px using the CSS MQ4 initial
/// font size of 16px for `em`/`rem`.
fn parse_media_length_px(values: &[ComponentValue]) -> Option<f32> {
    let spec = parse_length_spec(values)?;
    match spec {
        LengthSpec::Px(v) if v.is_finite() => Some(v),
        LengthSpec::Em(v) | LengthSpec::Rem(v) if v.is_finite() => Some(v * 16.0),
        _ => None,
    }
}

fn parse_media_ratio(values: &[ComponentValue]) -> Option<(f32, f32)> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    if let Some(slash) = trimmed
        .iter()
        .position(|cv| matches!(cv.as_token(), Some(Token::Delim('/'))))
    {
        let a = extract_non_negative_number(trim_ws(&trimmed[..slash]))?;
        let b = extract_non_negative_number(trim_ws(&trimmed[slash + 1..]))?;
        if b <= 0.0 {
            return None;
        }
        return Some((a, b));
    }
    let a = extract_non_negative_number(trimmed)?;
    Some((a, 1.0))
}

fn extract_non_negative_number(values: &[ComponentValue]) -> Option<f32> {
    let [
        ComponentValue::Token(SpannedToken {
            token: Token::Number { value, .. },
            ..
        }),
    ] = values
    else {
        return None;
    };
    (value.is_finite() && *value >= 0.0).then_some(*value as f32)
}

fn parse_media_resolution_dppx(values: &[ComponentValue]) -> Option<f32> {
    let [
        ComponentValue::Token(SpannedToken {
            token: Token::Dimension { value, unit },
            ..
        }),
    ] = trim_ws(values)
    else {
        return None;
    };
    if !value.is_finite() || *value < 0.0 {
        return None;
    }
    let v = *value as f32;
    match unit.to_ascii_lowercase().as_str() {
        "dppx" | "x" => Some(v),
        "dpi" => Some(v / 96.0),
        "dpcm" => Some(v * 2.54 / 96.0),
        _ => None,
    }
}

// ============================================================================
// 2. @font-face -> nana_ui_core::FontFaceRule
// ============================================================================

/// Lower a parsed [`SyntaxFontFaceRule`] into [`nana_ui_core::FontFaceRule`].
///
/// Returns `None` if either `font-family` or `src` is missing/empty (invalid
/// per CSS Fonts 4).
pub fn lower_font_face_rule(
    rule: &SyntaxFontFaceRule,
    base_href: Option<&str>,
) -> Option<CoreFontFaceRule> {
    let family = rule.font_family.as_ref()?;
    if family.is_empty() || rule.src.is_empty() {
        return None;
    }
    let sources = rule
        .src
        .iter()
        .map(|s| match s.kind {
            FontFaceSrcKind::Url => CoreFontFaceSource::Url {
                href: s.value.clone(),
                format: s.format.clone(),
            },
            FontFaceSrcKind::Local => CoreFontFaceSource::Local(s.value.clone()),
        })
        .collect();
    let (weight, weight_end) = match rule.font_weight {
        Some(range) => (
            Some(range.min),
            (range.max != range.min).then_some(range.max),
        ),
        None => (None, None),
    };
    let style = rule.font_style.map(|st| match st {
        FontFaceStyle::Normal => FontStyleSpec::Normal,
        FontFaceStyle::Italic => FontStyleSpec::Italic,
        FontFaceStyle::Oblique { .. } => FontStyleSpec::Oblique,
    });
    Some(CoreFontFaceRule {
        family: family.clone(),
        sources,
        weight,
        weight_end,
        style,
        base_href: base_href.map(str::to_owned),
    })
}

// ============================================================================
// 3. @container -> Compiled ContainerQuery intervals + Breakpoint Planner
// ============================================================================

/// Most breakpoints one element's container rules may use (matches runtime's
/// `MAX_RESPONSIVE_BREAKPOINTS`).
pub const MAX_CONTAINER_BREAKPOINTS: usize = 16;

/// Container axis read by a compiled `@container` query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerAxis {
    Inline,
    Block,
    Width,
    Height,
}

impl ContainerAxis {
    pub const fn to_hot_axis(self) -> HotSheetAxis {
        match self {
            Self::Inline => HotSheetAxis::Inline,
            Self::Block => HotSheetAxis::Block,
            Self::Width => HotSheetAxis::Width,
            Self::Height => HotSheetAxis::Height,
        }
    }
}

/// Half-open interval `lo <= x < hi` on a container axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContainerInterval {
    pub lo: f32,
    pub hi: f32,
}

/// Compiled `@container` query: target container name (or nearest), single axis,
/// and ascending disjoint half-open intervals where it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerQuery {
    pub name: Option<String>,
    pub axis: ContainerAxis,
    pub intervals: Vec<ContainerInterval>,
}

impl ContainerQuery {
    /// Whether this query holds for a container measuring `extent` on its axis.
    pub fn holds_at(&self, extent: f32) -> bool {
        self.intervals
            .iter()
            .any(|interval| interval.lo <= extent && extent < interval.hi)
    }

    /// Convert to [`nana_ui_core::HotSheetQuery`] for dev hot reload.
    pub fn to_hot_sheet_query(&self) -> HotSheetQuery {
        HotSheetQuery {
            name: self.name.clone(),
            axis: self.axis.to_hot_axis(),
            intervals: self.intervals.iter().map(|i| (i.lo, i.hi)).collect(),
        }
    }
}

/// Why an `@container` block cannot be evaluated on a single responsive axis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerQueryUnsupported {
    Invalid,
    ConditionList,
    NoCondition,
    Function(String),
    Feature(String),
    Unit(String),
    MixedAxes,
    Nested,
}

impl fmt::Display for ContainerQueryUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => f.write_str("an invalid condition"),
            Self::ConditionList => f.write_str("a list of conditions"),
            Self::NoCondition => f.write_str("a container name with no condition"),
            Self::Function(name) => write!(f, "`{name}()`"),
            Self::Feature(name) => write!(f, "the `{name}` feature"),
            Self::Unit(unit) => write!(f, "a length in `{unit}` (only `px` is evaluated)"),
            Self::MixedAxes => f.write_str("features on more than one axis"),
            Self::Nested => f.write_str("an `@container` inside another `@container`"),
        }
    }
}

/// Breakpoint plan for all `@container` queries matching one element.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerPlan {
    pub name: Option<String>,
    pub axis: ContainerAxis,
    pub breakpoints: Vec<f32>,
    pub active: Vec<Vec<usize>>,
}

/// Why one element's `@container` rules cannot be combined into a single plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerPlanUnsupported {
    MixedContainers,
    TooManyBreakpoints,
}

/// Cut one element's matching `@container` queries into disjoint extent buckets.
pub fn plan_container_queries<'q>(
    queries: impl IntoIterator<Item = &'q Result<ContainerQuery, ContainerQueryUnsupported>>,
) -> Result<Option<ContainerPlan>, ContainerPlanUnsupported> {
    let queries: Vec<Option<&'q ContainerQuery>> =
        queries.into_iter().map(|q| q.as_ref().ok()).collect();
    let mut supported = queries.iter().copied().flatten();
    let Some(first) = supported.next() else {
        return Ok(None);
    };
    if supported.any(|q| q.name != first.name || q.axis != first.axis) {
        return Err(ContainerPlanUnsupported::MixedContainers);
    }
    let mut breakpoints: Vec<f32> = queries
        .iter()
        .copied()
        .flatten()
        .flat_map(|q| &q.intervals)
        .flat_map(|i| [i.lo, i.hi])
        .filter(|b| b.is_finite())
        .collect();
    breakpoints.sort_by(f32::total_cmp);
    breakpoints.dedup();
    if breakpoints.len() > MAX_CONTAINER_BREAKPOINTS {
        return Err(ContainerPlanUnsupported::TooManyBreakpoints);
    }
    let active = std::iter::once(f32::NEG_INFINITY)
        .chain(breakpoints.iter().copied())
        .map(|lower| {
            queries
                .iter()
                .enumerate()
                .filter(|(_, q)| q.is_some_and(|q| q.holds_at(lower)))
                .map(|(index, _)| index)
                .collect()
        })
        .collect();
    Ok(Some(ContainerPlan {
        name: first.name.clone(),
        axis: first.axis,
        breakpoints,
        active,
    }))
}

/// Lower a parsed [`ContainerCondition`] into compiled [`ContainerQuery`] intervals.
pub fn lower_container_query(
    name: Option<String>,
    condition: &ContainerCondition,
) -> Result<ContainerQuery, ContainerQueryUnsupported> {
    let mut axes = Vec::new();
    let intervals = eval_container_cond(condition, &mut axes)?;
    let axis = *axes.first().ok_or(ContainerQueryUnsupported::Invalid)?;
    if axes.iter().any(|&a| a != axis) {
        return Err(ContainerQueryUnsupported::MixedAxes);
    }
    Ok(ContainerQuery {
        name,
        axis,
        intervals,
    })
}

fn eval_container_cond(
    cond: &ContainerCondition,
    axes: &mut Vec<ContainerAxis>,
) -> Result<Vec<ContainerInterval>, ContainerQueryUnsupported> {
    match cond {
        ContainerCondition::Feature(sq) => eval_container_size_query(sq, axes),
        ContainerCondition::Not(inner) => Ok(complement(&eval_container_cond(inner, axes)?)),
        ContainerCondition::And(items) => {
            let mut iter = items.iter();
            let first = iter.next().ok_or(ContainerQueryUnsupported::Invalid)?;
            let mut acc = eval_container_cond(first, axes)?;
            for item in iter {
                let next = eval_container_cond(item, axes)?;
                acc = intersect(&acc, &next);
            }
            Ok(acc)
        }
        ContainerCondition::Or(items) => {
            let mut iter = items.iter();
            let first = iter.next().ok_or(ContainerQueryUnsupported::Invalid)?;
            let mut acc = eval_container_cond(first, axes)?;
            for item in iter {
                let next = eval_container_cond(item, axes)?;
                acc = union(&acc, &next);
            }
            Ok(acc)
        }
        ContainerCondition::Unknown(cvs) => {
            if let Some(ComponentValue::Function { name, .. }) = cvs.first() {
                return Err(ContainerQueryUnsupported::Function(
                    name.to_ascii_lowercase(),
                ));
            }
            Err(ContainerQueryUnsupported::Invalid)
        }
    }
}

fn eval_container_size_query(
    sq: &ContainerSizeQuery,
    axes: &mut Vec<ContainerAxis>,
) -> Result<Vec<ContainerInterval>, ContainerQueryUnsupported> {
    match sq {
        ContainerSizeQuery::Plain { feature, value } => {
            let axis = container_feature_axis(*feature)?;
            let px = container_value_px(value)?;
            axes.push(axis);
            Ok(compare_interval(CompareOp::Eq, px))
        }
        ContainerSizeQuery::Min { feature, value } => {
            let axis = container_feature_axis(*feature)?;
            let px = container_value_px(value)?;
            axes.push(axis);
            Ok(compare_interval(CompareOp::Ge, px))
        }
        ContainerSizeQuery::Max { feature, value } => {
            let axis = container_feature_axis(*feature)?;
            let px = container_value_px(value)?;
            axes.push(axis);
            Ok(compare_interval(CompareOp::Le, px))
        }
        ContainerSizeQuery::Range { feature, bounds } => {
            let axis = container_feature_axis(*feature)?;
            if bounds.is_empty() {
                return Err(ContainerQueryUnsupported::Invalid);
            }
            axes.push(axis);
            let mut acc = vec![ContainerInterval {
                lo: f32::NEG_INFINITY,
                hi: f32::INFINITY,
            }];
            for (op, val) in bounds {
                let px = container_value_px(val)?;
                acc = intersect(&acc, &compare_interval(*op, px));
            }
            Ok(acc)
        }
        ContainerSizeQuery::Unknown(cvs) => {
            // Handle boolean container size feature like `(width)` / `(inline-size)`.
            if let [cv] = trim_ws(cvs)
                && let Some(ident) = cv.as_ident()
            {
                let axis = match ident.to_ascii_lowercase().as_str() {
                    "width" => ContainerAxis::Width,
                    "height" => ContainerAxis::Height,
                    "inline-size" => ContainerAxis::Inline,
                    "block-size" => ContainerAxis::Block,
                    other => return Err(ContainerQueryUnsupported::Feature(other.to_string())),
                };
                axes.push(axis);
                return Ok(complement(&compare_interval(CompareOp::Eq, 0.0)));
            }
            if let Some(ComponentValue::Function { name, .. }) = trim_ws(cvs).first() {
                return Err(ContainerQueryUnsupported::Function(
                    name.to_ascii_lowercase(),
                ));
            }
            Err(ContainerQueryUnsupported::Invalid)
        }
    }
}

fn container_feature_axis(
    feat: ContainerFeatureName,
) -> Result<ContainerAxis, ContainerQueryUnsupported> {
    match feat {
        ContainerFeatureName::Width => Ok(ContainerAxis::Width),
        ContainerFeatureName::Height => Ok(ContainerAxis::Height),
        ContainerFeatureName::InlineSize => Ok(ContainerAxis::Inline),
        ContainerFeatureName::BlockSize => Ok(ContainerAxis::Block),
        ContainerFeatureName::AspectRatio => {
            Err(ContainerQueryUnsupported::Feature("aspect-ratio".into()))
        }
        ContainerFeatureName::Orientation => {
            Err(ContainerQueryUnsupported::Feature("orientation".into()))
        }
    }
}

fn container_value_px(val: &ContainerValue) -> Result<f32, ContainerQueryUnsupported> {
    match val {
        ContainerValue::Length(LengthSpec::Px(v)) if v.is_finite() => {
            Ok(if *v == 0.0 { 0.0 } else { *v })
        }
        ContainerValue::Length(LengthSpec::Em(_)) => {
            Err(ContainerQueryUnsupported::Unit("em".into()))
        }
        ContainerValue::Length(LengthSpec::Rem(_)) => {
            Err(ContainerQueryUnsupported::Unit("rem".into()))
        }
        ContainerValue::Length(LengthSpec::Percent(_)) => {
            Err(ContainerQueryUnsupported::Unit("%".into()))
        }
        ContainerValue::Length(LengthSpec::Viewport { .. }) => {
            Err(ContainerQueryUnsupported::Unit("viewport".into()))
        }
        ContainerValue::Length(LengthSpec::Calc(_)) => {
            Err(ContainerQueryUnsupported::Function("calc".into()))
        }
        _ => Err(ContainerQueryUnsupported::Invalid),
    }
}

fn span_interval(lo: f32, hi: f32) -> Vec<ContainerInterval> {
    if lo < hi {
        vec![ContainerInterval { lo, hi }]
    } else {
        Vec::new()
    }
}

fn compare_interval(op: CompareOp, px: f32) -> Vec<ContainerInterval> {
    match op {
        CompareOp::Lt => span_interval(f32::NEG_INFINITY, px),
        CompareOp::Le => span_interval(f32::NEG_INFINITY, px.next_up()),
        CompareOp::Gt => span_interval(px.next_up(), f32::INFINITY),
        CompareOp::Ge => span_interval(px, f32::INFINITY),
        CompareOp::Eq => span_interval(px, px.next_up()),
    }
}

fn union(a: &[ContainerInterval], b: &[ContainerInterval]) -> Vec<ContainerInterval> {
    let mut all: Vec<ContainerInterval> = a.iter().chain(b).copied().collect();
    all.sort_by(|x, y| x.lo.total_cmp(&y.lo));
    let mut out: Vec<ContainerInterval> = Vec::with_capacity(all.len());
    for iv in all {
        match out.last_mut() {
            Some(last) if iv.lo <= last.hi => last.hi = last.hi.max(iv.hi),
            _ => out.push(iv),
        }
    }
    out
}

fn intersect(a: &[ContainerInterval], b: &[ContainerInterval]) -> Vec<ContainerInterval> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        out.extend(span_interval(a[i].lo.max(b[j].lo), a[i].hi.min(b[j].hi)));
        if a[i].hi < b[j].hi {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn complement(intervals: &[ContainerInterval]) -> Vec<ContainerInterval> {
    let mut out = Vec::new();
    let mut cursor = f32::NEG_INFINITY;
    for iv in intervals {
        out.extend(span_interval(cursor, iv.lo));
        cursor = cursor.max(iv.hi);
    }
    out.extend(span_interval(cursor, f32::INFINITY));
    out
}
