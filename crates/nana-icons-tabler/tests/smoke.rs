use nana_icons_tabler::{HOME, TRASH, USER};

#[test]
fn catalog_consts_resolve_with_valid_identity() {
    // Named IconData statics keep `Icon` pointer identity stable per process.
    assert_eq!(HOME, nana_icons_tabler::HOME);
    assert_ne!(HOME, TRASH);
}

#[test]
fn catalog_icons_carry_paintable_geometry() {
    for icon in [HOME, TRASH, USER] {
        assert!(
            !icon.shapes().is_empty(),
            "{:?} missing shapes",
            icon.name()
        );
    }
}

#[test]
fn catalog_icons_rasterize_through_the_real_paint_path() {
    for icon in [HOME, TRASH, USER] {
        let rgba = nana_svg_raster::rasterize_white_mask(icon.svg(), 28, 256)
            .unwrap_or_else(|| panic!("{} failed to rasterize", icon.name()));
        let ink = rgba.chunks(4).filter(|pixel| pixel[3] > 16).count();
        assert!(ink > 20, "{:?} inks only {ink} pixels", icon.name());
        assert!(
            rgba.chunks(4).any(|pixel| pixel[3] < 16),
            "{:?} must keep transparent padding",
            icon.name()
        );
    }
}
