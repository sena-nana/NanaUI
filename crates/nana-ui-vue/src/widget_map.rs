//! L2 Semantics 映射（唯一入口）：tag / class / role / type → [`WidgetKind`]。
//!
//! ## L2 边界
//! - **本模块**：Semantics 解析（hint → kind）。业务 / kit BEM 不得在此发明 kind。
//! - **`bridge`**：持有 `WidgetKind` / `WidgetProps` / 森林；调用本模块，不复制 class 表。
//! - **`layout_map`**：Layout 方向默认（Column/Row），不解析控件语义。
//! - **`iced_app`**：kind → `nana_ui` 控件（L3 绘制）。
//!
//! 已知 class（如 `nana-btn` / `nana-chip`）→ kind + 后续 props；**不是**把 class
//! 当 ThemeTokens 工厂。Vue 自定义组件通过组合这些 kind 表达，不旁路 paint。
//!
//! Overlay / layout kinds come from documented `nana-*` contracts, HTML tags, and
//! ARIA `role` — not product kit BEM (`ui-dialog`, `ctx-menu`, `dd__menu`, …).

use crate::bridge::{SemanticSnapshot, SemanticWidget, WidgetId, WidgetKind, WidgetProps};

/// Downlevel HTML / role / class hints onto Nana foundations.
pub fn resolve_kind_from_hints(
    tag: &str,
    class: Option<&str>,
    role: Option<&str>,
    input_type: Option<&str>,
) -> Option<WidgetKind> {
    let tag = tag.trim().to_ascii_lowercase();
    let class = class.unwrap_or("").to_ascii_lowercase();
    let role = role.unwrap_or("").to_ascii_lowercase();
    let input_type = input_type.unwrap_or("").to_ascii_lowercase();

    if tag.starts_with("nana-") {
        if let Some(kind) = WidgetKind::parse(&tag) {
            return Some(kind);
        }
    }

    for token in class.split_whitespace() {
        if let Some(kind) = class_token_kind(token) {
            return Some(kind);
        }
    }

    match role.as_str() {
        "button" => return Some(WidgetKind::Button),
        "switch" => return Some(WidgetKind::Switch),
        "checkbox" => return Some(WidgetKind::Checkbox),
        "tablist" => return Some(WidgetKind::Tabs),
        "tab" => return Some(WidgetKind::Chip),
        "slider" => return Some(WidgetKind::Range),
        "progressbar" => return Some(WidgetKind::Progress),
        "listitem" => return Some(WidgetKind::ListItem),
        "dialog" | "alertdialog" => return Some(WidgetKind::Dialog),
        "complementary"
            if class
                .split_whitespace()
                .any(|t| matches!(t, "nana-drawer" | "nana-sheet")) =>
        {
            return Some(WidgetKind::Drawer);
        }
        "menu" | "menubar" => return Some(WidgetKind::ContextMenu),
        "listbox" | "combobox" => return Some(WidgetKind::Select),
        "group" if class.contains("nana-segmented") || class.contains("segmented") => {
            return Some(WidgetKind::Segmented);
        }
        _ => {}
    }

    if !is_html_tag_name(&tag) {
        if let Some(kind) = WidgetKind::parse(&tag) {
            return Some(kind);
        }
    }

    Some(match tag.as_str() {
        "button" => WidgetKind::Button,
        "input" => {
            if input_type == "checkbox" {
                WidgetKind::Checkbox
            } else if input_type == "range" {
                WidgetKind::Range
            } else {
                WidgetKind::Input
            }
        }
        "textarea" => WidgetKind::Textarea,
        "select" => WidgetKind::Select,
        "progress" => WidgetKind::Progress,
        "li" => WidgetKind::ListItem,
        // Lucide / <i> glyphs stay Icon; structural <svg> charts keep children.
        // Raster <img> nodes use the generic surface path; <i> remains an icon.
        "img" => WidgetKind::Box,
        "i" => WidgetKind::Icon,
        "svg" | "g" => {
            // Lucide Vue stamps `lucide lucide-<name>` on the root <svg>.
            if class
                .split_whitespace()
                .any(|t| t == "lucide" || t.starts_with("lucide-"))
            {
                WidgetKind::Icon
            } else {
                WidgetKind::Column
            }
        }
        "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => WidgetKind::Box,
        "text" => WidgetKind::Text,
        "span" | "p" | "label" | "strong" | "em" | "code" | "small" | "b" | "h1" | "h2" | "h3"
        | "h4" | "h5" | "h6" | "output" | "#text" => WidgetKind::Text,
        "div" | "section" | "article" | "main" | "aside" | "nav" | "header" | "footer" | "ul"
        | "ol" | "form" | "fieldset" | "body" | "template" | "fragment" => {
            // Direction comes from CSS / documented utilities only — never invent
            // Row from product `--row` / `*horizontal*` class substrings.
            if class
                .split_whitespace()
                .any(|t| matches!(t, "flex-row" | "hstack" | "nana-row" | "row"))
            {
                WidgetKind::Row
            } else {
                WidgetKind::Column
            }
        }
        _ if tag.is_empty() => WidgetKind::Column,
        _ => WidgetKind::Column,
    })
}

fn is_html_tag_name(tag: &str) -> bool {
    matches!(
        tag,
        "div"
            | "span"
            | "p"
            | "section"
            | "article"
            | "main"
            | "aside"
            | "nav"
            | "header"
            | "footer"
            | "ul"
            | "ol"
            | "li"
            | "form"
            | "fieldset"
            | "a"
            | "img"
            | "i"
            | "svg"
            | "g"
            | "path"
            | "rect"
            | "circle"
            | "ellipse"
            | "line"
            | "polyline"
            | "polygon"
            | "text"
            | "body"
            | "template"
            | "fragment"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "strong"
            | "em"
            | "code"
            | "small"
            | "b"
            | "label"
            | "output"
            | "#text"
            | "button"
            | "input"
            | "textarea"
            | "select"
            | "progress"
    )
}

fn class_token_kind(token: &str) -> Option<WidgetKind> {
    let t = token.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(kind) = WidgetKind::parse(t) {
        return Some(kind);
    }
    Some(match t {
        "nana-tabs" | "nana-tabs__list" => WidgetKind::Tabs,
        "nana-tabs__item" => WidgetKind::Chip,
        "nana-segmented" => WidgetKind::Segmented,
        "nana-segmented__item" => WidgetKind::Chip,
        "nana-chip" | "chip" => WidgetKind::Chip,
        "nana-range-field" | "nana-range" => WidgetKind::Range,
        "nana-button" | "ui-button" => WidgetKind::Button,
        "nana-switch" | "ui-switch" => WidgetKind::Switch,
        "nana-checkbox" | "ui-checkbox" => WidgetKind::Checkbox,
        "nana-input" | "ui-input" => WidgetKind::Input,
        "nana-card" | "ui-card" | "card" => WidgetKind::Card,
        "nana-list-item" | "ui-list-item" | "list-item" => WidgetKind::ListItem,
        "nana-sidebar-row" | "sidebar-row" | "nana-sidebar-nav__item" => WidgetKind::SidebarRow,
        "nana-sidebar-frame" => WidgetKind::SidebarFrame,
        "nana-settings-row" | "settings-row" => WidgetKind::SettingsRow,
        "nana-settings-card" | "settings-card" => WidgetKind::SettingsCard,
        "nana-empty" | "empty-state" => WidgetKind::EmptyState,
        "nana-status" | "nana-status-badge" => WidgetKind::StatusBadge,
        "nana-validation" | "nana-validation-message" => WidgetKind::ValidationMessage,
        "nana-labeled-value" => WidgetKind::LabeledValue,
        "nana-progress" | "ui-progress" => WidgetKind::Progress,
        "nana-spinner" | "ui-spinner" => WidgetKind::Spinner,
        "nana-form-field" | "nana-form" => WidgetKind::FormField,
        "nana-interactive-card" => WidgetKind::InteractiveCard,
        "nana-skeleton" => WidgetKind::Skeleton,
        "nana-level-meter" | "nana-level" => WidgetKind::LevelMeter,
        "nana-column" | "vstack" => WidgetKind::Column,
        "nana-row" | "hstack" | "flex-row" => WidgetKind::Row,
        // Documented overlay contracts (`nana-*` + generic HTML names).
        "nana-dialog" | "nana-overlay" | "nana-confirm" | "nana-confirm-dialog" => {
            WidgetKind::Dialog
        }
        "nana-drawer" | "nana-sheet" => WidgetKind::Drawer,
        "nana-popover" | "popover" => WidgetKind::Popover,
        "nana-context-menu" | "context-menu" | "contextmenu" => WidgetKind::ContextMenu,
        "nana-dropdown" | "nana-select" | "ui-dropdown" | "dropdown" => WidgetKind::Select,
        _ if t == "lucide" || t.starts_with("lucide-") => WidgetKind::Icon,
        _ if t.contains("sidebar") && t.contains("row") => WidgetKind::SidebarRow,
        // Do NOT match arbitrary "*card*" substrings — that promotes layout
        // shells (and blocks flex-direction → Row) into Card.
        _ => return None,
    })
}

pub(crate) fn first_button_child_id(
    snapshot: &SemanticSnapshot,
    widget: &SemanticWidget,
) -> Option<WidgetId> {
    widget.children.iter().copied().find(|&id| {
        snapshot
            .get(id)
            .is_some_and(|child| child.kind == WidgetKind::Button)
    })
}

fn is_input_like_kind(kind: WidgetKind) -> bool {
    matches!(
        kind,
        WidgetKind::Input
            | WidgetKind::Textarea
            | WidgetKind::Select
            | WidgetKind::Checkbox
            | WidgetKind::Switch
            | WidgetKind::Range
    )
}

pub(crate) fn form_field_control_child_id(
    snapshot: &SemanticSnapshot,
    widget: &SemanticWidget,
) -> Option<WidgetId> {
    let mut first_non_text = None;
    let mut first_input_like = None;
    for &id in &widget.children {
        let Some(child) = snapshot.get(id) else {
            continue;
        };
        if first_non_text.is_none() && child.kind != WidgetKind::Text {
            first_non_text = Some(id);
        }
        if first_input_like.is_none() && is_input_like_kind(child.kind) {
            first_input_like = Some(id);
        }
    }
    first_non_text.or(first_input_like)
}

pub(crate) fn form_field_support(props: &WidgetProps) -> (Option<&str>, Option<&str>) {
    if props.hint.is_empty() {
        return (None, None);
    }
    if props.invalid {
        (None, Some(props.hint.as_str()))
    } else {
        (Some(props.hint.as_str()), None)
    }
}

pub(crate) fn level_meter_value(props: &WidgetProps) -> f32 {
    if props.progress.is_finite() && props.progress != 0.0 {
        props.progress
    } else if props.number.is_finite() {
        props.number
    } else {
        0.0
    }
}

pub(crate) fn labeled_value_caption(
    snapshot: &SemanticSnapshot,
    widget: &SemanticWidget,
) -> String {
    if !widget.props.label.is_empty() {
        return widget.props.label.clone();
    }
    widget
        .children
        .iter()
        .filter_map(|id| snapshot.get(*id))
        .find(|child| child.kind == WidgetKind::Text)
        .map(|child| child.props.display_label().to_string())
        .filter(|label| !label.is_empty())
        .unwrap_or_default()
}

pub(crate) fn validation_message_text(props: &WidgetProps) -> String {
    if !props.hint.is_empty() {
        props.hint.clone()
    } else {
        props.display_label().to_string()
    }
}

pub(crate) fn class_has_compact(props: &WidgetProps) -> bool {
    props
        .class_names
        .iter()
        .any(|class| class.contains("compact"))
}

pub(crate) fn attr_value<'a>(props: &'a WidgetProps, names: &[&str]) -> Option<&'a str> {
    for name in names {
        if let Some(value) = props.attrs.get(*name) {
            return Some(value.as_str());
        }
        if let Some((_, value)) = props
            .attrs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
        {
            return Some(value.as_str());
        }
    }
    None
}

pub(crate) fn parse_status_tone(raw: &str) -> Option<nana_ui_core::StatusTone> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "neutral" => Some(nana_ui_core::StatusTone::Neutral),
        "info" => Some(nana_ui_core::StatusTone::Info),
        "success" => Some(nana_ui_core::StatusTone::Success),
        "warning" | "warn" => Some(nana_ui_core::StatusTone::Warning),
        "danger" | "error" => Some(nana_ui_core::StatusTone::Danger),
        _ => None,
    }
}

pub(crate) fn status_tone_from_props(props: &WidgetProps) -> nana_ui_core::StatusTone {
    if let Some(tone) = attr_value(props, &["tone", "data-tone"]).and_then(parse_status_tone) {
        return tone;
    }
    for class in &props.class_names {
        if let Some(suffix) = class.rsplit_once("--").map(|(_, suffix)| suffix)
            && let Some(tone) = parse_status_tone(suffix)
        {
            return tone;
        }
    }
    nana_ui_core::StatusTone::Neutral
}

pub(crate) fn parse_validation_intent(raw: &str) -> Option<nana_ui_core::ValidationIntent> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "danger" | "error" => Some(nana_ui_core::ValidationIntent::Danger),
        "warning" | "warn" => Some(nana_ui_core::ValidationIntent::Warning),
        _ => None,
    }
}

pub(crate) fn validation_intent_from_props(props: &WidgetProps) -> nana_ui_core::ValidationIntent {
    if props.invalid {
        return nana_ui_core::ValidationIntent::Danger;
    }
    match attr_value(props, &["intent", "data-intent"]).and_then(parse_validation_intent) {
        Some(nana_ui_core::ValidationIntent::Danger) => nana_ui_core::ValidationIntent::Danger,
        _ => nana_ui_core::ValidationIntent::Warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_sidebar_nav_classes_map() {
        assert_eq!(
            resolve_kind_from_hints("section", Some("nana-settings-card"), None, None),
            Some(WidgetKind::SettingsCard)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-settings-row settings-row"), None, None),
            Some(WidgetKind::SettingsRow)
        );
        assert_eq!(
            resolve_kind_from_hints(
                "button",
                Some("nana-sidebar-nav__item is-active"),
                None,
                None
            ),
            Some(WidgetKind::SidebarRow)
        );
    }

    #[test]
    fn lucide_svg_maps_to_icon() {
        assert_eq!(
            resolve_kind_from_hints("svg", Some("lucide lucide-search"), None, None),
            Some(WidgetKind::Icon)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("lucide lucide-plus"), None, None),
            Some(WidgetKind::Icon),
            "class re-resolve without svg tag still upgrades Lucide"
        );
        assert_eq!(
            resolve_kind_from_hints("svg", Some("calendar-heatmap"), None, None),
            Some(WidgetKind::Column),
            "structural chart svg stays Column"
        );
    }

    #[test]
    fn maps_button_input_switch_and_flex_row_div() {
        assert_eq!(
            resolve_kind_from_hints("button", None, None, None),
            Some(WidgetKind::Button)
        );
        assert_eq!(
            resolve_kind_from_hints("input", None, None, Some("checkbox")),
            Some(WidgetKind::Checkbox)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-switch"), None, None),
            Some(WidgetKind::Switch)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("flex-row gap-md"), None, None),
            Some(WidgetKind::Row)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-column", None, None, None),
            Some(WidgetKind::Column)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("toolbar--row"), None, None),
            Some(WidgetKind::Column),
            "must not invent Row from --row BEM"
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("panel-horizontal"), None, None),
            Some(WidgetKind::Column),
            "must not invent Row from horizontal substring"
        );
    }

    #[test]
    fn overlay_surfaces_map_via_nana_contract_and_roles() {
        assert_eq!(
            resolve_kind_from_hints(
                "div",
                Some("nana-overlay nana-dialog"),
                Some("dialog"),
                None
            ),
            Some(WidgetKind::Dialog)
        );
        assert_eq!(
            resolve_kind_from_hints("div", None, Some("dialog"), None),
            Some(WidgetKind::Dialog)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-context-menu"), Some("menu"), None),
            Some(WidgetKind::ContextMenu)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-dropdown"), None, None),
            Some(WidgetKind::Select)
        );
        assert_eq!(
            resolve_kind_from_hints("div", None, Some("listbox"), None),
            Some(WidgetKind::Select)
        );
        assert_eq!(
            resolve_kind_from_hints("div", None, Some("combobox"), None),
            Some(WidgetKind::Select)
        );
        // Product kit BEM alone must not invent overlay kinds.
        assert_eq!(
            resolve_kind_from_hints("div", Some("ui-overlay ui-dialog"), None, None),
            Some(WidgetKind::Column)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("ctx-menu"), None, None),
            Some(WidgetKind::Column)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("dd__menu"), None, None),
            Some(WidgetKind::Column)
        );
    }

    #[test]
    fn documented_feedback_classes_map_without_promoting_html_text() {
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-status"), None, None),
            Some(WidgetKind::StatusBadge)
        );
        assert_eq!(
            resolve_kind_from_hints(
                "div",
                Some("nana-status-badge nana-status--danger"),
                None,
                None
            ),
            Some(WidgetKind::StatusBadge)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-validation"), None, None),
            Some(WidgetKind::ValidationMessage)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-validation-message"), None, None),
            Some(WidgetKind::ValidationMessage)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-labeled-value"), None, None),
            Some(WidgetKind::LabeledValue)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-form-field"), None, None),
            Some(WidgetKind::FormField)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-form", None, None, None),
            Some(WidgetKind::FormField)
        );
        assert_eq!(
            resolve_kind_from_hints("form", None, None, None),
            Some(WidgetKind::Column),
            "HTML form stays a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-interactive-card"), None, None),
            Some(WidgetKind::InteractiveCard)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-skeleton"), None, None),
            Some(WidgetKind::Skeleton)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-level-meter"), None, None),
            Some(WidgetKind::LevelMeter)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-level", None, None, None),
            Some(WidgetKind::LevelMeter)
        );
        assert_eq!(
            resolve_kind_from_hints("span", None, None, None),
            Some(WidgetKind::Text)
        );
        assert_eq!(
            resolve_kind_from_hints("output", None, None, None),
            Some(WidgetKind::Text)
        );
    }

    #[test]
    fn form_field_support_promotes_hint_to_error_when_invalid() {
        let mut props = WidgetProps::default();
        props.hint = "Required".into();
        assert_eq!(form_field_support(&props), (Some("Required"), None));
        props.invalid = true;
        assert_eq!(form_field_support(&props), (None, Some("Required")));
    }

    #[test]
    fn level_meter_value_prefers_progress_then_number() {
        let mut props = WidgetProps::default();
        props.number = 0.25;
        assert!((level_meter_value(&props) - 0.25).abs() < f32::EPSILON);
        props.progress = 0.8;
        assert!((level_meter_value(&props) - 0.8).abs() < f32::EPSILON);
    }
}
