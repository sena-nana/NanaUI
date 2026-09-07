//! The bundled Noto Sans SC faces, as a single source of truth.
//!
//! These constants live here rather than in `nana-ui` because three separate
//! crates need the regular face: the theme (`nana-ui`), the SVG rasterizer
//! (`nana-ui-vue`) and Canvas2D text (`nana-ui-web-api`). Each used to
//! `include_bytes!` the file itself, and the linker does not merge identical
//! constant data across crates — desktop binaries carried two copies of the
//! 2.5 MB regular face on top of the three other weights.
//!
//! Two features, so a host pays only for what it draws with:
//!
//! - `bundled-font-regular` — the regular face alone. On by default for every
//!   consumer, because the SVG and Canvas2D paths have no other fallback.
//! - `bundled-fonts` — all four UI weights. Implies the above. Desktop hosts
//!   that render the full theme want this; the Android host does not.
//!
//! The `.ttf` files still live under `crates/nana-ui/assets/fonts/`, which is
//! where they have always been and where `nana-ui`'s tests reference them.

/// LiliaUI's regular Noto Sans SC face.
#[cfg(feature = "bundled-font-regular")]
pub const UI_FONT_REGULAR: &[u8] =
    include_bytes!("../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf").as_slice();
/// LiliaUI's medium Noto Sans SC face.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_MEDIUM: &[u8] =
    include_bytes!("../../nana-ui/assets/fonts/NotoSansSC-Medium.ttf").as_slice();
/// LiliaUI's semibold Noto Sans SC face.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_SEMIBOLD: &[u8] =
    include_bytes!("../../nana-ui/assets/fonts/NotoSansSC-SemiBold.ttf").as_slice();
/// LiliaUI's bold Noto Sans SC face.
#[cfg(feature = "bundled-fonts")]
pub const UI_FONT_BOLD: &[u8] =
    include_bytes!("../../nana-ui/assets/fonts/NotoSansSC-Bold.ttf").as_slice();

/// The family name the bundled faces register under.
#[cfg(feature = "bundled-font-regular")]
pub const UI_FONT_FAMILY: &str = "Noto Sans SC";

/// Returns every bundled UI face for registration with the Scene text shaper.
#[cfg(feature = "bundled-fonts")]
pub const fn ui_font_sources() -> [&'static [u8]; 4] {
    [
        UI_FONT_REGULAR,
        UI_FONT_MEDIUM,
        UI_FONT_SEMIBOLD,
        UI_FONT_BOLD,
    ]
}
