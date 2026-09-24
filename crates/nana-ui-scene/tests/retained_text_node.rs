//! Issue #95: retained text nodes, the tiered text dirty graph and the
//! `nana-text` layout they retain, driven through the product frame loop.
//!
//! Every test runs `RuntimeDocument::flush`, so style resolution, the
//! scheduled and layout-scoped text passes, layout writeback and extraction
//! all take part. The engine holds only the bundled UI face.

use std::sync::{Arc, Mutex};

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine, TextWorkCounters};
use nana_ui_core::{
    FlexDirection, FontFeatureSetting, LayoutStyle, LengthSpec, PaintTransform, SemanticColorRole,
};
use nana_ui_runtime::{
    AnimatableProperty, AnimationId, AnimationSpec, Button, DocumentId, Easing, EmptyState,
    LayoutViewport, MeasureTextShaper, MotionTo, MotionValue, MutationQueue, NanaTextEngineShaper,
    NodeKind, NodeStyle, StableNodeId, TextContent, TextInput, TextShaper,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};
use std::time::Duration;

const DOCUMENT: u64 = 1;
const ROOT: u64 = 1;
const COLUMN: u64 = 2;

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

fn row_id(row: usize) -> StableNodeId {
    id(3 + row as u64 * 2)
}

fn label_id(row: usize) -> StableNodeId {
    id(4 + row as u64 * 2)
}

fn engine() -> SharedTextEngine {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    Arc::new(Mutex::new(NativeTextEngine::new(fonts)))
}

fn row_layout(height: f32) -> Arc<LayoutStyle> {
    Arc::new(LayoutStyle {
        width: Some(LengthSpec::Px(300.0)),
        height: Some(LengthSpec::Px(height)),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    })
}

/// Root → column → `rows` × (row → label). The #33 fixture.
fn document(labels: impl IntoIterator<Item = String>) -> (RuntimeDocument, usize) {
    let document = DocumentId::new(DOCUMENT).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    queue.create(id(ROOT), document, NodeKind::Document);
    queue.create(
        id(COLUMN),
        document,
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(id(ROOT), id(COLUMN), None);
    queue.set_style(
        id(COLUMN),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Fill),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    let mut rows = 0;
    for (row, label) in labels.into_iter().enumerate() {
        queue.create(
            row_id(row),
            document,
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(label_id(row), document, NodeKind::Text);
        queue.insert(id(COLUMN), row_id(row), None);
        queue.insert(row_id(row), label_id(row), None);
        queue.set_text(
            label_id(row),
            TextContent {
                value: label.into(),
            },
        );
        queue.set_style(
            row_id(row),
            NodeStyle {
                layout: row_layout(20.0),
                ..NodeStyle::default()
            },
        );
        queue.set_style(label_id(row), NodeStyle::default());
        rows += 1;
    }
    runtime.context_mut().commit_mutations(queue).unwrap();
    (runtime, rows)
}

fn numbered(rows: usize) -> impl Iterator<Item = String> {
    (0..rows).map(|row| format!("row {row}"))
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(300.0, 800.0)
}

fn settle(runtime: &mut RuntimeDocument, shaper: &mut impl TextShaper) {
    runtime.flush(viewport(), shaper).unwrap();
    for _ in 0..4 {
        runtime.flush(viewport(), shaper).unwrap();
    }
}

fn commit(runtime: &mut RuntimeDocument, build: impl FnOnce(&mut MutationQueue)) {
    let mut queue = MutationQueue::new();
    build(&mut queue);
    runtime.context_mut().commit_mutations(queue).unwrap();
}

fn text_work(runtime: &RuntimeDocument) -> TextWorkCounters {
    runtime.context().world().last_text_work_counters()
}

/// No shaping, no layout, and nothing read that scales with the text.
fn assert_no_text_work(work: TextWorkCounters, context: &str) {
    assert_eq!(work.text_nodes_shaped, 0, "{context}: shaped");
    assert_eq!(work.layouts_created, 0, "{context}: layouts created");
    assert_eq!(work.shape_cache_lookups, 0, "{context}: shape lookups");
    assert_eq!(work.layout_cache_lookups, 0, "{context}: layout lookups");
    assert_eq!(work.text_source_clones, 0, "{context}: source clones");
    assert_eq!(work.text_bytes_hashed, 0, "{context}: bytes hashed");
}

/// No text work in the frame just flushed. The counters keep the last frame
/// that ran a text pass, so a frame that ran none still shows `before`; any
/// other value is this frame's work and must be empty.
fn assert_no_new_text_work(before: TextWorkCounters, runtime: &RuntimeDocument, context: &str) {
    let now = text_work(runtime);
    if now != before {
        assert_no_text_work(now, context);
    }
}

fn label_style(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

#[test]
fn every_label_retains_the_layout_its_metrics_were_read_from_and_the_scene_carries_it() {
    let (mut runtime, rows) = document(numbered(8));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);

    let world = runtime.context().world();
    assert_eq!(world.retained_text_layouts(), rows);
    for row in 0..rows {
        let label = label_id(row);
        let (handle, layout) = world.text_layout(label).expect("label retains a layout");
        let extracted = runtime
            .scene()
            .primitives()
            .find_map(|primitive| match &primitive.kind {
                ScenePrimitiveKind::Text { layout, .. } if primitive.node == label => {
                    layout.clone()
                }
                _ => None,
            })
            .expect("the scene text primitive carries the retained layout");
        assert_eq!(extracted.id, handle);
        assert!(Arc::ptr_eq(&extracted.layout, layout));
        let metrics = world.text_metrics(label).unwrap();
        let widest = layout
            .lines
            .iter()
            .map(|line| line.metrics.width_px)
            .fold(0.0, f32::max);
        assert_eq!(metrics.width, widest, "one measurement authority");
    }
}

#[test]
fn a_frame_that_moves_every_label_without_resizing_one_does_zero_text_work() {
    let (mut runtime, rows) = document(numbered(64));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let handles: Vec<_> = (0..rows)
        .map(|row| {
            runtime
                .context()
                .world()
                .text_layout(label_id(row))
                .unwrap()
                .0
        })
        .collect();

    // Column padding moves every row, and so every label, without changing a
    // single label's size: every label is a layout candidate, none has work.
    for frame in 0..3 {
        commit(&mut runtime, |queue| {
            queue.set_style(
                id(COLUMN),
                label_style(LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Fill),
                    direction: Some(FlexDirection::Column),
                    padding_top: Some(LengthSpec::Px(4.0 + frame as f32)),
                    ..LayoutStyle::default()
                }),
            )
        });
        let update = runtime.flush(viewport(), &mut shaper).unwrap();
        assert!(!update.is_idle());
        let work = text_work(&runtime);
        assert_no_text_work(work, "moved frame");
        assert_eq!(
            work.text_nodes_considered, rows,
            "every label was a candidate"
        );
        assert_eq!(work.text_nodes_revision_skipped, rows);
    }
    for (row, handle) in handles.iter().enumerate() {
        assert_eq!(
            runtime
                .context()
                .world()
                .text_layout(label_id(row))
                .unwrap()
                .0,
            *handle,
            "a frame without text work keeps every retained layout handle"
        );
    }
}

#[test]
fn colour_opacity_and_transform_changes_never_reach_shaping_or_layout() {
    let (mut runtime, rows) = document(numbered(16));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    let before: Vec<_> = (0..rows)
        .map(|row| {
            (
                world.text_revisions(label_id(row)).unwrap(),
                world.text_layout(label_id(row)).unwrap().0,
            )
        })
        .collect();

    type Edit = (&'static str, fn() -> NodeStyle);
    let edits: [Edit; 4] = [
        ("colour", || {
            label_style(LayoutStyle {
                color: Some([0.9, 0.1, 0.1, 1.0]),
                ..LayoutStyle::default()
            })
        }),
        ("semantic foreground", || NodeStyle {
            foreground: Some(SemanticColorRole::Accent),
            ..NodeStyle::default()
        }),
        ("opacity", || {
            label_style(LayoutStyle {
                opacity: Some(0.5),
                ..LayoutStyle::default()
            })
        }),
        ("transform", || {
            label_style(LayoutStyle {
                transform: Some(PaintTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 12.0,
                    f: 4.0,
                }),
                ..LayoutStyle::default()
            })
        }),
    ];
    for (name, style) in edits {
        let before_work = text_work(&runtime);
        commit(&mut runtime, |queue| {
            for row in 0..rows {
                queue.set_style(label_id(row), style());
            }
        });
        let update = runtime.flush(viewport(), &mut shaper).unwrap();
        assert!(!update.is_idle(), "{name}: the edit is a frame");
        assert_no_new_text_work(before_work, &runtime, name);
        let world = runtime.context().world();
        for (row, (revisions, handle)) in before.iter().enumerate() {
            let now = world.text_revisions(label_id(row)).unwrap();
            assert_eq!(now.content, revisions.content, "{name}");
            assert_eq!(now.shape, revisions.shape, "{name}");
            assert_eq!(now.constraint, revisions.constraint, "{name}");
            assert_eq!(
                world.text_layout(label_id(row)).unwrap().0,
                *handle,
                "{name}"
            );
        }
    }
}

#[test]
fn a_width_change_relayouts_from_the_runs_it_already_shaped() {
    let (mut runtime, _) = document(std::iter::empty());
    let paragraph = id(900);
    let wrapping = |width: f32| {
        label_style(LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            ..LayoutStyle::default()
        })
    };
    commit(&mut runtime, |queue| {
        queue.create(
            paragraph,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Text,
        );
        queue.insert(id(COLUMN), paragraph, None);
        queue.set_text(
            paragraph,
            TextContent {
                value: "a paragraph long enough to wrap onto several lines at a narrow width"
                    .into(),
            },
        );
        queue.set_style(paragraph, wrapping(280.0));
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let wide = Arc::clone(runtime.context().world().text_layout(paragraph).unwrap().1);

    commit(&mut runtime, |queue| {
        queue.set_style(paragraph, wrapping(90.0))
    });
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = text_work(&runtime);
    assert_eq!(
        work.text_nodes_shaped, 0,
        "the text did not change: no shaping"
    );
    assert!(work.layouts_created >= 1);
    assert_eq!(
        work.constraint_only_relayouts, work.layouts_created,
        "every new layout reused shaped runs"
    );
    let world = runtime.context().world();
    let narrow = world.text_layout(paragraph).unwrap().1;
    assert!(narrow.lines.len() > wide.lines.len());
    assert!(world.text_revisions(paragraph).unwrap().constraint > 0);
}

/// A shrink-to-fit tag around a single ellipsizing label, in a header row
/// `header` wide that lets it shrink: the title-tag shape a card header uses.
fn shrink_row_around_an_ellipsizing_label(header: f32, label: &str) -> RuntimeDocument {
    let (mut runtime, _) = document([label.to_string()]);
    commit(&mut runtime, |queue| {
        queue.set_style(
            id(COLUMN),
            label_style(LayoutStyle {
                width: Some(LengthSpec::Px(header)),
                height: Some(LengthSpec::Px(20.0)),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            }),
        );
        queue.set_style(
            row_id(0),
            label_style(LayoutStyle {
                width: Some(LengthSpec::Shrink),
                height: Some(LengthSpec::Px(20.0)),
                direction: Some(FlexDirection::Row),
                flex_shrink: Some(1.0),
                min_width: Some(LengthSpec::Px(0.0)),
                ..LayoutStyle::default()
            }),
        );
        queue.set_style(
            label_id(0),
            label_style(LayoutStyle {
                white_space_nowrap: true,
                text_overflow_ellipsis: true,
                flex_shrink: Some(1.0),
                min_width: Some(LengthSpec::Px(0.0)),
                ..LayoutStyle::default()
            }),
        );
    });
    runtime
}

fn relabel(runtime: &mut RuntimeDocument, label: &str) {
    commit(runtime, |queue| {
        queue.set_text(
            label_id(0),
            TextContent {
                value: label.into(),
            },
        )
    });
}

fn ellipsized(runtime: &RuntimeDocument) -> bool {
    runtime
        .context()
        .world()
        .text_layout(label_id(0))
        .unwrap()
        .1
        .overflow
        .contains(nana_text::OverflowFlags::ELLIPSIZED)
}

#[test]
fn a_shrink_to_fit_label_grows_when_its_text_gets_longer() {
    let mut runtime = shrink_row_around_an_ellipsizing_label(300.0, "ab");
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let short = runtime
        .context()
        .world()
        .layout_box(row_id(0))
        .unwrap()
        .width;

    relabel(&mut runtime, "a label far longer than the last one");
    settle(&mut runtime, &mut shaper);
    let long = runtime
        .context()
        .world()
        .layout_box(row_id(0))
        .unwrap()
        .width;
    assert!(
        long > short * 4.0,
        "the row must widen to the new text, not keep the old box: {short} -> {long}"
    );
    assert!(!ellipsized(&runtime), "a label with room is not cut");

    relabel(&mut runtime, "ab");
    settle(&mut runtime, &mut shaper);
    let back = runtime
        .context()
        .world()
        .layout_box(row_id(0))
        .unwrap()
        .width;
    assert!(
        (back - short).abs() < 0.5,
        "and narrows again: {short} vs {back}"
    );
}

#[test]
fn a_label_that_cannot_fit_still_ellipsizes_inside_its_header() {
    let mut runtime = shrink_row_around_an_ellipsizing_label(60.0, "ab");
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    relabel(&mut runtime, "a label far longer than the header");
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    let label = world.layout_box(label_id(0)).unwrap();
    assert!(
        label.width <= 60.0 + 0.01,
        "the label keeps to its header: {label:?}"
    );
    assert!(ellipsized(&runtime), "and cuts its line with an ellipsis");
    let before = text_work(&runtime);
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert_no_new_text_work(before, &runtime, "a settled cut label");
}

#[test]
fn a_content_change_shapes_only_the_node_that_changed() {
    let (mut runtime, _) = document(numbered(32));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_text(
            label_id(7),
            TextContent {
                value: "renamed".into(),
            },
        )
    });
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = text_work(&runtime);
    assert_eq!(work.text_nodes_shaped, 1);
    assert_eq!(
        work.text_source_clones, 1,
        "one new source for one new text"
    );
    assert_eq!(work.text_bytes_hashed, "renamed".len());
    let layout = runtime
        .context()
        .world()
        .text_layout(label_id(7))
        .unwrap()
        .1;
    assert_eq!(layout.lines[0].source, 0.."renamed".len());
}

#[test]
fn a_feature_or_variation_change_reshapes_without_touching_content() {
    let (mut runtime, _) = document(numbered(4));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let label = label_id(2);
    let before = runtime.context().world().text_revisions(label).unwrap();
    commit(&mut runtime, |queue| {
        queue.set_style(
            label,
            label_style(LayoutStyle {
                font_features: Some(vec![FontFeatureSetting {
                    tag: *b"liga",
                    value: 0,
                }]),
                ..LayoutStyle::default()
            }),
        )
    });
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = text_work(&runtime);
    assert_eq!(work.text_nodes_shaped, 1, "a feature is a shaping input");
    assert_eq!(work.text_source_clones, 0, "the text itself did not change");
    let after = runtime.context().world().text_revisions(label).unwrap();
    assert_eq!(after.content, before.content);
    assert!(after.shape > before.shape);
}

#[test]
fn identical_labels_share_one_layout_however_many_there_are() {
    let created = |count: usize| {
        let (mut runtime, rows) = document((0..count).map(|_| "Same label".to_owned()));
        let mut shaper = NanaTextEngineShaper::new(engine());
        runtime.flush(viewport(), &mut shaper).unwrap();
        let mut created = text_work(&runtime).layouts_created;
        for _ in 0..4 {
            // An idle frame keeps the previous frame's counters; count only
            // frames that ran.
            if !runtime.flush(viewport(), &mut shaper).unwrap().is_idle() {
                created += text_work(&runtime).layouts_created;
            }
        }
        let world = runtime.context().world();
        let first = Arc::clone(world.text_layout(label_id(0)).unwrap().1);
        for row in 1..rows {
            let (_, layout) = world.text_layout(label_id(row)).unwrap();
            assert!(Arc::ptr_eq(layout, &first), "row {row} shares the layout");
        }
        created
    };
    assert_eq!(
        created(10),
        created(200),
        "layouts built for identical labels do not grow with their count"
    );
}

/// The #33 workload: resizing the first row shifts every row below it, so the
/// layout-scoped text pass visits every label. What it may not do is pay for
/// any of them beyond an O(1) revision check.
fn head_dirty_counters(rows: usize, shaper: &mut impl TextShaper) -> TextWorkCounters {
    let (mut runtime, rows) = document(numbered(rows));
    settle(&mut runtime, shaper);
    let mut total = TextWorkCounters::default();
    for frame in 0..6 {
        commit(&mut runtime, |queue| {
            queue.set_style(
                row_id(0),
                NodeStyle {
                    layout: row_layout(if frame % 2 == 0 { 22.0 } else { 20.0 }),
                    ..NodeStyle::default()
                },
            )
        });
        let update = runtime.flush(viewport(), shaper).unwrap();
        assert!(!update.is_idle());
        let work = text_work(&runtime);
        assert_no_text_work(work, "head-dirty frame");
        assert_eq!(
            work.text_nodes_revision_skipped, work.text_nodes_considered,
            "every candidate is decided on revision"
        );
        assert!(
            work.text_nodes_revision_skipped >= rows,
            "the scope covers every label"
        );
        total.accumulate(work);
    }
    total
}

#[test]
fn the_head_dirty_workload_skips_every_label_on_revision_with_either_backend() {
    for rows in [250, 500, 1000] {
        let measured = head_dirty_counters(rows, &mut MeasureTextShaper);
        let engine = head_dirty_counters(rows, &mut NanaTextEngineShaper::new(engine()));
        assert_eq!(
            measured.text_nodes_revision_skipped,
            engine.text_nodes_revision_skipped
        );
    }
    // Twice the document, twice the candidates: linear, never quadratic.
    let half = head_dirty_counters(400, &mut MeasureTextShaper).text_nodes_considered;
    let full = head_dirty_counters(800, &mut MeasureTextShaper).text_nodes_considered;
    assert_eq!(full, half * 2);
}

#[test]
fn a_font_set_change_retires_every_layout_laid_out_against_the_old_one() {
    let (mut runtime, rows) = document(numbered(6));
    let shared = engine();
    let mut shaper = NanaTextEngineShaper::new(Arc::clone(&shared));
    settle(&mut runtime, &mut shaper);
    let old: Vec<_> = (0..rows)
        .map(|row| {
            runtime
                .context()
                .world()
                .text_layout(label_id(row))
                .unwrap()
        })
        .map(|(handle, layout)| (handle, Arc::clone(layout)))
        .collect();

    let generation = {
        let mut engine = nana_text::lock_text_engine(&shared);
        engine
            .fonts_mut()
            .register_bytes(
                font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
                &FaceDescriptor::family("Second Copy"),
            )
            .unwrap();
        engine.epoch().font_generation
    };

    // Nothing in the document changed; the font set did. The next flush is not
    // idle, and keeps no layout from the old set.
    let update = runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(
        !update.is_idle(),
        "a new font set is work for a static document"
    );
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    for (row, (handle, layout)) in old.iter().enumerate() {
        let (now, current) = world.text_layout(label_id(row)).unwrap();
        assert_ne!(now, *handle, "row {row}: a stale handle was kept");
        assert_ne!(layout.font_generation, generation);
        assert_eq!(current.font_generation, generation, "row {row}");
    }
}

#[test]
fn layouts_are_released_with_their_nodes_and_survive_a_reparent() {
    let (mut runtime, rows) = document(numbered(5));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    assert_eq!(runtime.context().world().retained_text_layouts(), rows);

    // Removal releases exactly that node's layout.
    commit(&mut runtime, |queue| queue.despawn_subtree(row_id(4)));
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert_eq!(runtime.context().world().retained_text_layouts(), rows - 1);

    // A reparent into an identical row keeps the layout and its handle.
    let handle = runtime
        .context()
        .world()
        .text_layout(label_id(3))
        .unwrap()
        .0;
    commit(&mut runtime, |queue| {
        queue.detach(label_id(3));
        queue.insert(row_id(0), label_id(3), None);
    });
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    assert_eq!(world.retained_text_layouts(), rows - 1);
    assert_eq!(world.text_layout(label_id(3)).unwrap().0, handle);

    // Tearing the document's content down releases every layout.
    commit(&mut runtime, |queue| queue.despawn_subtree(id(COLUMN)));
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert_eq!(runtime.context().world().retained_text_layouts(), 0);
}

#[test]
fn a_steady_transform_and_opacity_animation_never_touches_text_revisions() {
    let (mut runtime, rows) = document(numbered(12));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let before: Vec<_> = (0..rows)
        .map(|row| {
            runtime
                .context()
                .world()
                .text_revisions(label_id(row))
                .unwrap()
        })
        .collect();
    let spec = |raw: u64, target, property, from, to| {
        AnimationSpec::new(
            AnimationId::new(raw).unwrap(),
            target,
            Duration::ZERO,
            Duration::from_millis(400),
            Duration::from_millis(16),
            Easing::Linear,
        )
        .with_property(property)
        .with_range(from, MotionTo::Value(to))
    };
    let shifted = |e: f32| {
        MotionValue::Transform(PaintTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e,
            f: 0.0,
        })
    };
    commit(&mut runtime, |queue| {
        for row in 0..rows {
            let label = label_id(row);
            queue.start_animation(spec(
                1 + row as u64 * 2,
                label,
                AnimatableProperty::Opacity,
                MotionValue::Scalar(0.0),
                MotionValue::Scalar(1.0),
            ));
            queue.start_animation(spec(
                2 + row as u64 * 2,
                label,
                AnimatableProperty::Transform,
                shifted(0.0),
                shifted(40.0),
            ));
        }
    });
    for frame in 0..=25u64 {
        runtime
            .context_mut()
            .advance_animations(Duration::from_millis(frame * 16));
        let update = runtime.flush(viewport(), &mut shaper).unwrap();
        // Presentation-only animation leaves the retained world alone: the
        // frame has no work, text pass included.
        assert!(update.is_idle(), "frame {frame} ran Runtime work");
    }
    let world = runtime.context().world();
    for (row, revisions) in before.iter().enumerate() {
        let now = world.text_revisions(label_id(row)).unwrap();
        assert_eq!(
            (now.content, now.shape, now.constraint),
            (revisions.content, revisions.shape, revisions.constraint),
            "row {row}: presentation animation moved a text revision"
        );
    }
}

#[test]
fn an_animated_padding_reflows_the_text_inside() {
    let (mut runtime, _) = document(std::iter::empty());
    let paragraph = id(900);
    commit(&mut runtime, |queue| {
        queue.create(
            paragraph,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Text,
        );
        queue.insert(id(COLUMN), paragraph, None);
        queue.set_text(
            paragraph,
            TextContent {
                value: "a paragraph long enough to wrap onto more lines once padded".into(),
            },
        );
        queue.set_style(
            paragraph,
            label_style(LayoutStyle {
                width: Some(LengthSpec::Px(260.0)),
                ..LayoutStyle::default()
            }),
        );
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let lines_before = runtime
        .context()
        .world()
        .text_layout(paragraph)
        .unwrap()
        .1
        .lines
        .len();

    commit(&mut runtime, |queue| {
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                paragraph,
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Padding)
            .with_range(
                MotionValue::Scalar(0.0),
                MotionTo::Value(MotionValue::Scalar(80.0)),
            ),
        )
    });
    runtime.context_mut().advance_animations(Duration::ZERO);
    runtime.flush(viewport(), &mut shaper).unwrap();
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(100));
    settle(&mut runtime, &mut shaper);

    let world = runtime.context().world();
    let layout = world.text_layout(paragraph).unwrap().1;
    assert!(
        layout.lines.len() > lines_before,
        "{} lines at 80px padding, {lines_before} without",
        layout.lines.len()
    );
    assert_eq!(layout.constraints.max_width_px, Some(260.0 - 160.0));
}

#[test]
fn an_animated_height_that_only_becomes_definite_is_still_a_constraint_change() {
    let (mut runtime, _) = document(std::iter::empty());
    let paragraph = id(900);
    commit(&mut runtime, |queue| {
        queue.create(
            paragraph,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Text,
        );
        queue.insert(id(COLUMN), paragraph, None);
        queue.set_text(
            paragraph,
            TextContent {
                value: "one line".into(),
            },
        );
        queue.set_style(
            paragraph,
            label_style(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                // The witness below is `max_height_px`, and a height only
                // reaches the engine when it is a *truncation* budget: a box
                // too short for its text is an overflow the scissor clips, not
                // a paragraph with lines removed. Asking for an ellipsis is
                // what makes the height mean something to layout, and this
                // test is about the height becoming definite, not about which
                // field carries it.
                text_overflow_ellipsis: true,
                ..LayoutStyle::default()
            }),
        );
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let height = runtime
        .context()
        .world()
        .layout_box(paragraph)
        .unwrap()
        .height;
    assert_eq!(
        runtime
            .context()
            .world()
            .text_layout(paragraph)
            .unwrap()
            .1
            .constraints
            .max_height_px,
        None
    );

    // The box keeps its size; only its height stops being `auto`, which is
    // what gives the text a height budget.
    commit(&mut runtime, |queue| {
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                paragraph,
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Height)
            .with_range(
                MotionValue::Scalar(height),
                MotionTo::Value(MotionValue::Scalar(height)),
            ),
        )
    });
    runtime.context_mut().advance_animations(Duration::ZERO);
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(100));
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    assert_eq!(world.layout_box(paragraph).unwrap().height, height);
    assert_eq!(
        world
            .text_layout(paragraph)
            .unwrap()
            .1
            .constraints
            .max_height_px,
        Some(height)
    );
}

#[test]
fn component_text_measured_in_the_same_pass_goes_through_the_same_engine() {
    // EmptyState, editor and button text are measured by the host's `shape`
    // inside the very passes that resolve plain labels through the engine:
    // no re-entrant lock, and the engine's work there is counted.
    let document = DocumentId::new(DOCUMENT).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    runtime
        .context_mut()
        .build(document, |ui| {
            ui.child(
                "empty",
                EmptyState::new("Nothing here").message("Add a file"),
            );
            ui.child("input", TextInput::new("editable text"));
            ui.child("button", Button::new("Save"));
        })
        .unwrap();
    let shared = engine();
    let mut shaper = NanaTextEngineShaper::new(Arc::clone(&shared));
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = text_work(&runtime);
    let created = nana_text::lock_text_engine(&shared)
        .layout_counters()
        .layout_created;
    assert!(created > 0);
    assert_eq!(
        work.layouts_created, created,
        "every layout the engine built this frame is on the frame's counters"
    );
    settle(&mut runtime, &mut shaper);
    assert!(runtime.scene().primitives().any(|primitive| matches!(
        &primitive.kind,
        ScenePrimitiveKind::Text { content, .. } if content == "Nothing here"
    )));
}

#[test]
fn a_font_set_change_also_remeasures_text_that_is_never_stamped() {
    // EmptyState text is measured by the host whenever it is visited and is
    // never stamped, so a font change has to schedule it explicitly.
    #[derive(Default)]
    struct GenerationShaper {
        generation: u64,
        shaped: Vec<String>,
    }
    impl TextShaper for GenerationShaper {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &nana_ui_runtime::ComputedStyle,
            constraints: nana_ui_runtime::TextShapeConstraints,
        ) -> nana_ui_runtime::TextMetrics {
            self.shaped.push(text.value.to_string());
            MeasureTextShaper.shape(id, text, style, constraints)
        }

        fn font_generation(&self) -> u64 {
            self.generation
        }
    }

    let (mut runtime, _) = document(numbered(1));
    let empty = id(900);
    commit(&mut runtime, |queue| {
        queue.create(
            empty,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(id(COLUMN), empty, None);
        queue.set_style(empty, NodeStyle::default());
        queue.set_standard_visual(
            empty,
            Some(nana_ui_runtime::StandardVisual::EmptyState {
                title: "Nothing here".into(),
                message: None,
                icon: None,
                compact: true,
                action: None,
            }),
        );
    });
    let mut shaper = GenerationShaper::default();
    settle(&mut runtime, &mut shaper);

    shaper.generation += 1;
    shaper.shaped.clear();
    settle(&mut runtime, &mut shaper);
    assert!(
        shaper.shaped.iter().any(|text| text == "Nothing here"),
        "the EmptyState title was measured against the new font set: {:?}",
        shaper.shaped
    );
}

#[test]
fn a_failed_pass_stamps_nothing_so_the_retry_resolves_every_node_again() {
    // The second label fails the first attempt after the first one measured
    // fine. Nothing of that attempt may stick: a stamp without its metrics
    // would skip the first label on retry and keep it unmeasured.
    struct FailOnce {
        failed: bool,
        shaped: Vec<String>,
    }
    impl TextShaper for FailOnce {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &nana_ui_runtime::ComputedStyle,
            constraints: nana_ui_runtime::TextShapeConstraints,
        ) -> nana_ui_runtime::TextMetrics {
            self.shaped.push(text.value.to_string());
            let mut metrics = MeasureTextShaper.shape(id, text, style, constraints);
            if text.value == "second" && !self.failed {
                self.failed = true;
                metrics.width = f32::NAN;
            }
            metrics
        }
    }

    let (mut runtime, _) = document(["first".to_owned(), "second".to_owned()]);
    let mut shaper = FailOnce {
        failed: false,
        shaped: Vec::new(),
    };
    assert!(runtime.flush(viewport(), &mut shaper).is_err());
    assert!(shaper.shaped.contains(&"first".to_owned()));

    shaper.shaped.clear();
    settle(&mut runtime, &mut shaper);
    assert!(
        shaper.shaped.contains(&"first".to_owned()),
        "the retry measured the first label again: {:?}",
        shaper.shaped
    );
    let world = runtime.context().world();
    assert!(world.text_metrics(label_id(0)).unwrap().width > 0.0);
    assert!(world.text_metrics(label_id(1)).unwrap().width > 0.0);
}

#[test]
fn a_re_resolution_that_yields_the_same_layout_keeps_the_handle() {
    let (mut runtime, _) = document(numbered(1));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let label = label_id(0);
    let (handle, layout) = {
        let (handle, layout) = runtime.context().world().text_layout(label).unwrap();
        (handle, Arc::clone(layout))
    };
    // A taller box whose height is not definite gives the text the same
    // constraints: it is resolved again and gets the very same layout back.
    let mut taller = runtime.context().world().layout_box(label).unwrap();
    taller.height += 10.0;
    commit(&mut runtime, |queue| queue.write_layout(label, taller));
    runtime
        .context_mut()
        .shape_text_for_layout_scoped(&[label], &mut shaper)
        .unwrap();
    let work = text_work(&runtime);
    assert_eq!(
        work.text_nodes_revision_skipped, 0,
        "the resize was a constraint change"
    );
    assert_eq!(work.text_layouts_reused, 1);
    let (now, current) = runtime.context().world().text_layout(label).unwrap();
    assert_eq!(now, handle);
    assert!(Arc::ptr_eq(current, &layout));
}

#[test]
fn an_empty_text_node_is_remeasured_on_a_new_font_set() {
    // An empty Text node still measures a line box, whose height is the
    // font's; it is not a box with nothing to measure.
    #[derive(Default)]
    struct TallerFonts {
        generation: u64,
    }
    impl TextShaper for TallerFonts {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &nana_ui_runtime::ComputedStyle,
            constraints: nana_ui_runtime::TextShapeConstraints,
        ) -> nana_ui_runtime::TextMetrics {
            let mut metrics = MeasureTextShaper.shape(id, text, style, constraints);
            metrics.height = 10.0 + 7.0 * self.generation as f32;
            metrics
        }

        fn font_generation(&self) -> u64 {
            self.generation
        }
    }
    let (mut runtime, _) = document(["x".to_owned(), String::new()]);
    let mut shaper = TallerFonts::default();
    settle(&mut runtime, &mut shaper);
    shaper.generation = 1;
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    assert_eq!(world.text_metrics(label_id(0)).unwrap().height, 17.0);
    assert_eq!(world.text_metrics(label_id(1)).unwrap().height, 17.0);
}

#[test]
fn a_label_that_becomes_an_empty_state_releases_its_layout() {
    let (mut runtime, rows) = document(numbered(2));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    assert_eq!(runtime.context().world().retained_text_layouts(), rows);
    commit(&mut runtime, |queue| {
        queue.set_standard_visual(
            label_id(0),
            Some(nana_ui_runtime::StandardVisual::EmptyState {
                title: "Nothing here".into(),
                message: None,
                icon: None,
                compact: true,
                action: None,
            }),
        )
    });
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    assert!(world.text_layout(label_id(0)).is_none());
    assert_eq!(world.retained_text_layouts(), rows - 1);
}

#[test]
fn an_alignment_change_alone_relays_the_retained_layout_out() {
    let (mut runtime, _) = document(numbered(1));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let label = label_id(0);
    commit(&mut runtime, |queue| {
        queue.set_style(
            label,
            NodeStyle {
                text_horizontal_alignment: nana_ui_runtime::TextHorizontalAlignment::Center,
                ..NodeStyle::default()
            },
        )
    });
    settle(&mut runtime, &mut shaper);
    let layout = runtime.context().world().text_layout(label).unwrap().1;
    assert_eq!(
        layout.constraints.align,
        nana_ui_core::TextAlignSpec::Center
    );
}

#[test]
fn an_authored_line_clamp_change_relays_the_retained_layout_out() {
    let (mut runtime, _) = document(std::iter::empty());
    let paragraph = id(900);
    let clamped = |lines: Option<u16>| {
        // A fixed box: the clamp changes no box size, so only the style
        // comparison can tell the text its constraints moved.
        label_style(LayoutStyle {
            width: Some(LengthSpec::Px(90.0)),
            height: Some(LengthSpec::Px(200.0)),
            line_clamp: lines,
            ..LayoutStyle::default()
        })
    };
    commit(&mut runtime, |queue| {
        queue.create(
            paragraph,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Text,
        );
        queue.insert(id(COLUMN), paragraph, None);
        queue.set_text(
            paragraph,
            TextContent {
                value: "a paragraph long enough to wrap onto several lines at this width".into(),
            },
        );
        queue.set_style(paragraph, clamped(None));
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    assert!(
        runtime
            .context()
            .world()
            .text_layout(paragraph)
            .unwrap()
            .1
            .lines
            .len()
            > 2
    );
    commit(&mut runtime, |queue| {
        queue.set_style(paragraph, clamped(Some(2)))
    });
    settle(&mut runtime, &mut shaper);
    let layout = runtime.context().world().text_layout(paragraph).unwrap().1;
    assert_eq!(layout.constraints.max_lines, Some(2));
    assert_eq!(layout.lines.len(), 2);
    // A clamp to another clamp: ellipsis and wrapping stay as they were.
    commit(&mut runtime, |queue| {
        queue.set_style(paragraph, clamped(Some(3)))
    });
    settle(&mut runtime, &mut shaper);
    let layout = runtime.context().world().text_layout(paragraph).unwrap().1;
    assert_eq!(layout.constraints.max_lines, Some(3));
    assert_eq!(layout.lines.len(), 3);
}

#[test]
fn an_element_whose_text_is_cleared_releases_its_layout() {
    let (mut runtime, _) = document(std::iter::empty());
    let element = id(900);
    commit(&mut runtime, |queue| {
        queue.create(
            element,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Element { tag: "span".into() },
        );
        queue.insert(id(COLUMN), element, None);
        queue.set_style(element, NodeStyle::default());
        queue.set_text(
            element,
            TextContent {
                value: "element text".into(),
            },
        );
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    assert!(runtime.context().world().text_layout(element).is_some());
    commit(&mut runtime, |queue| {
        queue.set_text(element, TextContent::default())
    });
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    assert!(world.text_layout(element).is_none());
    assert_eq!(world.retained_text_layouts(), 0);
}

#[test]
fn a_host_that_stops_offering_an_engine_releases_every_layout() {
    let (mut runtime, rows) = document(numbered(3));
    settle(&mut runtime, &mut NanaTextEngineShaper::new(engine()));
    assert_eq!(runtime.context().world().retained_text_layouts(), rows);
    settle(&mut runtime, &mut MeasureTextShaper);
    assert_eq!(runtime.context().world().retained_text_layouts(), 0);
}

#[test]
fn a_language_change_remeasures_component_text_through_the_engine_host() {
    let document = DocumentId::new(DOCUMENT).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    runtime
        .context_mut()
        .build(document, |ui| {
            ui.child("empty", EmptyState::new("Nothing here"));
        })
        .unwrap();
    let shared = engine();
    let mut shaper = NanaTextEngineShaper::new(Arc::clone(&shared));
    settle(&mut runtime, &mut shaper);
    nana_text::lock_text_engine(&shared)
        .set_language(Some(nana_text::font::LanguageTag::new("ja").unwrap()));
    let update = runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(!update.is_idle());
    assert!(
        runtime
            .context()
            .last_work_counters()
            .text_layout_cache_misses
            > 0,
        "measurements taken under the old language are not reused"
    );
}

#[test]
fn an_element_whose_input_state_is_cleared_releases_its_layout() {
    let (mut runtime, _) = document(std::iter::empty());
    let element = id(900);
    commit(&mut runtime, |queue| {
        queue.create(
            element,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Element { tag: "span".into() },
        );
        queue.insert(id(COLUMN), element, None);
        queue.set_style(element, NodeStyle::default());
        queue.set_text_input(element, Some(nana_ui_runtime::TextInputState::new("hello")));
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    assert!(runtime.context().world().text_layout(element).is_some());
    commit(&mut runtime, |queue| queue.set_text_input(element, None));
    settle(&mut runtime, &mut shaper);
    assert!(runtime.context().world().text_layout(element).is_none());
    assert_eq!(runtime.context().world().retained_text_layouts(), 0);
}

#[test]
fn text_resized_while_hidden_is_resolved_again_when_shown() {
    let (mut runtime, _) = document(std::iter::empty());
    let paragraph = id(900);
    let styled = |width: f32, hidden: bool| {
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            ..LayoutStyle::default()
        };
        layout.paint.visibility = hidden.then_some(nana_ui_core::VisibilitySpec::Hidden);
        label_style(layout)
    };
    commit(&mut runtime, |queue| {
        queue.create(
            paragraph,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Text,
        );
        queue.insert(id(COLUMN), paragraph, None);
        queue.set_text(
            paragraph,
            TextContent {
                value: "a paragraph long enough to wrap onto several lines when narrow".into(),
            },
        );
        queue.set_style(paragraph, styled(280.0, false));
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_style(paragraph, styled(280.0, true))
    });
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_style(paragraph, styled(90.0, true))
    });
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_style(paragraph, styled(90.0, false))
    });
    settle(&mut runtime, &mut shaper);
    let world = runtime.context().world();
    let layout = world.text_layout(paragraph).unwrap().1;
    assert_eq!(layout.constraints.max_width_px, Some(90.0));
    assert!(world.text_metrics(paragraph).unwrap().width <= 90.0);
}

#[test]
fn a_leading_checkbox_is_a_constraint_change_for_the_label_beside_it() {
    let (mut runtime, _) = document(std::iter::empty());
    let label = id(900);
    commit(&mut runtime, |queue| {
        queue.create(label, DocumentId::new(DOCUMENT).unwrap(), NodeKind::Text);
        queue.insert(id(COLUMN), label, None);
        queue.set_text(
            label,
            TextContent {
                value: "a label long enough to wrap once a checkbox takes its inset".into(),
            },
        );
        queue.set_style(
            label,
            label_style(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                ..LayoutStyle::default()
            }),
        );
    });
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_standard_visual(
            label,
            Some(nana_ui_runtime::StandardVisual::Checkbox {
                checked: false,
                indeterminate: false,
                size: nana_ui_core::ControlSize::Large,
            }),
        )
    });
    settle(&mut runtime, &mut shaper);
    let layout = runtime.context().world().text_layout(label).unwrap().1;
    assert!(
        layout.constraints.max_width_px.unwrap() < 200.0,
        "the indicator's inset reached the retained layout: {:?}",
        layout.constraints.max_width_px
    );
}

#[test]
fn showing_hidden_text_settles_in_the_same_pass() {
    let (mut runtime, rows) = document(numbered(50));
    let mut shaper = MeasureTextShaper;
    settle(&mut runtime, &mut shaper);
    let column = |hidden: bool| {
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        };
        layout.paint.visibility = hidden.then_some(nana_ui_core::VisibilitySpec::Hidden);
        label_style(layout)
    };
    commit(&mut runtime, |queue| {
        queue.set_style(id(COLUMN), column(true))
    });
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_style(id(COLUMN), column(false))
    });
    let update = runtime.flush(viewport(), &mut shaper).unwrap();
    assert_eq!(update.passes, 1, "showing text costs no extra flush pass");
    assert_eq!(text_work(&runtime).text_nodes_considered, rows);
}

#[test]
fn an_alignment_change_is_not_text_work_for_a_host_measured_label() {
    let (mut runtime, _) = document(numbered(1));
    let mut shaper = MeasureTextShaper;
    settle(&mut runtime, &mut shaper);
    let before = text_work(&runtime);
    commit(&mut runtime, |queue| {
        queue.set_style(
            label_id(0),
            NodeStyle {
                text_horizontal_alignment: nana_ui_runtime::TextHorizontalAlignment::Center,
                ..NodeStyle::default()
            },
        )
    });
    runtime.flush(viewport(), &mut shaper).unwrap();
    let now = text_work(&runtime);
    assert!(
        now == before || now.text_nodes_considered == 0,
        "host metrics do not read alignment: no text pass reached the label: {now:?}"
    );
}

#[test]
fn a_font_set_change_drops_cached_glyph_advances() {
    struct GlyphHost {
        generation: u64,
    }
    impl TextShaper for GlyphHost {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &nana_ui_runtime::ComputedStyle,
            constraints: nana_ui_runtime::TextShapeConstraints,
        ) -> nana_ui_runtime::TextMetrics {
            MeasureTextShaper.shape(id, text, style, constraints)
        }

        fn shape_cached(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &nana_ui_runtime::ComputedStyle,
            constraints: nana_ui_runtime::TextShapeConstraints,
            glyphs: &mut nana_ui_runtime::GlyphCache,
        ) -> nana_ui_runtime::TextMetrics {
            MeasureTextShaper.shape_cached(id, text, style, constraints, glyphs)
        }

        fn font_generation(&self) -> u64 {
            self.generation
        }
    }
    let (mut runtime, _) = document(["x".to_owned()]);
    let mut shaper = GlyphHost { generation: 0 };
    settle(&mut runtime, &mut shaper);
    shaper.generation = 1;
    runtime.flush(viewport(), &mut shaper).unwrap();
    let counters = runtime.context().last_work_counters();
    assert!(
        counters.glyph_cache_misses.unwrap_or(0) > 0,
        "the advance was measured again under the new font set: {counters:?}"
    );
}

#[test]
fn text_shown_in_a_frame_that_fails_is_still_resolved_on_retry() {
    struct FailOnBoom {
        failed: bool,
    }
    impl TextShaper for FailOnBoom {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &nana_ui_runtime::ComputedStyle,
            constraints: nana_ui_runtime::TextShapeConstraints,
        ) -> nana_ui_runtime::TextMetrics {
            let mut metrics = MeasureTextShaper.shape(id, text, style, constraints);
            if text.value == "boom" && !self.failed {
                self.failed = true;
                metrics.width = f32::NAN;
            }
            metrics
        }
    }
    let (mut runtime, _) = document(["label".to_owned()]);
    let paragraph = id(900);
    let column = |hidden: bool| {
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        };
        layout.paint.visibility = hidden.then_some(nana_ui_core::VisibilitySpec::Hidden);
        label_style(layout)
    };
    let sized = |width: f32| {
        label_style(LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            ..LayoutStyle::default()
        })
    };
    commit(&mut runtime, |queue| {
        queue.create(
            paragraph,
            DocumentId::new(DOCUMENT).unwrap(),
            NodeKind::Text,
        );
        queue.insert(id(COLUMN), paragraph, None);
        queue.set_text(
            paragraph,
            TextContent {
                value: "a paragraph long enough to wrap onto several lines when narrow".into(),
            },
        );
        queue.set_style(paragraph, sized(280.0));
    });
    let mut shaper = FailOnBoom { failed: false };
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_style(id(COLUMN), column(true))
    });
    settle(&mut runtime, &mut shaper);
    commit(&mut runtime, |queue| {
        queue.set_style(paragraph, sized(90.0))
    });
    settle(&mut runtime, &mut shaper);
    // Shown in the same frame another text fails to measure.
    commit(&mut runtime, |queue| {
        queue.set_style(id(COLUMN), column(false));
        queue.set_text(
            label_id(0),
            TextContent {
                value: "boom".into(),
            },
        );
    });
    assert!(runtime.flush(viewport(), &mut shaper).is_err());
    settle(&mut runtime, &mut shaper);
    let width = runtime
        .context()
        .world()
        .text_metrics(paragraph)
        .unwrap()
        .width;
    assert!(
        width <= 90.0,
        "measured at the 90px box it was given while hidden: {width}"
    );
}

#[test]
fn another_host_shaper_at_the_same_font_generation_measures_again() {
    // Both hosts report font generation 0; their numbers must not be mixed.
    struct Wide;
    impl TextShaper for Wide {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &nana_ui_runtime::ComputedStyle,
            constraints: nana_ui_runtime::TextShapeConstraints,
        ) -> nana_ui_runtime::TextMetrics {
            let mut metrics = MeasureTextShaper.shape(id, text, style, constraints);
            metrics.height = 99.0;
            metrics
        }
    }
    let (mut runtime, _) = document(["label".to_owned()]);
    settle(&mut runtime, &mut MeasureTextShaper);
    assert_ne!(
        runtime
            .context()
            .world()
            .text_metrics(label_id(0))
            .unwrap()
            .height,
        99.0
    );
    settle(&mut runtime, &mut Wide);
    assert_eq!(
        runtime
            .context()
            .world()
            .text_metrics(label_id(0))
            .unwrap()
            .height,
        99.0
    );
}
