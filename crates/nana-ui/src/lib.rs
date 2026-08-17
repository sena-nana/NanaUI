//! NanaUI L3 adapter and the Iced/WGPU compatibility paint of Runtime/UiScene.
//!
//! Product retained/render contracts live in `nana-ui-runtime` and `nana-ui-scene`.
//! This crate adapts Style Model (`nana_ui_core::style_model`) and Scene frames to
//! Iced widgets for the current desktop host backend. It is not the long-term
//! application programming model. L1/L2 Vue + JS (`nana-ui-vue`, `nanavue-*`)
//! map into the same model.
//!
//! [`WorkspaceController`], [`WorkspaceSlots`], and [`workspace_view`] provide
//! the reusable workspace contract.

pub mod absolute;
#[cfg(feature = "hosted")]
mod accessibility;
mod async_runtime;
pub mod command;
pub mod component_support;
pub mod components;
#[cfg(feature = "gpu")]
mod default_gpu_view;
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
mod nana_text;
pub mod overlay;
pub mod pane;
mod runtime_animation;
#[cfg(feature = "hosted")]
mod runtime_dock;
#[cfg(feature = "hosted")]
mod runtime_host;
mod runtime_input;
mod runtime_text;
#[cfg(feature = "gpu")]
mod scene_gpu;
#[cfg(feature = "hosted")]
mod scene_host;
#[cfg(feature = "gpu")]
mod scene_paint;
#[cfg(feature = "gpu")]
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
/// New applications should build retained state through this module. Qualified
/// top-level component exports also route here; remaining Iced-shaped exports
/// are migration adapters, not the framework's stable extension contract.
pub mod runtime {
    pub use nana_ui_runtime::*;
    pub use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, SceneDelta, UiScene};
}

/// Explicit Iced compatibility adapters retained while components migrate to
/// the backend-neutral Runtime API.
///
/// Root-level component exports move to their Runtime implementation only
/// after the corresponding catalog entry is qualified. Existing consumers can
/// opt into this namespace while completing their own migration.
pub mod compatibility {
    pub use crate::components::actions::{Button, IconButton};
    #[cfg(feature = "calendar")]
    pub use crate::components::calendar::{
        CalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapDatum,
        CalendarHeatmapDayLabel, CalendarHeatmapEvent, CalendarHeatmapModel,
        CalendarHeatmapMonthLabel, CalendarHeatmapOptions, CalendarHeatmapState,
        CalendarLevelResolver, CalendarLevelStrategy, CalendarTitleFormatter,
        build_calendar_heatmap_model,
    };
    #[cfg(feature = "charts")]
    pub use crate::components::charts::TimeSeriesChart;
    #[cfg(feature = "overlays")]
    pub use crate::components::command_palette::CommandPalette;
    #[cfg(feature = "syntax-highlighting")]
    pub use crate::components::controls::HostedSyntaxHighlighting;
    #[cfg(feature = "controls")]
    pub use crate::components::controls::{
        Checkbox, HostedTextarea, HostedTextareaState, Input, RangeField, SegmentedControl, Select,
        SelectionOption, Switch, Tabs, Textarea,
    };
    #[cfg(feature = "feedback")]
    pub use crate::components::feedback::{
        LevelMeter, Progress, Skeleton, Spinner, StatusBadge, Toast, ValidationMessage,
    };
    #[cfg(feature = "graph-canvas")]
    pub use crate::components::graph_canvas::{GraphCanvas, GraphCanvasEvent, GraphCanvasState};
    #[cfg(feature = "image-viewer")]
    pub use crate::components::image_viewer::{ImageViewer, ImageViewerSource};
    pub use crate::components::key_capture_layer::{KeyCaptureEvent, KeyCaptureLayer};
    pub use crate::components::keymap_layer::KeymapLayer;
    pub use crate::components::menus::{ActionMenuItem, AnchoredActionMenu, OverlayHost};
    #[cfg(feature = "overlays")]
    pub use crate::components::overlays::{ConfirmDialog, Dialog, Drawer, Tooltip};
    #[cfg(feature = "popover")]
    pub use crate::components::popover::{ActionMenu, Popover};
    #[cfg(feature = "qr-code")]
    pub use crate::components::qr_code::QrCodeCanvas;
    #[cfg(feature = "controls")]
    pub use crate::components::reorder_list::{
        ReorderItem, ReorderList, TreeDropIntent, TreeDropPosition,
    };
    #[cfg(feature = "rich-text")]
    pub use crate::components::rich_text::{
        MarkdownBlock, MarkdownBlockKind, MarkdownImage, MarkdownSpan, MarkdownTable,
        MarkdownTableAlignment, NativeMarkdown, native_markdown,
    };
    #[cfg(feature = "rich-text")]
    pub use crate::components::selectable_rich_text::{
        SelectableRichText, TextSelectionGroup, TextSelectionSnapshot,
    };
    #[cfg(feature = "selects")]
    pub use crate::components::selects::{
        Dropdown, DropdownEvent, DropdownOption, DropdownSelection, SearchDropdown,
        SearchDropdownOption, SearchDropdownState,
    };
    #[cfg(feature = "settings-components")]
    pub use crate::components::settings_sections::{
        AboutMetadata, AboutSection, AppearanceSection, SettingsCollapsibleCard,
    };
    #[cfg(feature = "surfaces")]
    pub use crate::components::surfaces::DockPanel;
    #[cfg(feature = "surfaces")]
    pub use crate::components::surfaces::{
        Card, EmptyState, FormField, InteractiveCard, LabeledValue, ListItem,
    };
    #[cfg(feature = "surfaces")]
    pub use crate::components::tree_view::{
        TreeNavigation, TreeNode, TreeView, TreeViewEvent, tree_navigation_event,
        tree_navigation_from_iced_key,
    };
    #[cfg(feature = "xy-pad")]
    pub use crate::components::xy_pad::XYPad;
    #[cfg(feature = "gpu")]
    pub use crate::gpu_texture::GpuTextureView;
    #[cfg(feature = "gpu")]
    pub use crate::gpu_view::{GpuView, GpuViewMode, GpuViewPalette, RenderSlot};
    pub use crate::pane::{
        PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode,
    };
    pub use crate::settings::{SettingsCard, SettingsRow};
    pub use crate::shell::AppTitleBar;
    pub use crate::sidebar::{
        SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState,
        SidebarRowTone, SidebarSection, SidebarSectionState,
    };
    pub use iced::widget::Text;
}

pub use absolute::{Absolute, absolute_content_max};
pub use async_runtime::{run_subscription, run_task};
pub use command::{
    ActionDescriptor, ActionId, ActionMatch, ActionPickerNavigation, ActionPickerSelection,
    ActionPickerState, ActionRegistry, ActionRegistryError, ContextPredicate, KeyBinding,
    KeyContext, KeyModifiers, KeyStroke, Keymap, KeymapMatch, KeymapState,
    action_picker_from_iced_key,
};
pub use component_support::{
    ComponentCapability, ComponentFamily, ComponentId, ComponentMigrationState, ComponentSupport,
    component_catalog, component_ids, component_support, component_uses_runtime,
};
pub use components::actions::ControlSize;
#[cfg(feature = "overlays")]
pub use components::command_palette::COMMAND_PALETTE_INPUT_ID;
#[cfg(feature = "controls")]
pub use components::controls::SelectionOption;
#[cfg(feature = "feedback")]
pub use components::feedback::{StatusTone, ToastTone, ValidationIntent};
#[cfg(feature = "image-viewer")]
pub use components::image_viewer::ImageViewerSource;
pub use components::menus::{
    AnchoredMenuPlacement, AnchoredMenuPosition, ContextMenuAnchor, ContextMenuEvent,
    ContextMenuHost, ContextMenuItem, ContextMenuTrigger,
};
#[cfg(feature = "rich-text")]
pub use components::rich_text::native_markdown;
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

#[cfg(feature = "xy-pad")]
pub use components::xy_pad::XYPadState;

#[cfg(feature = "gpu")]
pub use default_gpu_view::{
    DefaultGpuViewRenderer, default_scene_gpu_renderers, default_scene_gpu_renderers_with_host,
    resolve_scene_gpu_renderers,
};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use dock::{
    DockAction, DockAxis, DockBounds, DockChromeStyle, DockContents, DockController,
    DockDropTarget, DockDropZone, DockError, DockHostEffect, DockId, DockItemLayout, DockItemSpec,
    DockLayout, DockMutation, DockNode, DockSplitLayout, DockSurfaceId, DockSurfaceLayout,
    DockTabsLayout, DockUpdate, FloatingDock, dock_window_workspace, dock_workspace,
};
#[cfg(feature = "hosted")]
pub use dock::{hosted_dock_update, hosted_dock_update_with_title_bar};
pub use geometry::{LogicalPoint, LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
#[cfg(feature = "gpu")]
pub use gpu_texture::{
    HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureLayer, HostTextureRegistry,
};
#[cfg(feature = "gpu")]
pub use gpu_view::RenderSlot;
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
pub use nana_text::NanaTextShaper;
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
    ConfirmDialog, ContextMenu, Dialog, Dock, DockFloatingSurface, DockPanel, DockSurfaceSpec,
    DockWorkspace, DockWorkspaceEvent, Drawer, Dropdown, DropdownEvent, DropdownOption,
    DropdownSelection, EmptyState, FormField, HostedTextarea, IconButton, InteractiveCard,
    LabeledValue, LevelMeter, ListItem, OverlayHost, PaneChrome, PaneChromeAction,
    PaneChromeActionKind, PaneTree, PaneTreeNode, Popover, Progress, ProgressCancelled, QrCode,
    QrCodeError, RangeField, SearchDropdown, SearchDropdownEvent, SearchDropdownOption,
    SegmentedControl, Select, SelectOption, SettingsCard, SettingsCollapsibleCard, SettingsRow,
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState, SidebarRowTone,
    SidebarSection, SidebarSectionSlots, SidebarSectionState, Skeleton, Spinner, SplitPane,
    StatusBadge, Switch, TabDragGroup, TabDragLease, TabDragSurface, TabOption, Tabs, TabsEvent,
    Text, TextArea, TextInput, Toast, Tooltip, TreeNavigation, TreeNode, TreeView, TreeViewEvent,
    ValidationMessage, Workspace, WorkspaceRegionSlot, WorkspaceResizeHandle, XYPad,
    tree_navigation_event,
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
pub use runtime_text::IcedTextShaper;
#[cfg(feature = "gpu")]
pub use scene_gpu::{
    SceneGpuNode, SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer,
    SceneGpuRendererRegistry, SceneResourceEncodeContext, SceneResourceProduceError,
    SceneResourceProducer, SceneResourceProducerRegistry,
};
#[cfg(feature = "hosted")]
pub use scene_host::run_runtime_scene;
#[cfg(feature = "gpu")]
pub use scene_paint::{ScenePaintViewport, SceneWgpuPainter};
#[cfg(feature = "gpu")]
pub use scene_view::{HostTextureSceneResolver, IcedSceneView, ScenePaintError};
pub use selection::{SelectionMove, SingleSelection};
pub use settings::{
    AppearanceSettings, BackdropTarget, SettingsError, SettingsModel, SettingsState, SettingsTab,
    SettingsTabId, WindowMaterialMode, settings_page, settings_sidebar,
};
pub use shell::{DesktopShell, PopupShell, PopupTitleBarFrame, app_shell, app_title_bar};
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
