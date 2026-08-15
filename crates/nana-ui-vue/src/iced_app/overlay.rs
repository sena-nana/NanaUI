// Nana Overlay adapters: Dialog / Drawer / Popover / ContextMenu.

fn overlay_is_open(props: &crate::bridge::WidgetProps) -> bool {
    props.active || props.toggled
}

/// Searchable menus stay on Iced [`ContextMenuHost`] + [`MenuStore`] so the
/// search field is not dropped. Nested `parent/child` values Scene-route.
fn context_menu_requires_iced_host(props: &WidgetProps) -> bool {
    context_menu_is_searchable(props)
}

fn context_menu_is_searchable(props: &WidgetProps) -> bool {
    props.options.len() >= 6
        || props
            .class_names
            .iter()
            .any(|class| class.contains("search"))
}

fn is_action_menu_props(props: &WidgetProps) -> bool {
    props.class_names.iter().any(|class| {
        matches!(
            class.as_str(),
            "nana-action-menu" | "action-menu" | "nana-actionmenu"
        )
    })
}

fn overlay_dialog<'a, Message>(
    snap: &'a SemanticSnapshot,
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    if !overlay_is_open(&widget.props) {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let id = widget.id;
    if is_confirm_dialog_props(&widget.props) {
        return overlay_confirm_dialog(&widget.props, id, tokens, map_event);
    }
    let map_close = map_event.clone();
    let map_outside = map_event.clone();
    let child_box = widget.props.layout.resolve_content_box(parent_box);
    let mut body = column![].spacing(8).width(Length::Fill);
    if widget.children.is_empty() && !widget.props.hint.is_empty() {
        body = body.push(text(widget.props.hint.as_str()).size(13));
    }
    for &child in &widget.children {
        body = body.push(view_widget(
            snap,
            child,
            tokens,
            child_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            None,
            map_event.clone(),
        ));
    }
    let mut dialog = Dialog::new(widget.props.display_label(), body)
        .on_close(map_close(BridgeEvent::Toggle { id, value: false }))
        .on_outside(map_outside(BridgeEvent::Toggle { id, value: false }));
    if !widget.props.hint.is_empty() && !widget.children.is_empty() {
        dialog = dialog.description(widget.props.hint.as_str());
    }
    dialog.view(tokens)
}

fn overlay_dialog_owned<Message>(
    snap: &SemanticSnapshot,
    props: &crate::bridge::WidgetProps,
    id: WidgetId,
    children: &[WidgetId],
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    if !overlay_is_open(props) {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    if is_confirm_dialog_props(props) {
        return overlay_confirm_dialog(props, id, tokens, map_event);
    }
    let map_close = map_event.clone();
    let map_outside = map_event.clone();
    let child_box = props.layout.resolve_content_box(parent_box);
    let mut body = column![].spacing(8).width(Length::Fill);
    if children.is_empty() && !props.hint.is_empty() {
        body = body.push(text(props.hint.clone()).size(13));
    }
    for &child in children {
        body = body.push(view_widget_owned(
            snap,
            child,
            tokens,
            child_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            viewport,
            None,
            map_event.clone(),
        ));
    }
    let mut dialog = Dialog::new(owned_display(props), body)
        .on_close(map_close(BridgeEvent::Toggle { id, value: false }))
        .on_outside(map_outside(BridgeEvent::Toggle { id, value: false }));
    if !props.hint.is_empty() && !children.is_empty() {
        dialog = dialog.description(props.hint.clone());
    }
    dialog.view(tokens)
}

/// `role=alertdialog` / `data-variant=confirm` / `nana-confirm*` / `kind=danger`
/// → real [`ConfirmDialog`]. No product class-substring matching.
fn is_confirm_dialog_props(props: &WidgetProps) -> bool {
    let role = props.role.to_ascii_lowercase();
    if role == "alertdialog" {
        return true;
    }
    if matches!(props.button_kind, ButtonKind::Danger) {
        return true;
    }
    if props
        .attrs
        .get("data-variant")
        .is_some_and(|v| v.eq_ignore_ascii_case("confirm") || v.eq_ignore_ascii_case("alertdialog"))
    {
        return true;
    }
    props.class_names.iter().any(|c| {
        matches!(
            c.as_str(),
            "nana-confirm" | "nana-confirm-dialog" | "nana-alertdialog"
        )
    })
}

fn confirm_dialog_danger(props: &WidgetProps) -> bool {
    matches!(props.button_kind, ButtonKind::Danger)
        || props
            .attrs
            .get("data-variant")
            .is_some_and(|v| v.eq_ignore_ascii_case("danger"))
}

fn overlay_confirm_dialog<'a, Message>(
    props: &WidgetProps,
    id: WidgetId,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let map_confirm = map_event.clone();
    let map_cancel = map_event.clone();
    let map_outside = map_event.clone();
    let map_interact = map_event;
    // Host / app owns copy — engine never invents locale business strings.
    let title = props.label.clone();
    let message = if !props.hint.is_empty() {
        props.hint.clone()
    } else {
        props.value.clone()
    };
    ConfirmDialog::new(
        title,
        message,
        map_confirm(BridgeEvent::SelectValue {
            id,
            value: "confirm".into(),
        }),
        map_cancel(BridgeEvent::Toggle { id, value: false }),
        map_interact(BridgeEvent::Press { id }),
    )
    .danger(confirm_dialog_danger(props))
    .on_outside(map_outside(BridgeEvent::Toggle { id, value: false }))
    .view(tokens)
}

fn drawer_side_from_props(props: &WidgetProps) -> DrawerSide {
    let side = props.side.to_ascii_lowercase();
    if side == "left" || side == "start" {
        return DrawerSide::Left;
    }
    if side == "right" || side == "end" {
        return DrawerSide::Right;
    }
    if props
        .attrs
        .get("data-side")
        .is_some_and(|s| s.eq_ignore_ascii_case("left") || s.eq_ignore_ascii_case("start"))
        || props
            .class_names
            .iter()
            .any(|c| c == "nana-drawer-left" || c == "nana-sheet-left")
    {
        return DrawerSide::Left;
    }
    DrawerSide::Right
}

fn drawer_width_from_props(props: &WidgetProps) -> f32 {
    if let Some(LengthSpec::Px(px)) = props.layout.width {
        return px.max(240.0);
    }
    if props.number >= 240.0 {
        return props.number;
    }
    360.0
}

fn is_drawer_footer_props(props: &WidgetProps) -> bool {
    props
        .attrs
        .get("data-slot")
        .is_some_and(|s| s.eq_ignore_ascii_case("drawer-footer"))
        || props.class_names.iter().any(|c| {
            matches!(
                c.as_str(),
                "nana-drawer-footer" | "drawer-footer" | "drawer__footer"
            )
        })
        || props.role.eq_ignore_ascii_case("contentinfo")
}

/// Footer action semantics for buttons/chips inside a Drawer footer slot.
///
/// Mirrors [`ConfirmDialog`]: confirm → `SelectValue` on the **drawer** id;
/// cancel → `Toggle { value: false }` (close). Neutral keeps the child `Press`/`Select`.
/// Prefer `data-nana-action` / documented `nana-*` class tokens — no locale copy matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawerFooterAction {
    Confirm,
    Cancel,
    Neutral,
}

fn drawer_footer_token_hit(raw: &str, needles: &[&str]) -> bool {
    let s = raw.to_ascii_lowercase();
    needles.iter().any(|n| s == *n)
}

fn drawer_footer_action(props: &WidgetProps) -> DrawerFooterAction {
    const CANCEL: &[&str] = &[
        "cancel",
        "dismiss",
        "close",
        "drawer-footer-cancel",
        "footer-cancel",
        "nana-drawer-cancel",
    ];
    const CONFIRM: &[&str] = &[
        "confirm",
        "apply",
        "save",
        "ok",
        "drawer-footer-confirm",
        "drawer-footer-apply",
        "footer-confirm",
        "footer-apply",
        "footer-ok",
        "nana-drawer-confirm",
    ];

    if let Some(action) = props
        .attrs
        .get("data-nana-action")
        .or_else(|| props.attrs.get("data-action"))
    {
        let a = action.to_ascii_lowercase();
        if CANCEL.contains(&a.as_str()) {
            return DrawerFooterAction::Cancel;
        }
        if CONFIRM.contains(&a.as_str()) {
            return DrawerFooterAction::Confirm;
        }
    }

    let class_hit = |needles: &[&str]| {
        props
            .class_names
            .iter()
            .any(|c| drawer_footer_token_hit(c, needles))
    };
    if class_hit(CANCEL)
        || drawer_footer_token_hit(&props.value, CANCEL)
        || drawer_footer_token_hit(&props.role, CANCEL)
    {
        return DrawerFooterAction::Cancel;
    }
    if class_hit(CONFIRM)
        || drawer_footer_token_hit(&props.value, CONFIRM)
        || drawer_footer_token_hit(&props.role, CONFIRM)
    {
        return DrawerFooterAction::Confirm;
    }
    // Primary / selected footer controls default to confirm (Cancel must be explicit).
    if matches!(
        props.button_kind,
        ButtonKind::Primary | ButtonKind::Selected
    ) {
        return DrawerFooterAction::Confirm;
    }
    DrawerFooterAction::Neutral
}

fn drawer_footer_confirm_value(props: &WidgetProps) -> String {
    let v = props.value.trim();
    if !v.is_empty()
        && !drawer_footer_token_hit(
            v,
            &[
                "cancel",
                "dismiss",
                "close",
                "drawer-footer-cancel",
                "nana-drawer-cancel",
            ],
        )
    {
        return v.to_string();
    }
    "confirm".into()
}

fn drawer_footer_press_event(
    drawer_id: WidgetId,
    props: &WidgetProps,
    control_id: WidgetId,
) -> BridgeEvent {
    match drawer_footer_action(props) {
        DrawerFooterAction::Confirm => BridgeEvent::SelectValue {
            id: drawer_id,
            value: drawer_footer_confirm_value(props),
        },
        DrawerFooterAction::Cancel => BridgeEvent::Toggle {
            id: drawer_id,
            value: false,
        },
        DrawerFooterAction::Neutral => BridgeEvent::Press { id: control_id },
    }
}

fn drawer_footer_chip_event(
    drawer_id: WidgetId,
    props: &WidgetProps,
    control_id: WidgetId,
) -> BridgeEvent {
    match drawer_footer_action(props) {
        DrawerFooterAction::Confirm => BridgeEvent::SelectValue {
            id: drawer_id,
            value: drawer_footer_confirm_value(props),
        },
        DrawerFooterAction::Cancel => BridgeEvent::Toggle {
            id: drawer_id,
            value: false,
        },
        DrawerFooterAction::Neutral => BridgeEvent::Select { id: control_id },
    }
}

fn view_drawer_footer_node<'a, Message>(
    snap: &'a SemanticSnapshot,
    id: WidgetId,
    drawer_id: WidgetId,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let Some(widget) = snap.get(id) else {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    };
    if widget.props.layout.hidden {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    match widget.kind {
        WidgetKind::Button => {
            let map = map_event.clone();
            let event = drawer_footer_press_event(drawer_id, &widget.props, widget.id);
            Button::label(widget.props.display_label())
                .kind(widget.props.button_kind)
                .size(widget.props.size)
                .disabled(widget.props.disabled)
                .loading(widget.props.loading, 0)
                .on_press(map(event))
                .view(tokens)
        }
        WidgetKind::Chip => {
            let label = widget.props.display_label();
            if label.is_empty() {
                return space().width(Length::Shrink).height(Length::Shrink).into();
            }
            let map = map_event.clone();
            let event = drawer_footer_chip_event(drawer_id, &widget.props, widget.id);
            let kind = if widget.props.active {
                ButtonKind::Selected
            } else {
                ButtonKind::Subtle
            };
            Button::label(label)
                .kind(kind)
                .size(ControlSize::Small)
                .disabled(widget.props.disabled)
                .on_press(map(event))
                .view(tokens)
        }
        WidgetKind::Row | WidgetKind::Column | WidgetKind::Box | WidgetKind::Card => {
            let child_box = widget.props.layout.resolve_content_box(parent_box);
            let mut row_el = row![].spacing(8).width(Length::Fill);
            let mut col_el = column![].spacing(8).width(Length::Fill);
            let as_row = matches!(widget.kind, WidgetKind::Row)
                || widget
                    .props
                    .class_names
                    .iter()
                    .any(|c| c.to_ascii_lowercase().contains("footer"));
            for &child in &widget.children {
                let el = view_drawer_footer_node(
                    snap,
                    child,
                    drawer_id,
                    tokens,
                    child_box,
                    editors,
                    menus,
                    map_event.clone(),
                );
                if as_row {
                    row_el = row_el.push(el);
                } else {
                    col_el = col_el.push(el);
                }
            }
            if as_row {
                row_el.into()
            } else {
                col_el.into()
            }
        }
        _ => view_widget(
            snap,
            id,
            tokens,
            parent_box,
            FlexDirection::Row,
            AlignSpec::Stretch,
            editors,
            menus,
            None,
            map_event,
        ),
    }
}

fn view_drawer_footer_node_owned<Message>(
    snap: &SemanticSnapshot,
    id: WidgetId,
    drawer_id: WidgetId,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let Some(widget) = snap.get(id) else {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    };
    if widget.props.layout.hidden {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let kind = widget.kind;
    let props = widget.props.clone();
    let children = widget.children.clone();
    let wid = widget.id;
    match kind {
        WidgetKind::Button => {
            let map = map_event.clone();
            let event = drawer_footer_press_event(drawer_id, &props, wid);
            Button::label(owned_display(&props))
                .kind(props.button_kind)
                .size(props.size)
                .disabled(props.disabled)
                .loading(props.loading, 0)
                .on_press(map(event))
                .view(tokens)
        }
        WidgetKind::Chip => {
            let label = owned_display(&props);
            if label.is_empty() {
                return space().width(Length::Shrink).height(Length::Shrink).into();
            }
            let map = map_event.clone();
            let event = drawer_footer_chip_event(drawer_id, &props, wid);
            let kind = if props.active {
                ButtonKind::Selected
            } else {
                ButtonKind::Subtle
            };
            Button::label(label)
                .kind(kind)
                .size(ControlSize::Small)
                .disabled(props.disabled)
                .on_press(map(event))
                .view(tokens)
        }
        WidgetKind::Row | WidgetKind::Column | WidgetKind::Box | WidgetKind::Card => {
            let child_box = props.layout.resolve_content_box(parent_box);
            let mut row_el = row![].spacing(8).width(Length::Fill);
            let mut col_el = column![].spacing(8).width(Length::Fill);
            let as_row = matches!(kind, WidgetKind::Row)
                || props
                    .class_names
                    .iter()
                    .any(|c| c.to_ascii_lowercase().contains("footer"));
            for child in children {
                let el = view_drawer_footer_node_owned(
                    snap,
                    child,
                    drawer_id,
                    tokens,
                    child_box,
                    editors,
                    menus,
                    viewport,
                    map_event.clone(),
                );
                if as_row {
                    row_el = row_el.push(el);
                } else {
                    col_el = col_el.push(el);
                }
            }
            if as_row {
                row_el.into()
            } else {
                col_el.into()
            }
        }
        _ => view_widget_owned(
            snap,
            id,
            tokens,
            parent_box,
            FlexDirection::Row,
            AlignSpec::Stretch,
            editors,
            menus,
            viewport,
            None,
            map_event,
        ),
    }
}

fn overlay_drawer<'a, Message>(
    snap: &'a SemanticSnapshot,
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    if !overlay_is_open(&widget.props) {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let id = widget.id;
    let map_close = map_event.clone();
    let map_interact = map_event.clone();
    let child_box = widget.props.layout.resolve_content_box(parent_box);
    let (body_ids, footer_ids): (Vec<WidgetId>, Vec<WidgetId>) = widget
        .children
        .iter()
        .copied()
        .partition(|&cid| {
            !snap
                .get(cid)
                .is_some_and(|c| is_drawer_footer_props(&c.props))
        });
    let mut body = column![].spacing(8).width(Length::Fill);
    if body_ids.is_empty() && footer_ids.is_empty() && !widget.props.hint.is_empty() {
        body = body.push(text(widget.props.hint.as_str()).size(13));
    }
    for &child in &body_ids {
        body = body.push(view_widget(
            snap,
            child,
            tokens,
            child_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            None,
            map_event.clone(),
        ));
    }
    let title = if widget.props.label.is_empty() {
        "抽屉"
    } else {
        widget.props.display_label()
    };
    let mut drawer = Drawer::new(
        title,
        body,
        map_close(BridgeEvent::Toggle { id, value: false }),
        map_interact(BridgeEvent::Press { id }),
    )
    .side(drawer_side_from_props(&widget.props))
    .width(drawer_width_from_props(&widget.props));
    if !footer_ids.is_empty() {
        let mut footer = row![].spacing(8).width(Length::Fill);
        for &child in &footer_ids {
            footer = footer.push(view_drawer_footer_node(
                snap,
                child,
                id,
                tokens,
                child_box,
                editors,
                menus,
                map_event.clone(),
            ));
        }
        drawer = drawer.footer(footer);
    }
    drawer.view(tokens)
}

fn overlay_drawer_owned<Message>(
    snap: &SemanticSnapshot,
    props: &WidgetProps,
    id: WidgetId,
    children: &[WidgetId],
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    if !overlay_is_open(props) {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let map_close = map_event.clone();
    let map_interact = map_event.clone();
    let child_box = props.layout.resolve_content_box(parent_box);
    let (body_ids, footer_ids): (Vec<WidgetId>, Vec<WidgetId>) =
        children.iter().copied().partition(|&cid| {
            !snap
                .get(cid)
                .is_some_and(|c| is_drawer_footer_props(&c.props))
        });
    let mut body = column![].spacing(8).width(Length::Fill);
    if body_ids.is_empty() && footer_ids.is_empty() && !props.hint.is_empty() {
        body = body.push(text(props.hint.clone()).size(13));
    }
    for &child in &body_ids {
        body = body.push(view_widget_owned(
            snap,
            child,
            tokens,
            child_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            viewport,
            None,
            map_event.clone(),
        ));
    }
    let title = if props.label.is_empty() {
        "抽屉".to_string()
    } else {
        owned_display(props)
    };
    let mut drawer = Drawer::new(
        title,
        body,
        map_close(BridgeEvent::Toggle { id, value: false }),
        map_interact(BridgeEvent::Press { id }),
    )
    .side(drawer_side_from_props(props))
    .width(drawer_width_from_props(props));
    if !footer_ids.is_empty() {
        let mut footer = row![].spacing(8).width(Length::Fill);
        for &child in &footer_ids {
            footer = footer.push(view_drawer_footer_node_owned(
                snap,
                child,
                id,
                tokens,
                child_box,
                editors,
                menus,
                viewport,
                map_event.clone(),
            ));
        }
        drawer = drawer.footer(footer);
    }
    drawer.view(tokens)
}

fn overlay_popover<'a, Message>(
    snap: &'a SemanticSnapshot,
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let id = widget.id;
    let open = overlay_is_open(&widget.props);
    let map_toggle = map_event.clone();
    let map_close = map_event.clone();
    let child_box = widget.props.layout.resolve_content_box(parent_box);
    let mut content = column![].spacing(6).width(Length::Fill);
    if widget.children.is_empty() {
        content = content.push(text(widget.props.hint.as_str()).size(12));
    }
    for &child in &widget.children {
        content = content.push(view_widget(
            snap,
            child,
            tokens,
            child_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            None,
            map_event.clone(),
        ));
    }
    let trigger = text(widget.props.display_label()).size(13);
    Popover::new(
        trigger,
        content,
        open,
        map_toggle(BridgeEvent::Toggle { id, value: !open }),
        map_close(BridgeEvent::Toggle { id, value: false }),
        tokens,
    )
    .view()
}

fn overlay_popover_owned<Message>(
    snap: &SemanticSnapshot,
    props: &crate::bridge::WidgetProps,
    id: WidgetId,
    children: &[WidgetId],
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let open = overlay_is_open(props);
    let map_toggle = map_event.clone();
    let map_close = map_event.clone();
    let child_box = props.layout.resolve_content_box(parent_box);
    let mut content = column![].spacing(6).width(Length::Fill);
    if children.is_empty() {
        content = content.push(text(props.hint.clone()).size(12));
    }
    for &child in children {
        content = content.push(view_widget_owned(
            snap,
            child,
            tokens,
            child_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            viewport,
            None,
            map_event.clone(),
        ));
    }
    let trigger = text(owned_display(props)).size(13);
    Popover::new(
        trigger,
        content,
        open,
        map_toggle(BridgeEvent::Toggle { id, value: !open }),
        map_close(BridgeEvent::Toggle { id, value: false }),
        tokens,
    )
    .view()
}

fn map_context_menu_event<Message>(
    id: WidgetId,
    event: ContextMenuEvent<String>,
    map: &impl Fn(BridgeEvent) -> Message,
) -> Message {
    match event {
        ContextMenuEvent::Select(value) => map(BridgeEvent::SelectValue { id, value }),
        ContextMenuEvent::Dismiss => map(BridgeEvent::Toggle { id, value: false }),
        ContextMenuEvent::Interaction => map(BridgeEvent::Press { id }),
        ContextMenuEvent::Search(query) => map(BridgeEvent::MenuSearch { id, query }),
        ContextMenuEvent::OpenSubmenu(path) => map(BridgeEvent::MenuPath { id, path }),
    }
}

fn overlay_context_menu<'a, Message>(
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    if !overlay_is_open(&widget.props) {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let id = widget.id;
    let viewport = Size::new(
        parent_box.width.unwrap_or(1280.0).max(320.0),
        parent_box.height.unwrap_or(800.0).max(240.0),
    );
    let position =
        AnchoredMenuPosition::new(Point::new(widget.props.anchor_x, widget.props.anchor_y))
            .placement(AnchoredMenuPlacement::BottomStart);

    if let Some(slot) = menus.and_then(|store| store.get(id)) {
        let map = map_event.clone();
        let pending = slot.pending.as_ref();
        let menu = ContextMenuHost::new(
            &slot.items,
            position,
            viewport,
            move |event| map_context_menu_event(id, event, &map),
            tokens,
        )
        .search(&slot.query, slot.searchable)
        .active_path(&slot.active_path)
        .pending(pending)
        .view();
        return OverlayHost::new(space().width(Length::Fill).height(Length::Fill))
            .push(menu)
            .view();
    }

    // Fallback when host has not prepared MenuStore yet.
    let map_dismiss = map_event.clone();
    let map_interact = map_event.clone();
    let mut col = column![].spacing(1).width(Length::Fill);
    if widget.props.options.is_empty() {
        col = col.push(
            ActionMenuItem::new(widget.props.display_label())
                .on_press(map_event(BridgeEvent::Select { id }))
                .view(tokens),
        );
    } else {
        for opt in &widget.props.options {
            let value = opt.value.clone();
            let map = map_event.clone();
            let mut item = ActionMenuItem::new(opt.label.as_str());
            if !opt.disabled {
                item = item.on_press(map(BridgeEvent::SelectValue { id, value }));
            } else {
                item = item.disabled(true);
            }
            col = col.push(item.view(tokens));
        }
    }
    let menu = AnchoredActionMenu::new(
        col,
        position,
        viewport,
        map_dismiss(BridgeEvent::Toggle { id, value: false }),
        map_interact(BridgeEvent::Press { id }),
    )
    .menu_size(
        200.0,
        (32.0 + widget.props.options.len().max(1) as f32 * 28.0).min(320.0),
    )
    .view(tokens);
    OverlayHost::new(space().width(Length::Fill).height(Length::Fill))
        .push(menu)
        .view()
}

fn overlay_context_menu_owned<Message>(
    props: &crate::bridge::WidgetProps,
    id: WidgetId,
    tokens: ThemeTokens,
    viewport: Size,
    menus: Option<&MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    if !overlay_is_open(props) {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let position = AnchoredMenuPosition::new(Point::new(props.anchor_x, props.anchor_y))
        .placement(AnchoredMenuPlacement::BottomStart);

    if let Some(store) = menus {
        if let (Some(items), Some(query), Some(path)) = (
            store.items_static(id),
            store.query_static(id),
            store.path_static(id),
        ) {
            let searchable = store.get(id).map(|s| s.searchable).unwrap_or(false);
            let pending = store.pending_static(id);
            let map = map_event.clone();
            let menu = ContextMenuHost::new(
                items,
                position,
                viewport,
                move |event| map_context_menu_event(id, event, &map),
                tokens,
            )
            .search(query, searchable)
            .active_path(path)
            .pending(pending)
            .view();
            return OverlayHost::new(space().width(Length::Fill).height(Length::Fill))
                .push(menu)
                .view();
        }
    }

    let map_dismiss = map_event.clone();
    let map_interact = map_event.clone();
    let mut col = column![].spacing(1).width(Length::Fill);
    if props.options.is_empty() {
        let map = map_event.clone();
        col = col.push(
            ActionMenuItem::new(owned_display(props))
                .on_press(map(BridgeEvent::Select { id }))
                .view(tokens),
        );
    } else {
        for opt in &props.options {
            let value = opt.value.clone();
            let map = map_event.clone();
            let mut item = ActionMenuItem::new(opt.label.clone());
            if !opt.disabled {
                item = item.on_press(map(BridgeEvent::SelectValue { id, value }));
            } else {
                item = item.disabled(true);
            }
            col = col.push(item.view(tokens));
        }
    }
    let menu = AnchoredActionMenu::new(
        col,
        position,
        viewport,
        map_dismiss(BridgeEvent::Toggle { id, value: false }),
        map_interact(BridgeEvent::Press { id }),
    )
    .menu_size(
        200.0,
        (32.0 + props.options.len().max(1) as f32 * 28.0).min(320.0),
    )
    .view(tokens);
    OverlayHost::new(space().width(Length::Fill).height(Length::Fill))
        .push(menu)
        .view()
}
