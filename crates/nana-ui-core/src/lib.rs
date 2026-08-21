//! Backend-neutral NanaUI contracts shared by Iced and Vue bridge adapters.
//!
//! ## Style Model (L1 / L2 / L3)
//!
//! All styling paths converge on one model — see [`style_model`]:
//! **Tokens + Semantics + Layout**. L1 CSS adapters and L2 Vue props map *into*
//! this model; L3 Rust APIs are the native entry. Drawing stays in `nana-ui`
//! widgets. This crate must **not** depend on Iced, Blitz, CSS parsers,
//! QuickJS, V8, WebView, or window backends.

pub mod action;
pub mod box_layout;
pub mod dialog;
pub mod expansion;
pub mod geometry;
pub mod graph;
pub mod icon;
pub mod layout;
pub mod menu;
pub mod overlay;
pub mod selection;
pub mod semantics;
pub mod settings;
pub mod split_pane;
pub mod style_model;
pub mod tab_drag;
pub mod theme;
pub mod tree;
pub mod virtual_list;
pub mod virtual_table;
pub mod virtual_tree;
pub mod work;
pub mod workspace_model;

pub use action::{
    ActionId, ActionPickerNavigation, CommandPaletteEvent, CommandPaletteItem, ContextPredicate,
    KeyContext,
};
pub use box_layout::{
    AlignSpec, BoxSizing, DisplaySpec, FlexDirection, FlexWrap, FontSizeContext, GridAutoFlow,
    GridTrack, GridTrackListUnsupported, JustifySpec, LayoutStyle, LengthAtom, LengthSpec,
    LineHeightSpec, OverflowSpec, PaddingSpec, PaintTransform, ParentBox, PositionSpec,
    ViewportAxis, resolve_grid_column_widths, resolve_grid_track_sizes,
};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use expansion::ExpansionState;
pub use geometry::{
    ContentFit, LogicalPoint, LogicalRect, PhysicalRect, RESIZE_HANDLE_SIZE, RegionRect,
    TITLE_BAR_HEIGHT, WorkspaceGeometry,
};
pub use graph::{
    GRAPH_EDGE_HIT_TOLERANCE, GRAPH_MAX_ZOOM, GRAPH_MIN_ZOOM, GRAPH_PORT_HIT_RADIUS, GraphCanvasId,
    GraphEdge, GraphEdgeId, GraphEndpoint, GraphModel, GraphModelError, GraphNode, GraphNodeId,
    GraphPoint, GraphPort, GraphPortId, GraphPortKind, GraphPortSide, GraphRect, GraphSelection,
    GraphSize, GraphTarget, GraphTargetDescriptor, GraphTargetId, GraphTargetKind, GraphViewport,
    cubic_point, port_tangent,
};
pub use icon::Icon;
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use menu::{MenuConfirmation, MenuSelection};
pub use overlay::ExclusiveOverlay;
pub use selection::{SelectionMove, SingleSelection};
pub use semantics::{
    AnchoredMenuPlacement, ButtonKind, CardKind, ControlSize, DrawerSide, DropdownEvent,
    DropdownSelection, PopoverAlignment, PopoverPlacement, StatusTone, SwitchControlPosition,
    ToastTone, TooltipConfig, TooltipPlacement, ValidationIntent, WindowChrome, WindowChromeAction,
    WindowControlMode, XYPadEvent, XYPadValue,
};
pub use settings::{
    AppearanceEvent, AppearanceSettings, BackdropTarget, SettingsError, SettingsModel,
    SettingsState, SettingsTab, SettingsTabId, WindowMaterialMode,
};
pub use split_pane::{SplitAxis, SplitPaneModel, SplitPaneMutation};
pub use style_model::{
    ControlSemantics, SemanticColor, SemanticColorRole, SemanticPalette, StyleModelRef,
};
pub use tab_drag::{
    TabDragGroup, TabDragLease, TabDragRect, TabDragSurface, TabDropIndicator, TabStripPaint,
    drop_before_index, reorder_changes_position, tab_at,
};
pub use theme::{ThemeMetrics, ThemeMode, UI_BASE_TEXT_SIZE, UI_METRICS};
pub use tree::{TreeNavigation, TreeNode, TreeViewEvent, tree_navigation_event};
pub use virtual_list::{
    VirtualListLayout, VirtualListMaterialization, VirtualListMaterializationError,
    VirtualListMaterializer, VirtualListMount, VirtualListWindow,
};
pub use virtual_table::{
    TableColumn, TableCursor, TableNavigation, VirtualTableLayout, VirtualTableMaterialization,
    VirtualTableMaterializer, VirtualTableWindow,
};
pub use virtual_tree::{VirtualTreeLayout, VirtualTreeRow, VirtualTreeWindow};
pub use work::{FrameStage, GpuWorkObservation, WorkCounters};
pub use workspace_model::{
    WORKSPACE_REGION_TRANSITION_DURATION, WorkspaceModel, WorkspaceMutation,
};
