//! Backend-neutral retained runtime for NanaUI.
//!
//! Applications and compatibility adapters use stable Nana IDs. The internal
//! node table is deliberately not part of the public contract, so changing
//! storage cannot invalidate JS handles, diagnostics, snapshots, or persisted
//! data.

#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

mod animation;
mod builtin_components;
mod calendar;
mod charts;
mod color_field;
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
mod graph_minimap;
mod image_viewer;
mod key_layers;
mod layout_engine;
mod menus;
mod mutation;
mod overlay_surfaces;
mod pane;
mod pane_section;
mod path_field;
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
mod store;
mod tabs;
mod text_editing;
mod text_layout_cache;
mod thumbnail;
mod toast;
mod tree_view;
mod video;
mod view_components;
mod workspace;
mod world;
mod xy_pad;

pub use animation::{
    AnimationDirection, AnimationFillMode, AnimationFrame, AnimationId, AnimationIteration,
    AnimationPlayState, AnimationPlayback, AnimationSample, AnimationSpec, Easing,
};
pub use builtin_components::NanaBuiltinComponents;
pub use calendar::{
    CalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapCellPaint,
    CalendarHeatmapDatum, CalendarHeatmapDayLabel, CalendarHeatmapEvent, CalendarHeatmapLabelPaint,
    CalendarHeatmapModel, CalendarHeatmapMonthLabel, CalendarHeatmapOptions, CalendarLevelResolver,
    CalendarLevelStrategy, CalendarMonthFormatter, CalendarTitleFormatter,
    build_calendar_heatmap_model, calendar_cell_fill,
};
pub use charts::{TimeSeriesChart, TimeSeriesPaint, time_series_paint};
pub use color_field::{
    ColorChanged, ColorField, ColorInput, format_hex, hsv_to_rgb, parse_hex, rgb_to_hsv,
    sanitize_rgba,
};
pub use command_palette::CommandPalette;
pub use component_registry::{
    ComponentBindKind, ComponentRegistry, ComponentTypeId, RegisterableComponent, SemanticOption,
    SemanticSpec, normalize_tag,
};
pub use components::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityDelta, AccessibilityNode,
    AccessibilityRole, AccessibilityState, AccessibilityUpdate, CalendarHoverGeometry,
    ComponentElevation, ComponentGeometry, ComponentTextRegion, ComponentTriggerSurface,
    ComputedStyle, CustomRenderNode, EventListeners, EventRoute, ExtractedNode, ExtractedTextSpan,
    ImeComposition, InteractionState, InteractionStyle, LayoutBox, LayoutInput, LineLabel,
    MeasureTextShaper, MenuSurfaceKind, ModalLayoutInput, MountState, NodeStyle, NumberSteppers,
    OverlayHostState, PointerCaptureChange, RadioIndicator, ScrollMetrics, ScrollOffset,
    ScrollbarBar, SelectMenuGeometry, SelectOptionData, SelectOptionGeometry, SemanticPaint,
    StandardVisual, TEXT_COMPLETION_MAX_CONTENT_WIDTH, TEXT_COMPLETION_PANEL_PAD,
    TEXT_COMPLETION_VISIBLE_ROWS, TEXT_HOVER_MAX_BODY_ROWS, TextCodeFold, TextCompletion,
    TextCompletionPopup, TextCompletionPopupMetrics, TextCompletionRow, TextCompletionSnapshot,
    TextContent, TextDiagnosticMark, TextDiagnosticSeverity, TextDiagnosticSpan,
    TextEditorRenderOptions, TextFoldGeometry, TextFoldGutter, TextFoldMark, TextFoldMarker,
    TextHorizontalAlignment, TextHover, TextHoverPopup, TextInputPresentation, TextInputState,
    TextMatchMark, TextMatchMarker, TextMatchSpan, TextMetrics, TextOverlayMetrics, TextSelection,
    TextShapeConstraints, TextShaper, TextShaping, TextSnippet, TextSnippetSession,
    TextVerticalAlignment, TextWhitespaceKind, TextWhitespaceMark, TooltipVisual,
};
pub use dock::{
    DOCK_DIVIDER_HIT_SIZE, DOCK_SPLIT_KEYBOARD_STEP, Dock, DockAxis, DockBoundsPersist,
    DockDropZone, DockFloatingPersist, DockFloatingSurface, DockNode, DockNodePersist, DockPanel,
    DockSurfaceSpec, DockWorkspace, DockWorkspaceEvent, DockWorkspacePersist, MAIN_SURFACE_ID,
    MAX_SPLIT_RATIO, MIN_SPLIT_RATIO, clamp_ratio, dock_nudge_split_ratio,
    dock_split_child_lengths, dock_split_ratio_from_pointer, dock_surface_window_key,
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
    Task, TextDeleteKind, UiBuilder, UiExtension, View, ViewContext, VirtualListItems,
    VirtualTableItems, VirtualTreeItems,
};
pub use glyph_cache::GlyphCache;
pub use gpu_slots::{
    GPU_TEXTURE_VIEW_RENDERER, GPU_VIEW_RENDERER, GpuTextureView, GpuView, GpuViewMode,
    GpuViewPalette, HOST_TEXTURE_RENDERER, gpu_view_params, pack_gpu_revision, unpack_gpu_revision,
};
pub use graph_canvas::{
    GRAPH_CANVAS_RENDERER, GraphCanvas, GraphCanvasAdjustment, GraphCanvasEvent, GraphEdgePaint,
    GraphInteraction, GraphNodeContent, GraphNodePaint, GraphPointerButton, GraphPortPaint,
    GraphScrollDelta, wheel_zoom_factor,
};
pub use graph_minimap::{GraphMinimap, GraphMinimapDrag, GraphMinimapEvent};
pub use image_viewer::{
    ImageViewer, ImageViewerContent, ImageViewerDrag, ImageViewerEvent, ImageViewerGeometry,
    ImageViewerHit, ImageViewerOffset, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP,
};
pub use key_layers::{
    ActionDescriptor, ActionRegistry, ActionRegistryError, CapturedStroke, KeyBinding,
    KeyCaptureEvent, KeyCaptureLayer, KeyInput, KeyModifiers, Keymap, KeymapLayer, KeymapMatch,
    KeymapState,
};
pub use layout_engine::{
    LayoutViewport, RetainedLayoutCache, RuntimeLayoutEngine, StyleLayoutNode,
};
pub use menus::{
    ActionMenuItem, AnchoredActionMenu, ContextMenu, ContextMenuEvent, ContextMenuItem,
    resolve_anchored_origin,
};
pub use mutation::{MutationQueue, UiMutation};
pub use nana_ui_core::{
    ActionId, ActionPickerNavigation, AlignSpec, CommandPaletteEvent, CommandPaletteItem,
    ContentFit, ContextPredicate, DropdownEvent, DropdownSelection, FlexDirection,
    FontFeatureSetting, FontKerningSpec, FontVariationSetting, FrameStage,
    GRAPH_EDGE_HIT_TOLERANCE, GRAPH_MAX_ZOOM, GRAPH_MIN_ZOOM, GRAPH_NODE_TITLE_HEIGHT,
    GRAPH_PORT_HIT_RADIUS, GRAPH_PORT_INSET, GRAPH_PORT_PITCH, GpuWorkObservation, GraphCanvasId,
    GraphEdge, GraphEdgeId, GraphEndpoint, GraphModel, GraphModelError, GraphNode, GraphNodeId,
    GraphPoint, GraphPort, GraphPortId, GraphPortKind, GraphPortSide, GraphRect, GraphSelection,
    GraphSize, GraphTarget, GraphTargetDescriptor, GraphTargetId, GraphTargetKind, GraphViewport,
    JustifySpec, KeyContext, LayoutStyle, LengthSpec, LineBreakSpec, PopoverAlignment,
    PopoverPlacement, PositionSpec, SemanticColorRole, StatusTone, TabDragGroup, TabDragLease,
    TabDragRect, TabDragSurface, TabDropIndicator, TabStripPaint, TableCursor, TableNavigation,
    TextAlignSpec, ThemeMode, TreeNavigation, TreeNode, TreeViewEvent, ValidationIntent,
    VirtualListLayout, VirtualListMaterialization, VirtualListMaterializationError,
    VirtualListMaterializer, VirtualListMount, VirtualListWindow, VirtualTableLayout,
    VirtualTableMaterialization, VirtualTableMaterializer, VirtualTableWindow, VirtualTreeLayout,
    VirtualTreeRow, VirtualTreeWindow, WordBreakSpec, WorkCounters, graph_node_fitted_height,
    port_tangent, tree_navigation_event,
};
pub use overlay_surfaces::{
    ConfirmDialog, ConfirmIntent, ConfirmSlots, Drawer, ModalBehavior, ModalInitialFocus,
    ModalSlots, ModalSurface, ModalSurfaceKind,
};
pub use pane::{PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode};
pub use pane_section::PaneSection;
pub use path_field::{BrowseRequested, PathField};
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
    RovingFocusIntent, RovingFocusPolicy, SegmentedControl, SegmentedOption,
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
    sidebar_row_depth_inset, sidebar_row_tool_button, sidebar_section_tool_button,
    sidebar_top_bar_tool_button,
};
pub use split_pane::SplitPane;
pub use tabs::{TabOption, Tabs, TabsEvent};
pub use text_editing::{
    OCCURRENCE_HIGHLIGHT_LIMIT, TextCaretIntent, TextLineDirection, TextSearchOptions,
    find_matches, find_matches_capped, find_next_match, find_previous_match,
    matching_bracket_pair, occurrence_query_at, replace_all_matches, sort_lines,
};
pub use thumbnail::{DEFAULT_ASPECT as THUMBNAIL_DEFAULT_ASPECT, Thumbnail, ThumbnailState};
pub use toast::{Toast, ToastDismissed, ToastTone};
pub use tree_view::TreeView;
pub use video::Video;
pub use view_components::{
    Activate, Button, Card, Checkbox, CodeEditing, ComponentView, Dialog, Divider, HostedTextarea,
    IconButton, IconButtonTooltip, IconGlyph, List, ListItem, ListItemSlots, NumberChanged,
    NumberInput, OverlayChanged, OverlayHost, RangeAdjustment, RangeChanged, RangeDragState,
    RangeField, ScrollAxes, ScrollChanged, ScrollView, ScrollbarDragState, SecondaryPress,
    SliderError, Stack, Switch, Table, TableCell, TableCellFocused, TableRow, Text, TextArea,
    TextChanged, TextInput, ToggleChanged, Tooltip,
};
pub use workspace::{Workspace, WorkspaceRegionSlot, WorkspaceResizeHandle};
pub use world::{
    CommitReport, DocumentId, NodeKind, NodeSnapshot, StableNodeId, UiWorld, UiWorldError,
};
pub use xy_pad::{
    XYPad, XYPadAdjustment, XYPadAxisLock, XYPadDragState, XYPadEvent, XYPadValue, xy_pad_height,
};
