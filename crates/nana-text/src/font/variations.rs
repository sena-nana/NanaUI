//! Variation axes and OpenType features, and the coordinates a face ends up
//! rendered at.
//!
//! [`FontVariations`] and [`FontFeatures`] are what the author asked for, tags
//! verbatim. [`FontInstance`] is what one concrete face will be shaped and
//! rasterized at: every axis the face really has, clamped to its range, with
//! everything the face lacks reported and dropped. Its [`FontInstanceKey`] is
//! the value later shape and glyph-raster caches key on.
//!
//! # Coordinate precedence (per axis, lowest to highest)
//!
//! 1. The face's default.
//! 2. What the query implies: `wght` from `font-weight`, `wdth` from
//!    `font-stretch`, `ital = 1` for italic, `slnt = -14` for oblique — only on
//!    faces that have that axis.
//! 3. An explicit `font-variation-settings` value for that axis.
//!
//! An explicit axis the face does not have is **fail-closed**: it lands in
//! [`FontInstance::ignored_axes`] and changes nothing. In particular it is
//! never reinterpreted as `wght`.

use super::face::FaceDetails;
use super::query::{FontQuery, FontStyle, canonical_bits};
use crate::id::{FontGeneration, FontId};
use nana_ui_core::{FontFeatureSetting, FontVariationSetting};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub const WGHT: [u8; 4] = *b"wght";
pub const WDTH: [u8; 4] = *b"wdth";
pub const ITAL: [u8; 4] = *b"ital";
pub const SLNT: [u8; 4] = *b"slnt";

/// CSS's default `oblique` angle, as a `slnt` coordinate (negative is
/// clockwise, i.e. leaning right).
const OBLIQUE_SLNT: f32 = -14.0;

/// One axis coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisCoord {
    pub tag: [u8; 4],
    pub value: f32,
}

impl Eq for AxisCoord {}

impl Hash for AxisCoord {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tag.hash(state);
        canonical_bits(self.value).hash(state);
    }
}

/// One variation axis as a face declares it, in user-space units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FontAxis {
    pub tag: [u8; 4],
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

impl FontAxis {
    pub fn clamp(&self, value: f32) -> f32 {
        value.clamp(self.min, self.max)
    }
}

/// A named instance (`fvar` instance record), e.g. "Bold" at `wght 700`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedInstance {
    /// English subfamily name when the face has one.
    pub name: Option<String>,
    /// User-space coordinate per axis, in the face's axis order.
    pub coords: Vec<AxisCoord>,
}

/// Author-requested axis values, sorted by tag, one value per tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct FontVariations(Vec<AxisCoord>);

impl FontVariations {
    /// Later settings for the same tag win, as in CSS. Non-finite values are
    /// dropped rather than clamped into something the author did not write.
    pub fn from_settings(settings: &[FontVariationSetting]) -> Self {
        let mut coords: Vec<AxisCoord> = Vec::with_capacity(settings.len());
        for setting in settings {
            if !setting.value.is_finite() {
                continue;
            }
            match coords.iter_mut().find(|coord| coord.tag == setting.tag) {
                Some(coord) => coord.value = setting.value,
                None => coords.push(AxisCoord {
                    tag: setting.tag,
                    value: setting.value,
                }),
            }
        }
        coords.sort_by_key(|coord| coord.tag);
        Self(coords)
    }

    pub fn as_slice(&self) -> &[AxisCoord] {
        &self.0
    }

    pub fn get(&self, tag: [u8; 4]) -> Option<f32> {
        self.0
            .iter()
            .find(|coord| coord.tag == tag)
            .map(|coord| coord.value)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One OpenType feature setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FeatureValue {
    pub tag: [u8; 4],
    pub value: u32,
}

/// Author-requested features, sorted by tag, one value per tag. Tags are
/// passed to shaping verbatim; the font system does not interpret them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct FontFeatures(Vec<FeatureValue>);

impl FontFeatures {
    pub fn from_settings(settings: &[FontFeatureSetting]) -> Self {
        let mut features: Vec<FeatureValue> = Vec::with_capacity(settings.len());
        for setting in settings {
            match features
                .iter_mut()
                .find(|feature| feature.tag == setting.tag)
            {
                Some(feature) => feature.value = setting.value,
                None => features.push(FeatureValue {
                    tag: setting.tag,
                    value: setting.value,
                }),
            }
        }
        features.sort_by_key(|feature| feature.tag);
        Self(features)
    }

    pub fn as_slice(&self) -> &[FeatureValue] {
        &self.0
    }
}

/// Styling a renderer has to fake because the chosen face cannot provide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Synthesis {
    /// Asked for >= 600 and the face cannot reach 600.
    pub bold: bool,
    /// Asked for italic or oblique and the face has neither the style nor an
    /// `ital` / `slnt` axis.
    pub oblique: bool,
}

/// Hashable identity of a face at concrete coordinates.
///
/// Two instances that render identically compare equal: coordinates are
/// clamped and default-valued axes are omitted, so `wdth 1000` on a face whose
/// `wdth` stops at 200 keys the same as `wdth 200`. The [`FontId`] carries its
/// slot generation, so a replaced face never aliases the old one's entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FontInstanceKey {
    pub font: FontId,
    pub coords: Arc<[AxisCoord]>,
    pub synthesis: Synthesis,
}

/// A face resolved to the coordinates it will be shaped and rasterized at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontInstance {
    pub key: FontInstanceKey,
    /// The font database generation this was resolved under.
    pub generation: FontGeneration,
    /// Explicit axes the face does not have, sorted. Fail-closed: reported,
    /// never applied, never remapped.
    pub ignored_axes: Vec<[u8; 4]>,
}

impl FontInstance {
    pub fn font(&self) -> FontId {
        self.key.font
    }

    /// Non-default coordinates, sorted by tag.
    pub fn coords(&self) -> &[AxisCoord] {
        &self.key.coords
    }

    pub fn coord(&self, tag: [u8; 4]) -> Option<f32> {
        self.key
            .coords
            .iter()
            .find(|coord| coord.tag == tag)
            .map(|coord| coord.value)
    }
}

pub(crate) struct StaticFaceTraits {
    pub weight_max: f32,
    pub style: FontStyle,
}

pub(crate) fn resolve_instance(
    font: FontId,
    generation: FontGeneration,
    details: &FaceDetails,
    traits: StaticFaceTraits,
    query: &FontQuery,
    variations: &FontVariations,
) -> FontInstance {
    let mut coords: Vec<AxisCoord> = Vec::new();
    for axis in &details.axes {
        let implied = match axis.tag {
            WGHT => Some(query.weight.0),
            WDTH => Some(query.stretch.0),
            ITAL => (query.style == FontStyle::Italic).then_some(1.0),
            SLNT => (query.style == FontStyle::Oblique).then_some(OBLIQUE_SLNT),
            _ => None,
        };
        let Some(value) = variations.get(axis.tag).or(implied) else {
            continue;
        };
        let value = axis.clamp(value);
        if value != axis.default {
            coords.push(AxisCoord {
                tag: axis.tag,
                value,
            });
        }
    }
    coords.sort_by_key(|coord| coord.tag);

    let ignored_axes: Vec<[u8; 4]> = variations
        .as_slice()
        .iter()
        .map(|coord| coord.tag)
        .filter(|tag| details.axis(*tag).is_none())
        .collect();

    let reachable_weight = details
        .axis(WGHT)
        .map_or(traits.weight_max, |axis| axis.max);
    let slanted = matches!(query.style, FontStyle::Italic | FontStyle::Oblique);
    let face_slants = traits.style != FontStyle::Normal
        || details.axis(ITAL).is_some()
        || details.axis(SLNT).is_some_and(|axis| axis.min < 0.0);
    FontInstance {
        key: FontInstanceKey {
            font,
            coords: coords.into(),
            synthesis: Synthesis {
                bold: query.weight.0 >= 600.0 && reachable_weight < 600.0,
                oblique: slanted && !face_slants,
            },
        },
        generation,
        ignored_axes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::query::{FontStretch, FontWeight};
    use crate::font::variations::FontAxis;

    fn details() -> FaceDetails {
        FaceDetails {
            axes: vec![
                FontAxis {
                    tag: WGHT,
                    min: 100.0,
                    default: 400.0,
                    max: 900.0,
                },
                FontAxis {
                    tag: WDTH,
                    min: 50.0,
                    default: 100.0,
                    max: 200.0,
                },
                FontAxis {
                    tag: *b"BEVL",
                    min: 0.0,
                    default: 0.0,
                    max: 100.0,
                },
            ],
            ..FaceDetails::default()
        }
    }

    fn resolve(query: &FontQuery, settings: &[FontVariationSetting]) -> FontInstance {
        resolve_instance(
            FontId::from_parts(0, 1),
            FontGeneration::new(3),
            &details(),
            StaticFaceTraits {
                weight_max: 400.0,
                style: FontStyle::Normal,
            },
            query,
            &FontVariations::from_settings(settings),
        )
    }

    #[test]
    fn explicit_axes_beat_implied_ones_and_unknown_axes_are_reported_not_applied() {
        let query = FontQuery {
            weight: FontWeight(700.0),
            stretch: FontStretch(75.0),
            ..FontQuery::default()
        };
        let instance = resolve(
            &query,
            &[
                FontVariationSetting::new(*b"BEVL", 42.0),
                FontVariationSetting::new(*b"wght", 550.0),
                FontVariationSetting::new(*b"XXXX", 7.0),
            ],
        );
        assert_eq!(instance.coord(WGHT), Some(550.0), "explicit wght wins");
        assert_eq!(instance.coord(WDTH), Some(75.0), "stretch implies wdth");
        assert_eq!(instance.coord(*b"BEVL"), Some(42.0));
        assert_eq!(instance.ignored_axes, vec![*b"XXXX"]);
        assert!(instance.coords().iter().all(|coord| coord.tag != *b"XXXX"));
    }

    #[test]
    fn clamped_and_default_coordinates_key_identically() {
        let query = FontQuery::default();
        let clamped = resolve(&query, &[FontVariationSetting::new(*b"wdth", 1000.0)]);
        let at_max = resolve(&query, &[FontVariationSetting::new(*b"wdth", 200.0)]);
        assert_eq!(clamped.key, at_max.key);
        let default = resolve(&query, &[FontVariationSetting::new(*b"BEVL", 0.0)]);
        assert_eq!(default.key, resolve(&query, &[]).key);
        assert_ne!(
            resolve(&query, &[FontVariationSetting::new(*b"BEVL", 42.0)]).key,
            default.key
        );
    }

    #[test]
    fn later_settings_for_the_same_tag_win() {
        let variations = FontVariations::from_settings(&[
            FontVariationSetting::new(*b"wdth", 80.0),
            FontVariationSetting::new(*b"BEVL", f32::NAN),
            FontVariationSetting::new(*b"wdth", 90.0),
        ]);
        assert_eq!(variations.as_slice().len(), 1);
        assert_eq!(variations.get(WDTH), Some(90.0));
    }
}
