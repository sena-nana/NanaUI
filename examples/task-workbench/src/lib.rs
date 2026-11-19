//! Golden L3 authoring example:
//! - `#[derive(Store)]` + `store_with_history` for field-granular state and undo/redo
//! - `each_virtual` for bounded 2,000+ row virtualization
//! - `FormField` + `ValidationMessage` + `AppContext::validity_of` form validation
//! - Unified `ActionRegistry` + `Keymap` + `MenuActionMap` + `AppMenuBar` + `ActionMenu` + `CommandPalette`

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nana_ui::runtime::view::{
    Signal, Store, StoreList, StorePath, VirtualListRef, button, checkbox, column, computed,
    entity_ref, number_input, row, signal, store, store_with_history, text, text_input,
    virtual_list_ref, when, widget, with_refs,
};
use nana_ui::runtime::{
    ActionDescriptor, ActionId, ActionMenu, ActionMenuItem, ActionRegistry, Activate, AppContext,
    AppMenuBar, CONTEXT_MODAL, CONTEXT_TEXT_INPUT, CapturedStroke, CommandPalette,
    CommandPaletteEvent, ContextPredicate, DocumentId, Entity, FormField, FrameworkError,
    KeyBinding, KeyContext, KeyModifiers, Keymap, KeymapState, LengthSpec, MenuActionMap,
    OverlayHost, PositionSpec, Stack, ValidationIntent, ValidationMessage,
};
use nana_ui::{
    ApplicationState, ApplicationWindow, Menu, MenuBar, MenuEntry, RoutedInput,
    RuntimeProgramContext, RuntimeProgramUpdate, take_menu_activations,
};
use nana_ui_platform::WindowId;

pub const ACTION_UNDO: &str = "edit.undo";
pub const ACTION_REDO: &str = "edit.redo";
pub const ACTION_ADD_TASK: &str = "task.add";
pub const ACTION_TOGGLE_PALETTE: &str = "palette.toggle";

const MENU_ID_UNDO: u32 = 101;
const MENU_ID_REDO: u32 = 102;
const MENU_ID_ADD_TASK: u32 = 103;
const MENU_ID_PALETTE: u32 = 104;

#[derive(Clone, Debug, PartialEq, Store)]
pub struct TaskItem {
    pub task_id: u64,
    pub title: String,
    pub hours: f64,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Store)]
pub struct DraftForm {
    pub title: String,
    pub hours: f64,
}

#[derive(Clone, Debug, PartialEq, Store)]
pub struct WorkbenchBoard {
    pub next_id: u64,
    pub last_action: String,
    pub tasks: Vec<TaskItem>,
}

impl WorkbenchBoard {
    pub fn seeded(count: usize) -> Self {
        let tasks = (1..=count as u64)
            .map(|task_id| TaskItem {
                task_id,
                title: format!("任务 #{task_id}: 验证虚拟列表行"),
                hours: ((task_id % 8) + 1) as f64,
                done: task_id % 5 == 0,
            })
            .collect();
        Self {
            next_id: count as u64 + 1,
            last_action: String::from("初始化列表"),
            tasks,
        }
    }
}

pub fn validate_title(title: &str) -> Option<&'static str> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        Some("任务标题不能为空")
    } else if trimmed.chars().count() < 2 {
        Some("标题至少需要 2 个字符")
    } else if trimmed.chars().count() > 80 {
        Some("标题不能超过 80 个字符")
    } else {
        None
    }
}

pub fn validate_hours(hours: f64) -> Option<&'static str> {
    if !hours.is_finite() || hours <= 0.0 {
        Some("预计工时必须大于 0 小时")
    } else if hours > 200.0 {
        Some("单条任务工时不能超过 200 小时")
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchMessage {
    Action(ActionId),
    Palette(CommandPaletteEvent),
}

pub struct WorkbenchSession {
    pub document: DocumentId,
    pub board: Store<WorkbenchBoard>,
    pub draft: Store<DraftForm>,
    pub form: Entity<Stack>,
    pub list_ref: VirtualListRef<u64>,
    pub overlay_host: Entity<OverlayHost>,
    pub menu_bar: Entity<AppMenuBar>,
    pub action_menu: Entity<ActionMenu>,
    pub action_menu_items: Vec<(ActionId, Entity<ActionMenuItem>)>,
    pub palette: Arc<Mutex<Option<Entity<CommandPalette>>>>,
    pub registry: ActionRegistry,
    pub keymap: Keymap,
    pub keymap_state: KeymapState,
    pub menu_map: MenuActionMap,
    pub submit_attempted: Signal<bool>,
    pub status: Signal<String>,
}

impl WorkbenchSession {
    pub fn key_context(&self, cx: &AppContext) -> KeyContext {
        cx.document_key_context(self.document, KeyContext::new(["workbench"]))
    }

    pub fn sync_menus(&self, cx: &mut AppContext) -> Result<(), FrameworkError> {
        let context = self.key_context(cx);
        let mut bar = cx.read(self.menu_bar, |bar| bar.bar.clone())?;
        self.menu_map
            .sync_menu_bar(&mut bar, &self.registry, &context, Some(&self.keymap));
        cx.update_component(self.menu_bar, |menu_bar, _| {
            menu_bar.set_bar(bar);
        })?;
        for (action_id, entity) in &self.action_menu_items {
            if let Some(updated) =
                self.registry
                    .action_menu_item(action_id, &context, Some(&self.keymap))
            {
                cx.set_component(*entity, updated)?;
            }
        }
        Ok(())
    }

    pub fn dispatch_action(
        &mut self,
        cx: &mut AppContext,
        action: &ActionId,
    ) -> Result<(), FrameworkError> {
        let context = self.key_context(cx);
        let result = cx.dispatch_registered_action(&self.registry, action, &context);
        if result.is_ok() {
            cx.flush_reactive()?;
            self.sync_menus(cx)?;
        }
        result
    }
}

fn build_registry(board: Store<WorkbenchBoard>) -> ActionRegistry {
    let mut registry = ActionRegistry::new();
    registry
        .register(
            ActionDescriptor::labeled(ACTION_ADD_TASK, "添加任务")
                .category("任务")
                .keywords(["add", "task", "submit"])
                .when(ContextPredicate::always().none_of([CONTEXT_MODAL])),
        )
        .expect("unique action id");
    registry
        .register(
            ActionDescriptor::labeled(ACTION_UNDO, "撤销")
                .category("编辑")
                .keywords(["undo", "history"])
                .when(ContextPredicate::always().none_of([CONTEXT_MODAL])),
        )
        .expect("unique action id");
    registry
        .register(
            ActionDescriptor::labeled(ACTION_REDO, "重做")
                .category("编辑")
                .keywords(["redo", "history"])
                .when(ContextPredicate::always().none_of([CONTEXT_MODAL])),
        )
        .expect("unique action id");
    registry
        .register(
            ActionDescriptor::labeled(ACTION_TOGGLE_PALETTE, "打开命令面板")
                .category("视图")
                .keywords(["command", "palette"]),
        )
        .expect("unique action id");

    registry.bind_enabled(&ActionId::new(ACTION_UNDO), move |_| board.can_undo());
    registry.bind_enabled(&ActionId::new(ACTION_REDO), move |_| board.can_redo());
    registry.bind_label(&ActionId::new(ACTION_UNDO), move |_| {
        format!("撤销: {}", board.last_action().get_untracked())
    });
    registry.bind_label(&ActionId::new(ACTION_REDO), move |_| {
        format!("重做: {}", board.last_action().get_untracked())
    });
    registry
}

fn build_keymap() -> Keymap {
    let primary = KeyModifiers::primary();
    Keymap::new([
        KeyBinding::new(ACTION_ADD_TASK, CapturedStroke::new("Enter", primary))
            .when(ContextPredicate::always().none_of([CONTEXT_MODAL])),
        KeyBinding::new(ACTION_UNDO, CapturedStroke::new("z", primary))
            .when(ContextPredicate::always().none_of([CONTEXT_MODAL, CONTEXT_TEXT_INPUT])),
        KeyBinding::new(
            ACTION_REDO,
            CapturedStroke::new(
                "z",
                KeyModifiers {
                    shift: true,
                    ..primary
                },
            ),
        )
        .when(ContextPredicate::always().none_of([CONTEXT_MODAL, CONTEXT_TEXT_INPUT])),
        KeyBinding::new(
            ACTION_TOGGLE_PALETTE,
            CapturedStroke::new(
                "p",
                KeyModifiers {
                    shift: true,
                    ..primary
                },
            ),
        ),
    ])
}

fn build_menu_map() -> MenuActionMap {
    let mut map = MenuActionMap::new();
    map.bind(MENU_ID_UNDO, ACTION_UNDO).expect("unique menu id");
    map.bind(MENU_ID_REDO, ACTION_REDO).expect("unique menu id");
    map.bind(MENU_ID_ADD_TASK, ACTION_ADD_TASK)
        .expect("unique menu id");
    map.bind(MENU_ID_PALETTE, ACTION_TOGGLE_PALETTE)
        .expect("unique menu id");
    map
}

pub fn mount_workbench(
    cx: &mut AppContext,
    document: DocumentId,
) -> Result<WorkbenchSession, FrameworkError> {
    let board_slot = std::cell::Cell::new(None);
    let draft_slot = std::cell::Cell::new(None);
    let submit_slot = std::cell::Cell::new(None);
    let status_slot = std::cell::Cell::new(None);
    let list_ref = virtual_list_ref::<u64>();
    let list_ref_for_view = list_ref.clone();

    let keymap = build_keymap();
    let menu_map = build_menu_map();

    let placeholder_menu = MenuBar::new([
        Menu::new(
            "任务",
            [
                MenuEntry::item(MENU_ID_ADD_TASK, "添加任务"),
                MenuEntry::Separator,
                MenuEntry::item(MENU_ID_PALETTE, "打开命令面板"),
            ],
        ),
        Menu::new(
            "编辑",
            [
                MenuEntry::item(MENU_ID_UNDO, "撤销"),
                MenuEntry::item(MENU_ID_REDO, "重做"),
            ],
        ),
    ]);

    let (_, (host, form, menu_bar, action_menu)) = cx.mount_view_root(document, || {
        let board = store_with_history(WorkbenchBoard::seeded(2_000), 64);
        let draft = store(DraftForm {
            title: String::new(),
            hours: 1.0,
        });
        let submit_attempted = signal(false);
        let status = signal(String::from("就绪：已载入 2000 条虚拟任务"));

        board_slot.set(Some(board));
        draft_slot.set(Some(draft));
        submit_slot.set(Some(submit_attempted));
        status_slot.set(Some(status));

        let host_ref = entity_ref::<OverlayHost>();
        let form_ref = entity_ref::<Stack>();
        let menu_bar_ref = entity_ref::<AppMenuBar>();
        let action_menu_ref = entity_ref::<ActionMenu>();

        let title_err = computed(move || {
            let value = draft.title().get();
            if submit_attempted.get() || !value.is_empty() {
                validate_title(&value)
            } else {
                None
            }
        });
        let hours_err = computed(move || {
            let value = draft.hours().get();
            if submit_attempted.get() || (value - 1.0).abs() > f64::EPSILON {
                validate_hours(value)
            } else {
                None
            }
        });

        let header = row().gap(8.0).children((
            widget(AppMenuBar::new(placeholder_menu)).entity_ref(menu_bar_ref),
            widget(ActionMenu::new().trigger("快捷动作")).entity_ref(action_menu_ref),
            button("命令面板").on_cx(|_, _: &Activate, cx| {
                cx.dispatch_program_all(WorkbenchMessage::Action(ActionId::new(
                    ACTION_TOGGLE_PALETTE,
                )));
            }),
        ));

        let form_view = column().gap(8.0).entity_ref(form_ref).children((
            row().gap(12.0).children((
                widget(FormField::new("任务标题"))
                    .bind(move |field| field.error = title_err.get().map(Arc::from))
                    .control(
                        text_input()
                            .placeholder("输入任务标题（至少 2 个字符）")
                            .model(draft.title())
                            .invalid(move || validate_title(&draft.title().get()).is_some()),
                    ),
                widget(FormField::new("预计工时"))
                    .bind(move |field| field.error = hours_err.get().map(Arc::from))
                    .control(
                        number_input()
                            .placeholder("工时")
                            .unit("h")
                            .model(draft.hours())
                            .invalid(move || validate_hours(draft.hours().get()).is_some()),
                    ),
                button("添加任务").on_cx(|_, _: &Activate, cx| {
                    cx.dispatch_program_all(WorkbenchMessage::Action(ActionId::new(
                        ACTION_ADD_TASK,
                    )));
                }),
            )),
            when(
                move || title_err.get().is_some() || hours_err.get().is_some(),
                move || {
                    widget(ValidationMessage::new("", ValidationIntent::Danger)).bind(move |msg| {
                        if let Some(err) = title_err.get().or_else(|| hours_err.get()) {
                            msg.message = Arc::from(err);
                        }
                    })
                },
            ),
            text(move || status.get()),
        ));

        let virtual_tasks = board
            .tasks()
            .keyed(|task: &TaskItem| task.task_id)
            .each_virtual(40.0, move |task| {
                let id = task.task_id().get_untracked();
                row().gap(8.0).children((
                    checkbox(task.title()).model(task.done()),
                    text(move || format!("{:.1}h", task.hours().get())),
                    button("删除").on_activate(move || {
                        board.last_action().set(format!("删除任务 #{id}"));
                        board.tasks().retain(|item| item.task_id != id);
                    }),
                ))
            })
            .height(320.0)
            .list_ref(list_ref_for_view);

        let page = widget(
            Stack::fill_column(12.0)
                .padding(16.0)
                .with_layout(|layout| layout.padding_top = Some(LengthSpec::Px(48.0))),
        )
        .children((header, form_view, virtual_tasks));

        let mut overlay = OverlayHost::new();
        {
            let layout = Arc::make_mut(&mut overlay.style.layout);
            layout.position = PositionSpec::Fixed;
            layout.offset_left = Some(LengthSpec::Px(0.0));
            layout.offset_top = Some(LengthSpec::Px(0.0));
            layout.width = Some(LengthSpec::Percent(100.0));
            layout.height = Some(LengthSpec::Percent(100.0));
        }
        let overlay_host = widget(overlay).entity_ref(host_ref);

        let root = widget(Stack::fill_column(0.0)).children((page, overlay_host));
        with_refs(root, (host_ref, form_ref, menu_bar_ref, action_menu_ref))
    })?;

    let board = board_slot.get().expect("board store initialized");
    let draft = draft_slot.get().expect("draft store initialized");
    let submit_attempted = submit_slot.get().expect("submit signal initialized");
    let status = status_slot.get().expect("status signal initialized");
    let registry = build_registry(board);

    let mut action_menu_items = Vec::new();
    let initial_ctx = cx.document_key_context(document, KeyContext::new(["workbench"]));
    for raw_id in [
        ACTION_ADD_TASK,
        ACTION_UNDO,
        ACTION_REDO,
        ACTION_TOGGLE_PALETTE,
    ] {
        let action_id = ActionId::new(raw_id);
        let item_view = registry
            .action_menu_item(&action_id, &initial_ctx, Some(&keymap))
            .expect("action exists in registry");
        let item_entity = cx.create_detached_component(document, item_view)?;
        cx.append_child(action_menu, item_entity)?;
        let dispatch_id = action_id.clone();
        cx.on(item_entity, move |_, _: &Activate, cx| {
            cx.dispatch_program_all(WorkbenchMessage::Action(dispatch_id.clone()));
        })?;
        action_menu_items.push((action_id, item_entity));
    }

    let palette = Arc::new(Mutex::new(None::<Entity<CommandPalette>>));

    cx.bind_action_handler(ACTION_ADD_TASK, move |cx| {
        submit_attempted.set(true);
        cx.flush_reactive()?;
        let validity = cx.validity_of(form.stable_id());
        if !validity.is_valid() {
            if let Some(first_focusable) = validity
                .invalid
                .iter()
                .copied()
                .find(|&id| cx.world().interaction(id).is_some_and(|i| i.focusable))
            {
                cx.focus_node(document, first_focusable)?;
            }
            status.set("请先修正表单中的校验错误".to_owned());
            return Ok(());
        }
        let title = draft.title().get_untracked().trim().to_owned();
        let hours = draft.hours().get_untracked();
        let id = board.next_id().get_untracked();
        board.next_id().set(id + 1);
        board.last_action().set(format!("添加任务 #{id}"));
        board.tasks().insert(
            0,
            TaskItem {
                task_id: id,
                title: title.clone(),
                hours,
                done: false,
            },
        );
        draft.title().set(String::new());
        draft.hours().set(1.0);
        submit_attempted.set(false);
        status.set(format!("已添加任务 #{id}: {title}"));
        Ok(())
    })?;

    cx.bind_action_handler(ACTION_UNDO, move |_| {
        if board.undo() {
            status.set(format!("已撤销（当前任务数：{}）", board.tasks().len()));
        }
        Ok(())
    })?;

    cx.bind_action_handler(ACTION_REDO, move |_| {
        if board.redo() {
            status.set(format!("已重做（当前任务数：{}）", board.tasks().len()));
        }
        Ok(())
    })?;

    let palette_slot = Arc::clone(&palette);
    let registry_for_palette = registry.clone();
    let keymap_for_palette = keymap.clone();
    cx.bind_action_handler(ACTION_TOGGLE_PALETTE, move |cx| {
        let mut guard = palette_slot.lock().unwrap();
        if let Some(existing) = guard.take() {
            let _ = cx.dismiss_overlay(host);
            let _ = cx.remove_view(existing);
            return Ok(());
        }
        let ctx = cx.document_key_context(document, KeyContext::new(["workbench"]));
        let items = registry_for_palette.palette_items("", &ctx, Some(&keymap_for_palette));
        let mounted = cx.mount_view_detached(document, || {
            widget(CommandPalette::new("命令面板", items).filtered_items(true)).on_cx(
                |_, event: &CommandPaletteEvent, cx| {
                    cx.dispatch_program_all(WorkbenchMessage::Palette(event.clone()));
                },
            )
        })?;
        let entity = Entity::<CommandPalette>::from_stable_id(mounted.roots()[0]);
        cx.append_child(host, entity)?;
        cx.activate_overlay(host, entity)?;
        *guard = Some(entity);
        Ok(())
    })?;

    let session = WorkbenchSession {
        document,
        board,
        draft,
        form,
        list_ref,
        overlay_host: host,
        menu_bar,
        action_menu,
        action_menu_items,
        palette,
        registry,
        keymap,
        keymap_state: KeymapState::default(),
        menu_map,
        submit_attempted,
        status,
    };
    session.sync_menus(cx)?;
    Ok(session)
}

#[derive(Default)]
pub struct WorkbenchApp {
    sessions: HashMap<WindowId, WorkbenchSession>,
}

impl WorkbenchApp {
    pub fn session(&self, id: WindowId) -> Option<&WorkbenchSession> {
        self.sessions.get(&id)
    }

    pub fn session_mut(&mut self, id: WindowId) -> Option<&mut WorkbenchSession> {
        self.sessions.get_mut(&id)
    }
}

impl ApplicationState for WorkbenchApp {
    type Message = WorkbenchMessage;
    type Error = FrameworkError;

    fn initialize(_: &RuntimeProgramContext<Self::Message>) -> Result<Self, Self::Error> {
        Ok(Self::default())
    }

    fn window_closed(&mut self, id: WindowId) {
        self.sessions.remove(&id);
    }

    fn build(
        &mut self,
        window: &mut ApplicationWindow,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), Self::Error> {
        let document = window.document.document();
        let session = mount_workbench(window.document.context_mut(), document)?;
        self.sessions.insert(context.window_id(), session);
        Ok(())
    }

    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let mut redraw = false;
        if let Some(window) = windows.get_mut(&id)
            && let Some(session) = self.sessions.get_mut(&id)
        {
            let cx = window.document.context_mut();
            for menu_id in take_menu_activations() {
                if cx
                    .dispatch_menu_action(
                        &session.registry,
                        &session.menu_map,
                        menu_id,
                        &session.key_context(cx),
                    )
                    .is_ok()
                {
                    cx.flush_reactive()?;
                    session.sync_menus(cx)?;
                    redraw = true;
                }
            }
            let key_context = session.key_context(cx);
            if cx
                .dispatch_routed_key(
                    &session.keymap,
                    &mut session.keymap_state,
                    &session.registry,
                    input.event,
                    input.disposition,
                    &key_context,
                )
                .is_ok_and(|matched| matched.consumed())
            {
                cx.flush_reactive()?;
                session.sync_menus(cx)?;
                redraw = true;
            }
        }
        Ok(if redraw {
            RuntimeProgramUpdate::redraw(id)
        } else {
            RuntimeProgramUpdate::default()
        })
    }

    fn update(
        &mut self,
        message: Self::Message,
        windows: &mut HashMap<WindowId, ApplicationWindow>,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        let id = context.window_id();
        let Some(window) = windows.get_mut(&id) else {
            return RuntimeProgramUpdate::default();
        };
        let Some(session) = self.sessions.get_mut(&id) else {
            return RuntimeProgramUpdate::default();
        };
        let cx = window.document.context_mut();
        match message {
            WorkbenchMessage::Action(action) => {
                let _ = session.dispatch_action(cx, &action);
            }
            WorkbenchMessage::Palette(event) => {
                let key_context = session.key_context(cx);
                match &event {
                    CommandPaletteEvent::Search(query) => {
                        if let Some(palette) = *session.palette.lock().unwrap() {
                            let items = session.registry.palette_items(
                                query,
                                &key_context,
                                Some(&session.keymap),
                            );
                            let _ = cx.update_component(palette, |view, _| {
                                view.items = items;
                                view.selected = 0;
                            });
                        }
                    }
                    CommandPaletteEvent::Select(_) => {
                        if let Some(palette) = session.palette.lock().unwrap().take() {
                            let _ = cx.dismiss_overlay(session.overlay_host);
                            let _ = cx.remove_view(palette);
                        }
                        if cx
                            .dispatch_palette_event(&session.registry, &event, &key_context)
                            .is_ok()
                        {
                            let _ = cx.flush_reactive();
                            let _ = session.sync_menus(cx);
                        }
                    }
                    CommandPaletteEvent::Dismiss => {
                        if let Some(palette) = session.palette.lock().unwrap().take() {
                            let _ = cx.dismiss_overlay(session.overlay_host);
                            let _ = cx.remove_view(palette);
                        }
                    }
                    CommandPaletteEvent::Navigate(_) => {}
                }
            }
        }
        RuntimeProgramUpdate::redraw(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::NanaTextShaper;
    use nana_ui::runtime::host::RuntimeDocument;
    use nana_ui::runtime::{Checkbox, HeadlessInput, LayoutViewport, TextInput};
    use nana_ui::{InputPayload, KeyInput, KeyState};

    fn settle(document: &mut RuntimeDocument, shaper: &mut NanaTextShaper) {
        let viewport = LayoutViewport::new(960.0, 640.0);
        for _ in 0..4 {
            let _ = document.context_mut().flush_reactive();
            let _ = document.flush(viewport, shaper);
        }
        let _ = document.context_mut().flush_reactive();
    }

    #[test]
    fn golden_example_virtualizes_2000_tasks_validates_form_and_tracks_store_fields() {
        let doc_id = DocumentId::new(28401).unwrap();
        let mut document = RuntimeDocument::new(doc_id);
        let mut shaper = NanaTextShaper::default();
        let session = mount_workbench(document.context_mut(), doc_id).unwrap();
        settle(&mut document, &mut shaper);

        // 1. each_virtual mounts only the visible viewport window out of 2,000 tasks.
        let built_rows = (1..=2_000_u64)
            .filter(|id| session.list_ref.row(id).is_some())
            .count();
        assert!(
            built_rows > 0 && built_rows < 30,
            "expected bounded virtual window, got {built_rows} live rows"
        );

        // 2. Form validation rejects empty title via validity_of and focuses first invalid field.
        let ctx = session.key_context(document.context());
        document
            .context_mut()
            .dispatch_registered_action(&session.registry, &ActionId::new(ACTION_ADD_TASK), &ctx)
            .unwrap();
        settle(&mut document, &mut shaper);
        assert_eq!(session.board.tasks().len(), 2_000);
        let validity = document.context().validity_of(session.form.stable_id());
        assert!(!validity.is_valid());
        let first_focusable = validity.invalid.iter().copied().find(|&id| {
            document
                .context()
                .world()
                .interaction(id)
                .is_some_and(|i| i.focusable)
        });
        assert_eq!(document.context().world().focused(doc_id), first_focusable);
        let focused = document.context().world().focused(doc_id).unwrap();
        assert!(
            document
                .context()
                .read(Entity::<TextInput>::from_stable_id(focused), |_| ())
                .is_ok()
        );

        // 3. Valid input passes validation and inserts a new task as one undo step.
        session.draft.title().set(String::from("验收 Golden 示例"));
        session.draft.hours().set(3.5);
        settle(&mut document, &mut shaper);
        assert!(
            document
                .context()
                .validity_of(session.form.stable_id())
                .is_valid()
        );
        document
            .context_mut()
            .dispatch_registered_action(&session.registry, &ActionId::new(ACTION_ADD_TASK), &ctx)
            .unwrap();
        settle(&mut document, &mut shaper);
        assert_eq!(session.board.tasks().len(), 2_001);
        assert_eq!(
            session.board.tasks().get_untracked()[0].title,
            "验收 Golden 示例"
        );

        // 4. Fine-grained #[derive(Store)] update toggles only row #1's checkbox.
        let tasks = session.board.tasks().keyed(|task| task.task_id);
        let slot1_id = session.list_ref.row(&1).expect("row 1 is visible");
        let slot2_id = session.list_ref.row(&2).expect("row 2 is visible");
        let row1_id = document.context().world().node(slot1_id).unwrap().children[0];
        let row2_id = document.context().world().node(slot2_id).unwrap().children[0];
        let row1_checkbox = Entity::<Checkbox>::from_stable_id(
            document.context().world().node(row1_id).unwrap().children[0],
        );
        let row2_checkbox = Entity::<Checkbox>::from_stable_id(
            document.context().world().node(row2_id).unwrap().children[0],
        );
        assert!(
            !document
                .context()
                .read(row1_checkbox, |c| c.checked)
                .unwrap()
        );
        assert!(
            !document
                .context()
                .read(row2_checkbox, |c| c.checked)
                .unwrap()
        );
        tasks.at(&1).done().set(true);
        document.context_mut().flush_reactive().unwrap();
        assert!(
            document
                .context()
                .read(row1_checkbox, |c| c.checked)
                .unwrap()
        );
        assert!(
            !document
                .context()
                .read(row2_checkbox, |c| c.checked)
                .unwrap()
        );
    }

    #[test]
    fn golden_example_unifies_dynamic_undo_redo_across_keymap_menu_palette_and_program() {
        let doc_id = DocumentId::new(28402).unwrap();
        let mut document = RuntimeDocument::new(doc_id);
        let mut shaper = NanaTextShaper::default();
        let mut session = mount_workbench(document.context_mut(), doc_id).unwrap();
        settle(&mut document, &mut shaper);

        let undo_id = ActionId::new(ACTION_UNDO);
        let redo_id = ActionId::new(ACTION_REDO);
        let ctx = session.key_context(document.context());
        assert!(!session.registry.is_available(&undo_id, &ctx));
        assert!(!session.registry.is_available(&redo_id, &ctx));

        // Add a valid task so undo becomes enabled with dynamic label.
        session.draft.title().set(String::from("统一动作管线"));
        session.draft.hours().set(2.0);
        session
            .dispatch_action(document.context_mut(), &ActionId::new(ACTION_ADD_TASK))
            .unwrap();
        settle(&mut document, &mut shaper);
        assert_eq!(session.board.tasks().len(), 2_001);

        let ctx = session.key_context(document.context());
        assert!(session.registry.is_available(&undo_id, &ctx));
        let palette = session
            .registry
            .palette_items("撤销", &ctx, Some(&session.keymap));
        assert_eq!(palette.len(), 1);
        assert!(palette[0].label.starts_with("撤销: "));
        let menu_bar = document
            .context()
            .read(session.menu_bar, |bar| bar.bar.clone())
            .unwrap();
        assert!(matches!(
            &menu_bar.menus[1].entries[0],
            MenuEntry::Item { label, enabled: true, .. } if label.starts_with("撤销: ")
        ));
        assert!(
            !document
                .context()
                .read(session.action_menu_items[1].1, |item| item.disabled)
                .unwrap()
        );

        // Undo via Keymap (after clearing focus so CONTEXT_TEXT_INPUT does not block Ctrl/Cmd+Z).
        document.context_mut().clear_focus(doc_id).unwrap();
        let ctx = session.key_context(document.context());
        let primary = KeyModifiers::primary();
        let mut input = HeadlessInput::bind(document.context_mut(), doc_id);
        let undo_ev = input.stamp(InputPayload::Key(KeyInput::named(
            "z",
            "z",
            KeyState::Pressed,
            primary,
        )));
        let routed = document
            .context_mut()
            .route_input(&undo_ev, input.services_mut(), None)
            .unwrap();
        assert!(
            document
                .context_mut()
                .dispatch_routed_key(
                    &session.keymap,
                    &mut session.keymap_state,
                    &session.registry,
                    &undo_ev,
                    routed.disposition(),
                    &ctx,
                )
                .unwrap()
                .consumed()
        );
        document.context_mut().flush_reactive().unwrap();
        session.sync_menus(document.context_mut()).unwrap();
        assert_eq!(session.board.tasks().len(), 2_000);
        assert!(session.registry.is_available(&redo_id, &ctx));

        // Redo via MenuActionMap dispatch.
        assert_eq!(
            document
                .context_mut()
                .dispatch_menu_action(&session.registry, &session.menu_map, MENU_ID_REDO, &ctx)
                .unwrap(),
            redo_id
        );
        document.context_mut().flush_reactive().unwrap();
        session.sync_menus(document.context_mut()).unwrap();
        assert_eq!(session.board.tasks().len(), 2_001);

        // Undo again via CommandPaletteEvent::Select.
        let event = CommandPaletteEvent::Select(undo_id.clone());
        assert_eq!(
            document
                .context_mut()
                .dispatch_palette_event(&session.registry, &event, &ctx)
                .unwrap(),
            Some(undo_id)
        );
        assert_eq!(session.board.tasks().len(), 2_000);
    }
}
