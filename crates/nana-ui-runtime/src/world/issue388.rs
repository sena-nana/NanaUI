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

/// The controls whose parts sit on logical sides (#388).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Checkbox,
    Switch(nana_ui_core::SwitchControlPosition),
    Radio,
    Segmented,
    StatusBadge,
    Validation,
    Toast,
    AssembledToast,
    MenuLabel,
    MenuIcon,
}

const CONTROLS: [Control; 11] = [
    Control::Checkbox,
    Control::Switch(nana_ui_core::SwitchControlPosition::Start),
    Control::Switch(nana_ui_core::SwitchControlPosition::End),
    Control::Radio,
    Control::Segmented,
    Control::StatusBadge,
    Control::Validation,
    Control::Toast,
    Control::AssembledToast,
    Control::MenuLabel,
    Control::MenuIcon,
];

fn create_control(cx: &mut AppContext, control: Control) -> StableNodeId {
    let document = document();
    let id = |entity: StableNodeId| entity;
    match control {
        Control::Checkbox => id(cx
            .create_detached_component(document, crate::Checkbox::new("Notifications", true))
            .unwrap()
            .stable_id()),
        Control::Switch(position) => id(cx
            .create_detached_component(
                document,
                crate::Switch::new("Wi-Fi", true)
                    .hint("Connect automatically")
                    .control_position(position),
            )
            .unwrap()
            .stable_id()),
        Control::Radio | Control::Segmented => {
            let chrome = if control == Control::Radio {
                crate::SelectionChrome::Radio
            } else {
                crate::SelectionChrome::Segmented
            };
            let mut view = crate::SegmentedOption::new("Automatic").surface(
                nana_ui_core::ControlSize::Medium,
                chrome,
                false,
            );
            view.icon = Some(nana_ui_core::Icon::File);
            cx.create_detached_component(document, view)
                .unwrap()
                .stable_id()
        }
        Control::StatusBadge => cx
            .create_detached_component(
                document,
                crate::StatusBadge::new("Synced", nana_ui_core::StatusTone::Success),
            )
            .unwrap()
            .stable_id(),
        Control::Validation => cx
            .create_detached_component(
                document,
                crate::ValidationMessage::new(
                    "Name is required",
                    nana_ui_core::ValidationIntent::Danger,
                ),
            )
            .unwrap()
            .stable_id(),
        Control::Toast | Control::AssembledToast => {
            let toast = cx
                .create_detached_component(
                    document,
                    crate::Toast::new("Export complete", crate::ToastTone::Success)
                        .description("Master sent to disk")
                        .dismissible(true),
                )
                .unwrap();
            if control == Control::AssembledToast {
                cx.assemble_toast(toast).unwrap();
            }
            toast.stable_id()
        }
        Control::MenuLabel | Control::MenuIcon => {
            let mut menu = crate::ActionMenu::new();
            menu.popover = if control == Control::MenuLabel {
                menu.popover.trigger("Options")
            } else {
                menu.popover.trigger_icon(nana_ui_core::Icon::Add, "Add")
            };
            let menu = cx
                .create_detached_component(document, menu)
                .unwrap()
                .stable_id();
            let item = cx
                .create_detached_component(document, crate::ActionMenuItem::new("Open"))
                .unwrap()
                .stable_id();
            let mut q = crate::MutationQueue::new();
            q.insert(menu, item, None);
            cx.commit_mutations(q).unwrap();
            menu
        }
    }
}

fn direction_style(
    direction: nana_ui_core::DirSpec,
) -> impl FnOnce(&mut nana_ui_core::LayoutStyle) {
    move |layout| {
        layout.width = Some(LengthSpec::Px(420.0));
        layout.dir = Some(direction);
    }
}

/// Every control in a 420px column laid out in `direction`, laid out once.
fn controls_page(
    direction: nana_ui_core::DirSpec,
) -> (AppContext, StableNodeId, Vec<(Control, StableNodeId)>) {
    let mut cx = AppContext::new();
    let root = cx
        .create_component(
            document(),
            Stack::column(4.0).with_layout(direction_style(direction)),
        )
        .unwrap();
    let controls = CONTROLS
        .iter()
        .map(|&control| {
            let id = create_control(&mut cx, control);
            let mut q = crate::MutationQueue::new();
            q.insert(root.stable_id(), id, None);
            cx.commit_mutations(q).unwrap();
            (control, id)
        })
        .collect();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    (cx, root.stable_id(), controls)
}

/// `rect`, laid out in `from`, reflected across that box's vertical centre
/// line and carried to `to`, a box as large: where the mirror image sits.
fn mirror(
    rect: crate::LayoutBox,
    from: crate::LayoutBox,
    to: crate::LayoutBox,
) -> crate::LayoutBox {
    crate::LayoutBox {
        x: to.x + from.x + from.width - rect.x - rect.width,
        y: to.y + rect.y - from.y,
        ..rect
    }
}

fn assert_close(ours: crate::LayoutBox, theirs: crate::LayoutBox, what: &str) {
    assert!(
        (ours.x - theirs.x).abs() < 1e-3
            && (ours.y - theirs.y).abs() < 1e-3
            && (ours.width - theirs.width).abs() < 1e-3
            && (ours.height - theirs.height).abs() < 1e-3,
        "{what}: {ours:?} != {theirs:?}"
    );
}

/// Each control's parts in `rtl` are its parts in `ltr` reflected across
/// its own box (which the page's column may put elsewhere): the indicator or mark at the inline start, a dismiss target
/// at the inline end, the switch track at the side its position names, and
/// what an assembled toast lays out after its mark with it.
fn assert_mirrored(ltr: &AppContext, rtl: &AppContext, controls: &[(Control, StableNodeId)]) {
    for &(control, id) in controls {
        let frame = |cx: &AppContext| cx.world().layout_box(id).unwrap();
        let (left, right) = (frame(ltr), frame(rtl));
        assert_eq!(
            (left.width, left.height),
            (right.width, right.height),
            "{control:?}"
        );
        let parts = |cx: &AppContext| cx.world().layout_result(id).unwrap().parts.clone();
        let (left_parts, right_parts) = (parts(ltr), parts(rtl));
        let kinds =
            |parts: &[crate::LayoutPart]| parts.iter().map(|part| part.kind).collect::<Vec<_>>();
        assert_eq!(kinds(&left_parts), kinds(&right_parts), "{control:?}");
        let mut content = 0;
        for (ours, theirs) in left_parts.iter().zip(right_parts.iter()) {
            if ours.kind == LayoutPartKind::ChildPlacement {
                continue;
            }
            content += usize::from(!matches!(ours.kind, LayoutPartKind::ComponentContent));
            assert_close(
                theirs.bounds,
                mirror(ours.bounds, left, right),
                &format!("{control:?} {:?}", ours.kind),
            );
        }
        assert!(content > 0, "{control:?} publishes no content part");
        if control == Control::AssembledToast {
            for &child in ltr.world().child_ids(id) {
                let child_box = |cx: &AppContext| cx.world().layout_box(child).unwrap();
                // A child that generates no box (a hidden slot) is nowhere.
                if child_box(ltr).width == 0.0 && child_box(ltr).height == 0.0 {
                    continue;
                }
                assert_close(
                    child_box(rtl),
                    mirror(child_box(ltr), left, right),
                    &format!("{control:?} child {child:?}"),
                );
            }
        }
    }
}

/// Under `direction: rtl` a checkbox's indicator, a switch's track (and its
/// "on" end), a radio ring, a segmented item's icon, a status dot, a toast's
/// mark and dismiss target and a menu trigger's label and glyph sit on
/// their logical sides: every control laid out right to left is the mirror
/// image of itself laid out left to right. Flipping the page's direction
/// either way lays out what a cold layout in that direction does.
#[test]
fn issue388_controls_mirror_their_parts_under_rtl() {
    use super::reflow_oracle::assert_matches_cold;
    use nana_ui_core::DirSpec;
    let (ltr, _, controls) = controls_page(DirSpec::Ltr);
    let (rtl, _, rtl_controls) = controls_page(DirSpec::Rtl);
    assert_eq!(controls, rtl_controls);
    assert_mirrored(&ltr, &rtl, &controls);

    let (mut cx, root, _) = controls_page(DirSpec::Ltr);
    let mut shaper = bundled_face_shaper();
    for direction in [DirSpec::Rtl, DirSpec::Ltr, DirSpec::Rtl] {
        let mut style = cx.world().node_style(root).unwrap().clone();
        direction_style(direction)(Arc::make_mut(&mut style.layout));
        let mut q = crate::MutationQueue::new();
        q.set_style(root, style);
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, ..) = controls_page(direction);
        assert_matches_cold(&mut cx, &mut cold, document());
    }
    assert_mirrored(&ltr, &cx, &controls);
}
