//! What a box declares about its own elasticity.

use serde::{Deserialize, Serialize};

use super::{
    AdjustmentKind, AdjustmentSegment, EnvelopeShape, ExecutionClass, LayoutCost, LayoutUnits,
    costs,
};

/// How far one length of a box may close up, and what each pixel of that
/// costs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ElasticLength {
    pub floor: ElasticFloor,
    pub cost: LayoutCost,
}

impl ElasticLength {
    pub const fn new(floor: ElasticFloor, cost: LayoutCost) -> Self {
        Self { floor, cost }
    }

    /// What `resolved` px may give up.
    pub fn capacity(&self, resolved: f32) -> f32 {
        let floor = match self.floor {
            ElasticFloor::Px(px) => px,
            ElasticFloor::Fraction(parts) => resolved * f32::from(parts) / 255.0,
        };
        (resolved - floor.max(0.0)).max(0.0)
    }
}

/// The least a length closes up to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ElasticFloor {
    Px(f32),
    /// Parts in 255 of the resolved length.
    Fraction(u8),
}

/// What a box may give up along one axis: its padding (both edges), the gap
/// between its children, and a gap inside the box that is not a child's (a
/// button's icon against its label).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AxisElasticity {
    pub padding: Option<ElasticLength>,
    pub gap: Option<ElasticLength>,
    pub content_gap: Option<ElasticLength>,
}

impl AxisElasticity {
    pub const NONE: Self = Self {
        padding: None,
        gap: None,
        content_gap: None,
    };

    pub fn is_none(&self) -> bool {
        self.padding.is_none() && self.gap.is_none() && self.content_gap.is_none()
    }
}

/// Whether a box keeps with a neighbour across a break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum KeepWith {
    #[default]
    None,
    Previous,
    Next,
}

/// One discrete arrangement a box may switch to, and what switching saves
/// and costs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DiscreteCandidate {
    pub saves_px: f32,
    pub cost: LayoutCost,
}

/// Discrete arrangements (a compact variant). Explicit opt-in: a solver
/// considers them only when its policy allows structural work. The
/// candidate in use is the component's own state, an input to the solve;
/// `hysteresis_px` is how far past a switch point the extent must move
/// before the component switches back.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DiscreteAdaptation {
    pub candidates: [Option<DiscreteCandidate>; 4],
    pub hysteresis_px: f32,
}

/// What a box declares about its elasticity (Issue #208).
///
/// As a child, `inline` and `block` say what it may give up when its parent's
/// line runs short, and `aggregate_children` says it passes on what its own
/// children declare. As a container, `solve_overflow` says whether its line
/// runs the solver at all when it overflows. A container that does not --
/// most do not -- never reads its children's declarations, so declaring costs
/// nothing until a container asks.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AdaptationProfile {
    pub inline: AxisElasticity,
    pub block: AxisElasticity,
    pub aggregate_children: bool,
    pub solve_overflow: bool,
    pub break_before: Option<LayoutCost>,
    pub break_after: Option<LayoutCost>,
    pub keep_with: KeepWith,
    pub discrete: Option<DiscreteAdaptation>,
}

impl AdaptationProfile {
    /// Declares nothing and solves nothing: what a box with no profile is.
    pub const RIGID: Self = Self {
        inline: AxisElasticity::NONE,
        block: AxisElasticity::NONE,
        aggregate_children: false,
        solve_overflow: false,
        break_before: None,
        break_after: None,
        keep_with: KeepWith::None,
        discrete: None,
    };

    /// What this box declares along an axis.
    pub fn axis(&self, inline: bool) -> &AxisElasticity {
        if inline { &self.inline } else { &self.block }
    }

    /// Padding that closes up to a fraction of itself, at the padding price.
    pub fn padding(fraction: u8) -> ElasticLength {
        ElasticLength::new(ElasticFloor::Fraction(fraction), costs::PADDING)
    }

    /// A gap between children that closes up to a fraction of itself.
    pub fn gap(fraction: u8) -> ElasticLength {
        ElasticLength::new(ElasticFloor::Fraction(fraction), costs::PLACEMENT_GAP)
    }

    /// A gap inside the box that closes up to a fraction of itself.
    pub fn content_gap(fraction: u8) -> ElasticLength {
        ElasticLength::new(ElasticFloor::Fraction(fraction), costs::CONTENT_GAP)
    }
}

/// What a box gives up along one axis, as its parent reads it: its padding
/// (`padding` start + end, resolved px), `gaps` gaps of `gap` px between its
/// children, and `content_gaps` gaps of `content_gap` px inside it. All of it
/// is local-box work for the parent: the box resolves a smaller extent inside
/// itself and its content keeps its size. The single lowering every box goes
/// through; text lowers into the same shape type.
pub fn lower_box(
    elasticity: &AxisElasticity,
    padding: (f32, f32),
    gap: f32,
    gaps: usize,
    content_gap: f32,
    content_gaps: usize,
) -> EnvelopeShape {
    let mut segments = Vec::with_capacity(3);
    if let Some(length) = elasticity.padding {
        let capacity = length.capacity(padding.0.max(0.0)) + length.capacity(padding.1.max(0.0));
        segments.push(AdjustmentSegment::new(
            LayoutUnits::from_px(capacity),
            length.cost,
            AdjustmentKind::Padding,
            ExecutionClass::LocalBox,
        ));
    }
    let mut push = |length: Option<ElasticLength>, resolved: f32, count: f32, kind| {
        let Some(length) = length else {
            return;
        };
        let capacity = LayoutUnits::from_px(length.capacity(resolved) * count);
        segments.push(AdjustmentSegment::new(
            capacity,
            length.cost,
            kind,
            ExecutionClass::LocalBox,
        ));
    };
    push(
        elasticity.gap,
        gap.max(0.0),
        gaps as f32,
        AdjustmentKind::Gap,
    );
    push(
        elasticity.content_gap,
        content_gap.max(0.0),
        content_gaps as f32,
        AdjustmentKind::ContentGap,
    );
    EnvelopeShape::normalized(&mut segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_lowers_padding_gaps_and_content_gaps_into_one_shape() {
        let elasticity = AxisElasticity {
            padding: Some(ElasticLength::new(ElasticFloor::Px(4.0), costs::PADDING)),
            gap: Some(AdaptationProfile::gap(64)),
            content_gap: Some(ElasticLength::new(
                ElasticFloor::Fraction(128),
                costs::CONTENT_GAP,
            )),
        };
        let shape = lower_box(&elasticity, (16.0, 16.0), 8.0, 3, 6.0, 1);
        let capacity = |kind| {
            shape
                .segments()
                .iter()
                .find(|segment| segment.kind == kind)
                .map(|segment| segment.capacity.to_px())
        };
        assert_eq!(capacity(AdjustmentKind::Padding), Some(24.0));
        assert!(
            (capacity(AdjustmentKind::Gap).unwrap() - 3.0 * (8.0 - 8.0 * 64.0 / 255.0)).abs()
                < 0.02
        );
        assert!(
            (capacity(AdjustmentKind::ContentGap).unwrap() - (6.0 - 6.0 * 128.0 / 255.0)).abs()
                < 0.02
        );
        // Cheapest first: gaps, then padding, then the gap inside.
        let kinds: Vec<_> = shape
            .segments()
            .iter()
            .map(|segment| segment.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                AdjustmentKind::Gap,
                AdjustmentKind::Padding,
                AdjustmentKind::ContentGap
            ]
        );
        assert!(lower_box(&AxisElasticity::NONE, (16.0, 16.0), 8.0, 3, 6.0, 1).is_rigid());
    }
}
