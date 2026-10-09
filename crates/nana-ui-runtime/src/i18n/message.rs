//! Message patterns (Issue #270): parsed once into a compiled message, then
//! formatted against arguments as often as they change.
//!
//! The syntax is the ICU MessageFormat subset applications write by hand:
//!
//! - `{name}`: an argument as it is (text as written, a number in the
//!   locale's digits, a date and time at medium length);
//! - `{name, number}`, `{name, number, integer}`, `{name, number, percent}`,
//!   `{name, number, currency}`;
//! - `{name, date}`, `{name, time}`, `{name, datetime}`, each optionally
//!   `, short`, `, medium`, `, long` or `, full`;
//! - `{name, plural, offset:1 =0 {..} one {..} other {..}}` and
//!   `{name, selectordinal, ..}`, where `#` is the number less the offset;
//! - `{name, select, a {..} b {..} other {..}}`;
//! - `''` is an apostrophe; an apostrophe before `{`, `}` or `#` quotes
//!   until the next lone apostrophe.

use std::fmt::Write;

use nana_text::font::LanguageTag;

use super::format::{DateStyle, LocaleFormatter, NumberStyle, PluralCategory};
use super::{MessageArg, MessageArgs};

/// A pattern parsed once: what formatting it needs, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompiledMessage {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(Box<str>),
    Value(Box<str>),
    Number(Box<str>, NumberStyle),
    Currency(Box<str>),
    DateTime(Box<str>, Option<DateStyle>, Option<DateStyle>),
    Plural {
        arg: Box<str>,
        offset: f64,
        ordinal: bool,
        exact: Vec<(f64, Vec<Part>)>,
        cases: Vec<(PluralCategory, Vec<Part>)>,
        other: Vec<Part>,
    },
    Select {
        arg: Box<str>,
        cases: Vec<(Box<str>, Vec<Part>)>,
        other: Vec<Part>,
    },
    /// `#` in a plural case: the number less the offset.
    Pound,
}

/// Why a pattern is not a message: the byte it stopped at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PatternError {
    pub(crate) at: usize,
}

/// What formatting could not do; the output says what it could.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormatIssue {
    /// An argument the pattern names that the text does not give, or gives
    /// as the wrong kind.
    Argument(Box<str>),
}

impl CompiledMessage {
    pub(crate) fn parse(pattern: &str) -> Result<Self, PatternError> {
        let mut parser = Parser {
            source: pattern,
            at: 0,
        };
        let parts = parser.message(false, 0)?;
        if parser.at < pattern.len() {
            return Err(PatternError { at: parser.at });
        }
        Ok(Self { parts })
    }

    /// The message as plain text, for a pattern that is not one: shown as
    /// written.
    pub(crate) fn literal(pattern: &str) -> Self {
        Self {
            parts: vec![Part::Text(pattern.into())],
        }
    }

    /// Whether formatting reads the locale's numbers, dates or plural
    /// rules: whether a formatting locale change reaches it.
    pub(crate) fn formats(&self) -> bool {
        fn formats(parts: &[Part]) -> bool {
            parts.iter().any(|part| match part {
                Part::Text(_) => false,
                Part::Value(_) | Part::Number(..) | Part::Currency(_) | Part::DateTime(..) => true,
                Part::Pound | Part::Plural { .. } => true,
                Part::Select { cases, other, .. } => {
                    cases.iter().any(|(_, parts)| formats(parts)) || formats(other)
                }
            })
        }
        formats(&self.parts)
    }

    /// Write the message for `args` in `locale` to `out`.
    pub(crate) fn format(
        &self,
        locale: &LanguageTag,
        args: &MessageArgs,
        formatter: &dyn LocaleFormatter,
        out: &mut String,
        issues: &mut Vec<FormatIssue>,
    ) {
        let mut context = Context {
            locale,
            args,
            formatter,
            issues,
        };
        context.parts(&self.parts, None, out);
    }
}

struct Context<'a> {
    locale: &'a LanguageTag,
    args: &'a MessageArgs,
    formatter: &'a dyn LocaleFormatter,
    issues: &'a mut Vec<FormatIssue>,
}

impl Context<'_> {
    fn parts(&mut self, parts: &[Part], pound: Option<f64>, out: &mut String) {
        for part in parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Pound => match pound {
                    Some(value) => {
                        self.formatter
                            .number(self.locale, value, NumberStyle::Decimal, out)
                    }
                    None => out.push('#'),
                },
                Part::Value(name) => match self.args.get(name) {
                    Some(MessageArg::Text(text)) => out.push_str(text),
                    Some(MessageArg::Number(value)) => {
                        self.formatter
                            .number(self.locale, *value, NumberStyle::Decimal, out)
                    }
                    Some(MessageArg::Currency { amount, currency }) => {
                        self.formatter.currency(self.locale, *amount, currency, out)
                    }
                    Some(MessageArg::DateTime(value)) => self.formatter.date_time(
                        self.locale,
                        *value,
                        Some(DateStyle::Medium),
                        value.time.map(|_| DateStyle::Short),
                        out,
                    ),
                    None => self.missing(name, out),
                },
                Part::Number(name, style) => match self.number(name) {
                    Some(value) => self.formatter.number(self.locale, value, *style, out),
                    None => self.missing(name, out),
                },
                Part::Currency(name) => match self.args.get(name) {
                    Some(MessageArg::Currency { amount, currency }) => {
                        self.formatter.currency(self.locale, *amount, currency, out)
                    }
                    _ => self.missing(name, out),
                },
                Part::DateTime(name, date, time) => match self.args.get(name) {
                    Some(MessageArg::DateTime(value)) => {
                        self.formatter
                            .date_time(self.locale, *value, *date, *time, out)
                    }
                    _ => self.missing(name, out),
                },
                Part::Plural {
                    arg,
                    offset,
                    ordinal,
                    exact,
                    cases,
                    other,
                } => {
                    let Some(value) = self.number(arg) else {
                        self.missing(arg, out);
                        continue;
                    };
                    let shown = value - offset;
                    let case = exact
                        .iter()
                        .find(|(exact, _)| *exact == value)
                        .map(|(_, parts)| parts)
                        .unwrap_or_else(|| {
                            let category = self.formatter.plural(self.locale, shown, *ordinal);
                            cases
                                .iter()
                                .find(|(case, _)| *case == category)
                                .map_or(other, |(_, parts)| parts)
                        });
                    self.parts(case, Some(shown), out);
                }
                Part::Select { arg, cases, other } => {
                    // Text selects as it is; only a number is written out.
                    let number;
                    let key = match self.args.get(arg) {
                        Some(MessageArg::Text(text)) => Some(text.as_ref()),
                        Some(MessageArg::Number(value)) => {
                            number = value.to_string();
                            Some(number.as_str())
                        }
                        _ => None,
                    };
                    let case = key
                        .and_then(|key| cases.iter().find(|(case, _)| case.as_ref() == key))
                        .map_or(other, |(_, parts)| parts);
                    self.parts(case, pound, out);
                }
            }
        }
    }

    fn number(&self, name: &str) -> Option<f64> {
        match self.args.get(name)? {
            MessageArg::Number(value) => Some(*value),
            MessageArg::Currency { amount, .. } => Some(*amount),
            _ => None,
        }
    }

    /// An argument the text does not give: written as the pattern names it.
    fn missing(&mut self, name: &str, out: &mut String) {
        let _ = write!(out, "{{{name}}}");
        self.issues.push(FormatIssue::Argument(name.into()));
    }
}

struct Parser<'a> {
    source: &'a str,
    at: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        &self.source[self.at..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let next = self.peek()?;
        self.at += next.len_utf8();
        Some(next)
    }

    fn error<T>(&self) -> Result<T, PatternError> {
        Err(PatternError { at: self.at })
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    /// A message up to the end, or to the `}` that closes a case.
    fn message(&mut self, in_plural: bool, depth: usize) -> Result<Vec<Part>, PatternError> {
        if depth > 16 {
            return self.error();
        }
        let mut parts = Vec::new();
        let mut text = String::new();
        while let Some(next) = self.peek() {
            match next {
                '}' => break,
                '{' => {
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text).into()));
                    }
                    self.bump();
                    parts.push(self.argument(depth)?);
                }
                '#' if in_plural => {
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text).into()));
                    }
                    self.bump();
                    parts.push(Part::Pound);
                }
                '\'' => {
                    self.bump();
                    match self.peek() {
                        Some('\'') => {
                            self.bump();
                            text.push('\'');
                        }
                        Some('{' | '}') | Some('#') => {
                            // Quoted until the next lone apostrophe.
                            while let Some(quoted) = self.bump() {
                                if quoted == '\'' {
                                    if self.peek() == Some('\'') {
                                        self.bump();
                                        text.push('\'');
                                        continue;
                                    }
                                    break;
                                }
                                text.push(quoted);
                            }
                        }
                        _ => text.push('\''),
                    }
                }
                _ => {
                    self.bump();
                    text.push(next);
                }
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text.into()));
        }
        Ok(parts)
    }

    fn name(&mut self) -> Result<Box<str>, PatternError> {
        self.skip_space();
        let start = self.at;
        while self
            .peek()
            .is_some_and(|next| next.is_alphanumeric() || matches!(next, '_' | '-' | '.'))
        {
            self.bump();
        }
        if self.at == start {
            return self.error();
        }
        let name = self.source[start..self.at].into();
        self.skip_space();
        Ok(name)
    }

    fn expect(&mut self, expected: char) -> Result<(), PatternError> {
        self.skip_space();
        if self.peek() == Some(expected) {
            self.bump();
            Ok(())
        } else {
            self.error()
        }
    }

    /// After a `{`: an argument, through its `}`.
    fn argument(&mut self, depth: usize) -> Result<Part, PatternError> {
        let arg = self.name()?;
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Part::Value(arg));
        }
        self.expect(',')?;
        let kind = self.name()?;
        let part = match kind.as_ref() {
            "number" => {
                let style = self.style()?;
                match style.as_deref() {
                    None => Part::Number(arg, NumberStyle::Decimal),
                    Some("integer") => Part::Number(arg, NumberStyle::Integer),
                    Some("percent") => Part::Number(arg, NumberStyle::Percent),
                    Some("currency") => Part::Currency(arg),
                    Some(_) => return self.error(),
                }
            }
            "date" | "time" | "datetime" => {
                let style = match self.style()?.as_deref() {
                    None | Some("medium") => DateStyle::Medium,
                    Some("short") => DateStyle::Short,
                    Some("long") => DateStyle::Long,
                    Some("full") => DateStyle::Full,
                    Some(_) => return self.error(),
                };
                match kind.as_ref() {
                    "date" => Part::DateTime(arg, Some(style), None),
                    "time" => Part::DateTime(arg, None, Some(style)),
                    _ => Part::DateTime(arg, Some(style), Some(style)),
                }
            }
            "plural" | "selectordinal" => {
                self.expect(',')?;
                return self.plural(arg, kind.as_ref() == "selectordinal", depth);
            }
            "select" => {
                self.expect(',')?;
                return self.select(arg, depth);
            }
            _ => return self.error(),
        };
        self.expect('}')?;
        Ok(part)
    }

    /// `, style` before the closing brace, if there is one.
    fn style(&mut self) -> Result<Option<String>, PatternError> {
        self.skip_space();
        if self.peek() != Some(',') {
            return Ok(None);
        }
        self.bump();
        Ok(Some(self.name()?.into_string()))
    }

    fn case(&mut self, depth: usize, in_plural: bool) -> Result<Vec<Part>, PatternError> {
        self.expect('{')?;
        let parts = self.message(in_plural, depth + 1)?;
        if self.bump() != Some('}') {
            return self.error();
        }
        self.skip_space();
        Ok(parts)
    }

    fn plural(&mut self, arg: Box<str>, ordinal: bool, depth: usize) -> Result<Part, PatternError> {
        let mut offset = 0.0;
        let mut exact = Vec::new();
        let mut cases = Vec::new();
        let mut other = None;
        self.skip_space();
        if self.rest().starts_with("offset:") {
            self.at += "offset:".len();
            offset = self.number()?;
        }
        loop {
            self.skip_space();
            match self.peek() {
                Some('}') => {
                    self.bump();
                    break;
                }
                Some('=') => {
                    self.bump();
                    let value = self.number()?;
                    exact.push((value, self.case(depth, true)?));
                }
                Some(_) => {
                    let selector = self.name()?;
                    let parts = self.case(depth, true)?;
                    match selector.as_ref() {
                        "other" => other = Some(parts),
                        "zero" => cases.push((PluralCategory::Zero, parts)),
                        "one" => cases.push((PluralCategory::One, parts)),
                        "two" => cases.push((PluralCategory::Two, parts)),
                        "few" => cases.push((PluralCategory::Few, parts)),
                        "many" => cases.push((PluralCategory::Many, parts)),
                        _ => return self.error(),
                    }
                }
                None => return self.error(),
            }
        }
        let Some(other) = other else {
            return self.error();
        };
        Ok(Part::Plural {
            arg,
            offset,
            ordinal,
            exact,
            cases,
            other,
        })
    }

    fn select(&mut self, arg: Box<str>, depth: usize) -> Result<Part, PatternError> {
        let mut cases = Vec::new();
        let mut other = None;
        loop {
            self.skip_space();
            match self.peek() {
                Some('}') => {
                    self.bump();
                    break;
                }
                Some(_) => {
                    let key = self.name()?;
                    let parts = self.case(depth, false)?;
                    if key.as_ref() == "other" {
                        other = Some(parts);
                    } else {
                        cases.push((key, parts));
                    }
                }
                None => return self.error(),
            }
        }
        let Some(other) = other else {
            return self.error();
        };
        Ok(Part::Select { arg, cases, other })
    }

    fn number(&mut self) -> Result<f64, PatternError> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|next| next.is_ascii_digit() || matches!(next, '.' | '-'))
        {
            self.bump();
        }
        self.source[start..self.at]
            .parse()
            .or_else(|_| self.error())
    }
}

#[cfg(test)]
mod tests {
    use super::super::format::PlainFormatter;
    use super::*;

    fn format(pattern: &str, args: &MessageArgs) -> (String, Vec<FormatIssue>) {
        let message = CompiledMessage::parse(pattern).expect("a message");
        let mut out = String::new();
        let mut issues = Vec::new();
        message.format(
            &LanguageTag::new("en").unwrap(),
            args,
            &PlainFormatter,
            &mut out,
            &mut issues,
        );
        (out, issues)
    }

    #[test]
    fn arguments_plurals_and_selects_format() {
        let args = MessageArgs::new()
            .with("count", 1u32)
            .with("who", "Ada")
            .with("kind", "folder");
        assert_eq!(format("Hi {who}", &args).0, "Hi Ada");
        assert_eq!(
            format("{count, plural, one {# file} other {# files}}", &args).0,
            "1 file"
        );
        let many = args.clone().with("count", 3u32);
        assert_eq!(
            format(
                "{count, plural, =0 {none} one {# file} other {# files}}",
                &many
            )
            .0,
            "3 files"
        );
        let zero = args.clone().with("count", 0u32);
        assert_eq!(
            format(
                "{count, plural, =0 {none} one {# file} other {# files}}",
                &zero
            )
            .0,
            "none"
        );
        assert_eq!(
            format(
                "{kind, select, file {a file} folder {a folder} other {it}}",
                &args
            )
            .0,
            "a folder"
        );
        assert_eq!(
            format(
                "{count, plural, offset:1 one {just {who}} other {{who} and # more}}",
                &many
            )
            .0,
            "Ada and 2 more"
        );
    }

    #[test]
    fn a_currency_argument_formats_as_an_amount_of_its_currency() {
        let args = MessageArgs::new().with("price", MessageArg::currency(1234.5, "EUR"));
        assert_eq!(
            format("Total: {price, number, currency}", &args).0,
            "Total: EUR 1234.50"
        );
        assert_eq!(format("Total: {price}", &args).0, "Total: EUR 1234.50");
        // A plain number is not an amount of a currency.
        let (out, issues) = format(
            "Total: {price, number, currency}",
            &MessageArgs::new().with("price", 1234.5),
        );
        assert_eq!(out, "Total: {price}");
        assert_eq!(issues, vec![FormatIssue::Argument("price".into())]);
    }

    #[test]
    fn apostrophes_quote_and_escape() {
        let args = MessageArgs::new().with("n", 2u32);
        assert_eq!(format("It''s {n}", &args).0, "It's 2");
        assert_eq!(format("'{n}' is {n}", &args).0, "{n} is 2");
        assert_eq!(format("don't", &args).0, "don't");
    }

    #[test]
    fn a_missing_argument_shows_its_name_and_is_reported() {
        let (out, issues) = format("Hello {who}", &MessageArgs::new());
        assert_eq!(out, "Hello {who}");
        assert_eq!(issues, vec![FormatIssue::Argument("who".into())]);
    }

    #[test]
    fn malformed_patterns_are_errors() {
        for pattern in [
            "{",
            "{count, plural, one {x}}",
            "{count, nope}",
            "{a, select, x {y}}",
            "x }",
        ] {
            assert!(CompiledMessage::parse(pattern).is_err(), "{pattern}");
        }
    }
}
