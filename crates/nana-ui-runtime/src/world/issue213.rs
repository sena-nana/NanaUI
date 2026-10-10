//! Issue #213: envelopes and line solves are kept across passes. A resize
//! whose deficit stays in the cost level the last solve stopped in only
//! re-shares that level; a resize that crosses a level solves again from
//! nothing. Either way the answer is the cold one, whatever the history.
//!
//! The toolbar of `issue212`: from 442 px down to 298 px the deficit falls
//! in the items' padding level (the gaps are spent at 442).

#![cfg(test)]

use super::issue212::{Bar, Toolbar};
use super::reflow_oracle::assert_matches_cold;

/// Inside one level, every frame after the first re-shares the held solve:
/// no envelope is built or rebuilt, no cold solve runs. Going out and back
/// gives the same boxes at the same width.
#[test]
fn issue213_a_resize_inside_one_level_only_re_shares_it() {
    let mut toolbar = Toolbar::new(Bar::new(320.0));
    let mut out = Vec::new();
    for width in (321..=400).map(|width| width as f32) {
        let counters = toolbar.resize(width).dynamic;
        assert_eq!(
            counters.envelope_misses + counters.envelope_rebuilds,
            0,
            "{width}"
        );
        assert!(counters.envelope_hits > 0, "{width}");
        assert_eq!(counters.cold_solves, 0, "{width}: {counters:?}");
        assert!(counters.incremental_assignments > 0, "{width}");
        assert_eq!(counters.budget_fallbacks, 0);
        out.push((width, toolbar.item_boxes()));
    }
    for (width, boxes) in out.iter().rev().skip(1) {
        toolbar.resize(*width);
        assert_eq!(&toolbar.item_boxes(), boxes, "{width} back");
    }
    let mut cold = Toolbar::new(Bar::new(321.0));
    assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
}

/// Crossing from the padding level into the gap level solves from nothing
/// once; staying there re-shares again.
#[test]
fn issue213_crossing_a_level_solves_once_from_nothing() {
    let mut toolbar = Toolbar::new(Bar::new(400.0));
    let crossing = toolbar.resize(450.0).dynamic;
    assert_eq!(crossing.cold_solves, 1, "{crossing:?}");
    let staying = toolbar.resize(455.0).dynamic;
    assert_eq!(staying.cold_solves, 0, "{staying:?}");
    assert!(staying.incremental_assignments > 0);
    let mut cold = Toolbar::new(Bar::new(455.0));
    assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
}

/// A width reached from above and from below lays out the same: the held
/// solve is a hint, never part of the answer.
#[test]
fn issue213_the_answer_does_not_depend_on_where_a_resize_came_from() {
    for width in [300.0, 350.0, 441.0, 445.0, 470.0] {
        let mut from_above = Toolbar::new(Bar::new(600.0));
        from_above.resize(width);
        let mut from_below = Toolbar::new(Bar::new(200.0));
        from_below.resize(width);
        let fresh = Toolbar::new(Bar::new(width));
        assert_eq!(
            from_above.item_boxes(),
            fresh.item_boxes(),
            "{width} from above"
        );
        assert_eq!(
            from_below.item_boxes(),
            fresh.item_boxes(),
            "{width} from below"
        );
    }
}
