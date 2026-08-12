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
pub mod menu;
pub mod overlay;
mod resize_drag;
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
pub mod workspace;

pub use absolute::{Absolute, absolute_content_max};
#[cfg(feature = "rich-text")]
pub use components::SelectableRichText;
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
pub use components::menus::{
    ActionMenuItem, AnchoredActionMenu, AnchoredMenuPlacement, AnchoredMenuPosition,
    ContextMenuEvent, ContextMenuHost, ContextMenuItem, OverlayHost,
};
#[cfg(feature = "overlays")]
pub use components::overlays::{ConfirmDialog, Dialog, Drawer, DrawerSide, Tooltip};
#[cfg(feature = "popover")]
pub use components::popover::{Popover, PopoverPlacement};
#[cfg(feature = "qr-code")]
pub use components::qr_code::{QrCodeCanvas, QrCodeError};
#[cfg(feature = "controls")]
pub use components::reorder_list::{ReorderItem, ReorderList};
#[cfg(feature = "rich-text")]
pub use components::rich_text::{
    MarkdownBlock, MarkdownBlockKind, MarkdownSpan, MarkdownTable, MarkdownTableAlignment,
    NativeMarkdown, native_markdown,
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
#[cfg(feature = "xy-pad")]
pub use components::xy_pad::{XYPad, XYPadEvent, XYPadState, XYPadValue};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use dock::{
    DockAction, DockAxis, DockBounds, DockChromeStyle, DockContents, DockController,
    DockDropTarget, DockDropZone, DockError, DockHostEffect, DockId, DockItemSpec, DockLayout,
    DockNode, DockSurfaceId, DockUpdate, FloatingDock, dock_window_workspace, dock_workspace,
};
#[cfg(feature = "hosted")]
pub use dock::{hosted_dock_update, hosted_dock_update_with_title_bar};
pub use geometry::{LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
#[cfg(feature = "gpu")]
pub use gpu_texture::{GpuTextureView, HostTexture, HostTextureLayer};
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
    HostedGpuContext, HostedGpuError, HostedGpuResources, HostedGpuSurface, HostedSurfaceFrame,
};
#[cfg(feature = "hosted")]
pub use hosted_renderer::{HostedUiFrame, HostedUiRenderer, HostedUiTarget};
#[cfg(feature = "hosted")]
pub use hosted_runtime::{
    HostedProgram, HostedProgramContext, HostedProgramUpdate, HostedRedraw, HostedRunError,
    HostedRuntimeEvent, HostedTitleBarMode, HostedUiCommand, HostedWindowAction,
    HostedWindowCommand, HostedWindowEvent, HostedWindowGeometry, HostedWindowId, HostedWindowRole,
    HostedWindowSettings, run_hosted,
};
pub use icons::{Icon, disclosure_icon, icon, spinner_icon, status_indicator};
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use menu::{MenuConfirmation, MenuSelection};
#[cfg(feature = "hosted")]
pub use nana_window::apply_hosted_system_material;
pub use nana_window::{
    Appearance as WindowAppearance, FallbackColor, MaterialEffect, MaterialFallback,
    MaterialOutcome, PlatformMaterialSupport, apply_system_material, clear_system_material,
    platform_material_support,
};
pub use overlay::ExclusiveOverlay;
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
pub use virtual_list::{VirtualListLayout, VirtualListWindow};
pub use widgets::{ButtonKind, ButtonPaintOverride, CardKind};
pub use window_chrome::{
    WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState, WindowControlMode,
    custom_title_bar_window,
};
pub use workspace::{
    WorkspaceAction, WorkspaceController, WorkspaceRegion, WorkspaceRegions, WorkspaceSlots,
    workspace_view,
};
