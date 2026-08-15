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

/// Evidence state for replacing a component's Iced compatibility path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum ComponentMigrationState {
    /// The compatibility implementation remains the complete supported path.
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
    TEXTAREA => { id: "textarea", name: "Textarea", family: Control, migration: RuntimeCandidate, feature: Some("controls"), compiled: cfg!(feature = "controls"), capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility] },
    HOSTED_TEXTAREA => { id: "hosted-textarea", name: "HostedTextarea", family: Control, migration: Compatibility, feature: Some("controls"), compiled: cfg!(feature = "controls"), capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility] },
    RANGE_FIELD => { id: "range-field", name: "RangeField", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SELECT => { id: "select", name: "Select", family: Control, migration: RuntimeCandidate, feature: Some("controls"), compiled: cfg!(feature = "controls"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    SEGMENTED_CONTROL => { id: "segmented-control", name: "SegmentedControl", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    TABS => { id: "tabs", name: "Tabs", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    REORDER_LIST => { id: "reorder-list", name: "ReorderList", family: Navigation, migration: Compatibility, feature: Some("controls"), compiled: cfg!(feature = "controls"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    CALENDAR_HEATMAP => { id: "calendar-heatmap", name: "CalendarHeatmap", family: Data, migration: Compatibility, feature: Some("calendar"), compiled: cfg!(feature = "calendar"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    TIME_SERIES_CHART => { id: "time-series-chart", name: "TimeSeriesChart", family: Data, migration: Compatibility, feature: Some("charts"), compiled: cfg!(feature = "charts"), capabilities: [Render, Accessibility] },
    PROGRESS => { id: "progress", name: "Progress", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    SPINNER => { id: "spinner", name: "Spinner", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Animation, Accessibility] },
    SKELETON => { id: "skeleton", name: "Skeleton", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Animation] },
    LEVEL_METER => { id: "level-meter", name: "LevelMeter", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    STATUS_BADGE => { id: "status-badge", name: "StatusBadge", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    VALIDATION_MESSAGE => { id: "validation-message", name: "ValidationMessage", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    TOAST => { id: "toast", name: "Toast", family: Feedback, migration: RuntimeCandidate, feature: Some("feedback"), compiled: cfg!(feature = "feedback"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    COMMAND_PALETTE => { id: "command-palette", name: "CommandPalette", family: Overlay, migration: Compatibility, feature: Some("overlays"), compiled: cfg!(feature = "overlays"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    DIALOG => { id: "dialog", name: "Dialog", family: Overlay, migration: RuntimeCandidate, feature: Some("overlays"), compiled: cfg!(feature = "overlays"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    CONFIRM_DIALOG => { id: "confirm-dialog", name: "ConfirmDialog", family: Overlay, migration: RuntimeCandidate, feature: Some("overlays"), compiled: cfg!(feature = "overlays"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    DRAWER => { id: "drawer", name: "Drawer", family: Overlay, migration: RuntimeCandidate, feature: Some("overlays"), compiled: cfg!(feature = "overlays"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    TOOLTIP => { id: "tooltip", name: "Tooltip", family: Overlay, migration: RuntimeCandidate, feature: Some("overlays"), compiled: cfg!(feature = "overlays"), capabilities: [Render, Accessibility, Overlay] },
    POPOVER => { id: "popover", name: "Popover", family: Overlay, migration: RuntimeCandidate, feature: Some("popover"), compiled: cfg!(feature = "popover"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    ACTION_MENU => { id: "action-menu", name: "ActionMenu", family: Overlay, migration: RuntimeCandidate, feature: Some("popover"), compiled: cfg!(feature = "popover"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    CONTEXT_MENU => { id: "context-menu", name: "ContextMenu", family: Overlay, migration: RuntimeCandidate, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    ACTION_MENU_ITEM => { id: "action-menu-item", name: "ActionMenuItem", family: Overlay, migration: RuntimeCandidate, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    ANCHORED_ACTION_MENU => { id: "anchored-action-menu", name: "AnchoredActionMenu", family: Overlay, migration: RuntimeCandidate, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    OVERLAY_HOST => { id: "overlay-host", name: "OverlayHost", family: Overlay, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    DROPDOWN => { id: "dropdown", name: "Dropdown", family: Control, migration: Compatibility, feature: Some("selects"), compiled: cfg!(feature = "selects"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Overlay] },
    SEARCH_DROPDOWN => { id: "search-dropdown", name: "SearchDropdown", family: Control, migration: Compatibility, feature: Some("selects"), compiled: cfg!(feature = "selects"), capabilities: [Render, Pointer, Keyboard, Focus, Ime, Accessibility, Overlay] },
    TREE_VIEW => { id: "tree-view", name: "TreeView", family: Navigation, migration: Compatibility, feature: Some("surfaces"), compiled: cfg!(feature = "surfaces"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SIDEBAR_FRAME => { id: "sidebar-frame", name: "SidebarFrame", family: Navigation, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    SIDEBAR_SECTION => { id: "sidebar-section", name: "SidebarSection", family: Navigation, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Animation] },
    SIDEBAR_ROW => { id: "sidebar-row", name: "SidebarRow", family: Navigation, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SIDEBAR_FOOTER => { id: "sidebar-footer", name: "SidebarFooter", family: Navigation, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    KEY_CAPTURE_LAYER => { id: "key-capture-layer", name: "KeyCaptureLayer", family: Control, migration: Compatibility, feature: None, compiled: true, capabilities: [Keyboard, Focus, Accessibility] },
    KEYMAP_LAYER => { id: "keymap-layer", name: "KeymapLayer", family: Control, migration: Compatibility, feature: None, compiled: true, capabilities: [Keyboard, Focus] },
    NATIVE_MARKDOWN => { id: "native-markdown", name: "NativeMarkdown", family: Data, migration: Compatibility, feature: Some("rich-text"), compiled: cfg!(feature = "rich-text"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SELECTABLE_RICH_TEXT => { id: "selectable-rich-text", name: "SelectableRichText", family: Data, migration: Compatibility, feature: Some("rich-text"), compiled: cfg!(feature = "rich-text"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    QR_CODE => { id: "qr-code", name: "QrCodeCanvas", family: Data, migration: RuntimeCandidate, feature: Some("qr-code"), compiled: cfg!(feature = "qr-code"), capabilities: [Render, Accessibility] },
    GRAPH_CANVAS => { id: "graph-canvas", name: "GraphCanvas", family: Data, migration: Compatibility, feature: Some("graph-canvas"), compiled: cfg!(feature = "graph-canvas"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Gpu] },
    IMAGE_VIEWER => { id: "image-viewer", name: "ImageViewer", family: Media, migration: Compatibility, feature: Some("image-viewer"), compiled: cfg!(feature = "image-viewer"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    XY_PAD => { id: "xy-pad", name: "XYPad", family: Control, migration: RuntimeCandidate, feature: Some("xy-pad"), compiled: cfg!(feature = "xy-pad"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    FORM_FIELD => { id: "form-field", name: "FormField", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    LABELED_VALUE => { id: "labeled-value", name: "LabeledValue", family: Data, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    EMPTY_STATE => { id: "empty-state", name: "EmptyState", family: Feedback, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Accessibility] },
    INTERACTIVE_CARD => { id: "interactive-card", name: "InteractiveCard", family: Control, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    LIST_ITEM => { id: "list-item", name: "ListItem", family: Navigation, migration: RuntimeQualified, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    DOCK_PANEL => { id: "dock-panel", name: "DockPanel", family: Workspace, migration: Compatibility, feature: Some("surfaces"), compiled: cfg!(feature = "surfaces"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    WORKSPACE => { id: "workspace", name: "Workspace", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Animation, Persistence] },
    DOCK => { id: "dock", name: "Dock", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Animation, Persistence] },
    SPLIT_PANE => { id: "split-pane", name: "SplitPane", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    PANE_CHROME => { id: "pane-chrome", name: "PaneChrome", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    PANE_TREE => { id: "pane-tree", name: "PaneTree", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    APP_SHELL => { id: "app-shell", name: "AppShell", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    APP_TITLE_BAR => { id: "app-title-bar", name: "AppTitleBar", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility] },
    SETTINGS => { id: "settings", name: "Settings", family: Workspace, migration: Compatibility, feature: None, compiled: true, capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    APPEARANCE_SECTION => { id: "appearance-section", name: "AppearanceSection", family: Workspace, migration: Compatibility, feature: Some("settings-components"), compiled: cfg!(feature = "settings-components"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    ABOUT_SECTION => { id: "about-section", name: "AboutSection", family: Workspace, migration: Compatibility, feature: Some("settings-components"), compiled: cfg!(feature = "settings-components"), capabilities: [Render, Accessibility] },
    SETTINGS_COLLAPSIBLE_CARD => { id: "settings-collapsible-card", name: "SettingsCollapsibleCard", family: Workspace, migration: Compatibility, feature: Some("settings-components"), compiled: cfg!(feature = "settings-components"), capabilities: [Render, Pointer, Keyboard, Focus, Accessibility, Persistence] },
    GPU_VIEW => { id: "gpu-view", name: "GpuView", family: Gpu, migration: Compatibility, feature: Some("gpu"), compiled: cfg!(feature = "gpu"), capabilities: [Render, Pointer, Gpu] },
    GPU_TEXTURE_VIEW => { id: "gpu-texture-view", name: "GpuTextureView", family: Gpu, migration: Compatibility, feature: Some("gpu"), compiled: cfg!(feature = "gpu"), capabilities: [Render, Gpu] },
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
/// public catalog. Candidate components deliberately remain on compatibility.
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
        ] {
            let support = component_support(id).expect("qualified component is cataloged");
            assert_eq!(support.migration, ComponentMigrationState::RuntimeQualified);
            assert_eq!(component_uses_runtime(id), support.compiled);
        }
    }

    #[test]
    fn candidate_components_keep_the_compatibility_default_route() {
        for id in [
            component_ids::TEXTAREA,
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
        ] {
            let support = component_support(id).expect("candidate component is cataloged");
            assert_eq!(support.migration, ComponentMigrationState::RuntimeCandidate);
            assert!(!component_uses_runtime(id));
        }

        let hosted = component_support(component_ids::HOSTED_TEXTAREA)
            .expect("hosted textarea is cataloged");
        assert_eq!(hosted.migration, ComponentMigrationState::Compatibility);
        assert!(!component_uses_runtime(component_ids::HOSTED_TEXTAREA));
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
        let _: nana_ui_runtime::Skeleton =
            crate::Skeleton::new(nana_ui_core::LengthSpec::Fill, 16.0);
        let _: nana_ui_runtime::LevelMeter = crate::LevelMeter::new(0.5);
        let _: nana_ui_runtime::FormField = crate::FormField::new("Name");
        let _: nana_ui_runtime::InteractiveCard = crate::InteractiveCard::new();

        let _: nana_ui_runtime::Progress = crate::components::Progress::new(1.0, 2.0);
        let _: nana_ui_runtime::Spinner = crate::components::Spinner::new("Loading");
        let _: nana_ui_runtime::Skeleton =
            crate::components::Skeleton::new(nana_ui_core::LengthSpec::Fill, 16.0);
        let _: nana_ui_runtime::LevelMeter = crate::components::LevelMeter::new(0.5);
        let _: nana_ui_runtime::FormField = crate::components::FormField::new("Name");
        let _: nana_ui_runtime::InteractiveCard = crate::components::InteractiveCard::new();
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
