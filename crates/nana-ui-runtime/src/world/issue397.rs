//! Issue #397: a confirm dialog whose copy wraps settles after a theme
//! switch; idle frames after it lay nothing out.
#![cfg(test)]

use super::reflow_oracle::{bundled_face_shaper, product_frame};
use crate::{AppContext, DocumentId, LayoutViewport, StableNodeId, Stack};
use nana_ui_core::LengthSpec;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

const LONG_TITLE: &str = "Connect the plugin to this machine and every window it opens from now on";

const MESSAGE: &str = "The plugin and its settings are removed from every window; the files it \
                       made stay where they are on disk.";

fn confirm_page() -> (AppContext, StableNodeId) {
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document(), Stack::column(0.0).width(LengthSpec::Px(800.0)))
        .unwrap();
    let host = cx
        .create_detached_component(document(), crate::OverlayHost::new())
        .unwrap();
    cx.append_child(root, host).unwrap();
    let confirm = cx
        .create_detached_component(
            document(),
            crate::ConfirmDialog::new("Remove plugin", MESSAGE),
        )
        .unwrap();
    cx.append_child(host, confirm).unwrap();
    cx.assemble_confirm_dialog(confirm).unwrap();
    cx.activate_overlay(host, confirm).unwrap();
    (cx, confirm.stable_id())
}

/// Whether the confirm's copy runs to more than one line (its one-line
/// title stands for a line).
fn copy_wraps(cx: &AppContext, id: StableNodeId) -> bool {
    let shaped = cx.world().nodes.modal_text(id).unwrap();
    shaped.body.unwrap().height > shaped.title.height * 1.5
}

/// Idle frames after a change: no layout work and no new layout output.
fn assert_settled(
    cx: &mut AppContext,
    id: StableNodeId,
    shaper: &mut crate::NanaTextEngineShaper,
    step: &str,
) {
    let generation = cx.world().layout_generation();
    let result = std::sync::Arc::clone(&cx.world().layout_results[&id]);
    for frame in 0..8 {
        let work = product_frame(cx, document(), viewport(), shaper);
        let own = cx.world().frame_counters;
        for counters in [work, own] {
            assert_eq!(
                (
                    counters.layout_measure_nodes,
                    counters.layout_placement_nodes
                ),
                (0, 0),
                "{step}, idle frame {frame}"
            );
        }
        assert_eq!(cx.world().layout_generation(), generation, "{step}");
    }
    assert!(std::sync::Arc::ptr_eq(
        &result,
        &cx.world().layout_results[&id]
    ));
}

/// A confirm whose copy wraps, hovered, edited and then switched to a theme
/// with another dialog recipe, lays out on the frame each change lands and
/// on none of the idle frames after it. The frames that did the work report
/// it; the idle ones report zero rather than the last frame that worked.
#[test]
fn issue397_a_wrapping_confirm_settles_after_a_theme_switch() {
    use crate::Entity;
    use std::time::Duration;
    let mut shaper = bundled_face_shaper();
    let (mut cx, id) = confirm_page();
    product_frame(&mut cx, document(), viewport(), &mut shaper);
    assert!(copy_wraps(&cx, id), "the copy wraps");
    assert_settled(&mut cx, id, &mut shaper, "mounted");
    cx.rebuild_hit_test(document());
    cx.set_pointer_hover_at(document(), 1, Some(id), Duration::from_millis(16))
        .unwrap();
    product_frame(&mut cx, document(), viewport(), &mut shaper);
    assert_settled(&mut cx, id, &mut shaper, "hovered");
    let recipe = nana_ui_core::DialogRecipe {
        header_gap: 20.0,
        ..nana_ui_core::DialogRecipe::DEFAULT
    };
    for (step, title, message, themed) in [
        ("long title", LONG_TITLE, MESSAGE, false),
        ("short copy", "Remove plugin", "Removed.", false),
        ("theme switch", "Remove plugin", MESSAGE, true),
        ("dark theme", "Remove plugin", MESSAGE, true),
    ] {
        cx.update_component(
            Entity::<crate::ConfirmDialog>::from_stable_id(id),
            |view, _| {
                view.title = title.into();
                view.message = message.into();
            },
        )
        .unwrap();
        cx.assemble_confirm_dialog(Entity::from_stable_id(id))
            .unwrap();
        if themed {
            let theme = if step == "dark theme" {
                nana_ui_core::ThemeDefinition::NANA_DARK
            } else {
                nana_ui_core::ThemeDefinition::NANA_LIGHT
            };
            cx.set_theme_definition(&theme.with_dialog(recipe)).unwrap();
        }
        let work = product_frame(&mut cx, document(), viewport(), &mut shaper);
        if step != "dark theme" {
            assert!(work.layout_measure_nodes > 0, "{step} lays out");
        }
        assert_settled(&mut cx, id, &mut shaper, step);
    }
    assert!(copy_wraps(&cx, id), "the copy wraps under the new theme");
}
