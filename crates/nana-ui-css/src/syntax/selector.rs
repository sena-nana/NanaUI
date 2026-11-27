//! W3C Selectors Level 4 AST, parser (operating directly on `&[ComponentValue]`),
//! `An+B` microsyntax parser, and exact [`Specificity`] computation.

use std::ops::Add;

use super::MAX_NESTING;
use super::diag::{CssDiag, DiagCode};
use super::parser::{BlockKind, ComponentValue, span_of_slice, split_comma_groups, trim_ws};
use super::span::Span;
use super::tokenizer::{SpannedToken, Token};

/// Exact W3C Selectors Level 4 specificity tuple `(id, class/attr/pseudo-class, type/pseudo-element)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl Specificity {
    pub const ZERO: Self = Self(0, 0, 0);
    pub const ID: Self = Self(1, 0, 0);
    pub const CLASS: Self = Self(0, 1, 0);
    pub const TYPE: Self = Self(0, 0, 1);

    #[inline]
    pub const fn new(id: u32, class: u32, ty: u32) -> Self {
        Self(id, class, ty)
    }

    #[inline]
    pub const fn add_spec(self, other: Self) -> Self {
        Self(
            self.0.saturating_add(other.0),
            self.1.saturating_add(other.1),
            self.2.saturating_add(other.2),
        )
    }
}

impl Add for Specificity {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        self.add_spec(rhs)
    }
}

/// Comma-separated list of complex selectors.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorList(pub Vec<ComplexSelector>);

impl SelectorList {
    /// Maximum specificity across all selectors in the list (used by `:is()` and `:not()`).
    pub fn max_specificity(&self) -> Specificity {
        self.0
            .iter()
            .map(ComplexSelector::specificity)
            .max()
            .unwrap_or(Specificity::ZERO)
    }
}

/// Combinator linking two adjacent [`CompoundSelector`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combinator {
    /// Whitespace (`A B`).
    Descendant,
    /// Direct child (`A > B`).
    Child,
    /// Next sibling (`A + B`).
    NextSibling,
    /// Subsequent sibling (`A ~ B`).
    SubsequentSibling,
}

impl Combinator {
    #[inline]
    pub const fn is_sibling(self) -> bool {
        matches!(self, Self::NextSibling | Self::SubsequentSibling)
    }
}

/// Sequence of [`CompoundSelector`]s separated by [`Combinator`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct ComplexSelector {
    pub head: CompoundSelector,
    pub rest: Vec<(Combinator, CompoundSelector)>,
    pub span: Span,
}

impl ComplexSelector {
    /// Exact W3C specificity of this complex selector.
    pub fn specificity(&self) -> Specificity {
        let mut total = self.head.specificity();
        for (_, comp) in &self.rest {
            total = total + comp.specificity();
        }
        total
    }

    /// Returns `true` if any combinator in this selector is `+` or `~`.
    pub fn has_sibling_combinator(&self) -> bool {
        self.rest.iter().any(|(c, _)| c.is_sibling())
    }

    /// The rightmost (subject) compound selector.
    pub fn subject(&self) -> &CompoundSelector {
        self.rest.last().map_or(&self.head, |(_, c)| c)
    }
}

/// Type or universal selector (`div` vs `*`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeSelector {
    Universal,
    Tag(String),
}

/// Attribute comparison operator inside `[attr op value]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttrOperator {
    /// `=`
    Equals,
    /// `~=`
    Includes,
    /// `|=`
    DashMatch,
    /// `^=`
    Prefix,
    /// `$=`
    Suffix,
    /// `*=`
    Substring,
}

/// Case-sensitivity flag inside `[attr="val" i|s]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttrCase {
    Insensitive,
    Sensitive,
}

/// Parsed attribute selector `[name]` or `[name op value (i|s)?]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSelector {
    pub name: String,
    pub op: Option<AttrOperator>,
    pub value: Option<String>,
    pub case_sensitivity: Option<AttrCase>,
    pub span: Span,
}

/// `An+B` formula for `:nth-child()` / `:nth-last-child()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnPlusB {
    pub a: i32,
    pub b: i32,
}

impl AnPlusB {
    pub const ODD: Self = Self { a: 2, b: 1 };
    pub const EVEN: Self = Self { a: 2, b: 0 };

    /// Returns `true` when 1-based child index `index` satisfies `a*n + b` for
    /// some integer `n >= 0`.
    pub fn matches(self, index_1based: usize) -> bool {
        let Ok(idx) = i64::try_from(index_1based) else {
            return false;
        };
        if idx < 1 {
            return false;
        }
        let a = i64::from(self.a);
        let b = i64::from(self.b);
        if a == 0 {
            return idx == b;
        }
        let diff = idx - b;
        if diff % a != 0 {
            return false;
        }
        (diff / a) >= 0
    }
}

/// Pseudo-class on a [`CompoundSelector`].
///
/// Each `:is(...)`, `:where(...)`, and `:not(...)` is stored as its own item in
/// [`CompoundSelector::pseudos`] so multiple functional pseudo-classes on one
/// compound remain AND-composed (never flattened into a single OR list).
#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Selected,
    Disabled,
    Enabled,
    Checked,
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    NthChild(AnPlusB),
    NthLastChild(AnPlusB),
    Not(SelectorList),
    Is(SelectorList),
    Where(SelectorList),
    Lang(String),
    Unknown {
        name: String,
        args: Option<Vec<ComponentValue>>,
        span: Span,
    },
}

impl PseudoClass {
    /// Exact W3C Selectors Level 4 specificity of this pseudo-class.
    pub fn specificity(&self) -> Specificity {
        match self {
            Self::Where(_) => Specificity::ZERO,
            Self::Is(list) | Self::Not(list) => list.max_specificity(),
            _ => Specificity::CLASS,
        }
    }
}

/// Pseudo-element attached to a [`CompoundSelector`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Placeholder,
    Selection,
    WebkitScrollbar,
    WebkitScrollbarThumb,
    WebkitScrollbarTrack,
    Before,
    After,
    Unknown(String),
}

/// A single compound selector (e.g. `button#save.primary[data-x="a*=b"]:hover::before`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompoundSelector {
    pub nesting: bool,
    pub type_sel: Option<TypeSelector>,
    pub id: Option<String>,
    /// Additional `#id` selectors if more than one appeared on the same compound.
    pub extra_ids: Vec<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSelector>,
    pub pseudos: Vec<PseudoClass>,
    pub pseudo_element: Option<PseudoElement>,
    pub span: Span,
}

impl CompoundSelector {
    /// Returns `true` if the compound selector contains no simple selectors.
    pub fn is_empty(&self) -> bool {
        !self.nesting
            && self.type_sel.is_none()
            && self.id.is_none()
            && self.extra_ids.is_empty()
            && self.classes.is_empty()
            && self.attrs.is_empty()
            && self.pseudos.is_empty()
            && self.pseudo_element.is_none()
    }

    /// Exact W3C specificity of this compound selector.
    pub fn specificity(&self) -> Specificity {
        let mut s = Specificity::ZERO;
        if let Some(TypeSelector::Tag(_)) = &self.type_sel {
            s = s + Specificity::TYPE;
        }
        if self.id.is_some() {
            s = s + Specificity::ID;
        }
        s.0 = s.0.saturating_add(self.extra_ids.len() as u32);
        s.1 =
            s.1.saturating_add(self.classes.len() as u32)
                .saturating_add(self.attrs.len() as u32);
        for p in &self.pseudos {
            s = s + p.specificity();
        }
        if self.pseudo_element.is_some() {
            s = s + Specificity::TYPE;
        }
        s
    }
}

/// Parse a comma-separated [`SelectorList`] from `&[ComponentValue]`.
pub fn parse_selector_list(
    values: &[ComponentValue],
    diags: &mut Vec<CssDiag>,
) -> Option<SelectorList> {
    parse_selector_list_depth(values, 0, diags)
}

fn parse_selector_list_depth(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<SelectorList> {
    if depth >= MAX_NESTING {
        let span = span_of_slice(values).unwrap_or(Span::EMPTY);
        diags.push(CssDiag::error(
            span,
            DiagCode::NestingLimitExceeded,
            format!("selector nesting limit ({MAX_NESTING}) exceeded"),
        ));
        return None;
    }

    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }

    let groups = split_comma_groups(trimmed);
    let mut selectors = Vec::with_capacity(groups.len());
    for group in groups {
        if group.is_empty() {
            let span = span_of_slice(trimmed).unwrap_or(Span::EMPTY);
            diags.push(CssDiag::error(
                span,
                DiagCode::InvalidSelector,
                "empty selector in comma-separated selector list",
            ));
            return None;
        }
        let complex = parse_complex_selector(group, depth, diags)?;
        selectors.push(complex);
    }

    Some(SelectorList(selectors))
}

fn parse_complex_selector(
    values: &[ComponentValue],
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<ComplexSelector> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    let full_span = span_of_slice(trimmed).unwrap_or(Span::EMPTY);
    let mut pos = 0;

    let head = parse_compound(trimmed, &mut pos, depth, diags)?;
    let mut rest = Vec::new();

    while pos < trimmed.len() {
        let mut saw_ws = false;
        while pos < trimmed.len() && trimmed[pos].is_whitespace() {
            saw_ws = true;
            pos += 1;
        }
        if pos >= trimmed.len() {
            break;
        }

        let explicit_comb = match trimmed[pos].as_token() {
            Some(Token::Delim('>')) => Some(Combinator::Child),
            Some(Token::Delim('+')) => Some(Combinator::NextSibling),
            Some(Token::Delim('~')) => Some(Combinator::SubsequentSibling),
            _ => None,
        };

        let combinator = if let Some(comb) = explicit_comb {
            pos += 1;
            while pos < trimmed.len() && trimmed[pos].is_whitespace() {
                pos += 1;
            }
            comb
        } else if saw_ws {
            Combinator::Descendant
        } else {
            diags.push(CssDiag::error(
                trimmed[pos].span(),
                DiagCode::InvalidSelector,
                "unexpected token in selector",
            ));
            return None;
        };

        if pos >= trimmed.len() {
            diags.push(CssDiag::error(
                full_span,
                DiagCode::InvalidSelector,
                "trailing combinator in selector",
            ));
            return None;
        }

        let next_comp = parse_compound(trimmed, &mut pos, depth, diags)?;
        rest.push((combinator, next_comp));
    }

    Some(ComplexSelector {
        head,
        rest,
        span: full_span,
    })
}

fn parse_compound(
    values: &[ComponentValue],
    pos: &mut usize,
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<CompoundSelector> {
    let start_pos = *pos;
    if start_pos >= values.len() {
        return None;
    }
    let start_byte = values[start_pos].span().start;
    let mut comp = CompoundSelector::default();

    // Optional leading type / universal / `&` selector.
    if *pos < values.len() {
        match &values[*pos] {
            ComponentValue::Token(SpannedToken {
                token: Token::Ident(tag),
                ..
            }) => {
                comp.type_sel = Some(TypeSelector::Tag(tag.clone()));
                *pos += 1;
            }
            ComponentValue::Token(SpannedToken {
                token: Token::Delim('*'),
                ..
            }) => {
                comp.type_sel = Some(TypeSelector::Universal);
                *pos += 1;
            }
            ComponentValue::Token(SpannedToken {
                token: Token::Delim('&'),
                ..
            }) => {
                comp.nesting = true;
                *pos += 1;
            }
            _ => {}
        }
    }

    // Subclass selectors, `&`, attribute blocks, and pseudo-classes / pseudo-elements.
    while *pos < values.len() {
        let cv = &values[*pos];
        if cv.is_whitespace() {
            break;
        }
        match cv {
            ComponentValue::Token(SpannedToken {
                token: Token::Delim('>' | '+' | '~'),
                ..
            }) => break,

            ComponentValue::Token(SpannedToken {
                token: Token::Delim('&'),
                ..
            }) => {
                comp.nesting = true;
                *pos += 1;
            }

            ComponentValue::Token(SpannedToken {
                token: Token::Hash { value, id_flag },
                span,
            }) => {
                if !id_flag {
                    diags.push(CssDiag::error(
                        *span,
                        DiagCode::InvalidSelector,
                        format!("invalid ID selector `#{value}`"),
                    ));
                    return None;
                }
                if comp.id.is_none() {
                    comp.id = Some(value.clone());
                } else {
                    comp.extra_ids.push(value.clone());
                }
                *pos += 1;
            }

            ComponentValue::Token(SpannedToken {
                token: Token::Delim('.'),
                span,
            }) => {
                *pos += 1;
                let Some(ComponentValue::Token(SpannedToken {
                    token: Token::Ident(cls),
                    ..
                })) = values.get(*pos)
                else {
                    diags.push(CssDiag::error(
                        *span,
                        DiagCode::InvalidSelector,
                        "expected class identifier after `.`",
                    ));
                    return None;
                };
                comp.classes.push(cls.clone());
                *pos += 1;
            }

            ComponentValue::SimpleBlock {
                kind: BlockKind::Square,
                span,
                values: inner,
            } => {
                let attr = parse_attr_selector(inner, *span, diags)?;
                comp.attrs.push(attr);
                *pos += 1;
            }

            ComponentValue::Token(SpannedToken {
                token: Token::Colon,
                span: colon_span,
            }) => {
                *pos += 1;
                // Double colon -> pseudo-element `::...`
                if matches!(
                    values.get(*pos).and_then(ComponentValue::as_token),
                    Some(Token::Colon)
                ) {
                    *pos += 1;
                    let Some(ComponentValue::Token(SpannedToken {
                        token: Token::Ident(name),
                        ..
                    })) = values.get(*pos)
                    else {
                        diags.push(CssDiag::error(
                            *colon_span,
                            DiagCode::InvalidSelector,
                            "expected pseudo-element name after `::`",
                        ));
                        return None;
                    };
                    comp.pseudo_element = Some(classify_pseudo_element(name));
                    *pos += 1;
                } else {
                    match values.get(*pos) {
                        Some(ComponentValue::Token(SpannedToken {
                            token: Token::Ident(name),
                            span: name_span,
                        })) => {
                            let lower = name.to_ascii_lowercase();
                            // Legacy CSS2 single-colon `:before` / `:after`
                            if lower == "before" {
                                comp.pseudo_element = Some(PseudoElement::Before);
                            } else if lower == "after" {
                                comp.pseudo_element = Some(PseudoElement::After);
                            } else {
                                let pc = match lower.as_str() {
                                    "hover" => PseudoClass::Hover,
                                    "active" => PseudoClass::Active,
                                    "focus" => PseudoClass::Focus,
                                    "focus-visible" => PseudoClass::FocusVisible,
                                    "focus-within" => PseudoClass::FocusWithin,
                                    "selected" => PseudoClass::Selected,
                                    "disabled" => PseudoClass::Disabled,
                                    "enabled" => PseudoClass::Enabled,
                                    "checked" => PseudoClass::Checked,
                                    "root" => PseudoClass::Root,
                                    "empty" => PseudoClass::Empty,
                                    "first-child" => PseudoClass::FirstChild,
                                    "last-child" => PseudoClass::LastChild,
                                    "only-child" => PseudoClass::OnlyChild,
                                    "first-of-type" => PseudoClass::FirstOfType,
                                    "last-of-type" => PseudoClass::LastOfType,
                                    _ => PseudoClass::Unknown {
                                        name: name.clone(),
                                        args: None,
                                        span: colon_span.merge(*name_span),
                                    },
                                };
                                comp.pseudos.push(pc);
                            }
                            *pos += 1;
                        }
                        Some(ComponentValue::Function {
                            name,
                            span: fn_span,
                            args,
                        }) => {
                            let full = colon_span.merge(*fn_span);
                            let pc = parse_functional_pseudo(name, args, full, depth, diags)?;
                            comp.pseudos.push(pc);
                            *pos += 1;
                        }
                        _ => {
                            diags.push(CssDiag::error(
                                *colon_span,
                                DiagCode::InvalidSelector,
                                "expected pseudo-class name after `:`",
                            ));
                            return None;
                        }
                    }
                }
            }

            other => {
                diags.push(CssDiag::error(
                    other.span(),
                    DiagCode::InvalidSelector,
                    "unexpected token in compound selector",
                ));
                return None;
            }
        }
    }

    if comp.is_empty() {
        diags.push(CssDiag::error(
            values[start_pos].span(),
            DiagCode::InvalidSelector,
            "empty compound selector",
        ));
        return None;
    }

    let end_byte = values[*pos - 1].span().end;
    comp.span = Span::new(start_byte, end_byte);
    Some(comp)
}

fn classify_pseudo_element(name: &str) -> PseudoElement {
    match name.to_ascii_lowercase().as_str() {
        "placeholder" => PseudoElement::Placeholder,
        "selection" => PseudoElement::Selection,
        "-webkit-scrollbar" => PseudoElement::WebkitScrollbar,
        "-webkit-scrollbar-thumb" => PseudoElement::WebkitScrollbarThumb,
        "-webkit-scrollbar-track" => PseudoElement::WebkitScrollbarTrack,
        "before" => PseudoElement::Before,
        "after" => PseudoElement::After,
        _ => PseudoElement::Unknown(name.to_string()),
    }
}

fn parse_functional_pseudo(
    name: &str,
    args: &[ComponentValue],
    span: Span,
    depth: usize,
    diags: &mut Vec<CssDiag>,
) -> Option<PseudoClass> {
    match name.to_ascii_lowercase().as_str() {
        "is" => {
            let list = parse_selector_list_depth(args, depth + 1, diags)?;
            Some(PseudoClass::Is(list))
        }
        "where" => {
            let list = parse_selector_list_depth(args, depth + 1, diags)?;
            Some(PseudoClass::Where(list))
        }
        "not" => {
            let list = parse_selector_list_depth(args, depth + 1, diags)?;
            Some(PseudoClass::Not(list))
        }
        "nth-child" => {
            let Some(formula) = parse_an_plus_b(args) else {
                diags.push(CssDiag::error(
                    span,
                    DiagCode::InvalidSelector,
                    "invalid An+B expression in `:nth-child(...)`",
                ));
                return None;
            };
            Some(PseudoClass::NthChild(formula))
        }
        "nth-last-child" => {
            let Some(formula) = parse_an_plus_b(args) else {
                diags.push(CssDiag::error(
                    span,
                    DiagCode::InvalidSelector,
                    "invalid An+B expression in `:nth-last-child(...)`",
                ));
                return None;
            };
            Some(PseudoClass::NthLastChild(formula))
        }
        "lang" => {
            let trimmed = trim_ws(args);
            let value = match trimmed {
                [
                    ComponentValue::Token(SpannedToken {
                        token: Token::Ident(s) | Token::String(s),
                        ..
                    }),
                ] => s.clone(),
                _ => {
                    diags.push(CssDiag::error(
                        span,
                        DiagCode::InvalidSelector,
                        "expected identifier or string in `:lang(...)`",
                    ));
                    return None;
                }
            };
            Some(PseudoClass::Lang(value))
        }
        _ => Some(PseudoClass::Unknown {
            name: name.to_string(),
            args: Some(args.to_vec()),
            span,
        }),
    }
}

fn parse_attr_selector(
    inner: &[ComponentValue],
    span: Span,
    diags: &mut Vec<CssDiag>,
) -> Option<AttrSelector> {
    let trimmed = trim_ws(inner);
    if trimmed.is_empty() {
        diags.push(CssDiag::error(
            span,
            DiagCode::InvalidSelector,
            "empty attribute selector `[]`",
        ));
        return None;
    }

    let mut pos = 0;
    let Some(ComponentValue::Token(SpannedToken {
        token: Token::Ident(name),
        ..
    })) = trimmed.get(pos)
    else {
        diags.push(CssDiag::error(
            span,
            DiagCode::InvalidSelector,
            "attribute selector must start with an attribute name",
        ));
        return None;
    };
    let name = name.clone();
    pos += 1;

    while pos < trimmed.len() && trimmed[pos].is_whitespace() {
        pos += 1;
    }
    if pos >= trimmed.len() {
        return Some(AttrSelector {
            name,
            op: None,
            value: None,
            case_sensitivity: None,
            span,
        });
    }

    let op = match trimmed[pos].as_token() {
        Some(Token::Delim('=')) => AttrOperator::Equals,
        Some(Token::IncludeMatch) => AttrOperator::Includes,
        Some(Token::DashMatch) => AttrOperator::DashMatch,
        Some(Token::PrefixMatch) => AttrOperator::Prefix,
        Some(Token::SuffixMatch) => AttrOperator::Suffix,
        Some(Token::SubstringMatch) => AttrOperator::Substring,
        _ => {
            diags.push(CssDiag::error(
                trimmed[pos].span(),
                DiagCode::InvalidSelector,
                "invalid operator in attribute selector",
            ));
            return None;
        }
    };
    pos += 1;

    while pos < trimmed.len() && trimmed[pos].is_whitespace() {
        pos += 1;
    }

    let value = match trimmed.get(pos).and_then(ComponentValue::as_token) {
        Some(Token::String(s) | Token::Ident(s)) => s.clone(),
        _ => {
            diags.push(CssDiag::error(
                span,
                DiagCode::InvalidSelector,
                "expected string or identifier value in attribute selector",
            ));
            return None;
        }
    };
    pos += 1;

    while pos < trimmed.len() && trimmed[pos].is_whitespace() {
        pos += 1;
    }

    let mut case_sensitivity = None;
    if pos < trimmed.len() {
        match trimmed[pos].as_ident() {
            Some(flag) if flag.eq_ignore_ascii_case("i") => {
                case_sensitivity = Some(AttrCase::Insensitive);
                pos += 1;
            }
            Some(flag) if flag.eq_ignore_ascii_case("s") => {
                case_sensitivity = Some(AttrCase::Sensitive);
                pos += 1;
            }
            _ => {
                diags.push(CssDiag::error(
                    trimmed[pos].span(),
                    DiagCode::InvalidSelector,
                    "unexpected modifier in attribute selector (expected `i` or `s`)",
                ));
                return None;
            }
        }
    }

    while pos < trimmed.len() && trimmed[pos].is_whitespace() {
        pos += 1;
    }
    if pos < trimmed.len() {
        diags.push(CssDiag::error(
            trimmed[pos].span(),
            DiagCode::InvalidSelector,
            "trailing tokens in attribute selector",
        ));
        return None;
    }

    Some(AttrSelector {
        name,
        op: Some(op),
        value: Some(value),
        case_sensitivity,
        span,
    })
}

/// Parse a W3C CSS Syntax Level 3 §6 `An+B` expression from `&[ComponentValue]`.
pub fn parse_an_plus_b(values: &[ComponentValue]) -> Option<AnPlusB> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }

    // Single keyword `odd` / `even` or single integer `B`.
    if trimmed.len() == 1 {
        match trimmed[0].as_token()? {
            Token::Ident(s) if s.eq_ignore_ascii_case("odd") => return Some(AnPlusB::ODD),
            Token::Ident(s) if s.eq_ignore_ascii_case("even") => return Some(AnPlusB::EVEN),
            Token::Number { int: Some(b), .. } => {
                return Some(AnPlusB {
                    a: 0,
                    b: i32::try_from(*b).ok()?,
                });
            }
            _ => {}
        }
    }

    // Collect non-whitespace tokens (preserving sign adjacency info from raw token forms).
    let mut pos = 0;

    // Parse the `An` part (may be `+n`, `-n`, `n`, `2n`, `-2n`, `2n-1`, `n-3`, `-n-3`, etc.)
    let (a, embedded_b) = match trimmed.get(pos)?.as_token()? {
        Token::Dimension { value, unit } => {
            // Must be an integer coefficient (no fractional part).
            if value.fract() != 0.0 {
                return None;
            }
            let a = *value as i32;
            let (has_b, b_val) = parse_n_unit(unit)?;
            pos += 1;
            (a, if has_b { Some(b_val) } else { None })
        }
        Token::Ident(s) => {
            let lower = s.to_ascii_lowercase();
            let (a, rest) = if let Some(r) = lower.strip_prefix("-n") {
                (-1, r)
            } else {
                (1, lower.strip_prefix('n')?)
            };
            let embedded = parse_n_tail(rest)?;
            pos += 1;
            (a, embedded)
        }
        Token::Delim('+') => {
            pos += 1;
            // Must immediately be followed by ident starting with `n` (no whitespace per spec).
            let Some(Token::Ident(s)) = trimmed.get(pos).and_then(ComponentValue::as_token) else {
                return None;
            };
            let lower = s.to_ascii_lowercase();
            let rest = lower.strip_prefix('n')?;
            let embedded = parse_n_tail(rest)?;
            pos += 1;
            (1, embedded)
        }
        _ => return None,
    };

    // Skip optional whitespace after `An`.
    while pos < trimmed.len() && trimmed[pos].is_whitespace() {
        pos += 1;
    }

    let b = if let Some(eb) = embedded_b {
        if pos < trimmed.len() {
            return None;
        }
        eb
    } else if pos >= trimmed.len() {
        0
    } else {
        // Expect signed integer (`+3`, `-1`) OR (`+`/`-` <ws>* unsigned integer).
        match trimmed[pos].as_token()? {
            Token::Number {
                int: Some(n), raw, ..
            } => {
                if !raw.starts_with('+') && !raw.starts_with('-') {
                    return None;
                }
                pos += 1;
                i32::try_from(*n).ok()?
            }
            Token::Delim(sign @ ('+' | '-')) => {
                let sign_mul = if *sign == '-' { -1 } else { 1 };
                pos += 1;
                while pos < trimmed.len() && trimmed[pos].is_whitespace() {
                    pos += 1;
                }
                let Some(Token::Number {
                    int: Some(n), raw, ..
                }) = trimmed.get(pos).and_then(ComponentValue::as_token)
                else {
                    return None;
                };
                if raw.starts_with('+') || raw.starts_with('-') {
                    return None;
                }
                pos += 1;
                i32::try_from(*n).ok()?.checked_mul(sign_mul)?
            }
            _ => return None,
        }
    };

    while pos < trimmed.len() && trimmed[pos].is_whitespace() {
        pos += 1;
    }
    if pos < trimmed.len() {
        return None;
    }

    Some(AnPlusB { a, b })
}

fn parse_n_unit(unit: &str) -> Option<(bool, i32)> {
    let lower = unit.to_ascii_lowercase();
    let rest = lower.strip_prefix('n')?;
    match parse_n_tail(rest)? {
        Some(b) => Some((true, b)),
        None => Some((false, 0)),
    }
}

fn parse_n_tail(rest: &str) -> Option<Option<i32>> {
    if rest.is_empty() {
        return Some(None);
    }
    if rest == "-" {
        // Handled only if followed by unsigned integer? In CSS An+B `n- 3` is actually
        // `Ident("n-")` + whitespace + unsigned integer; disallow ambiguous forms unless digits follow.
        return None;
    }
    let digits = rest.strip_prefix('-')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mag: i32 = digits.parse().ok()?;
    Some(Some(-mag))
}
