//! Backend-neutral `@font-face` registration contract.
//!
//! CSS parsing stays in `nana-ui-css`; host font loading stays in `nana-ui`.
//! This module defines the pure data descriptors shared across both layers so
//! compiled stylesheets and host font ingest speak one typed contract.

use serde::{Deserialize, Serialize};

/// Per-file byte cap for a single `@font-face` source (8 MiB, matching
/// [`crate::MAX_LOCAL_URL_BYTES`]).
pub const MAX_FONT_FACE_BYTES: u64 = crate::MAX_LOCAL_URL_BYTES;

/// Cumulative host-side byte cap across all registered `@font-face` sources
/// (16 MiB).
pub const MAX_REGISTERED_FONT_BYTES: u64 = 16 * 1024 * 1024;

/// Returns `true` if adding `add` bytes would exceed either the per-file cap
/// ([`MAX_FONT_FACE_BYTES`]) or the cumulative cap ([`MAX_REGISTERED_FONT_BYTES`]).
pub const fn font_registration_would_exceed_cap(used: u64, add: u64) -> bool {
    add > MAX_FONT_FACE_BYTES || used.saturating_add(add) > MAX_REGISTERED_FONT_BYTES
}

/// Slant style declared by a `@font-face` rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FontStyleSpec {
    #[default]
    Normal,
    Italic,
    Oblique,
}

/// One candidate source entry from a `@font-face` `src` list, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FontFaceSource {
    /// Local filesystem, packaged (`nana://res/`), or `data:` URL reference,
    /// with an optional format hint (e.g. `"woff2"`, `"truetype"`).
    Url {
        href: String,
        format: Option<String>,
    },
    /// Locally installed system face name (`local(...)`).
    Local(String),
}

impl FontFaceSource {
    /// Construct a URL source without a format hint.
    pub fn url(href: impl Into<String>) -> Self {
        Self::Url {
            href: href.into(),
            format: None,
        }
    }

    /// Construct a URL source with an explicit format hint.
    pub fn url_with_format(href: impl Into<String>, format: impl Into<String>) -> Self {
        Self::Url {
            href: href.into(),
            format: Some(format.into()),
        }
    }

    /// Construct a `local(...)` font source.
    pub fn local(name: impl Into<String>) -> Self {
        Self::Local(name.into())
    }

    /// The URL string if this is a [`Self::Url`] source.
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Self::Url { href, .. } => Some(href.as_str()),
            Self::Local(_) => None,
        }
    }

    /// The local face name if this is a [`Self::Local`] source.
    pub fn as_local(&self) -> Option<&str> {
        match self {
            Self::Local(name) => Some(name.as_str()),
            Self::Url { .. } => None,
        }
    }

    /// Optional format hint if this is a [`Self::Url`] source with one.
    pub fn format(&self) -> Option<&str> {
        match self {
            Self::Url { format, .. } => format.as_deref(),
            Self::Local(_) => None,
        }
    }
}

/// Typed `@font-face` descriptor payload shared between build-time compilers
/// and host font ingestion.
///
/// Mirrors the host registration contract (`nana_ui::ingest_host_font_faces`):
/// - `family` replaces the font face's embedded family names on registration.
/// - `weight` + `weight_end` define an inclusive weight range (e.g. `200..=700`);
///   `weight_end: None` registers a single weight equal to `weight`.
/// - `style` selects `Normal` / `Italic` / `Oblique` (`None` keeps the face's
///   default style).
/// - `base_href` records the originating stylesheet href for resolving relative
///   URL sources when not already absolutized.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct FontFaceRule {
    pub family: String,
    pub sources: Vec<FontFaceSource>,
    /// Start of the inclusive CSS `font-weight` range (1..=1000), or a single
    /// weight when [`Self::weight_end`] is `None`.
    pub weight: Option<u16>,
    /// Inclusive end of the CSS `font-weight` range. `None` means single weight.
    pub weight_end: Option<u16>,
    /// Optional slant style (`normal` / `italic` / `oblique`).
    pub style: Option<FontStyleSpec>,
    /// Canonical href of the declaring sheet for resolving relative URLs.
    pub base_href: Option<String>,
}

impl FontFaceRule {
    /// Construct a minimal rule with a family name and source list.
    pub fn new(family: impl Into<String>, sources: Vec<FontFaceSource>) -> Self {
        Self {
            family: family.into(),
            sources,
            weight: None,
            weight_end: None,
            style: None,
            base_href: None,
        }
    }

    /// Whether this rule carries a non-empty family name and at least one source.
    pub fn is_valid(&self) -> bool {
        !self.family.is_empty() && !self.sources.is_empty()
    }

    /// Inclusive `(min, max)` weight span, or `None` if no weight descriptor
    /// was specified.
    pub fn weight_span(&self) -> Option<(u16, u16)> {
        let start = self.weight?;
        let end = self.weight_end.unwrap_or(start);
        Some((start.min(end), start.max(end)))
    }

    /// Iterate `url(...)` source hrefs in declaration order, skipping `local(...)`.
    pub fn url_sources(&self) -> impl Iterator<Item = &str> {
        self.sources.iter().filter_map(FontFaceSource::as_url)
    }

    /// First `url(...)` source href in declaration order, if any.
    pub fn first_url(&self) -> Option<&str> {
        self.url_sources().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_span_handles_single_range_and_reversed_bounds() {
        let mut rule = FontFaceRule::new("Display", vec![FontFaceSource::url("./Display.ttf")]);
        assert_eq!(rule.weight_span(), None);

        rule.weight = Some(600);
        assert_eq!(rule.weight_span(), Some((600, 600)));

        rule.weight = Some(200);
        rule.weight_end = Some(700);
        assert_eq!(rule.weight_span(), Some((200, 700)));

        rule.weight = Some(800);
        rule.weight_end = Some(300);
        assert_eq!(rule.weight_span(), Some((300, 800)));
    }

    #[test]
    fn url_sources_preserves_order_and_skips_local() {
        let rule = FontFaceRule {
            family: "Brand".into(),
            sources: vec![
                FontFaceSource::local("Brand Local"),
                FontFaceSource::url_with_format("./brand.woff2", "woff2"),
                FontFaceSource::url("./brand.ttf"),
            ],
            weight: Some(400),
            weight_end: None,
            style: Some(FontStyleSpec::Italic),
            base_href: Some("nana://res/styles/app.css".into()),
        };

        assert!(rule.is_valid());
        assert_eq!(rule.sources[0].as_local(), Some("Brand Local"));
        assert_eq!(rule.sources[0].as_url(), None);
        assert_eq!(rule.sources[1].format(), Some("woff2"));
        assert_eq!(rule.sources[2].format(), None);
        assert_eq!(rule.first_url(), Some("./brand.woff2"));
        let urls: Vec<&str> = rule.url_sources().collect();
        assert_eq!(urls, vec!["./brand.woff2", "./brand.ttf"]);
    }

    #[test]
    fn validity_and_byte_caps() {
        assert!(!FontFaceRule::default().is_valid());
        assert!(!FontFaceRule::new("EmptySrc", Vec::new()).is_valid());
        assert!(!FontFaceRule::new("", vec![FontFaceSource::url("a.ttf")]).is_valid());

        assert!(!font_registration_would_exceed_cap(0, MAX_FONT_FACE_BYTES));
        assert!(font_registration_would_exceed_cap(
            0,
            MAX_FONT_FACE_BYTES + 1
        ));
        assert!(!font_registration_would_exceed_cap(
            MAX_REGISTERED_FONT_BYTES - 1024,
            1024
        ));
        assert!(font_registration_would_exceed_cap(
            MAX_REGISTERED_FONT_BYTES - 1024,
            1025
        ));
    }

    #[test]
    fn serde_roundtrip_preserves_all_fields() {
        let rule = FontFaceRule {
            family: "Mono".into(),
            sources: vec![
                FontFaceSource::local("JetBrains Mono"),
                FontFaceSource::url_with_format("data:font/ttf;base64,AAAA", "truetype"),
            ],
            weight: Some(300),
            weight_end: Some(700),
            style: Some(FontStyleSpec::Oblique),
            base_href: Some("file:///app/style.css".into()),
        };
        let json = serde_json::to_string(&rule).expect("serialize");
        let back: FontFaceRule = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, rule);
    }
}
