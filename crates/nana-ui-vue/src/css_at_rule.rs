//! L1 at-rule helpers: `@import`, `@media`, `@font-face`, `@supports`, `@layer`.
//!
//! Parse stays in [`crate::css_cascade`]; this module owns query evaluation,
//! href resolution, and `@font-face` descriptors.
//!
//! `@supports` evaluates a tiny predicate subset (`display: flex|grid|block`,
//! `color` values [`crate::style::parse_css_color`] accepts, `width` values
//! [`crate::css_map::LengthSpec::parse`] accepts). Unknown predicates fail
//! closed. `@layer name { rules }` / anonymous `@layer { }` join author source
//! order with names recorded — full cascade-layer priority is not implemented.
//! `@import … layer()` / `supports()` stay fail-closed (do not load).

#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::css_map::{CssLayoutParse, LengthSpec};

/// Nested `@import` depth cap (cycle guard is separate).
pub const MAX_IMPORT_DEPTH: u32 = 16;
/// CSS `@import` file size cap (fail closed when exceeded).
pub const MAX_STYLESHEET_BYTES: u64 = 1024 * 1024;
/// `@font-face` `url()` file size cap (fail closed when exceeded).
pub const MAX_FONT_FACE_BYTES: u64 = nana_ui_core::MAX_LOCAL_URL_BYTES;
/// Host-side cap on successfully registered `@font-face` bytes.
pub const MAX_REGISTERED_FONT_BYTES: u64 = 16 * 1024 * 1024;

/// Viewport + color-scheme facts used by CSS `@media` (mirrors JS `evaluateMediaQuery`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaEnvironment {
    pub width: f32,
    pub height: f32,
    pub color_scheme_dark: bool,
}

impl Default for MediaEnvironment {
    fn default() -> Self {
        // Match web-api `visualViewport` seed (960×640, light).
        Self {
            width: 960.0,
            height: 640.0,
            color_scheme_dark: false,
        }
    }
}

/// CSS media type in an `@media` / `@import` query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    All,
    Screen,
    Print,
    Other,
}

/// One media feature in an `and`-combined query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaFeature {
    MinWidth(f32),
    MaxWidth(f32),
    Width(f32),
    MinHeight(f32),
    MaxHeight(f32),
    Height(f32),
    OrientationLandscape,
    OrientationPortrait,
    PrefersColorSchemeDark,
    PrefersColorSchemeLight,
    /// Unknown feature — query fails closed (same as JS `evaluateMediaQuery`).
    Unsupported,
}

/// One comma-separated `@media` alternative (`not` / type / `and` features).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaQuery {
    pub negated: bool,
    pub media_type: MediaType,
    pub features: Vec<MediaFeature>,
}

/// Comma-separated media query list (OR). Empty list matches `all`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaQueryList {
    pub queries: Vec<MediaQuery>,
}

impl MediaQueryList {
    pub fn all() -> Self {
        Self {
            queries: vec![MediaQuery {
                negated: false,
                media_type: MediaType::All,
                features: Vec::new(),
            }],
        }
    }

    pub fn is_unconditional(&self) -> bool {
        self.queries.is_empty()
            || self
                .queries
                .iter()
                .any(|q| !q.negated && q.media_type == MediaType::All && q.features.is_empty())
    }
}

/// Load imported stylesheet text. `from` is the canonical href of the importer.
pub trait StylesheetLoader {
    /// Returns `(css_text, canonical_href)` or `None` when the href cannot be read.
    fn load(&self, href: &str, from: Option<&str>) -> Option<(String, String)>;
}

/// In-memory map for unit tests (`"a.css"` → body).
#[derive(Debug, Clone, Default)]
pub struct MemoryStylesheetLoader {
    pub files: HashMap<String, String>,
}

impl StylesheetLoader for MemoryStylesheetLoader {
    fn load(&self, href: &str, from: Option<&str>) -> Option<(String, String)> {
        if is_blocked_href(href) {
            return None;
        }
        let key = resolve_memory_href(href, from);
        let css = self.files.get(&key)?.clone();
        if css.len() as u64 > stylesheet_byte_cap() {
            return None;
        }
        Some((css, key))
    }
}

/// Filesystem loader: relative/`file://` only; remote / UNC / jail escape refused.
#[derive(Debug, Clone, Copy)]
pub struct FsStylesheetLoader<'a> {
    pub base: &'a Path,
}

impl StylesheetLoader for FsStylesheetLoader<'_> {
    fn load(&self, href: &str, from: Option<&str>) -> Option<(String, String)> {
        load_stylesheet_file(href, from, self.base)
    }
}

/// Options for [`crate::css_cascade::parse_stylesheet_full_with_options`].
pub struct ParseStylesheetOptions<'a> {
    pub media: MediaEnvironment,
    pub loader: Option<&'a dyn StylesheetLoader>,
    /// Canonical href of the sheet being parsed (cycle identity for the root).
    pub base_href: Option<&'a str>,
    pub import_cache: Option<&'a mut HashMap<String, crate::css_interactive::ParsedStylesheet>>,
}

impl Default for ParseStylesheetOptions<'static> {
    fn default() -> Self {
        Self {
            media: MediaEnvironment::default(),
            loader: None,
            base_href: None,
            import_cache: None,
        }
    }
}

/// Parsed `@font-face` descriptors (not a CSSOM FontFace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRule {
    pub family: String,
    pub src: Vec<FontFaceSrc>,
    /// CSS `font-weight` start (`400`, `bold` → 700). Range start for `200 700`.
    pub weight: Option<u16>,
    /// Inclusive CSS `font-weight` range end. `None` means a single value.
    pub weight_end: Option<u16>,
    /// Canonical href of the sheet that declared this rule (relative `url()` base).
    pub base_href: Option<String>,
}

impl FontFaceRule {
    /// Inclusive `(min, max)` CSS weight span, or `None` if the descriptor was omitted.
    pub fn weight_span(&self) -> Option<(u16, u16)> {
        let start = self.weight?;
        let end = self.weight_end.unwrap_or(start);
        Some((start.min(end), start.max(end)))
    }
}

/// `@import` prelude after the URL: load, or fail closed (`layer` / `supports`).
#[derive(Debug, Clone, PartialEq)]
pub enum ImportPrelude {
    Ready {
        href: String,
        media: MediaQueryList,
    },
    /// `@import … layer` / `supports()` — not implemented; do not load.
    Unsupported,
}

/// `@layer` prelude: comma-separated names (possibly dotted). Empty = anonymous.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayerPrelude {
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontFaceSrc {
    Url(String),
    Local(String),
}

/// Parse a media query list (`screen and (min-width: 800px), (orientation: landscape)`).
pub fn parse_media_query_list(prelude: &str) -> MediaQueryList {
    let trimmed = prelude.trim();
    if trimmed.is_empty() {
        return MediaQueryList::all();
    }
    let mut queries = Vec::new();
    for part in split_comma_respecting_parens(trimmed) {
        if let Some(query) = parse_one_media_query(part) {
            queries.push(query);
        } else {
            queries.push(MediaQuery {
                negated: false,
                media_type: MediaType::All,
                features: vec![MediaFeature::Unsupported],
            });
        }
    }
    if queries.is_empty() {
        MediaQueryList::all()
    } else {
        MediaQueryList { queries }
    }
}

/// Evaluate a query list against viewport / color-scheme (OR of alternatives).
pub fn evaluate_media_query_list(list: &MediaQueryList, env: &MediaEnvironment) -> bool {
    if list.queries.is_empty() {
        return true;
    }
    list.queries
        .iter()
        .any(|query| evaluate_media_query(query, env))
}

/// Evaluate one query. Unknown features fail closed, matching JS `evaluateMediaQuery`.
pub fn evaluate_media_query(query: &MediaQuery, env: &MediaEnvironment) -> bool {
    let type_ok = match query.media_type {
        MediaType::All | MediaType::Screen => true,
        MediaType::Print | MediaType::Other => false,
    };
    let features_ok = query.features.iter().all(|feature| match *feature {
        MediaFeature::MinWidth(px) => env.width >= px,
        MediaFeature::MaxWidth(px) => env.width <= px,
        MediaFeature::Width(px) => (env.width - px).abs() < 0.5,
        MediaFeature::MinHeight(px) => env.height >= px,
        MediaFeature::MaxHeight(px) => env.height <= px,
        MediaFeature::Height(px) => (env.height - px).abs() < 0.5,
        MediaFeature::OrientationLandscape => env.width >= env.height,
        MediaFeature::OrientationPortrait => env.height > env.width,
        MediaFeature::PrefersColorSchemeDark => env.color_scheme_dark,
        MediaFeature::PrefersColorSchemeLight => !env.color_scheme_dark,
        MediaFeature::Unsupported => false,
    });
    let matched = type_ok && features_ok;
    if query.negated { !matched } else { matched }
}

/// `@import` prelude after `@import`. `layer` / `supports()` fail closed (no load).
pub fn parse_import_prelude(prelude: &str) -> Option<ImportPrelude> {
    if prelude_has_unsupported_import_function(prelude) {
        return Some(ImportPrelude::Unsupported);
    }
    let (href, after_url) = parse_css_url_or_string(prelude.trim_start())?;
    Some(ImportPrelude::Ready {
        href,
        media: parse_media_query_list(after_url),
    })
}

/// Evaluate an `@supports` condition. `None` = unknown predicate (fail closed).
///
/// Tiny L1 subset: `(display: flex|grid|block)`, `(color: <parseable>)`,
/// `(width: <parseable>)`, plus `not` / `and` / `or` of those. Mixing `and`
/// and `or` at the same level without wrapping parens is unknown.
pub fn evaluate_supports_condition(prelude: &str) -> Option<bool> {
    evaluate_supports_expr(prelude.trim())
}

/// Parse `@layer` names (`base`, `framework.layout, utilities`). Empty prelude
/// is the anonymous layer. Junk / `supports()` / `layer()` → `None`.
pub fn parse_layer_prelude(prelude: &str) -> Option<LayerPrelude> {
    let trimmed = prelude.trim();
    if trimmed.is_empty() {
        return Some(LayerPrelude { names: Vec::new() });
    }
    let mut names = Vec::new();
    for part in trimmed.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        names.push(parse_layer_name(part)?);
    }
    Some(LayerPrelude { names })
}

/// Remote / data / protocol-relative / UNC hrefs are never fetched for L1
/// `@import` / `@font-face`. Checked before any filesystem canonicalize.
pub fn is_blocked_href(href: &str) -> bool {
    nana_ui_core::is_remote_or_data_href(href)
        || nana_ui_core::href_is_protocol_relative_or_unc(href)
}

pub(crate) use nana_ui_core::stylesheet_base_from_href;

/// Cumulative `@font-face` host cap (per-file [`MAX_FONT_FACE_BYTES`], total
/// [`MAX_REGISTERED_FONT_BYTES`]).
pub fn font_registration_would_exceed_cap(used: u64, add: u64) -> bool {
    add > MAX_FONT_FACE_BYTES || used.saturating_add(add) > MAX_REGISTERED_FONT_BYTES
}

/// Build a font-face from already-split `property` / `value` pairs.
pub fn font_face_from_pairs<'a, I>(pairs: I) -> Option<FontFaceRule>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut family = None;
    let mut src = Vec::new();
    let mut weight = None;
    let mut weight_end = None;
    for (property, value) in pairs {
        if property.eq_ignore_ascii_case("font-family") {
            let name = unquote_css_string(value.trim());
            if !name.is_empty() {
                family = Some(name);
            }
        } else if property.eq_ignore_ascii_case("src") {
            src = parse_font_face_src(value);
        } else if property.eq_ignore_ascii_case("font-weight") {
            if let Some((start, end)) = parse_font_weight_range(value) {
                weight = Some(start);
                weight_end = (end != start).then_some(end);
            }
        }
    }
    let family = family?;
    if src.is_empty() {
        return None;
    }
    Some(FontFaceRule {
        family,
        src,
        weight,
        weight_end,
        base_href: None,
    })
}

/// First `url(...)` in `src` suitable for host font loading.
#[cfg(test)]
pub fn font_face_url_src(face: &FontFaceRule) -> Option<&str> {
    font_face_url_srcs(face).next()
}

/// `url(...)` entries in declaration order. `local()` is omitted here
/// (host registration walks [`FontFaceRule::src`] including `local()`);
/// `format()` / `tech()` were dropped while parsing `src`.
#[cfg(test)]
pub fn font_face_url_srcs(face: &FontFaceRule) -> impl Iterator<Item = &str> {
    face.src.iter().filter_map(|src| match src {
        FontFaceSrc::Url(url) => Some(url.as_str()),
        FontFaceSrc::Local(_) => None,
    })
}

/// Load a CSS file: jail + size cap + remote / protocol-relative / UNC refuse.
pub fn load_stylesheet_file(
    href: &str,
    from: Option<&str>,
    jail: &Path,
) -> Option<(String, String)> {
    let (bytes, canonical) =
        nana_ui_core::read_file_within_jail(href, from, jail, stylesheet_byte_cap())?;
    let css = String::from_utf8(bytes).ok()?;
    Some((css, canonical.to_string_lossy().into_owned()))
}

/// Load `@font-face` bytes (relative to the declaring sheet, then jail).
///
/// Returns `(bytes, canonical_path)` so the host can dedupe by path/family/weight.
pub fn load_font_face_bytes(
    href: &str,
    from: Option<&str>,
    jail: &Path,
) -> Option<(Vec<u8>, PathBuf)> {
    nana_ui_core::read_file_within_jail(href, from, jail, font_face_byte_cap())
}

fn stylesheet_byte_cap() -> u64 {
    #[cfg(test)]
    {
        if let Some(cap) = TEST_STYLESHEET_CAP.with(Cell::get) {
            return cap;
        }
    }
    MAX_STYLESHEET_BYTES
}

fn font_face_byte_cap() -> u64 {
    #[cfg(test)]
    {
        if let Some(cap) = TEST_FONT_CAP.with(Cell::get) {
            return cap;
        }
    }
    MAX_FONT_FACE_BYTES
}

#[cfg(test)]
thread_local! {
    static TEST_STYLESHEET_CAP: Cell<Option<u64>> = const { Cell::new(None) };
    static TEST_FONT_CAP: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Override the CSS read cap for one test (restored afterwards).
#[cfg(test)]
pub fn with_stylesheet_byte_cap<R>(cap: u64, f: impl FnOnce() -> R) -> R {
    TEST_STYLESHEET_CAP.with(|slot| {
        let prev = slot.replace(Some(cap));
        let out = f();
        slot.set(prev);
        out
    })
}

/// Override the font read cap for one test (restored afterwards).
#[cfg(test)]
pub fn with_font_face_byte_cap<R>(cap: u64, f: impl FnOnce() -> R) -> R {
    TEST_FONT_CAP.with(|slot| {
        let prev = slot.replace(Some(cap));
        let out = f();
        slot.set(prev);
        out
    })
}

pub(crate) fn resolve_memory_href(href: &str, from: Option<&str>) -> String {
    let trimmed = href.trim().replace('\\', "/");
    if trimmed.starts_with('/') {
        return normalize_memory_path(&trimmed);
    }
    let origin_dir = from
        .map(|f| {
            let n = f.replace('\\', "/");
            match n.rfind('/') {
                Some(i) => n[..i].to_string(),
                None => String::new(),
            }
        })
        .unwrap_or_default();
    if origin_dir.is_empty() {
        normalize_memory_path(&trimmed)
    } else {
        normalize_memory_path(&format!("{origin_dir}/{trimmed}"))
    }
}

fn normalize_memory_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// CSS ident with escapes (`supp\6frts` → `supports`). `None` if `start` is not ident-start.
fn consume_css_ident(s: &str, start: usize) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    let first = *bytes.get(start)?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'\\') {
        return None;
    }
    let mut i = start;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            let (ch, next) = consume_css_escape(s, i);
            out.push(ch);
            i = next;
            continue;
        }
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
            i += 1;
            continue;
        }
        break;
    }
    if out.is_empty() { None } else { Some((out, i)) }
}

/// CSS Syntax: consume an escaped code point starting at `\\`.
fn consume_css_escape(s: &str, backslash_at: usize) -> (char, usize) {
    let bytes = s.as_bytes();
    let mut i = backslash_at + 1;
    if i >= bytes.len() {
        return ('\u{FFFD}', i);
    }
    if bytes[i].is_ascii_hexdigit() {
        let hex_start = i;
        let mut n = 0;
        while i < bytes.len() && n < 6 && bytes[i].is_ascii_hexdigit() {
            i += 1;
            n += 1;
        }
        let code = u32::from_str_radix(&s[hex_start..i], 16).unwrap_or(0);
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
        }
        let ch = if code == 0 {
            '\u{FFFD}'
        } else {
            char::from_u32(code).unwrap_or('\u{FFFD}')
        };
        return (ch, i);
    }
    if bytes[i] == b'\n' || bytes[i] == b'\r' || bytes[i] == b'\x0c' {
        return ('\u{FFFD}', i);
    }
    let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
    (ch, i + ch.len_utf8())
}

/// `layer` / `supports` tokens anywhere in the prelude (not inside `url()` / quotes).
/// Ident escapes are decoded so `supp\6frts` still fail-closes.
fn prelude_has_unsupported_import_function(prelude: &str) -> bool {
    let bytes = prelude.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                i += 1;
            }
            continue;
        }
        if i + 4 <= bytes.len() && prelude[i..i + 4].eq_ignore_ascii_case("url(") {
            i += 4;
            let mut depth = 1i32;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'\\' {
            if let Some((ident, next)) = consume_css_ident(prelude, i) {
                i = next;
                let ident = ident.to_ascii_lowercase();
                if ident == "layer" || ident == "supports" {
                    return true;
                }
                continue;
            }
        }
        i += 1;
    }
    false
}

fn parse_layer_name(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut rest = s;
    loop {
        let (ident, after) = take_ident(rest)?;
        if ident.is_empty() {
            return None;
        }
        rest = after;
        if rest.starts_with('.') {
            rest = &rest[1..];
            continue;
        }
        if rest.is_empty() {
            return Some(s.to_string());
        }
        return None;
    }
}

fn evaluate_supports_expr(s: &str) -> Option<bool> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let has_and = has_keyword_at_depth0(s, "and");
    let has_or = has_keyword_at_depth0(s, "or");
    if has_and && has_or {
        return None;
    }
    if has_and {
        let mut acc = true;
        for part in split_keyword_parts(s, "and") {
            acc = acc && evaluate_supports_term(part)?;
        }
        return Some(acc);
    }
    if has_or {
        let mut acc = false;
        let parts = split_keyword_parts(s, "or");
        if parts.is_empty() {
            return None;
        }
        for part in parts {
            acc = acc || evaluate_supports_term(part)?;
        }
        return Some(acc);
    }
    evaluate_supports_term(s)
}

fn evaluate_supports_term(s: &str) -> Option<bool> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (negated, rest) = if let Some(after) = strip_ident_prefix(s, "not") {
        (true, after.trim_start())
    } else {
        (false, s)
    };
    if rest.is_empty() {
        return None;
    }
    let value = evaluate_supports_paren(rest)?;
    Some(if negated { !value } else { value })
}

fn evaluate_supports_paren(s: &str) -> Option<bool> {
    let s = s.trim();
    if !s.starts_with('(') {
        return None;
    }
    let end = matching_paren_end(s)?;
    if end != s.len() - 1 {
        return None;
    }
    let inner = s[1..end].trim();
    if inner.is_empty() {
        return None;
    }
    if inner.starts_with('(')
        || strip_ident_prefix(inner, "not").is_some()
        || has_keyword_at_depth0(inner, "and")
        || has_keyword_at_depth0(inner, "or")
    {
        return evaluate_supports_expr(inner);
    }
    evaluate_supports_declaration(inner)
}

fn evaluate_supports_declaration(inner: &str) -> Option<bool> {
    let colon = find_colon_depth0(inner)?;
    let property = inner[..colon].trim();
    let value = inner[colon + 1..].trim();
    if property.is_empty() || value.is_empty() {
        return None;
    }
    if take_ident(property).is_none_or(|(ident, rest)| ident != property || !rest.is_empty()) {
        return None;
    }
    l1_supports_property_value(property, value)
}

fn l1_supports_property_value(property: &str, value: &str) -> Option<bool> {
    match property.to_ascii_lowercase().as_str() {
        "display" => Some(matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "flex"
                | "grid"
                | "block"
                | "none"
                | "contents"
                | "inline"
                | "inline-block"
                | "inline-flex"
                | "inline-grid"
                | "flow-root"
        )),
        "color" => Some(crate::style::parse_css_color(value).is_some()),
        "width" => Some(LengthSpec::parse(value).is_some()),
        _ => None,
    }
}

fn matching_paren_end(s: &str) -> Option<usize> {
    if !s.starts_with('(') {
        return None;
    }
    let mut depth = 0i32;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_colon_depth0(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn has_keyword_at_depth0(s: &str, keyword: &str) -> bool {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && is_keyword_at(s, i, keyword) {
            return true;
        }
        i += 1;
    }
    false
}

fn split_keyword_parts<'a>(s: &'a str, keyword: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && is_keyword_at(s, i, keyword) {
            let part = s[start..i].trim();
            if !part.is_empty() {
                out.push(part);
            }
            i += keyword.len();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            start = i;
            continue;
        }
        i += 1;
    }
    let part = s[start..].trim();
    if !part.is_empty() {
        out.push(part);
    }
    out
}

fn is_keyword_at(s: &str, i: usize, keyword: &str) -> bool {
    let bytes = s.as_bytes();
    if i > 0 && !bytes[i - 1].is_ascii_whitespace() {
        return false;
    }
    let end = i + keyword.len();
    if end > s.len() || !s[i..end].eq_ignore_ascii_case(keyword) {
        return false;
    }
    if end < bytes.len() {
        let next = bytes[end];
        if !next.is_ascii_whitespace() && next != b'(' {
            return false;
        }
    }
    true
}

fn parse_one_media_query(raw: &str) -> Option<MediaQuery> {
    let mut rest = raw.trim();
    if rest.is_empty() {
        return Some(MediaQuery {
            negated: false,
            media_type: MediaType::All,
            features: Vec::new(),
        });
    }
    let mut negated = false;
    if let Some(after) = strip_ident_prefix(rest, "not") {
        negated = true;
        rest = after.trim_start();
    } else if let Some(after) = strip_ident_prefix(rest, "only") {
        rest = after.trim_start();
    }

    let mut media_type = MediaType::All;
    let mut features = Vec::new();
    if rest.starts_with('(') {
        for feature_text in split_and_parts(rest) {
            features.push(parse_media_feature(feature_text));
        }
    } else {
        let (ty, after_ty) = take_ident(rest)?;
        media_type = match ty.to_ascii_lowercase().as_str() {
            "all" => MediaType::All,
            "screen" => MediaType::Screen,
            "print" => MediaType::Print,
            _ => MediaType::Other,
        };
        let after_ty = after_ty.trim_start();
        if let Some(after_and) = strip_ident_prefix(after_ty, "and") {
            for feature_text in split_and_parts(after_and.trim_start()) {
                features.push(parse_media_feature(feature_text));
            }
        } else if !after_ty.is_empty() {
            features.push(MediaFeature::Unsupported);
        }
    }
    Some(MediaQuery {
        negated,
        media_type,
        features,
    })
}

fn parse_media_feature(raw: &str) -> MediaFeature {
    let mut q = raw.trim();
    if q.starts_with('(') && q.ends_with(')') && q.len() >= 2 {
        q = q[1..q.len() - 1].trim();
    }
    let q = q.to_ascii_lowercase();
    if let Some(px) = px_after(&q, "min-width") {
        return MediaFeature::MinWidth(px);
    }
    if let Some(px) = px_after(&q, "max-width") {
        return MediaFeature::MaxWidth(px);
    }
    if let Some(px) = px_after(&q, "width") {
        return MediaFeature::Width(px);
    }
    if let Some(px) = px_after(&q, "min-height") {
        return MediaFeature::MinHeight(px);
    }
    if let Some(px) = px_after(&q, "max-height") {
        return MediaFeature::MaxHeight(px);
    }
    if let Some(px) = px_after(&q, "height") {
        return MediaFeature::Height(px);
    }
    if q == "orientation: landscape" || q == "orientation:landscape" {
        return MediaFeature::OrientationLandscape;
    }
    if q == "orientation: portrait" || q == "orientation:portrait" {
        return MediaFeature::OrientationPortrait;
    }
    let compact = q.replace(' ', "");
    if compact == "orientation:landscape" {
        return MediaFeature::OrientationLandscape;
    }
    if compact == "orientation:portrait" {
        return MediaFeature::OrientationPortrait;
    }
    if compact == "prefers-color-scheme:dark" {
        return MediaFeature::PrefersColorSchemeDark;
    }
    if compact == "prefers-color-scheme:light" {
        return MediaFeature::PrefersColorSchemeLight;
    }
    MediaFeature::Unsupported
}

fn px_after(q: &str, name: &str) -> Option<f32> {
    let prefix = format!("{name}:");
    let rest = q.strip_prefix(&prefix)?.trim();
    let rest = rest.strip_suffix("px")?.trim();
    rest.parse().ok()
}

fn split_and_parts(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let lower = s.to_ascii_lowercase();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && i + 5 <= bytes.len() && lower.as_bytes()[i..].starts_with(b" and ") {
            let part = s[start..i].trim();
            if !part.is_empty() {
                out.push(part);
            }
            i += 5;
            start = i;
            continue;
        }
        i += 1;
    }
    let part = s[start..].trim();
    if !part.is_empty() {
        out.push(part);
    }
    out
}

fn split_comma_respecting_parens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                let part = s[start..i].trim();
                if !part.is_empty() {
                    out.push(part);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let part = s[start..].trim();
    if !part.is_empty() {
        out.push(part);
    }
    out
}

fn strip_ident_prefix<'a>(s: &'a str, ident: &str) -> Option<&'a str> {
    if s.len() < ident.len() {
        return None;
    }
    if !s[..ident.len()].eq_ignore_ascii_case(ident) {
        return None;
    }
    let after = &s[ident.len()..];
    if after
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(after)
}

fn take_ident(s: &str) -> Option<(&str, &str)> {
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    Some((&s[..end], &s[end..]))
}

pub(crate) fn parse_css_url_or_string(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    if s.len() >= 4 && s[..4].eq_ignore_ascii_case("url(") {
        let inner = &s[4..];
        let end = inner.find(')')?;
        let raw = inner[..end].trim();
        let href = unquote_css_string(raw);
        if href.is_empty() {
            return None;
        }
        return Some((href, inner[end + 1..].trim_start()));
    }
    if let Some((quoted, rest)) = take_quoted(s) {
        if quoted.is_empty() {
            return None;
        }
        return Some((quoted, rest.trim_start()));
    }
    None
}

fn take_quoted(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    let quote = *bytes.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            return Some((s[1..i].to_string(), &s[i + 1..]));
        }
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        i += 1;
    }
    None
}

fn unquote_css_string(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn parse_font_face_src(value: &str) -> Vec<FontFaceSrc> {
    let mut out = Vec::new();
    let mut rest = value.trim();
    while !rest.is_empty() {
        rest = rest.trim_start_matches(',').trim_start();
        if rest.is_empty() {
            break;
        }
        if rest.len() >= 6 && rest[..6].eq_ignore_ascii_case("local(") {
            let inner = &rest[6..];
            if let Some(end) = inner.find(')') {
                let name = unquote_css_string(inner[..end].trim());
                if !name.is_empty() {
                    out.push(FontFaceSrc::Local(name));
                }
                rest = inner[end + 1..].trim_start();
                rest = skip_format_hint(rest);
                continue;
            }
            break;
        }
        if let Some((href, after)) = parse_css_url_or_string(rest) {
            out.push(FontFaceSrc::Url(href));
            rest = skip_format_hint(after);
            continue;
        }
        break;
    }
    out
}

fn skip_format_hint(s: &str) -> &str {
    let mut t = s.trim_start();
    loop {
        if t.len() >= 7 && t[..7].eq_ignore_ascii_case("format(") {
            if let Some(end) = t.find(')') {
                t = t[end + 1..].trim_start();
                continue;
            }
        }
        if t.len() >= 5 && t[..5].eq_ignore_ascii_case("tech(") {
            if let Some(end) = t.find(')') {
                t = t[end + 1..].trim_start();
                continue;
            }
        }
        return t;
    }
}

fn parse_font_weight_range(value: &str) -> Option<(u16, u16)> {
    let mut parts = value.split_whitespace();
    let start = parse_one_font_weight_token(parts.next()?.trim())?;
    let end = match parts.next() {
        Some(token) => parse_one_font_weight_token(token.trim())?,
        None => start,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((start.min(end), start.max(end)))
}

fn parse_one_font_weight_token(token: &str) -> Option<u16> {
    match token.to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        other => {
            let n: f32 = other.parse().ok()?;
            if !n.is_finite() {
                return None;
            }
            Some(n.round().clamp(1.0, 1000.0) as u16)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_min_width_matches_js_subset() {
        let env = MediaEnvironment {
            width: 900.0,
            height: 500.0,
            color_scheme_dark: false,
        };
        let yes = parse_media_query_list("(min-width: 800px)");
        let no = parse_media_query_list("(min-width: 1000px)");
        assert!(evaluate_media_query_list(&yes, &env));
        assert!(!evaluate_media_query_list(&no, &env));
    }

    #[test]
    fn media_prefers_color_scheme_and_orientation() {
        let dark = MediaEnvironment {
            width: 400.0,
            height: 800.0,
            color_scheme_dark: true,
        };
        assert!(evaluate_media_query_list(
            &parse_media_query_list("(prefers-color-scheme: dark)"),
            &dark
        ));
        assert!(!evaluate_media_query_list(
            &parse_media_query_list("(prefers-color-scheme: light)"),
            &dark
        ));
        assert!(evaluate_media_query_list(
            &parse_media_query_list("(orientation: portrait)"),
            &dark
        ));
        assert!(!evaluate_media_query_list(
            &parse_media_query_list("(orientation: landscape)"),
            &dark
        ));
    }

    #[test]
    fn unsupported_media_feature_fails_closed() {
        let env = MediaEnvironment::default();
        assert!(!evaluate_media_query_list(
            &parse_media_query_list("(hover: hover)"),
            &env
        ));
    }

    #[test]
    fn font_face_from_pairs_reads_family_src_weight() {
        let face = font_face_from_pairs([
            ("font-family", "\"Display\""),
            (
                "src",
                "url(\"./a.woff2\") format(\"woff2\"), local(\"Arial\")",
            ),
            ("font-weight", "700"),
        ])
        .expect("face");
        assert_eq!(face.family, "Display");
        assert_eq!(face.weight, Some(700));
        assert_eq!(face.weight_end, None);
        assert_eq!(face.weight_span(), Some((700, 700)));
        assert_eq!(face.src[0], FontFaceSrc::Url("./a.woff2".into()));
        assert_eq!(face.src[1], FontFaceSrc::Local("Arial".into()));
        assert_eq!(font_face_url_src(&face), Some("./a.woff2"));
        assert_eq!(
            font_face_url_srcs(&face).collect::<Vec<_>>(),
            vec!["./a.woff2"]
        );
        assert!(
            load_font_face_bytes("https://example.com/a.woff2", None, Path::new(".")).is_none()
        );
    }

    #[test]
    fn font_face_weight_range_is_not_only_start() {
        let face = font_face_from_pairs([
            ("font-family", "Alimama FangYuanTi VF"),
            ("src", "url(\"./AlimamaFangYuanTiVF.ttf\")"),
            ("font-weight", "200 700"),
        ])
        .expect("face");
        assert_eq!(face.weight, Some(200));
        assert_eq!(face.weight_end, Some(700));
        assert_eq!(face.weight_span(), Some((200, 700)));
        let swapped = font_face_from_pairs([
            ("font-family", "Display"),
            ("src", "url(\"./a.ttf\")"),
            ("font-weight", "700 200"),
        ])
        .expect("swapped");
        assert_eq!(swapped.weight_span(), Some((200, 700)));
    }

    #[test]
    fn font_face_src_keeps_local_then_url_order() {
        let face = font_face_from_pairs([
            ("font-family", "Display"),
            (
                "src",
                r#"local("Noto Sans SC"), url("./Display.woff2") format("woff2")"#,
            ),
        ])
        .expect("face");
        assert_eq!(face.src[0], FontFaceSrc::Local("Noto Sans SC".into()));
        assert_eq!(face.src[1], FontFaceSrc::Url("./Display.woff2".into()));
        assert_eq!(
            font_face_url_srcs(&face).collect::<Vec<_>>(),
            vec!["./Display.woff2"]
        );
    }

    #[test]
    fn font_face_src_keeps_urls_after_format_and_tech() {
        let face = font_face_from_pairs([
            ("font-family", "Display"),
            (
                "src",
                r#"url("./missing.woff2") format("woff2") tech("color-COLRv0"), url("./ok.ttf") format("truetype")"#,
            ),
        ])
        .expect("face");
        assert_eq!(
            font_face_url_srcs(&face).collect::<Vec<_>>(),
            vec!["./missing.woff2", "./ok.ttf"]
        );
    }

    #[test]
    fn font_face_src_tries_next_url_when_first_missing() {
        let jail =
            std::env::temp_dir().join(format!("nanaui-font-src-fallback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&jail);
        std::fs::create_dir_all(&jail).expect("jail");
        std::fs::write(jail.join("ok.ttf"), b"dummy-font-bytes").expect("ttf");
        let face = font_face_from_pairs([
            ("font-family", "Display"),
            (
                "src",
                r#"url("./missing.woff2") format("woff2"), url("./ok.ttf") format("truetype")"#,
            ),
        ])
        .expect("face");
        let mut loaded = None;
        for url in font_face_url_srcs(&face) {
            if let Some(pair) = load_font_face_bytes(url, None, &jail) {
                loaded = Some((url.to_string(), pair));
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&jail);
        let (url, (bytes, path)) = loaded.expect("second url must load");
        assert_eq!(url, "./ok.ttf");
        assert_eq!(bytes, b"dummy-font-bytes");
        assert!(
            path.ends_with("ok.ttf"),
            "canonical path should be the second src: {}",
            path.display()
        );
    }

    #[test]
    fn import_layer_and_supports_are_unsupported() {
        assert!(matches!(
            parse_import_prelude("\"a.css\" layer"),
            Some(ImportPrelude::Unsupported)
        ));
        assert!(matches!(
            parse_import_prelude("url(\"a.css\") layer(utilities)"),
            Some(ImportPrelude::Unsupported)
        ));
        assert!(matches!(
            parse_import_prelude("url(a.css) supports(display: grid)"),
            Some(ImportPrelude::Unsupported)
        ));
        assert!(matches!(
            parse_import_prelude("\"a.css\" (min-width: 1px)"),
            Some(ImportPrelude::Ready { .. })
        ));
        assert!(matches!(
            parse_import_prelude("layer url(\"a.css\")"),
            Some(ImportPrelude::Unsupported)
        ));
        assert!(matches!(
            parse_import_prelude("url(\"layer.css\")"),
            Some(ImportPrelude::Ready { href, .. }) if href == "layer.css"
        ));
        assert!(matches!(
            parse_import_prelude(r#"url("a.css") supp\6frts(display: grid)"#),
            Some(ImportPrelude::Unsupported)
        ));
        assert!(matches!(
            parse_import_prelude(r#"url("a.css") l\61yer"#),
            Some(ImportPrelude::Unsupported)
        ));
        assert!(matches!(
            parse_import_prelude(r#"url("a.css") \73upports(display:flex)"#),
            Some(ImportPrelude::Unsupported)
        ));
    }

    #[test]
    fn supports_tiny_predicate_subset() {
        assert_eq!(evaluate_supports_condition("(display: flex)"), Some(true));
        assert_eq!(evaluate_supports_condition("(display:grid)"), Some(true));
        assert_eq!(evaluate_supports_condition("(display: block)"), Some(true));
        assert_eq!(evaluate_supports_condition("(width: 10px)"), Some(true));
        assert_eq!(evaluate_supports_condition("(color: red)"), Some(true));
        assert_eq!(
            evaluate_supports_condition("(color: lab(0% 0 0))"),
            Some(false)
        );
        assert_eq!(
            evaluate_supports_condition("(color: color(display-p3 0 0 0))"),
            Some(false)
        );
        assert_eq!(evaluate_supports_condition("(display: table)"), Some(false));
        assert!(evaluate_supports_condition("(gap: 8px)").is_none());
        assert!(evaluate_supports_condition("selector(.a)").is_none());
        assert_eq!(
            evaluate_supports_condition("not (color: lab(0% 0 0))"),
            Some(true)
        );
        assert_eq!(
            evaluate_supports_condition("(display: flex) and (width: 1px)"),
            Some(true)
        );
        assert!(
            evaluate_supports_condition("(display: flex) and (gap: 8px)").is_none(),
            "unknown predicate in and must fail closed"
        );
    }

    #[test]
    fn layer_prelude_names_and_anonymous() {
        assert_eq!(
            parse_layer_prelude(""),
            Some(LayerPrelude { names: vec![] })
        );
        assert_eq!(
            parse_layer_prelude("base"),
            Some(LayerPrelude {
                names: vec!["base".into()]
            })
        );
        assert_eq!(
            parse_layer_prelude("framework.layout, utilities"),
            Some(LayerPrelude {
                names: vec!["framework.layout".into(), "utilities".into()]
            })
        );
        assert!(parse_layer_prelude("foo bar").is_none());
        assert!(parse_layer_prelude("supports(display: flex)").is_none());
        assert_eq!(
            parse_layer_prelude("layer"),
            Some(LayerPrelude {
                names: vec!["layer".into()]
            })
        );
    }

    #[test]
    fn protocol_relative_and_unc_hrefs_are_blocked() {
        assert!(is_blocked_href("//evil.example/a.css"));
        assert!(is_blocked_href(r"\\evil.example\share\a.css"));
        assert!(is_blocked_href("%2f%2fevil.example/a.css"));
        assert!(is_blocked_href("https://example.com/a.css"));
        assert!(is_blocked_href("data:text/css,body{}"));
        assert!(!is_blocked_href("./a.css"));
        assert!(load_stylesheet_file("//evil.example/a.css", None, Path::new(".")).is_none());
        assert!(matches!(
            parse_import_prelude("url(\"//evil.example/a.css\")"),
            Some(ImportPrelude::Ready { href, .. }) if href == "//evil.example/a.css"
        ));
    }

    #[test]
    fn registered_font_cap_is_sixteen_mib_cumulative() {
        assert_eq!(MAX_REGISTERED_FONT_BYTES, 16 * 1024 * 1024);
        assert!(!font_registration_would_exceed_cap(0, MAX_FONT_FACE_BYTES));
        assert!(!font_registration_would_exceed_cap(
            MAX_FONT_FACE_BYTES,
            MAX_FONT_FACE_BYTES
        ));
        assert!(font_registration_would_exceed_cap(
            MAX_REGISTERED_FONT_BYTES,
            1
        ));
        assert!(font_registration_would_exceed_cap(
            0,
            MAX_FONT_FACE_BYTES + 1
        ));
    }

    #[test]
    fn memory_href_resolves_relative_to_declaring_sheet() {
        assert_eq!(
            resolve_memory_href("./fonts/n.ttf", Some("sheets/theme.css")),
            "sheets/fonts/n.ttf"
        );
        assert_eq!(resolve_memory_href("./n.ttf", Some("theme.css")), "n.ttf");
    }

    #[test]
    fn font_read_size_cap_skips_oversize() {
        let jail = std::env::temp_dir().join(format!("nanaui-font-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&jail);
        std::fs::create_dir_all(&jail).expect("jail");
        std::fs::write(jail.join("big.ttf"), vec![0u8; 64]).expect("font");
        let missed = with_font_face_byte_cap(8, || load_font_face_bytes("./big.ttf", None, &jail));
        let _ = std::fs::remove_dir_all(&jail);
        assert!(missed.is_none());
    }
}
