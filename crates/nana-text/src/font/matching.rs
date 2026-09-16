//! Picking one face out of a family. CSS Fonts 4 §5.2, deterministic.
//!
//! Narrowing runs `font-stretch`, then `font-style`, then `font-weight`, the
//! order the CSS algorithm specifies; each step keeps only the faces at the
//! best distance and hands them to the next. Variable faces take part as
//! ranges (`wght` / `wdth` axes, `ital` / `slnt` axes), not as their default
//! instance.
//!
//! When faces are still tied after all three steps, the higher
//! [`FontOrigin`](super::FontOrigin) rank wins (host registrations beat the
//! system scan), then the later registration wins (a re-registered or
//! replacement face beats the one it shadows). No step iterates a hash map,
//! so the same face set and query always pick the same face.

use super::query::{FontStretch, FontStyle, FontWeight};
use crate::id::FontId;

/// Which styles a face can render without synthesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StyleSupport {
    pub normal: bool,
    pub italic: bool,
    pub oblique: bool,
}

impl StyleSupport {
    fn supports(self, style: FontStyle) -> bool {
        match style {
            FontStyle::Normal => self.normal,
            FontStyle::Italic => self.italic,
            FontStyle::Oblique => self.oblique,
        }
    }
}

/// A candidate face as matching sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchFace {
    pub id: FontId,
    /// Inclusive CSS weight range the face can render.
    pub weight: (f32, f32),
    /// Inclusive width range, percent.
    pub stretch: (f32, f32),
    pub styles: StyleSupport,
    pub origin_rank: u8,
    pub registration: u64,
}

/// The CSS style fallback order for a requested style.
fn style_order(style: FontStyle) -> [FontStyle; 3] {
    match style {
        FontStyle::Italic => [FontStyle::Italic, FontStyle::Oblique, FontStyle::Normal],
        FontStyle::Oblique => [FontStyle::Oblique, FontStyle::Italic, FontStyle::Normal],
        FontStyle::Normal => [FontStyle::Normal, FontStyle::Oblique, FontStyle::Italic],
    }
}

/// Ranks a face within one fallback tier; `None` rejects it from the tier.
type Tier<'t> = &'t dyn Fn(&MatchFace) -> Option<f32>;

/// Keeps the faces at the smallest `rank`, where `rank` returns `None` for a
/// face the current tier rejects. Tiers are tried in order; the first tier
/// with any face wins.
fn narrow<'a>(faces: Vec<&'a MatchFace>, tiers: &[Tier<'_>]) -> Vec<&'a MatchFace> {
    for tier in tiers {
        let ranked: Vec<(f32, &MatchFace)> = faces
            .iter()
            .filter_map(|face| tier(face).map(|rank| (rank, *face)))
            .collect();
        let Some(best) = ranked
            .iter()
            .map(|(rank, _)| *rank)
            .min_by(|a, b| a.total_cmp(b))
        else {
            continue;
        };
        return ranked
            .into_iter()
            .filter(|(rank, _)| *rank == best)
            .map(|(_, face)| face)
            .collect();
    }
    faces
}

fn contains((min, max): (f32, f32), value: f32) -> bool {
    min <= value && value <= max
}

fn narrow_stretch(faces: Vec<&MatchFace>, stretch: FontStretch) -> Vec<&MatchFace> {
    let s = stretch.0;
    let inside = |face: &MatchFace| contains(face.stretch, s).then_some(0.0);
    let narrower = |face: &MatchFace| (face.stretch.1 < s).then_some(s - face.stretch.1);
    let wider = |face: &MatchFace| (face.stretch.0 > s).then_some(face.stretch.0 - s);
    if s <= 100.0 {
        narrow(faces, &[&inside, &narrower, &wider])
    } else {
        narrow(faces, &[&inside, &wider, &narrower])
    }
}

fn narrow_style(faces: Vec<&MatchFace>, style: FontStyle) -> Vec<&MatchFace> {
    for candidate in style_order(style) {
        let supporting: Vec<&MatchFace> = faces
            .iter()
            .copied()
            .filter(|face| face.styles.supports(candidate))
            .collect();
        if !supporting.is_empty() {
            return supporting;
        }
    }
    faces
}

fn narrow_weight(faces: Vec<&MatchFace>, weight: FontWeight) -> Vec<&MatchFace> {
    let w = weight.0;
    let inside = |face: &MatchFace| contains(face.weight, w).then_some(0.0);
    let below = |face: &MatchFace| (face.weight.1 < w).then_some(w - face.weight.1);
    let above = |face: &MatchFace| (face.weight.0 > w).then_some(face.weight.0 - w);
    if (400.0..=500.0).contains(&w) {
        // Heavier faces up to 500 first, then lighter, then heavier than 500.
        let up_to_500 = |face: &MatchFace| {
            (face.weight.0 > w && face.weight.0 <= 500.0).then_some(face.weight.0 - w)
        };
        narrow(faces, &[&inside, &up_to_500, &below, &above])
    } else if w < 400.0 {
        narrow(faces, &[&inside, &below, &above])
    } else {
        narrow(faces, &[&inside, &above, &below])
    }
}

/// The face CSS matching picks, or `None` for an empty family.
pub fn best_face(
    faces: &[MatchFace],
    weight: FontWeight,
    stretch: FontStretch,
    style: FontStyle,
) -> Option<FontId> {
    let all: Vec<&MatchFace> = faces.iter().collect();
    let narrowed = narrow_weight(narrow_style(narrow_stretch(all, stretch), style), weight);
    narrowed
        .into_iter()
        .max_by_key(|face| (face.origin_rank, face.registration))
        .map(|face| face.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(slot: u32, weight: f32, style: FontStyle) -> MatchFace {
        MatchFace {
            id: FontId::from_parts(slot, 1),
            weight: (weight, weight),
            stretch: (100.0, 100.0),
            styles: StyleSupport {
                normal: style == FontStyle::Normal,
                italic: style == FontStyle::Italic,
                oblique: style == FontStyle::Oblique,
            },
            origin_rank: 0,
            registration: u64::from(slot),
        }
    }

    fn pick(faces: &[MatchFace], weight: f32, style: FontStyle) -> u32 {
        best_face(faces, FontWeight(weight), FontStretch::NORMAL, style)
            .unwrap()
            .index()
    }

    #[test]
    fn weight_fallback_follows_the_css_direction_rules() {
        let faces = [
            face(0, 300.0, FontStyle::Normal),
            face(1, 400.0, FontStyle::Normal),
            face(2, 600.0, FontStyle::Normal),
            face(3, 900.0, FontStyle::Normal),
        ];
        assert_eq!(pick(&faces, 400.0, FontStyle::Normal), 1);
        // 450 with no 450..=500 face falls to lighter first.
        assert_eq!(pick(&faces, 450.0, FontStyle::Normal), 1);
        // Below 400 prefers lighter, above 500 prefers heavier.
        assert_eq!(pick(&faces, 350.0, FontStyle::Normal), 0);
        assert_eq!(pick(&faces, 700.0, FontStyle::Normal), 3);
        assert_eq!(pick(&faces, 950.0, FontStyle::Normal), 3);
    }

    #[test]
    fn style_outranks_weight() {
        let faces = [
            face(0, 700.0, FontStyle::Normal),
            face(1, 300.0, FontStyle::Italic),
        ];
        assert_eq!(pick(&faces, 700.0, FontStyle::Italic), 1);
        assert_eq!(pick(&faces, 300.0, FontStyle::Oblique), 1);
        assert_eq!(pick(&faces, 300.0, FontStyle::Normal), 0);
    }

    #[test]
    fn a_variable_range_containing_the_request_beats_an_exact_static_face_only_on_ties() {
        let mut variable = face(0, 400.0, FontStyle::Normal);
        variable.weight = (100.0, 900.0);
        let exact = face(1, 650.0, FontStyle::Normal);
        // Both contain 650, so the later registration decides.
        assert_eq!(pick(&[variable, exact], 650.0, FontStyle::Normal), 1);
        assert_eq!(
            pick(
                &[exact, {
                    let mut v = variable;
                    v.registration = 9;
                    v
                }],
                650.0,
                FontStyle::Normal
            ),
            0
        );
    }

    #[test]
    fn origin_rank_breaks_ties_before_registration_order() {
        let mut host = face(0, 400.0, FontStyle::Normal);
        host.origin_rank = 2;
        let system = face(1, 400.0, FontStyle::Normal);
        assert_eq!(pick(&[host, system], 400.0, FontStyle::Normal), 0);
    }
}
