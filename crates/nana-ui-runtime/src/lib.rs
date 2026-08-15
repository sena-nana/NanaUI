//! Backend-neutral retained runtime for NanaUI.
//!
//! Applications and compatibility adapters use stable Nana IDs. The internal
//! generational entity representation is deliberately not part of the public
//! contract, so changing ECS implementations cannot invalidate JS handles,
//! diagnostics, snapshots, or persisted data.

mod animation;
mod components;
mod feedback;
mod framework;
mod layout_engine;
mod mutation;
mod overlay_surfaces;
mod schedule;
mod selection;
mod view_components;
mod world;

pub use animation::{AnimationFrame, AnimationId, AnimationSample, AnimationSpec, Easing};
pub use components::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityDelta, AccessibilityNode,
    AccessibilityRole, AccessibilityState, AccessibilityUpdate, ComponentElevation,
    ComponentGeometry, ComponentTextRegion, ComputedStyle, CustomRenderNode, EventRoute,
    ExtractedNode, ImeComposition, InteractionState, InteractionStyle, LayoutBox, LayoutInput,
    ModalLayoutInput, MountState, NodeStyle, OverlayHostState, PointerCaptureChange, ScrollMetrics,
    ScrollOffset, SemanticPaint, StandardVisual, TextContent, TextHorizontalAlignment,
    TextInputPresentation, TextInputState, TextMetrics, TextSelection, TextShapeConstraints,
    TextShaper, TextShaping, TextVerticalAlignment, TooltipVisual,
};
pub use feedback::{
    EmptyState, LabeledValue, Progress, Spinner, StatusBadge, ValidationMessage, ValueEmphasis,
};
pub use framework::{
    ActiveRuntimeOverlay, AppContext, Entity, ExtensionRegistrar, FrameworkError, OverlayKey,
    OverlayPointerDecision, OverlayPointerPhase, RuntimeOverlayKind, Subscription, Task,
    UiExtension, View, ViewContext, VirtualListItems, VirtualTableItems,
};
pub use layout_engine::{LayoutViewport, RuntimeLayoutEngine};
pub use mutation::{MutationQueue, UiMutation};
pub use nana_ui_core::{
    ActionId, AlignSpec, ContextPredicate, FlexDirection, JustifySpec, KeyContext, LayoutStyle,
    LengthSpec, PositionSpec, SemanticColorRole, StatusTone, TableCursor, TableNavigation,
    ThemeMode, ValidationIntent, VirtualListLayout, VirtualListMaterialization,
    VirtualListMaterializationError, VirtualListMaterializer, VirtualListMount, VirtualListWindow,
    VirtualTableLayout, VirtualTableMaterialization, VirtualTableMaterializer, VirtualTableWindow,
};
pub use overlay_surfaces::{
    ConfirmDialog, ConfirmIntent, ConfirmSlots, Drawer, ModalBehavior, ModalInitialFocus,
    ModalSlots, ModalSurface, ModalSurfaceKind,
};
pub use schedule::SystemWork;
pub use selection::{
    Radio, RadioGroup, RovingFocusIntent, RovingFocusPolicy, SegmentedControl, SegmentedOption,
    SegmentedSelectionRequested, SelectionOrientation,
};
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
