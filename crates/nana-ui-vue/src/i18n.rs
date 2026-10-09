//! Localized text on the Vue path (Issue #267): what the JS host sends -- a
//! `<nana-text>`'s arguments, a catalog, a locale -- read into the Runtime's
//! i18n types. Messages resolve in the Runtime, in the locale of the scope a
//! node is in; nothing here formats or picks a translation.
//!
//! Arguments and catalogs cross as JSON text and string triples rather than
//! objects: the engine bridge drops object keys named `key`, `ref` or `on*`
//! (`onboarding.title` would vanish) and turns a `Date` into `{}`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use nana_js_engine::{HostValue, JsException};
use nana_ui_core::{CivilDate, DirSpec};
use nana_ui_runtime::{
    LanguageTag, Locale, MessageArg, MessageArgs, MessageDateTime, MessageId, MessageTable,
    MissingMessage,
};

/// The arguments a `message-args` attribute gives, and why any were left out.
///
/// The attribute is a JSON object of argument name to a number, a string, a
/// boolean (`select` reads it as `"true"` / `"false"`), `{"$date": [y, m, d]}`
/// or `[y, m, d, h, mi, s]` in civil fields, or `{"$currency": [amount,
/// "EUR"]}`. `null` leaves the argument out, so the message names it as
/// missing. A value of any other shape is left out the same way and reported.
pub(crate) fn parse_message_args(text: &str) -> (MessageArgs, Option<String>) {
    let mut args = MessageArgs::new();
    if text.trim().is_empty() {
        return (args, None);
    }
    let entries = match HostValue::from_json_str(text) {
        Ok(HostValue::Object(entries)) => entries,
        Ok(_) => return (args, Some("message-args is not a JSON object".to_owned())),
        Err(error) => return (args, Some(format!("message-args: {error}"))),
    };
    let mut error = None;
    for (name, value) in &entries {
        match message_arg(value) {
            Ok(Some(arg)) => args = args.with(name, arg),
            Ok(None) => {}
            Err(()) => {
                error.get_or_insert_with(|| {
                    format!(
                        "message argument `{name}` is not a number, a string, $date or $currency"
                    )
                });
            }
        }
    }
    (args, error)
}

fn message_arg(value: &HostValue) -> Result<Option<MessageArg>, ()> {
    Ok(Some(match value {
        HostValue::Null | HostValue::Undefined => return Ok(None),
        HostValue::Number(number) if number.is_finite() => MessageArg::Number(*number),
        HostValue::String(text) => MessageArg::from(text.as_str()),
        HostValue::Bool(flag) => MessageArg::from(if *flag { "true" } else { "false" }),
        HostValue::Object(fields) if fields.len() == 1 => {
            match fields
                .iter()
                .next()
                .map(|(key, value)| (key.as_str(), value))
            {
                Some(("$date", HostValue::Array(parts))) => date_arg(parts)?,
                Some(("$currency", HostValue::Array(parts))) => currency_arg(parts)?,
                _ => return Err(()),
            }
        }
        _ => return Err(()),
    }))
}

fn date_arg(parts: &[HostValue]) -> Result<MessageArg, ()> {
    let field = |at: usize| {
        parts
            .get(at)
            .and_then(HostValue::as_f64)
            .filter(|value| value.is_finite() && value.fract() == 0.0)
            .ok_or(())
    };
    let date = CivilDate::new(field(0)? as i32, field(1)? as u8, field(2)? as u8).ok_or(())?;
    Ok(MessageArg::from(match parts.len() {
        3 => MessageDateTime::date(date),
        6 => {
            let (hour, minute, second) = (field(3)?, field(4)?, field(5)?);
            if !(0.0..24.0).contains(&hour)
                || !(0.0..60.0).contains(&minute)
                || !(0.0..60.0).contains(&second)
            {
                return Err(());
            }
            MessageDateTime::at(date, hour as u8, minute as u8, second as u8)
        }
        _ => return Err(()),
    }))
}

fn currency_arg(parts: &[HostValue]) -> Result<MessageArg, ()> {
    match parts {
        [HostValue::Number(amount), HostValue::String(code)]
            if amount.is_finite() && !code.trim().is_empty() =>
        {
            Ok(MessageArg::currency(
                *amount,
                &code.trim().to_ascii_uppercase(),
            ))
        }
        _ => Err(()),
    }
}

/// A catalog as `Nana.i18n.setCatalog` sends it: `[[locale, message,
/// pattern], ...]`, the fallback locale, and what a missing message shows.
pub(crate) struct CatalogRequest {
    entries: Vec<(LanguageTag, String, String)>,
    /// Of the entries in order: the same catalog sent again (an isolated
    /// window runs the application script again) is recognized without
    /// building or installing anything.
    pub(crate) fingerprint: u64,
    pub(crate) fallback: Option<LanguageTag>,
    pub(crate) missing: MissingMessage,
}

impl CatalogRequest {
    pub(crate) fn parse(args: &[HostValue]) -> Result<Self, JsException> {
        let invalid = |at: usize| {
            JsException::new(format!(
                "catalog entry {at} is not [locale, message, pattern]"
            ))
            .with_name("TypeError")
        };
        let items = match args.first() {
            Some(HostValue::Array(items)) => items.as_slice(),
            None | Some(HostValue::Null | HostValue::Undefined) => &[],
            Some(_) => {
                return Err(
                    JsException::new("a catalog is a list of [locale, message, pattern]")
                        .with_name("TypeError"),
                );
            }
        };
        let mut entries = Vec::with_capacity(items.len());
        let mut hasher = DefaultHasher::new();
        for (at, item) in items.iter().enumerate() {
            let [
                HostValue::String(locale),
                HostValue::String(message),
                HostValue::String(pattern),
            ] = item
                .as_array()
                .map(Vec::as_slice)
                .ok_or_else(|| invalid(at))?
            else {
                return Err(invalid(at));
            };
            let locale = LanguageTag::new(locale).ok_or_else(|| invalid(at))?;
            if message.trim().is_empty() {
                return Err(invalid(at));
            }
            (locale.as_str(), message, pattern).hash(&mut hasher);
            entries.push((locale, message.clone(), pattern.clone()));
        }
        let fallback = match args.get(1) {
            None | Some(HostValue::Null | HostValue::Undefined) => None,
            Some(HostValue::String(tag)) => LanguageTag::new(tag),
            Some(_) => {
                return Err(
                    JsException::new("a fallback locale is a language tag").with_name("TypeError")
                );
            }
        };
        let missing = match args.get(2) {
            None | Some(HostValue::Null | HostValue::Undefined) => MissingMessage::Key,
            Some(value) => match value.as_str() {
                Some("key") => MissingMessage::Key,
                Some("keep-previous") => MissingMessage::KeepPrevious,
                _ => {
                    return Err(
                        JsException::new("missing must be \"key\" or \"keep-previous\"")
                            .with_name("TypeError"),
                    );
                }
            },
        };
        Ok(Self {
            entries,
            fingerprint: hasher.finish(),
            fallback,
            missing,
        })
    }

    /// The table the entries make; a later entry for the same locale and
    /// message wins.
    pub(crate) fn table(&self) -> Arc<MessageTable> {
        let mut table = MessageTable::new();
        for (locale, message, pattern) in &self.entries {
            table.insert(locale.clone(), MessageId::new(message), pattern.as_str());
        }
        Arc::new(table)
    }
}

/// A locale as JavaScript gives one: a language tag, `{ messages, language,
/// direction, formatting }`, or `null` for none. An empty tag is none.
pub(crate) fn parse_locale(value: Option<&HostValue>) -> Result<Option<Locale>, JsException> {
    let invalid = || {
        JsException::new(
            "a locale is a language tag, { messages, language, direction, formatting } or null",
        )
        .with_name("TypeError")
    };
    let fields = match value {
        None | Some(HostValue::Null | HostValue::Undefined) => return Ok(None),
        Some(HostValue::String(tag)) => return Ok(Locale::parse(tag)),
        Some(HostValue::Object(fields)) => fields,
        Some(_) => return Err(invalid()),
    };
    let tag = |name: &str| match fields.get(name) {
        None | Some(HostValue::Null | HostValue::Undefined) => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(LanguageTag::new)
            .map(Some)
            .ok_or_else(invalid),
    };
    let mut locale = Locale::new(tag("messages")?.ok_or_else(invalid)?);
    if let Some(language) = tag("language")? {
        locale = locale.with_language(language);
    }
    if let Some(formatting) = tag("formatting")? {
        locale = locale.with_formatting(formatting);
    }
    match fields.get("direction") {
        None | Some(HostValue::Null | HostValue::Undefined) => {}
        Some(value) => {
            locale = locale.with_direction(match value.as_str() {
                Some("ltr") => DirSpec::Ltr,
                Some("rtl") => DirSpec::Rtl,
                _ => return Err(invalid()),
            });
        }
    }
    if fields.keys().any(|key| {
        !matches!(
            key.as_str(),
            "messages" | "language" | "direction" | "formatting"
        )
    }) {
        return Err(invalid());
    }
    Ok(Some(locale))
}

/// A locale as `Nana.i18n.locale` reads it back: the tag of its messages.
pub(crate) fn locale_tag_value(locale: Option<&Locale>) -> HostValue {
    locale.map_or(HostValue::Null, |locale| {
        HostValue::string(locale.messages().as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(value: &str) -> HostValue {
        HostValue::string(value)
    }

    #[test]
    fn message_args_read_numbers_text_dates_and_currencies() {
        let (args, error) = parse_message_args(
            r#"{"count": 3, "name": "Nana", "on": true, "when": {"$date": [2026, 10, 9]},
                "at": {"$date": [2026, 10, 9, 14, 5, 0]}, "price": {"$currency": [9.5, "eur"]},
                "gone": null}"#,
        );
        assert_eq!(error, None);
        assert_eq!(args.get("count"), Some(&MessageArg::Number(3.0)));
        assert_eq!(args.get("name"), Some(&MessageArg::from("Nana")));
        assert_eq!(args.get("on"), Some(&MessageArg::from("true")));
        let day = CivilDate::new(2026, 10, 9).unwrap();
        assert_eq!(
            args.get("when"),
            Some(&MessageArg::from(MessageDateTime::date(day)))
        );
        assert_eq!(
            args.get("at"),
            Some(&MessageArg::from(MessageDateTime::at(day, 14, 5, 0)))
        );
        assert_eq!(args.get("price"), Some(&MessageArg::currency(9.5, "EUR")));
        assert_eq!(args.get("gone"), None);
    }

    #[test]
    fn broken_message_args_are_reported_and_keep_the_rest() {
        let (args, error) = parse_message_args("{\"count\": 3");
        assert_eq!(args, MessageArgs::new());
        assert!(error.is_some_and(|error| error.starts_with("message-args:")));

        let (args, error) = parse_message_args("[1, 2]");
        assert_eq!(args, MessageArgs::new());
        assert!(error.is_some());

        let (args, error) =
            parse_message_args(r#"{"count": 3, "when": {"$date": [2026, 13, 1]}, "list": [1]}"#);
        assert_eq!(args, MessageArgs::new().with("count", 3));
        assert!(error.is_some_and(|error| error.contains("`list`") || error.contains("`when`")));
    }

    #[test]
    fn a_catalog_is_triples_and_the_same_catalog_has_the_same_fingerprint() {
        let entries = |pattern: &str| {
            HostValue::Array(vec![
                HostValue::Array(vec![string("en-US"), string("files"), string(pattern)]),
                HostValue::Array(vec![
                    string("ar"),
                    string("onboarding.title"),
                    string("مرحبا"),
                ]),
            ])
        };
        let first = CatalogRequest::parse(&[entries("{count} files")]).unwrap();
        let again = CatalogRequest::parse(&[entries("{count} files")]).unwrap();
        let other = CatalogRequest::parse(&[entries("{count} items")]).unwrap();
        assert_eq!(first.fingerprint, again.fingerprint);
        assert_ne!(first.fingerprint, other.fingerprint);
        assert_eq!(first.fallback, None);
        assert_eq!(first.missing, MissingMessage::Key);
        let table = first.table();
        use nana_ui_runtime::MessageCatalog;
        assert_eq!(
            table
                .pattern(
                    &LanguageTag::new("ar").unwrap(),
                    MessageId::new("onboarding.title")
                )
                .as_deref(),
            Some("مرحبا")
        );

        let options =
            CatalogRequest::parse(&[entries("x"), string("en"), string("keep-previous")]).unwrap();
        assert_eq!(options.fallback, LanguageTag::new("en"));
        assert_eq!(options.missing, MissingMessage::KeepPrevious);

        for bad in [
            vec![string("files")],
            vec![HostValue::Array(vec![HostValue::Array(vec![
                string("en"),
                string("files"),
            ])])],
            vec![HostValue::Array(vec![]), HostValue::Number(1.0)],
            vec![
                HostValue::Array(vec![]),
                HostValue::Null,
                string("previous"),
            ],
        ] {
            assert!(CatalogRequest::parse(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_locale_is_a_tag_or_its_four_parts() {
        assert_eq!(parse_locale(None).unwrap(), None);
        assert_eq!(parse_locale(Some(&string(""))).unwrap(), None);
        assert_eq!(
            parse_locale(Some(&string("ar"))).unwrap(),
            Locale::parse("ar")
        );
        let parts = HostValue::Object(
            [
                ("messages".to_owned(), string("ar")),
                ("direction".to_owned(), string("ltr")),
                ("formatting".to_owned(), string("ar-EG")),
                ("language".to_owned(), HostValue::Null),
            ]
            .into_iter()
            .collect(),
        );
        let locale = parse_locale(Some(&parts)).unwrap().unwrap();
        assert_eq!(locale.messages().as_str(), "ar");
        assert_eq!(locale.direction(), DirSpec::Ltr);
        assert_eq!(locale.formatting().as_str(), "ar-eg");
        assert_eq!(locale_tag_value(Some(&locale)), string("ar"));
        assert_eq!(locale_tag_value(None), HostValue::Null);

        for bad in [
            HostValue::Number(1.0),
            HostValue::Object([("message".to_owned(), string("ar"))].into_iter().collect()),
            HostValue::Object(
                [
                    ("messages".to_owned(), string("ar")),
                    ("direction".to_owned(), string("up")),
                ]
                .into_iter()
                .collect(),
            ),
        ] {
            assert!(parse_locale(Some(&bad)).is_err(), "{bad:?}");
        }
    }
}
