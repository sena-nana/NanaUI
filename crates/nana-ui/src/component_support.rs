//! Public, read-only component migration capabilities.
//!
//! This catalog describes support and promotion evidence and is the single
//! source for NanaUI's internal default-backend decision. It must not be used
//! to maintain parallel application state.

use std::fmt;

/// Stable semantic identity of a NanaUI component across backend migrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentId(&'static str);

impl ComponentId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// Evidence state for promoting a component onto the Runtime / UiScene default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum ComponentMigrationState {
    /// Runtime promotion is incomplete; this entry is not a product-default path.
    Compatibility,
    /// A Runtime implementation exists, but has not passed every promotion gate.
    RuntimeCandidate,
    /// Runtime behavior, layout and reviewed visuals qualify as the default path.
    RuntimeQualified,
}

impl ComponentMigrationState {
    /// Migration is monotonic. A regression must be fixed instead of hidden by
    /// silently downgrading the advertised state.
    pub const fn allows_transition_to(self, next: Self) -> bool {
        next as u8 >= self as u8
    }
}

/// Stable behavior that consumers can rely on for a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ComponentCapability {
    Render,
    Pointer,
    Keyboard,
    Focus,
    Ime,
    Accessibility,
    Animation,
    Overlay,
    Gpu,
    Persistence,
}

/// Broad catalog grouping; this does not own application navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ComponentFamily {
    Primitive,
    Control,
    Data,
    Feedback,
    Navigation,
    Overlay,
    Workspace,
    Media,
    Gpu,
}

/// Immutable support facts for one public component identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentSupport {
    pub id: ComponentId,
    pub name: &'static str,
    pub family: ComponentFamily,
    pub migration: ComponentMigrationState,
    /// Cargo feature required by the current public implementation, if any.
    pub required_feature: Option<&'static str>,
    /// Whether the required implementation is compiled in this build.
    pub compiled: bool,
    pub capabilities: &'static [ComponentCapability],
}

impl ComponentSupport {
    pub fn supports(&self, capability: ComponentCapability) -> bool {
        self.capabilities.contains(&capability)
    }
}

macro_rules! component_catalog {
    ($(
        $constant:ident => {
            id: $id:literal,
            name: $name:literal,
            family: $family:ident,
            migration: $migration:ident,
            feature: $feature:expr,
            compiled: $compiled:expr,
            capabilities: [$($capability:ident),* $(,)?]
        }
    ),* $(,)?) => {
        /// Stable component identities. New identities are additive; existing
        /// values must not be renamed as part of a backend migration.
        pub mod component_ids {
            use super::ComponentId;

            $(pub const $constant: ComponentId = ComponentId::new($id);)*
        }

        static COMPONENT_CATALOG: &[ComponentSupport] = &[
            $(ComponentSupport {
                id: component_ids::$constant,
                name: $name,
                family: ComponentFamily::$family,
                migration: ComponentMigrationState::$migration,
                required_feature: $feature,
                compiled: $compiled,
                capabilities: &[$(ComponentCapability::$capability),*],
            }),*
        ];
    };
}

component_catalog! {
    TEXT => { id: "text", name: "Text", family: Primitive, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    BUTTON => { id: "button", name: "Button", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    TEXT_INPUT => { id: "text-input", name: "TextInput", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility] },
    CHECKBOX => { id: "checkbox", name: "Checkbox", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    ICON_BUTTON => { id: "icon-button", name: "IconButton", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    CARD => { id: "card", name: "Card", family: Primitive, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility, Animation] },
    SWITCH => { id: "switch", name: "Switch", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Animation] },
    TEXTAREA => { id: "textarea", name: "Textarea", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility] },
    HOSTED_TEXTAREA => { id: "hosted-textarea", name: "HostedTextarea", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility] },
    RANGE_FIELD => { id: "range-field", name: "RangeField", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SELECT => { id: "select", name: "Select", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    SEGMENTED_CONTROL => { id: "segmented-control", name: "SegmentedControl", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    TABS => { id: "tabs", name: "Tabs", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    REORDER_LIST => { id: "reorder-list", name: "ReorderList", family: Navigation, migration: RuntimeQualified, feature: Some("controls"), compiled: cfg!(feature = "controls"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    CALENDAR_HEATMAP => { id: "calendar-heatmap", name: "CalendarHeatmap", family: Data, migration: RuntimeQualified, feature: Some("calendar"), compiled: cfg!(feature = "calendar"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    TIME_SERIES_CHART => { id: "time-series-chart", name: "TimeSeriesChart", family: Data, migration: RuntimeQualified, feature: Some("charts"), compiled: cfg!(feature = "charts"), capabilities: [Render, Accessibility] },
    PROGRESS => { id: "progress", name: "Progress", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SPINNER => { id: "spinner", name: "Spinner", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Animation, Accessibility] },
    SKELETON => { id: "skeleton", name: "Skeleton", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Animation] },
    LEVEL_METER => { id: "level-meter", name: "LevelMeter", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    STATUS_BADGE => { id: "status-badge", name: "StatusBadge", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    VALIDATION_MESSAGE => { id: "validation-message", name: "ValidationMessage", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    TOAST => { id: "toast", name: "Toast", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    COMMAND_PALETTE => { id: "command-palette", name: "CommandPalette", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility, Overlay] },
    DIALOG => { id: "dialog", name: "Dialog", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    CONFIRM_DIALOG => { id: "confirm-dialog", name: "ConfirmDialog", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    DRAWER => { id: "drawer", name: "Drawer", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    TOOLTIP => { id: "tooltip", name: "Tooltip", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility, Overlay] },
    POPOVER => { id: "popover", name: "Popover", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    ACTION_MENU => { id: "action-menu", name: "ActionMenu", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    CONTEXT_MENU => { id: "context-menu", name: "ContextMenu", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    ACTION_MENU_ITEM => { id: "action-menu-item", name: "ActionMenuItem", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    ANCHORED_ACTION_MENU => { id: "anchored-action-menu", name: "AnchoredActionMenu", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    OVERLAY_HOST => { id: "overlay-host", name: "OverlayHost", family: Overlay, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    DROPDOWN => { id: "dropdown", name: "Dropdown", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    SEARCH_DROPDOWN => { id: "search-dropdown", name: "SearchDropdown", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility, Overlay] },
    TREE_VIEW => { id: "tree-view", name: "TreeView", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SIDEBAR_FRAME => { id: "sidebar-frame", name: "SidebarFrame", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    SIDEBAR_SECTION => { id: "sidebar-section", name: "SidebarSection", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Animation] },
    SIDEBAR_ROW => { id: "sidebar-row", name: "SidebarRow", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SIDEBAR_FOOTER => { id: "sidebar-footer", name: "SidebarFooter", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    KEY_CAPTURE_LAYER => { id: "key-capture-layer", name: "KeyCaptureLayer", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Keyboard, Focus, Accessibility] },
    KEYMAP_LAYER => { id: "keymap-layer", name: "KeymapLayer", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Keyboard, Focus] },
    NATIVE_MARKDOWN => { id: "native-markdown", name: "NativeMarkdown", family: Data, migration: RuntimeQualified, feature: Some("rich-text"), compiled: cfg!(feature = "rich-text"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SELECTABLE_RICH_TEXT => { id: "selectable-rich-text", name: "SelectableRichText", family: Data, migration: RuntimeQualified, feature: Some("rich-text"), compiled: cfg!(feature = "rich-text"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    QR_CODE => { id: "qr-code", name: "QrCodeCanvas", family: Data, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    GRAPH_CANVAS => { id: "graph-canvas", name: "GraphCanvas", family: Data, migration: RuntimeQualified, feature: Some("graph-canvas"), compiled: cfg!(feature = "graph-canvas"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Gpu] },
    GRAPH_MINIMAP => { id: "graph-minimap", name: "GraphMinimap", family: Data, migration: RuntimeQualified, feature: Some("graph-canvas"), compiled: cfg!(feature = "graph-canvas"), capabilities: [Render, Pointer, Accessibility] },
    IMAGE_VIEWER => { id: "image-viewer", name: "ImageViewer", family: Media, migration: RuntimeQualified, feature: Some("image-viewer"), compiled: cfg!(feature = "image-viewer"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    XY_PAD => { id: "xy-pad", name: "XYPad", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    FORM_FIELD => { id: "form-field", name: "FormField", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    LABELED_VALUE => { id: "labeled-value", name: "LabeledValue", family: Data, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    EMPTY_STATE => { id: "empty-state", name: "EmptyState", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    INTERACTIVE_CARD => { id: "interactive-card", name: "InteractiveCard", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    LIST_ITEM => { id: "list-item", name: "ListItem", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    THUMBNAIL => { id: "thumbnail", name: "Thumbnail", family: Primitive, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Gpu, Animation, Accessibility] },
    DOCK_PANEL => { id: "dock-panel", name: "DockPanel", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    WORKSPACE => { id: "workspace", name: "Workspace", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Animation, Persistence] },
    DOCK => { id: "dock", name: "Dock", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Animation, Persistence] },
    SPLIT_PANE => { id: "split-pane", name: "SplitPane", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    PANE_CHROME => { id: "pane-chrome", name: "PaneChrome", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    PANE_TREE => { id: "pane-tree", name: "PaneTree", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    APP_SHELL => { id: "app-shell", name: "AppShell", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    APP_TITLE_BAR => { id: "app-title-bar", name: "AppTitleBar", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SETTINGS => { id: "settings", name: "Settings", family: Workspace, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    APPEARANCE_SECTION => { id: "appearance-section", name: "AppearanceSection", family: Workspace, migration: RuntimeQualified, feature: Some("settings-components"), compiled: cfg!(feature = "settings-components"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    ABOUT_SECTION => { id: "about-section", name: "AboutSection", family: Workspace, migration: RuntimeQualified, feature: Some("settings-components"), compiled: cfg!(feature = "settings-components"), capabilities: [Render, Accessibility] },
    SETTINGS_COLLAPSIBLE_CARD => { id: "settings-collapsible-card", name: "SettingsCollapsibleCard", family: Workspace, migration: RuntimeQualified, feature: Some("settings-components"), compiled: cfg!(feature = "settings-components"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    GPU_VIEW => { id: "gpu-view", name: "GpuView", family: Gpu, migration: RuntimeQualified, feature: Some("gpu"), compiled: cfg!(feature = "gpu"), capabilities: [Render, Pointer, Gpu] },
    GPU_TEXTURE_VIEW => { id: "gpu-texture-view", name: "GpuTextureView", family: Gpu, migration: RuntimeQualified, feature: Some("gpu"), compiled: cfg!(feature = "gpu"), capabilities: [Render, Gpu] },
}

/// Complete catalog for this NanaUI build.
pub const fn component_catalog() -> &'static [ComponentSupport] {
    COMPONENT_CATALOG
}

/// Look up support facts without selecting or changing a renderer.
pub fn component_support(id: ComponentId) -> Option<&'static ComponentSupport> {
    COMPONENT_CATALOG.iter().find(|support| support.id == id)
}

/// Internal default-backend routing derived from the same declaration as the
/// public catalog. Only compiled `RuntimeQualified` entries take the Runtime path.
#[doc(hidden)]
pub fn component_uses_runtime(id: ComponentId) -> bool {
    component_support(id).is_some_and(|support| {
        support.compiled && support.migration == ComponentMigrationState::RuntimeQualified
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn catalog_has_unique_stable_identities_and_real_capabilities() {
        let mut ids = BTreeSet::new();
        for support in component_catalog() {
            assert!(!support.id.as_str().is_empty());
            assert!(!support.name.is_empty());
            assert!(!support.capabilities.is_empty());
            assert!(ids.insert(support.id));
            assert_eq!(component_support(support.id), Some(support));
        }
    }

    #[test]
    fn qualified_components_route_only_reviewed_runtime_paths() {
        for id in [
            component_ids::TEXT,
            component_ids::BUTTON,
            component_ids::TEXT_INPUT,
            component_ids::CHECKBOX,
            component_ids::ICON_BUTTON,
            component_ids::SWITCH,
            component_ids::CARD,
            component_ids::LIST_ITEM,
            component_ids::THUMBNAIL,
            component_ids::RANGE_FIELD,
            component_ids::SEGMENTED_CONTROL,
            component_ids::STATUS_BADGE,
            component_ids::VALIDATION_MESSAGE,
            component_ids::EMPTY_STATE,
            component_ids::LABELED_VALUE,
            component_ids::PROGRESS,
            component_ids::SPINNER,
            component_ids::TABS,
            component_ids::SKELETON,
            component_ids::LEVEL_METER,
            component_ids::FORM_FIELD,
            component_ids::INTERACTIVE_CARD,
            component_ids::TEXTAREA,
            component_ids::HOSTED_TEXTAREA,
            component_ids::REORDER_LIST,
            component_ids::CALENDAR_HEATMAP,
            component_ids::TIME_SERIES_CHART,
            component_ids::KEY_CAPTURE_LAYER,
            component_ids::KEYMAP_LAYER,
            component_ids::NATIVE_MARKDOWN,
            component_ids::SELECTABLE_RICH_TEXT,
            component_ids::IMAGE_VIEWER,
            component_ids::GRAPH_CANVAS,
            component_ids::GPU_VIEW,
            component_ids::GPU_TEXTURE_VIEW,
            component_ids::TOOLTIP,
            component_ids::DIALOG,
            component_ids::CONFIRM_DIALOG,
            component_ids::DRAWER,
            component_ids::TOAST,
            component_ids::XY_PAD,
            component_ids::QR_CODE,
            component_ids::SELECT,
            component_ids::POPOVER,
            component_ids::ACTION_MENU,
            component_ids::ACTION_MENU_ITEM,
            component_ids::ANCHORED_ACTION_MENU,
            component_ids::CONTEXT_MENU,
            component_ids::OVERLAY_HOST,
            component_ids::DROPDOWN,
            component_ids::SEARCH_DROPDOWN,
            component_ids::COMMAND_PALETTE,
            component_ids::TREE_VIEW,
            component_ids::SIDEBAR_ROW,
            component_ids::SETTINGS,
            component_ids::SIDEBAR_FRAME,
            component_ids::SIDEBAR_SECTION,
            component_ids::SIDEBAR_FOOTER,
            component_ids::APPEARANCE_SECTION,
            component_ids::ABOUT_SECTION,
            component_ids::SETTINGS_COLLAPSIBLE_CARD,
            component_ids::WORKSPACE,
            component_ids::DOCK,
            component_ids::DOCK_PANEL,
            component_ids::SPLIT_PANE,
            component_ids::PANE_CHROME,
            component_ids::PANE_TREE,
            component_ids::APP_SHELL,
            component_ids::APP_TITLE_BAR,
        ] {
            let support = component_support(id).expect("qualified component is cataloged");
            assert_eq!(support.migration, ComponentMigrationState::RuntimeQualified);
            assert_eq!(component_uses_runtime(id), support.compiled);
        }
    }

    #[test]
    fn catalog_has_no_runtime_candidates() {
        let leftover: Vec<_> = component_catalog()
            .iter()
            .filter(|support| support.migration == ComponentMigrationState::RuntimeCandidate)
            .map(|support| support.id)
            .collect();
        assert!(
            leftover.is_empty(),
            "catalog still lists RuntimeCandidate entries: {leftover:?}"
        );
    }

    #[test]
    fn hosted_textarea_public_export_is_the_runtime_highlighter() {
        let hosted = component_support(component_ids::HOSTED_TEXTAREA)
            .expect("hosted textarea is cataloged");
        assert_eq!(hosted.migration, ComponentMigrationState::RuntimeQualified);
        assert!(component_uses_runtime(component_ids::HOSTED_TEXTAREA));
        let _: nana_ui_runtime::HostedTextarea = crate::HostedTextarea::new("fn main() {}", "rs");
        let _: nana_ui_runtime::HostedTextarea =
            crate::components::HostedTextarea::new("fn main() {}", "rs");
        let _: nana_ui_runtime::KeyCaptureLayer = crate::KeyCaptureLayer::new();
        let _: nana_ui_runtime::KeymapLayer = crate::KeymapLayer::new(
            nana_ui_runtime::Keymap::new([]),
            nana_ui_core::KeyContext::default(),
            nana_ui_runtime::ActionRegistry::new(),
        );
    }

    #[cfg(all(
        feature = "calendar",
        feature = "charts",
        feature = "controls",
        feature = "rich-text",
        feature = "image-viewer"
    ))]
    #[test]
    fn candidate_cutover_public_exports_include_new_runtime_leaves() {
        let _: nana_ui_runtime::CalendarHeatmap = crate::CalendarHeatmap::new([]);
        let _: nana_ui_runtime::TimeSeriesChart = crate::TimeSeriesChart::new([1.0]);
        let _: nana_ui_runtime::ReorderList = crate::ReorderList::new([]);
        let _: nana_ui_runtime::NativeMarkdown = crate::NativeMarkdown::new();
        let _: nana_ui_runtime::SelectableRichText = crate::SelectableRichText::new([]);
        let _: nana_ui_runtime::ImageViewer =
            crate::ImageViewer::new(nana_ui_runtime::ImageViewerContent::None);
        let _: nana_ui_runtime::CalendarHeatmap = crate::components::CalendarHeatmap::new([]);
        let _: nana_ui_runtime::NativeMarkdown = crate::components::NativeMarkdown::new();
    }

    #[cfg(all(feature = "graph-canvas", feature = "gpu"))]
    #[test]
    fn graph_and_gpu_public_exports_are_runtime_components() {
        let _: nana_ui_runtime::GraphCanvas =
            crate::GraphCanvas::new("main", nana_ui_core::GraphModel::empty());
        let _: nana_ui_runtime::GraphMinimap =
            crate::GraphMinimap::new(nana_ui_core::GraphModel::empty());
        let _: nana_ui_runtime::GpuView = crate::GpuView::new(1);
        let _: nana_ui_runtime::GpuTextureView = crate::GpuTextureView::new("slot");
        let _: nana_ui_runtime::Thumbnail = crate::Thumbnail::empty();
        let _: nana_ui_runtime::Thumbnail = crate::components::Thumbnail::empty();
        let _: nana_ui_runtime::GraphCanvas =
            crate::components::GraphCanvas::new("main", nana_ui_core::GraphModel::empty());
    }

    #[test]
    fn first_batch_public_exports_are_runtime_components() {
        let _: nana_ui_runtime::Text = crate::Text::new("Status");
        let _: nana_ui_runtime::Button = crate::Button::new("Run");
        let _: nana_ui_runtime::TextInput = crate::TextInput::new("main");
        let _: nana_ui_runtime::Checkbox = crate::Checkbox::new("Enabled", true);

        let _: nana_ui_runtime::Text = crate::components::Text::new("Status");
        let _: nana_ui_runtime::Button = crate::components::Button::new("Run");
        let _: nana_ui_runtime::TextInput = crate::components::TextInput::new("main");
        let _: nana_ui_runtime::Checkbox = crate::components::Checkbox::new("Enabled", true);
    }

    #[test]
    fn third_batch_public_exports_are_runtime_components() {
        let _: nana_ui_runtime::StatusBadge =
            crate::StatusBadge::new("Ready", nana_ui_runtime::StatusTone::Neutral);
        let _: nana_ui_runtime::ValidationMessage =
            crate::ValidationMessage::new("Required", nana_ui_runtime::ValidationIntent::Danger);
        let _: nana_ui_runtime::EmptyState = crate::EmptyState::new("Nothing here");
        let _: nana_ui_runtime::LabeledValue = crate::LabeledValue::new("Revision", "42");
        let _: nana_ui_runtime::SegmentedControl = crate::SegmentedControl::new();

        let _: nana_ui_runtime::StatusBadge =
            crate::components::StatusBadge::new("Ready", nana_ui_runtime::StatusTone::Neutral);
        let _: nana_ui_runtime::ValidationMessage = crate::components::ValidationMessage::new(
            "Required",
            nana_ui_runtime::ValidationIntent::Danger,
        );
        let _: nana_ui_runtime::EmptyState = crate::components::EmptyState::new("Nothing here");
        let _: nana_ui_runtime::LabeledValue =
            crate::components::LabeledValue::new("Revision", "42");
        let _: nana_ui_runtime::SegmentedControl = crate::components::SegmentedControl::new();
    }

    #[test]
    fn fourth_batch_public_exports_are_runtime_components() {
        let _: nana_ui_runtime::Progress = crate::Progress::new(1.0, 2.0);
        let _: nana_ui_runtime::Spinner = crate::Spinner::new("Loading");
        let _: nana_ui_runtime::Thumbnail = crate::Thumbnail::empty();
        let _: nana_ui_runtime::Skeleton =
            crate::Skeleton::new(nana_ui_core::LengthSpec::Fill, 16.0);
        let _: nana_ui_runtime::LevelMeter = crate::LevelMeter::new(0.5);
        let _: nana_ui_runtime::FormField = crate::FormField::new("Name");
        let _: nana_ui_runtime::InteractiveCard = crate::InteractiveCard::new();
        let _: nana_ui_runtime::Tabs = crate::Tabs::new("code");

        let _: nana_ui_runtime::Progress = crate::components::Progress::new(1.0, 2.0);
        let _: nana_ui_runtime::Spinner = crate::components::Spinner::new("Loading");
        let _: nana_ui_runtime::Thumbnail = crate::components::Thumbnail::empty();
        let _: nana_ui_runtime::Skeleton =
            crate::components::Skeleton::new(nana_ui_core::LengthSpec::Fill, 16.0);
        let _: nana_ui_runtime::LevelMeter = crate::components::LevelMeter::new(0.5);
        let _: nana_ui_runtime::FormField = crate::components::FormField::new("Name");
        let _: nana_ui_runtime::InteractiveCard = crate::components::InteractiveCard::new();
        let _: nana_ui_runtime::Tabs = crate::components::Tabs::new("code");
    }

    #[test]
    fn candidate_cutover_public_exports_are_runtime_components() {
        let _: nana_ui_runtime::TextArea = crate::Textarea::new("notes");
        let _: nana_ui_runtime::Tooltip = crate::Tooltip::new("Hint");
        let _: nana_ui_runtime::Dialog = crate::Dialog::new("Rename");
        let _: nana_ui_runtime::ConfirmDialog =
            crate::ConfirmDialog::new("Delete", "This cannot be undone.");
        let _: nana_ui_runtime::Drawer = crate::Drawer::new("Inspector");
        let _: nana_ui_runtime::Toast =
            crate::Toast::new("Saved", nana_ui_runtime::ToastTone::Info);
        let _: nana_ui_runtime::XYPad = crate::XYPad::new(nana_ui_core::XYPadValue::new(0.5, 0.5));
        let _: nana_ui_runtime::QrCode =
            crate::QrCode::from_modules(vec![false], 1, 64.0).expect("single module encodes");
        let _: nana_ui_runtime::Select = crate::Select::new(Some("code"));
        let _: nana_ui_runtime::Popover = crate::Popover::new();
        let _: nana_ui_runtime::ActionMenu = crate::ActionMenu::new();
        let _: nana_ui_runtime::ActionMenuItem = crate::ActionMenuItem::new("Rename");
        let _: nana_ui_runtime::AnchoredActionMenu = crate::AnchoredActionMenu::new(24.0, 36.0);
        let _: nana_ui_runtime::ContextMenu = crate::ContextMenu::new(24.0, 36.0);
        let _: nana_ui_runtime::OverlayHost = crate::OverlayHost::new();
        let _: nana_ui_runtime::Dropdown = crate::Dropdown::single(Some("code"));
        let _: nana_ui_runtime::SearchDropdown = crate::SearchDropdown::new(None::<&str>);
        let _: nana_ui_runtime::CommandPalette = crate::CommandPalette::new("命令面板", []);
        let _: nana_ui_runtime::TreeView = crate::TreeView::new([]);

        let _: nana_ui_runtime::TextArea = crate::components::Textarea::new("notes");
        let _: nana_ui_runtime::Tooltip = crate::components::Tooltip::new("Hint");
        let _: nana_ui_runtime::Dialog = crate::components::Dialog::new("Rename");
        let _: nana_ui_runtime::ConfirmDialog =
            crate::components::ConfirmDialog::new("Delete", "This cannot be undone.");
        let _: nana_ui_runtime::Drawer = crate::components::Drawer::new("Inspector");
        let _: nana_ui_runtime::Toast =
            crate::components::Toast::new("Saved", nana_ui_runtime::ToastTone::Info);
        let _: nana_ui_runtime::XYPad =
            crate::components::XYPad::new(nana_ui_core::XYPadValue::new(0.5, 0.5));
        let _: nana_ui_runtime::QrCode =
            crate::components::QrCode::from_modules(vec![false], 1, 64.0)
                .expect("single module encodes");
        let _: nana_ui_runtime::Select = crate::components::Select::new(Some("code"));
        let _: nana_ui_runtime::Popover = crate::components::Popover::new();
        let _: nana_ui_runtime::ActionMenu = crate::components::ActionMenu::new();
        let _: nana_ui_runtime::ActionMenuItem = crate::components::ActionMenuItem::new("Rename");
        let _: nana_ui_runtime::AnchoredActionMenu =
            crate::components::AnchoredActionMenu::new(24.0, 36.0);
        let _: nana_ui_runtime::ContextMenu = crate::components::ContextMenu::new(24.0, 36.0);
        let _: nana_ui_runtime::OverlayHost = crate::components::OverlayHost::new();
        let _: nana_ui_runtime::Dropdown = crate::components::Dropdown::single(Some("code"));
        let _: nana_ui_runtime::SearchDropdown =
            crate::components::SearchDropdown::new(None::<&str>);
        let _: nana_ui_runtime::CommandPalette =
            crate::components::CommandPalette::new("命令面板", []);
        let _: nana_ui_runtime::TreeView = crate::components::TreeView::new([]);
    }

    #[test]
    fn sidebar_and_settings_leaf_exports_are_runtime_components() {
        let _: nana_ui_runtime::SidebarRow = crate::SidebarRow::new("工作区");
        let _: nana_ui_runtime::SettingsRow = crate::SettingsRow::new("主题");
        let _: nana_ui_runtime::SettingsCard = crate::SettingsCard::new("外观");
        let _: nana_ui_runtime::SidebarFrame = crate::SidebarFrame::new();
        let _: nana_ui_runtime::SidebarSection = crate::SidebarSection::new("资源");
        let _: nana_ui_runtime::SidebarFooter = crate::SidebarFooter::new();
        let _: nana_ui_runtime::AppearanceSection = crate::AppearanceSection::new(
            nana_ui_core::ThemeMode::Dark,
            nana_ui_core::AppearanceSettings::default(),
        );
        let _: nana_ui_runtime::AboutSection =
            crate::AboutSection::new(nana_ui_runtime::AboutMetadata::new("NanaUI", "0"));
        let _: nana_ui_runtime::SettingsCollapsibleCard = crate::SettingsCollapsibleCard::new(true);

        let _: nana_ui_runtime::SidebarRow = crate::components::SidebarRow::new("工作区");
        let _: nana_ui_runtime::SettingsRow = crate::components::SettingsRow::new("主题");
        let _: nana_ui_runtime::SettingsCard = crate::components::SettingsCard::new("外观");
        let _: nana_ui_runtime::SidebarFrame = crate::components::SidebarFrame::new();
        let _: nana_ui_runtime::SidebarSection = crate::components::SidebarSection::new("资源");
        let _: nana_ui_runtime::SidebarFooter = crate::components::SidebarFooter::new();
        let _: nana_ui_runtime::AppearanceSection = crate::components::AppearanceSection::new(
            nana_ui_core::ThemeMode::Dark,
            nana_ui_core::AppearanceSettings::default(),
        );
        let _: nana_ui_runtime::AboutSection = crate::components::AboutSection::new(
            nana_ui_runtime::AboutMetadata::new("NanaUI", "0"),
        );
        let _: nana_ui_runtime::SettingsCollapsibleCard =
            crate::components::SettingsCollapsibleCard::new(true);
    }

    #[test]
    fn workspace_family_public_exports_are_runtime_components() {
        let _: nana_ui_runtime::Workspace = crate::Workspace::new();
        let _: nana_ui_runtime::Dock =
            crate::Dock::new(nana_ui_runtime::DockNode::item("main", None));
        let _: nana_ui_runtime::DockPanel = crate::DockPanel::new();
        let _: fn(
            &nana_ui_core::SplitPaneModel,
            nana_ui_runtime::StableNodeId,
            nana_ui_runtime::StableNodeId,
        ) -> nana_ui_runtime::SplitPane = crate::SplitPane::from_model;
        let _: nana_ui_runtime::PaneChrome = crate::PaneChrome::new();
        let _: nana_ui_runtime::PaneTree =
            crate::PaneTree::new(nana_ui_runtime::PaneTreeNode::leaf("editor"));
        let _: nana_ui_runtime::AppShell = crate::AppShell::new();
        let _: nana_ui_runtime::AppTitleBar = crate::AppTitleBar::new("NanaUI");
    }

    #[test]
    fn migration_state_only_allows_monotonic_promotion() {
        use ComponentMigrationState::{Compatibility, RuntimeCandidate, RuntimeQualified};

        assert!(Compatibility.allows_transition_to(RuntimeCandidate));
        assert!(RuntimeCandidate.allows_transition_to(RuntimeQualified));
        assert!(RuntimeQualified.allows_transition_to(RuntimeQualified));
        assert!(!RuntimeQualified.allows_transition_to(RuntimeCandidate));
        assert!(!RuntimeCandidate.allows_transition_to(Compatibility));
    }

    #[test]
    fn feature_availability_reports_the_current_build() {
        let switch = component_support(component_ids::SWITCH).unwrap();
        assert_eq!(switch.required_feature, None);
        assert!(switch.compiled);

        let gpu = component_support(component_ids::GPU_VIEW).unwrap();
        assert_eq!(gpu.required_feature, Some("gpu"));
        assert_eq!(gpu.compiled, cfg!(feature = "gpu"));
    }
}
