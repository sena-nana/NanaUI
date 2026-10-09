//! Localization (Issues #267, #268): which text is localized, in which
//! locale, and from which catalog.
//!
//! A node's text is literal -- written as it is, in no locale -- or
//! localized: a message and its arguments, resolved in the locale of the
//! scope the node is in. A scope is the application, a window, or a subtree.
//! A [`Locale`] names four things that change apart:
//!
//! - the messages it selects;
//! - the language localized text shapes in (font fallback, `locl`);
//! - the direction its scope lays out in;
//! - the locale numbers and dates format in.
//!
//! Literal text depends on none of them. Changing a scope's locale resolves
//! the localized text in it and nothing else, and text whose string,
//! language and direction held does no text or layout work. Layout hears of
//! a switch only through the text metrics and the direction that moved.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use nana_text::font::LanguageTag;
use nana_ui_core::DirSpec;

mod format;
pub(crate) mod message;

#[cfg(feature = "icu")]
pub use format::IcuFormatter;
pub(crate) use format::default_formatter;
pub use format::{
    DateStyle, FormatterCounts, LocaleFormatter, MessageDateTime, NumberStyle, PlainFormatter,
    PluralCategory,
};

/// A message's identity: an interned key. Authoring and diagnostics read the
/// key; the runtime compares the number.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageId(u32);

#[derive(Default)]
struct Interner {
    ids: HashMap<Arc<str>, u32>,
    keys: Vec<Arc<str>>,
}

fn interner() -> &'static Mutex<Interner> {
    static INTERNER: OnceLock<Mutex<Interner>> = OnceLock::new();
    INTERNER.get_or_init(Mutex::default)
}

impl MessageId {
    /// The message named `key`; the same key is the same message.
    pub fn new(key: &str) -> Self {
        let mut interner = interner().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(id) = interner.ids.get(key) {
            return Self(*id);
        }
        let id = u32::try_from(interner.keys.len()).expect("fewer than 2^32 message keys");
        let key: Arc<str> = Arc::from(key);
        interner.keys.push(Arc::clone(&key));
        interner.ids.insert(key, id);
        Self(id)
    }

    /// The key the message was named by.
    pub fn key(self) -> Arc<str> {
        let interner = interner().lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(&interner.keys[self.0 as usize])
    }
}

impl fmt::Debug for MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "MessageId({:?})", self.key())
    }
}

/// One argument of a message.
#[derive(Debug, Clone, PartialEq)]
pub enum MessageArg {
    Text(Arc<str>),
    Number(f64),
    /// An amount of an ISO 4217 currency: `{price, number, currency}`.
    Currency {
        amount: f64,
        currency: Arc<str>,
    },
    /// A date, and a time of day: `{when, date}`, `{when, time}`.
    DateTime(MessageDateTime),
}

impl MessageArg {
    /// `amount` of the ISO 4217 `currency`, such as `"EUR"`.
    pub fn currency(amount: f64, currency: &str) -> Self {
        Self::Currency {
            amount,
            currency: Arc::from(currency),
        }
    }
}

impl From<MessageDateTime> for MessageArg {
    fn from(value: MessageDateTime) -> Self {
        Self::DateTime(value)
    }
}

impl From<nana_ui_core::CivilDate> for MessageArg {
    fn from(value: nana_ui_core::CivilDate) -> Self {
        Self::DateTime(MessageDateTime::date(value))
    }
}

impl From<&str> for MessageArg {
    fn from(value: &str) -> Self {
        Self::Text(Arc::from(value))
    }
}

impl From<String> for MessageArg {
    fn from(value: String) -> Self {
        Self::Text(Arc::from(value))
    }
}

impl From<f64> for MessageArg {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

/// An integer argument is a number.
macro_rules! integer_args {
    ($($integer:ty),*) => {$(
        impl From<$integer> for MessageArg {
            fn from(value: $integer) -> Self {
                Self::Number(value as f64)
            }
        }
    )*};
}

integer_args!(i32, i64, u32, u64, usize);

/// A message's named arguments, kept sorted by name: two argument sets are
/// equal when they say the same, however they were built.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageArgs(Arc<[(Arc<str>, MessageArg)]>);

impl MessageArgs {
    pub fn new() -> Self {
        Self::default()
    }

    /// These arguments with `name` set to `value`.
    pub fn with(self, name: &str, value: impl Into<MessageArg>) -> Self {
        let mut args: Vec<(Arc<str>, MessageArg)> = self.0.iter().cloned().collect();
        let value = value.into();
        match args.binary_search_by(|(arg, _)| arg.as_ref().cmp(name)) {
            Ok(at) => args[at].1 = value,
            Err(at) => args.insert(at, (Arc::from(name), value)),
        }
        Self(args.into())
    }

    pub fn get(&self, name: &str) -> Option<&MessageArg> {
        self.0
            .binary_search_by(|(arg, _)| arg.as_ref().cmp(name))
            .ok()
            .map(|at| &self.0[at].1)
    }
}

/// What a localized node says: a message and its arguments, resolved in the
/// locale of the scope the node is in.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalizedText {
    pub message: MessageId,
    pub args: MessageArgs,
}

impl LocalizedText {
    /// The message named `key`, with no arguments.
    pub fn new(key: &str) -> Self {
        Self {
            message: MessageId::new(key),
            args: MessageArgs::new(),
        }
    }

    /// This text with argument `name` set to `value`.
    pub fn arg(mut self, name: &str, value: impl Into<MessageArg>) -> Self {
        self.args = self.args.with(name, value);
        self
    }
}

/// Where messages come from. An adapter over Fluent, ICU or a table answers
/// a message's pattern in exactly one locale; the runtime walks the fallback
/// chain and formats. No backend type crosses this boundary.
pub trait MessageCatalog: Send + Sync {
    /// The pattern of `message` in exactly `locale`, if the catalog has one.
    fn pattern(&self, locale: &LanguageTag, message: MessageId) -> Option<Arc<str>>;
}

/// A catalog held as a table: patterns by locale and message.
#[derive(Debug, Clone, Default)]
pub struct MessageTable {
    patterns: HashMap<(LanguageTag, MessageId), Arc<str>>,
}

impl MessageTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// This table with `key` in `locale` saying `pattern`. A locale that is
    /// not a language tag is skipped.
    pub fn with(mut self, locale: &str, key: &str, pattern: &str) -> Self {
        if let Some(locale) = LanguageTag::new(locale) {
            self.insert(locale, MessageId::new(key), pattern);
        }
        self
    }

    pub fn insert(
        &mut self,
        locale: LanguageTag,
        message: MessageId,
        pattern: impl Into<Arc<str>>,
    ) {
        self.patterns.insert((locale, message), pattern.into());
    }
}

impl MessageCatalog for MessageTable {
    fn pattern(&self, locale: &LanguageTag, message: MessageId) -> Option<Arc<str>> {
        self.patterns.get(&(locale.clone(), message)).cloned()
    }
}

/// A scope's locale: the messages it selects, and the language, direction
/// and formatting that follow them unless named apart.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale {
    messages: LanguageTag,
    language: Option<LanguageTag>,
    direction: Option<DirSpec>,
    formatting: Option<LanguageTag>,
}

impl Locale {
    pub fn new(messages: LanguageTag) -> Self {
        Self {
            messages,
            language: None,
            direction: None,
            formatting: None,
        }
    }

    /// The locale of tag `tag`, if it is one.
    pub fn parse(tag: &str) -> Option<Self> {
        LanguageTag::new(tag).map(Self::new)
    }

    /// Shape localized text in `language` instead of the messages' own.
    pub fn with_language(mut self, language: LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Lay the scope out in `direction` instead of its script's.
    pub fn with_direction(mut self, direction: DirSpec) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Format numbers and dates in `formatting` instead of the messages'.
    pub fn with_formatting(mut self, formatting: LanguageTag) -> Self {
        self.formatting = Some(formatting);
        self
    }

    pub fn messages(&self) -> &LanguageTag {
        &self.messages
    }

    /// The language named apart from the messages, if one is.
    pub fn language(&self) -> Option<&LanguageTag> {
        self.language.as_ref()
    }

    /// The direction the scope lays out in: the one named, else its
    /// messages' script's.
    pub fn direction(&self) -> DirSpec {
        self.direction
            .unwrap_or_else(|| script_direction(&self.messages))
    }

    pub fn formatting(&self) -> &LanguageTag {
        self.formatting.as_ref().unwrap_or(&self.messages)
    }
}

/// `tag`, then each shorter tag of its whole leading subtags: `zh-hant-tw`,
/// `zh-hant`, `zh`.
pub(crate) fn fallback_chain(tag: &LanguageTag) -> impl Iterator<Item = LanguageTag> + '_ {
    let text = tag.as_str();
    let mut end = Some(text.len());
    std::iter::from_fn(move || {
        let at = end?;
        let candidate = &text[..at];
        end = candidate.rfind('-');
        LanguageTag::new(candidate)
    })
}

/// The direction a language writes in, by its script subtag, else by the
/// language: right to left for the Arabic, Hebrew, Thaana, Syriac, N'Ko and
/// Adlam scripts and the languages written in them.
pub(crate) fn script_direction(tag: &LanguageTag) -> DirSpec {
    const RTL_SCRIPTS: &[&str] = &[
        "arab", "hebr", "thaa", "syrc", "nkoo", "adlm", "rohg", "mand", "samr", "mend",
    ];
    const RTL_LANGUAGES: &[&str] = &[
        "ar", "he", "iw", "fa", "ur", "ps", "sd", "yi", "dv", "ug", "ckb", "syr", "nqo", "arc",
        "ks",
    ];
    let mut subtags = tag.as_str().split('-');
    let language = subtags.next().unwrap_or_default();
    let rtl = match subtags.find(|subtag| subtag.len() == 4) {
        Some(script) => RTL_SCRIPTS.contains(&script),
        None => RTL_LANGUAGES.contains(&language),
    };
    if rtl { DirSpec::Rtl } else { DirSpec::Ltr }
}

/// What a message no locale in the chain has shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MissingMessage {
    /// The message's key, so the gap is visible.
    #[default]
    Key,
    /// What the node showed before.
    KeepPrevious,
}
