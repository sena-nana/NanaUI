//! Lightweight, zero-dependency Fluent (`.ftl`) parser and [`MessageCatalog`]
//! adapter.
//!
//! Supports the Fluent subset used by desktop and mobile applications without
//! linking external Fluent crates:
//! - comments (`#`, `##`, `###`) and blank lines;
//! - message definitions (`key = value`) and attributes (`.attr = value`,
//!   indexed as `"key.attr"`);
//! - term definitions (`-term = value`) and term/message references (`{-term}`,
//!   `{other-key}`);
//! - multiline values with Fluent common-indent stripping;
//! - variable placeables (`{$var}`) and string/numeric literals (`{"..."}`);
//! - select expressions (`{ $var -> [one] ... *[other] ... }`), lowered into
//!   NanaUI's [`crate::MessagePattern`] plural/select syntax.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use nana_text::font::LanguageTag;

use super::{MessageCatalog, MessageId};

/// Error reported while parsing a Fluent (`.ftl`) resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FluentParseError {
    /// 1-based line number where parsing failed.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for FluentParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fluent parse error at line {}: {}",
            self.line, self.message
        )
    }
}

impl std::error::Error for FluentParseError {}

/// A parsed `.ftl` file: message keys (and `key.attribute` keys) mapped to
/// [`crate::MessagePattern`]-compatible pattern strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FluentResource {
    entries: HashMap<Arc<str>, Arc<str>>,
}

impl FluentResource {
    /// Parse a Fluent `.ftl` source string into a [`FluentResource`].
    pub fn parse(source: &str) -> Result<Self, FluentParseError> {
        let raw = parse_raw_entries(source)?;
        let mut resolved = HashMap::with_capacity(raw.messages.len());
        let mut visiting = Vec::new();
        for (key, raw_pattern) in &raw.messages {
            let lowered = lower_pattern(
                raw_pattern,
                &raw.messages,
                &raw.terms,
                &mut visiting,
                key.line,
            )?;
            resolved.insert(Arc::clone(&key.name), Arc::from(lowered));
        }
        Ok(Self { entries: resolved })
    }

    /// Look up the lowered pattern string for `key` (or `"key.attribute"`).
    pub fn get(&self, key: &str) -> Option<&Arc<str>> {
        self.entries.get(key)
    }

    /// Every parsed message and attribute entry.
    pub fn entries(&self) -> &HashMap<Arc<str>, Arc<str>> {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A [`MessageCatalog`] populated from one or more Fluent (`.ftl`) resources.
#[derive(Debug, Clone, Default)]
pub struct FluentCatalog {
    patterns: HashMap<(LanguageTag, MessageId), Arc<str>>,
}

impl FluentCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `source` as FTL and insert every message and attribute under
    /// `locale`. Returns the number of entries added.
    pub fn add_ftl(&mut self, locale: &str, source: &str) -> Result<usize, FluentParseError> {
        let tag = LanguageTag::new(locale).ok_or_else(|| FluentParseError {
            line: 1,
            message: format!("invalid BCP-47 language tag `{locale}`"),
        })?;
        let resource = FluentResource::parse(source)?;
        let count = resource.len();
        self.add_resource(tag, &resource);
        Ok(count)
    }

    /// Builder form of [`Self::add_ftl`].
    pub fn with_ftl(mut self, locale: &str, source: &str) -> Result<Self, FluentParseError> {
        self.add_ftl(locale, source)?;
        Ok(self)
    }

    /// Insert every entry of an already-parsed [`FluentResource`] under `locale`.
    pub fn add_resource(&mut self, locale: LanguageTag, resource: &FluentResource) {
        for (key, pattern) in resource.entries() {
            self.patterns
                .insert((locale.clone(), MessageId::new(key)), Arc::clone(pattern));
        }
    }
}

impl MessageCatalog for FluentCatalog {
    fn pattern(&self, locale: &LanguageTag, message: MessageId) -> Option<Arc<str>> {
        self.patterns.get(&(locale.clone(), message)).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EntryKey {
    name: Arc<str>,
    line: usize,
}

struct RawEntries {
    messages: HashMap<EntryKey, String>,
    terms: HashMap<String, String>,
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

fn valid_ident(ident: &str) -> bool {
    let mut chars = ident.chars();
    chars.next().is_some_and(is_ident_start) && chars.all(is_ident_continue)
}

fn parse_raw_entries(source: &str) -> Result<RawEntries, FluentParseError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let lines: Vec<&str> = source.lines().collect();
    let mut messages: HashMap<EntryKey, String> = HashMap::new();
    let mut terms: HashMap<String, String> = HashMap::new();

    let mut idx = 0usize;
    let mut current_owner: Option<String> = None;

    while idx < lines.len() {
        let line_no = idx + 1;
        let line = lines[idx];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            idx += 1;
            continue;
        }

        // Attribute belonging to the preceding message/term: `    .attr = value`
        if line.starts_with([' ', '\t']) && trimmed.starts_with('.') {
            let Some(owner) = current_owner.as_deref() else {
                return Err(FluentParseError {
                    line: line_no,
                    message: "attribute has no owning message".into(),
                });
            };
            let rest = &trimmed[1..];
            let Some((attr_raw, value_head)) = rest.split_once('=') else {
                return Err(FluentParseError {
                    line: line_no,
                    message: "expected `=` in attribute definition".into(),
                });
            };
            let attr = attr_raw.trim();
            if !valid_ident(attr) {
                return Err(FluentParseError {
                    line: line_no,
                    message: format!("invalid attribute identifier `{attr}`"),
                });
            }
            let (value, next_idx) = collect_value(&lines, idx, value_head);
            idx = next_idx;
            let full = format!("{owner}.{attr}");
            if owner.starts_with('-') {
                terms.insert(full, value);
            } else {
                messages.insert(
                    EntryKey {
                        name: Arc::from(full),
                        line: line_no,
                    },
                    value,
                );
            }
            continue;
        }

        // Top-level message (`key = ...`) or term (`-term = ...`).
        if line.starts_with([' ', '\t']) {
            return Err(FluentParseError {
                line: line_no,
                message: "unexpected indented line outside message value".into(),
            });
        }
        let Some((lhs_raw, value_head)) = line.split_once('=') else {
            return Err(FluentParseError {
                line: line_no,
                message: "expected `=` in entry definition".into(),
            });
        };
        let lhs = lhs_raw.trim();
        let is_term = lhs.starts_with('-');
        let ident = if is_term { &lhs[1..] } else { lhs };
        if !valid_ident(ident) {
            return Err(FluentParseError {
                line: line_no,
                message: format!("invalid entry identifier `{lhs}`"),
            });
        }
        let (value, next_idx) = collect_value(&lines, idx, value_head);
        idx = next_idx;
        current_owner = Some(lhs.to_owned());

        if is_term {
            terms.insert(lhs.to_owned(), value);
        } else if !value.is_empty() {
            messages.insert(
                EntryKey {
                    name: Arc::from(lhs),
                    line: line_no,
                },
                value,
            );
        }
    }

    Ok(RawEntries { messages, terms })
}

/// Collect the inline head and any indented continuation lines of a Fluent
/// value, stopping before the next attribute (`.attr = ...`), comment, or
/// top-level entry while keeping select-expression blocks intact.
fn collect_value(lines: &[&str], start_idx: usize, head_after_eq: &str) -> (String, usize) {
    let head = head_after_eq.trim_start_matches([' ', '\t']);
    let mut brace_depth = brace_delta(head);
    let mut cont: Vec<&str> = Vec::new();
    let mut idx = start_idx + 1;

    while idx < lines.len() {
        let line = lines[idx];
        let trimmed = line.trim();
        if brace_depth <= 0 {
            if trimmed.is_empty() {
                // Keep blank lines only if another indented value continuation follows.
                let mut look = idx + 1;
                while look < lines.len() && lines[look].trim().is_empty() {
                    look += 1;
                }
                if look < lines.len()
                    && lines[look].starts_with([' ', '\t'])
                    && !lines[look].trim_start().starts_with('.')
                    && !lines[look].trim_start().starts_with('#')
                {
                    cont.push("");
                    idx += 1;
                    continue;
                }
                break;
            }
            if !line.starts_with([' ', '\t'])
                || trimmed.starts_with('#')
                || (trimmed.starts_with('.') && trimmed.contains('='))
            {
                break;
            }
        }
        brace_depth += brace_delta(line);
        cont.push(line);
        idx += 1;
    }

    if cont.is_empty() {
        return (head.trim_end().to_owned(), idx);
    }

    // Strip common leading spaces across non-empty continuation lines (Fluent
    // block text dedent).
    let common_indent = cont
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.bytes()
                .take_while(|b| *b == b' ' || *b == b'\t')
                .count()
        })
        .min()
        .unwrap_or(0);

    let mut out = String::new();
    if !head.is_empty() {
        out.push_str(head.trim_end());
    }
    for line in cont {
        if !out.is_empty() {
            out.push('\n');
        }
        if line.trim().is_empty() {
            continue;
        }
        let dedented = &line[common_indent.min(line.len())..];
        out.push_str(dedented.trim_end());
    }
    (out, idx)
}

fn brace_delta(text: &str) -> isize {
    let mut depth = 0isize;
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn lower_pattern(
    raw: &str,
    messages: &HashMap<EntryKey, String>,
    terms: &HashMap<String, String>,
    visiting: &mut Vec<String>,
    line: usize,
) -> Result<String, FluentParseError> {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = raw[i..].chars().next().unwrap();
        if ch == '{' {
            let end = find_matching_brace(raw, i).ok_or_else(|| FluentParseError {
                line,
                message: "unclosed `{` placeable".into(),
            })?;
            let inner = raw[i + 1..end].trim();
            lower_placeable(inner, messages, terms, visiting, line, &mut out)?;
            i = end + 1;
        } else {
            push_escaped_literal_char(&mut out, ch);
            i += ch.len_utf8();
        }
    }
    Ok(out)
}

fn push_escaped_literal_char(out: &mut String, ch: char) {
    match ch {
        '\'' => out.push_str("''"),
        '{' | '}' => {
            out.push('\'');
            out.push(ch);
            out.push('\'');
        }
        _ => out.push(ch),
    }
}

fn push_escaped_literal(out: &mut String, text: &str) {
    for ch in text.chars() {
        push_escaped_literal_char(out, ch);
    }
}

fn find_matching_brace(s: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in s[open_idx..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_idx + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn lower_placeable(
    inner: &str,
    messages: &HashMap<EntryKey, String>,
    terms: &HashMap<String, String>,
    visiting: &mut Vec<String>,
    line: usize,
    out: &mut String,
) -> Result<(), FluentParseError> {
    // Select expression: `$selector -> [key] val *[other] val`
    if let Some((selector_raw, variants_raw)) = split_top_level_arrow(inner) {
        return lower_select(
            selector_raw.trim(),
            variants_raw,
            messages,
            terms,
            visiting,
            line,
            out,
        );
    }

    // String literal: `"..."`
    if inner.starts_with('"') && inner.ends_with('"') && inner.len() >= 2 {
        let unescaped = unescape_fluent_string(&inner[1..inner.len() - 1], line)?;
        push_escaped_literal(out, &unescaped);
        return Ok(());
    }

    // Variable reference: `$name`
    if let Some(var) = inner.strip_prefix('$') {
        let var = var.trim();
        if !valid_ident(var) {
            return Err(FluentParseError {
                line,
                message: format!("invalid variable name `${var}`"),
            });
        }
        out.push('{');
        out.push_str(var);
        out.push('}');
        return Ok(());
    }

    // Term reference: `-term` or `-term.attr`
    if inner.starts_with('-') {
        let term_key = inner.split('(').next().unwrap_or(inner).trim();
        if visiting.iter().any(|k| k == term_key) {
            return Err(FluentParseError {
                line,
                message: format!("cyclic term reference `{term_key}`"),
            });
        }
        if let Some(raw_term) = terms.get(term_key) {
            visiting.push(term_key.to_owned());
            let lowered = lower_pattern(raw_term, messages, terms, visiting, line)?;
            visiting.pop();
            out.push_str(&lowered);
            return Ok(());
        }
        push_escaped_literal(out, term_key);
        return Ok(());
    }

    // Numeric literal
    if inner.parse::<f64>().is_ok() {
        out.push_str(inner);
        return Ok(());
    }

    // Message reference (`other-key` or `other-key.attr`)
    if let Some((_, raw_msg)) = messages.iter().find(|(k, _)| k.name.as_ref() == inner) {
        if visiting.iter().any(|k| k == inner) {
            return Err(FluentParseError {
                line,
                message: format!("cyclic message reference `{inner}`"),
            });
        }
        visiting.push(inner.to_owned());
        let lowered = lower_pattern(raw_msg, messages, terms, visiting, line)?;
        visiting.pop();
        out.push_str(&lowered);
        return Ok(());
    }

    // Fallback: emit as literal text if unknown call/ref.
    push_escaped_literal(out, inner);
    Ok(())
}

fn split_top_level_arrow(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let ch = s[i..].chars().next().unwrap();
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += ch.len_utf8();
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '-' if depth == 0 && bytes[i + 1] == b'>' => {
                return Some((&s[..i], &s[i + 2..]));
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn is_plural_category(key: &str) -> bool {
    matches!(key, "zero" | "one" | "two" | "few" | "many" | "other")
}

fn lower_select(
    selector: &str,
    variants_raw: &str,
    messages: &HashMap<EntryKey, String>,
    terms: &HashMap<String, String>,
    visiting: &mut Vec<String>,
    line: usize,
    out: &mut String,
) -> Result<(), FluentParseError> {
    let var = selector.strip_prefix('$').unwrap_or(selector).trim();
    if !valid_ident(var) {
        return Err(FluentParseError {
            line,
            message: format!("unsupported select selector `{selector}`"),
        });
    }

    // Parse `[key] value` and `*[key] value` variants.
    let mut variants: Vec<(bool, String, String)> = Vec::new();
    for raw_line in variants_raw.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (is_default, rest) = if let Some(after_star) = trimmed.strip_prefix('*') {
            (true, after_star.trim_start())
        } else {
            (false, trimmed)
        };
        if let Some(after_open) = rest.strip_prefix('[')
            && let Some((key_raw, val_raw)) = after_open.split_once(']')
        {
            variants.push((
                is_default,
                key_raw.trim().to_owned(),
                val_raw.trim().to_owned(),
            ));
        } else if let Some(last) = variants.last_mut() {
            if !last.2.is_empty() {
                last.2.push('\n');
            }
            last.2.push_str(trimmed);
        }
    }

    if variants.is_empty() {
        return Err(FluentParseError {
            line,
            message: "select expression has no variants".into(),
        });
    }

    let is_plural = variants
        .iter()
        .all(|(_, key, _)| is_plural_category(key) || key.parse::<f64>().is_ok());

    out.push('{');
    out.push_str(var);
    out.push_str(if is_plural { ", plural," } else { ", select," });

    let mut has_other = false;
    let mut default_body: Option<String> = None;

    for (is_default, key, raw_body) in &variants {
        let body = lower_pattern(raw_body, messages, terms, visiting, line)?;
        if *is_default {
            default_body = Some(body.clone());
        }
        if key == "other" {
            has_other = true;
        }
        out.push(' ');
        if is_plural && key.parse::<f64>().is_ok() {
            out.push('=');
            out.push_str(key);
        } else {
            out.push_str(key);
        }
        out.push_str(" {");
        out.push_str(&body);
        out.push('}');
    }

    if !has_other {
        if let Some(fallback) = default_body {
            out.push_str(" other {");
            out.push_str(&fallback);
            out.push('}');
        }
    }
    out.push('}');
    Ok(())
}

fn unescape_fluent_string(raw: &str, line: usize) -> Result<String, FluentParseError> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let code = u32::from_str_radix(&hex, 16).map_err(|_| FluentParseError {
                    line,
                    message: format!("invalid \\u escape `\\u{hex}`"),
                })?;
                let decoded = char::from_u32(code).ok_or_else(|| FluentParseError {
                    line,
                    message: format!("invalid unicode scalar U+{code:04X}"),
                })?;
                out.push(decoded);
            }
            Some('U') => {
                let hex: String = chars.by_ref().take(6).collect();
                let code = u32::from_str_radix(&hex, 16).map_err(|_| FluentParseError {
                    line,
                    message: format!("invalid \\U escape `\\U{hex}`"),
                })?;
                let decoded = char::from_u32(code).ok_or_else(|| FluentParseError {
                    line,
                    message: format!("invalid unicode scalar U+{code:06X}"),
                })?;
                out.push(decoded);
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => {
                return Err(FluentParseError {
                    line,
                    message: "trailing backslash in string literal".into(),
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, LocalizedText, MessageArgs, MessagePattern};
    use crate::{DocumentId, Text};

    #[test]
    fn parses_messages_attributes_terms_multiline_and_selectors() {
        let ftl = r#"
# File header comment
-brand = NanaLive

greeting = Hello, {$name}!
about = Welcome to {-brand}.
dialog-save = Save
    .tooltip = Save changes to {$file}

multiline =
    First line
    Second line

items = { $count ->
    [0] No items
    [one] { $count } item
   *[other] { $count } items
}

role-label = { $role ->
    [admin] Administrator
   *[member] Member
}
"#;
        let catalog = FluentCatalog::new().with_ftl("en-US", ftl).unwrap();
        let en = LanguageTag::new("en-US").unwrap();

        let format = |key: &str, args: MessageArgs| {
            let pat = catalog.pattern(&en, MessageId::new(key)).expect(key);
            MessagePattern::parse(&pat).unwrap().format(&en, &args)
        };

        assert_eq!(
            format("greeting", MessageArgs::new().with("name", "Sena")),
            "Hello, Sena!"
        );
        assert_eq!(format("about", MessageArgs::new()), "Welcome to NanaLive.");
        assert_eq!(format("dialog-save", MessageArgs::new()), "Save");
        assert_eq!(
            format(
                "dialog-save.tooltip",
                MessageArgs::new().with("file", "a.json")
            ),
            "Save changes to a.json"
        );
        assert_eq!(
            format("multiline", MessageArgs::new()),
            "First line\nSecond line"
        );
        assert_eq!(
            format("items", MessageArgs::new().with("count", 0)),
            "No items"
        );
        assert_eq!(
            format("items", MessageArgs::new().with("count", 1)),
            "1 item"
        );
        assert_eq!(
            format("items", MessageArgs::new().with("count", 5)),
            "5 items"
        );
        assert_eq!(
            format("role-label", MessageArgs::new().with("role", "admin")),
            "Administrator"
        );
        assert_eq!(
            format("role-label", MessageArgs::new().with("role", "guest")),
            "Member"
        );
    }

    #[test]
    fn fluent_catalog_drives_ui_world_localization() {
        let catalog = FluentCatalog::new()
            .with_ftl("en-US", "save = Save {$what}")
            .unwrap()
            .with_ftl("zh-CN", "save = 保存 {$what}")
            .unwrap();
        let mut cx = crate::AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        cx.set_message_catalog(Some(Arc::new(catalog)));
        cx.set_default_locale(Locale::parse("en-US"));
        let label = cx
            .create_component(
                doc,
                Text::localized(LocalizedText::new("save").arg("what", "scene")),
            )
            .unwrap();
        assert_eq!(cx.world().text(label.stable_id()), Some("Save scene"));
        cx.set_default_locale(Locale::parse("zh-CN"));
        assert_eq!(cx.world().text(label.stable_id()), Some("保存 scene"));
    }
}
