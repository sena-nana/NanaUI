//! OpenType shaping via HarfRust. The only file allowed to name it.
//!
//! Everything a run needs is passed explicitly — face bytes and variation
//! coordinates, direction, script, language, features — so nothing is left to
//! HarfRust's segment guessing except a script the caller could not determine.
//! Clusters are byte offsets into the **whole** text, and the text around the
//! item is handed over as pre/post context, so contextual forms (Arabic
//! joining across a font or style boundary) come out as they would unsplit.

use crate::font::{AxisCoord, FeatureValue, FontData};
use crate::id::FontId;
use crate::shape::ScriptTag;
use harfrust::{
    BufferFlags, Direction, Feature, FontRef, Language, Script, ShapeOptions, ShaperData,
    ShaperInstance, Tag, UnicodeBuffer,
};
use std::collections::HashMap;
use std::ops::Range;

/// One glyph as HarfRust produced it, already scaled to px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawGlyph {
    pub glyph_id: u32,
    /// Byte offset into the whole text.
    pub cluster: u32,
    /// Advance **along the line**. For a [`ShapeInput::vertical`] item that is
    /// the glyph's vertical advance, turned positive: HarfRust reports it as a
    /// negative `y_advance`, and a line breaker that measured it that way
    /// would think every upright glyph shrinks the line.
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// What one shaping call needs besides the face.
pub struct ShapeInput<'a> {
    /// The whole text, so the item keeps its context.
    pub text: &'a str,
    pub range: Range<usize>,
    pub rtl: bool,
    /// Shape top-to-bottom with the face's vertical metrics and `vert` forms:
    /// an upright run of a vertical line (#59). Overrides `rtl`, which a
    /// vertical direction has no room for.
    pub vertical: bool,
    pub script: Option<ScriptTag>,
    pub language: Option<&'a str>,
    pub features: &'a [FeatureValue],
    pub disable_kerning: bool,
    pub coords: &'a [AxisCoord],
    pub size_px: f32,
}

struct FaceShaper {
    data: FontData,
    shaper_data: ShaperData,
}

/// Per-face HarfRust acceleration data. Built once per face and dropped
/// wholesale when the font generation moves, so a replaced face never shapes
/// with a retired face's tables.
#[derive(Default)]
pub struct FaceShapers {
    faces: HashMap<FontId, FaceShaper>,
}

impl FaceShapers {
    pub fn clear(&mut self) {
        self.faces.clear();
    }

    /// Shapes one item, or `None` when the face's bytes cannot be read.
    pub fn shape(
        &mut self,
        font: FontId,
        load: impl FnOnce() -> Option<FontData>,
        input: &ShapeInput<'_>,
    ) -> Option<Vec<RawGlyph>> {
        let face = match self.faces.entry(font) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let data = load()?;
                let font_ref = FontRef::from_index(data.bytes(), data.index()).ok()?;
                let shaper_data = ShaperData::new(&font_ref);
                entry.insert(FaceShaper { data, shaper_data })
            }
        };
        let font_ref = FontRef::from_index(face.data.bytes(), face.data.index()).ok()?;
        let instance = ShaperInstance::from_variations(
            &font_ref,
            input
                .coords
                .iter()
                .map(|coord| (Tag::new(&coord.tag), coord.value)),
        );
        let shaper = face
            .shaper_data
            .shaper(&font_ref)
            .instance(Some(&instance))
            .build();

        let mut buffer = UnicodeBuffer::new();
        for (offset, ch) in input.text[input.range.clone()].char_indices() {
            buffer.add(ch, (input.range.start + offset) as u32);
        }
        // Context after `add`: adding resets the post-context.
        buffer.set_pre_context(&input.text[..input.range.start]);
        buffer.set_post_context(&input.text[input.range.end..]);
        let mut flags = BufferFlags::empty();
        if input.range.start == 0 {
            flags |= BufferFlags::BEGINNING_OF_TEXT;
        }
        if input.range.end == input.text.len() {
            flags |= BufferFlags::END_OF_TEXT;
        }
        buffer.set_flags(flags);
        buffer.set_direction(if input.vertical {
            Direction::TopToBottom
        } else if input.rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        });
        if let Some(script) = input
            .script
            .and_then(|script| Script::from_iso15924_tag(Tag::new(&script.0)))
        {
            buffer.set_script(script);
        }
        if let Some(language) = input
            .language
            .and_then(|language| language.parse::<Language>().ok())
        {
            buffer.set_language(language);
        }
        buffer.guess_segment_properties();

        let mut features: Vec<Feature> = input
            .features
            .iter()
            .map(|feature| Feature::new(Tag::new(&feature.tag), feature.value, ..))
            .collect();
        if input.disable_kerning {
            features.push(Feature::new(Tag::new(b"kern"), 0, ..));
        }

        let output = shaper.shape(buffer, ShapeOptions::new().features(&features));
        let upem = shaper.units_per_em();
        let scale = if upem > 0 {
            input.size_px / upem as f32
        } else {
            0.0
        };
        Some(
            output
                .glyph_infos()
                .iter()
                .zip(output.glyph_positions())
                .map(|(info, position)| {
                    let (along, across) = if input.vertical {
                        (-position.y_advance, position.x_advance)
                    } else {
                        (position.x_advance, position.y_advance)
                    };
                    RawGlyph {
                        glyph_id: info.glyph_id,
                        cluster: info.cluster,
                        x_advance: along as f32 * scale,
                        y_advance: across as f32 * scale,
                        x_offset: position.x_offset as f32 * scale,
                        y_offset: position.y_offset as f32 * scale,
                    }
                })
                .collect(),
        )
    }
}
