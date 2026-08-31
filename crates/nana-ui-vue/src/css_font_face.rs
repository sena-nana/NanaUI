//! `@font-face` subset parser, collected by [`crate::css_cascade::parse_stylesheet_full`].
//!
//! Unknown descriptors are ignored. This module does not fetch, touch
//! FontSystem, or depend on `nana-ui`. Host ingest runs at `inject_stylesheet`
//! when `scene-view` is on. This is not a CSSOM `CSSFontFaceRule`.

/// One `@font-face` block after a successful parse of the supported descriptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRule {
    pub font_family: Option<String>,
    pub src: Vec<FontFaceSrc>,
    /// CSS `font-weight` 100..=900 when the descriptor parsed.
    pub font_weight: Option<u16>,
    /// `normal` / `italic` / `oblique`. Other values skipped.
    pub font_style: Option<FontFaceStyle>,
}

/// `src` list entry (`url(...)` or `local(...)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceSrc {
    pub kind: FontFaceSrcKind,
    pub value: String,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFaceSrcKind {
    Url,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFaceStyle {
    Normal,
    Italic,
    Oblique,
}

/// Parse every `@font-face` block in `css`. Other at-rules and style rules are
/// skipped. Comments are stripped first.
pub fn parse_font_face_rules(css: &str) -> Vec<FontFaceRule> {
    let stripped = strip_css_comments(css);
    let mut rules = Vec::new();
    let mut rest = stripped.as_str();
    while let Some(at) = rest.find('@') {
        rest = &rest[at..];
        if !starts_with_font_face(rest) {
            rest = &rest[1..];
            continue;
        }
        let after_name = rest["@font-face".len()..].trim_start();
        let Some(body_start) = after_name.find('{') else {
            break;
        };
        let Some((body, next)) = split_block(&after_name[body_start..]) else {
            break;
        };
        if let Some(rule) = parse_font_face_body(body) {
            rules.push(rule);
        }
        rest = next;
    }
    rules
}

/// Parse a leading `@font-face { … }` at-rule, returning the rule and the rest
/// of the stylesheet. `None` if this is not `@font-face` or the block has no
/// supported descriptors (caller then counts the at-rule as skipped).
pub fn parse_font_face_at_rule(css: &str) -> Option<(FontFaceRule, &str)> {
    let rest = css.trim_start();
    if !starts_with_font_face(rest) {
        return None;
    }
    let after_name = rest["@font-face".len()..].trim_start();
    if !after_name.starts_with('{') {
        return None;
    }
    let (body, next) = split_block(after_name)?;
    let rule = parse_font_face_body(body)?;
    Some((rule, next))
}

fn starts_with_font_face(s: &str) -> bool {
    let lower = s.get(..10).map(|h| h.eq_ignore_ascii_case("@font-face"));
    lower == Some(true)
        && s.get(10..)
            .and_then(|tail| tail.chars().next())
            .is_none_or(|c| c.is_ascii_whitespace() || c == '{')
}

fn parse_font_face_body(body: &str) -> Option<FontFaceRule> {
    let mut rule = FontFaceRule {
        font_family: None,
        src: Vec::new(),
        font_weight: None,
        font_style: None,
    };
    let mut saw_supported = false;
    for decl in split_decls(body) {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((raw_key, raw_val)) = decl.split_once(':') else {
            continue;
        };
        let key = raw_key.trim().to_ascii_lowercase();
        let val = raw_val.trim();
        match key.as_str() {
            "font-family" => {
                if let Some(name) = parse_family_name(val) {
                    rule.font_family = Some(name);
                    saw_supported = true;
                }
            }
            "src" => {
                let src = parse_src_list(val);
                if !src.is_empty() {
                    rule.src = src;
                    saw_supported = true;
                }
            }
            "font-weight" => {
                if let Some(weight) = parse_face_weight(val) {
                    rule.font_weight = Some(weight);
                    saw_supported = true;
                }
            }
            "font-style" => {
                if let Some(style) = parse_face_style(val) {
                    rule.font_style = Some(style);
                    saw_supported = true;
                }
            }
            _ => {}
        }
    }
    if saw_supported { Some(rule) } else { None }
}

fn parse_family_name(input: &str) -> Option<String> {
    let name = strip_quotes(input.trim());
    if name.is_empty() || name.eq_ignore_ascii_case("inherit") {
        return None;
    }
    Some(name)
}

fn parse_face_weight(input: &str) -> Option<u16> {
    match input.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        other => {
            let n: f32 = other.parse().ok()?;
            if !(1.0..=1000.0).contains(&n) {
                return None;
            }
            Some(((n / 100.0).round() as i32 * 100).clamp(100, 900) as u16)
        }
    }
}

fn parse_face_style(input: &str) -> Option<FontFaceStyle> {
    match input.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(FontFaceStyle::Normal),
        "italic" => Some(FontFaceStyle::Italic),
        "oblique" => Some(FontFaceStyle::Oblique),
        _ => None,
    }
}

fn parse_src_list(input: &str) -> Vec<FontFaceSrc> {
    let mut out = Vec::new();
    for item in split_comma_respecting_parens(input) {
        if let Some(src) = parse_src_item(item) {
            out.push(src);
        }
    }
    out
}

fn parse_src_item(item: &str) -> Option<FontFaceSrc> {
    let item = item.trim();
    if item.is_empty() {
        return None;
    }
    let lower = item.to_ascii_lowercase();
    let (kind, after_fn) = if let Some(rest) = strip_func(&lower, item, "url") {
        (FontFaceSrcKind::Url, rest)
    } else if let Some(rest) = strip_func(&lower, item, "local") {
        (FontFaceSrcKind::Local, rest)
    } else {
        return None;
    };
    let (value_raw, tail) = split_func_arg(after_fn)?;
    let value = strip_quotes(value_raw.trim());
    if value.is_empty() {
        return None;
    }
    let format = parse_format_hint(tail);
    Some(FontFaceSrc {
        kind,
        value,
        format,
    })
}

fn strip_func<'a>(lower: &str, original: &'a str, name: &str) -> Option<&'a str> {
    if !lower.starts_with(name) {
        return None;
    }
    let rest = original.get(name.len()..)?.trim_start();
    rest.strip_prefix('(')
}

fn split_func_arg(after_open_paren: &str) -> Option<(&str, &str)> {
    let mut depth = 1u32;
    let mut in_quote: Option<char> = None;
    for (i, ch) in after_open_paren.char_indices() {
        match (ch, in_quote) {
            ('"' | '\'', None) => in_quote = Some(ch),
            (q, Some(open)) if q == open => in_quote = None,
            ('(', None) => depth += 1,
            (')', None) => {
                depth -= 1;
                if depth == 0 {
                    return Some((&after_open_paren[..i], &after_open_paren[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_format_hint(tail: &str) -> Option<String> {
    let tail = tail.trim();
    let lower = tail.to_ascii_lowercase();
    let rest = strip_func(&lower, tail, "format")?;
    let (arg, _) = split_func_arg(rest)?;
    let name = strip_quotes(arg.trim());
    if name.is_empty() { None } else { Some(name) }
}

fn split_decls(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_quote: Option<char> = None;
    for (i, ch) in input.char_indices() {
        match (ch, in_quote) {
            ('"' | '\'', None) => in_quote = Some(ch),
            (q, Some(open)) if q == open => in_quote = None,
            ('(', None) => depth += 1,
            (')', None) => depth -= 1,
            (';', None) if depth == 0 => {
                out.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start <= input.len() {
        out.push(&input[start..]);
    }
    out
}

fn split_comma_respecting_parens(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_quote: Option<char> = None;
    for (i, ch) in input.char_indices() {
        match (ch, in_quote) {
            ('"' | '\'', None) => in_quote = Some(ch),
            (q, Some(open)) if q == open => in_quote = None,
            ('(', None) => depth += 1,
            (')', None) => depth -= 1,
            (',', None) if depth == 0 => {
                out.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start <= input.len() {
        out.push(&input[start..]);
    }
    out
}

fn split_block(at_brace: &str) -> Option<(&str, &str)> {
    let rest = at_brace.strip_prefix('{')?;
    let mut depth = 1i32;
    for (i, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((rest[..i].trim(), rest[i + 1..].trim_start()));
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_quotes(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 {
        let bytes = t.as_bytes();
        if (bytes[0] == b'"' && bytes[t.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[t.len() - 1] == b'\'')
        {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev_star = false;
            for c in chars.by_ref() {
                if prev_star && c == '/' {
                    break;
                }
                prev_star = c == '*';
            }
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_and_local_src() {
        let rules = parse_font_face_rules(
            r#"
            /* product UI face */
            @font-face {
                font-family: "Host Sans";
                src: url("/fonts/HostSans.woff2") format("woff2"),
                     local("Host Sans");
                font-weight: 500;
                font-style: normal;
                font-display: swap;
            }
            "#,
        );
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert_eq!(rule.font_family.as_deref(), Some("Host Sans"));
        assert_eq!(rule.font_weight, Some(500));
        assert_eq!(rule.font_style, Some(FontFaceStyle::Normal));
        assert_eq!(rule.src.len(), 2);
        assert_eq!(rule.src[0].kind, FontFaceSrcKind::Url);
        assert_eq!(rule.src[0].value, "/fonts/HostSans.woff2");
        assert_eq!(rule.src[0].format.as_deref(), Some("woff2"));
        assert_eq!(rule.src[1].kind, FontFaceSrcKind::Local);
        assert_eq!(rule.src[1].value, "Host Sans");
    }

    #[test]
    fn skips_unknown_at_rules_and_empty_faces() {
        let rules = parse_font_face_rules(
            r#"
            @media screen { .x { color: red; } }
            @font-face { font-display: swap; unicode-range: U+0000; }
            @font-face { font-family: App; src: url(app.ttf); }
            "#,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].font_family.as_deref(), Some("App"));
        assert_eq!(rules[0].src[0].value, "app.ttf");
    }

    #[test]
    fn does_not_hook_style_rules() {
        let rules = parse_font_face_rules(".title { font-family: Host Sans; }");
        assert!(rules.is_empty());
    }

    #[test]
    fn stylesheet_full_font_face_collects_without_io() {
        let css = r#"
            @font-face {
                font-family: "Host Sans";
                src: url("data:font/ttf;base64,AAAA") format("truetype");
                font-display: swap;
                unicode-range: U+0000-00FF;
            }
            .title { font-family: "Host Sans", sans-serif; }
        "#;
        let (sheet, report) = crate::css_cascade::parse_stylesheet_full(css, 0);
        assert_eq!(report.skipped_at_rules, 0);
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "Host Sans");
        assert_eq!(
            sheet.font_faces[0].src[0],
            crate::css_at_rule::FontFaceSrc::Url("data:font/ttf;base64,AAAA".into())
        );
    }

    #[test]
    fn font_face_unknown_only_descriptors_count_as_skipped() {
        let (sheet, report) = crate::css_cascade::parse_stylesheet_full(
            "@font-face { font-display: swap; unicode-range: U+0000; }",
            0,
        );
        assert!(sheet.font_faces.is_empty());
        assert_eq!(report.skipped_at_rules, 1);
    }
}

#[cfg(all(test, feature = "scene-view"))]
mod host_ingest_tests {
    use super::*;
    use crate::bridge::MessageBridge;
    use std::sync::OnceLock;

    fn noto_data_url() -> &'static str {
        static URL: OnceLock<String> = OnceLock::new();
        URL.get_or_init(|| {
            use base64::Engine as _;
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../nana-ui/assets/fonts/NotoSansSC-Regular.ttf"),
            )
            .expect("bundled Noto Sans SC");
            format!(
                "data:font/ttf;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        })
        .as_str()
    }

    #[test]
    fn inject_font_face_aliases_css_family_onto_loaded_face() {
        let css = format!(
            r#"
            @font-face {{
                font-family: "Host Sans";
                src: url("{src}") format("truetype");
                font-weight: 400;
                font-display: swap;
            }}
            .title {{ font-family: "Host Sans", sans-serif; }}
            "#,
            src = noto_data_url()
        );
        let mut bridge = MessageBridge::new();
        bridge.inject_stylesheet(&css);
        let used = nana_ui::shaped_face_families("Host Sans", "H");
        assert!(
            used.iter().any(|name| name == "Host Sans"),
            "CSS family must map to the loaded face, used={used:?}"
        );
        assert!(
            used.iter().any(|name| name.contains("Noto")),
            "used face name table must stay Noto (≠ Host Sans), used={used:?}"
        );
    }

    #[test]
    fn inject_font_face_bad_src_is_not_registered() {
        let mut bridge = MessageBridge::new();
        bridge.inject_stylesheet(
            r#"
            @font-face {
                font-family: "Ghost Face";
                src: url("data:font/ttf;base64,AAAA");
            }
            "#,
        );
        let used = nana_ui::shaped_face_families("Ghost Face", "H");
        assert!(
            !used.iter().any(|name| name == "Ghost Face"),
            "bad src must not alias a family, used={used:?}"
        );
    }
}
