use super::*;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::{
    Activate, AnimationId, AnimationSpec, Button, Card, Checkbox, Easing, IconButton, List,
    ListItem, NodeStyle, RangeChanged, RangeField, ScrollAxes, ScrollChanged, ScrollView,
    SegmentedControl, SegmentedOption, SegmentedSelectionRequested, Stack, StandardVisual, Switch,
    TabOption, Table, TableCell, TableCellFocused, TableNavigation, TableRow, Tabs, Text, TextArea,
    TextChanged, TextContent, TextInput, TextSelection, ToggleChanged,
};

#[derive(Debug)]
struct Counter {
    value: usize,
}

struct Increment(usize);
struct Cascade;

#[test]
fn bind_component_requires_an_existing_node_then_enables_read() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let id = StableNodeId::new(7).unwrap();
    let button = Button::new("Go");
    assert_eq!(
        context.bind_component(id, button.clone()),
        Err(FrameworkError::MissingView(id))
    );
    let mut queue = MutationQueue::new();
    queue.create(id, document, button.node_kind());
    context.commit_mutations(queue).unwrap();
    let entity = context.bind_component(id, button).unwrap();
    assert_eq!(entity.stable_id(), id);
    assert_eq!(
        context
            .read(entity, |button| button.label.to_string())
            .unwrap(),
        "Go"
    );
    assert_eq!(context.world().text(id), Some("Go"));
}

#[test]
fn mount_reuses_keyed_entities_and_drops_unused() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let card = context.create_component(document, Card::new()).unwrap();
    let mut title = None;
    let mut save = None;
    context
        .mount(card, |ui| {
            title = Some(ui.child("title", Text::new("Nana"))?);
            save = Some(ui.child("save", Button::new("Save"))?);
            Ok(())
        })
        .unwrap();
    let title = title.unwrap();
    let save = save.unwrap();
    let title_id = title.stable_id();
    let save_id = save.stable_id();
    context
        .mount(card, |ui| {
            ui.child("title", Text::new("Nana"))?;
            ui.child("save", Button::new("Saved"))?;
            Ok(())
        })
        .unwrap();
    assert_eq!(title.stable_id(), title_id);
    assert_eq!(save.stable_id(), save_id);
    assert_eq!(
        context.read(save, |button| button.label.clone()).unwrap(),
        "Saved"
    );
    context
        .mount(card, |ui| {
            ui.child("title", Text::new("Nana"))?;
            Ok(())
        })
        .unwrap();
    assert!(context.world().contains(title_id));
    assert!(!context.world().contains(save_id));
}

#[test]
fn build_commits_nested_tree_once_and_installs_handlers() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let before = context.world().generation();
    let start = context
        .build(document, |ui| {
            ui.column(12.0, |ui| {
                ui.child("title", Text::new("你好"));
                let start = ui.child("start", Button::new("开始"));
                ui.on(start, |_, _: &Activate, cx| {
                    cx.dispatch_program("start");
                });
                ui.row(8.0, |ui| {
                    ui.child("open", Button::new("打开"));
                    ui.child("float", Button::new("浮窗"));
                });
                start
            })
        })
        .unwrap();
    assert_eq!(context.world().generation(), before + 1);

    let start_node = context.world().node(start.stable_id()).unwrap();
    let column = start_node.parent.expect("start lives under column");
    let column_children = context.world().node(column).unwrap().children;
    assert_eq!(column_children.len(), 3);
    assert_eq!(context.world().text(column_children[0]), Some("你好"));
    assert_eq!(column_children[1], start.stable_id());
    let row = column_children[2];
    let row_children = context.world().node(row).unwrap().children;
    assert_eq!(row_children.len(), 2);
    assert_eq!(
        context.read(start, |button| button.label.clone()).unwrap(),
        "开始"
    );
    assert!(context.activate_button(start).unwrap());
    let queued = context.take_program_messages();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].downcast_ref::<&str>().copied(), Some("start"));
}

#[test]
fn create_component_append_commits_once_per_call() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let before = context.world().generation();
    let column = context
        .create_component(document, Stack::column(12.0))
        .unwrap();
    let title = context
        .create_component(document, Text::new("你好"))
        .unwrap();
    let start = context
        .create_component(document, Button::new("开始"))
        .unwrap();
    context.append_child(column, title).unwrap();
    context.append_child(column, start).unwrap();
    assert_eq!(context.world().generation(), before + 5);
}

#[test]
fn build_rejects_duplicate_keys_without_committing() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let before = context.world().generation();
    let error = context
        .build(document, |ui| {
            ui.column(8.0, |ui| {
                ui.child("save", Button::new("Save"));
                ui.child("save", Button::new("Saved"));
            });
        })
        .unwrap_err();
    assert!(matches!(error, FrameworkError::DuplicateAssemblyKey { .. }));
    assert_eq!(context.world().generation(), before);
    assert!(
        context
            .world()
            .node(StableNodeId::new(1).unwrap())
            .is_none()
    );
}

#[test]
fn build_detached_parks_roots_until_inserted() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let (root, child) = context
        .build_detached(document, |ui| {
            let child = ui.leaf(Text::new("parked"));
            let root = ui.child("root", Stack::column(8.0));
            ui.nest(root, |ui| ui.adopt(child));
            (root, child)
        })
        .unwrap();
    assert_eq!(
        context.world().mount_state(root.stable_id()),
        Some(crate::MountState::Parked)
    );
    assert_eq!(
        context.world().mount_state(child.stable_id()),
        Some(crate::MountState::Parked)
    );
    let host = context
        .create_component(document, Stack::column(0.0))
        .unwrap();
    context.append_child(host, root).unwrap();
    assert_eq!(
        context.world().mount_state(root.stable_id()),
        Some(crate::MountState::Mounted)
    );
    assert_eq!(
        context.world().mount_state(child.stable_id()),
        Some(crate::MountState::Mounted)
    );
    assert_eq!(
        context.world().node(root.stable_id()).unwrap().children,
        vec![child.stable_id()]
    );
}

#[test]
fn build_child_keys_are_reused_by_mount() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let card = context.create_component(document, Card::new()).unwrap();
    let save = context
        .build_child(card, |ui| ui.child("save", Button::new("Save")))
        .unwrap();
    let save_id = save.stable_id();
    context
        .mount(card, |ui| {
            ui.child("save", Button::new("Saved"))?;
            Ok(())
        })
        .unwrap();
    assert_eq!(save.stable_id(), save_id);
    assert_eq!(
        context.read(save, |button| button.label.clone()).unwrap(),
        "Saved"
    );
}

#[test]
fn sidebar_row_activate_queues_a_program_message() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let row = context
        .create_component(document, SidebarRow::new("舞台"))
        .unwrap();
    context
        .on(row, |_row, _event: &Activate, cx| {
            cx.dispatch_program("stage");
        })
        .unwrap();
    assert!(context.activate_sidebar_row(row).unwrap());
    let queued = context.take_program_messages();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].downcast_ref::<&str>().copied(), Some("stage"));
    assert!(context.take_program_messages().is_empty());
}

#[test]
fn dispatch_program_keeps_the_latest_message_of_each_type() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let row = context
        .create_component(document, SidebarRow::new("舞台"))
        .unwrap();
    context
        .on(row, |_row, _event: &Activate, cx| {
            cx.dispatch_program("stage");
            cx.dispatch_program("functions");
            cx.dispatch_program(1_u8);
        })
        .unwrap();
    assert!(context.activate_sidebar_row(row).unwrap());
    assert!(context.has_program_messages());
    let queued = context.take_program_messages();
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].downcast_ref::<&str>().copied(), Some("functions"));
    assert_eq!(queued[1].downcast_ref::<u8>().copied(), Some(1));
    assert!(!context.has_program_messages());
}

#[test]
fn plugin_register_activation_reaches_activate_node() {
    #[derive(Clone)]
    struct Ping;
    impl ComponentView for Ping {
        fn node_kind(&self) -> NodeKind {
            NodeKind::Element { tag: "ping".into() }
        }
        fn project(&self, _id: StableNodeId, _world: &UiWorld, _mutations: &mut MutationQueue) {}
    }
    impl crate::RegisterableComponent for Ping {
        const TYPE_ID: &'static str = "test.ping";
        const TAGS: &'static [&'static str] = &["ping"];
        fn from_semantic(_: &crate::SemanticSpec<'_>) -> Self {
            Ping
        }
    }
    fn activate_ping(
        context: &mut AppContext,
        entity: Entity<Ping>,
    ) -> Result<bool, FrameworkError> {
        context.update_component(entity, |_, cx| cx.emit(Activate))?;
        Ok(true)
    }
    struct PingExt;
    impl UiExtension for PingExt {
        fn name(&self) -> &'static str {
            "test.ping"
        }
        fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
            registrar.register_component::<Ping>()?;
            registrar.register_activation::<Ping>(activate_ping)
        }
    }

    let mut context = AppContext::new();
    context.install(&PingExt).unwrap();
    let ping = context
        .create_component(DocumentId::new(1).unwrap(), Ping)
        .unwrap();
    let hits = Arc::new(Mutex::new(0));
    let observed = Arc::clone(&hits);
    context
        .on(ping, move |_, _: &Activate, _| {
            *observed.lock().unwrap() += 1;
        })
        .unwrap();
    assert!(context.activate_node(ping.stable_id()).unwrap());
    assert_eq!(*hits.lock().unwrap(), 1);
}

#[test]
fn typed_view_update_delivers_closure_events_and_commits_one_batch() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let entity = context
        .create_view(document, NodeKind::Text, Counter { value: 0 })
        .unwrap();
    context
        .on(entity, |view, event: &Increment, cx| {
            view.value += event.0;
            let id = cx.entity().stable_id();
            cx.mutations().set_text(
                id,
                TextContent {
                    value: view.value.to_string(),
                },
            );
            cx.emit(Cascade);
        })
        .unwrap();
    context
        .on(entity, |view, _event: &Cascade, _cx| view.value += 1)
        .unwrap();

    context
        .update(entity, |_view, cx| cx.emit(Increment(2)))
        .unwrap();
    assert_eq!(context.read(entity, |view| view.value).unwrap(), 3);
    assert_eq!(context.world().generation(), 2);
    assert!(
        context
            .world_mut()
            .take_system_work()
            .text
            .contains(&entity.stable_id())
    );
}

#[test]
fn forged_view_type_is_an_error_and_does_not_remove_state() {
    let mut context = AppContext::new();
    let entity = context
        .create_view(
            DocumentId::new(1).unwrap(),
            NodeKind::Document,
            Counter { value: 7 },
        )
        .unwrap();
    let wrong = Entity::<String>::from_stable_id(entity.stable_id());
    assert_eq!(
        context.update(wrong, |_, _| ()),
        Err(FrameworkError::ViewType(entity.stable_id()))
    );
    assert_eq!(context.read(entity, |view| view.value).unwrap(), 7);
}

#[test]
fn native_components_project_final_event_state_into_one_retained_tree() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let list = context
        .create_component(document, List::new().label("Actions"))
        .unwrap();
    let button = context
        .create_component(document, Button::new("Build"))
        .unwrap();
    let input = context
        .create_component(document, TextInput::new("你好ab").label("Name"))
        .unwrap();
    context.append_child(list, button).unwrap();
    context.append_child(list, input).unwrap();
    context
        .on(button, |button, _event: &Activate, _cx| {
            button.label = "Running".into();
        })
        .unwrap();
    let observed_change = Arc::new(Mutex::new(None));
    let observer = Arc::clone(&observed_change);
    context
        .on(input, move |_input, event: &TextChanged, _cx| {
            *observer.lock().unwrap() = Some(event.clone());
        })
        .unwrap();

    assert!(context.activate_button(button).unwrap());
    context
        .update_component(input, |input, _cx| {
            input.state.selection = TextSelection {
                anchor: 0,
                focus: "你".len(),
            };
        })
        .unwrap();
    assert!(context.replace_text_input_selection(input, "娜").unwrap());

    assert_eq!(context.world().text(button.stable_id()), Some("Running"));
    assert_eq!(
        context.world().text_input(input.stable_id()).unwrap().value,
        "娜好ab"
    );
    assert_eq!(context.world().text(input.stable_id()), Some("娜好ab"));
    assert_eq!(
        observed_change.lock().unwrap().as_ref().unwrap().selection,
        TextSelection::caret("娜".len())
    );
    assert_eq!(
        context.world().node(list.stable_id()).unwrap().children,
        vec![button.stable_id(), input.stable_id()]
    );
    let accessibility = context.world().project_accessibility(document);
    assert_eq!(accessibility[0].role, crate::AccessibilityRole::List);
    assert_eq!(accessibility[1].role, crate::AccessibilityRole::Button);
    assert_eq!(accessibility[1].label.as_deref(), Some("Running"));
    assert_eq!(accessibility[2].role, crate::AccessibilityRole::TextInput);
    assert_eq!(accessibility[2].value.as_deref(), Some("娜好ab"));
    let extracted_button = context
        .world()
        .extract_document(document)
        .into_iter()
        .find(|node| node.id == button.stable_id())
        .unwrap();
    assert_eq!(extracted_button.text.unwrap().value, "Running");

    let generation = context.world().generation();
    context.update_component(button, |_button, _cx| {}).unwrap();
    assert_eq!(context.world().generation(), generation);
}

#[test]
fn loading_button_owns_size_semantics_animation_and_activation_gate() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let button = context
        .create_component(
            document,
            Button::new("Deploy")
                .kind(nana_ui_core::ButtonKind::Warning)
                .size(nana_ui_core::ControlSize::Large)
                .loading(true)
                .invalid(true),
        )
        .unwrap();

    assert!(!context.activate_button(button).unwrap());
    assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));
    assert_eq!(
        context
            .world()
            .node_style(button.stable_id())
            .unwrap()
            .layout
            .min_height,
        Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::ControlSize::Large.height()
        ))
    );
    let accessibility = context.world().accessibility(button.stable_id()).unwrap();
    assert!(accessibility.disabled);
    assert!(accessibility.busy);
    assert!(accessibility.invalid);
    assert!(matches!(
        context.world().standard_visual(button.stable_id()),
        Some(StandardVisual::Button {
            kind: nana_ui_core::ButtonKind::Warning,
            size: nana_ui_core::ControlSize::Large,
            loading: true,
            invalid: true,
            ..
        })
    ));

    let frame = context.advance_animations(Duration::from_millis(400));
    assert_eq!(frame.component_updates, vec![button.stable_id()]);
    assert_eq!(frame.next_deadline, Some(Duration::from_millis(416)));
    assert!(matches!(
        context.world().standard_visual(button.stable_id()),
        Some(StandardVisual::Button {
            loading_phase,
            ..
        }) if (loading_phase - 0.5).abs() < f32::EPSILON
    ));

    context
        .update_component(button, |button, _cx| button.loading = false)
        .unwrap();
    assert_eq!(context.next_animation_deadline(), None);
    assert!(context.activate_button(button).unwrap());
}

#[test]
fn text_input_owns_editability_privacy_size_and_busy_semantics() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let input = context
        .create_component(
            document,
            TextInput::new("secret")
                .placeholder("Password")
                .size(nana_ui_core::ControlSize::Large)
                .read_only(true)
                .secure(true)
                .invalid(true),
        )
        .unwrap();

    assert!(context.focus_node(document, input.stable_id()).unwrap());
    assert!(!context.replace_text_input_selection(input, "x").unwrap());
    assert!(
        !context
            .set_ime_preedit(document, "输入".into(), None)
            .unwrap()
    );
    let node = context
        .world()
        .project_accessibility(document)
        .into_iter()
        .find(|node| node.id == input.stable_id())
        .unwrap();
    assert!(node.focused);
    assert!(!node.editable);
    assert!(node.invalid);
    assert_eq!(node.value, None);
    assert_eq!(
        context
            .world()
            .node_style(input.stable_id())
            .unwrap()
            .layout
            .min_height,
        Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::ControlSize::Large.height()
        ))
    );

    context
        .update_component(input, |input, _cx| {
            input.read_only = false;
            input.loading = true;
        })
        .unwrap();
    let state = context.world().accessibility(input.stable_id()).unwrap();
    assert!(state.disabled);
    assert!(state.busy);
    assert!(!state.editable);
    assert!(!context.replace_text_input_selection(input, "x").unwrap());

    context
        .update_component(input, |input, _cx| input.loading = false)
        .unwrap();
    assert_eq!(context.world().focused(document), Some(input.stable_id()));
    assert!(
        context
            .set_ime_preedit(document, "输入".into(), None)
            .unwrap()
    );
}

#[test]
fn text_input_placeholder_uses_layout_color_and_opacity() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let mut field = TextInput::new("").placeholder("Hint");
    {
        let layout = Arc::make_mut(&mut field.style.layout);
        layout.placeholder_color = Some([1.0, 0.0, 0.0, 1.0]);
        layout.placeholder_opacity = Some(0.5);
    }
    let input = context.create_component(document, field).unwrap();
    let mut mutations = MutationQueue::new();
    mutations.write_layout(
        input.stable_id(),
        crate::LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 32.0,
        },
    );
    context.commit_mutations(mutations).unwrap();
    context
        .world_mut()
        .resolve_styles(&[input.stable_id()])
        .unwrap();
    context
        .world_mut()
        .shape_text(&[input.stable_id()], &mut crate::MeasureTextShaper)
        .unwrap();

    match context.world().component_geometry(input.stable_id()) {
        Some(crate::ComponentGeometry::TextInput { text, .. }) => {
            assert_eq!(text.color, Some([1.0, 0.0, 0.0, 0.5]));
        }
        other => panic!("expected text input geometry, got {other:?}"),
    }
}

#[test]
fn card_icon_button_and_list_item_keep_visual_and_semantic_content_distinct() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let card = context
        .create_component(document, Card::new().label("Build actions"))
        .unwrap();
    let icon = context
        .create_component(
            document,
            IconButton::new(nana_ui_core::Icon::Add, "Add source"),
        )
        .unwrap();
    let item = context
        .create_component(document, ListItem::new("Camera").selected(true))
        .unwrap();
    context.append_child(card, icon).unwrap();
    context.append_child(card, item).unwrap();
    context
        .on(icon, |button, _event: &Activate, _cx| {
            button.selected = true;
        })
        .unwrap();
    context
        .on(item, |item, _event: &Activate, _cx| {
            item.selected = false;
        })
        .unwrap();

    assert!(context.activate_icon_button(icon).unwrap());
    assert!(context.activate_list_item(item).unwrap());
    assert_eq!(context.world().text(icon.stable_id()), Some(""));
    assert_eq!(
        context.world().standard_visual(icon.stable_id()),
        Some(StandardVisual::Icon {
            icon: nana_ui_core::Icon::Add,
            size: nana_ui_core::ControlSize::Medium.icon_size(),
            tooltip: None,
        })
    );
    assert_eq!(context.world().text(item.stable_id()), Some("Camera"));

    let nodes = context.world().project_accessibility(document);
    let icon_node = nodes
        .iter()
        .find(|node| node.id == icon.stable_id())
        .unwrap();
    assert_eq!(icon_node.role, crate::AccessibilityRole::Button);
    assert_eq!(icon_node.label.as_deref(), Some("Add source"));
    assert_eq!(
        context
            .world()
            .node_style(icon.stable_id())
            .unwrap()
            .layout
            .min_width,
        Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.icon_button_size
        ))
    );
    let item_node = nodes
        .iter()
        .find(|node| node.id == item.stable_id())
        .unwrap();
    assert_eq!(item_node.role, crate::AccessibilityRole::ListItem);
    assert_eq!(item_node.selected, Some(false));
    assert_eq!(
        context
            .world()
            .node_style(card.stable_id())
            .unwrap()
            .layout
            .padding_top,
        Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.panel_padding_y + 24.0
        ))
    );
    assert_eq!(
        context.world().node(card.stable_id()).unwrap().children,
        vec![icon.stable_id(), item.stable_id()]
    );
}

#[test]
fn text_area_reuses_utf8_editing_and_projects_multiline_semantics() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let area = context
        .create_component(document, TextArea::new("第一行\nsecond").label("Notes"))
        .unwrap();
    context
        .update_component(area, |area, _cx| {
            area.state.selection = TextSelection {
                anchor: "第一".len(),
                focus: "第一行\n".len(),
            };
        })
        .unwrap();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&changes);
    context
        .on(area, move |_area, event: &TextChanged, _cx| {
            observed.lock().unwrap().push(event.clone());
        })
        .unwrap();

    assert!(context.replace_text_area_selection(area, "段落\n").unwrap());
    assert_eq!(
        context.world().text_input(area.stable_id()).unwrap().value,
        "第一段落\nsecond"
    );
    assert_eq!(changes.lock().unwrap().len(), 1);
    let accessibility = context.world().project_accessibility(document);
    assert_eq!(accessibility[0].role, crate::AccessibilityRole::TextInput);
    assert_eq!(accessibility[0].label.as_deref(), Some("Notes"));
    assert!(accessibility[0].multiline);
    assert_eq!(accessibility[0].value.as_deref(), Some("第一段落\nsecond"));
}

#[test]
fn text_area_projects_git_gutter_marks_into_the_visual() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let area = context
        .create_component(
            document,
            TextArea::new("a\nb").git_gutter(Arc::from([
                crate::TextGitMark::new(1, crate::TextGitMarkKind::Added),
                crate::TextGitMark::new(2, crate::TextGitMarkKind::Deleted),
            ])),
        )
        .unwrap();

    let Some(StandardVisual::TextInput { git_marks, .. }) =
        context.world().standard_visual(area.stable_id())
    else {
        panic!("expected text input visual");
    };
    assert_eq!(
        git_marks.as_ref(),
        &[
            crate::TextGitMark::new(1, crate::TextGitMarkKind::Added),
            crate::TextGitMark::new(2, crate::TextGitMarkKind::Deleted),
        ]
    );
}

#[test]
fn text_area_projects_visual_state_and_deletes_a_whole_grapheme() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let emoji = "👩‍💻";
    let area = context
        .create_component(
            document,
            TextArea::new(format!("{emoji}\n界"))
                .placeholder("Write notes")
                .invalid(true)
                .height(144.0)
                .scroll_offset(ScrollOffset { x: 4.0, y: 12.0 }),
        )
        .unwrap();

    assert!(matches!(
        context.world().standard_visual(area.stable_id()),
        Some(StandardVisual::TextInput {
            placeholder,
            secure: false,
            invalid: true,
            ..
        }) if placeholder.as_ref() == "Write notes"
    ));
    assert_eq!(
        context.world().node_style(area.stable_id()).unwrap().border,
        Some(nana_ui_core::SemanticColorRole::Danger)
    );
    assert_eq!(
        context.world().scroll_offset(area.stable_id()),
        Some(ScrollOffset { x: 4.0, y: 12.0 })
    );
    assert_eq!(
        context
            .world()
            .node_style(area.stable_id())
            .unwrap()
            .layout
            .height,
        Some(LengthSpec::Px(144.0))
    );

    context
        .update_component(area, |area, _cx| {
            area.state.selection = TextSelection::caret(emoji.len());
        })
        .unwrap();
    assert!(context.focus_node(document, area.stable_id()).unwrap());
    assert!(context.delete_focused_text_backward(document).unwrap());
    let state = context.world().text_input(area.stable_id()).unwrap();
    assert_eq!(state.value, "\n界");
    assert_eq!(state.selection, TextSelection::caret(0));

    assert!(
        context
            .set_ime_preedit(document, "输入".into(), None)
            .unwrap()
    );
    assert!(context.commit_ime(document, "输入").unwrap());
    let state = context.world().text_input(area.stable_id()).unwrap();
    assert_eq!(state.value, "输入\n界");
    assert_eq!(state.selection, TextSelection::caret("输入".len()));
    assert_eq!(context.world().ime(area.stable_id()), None);

    context
        .update_component(area, |area, _cx| area.disabled = true)
        .unwrap();
    assert_eq!(context.world().focused(document), None);
    assert_eq!(context.world().ime(area.stable_id()), None);
    let accessibility = context.world().accessibility(area.stable_id()).unwrap();
    assert!(accessibility.disabled);
    assert!(accessibility.invalid);
    assert!(accessibility.multiline);
}

#[test]
fn native_table_projects_hierarchy_text_and_accessibility_roles() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let table = context
        .create_component(document, Table::new().label("Builds"))
        .unwrap();
    let row = context
        .create_component(document, TableRow::new().selected(true))
        .unwrap();
    let header = context
        .create_component(document, TableCell::new("Status").column_header(true))
        .unwrap();
    let cell = context
        .create_component(document, TableCell::new("Running").selected(true))
        .unwrap();
    context.append_child(table, row).unwrap();
    context.append_child(row, header).unwrap();
    context.append_child(row, cell).unwrap();
    let focused_events = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::clone(&focused_events);
    context
        .on(table, move |_table, event: &TableCellFocused, _cx| {
            events.lock().unwrap().push(event.clone());
        })
        .unwrap();

    assert!(
        context
            .navigate_table(table, TableNavigation::NextRow, 10)
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(header.stable_id()));
    assert!(
        context
            .navigate_table(table, TableNavigation::NextColumn, 10)
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(cell.stable_id()));
    assert_eq!(
        focused_events.lock().unwrap().last().unwrap(),
        &TableCellFocused {
            row: 0,
            column: 1,
            cell: cell.stable_id(),
        }
    );

    let accessibility = context.world().project_accessibility(document);
    assert_eq!(accessibility[0].role, crate::AccessibilityRole::Table);
    assert_eq!(accessibility[0].label.as_deref(), Some("Builds"));
    assert_eq!(accessibility[1].role, crate::AccessibilityRole::Row);
    assert_eq!(accessibility[1].selected, Some(true));
    assert_eq!(
        accessibility[2].role,
        crate::AccessibilityRole::ColumnHeader
    );
    assert_eq!(accessibility[3].role, crate::AccessibilityRole::Cell);
    assert_eq!(accessibility[3].label.as_deref(), Some("Running"));
    assert_eq!(context.world().text(cell.stable_id()), Some("Running"));
}

#[test]
fn native_toggle_and_slider_state_share_events_visuals_and_accessibility() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let checkbox = context
        .create_component(
            document,
            Checkbox::new("Notifications", false).invalid(true),
        )
        .unwrap();
    let switch = context
        .create_component(document, Switch::new("Auto build", true))
        .unwrap();
    let slider = context
        .create_component(
            document,
            RangeField::new(25.0, 0.0, 100.0, 1.0)
                .unwrap()
                .label("Volume"),
        )
        .unwrap();
    let toggles = Arc::new(Mutex::new(Vec::new()));
    let checkbox_events = Arc::clone(&toggles);
    context
        .on(checkbox, move |_checkbox, event: &ToggleChanged, _cx| {
            checkbox_events.lock().unwrap().push(event.checked);
        })
        .unwrap();
    let slider_values = Arc::new(Mutex::new(Vec::new()));
    let values = Arc::clone(&slider_values);
    context
        .on(slider, move |_slider, event: &RangeChanged, _cx| {
            values.lock().unwrap().push(event.value);
        })
        .unwrap();

    assert!(context.toggle_checkbox(checkbox).unwrap());
    assert!(context.toggle_switch(switch).unwrap());
    assert!(context.set_range_value(slider, 150.0).unwrap());
    assert!(!context.set_range_value(slider, 100.0).unwrap());
    assert_eq!(
        context.set_range_value(slider, f64::NAN),
        Err(FrameworkError::InvalidComponentValue(slider.stable_id()))
    );

    assert_eq!(*toggles.lock().unwrap(), vec![true]);
    assert_eq!(*slider_values.lock().unwrap(), vec![100.0]);
    assert_eq!(
        context.world().standard_visual(checkbox.stable_id()),
        Some(StandardVisual::Checkbox {
            checked: true,
            indeterminate: false,
            size: nana_ui_core::ControlSize::Medium,
        })
    );
    assert_eq!(
        context
            .world()
            .node_style(checkbox.stable_id())
            .unwrap()
            .layout
            .min_height,
        Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::ControlSize::Medium.height()
        ))
    );
    assert_eq!(
        context.world().standard_visual(switch.stable_id()),
        Some(StandardVisual::Switch {
            thumb_progress: 1.0,
            label: Arc::from("Auto build"),
            hint: None,
            checked: false,
            control_position: nana_ui_core::SwitchControlPosition::End,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        })
    );
    assert_eq!(
        context.world().standard_visual(slider.stable_id()),
        Some(StandardVisual::Range {
            label: Some(Arc::from("Volume")),
            value: Arc::from("100"),
            unit: None,
            size: nana_ui_core::ControlSize::Medium,
            ratio: 1.0,
            invalid: false,
        })
    );
    let accessibility = context.world().project_accessibility(document);
    assert_eq!(accessibility[0].role, crate::AccessibilityRole::Checkbox);
    assert_eq!(accessibility[0].checked, Some(true));
    assert!(accessibility[0].invalid);
    assert_eq!(accessibility[1].role, crate::AccessibilityRole::Switch);
    assert_eq!(accessibility[1].checked, Some(false));
    assert_eq!(accessibility[2].role, crate::AccessibilityRole::Slider);
    assert_eq!(accessibility[2].value.as_deref(), Some("100"));

    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    let checkbox_paint = context
        .world()
        .extract_nodes(&[checkbox.stable_id()])
        .pop()
        .unwrap();
    assert_eq!(
        checkbox_paint.style.background,
        Some(nana_ui_core::SemanticPalette::dark().accent.as_rgba_array())
    );
    assert_eq!(
        checkbox_paint.style.border_color,
        Some(nana_ui_core::SemanticPalette::dark().danger.as_rgba_array())
    );
    context
        .world_mut()
        .set_pointer_hover(document, 1, Some(checkbox.stable_id()))
        .unwrap();
    context.advance_animations(nana_ui_core::motion::HOVER_COLOR);
    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    let hovered_checked = context
        .world()
        .extract_nodes(&[checkbox.stable_id()])
        .pop()
        .unwrap();
    assert_ne!(
        hovered_checked.style.background, checkbox_paint.style.background,
        "a selected toggle must expose a distinct hover state"
    );
}

#[test]
fn an_indeterminate_checkbox_reads_mixed_and_paints_as_engaged() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let mixed = context
        .create_component(
            document,
            Checkbox::new("Notifications", false)
                .indeterminate(true)
                .size(nana_ui_core::ControlSize::Large),
        )
        .unwrap();
    assert_eq!(
        context.world().standard_visual(mixed.stable_id()),
        Some(StandardVisual::Checkbox {
            checked: false,
            indeterminate: true,
            size: nana_ui_core::ControlSize::Large,
        })
    );
    assert_eq!(
        context
            .world()
            .node_style(mixed.stable_id())
            .unwrap()
            .layout
            .min_height,
        Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::ControlSize::Large.height()
        ))
    );
    let accessibility = context.world().project_accessibility(document);
    assert_eq!(accessibility[0].checked, Some(false));
    assert!(
        accessibility[0].mixed,
        "a mixed checkbox must not read as merely unchecked"
    );

    // Mixed shares the engaged surface with checked, so a parent checkbox
    // is not mistaken for an empty one.
    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    let paint = context
        .world()
        .extract_nodes(&[mixed.stable_id()])
        .pop()
        .unwrap();
    assert_eq!(
        paint.style.background,
        Some(nana_ui_core::SemanticPalette::dark().accent.as_rgba_array())
    );
}

#[test]
fn a_divider_is_an_inert_hairline_with_separator_semantics() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let horizontal = context
        .create_component(document, crate::Divider::horizontal())
        .unwrap();
    let vertical = context
        .create_component(
            document,
            crate::Divider::vertical().thickness(2.0).inset(8.0),
        )
        .unwrap();

    let layout = |entity: StableNodeId| {
        context
            .world()
            .node_style(entity)
            .map(|style| Arc::clone(&style.layout))
            .unwrap()
    };
    let horizontal_layout = layout(horizontal.stable_id());
    assert_eq!(
        horizontal_layout.width,
        Some(nana_ui_core::LengthSpec::Fill)
    );
    assert_eq!(
        horizontal_layout.height,
        Some(nana_ui_core::LengthSpec::Px(1.0))
    );
    let vertical_layout = layout(vertical.stable_id());
    assert_eq!(
        vertical_layout.width,
        Some(nana_ui_core::LengthSpec::Px(2.0))
    );
    assert_eq!(vertical_layout.height, Some(nana_ui_core::LengthSpec::Fill));
    assert_eq!(
        vertical_layout.margin_top,
        Some(nana_ui_core::LengthSpec::Px(8.0))
    );

    for divider in [horizontal.stable_id(), vertical.stable_id()] {
        let interaction = context.world().interaction(divider).unwrap();
        assert!(!interaction.pointer_events);
        assert!(!interaction.focusable);
        assert_eq!(
            context.world().accessibility(divider).map(|s| s.role),
            Some(crate::AccessibilityRole::Separator)
        );
    }
    assert_eq!(
        context
            .world()
            .accessibility(vertical.stable_id())
            .and_then(|state| state.orientation),
        Some(crate::SelectionOrientation::Vertical)
    );
}

#[test]
fn a_number_input_steps_snaps_and_settles_its_draft() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let input = context
        .create_component(
            document,
            crate::NumberInput::new(1.0)
                .range(0.0, 2.0)
                .step(0.5)
                .precision(1)
                .label("Scale"),
        )
        .unwrap();
    let values = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&values);
    context
        .on(input, move |_input, event: &crate::NumberChanged, _cx| {
            observed.lock().unwrap().push(event.value);
        })
        .unwrap();

    assert_eq!(
        context.world().text_input(input.stable_id()).unwrap().value,
        "1.0"
    );
    assert!(context.step_number_input(input, 1).unwrap());
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 1.5);
    assert!(context.step_number_input(input, 2).unwrap());
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 2.0);
    // Already at the maximum: no event, no phantom change.
    assert!(!context.step_number_input(input, 1).unwrap());

    // A draft is only adopted on commit, and it snaps to the step grid.
    context
        .update_component(input, |input, _| {
            input.state.replace_value("0.7".to_owned());
        })
        .unwrap();
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 2.0);
    assert!(context.commit_number_input(input).unwrap());
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 0.5);

    // Nonsense restores the committed value instead of inventing one.
    context
        .update_component(input, |input, _| {
            input.state.replace_value("banana".to_owned());
        })
        .unwrap();
    assert!(context.commit_number_input(input).unwrap());
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 0.5);
    assert_eq!(
        context.world().text_input(input.stable_id()).unwrap().value,
        "0.5"
    );

    assert_eq!(*values.lock().unwrap(), vec![1.5, 2.0, 0.5]);
    let accessibility = context.world().project_accessibility(document);
    assert_eq!(accessibility[0].role, crate::AccessibilityRole::TextInput);
    assert_eq!(accessibility[0].numeric_value, Some(0.5));
    assert_eq!(accessibility[0].numeric_minimum, Some(0.0));
    assert_eq!(accessibility[0].numeric_maximum, Some(2.0));
    assert_eq!(accessibility[0].numeric_step, Some(0.5));
}

#[test]
fn a_disabled_number_input_refuses_both_steppers() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let input = context
        .create_component(
            document,
            crate::NumberInput::new(4.0).range(0.0, 10.0).disabled(true),
        )
        .unwrap();
    assert!(!context.step_number_input(input, 1).unwrap());
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 4.0);
    let read_only = context
        .create_component(
            document,
            crate::NumberInput::new(4.0)
                .range(0.0, 10.0)
                .read_only(true),
        )
        .unwrap();
    assert!(!context.step_number_input(read_only, 1).unwrap());
}

#[test]
fn pressing_the_spinner_steps_and_pressing_the_text_does_not() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let input = context
        .create_component(
            document,
            crate::NumberInput::new(4.0).range(0.0, 10.0).step(1.0),
        )
        .unwrap();
    let mut mutations = MutationQueue::new();
    mutations.write_layout(
        input.stable_id(),
        crate::LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 32.0,
        },
    );
    context.commit_mutations(mutations).unwrap();
    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    context
        .world_mut()
        .shape_text(&work.text, &mut crate::MeasureTextShaper)
        .unwrap();

    let steppers = match context.world().component_geometry(input.stable_id()) {
        Some(crate::ComponentGeometry::TextInput {
            steppers: Some(steppers),
            ..
        }) => steppers,
        other => panic!("expected spinner geometry, got {other:?}"),
    };
    let point = |bounds: crate::LayoutBox| {
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    };
    let (up_x, up_y) = point(steppers.increment);
    let (down_x, down_y) = point(steppers.decrement);
    assert_eq!(
        context.number_stepper_at(input.stable_id(), up_x, up_y),
        Some(1)
    );
    assert_eq!(
        context.number_stepper_at(input.stable_id(), down_x, down_y),
        Some(-1)
    );
    // The editable text area is not a stepper, so caret placement still wins.
    assert_eq!(
        context.number_stepper_at(input.stable_id(), 8.0, 16.0),
        None
    );

    assert!(
        context
            .press_number_stepper(input.stable_id(), up_x, up_y)
            .unwrap()
    );
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 5.0);
    assert!(
        context
            .press_number_stepper(input.stable_id(), down_x, down_y)
            .unwrap()
    );
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 4.0);
    assert!(
        !context
            .press_number_stepper(input.stable_id(), 8.0, 16.0)
            .unwrap()
    );
}

#[test]
fn moving_focus_away_settles_a_pending_numeric_draft() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let input = context
        .create_component(
            document,
            crate::NumberInput::new(1.0).range(0.0, 9.0).step(1.0),
        )
        .unwrap();
    let elsewhere = context
        .create_component(document, Button::new("Done"))
        .unwrap();
    assert!(context.focus_node(document, input.stable_id()).unwrap());
    context
        .update_component(input, |input, _| {
            input.state.replace_value("7".to_owned());
        })
        .unwrap();
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 1.0);

    assert!(context.focus_node(document, elsewhere.stable_id()).unwrap());
    assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 7.0);
}

#[test]
fn range_accessibility_set_value_uses_quantized_typed_action() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let range = context
        .create_component(
            document,
            RangeField::new(0.25, 0.0, 1.0, 0.25)
                .unwrap()
                .label("Opacity")
                .unit("%"),
        )
        .unwrap();
    let values = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&values);
    context
        .on(range, move |_range, event: &RangeChanged, _cx| {
            observed.lock().unwrap().push(event.value);
        })
        .unwrap();

    assert!(
        context
            .apply_accessibility_action(
                document,
                AccessibilityActionRequest {
                    target: range.stable_id(),
                    action: AccessibilityAction::SetValue("0.62".into()),
                },
            )
            .unwrap()
    );
    assert_eq!(*values.lock().unwrap(), vec![0.5]);
    let node = context.world().project_accessibility(document).remove(0);
    assert_eq!(node.numeric_minimum, Some(0.0));
    assert_eq!(node.numeric_maximum, Some(1.0));
    assert_eq!(node.numeric_step, Some(0.25));
    assert_eq!(node.numeric_value, Some(0.5));
}

#[test]
fn failed_component_projection_keeps_typed_state_and_world_unchanged() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let slider = context
        .create_component(document, RangeField::new(25.0, 0.0, 100.0, 1.0).unwrap())
        .unwrap();
    let generation = context.world().generation();
    let visual = context.world().standard_visual(slider.stable_id());

    assert!(
        context
            .update_component(slider, |slider, _cx| slider.value = f64::NAN)
            .is_err()
    );
    assert_eq!(context.read(slider, |slider| slider.value).unwrap(), 25.0);
    assert_eq!(context.world().generation(), generation);
    assert_eq!(context.world().standard_visual(slider.stable_id()), visual);
}

#[test]
fn overlay_host_switches_exclusive_visibility_and_restores_focus() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let base = context
        .create_component(document, Button::new("Open"))
        .unwrap();
    let host = context
        .create_component(document, OverlayHost::new())
        .unwrap();
    let dialog = context
        .create_component(
            document,
            Dialog::new("Settings").close_policy(nana_ui_core::DialogClosePolicy {
                close_on_outside: false,
                ..nana_ui_core::DialogClosePolicy::default()
            }),
        )
        .unwrap();
    let menu = context
        .create_component(document, crate::ActionMenu::new().open(true))
        .unwrap();
    let menu_item = context
        .create_component(document, crate::ActionMenuItem::new("Build"))
        .unwrap();
    context.append_child(host, dialog).unwrap();
    context.append_child(host, menu).unwrap();
    context.append_child(menu, menu_item).unwrap();
    let mut focus = MutationQueue::new();
    focus.request_focus(document, Some(base.stable_id()));
    context.world_mut().commit(focus).unwrap();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&changes);
    context
        .on(host, move |_host, event: &OverlayChanged, _cx| {
            observed.lock().unwrap().push(event.active);
        })
        .unwrap();
    let activations = Arc::new(Mutex::new(0));
    let observed_activations = Arc::clone(&activations);
    context
        .on(menu_item, move |_item, _event: &Activate, _cx| {
            *observed_activations.lock().unwrap() += 1;
        })
        .unwrap();

    let initial_work = context.world_mut().take_system_work();
    context
        .world_mut()
        .resolve_styles(&initial_work.style)
        .unwrap();
    let initial = context.world().extract_document(document);
    assert!(!initial.iter().any(|node| node.id == dialog.stable_id()));
    assert!(!initial.iter().any(|node| node.id == menu.stable_id()));

    assert!(context.activate_overlay(host, dialog).unwrap());
    let dialog_work = context.world_mut().take_system_work();
    context
        .world_mut()
        .resolve_styles(&dialog_work.style)
        .unwrap();
    assert_eq!(context.world().focused(document), Some(dialog.stable_id()));
    let generation = context.world().generation();
    assert_eq!(
        context.append_child(menu, dialog),
        Err(FrameworkError::World(
            crate::UiWorldError::InvalidOverlayHost(host.stable_id())
        ))
    );
    assert_eq!(context.world().generation(), generation);
    assert_eq!(
        context.world().node(dialog.stable_id()).unwrap().parent,
        Some(host.stable_id())
    );
    assert!(
        context
            .world()
            .project_accessibility(document)
            .iter()
            .any(|node| node.id == dialog.stable_id() && node.modal)
    );
    let mut escape_modal = MutationQueue::new();
    escape_modal.request_focus(document, Some(base.stable_id()));
    assert_eq!(
        context.world_mut().commit(escape_modal),
        Err(crate::UiWorldError::NotFocusable(base.stable_id()))
    );
    assert_eq!(context.world().focused(document), Some(dialog.stable_id()));
    assert!(
        !context
            .dismiss_dialog(host, nana_ui_core::DialogCloseTrigger::Outside)
            .unwrap()
    );
    let mut capture = MutationQueue::new();
    capture.capture_pointer(7, dialog.stable_id());
    context.world_mut().commit(capture).unwrap();
    assert!(context.activate_overlay(host, menu).unwrap());
    let menu_work = context.world_mut().take_system_work();
    assert!(menu_work.accessibility.contains(&host.stable_id()));
    context
        .world_mut()
        .resolve_styles(&menu_work.style)
        .unwrap();
    let visible = context.world().extract_document(document);
    assert!(!visible.iter().any(|node| node.id == dialog.stable_id()));
    assert!(visible.iter().any(|node| node.id == menu.stable_id()));
    assert_eq!(
        context.world().focused(document),
        Some(menu_item.stable_id())
    );
    assert!(context.activate_action_menu_item(menu_item).unwrap());
    assert_eq!(*activations.lock().unwrap(), 1);
    assert_eq!(context.world().pointer_capture(document, 7), None);
    assert!(
        context
            .world_mut()
            .take_pointer_capture_changes()
            .iter()
            .any(|change| change.pointer_id == 7 && !change.captured)
    );

    assert!(!context.dismiss_overlay(host).unwrap());
    assert!(context.active_runtime_overlay(document).is_none());
    context.advance_animations(nana_ui_core::motion::MENU_POP);
    let dismissed_work = context.world_mut().take_system_work();
    assert!(dismissed_work.accessibility.contains(&host.stable_id()));
    context
        .world_mut()
        .resolve_styles(&dismissed_work.style)
        .unwrap();
    assert_eq!(context.world().focused(document), Some(base.stable_id()));
    assert_eq!(
        context
            .world()
            .overlay_host(host.stable_id())
            .unwrap()
            .active,
        None
    );
    assert_eq!(
        changes.lock().unwrap().as_slice(),
        [Some(dialog.stable_id()), Some(menu.stable_id()), None]
    );
}

#[test]
fn destroying_the_active_overlay_clears_authority_and_restores_focus() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let base = context
        .create_component(document, Button::new("Open"))
        .unwrap();
    let host = context
        .create_component(document, OverlayHost::new())
        .unwrap();
    let dialog = context
        .create_component(document, Dialog::new("Temporary"))
        .unwrap();
    context.append_child(host, dialog).unwrap();
    let mut focus = MutationQueue::new();
    focus.request_focus(document, Some(base.stable_id()));
    context.world_mut().commit(focus).unwrap();
    context.activate_overlay(host, dialog).unwrap();

    context.remove_view(dialog).unwrap();

    assert_eq!(context.world().focused(document), Some(base.stable_id()));
    assert_eq!(
        context.world().overlay_host(host.stable_id()),
        Some(crate::OverlayHostState::default())
    );
    assert!(!context.dismiss_overlay(host).unwrap());
}

#[test]
fn segmented_options_reconcile_atomically_and_roving_selection_skips_disabled() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let control = context
        .create_component(document, SegmentedControl::new().label("Preview mode"))
        .unwrap();
    let first = context
        .create_detached_component(document, SegmentedOption::new("Code"))
        .unwrap();
    let disabled = context
        .create_detached_component(document, SegmentedOption::new("Split").disabled(true))
        .unwrap();
    let last = context
        .create_detached_component(document, SegmentedOption::new("Preview"))
        .unwrap();
    assert!(
        context
            .set_segmented_options(control, vec![first, disabled, last], Some(first))
            .unwrap()
    );

    let observed = Arc::new(Mutex::new(Vec::new()));
    let selected = Arc::clone(&observed);
    context
        .on(
            control,
            move |_control, event: &SegmentedSelectionRequested, _cx| {
                selected.lock().unwrap().push(event.option);
            },
        )
        .unwrap();
    context.focus_node(document, first.stable_id()).unwrap();
    assert!(
        context
            .navigate_focused_segmented(document, RovingFocusIntent::Next)
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(last.stable_id()));
    assert_eq!(
        context.read(control, |control| control.selected).unwrap(),
        Some(first.stable_id())
    );
    assert!(context.read(first, |option| option.selected).unwrap());
    assert!(!context.read(last, |option| option.selected).unwrap());
    assert_eq!(&*observed.lock().unwrap(), &[last.stable_id()]);
    assert!(context.activate_node(last.stable_id()).unwrap());
    assert_eq!(
        &*observed.lock().unwrap(),
        &[last.stable_id(), last.stable_id()]
    );
    assert!(!context.activate_node(disabled.stable_id()).unwrap());
    let generation = context.world().generation();
    assert!(
        context
            .set_segmented_selection(control, Some(last))
            .unwrap()
    );
    assert_eq!(context.world().generation(), generation + 1);
    assert!(!context.read(first, |option| option.selected).unwrap());
    assert!(context.read(last, |option| option.selected).unwrap());
    assert!(
        context
            .apply_accessibility_action(
                document,
                AccessibilityActionRequest {
                    target: last.stable_id(),
                    action: AccessibilityAction::Click,
                },
            )
            .unwrap()
    );
    assert!(context.read(last, |option| option.selected).unwrap());
    assert!(
        context
            .navigate_focused_segmented(document, RovingFocusIntent::Next)
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(first.stable_id()));
    assert_eq!(
        context.read(control, |control| control.selected).unwrap(),
        Some(last.stable_id())
    );

    let generation = context.world().generation();
    assert_eq!(
        context.set_segmented_options(control, vec![first, first], Some(first)),
        Err(FrameworkError::InvalidComponentValue(control.stable_id()))
    );
    assert_eq!(context.world().generation(), generation);

    assert!(
        context
            .set_segmented_options(control, vec![first, last], Some(first))
            .unwrap()
    );
    assert_eq!(
        context.world().mount_state(disabled.stable_id()),
        Some(crate::MountState::Parked)
    );
    assert!(
        !context
            .world()
            .project_accessibility(document)
            .iter()
            .any(|node| node.id == disabled.stable_id())
    );
    let accessibility = context.world().project_accessibility(document);
    assert_eq!(accessibility[0].role, crate::AccessibilityRole::RadioGroup);
    assert_eq!(accessibility[1].role, crate::AccessibilityRole::Radio);
    assert_eq!(accessibility[1].checked, Some(true));
    assert_eq!(accessibility[2].checked, Some(false));
    assert_eq!(context.next_animation_deadline(), None);
}

#[test]
fn updating_a_filled_tab_option_keeps_the_control_surface() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let control = context
        .create_component(
            document,
            Tabs::new("code")
                .size(nana_ui_core::ControlSize::Small)
                .fill(true)
                .options([
                    TabOption::new("code", "Code"),
                    TabOption::new("preview", "Preview"),
                ]),
        )
        .unwrap();
    let first = Entity::<SegmentedOption>::from_stable_id(
        context
            .read(control, |tabs| tabs.option_nodes()[0].1)
            .unwrap(),
    );
    context
        .update_component(first, |option, _| {
            *option = SegmentedOption::new("Code")
                .size(nana_ui_core::ControlSize::Small)
                .with_selected(true);
        })
        .unwrap();
    context
        .update_component(control, |tabs, _| {
            tabs.fill = true;
        })
        .unwrap();
    assert!(context.read(first, |option| option.fill).unwrap());
    assert_eq!(
        context
            .read(first, |option| option.style.layout.width)
            .unwrap(),
        Some(LengthSpec::Fill)
    );
    assert_eq!(
        context.read(first, |option| option.node_kind()).unwrap(),
        NodeKind::Element { tag: "tab".into() }
    );
}

#[test]
fn segmented_size_disabled_and_sequential_focus_share_one_authority() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let before = context
        .create_component(document, Button::new("Before"))
        .unwrap();
    let control = context
        .create_component(document, SegmentedControl::new())
        .unwrap();
    let first = context
        .create_detached_component(document, SegmentedOption::new("Code"))
        .unwrap();
    let second = context
        .create_detached_component(document, SegmentedOption::new("Preview"))
        .unwrap();
    let after = context
        .create_component(document, Button::new("After"))
        .unwrap();
    context
        .set_segmented_options(control, vec![first, second], Some(first))
        .unwrap();

    let generation = context.world().generation();
    assert!(
        context
            .set_segmented_size(control, nana_ui_core::ControlSize::Large)
            .unwrap()
    );
    assert_eq!(context.world().generation(), generation + 1);
    assert_eq!(
        context.read(control, |control| control.size).unwrap(),
        nana_ui_core::ControlSize::Large
    );
    assert_eq!(
        context.read(first, |option| option.size).unwrap(),
        nana_ui_core::ControlSize::Large
    );
    assert_eq!(
        context.read(second, |option| option.size).unwrap(),
        nana_ui_core::ControlSize::Large
    );
    let radius = context.world().theme_metrics().radius_md;
    assert_eq!(
        context
            .world()
            .node_style(control.stable_id())
            .unwrap()
            .layout
            .border_radius,
        Some(radius)
    );
    assert_eq!(
        context
            .world()
            .node_style(first.stable_id())
            .unwrap()
            .layout
            .border_radius,
        Some((radius - 3.0).max(0.0))
    );

    context.focus_node(document, before.stable_id()).unwrap();
    assert!(context.navigate_sequential_focus(document, false).unwrap());
    assert_eq!(context.world().focused(document), Some(first.stable_id()));
    assert!(context.navigate_sequential_focus(document, false).unwrap());
    assert_eq!(context.world().focused(document), Some(after.stable_id()));
    assert!(
        context
            .apply_accessibility_action(
                document,
                AccessibilityActionRequest {
                    target: second.stable_id(),
                    action: AccessibilityAction::Focus,
                },
            )
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(second.stable_id()));
    assert_eq!(
        context
            .read(control, |control| control.focus_target)
            .unwrap(),
        Some(second.stable_id())
    );
    assert!(
        context
            .set_segmented_selection(control, Some(second))
            .unwrap()
    );
    assert!(
        context
            .set_segmented_selection(control, Some(first))
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(second.stable_id()));
    assert_eq!(
        context
            .read(control, |control| control.focus_target)
            .unwrap(),
        Some(second.stable_id())
    );
    assert!(context.navigate_sequential_focus(document, false).unwrap());
    assert_eq!(context.world().focused(document), Some(after.stable_id()));

    context.focus_node(document, first.stable_id()).unwrap();
    assert!(
        context
            .set_segmented_option_disabled(control, first, true)
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(second.stable_id()));
    assert_eq!(
        context
            .read(control, |control| control.focus_target)
            .unwrap(),
        Some(second.stable_id())
    );
    assert_eq!(
        context.read(control, |control| control.selected).unwrap(),
        Some(first.stable_id())
    );
    assert!(context.read(first, |option| option.selected).unwrap());
    assert!(context.read(first, |option| option.disabled).unwrap());
    assert_eq!(
        context
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == first.stable_id())
            .unwrap()
            .checked,
        Some(true)
    );
    assert!(
        context
            .set_segmented_option_disabled(control, first, false)
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(second.stable_id()));
    assert_eq!(
        context
            .read(control, |control| control.focus_target)
            .unwrap(),
        Some(second.stable_id())
    );
}

#[test]
fn segmented_intrinsic_width_is_stable_across_viewports_sizes_icons_and_empty_groups() {
    struct FixedShaper;
    impl crate::TextShaper for FixedShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            text: &TextContent,
            _style: &crate::ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> crate::TextMetrics {
            crate::TextMetrics {
                width: text.value.chars().count() as f32 * 7.0,
                height: 16.0,
                ascent: None,
            }
        }
    }

    for (index, size) in [
        nana_ui_core::ControlSize::Small,
        nana_ui_core::ControlSize::Medium,
        nana_ui_core::ControlSize::Large,
    ]
    .into_iter()
    .enumerate()
    {
        let mut context = AppContext::new();
        let document = DocumentId::new(index as u64 + 1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new().size(size))
            .unwrap();
        let icon = context
            .create_detached_component(
                document,
                SegmentedOption::new("Code").icon(nana_ui_core::Icon::File),
            )
            .unwrap();
        let plain = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        context
            .set_segmented_options(control, vec![icon, plain], Some(icon))
            .unwrap();
        let mut shaper = FixedShaper;
        while context
            .shape_text_for_layout(document, &mut shaper)
            .unwrap()
        {}
        context
            .layout_document(document, crate::LayoutViewport::new(320.0, 100.0))
            .unwrap();
        let narrow = context.world().layout_box(control.stable_id()).unwrap();
        let icon_bounds = context.world().layout_box(icon.stable_id()).unwrap();
        let plain_bounds = context.world().layout_box(plain.stable_id()).unwrap();
        assert_eq!(narrow.height, size.height());
        assert!(icon_bounds.width > plain_bounds.width);
        assert!((narrow.width - (icon_bounds.width + plain_bounds.width + 8.0)).abs() < 0.01);

        context
            .layout_document(document, crate::LayoutViewport::new(640.0, 100.0))
            .unwrap();
        assert_eq!(
            context.world().layout_box(control.stable_id()),
            Some(narrow)
        );
    }

    let mut context = AppContext::new();
    let document = DocumentId::new(9).unwrap();
    let empty = context
        .create_component(document, SegmentedControl::new())
        .unwrap();
    context
        .layout_document(document, crate::LayoutViewport::new(640.0, 100.0))
        .unwrap();
    let empty = context.world().layout_box(empty.stable_id()).unwrap();
    assert_eq!(empty.width, 6.0);
    assert_eq!(empty.height, nana_ui_core::ControlSize::Medium.height());
}

#[test]
fn segmented_request_focus_and_event_roll_back_together_when_focus_is_blocked() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let control = context
        .create_component(document, SegmentedControl::new())
        .unwrap();
    let first = context
        .create_detached_component(document, SegmentedOption::new("Code"))
        .unwrap();
    let second = context
        .create_detached_component(document, SegmentedOption::new("Preview"))
        .unwrap();
    context
        .set_segmented_options(control, vec![first, second], Some(first))
        .unwrap();
    let host = context
        .create_component(document, OverlayHost::new())
        .unwrap();
    let dialog = context
        .create_component(document, Dialog::new("Settings"))
        .unwrap();
    context.append_child(host, dialog).unwrap();
    context.activate_overlay(host, dialog).unwrap();
    let generation = context.world().generation();

    assert!(matches!(
        context.request_segmented_selection(control, second),
        Err(FrameworkError::World(crate::UiWorldError::NotFocusable(id)))
            if id == second.stable_id()
    ));
    assert_eq!(context.world().generation(), generation);
    assert!(context.read(first, |option| option.selected).unwrap());
    assert!(!context.read(second, |option| option.selected).unwrap());
    assert_eq!(
        context
            .read(control, |control| control.focus_target)
            .unwrap(),
        Some(first.stable_id())
    );
    assert_eq!(
        context.read(control, |control| control.selected).unwrap(),
        Some(first.stable_id())
    );
}

#[test]
fn segmented_request_rolls_back_focus_when_an_event_handler_mutation_is_invalid() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let foreign_document = DocumentId::new(2).unwrap();
    let control = context
        .create_component(document, SegmentedControl::new())
        .unwrap();
    let first = context
        .create_detached_component(document, SegmentedOption::new("Code"))
        .unwrap();
    let second = context
        .create_detached_component(document, SegmentedOption::new("Preview"))
        .unwrap();
    let foreign = context
        .create_component(foreign_document, Button::new("Foreign"))
        .unwrap();
    context
        .set_segmented_options(control, vec![first, second], Some(first))
        .unwrap();
    context.focus_node(document, first.stable_id()).unwrap();
    context
        .on(
            control,
            move |_control, _event: &SegmentedSelectionRequested, cx| {
                cx.mutations()
                    .insert(foreign.stable_id(), second.stable_id(), None);
            },
        )
        .unwrap();
    let generation = context.world().generation();

    assert!(
        context
            .request_segmented_selection(control, second)
            .is_err()
    );
    assert_eq!(context.world().generation(), generation);
    assert_eq!(context.world().focused(document), Some(first.stable_id()));
    assert_eq!(
        context
            .read(control, SegmentedControl::focus_target)
            .unwrap(),
        Some(first.stable_id())
    );
    assert_eq!(
        context.read(control, SegmentedControl::selected).unwrap(),
        Some(first.stable_id())
    );
    assert!(context.read(first, SegmentedOption::selected).unwrap());
    assert!(!context.read(second, SegmentedOption::selected).unwrap());
}

#[test]
fn overlay_tab_trap_reuses_segmented_sequential_focus_authority() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let host = context
        .create_component(document, OverlayHost::new())
        .unwrap();
    let dialog = context
        .create_component(
            document,
            Dialog::new("Settings").initial_focus(crate::ModalInitialFocus::Surface),
        )
        .unwrap();
    let control = context
        .create_detached_component(document, SegmentedControl::new())
        .unwrap();
    let first = context
        .create_detached_component(document, SegmentedOption::new("Code"))
        .unwrap();
    let second = context
        .create_detached_component(document, SegmentedOption::new("Preview"))
        .unwrap();
    let action = context
        .create_detached_component(document, Button::new("Save"))
        .unwrap();
    context
        .set_segmented_options(control, vec![first, second], Some(first))
        .unwrap();
    context.append_child(host, dialog).unwrap();
    context
        .set_modal_slots(
            dialog,
            ModalSlots {
                body: Some(control.stable_id()),
                actions: vec![action.stable_id()],
                ..Default::default()
            },
        )
        .unwrap();
    context.activate_overlay(host, dialog).unwrap();
    assert_eq!(context.world().focused(document), Some(dialog.stable_id()));
    assert!(
        context
            .route_overlay_key(document, OverlayKey::Tab { reverse: false })
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(first.stable_id()));
    assert!(
        context
            .route_overlay_key(document, OverlayKey::Tab { reverse: false })
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(action.stable_id()));
    assert!(
        context
            .route_overlay_key(document, OverlayKey::Tab { reverse: false })
            .unwrap()
    );
    assert_eq!(context.world().focused(document), Some(first.stable_id()));
    assert!(context.focus_node(document, second.stable_id()).unwrap());
}

#[test]
fn native_scroll_view_projects_axes_and_typed_runtime_offset() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let scroll = context
        .create_component(
            document,
            ScrollView::new(ScrollAxes::Vertical).label("Builds"),
        )
        .unwrap();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&changes);
    context
        .on(scroll, move |_scroll, event: &ScrollChanged, _cx| {
            observed.lock().unwrap().push(event.offset);
        })
        .unwrap();
    context.world_mut().take_system_work();

    assert!(
        context
            .scroll_to(scroll, ScrollOffset { x: 40.0, y: 120.0 })
            .unwrap()
    );
    assert_eq!(
        context.world().scroll_offset(scroll.stable_id()),
        Some(ScrollOffset { x: 0.0, y: 120.0 })
    );
    assert_eq!(
        *changes.lock().unwrap(),
        vec![ScrollOffset { x: 0.0, y: 120.0 }]
    );
    assert_eq!(
        context
            .world()
            .node_style(scroll.stable_id())
            .unwrap()
            .layout
            .overflow_y,
        nana_ui_core::OverflowSpec::Scroll
    );
    let work = context.world_mut().take_system_work();
    assert_eq!(work.input_hit_test, vec![scroll.stable_id()]);
    assert_eq!(work.render_extraction, vec![scroll.stable_id()]);
    assert!(work.layout.is_empty());
    assert!(
        context
            .set_scroll_metrics(
                scroll,
                ScrollMetrics {
                    viewport_width: 100.0,
                    viewport_height: 100.0,
                    content_width: 100.0,
                    content_height: 250.0,
                },
            )
            .unwrap()
    );
    assert!(
        context
            .scroll_by(scroll, ScrollOffset { x: 0.0, y: 80.0 })
            .unwrap()
    );
    assert_eq!(
        context.world().scroll_offset(scroll.stable_id()).unwrap().y,
        150.0
    );
    assert!(
        context
            .set_scroll_metrics(
                scroll,
                ScrollMetrics {
                    viewport_width: 100.0,
                    viewport_height: 100.0,
                    content_width: 100.0,
                    content_height: 130.0,
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.world().scroll_offset(scroll.stable_id()).unwrap().y,
        30.0
    );
    assert_eq!(
        changes.lock().unwrap().as_slice(),
        [
            ScrollOffset { x: 0.0, y: 120.0 },
            ScrollOffset { x: 0.0, y: 150.0 },
            ScrollOffset { x: 0.0, y: 30.0 },
        ]
    );
    assert!(
        !context
            .scroll_to(scroll, ScrollOffset { x: 0.0, y: 30.0 })
            .unwrap()
    );
    assert_eq!(
        context.scroll_to(
            scroll,
            ScrollOffset {
                x: 0.0,
                y: f32::NAN
            }
        ),
        Err(FrameworkError::InvalidComponentValue(scroll.stable_id()))
    );
}

#[test]
fn layout_publishes_scroll_metrics_and_clamps_wheel_offset() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let mut viewport = NodeStyle::default();
    {
        let layout = std::sync::Arc::make_mut(&mut viewport.layout);
        layout.width = Some(LengthSpec::Px(200.0));
        layout.height = Some(LengthSpec::Px(120.0));
    }
    let scroll = context
        .create_component(
            document,
            ScrollView::new(ScrollAxes::Vertical).style(viewport),
        )
        .unwrap();
    for index in 0..5 {
        let mut row = NodeStyle::default();
        {
            let layout = std::sync::Arc::make_mut(&mut row.layout);
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Px(40.0));
        }
        let row = context
            .create_component(document, Text::new(format!("Row {index}")).style(row))
            .unwrap();
        context.append_child(scroll, row).unwrap();
    }
    context
        .layout_document(document, crate::LayoutViewport::new(200.0, 120.0))
        .unwrap();
    let metrics = context
        .world()
        .scroll_metrics(scroll.stable_id())
        .expect("layout publishes scroll metrics");
    assert!(
        metrics.content_height > metrics.viewport_height,
        "content {metrics:?} should overflow the viewport"
    );
    let max_y = (metrics.content_height - metrics.viewport_height).max(0.0);
    assert!(
        context
            .scroll_by(
                scroll,
                ScrollOffset {
                    x: 0.0,
                    y: max_y + 400.0
                }
            )
            .unwrap()
    );
    let offset = context.world().scroll_offset(scroll.stable_id()).unwrap();
    assert!(
        (offset.y - max_y).abs() < 0.01,
        "wheel offset {} should clamp to {max_y}",
        offset.y
    );
    assert_eq!(offset.x, 0.0);
}

/// 200x120 scrollport holding 200px of rows, so the vertical axis overflows
/// by 80px.
fn overflowing_scroll_view(
    context: &mut AppContext,
    document: DocumentId,
    visibility: nana_ui_core::ScrollbarVisibility,
) -> Entity<ScrollView> {
    let mut viewport = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut viewport.layout);
        layout.width = Some(LengthSpec::Px(200.0));
        layout.height = Some(LengthSpec::Px(120.0));
    }
    let scroll = context
        .create_component(
            document,
            ScrollView::new(ScrollAxes::Vertical)
                .scrollbars(visibility)
                .style(viewport),
        )
        .unwrap();
    for index in 0..5 {
        let mut row = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut row.layout);
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Px(40.0));
        }
        let row = context
            .create_component(document, Text::new(format!("Row {index}")).style(row))
            .unwrap();
        context.append_child(scroll, row).unwrap();
    }
    context
        .layout_document(document, crate::LayoutViewport::new(200.0, 120.0))
        .unwrap();
    scroll
}

fn vertical_bar(context: &AppContext, scroll: Entity<ScrollView>) -> Option<crate::ScrollbarBar> {
    match context.world().component_geometry(scroll.stable_id()) {
        Some(crate::ComponentGeometry::Scrollbar { vertical, .. }) => vertical,
        _ => None,
    }
}

#[test]
fn auto_hiding_scrollbars_appear_on_hover_and_follow_the_scroll_offset() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let scroll = overflowing_scroll_view(
        &mut context,
        document,
        nana_ui_core::ScrollbarVisibility::AutoHide,
    );
    assert!(
        vertical_bar(&context, scroll).is_none(),
        "an idle auto-hiding container draws no bar"
    );

    context
        .set_pointer_hover_at(
            document,
            1,
            Some(scroll.stable_id()),
            std::time::Duration::ZERO,
        )
        .unwrap();
    let bar = vertical_bar(&context, scroll).expect("hover reveals the bar");
    // 120 of 200 content is visible, so the thumb takes 60% of the track.
    assert!(
        (bar.thumb.height - 72.0).abs() < 0.01,
        "thumb {:?}",
        bar.thumb
    );
    assert!(
        (bar.thumb.y - bar.track.y).abs() < 0.01,
        "thumb starts at the top"
    );
    assert!((bar.max_offset - 80.0).abs() < 0.01);
    assert_eq!(bar.track_background, None, "auto-hide draws no track");

    assert!(
        context
            .scroll_to(scroll, ScrollOffset { x: 0.0, y: 80.0 })
            .unwrap()
    );
    let bar = vertical_bar(&context, scroll).expect("still hovered");
    assert!(
        (bar.thumb.y + bar.thumb.height - (bar.track.y + bar.track.height)).abs() < 0.01,
        "a maxed offset pins the thumb to the track end: {:?}",
        bar.thumb
    );

    context
        .set_pointer_hover_at(document, 1, None, std::time::Duration::ZERO)
        .unwrap();
    assert!(
        vertical_bar(&context, scroll).is_none(),
        "leaving the container hides the bar again"
    );
}

#[test]
fn resident_scrollbars_draw_a_track_without_hover() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let scroll = overflowing_scroll_view(
        &mut context,
        document,
        nana_ui_core::ScrollbarVisibility::Always,
    );
    let bar = vertical_bar(&context, scroll).expect("resident bars need no hover");
    assert!(bar.track_background.is_some());
    assert!((bar.track.width - nana_ui_core::SCROLLBAR_METRICS.thickness).abs() < 0.01);
    assert!(
        (bar.track.x + bar.track.width - 200.0).abs() < 0.01,
        "bar hugs the right edge"
    );
}

#[test]
fn hidden_scrollbars_leave_wheel_scrolling_alone() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let scroll = overflowing_scroll_view(
        &mut context,
        document,
        nana_ui_core::ScrollbarVisibility::Hidden,
    );
    context
        .set_pointer_hover_at(
            document,
            1,
            Some(scroll.stable_id()),
            std::time::Duration::ZERO,
        )
        .unwrap();
    assert!(vertical_bar(&context, scroll).is_none());
    assert!(
        context
            .scroll_by(scroll, ScrollOffset { x: 0.0, y: 40.0 })
            .unwrap()
    );
    assert_eq!(
        context.world().scroll_offset(scroll.stable_id()),
        Some(ScrollOffset { x: 0.0, y: 40.0 })
    );
}

#[test]
fn dragging_the_thumb_moves_the_authoritative_scroll_offset() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let scroll = overflowing_scroll_view(
        &mut context,
        document,
        nana_ui_core::ScrollbarVisibility::Always,
    );
    let bar = vertical_bar(&context, scroll).expect("resident bar");
    let grab_x = bar.thumb.x + bar.thumb.width / 2.0;
    let grab_y = bar.thumb.y + bar.thumb.height / 2.0;
    assert_eq!(
        context.scrollbar_axis_at(scroll.stable_id(), grab_x, grab_y),
        Some(nana_ui_core::ScrollbarAxis::Vertical)
    );
    assert!(
        context
            .begin_scrollbar_drag(
                7,
                scroll.stable_id(),
                nana_ui_core::ScrollbarAxis::Vertical,
                grab_x,
                grab_y,
            )
            .unwrap()
    );
    assert_eq!(
        context.world().pointer_capture(document, 7),
        Some(scroll.stable_id())
    );
    assert_eq!(
        context.world().scroll_offset(scroll.stable_id()),
        Some(ScrollOffset::default()),
        "grabbing the thumb must not jump the content"
    );

    // Travel is 48px for 80px of content, so half the travel is 40px.
    assert!(
        context
            .update_scrollbar_drag(document, 7, grab_x, grab_y + 24.0)
            .unwrap()
    );
    let offset = context.world().scroll_offset(scroll.stable_id()).unwrap();
    assert!((offset.y - 40.0).abs() < 0.01, "offset {offset:?}");
    assert_eq!(offset.x, 0.0, "a vertical drag holds the other axis");

    assert!(
        context
            .update_scrollbar_drag(document, 7, grab_x, grab_y + 4000.0)
            .unwrap()
    );
    assert!(
        (context.world().scroll_offset(scroll.stable_id()).unwrap().y - 80.0).abs() < 0.01,
        "the drag clamps at the maximum offset"
    );

    assert!(context.end_scrollbar_drag(document, 7, false).unwrap());
    assert_eq!(context.world().pointer_capture(document, 7), None);
    assert!(
        !context
            .update_scrollbar_drag(document, 7, grab_x, grab_y)
            .unwrap(),
        "a released pointer no longer drives the bar"
    );
}

#[test]
fn pressing_bare_track_pages_toward_the_press_and_cancelling_restores_it() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let scroll = overflowing_scroll_view(
        &mut context,
        document,
        nana_ui_core::ScrollbarVisibility::Always,
    );
    let bar = vertical_bar(&context, scroll).expect("resident bar");
    let track_end = bar.track.y + bar.track.height - 1.0;
    assert!(
        context
            .begin_scrollbar_drag(
                3,
                scroll.stable_id(),
                nana_ui_core::ScrollbarAxis::Vertical,
                bar.thumb.x + 1.0,
                track_end,
            )
            .unwrap()
    );
    assert!(
        (context.world().scroll_offset(scroll.stable_id()).unwrap().y - 80.0).abs() < 0.01,
        "a press below the thumb centres it on the press"
    );
    assert!(context.end_scrollbar_drag(document, 3, true).unwrap());
    assert_eq!(
        context.world().scroll_offset(scroll.stable_id()),
        Some(ScrollOffset::default()),
        "cancel restores the offset the drag started from"
    );
}

#[test]
fn a_secondary_press_reaches_the_nearest_handler_above_the_hit_node() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let mut card = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut card.layout);
        layout.width = Some(LengthSpec::Px(200.0));
        layout.height = Some(LengthSpec::Px(100.0));
    }
    let card = context
        .create_component(document, Card::new().style(card))
        .unwrap();
    let mut row = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut row.layout);
        layout.width = Some(LengthSpec::Px(200.0));
        layout.height = Some(LengthSpec::Px(40.0));
    }
    let row = context
        .create_component(document, Button::new("Row").style(row))
        .unwrap();
    context.append_child(card, row).unwrap();
    let presses = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&presses);
    context
        .on(card, move |_card, press: &SecondaryPress, _cx| {
            observed.lock().unwrap().push(*press);
        })
        .unwrap();
    context
        .layout_document(document, crate::LayoutViewport::new(200.0, 100.0))
        .unwrap();
    context.rebuild_hit_test(document);

    assert_eq!(
        context.secondary_press_at(document, 20.0, 20.0).unwrap(),
        Some(card.stable_id()),
        "the press bubbles to the enclosing handler"
    );
    let press = *presses.lock().unwrap().first().expect("one press");
    assert_eq!(press.target, row.stable_id(), "it carries the hit node");
    assert_eq!((press.x, press.y), (20.0, 20.0));

    assert_eq!(
        context.secondary_press_at(document, 900.0, 900.0).unwrap(),
        None,
        "a press outside the tree hits nothing"
    );
    assert_eq!(presses.lock().unwrap().len(), 1);
}

#[test]
#[cfg(feature = "rich-text")]
fn selection_reads_back_from_editors_and_from_focused_rich_text() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let input = context
        .create_component(document, TextInput::new("Nana"))
        .unwrap();
    assert!(context.focus_node(document, input.stable_id()).unwrap());
    assert_eq!(
        context.focused_selected_text(document),
        None,
        "a caret selects nothing"
    );
    assert!(context.select_all_focused_text(document).unwrap());
    assert_eq!(
        context.focused_selected_text(document).as_deref(),
        Some("Nana")
    );
    assert!(
        !context.select_all_focused_text(document).unwrap(),
        "selecting all twice is not a change"
    );
    assert_eq!(
        context.cut_focused_text(document).unwrap().as_deref(),
        Some("Nana")
    );
    assert_eq!(context.world().text(input.stable_id()), Some(""));
    assert_eq!(context.cut_focused_text(document).unwrap(), None);

    let text = context
        .create_component(
            document,
            crate::SelectableRichText::new([crate::RichSpan::plain("Hello")]),
        )
        .unwrap();
    assert!(context.focus_node(document, text.stable_id()).unwrap());
    let area = crate::LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 20.0,
    };
    let caret = |index: usize| index as f32 * crate::rich_text::GRAPHEME_ADVANCE + 1.0;
    context
        .read(text, |text| {
            assert!(text.pointer_down(caret(0), 8.0, area));
            assert!(text.pointer_move(caret(4), 8.0, area));
            text.pointer_up(caret(4), 8.0, area)
        })
        .unwrap();
    assert_eq!(
        context.focused_selected_text(document).as_deref(),
        Some("Hell"),
        "a rich-text selection is what a host copy takes"
    );
    assert_eq!(
        context.cut_focused_text(document).unwrap(),
        None,
        "rich text is not editable, so nothing is cut"
    );
}

#[test]
fn a_secondary_press_without_a_handler_opens_nothing() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(120.0));
        layout.height = Some(LengthSpec::Px(40.0));
    }
    let button = context
        .create_component(document, Button::new("Build").style(style))
        .unwrap();
    context
        .layout_document(document, crate::LayoutViewport::new(120.0, 40.0))
        .unwrap();
    context.rebuild_hit_test(document);
    let generation = context.world().generation();
    assert_eq!(
        context.secondary_press_at(document, 10.0, 10.0).unwrap(),
        None
    );
    assert_eq!(
        context.world().generation(),
        generation,
        "an unhandled secondary press must not touch the tree"
    );
    assert!(context.world().focused(document).is_none());
    let _ = button;
}

#[test]
fn scroll_view_with_forty_rows_dirties_forty_one_hit_targets() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let scroll = context
        .create_component(document, ScrollView::new(ScrollAxes::Vertical))
        .unwrap();
    for index in 0..40 {
        let row = context
            .create_component(document, Text::new(format!("Visible row {index}")))
            .unwrap();
        context.append_child(scroll, row).unwrap();
    }
    let _ = context.take_system_work();
    assert!(
        context
            .scroll_to(scroll, ScrollOffset { x: 0.0, y: 120.0 })
            .unwrap()
    );
    let work = context.take_system_work();
    // Scroller-only hit/extract; Scene recomposes descendants from offset.
    assert_eq!(work.input_hit_test.len(), 1);
    assert_eq!(work.render_extraction.len(), 1);
    assert!(work.layout.is_empty());
    let updates = context.take_scroll_hit_updates();
    assert!(
        context.hit_test_work_is_scroll_only(&work.input_hit_test, &updates),
        "pure scrolling must be recognized as patch-only"
    );
}

#[test]
fn native_theme_resolves_semantic_component_paint_without_layout_work() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let button = context
        .create_component(
            document,
            Button::new("Build").kind(nana_ui_core::ButtonKind::Primary),
        )
        .unwrap();
    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    let dark = context
        .world()
        .extract_nodes(&[button.stable_id()])
        .pop()
        .unwrap();
    assert_eq!(
        dark.style.background,
        Some(
            nana_ui_core::SemanticPalette::dark()
                .accent_soft
                .as_rgba_array()
        )
    );
    context
        .world_mut()
        .set_pointer_hover(document, 1, Some(button.stable_id()))
        .unwrap();
    context.advance_animations(nana_ui_core::motion::HOVER_COLOR);
    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    assert_eq!(
        context
            .world()
            .extract_nodes(&[button.stable_id()])
            .pop()
            .unwrap()
            .style
            .background,
        Some(
            nana_ui_core::SemanticPalette::dark()
                .accent_soft_hover
                .as_rgba_array()
        )
    );
    context
        .world_mut()
        .press_pointer(document, 1, button.stable_id())
        .unwrap();
    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    assert_eq!(
        context
            .world()
            .extract_nodes(&[button.stable_id()])
            .pop()
            .unwrap()
            .style
            .background,
        Some(
            nana_ui_core::SemanticPalette::dark()
                .accent_soft_pressed
                .as_rgba_array()
        )
    );
    assert_eq!(
        context.release_pointer(document, 1),
        Some(button.stable_id())
    );
    context
        .world_mut()
        .set_pointer_hover(document, 1, None)
        .unwrap();
    context.world_mut().take_system_work();

    assert!(context.set_theme(ThemeMode::Light).unwrap());
    let work = context.world_mut().take_system_work();
    assert!(work.style.is_empty());
    assert!(work.layout.is_empty());
    assert!(work.render_extraction.contains(&button.stable_id()));
    let light = context
        .world()
        .extract_nodes(&[button.stable_id()])
        .pop()
        .unwrap();
    assert_eq!(
        light.style.background,
        Some(
            nana_ui_core::SemanticPalette::light()
                .accent_soft
                .as_rgba_array()
        )
    );

    let mut focus = MutationQueue::new();
    focus.request_focus(document, Some(button.stable_id()));
    context.world_mut().commit(focus).unwrap();
    let work = context.world_mut().take_system_work();
    assert_eq!(work.focus_ime, vec![button.stable_id()]);
    assert_eq!(work.accessibility, vec![button.stable_id()]);
    assert!(context.world_mut().take_system_work().is_empty());
    context.world_mut().resolve_styles(&work.style).unwrap();
    assert!(context.world_mut().take_system_work().is_empty());
    let focused = context
        .world()
        .extract_nodes(&[button.stable_id()])
        .pop()
        .unwrap();
    assert_eq!(focused.style.border_color, None);
    assert_eq!(
        focused.style.background,
        Some(
            nana_ui_core::SemanticPalette::light()
                .accent_soft
                .as_rgba_array()
        )
    );

    context
        .update_component(button, |button, _cx| button.disabled = true)
        .unwrap();
    let work = context.world_mut().take_system_work();
    assert_eq!(work.focus_ime, vec![button.stable_id()]);
    assert_eq!(work.accessibility, vec![button.stable_id()]);
    assert!(context.world_mut().take_system_work().is_empty());
    context.world_mut().resolve_styles(&work.style).unwrap();
    let post_resolve = context.world_mut().take_system_work();
    assert_eq!(post_resolve.focus_ime, vec![button.stable_id()]);
    assert_eq!(post_resolve.accessibility, vec![button.stable_id()]);
    assert_eq!(post_resolve.render_extraction, vec![button.stable_id()]);
    assert_eq!(context.world().focused(document), None);
    let disabled = context
        .world()
        .extract_nodes(&[button.stable_id()])
        .pop()
        .unwrap();
    assert_eq!(
        disabled.style.background,
        Some(
            nana_ui_core::SemanticPalette::light()
                .subtle
                .as_rgba_array()
        )
    );

    let generation = context.world().generation();
    assert!(!context.set_theme(ThemeMode::Light).unwrap());
    assert_eq!(context.world().generation(), generation);
    let idle = context.world_mut().take_system_work();
    assert!(
        idle.is_empty(),
        "unexpected work after theme no-op: {idle:?}"
    );
}

#[test]
fn view_mutations_schedule_host_driven_animation_frames() {
    let mut context = AppContext::new();
    let entity = context
        .create_view(
            DocumentId::new(1).unwrap(),
            NodeKind::Document,
            Counter { value: 0 },
        )
        .unwrap();
    let id = AnimationId::new(1).unwrap();
    context
        .update(entity, |_view, cx| {
            let target = cx.entity().stable_id();
            cx.mutations().start_animation(AnimationSpec {
                id,
                target,
                start: Duration::from_millis(40),
                duration: Duration::from_millis(80),
                frame_interval: Duration::from_millis(10),
                easing: Easing::Linear,
                iteration_count: crate::AnimationIteration::ONCE,
                direction: crate::AnimationDirection::Normal,
                fill_mode: crate::AnimationFillMode::None,
                play_state: crate::AnimationPlayState::Running,
            });
        })
        .unwrap();

    assert_eq!(
        context.next_animation_deadline(),
        Some(Duration::from_millis(40))
    );
    let frame = context.advance_animations(Duration::from_millis(80));
    assert_eq!(frame.samples.len(), 1);
    assert_eq!(frame.samples[0].target, entity.stable_id());
    assert_eq!(frame.samples[0].progress, 0.5);
    assert_eq!(frame.next_deadline, Some(Duration::from_millis(90)));
}

#[test]
fn remount_resumes_loading_lifecycle_in_a_retained_descendant() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let host = context
        .create_component(document, Button::new("Host"))
        .unwrap();
    let parent = context
        .create_component(document, Button::new("Parent"))
        .unwrap();
    let loading = context
        .create_detached_component(document, Button::new("Loading").loading(true))
        .unwrap();

    assert_eq!(context.next_animation_deadline(), None);
    context.append_child(parent, loading).unwrap();
    assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));

    let mut park = MutationQueue::new();
    park.park_subtree(parent.stable_id());
    context.commit_mutations(park).unwrap();
    assert_eq!(context.next_animation_deadline(), None);

    let mut remount = MutationQueue::new();
    remount.insert(host.stable_id(), parent.stable_id(), None);
    context.commit_mutations(remount).unwrap();
    assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));

    let frame = context.advance_animations(Duration::from_millis(400));
    assert!(frame.component_updates.contains(&loading.stable_id()));
    assert_eq!(
        context
            .read(loading, |button| button.loading_phase)
            .unwrap(),
        0.5
    );
    assert!(context.next_animation_deadline().is_some());
}

#[test]
fn icon_button_tooltip_uses_hover_clock_and_real_overlay_child() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let button = context
        .create_component(
            document,
            IconButton::new(nana_ui_core::Icon::About, "Details").tooltip(
                "More details",
                nana_ui_core::TooltipConfig {
                    placement: nana_ui_core::TooltipPlacement::Left,
                    delay_ms: 100,
                    gap: 6.0,
                    viewport_padding: 4.0,
                    max_width: 120.0,
                },
            ),
        )
        .unwrap();
    let tooltip = context.icon_button_tooltip(button).unwrap().unwrap();
    assert_eq!(
        context.world().node(tooltip.stable_id()).unwrap().parent,
        Some(button.stable_id())
    );
    assert_eq!(
        context.world().overlay_host(button.stable_id()),
        Some(crate::OverlayHostState::default())
    );
    context
        .layout_document(document, crate::LayoutViewport::new(160.0, 80.0))
        .unwrap();
    let mut layout = MutationQueue::new();
    layout.write_layout(
        button.stable_id(),
        crate::LayoutBox {
            x: 20.0,
            y: 50.0,
            width: 28.0,
            height: 28.0,
        },
    );
    context.commit_mutations(layout).unwrap();

    context
        .set_pointer_hover_at(
            document,
            1,
            Some(button.stable_id()),
            Duration::from_millis(10),
        )
        .unwrap();
    assert!(
        context
            .next_animation_deadline()
            .is_some_and(|time| time <= Duration::from_millis(110))
    );
    context.advance_animations(Duration::from_millis(109));
    assert_eq!(
        context
            .world()
            .overlay_host(button.stable_id())
            .unwrap()
            .active,
        None
    );
    assert!(
        context
            .advance_animations(Duration::from_millis(110))
            .component_updates
            .contains(&button.stable_id())
    );
    assert_eq!(
        context
            .world()
            .overlay_host(button.stable_id())
            .unwrap()
            .active,
        Some(tooltip.stable_id())
    );
    let tooltip_style = context.world().node_style(tooltip.stable_id()).unwrap();
    assert!(matches!(
        tooltip_style.layout.offset_left,
        Some(LengthSpec::Px(x)) if (4.0..=156.0).contains(&x)
    ));
    assert!(matches!(
        tooltip_style.layout.offset_top,
        Some(LengthSpec::Px(y)) if (4.0..=76.0).contains(&y)
    ));
    assert!(
        matches!(
            tooltip_style.layout.offset_left,
            Some(LengthSpec::Px(x)) if x >= 54.0
        ),
        "tooltip should flip to the anchor's right, got {:?}",
        tooltip_style.layout.offset_left
    );

    context
        .set_pointer_hover_at(document, 1, None, Duration::from_millis(111))
        .unwrap();
    assert_eq!(
        context.world().overlay_host(button.stable_id()),
        Some(crate::OverlayHostState::default())
    );
    context.advance_animations(Duration::from_millis(231));
    assert_eq!(context.next_animation_deadline(), None);
}

#[test]
fn parked_icon_button_closes_tooltip_projection_and_does_not_reopen_on_remount() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let host = context
        .create_component(document, Button::new("Host"))
        .unwrap();
    let button = context
        .create_component(
            document,
            IconButton::new(nana_ui_core::Icon::About, "Details").tooltip(
                "More details",
                nana_ui_core::TooltipConfig {
                    delay_ms: 0,
                    ..nana_ui_core::TooltipConfig::default()
                },
            ),
        )
        .unwrap();

    context
        .set_pointer_hover_at(document, 1, Some(button.stable_id()), Duration::ZERO)
        .unwrap();
    assert!(context.read(button, |button| button.tooltip_open).unwrap());
    assert!(matches!(
        context.world().standard_visual(button.stable_id()),
        Some(StandardVisual::Icon {
            tooltip: Some(crate::TooltipVisual { open: true, .. }),
            ..
        })
    ));

    let mut park = MutationQueue::new();
    park.park_subtree(button.stable_id());
    context.commit_mutations(park).unwrap();
    assert!(!context.read(button, |button| button.tooltip_open).unwrap());
    assert!(matches!(
        context.world().standard_visual(button.stable_id()),
        Some(StandardVisual::Icon {
            tooltip: Some(crate::TooltipVisual { open: false, .. }),
            ..
        })
    ));
    assert_eq!(
        context.world().overlay_host(button.stable_id()),
        Some(crate::OverlayHostState::default())
    );
    assert_eq!(context.next_animation_deadline(), None);

    let mut remount = MutationQueue::new();
    remount.insert(host.stable_id(), button.stable_id(), None);
    context.commit_mutations(remount).unwrap();
    assert!(
        !context
            .advance_animations(Duration::from_secs(1))
            .has_updates()
    );
    assert!(!context.read(button, |button| button.tooltip_open).unwrap());
    assert_eq!(
        context.world().overlay_host(button.stable_id()),
        Some(crate::OverlayHostState::default())
    );

    context
        .set_pointer_hover_at(
            document,
            2,
            Some(button.stable_id()),
            Duration::from_secs(2),
        )
        .unwrap();
    assert!(context.read(button, |button| button.tooltip_open).unwrap());
    context
        .update_component(button, |_button, cx| {
            cx.mutations().park_subtree(button.stable_id());
        })
        .unwrap();
    assert!(!context.read(button, |button| button.tooltip_open).unwrap());
    assert!(matches!(
        context.world().standard_visual(button.stable_id()),
        Some(StandardVisual::Icon {
            tooltip: Some(crate::TooltipVisual { open: false, .. }),
            ..
        })
    ));
}

#[test]
fn tooltip_default_delay_stays_closed_until_deadline_and_is_label_only() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let button = context
        .create_component(
            document,
            IconButton::new(nana_ui_core::Icon::About, "Details")
                .tooltip("More details", nana_ui_core::TooltipConfig::default()),
        )
        .unwrap();
    let tooltip = context.icon_button_tooltip(button).unwrap().unwrap();
    assert_eq!(
        context.world().text(tooltip.stable_id()),
        Some("More details")
    );
    let accessibility = context.world().accessibility(tooltip.stable_id()).unwrap();
    assert_eq!(accessibility.role, crate::AccessibilityRole::Tooltip);
    assert_eq!(accessibility.label.as_deref(), Some("More details"));
    assert!(
        !context
            .world()
            .interaction(tooltip.stable_id())
            .unwrap()
            .focusable
    );

    context
        .set_pointer_hover_at(
            document,
            1,
            Some(button.stable_id()),
            Duration::from_millis(10),
        )
        .unwrap();
    assert!(
        context
            .next_animation_deadline()
            .is_some_and(|time| time <= Duration::from_millis(360))
    );
    context.advance_animations(Duration::from_millis(359));
    assert_eq!(
        context
            .world()
            .overlay_host(button.stable_id())
            .unwrap()
            .active,
        None
    );
    assert_eq!(
        context.world().overlay_host(button.stable_id()),
        Some(crate::OverlayHostState::default())
    );
    assert!(
        context
            .advance_animations(Duration::from_millis(360))
            .component_updates
            .contains(&button.stable_id())
    );
    assert_eq!(
        context
            .world()
            .overlay_host(button.stable_id())
            .unwrap()
            .active,
        Some(tooltip.stable_id())
    );
    assert!(context.read(button, |button| button.tooltip_open).unwrap());
}

#[test]
fn tooltip_default_follows_pointer_as_a_compact_card() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let button = context
        .create_component(
            document,
            IconButton::new(nana_ui_core::Icon::About, "Details").tooltip(
                "More details",
                nana_ui_core::TooltipConfig {
                    delay_ms: 0,
                    ..nana_ui_core::TooltipConfig::default()
                },
            ),
        )
        .unwrap();
    let tooltip = context.icon_button_tooltip(button).unwrap().unwrap();
    context
        .layout_document(document, crate::LayoutViewport::new(200.0, 120.0))
        .unwrap();
    let mut layout = MutationQueue::new();
    layout.write_layout(
        button.stable_id(),
        crate::LayoutBox {
            x: 20.0,
            y: 50.0,
            width: 28.0,
            height: 28.0,
        },
    );
    context.commit_mutations(layout).unwrap();

    context.set_pointer_location(document, 1, Some((48.0, 72.0)));
    context
        .set_pointer_hover_at(document, 1, Some(button.stable_id()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        context
            .world()
            .overlay_host(button.stable_id())
            .unwrap()
            .active,
        Some(tooltip.stable_id())
    );

    let tooltip_style = context.world().node_style(tooltip.stable_id()).unwrap();
    assert_eq!(
        tooltip_style.layout.padding_left,
        Some(LengthSpec::Px(TooltipConfig::PADDING_X))
    );
    assert_eq!(
        tooltip_style.layout.padding_top,
        Some(LengthSpec::Px(TooltipConfig::PADDING_Y))
    );
    assert_eq!(
        tooltip_style.layout.border_radius,
        Some(TooltipConfig::RADIUS)
    );
    assert_eq!(
        tooltip_style.border,
        Some(nana_ui_core::SemanticColorRole::BorderSoft)
    );
    assert!(
        matches!(
            tooltip_style.layout.offset_left,
            Some(LengthSpec::Px(x)) if (x - 48.0).abs() < 0.01
        ),
        "default tooltip should bind to the pointer x, got {:?}",
        tooltip_style.layout.offset_left
    );
    assert!(
        matches!(
            tooltip_style.layout.offset_top,
            Some(LengthSpec::Px(y)) if y < 72.0 - TooltipConfig::PADDING_Y
        ),
        "tooltip should sit above the pointer, got {:?}",
        tooltip_style.layout.offset_top
    );
}

#[test]
fn loading_components_schedule_only_while_loading() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let switch = context
        .create_component(document, Switch::new("Sync", false).loading(true))
        .unwrap();
    let card = context
        .create_component(document, crate::Card::new().loading(true))
        .unwrap();
    assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));
    let frame = context.advance_animations(Duration::ZERO);
    assert_eq!(
        frame
            .component_updates
            .iter()
            .copied()
            .collect::<HashSet<_>>(),
        HashSet::from([switch.stable_id(), card.stable_id()])
    );
    assert_eq!(
        context.next_animation_deadline(),
        Some(COMPONENT_FRAME_INTERVAL)
    );
    context
        .update_component(switch, |switch, _| switch.loading = false)
        .unwrap();
    context
        .update_component(card, |card, _| card.loading = false)
        .unwrap();
    assert_eq!(context.next_animation_deadline(), None);
    assert!(
        !context
            .advance_animations(Duration::from_secs(1))
            .has_updates()
    );
}

#[test]
fn workspace_transitions_schedule_only_while_transitioning() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let layout = nana_ui_core::WorkspaceLayout::new([
        nana_ui_core::RegionState::new(
            nana_ui_core::RegionId::Resources,
            nana_ui_core::RegionRole::Resources,
        )
        .size(240.0)
        .collapsible(true),
        nana_ui_core::RegionState::new(
            nana_ui_core::RegionId::Primary,
            nana_ui_core::RegionRole::Primary,
        )
        .fill_priority(1),
    ])
    .expect("workspace layout");
    let workspace = context
        .create_component(
            document,
            Workspace::from_model(&nana_ui_core::WorkspaceModel::with_layout(layout), []),
        )
        .unwrap();
    context
        .update_component(workspace, |workspace, _| {
            assert!(workspace.model.update(
                WorkspaceMutation::SetRegionCollapsed(nana_ui_core::RegionId::Resources, true,),
                Duration::ZERO,
            ));
        })
        .unwrap();
    // 过渡登记进帧调度，deadline 立即生效。
    assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));
    let frame = context.advance_animations(Duration::ZERO);
    assert!(frame.component_updates.contains(&workspace.stable_id()));
    assert_eq!(
        context.next_animation_deadline(),
        Some(COMPONENT_FRAME_INTERVAL)
    );
    // 过渡结束的帧：回收过渡并撤销帧调度。
    let frame = context.advance_animations(nana_ui_core::WORKSPACE_REGION_TRANSITION_DURATION);
    assert!(frame.component_updates.contains(&workspace.stable_id()));
    assert_eq!(context.next_animation_deadline(), None);
    assert!(
        !context
            .advance_animations(nana_ui_core::WORKSPACE_REGION_TRANSITION_DURATION)
            .has_updates()
    );
    assert_eq!(
        context
            .read(workspace, |workspace| workspace
                .model
                .region_extent(&nana_ui_core::RegionId::Resources))
            .unwrap(),
        0.0
    );
}

#[test]
fn list_item_slots_are_unique_direct_children_in_canonical_order() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let item = context
        .create_component(document, ListItem::new("fallback"))
        .unwrap();
    let leading = context.create_component(document, Text::new("L")).unwrap();
    let content = context.create_component(document, Text::new("C")).unwrap();
    let trailing = context.create_component(document, Text::new("T")).unwrap();
    context.append_child(item, content).unwrap();
    context.append_child(item, trailing).unwrap();
    context.append_child(item, leading).unwrap();
    let slots = ListItemSlots {
        leading: Some(leading.stable_id()),
        content: Some(content.stable_id()),
        trailing: Some(trailing.stable_id()),
    };
    assert!(context.set_list_item_slots(item, slots).unwrap());
    assert_eq!(
        context.world().node(item.stable_id()).unwrap().children,
        vec![
            leading.stable_id(),
            content.stable_id(),
            trailing.stable_id()
        ]
    );
    assert!(!context.set_list_item_slots(item, slots).unwrap());

    let duplicate = ListItemSlots {
        leading: Some(leading.stable_id()),
        content: Some(leading.stable_id()),
        trailing: Some(trailing.stable_id()),
    };
    assert!(matches!(
        context.set_list_item_slots(item, duplicate),
        Err(FrameworkError::InvalidListItemSlots {
            item: invalid,
            slot: None
        }) if invalid == item.stable_id()
    ));
}

#[test]
fn composite_geometry_separates_text_controls_and_range_drag_axis() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let sized = |width, height| {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(width));
        layout.height = Some(LengthSpec::Px(height));
        style
    };
    let switch = context
        .create_component(
            document,
            Switch::new("Automatic updates", false)
                .hint("Runs in the background")
                .style(sized(380.0, 52.0)),
        )
        .unwrap();
    let range = context
        .create_component(
            document,
            RangeField::new(50.0, 0.0, 100.0, 1.0)
                .unwrap()
                .label("Volume")
                .unit("%")
                .style(sized(300.0, 58.0)),
        )
        .unwrap();
    let card = context
        .create_component(
            document,
            Card::new().title("Overview").padding(28.0).height(120.0),
        )
        .unwrap();
    let body = context
        .create_component(document, Text::new("Body"))
        .unwrap();
    context.append_child(card, body).unwrap();
    context
        .layout_document(document, crate::LayoutViewport::new(640.0, 480.0))
        .unwrap();

    let crate::ComponentGeometry::Switch {
        label,
        hint: Some(hint),
        control,
        ..
    } = context
        .world()
        .component_geometry(switch.stable_id())
        .unwrap()
    else {
        panic!("switch geometry must include label, hint, and control");
    };
    assert!(label.bounds.x + label.bounds.width <= control.x);
    assert!(label.bounds.y + label.bounds.height <= hint.bounds.y);

    let crate::ComponentGeometry::Card {
        title: Some(title),
        content,
        ..
    } = context
        .world()
        .component_geometry(card.stable_id())
        .unwrap()
    else {
        panic!("card geometry must include title and content");
    };
    assert!(title.bounds.y + title.bounds.height <= content.y);
    assert!(title.bounds.width >= content.width - 0.01);
    assert!(context.world().layout_box(body.stable_id()).unwrap().y >= content.y);
    let card_layout = context
        .world()
        .node_style(card.stable_id())
        .unwrap()
        .layout
        .as_ref();
    assert_eq!(card_layout.padding_top, Some(LengthSpec::Px(52.0)));
    assert_eq!(card_layout.padding_bottom, Some(LengthSpec::Px(28.0)));

    let crate::ComponentGeometry::Range { track, .. } = context
        .world()
        .component_geometry(range.stable_id())
        .unwrap()
    else {
        panic!("range geometry must expose the interaction axis");
    };
    assert!(track.x > context.world().layout_box(range.stable_id()).unwrap().x);
    context
        .begin_range_drag(document, 7, range.stable_id(), track.x)
        .unwrap();
    assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);
    context
        .update_range_drag(document, 7, track.x + track.width)
        .unwrap();
    assert_eq!(context.read(range, |range| range.value).unwrap(), 100.0);
}

#[test]
fn a_focused_range_keeps_its_rail_interaction_free() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let range = context
        .create_component(
            document,
            RangeField::new(25.0, 0.0, 100.0, 1.0)
                .unwrap()
                .label("Volume")
                .unit("%"),
        )
        .unwrap();
    context
        .layout_document(document, crate::LayoutViewport::new(640.0, 480.0))
        .unwrap();
    context.focus_node(document, range.stable_id()).unwrap();
    let work = context.world_mut().take_system_work();
    context.world_mut().resolve_styles(&work.style).unwrap();
    let focused = context
        .world()
        .extract_nodes(&[range.stable_id()])
        .pop()
        .unwrap();
    // The rail paints with the resolved border colour, so focus must not
    // touch it: the thumb carries the focus ring instead (LiliaUI).
    assert_eq!(
        focused.style.border_color,
        Some(
            nana_ui_core::SemanticPalette::dark()
                .border_strong
                .as_rgba_array()
        ),
    );
}

#[test]
fn component_size_kind_and_fallback_geometry_preserve_design_contracts() {
    for size in [
        nana_ui_core::ControlSize::Small,
        nana_ui_core::ControlSize::Medium,
        nana_ui_core::ControlSize::Large,
    ] {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let switch = context
            .create_component(
                document,
                Switch::new("Automatic updates", false)
                    .hint("Runs in the background")
                    .size(size),
            )
            .unwrap();
        let range = context
            .create_component(
                document,
                RangeField::new(0.7, 0.0, 1.0, 0.1)
                    .unwrap()
                    .label("Opacity")
                    .unit("%")
                    .size(size),
            )
            .unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(380.0, 120.0))
            .unwrap();
        assert_eq!(
            context
                .world()
                .layout_box(switch.stable_id())
                .unwrap()
                .width,
            380.0
        );
        let crate::ComponentGeometry::Switch { label, control, .. } = context
            .world()
            .component_geometry(switch.stable_id())
            .unwrap()
        else {
            panic!("switch geometry expected");
        };
        assert_eq!(label.font_size, size.text_size());
        assert!(label.bounds.x + label.bounds.width <= control.x);
        let switch_interaction = context
            .world()
            .node_style(switch.stable_id())
            .unwrap()
            .interaction;
        assert_ne!(switch_interaction.hovered, switch_interaction.pressed);
        assert_ne!(switch_interaction.pressed, switch_interaction.focused);
        let crate::ComponentGeometry::Range {
            label: Some(label),
            value,
            unit: Some(unit),
            track,
        } = context
            .world()
            .component_geometry(range.stable_id())
            .unwrap()
        else {
            panic!("range geometry expected");
        };
        assert_eq!(label.font_size, size.text_size());
        assert!(label.bounds.x + label.bounds.width <= track.x);
        assert!(track.x + track.width <= value.bounds.x);
        assert!(value.bounds.x + value.bounds.width <= unit.bounds.x + 0.01);
        assert_eq!(
            context.world().standard_visual(range.stable_id()),
            Some(StandardVisual::Range {
                label: Some(Arc::from("Opacity")),
                value: Arc::from("0.7"),
                unit: Some(Arc::from("%")),
                size,
                ratio: 0.7,
                invalid: false,
            })
        );
    }

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    for kind in [
        nana_ui_core::CardKind::Surface,
        nana_ui_core::CardKind::Outlined,
        nana_ui_core::CardKind::Raised,
        nana_ui_core::CardKind::Flat,
        nana_ui_core::CardKind::Selected,
    ] {
        let card = context
            .create_component(document, Card::new().kind(kind))
            .unwrap();
        let (background, border, border_width) = {
            let style = context.world().node_style(card.stable_id()).unwrap();
            (style.background, style.border, style.layout.border_width)
        };
        context
            .layout_document(document, crate::LayoutViewport::new(240.0, 120.0))
            .unwrap();
        let crate::ComponentGeometry::Card { elevation, .. } = context
            .world()
            .component_geometry(card.stable_id())
            .unwrap()
        else {
            panic!("card geometry expected");
        };
        assert_eq!(
            elevation,
            (kind == nana_ui_core::CardKind::Raised).then_some(
                crate::ComponentElevation::surface_shadow(nana_ui_core::ThemeMode::Dark)
            )
        );
        assert_eq!(
            background,
            match kind {
                nana_ui_core::CardKind::Surface | nana_ui_core::CardKind::Raised => {
                    Some(nana_ui_core::SemanticColorRole::Surface)
                }
                nana_ui_core::CardKind::Selected => {
                    Some(nana_ui_core::SemanticColorRole::Selected)
                }
                nana_ui_core::CardKind::Outlined | nana_ui_core::CardKind::Flat => None,
            }
        );
        assert_eq!(
            (border, border_width),
            match kind {
                nana_ui_core::CardKind::Outlined => {
                    (Some(nana_ui_core::SemanticColorRole::Border), Some(1.0))
                }
                nana_ui_core::CardKind::Selected => {
                    (Some(nana_ui_core::SemanticColorRole::BorderSoft), Some(1.0))
                }
                _ => (None, Some(0.0)),
            }
        );
    }

    let item = context
        .create_component(document, ListItem::new("Camera"))
        .unwrap();
    let item_interaction = context
        .world()
        .node_style(item.stable_id())
        .unwrap()
        .interaction;
    assert_ne!(item_interaction.selected, item_interaction.selected_hovered);
    let leading = context.create_component(document, Text::new("L")).unwrap();
    let trailing = context.create_component(document, Text::new("T")).unwrap();
    context.append_child(item, leading).unwrap();
    context.append_child(item, trailing).unwrap();
    context
        .set_list_item_slots(
            item,
            ListItemSlots {
                leading: Some(leading.stable_id()),
                content: None,
                trailing: Some(trailing.stable_id()),
            },
        )
        .unwrap();
    context
        .layout_document(document, crate::LayoutViewport::new(240.0, 120.0))
        .unwrap();
    let crate::ComponentGeometry::ListItem {
        leading: Some(leading),
        content: Some(content),
        trailing: Some(trailing),
        detail: None,
    } = context
        .world()
        .component_geometry(item.stable_id())
        .unwrap()
    else {
        panic!("list item fallback geometry expected");
    };
    assert!(leading.x + leading.width <= content.x);
    assert!(content.x + content.width <= trailing.x);

    let disabled_range = context
        .create_component(
            document,
            RangeField::new(0.5, 0.0, 1.0, 0.1)
                .unwrap()
                .label("Opacity")
                .disabled(true),
        )
        .unwrap();
    assert_eq!(
        context
            .world()
            .node_style(disabled_range.stable_id())
            .unwrap()
            .interaction
            .disabled
            .foreground,
        Some(nana_ui_core::SemanticColorRole::Muted)
    );
}

#[test]
fn observer_view_receives_source_event_and_owns_nested_events() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let source = context
        .create_view(document, NodeKind::Text, Counter { value: 0 })
        .unwrap();
    let observer = context
        .create_view(document, NodeKind::Text, Counter { value: 0 })
        .unwrap();
    context
        .observe(source, observer, |view, event: &Increment, cx| {
            view.value += event.0;
            cx.emit(Cascade);
        })
        .unwrap();
    context
        .on(observer, |view, _event: &Cascade, _cx| view.value += 1)
        .unwrap();
    context
        .update(source, |_view, cx| cx.emit(Increment(4)))
        .unwrap();
    assert_eq!(context.read(observer, |view| view.value).unwrap(), 5);
}

#[test]
fn action_context_extension_and_view_removal_have_explicit_ownership() {
    struct TestExtension {
        installed: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl UiExtension for TestExtension {
        fn name(&self) -> &'static str {
            "test.extension"
        }

        fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
            let installed = Arc::clone(&self.installed);
            registrar.register_action(
                "counter.increment",
                ContextPredicate::always().all_of(["editor"]),
                move |_| {
                    installed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Ok(())
                },
            )
        }
    }

    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let extension = TestExtension {
        installed: Arc::clone(&count),
    };
    let mut context = AppContext::new();
    context.install(&extension).unwrap();
    assert_eq!(
        context.install(&extension),
        Err(FrameworkError::DuplicateExtension("test.extension".into()))
    );
    let action = ActionId::new("counter.increment");
    assert_eq!(
        context.dispatch_action(&action, &KeyContext::default()),
        Err(FrameworkError::ActionUnavailable(action.clone()))
    );
    context
        .dispatch_action(&action, &KeyContext::new(["editor"]))
        .unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 1);

    let entity = context
        .create_view(
            DocumentId::new(1).unwrap(),
            NodeKind::Document,
            Counter { value: 9 },
        )
        .unwrap();
    let child = context
        .create_view(
            DocumentId::new(1).unwrap(),
            NodeKind::Text,
            Counter { value: 3 },
        )
        .unwrap();
    context
        .update(entity, |_, cx| {
            cx.mutations()
                .insert(entity.stable_id(), child.stable_id(), None);
        })
        .unwrap();
    let removed = context.remove_view(entity).unwrap();
    assert_eq!(removed.value, 9);
    assert!(!context.world().contains(entity.stable_id()));
    assert_eq!(
        context.read(child, |view| view.value),
        Err(FrameworkError::MissingView(child.stable_id()))
    );
}

#[test]
fn recursive_events_are_bounded_per_update() {
    let mut context = AppContext::new();
    let entity = context
        .create_view(
            DocumentId::new(1).unwrap(),
            NodeKind::Document,
            Counter { value: 0 },
        )
        .unwrap();
    context
        .on(entity, |_view, _event: &Cascade, cx| cx.emit(Cascade))
        .unwrap();
    assert_eq!(
        context.update(entity, |_view, cx| cx.emit(Cascade)),
        Err(FrameworkError::EventOverflow(entity.stable_id()))
    );
    assert_eq!(context.read(entity, |view| view.value).unwrap(), 0);
}

#[test]
fn extension_registration_is_atomic_on_conflict() {
    struct Conflict;
    impl UiExtension for Conflict {
        fn name(&self) -> &'static str {
            "conflict.extension"
        }

        fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
            registrar.register_action("unique.action", ContextPredicate::always(), |_| Ok(()))?;
            registrar.register_action("existing.action", ContextPredicate::always(), |_| Ok(()))
        }
    }

    let mut context = AppContext::new();
    context
        .register_action("existing.action", ContextPredicate::always(), |_| Ok(()))
        .unwrap();
    assert_eq!(
        context.install(&Conflict),
        Err(FrameworkError::DuplicateAction(ActionId::new(
            "existing.action"
        )))
    );
    assert_eq!(
        context.dispatch_action(&ActionId::new("unique.action"), &KeyContext::default()),
        Err(FrameworkError::MissingAction(ActionId::new(
            "unique.action"
        )))
    );
}

#[test]
fn presenter_extension_installs_onto_the_world() {
    struct Keyword;
    impl crate::TextPresenter for Keyword {
        fn name(&self) -> &'static str {
            "keyword"
        }

        fn present(&self, text: &str, _request: &crate::HighlightRequest) -> Vec<crate::TextSpan> {
            text.match_indices("fn")
                .map(|(start, token)| crate::TextSpan {
                    start,
                    end: start + token.len(),
                    color: nana_ui_core::SemanticColorRole::Accent,
                })
                .collect()
        }
    }
    struct HighlightExt;
    impl UiExtension for HighlightExt {
        fn name(&self) -> &'static str {
            "test.highlight"
        }

        fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
            registrar.register_presenter(Box::new(Keyword))
        }
    }

    let mut context = AppContext::new();
    context.install(&HighlightExt).unwrap();
    assert_eq!(
        context.install(&HighlightExt),
        Err(FrameworkError::DuplicateExtension("test.highlight".into()))
    );
    assert_eq!(
        context.register_presenter(Box::new(Keyword)),
        Err(FrameworkError::World(UiWorldError::DuplicatePresenter(
            "keyword".into()
        )))
    );
    let mut request = crate::HighlightRequest::highlight("rs");
    request.presenter = Arc::from("keyword");
    let entity = context
        .create_component(
            DocumentId::new(1).unwrap(),
            TextArea {
                highlight: Some(request),
                ..TextArea::new("fn main")
            },
        )
        .unwrap();
    assert_eq!(
        context
            .world()
            .highlight_request(entity.stable_id())
            .map(|request| request.language.as_ref()),
        Some("rs")
    );
    context
        .resolve_presentations(&[entity.stable_id()])
        .unwrap();
    assert_eq!(
        context
            .world()
            .text_presentation(entity.stable_id())
            .map(|presentation| presentation.spans.len()),
        Some(1)
    );
}

#[test]
fn builtin_and_plugin_components_share_one_registry() {
    let mut context = AppContext::new();
    assert!(context.resolve_component_tag("button").is_some());
    assert_eq!(
        context
            .resolve_component_tag("nana-gpu")
            .map(ComponentTypeId::as_str),
        Some("nana.gpu")
    );
    assert_eq!(
        context
            .resolve_component_tag("gpu-view")
            .map(ComponentTypeId::as_str),
        Some("nana.gpu-view")
    );
    assert_eq!(
        context.resolve_component_tag("chip"),
        None,
        "chip is a Button variant, not a registry tag"
    );
    assert_eq!(
        context.resolve_component_tag("virtual-list"),
        None,
        "virtual windows use scroll-view, not a second type"
    );
    assert_eq!(
        context
            .resolve_component_tag("nana-button")
            .map(ComponentTypeId::as_str),
        Some("nana.button")
    );
    assert_eq!(
        context
            .resolve_component_tag("select")
            .map(ComponentTypeId::as_str),
        Some("nana.select")
    );
    assert_eq!(
        context
            .resolve_component_tag("nana-select")
            .map(ComponentTypeId::as_str),
        Some("nana.select")
    );
    assert_eq!(
        context
            .resolve_component_tag("tabs")
            .map(ComponentTypeId::as_str),
        Some("nana.tabs")
    );
    assert_eq!(
        context
            .resolve_component_tag("dock")
            .map(ComponentTypeId::as_str),
        Some("nana.dock")
    );
    assert_eq!(
        context
            .resolve_component_tag("form-field")
            .map(ComponentTypeId::as_str),
        Some("nana.form-field")
    );
    assert_eq!(
        context
            .resolve_component_tag("nana-form-field")
            .map(ComponentTypeId::as_str),
        Some("nana.form-field")
    );
    assert!(
        context.resolve_component_tag("form").is_none(),
        "HTML form stays a layout box; nana-form-field owns form-field"
    );
    assert!(
        context.resolve_component_tag("search").is_none(),
        "HTML search is a landmark; SearchDropdown owns search-dropdown"
    );
    assert_eq!(
        context
            .resolve_component_tag("search-dropdown")
            .map(ComponentTypeId::as_str),
        Some("nana.search-dropdown")
    );
    assert_eq!(
        context
            .resolve_component_tag("nana-search-dropdown")
            .map(ComponentTypeId::as_str),
        Some("nana.search-dropdown")
    );

    #[derive(Clone)]
    struct ProbeCard {
        title: String,
    }
    impl ComponentView for ProbeCard {
        fn node_kind(&self) -> NodeKind {
            NodeKind::Element {
                tag: "probe-card".into(),
            }
        }
        fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
            if world.text(id) != Some(self.title.as_str()) {
                mutations.set_text(
                    id,
                    crate::TextContent {
                        value: self.title.clone(),
                    },
                );
            }
        }
    }
    impl crate::RegisterableComponent for ProbeCard {
        const TYPE_ID: &'static str = "test.probe-card";
        const TAGS: &'static [&'static str] = &["nana-probe-card", "probe-card"];
        fn from_semantic(spec: &crate::SemanticSpec<'_>) -> Self {
            Self {
                title: spec
                    .attr("handle")
                    .unwrap_or_else(|| spec.display_label())
                    .to_owned(),
            }
        }
    }
    struct ProbePlugin;
    impl UiExtension for ProbePlugin {
        fn name(&self) -> &'static str {
            "test.probe"
        }
        fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
            registrar.register_component::<ProbeCard>()
        }
    }

    context.install(&ProbePlugin).unwrap();
    assert_eq!(
        context
            .resolve_component_tag("nana-probe-card")
            .map(ComponentTypeId::as_str),
        Some("test.probe-card")
    );

    let document = DocumentId::new(1).unwrap();
    let button = context
        .create_component(document, Button::new("Save"))
        .unwrap();
    assert_eq!(
        context
            .world()
            .component_type(button.stable_id())
            .map(ComponentTypeId::as_str),
        Some("nana.button")
    );
    let select_type = context.resolve_component_tag("select").unwrap().clone();
    let select_layout = std::sync::Arc::new(nana_ui_core::LayoutStyle::default());
    let select_spec = crate::SemanticSpec::from_parts(&select_type, &select_layout);
    let select = context
        .create_component(document, Select::from_semantic(&select_spec))
        .unwrap();
    assert_eq!(
        context
            .world()
            .component_type(select.stable_id())
            .map(ComponentTypeId::as_str),
        Some("nana.select")
    );
    let dock = context
        .create_component(
            document,
            crate::Dock::new(crate::DockNode::item("dock", None)),
        )
        .unwrap();
    assert_eq!(
        context
            .world()
            .component_type(dock.stable_id())
            .map(ComponentTypeId::as_str),
        Some("nana.dock")
    );

    let id = StableNodeId::new(42).unwrap();
    let mut queue = MutationQueue::new();
    queue.create(
        id,
        document,
        NodeKind::Element {
            tag: "probe-card".into(),
        },
    );
    context.commit_mutations(queue).unwrap();
    let type_id = context.resolve_component_tag("probe-card").unwrap().clone();
    let layout = std::sync::Arc::new(nana_ui_core::LayoutStyle::default());
    let spec = crate::SemanticSpec {
        label: "User",
        ..crate::SemanticSpec::from_parts(&type_id, &layout)
    };
    let mut mutations = MutationQueue::new();
    assert_eq!(
        context.bind_semantic(id, &spec, &mut mutations).unwrap(),
        crate::ComponentBindKind::Projected
    );
    context.commit_mutations(mutations).unwrap();
    assert_eq!(context.world().text(id), Some("User"));
    assert_eq!(
        context
            .world()
            .component_type(id)
            .map(ComponentTypeId::as_str),
        Some("test.probe-card")
    );
}

/// Documented containers and chrome must carry a type identity, or Vue tag
/// resolution and devtools cannot name the node.
#[test]
#[cfg(feature = "charts")]
fn documented_containers_and_chrome_carry_a_type_identity() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    for (tag, type_id) in [
        ("list", "nana.list"),
        ("scroll-view", "nana.scroll-view"),
        ("table", "nana.table"),
        ("tr", "nana.table-row"),
        ("td", "nana.table-cell"),
        ("reorder-list", "nana.reorder-list"),
        ("time-series-chart", "nana.time-series-chart"),
        ("desktop-shell", "nana.desktop-shell"),
        ("app-title-bar", "nana.app-title-bar"),
        ("pane-chrome", "nana.pane-chrome"),
        ("sidebar-section", "nana.sidebar-section"),
        ("sidebar-footer", "nana.sidebar-footer"),
        (
            "settings-collapsible-card",
            "nana.settings-collapsible-card",
        ),
    ] {
        assert_eq!(
            context
                .resolve_component_tag(tag)
                .map(ComponentTypeId::as_str),
            Some(type_id),
            "tag `{tag}` must resolve"
        );
    }
    assert!(
        context.resolve_component_tag("scroll").is_none(),
        "aliases are pruned; scroll-view keeps the single tag"
    );

    let list = context
        .create_component(document, crate::List::new())
        .unwrap();
    assert_eq!(
        context
            .world()
            .component_type(list.stable_id())
            .map(ComponentTypeId::as_str),
        Some("nana.list")
    );
    let section = context
        .create_component(document, crate::SidebarSection::new("Files"))
        .unwrap();
    assert_eq!(
        context
            .world()
            .component_type(section.stable_id())
            .map(ComponentTypeId::as_str),
        Some("nana.sidebar-section")
    );
    let chart = context
        .create_component(document, crate::TimeSeriesChart::new([1.0, 2.0]))
        .unwrap();
    assert_eq!(
        context
            .world()
            .component_type(chart.stable_id())
            .map(ComponentTypeId::as_str),
        Some("nana.time-series-chart")
    );
}

#[test]
fn plugin_component_registration_is_atomic_on_conflict() {
    #[derive(Clone)]
    struct StealButton;
    impl ComponentView for StealButton {
        fn node_kind(&self) -> NodeKind {
            NodeKind::Element {
                tag: "button".into(),
            }
        }
        fn project(&self, _id: StableNodeId, _world: &UiWorld, _mutations: &mut MutationQueue) {}
    }
    impl crate::RegisterableComponent for StealButton {
        const TYPE_ID: &'static str = "nana.button";
        const TAGS: &'static [&'static str] = &["stolen"];
        fn from_semantic(_spec: &crate::SemanticSpec<'_>) -> Self {
            Self
        }
    }
    struct Conflict;
    impl UiExtension for Conflict {
        fn name(&self) -> &'static str {
            "conflict.components"
        }
        fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
            registrar.register_component::<StealButton>()
        }
    }

    let mut context = AppContext::new();
    assert_eq!(
        context.install(&Conflict),
        Err(FrameworkError::DuplicateComponentType("nana.button".into()))
    );
    assert!(context.resolve_component_tag("stolen").is_none());
    assert!(context.resolve_component_tag("button").is_some());
}

#[cfg(feature = "syntax-highlighting")]
#[test]
fn new_context_installs_the_official_highlight_presenter() {
    let mut context = AppContext::new();
    assert!(context.world().has_presenter(crate::HIGHLIGHT_PRESENTER));
    let entity = context
        .create_component(
            DocumentId::new(1).unwrap(),
            crate::HostedTextarea::new("fn main() {}", "rs"),
        )
        .unwrap();
    assert_eq!(
        context
            .world()
            .highlight_request(entity.stable_id())
            .map(|request| request.presenter.as_ref()),
        Some(crate::HIGHLIGHT_PRESENTER)
    );
    context
        .resolve_presentations(&[entity.stable_id()])
        .unwrap();
    let spans = context
        .world()
        .text_presentation(entity.stable_id())
        .map(|presentation| presentation.spans.clone())
        .unwrap_or_default();
    assert!(
        spans.iter().any(|span| {
            matches!(
                span.color,
                nana_ui_core::SemanticColorRole::Accent
                    | nana_ui_core::SemanticColorRole::AccentStrong
            ) && &"fn main() {}"[span.start..span.end] == "fn"
        }),
        "default Syntect presenter must color rust `fn`, got {spans:?}"
    );
}

struct One<T>(Option<T>);

impl<T: Unpin> Stream for One<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<T>> {
        Poll::Ready(self.0.take())
    }
}

#[test]
fn task_and_subscription_preserve_host_owned_async_work() {
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut future = Task::ready(2).map(|value| value + 1).into_future();
    assert_eq!(future.as_mut().poll(&mut cx), Poll::Ready(3));

    let subscription = Subscription::new("window.events", One(Some(7)));
    assert_eq!(subscription.id(), "window.events");
    let mut stream = subscription.into_stream();
    assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(7)));
    assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(None));
}

#[test]
fn virtual_list_materializes_only_visible_items_and_reuses_overlap() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let list = context.create_component(document, List::new()).unwrap();
    let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 10_000));
    let mut items = VirtualListItems::<usize, Text>::default();

    let first = context
        .materialize_virtual_list(
            list,
            &mut items,
            &layout,
            0.0,
            100.0,
            20.0,
            |index| index,
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    assert!(first.range.len() < 10);
    assert_eq!(
        context
            .world()
            .node(list.stable_id())
            .unwrap()
            .children
            .len(),
        first.range.len()
    );
    let overlap_key = first.range.end - 1;
    let overlap_entity = items.entity(&overlap_key).unwrap();
    let removed_key = first.range.start;
    let removed_entity = items.entity(&removed_key).unwrap();

    let next = context
        .materialize_virtual_list(
            list,
            &mut items,
            &layout,
            80.0,
            100.0,
            20.0,
            |index| index,
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    assert!(next.range.contains(&overlap_key));
    assert_eq!(items.entity(&overlap_key), Some(overlap_entity));
    assert!(!context.world().contains(removed_entity.stable_id()));
    assert_eq!(items.mounted_keys(), next.range.clone().collect::<Vec<_>>());
    let generation = context.world().generation();

    context
        .materialize_virtual_list(
            list,
            &mut items,
            &layout,
            80.0,
            100.0,
            20.0,
            |index| index,
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    assert_eq!(context.world().generation(), generation);
}

#[test]
fn virtual_list_rejects_foreign_item_ownership_without_mutating() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let list = context.create_component(document, List::new()).unwrap();
    let other_list = context.create_component(document, List::new()).unwrap();
    let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 100));
    let mut items = VirtualListItems::<usize, Text>::default();

    let first = context
        .materialize_virtual_list(
            list,
            &mut items,
            &layout,
            0.0,
            100.0,
            0.0,
            |index| index,
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    let moved = items.entity(&first.range.start).unwrap();
    context.append_child(other_list, moved).unwrap();
    let generation = context.world().generation();

    assert_eq!(
        context.materialize_virtual_list(
            list,
            &mut items,
            &layout,
            200.0,
            100.0,
            0.0,
            |index| index,
            |index, _| Text::new(format!("row {index}")),
        ),
        Err(FrameworkError::InvalidVirtualization)
    );
    assert_eq!(context.world().generation(), generation);
    assert_eq!(
        context.world().node(moved.stable_id()).unwrap().parent,
        Some(other_list.stable_id())
    );
    assert!(context.world().contains(moved.stable_id()));
}

#[test]
fn virtual_table_materializes_a_bounded_grid_and_reuses_both_axes() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let table = context.create_component(document, Table::new()).unwrap();
    let layout = VirtualTableLayout::new(
        std::iter::repeat_n(20.0, 10_000),
        (0..100).map(|index| nana_ui_core::TableColumn::new(index.to_string(), 80.0)),
    );
    let mut items = VirtualTableItems::<usize, usize>::default();

    let first = context
        .materialize_virtual_table(
            table,
            &mut items,
            &layout,
            (0.0, 0.0),
            (160.0, 100.0),
            (0.0, 20.0),
            |index| index,
            |index| index,
            |_index, _| TableRow::new(),
            |row, _, column, _| TableCell::new(format!("{row}:{column}")),
        )
        .unwrap();
    assert!(first.rows.range.len() < 10);
    assert!(first.columns.range.len() < 5);
    let overlap_row = first.rows.range.end - 1;
    let overlap_column = first.columns.range.end - 1;
    let overlap_row_entity = items.row_entity(&overlap_row).unwrap();
    let overlap_cell_entity = items.cell_entity(&overlap_row, &overlap_column).unwrap();
    let removed_row = first.rows.range.start;
    let removed_cell = items
        .cell_entity(&removed_row, &first.columns.range.start)
        .unwrap();

    let next = context
        .materialize_virtual_table(
            table,
            &mut items,
            &layout,
            (80.0, 40.0),
            (160.0, 100.0),
            (0.0, 20.0),
            |index| index,
            |index| index,
            |_index, _| TableRow::new(),
            |row, _, column, _| TableCell::new(format!("{row}:{column}")),
        )
        .unwrap();
    assert_eq!(items.row_entity(&overlap_row), Some(overlap_row_entity));
    assert_eq!(
        items.cell_entity(&overlap_row, &overlap_column),
        Some(overlap_cell_entity)
    );
    assert!(!context.world().contains(removed_cell.stable_id()));
    assert_eq!(
        items.mounted_rows(),
        next.rows.range.clone().collect::<Vec<_>>()
    );
    assert_eq!(
        items.mounted_columns(),
        next.columns.range.clone().collect::<Vec<_>>()
    );
    let retained_cells = next.rows.range.len() * next.columns.range.len();
    assert_eq!(
        next.rows
            .range
            .clone()
            .map(|row| context
                .world()
                .node(items.row_entity(&row).unwrap().stable_id())
                .unwrap()
                .children
                .len())
            .sum::<usize>(),
        retained_cells
    );
    let generation = context.world().generation();
    context
        .materialize_virtual_table(
            table,
            &mut items,
            &layout,
            (80.0, 40.0),
            (160.0, 100.0),
            (0.0, 20.0),
            |index| index,
            |index| index,
            |_index, _| TableRow::new(),
            |row, _, column, _| TableCell::new(format!("{row}:{column}")),
        )
        .unwrap();
    assert_eq!(context.world().generation(), generation);
}

#[test]
fn virtual_table_rejects_foreign_row_ownership_without_mutating() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let table = context.create_component(document, Table::new()).unwrap();
    let other_table = context.create_component(document, Table::new()).unwrap();
    let layout = VirtualTableLayout::new(
        std::iter::repeat_n(20.0, 100),
        (0..10).map(|index| nana_ui_core::TableColumn::new(index.to_string(), 80.0)),
    );
    let mut items = VirtualTableItems::<usize, usize>::default();
    let window = context
        .materialize_virtual_table(
            table,
            &mut items,
            &layout,
            (0.0, 0.0),
            (160.0, 100.0),
            (0.0, 0.0),
            |index| index,
            |index| index,
            |_index, _| TableRow::new(),
            |row, _, column, _| TableCell::new(format!("{row}:{column}")),
        )
        .unwrap();
    let moved = items.row_entity(&window.rows.range.start).unwrap();
    context.append_child(other_table, moved).unwrap();
    let generation = context.world().generation();

    assert_eq!(
        context.materialize_virtual_table(
            table,
            &mut items,
            &layout,
            (0.0, 200.0),
            (160.0, 100.0),
            (0.0, 0.0),
            |index| index,
            |index| index,
            |_index, _| TableRow::new(),
            |row, _, column, _| TableCell::new(format!("{row}:{column}")),
        ),
        Err(FrameworkError::InvalidVirtualization)
    );
    assert_eq!(context.world().generation(), generation);
    assert_eq!(
        context.world().node(moved.stable_id()).unwrap().parent,
        Some(other_table.stable_id())
    );
}

#[test]
fn virtual_tree_materializes_only_visible_rows_and_reuses_overlap_on_scroll_and_expand() {
    const ROW: f32 = 20.0;
    const VIEWPORT: f32 = 100.0;
    const OVERSCAN: f32 = 20.0;
    let cap = VirtualListLayout::uniform_window_item_cap(VIEWPORT, OVERSCAN, ROW);
    assert!(cap < 10_000);

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let tree = context.create_component(document, List::new()).unwrap();
    let mut keys = (0..10_000).collect::<Vec<_>>();
    let mut layout = VirtualTreeLayout::uniform(ROW, std::iter::repeat_n(0, keys.len()));
    let mut items = VirtualTreeItems::<usize, Text>::default();

    let first = context
        .materialize_virtual_tree(
            tree,
            &mut items,
            &layout,
            0.0,
            VIEWPORT,
            OVERSCAN,
            |index| keys[index],
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    assert!(first.range.len() <= cap);
    assert_eq!(
        context
            .world()
            .node(tree.stable_id())
            .unwrap()
            .children
            .len(),
        first.range.len()
    );
    let overlap_key = keys[first.range.end - 1];
    let overlap_entity = items.entity(&overlap_key).unwrap();
    let removed_key = keys[first.range.start];
    let removed_entity = items.entity(&removed_key).unwrap();

    let next = context
        .materialize_virtual_tree(
            tree,
            &mut items,
            &layout,
            80.0,
            VIEWPORT,
            OVERSCAN,
            |index| keys[index],
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    assert!(next.range.len() <= cap);
    assert!(items.mounted_keys().contains(&overlap_key));
    assert_eq!(items.entity(&overlap_key), Some(overlap_entity));
    assert!(!context.world().contains(removed_entity.stable_id()));
    assert_eq!(
        items.mounted_keys(),
        next.range
            .clone()
            .map(|index| keys[index])
            .collect::<Vec<_>>()
    );

    let parent = keys.iter().position(|key| *key == overlap_key).unwrap();
    let child_keys = [1_000_000usize, 1_000_001];
    assert!(layout.expand(
        parent,
        child_keys.map(|_| nana_ui_core::VirtualTreeRow {
            extent: ROW,
            descendant_count: 0,
        })
    ));
    keys.splice(parent + 1..parent + 1, child_keys);
    let expanded = context
        .materialize_virtual_tree(
            tree,
            &mut items,
            &layout,
            80.0,
            VIEWPORT,
            OVERSCAN,
            |index| keys[index],
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    assert!(expanded.range.len() <= cap);
    assert_eq!(items.entity(&overlap_key), Some(overlap_entity));
    assert!(
        context
            .world()
            .node(tree.stable_id())
            .unwrap()
            .children
            .len()
            <= cap
    );
    assert!(items.entity(&child_keys[0]).is_some());
    let generation = context.world().generation();
    context
        .materialize_virtual_tree(
            tree,
            &mut items,
            &layout,
            80.0,
            VIEWPORT,
            OVERSCAN,
            |index| keys[index],
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    assert_eq!(context.world().generation(), generation);
}

#[test]
fn virtual_tree_expand_keeps_live_children_below_geometric_cap() {
    const ROW: f32 = 20.0;
    const VIEWPORT: f32 = 100.0;
    const OVERSCAN: f32 = 20.0;
    const DESCENDANTS: usize = 10_000;
    let cap = VirtualListLayout::uniform_window_item_cap(VIEWPORT, OVERSCAN, ROW);
    assert!(cap < DESCENDANTS);

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let tree = context.create_component(document, List::new()).unwrap();
    let mut keys = vec![0usize, 1, 2];
    let mut layout = VirtualTreeLayout::uniform(ROW, [0, 0, 0]);
    let mut items = VirtualTreeItems::<usize, Text>::default();

    context
        .materialize_virtual_tree(
            tree,
            &mut items,
            &layout,
            0.0,
            VIEWPORT,
            OVERSCAN,
            |index| keys[index],
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();

    let child_keys = (1_000_000..1_000_000 + DESCENDANTS).collect::<Vec<_>>();
    assert!(layout.expand(
        0,
        child_keys.iter().map(|_| nana_ui_core::VirtualTreeRow {
            extent: ROW,
            descendant_count: 0,
        })
    ));
    keys.splice(1..1, child_keys);
    let descendant_count = layout
        .descendant_count(0)
        .expect("expanded parent keeps a descendant count");
    assert_eq!(descendant_count, DESCENDANTS);

    context
        .materialize_virtual_tree(
            tree,
            &mut items,
            &layout,
            0.0,
            VIEWPORT,
            OVERSCAN,
            |index| keys[index],
            |index, _| Text::new(format!("row {index}")),
        )
        .unwrap();
    let live = context
        .world()
        .node(tree.stable_id())
        .unwrap()
        .children
        .len();
    assert!(
        live <= cap,
        "live List children {live} exceed geometric cap {cap}"
    );
    assert!(
        live < descendant_count,
        "live List children {live} mounted every expanded descendant ({descendant_count})"
    );
}

#[test]
fn stack_presets_express_row_and_column_layout() {
    let row = Stack::row(8.0).node_style();
    let layout = row.layout;
    assert_eq!(layout.direction, Some(nana_ui_core::FlexDirection::Row));
    assert_eq!(layout.gap, Some(nana_ui_core::LengthSpec::Px(8.0)));
    assert_eq!(layout.align_items, nana_ui_core::AlignSpec::Center);
    assert_eq!(layout.width, Some(nana_ui_core::LengthSpec::Shrink));

    let fill_column = Stack::fill_column(0.0).node_style();
    assert_eq!(
        fill_column.layout.direction,
        Some(nana_ui_core::FlexDirection::Column)
    );
    assert_eq!(
        fill_column.layout.width,
        Some(nana_ui_core::LengthSpec::Fill)
    );
    assert_eq!(
        fill_column.layout.height,
        Some(nana_ui_core::LengthSpec::Fill)
    );
    assert_eq!(fill_column.layout.flex_grow, Some(1.0));
    assert_eq!(fill_column.layout.flex_shrink, Some(1.0));

    let outlined = Stack::column(4.0)
        .outline(nana_ui_core::SemanticColorRole::Border, 1.0)
        .node_style();
    assert_eq!(
        outlined.border,
        Some(nana_ui_core::SemanticColorRole::Border)
    );
    assert_eq!(outlined.layout.border_width, Some(1.0));
}

#[test]
fn card_kind_defaults_yield_to_explicit_style() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let surface = context.create_component(document, Card::new()).unwrap();
    let style = context
        .world()
        .node_style(surface.stable_id())
        .cloned()
        .unwrap();
    assert_eq!(
        style.background,
        Some(nana_ui_core::SemanticColorRole::Surface)
    );
    assert_eq!(style.border, None);
    assert_eq!(style.layout.border_width, Some(0.0));

    let custom = NodeStyle::default().outline(nana_ui_core::SemanticColorRole::Border, 2.0);
    let outlined = context
        .create_component(
            document,
            Card::new()
                .kind(nana_ui_core::CardKind::Outlined)
                .style(custom),
        )
        .unwrap();
    let style = context
        .world()
        .node_style(outlined.stable_id())
        .cloned()
        .unwrap();
    assert_eq!(
        style.border,
        Some(nana_ui_core::SemanticColorRole::Border),
        "用户显式设置的边框不得被 kind 默认值覆盖"
    );
    assert_eq!(
        style.layout.border_width,
        Some(2.0),
        "用户显式设置的边框宽度不得被 kind 默认值覆盖"
    );
}

/// Memoizes a probe of the retained subtree (own child count) into text
/// state: exactly the stale-snapshot shape that `wants_child_reproject`
/// exists for. Two types share this projection; only one opts in.
#[derive(Debug, Clone, Default)]
struct ReprojectProbe;

#[derive(Debug, Clone, Default)]
struct PlainProbe;

fn project_child_count(id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
    let count = world.node(id).map(|node| node.children.len()).unwrap_or(0);
    let value = count.to_string();
    if world.text(id) != Some(value.as_str()) {
        mutations.set_text(id, crate::TextContent { value });
    }
}

impl ComponentView for ReprojectProbe {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "reproject-probe".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_child_count(id, world, mutations);
    }

    fn wants_child_reproject() -> bool {
        true
    }
}

impl ComponentView for PlainProbe {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "plain-probe".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_child_count(id, world, mutations);
    }
}

#[test]
fn opt_in_component_reprojects_when_children_mount() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let probe: Entity<ReprojectProbe> = context.create_component(document, ReprojectProbe).unwrap();
    assert_eq!(context.world().text(probe.stable_id()), Some("0"));

    context
        .build_child(probe, |builder| {
            builder.child("row", crate::Text::new("row"));
        })
        .unwrap();

    assert_eq!(
        context
            .world()
            .node(probe.stable_id())
            .expect("probe node")
            .children
            .len(),
        1
    );
    assert_eq!(
        context.world().text(probe.stable_id()),
        Some("1"),
        "child mount must rerun project for opted-in components"
    );
}

#[test]
fn opt_in_component_reprojects_when_child_detaches() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let probe: Entity<ReprojectProbe> = context.create_component(document, ReprojectProbe).unwrap();
    context
        .build_child(probe, |builder| {
            builder.child("row", crate::Text::new("row"));
        })
        .unwrap();
    assert_eq!(context.world().text(probe.stable_id()), Some("1"));

    let row = context
        .world()
        .node(probe.stable_id())
        .expect("probe node")
        .children[0];
    let mut queue = MutationQueue::new();
    queue.detach(row);
    context.commit_mutations(queue).unwrap();

    assert_eq!(
        context.world().text(probe.stable_id()),
        Some("0"),
        "child detach must rerun project for opted-in components"
    );
}

#[test]
fn component_without_opt_in_keeps_data_change_schedule() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let probe: Entity<PlainProbe> = context.create_component(document, PlainProbe).unwrap();
    assert_eq!(context.world().text(probe.stable_id()), Some("0"));

    context
        .build_child(probe, |builder| {
            builder.child("row", crate::Text::new("row"));
        })
        .unwrap();

    assert_eq!(
        context
            .world()
            .node(probe.stable_id())
            .expect("probe node")
            .children
            .len(),
        1
    );
    assert_eq!(
        context.world().text(probe.stable_id()),
        Some("0"),
        "components that do not opt in must not reproject on child structure changes"
    );
}
