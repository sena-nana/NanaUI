//! What `skrifa` can tell about one face. The only file allowed to name it.
//!
//! Everything returned here is a Nana type, so no `skrifa` or `read_fonts`
//! item can escape through the font system's public API.

use super::coverage::CoverageSet;
use super::variations::{AxisCoord, FontAxis, NamedInstance};
use crate::metrics::RunMetrics;
use skrifa::prelude::Size;
use skrifa::{FontRef, MetadataProvider, Tag};

/// Colour glyph tables. Presence only: painting them is a renderer concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorTables {
    pub colr: bool,
    pub cbdt: bool,
    pub sbix: bool,
    pub svg: bool,
}

impl ColorTables {
    pub fn any(self) -> bool {
        self.colr || self.cbdt || self.sbix || self.svg
    }
}

/// Face facts that need the font bytes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FaceDetails {
    pub axes: Vec<FontAxis>,
    pub named_instances: Vec<NamedInstance>,
    pub color: ColorTables,
}

impl FaceDetails {
    pub fn axis(&self, tag: [u8; 4]) -> Option<&FontAxis> {
        self.axes.iter().find(|axis| axis.tag == tag)
    }
}

/// Parses the details of face `index` in `data`, or `None` when the bytes are
/// not a face `skrifa` can read.
pub fn read_details(data: &[u8], index: u32) -> Option<FaceDetails> {
    let font = FontRef::from_index(data, index).ok()?;
    let axes: Vec<FontAxis> = font
        .axes()
        .iter()
        .map(|axis| FontAxis {
            tag: axis.tag().to_be_bytes(),
            min: axis.min_value(),
            default: axis.default_value(),
            max: axis.max_value(),
        })
        .collect();
    let named_instances = font
        .named_instances()
        .iter()
        .map(|instance| NamedInstance {
            name: font
                .localized_strings(instance.subfamily_name_id())
                .english_or_first()
                .map(|name| name.chars().collect()),
            coords: axes
                .iter()
                .zip(instance.user_coords())
                .map(|(axis, value)| AxisCoord {
                    tag: axis.tag,
                    value,
                })
                .collect(),
        })
        .collect();
    let has = |tag: &[u8; 4]| font.table_data(Tag::new(tag)).is_some();
    Some(FaceDetails {
        axes,
        named_instances,
        color: ColorTables {
            colr: has(b"COLR"),
            cbdt: has(b"CBDT"),
            sbix: has(b"sbix"),
            svg: has(b"SVG "),
        },
    })
}

/// Ascent, descent and line gap in px at `coords` and `size_px`, as positive
/// numbers, plus the underline (`post`) and strikeout (`OS/2`) the face
/// suggests.
///
/// A face that leaves either out — or states a thickness of zero — gets a
/// stand-in proportional to its size: a 1/14 em stroke, the underline a tenth
/// of an em under the baseline, the strikeout centred on half the x-height.
pub fn read_metrics(
    data: &[u8],
    index: u32,
    coords: &[AxisCoord],
    size_px: f32,
) -> Option<RunMetrics> {
    let font = FontRef::from_index(data, index).ok()?;
    let location = font.axes().location(
        coords
            .iter()
            .map(|coord| (Tag::new(&coord.tag), coord.value)),
    );
    let metrics = font.metrics(Size::new(size_px), &location);
    let fallback_thickness = (size_px / 14.0).max(f32::MIN_POSITIVE);
    let usable = |decoration: Option<skrifa::metrics::Decoration>| {
        decoration.filter(|decoration| {
            decoration.thickness.is_finite()
                && decoration.thickness > 0.0
                && decoration.offset.is_finite()
        })
    };
    let (underline_offset_px, underline_thickness_px) = usable(metrics.underline).map_or(
        (size_px * 0.1, fallback_thickness),
        // skrifa's offset is to the top of the stroke, y-up; this one is
        // positive down.
        |decoration| (-decoration.offset, decoration.thickness),
    );
    let x_height = metrics
        .x_height
        .filter(|height| height.is_finite() && *height > 0.0)
        .unwrap_or(metrics.ascent * 0.5);
    let (strikeout_offset_px, strikeout_thickness_px) = usable(metrics.strikeout).map_or(
        (
            x_height * 0.5 + fallback_thickness * 0.5,
            fallback_thickness,
        ),
        |decoration| (decoration.offset, decoration.thickness),
    );
    let (ascent_px, descent_px) = centred_on_ideographic_em_box(
        &font,
        size_px,
        metrics.units_per_em,
        metrics.ascent,
        metrics.descent.abs(),
    );
    Some(RunMetrics {
        ascent_px,
        descent_px,
        line_gap_px: metrics.leading,
        underline_offset_px,
        underline_thickness_px,
        strikeout_offset_px,
        strikeout_thickness_px,
        x_height_px: x_height,
    })
}

/// A CJK face's ascent and descent, moved so their midpoint is the middle of
/// the ideographic em box the face declares; any other face's, unchanged.
///
/// CJK faces set `hhea` generously and lopsidedly (Source Han / Noto CJK:
/// 1.16 above, 0.288 below) while their ideographs fill an em box at
/// 0.88 / −0.12. Half-leading splits the line around the `hhea` midpoint, so
/// a single line in a box of any height sits about 0.06 em low. The face's
/// `BASE` table states the em box (`ideo`, and `idtp` or one em above it);
/// centring on it is what the central baseline means for these faces. The
/// sum, and with it a `normal` line height, is unchanged, so a paragraph's
/// line pitch is too. Only faces that map a Han, kana or Hangul character
/// count: a Latin face may carry an `ideo` baseline for alignment against
/// CJK, but its own letters are not drawn on that em box.
fn centred_on_ideographic_em_box(
    font: &FontRef<'_>,
    size_px: f32,
    units_per_em: u16,
    ascent: f32,
    descent: f32,
) -> (f32, f32) {
    let Some((top, bottom)) = ideographic_em_box(font, units_per_em) else {
        return (ascent, descent);
    };
    let scale = size_px / f32::from(units_per_em.max(1));
    let shift = ((ascent - descent) - (top + bottom) * scale) * 0.5;
    if !shift.is_finite() {
        return (ascent, descent);
    }
    (ascent - shift, descent + shift)
}

/// The ideographic em box from the face's horizontal `BASE` axis, in font
/// units with y up: its top and bottom edges. `None` unless the face draws
/// CJK and states an `ideo` baseline.
fn ideographic_em_box(font: &FontRef<'_>, units_per_em: u16) -> Option<(f32, f32)> {
    use skrifa::raw::TableProvider;
    let charmap = font.charmap();
    if !['\u{4E00}', '\u{3042}', '\u{30A2}', '\u{AC00}']
        .into_iter()
        .any(|ch| charmap.map(ch).is_some())
    {
        return None;
    }
    let axis = font.base().ok()?.horiz_axis()?.ok()?;
    let tags = axis.base_tag_list()?.ok()?;
    let tags = tags.baseline_tags();
    let index = |tag: &[u8; 4]| tags.iter().position(|t| t.get() == Tag::new(tag));
    let ideo = index(b"ideo")?;
    let idtp = index(b"idtp");
    let list = axis.base_script_list().ok()?;
    let records = list.base_script_records();
    // The em box is one per face in practice; prefer the Han record, then any
    // CJK one, then the default.
    let record = [b"hani", b"kana", b"hang", b"DFLT"]
        .into_iter()
        .find_map(|script| {
            records
                .iter()
                .find(|record| record.base_script_tag() == Tag::new(script))
        })
        .or_else(|| records.first())?;
    let values = record
        .base_script(list.offset_data())
        .ok()?
        .base_values()?
        .ok()?;
    let coords = values.base_coords();
    let coord =
        |index: usize| -> Option<f32> { Some(f32::from(coords.get(index).ok()?.coordinate())) };
    let bottom = coord(ideo)?;
    let top = idtp
        .and_then(coord)
        .unwrap_or(bottom + f32::from(units_per_em));
    (top > bottom).then_some((top, bottom))
}

/// The left and right edges of glyph `glyph_id`'s ink at `size_px`, from
/// its pen position along a horizontal line; `None` when the face gives no
/// bounds for it.
pub fn read_glyph_ink(
    data: &[u8],
    index: u32,
    coords: &[AxisCoord],
    size_px: f32,
    glyph_id: u32,
) -> Option<(f32, f32)> {
    let font = FontRef::from_index(data, index).ok()?;
    let location = font.axes().location(
        coords
            .iter()
            .map(|coord| (Tag::new(&coord.tag), coord.value)),
    );
    let bounds = font
        .glyph_metrics(Size::new(size_px), &location)
        .bounds(skrifa::GlyphId::new(glyph_id))?;
    (bounds.x_min.is_finite() && bounds.x_max.is_finite() && bounds.x_max > bounds.x_min)
        .then_some((bounds.x_min, bounds.x_max))
}

/// Every codepoint the face's best cmap maps to a glyph it can draw,
/// folded into ranges.
///
/// A visible character mapped to an empty outline is left out, so fallback
/// looks further: some faces map a character they do not draw (macOS's
/// Songti SC Black maps U+00A5 YEN SIGN to a blank glyph), and taking the
/// mapping at its word drew a gap where the character belonged.
pub fn read_coverage(data: &[u8], index: u32) -> CoverageSet {
    let Ok(font) = FontRef::from_index(data, index) else {
        return CoverageSet::default();
    };
    let blank = blank_outlines(&font);
    let mut codepoints: Vec<u32> = font
        .charmap()
        .mappings()
        .filter(|(_, glyph)| glyph.to_u32() != 0)
        .filter(|(codepoint, glyph)| {
            !blank.as_ref().is_some_and(|blank| blank(glyph.to_u32()))
                || char::from_u32(*codepoint).is_none_or(|ch| !visible(ch))
        })
        .map(|(codepoint, _)| codepoint)
        .collect();
    codepoints.sort_unstable();
    codepoints.dedup();
    CoverageSet::from_sorted_codepoints(&codepoints)
}

/// Which glyphs have no outline, for a face drawn from `glyf` outlines
/// alone: a glyph whose `loca` entry and the next one are equal is empty.
/// `None` for any other face. A colour or bitmap face may leave a glyph's
/// outline empty and draw it from its own table, and a CFF face's empty
/// glyphs cannot be told without running their charstrings.
fn blank_outlines<'a>(font: &FontRef<'a>) -> Option<impl Fn(u32) -> bool + 'a> {
    use skrifa::raw::TableProvider;
    let drawn_elsewhere = [b"COLR", b"sbix", b"CBDT", b"EBDT", b"SVG "]
        .iter()
        .any(|tag| font.table_data(Tag::new(tag)).is_some());
    if drawn_elsewhere || font.table_data(Tag::new(b"glyf")).is_none() {
        return None;
    }
    let loca = font.loca(None).ok()?;
    Some(move |glyph: u32| {
        let glyph = glyph as usize;
        matches!(
            (loca.get_raw(glyph), loca.get_raw(glyph + 1)),
            (Some(start), Some(end)) if start == end
        )
    })
}

/// A character that draws ink: not white space, a control or a default
/// ignorable, whose glyphs are rightly empty.
fn visible(ch: char) -> bool {
    !ch.is_whitespace() && !ch.is_control() && !super::unicode::is_default_ignorable(ch)
}

#[cfg(test)]
mod blank_glyph_tests {
    use super::*;
    use skrifa::raw::TableProvider;

    const KR: &[u8] = include_bytes!("../../fonts/noto-sans-kr.ttf");
    const COLOR: &[u8] = include_bytes!("../../fonts/nana-test-color.ttf");

    fn mapped(data: &[u8]) -> Vec<(u32, u32)> {
        FontRef::new(data)
            .unwrap()
            .charmap()
            .mappings()
            .map(|(c, g)| (c, g.to_u32()))
            .filter(|(_, g)| *g != 0)
            .collect()
    }

    /// `data` with glyph `glyph` emptied: its next `loca` entry made equal
    /// to its own, as a face that maps a character it does not draw has it.
    fn emptied(data: &[u8], glyph: u32) -> Vec<u8> {
        let font = FontRef::new(data).unwrap();
        let long = font.head().unwrap().index_to_loc_format() == 1;
        let record = font
            .table_directory
            .table_records()
            .iter()
            .find(|r| r.tag() == Tag::new(b"loca"))
            .unwrap();
        let at = record.offset() as usize;
        let mut out = data.to_vec();
        let (size, i) = (if long { 4 } else { 2 }, glyph as usize);
        let (from, to) = (at + i * size, at + (i + 1) * size);
        let entry = out[from..from + size].to_vec();
        out[to..to + size].copy_from_slice(&entry);
        out
    }

    #[test]
    fn a_character_mapped_to_an_empty_outline_is_not_covered() {
        let font = FontRef::new(KR).unwrap();
        let loca = font.loca(None).unwrap();
        let blank = |g: u32| loca.get_raw(g as usize) == loca.get_raw(g as usize + 1);
        let (ch, glyph) = mapped(KR)
            .into_iter()
            .find(|(c, g)| {
                char::from_u32(*c).is_some_and(visible)
                    && !blank(*g)
                    && (*g as usize + 2) < loca.len()
            })
            .expect("a drawn character");
        assert!(read_coverage(KR, 0).contains(ch));
        let patched = emptied(KR, glyph);
        assert!(
            !read_coverage(&patched, 0).contains(ch),
            "U+{ch:04X} maps to a blank glyph, so fallback must look further"
        );
        // White space draws nothing and stays covered.
        if mapped(KR).iter().any(|(c, _)| *c == 0x20) {
            assert!(read_coverage(&patched, 0).contains(0x20));
        }
    }

    #[test]
    fn a_colour_face_keeps_every_mapping() {
        let coverage = read_coverage(COLOR, 0);
        for (ch, _) in mapped(COLOR) {
            assert!(coverage.contains(ch), "U+{ch:04X}");
        }
    }
}

#[cfg(feature = "raster")]
mod raster_support {
    use crate::font::AxisCoord;
    use skrifa::bitmap::{BitmapData, BitmapFormat, BitmapStrikes, Origin as BitmapOrigin};
    use skrifa::instance::NormalizedCoord;
    use skrifa::outline::{
        DrawSettings, HintingInstance, HintingMode, LcdLayout, OutlineGlyphCollection,
        OutlineGlyphFormat, OutlinePen,
    };
    use skrifa::prelude::{LocationRef, Size};
    use skrifa::raw::TableProvider;
    use skrifa::string::StringId;
    use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};

    /// Maximum cached `HintingInstance` entries per outline format.
    const MAX_CACHED_HINT_INSTANCES: usize = 8;

    const HINTING_MODE: HintingMode = HintingMode::Smooth {
        lcd_subpixel: Some(LcdLayout::Horizontal),
        preserve_linear_metrics: true,
    };

    /// Receiver for scaled glyph outline contours and optional COLR layer colors.
    pub(crate) trait FaceOutlineSink {
        fn begin_layer(&mut self, color: Option<[u8; 4]>);
        fn move_to(&mut self, x: f32, y: f32);
        fn line_to(&mut self, x: f32, y: f32);
        fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32);
        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32);
        fn close(&mut self);
        fn finish_layer(&mut self);
    }

    /// Capabilities exposed by a font face for CPU glyph rasterization.
    #[derive(Clone, Copy, Debug, Default)]
    pub(crate) struct FaceRasterCaps {
        pub has_outlines: bool,
        pub has_color_outlines: bool,
        pub has_color_bitmaps: bool,
    }

    /// Borrowed color bitmap bytes stored inside a strike table.
    pub(crate) enum RawBitmapPayload<'a> {
        Png(&'a [u8]),
        Bgra(&'a [u8]),
    }

    /// Unscaled metadata and raw bytes for one color bitmap strike glyph.
    pub(crate) struct RawColorBitmap<'a> {
        pub payload: RawBitmapPayload<'a>,
        pub ppem: f32,
        pub width: u32,
        pub height: u32,
        pub left: i32,
        pub top: i32,
    }

    struct HintingEntry {
        font_key: u64,
        size: Size,
        coords: Vec<NormalizedCoord>,
        instance: HintingInstance,
        serial: u64,
    }

    /// Small LRU cache of `skrifa` hinting instances partitioned by outline format.
    #[derive(Default)]
    pub(crate) struct FaceScalerCache {
        glyf_entries: Vec<HintingEntry>,
        cff_entries: Vec<HintingEntry>,
        other_entries: Vec<HintingEntry>,
        serial: u64,
    }

    impl FaceScalerCache {
        fn get(
            &mut self,
            font_key: u64,
            outlines: &OutlineGlyphCollection<'_>,
            size: Size,
            coords: &[NormalizedCoord],
        ) -> Option<&HintingInstance> {
            let entries = match outlines.format()? {
                OutlineGlyphFormat::Glyf => &mut self.glyf_entries,
                OutlineGlyphFormat::Cff | OutlineGlyphFormat::Cff2 => &mut self.cff_entries,
                #[allow(unreachable_patterns)]
                _ => &mut self.other_entries,
            };
            let mut oldest_serial = u64::MAX;
            let mut oldest_ix = 0;
            let mut hit_ix = None;
            for (ix, entry) in entries.iter().enumerate() {
                if entry.font_key == font_key && entry.size == size && entry.coords == coords {
                    hit_ix = Some(ix);
                    break;
                }
                if entry.serial < oldest_serial {
                    oldest_serial = entry.serial;
                    oldest_ix = ix;
                }
            }
            self.serial += 1;
            let serial = self.serial;
            if let Some(ix) = hit_ix {
                let entry = entries.get_mut(ix)?;
                entry.serial = serial;
                return Some(&entry.instance);
            }
            if entries.len() < MAX_CACHED_HINT_INSTANCES {
                let instance = HintingInstance::new(outlines, size, coords, HINTING_MODE).ok()?;
                entries.push(HintingEntry {
                    font_key,
                    size,
                    coords: coords.to_vec(),
                    instance,
                    serial,
                });
                return entries.last().map(|entry| &entry.instance);
            }
            let entry = entries.get_mut(oldest_ix)?;
            entry.font_key = font_key;
            entry.size = size;
            entry.coords.clear();
            entry.coords.extend_from_slice(coords);
            entry.serial = serial;
            entry
                .instance
                .reconfigure(outlines, size, coords, HINTING_MODE)
                .ok()?;
            Some(&entry.instance)
        }
    }

    struct PenAdapter<'a, S>(&'a mut S);

    impl<S: FaceOutlineSink> OutlinePen for PenAdapter<'_, S> {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to(x, y);
        }

        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to(x, y);
        }

        fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
            self.0.quad_to(cx0, cy0, x, y);
        }

        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
            self.0.curve_to(cx0, cy0, cx1, cy1, x, y);
        }

        fn close(&mut self) {
            self.0.close();
        }
    }

    /// Reads whether a face carries scalable outlines, COLR/CPAL layers, or color bitmap strikes.
    pub(crate) fn read_raster_caps(data: &[u8], index: u32) -> Option<FaceRasterCaps> {
        let font = FontRef::from_index(data, index).ok()?;
        let has_outlines = font.outline_glyphs().format().is_some();
        let has_color_outlines = font.table_data(Tag::new(b"COLR")).is_some()
            && font.table_data(Tag::new(b"CPAL")).is_some();
        let has_color_bitmaps = BitmapStrikes::with_format(&font, BitmapFormat::Sbix).is_some()
            || BitmapStrikes::with_format(&font, BitmapFormat::Cbdt).is_some();
        Some(FaceRasterCaps {
            has_outlines,
            has_color_outlines,
            has_color_bitmaps,
        })
    }

    fn draw_single_layer(
        outlines: &OutlineGlyphCollection<'_>,
        hinting: Option<&HintingInstance>,
        size: Size,
        coords: &[NormalizedCoord],
        glyph_id: u16,
        color: Option<[u8; 4]>,
        sink: &mut impl FaceOutlineSink,
    ) -> bool {
        let Some(glyph) = outlines.get(GlyphId::from(glyph_id)) else {
            return false;
        };
        sink.begin_layer(color);
        let settings = match hinting {
            Some(instance) => DrawSettings::hinted(instance, false),
            None => DrawSettings::unhinted(size, LocationRef::new(coords)),
        };
        if glyph.draw(settings, &mut PenAdapter(sink)).is_err() {
            return false;
        }
        sink.finish_layer();
        true
    }

    fn resolve_outline_context<'a>(
        font: &'a FontRef<'a>,
        font_key: u64,
        coords: &[AxisCoord],
        size_px: f32,
        hint: bool,
        cache: &'a mut FaceScalerCache,
    ) -> (
        OutlineGlyphCollection<'a>,
        Size,
        skrifa::instance::Location,
        Option<&'a HintingInstance>,
    ) {
        let outlines = font.outline_glyphs();
        let upem = font.head().map(|head| head.units_per_em()).unwrap_or(0);
        let size = if size_px > 0.0 && upem != 0 {
            Size::new(size_px)
        } else {
            Size::unscaled()
        };
        let location = if coords.is_empty() {
            skrifa::instance::Location::default()
        } else {
            font.axes().location(
                coords
                    .iter()
                    .map(|coord| (Tag::new(&coord.tag), coord.value)),
            )
        };
        let hinting = if hint && outlines.format().is_some() {
            cache.get(font_key, &outlines, size, location.coords())
        } else {
            None
        };
        (outlines, size, location, hinting)
    }

    /// Scales a plain single-layer glyph outline into `sink`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scale_plain_outline(
        data: &[u8],
        index: u32,
        font_key: u64,
        coords: &[AxisCoord],
        size_px: f32,
        hint: bool,
        glyph_id: u16,
        cache: &mut FaceScalerCache,
        sink: &mut impl FaceOutlineSink,
    ) -> bool {
        let Ok(font) = FontRef::from_index(data, index) else {
            return false;
        };
        let (outlines, size, location, hinting) =
            resolve_outline_context(&font, font_key, coords, size_px, hint, cache);
        if outlines.format().is_none() {
            return false;
        }
        draw_single_layer(
            &outlines,
            hinting,
            size,
            location.coords(),
            glyph_id,
            None,
            sink,
        )
    }

    /// Scales a COLRv0 layered color glyph into `sink` using CPAL palette 0.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scale_color_outline(
        data: &[u8],
        index: u32,
        font_key: u64,
        coords: &[AxisCoord],
        size_px: f32,
        hint: bool,
        glyph_id: u16,
        cache: &mut FaceScalerCache,
        sink: &mut impl FaceOutlineSink,
    ) -> bool {
        let Ok(font) = FontRef::from_index(data, index) else {
            return false;
        };
        if font.table_data(Tag::new(b"CPAL")).is_none() {
            return false;
        }
        let Ok(colr) = font.colr() else {
            return false;
        };
        let Ok(Some(range)) = colr.v0_base_glyph(GlyphId::from(glyph_id)) else {
            return false;
        };
        let palettes = font.color_palettes();
        let palette = palettes.get(0);
        let (outlines, size, location, hinting) =
            resolve_outline_context(&font, font_key, coords, size_px, hint, cache);
        if outlines.format().is_none() {
            return false;
        }
        for layer_index in range {
            let Ok((layer_gid, palette_index)) = colr.v0_layer(layer_index) else {
                return false;
            };
            let color = if palette_index == 0xFFFF {
                None
            } else {
                palette.as_ref().map(|pal| {
                    pal.colors()
                        .get(usize::from(palette_index))
                        .map_or([0, 0, 0, 0], |entry| {
                            [entry.red(), entry.green(), entry.blue(), entry.alpha()]
                        })
                })
            };
            let Ok(layer_gid_u16) = u16::try_from(layer_gid.to_u32()) else {
                return false;
            };
            if !draw_single_layer(
                &outlines,
                hinting,
                size,
                location.coords(),
                layer_gid_u16,
                color,
                sink,
            ) {
                return false;
            }
        }
        true
    }

    /// Reads the best-fit `sbix` or `CBDT` color bitmap glyph entry at `size_px`.
    pub(crate) fn read_color_bitmap(
        data: &[u8],
        index: u32,
        size_px: f32,
        glyph_id: u16,
    ) -> Option<RawColorBitmap<'_>> {
        if size_px <= 0.0 {
            return None;
        }
        let font = FontRef::from_index(data, index).ok()?;
        let strikes = BitmapStrikes::with_format(&font, BitmapFormat::Sbix)
            .or_else(|| BitmapStrikes::with_format(&font, BitmapFormat::Cbdt))?;
        let glyph = strikes.glyph_for_size(Size::new(size_px), GlyphId::from(glyph_id))?;
        if glyph.ppem_y <= 0.0 {
            return None;
        }
        let payload = match glyph.data {
            BitmapData::Png(bytes) => RawBitmapPayload::Png(bytes),
            BitmapData::Bgra(bytes) => RawBitmapPayload::Bgra(bytes),
            BitmapData::Mask(_) => return None,
        };
        let left = glyph.inner_bearing_x as i32;
        let mut top = glyph.inner_bearing_y as i32;
        if glyph.placement_origin == BitmapOrigin::BottomLeft {
            if top == 0
                && font
                    .localized_strings(StringId::FAMILY_NAME)
                    .english_or_first()
                    .is_some_and(|name| name.chars().eq("Apple Color Emoji".chars()))
            {
                let upem = font
                    .head()
                    .map(|head| head.units_per_em())
                    .unwrap_or(1)
                    .max(1);
                top = (-100.0 * (glyph.ppem_y / f32::from(upem))).round() as i32;
            }
            top += glyph.height as i32;
        }
        Some(RawColorBitmap {
            payload,
            ppem: glyph.ppem_y,
            width: glyph.width,
            height: glyph.height,
            left,
            top,
        })
    }

    #[cfg(test)]
    pub(crate) fn map_char_for_test(data: &[u8], ch: char) -> u16 {
        FontRef::new(data)
            .ok()
            .and_then(|font| font.charmap().map(ch))
            .map_or(0, |glyph| glyph.to_u32() as u16)
    }
}

#[cfg(all(test, feature = "raster"))]
pub(crate) use raster_support::map_char_for_test;
#[cfg(feature = "raster")]
pub(crate) use raster_support::{
    FaceOutlineSink, FaceScalerCache, RawBitmapPayload, read_color_bitmap, read_raster_caps,
    scale_color_outline, scale_plain_outline,
};
