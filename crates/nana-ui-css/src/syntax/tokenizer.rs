//! Strict W3C CSS Syntax Level 3 tokenizer operating on UTF-8 `char` streams.
//!
//! Guarantees:
//! - Never casts raw UTF-8 bytes to `char` (full multi-byte UTF-8 support in
//!   identifiers, strings, URLs, and comments).
//! - Comments (`/* ... */`) are stripped at lex time and never recognized inside
//!   strings or unquoted URLs.
//! - Implements CSS escape sequences (`\xxxxxx` + optional single whitespace,
//!   and single-character escapes).
//! - Distinguishes unquoted `url(...)` (`Token::Url`) from quoted `url("...")`
//!   (`Token::Function("url")` + `Token::String` + `Token::RParen`) per W3C
//!   CSS Syntax Level 3 §4.3.4 / §4.3.6.
//! - Supports scientific notation numbers (`1e-2`, `+3.5E+1`) and preserves exact
//!   byte [`Span`]s into the original source string.

use super::diag::{CssDiag, DiagCode};
use super::span::Span;

/// A CSS Syntax Level 3 token.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Function(String),
    AtKeyword(String),
    Hash {
        value: String,
        /// `true` for W3C `id` flag (valid ident sequence after `#`),
        /// `false` for `unrestricted` (e.g. hex color `#123`).
        id_flag: bool,
    },
    String(String),
    BadString,
    Url(String),
    BadUrl,
    Delim(char),
    Number {
        value: f64,
        int: Option<i64>,
        raw: String,
    },
    Percentage(f64),
    Dimension {
        value: f64,
        unit: String,
    },
    Whitespace,
    Colon,
    Semicolon,
    Comma,
    Cdo,
    Cdc,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    IncludeMatch,
    DashMatch,
    PrefixMatch,
    SuffixMatch,
    SubstringMatch,
    Column,
    Eof,
}

/// A [`Token`] paired with its exact byte [`Span`] in the input source.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: Span,
}

impl SpannedToken {
    #[inline]
    pub const fn new(token: Token, span: Span) -> Self {
        Self { token, span }
    }
}

/// Tokenize a complete CSS source string into spanned tokens (excluding a
/// trailing `Eof` token; use [`Tokenizer`] directly if streaming with `Eof`).
pub fn tokenize(src: &str, diags: &mut Vec<CssDiag>) -> Vec<SpannedToken> {
    let mut lexer = Tokenizer::new(src);
    let mut out = Vec::new();
    loop {
        let tok = lexer.next_token(diags);
        if matches!(tok.token, Token::Eof) {
            break;
        }
        out.push(tok);
    }
    out
}

/// Streaming W3C CSS Syntax Level 3 lexer.
pub struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    #[inline]
    pub const fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    #[inline]
    pub const fn position(&self) -> usize {
        self.pos
    }

    #[inline]
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    #[inline]
    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Peek the nth Unicode scalar from `self.pos` (0-indexed), with CSS input
    /// stream preprocessing (`\r\n`, `\r`, `\u{000C}` normalized to `\n`,
    /// `\0` normalized to `\u{FFFD}`).
    fn peek_preprocessed(&self, n: usize) -> Option<char> {
        let mut offset = self.pos;
        for step in 0..=n {
            let (ch, next_offset) = next_preprocessed_at(self.src, offset)?;
            if step == n {
                return Some(ch);
            }
            offset = next_offset;
        }
        None
    }

    /// Consume one preprocessed character and advance `self.pos`.
    fn bump(&mut self) -> Option<char> {
        let (ch, next_offset) = next_preprocessed_at(self.src, self.pos)?;
        self.pos = next_offset;
        Some(ch)
    }

    /// Advance past a raw ASCII byte sequence if it matches at `self.pos`.
    fn starts_with(&self, pat: &str) -> bool {
        self.rest().starts_with(pat)
    }

    /// Consume all leading `/* ... */` comments.
    fn consume_comments(&mut self, diags: &mut Vec<CssDiag>) {
        while self.starts_with("/*") {
            let start = self.pos;
            self.pos += 2;
            if let Some(rel_end) = self.rest().find("*/") {
                self.pos += rel_end + 2;
            } else {
                self.pos = self.src.len();
                diags.push(CssDiag::warning(
                    Span::new(start, self.pos),
                    DiagCode::UnterminatedComment,
                    "unterminated CSS comment",
                ));
                break;
            }
        }
    }

    /// Consume the next token per W3C CSS Syntax Level 3 §4.3.1.
    pub fn next_token(&mut self, diags: &mut Vec<CssDiag>) -> SpannedToken {
        self.consume_comments(diags);
        let start = self.pos;
        let Some(ch) = self.peek_preprocessed(0) else {
            return SpannedToken::new(Token::Eof, Span::new(start, start));
        };

        // Whitespace: U+0009 TAB, U+000A LF (incl. preprocessed CR/CRLF/FF), U+0020 SPACE
        if is_whitespace(ch) {
            while self.peek_preprocessed(0).is_some_and(is_whitespace) {
                self.bump();
            }
            return SpannedToken::new(Token::Whitespace, Span::new(start, self.pos));
        }

        // String tokens
        if ch == '"' || ch == '\'' {
            let quote = ch;
            self.bump();
            return self.consume_string_token(start, quote, diags);
        }

        // Hash token or Delim('#')
        if ch == '#' {
            self.bump();
            let next0 = self.peek_preprocessed(0);
            let next1 = self.peek_preprocessed(1);
            if next0.is_some_and(is_ident_char) || is_valid_escape(next0, next1) {
                let id_flag = self.would_start_ident();
                let value = self.consume_ident_sequence();
                return SpannedToken::new(
                    Token::Hash { value, id_flag },
                    Span::new(start, self.pos),
                );
            }
            return SpannedToken::new(Token::Delim('#'), Span::new(start, self.pos));
        }

        // Single-char punctuation
        match ch {
            '(' => {
                self.bump();
                return SpannedToken::new(Token::LParen, Span::new(start, self.pos));
            }
            ')' => {
                self.bump();
                return SpannedToken::new(Token::RParen, Span::new(start, self.pos));
            }
            ',' => {
                self.bump();
                return SpannedToken::new(Token::Comma, Span::new(start, self.pos));
            }
            ':' => {
                self.bump();
                return SpannedToken::new(Token::Colon, Span::new(start, self.pos));
            }
            ';' => {
                self.bump();
                return SpannedToken::new(Token::Semicolon, Span::new(start, self.pos));
            }
            '[' => {
                self.bump();
                return SpannedToken::new(Token::LBracket, Span::new(start, self.pos));
            }
            ']' => {
                self.bump();
                return SpannedToken::new(Token::RBracket, Span::new(start, self.pos));
            }
            '{' => {
                self.bump();
                return SpannedToken::new(Token::LBrace, Span::new(start, self.pos));
            }
            '}' => {
                self.bump();
                return SpannedToken::new(Token::RBrace, Span::new(start, self.pos));
            }
            _ => {}
        }

        // '+' or '.' -> numeric token or delim
        if ch == '+' || ch == '.' {
            if self.would_start_number() {
                return self.consume_numeric_token(start);
            }
            self.bump();
            return SpannedToken::new(Token::Delim(ch), Span::new(start, self.pos));
        }

        // '-' -> numeric, CDC (`-->`), ident-like, or delim
        if ch == '-' {
            if self.would_start_number() {
                return self.consume_numeric_token(start);
            }
            if self.starts_with("-->") {
                self.pos += 3;
                return SpannedToken::new(Token::Cdc, Span::new(start, self.pos));
            }
            if self.would_start_ident() {
                return self.consume_ident_like_token(start, diags);
            }
            self.bump();
            return SpannedToken::new(Token::Delim('-'), Span::new(start, self.pos));
        }

        // '<' -> CDO (`<!--`) or delim
        if ch == '<' {
            if self.starts_with("<!--") {
                self.pos += 4;
                return SpannedToken::new(Token::Cdo, Span::new(start, self.pos));
            }
            self.bump();
            return SpannedToken::new(Token::Delim('<'), Span::new(start, self.pos));
        }

        // '@' -> at-keyword or delim
        if ch == '@' {
            self.bump();
            if self.would_start_ident() {
                let name = self.consume_ident_sequence();
                return SpannedToken::new(Token::AtKeyword(name), Span::new(start, self.pos));
            }
            return SpannedToken::new(Token::Delim('@'), Span::new(start, self.pos));
        }

        // '\' -> escaped ident-like or syntax error delim
        if ch == '\\' {
            if is_valid_escape(self.peek_preprocessed(0), self.peek_preprocessed(1)) {
                return self.consume_ident_like_token(start, diags);
            }
            self.bump();
            diags.push(CssDiag::error(
                Span::new(start, self.pos),
                DiagCode::SyntaxError,
                "invalid escape sequence before newline or EOF",
            ));
            return SpannedToken::new(Token::Delim('\\'), Span::new(start, self.pos));
        }

        // Attribute match & column combinator two-char operators
        match ch {
            '~' if self.starts_with("~=") => {
                self.pos += 2;
                return SpannedToken::new(Token::IncludeMatch, Span::new(start, self.pos));
            }
            '|' if self.starts_with("|=") => {
                self.pos += 2;
                return SpannedToken::new(Token::DashMatch, Span::new(start, self.pos));
            }
            '|' if self.starts_with("||") => {
                self.pos += 2;
                return SpannedToken::new(Token::Column, Span::new(start, self.pos));
            }
            '^' if self.starts_with("^=") => {
                self.pos += 2;
                return SpannedToken::new(Token::PrefixMatch, Span::new(start, self.pos));
            }
            '$' if self.starts_with("$=") => {
                self.pos += 2;
                return SpannedToken::new(Token::SuffixMatch, Span::new(start, self.pos));
            }
            '*' if self.starts_with("*=") => {
                self.pos += 2;
                return SpannedToken::new(Token::SubstringMatch, Span::new(start, self.pos));
            }
            _ => {}
        }

        // Digit -> numeric token
        if ch.is_ascii_digit() {
            return self.consume_numeric_token(start);
        }

        // Ident-start code point -> ident-like token
        if is_ident_start_char(ch) {
            return self.consume_ident_like_token(start, diags);
        }

        // Fallback: single-character Delim
        self.bump();
        SpannedToken::new(Token::Delim(ch), Span::new(start, self.pos))
    }

    /// §4.3.9: Check if the next 3 code points would start an ident sequence.
    fn would_start_ident(&self) -> bool {
        let c0 = self.peek_preprocessed(0);
        let c1 = self.peek_preprocessed(1);
        let c2 = self.peek_preprocessed(2);
        match c0 {
            Some('-') => {
                c1.is_some_and(|c| is_ident_start_char(c) || c == '-') || is_valid_escape(c1, c2)
            }
            Some(c) if is_ident_start_char(c) => true,
            Some('\\') => is_valid_escape(c0, c1),
            _ => false,
        }
    }

    /// §4.3.10: Check if the next 3 code points would start a number.
    fn would_start_number(&self) -> bool {
        let c0 = self.peek_preprocessed(0);
        let c1 = self.peek_preprocessed(1);
        let c2 = self.peek_preprocessed(2);
        match c0 {
            Some('+' | '-') => {
                c1.is_some_and(|c| c.is_ascii_digit())
                    || (c1 == Some('.') && c2.is_some_and(|c| c.is_ascii_digit()))
            }
            Some('.') => c1.is_some_and(|c| c.is_ascii_digit()),
            Some(c) if c.is_ascii_digit() => true,
            _ => false,
        }
    }

    /// §4.3.7: Consume an escaped code point (assuming leading `\` already consumed).
    fn consume_escaped_code_point(&mut self) -> char {
        let Some(first) = self.peek_preprocessed(0) else {
            return '\u{FFFD}';
        };
        if first.is_ascii_hexdigit() {
            let mut val: u32 = 0;
            let mut count = 0usize;
            while count < 6 {
                let Some(hex) = self.peek_preprocessed(0) else {
                    break;
                };
                if let Some(d) = hex.to_digit(16) {
                    val = (val << 4) | d;
                    self.bump();
                    count += 1;
                } else {
                    break;
                }
            }
            // Optional single trailing whitespace is consumed with the hex escape.
            if self.peek_preprocessed(0).is_some_and(is_whitespace) {
                self.bump();
            }
            if val == 0 || val > 0x10_FFFF {
                '\u{FFFD}'
            } else {
                char::from_u32(val).unwrap_or('\u{FFFD}')
            }
        } else {
            self.bump().unwrap_or('\u{FFFD}')
        }
    }

    /// §4.3.11: Consume an ident sequence.
    fn consume_ident_sequence(&mut self) -> String {
        let mut out = String::new();
        while let Some(ch) = self.peek_preprocessed(0) {
            if is_ident_char(ch) {
                self.bump();
                out.push(ch);
            } else if is_valid_escape(Some(ch), self.peek_preprocessed(1)) {
                self.bump(); // consume '\'
                out.push(self.consume_escaped_code_point());
            } else {
                break;
            }
        }
        out
    }

    /// §4.3.4: Consume an ident-like token (`Ident`, `Function`, `Url`, `BadUrl`).
    fn consume_ident_like_token(&mut self, start: usize, diags: &mut Vec<CssDiag>) -> SpannedToken {
        let name = self.consume_ident_sequence();
        if name.eq_ignore_ascii_case("url") && self.peek_char() == Some('(') {
            self.bump(); // consume '('
            // Per §4.3.4: skip whitespace to check whether the argument is quoted.
            let after_open = self.pos;
            while self.peek_preprocessed(0).is_some_and(is_whitespace) {
                self.bump();
            }
            if matches!(self.peek_preprocessed(0), Some('"' | '\'')) {
                // Quoted url("...") -> Function("url"), rewind whitespace so parser sees it.
                self.pos = after_open;
                return SpannedToken::new(Token::Function(name), Span::new(start, self.pos));
            }
            return self.consume_url_token(start, diags);
        }
        if self.peek_char() == Some('(') {
            self.bump();
            return SpannedToken::new(Token::Function(name), Span::new(start, self.pos));
        }
        SpannedToken::new(Token::Ident(name), Span::new(start, self.pos))
    }

    /// §4.3.6: Consume an unquoted URL token (leading `url(` and whitespace already consumed).
    fn consume_url_token(&mut self, start: usize, diags: &mut Vec<CssDiag>) -> SpannedToken {
        let mut value = String::new();
        loop {
            match self.peek_preprocessed(0) {
                None => {
                    diags.push(CssDiag::error(
                        Span::new(start, self.pos),
                        DiagCode::BadUrl,
                        "unterminated url(...) token",
                    ));
                    return SpannedToken::new(Token::Url(value), Span::new(start, self.pos));
                }
                Some(')') => {
                    self.bump();
                    return SpannedToken::new(Token::Url(value), Span::new(start, self.pos));
                }
                Some(c) if is_whitespace(c) => {
                    while self.peek_preprocessed(0).is_some_and(is_whitespace) {
                        self.bump();
                    }
                    match self.peek_preprocessed(0) {
                        Some(')') => {
                            self.bump();
                            return SpannedToken::new(
                                Token::Url(value),
                                Span::new(start, self.pos),
                            );
                        }
                        None => {
                            diags.push(CssDiag::error(
                                Span::new(start, self.pos),
                                DiagCode::BadUrl,
                                "unterminated url(...) token",
                            ));
                            return SpannedToken::new(
                                Token::Url(value),
                                Span::new(start, self.pos),
                            );
                        }
                        Some(_) => {
                            self.consume_bad_url_remnants();
                            diags.push(CssDiag::error(
                                Span::new(start, self.pos),
                                DiagCode::BadUrl,
                                "unexpected whitespace inside unquoted url(...)",
                            ));
                            return SpannedToken::new(Token::BadUrl, Span::new(start, self.pos));
                        }
                    }
                }
                Some('"' | '\'' | '(') => {
                    self.consume_bad_url_remnants();
                    diags.push(CssDiag::error(
                        Span::new(start, self.pos),
                        DiagCode::BadUrl,
                        "invalid character inside unquoted url(...)",
                    ));
                    return SpannedToken::new(Token::BadUrl, Span::new(start, self.pos));
                }
                Some(c) if is_non_printable(c) => {
                    self.consume_bad_url_remnants();
                    diags.push(CssDiag::error(
                        Span::new(start, self.pos),
                        DiagCode::BadUrl,
                        "non-printable character inside url(...)",
                    ));
                    return SpannedToken::new(Token::BadUrl, Span::new(start, self.pos));
                }
                Some('\\') => {
                    if is_valid_escape(Some('\\'), self.peek_preprocessed(1)) {
                        self.bump();
                        value.push(self.consume_escaped_code_point());
                    } else {
                        self.consume_bad_url_remnants();
                        diags.push(CssDiag::error(
                            Span::new(start, self.pos),
                            DiagCode::BadUrl,
                            "invalid escape inside url(...)",
                        ));
                        return SpannedToken::new(Token::BadUrl, Span::new(start, self.pos));
                    }
                }
                Some(c) => {
                    self.bump();
                    value.push(c);
                }
            }
        }
    }

    /// §4.3.14: Consume remnants of a bad URL cleanly.
    fn consume_bad_url_remnants(&mut self) {
        while let Some(c) = self.peek_preprocessed(0) {
            if c == ')' {
                self.bump();
                break;
            }
            if is_valid_escape(Some(c), self.peek_preprocessed(1)) {
                self.bump();
                let _ = self.consume_escaped_code_point();
            } else {
                self.bump();
            }
        }
    }

    /// §4.3.5: Consume a string token (opening quote already consumed).
    fn consume_string_token(
        &mut self,
        start: usize,
        ending_quote: char,
        diags: &mut Vec<CssDiag>,
    ) -> SpannedToken {
        let mut out = String::new();
        loop {
            match self.peek_preprocessed(0) {
                None => {
                    diags.push(CssDiag::error(
                        Span::new(start, self.pos),
                        DiagCode::UnterminatedString,
                        "unterminated string literal",
                    ));
                    return SpannedToken::new(Token::String(out), Span::new(start, self.pos));
                }
                Some(c) if c == ending_quote => {
                    self.bump();
                    return SpannedToken::new(Token::String(out), Span::new(start, self.pos));
                }
                Some('\n') => {
                    // Unescaped newline in string -> BadString (do not consume newline).
                    diags.push(CssDiag::error(
                        Span::new(start, self.pos),
                        DiagCode::BadString,
                        "unescaped newline in string literal",
                    ));
                    return SpannedToken::new(Token::BadString, Span::new(start, self.pos));
                }
                Some('\\') => {
                    self.bump(); // consume '\'
                    match self.peek_preprocessed(0) {
                        None => {
                            // Trailing backslash at EOF: do nothing, loop hits EOF.
                        }
                        Some('\n') => {
                            // Line continuation: consume newline, emit nothing.
                            self.bump();
                        }
                        Some(_) => {
                            out.push(self.consume_escaped_code_point());
                        }
                    }
                }
                Some(c) => {
                    self.bump();
                    out.push(c);
                }
            }
        }
    }

    /// §4.3.3 & §4.3.12: Consume a numeric token (`Number`, `Percentage`, `Dimension`).
    fn consume_numeric_token(&mut self, start: usize) -> SpannedToken {
        let mut raw = String::new();
        let mut is_integer = true;

        if matches!(self.peek_preprocessed(0), Some('+' | '-'))
            && let Some(sign) = self.bump()
        {
            raw.push(sign);
        }

        while let Some(d) = self.peek_preprocessed(0) {
            if d.is_ascii_digit() {
                self.bump();
                raw.push(d);
            } else {
                break;
            }
        }

        if self.peek_preprocessed(0) == Some('.')
            && self
                .peek_preprocessed(1)
                .is_some_and(|c| c.is_ascii_digit())
        {
            is_integer = false;
            raw.push(self.bump().unwrap());
            while let Some(d) = self.peek_preprocessed(0) {
                if d.is_ascii_digit() {
                    self.bump();
                    raw.push(d);
                } else {
                    break;
                }
            }
        }

        if matches!(self.peek_preprocessed(0), Some('e' | 'E')) {
            let c1 = self.peek_preprocessed(1);
            let c2 = self.peek_preprocessed(2);
            let has_exp = c1.is_some_and(|c| c.is_ascii_digit())
                || (matches!(c1, Some('+' | '-')) && c2.is_some_and(|c| c.is_ascii_digit()));
            if has_exp {
                is_integer = false;
                raw.push(self.bump().unwrap()); // 'e' or 'E'
                if matches!(self.peek_preprocessed(0), Some('+' | '-')) {
                    raw.push(self.bump().unwrap());
                }
                while let Some(d) = self.peek_preprocessed(0) {
                    if d.is_ascii_digit() {
                        self.bump();
                        raw.push(d);
                    } else {
                        break;
                    }
                }
            }
        }

        let value = raw.parse::<f64>().unwrap_or(0.0);
        let int = if is_integer {
            raw.parse::<i64>().ok()
        } else {
            None
        };

        if self.would_start_ident() {
            let unit = self.consume_ident_sequence();
            return SpannedToken::new(Token::Dimension { value, unit }, Span::new(start, self.pos));
        }

        if self.peek_preprocessed(0) == Some('%') {
            self.bump();
            return SpannedToken::new(Token::Percentage(value), Span::new(start, self.pos));
        }

        SpannedToken::new(
            Token::Number { value, int, raw },
            Span::new(start, self.pos),
        )
    }
}

/// Read one preprocessed code point at byte `offset`, returning `(char, next_byte_offset)`.
#[inline]
fn next_preprocessed_at(src: &str, offset: usize) -> Option<(char, usize)> {
    let ch = src.get(offset..)?.chars().next()?;
    let next = offset + ch.len_utf8();
    match ch {
        '\r' => {
            if src.as_bytes().get(next) == Some(&b'\n') {
                Some(('\n', next + 1))
            } else {
                Some(('\n', next))
            }
        }
        '\u{000C}' => Some(('\n', next)),
        '\0' => Some(('\u{FFFD}', next)),
        _ => Some((ch, next)),
    }
}

#[inline]
fn is_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n')
}

#[inline]
fn is_ident_start_char(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || (ch as u32) >= 0x80
}

#[inline]
fn is_ident_char(ch: char) -> bool {
    is_ident_start_char(ch) || ch.is_ascii_digit() || ch == '-'
}

#[inline]
fn is_valid_escape(c0: Option<char>, c1: Option<char>) -> bool {
    c0 == Some('\\') && c1 != Some('\n')
}

#[inline]
fn is_non_printable(ch: char) -> bool {
    let cp = ch as u32;
    cp <= 0x08 || cp == 0x0B || (0x0E..=0x1F).contains(&cp) || cp == 0x7F
}
