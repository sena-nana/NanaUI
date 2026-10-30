//! Issue #388: the decisions left open by the content-part migration (#205).
//! A checkbox's indicator is the `Indicator` part, as a radio ring's is.
#![cfg(test)]

use super::reflow_oracle::{bundled_face_shaper, product_frame};
use crate::{
    AppContext, ComponentGeometry, DocumentId, LayoutPartKind, LayoutViewport, StableNodeId, Stack,
};
use nana_ui_core::LengthSpec;

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
