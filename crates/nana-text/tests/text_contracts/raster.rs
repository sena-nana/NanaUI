//! `nana_text::raster` and the placements it draws from: a paragraph's glyphs
//! land where the pen puts them and rasterize into coverage.

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

/// One rasterizer draws every weight of the bundled variable face. A glyph at
/// the default instance must come out the same whether or not a heavier
/// instance was drawn just before it: the scale context is shared, and its
/// axis coordinates must not leak from one glyph into the next.
#[test]
fn a_default_instance_glyph_does_not_inherit_the_previous_glyphs_axis() {
    let engine = engine();
    let at_weight = |weight: u16| {
        let mut counters = TextWorkCounters::default();
        let layout = lock_text_engine(&engine).layout(
            TextKind::Label,
            &TextSource::new("中"),
            &TextStyle {
                font_size_px: 32.0,
                font_weight: weight,
                ..TextStyle::default()
            },
            &TextConstraints::default(),
            &mut counters,
        );
        (*layout).clone()
    };
    let regular = at_weight(400);
    let bold = at_weight(700);
    let ink = |rasterizer: &mut OutlineRasterizer, layout: &TextLayout| -> u64 {
        rasterize_layout(rasterizer, layout, 1.0, [0.0, 0.0], GlyphRenderMode::Mask)
            .iter()
            .flat_map(|glyph| glyph.image.data.iter())
            .map(|value| u64::from(*value))
            .sum()
    };

    let mut fresh = OutlineRasterizer::new(Arc::clone(&engine));
    let regular_alone = ink(&mut fresh, &regular);

    let mut shared = OutlineRasterizer::new(Arc::clone(&engine));
    let bold_ink = ink(&mut shared, &bold);
    let regular_after_bold = ink(&mut shared, &regular);
    assert!(bold_ink > regular_alone, "{bold_ink} > {regular_alone}");
    assert_eq!(regular_after_bold, regular_alone);
}

/// Rows of `glyphs`' ink, top and bottom (exclusive), in raster px.
fn ink_rows(glyphs: &[RasterizedGlyph]) -> (i32, i32) {
    let mut top = i32::MAX;
    let mut bottom = i32::MIN;
    for glyph in glyphs {
        let width = glyph.image.width as usize;
        for (row, pixels) in glyph.image.data.chunks(width.max(1)).enumerate() {
            if pixels.iter().any(|coverage| *coverage > 0) {
                let y = glyph.at[1] + row as i32;
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (top, bottom)
}

/// One line of Chinese in a capsule: the line box is the capsule's height,
/// and the ideographs' ink sits in its middle. The bundled face's `hhea` puts
/// 1.16 em above the baseline and 0.288 below; centring the line on that
/// left the glyphs about 0.06 em low (10 px over, 6 under, in an 18 px chip
/// at scale 2). Centred on the face's ideographic em box the gaps match.
#[test]
fn one_line_of_cjk_is_centred_in_its_line_box() {
    let engine = engine();
    for (text, font_px, line_px) in [
        ("小贴士", 12.0, 18.0),
        ("打开模型", 14.0, 25.0),
        ("中", 32.0, 40.0),
    ] {
        let mut counters = TextWorkCounters::default();
        let layout = lock_text_engine(&engine).layout(
            TextKind::Label,
            &TextSource::new(text),
            &TextStyle {
                font_size_px: font_px,
                line_height: Some(nana_ui_core::LineHeightSpec::Absolute(line_px)),
                ..TextStyle::default()
            },
            &TextConstraints::default(),
            &mut counters,
        );
        let mut rasterizer = OutlineRasterizer::new(Arc::clone(&engine));
        let scale = 2.0;
        let glyphs = rasterize_layout(
            &mut rasterizer,
            &layout,
            scale,
            [0.0, 0.0],
            GlyphRenderMode::Mask,
        );
        let (top, bottom) = ink_rows(&glyphs);
        let box_px = (line_px * scale) as i32;
        let (above, below) = (top, box_px - bottom);
        assert!(
            (above - below).abs() <= 1,
            "{text} at {font_px}px in {line_px}px: {above} px above the ink, {below} below"
        );
    }
}

/// The recentring moves where a line sits in its box, not how far apart the
/// lines are.
#[test]
fn recentred_lines_keep_their_pitch() {
    let engine = engine();
    let mut counters = TextWorkCounters::default();
    let layout = lock_text_engine(&engine).layout(
        TextKind::Paragraph,
        &TextSource::new("第一行第二行Third"),
        &TextStyle {
            font_size_px: 14.0,
            line_height: Some(nana_ui_core::LineHeightSpec::Absolute(22.0)),
            ..TextStyle::default()
        },
        &TextConstraints {
            max_width_px: Some(45.0),
            wrap: Some(nana_ui_core::TextWrapBreak::WordOrGlyph),
            ..TextConstraints::default()
        },
        &mut counters,
    );
    let baselines: Vec<f32> = layout
        .lines
        .iter()
        .map(|line| line.metrics.baseline_y_px)
        .collect();
    assert_eq!(baselines.len(), 3);
    for pair in baselines.windows(2) {
        assert!((pair[1] - pair[0] - 22.0).abs() < 1e-3, "{baselines:?}");
    }
}

/// Only a CJK face with an ideographic em box moves: a Latin face keeps its
/// `hhea` ascent and descent, and the CJK face keeps their sum, so a
/// `normal` line height is unchanged.
#[test]
fn only_cjk_faces_are_recentred_and_keep_their_extent() {
    let engine = engine();
    {
        let mut engine = lock_text_engine(&engine);
        engine
            .fonts_mut()
            .register_bytes(
                font_blob(include_bytes!("../../fonts/nana-test-vf.ttf")),
                &FaceDescriptor::default(),
            )
            .unwrap();
    }
    let metrics = |text: &str, family: Option<&str>| {
        let layout = layout(&engine, text, family);
        let run = layout.line_runs(&layout.lines[0])[0].metrics;
        (run.ascent_px, run.descent_px)
    };
    // nana-test-vf: hhea 800 / −200 per 1000, no BASE.
    let (ascent, descent) = metrics("Ab", Some("NanaTestVF"));
    assert!((ascent - 32.0 * 0.8).abs() < 1e-3, "{ascent}");
    assert!((descent - 32.0 * 0.2).abs() < 1e-3, "{descent}");
    // Noto Sans SC: hhea 1160 / −288, em box 880 / −120.
    let (ascent, descent) = metrics("中", None);
    assert!((ascent + descent - 32.0 * 1.448).abs() < 1e-3);
    assert!((ascent - 32.0 * 1.104).abs() < 1e-3, "{ascent}");
}
