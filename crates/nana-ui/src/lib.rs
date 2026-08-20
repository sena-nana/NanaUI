//! NanaUI L3 adapter and the Scene/WGPU paint of Runtime/UiScene.
//!
//! Product retained/render contracts live in `nana-ui-runtime` and `nana-ui-scene`.
//! This crate adapts Style Model (`nana_ui_core::style_model`) and Scene frames to
//! the Nana Scene host. It is not the long-term application programming model.
//! L1/L2 Vue + JS (`nana-ui-vue`, `nanavue-*`) map into the same model.
//!
//! [`WorkspaceController`] owns workspace region layout and interaction.

#[cfg(feature = "hosted")]
mod accessibility;
pub mod command;
pub mod component_support;
pub mod components;
#[cfg(feature = "gpu")]
mod default_gpu_view;
pub mod dialog;
pub mod dock;
pub mod geometry;
#[cfg(feature = "gpu")]
pub mod gpu_texture;
#[cfg(feature = "gpu")]
pub mod gpu_view;
#[cfg(feature = "gpu")]
mod gpu_work;
#[cfg(feature = "graph-canvas")]
pub mod graph;
#[cfg(feature = "hosted")]
mod hosted_context;
pub mod icons;
pub mod layout;
pub mod menu;
mod nana_text;
pub mod overlay;
pub mod pane;
mod runtime_animation;
#[cfg(feature = "hosted")]
mod runtime_dock;
#[cfg(feature = "hosted")]
mod runtime_host;
mod runtime_input;
#[cfg(feature = "gpu")]
mod scene_gpu;
#[cfg(feature = "hosted")]
mod scene_host;
#[cfg(feature = "gpu")]
mod scene_paint;
pub mod selection;
pub mod settings;
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
/// New applications should build retained state through this module. Qualified
/// top-level component exports also route here.
pub mod runtime {
    pub use nana_ui_runtime::*;
    pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
}

pub use command::{
    ActionDescriptor, ActionId, ActionMatch, ActionPickerNavigation, ActionPickerSelection,
    ActionPickerState, ActionRegistry, ActionRegistryError, ContextPredicate, KeyBinding,
    KeyContext, KeyModifiers, KeyStroke, Keymap, KeymapMatch, KeymapState,
    action_picker_from_key_name,
};
pub use component_support::{
    ComponentCapability, ComponentFamily, ComponentId, ComponentMigrationState, ComponentSupport,
    component_catalog, component_ids, component_support, component_uses_runtime,
};
pub use nana_ui_core::ControlSize;
pub use nana_ui_core::{AnchoredMenuPlacement, StatusTone, ToastTone, ValidationIntent};
pub use nana_ui_core::{AppearanceEvent, CommandPaletteEvent, CommandPaletteItem};
#[cfg(feature = "charts")]
pub use nana_ui_runtime::TimeSeriesChart;
#[cfg(feature = "calendar")]
pub use nana_ui_runtime::{
    CalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapCellPaint,
    CalendarHeatmapDatum, CalendarHeatmapDayLabel, CalendarHeatmapEvent, CalendarHeatmapLabelPaint,
    CalendarHeatmapModel, CalendarHeatmapMonthLabel, CalendarHeatmapOptions, CalendarLevelResolver,
    CalendarLevelStrategy, CalendarMonthFormatter, CalendarTitleFormatter,
    build_calendar_heatmap_model, calendar_cell_fill,
};
pub use nana_ui_runtime::{
    CapturedStroke, KeyCaptureEvent, KeyCaptureLayer, KeyInput, KeymapLayer,
};
#[cfg(feature = "gpu")]
pub use nana_ui_runtime::{GpuTextureView, GpuView, GpuViewMode, GpuViewPalette};
#[cfg(feature = "graph-canvas")]
pub use nana_ui_runtime::{
    GraphCanvas, GraphCanvasAdjustment, GraphCanvasEvent, GraphInteraction, GraphPointerButton,
    GraphScrollDelta,
};
#[cfg(feature = "syntax-highlighting")]
pub use nana_ui_runtime::{HIGHLIGHT_PRESENTER, HighlightPresentation, SyntectHighlighter};
#[cfg(feature = "image-viewer")]
pub use nana_ui_runtime::{
    ImageViewer, ImageViewerContent, ImageViewerEvent, ImageViewerGeometry, ImageViewerHit,
    ImageViewerOffset,
};
#[cfg(feature = "rich-text")]
pub use nana_ui_runtime::{
    MarkdownBlock, MarkdownBlockKind, MarkdownImage, MarkdownSpan, MarkdownTable,
    MarkdownTableAlignment, NativeMarkdown, RichSpan, RichTextEvent, SelectableRichText,
    TextSelectionGroup, TextSelectionGroupId, TextSelectionSnapshot,
};
#[cfg(feature = "controls")]
pub use nana_ui_runtime::{
    ReorderItem, ReorderList, ReorderListEvent, ReorderListPointer, ReorderRowPaint,
    TreeDropIntent, TreeDropPosition,
};

#[cfg(feature = "gpu")]
pub use default_gpu_view::{
    DefaultGpuViewRenderer, default_scene_gpu_renderers, default_scene_gpu_renderers_with_host,
    resolve_scene_gpu_renderers,
};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
#[cfg(feature = "hosted")]
pub use dock::hosted_dock_update;
pub use dock::{
    DockAction, DockAxis, DockBounds, DockChromeStyle, DockController, DockDropTarget,
    DockDropZone, DockError, DockHostEffect, DockId, DockItemLayout, DockItemSpec, DockLayout,
    DockLayoutPersist, DockMutation, DockNode, DockSplitLayout, DockSurfaceId, DockSurfaceLayout,
    DockTabsLayout, DockUpdate, FloatingDock,
};
pub use geometry::{LogicalPoint, LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
#[cfg(feature = "gpu")]
pub use gpu_texture::{
    HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureLayer, HostTextureRegistry,
};
#[cfg(feature = "gpu")]
pub use gpu_view::RenderSlot;
#[cfg(feature = "gpu")]
pub use gpu_work::{GpuStageTimings, GpuWorkSink};
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
    HostedRunError, HostedSurfaceFrame,
};
pub use icons::Icon;
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use menu::{MenuConfirmation, MenuSelection};
pub use nana_text::NanaTextShaper;
#[cfg(feature = "gpu")]
pub use nana_ui_core::GpuWorkObservation;
pub use nana_ui_core::{DrawerSide, PopoverAlignment, PopoverPlacement};
pub use nana_ui_core::{
    ExpansionState, SplitPaneModel, SplitPaneMutation, WORKSPACE_REGION_TRANSITION_DURATION,
    WorkspaceModel, WorkspaceMutation,
};
pub use nana_ui_core::{XYPadEvent, XYPadValue};
#[cfg(feature = "hosted")]
pub use nana_ui_platform::ImeEvent;
pub use nana_ui_runtime::TextArea as Textarea;
pub use nana_ui_runtime::{
    AboutMetadata, AboutSection, ActionMenu, ActionMenuItem, AnchoredActionMenu, AppShell,
    AppTitleBar, AppTitleBarControls, AppearanceSection, Button, Card, Checkbox, CommandPalette,
    ConfirmDialog, ContextMenu, ContextMenuEvent, ContextMenuItem, DesktopShell, Dialog, Dock,
    DockFloatingSurface, DockPanel, DockSurfaceSpec, DockWorkspace, DockWorkspaceEvent, Drawer,
    Dropdown, DropdownEvent, DropdownOption, DropdownSelection, EmptyState, FormField,
    HostedTextarea, IconButton, InteractiveCard, LabeledValue, LevelMeter, ListItem, OverlayHost,
    PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode, Popover, Progress,
    ProgressCancelled, QrCode, QrCodeError, RangeField, SearchDropdown, SearchDropdownEvent,
    SearchDropdownOption, SegmentedControl, Select, SelectOption, SettingsCard,
    SettingsCollapsibleCard, SettingsRow, SidebarFooter, SidebarFooterButton, SidebarFrame,
    SidebarRow, SidebarRowState, SidebarRowTone, SidebarSection, SidebarSectionSlots,
    SidebarSectionState, Skeleton, Spinner, SplitPane, StatusBadge, Switch, TabDragGroup,
    TabDragLease, TabDragSurface, TabOption, Tabs, TabsEvent, Text, TextArea, TextInput, Toast,
    Tooltip, TreeNavigation, TreeNode, TreeView, TreeViewEvent, ValidationMessage, Workspace,
    WorkspaceRegionSlot, WorkspaceResizeHandle, XYPad, tree_navigation_event,
};
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
pub use pane::ratio_pane_split;
pub use runtime_animation::RuntimeAnimationClock;
#[cfg(feature = "hosted")]
pub use runtime_dock::{dock_workspace_window_id, runtime_dock_window_update};
#[cfg(feature = "hosted")]
pub use runtime_host::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw, RuntimeTaskError,
    RuntimeWindowSettings, run_runtime,
};
pub use runtime_input::RuntimeInputAdapter;
#[cfg(feature = "gpu")]
pub use scene_gpu::{
    SceneGpuNode, SceneGpuPassContext, SceneGpuPrepareContext, SceneGpuRenderContext,
    SceneGpuRenderer, SceneGpuRendererRegistry, SceneResourceEncodeContext,
    SceneResourceProduceError, SceneResourceProducer, SceneResourceProducerRegistry,
};
#[cfg(feature = "hosted")]
pub use scene_host::run_runtime_scene;
#[cfg(feature = "gpu")]
pub use scene_paint::{
    HostTextureSceneResolver, ScenePaintError, ScenePaintViewport, SceneWgpuPainter,
};
pub use selection::{SelectionMove, SingleSelection};
pub use settings::{
    AppearanceSettings, BackdropTarget, SettingsError, SettingsModel, SettingsState, SettingsTab,
    SettingsTabId, WindowMaterialMode,
};
pub use split_pane::{SplitAxis, SplitPaneAction, SplitPaneController};
pub use theme::{
    Color, Colors, SemanticColor, SemanticPalette, ThemeMetrics, ThemeMode, ThemeModeExt,
    ThemeTokens, UI_BASE_TEXT_SIZE, UI_METRICS,
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
};
pub use workspace::{WorkspaceAction, WorkspaceController};
