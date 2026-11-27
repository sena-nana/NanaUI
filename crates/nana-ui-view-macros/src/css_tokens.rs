//! CSS written as Rust tokens (`view!`'s `<style>`, `css!`, `stylesheet!`) or
//! as a raw string literal (`css!(r#"..."#)`), turned into CSS text with a map
//! from each piece of the text back to its token span.
//!
//! In Token mode, whitespace comes back from where the tokens sit in the source,
//! so `.a.b` and `.a .b` stay apart. Double-quoted strings stay quoted on
//! string-expecting CSS properties (`grid-template-areas`, `font-feature-settings`,
//! `font-variation-settings`, `font-family`, `content`, `src`, etc.), inside
//! `url(...)`, and in selectors; on non-string properties (`color: "#9ecafe"`,
//! `font-size: "1.5em"`), a string is spliced without its quotes as the escape
//! hatch for values Rust cannot tokenize.

use std::ops::Range;

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use quote::ToTokens;

pub(crate) struct CssText {
    pub(crate) text: String,
    /// Each token's text, in order.
    pieces: Vec<(Range<usize>, Span)>,
    /// Where the CSS was written, for a range that maps to no token.
    whole: Span,
}

impl CssText {
    /// Wrap a raw CSS string literal (`css!(r#"..."#)` / `<style>r#"..."#</style>`).
    pub(crate) fn from_lit(lit: &syn::LitStr) -> Self {
        let text = lit.value();
        let span = lit.span();
        Self {
            pieces: vec![(0..text.len(), span)],
            text,
            whole: span,
        }
    }

    /// Empty CSS text anchored at `whole`.
    pub(crate) fn empty(whole: Span) -> Self {
        Self {
            text: String::new(),
            pieces: Vec::new(),
            whole,
        }
    }

    /// Where `.class` is first written in a selector.
    pub(crate) fn class_span(&self, class: &str) -> Span {
        let name = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
        let dotted = format!(".{class}");
        self.text
            .match_indices(&dotted)
            .find(|(at, _)| !self.text[at + dotted.len()..].starts_with(name))
            .map_or(self.whole, |(at, _)| {
                self.span(&(at + 1..at + dotted.len()))
            })
    }

    /// The token a range of the text starts in (or the next one).
    pub(crate) fn span(&self, range: &Range<usize>) -> Span {
        self.pieces
            .iter()
            .find(|(piece, _)| piece.end > range.start)
            .map_or(self.whole, |(_, span)| *span)
    }
}

/// CSS properties whose grammar expects CSS string tokens (`"..."`), so Token
/// mode must preserve double quotes rather than unwrapping them.
fn prop_preserves_quotes(prop: &str) -> bool {
    matches!(
        prop.trim().to_ascii_lowercase().as_str(),
        "grid-template-areas"
            | "grid-template"
            | "grid"
            | "font-feature-settings"
            | "font-variation-settings"
            | "font-family"
            | "font"
            | "content"
            | "src"
            | "quotes"
    )
}

/// Where the builder is inside the CSS.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    /// Selectors and at-rule preludes.
    Rules,
    /// A declaration block, before a declaration's `:`.
    Property,
    /// After it, up to `;`.
    Value,
}

struct Builder {
    out: CssText,
    last: Option<proc_macro2::LineColumn>,
    /// The previous token was the identifier `url` or `local` or `format`.
    after_quoted_fn: bool,
    /// Property name accumulated in `Place::Property`.
    current_prop: String,
}

impl Builder {
    fn push(&mut self, text: &str, span: Span) {
        let start = span.start();
        let gap = match self.last {
            None => false,
            Some(last) if start.line != last.line => {
                self.out.text.push('\n');
                false
            }
            Some(last) if start.column >= last.column => start.column > last.column,
            // Tokens a macro made carry no usable position: space words
            // apart and nothing else.
            Some(_) => {
                let word = |c: char| c.is_alphanumeric() || matches!(c, '%' | '"' | ')' | '_');
                self.out.text.ends_with(word) && text.starts_with(word)
            }
        };
        if gap {
            self.out.text.push(' ');
        }
        let at = self.out.text.len();
        self.out.text.push_str(text);
        self.out.pieces.push((at..self.out.text.len(), span));
        self.last = Some(span.end());
    }

    fn stream(&mut self, tokens: TokenStream, mut place: Place, in_quoted_fn: bool) {
        // An at-rule's block holds rules (except `@font-face`, which holds descriptors).
        let mut at_rule = false;
        let mut at_rule_name: Option<String> = None;
        for token in tokens {
            let after_quoted_fn = std::mem::take(&mut self.after_quoted_fn);
            match token {
                TokenTree::Group(group) => {
                    let (open, close, inner, quoted_fn, reset_to_prop) = match group.delimiter() {
                        Delimiter::Brace if place == Place::Rules => {
                            let is_decl_at_rule =
                                at_rule_name.as_deref().is_some_and(|n| n == "font-face");
                            let inner = if at_rule && !is_decl_at_rule {
                                Place::Rules
                            } else {
                                Place::Property
                            };
                            ("{", "}", inner, false, false)
                        }
                        Delimiter::Brace => {
                            // Nested style rule block inside a declaration block (CSS Nesting).
                            ("{", "}", Place::Property, false, true)
                        }
                        Delimiter::Parenthesis => {
                            ("(", ")", place, in_quoted_fn || after_quoted_fn, false)
                        }
                        Delimiter::Bracket => ("[", "]", place, in_quoted_fn, false),
                        Delimiter::None => {
                            self.stream(group.stream(), place, in_quoted_fn);
                            continue;
                        }
                    };
                    let saved_prop = if open == "{" {
                        std::mem::take(&mut self.current_prop)
                    } else {
                        String::new()
                    };
                    self.push(open, group.span_open());
                    self.stream(group.stream(), inner, quoted_fn);
                    self.push(close, group.span_close());
                    if open == "{" {
                        self.current_prop = if reset_to_prop {
                            place = Place::Property;
                            String::new()
                        } else {
                            saved_prop
                        };
                        if place == Place::Rules {
                            at_rule = false;
                            at_rule_name = None;
                        }
                    }
                }
                TokenTree::Punct(punct) => {
                    let c = punct.as_char();
                    match (place, c) {
                        (Place::Rules, '@') => {
                            at_rule = true;
                            at_rule_name = None;
                        }
                        (Place::Rules, ';') => {
                            at_rule = false;
                            at_rule_name = None;
                        }
                        (Place::Property, ':') => place = Place::Value,
                        (Place::Property, '-') => self.current_prop.push('-'),
                        (Place::Value, ';') => {
                            place = Place::Property;
                            self.current_prop.clear();
                        }
                        _ => {}
                    }
                    self.push(&c.to_string(), punct.span());
                }
                TokenTree::Ident(ident) => {
                    let s = ident.to_string();
                    if place == Place::Rules && at_rule && at_rule_name.is_none() {
                        at_rule_name = Some(s.to_ascii_lowercase());
                    } else if place == Place::Property {
                        self.current_prop.push_str(&s);
                    }
                    self.after_quoted_fn = s.eq_ignore_ascii_case("url")
                        || s.eq_ignore_ascii_case("local")
                        || s.eq_ignore_ascii_case("format");
                    self.push(&s, ident.span());
                }
                TokenTree::Literal(literal) => {
                    let text = match syn::parse2::<syn::LitStr>(literal.to_token_stream()) {
                        Ok(string)
                            if place == Place::Value
                                && !in_quoted_fn
                                && !prop_preserves_quotes(&self.current_prop) =>
                        {
                            string.value()
                        }
                        Ok(string) => format!("{:?}", string.value()),
                        Err(_) => literal.to_string(),
                    };
                    self.push(&text, literal.span());
                }
            }
        }
    }
}

/// `tokens` as CSS: a sheet (`declarations == false`) or one declaration
/// block's contents.
pub(crate) fn css(tokens: TokenStream, declarations: bool, whole: Span) -> CssText {
    let mut builder = Builder {
        out: CssText {
            text: String::new(),
            pieces: Vec::new(),
            whole,
        },
        last: None,
        after_quoted_fn: false,
        current_prop: String::new(),
    };
    let place = if declarations {
        Place::Property
    } else {
        Place::Rules
    };
    builder.stream(tokens, place, false);
    builder.out
}
