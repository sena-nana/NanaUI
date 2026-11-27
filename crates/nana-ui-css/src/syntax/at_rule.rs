//! Unified at-rule parsers operating directly on [`AtRule`] preludes & blocks:
//! - `@layer` (statements, named/anonymous blocks, deterministic [`LayerRegistry`])
//! - `@keyframes` (keyframe stop lists & declaration blocks)
//! - `@container` (named/unnamed size queries with range & min/max syntax)
//! - `@media` (media types, modifiers, range/plain features, Kleene 3-valued logic)
//! - `@supports` (`and` / `or` / `not`, `(prop: val)`, `selector(...)`)
//! - `@font-face` (`font-family`, `src`, `font-weight` range, `font-style`,
//!   `font-stretch`, `font-display`, `unicode-range`)

use std::cmp::Ordering;
use std::collections::HashMap;

use nana_ui_core::LengthSpec;

use super::MAX_NESTING;
use super::diag::{CssDiag, DiagCode};
use super::parser::{
    AtRule, BlockKind, ComponentValue, Declaration, Rule, parse_declarations_from_values,
    parse_rule_list, span_of_slice, split_comma_groups, split_whitespace_items, trim_ws,
};
use super::span::Span;
use super::tokenizer::{SpannedToken, Token};
use super::values::{parse_angle_token_deg, parse_length_spec};

// ============================================================================
// 1. @layer & LayerRegistry (CSS Cascade 5)
// ============================================================================

/// Dot-separated cascade layer name (e.g. `framework.base` -> `["framework", "base"]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayerName(pub Vec<String>);

impl LayerName {
    /// Join segments with `.`.
    pub fn full_name(&self) -> String {
        self.0.join(".")
    }

    /// Qualify `child` underneath `self` (for nested `@layer`).
    pub fn join(&self, child: &Self) -> Self {
        let mut out = self.0.clone();
        out.extend(child.0.iter().cloned());
        Self(out)
    }
}

/// Parsed prelude of an `@layer` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerPrelude {
    /// `@layer a, b, c;` (no `{}` block).
    Statement(Vec<LayerName>),
    /// `@layer a.b { ... }` or anonymous `@layer { ... }`.
    Block(Option<LayerName>),
}

/// Deterministic layer rank assigned by [`LayerRegistry`].
///
/// Unlayered rules receive [`LayerOrder::UNLAYERED`] (`u32::MAX`), which sorts
/// after all declared layers for normal declarations and before all declared
/// layers when reversed by `!important` per CSS Cascade 5 §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerOrder(pub u32);

impl LayerOrder {
    pub const UNLAYERED: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_unlayered(self) -> bool {
        self.0 == u32::MAX
    }
}

/// Compare two declarations by `(important, layer)` per CSS Cascade 5 §6.1:
/// - Normal layered (ascending layer order)
/// - Normal unlayered
/// - `!important` unlayered
/// - `!important` layered (descending / reversed layer order — first layer wins!)
#[inline]
pub fn compare_layer_cascade(
    a_important: bool,
    a_layer: LayerOrder,
    b_important: bool,
    b_layer: LayerOrder,
) -> Ordering {
    match (a_important, b_important) {
        (false, false) => a_layer.cmp(&b_layer),
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (true, true) => b_layer.cmp(&a_layer),
    }
}

/// Registry tracking CSS `@layer` declaration order.
#[derive(Debug, Clone, Default)]
pub struct LayerRegistry {
    orders: HashMap<String, LayerOrder>,
    next_id: u32,
}

impl LayerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a named layer (idempotent: first declaration fixes its rank).
    pub fn declare(&mut self, name: &LayerName) -> LayerOrder {
        let key = name.full_name();
        if let Some(&existing) = self.orders.get(&key) {
            return existing;
        }
        let order = self.alloc_order();
        self.orders.insert(key, order);
        order
    }

    /// Declare a fresh anonymous `@layer { ... }` block with a unique rank.
    pub fn declare_anonymous(&mut self) -> LayerOrder {
        self.alloc_order()
    }

    /// Look up an already-declared layer's order.
    pub fn order_of(&self, name: &LayerName) -> Option<LayerOrder> {
        self.orders.get(&name.full_name()).copied()
    }

    /// Compare two declarations by `(important, layer)` per CSS Cascade 5.
    #[inline]
    pub fn compare_cascade(
        a_important: bool,
        a_layer: LayerOrder,
        b_important: bool,
        b_layer: LayerOrder,
    ) -> Ordering {
        compare_layer_cascade(a_important, a_layer, b_important, b_layer)
    }

    fn alloc_order(&mut self) -> LayerOrder {
        let id = self.next_id;
        // Reserve u32::MAX for UNLAYERED.
        if self.next_id < u32::MAX - 1 {
            self.next_id += 1;
        }
        LayerOrder(id)
    }
}

/// Parse an `@layer` at-rule prelude (validating statement vs block form).
pub fn parse_layer_prelude(at: &AtRule, diags: &mut Vec<CssDiag>) -> Option<LayerPrelude> {
    let prelude = trim_ws(&at.prelude);
    if at.block.is_some() {
        if prelude.is_empty() {
            return Some(LayerPrelude::Block(None));
        }
        let name = parse_layer_name(prelude, at.prelude_span, diags)?;
        Some(LayerPrelude::Block(Some(name)))
    } else {
        if prelude.is_empty() {
            diags.push(CssDiag::error(
                at.span,
                DiagCode::InvalidAtRule,
                "@layer statement requires at least one layer name",
            ));
            return None;
        }
        let groups = split_comma_groups(prelude);
        let mut names = Vec::with_capacity(groups.len());
        for group in groups {
            let span = span_of_slice(group).unwrap_or(at.prelude_span);
            let name = parse_layer_name(group, span, diags)?;
            names.push(name);
        }
        Some(LayerPrelude::Statement(names))
    }
}

fn parse_layer_name(
    values: &[ComponentValue],
    span: Span,
    diags: &mut Vec<CssDiag>,
) -> Option<LayerName> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        diags.push(CssDiag::error(
            span,
            DiagCode::InvalidAtRule,
            "empty layer name in @layer",
        ));
        return None;
    }
    let mut segments = Vec::new();
    let mut expect_ident = true;
    for cv in trimmed {
        if cv.is_whitespace() {
            diags.push(CssDiag::error(
                cv.span(),
                DiagCode::InvalidAtRule,
                "whitespace is not allowed inside a dotted @layer name",
            ));
            return None;
        }
        if expect_ident {
            let Some(ident) = cv.as_ident() else {
                diags.push(CssDiag::error(
                    cv.span(),
                    DiagCode::InvalidAtRule,
                    "expected identifier in @layer name",
                ));
                return None;
            };
            segments.push(ident.to_string());
            expect_ident = false;
        } else {
            if !matches!(cv.as_token(), Some(Token::Delim('.'))) {
                diags.push(CssDiag::error(
                    cv.span(),
                    DiagCode::InvalidAtRule,
                    "expected `.` between @layer name segments",
                ));
                return None;
            }
            expect_ident = true;
        }
    }
    if expect_ident {
        diags.push(CssDiag::error(
            span,
            DiagCode::InvalidAtRule,
            "trailing `.` in @layer name",
        ));
        return None;
    }
    Some(LayerName(segments))
}

// ============================================================================
// 2. @keyframes
// ============================================================================

/// Single keyframe selector stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyframeStop {
    From,
    To,
    Percentage(f32),
}

impl KeyframeStop {
    /// Convert stop to normalized progress `0.0..=1.0`.
    #[inline]
    pub fn as_progress(self) -> f32 {
        match self {
            Self::From => 0.0,
            Self::To => 1.0,
            Self::Percentage(p) => (p / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// One keyframe stop block (`from, 50% { opacity: 0.5; }`).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeBlock {
    pub stops: Vec<KeyframeStop>,
    pub declarations: Vec<Declaration>,
    pub span: Span,
}

/// Parsed `@keyframes <name> { ... }` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframesRule {
    pub name: String,
    pub blocks: Vec<KeyframeBlock>,
    pub span: Span,
}

/// Parse an `@keyframes` at-rule.
pub fn parse_keyframes_rule(at: &AtRule, diags: &mut Vec<CssDiag>) -> Option<KeyframesRule> {
    let prelude = trim_ws(&at.prelude);
    let name = match prelude {
        [
            ComponentValue::Token(SpannedToken {
                token: Token::Ident(s) | Token::String(s),
                ..
            }),
        ] if !s.eq_ignore_ascii_case("none") => s.clone(),
        _ => {
            diags.push(CssDiag::error(
                at.prelude_span,
                DiagCode::InvalidAtRule,
                "@keyframes requires a single animation name",
            ));
            return None;
        }
    };
    let Some(block) = &at.block else {
        diags.push(CssDiag::error(
            at.span,
            DiagCode::InvalidAtRule,
            "@keyframes requires a `{ ... }` block",
        ));
        return None;
    };

    let raw_rules = parse_rule_list(block, diags);
    let mut blocks = Vec::new();
    for rule in raw_rules {
        let Rule::Qualified(q) = rule else {
            diags.push(CssDiag::error(
                rule.span(),
                DiagCode::InvalidAtRule,
                "unexpected nested at-rule inside @keyframes",
            ));
            continue;
        };
        let groups = split_comma_groups(&q.prelude);
        let mut stops = Vec::new();
        let mut valid = !groups.is_empty();
        for g in groups {
            match g {
                [
                    ComponentValue::Token(SpannedToken {
                        token: Token::Ident(kw),
                        ..
                    }),
                ] if kw.eq_ignore_ascii_case("from") => stops.push(KeyframeStop::From),
                [
                    ComponentValue::Token(SpannedToken {
                        token: Token::Ident(kw),
                        ..
                    }),
                ] if kw.eq_ignore_ascii_case("to") => stops.push(KeyframeStop::To),
                [
                    ComponentValue::Token(SpannedToken {
                        token: Token::Percentage(p),
                        ..
                    }),
                ] if p.is_finite() && (0.0..=100.0).contains(p) => {
                    stops.push(KeyframeStop::Percentage(*p as f32));
                }
                _ => {
                    valid = false;
                    diags.push(CssDiag::error(
                        q.prelude_span,
                        DiagCode::InvalidAtRule,
                        "invalid keyframe selector stop (expected `from`, `to`, or 0%..100%)",
                    ));
                    break;
                }
            }
        }
        if valid {
            let declarations = parse_declarations_from_values(&q.block, diags);
            blocks.push(KeyframeBlock {
                stops,
                declarations,
                span: q.span,
            });
        }
    }

    Some(KeyframesRule {
        name,
        blocks,
        span: at.span,
    })
}

// ============================================================================
// Shared comparison operator & Kleene 3-valued logic
// ============================================================================

/// Range comparison operator shared by `@container` and `@media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    Lt,
    Le,
    Eq,
    Gt,
    Ge,
}

impl CompareOp {
    /// Flip operator direction when swapping `val <op> feature` into `feature <op'> val`.
    #[inline]
    pub const fn flip(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::Le => Self::Ge,
            Self::Eq => Self::Eq,
            Self::Gt => Self::Lt,
            Self::Ge => Self::Le,
        }
    }

    /// Evaluate `lhs <op> rhs`.
    #[inline]
    pub fn eval_f32(self, lhs: f32, rhs: f32) -> bool {
        match self {
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            Self::Eq => (lhs - rhs).abs() <= 1e-4,
            Self::Gt => lhs > rhs,
            Self::Ge => lhs >= rhs,
        }
    }
}

/// Kleene 3-valued logic (`True`, `False`, `Unknown`) per CSS Media Queries 4
/// and CSS Containment 3.
///
/// Crucially, `Kleene3::Unknown.not() == Kleene3::Unknown` (so `@media not (unknown)`
/// evaluates to `Unknown`, which does NOT match as a top-level query).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kleene3 {
    True,
    False,
    Unknown,
}

impl Kleene3 {
    #[inline]
    pub const fn from_bool(v: bool) -> Self {
        if v { Self::True } else { Self::False }
    }

    #[inline]
    pub const fn is_true(self) -> bool {
        matches!(self, Self::True)
    }

    #[inline]
    pub const fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }

    #[inline]
    pub const fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    #[inline]
    pub const fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }
}

// ============================================================================
// 3. @container
// ============================================================================

/// Size feature name inside `@container`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerFeatureName {
    Width,
    Height,
    InlineSize,
    BlockSize,
    AspectRatio,
    Orientation,
}

impl ContainerFeatureName {
    fn from_ident(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "width" => Some(Self::Width),
            "height" => Some(Self::Height),
            "inline-size" => Some(Self::InlineSize),
            "block-size" => Some(Self::BlockSize),
            "aspect-ratio" => Some(Self::AspectRatio),
            "orientation" => Some(Self::Orientation),
            _ => None,
        }
    }
}

/// Typed value inside a `@container` size query.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerValue {
    Length(LengthSpec),
    Ratio(f32, f32),
    Ident(String),
}

/// One size query atom inside `@container`.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerSizeQuery {
    Plain {
        feature: ContainerFeatureName,
        value: ContainerValue,
    },
    Min {
        feature: ContainerFeatureName,
        value: ContainerValue,
    },
    Max {
        feature: ContainerFeatureName,
        value: ContainerValue,
    },
    /// Range syntax: e.g. `width >= 400px` or `300px < inline-size <= 600px`.
    /// Each bound is stored in normalized `feature <op> value` form.
    Range {
        feature: ContainerFeatureName,
        bounds: Vec<(CompareOp, ContainerValue)>,
    },
    Unknown(Vec<ComponentValue>),
}

/// Boolean condition tree for `@container`.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerCondition {
    Feature(ContainerSizeQuery),
    Not(Box<ContainerCondition>),
    And(Vec<ContainerCondition>),
    Or(Vec<ContainerCondition>),
    Unknown(Vec<ComponentValue>),
}

/// Parsed `@container [name] <condition> { <rules> }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerRule {
    pub name: Option<String>,
    pub condition: ContainerCondition,
    pub rules: Vec<Rule>,
    pub span: Span,
}

/// Parse an `@container` prelude into `(optional_name, condition)`.
pub fn parse_container_prelude(
    prelude: &[ComponentValue],
    diags: &mut Vec<CssDiag>,
) -> Option<(Option<String>, ContainerCondition)> {
    let trimmed = trim_ws(prelude);
    if trimmed.is_empty() {
        diags.push(CssDiag::error(
            Span::EMPTY,
            DiagCode::InvalidAtRule,
            "empty @container prelude",
        ));
        return None;
    }

    let mut rest = trimmed;
    let mut name = None;
    if let Some(ident) = rest.first().and_then(ComponentValue::as_ident)
        && !ident.eq_ignore_ascii_case("not")
        && !ident.eq_ignore_ascii_case("and")
        && !ident.eq_ignore_ascii_case("or")
        && !ident.eq_ignore_ascii_case("none")
    {
        name = Some(ident.to_string());
        rest = trim_ws(&rest[1..]);
    }

    let cond = parse_container_condition(rest, 0, diags)?;
    Some((name, cond))
}

/// Parse a full `@container` at-rule including nested rules.
pub fn parse_container_rule(at: &AtRule, diags: &mut Vec<CssDiag>) -> Option<ContainerRule> {
    let Some(block) = &at.block else {
        diags.push(CssDiag::error(
            at.span,
            DiagCode::InvalidAtRule,
            "@container requires a `{ ... }` block",
        ));
        return None;
    };
    let (name, condition) = parse_container_prelude(&at.prelude, diags)?;
    let rules = parse_rule_list(block, diags);
    Some(ContainerRule {
        name,
        condition,
        rules,
        span: at.span,
    })
}

fn parse_container_condition(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<ContainerCondition> {
    if depth >= MAX_NESTING {
        let span = span_of_slice(values).unwrap_or(Span::EMPTY);
        diags.push(CssDiag::error(
            span,
            DiagCode::NestingLimitExceeded,
            format!("@container condition nesting limit ({MAX_NESTING}) exceeded"),
        ));
        return None;
    }
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }

    // Leading `not <in-parens>`
    if let Some(kw) = trimmed.first().and_then(ComponentValue::as_ident)
        && kw.eq_ignore_ascii_case("not")
    {
        let inner = parse_container_in_parens(trim_ws(&trimmed[1..]), depth + 1, diags)?;
        return Some(ContainerCondition::Not(Box::new(inner)));
    }

    // Split top-level `and` / `or` sequence (CSS forbids mixing `and` and `or` at the same level).
    let (op, parts) = split_boolean_chain(trimmed)?;
    match op {
        None => parse_container_in_parens(parts[0], depth + 1, diags),
        Some(BoolOp::And) => {
            let mut items = Vec::with_capacity(parts.len());
            for p in parts {
                items.push(parse_container_in_parens(p, depth + 1, diags)?);
            }
            Some(ContainerCondition::And(items))
        }
        Some(BoolOp::Or) => {
            let mut items = Vec::with_capacity(parts.len());
            for p in parts {
                items.push(parse_container_in_parens(p, depth + 1, diags)?);
            }
            Some(ContainerCondition::Or(items))
        }
    }
}

fn parse_container_in_parens(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<ContainerCondition> {
    if depth >= MAX_NESTING {
        let span = span_of_slice(values).unwrap_or(Span::EMPTY);
        diags.push(CssDiag::error(
            span,
            DiagCode::NestingLimitExceeded,
            format!("@container nesting limit ({MAX_NESTING}) exceeded"),
        ));
        return None;
    }
    let trimmed = trim_ws(values);
    match trimmed {
        [
            ComponentValue::SimpleBlock {
                kind: BlockKind::Paren,
                values: inner,
                ..
            },
        ] => {
            let inner_trimmed = trim_ws(inner);
            if looks_like_nested_condition(inner_trimmed) {
                parse_container_condition(inner_trimmed, depth + 1, diags)
            } else {
                Some(ContainerCondition::Feature(parse_container_size_query(
                    inner_trimmed,
                )))
            }
        }
        _ => Some(ContainerCondition::Unknown(trimmed.to_vec())),
    }
}

fn parse_container_size_query(inner: &[ComponentValue]) -> ContainerSizeQuery {
    // Check colon form: `feature: value` / `min-feature: value` / `max-feature: value`
    if let Some(colon_idx) = inner
        .iter()
        .position(|cv| matches!(cv.as_token(), Some(Token::Colon)))
    {
        let lhs = trim_ws(&inner[..colon_idx]);
        let rhs = trim_ws(&inner[colon_idx + 1..]);
        if let [
            ComponentValue::Token(SpannedToken {
                token: Token::Ident(raw_name),
                ..
            }),
        ] = lhs
            && let Some(val) = parse_container_value(rhs)
        {
            let lower = raw_name.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("min-")
                && let Some(feat) = ContainerFeatureName::from_ident(rest)
            {
                return ContainerSizeQuery::Min {
                    feature: feat,
                    value: val,
                };
            }
            if let Some(rest) = lower.strip_prefix("max-")
                && let Some(feat) = ContainerFeatureName::from_ident(rest)
            {
                return ContainerSizeQuery::Max {
                    feature: feat,
                    value: val,
                };
            }
            if let Some(feat) = ContainerFeatureName::from_ident(&lower) {
                return ContainerSizeQuery::Plain {
                    feature: feat,
                    value: val,
                };
            }
        }
        return ContainerSizeQuery::Unknown(inner.to_vec());
    }

    // Check range syntax: `width >= 400px`, `400px <= width`, `300px < inline-size <= 600px`.
    if let Some(range) = parse_range_parts(inner) {
        let (name, bounds_raw) = range;
        if let Some(feat) = ContainerFeatureName::from_ident(&name) {
            let mut bounds = Vec::with_capacity(bounds_raw.len());
            let mut ok = true;
            for (op, val_cvs) in bounds_raw {
                if let Some(v) = parse_container_value(&val_cvs) {
                    bounds.push((op, v));
                } else {
                    ok = false;
                    break;
                }
            }
            if ok && !bounds.is_empty() {
                return ContainerSizeQuery::Range {
                    feature: feat,
                    bounds,
                };
            }
        }
    }

    ContainerSizeQuery::Unknown(inner.to_vec())
}

fn parse_container_value(values: &[ComponentValue]) -> Option<ContainerValue> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    // Ratio `a / b`
    if let Some(slash) = trimmed
        .iter()
        .position(|cv| matches!(cv.as_token(), Some(Token::Delim('/'))))
    {
        let a = extract_positive_number(trim_ws(&trimmed[..slash]))?;
        let b = extract_positive_number(trim_ws(&trimmed[slash + 1..]))?;
        return Some(ContainerValue::Ratio(a, b));
    }
    if let Some(len) = parse_length_spec(trimmed) {
        return Some(ContainerValue::Length(len));
    }
    if let [cv] = trimmed
        && let Some(ident) = cv.as_ident()
    {
        return Some(ContainerValue::Ident(ident.to_ascii_lowercase()));
    }
    None
}

fn extract_positive_number(values: &[ComponentValue]) -> Option<f32> {
    if let [
        ComponentValue::Token(SpannedToken {
            token: Token::Number { value, .. },
            ..
        }),
    ] = values
        && value.is_finite()
        && *value > 0.0
    {
        Some(*value as f32)
    } else {
        None
    }
}

// ============================================================================
// 4. @media (with Kleene 3-valued logic)
// ============================================================================

/// Media type in `@media`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaType {
    All,
    Screen,
    Print,
    Speech,
    Unknown(String),
}

impl MediaType {
    fn from_ident(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "all" => Self::All,
            "screen" => Self::Screen,
            "print" => Self::Print,
            "speech" => Self::Speech,
            _ => Self::Unknown(name.to_ascii_lowercase()),
        }
    }
}

/// Leading modifier on a media query with a media type (`not screen`, `only screen`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaModifier {
    Not,
    Only,
}

/// One media feature atom inside `@media`.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaFeature {
    /// Boolean `(color)` or plain `(min-width: 600px)` / `(prefers-color-scheme: dark)`.
    Plain {
        name: String,
        value: Option<Vec<ComponentValue>>,
    },
    /// Range syntax `(width >= 600px)` or `(400px <= width < 1000px)`.
    /// Each bound is normalized to `feature <op> value`.
    Range {
        name: String,
        bounds: Vec<(CompareOp, Vec<ComponentValue>)>,
    },
    /// Unrecognized syntax preserved for Kleene 3-valued evaluation (`Unknown`).
    Unknown(Vec<ComponentValue>),
}

/// Boolean condition tree for `@media`.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaCondition {
    Feature(MediaFeature),
    Not(Box<MediaCondition>),
    And(Vec<MediaCondition>),
    Or(Vec<MediaCondition>),
    Unknown(Vec<ComponentValue>),
}

impl MediaCondition {
    /// Evaluate this condition using Kleene 3-valued logic (`Unknown` propagates
    /// through `not` without flipping to `True`!).
    pub fn evaluate(&self, eval_feature: &impl Fn(&MediaFeature) -> Kleene3) -> Kleene3 {
        match self {
            Self::Feature(f) => eval_feature(f),
            Self::Not(inner) => inner.evaluate(eval_feature).not(),
            Self::And(list) => {
                let mut acc = Kleene3::True;
                for item in list {
                    acc = acc.and(item.evaluate(eval_feature));
                }
                acc
            }
            Self::Or(list) => {
                let mut acc = Kleene3::False;
                for item in list {
                    acc = acc.or(item.evaluate(eval_feature));
                }
                acc
            }
            Self::Unknown(_) => Kleene3::Unknown,
        }
    }
}

/// One media query in a comma-separated [`MediaQueryList`].
#[derive(Debug, Clone, PartialEq)]
pub enum MediaQuery {
    TypeAndCondition {
        modifier: Option<MediaModifier>,
        media_type: MediaType,
        condition: Option<MediaCondition>,
    },
    ConditionOnly(MediaCondition),
    /// Malformed query (`not all` per Media Queries Level 4 — never matches,
    /// does not poison sibling queries).
    Invalid,
}

impl MediaQuery {
    /// Evaluate this single media query against a media-type matcher and a
    /// Kleene 3-valued feature evaluator.
    pub fn matches(
        &self,
        match_type: &impl Fn(&MediaType) -> bool,
        eval_feature: &impl Fn(&MediaFeature) -> Kleene3,
    ) -> bool {
        match self {
            Self::Invalid => false,
            Self::ConditionOnly(cond) => cond.evaluate(eval_feature).is_true(),
            Self::TypeAndCondition {
                modifier,
                media_type,
                condition,
            } => {
                let type_k = Kleene3::from_bool(match_type(media_type));
                let combined = match condition {
                    Some(c) => type_k.and(c.evaluate(eval_feature)),
                    None => type_k,
                };
                match modifier {
                    Some(MediaModifier::Not) => combined.not().is_true(),
                    Some(MediaModifier::Only) | None => combined.is_true(),
                }
            }
        }
    }
}

/// Comma-separated list of [`MediaQuery`]s (matches if ANY query matches, or
/// if the list is empty per CSS Media Queries 4).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaQueryList(pub Vec<MediaQuery>);

impl MediaQueryList {
    pub fn matches(
        &self,
        match_type: &impl Fn(&MediaType) -> bool,
        eval_feature: &impl Fn(&MediaFeature) -> Kleene3,
    ) -> bool {
        if self.0.is_empty() {
            return true;
        }
        self.0.iter().any(|q| q.matches(match_type, eval_feature))
    }
}

/// Parse an `@media` prelude into a [`MediaQueryList`].
pub fn parse_media_query_list(
    prelude: &[ComponentValue],
    diags: &mut Vec<CssDiag>,
) -> MediaQueryList {
    let trimmed = trim_ws(prelude);
    if trimmed.is_empty() {
        return MediaQueryList(Vec::new());
    }
    let groups = split_comma_groups(trimmed);
    let mut queries = Vec::with_capacity(groups.len());
    for group in groups {
        if group.is_empty() {
            queries.push(MediaQuery::Invalid);
            continue;
        }
        queries.push(parse_single_media_query(group, diags));
    }
    MediaQueryList(queries)
}

fn parse_single_media_query(values: &[ComponentValue], diags: &mut Vec<CssDiag>) -> MediaQuery {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return MediaQuery::Invalid;
    }

    // Condition-only starting with `(...)`
    if matches!(
        trimmed.first(),
        Some(ComponentValue::SimpleBlock {
            kind: BlockKind::Paren,
            ..
        })
    ) {
        return match parse_media_condition(trimmed, 0, diags) {
            Some(c) => MediaQuery::ConditionOnly(c),
            None => MediaQuery::Invalid,
        };
    }

    // Check leading `not` followed by `(...)`: that is a condition-only `not (feature)`!
    if let Some(first_ident) = trimmed.first().and_then(ComponentValue::as_ident) {
        if first_ident.eq_ignore_ascii_case("not") {
            let after_not = trim_ws(&trimmed[1..]);
            if matches!(
                after_not.first(),
                Some(ComponentValue::SimpleBlock {
                    kind: BlockKind::Paren,
                    ..
                })
            ) {
                return match parse_media_condition(trimmed, 0, diags) {
                    Some(c) => MediaQuery::ConditionOnly(c),
                    None => MediaQuery::Invalid,
                };
            }
        }

        // `[not | only]? <media-type> [and <media-condition-without-or>]?`
        let mut rest = trimmed;
        let modifier = if first_ident.eq_ignore_ascii_case("not") {
            rest = trim_ws(&rest[1..]);
            Some(MediaModifier::Not)
        } else if first_ident.eq_ignore_ascii_case("only") {
            rest = trim_ws(&rest[1..]);
            Some(MediaModifier::Only)
        } else {
            None
        };

        let Some(type_ident) = rest.first().and_then(ComponentValue::as_ident) else {
            return MediaQuery::Invalid;
        };
        if type_ident.eq_ignore_ascii_case("only")
            || type_ident.eq_ignore_ascii_case("not")
            || type_ident.eq_ignore_ascii_case("and")
            || type_ident.eq_ignore_ascii_case("or")
        {
            return MediaQuery::Invalid;
        }
        let media_type = MediaType::from_ident(type_ident);
        rest = trim_ws(&rest[1..]);
        if rest.is_empty() {
            return MediaQuery::TypeAndCondition {
                modifier,
                media_type,
                condition: None,
            };
        }

        // Must be `and <media-condition>`
        let Some(and_kw) = rest.first().and_then(ComponentValue::as_ident) else {
            return MediaQuery::Invalid;
        };
        if !and_kw.eq_ignore_ascii_case("and") {
            return MediaQuery::Invalid;
        }
        let cond_slice = trim_ws(&rest[1..]);
        let Some(cond) = parse_media_condition(cond_slice, 0, diags) else {
            return MediaQuery::Invalid;
        };
        return MediaQuery::TypeAndCondition {
            modifier,
            media_type,
            condition: Some(cond),
        };
    }

    MediaQuery::Invalid
}

fn parse_media_condition(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<MediaCondition> {
    if depth >= MAX_NESTING {
        let span = span_of_slice(values).unwrap_or(Span::EMPTY);
        diags.push(CssDiag::error(
            span,
            DiagCode::NestingLimitExceeded,
            format!("@media condition nesting limit ({MAX_NESTING}) exceeded"),
        ));
        return None;
    }
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }

    if let Some(kw) = trimmed.first().and_then(ComponentValue::as_ident)
        && kw.eq_ignore_ascii_case("not")
    {
        let inner = parse_media_in_parens(trim_ws(&trimmed[1..]), depth + 1, diags)?;
        return Some(MediaCondition::Not(Box::new(inner)));
    }

    let (op, parts) = split_boolean_chain(trimmed)?;
    match op {
        None => parse_media_in_parens(parts[0], depth + 1, diags),
        Some(BoolOp::And) => {
            let mut items = Vec::with_capacity(parts.len());
            for p in parts {
                items.push(parse_media_in_parens(p, depth + 1, diags)?);
            }
            Some(MediaCondition::And(items))
        }
        Some(BoolOp::Or) => {
            let mut items = Vec::with_capacity(parts.len());
            for p in parts {
                items.push(parse_media_in_parens(p, depth + 1, diags)?);
            }
            Some(MediaCondition::Or(items))
        }
    }
}

fn parse_media_in_parens(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<MediaCondition> {
    if depth >= MAX_NESTING {
        let span = span_of_slice(values).unwrap_or(Span::EMPTY);
        diags.push(CssDiag::error(
            span,
            DiagCode::NestingLimitExceeded,
            format!("@media nesting limit ({MAX_NESTING}) exceeded"),
        ));
        return None;
    }
    let trimmed = trim_ws(values);
    match trimmed {
        [
            ComponentValue::SimpleBlock {
                kind: BlockKind::Paren,
                values: inner,
                ..
            },
        ] => {
            let inner_trimmed = trim_ws(inner);
            if inner_trimmed.is_empty() {
                return Some(MediaCondition::Unknown(Vec::new()));
            }
            if looks_like_nested_condition(inner_trimmed) {
                parse_media_condition(inner_trimmed, depth + 1, diags)
            } else {
                Some(MediaCondition::Feature(parse_media_feature(inner_trimmed)))
            }
        }
        _ => Some(MediaCondition::Unknown(trimmed.to_vec())),
    }
}

fn parse_media_feature(inner: &[ComponentValue]) -> MediaFeature {
    // Plain `name: value`
    if let Some(colon_idx) = inner
        .iter()
        .position(|cv| matches!(cv.as_token(), Some(Token::Colon)))
    {
        let lhs = trim_ws(&inner[..colon_idx]);
        let rhs = trim_ws(&inner[colon_idx + 1..]);
        if let [
            ComponentValue::Token(SpannedToken {
                token: Token::Ident(name),
                ..
            }),
        ] = lhs
            && !rhs.is_empty()
        {
            return MediaFeature::Plain {
                name: name.to_ascii_lowercase(),
                value: Some(rhs.to_vec()),
            };
        }
        return MediaFeature::Unknown(inner.to_vec());
    }

    // Range `width >= 600px` / `400px <= width < 1000px`
    if let Some((name, bounds)) = parse_range_parts(inner) {
        return MediaFeature::Range { name, bounds };
    }

    // Boolean feature `(color)` / `(hover)` / `(unknown-feature)`
    if let [
        ComponentValue::Token(SpannedToken {
            token: Token::Ident(name),
            ..
        }),
    ] = inner
    {
        return MediaFeature::Plain {
            name: name.to_ascii_lowercase(),
            value: None,
        };
    }

    MediaFeature::Unknown(inner.to_vec())
}

// ============================================================================
// 5. @supports
// ============================================================================

/// Parsed `@supports` condition AST.
#[derive(Debug, Clone, PartialEq)]
pub enum SupportsCondition {
    Declaration {
        property: String,
        value: Vec<ComponentValue>,
    },
    Selector(Vec<ComponentValue>),
    Not(Box<SupportsCondition>),
    And(Vec<SupportsCondition>),
    Or(Vec<SupportsCondition>),
    Unknown(Vec<ComponentValue>),
}

impl SupportsCondition {
    /// Evaluate this `@supports` condition using caller-supplied predicates for
    /// declarations and `selector(...)`.
    pub fn evaluate(
        &self,
        supports_decl: &impl Fn(&str, &[ComponentValue]) -> bool,
        supports_selector: &impl Fn(&[ComponentValue]) -> bool,
    ) -> bool {
        match self {
            Self::Declaration { property, value } => supports_decl(property, value),
            Self::Selector(sel) => supports_selector(sel),
            Self::Not(inner) => !inner.evaluate(supports_decl, supports_selector),
            Self::And(list) => list
                .iter()
                .all(|c| c.evaluate(supports_decl, supports_selector)),
            Self::Or(list) => list
                .iter()
                .any(|c| c.evaluate(supports_decl, supports_selector)),
            Self::Unknown(_) => false,
        }
    }
}

/// Parse an `@supports` prelude into a [`SupportsCondition`].
pub fn parse_supports_condition(
    prelude: &[ComponentValue],
    diags: &mut Vec<CssDiag>,
) -> Option<SupportsCondition> {
    parse_supports_cond_depth(prelude, 0, diags)
}

fn parse_supports_cond_depth(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<SupportsCondition> {
    if depth >= MAX_NESTING {
        let span = span_of_slice(values).unwrap_or(Span::EMPTY);
        diags.push(CssDiag::error(
            span,
            DiagCode::NestingLimitExceeded,
            format!("@supports condition nesting limit ({MAX_NESTING}) exceeded"),
        ));
        return None;
    }
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }

    if let Some(kw) = trimmed.first().and_then(ComponentValue::as_ident)
        && kw.eq_ignore_ascii_case("not")
    {
        let inner = parse_supports_in_parens(trim_ws(&trimmed[1..]), depth + 1, diags)?;
        return Some(SupportsCondition::Not(Box::new(inner)));
    }

    let (op, parts) = split_boolean_chain(trimmed)?;
    match op {
        None => parse_supports_in_parens(parts[0], depth + 1, diags),
        Some(BoolOp::And) => {
            let mut items = Vec::with_capacity(parts.len());
            for p in parts {
                items.push(parse_supports_in_parens(p, depth + 1, diags)?);
            }
            Some(SupportsCondition::And(items))
        }
        Some(BoolOp::Or) => {
            let mut items = Vec::with_capacity(parts.len());
            for p in parts {
                items.push(parse_supports_in_parens(p, depth + 1, diags)?);
            }
            Some(SupportsCondition::Or(items))
        }
    }
}

fn parse_supports_in_parens(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<SupportsCondition> {
    if depth >= MAX_NESTING {
        let span = span_of_slice(values).unwrap_or(Span::EMPTY);
        diags.push(CssDiag::error(
            span,
            DiagCode::NestingLimitExceeded,
            format!("@supports nesting limit ({MAX_NESTING}) exceeded"),
        ));
        return None;
    }
    let trimmed = trim_ws(values);
    match trimmed {
        [ComponentValue::Function { name, args, .. }] if name.eq_ignore_ascii_case("selector") => {
            Some(SupportsCondition::Selector(trim_ws(args).to_vec()))
        }
        [
            ComponentValue::SimpleBlock {
                kind: BlockKind::Paren,
                values: inner,
                ..
            },
        ] => {
            let inner_trimmed = trim_ws(inner);
            if inner_trimmed.is_empty() {
                return Some(SupportsCondition::Unknown(Vec::new()));
            }
            // Check `(property: value)` first (declaration feature)
            if let Some(colon_idx) = inner_trimmed
                .iter()
                .position(|cv| matches!(cv.as_token(), Some(Token::Colon)))
            {
                let lhs = trim_ws(&inner_trimmed[..colon_idx]);
                let rhs = trim_ws(&inner_trimmed[colon_idx + 1..]);
                if let [
                    ComponentValue::Token(SpannedToken {
                        token: Token::Ident(prop),
                        ..
                    }),
                ] = lhs
                {
                    return Some(SupportsCondition::Declaration {
                        property: prop.to_ascii_lowercase(),
                        value: rhs.to_vec(),
                    });
                }
            }
            // Otherwise nested condition: `((a: b) or (c: d))`, `(not (a: b))`, `(selector(...))`
            if looks_like_nested_condition(inner_trimmed)
                || matches!(inner_trimmed.first(), Some(ComponentValue::Function { .. }))
            {
                parse_supports_cond_depth(inner_trimmed, depth + 1, diags)
            } else {
                Some(SupportsCondition::Unknown(inner_trimmed.to_vec()))
            }
        }
        _ => Some(SupportsCondition::Unknown(trimmed.to_vec())),
    }
}

// ============================================================================
// 6. @font-face (single unified descriptor parser)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontFaceSrcKind {
    Url,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceSrc {
    pub kind: FontFaceSrcKind,
    pub value: String,
    pub format: Option<String>,
    pub tech: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontFaceStyle {
    Normal,
    Italic,
    Oblique {
        min_deg: Option<f32>,
        max_deg: Option<f32>,
    },
}

/// CSS Fonts 4 `font-weight` descriptor (single value or range `min max`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontWeightRange {
    pub min: u16,
    pub max: u16,
}

/// CSS Fonts 4 `font-stretch` descriptor percentage range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontStretchRange {
    pub min_percent: f32,
    pub max_percent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontDisplay {
    Auto,
    Block,
    Swap,
    Fallback,
    Optional,
}

/// Inclusive Unicode scalar range (`U+0025-00FF`, `U+4??`, `U+0026`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnicodeRange {
    pub start: u32,
    pub end: u32,
}

/// Parsed `@font-face` rule with all supported CSS Fonts 4 descriptors.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFaceRule {
    pub font_family: Option<String>,
    pub src: Vec<FontFaceSrc>,
    pub font_weight: Option<FontWeightRange>,
    pub font_style: Option<FontFaceStyle>,
    pub font_stretch: Option<FontStretchRange>,
    pub font_display: Option<FontDisplay>,
    pub unicode_range: Vec<UnicodeRange>,
    pub span: Span,
}

/// Parse an `@font-face` at-rule using the unified declaration parser.
pub fn parse_font_face_rule(at: &AtRule, diags: &mut Vec<CssDiag>) -> Option<FontFaceRule> {
    if !trim_ws(&at.prelude).is_empty() {
        diags.push(CssDiag::error(
            at.prelude_span,
            DiagCode::InvalidAtRule,
            "@font-face does not accept a prelude",
        ));
        return None;
    }
    let Some(block) = &at.block else {
        diags.push(CssDiag::error(
            at.span,
            DiagCode::InvalidAtRule,
            "@font-face requires a `{ ... }` block",
        ));
        return None;
    };

    let decls = parse_declarations_from_values(block, diags);
    let mut rule = FontFaceRule {
        font_family: None,
        src: Vec::new(),
        font_weight: None,
        font_style: None,
        font_stretch: None,
        font_display: None,
        unicode_range: Vec::new(),
        span: at.span,
    };

    for decl in decls {
        match decl.name.to_ascii_lowercase().as_str() {
            "font-family" => {
                if let Some(fam) = parse_font_family_descriptor(&decl.value) {
                    rule.font_family = Some(fam);
                } else {
                    diags.push(CssDiag::error(
                        decl.value_span,
                        DiagCode::InvalidValue,
                        "invalid `font-family` descriptor in @font-face",
                    ));
                }
            }
            "src" => {
                let srcs = parse_font_face_src_list(&decl.value);
                if !srcs.is_empty() {
                    rule.src = srcs;
                } else {
                    diags.push(CssDiag::error(
                        decl.value_span,
                        DiagCode::InvalidValue,
                        "invalid `src` descriptor in @font-face",
                    ));
                }
            }
            "font-weight" => {
                if let Some(wr) = parse_font_weight_range(&decl.value) {
                    rule.font_weight = Some(wr);
                } else {
                    diags.push(CssDiag::error(
                        decl.value_span,
                        DiagCode::InvalidValue,
                        "invalid `font-weight` descriptor in @font-face",
                    ));
                }
            }
            "font-style" => {
                if let Some(st) = parse_font_style_descriptor(&decl.value) {
                    rule.font_style = Some(st);
                } else {
                    diags.push(CssDiag::error(
                        decl.value_span,
                        DiagCode::InvalidValue,
                        "invalid `font-style` descriptor in @font-face",
                    ));
                }
            }
            "font-stretch" => {
                if let Some(sr) = parse_font_stretch_descriptor(&decl.value) {
                    rule.font_stretch = Some(sr);
                } else {
                    diags.push(CssDiag::error(
                        decl.value_span,
                        DiagCode::InvalidValue,
                        "invalid `font-stretch` descriptor in @font-face",
                    ));
                }
            }
            "font-display" => {
                if let Some(fd) = parse_font_display_descriptor(&decl.value) {
                    rule.font_display = Some(fd);
                } else {
                    diags.push(CssDiag::error(
                        decl.value_span,
                        DiagCode::InvalidValue,
                        "invalid `font-display` descriptor in @font-face",
                    ));
                }
            }
            "unicode-range" => {
                let ranges = parse_unicode_range_list(&decl.value);
                if !ranges.is_empty() {
                    rule.unicode_range = ranges;
                } else {
                    diags.push(CssDiag::error(
                        decl.value_span,
                        DiagCode::InvalidValue,
                        "invalid `unicode-range` descriptor in @font-face",
                    ));
                }
            }
            _ => {}
        }
    }

    Some(rule)
}

fn parse_font_family_descriptor(values: &[ComponentValue]) -> Option<String> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    if let [
        ComponentValue::Token(SpannedToken {
            token: Token::String(s),
            ..
        }),
    ] = trimmed
    {
        return (!s.is_empty()).then(|| s.clone());
    }
    // Unquoted multi-ident family name (e.g. `Noto Sans SC`).
    let mut parts = Vec::new();
    for item in split_whitespace_items(trimmed) {
        if let [cv] = item
            && let Some(ident) = cv.as_ident()
        {
            parts.push(ident);
        } else {
            return None;
        }
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join(" ");
    if joined.eq_ignore_ascii_case("inherit") || joined.eq_ignore_ascii_case("initial") {
        return None;
    }
    Some(joined)
}

fn parse_font_face_src_list(values: &[ComponentValue]) -> Vec<FontFaceSrc> {
    let mut out = Vec::new();
    for group in split_comma_groups(values) {
        if let Some(src) = parse_font_face_src_item(group) {
            out.push(src);
        }
    }
    out
}

fn parse_font_face_src_item(values: &[ComponentValue]) -> Option<FontFaceSrc> {
    let items = split_whitespace_items(values);
    let first = items.first()?;
    if first.len() != 1 {
        return None;
    }
    let (kind, value) = if let Some(url) = first[0].as_url() {
        if url.is_empty() {
            return None;
        }
        (FontFaceSrcKind::Url, url.to_string())
    } else if let ComponentValue::Function { name, args, .. } = &first[0]
        && name.eq_ignore_ascii_case("local")
    {
        let fam = parse_font_family_descriptor(args)?;
        (FontFaceSrcKind::Local, fam)
    } else {
        return None;
    };

    let mut format = None;
    let mut tech = Vec::new();
    for extra in &items[1..] {
        if let [ComponentValue::Function { name, args, .. }] = extra {
            if name.eq_ignore_ascii_case("format") {
                if let [
                    ComponentValue::Token(SpannedToken {
                        token: Token::String(s) | Token::Ident(s),
                        ..
                    }),
                ] = trim_ws(args)
                {
                    format = Some(s.clone());
                }
            } else if name.eq_ignore_ascii_case("tech") {
                for g in split_comma_groups(args) {
                    if let [cv] = g
                        && let Some(ident) = cv.as_ident()
                    {
                        tech.push(ident.to_ascii_lowercase());
                    }
                }
            }
        }
    }

    Some(FontFaceSrc {
        kind,
        value,
        format,
        tech,
    })
}

fn parse_font_weight_range(values: &[ComponentValue]) -> Option<FontWeightRange> {
    let items = split_whitespace_items(values);
    let parse_one = |slice: &[ComponentValue]| -> Option<u16> {
        let [cv] = slice else { return None };
        match cv.as_token()? {
            Token::Ident(s) if s.eq_ignore_ascii_case("normal") => Some(400),
            Token::Ident(s) if s.eq_ignore_ascii_case("bold") => Some(700),
            Token::Number { value, .. } if value.is_finite() && (1.0..=1000.0).contains(value) => {
                Some(value.round() as u16)
            }
            _ => None,
        }
    };
    match items.as_slice() {
        [only] => {
            if let [cv] = *only
                && cv
                    .as_ident()
                    .is_some_and(|s| s.eq_ignore_ascii_case("auto"))
            {
                return Some(FontWeightRange { min: 1, max: 1000 });
            }
            let w = parse_one(only)?;
            Some(FontWeightRange { min: w, max: w })
        }
        [lo, hi] => {
            let a = parse_one(lo)?;
            let b = parse_one(hi)?;
            if a > b {
                return None;
            }
            Some(FontWeightRange { min: a, max: b })
        }
        _ => None,
    }
}

fn parse_font_style_descriptor(values: &[ComponentValue]) -> Option<FontFaceStyle> {
    let items = split_whitespace_items(values);
    let first = items.first()?;
    let [first_cv] = *first else { return None };
    let kw = first_cv.as_ident()?.to_ascii_lowercase();
    match kw.as_str() {
        "normal" if items.len() == 1 => Some(FontFaceStyle::Normal),
        "italic" if items.len() == 1 => Some(FontFaceStyle::Italic),
        "oblique" => match items.len() {
            1 => Some(FontFaceStyle::Oblique {
                min_deg: None,
                max_deg: None,
            }),
            2 => {
                let [a] = items[1] else { return None };
                let deg = parse_angle_token_deg(a)?;
                if !(-90.0..=90.0).contains(&deg) {
                    return None;
                }
                Some(FontFaceStyle::Oblique {
                    min_deg: Some(deg),
                    max_deg: Some(deg),
                })
            }
            3 => {
                let ([a], [b]) = (items[1], items[2]) else {
                    return None;
                };
                let d1 = parse_angle_token_deg(a)?;
                let d2 = parse_angle_token_deg(b)?;
                if !(-90.0..=90.0).contains(&d1) || !(-90.0..=90.0).contains(&d2) || d1 > d2 {
                    return None;
                }
                Some(FontFaceStyle::Oblique {
                    min_deg: Some(d1),
                    max_deg: Some(d2),
                })
            }
            _ => None,
        },
        _ => None,
    }
}

fn parse_font_stretch_descriptor(values: &[ComponentValue]) -> Option<FontStretchRange> {
    let items = split_whitespace_items(values);
    let parse_one = |slice: &[ComponentValue]| -> Option<f32> {
        let [cv] = slice else { return None };
        match cv.as_token()? {
            Token::Percentage(p) if p.is_finite() && *p > 0.0 => Some(*p as f32),
            Token::Ident(kw) => Some(match kw.to_ascii_lowercase().as_str() {
                "ultra-condensed" => 50.0,
                "extra-condensed" => 62.5,
                "condensed" => 75.0,
                "semi-condensed" => 87.5,
                "normal" => 100.0,
                "semi-expanded" => 112.5,
                "expanded" => 125.0,
                "extra-expanded" => 150.0,
                "ultra-expanded" => 200.0,
                _ => return None,
            }),
            _ => None,
        }
    };
    match items.as_slice() {
        [only] => {
            let p = parse_one(only)?;
            Some(FontStretchRange {
                min_percent: p,
                max_percent: p,
            })
        }
        [lo, hi] => {
            let a = parse_one(lo)?;
            let b = parse_one(hi)?;
            if a > b {
                return None;
            }
            Some(FontStretchRange {
                min_percent: a,
                max_percent: b,
            })
        }
        _ => None,
    }
}

fn parse_font_display_descriptor(values: &[ComponentValue]) -> Option<FontDisplay> {
    let [cv] = trim_ws(values) else { return None };
    match cv.as_ident()?.to_ascii_lowercase().as_str() {
        "auto" => Some(FontDisplay::Auto),
        "block" => Some(FontDisplay::Block),
        "swap" => Some(FontDisplay::Swap),
        "fallback" => Some(FontDisplay::Fallback),
        "optional" => Some(FontDisplay::Optional),
        _ => None,
    }
}

fn parse_unicode_range_list(values: &[ComponentValue]) -> Vec<UnicodeRange> {
    let mut out = Vec::new();
    for group in split_comma_groups(values) {
        if let Some(ur) = parse_single_unicode_range(group) {
            out.push(ur);
        }
    }
    out
}

fn parse_single_unicode_range(values: &[ComponentValue]) -> Option<UnicodeRange> {
    // Reconstruct the contiguous token text of one unicode-range item (e.g.
    // `U` `+0025` `-00FF` or `U` `+4` `?` `?`) without whitespace inside.
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    let mut raw = String::new();
    for cv in trimmed {
        if cv.is_whitespace() {
            return None;
        }
        match cv.as_token()? {
            Token::Ident(s) => raw.push_str(s),
            Token::Delim(c) => raw.push(*c),
            Token::Number { raw: num_raw, .. } => raw.push_str(num_raw),
            Token::Dimension { value, unit } => {
                // Reconstruct leading sign for unicode-range tokens such as
                // `+00A0` (following `U`) or `-00FF` (`-0.0_f64` has negative sign bit).
                let neg = value.is_sign_negative();
                let abs_int = value.abs() as u64;
                if neg {
                    raw.push('-');
                } else if raw.eq_ignore_ascii_case("u") {
                    raw.push('+');
                }
                let _ = std::fmt::Write::write_fmt(&mut raw, format_args!("{abs_int}{unit}"));
            }
            _ => return None,
        }
    }
    let rest = raw.strip_prefix("U+").or_else(|| raw.strip_prefix("u+"))?;
    if rest.is_empty() {
        return None;
    }
    if let Some((a, b)) = rest.split_once('-') {
        if a.is_empty() || a.len() > 6 || b.is_empty() || b.len() > 6 {
            return None;
        }
        let start = u32::from_str_radix(a, 16).ok()?;
        let end = u32::from_str_radix(b, 16).ok()?;
        if start <= end && end <= 0x10_FFFF {
            return Some(UnicodeRange { start, end });
        }
        return None;
    }
    if rest.len() > 6 {
        return None;
    }
    if rest.contains('?') {
        // Wildcards may only appear as a trailing run.
        let q_start = rest.find('?')?;
        let prefix = &rest[..q_start];
        let wild = &rest[q_start..];
        if !wild.bytes().all(|b| b == b'?') {
            return None;
        }
        let lo_str = format!("{prefix}{}", "0".repeat(wild.len()));
        let hi_str = format!("{prefix}{}", "F".repeat(wild.len()));
        let start = u32::from_str_radix(&lo_str, 16).ok()?;
        let end = u32::from_str_radix(&hi_str, 16).ok()?.min(0x10_FFFF);
        return (start <= end).then_some(UnicodeRange { start, end });
    }
    let cp = u32::from_str_radix(rest, 16).ok()?;
    (cp <= 0x10_FFFF).then_some(UnicodeRange { start: cp, end: cp })
}

// ============================================================================
// Shared boolean-chain & range-syntax helpers
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoolOp {
    And,
    Or,
}

/// Split `a and b and c` or `a or b or c`. Rejects mixing `and` and `or` at the
/// same nesting level per CSS Media Queries 4 / Conditional Rules 3.
fn split_boolean_chain(
    values: &[ComponentValue],
) -> Option<(Option<BoolOp>, Vec<&[ComponentValue]>)> {
    let mut op: Option<BoolOp> = None;
    let mut parts = Vec::new();
    let mut seg_start = 0;

    for (i, cv) in values.iter().enumerate() {
        let Some(ident) = cv.as_ident() else {
            continue;
        };
        let this_op = if ident.eq_ignore_ascii_case("and") {
            Some(BoolOp::And)
        } else if ident.eq_ignore_ascii_case("or") {
            Some(BoolOp::Or)
        } else {
            None
        };
        if let Some(found) = this_op {
            if let Some(prev) = op {
                if prev != found {
                    return None;
                }
            } else {
                op = Some(found);
            }
            let seg = trim_ws(&values[seg_start..i]);
            if seg.is_empty() {
                return None;
            }
            parts.push(seg);
            seg_start = i + 1;
        }
    }

    let tail = trim_ws(&values[seg_start..]);
    if tail.is_empty() {
        return None;
    }
    parts.push(tail);
    Some((op, parts))
}

fn looks_like_nested_condition(inner: &[ComponentValue]) -> bool {
    let Some(first) = inner.first() else {
        return false;
    };
    if matches!(
        first,
        ComponentValue::SimpleBlock {
            kind: BlockKind::Paren,
            ..
        }
    ) {
        return true;
    }
    if let Some(kw) = first.as_ident()
        && kw.eq_ignore_ascii_case("not")
    {
        return true;
    }
    inner.iter().any(|cv| {
        cv.as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("and") || s.eq_ignore_ascii_case("or"))
    })
}

/// Parse range feature syntax:
/// - `<name> <op> <value>` (e.g. `width >= 400px`)
/// - `<value> <op> <name>` (e.g. `400px <= width`)
/// - `<value1> <op1> <name> <op2> <value2>` (e.g. `300px < inline-size <= 600px`)
///
/// Returns `(lowercase_feature_name, normalized_bounds)` where every bound is
/// expressed as `feature <op> value`.
fn parse_range_parts(
    inner: &[ComponentValue],
) -> Option<(String, Vec<(CompareOp, Vec<ComponentValue>)>)> {
    // Locate comparison operators (`<`, `<=`, `=`, `>`, `>=`) at top level.
    let mut ops: Vec<(usize, usize, CompareOp)> = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        match inner[i].as_token() {
            Some(Token::Delim('<')) => {
                if matches!(
                    inner.get(i + 1).and_then(ComponentValue::as_token),
                    Some(Token::Delim('='))
                ) {
                    ops.push((i, i + 2, CompareOp::Le));
                    i += 2;
                } else {
                    ops.push((i, i + 1, CompareOp::Lt));
                    i += 1;
                }
            }
            Some(Token::Delim('>')) => {
                if matches!(
                    inner.get(i + 1).and_then(ComponentValue::as_token),
                    Some(Token::Delim('='))
                ) {
                    ops.push((i, i + 2, CompareOp::Ge));
                    i += 2;
                } else {
                    ops.push((i, i + 1, CompareOp::Gt));
                    i += 1;
                }
            }
            Some(Token::Delim('=')) => {
                ops.push((i, i + 1, CompareOp::Eq));
                i += 1;
            }
            _ => i += 1,
        }
    }

    match ops.as_slice() {
        [(op_start, op_end, op)] => {
            let lhs = trim_ws(&inner[..*op_start]);
            let rhs = trim_ws(&inner[*op_end..]);
            if lhs.is_empty() || rhs.is_empty() {
                return None;
            }
            if let [cv] = lhs
                && let Some(name) = cv.as_ident()
            {
                return Some((name.to_ascii_lowercase(), vec![(*op, rhs.to_vec())]));
            }
            if let [cv] = rhs
                && let Some(name) = cv.as_ident()
            {
                return Some((name.to_ascii_lowercase(), vec![(op.flip(), lhs.to_vec())]));
            }
            None
        }
        [(op1_s, op1_e, op1), (op2_s, op2_e, op2)] => {
            // Both operators must point in the same direction (`<`/`<=` or `>`/`>=`, never `=` or mixed).
            let both_asc = matches!(op1, CompareOp::Lt | CompareOp::Le)
                && matches!(op2, CompareOp::Lt | CompareOp::Le);
            let both_desc = matches!(op1, CompareOp::Gt | CompareOp::Ge)
                && matches!(op2, CompareOp::Gt | CompareOp::Ge);
            if !both_asc && !both_desc {
                return None;
            }
            let left = trim_ws(&inner[..*op1_s]);
            let mid = trim_ws(&inner[*op1_e..*op2_s]);
            let right = trim_ws(&inner[*op2_e..]);
            if left.is_empty() || right.is_empty() {
                return None;
            }
            let [mid_cv] = mid else { return None };
            let name = mid_cv.as_ident()?.to_ascii_lowercase();
            Some((
                name,
                vec![(op1.flip(), left.to_vec()), (*op2, right.to_vec())],
            ))
        }
        _ => None,
    }
}
