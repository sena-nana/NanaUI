//! CSS `@container`, compiled into data the runtime evaluates.
//!
//! This engine never measures a container: sizes stay the runtime's, the one
//! layout authority. An `@container` block parses into a [`ContainerRule`]
//! whose query is compiled to data: the container it asks (a name, or the
//! nearest container), the axis it reads, and the extents of that axis where
//! it holds, a union of half-open intervals `[lo, hi)`. Its rules stay out of
//! the unconditional cascade.
//!
//! For one element:
//! 1. [`ContainerRuleSet::matching`] finds the container rules its selectors
//!    match;
//! 2. [`ContainerRuleSet::plan`] (over [`plan_container_queries`]) cuts the
//!    container's extent at every bound those queries use, into at most
//!    [`MAX_CONTAINER_BREAKPOINTS`] + 1 buckets, and lists the rules that hold
//!    in each;
//! 3. [`crate::css_cascade::rebuild_layout_style_indexed_with_extra`] cascades
//!    each bucket's rules ([`ContainerRuleSet::active_rules`]) with the
//!    sheet's own, in cascade order.
//!
//! The buckets become one responsive rule of the runtime (the nearest
//! container of that name eligible for that axis), which picks the bucket
//! from the container's measured size.
//!
//! Subset: a prelude `[<name>] <condition>` with `and`, `or` and `not`; the
//! features `width`, `height`, `inline-size` and `block-size` with `min-` /
//! `max-`, in plain (`(width: 480px)`), boolean (`(width)`) and range form
//! (`(width < 480px)`, `(400px <= width < 800px)`); lengths in `px`, or a
//! unitless `0`. Bounds follow CSS: `max-width: 480px` includes 480, so its
//! interval ends at `480f32.next_up()`; `width < 480px` ends at 480.
//!
//! Everything else is [`ContainerQueryUnsupported`] and never applies: other
//! units, other features (`aspect-ratio`, `orientation`), `style()`,
//! `scroll-state()` and other functions, features on two axes in one query, a
//! list of conditions, a name with no condition, and an `@container` inside
//! another. Only the style rules of a block take part: `:hover` and the other
//! interactive states, pseudo-elements, transitions, `@keyframes` and
//! `@font-face` inside an `@container` block are not applied.

use std::fmt;

use nana_ui_core::ContainerType;

use crate::css_cascade::{MatchContext, RuleIndex, StyleRule, selector_matches};
use crate::css_interactive::ParsedStylesheet;

/// Most breakpoints one element's container rules may use: the runtime's
/// `MAX_RESPONSIVE_BREAKPOINTS`, so a plan always fits one responsive rule.
pub const MAX_CONTAINER_BREAKPOINTS: usize = 16;

/// The container axis a query reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerAxis {
    /// `inline-size`: the container's size along its inline axis.
    Inline,
    /// `block-size`: along its block axis.
    Block,
    /// `width`: the physical width.
    Width,
    /// `height`: the physical height.
    Height,
}

/// The extents `lo <= x < hi` of a container axis. `lo` may be
/// `f32::NEG_INFINITY` and `hi` `f32::INFINITY`; other bounds are finite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContainerInterval {
    pub lo: f32,
    pub hi: f32,
}

impl ContainerInterval {
    pub fn contains(self, extent: f32) -> bool {
        self.lo <= extent && extent < self.hi
    }
}

/// A compiled container query.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerQuery {
    /// `@container sidebar (…)`: the nearest container named `sidebar`.
    /// `None`: the nearest container eligible for [`Self::axis`].
    pub name: Option<String>,
    pub axis: ContainerAxis,
    /// Where the query holds: ascending, disjoint, never touching, none
    /// empty. Empty when it holds nowhere.
    pub intervals: Vec<ContainerInterval>,
}

impl ContainerQuery {
    /// Whether the query holds for a container whose [`Self::axis`]
    /// measures `extent`.
    pub fn holds_at(&self, extent: f32) -> bool {
        self.intervals
            .iter()
            .any(|interval| interval.contains(extent))
    }
}

/// Why an `@container` block never applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerQueryUnsupported {
    /// The prelude is not a container condition.
    Invalid,
    /// A comma-separated list of conditions.
    ConditionList,
    /// A container name with no condition.
    NoCondition,
    /// `style()`, `scroll-state()`, `calc()` or another function.
    Function(String),
    /// A feature other than `width`, `height`, `inline-size` and
    /// `block-size` (`aspect-ratio`, `orientation`, …).
    Feature(String),
    /// A length in a unit other than `px`.
    Unit(String),
    /// Features on more than one axis in one query.
    MixedAxes,
    /// An `@container` inside another `@container`.
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

/// A parsed `@container` block. Its rules apply to an element only where
/// `query` holds for the element's container; they never join the
/// unconditional cascade, and an unsupported query holds nowhere.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerRule {
    pub query: Result<ContainerQuery, ContainerQueryUnsupported>,
    pub sheet: ParsedStylesheet,
}

/// How one element's container rules cut its container's extent.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerPlan {
    /// The container's name; `None`: the nearest eligible container.
    pub name: Option<String>,
    pub axis: ContainerAxis,
    /// Ascending, unique and finite; at most [`MAX_CONTAINER_BREAKPOINTS`].
    /// Bucket 0 covers `(-inf, breakpoints[0])`, bucket `i` covers
    /// `[breakpoints[i - 1], breakpoints[i])` and the last one
    /// `[breakpoints[n - 1], +inf)`.
    pub breakpoints: Vec<f32>,
    /// For each of the `breakpoints.len() + 1` buckets, the rules whose
    /// query holds there, in the order they were given. They index the list
    /// the plan was made from: the `queries` of [`plan_container_queries`],
    /// or [`ContainerRuleSet::rules`] for [`ContainerRuleSet::plan`].
    pub active: Vec<Vec<usize>>,
}

impl ContainerPlan {
    /// The bucket a container measuring `extent` along [`Self::axis`] is in.
    pub fn bucket_for(&self, extent: f32) -> usize {
        self.breakpoints.partition_point(|bound| *bound <= extent)
    }
}

/// Why one element's container rules cannot be planned. None of them then
/// applies to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerPlanUnsupported {
    /// The rules ask more than one container, or more than one axis of it;
    /// a plan reads one.
    MixedContainers,
    /// Their queries use more than [`MAX_CONTAINER_BREAKPOINTS`] distinct
    /// bounds.
    TooManyBreakpoints,
}

impl fmt::Display for ContainerPlanUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MixedContainers => {
                f.write_str("its @container rules ask more than one container or axis")
            }
            Self::TooManyBreakpoints => write!(
                f,
                "its @container rules use more than {MAX_CONTAINER_BREAKPOINTS} breakpoints"
            ),
        }
    }
}

/// Plans the buckets of one element's container rules, given their queries.
///
/// Every finite bound of every supported query is a breakpoint, so a query
/// holds across a whole bucket or nowhere in it; each one is evaluated at its
/// bucket's lower bound (`f32::NEG_INFINITY` for bucket 0). An unsupported
/// query holds in no bucket. `Ok(None)`: no query is supported, so there is
/// nothing to plan.
pub fn plan_container_queries<'q>(
    queries: impl IntoIterator<Item = &'q Result<ContainerQuery, ContainerQueryUnsupported>>,
) -> Result<Option<ContainerPlan>, ContainerPlanUnsupported> {
    let queries: Vec<Option<&'q ContainerQuery>> = queries
        .into_iter()
        .map(|query| query.as_ref().ok())
        .collect();
    let mut key: Option<(&'q Option<String>, ContainerAxis)> = None;
    let mut breakpoints = Vec::new();
    for query in queries.iter().copied().flatten() {
        match key {
            None => key = Some((&query.name, query.axis)),
            Some((name, axis)) if *name == query.name && axis == query.axis => {}
            Some(_) => return Err(ContainerPlanUnsupported::MixedContainers),
        }
        for interval in &query.intervals {
            breakpoints.extend(
                [interval.lo, interval.hi]
                    .into_iter()
                    .filter(|bound| bound.is_finite()),
            );
        }
    }
    let Some((name, axis)) = key else {
        return Ok(None);
    };
    breakpoints.sort_by(f32::total_cmp);
    breakpoints.dedup();
    if breakpoints.len() > MAX_CONTAINER_BREAKPOINTS {
        return Err(ContainerPlanUnsupported::TooManyBreakpoints);
    }
    let active = (0..=breakpoints.len())
        .map(|bucket| {
            let lower = bucket
                .checked_sub(1)
                .map_or(f32::NEG_INFINITY, |below| breakpoints[below]);
            queries
                .iter()
                .enumerate()
                .filter_map(|(index, query)| {
                    query.filter(|query| query.holds_at(lower)).map(|_| index)
                })
                .collect()
        })
        .collect();
    Ok(Some(ContainerPlan {
        name: name.clone(),
        axis,
        breakpoints,
        active,
    }))
}

/// The style rules of a flattened sheet's `@container` blocks, indexed for
/// matching like ordinary rules ([`RuleIndex`]).
#[derive(Debug, Clone, Default)]
pub struct ContainerRuleSet {
    /// One per block that holds a rule.
    queries: Vec<Result<ContainerQuery, ContainerQueryUnsupported>>,
    rules: Vec<StyleRule>,
    /// Parallel to `rules`: the rule's block, in `queries`.
    block_of: Vec<usize>,
    index: RuleIndex,
}

impl ContainerRuleSet {
    /// The style rules of `blocks`: the container rules of a flattened
    /// sheet ([`ParsedStylesheet::flatten`]). Rules still under an `@media`
    /// or `@container` inside a block are not taken: flattening resolves the
    /// first, and the parser lifts the second out as
    /// [`ContainerQueryUnsupported::Nested`].
    pub fn build(blocks: &[ContainerRule]) -> Self {
        let mut set = Self::default();
        for block in blocks {
            if block.sheet.static_rules.is_empty() {
                continue;
            }
            let at = set.queries.len();
            set.queries.push(block.query.clone());
            for rule in &block.sheet.static_rules {
                set.rules.push(rule.clone());
                set.block_of.push(at);
            }
        }
        set.index = RuleIndex::build(&set.rules);
        set
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Every container style rule, each with its own source order.
    pub fn rules(&self) -> &[StyleRule] {
        &self.rules
    }

    /// The query guarding rule `rule` of [`Self::rules`].
    pub fn query(&self, rule: usize) -> &Result<ContainerQuery, ContainerQueryUnsupported> {
        &self.queries[self.block_of[rule]]
    }

    /// The rules whose selector matches `ctx`, ascending: indices into
    /// [`Self::rules`].
    pub fn matching(&self, ctx: &MatchContext<'_>) -> Vec<usize> {
        self.index
            .candidate_ids(ctx)
            .into_iter()
            .map(|id| id as usize)
            .filter(|&rule| {
                self.rules[rule]
                    .selectors
                    .iter()
                    .any(|selector| selector_matches(selector, ctx))
            })
            .collect()
    }

    /// Whether one of `matched` has an unsupported query, so it never
    /// applies: the element is counted by
    /// [`crate::css_cascade::UnsupportedCssTally::observe_container_queries`].
    pub fn any_unsupported(&self, matched: &[usize]) -> bool {
        matched.iter().any(|&rule| self.query(rule).is_err())
    }

    /// [`plan_container_queries`] for `matched` ([`Self::matching`]);
    /// [`ContainerPlan::active`] holds indices into [`Self::rules`].
    pub fn plan(
        &self,
        matched: &[usize],
    ) -> Result<Option<ContainerPlan>, ContainerPlanUnsupported> {
        let mut plan = plan_container_queries(matched.iter().map(|&rule| self.query(rule)))?;
        if let Some(plan) = &mut plan {
            for bucket in &mut plan.active {
                for rule in bucket.iter_mut() {
                    *rule = matched[*rule];
                }
            }
        }
        Ok(plan)
    }

    /// The rules of `plan` (from [`Self::plan`]) that hold in `bucket`, for
    /// [`crate::css_cascade::rebuild_layout_style_indexed_with_extra`].
    pub fn active_rules(&self, plan: &ContainerPlan, bucket: usize) -> Vec<&StyleRule> {
        plan.active
            .get(bucket)
            .into_iter()
            .flatten()
            .map(|&rule| &self.rules[rule])
            .collect()
    }
}

/// Lifts every `@container` block out of `sheet`, its own and those under
/// its `@media` blocks: what the parser found inside another `@container`.
pub(crate) fn take_nested_container_rules(sheet: &mut ParsedStylesheet) -> Vec<ContainerRule> {
    let mut lifted = std::mem::take(&mut sheet.container_rules);
    for media in &mut sheet.media_rules {
        lifted.extend(take_nested_container_rules(&mut media.sheet));
    }
    lifted
}

/// Parses an `@container` prelude: `[<name>] <condition>`.
pub fn parse_container_prelude(prelude: &str) -> Result<ContainerQuery, ContainerQueryUnsupported> {
    use ContainerQueryUnsupported::{ConditionList, Invalid, MixedAxes, NoCondition};
    let tokens = tokenize(prelude);
    let mut depth = 0i32;
    for token in &tokens {
        match token {
            Token::Open | Token::Function(_) => depth += 1,
            Token::Close => depth -= 1,
            Token::Comma if depth == 0 => return Err(ConditionList),
            _ => {}
        }
    }
    let name = match tokens.first() {
        Some(Token::Ident(word)) if !word.eq_ignore_ascii_case("not") => {
            if !is_container_name(word) {
                return Err(Invalid);
            }
            Some(word.clone())
        }
        _ => None,
    };
    let mut parser = Parser {
        tokens: &tokens,
        pos: usize::from(name.is_some()),
    };
    if parser.pos == tokens.len() {
        return Err(if name.is_some() { NoCondition } else { Invalid });
    }
    let condition = parser.condition()?;
    if parser.pos != tokens.len() {
        return Err(Invalid);
    }
    let mut axes = Vec::new();
    condition.axes(&mut axes);
    let axis = *axes.first().ok_or(Invalid)?;
    if axes.iter().any(|other| *other != axis) {
        return Err(MixedAxes);
    }
    Ok(ContainerQuery {
        name,
        axis,
        intervals: condition.intervals(),
    })
}

/// `container-type`: `normal | [ size | inline-size ] || scroll-state`, or
/// `initial` / `unset`. `scroll-state` alone answers no size query, so it is
/// [`ContainerType::Normal`] here.
pub fn parse_container_type(value: &str) -> Option<ContainerType> {
    if is_reset(value) {
        return Some(ContainerType::Normal);
    }
    container_type_words(value)
}

/// `container-name`: `none | <custom-ident>+`, or `initial` / `unset`.
/// Names are case-sensitive.
pub fn parse_container_name(value: &str) -> Option<Vec<String>> {
    if is_reset(value) {
        return Some(Vec::new());
    }
    container_names(value)
}

/// The `container` shorthand, `<'container-name'> [ / <'container-type'> ]?`:
/// the names and the type, `normal` when it is left out.
pub fn parse_container_shorthand(value: &str) -> Option<(Vec<String>, ContainerType)> {
    if is_reset(value) {
        return Some((Vec::new(), ContainerType::Normal));
    }
    let (names, container_type) = match value.split_once('/') {
        Some((names, container_type)) => (names, container_type_words(container_type)?),
        None => (value, ContainerType::Normal),
    };
    Some((container_names(names)?, container_type))
}

fn is_reset(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("initial") || value.eq_ignore_ascii_case("unset")
}

fn container_type_words(value: &str) -> Option<ContainerType> {
    let lower = value.trim().to_ascii_lowercase();
    if lower == "normal" {
        return Some(ContainerType::Normal);
    }
    let mut size = None;
    let mut scroll_state = false;
    for word in lower.split_whitespace() {
        match word {
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

fn container_names(value: &str) -> Option<Vec<String>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let names: Vec<String> = value.split_whitespace().map(str::to_owned).collect();
    let valid = !names.is_empty()
        && names
            .iter()
            .all(|name| is_ident(name) && is_container_name(name));
    valid.then_some(names)
}

/// A `<custom-ident>` a container may be named: not a keyword of the
/// grammar nor a CSS-wide keyword.
fn is_container_name(word: &str) -> bool {
    const RESERVED: [&str; 10] = [
        "none",
        "and",
        "or",
        "not",
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ];
    !RESERVED
        .iter()
        .any(|reserved| word.eq_ignore_ascii_case(reserved))
}

fn is_ident(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    starts_ident(&chars, 0) && ident_end(&chars, 0) == chars.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Cmp {
    /// `value <op> feature` as `feature <op'> value`.
    fn flip(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::Le => Self::Ge,
            Self::Gt => Self::Lt,
            Self::Ge => Self::Le,
            Self::Eq => Self::Eq,
        }
    }

    /// Both ends of a two-sided range point the same way; `=` never does.
    fn same_direction(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Lt | Self::Le, Self::Lt | Self::Le) | (Self::Gt | Self::Ge, Self::Gt | Self::Ge)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    /// An identifier directly followed by `(`, which it consumes.
    Function(String),
    Open,
    Close,
    /// `unit` is empty for a plain number and `%` for a percentage.
    Number {
        value: f32,
        unit: String,
    },
    Cmp(Cmp),
    Colon,
    Comma,
    Other,
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-'
}

fn starts_ident(chars: &[char], at: usize) -> bool {
    match chars.get(at) {
        Some('-') => chars
            .get(at + 1)
            .is_some_and(|&next| is_name_start(next) || next == '-'),
        Some(&c) => is_name_start(c),
        None => false,
    }
}

fn ident_end(chars: &[char], mut at: usize) -> usize {
    while chars.get(at).is_some_and(|&c| is_name_char(c)) {
        at += 1;
    }
    at
}

fn starts_number(chars: &[char], at: usize) -> bool {
    let digit = |k: usize| chars.get(k).is_some_and(char::is_ascii_digit);
    match chars.get(at) {
        Some('+' | '-') => digit(at + 1) || (chars.get(at + 1) == Some(&'.') && digit(at + 2)),
        Some('.') => digit(at + 1),
        Some(c) => c.is_ascii_digit(),
        None => false,
    }
}

/// A number and its unit, from `start`; the token and where it ends.
fn number(chars: &[char], start: usize) -> (Token, usize) {
    let digit = |k: usize| chars.get(k).is_some_and(char::is_ascii_digit);
    let mut at = start;
    if matches!(chars.get(at), Some('+' | '-')) {
        at += 1;
    }
    while digit(at) {
        at += 1;
    }
    if chars.get(at) == Some(&'.') && digit(at + 1) {
        at += 1;
        while digit(at) {
            at += 1;
        }
    }
    if matches!(chars.get(at), Some('e' | 'E')) {
        let first = at + 1 + usize::from(matches!(chars.get(at + 1), Some('+' | '-')));
        if digit(first) {
            at = first;
            while digit(at) {
                at += 1;
            }
        }
    }
    let text: String = chars[start..at].iter().collect();
    let value = text.parse::<f32>().unwrap_or(f32::NAN);
    let unit = if chars.get(at) == Some(&'%') {
        at += 1;
        "%".to_owned()
    } else if starts_ident(chars, at) {
        let end = ident_end(chars, at);
        let unit = chars[at..end].iter().collect();
        at = end;
        unit
    } else {
        String::new()
    };
    (Token::Number { value, unit }, at)
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while let Some(&c) = chars.get(at) {
        if c.is_whitespace() {
            at += 1;
            continue;
        }
        if starts_number(&chars, at) {
            let (token, end) = number(&chars, at);
            tokens.push(token);
            at = end;
            continue;
        }
        if starts_ident(&chars, at) {
            let end = ident_end(&chars, at);
            let word: String = chars[at..end].iter().collect();
            if chars.get(end) == Some(&'(') {
                tokens.push(Token::Function(word));
                at = end + 1;
            } else {
                tokens.push(Token::Ident(word));
                at = end;
            }
            continue;
        }
        let token = match c {
            '(' => Token::Open,
            ')' => Token::Close,
            ':' => Token::Colon,
            ',' => Token::Comma,
            '=' => Token::Cmp(Cmp::Eq),
            '<' | '>' => {
                // `<=` only without whitespace between the two.
                let or_equal = chars.get(at + 1) == Some(&'=');
                at += usize::from(or_equal);
                Token::Cmp(match (c, or_equal) {
                    ('<', false) => Cmp::Lt,
                    ('<', true) => Cmp::Le,
                    (_, false) => Cmp::Gt,
                    (_, true) => Cmp::Ge,
                })
            }
            _ => Token::Other,
        };
        tokens.push(token);
        at += 1;
    }
    tokens
}

enum Condition {
    Feature(ContainerAxis, Vec<ContainerInterval>),
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
}

impl Condition {
    fn axes(&self, out: &mut Vec<ContainerAxis>) {
        match self {
            Self::Feature(axis, _) => out.push(*axis),
            Self::Not(inner) => inner.axes(out),
            Self::And(terms) | Self::Or(terms) => {
                for term in terms {
                    term.axes(out);
                }
            }
        }
    }

    fn intervals(&self) -> Vec<ContainerInterval> {
        match self {
            Self::Feature(_, intervals) => intervals.clone(),
            Self::Not(inner) => complement(&inner.intervals()),
            Self::And(terms) => terms
                .iter()
                .map(Self::intervals)
                .reduce(|a, b| intersect(&a, &b))
                .unwrap_or_default(),
            Self::Or(terms) => terms
                .iter()
                .map(Self::intervals)
                .reduce(|a, b| union(&a, &b))
                .unwrap_or_default(),
        }
    }
}

struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
}

impl Parser<'_> {
    fn keyword(&self, word: &str) -> bool {
        matches!(self.tokens.get(self.pos), Some(Token::Ident(w)) if w.eq_ignore_ascii_case(word))
    }

    /// `not <query-in-parens>`, or `<query-in-parens>`s joined by `and` or
    /// by `or`, never both.
    fn condition(&mut self) -> Result<Condition, ContainerQueryUnsupported> {
        if self.keyword("not") {
            self.pos += 1;
            return Ok(Condition::Not(Box::new(self.in_parens()?)));
        }
        let mut terms = vec![self.in_parens()?];
        let mut joined_by_and = None;
        loop {
            let and = if self.keyword("and") {
                true
            } else if self.keyword("or") {
                false
            } else {
                break;
            };
            if joined_by_and.is_some_and(|previous| previous != and) {
                return Err(ContainerQueryUnsupported::Invalid);
            }
            joined_by_and = Some(and);
            self.pos += 1;
            terms.push(self.in_parens()?);
        }
        Ok(match joined_by_and {
            None => terms.swap_remove(0),
            Some(true) => Condition::And(terms),
            Some(false) => Condition::Or(terms),
        })
    }

    /// `( <condition> )`, `( <size-feature> )`, or a function such as
    /// `style()`, which is never evaluated.
    fn in_parens(&mut self) -> Result<Condition, ContainerQueryUnsupported> {
        let open = self.pos;
        let function = match self.tokens.get(open) {
            Some(Token::Open) => None,
            Some(Token::Function(name)) => Some(name.to_ascii_lowercase()),
            _ => return Err(ContainerQueryUnsupported::Invalid),
        };
        let close = closing(self.tokens, open + 1).ok_or(ContainerQueryUnsupported::Invalid)?;
        self.pos = close + 1;
        if let Some(name) = function {
            return Err(ContainerQueryUnsupported::Function(name));
        }
        let inner = &self.tokens[open + 1..close];
        let nested = match inner {
            [Token::Open | Token::Function(_), ..] => true,
            [Token::Ident(word), Token::Open | Token::Function(_), ..] => {
                word.eq_ignore_ascii_case("not")
            }
            _ => false,
        };
        if !nested {
            return size_feature(inner);
        }
        let mut parser = Parser {
            tokens: inner,
            pos: 0,
        };
        let condition = parser.condition()?;
        if parser.pos == inner.len() {
            Ok(condition)
        } else {
            Err(ContainerQueryUnsupported::Invalid)
        }
    }
}

/// The `)` closing the group that starts at `from`.
fn closing(tokens: &[Token], from: usize) -> Option<usize> {
    let mut depth = 1usize;
    for (at, token) in tokens.iter().enumerate().skip(from) {
        match token {
            Token::Open | Token::Function(_) => depth += 1,
            Token::Close => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prefix {
    Min,
    Max,
}

fn feature(name: &str) -> Result<(ContainerAxis, Option<Prefix>), ContainerQueryUnsupported> {
    let lower = name.to_ascii_lowercase();
    let (prefix, base) = if let Some(base) = lower.strip_prefix("min-") {
        (Some(Prefix::Min), base)
    } else if let Some(base) = lower.strip_prefix("max-") {
        (Some(Prefix::Max), base)
    } else {
        (None, lower.as_str())
    };
    let axis = match base {
        "width" => ContainerAxis::Width,
        "height" => ContainerAxis::Height,
        "inline-size" => ContainerAxis::Inline,
        "block-size" => ContainerAxis::Block,
        _ => return Err(ContainerQueryUnsupported::Feature(lower)),
    };
    Ok((axis, prefix))
}

/// A feature in boolean or range form, where `min-` / `max-` is invalid.
fn unprefixed_feature(name: &str) -> Result<ContainerAxis, ContainerQueryUnsupported> {
    match feature(name)? {
        (axis, None) => Ok(axis),
        (_, Some(_)) => Err(ContainerQueryUnsupported::Invalid),
    }
}

fn length(token: &Token) -> Result<f32, ContainerQueryUnsupported> {
    let Token::Number { value, unit } = token else {
        return Err(ContainerQueryUnsupported::Invalid);
    };
    if !value.is_finite() {
        return Err(ContainerQueryUnsupported::Invalid);
    }
    if !unit.eq_ignore_ascii_case("px") && !(unit.is_empty() && *value == 0.0) {
        return Err(if unit.is_empty() {
            ContainerQueryUnsupported::Invalid
        } else {
            ContainerQueryUnsupported::Unit(unit.to_ascii_lowercase())
        });
    }
    // `-0px` is 0: one breakpoint, not two.
    Ok(if *value == 0.0 { 0.0 } else { *value })
}

fn size_feature(tokens: &[Token]) -> Result<Condition, ContainerQueryUnsupported> {
    let function = tokens.iter().find_map(|token| match token {
        Token::Function(name) => Some(name),
        _ => None,
    });
    if let Some(name) = function {
        return Err(ContainerQueryUnsupported::Function(
            name.to_ascii_lowercase(),
        ));
    }
    let (axis, intervals) = match tokens {
        // Boolean context: true for any extent but 0.
        [Token::Ident(name)] => (
            unprefixed_feature(name)?,
            complement(&compare(Cmp::Eq, 0.0)),
        ),
        [Token::Ident(name), Token::Colon, value] => {
            let (axis, prefix) = feature(name)?;
            let cmp = match prefix {
                Some(Prefix::Min) => Cmp::Ge,
                Some(Prefix::Max) => Cmp::Le,
                None => Cmp::Eq,
            };
            (axis, compare(cmp, length(value)?))
        }
        [Token::Ident(name), Token::Cmp(cmp), value] => {
            (unprefixed_feature(name)?, compare(*cmp, length(value)?))
        }
        [value, Token::Cmp(cmp), Token::Ident(name)] => (
            unprefixed_feature(name)?,
            compare(cmp.flip(), length(value)?),
        ),
        [
            low,
            Token::Cmp(first),
            Token::Ident(name),
            Token::Cmp(second),
            high,
        ] => {
            let axis = unprefixed_feature(name)?;
            if !first.same_direction(*second) {
                return Err(ContainerQueryUnsupported::Invalid);
            }
            let from = compare(first.flip(), length(low)?);
            let to = compare(*second, length(high)?);
            (axis, intersect(&from, &to))
        }
        _ => {
            // Name the feature when there is one: that is the better reason.
            let name = tokens.iter().find_map(|token| match token {
                Token::Ident(name) => Some(name),
                _ => None,
            });
            if let Some(name) = name {
                feature(name)?;
            }
            return Err(ContainerQueryUnsupported::Invalid);
        }
    };
    Ok(Condition::Feature(axis, intervals))
}

fn span(lo: f32, hi: f32) -> Vec<ContainerInterval> {
    if lo < hi {
        vec![ContainerInterval { lo, hi }]
    } else {
        Vec::new()
    }
}

/// `feature <cmp> px`. An inclusive upper bound ends at `px.next_up()` and
/// an exclusive lower bound starts there.
fn compare(cmp: Cmp, px: f32) -> Vec<ContainerInterval> {
    match cmp {
        Cmp::Lt => span(f32::NEG_INFINITY, px),
        Cmp::Le => span(f32::NEG_INFINITY, px.next_up()),
        Cmp::Gt => span(px.next_up(), f32::INFINITY),
        Cmp::Ge => span(px, f32::INFINITY),
        Cmp::Eq => span(px, px.next_up()),
    }
}

fn union(a: &[ContainerInterval], b: &[ContainerInterval]) -> Vec<ContainerInterval> {
    let mut all: Vec<ContainerInterval> = a.iter().chain(b).copied().collect();
    all.sort_by(|x, y| x.lo.total_cmp(&y.lo));
    let mut out: Vec<ContainerInterval> = Vec::with_capacity(all.len());
    for interval in all {
        match out.last_mut() {
            Some(last) if interval.lo <= last.hi => last.hi = last.hi.max(interval.hi),
            _ => out.push(interval),
        }
    }
    out
}

fn intersect(a: &[ContainerInterval], b: &[ContainerInterval]) -> Vec<ContainerInterval> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        out.extend(span(a[i].lo.max(b[j].lo), a[i].hi.min(b[j].hi)));
        if a[i].hi < b[j].hi {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn complement(a: &[ContainerInterval]) -> Vec<ContainerInterval> {
    let mut out = Vec::new();
    let mut from = f32::NEG_INFINITY;
    for interval in a {
        out.extend(span(from, interval.lo));
        from = interval.hi;
    }
    out.extend(span(from, f32::INFINITY));
    out
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;
    use crate::css_at_rule::{MediaEnvironment, MemoryStylesheetLoader, ParseStylesheetOptions};
    use crate::css_cascade::{
        MediaEnv, UnsupportedCssTally, collect_document_custom_properties_from_rules,
        matched_custom_properties_indexed, matched_custom_properties_indexed_with_extra,
        parse_stylesheet, parse_stylesheet_full, parse_stylesheet_full_with_options,
        rebuild_layout_style_indexed, rebuild_layout_style_indexed_with_extra,
    };
    use crate::css_interactive::{merge_parsed_stylesheet, offset_source_order};
    use crate::css_map::{LayoutStyle, LayoutStyleCss, LengthSpec};
    use ContainerQueryUnsupported as Why;

    const INF: f32 = f32::INFINITY;
    const NEG_INF: f32 = f32::NEG_INFINITY;

    fn element<'a>(
        tag: &'a str,
        id: &'a str,
        classes: &'a [String],
        attrs: &'a BTreeMap<String, String>,
    ) -> MatchContext<'a> {
        MatchContext {
            tag,
            id,
            classes,
            attrs,
            ancestors: &[],
            preceding_siblings: &[],
            sibling_index: 0,
            sibling_count: 1,
            of_type_index: 0,
            of_type_count: 1,
            has_bits: 0,
            has_args: &[],
            focus_within: false,
            is_empty: true,
            checked: false,
            media: MediaEnv::default(),
            children: &[],
            following_siblings: &[],
            all_siblings: &[],
            ancestor_subtrees: &[],
            owned_children: &[],
            owned_following: &[],
            owned_ancestor_trees: &[],
            relative: None,
            relative_id: 0,
        }
    }

    fn intervals(prelude: &str) -> Vec<(f32, f32)> {
        parse_container_prelude(prelude)
            .unwrap_or_else(|why| panic!("{prelude}: {why}"))
            .intervals
            .iter()
            .map(|interval| (interval.lo, interval.hi))
            .collect()
    }

    fn queries(preludes: &[&str]) -> Vec<Result<ContainerQuery, ContainerQueryUnsupported>> {
        preludes
            .iter()
            .map(|prelude| parse_container_prelude(prelude))
            .collect()
    }

    fn class_names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn first_class(rule: &StyleRule) -> &str {
        &rule.selectors[0].subject.classes[0]
    }

    #[test]
    fn plain_and_range_forms_compile_to_half_open_intervals() {
        let up = |px: f32| px.next_up();
        assert_eq!(intervals("(width < 480px)"), [(NEG_INF, 480.0)]);
        assert_eq!(intervals("(width <= 480px)"), [(NEG_INF, up(480.0))]);
        assert_eq!(intervals("(width > 480px)"), [(up(480.0), INF)]);
        assert_eq!(intervals("(width >= 480px)"), [(480.0, INF)]);
        assert_eq!(intervals("(width = 480px)"), [(480.0, up(480.0))]);
        assert_eq!(intervals("(width: 480px)"), [(480.0, up(480.0))]);
        assert_eq!(intervals("(min-width: 480px)"), [(480.0, INF)]);
        assert_eq!(intervals("(max-width: 480px)"), [(NEG_INF, up(480.0))]);
        assert_eq!(intervals("(480px > width)"), [(NEG_INF, 480.0)]);
        assert_eq!(intervals("(480px <= width)"), [(480.0, INF)]);
        assert_eq!(intervals("(400px <= width < 800px)"), [(400.0, 800.0)]);
        assert_eq!(intervals("(800px > width >= 400px)"), [(400.0, 800.0)]);
        assert_eq!(
            intervals("(400px < width <= 800px)"),
            [(up(400.0), up(800.0))]
        );
        assert_eq!(intervals("(WIDTH<480PX)"), [(NEG_INF, 480.0)]);
        assert_eq!(intervals("(min-width: 0)"), [(0.0, INF)]);
        assert_eq!(intervals("(width >= 1e3px)"), [(1000.0, INF)]);
        assert_eq!(intervals("(width >= -0px)"), [(0.0, INF)]);
        // Boolean context: any extent but 0.
        assert_eq!(intervals("(width)"), [(NEG_INF, 0.0), (up(0.0), INF)]);
    }

    #[test]
    fn names_axes_and_boolean_logic() {
        let query = parse_container_prelude("Sidebar (inline-size > 300px)").expect("query");
        assert_eq!(
            query.name.as_deref(),
            Some("Sidebar"),
            "names keep their case"
        );
        assert_eq!(query.axis, ContainerAxis::Inline);
        let axis = |prelude| parse_container_prelude(prelude).expect("query").axis;
        assert_eq!(axis("(block-size > 1px)"), ContainerAxis::Block);
        assert_eq!(axis("(min-height: 1px)"), ContainerAxis::Height);
        assert_eq!(axis("card not (width < 1px)"), ContainerAxis::Width);

        let up = |px: f32| px.next_up();
        assert_eq!(
            parse_container_prelude("not (width < 400px)").unwrap().name,
            None
        );
        assert_eq!(intervals("not (width < 400px)"), [(400.0, INF)]);
        assert_eq!(
            intervals("(width > 100px) and (width < 200px)"),
            [(up(100.0), 200.0)]
        );
        assert_eq!(
            intervals("(width < 300px) or (width >= 600px)"),
            [(NEG_INF, 300.0), (600.0, INF)]
        );
        assert_eq!(
            intervals("not ((width >= 300px) and (width < 600px))"),
            [(NEG_INF, 300.0), (600.0, INF)]
        );
        assert_eq!(
            intervals("((width > 100px) and (width < 200px)) or (width > 300px)"),
            [(up(100.0), 200.0), (up(300.0), INF)]
        );
        assert_eq!(
            intervals("(width < 200px) or (width >= 200px)"),
            [(NEG_INF, INF)],
            "touching intervals merge"
        );
        assert!(intervals("(width > 500px) and (width < 400px)").is_empty());
        assert!(intervals("(500px <= width < 400px)").is_empty());
    }

    #[test]
    fn unsupported_queries_never_parse_open_and_say_why() {
        let why = |prelude: &str| parse_container_prelude(prelude).expect_err(prelude);
        assert_eq!(why("style(--wide: 1)"), Why::Function("style".into()));
        assert_eq!(
            why("scroll-state(stuck: top)"),
            Why::Function("scroll-state".into())
        );
        assert_eq!(
            why("(width > 1px) and style(--x: 1)"),
            Why::Function("style".into())
        );
        assert_eq!(
            why("(width > calc(10px + 1px))"),
            Why::Function("calc".into())
        );
        assert_eq!(why("sidebar(width > 1px)"), Why::Function("sidebar".into()));
        assert_eq!(
            why("(aspect-ratio > 1)"),
            Why::Feature("aspect-ratio".into())
        );
        assert_eq!(
            why("(orientation: landscape)"),
            Why::Feature("orientation".into())
        );
        assert_eq!(
            why("(16/9 < aspect-ratio)"),
            Why::Feature("aspect-ratio".into())
        );
        assert_eq!(why("(width > 10em)"), Why::Unit("em".into()));
        assert_eq!(why("(inline-size > 50%)"), Why::Unit("%".into()));
        assert_eq!(why("(width > 1px) and (height > 1px)"), Why::MixedAxes);
        assert_eq!(why("(width > 1px) or (inline-size > 1px)"), Why::MixedAxes);
        assert_eq!(why("a (width > 1px), b (width > 2px)"), Why::ConditionList);
        assert_eq!(why("sidebar"), Why::NoCondition);
        for invalid in [
            "",
            "(width > 10)",
            "none (width > 1px)",
            "and (width > 1px)",
            "(width > 1px) and (width < 2px) or (width > 3px)",
            "not (width > 1px) and (width < 2px)",
            "(400px < width > 300px)",
            "(400px = width = 400px)",
            "(min-width > 10px)",
            "(min-width)",
            "(width < = 10px)",
            "(width > 1px",
            "width > 1px",
            "(width > 1px) (width < 2px)",
            "(width: auto)",
        ] {
            assert_eq!(why(invalid), Why::Invalid, "{invalid:?}");
        }
        assert!(Why::Unit("em".into()).to_string().contains("`em`"));
    }

    #[test]
    fn plan_buckets_for_min_max_range_and_or() {
        let plan = |preludes: &[&str]| {
            plan_container_queries(&queries(preludes))
                .expect("plannable")
                .expect("a supported query")
        };

        let min = plan(&["(min-width: 480px)"]);
        assert_eq!(min.axis, ContainerAxis::Width);
        assert_eq!(min.breakpoints, [480.0]);
        assert_eq!(min.active, [vec![], vec![0]]);
        assert_eq!(min.bucket_for(479.9), 0);
        assert_eq!(min.bucket_for(480.0), 1, "min-width includes its bound");

        let max = plan(&["(max-width: 480px)"]);
        assert_eq!(max.breakpoints, [480f32.next_up()]);
        assert_eq!(max.active, [vec![0], vec![]]);
        assert_eq!(max.bucket_for(480.0), 0, "max-width includes its bound");
        assert_eq!(max.bucket_for(480.01), 1);

        let below = plan(&["(width < 480px)"]);
        assert_eq!(below.breakpoints, [480.0]);
        assert_eq!(
            below.active[below.bucket_for(480.0)],
            Vec::<usize>::new(),
            "`<` excludes its bound"
        );

        let range = plan(&["(400px <= width < 800px)"]);
        assert_eq!(range.breakpoints, [400.0, 800.0]);
        assert_eq!(range.active, [vec![], vec![0], vec![]]);

        let either = plan(&["(width < 300px) or (width >= 600px)"]);
        assert_eq!(either.breakpoints, [300.0, 600.0]);
        assert_eq!(either.active, [vec![0], vec![], vec![0]]);

        let steps = plan(&[
            "(min-width: 400px)",
            "(min-width: 800px)",
            "(min-width: 400px)",
        ]);
        assert_eq!(
            steps.breakpoints,
            [400.0, 800.0],
            "a shared bound counts once"
        );
        assert_eq!(steps.active, [vec![], vec![0, 2], vec![0, 1, 2]]);

        let always = plan(&["(width < 0px) or (width >= 0px)"]);
        assert!(always.breakpoints.is_empty());
        assert_eq!(always.active, [vec![0]]);

        let named = plan(&["sidebar (inline-size >= 30px)"]);
        assert_eq!(named.name.as_deref(), Some("sidebar"));
        assert_eq!(named.axis, ContainerAxis::Inline);

        // An unsupported query holds in no bucket; the others still plan.
        let mixed = plan(&["(min-width: 400px)", "style(--x: 1)"]);
        assert_eq!(mixed.active, [vec![], vec![0]]);
        assert_eq!(
            plan_container_queries(&queries(&["style(--x: 1)"])),
            Ok(None)
        );
        assert_eq!(plan_container_queries(&queries(&[])), Ok(None));
    }

    #[test]
    fn a_plan_reads_one_container_axis_and_at_most_sixteen_breakpoints() {
        let mixed = |preludes: &[&str]| plan_container_queries(&queries(preludes));
        for pair in [
            ["a (width > 1px)", "b (width > 1px)"],
            ["(width > 1px)", "sidebar (width > 1px)"],
            ["(width > 1px)", "(inline-size > 1px)"],
        ] {
            assert_eq!(
                mixed(&pair),
                Err(ContainerPlanUnsupported::MixedContainers),
                "{pair:?}"
            );
        }

        let steps = |count: usize| -> Vec<String> {
            (1..=count)
                .map(|px| format!("(min-width: {px}px)"))
                .collect()
        };
        let sixteen = steps(MAX_CONTAINER_BREAKPOINTS);
        let sixteen: Vec<&str> = sixteen.iter().map(String::as_str).collect();
        let plan = mixed(&sixteen).expect("sixteen fit").expect("a plan");
        assert_eq!(plan.breakpoints.len(), MAX_CONTAINER_BREAKPOINTS);
        assert_eq!(plan.active.len(), MAX_CONTAINER_BREAKPOINTS + 1);
        assert!(plan.breakpoints.windows(2).all(|pair| pair[0] < pair[1]));
        let seventeen = steps(MAX_CONTAINER_BREAKPOINTS + 1);
        let seventeen: Vec<&str> = seventeen.iter().map(String::as_str).collect();
        assert_eq!(
            mixed(&seventeen),
            Err(ContainerPlanUnsupported::TooManyBreakpoints)
        );
    }

    #[test]
    fn container_blocks_parse_into_their_own_bucket() {
        let css = "@container card (inline-size >= 400px) { .a { width: 10px; } .b { height: 2px; } } \
                   .always { height: 8px; }";
        let (sheet, report) = parse_stylesheet_full(css, 0);
        assert_eq!(report.skipped_at_rules, 0);
        assert_eq!(report.rules, 3);
        assert_eq!(sheet.container_rules.len(), 1);
        assert_eq!(sheet.static_rules.len(), 1);
        assert_eq!(first_class(&sheet.static_rules[0]), "always");
        let block = &sheet.container_rules[0];
        let query = block.query.as_ref().expect("supported");
        assert_eq!(query.name.as_deref(), Some("card"));
        assert_eq!(query.axis, ContainerAxis::Inline);
        let orders: Vec<u32> = block
            .sheet
            .static_rules
            .iter()
            .chain(&sheet.static_rules)
            .map(|rule| rule.source_order)
            .collect();
        assert_eq!(orders, [0, 1, 2], "one source order across the blocks");
        assert_eq!(sheet.max_source_order(), Some(2));

        // Never in the unconditional cascade, whatever the media.
        let rules = parse_stylesheet(css, 0);
        assert_eq!(rules.len(), 1);
        assert_eq!(first_class(&rules[0]), "always");
        let flat = sheet.flatten(&MediaEnvironment::default());
        assert_eq!(flat.static_rules.len(), 1);
        assert_eq!(flat.container_rules.len(), 1);

        let (only, _) = parse_stylesheet_full("@container (width > 1px) { .a { width: 1px } }", 0);
        assert!(!only.is_cascade_empty());
        assert!(only.static_rules.is_empty());
        let (bodiless, report) =
            parse_stylesheet_full("@container (width > 1px); .a { width: 1px }", 0);
        assert_eq!(report.skipped_at_rules, 1);
        assert!(bodiless.container_rules.is_empty());
    }

    #[test]
    fn container_blocks_flatten_with_media() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            @media (min-width: 800px) {
                @container (width < 300px) { .a { width: 1px; } }
                .wide { width: 3px; }
            }
            @container card (width >= 300px) {
                @media (prefers-color-scheme: dark) { .a { color: red; } }
                .a { height: 2px; }
            }
            "#,
            0,
        );
        assert_eq!(report.skipped_at_rules, 0);
        assert_eq!(sheet.media_rules.len(), 1);
        assert_eq!(sheet.container_rules.len(), 1);

        let wide_dark = MediaEnvironment {
            width: 900.0,
            height: 500.0,
            color_scheme_dark: true,
        };
        let flat = sheet.flatten(&wide_dark);
        let names: Vec<&str> = flat.static_rules.iter().map(first_class).collect();
        assert_eq!(
            names,
            ["wide"],
            "container rules never flatten into static rules"
        );
        assert_eq!(
            flat.container_rules.len(),
            2,
            "a matching @media keeps its blocks"
        );
        let card = flat
            .container_rules
            .iter()
            .find(|rule| rule.query.as_ref().is_ok_and(|q| q.name.is_some()))
            .expect("card");
        assert!(card.sheet.media_rules.is_empty());
        assert_eq!(card.sheet.static_rules.len(), 2, "an @media inside matches");

        let narrow_light = MediaEnvironment {
            width: 400.0,
            height: 500.0,
            color_scheme_dark: false,
        };
        let flat = sheet.flatten(&narrow_light);
        assert!(flat.static_rules.is_empty());
        assert_eq!(flat.container_rules.len(), 1);
        assert_eq!(flat.container_rules[0].sheet.static_rules.len(), 1);
        assert!(flat.container_rules[0].sheet.media_rules.is_empty());
    }

    #[test]
    fn container_blocks_follow_imports_and_source_order_offsets() {
        let mut files = HashMap::new();
        files.insert(
            "c.css".into(),
            "@container (width > 1px) { .imported { width: 1px; } }".into(),
        );
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            base_href: Some("main.css"),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            "@import \"c.css\"; @import \"c.css\" (min-width: 800px); .local { width: 2px; }",
            0,
            &mut options,
        );
        assert_eq!(report.imported_sheets, 2);
        assert_eq!(sheet.container_rules.len(), 1);
        assert_eq!(sheet.media_rules.len(), 1);
        let imported = sheet.container_rules[0].sheet.static_rules[0].source_order;
        assert!(imported < sheet.static_rules[0].source_order);
        assert_eq!(
            sheet.max_source_order(),
            Some(sheet.static_rules[0].source_order)
        );

        let mut moved = sheet.clone();
        offset_source_order(&mut moved, 10);
        assert_eq!(
            moved.container_rules[0].sheet.static_rules[0].source_order,
            imported + 10
        );
        let mut merged = crate::css_interactive::ParsedStylesheet::default();
        merge_parsed_stylesheet(&mut merged, moved);
        assert_eq!(merged.container_rules.len(), 1);
    }

    #[test]
    fn unsupported_and_nested_blocks_are_counted_and_never_applied() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            @container style(--wide: 1) { .a { width: 1px; } }
            @container (width > 10em) { .a { height: 1px; } }
            @container (width > 100px) {
                @container (height > 100px) { .a { padding: 1px; } }
                @media (min-width: 1px) { @container (height > 5px) { .a { gap: 1px; } } }
                .a { opacity: 0.5; }
            }
            .a { order: 1; }
            "#,
            0,
        );
        assert_eq!(report.skipped_at_rules, 4);
        let reasons: Vec<Result<(), Why>> = sheet
            .container_rules
            .iter()
            .map(|rule| rule.query.as_ref().map(|_| ()).map_err(Clone::clone))
            .collect();
        assert_eq!(
            reasons,
            [
                Err(Why::Function("style".into())),
                Err(Why::Unit("em".into())),
                Ok(()),
                Err(Why::Nested),
                Err(Why::Nested),
            ]
        );
        let outer = &sheet.container_rules[2];
        assert!(outer.sheet.container_rules.is_empty());
        assert!(outer.sheet.media_rules[0].sheet.container_rules.is_empty());

        let flat = sheet.flatten(&MediaEnvironment::default());
        let set = ContainerRuleSet::build(&flat.container_rules);
        let classes = class_names(&["a"]);
        let attrs = BTreeMap::new();
        let ctx = element("div", "", &classes, &attrs);
        let matched = set.matching(&ctx);
        assert_eq!(matched.len(), 5);
        assert!(set.any_unsupported(&matched));
        let plan = set.plan(&matched).expect("one container").expect("a plan");
        assert_eq!(plan.breakpoints, [100f32.next_up()]);
        let opacity_only: Vec<&str> = (0..plan.active.len())
            .flat_map(|bucket| set.active_rules(&plan, bucket))
            .flat_map(|rule| rule.declaration_entries.iter())
            .map(|entry| entry.property.as_str())
            .collect();
        assert_eq!(
            opacity_only,
            ["opacity"],
            "only the supported block applies"
        );

        let index = RuleIndex::build(&flat.static_rules);
        for bucket in 0..plan.active.len() {
            let layout = rebuild_layout_style_indexed_with_extra(
                LayoutStyle::default(),
                &flat.static_rules,
                &index,
                &set.active_rules(&plan, bucket),
                &ctx,
                "",
                "",
                None,
                None,
            );
            assert!(layout.width.is_none() && layout.height.is_none());
            assert!(layout.padding.is_none() && layout.gap.is_none());
            assert_eq!(layout.order, 1);
        }

        // Counted per node, replaced on every pass, given back when it goes.
        let mut tally = UnsupportedCssTally::default();
        tally.observe_container_queries(1, set.any_unsupported(&matched));
        tally.observe_container_queries(1, true);
        assert_eq!(tally.report().container_queries, 1);
        tally.observe_container_queries(2, true);
        assert_eq!(tally.report().container_queries, 2);
        tally.observe_container_queries(1, false);
        assert_eq!(tally.report().container_queries, 1);
        tally.forget(2);
        tally.forget(2);
        assert_eq!(tally.report().container_queries, 0);
        assert!(tally.report().is_empty());
    }

    #[test]
    fn active_container_rules_take_their_place_in_the_cascade() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            .a { width: 1px; height: 1px; }
            @container (width > 400px) {
                .a { width: 2px; height: 2px; padding: 2px; gap: 2px !important; }
            }
            .a { height: 3px; }
            .a.b { padding: 4px; }
            #x { gap: 9px; }
            "#,
            0,
        );
        assert_eq!(report.skipped_at_rules, 0);
        let flat = sheet.flatten(&MediaEnvironment::default());
        let index = RuleIndex::build(&flat.static_rules);
        let set = ContainerRuleSet::build(&flat.container_rules);
        let classes = class_names(&["a", "b"]);
        let attrs = BTreeMap::new();
        let ctx = element("div", "x", &classes, &attrs);
        let matched = set.matching(&ctx);
        assert_eq!(matched, [0]);
        assert!(!set.any_unsupported(&matched));
        let plan = set.plan(&matched).expect("one container").expect("a plan");
        assert_eq!(plan.breakpoints, [400f32.next_up()]);
        assert_eq!(plan.active, [vec![], vec![0]]);

        let cascade = |bucket: usize, inline: &str| {
            rebuild_layout_style_indexed_with_extra(
                LayoutStyle::default(),
                &flat.static_rules,
                &index,
                &set.active_rules(&plan, bucket),
                &ctx,
                "",
                inline,
                None,
                None,
            )
        };
        let narrow = cascade(0, "");
        assert_eq!(
            narrow,
            rebuild_layout_style_indexed(
                LayoutStyle::default(),
                &flat.static_rules,
                &index,
                &ctx,
                "",
                "",
                None,
                None,
            )
        );
        assert_eq!(narrow.width, Some(LengthSpec::Px(1.0)));

        let wide = cascade(1, "");
        assert_eq!(
            wide.width,
            Some(LengthSpec::Px(2.0)),
            "a later container rule beats an earlier rule of the same specificity"
        );
        assert_eq!(
            wide.height,
            Some(LengthSpec::Px(3.0)),
            "a later rule beats the container rule: it is not appended last"
        );
        assert_eq!(
            wide.padding,
            Some(LengthSpec::Px(4.0)),
            "a more specific rule beats the container rule"
        );
        assert_eq!(
            wide.gap,
            Some(LengthSpec::Px(2.0)),
            "container !important beats a normal id rule"
        );

        let inline = cascade(1, "gap: 7px; height: 5px");
        assert_eq!(inline.gap, Some(LengthSpec::Px(2.0)), "and inline normal");
        assert_eq!(inline.height, Some(LengthSpec::Px(5.0)));
        let inline_important = cascade(1, "gap: 8px !important");
        assert_eq!(inline_important.gap, Some(LengthSpec::Px(8.0)));
    }

    #[test]
    fn container_custom_properties_only_join_their_bucket() {
        let (sheet, _) = parse_stylesheet_full(
            r#"
            :root { --base: 1px; }
            .a { --own: 1px; }
            @container (width > 400px) {
                :root { --base: 2px; }
                .a { --own: 2px; --wide: 3px; }
            }
            "#,
            0,
        );
        let flat = sheet.flatten(&MediaEnvironment::default());
        let document = collect_document_custom_properties_from_rules(&flat.static_rules, "light");
        assert_eq!(document.get("--base").map(String::as_str), Some("1px"));

        let index = RuleIndex::build(&flat.static_rules);
        let set = ContainerRuleSet::build(&flat.container_rules);
        let classes = class_names(&["a"]);
        let attrs = BTreeMap::new();
        let parent = [crate::css_cascade::MatchNode {
            tag: "div",
            id: "",
            classes: &[],
            attrs: &attrs,
            is_empty: false,
            checked: false,
        }];
        let mut ctx = element("div", "", &classes, &attrs);
        ctx.ancestors = &parent;
        let unconditional = matched_custom_properties_indexed(&flat.static_rules, &index, &ctx);
        assert_eq!(unconditional.get("--own").map(String::as_str), Some("1px"));
        assert!(!unconditional.contains_key("--wide"));

        let matched = set.matching(&ctx);
        let plan = set.plan(&matched).expect("one container").expect("a plan");
        let wide = matched_custom_properties_indexed_with_extra(
            &flat.static_rules,
            &index,
            &set.active_rules(&plan, 1),
            &ctx,
        );
        assert_eq!(wide.get("--own").map(String::as_str), Some("2px"));
        assert_eq!(wide.get("--wide").map(String::as_str), Some("3px"));
    }

    #[test]
    fn container_properties_map_onto_the_style_model() {
        let applied = |declarations: &str| {
            let mut layout = LayoutStyle::default();
            layout.apply_css_text(declarations, None, None);
            (layout.container_type, layout.container_name)
        };
        assert_eq!(
            applied("container-type: inline-size").0,
            ContainerType::InlineSize
        );
        assert_eq!(applied("container-type: SIZE").0, ContainerType::Size);
        assert_eq!(applied("containerType: size").0, ContainerType::Size);
        assert_eq!(
            applied("container-type: scroll-state size").0,
            ContainerType::Size
        );
        for normal in [
            "container-type: size; container-type: normal",
            "container-type: size; container-type: initial",
            "container-type: size; container-type: scroll-state",
        ] {
            assert_eq!(applied(normal).0, ContainerType::Normal, "{normal}");
        }
        for ignored in ["inline-size size", "bogus", "size size", ""] {
            let css = format!("container-type: size; container-type: {ignored}");
            assert_eq!(applied(&css).0, ContainerType::Size, "{ignored:?}");
        }

        assert_eq!(
            applied("container-name: sidebar Card").1,
            class_names(&["sidebar", "Card"])
        );
        assert!(
            applied("container-name: a; container-name: none")
                .1
                .is_empty()
        );
        assert!(
            applied("container-name: a; container-name: unset")
                .1
                .is_empty()
        );
        for ignored in ["and", "a none", "1abc", "inherit"] {
            let css = format!("container-name: a; container-name: {ignored}");
            assert_eq!(applied(&css).1, class_names(&["a"]), "{ignored:?}");
        }

        assert_eq!(
            applied("container: sidebar / inline-size"),
            (ContainerType::InlineSize, class_names(&["sidebar"]))
        );
        assert_eq!(
            applied("container-type: size; container: card"),
            (ContainerType::Normal, class_names(&["card"])),
            "the shorthand resets the type it leaves out"
        );
        assert_eq!(
            applied("container: none / size"),
            (ContainerType::Size, Vec::new())
        );
        for ignored in ["/ size", "a / b", "a / size / size", "a / initial"] {
            let css = format!("container: x / size; container: {ignored}");
            assert_eq!(
                applied(&css),
                (ContainerType::Size, class_names(&["x"])),
                "{ignored:?}"
            );
        }

        // The cascade writes them like any declaration.
        let rules = parse_stylesheet(".panel { container: panel / size; }", 0);
        let classes = class_names(&["panel"]);
        let attrs = BTreeMap::new();
        let mut layout = LayoutStyle::default();
        crate::css_cascade::apply_stylesheet_to_layout(
            &mut layout,
            &rules,
            &element("div", "", &classes, &attrs),
            None,
            None,
        );
        assert_eq!(layout.container_type, ContainerType::Size);
        assert_eq!(layout.container_name, class_names(&["panel"]));
    }
}
