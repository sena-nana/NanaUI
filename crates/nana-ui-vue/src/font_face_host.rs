//! Host-adapter ingest for parsed `@font-face` rules.
//!
//! Only compiled with `scene-view` so CSS parse / css-parity stay off the
//! `nana-ui` FontSystem path.

use crate::css_font_face::{FontFaceRule, FontFaceSrcKind, FontFaceStyle};

/// Map parsed faces onto [`nana_ui::ingest_host_font_faces`].
pub(crate) fn ingest_parsed_font_faces(rules: &[FontFaceRule]) -> usize {
    if rules.is_empty() {
        return 0;
    }
    let specs: Vec<nana_ui::HostFontFaceSpec> = rules
        .iter()
        .map(|rule| nana_ui::HostFontFaceSpec {
            family: rule.font_family.clone(),
            urls: rule
                .src
                .iter()
                .filter(|src| src.kind == FontFaceSrcKind::Url)
                .map(|src| src.value.clone())
                .collect(),
            weight: rule.font_weight,
            style: rule.font_style.map(|style| match style {
                FontFaceStyle::Normal => nana_ui::HostFontStyle::Normal,
                FontFaceStyle::Italic => nana_ui::HostFontStyle::Italic,
                FontFaceStyle::Oblique => nana_ui::HostFontStyle::Oblique,
            }),
        })
        .collect();
    nana_ui::ingest_host_font_faces(&specs)
}
