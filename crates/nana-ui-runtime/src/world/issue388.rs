//! Issue #388: the decisions left open by the content-part migration (#205).
//! A checkbox's indicator is the `Indicator` part, as a radio ring's is.
#![cfg(test)]

use super::reflow_oracle::{bundled_face_shaper, product_frame};
use crate::{
    AppContext, ComponentGeometry, DocumentId, LayoutPartKind, LayoutViewport, StableNodeId, Stack,
};
use nana_ui_core::LengthSpec;
use std::sync::Arc;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(600.0, 400.0)
}

fn kinds(cx: &AppContext, id: StableNodeId) -> Vec<LayoutPartKind> {
    cx.world()
        .layout_result(id)
        .unwrap()
        .parts
        .iter()
        .map(|part| part.kind)
        .collect()
}

fn part(cx: &AppContext, id: StableNodeId, kind: LayoutPartKind) -> crate::LayoutBox {
    cx.world()
        .layout_result(id)
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == kind)
        .unwrap_or_else(|| panic!("{kind:?} part of {id:?}"))
        .bounds
}

fn contains(outer: crate::LayoutBox, inner: crate::LayoutBox) -> bool {
    inner.x >= outer.x - 1e-3
        && inner.y >= outer.y - 1e-3
        && inner.x + inner.width <= outer.x + outer.width + 1e-3
        && inner.y + inner.height <= outer.y + outer.height + 1e-3
}

/// A checkbox publishes its indicator as the `Indicator` part, the kind a
/// radio ring publishes, and no `LeadingContent`: paint draws that part,
/// a pointer on it hits the checkbox, and the checkbox's accessible bounds
/// hold it, labelled or not.
#[test]
fn issue388_a_checkbox_indicator_is_the_indicator_part() {
    for label in ["Notifications", ""] {
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document(), Stack::row(0.0).width(LengthSpec::Px(400.0)))
            .unwrap();
        let checkbox = cx
            .create_detached_component(document(), crate::Checkbox::new(label, true))
            .unwrap();
        cx.append_child(root, checkbox).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
        cx.rebuild_hit_test(document());
        let id = checkbox.stable_id();
        let kinds = kinds(&cx, id);
        assert!(kinds.contains(&LayoutPartKind::Indicator), "{kinds:?}");
        assert!(
            !kinds.contains(&LayoutPartKind::LeadingContent),
            "{kinds:?}"
        );
        let indicator = part(&cx, id, LayoutPartKind::Indicator);
        assert!(indicator.width > 0.0 && indicator.height > 0.0);
        let world = cx.world();
        let Some(ComponentGeometry::Checkbox {
            indicator: painted, ..
        }) = world.component_geometry(id)
        else {
            panic!("checkbox geometry")
        };
        assert_eq!(painted, indicator, "{label:?}");
        assert_eq!(
            world.hit_test(
                document(),
                indicator.x + indicator.width / 2.0,
                indicator.y + indicator.height / 2.0
            ),
            Some(id),
            "{label:?}"
        );
        let node = world
            .project_accessibility_nodes(&[id])
            .into_iter()
            .find(|node| node.id == id)
            .unwrap();
        assert!(contains(node.bounds, indicator), "{node:?} {indicator:?}");
    }
}

/// `view` and a button after it in a row that shrink-wraps them (or is
/// `row` wide), laid out once.
fn switch_row(
    view: crate::Switch,
    row: Option<f32>,
) -> (AppContext, crate::Entity<crate::Switch>, StableNodeId) {
    let mut cx = AppContext::new();
    let mut stack =
        Stack::row(8.0).with_layout(|layout| layout.align_items = nana_ui_core::AlignSpec::Start);
    stack = stack.width(row.map_or(LengthSpec::MaxContent, LengthSpec::Px));
    let page = cx
        .create_component(
            document(),
            Stack::column(0.0)
                .with_layout(|layout| layout.align_items = nana_ui_core::AlignSpec::Start),
        )
        .unwrap();
    let root = cx.create_detached_component(document(), stack).unwrap();
    cx.append_child(page, root).unwrap();
    let switch = cx.create_detached_component(document(), view).unwrap();
    cx.append_child(root, switch).unwrap();
    let after = cx
        .create_detached_component(document(), crate::Button::new("After"))
        .unwrap();
    cx.append_child(root, after).unwrap();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    (cx, switch, after.stable_id())
}

/// A switch that shrink-wraps its content (`fit-content`) rather than
/// filling its row, as a labelled switch does by default.
fn labelled_switch(label: &str, hint: Option<&str>) -> crate::Switch {
    let mut view = crate::Switch::new(label, false);
    if let Some(hint) = hint {
        view = view.hint(hint);
    }
    Arc::make_mut(&mut view.style.layout).width = Some(LengthSpec::FitContent);
    view
}

/// A labelled switch that shrink-wraps in a row is as wide as its track,
/// its gap and the wider of its label and hint, so the label part holds the
/// whole label and the next item starts past it; in a row too narrow for
/// that it keeps to the row. Label and hint changes lay out what a cold
/// layout does.
#[test]
fn issue388_a_switch_label_counts_toward_its_width() {
    use super::reflow_oracle::assert_matches_cold;
    let short = "Wi-Fi";
    let long = "Connect automatically to known networks";
    for (label, hint) in [(short, None), (long, None), (short, Some(long))] {
        let (cx, switch, after) = switch_row(labelled_switch(label, hint), None);
        let id = switch.stable_id();
        let world = cx.world();
        let inputs = world.switch_inputs(id).unwrap();
        let measure = world.chrome_text_measure(id);
        let typography = world.theme().typography();
        let size = nana_ui_core::ControlSize::Medium;
        let label_width = measure.width(
            label,
            size.text_size_in(typography),
            Some(typography.medium),
        );
        let text = hint.map_or(label_width, |hint| {
            label_width.max(measure.width(
                hint,
                size.caption_size_in(typography),
                Some(typography.regular),
            ))
        });
        assert!(text > 0.0);
        let result = world.layout_result(id).unwrap();
        let content = result.content_box;
        let expected = inputs.track.width + inputs.gap + text;
        assert!(
            (content.width - expected).abs() < 0.01,
            "{label:?}: {} != {expected}",
            content.width
        );
        let label_part = part(&cx, id, LayoutPartKind::Label);
        assert!(label_part.width >= label_width - 0.01, "{label_part:?}");
        let border = world.layout_box(id).unwrap();
        assert!(world.layout_box(after).unwrap().x >= border.x + border.width);
    }

    // A row narrower than the label keeps the switch inside it.
    let (cx, switch, _) = switch_row(labelled_switch(long, None), Some(160.0));
    let border = cx.world().layout_box(switch.stable_id()).unwrap();
    assert!(border.x + border.width <= 160.0 + 0.01, "{border:?}");

    // The label and hint moving the width relay the row out as a cold
    // layout of the same switch does.
    let mut shaper = bundled_face_shaper();
    let (mut cx, switch, _) = switch_row(labelled_switch(short, None), None);
    for (label, hint) in [
        (long, None),
        (short, Some(long)),
        (short, Some(short)),
        ("", None),
    ] {
        cx.update_component(switch, |view, _| *view = labelled_switch(label, hint))
            .unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, cold_switch, _) = switch_row(labelled_switch(label, hint), None);
        assert_eq!(cold_switch.stable_id(), switch.stable_id());
        assert_matches_cold(&mut cx, &mut cold, document());
    }
}
