//! W3C CSS Syntax Level 3 parser producing [`ComponentValue`] trees,
//! [`Stylesheet`]s, [`Rule`]s, [`Declaration`]s, and nested style-block items.
//!
//! All nesting (blocks `{}`, `[]`, `()`, and functions `fn(...)`) is strictly
//! bounded by [`MAX_NESTING`] using iterative recovery when exceeded so hostile
//! inputs can never overflow the Rust call stack.

use super::MAX_NESTING;
use super::diag::{CssDiag, DiagCode};
use super::span::Span;
use super::tokenizer::{SpannedToken, Token, tokenize};

/// Delimiter pair for [`ComponentValue::SimpleBlock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Curly,
    Square,
    Paren,
}

impl BlockKind {
    #[inline]
    pub const fn open_char(self) -> char {
        match self {
            Self::Curly => '{',
            Self::Square => '[',
            Self::Paren => '(',
        }
    }

    #[inline]
    pub const fn close_char(self) -> char {
        match self {
            Self::Curly => '}',
            Self::Square => ']',
            Self::Paren => ')',
        }
    }
}

/// W3C CSS Syntax Level 3 component value: either a preserved token, a balanced
/// simple block (`{...}`, `[...]`, `(...)`), or a function (`name(...)`).
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentValue {
    Token(SpannedToken),
    SimpleBlock {
        kind: BlockKind,
        span: Span,
        values: Vec<ComponentValue>,
    },
    Function {
        name: String,
        span: Span,
        args: Vec<ComponentValue>,
    },
}

impl ComponentValue {
    /// Exact byte span of this component value in the source.
    #[inline]
    pub const fn span(&self) -> Span {
        match self {
            Self::Token(t) => t.span,
            Self::SimpleBlock { span, .. } | Self::Function { span, .. } => *span,
        }
    }

    /// Returns `true` if this value is a whitespace token.
    #[inline]
    pub const fn is_whitespace(&self) -> bool {
        matches!(
            self,
            Self::Token(SpannedToken {
                token: Token::Whitespace,
                ..
            })
        )
    }

    /// Borrow the inner [`Token`] when this is a [`ComponentValue::Token`].
    #[inline]
    pub const fn as_token(&self) -> Option<&Token> {
        match self {
            Self::Token(st) => Some(&st.token),
            _ => None,
        }
    }

    /// Borrow an identifier string when this is `Token::Ident`.
    #[inline]
    pub fn as_ident(&self) -> Option<&str> {
        match self {
            Self::Token(SpannedToken {
                token: Token::Ident(s),
                ..
            }) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Extract a URL string from either an unquoted `Token::Url(s)` or a quoted
    /// `url("...")` function component value.
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Self::Token(SpannedToken {
                token: Token::Url(s),
                ..
            }) => Some(s.as_str()),
            Self::Function { name, args, .. } if name.eq_ignore_ascii_case("url") => {
                let trimmed = trim_ws(args);
                if let [
                    ComponentValue::Token(SpannedToken {
                        token: Token::String(s),
                        ..
                    }),
                ] = trimmed
                {
                    Some(s.as_str())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Top-level parsed stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub diags: Vec<CssDiag>,
}

/// Either a qualified style rule (`selector { ... }`) or an at-rule (`@name ...`).
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    Qualified(QualifiedRule),
    At(AtRule),
}

impl Rule {
    #[inline]
    pub const fn span(&self) -> Span {
        match self {
            Self::Qualified(r) => r.span,
            Self::At(r) => r.span,
        }
    }
}

/// A qualified rule (`prelude { block }`).
#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedRule {
    pub prelude: Vec<ComponentValue>,
    pub block: Vec<ComponentValue>,
    pub span: Span,
    pub prelude_span: Span,
    pub block_span: Span,
}

/// An at-rule (`@name prelude;` or `@name prelude { block }`).
#[derive(Debug, Clone, PartialEq)]
pub struct AtRule {
    pub name: String,
    pub prelude: Vec<ComponentValue>,
    pub block: Option<Vec<ComponentValue>>,
    pub span: Span,
    pub prelude_span: Span,
    pub block_span: Option<Span>,
}

/// One CSS declaration (`name: value [!important]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: Vec<ComponentValue>,
    pub important: bool,
    pub span: Span,
    pub name_span: Span,
    pub value_span: Span,
}

/// Item parsed inside a CSS Nesting style rule block: either a declaration or a
/// nested qualified/at rule.
#[derive(Debug, Clone, PartialEq)]
pub enum StyleBlockItem {
    Declaration(Declaration),
    NestedRule(Rule),
}

// ============================================================================
// Public parse entrypoints
// ============================================================================

/// Parse a full CSS stylesheet into rules + diagnostics.
pub fn parse_stylesheet(src: &str) -> Stylesheet {
    let mut diags = Vec::new();
    let tokens = tokenize(src, &mut diags);
    let mut parser = TokenParser::new(&tokens);
    let rules = parser.consume_rule_list(true, &mut diags);
    Stylesheet { rules, diags }
}

/// Parse a standalone list of component values (for values, selectors, preludes).
pub fn parse_component_values(src: &str, diags: &mut Vec<CssDiag>) -> Vec<ComponentValue> {
    let tokens = tokenize(src, diags);
    let mut parser = TokenParser::new(&tokens);
    let mut out = Vec::new();
    while !parser.at_eof() {
        if let Some(cv) = parser.consume_component_value(0, diags) {
            out.push(cv);
        }
    }
    out
}

/// Parse an inline declaration list (`prop: val; prop2: val2 !important`).
pub fn parse_declaration_list(src: &str, diags: &mut Vec<CssDiag>) -> Vec<Declaration> {
    let values = parse_component_values(src, diags);
    parse_declarations_from_values(&values, diags)
}

/// Parse a list of rules from already-structured block [`ComponentValue`]s
/// (e.g. inside `@media { ... }`, `@layer { ... }`, `@container { ... }`,
/// `@supports { ... }`).
pub fn parse_rule_list(values: &[ComponentValue], diags: &mut Vec<CssDiag>) -> Vec<Rule> {
    let mut p = CvParser::new(values);
    p.consume_rule_list(diags)
}

/// Parse declarations from already-structured block [`ComponentValue`]s with
/// W3C error recovery (an invalid declaration skips cleanly to the next `;`).
pub fn parse_declarations_from_values(
    values: &[ComponentValue],
    diags: &mut Vec<CssDiag>,
) -> Vec<Declaration> {
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < values.len() {
        // Skip whitespace and empty semicolons.
        while idx < values.len()
            && (values[idx].is_whitespace()
                || matches!(values[idx].as_token(), Some(Token::Semicolon)))
        {
            idx += 1;
        }
        if idx >= values.len() {
            break;
        }

        // Collect one segment up to top-level `;` or end.
        let seg_start = idx;
        while idx < values.len() && !matches!(values[idx].as_token(), Some(Token::Semicolon)) {
            idx += 1;
        }
        let seg_end = idx;
        if idx < values.len() {
            idx += 1; // consume ';'
        }

        if let Some(decl) = parse_single_declaration(&values[seg_start..seg_end], diags) {
            out.push(decl);
        }
    }
    out
}

/// Parse a style rule block supporting CSS Nesting Level 1: mixes ordinary
/// declarations with nested at-rules (`@media`, `@container`, ...) and nested
/// qualified rules (`&:hover { ... }`, `.child { ... }`).
pub fn parse_style_block(
    values: &[ComponentValue],
    diags: &mut Vec<CssDiag>,
) -> Vec<StyleBlockItem> {
    let mut items = Vec::new();
    let mut idx = 0;
    while idx < values.len() {
        while idx < values.len()
            && (values[idx].is_whitespace()
                || matches!(values[idx].as_token(), Some(Token::Semicolon)))
        {
            idx += 1;
        }
        if idx >= values.len() {
            break;
        }

        // Nested at-rule starts with Token::AtKeyword.
        if matches!(values[idx].as_token(), Some(Token::AtKeyword(_))) {
            let mut sub = CvParser { values, pos: idx };
            if let Some(at) = sub.consume_at_rule(diags) {
                items.push(StyleBlockItem::NestedRule(Rule::At(at)));
            }
            idx = sub.pos;
            continue;
        }

        // Look ahead to the next top-level `;` or curly `{ ... }` block to
        // distinguish a declaration from a nested qualified rule.
        let seg_start = idx;
        let mut hit_curly = false;
        while idx < values.len() {
            match &values[idx] {
                ComponentValue::Token(SpannedToken {
                    token: Token::Semicolon,
                    ..
                }) => {
                    break;
                }
                ComponentValue::SimpleBlock {
                    kind: BlockKind::Curly,
                    ..
                } => {
                    hit_curly = true;
                    break;
                }
                _ => idx += 1,
            }
        }

        if hit_curly {
            // If this is a custom property (`--foo: { ... }`), keep as declaration.
            let is_custom_prop_decl = is_custom_property_start(&values[seg_start..idx]);
            if is_custom_prop_decl {
                // Consume through next `;` or end.
                while idx < values.len()
                    && !matches!(values[idx].as_token(), Some(Token::Semicolon))
                {
                    idx += 1;
                }
                let seg_end = idx;
                if idx < values.len() {
                    idx += 1;
                }
                if let Some(decl) = parse_single_declaration(&values[seg_start..seg_end], diags) {
                    items.push(StyleBlockItem::Declaration(decl));
                }
            } else {
                let block_cv = &values[idx];
                let ComponentValue::SimpleBlock {
                    span: block_span,
                    values: block_vals,
                    ..
                } = block_cv
                else {
                    unreachable!();
                };
                let prelude = trim_ws(&values[seg_start..idx]).to_vec();
                let prelude_span = span_of_slice(&prelude)
                    .unwrap_or(Span::new(values[seg_start].span().start, block_span.start));
                let full_span = Span::new(values[seg_start].span().start, block_span.end);
                idx += 1; // consume curly block
                items.push(StyleBlockItem::NestedRule(Rule::Qualified(QualifiedRule {
                    prelude,
                    block: block_vals.clone(),
                    span: full_span,
                    prelude_span,
                    block_span: *block_span,
                })));
            }
        } else {
            let seg_end = idx;
            if idx < values.len() {
                idx += 1; // consume ';'
            }
            if let Some(decl) = parse_single_declaration(&values[seg_start..seg_end], diags) {
                items.push(StyleBlockItem::Declaration(decl));
            }
        }
    }
    items
}

fn is_custom_property_start(prelude: &[ComponentValue]) -> bool {
    let trimmed = trim_ws(prelude);
    let Some(ComponentValue::Token(SpannedToken {
        token: Token::Ident(name),
        ..
    })) = trimmed.first()
    else {
        return false;
    };
    if !name.starts_with("--") {
        return false;
    }
    let after_name = trim_ws(&trimmed[1..]);
    matches!(
        after_name.first().and_then(ComponentValue::as_token),
        Some(Token::Colon)
    )
}

fn parse_single_declaration(
    segment: &[ComponentValue],
    diags: &mut Vec<CssDiag>,
) -> Option<Declaration> {
    let trimmed = trim_ws(segment);
    if trimmed.is_empty() {
        return None;
    }
    let full_span = span_of_slice(trimmed)?;

    let ComponentValue::Token(SpannedToken {
        token: Token::Ident(name),
        span: name_span,
    }) = &trimmed[0]
    else {
        diags.push(CssDiag::error(
            full_span,
            DiagCode::InvalidDeclaration,
            "declaration must start with a property identifier",
        ));
        return None;
    };

    let mut pos = 1;
    while pos < trimmed.len() && trimmed[pos].is_whitespace() {
        pos += 1;
    }

    if !matches!(
        trimmed.get(pos).and_then(ComponentValue::as_token),
        Some(Token::Colon)
    ) {
        diags.push(CssDiag::error(
            full_span,
            DiagCode::InvalidDeclaration,
            format!("expected `:` after property `{name}`"),
        ));
        return None;
    }
    pos += 1; // consume ':'

    let mut val_slice = trim_ws(&trimmed[pos..]);
    let mut important = false;

    // Check trailing `!` <ws>* `important` (ASCII case-insensitive).
    if let Some(stripped) = strip_trailing_important(val_slice) {
        important = true;
        val_slice = trim_ws(stripped);
    }

    let value_span =
        span_of_slice(val_slice).unwrap_or_else(|| Span::new(full_span.end, full_span.end));

    Some(Declaration {
        name: name.clone(),
        value: val_slice.to_vec(),
        important,
        span: full_span,
        name_span: *name_span,
        value_span,
    })
}

fn strip_trailing_important(values: &[ComponentValue]) -> Option<&[ComponentValue]> {
    let mut end = values.len();
    while end > 0 && values[end - 1].is_whitespace() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    let last_is_important = matches!(
        values[end - 1].as_ident(),
        Some(ident) if ident.eq_ignore_ascii_case("important")
    );
    if !last_is_important {
        return None;
    }
    end -= 1;
    while end > 0 && values[end - 1].is_whitespace() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    if matches!(values[end - 1].as_token(), Some(Token::Delim('!'))) {
        Some(&values[..end - 1])
    } else {
        None
    }
}

// ============================================================================
// ComponentValue slice helpers
// ============================================================================

/// Trim leading and trailing [`Token::Whitespace`] component values.
#[inline]
pub fn trim_ws(mut values: &[ComponentValue]) -> &[ComponentValue] {
    while let Some(first) = values.first() {
        if first.is_whitespace() {
            values = &values[1..];
        } else {
            break;
        }
    }
    while let Some(last) = values.last() {
        if last.is_whitespace() {
            values = &values[..values.len() - 1];
        } else {
            break;
        }
    }
    values
}

/// Split a slice of component values on top-level [`Token::Comma`], trimming
/// whitespace around each group.
///
/// Because functions (`calc(...)`, `rgb(...)`, `var(...)`) and blocks (`[...]`)
/// are single [`ComponentValue`] variants, inner commas are never split.
pub fn split_comma_groups(values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut groups = Vec::new();
    let mut start = 0;
    for (i, cv) in trimmed.iter().enumerate() {
        if matches!(cv.as_token(), Some(Token::Comma)) {
            groups.push(trim_ws(&trimmed[start..i]));
            start = i + 1;
        }
    }
    groups.push(trim_ws(&trimmed[start..]));
    groups
}

/// Split a slice of component values on top-level [`Token::Whitespace`] runs.
///
/// Each returned slice contains one or more adjacent non-whitespace
/// [`ComponentValue`]s. A function like `calc(10px + 2px)` is a single
/// [`ComponentValue::Function`] and is therefore NEVER split on its inner spaces.
pub fn split_whitespace_items(values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < values.len() {
        while i < values.len() && values[i].is_whitespace() {
            i += 1;
        }
        if i >= values.len() {
            break;
        }
        let start = i;
        while i < values.len() && !values[i].is_whitespace() {
            i += 1;
        }
        items.push(&values[start..i]);
    }
    items
}

/// Verify that only optional whitespace remains in `values`; otherwise return
/// the [`Span`] of the first unexpected component value.
#[inline]
pub fn expect_eof(values: &[ComponentValue]) -> Result<(), Span> {
    let trimmed = trim_ws(values);
    if let Some(first) = trimmed.first() {
        Err(first.span())
    } else {
        Ok(())
    }
}

/// Compute the enclosing [`Span`] of a slice of component values, or `None` if empty.
#[inline]
pub fn span_of_slice(values: &[ComponentValue]) -> Option<Span> {
    let first = values.first()?.span();
    let last = values.last()?.span();
    Some(first.merge(last))
}

/// Serialize a slice of [`ComponentValue`]s back to canonical CSS syntax text
/// (useful for diagnostics, debugging, and custom-property serialization).
pub fn serialize_component_values(values: &[ComponentValue]) -> String {
    let mut out = String::new();
    write_component_values(values, &mut out, 0);
    out
}

fn write_component_values(values: &[ComponentValue], out: &mut String, depth: usize) {
    if depth > MAX_NESTING {
        return;
    }
    for cv in values {
        match cv {
            ComponentValue::Token(st) => write_token(&st.token, out),
            ComponentValue::SimpleBlock { kind, values, .. } => {
                out.push(kind.open_char());
                write_component_values(values, out, depth + 1);
                out.push(kind.close_char());
            }
            ComponentValue::Function { name, args, .. } => {
                out.push_str(name);
                out.push('(');
                write_component_values(args, out, depth + 1);
                out.push(')');
            }
        }
    }
}

fn write_token(tok: &Token, out: &mut String) {
    match tok {
        Token::Ident(s) => out.push_str(s),
        Token::Function(s) => {
            out.push_str(s);
            out.push('(');
        }
        Token::AtKeyword(s) => {
            out.push('@');
            out.push_str(s);
        }
        Token::Hash { value, .. } => {
            out.push('#');
            out.push_str(value);
        }
        Token::String(s) => {
            out.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\a "),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        Token::BadString => out.push('"'),
        Token::Url(s) => {
            out.push_str("url(");
            out.push_str(s);
            out.push(')');
        }
        Token::BadUrl => out.push_str("url()"),
        Token::Delim(c) => out.push(*c),
        Token::Number { raw, .. } => out.push_str(raw),
        Token::Percentage(v) => {
            let _ = std::fmt::Write::write_fmt(out, format_args!("{v}%"));
        }
        Token::Dimension { value, unit } => {
            let _ = std::fmt::Write::write_fmt(out, format_args!("{value}{unit}"));
        }
        Token::Whitespace => out.push(' '),
        Token::Colon => out.push(':'),
        Token::Semicolon => out.push(';'),
        Token::Comma => out.push(','),
        Token::Cdo => out.push_str("<!--"),
        Token::Cdc => out.push_str("-->"),
        Token::LBracket => out.push('['),
        Token::RBracket => out.push(']'),
        Token::LParen => out.push('('),
        Token::RParen => out.push(')'),
        Token::LBrace => out.push('{'),
        Token::RBrace => out.push('}'),
        Token::IncludeMatch => out.push_str("~="),
        Token::DashMatch => out.push_str("|="),
        Token::PrefixMatch => out.push_str("^="),
        Token::SuffixMatch => out.push_str("$="),
        Token::SubstringMatch => out.push_str("*="),
        Token::Column => out.push_str("||"),
        Token::Eof => {}
    }
}

// ============================================================================
// Internal token-stream parser (strictly bounded recursion)
// ============================================================================

struct TokenParser<'a> {
    tokens: &'a [SpannedToken],
    pos: usize,
}

impl<'a> TokenParser<'a> {
    fn new(tokens: &'a [SpannedToken]) -> Self {
        Self { tokens, pos: 0 }
    }

    #[inline]
    fn at_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    #[inline]
    fn peek(&self) -> Option<&'a SpannedToken> {
        self.tokens.get(self.pos)
    }

    #[inline]
    fn bump(&mut self) -> Option<&'a SpannedToken> {
        let t = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(t)
    }

    #[inline]
    fn last_end(&self) -> usize {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span.end
        } else {
            0
        }
    }

    fn consume_rule_list(&mut self, top_level: bool, diags: &mut Vec<CssDiag>) -> Vec<Rule> {
        let mut rules = Vec::new();
        while let Some(tok) = self.peek() {
            match &tok.token {
                Token::Whitespace => {
                    self.bump();
                }
                Token::Cdo | Token::Cdc if top_level => {
                    self.bump();
                }
                Token::AtKeyword(_) => {
                    if let Some(at) = self.consume_at_rule(0, diags) {
                        rules.push(Rule::At(at));
                    }
                }
                _ => {
                    if let Some(q) = self.consume_qualified_rule(0, diags) {
                        rules.push(Rule::Qualified(q));
                    }
                }
            }
        }
        rules
    }

    fn consume_at_rule(&mut self, depth: usize, diags: &mut Vec<CssDiag>) -> Option<AtRule> {
        let head = self.bump()?;
        let Token::AtKeyword(name) = &head.token else {
            return None;
        };
        let start = head.span.start;
        let mut prelude = Vec::new();
        while let Some(tok) = self.peek() {
            match &tok.token {
                Token::Semicolon => {
                    let semi = self.bump().unwrap();
                    let trimmed_prelude = trim_ws(&prelude).to_vec();
                    let prelude_span = span_of_slice(&trimmed_prelude)
                        .unwrap_or_else(|| Span::new(head.span.end, semi.span.start));
                    return Some(AtRule {
                        name: name.clone(),
                        prelude: trimmed_prelude,
                        block: None,
                        span: Span::new(start, semi.span.end),
                        prelude_span,
                        block_span: None,
                    });
                }
                Token::LBrace => {
                    let block_cv = self.consume_simple_block(BlockKind::Curly, depth, diags);
                    let ComponentValue::SimpleBlock {
                        span: block_span,
                        values: block_vals,
                        ..
                    } = block_cv
                    else {
                        unreachable!();
                    };
                    let trimmed_prelude = trim_ws(&prelude).to_vec();
                    let prelude_span = span_of_slice(&trimmed_prelude)
                        .unwrap_or_else(|| Span::new(head.span.end, block_span.start));
                    return Some(AtRule {
                        name: name.clone(),
                        prelude: trimmed_prelude,
                        block: Some(block_vals),
                        span: Span::new(start, block_span.end),
                        prelude_span,
                        block_span: Some(block_span),
                    });
                }
                _ => {
                    if let Some(cv) = self.consume_component_value(depth, diags) {
                        prelude.push(cv);
                    }
                }
            }
        }
        // Reached EOF without `;` or `{`: return statement at-rule per W3C spec.
        let end = self.last_end();
        let trimmed_prelude = trim_ws(&prelude).to_vec();
        let prelude_span =
            span_of_slice(&trimmed_prelude).unwrap_or_else(|| Span::new(head.span.end, end));
        Some(AtRule {
            name: name.clone(),
            prelude: trimmed_prelude,
            block: None,
            span: Span::new(start, end),
            prelude_span,
            block_span: None,
        })
    }

    fn consume_qualified_rule(
        &mut self,
        depth: usize,
        diags: &mut Vec<CssDiag>,
    ) -> Option<QualifiedRule> {
        let start = self.peek()?.span.start;
        let mut prelude = Vec::new();
        while let Some(tok) = self.peek() {
            if matches!(tok.token, Token::LBrace) {
                let block_cv = self.consume_simple_block(BlockKind::Curly, depth, diags);
                let ComponentValue::SimpleBlock {
                    span: block_span,
                    values: block_vals,
                    ..
                } = block_cv
                else {
                    unreachable!();
                };
                let trimmed_prelude = trim_ws(&prelude).to_vec();
                let prelude_span = span_of_slice(&trimmed_prelude)
                    .unwrap_or_else(|| Span::new(start, block_span.start));
                return Some(QualifiedRule {
                    prelude: trimmed_prelude,
                    block: block_vals,
                    span: Span::new(start, block_span.end),
                    prelude_span,
                    block_span,
                });
            }
            if let Some(cv) = self.consume_component_value(depth, diags) {
                prelude.push(cv);
            }
        }
        // EOF without `{`: parse error, discard rule.
        if !trim_ws(&prelude).is_empty() {
            diags.push(CssDiag::error(
                Span::new(start, self.last_end()),
                DiagCode::SyntaxError,
                "qualified rule missing `{ ... }` block before end of input",
            ));
        }
        None
    }

    fn consume_component_value(
        &mut self,
        depth: usize,
        diags: &mut Vec<CssDiag>,
    ) -> Option<ComponentValue> {
        let tok = self.peek()?;
        match &tok.token {
            Token::LBrace => Some(self.consume_simple_block(BlockKind::Curly, depth, diags)),
            Token::LBracket => Some(self.consume_simple_block(BlockKind::Square, depth, diags)),
            Token::LParen => Some(self.consume_simple_block(BlockKind::Paren, depth, diags)),
            Token::Function(_) => Some(self.consume_function(depth, diags)),
            _ => {
                let t = self.bump().unwrap();
                Some(ComponentValue::Token(t.clone()))
            }
        }
    }

    fn consume_simple_block(
        &mut self,
        kind: BlockKind,
        depth: usize,
        diags: &mut Vec<CssDiag>,
    ) -> ComponentValue {
        let open = self.bump().expect("opening delimiter");
        let start = open.span.start;
        if depth >= MAX_NESTING {
            let end = self.skip_balanced_iteratively(kind, open.span.end);
            diags.push(CssDiag::error(
                Span::new(start, end),
                DiagCode::NestingLimitExceeded,
                format!("CSS nesting limit ({MAX_NESTING}) exceeded"),
            ));
            return ComponentValue::SimpleBlock {
                kind,
                span: Span::new(start, end),
                values: Vec::new(),
            };
        }

        let mut values = Vec::new();
        while let Some(tok) = self.peek() {
            let is_close = matches!(
                (kind, &tok.token),
                (BlockKind::Curly, Token::RBrace)
                    | (BlockKind::Square, Token::RBracket)
                    | (BlockKind::Paren, Token::RParen)
            );
            if is_close {
                let close = self.bump().unwrap();
                return ComponentValue::SimpleBlock {
                    kind,
                    span: Span::new(start, close.span.end),
                    values,
                };
            }
            if let Some(cv) = self.consume_component_value(depth + 1, diags) {
                values.push(cv);
            }
        }

        // Unclosed block at EOF.
        let end = self.last_end().max(open.span.end);
        ComponentValue::SimpleBlock {
            kind,
            span: Span::new(start, end),
            values,
        }
    }

    fn consume_function(&mut self, depth: usize, diags: &mut Vec<CssDiag>) -> ComponentValue {
        let head = self.bump().expect("function token");
        let start = head.span.start;
        let name = match &head.token {
            Token::Function(n) => n.clone(),
            _ => unreachable!(),
        };
        if depth >= MAX_NESTING {
            let end = self.skip_balanced_iteratively(BlockKind::Paren, head.span.end);
            diags.push(CssDiag::error(
                Span::new(start, end),
                DiagCode::NestingLimitExceeded,
                format!("CSS function nesting limit ({MAX_NESTING}) exceeded in `{name}(...)`"),
            ));
            return ComponentValue::Function {
                name,
                span: Span::new(start, end),
                args: Vec::new(),
            };
        }

        let mut args = Vec::new();
        while let Some(tok) = self.peek() {
            if matches!(tok.token, Token::RParen) {
                let close = self.bump().unwrap();
                return ComponentValue::Function {
                    name,
                    span: Span::new(start, close.span.end),
                    args,
                };
            }
            if let Some(cv) = self.consume_component_value(depth + 1, diags) {
                args.push(cv);
            }
        }

        let end = self.last_end().max(head.span.end);
        ComponentValue::Function {
            name,
            span: Span::new(start, end),
            args,
        }
    }

    /// Iteratively skip tokens until the matching closing delimiter for
    /// `initial_kind` is consumed (zero recursion).
    fn skip_balanced_iteratively(&mut self, initial_kind: BlockKind, fallback_end: usize) -> usize {
        let mut stack = vec![initial_kind];
        let mut last_end = fallback_end;
        while let Some(tok) = self.bump() {
            last_end = tok.span.end;
            match &tok.token {
                Token::LBrace => stack.push(BlockKind::Curly),
                Token::LBracket => stack.push(BlockKind::Square),
                Token::LParen | Token::Function(_) => stack.push(BlockKind::Paren),
                Token::RBrace if stack.last() == Some(&BlockKind::Curly) => {
                    stack.pop();
                    if stack.is_empty() {
                        return last_end;
                    }
                }
                Token::RBracket if stack.last() == Some(&BlockKind::Square) => {
                    stack.pop();
                    if stack.is_empty() {
                        return last_end;
                    }
                }
                Token::RParen if stack.last() == Some(&BlockKind::Paren) => {
                    stack.pop();
                    if stack.is_empty() {
                        return last_end;
                    }
                }
                _ => {}
            }
        }
        last_end
    }
}

// ============================================================================
// ComponentValue-slice rule parser (for rules inside at-rule blocks)
// ============================================================================

struct CvParser<'a> {
    values: &'a [ComponentValue],
    pos: usize,
}

impl<'a> CvParser<'a> {
    fn new(values: &'a [ComponentValue]) -> Self {
        Self { values, pos: 0 }
    }

    fn peek(&self) -> Option<&'a ComponentValue> {
        self.values.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a ComponentValue> {
        let cv = self.values.get(self.pos)?;
        self.pos += 1;
        Some(cv)
    }

    fn consume_rule_list(&mut self, diags: &mut Vec<CssDiag>) -> Vec<Rule> {
        let mut rules = Vec::new();
        while let Some(cv) = self.peek() {
            if cv.is_whitespace() {
                self.bump();
                continue;
            }
            if matches!(cv.as_token(), Some(Token::AtKeyword(_))) {
                if let Some(at) = self.consume_at_rule(diags) {
                    rules.push(Rule::At(at));
                }
            } else if let Some(q) = self.consume_qualified_rule(diags) {
                rules.push(Rule::Qualified(q));
            }
        }
        rules
    }

    fn consume_at_rule(&mut self, diags: &mut Vec<CssDiag>) -> Option<AtRule> {
        let head = self.bump()?;
        let ComponentValue::Token(SpannedToken {
            token: Token::AtKeyword(name),
            span: head_span,
        }) = head
        else {
            return None;
        };
        let start = head_span.start;
        let mut prelude = Vec::new();
        while let Some(cv) = self.peek() {
            match cv {
                ComponentValue::Token(SpannedToken {
                    token: Token::Semicolon,
                    span: semi_span,
                }) => {
                    self.bump();
                    let trimmed = trim_ws(&prelude).to_vec();
                    let prelude_span = span_of_slice(&trimmed)
                        .unwrap_or_else(|| Span::new(head_span.end, semi_span.start));
                    return Some(AtRule {
                        name: name.clone(),
                        prelude: trimmed,
                        block: None,
                        span: Span::new(start, semi_span.end),
                        prelude_span,
                        block_span: None,
                    });
                }
                ComponentValue::SimpleBlock {
                    kind: BlockKind::Curly,
                    span: block_span,
                    values: block_vals,
                } => {
                    self.bump();
                    let trimmed = trim_ws(&prelude).to_vec();
                    let prelude_span = span_of_slice(&trimmed)
                        .unwrap_or_else(|| Span::new(head_span.end, block_span.start));
                    return Some(AtRule {
                        name: name.clone(),
                        prelude: trimmed,
                        block: Some(block_vals.clone()),
                        span: Span::new(start, block_span.end),
                        prelude_span,
                        block_span: Some(*block_span),
                    });
                }
                _ => {
                    prelude.push(self.bump().unwrap().clone());
                }
            }
        }
        let end = prelude
            .last()
            .map(ComponentValue::span)
            .map_or(head_span.end, |s| s.end);
        let trimmed = trim_ws(&prelude).to_vec();
        let prelude_span = span_of_slice(&trimmed).unwrap_or_else(|| Span::new(head_span.end, end));
        let _ = diags;
        Some(AtRule {
            name: name.clone(),
            prelude: trimmed,
            block: None,
            span: Span::new(start, end),
            prelude_span,
            block_span: None,
        })
    }

    fn consume_qualified_rule(&mut self, diags: &mut Vec<CssDiag>) -> Option<QualifiedRule> {
        let start = self.peek()?.span().start;
        let mut prelude = Vec::new();
        while let Some(cv) = self.peek() {
            if let ComponentValue::SimpleBlock {
                kind: BlockKind::Curly,
                span: block_span,
                values: block_vals,
            } = cv
            {
                self.bump();
                let trimmed = trim_ws(&prelude).to_vec();
                let prelude_span =
                    span_of_slice(&trimmed).unwrap_or_else(|| Span::new(start, block_span.start));
                return Some(QualifiedRule {
                    prelude: trimmed,
                    block: block_vals.clone(),
                    span: Span::new(start, block_span.end),
                    prelude_span,
                    block_span: *block_span,
                });
            }
            prelude.push(self.bump().unwrap().clone());
        }
        if !trim_ws(&prelude).is_empty() {
            let end = prelude.last().map_or(start, |c| c.span().end);
            diags.push(CssDiag::error(
                Span::new(start, end),
                DiagCode::SyntaxError,
                "qualified rule missing `{ ... }` block",
            ));
        }
        None
    }
}
