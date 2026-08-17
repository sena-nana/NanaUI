//! nanavue → NanaUI (Iced) semantic message bridge.
//!
//! ## L2 边界
//! - 本模块是语义森林载体：`WidgetKind` / `WidgetProps` / `SemanticSnapshot`。
//! - **kind 解析**集中在 [`crate::widget_map::resolve_kind_from_hints`]（本文件
//!   `pub use` 转发）；勿在 bridge 内再维护第二份 class/role 表。
//! - Layout 声明解析属 L1（`css_map` / cascade）；本模块只存储与触发 rebuild。
//!
//! Vue Custom Renderer hostOps maintain a semantic widget tree. **Every visible
//! node** is meant to downlevel onto Nana layout primitives + base controls
//! (variants / composition), then draw as real `nana_ui` widgets.
//!
//! Vue "custom components" are combinations and variants of those foundations —
//! not a separate CPU paint channel. CustomContent has been removed.

use std::collections::{BTreeMap, HashMap, VecDeque};

use nana_ui_core::{
    AppearanceSettings, BackdropTarget, ButtonKind, CardKind, ControlSize, Icon,
    SwitchControlPosition, ThemeMode, WindowMaterialMode,
};

use crate::css_cascade::{
    MatchContext, MatchNode, StyleRule, collect_document_custom_properties_from_rules,
    parse_stylesheet, rebuild_layout_style,
};
use crate::css_map::{
    FlexDirection, GridTrack, LayoutStyle, LayoutStyleCss, LengthSpec, ParentBox,
};
use crate::layout_map::{apply_display_to_kind, default_layout_for_kind};
use crate::tree::NodeHandle;
pub use crate::widget_map::resolve_kind_from_hints;

/// Stable widget id — same numeric space as [`NodeHandle`].
pub type WidgetId = u64;

/// Nana layout primitives + base controls mirrored by nanavue / HTML downlevel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetKind {
    /// Vertical stack (default layout box for `div` / section / …).
    Column,
    /// Horizontal stack (`nana-row`, flex-row class, …).
    Row,
    /// Generic layout box (same view as Column; kept for diagnostics).
    Box,
    Text,
    Button,
    /// Compact selectable chip — Button Selected/Subtle variant.
    Chip,
    Input,
    Textarea,
    Checkbox,
    Switch,
    Select,
    Tabs,
    Segmented,
    Range,
    Card,
    ListItem,
    EmptyState,
    StatusBadge,
    ValidationMessage,
    LabeledValue,
    Progress,
    Spinner,
    FormField,
    InteractiveCard,
    Skeleton,
    LevelMeter,
    SidebarFrame,
    SidebarRow,
    SettingsRow,
    SettingsCard,
    Icon,
    /// Modal dialog → `nana_ui::Dialog` (open via `active` / `open` / `toggled`).
    Dialog,
    /// Side drawer → `nana_ui::Drawer` (`side=left|right`, open via `active` / `open`).
    Drawer,
    /// Anchored popover → `nana_ui::Popover`.
    Popover,
    /// Context menu → Runtime `ContextMenu` (`anchor-x` / `anchor-y`, search
    /// when ≥6 options or `search` class; nested via `parent/child` values).
    ContextMenu,
    /// Outlined notification → Runtime `Toast`.
    Toast,
    /// Compact label-only hover card → Runtime `Tooltip`.
    Tooltip,
    /// Trigger-bound action menu → Runtime `ActionMenu`.
    ActionMenu,
    /// Selectable menu row → Runtime `ActionMenuItem`.
    ActionMenuItem,
    /// Two-axis pad → Runtime `XYPad`.
    XYPad,
    /// Scanner-safe QR matrix → Runtime `QrCode` when modules are supplied.
    QrCode,
    /// Modal command list → Runtime `CommandPalette`.
    CommandPalette,
    /// Flattened disclosure tree → Runtime `TreeView`.
    TreeView,
    /// Contribution heatmap leaf → Runtime `CalendarHeatmap` (never SVG path-d).
    CalendarHeatmap,
    /// Host-texture image overlay → Runtime `ImageViewer`.
    ImageViewer,
    /// Parsed markdown leaf → Runtime `NativeMarkdown`.
    NativeMarkdown,
    /// Node/edge canvas → Runtime `GraphCanvas`.
    GraphCanvas,
    /// Workspace chrome → Runtime `Workspace` (region children become slots).
    Workspace,
    /// Dock chrome → Runtime `Dock` (children / `layout` / `root` become items).
    Dock,
    /// Split chrome → Runtime `SplitPane` (first two children are panes).
    SplitPane,
    /// App shell chrome → Runtime `AppShell` (title bar + body + overlay slots).
    AppShell,
    /// Settings content chrome → Runtime `SettingsPage` (header + scroll).
    SettingsPage,
}

impl WidgetKind {
    /// Parse an explicit `nana-*` / createWidget kind string.
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim().to_ascii_lowercase();
        let s = s.strip_prefix("nana-").unwrap_or(&s);
        Some(match s {
            "column" | "col" | "vstack" => Self::Column,
            "row" | "hstack" => Self::Row,
            "box" | "container" | "layout" => Self::Box,
            "text" | "label" => Self::Text,
            "button" | "btn" => Self::Button,
            "chip" => Self::Chip,
            "input" | "text-field" | "textfield" => Self::Input,
            "textarea" => Self::Textarea,
            "checkbox" | "check" => Self::Checkbox,
            "switch" | "toggle" => Self::Switch,
            "select" | "pick-list" | "picklist" | "dropdown" => Self::Select,
            "search" | "search-dropdown" | "searchdropdown" => Self::Select,
            "tabs" | "tab-list" | "tablist" => Self::Tabs,
            "segmented" | "segmented-control" => Self::Segmented,
            "range" | "range-field" | "slider" => Self::Range,
            "card" => Self::Card,
            "list-item" | "listitem" | "li" => Self::ListItem,
            "empty" | "empty-state" | "emptystate" => Self::EmptyState,
            "status" | "status-badge" | "statusbadge" => Self::StatusBadge,
            "validation" | "validation-message" | "validationmessage" => Self::ValidationMessage,
            "labeled-value" | "labeledvalue" => Self::LabeledValue,
            "progress" => Self::Progress,
            "spinner" | "loading" => Self::Spinner,
            "form-field" | "formfield" | "form" => Self::FormField,
            "interactive-card" | "interactivecard" => Self::InteractiveCard,
            "skeleton" => Self::Skeleton,
            "level-meter" | "levelmeter" | "level" => Self::LevelMeter,
            "sidebar-frame" | "sidebarframe" | "sidebar_frame" => Self::SidebarFrame,
            "sidebar-row" | "sidebarrow" | "sidebar_row" => Self::SidebarRow,
            "settings-row" | "settingsrow" => Self::SettingsRow,
            "settings-card" | "settingscard" => Self::SettingsCard,
            "icon" => Self::Icon,
            "dialog" | "modal" => Self::Dialog,
            "drawer" | "sheet" => Self::Drawer,
            "popover" => Self::Popover,
            "context-menu" | "contextmenu" => Self::ContextMenu,
            "toast" => Self::Toast,
            "tooltip" => Self::Tooltip,
            "action-menu" | "actionmenu" => Self::ActionMenu,
            "action-menu-item" | "actionmenuitem" => Self::ActionMenuItem,
            "xy-pad" | "xypad" | "xy_pad" => Self::XYPad,
            "qr-code" | "qr" | "qrcode" => Self::QrCode,
            "command-palette" | "commandpalette" => Self::CommandPalette,
            "tree-view" | "treeview" => Self::TreeView,
            "calendar" | "calendar-heatmap" => Self::CalendarHeatmap,
            "image-viewer" | "imageviewer" => Self::ImageViewer,
            "markdown" | "native-markdown" | "nativemarkdown" => Self::NativeMarkdown,
            "graph-canvas" | "graphcanvas" => Self::GraphCanvas,
            "workspace" => Self::Workspace,
            "dock" => Self::Dock,
            "split-pane" | "splitpane" => Self::SplitPane,
            "app-shell" | "appshell" => Self::AppShell,
            "settings-page" | "settingspage" => Self::SettingsPage,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Column => "column",
            Self::Row => "row",
            Self::Box => "box",
            Self::Text => "text",
            Self::Button => "button",
            Self::Chip => "chip",
            Self::Input => "input",
            Self::Textarea => "textarea",
            Self::Checkbox => "checkbox",
            Self::Switch => "switch",
            Self::Select => "select",
            Self::Tabs => "tabs",
            Self::Segmented => "segmented",
            Self::Range => "range",
            Self::Card => "card",
            Self::ListItem => "list-item",
            Self::EmptyState => "empty-state",
            Self::StatusBadge => "status-badge",
            Self::ValidationMessage => "validation-message",
            Self::LabeledValue => "labeled-value",
            Self::Progress => "progress",
            Self::Spinner => "spinner",
            Self::FormField => "form-field",
            Self::InteractiveCard => "interactive-card",
            Self::Skeleton => "skeleton",
            Self::LevelMeter => "level-meter",
            Self::SidebarFrame => "sidebar-frame",
            Self::SidebarRow => "sidebar-row",
            Self::SettingsRow => "settings-row",
            Self::SettingsCard => "settings-card",
            Self::Icon => "icon",
            Self::Dialog => "dialog",
            Self::Drawer => "drawer",
            Self::Popover => "popover",
            Self::ContextMenu => "context-menu",
            Self::Toast => "toast",
            Self::Tooltip => "tooltip",
            Self::ActionMenu => "action-menu",
            Self::ActionMenuItem => "action-menu-item",
            Self::XYPad => "xy-pad",
            Self::QrCode => "qr-code",
            Self::CommandPalette => "command-palette",
            Self::TreeView => "tree-view",
            Self::CalendarHeatmap => "calendar-heatmap",
            Self::ImageViewer => "image-viewer",
            Self::NativeMarkdown => "native-markdown",
            Self::GraphCanvas => "graph-canvas",
            Self::Workspace => "workspace",
            Self::Dock => "dock",
            Self::SplitPane => "split-pane",
            Self::AppShell => "app-shell",
            Self::SettingsPage => "settings-page",
        }
    }

    pub fn element_tag(self) -> &'static str {
        match self {
            Self::Column => "nana-column",
            Self::Row => "nana-row",
            Self::Box => "nana-box",
            Self::Text => "nana-text",
            Self::Button => "nana-button",
            Self::Chip => "nana-chip",
            Self::Input => "nana-input",
            Self::Textarea => "nana-textarea",
            Self::Checkbox => "nana-checkbox",
            Self::Switch => "nana-switch",
            Self::Select => "nana-select",
            Self::Tabs => "nana-tabs",
            Self::Segmented => "nana-segmented",
            Self::Range => "nana-range",
            Self::Card => "nana-card",
            Self::ListItem => "nana-list-item",
            Self::EmptyState => "nana-empty-state",
            Self::StatusBadge => "nana-status-badge",
            Self::ValidationMessage => "nana-validation-message",
            Self::LabeledValue => "nana-labeled-value",
            Self::Progress => "nana-progress",
            Self::Spinner => "nana-spinner",
            Self::FormField => "nana-form-field",
            Self::InteractiveCard => "nana-interactive-card",
            Self::Skeleton => "nana-skeleton",
            Self::LevelMeter => "nana-level-meter",
            Self::SidebarFrame => "nana-sidebar-frame",
            Self::SidebarRow => "nana-sidebar-row",
            Self::SettingsRow => "nana-settings-row",
            Self::SettingsCard => "nana-settings-card",
            Self::Icon => "nana-icon",
            Self::Dialog => "nana-dialog",
            Self::Drawer => "nana-drawer",
            Self::Popover => "nana-popover",
            Self::ContextMenu => "nana-context-menu",
            Self::Toast => "nana-toast",
            Self::Tooltip => "nana-tooltip",
            Self::ActionMenu => "nana-action-menu",
            Self::ActionMenuItem => "nana-action-menu-item",
            Self::XYPad => "nana-xy-pad",
            Self::QrCode => "nana-qr-code",
            Self::CommandPalette => "nana-command-palette",
            Self::TreeView => "nana-tree-view",
            Self::CalendarHeatmap => "nana-calendar",
            Self::ImageViewer => "nana-image-viewer",
            Self::NativeMarkdown => "nana-markdown",
            Self::GraphCanvas => "nana-graph-canvas",
            Self::Workspace => "nana-workspace",
            Self::Dock => "nana-dock",
            Self::SplitPane => "nana-split-pane",
            Self::AppShell => "nana-app-shell",
            Self::SettingsPage => "nana-settings-page",
        }
    }

    pub fn is_layout(self) -> bool {
        matches!(
            self,
            Self::Column
                | Self::Row
                | Self::Box
                | Self::SidebarFrame
                | Self::Card
                | Self::SettingsCard
        )
    }

    /// Dialog / Drawer / Popover / ContextMenu — open via `active` / `open` / `toggled`.
    pub fn is_overlay(self) -> bool {
        matches!(
            self,
            Self::Dialog
                | Self::Drawer
                | Self::Popover
                | Self::ContextMenu
                | Self::Toast
                | Self::Tooltip
                | Self::ActionMenu
                | Self::CommandPalette
                | Self::ImageViewer
        )
    }
}

/// One option for Tabs / Segmented / Select.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOptionProp {
    pub value: String,
    pub label: String,
    pub disabled: bool,
}

/// Props mirrored from Vue Nana* wrappers and HTML downlevel attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetProps {
    /// Registered Rust/Iced component name without the `nana-` prefix.
    /// Built-in widgets leave this empty and continue through [`WidgetKind`].
    pub native_component: Option<String>,
    /// Lossless Vue props for registered native components. Layout/class/event
    /// props still flow through the normal semantic fields as well.
    pub native_props: BTreeMap<String, nana_js_engine::HostValue>,
    /// Context-menu / popover anchor X (logical px).
    pub anchor_x: f32,
    /// Context-menu / popover anchor Y (logical px).
    pub anchor_y: f32,
    pub label: String,
    pub hint: String,
    pub placeholder: String,
    pub value: String,
    pub button_kind: ButtonKind,
    pub card_kind: CardKind,
    pub size: ControlSize,
    pub control_position: SwitchControlPosition,
    pub auto_height: bool,
    pub disabled: bool,
    pub loading: bool,
    pub read_only: bool,
    pub secure: bool,
    pub toggled: bool,
    pub active: bool,
    pub muted: bool,
    pub invalid: bool,
    pub fill: bool,
    pub agent_id: String,
    /// Workspace region contract from `data-region` / `region`
    /// (`global-navigation`, `section-navigation`, `inspector`, …).
    pub region: String,
    pub class_names: Vec<String>,
    /// Element tag from createElement (`div`, `section`, …) for selector matching.
    pub element_tag: String,
    /// HTML `id` attribute (for `#id` selectors).
    pub element_id: String,
    /// Attribute map for `[attr]` / `[attr=value]` matching (incl. Vue scope ids).
    pub attrs: BTreeMap<String, String>,
    /// Raw inline `style` declaration text (cascade layer above stylesheet).
    pub inline_style: String,
    /// Layout props applied via attributes (`gap` / `width` / …) as CSS text.
    pub prop_style: String,
    pub role: String,
    /// Drawer / sheet side hint: `left` | `right` (empty → default right).
    pub side: String,
    pub options: Vec<SelectOptionProp>,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub number: f32,
    pub unit: String,
    pub progress: f32,
    pub progress_max: f32,
    /// CSS / class 布局意图（gap、padding、width%、flex 方向…）。
    pub layout: LayoutStyle,
    /// 最近已知的包含块宽度（父 content box）。供 `style`/`gap` 等 `%` 解析；
    /// margin/padding `%` 仍存 [`LengthSpec`]，布局时再用同一基兑现。
    pub containing_block_width: Option<f32>,
    /// 最近已知的包含块高度（父 content box）。
    pub containing_block_height: Option<f32>,
}

impl Default for WidgetProps {
    fn default() -> Self {
        Self {
            native_component: None,
            native_props: BTreeMap::new(),
            anchor_x: 96.0,
            anchor_y: 96.0,
            label: String::new(),
            hint: String::new(),
            placeholder: String::new(),
            value: String::new(),
            button_kind: ButtonKind::Ghost,
            card_kind: CardKind::Surface,
            size: ControlSize::Medium,
            control_position: SwitchControlPosition::End,
            auto_height: false,
            disabled: false,
            loading: false,
            read_only: false,
            secure: false,
            toggled: false,
            active: false,
            muted: false,
            invalid: false,
            fill: false,
            agent_id: String::new(),
            region: String::new(),
            class_names: Vec::new(),
            element_tag: String::new(),
            element_id: String::new(),
            attrs: BTreeMap::new(),
            inline_style: String::new(),
            prop_style: String::new(),
            role: String::new(),
            side: String::new(),
            options: Vec::new(),
            min: 0.0,
            max: 100.0,
            step: 1.0,
            number: 0.0,
            unit: String::new(),
            progress: 0.0,
            progress_max: 1.0,
            layout: LayoutStyle::default(),
            containing_block_width: None,
            containing_block_height: None,
        }
    }
}

impl WidgetProps {
    pub fn from_map(map: &std::collections::BTreeMap<String, nana_js_engine::HostValue>) -> Self {
        let mut props = Self::default();
        for (key, value) in map {
            props.apply_prop(key, value);
        }
        props
    }

    #[cfg(feature = "scene-view")]
    pub(crate) fn attach_native_component(
        &mut self,
        name: String,
        map: &BTreeMap<String, nana_js_engine::HostValue>,
    ) {
        self.native_component = Some(name);
        self.native_props.clear();
        for (key, value) in map {
            let key = normalize_prop_key(key);
            if !is_framework_native_prop(&key)
                && !matches!(
                    value,
                    nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined
                )
            {
                self.native_props.insert(key, value.clone());
            }
        }
    }

    pub fn apply_prop(&mut self, key: &str, value: &nana_js_engine::HostValue) {
        let key = normalize_prop_key(key);
        if self.native_component.is_some() && !is_framework_native_prop(&key) {
            if matches!(
                value,
                nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined
            ) {
                self.native_props.remove(&key);
            } else {
                self.native_props.insert(key.clone(), value.clone());
            }
        }
        match key.as_str() {
            "data" | "nodes" | "edges" | "model" | "source" | "markdown" | "tree" | "items"
            | "viewport" | "selection" | "layout" | "root" | "axis" | "size" | "default-size"
            | "min" | "max" | "settings" | "tab" | "hide-header" => {
                self.persist_native_payload(&key, value);
            }
            "options" => {
                self.persist_native_payload(&key, value);
            }
            "mermaid-renderer" | "math-renderer" => {
                self.persist_native_payload(&key, value);
            }
            _ => {}
        }
        match key.as_str() {
            "label" | "text" | "title" => self.label = host_string(value),
            "aria-label" | "arialabel" => {
                let s = host_string(value);
                if !s.is_empty() {
                    // Prefer aria-label as accessible caption for icon-only buttons.
                    if self.label.is_empty() {
                        self.label = s.clone();
                    }
                    if self.hint.is_empty() {
                        self.hint = s;
                    }
                }
            }
            "hint" | "message" | "description" => self.hint = host_string(value),
            "placeholder" => self.placeholder = host_string(value),
            // Prefer icon/icon-name; do not steal HTML `name=` on inputs.
            "icon" | "icon-name" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.value = s.clone();
                    if self.label.is_empty() {
                        self.label = s;
                    }
                }
            }
            "kind" | "button-kind" => {
                let value = host_string(value);
                if let Some(k) = parse_button_kind(&value) {
                    self.button_kind = k;
                }
                if let Some(k) = parse_card_kind(&value) {
                    self.card_kind = k;
                }
            }
            "card-kind" | "cardkind" => {
                if let Some(k) = parse_card_kind(&host_string(value)) {
                    self.card_kind = k;
                }
            }
            "data-variant" => {
                let s = host_string(value);
                self.attrs.insert("data-variant".into(), s.clone());
                if let Some(k) = parse_button_kind(&s) {
                    self.button_kind = k;
                }
            }
            "size" => {
                if let Some(s) = parse_control_size(&host_string(value)) {
                    self.size = s;
                }
            }
            "control-position" | "controlposition" => {
                self.control_position = match host_string(value).to_ascii_lowercase().as_str() {
                    "start" | "left" => SwitchControlPosition::Start,
                    _ => SwitchControlPosition::End,
                };
            }
            "auto-height" | "autoheight" => self.auto_height = host_truthy(value),
            "disabled" => self.disabled = host_truthy(value),
            "loading" => self.loading = host_truthy(value),
            "readonly" | "read-only" => self.read_only = host_truthy(value),
            "toggled" | "model-value" | "checked" => {
                if value.as_bool().is_some() || matches!(value, nana_js_engine::HostValue::Bool(_))
                {
                    self.toggled = host_truthy(value);
                } else if let nana_js_engine::HostValue::Number(n) = value {
                    self.toggled = *n != 0.0;
                    self.number = *n as f32;
                    self.value = host_string(value);
                } else {
                    let s = host_string(value);
                    if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") {
                        self.toggled = host_truthy(value);
                    } else if !s.is_empty() {
                        self.value = s.clone();
                        if let Ok(n) = s.parse::<f32>() {
                            self.number = n;
                        }
                    }
                }
            }
            "value" => {
                if matches!(
                    value,
                    nana_js_engine::HostValue::Array(_) | nana_js_engine::HostValue::Object(_)
                ) {
                    self.persist_native_payload("value", value);
                }
                if value.as_bool().is_some() {
                    self.toggled = host_truthy(value);
                } else {
                    let s = host_string(value);
                    self.value = s.clone();
                    if let Ok(n) = s.parse::<f32>() {
                        self.number = n;
                        self.progress = n;
                    }
                    if self.label.is_empty() && !s.is_empty() {
                        // Keep label for leaf text/buttons when only value is set.
                    }
                }
            }
            "open" => {
                let open = host_truthy(value);
                self.active = open;
                self.toggled = open;
            }
            "aria-expanded" => {
                let s = host_string(value);
                self.attrs.insert("aria-expanded".into(), s.clone());
                if s.eq_ignore_ascii_case("true") {
                    self.active = true;
                    self.toggled = true;
                } else if s.eq_ignore_ascii_case("false") {
                    self.active = false;
                    self.toggled = false;
                }
            }
            "aria-modal" => {
                let s = host_string(value);
                if matches!(value, nana_js_engine::HostValue::Bool(true)) || s.is_empty() {
                    self.attrs.insert("aria-modal".into(), String::new());
                } else {
                    self.attrs.insert("aria-modal".into(), s);
                }
            }
            "anchor-x" | "data-anchor-x" => self.anchor_x = host_f32(value, self.anchor_x),
            "anchor-y" | "data-anchor-y" => self.anchor_y = host_f32(value, self.anchor_y),
            "active" | "selected" | "aria-selected" | "aria-pressed" => {
                self.active = host_truthy(value);
                if self.active {
                    self.button_kind = ButtonKind::Selected;
                }
            }
            "muted" => self.muted = host_truthy(value),
            "invalid" => self.invalid = host_truthy(value),
            "danger" => {
                if host_truthy(value) {
                    self.button_kind = ButtonKind::Danger;
                    self.attrs.insert("danger".into(), String::new());
                } else {
                    self.attrs.remove("danger");
                }
            }
            "dismissible" | "closable" => {
                if host_truthy(value) {
                    self.attrs.insert(key.clone(), String::new());
                } else {
                    self.attrs.remove(&key);
                }
            }
            "ondismiss" | "on-dismiss" => {
                self.attrs.insert("ondismiss".into(), String::new());
            }
            "x" => {
                self.number = host_f32(value, self.number);
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("x");
                } else {
                    self.attrs.insert("x".into(), s);
                }
            }
            "y" => {
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("y");
                } else {
                    self.attrs.insert("y".into(), s);
                }
            }
            "x-min" | "xmin" | "x-max" | "xmax" | "y-min" | "ymin" | "y-max" | "ymax" => {
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove(&key);
                } else {
                    self.attrs.insert(key, s);
                }
            }
            "modules" => {
                let encoded = encode_qr_modules_attr(value);
                if encoded.is_empty() {
                    self.attrs.remove("modules");
                } else {
                    self.attrs.insert("modules".into(), encoded);
                }
            }
            "payload" => {
                let s = host_string(value);
                self.attrs.insert("payload".into(), s.clone());
                if self.value.is_empty() {
                    self.value = s;
                }
            }
            "src" | "data-src" => {
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("src");
                    if key == "data-src" {
                        self.attrs.remove("data-src");
                    }
                    self.native_props.remove("src");
                } else {
                    self.attrs.insert("src".into(), s.clone());
                    if key == "data-src" {
                        self.attrs.insert("data-src".into(), s.clone());
                    }
                    self.persist_native_payload("src", value);
                }
            }
            "module-width" | "modules-width" => {
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("module-width");
                } else {
                    self.attrs.insert("module-width".into(), s);
                }
            }
            "tone" | "data-tone" => {
                let s = host_string(value);
                self.attrs.insert("tone".into(), s.clone());
                if key == "data-tone" {
                    self.attrs.insert("data-tone".into(), s);
                }
            }
            "mermaid-renderer" | "data-mermaid-renderer" => {
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("mermaid-renderer");
                    if key == "data-mermaid-renderer" {
                        self.attrs.remove("data-mermaid-renderer");
                    }
                } else {
                    self.attrs.insert("mermaid-renderer".into(), s.clone());
                    if key == "data-mermaid-renderer" {
                        self.attrs.insert("data-mermaid-renderer".into(), s);
                    }
                }
            }
            "math-renderer" | "data-math-renderer" => {
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("math-renderer");
                    if key == "data-math-renderer" {
                        self.attrs.remove("data-math-renderer");
                    }
                } else {
                    self.attrs.insert("math-renderer".into(), s.clone());
                    if key == "data-math-renderer" {
                        self.attrs.insert("data-math-renderer".into(), s);
                    }
                }
            }
            "intent" | "data-intent" => {
                let s = host_string(value);
                self.attrs.insert("intent".into(), s.clone());
                if key == "data-intent" {
                    self.attrs.insert("data-intent".into(), s);
                }
            }
            "fill" => {
                // SVG `fill="…color…"` vs flex `fill` truthy flag.
                let s = host_string(value);
                // Keep raw paint for Lucide SVG serialization (`none` / `currentColor`).
                if !s.is_empty()
                    && !s.eq_ignore_ascii_case("true")
                    && !s.eq_ignore_ascii_case("false")
                    && value.as_bool().is_none()
                {
                    self.attrs.insert("fill".into(), s.clone());
                }
                if let Some(c) = crate::css_map::resolve_paint_color(&s) {
                    self.layout.background = Some(c);
                } else if s.is_empty()
                    || s.eq_ignore_ascii_case("true")
                    || s.eq_ignore_ascii_case("false")
                    || value.as_bool().is_some()
                    || matches!(value, nana_js_engine::HostValue::Number(_))
                {
                    self.fill = host_truthy(value);
                }
            }
            "stroke" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.attrs.insert("stroke".into(), s.clone());
                }
                if let Some(c) = crate::css_map::resolve_paint_color(&s) {
                    self.layout.border_color = Some(c);
                    if self.layout.border_width.is_none() {
                        self.layout.border_width = Some(8.0);
                    }
                }
            }
            "stroke-dasharray" => {
                // Keep geometry in attrs for SVG rebuild (pie rings via pathLength).
                // Do not overwrite `value` — that leaked dash strings into captions.
                let s = host_string(value);
                if !s.is_empty() {
                    self.attrs.insert("stroke-dasharray".into(), s);
                }
            }
            "stroke-dashoffset" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.attrs.insert("stroke-dashoffset".into(), s);
                }
            }
            "d" => {
                // Always keep geometry in attrs for SVG rebuild. Lucide roots keep
                // glyph name in `value`; writing `d` there leaked paths into labels.
                let s = host_string(value);
                if s.is_empty() {
                    return;
                }
                self.attrs.insert("d".into(), s.clone());
                let lucide = self
                    .class_names
                    .iter()
                    .any(|c| c == "lucide" || c.starts_with("lucide-"));
                if !lucide {
                    self.value = s;
                }
            }
            "agent-id" | "data-agent-id" => {
                let s = host_string(value);
                self.agent_id = s.clone();
                self.attrs.insert("data-agent-id".into(), s);
            }
            // Workspace shell region contract (`RegionId` string form). Distinct
            // from element `id` / `data-region-id` (selector matching only).
            // Lilia / @lilia/ui emit `data-region-role` (role=resources|primary|…);
            // nanavue also accepts `region` / `data-region` / `region-role`.
            "region" | "data-region" | "data-region-role" | "region-role" => {
                let s = host_string(value);
                self.region = s.clone();
                self.attrs.insert("data-region".into(), s.clone());
                if key == "data-region-role" || key == "region-role" {
                    self.attrs.insert("data-region-role".into(), s);
                }
            }
            // Slot contract for NanaSidebarFrame children (top / body / footer).
            "data-slot" => {
                let slot = host_string(value);
                self.attrs.insert("data-slot".into(), slot.clone());
                let hint = match slot.as_str() {
                    "sidebar-top" => Some("nana-sidebar-frame__top"),
                    "sidebar-body" => Some("nana-sidebar-frame__body"),
                    "sidebar-footer" => Some("nana-sidebar-frame__footer"),
                    _ => None,
                };
                if let Some(class) = hint
                    && !self.class_names.iter().any(|c| c == class)
                {
                    self.class_names.push(class.to_string());
                }
            }
            "id" | "data-region-id" => {
                let id = host_string(value);
                self.element_id = id.clone();
                self.attrs.insert("id".into(), id.clone());
                if key == "data-region-id" {
                    self.attrs.insert("data-region-id".into(), id.clone());
                }
            }
            "class" | "classname" => {
                self.class_names = host_string(value)
                    .split_whitespace()
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
                    .collect();
                if self
                    .class_names
                    .iter()
                    .any(|c| c == "is-active" || c == "is-selected")
                {
                    self.active = true;
                }
                // Seed Lucide glyph name onto Icon props when class arrives after create.
                if self.value.is_empty()
                    && let Some(glyph) = self.class_names.iter().find_map(|c| {
                        let raw = c
                            .strip_prefix("lucide-")
                            .unwrap_or(c.as_str())
                            .strip_suffix("-icon")
                            .unwrap_or(c.strip_prefix("lucide-").unwrap_or(c.as_str()));
                        Icon::parse_name(c).map(|_| raw.to_string())
                    })
                {
                    self.value = glyph;
                }
                // Layout cascade rebuilt by MessageBridge after patch/register.
            }
            "aria-current" => {
                let s = host_string(value);
                if !s.is_empty() && !s.eq_ignore_ascii_case("false") {
                    self.active = true;
                }
            }
            "role" => {
                self.role = host_string(value);
                self.attrs.insert("role".into(), self.role.clone());
            }
            "side" | "drawer-side" | "placement" => self.side = host_string(value),
            "axis" => {
                let axis = host_string(value);
                if !axis.is_empty() {
                    self.attrs.insert("axis".into(), axis);
                } else {
                    self.attrs.remove("axis");
                }
            }
            "options" => self.options = parse_options(value),
            "min" | "aria-valuemin" => self.min = host_f32(value, self.min),
            "max" | "aria-valuemax" => self.max = host_f32(value, self.max),
            "step" => self.step = host_f32(value, self.step),
            "unit" => self.unit = host_string(value),
            "progress" | "aria-valuenow" => {
                self.progress = host_f32(value, self.progress);
                self.number = self.progress;
            }
            "progress-max" => self.progress_max = host_f32(value, self.progress_max),
            "type" => {
                // input type=checkbox upgrades handled by resolve layer.
                let t = host_string(value).to_ascii_lowercase();
                self.secure = t == "password";
                if t == "checkbox" {
                    self.toggled = self.toggled || host_truthy(value);
                }
            }
            "style" => {
                self.inline_style = host_style_to_css_text(value);
                // Layout cascade rebuilt by MessageBridge after patch/register.
            }
            "gap" => {
                if let nana_js_engine::HostValue::Number(n) = value {
                    let px = (*n as f32).max(0.0);
                    self.record_prop_style("gap", &format!("{px}px"));
                } else {
                    let s = host_string(value);
                    if !s.is_empty() {
                        self.record_prop_style("gap", &s);
                    }
                }
            }
            "padding" => {
                if let nana_js_engine::HostValue::Number(n) = value {
                    self.record_prop_style("padding", &format!("{}px", (*n as f32).max(0.0)));
                } else {
                    let s = host_string(value);
                    if !s.is_empty() {
                        self.record_prop_style("padding", &s);
                    }
                }
            }
            "width" => {
                if let nana_js_engine::HostValue::Number(n) = value {
                    self.record_prop_style("width", &format!("{}px", *n as f32));
                } else {
                    let s = host_string(value);
                    if !s.is_empty() {
                        self.record_prop_style("width", &s);
                    }
                }
            }
            "height" => {
                if let nana_js_engine::HostValue::Number(n) = value {
                    self.record_prop_style("height", &format!("{}px", *n as f32));
                } else {
                    let s = host_string(value);
                    if !s.is_empty() {
                        self.record_prop_style("height", &s);
                    }
                }
            }
            "flex-direction" | "flexdirection" => {
                let d = host_string(value).to_ascii_lowercase();
                if !d.is_empty() {
                    self.record_prop_style("flex-direction", &d);
                }
            }
            "justify-content" | "justifycontent" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.record_prop_style("justify-content", &s);
                }
            }
            "flex" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.record_prop_style("flex", &s);
                }
            }
            "flex-grow" | "flexgrow" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.record_prop_style("flex-grow", &s);
                }
            }
            "min-width" | "minwidth" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.record_prop_style("min-width", &s);
                }
            }
            "overflow" | "overflow-y" | "overflowy" => {
                let css_key = if key.contains('y') || key.ends_with("overflow-y") {
                    "overflow-y"
                } else {
                    "overflow"
                };
                let s = host_string(value);
                if !s.is_empty() {
                    self.record_prop_style(css_key, &s);
                }
            }
            "grid-template-columns" | "gridtemplatecolumns" => {
                let s = host_string(value);
                if !s.is_empty() {
                    self.record_prop_style("grid-template-columns", &s);
                }
            }
            "hidden" => {
                self.layout.hidden = host_truthy(value);
                if host_truthy(value) {
                    self.attrs.insert("hidden".into(), String::new());
                } else {
                    self.attrs.remove("hidden");
                }
            }
            "multiple" => {
                if host_truthy(value) {
                    self.attrs.insert("multiple".into(), String::new());
                } else {
                    self.attrs.remove("multiple");
                }
            }
            other => {
                // Persist data-* / aria-* / SVG attrs for selectors & Lucide.
                if other.starts_with("data-")
                    || other.starts_with("aria-")
                    || other.starts_with("xlink:")
                    || other.starts_with("xml:")
                    || is_common_svg_attr(other)
                {
                    let s = host_string(value);
                    if s.is_empty()
                        && matches!(
                            value,
                            nana_js_engine::HostValue::Null
                                | nana_js_engine::HostValue::Undefined
                                | nana_js_engine::HostValue::Bool(false)
                        )
                    {
                        self.attrs.remove(other);
                    } else if matches!(value, nana_js_engine::HostValue::Bool(true)) {
                        self.attrs.insert(other.to_string(), String::new());
                    } else {
                        self.attrs.insert(other.to_string(), s);
                    }
                }
            }
        }
    }

    fn persist_native_payload(&mut self, key: &str, value: &nana_js_engine::HostValue) {
        if matches!(
            value,
            nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined
        ) {
            self.native_props.remove(key);
        } else {
            self.native_props.insert(key.to_string(), value.clone());
        }
    }

    fn record_prop_style(&mut self, property: &str, value: &str) {
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        if property.is_empty() || value.is_empty() {
            return;
        }
        let mut kept = Vec::new();
        for decl in self.prop_style.split(';') {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            let Some((k, _)) = decl.split_once(':') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case(&property) {
                continue;
            }
            kept.push(decl.to_string());
        }
        kept.push(format!("{property}: {value}"));
        self.prop_style = kept.join("; ");
    }

    pub fn display_label(&self) -> &str {
        if self.label == "[object Object]" {
            return "";
        }
        if !self.label.is_empty() {
            &self.label
        } else {
            &self.value
        }
    }
}

/// User / host action crossing the Vue ↔ Iced boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeEvent {
    Press {
        id: WidgetId,
    },
    Toggle {
        id: WidgetId,
        value: bool,
    },
    Select {
        id: WidgetId,
    },
    SelectValue {
        id: WidgetId,
        value: String,
    },
    Input {
        id: WidgetId,
        value: String,
    },
    Change {
        id: WidgetId,
        value: f64,
    },
    /// Host scroll viewport changed. This updates retained runtime state only;
    /// it is not a Vue DOM event.
    Scroll {
        id: WidgetId,
        offset: nana_ui_runtime::ScrollOffset,
        metrics: nana_ui_runtime::ScrollMetrics,
    },
    /// Event emitted by a Rust/Iced component registered into the Vue tree.
    Native {
        id: WidgetId,
        name: String,
        payload: nana_js_engine::HostValue,
    },
    /// Context-menu search query.
    #[cfg(feature = "scene-view")]
    MenuSearch {
        id: WidgetId,
        query: String,
    },
    /// Context-menu open submenu path (host-owned [`crate::MenuStore`]).
    #[cfg(feature = "scene-view")]
    MenuPath {
        id: WidgetId,
        path: Vec<usize>,
    },
}

impl BridgeEvent {
    pub fn widget_id(&self) -> WidgetId {
        match self {
            Self::Press { id }
            | Self::Toggle { id, .. }
            | Self::Select { id }
            | Self::SelectValue { id, .. }
            | Self::Input { id, .. }
            | Self::Change { id, .. }
            | Self::Scroll { id, .. }
            | Self::Native { id, .. } => *id,
            #[cfg(feature = "scene-view")]
            Self::MenuSearch { id, .. } | Self::MenuPath { id, .. } => *id,
        }
    }

    /// JS event name for `__nanaFireEvent`.
    pub fn js_event_name(&self) -> &str {
        match self {
            Self::Press { .. } => "press",
            Self::Toggle { .. } => "change",
            Self::Select { .. } => "select",
            Self::SelectValue { .. } => "select",
            Self::Input { .. } => "input",
            Self::Change { .. } => "change",
            Self::Scroll { .. } => "scroll",
            Self::Native { name, .. } => name,
            #[cfg(feature = "scene-view")]
            Self::MenuSearch { .. } => "input",
            #[cfg(feature = "scene-view")]
            Self::MenuPath { .. } => "press",
        }
    }
}

fn is_framework_native_prop(key: &str) -> bool {
    matches!(
        key,
        "class"
            | "classname"
            | "style"
            | "id"
            | "role"
            | "hidden"
            | "disabled"
            | "tabindex"
            | "ref"
            | "ref-key"
            | "ref-for"
    ) || key.starts_with("aria-")
        || key.starts_with("data-")
        || key.starts_with("on")
}

/// Flat snapshot for Iced `view` (pre-order under each root).
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSnapshot {
    pub revision: u64,
    pub theme: ThemeMode,
    /// Appearance backdrop fields synced from L1 document dataset/style.
    pub appearance: AppearanceSettings,
    pub roots: Vec<WidgetId>,
    pub widgets: Vec<SemanticWidget>,
}

impl SemanticSnapshot {
    pub fn get(&self, id: WidgetId) -> Option<&SemanticWidget> {
        self.widgets.iter().find(|w| w.id == id)
    }

    /// Keep `seeds` and their descendants; re-root at the seeds.
    pub fn subtree_from(&self, seeds: impl IntoIterator<Item = WidgetId>) -> Self {
        use std::collections::{BTreeSet, HashMap, VecDeque};

        let by_id: HashMap<_, _> = self
            .widgets
            .iter()
            .map(|widget| (widget.id, widget))
            .collect();
        let mut keep = BTreeSet::new();
        let mut queue: VecDeque<WidgetId> = seeds.into_iter().collect();
        let roots: Vec<WidgetId> = queue.iter().copied().collect();
        let root_ids: BTreeSet<WidgetId> = roots.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            if !keep.insert(id) {
                continue;
            }
            if let Some(widget) = by_id.get(&id) {
                for &child in &widget.children {
                    queue.push_back(child);
                }
            }
        }
        let widgets: Vec<SemanticWidget> = self
            .widgets
            .iter()
            .filter(|w| keep.contains(&w.id))
            .map(|w| {
                let mut cloned = w.clone();
                cloned.children.retain(|c| keep.contains(c));
                if root_ids.contains(&cloned.id)
                    || cloned.parent.is_some_and(|p| !keep.contains(&p))
                {
                    cloned.parent = None;
                }
                cloned
            })
            .collect();
        Self {
            revision: self.revision,
            theme: self.theme,
            appearance: self.appearance,
            roots: roots
                .into_iter()
                .filter(|id| keep.contains(id) && by_id.contains_key(id))
                .collect(),
            widgets,
        }
    }

    /// Mutually exclusive DesktopShell region projections of this forest.
    ///
    /// Contract:
    /// - each widget id appears in **at most one** of the returned views
    /// - region membership is by **explicit region tags** (`data-region` /
    ///   `agent-id` / `role` / class), never by control-kind harvest (e.g.
    ///   every `SidebarFrame`)
    /// - ownership uses the **nearest region-tag ancestor** (including self):
    ///   walk up until a Navigation or Inspector marker is found; that marker
    ///   owns the node. On a dual-tagged node, **Inspector wins** (more
    ///   specific panel) so a nav ancestor does not re-harvest nested
    ///   inspector forests
    /// - region-owned widgets are **claimed** out of primary even when a seed
    ///   cap truncates the Navigation / Inspector projection — truncated
    ///   tagged content is omitted, not left in primary
    /// - **hollow ancestors** of claimed subtrees (outer shells left with only
    ///   claimed children and/or layout chrome such as resize handles) are
    ///   also removed from primary so nested tags cannot leave an empty
    ///   fixed-width track beside DesktopShell Navigation
    /// - after claimed children leave a parent, **stale multi-track grids**
    ///   (`grid-template-columns` / rows with more tracks than remaining
    ///   children) collapse to equal `1fr` tracks so a lone primary column is
    ///   not squeezed into the former sidebar track (e.g. 220px)
    /// - untagged widgets remain in [`SemanticRegionViews::primary`]
    ///
    /// Hosts must paint each view at most once. Painting the full snapshot in
    /// Primary **and** a region projection is a double-view bug. Exclusivity is
    /// enforced at build time (not only via [`SemanticRegionViews::overlapping_ids`]).
    pub fn region_views(&self) -> SemanticRegionViews {
        self.region_views_limited(usize::MAX, usize::MAX)
    }

    /// Like [`Self::region_views`] with seed caps for Navigation / Inspector.
    ///
    /// Caps only shrink the projected Navigation / Inspector forests. All
    /// region-owned widgets (nearest-tag rule) are still excluded from primary.
    pub fn region_views_limited(&self, nav_limit: usize, insp_limit: usize) -> SemanticRegionViews {
        use std::collections::BTreeSet;

        let index = RegionProjectionIndex::new(self);
        let mut claimed: BTreeSet<_> = index.owners.keys().copied().collect();
        // Nested region tags (e.g. NanaSidebarFrame inside an outer start panel)
        // must not leave a fixed-width empty shell track in Primary. Claim hollow
        // ancestors whose remaining children are only claimed nodes or layout
        // chrome (resize handles / separators). Hollow shells are omitted from
        // Primary; they are not re-projected into Navigation / Inspector.
        self.claim_hollow_region_ancestors(&index, &mut claimed);
        let navigation =
            self.exclusive_tagged_region_slice(&index, SemanticRegionTag::Navigation, nav_limit);
        let inspector =
            self.exclusive_tagged_region_slice(&index, SemanticRegionTag::Inspector, insp_limit);
        let primary = self.excluding_ids(&claimed);
        SemanticRegionViews {
            primary,
            navigation,
            inspector,
        }
    }

    /// Expand `claimed` to cover empty outer shells left after nested region lifts.
    fn claim_hollow_region_ancestors(
        &self,
        index: &RegionProjectionIndex<'_>,
        claimed: &mut std::collections::BTreeSet<WidgetId>,
    ) {
        use std::collections::{HashSet, VecDeque};

        if claimed.is_empty() {
            return;
        }
        let mut queue = VecDeque::new();
        let mut queued = HashSet::new();
        for id in claimed.iter().copied() {
            if let Some(parent) = index.get(id).and_then(|widget| widget.parent)
                && queued.insert(parent)
            {
                queue.push_back(parent);
            }
        }
        while let Some(id) = queue.pop_front() {
            queued.remove(&id);
            let Some(widget) = index.get(id) else {
                continue;
            };
            if claimed.contains(&id) || widget.children.is_empty() {
                continue;
            }
            let saw_claimed_content = widget.children.iter().any(|child| claimed.contains(child));
            let all_claimed_or_chrome = widget.children.iter().all(|child| {
                claimed.contains(child) || index.get(*child).is_some_and(widget_is_layout_chrome)
            });
            if !saw_claimed_content || !all_claimed_or_chrome {
                continue;
            }
            claimed.insert(id);
            claimed.extend(
                widget
                    .children
                    .iter()
                    .copied()
                    .filter(|child| index.get(*child).is_some_and(widget_is_layout_chrome)),
            );
            if let Some(parent) = widget.parent
                && queued.insert(parent)
            {
                queue.push_back(parent);
            }
        }
    }

    /// Navigation projection only (region-tagged seeds). Prefer [`Self::region_views`].
    pub fn navigation_slice(&self, limit: usize) -> Self {
        self.exclusive_tagged_region_slice(
            &RegionProjectionIndex::new(self),
            SemanticRegionTag::Navigation,
            limit,
        )
    }

    /// Inspector projection only (region-tagged seeds). Prefer [`Self::region_views`].
    ///
    /// Does **not** harvest untagged `Card` / `SettingsCard` — those stay in Primary.
    pub fn inspector_slice(&self, limit: usize) -> Self {
        self.exclusive_tagged_region_slice(
            &RegionProjectionIndex::new(self),
            SemanticRegionTag::Inspector,
            limit,
        )
    }

    /// Region projection that only keeps widgets owned by `tag` under the
    /// nearest-tag rule, so Navigation and Inspector forests never share ids.
    fn exclusive_tagged_region_slice(
        &self,
        index: &RegionProjectionIndex<'_>,
        tag: SemanticRegionTag,
        limit: usize,
    ) -> Self {
        use std::collections::{BTreeSet, VecDeque};

        if limit == 0 {
            return self.subtree_from(std::iter::empty());
        }
        let seeds: Vec<WidgetId> = self
            .widgets
            .iter()
            .filter(|widget| index.reachable.contains(&widget.id))
            .filter(|widget| widget_matches_region_tag(widget, tag))
            .filter(|widget| index.owner(widget.id) == Some(tag))
            .filter(|widget| !index.has_same_region_ancestor(widget.id, tag))
            .map(|w| w.id)
            .take(limit)
            .collect();

        let mut keep = BTreeSet::new();
        let mut queue: VecDeque<WidgetId> = seeds.iter().copied().collect();
        let roots = seeds.clone();
        let root_ids: BTreeSet<WidgetId> = roots.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            if index.owner(id) != Some(tag) {
                continue;
            }
            if !keep.insert(id) {
                continue;
            }
            if let Some(widget) = index.get(id) {
                for &child in &widget.children {
                    queue.push_back(child);
                }
            }
        }
        let widgets: Vec<SemanticWidget> = self
            .widgets
            .iter()
            .filter(|w| keep.contains(&w.id))
            .map(|w| {
                let mut cloned = w.clone();
                cloned.children.retain(|c| keep.contains(c));
                if root_ids.contains(&cloned.id)
                    || cloned.parent.is_some_and(|p| !keep.contains(&p))
                {
                    cloned.parent = None;
                }
                cloned
            })
            .collect();
        Self {
            revision: self.revision,
            theme: self.theme,
            appearance: self.appearance,
            roots: roots
                .into_iter()
                .filter(|id| keep.contains(id) && index.get(*id).is_some())
                .collect(),
            widgets,
        }
    }

    fn excluding_ids(&self, ids: &std::collections::BTreeSet<WidgetId>) -> Self {
        if ids.is_empty() {
            return self.clone();
        }
        let widgets: Vec<SemanticWidget> = self
            .widgets
            .iter()
            .filter(|w| !ids.contains(&w.id))
            .map(|w| {
                let mut cloned = w.clone();
                cloned.children.retain(|c| !ids.contains(c));
                if cloned.parent.is_some_and(|p| ids.contains(&p)) {
                    cloned.parent = None;
                }
                cloned
            })
            .collect();
        let keep: std::collections::BTreeSet<WidgetId> = widgets.iter().map(|w| w.id).collect();
        let mut out = Self {
            revision: self.revision,
            theme: self.theme,
            appearance: self.appearance,
            roots: self
                .roots
                .iter()
                .copied()
                .filter(|id| keep.contains(id))
                .collect(),
            widgets,
        };
        out.collapse_stale_grid_tracks_after_child_removal();
        out
    }

    /// When region projection removes children, multi-track grids that still
    /// declare more columns/rows than remaining kids would size the first
    /// survivor into the former sidebar/header track (e.g. 220px). Collapse to
    /// `n` equal `minmax(0,1fr)` tracks (or clear when empty).
    fn collapse_stale_grid_tracks_after_child_removal(&mut self) {
        for w in &mut self.widgets {
            let child_count = w.children.len();
            Self::collapse_stale_grid_tracks(&mut w.props.layout.grid_columns, child_count);
            Self::collapse_stale_grid_tracks(&mut w.props.layout.grid_rows, child_count);
        }
    }

    fn collapse_stale_grid_tracks(tracks: &mut Option<Vec<GridTrack>>, child_count: usize) {
        if tracks
            .as_ref()
            .is_some_and(|tracks| tracks.len() > child_count)
        {
            *tracks = (child_count > 0).then(|| {
                (0..child_count)
                    .map(|_| GridTrack::MinMax {
                        min_px: 0.0,
                        fr: 1.0,
                        max_px: None,
                    })
                    .collect()
            });
        }
    }
}

/// Mutually exclusive semantic projections for DesktopShell regions.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticRegionViews {
    pub primary: SemanticSnapshot,
    pub navigation: SemanticSnapshot,
    pub inspector: SemanticSnapshot,
}

impl SemanticRegionViews {
    /// Widget ids that appear in more than one region view.
    ///
    /// [`SemanticSnapshot::region_views`] builds exclusive projections, so this
    /// should always be empty; tests assert it, and hosts may `debug_assert` it.
    pub fn overlapping_ids(&self) -> Vec<WidgetId> {
        use std::collections::{BTreeMap, BTreeSet};

        let mut counts: BTreeMap<WidgetId, u8> = BTreeMap::new();
        for id in self
            .primary
            .widgets
            .iter()
            .chain(self.navigation.widgets.iter())
            .chain(self.inspector.widgets.iter())
            .map(|w| w.id)
        {
            *counts.entry(id).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .filter_map(|(id, n)| (n > 1).then_some(id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SemanticRegionTag {
    Navigation,
    Inspector,
}

/// One-pass index for DesktopShell region projection.
///
/// The snapshot remains an ordered `Vec` for stable public output. Projection
/// builds this private index once so ownership, reachability, and nested-region
/// checks do not repeatedly scan the full forest.
struct RegionProjectionIndex<'a> {
    by_id: std::collections::HashMap<WidgetId, &'a SemanticWidget>,
    owners: std::collections::HashMap<WidgetId, SemanticRegionTag>,
    reachable: std::collections::HashSet<WidgetId>,
    navigation_ancestors: std::collections::HashSet<WidgetId>,
    inspector_ancestors: std::collections::HashSet<WidgetId>,
}

impl<'a> RegionProjectionIndex<'a> {
    fn new(snapshot: &'a SemanticSnapshot) -> Self {
        use std::collections::{HashMap, HashSet};

        let by_id: HashMap<_, _> = snapshot
            .widgets
            .iter()
            .map(|widget| (widget.id, widget))
            .collect();
        let mut index = Self {
            by_id,
            owners: HashMap::new(),
            reachable: HashSet::new(),
            navigation_ancestors: HashSet::new(),
            inspector_ancestors: HashSet::new(),
        };
        let mut visited = HashSet::new();

        for &root in &snapshot.roots {
            index.extend_from(root, None, false, false, true, &mut visited);
        }
        for widget in &snapshot.widgets {
            if visited.contains(&widget.id) {
                continue;
            }
            if widget
                .parent
                .is_none_or(|parent| !index.by_id.contains_key(&parent))
            {
                index.extend_from(widget.id, None, false, false, false, &mut visited);
            }
        }
        // Malformed cycles fail closed as unreachable orphan forests.
        for widget in &snapshot.widgets {
            if !visited.contains(&widget.id) {
                index.extend_from(widget.id, None, false, false, false, &mut visited);
            }
        }
        index
    }

    fn extend_from(
        &mut self,
        root: WidgetId,
        inherited_owner: Option<SemanticRegionTag>,
        has_navigation_ancestor: bool,
        has_inspector_ancestor: bool,
        reachable: bool,
        visited: &mut std::collections::HashSet<WidgetId>,
    ) {
        let mut stack = vec![(
            root,
            inherited_owner,
            has_navigation_ancestor,
            has_inspector_ancestor,
        )];
        while let Some((id, inherited, nav_ancestor, inspector_ancestor)) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Some(widget) = self.by_id.get(&id).copied() else {
                continue;
            };
            if reachable {
                self.reachable.insert(id);
            }
            if nav_ancestor {
                self.navigation_ancestors.insert(id);
            }
            if inspector_ancestor {
                self.inspector_ancestors.insert(id);
            }
            let explicit = widget_region_tag(widget);
            let owner = explicit.or(inherited);
            if let Some(owner) = owner {
                self.owners.insert(id, owner);
            }
            let child_nav_ancestor =
                nav_ancestor || explicit == Some(SemanticRegionTag::Navigation);
            let child_inspector_ancestor =
                inspector_ancestor || explicit == Some(SemanticRegionTag::Inspector);
            stack.extend(
                widget
                    .children
                    .iter()
                    .rev()
                    .map(|&child| (child, owner, child_nav_ancestor, child_inspector_ancestor)),
            );
        }
    }

    fn get(&self, id: WidgetId) -> Option<&'a SemanticWidget> {
        self.by_id.get(&id).copied()
    }

    fn owner(&self, id: WidgetId) -> Option<SemanticRegionTag> {
        self.owners.get(&id).copied()
    }

    fn has_same_region_ancestor(&self, id: WidgetId, tag: SemanticRegionTag) -> bool {
        match tag {
            SemanticRegionTag::Navigation => self.navigation_ancestors.contains(&id),
            SemanticRegionTag::Inspector => self.inspector_ancestors.contains(&id),
        }
    }
}

fn widget_region_tag(widget: &SemanticWidget) -> Option<SemanticRegionTag> {
    if widget_is_inspector_region(widget) {
        Some(SemanticRegionTag::Inspector)
    } else if widget_is_navigation_region(widget) {
        Some(SemanticRegionTag::Navigation)
    } else {
        None
    }
}

fn widget_matches_region_tag(widget: &SemanticWidget, tag: SemanticRegionTag) -> bool {
    match tag {
        SemanticRegionTag::Navigation => widget_is_navigation_region(widget),
        SemanticRegionTag::Inspector => widget_is_inspector_region(widget),
    }
}

/// Explicit DesktopShell Navigation opt-in — not control-kind / Lilia in-tree heuristics.
///
/// Contract markers (nanavue [`NanaWorkspaceShell`] / explicit host tags):
/// - `data-region` / `region` / `data-region-role`: `global-navigation` |
///   `section-navigation` (and compact aliases)
/// - `data-agent-id` that **names the DesktopShell contract** (`*.global-navigation`,
///   `*.section-navigation`, `nana.workspace.sidebar` only when paired with an
///   explicit region token above — agent alone is not enough)
/// - `role` / class mirrors of `global-navigation` / `section-navigation`
///
/// **Not** Navigation (stay in Primary / in-tree workspace paint):
/// - Lilia `@lilia/ui` `data-region-role=resources` / `workspace.region.sidebar`
/// - bare `agent-id=sidebar` / default `nana.sidebar-frame` (SecondaryPanel /
///   NanaSidebarFrame identity, not DesktopShell lift)
/// - WidgetKind::SidebarFrame alone
fn widget_is_navigation_region(widget: &SemanticWidget) -> bool {
    let region = widget_region_token(widget);
    if matches!(
        region.as_str(),
        "global-navigation" | "section-navigation" | "globalnavigation" | "sectionnavigation"
    ) {
        return true;
    }
    // Agent may reinforce an explicit region token, but must not invent one.
    // `nana.workspace.sidebar` without `data-region` is layout identity only.
    let agent = widget.props.agent_id.to_ascii_lowercase();
    if agent.contains("global-navigation")
        || agent.contains("section-navigation")
        || agent.ends_with(".global-navigation")
        || agent.ends_with(".section-navigation")
    {
        return true;
    }
    let role = widget.props.role.to_ascii_lowercase();
    if matches!(
        role.as_str(),
        "global-navigation" | "section-navigation" | "globalnavigation" | "sectionnavigation"
    ) {
        return true;
    }
    widget.props.class_names.iter().any(|c| {
        matches!(
            c.to_ascii_lowercase().as_str(),
            "nana-global-navigation" | "nana-section-navigation"
        )
    })
}

/// Resize handles / separators that stay behind when region content is lifted.
fn widget_is_layout_chrome(widget: &SemanticWidget) -> bool {
    let role = widget.props.role.to_ascii_lowercase();
    if role == "separator" || role == "presentation" {
        return true;
    }
    let agent = widget.props.agent_id.to_ascii_lowercase();
    if agent.contains(".resize") || agent.ends_with("resize") {
        return true;
    }
    widget.props.class_names.iter().any(|c| {
        let c = c.to_ascii_lowercase();
        c.contains("resize-handle") || c.ends_with("__resize")
    })
}

/// Region id from `props.region` or `data-region-role` / `data-region` attrs.
fn widget_region_token(widget: &SemanticWidget) -> String {
    if !widget.props.region.is_empty() {
        return widget.props.region.to_ascii_lowercase();
    }
    for key in ["data-region-role", "data-region", "region"] {
        if let Some(v) = widget.props.attrs.get(key)
            && !v.is_empty()
        {
            return v.to_ascii_lowercase();
        }
    }
    String::new()
}

fn widget_is_inspector_region(widget: &SemanticWidget) -> bool {
    let region = widget_region_token(widget);
    if region == "inspector" || region.contains("inspector") {
        return true;
    }
    let agent = widget.props.agent_id.to_ascii_lowercase();
    if agent.contains("inspector") {
        return true;
    }
    let role = widget.props.role.to_ascii_lowercase();
    if role == "inspector" || role.contains("inspector") {
        return true;
    }
    widget
        .props
        .class_names
        .iter()
        .any(|c| matches!(c.to_ascii_lowercase().as_str(), "nana-inspector"))
}

/// One semantic widget node.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticWidget {
    pub id: WidgetId,
    pub kind: WidgetKind,
    pub props: WidgetProps,
    pub children: Vec<WidgetId>,
    pub parent: Option<WidgetId>,
}

/// Owns the semantic widget forest + pending Iced-bound events + theme inject state.
#[derive(Debug)]
pub struct MessageBridge {
    widgets: HashMap<WidgetId, SemanticWidget>,
    roots: Vec<WidgetId>,
    pending: VecDeque<BridgeEvent>,
    revision: u64,
    theme: ThemeMode,
    appearance: AppearanceSettings,
    /// When true, html/body scaffold owns roots — createElement must not promote.
    scaffolded: bool,
    /// Parsed author stylesheet rules (source order across inject calls).
    /// Declaration entries are cached on each [`StyleRule`] at parse time.
    stylesheet_rules: Vec<StyleRule>,
    next_rule_order: u32,
    /// Document-level custom properties (`:root` / `html` / `body` …) as inheritance base.
    /// Rebuilt from [`stylesheet_rules`] (no raw CSS re-scrape).
    stylesheet_vars: BTreeMap<String, String>,
    /// Last synced layout viewport (`vw`/`vh` resolve during cascade).
    layout_viewport: Option<(f32, f32)>,
}

impl Default for MessageBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageBridge {
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
            roots: Vec::new(),
            pending: VecDeque::new(),
            revision: 0,
            theme: ThemeMode::Light,
            appearance: AppearanceSettings::default(),
            scaffolded: false,
            stylesheet_rules: Vec::new(),
            next_rule_order: 0,
            stylesheet_vars: BTreeMap::new(),
            layout_viewport: None,
        }
    }

    /// Parse and retain stylesheet rules, then re-apply cascade to all widgets.
    ///
    /// Empty / fully-deferred sheets are a no-op (no dirty cascade). Non-empty
    /// injects still require a full-tree reapply because new selectors may match
    /// any node; per-rule declaration parse is not repeated (cached on rules).
    pub fn inject_stylesheet(&mut self, css: &str) {
        if css.trim().is_empty() {
            return;
        }
        let parsed = parse_stylesheet(css, self.next_rule_order);
        if parsed.is_empty() {
            return;
        }
        if let Some(last) = parsed.last() {
            self.next_rule_order = last.source_order.saturating_add(1);
        }
        self.stylesheet_rules.extend(parsed);
        self.rebuild_stylesheet_vars();
        self.reapply_layout_cascade_all();
    }

    /// Re-collect document `--*` for the active theme from cached rule entries.
    fn rebuild_stylesheet_vars(&mut self) {
        let theme = self.theme_label().to_string();
        self.stylesheet_vars =
            collect_document_custom_properties_from_rules(&self.stylesheet_rules, &theme);
    }

    pub fn stylesheet_rule_count(&self) -> usize {
        self.stylesheet_rules.len()
    }

    /// Record Vue `setScopeId` attribute for scoped selector matching.
    pub fn set_scope_attr(&mut self, id: WidgetId, scope: &str) {
        let Some(widget) = self.widgets.get_mut(&id) else {
            return;
        };
        let name = if scope.starts_with("data-") {
            scope.to_string()
        } else {
            format!("data-v-{scope}")
        };
        widget.props.attrs.insert(name, String::new());
        self.reapply_layout_for(id);
        self.bump();
    }

    fn reapply_layout_cascade_all(&mut self) {
        let mut ids: Vec<WidgetId> = self.widgets.keys().copied().collect();
        // Parents before children so inherited typography / em font-size see
        // computed ancestor `font-size` (CSS inheritance + rem root).
        ids.sort_by_cached_key(|id| self.widget_depth(*id));
        for id in ids {
            self.reapply_layout_for(id);
        }
        self.bump();
    }

    fn widget_depth(&self, id: WidgetId) -> usize {
        let mut depth = 0usize;
        let mut cur = self.widgets.get(&id).and_then(|w| w.parent);
        while let Some(pid) = cur {
            depth += 1;
            cur = self.widgets.get(&pid).and_then(|w| w.parent);
        }
        depth
    }

    /// Root `rem` base: html → body → CSS initial 16px.
    fn document_root_font_px(&self) -> f32 {
        self.widgets
            .values()
            .find(|w| w.props.element_tag.eq_ignore_ascii_case("html"))
            .or_else(|| {
                self.widgets
                    .values()
                    .find(|w| w.props.element_tag.eq_ignore_ascii_case("body"))
            })
            .and_then(|w| w.props.layout.font_size)
            .unwrap_or(16.0)
            .max(1.0)
    }

    /// Parent computed font-size as `em` base while applying this node's CSS.
    fn font_context_for(&self, id: WidgetId) -> crate::css_map::FontSizeContext {
        let root_px = self.document_root_font_px();
        let parent_px = self
            .widgets
            .get(&id)
            .and_then(|w| w.parent)
            .and_then(|pid| self.widgets.get(&pid))
            .and_then(|p| p.props.layout.font_size)
            .unwrap_or(root_px);
        crate::css_map::FontSizeContext::new(root_px, parent_px)
    }

    fn reapply_layout_for(&mut self, id: WidgetId) {
        let vars = self.inherited_css_vars_for(id);
        let fonts = self.font_context_for(id);
        let viewport = self.layout_viewport;
        let dark = matches!(self.theme, ThemeMode::Dark);
        let run = || {
            if let Some((vw, vh)) = viewport {
                crate::css_map::with_active_viewport(vw, vh, || {
                    crate::css_map::with_active_font_sizes(fonts, || {
                        crate::css_map::with_active_css_vars(&vars, || {
                            self.reapply_layout_for_inner(id);
                        })
                    })
                });
            } else {
                crate::css_map::with_active_font_sizes(fonts, || {
                    crate::css_map::with_active_css_vars(&vars, || {
                        self.reapply_layout_for_inner(id);
                    })
                });
            }
        };
        crate::css_map::with_active_color_scheme_dark(dark, run);
        self.strip_deferred_position_on_overlay(id);
    }

    /// Overlay kinds must not retain companion CSS `fixed`/`sticky`.
    /// L2 floats use Nana Overlay; anonymous CSS fixed stays on non-overlay nodes.
    fn strip_deferred_position_on_overlay(&mut self, id: WidgetId) {
        if let Some(w) = self.widgets.get_mut(&id)
            && w.kind.is_overlay()
            && matches!(
                w.props.layout.position,
                crate::css_map::PositionSpec::Fixed | crate::css_map::PositionSpec::Sticky
            )
        {
            w.props.layout.position = crate::css_map::PositionSpec::Static;
        }
    }

    /// Document vars + ancestor/self matched `--*` + inline/prop (root → leaf).
    fn inherited_css_vars_for(&self, id: WidgetId) -> BTreeMap<String, String> {
        let mut chain = Vec::new();
        let mut cur = Some(id);
        while let Some(cid) = cur {
            chain.push(cid);
            cur = self.widgets.get(&cid).and_then(|w| w.parent);
        }
        chain.reverse();
        let mut map = self.stylesheet_vars.clone();
        for cid in chain {
            let overlay = self.authored_custom_properties_on(cid);
            if !overlay.is_empty() {
                map = crate::css_map::merge_css_custom_properties(&map, &overlay);
            }
        }
        // Ensure nested var()/simple calc in the inherited map are folded.
        crate::css_map::merge_css_custom_properties(&map, &BTreeMap::new())
    }

    fn authored_custom_properties_on(&self, id: WidgetId) -> BTreeMap<String, String> {
        let Some(ancestry) = self.match_ancestry(id) else {
            return BTreeMap::new();
        };
        let Some(widget) = self.widgets.get(&id) else {
            return BTreeMap::new();
        };
        let leaf_classes = widget.props.class_names.clone();
        let leaf_attrs = widget.props.attrs.clone();
        let leaf_tag = widget.props.element_tag.clone();
        let leaf_id = widget.props.element_id.clone();
        let prop_style = widget.props.prop_style.clone();
        let inline_style = widget.props.inline_style.clone();
        let (sibling_index, sibling_count) = self.sibling_position(id);
        let (of_type_index, of_type_count) = self.of_type_position(id);
        let prev_snaps = self.prev_sibling_snaps(id);
        let ancestor_nodes: Vec<MatchNode<'_>> = ancestry
            .iter()
            .skip(1)
            .map(|n| MatchNode {
                tag: n.tag.as_str(),
                id: n.id.as_str(),
                classes: n.classes.as_slice(),
                attrs: &n.attrs,
            })
            .collect();
        let prev_nodes: Vec<MatchNode<'_>> = prev_snaps
            .iter()
            .map(|n| MatchNode {
                tag: n.tag.as_str(),
                id: n.id.as_str(),
                classes: n.classes.as_slice(),
                attrs: &n.attrs,
            })
            .collect();
        let ctx = MatchContext {
            tag: leaf_tag.as_str(),
            id: leaf_id.as_str(),
            classes: leaf_classes.as_slice(),
            attrs: &leaf_attrs,
            ancestors: ancestor_nodes.as_slice(),
            preceding_siblings: prev_nodes.as_slice(),
            sibling_index,
            sibling_count,
            of_type_index,
            of_type_count,
        };
        let mut map = crate::css_cascade::matched_custom_properties(&self.stylesheet_rules, &ctx);
        for (k, v) in crate::css_map::extract_css_custom_properties_from_decls(&prop_style) {
            map.insert(k, v);
        }
        for (k, v) in crate::css_map::extract_css_custom_properties_from_decls(&inline_style) {
            map.insert(k, v);
        }
        map
    }

    fn reapply_layout_for_inner(&mut self, id: WidgetId) {
        let Some(ancestry) = self.match_ancestry(id) else {
            return;
        };
        let Some(widget) = self.widgets.get(&id) else {
            return;
        };
        let kind = widget.kind;
        let class_names = widget.props.class_names.clone();
        let element_tag = widget.props.element_tag.clone();
        let element_id = widget.props.element_id.clone();
        let attrs = widget.props.attrs.clone();
        let inline_style = widget.props.inline_style.clone();
        let prop_style = widget.props.prop_style.clone();
        let hidden = widget.props.layout.hidden;
        let keep_bg = widget.props.layout.background;
        let keep_border_color = widget.props.layout.border_color;
        let keep_border_width = widget.props.layout.border_width;
        let cb_w = widget.props.containing_block_width;
        let cb_h = widget.props.containing_block_height;

        // ancestry is [self, parent, grandparent, …] — full chain for combinators.
        let leaf_classes = class_names;
        let leaf_attrs = attrs;
        let leaf_tag = element_tag;
        let leaf_id = element_id;

        let (sibling_index, sibling_count) = self.sibling_position(id);
        let (of_type_index, of_type_count) = self.of_type_position(id);
        let prev_snaps = self.prev_sibling_snaps(id);

        let ancestor_nodes: Vec<MatchNode<'_>> = ancestry
            .iter()
            .skip(1)
            .map(|n| MatchNode {
                tag: n.tag.as_str(),
                id: n.id.as_str(),
                classes: n.classes.as_slice(),
                attrs: &n.attrs,
            })
            .collect();
        let prev_nodes: Vec<MatchNode<'_>> = prev_snaps
            .iter()
            .map(|n| MatchNode {
                tag: n.tag.as_str(),
                id: n.id.as_str(),
                classes: n.classes.as_slice(),
                attrs: &n.attrs,
            })
            .collect();
        let ctx = MatchContext {
            tag: leaf_tag.as_str(),
            id: leaf_id.as_str(),
            classes: leaf_classes.as_slice(),
            attrs: &leaf_attrs,
            ancestors: ancestor_nodes.as_slice(),
            preceding_siblings: prev_nodes.as_slice(),
            sibling_index,
            sibling_count,
            of_type_index,
            of_type_count,
        };

        // Layer order: kind default → stylesheet → class hints → prop → inline.
        // When any author text layer or retained stylesheet exists, rebuild from
        // a clean base so prior stylesheet-computed fields do not stick after
        // selector/class changes. Document-root Fill is restored by public
        // `nana-*-root` class contracts — not by preserving computed layout
        // across a global stylesheet inject.
        //
        // Critical: do **not** seed `direction` from WidgetKind when author CSS
        // is present. `default_layout_for_kind(Column)` would set Column, then
        // `display:flex` (`if direction.is_none() → Row`) would no-op — toolbars
        // with `justify-content:space-between` stay vertical and eat the Fill
        // height, clipping siblings (Repo evidence main pane painted empty).
        let base = if self.stylesheet_rules.is_empty()
            && inline_style.trim().is_empty()
            && prop_style.trim().is_empty()
        {
            // Preserve LayoutStyle fields assigned directly (scaffold /
            // createWidget / register props) when no author CSS layers exist.
            let mut layout = self
                .widgets
                .get(&id)
                .map(|w| w.props.layout.clone())
                .unwrap_or_else(|| default_layout_for_kind(kind));
            let defaults = default_layout_for_kind(kind);
            if layout.direction.is_none() {
                layout.direction = defaults.direction;
            }
            if layout.gap.is_none() {
                layout.gap = defaults.gap;
            }
            if layout.padding.is_none() {
                layout.padding = defaults.padding;
            }
            layout
        } else {
            let mut layout = LayoutStyle::default();
            let defaults = default_layout_for_kind(kind);
            // Card/SettingsCard keep kind padding seed only — never direction.
            // Gap must come from author CSS / `gap-*` hints (not kind default).
            layout.gap = defaults.gap;
            layout.padding = defaults.padding;
            layout
        };

        // Author layers: stylesheet → class hints → prop style → class hints →
        // inline → class hints. Layout sizing comes from those layers / public
        // class contracts — not from id / data-region-id / kind whitelists.
        let mut layout = rebuild_layout_style(
            base,
            &self.stylesheet_rules,
            &ctx,
            &prop_style,
            &inline_style,
            cb_w,
            cb_h,
        );
        // Custom-element contract: tag `nana-sidebar-frame` / `nana-sidebar-row`
        // mirrors the public class hints when Vue omitted `class` (host CEs often
        // only set the tag). This is the element-name contract — not a WidgetKind
        // whitelist inventing geometry from the enum alone.
        if leaf_tag.starts_with("nana-") && !leaf_classes.iter().any(|c| c == &leaf_tag) {
            layout.apply_class_layout_hints(std::slice::from_ref(&leaf_tag));
        }
        // Preserve SVG fill/stroke paint when stylesheet didn't set them —
        // unless the author explicitly declared `fill`/`stroke` this pass and
        // resolution failed (e.g. LightningCSS `light-dark` → `initial`). Keeping
        // a prior dark `#1c1c1c` would paint black empty heatmap cells on light.
        let author_fill = css_decl_mentions(&inline_style, "fill")
            || css_decl_mentions(&prop_style, "fill")
            || leaf_attrs.contains_key("fill");
        let author_stroke = css_decl_mentions(&inline_style, "stroke")
            || css_decl_mentions(&prop_style, "stroke")
            || leaf_attrs.contains_key("stroke");
        if layout.background.is_none() && !author_fill {
            layout.background = keep_bg;
        }
        if layout.border_color.is_none() && !author_stroke {
            layout.border_color = keep_border_color;
        }
        if layout.border_width.is_none() && !author_stroke {
            layout.border_width = keep_border_width;
        }
        // CSS typography inherits when the author layers leave fields unset.
        if let Some(parent_id) = self.widgets.get(&id).and_then(|w| w.parent)
            && let Some(parent) = self.widgets.get(&parent_id)
        {
            layout.inherit_typography_from(&parent.props.layout);
        }
        // Preserve explicit hidden flag from the `hidden` attribute.
        if hidden {
            layout.hidden = true;
        }

        if let Some(widget) = self.widgets.get_mut(&id) {
            widget.props.layout = layout;
            pin_svg_chart_min_height(&mut widget.props);
            widget.kind = apply_display_to_kind(widget.kind, &widget.props.layout);
        }
    }

    fn match_ancestry(&self, id: WidgetId) -> Option<Vec<MatchNodeSnap>> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(cid) = cur {
            let w = self.widgets.get(&cid)?;
            out.push(MatchNodeSnap {
                tag: if w.props.element_tag.is_empty() {
                    w.kind.element_tag().to_string()
                } else {
                    w.props.element_tag.clone()
                },
                id: w.props.element_id.clone(),
                classes: w.props.class_names.clone(),
                attrs: w.props.attrs.clone(),
            });
            cur = w.parent;
        }
        Some(out)
    }

    /// Position among parent's children for `:first-child` / `:last-child`.
    fn sibling_position(&self, id: WidgetId) -> (usize, usize) {
        let Some(widget) = self.widgets.get(&id) else {
            return (0, 1);
        };
        let Some(parent_id) = widget.parent else {
            return (0, 1);
        };
        let Some(parent) = self.widgets.get(&parent_id) else {
            return (0, 1);
        };
        let count = parent.children.len();
        let index = parent
            .children
            .iter()
            .position(|&cid| cid == id)
            .unwrap_or(0);
        (index, count.max(1))
    }

    /// Position among same-tag siblings for `:nth-of-type` (0-based index, count).
    fn of_type_position(&self, id: WidgetId) -> (usize, usize) {
        let Some(widget) = self.widgets.get(&id) else {
            return (0, 1);
        };
        let tag = if widget.props.element_tag.is_empty() {
            widget.kind.element_tag().to_string()
        } else {
            widget.props.element_tag.clone()
        };
        let Some(parent_id) = widget.parent else {
            return (0, 1);
        };
        let Some(parent) = self.widgets.get(&parent_id) else {
            return (0, 1);
        };
        let mut index = 0usize;
        let mut count = 0usize;
        for &cid in &parent.children {
            let Some(w) = self.widgets.get(&cid) else {
                continue;
            };
            let t = if w.props.element_tag.is_empty() {
                w.kind.element_tag().to_string()
            } else {
                w.props.element_tag.clone()
            };
            if !t.eq_ignore_ascii_case(&tag) {
                continue;
            }
            if cid == id {
                index = count;
            }
            count += 1;
        }
        (index, count.max(1))
    }

    fn prev_sibling_snaps(&self, id: WidgetId) -> Vec<MatchNodeSnap> {
        let Some(widget) = self.widgets.get(&id) else {
            return Vec::new();
        };
        let Some(parent_id) = widget.parent else {
            return Vec::new();
        };
        let Some(parent) = self.widgets.get(&parent_id) else {
            return Vec::new();
        };
        let Some(index) = parent.children.iter().position(|&cid| cid == id) else {
            return Vec::new();
        };
        parent.children[..index]
            .iter()
            .rev()
            .filter_map(|&cid| {
                let w = self.widgets.get(&cid)?;
                Some(MatchNodeSnap {
                    tag: if w.props.element_tag.is_empty() {
                        w.kind.element_tag().to_string()
                    } else {
                        w.props.element_tag.clone()
                    },
                    id: w.props.element_id.clone(),
                    classes: w.props.class_names.clone(),
                    attrs: w.props.attrs.clone(),
                })
            })
            .collect()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn theme(&self) -> ThemeMode {
        self.theme
    }

    pub fn set_theme(&mut self, theme: ThemeMode) {
        let changed = self.theme != theme;
        self.theme = theme;
        // Mirror JS `documentElement.dataset.theme` onto the html scaffold so
        // cascade `[data-theme=…]` / `:root[data-theme=…]` can match the node
        // (document `--*` still come from theme-aware stylesheet_vars).
        self.sync_document_theme_attr();
        if changed {
            // Theme-conditional document vars (`:root[data-theme=…]`) must
            // re-resolve; otherwise Primary paint sticks on the last inject.
            self.rebuild_stylesheet_vars();
            self.reapply_layout_cascade_all();
        }
    }

    fn sync_document_theme_attr(&mut self) {
        let label = self.theme_label().to_string();
        for w in self.widgets.values_mut() {
            let is_html_root = w.parent.is_none()
                && (w.props.element_tag.eq_ignore_ascii_case("html")
                    || w.props.class_names.iter().any(|c| c == "nana-html-root"));
            if is_html_root {
                w.props.attrs.insert("data-theme".into(), label.clone());
            }
        }
    }

    pub fn appearance(&self) -> AppearanceSettings {
        self.appearance
    }

    pub fn set_appearance(&mut self, appearance: AppearanceSettings) {
        if self.appearance != appearance {
            self.appearance = appearance;
            self.bump();
        }
    }

    /// Sync theme + Appearance fields from L1 `documentElement` dataset/style.
    ///
    /// Keys match `nanavue-components` / web-api shim:
    /// `theme`, `backdrop`, `backdropTarget`, `titlebarFollowsSidebar`,
    /// `workspaceCorners`, and style `--nana-backdrop-opacity` /
    /// `--backdrop-opacity` / `--app-corner-radius`.
    ///
    /// Theme direction: JS `dataset.theme` → bridge [`ThemeMode`] (paired with
    /// [`crate::VueHost::inject_theme`] for Rust → JS).
    pub fn apply_document_appearance(
        &mut self,
        dataset: &BTreeMap<String, String>,
        style: &BTreeMap<String, String>,
    ) {
        if let Some(raw) = dataset.get("theme") {
            let mode = if raw.eq_ignore_ascii_case("dark") {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
            self.set_theme(mode);
        }
        let mut next = self.appearance;
        if let Some(raw) = dataset.get("backdrop") {
            let mode = match raw.as_str() {
                "translucent" | "system" | "mica" | "acrylic" => WindowMaterialMode::Translucent,
                _ => WindowMaterialMode::Solid,
            };
            next.set_window_material(mode);
        }
        if let Some(raw) = dataset.get("backdropTarget") {
            let target = if raw == "main" {
                BackdropTarget::Main
            } else {
                BackdropTarget::Sidebar
            };
            next.set_backdrop_target(target);
        }
        if let Some(raw) = dataset.get("titlebarFollowsSidebar") {
            next.set_titlebar_follows_sidebar(raw != "false");
        }
        if let Some(raw) = dataset.get("workspaceCorners") {
            next.set_workspace_corners_enabled(raw != "false");
        } else if let Some(raw) = dataset.get("corners") {
            // Legacy: only "square" disables workspace corners.
            next.set_workspace_corners_enabled(raw != "square");
        }
        if let Some(raw) = style
            .get("--nana-backdrop-opacity")
            .or_else(|| style.get("--backdrop-opacity"))
            .or_else(|| style.get("backdrop-opacity"))
            .or_else(|| style.get("nana-backdrop-opacity"))
            && let Ok(opacity) = raw.parse::<f32>()
        {
            next.set_backdrop_opacity(opacity);
        }
        if let Some(raw) = style
            .get("--app-corner-radius")
            .or_else(|| style.get("app-corner-radius"))
        {
            let px = raw.trim_end_matches("px").trim();
            if let Ok(radius) = px.parse::<f32>() {
                next.set_standard_radius(radius);
            }
        }
        self.set_appearance(next);
    }

    pub fn theme_label(&self) -> &'static str {
        match self.theme {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn get(&self, id: WidgetId) -> Option<&SemanticWidget> {
        self.widgets.get(&id)
    }

    pub fn get_mut(&mut self, id: WidgetId) -> Option<&mut SemanticWidget> {
        self.widgets.get_mut(&id)
    }

    pub(crate) fn root_ids(&self) -> &[WidgetId] {
        &self.roots
    }

    pub fn contains(&self, id: WidgetId) -> bool {
        self.widgets.contains_key(&id)
    }

    /// Register html + body so Vue mounts parent under a real semantic root.
    pub fn ensure_document_roots(&mut self, html_id: WidgetId, body_id: WidgetId) {
        let theme_label = self.theme_label().to_string();
        self.widgets.entry(html_id).or_insert_with(|| {
            let mut props = WidgetProps::default();
            props.layout.width = Some(LengthSpec::Fill);
            props.layout.height = Some(LengthSpec::Fill);
            props.layout.direction = Some(FlexDirection::Column);
            props.class_names = vec!["nana-html-root".into()];
            props.element_tag = "html".into();
            props.attrs.insert("data-theme".into(), theme_label);
            SemanticWidget {
                id: html_id,
                kind: WidgetKind::Column,
                props,
                children: vec![body_id],
                parent: None,
            }
        });
        self.sync_document_theme_attr();
        self.widgets.entry(body_id).or_insert_with(|| {
            let mut props = WidgetProps::default();
            props.layout.width = Some(LengthSpec::Fill);
            props.layout.height = Some(LengthSpec::Fill);
            props.layout.direction = Some(FlexDirection::Column);
            props.class_names = vec!["nana-mount-root".into()];
            SemanticWidget {
                id: body_id,
                kind: WidgetKind::Column,
                props,
                children: Vec::new(),
                parent: Some(html_id),
            }
        });
        if let Some(body) = self.widgets.get_mut(&body_id) {
            body.parent = Some(html_id);
        }
        let child_ok: Vec<WidgetId> = self
            .widgets
            .get(&html_id)
            .map(|html| {
                html.children
                    .iter()
                    .copied()
                    .filter(|c| *c == body_id || self.widgets.contains_key(c))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(html) = self.widgets.get_mut(&html_id) {
            html.children = child_ok;
            if !html.children.contains(&body_id) {
                html.children.push(body_id);
            }
        }
        self.roots.clear();
        // Paint from body — html is scaffolding only.
        self.roots.push(body_id);
        self.scaffolded = true;
        self.bump();
    }

    /// Drop mounted app widgets under body; keep html/body scaffold.
    pub fn clear_mounted(&mut self) {
        if !self.scaffolded {
            self.widgets.clear();
            self.roots.clear();
            self.bump();
            return;
        }
        let body_id = self
            .widgets
            .iter()
            .find(|(_, w)| w.props.class_names.iter().any(|c| c == "nana-mount-root"))
            .map(|(id, _)| *id);
        let Some(body_id) = body_id else {
            return;
        };
        let children: Vec<WidgetId> = self
            .widgets
            .get(&body_id)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        for child in children {
            self.unregister(child);
        }
        // Sweep orphans that lost their parent during partial unmounts.
        let orphans: Vec<WidgetId> = self
            .widgets
            .iter()
            .filter_map(|(&id, w)| {
                if self.roots.contains(&id) || id == body_id {
                    return None;
                }
                match w.parent {
                    Some(p) if self.widgets.contains_key(&p) => None,
                    _ => Some(id),
                }
            })
            .collect();
        for id in orphans {
            self.unregister(id);
        }
        self.bump();
    }

    pub fn register(&mut self, id: WidgetId, kind: WidgetKind, mut props: WidgetProps) {
        if props.element_tag.is_empty() {
            props.element_tag = kind.element_tag().to_string();
        }
        // Seed layout defaults for layout kinds; stylesheet / class / inline win later.
        let defaults = default_layout_for_kind(kind);
        if props.layout.direction.is_none() {
            props.layout.direction = defaults.direction;
        }
        if props.layout.gap.is_none() {
            props.layout.gap = defaults.gap;
        }
        if props.layout.padding.is_none() {
            props.layout.padding = defaults.padding;
        }
        let kind = apply_display_to_kind(kind, &props.layout);
        if kind.is_overlay() {
            apply_overlay_presence_open(&mut props);
            // Product floats use Nana Overlay — strip companion CSS fixed/sticky.
            if matches!(
                props.layout.position,
                crate::css_map::PositionSpec::Fixed | crate::css_map::PositionSpec::Sticky
            ) {
                props.layout.position = crate::css_map::PositionSpec::Static;
            }
        }
        self.widgets.insert(
            id,
            SemanticWidget {
                id,
                kind,
                props,
                children: Vec::new(),
                parent: None,
            },
        );
        // With document scaffold, only html is a root — insert parents under body.
        // Without scaffold (unit tests), keep legacy "register ⇒ root" behavior.
        if !self.scaffolded && !self.roots.contains(&id) {
            self.roots.push(id);
        }
        self.reapply_layout_for(id);
        self.bump();
    }

    /// Copy kind+props from `src` onto `dst` (no parenting). Returns false if `src` missing.
    pub fn clone_register(&mut self, src: WidgetId, dst: WidgetId) -> bool {
        let Some(widget) = self.widgets.get(&src).cloned() else {
            return false;
        };
        self.register(dst, widget.kind, widget.props);
        true
    }

    /// Ensure a node is registered; used when downleveling bare HTML.
    pub fn ensure(&mut self, id: WidgetId, kind: WidgetKind, props: WidgetProps) {
        if self.widgets.contains_key(&id) {
            if let Some(w) = self.widgets.get_mut(&id) {
                w.kind = kind;
                // Merge non-empty props lightly.
                if !props.label.is_empty() {
                    w.props.label = props.label;
                }
                if !props.value.is_empty() {
                    w.props.value = props.value;
                }
                if !props.class_names.is_empty() {
                    w.props.class_names = props.class_names;
                }
                if !props.role.is_empty() {
                    w.props.role = props.role;
                }
            }
            self.bump();
        } else {
            self.register(id, kind, props);
        }
    }

    pub fn set_kind(&mut self, id: WidgetId, kind: WidgetKind) {
        if let Some(w) = self.widgets.get_mut(&id)
            && w.kind != kind
        {
            w.kind = kind;
            self.bump();
        }
    }

    pub fn unregister(&mut self, id: WidgetId) {
        if let Some(widget) = self.widgets.remove(&id) {
            if let Some(parent) = widget.parent
                && let Some(p) = self.widgets.get_mut(&parent)
            {
                p.children.retain(|&c| c != id);
            }
            for child in widget.children {
                self.unregister(child);
            }
        }
        self.roots.retain(|&r| r != id);
        self.bump();
    }

    pub fn insert_child(&mut self, child: WidgetId, parent: WidgetId, anchor: Option<WidgetId>) {
        if let Some(prev) = self.widgets.get(&child).and_then(|w| w.parent)
            && let Some(p) = self.widgets.get_mut(&prev)
        {
            p.children.retain(|&c| c != child);
        }
        self.roots.retain(|&r| r != child);

        if !self.widgets.contains_key(&parent) {
            // With document scaffold, never promote random orphans to roots —
            // attach under the mount body instead so the forest stays single-rooted.
            if self.scaffolded
                && let Some(body_id) = self.mount_body_id()
            {
                return self.insert_child(child, body_id, None);
            }
            if self.widgets.contains_key(&child) {
                if let Some(w) = self.widgets.get_mut(&child) {
                    w.parent = None;
                }
                if !self.roots.contains(&child) {
                    self.roots.push(child);
                }
                self.bump();
            }
            return;
        }

        if let Some(w) = self.widgets.get_mut(&child) {
            w.parent = Some(parent);
        }
        if let Some(p) = self.widgets.get_mut(&parent) {
            let idx = anchor
                .and_then(|a| p.children.iter().position(|&c| c == a))
                .unwrap_or(p.children.len());
            if !p.children.contains(&child) {
                p.children.insert(idx, child);
            }
        }
        self.sync_containing_block_from_parent(child);
        // Parent combinators (`.parent > .child`) need a rebuild after insert.
        self.reapply_layout_for(child);
        self.bump();
    }

    /// Host / iced 回写最近布局得到的包含块尺寸（供后续 `style` `%` 解析）。
    pub fn set_containing_block(&mut self, id: WidgetId, width: Option<f32>, height: Option<f32>) {
        if !self.write_containing_block(id, width, height) {
            return;
        }
        let children = self
            .widgets
            .get(&id)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        self.bump();
        for child in children {
            self.sync_containing_block_from_parent(child);
        }
    }

    /// Iced / viewport 布局回写：按 Fill 父链把 viewport → root CB → 子 content box。
    ///
    /// 与 [`LayoutStyle::resolve_content_box`] 一致；稳定时不 bump。
    pub fn sync_layout_containing_blocks(&mut self, viewport: ParentBox) {
        let mut viewport_changed = false;
        if let (Some(w), Some(h)) = (viewport.width, viewport.height) {
            let next = Some((w, h));
            if self.layout_viewport != next {
                self.layout_viewport = next;
                viewport_changed = true;
            }
        }
        let roots = self.roots.clone();
        if roots.is_empty() {
            return;
        }
        let mut changed = false;
        let vp = self.layout_viewport;
        for root in roots {
            if self.write_containing_block(root, viewport.width, viewport.height) {
                changed = true;
            }
            self.propagate_layout_containing_blocks(root, vp, &mut changed);
        }
        // Re-cascade after CB writeback so % / vh resolve against fresh bases.
        if viewport_changed {
            self.reapply_layout_cascade_all();
        } else if changed {
            self.bump();
        }
    }

    /// Resolve the pre-paint document geometry from the canonical semantic tree.
    ///
    /// Headless entry for first insert and for nodes Iced has not painted yet.
    /// Painted Iced probe boxes stay authoritative for those nodes.
    pub(crate) fn resolve_document_layout(&mut self, doc: &mut crate::tree::NanaTreeDocument) {
        let (logical_w, logical_h) = doc.logical_size();
        self.reparent_orphans();
        self.sync_sidebar_footer_into_document(doc);
        self.sync_layout_containing_blocks(ParentBox::from_viewport(logical_w, logical_h));
        let boxes = crate::measure_bridge_layout_boxes(self, logical_w, logical_h);
        doc.apply_layout_boxes(&boxes);
    }

    /// Measure only nodes that still have no Runtime layout box.
    pub(crate) fn resolve_missing_document_layout(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        let (logical_w, logical_h) = doc.logical_size();
        self.reparent_orphans();
        self.sync_sidebar_footer_into_document(doc);
        self.sync_layout_containing_blocks(ParentBox::from_viewport(logical_w, logical_h));
        let boxes = crate::measure_bridge_layout_boxes(self, logical_w, logical_h);
        let missing: Vec<_> = boxes
            .into_iter()
            .filter(|(handle, _)| doc.layout_box(*handle).is_none())
            .collect();
        if !missing.is_empty() {
            doc.apply_layout_boxes(&missing);
        }
    }

    fn write_containing_block(
        &mut self,
        id: WidgetId,
        width: Option<f32>,
        height: Option<f32>,
    ) -> bool {
        let Some(w) = self.widgets.get_mut(&id) else {
            return false;
        };
        let next_w = width.filter(|v| *v > 0.0);
        let next_h = height.filter(|v| *v > 0.0);
        if w.props.containing_block_width == next_w && w.props.containing_block_height == next_h {
            return false;
        }
        w.props.containing_block_width = next_w;
        w.props.containing_block_height = next_h;
        true
    }

    fn propagate_layout_containing_blocks(
        &mut self,
        id: WidgetId,
        viewport: Option<(f32, f32)>,
        changed: &mut bool,
    ) {
        let (content, children) = {
            let Some(widget) = self.widgets.get(&id) else {
                return;
            };
            let parent = ParentBox {
                width: widget.props.containing_block_width,
                height: widget.props.containing_block_height,
            };
            let content = widget
                .props
                .layout
                .resolve_content_box_with_viewport(parent, viewport);
            (content, widget.children.clone())
        };
        for child in children {
            if self.write_containing_block(child, content.width, content.height) {
                *changed = true;
            }
            self.propagate_layout_containing_blocks(child, viewport, changed);
        }
    }

    /// 用父节点 content box（Fill 链友好）写入子节点的包含块基。
    fn sync_containing_block_from_parent(&mut self, id: WidgetId) {
        let parent_id = match self.widgets.get(&id).and_then(|w| w.parent) {
            Some(p) => p,
            None => return,
        };
        let (cw, ch) = self.estimate_content_box(parent_id);
        let _ = self.write_containing_block(id, cw, ch);
    }

    fn estimate_content_box(&self, id: WidgetId) -> (Option<f32>, Option<f32>) {
        let Some(widget) = self.widgets.get(&id) else {
            return (None, None);
        };
        // Match iced `resolve_content_box` so Fill/grow parents pass viewport size.
        let parent = ParentBox {
            width: widget.props.containing_block_width,
            height: widget.props.containing_block_height,
        };
        let content = widget
            .props
            .layout
            .resolve_content_box_with_viewport(parent, self.layout_viewport);
        (content.width, content.height)
    }

    fn mount_body_id(&self) -> Option<WidgetId> {
        self.widgets.iter().find_map(|(&id, w)| {
            w.props
                .class_names
                .iter()
                .any(|c| c == "nana-mount-root")
                .then_some(id)
        })
    }

    /// Attach unreachable sidebar shells under a stable workspace shell so iced
    /// paints them.
    ///
    /// At most **one** orphan sidebar is reparented. Remount leftovers used to pile
    /// multiple `SidebarFrame`s into the row and starve the primary column width.
    ///
    /// Parent preference (first match wins):
    /// 1. `nana-workspace-shell__body` (nanavue DesktopShell contract)
    /// 2. Documented region content (`nana-workspace-region__content` under a
    ///    resources region via `data-region-role` / `agent_id`)
    /// 3. Resources region host (`data-region-role` / `agent_id`)
    ///
    /// Never steal onto a bare `flex-row` without workspace identity.
    pub fn reparent_orphans(&mut self) {
        if !self.scaffolded {
            return;
        }
        let Some(workspace_row) = self.find_sidebar_reparent_host() else {
            self.reparent_sidebar_footer_slots();
            return;
        };
        let already_has_sidebar = self.widgets.get(&workspace_row).is_some_and(|row| {
            row.children.iter().any(|cid| {
                self.widgets
                    .get(cid)
                    .is_some_and(|w| matches!(w.kind, WidgetKind::SidebarFrame))
            })
        });
        if already_has_sidebar {
            self.reparent_sidebar_footer_slots();
            return;
        }
        let mut reachable = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_reachable(root, &mut reachable);
        }
        let mut orphans: Vec<(WidgetId, usize)> = self
            .widgets
            .iter()
            .filter_map(|(&id, w)| {
                if id <= 2 || reachable.contains(&id) || self.roots.contains(&id) {
                    return None;
                }
                matches!(w.kind, WidgetKind::SidebarFrame).then_some((id, w.children.len()))
            })
            .collect();
        // Prefer the densest sidebar shell (real nav over empty remount leftovers).
        orphans.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        let Some((id, _)) = orphans.into_iter().next() else {
            self.reparent_sidebar_footer_slots();
            return;
        };
        if id == workspace_row {
            self.reparent_sidebar_footer_slots();
            return;
        }
        let mut seen = std::collections::HashSet::new();
        self.collect_reachable(id, &mut seen);
        if seen.contains(&workspace_row) {
            self.reparent_sidebar_footer_slots();
            return;
        }
        // Insert before the first existing child so the sidebar stays left of Primary.
        let anchor = self
            .widgets
            .get(&workspace_row)
            .and_then(|r| r.children.first().copied());
        self.insert_child(id, workspace_row, anchor);
        // Workspace-row fallback skips ResourcePanel's height:100% content host.
        // Re-seed CB from the layout viewport so Fill / overflow-y scrollports
        // resolve to a finite height instead of iced Fill→0 under auto parents.
        if let Some((vw, vh)) = self.layout_viewport {
            self.sync_layout_containing_blocks(ParentBox::from_viewport(vw, vh));
        }
        self.reparent_sidebar_footer_slots();
        self.bump();
    }

    /// Reattach orphaned `nana-sidebar-frame__footer` slots (and their content)
    /// under the live reachable [`WidgetKind::SidebarFrame`].
    ///
    /// Remount + stale wrapNode insert targets used to detach footer columns
    /// from the frame while leaving top/body intact. Heal the slot contract so
    /// iced paints the fixed footer again.
    pub fn reparent_sidebar_footer_slots(&mut self) {
        if !self.scaffolded {
            return;
        }
        let reachable = self.roots_reachable();
        let Some(frame_id) = self.reachable_sidebar_frame(&reachable) else {
            return;
        };
        let has_footer_slot = self.widgets.get(&frame_id).is_some_and(|frame| {
            frame.children.iter().any(|cid| {
                self.widgets
                    .get(cid)
                    .is_some_and(|c| is_sidebar_footer_slot(&c.props))
            })
        });
        if !has_footer_slot {
            let footer_orphans: Vec<(WidgetId, usize, u64)> = self
                .widgets
                .iter()
                .filter_map(|(&id, w)| {
                    if reachable.contains(&id) || id <= 2 || !is_sidebar_footer_slot(&w.props) {
                        return None;
                    }
                    // Prefer densest / newest leftover from the latest remount.
                    Some((id, w.children.len(), id))
                })
                .collect();
            if let Some(footer_id) = prefer_dense_newest(footer_orphans) {
                self.insert_child(footer_id, frame_id, None);
            }
        }
        let Some(footer_id) = self.widgets.get(&frame_id).and_then(|frame| {
            frame.children.iter().copied().find(|cid| {
                self.widgets
                    .get(cid)
                    .is_some_and(|c| is_sidebar_footer_slot(&c.props))
            })
        }) else {
            return;
        };
        if self
            .widgets
            .get(&footer_id)
            .is_some_and(|f| !f.children.is_empty())
        {
            return;
        }
        // Content often sits on an orphan div that still hosts sidebar.footer.*
        // actions after the slot column was emptied by a failed re-insert.
        let content_orphans: Vec<(WidgetId, usize, u64)> = self
            .widgets
            .iter()
            .filter_map(|(&id, w)| {
                if reachable.contains(&id) || id == footer_id || id <= 2 {
                    return None;
                }
                if w.parent.is_some_and(|p| self.widgets.contains_key(&p)) {
                    return None;
                }
                hosts_sidebar_footer_content(w, &self.widgets).then_some((id, w.children.len(), id))
            })
            .collect();
        if let Some(content_id) = prefer_dense_newest(content_orphans) {
            self.insert_child(content_id, footer_id, None);
        }
    }

    /// Mirror bridge footer parenting into the document tree (shared id space).
    pub fn sync_sidebar_footer_into_document(&self, doc: &mut crate::tree::NanaTreeDocument) {
        let reachable = self.roots_reachable();
        let Some(frame_id) = self.reachable_sidebar_frame(&reachable) else {
            return;
        };
        let Some(frame) = self.widgets.get(&frame_id) else {
            return;
        };
        for &cid in &frame.children {
            let Some(child) = self.widgets.get(&cid) else {
                continue;
            };
            if !is_sidebar_footer_slot(&child.props) {
                continue;
            }
            doc.insert(
                crate::tree::NodeHandle(cid),
                crate::tree::NodeHandle(frame_id),
                None,
            );
            for &gcid in &child.children {
                doc.insert(
                    crate::tree::NodeHandle(gcid),
                    crate::tree::NodeHandle(cid),
                    None,
                );
            }
        }
    }

    fn roots_reachable(&self) -> std::collections::HashSet<WidgetId> {
        let mut reachable = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_reachable(root, &mut reachable);
        }
        reachable
    }

    fn reachable_sidebar_frame(
        &self,
        reachable: &std::collections::HashSet<WidgetId>,
    ) -> Option<WidgetId> {
        self.widgets.iter().find_map(|(&id, w)| {
            (reachable.contains(&id) && matches!(w.kind, WidgetKind::SidebarFrame)).then_some(id)
        })
    }

    fn find_sidebar_reparent_host(&self) -> Option<WidgetId> {
        let mut reachable = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_reachable(root, &mut reachable);
        }
        let reachable = reachable;

        // 1) nanavue NanaWorkspaceShell body
        if let Some(id) = self.widgets.iter().find_map(|(&id, w)| {
            (reachable.contains(&id)
                && w.kind == WidgetKind::Row
                && w.props
                    .class_names
                    .iter()
                    .any(|c| c == "nana-workspace-shell__body"))
            .then_some(id)
        }) {
            return Some(id);
        }
        let is_resources_shell = |w: &SemanticWidget| {
            w.props.region.eq_ignore_ascii_case("resources")
                || w.props.agent_id == "workspace.region.sidebar"
                || w.props.agent_id == "workspace.region.resources"
                || w.props
                    .attrs
                    .get("data-region-role")
                    .is_some_and(|r| r.eq_ignore_ascii_case("resources"))
        };
        // 2) reachable resources region content wrapper (height chain)
        if let Some(id) = self.widgets.iter().find_map(|(&id, w)| {
            if !reachable.contains(&id) {
                return None;
            }
            let is_content = w
                .props
                .class_names
                .iter()
                .any(|c| c == "nana-workspace-region__content");
            if !is_content {
                return None;
            }
            let parent_ok = w
                .parent
                .and_then(|p| self.widgets.get(&p))
                .is_some_and(is_resources_shell);
            parent_ok.then_some(id)
        }) {
            return Some(id);
        }
        // 3) reachable resources aside
        if let Some(id) = self
            .widgets
            .iter()
            .find_map(|(&id, w)| (reachable.contains(&id) && is_resources_shell(w)).then_some(id))
        {
            return Some(id);
        }
        // 4) Fallback: reachable workspace shell body (Row) when resources remount
        // left no content host — better a direct SidebarFrame sibling of Primary
        // than an invisible orphan.
        self.widgets.iter().find_map(|(&id, w)| {
            (reachable.contains(&id)
                && w.kind == WidgetKind::Row
                && w.props
                    .class_names
                    .iter()
                    .any(|c| c == "nana-workspace-shell__body"))
            .then_some(id)
        })
    }

    fn collect_reachable(&self, id: WidgetId, out: &mut std::collections::HashSet<WidgetId>) {
        if !out.insert(id) {
            return;
        }
        if let Some(w) = self.widgets.get(&id) {
            for &child in &w.children {
                self.collect_reachable(child, out);
            }
        }
    }

    pub fn patch_prop(&mut self, id: WidgetId, key: &str, value: &nana_js_engine::HostValue) {
        if key.starts_with("on") || key.starts_with("On") {
            return;
        }
        if !self.widgets.contains_key(&id) {
            return;
        }
        let key_n = normalize_prop_key(key);
        // Refresh CB from parent before style/gap `%` parse.
        if matches!(key_n.as_str(), "style" | "gap" | "padding") {
            self.sync_containing_block_from_parent(id);
        }
        let prev_kind = self
            .widgets
            .get(&id)
            .map(|w| w.kind)
            .unwrap_or(WidgetKind::Column);
        {
            let Some(widget) = self.widgets.get_mut(&id) else {
                return;
            };
            widget.props.apply_prop(key, value);
            // Overlays use `active || toggled` as open. Vue may patch only one side
            // (`active` / `open` / `selected` / `toggled` / `model-value` / aria-*);
            // keep both in sync so host dismiss and v-model close actually collapse
            // (mirrors note_toggle). `selected` alone must not leave toggled stuck.
            if widget.kind.is_overlay() {
                let sync_open = match key_n.as_str() {
                    "active" | "open" | "selected" | "aria-selected" | "aria-pressed"
                    | "aria-expanded" => Some(widget.props.active),
                    "toggled" | "model-value" if host_is_open_flag(value) => {
                        Some(widget.props.toggled)
                    }
                    "aria-modal" => {
                        apply_overlay_presence_open(&mut widget.props);
                        None
                    }
                    _ => None,
                };
                if let Some(open) = sync_open {
                    widget.props.active = open;
                    widget.props.toggled = open;
                }
            }
            // Re-resolve kind from class / role / type after attribute patches.
            if matches!(
                key_n.as_str(),
                "class"
                    | "classname"
                    | "role"
                    | "type"
                    | "aria-pressed"
                    | "aria-selected"
                    | "aria-modal"
                    | "aria-expanded"
                    | "style"
                    | "flex-direction"
                    | "flexdirection"
                    | "gap"
                    | "padding"
                    | "width"
                    | "height"
                    | "id"
                    | "data-region-id"
            ) {
                let class = widget.props.class_names.join(" ");
                let role = widget.props.role.clone();
                let input_type = if key_n == "type" {
                    host_string(value)
                } else {
                    String::new()
                };
                if let Some(next) =
                    resolve_kind_from_hints("div", Some(&class), Some(&role), Some(&input_type))
                {
                    let allow = next != prev_kind && (!next.is_layout() || prev_kind.is_layout());
                    if allow {
                        widget.kind = next;
                        if next.is_overlay() && !prev_kind.is_overlay() {
                            apply_overlay_presence_open(&mut widget.props);
                        }
                    }
                }
                if widget.kind.is_overlay()
                    && matches!(
                        widget.props.layout.position,
                        crate::css_map::PositionSpec::Fixed | crate::css_map::PositionSpec::Sticky
                    )
                {
                    widget.props.layout.position = crate::css_map::PositionSpec::Static;
                }
            }
        }
        // Rebuild LayoutStyle from stylesheet + class hints + inline/prop style.
        let full_rebuild = matches!(
            key_n.as_str(),
            "class" | "classname" | "style" | "id" | "data-region-id" | "hidden"
        ) || key_n.starts_with("data-");
        let layout_prop = matches!(
            key_n.as_str(),
            "gap"
                | "padding"
                | "width"
                | "height"
                | "flex"
                | "flex-direction"
                | "flexdirection"
                | "flex-grow"
                | "flexgrow"
                | "min-width"
                | "minwidth"
                | "justify-content"
                | "justifycontent"
                | "overflow"
                | "overflow-y"
                | "overflowy"
                | "grid-template-columns"
                | "gridtemplatecolumns"
        );
        if full_rebuild {
            self.reapply_layout_for(id);
        } else if layout_prop {
            // Incremental: apply the prop declaration, then re-apply class hints
            // so public nana-* contracts still win over Vue layout props
            // (same layer order as rebuild_layout_style).
            if let Some(widget) = self.widgets.get_mut(&id) {
                let css = widget.props.prop_style.clone();
                let classes = widget.props.class_names.clone();
                let cb_w = widget.props.containing_block_width;
                let cb_h = widget.props.containing_block_height;
                if !css.is_empty() {
                    // Apply only the latest prop declaration for this key.
                    let key_css = key_n.replace("flexdirection", "flex-direction");
                    let key_css = key_css.replace("flexgrow", "flex-grow");
                    let key_css = key_css.replace("minwidth", "min-width");
                    let key_css = key_css.replace("justifycontent", "justify-content");
                    let key_css = key_css.replace("overflowy", "overflow-y");
                    let key_css = key_css.replace("gridtemplatecolumns", "grid-template-columns");
                    if let Some((_, val)) = css
                        .split(';')
                        .rev()
                        .map(str::trim)
                        .filter_map(|decl| decl.split_once(':'))
                        .find(|(key, _)| key.trim().eq_ignore_ascii_case(&key_css))
                    {
                        widget.props.layout.apply_css_property(
                            key_css.trim(),
                            val.trim(),
                            cb_w,
                            cb_h,
                        );
                    }
                }
                widget.props.layout.apply_class_layout_hints(&classes);
                pin_svg_chart_min_height(&mut widget.props);
                widget.kind = apply_display_to_kind(widget.kind, &widget.props.layout);
            }
        } else if let Some(widget) = self.widgets.get_mut(&id) {
            pin_svg_chart_min_height(&mut widget.props);
            widget.kind = apply_display_to_kind(widget.kind, &widget.props.layout);
        }
        // Parent size/padding change updates children's containing-block base.
        if matches!(key_n.as_str(), "style" | "width" | "height" | "padding") {
            let children = self
                .widgets
                .get(&id)
                .map(|w| w.children.clone())
                .unwrap_or_default();
            for child in children {
                self.sync_containing_block_from_parent(child);
            }
        }
        self.strip_deferred_position_on_overlay(id);
        self.bump();
    }

    pub fn set_label(&mut self, id: WidgetId, label: impl Into<String>) {
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.label = label.into();
            self.bump();
        }
    }

    pub fn push_event(&mut self, event: BridgeEvent) {
        self.pending.push_back(event);
    }

    pub fn drain_events(&mut self) -> Vec<BridgeEvent> {
        self.pending.drain(..).collect()
    }

    pub fn peek_events(&self) -> impl Iterator<Item = &BridgeEvent> {
        self.pending.iter()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn note_press(&mut self, id: WidgetId) -> Vec<&'static str> {
        let kind = match self.widgets.get(&id).map(|w| w.kind) {
            Some(k) => k,
            None => return Vec::new(),
        };
        match kind {
            WidgetKind::Button | WidgetKind::Chip => {
                self.push_event(BridgeEvent::Press { id });
                vec!["press", "click"]
            }
            WidgetKind::SidebarRow | WidgetKind::ListItem | WidgetKind::InteractiveCard => {
                self.push_event(BridgeEvent::Select { id });
                vec!["select", "click"]
            }
            _ => {
                self.push_event(BridgeEvent::Press { id });
                vec!["click"]
            }
        }
    }

    pub fn note_toggle(&mut self, id: WidgetId, value: bool) -> Vec<&'static str> {
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.toggled = value;
            // Overlays treat `active || toggled` as open; keep both in sync so
            // dismiss (close / outside / cancel) actually collapses when opened
            // via `active` / `open`.
            if w.kind.is_overlay() {
                w.props.active = value;
            }
            self.bump();
        } else {
            return Vec::new();
        }
        self.push_event(BridgeEvent::Toggle { id, value });
        vec!["change", "update:modelValue"]
    }

    pub fn note_select(&mut self, id: WidgetId) -> Vec<&'static str> {
        if !self.widgets.contains_key(&id) {
            return Vec::new();
        }
        self.push_event(BridgeEvent::Select { id });
        vec!["select", "click"]
    }

    pub fn note_select_value(
        &mut self,
        id: WidgetId,
        value: impl Into<String>,
    ) -> Vec<&'static str> {
        let value = value.into();
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.value = value.clone();
            if w.kind.is_overlay() {
                // Confirm / Drawer footer / menu item selection closes the overlay.
                w.props.active = false;
                w.props.toggled = false;
            } else {
                w.props.active = true;
            }
            self.bump();
        } else {
            return Vec::new();
        }
        self.push_event(BridgeEvent::SelectValue { id, value });
        vec!["select", "update:modelValue", "change"]
    }

    pub fn note_input(&mut self, id: WidgetId, value: impl Into<String>) -> Vec<&'static str> {
        let value = value.into();
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.value = value.clone();
            w.props.label = value.clone();
            self.bump();
        } else {
            return Vec::new();
        }
        self.push_event(BridgeEvent::Input { id, value });
        vec!["input", "update:modelValue"]
    }

    pub fn note_change(&mut self, id: WidgetId, value: f64) -> Vec<&'static str> {
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.number = value as f32;
            w.props.progress = value as f32;
            w.props.value = value.to_string();
            self.bump();
        } else {
            return Vec::new();
        }
        self.push_event(BridgeEvent::Change { id, value });
        vec!["change", "update:modelValue"]
    }

    pub fn snapshot(&self) -> SemanticSnapshot {
        let mut widgets = Vec::with_capacity(self.widgets.len());
        let mut seen = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_preorder(root, &mut widgets, &mut seen);
        }
        for (&id, widget) in &self.widgets {
            if seen.insert(id) {
                widgets.push(widget.clone());
            }
        }
        SemanticSnapshot {
            revision: self.revision,
            theme: self.theme,
            appearance: self.appearance,
            roots: self.roots.clone(),
            widgets,
        }
    }

    /// Map a tree tag to a widget kind (`nana-*` or HTML downlevel).
    pub fn kind_from_tag(tag: &str) -> Option<WidgetKind> {
        resolve_kind_from_hints(tag, None, None, None)
    }

    fn collect_preorder(
        &self,
        id: WidgetId,
        out: &mut Vec<SemanticWidget>,
        seen: &mut std::collections::HashSet<WidgetId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        let Some(widget) = self.widgets.get(&id).cloned() else {
            return;
        };
        let children = widget.children.clone();
        out.push(widget);
        for child in children {
            self.collect_preorder(child, out, seen);
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

/// Structural `<svg viewBox>` with author `height: Npx`: raise `min-height` so
/// column flex-shrink cannot crush chart geometry (heatmap weekday rows).
/// Horizontal crop stays with overflow:hidden + EndCrop — do not pin min-width.
/// `overflow-y: hidden` keeps CSS min-size:auto → 0 (may shrink).
fn pin_svg_chart_min_height(props: &mut WidgetProps) {
    if !props.element_tag.eq_ignore_ascii_case("svg") {
        return;
    }
    if props.layout.overflow_y.clips() {
        return;
    }
    let has_view_box = props.attrs.keys().any(|k| {
        let n: String = k
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .flat_map(|c| c.to_lowercase())
            .collect();
        n == "viewbox"
    });
    if !has_view_box {
        return;
    }
    let Some(LengthSpec::Px(h)) = props.layout.height else {
        return;
    };
    if !h.is_finite() || h <= 0.0 {
        return;
    }
    let raise = match props.layout.min_height {
        None => true,
        Some(LengthSpec::Px(mh)) => mh + 0.5 < h,
        _ => false,
    };
    if raise {
        props.layout.min_height = Some(LengthSpec::Px(h));
    }
}

fn is_sidebar_footer_slot(props: &WidgetProps) -> bool {
    props
        .class_names
        .iter()
        .any(|c| c == "nana-sidebar-frame__footer")
        || props
            .attrs
            .get("data-slot")
            .is_some_and(|s| s == "sidebar-footer")
}

fn hosts_sidebar_footer_content(
    w: &SemanticWidget,
    widgets: &std::collections::HashMap<WidgetId, SemanticWidget>,
) -> bool {
    w.props.class_names.iter().any(|c| c == "sb-footer")
        || w.props.agent_id.starts_with("sidebar.footer.")
        || w.children.iter().any(|cid| {
            widgets.get(cid).is_some_and(|c| {
                c.props.agent_id.starts_with("sidebar.footer.")
                    || c.props
                        .class_names
                        .iter()
                        .any(|cls| cls == "sb-footer" || cls.starts_with("sb-footer__"))
            })
        })
}

fn prefer_dense_newest(mut items: Vec<(WidgetId, usize, u64)>) -> Option<WidgetId> {
    items.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    items.into_iter().next().map(|(id, _, _)| id)
}

/// Convenience: handle as widget id.
pub fn widget_id(handle: NodeHandle) -> WidgetId {
    handle.0
}

fn normalize_prop_key(key: &str) -> String {
    // Vue `.prop` / `^attr` modifiers arrive on the host key; strip first.
    let key = key.trim();
    let key = key
        .strip_prefix('.')
        .or_else(|| key.strip_prefix('^'))
        .unwrap_or(key);
    let key = key.replace('_', "-");
    let kebab = if key.chars().any(|c| c.is_ascii_uppercase()) {
        camel_to_kebab_simple(&key)
    } else {
        key.to_string()
    };
    kebab.to_ascii_lowercase()
}

/// Common SVG attrs after [`normalize_prop_key`] (kebab / lowercase).
fn is_common_svg_attr(key: &str) -> bool {
    matches!(
        key,
        "viewbox"
            | "view-box"
            | "preserveaspectratio"
            | "preserve-aspect-ratio"
            | "pathlength"
            | "path-length"
            | "cx"
            | "cy"
            | "r"
            | "rx"
            | "ry"
            | "x"
            | "y"
            | "x1"
            | "x2"
            | "y1"
            | "y2"
            | "points"
            | "transform"
            | "opacity"
            | "stroke-width"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "fill-opacity"
            | "stroke-opacity"
            | "fill-rule"
            | "clip-path"
            | "href"
            | "xmlns"
            | "d"
            | "fill"
            | "stroke"
    )
}

fn camel_to_kebab_simple(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    for (i, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn host_string(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined => String::new(),
        nana_js_engine::HostValue::Bool(v) => v.to_string(),
        nana_js_engine::HostValue::Number(v) => {
            if v.fract() == 0.0 && v.is_finite() {
                format!("{}", *v as i64)
            } else {
                v.to_string()
            }
        }
        nana_js_engine::HostValue::String(v) => v.clone(),
        nana_js_engine::HostValue::Object(map) => {
            // Vue sometimes passes option/chip props as objects; prefer human labels.
            for key in ["label", "name", "title", "text", "value", "id"] {
                if let Some(v) = map.get(key) {
                    let s = host_string(v);
                    if !s.is_empty() && s != "[object Object]" {
                        return s;
                    }
                }
            }
            String::new()
        }
        nana_js_engine::HostValue::Array(items) => items
            .iter()
            .map(host_string)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        other => {
            let s = other.to_json_string();
            if s == "[object Object]" || s.starts_with('{') {
                String::new()
            } else {
                s
            }
        }
    }
}

#[derive(Debug, Clone)]
struct MatchNodeSnap {
    tag: String,
    id: String,
    classes: Vec<String>,
    attrs: BTreeMap<String, String>,
}

fn host_style_to_css_text(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::Object(map) => {
            let mut out = String::with_capacity(map.len().saturating_mul(24));
            for (key, v) in map {
                let prop = if key.starts_with("--") || key.contains('-') {
                    key.clone()
                } else {
                    camel_to_kebab_simple(key)
                };
                let val = host_string(v);
                if prop.is_empty() || val.is_empty() {
                    continue;
                }
                out.push_str(&prop);
                out.push(':');
                out.push_str(&val);
                out.push(';');
            }
            out
        }
        _ => host_string(value),
    }
}

/// True when a CSS declaration list mentions `property` (case-insensitive).
fn css_decl_mentions(style: &str, property: &str) -> bool {
    let want = property.trim();
    if want.is_empty() || style.trim().is_empty() {
        return false;
    }
    for decl in style.split(';') {
        let decl = decl.trim();
        let Some((name, _)) = decl.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(want) {
            return true;
        }
    }
    false
}

fn host_truthy(value: &nana_js_engine::HostValue) -> bool {
    match value {
        nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined => false,
        nana_js_engine::HostValue::Bool(v) => *v,
        nana_js_engine::HostValue::Number(n) => *n != 0.0,
        nana_js_engine::HostValue::String(s) => {
            let s = s.trim();
            !(s.is_empty() || s.eq_ignore_ascii_case("false") || s == "0")
        }
        _ => true,
    }
}

/// Open/close flag for overlay props — not a select / confirm string `model-value`.
fn host_is_open_flag(value: &nana_js_engine::HostValue) -> bool {
    match value {
        nana_js_engine::HostValue::Bool(_) | nana_js_engine::HostValue::Number(_) => true,
        nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined => true,
        nana_js_engine::HostValue::String(s) => {
            let s = s.trim();
            s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false")
        }
        _ => false,
    }
}

/// Host Teleport / `v-if` surfaces mount only while open and often omit `open`/`active`.
/// Presence cues (`aria-modal`, `is-open` / `is-active`, `data-nana-open`) imply Nana
/// Overlay open — without inventing CSS `fixed`/`sticky`. Explicit `open`/`active`/
/// `toggled` still win via later patches. Do **not** key off bare `nana-*` / `role`
/// alone — closed Nana* wrappers stay mounted with those hints. Do **not** key off
/// product kit BEM (`ui-dialog`, `ctx-menu`, …).
fn apply_overlay_presence_open(props: &mut WidgetProps) {
    if props.active || props.toggled {
        return;
    }
    if overlay_presence_implies_open(props) {
        props.active = true;
        props.toggled = true;
    }
}

fn overlay_presence_implies_open(props: &WidgetProps) -> bool {
    props.class_names.iter().any(|class| {
        let class = class.to_ascii_lowercase();
        class == "is-open" || class == "is-active"
    }) || props
        .attrs
        .get("aria-expanded")
        .is_some_and(|expanded| expanded.eq_ignore_ascii_case("true"))
        || props.attrs.get("aria-modal").is_some_and(|modal| {
            // Empty string = boolean true attribute; "true" likewise.
            modal.is_empty() || modal.eq_ignore_ascii_case("true")
        })
        || props
            .attrs
            .get("data-nana-open")
            .is_some_and(|open| open.is_empty() || open.eq_ignore_ascii_case("true"))
        || props.attrs.get("open").is_some_and(|open| {
            open.is_empty()
                || open.eq_ignore_ascii_case("true")
                || open.eq_ignore_ascii_case("open")
        })
}

fn encode_qr_modules_attr(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::Array(items) => items
            .iter()
            .map(|item| {
                if matches!(item, nana_js_engine::HostValue::Number(n) if *n != 0.0)
                    || host_truthy(item)
                    || host_string(item) == "1"
                {
                    "1"
                } else {
                    "0"
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        _ => host_string(value),
    }
}

fn host_f32(value: &nana_js_engine::HostValue, default: f32) -> f32 {
    match value {
        nana_js_engine::HostValue::Number(n) if n.is_finite() => *n as f32,
        nana_js_engine::HostValue::String(s) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

fn parse_option_item(item: &nana_js_engine::HostValue) -> Option<SelectOptionProp> {
    match item {
        nana_js_engine::HostValue::Object(map) => {
            let value = map
                .get("value")
                .or_else(|| map.get("key"))
                .map(host_string)
                .unwrap_or_default();
            let label = map
                .get("label")
                .map(host_string)
                .filter(|s| !s.is_empty() && s != "[object Object]")
                .unwrap_or_else(|| value.clone());
            let disabled = map.get("disabled").map(host_truthy).unwrap_or(false);
            if (value.is_empty() && label.is_empty())
                || value == "[object Object]"
                || label == "[object Object]"
            {
                None
            } else {
                Some(SelectOptionProp {
                    value,
                    label,
                    disabled,
                })
            }
        }
        nana_js_engine::HostValue::String(s) => {
            if s.is_empty() || s == "[object Object]" {
                None
            } else {
                Some(SelectOptionProp {
                    value: s.clone(),
                    label: s.clone(),
                    disabled: false,
                })
            }
        }
        _ => None,
    }
}

fn parse_options(value: &nana_js_engine::HostValue) -> Vec<SelectOptionProp> {
    match value {
        nana_js_engine::HostValue::Array(items) => {
            items.iter().filter_map(parse_option_item).collect()
        }
        nana_js_engine::HostValue::Object(map) => {
            // Numeric-key object (reactive array shape) → options list.
            let mut indexed = map
                .iter()
                .filter_map(|(k, v)| k.parse::<usize>().ok().map(|i| (i, v)))
                .collect::<Vec<_>>();
            if !indexed.is_empty() {
                indexed.sort_by_key(|(i, _)| *i);
                return indexed
                    .into_iter()
                    .filter_map(|(_, v)| parse_option_item(v))
                    .collect();
            }
            parse_option_item(value).into_iter().collect()
        }
        nana_js_engine::HostValue::String(s) => {
            // Reject Array.prototype.toString of object options.
            if s.contains("[object Object]") {
                return Vec::new();
            }
            // Comma-separated "value:label" or bare values.
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty() && *p != "[object Object]")
                .map(|part| {
                    if let Some((v, l)) = part.split_once(':') {
                        SelectOptionProp {
                            value: v.trim().to_string(),
                            label: l.trim().to_string(),
                            disabled: false,
                        }
                    } else {
                        SelectOptionProp {
                            value: part.to_string(),
                            label: part.to_string(),
                            disabled: false,
                        }
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub fn parse_button_kind(raw: &str) -> Option<ButtonKind> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "ghost" => ButtonKind::Ghost,
        "subtle" => ButtonKind::Subtle,
        "selected" => ButtonKind::Selected,
        "primary" => ButtonKind::Primary,
        "warning" => ButtonKind::Warning,
        "danger" => ButtonKind::Danger,
        "text" => ButtonKind::Text,
        _ => return None,
    })
}

pub fn parse_card_kind(raw: &str) -> Option<CardKind> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "surface" => CardKind::Surface,
        "outlined" | "outline" => CardKind::Outlined,
        "raised" | "elevated" => CardKind::Raised,
        "flat" => CardKind::Flat,
        "selected" => CardKind::Selected,
        _ => return None,
    })
}

pub fn parse_control_size(raw: &str) -> Option<ControlSize> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "small" | "sm" => ControlSize::Small,
        "medium" | "md" => ControlSize::Medium,
        "large" | "lg" => ControlSize::Large,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_map::JustifySpec;
    use nana_js_engine::HostValue;
    use std::collections::BTreeMap;

    #[test]
    fn measure_layout_boxes_place_row_children() {
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        let mut row = WidgetProps::default();
        row.layout.apply_css_text(
            "display:flex;flex-direction:row;gap:8px;width:200px;height:40px",
            None,
            None,
        );
        bridge.register(3, WidgetKind::Row, row);
        bridge.insert_child(3, 2, None);
        let mut a = WidgetProps::default();
        a.layout.width = Some(LengthSpec::Px(40.0));
        a.layout.height = Some(LengthSpec::Px(20.0));
        bridge.register(4, WidgetKind::Box, a);
        bridge.insert_child(4, 3, None);
        let mut b = WidgetProps::default();
        b.layout.width = Some(LengthSpec::Px(40.0));
        b.layout.height = Some(LengthSpec::Px(20.0));
        bridge.register(5, WidgetKind::Box, b);
        bridge.insert_child(5, 3, None);

        let root = {
            fn to_node(bridge: &MessageBridge, id: WidgetId) -> Option<crate::LayoutNode> {
                let w = bridge.get(id)?;
                let children = w
                    .children
                    .iter()
                    .filter_map(|&c| to_node(bridge, c))
                    .collect();
                Some(crate::LayoutNode::with_children(
                    id.to_string(),
                    w.props.layout.clone(),
                    children,
                ))
            }
            to_node(&bridge, 2).expect("body")
        };
        let boxes: std::collections::BTreeMap<_, _> = crate::measure_layout(&root, 400.0, 300.0)
            .into_iter()
            .collect();
        let a = boxes.get("4").expect("a");
        let b = boxes.get("5").expect("b");
        assert!(
            (b.x - a.x - a.width - 8.0).abs() < 0.5,
            "row gap should separate children"
        );
    }

    #[test]
    fn display_block_remains_column_when_stale_flex_direction_is_row() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());

        bridge.patch_prop(1, "flex-direction", &HostValue::string("row"));
        bridge.patch_prop(
            1,
            "style",
            &HostValue::string("display:block;flex-direction:row"),
        );

        assert_eq!(
            bridge.get(1).map(|widget| widget.kind),
            Some(WidgetKind::Column)
        );
    }

    #[test]
    fn create_button_and_press_queues_event() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            10,
            WidgetKind::Button,
            WidgetProps {
                label: "Increment".into(),
                button_kind: ButtonKind::Primary,
                ..WidgetProps::default()
            },
        );
        let events = bridge.note_press(10);
        assert_eq!(events, vec!["press", "click"]);
        let pending = bridge.drain_events();
        assert_eq!(pending, vec![BridgeEvent::Press { id: 10 }]);
    }

    #[test]
    fn column_insert_builds_tree_snapshot() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Text,
            WidgetProps {
                label: "0".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Button,
            WidgetProps {
                label: "inc".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        let snap = bridge.snapshot();
        assert_eq!(snap.roots, vec![1]);
        assert_eq!(snap.widgets[0].id, 1);
        assert_eq!(snap.widgets[1].props.label, "0");
        assert_eq!(snap.widgets[2].kind, WidgetKind::Button);
    }

    #[test]
    fn props_from_map_parses_button() {
        let mut map = BTreeMap::new();
        map.insert("label".into(), HostValue::string("Go"));
        map.insert("kind".into(), HostValue::string("primary"));
        map.insert("disabled".into(), HostValue::Bool(true));
        let props = WidgetProps::from_map(&map);
        assert_eq!(props.label, "Go");
        assert_eq!(props.button_kind, ButtonKind::Primary);
        assert!(props.disabled);
    }

    #[test]
    fn theme_inject_bumps_revision() {
        let mut bridge = MessageBridge::new();
        let r0 = bridge.revision();
        bridge.set_theme(ThemeMode::Dark);
        assert_eq!(bridge.theme(), ThemeMode::Dark);
        assert!(bridge.revision() > r0);
        assert_eq!(bridge.theme_label(), "dark");
    }

    #[test]
    fn document_appearance_syncs_backdrop_fields() {
        let mut bridge = MessageBridge::new();
        let mut dataset = BTreeMap::new();
        dataset.insert("backdrop".into(), "translucent".into());
        dataset.insert("backdropTarget".into(), "main".into());
        dataset.insert("titlebarFollowsSidebar".into(), "false".into());
        let mut style = BTreeMap::new();
        style.insert("--nana-backdrop-opacity".into(), "0.5".into());
        bridge.apply_document_appearance(&dataset, &style);
        let appearance = bridge.appearance();
        assert_eq!(
            appearance.window_material(),
            WindowMaterialMode::Translucent
        );
        assert_eq!(appearance.backdrop_target(), BackdropTarget::Main);
        assert!(!appearance.titlebar_follows_sidebar());
        assert!((appearance.backdrop_opacity() - 0.5).abs() < f32::EPSILON);
        let snap = bridge.snapshot();
        assert_eq!(snap.appearance.backdrop_target(), BackdropTarget::Main);
    }

    #[test]
    fn document_appearance_syncs_theme_from_dataset() {
        let mut bridge = MessageBridge::new();
        assert_eq!(bridge.theme(), ThemeMode::Light);
        let mut dataset = BTreeMap::new();
        dataset.insert("theme".into(), "dark".into());
        bridge.apply_document_appearance(&dataset, &BTreeMap::new());
        assert_eq!(bridge.theme(), ThemeMode::Dark);
        assert_eq!(bridge.snapshot().theme, ThemeMode::Dark);
        dataset.insert("theme".into(), "light".into());
        bridge.apply_document_appearance(&dataset, &BTreeMap::new());
        assert_eq!(bridge.theme(), ThemeMode::Light);
    }

    #[test]
    fn theme_change_reapplies_var_bg_from_data_theme_rules() {
        // Primary content uses `background: var(--bg)`. Document vars must track
        // theme — not stick on the light overlay from blind last-wins merge.
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::Column,
            WidgetProps {
                class_names: vec!["surface".into()],
                ..WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(
            r#"
            :root { --bg: #181818; }
            :root[data-theme="light"] { --bg: #ffffff; }
            .surface { background: var(--bg); width: 100px; height: 40px; }
            "#,
        );
        let light_bg = bridge.get(1).unwrap().props.layout.background;
        assert_eq!(
            light_bg,
            Some([1.0, 1.0, 1.0, 1.0]),
            "default ThemeMode::Light must resolve light --bg"
        );

        let mut dataset = BTreeMap::new();
        dataset.insert("theme".into(), "dark".into());
        bridge.apply_document_appearance(&dataset, &BTreeMap::new());
        let dark_bg = bridge.get(1).unwrap().props.layout.background;
        assert_eq!(
            dark_bg,
            Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
            "dark theme must drop light overlay and keep :root --bg"
        );

        dataset.insert("theme".into(), "light".into());
        bridge.apply_document_appearance(&dataset, &BTreeMap::new());
        assert_eq!(
            bridge.get(1).unwrap().props.layout.background,
            Some([1.0, 1.0, 1.0, 1.0])
        );
    }

    #[test]
    fn lightningcss_companion_tokens_paint_light_under_html_scaffold() {
        // Real LiliaGithub companion shape under html/body scaffold (orphans
        // previously rematched bare :root and stuck on #181818/#202020).
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        assert_eq!(
            bridge
                .get(1)
                .unwrap()
                .props
                .attrs
                .get("data-theme")
                .map(String::as_str),
            Some("light")
        );
        let mut props = WidgetProps::default();
        props.class_names = vec!["surface".into()];
        props.element_tag = "div".into();
        bridge.register(3, WidgetKind::Column, props);
        bridge.insert_child(3, 2, None);
        bridge.inject_stylesheet(
            r#"
            :root{--bg:#181818;--bg-elev:#202020}
            @supports (color:lab(0% 0 0)){:root{--bg:lab(8.244% 0 0);--bg-elev:lab(12% 0 0)}}
            :root[data-theme=light]{--bg:#fff;--bg-elev:#f3f4f6}
            @supports (color:lab(0% 0 0)){:root[data-theme=light]{--bg:lab(100% 0 0)}}
            .surface{background:var(--bg);width:100px;height:40px}
            .raised{background:var(--bg-elev);width:100px;height:40px}
            "#,
        );
        assert_eq!(
            bridge.get(3).unwrap().props.layout.background,
            Some([1.0, 1.0, 1.0, 1.0]),
            "scaffold + lightningcss light tokens must paint white --bg"
        );
        let mut raised = WidgetProps::default();
        raised.class_names = vec!["raised".into()];
        raised.element_tag = "div".into();
        bridge.register(4, WidgetKind::Column, raised);
        bridge.insert_child(4, 2, None);
        // Class may arrive after register in Vue; re-inject is the host path that
        // rebuilds cascade for late nodes — here a no-op stylesheet bump.
        bridge.inject_stylesheet(".raised{background:var(--bg-elev)}");
        assert_eq!(
            bridge.get(4).unwrap().props.layout.background,
            Some([243.0 / 255.0, 244.0 / 255.0, 246.0 / 255.0, 1.0]),
            "light --bg-elev must resolve (not dark #202020)"
        );
    }

    #[cfg(feature = "scene-view")]
    #[test]
    fn snapshot_theme_tokens_honor_backdrop_and_titlebar_follow() {
        use crate::theme_tokens_from_snapshot;

        let mut bridge = MessageBridge::new();
        let mut dataset = BTreeMap::new();
        dataset.insert("backdrop".into(), "translucent".into());
        dataset.insert("backdropTarget".into(), "sidebar".into());
        dataset.insert("titlebarFollowsSidebar".into(), "false".into());
        let mut style = BTreeMap::new();
        style.insert("--nana-backdrop-opacity".into(), "0.5".into());
        bridge.apply_document_appearance(&dataset, &style);
        let snap = bridge.snapshot();
        let tokens = theme_tokens_from_snapshot(&snap, true);
        assert!((tokens.colors.surface.a - 0.5).abs() < f32::EPSILON);
        assert!((tokens.titlebar.a - 1.0).abs() < f32::EPSILON);
        assert!((tokens.colors.background.a - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn stylesheet_cascade_drives_anonymous_class_layout() {
        let mut bridge = MessageBridge::new();
        bridge.inject_stylesheet(
            r#"
            .anon-shell { display:grid; grid-template-rows:minmax(0,1fr); height:100%; width:100%; overflow:hidden; }
            .anon-grid { display:flex; flex-direction:row; flex-wrap:wrap; gap:12px; height:100%; }
            .anon-card { padding:12px; border-radius:16px; }
            "#,
        );
        let mut shell = WidgetProps::default();
        shell.element_tag = "div".into();
        bridge.register(1, WidgetKind::Column, shell);
        bridge.patch_prop(1, "class", &HostValue::string("anon-shell"));
        let shell_layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(shell_layout.height, Some(LengthSpec::Fill));
        assert_eq!(shell_layout.width, Some(LengthSpec::Fill));
        assert!(
            shell_layout
                .grid_rows
                .as_ref()
                .is_some_and(|r| !r.is_empty())
        );

        let mut grid = WidgetProps::default();
        grid.element_tag = "div".into();
        bridge.register(2, WidgetKind::Column, grid);
        bridge.patch_prop(2, "class", &HostValue::string("anon-grid"));
        let grid_layout = &bridge.get(2).unwrap().props.layout;
        assert_eq!(grid_layout.direction, Some(FlexDirection::Row));
        assert_eq!(grid_layout.gap, Some(LengthSpec::Px(12.0)));
        assert_eq!(grid_layout.height, Some(LengthSpec::Fill));
        assert_eq!(bridge.get(2).unwrap().kind, WidgetKind::Row);

        // Late stylesheet injection re-applies onto existing nodes.
        bridge.inject_stylesheet(".anon-shell { padding: 20px; }");
        assert_eq!(
            bridge.get(1).unwrap().props.layout.padding,
            Some(LengthSpec::Px(20.0))
        );
    }

    #[test]
    fn html_and_class_downlevel_to_foundations() {
        assert_eq!(
            resolve_kind_from_hints("div", None, None, None),
            Some(WidgetKind::Column)
        );
        assert_eq!(
            resolve_kind_from_hints("button", None, None, None),
            Some(WidgetKind::Button)
        );
        assert_eq!(
            resolve_kind_from_hints("input", None, None, Some("checkbox")),
            Some(WidgetKind::Checkbox)
        );
        assert_eq!(
            resolve_kind_from_hints("div", Some("nana-tabs"), Some("tablist"), None),
            Some(WidgetKind::Tabs)
        );
        assert_eq!(
            resolve_kind_from_hints("button", Some("nana-chip is-selected"), None, None),
            Some(WidgetKind::Chip)
        );
        assert_eq!(
            resolve_kind_from_hints("li", None, None, None),
            Some(WidgetKind::ListItem)
        );
    }

    #[test]
    fn class_patch_upgrades_layout_to_chip() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "class", &HostValue::string("nana-chip"));
        assert_eq!(bridge.get(1).unwrap().kind, WidgetKind::Chip);
    }

    #[test]
    fn class_patch_does_not_demote_button_to_column() {
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.label = "搜索".into();
        bridge.register(1, WidgetKind::Button, props);
        // Re-resolve uses tag "div"; anonymous toolbar class must not demote Button.
        bridge.patch_prop(1, "class", &HostValue::string("anon-toolbar-btn"));
        bridge.patch_prop(
            1,
            "style",
            &HostValue::string("width:32px;height:32px;border-radius:6px"),
        );
        assert_eq!(bridge.get(1).unwrap().kind, WidgetKind::Button);
        assert_eq!(
            bridge.get(1).unwrap().props.layout.width,
            Some(LengthSpec::Px(32.0))
        );
    }

    #[test]
    fn style_patch_sets_gap_padding_and_flips_to_row() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(
            1,
            "style",
            &HostValue::string(
                "display:flex; flex-direction:row; gap:12px; padding:8px; width:100%",
            ),
        );
        let w = bridge.get(1).unwrap();
        assert_eq!(w.kind, WidgetKind::Row);
        assert_eq!(w.props.layout.gap, Some(LengthSpec::Px(12.0)));
        assert_eq!(w.props.layout.padding, Some(LengthSpec::Px(8.0)));
        assert_eq!(w.props.layout.width, Some(LengthSpec::Fill));
    }

    #[test]
    fn padding_prop_and_style_preserve_percent_without_containing_block() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "padding", &HostValue::string("10%"));
        assert_eq!(
            bridge.get(1).unwrap().props.layout.padding,
            Some(LengthSpec::Percent(10.0)),
            "padding prop must not drop % when percent base is unknown"
        );
        bridge.patch_prop(1, "style", &HostValue::string("margin:5%;padding-left:8%"));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.margin, Some(LengthSpec::Percent(5.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Percent(8.0)));
        let pad = layout.resolved_padding_against(Some(200.0));
        let margin = layout.resolved_margin_against(Some(200.0));
        assert_eq!(pad.left, 16.0);
        assert_eq!(margin.top, 10.0);
    }

    #[test]
    fn gap_prop_clears_row_and_column_gap_longhands() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "style", &HostValue::string("gap: 8px 20px"));
        {
            let layout = &bridge.get(1).unwrap().props.layout;
            assert!(layout.gap.is_none());
            assert_eq!(layout.row_gap, Some(LengthSpec::Px(8.0)));
            assert_eq!(layout.column_gap, Some(LengthSpec::Px(20.0)));
        }
        // Uniform `gap` prop must reset axis longhands (CSS shorthand cascade).
        bridge.patch_prop(1, "gap", &HostValue::string("12px"));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.gap, Some(LengthSpec::Px(12.0)));
        assert!(layout.row_gap.is_none(), "row-gap longhand cleared");
        assert!(layout.column_gap.is_none(), "column-gap longhand cleared");
        assert_eq!(layout.resolved_row_gap(), 12.0);
        assert_eq!(layout.resolved_column_gap(), 12.0);

        bridge.patch_prop(
            1,
            "style",
            &HostValue::string("row-gap: 4px; column-gap: 6px"),
        );
        bridge.patch_prop(1, "gap", &HostValue::Number(10.0));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.gap, Some(LengthSpec::Px(10.0)));
        assert!(layout.row_gap.is_none());
        assert!(layout.column_gap.is_none());
    }

    #[test]
    fn gap_percent_survives_until_layout_resolve_after_cb_sync() {
        // Early style/% gap before any CB write — must not drop or freeze wrong px.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "gap", &HostValue::string("10%"));
        assert_eq!(
            bridge.get(1).unwrap().props.layout.gap,
            Some(LengthSpec::Percent(10.0)),
            "gap prop must keep % when CB is unknown"
        );
        assert_eq!(
            bridge.get(1).unwrap().props.layout.resolved_column_gap(),
            0.0,
            "without CB, % gap does not invent px"
        );

        bridge.patch_prop(1, "style", &HostValue::string("gap:8%"));
        assert_eq!(
            bridge.get(1).unwrap().props.layout.gap,
            Some(LengthSpec::Percent(8.0))
        );

        // Later CB sync: same LengthSpec re-resolves (no style re-patch required).
        bridge.sync_layout_containing_blocks(ParentBox::from_viewport(250.0, 100.0));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.gap, Some(LengthSpec::Percent(8.0)));
        assert_eq!(
            layout.resolved_column_gap_against(bridge.get(1).unwrap().props.containing_block_width),
            20.0
        );
    }

    #[test]
    fn style_patch_uses_parent_content_width_as_percent_base() {
        let mut bridge = MessageBridge::new();
        let mut parent = WidgetProps::default();
        parent.layout.width = Some(LengthSpec::Px(200.0));
        parent.layout.padding = Some(LengthSpec::Px(0.0));
        bridge.register(1, WidgetKind::Column, parent);
        bridge.register(2, WidgetKind::Box, WidgetProps::default());
        bridge.insert_child(2, 1, None);
        assert_eq!(
            bridge.get(2).unwrap().props.containing_block_width,
            Some(200.0),
            "insert_child syncs child CB from parent content width"
        );
        bridge.patch_prop(
            2,
            "style",
            &HostValue::string("margin:10%;padding:5%;gap:10%"),
        );
        let child = &bridge.get(2).unwrap().props;
        assert_eq!(child.layout.margin, Some(LengthSpec::Percent(10.0)));
        assert_eq!(child.layout.padding, Some(LengthSpec::Percent(5.0)));
        assert_eq!(
            child.layout.gap,
            Some(LengthSpec::Percent(10.0)),
            "gap % must stay LengthSpec like margin/padding (not eager px)"
        );
        let base = child.containing_block_width;
        assert_eq!(child.layout.resolved_margin_against(base).top, 20.0);
        assert_eq!(child.layout.resolved_padding_against(base).left, 10.0);
        assert_eq!(
            child.layout.resolved_column_gap_against(base),
            20.0,
            "gap % resolves against CB at layout time"
        );
    }

    #[test]
    fn set_containing_block_feeds_style_percent_base() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.set_containing_block(1, Some(400.0), None);
        bridge.patch_prop(1, "style", &HostValue::string("margin-top:10%;gap:5%"));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.margin_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.gap, Some(LengthSpec::Percent(5.0)));
        assert_eq!(layout.resolved_margin_against(Some(400.0)).top, 40.0);
        assert_eq!(layout.resolved_column_gap_against(Some(400.0)), 20.0);
    }

    #[test]
    fn sync_layout_containing_blocks_fill_parent_chain() {
        let mut bridge = MessageBridge::new();
        let mut shell = WidgetProps::default();
        shell.layout.width = Some(LengthSpec::Fill);
        shell.layout.height = Some(LengthSpec::Fill);
        shell.layout.padding = Some(LengthSpec::Px(20.0));
        bridge.register(1, WidgetKind::Column, shell);
        bridge.register(2, WidgetKind::Box, WidgetProps::default());
        bridge.insert_child(2, 1, None);

        bridge.sync_layout_containing_blocks(ParentBox::from_viewport(400.0, 300.0));
        assert_eq!(
            bridge.get(1).unwrap().props.containing_block_width,
            Some(400.0),
            "root CB = viewport"
        );
        assert_eq!(
            bridge.get(2).unwrap().props.containing_block_width,
            Some(360.0),
            "Fill parent content = viewport − padding"
        );
        assert_eq!(
            bridge.get(2).unwrap().props.containing_block_height,
            Some(260.0)
        );

        // Next style patch on child uses Fill-chain CB for gap %.
        bridge.patch_prop(2, "style", &HostValue::string("margin:10%;gap:10%"));
        let child = &bridge.get(2).unwrap().props;
        assert_eq!(child.layout.margin, Some(LengthSpec::Percent(10.0)));
        assert_eq!(child.layout.gap, Some(LengthSpec::Percent(10.0)));
        assert_eq!(
            child
                .layout
                .resolved_margin_against(child.containing_block_width)
                .top,
            36.0
        );
        assert_eq!(
            child
                .layout
                .resolved_column_gap_against(child.containing_block_width),
            36.0
        );
    }

    #[test]
    fn sync_layout_containing_blocks_nested_fill_percent_padding() {
        // viewport 500×400
        // A Fill pad 20px → content 460×360
        // B Fill pad 10% (of 460) → pad 46 → content 368×268
        // C CB = 368×268; gap 10% → 36.8
        let mut bridge = MessageBridge::new();
        let mut a = WidgetProps::default();
        a.layout.width = Some(LengthSpec::Fill);
        a.layout.height = Some(LengthSpec::Fill);
        a.layout.padding = Some(LengthSpec::Px(20.0));
        bridge.register(1, WidgetKind::Column, a);

        let mut b = WidgetProps::default();
        b.layout.width = Some(LengthSpec::Fill);
        b.layout.height = Some(LengthSpec::Fill);
        b.layout.padding = Some(LengthSpec::Percent(10.0));
        bridge.register(2, WidgetKind::Column, b);
        bridge.insert_child(2, 1, None);

        bridge.register(3, WidgetKind::Box, WidgetProps::default());
        bridge.insert_child(3, 2, None);

        bridge.sync_layout_containing_blocks(ParentBox::from_viewport(500.0, 400.0));
        assert_eq!(
            bridge.get(1).unwrap().props.containing_block_width,
            Some(500.0)
        );
        assert_eq!(
            bridge.get(2).unwrap().props.containing_block_width,
            Some(460.0)
        );
        assert_eq!(
            bridge.get(2).unwrap().props.containing_block_height,
            Some(360.0)
        );
        assert_eq!(
            bridge.get(3).unwrap().props.containing_block_width,
            Some(368.0),
            "second Fill level subtracts % padding of mid CB"
        );
        assert_eq!(
            bridge.get(3).unwrap().props.containing_block_height,
            Some(268.0)
        );

        bridge.patch_prop(3, "style", &HostValue::string("margin:10%;gap:10%"));
        let leaf = &bridge.get(3).unwrap().props;
        assert_eq!(leaf.layout.gap, Some(LengthSpec::Percent(10.0)));
        assert!(
            (leaf
                .layout
                .resolved_column_gap_against(leaf.containing_block_width)
                - 36.8)
                .abs()
                < 0.01
        );
        assert!(
            (leaf
                .layout
                .resolved_margin_against(leaf.containing_block_width)
                .top
                - 36.8)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn style_patch_justify_space_between_and_flex_grow() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.patch_prop(
            1,
            "style",
            &HostValue::string("justify-content:space-between; align-items:center"),
        );
        assert_eq!(
            bridge.get(1).unwrap().props.layout.justify_content,
            JustifySpec::SpaceBetween
        );
        bridge.register(2, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(
            2,
            "style",
            &HostValue::string("flex:1; min-width:0; overflow-y:auto"),
        );
        let child = &bridge.get(2).unwrap().props.layout;
        assert_eq!(child.flex_grow, Some(1.0));
        assert!(child.allow_shrink);
        assert!(child.scrolls_y());
    }

    #[test]
    fn nana_settings_row_class_maps_layout() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "class", &HostValue::string("nana-settings-row"));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.justify_content, JustifySpec::SpaceBetween);
        assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
    }

    #[test]
    fn prop_style_does_not_break_nana_settings_row_contract() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "class", &HostValue::string("nana-settings-row"));
        // Vue layout props land in prop_style and would otherwise wipe the row
        // contract if class hints were not re-applied after prop_style.
        bridge.patch_prop(1, "flex-direction", &HostValue::string("column"));
        bridge.patch_prop(1, "gap", &HostValue::Number(4.0));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
        assert_eq!(layout.justify_content, JustifySpec::SpaceBetween);
        assert!(
            !bridge.get(1).unwrap().props.prop_style.is_empty(),
            "layout props must still record prop_style"
        );
        // Full cascade rebuild (e.g. late stylesheet) must keep the same contract.
        bridge.inject_stylesheet(".unused-rule { color: red; }");
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
    }

    #[test]
    fn custom_property_inheritance_from_parent_scope() {
        // Parent sets --row-h; child uses var(--row-h) without defining it.
        // Class-scoped props must not pollute the document flat map.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "class", &HostValue::string("menu"));
        let mut child = WidgetProps::default();
        child.class_names = vec!["item".into()];
        bridge.register(2, WidgetKind::Row, child);
        bridge.insert_child(2, 1, None);
        bridge.inject_stylesheet(
            r#"
            :root { --pad: 4px; }
            .other { --row-h: 99px; }
            .menu { --row-h: 28px; gap: var(--pad); }
            .item { height: var(--row-h); width: var(--missing, 40px); }
            "#,
        );
        let menu = &bridge.get(1).unwrap().props.layout;
        assert_eq!(menu.gap, Some(LengthSpec::Px(4.0)));
        let item = &bridge.get(2).unwrap().props.layout;
        assert_eq!(
            item.height,
            Some(LengthSpec::Px(28.0)),
            "child must inherit parent --row-h, not .other's 99px"
        );
        assert_eq!(item.width, Some(LengthSpec::Px(40.0)));
    }

    #[test]
    fn unrelated_stylesheet_keeps_mount_root_fill_contract() {
        // inject_stylesheet must not break the viewport → mount → % height chain
        // by clearing scaffold Fill without a public class contract to restore it.
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        let mount = &bridge.get(2).unwrap().props;
        assert!(
            mount.class_names.iter().any(|c| c == "nana-mount-root"),
            "scaffold must expose nana-mount-root"
        );
        assert_eq!(mount.layout.width, Some(LengthSpec::Fill));
        assert_eq!(mount.layout.height, Some(LengthSpec::Fill));

        bridge.inject_stylesheet(".unrelated { color: red; padding: 4px; }");
        let mount = &bridge.get(2).unwrap().props.layout;
        assert_eq!(
            mount.width,
            Some(LengthSpec::Fill),
            "mount-root width Fill must survive unrelated stylesheet"
        );
        assert_eq!(
            mount.height,
            Some(LengthSpec::Fill),
            "mount-root height Fill must survive unrelated stylesheet"
        );
        let html = &bridge.get(1).unwrap().props.layout;
        assert_eq!(html.width, Some(LengthSpec::Fill));
        assert_eq!(html.height, Some(LengthSpec::Fill));
    }

    #[test]
    fn sidebar_frame_nana_tag_applies_contract_width() {
        // Custom-element tag `nana-sidebar-frame` mirrors the public class contract.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::SidebarFrame, WidgetProps::default());
        assert_eq!(
            bridge.get(1).unwrap().props.element_tag,
            "nana-sidebar-frame"
        );
        assert_eq!(
            bridge.get(1).unwrap().props.layout.width,
            Some(LengthSpec::Px(220.0)),
            "nana-sidebar-frame tag must apply class-contract width"
        );
    }

    #[test]
    fn sidebar_frame_non_contract_tag_does_not_invent_width() {
        // WidgetKind::SidebarFrame alone (foreign/div tag) must not invent 220px.
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.element_tag = "div".into();
        bridge.register(1, WidgetKind::SidebarFrame, props);
        assert!(
            bridge.get(1).unwrap().props.layout.width.is_none(),
            "non-contract tag must not invent SidebarFrame width"
        );

        // Orphan reparent with a foreign tag stays honest.
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        let mut row = WidgetProps::default();
        row.class_names = vec!["nana-workspace-shell__body".into()];
        bridge.register(3, WidgetKind::Row, row);
        bridge.insert_child(3, 2, None);
        let mut orphan = WidgetProps::default();
        orphan.element_tag = "div".into();
        bridge.register(4, WidgetKind::SidebarFrame, orphan);
        assert!(!bridge.get(3).unwrap().children.contains(&4));
        bridge.reparent_orphans();
        assert!(
            bridge.get(3).unwrap().children.contains(&4),
            "orphan SidebarFrame should reparent under workspace row"
        );
        assert!(
            bridge.get(4).unwrap().props.layout.width.is_none(),
            "reparent_orphans must not invent width for non-contract tags"
        );
    }

    #[test]
    fn reparent_orphans_ignores_bare_flex_row() {
        // A random flex-row without workspace/resources identity must not host
        // orphan sidebars.
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        let mut row = WidgetProps::default();
        row.class_names = vec!["flex-row".into()];
        bridge.register(3, WidgetKind::Row, row);
        bridge.insert_child(3, 2, None);
        let mut orphan = WidgetProps::default();
        orphan.element_tag = "nana-sidebar-frame".into();
        bridge.register(4, WidgetKind::SidebarFrame, orphan);
        bridge.reparent_orphans();
        assert!(
            !bridge.get(3).unwrap().children.contains(&4),
            "bare flex-row must not receive orphan SidebarFrame"
        );
    }

    #[test]
    fn reparent_sidebar_footer_slot_under_live_frame() {
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        let mut body = WidgetProps::default();
        body.class_names = vec!["nana-workspace-shell__body".into(), "flex-row".into()];
        bridge.register(3, WidgetKind::Row, body);
        bridge.insert_child(3, 2, None);
        let mut frame = WidgetProps::default();
        frame.element_tag = "nana-sidebar-frame".into();
        frame.class_names = vec!["nana-sidebar-frame".into()];
        frame.agent_id = "sidebar".into();
        bridge.register(4, WidgetKind::SidebarFrame, frame);
        bridge.insert_child(4, 3, None);
        let mut top = WidgetProps::default();
        top.class_names = vec!["nana-sidebar-frame__top".into()];
        top.attrs.insert("data-slot".into(), "sidebar-top".into());
        bridge.register(5, WidgetKind::Column, top);
        bridge.insert_child(5, 4, None);
        let mut body_slot = WidgetProps::default();
        body_slot.class_names = vec!["nana-sidebar-frame__body".into()];
        body_slot
            .attrs
            .insert("data-slot".into(), "sidebar-body".into());
        bridge.register(6, WidgetKind::Column, body_slot);
        bridge.insert_child(6, 4, None);
        // Orphan footer slot + content (simulates remount detach-then-fail).
        let mut footer = WidgetProps::default();
        footer.class_names = vec!["nana-sidebar-frame__footer".into()];
        footer
            .attrs
            .insert("data-slot".into(), "sidebar-footer".into());
        bridge.register(7, WidgetKind::Column, footer);
        let mut content = WidgetProps::default();
        content.class_names = vec!["sb-footer".into()];
        bridge.register(8, WidgetKind::Column, content);
        let mut settings = WidgetProps::default();
        settings.agent_id = "sidebar.footer.settings".into();
        settings.class_names = vec!["sb-footer__btn".into()];
        bridge.register(9, WidgetKind::Row, settings);
        bridge.insert_child(9, 8, None);
        assert_eq!(bridge.get(4).unwrap().children.len(), 2);
        bridge.reparent_orphans();
        let frame = bridge.get(4).unwrap();
        assert!(
            frame.children.contains(&7),
            "footer slot must reattach under live SidebarFrame: {:?}",
            frame.children
        );
        assert!(
            bridge.get(7).unwrap().children.contains(&8),
            "footer content must reattach under footer slot"
        );
        assert!(
            bridge.get(8).unwrap().children.contains(&9),
            "settings action must stay under footer content"
        );
    }

    #[test]
    fn reparent_orphans_prefers_resources_content_host() {
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        // Workspace row without shell-body contract — resources content must win.
        let mut workspace = WidgetProps::default();
        workspace.class_names = vec!["flex-row".into()];
        bridge.register(3, WidgetKind::Row, workspace);
        bridge.insert_child(3, 2, None);
        let mut resources = WidgetProps::default();
        resources.region = "resources".into();
        resources.agent_id = "workspace.region.sidebar".into();
        resources
            .attrs
            .insert("data-region-role".into(), "resources".into());
        bridge.register(4, WidgetKind::Column, resources);
        bridge.insert_child(4, 3, None);
        let mut content = WidgetProps::default();
        content.class_names = vec!["nana-workspace-region__content".into()];
        bridge.register(5, WidgetKind::Column, content);
        bridge.insert_child(5, 4, None);
        let mut orphan = WidgetProps::default();
        orphan.element_tag = "nana-sidebar-frame".into();
        bridge.register(6, WidgetKind::SidebarFrame, orphan);
        bridge.reparent_orphans();
        assert!(
            bridge.get(5).unwrap().children.contains(&6),
            "orphan must reparent under resources content, not workspace flex-row"
        );
        assert!(!bridge.get(3).unwrap().children.contains(&6));
    }

    #[test]
    fn reparent_orphans_workspace_fallback_seeds_finite_height_cb() {
        // When resources remount leaves no reachable content host, orphan
        // SidebarFrame attaches under nana-workspace-shell__body. The auto-height
        // shell content wrapper must not leave Fill CB height as None — otherwise
        // overflow-y scrollports paint at 0.
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        let mut shell = WidgetProps::default();
        shell.class_names = vec!["nana-app-shell".into()];
        shell.layout.height = Some(LengthSpec::Fill);
        shell.layout.width = Some(LengthSpec::Fill);
        bridge.register(3, WidgetKind::Column, shell);
        bridge.insert_child(3, 2, None);
        let mut content = WidgetProps::default();
        content.class_names = vec!["nana-app-shell__content".into()];
        // Grid-track content is often height-auto in CSS; give Fill so the
        // workspace Fill child still receives a definite CB in this unit test.
        content.layout.height = Some(LengthSpec::Fill);
        bridge.register(4, WidgetKind::Column, content);
        bridge.insert_child(4, 3, None);
        let mut workspace = WidgetProps::default();
        workspace.class_names = vec!["nana-workspace-shell__body".into(), "flex-row".into()];
        workspace.layout.width = Some(LengthSpec::Fill);
        workspace.layout.height = Some(LengthSpec::Fill);
        bridge.register(5, WidgetKind::Row, workspace);
        bridge.insert_child(5, 4, None);
        let mut primary = WidgetProps::default();
        primary.region = "primary".into();
        primary.layout.width = Some(LengthSpec::Fill);
        primary.layout.height = Some(LengthSpec::Fill);
        bridge.register(6, WidgetKind::Column, primary);
        bridge.insert_child(6, 5, None);
        let mut orphan = WidgetProps::default();
        orphan.element_tag = "nana-sidebar-frame".into();
        orphan.class_names = vec!["nana-sidebar-frame".into()];
        orphan.layout.apply_class_layout_hints(&orphan.class_names);
        bridge.register(7, WidgetKind::SidebarFrame, orphan);
        let mut body = WidgetProps::default();
        body.class_names = vec!["nana-sidebar-frame__body".into()];
        body.layout.apply_class_layout_hints(&body.class_names);
        bridge.register(8, WidgetKind::Column, body);
        bridge.insert_child(8, 7, None);

        bridge.sync_layout_containing_blocks(ParentBox::from_viewport(1280.0, 800.0));
        bridge.reparent_orphans();
        assert!(
            bridge.get(5).unwrap().children.contains(&7),
            "orphan must reparent under workspace when resources host is gone"
        );
        let body_cb = bridge.get(8).unwrap().props.containing_block_height;
        assert!(
            body_cb.is_some_and(|h| h > 100.0),
            "sidebar body CB height must stay finite after workspace fallback, got {body_cb:?}"
        );
    }

    #[test]
    fn data_region_role_maps_into_widget_region() {
        use nana_js_engine::HostValue;

        let mut patched = WidgetProps::default();
        patched.apply_prop("data-region-role", &HostValue::string("resources"));
        assert_eq!(patched.region, "resources");
        assert_eq!(
            patched.attrs.get("data-region-role").map(String::as_str),
            Some("resources")
        );
    }

    #[test]
    fn data_slot_sidebar_body_applies_frame_body_hints() {
        use nana_js_engine::HostValue;

        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "data-slot", &HostValue::string("sidebar-body"));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert_eq!(layout.flex_grow, Some(1.0));
        assert_eq!(layout.height, Some(LengthSpec::Fill));
        assert!(
            bridge
                .get(1)
                .unwrap()
                .props
                .class_names
                .iter()
                .any(|c| c == "nana-sidebar-frame__body")
        );
    }

    #[test]
    fn region_views_keep_lilia_resources_and_sidebar_agent_in_primary() {
        // Lilia ResourcePanel (`data-region-role=resources` / workspace.region.sidebar)
        // and SecondaryPanel (`agent-id=sidebar`) are in-tree workspace chrome —
        // not DesktopShell Navigation opt-in. Lifting them emptied Primary and
        // stacked remount leftovers into an empty-looking shell sidebar.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::SidebarFrame,
            WidgetProps {
                agent_id: "sidebar".into(),
                element_tag: "nana-sidebar-frame".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Column,
            WidgetProps {
                region: "resources".into(),
                agent_id: "workspace.region.sidebar".into(),
                class_names: vec!["lilia-workspace-region--resources".into()],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Column,
            WidgetProps {
                region: "primary".into(),
                agent_id: "workspace.region.main".into(),
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(3, 1, None);
        bridge.insert_child(2, 3, None);
        bridge.insert_child(4, 1, None);
        let views = bridge.snapshot().region_views();
        assert!(
            views.navigation.widgets.is_empty(),
            "Lilia resources/sidebar must not invent DesktopShell Navigation"
        );
        assert!(
            views.primary.widgets.iter().any(|w| w.id == 2),
            "SidebarFrame stays in Primary"
        );
        assert!(
            views.primary.widgets.iter().any(|w| w.id == 3),
            "resources shell stays in Primary"
        );
        assert!(
            views.primary.widgets.iter().any(|w| w.id == 4),
            "primary region stays in Primary"
        );
        assert!(views.overlapping_ids().is_empty());
    }

    #[test]
    fn deep_child_combinator_stylesheet_matches_full_ancestry() {
        let mut bridge = MessageBridge::new();
        bridge.inject_stylesheet(".a > .b > .c > .leaf { gap: 18px; width: 100%; }");
        for (id, class) in [(1, "a"), (2, "b"), (3, "c"), (4, "leaf")] {
            let mut props = WidgetProps::default();
            props.element_tag = "div".into();
            bridge.register(id, WidgetKind::Column, props);
            bridge.patch_prop(id, "class", &HostValue::string(class));
        }
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 3, None);
        // Re-apply after tree links so MatchContext sees the full parent chain.
        bridge.patch_prop(4, "class", &HostValue::string("leaf"));
        let leaf = &bridge.get(4).unwrap().props.layout;
        assert_eq!(leaf.gap, Some(LengthSpec::Px(18.0)));
        assert_eq!(leaf.width, Some(LengthSpec::Fill));
    }

    #[test]
    fn element_id_alone_does_not_invent_region_layout() {
        // Layout must come from stylesheet / class hints / inline — not an
        // id|data-region-id whitelist (sidebar/main/primary/left).
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(1, "id", &HostValue::string("sidebar"));
        let layout = &bridge.get(1).unwrap().props.layout;
        assert!(layout.width.is_none());
        assert_ne!(layout.width, Some(LengthSpec::Px(220.0)));
        assert!(layout.flex_grow.is_none());

        bridge.register(2, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(2, "data-region-id", &HostValue::string("main"));
        let main = &bridge.get(2).unwrap().props.layout;
        assert!(main.width.is_none());
        assert!(main.flex_grow.is_none());

        // Public shell class contract still sizes the sidebar.
        bridge.register(3, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(
            3,
            "class",
            &HostValue::string("nana-workspace-shell__sidebar"),
        );
        assert_eq!(
            bridge.get(3).unwrap().props.layout.width,
            Some(LengthSpec::Px(220.0))
        );
    }

    #[test]
    fn tabs_select_value_updates_props() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            5,
            WidgetKind::Tabs,
            WidgetProps {
                options: vec![
                    SelectOptionProp {
                        value: "a".into(),
                        label: "A".into(),
                        disabled: false,
                    },
                    SelectOptionProp {
                        value: "b".into(),
                        label: "B".into(),
                        disabled: false,
                    },
                ],
                value: "a".into(),
                ..WidgetProps::default()
            },
        );
        let names = bridge.note_select_value(5, "b");
        assert!(names.contains(&"update:modelValue"));
        assert_eq!(bridge.get(5).unwrap().props.value, "b");
        assert!(bridge.get(5).unwrap().props.active);
    }

    #[test]
    fn widget_kind_parses_catalog_professional_aliases() {
        assert_eq!(
            WidgetKind::parse("nana-command-palette"),
            Some(WidgetKind::CommandPalette)
        );
        assert_eq!(
            WidgetKind::parse("command-palette"),
            Some(WidgetKind::CommandPalette)
        );
        assert_eq!(
            WidgetKind::parse("commandpalette"),
            Some(WidgetKind::CommandPalette)
        );
        assert_eq!(
            WidgetKind::parse("nana-tree-view"),
            Some(WidgetKind::TreeView)
        );
        assert_eq!(WidgetKind::parse("tree-view"), Some(WidgetKind::TreeView));
        assert_eq!(WidgetKind::parse("treeview"), Some(WidgetKind::TreeView));
        assert_eq!(
            WidgetKind::parse("nana-calendar"),
            Some(WidgetKind::CalendarHeatmap)
        );
        assert_eq!(
            WidgetKind::parse("calendar-heatmap"),
            Some(WidgetKind::CalendarHeatmap)
        );
        assert_eq!(
            WidgetKind::parse("calendar"),
            Some(WidgetKind::CalendarHeatmap)
        );
        assert_eq!(
            WidgetKind::parse("nana-image-viewer"),
            Some(WidgetKind::ImageViewer)
        );
        assert_eq!(
            WidgetKind::parse("image-viewer"),
            Some(WidgetKind::ImageViewer)
        );
        assert_eq!(
            WidgetKind::parse("nana-markdown"),
            Some(WidgetKind::NativeMarkdown)
        );
        assert_eq!(
            WidgetKind::parse("native-markdown"),
            Some(WidgetKind::NativeMarkdown)
        );
        assert_eq!(
            WidgetKind::parse("markdown"),
            Some(WidgetKind::NativeMarkdown)
        );
        assert_eq!(
            WidgetKind::parse("nana-graph-canvas"),
            Some(WidgetKind::GraphCanvas)
        );
        assert_eq!(
            WidgetKind::parse("graph-canvas"),
            Some(WidgetKind::GraphCanvas)
        );
        assert_eq!(
            WidgetKind::parse("graphcanvas"),
            Some(WidgetKind::GraphCanvas)
        );
        assert_eq!(
            WidgetKind::parse("nana-workspace"),
            Some(WidgetKind::Workspace)
        );
        assert_eq!(WidgetKind::parse("nana-dock"), Some(WidgetKind::Dock));
        assert_eq!(
            WidgetKind::parse("nana-split-pane"),
            Some(WidgetKind::SplitPane)
        );
        assert_eq!(WidgetKind::parse("split-pane"), Some(WidgetKind::SplitPane));
        assert_eq!(
            WidgetKind::parse("nana-app-shell"),
            Some(WidgetKind::AppShell)
        );
        assert_eq!(WidgetKind::parse("app-shell"), Some(WidgetKind::AppShell));
        assert_eq!(
            WidgetKind::parse("nana-settings-page"),
            Some(WidgetKind::SettingsPage)
        );
        assert_eq!(
            WidgetKind::parse("settings-page"),
            Some(WidgetKind::SettingsPage)
        );
        assert_eq!(
            WidgetKind::parse("settingspage"),
            Some(WidgetKind::SettingsPage)
        );
        assert_eq!(WidgetKind::SettingsPage.as_str(), "settings-page");
        assert_eq!(WidgetKind::SettingsPage.element_tag(), "nana-settings-page");
        assert_eq!(WidgetKind::CommandPalette.as_str(), "command-palette");
        assert_eq!(
            WidgetKind::CommandPalette.element_tag(),
            "nana-command-palette"
        );
        assert_eq!(WidgetKind::CalendarHeatmap.element_tag(), "nana-calendar");
        assert_eq!(WidgetKind::NativeMarkdown.as_str(), "native-markdown");
        assert_eq!(WidgetKind::GraphCanvas.element_tag(), "nana-graph-canvas");
        assert_eq!(WidgetKind::GraphCanvas.as_str(), "graph-canvas");
        assert!(WidgetKind::CommandPalette.is_overlay());
        assert!(WidgetKind::ImageViewer.is_overlay());
        assert!(!WidgetKind::GraphCanvas.is_overlay());
        assert!(!WidgetKind::Workspace.is_overlay());
        assert!(!WidgetKind::TreeView.is_overlay());
    }

    #[test]
    fn overlay_toggle_false_clears_active_and_toggled() {
        // Opened via `active`/`open` (common Vue path); dismiss must clear both
        // because overlay_is_open = active || toggled.
        for kind in [
            WidgetKind::Dialog,
            WidgetKind::Drawer,
            WidgetKind::Popover,
            WidgetKind::ContextMenu,
        ] {
            let mut bridge = MessageBridge::new();
            bridge.register(
                1,
                kind,
                WidgetProps {
                    active: true,
                    toggled: true,
                    ..WidgetProps::default()
                },
            );
            assert!(kind.is_overlay());
            let names = bridge.note_toggle(1, false);
            assert!(names.contains(&"update:modelValue"));
            let props = &bridge.get(1).unwrap().props;
            assert!(!props.active, "{kind:?} active should clear on dismiss");
            assert!(!props.toggled, "{kind:?} toggled should clear on dismiss");
            assert!(
                !(props.active || props.toggled),
                "{kind:?} must not remain open after Toggle false"
            );
        }
    }

    #[test]
    fn overlay_toggle_false_clears_active_only_open() {
        // Opened with active=true, toggled=false (apply_prop "active" path).
        let mut bridge = MessageBridge::new();
        bridge.register(
            2,
            WidgetKind::Dialog,
            WidgetProps {
                active: true,
                toggled: false,
                ..WidgetProps::default()
            },
        );
        bridge.note_toggle(2, false);
        let props = &bridge.get(2).unwrap().props;
        assert!(!props.active);
        assert!(!props.toggled);
    }

    #[test]
    fn modal_presence_opens_dialog_without_fixed() {
        // Teleport dialog: aria-modal presence, no open= — Nana Overlay only.
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.role = "dialog".into();
        props.class_names = vec!["nana-dialog".into()];
        props.attrs.insert("aria-modal".into(), "true".into());
        props.layout.position = crate::css_map::PositionSpec::Fixed;
        bridge.register(10, WidgetKind::Dialog, props);
        let w = bridge.get(10).unwrap();
        assert!(
            w.props.active && w.props.toggled,
            "presence must open Dialog"
        );
        assert_eq!(
            w.props.layout.position,
            crate::css_map::PositionSpec::Static,
            "must strip deferred fixed — Nana Overlay only"
        );
    }

    #[test]
    fn non_overlay_keeps_css_fixed_for_viewport_subset() {
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.label = "pin".into();
        props.layout.apply_css_text(
            "position:fixed;top:0;left:0;width:40px;height:24px",
            None,
            None,
        );
        bridge.register(20, WidgetKind::Box, props);
        let w = bridge.get(20).unwrap();
        assert_eq!(w.props.layout.position, crate::css_map::PositionSpec::Fixed);
        assert!(w.props.layout.is_fixed());
        assert!(!w.props.layout.position.is_unsupported_positioning());
    }

    #[test]
    fn modal_role_patch_promotes_and_opens() {
        let mut bridge = MessageBridge::new();
        bridge.register(11, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(11, "class", &HostValue::string("nana-dialog"));
        bridge.patch_prop(11, "role", &HostValue::string("dialog"));
        bridge.patch_prop(11, "aria-modal", &HostValue::string("true"));
        let w = bridge.get(11).unwrap();
        assert_eq!(w.kind, WidgetKind::Dialog);
        assert!(w.props.active && w.props.toggled);
    }

    #[test]
    fn closed_nana_dialog_does_not_auto_open_from_class() {
        // NanaDialog stays mounted with class nana-dialog while open=false.
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.class_names = vec!["nana-dialog".into()];
        props.role = "dialog".into();
        props.active = false;
        props.toggled = false;
        bridge.register(12, WidgetKind::Dialog, props);
        let w = bridge.get(12).unwrap();
        assert!(!w.props.active && !w.props.toggled);
    }

    #[test]
    fn dropdown_class_maps_to_select_not_fixed_menu() {
        let mut bridge = MessageBridge::new();
        bridge.register(13, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(13, "class", &HostValue::string("nana-dropdown"));
        assert_eq!(bridge.get(13).unwrap().kind, WidgetKind::Select);
        // Unregistered id is a no-op; register panel explicitly.
        bridge.register(14, WidgetKind::Column, WidgetProps::default());
        bridge.patch_prop(14, "class", &HostValue::string("nana-select"));
        bridge.patch_prop(14, "role", &HostValue::string("listbox"));
        assert_eq!(bridge.get(14).unwrap().kind, WidgetKind::Select);
    }

    #[test]
    fn switch_toggle_does_not_force_active() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            3,
            WidgetKind::Switch,
            WidgetProps {
                toggled: true,
                active: false,
                ..WidgetProps::default()
            },
        );
        bridge.note_toggle(3, false);
        let props = &bridge.get(3).unwrap().props;
        assert!(!props.toggled);
        assert!(!props.active);
        bridge.note_toggle(3, true);
        let props = &bridge.get(3).unwrap().props;
        assert!(props.toggled);
        assert!(
            !props.active,
            "Switch toggle must not set active (overlay-only sync)"
        );
    }

    #[test]
    fn patch_prop_switch_input_segmented_semantics() {
        let mut bridge = MessageBridge::new();

        // Switch: boolean disabled + toggled via `.` modifier and plain keys.
        bridge.register(1, WidgetKind::Switch, WidgetProps::default());
        bridge.patch_prop(1, ".disabled", &HostValue::Bool(true));
        bridge.patch_prop(1, "toggled", &HostValue::Bool(true));
        let sw = bridge.get(1).unwrap();
        assert!(sw.props.disabled);
        assert!(sw.props.toggled);

        // Input: value + placeholder + disabled false clears.
        bridge.register(2, WidgetKind::Input, WidgetProps::default());
        bridge.patch_prop(2, ".value", &HostValue::string("typed"));
        bridge.patch_prop(2, "placeholder", &HostValue::string("hint"));
        bridge.patch_prop(2, "disabled", &HostValue::Bool(false));
        let input = bridge.get(2).unwrap();
        assert_eq!(input.props.value, "typed");
        assert_eq!(input.props.placeholder, "hint");
        assert!(!input.props.disabled);

        // Segmented: options array + value selection.
        bridge.register(3, WidgetKind::Segmented, WidgetProps::default());
        let options = HostValue::Array(vec![
            HostValue::Object(
                [
                    ("value".into(), HostValue::string("light")),
                    ("label".into(), HostValue::string("浅色")),
                ]
                .into_iter()
                .collect(),
            ),
            HostValue::Object(
                [
                    ("value".into(), HostValue::string("dark")),
                    ("label".into(), HostValue::string("暗色")),
                ]
                .into_iter()
                .collect(),
            ),
        ]);
        bridge.patch_prop(3, "options", &options);
        bridge.patch_prop(3, "value", &HostValue::string("dark"));
        let seg = bridge.get(3).unwrap();
        assert_eq!(seg.props.options.len(), 2);
        assert_eq!(seg.props.options[1].label, "暗色");
        assert_eq!(seg.props.value, "dark");
    }

    #[test]
    fn patch_prop_svg_attrs_and_force_attr() {
        let mut bridge = MessageBridge::new();
        bridge.register(4, WidgetKind::Icon, WidgetProps::default());
        bridge.patch_prop(4, "viewBox", &HostValue::string("0 0 24 24"));
        bridge.patch_prop(4, "^xlink:href", &HostValue::string("#star"));
        let icon = bridge.get(4).unwrap();
        assert!(
            icon.props.attrs.contains_key("view-box") || icon.props.attrs.contains_key("viewbox"),
            "viewBox must land in attrs, got {:?}",
            icon.props.attrs
        );
        assert_eq!(
            icon.props.attrs.get("xlink:href").map(String::as_str),
            Some("#star")
        );
    }

    #[test]
    fn chart_svg_pins_min_height_to_author_px_height() {
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.element_tag = "svg".into();
        bridge.register(9, WidgetKind::Box, props);
        bridge.patch_prop(9, "viewBox", &HostValue::string("0 0 905 125"));
        bridge.patch_prop(9, "width", &HostValue::Number(905.0));
        bridge.patch_prop(9, "height", &HostValue::Number(125.0));
        let w = bridge.get(9).unwrap();
        assert_eq!(
            w.props.layout.height,
            Some(LengthSpec::Px(125.0)),
            "height attr must map to layout"
        );
        assert_eq!(
            w.props.layout.min_height,
            Some(LengthSpec::Px(125.0)),
            "chart svg must pin min-height so flex cannot crush weekday rows, got {:?}",
            w.props.layout.min_height
        );

        // overflow:hidden keeps CSS min-size:auto → 0 (may shrink).
        let mut clipped = WidgetProps::default();
        clipped.element_tag = "svg".into();
        bridge.register(10, WidgetKind::Box, clipped);
        bridge.patch_prop(10, "viewBox", &HostValue::string("0 0 40 20"));
        bridge.patch_prop(10, "height", &HostValue::Number(20.0));
        bridge.patch_prop(10, "style", &HostValue::string("overflow: hidden"));
        let c = bridge.get(10).unwrap();
        assert!(
            c.props.layout.min_height.is_none()
                || matches!(c.props.layout.min_height, Some(LengthSpec::Px(mh)) if mh < 1.0),
            "overflow:hidden chart svg must not raise min-height, got {:?}",
            c.props.layout.min_height
        );
    }

    #[test]
    fn patch_prop_stroke_dash_attrs_stay_out_of_value() {
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.element_tag = "circle".into();
        props.value = String::new();
        props.hint = String::new();
        bridge.register(7, WidgetKind::Box, props);
        bridge.patch_prop(7, "stroke-dasharray", &HostValue::string("68 32"));
        bridge.patch_prop(7, "stroke-dashoffset", &HostValue::string("-12.5"));
        bridge.patch_prop(7, "pathLength", &HostValue::string("100"));
        let w = bridge.get(7).unwrap();
        assert_eq!(
            w.props.attrs.get("stroke-dasharray").map(String::as_str),
            Some("68 32")
        );
        assert_eq!(
            w.props.attrs.get("stroke-dashoffset").map(String::as_str),
            Some("-12.5")
        );
        assert!(
            w.props.attrs.contains_key("pathlength")
                || w.props.attrs.contains_key("path-length")
                || w.props.attrs.contains_key("pathLength"),
            "pathLength in attrs, got {:?}",
            w.props.attrs
        );
        assert!(
            w.props.value.is_empty(),
            "dasharray must not clobber value, got {:?}",
            w.props.value
        );
        assert!(
            w.props.hint.is_empty(),
            "dashoffset must not clobber hint, got {:?}",
            w.props.hint
        );
    }

    #[test]
    fn overlay_select_value_closes_after_confirm() {
        // ConfirmDialog / Drawer footer emit SelectValue on the overlay id;
        // confirm must close unless product keeps it open.
        for (kind, value) in [
            (WidgetKind::Dialog, "confirm"),
            (WidgetKind::Drawer, "confirm"),
            (WidgetKind::ContextMenu, "item-a"),
        ] {
            let mut bridge = MessageBridge::new();
            bridge.register(
                4,
                kind,
                WidgetProps {
                    active: true,
                    toggled: true,
                    ..WidgetProps::default()
                },
            );
            let names = bridge.note_select_value(4, value);
            assert!(names.contains(&"update:modelValue"));
            let props = &bridge.get(4).unwrap().props;
            assert_eq!(props.value, value);
            assert!(
                !props.active && !props.toggled,
                "{kind:?} should close after SelectValue confirm"
            );
        }
    }

    #[test]
    fn overlay_patch_prop_false_clears_both_open_flags() {
        // Vue often patches only `active` / `selected` or only `model-value`/`toggled`.
        // overlay_is_open = active || toggled, so the other side must clear too.
        for kind in [
            WidgetKind::Dialog,
            WidgetKind::Drawer,
            WidgetKind::Popover,
            WidgetKind::ContextMenu,
        ] {
            for key in [
                "active",
                "open",
                "selected",
                "aria-selected",
                "aria-pressed",
                "toggled",
                "model-value",
            ] {
                let mut bridge = MessageBridge::new();
                bridge.register(
                    10,
                    kind,
                    WidgetProps {
                        active: true,
                        toggled: true,
                        ..WidgetProps::default()
                    },
                );
                bridge.patch_prop(10, key, &HostValue::Bool(false));
                let props = &bridge.get(10).unwrap().props;
                assert!(
                    !props.active && !props.toggled,
                    "{kind:?} patch {key}=false must clear both open flags"
                );
            }
        }
    }

    #[test]
    fn overlay_patch_selected_false_closes_when_toggled_stuck() {
        // Regression: apply_prop writes selected → active only; without bilateral
        // sync, toggled stays true and overlay_is_open remains open.
        let mut bridge = MessageBridge::new();
        bridge.register(
            13,
            WidgetKind::Popover,
            WidgetProps {
                active: true,
                toggled: true,
                ..WidgetProps::default()
            },
        );
        bridge.patch_prop(13, "selected", &HostValue::Bool(false));
        let props = &bridge.get(13).unwrap().props;
        assert!(!props.active, "selected=false must clear active");
        assert!(
            !props.toggled,
            "selected=false must clear toggled (bilateral sync)"
        );
    }

    #[test]
    fn overlay_patch_prop_true_syncs_both_open_flags() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            11,
            WidgetKind::Dialog,
            WidgetProps {
                active: false,
                toggled: false,
                ..WidgetProps::default()
            },
        );
        bridge.patch_prop(11, "active", &HostValue::Bool(true));
        let props = &bridge.get(11).unwrap().props;
        assert!(
            props.active && props.toggled,
            "active=true should open both"
        );

        bridge.patch_prop(11, "active", &HostValue::Bool(false));
        bridge.patch_prop(11, "model-value", &HostValue::Bool(true));
        let props = &bridge.get(11).unwrap().props;
        assert!(
            props.active && props.toggled,
            "model-value=true should open both"
        );
    }

    #[test]
    fn overlay_patch_model_value_string_does_not_reopen() {
        // After SelectValue close, Vue may patch model-value to the confirm string.
        let mut bridge = MessageBridge::new();
        bridge.register(
            12,
            WidgetKind::Dialog,
            WidgetProps {
                active: false,
                toggled: false,
                value: String::new(),
                ..WidgetProps::default()
            },
        );
        bridge.patch_prop(12, "model-value", &HostValue::string("confirm"));
        let props = &bridge.get(12).unwrap().props;
        assert_eq!(props.value, "confirm");
        assert!(
            !props.active && !props.toggled,
            "string model-value must not reopen overlay"
        );
    }

    #[test]
    fn switch_patch_model_value_does_not_force_active() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            13,
            WidgetKind::Switch,
            WidgetProps {
                toggled: true,
                active: false,
                ..WidgetProps::default()
            },
        );
        bridge.patch_prop(13, "model-value", &HostValue::Bool(false));
        let props = &bridge.get(13).unwrap().props;
        assert!(!props.toggled);
        assert!(!props.active);
        bridge.patch_prop(13, "model-value", &HostValue::Bool(true));
        let props = &bridge.get(13).unwrap().props;
        assert!(props.toggled);
        assert!(
            !props.active,
            "Switch model-value must not set active (overlay-only sync)"
        );
    }

    #[test]
    fn region_views_are_mutually_exclusive_by_region_tags() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                // DesktopShell Navigation requires an explicit region token —
                // agent suffixes like `.navigation` alone must not invent lift.
                region: "global-navigation".into(),
                agent_id: "nana.workspace.navigation".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::SidebarRow,
            WidgetProps {
                label: "Home".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Column,
            WidgetProps {
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Card,
            WidgetProps {
                label: "card".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::Column,
            WidgetProps {
                role: "inspector".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            7,
            WidgetKind::Text,
            WidgetProps {
                label: "facts".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 1, None);
        bridge.insert_child(5, 4, None);
        bridge.insert_child(6, 1, None);
        bridge.insert_child(7, 6, None);
        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(
            views.overlapping_ids().is_empty(),
            "region views must not share widget ids: {:?}",
            views.overlapping_ids()
        );
        assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
        assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
        assert!(views.inspector.widgets.iter().any(|w| w.id == 6));
        assert!(views.primary.widgets.iter().any(|w| w.id == 4));
        assert!(views.primary.widgets.iter().any(|w| w.id == 5));
        assert!(!views.primary.widgets.iter().any(|w| w.id == 2 || w.id == 3));
        assert!(!views.primary.widgets.iter().any(|w| w.id == 6 || w.id == 7));
    }

    #[test]
    fn region_views_do_not_harvest_untagged_sidebar_or_cards() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(2, WidgetKind::SidebarFrame, WidgetProps::default());
        bridge.register(
            3,
            WidgetKind::SidebarRow,
            WidgetProps {
                label: "Home".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Card,
            WidgetProps {
                label: "card".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::SettingsCard,
            WidgetProps {
                label: "外观".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 1, None);
        bridge.insert_child(5, 1, None);
        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(
            views.navigation.widgets.is_empty(),
            "untagged SidebarFrame must not be claimed as Navigation"
        );
        assert!(
            views.inspector.widgets.is_empty(),
            "untagged Card/Settings must not be claimed as Inspector"
        );
        assert!(views.primary.widgets.iter().any(|w| w.id == 2));
        assert!(views.primary.widgets.iter().any(|w| w.id == 4));
        assert!(views.primary.widgets.iter().any(|w| w.id == 5));
        assert_eq!(views.primary.widgets.len(), snap.widgets.len());
        assert!(views.overlapping_ids().is_empty());
    }

    #[test]
    fn region_views_collapse_shell_grid_when_nav_column_extracted() {
        // NanaWorkspaceShell body: 220px + 1fr. Tagged nav leaves Primary with
        // only the primary column — stale 2-col grid must not squeeze it to 220.
        let mut bridge = MessageBridge::new();
        let mut body = WidgetProps::default();
        body.class_names = vec!["nana-workspace-shell__body".into()];
        body.layout.apply_class_layout_hints(&body.class_names);
        bridge.register(1, WidgetKind::Row, body);
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                region: "global-navigation".into(),
                agent_id: "nana.workspace.sidebar".into(),
                class_names: vec!["nana-workspace-shell__sidebar".into()],
                layout: {
                    let mut l = LayoutStyle::default();
                    l.apply_class_layout_hints(&["nana-workspace-shell__sidebar".into()]);
                    l
                },
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Column,
            WidgetProps {
                region: "primary".into(),
                agent_id: "nana.workspace.primary".into(),
                class_names: vec!["nana-workspace-shell__primary".into()],
                layout: {
                    let mut l = LayoutStyle::default();
                    l.apply_class_layout_hints(&["nana-workspace-shell__primary".into()]);
                    l
                },
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Text,
            WidgetProps {
                label: "main content".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        bridge.insert_child(4, 3, None);

        let snap = bridge.snapshot();
        assert_eq!(
            snap.get(1)
                .unwrap()
                .props
                .layout
                .grid_columns
                .as_ref()
                .map(|c| c.len()),
            Some(2),
            "full forest keeps 2-col shell body"
        );

        let views = snap.region_views();
        assert!(views.overlapping_ids().is_empty());
        assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
        assert!(views.primary.widgets.iter().any(|w| w.id == 3));
        assert!(!views.primary.widgets.iter().any(|w| w.id == 2));

        let body = views.primary.get(1).expect("shell body remains in primary");
        assert_eq!(body.children, vec![3], "only primary column remains");
        let cols = body
            .props
            .layout
            .grid_columns
            .as_ref()
            .expect("collapsed track list");
        assert_eq!(
            cols.len(),
            1,
            "stale 2-col grid must collapse to remaining child count"
        );
        assert_eq!(
            cols[0],
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: None,
            }
        );

        // Measure: primary column must get full body width, not ~220.
        let mut body_style = body.props.layout.clone();
        body_style.width = Some(LengthSpec::Px(800.0));
        body_style.height = Some(LengthSpec::Px(400.0));
        let mut primary_style = views.primary.get(3).unwrap().props.layout.clone();
        primary_style.height = Some(LengthSpec::Fill);
        let tree = crate::measure::LayoutNode::with_children(
            "body",
            body_style,
            vec![crate::measure::LayoutNode::leaf("primary", primary_style)],
        );
        let boxes = crate::measure::measure_layout(&tree, 800.0, 400.0);
        let primary_box = boxes
            .iter()
            .find(|(id, _)| id == "primary")
            .map(|(_, b)| b)
            .expect("primary box");
        assert!(
            primary_box.width > 500.0,
            "primary must not be squeezed to sidebar track (~220); got {}",
            primary_box.width
        );
        assert!((primary_box.width - 800.0).abs() < 1.0);
    }

    #[test]
    fn region_views_claim_hollow_outer_shell_after_nested_nav_lift() {
        // Lilia-shaped: workspace row → outer start panel (+ resize chrome) →
        // nested tagged SidebarFrame. Nested tag must not leave a fixed-width
        // empty shell in Primary beside DesktopShell Navigation.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                agent_id: "workspace.region.sidebar".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Column,
            WidgetProps {
                class_names: vec!["nana-workspace-region__content".into()],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::SidebarFrame,
            WidgetProps {
                region: "global-navigation".into(),
                agent_id: "sidebar".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Text,
            WidgetProps {
                label: "项目总览".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::Column,
            WidgetProps {
                role: "separator".into(),
                agent_id: "workspace.region.sidebar.resize".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            7,
            WidgetKind::Column,
            WidgetProps {
                agent_id: "workspace.region.main".into(),
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(6, 2, None);
        bridge.insert_child(4, 3, None);
        bridge.insert_child(5, 4, None);
        bridge.insert_child(7, 1, None);

        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(views.overlapping_ids().is_empty());
        assert!(
            views.navigation.widgets.iter().any(|w| w.id == 4),
            "nested tagged frame still projects to Navigation"
        );
        assert!(
            !views
                .primary
                .widgets
                .iter()
                .any(|w| w.id == 2 || w.id == 3 || w.id == 6),
            "hollow outer shell + resize chrome must leave Primary: {:?}",
            views
                .primary
                .widgets
                .iter()
                .map(|w| w.id)
                .collect::<Vec<_>>()
        );
        assert!(
            views.primary.widgets.iter().any(|w| w.id == 7),
            "main column stays in Primary"
        );
        assert!(
            !views.primary.widgets.iter().any(|w| w.id == 4 || w.id == 5),
            "nav content exclusive of Primary"
        );
    }

    #[test]
    fn inspector_slice_keeps_region_tagged_settings() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                agent_id: "nana.workspace.inspector".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::SettingsCard,
            WidgetProps {
                label: "Inspector card".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::SettingsCard,
            WidgetProps {
                label: "Primary appearance".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 1, None);
        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(
            views
                .inspector
                .widgets
                .iter()
                .any(|w| w.id == 2 || w.id == 3)
        );
        assert!(
            !views.inspector.widgets.iter().any(|w| w.id == 4),
            "untagged Primary SettingsCard must stay out of Inspector"
        );
        assert!(views.primary.widgets.iter().any(|w| w.id == 4));
        assert!(!views.primary.widgets.iter().any(|w| w.id == 2 || w.id == 3));
        assert!(views.overlapping_ids().is_empty());
    }

    #[test]
    fn data_region_prop_maps_into_widget_props() {
        use nana_js_engine::HostValue;
        use std::collections::BTreeMap;

        let mut map = BTreeMap::new();
        map.insert("data-region".into(), HostValue::string("global-navigation"));
        map.insert(
            "data-agent-id".into(),
            HostValue::string("nana.workspace.sidebar"),
        );
        let props = WidgetProps::from_map(&map);
        assert_eq!(props.region, "global-navigation");
        assert_eq!(props.agent_id, "nana.workspace.sidebar");

        let mut patched = WidgetProps::default();
        patched.apply_prop("data-region", &HostValue::string("section-navigation"));
        assert_eq!(patched.region, "section-navigation");
    }

    #[test]
    fn region_views_honor_data_region_and_sidebar_agent_contract() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        // nanavue NanaWorkspaceShell aside / NanaSidebarFrame contract tags
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                region: "global-navigation".into(),
                agent_id: "nana.workspace.sidebar".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::SidebarFrame,
            WidgetProps {
                region: "global-navigation".into(),
                agent_id: "sidebar.main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Column,
            WidgetProps {
                region: "section-navigation".into(),
                agent_id: "nana.sidebar-nav".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Column,
            WidgetProps {
                region: "primary".into(),
                agent_id: "nana.workspace.primary".into(),
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::Text,
            WidgetProps {
                label: "body".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 2, None);
        bridge.insert_child(5, 1, None);
        bridge.insert_child(6, 5, None);

        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(
            views.overlapping_ids().is_empty(),
            "overlapping: {:?}",
            views.overlapping_ids()
        );
        assert!(
            !views.navigation.widgets.is_empty(),
            "contract-tagged navigation must be non-empty"
        );
        assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
        assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
        assert!(views.navigation.widgets.iter().any(|w| w.id == 4));
        assert!(
            !views
                .navigation
                .widgets
                .iter()
                .any(|w| w.id == 5 || w.id == 6),
            "primary-tagged content must not enter navigation"
        );
        assert!(views.primary.widgets.iter().any(|w| w.id == 5));
        assert!(views.primary.widgets.iter().any(|w| w.id == 6));
        assert!(
            !views
                .primary
                .widgets
                .iter()
                .any(|w| w.id == 2 || w.id == 3 || w.id == 4),
            "navigation-tagged nodes must be exclusive of primary"
        );
    }

    #[test]
    fn region_views_limited_excludes_truncated_tagged_seeds_from_primary() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                region: "global-navigation".into(),
                agent_id: "nav.a".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Text,
            WidgetProps {
                label: "A".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Column,
            WidgetProps {
                region: "section-navigation".into(),
                agent_id: "nav.b".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Text,
            WidgetProps {
                label: "B".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::Column,
            WidgetProps {
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 1, None);
        bridge.insert_child(5, 4, None);
        bridge.insert_child(6, 1, None);

        let snap = bridge.snapshot();
        let views = snap.region_views_limited(1, 0);
        assert!(
            views.overlapping_ids().is_empty(),
            "overlapping: {:?}",
            views.overlapping_ids()
        );
        assert_eq!(
            views.navigation.roots.len(),
            1,
            "nav_limit=1 projects a single seed"
        );
        assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
        assert!(
            !views
                .navigation
                .widgets
                .iter()
                .any(|w| w.id == 4 || w.id == 5),
            "truncated tagged seed must not appear in the limited projection"
        );
        assert!(
            !views
                .primary
                .widgets
                .iter()
                .any(|w| w.id == 2 || w.id == 3 || w.id == 4 || w.id == 5),
            "all region-tagged nodes must leave primary even when truncated"
        );
        assert!(views.primary.widgets.iter().any(|w| w.id == 6));
        assert!(views.inspector.widgets.is_empty());
    }

    #[test]
    fn region_views_inspector_nested_under_nav_is_exclusive() {
        // Nearest-tag rule: inspector under a navigation ancestor belongs to
        // Inspector only — nav must not re-harvest that subtree.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                region: "global-navigation".into(),
                agent_id: "nana.workspace.sidebar".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::SidebarRow,
            WidgetProps {
                label: "Home".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Column,
            WidgetProps {
                role: "inspector".into(),
                agent_id: "nana.workspace.inspector".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Text,
            WidgetProps {
                label: "facts".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::Column,
            WidgetProps {
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 2, None);
        bridge.insert_child(5, 4, None);
        bridge.insert_child(6, 1, None);

        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(
            views.overlapping_ids().is_empty(),
            "overlapping: {:?}",
            views.overlapping_ids()
        );
        assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
        assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
        assert!(
            !views
                .navigation
                .widgets
                .iter()
                .any(|w| w.id == 4 || w.id == 5),
            "inspector subtree must not remain in navigation"
        );
        assert!(views.inspector.widgets.iter().any(|w| w.id == 4));
        assert!(views.inspector.widgets.iter().any(|w| w.id == 5));
        assert!(
            !views
                .inspector
                .widgets
                .iter()
                .any(|w| w.id == 2 || w.id == 3)
        );
        assert!(views.primary.widgets.iter().any(|w| w.id == 6));
        assert!(
            !views
                .primary
                .widgets
                .iter()
                .any(|w| w.id == 2 || w.id == 3 || w.id == 4 || w.id == 5)
        );
    }

    #[test]
    fn region_views_dual_tagged_node_prefers_inspector() {
        // Dual Navigation+Inspector markers on one node → Inspector wins.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                region: "global-navigation".into(),
                role: "inspector".into(),
                agent_id: "nana.workspace.sidebar".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Text,
            WidgetProps {
                label: "panel".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Column,
            WidgetProps {
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 1, None);

        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(
            views.overlapping_ids().is_empty(),
            "overlapping: {:?}",
            views.overlapping_ids()
        );
        assert!(
            views.navigation.widgets.is_empty(),
            "dual-tagged seed must not also project as navigation"
        );
        assert!(views.inspector.widgets.iter().any(|w| w.id == 2));
        assert!(views.inspector.widgets.iter().any(|w| w.id == 3));
        assert!(views.primary.widgets.iter().any(|w| w.id == 4));
        assert!(!views.primary.widgets.iter().any(|w| w.id == 2 || w.id == 3));
    }

    #[test]
    fn region_views_limited_keeps_nav_insp_exclusive_after_truncation() {
        // Truncated nav seed still claims its forest from primary; nested
        // inspector under a kept nav seed stays exclusive of navigation.
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Row, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                region: "global-navigation".into(),
                agent_id: "nav.kept".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Text,
            WidgetProps {
                label: "nav-body".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Column,
            WidgetProps {
                region: "inspector".into(),
                agent_id: "nana.workspace.inspector".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Text,
            WidgetProps {
                label: "insp-body".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::Column,
            WidgetProps {
                region: "section-navigation".into(),
                agent_id: "nav.truncated".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            7,
            WidgetKind::Text,
            WidgetProps {
                label: "trunc".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            8,
            WidgetKind::Column,
            WidgetProps {
                label: "main".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 2, None);
        bridge.insert_child(5, 4, None);
        bridge.insert_child(6, 1, None);
        bridge.insert_child(7, 6, None);
        bridge.insert_child(8, 1, None);

        let snap = bridge.snapshot();
        let views = snap.region_views_limited(1, 1);
        assert!(
            views.overlapping_ids().is_empty(),
            "overlapping after limit: {:?}",
            views.overlapping_ids()
        );
        assert_eq!(views.navigation.roots.len(), 1);
        assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
        assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
        assert!(
            !views
                .navigation
                .widgets
                .iter()
                .any(|w| w.id == 4 || w.id == 5),
            "nested inspector must stay out of limited navigation"
        );
        assert!(
            !views
                .navigation
                .widgets
                .iter()
                .any(|w| w.id == 6 || w.id == 7),
            "truncated nav seed omitted from projection"
        );
        assert!(views.inspector.widgets.iter().any(|w| w.id == 4));
        assert!(views.inspector.widgets.iter().any(|w| w.id == 5));
        assert!(
            !views
                .primary
                .widgets
                .iter()
                .any(|w| { matches!(w.id, 2 | 3 | 4 | 5 | 6 | 7) }),
            "all region-owned ids claimed from primary despite truncation"
        );
        assert!(views.primary.widgets.iter().any(|w| w.id == 8));
    }

    #[test]
    fn untagged_forest_stays_in_primary_without_region_tags() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Column,
            WidgetProps {
                agent_id: "nana.workspace.primary".into(),
                region: "primary".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Text,
            WidgetProps {
                label: "only primary".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        let snap = bridge.snapshot();
        let views = snap.region_views();
        assert!(views.navigation.widgets.is_empty());
        assert!(views.inspector.widgets.is_empty());
        assert_eq!(views.primary.widgets.len(), snap.widgets.len());
        assert!(views.overlapping_ids().is_empty());
    }

    #[test]
    fn migration_component_props_keep_typed_semantics() {
        let props = WidgetProps::from_map(&BTreeMap::from([
            (
                "cardKind".into(),
                nana_js_engine::HostValue::string("raised"),
            ),
            (
                "controlPosition".into(),
                nana_js_engine::HostValue::string("start"),
            ),
            ("autoHeight".into(), nana_js_engine::HostValue::Bool(true)),
            ("loading".into(), nana_js_engine::HostValue::Bool(true)),
            ("invalid".into(), nana_js_engine::HostValue::Bool(true)),
            ("readonly".into(), nana_js_engine::HostValue::Bool(true)),
            ("type".into(), nana_js_engine::HostValue::string("password")),
            ("step".into(), nana_js_engine::HostValue::Number(0.25)),
        ]));

        assert_eq!(props.card_kind, CardKind::Raised);
        assert_eq!(props.control_position, SwitchControlPosition::Start);
        assert!(props.auto_height);
        assert!(props.loading);
        assert!(props.invalid);
        assert!(props.read_only);
        assert!(props.secure);
        assert_eq!(props.step, 0.25);
    }
}
