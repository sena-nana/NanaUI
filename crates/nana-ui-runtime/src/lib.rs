//! Backend-neutral retained runtime for NanaUI.
//!
//! Applications and compatibility adapters use stable Nana IDs. The internal
//! generational entity representation is deliberately not part of the public
//! contract, so changing ECS implementations cannot invalidate JS handles,
//! diagnostics, snapshots, or persisted data.

mod animation;
mod command_palette;
mod components;
mod dropdown;
mod feedback;
mod form_surfaces;
mod framework;
mod layout_engine;
mod menus;
mod mutation;
mod overlay_surfaces;
mod placeholders;
mod popover;
mod presentation;
mod qr_code;
mod query;
mod schedule;
mod search_dropdown;
mod select;
mod selection;
mod settings;
mod sidebar;
mod tabs;
mod toast;
mod tree_view;
mod view_components;
mod world;
mod xy_pad;

pub use animation::{AnimationFrame, AnimationId, AnimationSample, AnimationSpec, Easing};
pub use command_palette::CommandPalette;
pub use components::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityDelta, AccessibilityNode,
    AccessibilityRole, AccessibilityState, AccessibilityUpdate, ComponentElevation,
    ComponentGeometry, ComponentTextRegion, ComputedStyle, CustomRenderNode, EventRoute,
    ExtractedNode, ExtractedTextSpan, ImeComposition, InteractionState, InteractionStyle,
    LayoutBox, LayoutInput, MenuSurfaceKind, ModalLayoutInput, MountState, NodeStyle,
    OverlayHostState, PointerCaptureChange, ScrollMetrics, ScrollOffset, SelectMenuGeometry,
    SelectOptionData, SelectOptionGeometry, SemanticPaint, StandardVisual, TextContent,
    TextHorizontalAlignment, TextInputPresentation, TextInputState, TextMetrics, TextSelection,
    TextShapeConstraints, TextShaper, TextShaping, TextVerticalAlignment, TooltipVisual,
};
pub use dropdown::{Dropdown, DropdownOption};
pub use feedback::{
    EmptyState, LabeledValue, Progress, ProgressCancelled, Spinner, StatusBadge, ValidationMessage,
    ValueEmphasis,
};
pub use form_surfaces::{FormField, InteractiveCard};
pub use framework::{
    ActiveRuntimeOverlay, AppContext, Entity, ExtensionRegistrar, FrameworkError, OverlayKey,
    OverlayPointerDecision, OverlayPointerPhase, RuntimeOverlayKind, Subscription, Task,
    UiExtension, View, ViewContext, VirtualListItems, VirtualTableItems,
};
pub use layout_engine::{LayoutViewport, RuntimeLayoutEngine};
pub use menus::{
    ActionMenuItem, AnchoredActionMenu, ContextMenu, ContextMenuEvent, ContextMenuItem,
    resolve_anchored_origin,
};
pub use mutation::{MutationQueue, UiMutation};
pub use nana_ui_core::{
    ActionId, ActionPickerNavigation, AlignSpec, CommandPaletteEvent, CommandPaletteItem,
    ContextPredicate, DropdownEvent, DropdownSelection, FlexDirection, JustifySpec, KeyContext,
    LayoutStyle, LengthSpec, PopoverAlignment, PopoverPlacement, PositionSpec, SemanticColorRole,
    StatusTone, TabDragGroup, TabDragLease, TabDragRect, TabDragSurface, TabDropIndicator,
    TabStripPaint, TableCursor, TableNavigation, ThemeMode, TreeNavigation, TreeNode,
    TreeViewEvent, ValidationIntent, VirtualListLayout, VirtualListMaterialization,
    VirtualListMaterializationError, VirtualListMaterializer, VirtualListMount, VirtualListWindow,
    VirtualTableLayout, VirtualTableMaterialization, VirtualTableMaterializer, VirtualTableWindow,
    tree_navigation_event,
};
pub use overlay_surfaces::{
    ConfirmDialog, ConfirmIntent, ConfirmSlots, Drawer, ModalBehavior, ModalInitialFocus,
    ModalSlots, ModalSurface, ModalSurfaceKind,
};
pub use placeholders::{LevelMeter, Skeleton};
pub use popover::{ActionMenu, Popover, PopoverClosed, PopoverToggled, resolve_popover_origin};
pub use presentation::{
    HIGHLIGHT_PRESENTER, HighlightRequest, TextPresentation, TextPresenter, TextSpan,
};
#[cfg(feature = "syntax-highlighting")]
pub use presentation::{HighlightPresentation, SyntectHighlighter};
pub use qr_code::{QrCode, QrCodeError};
pub use schedule::SystemWork;
pub use search_dropdown::{SearchDropdown, SearchDropdownEvent, SearchDropdownOption};
pub use select::{Select, SelectChanged, SelectOption};
pub use selection::{
    Radio, RadioGroup, RovingFocusIntent, RovingFocusPolicy, SegmentedControl, SegmentedOption,
    SegmentedSelectionRequested, SelectionChrome, SelectionOrientation,
};
pub use settings::{
    AboutMetadata, AboutSection, AboutSectionAssembly, AppearanceSection,
    AppearanceSectionAssembly, SettingsCard, SettingsCollapsibleCard, SettingsRow,
    apply_appearance_event,
};
pub use sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowIcon, SidebarRowState,
    SidebarRowTone, SidebarSection, SidebarSectionSlots, SidebarSectionState,
    sidebar_row_depth_inset,
};
pub use tabs::{TabOption, Tabs, TabsEvent};
pub use toast::{Toast, ToastDismissed, ToastTone};
pub use tree_view::TreeView;
pub use view_components::{
    Activate, Button, Card, Checkbox, ComponentView, Dialog, IconButton, IconButtonTooltip, List,
    ListItem, ListItemSlots, Menu, MenuItem, OverlayChanged, OverlayHost, RangeAdjustment,
    RangeChanged, RangeDragState, RangeField, ScrollAxes, ScrollChanged, ScrollView, Slider,
    SliderChanged, SliderError, Switch, Tab, TabList, TabSelected, Table, TableCell,
    TableCellFocused, TableRow, Text, TextArea, TextChanged, TextInput, ToggleChanged, Tooltip,
};
pub use world::{
    CommitReport, DocumentId, NodeKind, NodeSnapshot, StableNodeId, UiWorld, UiWorldError,
};
pub use xy_pad::{
    XYPad, XYPadAdjustment, XYPadAxisLock, XYPadDragState, XYPadEvent, XYPadValue, xy_pad_height,
};
