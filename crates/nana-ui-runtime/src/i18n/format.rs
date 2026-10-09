//! Locale-aware formatting (Issue #270): plural rules, numbers, currency,
//! dates and times, behind one backend-neutral trait.
//!
//! [`LocaleFormatter`] is what messages format through. With the `icu`
//! feature (on by default) the runtime installs [`IcuFormatter`], ICU4X with
//! its compiled CLDR data; an application can install its own. ICU types
//! stay behind the trait. Formatters are built once per locale and options
//! and kept: formatting the same kind of value again builds nothing.

use std::fmt::Write;

use nana_text::font::LanguageTag;
use nana_ui_core::CivilDate;

/// A CLDR plural category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

/// How a number is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberStyle {
    /// As many fraction digits as it has, grouped in the locale's way.
    Decimal,
    /// Rounded to a whole number.
    Integer,
    /// A fraction as a percentage: `0.25` is 25%.
    Percent,
}

/// How long a date or time is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateStyle {
    Short,
    Medium,
    Long,
    Full,
}

/// A civil date, and a time of day if it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageDateTime {
    pub date: CivilDate,
    /// Hour, minute and second.
    pub time: Option<(u8, u8, u8)>,
}

impl MessageDateTime {
    pub fn date(date: CivilDate) -> Self {
        Self { date, time: None }
    }

    pub fn at(date: CivilDate, hour: u8, minute: u8, second: u8) -> Self {
        Self {
            date,
            time: Some((hour, minute, second)),
        }
    }
}

/// What formatters cost so far: formatters a lookup found, ones it had to
/// build. Cumulative; the runtime reads the difference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormatterCounts {
    pub hits: usize,
    pub misses: usize,
    pub built: usize,
}

/// Locale-aware formatting: plural categories, numbers, currency, dates and
/// times. Implementations write into `out` and keep what they build.
pub trait LocaleFormatter: Send + Sync {
    /// The plural category of `value` in `locale`: cardinal, or ordinal.
    fn plural(&self, locale: &LanguageTag, value: f64, ordinal: bool) -> PluralCategory;
    fn number(&self, locale: &LanguageTag, value: f64, style: NumberStyle, out: &mut String);
    /// `amount` of the ISO 4217 `currency`.
    fn currency(&self, locale: &LanguageTag, amount: f64, currency: &str, out: &mut String);
    /// `value`'s date at `date` length and its time at `time` length; a part
    /// with no length is left out.
    fn date_time(
        &self,
        locale: &LanguageTag,
        value: MessageDateTime,
        date: Option<DateStyle>,
        time: Option<DateStyle>,
        out: &mut String,
    );
    /// What lookups of built formatters cost so far.
    fn counts(&self) -> FormatterCounts {
        FormatterCounts::default()
    }
}

/// Formatting with no locale data: English plural rules, plain digits with
/// a `.` decimal point, ISO dates. What a build without the `icu` feature
/// formats with.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainFormatter;

impl LocaleFormatter for PlainFormatter {
    fn plural(&self, _locale: &LanguageTag, value: f64, ordinal: bool) -> PluralCategory {
        if ordinal {
            let whole = value.abs() as u64;
            return match (whole % 10, whole % 100) {
                (1, tens) if tens != 11 => PluralCategory::One,
                (2, tens) if tens != 12 => PluralCategory::Two,
                (3, tens) if tens != 13 => PluralCategory::Few,
                _ => PluralCategory::Other,
            };
        }
        if value == 1.0 {
            PluralCategory::One
        } else {
            PluralCategory::Other
        }
    }

    fn number(&self, _locale: &LanguageTag, value: f64, style: NumberStyle, out: &mut String) {
        let _ = match style {
            NumberStyle::Decimal => write!(out, "{value}"),
            NumberStyle::Integer => write!(out, "{}", value.round()),
            NumberStyle::Percent => write!(out, "{}%", (value * 100.0).round()),
        };
    }

    fn currency(&self, _locale: &LanguageTag, amount: f64, currency: &str, out: &mut String) {
        let _ = write!(out, "{currency} {amount:.2}");
    }

    fn date_time(
        &self,
        _locale: &LanguageTag,
        value: MessageDateTime,
        date: Option<DateStyle>,
        time: Option<DateStyle>,
        out: &mut String,
    ) {
        if date.is_some() {
            let _ = write!(
                out,
                "{:04}-{:02}-{:02}",
                value.date.year(),
                value.date.month(),
                value.date.day()
            );
        }
        if let (Some(_), Some((hour, minute, second))) = (time, value.time) {
            if date.is_some() {
                out.push(' ');
            }
            let _ = write!(out, "{hour:02}:{minute:02}:{second:02}");
        }
    }
}

#[cfg(feature = "icu")]
pub use icu::IcuFormatter;

#[cfg(feature = "icu")]
mod icu {
    use std::collections::HashMap;
    use std::sync::{Mutex, PoisonError};

    use fixed_decimal::{Decimal, FloatPrecision};
    use icu_datetime::DateTimeFormatter;
    use icu_datetime::fieldsets;
    use icu_datetime::options::TimePrecision;
    use icu_decimal::DecimalFormatter;
    use icu_locale_core::Locale;
    use icu_plurals::{PluralOperands, PluralRules};
    use writeable::Writeable;

    use super::*;

    /// ICU4X formatting with its compiled CLDR data. Each formatter is built
    /// the first time a locale and options ask for it, and kept.
    #[derive(Default)]
    pub struct IcuFormatter {
        cache: Mutex<Cache>,
    }

    impl std::fmt::Debug for IcuFormatter {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("IcuFormatter")
        }
    }

    #[derive(Default)]
    struct Cache {
        /// Plural rules by locale and whether they are ordinal.
        plurals: HashMap<(LanguageTag, bool), Option<PluralRules>>,
        decimal: HashMap<LanguageTag, Option<DecimalFormatter>>,
        dates: HashMap<(LanguageTag, DateStyle), Option<DateTimeFormatter<fieldsets::YMD>>>,
        full_dates: HashMap<LanguageTag, Option<DateTimeFormatter<fieldsets::YMDE>>>,
        times: HashMap<(LanguageTag, DateStyle), Option<DateTimeFormatter<fieldsets::T>>>,
        counts: FormatterCounts,
    }

    impl Cache {
        /// The entry for `key` in `map`, built by `build` on a miss.
        fn get<'a, K: std::hash::Hash + Eq, V>(
            counts: &mut FormatterCounts,
            map: &'a mut HashMap<K, Option<V>>,
            key: K,
            build: impl FnOnce() -> Option<V>,
        ) -> Option<&'a V> {
            use std::collections::hash_map::Entry;
            match map.entry(key) {
                Entry::Occupied(entry) => {
                    counts.hits += 1;
                    entry.into_mut().as_ref()
                }
                Entry::Vacant(entry) => {
                    counts.misses += 1;
                    let built = build();
                    counts.built += usize::from(built.is_some());
                    entry.insert(built).as_ref()
                }
            }
        }
    }

    fn locale(tag: &LanguageTag) -> Locale {
        tag.as_str().parse().unwrap_or(Locale::UNKNOWN)
    }

    fn decimal(value: f64, style: NumberStyle) -> Option<Decimal> {
        let value = match style {
            NumberStyle::Percent => value * 100.0,
            _ => value,
        };
        let mut decimal = Decimal::try_from_f64(value, FloatPrecision::RoundTrip).ok()?;
        if !matches!(style, NumberStyle::Decimal) {
            decimal.round(0);
        }
        Some(decimal)
    }

    impl IcuFormatter {
        pub fn new() -> Self {
            Self::default()
        }

        fn with<R>(&self, read: impl FnOnce(&mut Cache) -> R) -> R {
            read(&mut self.cache.lock().unwrap_or_else(PoisonError::into_inner))
        }

        fn write_decimal(cache: &mut Cache, tag: &LanguageTag, value: &Decimal, out: &mut String) {
            let formatter = Cache::get(&mut cache.counts, &mut cache.decimal, tag.clone(), || {
                DecimalFormatter::try_new(locale(tag).into(), Default::default()).ok()
            });
            match formatter {
                Some(formatter) => {
                    let _ = formatter.format(value).write_to(out);
                }
                None => {
                    let _ = value.write_to(out);
                }
            }
        }
    }

    impl LocaleFormatter for IcuFormatter {
        fn plural(&self, tag: &LanguageTag, value: f64, ordinal: bool) -> PluralCategory {
            let Ok(decimal) = Decimal::try_from_f64(value, FloatPrecision::RoundTrip) else {
                return PluralCategory::Other;
            };
            self.with(|cache| {
                let key = (tag.clone(), ordinal);
                let rules = Cache::get(&mut cache.counts, &mut cache.plurals, key, || {
                    let preferences = locale(tag).into();
                    if ordinal {
                        PluralRules::try_new_ordinal(preferences).ok()
                    } else {
                        PluralRules::try_new_cardinal(preferences).ok()
                    }
                });
                match rules.map(|rules| rules.category_for(PluralOperands::from(&decimal))) {
                    Some(icu_plurals::PluralCategory::Zero) => PluralCategory::Zero,
                    Some(icu_plurals::PluralCategory::One) => PluralCategory::One,
                    Some(icu_plurals::PluralCategory::Two) => PluralCategory::Two,
                    Some(icu_plurals::PluralCategory::Few) => PluralCategory::Few,
                    Some(icu_plurals::PluralCategory::Many) => PluralCategory::Many,
                    _ => PluralCategory::Other,
                }
            })
        }

        fn number(&self, tag: &LanguageTag, value: f64, style: NumberStyle, out: &mut String) {
            let Some(decimal) = decimal(value, style) else {
                let _ = write!(out, "{value}");
                return;
            };
            self.with(|cache| Self::write_decimal(cache, tag, &decimal, out));
            if style == NumberStyle::Percent {
                out.push('%');
            }
        }

        fn currency(&self, tag: &LanguageTag, amount: f64, currency: &str, out: &mut String) {
            let Ok(mut decimal) = Decimal::try_from_f64(amount, FloatPrecision::RoundTrip) else {
                return PlainFormatter.currency(tag, amount, currency, out);
            };
            decimal.round(-2);
            decimal.pad_end(-2);
            out.push_str(currency);
            out.push('\u{a0}');
            self.with(|cache| Self::write_decimal(cache, tag, &decimal, out));
        }

        fn date_time(
            &self,
            tag: &LanguageTag,
            value: MessageDateTime,
            date: Option<DateStyle>,
            time: Option<DateStyle>,
            out: &mut String,
        ) {
            let Ok(iso) = icu_calendar::Date::try_new_iso(
                value.date.year(),
                value.date.month(),
                value.date.day(),
            ) else {
                return PlainFormatter.date_time(tag, value, date, time, out);
            };
            self.with(|cache| {
                if let Some(style) = date {
                    let written = if style == DateStyle::Full {
                        Cache::get(
                            &mut cache.counts,
                            &mut cache.full_dates,
                            tag.clone(),
                            || {
                                DateTimeFormatter::try_new(
                                    locale(tag).into(),
                                    fieldsets::YMDE::long(),
                                )
                                .ok()
                            },
                        )
                        .map(|formatter| formatter.format(&iso).write_to(out))
                    } else {
                        Cache::get(
                            &mut cache.counts,
                            &mut cache.dates,
                            (tag.clone(), style),
                            || {
                                let fields = match style {
                                    DateStyle::Short => fieldsets::YMD::short(),
                                    DateStyle::Medium => fieldsets::YMD::medium(),
                                    _ => fieldsets::YMD::long(),
                                };
                                DateTimeFormatter::try_new(locale(tag).into(), fields).ok()
                            },
                        )
                        .map(|formatter| formatter.format(&iso).write_to(out))
                    };
                    if written.is_none() {
                        PlainFormatter.date_time(tag, value, Some(style), None, out);
                    }
                }
                if let (Some(style), Some((hour, minute, second))) = (time, value.time) {
                    if date.is_some() {
                        out.push(' ');
                    }
                    let Ok(clock) = icu_time::Time::try_new(hour, minute, second, 0) else {
                        return;
                    };
                    let written = Cache::get(
                        &mut cache.counts,
                        &mut cache.times,
                        (tag.clone(), style),
                        || {
                            let fields = match style {
                                DateStyle::Short => {
                                    fieldsets::T::short().with_time_precision(TimePrecision::Minute)
                                }
                                DateStyle::Medium => fieldsets::T::medium(),
                                _ => fieldsets::T::long(),
                            };
                            DateTimeFormatter::try_new(locale(tag).into(), fields).ok()
                        },
                    )
                    .map(|formatter| formatter.format(&clock).write_to(out));
                    if written.is_none() {
                        let _ = write!(out, "{hour:02}:{minute:02}:{second:02}");
                    }
                }
            });
        }

        fn counts(&self) -> FormatterCounts {
            self.with(|cache| cache.counts)
        }
    }
}

/// The formatter a world starts with: ICU4X with the `icu` feature, else
/// [`PlainFormatter`].
pub(crate) fn default_formatter() -> std::sync::Arc<dyn LocaleFormatter> {
    #[cfg(feature = "icu")]
    {
        std::sync::Arc::new(IcuFormatter::new())
    }
    #[cfg(not(feature = "icu"))]
    {
        std::sync::Arc::new(PlainFormatter)
    }
}

#[cfg(all(test, feature = "icu"))]
mod tests {
    use super::*;

    fn tag(tag: &str) -> LanguageTag {
        LanguageTag::new(tag).unwrap()
    }

    fn number(formatter: &IcuFormatter, locale: &str, value: f64, style: NumberStyle) -> String {
        let mut out = String::new();
        formatter.number(&tag(locale), value, style, &mut out);
        out
    }

    #[test]
    fn plural_rules_follow_cldr() {
        let formatter = IcuFormatter::new();
        let arabic: Vec<PluralCategory> = [0.0, 1.0, 2.0, 3.0, 11.0, 100.0]
            .into_iter()
            .map(|value| formatter.plural(&tag("ar"), value, false))
            .collect();
        assert_eq!(
            arabic,
            [
                PluralCategory::Zero,
                PluralCategory::One,
                PluralCategory::Two,
                PluralCategory::Few,
                PluralCategory::Many,
                PluralCategory::Other,
            ]
        );
        let ordinals: Vec<PluralCategory> = [1.0, 2.0, 3.0, 4.0, 11.0, 22.0]
            .into_iter()
            .map(|value| formatter.plural(&tag("en"), value, true))
            .collect();
        assert_eq!(
            ordinals,
            [
                PluralCategory::One,
                PluralCategory::Two,
                PluralCategory::Few,
                PluralCategory::Other,
                PluralCategory::Other,
                PluralCategory::Two,
            ]
        );
        assert_eq!(
            formatter.plural(&tag("zh-cn"), 1.0, false),
            PluralCategory::Other
        );
    }

    #[test]
    fn numbers_currency_and_dates_take_the_locale() {
        let formatter = IcuFormatter::new();
        assert_eq!(
            number(&formatter, "de", 1_234_567.891, NumberStyle::Decimal),
            "1.234.567,891"
        );
        assert_eq!(
            number(&formatter, "en-us", 1_234.5, NumberStyle::Decimal),
            "1,234.5"
        );
        assert_eq!(number(&formatter, "en-us", 2.6, NumberStyle::Integer), "3");
        assert_eq!(
            number(&formatter, "en-us", 0.25, NumberStyle::Percent),
            "25%"
        );
        let mut price = String::new();
        formatter.currency(&tag("en-us"), 1_234.5, "USD", &mut price);
        assert_eq!(price, "USD\u{a0}1,234.50");
        let day = CivilDate::new(2026, 10, 9).unwrap();
        let mut date = String::new();
        formatter.date_time(
            &tag("zh-cn"),
            MessageDateTime::date(day),
            Some(DateStyle::Long),
            None,
            &mut date,
        );
        assert_eq!(date, "2026年10月9日");
        let mut time = String::new();
        formatter.date_time(
            &tag("en-us"),
            MessageDateTime::at(day, 14, 5, 9),
            None,
            Some(DateStyle::Short),
            &mut time,
        );
        assert_eq!(time, "2:05\u{202f}PM");
    }

    #[test]
    fn a_formatter_is_built_once_per_locale_and_kept() {
        let formatter = IcuFormatter::new();
        for value in 0..1_000 {
            number(&formatter, "fr", f64::from(value), NumberStyle::Decimal);
        }
        let counts = formatter.counts();
        assert_eq!(counts.built, 1);
        assert_eq!(counts.misses, 1);
        assert_eq!(counts.hits, 999);
    }
}
