//! Per-widget-kit assembly: settings, markdown, workspace / dock / shell.
//!
//! Split out of `tree.rs`; these helpers turn semantic widgets (and their
//! host JSON props) into Runtime component specs and tree models.
use super::*;

pub(crate) fn host_tree_nodes(
    props: &crate::WidgetProps,
) -> Option<Vec<nana_ui_core::TreeNode<Arc<str>>>> {
    let value = props
        .native_props
        .get("tree")
        .or_else(|| props.native_props.get("nodes"))
        .or_else(|| props.native_props.get("options"))?;
    let nodes = host_array(value)
        .into_iter()
        .filter_map(tree_node_from_host)
        .collect::<Vec<_>>();
    (!nodes.is_empty()).then_some(nodes)
}

pub(crate) fn tree_node_from_host(
    value: &nana_js_engine::HostValue,
) -> Option<nana_ui_core::TreeNode<Arc<str>>> {
    match value {
        nana_js_engine::HostValue::Object(map) => {
            let id = host_map_text(map, &["value", "id", "key"]);
            let label = host_map_text(map, &["label", "title", "text"]);
            if id.is_empty() && label.is_empty() {
                return None;
            }
            let children = map
                .get("children")
                .map(host_array)
                .unwrap_or_default()
                .into_iter()
                .filter_map(tree_node_from_host)
                .collect::<Vec<_>>();
            let expanded = map.get("expanded").is_some_and(host_value_truthy);
            let selected = map.get("selected").is_some_and(host_value_truthy);
            let disabled = map.get("disabled").is_some_and(host_value_truthy);
            let identity = if id.is_empty() { label.clone() } else { id };
            let caption = if label.is_empty() {
                identity.clone()
            } else {
                label
            };
            let mut node = if children.is_empty() {
                nana_ui_core::TreeNode::leaf(Arc::<str>::from(identity.as_str()), caption)
            } else {
                nana_ui_core::TreeNode::branch(
                    Arc::<str>::from(identity.as_str()),
                    caption,
                    expanded,
                    children,
                )
            };
            node = node.selected(selected).disabled(disabled);
            if let Some(icon) = map
                .get("icon")
                .map(host_value_text)
                .filter(|name| !name.is_empty())
                .and_then(|name| nana_ui_core::Icon::parse_name(&name))
            {
                node = node.icon(icon);
            }
            Some(node)
        }
        nana_js_engine::HostValue::String(text) if !text.is_empty() => Some(
            nana_ui_core::TreeNode::leaf(Arc::<str>::from(text.as_str()), text.clone()),
        ),
        _ => None,
    }
}

pub(crate) fn settings_model_from_props(
    props: &crate::WidgetProps,
) -> Option<nana_ui_core::SettingsModel> {
    let map = settings_host_map(props)?;
    let mut tabs = settings_tabs_from_host(host_map_value(&map, &["tabs", "items"])?);
    if tabs.is_empty() {
        return None;
    }
    let full_page = settings_full_page_keys(&map);
    if !full_page.is_empty() {
        tabs = tabs
            .into_iter()
            .map(|tab| {
                let flagged =
                    tab.full_page_value() || full_page.iter().any(|key| key == tab.id().as_str());
                let mut next = nana_ui_core::SettingsTab::new(tab.id().clone(), tab.label());
                if let Some(icon) = tab.icon_value() {
                    next = next.icon(icon);
                }
                next.full_page(flagged)
            })
            .collect();
    }
    let default_tab = host_map_text(&map, &["defaultTab", "default_tab", "default-tab"]);
    let default_tab =
        if default_tab.is_empty() || tabs.iter().all(|tab| tab.id().as_str() != default_tab) {
            tabs[0].id().as_str().to_string()
        } else {
            default_tab
        };
    let mut model = nana_ui_core::SettingsModel::new(default_tab, tabs).ok()?;
    if let Some(nana_js_engine::HostValue::Object(aliases)) =
        host_map_value(&map, &["aliases", "alias"])
    {
        for (alias, target) in aliases {
            let target = host_value_text(target);
            if target.is_empty() {
                continue;
            }
            if let Ok(next) = model.clone().with_alias(alias.as_str(), target) {
                model = next;
            }
        }
    }
    let hide_header = host_map_truthy(&map, &["hideHeader", "hide_header", "hide-header"])
        || settings_hide_header_from_props(props);
    Some(model.hide_header(hide_header))
}

pub(crate) fn settings_host_map(
    props: &crate::WidgetProps,
) -> Option<BTreeMap<String, nana_js_engine::HostValue>> {
    if let Some(value) = props
        .native_props
        .get("settings")
        .or_else(|| props.native_props.get("model"))
    {
        return match value {
            nana_js_engine::HostValue::Object(map) => Some(map.clone()),
            nana_js_engine::HostValue::String(json) => settings_object_from_json(json),
            _ => None,
        };
    }
    crate::widget_map::attr_value(props, &["settings", "model"]).and_then(settings_object_from_json)
}

pub(crate) fn settings_object_from_json(
    json: &str,
) -> Option<BTreeMap<String, nana_js_engine::HostValue>> {
    match nana_js_engine::HostValue::from_json_str(json).ok()? {
        nana_js_engine::HostValue::Object(map) => Some(map),
        _ => None,
    }
}

pub(crate) fn settings_tabs_from_host(
    value: &nana_js_engine::HostValue,
) -> Vec<nana_ui_core::SettingsTab> {
    let items = host_array(value);
    if !items.is_empty() {
        return items
            .into_iter()
            .filter_map(settings_tab_from_host)
            .collect();
    }
    if let Some(tab) = settings_tab_from_host(value) {
        return vec![tab];
    }
    let nana_js_engine::HostValue::Object(map) = value else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, item)| match item {
            nana_js_engine::HostValue::String(label) if !label.is_empty() => {
                Some(nana_ui_core::SettingsTab::new(key.as_str(), label.as_str()))
            }
            nana_js_engine::HostValue::Object(_) => {
                let tab = settings_tab_from_host(item)?;
                if tab.id().as_str().is_empty() {
                    Some(nana_ui_core::SettingsTab::new(key.as_str(), tab.label()))
                } else {
                    Some(tab)
                }
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn settings_tab_from_host(
    value: &nana_js_engine::HostValue,
) -> Option<nana_ui_core::SettingsTab> {
    match value {
        nana_js_engine::HostValue::String(id) if !id.is_empty() => {
            Some(nana_ui_core::SettingsTab::new(id.as_str(), id.as_str()))
        }
        nana_js_engine::HostValue::Object(map) => {
            let id = host_map_text(map, &["key", "id", "value"]);
            if id.is_empty() {
                return None;
            }
            let label = host_map_text(map, &["label", "title", "name"]);
            let label = if label.is_empty() { id.clone() } else { label };
            let mut tab = nana_ui_core::SettingsTab::new(id, label);
            if let Some(icon) = nana_ui_core::Icon::parse_name(&host_map_text(
                map,
                &["icon", "iconName", "icon-name"],
            )) {
                tab = tab.icon(icon);
            }
            Some(tab.full_page(host_map_truthy(
                map,
                &["fullPage", "full_page", "full-page"],
            )))
        }
        _ => None,
    }
}

pub(crate) fn settings_full_page_keys(
    map: &BTreeMap<String, nana_js_engine::HostValue>,
) -> Vec<String> {
    host_map_value(map, &["fullPageTabs", "full_page_tabs", "full-page-tabs"])
        .map(host_array)
        .unwrap_or_default()
        .into_iter()
        .map(host_value_text)
        .filter(|key| !key.is_empty())
        .collect()
}

pub(crate) fn settings_hide_header_from_props(props: &crate::WidgetProps) -> bool {
    if let Some(value) = props
        .native_props
        .get("hide-header")
        .or_else(|| props.native_props.get("hideHeader"))
    {
        return host_value_truthy(value);
    }
    crate::widget_map::attr_value(props, &["hide-header", "hideheader", "hideHeader"]).is_some_and(
        |value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "true" | "1" | "yes"
            )
        },
    )
}

pub(crate) fn settings_active_tab(
    props: &crate::WidgetProps,
) -> Option<nana_ui_core::SettingsTabId> {
    native_prop_text(props, &["tab", "value"])
        .or_else(|| {
            crate::widget_map::attr_value(props, &["tab", "value"])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            let value = props.value.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
        .map(nana_ui_core::SettingsTabId::from)
}

pub(crate) fn settings_page_content_child(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Option<StableNodeId> {
    widget.children.iter().copied().find_map(|id| {
        let child = snapshot.get(id)?;
        if is_settings_page_header_child(child) {
            return None;
        }
        StableNodeId::new(id)
    })
}

pub(crate) fn is_settings_page_header_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget
        .props
        .attrs
        .get("data-slot")
        .map(String::as_str)
        .unwrap_or("");
    if matches!(slot, "header" | "page-header" | "title") {
        return true;
    }
    widget
        .props
        .class_names
        .iter()
        .any(|class| class.contains("nana-settings-page__header") || class.contains("page-header"))
}

pub(crate) fn markdown_source_from_props(props: &crate::WidgetProps) -> String {
    if !props.value.trim().is_empty() {
        return props.value.clone();
    }
    if let Some(source) = crate::widget_map::attr_value(props, &["source", "markdown", "value"])
        && !source.trim().is_empty()
    {
        return source.to_string();
    }
    for key in ["source", "markdown", "value"] {
        if let Some(text) = props.native_props.get(key).map(host_value_text)
            && !text.trim().is_empty()
        {
            return text;
        }
    }
    String::new()
}

pub(crate) fn markdown_renderer_request(
    markdown: &RuntimeNativeMarkdown,
    props: &crate::WidgetProps,
) -> Option<HighlightRequest> {
    let mermaid = crate::widget_map::mermaid_renderer(props)
        .map(str::to_string)
        .or_else(|| native_prop_text(props, &["mermaid-renderer", "mermaidrenderer"]));
    let math = crate::widget_map::math_renderer(props)
        .map(str::to_string)
        .or_else(|| native_prop_text(props, &["math-renderer", "mathrenderer"]));
    for block in markdown.blocks() {
        match block {
            nana_ui_runtime::MarkdownBlock::Mermaid(source) => {
                if let Some(name) = mermaid.as_deref() {
                    return Some(HighlightRequest::new(name, format!("mermaid:{source}")));
                }
            }
            nana_ui_runtime::MarkdownBlock::DisplayMath(source) => {
                if let Some(name) = math.as_deref() {
                    return Some(HighlightRequest::new(name, format!("math:{source}")));
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn native_prop_text(props: &crate::WidgetProps, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        props
            .native_props
            .get(*key)
            .map(host_value_text)
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    })
}

pub(crate) fn host_value_number(value: &nana_js_engine::HostValue) -> Option<f32> {
    match value {
        nana_js_engine::HostValue::Number(number) if number.is_finite() => Some(*number as f32),
        nana_js_engine::HostValue::String(text) => text.trim().parse().ok(),
        nana_js_engine::HostValue::Bool(true) => Some(1.0),
        nana_js_engine::HostValue::Bool(false) => Some(0.0),
        _ => None,
    }
}

pub(crate) fn host_array(value: &nana_js_engine::HostValue) -> Vec<&nana_js_engine::HostValue> {
    match value {
        nana_js_engine::HostValue::Array(items) => items.iter().collect(),
        nana_js_engine::HostValue::Object(map) => {
            let mut indexed = map
                .iter()
                .filter_map(|(key, item)| key.parse::<usize>().ok().map(|index| (index, item)))
                .collect::<Vec<_>>();
            if indexed.is_empty() {
                return Vec::new();
            }
            indexed.sort_by_key(|(index, _)| *index);
            indexed.into_iter().map(|(_, item)| item).collect()
        }
        _ => Vec::new(),
    }
}

pub(crate) fn host_map_value<'a>(
    map: &'a BTreeMap<String, nana_js_engine::HostValue>,
    keys: &[&str],
) -> Option<&'a nana_js_engine::HostValue> {
    for key in keys {
        if let Some(value) = map.get(*key) {
            return Some(value);
        }
    }
    for key in keys {
        if let Some((_, value)) = map
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        {
            return Some(value);
        }
    }
    None
}

pub(crate) fn host_map_text(
    map: &BTreeMap<String, nana_js_engine::HostValue>,
    keys: &[&str],
) -> String {
    host_map_value(map, keys)
        .map(host_value_text)
        .filter(|text| !text.is_empty())
        .unwrap_or_default()
}

pub(crate) fn host_map_truthy(
    map: &BTreeMap<String, nana_js_engine::HostValue>,
    keys: &[&str],
) -> bool {
    host_map_value(map, keys).is_some_and(host_value_truthy)
}

pub(crate) fn host_map_number(
    map: &BTreeMap<String, nana_js_engine::HostValue>,
    keys: &[&str],
) -> Option<f32> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(host_value_number))
}

pub(crate) fn host_value_text(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::String(text) => text.clone(),
        nana_js_engine::HostValue::Number(number) if number.is_finite() => number.to_string(),
        nana_js_engine::HostValue::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

pub(crate) fn host_value_truthy(value: &nana_js_engine::HostValue) -> bool {
    match value {
        nana_js_engine::HostValue::Bool(flag) => *flag,
        nana_js_engine::HostValue::Number(number) => *number != 0.0,
        nana_js_engine::HostValue::String(text) => {
            !text.is_empty() && !text.eq_ignore_ascii_case("false")
        }
        _ => false,
    }
}

pub(crate) fn is_shell_composer_slot(
    snapshot: &crate::SemanticSnapshot,
    widget: &crate::SemanticWidget,
) -> bool {
    widget
        .parent
        .and_then(|parent| snapshot.get(parent))
        .is_some_and(|parent| is_shell_composer_kind(effective_kind(parent)))
}

pub(crate) fn retain_projected_children(
    widget: &crate::SemanticWidget,
    parent: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    for child in &widget.children {
        let Some(child_id) = StableNodeId::new(*child) else {
            continue;
        };
        if !world.contains(child_id) {
            continue;
        }
        if world
            .node(child_id)
            .is_some_and(|node| node.parent != Some(parent))
        {
            mutations.insert(parent, child_id, None);
        }
    }
}

pub(crate) fn element_child_widgets<'a>(
    widget: &'a crate::SemanticWidget,
    snapshot: &'a crate::SemanticSnapshot,
) -> Vec<&'a crate::SemanticWidget> {
    widget
        .children
        .iter()
        .filter_map(|child| snapshot.get(*child))
        .filter(|child| child.kind != crate::WidgetKind::Text)
        .collect()
}

pub(crate) fn widget_slot(widget: &crate::SemanticWidget) -> String {
    crate::widget_map::attr_value(&widget.props, &["data-slot", "slot"])
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

pub(crate) fn widget_tag(widget: &crate::SemanticWidget) -> String {
    widget.props.element_tag.trim().to_ascii_lowercase()
}

pub(crate) fn widget_class_blob(widget: &crate::SemanticWidget) -> String {
    widget
        .props
        .class_names
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(crate) fn is_title_bar_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget_slot(widget);
    if matches!(slot.as_str(), "title-bar" | "titlebar" | "app-title-bar") {
        return true;
    }
    let tag = widget_tag(widget);
    let class = widget_class_blob(widget);
    tag.contains("title-bar")
        || tag.contains("titlebar")
        || tag.contains("app-title-bar")
        || class.contains("title-bar")
        || class.contains("titlebar")
        || class.contains("nana-app-title-bar")
}

pub(crate) fn is_body_child(widget: &crate::SemanticWidget) -> bool {
    widget_slot(widget) == "body"
}

pub(crate) fn is_overlay_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget_slot(widget);
    if slot == "overlay" {
        return true;
    }
    let tag = widget_tag(widget);
    let class = widget_class_blob(widget);
    tag.contains("overlay") || class.contains("nana-app-shell__overlay")
}

pub(crate) fn is_split_handle_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget_slot(widget);
    if matches!(slot.as_str(), "handle" | "split-handle") {
        return true;
    }
    let tag = widget_tag(widget);
    let class = widget_class_blob(widget);
    tag.contains("split-handle")
        || class.contains("nana-split-handle")
        || class.contains("split-handle")
}

pub(crate) fn native_prop_number(props: &crate::WidgetProps, keys: &[&str]) -> Option<f32> {
    keys.iter().find_map(|key| {
        props
            .native_props
            .get(*key)
            .and_then(host_value_number)
            .filter(|number| number.is_finite())
    })
}

pub(crate) fn split_axis_from_props(props: &crate::WidgetProps) -> nana_ui_core::SplitAxis {
    let raw = native_prop_text(props, &["axis"])
        .or_else(|| {
            crate::widget_map::attr_value(props, &["axis", "data-axis"]).map(str::to_string)
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    match raw.as_str() {
        "vertical" | "column" | "y" => nana_ui_core::SplitAxis::Vertical,
        _ => nana_ui_core::SplitAxis::Horizontal,
    }
}

pub(crate) fn split_pane_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    context: &AppContext,
    id: StableNodeId,
) -> RuntimeSplitPane {
    let children = element_child_widgets(widget, snapshot);
    let handle = children
        .iter()
        .find(|child| is_split_handle_child(child))
        .and_then(|child| StableNodeId::new(child.id))
        .or_else(|| {
            (children.len() >= 3)
                .then(|| StableNodeId::new(children[2].id))
                .flatten()
        })
        .or_else(|| {
            context
                .read(Entity::<RuntimeSplitPane>::from_stable_id(id), |pane| {
                    pane.handle
                })
                .ok()
                .flatten()
        });
    let panes = children
        .iter()
        .filter(|child| Some(child.id) != handle.map(StableNodeId::get))
        .filter_map(|child| StableNodeId::new(child.id))
        .collect::<Vec<_>>();
    let first = panes.first().copied();
    let second = panes.get(1).copied();
    let default_size = native_prop_number(&widget.props, &["default-size", "defaultsize"])
        .or_else(|| native_prop_number(&widget.props, &["size"]))
        .unwrap_or(240.0);
    let min = native_prop_number(&widget.props, &["min"]).unwrap_or(120.0);
    let max = native_prop_number(&widget.props, &["max"]).unwrap_or(800.0);
    let mut model = nana_ui_core::SplitPaneModel::new(
        split_axis_from_props(&widget.props),
        default_size,
        min,
        max,
    );
    if let Some(size) = native_prop_number(&widget.props, &["size"])
        && (size - model.size()).abs() > f32::EPSILON
    {
        model.update(nana_ui_core::SplitPaneMutation::SetSize(size));
    }
    match (first, second) {
        (Some(first), Some(second)) => {
            let mut pane = RuntimeSplitPane::from_model(&model, first, second);
            if let Some(handle) = handle {
                pane = pane.handle(handle);
            }
            pane
        }
        _ => RuntimeSplitPane {
            first,
            second,
            handle,
            model,
            style: NodeStyle::default(),
        },
    }
}

pub(crate) fn workspace_region_token(widget: &crate::SemanticWidget) -> String {
    if !widget.props.region.is_empty() {
        return widget.props.region.to_ascii_lowercase();
    }
    crate::widget_map::attr_value(
        &widget.props,
        &[
            "data-region-role",
            "data-region",
            "region",
            "region-role",
            "data-region-id",
        ],
    )
    .unwrap_or("")
    .trim()
    .to_ascii_lowercase()
}

pub(crate) fn region_id_from_token(token: &str) -> Option<nana_ui_core::RegionId> {
    let token = token.trim().trim_start_matches("region-");
    if token.is_empty() {
        return None;
    }
    Some(match token {
        "global-navigation" | "globalnavigation" | "global" => {
            nana_ui_core::RegionId::GlobalNavigation
        }
        "section-navigation" | "sectionnavigation" => nana_ui_core::RegionId::SectionNavigation,
        "resources" | "sidebar" | "files" => nana_ui_core::RegionId::Resources,
        "primary-toolbar" | "primarytoolbar" | "toolbar" => nana_ui_core::RegionId::PrimaryToolbar,
        "primary" | "main" => nana_ui_core::RegionId::Primary,
        "inspector" => nana_ui_core::RegionId::Inspector,
        "diagnostics" | "console" => nana_ui_core::RegionId::Diagnostics,
        other => nana_ui_core::RegionId::custom(other),
    })
}

pub(crate) fn workspace_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    context: &AppContext,
    id: StableNodeId,
) -> RuntimeWorkspace {
    let slots = workspace_slots_from_widget(widget, snapshot);
    let mut component = RuntimeWorkspace::from_model(&nana_ui_core::WorkspaceModel::new(), slots);
    if let Ok(existing) = context.read(Entity::<RuntimeWorkspace>::from_stable_id(id), Clone::clone)
    {
        component.middle = existing.middle;
        component.primary_column = existing.primary_column;
        component.primary_row = existing.primary_row;
        component.editor_stack = existing.editor_stack;
        component.handles = existing.handles;
    }
    component
}

pub(crate) fn workspace_slots_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Vec<WorkspaceRegionSlot> {
    let children = element_child_widgets(widget, snapshot);
    let mut slots = Vec::new();
    for (index, child) in children.iter().enumerate() {
        let Some(content) = StableNodeId::new(child.id) else {
            continue;
        };
        let token = workspace_region_token(child);
        let region = if let Some(region) = region_id_from_token(&token) {
            region
        } else {
            let fallback = if !child.props.element_id.is_empty() {
                child.props.element_id.clone()
            } else if !child.props.value.is_empty() {
                child.props.value.clone()
            } else {
                format!("region-{index}")
            };
            nana_ui_core::RegionId::custom(fallback)
        };
        if slots
            .iter()
            .any(|slot: &WorkspaceRegionSlot| slot.id == region)
        {
            continue;
        }
        slots.push(WorkspaceRegionSlot::new(region, content));
    }
    slots
}

pub(crate) fn dock_item_id(widget: &crate::SemanticWidget, index: usize) -> String {
    crate::widget_map::attr_value(&widget.props, &["data-dock-id", "dock-id", "id", "data-id"])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| (!widget.props.element_id.is_empty()).then(|| widget.props.element_id.clone()))
        .or_else(|| (!widget.props.value.is_empty()).then(|| widget.props.value.clone()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let label = widget.props.display_label();
            if label.is_empty() {
                format!("item-{index}")
            } else {
                label.to_string()
            }
        })
}

pub(crate) fn dock_item_title(widget: &crate::SemanticWidget, id: &str) -> String {
    let title = widget.props.display_label();
    if title.is_empty() {
        id.to_string()
    } else {
        title.to_string()
    }
}

pub(crate) fn dock_content_for_id(
    id: &str,
    children: &[(&crate::SemanticWidget, StableNodeId)],
) -> Option<StableNodeId> {
    children.iter().find_map(|(child, node)| {
        let child_id = dock_item_id(child, 0);
        (child_id == id).then_some(*node)
    })
}

pub(crate) fn parse_dock_axis(raw: &str) -> DockAxis {
    match raw.trim().to_ascii_lowercase().as_str() {
        "vertical" | "column" | "y" => DockAxis::Vertical,
        _ => DockAxis::Horizontal,
    }
}

pub(crate) fn dock_node_from_host(
    value: &nana_js_engine::HostValue,
    children: &[(&crate::SemanticWidget, StableNodeId)],
) -> Option<DockNode> {
    match value {
        nana_js_engine::HostValue::String(id) if !id.is_empty() => Some(DockNode::item(
            id.as_str(),
            dock_content_for_id(id, children),
        )),
        nana_js_engine::HostValue::Array(items) => {
            let nodes = items
                .iter()
                .filter_map(|item| dock_node_from_host(item, children))
                .collect::<Vec<_>>();
            dock_nodes_join(nodes)
        }
        nana_js_engine::HostValue::Object(map) => {
            let kind = host_map_text(map, &["type", "kind", "node"]).to_ascii_lowercase();
            if kind == "split" || map.contains_key("first") || map.contains_key("second") {
                let first = map
                    .get("first")
                    .and_then(|value| dock_node_from_host(value, children))?;
                let second = map
                    .get("second")
                    .and_then(|value| dock_node_from_host(value, children))?;
                let axis = parse_dock_axis(&host_map_text(map, &["axis"]));
                let ratio = host_map_number(map, &["ratio", "size"]).unwrap_or(0.5);
                return Some(DockNode::split(axis, ratio, first, second));
            }
            if kind == "tabs" || map.contains_key("tabs") {
                let tab_values = map.get("tabs").map(host_array).unwrap_or_default();
                let mut tabs = Vec::new();
                let mut contents = Vec::new();
                for tab in tab_values {
                    let (id, content) = match tab {
                        nana_js_engine::HostValue::String(id) if !id.is_empty() => {
                            (id.clone(), dock_content_for_id(id, children))
                        }
                        nana_js_engine::HostValue::Object(tab_map) => {
                            let id = host_map_text(tab_map, &["id", "key", "value"]);
                            if id.is_empty() {
                                continue;
                            }
                            (id.clone(), dock_content_for_id(&id, children))
                        }
                        _ => continue,
                    };
                    if tabs
                        .iter()
                        .any(|existing: &Arc<str>| existing.as_ref() == id)
                    {
                        continue;
                    }
                    let id = Arc::<str>::from(id);
                    contents.push((Arc::clone(&id), content));
                    tabs.push(id);
                }
                if tabs.is_empty() {
                    return None;
                }
                let active = host_map_text(map, &["active", "value"]);
                let active = if active.is_empty() {
                    Arc::clone(&tabs[0])
                } else {
                    Arc::<str>::from(active)
                };
                return Some(DockNode::tabs(tabs, active, contents));
            }
            let id = host_map_text(map, &["id", "key", "value", "dock-id", "data-dock-id"]);
            if id.is_empty() {
                return None;
            }
            Some(DockNode::item(
                id.as_str(),
                dock_content_for_id(&id, children),
            ))
        }
        _ => None,
    }
}

pub(crate) fn dock_nodes_join(mut nodes: Vec<DockNode>) -> Option<DockNode> {
    match nodes.len() {
        0 => None,
        1 => nodes.pop(),
        _ => {
            let ids = nodes.iter().flat_map(DockNode::flatten).collect::<Vec<_>>();
            if ids.is_empty() {
                return None;
            }
            let mut contents = Vec::new();
            for node in &nodes {
                collect_dock_contents(node, &mut contents);
            }
            let active = Arc::clone(&ids[0]);
            Some(DockNode::tabs(ids, active, contents))
        }
    }
}

pub(crate) fn collect_dock_contents(
    node: &DockNode,
    output: &mut Vec<(Arc<str>, Option<StableNodeId>)>,
) {
    match node {
        DockNode::Item { id, content } => output.push((Arc::clone(id), *content)),
        DockNode::Tabs { contents, .. } => output.extend(contents.iter().cloned()),
        DockNode::Split { first, second, .. } => {
            collect_dock_contents(first, output);
            collect_dock_contents(second, output);
        }
    }
}

pub(crate) fn dock_root_from_children(
    children: &[(&crate::SemanticWidget, StableNodeId)],
) -> (DockNode, Vec<(Arc<str>, Arc<str>)>) {
    let mut titles = Vec::new();
    let mut items = Vec::new();
    for (index, (child, content)) in children.iter().enumerate() {
        let id = dock_item_id(child, index);
        let title = dock_item_title(child, &id);
        titles.push((Arc::<str>::from(id.as_str()), Arc::<str>::from(title)));
        items.push((Arc::<str>::from(id), Some(*content)));
    }
    let root = match items.as_slice() {
        [] => DockNode::item("dock", None),
        [(id, content)] => DockNode::item(Arc::clone(id), *content),
        _ => {
            let tabs = items
                .iter()
                .map(|(id, _)| Arc::clone(id))
                .collect::<Vec<_>>();
            let active = Arc::clone(&tabs[0]);
            DockNode::tabs(tabs, active, items)
        }
    };
    (root, titles)
}

pub(crate) fn dock_from_widget_bound(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    context: &AppContext,
    id: StableNodeId,
) -> RuntimeDock {
    let next = dock_from_widget(widget, snapshot);
    match context.read(Entity::<RuntimeDock>::from_stable_id(id), Clone::clone) {
        Ok(mut existing) => {
            existing.root = next.root;
            existing.drop_target = next.drop_target;
            existing.locked = next.locked;
            existing.titles = next.titles;
            existing.style = next.style;
            existing
        }
        Err(_) => next,
    }
}

pub(crate) fn dock_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> RuntimeDock {
    let child_pairs = element_child_widgets(widget, snapshot)
        .into_iter()
        .filter_map(|child| StableNodeId::new(child.id).map(|id| (child, id)))
        .collect::<Vec<_>>();
    let host_root = widget
        .props
        .native_props
        .get("root")
        .or_else(|| widget.props.native_props.get("layout"))
        .and_then(|value| dock_node_from_host(value, &child_pairs));
    let (root, titles) = if let Some(root) = host_root {
        let mut titles = Vec::new();
        for (child, _) in &child_pairs {
            let id = dock_item_id(child, 0);
            let title = dock_item_title(child, &id);
            if title != id {
                titles.push((Arc::<str>::from(id), Arc::<str>::from(title)));
            }
        }
        (root, titles)
    } else {
        dock_root_from_children(&child_pairs)
    };
    let mut dock = RuntimeDock::new(root);
    for (id, title) in titles {
        dock = dock.title(id, title);
    }
    dock
}

pub(crate) fn app_shell_slots(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> (
    Option<StableNodeId>,
    Option<StableNodeId>,
    Option<StableNodeId>,
) {
    let children = element_child_widgets(widget, snapshot);
    let title_bar = children
        .iter()
        .find(|child| is_title_bar_child(child))
        .and_then(|child| StableNodeId::new(child.id));
    let overlay = children
        .iter()
        .find(|child| is_overlay_child(child) && Some(child.id) != title_bar.map(StableNodeId::get))
        .and_then(|child| StableNodeId::new(child.id));
    let body = children
        .iter()
        .find(|child| {
            is_body_child(child)
                && Some(child.id) != title_bar.map(StableNodeId::get)
                && Some(child.id) != overlay.map(StableNodeId::get)
        })
        .or_else(|| {
            children.iter().find(|child| {
                let id = child.id;
                Some(id) != title_bar.map(StableNodeId::get)
                    && Some(id) != overlay.map(StableNodeId::get)
            })
        })
        .and_then(|child| StableNodeId::new(child.id))
        .or_else(|| {
            title_bar.and_then(|title_bar| {
                snapshot.get(title_bar.get()).and_then(|bar| {
                    element_child_widgets(bar, snapshot)
                        .into_iter()
                        .find(|child| is_body_child(child))
                        .and_then(|child| StableNodeId::new(child.id))
                })
            })
        });
    (title_bar, body, overlay)
}

pub(crate) fn normalize_event_name(event: &str) -> String {
    let e = event.trim();
    let lower = if let Some(rest) = e.strip_prefix("on").or_else(|| e.strip_prefix("On")) {
        rest
    } else {
        e
    };
    lower.to_ascii_lowercase()
}

/// Minimal trusted HTML fragment for Vue static content (compiler output only).
#[derive(Debug, Clone)]
pub(crate) enum FragNode {
    Text(String),
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<FragNode>,
    },
}

pub(crate) fn decode_basic_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", "\u{00A0}")
        .replace("&amp;", "&")
}

pub(crate) fn parse_html_fragment(html: &str) -> Vec<FragNode> {
    let bytes = html.as_bytes();
    let mut i = 0usize;
    let mut roots = Vec::new();
    parse_html_children(bytes, &mut i, &mut roots, None);
    roots
}

pub(crate) fn is_void_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

pub(crate) fn parse_html_children(
    bytes: &[u8],
    i: &mut usize,
    out: &mut Vec<FragNode>,
    stop_tag: Option<&str>,
) {
    while *i < bytes.len() {
        if bytes[*i] == b'<' {
            if bytes.get(*i + 1) == Some(&b'/') {
                let start = *i + 2;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'>' {
                    end += 1;
                }
                let name = std::str::from_utf8(&bytes[start..end])
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                *i = (end + 1).min(bytes.len());
                if stop_tag.is_some_and(|t| t == name) {
                    return;
                }
                continue;
            }
            if bytes.get(*i + 1) == Some(&b'!') {
                *i += 2;
                while *i < bytes.len() && bytes[*i] != b'>' {
                    *i += 1;
                }
                *i = (*i + 1).min(bytes.len());
                continue;
            }
            *i += 1;
            let tag_start = *i;
            while *i < bytes.len()
                && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'-' || bytes[*i] == b':')
            {
                *i += 1;
            }
            let tag = std::str::from_utf8(&bytes[tag_start..*i])
                .unwrap_or("")
                .to_ascii_lowercase();
            if tag.is_empty() {
                continue;
            }
            // Attributes until `>` or `/>`
            let attr_start = *i;
            let mut self_closing = false;
            while *i < bytes.len() {
                if bytes[*i] == b'>' {
                    if *i > attr_start && bytes[*i - 1] == b'/' {
                        self_closing = true;
                    }
                    break;
                }
                *i += 1;
            }
            let attr_end = if self_closing { *i - 1 } else { *i };
            let attr_str = std::str::from_utf8(&bytes[attr_start..attr_end]).unwrap_or("");
            let attrs = parse_html_attrs(attr_str);
            *i = (*i + 1).min(bytes.len());
            let void = self_closing || is_void_html_tag(&tag);
            let mut children = Vec::new();
            if !void {
                parse_html_children(bytes, i, &mut children, Some(&tag));
            }
            out.push(FragNode::Element {
                tag,
                attrs,
                children,
            });
            continue;
        }
        let start = *i;
        while *i < bytes.len() && bytes[*i] != b'<' {
            *i += 1;
        }
        if *i > start {
            let text = std::str::from_utf8(&bytes[start..*i]).unwrap_or("");
            if !text.is_empty() {
                out.push(FragNode::Text(decode_basic_entities(text)));
            }
        }
    }
}

pub(crate) fn parse_html_attrs(attr_str: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = attr_str.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let name = std::str::from_utf8(&bytes[name_start..i])
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.is_empty() {
            break;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if bytes.get(i) == Some(&b'"') || bytes.get(i) == Some(&b'\'') {
                let quote = bytes[i];
                i += 1;
                let vstart = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                let v = std::str::from_utf8(&bytes[vstart..i]).unwrap_or("");
                if i < bytes.len() {
                    i += 1;
                }
                decode_basic_entities(v)
            } else {
                let vstart = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
                    i += 1;
                }
                decode_basic_entities(std::str::from_utf8(&bytes[vstart..i]).unwrap_or(""))
            }
        } else {
            String::new()
        };
        out.push((name, value));
    }
    out
}

pub(crate) fn selector_list_matches(sel: &str, tag: &str, attrs: &HashMap<String, String>) -> bool {
    sel.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|simple| selector_matches(simple, tag, attrs))
}

/// Match one compound simple selector: optional tag + `.class*` + optional `#id` + `[attr]*`.
pub(crate) fn selector_matches(sel: &str, tag: &str, attrs: &HashMap<String, String>) -> bool {
    let sel = sel.trim();
    if sel.is_empty() {
        return false;
    }
    let bytes = sel.as_bytes();
    let mut i = 0usize;
    // Optional type selector
    if bytes[0].is_ascii_alphabetic() || bytes[0] == b'*' {
        let start = i;
        i += 1;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
        {
            i += 1;
        }
        let type_sel = &sel[start..i];
        if type_sel != "*" && !tag.eq_ignore_ascii_case(type_sel) {
            return false;
        }
    }
    while i < bytes.len() {
        match bytes[i] {
            b'.' => {
                i += 1;
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
                {
                    i += 1;
                }
                if start == i {
                    return false;
                }
                let class = &sel[start..i];
                if !attrs
                    .get("class")
                    .map(|c| c.split_whitespace().any(|x| x == class))
                    .unwrap_or(false)
                {
                    return false;
                }
            }
            b'#' => {
                i += 1;
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
                {
                    i += 1;
                }
                if start == i {
                    return false;
                }
                let id = &sel[start..i];
                if attrs.get("id").map(|v| v.as_str()) != Some(id) {
                    return false;
                }
            }
            b'[' => {
                let Some(rel_end) = sel[i..].find(']') else {
                    return false;
                };
                let end = i + rel_end;
                let inner = sel[i + 1..end].trim();
                if let Some((k, v)) = inner.split_once('=') {
                    let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                    if attrs.get(k.trim()).map(|x| x.as_str()) != Some(v) {
                        return false;
                    }
                } else if !attrs.contains_key(inner) {
                    return false;
                }
                i = end + 1;
            }
            _ => return false,
        }
    }
    // Bare tag already handled; empty remainder after type means tag-only match.
    // If we never consumed a type and never entered the loop, treat as tag name
    // (legacy path: `body`, `html`, `div`).
    if i == 0 {
        return tag.eq_ignore_ascii_case(sel);
    }
    true
}
