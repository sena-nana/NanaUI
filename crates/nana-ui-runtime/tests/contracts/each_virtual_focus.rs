//! Column regrouping keeps focus on the same keyed item and descendant.
use std::sync::{Arc, Mutex};

use nana_ui_core::{LengthSpec, SelectionGesture, SelectionState, SpacingStep, ThemeDefinition};
use nana_ui_input::{
    InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey, PointerInput,
    PointerPhase,
};
use nana_ui_runtime::view::{
    Class, IntoView, PSEUDO_SELECTED, Sheet, SheetRule, StylePatch, each_virtual, list_item,
    signal, theme_signal, virtual_list_ref, widget,
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
    cx.rebuild_hit_test(document);
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

fn pointer(phase: PointerPhase, x: f32, y: f32, modifiers: InputModifiers) -> InputPayload {
    InputPayload::Pointer(PointerInput {
        modifiers,
        ..PointerInput::mouse(phase, x, y)
    })
}

fn key_payload(name: &'static str, modifiers: InputModifiers) -> InputPayload {
    InputPayload::Key(KeyInput {
        physical: PhysicalKey(name.into()),
        logical: LogicalKey(name.into()),
        state: KeyState::Pressed,
        repeat: false,
        modifiers,
    })
}

fn mounted_item(
    cx: &AppContext,
    rows: &nana_ui_runtime::view::VirtualListRef<u32>,
    key: u32,
) -> Option<nana_ui_runtime::StableNodeId> {
    let slot = rows.row(&key)?;
    cx.world().node(slot)?.children.first().copied()
}

#[test]
fn each_virtual_selection_preserves_offscreen_keys_and_handles_reconcile_and_reveal_focus() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let rows = virtual_list_ref::<u32>();
    let sel = signal(SelectionState::<u32>::multiple());
    let items_sig = signal((0..1000u32).collect::<Vec<_>>());

    let rows_for_mount = rows.clone();
    let rows_for_select = rows.clone();
    let _mounted = cx
        .mount_view_root(document, move || {
            each_virtual(
                items_sig,
                |item| *item,
                24.0,
                move |item| {
                    list_item(format!("row {item}")).selected(move || sel.get().is_selected(&item))
                },
            )
            .multiselectable(true)
            .list_ref(rows_for_mount.clone())
            .on_select(move |request| {
                let order = rows_for_select.keys();
                let mut next = sel.get();
                match (request.gesture, request.step) {
                    (SelectionGesture::Move(modifiers), Some(step)) => {
                        if let Some(target) = next.navigate(step, modifiers, &order) {
                            rows_for_select.reveal_and_focus(&target);
                        }
                    }
                    (gesture, _) => {
                        next.apply(gesture, request.key, &order);
                    }
                }
                sel.set(next);
            })
            .width(300.0)
            .height(200.0)
        })
        .unwrap();
    frames(&mut cx, document);

    let mut input = HeadlessInput::bind(&mut cx, document);
    let ctrl = InputModifiers {
        control: true,
        ..InputModifiers::empty()
    };
    let shift = InputModifiers {
        shift: true,
        ..InputModifiers::empty()
    };
    let center = |cx: &AppContext, key: u32| {
        let id = mounted_item(cx, &rows, key).unwrap();
        let b = cx.world().presentation_input_bounds(id).unwrap();
        (b.x + b.width / 2.0, b.y + b.height / 2.0)
    };
    let click = |cx: &mut AppContext, input: &mut HeadlessInput, key: u32, mods| {
        let (x, y) = center(cx, key);
        input
            .route(cx, pointer(PointerPhase::Down, x, y, mods))
            .unwrap();
        input
            .route(cx, pointer(PointerPhase::Up, x, y, mods))
            .unwrap();
        frames(cx, document);
    };

    click(&mut cx, &mut input, 1, InputModifiers::empty());
    click(&mut cx, &mut input, 4, shift);
    assert_eq!(sel.get().selected_in(&rows.keys()), [1, 2, 3, 4]);

    // Scroll far down so 1..=4 unmount: offscreen keys remain selected.
    rows.scroll_to(&800, nana_ui_core::VirtualAlignment::Start);
    frames(&mut cx, document);
    assert!(rows.row(&1).is_none());
    click(&mut cx, &mut input, 802, ctrl);
    assert_eq!(sel.get().selected_in(&rows.keys()), [1, 2, 3, 4, 802]);

    // Reconcile drops removed keys (2, 802) and moves current to nearest surviving key (803).
    items_sig.set(
        (0..1000u32)
            .filter(|k| *k != 2 && *k != 802)
            .collect::<Vec<_>>(),
    );
    frames(&mut cx, document);
    {
        let mut next = sel.get();
        assert!(next.reconcile(&rows.keys()));
        assert_eq!(next.current(), Some(&804));
        sel.set(next);
    }
    frames(&mut cx, document);
    assert_eq!(sel.get().selected_in(&rows.keys()), [1, 3, 4]);

    // Focus visible item 801, press End -> selects & reveals/focuses 999.
    let target_801 = mounted_item(&cx, &rows, 801).unwrap();
    assert!(cx.focus_node_in_place(document, target_801).unwrap());
    input
        .route(&mut cx, key_payload("End", InputModifiers::empty()))
        .unwrap();
    frames(&mut cx, document);
    assert_eq!(sel.get().selected_in(&rows.keys()), [999]);
    let focus_999 = mounted_item(&cx, &rows, 999).expect("999 revealed");
    assert_eq!(cx.world().focused(document), Some(focus_999));

    // Ctrl+Shift+ArrowUp extends from anchor 999 -> [998, 999], while Ctrl+click preserves prior.
    rows.scroll_to(&1, Default::default());
    frames(&mut cx, document);
    click(&mut cx, &mut input, 3, InputModifiers::empty());
    rows.scroll_to(&999, Default::default());
    frames(&mut cx, document);
    click(&mut cx, &mut input, 999, ctrl);
    let focus_999 = mounted_item(&cx, &rows, 999).unwrap();
    assert_eq!(cx.world().focused(document), Some(focus_999));
    let ctrl_shift = InputModifiers {
        control: true,
        shift: true,
        ..InputModifiers::empty()
    };
    input
        .route(&mut cx, key_payload("ArrowUp", ctrl_shift))
        .unwrap();
    frames(&mut cx, document);
    assert_eq!(sel.get().selected_in(&rows.keys()), [3, 998, 999]);
}

static GRID_RESTING: StylePatch = StylePatch::new(|layout| {
    layout.opacity = Some(1.0);
});
static GRID_SELECTED: StylePatch = StylePatch::new(|layout| {
    layout.opacity = Some(0.5);
});
static GRID_RULES: [SheetRule; 2] = [
    SheetRule {
        classes: &[0],
        pseudo: 0,
        patch: &GRID_RESTING,
        query: None,
    },
    SheetRule {
        classes: &[0],
        pseudo: PSEUDO_SELECTED,
        patch: &GRID_SELECTED,
        query: None,
    },
];
static GRID_SHEET: Sheet = Sheet::new(&GRID_RULES, &[]);

#[test]
fn each_virtual_grid_selection_supports_2d_arrows_and_custom_cards_with_selected_pseudo() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let rows = virtual_list_ref::<u32>();
    let sel = signal(SelectionState::<u32>::multiple());
    let card_class = Class::new(&GRID_SHEET, 0);

    let rows_for_mount = rows.clone();
    let rows_for_select = rows.clone();
    let _mounted = cx
        .mount_view_root(document, move || {
            each_virtual(
                signal((0..40u32).collect::<Vec<_>>()),
                |item| *item,
                60.0,
                move |item| {
                    widget(Stack::column(0.0).height(LengthSpec::Px(60.0)))
                        .hittable()
                        .focusable()
                        .aria_selected(move || Some(sel.get().is_selected(&item)))
                        .class(card_class)
                },
            )
            .grid(100.0, 0.0)
            .multiselectable(true)
            .list_ref(rows_for_mount.clone())
            .on_select(move |request| {
                let order = rows_for_select.keys();
                let mut next = sel.get();
                match (request.gesture, request.step) {
                    (SelectionGesture::Move(modifiers), Some(step)) => {
                        if let Some(target) = next.navigate(step, modifiers, &order) {
                            rows_for_select.reveal_and_focus(&target);
                        }
                    }
                    (gesture, _) => {
                        next.apply(gesture, request.key, &order);
                    }
                }
                sel.set(next);
            })
            .width(300.0)
            .height(200.0)
        })
        .unwrap();
    frames(&mut cx, document);
    assert_eq!(rows.columns(), 3);

    let mut input = HeadlessInput::bind(&mut cx, document);
    let card_1 = mounted_item(&cx, &rows, 1).unwrap();
    let b1 = cx.world().presentation_input_bounds(card_1).unwrap();
    input
        .route(
            &mut cx,
            pointer(
                PointerPhase::Down,
                b1.x + b1.width / 2.0,
                b1.y + b1.height / 2.0,
                InputModifiers::empty(),
            ),
        )
        .unwrap();
    frames(&mut cx, document);
    assert_eq!(sel.get().selected_in(&rows.keys()), [1]);
    assert_eq!(
        cx.world().node_style(card_1).unwrap().layout.opacity,
        Some(0.5)
    );

    assert_eq!(cx.world().focused(document), Some(card_1));
    for (k, expected_key) in [
        ("ArrowRight", 2u32),
        ("ArrowDown", 5u32),
        ("ArrowLeft", 4u32),
        ("ArrowUp", 1u32),
    ] {
        input
            .route(&mut cx, key_payload(k, InputModifiers::empty()))
            .unwrap();
        frames(&mut cx, document);
        assert_eq!(sel.get().selected_in(&rows.keys()), [expected_key]);
        let node = mounted_item(&cx, &rows, expected_key).unwrap();
        assert_eq!(cx.world().focused(document), Some(node));
        assert_eq!(
            cx.world().node_style(node).unwrap().layout.opacity,
            Some(0.5)
        );
    }
}
