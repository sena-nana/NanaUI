//! Semantic widget → Runtime component resolution and binding.
//!
//! Split out of `tree.rs`: the ordered rule table, tag resolution and the
//! per-component bind helpers behind `try_bind_registered_component`.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;

pub(crate) fn is_sidebar_frame_body(widget: &SemanticWidgetView<'_>) -> bool {
    crate::scroll::is_runtime_scroll_body(&widget.props)
}

pub(crate) fn widget_icon(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> Option<nana_ui_core::Icon> {
    widget
        .children
        .iter()
        .filter_map(|child| snapshot.get(*child))
        .find(|child| child.kind == crate::WidgetKind::Icon)
        .and_then(|widget| glyph_name_icon(&widget))
        .or_else(|| glyph_name_icon(widget))
}

pub(crate) fn glyph_name_icon(widget: &SemanticWidgetView<'_>) -> Option<nana_ui_core::Icon> {
    nana_ui_core::Icon::parse_name(widget.props.display_label())
        .or_else(|| nana_ui_core::Icon::parse_name(&widget.props.value))
        .or_else(|| {
            widget
                .props
                .class_names
                .iter()
                .find_map(|class| nana_ui_core::Icon::parse_name(class))
        })
}

pub(crate) fn icon_consumed_by_parent(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> bool {
    let Some(parent) = widget.parent.and_then(|parent| snapshot.get(parent)) else {
        return false;
    };
    match parent.kind {
        crate::WidgetKind::Button => widget_icon(&parent, snapshot).is_some(),
        crate::WidgetKind::EmptyState => true,
        _ => false,
    }
}

/// Outcome of one ordered component-assembly rule.
pub(crate) enum ComponentRuleOutcome {
    /// Try this tag; if the registry has no such component, fall through to
    /// the next rule / the element-tag chain.
    Tag(&'static str),
    /// This node must not project any component.
    Veto,
}

/// Rule signature: `(kind + prop composition) → tag`.
pub(crate) type ComponentRule =
    fn(&SemanticWidgetView<'_>, &SemanticRead<'_>) -> Option<ComponentRuleOutcome>;

pub(crate) fn rule_button_with_icon(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> Option<ComponentRuleOutcome> {
    (widget.kind == crate::WidgetKind::Button && widget_icon(widget, snapshot).is_some())
        .then_some(ComponentRuleOutcome::Tag("icon-button"))
}

pub(crate) fn rule_chip_variant(
    widget: &SemanticWidgetView<'_>,
    _snapshot: &SemanticRead<'_>,
) -> Option<ComponentRuleOutcome> {
    (widget.kind == crate::WidgetKind::Chip).then_some(ComponentRuleOutcome::Tag("chip"))
}

pub(crate) fn rule_standalone_icon(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> Option<ComponentRuleOutcome> {
    if widget.kind != crate::WidgetKind::Icon {
        return None;
    }
    if icon_consumed_by_parent(widget, snapshot) || glyph_name_icon(widget).is_none() {
        return Some(ComponentRuleOutcome::Veto);
    }
    Some(ComponentRuleOutcome::Tag("icon"))
}

pub(crate) fn rule_confirm_dialog(
    widget: &SemanticWidgetView<'_>,
    _snapshot: &SemanticRead<'_>,
) -> Option<ComponentRuleOutcome> {
    (widget.kind == crate::WidgetKind::Dialog && vue_confirm_dialog(&widget.props))
        .then_some(ComponentRuleOutcome::Tag("confirm-dialog"))
}

pub(crate) fn rule_hosted_textarea(
    widget: &SemanticWidgetView<'_>,
    _snapshot: &SemanticRead<'_>,
) -> Option<ComponentRuleOutcome> {
    (widget.kind == crate::WidgetKind::Textarea
        && crate::widget_map::highlight_language(&widget.props).is_some())
    .then_some(ComponentRuleOutcome::Tag("hosted-textarea"))
}

pub(crate) fn rule_choice_field(
    widget: &SemanticWidgetView<'_>,
    _snapshot: &SemanticRead<'_>,
) -> Option<ComponentRuleOutcome> {
    if !widget.kind.is_choice_field() {
        return None;
    }
    let tag = if widget.kind == crate::WidgetKind::SearchDropdown
        || crate::widget_map::is_search_dropdown(&widget.props)
    {
        "search-dropdown"
    } else if widget.kind == crate::WidgetKind::Dropdown
        || crate::widget_map::is_dropdown_field(&widget.props)
    {
        "dropdown"
    } else {
        "select"
    };
    Some(ComponentRuleOutcome::Tag(tag))
}

/// Ordered kind + prop composition → Runtime tag rules. First match wins;
/// later rules never see a node an earlier rule tagged or vetoed. Keep the
/// order locked by `kind_rule_precedence_*` tests below.
pub(crate) const COMPONENT_RULES: &[ComponentRule] = &[
    rule_button_with_icon,
    rule_chip_variant,
    rule_standalone_icon,
    rule_confirm_dialog,
    rule_hosted_textarea,
    rule_choice_field,
];

pub(crate) fn resolve_widget_component_type(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
    context: &AppContext,
) -> Option<ComponentTypeId> {
    for rule in COMPONENT_RULES {
        match rule(widget, snapshot) {
            Some(ComponentRuleOutcome::Veto) => return None,
            Some(ComponentRuleOutcome::Tag(tag)) => {
                if let Some(id) = context.resolve_component_tag(tag) {
                    return Some(id.clone());
                }
            }
            None => {}
        }
    }
    let kind = widget.kind;
    let element_tag = widget.props.element_tag.as_str();
    // Distinct normalized candidates, probed once each: element_tag, then the
    // kind's downlevel tag, then the canonical kind string.
    let mut candidates: Vec<String> = Vec::with_capacity(3);
    let mut try_tag = |tag: &str| -> Option<ComponentTypeId> {
        if tag.is_empty() {
            return None;
        }
        let normalized = nana_ui_runtime::normalize_tag(tag);
        if candidates.contains(&normalized) {
            return None;
        }
        candidates.push(normalized.clone());
        context
            .resolve_component_tag_normalized(&normalized)
            .cloned()
    };
    let html_layout_collision =
        kind.is_layout() && matches!(element_tag.to_ascii_lowercase().as_str(), "search");
    if !html_layout_collision && let Some(id) = try_tag(element_tag) {
        return Some(id);
    }
    if element_tag.starts_with("nana-") {
        return None;
    }
    try_tag(kind.element_tag()).or_else(|| try_tag(kind.as_str()))
}

pub(crate) fn can_bind_from_semantic(widget: &SemanticWidgetView<'_>) -> bool {
    if widget.kind == crate::WidgetKind::Chip && widget.props.role.eq_ignore_ascii_case("tab") {
        return false;
    }
    let kind = effective_kind(widget);
    kind.is_choice_field()
        || matches!(
            kind,
            crate::WidgetKind::Column
                | crate::WidgetKind::Box
                | crate::WidgetKind::Row
                | crate::WidgetKind::Button
                | crate::WidgetKind::IconButton
                | crate::WidgetKind::Chip
                | crate::WidgetKind::Input
                | crate::WidgetKind::NumberInput
                | crate::WidgetKind::Textarea
                | crate::WidgetKind::Checkbox
                | crate::WidgetKind::Range
                | crate::WidgetKind::Spinner
                | crate::WidgetKind::InteractiveCard
                | crate::WidgetKind::Skeleton
                | crate::WidgetKind::Tooltip
                | crate::WidgetKind::ActionMenu
                | crate::WidgetKind::ActionMenuItem
                | crate::WidgetKind::Dialog
                | crate::WidgetKind::Popover
                | crate::WidgetKind::Switch
                | crate::WidgetKind::Card
                | crate::WidgetKind::Progress
                | crate::WidgetKind::StatusBadge
                | crate::WidgetKind::ValidationMessage
                | crate::WidgetKind::Toast
                | crate::WidgetKind::LevelMeter
                | crate::WidgetKind::ImageViewer
                | crate::WidgetKind::XYPad
                | crate::WidgetKind::QrCode
                | crate::WidgetKind::ListItem
                | crate::WidgetKind::EmptyState
                | crate::WidgetKind::LabeledValue
                | crate::WidgetKind::FormField
                | crate::WidgetKind::Drawer
                | crate::WidgetKind::SidebarFrame
                | crate::WidgetKind::SidebarRow
                | crate::WidgetKind::SettingsRow
                | crate::WidgetKind::SettingsCard
                | crate::WidgetKind::ContextMenu
                | crate::WidgetKind::CommandPalette
                | crate::WidgetKind::Segmented
                | crate::WidgetKind::Tabs
                | crate::WidgetKind::TreeView
                | crate::WidgetKind::CalendarHeatmap
                | crate::WidgetKind::NativeMarkdown
                | crate::WidgetKind::Icon
                | crate::WidgetKind::GraphCanvas
                | crate::WidgetKind::Workspace
                | crate::WidgetKind::Dock
                | crate::WidgetKind::SplitPane
                | crate::WidgetKind::AppShell
                | crate::WidgetKind::DesktopShell
                | crate::WidgetKind::AppTitleBar
                | crate::WidgetKind::PaneChrome
                | crate::WidgetKind::SidebarSection
                | crate::WidgetKind::SidebarFooter
                | crate::WidgetKind::SettingsPage
                | crate::WidgetKind::SettingsCollapsibleCard
                | crate::WidgetKind::Divider
                | crate::WidgetKind::Thumbnail
                | crate::WidgetKind::Avatar
                | crate::WidgetKind::List
                | crate::WidgetKind::ScrollView
                | crate::WidgetKind::Table
                | crate::WidgetKind::TableRow
                | crate::WidgetKind::TableCell
                | crate::WidgetKind::ReorderList
                | crate::WidgetKind::TimeSeriesChart
                | crate::WidgetKind::GpuTextureView
                | crate::WidgetKind::GpuView
        )
}

pub(crate) fn semantic_numeric_fields(widget: &SemanticWidgetView<'_>) -> (f32, f32) {
    match widget.kind {
        crate::WidgetKind::Progress => (widget.props.progress, widget.props.progress_max),
        crate::WidgetKind::LevelMeter => {
            let raw = crate::widget_map::level_meter_value(&widget.props);
            if !widget.props.element_tag.eq_ignore_ascii_case("meter") {
                return (raw, widget.props.max);
            }
            let min = widget.props.min;
            let max = crate::widget_map::attr_value(&widget.props, &["max"])
                .and_then(|value| value.parse().ok())
                .unwrap_or(1.0);
            let span = (max - min).abs().max(f32::EPSILON);
            (((raw - min) / span).clamp(0.0, 1.0), 1.0)
        }
        _ => (widget.props.number, widget.props.max),
    }
}

pub(crate) fn bind_attr_overrides(widget: &SemanticWidgetView<'_>) -> Vec<(String, String)> {
    let mut extras = Vec::new();
    let missing = |name: &str| {
        !widget
            .props
            .attrs
            .keys()
            .any(|key| key.eq_ignore_ascii_case(name))
    };
    match widget.kind {
        crate::WidgetKind::Switch if missing("control-position") => extras.push((
            "control-position".into(),
            match widget.props.control_position {
                nana_ui_core::SwitchControlPosition::Start => "start",
                nana_ui_core::SwitchControlPosition::End => "end",
            }
            .into(),
        )),
        crate::WidgetKind::Card if missing("card-kind") => extras.push((
            "card-kind".into(),
            match widget.props.card_kind {
                nana_ui_core::CardKind::Surface => "surface",
                nana_ui_core::CardKind::Outlined => "outlined",
                nana_ui_core::CardKind::Raised => "raised",
                nana_ui_core::CardKind::Flat => "flat",
                nana_ui_core::CardKind::Selected => "selected",
            }
            .into(),
        )),
        crate::WidgetKind::StatusBadge => {
            if missing("compact") && crate::widget_map::class_has_compact(&widget.props) {
                extras.push(("compact".into(), "true".into()));
            }
            if missing("tone") {
                extras.push((
                    "tone".into(),
                    status_tone_attr(crate::widget_map::status_tone_from_props(&widget.props))
                        .into(),
                ));
            }
        }
        crate::WidgetKind::Toast => {
            if missing("tone") {
                extras.push((
                    "tone".into(),
                    toast_tone_attr(crate::widget_map::toast_tone_from_props(&widget.props)).into(),
                ));
            }
            if missing("dismissible") && crate::widget_map::toast_dismissible(&widget.props) {
                extras.push(("dismissible".into(), "true".into()));
            }
        }
        crate::WidgetKind::Progress => {
            if missing("cancellable") && crate::widget_map::progress_cancellable(&widget.props) {
                extras.push(("cancellable".into(), "true".into()));
            }
        }
        crate::WidgetKind::Range if missing("unit") && !widget.props.unit.is_empty() => {
            extras.push(("unit".into(), widget.props.unit.clone()));
        }
        crate::WidgetKind::ActionMenuItem => {
            if missing("danger") && crate::widget_map::action_menu_item_danger(&widget.props) {
                extras.push(("danger".into(), "true".into()));
            }
        }
        crate::WidgetKind::LevelMeter if missing("tone") => extras.push((
            "tone".into(),
            status_tone_attr(crate::widget_map::status_tone_from_props(&widget.props)).into(),
        )),
        crate::WidgetKind::Segmented => {
            if missing("chrome") && crate::widget_map::is_radio_group(&widget.props) {
                extras.push(("chrome".into(), "radio".into()));
            }
            if missing("role") && !widget.props.role.is_empty() {
                extras.push(("role".into(), widget.props.role.clone()));
            }
            if missing("fill") && widget.props.fill {
                extras.push(("fill".into(), "true".into()));
            }
        }
        crate::WidgetKind::ImageViewer if missing("src") => {
            if let Some(src) = widget
                .props
                .native_props
                .get("src")
                .map(host_value_text)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                extras.push(("src".into(), src));
            }
        }
        crate::WidgetKind::ListItem
            if missing("auto-height") && missing("autoheight") && widget.props.auto_height =>
        {
            extras.push(("auto-height".into(), "true".into()));
        }
        crate::WidgetKind::EmptyState | crate::WidgetKind::LabeledValue
            if missing("compact") && crate::widget_map::class_has_compact(&widget.props) =>
        {
            extras.push(("compact".into(), "true".into()));
        }
        crate::WidgetKind::LabeledValue if missing("muted") && widget.props.muted => {
            extras.push(("muted".into(), "true".into()));
        }
        crate::WidgetKind::Drawer if missing("side") && !widget.props.side.is_empty() => {
            extras.push(("side".into(), widget.props.side.clone()));
        }
        crate::WidgetKind::SidebarRow => {
            if missing("state") {
                extras.push((
                    "state".into(),
                    match crate::widget_map::sidebar_row_state(&widget.props) {
                        nana_ui_runtime::SidebarRowState::Idle => "idle",
                        nana_ui_runtime::SidebarRowState::Active => "active",
                        nana_ui_runtime::SidebarRowState::AncestorActive => "ancestor",
                        nana_ui_runtime::SidebarRowState::Disabled => "disabled",
                    }
                    .into(),
                ));
            }
            if missing("tone") {
                extras.push((
                    "tone".into(),
                    match crate::widget_map::sidebar_row_tone(&widget.props) {
                        nana_ui_runtime::SidebarRowTone::Default => "default",
                        nana_ui_runtime::SidebarRowTone::Warning => "warning",
                        nana_ui_runtime::SidebarRowTone::Error => "error",
                    }
                    .into(),
                ));
            }
            if missing("depth") && missing("data-depth") && missing("indent") {
                let depth = crate::widget_map::sidebar_row_depth(&widget.props);
                if depth > 0 {
                    extras.push(("depth".into(), depth.to_string()));
                }
            }
        }
        crate::WidgetKind::SettingsRow => {
            let (stacked, divided, loose, first, last) =
                crate::widget_map::settings_row_flags(&widget.props);
            if missing("stacked") && stacked {
                extras.push(("stacked".into(), "true".into()));
            }
            if missing("divided") && divided {
                extras.push(("divided".into(), "true".into()));
            }
            if missing("loose") && loose {
                extras.push(("loose".into(), "true".into()));
            }
            if missing("first-in-group") && missing("firstInGroup") && first {
                extras.push(("first-in-group".into(), "true".into()));
            }
            if missing("last-in-group") && missing("lastInGroup") && last {
                extras.push(("last-in-group".into(), "true".into()));
            }
        }
        crate::WidgetKind::ContextMenu => {
            if missing("anchor-x") && missing("data-anchor-x") {
                extras.push(("anchor-x".into(), widget.props.anchor_x.to_string()));
            }
            if missing("anchor-y") && missing("data-anchor-y") {
                extras.push(("anchor-y".into(), widget.props.anchor_y.to_string()));
            }
            if missing("searchable") && context_menu_searchable(&widget.props) {
                extras.push(("searchable".into(), "true".into()));
            }
        }
        crate::WidgetKind::Tabs if missing("fill") && widget.props.fill => {
            extras.push(("fill".into(), "true".into()));
        }
        _ => {}
    }
    extras.extend(bind_native_json_attrs(widget));
    extras
}

pub(crate) fn stringify_host_attr(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::String(text) => text.clone(),
        nana_js_engine::HostValue::Number(number) if number.is_finite() => {
            if number.fract() == 0.0 {
                format!("{}", *number as i64)
            } else {
                number.to_string()
            }
        }
        nana_js_engine::HostValue::Bool(flag) => flag.to_string(),
        nana_js_engine::HostValue::Array(_) | nana_js_engine::HostValue::Object(_) => {
            value.to_json_string()
        }
        _ => value.to_json_string(),
    }
}

pub(crate) fn bind_native_json_attrs(widget: &SemanticWidgetView<'_>) -> Vec<(String, String)> {
    let mut extras = Vec::new();
    let missing = |name: &str, extras: &[(String, String)]| {
        !widget
            .props
            .attrs
            .keys()
            .any(|key| key.eq_ignore_ascii_case(name))
            && !extras.iter().any(|(key, _)| key.eq_ignore_ascii_case(name))
    };
    let push = |extras: &mut Vec<(String, String)>, key: &str, host_keys: &[&str]| {
        if !missing(key, extras) {
            return;
        }
        for host in host_keys {
            if let Some(value) = widget.props.native_props.get(*host) {
                extras.push((key.to_string(), stringify_host_attr(value)));
                return;
            }
        }
    };
    let push_json = |extras: &mut Vec<(String, String)>, key: &str, host_keys: &[&str]| {
        if !missing(key, extras) {
            return;
        }
        for host in host_keys {
            if let Some(value) = widget.props.native_props.get(*host)
                && matches!(
                    value,
                    nana_js_engine::HostValue::Array(_) | nana_js_engine::HostValue::Object(_)
                )
            {
                extras.push((key.to_string(), value.to_json_string()));
                return;
            }
        }
    };
    match effective_kind(widget) {
        crate::WidgetKind::CommandPalette => {
            push_json(&mut extras, "items", &["items", "options"]);
        }
        crate::WidgetKind::TreeView => {
            push_json(&mut extras, "tree", &["tree"]);
            push_json(&mut extras, "nodes", &["nodes"]);
            push_json(&mut extras, "options", &["options"]);
        }
        crate::WidgetKind::CalendarHeatmap => {
            push_json(&mut extras, "data", &["data", "value"]);
            push(&mut extras, "options", &["options"]);
        }
        crate::WidgetKind::NativeMarkdown => {
            if widget.props.value.trim().is_empty() {
                let source = markdown_source_from_props(&widget.props);
                if !source.trim().is_empty() && missing("source", &extras) {
                    extras.push(("source".into(), source));
                }
            }
            push(
                &mut extras,
                "mermaid-renderer",
                &["mermaid-renderer", "mermaidrenderer"],
            );
            push(
                &mut extras,
                "math-renderer",
                &["math-renderer", "mathrenderer"],
            );
        }
        crate::WidgetKind::GraphCanvas => {
            if widget.props.native_props.contains_key("model") {
                push(&mut extras, "model", &["model"]);
            } else {
                let nodes = widget.props.native_props.get("nodes");
                let edges = widget.props.native_props.get("edges");
                if (nodes.is_some() || edges.is_some()) && missing("model", &extras) {
                    let mut map = BTreeMap::new();
                    if let Some(nodes) = nodes {
                        map.insert("nodes".into(), nodes.clone());
                    }
                    if let Some(edges) = edges {
                        map.insert("edges".into(), edges.clone());
                    }
                    extras.push((
                        "model".into(),
                        nana_js_engine::HostValue::Object(map).to_json_string(),
                    ));
                }
            }
            push(&mut extras, "nodes", &["nodes"]);
            push(&mut extras, "edges", &["edges"]);
            push(&mut extras, "viewport", &["viewport"]);
            push(&mut extras, "selection", &["selection"]);
        }
        crate::WidgetKind::Dock => {
            push_json(&mut extras, "root", &["root"]);
            push_json(&mut extras, "layout", &["layout"]);
        }
        crate::WidgetKind::ScrollView => {
            push(&mut extras, "scrollbars", &["scrollbars", "scrollbar"]);
            push(&mut extras, "axes", &["axes", "axis", "direction"]);
        }
        crate::WidgetKind::NumberInput => {
            push(&mut extras, "precision", &["precision"]);
        }
        crate::WidgetKind::Divider => {
            push(&mut extras, "orientation", &["orientation"]);
            push(&mut extras, "thickness", &["thickness"]);
            push(&mut extras, "inset", &["inset"]);
        }
        crate::WidgetKind::Thumbnail => {
            push(&mut extras, "aspect", &["aspect"]);
        }
        crate::WidgetKind::Chip => {
            push(&mut extras, "dismissible", &["dismissible", "dismiss"]);
        }
        crate::WidgetKind::Avatar => {
            push(&mut extras, "size", &["size"]);
        }
        crate::WidgetKind::TableCell => {
            push(
                &mut extras,
                "header",
                &["header", "column-header", "columnheader"],
            );
        }
        crate::WidgetKind::ReorderList => {
            push(&mut extras, "tree-drop", &["tree-drop", "treedrop"]);
            push(&mut extras, "spacing", &["spacing", "gap"]);
        }
        crate::WidgetKind::TimeSeriesChart => {
            push_json(&mut extras, "values", &["values", "data", "series"]);
        }
        crate::WidgetKind::AppTitleBar => {
            push(&mut extras, "maximized", &["maximized"]);
            push(
                &mut extras,
                "window-controls",
                &["window-controls", "windowcontrols"],
            );
            push(
                &mut extras,
                "center-width",
                &["center-width", "centerwidth"],
            );
        }
        crate::WidgetKind::SidebarSection => {
            push(&mut extras, "collapsible", &["collapsible"]);
            push(&mut extras, "expanded", &["expanded", "data-expanded"]);
            push(&mut extras, "count", &["count"]);
        }
        crate::WidgetKind::SplitPane => {
            push(&mut extras, "axis", &["axis"]);
            push(&mut extras, "size", &["size"]);
            push(
                &mut extras,
                "default-size",
                &["default-size", "defaultsize"],
            );
            push(&mut extras, "min", &["min"]);
            push(&mut extras, "max", &["max"]);
        }
        crate::WidgetKind::SettingsPage => {
            push(
                &mut extras,
                "content-padding",
                &["content-padding", "contentPadding", "contentpadding"],
            );
            push(
                &mut extras,
                "content-gap",
                &["content-gap", "contentGap", "contentgap"],
            );
            push_json(&mut extras, "settings", &["settings", "model"]);
            push(&mut extras, "tab", &["tab", "value"]);
            push(
                &mut extras,
                "hide-header",
                &["hide-header", "hideheader", "hideHeader"],
            );
        }
        _ => {}
    }
    extras
}

pub(crate) fn status_tone_attr(tone: nana_ui_core::StatusTone) -> &'static str {
    match tone {
        nana_ui_core::StatusTone::Neutral => "neutral",
        nana_ui_core::StatusTone::Info => "info",
        nana_ui_core::StatusTone::Success => "success",
        nana_ui_core::StatusTone::Warning => "warning",
        nana_ui_core::StatusTone::Danger => "danger",
    }
}

pub(crate) fn toast_tone_attr(tone: nana_ui_core::ToastTone) -> &'static str {
    match tone {
        nana_ui_core::ToastTone::Info => "info",
        nana_ui_core::ToastTone::Success => "success",
        nana_ui_core::ToastTone::Warning => "warning",
        nana_ui_core::ToastTone::Danger => "danger",
    }
}

pub(crate) fn is_shell_composer_kind(kind: crate::WidgetKind) -> bool {
    matches!(
        kind,
        crate::WidgetKind::Workspace
            | crate::WidgetKind::Dock
            | crate::WidgetKind::SplitPane
            | crate::WidgetKind::AppShell
            | crate::WidgetKind::DesktopShell
    )
}

pub(crate) fn shell_kind_from_ident(raw: &str) -> Option<crate::WidgetKind> {
    let parsed = crate::WidgetKind::parse(raw).or_else(|| {
        let lower = raw.trim().to_ascii_lowercase();
        let stripped = lower.strip_prefix("nana.").unwrap_or(&lower);
        crate::WidgetKind::parse(stripped)
    })?;
    is_shell_composer_kind(parsed).then_some(parsed)
}

pub(crate) fn effective_kind(widget: &SemanticWidgetView<'_>) -> crate::WidgetKind {
    if !matches!(
        widget.kind,
        crate::WidgetKind::Column | crate::WidgetKind::Box | crate::WidgetKind::Row
    ) {
        return widget.kind;
    }
    if widget.props.element_tag.is_empty() {
        return widget.kind;
    }
    shell_kind_from_ident(&widget.props.element_tag).unwrap_or(widget.kind)
}

pub(crate) fn bind_semantic_slots(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> Vec<(String, StableNodeId)> {
    let mut slots = Vec::new();
    let push = |slots: &mut Vec<(String, StableNodeId)>, name: &str, id: Option<StableNodeId>| {
        let Some(id) = id else {
            return;
        };
        if slots
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
        {
            return;
        }
        slots.push((name.to_string(), id));
    };
    let data_slot = |name: &str| {
        widget.children.iter().find_map(|child| {
            let child = snapshot.get(*child)?;
            (child.props.attrs.get("data-slot").map(String::as_str) == Some(name))
                .then(|| StableNodeId::new(child.id))
                .flatten()
        })
    };
    let assigned = |slots: &[(String, StableNodeId)], id: StableNodeId| {
        slots.iter().any(|(_, existing)| *existing == id)
    };

    match effective_kind(widget) {
        crate::WidgetKind::ListItem | crate::WidgetKind::SidebarRow => {
            push(
                &mut slots,
                "leading",
                data_slot("leading").or_else(|| data_slot("list-item-leading")),
            );
            let tools = data_slot("tools");
            if widget.kind == crate::WidgetKind::SidebarRow {
                push(&mut slots, "tools", tools);
            }
            let trailing = data_slot("trailing")
                .or_else(|| data_slot("list-item-trailing"))
                .filter(|id| Some(*id) != tools);
            push(&mut slots, "trailing", trailing);
            let content = data_slot("content")
                .or_else(|| data_slot("list-item-content"))
                .or_else(|| {
                    widget.children.iter().find_map(|child| {
                        let id = StableNodeId::new(*child)?;
                        (!assigned(&slots, id)).then_some(id)
                    })
                });
            push(&mut slots, "content", content);
        }
        crate::WidgetKind::EmptyState | crate::WidgetKind::LabeledValue => {
            push(&mut slots, "action", data_slot("action"));
            push(
                &mut slots,
                "action",
                crate::widget_map::first_button_child_id(snapshot, widget)
                    .and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::FormField => {
            push(&mut slots, "control", data_slot("control"));
            push(
                &mut slots,
                "control",
                crate::widget_map::form_field_control_child_id(snapshot, widget)
                    .and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::Drawer | crate::WidgetKind::Dialog => {
            push(&mut slots, "body", data_slot("body"));
            push(
                &mut slots,
                "body",
                widget.children.first().copied().and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::SidebarFrame => {
            let (top, body, footer) = crate::widget_map::sidebar_frame_slots(snapshot, widget);
            push(&mut slots, "top", top.and_then(StableNodeId::new));
            push(&mut slots, "body", body.and_then(StableNodeId::new));
            push(&mut slots, "footer", footer.and_then(StableNodeId::new));
        }
        crate::WidgetKind::SettingsRow => {
            let row = crate::widget_map::settings_row_slots(snapshot, widget);
            push(&mut slots, "copy", row.copy.and_then(StableNodeId::new));
            push(&mut slots, "label", row.label.and_then(StableNodeId::new));
            push(&mut slots, "hint", row.hint.and_then(StableNodeId::new));
            push(&mut slots, "control", data_slot("control"));
            push(
                &mut slots,
                "control",
                crate::widget_map::settings_row_control_child_id(snapshot, widget)
                    .and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::SettingsCard => {
            push(&mut slots, "title", data_slot("title"));
            let title = widget.children.iter().find_map(|child| {
                let child = snapshot.get(*child)?;
                child
                    .props
                    .class_names
                    .iter()
                    .any(|class| {
                        class.contains("nana-settings-card__title")
                            || class.contains("settings-card__title")
                    })
                    .then(|| StableNodeId::new(child.id))
                    .flatten()
            });
            push(&mut slots, "title", title);
        }
        crate::WidgetKind::AppShell => {
            let (title_bar, body, overlay) = app_shell_slots(widget, snapshot);
            push(&mut slots, "title-bar", title_bar);
            push(&mut slots, "body", body);
            push(&mut slots, "overlay", overlay);
        }
        crate::WidgetKind::DesktopShell => {
            push(
                &mut slots,
                "title-bar",
                data_slot("title-bar").or_else(|| data_slot("title_bar")),
            );
            push(
                &mut slots,
                "primary",
                data_slot("primary").or_else(|| data_slot("main")),
            );
            push(
                &mut slots,
                "navigation",
                data_slot("navigation").or_else(|| data_slot("nav")),
            );
            push(
                &mut slots,
                "navigation-footer",
                data_slot("navigation-footer").or_else(|| data_slot("navigation_footer")),
            );
            push(&mut slots, "inspector", data_slot("inspector"));
            push(&mut slots, "bottom", data_slot("bottom"));
            push(&mut slots, "overlay", data_slot("overlay"));
        }
        crate::WidgetKind::AppTitleBar => {
            push(&mut slots, "leading", data_slot("leading"));
            push(&mut slots, "center", data_slot("center"));
            push(&mut slots, "trailing", data_slot("trailing"));
            push(&mut slots, "controls", data_slot("controls"));
        }
        crate::WidgetKind::PaneChrome => {
            push(&mut slots, "tabs", data_slot("tabs"));
            push(&mut slots, "body", data_slot("body"));
            push(&mut slots, "header", data_slot("header"));
        }
        crate::WidgetKind::SidebarSection => {
            push(&mut slots, "tools", data_slot("tools"));
            push(&mut slots, "header", data_slot("header"));
            push(&mut slots, "body", data_slot("body"));
        }
        crate::WidgetKind::SettingsCollapsibleCard => {
            let summary = data_slot("summary")
                .or_else(|| data_slot("header"))
                .or_else(|| {
                    widget.children.iter().find_map(|child| {
                        let child = snapshot.get(*child)?;
                        widget_tag(&child)
                            .eq_ignore_ascii_case("summary")
                            .then(|| StableNodeId::new(child.id))
                            .flatten()
                    })
                });
            push(&mut slots, "summary", summary);
            push(
                &mut slots,
                "details",
                data_slot("details")
                    .or_else(|| data_slot("body"))
                    .or_else(|| {
                        widget.children.iter().find_map(|child| {
                            let child = snapshot.get(*child)?;
                            let id = StableNodeId::new(child.id)?;
                            (Some(id) != summary
                                && !widget_tag(&child).eq_ignore_ascii_case("summary"))
                            .then_some(id)
                        })
                    }),
            );
            push(&mut slots, "accessory", data_slot("accessory"));
        }
        crate::WidgetKind::SplitPane => {
            let children = element_child_widgets(widget, snapshot);
            let handle = data_slot("handle")
                .or_else(|| data_slot("split-handle"))
                .or_else(|| {
                    children
                        .iter()
                        .find(|child| is_split_handle_child(child))
                        .and_then(|child| StableNodeId::new(child.id))
                })
                .or_else(|| {
                    (children.len() >= 3)
                        .then(|| StableNodeId::new(children[2].id))
                        .flatten()
                });
            let panes = children
                .iter()
                .filter(|child| Some(child.id) != handle.map(StableNodeId::get))
                .filter_map(|child| StableNodeId::new(child.id))
                .collect::<Vec<_>>();
            push(
                &mut slots,
                "first",
                data_slot("first").or(panes.first().copied()),
            );
            push(
                &mut slots,
                "second",
                data_slot("second").or(panes.get(1).copied()),
            );
            push(&mut slots, "handle", handle);
        }
        crate::WidgetKind::Workspace => {
            for slot in workspace_slots_from_widget(widget, snapshot) {
                if let Some(content) = slot.content {
                    push(&mut slots, slot.id.as_str(), Some(content));
                }
            }
        }
        crate::WidgetKind::Dock => {
            for (index, child) in element_child_widgets(widget, snapshot)
                .into_iter()
                .enumerate()
            {
                let Some(id) = StableNodeId::new(child.id) else {
                    continue;
                };
                push(&mut slots, &dock_item_id(&child, index), Some(id));
            }
        }
        crate::WidgetKind::SettingsPage => {
            push(&mut slots, "content", data_slot("content"));
            push(&mut slots, "body", data_slot("body"));
            push(
                &mut slots,
                "content",
                settings_page_content_child(widget, snapshot),
            );
        }
        _ => {
            for &child in widget.children.iter() {
                let Some(child) = snapshot.get(child) else {
                    continue;
                };
                let Some(id) = StableNodeId::new(child.id) else {
                    continue;
                };
                let Some(raw) = child.props.attrs.get("data-slot") else {
                    continue;
                };
                let raw_lower = raw.trim().to_ascii_lowercase();
                let name = match raw_lower.as_str() {
                    "leading" | "list-item-leading" => "leading",
                    "content" | "list-item-content" => "content",
                    "trailing" | "list-item-trailing" => "trailing",
                    "tools" => "tools",
                    "action" => "action",
                    "control" => "control",
                    "body" | "sidebar-body" => "body",
                    "top" | "sidebar-top" => "top",
                    "footer" | "sidebar-footer" => "footer",
                    "copy" => "copy",
                    "label" => "label",
                    "hint" => "hint",
                    "title" => "title",
                    "title-bar" | "titlebar" | "title_bar" | "app-title-bar" => "title-bar",
                    "overlay" => "overlay",
                    "primary" | "main" => "primary",
                    "navigation" | "nav" => "navigation",
                    "navigation-footer" | "navigation_footer" => "navigation-footer",
                    "inspector" => "inspector",
                    "bottom" => "bottom",
                    "tabs" => "tabs",
                    "header" => "header",
                    "summary" => "summary",
                    "details" => "details",
                    "accessory" => "accessory",
                    "center" => "center",
                    "controls" => "controls",
                    "first" => "first",
                    "second" => "second",
                    "handle" | "split-handle" => "handle",
                    other => other,
                };
                push(&mut slots, name, Some(id));
            }
        }
    }
    slots
}

pub(crate) fn bind_semantic_copy(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> (Option<String>, Option<String>) {
    match widget.kind {
        crate::WidgetKind::LabeledValue => {
            let caption = crate::widget_map::labeled_value_caption(snapshot, widget);
            let label = (widget.props.label.is_empty() && !caption.is_empty()).then_some(caption);
            (label, None)
        }
        crate::WidgetKind::SettingsRow => {
            let slots = crate::widget_map::settings_row_slots(snapshot, widget);
            let slot_text = |slot: Option<crate::WidgetId>, fallback: &str| {
                if !fallback.is_empty() {
                    return None;
                }
                slot.map(|id| crate::widget_map::settings_row_plain_text(snapshot, id))
                    .filter(|text| !text.is_empty())
            };
            (
                slot_text(slots.label, widget.props.display_label()),
                slot_text(slots.hint, widget.props.hint.as_str()),
            )
        }
        crate::WidgetKind::Table if widget.props.label.is_empty() => {
            let caption = widget.children.iter().find_map(|child| {
                let child = snapshot.get(*child)?;
                widget_tag(&child)
                    .eq_ignore_ascii_case("caption")
                    .then(|| child.props.display_label().to_string())
                    .filter(|text| !text.is_empty())
            });
            (caption, None)
        }
        _ => (None, None),
    }
}

pub(crate) fn context_menu_searchable(props: &crate::WidgetProps) -> bool {
    props.options.len() >= 6
        || props
            .class_names
            .iter()
            .any(|class| class.contains("search"))
}

pub(crate) fn try_bind_registered_component(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
    id: StableNodeId,
    context: &AppContext,
    mutations: &mut MutationQueue,
    pending: &mut PendingAssembly,
) -> Option<bool> {
    let type_id = resolve_widget_component_type(widget, snapshot, context)?;
    if !can_bind_from_semantic(widget) {
        if context.world().component_type(id) != Some(&type_id) {
            mutations.set_component_type(id, Some(type_id));
        }
        return None;
    }
    let layout = Arc::new(widget.props.layout.clone());
    let mut extra_attrs = bind_attr_overrides(widget);
    let missing_query = !widget
        .props
        .attrs
        .keys()
        .any(|key| key.eq_ignore_ascii_case("query") || key.eq_ignore_ascii_case("data-query"))
        && !extra_attrs
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("query"));
    if missing_query
        && ((widget.kind == crate::WidgetKind::ContextMenu
            && context_menu_searchable(&widget.props))
            || widget.kind == crate::WidgetKind::SearchDropdown
            || (widget.kind == crate::WidgetKind::Select
                && crate::widget_map::is_search_dropdown(&widget.props)))
        && let Some(state) = context.world().text_input(id)
    {
        extra_attrs.push(("query".into(), state.value.clone()));
    }
    let attr_pairs: Vec<(&str, &str)> = widget
        .props
        .attrs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .chain(
            extra_attrs
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .collect();
    let slot_pairs = bind_semantic_slots(widget, snapshot);
    let slot_refs: Vec<(&str, StableNodeId)> = slot_pairs
        .iter()
        .map(|(name, id)| (name.as_str(), *id))
        .collect();
    let (owned_label, owned_hint) = bind_semantic_copy(widget, snapshot);
    let (number, max) = semantic_numeric_fields(widget);
    let tree_child_options = tree_child_bind_options(widget, snapshot);
    let option_list: Vec<SemanticOption<'_>> = if !widget.props.options.is_empty() {
        widget
            .props
            .options
            .iter()
            .map(|option| SemanticOption {
                value: option.value.as_str(),
                label: option.label.as_str(),
                disabled: option.disabled,
            })
            .collect()
    } else {
        tree_child_options
            .iter()
            .map(|(value, label, disabled)| SemanticOption {
                value: value.as_str(),
                label: label.as_str(),
                disabled: *disabled,
            })
            .collect()
    };
    let is_icon_button = type_id.as_str() == "nana.icon-button";
    let button_kind = if widget.kind == crate::WidgetKind::Chip {
        if widget.props.active || widget.props.toggled {
            nana_ui_core::ButtonKind::Selected
        } else {
            nana_ui_core::ButtonKind::Subtle
        }
    } else {
        widget.props.button_kind
    };
    let label = if is_icon_button && !widget.props.hint.is_empty() {
        widget.props.hint.as_str()
    } else if let Some(label) = owned_label.as_deref() {
        label
    } else {
        widget.props.label.as_str()
    };
    let hint = owned_hint.as_deref().unwrap_or(widget.props.hint.as_str());
    let spec = SemanticSpec {
        label,
        value: widget.props.value.as_str(),
        hint,
        placeholder: widget.props.placeholder.as_str(),
        disabled: widget.props.disabled
            || (widget.kind == crate::WidgetKind::Checkbox && widget.props.loading),
        loading: widget.props.loading,
        invalid: widget.props.invalid
            || (widget.kind == crate::WidgetKind::Dialog && vue_confirm_danger(&widget.props)),
        active: widget.props.active,
        toggled: widget.props.toggled,
        read_only: widget.props.read_only,
        secure: widget.props.secure,
        button_kind,
        size: widget.props.size,
        icon: widget_icon(widget, snapshot),
        min: widget.props.min,
        max,
        step: widget.props.step,
        number,
        options: &option_list,
        attrs: &attr_pairs,
        slots: &slot_refs,
        ..SemanticSpec::from_parts(&type_id, &layout)
    };
    let existing_input = matches!(
        widget.kind,
        crate::WidgetKind::Input | crate::WidgetKind::NumberInput | crate::WidgetKind::Textarea
    )
    .then(|| context.world().text_input(id).cloned())
    .flatten();
    let binding = context
        .prepare_semantic_binding(id, &spec, mutations)
        .ok()?;
    let kind = binding.kind();
    pending.bindings.push(binding);
    match kind {
        ComponentBindKind::Projected => {
            if let Some(mut state) = existing_input {
                if state.value != widget.props.value {
                    state.replace_value(&widget.props.value);
                }
                mutations.set_text_input(id, Some(state));
            }
            if widget.kind == crate::WidgetKind::Popover {
                retain_projected_children(widget, id, context.world(), mutations);
            }
            if widget.kind == crate::WidgetKind::Tooltip
                && context.world().standard_visual(id).is_some()
            {
                mutations.set_standard_visual(id, None);
            }
            if matches!(
                widget.kind,
                crate::WidgetKind::Segmented | crate::WidgetKind::Tabs
            ) {
                let chrome = selection_chrome(widget.kind, &widget.props);
                for (child, option) in widget.children.iter().zip(widget.props.options.iter()) {
                    let Some(child_id) = StableNodeId::new(*child) else {
                        continue;
                    };
                    project_segmented_option(
                        child_id,
                        option,
                        option.value == widget.props.value,
                        widget.props.size,
                        chrome,
                        widget.kind == crate::WidgetKind::Tabs && widget.props.fill,
                        context.world(),
                        mutations,
                    );
                }
            }
            #[cfg(feature = "graph-canvas")]
            if widget.kind == crate::WidgetKind::GraphCanvas {
                pending
                    .graph_canvases
                    .push((id, RuntimeGraphCanvas::from_semantic(&spec)));
            }
            enqueue_bound_assembly(widget, snapshot, &spec, id, context, mutations, pending);
            Some(true)
        }
        ComponentBindKind::Layout => None,
    }
}

pub(crate) fn selection_chrome(
    kind: crate::WidgetKind,
    props: &crate::WidgetProps,
) -> SelectionChrome {
    if kind == crate::WidgetKind::Tabs {
        SelectionChrome::Tabs
    } else if crate::widget_map::is_radio_group(props) {
        SelectionChrome::Radio
    } else {
        SelectionChrome::Segmented
    }
}

pub(crate) fn option_from_widget(child: &SemanticWidgetView<'_>) -> (String, String, bool) {
    let id = if !child.props.value.is_empty() {
        child.props.value.clone()
    } else if !child.props.element_id.is_empty() {
        child.props.element_id.clone()
    } else {
        child.props.display_label().to_string()
    };
    (
        id,
        child.props.display_label().to_string(),
        child.props.disabled,
    )
}

pub(crate) fn is_option_child(child: &SemanticWidgetView<'_>) -> bool {
    let tag = widget_tag(child);
    child.kind == crate::WidgetKind::Radio
        || tag.eq_ignore_ascii_case("option")
        || child.props.role.eq_ignore_ascii_case("option")
        || child.props.role.eq_ignore_ascii_case("radio")
}

pub(crate) fn collect_choice_options(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for child in element_child_widgets(widget, snapshot) {
        if is_option_child(&child) {
            out.push(option_from_widget(&child));
            continue;
        }
        if widget_tag(&child).eq_ignore_ascii_case("optgroup") {
            for nested in element_child_widgets(&child, snapshot) {
                if is_option_child(&nested) {
                    out.push(option_from_widget(&nested));
                }
            }
        }
    }
    out
}

pub(crate) fn tree_child_bind_options(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
) -> Vec<(String, String, bool)> {
    if !widget.props.options.is_empty() {
        return Vec::new();
    }
    match effective_kind(widget) {
        crate::WidgetKind::TreeView if host_tree_nodes(&widget.props).is_none() => {
            element_child_widgets(widget, snapshot)
                .into_iter()
                .map(|widget| option_from_widget(&widget))
                .collect()
        }
        crate::WidgetKind::Select
        | crate::WidgetKind::Dropdown
        | crate::WidgetKind::SearchDropdown
        | crate::WidgetKind::Segmented
        | crate::WidgetKind::Tabs => collect_choice_options(widget, snapshot),
        _ => Vec::new(),
    }
}

pub(crate) fn enqueue_bound_assembly(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
    spec: &SemanticSpec<'_>,
    id: StableNodeId,
    context: &AppContext,
    mutations: &mut MutationQueue,
    pending: &mut PendingAssembly,
) {
    let world = context.world();
    match effective_kind(widget) {
        crate::WidgetKind::AppShell => {
            let title = widget.props.display_label();
            let (title_bar, body, overlay) = app_shell_slots(widget, snapshot);
            if let Some(title_bar) = title_bar {
                let bar_title = if title.is_empty() {
                    snapshot
                        .get(title_bar.get())
                        .map(|child| child.props.display_label().to_string())
                        .unwrap_or_default()
                } else {
                    title.to_string()
                };
                let bar = RuntimeAppTitleBar::new(bar_title);
                if !snapshot
                    .get(title_bar.get())
                    .is_some_and(|widget| is_title_bar_child(&widget))
                {
                    bar.project(title_bar, world, mutations);
                    pending.title_bars.push((title_bar, bar));
                }
            }
            let mut component = RuntimeAppShell::new();
            if let Some(title_bar) = title_bar {
                component = component.title_bar(title_bar);
            }
            if let Some(body) = body {
                component = component.body(body);
            }
            if let Some(overlay) = overlay {
                component = component.overlay(overlay);
            }
            pending.app_shells.push((id, component));
        }
        crate::WidgetKind::SplitPane => {
            pending
                .split_panes
                .push((id, split_pane_from_widget(widget, snapshot, context, id)));
        }
        crate::WidgetKind::Workspace => {
            pending
                .workspaces
                .push((id, workspace_from_widget(widget, snapshot, context, id)));
        }
        crate::WidgetKind::Dock => {
            pending
                .docks
                .push((id, dock_from_widget_bound(widget, snapshot, context, id)));
        }
        crate::WidgetKind::SettingsPage => {
            let model = settings_model_from_props(&widget.props).unwrap_or_else(|| {
                let label = widget.props.display_label();
                let label = if label.is_empty() { "Settings" } else { label };
                nana_ui_core::SettingsModel::new(
                    "settings",
                    [nana_ui_core::SettingsTab::new("settings", label)],
                )
                .expect("fallback settings model has one tab")
            });
            let mut state = nana_ui_core::SettingsState::new(&model);
            if let Some(tab) = settings_active_tab(&widget.props) {
                state.select(&model, &tab);
            }
            let mut component =
                <RuntimeSettingsPage as nana_ui_runtime::RegisterableComponent>::from_semantic(
                    spec,
                );
            component.model = model;
            component.state = state;
            if let Some(content) = settings_page_content_child(widget, snapshot) {
                component = component.content(content);
            }
            if let Ok(existing) = context.read(
                Entity::<RuntimeSettingsPage>::from_stable_id(id),
                Clone::clone,
            ) {
                component.assembly = existing.assembly;
            }
            pending.settings_pages.push((id, component));
        }
        _ => {}
    }
}

pub(crate) fn project_migrating_component(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
    id: StableNodeId,
    context: &AppContext,
    mutations: &mut MutationQueue,
    pending: &mut PendingAssembly,
) -> bool {
    let world = context.world();
    if crate::widget_map::is_settings_row_projected_slot(snapshot, widget) {
        return true;
    }
    if is_title_bar_child(widget) {
        let title = widget.props.display_label();
        let bar = RuntimeAppTitleBar::new(title);
        bar.project(id, world, mutations);
        pending.title_bars.push((id, bar));
        return true;
    }
    if !is_sidebar_frame_body(widget)
        && try_bind_registered_component(widget, snapshot, id, context, mutations, pending)
            == Some(true)
    {
        return true;
    }
    match effective_kind(widget) {
        crate::WidgetKind::Column | crate::WidgetKind::Box if is_sidebar_frame_body(widget) => {
            RuntimeSidebarFrame::scroll_body(widget.props.layout.clone())
                .project(id, world, mutations);
            true
        }
        _ => {
            if project_aligned_segmented_option(widget, snapshot, id, world, mutations) {
                return true;
            }
            if widget.kind == crate::WidgetKind::Radio {
                let label = widget.props.display_label();
                RuntimeSegmentedOption::new(Arc::<str>::from(label))
                    .disabled(widget.props.disabled)
                    .with_selected(widget.props.toggled || widget.props.active)
                    .surface(widget.props.size, SelectionChrome::Radio, false)
                    .project(id, world, mutations);
                return true;
            }
            if matches!(
                world.standard_visual(id),
                Some(
                    nana_ui_runtime::StandardVisual::Icon { .. }
                        | nana_ui_runtime::StandardVisual::Button { .. }
                        | nana_ui_runtime::StandardVisual::TextInput { .. }
                        | nana_ui_runtime::StandardVisual::Switch { .. }
                        | nana_ui_runtime::StandardVisual::Range { .. }
                        | nana_ui_runtime::StandardVisual::Card { .. }
                        | nana_ui_runtime::StandardVisual::StatusBadge { .. }
                        | nana_ui_runtime::StandardVisual::ValidationMessage { .. }
                        | nana_ui_runtime::StandardVisual::EmptyState { .. }
                        | nana_ui_runtime::StandardVisual::LabeledValue { .. }
                        | nana_ui_runtime::StandardVisual::SelectionOption { .. }
                        | nana_ui_runtime::StandardVisual::Progress { .. }
                        | nana_ui_runtime::StandardVisual::Spinner { .. }
                        | nana_ui_runtime::StandardVisual::LevelMeter { .. }
                        | nana_ui_runtime::StandardVisual::FormField { .. }
                        | nana_ui_runtime::StandardVisual::Toast { .. }
                        | nana_ui_runtime::StandardVisual::XYPad { .. }
                        | nana_ui_runtime::StandardVisual::QrCode { .. }
                        | nana_ui_runtime::StandardVisual::Select { .. }
                        | nana_ui_runtime::StandardVisual::MenuSurface { .. }
                        | nana_ui_runtime::StandardVisual::ActionMenuItem { .. }
                        | nana_ui_runtime::StandardVisual::TreeView { .. }
                        | nana_ui_runtime::StandardVisual::CommandPalette { .. }
                        | nana_ui_runtime::StandardVisual::ModalFrame { .. }
                        | nana_ui_runtime::StandardVisual::KeyCaptureLayer { .. }
                        | nana_ui_runtime::StandardVisual::KeymapLayer
                )
            ) || world
                .standard_visual(id)
                .is_some_and(|visual| visual.required_feature().is_some())
            {
                mutations.set_standard_visual(id, None);
            }
            false
        }
    }
}

pub(crate) fn vue_confirm_dialog(props: &crate::WidgetProps) -> bool {
    if props.role.eq_ignore_ascii_case("alertdialog") {
        return true;
    }
    if matches!(props.button_kind, nana_ui_core::ButtonKind::Danger) {
        return true;
    }
    if props.attrs.get("data-variant").is_some_and(|value| {
        value.eq_ignore_ascii_case("confirm") || value.eq_ignore_ascii_case("alertdialog")
    }) {
        return true;
    }
    props.class_names.iter().any(|class| {
        matches!(
            class.as_str(),
            "nana-confirm" | "nana-confirm-dialog" | "nana-alertdialog"
        )
    })
}

pub(crate) fn vue_confirm_danger(props: &crate::WidgetProps) -> bool {
    matches!(props.button_kind, nana_ui_core::ButtonKind::Danger)
}

pub(crate) fn project_segmented_option(
    id: StableNodeId,
    option: &crate::SelectOptionProp,
    selected: bool,
    size: nana_ui_core::ControlSize,
    chrome: SelectionChrome,
    fill: bool,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    RuntimeSegmentedOption::new(Arc::<str>::from(option.label.as_str()))
        .disabled(option.disabled)
        .with_selected(selected)
        .surface(size, chrome, fill)
        .project(id, world, mutations);
}

pub(crate) fn project_aligned_segmented_option(
    widget: &SemanticWidgetView<'_>,
    snapshot: &SemanticRead<'_>,
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) -> bool {
    let Some(parent) = widget.parent.and_then(|parent| snapshot.get(parent)) else {
        return false;
    };
    if !matches!(
        parent.kind,
        crate::WidgetKind::Segmented | crate::WidgetKind::Tabs
    ) {
        return false;
    }
    let Some(index) = parent.children.iter().position(|child| *child == widget.id) else {
        return false;
    };
    let Some(option) = parent.props.options.get(index) else {
        return false;
    };
    let chrome = selection_chrome(parent.kind, &parent.props);
    project_segmented_option(
        id,
        option,
        option.value == parent.props.value,
        parent.props.size,
        chrome,
        parent.kind == crate::WidgetKind::Tabs && parent.props.fill,
        world,
        mutations,
    );
    true
}

pub(crate) fn accessibility_role(
    kind: crate::WidgetKind,
    explicit_role: &str,
    element_tag: &str,
    accessible_name: Option<&str>,
    is_top_level: bool,
) -> AccessibilityRole {
    match explicit_role.trim().to_ascii_lowercase().as_str() {
        "document" => return AccessibilityRole::Document,
        "text" => return AccessibilityRole::Text,
        "button" => return AccessibilityRole::Button,
        "textbox" | "searchbox" => return AccessibilityRole::TextInput,
        "checkbox" => return AccessibilityRole::Checkbox,
        "radio" => return AccessibilityRole::Radio,
        "radiogroup" => return AccessibilityRole::RadioGroup,
        "switch" => return AccessibilityRole::Switch,
        "slider" => return AccessibilityRole::Slider,
        "combobox" => return AccessibilityRole::ComboBox,
        "progressbar" => return AccessibilityRole::ProgressIndicator,
        "list" | "listbox" => return AccessibilityRole::List,
        "listitem" | "option" => return AccessibilityRole::ListItem,
        "tablist" => return AccessibilityRole::TabList,
        "tab" => return AccessibilityRole::Tab,
        "dialog" | "alertdialog" => return AccessibilityRole::Dialog,
        "menu" => return AccessibilityRole::Menu,
        "menuitem" => return AccessibilityRole::MenuItem,
        "tooltip" => return AccessibilityRole::Tooltip,
        "img" => return AccessibilityRole::Image,
        "main" => return AccessibilityRole::Main,
        "navigation" => return AccessibilityRole::Navigation,
        "banner" => return AccessibilityRole::Banner,
        "contentinfo" => return AccessibilityRole::ContentInfo,
        "complementary" => return AccessibilityRole::Complementary,
        "region" => return AccessibilityRole::Region,
        "search" => return AccessibilityRole::Search,
        "form" => return AccessibilityRole::Form,
        _ => {}
    }
    // HTML landmark tags carry their landmark role without becoming widgets:
    // `<search>` stays a Column, it never resolves to SearchDropdown. Hints
    // that repurpose the tag into a concrete control (`<nav role="tablist">`,
    // `<footer class="nana-search">`) keep the control role; layout kinds and
    // kinds without their own role still yield to the landmark.
    let kind_role = match kind {
        crate::WidgetKind::Text => AccessibilityRole::Text,
        crate::WidgetKind::Button | crate::WidgetKind::IconButton | crate::WidgetKind::Chip => {
            AccessibilityRole::Button
        }
        crate::WidgetKind::Input | crate::WidgetKind::NumberInput | crate::WidgetKind::Textarea => {
            AccessibilityRole::TextInput
        }
        crate::WidgetKind::Checkbox => AccessibilityRole::Checkbox,
        crate::WidgetKind::Radio => AccessibilityRole::Radio,
        crate::WidgetKind::Switch => AccessibilityRole::Switch,
        crate::WidgetKind::Range => AccessibilityRole::Slider,
        crate::WidgetKind::Select
        | crate::WidgetKind::Dropdown
        | crate::WidgetKind::SearchDropdown => AccessibilityRole::ComboBox,
        crate::WidgetKind::Progress | crate::WidgetKind::LevelMeter => {
            AccessibilityRole::ProgressIndicator
        }
        crate::WidgetKind::InteractiveCard => AccessibilityRole::Button,
        crate::WidgetKind::ListItem
        | crate::WidgetKind::SidebarRow
        | crate::WidgetKind::TableRow => AccessibilityRole::ListItem,
        crate::WidgetKind::List | crate::WidgetKind::ReorderList | crate::WidgetKind::Table => {
            AccessibilityRole::List
        }
        crate::WidgetKind::Tabs | crate::WidgetKind::Segmented => AccessibilityRole::TabList,
        crate::WidgetKind::StatusBadge | crate::WidgetKind::ValidationMessage => {
            AccessibilityRole::Text
        }
        crate::WidgetKind::Dialog => AccessibilityRole::Dialog,
        crate::WidgetKind::ContextMenu | crate::WidgetKind::ActionMenu => AccessibilityRole::Menu,
        crate::WidgetKind::ActionMenuItem => AccessibilityRole::MenuItem,
        crate::WidgetKind::Tooltip => AccessibilityRole::Tooltip,
        crate::WidgetKind::XYPad => AccessibilityRole::Slider,
        crate::WidgetKind::QrCode => AccessibilityRole::Image,
        crate::WidgetKind::Icon
        | crate::WidgetKind::GpuTextureView
        | crate::WidgetKind::GpuView
        | crate::WidgetKind::Video => AccessibilityRole::Image,
        crate::WidgetKind::CommandPalette | crate::WidgetKind::ImageViewer => {
            AccessibilityRole::Dialog
        }
        crate::WidgetKind::TreeView => AccessibilityRole::List,
        crate::WidgetKind::CalendarHeatmap => AccessibilityRole::Image,
        crate::WidgetKind::NativeMarkdown => AccessibilityRole::Document,
        crate::WidgetKind::GraphCanvas => AccessibilityRole::Generic,
        _ => AccessibilityRole::Generic,
    };
    if (kind.is_layout() || matches!(kind_role, AccessibilityRole::Generic))
        && let Some(role) =
            landmark_role_from_tag(element_tag, accessible_name.is_some(), is_top_level)
    {
        return role;
    }
    kind_role
}

/// HTML landmark tags map to landmark roles per HTML-AAM. `section` and
/// `form` are only landmarks when they carry an accessible name; `header`
/// and `footer` only outside sectioning ancestors (ARIA in HTML).
pub(crate) fn landmark_role_from_tag(
    tag: &str,
    has_accessible_name: bool,
    is_top_level: bool,
) -> Option<AccessibilityRole> {
    match tag.trim().to_ascii_lowercase().as_str() {
        "main" => Some(AccessibilityRole::Main),
        "nav" => Some(AccessibilityRole::Navigation),
        "aside" => Some(AccessibilityRole::Complementary),
        "search" => Some(AccessibilityRole::Search),
        "header" if is_top_level => Some(AccessibilityRole::Banner),
        "footer" if is_top_level => Some(AccessibilityRole::ContentInfo),
        "section" if has_accessible_name => Some(AccessibilityRole::Region),
        "form" if has_accessible_name => Some(AccessibilityRole::Form),
        _ => None,
    }
}
