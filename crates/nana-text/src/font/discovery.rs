//! Face metadata and system font discovery via `fontdb`. The only file allowed
//! to name it.
//!
//! `fontdb` is used for exactly two things: walking the platform's font
//! directories, and reading the name / OS/2 metadata of a face. Every face the
//! font system holds — system, file or memory — goes through [`parse_faces`]
//! or [`system_faces`], so there is one interpretation of "what family and
//! weight is this face", not one per source kind. `fontdb`'s own query and
//! fallback are not used; matching is Nana's (see `matching.rs`).

use super::query::FontStyle;
use super::system::FontBlob;
use std::path::PathBuf;
use std::sync::Arc;

/// Metadata of one face, independent of where its bytes live.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceMeta {
    pub index: u32,
    /// English (US) family first when the face has one, then every other
    /// localized family name, de-duplicated.
    pub families: Vec<Arc<str>>,
    pub post_script_name: Arc<str>,
    pub weight: f32,
    /// Percentage of normal width.
    pub stretch: f32,
    pub style: FontStyle,
    pub monospaced: bool,
}

/// Where a discovered system face's bytes live.
pub enum DiscoveredSource {
    File(PathBuf),
    Blob(FontBlob),
}

fn stretch_percent(stretch: fontdb::Stretch) -> f32 {
    match stretch {
        fontdb::Stretch::UltraCondensed => 50.0,
        fontdb::Stretch::ExtraCondensed => 62.5,
        fontdb::Stretch::Condensed => 75.0,
        fontdb::Stretch::SemiCondensed => 87.5,
        fontdb::Stretch::Normal => 100.0,
        fontdb::Stretch::SemiExpanded => 112.5,
        fontdb::Stretch::Expanded => 125.0,
        fontdb::Stretch::ExtraExpanded => 150.0,
        fontdb::Stretch::UltraExpanded => 200.0,
    }
}

fn meta(info: &fontdb::FaceInfo) -> FaceMeta {
    let mut families: Vec<Arc<str>> = Vec::with_capacity(info.families.len());
    for (name, _) in &info.families {
        if !families
            .iter()
            .any(|known| known.eq_ignore_ascii_case(name))
        {
            families.push(Arc::from(name.as_str()));
        }
    }
    FaceMeta {
        index: info.index,
        families,
        post_script_name: Arc::from(info.post_script_name.as_str()),
        weight: f32::from(info.weight.0),
        stretch: stretch_percent(info.stretch),
        style: match info.style {
            fontdb::Style::Normal => FontStyle::Normal,
            fontdb::Style::Italic => FontStyle::Italic,
            fontdb::Style::Oblique => FontStyle::Oblique,
        },
        monospaced: info.monospaced,
    }
}

/// Every face in a font file or collection, in collection-index order. Empty
/// when the bytes are not a font.
pub fn parse_faces(blob: &FontBlob) -> Vec<FaceMeta> {
    let mut db = fontdb::Database::new();
    let ids = db.load_font_source(fontdb::Source::Binary(Arc::clone(blob)));
    let mut faces: Vec<FaceMeta> = ids.iter().filter_map(|id| db.face(*id)).map(meta).collect();
    faces.sort_by_key(|face| face.index);
    faces
}

/// Every face the platform's font directories provide, in `fontdb`'s scan
/// order (stable for a given machine and font install).
pub fn system_faces() -> Vec<(DiscoveredSource, FaceMeta)> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    #[cfg(target_os = "android")]
    db.load_fonts_dir("/system/fonts");
    db.faces()
        .map(|info| {
            let source = match &info.source {
                fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                    DiscoveredSource::File(path.clone())
                }
                fontdb::Source::Binary(data) => DiscoveredSource::Blob(Arc::clone(data)),
            };
            (source, meta(info))
        })
        .collect()
}
