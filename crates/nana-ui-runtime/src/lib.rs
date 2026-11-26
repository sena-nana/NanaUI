//! Backend-neutral retained runtime for NanaUI.
//!
//! Applications and hosts use stable Nana IDs. The internal
//! node table is deliberately not part of the public contract, so changing
//! storage cannot invalidate held handles, diagnostics, snapshots, or persisted
//! data.

#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

mod animation;
mod app_menu_bar;
#[cfg(feature = "area-layout")]
mod area_layout;
mod avatar;
mod bars;
mod breadcrumb;
#[cfg(feature = "calendar")]
mod calendar;
#[cfg(feature = "charts")]
mod charts;
mod chip;
mod color_field;
mod size_field;
pub use size_field::{SizeField, SizeFieldChanged, SizeFieldSubmitted, SizePreset};
mod command_palette;
pub mod component_descriptors;
mod component_registry;
mod components;
mod custom_paint;
#[cfg(feature = "paint-script")]
mod paint_script;
#[cfg(feature = "property-grid")]
mod property_grid;
mod range_field;
mod scrub_field;
mod theme_intent;
/// The BCP 47 tag [`MutationQueue::set_language`] and
/// [`UiWorld::set_default_language`] take.
pub use nana_text::font::LanguageTag;
#[cfg(feature = "paint-script")]
pub use paint_script::PaintScript;
mod data_table;
mod date_picker;
#[cfg(feature = "diff-view")]
mod diff_view;
#[cfg(feature = "dock")]
mod dock;
pub mod drag_overlay;
mod dropdown;
mod dynamic_form;
mod feedback;
mod file_tab;
mod find_replace;
mod focus;
mod form_surfaces;
mod framework;
mod gpu_slots;
#[cfg(feature = "graph-canvas")]
mod graph_canvas;
#[cfg(feature = "graph-canvas")]
mod graph_minimap;
mod hover_card;
#[cfg(feature = "image-viewer")]
mod image_viewer;
mod inline_content;
mod inline_flow;
mod interaction_request_card;
pub mod intrinsic_cache;
mod key_layers;
mod layout_edits;
mod layout_engine;
mod layout_frontier;
mod layout_node;
mod stack;
pub use inline_content::{
    InlineAtomic, InlineContent, InlineContentBuilder, InlineItem, InlineTextSemantics,
    MixedInlineContent,
};
pub use inline_flow::InlineFlow;
mod layout_result;
mod list_selection;
mod sticky;
/// Benchmark-only coarse phase clocks for `--profile-layout`. How much work
/// layout did, plan reuse included, is on [`WorkCounters`].
#[cfg(feature = "benchmark")]
pub use layout_engine::plan_stats;
pub use sticky::{StickyAncestor, StickyBoxes, StickyDependencies, sticky_translation};
mod browser_view;
pub mod fold;
mod menus;
pub mod modifier_hold;
mod motion_api;
mod mutation;
mod native_content;
pub use browser_view::BrowserView;
mod i18n;
#[cfg(feature = "rich-text")]
mod markdown_diagram;
#[cfg(feature = "rich-text")]
mod markdown_drawing;
mod media_transport;
mod overlay_surfaces;
mod overlay_visibility;
mod pane;
mod pane_section;
mod panel;
mod path_field;
mod path_text;
pub use path_text::{PathText, PathTextVisual};
mod curve_field;
mod placeholders;
mod popover;
mod presence;
mod presentation;
mod profiler;
mod qr_code;
mod query;
mod range_span;
mod responsive;
mod submenu_aim;
#[cfg(feature = "icu")]
pub use i18n::IcuFormatter;
pub use i18n::{
    DateStyle, FluentCatalog, FluentParseError, FluentResource, FormatterCounts, Locale,
    LocaleFormatter, LocalizedResolver, LocalizedText, MessageArg, MessageArgs, MessageCatalog,
    MessageDateTime, MessageId, MessagePattern, MessagePatternError, MessageTable, MissingMessage,
    NumberStyle, PlainFormatter, PluralCategory,
};
pub use responsive::{
    MAX_RESPONSIVE_BREAKPOINTS, ResponsiveAxis, ResponsiveContainer, ResponsiveRule, StyleVariant,
};
#[cfg(feature = "controls")]
mod reorder_list;
#[cfg(feature = "rich-text")]
pub use markdown_diagram::DiagramDrawing;
#[cfg(feature = "rich-text")]
pub use markdown_drawing::{MarkdownDrawing, MarkdownDrawingCommand};
#[cfg(feature = "rich-text")]
mod rich_text;
mod rich_text_editor;
mod rich_text_view;
pub use rich_text_editor::{
    RichAttrMask, RichClipboardCommand, RichEditCommand, RichSelectionAttrs, RichTextEditor,
    RichTextEditorEvent, RichTypingStylePolicy,
};
pub use rich_text_view::RichTextView;
mod schedule;
mod search_dropdown;
mod select;
mod selection;
mod settings;
mod shell;
mod sidebar;
mod snippet;
mod split_pane;
pub mod spotlight;
pub use snippet::{
    SnippetExpansion, SnippetPlaceholder, SnippetTransform, expand_text_snippet,
    expand_text_snippet_with_variables,
};
mod media_time;
mod node_space;
mod presentation_clock;
mod store;
mod tabs;
#[cfg(feature = "terminal")]
mod terminal;
mod text_editing;
mod text_engine_shaper;
mod text_input_limit;
mod text_node;
#[cfg(any(test, feature = "benchmark"))]
pub mod text_shape_stats;
mod text_width;
mod thumbnail;
#[cfg(feature = "timeline")]
mod timeline;
mod toast;
mod toast_stack;
mod tree_view;
mod video;
pub mod view;
mod view_components;
mod widget_visual;
mod workspace;
mod world;
mod xy_pad;

pub use animation::{
    AnimatableProperty, AnimationClass, AnimationDirection, AnimationEvent, AnimationEventKind,
    AnimationFillMode, AnimationFrame, AnimationId, AnimationIteration, AnimationPlayState,
    AnimationPlayback, AnimationSample, AnimationSpec, CompiledMotion, DecayParams, Easing,
    FlipRect, Keyframe, MOTION_DESCRIPTOR_VERSION, MotionCodecError, MotionCodecId,
    MotionCodecInfo, MotionCodecRegistry, MotionCurve, MotionDescriptor, MotionDescriptorError,
    MotionDescriptorStore, MotionEvaluatorBackend, MotionGraph, MotionHandle, MotionInspectorEntry,
    MotionInterrupt, MotionLayer, MotionSample, MotionTargetId, MotionTiming, MotionTo,
    MotionTrack, MotionTrackId, MotionValue, MotionValueKind, MotionWorkCounters,
    PresentationOverlay, PresentationPair, PresentationSlot, PresentationStore, Spring,
    SpringParams, StepJump, Timeline, classify_animatable_property, compile_motion_descriptor,
    component_animation_id, component_animation_kinds, cpu_fallback_reason, decode_motion_track,
    evaluate_descriptor, evaluate_progress, evaluate_track, evaluate_track_at,
    invert_flip_translate, is_font_variation_settings, layout_flip_hold_spec,
    layout_flip_play_spec, layout_flip_spec, loading_animation, retarget_track,
    shared_element_morph_specs, toast_timeout_animation, track_completion_deadline,
    workspace_animation,
};
pub use app_menu_bar::{AppMenuBar, MenuBarActivated, ShortcutNotation, menu_shortcut_label};
#[cfg(feature = "area-layout")]
pub use area_layout::{
    Area, AreaAxis, AreaChange, AreaChrome, AreaEditorSelect, AreaGeometry, AreaLayout,
    AreaLayoutChanged, AreaLayoutError, AreaLayoutModel, AreaNode, AreaSplitter,
};
pub use avatar::{Avatar, DEFAULT_SIZE as AVATAR_DEFAULT_SIZE};
pub use bars::{StatusBar, ToolGroup, ToolGroupMode, Toolbar};
pub use breadcrumb::{
    Breadcrumb, BreadcrumbEvent, BreadcrumbItem, BreadcrumbSegment, BreadcrumbTone,
};
#[cfg(feature = "calendar")]
pub use calendar::{
    CalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapCellPaint,
    CalendarHeatmapDatum, CalendarHeatmapDayLabel, CalendarHeatmapEvent, CalendarHeatmapLabelPaint,
    CalendarHeatmapLayout, CalendarHeatmapModel, CalendarHeatmapMonthLabel, CalendarHeatmapOptions,
    CalendarHeatmapVisual, CalendarHoverGeometry, CalendarLevelResolver, CalendarLevelStrategy,
    CalendarMonthFormatter, CalendarTitleFormatter, build_calendar_heatmap_model,
    build_calendar_heatmap_model_in, calendar_cell_fill,
};
#[cfg(feature = "charts")]
pub use charts::{
    Chart, ChartEvent, ChartHoverVisual, ChartSpec, ChartTooltip, ChartTooltipVisual, ChartVisual,
};
pub use chip::{Chip, ChipDismissed};
pub use color_field::{
    ColorChanged, ColorField, ColorInput, ColorPickRequested, ColorSubmitted, format_hex,
    hsv_to_rgb, hue_spectrum, parse_hex, rgb_to_hsv, sanitize_rgba,
};
pub use command_palette::{
    CommandPalette, CommandPaletteLayout, CommandPaletteVisual, PaletteRowData, PaletteRowGeometry,
};
pub use component_registry::{
    ComponentRegistry, ComponentTypeId, RegisterableComponent, normalize_tag,
};
pub use components::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityDelta, AccessibilityLive,
    AccessibilityNode, AccessibilityPlacements, AccessibilityRole, AccessibilityScrollAxis,
    AccessibilityScrollDirection, AccessibilityScrollUnit, AccessibilityState, AccessibilityUpdate,
    ComputedStyle, CustomRenderNode, DocumentTextSelection, EventListeners, EventRoute, FontStrut,
    ImeComposition, ImeView, InheritedStyle, InheritedStyleExt, InteractionState, InteractionStyle,
    LayoutBox, LayoutInput, LineLabel, MeasureTextShaper, MenuItemMark, ModalLayoutInput,
    MountState, NodeStyle, RangeCaution, RangeValueThumb, ScrollMetrics, ScrollOffset,
    SelectMenuDrag, SelectMenuGeometry, SelectMenuRoom, SelectMenuScroll, SelectOptionData,
    SelectOptionGeometry, SemanticPaint, TextAffinity, TextAtomChip, TextAtomClosed, TextAtomSpan,
    TextCodeFold, TextColorSwatchSpan, TextCompletion, TextCompletionEdit, TextCompletionPopup,
    TextCompletionRow, TextCompletionSnapshot, TextContent, TextDiagnosticSeverity,
    TextDiagnosticSpan, TextEditorRenderOptions, TextFoldGeometry, TextFoldGutter,
    TextGitGutterGeometry, TextGitMark, TextGitMarkKind, TextHit, TextHorizontalAlignment,
    TextHover, TextHoverPopup, TextInlay, TextInputCharacterCounter, TextInputGeometry,
    TextInputScroll, TextInputState, TextInputSuffix, TextInputView, TextInputVisual,
    TextMatchMarker, TextMatchSpan, TextMetrics, TextMinimapGeometry, TextSelection,
    TextShapeConstraints, TextShaper, TextShaping, TextSignatureHelp, TextSignaturePopup,
    TextSnippet, TextStickyLineGeometry, TextValue, TextVerticalAlignment, TextWhitespaceKind,
    TooltipVisual,
};
pub use curve_field::{
    CurveChanged, CurveDragState, CurveDragging, CurveField, CurveInput, CurvePoint,
    curve_plot_inset, curve_point_extent,
};
pub use custom_paint::{
    AFFINE_IDENTITY, Affine, BlendMode, BoxPaint, ColorStop, CornerRadii, FillRule, Gradient,
    GradientColorSpace, GradientExtend, GradientShape, ImageFit, LineCap, LineJoin,
    MAX_PATH_COORDINATE, MaskKind, NodePainter, PATH_FLATTEN_TOLERANCE, Paint, PaintColor,
    PaintContext, PaintImage, PaintOp, PaintPath, PaintPattern, PaintPhase, PaintRecording,
    PaintShadow, PaintState, PaintText, PaintTextDecoration, PaintTextLineMetrics,
    PaintTextMetrics, PaintTextShadow, PaintTextStroke, Painter, PathBooleanOp, PathVerb,
    PatternRepeat, Radius, ResolvedGradient, ResolvedPaint, ResolvedPaintTextDecoration,
    ResolvedPaintTextEffects, ResolvedPaintTextStroke, ResolvedPattern, StrokeStyle, TextureRegion,
    affine_multiply,
};
pub use data_table::{
    TABLE_RESIZE_HANDLE, TableColumnResized, TableSelectionRequested, selection_modifiers,
};
pub use date_picker::{DateChanged, DateCursorMoved, DatePicker, DatePickerSlots};
#[cfg(feature = "diff-view")]
pub use diff_view::{DiffEvent, DiffHunk, DiffLayout, DiffLine, DiffLineKind, DiffView};
#[cfg(feature = "dock")]
pub use dock::{
    DOCK_DIVIDER_HIT_SIZE, DOCK_SPLIT_KEYBOARD_STEP, Dock, DockAxis, DockBoundsPersist,
    DockCommand, DockCommandOutcome, DockDropZone, DockFloatingSurface, DockItemLimits, DockNode,
    DockPanel, DockSurfaceSpec, DockWorkspace, DockWorkspaceEvent, MAIN_SURFACE_ID,
    MAX_SPLIT_RATIO, MIN_SPLIT_RATIO, clamp_ratio, dock_nudge_split_ratio,
    dock_split_child_lengths, dock_split_ratio_from_pointer, dock_surface_window_key,
};
pub use drag_overlay::{
    CrossWindowDragHandoff, CursorFollower, CursorFollowerPlacement, DragOverlay,
    DragSourcePresentation, DropZone, DropZoneConfig, DropZoneDecision, DropZoneRegistry,
    PendingDrop,
};
pub use dropdown::{Dropdown, DropdownOption};
pub use dynamic_form::{DynamicForm, DynamicFormEvent, DynamicFormField, DynamicFormOption};
pub use feedback::{
    EmptyState, EmptyStateGeometry, EmptyStateVisual, LabeledValue, LabeledValueVisual, Progress,
    ProgressCancelled, ProgressMode, ProgressVisual, Spinner, SpinnerVisual, StatusBadge,
    StatusBadgeVisual, ValidationMessage, ValidationMessageVisual, ValueEmphasis,
};
pub use file_tab::{FileTab, FileTabEvent};
pub use find_replace::{FindReplaceBar, FindReplaceEvent, TextSearchBar};
pub use focus::{DirectionalFocusScope, FocusCounters, FocusNeighbors, FocusPolicy, FocusScope};
pub use form_surfaces::{FormField, FormFieldVisual, InteractiveCard};
pub use framework::MountedView;
pub(crate) use framework::UiBuilder;
pub use framework::{
    ASSEMBLY_PATH_SEPARATOR, ActiveDragSession, ActiveRuntimeOverlay, AppContext, AssemblyScope,
    DragSource, Entity, ExtensionRegistrar, FormValidity, FrameworkError, HeadlessInput,
    InputBindError, InputCounters, InputRouteError, InputRouteOutcome, NodePointer,
    NodePointerHooks, NodeWheel, OverlayKey, OverlayPointerDecision, OverlayPointerPhase,
    RoutedEvent, RuntimeOverlayKind, ScrollPhysics, Task, TextDeleteKind, TextEditAction,
    TextEditOrigin, TextFindScope, UiExtension, View, ViewContext, VirtualListItems,
    VirtualTableItems, VirtualTreeItems,
};
pub use gpu_slots::{
    GPU_TEXTURE_VIEW_RENDERER, GPU_VIEW_RENDERER, GpuTextureView, GpuView, GpuViewMode,
    GpuViewPalette, HOST_TEXTURE_RENDERER, TextureSlotName, gpu_view_params, pack_gpu_revision,
    unpack_gpu_revision,
};
#[cfg(feature = "graph-canvas")]
pub use graph_canvas::{
    GRAPH_CANVAS_RENDERER, GraphCanvas, GraphCanvasAdjustment, GraphCanvasEvent, GraphCanvasHit,
    GraphCanvasLayout, GraphCanvasVisual, GraphEdgePaint, GraphInteraction, GraphNodeContent,
    GraphNodePaint, GraphPointerButton, GraphPortPaint, GraphScrollDelta, wheel_zoom_factor,
};
#[cfg(feature = "graph-canvas")]
pub use graph_minimap::{
    GraphMinimap, GraphMinimapDrag, GraphMinimapEvent, GraphMinimapLayout, GraphMinimapVisual,
};
pub use hover_card::HoverCard;
#[cfg(feature = "image-viewer")]
pub use image_viewer::{
    ImageViewer, ImageViewerContent, ImageViewerDrag, ImageViewerEvent, ImageViewerGeometry,
    ImageViewerHit, ImageViewerLayout, ImageViewerOffset, ImageViewerPosition, ImageViewerVisual,
    ZOOM_MAX, ZOOM_MIN, ZOOM_STEP,
};
pub use interaction_request_card::{InteractionRequestCard, InteractionRequestField};
pub use intrinsic_cache::{
    Baseline, ConstraintClass, FormattingContextId, IntrinsicCacheCounters, IntrinsicCacheKey,
    IntrinsicGeneration, IntrinsicMetrics, InvalidationReason, MaxBlockInput, PassIntrinsicFacts,
    UsedSize,
};
pub use key_layers::{
    ActionDescriptor, ActionMatch, ActionRegistry, ActionRegistryError, CONTEXT_IME_COMPOSING,
    CONTEXT_MODAL, CONTEXT_TEXT_INPUT, CapturedStroke, KeyBinding, KeyCaptureEvent,
    KeyCaptureLayer, KeyCaptureLayerLayout, KeyCaptureLayerVisual, KeyModifiers, Keymap,
    KeymapLayer, KeymapLayerLayout, KeymapLayerVisual, KeymapMatch, KeymapState, MenuActionMap,
    MenuActionMapError,
};
pub use layout_engine::{
    LayoutSliceBudget, LayoutViewport, RetainedLayoutCache, RuntimeLayoutEngine, StyleLayoutNode,
};
pub use layout_frontier::{
    LayoutDependencyGraph, LayoutFrontier, LayoutFrontierSeed, LayoutFrontierStats,
};
pub use layout_node::{
    ExplicitRect, FormattingContextKind, LayoutContentKind, LayoutNode, ParticipationKind,
};
pub use layout_result::{
    LayoutChildPlacement, LayoutFragment, LayoutFragmentKind, LayoutFrame, LayoutPart,
    LayoutPartKind, LayoutResult, LayoutResultSource,
};
pub use list_selection::ListSelectionRequested;
pub use media_time::{MediaPacing, MediaSampleRequest, MediaTimeMapping};
pub use media_transport::{
    MediaTransportBar, MediaTransportDensity, MediaTransportEvent, MediaTransportIcons,
    MediaTransportPlacement, MediaTransportSlots, media_clock,
};
pub use menus::{
    ActionMenuItem, ActionMenuItemGeometry, ActionMenuItemVisual, AnchoredActionMenu,
    AnchoredMenuDismissed, ContextMenu, ContextMenuEvent, ContextMenuItem, resolve_anchored_origin,
};
pub use modifier_hold::{
    ModifierHoldConfig, ModifierHoldState, ModifierHoldTracker, ModifierHoldTransition, ModifierKey,
};
pub use motion_api::{
    FlipBuilder, FoldBuilder, NodeMotion, SharedElementBuilder, SpringBuilder, TimelineBuilder,
    TransitionBuilder,
};
pub use mutation::{MutationQueue, UiMutation};
/// The chart model: options, layout and the GPU mark contract.
#[cfg(feature = "charts")]
pub use nana_ui_charts as chart;
pub use nana_ui_core::FrameworkStrings;
pub use nana_ui_core::{
    ActionId, ActionPickerNavigation, AlignSpec, AnchoredMenuPlacement, AppearanceEvent,
    ClipCircle, ClipEllipse, ClipInset, ClipPath, ClipPoint, ClipShapeRadius, CommandPaletteEvent,
    CommandPaletteItem, CompiledTheme, ContentFit, ContextPredicate, ControlSize, CursorSpec,
    CustomCursorError, CustomCursorSpec, CustomCursorVariant, CustomCursorVector, DataFormat,
    DataFormatError, DataTransfer, DirSpec, DisplaySpec, DragCancelReason, DragSessionId,
    DrawerSide, DropAccepts, DropEffect, DropEffects, DropKind, DropdownEvent, DropdownSelection,
    DynamicLayoutCounters, ElevationRole, ExpansionState, FileDragKind, FlexDirection, FlexWrap,
    FontFeatureSetting, FontKerningSpec, FontVariationSetting, FrameStage,
    GRAPH_EDGE_HIT_TOLERANCE, GRAPH_MAX_ZOOM, GRAPH_MIN_ZOOM, GRAPH_NODE_TITLE_HEIGHT,
    GRAPH_PORT_HIT_RADIUS, GRAPH_PORT_INSET, GRAPH_PORT_PITCH, GpuWorkObservation, GraphCanvasId,
    GraphEdge, GraphEdgeId, GraphEndpoint, GraphModel, GraphModelError, GraphNode, GraphNodeId,
    GraphPoint, GraphPort, GraphPortId, GraphPortKind, GraphPortSide, GraphRect, GraphSelection,
    GraphSize, GraphTarget, GraphTargetDescriptor, GraphTargetId, GraphTargetKind, GraphViewport,
    GridLine, GridPlacement, GridRepeatAuto, GridStyle, GridTemplateAreas, GridTrack,
    GridTrackListUnsupported, HitTestBehavior, I18nCounters, Icon, IconData, IconPathCommand,
    IconShape, ImageSampling, InputRegionOutcome, InputRegionRect, InputRegionSnapshot,
    InternalPayloadToken, JustifySpec, KeyContext, LayoutStyle, LazyFnProvider, LengthAtom,
    LengthSpec, LineBreakSpec, LineHeightSpec, NativeHitTestClass, OutputWorkObservation,
    OverflowSpec, OverlayMount, PaintTransform, PhysicalEdge, PointerEventsSpec, PopoverAlignment,
    PopoverPlacement, PositionSpec, RadiusTier, ScopeGeneration, SemanticColorMix,
    SemanticColorRole, ShapeRecipe, ShapeRole, SharedElementKey, SharedElementSizeMode,
    SharedElementSnapshot, SharedElementTransition, SplitPaneModel, SplitPaneMutation, StatusTone,
    SwitchControlPosition, SystemCursor, TITLE_BAR_HEIGHT, TabDragGroup, TabDragLease, TabDragRect,
    TabDragSurface, TabDropIndicator, TabStripPaint, TableCursor, TableNavigation, TextAlignSpec,
    TextShadowSpec, ThemeAppearance, ThemeCategoryGenerations, ThemeDependencyMask, ThemeId,
    ThemeMetricsPatch, ThemePackage, ThemePackageError, ThemePatch, ThemeRecipeFamily,
    ThemeRegistry, ThemeScopeId, ThemeWorkCounters, TransferAction, TransferCachePolicy,
    TransferEntryMeta, TransferError, TransferImage, TransferLifetime, TransferOrigin,
    TransferProvider, TransformOrigin, TreeNavigation, TreeNode, TreeOrder, TreeViewEvent,
    UserSelectSpec, ValidationIntent, VerticalAlignSpec, ViewportAxis, VirtualAlignment,
    VirtualFrozenWindow, VirtualListLayout, VirtualListMaterialization,
    VirtualListMaterializationError, VirtualListMaterializer, VirtualListMount, VirtualListWindow,
    VirtualScrollAnchor, VirtualTableFrozenWindow, VirtualTableLayout, VirtualTableMaterialization,
    VirtualTableMaterializer, VirtualTableWindow, VirtualTreeLayout, VirtualTreeRow,
    VirtualTreeWindow, VirtualViewport, VisibilitySpec, WINDOW_CONTROL_GAP, WINDOW_CONTROL_WIDTH,
    WORKSPACE_REGION_TRANSITION_DURATION, WhiteSpaceSpec, WindowInteractionCapabilities,
    WindowResizeEdge, WordBreakSpec, WorkCounters, WorkspaceModel, WorkspaceMutation,
    WritingContext, WritingModeSpec, custom_window_controls_width, file_uri_to_path,
    format_uri_list, graph_node_fitted_height, parse_uri_list, path_to_file_uri, port_tangent,
    space, tree_navigation_event, type_scale,
};
pub use nana_ui_core::{
    KeyedOrder, SelectionGesture, SelectionMode, SelectionModifiers, SelectionMove, SelectionOrder,
    SelectionState, SelectionStep, SingleSelection, TableColumn, keyed_order,
};
pub use nana_ui_input::{FocusDirection, NavigationInput, NavigationIntent, NavigationSource};
pub use overlay_surfaces::{
    ConfirmDialog, ConfirmIntent, ConfirmSlots, Drawer, ModalBackdrop, ModalBackdropVisual,
    ModalBehavior, ModalFrameGeometry, ModalFrameVisual, ModalInitialFocus, ModalSlots,
    ModalSurface, ModalSurfaceKind, ModalSurfaceSections,
};
pub use overlay_visibility::{
    OVERLAY_IDLE, OverlayLocks, OverlayVisibility, OverlayVisibilityChanged,
    OverlayVisibilityConfig,
};
pub use pane::{
    PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneSlot, PaneTree, PaneTreeNode,
};
pub use pane_section::PaneSection;
pub use path_field::{BrowseRequested, PathField};
pub use placeholders::{LevelMeter, LevelMeterVisual, Skeleton};
pub use popover::{
    ActionMenu, AnchorOverflow, MENU_OVERLAY_Z_INDEX, MenuSurfaceGeometry, MenuSurfaceVisual,
    Popover, PopoverClosed, PopoverToggled, TOP_LAYER_Z_INDEX, resolve_anchored_surface,
    resolve_popover_origin,
};
pub use presence::{
    PresenceGroup, PresenceHiddenMode, PresenceLifecycle, PresencePhase, PresenceSettled,
};
pub use presentation::{
    HIGHLIGHT_PRESENTER, HighlightRequest, TextPresentation, TextPresenter, TextPresenterFn,
    TextSpan,
};
#[cfg(feature = "syntax-highlighting")]
pub use presentation::{HighlightPresentation, SyntectHighlighter};
pub use presentation_clock::PresentationPlayback;
pub use profiler::{FrameProfile, FrameProfiler, StageStatus, StageTiming};
#[cfg(feature = "property-grid")]
pub use property_grid::{
    PropertyGrid, PropertyLabelWidthChanged, PropertyRow, PropertySection, PropertySectionToggled,
};
pub use qr_code::{QrCode, QrCodeError, QrCodeVisual};
pub use range_span::{
    RangeSpanChanged, RangeSpanDragState, RangeSpanDragging, RangeSpanEntered, RangeSpanField,
    RangeSpanHandle, RangeSpanInput, RangeSpanOrientation, RangeSpanThumb, range_span_thumb_extent,
    range_span_track_inset, range_span_value_thumb_height, range_span_value_track_inset,
};
#[cfg(feature = "controls")]
pub use reorder_list::{
    REORDER_PREVIEW_Z_INDEX, ReorderAccepts, ReorderDragPreview, ReorderDragVisual,
    ReorderDropMark, ReorderExternalDrag, ReorderGapPlan, ReorderItem, ReorderKeyMove,
    ReorderLayout, ReorderList, ReorderListEvent, ReorderListLayout, ReorderListPointer,
    ReorderListVisual, ReorderRowPaint,
};
#[cfg(feature = "rich-text")]
pub use rich_text::{
    ALERT_BAR_WIDTH, ALERT_TITLE_HEIGHT, MarkdownAlertKind, MarkdownBlock, MarkdownBlockKind,
    MarkdownDiagram, MarkdownDiagramPlacement, MarkdownImage, MarkdownImageResource, MarkdownSpan,
    MarkdownTable, MarkdownTableAlignment, NativeMarkdown, NativeMarkdownLayout,
    NativeMarkdownVisual, RichSpan, RichTextEvent, SelectableRichText, SelectableRichTextLayout,
    SelectableRichTextVisual, TextSelectionGroup, TextSelectionGroupId, TextSelectionSnapshot,
};
pub use schedule::SystemWork;
pub use scrub_field::{ScrubChanged, ScrubDragging, ScrubField, ScrubInput};
pub use search_dropdown::{SearchDropdown, SearchDropdownEvent, SearchDropdownOption};
pub use select::{
    Select, SelectChanged, SelectGeometry, SelectOption, SelectToggled, SelectVisual,
};
pub use selection::{
    RovingEdge, RovingFocusEdge, RovingFocusGroup, RovingFocusIntent, RovingFocusPolicy,
    SegmentedControl, SegmentedOption, SegmentedOptionChosen, SegmentedSelectionRequested,
    SelectionChrome, SelectionOptionVisual, SelectionOrientation,
};
pub use settings::{
    AboutMetadata, AboutSection, AboutSectionAssembly, AppearanceSection,
    AppearanceSectionAssembly, HintPlacement, SettingsBack, SettingsCard, SettingsCollapsibleCard,
    SettingsGroup, SettingsPage, SettingsPageAssembly, SettingsRow, SettingsSidebar,
    SettingsSidebarAssembly, SettingsTabSelected, apply_appearance_event,
};
pub use shell::{AppShell, AppTitleBar, AppTitleBarControls, DesktopShell, WindowChromeAction};
pub use sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowIcon, SidebarRowState,
    SidebarRowTone, SidebarSection, SidebarSectionSlots, SidebarSectionState,
    sidebar_row_depth_inset, sidebar_row_tool_button, sidebar_section_tool_button,
    sidebar_top_bar_tool_button,
};
pub use split_pane::SplitPane;
pub use spotlight::{
    AnchoredCallout, CalloutPlacement, CalloutPlatePainter, CalloutSide, SpotlightHole,
    SpotlightHoleShape, SpotlightScrim,
};
pub use tabs::{TabOption, Tabs, TabsEvent};
#[cfg(feature = "terminal")]
pub use terminal::{
    MAX_TERMINAL_CELLS, TerminalCell, TerminalCursor, TerminalCursorShape, TerminalEvent,
    TerminalPosition, TerminalScreen, TerminalSelection, TerminalView,
};
pub use text_editing::{
    TextCaretIntent, TextLineDirection, TextSearchOptions, expanded_selection, find_matches,
    find_matches_capped, find_matches_in_range, find_next_match, find_previous_match,
    matching_bracket_pair, preserve_case_replacement, replace_all_matches,
    replace_all_matches_in_range, sort_lines,
};
pub use text_engine_shaper::NanaTextEngineShaper;
pub use text_node::{
    RetainedTextLayout, TextDirty, TextRevisions, TextWork, apply_used_typography, nana_font_family,
};
pub use thumbnail::{
    DEFAULT_ASPECT as THUMBNAIL_DEFAULT_ASPECT, Thumbnail, ThumbnailPick, ThumbnailPoint,
    ThumbnailState,
};
#[cfg(feature = "timeline")]
pub use timeline::{
    TIMELINE_ROW, TIMELINE_RULER, TimelineClip, TimelineClipEdge, TimelineClipSelectionChanged,
    TimelineClipTone, TimelineClipTrimmed, TimelineClipsDeleteRequested, TimelineClipsMoved,
    TimelineEditor, TimelineHandleMoved, TimelineHit, TimelineKey, TimelineKeyRef,
    TimelineKeysMoved, TimelineMode, TimelinePlayheadChanged, TimelineSelectionChanged,
    TimelineTick, TimelineTrack, TimelineTrackToggled, TimelineUnit, TimelineView,
    TimelineViewChanged,
};
pub use toast::{Toast, ToastAction, ToastDismissed, ToastProgress, ToastTone, ToastVisual};
pub use toast_stack::ToastStack;
pub use tree_view::{
    TreeDragReleased, TreeDropIntent, TreeDropMark, TreeDropPosition, TreeDropRequested,
    TreeRenameRequested, TreeRowData, TreeRowGeometry, TreeSelection, TreeSelectionRequested,
    TreeToggleChanged, TreeToggleColumn, TreeView, TreeViewLayout, TreeViewVisual,
};
pub use video::Video;
pub use view::VirtualSelectionRequest;
pub use view_components::{
    Activate, Button, ButtonVisual, Card, CardVisual, Checkbox, CheckboxVisual, CodeEditing,
    ComponentView, Dialog, DialogCloseRequested, DialogToggled, Divider, DragDropEvent,
    DragSourceEvent, FileDropEvent, FocusWithinChanged, HostedTextarea, IconButton,
    IconButtonTooltip, IconGlyph, IconVisual, List, ListItem, ListItemRole, ListItemSlots,
    ListItemVisual, NumberChanged, NumberInput, OverlayChanged, OverlayClosing, OverlayHost,
    PointerDragPhase, PointerDragged, PointerHoverChanged, PointerMoved, RANGE_MAXIMUM_SLOT,
    RANGE_MINIMUM_SLOT, RANGE_THUMB_TEXT_SLOT, RangeAdjustment, RangeChanged, RangeDragState,
    RangeDragging, RangeField, RangeGeometry, RangeInput, RangeMarker, RangeVisual,
    RowToolsPresence, RowToolsRest, ScrollAnchor, ScrollAxes, ScrollChanged, ScrollLaidOut,
    ScrollView, ScrollViewportChanged, ScrollbarDragState, ScrollbarVisual, SecondaryPress,
    SizeChanged, Stack, Switch, SwitchVisual, Table, TableCell, TableCellFocused, TableRow, Text,
    TextArea, TextChanged, TextClamped, TextInput, TextSubmitted, ToggleChanged, Tooltip,
    TrailingAlign, UserScroll,
};
/// Scene and painter internal contract shared with `nana-ui-scene`, `nana-ui`,
/// `nana-ui-devtools`, and host benchmarks/snapshot tools.
#[doc(hidden)]
pub mod scene_contract {
    pub use crate::components::{
        ComponentElevation, ComponentImage, ComponentTextRegion, ComponentTriggerSurface,
        ContrastRing, DropHoverOverlay, ExtractedCompositor, ExtractedLayout, ExtractedMove,
        ExtractedNode, ExtractedTextSpan, MenuItemMark, MenuSurfaceKind, NumberSteppers,
        OverlayHostState, PointerCaptureChange, RichEditorMarks, ScrollbarBar, ScrollbarBars,
        TriggeredMenuOverlay, VisualParent, decode_virtual_menu_item, decode_virtual_tree_item,
        decode_virtual_tree_toggle, menu_row_index, menu_row_slots, virtual_listbox_id,
        virtual_menu_item_id, virtual_tree_item_id, virtual_tree_toggle_id,
    };
    pub use crate::framework::{EditableHooks, TypeBehavior};
    pub use crate::layout_engine::parts::{
        CONTENT_PART_SLOTS, ContentParts, LeadingParts, NO_CONTENT_PARTS,
    };
    pub use crate::rich_text_editor::caret_box as rich_editor_caret_box;
    pub use crate::widget_visual::{
        CHART_RENDERER, CONTEXT_MENU_HINTS, CONTEXT_MENU_IMAGE_BACKDROPS, CONTEXT_MENU_IMAGES,
        CustomGpuPayload, MODAL_SCRIM_SLOT, SCROLLBAR_CHROME_SLOT_BASE, SELECT_MENU_BACKGROUNDS,
        SELECT_MENU_HINTS, SELECT_MENU_IMAGE_BACKDROPS, SELECT_MENU_IMAGES, SELECT_MENU_LABELS,
        SELECT_MENU_MARKS, SELECT_MENU_SCROLLBAR, SELECT_MENU_SURFACE_SLOT, TEXT_ATOM_ICONS_NS,
        TEXT_ATOM_LABELS_NS, TEXT_DIAGNOSTIC_LABELS_NS, TEXT_DIAGNOSTIC_MARKERS_NS,
        TEXT_LINE_LABELS_NS, TRIGGERED_OVERLAY_SURFACE_SLOT, WIDGET_VECTOR_LAYER_BASE,
        WidgetBatchColors, WidgetChildTextSuppression, WidgetFloatingOverlay,
        WidgetFloatingOverlayKind, WidgetGlyph, WidgetIcon, WidgetImage, WidgetModalFrame,
        WidgetPaintCtx, WidgetPaintOp, WidgetPaintOutput, WidgetQuad, WidgetQuadBatch,
        WidgetRetainedText, WidgetRootSurface, WidgetStroke, WidgetTaggedOp, WidgetText,
        WidgetVisual, WidgetVisualContent, WidgetVisualHandle, is_floating_overlay_slot,
        is_select_menu_slot, scrollbar_chrome_slot, widget_collection_slot,
    };
    pub use crate::world::inline_content::text_layout_origin;
}
pub(crate) use scene_contract::*;
pub use workspace::{Workspace, WorkspaceRegionSlot, WorkspaceResizeHandle};
pub use world::{
    BuildIdHasher, CommitReport, DocumentId, IdHasher, InputModality, InputRegionBackground,
    LastThemeInvalidation, LocaleGenerations, LocaleScope, MAX_SPLIT_DEPTH, MAX_TREE_DEPTH,
    NodeKind, NodeMap, NodeSet, NodeSnapshot, ReplacedMetadata, ReplacedResource, StableNodeId,
    ThemeDependencySummary, ThemeInteractionState, ThemeInvalidationTrigger, ThemeNodeInspection,
    ThemeNodeInvalidationStatus, ThemeResolvedToken, ThemeScopeChainEntry, ThemeTraceLayer,
    ThemeTransitionSource, UiWorld, UiWorldError, WindowInteractionRegionBuilder,
    WindowInteractionRegionConfig,
};
pub use xy_pad::{
    XYPad, XYPadAdjustment, XYPadAxisLock, XYPadDragState, XYPadEvent, XYPadLayout, XYPadSurface,
    XYPadValue, XYPadVisual, xy_pad_height,
};

pub use native_content::{NATIVE_CONTENT_RENDERER, NativeContent};
pub use panel::{Panel, PanelEdge, PanelInsets};

/// Reported when Enter confirms an editable [`NumberInput`], after any
/// pending draft has been committed (or reverted if unparseable) and
/// excluding IME preedit. Fires even when the committed value did not change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberSubmitted {
    pub value: f64,
}

/// Reported when Escape cancels editing on an editable [`NumberInput`],
/// after any pending draft has been reverted and excluding IME preedit.
/// Fires even when no draft differed from the committed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberCancelled;
