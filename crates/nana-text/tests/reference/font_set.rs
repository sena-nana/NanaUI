//! A hermetic font database, built per corpus case.
//!
//! The product path seeds one process-wide `FontSystem` from the system's
//! fonts. Goldens recorded against that would encode whatever happened to be
//! installed on one machine and would fail everywhere else, so the reference
//! engine never touches it: it builds an empty `fontdb::Database`, loads only
//! the faces a case declares, in the order the case declares them, and pins the
//! locale. Face ids are therefore deterministic and the fallback chain is
//! exactly the case's font list.

use cosmic_text::FontSystem;
use cosmic_text::fontdb;
use nana_text::parity::KNOWN_FONT_FIXTURES;
use std::path::{Path, PathBuf};

/// The locale every case shapes under. Pinned because `sys-locale` would
/// otherwise vary per machine and per CI runner.
pub const CORPUS_LOCALE: &str = "en-US";

fn fonts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts")
}

/// The bytes behind a corpus font fixture id.
///
/// The product UI face is read from `nana-ui-core` rather than copied, so there
/// is only ever one Noto Sans SC in the tree.
pub fn font_bytes(id: &str) -> Option<Vec<u8>> {
    match id {
        "noto-sans-sc" => Some(nana_ui_core::fonts::UI_FONT_REGULAR.to_vec()),
        "nana-test-vf" | "noto-sans-arabic" | "noto-sans-kr" | "noto-emoji" => {
            std::fs::read(fonts_dir().join(format!("{id}.ttf"))).ok()
        }
        _ => None,
    }
}

/// The family name each fixture registers under, for `Attrs::family`.
pub fn font_family(id: &str) -> Option<&'static str> {
    match id {
        "noto-sans-sc" => Some("Noto Sans SC"),
        "nana-test-vf" => Some("NanaTestVF"),
        "noto-sans-arabic" => Some("Noto Sans Arabic"),
        "noto-sans-kr" => Some("Noto Sans KR"),
        "noto-emoji" => Some("Noto Emoji"),
        _ => None,
    }
}

/// Builds a `FontSystem` holding exactly the declared fixtures.
///
/// Returns the system alongside the `fontdb::ID`s in declaration order, which
/// is what the engine's `FontId` side table indexes.
pub fn hermetic_font_system(ids: &[String]) -> (FontSystem, Vec<fontdb::ID>) {
    let mut db = fontdb::Database::new();
    let mut faces = Vec::new();
    for id in ids {
        let bytes = font_bytes(id).unwrap_or_else(|| panic!("unknown font fixture {id:?}"));
        let before: Vec<fontdb::ID> = db.faces().map(|face| face.id).collect();
        db.load_font_data(bytes);
        let added: Vec<fontdb::ID> = db
            .faces()
            .map(|face| face.id)
            .filter(|id| !before.contains(id))
            .collect();
        assert!(
            !added.is_empty(),
            "font fixture {id:?} registered no faces; the subset is probably broken"
        );
        faces.extend(added);
    }
    // The first declared family is also the default, so a case that leaves
    // `font_family` unset still shapes against a known face rather than
    // whatever `sans-serif` resolves to.
    if let Some(first) = ids.first().and_then(|id| font_family(id)) {
        db.set_sans_serif_family(first);
    }
    (
        FontSystem::new_with_locale_and_db(CORPUS_LOCALE.to_string(), db),
        faces,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_font_fixture_can_be_supplied() {
        for id in KNOWN_FONT_FIXTURES {
            let bytes =
                font_bytes(id).unwrap_or_else(|| panic!("no bytes for declared fixture {id:?}"));
            assert!(
                bytes.len() > 512,
                "{id} looks truncated: {} bytes",
                bytes.len()
            );
            assert!(font_family(id).is_some(), "{id} has no family name");
        }
    }
}
