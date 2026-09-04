//! Semantic props and document projections. UiWorld remains the topology authority.
use super::*;

/// Stable widget id — same numeric space as [`NodeHandle`].
pub type WidgetId = u64;

/// Vue/JS kind facade for host-op parsing. Not a second instantiation ABI.
///
/// Layout, hit-testing, and Scene identity go through Runtime
/// `ComponentRegistry` / `register_component`.
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
    /// Compact icon control → Runtime `IconButton` (`nana.icon-button`).
    IconButton,
    /// Compact selectable chip — Runtime `Chip` (`nana.chip`).
    Chip,
    Input,
    /// Numeric stepper → Runtime `NumberInput` (`nana.number-input`).
    NumberInput,
    Textarea,
    Checkbox,
    /// Exclusive choice → Runtime `SegmentedControl` radio chrome.
    Radio,
    Switch,
    Select,
    /// Multi-select capable field → Runtime `Dropdown` (`nana.dropdown`).
    Dropdown,
    /// Query-filtered field → Runtime `SearchDropdown` (`nana.search-dropdown`).
    SearchDropdown,
    Tabs,
    Segmented,
    Range,
    Card,
    /// Horizontal or vertical rule → Runtime `Divider`.
    Divider,
    /// Image/file preview tile → Runtime `Thumbnail`.
    Thumbnail,
    /// Circular cover-fit host-texture slot → Runtime `Avatar`.
    Avatar,
    /// Vertical item stack → Runtime `List`.
    List,
    ListItem,
    /// Scroll container with optional chrome → Runtime `ScrollView`.
    ScrollView,
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
    /// Desktop workspace chrome → Runtime `DesktopShell`.
    DesktopShell,
    /// Window title bar → Runtime `AppTitleBar`.
    AppTitleBar,
    /// Pane header / tabs / body chrome → Runtime `PaneChrome`.
    PaneChrome,
    /// Sidebar section header + body → Runtime `SidebarSection`.
    SidebarSection,
    /// Sidebar footer region → Runtime `SidebarFooter`.
    SidebarFooter,
    /// Settings content chrome → Runtime `SettingsPage` (header + scroll).
    SettingsPage,
    /// Collapsible settings card → Runtime `SettingsCollapsibleCard`.
    SettingsCollapsibleCard,
    /// Tabular grid → Runtime `Table`.
    Table,
    /// Table row → Runtime `TableRow`.
    TableRow,
    /// Table cell → Runtime `TableCell`.
    TableCell,
    /// Drag-reorder list → Runtime `ReorderList`.
    ReorderList,
    /// Sparkline / series leaf → Runtime `TimeSeriesChart`.
    TimeSeriesChart,
    /// Host-owned texture slot → Runtime `GpuTextureView`.
    GpuTextureView,
    /// In-pass GPU node → Runtime `GpuView`.
    GpuView,
    /// Host-fed video surface → Runtime `Video` (`nana.video`); frames are
    /// pushed by the host through the video surface API.
    Video,
}

/// Single source of truth for [`WidgetKind`]'s three string projections.
///
/// `as_str` is the canonical kind identifier; `aliases` are additional
/// `parse`-accepted spellings (retired names or shared HTML tags); `tag` is
/// the DOM element tag mirrored into Vue. The macro generates `parse`,
/// `as_str`, `element_tag` and [`WidgetKind::ALL`] from one table, so an
/// enum variant without a table row — or a row without a variant — fails to
/// compile instead of silently returning `None` from `parse`.
macro_rules! widget_kind_table {
    ( $(
        $variant:ident => {
            aliases: [$($alias:literal),* $(,)?],
            as_str: $kind:literal,
            tag: $tag:literal
        }
    ),* $(,)? ) => {
        impl WidgetKind {
            /// Parse an explicit `nana-*` / createWidget kind string.
            pub fn parse(raw: &str) -> Option<Self> {
                let original = raw.trim().to_ascii_lowercase();
                let s = original.strip_prefix("nana-").unwrap_or(&original);
                Some(match s {
                    $($($alias)|* | $kind => Self::$variant,)*
                    _ => return None,
                })
            }

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $kind,)*
                }
            }

            pub fn element_tag(self) -> &'static str {
                match self {
                    $(Self::$variant => $tag,)*
                }
            }

            /// Every variant, in declaration order.
            pub const ALL: &'static [WidgetKind] = &[$(Self::$variant),*];
        }
    };
}

widget_kind_table! {
    Column => { aliases: [], as_str: "column", tag: "nana-column" },
    Row => { aliases: [], as_str: "row", tag: "nana-row" },
    Box => { aliases: [], as_str: "box", tag: "nana-box" },
    Text => { aliases: [], as_str: "text", tag: "nana-text" },
    Button => { aliases: [], as_str: "button", tag: "button" },
    IconButton => { aliases: [], as_str: "icon-button", tag: "nana-icon-button" },
    Chip => { aliases: [], as_str: "chip", tag: "nana-chip" },
    Input => { aliases: ["input"], as_str: "text-input", tag: "input" },
    NumberInput => { aliases: [], as_str: "number-input", tag: "input" },
    Textarea => { aliases: [], as_str: "textarea", tag: "textarea" },
    Checkbox => { aliases: [], as_str: "checkbox", tag: "checkbox" },
    Radio => { aliases: [], as_str: "radio", tag: "input" },
    Switch => { aliases: [], as_str: "switch", tag: "nana-switch" },
    Select => { aliases: [], as_str: "select", tag: "select" },
    Dropdown => { aliases: [], as_str: "dropdown", tag: "nana-dropdown" },
    SearchDropdown => { aliases: [], as_str: "search-dropdown", tag: "search-dropdown" },
    Tabs => { aliases: [], as_str: "tabs", tag: "nana-tabs" },
    Segmented => { aliases: [], as_str: "segmented", tag: "nana-segmented" },
    Range => { aliases: [], as_str: "range-field", tag: "range-field" },
    Card => { aliases: [], as_str: "card", tag: "nana-card" },
    Divider => { aliases: ["hr"], as_str: "divider", tag: "hr" },
    Thumbnail => { aliases: [], as_str: "thumbnail", tag: "nana-thumbnail" },
    Avatar => { aliases: [], as_str: "avatar", tag: "nana-avatar" },
    List => { aliases: [], as_str: "list", tag: "ul" },
    ListItem => { aliases: [], as_str: "list-item", tag: "li" },
    ScrollView => { aliases: [], as_str: "scroll-view", tag: "nana-scroll-view" },
    EmptyState => { aliases: [], as_str: "empty-state", tag: "nana-empty-state" },
    StatusBadge => { aliases: [], as_str: "status-badge", tag: "nana-status-badge" },
    ValidationMessage => { aliases: [], as_str: "validation-message", tag: "nana-validation-message" },
    LabeledValue => { aliases: [], as_str: "labeled-value", tag: "nana-labeled-value" },
    Progress => { aliases: [], as_str: "progress", tag: "progress" },
    Spinner => { aliases: [], as_str: "spinner", tag: "nana-spinner" },
    FormField => { aliases: [], as_str: "form-field", tag: "nana-form-field" },
    InteractiveCard => { aliases: [], as_str: "interactive-card", tag: "nana-interactive-card" },
    Skeleton => { aliases: [], as_str: "skeleton", tag: "nana-skeleton" },
    LevelMeter => { aliases: [], as_str: "level-meter", tag: "meter" },
    SidebarFrame => { aliases: [], as_str: "sidebar-frame", tag: "nana-sidebar-frame" },
    SidebarRow => { aliases: [], as_str: "sidebar-row", tag: "nana-sidebar-row" },
    SettingsRow => { aliases: [], as_str: "settings-row", tag: "nana-settings-row" },
    SettingsCard => { aliases: [], as_str: "settings-card", tag: "nana-settings-card" },
    Icon => { aliases: [], as_str: "icon", tag: "nana-icon" },
    Dialog => { aliases: [], as_str: "dialog", tag: "dialog" },
    Drawer => { aliases: [], as_str: "drawer", tag: "nana-drawer" },
    Popover => { aliases: [], as_str: "popover", tag: "nana-popover" },
    ContextMenu => { aliases: [], as_str: "context-menu", tag: "nana-context-menu" },
    Toast => { aliases: [], as_str: "toast", tag: "nana-toast" },
    Tooltip => { aliases: [], as_str: "tooltip", tag: "nana-tooltip" },
    ActionMenu => { aliases: [], as_str: "action-menu", tag: "nana-action-menu" },
    ActionMenuItem => { aliases: [], as_str: "action-menu-item", tag: "nana-action-menu-item" },
    XYPad => { aliases: [], as_str: "xy-pad", tag: "nana-xy-pad" },
    QrCode => { aliases: [], as_str: "qr-code", tag: "nana-qr-code" },
    CommandPalette => { aliases: [], as_str: "command-palette", tag: "nana-command-palette" },
    TreeView => { aliases: [], as_str: "tree-view", tag: "nana-tree-view" },
    CalendarHeatmap => { aliases: [], as_str: "calendar-heatmap", tag: "nana-calendar-heatmap" },
    ImageViewer => { aliases: [], as_str: "image-viewer", tag: "nana-image-viewer" },
    NativeMarkdown => { aliases: [], as_str: "native-markdown", tag: "nana-native-markdown" },
    GraphCanvas => { aliases: [], as_str: "graph-canvas", tag: "nana-graph-canvas" },
    Workspace => { aliases: [], as_str: "workspace", tag: "nana-workspace" },
    Dock => { aliases: [], as_str: "dock", tag: "nana-dock" },
    SplitPane => { aliases: [], as_str: "split-pane", tag: "nana-split-pane" },
    AppShell => { aliases: [], as_str: "app-shell", tag: "nana-app-shell" },
    DesktopShell => { aliases: [], as_str: "desktop-shell", tag: "nana-desktop-shell" },
    AppTitleBar => { aliases: [], as_str: "app-title-bar", tag: "nana-app-title-bar" },
    PaneChrome => { aliases: [], as_str: "pane-chrome", tag: "nana-pane-chrome" },
    SidebarSection => { aliases: [], as_str: "sidebar-section", tag: "nana-sidebar-section" },
    SidebarFooter => { aliases: [], as_str: "sidebar-footer", tag: "nana-sidebar-footer" },
    SettingsPage => { aliases: [], as_str: "settings-page", tag: "nana-settings-page" },
    SettingsCollapsibleCard => { aliases: [], as_str: "settings-collapsible-card", tag: "details" },
    Table => { aliases: [], as_str: "table", tag: "table" },
    TableRow => { aliases: ["table-row"], as_str: "tr", tag: "tr" },
    TableCell => { aliases: ["th", "table-cell"], as_str: "td", tag: "td" },
    ReorderList => { aliases: [], as_str: "reorder-list", tag: "nana-reorder-list" },
    TimeSeriesChart => { aliases: [], as_str: "time-series-chart", tag: "nana-time-series-chart" },
    GpuTextureView => { aliases: [], as_str: "gpu", tag: "nana-gpu" },
    GpuView => { aliases: [], as_str: "gpu-view", tag: "nana-gpu-view" },
    Video => { aliases: [], as_str: "video", tag: "nana-video" },
}

impl WidgetKind {
    /// Select / Dropdown / SearchDropdown — one option list, three Runtime types.
    pub fn is_choice_field(self) -> bool {
        matches!(self, Self::Select | Self::Dropdown | Self::SearchDropdown)
    }

    pub fn is_layout(self) -> bool {
        matches!(
            self,
            Self::Column
                | Self::Row
                | Self::Box
                | Self::SidebarFrame
                | Self::SidebarSection
                | Self::SidebarFooter
                | Self::Card
                | Self::List
                | Self::ScrollView
                | Self::Table
                | Self::DesktopShell
                | Self::PaneChrome
                | Self::SettingsCard
                | Self::SettingsCollapsibleCard
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
    /// Registered native Runtime component name without the `nana-` prefix.
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
            | "values" | "series" | "viewport" | "selection" | "layout" | "root" | "axis"
            | "size" | "default-size" | "min" | "max" | "settings" | "tab" | "hide-header"
            | "content-padding" | "content-gap" => {
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
            "disabled" => {
                self.disabled = host_truthy(value);
                if self.disabled {
                    self.attrs.insert("disabled".into(), String::new());
                } else {
                    self.attrs.remove("disabled");
                }
            }
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
                if !self.toggled {
                    self.attrs.remove("checked");
                } else if key == "checked" {
                    self.attrs.insert("checked".into(), String::new());
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
            "poster" => {
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("poster");
                } else {
                    self.attrs.insert("poster".into(), s);
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
                // resvg HostTexture serializes these attrs; there is no Vue/CSS
                // extract into Scene StrokePattern.path_length.
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
            "axis" | "axes" | "scrollbars" | "scrollbar" | "orientation" | "thickness"
            | "inset" | "precision" | "aspect" | "header" | "column-header" | "columnheader"
            | "tree-drop" | "treedrop" | "spacing" | "collapsible" | "expanded" | "count"
            | "window-controls" | "windowcontrols" | "maximized" | "center-width"
            | "centerwidth" => {
                let raw = host_string(value);
                if raw.is_empty() && !matches!(value, nana_js_engine::HostValue::Bool(true)) {
                    self.attrs.remove(&key);
                } else {
                    self.attrs.insert(key, raw);
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
                if t.is_empty() {
                    self.attrs.remove("type");
                } else {
                    self.attrs.insert("type".into(), t.clone());
                }
                if t == "checkbox" {
                    self.toggled = self.toggled || host_truthy(value);
                }
            }
            "name" | "for" | "html-for" | "htmlfor" | "href" => {
                let s = host_string(value);
                let attr = if key == "html-for" || key == "htmlfor" {
                    "for"
                } else {
                    key.as_str()
                };
                if s.is_empty() {
                    self.attrs.remove(attr);
                } else {
                    self.attrs.insert(attr.to_string(), s);
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
                // Uniform `gap` prop resets axis longhands from prior `style` (CSS shorthand).
                self.strip_inline_css_properties(&["gap", "row-gap", "column-gap"]);
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
            "width" | "height" => {
                let dimension = host_string(value);
                if let Ok(px) = dimension.trim().parse::<f32>()
                    && px.is_finite()
                {
                    // HTML attributes arrive as strings, while bound Vue props
                    // can be numbers. Both use pixel dimensions.
                    self.record_prop_style(&key, &format!("{px}px"));
                } else if !dimension.is_empty() {
                    self.record_prop_style(&key, &dimension);
                }
                persist_svg_length_attr(self, &key, value);
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
            "dir" => {
                // Presentational hint: persist for cascade seed + `[dir]` selectors.
                // Used `LayoutStyle.dir` is applied in `reapply_layout_for_inner`.
                let s = host_string(value);
                if s.is_empty() {
                    self.attrs.remove("dir");
                } else {
                    self.attrs.insert("dir".into(), s);
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

    fn strip_inline_css_properties(&mut self, properties: &[&str]) {
        if self.inline_style.trim().is_empty() || properties.is_empty() {
            return;
        }
        let mut kept = Vec::new();
        'decl: for decl in self.inline_style.split(';') {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            let Some((k, _)) = decl.split_once(':') else {
                kept.push(decl.to_string());
                continue;
            };
            for property in properties {
                if k.trim().eq_ignore_ascii_case(property) {
                    continue 'decl;
                }
            }
            kept.push(decl.to_string());
        }
        self.inline_style = kept.join("; ");
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

/// User / host action crossing the Vue ↔ Runtime boundary.
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
    /// Event emitted by a native Runtime component registered into the Vue tree.
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

pub(super) fn is_framework_native_prop(key: &str) -> bool {
    matches!(
        key,
        "class"
            | "classname"
            | "style"
            | "id"
            | "role"
            | "hidden"
            | "dir"
            | "disabled"
            | "tabindex"
            | "ref"
            | "ref-key"
            | "ref-for"
    ) || key.starts_with("aria-")
        || key.starts_with("data-")
        || key.starts_with("on")
}

/// Mutation footprint accumulated since the previous
/// [`MessageBridge::snapshot`] — the unit of incremental semantic sync.
///
/// Empty changes with a bumped revision mean a consumer took the footprint
/// already; the sync side answers that defensively with a full pass.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct SnapshotChanges {
    /// Widgets whose props/kind/label changed, plus affected subtrees.
    pub(crate) dirty: std::collections::BTreeSet<WidgetId>,
    /// Tree shape changed (insert / remove / reparent / roots).
    pub(crate) structure_changed: bool,
    /// Whole-document invalidation (theme, global cascade, viewport CB).
    pub(crate) all: bool,
}

impl SnapshotChanges {
    /// `true` when the sync must project every widget.
    pub fn needs_full_pass(&self) -> bool {
        self.all || self.structure_changed
    }
}

/// Flat snapshot for Scene host view (pre-order under each root).
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSnapshot {
    pub revision: u64,
    pub theme: ThemeMode,
    /// Appearance backdrop fields synced from L1 document dataset/style.
    pub appearance: AppearanceSettings,
    pub roots: Vec<WidgetId>,
    pub widgets: Vec<SemanticWidget>,
    /// Mutations accumulated since the previous snapshot of this bridge.
    pub changes: SnapshotChanges,
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
            changes: SnapshotChanges::default(),
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
            changes: SnapshotChanges::default(),
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
            changes: SnapshotChanges::default(),
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
pub(super) fn widget_is_navigation_region(widget: &SemanticWidget) -> bool {
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
pub(super) fn widget_is_layout_chrome(widget: &SemanticWidget) -> bool {
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
pub(super) fn widget_region_token(widget: &SemanticWidget) -> String {
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

pub(super) fn widget_is_inspector_region(widget: &SemanticWidget) -> bool {
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

/// One L1/L2 semantic projection node. Not a Runtime entity.
///
/// `parent` / `children` are a CSS-cascade working index. Before Scene paint,
/// [`crate::NanaTreeDocument::apply_runtime_hierarchy`] overwrites them from
/// `UiWorld`, which is the only hierarchy authority.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticWidget {
    pub id: WidgetId,
    pub kind: WidgetKind,
    pub props: WidgetProps,
    pub children: Vec<WidgetId>,
    pub parent: Option<WidgetId>,
}
