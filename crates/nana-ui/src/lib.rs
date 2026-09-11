//! NanaUI adapter and the Scene/WGPU paint of Runtime/UiScene.
//!
//! Product retained/render contracts live in `nana-ui-runtime` and `nana-ui-scene`.
//! New applications should use [`runtime`] (`AppContext`, `build`, `mount`,
//! `ComponentView`, `register_component`). See
//! [`docs/how-it-works.md`](../../../docs/how-it-works.md),
//! [`docs/start.md`](../../../docs/start.md),
//! [`docs/application-api.md`](../../../docs/application-api.md).
//! Crate-root widget re-exports are a compatibility surface, not the extension
//! contract. Vue + JS (`nana-ui-vue`, `nanavue-*`) map into the same model.
//!
//! [`WorkspaceController`] is a host adapter (Instant→Duration, pointer →
//! [`WorkspaceMutation`]). Product region state is [`WorkspaceModel`].

#[cfg(any(feature = "hosted", feature = "accesskit-tree"))]
mod accessibility;
#[cfg(feature = "accesskit-tree")]
mod accessibility_tree;
pub mod command;
pub mod component_support;
pub mod components;
#[cfg(feature = "gpu")]
mod default_gpu_view;
pub mod dialog;
/// Host adapter (`nana_ui::dock::*`): pointer/dwell/frame → [`dock::DockMutation`].
/// Product dock is Runtime [`DockWorkspace`], re-exported at crate root.
pub mod dock;
#[cfg(feature = "gpu")]
mod font_face_ingest;
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
#[cfg(all(feature = "hosted", target_os = "windows"))]
mod windows_composition;
#[cfg(all(feature = "hosted", target_os = "windows"))]
pub use windows_composition::{
    WindowsComposition, WindowsCompositionError, WindowsCompositionRect, WindowsNativeVisual,
};
#[cfg(feature = "hosted")]
mod application;
#[cfg(feature = "hosted")]
pub use application::{ApplicationState, ApplicationWindow, RuntimeApplication};
pub mod icons;
pub mod layout;
pub mod menu;
mod nana_text;
#[cfg(feature = "hosted")]
mod native_browser;
#[cfg(feature = "gpu")]
mod native_content;
#[cfg(feature = "hosted")]
pub use native_browser::{
    BrowserCommand, BrowserEvent, BrowserPolicy, BrowserRect, BrowserState, NativeBrowserEvent,
    NativeBrowserRequest,
};
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
#[cfg(feature = "gpu")]
pub use native_content::{NativeContentRegion, native_content_regions};
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
pub mod workspace;

pub mod runtime;

pub use command::{
    ActionDescriptor, ActionId, ActionMatch, ActionPickerNavigation, ActionPickerSelection,
    ActionPickerState, ActionRegistry, ActionRegistryError, ContextPredicate, KeyBinding,
    KeyContext, KeyModifiers, KeyStroke, Keymap, KeymapMatch, KeymapState,
    action_picker_from_key_name,
};
pub use component_support::{
    ComponentCapability, ComponentFamily, ComponentId, ComponentSupport, component_catalog,
    component_ids, component_support, component_uses_runtime,
};
pub use nana_ui_core::ContentFit;
pub use nana_ui_core::ControlSize;
pub use nana_ui_core::{AnchoredMenuPlacement, StatusTone, ToastTone, ValidationIntent};
pub use nana_ui_core::{AppearanceEvent, CommandPaletteEvent, CommandPaletteItem};
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
#[cfg(feature = "charts")]
pub use nana_ui_runtime::{DonutChart, DonutSlice, TimeSeriesChart, TimeSeriesLayer};
#[cfg(feature = "gpu")]
pub use nana_ui_runtime::{GpuTextureView, GpuView, GpuViewMode, GpuViewPalette};
#[cfg(feature = "graph-canvas")]
pub use nana_ui_runtime::{
    GraphCanvas, GraphCanvasAdjustment, GraphCanvasEvent, GraphCanvasHit, GraphInteraction,
    GraphMinimap, GraphMinimapEvent, GraphNodeContent, GraphPointerButton, GraphScrollDelta,
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

#[cfg(feature = "accesskit-tree")]
pub use accessibility_tree::AccessTreeProjector;
#[cfg(feature = "gpu")]
pub use default_gpu_view::{
    DefaultGpuViewRenderer, default_scene_gpu_renderers, default_scene_gpu_renderers_with_host,
    resolve_scene_gpu_renderers,
};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
#[cfg(feature = "gpu")]
pub use font_face_ingest::{HostFontFaceSpec, ingest_host_font_faces};
pub use geometry::{LogicalPoint, LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
#[cfg(feature = "gpu")]
pub use gpu_texture::{
    HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureRegistry, TextureSlot,
    TextureSubscription,
};
#[cfg(feature = "gpu")]
pub use gpu_view::RenderSlot;
#[cfg(feature = "gpu")]
pub use gpu_work::{GpuStageTimings, GpuWorkSink};
#[cfg(feature = "graph-canvas")]
pub use graph::{
    GRAPH_EDGE_HIT_TOLERANCE, GRAPH_MAX_ZOOM, GRAPH_MIN_ZOOM, GRAPH_NODE_TITLE_HEIGHT,
    GRAPH_PORT_HIT_RADIUS, GRAPH_PORT_INSET, GRAPH_PORT_PITCH, GraphCanvasId, GraphEdge,
    GraphEdgeId, GraphEndpoint, GraphModel, GraphModelError, GraphNode, GraphNodeId, GraphPoint,
    GraphPort, GraphPortId, GraphPortKind, GraphPortSide, GraphRect, GraphSelection, GraphSize,
    GraphTarget, GraphTargetDescriptor, GraphTargetId, GraphTargetKind, GraphViewport,
    graph_node_fitted_height,
};
#[cfg(feature = "hosted")]
pub use hosted_context::{
    HostedDeviceLost, HostedGpuContext, HostedGpuError, HostedGpuResources, HostedGpuSurface,
    HostedRunError, HostedSurfaceFrame, HostedSurfaceMode,
};
pub use icons::Icon;
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use menu::{MenuConfirmation, MenuSelection};
#[cfg(feature = "hosted")]
pub use nana_app_icon::{default_window_icon, window_icon_from_png};
/// Full generated Tabler catalog (`icons_tabler::USER`, `::KEYBOARD`, …) as
/// typed [`Icon`] constants, behind the `icons-tabler` feature. The built-in
/// catalog only covers shell chrome; reach here before hand-authoring
/// [`IconData`]. The linker drops every constant the product never names.
#[cfg(feature = "icons-tabler")]
pub use nana_icons_tabler as icons_tabler;
pub use nana_text::{
    HostFontError, HostFontStyle, NanaTextShaper, alias_host_font_face_local,
    register_host_font_bytes, register_host_font_face, register_host_font_file,
    set_sans_serif_family, shaped_face_families,
};
#[cfg(feature = "gpu")]
pub use nana_ui_core::GpuWorkObservation;
pub use nana_ui_core::{DrawerSide, PopoverAlignment, PopoverPlacement};
pub use nana_ui_core::{
    ExpansionState, SplitPaneModel, SplitPaneMutation, WORKSPACE_REGION_TRANSITION_DURATION,
    WorkspaceModel, WorkspaceMutation,
};
pub use nana_ui_core::{XYPadEvent, XYPadValue};
/// Fetch host boundary, re-exported so hosts can configure [`set_resource_fetch_host`]
/// without depending on `nana-ui-platform` directly.
#[cfg(feature = "gpu")]
pub use nana_ui_platform::{
    FetchError, FetchErrorKind, FetchHost, FetchPolicy, FetchRequest, FetchResponse,
    NativeFetchHost, SharedFetchHost, shared_fetch_host,
};
#[cfg(feature = "hosted")]
pub use nana_ui_platform::{
    ImeEvent, WindowIcon, WindowIconError, clear_registered_application_icon,
    register_application_icon,
};
/// Compatibility re-export of Runtime `TextArea`. Prefer [`runtime::TextArea`].
pub use nana_ui_runtime::TextArea as Textarea;
/// Compatibility widget surface. New applications should import from [`runtime`].
pub use nana_ui_runtime::{
    AboutMetadata, AboutSection, ActionMenu, ActionMenuItem, AnchoredActionMenu, AppShell,
    AppTitleBar, AppTitleBarControls, AppearanceSection, Avatar, BrowseRequested, Button, Card,
    Checkbox, Chip, ChipDismissed, ColorChanged, ColorField, ColorInput, CommandPalette,
    ConfirmDialog, ContextMenu, ContextMenuEvent, ContextMenuItem, DesktopShell, Dialog, Drawer,
    Dropdown, DropdownEvent, DropdownOption, DropdownSelection, EmptyState, FormField,
    HostedTextarea, IconButton, IconGlyph, InteractiveCard, LabeledValue, LevelMeter, ListItem,
    OVERLAY_IDLE, OverlayHost, OverlayLocks, OverlayVisibility, OverlayVisibilityConfig,
    PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode, PathField, Popover,
    Progress, ProgressCancelled, QrCode, QrCodeError, RangeField, SearchDropdown,
    SearchDropdownEvent, SearchDropdownOption, SegmentedControl, Select, SelectOption,
    SettingsCard, SettingsCollapsibleCard, SettingsRow, SidebarFooter, SidebarFooterButton,
    SidebarFrame, SidebarRow, SidebarRowState, SidebarRowTone, SidebarSection, SidebarSectionSlots,
    SidebarSectionState, Skeleton, Spinner, SplitPane, StatusBadge, Switch, TabDragGroup,
    TabDragLease, TabDragSurface, TabOption, Tabs, TabsEvent, Text, TextArea, TextInput, Thumbnail,
    ThumbnailState, Toast, Tooltip, TreeNavigation, TreeNode, TreeView, TreeViewEvent,
    ValidationMessage, Workspace, WorkspaceRegionSlot, WorkspaceResizeHandle, XYPad,
    tree_navigation_event,
};
pub use nana_ui_runtime::{
    AccessibilityActionRequest, AccessibilityNode, AccessibilityRole, AccessibilityUpdate,
};
/// Product dock from Runtime. `nana_ui::dock::*` is the host adapter, not a second dock.
pub use nana_ui_runtime::{
    Dock, DockFloatingSurface, DockPanel, DockSurfaceSpec, DockWorkspace, DockWorkspaceEvent,
};
#[cfg(feature = "hosted")]
pub use nana_window::apply_hosted_system_material;
#[cfg(feature = "hosted")]
pub use nana_window::{
    Appearance as WindowAppearance, FallbackColor, MaterialEffect, MaterialFallback,
    MaterialOutcome, Menu, MenuBar, MenuBarSupport, MenuEntry, MenuShortcut,
    PlatformMaterialSupport, apply_system_material, clear_system_material,
    hosted_platform_material_support, install_menu_bar, menu_bar_support,
    platform_material_support, take_menu_activations,
};
pub use overlay::ExclusiveOverlay;
pub use pane::ratio_pane_split;
pub use runtime_animation::RuntimeAnimationClock;
#[cfg(feature = "hosted")]
pub use runtime_dock::{dock_workspace_window_id, runtime_dock_window_update};
#[cfg(feature = "hosted")]
pub use runtime_host::{
    FrameDemand, HostFailure, RoutedInput, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, RuntimeRedraw, RuntimeTaskError, RuntimeWindowSettings, run_runtime,
};
pub use runtime_input::RuntimeInputAdapter;
#[cfg(feature = "gpu")]
pub use scene_gpu::{
    PreparedSceneResources, SceneGpuBatchNode, SceneGpuBatchPassContext, SceneGpuNode,
    SceneGpuPassContext, SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer,
    SceneGpuRendererRegistry, SceneResourceEncodeContext, SceneResourceProduceError,
    SceneResourceProducer, SceneResourceProducerRegistry,
};
#[cfg(feature = "hosted")]
pub use scene_host::run_runtime_scene;
#[cfg(feature = "gpu")]
pub use scene_paint::{
    HostTextureSceneResolver, RenderTargetId, ScenePaintError, ScenePaintViewport,
    SceneWgpuPainter, resolve_background_image_url, set_background_image_url_base,
    set_resource_fetch_host,
};
pub use selection::{SelectionMove, SingleSelection};
pub use settings::{
    AppearanceSettings, BackdropTarget, SettingsError, SettingsModel, SettingsState, SettingsTab,
    SettingsTabId, WindowMaterialMode,
};
#[cfg(feature = "hosted")]
pub use settings::{hosted_window_material_modes, window_material_effect};
pub use split_pane::{SplitAxis, SplitPaneAction, SplitPaneController};
pub use theme::{
    Color, Colors, SemanticColor, SemanticPalette, ThemeMetrics, ThemeMode, ThemeModeExt,
    ThemeTokens, UI_BASE_TEXT_SIZE, UI_METRICS, install_theme_tokens,
};
#[cfg(feature = "bundled-fonts")]
pub use theme::{UI_FONT_BOLD, UI_FONT_MEDIUM, UI_FONT_REGULAR, UI_FONT_SEMIBOLD, ui_font_sources};
pub use tooltip::{TooltipConfig, TooltipPlacement};
pub use virtual_list::{
    TableColumn, TableCursor, TableNavigation, VirtualAlignment, VirtualFrozenWindow,
    VirtualListLayout, VirtualListMaterialization, VirtualListMaterializationError,
    VirtualListMaterializer, VirtualListMount, VirtualListWindow, VirtualScrollAnchor,
    VirtualTableFrozenWindow, VirtualTableLayout, VirtualTableMaterialization,
    VirtualTableMaterializer, VirtualTableWindow, VirtualTreeLayout, VirtualTreeRow,
    VirtualTreeWindow, VirtualViewport,
};
/// The exact `wgpu` the framework links. Hosts that upload their own textures
/// into the shared Device/Queue must import through this re-export: a direct
/// `wgpu` dependency silently resolves to a second copy when the framework
/// moves to a new major version.
#[cfg(feature = "gpu")]
pub use wgpu;
pub use widgets::{ButtonKind, ButtonPaintOverride, CardKind};
pub use window_chrome::{
    TitleBarDragTracker, WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState,
    WindowControlMode, apply_title_bar_pointer, title_bar_hits_window_control,
    window_commands_for_chrome_action,
};
pub use workspace::{WorkspaceAction, WorkspaceController};

pub use nana_ui_scene::DocumentAccessError;

pub use nana_ui_core::{
    FileDialogError, FileDialogKind, FileDialogRequest, FileDialogResult, FileFilter,
};
