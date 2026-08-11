// Select / Tabs / Segmented / Textarea adapters.

fn selection_select<'a, Message>(
    widget: &'a crate::bridge::SemanticWidget,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let id = widget.id;
    let map = map_event;
    let options: Vec<SelectionOption<'_, String>> = widget
        .props
        .options
        .iter()
        .map(|o| {
            let mut opt = SelectionOption::new(o.value.clone(), o.label.as_str());
            if o.disabled {
                opt = opt.disabled(true);
            }
            opt
        })
        .collect();
    let value = if widget.props.value.is_empty() {
        None
    } else {
        Some(widget.props.value.clone())
    };
    if options.is_empty() {
        return text(widget.props.display_label()).size(13).into();
    }
    let mut select = Select::new(value, options, move |v| {
        map(BridgeEvent::SelectValue { id, value: v })
    })
    .size(widget.props.size)
    .disabled(widget.props.disabled)
    .loading(widget.props.loading)
    .invalid(widget.props.invalid);
    if !widget.props.placeholder.is_empty() {
        select = select.placeholder(widget.props.placeholder.as_str());
    } else if !widget.props.hint.is_empty() {
        select = select.placeholder(widget.props.hint.as_str());
    }
    select.view(tokens)
}

fn selection_select_owned<Message>(
    props: &crate::bridge::WidgetProps,
    id: WidgetId,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let map = map_event;
    let options: Vec<SelectionOption<'static, String>> = props
        .options
        .iter()
        .map(|o| {
            let mut opt = SelectionOption::new(o.value.clone(), o.label.clone());
            if o.disabled {
                opt = opt.disabled(true);
            }
            opt
        })
        .collect();
    let value = if props.value.is_empty() {
        None
    } else {
        Some(props.value.clone())
    };
    if options.is_empty() {
        return text(owned_display(props)).into();
    }
    let mut select = Select::new(value, options, move |v| {
        map(BridgeEvent::SelectValue { id, value: v })
    })
    .size(props.size)
    .disabled(props.disabled)
    .loading(props.loading)
    .invalid(props.invalid);
    if !props.placeholder.is_empty() {
        select = select.placeholder(props.placeholder.clone());
    } else if !props.hint.is_empty() {
        select = select.placeholder(props.hint.clone());
    }
    select.view(tokens)
}

fn textarea_view<'a, Message>(
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    editors: Option<&'a EditorStore>,
    _menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let id = widget.id;
    let placeholder = if widget.props.placeholder.is_empty() {
        widget.props.hint.as_str()
    } else {
        widget.props.placeholder.as_str()
    };
    let Some(store) = editors else {
        let map = map_event;
        return Input::new(placeholder, widget.props.value.as_str())
            .size(widget.props.size)
            .disabled(widget.props.disabled)
            .on_input(move |value| map(BridgeEvent::Input { id, value }))
            .view(tokens);
    };
    let Some(content) = store.get(id) else {
        let map = map_event;
        return Input::new(placeholder, widget.props.value.as_str())
            .size(widget.props.size)
            .disabled(widget.props.disabled)
            .on_input(move |value| map(BridgeEvent::Input { id, value }))
            .view(tokens);
    };
    let map = map_event;
    let mut editor = Textarea::new(content)
        .placeholder(placeholder)
        .disabled(widget.props.disabled)
        .invalid(widget.props.invalid)
        .on_action(move |action| map(BridgeEvent::Editor { id, action }));
    if let Some(LengthSpec::Px(h)) = widget.props.layout.height {
        editor = editor.height(h);
    }
    editor.view(tokens)
}

fn textarea_view_owned<Message>(
    props: &crate::bridge::WidgetProps,
    id: WidgetId,
    tokens: ThemeTokens,
    editors: Option<&EditorStore>,
    _menus: Option<&MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let placeholder = if props.placeholder.is_empty() {
        props.hint.clone()
    } else {
        props.placeholder.clone()
    };
    let Some(store) = editors else {
        let map = map_event;
        return Input::new(placeholder, props.value.clone())
            .size(props.size)
            .disabled(props.disabled)
            .on_input(move |value| map(BridgeEvent::Input { id, value }))
            .view(tokens);
    };
    let Some(content) = store.content_static(id) else {
        let map = map_event;
        return Input::new(placeholder, props.value.clone())
            .size(props.size)
            .disabled(props.disabled)
            .on_input(move |value| map(BridgeEvent::Input { id, value }))
            .view(tokens);
    };
    let map = map_event;
    let mut editor = Textarea::new(content)
        .placeholder(placeholder)
        .disabled(props.disabled)
        .invalid(props.invalid)
        .on_action(move |action| map(BridgeEvent::Editor { id, action }));
    if let Some(LengthSpec::Px(h)) = props.layout.height {
        editor = editor.height(h);
    }
    editor.view(tokens)
}

fn selection_tabs<'a, Message>(
    widget: &'a crate::bridge::SemanticWidget,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let id = widget.id;
    let map = map_event;
    let options: Vec<SelectionOption<'_, String>> = widget
        .props
        .options
        .iter()
        .map(|o| {
            let mut opt = SelectionOption::new(o.value.clone(), o.label.as_str());
            if o.disabled {
                opt = opt.disabled(true);
            }
            opt
        })
        .collect();
    let value = if widget.props.value.is_empty() {
        options.first().map(|o| o.value.clone()).unwrap_or_default()
    } else {
        widget.props.value.clone()
    };
    if options.is_empty() {
        return text(widget.props.display_label()).into();
    }
    let mut tabs = Tabs::new(value, options, move |v| {
        map(BridgeEvent::SelectValue { id, value: v })
    })
    .size(widget.props.size);
    if widget.props.fill {
        tabs = tabs.fill();
    }
    tabs.view(tokens)
}

fn selection_segmented<'a, Message>(
    widget: &'a crate::bridge::SemanticWidget,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let id = widget.id;
    let map = map_event;
    let options: Vec<SelectionOption<'_, String>> = widget
        .props
        .options
        .iter()
        .map(|o| {
            let mut opt = SelectionOption::new(o.value.clone(), o.label.as_str());
            if o.disabled {
                opt = opt.disabled(true);
            }
            opt
        })
        .collect();
    let value = if widget.props.value.is_empty() {
        options.first().map(|o| o.value.clone()).unwrap_or_default()
    } else {
        widget.props.value.clone()
    };
    if options.is_empty() {
        return Button::label(widget.props.display_label())
            .kind(ButtonKind::Subtle)
            .view(tokens);
    }
    SegmentedControl::new(value, options, move |v| {
        map(BridgeEvent::SelectValue { id, value: v })
    })
    .size(widget.props.size)
    .view(tokens)
}

fn selection_tabs_owned<Message>(
    props: &crate::bridge::WidgetProps,
    id: WidgetId,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let map = map_event;
    let options: Vec<SelectionOption<'static, String>> = props
        .options
        .iter()
        .map(|o| {
            let mut opt = SelectionOption::new(o.value.clone(), o.label.clone());
            if o.disabled {
                opt = opt.disabled(true);
            }
            opt
        })
        .collect();
    let value = if props.value.is_empty() {
        options.first().map(|o| o.value.clone()).unwrap_or_default()
    } else {
        props.value.clone()
    };
    if options.is_empty() {
        return text(owned_display(props)).into();
    }
    let mut tabs = Tabs::new(value, options, move |v| {
        map(BridgeEvent::SelectValue { id, value: v })
    })
    .size(props.size);
    if props.fill {
        tabs = tabs.fill();
    }
    tabs.view(tokens)
}

fn selection_segmented_owned<Message>(
    props: &crate::bridge::WidgetProps,
    id: WidgetId,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let map = map_event;
    let options: Vec<SelectionOption<'static, String>> = props
        .options
        .iter()
        .map(|o| {
            let mut opt = SelectionOption::new(o.value.clone(), o.label.clone());
            if o.disabled {
                opt = opt.disabled(true);
            }
            opt
        })
        .collect();
    let value = if props.value.is_empty() {
        options.first().map(|o| o.value.clone()).unwrap_or_default()
    } else {
        props.value.clone()
    };
    if options.is_empty() {
        return Button::label(owned_display(props))
            .kind(ButtonKind::Subtle)
            .view(tokens);
    }
    SegmentedControl::new(value, options, move |v| {
        map(BridgeEvent::SelectValue { id, value: v })
    })
    .size(props.size)
    .view(tokens)
}

fn owned_display(props: &crate::bridge::WidgetProps) -> String {
    let s = if !props.label.is_empty() {
        props.label.clone()
    } else {
        props.value.clone()
    };
    if s == "[object Object]" {
        String::new()
    } else {
        s
    }
}

/// Resolved iced text px for [`label_text`]: CSS `font-size` wins; otherwise
/// [`ControlSize::text_size`] (`UI_BASE_TEXT_SIZE` ± 1), matching `ui_font_defaults`.
fn label_text_size_px(size: ControlSize, layout: &crate::css_map::LayoutStyle) -> f32 {
    layout.font_size.unwrap_or(size.text_size()).max(1.0)
}
