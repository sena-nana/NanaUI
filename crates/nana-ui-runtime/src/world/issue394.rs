//! Issue #394: a flex container's baseline, read off where placement puts
//! its items (`layout_engine::arrange_flow`) rather than laid out again.
//!
//! On a flex row aligning first baselines, a flex container sits by the
//! line nearest the top of the page: by the baseline its items share under
//! `align-self: baseline`, or else by its item nearest the inline start,
//! wherever its alignment, its direction or `wrap-reverse` put that item.
//! A column sits by its topmost item. A box holding no line box sits by its
//! border-box bottom. Chrome's numbers for the same trees are the
//! css-parity cases `T-FX01`..`T-FX03`.
#![cfg(test)]

use nana_ui_core::{AlignSpec, DisplaySpec, FlexDirection, FlexWrap, JustifySpec, LayoutStyle};
use nana_ui_core::{LengthSpec, LineHeightSpec};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, measured, product_frame, styled,
};
use crate::{AppContext, DocumentId, LayoutViewport, MutationQueue, StableNodeId};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(900.0, 900.0)
}

/// What the steps of the gate change.
#[derive(Clone, Copy, Debug)]
struct Page {
    /// The second shared item's label's font size.
    shared_font: f32,
    /// The centred container's first label's font size.
    centred_font: f32,
    /// How the centred container aligns its items.
    centred: AlignSpec,
}

const FIRST: Page = Page {
    shared_font: 16.0,
    centred_font: 16.0,
    centred: AlignSpec::Center,
};

/// The ids the gate reads.
struct Ids {
    /// The row's 10×10 empty reference box, which sits by its bottom.
    reference: StableNodeId,
    /// Per container, the label it sits by and one it does not.
    pairs: Vec<(&'static str, StableNodeId, Option<StableNodeId>)>,
    shared_label: StableNodeId,
    centred_label: StableNodeId,
    centred: StableNodeId,
    /// A column beside the row the steps never touch.
    unrelated: StableNodeId,
}

fn flex(edit: impl FnOnce(&mut LayoutStyle)) -> LayoutStyle {
    let mut layout = LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    };
    edit(&mut layout);
    layout
}

fn sized(width: f32, height: Option<f32>) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: height.map(LengthSpec::Px),
        ..LayoutStyle::default()
    }
}

/// A flex row aligning first baselines: the reference, then
/// - `shared`: two items sharing a baseline, under 7px of padding;
/// - `centred`: two fixed boxes centred (or `page.centred`) in 100px;
/// - `reversed`: `row-reverse`, centred, its leftmost item the second;
/// - `column`: packed to its end, its topmost item the first;
/// - `wrapped`: `wrap-reverse` into three lines, its top line the last;
/// - `empty-first`: an empty box first, at the end of 100px.
fn page(page: Page) -> (AppContext, Ids) {
    let (mut b, root) = Builder::page(document(), 1, 880.0);
    let row = b.element(
        root,
        flex(|layout| {
            layout.align_items = AlignSpec::Baseline;
            layout.width = Some(LengthSpec::Px(860.0));
        }),
    );
    let reference = b.element(row, sized(10.0, Some(10.0)));
    let font = |size: f32| LayoutStyle {
        font_size: Some(size),
        line_height: Some(LineHeightSpec::Relative(1.2)),
        ..LayoutStyle::default()
    };
    let labelled = |b: &mut Builder, parent, layout, text: &str, size| {
        let holder = b.element(parent, layout);
        let label = b.label(holder, text);
        b.queue.set_style(label, styled(font(size)));
        label
    };
    let mut pairs = Vec::new();

    let shared = b.element(
        row,
        flex(|layout| {
            layout.align_items = AlignSpec::Baseline;
            layout.padding_top = Some(LengthSpec::Px(7.0));
            layout.width = Some(LengthSpec::Px(130.0));
        }),
    );
    let first = labelled(&mut b, shared, sized(60.0, None), "P", 12.0);
    let shared_label = labelled(&mut b, shared, sized(60.0, None), "Q", page.shared_font);
    pairs.push(("shared", first, None));
    pairs.push(("shared", shared_label, None));

    let centred = b.element(
        row,
        flex(|layout| {
            layout.align_items = page.centred;
            layout.width = Some(LengthSpec::Px(130.0));
            layout.height = Some(LengthSpec::Px(100.0));
        }),
    );
    let centred_label = labelled(
        &mut b,
        centred,
        sized(60.0, Some(40.0)),
        "C",
        page.centred_font,
    );
    let other = labelled(&mut b, centred, sized(60.0, Some(60.0)), "D", 24.0);
    pairs.push(("centred", centred_label, Some(other)));

    let reversed = b.element(
        row,
        flex(|layout| {
            layout.flex_reverse = true;
            layout.align_items = AlignSpec::Center;
            layout.width = Some(LengthSpec::Px(130.0));
            layout.height = Some(LengthSpec::Px(100.0));
        }),
    );
    let right = labelled(&mut b, reversed, sized(60.0, Some(40.0)), "R", 16.0);
    let left = labelled(&mut b, reversed, sized(60.0, Some(60.0)), "L", 24.0);
    pairs.push(("reversed", left, Some(right)));

    let column = b.element(
        row,
        flex(|layout| {
            layout.direction = Some(FlexDirection::Column);
            layout.justify_content = JustifySpec::End;
            layout.width = Some(LengthSpec::Px(100.0));
            layout.height = Some(LengthSpec::Px(140.0));
        }),
    );
    let top = labelled(&mut b, column, sized(60.0, None), "T", 16.0);
    let below = labelled(&mut b, column, sized(60.0, None), "B", 24.0);
    pairs.push(("column", top, Some(below)));

    let wrapped = b.element(
        row,
        flex(|layout| {
            layout.flex_wrap = FlexWrap::WrapReverse;
            layout.align_items = AlignSpec::Baseline;
            layout.width = Some(LengthSpec::Px(100.0));
        }),
    );
    let first_line = labelled(&mut b, wrapped, sized(60.0, None), "W", 16.0);
    labelled(&mut b, wrapped, sized(60.0, None), "X", 16.0);
    let top_line = labelled(&mut b, wrapped, sized(60.0, None), "Y", 24.0);
    pairs.push(("wrapped", top_line, Some(first_line)));

    let empty_first = b.element(
        row,
        flex(|layout| {
            layout.align_items = AlignSpec::End;
            layout.width = Some(LengthSpec::Px(100.0));
            layout.height = Some(LengthSpec::Px(100.0));
        }),
    );
    let empty = b.element(empty_first, sized(30.0, Some(30.0)));
    let after = labelled(&mut b, empty_first, sized(60.0, None), "E", 16.0);
    pairs.push(("empty-first", empty, Some(after)));

    let unrelated = b.element(root, sized(400.0, None));
    for index in 0..20 {
        b.label(unrelated, &format!("Unrelated {index}"));
    }
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    product_frame(
        &mut context,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
    );
    (
        context,
        Ids {
            reference,
            pairs,
            shared_label,
            centred_label,
            centred,
            unrelated,
        },
    )
}

/// Each container's sitting label (or empty box, by its bottom) is on the
/// reference box's bottom, and its other one is not.
fn assert_on_the_reference(context: &AppContext, ids: &Ids) {
    let world = context.world();
    let bottom = |id: StableNodeId| {
        let layout = world.layout_box(id).unwrap();
        layout.y + layout.height
    };
    let baseline = |id: StableNodeId| {
        if matches!(world.node(id).unwrap().kind, super::NodeKind::Text) {
            world
                .layout_result(id)
                .and_then(|result| result.first_baseline)
                .unwrap_or_else(|| panic!("{id:?} publishes no baseline"))
        } else {
            bottom(id)
        }
    };
    let reference = bottom(ids.reference);
    for &(name, sitting, other) in &ids.pairs {
        assert!(
            (baseline(sitting) - reference).abs() < 0.01,
            "{name}: {} vs the reference's bottom {reference}",
            baseline(sitting)
        );
        if let Some(other) = other {
            assert!(
                (baseline(other) - reference).abs() > 1.0,
                "{name}: the other item is on the baseline too"
            );
        }
    }
}

fn set_font(queue: &mut MutationQueue, label: StableNodeId, size: f32) {
    queue.set_style(
        label,
        styled(LayoutStyle {
            font_size: Some(size),
            line_height: Some(LineHeightSpec::Relative(1.2)),
            ..LayoutStyle::default()
        }),
    );
}

/// A flex container on a baseline-aligned row sits by its top line's shared
/// baseline or by its start item's, where placement put them (#394). A
/// bigger font inside a fixed centred item, a bigger font in a shared item
/// and a new alignment each align the row again, as a cold layout does,
/// and lay out nothing beside the row.
#[test]
fn issue394_a_flex_container_sits_by_its_top_lines_baseline() {
    let mut shaper = bundled_face_shaper();
    let (mut warm, ids) = page(FIRST);
    assert_on_the_reference(&warm, &ids);
    let (mut cold, _) = page(FIRST);
    assert_matches_cold(&mut warm, &mut cold, document());

    let steps = [
        Page {
            centred_font: 28.0,
            ..FIRST
        },
        Page {
            centred_font: 28.0,
            shared_font: 30.0,
            ..FIRST
        },
        Page {
            centred_font: 28.0,
            shared_font: 30.0,
            centred: AlignSpec::End,
        },
    ];
    let mut previous = FIRST;
    for step in steps {
        let items = |context: &AppContext| {
            let world = context.world();
            let row = world.parent_id(ids.reference).unwrap();
            world
                .node(row)
                .unwrap()
                .children
                .iter()
                .map(|id| world.layout_box(*id))
                .collect::<Vec<_>>()
        };
        let before = items(&warm);
        let mut queue = MutationQueue::new();
        if step.centred_font != previous.centred_font {
            set_font(&mut queue, ids.centred_label, step.centred_font);
        }
        if step.shared_font != previous.shared_font {
            set_font(&mut queue, ids.shared_label, step.shared_font);
        }
        if step.centred != previous.centred {
            queue.set_style(
                ids.centred,
                styled(flex(|layout| {
                    layout.align_items = step.centred;
                    layout.width = Some(LengthSpec::Px(130.0));
                    layout.height = Some(LengthSpec::Px(100.0));
                })),
            );
        }
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document(), viewport(), &mut shaper);
        assert_on_the_reference(&warm, &ids);
        assert_ne!(items(&warm), before, "{step:?}: the row aligned again");
        assert!(
            !measured(&warm, ids.unrelated),
            "{step:?}: the column beside the row laid out"
        );
        let (mut cold, _) = page(step);
        assert_matches_cold(&mut warm, &mut cold, document());
        previous = step;
    }
}

/// A flex row aligning first baselines: the empty reference box, a block
/// flow 120 tall whose first child takes an auto top margin, and a block
/// flow whose first label clears a float. Returns the reference and each
/// flow's first label.
fn block_flow_page() -> (AppContext, [StableNodeId; 3]) {
    let (mut b, root) = Builder::page(document(), 1, 880.0);
    let row = b.element(
        root,
        flex(|layout| {
            layout.align_items = AlignSpec::Baseline;
            layout.width = Some(LengthSpec::Px(860.0));
        }),
    );
    let reference = b.element(row, sized(10.0, Some(10.0)));
    let auto = b.element(row, sized(120.0, Some(120.0)));
    let pushed = b.element(
        auto,
        LayoutStyle {
            margin_top: Some(LengthSpec::Auto),
            ..LayoutStyle::default()
        },
    );
    let auto_label = b.label(pushed, "Auto");
    let floats = b.element(row, sized(120.0, None));
    b.element(
        floats,
        LayoutStyle {
            float: nana_ui_core::FloatSpec::Left,
            ..sized(40.0, Some(50.0))
        },
    );
    let cleared = b.element(
        floats,
        LayoutStyle {
            clear: nana_ui_core::ClearSpec::Both,
            ..LayoutStyle::default()
        },
    );
    let clear_label = b.label(cleared, "Clear");
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    product_frame(
        &mut context,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
    );
    (context, [reference, auto_label, clear_label])
}

/// A block flow sits by its first line box where placement put it: below
/// the space an auto margin takes, and below a float its block clears.
#[test]
fn issue394_a_block_flow_sits_by_where_placement_put_its_first_line() {
    let (context, [reference, auto_label, clear_label]) = block_flow_page();
    let world = context.world();
    let reference = world.layout_box(reference).unwrap();
    let reference = reference.y + reference.height;
    for label in [auto_label, clear_label] {
        let parent = world.layout_box(world.parent_id(label).unwrap()).unwrap();
        let flow = world.layout_box(world.parent_id(world.parent_id(label).unwrap()).unwrap());
        assert!(
            parent.y > flow.unwrap().y + 40.0,
            "{parent:?} is not pushed down in {flow:?}"
        );
        let baseline = world.layout_result(label).unwrap().first_baseline.unwrap();
        assert!(
            (baseline - reference).abs() < 0.01,
            "{label:?}: {baseline} vs the reference's bottom {reference}"
        );
    }
}

/// What the grid gate's steps change.
#[derive(Clone, Copy, Debug)]
struct GridPage {
    /// The font size of the label in the centred grid's fixed first item.
    centred_font: f32,
    /// The font size of the second row's first label in the baseline grid.
    second_row_font: f32,
}

const GRID_FIRST: GridPage = GridPage {
    centred_font: 16.0,
    second_row_font: 16.0,
};

/// The grid gate's ids.
struct GridIds {
    reference: StableNodeId,
    /// The baseline grid's labels, by row.
    rows: [[StableNodeId; 3]; 2],
    /// Per centred or placed grid, the label it sits by and one it does
    /// not.
    pairs: Vec<(&'static str, StableNodeId, StableNodeId)>,
    centred_label: StableNodeId,
    unrelated: StableNodeId,
}

fn grid(edit: impl FnOnce(&mut LayoutStyle)) -> LayoutStyle {
    let mut layout = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        ..LayoutStyle::default()
    };
    edit(&mut layout);
    layout
}

/// A flex row aligning first baselines: the reference, then
/// - `baseline`: three 60px columns aligning items by baseline, two rows;
/// - `centred`: two fixed items centred in a 100px row;
/// - `placed`: the second item placed in the first column, both centred.
fn grid_page(page: GridPage) -> (AppContext, GridIds) {
    use nana_ui_core::{GridLine, GridPlacement, GridTrack};
    let (mut b, root) = Builder::page(document(), 1, 880.0);
    let row = b.element(
        root,
        flex(|layout| {
            layout.align_items = AlignSpec::Baseline;
            layout.width = Some(LengthSpec::Px(860.0));
        }),
    );
    let reference = b.element(row, sized(10.0, Some(10.0)));
    let font = |size: f32| LayoutStyle {
        font_size: Some(size),
        line_height: Some(LineHeightSpec::Relative(1.2)),
        ..LayoutStyle::default()
    };
    let labelled = |b: &mut Builder, parent, layout, text: &str, size| {
        let holder = b.element(parent, layout);
        let label = b.label(holder, text);
        b.queue.set_style(label, styled(font(size)));
        label
    };
    let columns = |count: usize| Some(vec![GridTrack::Px(60.0); count]);

    let baseline = b.element(
        row,
        grid(|layout| {
            layout.grid_columns = columns(3);
            layout.align_items = AlignSpec::Baseline;
            layout.width = Some(LengthSpec::Px(200.0));
        }),
    );
    let mut rows = [[reference; 3]; 2];
    for (index, (text, size)) in [
        ("A", 12.0),
        ("B", 24.0),
        ("C", 16.0),
        ("D", page.second_row_font),
        ("E", 12.0),
        ("F", 28.0),
    ]
    .into_iter()
    .enumerate()
    {
        rows[index / 3][index % 3] = labelled(&mut b, baseline, LayoutStyle::default(), text, size);
    }

    let mut pairs = Vec::new();
    let centred = b.element(
        row,
        grid(|layout| {
            layout.grid_columns = columns(2);
            layout.grid_rows = Some(vec![GridTrack::Px(100.0)]);
            layout.align_items = AlignSpec::Center;
            layout.width = Some(LengthSpec::Px(130.0));
        }),
    );
    let centred_label = labelled(
        &mut b,
        centred,
        sized(60.0, Some(40.0)),
        "G",
        page.centred_font,
    );
    let other = labelled(&mut b, centred, sized(60.0, Some(60.0)), "H", 24.0);
    pairs.push(("centred", centred_label, other));

    let placed = b.element(
        row,
        grid(|layout| {
            layout.grid_columns = columns(2);
            layout.grid_rows = Some(vec![GridTrack::Px(100.0)]);
            layout.align_items = AlignSpec::Center;
            layout.width = Some(LengthSpec::Px(130.0));
        }),
    );
    let at = |column: i32| LayoutStyle {
        grid_placement: GridPlacement {
            column_start: GridLine::Index(column),
            row_start: GridLine::Index(1),
            ..GridPlacement::default()
        }
        .into(),
        ..sized(60.0, None)
    };
    let second_column = labelled(&mut b, placed, at(2), "I", 16.0);
    let first_column = labelled(&mut b, placed, at(1), "J", 24.0);
    pairs.push(("placed", first_column, second_column));

    let unrelated = b.element(root, sized(400.0, None));
    for index in 0..20 {
        b.label(unrelated, &format!("Unrelated {index}"));
    }
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    product_frame(
        &mut context,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
    );
    (
        context,
        GridIds {
            reference,
            rows,
            pairs,
            centred_label,
            unrelated,
        },
    )
}

/// Each row of the baseline grid shares one baseline, its first the
/// reference's bottom; each other grid's sitting label is on it and its
/// other label is not.
fn assert_grids_on_the_reference(context: &AppContext, ids: &GridIds) {
    let world = context.world();
    let baseline = |id: StableNodeId| {
        world
            .layout_result(id)
            .and_then(|result| result.first_baseline)
            .unwrap_or_else(|| panic!("{id:?} publishes no baseline"))
    };
    let reference = world.layout_box(ids.reference).unwrap();
    let reference = reference.y + reference.height;
    for (index, row) in ids.rows.iter().enumerate() {
        let shared = baseline(row[0]);
        for label in row {
            assert!(
                (baseline(*label) - shared).abs() < 0.01,
                "row {index}: {} vs {shared}",
                baseline(*label)
            );
        }
        assert_eq!(
            (shared - reference).abs() < 0.01,
            index == 0,
            "row {index} at {shared}, the reference's bottom at {reference}"
        );
    }
    for &(name, sitting, other) in &ids.pairs {
        assert!(
            (baseline(sitting) - reference).abs() < 0.01,
            "{name}: {} vs the reference's bottom {reference}",
            baseline(sitting)
        );
        assert!(
            (baseline(other) - reference).abs() > 1.0,
            "{name}: the other item is on the baseline too"
        );
    }
}

/// Grid items aligned by baseline share one per start row, and a grid on a
/// baseline-aligned row sits by its first row's shared baseline or by its
/// first item in grid order (#394). A bigger font in a centred fixed item
/// aligns the outer row again; one in the second row moves that row's
/// items and not the first. Each step matches a cold layout and lays out
/// nothing beside the row.
#[test]
fn issue394_grid_items_share_a_baseline_per_row_and_a_grid_sits_by_its_first() {
    let mut shaper = bundled_face_shaper();
    let (mut warm, ids) = grid_page(GRID_FIRST);
    assert_grids_on_the_reference(&warm, &ids);
    let (mut cold, _) = grid_page(GRID_FIRST);
    assert_matches_cold(&mut warm, &mut cold, document());
    let first_row =
        |context: &AppContext| ids.rows[0].map(|label| context.world().layout_box(label));
    let mut previous = GRID_FIRST;
    for step in [
        GridPage {
            centred_font: 28.0,
            ..GRID_FIRST
        },
        GridPage {
            centred_font: 28.0,
            second_row_font: 32.0,
        },
    ] {
        let mut queue = MutationQueue::new();
        let moved = if step.centred_font != previous.centred_font {
            set_font(&mut queue, ids.centred_label, step.centred_font);
            ids.pairs[0].1
        } else {
            set_font(&mut queue, ids.rows[1][0], step.second_row_font);
            ids.rows[1][1]
        };
        let before = (warm.world().layout_box(moved), first_row(&warm));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document(), viewport(), &mut shaper);
        assert_grids_on_the_reference(&warm, &ids);
        let after = (warm.world().layout_box(moved), first_row(&warm));
        assert_ne!(after.0, before.0, "{step:?}: nothing aligned again");
        if step.second_row_font != previous.second_row_font {
            assert_eq!(after.1, before.1, "{step:?}: the first row moved");
        }
        assert!(
            !measured(&warm, ids.unrelated),
            "{step:?}: the column beside the row laid out"
        );
        let (mut cold, _) = grid_page(step);
        assert_matches_cold(&mut warm, &mut cold, document());
        previous = step;
    }
}
