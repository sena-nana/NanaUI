use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use nana_ui::runtime::view::{StorePath, button, column, store_with_history, widget};
use nana_ui::runtime::{
    ActionDescriptor, ActionId, ActionMenu, ActionRegistry, AppContext, AppMenuBar,
    CONTEXT_IME_COMPOSING, CONTEXT_MODAL, CONTEXT_TEXT_INPUT, CapturedStroke, CommandPalette,
    CommandPaletteEvent, ContextPredicate, Dialog, DocumentId, FrameworkError, HeadlessInput,
    KeyBinding, KeyContext, KeyModifiers, Keymap, KeymapMatch, KeymapState, MenuActionMap,
    MenuBarActivated, OverlayHost, Stack, TextInput,
};
use nana_ui::{CompositionInput, InputPayload, KeyInput, KeyState};
use nana_ui_core::{Menu, MenuBar, MenuEntry, take_menu_activations};

fn key(name: &'static str, modifiers: KeyModifiers) -> KeyInput {
    KeyInput::named(name, name, KeyState::Pressed, modifiers)
}

#[test]
fn same_action_fires_consistently_across_keymap_menu_action_menu_palette_and_program() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(28701).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let action_id = ActionId::new("  canvas.reset ");
    let canonical = ActionId::new("canvas.reset");

    let mut registry = ActionRegistry::new();
    registry
        .register(
            ActionDescriptor::labeled("  canvas.reset ", "重置画布")
                .category("画布")
                .when(ContextPredicate::always().none_of([CONTEXT_MODAL])),
        )
        .unwrap();
    let hit_counter = Arc::clone(&hits);
    cx.bind_action_handler(" canvas.reset ", move |_| {
        hit_counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();

    let primary = KeyModifiers::primary();
    let keymap = Keymap::new([KeyBinding::new(
        " canvas.reset ",
        CapturedStroke::new("r", primary),
    )]);
    let mut keymap_state = KeymapState::default();
    let mut menu_map = MenuActionMap::new();
    menu_map.bind(42, " canvas.reset ").unwrap();

    let ctx = KeyContext::default();

    // 1) Mount AppMenuBar using the unified MenuActionMap entry.
    let bar_entity = std::cell::Cell::new(None);
    let menu_entry = menu_map
        .menu_entry(&canonical, &registry, &ctx, Some(&keymap))
        .expect("menu entry from registry + keymap");
    let menu_bar_events = Arc::new(Mutex::new(Vec::<u32>::new()));
    let menu_bar_sink = Arc::clone(&menu_bar_events);
    cx.mount_view_root(doc, || {
        let bar = nana_ui::runtime::view::entity_ref::<AppMenuBar>();
        let view = column().children((
            widget(AppMenuBar::new(MenuBar::new([Menu::new(
                "画布",
                [menu_entry.clone()],
            )])))
            .entity_ref(bar)
            .on(move |event: &MenuBarActivated| {
                menu_bar_sink.lock().unwrap().push(event.id);
            }),
            button("占位"),
        ));
        nana_ui::runtime::view::with_refs(view, bar)
    })
    .map(|(_, bar)| bar_entity.set(Some(bar)))
    .unwrap();
    let bar = bar_entity.get().unwrap();

    // Drain any prior menu activations from other tests in the process.
    let _ = take_menu_activations();

    // 1. Keymap path via HeadlessInput -> route_input -> dispatch_routed_key.
    let mut input = HeadlessInput::bind(&mut cx, doc);
    let key_event = input.stamp(InputPayload::Key(key("r", primary)));
    let outcome = cx
        .route_input(&key_event, input.services_mut(), None)
        .unwrap();
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &key_event,
            outcome.disposition(),
            &ctx,
        )
        .unwrap(),
        KeymapMatch::Dispatch(canonical.clone())
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    // 2. AppMenuBar path: F10 -> ArrowDown -> Enter emits MenuBarActivated(42)
    //    and pushes 42 into take_menu_activations(), then dispatches via adapter.
    for key_name in ["F10", "ArrowDown", "Enter"] {
        let ev = input.stamp(InputPayload::Key(key(key_name, KeyModifiers::empty())));
        let routed = cx.route_input(&ev, input.services_mut(), None).unwrap();
        assert!(routed.prevent_default);
    }
    assert_eq!(menu_bar_events.lock().unwrap().as_slice(), &[42]);
    let drained = take_menu_activations();
    assert_eq!(drained, vec![42]);
    let dispatched = cx
        .dispatch_menu_action(&registry, &menu_map, drained[0], &ctx)
        .unwrap();
    assert_eq!(dispatched, canonical);
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // 3. ActionMenu path: build ActionMenuItem from ActionRegistry adapter,
    //    mount inside ActionMenu, activate it, and dispatch through adapter.
    let menu = cx
        .create_component(doc, ActionMenu::new().trigger("画布操作").open(true))
        .unwrap();
    let item_view = registry
        .action_menu_item(&action_id, &ctx, Some(&keymap))
        .unwrap();
    let item = cx.create_detached_component(doc, item_view).unwrap();
    cx.append_child(menu, item).unwrap();
    let activated_action = Arc::new(Mutex::new(None::<ActionId>));
    let activated_sink = Arc::clone(&activated_action);
    let item_action = canonical.clone();
    cx.on(item, move |_, _: &nana_ui::runtime::Activate, _| {
        *activated_sink.lock().unwrap() = Some(item_action.clone());
    })
    .unwrap();
    assert!(cx.activate_action_menu_item(item).unwrap());
    let picked = activated_action.lock().unwrap().take().unwrap();
    cx.dispatch_registered_action(&registry, &picked, &ctx)
        .unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    // 4. CommandPalette path: official palette_items adapter -> confirm -> dispatch_palette_event.
    let items = registry.palette_items("", &ctx, Some(&keymap));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].action, canonical);
    let palette = CommandPalette::new("命令面板", items);
    let event = palette.confirm().expect("palette item selected");
    assert_eq!(
        cx.dispatch_palette_event(&registry, &event, &ctx).unwrap(),
        Some(canonical.clone())
    );
    assert_eq!(hits.load(Ordering::SeqCst), 4);

    // 5. Direct program invocation with untrimmed ActionId.
    cx.dispatch_registered_action(&registry, &action_id, &ctx)
        .unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 5);

    // Disabling in ActionRegistry keeps all four UI entries and program path in lockstep.
    assert!(registry.set_enabled(&action_id, false));
    assert!(!cx.can_dispatch_registered_action(&registry, &action_id, &ctx));

    let disabled_key = input.stamp(InputPayload::Key(key("r", primary)));
    let disabled_outcome = cx
        .route_input(&disabled_key, input.services_mut(), None)
        .unwrap();
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &disabled_key,
            disabled_outcome.disposition(),
            &ctx,
        )
        .unwrap(),
        KeymapMatch::NoMatch
    );

    let mut synced_bar = cx.read(bar, |bar| bar.bar.clone()).unwrap();
    menu_map.sync_menu_bar(&mut synced_bar, &registry, &ctx, Some(&keymap));
    assert!(matches!(
        synced_bar.menus[0].entries[0],
        MenuEntry::Item { enabled: false, .. }
    ));
    assert_eq!(
        cx.dispatch_menu_action(&registry, &menu_map, 42, &ctx),
        Err(FrameworkError::ActionUnavailable(canonical.clone()))
    );

    let disabled_menu_item = registry
        .action_menu_item(&action_id, &ctx, Some(&keymap))
        .unwrap();
    assert!(disabled_menu_item.disabled);
    cx.set_component(item, disabled_menu_item).unwrap();
    assert!(!cx.activate_action_menu_item(item).unwrap());

    assert!(registry.palette_items("", &ctx, Some(&keymap)).is_empty());
    assert_eq!(
        cx.dispatch_palette_event(
            &registry,
            &CommandPaletteEvent::Select(action_id.clone()),
            &ctx,
        ),
        Err(FrameworkError::ActionUnavailable(canonical.clone()))
    );

    assert_eq!(
        cx.dispatch_registered_action(&registry, &action_id, &ctx),
        Err(FrameworkError::ActionUnavailable(canonical.clone()))
    );
    assert_eq!(
        cx.dispatch_registered_action(&registry, &ActionId::new("missing.action"), &ctx),
        Err(FrameworkError::MissingAction(ActionId::new(
            "missing.action"
        )))
    );
    assert_eq!(
        cx.dispatch_registered_action(&registry, &ActionId::new("   "), &ctx),
        Err(FrameworkError::InvalidAction)
    );
    assert_eq!(hits.load(Ordering::SeqCst), 5);
}

#[test]
fn dynamic_undo_redo_and_focus_scope_synchronize_all_surfaces() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(28702).unwrap();
    let history = store_with_history(vec![String::from("节点A")], 16);
    let last_op = Arc::new(Mutex::new(String::from("移动节点")));

    let mut registry = ActionRegistry::new();
    let undo_id = ActionId::new("edit.undo");
    let redo_id = ActionId::new("edit.redo");
    registry
        .register(ActionDescriptor::labeled(undo_id.clone(), "撤销").category("编辑"))
        .unwrap();
    registry
        .register(ActionDescriptor::labeled(redo_id.clone(), "重做").category("编辑"))
        .unwrap();

    assert!(registry.bind_enabled(&undo_id, move |_| history.can_undo()));
    assert!(registry.bind_enabled(&redo_id, move |_| history.can_redo()));

    let op_for_undo = Arc::clone(&last_op);
    assert!(registry.bind_label(&undo_id, move |ctx| {
        let op = op_for_undo.lock().unwrap().clone();
        if ctx.contains("inspector") {
            format!("撤销属性: {op}")
        } else {
            format!("撤销: {op}")
        }
    }));
    let op_for_redo = Arc::clone(&last_op);
    assert!(registry.bind_label(&redo_id, move |ctx| {
        let op = op_for_redo.lock().unwrap().clone();
        if ctx.contains("inspector") {
            format!("重做属性: {op}")
        } else {
            format!("重做: {op}")
        }
    }));

    cx.bind_action_handler(undo_id.clone(), move |_| {
        history.undo();
        Ok(())
    })
    .unwrap();
    cx.bind_action_handler(redo_id.clone(), move |_| {
        history.redo();
        Ok(())
    })
    .unwrap();

    let primary = KeyModifiers::primary();
    let keymap = Keymap::new([
        KeyBinding::new(undo_id.clone(), CapturedStroke::new("z", primary)),
        KeyBinding::new(
            redo_id.clone(),
            CapturedStroke::new(
                "z",
                KeyModifiers {
                    shift: true,
                    ..primary
                },
            ),
        ),
    ]);
    let mut keymap_state = KeymapState::default();
    let mut menu_map = MenuActionMap::new();
    menu_map.bind(1, undo_id.clone()).unwrap();
    menu_map.bind(2, redo_id.clone()).unwrap();

    let mut menu_bar = MenuBar::new([Menu::new(
        "编辑",
        [
            menu_map
                .menu_entry(&undo_id, &registry, &KeyContext::default(), Some(&keymap))
                .unwrap(),
            menu_map
                .menu_entry(&redo_id, &registry, &KeyContext::default(), Some(&keymap))
                .unwrap(),
        ],
    )]);

    let canvas = KeyContext::new(["canvas"]);
    assert!(!registry.is_available(&undo_id, &canvas));
    assert!(!registry.is_available(&redo_id, &canvas));
    assert!(
        registry
            .palette_items("", &canvas, Some(&keymap))
            .is_empty()
    );

    // Mutate history -> undo becomes dynamically enabled with label "撤销: 移动节点".
    history.set(vec![String::from("节点A (已移动)")]);
    assert!(registry.is_available(&undo_id, &canvas));
    assert!(!registry.is_available(&redo_id, &canvas));

    let palette = registry.palette_items("移动节点", &canvas, Some(&keymap));
    assert_eq!(palette.len(), 1);
    assert_eq!(palette[0].action, undo_id);
    assert_eq!(palette[0].label, "撤销: 移动节点");

    let undo_item = registry
        .action_menu_item(&undo_id, &canvas, Some(&keymap))
        .unwrap();
    assert_eq!(undo_item.label.as_ref(), "撤销: 移动节点");
    assert!(!undo_item.disabled);

    menu_map.sync_menu_bar(&mut menu_bar, &registry, &canvas, Some(&keymap));
    assert!(matches!(
        &menu_bar.menus[0].entries[0],
        MenuEntry::Item { label, enabled: true, .. } if label == "撤销: 移动节点"
    ));
    assert!(matches!(
        &menu_bar.menus[0].entries[1],
        MenuEntry::Item { label, enabled: false, .. } if label == "重做: 移动节点"
    ));

    // Trigger undo via Keymap.
    let mut input = HeadlessInput::bind(&mut cx, doc);
    let undo_key = input.stamp(InputPayload::Key(key("z", primary)));
    let routed = cx
        .route_input(&undo_key, input.services_mut(), None)
        .unwrap();
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &undo_key,
            routed.disposition(),
            &canvas,
        )
        .unwrap(),
        KeymapMatch::Dispatch(undo_id.clone())
    );
    assert_eq!(history.get_untracked(), vec![String::from("节点A")]);
    assert!(!registry.is_available(&undo_id, &canvas));
    assert!(registry.is_available(&redo_id, &canvas));

    // Focus scope switch updates dynamic labels without re-registering actions.
    let inspector = KeyContext::new(["inspector"]);
    *last_op.lock().unwrap() = String::from("改色");
    menu_map.sync_menu_bar(&mut menu_bar, &registry, &inspector, Some(&keymap));
    assert!(matches!(
        &menu_bar.menus[0].entries[1],
        MenuEntry::Item { label, enabled: true, .. } if label == "重做属性: 改色"
    ));

    // Trigger redo via MenuActionMap dispatch.
    assert_eq!(
        cx.dispatch_menu_action(&registry, &menu_map, 2, &inspector)
            .unwrap(),
        redo_id
    );
    assert_eq!(
        history.get_untracked(),
        vec![String::from("节点A (已移动)")]
    );
    assert!(registry.is_available(&undo_id, &inspector));
    assert!(!registry.is_available(&redo_id, &inspector));
}

#[test]
fn focus_modal_text_input_ime_and_menu_bar_priority_gate_keymap() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(28703).unwrap();

    let host = cx.create_component(doc, OverlayHost::new()).unwrap();
    let root = cx.create_component(doc, Stack::column(8.0)).unwrap();

    let field = cx.create_component(doc, TextInput::new("")).unwrap();
    cx.append_child(root, field).unwrap();

    let bar = cx
        .create_component(
            doc,
            AppMenuBar::new(MenuBar::new([Menu::new(
                "文件",
                [MenuEntry::item(1, "新建")],
            )])),
        )
        .unwrap();
    cx.append_child(root, bar).unwrap();
    cx.assemble_app_menu_bar(bar).unwrap();

    let dialog = cx
        .create_detached_component(doc, Dialog::new("确认"))
        .unwrap();

    let undo_hits = Arc::new(AtomicUsize::new(0));
    let tool_hits = Arc::new(AtomicUsize::new(0));
    let mut registry = ActionRegistry::new();
    registry
        .register(
            ActionDescriptor::labeled("edit.undo", "撤销")
                .when(ContextPredicate::always().none_of([CONTEXT_MODAL])),
        )
        .unwrap();
    registry
        .register(
            ActionDescriptor::labeled("canvas.tool", "切换工具")
                .when(ContextPredicate::always().none_of([CONTEXT_MODAL, CONTEXT_TEXT_INPUT])),
        )
        .unwrap();

    let undo_counter = Arc::clone(&undo_hits);
    cx.bind_action_handler("edit.undo", move |_| {
        undo_counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    let tool_counter = Arc::clone(&tool_hits);
    cx.bind_action_handler("canvas.tool", move |_| {
        tool_counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();

    let primary = KeyModifiers::primary();
    let keymap = Keymap::new([
        KeyBinding::new("edit.undo", CapturedStroke::new("z", primary)),
        KeyBinding::new(
            "canvas.tool",
            CapturedStroke::new("k", KeyModifiers::empty()),
        )
        .when(ContextPredicate::always().none_of([CONTEXT_MODAL, CONTEXT_TEXT_INPUT])),
    ]);
    let mut keymap_state = KeymapState::default();
    let mut input = HeadlessInput::bind(&mut cx, doc);

    // 1. Modal dialog barrier blocks keymap shortcuts via both `prevent_default` and CONTEXT_MODAL.
    cx.append_child(host, dialog).unwrap();
    assert!(cx.activate_overlay(host, dialog).unwrap());
    let modal_ctx = cx.document_key_context(doc, KeyContext::default());
    assert!(modal_ctx.contains(CONTEXT_MODAL));
    let modal_event = input.stamp(InputPayload::Key(key("k", KeyModifiers::empty())));
    let modal_outcome = cx
        .route_input(&modal_event, input.services_mut(), None)
        .unwrap();
    assert!(modal_outcome.prevent_default);
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &modal_event,
            modal_outcome.disposition(),
            &modal_ctx,
        )
        .unwrap(),
        KeymapMatch::NoMatch
    );
    assert!(cx.dismiss_overlay(host).unwrap());
    assert!(
        !cx.document_key_context(doc, KeyContext::default())
            .contains(CONTEXT_MODAL)
    );

    // 2. Focused TextInput consumes typing and Ctrl/Cmd+Z for its own text history.
    cx.focus_node(doc, field.stable_id()).unwrap();
    let input_ctx = cx.document_key_context(doc, KeyContext::default());
    assert!(input_ctx.contains(CONTEXT_TEXT_INPUT));

    let type_k = input.stamp(InputPayload::Key(key("k", KeyModifiers::empty())));
    let type_k_outcome = cx.route_input(&type_k, input.services_mut(), None).unwrap();
    // Key transition wasn't consumed as a command, so TextInput receives text "k",
    // while `CONTEXT_TEXT_INPUT` still prevents the plain-key canvas shortcut!
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &type_k,
            type_k_outcome.disposition(),
            &input_ctx,
        )
        .unwrap(),
        KeymapMatch::NoMatch
    );
    input.text(&mut cx, "k").unwrap();
    assert_eq!(cx.read(field, |f| f.state.value.clone()).unwrap(), "k");

    let editor_undo = input.stamp(InputPayload::Key(key("z", primary)));
    let editor_undo_outcome = cx
        .route_input(&editor_undo, input.services_mut(), None)
        .unwrap();
    assert!(editor_undo_outcome.prevent_default);
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &editor_undo,
            editor_undo_outcome.disposition(),
            &input_ctx,
        )
        .unwrap(),
        KeymapMatch::NoMatch
    );
    assert_eq!(cx.read(field, |f| f.state.value.clone()).unwrap(), "");
    assert_eq!(undo_hits.load(Ordering::SeqCst), 0);

    // 3. IME composition consumes key transitions before keymap and populates CONTEXT_IME_COMPOSING.
    input.composition(&mut cx, CompositionInput::Start).unwrap();
    input
        .composition(
            &mut cx,
            CompositionInput::Update {
                text: "ni".into(),
                selection: Some((2, 2)),
            },
        )
        .unwrap();
    let ime_ctx = cx.document_key_context(doc, KeyContext::default());
    assert!(ime_ctx.contains(CONTEXT_IME_COMPOSING));
    let ime_key = input.stamp(InputPayload::Key(key("z", primary)));
    let ime_outcome = cx
        .route_input(&ime_key, input.services_mut(), None)
        .unwrap();
    assert!(ime_outcome.prevent_default);
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &ime_key,
            ime_outcome.disposition(),
            &ime_ctx,
        )
        .unwrap(),
        KeymapMatch::NoMatch
    );
    input.composition(&mut cx, CompositionInput::End).unwrap();
    cx.clear_focus(doc).unwrap();

    // 4. Open AppMenuBar captures keys before global keymap.
    for open_key in ["F10", "ArrowDown"] {
        let ev = input.stamp(InputPayload::Key(key(open_key, KeyModifiers::empty())));
        assert!(
            cx.route_input(&ev, input.services_mut(), None)
                .unwrap()
                .prevent_default
        );
    }
    let menu_ctx = cx.document_key_context(doc, KeyContext::default());
    let blocked_by_menu = input.stamp(InputPayload::Key(key("k", KeyModifiers::empty())));
    let menu_outcome = cx
        .route_input(&blocked_by_menu, input.services_mut(), None)
        .unwrap();
    assert!(menu_outcome.prevent_default);
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &blocked_by_menu,
            menu_outcome.disposition(),
            &menu_ctx,
        )
        .unwrap(),
        KeymapMatch::NoMatch
    );
    for close_key in ["Escape", "Escape"] {
        let ev = input.stamp(InputPayload::Key(key(close_key, KeyModifiers::empty())));
        assert!(
            cx.route_input(&ev, input.services_mut(), None)
                .unwrap()
                .prevent_default
        );
    }

    // 5. Unblocked document dispatches both plain and primary shortcuts through the keymap.
    let clear_ctx = cx.document_key_context(doc, KeyContext::default());
    assert!(!clear_ctx.contains(CONTEXT_MODAL));
    assert!(!clear_ctx.contains(CONTEXT_TEXT_INPUT));
    assert!(!clear_ctx.contains(CONTEXT_IME_COMPOSING));

    let tool_ev = input.stamp(InputPayload::Key(key("k", KeyModifiers::empty())));
    let tool_outcome = cx
        .route_input(&tool_ev, input.services_mut(), None)
        .unwrap();
    assert!(!tool_outcome.prevent_default);
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &tool_ev,
            tool_outcome.disposition(),
            &clear_ctx,
        )
        .unwrap(),
        KeymapMatch::Dispatch(ActionId::new("canvas.tool"))
    );

    let undo_ev = input.stamp(InputPayload::Key(key("z", primary)));
    let undo_outcome = cx
        .route_input(&undo_ev, input.services_mut(), None)
        .unwrap();
    assert!(!undo_outcome.prevent_default);
    assert_eq!(
        cx.dispatch_routed_key(
            &keymap,
            &mut keymap_state,
            &registry,
            &undo_ev,
            undo_outcome.disposition(),
            &clear_ctx,
        )
        .unwrap(),
        KeymapMatch::Dispatch(ActionId::new("edit.undo"))
    );
    assert_eq!(tool_hits.load(Ordering::SeqCst), 1);
    assert_eq!(undo_hits.load(Ordering::SeqCst), 1);
}
