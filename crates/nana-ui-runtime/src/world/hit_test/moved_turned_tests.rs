//! Boxes moved under ancestors that rotate, skew, flip or scale: each such
//! ancestor hands its children its own layout coordinates
//! (`HitEntry::local_children`), so a box moved under it is an upright move
//! there and only the moved boxes' entries are built again -- and so is the
//! ancestor's own, when the move resizes it and its transform origin moves,
//! or its transform changes. Every frame answers the pointer as a twin page
//! whose index is built from scratch does, and the cold check
//! (`check_moved_hit_entries`) runs on every patch.

use nana_ui_core::{
    FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PaintTransform, PositionSpec,
};

use super::super::reflow_oracle::{
    Builder, accessible_frame, bundled_face_shaper, fixed, node, styled,
};
use crate::{
    AppContext, DocumentId, LayoutViewport, MutationQueue, NodeKind, ScrollOffset, StableNodeId,
};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

struct Page {
    context: AppContext,
    /// The turned ancestors, the outermost first, with their transforms.
    wrappers: Vec<(StableNodeId, PaintTransform)>,
    /// Whether they open a 3D context.
    preserve_3d: bool,
    /// The scroller the wrappers stand in, if any.
    outer: Option<StableNodeId>,
    /// Rows straight in the innermost wrapper: growing the first moves the
    /// others, the scroller after them, and resizes every wrapper.
    plain: Vec<StableNodeId>,
    scroller: StableNodeId,
    /// Rows in the scroller.
    rows: Vec<StableNodeId>,
    /// Every node, for the messages.
    entries: usize,
}

/// A rotation, a skew, a flip or a scale with a move, as a CSS `matrix`.
fn turn(kind: &str, angle: f32) -> PaintTransform {
    let (sin, cos) = angle.sin_cos();
    match kind {
        "rotated" => PaintTransform {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 2.0,
            f: 1.0,
        },
        "skewed" => PaintTransform {
            c: angle.tan(),
            e: 1.5,
            ..Default::default()
        },
        "flipped" => PaintTransform {
            a: -1.0,
            f: 3.25,
            ..Default::default()
        },
        "scaled" => PaintTransform {
            a: 1.5 + angle,
            d: 1.25,
            e: 3.3,
            f: 7.1,
            ..Default::default()
        },
        other => panic!("no turn {other}"),
    }
}

fn wrapper_style(depth: usize, transform: PaintTransform, preserve_3d: bool) -> LayoutStyle {
    let mut layout = LayoutStyle {
        padding_left: Some(LengthSpec::Px(1.3)),
        padding_top: Some(LengthSpec::Px(2.1)),
        width: Some(LengthSpec::Px(340.0 - 10.0 * depth as f32)),
        direction: Some(FlexDirection::Column),
        flex_shrink: Some(0.0),
        position: PositionSpec::Relative,
        ..LayoutStyle::default()
    };
    layout.transforms.transform = Some(transform);
    layout.transforms.preserve_3d = preserve_3d;
    layout
}

fn row_style(height: f32) -> LayoutStyle {
    LayoutStyle {
        flex_shrink: Some(0.0),
        position: PositionSpec::Relative,
        ..fixed(300.0, height)
    }
}

/// A row holding a padded cell holding a leaf.
fn row(b: &mut Builder, parent: StableNodeId) -> (StableNodeId, StableNodeId) {
    let row = b.element(parent, row_style(20.0));
    let cell = b.element(
        row,
        LayoutStyle {
            padding_left: Some(LengthSpec::Px(1.3)),
            padding_top: Some(LengthSpec::Px(2.5)),
            ..fixed(100.0, 14.0)
        },
    );
    b.element(cell, fixed(50.0, 5.0));
    (row, cell)
}

/// What a [`page`] is made of: the turned wrappers, the outermost first;
/// whether they stand in a scroller of their own (too short for them, so it
/// scrolls); the turn of the scroller they hold, if it turns too; and
/// whether they open a 3D context.
struct Shape {
    transforms: Vec<PaintTransform>,
    outer: bool,
    scroller_turn: Option<PaintTransform>,
    /// The wrappers open a 3D context (`preserve-3d`): what they hand down
    /// is then composed as before, and moves under them patch whole.
    preserve_3d: bool,
}

/// Nested wrappers with the shape's transforms, sized by what they hold;
/// the innermost holds six plain rows and a 300 x 120 scroller of twelve
/// rows and one more that is not positioned. The second plain row's cell
/// holds a sticky (turned), a fixed and an absolute box, each with a plain
/// box inside; the last scrolled row's cell holds an absolute box measured from
/// the innermost wrapper, which escapes the scroller's clip (#201).
fn page(shape: &Shape) -> Page {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let mut parent = root;
    let outer = shape.outer.then(|| {
        parent = b.element(
            root,
            LayoutStyle {
                overflow_x: OverflowSpec::Auto,
                overflow_y: OverflowSpec::Auto,
                direction: Some(FlexDirection::Column),
                flex_shrink: Some(0.0),
                ..fixed(390.0, 200.0)
            },
        );
        parent
    });
    if shape.outer {
        // Beside the wrappers, straight in the page's scroller: a turned
        // sticky box with a box inside. A scroll of the page builds its
        // branch again whole, under the shift the index holds the
        // scroller's children at.
        let (_, cell) = row(&mut b, parent);
        let mut sticky_style = LayoutStyle {
            position: PositionSpec::Sticky,
            offset_top: Some(LengthSpec::Px(1.0)),
            ..fixed(30.0, 9.0)
        };
        sticky_style.transforms.transform = Some(turn("rotated", -0.35));
        let sticky = b.element(cell, sticky_style);
        b.element(sticky, fixed(14.0, 5.0));
    }
    let mut wrappers = Vec::new();
    for (depth, &transform) in shape.transforms.iter().enumerate() {
        parent = b.element(parent, wrapper_style(depth, transform, shape.preserve_3d));
        wrappers.push((parent, transform));
    }
    let mut plain = Vec::new();
    for index in 0..6 {
        let (row, cell) = row(&mut b, parent);
        if index == 1 {
            for position in [
                PositionSpec::Sticky,
                PositionSpec::Fixed,
                PositionSpec::Absolute,
            ] {
                // The sticky box turns too: a scroll of its scroller builds
                // its branch again, under the shift the index holds above.
                let mut held_style = LayoutStyle {
                    position,
                    offset_top: Some(LengthSpec::Px(1.0)),
                    offset_left: Some(LengthSpec::Px(7.0)),
                    ..fixed(12.0, 6.0)
                };
                if position == PositionSpec::Sticky {
                    held_style.transforms.transform = Some(turn("rotated", 0.25));
                }
                let held = b.element(cell, held_style);
                b.element(held, fixed(4.0, 3.0));
            }
        }
        plain.push(row);
    }
    let mut scroller_style = LayoutStyle {
        overflow_x: OverflowSpec::Auto,
        overflow_y: OverflowSpec::Auto,
        direction: Some(FlexDirection::Column),
        flex_shrink: Some(0.0),
        ..fixed(300.0, 120.0)
    };
    if shape.scroller_turn.is_some() {
        scroller_style.transforms.transform = shape.scroller_turn;
    }
    let scroller = b.element(parent, scroller_style);
    let list = b.element(
        scroller,
        LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..LayoutStyle::default()
        },
    );
    let rows = (0..12).map(|_| row(&mut b, list).0).collect();
    let unpositioned = b.element(
        list,
        LayoutStyle {
            flex_shrink: Some(0.0),
            ..fixed(300.0, 20.0)
        },
    );
    let escaping = b.element(
        unpositioned,
        LayoutStyle {
            position: PositionSpec::Absolute,
            offset_top: Some(LengthSpec::Px(3.3)),
            offset_left: Some(LengthSpec::Px(40.7)),
            ..fixed(30.0, 8.0)
        },
    );
    b.element(escaping, fixed(9.0, 4.0));
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let mut page = Page {
        context,
        wrappers,
        preserve_3d: shape.preserve_3d,
        outer,
        plain,
        scroller,
        rows,
        entries: 0,
    };
    frame(&mut page, false);
    page.context.rebuild_hit_test(document());
    page.entries = page.context.world().len();
    page
}

/// A product frame, then the hit index brought up to date as
/// `RuntimeDocument` does it: a pure scroll shifts the entries, anything
/// else patches them -- or, `cold`, builds the whole index again.
fn frame(page: &mut Page, cold: bool) {
    let work = accessible_frame(
        &mut page.context,
        document(),
        LayoutViewport::new(400.0, 600.0),
        &mut bundled_face_shaper(),
        false,
    );
    let updates = page.context.take_scroll_hit_updates();
    let mut dirty = work.input_hit_test.clone();
    dirty.sort_unstable();
    dirty.dedup();
    if cold {
        page.context.rebuild_hit_test(document());
    } else if !dirty.is_empty() {
        if page.context.hit_test_work_is_scroll_only(&dirty, &updates) {
            for (scroller, delta) in updates {
                page.context
                    .update_hit_test_scroll(document(), scroller, delta);
            }
        } else {
            page.context.rebuild_hit_test_for(document(), &dirty);
        }
    }
}

/// One point of [`grid`]: the topmost hit, every candidate under it, where
/// the point lands in each candidate's layout coordinates, and where that
/// layout point is mapped back to on the window.
type Point = (
    Option<StableNodeId>,
    Vec<StableNodeId>,
    Vec<Option<[f32; 4]>>,
);

/// What the pointer finds on a grid over the page. The points stay off the
/// boxes' edges, which a scroll shift and a fresh build may place an ulp
/// apart.
fn grid(page: &Page) -> Vec<Point> {
    let world = page.context.world();
    let mut hits = Vec::new();
    for row in 0..60 {
        let y = row as f32 * 9.7 + 0.71;
        for column in 0..36 {
            let x = column as f32 * 11.3 + 0.43;
            let candidates = world.hit_test_candidates(document(), x, y);
            let positions = candidates
                .iter()
                .map(|&id| {
                    let (lx, ly) = world.pointer_layout_position(id, x, y)?;
                    let (wx, wy) = world.layout_pointer_position(id, lx, ly)?;
                    Some([lx, ly, wx, wy])
                })
                .collect();
            hits.push((world.hit_test(document(), x, y), candidates, positions));
        }
    }
    hits
}

/// The two pages find the same boxes at every point of [`grid`], and put
/// the point in the same place in each: to rounding, which is all a scroll
/// shift and a fresh build may differ by there.
#[track_caller]
fn same(retained: &Page, cold: &Page, at: impl std::fmt::Display) {
    agrees_with_the_layout(retained, &at);
    let near = |a: f32, b: f32| (a - b).abs() <= 1e-3;
    let (ours, theirs) = (grid(retained), grid(cold));
    let differs = |(a, b): (&Point, &Point)| {
        a.0 != b.0
            || a.1 != b.1
            || a.2.iter().zip(&b.2).any(|pair| match pair {
                (Some(a), Some(b)) => (0..4).any(|i| !near(a[i], b[i])),
                (a, b) => a.is_some() != b.is_some(),
            })
    };
    if let Some(point) = (0..ours.len()).find(|&point| differs((&ours[point], &theirs[point]))) {
        panic!(
            "{at}: point {point} (row {}, column {}): {:?} != {:?}",
            point / 36,
            point % 36,
            ours[point],
            theirs[point]
        );
    }
}

/// The index answers as the walk that reads no retained transform or
/// bounds does: the presentation walk, which composes every box's
/// transform from the layout as it goes (and only takes the index's
/// structure and clips), finds the same candidates in the same order at
/// every point of [`grid`]; and the point lands in each candidate's layout
/// coordinates where the layout's own transform chain puts it
/// (`layout_projection_transform`), and back. A mistake both pages would
/// make alike shows here.
#[track_caller]
fn agrees_with_the_layout(page: &Page, at: &impl std::fmt::Display) {
    let world = page.context.world();
    let index = &world.hit_index.hit_test_index[&document()];
    let near = |a: f32, b: f32| (a - b).abs() <= 2e-3 * a.abs().max(b.abs()).max(1.0);
    for row in 0..60 {
        let y = row as f32 * 9.7 + 0.71;
        for column in 0..36 {
            let x = column as f32 * 11.3 + 0.43;
            let mut walked = Vec::new();
            index.visit_roots(x, y, &mut |id| {
                walked.push(id);
                false
            });
            let mut presented = Vec::new();
            world.visit_presentation_roots(index, x, y, &mut |id| {
                presented.push(id);
                false
            });
            // The two compose the same transforms in another order, so a
            // point within rounding of a box's edge may fall on either side
            // of it in the two; such boxes are left out of the comparison
            // (the twin pages above agree on them exactly).
            let on_edge = |id: &StableNodeId| {
                let (transform, persp) = world.layout_projection_transform(*id).unwrap();
                let (px, py) = super::transformed_point(transform, persp, x, y).unwrap();
                let b = world.layout_box(*id).unwrap();
                let edge =
                    |v: f32, from: f32, to: f32| (v - from).abs() <= 1e-3 || (v - to).abs() <= 1e-3;
                edge(px, b.x, b.x + b.width) || edge(py, b.y, b.y + b.height)
            };
            let firm = |ids: &[StableNodeId]| {
                ids.iter()
                    .copied()
                    .filter(|id| !on_edge(id))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                firm(&walked),
                firm(&presented),
                "{at}: candidates at ({x}, {y})"
            );
            for id in walked {
                let (transform, persp) = world.layout_projection_transform(id).unwrap();
                let expected = super::transformed_point(transform, persp, x, y).unwrap();
                let found = world.pointer_layout_position(id, x, y).unwrap();
                assert!(
                    near(found.0, expected.0) && near(found.1, expected.1),
                    "{at}: ({x}, {y}) in {id:?}: {found:?} != {expected:?}"
                );
                let back = world.layout_pointer_position(id, found.0, found.1).unwrap();
                assert!(
                    near(back.0, x) && near(back.1, y),
                    "{at}: ({x}, {y}) in {id:?} and back: {back:?}"
                );
            }
        }
    }
}

fn commit(page: &mut Page, edit: impl FnOnce(&mut MutationQueue, &Page)) {
    let mut queue = MutationQueue::new();
    edit(&mut queue, page);
    page.context.commit_mutations(queue).unwrap();
}

fn grow(queue: &mut MutationQueue, row: StableNodeId, height: f32) {
    queue.set_style(row, styled(row_style(height)));
}

fn scroll(queue: &mut MutationQueue, scroller: StableNodeId, y: f32) {
    queue.set_scroll_offset(scroller, ScrollOffset { x: 0.0, y });
}

/// The outermost wrapper with `transform` in place of its own.
fn set_turn(queue: &mut MutationQueue, page: &Page, transform: PaintTransform) {
    queue.set_style(
        page.wrappers[0].0,
        styled(wrapper_style(0, transform, page.preserve_3d)),
    );
}

/// A box appended to the outermost wrapper: a change of what it holds,
/// which patches its branch whole.
fn append(queue: &mut MutationQueue, page: &Page, id: StableNodeId) {
    queue.create(id, document(), NodeKind::Element { tag: "div".into() });
    queue.insert(page.wrappers[0].0, id, None);
    queue.set_style(id, styled(fixed(40.0, 6.3)));
}

/// Every turn of the outermost wrapper by `angle` more.
fn turn_more(queue: &mut MutationQueue, page: &Page, angle: f32) {
    let (wrapper, transform) = page.wrappers[0];
    let (sin, cos) = angle.sin_cos();
    // The 2x2 part times a rotation by `angle`, the move kept.
    let turned = PaintTransform {
        a: transform.a * cos + transform.c * sin,
        b: transform.b * cos + transform.d * sin,
        c: transform.c * cos - transform.a * sin,
        d: transform.d * cos - transform.b * sin,
        ..transform
    };
    queue.set_style(wrapper, styled(wrapper_style(0, turned, page.preserve_3d)));
}

/// Under each kind of turned ancestor, under two rotations, under a turn
/// in a scrolled page and over a turned scroller, the first plain row and
/// the first scrolled row grow by an uneven step again and again, with
/// scrolls, a scroll in the same frame as a move, and a change of the
/// outermost turn in between: only the moved rows' entries and the boxes
/// the move resizes are built again, never what the rows hold, and the
/// pointer finds what it finds on a twin page built from scratch.
#[test]
fn boxes_moved_under_a_turned_ancestor_build_only_their_own_entries() {
    let shape = |transforms: Vec<PaintTransform>| Shape {
        transforms,
        outer: false,
        scroller_turn: None,
        preserve_3d: false,
    };
    let cases = [
        ("rotated", shape(vec![turn("rotated", 0.3)])),
        ("skewed", shape(vec![turn("skewed", 0.4)])),
        ("flipped", shape(vec![turn("flipped", 0.0)])),
        ("scaled", shape(vec![turn("scaled", 0.0)])),
        (
            "rotated twice",
            shape(vec![turn("rotated", 0.3), turn("rotated", -0.7)]),
        ),
        (
            "rotated in a scale",
            shape(vec![turn("scaled", 0.0), turn("rotated", 0.3)]),
        ),
        (
            "rotated in a scrolled page",
            Shape {
                outer: true,
                ..shape(vec![turn("rotated", 0.3)])
            },
        ),
        (
            "a turned scroller",
            Shape {
                scroller_turn: Some(turn("rotated", -0.2)),
                outer: true,
                ..shape(vec![turn("skewed", 0.4)])
            },
        ),
        (
            "rotated in a 3D context",
            Shape {
                preserve_3d: true,
                ..shape(vec![turn("rotated", 0.3)])
            },
        ),
    ];
    for (name, shape) in cases {
        let mut retained = page(&shape);
        let mut cold = page(&shape);
        same(&retained, &cold, format!("{name}: built"));
        let mut plain_height = 20.0;
        let mut row_height = 20.0;
        let mut list_scroll = 30.37;
        let mut page_scroll = 7.31;
        for page in [&mut retained, &mut cold] {
            commit(page, |queue, page| {
                scroll(queue, page.scroller, list_scroll);
                if let Some(outer) = page.outer {
                    scroll(queue, outer, page_scroll);
                }
            });
        }
        // A pure scroll shifts entries; only the sticky boxes (and what
        // they hold) are placed again, when the page scroller is theirs.
        frame(&mut retained, false);
        frame(&mut cold, true);
        same(&retained, &cold, format!("{name}: scrolled"));
        // Rows and the boxes a move resizes or moves without anything they
        // hold: the wrappers, the scroller and its list.
        // The pinned (sticky, escaped) and the rows they sit in aside.
        for round in 0..3 {
            for step in 0..4 {
                let at = format!("{name}, round {round}, step {step}");
                plain_height += 1.37;
                for page in [&mut retained, &mut cold] {
                    commit(page, |queue, page| grow(queue, page.plain[0], plain_height));
                }
                frame(&mut retained, false);
                frame(&mut cold, true);
                same(&retained, &cold, format!("{at}: plain rows moved"));
                row_height += 1.37;
                for page in [&mut retained, &mut cold] {
                    commit(page, |queue, page| grow(queue, page.rows[0], row_height));
                }
                frame(&mut retained, false);
                frame(&mut cold, true);
                same(&retained, &cold, format!("{at}: scrolled rows moved"));
            }
            // A scroll on its own, then one with a move in the same frame.
            list_scroll += 11.13;
            page_scroll += 3.17;
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| {
                    scroll(queue, page.scroller, list_scroll);
                    if let Some(outer) = page.outer {
                        scroll(queue, outer, page_scroll);
                    }
                });
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(&retained, &cold, format!("{name}, round {round}: scrolled"));
            list_scroll -= 5.29;
            page_scroll -= 1.93;
            row_height += 1.37;
            plain_height += 1.37;
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| {
                    scroll(queue, page.scroller, list_scroll);
                    if let Some(outer) = page.outer {
                        scroll(queue, outer, page_scroll);
                    }
                    grow(queue, page.rows[0], row_height);
                    grow(queue, page.plain[0], plain_height);
                });
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(
                &retained,
                &cold,
                format!("{name}, round {round}: scrolled and moved"),
            );
            // The outermost turn changes, on its own and with a move.
            let angle = 0.11 * (round + 1) as f32;
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| turn_more(queue, page, angle));
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(&retained, &cold, format!("{name}, round {round}: turned"));
            plain_height += 1.37;
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| {
                    turn_more(queue, page, -angle);
                    grow(queue, page.plain[0], plain_height);
                });
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(
                &retained,
                &cold,
                format!("{name}, round {round}: turned back and moved"),
            );
            // What the outermost wrapper holds changes: its branch is built
            // again whole, under the page's scroll when there is one --
            // a shift the index holds, after a scroll of its own.
            page_scroll += 2.03;
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| {
                    if let Some(outer) = page.outer {
                        scroll(queue, outer, page_scroll);
                    }
                });
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(
                &retained,
                &cold,
                format!("{name}, round {round}: page scrolled"),
            );
            // The outermost wrapper's stacking changes and nothing else: its
            // branch alone is built again whole, under that shift.
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| {
                    let (wrapper, transform) = page.wrappers[0];
                    let style = LayoutStyle {
                        z_index: Some(1 + round),
                        ..wrapper_style(0, transform, page.preserve_3d)
                    };
                    queue.set_style(wrapper, styled(style));
                });
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(&retained, &cold, format!("{name}, round {round}: raised"));
            let added = node(1000 + round as u64);
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| append(queue, page, added));
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(&retained, &cold, format!("{name}, round {round}: appended"));
        }
        // The outermost wrapper stops turning (it only moves) and turns
        // again: what it hands its children changes kind both times.
        let upright = PaintTransform {
            e: 2.0,
            f: 1.0,
            ..Default::default()
        };
        // Its stacking back first, on its own.
        for page in [&mut retained, &mut cold] {
            commit(page, |queue, page| {
                set_turn(queue, page, page.wrappers[0].1)
            });
        }
        frame(&mut retained, false);
        frame(&mut cold, true);
        same(&retained, &cold, format!("{name}: lowered"));
        for transform in [upright, retained.wrappers[0].1] {
            plain_height += 1.37;
            for page in [&mut retained, &mut cold] {
                commit(page, |queue, page| {
                    set_turn(queue, page, transform);
                    grow(queue, page.plain[0], plain_height);
                });
            }
            frame(&mut retained, false);
            frame(&mut cold, true);
            same(&retained, &cold, format!("{name}: turned to {transform:?}"));
        }
    }
}

/// A `transform_3d` that turns only in the page's plane (`rotateZ`) has an
/// affine homography: with no 3D context above or on it, its hit entry hands
/// its children its own layout coordinates as a 2D turn does
/// (`NodeSpace::hit_frames_itself`), and what it holds is built in those.
/// It is still not built again in place itself (`NodeSpace::depth`), so a
/// row that resizes it patches its branch, as it did. Accessibility takes
/// the same box as 3D and pins it; the two are kept apart on purpose.
#[test]
fn boxes_moved_under_a_planar_3d_turn_answer_as_a_fresh_index() {
    let shape = Shape {
        transforms: vec![turn("rotated", 0.3)],
        outer: false,
        scroller_turn: None,
        preserve_3d: false,
    };
    let planar = |angle: f32| {
        let mut layout = wrapper_style(0, turn("rotated", 0.0), false);
        layout.transforms.transform = None;
        layout.transforms.transform_3d = Some(nana_ui_core::PaintMat4::rotate_z(angle));
        styled(layout)
    };
    let mut retained = page(&shape);
    let mut cold = page(&shape);
    for page in [&mut retained, &mut cold] {
        commit(page, |queue, page| {
            queue.set_style(page.wrappers[0].0, planar(0.3))
        });
    }
    frame(&mut retained, false);
    frame(&mut cold, true);
    same(&retained, &cold, "turned in 3D");
    let index = &retained.context.world().hit_index.hit_test_index[&document()];
    assert!(
        index.entries[&retained.wrappers[0].0].entry.local_children,
        "a planar 3D turn hands its children its own coordinates"
    );
    let mut height = 20.0;
    for step in 0..6 {
        height += 1.37;
        for page in [&mut retained, &mut cold] {
            commit(page, |queue, page| grow(queue, page.plain[0], height));
        }
        frame(&mut retained, false);
        frame(&mut cold, true);
        same(&retained, &cold, format!("moved {step}"));
    }
    // Turned on its plane by another angle.
    for page in [&mut retained, &mut cold] {
        commit(page, |queue, page| {
            queue.set_style(page.wrappers[0].0, planar(-0.45))
        });
    }
    frame(&mut retained, false);
    frame(&mut cold, true);
    same(&retained, &cold, "turned again");
}
