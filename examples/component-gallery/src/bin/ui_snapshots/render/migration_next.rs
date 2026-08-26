use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nana_ui::runtime::{
    AboutMetadata as RuntimeAboutMetadata, AboutSection as RuntimeAboutSection,
    AccessibilityAction, AccessibilityActionRequest, ActionMenu as RuntimeActionMenu,
    ActionMenuItem as RuntimeActionMenuItem, Activate,
    AnchoredActionMenu as RuntimeAnchoredActionMenu, AppShell as RuntimeAppShell,
    AppTitleBar as RuntimeAppTitleBar, AppearanceSection as RuntimeAppearanceSection,
    Button as RuntimeButton, CalendarHeatmap as RuntimeCalendarHeatmap,
    CalendarHeatmapDatum as RuntimeCalendarDatum, Card as RuntimeCard, Checkbox as RuntimeCheckbox,
    CommandPalette as RuntimeCommandPalette, ConfirmDialog as RuntimeConfirmDialog, ConfirmSlots,
    ContextMenu as RuntimeContextMenu, ContextMenuItem as RuntimeContextMenuItem,
    DesktopShell as RuntimeDesktopShell, Dialog as RuntimeDialog, Dock as RuntimeDock,
    DockNode as RuntimeDockNode, DockPanel as RuntimeDockPanel, DocumentId,
    Drawer as RuntimeDrawer, Dropdown as RuntimeDropdown, DropdownOption as RuntimeDropdownOption,
    EmptyState as RuntimeEmptyState, Entity, FormField as RuntimeFormField,
    GpuTextureView as RuntimeGpuTextureView, GpuView as RuntimeGpuView,
    GpuViewPalette as RuntimeGpuViewPalette, GraphCanvas as RuntimeGraphCanvas,
    HostedTextarea as RuntimeHostedTextarea, IconButton as RuntimeIconButton,
    ImageViewer as RuntimeImageViewer, ImageViewerContent,
    InteractiveCard as RuntimeInteractiveCard, KeyCaptureLayer as RuntimeKeyCaptureLayer,
    KeymapLayer as RuntimeKeymapLayer, LabeledValue as RuntimeLabeledValue, LayoutViewport,
    LevelMeter as RuntimeLevelMeter, List as RuntimeList, ListItem as RuntimeListItem,
    ListItemSlots, MarkdownBlock, MarkdownBlockKind, MarkdownSpan, ModalSlots, MountState,
    MutationQueue, NativeMarkdown as RuntimeNativeMarkdown, NodeStyle,
    OverlayHost as RuntimeOverlayHost, PaneChrome as RuntimePaneChrome,
    PaneTree as RuntimePaneTree, PaneTreeNode as RuntimePaneTreeNode, Popover as RuntimePopover,
    Progress as RuntimeProgress, QrCode as RuntimeQrCode, RangeField as RuntimeRangeField,
    ReorderItem as RuntimeReorderItem, ReorderList as RuntimeReorderList, RichSpan,
    RuntimeDocument, SearchDropdown as RuntimeSearchDropdown,
    SearchDropdownOption as RuntimeSearchDropdownOption,
    SegmentedControl as RuntimeSegmentedControl, SegmentedOption as RuntimeSegmentedOption,
    SegmentedSelectionRequested, Select as RuntimeSelect, SelectOption as RuntimeSelectOption,
    SelectableRichText as RuntimeSelectableRichText, SettingsCard as RuntimeSettingsCard,
    SettingsCollapsibleCard as RuntimeSettingsCollapsibleCard, SettingsPage as RuntimeSettingsPage,
    SettingsSidebar as RuntimeSettingsSidebar, SidebarFooter as RuntimeSidebarFooter,
    SidebarFooterButton as RuntimeSidebarFooterButton, SidebarFrame as RuntimeSidebarFrame,
    SidebarRow as RuntimeSidebarRow, SidebarSection as RuntimeSidebarSection,
    Skeleton as RuntimeSkeleton, Spinner as RuntimeSpinner, SplitPane as RuntimeSplitPane,
    StableNodeId, StatusBadge as RuntimeStatusBadge, Switch as RuntimeSwitch,
    TabOption as RuntimeTabOption, Tabs as RuntimeTabs, Text as RuntimeText,
    TextArea as RuntimeTextArea, TextHorizontalAlignment, TextInput as RuntimeTextInput,
    TextSelection, TextVerticalAlignment, Thumbnail as RuntimeThumbnail,
    TimeSeriesChart as RuntimeTimeSeriesChart, Toast as RuntimeToast, TreeView as RuntimeTreeView,
    ValidationMessage as RuntimeValidationMessage, ValueEmphasis, Workspace as RuntimeWorkspace,
    WorkspaceRegionSlot, XYPad as RuntimeXYPad,
};
use nana_ui::{
    ActionId, AppearanceSettings, CardKind, CommandPaletteItem, ComponentId,
    ComponentMigrationState, ControlSize, GraphEdge, GraphEndpoint, GraphModel, GraphNode,
    GraphPoint, GraphPort, GraphPortKind, GraphPortSide, GraphSize, Icon, NanaTextShaper, RegionId,
    RegionRole, RegionState, RuntimeInputAdapter, SettingsModel, SettingsState, SettingsTab,
    SettingsTabId, SplitAxis, ThemeMode, ThemeModeExt, TooltipConfig, TooltipPlacement, TreeNode,
    WindowMaterialMode, WorkspaceLayout, XYPadValue, component_catalog, component_ids,
};
use nana_ui_core::{
    DialogSize, DrawerSide, LengthSpec, SemanticColorRole, SplitPaneModel, StatusTone,
    SwitchControlPosition, ToastTone, ValidationIntent, WorkspaceModel,
};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};
use nana_ui_scene::ScenePrimitiveKind;

use crate::write::{self, Size};

use super::gpu::{self, SnapshotGpu};
use super::{pixel_difference, side_by_side};

const SIZE: Size<u32> = Size::new(420, 120);
const GAP: u32 = 8;
const SLOT_INSET: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    Text,
    Button,
    TextInput,
    Textarea,
    HostedTextarea,
    Checkbox,
    IconButton,
    Switch,
    Card,
    ListItem,
    RangeField,
    SegmentedControl,
    Tabs,
    StatusBadge,
    ValidationMessage,
    EmptyState,
    LabeledValue,
    Progress,
    Spinner,
    Skeleton,
    LevelMeter,
    FormField,
    InteractiveCard,
    Tooltip,
    Dialog,
    ConfirmDialog,
    Drawer,
    Toast,
    XYPad,
    QrCode,
    Select,
    Popover,
    ActionMenu,
    ActionMenuItem,
    AnchoredActionMenu,
    ContextMenu,
    SidebarFrame,
    SidebarSection,
    SidebarFooter,
    AppearanceSection,
    AboutSection,
    SettingsCollapsibleCard,
    CommandPalette,
    OverlayHost,
    Dropdown,
    SearchDropdown,
    TreeView,
    SidebarRow,
    Settings,
    SettingsSidebar,
    SettingsPage,
    Workspace,
    Dock,
    DockPanel,
    SplitPane,
    PaneChrome,
    PaneTree,
    AppShell,
    DesktopShell,
    AppTitleBar,
    CalendarHeatmap,
    TimeSeriesChart,
    ReorderList,
    NativeMarkdown,
    SelectableRichText,
    ImageViewer,
    GraphCanvas,
    KeyCaptureLayer,
    KeymapLayer,
    GpuTextureView,
    GpuView,
    Thumbnail,
}

impl Component {
    const fn id(self) -> ComponentId {
        match self {
            Self::Text => component_ids::TEXT,
            Self::Button => component_ids::BUTTON,
            Self::TextInput => component_ids::TEXT_INPUT,
            Self::Textarea => component_ids::TEXTAREA,
            Self::HostedTextarea => component_ids::HOSTED_TEXTAREA,
            Self::Checkbox => component_ids::CHECKBOX,
            Self::IconButton => component_ids::ICON_BUTTON,
            Self::Switch => component_ids::SWITCH,
            Self::Card => component_ids::CARD,
            Self::ListItem => component_ids::LIST_ITEM,
            Self::RangeField => component_ids::RANGE_FIELD,
            Self::SegmentedControl => component_ids::SEGMENTED_CONTROL,
            Self::Tabs => component_ids::TABS,
            Self::StatusBadge => component_ids::STATUS_BADGE,
            Self::ValidationMessage => component_ids::VALIDATION_MESSAGE,
            Self::EmptyState => component_ids::EMPTY_STATE,
            Self::LabeledValue => component_ids::LABELED_VALUE,
            Self::Progress => component_ids::PROGRESS,
            Self::Spinner => component_ids::SPINNER,
            Self::Skeleton => component_ids::SKELETON,
            Self::LevelMeter => component_ids::LEVEL_METER,
            Self::FormField => component_ids::FORM_FIELD,
            Self::InteractiveCard => component_ids::INTERACTIVE_CARD,
            Self::Tooltip => component_ids::TOOLTIP,
            Self::Dialog => component_ids::DIALOG,
            Self::ConfirmDialog => component_ids::CONFIRM_DIALOG,
            Self::Drawer => component_ids::DRAWER,
            Self::Toast => component_ids::TOAST,
            Self::XYPad => component_ids::XY_PAD,
            Self::QrCode => component_ids::QR_CODE,
            Self::Select => component_ids::SELECT,
            Self::Popover => component_ids::POPOVER,
            Self::ActionMenu => component_ids::ACTION_MENU,
            Self::ActionMenuItem => component_ids::ACTION_MENU_ITEM,
            Self::AnchoredActionMenu => component_ids::ANCHORED_ACTION_MENU,
            Self::ContextMenu => component_ids::CONTEXT_MENU,
            Self::SidebarFrame => component_ids::SIDEBAR_FRAME,
            Self::SidebarSection => component_ids::SIDEBAR_SECTION,
            Self::SidebarFooter => component_ids::SIDEBAR_FOOTER,
            Self::AppearanceSection => component_ids::APPEARANCE_SECTION,
            Self::AboutSection => component_ids::ABOUT_SECTION,
            Self::SettingsCollapsibleCard => component_ids::SETTINGS_COLLAPSIBLE_CARD,
            Self::CommandPalette => component_ids::COMMAND_PALETTE,
            Self::OverlayHost => component_ids::OVERLAY_HOST,
            Self::Dropdown => component_ids::DROPDOWN,
            Self::SearchDropdown => component_ids::SEARCH_DROPDOWN,
            Self::TreeView => component_ids::TREE_VIEW,
            Self::SidebarRow => component_ids::SIDEBAR_ROW,
            Self::Settings => component_ids::SETTINGS,
            Self::SettingsSidebar => component_ids::SETTINGS,
            Self::SettingsPage => component_ids::SETTINGS,
            Self::Workspace => component_ids::WORKSPACE,
            Self::Dock => component_ids::DOCK,
            Self::DockPanel => component_ids::DOCK_PANEL,
            Self::SplitPane => component_ids::SPLIT_PANE,
            Self::PaneChrome => component_ids::PANE_CHROME,
            Self::PaneTree => component_ids::PANE_TREE,
            Self::AppShell => component_ids::APP_SHELL,
            Self::DesktopShell => component_ids::APP_SHELL,
            Self::AppTitleBar => component_ids::APP_TITLE_BAR,
            Self::CalendarHeatmap => component_ids::CALENDAR_HEATMAP,
            Self::TimeSeriesChart => component_ids::TIME_SERIES_CHART,
            Self::ReorderList => component_ids::REORDER_LIST,
            Self::NativeMarkdown => component_ids::NATIVE_MARKDOWN,
            Self::SelectableRichText => component_ids::SELECTABLE_RICH_TEXT,
            Self::ImageViewer => component_ids::IMAGE_VIEWER,
            Self::GraphCanvas => component_ids::GRAPH_CANVAS,
            Self::KeyCaptureLayer => component_ids::KEY_CAPTURE_LAYER,
            Self::KeymapLayer => component_ids::KEYMAP_LAYER,
            Self::GpuTextureView => component_ids::GPU_TEXTURE_VIEW,
            Self::GpuView => component_ids::GPU_VIEW,
            Self::Thumbnail => component_ids::THUMBNAIL,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Fixture {
    id: ComponentId,
    component: Component,
    state: &'static str,
    expected: &'static str,
    reference_contract: &'static str,
    runtime_contract: &'static str,
    divergence: &'static str,
}

// This registry describes snapshot evidence only. Migration state remains owned by
// `component_catalog`; do not infer qualification from presence in this list.
const FIXTURE_REGISTRY: &[Fixture] = &[
    f(
        Component::Text,
        "normal",
        "body text uses the shared 13px baseline",
    ),
    f(
        Component::Text,
        "wrap",
        "long text wraps inside its authored content box",
    ),
    f(
        Component::Text,
        "ellipsis",
        "single-line overflow clips with an ellipsis",
    ),
    f(
        Component::Text,
        "centered",
        "horizontal and vertical anchors use the content box",
    ),
    f(
        Component::Text,
        "muted",
        "muted text keeps readable semantic contrast",
    ),
    f(
        Component::Button,
        "ghost",
        "ghost kind is transparent until interaction",
    ),
    f(
        Component::Button,
        "subtle",
        "subtle kind has neutral surface and border",
    ),
    f(
        Component::Button,
        "selected",
        "selected kind keeps persistent selected semantics",
    ),
    f(
        Component::Button,
        "primary",
        "primary kind uses accent-soft semantics",
    ),
    f(
        Component::Button,
        "warning",
        "warning kind uses warning semantics",
    ),
    f(
        Component::Button,
        "danger",
        "danger kind uses danger semantics",
    ),
    f(
        Component::Button,
        "text-kind",
        "text kind uses accent text without a surface",
    ),
    f(
        Component::Button,
        "small",
        "small size follows compact control metrics",
    ),
    f(
        Component::Button,
        "medium",
        "medium size follows standard control metrics",
    ),
    f(
        Component::Button,
        "large",
        "large size follows large control metrics",
    ),
    f(
        Component::Button,
        "hover",
        "hover feedback covers the complete hit target",
    ),
    f(
        Component::Button,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::Button,
        "focused",
        "focused keeps idle chrome; hover and pressed use background",
    ),
    f(
        Component::Button,
        "disabled",
        "disabled button cannot activate or focus",
    ),
    f(
        Component::Button,
        "loading",
        "loading is visible and prevents duplicate activation",
    ),
    f(
        Component::Button,
        "pointer-activation",
        "pointer activation emits once",
    ),
    f(
        Component::Button,
        "keyboard-activation",
        "Space activation emits once",
    ),
    f(
        Component::TextInput,
        "value",
        "committed value uses field padding and baseline",
    ),
    f(
        Component::TextInput,
        "placeholder",
        "empty input paints faint placeholder text",
    ),
    f(
        Component::TextInput,
        "hover",
        "hover strengthens the neutral border",
    ),
    f(
        Component::TextInput,
        "focused",
        "focus paints border and caret",
    ),
    f(
        Component::TextInput,
        "selection",
        "selected text has shaped highlight geometry",
    ),
    f(
        Component::TextInput,
        "disabled",
        "disabled input is inert and visibly disabled",
    ),
    f(
        Component::TextInput,
        "invalid",
        "invalid input keeps a danger border while focused",
    ),
    f(
        Component::TextInput,
        "secure",
        "secure input masks committed text",
    ),
    f(
        Component::TextInput,
        "small",
        "small size follows compact field metrics",
    ),
    f(
        Component::TextInput,
        "large",
        "large size follows large field metrics",
    ),
    f(
        Component::TextInput,
        "read-only",
        "read-only input remains focusable but rejects edits",
    ),
    f(
        Component::TextInput,
        "loading",
        "loading input is busy and rejects input",
    ),
    f(
        Component::TextInput,
        "keyboard-edit",
        "keyboard input commits through typed state",
    ),
    f(
        Component::TextInput,
        "ime-preedit",
        "IME preedit is visibly distinct from committed text",
    ),
    f(
        Component::TextInput,
        "ime-commit",
        "IME commit updates value and clears preedit",
    ),
    f(
        Component::TextInput,
        "accessibility-set-value",
        "accessibility SetValue updates the field",
    ),
    f(
        Component::Textarea,
        "placeholder",
        "empty multiline input paints faint placeholder text",
    ),
    f(
        Component::Textarea,
        "multiline",
        "committed lines retain top alignment and line spacing",
    ),
    f(
        Component::Textarea,
        "focused",
        "focus paints the multiline border and caret",
    ),
    f(
        Component::Textarea,
        "selection",
        "single-line selection uses shaped highlight geometry",
    ),
    f(
        Component::Textarea,
        "multiline-selection",
        "selection spanning line breaks produces one highlight per line",
    ),
    f(
        Component::Textarea,
        "invalid-focused",
        "focused invalid textarea uses a 2px danger field border without a second ring",
    ),
    f(
        Component::Textarea,
        "disabled",
        "disabled textarea is visibly inert and cannot focus",
    ),
    f(
        Component::Textarea,
        "clipped",
        "overflowing lines are clipped to the authored content box",
    ),
    f(
        Component::Textarea,
        "scroll",
        "the scrolled caret and text remain inside the content box",
    ),
    f(
        Component::HostedTextarea,
        "rust",
        "committed rust text is colored by the Runtime highlight presenter",
    ),
    f(
        Component::HostedTextarea,
        "placeholder",
        "empty highlighted editor paints faint placeholder text",
    ),
    f(
        Component::HostedTextarea,
        "disabled",
        "disabled highlighted editor is visibly inert",
    ),
    f(
        Component::CalendarHeatmap,
        "weeks",
        "week columns and level fills use theme accent, not a second canvas",
    ),
    f(
        Component::TimeSeriesChart,
        "series",
        "grid, area and line stay inside the 148px chart box",
    ),
    f(
        Component::ReorderList,
        "rows",
        "selected row uses the selected surface; labels stay left aligned",
    ),
    f(
        Component::NativeMarkdown,
        "blocks",
        "heading and paragraph project as wrapped body text",
    ),
    f(
        Component::SelectableRichText,
        "plain",
        "concatenated spans paint as selectable body text",
    ),
    f(
        Component::ImageViewer,
        "open",
        "scrim, surface and close chrome are present without embedded pixels",
    ),
    f(
        Component::GraphCanvas,
        "nodes",
        "two connected nodes paint as Scene quads inside the canvas",
    ),
    f(
        Component::KeyCaptureLayer,
        "recording",
        "recording badge is visible while capture is armed",
    ),
    f(
        Component::KeymapLayer,
        "idle",
        "keymap chrome is a non-focusable 28px badge",
    ),
    f(
        Component::GpuTextureView,
        "slot",
        "host texture slot snapshot-gpu is sampled in the layout region",
    ),
    f(
        Component::GpuView,
        "inline",
        "gpu-view custom renderer paints the inline slot",
    ),
    f(
        Component::Thumbnail,
        "empty",
        "empty thumbnail keeps the 1:1 control box without a host-texture node",
    ),
    f(
        Component::Thumbnail,
        "ready",
        "ready thumbnail samples nana.host-texture with contain",
    ),
    f(
        Component::Thumbnail,
        "wide",
        "host-declared 16:9 aspect widens the shared box",
    ),
    // Platform IME events cannot be injected into the compatibility widget by
    // this headless harness. Preedit remains a real Hosted acceptance gate.
    f(
        Component::Checkbox,
        "off",
        "off state exposes false in paint and accessibility",
    ),
    f(
        Component::Checkbox,
        "on",
        "on state exposes true in paint and accessibility",
    ),
    f(
        Component::Checkbox,
        "hover",
        "hover feedback reaches the indicator",
    ),
    f(
        Component::Checkbox,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::Checkbox,
        "focused",
        "keyboard focus is visible around the indicator",
    ),
    f(
        Component::Checkbox,
        "disabled",
        "disabled checkbox cannot toggle or focus",
    ),
    f(
        Component::Checkbox,
        "invalid",
        "invalid state keeps semantic danger treatment",
    ),
    f(
        Component::Checkbox,
        "pointer-toggle",
        "pointer activation toggles once",
    ),
    f(
        Component::Checkbox,
        "space-toggle",
        "Space activation toggles once",
    ),
    f(
        Component::Checkbox,
        "accessibility-toggle",
        "accessibility click toggles once",
    ),
    f(
        Component::IconButton,
        "normal",
        "labelled icon action has a complete square hit target",
    ),
    f(
        Component::IconButton,
        "hover",
        "hover feedback preserves icon contrast",
    ),
    f(
        Component::IconButton,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::IconButton,
        "focused",
        "focused keeps idle chrome; hover and pressed use background",
    ),
    f(
        Component::IconButton,
        "selected",
        "selected state is persistent and distinguishable",
    ),
    f(
        Component::IconButton,
        "disabled",
        "disabled action cannot receive pointer or focus",
    ),
    f(
        Component::IconButton,
        "keyboard-activation",
        "Space or Enter invokes the typed default action once",
    ),
    f(
        Component::IconButton,
        "tooltip-delay",
        "hover delay opens a real labelled tooltip",
    ),
    f(
        Component::IconButton,
        "tooltip-edge",
        "tooltip remains inside the viewport near an edge",
    ),
    f(
        Component::Switch,
        "off",
        "off state exposes false through paint and accessibility",
    ),
    f(
        Component::Switch,
        "on",
        "on state exposes true through paint and accessibility",
    ),
    f(
        Component::Switch,
        "hover",
        "hover feedback covers the complete row",
    ),
    f(
        Component::Switch,
        "pressed",
        "pressed feedback is distinguishable",
    ),
    f(Component::Switch, "focused", "keyboard focus is visible"),
    f(
        Component::Switch,
        "disabled",
        "disabled switch cannot toggle",
    ),
    f(
        Component::Switch,
        "invalid",
        "invalid state uses semantic danger treatment",
    ),
    f(
        Component::Switch,
        "label-hint",
        "label and hint retain hierarchy without clipping",
    ),
    f(
        Component::Switch,
        "control-start",
        "control may be placed before label content",
    ),
    f(
        Component::Switch,
        "control-end",
        "control defaults after label content",
    ),
    f(
        Component::Switch,
        "pointer-toggle",
        "pointer activation toggles once",
    ),
    f(
        Component::Switch,
        "space-toggle",
        "Space activation toggles once",
    ),
    f(
        Component::Switch,
        "accessibility-toggle",
        "accessibility click toggles once",
    ),
    f(
        Component::Card,
        "surface",
        "surface card contains title and arbitrary body",
    ),
    f(
        Component::Card,
        "outlined",
        "outlined kind has a semantic border",
    ),
    f(
        Component::Card,
        "raised",
        "raised kind remains legible above its background",
    ),
    f(
        Component::Card,
        "flat",
        "flat kind removes unnecessary chrome",
    ),
    f(
        Component::Card,
        "selected",
        "selected kind is visibly selected",
    ),
    f(
        Component::Card,
        "padding",
        "custom padding changes the content box, not semantics",
    ),
    f(
        Component::Card,
        "fixed-height",
        "fixed height constrains the outer card",
    ),
    f(
        Component::Card,
        "loading",
        "loading is busy and schedules only active animation",
    ),
    f(
        Component::Card,
        "long-content",
        "long content is clipped by the card content box",
    ),
    f(
        Component::ListItem,
        "three-slots",
        "leading content and trailing slots retain order and gap",
    ),
    f(
        Component::ListItem,
        "normal",
        "unselected item has a complete row hit target",
    ),
    f(
        Component::ListItem,
        "hover",
        "hover feedback covers the complete row",
    ),
    f(
        Component::ListItem,
        "pressed",
        "pressed feedback is distinguishable",
    ),
    f(Component::ListItem, "focused", "keyboard focus is visible"),
    f(
        Component::ListItem,
        "selected",
        "selected state is persistent",
    ),
    f(
        Component::ListItem,
        "selected-hover",
        "selected hover remains selected while adding hover feedback",
    ),
    f(
        Component::ListItem,
        "selected-pressed",
        "selected pressed remains selected while adding pressed feedback",
    ),
    f(
        Component::ListItem,
        "disabled",
        "disabled item cannot activate",
    ),
    f(
        Component::ListItem,
        "small",
        "small density remains readable",
    ),
    f(
        Component::ListItem,
        "medium",
        "medium density follows control metrics",
    ),
    f(
        Component::ListItem,
        "large",
        "large density follows control metrics",
    ),
    f(
        Component::ListItem,
        "auto-height",
        "multi-line content determines height without clipping",
    ),
    f(
        Component::ListItem,
        "pointer-activation",
        "pointer activation emits once",
    ),
    f(
        Component::ListItem,
        "keyboard-activation",
        "keyboard activation emits once",
    ),
    f(
        Component::RangeField,
        "minimum",
        "minimum maps to the start of the complete track",
    ),
    f(
        Component::RangeField,
        "middle",
        "middle value maps proportionally",
    ),
    f(
        Component::RangeField,
        "maximum",
        "maximum maps to the end of the complete track",
    ),
    f(
        Component::RangeField,
        "decimal-step",
        "decimal values are quantized to step",
    ),
    f(
        Component::RangeField,
        "drag",
        "drag updates through pointer capture",
    ),
    f(
        Component::RangeField,
        "drag-cancel",
        "cancel restores the drag origin and releases capture",
    ),
    f(
        Component::RangeField,
        "disabled",
        "disabled range cannot change or focus",
    ),
    f(
        Component::RangeField,
        "invalid",
        "invalid state uses semantic danger treatment",
    ),
    f(
        Component::RangeField,
        "arrow-decrement",
        "Arrow decreases by one step",
    ),
    f(
        Component::RangeField,
        "arrow-increment",
        "Arrow increases by one step",
    ),
    f(
        Component::RangeField,
        "page-decrement",
        "PageDown decreases by page step",
    ),
    f(
        Component::RangeField,
        "page-increment",
        "PageUp increases by page step",
    ),
    f(Component::RangeField, "home", "Home moves to minimum"),
    f(Component::RangeField, "end", "End moves to maximum"),
    f(
        Component::RangeField,
        "accessibility-set-value",
        "SetValue quantizes and updates once",
    ),
    f(
        Component::SegmentedControl,
        "small",
        "small density preserves the concentric segmented inset",
    ),
    f(
        Component::SegmentedControl,
        "medium-icon",
        "medium options align an authored semantic icon with the label",
    ),
    f(
        Component::SegmentedControl,
        "large",
        "large density preserves control and option height contracts",
    ),
    f(
        Component::SegmentedControl,
        "hover",
        "hover is visible without changing controlled selection",
    ),
    f(
        Component::SegmentedControl,
        "pressed",
        "pressed feedback is distinct from neutral hover",
    ),
    f(
        Component::SegmentedControl,
        "selected-hover",
        "selected hover retains checked semantics with its own surface",
    ),
    f(
        Component::SegmentedControl,
        "selected-pressed",
        "selected press retains checked semantics with its own surface",
    ),
    f(
        Component::SegmentedControl,
        "focused",
        "focus uses a two pixel outline with a two pixel external offset",
    ),
    f(
        Component::SegmentedControl,
        "disabled-selected",
        "a disabled selected option remains checked but is not a tab stop",
    ),
    f(
        Component::SegmentedControl,
        "empty",
        "an empty group remains inert and exposes no synthetic option",
    ),
    f(
        Component::SegmentedControl,
        "all-disabled",
        "an all-disabled group has no focus target or interactive option",
    ),
    f(
        Component::SegmentedControl,
        "pointer-request",
        "pointer activation requests selection once without committing it",
    ),
    f(
        Component::SegmentedControl,
        "pointer-cancel",
        "pointer cancel consumes its lease without requesting selection",
    ),
    f(
        Component::SegmentedControl,
        "selected-repeat-request",
        "activating the selected option still emits one controlled request",
    ),
    f(
        Component::SegmentedControl,
        "arrow-skip-wrap",
        "horizontal arrows skip disabled options and wrap",
    ),
    f(
        Component::SegmentedControl,
        "home-end",
        "Home and End request the first and last enabled options",
    ),
    f(
        Component::SegmentedControl,
        "space-enter-repeat",
        "Space and Enter reject repeats while normal activation requests selection",
    ),
    f(
        Component::SegmentedControl,
        "no-selection",
        "without selection the first enabled option is the sole sequential tab stop",
    ),
    f(
        Component::SegmentedControl,
        "dynamic-disable",
        "disabling the focused selected option repairs focus but preserves checked state",
    ),
    f(
        Component::SegmentedControl,
        "controlled-commit",
        "a request leaves selection unchanged until the application setter commits once",
    ),
    f(
        Component::SegmentedControl,
        "a11y-radio",
        "SegmentedControl exposes RadioGroup and Radio roles for one controlled checked option",
    ),
    f(
        Component::SegmentedControl,
        "atomic-reconcile",
        "invalid reconciliation is atomic and removed options park without ghosts",
    ),
    f(
        Component::StatusBadge,
        "neutral",
        "neutral status remains descriptive and visually quiet",
    ),
    f(
        Component::StatusBadge,
        "info",
        "informational status uses the semantic accent tone",
    ),
    f(
        Component::StatusBadge,
        "success",
        "successful status uses the semantic success tone",
    ),
    f(
        Component::StatusBadge,
        "warning",
        "warning status uses the semantic warning tone",
    ),
    f(
        Component::StatusBadge,
        "danger",
        "danger status uses the semantic danger tone",
    ),
    f(
        Component::ValidationMessage,
        "warning",
        "warning validation retains its outlined marker and regular text weight",
    ),
    f(
        Component::ValidationMessage,
        "danger",
        "danger validation retains its outlined marker and regular text weight",
    ),
    f(
        Component::EmptyState,
        "complete-action",
        "normal empty state owns icon, title and message while a real child owns action",
    ),
    f(
        Component::EmptyState,
        "compact",
        "compact empty state is start aligned with tighter intrinsic spacing",
    ),
    f(
        Component::EmptyState,
        "title-only",
        "title-only empty state has no synthetic icon, message or action",
    ),
    f(
        Component::EmptyState,
        "narrow-cjk",
        "long CJK and emoji content wraps and remains ordered at narrow width",
    ),
    f(
        Component::EmptyState,
        "extreme-clip",
        "extreme width clips intrinsic content and descendants to the authored box",
    ),
    f(
        Component::LabeledValue,
        "normal",
        "normal value uses medium emphasis without making the parent interactive",
    ),
    f(
        Component::LabeledValue,
        "strong",
        "strong value uses semibold emphasis without changing its semantic value",
    ),
    f(
        Component::LabeledValue,
        "action",
        "a real application-owned child action remains separate from the inert summary",
    ),
    f(
        Component::Progress,
        "labeled",
        "determinate progress keeps a Subtle track, Accent fill and optional label",
    ),
    f(
        Component::Progress,
        "empty",
        "zero progress leaves the Accent fill collapsed on the track",
    ),
    f(
        Component::Spinner,
        "loading",
        "spinner reuses the host-sampled Scene primitive beside a muted label",
    ),
    f(
        Component::Tabs,
        "selected",
        "tabs share segmented selection without the bordered pill chrome",
    ),
    f(
        Component::Tabs,
        "focused",
        "tab focus uses the same 2px external ring as segmented options",
    ),
    f(
        Component::Skeleton,
        "block",
        "skeleton is a Subtle rounded placeholder with authored width and height",
    ),
    f(
        Component::LevelMeter,
        "success",
        "level meter fills a compact tone-colored track without a progress label",
    ),
    f(
        Component::FormField,
        "error",
        "form field shows the label and danger support while the control stays a child",
    ),
    f(
        Component::InteractiveCard,
        "selected",
        "interactive card uses selected surface, border and activation semantics",
    ),
    f(
        Component::Tooltip,
        "open",
        "zero-delay hover opens a compact label-only tooltip bound to the pointer",
    ),
    f(
        Component::Tooltip,
        "delay",
        "hover before the delay leaves the tooltip closed",
    ),
    f(
        Component::Tooltip,
        "edge",
        "tooltip stays inside the viewport near an edge",
    ),
    f(
        Component::Dialog,
        "titled",
        "dialog paints a scrim, titled surface and application-owned body",
    ),
    f(
        Component::ConfirmDialog,
        "danger",
        "confirm dialog keeps cancel and danger confirm actions",
    ),
    f(
        Component::ConfirmDialog,
        "busy",
        "busy confirm dialog disables dismiss and shows a loading confirm",
    ),
    f(
        Component::Drawer,
        "right",
        "right drawer docks to the viewport edge over a scrim",
    ),
    f(
        Component::Drawer,
        "left",
        "left drawer docks to the start edge over a scrim",
    ),
    f(
        Component::Toast,
        "info",
        "info toast is an outlined tone card with a title",
    ),
    f(
        Component::Toast,
        "dismissible",
        "dismissible toast keeps a real dismiss affordance",
    ),
    f(
        Component::XYPad,
        "rest",
        "xy pad shows the two-axis value inside a field border",
    ),
    f(
        Component::XYPad,
        "invalid",
        "invalid xy pad uses a danger field border",
    ),
    f(
        Component::QrCode,
        "encoded",
        "qr code paints a white quiet zone and black modules",
    ),
    f(
        Component::Select,
        "closed",
        "select shows the selected label inside a field with a handle",
    ),
    f(
        Component::Select,
        "opened",
        "opened select keeps the field and paints a surface menu of options",
    ),
    f(
        Component::Select,
        "invalid",
        "invalid select uses a danger field border",
    ),
    f(
        Component::Popover,
        "open",
        "popover paints an anchored surface without a scrim",
    ),
    f(
        Component::ActionMenu,
        "open",
        "action menu uses start alignment and compact padding",
    ),
    f(
        Component::ActionMenuItem,
        "danger",
        "danger action menu item uses danger text and an optional hint",
    ),
    f(
        Component::AnchoredActionMenu,
        "open",
        "anchored action menu pins a menu surface to a logical point",
    ),
    f(
        Component::ContextMenu,
        "open",
        "context menu opens at the pointer anchor",
    ),
    f(
        Component::SidebarFrame,
        "chrome",
        "fixed top and footer stay outside the independently scrolling body",
    ),
    f(
        Component::SidebarSection,
        "expanded",
        "section header uses uppercase faint title, count and an expanded body",
    ),
    f(
        Component::SidebarSection,
        "collapsed",
        "collapsed section clips the body while keeping the header",
    ),
    f(
        Component::SidebarFooter,
        "actions",
        "footer hugs small icon actions without growing",
    ),
    f(
        Component::AppearanceSection,
        "solid",
        "appearance rows keep host-owned theme, material and radius controls",
    ),
    f(
        Component::AboutSection,
        "metadata",
        "about section shows injected name, version and description",
    ),
    f(
        Component::SettingsCollapsibleCard,
        "expanded",
        "collapsible card shows summary, divider and details when expanded",
    ),
    f(
        Component::SettingsCollapsibleCard,
        "collapsed",
        "collapsed card keeps summary and hides details",
    ),
    f(
        Component::CommandPalette,
        "open",
        "command palette shows search field and windowed rows",
    ),
    f(
        Component::OverlayHost,
        "stacked",
        "overlay host keeps exclusive stacking order",
    ),
    f(
        Component::Dropdown,
        "closed",
        "dropdown field shows the selected value and keeps disabled options",
    ),
    f(
        Component::SearchDropdown,
        "closed",
        "search dropdown field shows the committed query surface",
    ),
    f(
        Component::TreeView,
        "expanded",
        "tree view flattens visible disclosure rows",
    ),
    f(
        Component::SidebarRow,
        "active",
        "sidebar row keeps a 14px leading icon without a selected plate",
    ),
    f(
        Component::Settings,
        "row-card",
        "settings card groups a labeled control row",
    ),
    f(
        Component::SettingsSidebar,
        "settings-sidebar",
        "settings sidebar keeps back and tab rows in the frame",
    ),
    f(
        Component::SettingsPage,
        "settings-page",
        "settings page shows the tab title above host content",
    ),
    f(
        Component::SettingsPage,
        "settings-page-full",
        "full-page settings tab fills with content only",
    ),
    f(
        Component::Workspace,
        "default-regions",
        "workspace lays out start, primary, end and bottom regions from the model",
    ),
    f(
        Component::Dock,
        "split-tabs",
        "dock paints a split with a tabbed leaf and an item leaf",
    ),
    f(
        Component::DockPanel,
        "bordered",
        "dock panel is a radius-0 surface with a soft border",
    ),
    f(
        Component::SplitPane,
        "horizontal",
        "split pane sizes the first child and keeps an 8px handle",
    ),
    f(
        Component::PaneChrome,
        "active",
        "pane chrome keeps a 34px header over the body",
    ),
    f(
        Component::PaneTree,
        "nested",
        "pane tree preserves leaf order across a nested split",
    ),
    f(
        Component::AppShell,
        "stacked",
        "app shell stacks a 36px title bar over a fill body",
    ),
    f(
        Component::DesktopShell,
        "desktop-settings",
        "desktop shell stacks title, resources sidebar and primary settings page",
    ),
    f(
        Component::AppTitleBar,
        "titled",
        "title bar is 36px with a centered title",
    ),
];

const fn f(component: Component, state: &'static str, expected: &'static str) -> Fixture {
    Fixture {
        id: component.id(),
        component,
        state,
        expected,
        reference_contract: "reference rendered; interaction semantics may be incomplete",
        runtime_contract: "canonical frame must settle with layout, hit-test, accessibility and scene",
        divergence: "none unless recorded by the observed verdict",
    }
}

fn tooltip_fixture_config(state: &str) -> TooltipConfig {
    TooltipConfig {
        placement: if matches!(state, "edge" | "tooltip-edge") {
            TooltipPlacement::Left
        } else {
            TooltipPlacement::FollowCursor
        },
        delay_ms: if state == "delay" { 350 } else { 0 },
        gap: 6.0,
        viewport_padding: 4.0,
        max_width: 280.0,
    }
}

pub(super) fn generate_registered(
    snapshots: &mut super::offscreen::OffscreenSnapshots,
    output: &Path,
    theme: ThemeMode,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    validate_fixture_registry().map_err(std::io::Error::other)?;

    let colors = theme.colors();
    let gpu = gpu::create_snapshot_gpu(
        &snapshots.device,
        &snapshots.queue,
        colors.background,
        colors.accent_strong,
    );
    let mut paths = Vec::with_capacity(FIXTURE_REGISTRY.len() * 5);
    for fixture in FIXTURE_REGISTRY {
        paths.extend(render_fixture(snapshots, output, theme, *fixture, &gpu)?);
    }
    Ok(paths)
}

fn validate_fixture_registry() -> Result<(), String> {
    use std::collections::BTreeSet;

    let catalog_ids = component_catalog()
        .iter()
        .map(|support| support.id)
        .collect::<BTreeSet<_>>();
    let mut registered_ids = BTreeSet::new();
    let mut registered_states = BTreeSet::new();

    for fixture in FIXTURE_REGISTRY {
        let id = fixture.id;
        if !catalog_ids.contains(&id) {
            return Err(format!(
                "snapshot fixture references component `{id}` absent from the catalog"
            ));
        }
        if !registered_states.insert((id, fixture.state)) {
            return Err(format!(
                "duplicate snapshot fixture for component `{id}` state `{}`",
                fixture.state
            ));
        }
        registered_ids.insert(id);
    }

    let missing = component_catalog()
        .iter()
        .filter(|support| {
            support.compiled
                && support.migration == ComponentMigrationState::RuntimeQualified
                && !registered_ids.contains(&support.id)
        })
        .map(|support| support.id.as_str())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "compiled RuntimeQualified components lack snapshot fixtures: {}",
            missing.join(", ")
        ))
    }
}

fn render_fixture(
    snapshots: &mut super::offscreen::OffscreenSnapshots,
    output: &Path,
    theme: ThemeMode,
    fixture: Fixture,
    gpu: &SnapshotGpu,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let size = fixture_size(fixture);
    let theme_name = match theme {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    };
    let directory = output
        .join("component-migration")
        .join(fixture.id.as_str())
        .join(theme_name)
        .join(fixture.state);

    let runtime = runtime_fixture(theme, fixture, size)?;
    let (host_textures, gpu_renderers) = if is_gpu_fixture(fixture) {
        (Some(&gpu.textures), Some(&gpu.renderers))
    } else {
        (None, None)
    };
    let runtime_pixels = snapshots.paint(
        runtime.document.scene(),
        size,
        [
            theme.colors().background.r,
            theme.colors().background.g,
            theme.colors().background.b,
            theme.colors().background.a,
        ],
        host_textures,
        gpu_renderers,
    )?;
    let runtime_path = directory.join("runtime.png");
    write::png(&runtime_path, size, &runtime_pixels)?;

    let reference_path = directory.join("reference.png");
    let reference_pixels = if let Some((png_size, pixels)) = write::read_png(&reference_path) {
        if png_size == size && pixels.len() == runtime_pixels.len() {
            pixels
        } else {
            runtime_pixels.clone()
        }
    } else {
        write::png(&reference_path, size, &runtime_pixels)?;
        runtime_pixels.clone()
    };

    let side_size = Size::new(size.width * 2 + GAP, size.height);
    let side = side_by_side(&reference_pixels, &runtime_pixels, size, GAP);
    let side_path = directory.join("side-by-side.png");
    write::png(&side_path, side_size, &side)?;
    let difference = pixel_difference(&reference_pixels, &runtime_pixels);
    let difference_path = directory.join("difference.png");
    write::png(&difference_path, size, &difference)?;

    let evidence_path = directory.join("evidence.txt");
    write_evidence(&evidence_path, fixture, &runtime)?;
    Ok(vec![
        reference_path,
        runtime_path,
        side_path,
        difference_path,
        evidence_path,
    ])
}

fn fixture_size(fixture: Fixture) -> Size<u32> {
    match (fixture.component, fixture.state) {
        (Component::EmptyState, "complete-action") => Size::new(420, 190),
        (Component::EmptyState, "narrow-cjk") => Size::new(220, 220),
        (Component::EmptyState, "extreme-clip") => Size::new(92, 180),
        (Component::FormField, _) => Size::new(420, 160),
        (Component::InteractiveCard, _) => Size::new(420, 140),
        (Component::Tooltip, _) => Size::new(420, 140),
        (Component::Dialog, _) => Size::new(560, 280),
        (Component::ConfirmDialog, _) => Size::new(560, 260),
        (Component::Drawer, _) => Size::new(420, 240),
        (Component::Toast, _) => Size::new(420, 88),
        (Component::XYPad, _) => Size::new(420, 88),
        (Component::QrCode, _) => Size::new(280, 280),
        (Component::Select, "opened") => Size::new(420, 220),
        (Component::Popover | Component::ActionMenu, _) => Size::new(420, 200),
        (Component::AnchoredActionMenu | Component::ContextMenu, _) => Size::new(420, 220),
        (Component::SidebarFrame, _) => Size::new(240, 280),
        (Component::SidebarSection, _) => Size::new(240, 180),
        (Component::SidebarFooter, _) => Size::new(240, 72),
        (Component::AppearanceSection, _) => Size::new(420, 560),
        (Component::AboutSection, _) => Size::new(420, 180),
        (Component::SettingsCollapsibleCard, _) => Size::new(420, 160),
        (Component::CommandPalette, _) => Size::new(560, 320),
        (Component::OverlayHost, _) => Size::new(420, 160),
        (Component::TreeView, _) => Size::new(280, 160),
        (Component::Settings, _) => Size::new(420, 140),
        (Component::SettingsSidebar, _) => Size::new(220, 400),
        (Component::SettingsPage, _) => Size::new(420, 360),
        (Component::Workspace, _) => Size::new(720, 400),
        (Component::Dock, _) => Size::new(640, 320),
        (Component::DockPanel, _) => Size::new(280, 120),
        (Component::SplitPane, _) => Size::new(480, 200),
        (Component::PaneChrome, _) => Size::new(420, 180),
        (Component::PaneTree, _) => Size::new(480, 240),
        (Component::AppShell, _) => Size::new(560, 280),
        (Component::DesktopShell, _) => Size::new(560, 360),
        (Component::AppTitleBar, _) => Size::new(560, 80),
        (Component::CalendarHeatmap, _) => Size::new(280, 180),
        (Component::TimeSeriesChart, _) => Size::new(420, 180),
        (Component::NativeMarkdown, _) => Size::new(420, 140),
        (Component::ImageViewer, _) => Size::new(420, 240),
        (Component::GraphCanvas, _) => Size::new(420, 180),
        (Component::Thumbnail, "wide") => Size::new(80, 40),
        (Component::Thumbnail, _) => Size::new(40, 40),
        _ => SIZE,
    }
}

fn textarea_is_focused(state: &str) -> bool {
    matches!(
        state,
        "focused" | "selection" | "multiline-selection" | "invalid-focused" | "scroll"
    )
}

fn is_gpu_fixture(fixture: Fixture) -> bool {
    matches!(
        fixture.component,
        Component::GpuTextureView | Component::GpuView
    ) || (fixture.component == Component::Thumbnail && fixture.state == "ready")
}

struct RuntimeEvidence {
    document: RuntimeDocument,
    target: StableNodeId,
    first_passes: usize,
    first_accessibility_updates: usize,
    final_passes: usize,
    final_accessibility_updates: usize,
    idle: bool,
    action_applied: bool,
    feedback_contract_ok: bool,
    segmented_contract_ok: bool,
    segmented_options: Vec<StableNodeId>,
    segmented_requests: usize,
    next_deadline: Option<Duration>,
}

struct FeedbackActionFixture {
    action: Entity<RuntimeButton>,
    replacement: Entity<RuntimeButton>,
    activations: Arc<Mutex<usize>>,
}

struct SegmentedFixture {
    control: Entity<RuntimeSegmentedControl>,
    options: Vec<Entity<RuntimeSegmentedOption>>,
    requests: Arc<Mutex<Vec<StableNodeId>>>,
}

fn create_segmented_fixture(
    document: &mut RuntimeDocument,
    fixture: Fixture,
) -> Result<SegmentedFixture, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let control = document.context_mut().create_component(
        document_id,
        RuntimeSegmentedControl::new()
            .label("Editor mode")
            .size(segmented_control_size(fixture.state)),
    )?;
    let specs: &[(&str, Option<Icon>, bool)] = match fixture.state {
        "empty" => &[],
        "all-disabled" => &[
            ("Code", None, true),
            ("Split", None, true),
            ("Preview", None, true),
        ],
        "medium-icon" => &[
            ("Code", Some(Icon::File), false),
            ("Split", None, true),
            ("Preview", None, false),
        ],
        _ => &[
            ("Code", None, false),
            ("Split", None, true),
            ("Preview", None, false),
        ],
    };
    let mut options = Vec::with_capacity(specs.len());
    for (label, icon, disabled) in specs {
        let mut option = RuntimeSegmentedOption::new(*label).disabled(*disabled);
        if let Some(icon) = icon {
            option = option.icon(*icon);
        }
        options.push(
            document
                .context_mut()
                .create_detached_component(document_id, option)?,
        );
    }
    let selected = match fixture.state {
        "empty" | "all-disabled" | "no-selection" => None,
        "disabled-selected" => options.get(1).copied(),
        _ => options.first().copied(),
    };
    document
        .context_mut()
        .set_segmented_options(control, options.clone(), selected)?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    document.context_mut().on(
        control,
        move |_control, event: &SegmentedSelectionRequested, _context| {
            observed
                .lock()
                .expect("segmented request log")
                .push(event.option);
        },
    )?;
    Ok(SegmentedFixture {
        control,
        options,
        requests,
    })
}

fn create_tabs_fixture(
    document: &mut RuntimeDocument,
    fixture: Fixture,
) -> Result<Entity<RuntimeTabs>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let tabs = document.context_mut().create_component(
        document_id,
        RuntimeTabs::new("code").label("Editor mode").options([
            RuntimeTabOption::new("code", "Code"),
            RuntimeTabOption::new("split", "Split").disabled(true),
            RuntimeTabOption::new("preview", "Preview"),
        ]),
    )?;
    if fixture.state == "focused"
        && let Some(first) = document
            .context()
            .read(tabs, |tabs| tabs.option_nodes().first().map(|(_, id)| *id))?
    {
        document.context_mut().focus_node(document_id, first)?;
    }
    Ok(tabs)
}

fn runtime_fixture(
    theme: ThemeMode,
    fixture: Fixture,
    size: Size<u32>,
) -> Result<RuntimeEvidence, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(9).expect("migration fixture document id");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    let mut root_style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut root_style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.height = Some(LengthSpec::Percent(100.0));
        layout.padding_left = Some(LengthSpec::Px(20.0));
        layout.padding_right = Some(LengthSpec::Px(20.0));
        let vertical_padding = if fixture.component == Component::Textarea {
            12.0
        } else {
            20.0
        };
        layout.padding_top = Some(LengthSpec::Px(vertical_padding));
        layout.padding_bottom = Some(LengthSpec::Px(vertical_padding));
    }
    let root = document
        .context_mut()
        .create_component(document_id, RuntimeList::new().style(root_style))?;
    let feedback_action = if matches!(
        (fixture.component, fixture.state),
        (Component::EmptyState, "complete-action") | (Component::LabeledValue, "action")
    ) {
        let (label, replacement_label, kind) = if fixture.component == Component::EmptyState {
            (
                "Create project",
                "Import project",
                nana_ui::ButtonKind::Primary,
            )
        } else {
            ("View revision", "Open revision", nana_ui::ButtonKind::Ghost)
        };
        let action = document
            .context_mut()
            .create_detached_component(document_id, RuntimeButton::new(label).kind(kind))?;
        let replacement = document.context_mut().create_detached_component(
            document_id,
            RuntimeButton::new(replacement_label).kind(kind),
        )?;
        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        document
            .context_mut()
            .on(action, move |_button, _event: &Activate, _context| {
                *observed.lock().expect("feedback activation count") += 1;
            })?;
        Some(FeedbackActionFixture {
            action,
            replacement,
            activations,
        })
    } else {
        None
    };

    let mut segmented_fixture = None;
    let target = match fixture.component {
        Component::Text => {
            let mut style = NodeStyle {
                foreground: Some(if fixture.state == "muted" {
                    SemanticColorRole::Muted
                } else {
                    SemanticColorRole::Text
                }),
                text_horizontal_alignment: if fixture.state == "centered" {
                    TextHorizontalAlignment::Center
                } else {
                    TextHorizontalAlignment::Start
                },
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            };
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(if matches!(fixture.state, "wrap" | "ellipsis") {
                LengthSpec::Px(180.0)
            } else {
                LengthSpec::Percent(100.0)
            });
            layout.height = Some(LengthSpec::Px(if fixture.state == "wrap" {
                44.0
            } else {
                32.0
            }));
            layout.font_size = Some(13.0);
            layout.white_space_nowrap = fixture.state == "ellipsis";
            layout.text_overflow_ellipsis = fixture.state == "ellipsis";
            document
                .context_mut()
                .create_component(
                    document_id,
                    RuntimeText::new(if matches!(fixture.state, "wrap" | "ellipsis") {
                        "A deliberately long migration label that must respect its authored content box."
                    } else {
                        "Migration text 文本"
                    })
                    .style(style),
                )?
                .stable_id()
        }
        Component::Button => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeButton::new("Run build")
                    .kind(button_kind(fixture.state))
                    .size(button_control_size(fixture.state))
                    .disabled(fixture.state == "disabled")
                    .loading(fixture.state == "loading"),
            )?
            .stable_id(),
        Component::TextInput => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTextInput::new(if fixture.state == "placeholder" {
                    ""
                } else {
                    "release/next"
                })
                .label("Branch name")
                .placeholder("Branch name")
                .size(text_input_control_size(fixture.state))
                .disabled(fixture.state == "disabled")
                .loading(fixture.state == "loading")
                .read_only(fixture.state == "read-only")
                .invalid(fixture.state == "invalid")
                .secure(fixture.state == "secure"),
            )?
            .stable_id(),
        Component::Textarea => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTextArea::new(textarea_value(fixture.state))
                    .label("Issue description")
                    .placeholder("Describe the issue")
                    .height(96.0)
                    .invalid(fixture.state == "invalid-focused")
                    .disabled(fixture.state == "disabled"),
            )?
            .stable_id(),
        Component::HostedTextarea => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeHostedTextarea::new(hosted_textarea_value(fixture.state), "rs")
                    .label("Highlighted source")
                    .placeholder("fn main")
                    .height(96.0)
                    .disabled(fixture.state == "disabled"),
            )?
            .stable_id(),
        Component::CalendarHeatmap => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCalendarHeatmap::new([
                    RuntimeCalendarDatum::<()>::new("2026-06-01", 1.0),
                    RuntimeCalendarDatum::<()>::new("2026-06-02", 4.0),
                    RuntimeCalendarDatum::<()>::new("2026-06-03", 8.0),
                ]),
            )?
            .stable_id(),
        Component::TimeSeriesChart => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTimeSeriesChart::new([2.0, 5.0, 3.0, 8.0]),
            )?
            .stable_id(),
        Component::ReorderList => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeReorderList::new([
                    RuntimeReorderItem::new("alpha", "Alpha").selected(true),
                    RuntimeReorderItem::new("beta", "Beta"),
                    RuntimeReorderItem::new("gamma", "Gamma"),
                ]),
            )?
            .stable_id(),
        Component::NativeMarkdown => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeNativeMarkdown::from_blocks([
                    MarkdownBlock::Text {
                        kind: MarkdownBlockKind::Heading(1),
                        spans: vec![MarkdownSpan::plain("Title")],
                    },
                    MarkdownBlock::Text {
                        kind: MarkdownBlockKind::Paragraph,
                        spans: vec![MarkdownSpan::plain("Body copy.")],
                    },
                ]),
            )?
            .stable_id(),
        Component::SelectableRichText => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeSelectableRichText::new([RichSpan::plain("See "), RichSpan::plain("docs")]),
            )?
            .stable_id(),
        Component::ImageViewer => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeImageViewer::new(ImageViewerContent::None).name("Preview"),
            )?
            .stable_id(),
        Component::GraphCanvas => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeGraphCanvas::new("snapshot", snapshot_graph()),
            )?
            .stable_id(),
        Component::KeyCaptureLayer => document
            .context_mut()
            .create_component(document_id, RuntimeKeyCaptureLayer::new().recording(true))?
            .stable_id(),
        Component::KeymapLayer => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeKeymapLayer::new(
                    nana_ui::runtime::Keymap::new([]),
                    nana_ui_core::KeyContext::default(),
                    nana_ui::runtime::ActionRegistry::new(),
                ),
            )?
            .stable_id(),
        Component::Checkbox => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCheckbox::new(
                    "Notifications",
                    matches!(
                        fixture.state,
                        "on" | "pointer-toggle" | "space-toggle" | "accessibility-toggle"
                    ),
                )
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid"),
            )?
            .stable_id(),
        Component::IconButton => {
            let tooltip = TooltipConfig {
                placement: if fixture.state == "tooltip-edge" {
                    TooltipPlacement::Left
                } else {
                    TooltipPlacement::FollowCursor
                },
                ..TooltipConfig::default()
            };
            let component = RuntimeIconButton::new(Icon::Add, "Add source")
                .selected(fixture.state == "selected")
                .disabled(fixture.state == "disabled")
                .tooltip("Add source", tooltip);
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Switch => {
            let mut component = RuntimeSwitch::new("Auto build", fixture.state == "on")
                .hint("Run when sources change")
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid");
            component.control_position = if fixture.state == "control-start" {
                SwitchControlPosition::Start
            } else {
                SwitchControlPosition::End
            };
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Card => {
            let mut component = RuntimeCard::new()
                .title("Pipeline")
                .kind(card_kind(fixture.state))
                .loading(fixture.state == "loading")
                .padding(if fixture.state == "padding" {
                    28.0
                } else {
                    14.0
                });
            if fixture.state == "fixed-height" {
                component = component.height(92.0);
            }
            set_full_width(&mut component.style);
            let card = document
                .context_mut()
                .create_component(document_id, component)?;
            let body = document.context_mut().create_component(
                document_id,
                RuntimeText::new(if fixture.state == "long-content" {
                    "A deliberately long body that must remain inside the card content region even when space is constrained."
                } else {
                    "Build status: ready"
                }),
            )?;
            document.context_mut().append_child(card, body)?;
            card.stable_id()
        }
        Component::ListItem => {
            let mut component = RuntimeListItem::new(if fixture.state == "auto-height" {
                "Primary line\nSupporting line"
            } else {
                "Camera source"
            })
            .selected(fixture.state.starts_with("selected"))
            .disabled(fixture.state == "disabled")
            .size(control_size(fixture.state))
            .auto_height(fixture.state == "auto-height");
            set_full_width(&mut component.style);
            if fixture.state == "three-slots" {
                let leading = document
                    .context_mut()
                    .create_component(document_id, RuntimeText::new("●"))?;
                let content = document
                    .context_mut()
                    .create_component(document_id, RuntimeText::new("Camera source"))?;
                let trailing = document
                    .context_mut()
                    .create_component(document_id, RuntimeText::new("⌘1"))?;
                let slots = ListItemSlots {
                    leading: Some(leading.stable_id()),
                    content: Some(content.stable_id()),
                    trailing: Some(trailing.stable_id()),
                };
                let item = document
                    .context_mut()
                    .create_component(document_id, component)?;
                document.context_mut().set_list_item_slots(item, slots)?;
                item.stable_id()
            } else {
                document
                    .context_mut()
                    .create_component(document_id, component)?
                    .stable_id()
            }
        }
        Component::RangeField => {
            let mut component = RuntimeRangeField::new(range_value(fixture.state), 0.0, 1.0, 0.1)?
                .label("Opacity")
                .unit("×")
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid");
            set_full_width(&mut component.style);
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::SegmentedControl => {
            let segmented = create_segmented_fixture(&mut document, fixture)?;
            let target = segmented.control.stable_id();
            segmented_fixture = Some(segmented);
            target
        }
        Component::StatusBadge => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeStatusBadge::new(
                    status_badge_label(fixture.state),
                    status_tone(fixture.state),
                ),
            )?
            .stable_id(),
        Component::ValidationMessage => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeValidationMessage::new(
                    validation_message(fixture.state),
                    validation_intent(fixture.state),
                ),
            )?
            .stable_id(),
        Component::EmptyState => {
            let mut empty = RuntimeEmptyState::new(empty_title(fixture.state));
            if fixture.state != "title-only" {
                empty = empty
                    .icon(Icon::Folder)
                    .message(empty_message(fixture.state));
            }
            if fixture.state == "compact" {
                empty = empty.compact(true);
            }
            if fixture.state == "extreme-clip" {
                let mut style = NodeStyle::default();
                Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(100.0));
                empty = empty.style(style);
            }
            let empty = document
                .context_mut()
                .create_component(document_id, empty)?;
            if let Some(action) = feedback_action.as_ref() {
                document
                    .context_mut()
                    .set_empty_state_action(empty, Some(action.action.stable_id()))?;
            }
            empty.stable_id()
        }
        Component::LabeledValue => {
            let summary = document.context_mut().create_component(
                document_id,
                RuntimeLabeledValue::new("Revision", "42").emphasis(if fixture.state == "strong" {
                    ValueEmphasis::Strong
                } else {
                    ValueEmphasis::Normal
                }),
            )?;
            if let Some(action) = feedback_action.as_ref() {
                document
                    .context_mut()
                    .set_labeled_value_action(summary, Some(action.action.stable_id()))?;
            }
            summary.stable_id()
        }
        Component::Progress => {
            let value = if fixture.state == "empty" { 0.0 } else { 42.0 };
            let mut progress = RuntimeProgress::new(value, 100.0);
            if fixture.state == "labeled" {
                progress = progress.label("Copying");
            }
            set_full_width(&mut progress.style);
            document
                .context_mut()
                .create_component(document_id, progress)?
                .stable_id()
        }
        Component::Spinner => document
            .context_mut()
            .create_component(document_id, RuntimeSpinner::new("Loading").phase(0.25))?
            .stable_id(),
        Component::Tabs => create_tabs_fixture(&mut document, fixture)?.stable_id(),
        Component::Skeleton => document
            .context_mut()
            .create_component(document_id, RuntimeSkeleton::fill_width(16.0))?
            .stable_id(),
        Component::LevelMeter => {
            let mut meter = RuntimeLevelMeter::new(0.65).tone(StatusTone::Success);
            set_full_width(&mut meter.style);
            document
                .context_mut()
                .create_component(document_id, meter)?
                .stable_id()
        }
        Component::FormField => {
            let field = document.context_mut().create_component(
                document_id,
                RuntimeFormField::new("Email").error("Required"),
            )?;
            let control = document.context_mut().create_detached_component(
                document_id,
                RuntimeTextInput::new("name@studio.local").placeholder("name@studio.local"),
            )?;
            document
                .context_mut()
                .set_form_field_control(field, Some(control.stable_id()))?;
            field.stable_id()
        }
        Component::InteractiveCard => {
            let card = document
                .context_mut()
                .create_component(document_id, RuntimeInteractiveCard::new().selected(true))?;
            let label = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("Interactive surface"))?;
            document.context_mut().append_child(card, label)?;
            card.stable_id()
        }
        Component::Tooltip => {
            let component = RuntimeIconButton::new(Icon::Add, "Add source")
                .tooltip("Add source", tooltip_fixture_config(fixture.state));
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Dialog => {
            let dialog = document.context_mut().create_component(
                document_id,
                RuntimeDialog::new("Rename scene")
                    .description("This updates the workspace label.")
                    .size(DialogSize::Default),
            )?;
            let body = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("Camera A"))?;
            let close = document.context_mut().create_detached_component(
                document_id,
                RuntimeIconButton::new(Icon::Close, "Close"),
            )?;
            document.context_mut().set_modal_slots(
                dialog,
                ModalSlots {
                    body: Some(body.stable_id()),
                    close_action: Some(close.stable_id()),
                    ..ModalSlots::default()
                },
            )?;
            dialog.stable_id()
        }
        Component::ConfirmDialog => {
            let mut confirm = RuntimeConfirmDialog::new("Delete take", "This cannot be undone.");
            confirm.danger = fixture.state == "danger";
            confirm.busy = fixture.state == "busy";
            let confirm = document
                .context_mut()
                .create_component(document_id, confirm)?;
            let cancel = document
                .context_mut()
                .create_detached_component(document_id, RuntimeButton::new("取消"))?;
            let accept = document.context_mut().create_detached_component(
                document_id,
                RuntimeButton::new(if fixture.state == "busy" {
                    "处理中"
                } else {
                    "确认"
                })
                .kind(if fixture.state == "danger" {
                    nana_ui::ButtonKind::Danger
                } else {
                    nana_ui::ButtonKind::Primary
                })
                .loading(fixture.state == "busy"),
            )?;
            let close = (fixture.state != "busy")
                .then(|| {
                    document.context_mut().create_detached_component(
                        document_id,
                        RuntimeIconButton::new(Icon::Close, "Close"),
                    )
                })
                .transpose()?;
            document.context_mut().set_confirm_slots(
                confirm,
                ConfirmSlots {
                    body: None,
                    close_action: close.map(|close| close.stable_id()),
                    cancel: cancel.stable_id(),
                    secondary: None,
                    confirm: accept.stable_id(),
                },
            )?;
            confirm.stable_id()
        }
        Component::Drawer => {
            let drawer = document.context_mut().create_component(
                document_id,
                RuntimeDrawer::new("Inspector").side(if fixture.state == "left" {
                    DrawerSide::Left
                } else {
                    DrawerSide::Right
                }),
            )?;
            let body = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("Properties"))?;
            let close = document.context_mut().create_detached_component(
                document_id,
                RuntimeIconButton::new(Icon::Close, "Close"),
            )?;
            document.context_mut().set_modal_slots(
                drawer,
                ModalSlots {
                    body: Some(body.stable_id()),
                    close_action: Some(close.stable_id()),
                    ..ModalSlots::default()
                },
            )?;
            drawer.stable_id()
        }
        Component::Toast => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeToast::new(
                    if fixture.state == "dismissible" {
                        "Export complete"
                    } else {
                        "Listening"
                    },
                    if fixture.state == "dismissible" {
                        ToastTone::Success
                    } else {
                        ToastTone::Info
                    },
                )
                .description(if fixture.state == "dismissible" {
                    "Master sent to disk."
                } else {
                    "Program follow is armed."
                })
                .dismissible(fixture.state == "dismissible"),
            )?
            .stable_id(),
        Component::XYPad => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeXYPad::new(XYPadValue::new(0.35, 0.7)).invalid(fixture.state == "invalid"),
            )?
            .stable_id(),
        Component::QrCode => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeQrCode::encode("nana-ui://pair", 224.0).expect("fixture qr encodes"),
            )?
            .stable_id(),
        Component::Select => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeSelect::new(Some("code"))
                    .placeholder("Choose view")
                    .options([
                        RuntimeSelectOption::new("code", "Code"),
                        RuntimeSelectOption::new("split", "Split").disabled(true),
                        RuntimeSelectOption::new("preview", "Preview"),
                    ])
                    .invalid(fixture.state == "invalid")
                    .opened(fixture.state == "opened"),
            )?
            .stable_id(),
        Component::Popover => {
            let popover = document.context_mut().create_component(
                document_id,
                RuntimePopover::new().trigger("Details").open(true),
            )?;
            let body = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("Inspector content"))?;
            document.context_mut().append_child(popover, body)?;
            popover.stable_id()
        }
        Component::ActionMenu => {
            let menu = document.context_mut().create_component(
                document_id,
                RuntimeActionMenu::new().trigger("Actions").open(true),
            )?;
            let rename = document
                .context_mut()
                .create_detached_component(document_id, RuntimeActionMenuItem::new("Rename"))?;
            let delete = document.context_mut().create_detached_component(
                document_id,
                RuntimeActionMenuItem::new("Delete").danger(true),
            )?;
            document.context_mut().append_child(menu, rename)?;
            document.context_mut().append_child(menu, delete)?;
            menu.stable_id()
        }
        Component::ActionMenuItem => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeActionMenuItem::new("Delete").hint("⌫").danger(true),
            )?
            .stable_id(),
        Component::AnchoredActionMenu => {
            let menu = document.context_mut().create_component(
                document_id,
                RuntimeAnchoredActionMenu::new(24.0, 36.0)
                    .menu_size(200.0, 0.0)
                    .open(true),
            )?;
            let rename = document
                .context_mut()
                .create_detached_component(document_id, RuntimeActionMenuItem::new("Rename"))?;
            let delete = document.context_mut().create_detached_component(
                document_id,
                RuntimeActionMenuItem::new("Delete").danger(true),
            )?;
            document.context_mut().append_child(menu, rename)?;
            document.context_mut().append_child(menu, delete)?;
            menu.stable_id()
        }
        Component::ContextMenu => {
            let menu = document.context_mut().create_component(
                document_id,
                RuntimeContextMenu::new(24.0, 36.0)
                    .items([
                        RuntimeContextMenuItem::new("rename", "Rename"),
                        RuntimeContextMenuItem::new("delete", "Delete").danger(true),
                    ])
                    .open(true),
            )?;
            let rename = document
                .context_mut()
                .create_detached_component(document_id, RuntimeActionMenuItem::new("Rename"))?;
            let delete = document.context_mut().create_detached_component(
                document_id,
                RuntimeActionMenuItem::new("Delete").danger(true),
            )?;
            document.context_mut().append_child(menu, rename)?;
            document.context_mut().append_child(menu, delete)?;
            menu.stable_id()
        }
        Component::SidebarFrame => mount_runtime_sidebar_frame(&mut document, fixture)?,
        Component::SidebarSection => mount_runtime_sidebar_section(
            &mut document,
            fixture.state != "collapsed",
            &["外观", "工作区"],
            true,
        )?,
        Component::SidebarFooter => {
            let footer = document
                .context_mut()
                .create_component(document_id, RuntimeSidebarFooter::new())?;
            let settings = document.context_mut().create_detached_component(
                document_id,
                RuntimeSidebarFooterButton::new("设置", Icon::Settings).selected(true),
            )?;
            let search = document.context_mut().create_detached_component(
                document_id,
                RuntimeSidebarFooterButton::new("搜索", Icon::Search),
            )?;
            document.context_mut().append_child(footer, settings)?;
            document.context_mut().append_child(footer, search)?;
            footer.stable_id()
        }
        Component::AppearanceSection => {
            let mut appearance = AppearanceSettings::default();
            if fixture.state != "solid" {
                let _ = appearance.set_window_material(WindowMaterialMode::Translucent);
            }
            let section = document.context_mut().create_component(
                document_id,
                RuntimeAppearanceSection::new(theme, appearance),
            )?;
            document
                .context_mut()
                .assemble_appearance_section(section)?;
            section.stable_id()
        }
        Component::AboutSection => {
            let section = document.context_mut().create_component(
                document_id,
                RuntimeAboutSection::new(
                    RuntimeAboutMetadata::new("NanaUI Gallery", "0.1.0")
                        .description("Injected product metadata for the about card."),
                ),
            )?;
            document.context_mut().assemble_about_section(section)?;
            section.stable_id()
        }
        Component::SettingsCollapsibleCard => {
            let summary = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("高级选项"))?;
            let details = document.context_mut().create_detached_component(
                document_id,
                RuntimeText::new("折叠后应隐藏这段说明。"),
            )?;
            let card = document.context_mut().create_component(
                document_id,
                RuntimeSettingsCollapsibleCard::new(fixture.state != "collapsed")
                    .summary(summary.stable_id())
                    .details(details.stable_id()),
            )?;
            document
                .context_mut()
                .assemble_settings_collapsible_card(card)?;
            card.stable_id()
        }
        Component::CommandPalette => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCommandPalette::new(
                    "命令面板",
                    [
                        CommandPaletteItem::new(ActionId::new("rename"), "重命名"),
                        CommandPaletteItem::new(ActionId::new("delete"), "删除"),
                    ],
                ),
            )?
            .stable_id(),
        Component::OverlayHost => {
            let host = document
                .context_mut()
                .create_component(document_id, RuntimeOverlayHost::new())?;
            let base = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("Base surface"))?;
            document.context_mut().append_child(host, base)?;
            host.stable_id()
        }
        Component::Dropdown => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeDropdown::single(Some("code"))
                    .placeholder("Choose view")
                    .options([
                        RuntimeDropdownOption::new("code", "Code"),
                        RuntimeDropdownOption::new("split", "Split").disabled(true),
                        RuntimeDropdownOption::new("preview", "Preview"),
                    ]),
            )?
            .stable_id(),
        Component::SearchDropdown => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeSearchDropdown::new(Some("code"))
                    .placeholder("Search views")
                    .options([
                        RuntimeSearchDropdownOption::new("code", "Code"),
                        RuntimeSearchDropdownOption::new("preview", "Preview"),
                    ]),
            )?
            .stable_id(),
        Component::TreeView => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTreeView::new([
                    TreeNode::branch(
                        std::sync::Arc::<str>::from("src"),
                        "src",
                        true,
                        [
                            TreeNode::leaf(std::sync::Arc::<str>::from("main"), "main.rs")
                                .selected(true),
                        ],
                    ),
                    TreeNode::leaf(std::sync::Arc::<str>::from("readme"), "README.md"),
                ]),
            )?
            .stable_id(),
        Component::SidebarRow => {
            let leading = document.context_mut().create_detached_component(
                document_id,
                nana_ui::runtime::SidebarRowIcon::new(Icon::Workspace),
            )?;
            let row = document.context_mut().create_component(
                document_id,
                RuntimeSidebarRow::new("工作区")
                    .state(nana_ui::runtime::SidebarRowState::Active)
                    .slots(nana_ui::runtime::ListItemSlots {
                        leading: Some(leading.stable_id()),
                        content: None,
                        trailing: None,
                    }),
            )?;
            document.context_mut().append_child(row, leading)?;
            row.stable_id()
        }
        Component::Settings => {
            let control = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("暗色"))?;
            let row = document.context_mut().mount_settings_leaf_row(
                document_id,
                "主题",
                Some("选择应用配色，立即生效"),
                control.stable_id(),
            )?;
            let card = document
                .context_mut()
                .create_component(document_id, RuntimeSettingsCard::new("外观"))?;
            document.context_mut().append_child(card, row)?;
            card.stable_id()
        }
        Component::SettingsSidebar => mount_runtime_settings_sidebar(&mut document)?,
        Component::SettingsPage => mount_runtime_settings_page(&mut document, theme, fixture)?,
        Component::Workspace => mount_runtime_workspace(&mut document)?,
        Component::Dock => mount_runtime_dock(&mut document)?,
        Component::DockPanel => {
            let title = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new("Inspector"))?;
            let hint = document.context_mut().create_detached_component(
                document_id,
                RuntimeText::new("Selection").style({
                    let mut style = NodeStyle {
                        foreground: Some(SemanticColorRole::Muted),
                        ..NodeStyle::default()
                    };
                    Arc::make_mut(&mut style.layout).font_size = Some(10.0);
                    style
                }),
            )?;
            let mut body_style = NodeStyle::default();
            {
                let layout = Arc::make_mut(&mut body_style.layout);
                layout.direction = Some(nana_ui_core::FlexDirection::Column);
                layout.gap = Some(LengthSpec::Px(4.0));
            }
            let body = document
                .context_mut()
                .create_detached_component(document_id, RuntimeList::new().style(body_style))?;
            document.context_mut().append_child(body, title)?;
            document.context_mut().append_child(body, hint)?;
            let panel = document.context_mut().create_component(
                document_id,
                RuntimeDockPanel::new()
                    .padding(10.0)
                    .content(body.stable_id()),
            )?;
            document.context_mut().append_child(panel, body)?;
            panel.stable_id()
        }
        Component::SplitPane => mount_runtime_split_pane(&mut document)?,
        Component::PaneChrome => mount_runtime_pane_chrome(&mut document)?,
        Component::PaneTree => mount_runtime_pane_tree(&mut document)?,
        Component::AppShell => mount_runtime_app_shell(&mut document)?,
        Component::DesktopShell => mount_runtime_desktop_shell(&mut document, theme)?,
        Component::AppTitleBar => document
            .context_mut()
            .create_component(document_id, RuntimeAppTitleBar::new("NanaUI"))?
            .stable_id(),
        Component::GpuTextureView => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeGpuTextureView::new(gpu::SNAPSHOT_GPU_SLOT),
            )?
            .stable_id(),
        Component::Thumbnail => {
            let thumb = match fixture.state {
                "ready" => RuntimeThumbnail::new(gpu::SNAPSHOT_GPU_SLOT),
                "wide" => RuntimeThumbnail::empty().aspect(16.0 / 9.0),
                _ => RuntimeThumbnail::empty(),
            };
            document
                .context_mut()
                .create_component(document_id, thumb)?
                .stable_id()
        }
        Component::GpuView => {
            let colors = theme.colors();
            document
                .context_mut()
                .create_component(
                    document_id,
                    RuntimeGpuView::new(1).palette(RuntimeGpuViewPalette {
                        background: [
                            colors.background.r,
                            colors.background.g,
                            colors.background.b,
                            colors.background.a,
                        ],
                        accent: [
                            colors.accent_strong.r,
                            colors.accent_strong.g,
                            colors.accent_strong.b,
                            colors.accent_strong.a,
                        ],
                    }),
                )?
                .stable_id()
        }
    };
    let mut hierarchy = MutationQueue::new();
    hierarchy.insert(root.stable_id(), target, None);
    document.context_mut().commit_mutations(hierarchy)?;

    let viewport = LayoutViewport::new(size.width as f32, size.height as f32);
    let mut shaper = NanaTextShaper::default();
    let first = document.flush(viewport, &mut shaper)?;
    let (action_applied, feedback_contract_ok, segmented_contract_ok) = if let Some(action) =
        feedback_action
    {
        let contract_ok = exercise_feedback_action_lifecycle(
            &mut document,
            viewport,
            &mut shaper,
            fixture,
            target,
            action,
        )?;
        (contract_ok, contract_ok, true)
    } else if let Some(segmented) = segmented_fixture.as_ref() {
        let contract_ok =
            exercise_segmented_contract(&mut document, viewport, &mut shaper, fixture, segmented)?;
        (contract_ok, true, contract_ok)
    } else {
        (
            apply_runtime_state(&mut document, fixture, target)?,
            true,
            true,
        )
    };
    let final_update = document.flush(viewport, &mut shaper)?;
    let idle = document.flush(viewport, &mut shaper)?.is_idle();
    let next_deadline = document.context().next_animation_deadline();
    let segmented_options = segmented_fixture
        .as_ref()
        .map(|segmented| {
            segmented
                .options
                .iter()
                .map(|option| option.stable_id())
                .collect()
        })
        .unwrap_or_default();
    let segmented_requests = segmented_fixture
        .as_ref()
        .map(|segmented| segmented.requests.lock().expect("segmented requests").len())
        .unwrap_or_default();
    Ok(RuntimeEvidence {
        document,
        target,
        first_passes: first.passes,
        first_accessibility_updates: first.accessibility.updated.len(),
        final_passes: final_update.passes,
        final_accessibility_updates: final_update.accessibility.updated.len(),
        idle,
        action_applied,
        feedback_contract_ok,
        segmented_contract_ok,
        segmented_options,
        segmented_requests,
        next_deadline,
    })
}

fn mount_runtime_sidebar_section(
    document: &mut RuntimeDocument,
    expanded: bool,
    labels: &[&str],
    collapsible: bool,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let spec = RuntimeSidebarSection::new("资源")
        .count(3)
        .collapsible(collapsible)
        .expanded(expanded);
    let disclosure = if collapsible {
        Some(
            document
                .context_mut()
                .create_detached_component(document_id, spec.disclosure_mark())?,
        )
    } else {
        None
    };
    let title = document
        .context_mut()
        .create_detached_component(document_id, spec.title_label())?;
    let count = document
        .context_mut()
        .create_detached_component(document_id, spec.count_label())?;
    let spec = spec
        .title_slot(title.stable_id())
        .count_slot(count.stable_id());
    let spec = match &disclosure {
        Some(disclosure) => spec.disclosure(disclosure.stable_id()),
        None => spec,
    };
    let header = document
        .context_mut()
        .create_detached_component(document_id, spec.header_item())?;
    let body = document
        .context_mut()
        .create_detached_component(document_id, RuntimeSidebarSection::body_port())?;
    for label in labels {
        let row = document
            .context_mut()
            .create_detached_component(document_id, RuntimeSidebarRow::new(*label))?;
        document.context_mut().append_child(body, row)?;
    }
    if let Some(disclosure) = disclosure {
        document.context_mut().append_child(header, disclosure)?;
    }
    document.context_mut().append_child(header, title)?;
    document.context_mut().append_child(header, count)?;
    let section = document.context_mut().create_component(
        document_id,
        spec.header(header.stable_id()).body(body.stable_id()),
    )?;
    document.context_mut().append_child(section, header)?;
    document.context_mut().append_child(section, body)?;
    Ok(section.stable_id())
}

fn slot_label_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    let inset = LengthSpec::Px(SLOT_INSET);
    layout.padding_left = Some(inset);
    layout.padding_right = Some(inset);
    layout.padding_top = Some(inset);
    layout.padding_bottom = Some(inset);
    style
}

fn mount_runtime_label(
    document: &mut RuntimeDocument,
    label: &str,
) -> Result<nana_ui::runtime::Entity<RuntimeText>, Box<dyn std::error::Error>> {
    mount_runtime_label_styled(document, label, NodeStyle::default())
}

fn mount_runtime_slot_label(
    document: &mut RuntimeDocument,
    label: &str,
) -> Result<nana_ui::runtime::Entity<RuntimeText>, Box<dyn std::error::Error>> {
    mount_runtime_label_styled(document, label, slot_label_style())
}

fn mount_runtime_label_styled(
    document: &mut RuntimeDocument,
    label: &str,
    style: NodeStyle,
) -> Result<nana_ui::runtime::Entity<RuntimeText>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    Ok(document
        .context_mut()
        .create_detached_component(document_id, RuntimeText::new(label).style(style))?)
}

fn mount_runtime_workspace(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let nav = mount_runtime_slot_label(document, "Nav")?;
    let files = mount_runtime_slot_label(document, "Files")?;
    let toolbar = mount_runtime_slot_label(document, "Toolbar")?;
    let primary = mount_runtime_slot_label(document, "Primary")?;
    let inspector = mount_runtime_slot_label(document, "Inspector")?;
    let diagnostics = mount_runtime_slot_label(document, "Diagnostics")?;
    let workspace = document.context_mut().create_component(
        document_id,
        RuntimeWorkspace::from_model(
            &WorkspaceModel::new(),
            [
                WorkspaceRegionSlot::new(RegionId::GlobalNavigation, nav.stable_id()),
                WorkspaceRegionSlot::new(RegionId::Resources, files.stable_id()),
                WorkspaceRegionSlot::new(RegionId::PrimaryToolbar, toolbar.stable_id()),
                WorkspaceRegionSlot::new(RegionId::Primary, primary.stable_id()),
                WorkspaceRegionSlot::new(RegionId::Inspector, inspector.stable_id()),
                WorkspaceRegionSlot::new(RegionId::Diagnostics, diagnostics.stable_id()),
            ],
        ),
    )?;
    document.context_mut().append_child(workspace, nav)?;
    document.context_mut().append_child(workspace, files)?;
    document.context_mut().append_child(workspace, toolbar)?;
    document.context_mut().append_child(workspace, primary)?;
    document.context_mut().append_child(workspace, inspector)?;
    document
        .context_mut()
        .append_child(workspace, diagnostics)?;
    document.context_mut().assemble_workspace(workspace)?;
    Ok(workspace.stable_id())
}

fn mount_runtime_dock(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let nav = mount_runtime_slot_label(document, "Nav")?;
    let files = mount_runtime_slot_label(document, "Files")?;
    let primary = mount_runtime_slot_label(document, "Primary")?;
    let dock = document.context_mut().create_component(
        document_id,
        RuntimeDock::new(RuntimeDockNode::split(
            nana_ui::runtime::DockAxis::Horizontal,
            0.35,
            RuntimeDockNode::tabs(
                ["nav", "files"],
                "nav",
                [
                    ("nav", Some(nav.stable_id())),
                    ("files", Some(files.stable_id())),
                ],
            ),
            RuntimeDockNode::item("primary", Some(primary.stable_id())),
        ))
        .title("nav", "Nav")
        .title("files", "Files")
        .title("primary", "Primary"),
    )?;
    document.context_mut().append_child(dock, nav)?;
    document.context_mut().append_child(dock, files)?;
    document.context_mut().append_child(dock, primary)?;
    document.context_mut().assemble_dock(dock)?;
    Ok(dock.stable_id())
}

fn mount_runtime_split_pane(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let first = mount_runtime_slot_label(document, "First")?;
    let second = mount_runtime_slot_label(document, "Second")?;
    let handle = document
        .context_mut()
        .create_detached_component(document_id, RuntimeText::new(""))?;
    let indicator = document
        .context_mut()
        .create_detached_component(document_id, RuntimeText::new(""))?;
    let pane = document.context_mut().create_component(
        document_id,
        RuntimeSplitPane::from_model(
            &SplitPaneModel::new(SplitAxis::Horizontal, 160.0, 80.0, 280.0),
            first.stable_id(),
            second.stable_id(),
        )
        .handle(handle.stable_id()),
    )?;
    document.context_mut().append_child(pane, first)?;
    document.context_mut().append_child(handle, indicator)?;
    document.context_mut().append_child(pane, handle)?;
    document.context_mut().append_child(pane, second)?;
    document.context_mut().update_component(pane, |_, _| {})?;
    Ok(pane.stable_id())
}

fn mount_runtime_pane_chrome(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let header = document
        .context_mut()
        .create_detached_component(document_id, RuntimeText::new(""))?;
    let tabs = mount_runtime_label(document, "editor.rs")?;
    let body = mount_runtime_label(document, "Body")?;
    let close = mount_runtime_label(document, "关闭")?;
    let chrome = document.context_mut().create_component(
        document_id,
        RuntimePaneChrome::new()
            .header(header.stable_id())
            .tabs(tabs.stable_id())
            .body(body.stable_id())
            .actions([nana_ui::runtime::PaneChromeAction::new(
                nana_ui::runtime::PaneChromeActionKind::CloseItem,
                "关闭",
            )
            .target(close.stable_id())])
            .active(true),
    )?;
    document.context_mut().append_child(chrome, header)?;
    document.context_mut().append_child(header, tabs)?;
    document.context_mut().append_child(header, close)?;
    document.context_mut().append_child(chrome, body)?;
    Ok(chrome.stable_id())
}

fn mount_runtime_pane_tree(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let left = mount_runtime_slot_label(document, "left")?;
    let right = mount_runtime_slot_label(document, "right")?;
    let document_id = document.document();
    let tree = document.context_mut().create_component(
        document_id,
        RuntimePaneTree::new(RuntimePaneTreeNode::split(
            "root",
            SplitAxis::Horizontal,
            0.4,
            RuntimePaneTreeNode::leaf_content("left", left.stable_id()),
            RuntimePaneTreeNode::leaf_content("right", right.stable_id()),
        )),
    )?;
    document.context_mut().append_child(tree, left)?;
    document.context_mut().append_child(tree, right)?;
    Ok(tree.stable_id())
}

fn mount_runtime_app_shell(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let title = document
        .context_mut()
        .create_detached_component(document_id, RuntimeAppTitleBar::new("NanaUI"))?;
    let body = mount_runtime_label(document, "Workspace")?;
    let shell = document.context_mut().create_component(
        document_id,
        RuntimeAppShell::new()
            .title_bar(title.stable_id())
            .body(body.stable_id()),
    )?;
    document.context_mut().append_child(shell, title)?;
    document.context_mut().append_child(shell, body)?;
    Ok(shell.stable_id())
}

fn snapshot_settings_model() -> &'static SettingsModel {
    static MODEL: std::sync::OnceLock<SettingsModel> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| {
        SettingsModel::new(
            "appearance",
            [
                SettingsTab::new("appearance", "外观").icon(Icon::Appearance),
                SettingsTab::new("about", "关于")
                    .icon(Icon::About)
                    .full_page(true),
            ],
        )
        .expect("snapshot settings model")
    })
}

fn snapshot_settings_state() -> &'static SettingsState {
    static STATE: std::sync::OnceLock<SettingsState> = std::sync::OnceLock::new();
    STATE.get_or_init(|| SettingsState::new(snapshot_settings_model()))
}

fn snapshot_settings_full_state() -> &'static SettingsState {
    static STATE: std::sync::OnceLock<SettingsState> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        let model = snapshot_settings_model();
        let mut state = SettingsState::new(model);
        state.select(model, &SettingsTabId::from("about"));
        state
    })
}

fn snapshot_desktop_workspace_layout() -> WorkspaceLayout {
    WorkspaceLayout::new([
        RegionState::new(RegionId::Resources, RegionRole::Resources)
            .size(220.0)
            .min_size(180.0)
            .max_size(480.0)
            .collapsible(true)
            .resizable(true),
        RegionState::new(RegionId::Primary, RegionRole::Primary)
            .min_size(160.0)
            .fill_priority(1),
    ])
    .expect("desktop-settings regions")
}

fn mount_runtime_appearance_section(
    document: &mut RuntimeDocument,
    theme: ThemeMode,
) -> Result<nana_ui::runtime::Entity<RuntimeAppearanceSection>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let section = document.context_mut().create_detached_component(
        document_id,
        RuntimeAppearanceSection::new(theme, AppearanceSettings::default()),
    )?;
    document
        .context_mut()
        .assemble_appearance_section(section)?;
    Ok(section)
}

fn mount_runtime_about_section(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::Entity<RuntimeAboutSection>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let section = document.context_mut().create_detached_component(
        document_id,
        RuntimeAboutSection::new(
            RuntimeAboutMetadata::new("NanaUI Gallery", "0.1.0")
                .description("Injected product metadata for the about card."),
        ),
    )?;
    document.context_mut().assemble_about_section(section)?;
    Ok(section)
}

fn mount_runtime_settings_sidebar(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let sidebar = document.context_mut().create_component(
        document_id,
        RuntimeSettingsSidebar::new(
            snapshot_settings_model().clone(),
            snapshot_settings_state().clone(),
        ),
    )?;
    document.context_mut().assemble_settings_sidebar(sidebar)?;
    Ok(sidebar.stable_id())
}

fn mount_runtime_settings_page(
    document: &mut RuntimeDocument,
    theme: ThemeMode,
    fixture: Fixture,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let full_page = fixture.state == "settings-page-full";
    let content = if full_page {
        mount_runtime_about_section(document)?.stable_id()
    } else {
        mount_runtime_appearance_section(document, theme)?.stable_id()
    };
    let state = if full_page {
        snapshot_settings_full_state().clone()
    } else {
        snapshot_settings_state().clone()
    };
    let page = document.context_mut().create_component(
        document_id,
        RuntimeSettingsPage::new(snapshot_settings_model().clone(), state).content(content),
    )?;
    document.context_mut().assemble_settings_page(page)?;
    Ok(page.stable_id())
}

fn mount_runtime_desktop_shell(
    document: &mut RuntimeDocument,
    theme: ThemeMode,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let model = snapshot_settings_model().clone();
    let state = snapshot_settings_state().clone();
    let sidebar = document.context_mut().create_detached_component(
        document_id,
        RuntimeSettingsSidebar::new(model.clone(), state.clone()),
    )?;
    document.context_mut().assemble_settings_sidebar(sidebar)?;
    let content = mount_runtime_appearance_section(document, theme)?;
    let page = document.context_mut().create_detached_component(
        document_id,
        RuntimeSettingsPage::new(model, state).content(content.stable_id()),
    )?;
    document.context_mut().assemble_settings_page(page)?;
    let shell = document.context_mut().create_component(
        document_id,
        RuntimeDesktopShell::from_model(WorkspaceModel::with_layout(
            snapshot_desktop_workspace_layout(),
        ))
        .title("NanaUI")
        .navigation(sidebar.stable_id())
        .primary(page.stable_id()),
    )?;
    document.context_mut().assemble_desktop_shell(shell)?;
    Ok(shell.stable_id())
}

fn mount_runtime_sidebar_frame(
    document: &mut RuntimeDocument,
    _fixture: Fixture,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let top = document
        .context_mut()
        .create_detached_component(document_id, RuntimeSidebarRow::new("返回"))?;
    let body = document
        .context_mut()
        .create_detached_component(document_id, RuntimeSidebarFrame::vertical_body_scroll())?;
    let section = mount_runtime_sidebar_section(
        document,
        true,
        &["外观", "工作区", "设置", "关于", "日志", "调试"],
        false,
    )?;
    document.context_mut().append_child(
        body,
        Entity::<RuntimeSidebarSection>::from_stable_id(section),
    )?;
    let footer = document
        .context_mut()
        .create_detached_component(document_id, RuntimeSidebarFooter::new())?;
    let settings = document.context_mut().create_detached_component(
        document_id,
        RuntimeSidebarFooterButton::new("设置", Icon::Settings).selected(true),
    )?;
    document.context_mut().append_child(footer, settings)?;
    let frame = document.context_mut().create_component(
        document_id,
        RuntimeSidebarFrame::new()
            .top(top.stable_id())
            .body(body.stable_id())
            .footer(footer.stable_id()),
    )?;
    document.context_mut().append_child(frame, top)?;
    document.context_mut().append_child(frame, body)?;
    document.context_mut().append_child(frame, footer)?;
    Ok(frame.stable_id())
}

fn exercise_segmented_contract(
    document: &mut RuntimeDocument,
    viewport: LayoutViewport,
    shaper: &mut NanaTextShaper,
    fixture: Fixture,
    segmented: &SegmentedFixture,
) -> Result<bool, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let adapter = RuntimeInputAdapter::default();
    let ids = segmented
        .options
        .iter()
        .map(|option| option.stable_id())
        .collect::<Vec<_>>();
    let center = |document: &RuntimeDocument, id: StableNodeId| {
        let bounds = document
            .context()
            .world()
            .layout_box(id)
            .expect("segmented option layout");
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    };
    let selected_before = document
        .context()
        .read(segmented.control, RuntimeSegmentedControl::selected)?;
    let action_ok = match fixture.state {
        "empty" | "all-disabled" => {
            !document
                .context_mut()
                .navigate_sequential_focus(document_id, false)?
                && document.context().world().focused(document_id).is_none()
        }
        "hover" | "selected-hover" => {
            let id = if fixture.state == "hover" {
                ids[2]
            } else {
                ids[0]
            };
            let (x, y) = center(document, id);
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Move, x, y),
                )?
                .prevent_default
        }
        "pressed" | "selected-pressed" => {
            let id = if fixture.state == "pressed" {
                ids[2]
            } else {
                ids[0]
            };
            let (x, y) = center(document, id);
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Down, x, y),
                )?
                .prevent_default
        }
        "focused" => document.context_mut().focus_node(document_id, ids[0])?,
        "pointer-request" | "selected-repeat-request" => {
            let id = if fixture.state == "pointer-request" {
                ids[2]
            } else {
                ids[0]
            };
            let (x, y) = center(document, id);
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Down, x, y),
            )?;
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Up, x, y),
                )?
                .prevent_default
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [id]
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    == selected_before
        }
        "pointer-cancel" => {
            let (x, y) = center(document, ids[2]);
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Down, x, y),
            )?;
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Cancel, x, y),
                )?
                .prevent_default
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .is_empty()
        }
        "arrow-skip-wrap" => {
            document.context_mut().focus_node(document_id, ids[0])?;
            let left = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("ArrowLeft"))?
                .prevent_default;
            let right = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("ArrowRight"))?
                .prevent_default;
            left && right
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2], ids[0]]
        }
        "home-end" => {
            document.context_mut().focus_node(document_id, ids[2])?;
            let home = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("Home"))?
                .prevent_default;
            let end = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("End"))?
                .prevent_default;
            home && end
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[0], ids[2]]
        }
        "space-enter-repeat" => {
            document.context_mut().focus_node(document_id, ids[0])?;
            let repeated_space = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Space", true),
            )?;
            let repeated_enter = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Enter", true),
            )?;
            let normal_space = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Space", false),
            )?;
            let normal_enter = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Enter", false),
            )?;
            repeated_space.prevent_default
                && repeated_enter.prevent_default
                && normal_space.prevent_default
                && normal_enter.prevent_default
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[0], ids[0]]
        }
        "no-selection" => {
            document
                .context_mut()
                .navigate_sequential_focus(document_id, false)?
                && document.context().world().focused(document_id) == Some(ids[0])
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    .is_none()
        }
        "dynamic-disable" => {
            document.context_mut().focus_node(document_id, ids[0])?;
            document.context_mut().set_segmented_option_disabled(
                segmented.control,
                segmented.options[0],
                true,
            )? && document.context().world().focused(document_id) == Some(ids[2])
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    == Some(ids[0])
                && document
                    .context()
                    .read(segmented.options[0], RuntimeSegmentedOption::selected)?
        }
        "controlled-commit" => {
            let (x, y) = center(document, ids[2]);
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Down, x, y),
            )?;
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Up, x, y),
            )?;
            let remained_controlled = document
                .context()
                .read(segmented.control, RuntimeSegmentedControl::selected)?
                == selected_before;
            let committed = document
                .context_mut()
                .set_segmented_selection(segmented.control, Some(segmented.options[2]))?;
            remained_controlled
                && committed
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    == Some(ids[2])
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2]]
        }
        "a11y-radio" => {
            document.context_mut().apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target: ids[2],
                    action: AccessibilityAction::Click,
                },
            )? && document
                .context()
                .read(segmented.control, RuntimeSegmentedControl::selected)?
                == selected_before
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2]]
        }
        "atomic-reconcile" => {
            let generation = document.context().world().generation();
            let invalid = document
                .context_mut()
                .set_segmented_options(
                    segmented.control,
                    vec![segmented.options[0], segmented.options[0]],
                    Some(segmented.options[0]),
                )
                .is_err();
            let failure_atomic = document.context().world().generation() == generation
                && document
                    .context()
                    .read(segmented.control, |control| control.options().to_vec())?
                    .as_slice()
                    == ids.as_slice();
            let removed = segmented.options[1];
            let old_bounds = document
                .context()
                .world()
                .layout_box(removed.stable_id())
                .expect("option layout before parking");
            let changed = document.context_mut().set_segmented_options(
                segmented.control,
                vec![segmented.options[0], segmented.options[2]],
                Some(segmented.options[0]),
            )?;
            let update = document.flush(viewport, shaper)?;
            let parked_clean = parked_without_ghost(
                document,
                removed.stable_id(),
                old_bounds,
                &update.accessibility.removed,
            );
            let handler_preserved = document
                .context_mut()
                .request_segmented_selection(segmented.control, segmented.options[2])?
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2]];
            invalid && failure_atomic && changed && parked_clean && handler_preserved
        }
        _ => true,
    };
    let selected_after = document
        .context()
        .read(segmented.control, RuntimeSegmentedControl::selected)?;
    let selection_ok = if fixture.state == "controlled-commit" {
        selected_after == ids.get(2).copied()
    } else {
        selected_after == selected_before
    };
    let expected_requests = match fixture.state {
        "pointer-request"
        | "selected-repeat-request"
        | "controlled-commit"
        | "a11y-radio"
        | "atomic-reconcile" => 1,
        "arrow-skip-wrap" | "home-end" | "space-enter-repeat" => 2,
        _ => 0,
    };
    let request_count_ok =
        segmented.requests.lock().expect("segmented requests").len() == expected_requests;
    Ok(action_ok && selection_ok && request_count_ok)
}

fn exercise_feedback_action_lifecycle(
    document: &mut RuntimeDocument,
    viewport: LayoutViewport,
    shaper: &mut NanaTextShaper,
    fixture: Fixture,
    target: StableNodeId,
    action: FeedbackActionFixture,
) -> Result<bool, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let parent_inert = document
        .context()
        .world()
        .interaction(target)
        .is_some_and(|interaction| !interaction.pointer_events && !interaction.focusable);
    let action_bounds = document
        .context()
        .world()
        .layout_box(action.action.stable_id())
        .expect("mounted feedback action layout");
    let adapter = RuntimeInputAdapter::default();
    let action_x = action_bounds.x + action_bounds.width / 2.0;
    let action_y = action_bounds.y + action_bounds.height / 2.0;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Down, action_x, action_y),
    )?;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Up, action_x, action_y),
    )?;
    let first_click_once = *action
        .activations
        .lock()
        .expect("feedback activation count")
        == 1;

    set_feedback_action(
        document,
        fixture.component,
        target,
        Some(action.replacement.stable_id()),
    )?;
    let replacement_update = document.flush(viewport, shaper)?;
    let old_parked_without_ghost = parked_without_ghost(
        document,
        action.action.stable_id(),
        action_bounds,
        &replacement_update.accessibility.removed,
    );
    let replacement_bounds = document
        .context()
        .world()
        .layout_box(action.replacement.stable_id())
        .expect("replacement feedback action layout");

    set_feedback_action(document, fixture.component, target, None)?;
    let removal_update = document.flush(viewport, shaper)?;
    let replacement_parked_without_ghost = parked_without_ghost(
        document,
        action.replacement.stable_id(),
        replacement_bounds,
        &removal_update.accessibility.removed,
    );

    set_feedback_action(
        document,
        fixture.component,
        target,
        Some(action.action.stable_id()),
    )?;
    document.flush(viewport, shaper)?;
    let remounted_bounds = document
        .context()
        .world()
        .layout_box(action.action.stable_id())
        .expect("remounted feedback action layout");
    let remounted_x = remounted_bounds.x + remounted_bounds.width / 2.0;
    let remounted_y = remounted_bounds.y + remounted_bounds.height / 2.0;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Down, remounted_x, remounted_y),
    )?;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Up, remounted_x, remounted_y),
    )?;
    let remount_preserved_handler = *action
        .activations
        .lock()
        .expect("feedback activation count")
        == 2;
    set_feedback_action(document, fixture.component, target, None)?;
    let post_click_removal = document.flush(viewport, shaper)?;
    let focused_action_removed_without_ghost = parked_without_ghost(
        document,
        action.action.stable_id(),
        remounted_bounds,
        &post_click_removal.accessibility.removed,
    );
    set_feedback_action(
        document,
        fixture.component,
        target,
        Some(action.action.stable_id()),
    )?;
    document.flush(viewport, shaper)?;
    let final_parent_inert = document
        .context()
        .world()
        .interaction(target)
        .is_some_and(|interaction| !interaction.pointer_events && !interaction.focusable);
    let final_child_order = document
        .context()
        .world()
        .node(target)
        .is_some_and(|node| node.children == [action.action.stable_id()]);

    Ok(parent_inert
        && first_click_once
        && old_parked_without_ghost
        && replacement_parked_without_ghost
        && remount_preserved_handler
        && focused_action_removed_without_ghost
        && document.context().world().focused(document_id) != Some(action.action.stable_id())
        && final_parent_inert
        && final_child_order)
}

fn set_feedback_action(
    document: &mut RuntimeDocument,
    component: Component,
    target: StableNodeId,
    action: Option<StableNodeId>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let changed = match component {
        Component::EmptyState => document
            .context_mut()
            .set_empty_state_action(Entity::<RuntimeEmptyState>::from_stable_id(target), action)?,
        Component::LabeledValue => document.context_mut().set_labeled_value_action(
            Entity::<RuntimeLabeledValue>::from_stable_id(target),
            action,
        )?,
        _ => false,
    };
    Ok(changed)
}

fn parked_without_ghost(
    document: &RuntimeDocument,
    action: StableNodeId,
    old_bounds: nana_ui::runtime::LayoutBox,
    accessibility_removed: &[StableNodeId],
) -> bool {
    let world = document.context().world();
    world.mount_state(action) == Some(MountState::Parked)
        && !world.document_order(document.document()).contains(&action)
        && !document
            .scene()
            .primitives()
            .any(|primitive| primitive.node == action)
        && world.hit_test(
            document.document(),
            old_bounds.x + old_bounds.width / 2.0,
            old_bounds.y + old_bounds.height / 2.0,
        ) != Some(action)
        && accessibility_removed.contains(&action)
}

fn apply_runtime_state(
    document: &mut RuntimeDocument,
    fixture: Fixture,
    target: StableNodeId,
) -> Result<bool, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let context = document.context_mut();
    let adapter = RuntimeInputAdapter::default();
    let bounds = context.world().layout_box(target).expect("target layout");
    let center_x = bounds.x + bounds.width / 2.0;
    let center_y = bounds.y + bounds.height / 2.0;
    let (drag_x, drag_width, drag_y) = match context.world().component_geometry(target) {
        Some(nana_ui::runtime::ComponentGeometry::Range { track, .. }) => {
            (track.x, track.width, track.y + track.height / 2.0)
        }
        _ => (bounds.x, bounds.width, center_y),
    };
    match fixture.state {
        "hover" | "selected-hover" => Ok(adapter
            .dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
            )?
            .prevent_default),
        "open" if !matches!(fixture.component, Component::Tooltip) => Ok(true),
        "tooltip-delay" | "tooltip-edge" | "open" | "edge" => {
            adapter.dispatch_at(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
                Duration::ZERO,
            )?;
            let deadline = context.next_animation_deadline();
            if let Some(deadline) = deadline {
                context.advance_animations(deadline);
            }
            Ok(if matches!(fixture.state, "open" | "edge") {
                context
                    .world()
                    .overlay_host(target)
                    .is_some_and(|host| host.active.is_some())
            } else {
                deadline.is_some()
            })
        }
        "delay" if fixture.component == Component::Tooltip => {
            adapter.dispatch_at(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
                Duration::ZERO,
            )?;
            Ok(context.next_animation_deadline().is_some()
                && context
                    .world()
                    .overlay_host(target)
                    .is_some_and(|host| host.active.is_none()))
        }
        "pressed" | "selected-pressed" => Ok(adapter
            .dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, center_x, center_y),
            )?
            .prevent_default),
        "focused" => {
            let target = if fixture.component == Component::Tabs {
                context
                    .world()
                    .node(target)
                    .and_then(|node| node.children.first().copied())
                    .unwrap_or(target)
            } else {
                target
            };
            Ok(context.focus_node(document_id, target)?)
        }
        "invalid" if fixture.component == Component::TextInput => {
            Ok(context.focus_node(document_id, target)?)
        }
        "invalid-focused" if fixture.component == Component::Textarea => {
            Ok(context.focus_node(document_id, target)?)
        }
        "selection" => {
            context.focus_node(document_id, target)?;
            Ok(context.apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetSelection(TextSelection {
                        anchor: 0,
                        focus: "release".len(),
                    }),
                },
            )?)
        }
        "multiline-selection" if fixture.component == Component::Textarea => {
            context.focus_node(document_id, target)?;
            Ok(context.apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetSelection(TextSelection {
                        anchor: "First ".len(),
                        focus: "First line\nSecond line\nThird".len(),
                    }),
                },
            )?)
        }
        "scroll" if fixture.component == Component::Textarea => {
            let end = context
                .world()
                .text_input(target)
                .expect("textarea input state")
                .value
                .len();
            let focused = context.focus_node(document_id, target)?;
            let selected = context.apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetSelection(TextSelection::caret(end)),
                },
            )?;
            Ok(focused || selected)
        }
        "keyboard-activation" | "space-toggle" => {
            context.focus_node(document_id, target)?;
            Ok(adapter
                .dispatch(context, document_id, &keyboard("Space"))?
                .prevent_default)
        }
        "pointer-activation" | "pointer-toggle" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, center_x, center_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Up, center_x, center_y),
                )?
                .prevent_default)
        }
        "accessibility-toggle" => Ok(context.apply_accessibility_action(
            document_id,
            AccessibilityActionRequest {
                target,
                action: AccessibilityAction::Click,
            },
        )?),
        "keyboard-edit" => {
            context.focus_node(document_id, target)?;
            Ok(adapter
                .dispatch(context, document_id, &keyboard_text("X"))?
                .prevent_default)
        }
        "ime-preedit" => {
            context.focus_node(document_id, target)?;
            Ok(context.set_ime_preedit(document_id, "你".into(), None)?)
        }
        "ime-commit" => {
            context.focus_node(document_id, target)?;
            context.set_ime_preedit(document_id, "你".into(), None)?;
            Ok(context.commit_ime(document_id, "你")?)
        }
        "drag" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, drag_x + drag_width * 0.25, drag_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Move, drag_x + drag_width * 0.8, drag_y),
                )?
                .prevent_default)
        }
        "drag-cancel" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, drag_x + drag_width * 0.25, drag_y),
            )?;
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Move, drag_x + drag_width * 0.8, drag_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Cancel, drag_x + drag_width * 0.8, drag_y),
                )?
                .prevent_default)
        }
        "arrow-decrement" => dispatch_range_key(context, document_id, target, adapter, "ArrowLeft"),
        "arrow-increment" => {
            dispatch_range_key(context, document_id, target, adapter, "ArrowRight")
        }
        "page-decrement" => dispatch_range_key(context, document_id, target, adapter, "PageDown"),
        "page-increment" => dispatch_range_key(context, document_id, target, adapter, "PageUp"),
        "home" => dispatch_range_key(context, document_id, target, adapter, "Home"),
        "end" => dispatch_range_key(context, document_id, target, adapter, "End"),
        "accessibility-set-value" => Ok(context.apply_accessibility_action(
            document_id,
            AccessibilityActionRequest {
                target,
                action: AccessibilityAction::SetValue(
                    if fixture.component == Component::TextInput {
                        "updated"
                    } else {
                        "0.73"
                    }
                    .into(),
                ),
            },
        )?),
        _ => Ok(false),
    }
}

fn dispatch_range_key(
    context: &mut nana_ui::runtime::AppContext,
    document: DocumentId,
    target: StableNodeId,
    adapter: RuntimeInputAdapter,
    key: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    context.focus_node(document, target)?;
    Ok(adapter
        .dispatch(context, document, &keyboard(key))?
        .prevent_default)
}

fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
    InputEvent::Pointer {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x,
        y,
        screen_x: x,
        screen_y: y,
        button: 0,
        buttons: u16::from(phase == PointerPhase::Down),
        pressure: 0.0,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        modifiers: InputModifiers::default(),
    }
}

fn keyboard(key: &str) -> InputEvent {
    keyboard_with_repeat(key, false)
}

fn keyboard_with_repeat(key: &str, repeat: bool) -> InputEvent {
    InputEvent::Keyboard {
        pressed: true,
        key: key.into(),
        text: None,
        code: key.into(),
        repeat,
        modifiers: InputModifiers::default(),
    }
}

fn keyboard_text(text: &str) -> InputEvent {
    InputEvent::Keyboard {
        pressed: true,
        key: text.into(),
        text: Some(text.into()),
        code: "KeyX".into(),
        repeat: false,
        modifiers: InputModifiers::default(),
    }
}

fn write_evidence(
    path: &Path,
    fixture: Fixture,
    runtime: &RuntimeEvidence,
) -> Result<(), Box<dyn std::error::Error>> {
    let world = runtime.document.context().world();
    let bounds = world.layout_box(runtime.target);
    let hit = bounds.and_then(|bounds| {
        world.hit_test(
            runtime.document.document(),
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    });
    let accessibility = world.accessibility(runtime.target);
    let text_input = world.text_input(runtime.target);
    let geometry = world.component_geometry(runtime.target);
    let primitives = runtime.document.scene().primitives().collect::<Vec<_>>();
    let primitive = |slot| {
        primitives
            .iter()
            .copied()
            .find(|primitive| primitive.node == runtime.target && primitive.id.slot == slot)
    };
    let has_own_clip = |primitive: &nana_ui_scene::ScenePrimitive| {
        bounds.is_some_and(|bounds| {
            primitive.clips.iter().any(|clip| {
                (clip.bounds.x - bounds.x).abs() < 0.01
                    && (clip.bounds.y - bounds.y).abs() < 0.01
                    && (clip.bounds.width - bounds.width).abs() < 0.01
                    && (clip.bounds.height - bounds.height).abs() < 0.01
            })
        })
    };
    let has_clip = |primitive: &nana_ui_scene::ScenePrimitive,
                    clip: nana_ui::runtime::LayoutBox| {
        primitive.clips.iter().any(|candidate| {
            (candidate.bounds.x - clip.x).abs() < 0.01
                && (candidate.bounds.y - clip.y).abs() < 0.01
                && (candidate.bounds.width - clip.width).abs() < 0.01
                && (candidate.bounds.height - clip.height).abs() < 0.01
        })
    };
    let text_scene_ok = match fixture.component {
        Component::Textarea => primitive(2).is_some_and(|primitive| {
            has_own_clip(primitive)
                && matches!(
                    primitive.kind,
                    ScenePrimitiveKind::Text {
                        wrap: true,
                        vertical_alignment: TextVerticalAlignment::Top,
                        ..
                    }
                )
        }),
        Component::TextInput => primitive(2).is_some_and(|primitive| {
            matches!(
                primitive.kind,
                ScenePrimitiveKind::Text {
                    wrap: false,
                    vertical_alignment: TextVerticalAlignment::Center,
                    ..
                }
            )
        }),
        _ => true,
    };
    let textarea_geometry_ok = if fixture.component != Component::Textarea {
        true
    } else {
        match (bounds, geometry.as_ref()) {
            (
                Some(bounds),
                Some(nana_ui::runtime::ComponentGeometry::TextInput {
                    text,
                    multiline,
                    selection,
                    caret,
                    preedit,
                    focus_ring,
                    border,
                    border_width,
                    ..
                }),
            ) => {
                let selection_scene_ok = if selection.is_empty() {
                    primitive(1).is_none()
                } else {
                    primitive(1).is_some_and(|primitive| {
                        has_own_clip(primitive)
                            && matches!(
                                &primitive.kind,
                                ScenePrimitiveKind::QuadBatch {
                                    bounds: quads,
                                    ..
                                } if quads.len() == selection.len()
                            )
                    })
                };
                let focused = textarea_is_focused(fixture.state);
                let caret_scene_ok = if focused {
                    caret.is_some()
                        && primitive(4).is_some_and(|primitive| {
                            has_own_clip(primitive)
                                && matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
                        })
                        && focus_ring.is_none()
                        && primitive(7).is_none()
                } else {
                    caret.is_none() && primitive(4).is_none() && focus_ring.is_none()
                };
                let border_ok = match fixture.state {
                    "invalid-focused" => {
                        focus_ring.is_none() && border.is_some() && *border_width >= 2.0
                    }
                    "disabled" => focus_ring.is_none(),
                    _ => focus_ring.is_none() && (*border_width - 1.0).abs() < 0.01,
                };
                let selection_count_ok = match fixture.state {
                    "selection" => selection.len() == 1,
                    "multiline-selection" => selection.len() >= 2,
                    _ => selection.is_empty(),
                };
                let text_content_ok = if fixture.state == "placeholder" {
                    text.content.as_ref() == "Describe the issue"
                } else {
                    text.content.as_ref() == textarea_value(fixture.state)
                };
                let clip = primitive(2).and_then(|primitive| primitive.clips.last());
                let clipped_ok = match fixture.state {
                    "clipped" => clip.is_some_and(|clip| {
                        text.bounds.y + text.bounds.height
                            > clip.bounds.y + clip.bounds.height + 0.01
                    }),
                    "scroll" => clip.is_some_and(|clip| {
                        text.bounds.height + 0.01 >= clip.bounds.height
                            && text.bounds.y < clip.bounds.y
                    }),
                    _ => true,
                };
                let scroll_ok = if fixture.state == "scroll" {
                    clip.is_some_and(|clip| {
                        text.bounds.y < clip.bounds.y
                            && caret.is_some_and(|caret| {
                                caret.y >= clip.bounds.y
                                    && caret.y + caret.height
                                        <= clip.bounds.y + clip.bounds.height + 0.01
                            })
                    })
                } else {
                    true
                };
                *multiline
                    && text.bounds.x >= bounds.x
                    && text.bounds.width > 0.0
                    && text.bounds.width <= bounds.width + 0.01
                    && text.bounds.height > 0.0
                    && text_content_ok
                    && selection_count_ok
                    && selection_scene_ok
                    && caret_scene_ok
                    && border_ok
                    && preedit.is_empty()
                    && primitive(5).is_none()
                    && clipped_ok
                    && scroll_ok
            }
            _ => false,
        }
    };
    let mut segmented_geometry_ok = true;
    let mut segmented_accessibility_ok = true;
    if fixture.component == Component::SegmentedControl {
        let expected_option_height =
            (segmented_control_size(fixture.state).height() - 6.0).max(0.0);
        segmented_accessibility_ok = accessibility
            .is_some_and(|node| node.role == nana_ui::runtime::AccessibilityRole::RadioGroup);
        let control = Entity::<RuntimeSegmentedControl>::from_stable_id(runtime.target);
        let selected = runtime
            .document
            .context()
            .read(control, RuntimeSegmentedControl::selected)?;
        let focus_target = runtime
            .document
            .context()
            .read(control, RuntimeSegmentedControl::focus_target)?;
        let mounted_options = runtime
            .segmented_options
            .iter()
            .copied()
            .filter(|id| world.mount_state(*id) == Some(MountState::Mounted))
            .collect::<Vec<_>>();
        let mut checked = 0;
        let mut enabled = Vec::new();
        for id in &mounted_options {
            let option = Entity::<RuntimeSegmentedOption>::from_stable_id(*id);
            let option_selected = runtime
                .document
                .context()
                .read(option, RuntimeSegmentedOption::selected)?;
            let disabled = runtime
                .document
                .context()
                .read(option, RuntimeSegmentedOption::disabled_value)?;
            checked += usize::from(option_selected);
            if !disabled {
                enabled.push(*id);
            }
            segmented_accessibility_ok &= world.accessibility(*id).is_some_and(|node| {
                node.role == nana_ui::runtime::AccessibilityRole::Radio
                    && node.checked == Some(option_selected)
                    && node.disabled == disabled
            });
            let option_bounds = world.layout_box(*id);
            let option_geometry = world.component_geometry(*id);
            let option_surface = primitives
                .iter()
                .find(|primitive| primitive.node == *id && primitive.id.slot == 0);
            let option_text = primitives
                .iter()
                .find(|primitive| primitive.node == *id && primitive.id.slot == 2);
            segmented_geometry_ok &= matches!(
                (option_bounds, option_geometry),
                (
                    Some(bounds),
                    Some(nana_ui::runtime::ComponentGeometry::SelectionOption { label, .. })
                ) if bounds.height > 0.0
                    && (bounds.height - expected_option_height).abs() < 0.01
                    && label.bounds.x >= bounds.x
                    && label.bounds.x + label.bounds.width <= bounds.x + bounds.width + 0.01
            ) && option_surface
                .is_some_and(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Quad { .. }))
                && option_text.is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
                });
        }
        let expected_width = mounted_options
            .iter()
            .filter_map(|id| world.layout_box(*id))
            .map(|bounds| bounds.width)
            .sum::<f32>()
            + mounted_options.len().saturating_sub(1) as f32 * 2.0
            + 6.0;
        segmented_geometry_ok &=
            bounds.is_some_and(|bounds| (bounds.width - expected_width).abs() < 0.01);
        let expected_checked = usize::from(selected.is_some());
        let tab_stop_ok = match focus_target {
            Some(id) => enabled.contains(&id),
            None => enabled.is_empty(),
        };
        segmented_accessibility_ok &= checked == expected_checked && tab_stop_ok;
        if fixture.state == "medium-icon" {
            segmented_geometry_ok &= mounted_options.first().is_some_and(|id| {
                primitives.iter().any(|primitive| {
                    primitive.node == *id
                        && primitive.id.slot == 3
                        && matches!(primitive.kind, ScenePrimitiveKind::Icon { .. })
                })
            });
        }
        if fixture.state == "focused" {
            segmented_geometry_ok &= focus_target.is_some_and(|id| {
                let Some(bounds) = world.layout_box(id) else {
                    return false;
                };
                primitives.iter().any(|primitive| {
                    primitive.node == id
                        && primitive.id.slot == 7
                        && (primitive.bounds.x - (bounds.x - 4.0)).abs() < 0.01
                        && (primitive.bounds.y - (bounds.y - 4.0)).abs() < 0.01
                        && (primitive.bounds.width - (bounds.width + 8.0)).abs() < 0.01
                        && matches!(
                            primitive.kind,
                            ScenePrimitiveKind::Quad {
                                border_width,
                                ..
                            } if (border_width - 2.0).abs() < 0.01
                        )
                })
            });
        }
    }
    let feedback_parent_inert = !matches!(
        fixture.component,
        Component::StatusBadge
            | Component::ValidationMessage
            | Component::EmptyState
            | Component::LabeledValue
    ) || world
        .interaction(runtime.target)
        .is_some_and(|interaction| !interaction.pointer_events && !interaction.focusable);
    let feedback_accessibility_ok = match fixture.component {
        Component::StatusBadge => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some(status_badge_label(fixture.state))
                && !node.invalid
                && !node.disabled
        }),
        Component::ValidationMessage => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some(validation_message(fixture.state)) && node.invalid
        }),
        Component::EmptyState => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some(empty_title(fixture.state))
                && node.value.as_deref()
                    == (fixture.state != "title-only").then(|| empty_message(fixture.state))
        }),
        Component::LabeledValue => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some("Revision") && node.value.as_deref() == Some("42")
        }),
        _ => true,
    };
    let feedback_geometry_ok = match geometry.as_ref() {
        Some(nana_ui::runtime::ComponentGeometry::StatusBadge {
            indicator, label, ..
        }) if fixture.component == Component::StatusBadge => {
            label.content.as_ref() == status_badge_label(fixture.state)
                && (label.font_size - 11.0).abs() < 0.01
                && label.font_weight == Some(500)
                && label.bounds.x - (indicator.x + indicator.width) >= 4.9
                && primitive(0).is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
                })
                && primitive(2).is_some_and(|primitive| {
                    matches!(
                        &primitive.kind,
                        ScenePrimitiveKind::Text {
                            size,
                            weight: Some(500),
                            ..
                        } if (*size - 11.0).abs() < 0.01
                    )
                })
                && primitive(3).is_some_and(|primitive| {
                    matches!(
                        primitive.kind,
                        ScenePrimitiveKind::Quad {
                            background: Some(_),
                            border_width,
                            ..
                        } if border_width == 0.0
                    )
                })
        }
        Some(nana_ui::runtime::ComponentGeometry::ValidationMessage {
            indicator, label, ..
        }) if fixture.component == Component::ValidationMessage => {
            label.content.as_ref() == validation_message(fixture.state)
                && (label.font_size - 11.0).abs() < 0.01
                && label.font_weight.is_none()
                && label.bounds.x - (indicator.x + indicator.width) >= 4.9
                && primitive(2).is_some_and(|primitive| {
                    matches!(
                        &primitive.kind,
                        ScenePrimitiveKind::Text {
                            size,
                            weight: None,
                            ..
                        } if (*size - 11.0).abs() < 0.01
                    )
                })
                && primitive(3).is_some_and(|primitive| {
                    matches!(
                        primitive.kind,
                        ScenePrimitiveKind::Quad {
                            background: None,
                            border_color: Some(_),
                            border_width,
                            ..
                        } if (border_width - 1.0).abs() < 0.01
                    )
                })
        }
        Some(nana_ui::runtime::ComponentGeometry::EmptyState {
            root_clip,
            content_clip,
            icon,
            title,
            message,
            action,
        }) if fixture.component == Component::EmptyState => {
            let expects_content = fixture.state != "title-only";
            let intrinsic_scene_clipped = [2_u8, 3, 4].into_iter().all(|slot| {
                primitive(slot).is_none_or(|primitive| has_clip(primitive, *content_clip))
            });
            let action_scene_clipped = world
                .node(runtime.target)
                .and_then(|node| node.children.first().copied())
                .is_none_or(|action| {
                    primitives
                        .iter()
                        .filter(|primitive| primitive.node == action)
                        .all(|primitive| has_clip(primitive, *root_clip))
                });
            let ordering_ok = icon
                .as_ref()
                .is_none_or(|(_, icon, _)| icon.y + icon.height <= title.bounds.y + 0.01)
                && message.as_ref().is_none_or(|message| {
                    title.bounds.y + title.bounds.height <= message.bounds.y + 0.01
                        && action.is_none_or(|action| {
                            message.bounds.y + message.bounds.height <= action.y + 0.01
                        })
                });
            let alignment_ok = if fixture.state == "compact" {
                (title.bounds.x - content_clip.x).abs() < 0.01
            } else {
                let title_center = title.bounds.x + title.bounds.width / 2.0;
                let clip_center = content_clip.x + content_clip.width / 2.0;
                (title_center - clip_center).abs() < 0.01
            };
            let wrap_ok = !matches!(fixture.state, "narrow-cjk" | "extreme-clip")
                || title.bounds.height > title.font_size * 1.2
                || message
                    .as_ref()
                    .is_some_and(|message| message.bounds.height > message.font_size * 1.2);
            let extreme_clip_ok = fixture.state != "extreme-clip"
                || message.as_ref().is_some_and(|message| {
                    message.bounds.y + message.bounds.height > content_clip.y + content_clip.height
                });
            title.content.as_ref() == empty_title(fixture.state)
                && (title.font_size
                    - if fixture.state == "compact" {
                        12.0
                    } else {
                        13.0
                    })
                .abs()
                    < 0.01
                && title.font_weight == Some(600)
                && (icon.is_some() == expects_content)
                && (message.is_some() == expects_content)
                && (action.is_some() == (fixture.state == "complete-action"))
                && intrinsic_scene_clipped
                && action_scene_clipped
                && ordering_ok
                && alignment_ok
                && wrap_ok
                && extreme_clip_ok
        }
        Some(nana_ui::runtime::ComponentGeometry::LabeledValue {
            label,
            value,
            action,
        }) if fixture.component == Component::LabeledValue => {
            let expected_weight = if fixture.state == "strong" { 600 } else { 500 };
            label.content.as_ref() == "Revision"
                && value.content.as_ref() == "42"
                && (label.font_size - 11.0).abs() < 0.01
                && (value.font_size - 12.0).abs() < 0.01
                && value.font_weight == Some(expected_weight)
                && label.bounds.y + label.bounds.height <= value.bounds.y + 0.01
                && (action.is_some() == (fixture.state == "action"))
                && primitive(2).is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Text { size, .. } if (size - 11.0).abs() < 0.01)
                })
                && primitive(3).is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Text { size, weight: Some(weight), .. } if (size - 12.0).abs() < 0.01 && weight == expected_weight)
                })
        }
        _ => !matches!(
            fixture.component,
            Component::StatusBadge
                | Component::ValidationMessage
                | Component::EmptyState
                | Component::LabeledValue
        ),
    };
    let tooltip = matches!(
        fixture.component,
        Component::IconButton | Component::Tooltip
    )
    .then(|| {
        runtime
            .document
            .context()
            .icon_button_tooltip(Entity::<RuntimeIconButton>::from_stable_id(runtime.target))
            .ok()
            .flatten()
            .map(|tooltip| tooltip.stable_id())
    })
    .flatten();
    let active_overlay = world
        .overlay_host(runtime.target)
        .and_then(|host| host.active);
    let expects_hit =
        !matches!(fixture.state, "disabled" | "loading") && fixture.component != Component::Text;
    let hit_ok = if fixture.component == Component::SegmentedControl {
        if matches!(fixture.state, "empty" | "all-disabled") {
            hit.is_none()
        } else {
            runtime.segmented_options.iter().copied().any(|id| {
                world
                    .interaction(id)
                    .is_some_and(|interaction| interaction.pointer_events)
                    && world.layout_box(id).is_some_and(|bounds| {
                        world.hit_test(
                            runtime.document.document(),
                            bounds.x + bounds.width / 2.0,
                            bounds.y + bounds.height / 2.0,
                        ) == Some(id)
                    })
            })
        }
    } else if matches!(
        fixture.component,
        Component::Card
            | Component::Text
            | Component::StatusBadge
            | Component::ValidationMessage
            | Component::EmptyState
            | Component::LabeledValue
            | Component::Progress
            | Component::Spinner
            | Component::Skeleton
            | Component::LevelMeter
            | Component::FormField
            | Component::Workspace
            | Component::Dock
            | Component::DockPanel
            | Component::SplitPane
            | Component::PaneChrome
            | Component::PaneTree
            | Component::AppShell
            | Component::DesktopShell
            | Component::SettingsSidebar
            | Component::SettingsPage
            | Component::AppTitleBar
            | Component::GpuTextureView
            | Component::Thumbnail
    ) {
        hit != Some(runtime.target)
    } else if expects_hit {
        hit == Some(runtime.target)
    } else {
        hit != Some(runtime.target)
    };
    let action_state = (fixture.component == Component::TextInput && fixture.state == "invalid")
        || (fixture.component == Component::Textarea
            && matches!(
                fixture.state,
                "invalid-focused" | "multiline-selection" | "scroll"
            ))
        || matches!(
            fixture.state,
            "hover"
                | "pressed"
                | "selected-hover"
                | "selected-pressed"
                | "focused"
                | "selection"
                | "keyboard-activation"
                | "tooltip-delay"
                | "tooltip-edge"
                | "open"
                | "delay"
                | "edge"
                | "pointer-toggle"
                | "space-toggle"
                | "accessibility-toggle"
                | "pointer-activation"
                | "drag"
                | "drag-cancel"
                | "arrow-decrement"
                | "arrow-increment"
                | "page-decrement"
                | "page-increment"
                | "home"
                | "end"
                | "accessibility-set-value"
                | "keyboard-edit"
                | "ime-preedit"
                | "ime-commit"
        );
    let geometry_ok = matches!(
        fixture.component,
        Component::Text
            | Component::Button
            | Component::TextInput
            | Component::Textarea
            | Component::HostedTextarea
            | Component::Checkbox
            | Component::IconButton
            | Component::Tooltip
            | Component::SegmentedControl
            | Component::Tabs
            | Component::Spinner
            | Component::Skeleton
            | Component::Workspace
            | Component::Dock
            | Component::DockPanel
            | Component::SplitPane
            | Component::PaneChrome
            | Component::PaneTree
            | Component::AppShell
            | Component::DesktopShell
            | Component::SettingsSidebar
            | Component::SettingsPage
            | Component::AppTitleBar
            | Component::GpuTextureView
            | Component::GpuView
            | Component::Thumbnail
    ) || geometry.is_some();
    let layout_ok = bounds.is_some_and(|bounds| match fixture.component {
        Component::Text if matches!(fixture.state, "wrap" | "ellipsis") => {
            (bounds.width - 180.0).abs() < 0.01
        }
        Component::Text => bounds.width > 0.0 && bounds.height >= 32.0,
        Component::Button => {
            let expected = button_control_size(fixture.state).height();
            (bounds.height - expected).abs() < 0.01
        }
        Component::TextInput => {
            (bounds.width - 380.0).abs() < 0.01
                && (bounds.height - text_input_control_size(fixture.state).height()).abs() < 0.01
        }
        Component::Textarea | Component::HostedTextarea => {
            (bounds.width - 380.0).abs() < 0.01 && (bounds.height - 96.0).abs() < 0.01
        }
        Component::SegmentedControl => {
            (bounds.height - segmented_control_size(fixture.state).height()).abs() < 0.01
        }
        Component::Checkbox => bounds.height >= ControlSize::Medium.height(),
        Component::Thumbnail if fixture.state == "wide" => {
            let height = ControlSize::Small.height();
            let width = height * 16.0 / 9.0;
            (bounds.height - height).abs() < 0.01 && (bounds.width - width).abs() < 0.01
        }
        Component::Thumbnail => {
            let extent = ControlSize::Small.height();
            (bounds.width - extent).abs() < 0.01 && (bounds.height - extent).abs() < 0.01
        }
        Component::Dialog | Component::ConfirmDialog | Component::Drawer => {
            matches!(
                geometry,
                Some(nana_ui::runtime::ComponentGeometry::ModalFrame { .. })
            )
        }
        _ => true,
    });
    let runtime_ok = bounds.is_some()
        && accessibility.is_some()
        && geometry_ok
        && layout_ok
        && text_scene_ok
        && textarea_geometry_ok
        && segmented_geometry_ok
        && segmented_accessibility_ok
        && feedback_parent_inert
        && feedback_accessibility_ok
        && feedback_geometry_ok
        && runtime.feedback_contract_ok
        && runtime.segmented_contract_ok
        && runtime.idle
        && hit_ok
        && (!action_state || runtime.action_applied)
        && (fixture.state != "loading"
            || fixture.component == Component::TextInput
            || runtime.next_deadline.is_some())
        && (fixture.component != Component::TextInput
            || match fixture.state {
                "read-only" => accessibility.is_some_and(|node| !node.editable && !node.disabled),
                "loading" => accessibility.is_some_and(|node| node.busy && node.disabled),
                "secure" => accessibility.is_some_and(|node| node.value.is_none()),
                "selection" => matches!(
                    geometry,
                    Some(nana_ui::runtime::ComponentGeometry::TextInput {
                        ref selection,
                        ..
                    }) if !selection.is_empty()
                ),
                _ => true,
            })
        && (fixture.component != Component::Textarea
            || (accessibility.is_some_and(|node| node.multiline)
                && match fixture.state {
                    "focused" => world.focused(runtime.document.document()) == Some(runtime.target),
                    "invalid-focused" => {
                        accessibility.is_some_and(|node| node.invalid)
                            && world.focused(runtime.document.document()) == Some(runtime.target)
                    }
                    "disabled" => {
                        accessibility.is_some_and(|node| node.disabled)
                            && world.focused(runtime.document.document()) != Some(runtime.target)
                    }
                    state if textarea_is_focused(state) => {
                        world.focused(runtime.document.document()) == Some(runtime.target)
                    }
                    _ => true,
                }))
        && match (fixture.component, fixture.state) {
            (Component::Tooltip, "delay") => {
                tooltip.is_some()
                    && active_overlay.is_none()
                    && runtime.next_deadline.is_some()
                    && tooltip.is_some_and(|id| {
                        world.accessibility(id).is_some_and(|node| {
                            node.role == nana_ui::runtime::AccessibilityRole::Tooltip
                                && node.label.as_deref() == Some("Add source")
                        })
                    })
            }
            (Component::Tooltip, "open" | "edge") | (_, "tooltip-delay" | "tooltip-edge") => {
                tooltip.is_some()
                    && tooltip == active_overlay
                    && tooltip.is_some_and(|id| {
                        world.accessibility(id).is_some_and(|node| {
                            node.role == nana_ui::runtime::AccessibilityRole::Tooltip
                                && node.label.as_deref() == Some("Add source")
                        })
                    })
            }
            _ => true,
        };
    let reference_verdict =
        if fixture.component == Component::Textarea && textarea_is_focused(fixture.state) {
            "deterministic compatibility content and focus state rendered for manual review"
        } else if matches!(fixture.state, "control-start" | "disabled" | "invalid")
            && fixture.component == Component::RangeField
        {
            "compatibility defect: archived reference does not expose this product contract"
        } else if matches!(
            fixture.state,
            "pressed"
                | "selected-pressed"
                | "focused"
                | "keyboard-activation"
                | "space-toggle"
                | "accessibility-toggle"
                | "pointer-activation"
        ) {
            "reference only: headless fixture does not claim retained interaction evidence"
        } else {
            "rendered reference; visual judgment remains manual"
        };
    let machine_verdict = if runtime_ok { "pass" } else { "fail" };
    let (review_verdict, review_observed) = review_result(fixture);
    let divergence = intentional_divergence(fixture);
    let report = format!(
        "expected: {}\nreference_observed: {}\nreference_verdict: {}\nruntime_expected: {}\nruntime_observed: bounds={bounds:?}; layout_ok={layout_ok}; text_scene_ok={text_scene_ok}; textarea_geometry_ok={textarea_geometry_ok}; segmented_geometry_ok={segmented_geometry_ok}; segmented_accessibility_ok={segmented_accessibility_ok}; segmented_contract_ok={}; segmented_options={:?}; segmented_requests={}; feedback_parent_inert={feedback_parent_inert}; feedback_accessibility_ok={feedback_accessibility_ok}; feedback_geometry_ok={feedback_geometry_ok}; feedback_contract_ok={}; text_input={text_input:?}; geometry={geometry:?}; hit={hit:?}; accessibility={accessibility:?}; tooltip={tooltip:?}; active_overlay={active_overlay:?}; first_passes={}; first_accessibility_updates={}; final_passes={}; final_accessibility_updates={}; second_flush_idle={}; action_applied={}; next_animation_deadline={:?}; primitives={primitives:?}\nmachine_verdict: {}\nreview_observed: {}\nreview_verdict: {}\nintentional_divergence_reason: {}\n",
        fixture.expected,
        fixture.reference_contract,
        reference_verdict,
        fixture.runtime_contract,
        runtime.segmented_contract_ok,
        runtime.segmented_options,
        runtime.segmented_requests,
        runtime.feedback_contract_ok,
        runtime.first_passes,
        runtime.first_accessibility_updates,
        runtime.final_passes,
        runtime.final_accessibility_updates,
        runtime.idle,
        runtime.action_applied,
        runtime.next_deadline,
        machine_verdict,
        review_observed,
        review_verdict,
        divergence,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)?;
    Ok(())
}

fn review_result(fixture: Fixture) -> (&'static str, &'static str) {
    match (fixture.component, fixture.state) {
        (Component::Button, _) => (
            "pass",
            "Fresh isolated dark and light review confirms semantic kinds, three control sizes, hover and pressed backgrounds without an accent focus ring, disabled and loading presentation, complete label geometry, hit behavior and accessibility",
        ),
        (Component::TextInput, _) => (
            "pass",
            "Fresh isolated dark and light review confirms placeholder contrast, shaped selection and caret geometry, external focus, invalid, secure, size, read-only, loading, keyboard and IME preedit or commit presentation",
        ),
        (Component::Textarea, _) => (
            "manual-required",
            "Review the generated dark and light compatibility and Runtime images for placeholder, multiline, focus, selection, invalid, disabled, clipping and scrolling semantics; IME remains a real Hosted gate",
        ),
        (Component::HostedTextarea, _) => (
            "manual-required",
            "Review the generated dark and light images for Runtime presenter spans on committed rust text; Iced highlighter is a leftover reference, not the product path",
        ),
        (
            Component::CalendarHeatmap
            | Component::TimeSeriesChart
            | Component::ReorderList
            | Component::NativeMarkdown
            | Component::SelectableRichText
            | Component::ImageViewer
            | Component::GraphCanvas
            | Component::KeyCaptureLayer
            | Component::KeymapLayer,
            _,
        ) => (
            "manual-required",
            "Review Runtime Scene quads against design tokens; Iced canvas/widget output is a reference, not a pixel oracle",
        ),
        (Component::GpuTextureView | Component::GpuView, _) => (
            "manual-required",
            "Review host-texture sampling and gpu-view Scene paint; Iced shader output is a reference, not a pixel oracle",
        ),
        (Component::Thumbnail, _) => (
            "manual-required",
            "Review the compact list-row box, four shared-geometry states, and ready host-texture contain",
        ),
        (Component::Tooltip, _) => (
            "manual-required",
            "Review the generated dark and light Iced Tooltip and Runtime overlay images for open, delay-not-open and edge placement; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (Component::Dialog | Component::ConfirmDialog | Component::Drawer, _) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime modal images for scrim, surface, title and slotted body or actions; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (Component::Toast | Component::XYPad | Component::QrCode, _) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime images; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (
            Component::Select
            | Component::Popover
            | Component::ActionMenu
            | Component::ActionMenuItem
            | Component::AnchoredActionMenu
            | Component::ContextMenu,
            _,
        ) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime images for select fields and anchored menus; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (Component::SettingsSidebar | Component::SettingsPage | Component::DesktopShell, _) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime images; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (
            Component::Workspace
            | Component::Dock
            | Component::DockPanel
            | Component::SplitPane
            | Component::PaneChrome
            | Component::PaneTree
            | Component::AppShell
            | Component::AppTitleBar,
            _,
        ) => (
            "pass",
            "2026-08-16 windowed A/B and side-by-side review preferred Runtime (right): workspace-family is the accepted public default, fixture slot labels keep the shared 8px inset that Workspace/Dock/Split/AppShell chrome does not add, and Iced images remain a migration-era reference, not an oracle",
        ),
        (
            Component::SidebarFrame
            | Component::SidebarSection
            | Component::SidebarFooter
            | Component::AppearanceSection
            | Component::AboutSection
            | Component::SettingsCollapsibleCard,
            _,
        ) => (
            "pass",
            "2026-08-16 windowed A/B and side-by-side review preferred Runtime (right): frame top/footer stay outside the scrolling body, section collapse uses ChevronRight, footer hugs 28px icon actions, Appearance/About assemble SettingsRow children, and Iced uppercase title / missing radius track are Iced-side",
        ),
        (Component::SegmentedControl, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right) over Iced (left) for density, selected pill, icon alignment and disabled fade; selected surface is the only focus cue",
        ),
        (Component::Text, _) => (
            "pass",
            "Runtime text uses the authored content box, shared typography, semantic contrast, wrapping or ellipsis, alignment, clipping and accessibility in dark and light",
        ),
        (Component::Checkbox, _) => (
            "pass",
            "Runtime checkbox keeps indicator and label geometry, semantic checked and invalid paint, complete-row hit testing, focus, disabled behavior and accessibility in dark and light",
        ),
        (Component::IconButton, "hover" | "pressed" | "focused" | "selected") => (
            "pass",
            "Runtime uses distinct neutral hover and pressed layers without an accent focus ring, and a persistent accent-selected treatment while preserving icon contrast in dark and light",
        ),
        (Component::Switch, "hover" | "pressed" | "focused") => (
            "pass",
            "Runtime separates the complete-row hover and pressed layers from the track focus ring, so each interaction state remains visible and distinct in dark and light",
        ),
        (Component::StatusBadge, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): five tones, compact pill and indicator contrast are accepted without Iced pixel match",
        ),
        (Component::ValidationMessage, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): warning and danger contrast and inline spacing are accepted",
        ),
        (Component::EmptyState, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): icon/title/message order, compact layout, CJK wrap and solid Primary action are accepted",
        ),
        (Component::LabeledValue, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): label/value hierarchy and end-aligned action child are accepted",
        ),
        (Component::Progress | Component::Spinner, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): determinate track/fill, optional label and host-sampled spinner are accepted",
        ),
        (Component::Tabs, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): independent tab surface without a segmented focus ring",
        ),
        (Component::Skeleton | Component::LevelMeter, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): Subtle placeholder and tone-colored meter are accepted",
        ),
        (Component::FormField, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): enabled field, centered value and danger support with indicator",
        ),
        (Component::InteractiveCard, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): selected surface and centered child content",
        ),
        _ => (
            "pass",
            "Runtime text, internal geometry, contrast, clipping and state are correct in dark and light review; pixel similarity was not used as a gate",
        ),
    }
}

fn intentional_divergence(fixture: Fixture) -> &'static str {
    match (fixture.component, fixture.state) {
        (Component::Switch, "control-start") => {
            "intentional: Runtime implements the start-side control contract missing from the Iced adapter"
        }
        (Component::RangeField, "disabled" | "invalid" | "decimal-step") => {
            "intentional: Runtime implements the design contract missing from the Iced adapter"
        }
        (Component::RangeField, _) => {
            "intentional: Runtime reserves dedicated label, value and track regions instead of copying the Iced inline geometry"
        }
        (Component::SegmentedControl, "focused") => {
            "intentional: Segmented keeps selected surface only; it does not paint a 2px focus ring"
        }
        (Component::Tabs, "focused") => {
            "intentional: Tabs keep selected surface only; they do not paint the segmented 2px focus ring"
        }
        (Component::SegmentedControl, "no-selection" | "all-disabled") => {
            "intentional: the compatibility widget requires a value while Runtime supports controlled no-selection and derives tab stops only from enabled options"
        }
        (Component::SegmentedControl, _) => {
            "intentional: Runtime selected pill and option contrast are the accepted visual; Iced is reference only"
        }
        (Component::EmptyState, "complete-action") => {
            "intentional: Runtime paints a solid Primary action; Iced renders a weaker outlined control"
        }
        (Component::LabeledValue, "action") => {
            "intentional: Runtime end-aligns the action child; Iced places it beside the value"
        }
        (Component::Card, _) => {
            "intentional: Runtime preserves the authored title casing while Iced uppercases its compatibility heading"
        }
        (Component::Tooltip, _) => {
            "intentional: Runtime Tooltip is a compact pointer-bound hover card hosted by the trigger; Iced wraps arbitrary content. Visual review is the qualification gate"
        }
        (Component::Dialog | Component::ConfirmDialog | Component::Drawer, _) => {
            "intentional: Runtime ModalFrame owns scrim, surface and slotted children; Iced composes the same product chrome. Visual review is the qualification gate"
        }
        (Component::Select, "opened") => {
            "intentional: Runtime paints the opened menu in the same leaf; Iced pick-list overlay is not captured in this snapshot"
        }
        (
            Component::Select
            | Component::Popover
            | Component::ActionMenu
            | Component::ActionMenuItem
            | Component::AnchoredActionMenu
            | Component::ContextMenu,
            _,
        ) => {
            "intentional: Runtime keeps disabled select options visible and owns anchored menu chrome; Iced pick-list omits disabled popup rows. Visual review is the qualification gate"
        }
        (Component::SidebarSection, _) => {
            "intentional: Runtime header is ListItem chrome with ChevronDown/ChevronRight; Iced paints a tracked uppercase title and a rotating canvas chevron. Scene adapter cannot paint letter-spacing or rotation"
        }
        (Component::AppearanceSection | Component::AboutSection, _) => {
            "intentional: Runtime assembles qualified SettingsRow children; Iced composes the same host snapshot"
        }
        (Component::SettingsCollapsibleCard, _) => {
            "intentional: Runtime disclosure is non-interactive chrome; the card remains the single activation target"
        }
        (Component::SettingsSidebar | Component::SettingsPage | Component::DesktopShell, _) => {
            "intentional: Runtime settings and desktop composers are the public default; Iced is a migration-era reference"
        }
        (Component::GraphCanvas, _) => {
            "intentional: Runtime Scene approximates Bézier edges as quad samples and 1px grid lines; Iced strokes paths. Port discs use the Iced 4/5px radius, not the 8px hit target"
        }
        (Component::GpuTextureView, _) => {
            "intentional: Iced GpuTextureView samples the same host texture as Runtime nana.host-texture; layout chrome may differ"
        }
        (Component::GpuView, _) => {
            "intentional: Iced GpuView shader is inline; Runtime paints via DefaultGpuViewRenderer using the same WGSL, taking palette and seed from CustomRenderNode params"
        }
        (Component::Thumbnail, _) => {
            "intentional: Runtime Thumbnail is a compact HostTexture slot with empty/loading/unavailable chrome; Iced has no list-row thumbnail primitive"
        }
        _ => fixture.divergence,
    }
}

fn snapshot_graph() -> GraphModel {
    let source = GraphNode::new(
        "source",
        "In",
        GraphPoint::new(16.0, 36.0),
        GraphSize::new(96.0, 48.0),
    )
    .with_port(GraphPort::new(
        "out",
        "Out",
        GraphPortKind::Output,
        GraphPortSide::Right,
    ));
    let target = GraphNode::new(
        "target",
        "Out",
        GraphPoint::new(180.0, 36.0),
        GraphSize::new(96.0, 48.0),
    )
    .with_port(GraphPort::new(
        "in",
        "In",
        GraphPortKind::Input,
        GraphPortSide::Left,
    ));
    GraphModel::new(
        vec![source, target],
        vec![GraphEdge::new(
            "link",
            GraphEndpoint::new("source", "out"),
            GraphEndpoint::new("target", "in"),
        )],
    )
    .expect("snapshot graph is valid")
}

fn set_full_width(style: &mut NodeStyle) {
    Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Percent(100.0));
}

fn control_size(state: &str) -> ControlSize {
    match state {
        "medium" => ControlSize::Medium,
        "large" => ControlSize::Large,
        _ => ControlSize::Small,
    }
}

fn segmented_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

fn button_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

fn text_input_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

fn textarea_value(state: &str) -> &'static str {
    match state {
        "placeholder" => "",
        "clipped" | "scroll" => {
            "First line\nSecond line\nThird line\nFourth line\nFifth line\nSixth line stays"
        }
        _ => "First line\nSecond line\nThird line",
    }
}

fn hosted_textarea_value(state: &str) -> &'static str {
    match state {
        "placeholder" => "",
        _ => "fn main() {\n    let ready = true;\n}\n",
    }
}

fn button_kind(state: &str) -> nana_ui::ButtonKind {
    match state {
        "subtle" => nana_ui::ButtonKind::Subtle,
        "selected" => nana_ui::ButtonKind::Selected,
        "primary" => nana_ui::ButtonKind::Primary,
        "warning" => nana_ui::ButtonKind::Warning,
        "danger" => nana_ui::ButtonKind::Danger,
        "text-kind" => nana_ui::ButtonKind::Text,
        _ => nana_ui::ButtonKind::Ghost,
    }
}

fn card_kind(state: &str) -> CardKind {
    match state {
        "outlined" => CardKind::Outlined,
        "raised" => CardKind::Raised,
        "flat" => CardKind::Flat,
        "selected" => CardKind::Selected,
        _ => CardKind::Surface,
    }
}

fn range_value(state: &str) -> f64 {
    match state {
        "minimum" => 0.0,
        "maximum" => 1.0,
        "decimal-step" => 0.34,
        "arrow-decrement" | "page-decrement" => 0.7,
        "arrow-increment" | "page-increment" => 0.3,
        _ => 0.5,
    }
}

fn status_tone(state: &str) -> StatusTone {
    match state {
        "info" => StatusTone::Info,
        "success" => StatusTone::Success,
        "warning" => StatusTone::Warning,
        "danger" => StatusTone::Danger,
        _ => StatusTone::Neutral,
    }
}

fn status_badge_label(state: &str) -> &'static str {
    match state {
        "info" => "Syncing",
        "success" => "Ready",
        "warning" => "Delayed",
        "danger" => "Offline",
        _ => "Idle",
    }
}

fn validation_intent(state: &str) -> ValidationIntent {
    if state == "warning" {
        ValidationIntent::Warning
    } else {
        ValidationIntent::Danger
    }
}

fn validation_message(state: &str) -> &'static str {
    if state == "warning" {
        "This name may be ambiguous"
    } else {
        "A project name is required"
    }
}

fn empty_title(state: &str) -> &'static str {
    match state {
        "narrow-cjk" | "extreme-clip" => "暂无匹配的项目 👩🏽‍💻",
        "compact" => "No recent projects",
        "title-only" => "Nothing selected",
        _ => "No projects yet",
    }
}

fn empty_message(state: &str) -> &'static str {
    match state {
        "narrow-cjk" | "extreme-clip" => {
            "请调整筛选条件，或新建一个包含协作者、标签与说明的项目 🚀"
        }
        "compact" => "Open a project to see it here",
        _ => "Create the first project in this workspace",
    }
}

#[cfg(test)]
mod tests {
    use super::validate_fixture_registry;

    #[test]
    fn fixture_registry_covers_compiled_runtime_qualified_components() {
        validate_fixture_registry().expect("snapshot fixture registry must match the catalog");
    }
}
