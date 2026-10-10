//! Dynamic Layout (Issue #207): what a box can give up when its line does
//! not fit, what giving it up costs, and how a line chooses.
//!
//! A child exports a bounded [`EnvelopeShape`]: segments of capacity, each
//! with a [`LayoutCost`] per unit taken and the [`ExecutionClass`] of work
//! taking it needs. A container whose line overflows asks its children for
//! their shapes -- summaries, never a trial layout -- and [`solve_deficit`]
//! takes the cheapest capacity first, never opening a dearer class than the
//! deficit needs. The parent then assigns each child an extent and the child
//! resolves it inside itself: the parent never edits a child's padding.
//!
//! Everything here is integer arithmetic on [`LayoutUnits`] and ordered
//! integer costs, in a canonical participant order, so the same inputs give
//! the same answer whatever order they were built or resized in. Costs say
//! how much a result looks worse, not how long it takes to compute; the
//! execution class says that.
//!
//! The contract is the one Text (#211), components and containers share: a
//! context reads only what it understands of a shape, and there is no second
//! cost type.

mod aggregate;
mod cost;
mod envelope;
mod profile;
mod segment;
mod solver;
mod units;

pub use aggregate::{aggregate_parallel, aggregate_sequential};
pub use cost::{LayoutCost, TotalCost, costs};
pub use envelope::{AdjustmentEnvelope, EnvelopeShape};
pub use profile::{
    AdaptationProfile, AxisElasticity, DiscreteAdaptation, DiscreteCandidate, ElasticFloor,
    ElasticLength, KeepWith, lower_box,
};
pub use segment::{
    AdjustmentKind, AdjustmentSegment, ExecutionClass, MAX_ENVELOPE_SEGMENTS, SegmentList,
};
pub use solver::{
    Participant, ParticipantSource, SolveFrontier, SolveOutcome, SolverBudget, SolverPolicy,
    SolverScratch, reassign_within_frontier, solve_deficit,
};
pub use units::LayoutUnits;
