//! L2 Semantics 映射（唯一入口）：tag / class / role / type → [`WidgetKind`]。
//!
//! ## L2 边界
//! - **本模块**：Semantics 解析（hint → kind）。业务 / kit BEM 不得在此发明 kind。
//! - **`bridge`**：持有 `WidgetKind` / `WidgetProps` / 森林；调用本模块，不复制 class 表。
//! - **`layout_map`**：Layout 方向默认（Column/Row），不解析控件语义。
//! - Runtime / Scene host：kind → `nana_ui` Runtime 控件。
//!
//! 已知 class（如 `nana-btn` / `nana-chip`）→ kind + 后续 props；**不是**把 class
//! 当 ThemeTokens 工厂。Vue 自定义组件通过组合这些 kind 表达，不旁路 paint。
//! 可实例化权威是 Runtime `ComponentRegistry`（tag → type id）；`WidgetKind`
//! 仍负责 CSS / HTML downlevel。未注册自定义 tag 报错，不落到布局盒。
//!
//! Overlay / layout kinds come from documented `nana-*` contracts, HTML tags, and
//! ARIA `role` — not product kit BEM (`ui-dialog`, `ctx-menu`, `dd__menu`, …).

use crate::tree::semantic_read::{SemanticRead, SemanticWidgetView};
use std::collections::BTreeMap;

use crate::bridge::{WidgetId, WidgetKind, WidgetProps};

/// Resolve a host element tag. Known HTML / `nana-*` kinds succeed; retired
/// HTML aliases and tags that are neither HTML nor a registered component fail.
pub fn resolve_host_tag_kind(
    tag: &str,
    class: Option<&str>,
    role: Option<&str>,
    input_type: Option<&str>,
    is_registered_component: impl FnOnce(&str) -> bool,
) -> Result<WidgetKind, String> {
    let normalized = tag.trim().to_ascii_lowercase();
    if let Some(replacement) = retired_html_alias_replacement(&normalized) {
        return Err(format!("retired tag `{normalized}`; use `{replacement}`"));
    }
    if let Some(kind) = resolve_kind_from_hints(tag, class, role, input_type) {
        return Ok(kind);
    }
    if !normalized.is_empty() && is_registered_component(&normalized) {
        return Ok(WidgetKind::Column);
    }
    Err(format!("unknown tag: {normalized}"))
}

/// DOM text nodes (`#text` / `createText` / `nana-text`), not L2 `p` / `span` /
/// `label` / `h1`–`h6`. `WidgetKind::Text` is shared by both.
pub fn is_dom_text_node(kind: WidgetKind, tag: &str) -> bool {
    let resolved = if tag.is_empty() {
        if kind == WidgetKind::Text {
            "nana-text"
        } else {
            return false;
        }
    } else {
        tag
    };
    resolved.eq_ignore_ascii_case("#text")
        || resolved.eq_ignore_ascii_case("nana-text")
        || resolved.eq_ignore_ascii_case("createtext")
}

/// Checkbox / radio / switch hosts for cheap `:checked` (`toggled`).
/// Overlay `toggled` (dialog / drawer) is not a checked control.
pub fn is_checked_match_host(
    kind: WidgetKind,
    tag: &str,
    attrs: &BTreeMap<String, String>,
) -> bool {
    if matches!(
        kind,
        WidgetKind::Checkbox | WidgetKind::Switch | WidgetKind::Radio
    ) {
        return true;
    }
    let ty = attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("type"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    let type_ok = ty.eq_ignore_ascii_case("checkbox") || ty.eq_ignore_ascii_case("radio");
    if !type_ok {
        return false;
    }
    kind == WidgetKind::Input || tag.eq_ignore_ascii_case("input")
}

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

    if retired_html_alias_replacement(&tag).is_some() {
        return None;
    }

    if tag.starts_with("nana-")
        && let Some(kind) = WidgetKind::parse(&tag)
    {
        return Some(kind);
    }

    for token in class.split_whitespace() {
        if let Some(kind) = class_token_kind(token) {
            // Structural SVG charts keep Column; catalog CalendarHeatmap / GraphCanvas
            // are leaves. Do not promote `calendar-heatmap` SVG class to GraphCanvas.
            if matches!(kind, WidgetKind::CalendarHeatmap | WidgetKind::GraphCanvas)
                && is_svg_structural_tag(&tag)
            {
                continue;
            }
            return Some(kind);
        }
    }

    match role.as_str() {
        "button" => return Some(WidgetKind::Button),
        "link" => return Some(WidgetKind::Button),
        "switch" => return Some(WidgetKind::Switch),
        "checkbox" => return Some(WidgetKind::Checkbox),
        "radio" => return Some(WidgetKind::Radio),
        "radiogroup" => return Some(WidgetKind::Segmented),
        "tablist" => return Some(WidgetKind::Tabs),
        "tab" => return Some(WidgetKind::Chip),
        "slider" => return Some(WidgetKind::Range),
        "progressbar" => return Some(WidgetKind::Progress),
        "meter" => return Some(WidgetKind::LevelMeter),
        "list" => return Some(WidgetKind::List),
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
        "tooltip" => return Some(WidgetKind::Tooltip),
        "menuitem" => return Some(WidgetKind::ActionMenuItem),
        "listbox" | "combobox" => return Some(WidgetKind::Select),
        "group" if class.contains("nana-segmented") || class.contains("segmented") => {
            return Some(WidgetKind::Segmented);
        }
        _ => {}
    }

    if !is_html_tag_name(&tag)
        && let Some(kind) = WidgetKind::parse(&tag)
    {
        return Some(kind);
    }

    Some(match tag.as_str() {
        "button" => WidgetKind::Button,
        "a" => WidgetKind::Button,
        "input" => match input_type.as_str() {
            "checkbox" => WidgetKind::Checkbox,
            "radio" => WidgetKind::Radio,
            "range" => WidgetKind::Range,
            "number" => WidgetKind::NumberInput,
            _ => WidgetKind::Input,
        },
        "textarea" | "hosted-textarea" | "nana-hosted-textarea" => WidgetKind::Textarea,
        "select" => WidgetKind::Select,
        "progress" => WidgetKind::Progress,
        "meter" => WidgetKind::LevelMeter,
        "dialog" => WidgetKind::Dialog,
        "details" => WidgetKind::SettingsCollapsibleCard,
        "summary" | "legend" | "caption" | "figcaption" | "address" => WidgetKind::Text,
        "hr" => WidgetKind::Divider,
        "br" => WidgetKind::Text,
        "table" => WidgetKind::Table,
        "tr" => WidgetKind::TableRow,
        "td" | "th" => WidgetKind::TableCell,
        "thead" | "tbody" | "tfoot" => WidgetKind::Column,
        "ul" | "ol" | "menu" => WidgetKind::List,
        "li" => WidgetKind::ListItem,
        // Lucide / <i> glyphs stay Icon; structural <svg> is rasterized via
        // the shared image_url cache (see `svg_inline`). Raster <img> binds
        // `src` onto PaintStyle.content_image. `<canvas>` is a HostTexture slot
        // (`canvas:{id}`), not a fake 2D context. `<video>` samples the
        // `video:{id}` host-texture slot via the Runtime Video control; poster
        // stays the pre-first-frame fallback. `<iframe>`, `<audio>`,
        // `<embed>` and `<object>` are explicit L1 skips: there is no host
        // pipeline behind them yet (audio can follow the video host-feed path
        // when one exists), so they stay invisible layout boxes instead of
        // faking a player.
        "img" => WidgetKind::Box,
        "canvas" => WidgetKind::Box,
        "video" => WidgetKind::Video,
        "iframe" | "audio" | "embed" | "object" => WidgetKind::Box,
        // Media sub-resources and image-map hotspots have no visual of their
        // own; Runtime `Table` has no column definitions, so `col`/`colgroup`
        // skip the same way.
        "source" | "track" | "area" | "col" | "colgroup" => WidgetKind::Box,
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
        "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "defs"
        | "use" | "symbol" | "clippath" | "mask" | "lineargradient" | "radialgradient" | "stop"
        | "textpath" | "tspan" => WidgetKind::Box,
        "text" => WidgetKind::Text,
        "span" | "p" | "label" | "strong" | "em" | "code" | "small" | "b" | "h1" | "h2" | "h3"
        | "h4" | "h5" | "h6" | "output" | "pre" | "blockquote" | "dt" | "dd" | "#text" | "abbr"
        | "cite" | "q" | "kbd" | "samp" | "var" | "mark" | "time" | "ins" | "del" | "sub"
        | "sup" | "s" | "u" | "dfn" | "data" | "ruby" | "rt" | "rp" | "bdi" | "bdo" | "wbr" => {
            WidgetKind::Text
        }
        "div" | "section" | "article" | "main" | "aside" | "nav" | "header" | "footer" | "form"
        | "search" | "fieldset" | "dl" | "body" | "template" | "fragment" | "html" | "head"
        | "title" | "meta" | "link" | "style" | "script" | "noscript" | "base" | "math" => {
            // Direction is LayoutStyle only (CSS / class hints). Tag stays Column.
            WidgetKind::Column
        }
        // Block containers and slots: `<picture>` wraps `<source>`/`<img>` (the
        // img child still binds `src`), `<datalist>` is a data container whose
        // suggestions surface through the associated `input`, `<slot>` is
        // already expanded by the Vue runtime before it reaches the host, and
        // `<map>` only positions hotspots this renderer does not support.
        "figure" | "hgroup" | "picture" | "datalist" | "slot" | "map" | "option" | "optgroup" => {
            WidgetKind::Column
        }
        "pattern" | "image" | "foreignobject" | "desc" => WidgetKind::Box,
        _ if tag.is_empty() => WidgetKind::Column,
        _ => return None,
    })
}

fn is_svg_structural_tag(tag: &str) -> bool {
    matches!(
        tag,
        "svg"
            | "g"
            | "path"
            | "rect"
            | "circle"
            | "ellipse"
            | "line"
            | "polyline"
            | "polygon"
            | "defs"
            | "use"
            | "symbol"
            | "clippath"
            | "mask"
            | "lineargradient"
            | "radialgradient"
            | "stop"
            | "textpath"
            | "tspan"
    )
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
            | "search"
            | "fieldset"
            | "legend"
            | "dl"
            | "dt"
            | "dd"
            | "a"
            | "details"
            | "summary"
            | "meter"
            | "option"
            | "optgroup"
            | "caption"
            | "figcaption"
            | "figure"
            | "hgroup"
            | "address"
            | "menu"
            | "datalist"
            | "col"
            | "colgroup"
            | "slot"
            | "picture"
            | "audio"
            | "source"
            | "track"
            | "embed"
            | "object"
            | "map"
            | "area"
            | "br"
            | "pre"
            | "blockquote"
            | "img"
            | "canvas"
            | "video"
            | "iframe"
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
            | "defs"
            | "use"
            | "symbol"
            | "clippath"
            | "mask"
            | "lineargradient"
            | "radialgradient"
            | "stop"
            | "text"
            | "textpath"
            | "tspan"
            | "body"
            | "template"
            | "fragment"
            | "html"
            | "head"
            | "title"
            | "meta"
            | "link"
            | "style"
            | "script"
            | "noscript"
            | "base"
            | "math"
            | "pattern"
            | "image"
            | "foreignobject"
            | "desc"
            | "abbr"
            | "cite"
            | "q"
            | "kbd"
            | "samp"
            | "var"
            | "mark"
            | "time"
            | "ins"
            | "del"
            | "sub"
            | "sup"
            | "s"
            | "u"
            | "dfn"
            | "data"
            | "ruby"
            | "rt"
            | "rp"
            | "bdi"
            | "bdo"
            | "wbr"
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
            | "dialog"
            | "hr"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
    )
}

fn retired_html_alias_replacement(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "nana-button" => "button",
        "nana-text-input" | "nana-input" => "input",
        "nana-textarea" => "textarea",
        "nana-select" => "select",
        "nana-progress" => "progress",
        "nana-divider" => "hr",
        "nana-dialog" => "dialog",
        "nana-checkbox" => "input",
        "nana-range-field" | "nana-range" => "input",
        "nana-table" => "table",
        "nana-table-row" => "tr",
        "nana-table-cell" => "td",
        "nana-search" => "search-dropdown",
        "nana-list" => "ul",
        "nana-list-item" => "li",
        "nana-number-input" => "input",
        "nana-level-meter" => "meter",
        "nana-settings-collapsible-card" => "details",
        _ => return None,
    })
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
        "nana-tabs__list" => WidgetKind::Tabs,
        "nana-tabs__item" | "nana-segmented__item" => WidgetKind::Chip,
        "nana-range" => WidgetKind::Range,
        "ui-button" => WidgetKind::Button,
        "nana-scroll" => WidgetKind::ScrollView,
        "ui-switch" => WidgetKind::Switch,
        "ui-checkbox" => WidgetKind::Checkbox,
        "nana-input" | "ui-input" => WidgetKind::Input,
        "nana-hosted-textarea" | "hosted-textarea" => WidgetKind::Textarea,
        "ui-card" => WidgetKind::Card,
        "ui-list-item" => WidgetKind::ListItem,
        "nana-sidebar-nav__item" => WidgetKind::SidebarRow,
        "nana-empty" => WidgetKind::EmptyState,
        "nana-avatar" => WidgetKind::Avatar,
        "nana-status" => WidgetKind::StatusBadge,
        "nana-validation" => WidgetKind::ValidationMessage,
        "ui-progress" => WidgetKind::Progress,
        "ui-spinner" => WidgetKind::Spinner,
        "vstack" => WidgetKind::Column,
        "hstack" => WidgetKind::Row,
        "nana-overlay" | "nana-confirm" | "nana-confirm-dialog" => WidgetKind::Dialog,
        "nana-sheet" => WidgetKind::Drawer,
        "nana-xypad" => WidgetKind::XYPad,
        "nana-calendar" => WidgetKind::CalendarHeatmap,
        "nana-markdown" => WidgetKind::NativeMarkdown,
        "graphcanvas" => WidgetKind::GraphCanvas,
        "ui-dropdown" => WidgetKind::Dropdown,
        "nana-search" | "search-dropdown" => WidgetKind::SearchDropdown,
        _ if t == "lucide" || t.starts_with("lucide-") => WidgetKind::Icon,
        _ if t.contains("sidebar") && t.contains("row") => WidgetKind::SidebarRow,
        _ => return None,
    })
}

pub(crate) fn first_button_child_id(
    snapshot: &SemanticRead<'_>,
    widget: &SemanticWidgetView<'_>,
) -> Option<WidgetId> {
    widget.children.iter().copied().find(|&id| {
        snapshot
            .get(id)
            .is_some_and(|child| child.kind == WidgetKind::Button)
    })
}

fn is_input_like_kind(kind: WidgetKind) -> bool {
    kind.is_choice_field()
        || matches!(
            kind,
            WidgetKind::Input
                | WidgetKind::NumberInput
                | WidgetKind::Textarea
                | WidgetKind::Checkbox
                | WidgetKind::Radio
                | WidgetKind::Switch
                | WidgetKind::Range
        )
}

#[derive(Clone, Copy)]
pub(crate) struct SettingsRowSlots {
    pub copy: Option<WidgetId>,
    pub label: Option<WidgetId>,
    pub hint: Option<WidgetId>,
}

impl SettingsRowSlots {
    fn contains(self, id: WidgetId) -> bool {
        self.copy == Some(id) || self.label == Some(id) || self.hint == Some(id)
    }
}

fn settings_row_marked(props: &WidgetProps, slot: &str, classes: &[&str]) -> bool {
    props.attrs.get("data-slot").map(String::as_str) == Some(slot)
        || props
            .class_names
            .iter()
            .any(|class| classes.iter().any(|needle| class.contains(needle)))
}

fn settings_row_child(
    snapshot: &SemanticRead<'_>,
    parent: &SemanticWidgetView<'_>,
    pred: impl Fn(&WidgetProps) -> bool,
) -> Option<WidgetId> {
    parent
        .children
        .iter()
        .copied()
        .find(|&id| snapshot.get(id).is_some_and(|child| pred(&child.props)))
}

fn settings_row_descendent(
    snapshot: &SemanticRead<'_>,
    id: WidgetId,
    pred: impl Fn(&WidgetProps) -> bool + Copy,
) -> Option<WidgetId> {
    let widget = snapshot.get(id)?;
    for &child in widget.children.iter() {
        if snapshot.get(child).is_some_and(|node| pred(&node.props)) {
            return Some(child);
        }
        if let Some(found) = settings_row_descendent(snapshot, child, pred) {
            return Some(found);
        }
    }
    None
}

fn settings_row_text_or_self(snapshot: &SemanticRead<'_>, id: WidgetId) -> WidgetId {
    snapshot
        .get(id)
        .and_then(|node| {
            (node.kind == WidgetKind::Text).then_some(id).or_else(|| {
                node.children.iter().copied().find(|&child| {
                    snapshot
                        .get(child)
                        .is_some_and(|node| node.kind == WidgetKind::Text)
                })
            })
        })
        .unwrap_or(id)
}

pub(crate) fn settings_row_slots(
    snapshot: &SemanticRead<'_>,
    widget: &SemanticWidgetView<'_>,
) -> SettingsRowSlots {
    let is_label = |props: &WidgetProps| {
        settings_row_marked(
            props,
            "label",
            &["nana-settings-row__label", "settings-row__label"],
        )
    };
    let is_hint = |props: &WidgetProps| {
        settings_row_marked(
            props,
            "hint",
            &["nana-settings-row__hint", "settings-row__hint"],
        )
    };
    let container = settings_row_child(snapshot, widget, is_label);
    let hint = settings_row_child(snapshot, widget, is_hint)
        .or_else(|| container.and_then(|id| settings_row_descendent(snapshot, id, is_hint)))
        .map(|id| settings_row_text_or_self(snapshot, id));
    let label = container.and_then(|id| {
        let node = snapshot.get(id)?;
        node.children
            .iter()
            .copied()
            .find(|&child| {
                snapshot
                    .get(child)
                    .is_some_and(|child| !is_hint(&child.props) && child.kind == WidgetKind::Text)
            })
            .or_else(|| {
                node.children.iter().copied().find(|&child| {
                    snapshot
                        .get(child)
                        .is_some_and(|child| !is_hint(&child.props))
                })
            })
            .or(Some(id))
    });
    let copy = match (container, hint, label) {
        (Some(container), Some(hint), Some(label)) if hint != container && label != container => {
            Some(container)
        }
        (Some(container), Some(hint), None) if hint != container => Some(container),
        _ => None,
    };
    SettingsRowSlots { copy, label, hint }
}

pub(crate) fn settings_row_plain_text(snapshot: &SemanticRead<'_>, id: WidgetId) -> String {
    let Some(widget) = snapshot.get(id) else {
        return String::new();
    };
    let mut text = widget.props.display_label().to_string();
    for &child in widget.children.iter() {
        let child = settings_row_plain_text(snapshot, child);
        if child.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&child);
    }
    text
}

pub(crate) fn is_settings_row_projected_slot(
    snapshot: &SemanticRead<'_>,
    widget: &SemanticWidgetView<'_>,
) -> bool {
    let mut parent = widget.parent;
    while let Some(parent_id) = parent {
        let Some(row) = snapshot.get(parent_id) else {
            break;
        };
        if row.kind == WidgetKind::SettingsRow {
            let slots = settings_row_slots(snapshot, &row);
            let mut id = Some(widget.id);
            while let Some(node_id) = id {
                if node_id == parent_id {
                    break;
                }
                if slots.contains(node_id) {
                    return true;
                }
                id = snapshot.get(node_id).and_then(|node| node.parent);
            }
            return false;
        }
        parent = row.parent;
    }
    false
}

pub(crate) fn settings_row_control_child_id(
    snapshot: &SemanticRead<'_>,
    widget: &SemanticWidgetView<'_>,
) -> Option<WidgetId> {
    widget
        .children
        .iter()
        .copied()
        .find(|&id| {
            snapshot.get(id).is_some_and(|child| {
                child.props.attrs.get("data-slot").map(String::as_str) == Some("control")
                    || child.props.class_names.iter().any(|class| {
                        class.contains("settings-row__control")
                            || class.contains("nana-settings-row__control")
                    })
            })
        })
        .or_else(|| {
            widget.children.iter().rev().copied().find(|&id| {
                snapshot.get(id).is_some_and(|child| {
                    !child.props.class_names.iter().any(|class| {
                        class.contains("settings-row__label")
                            || class.contains("nana-settings-row__label")
                    })
                })
            })
        })
}

pub(crate) fn settings_row_flags(props: &WidgetProps) -> (bool, bool, bool, bool, bool) {
    let class_has = |needles: &[&str]| {
        props
            .class_names
            .iter()
            .any(|class| needles.iter().any(|needle| class.contains(needle)))
    };
    let attr_true = |names: &[&str]| {
        attr_value(props, names).is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "true" | "1" | "yes"
            )
        })
    };
    (
        class_has(&["nana-settings-row--stacked", "settings-row--stacked"])
            || attr_true(&["stacked"]),
        class_has(&["nana-settings-row--divided", "settings-row--divided"])
            || attr_true(&["divided"]),
        class_has(&[
            "nana-settings-row__control--loose",
            "settings-row__control--loose",
        ]) || attr_true(&["loose"]),
        class_has(&["is-first"]) || attr_true(&["first-in-group", "firstInGroup"]),
        class_has(&["is-last"]) || attr_true(&["last-in-group", "lastInGroup"]),
    )
}

pub(crate) fn sidebar_frame_slots(
    snapshot: &SemanticRead<'_>,
    widget: &SemanticWidgetView<'_>,
) -> (Option<WidgetId>, Option<WidgetId>, Option<WidgetId>) {
    let mut top = None;
    let mut body = None;
    let mut footer = None;
    for &id in widget.children.iter() {
        let Some(child) = snapshot.get(id) else {
            continue;
        };
        let slot = child.props.attrs.get("data-slot").map(String::as_str);
        let classes = &child.props.class_names;
        if slot == Some("sidebar-top")
            || classes
                .iter()
                .any(|class| class.contains("nana-sidebar-frame__top"))
        {
            top = Some(id);
        } else if slot == Some("sidebar-body")
            || classes
                .iter()
                .any(|class| class.contains("nana-sidebar-frame__body"))
        {
            body = Some(id);
        } else if slot == Some("sidebar-footer")
            || classes
                .iter()
                .any(|class| class.contains("nana-sidebar-frame__footer"))
        {
            footer = Some(id);
        }
    }
    (top, body, footer)
}

pub(crate) fn sidebar_row_tone(props: &WidgetProps) -> nana_ui_runtime::SidebarRowTone {
    let raw = attr_value(props, &["tone", "data-tone"]).unwrap_or("");
    let class_has = |needle: &str| props.class_names.iter().any(|class| class.contains(needle));
    if raw.eq_ignore_ascii_case("warning") || class_has("warning") {
        nana_ui_runtime::SidebarRowTone::Warning
    } else if raw.eq_ignore_ascii_case("error")
        || raw.eq_ignore_ascii_case("danger")
        || class_has("error")
        || class_has("danger")
    {
        nana_ui_runtime::SidebarRowTone::Error
    } else {
        nana_ui_runtime::SidebarRowTone::Default
    }
}

pub(crate) fn sidebar_row_state(props: &WidgetProps) -> nana_ui_runtime::SidebarRowState {
    if props.disabled {
        nana_ui_runtime::SidebarRowState::Disabled
    } else if props
        .class_names
        .iter()
        .any(|class| class.contains("ancestor"))
    {
        nana_ui_runtime::SidebarRowState::AncestorActive
    } else if props.active {
        nana_ui_runtime::SidebarRowState::Active
    } else {
        nana_ui_runtime::SidebarRowState::Idle
    }
}

pub(crate) fn sidebar_row_depth(props: &WidgetProps) -> u16 {
    attr_value(props, &["depth", "data-depth", "indent"])
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(0)
}

pub(crate) fn form_field_control_child_id(
    snapshot: &SemanticRead<'_>,
    widget: &SemanticWidgetView<'_>,
) -> Option<WidgetId> {
    let mut first_non_text = None;
    let mut first_input_like = None;
    for &id in widget.children.iter() {
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

pub(crate) fn progress_cancellable(props: &WidgetProps) -> bool {
    attr_value(props, &["cancel", "cancellable", "dismissible", "closable"]).is_some()
        || props.attrs.contains_key("ondismiss")
        || props
            .class_names
            .iter()
            .any(|class| class.contains("cancel") || class.contains("dismissible"))
}

pub(crate) fn is_search_dropdown(props: &WidgetProps) -> bool {
    tag_or_class_contains(props, &["search-dropdown", "nana-search", "searchdropdown"])
}

pub(crate) fn is_dropdown_field(props: &WidgetProps) -> bool {
    if is_search_dropdown(props) {
        return false;
    }
    props.attrs.contains_key("multiple")
        || tag_or_class_contains(props, &["nana-dropdown", "dropdown"])
}

fn tag_or_class_contains(props: &WidgetProps, needles: &[&str]) -> bool {
    let tag = props.element_tag.to_ascii_lowercase();
    needles.iter().any(|needle| {
        tag.contains(needle)
            || props
                .class_names
                .iter()
                .any(|class| class.eq_ignore_ascii_case(needle) || class.contains(needle))
    })
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
    snapshot: &SemanticRead<'_>,
    widget: &SemanticWidgetView<'_>,
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

pub(crate) fn class_has_compact(props: &WidgetProps) -> bool {
    props
        .class_names
        .iter()
        .any(|class| class.contains("compact"))
}

pub(crate) fn highlight_language(props: &WidgetProps) -> Option<&str> {
    attr_value(props, &["language", "lang", "syntax"])
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// HTML `dir="rtl"|"ltr"|"auto"` → [`nana_ui_core::DirSpec`].
///
/// `auto` / unknown stay `None` (no first-strong bidi; do not fake `ltr`).
/// Production callers use [`html_dir_spec_from_map`] directly.
#[cfg(test)]
pub(crate) fn html_dir_spec(props: &WidgetProps) -> Option<nana_ui_core::DirSpec> {
    html_dir_spec_from_map(&props.attrs)
}

pub(crate) fn html_dir_spec_from_map(
    attrs: &std::collections::BTreeMap<String, String>,
) -> Option<nana_ui_core::DirSpec> {
    attrs
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("dir"))
        .and_then(|(_, value)| crate::css_map::dir_spec_from_html_attr(value))
}

pub(crate) fn is_radio_group(props: &WidgetProps) -> bool {
    props.role.eq_ignore_ascii_case("radiogroup")
        || attr_value(props, &["chrome"]).is_some_and(|value| value.eq_ignore_ascii_case("radio"))
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

pub(crate) fn parse_toast_tone(raw: &str) -> Option<nana_ui_core::ToastTone> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "info" => Some(nana_ui_core::ToastTone::Info),
        "success" => Some(nana_ui_core::ToastTone::Success),
        "warning" | "warn" => Some(nana_ui_core::ToastTone::Warning),
        "danger" | "error" => Some(nana_ui_core::ToastTone::Danger),
        _ => None,
    }
}

pub(crate) fn toast_tone_from_props(props: &WidgetProps) -> nana_ui_core::ToastTone {
    if let Some(tone) = attr_value(props, &["tone", "data-tone"]).and_then(parse_toast_tone) {
        return tone;
    }
    for class in &props.class_names {
        if let Some(suffix) = class.rsplit_once("--").map(|(_, suffix)| suffix)
            && let Some(tone) = parse_toast_tone(suffix)
        {
            return tone;
        }
    }
    nana_ui_core::ToastTone::Info
}

fn attr_flag(props: &WidgetProps, names: &[&str]) -> bool {
    for name in names {
        let Some(value) = attr_value(props, &[name]) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty()
            || value.eq_ignore_ascii_case("true")
            || value == "1"
            || value.eq_ignore_ascii_case(name)
        {
            return true;
        }
        if value.eq_ignore_ascii_case("false") || value == "0" {
            return false;
        }
        return true;
    }
    false
}

pub(crate) fn toast_dismissible(props: &WidgetProps) -> bool {
    if attr_flag(
        props,
        &[
            "dismissible",
            "dismiss",
            "close",
            "closable",
            "data-dismissible",
            "data-dismiss",
            "data-close",
            "ondismiss",
            "on-dismiss",
        ],
    ) {
        return true;
    }
    props.attrs.keys().any(|key| {
        matches!(
            key.to_ascii_lowercase().as_str(),
            "ondismiss" | "on-dismiss" | "onclose" | "on-close"
        )
    }) || props.class_names.iter().any(|class| {
        let class = class.to_ascii_lowercase();
        class.contains("dismissible") || class.ends_with("--dismiss")
    })
}

pub(crate) fn action_menu_item_danger(props: &WidgetProps) -> bool {
    if matches!(props.button_kind, nana_ui_core::ButtonKind::Danger) {
        return true;
    }
    if attr_flag(props, &["danger", "data-danger"]) {
        return true;
    }
    if attr_value(props, &["intent", "data-intent", "data-variant"])
        .is_some_and(|value| value.eq_ignore_ascii_case("danger"))
    {
        return true;
    }
    props.class_names.iter().any(|class| {
        class
            .rsplit_once("--")
            .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("danger"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_dir_attr_maps_rtl_ltr_auto_fail_closed() {
        let mut props = WidgetProps::default();
        props.attrs.insert("dir".into(), "rtl".into());
        assert_eq!(html_dir_spec(&props), Some(nana_ui_core::DirSpec::Rtl));
        props.attrs.insert("dir".into(), "ltr".into());
        assert_eq!(html_dir_spec(&props), Some(nana_ui_core::DirSpec::Ltr));
        props.attrs.insert("dir".into(), "auto".into());
        assert!(html_dir_spec(&props).is_none());
        props.attrs.insert("DIR".into(), "RTL".into());
        props.attrs.remove("dir");
        assert_eq!(html_dir_spec(&props), Some(nana_ui_core::DirSpec::Rtl));
    }

    #[test]
    fn img_maps_to_box() {
        assert_eq!(
            resolve_kind_from_hints("img", None, None, None),
            Some(WidgetKind::Box)
        );
    }

    #[test]
    fn replaced_media_tags_map_kinds() {
        assert_eq!(
            resolve_kind_from_hints("canvas", None, None, None),
            Some(WidgetKind::Box)
        );
        assert_eq!(
            resolve_kind_from_hints("video", None, None, None),
            Some(WidgetKind::Video)
        );
        assert_eq!(
            resolve_kind_from_hints("iframe", None, None, None),
            Some(WidgetKind::Box)
        );
        // No host pipeline behind these yet: they stay layout boxes instead
        // of faking a player or external-content frame.
        for tag in ["iframe", "audio", "embed", "object"] {
            assert_eq!(
                resolve_kind_from_hints(tag, None, None, None),
                Some(WidgetKind::Box),
                "{tag} is an explicit L1 skip"
            );
        }
        for tag in ["source", "track", "area", "col", "colgroup"] {
            assert_eq!(
                resolve_kind_from_hints(tag, None, None, None),
                Some(WidgetKind::Box),
                "{tag} has no visual of its own"
            );
        }
    }

    #[test]
    fn html_structure_tags_map_to_layout_and_text() {
        for tag in ["figure", "hgroup", "picture", "datalist", "slot", "map"] {
            assert_eq!(
                resolve_kind_from_hints(tag, None, None, None),
                Some(WidgetKind::Column),
                "{tag} is a block container"
            );
        }
        for tag in ["figcaption", "address"] {
            assert_eq!(
                resolve_kind_from_hints(tag, None, None, None),
                Some(WidgetKind::Text),
                "{tag} is block text"
            );
        }
        // HTML-AAM: <menu> is equivalent to <ul>.
        assert_eq!(
            resolve_kind_from_hints("menu", None, None, None),
            Some(WidgetKind::List)
        );
        // An explicit menu role still wins over the tag.
        assert_eq!(
            resolve_kind_from_hints("div", None, Some("menu"), None),
            Some(WidgetKind::ContextMenu)
        );
    }

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
    #[cfg(feature = "graph-canvas")]
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
        assert_eq!(
            resolve_kind_from_hints("svg", Some("graph-canvas"), None, None),
            Some(WidgetKind::Column),
            "structural SVG must not become GraphCanvas"
        );
        assert_eq!(
            resolve_kind_from_hints("textPath", None, None, None),
            Some(WidgetKind::Box),
            "textPath stays a serializable SVG child, not a Text widget"
        );
    }

    #[test]
    fn maps_button_input_switch_and_flex_row_div() {
        assert_eq!(
            resolve_kind_from_hints("button", None, None, None),
            Some(WidgetKind::Button)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-button", None, None, None),
            None,
            "retired nana-button alias is not a Button or layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("dialog", None, None, None),
            Some(WidgetKind::Dialog)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-dialog", None, None, None),
            None,
            "retired nana-dialog alias is not a Dialog or layout box"
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
            Some(WidgetKind::Column),
            "class hints write LayoutStyle, not WidgetKind"
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
    fn video_uses_media_surface_and_audio_uses_control_box() {
        assert_eq!(
            resolve_kind_from_hints("img", None, None, None),
            Some(WidgetKind::Box)
        );
        assert_eq!(
            resolve_kind_from_hints("video", None, None, None),
            Some(WidgetKind::Video),
            "video projects to its existing HostTexture media surface"
        );
        assert_eq!(
            resolve_kind_from_hints("audio", None, None, None),
            Some(WidgetKind::Box),
            "audio is a control slot without a video frame"
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
            Some(WidgetKind::Dropdown)
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
    #[cfg(feature = "calendar")]
    #[cfg(feature = "charts")]
    #[cfg(feature = "rich-text")]
    #[cfg(feature = "graph-canvas")]
    #[cfg(feature = "image-viewer")]
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
            None,
            "nana-form is not a registered control or HTML layout tag"
        );
        assert_eq!(
            resolve_kind_from_hints("form", None, None, None),
            Some(WidgetKind::Column),
            "HTML form stays a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("search", None, None, None),
            Some(WidgetKind::Column),
            "HTML search stays a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("nana-search", None, None, None),
            None,
            "retired nana-search alias is not SearchDropdown or a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("search-dropdown", None, None, None),
            Some(WidgetKind::SearchDropdown)
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
            resolve_kind_from_hints("nana-level-meter", None, None, None),
            None,
            "retired nana-level-meter alias is not LevelMeter or a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("meter", None, None, None),
            Some(WidgetKind::LevelMeter)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-toast toast"), None, None),
            Some(WidgetKind::Toast)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-tooltip tooltip"), Some("tooltip"), None),
            Some(WidgetKind::Tooltip)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-action-menu", None, None, None),
            Some(WidgetKind::ActionMenu)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-action-menu-item"), Some("menuitem"), None),
            Some(WidgetKind::ActionMenuItem)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-xy-pad nana-xypad xy-pad"), None, None),
            Some(WidgetKind::XYPad)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-qr-code", None, None, None),
            Some(WidgetKind::QrCode)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-qr-code qr-code"), None, None),
            Some(WidgetKind::QrCode)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-command-palette", None, None, None),
            Some(WidgetKind::CommandPalette)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-hosted-textarea", None, None, None),
            Some(WidgetKind::Textarea)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("hosted-textarea"), None, None),
            Some(WidgetKind::Textarea)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-tree-view tree-view"), None, None),
            Some(WidgetKind::TreeView)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-calendar-heatmap", None, None, None),
            Some(WidgetKind::CalendarHeatmap)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-image-viewer", None, None, None),
            Some(WidgetKind::ImageViewer)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-native-markdown", None, None, None),
            Some(WidgetKind::NativeMarkdown)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-graph-canvas", None, None, None),
            Some(WidgetKind::GraphCanvas)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("graph-canvas graphcanvas"), None, None),
            Some(WidgetKind::GraphCanvas)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-workspace", None, None, None),
            Some(WidgetKind::Workspace)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-dock"), None, None),
            Some(WidgetKind::Dock)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-split-pane", None, None, None),
            Some(WidgetKind::SplitPane)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-app-shell", None, None, None),
            Some(WidgetKind::AppShell)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-settings-page", None, None, None),
            Some(WidgetKind::SettingsPage)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-icon-button", None, None, None),
            Some(WidgetKind::IconButton)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-number-input", None, None, None),
            None,
            "retired nana-number-input alias is not NumberInput or a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("input", None, None, Some("number")),
            Some(WidgetKind::NumberInput)
        );
        assert_eq!(
            resolve_kind_from_hints("input", None, None, Some("radio")),
            Some(WidgetKind::Radio)
        );
        assert_eq!(
            resolve_kind_from_hints("ul", None, None, None),
            Some(WidgetKind::List)
        );
        assert_eq!(
            resolve_kind_from_hints("ol", None, None, None),
            Some(WidgetKind::List)
        );
        assert_eq!(
            resolve_kind_from_hints("a", None, None, None),
            Some(WidgetKind::Button)
        );
        assert_eq!(
            resolve_kind_from_hints("details", None, None, None),
            Some(WidgetKind::SettingsCollapsibleCard)
        );
        assert_eq!(
            resolve_kind_from_hints("div", None, Some("radiogroup"), None),
            Some(WidgetKind::Segmented)
        );
        assert_eq!(
            resolve_kind_from_hints("hr", None, None, None),
            Some(WidgetKind::Divider)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-scroll-view", None, None, None),
            Some(WidgetKind::ScrollView)
        );
        assert_eq!(
            resolve_kind_from_hints("table", None, None, None),
            Some(WidgetKind::Table)
        );
        assert_eq!(
            resolve_kind_from_hints("tr", None, None, None),
            Some(WidgetKind::TableRow)
        );
        assert_eq!(
            resolve_kind_from_hints("td", None, None, None),
            Some(WidgetKind::TableCell)
        );
        assert_eq!(
            resolve_kind_from_hints("th", None, None, None),
            Some(WidgetKind::TableCell)
        );
        assert_eq!(
            resolve_kind_from_hints("thead", None, None, None),
            Some(WidgetKind::Column)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-table", None, None, None),
            None,
            "retired nana-table alias is not Table or a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("nana-desktop-shell", None, None, None),
            Some(WidgetKind::DesktopShell)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-app-title-bar", None, None, None),
            Some(WidgetKind::AppTitleBar)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-time-series-chart", None, None, None),
            Some(WidgetKind::TimeSeriesChart)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-settings-collapsible-card", None, None, None),
            None,
            "retired nana-settings-collapsible-card alias is not the disclosure or a layout box"
        );
        assert_eq!(
            resolve_kind_from_hints("nana-gpu", None, None, None),
            Some(WidgetKind::GpuTextureView)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-gpu-view", None, None, None),
            Some(WidgetKind::GpuView)
        );
        assert_eq!(
            resolve_kind_from_hints("nana-virtual-list", None, None, None),
            None,
            "virtual windows use nana-scroll-view, not a second tag"
        );
        assert_eq!(
            resolve_kind_from_hints(
                "section",
                Some("nana-settings-page settings-page"),
                None,
                None
            ),
            Some(WidgetKind::SettingsPage)
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
    fn dom_text_node_is_text_tag_not_l2_text_kind() {
        assert!(is_dom_text_node(WidgetKind::Text, "#text"));
        assert!(is_dom_text_node(WidgetKind::Text, "nana-text"));
        assert!(is_dom_text_node(WidgetKind::Text, "createText"));
        assert!(is_dom_text_node(WidgetKind::Text, ""));
        assert!(!is_dom_text_node(WidgetKind::Text, "span"));
        assert!(!is_dom_text_node(WidgetKind::Text, "p"));
        assert!(!is_dom_text_node(WidgetKind::Text, "label"));
        assert!(!is_dom_text_node(WidgetKind::Text, "h1"));
        assert!(!is_dom_text_node(WidgetKind::Column, ""));
    }

    #[test]
    fn checked_match_host_is_checkbox_radio_switch_not_overlay() {
        let mut checkbox = BTreeMap::new();
        checkbox.insert("type".into(), "checkbox".into());
        let mut radio = BTreeMap::new();
        radio.insert("type".into(), "radio".into());
        assert!(is_checked_match_host(
            WidgetKind::Checkbox,
            "input",
            &checkbox
        ));
        assert!(is_checked_match_host(WidgetKind::Input, "input", &radio));
        assert!(is_checked_match_host(WidgetKind::Radio, "input", &radio));
        assert!(is_checked_match_host(
            WidgetKind::Switch,
            "button",
            &BTreeMap::new()
        ));
        assert!(!is_checked_match_host(
            WidgetKind::Dialog,
            "dialog",
            &BTreeMap::new()
        ));
        assert!(!is_checked_match_host(
            WidgetKind::Input,
            "input",
            &BTreeMap::new()
        ));
    }

    #[test]
    fn level_meter_value_prefers_progress_then_number() {
        let mut props = WidgetProps::default();
        props.number = 0.25;
        assert!((level_meter_value(&props) - 0.25).abs() < f32::EPSILON);
        props.progress = 0.8;
        assert!((level_meter_value(&props) - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn unknown_tags_error_instead_of_becoming_layout_boxes() {
        assert_eq!(resolve_kind_from_hints("foo-bar", None, None, None), None);
        assert_eq!(
            resolve_kind_from_hints("html", None, None, None),
            Some(WidgetKind::Column)
        );
        assert_eq!(
            resolve_kind_from_hints("option", None, None, None),
            Some(WidgetKind::Column)
        );
        assert_eq!(
            resolve_host_tag_kind("nana-button", None, None, None, |_| true)
                .unwrap_err()
                .as_str(),
            "retired tag `nana-button`; use `button`"
        );
        assert_eq!(
            resolve_host_tag_kind("foo-bar", None, None, None, |_| false)
                .unwrap_err()
                .as_str(),
            "unknown tag: foo-bar"
        );
        assert_eq!(
            resolve_host_tag_kind("preview-card", None, None, None, |tag| tag
                == "preview-card"),
            Ok(WidgetKind::Column)
        );
    }
}
