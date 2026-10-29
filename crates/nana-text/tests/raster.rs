//! `nana_text::raster` and the placements it draws from: a paragraph's glyphs
//! land where the pen puts them and rasterize into coverage.

#![cfg(feature = "raster")]
use std::sync::{Arc, Mutex};

use nana_text::font::{FaceDescriptor, FontSystem, font_blob};
use nana_text::raster::{
    GlyphImageFormat, GlyphRenderMode, OutlineRasterizer, RasterizedGlyph, StrokeJoin,
    rasterize_layout,
};
use nana_text::{
    NativeTextEngine, SharedTextEngine, TextConstraints, TextEngine, TextKind, TextLayout,
    TextSource, TextStyle, TextWorkCounters, lock_text_engine,
};

fn engine() -> SharedTextEngine {
    let mut fonts = FontSystem::hermetic();
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .unwrap();
    Arc::new(Mutex::new(NativeTextEngine::new(fonts)))
}

fn layout(engine: &SharedTextEngine, text: &str, family: Option<&str>) -> TextLayout {
    let mut counters = TextWorkCounters::default();
    let layout = lock_text_engine(engine).layout(
        TextKind::Label,
        &TextSource::new(text),
        &TextStyle {
            font_family: family.map(Into::into),
            font_size_px: 32.0,
            ..TextStyle::default()
        },
        &TextConstraints::default(),
        &mut counters,
    );
    (*layout).clone()
}

#[test]
fn placements_walk_the_pen_along_each_line() {
    let engine = engine();
    let layout = layout(&engine, "AB", None);
    let at_one = layout.glyph_placements(1.0, [0.0, 0.0]);
    assert_eq!(at_one.len(), 2);
    assert!(at_one[1].position[0] > at_one[0].position[0]);
    assert_eq!(at_one[0].position[1], at_one[1].position[1]);
    assert_eq!(at_one[0].position[1].fract(), 0.0);
    assert_eq!(at_one[1].cluster, 1);
    // Twice the scale, an origin of its own: twice as far apart, moved.
    let at_two = layout.glyph_placements(2.0, [10.25, 5.0]);
    let gap =
        |placed: &[nana_text::GlyphPlacement<'_>]| placed[1].position[0] - placed[0].position[0];
    assert!((gap(&at_two) - 2.0 * gap(&at_one)).abs() < 1e-3);
    assert!((at_two[0].position[0] - 2.0 * at_one[0].position[0] - 10.25).abs() < 1e-3);
    assert_eq!(at_two[0].font_size_px, 64.0);
}

#[test]
fn a_paragraph_rasterizes_into_placed_coverage() {
    let engine = engine();
    let layout = layout(&engine, "Hi 你", None);
    let mut rasterizer = OutlineRasterizer::new(Arc::clone(&engine));
    let glyphs = rasterize_layout(
        &mut rasterizer,
        &layout,
        1.0,
        [0.0, 0.0],
        GlyphRenderMode::Mask,
    );
    // The space covers nothing and is left out.
    assert_eq!(glyphs.len(), 3);
    assert!(
        glyphs
            .iter()
            .all(|glyph| glyph.image.format == GlyphImageFormat::Mask)
    );
    assert!(glyphs.windows(2).all(|pair| pair[1].at[0] > pair[0].at[0]));
    assert_eq!(glyphs[2].cluster, "Hi ".len() as u32);
    let ink = |glyph: &RasterizedGlyph| glyph.image.data.iter().map(|v| u64::from(*v)).sum::<u64>();
    assert!(glyphs.iter().all(|glyph| ink(glyph) > 0));

    let stroked = rasterize_layout(
        &mut rasterizer,
        &layout,
        1.0,
        [0.0, 0.0],
        GlyphRenderMode::Stroke {
            width_q: GlyphRenderMode::quarters(3.0),
            join: StrokeJoin::Round,
        },
    );
    // An outline reaches past the glyph it strokes.
    assert!(stroked[0].image.width > glyphs[0].image.width);
}
