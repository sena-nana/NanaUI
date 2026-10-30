//! Issue #398: after a theme switch a modal's text colours are the ones a
//! cold layout under that theme resolves.
#![cfg(test)]

use super::reflow_oracle::{bundled_face_shaper, product_frame};
use crate::{AppContext, ComponentGeometry, DocumentId, LayoutViewport, StableNodeId, Stack};
use nana_ui_core::{LengthSpec, ThemeDefinition};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

#[derive(Clone, Copy, Debug)]
enum Modal {
    Dialog,
    Confirm { danger: bool },
}

fn page(modal: Modal, theme: Option<&ThemeDefinition>) -> (AppContext, StableNodeId) {
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(document(), Stack::column(0.0).width(LengthSpec::Px(800.0)))
        .unwrap();
    let host = cx
        .create_detached_component(document(), crate::OverlayHost::new())
        .unwrap();
    cx.append_child(root, host).unwrap();
    let id = match modal {
        Modal::Dialog => {
            let dialog = cx
                .create_detached_component(
                    document(),
                    crate::Dialog::new("Settings").description("Changes apply to every window"),
                )
                .unwrap();
            cx.append_child(host, dialog).unwrap();
            cx.activate_overlay(host, dialog).unwrap();
            dialog.stable_id()
        }
        Modal::Confirm { danger } => {
            let confirm = cx
                .create_detached_component(
                    document(),
                    crate::ConfirmDialog::new("Remove plugin", "The files stay on disk.")
                        .danger(danger),
                )
                .unwrap();
            cx.append_child(host, confirm).unwrap();
            cx.assemble_confirm_dialog(confirm).unwrap();
            cx.activate_overlay(host, confirm).unwrap();
            confirm.stable_id()
        }
    };
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    (cx, id)
}

/// The colours of a modal's title, description and body copy, as the
/// geometry query reports them and as extraction paints them.
fn text_colors(cx: &AppContext, id: StableNodeId) -> [[Option<[f32; 4]>; 3]; 2] {
    let regions = |geometry: &ComponentGeometry| match geometry {
        ComponentGeometry::ModalFrame {
            title,
            description,
            body_text,
            ..
        } => [
            title.color,
            description.as_ref().and_then(|region| region.color),
            body_text.as_ref().and_then(|region| region.color),
        ],
        other => panic!("{other:?}"),
    };
    let queried = regions(&cx.world().component_geometry(id).unwrap());
    let painted = cx.world().extract_nodes(&[id]);
    [
        queried,
        regions(painted[0].component_geometry.as_deref().unwrap()),
    ]
}

/// A dialog, a confirm and a danger confirm, switched from the default
/// (dark) theme to light, to a light theme with another dialog recipe and
/// back to dark, report and paint the title, description and body colours
/// a cold layout under each theme does: the text colour of the theme
/// installed, or for a danger confirm's title its danger tone.
#[test]
fn issue398_modal_text_colours_follow_a_theme_switch_like_a_cold_layout() {
    let recipe = ThemeDefinition::NANA_LIGHT.with_dialog(nana_ui_core::DialogRecipe {
        header_gap: 20.0,
        ..nana_ui_core::DialogRecipe::DEFAULT
    });
    for modal in [
        Modal::Dialog,
        Modal::Confirm { danger: false },
        Modal::Confirm { danger: true },
    ] {
        let mut shaper = bundled_face_shaper();
        let (mut cx, id) = page(modal, None);
        let theme_text = |cx: &AppContext| {
            cx.world()
                .theme()
                .style_model()
                .color(nana_ui_core::SemanticColorRole::Text)
                .as_rgba_array()
        };
        let mut previous = (theme_text(&cx), text_colors(&cx, id));
        for theme in [
            ThemeDefinition::NANA_LIGHT,
            recipe.clone(),
            ThemeDefinition::NANA_DARK,
        ] {
            cx.set_theme_definition(&theme).unwrap();
            product_frame(&mut cx, document(), viewport(), &mut shaper);
            let (cold, cold_id) = page(modal, Some(&theme));
            let colors = text_colors(&cx, id);
            let expected = text_colors(&cold, cold_id);
            assert_eq!(colors, expected, "{modal:?} under {}", theme.id.as_str());
            assert_eq!(colors[0], colors[1], "the query reports what is painted");
            let text = theme_text(&cx);
            if !matches!(modal, Modal::Confirm { danger: true }) {
                assert_eq!(colors[0][0], Some(text), "{modal:?}");
            }
            if matches!(modal, Modal::Confirm { .. }) {
                assert_eq!(colors[0][2], Some(text), "{modal:?}");
            }
            if previous.0 != text {
                // The text colour moved, so the colours did too: the checks
                // above are not passing on a colour that never changed.
                assert_ne!(previous.1, colors, "{modal:?}");
            }
            previous = (text, colors);
        }
    }
}
