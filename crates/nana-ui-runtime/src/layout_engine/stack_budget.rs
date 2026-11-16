//! The layout recursions' stack (see `recursion`): one level stays within its
//! budget, and no tree is too deep for the thread laying it out.
#![cfg(test)]

use nana_ui_core::{DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec};

use crate::layout_engine::LayoutExecutionStats;
use crate::{AppContext, DocumentId, LayoutViewport, Stack};

/// Most stack one level of each recursion may take, in bytes. Unoptimized
/// code keeps a slot for every local of a function, so a debug level costs
/// several times a release one.
///
/// Measured on x86_64 Windows (MSVC): a debug level was 25.8 KiB of placement
/// and 12.2 KiB of measure while `place_node_scoped` held the whole child walk
/// in one frame; with the walk and the plan paths in their own frames it is
/// 9.0 KiB of placement. Release: 4.8 KiB, then 3.2 KiB, and 2.5 KiB of
/// measure. Each budget leaves about a quarter of headroom over that.
///
/// On aarch64 macOS a release placement level was 2.7 KiB, and is 3.5 KiB
/// since a level splits into the head that writes its box and the body that
/// places its children (a boundary resumes at the body); debug placement is
/// 10.9 KiB and measure 11.9 KiB. Placement no longer runs deeper than
/// `recursion::SPAN` levels, so its level price bounds its whole stack.
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

/// The containers a level can be: flex, grid, layers.
const DISPLAYS: [DisplaySpec; 3] = [DisplaySpec::Flex, DisplaySpec::Grid, DisplaySpec::Layers];

/// `depth` boxes inside one another, cycling through `displays`, laid out in
/// full.
fn lay_out_nested(depth: usize, displays: &[DisplaySpec]) -> LayoutExecutionStats {
    let document = DocumentId::new(7).unwrap();
    let mut cx = AppContext::new();
    let mut parent = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(800.0)))
        .unwrap();
    for level in 0..depth {
        let display = displays[level % displays.len()];
        let mut layout = LayoutStyle {
            display: Some(display),
            direction: Some(FlexDirection::Column),
            padding: Some(LengthSpec::Px(0.25)),
            min_height: Some(LengthSpec::Px(1.0)),
            ..Default::default()
        };
        if display == DisplaySpec::Grid {
            layout.grid.columns = Some(vec![GridTrack::Fr(1.0)]);
        }
        let child = cx
            .create_detached_component(document, Stack::from_layout(layout))
            .unwrap();
        cx.append_child(parent, child).unwrap();
        parent = child;
    }
    cx.layout_document(document, LayoutViewport::new(800.0, 600.0))
        .unwrap();
    let stats = cx.layout_execution_stats(document);
    // Tear the tree down here: dropping it is as deep as building it.
    drop(cx);
    stats
}

#[test]
fn a_layout_level_stays_within_its_stack_budget() {
    if crate::stack::remaining().is_none() {
        // The platform cannot tell what is left, so nothing is priced.
        return;
    }
    for display in DISPLAYS {
        let stats = lay_out_nested(48, &[display]);
        let (measure, placement) = (
            stats.measure_stack_per_level,
            stats.placement_stack_per_level,
        );
        assert!(
            placement > 0,
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
    let stats = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| lay_out_nested(400, &DISPLAYS))
        .unwrap()
        .join()
        .expect("layout ran out of stack");
    assert!(stats.placement_nodes > 400);
}

#[test]
fn max_tree_depth_fits_in_384_kib_stack() {
    let stats = std::thread::Builder::new()
        .stack_size(384 * 1024)
        .spawn(|| lay_out_nested(511, &DISPLAYS))
        .unwrap()
        .join()
        .expect("512-node deep layout ran out of 384 KiB stack");
    assert!(stats.placement_nodes >= 512);
}

/// A level's price is reported, not only checked: `cargo test -p
/// nana-ui-runtime stack_budget -- --nocapture` prints it.
#[test]
fn report_layout_stack_per_level() {
    for display in DISPLAYS {
        let stats = lay_out_nested(48, &[display]);
        println!(
            "layout stack per level, {display:?} ({}): measure {} B, placement {} B",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            stats.measure_stack_per_level,
            stats.placement_stack_per_level,
        );
    }
}
