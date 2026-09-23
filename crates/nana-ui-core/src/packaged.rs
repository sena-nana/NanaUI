//! Packaged resources: `nana://res/<logical path>` (Issue #226).
//!
//! A packaged application ships its resources in `.nrpack` files under the
//! runtime resources location; a development build reads the same logical
//! paths from a directory. Stylesheets, `@font-face` and `url()` images name
//! them by logical path and never learn which: the process installs one
//! [`PackagedResourceSource`] at startup (`nana_ui::NanaApplicationBuilder`)
//! and every loader goes through [`read_packaged`].
//!
//! The URL is also the serialized form a future unified Resource contract
//! (Issues #148 / #149) keeps for package-backed resources.

use std::sync::{Arc, OnceLock};

/// Scheme and authority of packaged resource URLs. (`nana://app` is the
/// Vue/JS document origin and is unrelated.)
pub const PACKAGED_URL_PREFIX: &str = "nana://res/";

/// Why a packaged read failed. Callers treat every variant as "resource not
/// available"; the source records the details in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackagedReadError {
    /// No source is installed, or no pack holds the path.
    NotFound,
    /// Larger than the caller's limit (checked before reading).
    TooLarge,
    /// The pack's content key is not available.
    KeyUnavailable,
    /// Signature, hash or authentication failure; nothing was returned.
    Integrity,
    Io,
}

/// Reads logical paths. Implemented by the pack mount in `nana-ui` and by
/// the loose-directory source used in development.
pub trait PackagedResourceSource: Send + Sync + 'static {
    fn read(&self, logical_path: &str, max_bytes: u64) -> Result<Vec<u8>, PackagedReadError>;
}

static SOURCE: OnceLock<Arc<dyn PackagedResourceSource>> = OnceLock::new();

/// Install the process's source. The first call wins; later calls return
/// `false` and leave it in place.
pub fn install_packaged_source(source: Arc<dyn PackagedResourceSource>) -> bool {
    SOURCE.set(source).is_ok()
}

pub fn packaged_source_installed() -> bool {
    SOURCE.get().is_some()
}

/// `true` for `nana://res/...` (ASCII case-insensitive scheme).
pub fn is_packaged_url(href: &str) -> bool {
    href.trim()
        .get(..PACKAGED_URL_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PACKAGED_URL_PREFIX))
}

/// Resolve `href` to a logical path.
///
/// - `nana://res/ui/logo.png` → `ui/logo.png`
/// - `../img/a.png` with `from = nana://res/ui/css/app.css` → `ui/img/a.png`
///
/// Returns `None` for anything else, including a relative `href` whose
/// `from` is not packaged, and for paths that would leave the resource root
/// (`..` above it, percent-encoded or not), contain a backslash, NUL or
/// control character, or an empty segment. Query and fragment are dropped.
pub fn packaged_logical_path(href: &str, from: Option<&str>) -> Option<String> {
    let href = href.trim();
    let href = href.split(['?', '#']).next().unwrap_or_default();
    let (base, relative): (Vec<String>, &str) = if is_packaged_url(href) {
        (Vec::new(), &href[PACKAGED_URL_PREFIX.len()..])
    } else {
        let from = from.filter(|from| is_packaged_url(from))?;
        if href.is_empty() || href.contains(':') || href.starts_with('/') {
            return None;
        }
        let directory = packaged_logical_path(from, None)?;
        let mut segments: Vec<String> = directory.split('/').map(str::to_owned).collect();
        segments.pop(); // the importing file itself
        (segments, href)
    };
    let decoded = decode(relative)?;
    let mut segments = base;
    for segment in decoded.split('/') {
        match segment {
            "" => return None,
            "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other.to_owned()),
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

fn decode(text: &str) -> Option<String> {
    let decoded = if text.contains('%') {
        String::from_utf8(crate::url_jail::percent_decode_bytes(text)?).ok()?
    } else {
        text.to_owned()
    };
    (!decoded.contains('\\') && !decoded.chars().any(char::is_control)).then_some(decoded)
}

/// Read a packaged resource named by `href` (resolved against `from`).
/// `None` when it is not a packaged URL, no source is installed, or the read
/// failed for any reason.
pub fn read_packaged(href: &str, from: Option<&str>, max_bytes: u64) -> Option<Vec<u8>> {
    let path = packaged_logical_path(href, from)?;
    SOURCE.get()?.read(&path, max_bytes).ok()
}

/// Canonical `nana://res/` URL of a logical path, for dedup keys and as the
/// `from` of nested references. Characters that URL parsing would change
/// (`%`, `?`, `#`, space) or that would end a CSS `url(...)` token or string
/// (`(`, `)`, `"`, `'`) are percent-encoded, so
/// `packaged_logical_path(&packaged_url(p), None) == Some(p)` for every valid
/// logical path.
pub fn packaged_url(logical_path: &str) -> String {
    let mut url = String::with_capacity(PACKAGED_URL_PREFIX.len() + logical_path.len());
    url.push_str(PACKAGED_URL_PREFIX);
    for c in logical_path.chars() {
        match c {
            '%' => url.push_str("%25"),
            '?' => url.push_str("%3F"),
            '#' => url.push_str("%23"),
            ' ' => url.push_str("%20"),
            '(' => url.push_str("%28"),
            ')' => url.push_str("%29"),
            '"' => url.push_str("%22"),
            '\'' => url.push_str("%27"),
            // `href.trim()` would eat any other whitespace at the ends.
            other if other.is_whitespace() => {
                let mut buf = [0u8; 4];
                for byte in other.encode_utf8(&mut buf).bytes() {
                    url.push_str(&format!("%{byte:02X}"));
                }
            }
            other => url.push(other),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_and_relative_urls_resolve() {
        assert_eq!(
            packaged_logical_path("nana://res/ui/logo.png", None).as_deref(),
            Some("ui/logo.png")
        );
        assert_eq!(
            packaged_logical_path("NANA://RES/ui/a.png?v=2#x", None).as_deref(),
            Some("ui/a.png")
        );
        let css = Some("nana://res/ui/css/app.css");
        assert_eq!(
            packaged_logical_path("../img/a.png", css).as_deref(),
            Some("ui/img/a.png")
        );
        assert_eq!(
            packaged_logical_path("./b.css", css).as_deref(),
            Some("ui/css/b.css")
        );
        assert_eq!(
            packaged_logical_path("fonts/x%20y.ttf", css).as_deref(),
            Some("ui/css/fonts/x y.ttf")
        );
    }

    #[test]
    fn canonical_urls_round_trip() {
        for path in [
            "ui/a#1.png",
            "ui/x%41.png",
            "ui/what?.css",
            "ui/with space.ttf",
            "ui/ü.png",
            "ui/icon (1).png",
            "ui/it's \"q\".png",
            "ui/a\u{3000}",
            "ui/b\u{a0}",
        ] {
            let url = packaged_url(path);
            assert_eq!(
                packaged_logical_path(&url, None).as_deref(),
                Some(path),
                "{url}"
            );
            // And as the base of a relative reference.
            assert_eq!(
                packaged_logical_path("b.png", Some(&url)).as_deref(),
                Some("ui/b.png")
            );
        }
    }

    #[test]
    fn escapes_and_foreign_urls_are_rejected() {
        let css = Some("nana://res/ui/app.css");
        for bad in [
            "nana://res/../x",
            "nana://res/a//b",
            "nana://res/",
            "nana://res/a\\b",
            "nana://res/a/%2e%2e/%2e%2e/x",
            "nana://res/a%00b",
        ] {
            assert_eq!(packaged_logical_path(bad, None), None, "{bad}");
        }
        assert_eq!(packaged_logical_path("../../x", css), None);
        assert_eq!(packaged_logical_path("/etc/passwd", css), None);
        assert_eq!(packaged_logical_path("file:///etc/passwd", css), None);
        assert_eq!(packaged_logical_path("a.png", Some("/tmp/app.css")), None);
        assert_eq!(packaged_logical_path("nana://app/main.js", None), None);
    }
}
