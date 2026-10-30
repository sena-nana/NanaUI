//! The layout recursions' stack (see `recursion`): one level stays within its
//! budget, and no tree is too deep for the thread laying it out.
#![cfg(test)]

use nana_ui_core::{DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec};

use crate::{AppContext, DocumentId, LayoutViewport, Stack, WorkCounters};

/// Most stack one level of each recursion may take, in bytes. Unoptimized
/// code keeps a slot for every local of a function, so a debug level costs
/// several times a release one.
///
/// Measured on x86_64 Windows (MSVC): a debug level was 25.8 KiB of placement
/// and 12.2 KiB of measure while `place_node_scoped` held the whole child walk
/// in one frame; with the walk and the plan paths in their own frames it is
/// 9.0 KiB of placement. Release: 4.8 KiB, then 3.2 KiB, and 2.5 KiB of
/// measure. Each budget leaves about a quarter of headroom over that.
struct Budget {
    measure: usize,
    placement: usize,
}

const BUDGET: Budget = if cfg!(debug_assertions) {
    Budget {
        measure: 16 << 10,
        placement: 12 << 10,
    }
} else {
    Budget {
        measure: 4 << 10,
        placement: 4 << 10,
    }
};

/// The containers a level can be: block, flex column, single-track grid.
const DISPLAYS: [DisplaySpec; 3] = [DisplaySpec::Block, DisplaySpec::Flex, DisplaySpec::Grid];

/// `depth` boxes inside one another, cycling through `displays`, laid out in
/// full.
fn lay_out_nested(depth: usize, displays: &[DisplaySpec]) -> WorkCounters {
    let document = DocumentId::new(7).unwrap();
    let mut cx = AppContext::new();
    let mut parent = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(800.0)))
        .unwrap();
    for level in 0..depth {
        let display = displays[level % displays.len()];
        let layout = LayoutStyle {
            display: Some(display),
            direction: Some(FlexDirection::Column),
            grid_columns: (display == DisplaySpec::Grid).then(|| vec![GridTrack::Fr(1.0)]),
            padding: Some(LengthSpec::Px(0.25)),
            min_height: Some(LengthSpec::Px(1.0)),
            ..Default::default()
        };
        let child = cx
            .create_detached_component(document, Stack::from_layout(layout))
            .unwrap();
        cx.append_child(parent, child).unwrap();
        parent = child;
    }
    cx.layout_document(document, LayoutViewport::new(800.0, 600.0))
        .unwrap();
    let counters = cx.last_work_counters();
    // Tear the tree down here: dropping it is as deep as building it.
    drop(cx);
    counters
}

#[test]
fn a_layout_level_stays_within_its_stack_budget() {
    if stacker::remaining_stack().is_none() {
        // The platform cannot tell what is left, so nothing is priced.
        return;
    }
    for display in DISPLAYS {
        let counters = lay_out_nested(48, &[display]);
        let (measure, placement) = (
            counters.layout_measure_stack_per_level,
            counters.layout_placement_stack_per_level,
        );
        assert!(
            measure > 0 && placement > 0,
            "{display:?} unpriced: {measure} / {placement}"
        );
        assert!(
            measure <= BUDGET.measure,
            "a {display:?} measure level takes {measure} bytes, over its {} budget",
            BUDGET.measure
        );
        assert!(
            placement <= BUDGET.placement,
            "a {display:?} placement level takes {placement} bytes, over its {} budget",
            BUDGET.placement
        );
    }
}

#[test]
fn a_tree_deeper_than_the_stack_still_lays_out() {
    // Unguarded, 400 levels need several MiB of stack in a debug build;
    // this thread has the 1 MiB of a Windows main thread.
    let counters = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| lay_out_nested(400, &DISPLAYS))
        .unwrap()
        .join()
        .expect("layout ran out of stack");
    assert!(counters.layout_placement_nodes > 400);
}

/// A level's price is reported, not only checked: `cargo test -p
/// nana-ui-runtime stack_budget -- --nocapture` prints it.
#[test]
fn report_layout_stack_per_level() {
    for display in DISPLAYS {
        let counters = lay_out_nested(48, &[display]);
        println!(
            "layout stack per level, {display:?} ({}): measure {} B, placement {} B",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            counters.layout_measure_stack_per_level,
            counters.layout_placement_stack_per_level,
        );
    }
}
