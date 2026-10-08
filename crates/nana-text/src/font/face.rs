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
    Some(RunMetrics {
        ascent_px: metrics.ascent,
        descent_px: metrics.descent.abs(),
        line_gap_px: metrics.leading,
        underline_offset_px,
        underline_thickness_px,
        strikeout_offset_px,
        strikeout_thickness_px,
    })
}

/// Every codepoint the face's best cmap maps, folded into ranges.
pub fn read_coverage(data: &[u8], index: u32) -> CoverageSet {
    let Ok(font) = FontRef::from_index(data, index) else {
        return CoverageSet::default();
    };
    let mut codepoints: Vec<u32> = font
        .charmap()
        .mappings()
        .filter(|(_, glyph)| glyph.to_u32() != 0)
        .map(|(codepoint, _)| codepoint)
        .collect();
    codepoints.sort_unstable();
    codepoints.dedup();
    CoverageSet::from_sorted_codepoints(&codepoints)
}
