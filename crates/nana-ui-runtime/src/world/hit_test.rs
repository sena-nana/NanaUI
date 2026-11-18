//! Incremental hit index and transformed pointer queries.

use super::*;

mod bounds;
mod build;
mod candidates;
mod clip;
mod in_place;
mod index;
mod patch;
mod query;
mod scroll;
mod space;
#[cfg(any(test, feature = "layout-verify", debug_assertions))]
mod verify;

use bounds::{Bounds, BoundsTree};
use clip::*;
use index::*;
use query::*;
use space::*;

pub(super) use candidates::HitScratch;
pub(super) use clip::HitPathClip;
#[cfg(test)]
pub(super) use index::find_hit_transform;
pub(super) use index::{HitIndex, count_hit_entries, retain_hit_tree, sort_hit_children};
pub(super) use space::{project_transformed_box, then_hit, transformed_contains};

#[cfg(test)]
mod build_tests;

#[cfg(test)]
mod presentation_tests;

#[cfg(test)]
mod moved_scroll_tests;

#[cfg(test)]
mod moved_turned_tests;

#[cfg(test)]
mod scroll_invalidation_tests;
