//! NanaUI's native Lilia-style application framework — **L3** Style Model entry
//! and the **sole** Iced paint implementation.
//!
//! Styling contract: `nana_ui_core::style_model` (Tokens + Semantics + Layout).
//! This crate adapts that model to widgets; it does **not** parse CSS/DOM/JS.
//! L1/L2 bridges live outside (`nana-ui-vue`, `nanavue-*`) and map into the same
//! model.
//!
//! [`WorkspaceController`], [`WorkspaceSlots`], and [`workspace_view`] provide
//! the reusable workspace contract.

pub mod absolute;
#[cfg(feature = "hosted")]
mod accessibility;
mod async_runtime;
pub mod command;
pub mod components;
pub mod dialog;
pub mod dock;
mod drag_handle;
pub mod geometry;
#[cfg(feature = "gpu")]
pub mod gpu_texture;
#[cfg(feature = "gpu")]
pub mod gpu_view;
#[cfg(feature = "graph-canvas")]
pub mod graph;
#[cfg(feature = "hosted")]
mod hosted_context;
#[cfg(feature = "hosted")]
pub mod hosted_renderer;
#[cfg(feature = "hosted")]
mod hosted_runtime;
pub mod icons;
pub mod layout;
pub mod layout_probe;
pub mod menu;
pub mod overlay;
pub mod pane;
mod runtime_animation;
#[cfg(feature = "hosted")]
mod runtime_host;
mod runtime_input;
mod scene_view;
pub mod selection;
pub mod settings;
mod shell;
pub mod sidebar;
pub mod split_pane;
pub mod theme;
pub mod tooltip;
pub mod virtual_list;
pub mod widgets;
pub mod window_chrome;
#[cfg(all(feature = "hosted", target_os = "windows"))]
mod windows_pen;
pub mod workspace;

/// Canonical backend-neutral Nana framework API.
///
/// New applications should build retained state through this module. The
/// top-level Iced-shaped component exports remain migration compatibility
/// adapters and are not the framework's stable extension contract.
pub mod runtime {
    pub use nana_ui_runtime::*;
    pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
}

pub use absolute::{Absolute, absolute_content_max};
pub use async_runtime::{run_subscription, run_task};
pub use command::{
    ActionDescriptor, ActionId, ActionMatch, ActionPickerNavigation, ActionPickerSelection,
    ActionPickerState, ActionRegistry, ActionRegistryError, ContextPredicate, KeyBinding,
    KeyContext, KeyModifiers, KeyStroke, Keymap, KeymapMatch, KeymapState,
};
pub use components::actions::{Button, ControlSize, IconButton};
#[cfg(feature = "calendar")]
pub use components::calendar::{
    CalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapDatum,
    CalendarHeatmapDayLabel, CalendarHeatmapEvent, CalendarHeatmapModel, CalendarHeatmapMonthLabel,
    CalendarHeatmapOptions, CalendarHeatmapState, CalendarLevelResolver, CalendarLevelStrategy,
    CalendarTitleFormatter, build_calendar_heatmap_model,
};
#[cfg(feature = "charts")]
pub use components::charts::TimeSeriesChart;
#[cfg(feature = "overlays")]
pub use components::command_palette::{
    COMMAND_PALETTE_INPUT_ID, CommandPalette, CommandPaletteEvent, CommandPaletteItem,
};
#[cfg(feature = "syntax-highlighting")]
pub use components::controls::HostedSyntaxHighlighting;
#[cfg(feature = "controls")]
pub use components::controls::{
    Checkbox, HostedTextarea, HostedTextareaState, Input, RangeField, SegmentedControl, Select,
    SelectionOption, Switch, TabDragGroup, TabDragSurface, Tabs, Textarea,
};
#[cfg(feature = "feedback")]
pub use components::feedback::{
    LevelMeter, Progress, Skeleton, Spinner, StatusBadge, StatusTone, Toast, ToastTone,
    ValidationIntent, ValidationMessage,
};
#[cfg(feature = "graph-canvas")]
pub use components::graph_canvas::{GraphCanvas, GraphCanvasEvent, GraphCanvasState};
#[cfg(feature = "image-viewer")]
pub use components::image_viewer::{ImageViewer, ImageViewerSource};
pub use components::key_capture_layer::{KeyCaptureEvent, KeyCaptureLayer};
pub use components::keymap_layer::KeymapLayer;
pub use components::menus::{
    ActionMenuItem, AnchoredActionMenu, AnchoredMenuPlacement, AnchoredMenuPosition,
    ContextMenuAnchor, ContextMenuEvent, ContextMenuHost, ContextMenuItem, ContextMenuTrigger,
    OverlayHost,
};
#[cfg(feature = "overlays")]
pub use components::overlays::{ConfirmDialog, Dialog, Drawer, DrawerSide, Tooltip};
#[cfg(feature = "popover")]
pub use components::popover::{ActionMenu, Popover, PopoverAlignment, PopoverPlacement};
#[cfg(feature = "qr-code")]
pub use components::qr_code::{QrCodeCanvas, QrCodeError};
#[cfg(feature = "controls")]
pub use components::reorder_list::{ReorderItem, ReorderList, TreeDropIntent, TreeDropPosition};
#[cfg(feature = "rich-text")]
pub use components::rich_text::{
    MarkdownBlock, MarkdownBlockKind, MarkdownImage, MarkdownSpan, MarkdownTable,
    MarkdownTableAlignment, NativeMarkdown, native_markdown,
};
#[cfg(feature = "selects")]
pub use components::selects::{
    Dropdown, DropdownEvent, DropdownOption, DropdownSelection, SearchDropdown,
    SearchDropdownOption, SearchDropdownState,
};
#[cfg(feature = "settings-components")]
pub use components::settings_sections::{
    AboutMetadata, AboutSection, AppearanceEvent, AppearanceSection, SettingsCollapsibleCard,
};
#[cfg(feature = "surfaces")]
pub use components::surfaces::{
    Card, DockPanel, EmptyState, FormField, InteractiveCard, LabeledValue, ListItem,
};
#[cfg(feature = "surfaces")]
pub use components::tree_view::{
    TreeNavigation, TreeNode, TreeView, TreeViewEvent, tree_navigation_event,
};
#[cfg(feature = "xy-pad")]
pub use components::xy_pad::{XYPad, XYPadEvent, XYPadState, XYPadValue};
#[cfg(feature = "rich-text")]
pub use components::{SelectableRichText, TextSelectionGroup, TextSelectionSnapshot};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use dock::{
    DockAction, DockAxis, DockBounds, DockChromeStyle, DockContents, DockController,
    DockDropTarget, DockDropZone, DockError, DockHostEffect, DockId, DockItemSpec, DockLayout,
    DockMutation, DockNode, DockSurfaceId, DockUpdate, FloatingDock, dock_window_workspace,
    dock_workspace,
};
#[cfg(feature = "hosted")]
pub use dock::{hosted_dock_update, hosted_dock_update_with_title_bar};
pub use geometry::{LogicalPoint, LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
#[cfg(feature = "gpu")]
pub use gpu_texture::{
    GpuTextureView, HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureLayer,
    HostTextureRegistry,
};
#[cfg(feature = "gpu")]
pub use gpu_view::{GpuView, GpuViewMode, GpuViewPalette, RenderSlot};
#[cfg(feature = "graph-canvas")]
pub use graph::{
    GRAPH_EDGE_HIT_TOLERANCE, GRAPH_MAX_ZOOM, GRAPH_MIN_ZOOM, GRAPH_PORT_HIT_RADIUS, GraphCanvasId,
    GraphEdge, GraphEdgeId, GraphEndpoint, GraphModel, GraphModelError, GraphNode, GraphNodeId,
    GraphPoint, GraphPort, GraphPortId, GraphPortKind, GraphPortSide, GraphRect, GraphSelection,
    GraphSize, GraphTarget, GraphTargetDescriptor, GraphTargetId, GraphTargetKind, GraphViewport,
};
#[cfg(feature = "hosted")]
pub use hosted_context::{
    HostedDeviceLost, HostedGpuContext, HostedGpuError, HostedGpuResources, HostedGpuSurface,
    HostedSurfaceFrame,
};
#[cfg(feature = "hosted")]
pub use hosted_renderer::{HostedUiFrame, HostedUiRenderer, HostedUiTarget};
#[cfg(feature = "browser")]
pub use hosted_runtime::{
    HostedBrowserBounds, HostedBrowserCommand, HostedBrowserCommandKind, HostedBrowserEvent,
    HostedBrowserId, HostedBrowserLoadState,
};
#[cfg(feature = "hosted")]
pub use hosted_runtime::{
    HostedDisplayArea, HostedFrameMetrics, HostedInputDisposition, HostedInputEvent,
    HostedInputModifiers, HostedPointerPhase, HostedPointerType, HostedProgram,
    HostedProgramContext, HostedProgramUpdate, HostedRedraw, HostedRunError, HostedRuntimeEvent,
    HostedTextPosition, HostedTitleBarMode, HostedUiCommand, HostedWindowAction,
    HostedWindowCaptureId, HostedWindowCommand, HostedWindowEvent, HostedWindowGeometry,
    HostedWindowId, HostedWindowPlacement, HostedWindowRole, HostedWindowSettings, run_hosted,
    run_hosted_with,
};
pub use icons::{Icon, disclosure_icon, icon, spinner_icon, status_indicator};
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use layout_probe::{LayoutBounds, LayoutProbe};
pub use menu::{MenuConfirmation, MenuSelection};
pub use nana_ui_core::{
    ExpansionState, SplitPaneModel, SplitPaneMutation, WORKSPACE_REGION_TRANSITION_DURATION,
    WorkspaceModel, WorkspaceMutation,
};
#[cfg(feature = "hosted")]
pub use nana_ui_platform::ImeEvent;
pub use nana_ui_runtime::{
    AccessibilityActionRequest, AccessibilityNode, AccessibilityRole, AccessibilityUpdate,
};
#[cfg(feature = "hosted")]
pub use nana_window::apply_hosted_system_material;
pub use nana_window::{
    Appearance as WindowAppearance, FallbackColor, MaterialEffect, MaterialFallback,
    MaterialOutcome, PlatformMaterialSupport, apply_system_material, clear_system_material,
    platform_material_support,
};
pub use overlay::ExclusiveOverlay;
pub use pane::{
    PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode, ratio_pane_split,
};
pub use runtime_animation::RuntimeAnimationClock;
#[cfg(feature = "hosted")]
pub use runtime_host::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw, RuntimeTaskError,
    RuntimeWindowSettings, run_runtime,
};
pub use runtime_input::RuntimeInputAdapter;
pub use scene_view::{HostTextureSceneResolver, IcedSceneView, ScenePaintError};
pub use selection::{SelectionMove, SingleSelection};
pub use settings::{
    AppearanceSettings, BackdropTarget, SettingsCard, SettingsError, SettingsModel, SettingsRow,
    SettingsState, SettingsTab, SettingsTabId, WindowMaterialMode, settings_page, settings_sidebar,
};
pub use shell::{
    AppTitleBar, DesktopShell, PopupShell, PopupTitleBarFrame, app_shell, app_title_bar,
};
pub use sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState, SidebarRowTone,
    SidebarSection, SidebarSectionState,
};
pub use split_pane::{SplitAxis, SplitPaneAction, SplitPaneController, split_pane};
pub use theme::{
    Colors, SemanticColor, SemanticPalette, ThemeMetrics, ThemeMode, ThemeModeExt, ThemeTokens,
    UI_BASE_TEXT_SIZE, UI_METRICS, ui_font, ui_font_defaults,
};
#[cfg(feature = "bundled-fonts")]
pub use theme::{UI_FONT_BOLD, UI_FONT_MEDIUM, UI_FONT_REGULAR, UI_FONT_SEMIBOLD, ui_font_sources};
pub use tooltip::{TooltipConfig, TooltipPlacement};
pub use virtual_list::{
    TableColumn, TableCursor, TableNavigation, VirtualListLayout, VirtualListMaterialization,
    VirtualListMaterializationError, VirtualListMaterializer, VirtualListMount, VirtualListWindow,
    VirtualTableLayout, VirtualTableMaterialization, VirtualTableMaterializer, VirtualTableWindow,
};
pub use widgets::{ButtonKind, ButtonPaintOverride, CardKind};
pub use window_chrome::{
    WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState, WindowControlMode,
    custom_title_bar_window,
};
pub use workspace::{
    WorkspaceAction, WorkspaceController, WorkspaceRegion, WorkspaceRegions, WorkspaceSlots,
    workspace_view,
};
