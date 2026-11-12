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
    BufferFlags, Direction, Feature, FontRef, Language, Script, ShapeOptions, ShapePlan,
    ShapePlanKey, ShaperData, ShaperInstance, Tag, UnicodeBuffer,
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

/// Shape plans a face keeps. Compiling one costs about as much as shaping a
/// short line, and a face meets few distinct script, direction, language,
/// feature and instance combinations.
const PLANS_PER_FACE: usize = 8;

const MAX_SHAPER_FACES: usize = 64;

struct FaceShaper {
    data: FontData,
    shaper_data: ShaperData,
    /// Most recently used last.
    plans: Vec<ShapePlan>,
    last_used: u64,
}

/// Per-face HarfRust acceleration data. Built once per face and dropped
/// wholesale when the font generation moves, so a replaced face never shapes
/// with a retired face's tables.
#[derive(Default)]
pub struct FaceShapers {
    faces: HashMap<FontId, FaceShaper>,
    tick: u64,
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
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        if !self.faces.contains_key(&font) && self.faces.len() >= MAX_SHAPER_FACES {
            if let Some((&evict, _)) = self.faces.iter().min_by_key(|(_, face)| face.last_used) {
                self.faces.remove(&evict);
            }
        }
        let face = match self.faces.entry(font) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                let face = entry.into_mut();
                face.last_used = tick;
                face
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let data = load()?;
                let font_ref = FontRef::from_index(data.bytes(), data.index()).ok()?;
                let shaper_data = ShaperData::new(&font_ref);
                entry.insert(FaceShaper {
                    data,
                    shaper_data,
                    plans: Vec::new(),
                    last_used: tick,
                })
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

        let (direction, script, language) =
            (buffer.direction(), Some(buffer.script()), buffer.language());
        let key = ShapePlanKey::new(script, direction)
            .language(language.as_ref())
            .instance(Some(&instance))
            .features(&features);
        match face.plans.iter().position(|plan| key.matches(plan)) {
            Some(at) => {
                let plan = face.plans.remove(at);
                face.plans.push(plan);
            }
            None => {
                if face.plans.len() == PLANS_PER_FACE {
                    face.plans.remove(0);
                }
                face.plans.push(ShapePlan::new(
                    &shaper,
                    direction,
                    script,
                    language.as_ref(),
                    &features,
                ));
            }
        }
        let output = shaper.shape(
            buffer,
            ShapeOptions::new()
                .plan(face.plans.last())
                .features(&features),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{FontSystem, font_blob};

    fn face() -> (FontId, FontData) {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/noto-sans-arabic.ttf");
        let mut fonts = FontSystem::hermetic();
        let font = fonts
            .register_bytes(font_blob(std::fs::read(path).unwrap()), &Default::default())
            .unwrap()
            .faces[0];
        (font, fonts.face_data(font).unwrap())
    }

    fn input<'a>(
        text: &'a str,
        language: Option<&'a str>,
        features: &'a [FeatureValue],
    ) -> ShapeInput<'a> {
        ShapeInput {
            text,
            range: 0..text.len(),
            rtl: true,
            vertical: false,
            script: Some(ScriptTag(*b"Arab")),
            language,
            features,
            disable_kerning: false,
            coords: &[],
            size_px: 16.0,
        }
    }

    #[test]
    fn a_face_reuses_one_plan_per_shaping_setup_and_keeps_a_few() {
        let (font, data) = face();
        let shape = |shapers: &mut FaceShapers, text, language, features| {
            shapers
                .shape(
                    font,
                    || Some(data.clone()),
                    &input(text, language, features),
                )
                .unwrap()
        };
        let plans = |shapers: &FaceShapers| shapers.faces[&font].plans.len();
        let mut shapers = FaceShapers::default();
        shape(&mut shapers, "سلام عليكم", None, &[]);
        // Same setup, other text: shaped with the cached plan, the glyphs a
        // plan of its own gives.
        let cached = shape(&mut shapers, "مرحبا", None, &[]);
        let fresh = shape(&mut FaceShapers::default(), "مرحبا", None, &[]);
        assert!(!cached.is_empty());
        assert_eq!(cached, fresh);
        assert_eq!(plans(&shapers), 1);
        // A language or a feature change is another plan.
        shape(&mut shapers, "مرحبا", Some("fa"), &[]);
        let no_liga = [FeatureValue {
            tag: *b"liga",
            value: 0,
        }];
        shape(&mut shapers, "مرحبا", None, &no_liga);
        assert_eq!(plans(&shapers), 3);
        for language in ["ar", "ur", "ps", "ku", "sd", "ug", "ks", "bal"] {
            shape(&mut shapers, "مرحبا", Some(language), &[]);
        }
        assert_eq!(plans(&shapers), PLANS_PER_FACE);
    }

    #[test]
    fn face_shapers_evict_lru_faces_past_cap() {
        let (_, data) = face();
        let mut shapers = FaceShapers::default();
        for index in 0..((MAX_SHAPER_FACES + 8) as u32) {
            let id = FontId::from_parts(index, 1);
            shapers
                .shape(id, || Some(data.clone()), &input("مرحبا", None, &[]))
                .unwrap();
        }
        assert_eq!(shapers.faces.len(), MAX_SHAPER_FACES);
        assert!(!shapers.faces.contains_key(&FontId::from_parts(0, 1)));
        assert!(
            shapers
                .faces
                .contains_key(&FontId::from_parts((MAX_SHAPER_FACES + 7) as u32, 1))
        );
    }
}
