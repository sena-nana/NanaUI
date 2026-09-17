//! Driving a corpus case through the native pipeline.
//!
//! The reference engine builds a fresh `fontdb` per case and loads exactly the
//! fixtures the case declares, in declaration order. These helpers set the
//! native [`FontSystem`] up the same way, so a difference between the two is a
//! difference in shaping or layout rather than in which faces were available.

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontBlob, FontSystem, font_blob};
use nana_text::parity::CorpusCase;
use nana_text::{TextSource, TextSpan, TextStyle};
use nana_ui_core::fonts::UI_FONT_REGULAR;
use std::path::PathBuf;
use std::sync::Arc;

pub fn fixture_family(id: &str) -> &'static str {
    match id {
        "noto-sans-sc" => "Noto Sans SC",
        "nana-test-vf" => "NanaTestVF",
        "noto-sans-arabic" => "Noto Sans Arabic",
        "noto-sans-kr" => "Noto Sans KR",
        "noto-emoji" => "Noto Emoji",
        other => panic!("unknown corpus font {other}"),
    }
}

pub fn fixture_bytes(id: &str) -> FontBlob {
    if id == "noto-sans-sc" {
        return font_blob(UI_FONT_REGULAR);
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fonts")
        .join(format!("{id}.ttf"));
    font_blob(std::fs::read(path).unwrap())
}

/// A font system holding exactly the case's fixtures, in declaration order,
/// with no platform fallback policy.
pub fn hermetic_fonts(case: &CorpusCase) -> FontSystem {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    for id in &case.fonts {
        fonts
            .register_bytes(fixture_bytes(id), &FaceDescriptor::default())
            .unwrap();
    }
    fonts
}

/// The case's style with its fallback chain spelled out: the requested family
/// (or the first declared font, which is what the reference defaults to), then
/// every declared font in declaration order.
pub fn with_chain(style: &TextStyle, case: &CorpusCase) -> TextStyle {
    let mut families: Vec<String> = Vec::new();
    let first = style
        .font_family
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| fixture_family(&case.fonts[0]).to_string());
    families.push(format!("\"{first}\""));
    for id in &case.fonts {
        families.push(format!("\"{}\"", fixture_family(id)));
    }
    TextStyle {
        font_family: Some(Arc::from(families.join(", "))),
        ..style.clone()
    }
}

/// The case's text and spans as a source, with the fallback chain applied to
/// every span and authored newlines folded exactly as the constraints ask.
pub fn case_source(case: &CorpusCase) -> TextSource {
    let mut source = TextSource::new(case.text.clone());
    let spans: Vec<TextSpan> = case
        .spans
        .iter()
        .map(|span| TextSpan {
            style: with_chain(&span.style, case),
            ..span.clone()
        })
        .collect();
    if !spans.is_empty() {
        source.set_spans(spans);
    }
    if case.constraints.preserve_lines {
        return source;
    }
    match source.with_folded_newlines() {
        Some(folded) => folded.clone(),
        None => source,
    }
}
