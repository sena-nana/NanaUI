//! Backend-neutral retained runtime for NanaUI.
//!
//! Applications and compatibility adapters use stable Nana IDs. The internal
//! generational entity representation is deliberately not part of the public
//! contract, so changing ECS implementations cannot invalidate JS handles,
//! diagnostics, snapshots, or persisted data.

mod animation;
mod components;
mod framework;
mod layout_engine;
mod mutation;
mod schedule;
mod view_components;
mod world;

pub use animation::{AnimationFrame, AnimationId, AnimationSample, AnimationSpec, Easing};
pub use components::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityDelta, AccessibilityNode,
    AccessibilityRole, AccessibilityState, AccessibilityUpdate, ComputedStyle, CustomRenderNode,
    EventRoute, ExtractedNode, ImeComposition, InteractionState, InteractionStyle, LayoutBox,
    LayoutInput, NodeStyle, OverlayHostState, PointerCaptureChange, ScrollMetrics, ScrollOffset,
    SemanticPaint, StandardVisual, TextContent, TextHorizontalAlignment, TextInputState,
    TextMetrics, TextSelection, TextShapeConstraints, TextShaper, TextVerticalAlignment,
};
pub use framework::{
    AppContext, Entity, ExtensionRegistrar, FrameworkError, Subscription, Task, UiExtension, View,
    ViewContext, VirtualListItems, VirtualTableItems,
};
pub use layout_engine::{LayoutViewport, RuntimeLayoutEngine};
pub use mutation::{MutationQueue, UiMutation};
pub use nana_ui_core::{
    ActionId, ContextPredicate, KeyContext, TableCursor, TableNavigation, ThemeMode,
    VirtualListLayout, VirtualListMaterialization, VirtualListMaterializationError,
    VirtualListMaterializer, VirtualListMount, VirtualListWindow, VirtualTableLayout,
    VirtualTableMaterialization, VirtualTableMaterializer, VirtualTableWindow,
};
pub use schedule::SystemWork;
pub use view_components::{
    Activate, Button, Card, Checkbox, ComponentView, Dialog, IconButton, List, ListItem, Menu,
    MenuItem, OverlayChanged, OverlayHost, ScrollAxes, ScrollChanged, ScrollView, Slider,
    SliderChanged, SliderError, Switch, Tab, TabList, TabSelected, Table, TableCell,
    TableCellFocused, TableRow, Text, TextArea, TextChanged, TextInput, ToggleChanged, Tooltip,
};
pub use world::{
    CommitReport, DocumentId, NodeKind, NodeSnapshot, StableNodeId, UiWorld, UiWorldError,
};
