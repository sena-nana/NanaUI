// SettingsRow / SettingsCard adapter views.

fn settings_row_label_hint(
    snap: &SemanticSnapshot,
    props: &WidgetProps,
    children: &[WidgetId],
) -> (String, String) {
    let mut label = props.display_label().to_string();
    let mut hint = props.hint.clone();
    if let Some(label_id) = find_child_with_class(snap, children, "nana-settings-row__label")
        .or_else(|| find_child_with_class(snap, children, "settings-row__label"))
    {
        if label.is_empty() {
            // Prefer the first non-hint text node under __label.
            if let Some(w) = snap.get(label_id) {
                for &child in &w.children {
                    let child_w = snap.get(child);
                    let is_hint = child_w
                        .is_some_and(|c| c.props.class_names.iter().any(|n| n.contains("hint")));
                    if is_hint {
                        if hint.is_empty() {
                            hint = collect_plain_text(snap, child);
                        }
                        continue;
                    }
                    let t = collect_plain_text(snap, child);
                    if !t.is_empty() {
                        label = t;
                        break;
                    }
                }
                if label.is_empty() {
                    label = collect_plain_text(snap, label_id);
                }
            }
        }
        if hint.is_empty() {
            if let Some(hint_id) = snap.get(label_id).and_then(|w| {
                w.children.iter().copied().find(|&id| {
                    snap.get(id)
                        .is_some_and(|c| c.props.class_names.iter().any(|n| n.contains("hint")))
                })
            }) {
                hint = collect_plain_text(snap, hint_id);
            }
        }
    }
    (label, hint)
}

fn settings_row_control_owned<Message: Clone + 'static>(
    snap: &SemanticSnapshot,
    children: &[WidgetId],
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message> {
    let control_id = find_child_with_class(snap, children, "nana-settings-row__control")
        .or_else(|| find_child_with_class(snap, children, "settings-row__control"));
    let Some(control_id) = control_id else {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    };
    let Some(control) = snap.get(control_id) else {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    };
    let child_box = control.props.layout.resolve_content_box(parent_box);
    let visible: Vec<WidgetId> = control
        .children
        .iter()
        .copied()
        .filter(|&id| is_layout_visible(snap, id))
        .collect();
    match visible.as_slice() {
        [] => space().width(Length::Shrink).height(Length::Shrink).into(),
        [only] => view_widget_owned(
            snap,
            *only,
            tokens,
            child_box,
            FlexDirection::Row,
            AlignSpec::Stretch,
            editors,
            menus,
            viewport,
            None,
            map_event,
        ),
        many => {
            let mut r = row![].spacing(8).align_y(Alignment::Center);
            for &id in many {
                r = r.push(view_widget_owned(
                    snap,
                    id,
                    tokens,
                    child_box,
                    FlexDirection::Row,
                    AlignSpec::Stretch,
                    editors,
                    menus,
                    viewport,
                    None,
                    map_event.clone(),
                ));
            }
            r.into()
        }
    }
}

fn settings_row_view_owned<Message: Clone + 'static>(
    snap: &SemanticSnapshot,
    props: &WidgetProps,
    children: &[WidgetId],
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message> {
    let (label, hint) = settings_row_label_hint(snap, props, children);
    let control = settings_row_control_owned(
        snap, children, tokens, parent_box, editors, menus, viewport, map_event,
    );
    let mut row = SettingsRow::new(label, control);
    if !hint.is_empty() {
        row = row.hint(hint);
    }
    row = row
        .stacked(class_token(props, "nana-settings-row--stacked"))
        .divided(
            class_token(props, "nana-settings-row--divided")
                || class_token(props, "settings-row--divided"),
        )
        .loose(class_token(props, "nana-settings-row__control--loose"));
    if class_token(props, "is-first") {
        row = row.first_in_group();
    }
    if class_token(props, "is-last") {
        row = row.last_in_group();
    }
    row.view(tokens)
}

fn settings_card_view_owned<Message: Clone + 'static>(
    snap: &SemanticSnapshot,
    props: &WidgetProps,
    children: &[WidgetId],
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message> {
    let mut title = props.display_label().to_string();
    if title.is_empty() {
        if let Some(title_id) = find_child_with_class(snap, children, "nana-settings-card__title") {
            title = collect_plain_text(snap, title_id);
        }
    }
    let body_id = find_child_with_class(snap, children, "nana-settings-card__body");
    let row_ids: Vec<WidgetId> = if let Some(body_id) = body_id {
        snap.get(body_id)
            .map(|w| {
                w.children
                    .iter()
                    .copied()
                    .filter(|&id| is_layout_visible(snap, id))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        children
            .iter()
            .copied()
            .filter(|&id| {
                is_layout_visible(snap, id)
                    && !snap.get(id).is_some_and(|w| {
                        w.props
                            .class_names
                            .iter()
                            .any(|c| c == "nana-settings-card__title")
                    })
            })
            .collect()
    };
    let section_box = ParentBox {
        width: parent_box
            .width
            .filter(|w| *w > 0.0)
            .or_else(|| (viewport.width > 0.0).then_some(viewport.width)),
        height: None,
    };
    let mut body = column![].spacing(0).width(Length::Fill);
    for id in row_ids {
        body = body.push(view_widget_owned(
            snap,
            id,
            tokens,
            section_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            viewport,
            None,
            map_event.clone(),
        ));
    }
    SettingsCard::new(title, body).view(tokens)
}

fn settings_row_view<'a, Message: Clone + 'a>(
    snap: &'a SemanticSnapshot,
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    let (label, hint) = settings_row_label_hint(snap, &widget.props, &widget.children);
    let control_id = find_child_with_class(snap, &widget.children, "nana-settings-row__control")
        .or_else(|| find_child_with_class(snap, &widget.children, "settings-row__control"));
    let control: Element<'a, Message> = if let Some(control_id) = control_id {
        let Some(control) = snap.get(control_id) else {
            return SettingsRow::new(label, space()).view(tokens);
        };
        let child_box = control.props.layout.resolve_content_box(parent_box);
        let visible: Vec<WidgetId> = control
            .children
            .iter()
            .copied()
            .filter(|&id| is_layout_visible(snap, id))
            .collect();
        match visible.as_slice() {
            [] => space().into(),
            [only] => view_widget(
                snap,
                *only,
                tokens,
                child_box,
                FlexDirection::Row,
                AlignSpec::Stretch,
                editors,
                menus,
                None,
                map_event,
            ),
            many => {
                let mut r = row![].spacing(8).align_y(Alignment::Center);
                for &id in many {
                    r = r.push(view_widget(
                        snap,
                        id,
                        tokens,
                        child_box,
                        FlexDirection::Row,
                        AlignSpec::Stretch,
                        editors,
                        menus,
                        None,
                        map_event.clone(),
                    ));
                }
                r.into()
            }
        }
    } else {
        space().into()
    };
    let mut row = SettingsRow::new(label, control);
    if !hint.is_empty() {
        row = row.hint(hint);
    }
    row = row
        .stacked(class_token(&widget.props, "nana-settings-row--stacked"))
        .divided(
            class_token(&widget.props, "nana-settings-row--divided")
                || class_token(&widget.props, "settings-row--divided"),
        );
    if class_token(&widget.props, "is-first") {
        row = row.first_in_group();
    }
    if class_token(&widget.props, "is-last") {
        row = row.last_in_group();
    }
    row.view(tokens)
}

fn settings_card_view<'a, Message: Clone + 'a>(
    snap: &'a SemanticSnapshot,
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    let mut title = widget.props.display_label().to_string();
    if title.is_empty() {
        if let Some(title_id) =
            find_child_with_class(snap, &widget.children, "nana-settings-card__title")
        {
            title = collect_plain_text(snap, title_id);
        }
    }
    let body_id = find_child_with_class(snap, &widget.children, "nana-settings-card__body");
    let row_ids: Vec<WidgetId> = if let Some(body_id) = body_id {
        snap.get(body_id)
            .map(|w| {
                w.children
                    .iter()
                    .copied()
                    .filter(|&id| is_layout_visible(snap, id))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        widget
            .children
            .iter()
            .copied()
            .filter(|&id| {
                is_layout_visible(snap, id)
                    && !snap.get(id).is_some_and(|w| {
                        w.props
                            .class_names
                            .iter()
                            .any(|c| c == "nana-settings-card__title")
                    })
            })
            .collect()
    };
    let section_box = ParentBox {
        width: parent_box.width,
        height: None,
    };
    let mut body = column![].spacing(0).width(Length::Fill);
    for id in row_ids {
        body = body.push(view_widget(
            snap,
            id,
            tokens,
            section_box,
            FlexDirection::Column,
            AlignSpec::Stretch,
            editors,
            menus,
            None,
            map_event.clone(),
        ));
    }
    SettingsCard::new(title, body).view(tokens)
}
