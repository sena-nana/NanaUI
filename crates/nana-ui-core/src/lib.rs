//! Backend-neutral NanaUI contracts shared by Runtime and Vue adapters.
//!
//! ## Style Model (L1 / L2 / L3)
//!
//! All styling paths converge on one model — see [`style_model`]:
//! **Tokens + Semantics + Layout**. L1 CSS adapters and L2 Vue props map *into*
//! this model; L3 Rust APIs are the native entry. Drawing stays in Runtime /
//! UiScene. This crate must **not** depend on CSS parsers, JS engines, WebView,
//! or window backends.

pub mod action;
pub mod box_layout;
pub mod date;
pub mod dialog;
pub mod drag;
pub mod expansion;
pub mod file_dialog;
pub mod fonts;
pub mod geometry;
pub mod graph;
pub mod icon;
mod icon_data;
pub mod layout;
mod layout_style_api;
pub mod menu;
pub mod menu_bar;
pub mod motion;
pub mod number_field;
pub mod overlay;
pub mod persist;
pub mod scrollbar;
pub mod selection;
pub mod semantics;
pub mod settings;
pub mod split_pane;
pub mod style_model;
pub mod tab_drag;
pub mod theme;
pub mod tree;
pub mod typography;
pub mod url_jail;
pub mod virtual_list;
pub mod virtual_table;
pub mod virtual_tree;
pub mod work;
pub mod workspace_model;
pub mod writing;

pub use action::{
    ActionId, ActionPickerNavigation, CommandPaletteEvent, CommandPaletteItem, ContextPredicate,
    KeyContext,
};
pub use box_layout::{
    AlignSpec, BackdropFilter, BackgroundImage, BackgroundImageFit, BackgroundPosition,
    BackgroundRepeat, BorderImageSlice, BorderImageSpec, BorderImageTile, BorderStyle,
    BoxShadowSpec, BoxSizing, CalcBinOp, CalcExpr, CalcExprRef, ClearSpec, ClipCircle, ClipEllipse,
    ClipInset, ClipPath, ClipPoint, ClipShapeRadius, ColorFilter, CssGradient, CursorSpec, DirSpec,
    DisplaySpec, FilterDropShadow, FlexDirection, FlexWrap, FloatSpec, FontFeatureSetting,
    FontSizeContext, GradientStop, GridAutoFlow, GridLine, GridPlacement, GridRepeatAuto,
    GridTemplateAreas, GridTrack, GridTrackListUnsupported, JustifySpec, LayoutStyle, LengthAtom,
    LengthSpec, LineHeightSpec, LinearGradient, LogicalInlineEdges, LogicalInsets,
    MAX_BACKGROUND_LAYERS, MAX_BOX_SHADOWS, MaskImage, MixBlendMode, OutlineSpec, OutlineStyle,
    OverflowSpec, OverflowWrapSpec, PaddingSpec, PaintMat4, PaintStyle, PaintTransform, ParentBox,
    PointerEventsSpec, PositionSpec, RadialGradient, TEXT_APPROX_ASCENT_EM, TextAlignSpec,
    TextDecorationLine, TextShadowSpec, TextWrapBreak, TransformBox, TransformOrigin,
    UserSelectSpec, ViewportAxis, VisibilitySpec, WhiteSpaceSpec, WordBreakSpec, WritingModeSpec,
    glyph_box_center_from_line_top, icon_y_on_text_glyph_center, resolve_grid_column_widths,
    resolve_grid_track_sizes, text_line_box_height_px,
};
pub use date::{CivilDate, DayCell, MonthGrid, WeekStart, Weekday, days_in_month, is_leap_year};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use drag::{DropAccepts, DropEffect, DropKind, FileDragKind};
pub use expansion::ExpansionState;
pub use file_dialog::{
    FileDialogError, FileDialogKind, FileDialogRequest, FileDialogResult, FileFilter,
};
pub use geometry::{
    ContentFit, LogicalPoint, LogicalRect, PhysicalRect, RESIZE_HANDLE_SIZE, RegionRect,
    TITLE_BAR_HEIGHT, WINDOW_CONTROL_GAP, WINDOW_CONTROL_PADDING, WINDOW_CONTROL_WIDTH,
    WorkspaceGeometry, custom_window_controls_width,
};
pub use graph::{
    GRAPH_EDGE_HIT_TOLERANCE, GRAPH_MAX_ZOOM, GRAPH_MIN_ZOOM, GRAPH_NODE_TITLE_HEIGHT,
    GRAPH_PORT_HIT_RADIUS, GRAPH_PORT_INSET, GRAPH_PORT_PITCH, GraphCanvasId, GraphEdge,
    GraphEdgeId, GraphEndpoint, GraphModel, GraphModelError, GraphNode, GraphNodeId, GraphPoint,
    GraphPort, GraphPortId, GraphPortKind, GraphPortSide, GraphRect, GraphSelection, GraphSize,
    GraphTarget, GraphTargetDescriptor, GraphTargetId, GraphTargetKind, GraphViewport, cubic_point,
    graph_node_fitted_height, port_tangent,
};
pub use icon::{Icon, IconData, IconGeometry, IconPathCommand, IconShape};
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use menu::{MenuConfirmation, MenuSelection};
pub use menu_bar::{Menu, MenuBar, MenuEntry, MenuShortcut};
pub use motion::{
    AnimatableProperty, AnimationClass, AnimationDirection, AnimationFillMode, AnimationIteration,
    AnimationPlayState, AnimationPlayback, CompiledMotion, DecayParams, Easing, FlipRect, Keyframe,
    MOTION_DESCRIPTOR_VERSION, MOTION_GPU_DESCRIPTOR_SIZE, MOTION_GPU_KEYFRAME_SIZE,
    MOTION_GPU_TIME_SIZE, MotionCodecError, MotionCodecId, MotionCodecInfo, MotionCodecRegistry,
    MotionCurve, MotionDescriptor, MotionDescriptorError, MotionDescriptorStore,
    MotionEvaluatorBackend, MotionGpuDescriptor, MotionGpuKeyframe, MotionGpuTime, MotionGpuValue,
    MotionGraph, MotionHandle, MotionInspectorEntry, MotionInterrupt, MotionSample, MotionTargetId,
    MotionTiming, MotionTo, MotionTrack, MotionTrackId, MotionValue, MotionValueKind,
    MotionWorkCounters, PresentationOverlay, PresentationPair, PresentationSlot, PresentationStore,
    ProgressSample, Spring, SpringParams, StepJump, TimedProgress, Timeline,
    classify_animatable_property, compile_motion_descriptor, cpu_fallback_reason,
    decode_motion_track, evaluate_descriptor, evaluate_progress, evaluate_track, evaluate_track_at,
    invert_flip_translate, retarget_track, track_completion_deadline,
};
pub use number_field::NumberFieldSpec;
pub use overlay::ExclusiveOverlay;
pub use persist::{
    KEY_APPEARANCE_PREFIX, KEY_DOCK_PREFIX, KEY_WINDOW_PREFIX, MemoryStore, PersistentStore,
    SharedStore, StoreError, appearance_storage_key, dock_storage_key, is_framework_storage_key,
    memory_store, shared_store, window_storage_key,
};
pub use scrollbar::{
    SCROLLBAR_METRICS, ScrollbarAxis, ScrollbarMetrics, ScrollbarSkin, ScrollbarTrack,
    ScrollbarVisibility, scrollbar_track,
};
pub use selection::{SelectionMove, SingleSelection};
pub use semantics::{
    AnchoredMenuPlacement, ButtonKind, CardKind, ControlSize, DrawerSide, DropdownEvent,
    DropdownSelection, PopoverAlignment, PopoverPlacement, RADIO_ROW_INSET, StatusTone,
    SwitchControlPosition, ToastTone, TooltipConfig, TooltipPlacement, ValidationIntent,
    WindowChrome, WindowChromeAction, WindowControlMode, XYPadEvent, XYPadValue,
};
pub use settings::{
    AppearanceEvent, AppearanceSettings, BackdropTarget, SettingsError, SettingsModel,
    SettingsState, SettingsTab, SettingsTabId, WindowMaterialMode,
};
pub use split_pane::{SplitAxis, SplitPaneModel, SplitPaneMutation};
pub use style_model::{
    ControlSemantics, SemanticColor, SemanticColorMix, SemanticColorRole, SemanticPalette,
    StyleModelRef,
};
pub use tab_drag::{
    TabDragGroup, TabDragLease, TabDragRect, TabDragSurface, TabDropIndicator, TabStripPaint,
    drop_before_index, reorder_changes_position, tab_at,
};
pub use theme::{
    AccentRamp, BorderTokens, BorderWidth, ButtonRecipe, ButtonRecipeDraft, ButtonVariantDraft,
    ButtonVariantRecipe, ChromeRadii, CompiledRecipes, CompiledTheme, ComponentRecipe,
    ComponentRecipeDraft, ComponentRecipeId, ComponentThemeRegistry, ControlHeight, ControlPadding,
    DesignTokens, EasingRole, EffectTokens, ElevationRole, FoundationTokens, HAIRLINE, LineRole,
    MotionRole, MotionTokens, OpacityTokens, RadiusTier, SWITCH_METRICS, ShadowToken, SpacingStep,
    SpacingTokens, SquareSize, StateLayer, StatusRecipe, SurfaceMaterial, SurfacePadding,
    SurfaceRole, SurfaceSpec, SurfaceTokens, SwitchMetrics, TextWeight, ThemeCompileError,
    ThemeDefinition, ThemeGeneration, ThemeId, ThemeIdentity, ThemeMetrics, ThemeMode,
    ThemeSchemaVersion, TypeRole, TypographyTokens, UI_BASE_TEXT_SIZE, UI_METRICS, builtin_theme,
    builtin_theme_arc, space, type_scale,
};
pub use tree::{TreeNavigation, TreeNode, TreeViewEvent, tree_navigation_event};
pub use typography::{FontKerningSpec, FontVariationSetting, LineBreakSpec};
pub use url_jail::{
    MAX_LOCAL_URL_BYTES, canonicalize_within_jail, file_url_to_path,
    href_is_protocol_relative_or_unc, is_remote_or_data_href, path_looks_network, path_to_file_url,
    percent_decode_bytes, read_bytes_within_jail, read_file_within_jail, resolve_filesystem_href,
    stylesheet_base_from_href,
};
pub use virtual_list::{
    VirtualAlignment, VirtualFrozenWindow, VirtualListLayout, VirtualListMaterialization,
    VirtualListMaterializationError, VirtualListMaterializer, VirtualListMount, VirtualListWindow,
    VirtualScrollAnchor, VirtualViewport,
};
pub use virtual_table::{
    SortDirection, TableColumn, TableCursor, TableNavigation, TableSort, VirtualTableFrozenWindow,
    VirtualTableLayout, VirtualTableMaterialization, VirtualTableMaterializer, VirtualTableWindow,
};
pub use virtual_tree::{VirtualTreeLayout, VirtualTreeRow, VirtualTreeWindow};
pub use work::{FrameStage, GpuWorkObservation, ThemeWorkCounters, WorkCounters};
pub use workspace_model::{
    WORKSPACE_REGION_TRANSITION_DURATION, WorkspaceModel, WorkspaceMutation,
};
pub use writing::{PhysicalEdge, WritingContext};
