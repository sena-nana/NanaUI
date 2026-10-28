//! The renderer's own rasterizer boundary.
//!
//! [`GlyphRasterizer`] is the abstraction `NanaRenderer::text` owns; what sits
//! under it is an implementation detail it may replace. The implementation is
//! [`nana_text::raster::OutlineRasterizer`], which scales outlines off the
//! faces `nana-text`'s font layer issued, so there is exactly one font
//! instance resolution in the process: the coordinates a run was *shaped* at
//! are the ones it is *scaled* at.
//!
//! The boundary is deliberately wider than today's needs: a request names a
//! [`GlyphRenderMode`](super::glyph::GlyphRenderMode) and an answer names its
//! own [`GlyphImageFormat`], so an LCD, SDF or vector backend is a new arm
//! rather than a new pipeline.

#[cfg(windows)]
pub(super) use nana_text::raster::encode_subpixel;
pub(super) use nana_text::raster::{
    GlyphImage, GlyphImageFormat, OutlineRasterizer as SwashGlyphRasterizer, StrokeJoin,
};
#[cfg(test)]
pub(super) use nana_text::raster::{VARIATION_CAP, blur_mask};

use super::glyph::GlyphRasterKey;

/// [`GlyphRenderMode::Stroke`](super::glyph::GlyphRenderMode::Stroke)'s join
/// for a CSS stroke join.
pub(super) fn stroke_join(join: nana_ui_core::TextStrokeJoin) -> StrokeJoin {
    match join {
        nana_ui_core::TextStrokeJoin::Round => StrokeJoin::Round,
        nana_ui_core::TextStrokeJoin::Miter => StrokeJoin::Miter,
        nana_ui_core::TextStrokeJoin::Bevel => StrokeJoin::Bevel,
    }
}

pub(super) struct GlyphRasterRequest {
    pub key: GlyphRasterKey,
}

/// What `NanaRenderer::text` requires of a glyph raster backend.
pub(super) trait GlyphRasterizer {
    /// Rasterize one glyph, or `None` when the backend cannot produce it at
    /// all (an unknown face, a mode it does not implement). A glyph that is
    /// simply blank answers `Some` with an empty image, so the cache can tell
    /// "nothing to draw" from "ask again".
    fn rasterize(&mut self, request: &GlyphRasterRequest) -> Option<GlyphImage>;
}

impl GlyphRasterizer for SwashGlyphRasterizer {
    fn rasterize(&mut self, request: &GlyphRasterRequest) -> Option<GlyphImage> {
        SwashGlyphRasterizer::rasterize(self, &request.key)
    }
}

#[cfg(test)]
mod tests {
    use super::super::glyph::{GlyphRenderMode, GlyphSynthesis, GlyphVariationId};
    use super::*;
    use nana_text::font::Synthesis;
    use nana_text::font::{AxisCoord, FontInstanceKey};

    fn instance(font: nana_text::FontId, coords: Vec<AxisCoord>) -> FontInstanceKey {
        FontInstanceKey {
            font,
            coords: coords.into(),
            synthesis: Synthesis::default(),
        }
    }

    fn faces(count: usize) -> Vec<nana_text::FontId> {
        let engine = crate::text_engine::nana_text_engine();
        let engine = crate::text_engine::lock_engine(&engine);
        engine.fonts().faces().into_iter().take(count).collect()
    }

    /// A rasterizer and the key of the first glyph of `text` as the process
    /// engine shapes it at 32px, or `None` when no face shapes it.
    fn shaped_key(mode: GlyphRenderMode) -> Option<(SwashGlyphRasterizer, GlyphRasterKey)> {
        let engine = crate::text_engine::nana_text_engine();
        let layout = {
            let mut engine = crate::text_engine::lock_engine(&engine);
            let mut counters = nana_text::TextWorkCounters::default();
            nana_text::TextEngine::layout(
                &mut *engine,
                nana_text::TextKind::Label,
                &nana_text::TextSource::new("O"),
                &nana_text::TextStyle {
                    font_size_px: 32.0,
                    ..nana_text::TextStyle::default()
                },
                &nana_text::TextConstraints::default(),
                &mut counters,
            )
        };
        let run = layout.runs.first()?;
        let instance = run.instance.as_ref()?;
        let mut rasterizer = SwashGlyphRasterizer::new(engine);
        let (font, variation, synthesis) = rasterizer.intern_instance(instance);
        let key = GlyphRasterKey {
            font,
            font_generation: 0,
            variation,
            glyph: run.glyphs.first()?.glyph_id,
            size_bits: super::super::glyph::size_bits(32.0),
            subpixel_x: Default::default(),
            subpixel_y: Default::default(),
            synthesis,
            mode,
        };
        Some((rasterizer, key))
    }

    fn coverage(image: &GlyphImage) -> u64 {
        image.data.iter().map(|value| u64::from(*value)).sum()
    }

    /// The raster key of `ch` in `family` at 32px, as the process engine
    /// shapes it, or `None` when it does not shape it with `face`.
    fn glyph_key(
        family: &str,
        face: &str,
        ch: &str,
    ) -> Option<(SwashGlyphRasterizer, GlyphRasterKey)> {
        let engine = crate::text_engine::nana_text_engine();
        let layout = {
            let mut engine = crate::text_engine::lock_engine(&engine);
            let mut counters = nana_text::TextWorkCounters::default();
            nana_text::TextEngine::layout(
                &mut *engine,
                nana_text::TextKind::Label,
                &nana_text::TextSource::new(ch),
                &nana_text::TextStyle {
                    font_family: Some(family.into()),
                    font_size_px: 32.0,
                    ..nana_text::TextStyle::default()
                },
                &nana_text::TextConstraints::default(),
                &mut counters,
            )
        };
        let run = layout.runs.first()?;
        let instance = run.instance.as_ref()?;
        let described = crate::text_engine::lock_engine(&engine)
            .fonts()
            .describe(instance.font)?;
        if &*described.post_script_name != face {
            return None;
        }
        let mut rasterizer = SwashGlyphRasterizer::new(engine);
        let (font, variation, synthesis) = rasterizer.intern_instance(instance);
        let key = GlyphRasterKey {
            font,
            font_generation: 0,
            variation,
            glyph: run.glyphs.first()?.glyph_id,
            size_bits: super::super::glyph::size_bits(32.0),
            subpixel_x: Default::default(),
            subpixel_y: Default::default(),
            synthesis,
            mode: GlyphRenderMode::Mask,
        };
        Some((rasterizer, key))
    }

    /// swash's own embolden took these glyphs' winding from all of their
    /// contours joined into one polygon, got its sign wrong and pulled every
    /// edge in: fake bold left them with under half the ink of the regular.
    #[test]
    fn fake_bold_only_ever_adds_ink() {
        if let Some((mut rasterizer, key)) = glyph_key("Noto Sans SC", "NotoSansSC-Regular", " ") {
            let key = GlyphRasterKey {
                synthesis: key.synthesis.with(GlyphSynthesis::FAKE_BOLD),
                ..key
            };
            let space = rasterizer.rasterize(&key);
            assert!(
                space.is_some_and(|image| image.is_empty()),
                "a space stays blank"
            );
        }
        for ch in ["传", "仍", "ゴ", "O"] {
            let Some((mut rasterizer, plain_key)) =
                glyph_key("Noto Sans SC", "NotoSansSC-Regular", ch)
            else {
                return;
            };
            let plain = rasterizer
                .rasterize(&plain_key)
                .expect("the regular glyph rasterizes");
            let bold_key = GlyphRasterKey {
                synthesis: plain_key.synthesis.with(GlyphSynthesis::FAKE_BOLD),
                ..plain_key
            };
            let bold = rasterizer
                .rasterize(&bold_key)
                .expect("the fake bold rasterizes");
            assert!(
                coverage(&bold) as f64 > coverage(&plain) as f64 * 1.2,
                "{ch}: fake bold has {} of ink, the regular {}",
                coverage(&bold),
                coverage(&plain)
            );
            let at = |image: &GlyphImage, x: i32, y: i32| {
                let (col, row) = (x - image.left, image.top - y);
                if col < 0 || row < 0 || col >= image.width as i32 || row >= image.height as i32 {
                    return 0;
                }
                image.data[(row as u32 * image.width + col as u32) as usize]
            };
            for row in 0..plain.height as i32 {
                for col in 0..plain.width as i32 {
                    let (x, y) = (plain.left + col, plain.top - row);
                    if at(&plain, x, y) == 255 {
                        assert!(
                            at(&bold, x, y) >= 250,
                            "{ch}: fake bold uncovers ({x}, {y}) of the regular glyph"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_stroked_glyph_is_wider_than_its_fill_and_shares_its_centre() {
        let Some((mut rasterizer, fill_key)) = shaped_key(GlyphRenderMode::Mask) else {
            return;
        };
        let fill = rasterizer
            .rasterize(&fill_key)
            .expect("the fill rasterizes");
        let stroke_key = GlyphRasterKey {
            mode: GlyphRenderMode::Stroke {
                width_q: GlyphRenderMode::quarters(4.0),
                join: stroke_join(nana_ui_core::TextStrokeJoin::Round),
            },
            ..fill_key
        };
        let stroke = rasterizer
            .rasterize(&stroke_key)
            .expect("the outline strokes");
        assert_eq!(stroke.format, GlyphImageFormat::Mask);
        assert!(
            stroke.width >= fill.width + 3 && stroke.height >= fill.height + 3,
            "a 4px stroke reaches 2px past each edge: fill {}x{}, stroke {}x{}",
            fill.width,
            fill.height,
            stroke.width,
            stroke.height
        );
        let centre = |image: &GlyphImage| {
            (
                image.left as f32 + image.width as f32 * 0.5,
                image.top as f32 - image.height as f32 * 0.5,
            )
        };
        let (fill_x, fill_y) = centre(&fill);
        let (stroke_x, stroke_y) = centre(&stroke);
        assert!(
            (fill_x - stroke_x).abs() <= 1.0 && (fill_y - stroke_y).abs() <= 1.0,
            "the stroke is centred on the outline the fill is drawn from"
        );
    }

    #[test]
    fn a_blurred_glyph_keeps_its_coverage() {
        let Some((mut rasterizer, fill_key)) = shaped_key(GlyphRenderMode::Mask) else {
            return;
        };
        let fill = rasterizer
            .rasterize(&fill_key)
            .expect("the fill rasterizes");
        for radius in [3.0f32, 6.0, 12.0, 24.0] {
            let blurred = rasterizer
                .rasterize(&GlyphRasterKey {
                    mode: GlyphRenderMode::Blur {
                        radius_q: GlyphRenderMode::quarters(radius),
                        spread_q: 0,
                    },
                    ..fill_key
                })
                .expect("the shadow rasterizes");
            assert!(
                blurred.width > fill.width,
                "a blur spreads the coverage out"
            );
            let (before, after) = (coverage(&fill) as f64, coverage(&blurred) as f64);
            assert!(
                (after - before).abs() / before < 0.02,
                "radius {radius}: coverage {before} became {after}"
            );
            assert!(
                blurred.data.iter().copied().max().unwrap_or(0)
                    < fill.data.iter().copied().max().unwrap_or(0),
                "and lowers its peak"
            );
        }
    }

    #[test]
    fn blurring_a_square_conserves_coverage_at_full_and_half_resolution() {
        let (width, height) = (12u32, 12u32);
        let square = vec![255u8; (width * height) as usize];
        for radius in [1.0f32, 4.0, 8.0, 9.0, 16.0, 30.0] {
            let (data, blurred_width, blurred_height, pad) =
                blur_mask(&square, width, height, radius);
            assert_eq!(blurred_width, width + pad * 2);
            assert_eq!(blurred_height, height + pad * 2);
            let before: f64 = square.iter().map(|value| f64::from(*value)).sum();
            let after: f64 = data.iter().map(|value| f64::from(*value)).sum();
            assert!(
                (after - before).abs() / before < 0.02,
                "radius {radius}: {before} became {after}"
            );
        }
    }

    #[test]
    fn interning_the_same_face_twice_issues_one_id() {
        let mut rasterizer = SwashGlyphRasterizer::new(crate::text_engine::nana_text_engine());
        let ids = faces(2);
        if ids.is_empty() {
            return;
        }
        let first = rasterizer.intern_instance(&instance(ids[0], Vec::new())).0;
        assert_eq!(
            rasterizer.intern_instance(&instance(ids[0], Vec::new())).0,
            first
        );
        assert_eq!(rasterizer.face_count(), 1);
        if let Some(second) = ids.get(1) {
            assert_ne!(
                rasterizer.intern_instance(&instance(*second, Vec::new())).0,
                first,
                "a different face is a different id"
            );
        }
    }

    #[test]
    fn the_default_instance_and_a_varied_one_are_different_raster_keys() {
        let mut rasterizer = SwashGlyphRasterizer::new(crate::text_engine::nana_text_engine());
        let ids = faces(1);
        if ids.is_empty() {
            return;
        }
        let (_, default, _) = rasterizer.intern_instance(&instance(ids[0], Vec::new()));
        let (_, varied, _) = rasterizer.intern_instance(&instance(
            ids[0],
            vec![AxisCoord {
                tag: *b"wght",
                value: 700.0,
            }],
        ));
        assert_eq!(default, GlyphVariationId(0));
        assert_ne!(varied, default, "coordinates change the bitmap");
        let wide = vec![AxisCoord {
            tag: *b"wdth",
            value: 75.0,
        }];
        let (_, narrowed, _) = rasterizer.intern_instance(&instance(ids[0], wide.clone()));
        assert_ne!(narrowed, varied, "other coordinates are another instance");
        assert_eq!(rasterizer.coords(narrowed), wide.as_slice());
        let (_, again, _) = rasterizer.intern_instance(&instance(ids[0], wide));
        assert_eq!(
            again, narrowed,
            "the same coordinates are the same instance"
        );
        assert_eq!(
            rasterizer.face_count(),
            1,
            "coordinates are not a second face"
        );
    }

    #[test]
    fn synthesis_travels_from_the_font_layer_to_the_raster_key() {
        let mut rasterizer = SwashGlyphRasterizer::new(crate::text_engine::nana_text_engine());
        let ids = faces(1);
        if ids.is_empty() {
            return;
        }
        let mut key = instance(ids[0], Vec::new());
        key.synthesis = Synthesis {
            bold: true,
            oblique: true,
        };
        let (_, _, synthesis) = rasterizer.intern_instance(&key);
        assert!(synthesis.contains(GlyphSynthesis::FAKE_BOLD));
        assert!(synthesis.contains(GlyphSynthesis::FAKE_ITALIC));
    }
}
