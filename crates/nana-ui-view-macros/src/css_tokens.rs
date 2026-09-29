//! CSS written as Rust tokens (`view!`'s `<style>`, `css!`), turned back
//! into CSS text with a map from each piece of the text to its token.
//!
//! Whitespace comes back from where the tokens sit in the source, so
//! `.a.b` and `.a .b` stay apart. A string in a declaration's value is
//! spliced without its quotes: that is how a value Rust cannot tokenize is
//! written (`font-size: "1.5em"`, `color: "#9ecafe"`). A string inside
//! `url(…)` or in a selector keeps its quotes.

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
    /// The previous token was the identifier `url`.
    after_url: bool,
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

    fn stream(&mut self, tokens: TokenStream, mut place: Place, in_url: bool) {
        // An at-rule's block holds rules, a rule's declarations.
        let mut at_rule = false;
        for token in tokens {
            let after_url = std::mem::take(&mut self.after_url);
            match token {
                TokenTree::Group(group) => {
                    let (open, close, inner, url) = match group.delimiter() {
                        Delimiter::Brace if place == Place::Rules => {
                            let inner = if at_rule {
                                Place::Rules
                            } else {
                                Place::Property
                            };
                            ("{", "}", inner, false)
                        }
                        Delimiter::Brace => ("{", "}", place, in_url),
                        Delimiter::Parenthesis => ("(", ")", place, in_url || after_url),
                        Delimiter::Bracket => ("[", "]", place, in_url),
                        Delimiter::None => {
                            self.stream(group.stream(), place, in_url);
                            continue;
                        }
                    };
                    self.push(open, group.span_open());
                    self.stream(group.stream(), inner, url);
                    self.push(close, group.span_close());
                    if place == Place::Rules && open == "{" {
                        at_rule = false;
                    }
                }
                TokenTree::Punct(punct) => {
                    let c = punct.as_char();
                    match (place, c) {
                        (Place::Rules, '@') => at_rule = true,
                        (Place::Rules, ';') => at_rule = false,
                        (Place::Property, ':') => place = Place::Value,
                        (Place::Value, ';') => place = Place::Property,
                        _ => {}
                    }
                    self.push(&c.to_string(), punct.span());
                }
                TokenTree::Ident(ident) => {
                    self.after_url = ident == "url";
                    self.push(&ident.to_string(), ident.span());
                }
                TokenTree::Literal(literal) => {
                    let text = match syn::parse2::<syn::LitStr>(literal.to_token_stream()) {
                        Ok(string) if place == Place::Value && !in_url => string.value(),
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
        after_url: false,
    };
    let place = if declarations {
        Place::Property
    } else {
        Place::Rules
    };
    builder.stream(tokens, place, false);
    builder.out
}
