//! Issue #384: a range's label, value and unit, a progress bar's label and
//! cancel target, and a modal's title, description and body text are parts
//! layout publishes; geometry reads the parts and keeps no hand-placed
//! copy. A range's value has a fixed slot, so dragging repaints only.
#![cfg(test)]

use super::reflow_oracle::{assert_matches_cold, bundled_face_shaper, product_frame};
use crate::{
    AppContext, ComponentGeometry, DocumentId, Entity, LayoutPartKind, LayoutViewport, RangeField,
    StableNodeId, Stack,
};
use nana_ui_core::{ControlSize, LengthSpec};
use std::sync::Arc;
use std::time::Duration;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(600.0, 400.0)
}

fn part(cx: &AppContext, id: StableNodeId, kind: LayoutPartKind) -> Option<crate::LayoutBox> {
    cx.world()
        .layout_result(id)
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == kind)
        .map(|part| part.bounds)
}

/// A range's declared inputs, the ones its parts read.
#[derive(Clone, Debug, PartialEq)]
struct RangeCase {
    value: f64,
    minimum: f64,
    maximum: f64,
    step: f64,
    label: Option<&'static str>,
    unit: Option<&'static str>,
    size: ControlSize,
    show_value: bool,
}

const VOLUME: RangeCase = RangeCase {
    value: 25.0,
    minimum: 0.0,
    maximum: 100.0,
    step: 1.0,
    label: Some("Volume"),
    unit: Some("%"),
    size: ControlSize::Medium,
    show_value: true,
};

fn range_view(case: &RangeCase) -> RangeField {
    let mut view = RangeField::new(case.value, case.minimum, case.maximum, case.step)
        .size(case.size)
        .show_value(case.show_value);
    if let Some(label) = case.label {
        view = view.label(label);
    }
    if let Some(unit) = case.unit {
        view = view.unit(unit);
    }
    view
}

/// `case` in a 360px column under `theme` when given, laid out once.
fn range_page(
    case: &RangeCase,
    theme: Option<&nana_ui_core::ThemeDefinition>,
) -> (AppContext, Entity<RangeField>) {
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(document(), Stack::column(4.0).width(LengthSpec::Px(360.0)))
        .unwrap();
    let range = cx
        .create_detached_component(document(), range_view(case))
        .unwrap();
    cx.append_child(root, range).unwrap();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    cx.rebuild_hit_test(document());
    (cx, range)
}

fn range_geometry(
    cx: &AppContext,
    id: StableNodeId,
) -> (
    Option<crate::ComponentTextRegion>,
    crate::ComponentTextRegion,
    Option<crate::ComponentTextRegion>,
    crate::LayoutBox,
) {
    match cx.world().component_geometry(id).unwrap() {
        ComponentGeometry::Range {
            label,
            value,
            unit,
            track,
            ..
        } => (label, value, unit, track),
        other => panic!("{other:?}"),
    }
}

/// The formatted values a range steps through, from its minimum to its
/// maximum.
fn every_value(case: &RangeCase) -> Vec<Arc<str>> {
    let steps = ((case.maximum - case.minimum) / case.step).round() as usize;
    (0..=steps)
        .map(|index| {
            crate::view_components::format_range_value(
                case.minimum + index as f64 * case.step,
                case.step,
            )
        })
        .collect()
}

/// Paint reads the published label, value and unit parts and runs the track
/// in the lane between them; the value's slot is the wider of the formatted
/// limits in tabular figures, and every value the range steps through fits
/// it as painted.
#[test]
fn issue384_range_label_value_and_unit_are_published_parts() {
    for case in [
        VOLUME,
        RangeCase {
            value: -2.5,
            minimum: -12.0,
            maximum: 6.0,
            step: 0.5,
            label: Some("Gain"),
            unit: Some("dB"),
            size: ControlSize::Small,
            show_value: true,
        },
        RangeCase {
            label: None,
            unit: None,
            size: ControlSize::Large,
            ..VOLUME
        },
        RangeCase {
            show_value: false,
            ..VOLUME
        },
    ] {
        let (cx, range) = range_page(&case, None);
        let id = range.stable_id();
        let world = cx.world();
        let content = world.layout_result(id).unwrap().content_box;
        let style = world.computed_style(id).unwrap();
        let measure = world.chrome_text_measure(id);
        let gap = world.range_gap(case.size);
        let (label, value, unit, track) = range_geometry(&cx, id);
        let label_part = part(&cx, id, LayoutPartKind::Label);
        let value_part = part(&cx, id, LayoutPartKind::Value);
        let unit_part = part(&cx, id, LayoutPartKind::Detail);
        assert_eq!(label.as_ref().map(|label| label.bounds), label_part);
        assert_eq!(label_part.is_some(), case.label.is_some(), "{case:?}");
        assert_eq!(unit.as_ref().map(|unit| unit.bounds), unit_part);
        assert_eq!(
            unit_part.is_some(),
            case.unit.is_some() && case.show_value,
            "{case:?}"
        );
        if let Some(label) = label_part {
            assert_eq!(label.x, content.x);
            assert!(track.x >= label.x + label.width + gap - 1e-3);
        }
        match value_part {
            Some(slot) => {
                assert!(case.show_value);
                assert_eq!(value.bounds, slot);
                let limits = [
                    crate::view_components::format_range_value(case.minimum, case.step),
                    crate::view_components::format_range_value(case.maximum, case.step),
                ];
                let widest = limits
                    .iter()
                    .map(|limit| measure.tabular_width(limit, style.font_size, style.font_weight))
                    .fold(0.0, f32::max);
                assert_eq!(slot.width, widest, "{case:?}");
                for text in every_value(&case) {
                    assert!(
                        measure.width(&text, style.font_size, style.font_weight)
                            <= slot.width + 0.5,
                        "{text} overflows a {} slot",
                        slot.width
                    );
                }
                assert!(track.x + track.width <= slot.x - gap + 1e-3);
                let end = unit_part.unwrap_or(slot);
                assert!((end.x + end.width - (content.x + content.width)).abs() < 1e-3);
            }
            None => {
                assert!(!case.show_value);
                assert_eq!(value.bounds.width, 0.0);
            }
        }
        // One entity: the range and the column.
        assert_eq!(world.len(), 2);
    }
}

/// Dragging a range moves its value and its thumb and lays nothing out:
/// every step records zero measure and placement work and keeps the
/// published result, and the painted value changes. Hovering does the same.
#[test]
fn issue384_range_drag_steps_record_zero_layout_work() {
    let (mut cx, range) = range_page(&VOLUME, None);
    let id = range.stable_id();
    let mut shaper = bundled_face_shaper();
    let (_, _, _, track) = range_geometry(&cx, id);
    let generation = cx.world().layout_generation();
    let snapshot = Arc::clone(&cx.world().layout_results[&id]);
    let parts = cx.world().layout_result(id).unwrap().parts.clone();
    let mut painted = Vec::new();
    for (step, share) in [0.0_f32, 0.07, 0.5, 0.93, 1.0, 0.31]
        .into_iter()
        .enumerate()
    {
        let x = track.x + track.width * share;
        if step == 0 {
            assert!(cx.begin_range_drag(document(), 1, id, x).unwrap());
        } else {
            cx.update_range_drag(document(), 1, x).unwrap();
        }
        let work = product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert_eq!(
            (work.layout_measure_nodes, work.layout_placement_nodes),
            (0, 0),
            "drag step {step}"
        );
        let (_, value, _, _) = range_geometry(&cx, id);
        painted.push(value.content.to_string());
    }
    cx.end_range_drag(document(), 1, false).unwrap();
    let work = product_frame(&mut cx, document(), viewport(), &mut shaper);
    assert_eq!(
        (work.layout_measure_nodes, work.layout_placement_nodes),
        (0, 0)
    );
    cx.set_pointer_hover_at(document(), 1, Some(id), Duration::from_millis(16))
        .unwrap();
    let work = product_frame(&mut cx, document(), viewport(), &mut shaper);
    assert_eq!(
        (work.layout_measure_nodes, work.layout_placement_nodes),
        (0, 0)
    );
    assert_eq!(cx.world().layout_generation(), generation);
    assert!(Arc::ptr_eq(&snapshot, &cx.world().layout_results[&id]));
    assert_eq!(cx.world().layout_result(id).unwrap().parts, parts);
    assert_eq!(painted, ["0", "7", "50", "93", "100", "31"]);
    assert_eq!(cx.read(range, |range| range.value).unwrap(), 31.0);
}

/// Label, limits, step, unit, size and readout changes, and a theme that
/// moves the gaps and the type, republish the parts a cold layout of the
/// same range would.
#[test]
fn issue384_range_parts_follow_changes_like_a_cold_layout() {
    let mut theme =
        nana_ui_core::ThemeDefinition::for_appearance(nana_ui_core::ThemeAppearance::Light);
    theme.tokens.spacing.sm += 2.0;
    theme.tokens.spacing.md += 3.0;
    theme.tokens.spacing.lg += 4.0;
    theme.typography.hint += 1.0;
    theme.typography.meta += 1.0;
    let theme = theme.bump();
    let mut shaper = bundled_face_shaper();
    let (mut cx, range) = range_page(&VOLUME, None);
    let steps = [
        (
            RangeCase {
                maximum: 1000.0,
                ..VOLUME
            },
            false,
        ),
        (
            RangeCase {
                minimum: -1000.0,
                maximum: 10.0,
                step: 0.25,
                value: 3.5,
                ..VOLUME
            },
            false,
        ),
        (
            RangeCase {
                unit: Some("Hz"),
                label: Some("A longer label for the frequency"),
                ..VOLUME
            },
            false,
        ),
        (
            RangeCase {
                size: ControlSize::Large,
                ..VOLUME
            },
            true,
        ),
        (
            RangeCase {
                show_value: false,
                label: None,
                ..VOLUME
            },
            true,
        ),
        (VOLUME, true),
    ];
    let mut themed = false;
    for (case, under_theme) in steps {
        cx.update_component(range, |view, _| *view = range_view(&case))
            .unwrap();
        if under_theme && !themed {
            cx.set_theme_definition(&theme).unwrap();
            themed = true;
        }
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, cold_range) = range_page(&case, under_theme.then_some(&theme));
        assert_eq!(cold_range.stable_id(), range.stable_id());
        assert_matches_cold(&mut cx, &mut cold, document());
        assert_eq!(
            range_geometry(&cx, range.stable_id()),
            range_geometry(&cold, range.stable_id()),
            "{case:?}"
        );
    }
}

/// A focused range moved between Grid, Inline and Flex parents keeps its
/// entity, value and focus, and publishes the parts a cold layout in that
/// context would.
#[test]
fn issue384_range_parts_survive_context_transitions() {
    use crate::MutationQueue;
    use nana_ui_core::DisplaySpec;
    let build = |display: DisplaySpec| {
        let mut cx = AppContext::new();
        let root = cx
            .create_component(
                document(),
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(320.0));
                    layout.display = Some(display);
                }),
            )
            .unwrap();
        let range = cx
            .create_detached_component(document(), range_view(&VOLUME))
            .unwrap();
        cx.append_child(root, range).unwrap();
        let mut q = MutationQueue::new();
        q.request_focus(document(), Some(range.stable_id()));
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
        (cx, root, range)
    };
    let mut shaper = bundled_face_shaper();
    let (mut cx, root, range) = build(DisplaySpec::Flex);
    let id = range.stable_id();
    for display in [DisplaySpec::Grid, DisplaySpec::Inline, DisplaySpec::Flex] {
        let mut style = cx.world().node_style(root.stable_id()).unwrap().clone();
        Arc::make_mut(&mut style.layout).display = Some(display);
        let mut q = MutationQueue::new();
        q.set_style(root.stable_id(), style);
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
        assert_eq!(cx.read(range, |range| range.value).unwrap(), 25.0);
        assert_eq!(cx.world().focused(document()), Some(id));
        let (label, value, unit, _) = range_geometry(&cx, id);
        assert_eq!(
            label.map(|label| label.bounds),
            part(&cx, id, LayoutPartKind::Label)
        );
        assert_eq!(Some(value.bounds), part(&cx, id, LayoutPartKind::Value));
        assert_eq!(
            unit.map(|unit| unit.bounds),
            part(&cx, id, LayoutPartKind::Detail)
        );
        let (mut cold, ..) = build(display);
        assert_matches_cold(&mut cx, &mut cold, document());
    }
}

/// 1k and 10k ranges keep one entity each and publish the content box and
/// three parts, and an idle frame lays nothing out.
#[test]
fn issue384_ten_thousand_ranges_keep_one_entity_and_bounded_parts() {
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let root = cx
            .create_component(document(), Stack::column(0.0).width(LengthSpec::Px(360.0)))
            .unwrap();
        let mut children = Vec::with_capacity(count);
        for _ in 0..count {
            children.push(
                cx.create_detached_component(document(), range_view(&VOLUME))
                    .unwrap()
                    .stable_id(),
            );
        }
        let mut q = crate::MutationQueue::new();
        for &id in &children {
            q.insert(root.stable_id(), id, None);
        }
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for &id in &children {
            let result = cx.world().layout_result(id).unwrap();
            let kinds: Vec<_> = result.parts.iter().map(|part| part.kind).collect();
            assert_eq!(
                kinds,
                [
                    LayoutPartKind::ComponentContent,
                    LayoutPartKind::Label,
                    LayoutPartKind::Value,
                    LayoutPartKind::Detail
                ]
            );
        }
        let generation = cx.world().layout_generation();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
    }
}

/// A progress bar's declared inputs, the ones its parts read.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ProgressCase {
    label: bool,
    cancellable: bool,
    thickness: Option<f32>,
    padding: Option<f32>,
}

const COPYING: ProgressCase = ProgressCase {
    label: true,
    cancellable: true,
    thickness: None,
    padding: None,
};

fn progress_view(case: ProgressCase, value: f64) -> crate::Progress {
    let mut view = crate::Progress::new(value, 100.0).cancellable(case.cancellable);
    if case.label {
        view = view.label("Copying");
    }
    if let Some(thickness) = case.thickness {
        view = view.thickness(thickness);
    }
    if let Some(padding) = case.padding.map(LengthSpec::Px) {
        let layout = Arc::make_mut(&mut view.style.layout);
        layout.padding_top = Some(padding);
        layout.padding_right = Some(padding);
        layout.padding_bottom = Some(padding);
        layout.padding_left = Some(padding);
    }
    view
}

/// `case` in a 320px column above a button, under `theme` when given, laid
/// out once.
fn progress_page(
    case: ProgressCase,
    theme: Option<&nana_ui_core::ThemeDefinition>,
) -> (AppContext, Entity<crate::Progress>) {
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(document(), Stack::column(4.0).width(LengthSpec::Px(320.0)))
        .unwrap();
    let progress = cx
        .create_detached_component(document(), progress_view(case, 40.0))
        .unwrap();
    cx.append_child(root, progress).unwrap();
    let below = cx
        .create_detached_component(document(), crate::Button::new("Below"))
        .unwrap();
    cx.append_child(root, below).unwrap();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    cx.rebuild_hit_test(document());
    (cx, progress)
}

fn progress_geometry(
    cx: &AppContext,
    id: StableNodeId,
) -> (
    crate::LayoutBox,
    crate::LayoutBox,
    Option<crate::ComponentTextRegion>,
    Option<crate::LayoutBox>,
) {
    match cx.world().component_geometry(id).unwrap() {
        ComponentGeometry::Progress {
            track,
            fill,
            label,
            cancel,
            ..
        } => (track, fill, label, cancel),
        other => panic!("{other:?}"),
    }
}

/// Paint reads the published label and cancel parts and runs the track
/// below them; measure makes the content box exactly the heading, its gap
/// and the track (no style height), inside an author's padding.
#[test]
fn issue384_progress_label_and_cancel_are_published_parts() {
    for case in [
        COPYING,
        ProgressCase {
            cancellable: false,
            ..COPYING
        },
        ProgressCase {
            label: false,
            ..COPYING
        },
        ProgressCase {
            label: false,
            cancellable: false,
            ..COPYING
        },
        ProgressCase {
            thickness: Some(3.0),
            padding: Some(10.0),
            ..COPYING
        },
    ] {
        let (cx, progress) = progress_page(case, None);
        let id = progress.stable_id();
        let world = cx.world();
        let result = world.layout_result(id).unwrap();
        let content = result.content_box;
        let spacing = world.theme().spacing();
        let typography = world.theme().typography();
        let thickness = case.thickness.unwrap_or(spacing.sm);
        let heading = if case.cancellable {
            spacing
                .page
                .max(if case.label { typography.meta } else { 0.0 })
        } else if case.label {
            typography.meta
        } else {
            0.0
        };
        let expected = if heading > 0.0 {
            heading + spacing.sm + thickness
        } else {
            thickness
        };
        assert!(
            (content.height - expected).abs() < 1e-3,
            "{case:?}: {content:?} for {expected}"
        );
        assert_eq!(world.node_style(id).unwrap().layout.height, None);
        if let Some(padding) = case.padding {
            assert!((content.x - result.border_box.x - padding).abs() < 1e-3);
            assert!((content.y - result.border_box.y - padding).abs() < 1e-3);
        }
        let (track, fill, label, cancel) = progress_geometry(&cx, id);
        let label_part = part(&cx, id, LayoutPartKind::Label);
        let cancel_part = part(&cx, id, LayoutPartKind::TrailingContent);
        assert_eq!(label.as_ref().map(|label| label.bounds), label_part);
        assert_eq!(cancel, cancel_part);
        assert_eq!(
            (label_part.is_some(), cancel_part.is_some()),
            (case.label, case.cancellable)
        );
        if let Some(cancel) = cancel_part {
            assert_eq!((cancel.width, cancel.height), (spacing.page, spacing.page));
            assert!((cancel.x + cancel.width - (content.x + content.width)).abs() < 1e-3);
            if let Some(label) = label_part {
                assert!((cancel.x - spacing.md - (label.x + label.width)).abs() < 1e-3);
            }
        }
        if let Some(label) = label_part {
            assert_eq!(label.x, content.x);
            assert_eq!(label.height, typography.meta);
        }
        assert_eq!((track.x, track.width), (content.x, content.width));
        assert_eq!(track.height, thickness);
        assert!((track.y + track.height - (content.y + content.height)).abs() < 1e-3);
        assert!((fill.width - track.width * 0.4).abs() < 1e-3);
        assert_eq!(world.len(), 3);
    }
}

/// Label, cancel target, thickness and padding changes, and a theme that
/// moves the gaps, the cancel target and the label's type, lay out what a
/// cold layout of the same bar does; the value, the sweep and hovering
/// paint only.
#[test]
fn issue384_progress_parts_follow_changes_like_a_cold_layout_and_value_paints_only() {
    let mut theme =
        nana_ui_core::ThemeDefinition::for_appearance(nana_ui_core::ThemeAppearance::Light);
    theme.tokens.spacing.sm += 2.0;
    theme.tokens.spacing.md += 3.0;
    theme.tokens.spacing.page += 4.0;
    theme.typography.meta += 2.0;
    let theme = theme.bump();
    let mut shaper = bundled_face_shaper();
    let (mut cx, progress) = progress_page(COPYING, None);
    let steps = [
        (
            ProgressCase {
                cancellable: false,
                ..COPYING
            },
            false,
        ),
        (
            ProgressCase {
                label: false,
                thickness: Some(3.0),
                ..COPYING
            },
            false,
        ),
        (
            ProgressCase {
                padding: Some(12.0),
                ..COPYING
            },
            false,
        ),
        (COPYING, true),
        (
            ProgressCase {
                label: false,
                cancellable: false,
                ..COPYING
            },
            true,
        ),
        (COPYING, true),
    ];
    let mut themed = false;
    for (case, under_theme) in steps {
        cx.update_component(progress, |view, _| *view = progress_view(case, 40.0))
            .unwrap();
        if under_theme && !themed {
            cx.set_theme_definition(&theme).unwrap();
            themed = true;
        }
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, cold_progress) = progress_page(case, under_theme.then_some(&theme));
        assert_eq!(cold_progress.stable_id(), progress.stable_id());
        assert_matches_cold(&mut cx, &mut cold, document());
        assert_eq!(
            progress_geometry(&cx, progress.stable_id()),
            progress_geometry(&cold, progress.stable_id()),
            "{case:?}"
        );
    }

    let id = progress.stable_id();
    let generation = cx.world().layout_generation();
    let snapshot = Arc::clone(&cx.world().layout_results[&id]);
    for value in [55.0, 90.0, 100.0] {
        cx.update_component(progress, |view, _| view.value = value)
            .unwrap();
        let work = product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert_eq!(
            (work.layout_measure_nodes, work.layout_placement_nodes),
            (0, 0),
            "value {value}"
        );
        let (track, fill, ..) = progress_geometry(&cx, id);
        assert!((fill.width - track.width * (value as f32 / 100.0)).abs() < 1e-3);
    }
    cx.update_component(progress, |view, _| view.indeterminate = true)
        .unwrap();
    let swept = product_frame(&mut cx, document(), viewport(), &mut shaper);
    cx.set_pointer_hover_at(document(), 1, Some(id), Duration::from_millis(16))
        .unwrap();
    let hovered = product_frame(&mut cx, document(), viewport(), &mut shaper);
    for work in [swept, hovered] {
        assert_eq!(
            (work.layout_measure_nodes, work.layout_placement_nodes),
            (0, 0)
        );
    }
    assert_eq!(cx.world().layout_generation(), generation);
    assert!(Arc::ptr_eq(&snapshot, &cx.world().layout_results[&id]));
}

/// A progress bar moved between Grid, Inline and Flex parents keeps its
/// entity and value, and publishes the parts a cold layout in that context
/// would.
#[test]
fn issue384_progress_parts_survive_context_transitions() {
    use crate::MutationQueue;
    use nana_ui_core::DisplaySpec;
    let build = |display: DisplaySpec| {
        let mut cx = AppContext::new();
        let root = cx
            .create_component(
                document(),
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(320.0));
                    layout.display = Some(display);
                }),
            )
            .unwrap();
        let progress = cx
            .create_detached_component(document(), progress_view(COPYING, 40.0))
            .unwrap();
        cx.append_child(root, progress).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
        (cx, root, progress)
    };
    let mut shaper = bundled_face_shaper();
    let (mut cx, root, progress) = build(DisplaySpec::Flex);
    let id = progress.stable_id();
    for display in [DisplaySpec::Grid, DisplaySpec::Inline, DisplaySpec::Flex] {
        let mut style = cx.world().node_style(root.stable_id()).unwrap().clone();
        Arc::make_mut(&mut style.layout).display = Some(display);
        let mut q = MutationQueue::new();
        q.set_style(root.stable_id(), style);
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
        assert_eq!(cx.read(progress, |view| view.value).unwrap(), 40.0);
        let (_, _, label, cancel) = progress_geometry(&cx, id);
        assert_eq!(
            label.map(|label| label.bounds),
            part(&cx, id, LayoutPartKind::Label)
        );
        assert_eq!(cancel, part(&cx, id, LayoutPartKind::TrailingContent));
        let (mut cold, ..) = build(display);
        assert_matches_cold(&mut cx, &mut cold, document());
    }
}

/// 1k and 10k progress bars keep one entity each and publish the content
/// box and two parts, and an idle frame lays nothing out.
#[test]
fn issue384_ten_thousand_progress_bars_keep_one_entity_and_bounded_parts() {
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let root = cx
            .create_component(document(), Stack::column(0.0).width(LengthSpec::Px(320.0)))
            .unwrap();
        let mut children = Vec::with_capacity(count);
        for _ in 0..count {
            children.push(
                cx.create_detached_component(document(), progress_view(COPYING, 40.0))
                    .unwrap()
                    .stable_id(),
            );
        }
        let mut q = crate::MutationQueue::new();
        for &id in &children {
            q.insert(root.stable_id(), id, None);
        }
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for &id in &children {
            let kinds: Vec<_> = cx
                .world()
                .layout_result(id)
                .unwrap()
                .parts
                .iter()
                .map(|part| part.kind)
                .collect();
            assert_eq!(
                kinds,
                [
                    LayoutPartKind::ComponentContent,
                    LayoutPartKind::Label,
                    LayoutPartKind::TrailingContent
                ]
            );
        }
        let generation = cx.world().layout_generation();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
    }
}
