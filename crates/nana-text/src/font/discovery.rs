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
#[cfg(any(test, target_env = "ohos"))]
use std::path::Path;
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
            fontdb::Style::Normal => named_style(&info.post_script_name),
            fontdb::Style::Italic => FontStyle::Italic,
            fontdb::Style::Oblique => FontStyle::Oblique,
        },
        monospaced: info.monospaced,
    }
}

/// The slope a PostScript name's style part (after the last `-`) spells,
/// for faces whose OS/2 table leaves the italic and oblique bits clear:
/// Apple's `HelveticaNeue-MediumItalic` and `-ThinItalic` do, and would
/// otherwise tie with the upright face and win the tie as the later one.
fn named_style(post_script_name: &str) -> FontStyle {
    let Some((_, style)) = post_script_name.rsplit_once('-') else {
        return FontStyle::Normal;
    };
    if style.ends_with("Italic") {
        FontStyle::Italic
    } else if style.ends_with("Oblique") {
        FontStyle::Oblique
    } else {
        FontStyle::Normal
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
    // OHOS reports `target_os = "linux"`; fontdb's Linux scan (fontconfig XML,
    // `/usr/share/fonts`, `$HOME`) names nothing on the device.
    #[cfg(target_env = "ohos")]
    load_ohos_fonts(&mut db, Path::new("/"));
    #[cfg(not(target_env = "ohos"))]
    db.load_system_fonts();
    #[cfg(target_os = "android")]
    db.load_fonts_dir("/system/fonts");
    faces(&db)
}

/// The directories `root/system/etc/fontconfig.json` names, else
/// `root/system/fonts` (`super::ohos`).
#[cfg(any(test, target_env = "ohos"))]
fn load_ohos_fonts(db: &mut fontdb::Database, root: &Path) {
    let config = super::ohos::OhosFontConfig::read(root);
    for dir in super::ohos::font_dirs(root, config.as_ref()) {
        db.load_fonts_dir(dir);
    }
}

fn faces(db: &fontdb::Database) -> Vec<(DiscoveredSource, FaceMeta)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_postscript_style_part_names_the_slope_the_os2_bits_omit() {
        assert_eq!(named_style("HelveticaNeue-MediumItalic"), FontStyle::Italic);
        assert_eq!(named_style("Helvetica-Oblique"), FontStyle::Oblique);
        assert_eq!(named_style("HelveticaNeue-Medium"), FontStyle::Normal);
        // Only the style part counts, never the family part.
        assert_eq!(named_style("ItalicSans-Regular"), FontStyle::Normal);
        assert_eq!(named_style("NoStylePart"), FontStyle::Normal);
    }

    #[test]
    fn ohos_scan_reads_faces_from_the_configured_dir_under_the_root() {
        let root = std::env::temp_dir().join(format!("nana-text-ohos-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fonts = root.join("vendor/fonts");
        std::fs::create_dir_all(&fonts).unwrap();
        let config = root.join(super::super::ohos::CONFIG_PATH);
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, r#"{ "fontdir": ["/vendor/fonts/"] }"#).unwrap();
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/noto-sans-kr.ttf");
        std::fs::copy(fixture, fonts.join("NotoSansKR.ttf")).unwrap();
        // Not scanned: the config names another directory.
        let system = root.join(super::super::ohos::FONTS_DIR);
        std::fs::create_dir_all(&system).unwrap();
        std::fs::copy(fixture, system.join("Ignored.ttf")).unwrap();

        let mut db = fontdb::Database::new();
        load_ohos_fonts(&mut db, &root);
        let found = faces(&db);
        let _ = std::fs::remove_dir_all(&root);

        assert!(!found.is_empty());
        for (source, meta) in &found {
            let DiscoveredSource::File(path) = source else {
                panic!("a scanned face is file-backed");
            };
            assert_eq!(path.file_name().unwrap(), "NotoSansKR.ttf");
            assert!(
                meta.families
                    .iter()
                    .any(|family| &**family == "Noto Sans KR")
            );
        }
    }
}
