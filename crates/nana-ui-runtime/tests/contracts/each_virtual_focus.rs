//! Column regrouping keeps focus on the same keyed item and descendant.
use std::sync::{Arc, Mutex};

use nana_ui_core::{LengthSpec, SpacingStep, ThemeDefinition};
use nana_ui_input::{InputModifiers, KeyInput, KeyState};
use nana_ui_runtime::view::{
    IntoView, each_virtual, signal, theme_signal, virtual_list_ref, widget,
};
use nana_ui_runtime::{
    Activate, AppContext, Button, DocumentId, HeadlessInput, LayoutViewport, Stack,
};

fn frames(cx: &mut AppContext, document: DocumentId) {
    for _ in 0..8 {
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.layout_document(document, LayoutViewport::new(600.0, 400.0))
            .unwrap();
    }
}

#[test]
fn theme_column_regroup_restores_the_same_item_control_and_keyboard_action() {
    for measured in [false, true] {
        for focused_key in [2, 20] {
            let mut cx = AppContext::new();
            let document = DocumentId::new(1).unwrap();
            let rows = virtual_list_ref::<u32>();
            let activated = Arc::new(Mutex::new(Vec::new()));
            let actions = Arc::clone(&activated);
            let _mounted = cx
                .mount_view_root(document, || {
                    let list = each_virtual(
                        signal((0..60).collect::<Vec<_>>()),
                        |item| *item,
                        80.0,
                        move |item| {
                            let actions = Arc::clone(&actions);
                            let primary = widget(Button::new("primary")).key("primary").into_any();
                            let secondary = widget(Button::new("secondary"))
                                .key("secondary")
                                .on(move |_: &Activate| actions.lock().unwrap().push(item))
                                .into_any();
                            // A rebuilt responsive card may change sibling order;
                            // explicit keys still name the same action.
                            let controls =
                                if theme_signal().with(|theme| theme.spacing().xxxl >= 48.0) {
                                    vec![secondary, primary]
                                } else {
                                    vec![primary, secondary]
                                };
                            widget(Stack::column(0.0).height(LengthSpec::Px(80.0)))
                                .key("card")
                                .children(controls)
                        },
                    )
                    .grid_theme(120.0, SpacingStep::Xxxl)
                    .list_ref(rows.clone())
                    .width(400.0)
                    .height(200.0);
                    if measured { list.measured() } else { list }
                })
                .unwrap();
            frames(&mut cx, document);
            rows.scroll_to(&focused_key, Default::default());
            frames(&mut cx, document);
            let target = cx
                .resolve_assembly_path(rows.row(&focused_key).unwrap(), "card/secondary")
                .unwrap();
            assert!(cx.focus_node_in_place(document, target).unwrap());
            for gap in [48.0, 16.0, 64.0] {
                let mut theme = ThemeDefinition::NANA_DARK;
                theme.tokens.spacing.xxxl = gap;
                cx.set_theme_definition(&theme.bump()).unwrap();
                frames(&mut cx, document);
                let current = cx
                    .resolve_assembly_path(rows.row(&focused_key).unwrap(), "card/secondary")
                    .unwrap();
                assert_eq!(
                    cx.world().focused(document),
                    Some(current),
                    "measured={measured}, key={focused_key}, gap={gap}"
                );
                let focused_bounds = cx.world().layout_box(current).unwrap();
                let first = cx
                    .world()
                    .layout_box(rows.row(&focused_key).unwrap())
                    .unwrap();
                assert!(
                    focused_bounds.y >= first.y
                        && focused_bounds.y + focused_bounds.height <= first.y + first.height
                );
                let mut input = HeadlessInput::bind(&mut cx, document);
                input
                    .press(
                        &mut cx,
                        KeyInput::named(
                            "Enter",
                            "Enter",
                            KeyState::Pressed,
                            InputModifiers::default(),
                        ),
                        None,
                        None,
                    )
                    .unwrap();
                assert_eq!(activated.lock().unwrap().last(), Some(&focused_key));
            }
            assert_eq!(*activated.lock().unwrap(), vec![focused_key; 3]);
        }
    }
}

#[test]
fn removing_a_focused_key_while_regrouping_does_not_focus_its_replacement() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let rows = virtual_list_ref::<u32>();
    let source = std::cell::Cell::new(None);
    let _mounted = cx
        .mount_view_root(document, || {
            let items = signal((0..12).collect::<Vec<_>>());
            source.set(Some(items));
            each_virtual(
                items,
                |item| *item,
                40.0,
                |item| widget(Button::new(item.to_string())).key("action"),
            )
            .grid_theme(120.0, SpacingStep::Xxxl)
            .list_ref(rows.clone())
            .width(400.0)
            .height(200.0)
        })
        .unwrap();
    frames(&mut cx, document);
    let target = cx
        .resolve_assembly_path(rows.row(&2).unwrap(), "action")
        .unwrap();
    assert!(cx.focus_node_in_place(document, target).unwrap());
    source
        .get()
        .unwrap()
        .set((0..12).filter(|item| *item != 2).collect());
    let mut theme = ThemeDefinition::NANA_DARK;
    theme.tokens.spacing.xxxl = 48.0;
    cx.set_theme_definition(&theme.bump()).unwrap();
    frames(&mut cx, document);
    assert!(rows.item(&2).is_none());
    assert_eq!(cx.world().focused(document), None);
}
