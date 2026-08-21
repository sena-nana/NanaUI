//! Backend-neutral retained runtime for NanaUI.
//!
//! Applications and compatibility adapters use stable Nana IDs. The internal
//! generational entity representation is deliberately not part of the public
//! contract, so changing ECS implementations cannot invalidate JS handles,
//! diagnostics, snapshots, or persisted data.

#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

mod animation;
mod builtin_components;
mod calendar;
mod charts;
mod command_palette;
mod component_registry;
mod components;
mod dock;
mod dropdown;
mod feedback;
mod form_surfaces;
mod framework;
mod glyph_cache;
mod gpu_slots;
mod graph_canvas;
mod image_viewer;
mod key_layers;
mod layout_engine;
mod menus;
mod mutation;
mod overlay_surfaces;
mod pane;
mod placeholders;
mod popover;
mod presentation;
mod profiler;
mod qr_code;
mod query;
mod reorder_list;
mod rich_text;
mod schedule;
mod search_dropdown;
mod select;
mod selection;
mod settings;
mod shell;
mod sidebar;
mod split_pane;
mod tabs;
mod text_layout_cache;
mod toast;
mod tree_view;
mod view_components;
mod workspace;
mod world;
mod xy_pad;

pub use animation::{AnimationFrame, AnimationId, AnimationSample, AnimationSpec, Easing};
pub use builtin_components::NanaBuiltinComponents;
pub use calendar::{
    CalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapCellPaint,
    CalendarHeatmapDatum, CalendarHeatmapDayLabel, CalendarHeatmapEvent, CalendarHeatmapLabelPaint,
    CalendarHeatmapModel, CalendarHeatmapMonthLabel, CalendarHeatmapOptions, CalendarLevelResolver,
    CalendarLevelStrategy, CalendarMonthFormatter, CalendarTitleFormatter,
    build_calendar_heatmap_model, calendar_cell_fill,
};
pub use charts::{TimeSeriesChart, TimeSeriesPaint, time_series_paint};
pub use command_palette::CommandPalette;
pub use component_registry::{
    ComponentBindKind, ComponentRegistry, ComponentTypeId, RegisterableComponent, SemanticSpec,
};
pub use components::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityDelta, AccessibilityNode,
    AccessibilityRole, AccessibilityState, AccessibilityUpdate, CalendarHoverGeometry,
    ComponentElevation, ComponentGeometry, ComponentTextRegion, ComputedStyle, CustomRenderNode,
    EventListeners, EventRoute, ExtractedNode, ExtractedTextSpan, ImeComposition, InteractionState,
    InteractionStyle, LayoutBox, LayoutInput, MeasureTextShaper, MenuSurfaceKind, ModalLayoutInput,
    MountState, NodeStyle, OverlayHostState, PointerCaptureChange, ScrollMetrics, ScrollOffset,
    SelectMenuGeometry, SelectOptionData, SelectOptionGeometry, SemanticPaint, StandardVisual,
    TextContent, TextHorizontalAlignment, TextInputPresentation, TextInputState, TextMetrics,
    TextSelection, TextShapeConstraints, TextShaper, TextShaping, TextVerticalAlignment,
    TooltipVisual,
};
pub use dock::{
    Dock, DockAxis, DockDropZone, DockFloatingSurface, DockNode, DockPanel, DockSurfaceSpec,
    DockWorkspace, DockWorkspaceEvent, MAIN_SURFACE_ID, dock_surface_window_key,
};
pub use dropdown::{Dropdown, DropdownOption};
pub use feedback::{
    EmptyState, LabeledValue, Progress, ProgressCancelled, Spinner, StatusBadge, ValidationMessage,
    ValueEmphasis,
};
pub use form_surfaces::{FormField, InteractiveCard};
pub use framework::{
    ActiveRuntimeOverlay, AppContext, AssemblyScope, Entity, ExtensionRegistrar, FrameworkError,
    OverlayKey, OverlayPointerDecision, OverlayPointerPhase, RuntimeOverlayKind, Subscription,
    Task, UiExtension, View, ViewContext, VirtualListItems, VirtualTableItems, VirtualTreeItems,
};
pub use glyph_cache::GlyphCache;
pub use gpu_slots::{
    GPU_TEXTURE_VIEW_RENDERER, GPU_VIEW_RENDERER, GpuTextureView, GpuView, GpuViewMode,
    GpuViewPalette, RenderSlot, pack_gpu_revision, unpack_gpu_revision,
};
pub use graph_canvas::{
    GRAPH_CANVAS_RENDERER, GraphCanvas, GraphCanvasAdjustment, GraphCanvasEvent, GraphEdgePaint,
    GraphInteraction, GraphNodePaint, GraphPointerButton, GraphPortPaint, GraphScrollDelta,
    wheel_zoom_factor,
};
pub use image_viewer::{
    HOST_TEXTURE_RENDERER, ImageViewer, ImageViewerContent, ImageViewerDrag, ImageViewerEvent,
    ImageViewerGeometry, ImageViewerHit, ImageViewerOffset, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP,
};
pub use key_layers::{
    ActionDescriptor, ActionRegistry, ActionRegistryError, CapturedStroke, KeyBinding,
    KeyCaptureEvent, KeyCaptureLayer, KeyInput, KeyModifiers, Keymap, KeymapLayer, KeymapMatch,
    KeymapState,
};
pub use layout_engine::{LayoutViewport, RuntimeLayoutEngine};
pub use menus::{
    ActionMenuItem, AnchoredActionMenu, ContextMenu, ContextMenuEvent, ContextMenuItem,
    resolve_anchored_origin,
};
pub use mutation::{MutationQueue, UiMutation};
pub use nana_ui_core::{
    ActionId, ActionPickerNavigation, AlignSpec, CommandPaletteEvent, CommandPaletteItem,
    ContentFit, ContextPredicate, DropdownEvent, DropdownSelection, FlexDirection, FrameStage,
    GRAPH_EDGE_HIT_TOLERANCE, GRAPH_MAX_ZOOM, GRAPH_MIN_ZOOM, GRAPH_PORT_HIT_RADIUS,
    GpuWorkObservation, GraphCanvasId, GraphEdge, GraphEdgeId, GraphEndpoint, GraphModel,
    GraphModelError, GraphNode, GraphNodeId, GraphPoint, GraphPort, GraphPortId, GraphPortKind,
    GraphPortSide, GraphRect, GraphSelection, GraphSize, GraphTarget, GraphTargetDescriptor,
    GraphTargetId, GraphTargetKind, GraphViewport, JustifySpec, KeyContext, LayoutStyle,
    LengthSpec, PopoverAlignment, PopoverPlacement, PositionSpec, SemanticColorRole, StatusTone,
    TabDragGroup, TabDragLease, TabDragRect, TabDragSurface, TabDropIndicator, TabStripPaint,
    TableCursor, TableNavigation, ThemeMode, TreeNavigation, TreeNode, TreeViewEvent,
    ValidationIntent, VirtualListLayout, VirtualListMaterialization,
    VirtualListMaterializationError, VirtualListMaterializer, VirtualListMount, VirtualListWindow,
    VirtualTableLayout, VirtualTableMaterialization, VirtualTableMaterializer, VirtualTableWindow,
    VirtualTreeLayout, VirtualTreeRow, VirtualTreeWindow, WorkCounters, port_tangent,
    tree_navigation_event,
};
pub use overlay_surfaces::{
    ConfirmDialog, ConfirmIntent, ConfirmSlots, Drawer, ModalBehavior, ModalInitialFocus,
    ModalSlots, ModalSurface, ModalSurfaceKind,
};
pub use pane::{PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode};
pub use placeholders::{LevelMeter, Skeleton};
pub use popover::{ActionMenu, Popover, PopoverClosed, PopoverToggled, resolve_popover_origin};
pub use presentation::{
    HIGHLIGHT_PRESENTER, HighlightRequest, TextPresentation, TextPresenter, TextSpan,
};
#[cfg(feature = "syntax-highlighting")]
pub use presentation::{HighlightPresentation, SyntectHighlighter};
pub use profiler::{FrameProfile, FrameProfiler, StageStatus, StageTiming};
pub use qr_code::{QrCode, QrCodeError};
pub use reorder_list::{
    ReorderItem, ReorderList, ReorderListEvent, ReorderListPointer, ReorderRowPaint,
    TreeDropIntent, TreeDropPosition,
};
pub use rich_text::{
    MarkdownBlock, MarkdownBlockKind, MarkdownImage, MarkdownSpan, MarkdownTable,
    MarkdownTableAlignment, NativeMarkdown, RichSpan, RichTextEvent, SelectableRichText,
    TextSelectionGroup, TextSelectionGroupId, TextSelectionSnapshot,
};
pub use schedule::SystemWork;
pub use search_dropdown::{SearchDropdown, SearchDropdownEvent, SearchDropdownOption};
pub use select::{Select, SelectChanged, SelectOption};
pub use selection::{
    Radio, RadioGroup, RovingFocusIntent, RovingFocusPolicy, SegmentedControl, SegmentedOption,
    SegmentedSelectionRequested, SelectionChrome, SelectionOrientation,
};
pub use settings::{
    AboutMetadata, AboutSection, AboutSectionAssembly, AppearanceSection,
    AppearanceSectionAssembly, SettingsBack, SettingsCard, SettingsCollapsibleCard, SettingsPage,
    SettingsPageAssembly, SettingsRow, SettingsSidebar, SettingsSidebarAssembly,
    SettingsTabSelected, apply_appearance_event,
};
pub use shell::{AppShell, AppTitleBar, AppTitleBarControls, DesktopShell, WindowChromeAction};
pub use sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowIcon, SidebarRowState,
    SidebarRowTone, SidebarSection, SidebarSectionSlots, SidebarSectionState,
    sidebar_row_depth_inset,
};
pub use split_pane::SplitPane;
pub use tabs::{TabOption, Tabs, TabsEvent};
pub use toast::{Toast, ToastDismissed, ToastTone};
pub use tree_view::TreeView;
pub use view_components::{
    Activate, Button, Card, Checkbox, ComponentView, Dialog, HostedTextarea, IconButton,
    IconButtonTooltip, List, ListItem, ListItemSlots, Menu, MenuItem, OverlayChanged, OverlayHost,
    RangeAdjustment, RangeChanged, RangeDragState, RangeField, ScrollAxes, ScrollChanged,
    ScrollView, Slider, SliderChanged, SliderError, Switch, Tab, TabList, TabSelected, Table,
    TableCell, TableCellFocused, TableRow, Text, TextArea, TextChanged, TextInput, ToggleChanged,
    Tooltip,
};
pub use workspace::{Workspace, WorkspaceRegionSlot, WorkspaceResizeHandle};
pub use world::{
    CommitReport, DocumentId, NodeKind, NodeSnapshot, StableNodeId, UiWorld, UiWorldError,
};
pub use xy_pad::{
    XYPad, XYPadAdjustment, XYPadAxisLock, XYPadDragState, XYPadEvent, XYPadValue, xy_pad_height,
};
