use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::hash::Hash;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_core::Stream;
use nana_ui_core::{
    ActionId, ActionPickerNavigation, CommandPaletteEvent, ContextPredicate, KeyContext,
    LengthSpec, ThemeMode, TooltipConfig, TooltipPlacement, VirtualListLayout,
    VirtualListMaterializationError, VirtualListMaterializer, VirtualListWindow,
    VirtualTableLayout, VirtualTableMaterializer, VirtualTableWindow, VirtualTreeLayout,
};

#[cfg(test)]
use crate::Dialog;
use crate::component_registry::{
    ComponentBindKind, ComponentBindRequest, ComponentRegistry, ComponentTypeId,
    RegisterableComponent, SemanticSpec, alias_entry, registerable_entry, tag_entry,
};
use crate::{
    AccessibilityAction, AccessibilityActionRequest, ActionMenu, ActionMenuItem, Activate,
    AnimationFrame, Button, Checkbox, CodeEditing, CommandPalette, ComponentView, ContextMenu,
    ContextMenuEvent, DocumentId, Dropdown, EmptyState, FormField, FrameProfile, FrameProfiler,
    FrameStage, IconButton, LabeledValue, List, ListItem, ListItemSlots, ModalSlots, ModalSurface,
    MountState, MutationQueue, NodeKind, NumberChanged, NumberInput, OverlayChanged, OverlayHost,
    Popover, PopoverClosed, PopoverToggled, Progress, ProgressCancelled, RangeAdjustment,
    RangeChanged, RangeField, RovingFocusIntent, ScrollAxes, ScrollChanged, ScrollMetrics,
    ScrollOffset, ScrollView, SearchDropdown, SearchDropdownEvent, SecondaryPress,
    SegmentedControl, SegmentedOption, SegmentedSelectionRequested, Select,
    SettingsCollapsibleCard, SidebarFooterButton, SidebarRow, SidebarSection, StableNodeId,
    StandardVisual, Switch, Table, TableCell, TableRow, Tabs, TextArea, TextChanged, TextInput,
    TextInputState, TextPresenter, TextSelection, ToggleChanged, Tooltip, TreeView, UiWorld,
    UiWorldError, XYPad, XYPadDragState, XYPadEvent,
};

mod assemble;
mod build;
mod overlay;
mod text_edit;
pub use assemble::AssemblyScope;
pub use build::UiBuilder;
pub(crate) use overlay::overlay_kind_for_role;
pub use overlay::{
    ActiveRuntimeOverlay, OverlayKey, OverlayPointerDecision, OverlayPointerPhase,
    RuntimeOverlayKind,
};
pub use text_edit::TextDeleteKind;

const MAX_EVENTS_PER_UPDATE: usize = 16_384;
const COMPONENT_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const LOADING_CYCLE: Duration = Duration::from_millis(800);

pub trait View: Send + 'static {}

impl<T: Send + 'static> View for T {}

trait EditableText: ComponentView {
    type Change: Send + 'static;
    fn accepts_input(&self) -> bool;
    fn replace_selection(&mut self, text: &str) -> bool;
    fn delete_surrounding(&mut self, before_bytes: usize, after_bytes: usize) -> bool {
        self.state_mut()
            .delete_surrounding(before_bytes, after_bytes)
    }
    fn state(&self) -> &TextInputState;
    fn state_mut(&mut self) -> &mut TextInputState;
    fn change(&self) -> Self::Change;
    /// Whether the value lays out across multiple lines.
    fn is_multiline(&self) -> bool {
        false
    }
    /// Code-editor behaviors when the component opted in.
    fn code_editing(&self) -> Option<&CodeEditing> {
        None
    }
    fn set_value(&mut self, value: String) -> bool {
        if self.state().value == value {
            return false;
        }
        self.state_mut().replace_value(value);
        true
    }
}

fn text_changed(state: &TextInputState) -> TextChanged {
    TextChanged {
        value: state.value.clone(),
        selection: state.selection,
    }
}

/// Build a scroll offset that moves one axis and holds the other.
fn scroll_offset_on(axis: nana_ui_core::ScrollbarAxis, offset: f32, hold: f32) -> ScrollOffset {
    match axis {
        nana_ui_core::ScrollbarAxis::Horizontal => ScrollOffset { x: offset, y: hold },
        nana_ui_core::ScrollbarAxis::Vertical => ScrollOffset { x: hold, y: offset },
    }
}

impl EditableText for TextInput {
    type Change = TextChanged;

    fn accepts_input(&self) -> bool {
        !self.disabled && !self.loading && !self.read_only
    }

    fn replace_selection(&mut self, text: &str) -> bool {
        self.replace_selection(text)
    }

    fn state(&self) -> &TextInputState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut TextInputState {
        &mut self.state
    }

    fn change(&self) -> TextChanged {
        text_changed(&self.state)
    }
}

impl EditableText for NumberInput {
    type Change = TextChanged;

    fn accepts_input(&self) -> bool {
        self.accepts_input()
    }

    fn replace_selection(&mut self, text: &str) -> bool {
        self.state.replace_selection(text)
    }

    fn state(&self) -> &TextInputState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut TextInputState {
        &mut self.state
    }

    fn change(&self) -> TextChanged {
        text_changed(&self.state)
    }
}

impl EditableText for TextArea {
    type Change = TextChanged;

    fn accepts_input(&self) -> bool {
        !self.disabled
    }

    fn is_multiline(&self) -> bool {
        true
    }

    fn code_editing(&self) -> Option<&CodeEditing> {
        self.code_editing.as_ref()
    }

    fn replace_selection(&mut self, text: &str) -> bool {
        self.replace_selection(text)
    }

    fn state(&self) -> &TextInputState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut TextInputState {
        &mut self.state
    }

    fn change(&self) -> TextChanged {
        text_changed(&self.state)
    }
}

impl EditableText for SearchDropdown {
    type Change = SearchDropdownEvent;

    fn accepts_input(&self) -> bool {
        self.opened && !self.inactive()
    }

    fn replace_selection(&mut self, text: &str) -> bool {
        self.replace_selection(text)
    }

    fn delete_surrounding(&mut self, before_bytes: usize, after_bytes: usize) -> bool {
        self.delete_surrounding(before_bytes, after_bytes)
    }

    fn state(&self) -> &TextInputState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut TextInputState {
        &mut self.state
    }

    fn change(&self) -> SearchDropdownEvent {
        SearchDropdownEvent::Search(self.query.clone())
    }

    fn set_value(&mut self, value: String) -> bool {
        if self.query == value {
            return false;
        }
        let _ = self.set_query(value);
        true
    }
}

impl EditableText for ContextMenu {
    type Change = ContextMenuEvent;

    fn accepts_input(&self) -> bool {
        self.open && self.searchable
    }

    fn replace_selection(&mut self, text: &str) -> bool {
        if !self.state.replace_selection(text) {
            return false;
        }
        self.sync_query_from_state();
        true
    }

    fn delete_surrounding(&mut self, before_bytes: usize, after_bytes: usize) -> bool {
        if !self.state.delete_surrounding(before_bytes, after_bytes) {
            return false;
        }
        self.sync_query_from_state();
        true
    }

    fn state(&self) -> &TextInputState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut TextInputState {
        &mut self.state
    }

    fn change(&self) -> ContextMenuEvent {
        ContextMenuEvent::Search(Arc::clone(&self.query))
    }

    fn set_value(&mut self, value: String) -> bool {
        if self.query.as_ref() == value {
            return false;
        }
        self.set_query(value);
        true
    }
}

impl EditableText for CommandPalette {
    type Change = CommandPaletteEvent;

    fn accepts_input(&self) -> bool {
        true
    }

    fn replace_selection(&mut self, text: &str) -> bool {
        self.replace_selection(text)
    }

    fn delete_surrounding(&mut self, before_bytes: usize, after_bytes: usize) -> bool {
        self.delete_surrounding(before_bytes, after_bytes)
    }

    fn state(&self) -> &TextInputState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut TextInputState {
        &mut self.state
    }

    fn change(&self) -> CommandPaletteEvent {
        CommandPaletteEvent::Search(self.query.clone())
    }

    fn set_value(&mut self, value: String) -> bool {
        if self.query == value {
            return false;
        }
        let _ = self.set_query(value);
        true
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Entity<V: View> {
    id: StableNodeId,
    marker: PhantomData<fn() -> V>,
}

impl<V: View> Copy for Entity<V> {}

impl<V: View> Clone for Entity<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V: View> Entity<V> {
    pub const fn from_stable_id(id: StableNodeId) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    pub const fn stable_id(self) -> StableNodeId {
        self.id
    }
}

type BoxedEvent = (StableNodeId, TypeId, Box<dyn Any + Send>);
type ProgramMessage = Box<dyn Any + Send>;
type ErasedEventHandler = Box<
    dyn FnMut(
            &mut dyn Any,
            &dyn Any,
            &mut MutationQueue,
            &mut VecDeque<BoxedEvent>,
            &mut Vec<ProgramMessage>,
        ) + Send,
>;
struct EventHandler {
    observer: StableNodeId,
    callback: ErasedEventHandler,
}
type ActionHandler = Box<dyn FnMut(&mut AppContext) -> Result<(), FrameworkError> + Send>;

struct RegisteredAction {
    when: ContextPredicate,
    handler: ActionHandler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadingComponent {
    Button,
    Switch,
    Card,
}

#[derive(Debug, Clone, Copy)]
struct TooltipLifecycle {
    overlay: StableNodeId,
    show_at: Option<Duration>,
    open: bool,
}

#[derive(Default)]
struct ComponentLifecycle {
    now: Duration,
    viewports: HashMap<DocumentId, crate::LayoutViewport>,
    pointer_positions: HashMap<(DocumentId, u64), (f32, f32)>,
    tooltips: HashMap<StableNodeId, TooltipLifecycle>,
    loading: HashMap<StableNodeId, LoadingComponent>,
    next_loading_frame: Option<Duration>,
    overlay_pointer_sequences: HashSet<(DocumentId, u64)>,
    overlay_outside_presses: HashMap<(DocumentId, u64), (StableNodeId, u64)>,
    overlay_activation_tokens: HashMap<StableNodeId, u64>,
    next_overlay_activation_token: u64,
}

type ActivationFn =
    Arc<dyn Fn(&mut AppContext, StableNodeId) -> Result<bool, FrameworkError> + Send + Sync>;

/// Emit [`SecondaryPress`] on a node whose concrete component type the caller
/// no longer knows. Registered per type when a component is created.
type SecondaryPressFn = Arc<
    dyn Fn(&mut AppContext, StableNodeId, SecondaryPress) -> Result<(), FrameworkError>
        + Send
        + Sync,
>;

#[derive(Default)]
pub struct ExtensionRegistrar {
    actions: HashMap<ActionId, RegisteredAction>,
    presenters: Vec<Box<dyn TextPresenter>>,
    components: ComponentRegistry,
    activations: HashMap<TypeId, ActivationFn>,
}

impl ExtensionRegistrar {
    pub fn register_action(
        &mut self,
        id: impl Into<ActionId>,
        when: ContextPredicate,
        handler: impl FnMut(&mut AppContext) -> Result<(), FrameworkError> + Send + 'static,
    ) -> Result<(), FrameworkError> {
        insert_action(&mut self.actions, id, when, handler)
    }

    pub fn register_presenter(
        &mut self,
        presenter: Box<dyn TextPresenter>,
    ) -> Result<(), FrameworkError> {
        let name = presenter.name().trim();
        if name.is_empty() {
            return Err(FrameworkError::InvalidPresenter);
        }
        if self
            .presenters
            .iter()
            .any(|existing| existing.name() == name)
        {
            return Err(FrameworkError::DuplicatePresenter(name.to_owned()));
        }
        self.presenters.push(presenter);
        Ok(())
    }

    pub fn register_component<C: RegisterableComponent>(&mut self) -> Result<(), FrameworkError> {
        let (entry, tags) = registerable_entry::<C>()?;
        self.components.insert_with_tags(entry, tags)
    }

    /// Register pointer/keyboard activation for `C`.
    ///
    /// Plugins call this from [`UiExtension::install`] instead of adding a
    /// type branch to [`AppContext::activate_node`].
    pub fn register_activation<C: View>(
        &mut self,
        handler: fn(&mut AppContext, Entity<C>) -> Result<bool, FrameworkError>,
    ) -> Result<(), FrameworkError> {
        let type_id = TypeId::of::<C>();
        if self.activations.contains_key(&type_id) {
            return Err(FrameworkError::DuplicateActivation);
        }
        self.activations.insert(
            type_id,
            Arc::new(move |context, id| handler(context, Entity::from_stable_id(id))),
        );
        Ok(())
    }

    pub fn register_tags(
        &mut self,
        type_id: &'static str,
        tags: &'static [&'static str],
    ) -> Result<(), FrameworkError> {
        let (entry, tags) = tag_entry(type_id, tags)?;
        self.components.insert_with_tags(entry, tags)
    }

    /// Register extra type ids / tags that bind through `C::from_semantic`.
    pub fn register_component_alias<C: RegisterableComponent>(
        &mut self,
        type_id: &'static str,
        tags: &'static [&'static str],
    ) -> Result<(), FrameworkError> {
        let (entry, tags) = alias_entry::<C>(type_id, tags)?;
        self.components.insert_with_tags(entry, tags)
    }
}

pub struct ViewContext<'a, V: View> {
    entity: Entity<V>,
    mutations: &'a mut MutationQueue,
    events: &'a mut VecDeque<BoxedEvent>,
    program_messages: &'a mut Vec<ProgramMessage>,
}

impl<V: View> ViewContext<'_, V> {
    pub const fn entity(&self) -> Entity<V> {
        self.entity
    }

    pub fn mutations(&mut self) -> &mut MutationQueue {
        self.mutations
    }

    pub fn emit<E: Send + 'static>(&mut self, event: E) {
        self.events
            .push_back((self.entity.id, TypeId::of::<E>(), Box::new(event)));
    }

    /// Queue a `RuntimeProgram::Message` for the Scene host.
    ///
    /// The host delivers the latest message of each type on the next frame
    /// (`RuntimeProgram::update`), not during the current input handler.
    /// Repeated dispatches of the same type keep only the last value.
    pub fn dispatch_program<M: Send + 'static>(&mut self, message: M) {
        let type_id = TypeId::of::<M>();
        self.program_messages
            .retain(|queued| queued.as_ref().type_id() != type_id);
        self.program_messages.push(Box::new(message));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameworkError {
    World(UiWorldError),
    MissingView(StableNodeId),
    ViewType(StableNodeId),
    DuplicateAction(ActionId),
    InvalidAction,
    MissingAction(ActionId),
    ActionUnavailable(ActionId),
    DuplicateExtension(String),
    DuplicatePresenter(String),
    DuplicateComponentType(String),
    DuplicateComponentTag(String),
    MissingComponentType(String),
    InvalidComponentType,
    InvalidExtension,
    InvalidPresenter,
    InvalidInput,
    InvalidVirtualization,
    EventOverflow(StableNodeId),
    InvalidComponentValue(StableNodeId),
    InvalidComponentHierarchy {
        parent: StableNodeId,
        child: StableNodeId,
    },
    InvalidListItemSlots {
        item: StableNodeId,
        slot: Option<StableNodeId>,
    },
    InvalidFeedbackSlots {
        parent: StableNodeId,
        slot: Option<StableNodeId>,
    },
    InvalidModalSlots {
        parent: StableNodeId,
        slot: Option<StableNodeId>,
    },
    OverlayActivationTokenExhausted(StableNodeId),
    FrameDidNotSettle,
    DuplicateAssemblyKey {
        parent: StableNodeId,
        key: String,
    },
    DuplicateActivation,
}

impl fmt::Display for FrameworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(error) => error.fmt(formatter),
            Self::MissingView(id) => write!(formatter, "view {} does not exist", id.get()),
            Self::ViewType(id) => write!(formatter, "view {} has a different type", id.get()),
            Self::DuplicateAction(id) => write!(formatter, "action `{id}` is already registered"),
            Self::InvalidAction => formatter.write_str("action id must not be empty"),
            Self::MissingAction(id) => write!(formatter, "action `{id}` is not registered"),
            Self::ActionUnavailable(id) => {
                write!(formatter, "action `{id}` is unavailable in this context")
            }
            Self::DuplicateExtension(name) => {
                write!(formatter, "extension `{name}` is already installed")
            }
            Self::DuplicatePresenter(name) => {
                write!(formatter, "presenter `{name}` is already registered")
            }
            Self::DuplicateComponentType(name) => {
                write!(formatter, "component type `{name}` is already registered")
            }
            Self::DuplicateComponentTag(tag) => {
                write!(formatter, "component tag `{tag}` is already registered")
            }
            Self::MissingComponentType(name) => {
                write!(formatter, "component type `{name}` is not registered")
            }
            Self::InvalidComponentType => {
                formatter.write_str("component type id must not be empty")
            }
            Self::InvalidExtension => formatter.write_str("extension name must not be empty"),
            Self::InvalidPresenter => formatter.write_str("presenter name must not be empty"),
            Self::InvalidInput => formatter.write_str("input contains a non-finite coordinate"),
            Self::InvalidVirtualization => {
                formatter.write_str("virtualized component state is inconsistent")
            }
            Self::EventOverflow(id) => {
                write!(
                    formatter,
                    "view {} emitted too many events in one update",
                    id.get()
                )
            }
            Self::InvalidComponentValue(id) => {
                write!(
                    formatter,
                    "view {} received an invalid component value",
                    id.get()
                )
            }
            Self::InvalidComponentHierarchy { parent, child } => write!(
                formatter,
                "view {} is not a valid child of component {}",
                child.get(),
                parent.get()
            ),
            Self::InvalidListItemSlots { item, slot } => match slot {
                Some(slot) => write!(
                    formatter,
                    "view {} has an invalid list-item slot {}",
                    item.get(),
                    slot.get()
                ),
                None => write!(
                    formatter,
                    "view {} has duplicate list-item slots",
                    item.get()
                ),
            },
            Self::InvalidFeedbackSlots { parent, slot } => match slot {
                Some(slot) => write!(
                    formatter,
                    "view {} has an invalid feedback child slot {}",
                    parent.get(),
                    slot.get()
                ),
                None => write!(
                    formatter,
                    "view {} has duplicate feedback child slots",
                    parent.get()
                ),
            },
            Self::InvalidModalSlots { parent, slot } => match slot {
                Some(slot) => write!(
                    formatter,
                    "view {} has an invalid modal child slot {}",
                    parent.get(),
                    slot.get()
                ),
                None => write!(
                    formatter,
                    "view {} has duplicate modal child slots",
                    parent.get()
                ),
            },
            Self::OverlayActivationTokenExhausted(host) => write!(
                formatter,
                "overlay activation identity for host {} is exhausted",
                host.get()
            ),
            Self::FrameDidNotSettle => {
                formatter.write_str("runtime frame did not settle within the bounded pass limit")
            }
            Self::DuplicateAssemblyKey { parent, key } => write!(
                formatter,
                "assembly key `{key}` is duplicated under view {}",
                parent.get()
            ),
            Self::DuplicateActivation => {
                formatter.write_str("an activation handler is already registered for this type")
            }
        }
    }
}

impl std::error::Error for FrameworkError {}

impl From<UiWorldError> for FrameworkError {
    fn from(value: UiWorldError) -> Self {
        Self::World(value)
    }
}

/// Owns typed view state while [`UiWorld`] remains the retained UI authority.
pub struct AppContext {
    world: UiWorld,
    views: HashMap<StableNodeId, Box<dyn Any + Send>>,
    event_handlers: HashMap<(StableNodeId, TypeId), Vec<EventHandler>>,
    actions: HashMap<ActionId, RegisteredAction>,
    extensions: HashSet<String>,
    components: ComponentRegistry,
    activations: HashMap<TypeId, ActivationFn>,
    secondary_presses: HashMap<TypeId, SecondaryPressFn>,
    assembled: HashMap<StableNodeId, HashMap<String, assemble::AssembledChild>>,
    component_lifecycle: ComponentLifecycle,
    next_id: u64,
    frame_profiler: FrameProfiler,
    last_profile: FrameProfile,
    profiling: bool,
    /// Cross-frame layout memo for [`Self::layout_document_scoped`].
    layout_cache: crate::RetainedLayoutCache,
    /// Nodes recomputed by the last layout pass (relayout + shape scope).
    last_layout_scope: Vec<StableNodeId>,
    /// Full (`force_full`) [`Self::layout_document`] calls on this context.
    layout_full_invocations: usize,
    /// Layout passes on this context, scoped and full.
    layout_invocations: usize,
    program_messages: Vec<ProgramMessage>,
    /// Live text drag-selection: pointer id, node, and the anchor offset.
    text_pointer_drag: Option<(u64, StableNodeId, usize)>,
    /// Last press inside a text editor for double/triple click counting.
    text_pointer_click: Option<TextPointerClick>,
    /// Horizontal goal column retained across chained vertical moves.
    caret_goal_x: Option<(StableNodeId, f32)>,
}

/// Bookkeeping for multi-click selection inside a text editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextPointerClick {
    pub pointer_id: u64,
    pub node: StableNodeId,
    pub at: std::time::Duration,
    pub x: f32,
    pub y: f32,
    pub count: u8,
}

/// Application-owned mapping between visible data keys and retained component
/// entities. Only the visible window is kept in the Runtime tree.
#[derive(Debug)]
pub struct VirtualListItems<K, C: ComponentView> {
    materializer: VirtualListMaterializer<K>,
    entities: HashMap<K, Entity<C>>,
}

/// Application-owned visible row/cell identities for a virtual Table. The
/// retained tree contains only the cross-product of the current row and column
/// windows.
#[derive(Debug)]
pub struct VirtualTableItems<R, C> {
    materializer: VirtualTableMaterializer<R, C>,
    rows: HashMap<R, Entity<TableRow>>,
    cells: HashMap<(R, C), Entity<TableCell>>,
}

impl<R, C> Default for VirtualTableItems<R, C> {
    fn default() -> Self {
        Self {
            materializer: VirtualTableMaterializer::default(),
            rows: HashMap::new(),
            cells: HashMap::new(),
        }
    }
}

impl<R, C> VirtualTableItems<R, C>
where
    R: Clone + Eq + Hash,
    C: Clone + Eq + Hash,
{
    pub fn mounted_rows(&self) -> &[R] {
        self.materializer.mounted_rows()
    }

    pub fn mounted_columns(&self) -> &[C] {
        self.materializer.mounted_columns()
    }

    pub fn row_entity(&self, key: &R) -> Option<Entity<TableRow>> {
        self.rows.get(key).copied()
    }

    pub fn cell_entity(&self, row: &R, column: &C) -> Option<Entity<TableCell>> {
        self.cells.get(&(row.clone(), column.clone())).copied()
    }
}

/// Application-owned mapping between visible flattened tree keys and retained
/// row entities. Collapsed descendants are not kept in the Runtime tree.
#[derive(Debug)]
pub struct VirtualTreeItems<K, C: ComponentView> {
    items: VirtualListItems<K, C>,
}

impl<K, C: ComponentView> Default for VirtualTreeItems<K, C> {
    fn default() -> Self {
        Self {
            items: VirtualListItems::default(),
        }
    }
}

impl<K, C> VirtualTreeItems<K, C>
where
    K: Clone + Eq + Hash,
    C: ComponentView,
{
    pub fn mounted_keys(&self) -> &[K] {
        self.items.mounted_keys()
    }

    pub fn entity(&self, key: &K) -> Option<Entity<C>> {
        self.items.entity(key)
    }
}

impl<K, C: ComponentView> Default for VirtualListItems<K, C> {
    fn default() -> Self {
        Self {
            materializer: VirtualListMaterializer::default(),
            entities: HashMap::new(),
        }
    }
}

impl<K, C> VirtualListItems<K, C>
where
    K: Clone + Eq + Hash,
    C: ComponentView,
{
    pub fn mounted_keys(&self) -> &[K] {
        self.materializer.mounted()
    }

    pub fn entity(&self, key: &K) -> Option<Entity<C>> {
        self.entities.get(key).copied()
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new()
    }
}

impl AppContext {
    pub fn new() -> Self {
        Self::from_world(UiWorld::new())
    }

    /// Wrap an existing retained tree.
    ///
    /// Hosts that already own node identity (Vue) create nodes on a [`UiWorld`]
    /// then adopt it here. The component ID allocator still starts at 1 and
    /// skips live or retired IDs; do not treat allocated chrome IDs as host
    /// tree identities.
    pub fn from_world(world: UiWorld) -> Self {
        let mut context = Self {
            world,
            views: HashMap::new(),
            event_handlers: HashMap::new(),
            actions: HashMap::new(),
            extensions: HashSet::new(),
            components: ComponentRegistry::default(),
            activations: HashMap::new(),
            secondary_presses: HashMap::new(),
            assembled: HashMap::new(),
            component_lifecycle: ComponentLifecycle::default(),
            next_id: 1,
            frame_profiler: FrameProfiler::new(),
            last_profile: FrameProfile::default(),
            profiling: false,
            layout_cache: crate::RetainedLayoutCache::default(),
            last_layout_scope: Vec::new(),
            layout_full_invocations: 0,
            layout_invocations: 0,
            program_messages: Vec::new(),
            text_pointer_drag: None,
            text_pointer_click: None,
            caret_goal_x: None,
        };
        context
            .install(&crate::builtin_components::NanaBuiltinComponents)
            .expect("builtin component registry");
        context.register_builtin_activations();
        #[cfg(feature = "syntax-highlighting")]
        {
            context
                .install(&crate::HighlightPresentation)
                .expect("default highlight presenter");
        }
        context
    }

    pub fn world(&self) -> &UiWorld {
        &self.world
    }

    /// Messages queued by [`ViewContext::dispatch_program`] since the last take.
    /// The Scene host drains these into `RuntimeProgram::update` on the next frame.
    pub fn take_program_messages(&mut self) -> Vec<Box<dyn Any + Send>> {
        std::mem::take(&mut self.program_messages)
    }

    pub fn has_program_messages(&self) -> bool {
        !self.program_messages.is_empty()
    }

    pub fn resolve_component_tag(&self, tag: &str) -> Option<&ComponentTypeId> {
        self.components.resolve_tag(tag)
    }

    /// Resolve an already-normalized tag (see [`normalize_tag`]).
    pub fn resolve_component_tag_normalized(
        &self,
        normalized_tag: &str,
    ) -> Option<&ComponentTypeId> {
        self.components.resolve_normalized(normalized_tag)
    }

    pub fn bind_semantic(
        &self,
        id: StableNodeId,
        spec: &SemanticSpec<'_>,
        mutations: &mut MutationQueue,
    ) -> Result<ComponentBindKind, FrameworkError> {
        let mut request = ComponentBindRequest {
            id,
            world: &self.world,
            mutations,
            spec,
        };
        let kind = self.components.bind(&mut request)?;
        mutations.set_component_type(id, Some(spec.type_id.clone()));
        Ok(kind)
    }

    fn view_entity<C: View>(&self, id: StableNodeId) -> Option<Entity<C>> {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<C>())
            .then(|| Entity::from_stable_id(id))
    }

    fn focused_editor<C: EditableText>(&self, document: DocumentId) -> Option<Entity<C>> {
        let (target, _) = self.world.focused_text_input(document)?;
        self.view_entity(target)
    }

    /// Mutable retained tree for compatibility hosts that already own node
    /// identity (Vue) and for frame systems not yet expressed on `AppContext`.
    pub fn world_mut(&mut self) -> &mut UiWorld {
        &mut self.world
    }

    /// Commit one validated batch without exposing mutable access to the
    /// retained authority. Compatibility adapters and frame drivers use this
    /// for layout writeback and platform state projection.
    pub fn commit_mutations(
        &mut self,
        mut mutations: MutationQueue,
    ) -> Result<crate::CommitReport, FrameworkError> {
        let parked = mutations
            .as_slice()
            .iter()
            .filter_map(|mutation| match mutation {
                crate::UiMutation::ParkSubtree { root } => Some(*root),
                _ => None,
            })
            .flat_map(|root| self.retained_subtree(root))
            .collect::<HashSet<_>>();
        for id in &parked {
            let Some(StandardVisual::Icon {
                icon,
                size,
                tooltip: Some(mut tooltip),
            }) = self.world.standard_visual(*id)
            else {
                continue;
            };
            if tooltip.open {
                tooltip.open = false;
                mutations.set_standard_visual(
                    *id,
                    Some(StandardVisual::Icon {
                        icon,
                        size,
                        tooltip: Some(tooltip),
                    }),
                );
            }
        }
        let inserted = mutations
            .as_slice()
            .iter()
            .filter_map(|mutation| match mutation {
                crate::UiMutation::Insert { child, .. } => Some(*child),
                _ => None,
            })
            .flat_map(|root| self.retained_subtree(root))
            .collect::<HashSet<_>>();
        let report = self.world.commit(mutations).map_err(FrameworkError::from)?;
        for id in parked {
            self.suspend_component_lifecycle(id);
        }
        for id in inserted {
            if self.world.is_mounted(id) {
                self.resume_component_lifecycle(id);
            }
        }
        Ok(report)
    }

    /// Drain deterministic work scheduled since the previous frame.
    pub fn take_system_work(&mut self) -> crate::SystemWork {
        self.world.take_system_work()
    }

    /// Algorithm-level counters from the last drained system batch.
    pub fn last_work_counters(&self) -> crate::WorkCounters {
        self.world.last_work_counters()
    }

    /// Record extract output onto the last drained work counters.
    pub fn record_extract(&mut self, extracted: &[crate::ExtractedNode]) {
        self.world.record_extract(extracted);
    }

    /// Open a product-frame profiler and work-counter accumulator.
    pub fn begin_frame_profile(&mut self) {
        self.frame_profiler = FrameProfiler::new();
        self.frame_profiler.mark_runtime_unsupported();
        self.profiling = true;
        self.world.begin_frame_counters();
    }

    pub fn finish_frame_profile(&mut self) {
        self.world.end_frame_counters();
        self.profiling = false;
        let profile = std::mem::take(&mut self.frame_profiler).finish();
        // Match last_work_counters: an idle flush (no stage ran) must not wipe
        // the last non-empty product profile.
        if profile.any_stage_ran() {
            self.last_profile = profile;
        }
    }

    pub fn last_frame_profile(&self) -> &FrameProfile {
        &self.last_profile
    }

    fn stage_clock(&self) -> Option<Instant> {
        self.profiling.then(Instant::now)
    }

    fn record_stage(&mut self, stage: FrameStage, started: Option<Instant>) {
        if let Some(started) = started {
            self.frame_profiler.record(stage, started.elapsed());
        }
    }

    /// Record a stage duration while a product frame is open.
    pub fn time_stage_duration(&mut self, stage: FrameStage, duration: Duration) {
        if self.profiling {
            self.frame_profiler.record(stage, duration);
        }
    }

    /// Return a drained system batch to the scheduler after a canonical frame
    /// fails. Frame drivers should restore every consumed batch before retry.
    pub fn restore_system_work(&mut self, work: crate::SystemWork) {
        self.world.restore_system_work(work);
    }

    /// Resolve inherited style for the supplied dirty nodes.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), FrameworkError> {
        let started = self.stage_clock();
        let result = self.world.resolve_styles(ids).map_err(FrameworkError::from);
        self.record_stage(FrameStage::Style, started);
        result
    }

    /// Derive registered text presentations for scheduled nodes.
    pub fn resolve_presentations(&mut self, ids: &[StableNodeId]) -> Result<(), FrameworkError> {
        self.world
            .resolve_presentations(ids)
            .map_err(FrameworkError::from)
    }

    /// Shape only scheduled text through the host's real text backend.
    pub fn shape_text(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl crate::TextShaper,
    ) -> Result<(), FrameworkError> {
        let started = self.stage_clock();
        let result = self
            .world
            .shape_text(ids, shaper)
            .map_err(FrameworkError::from);
        self.record_stage(FrameStage::TextShape, started);
        result
    }

    pub fn shape_text_for_layout(
        &mut self,
        document: DocumentId,
        shaper: &mut impl crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let started = self.stage_clock();
        let result = self
            .world
            .shape_text_for_layout(document, shaper)
            .map_err(FrameworkError::from);
        self.record_stage(FrameStage::TextShape, started);
        result
    }

    /// [`Self::shape_text_for_layout`] restricted to `ids` (the last layout
    /// scope): nodes outside it keep shapes matching their unchanged boxes.
    pub fn shape_text_for_layout_scoped(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let started = self.stage_clock();
        let result = self
            .world
            .shape_text_for_layout_scoped(ids, shaper)
            .map_err(FrameworkError::from);
        self.record_stage(FrameStage::TextShape, started);
        result
    }
    /// Compute and atomically publish canonical Runtime layout for one window.
    ///
    /// Full pass: recomputes every box and rebuilds the retained layout cache.
    /// Used when viewport semantics changed or a complete layout is required.
    pub fn layout_document(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.layout_document_impl(document, viewport, &[], true)
    }

    /// [`Self::layout_document`] restricted to the ancestor closure of
    /// `dirty`. Clean subtrees reuse the retained cache, so the cost scales
    /// with the change, not the document. [`Self::take_last_layout_scope`]
    /// reports the recomputed set for scoped text re-shaping.
    pub fn layout_document_scoped(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        dirty: &[StableNodeId],
    ) -> Result<crate::CommitReport, FrameworkError> {
        let mut dirty = dirty.to_vec();
        dirty.sort_unstable();
        dirty.dedup();
        self.layout_document_impl(document, viewport, &dirty, false)
    }

    /// Relayout after a viewport change. Document roots plus any live
    /// `position: fixed` / `vw` / `vh` boxes are dirty; unchanged subtrees keep
    /// the retained cache.
    pub fn layout_document_for_viewport(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) -> Result<crate::CommitReport, FrameworkError> {
        let mut dirty = self.world.document_roots(document);
        dirty.extend(self.world.viewport_basis_ids());
        self.layout_document_impl(document, viewport, &dirty, false)
    }

    /// Nodes recomputed by the most recent layout pass; drains on read.
    pub fn take_last_layout_scope(&mut self) -> Vec<StableNodeId> {
        std::mem::take(&mut self.last_layout_scope)
    }

    /// Nodes carrying an undrained LAYOUT-dirty bit (e.g. set by a shaping
    /// pass after the work drain). Sorted for determinism.
    pub fn pending_layout_dirty(&mut self) -> Vec<StableNodeId> {
        self.world.pending_layout_dirty()
    }

    /// Full (`force_full`) layout passes, which discard the retained cache.
    pub fn layout_full_invocations(&self) -> usize {
        self.layout_full_invocations
    }

    /// Layout passes, scoped and full.
    pub fn layout_invocations(&self) -> usize {
        self.layout_invocations
    }

    fn layout_document_impl(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        dirty: &[StableNodeId],
        force_full: bool,
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.layout_invocations += 1;
        if force_full {
            self.layout_full_invocations += 1;
        }
        let started = self.stage_clock();
        self.component_lifecycle
            .viewports
            .insert(document, viewport);
        let result = (|| {
            self.position_open_tooltips(document)?;
            let layouts = crate::RuntimeLayoutEngine.layout_document_scoped(
                &self.world,
                document,
                viewport,
                dirty,
                &mut self.layout_cache,
                force_full,
            )?;
            let mut mutations = MutationQueue::new();
            let mut scope = Vec::with_capacity(layouts.len());
            for (id, layout) in layouts {
                scope.push(id);
                if self.world.layout_box(id) != Some(layout) {
                    mutations.write_layout(id, layout);
                }
            }
            self.last_layout_scope = scope;
            let report = self.commit_mutations(mutations)?;
            self.publish_document_scroll_metrics(document)?;
            Ok(report)
        })();
        self.record_stage(FrameStage::Layout, started);
        result
    }

    fn publish_document_scroll_metrics(
        &mut self,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        let updates = self
            .world
            .document_order(document)
            .into_iter()
            .filter(|id| self.is_scroll_view(*id))
            .filter_map(|id| {
                let metrics = self.scroll_metrics_from_layout(id)?;
                (self.world.scroll_metrics(id) != Some(metrics))
                    .then_some((Entity::<ScrollView>::from_stable_id(id), metrics))
            })
            .collect::<Vec<_>>();
        for (entity, metrics) in updates {
            self.set_scroll_metrics(entity, metrics)?;
        }
        Ok(())
    }

    fn scroll_metrics_from_layout(&self, id: StableNodeId) -> Option<ScrollMetrics> {
        let viewport = self.world.layout_box(id)?;
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return None;
        }
        let mut content_width = viewport.width;
        let mut content_height = viewport.height;
        let mut stack = self
            .world
            .node(id)
            .map(|node| node.children)
            .unwrap_or_default();
        while let Some(child) = stack.pop() {
            if self
                .world
                .node_style(child)
                .is_some_and(|style| style.layout.omits_box())
            {
                continue;
            }
            if let Some(bounds) = self.world.layout_box(child) {
                content_width = content_width.max(bounds.x + bounds.width - viewport.x);
                content_height = content_height.max(bounds.y + bounds.height - viewport.y);
            }
            if let Some(node) = self.world.node(child) {
                stack.extend(node.children);
            }
        }
        Some(ScrollMetrics {
            viewport_width: viewport.width,
            viewport_height: viewport.height,
            content_width: content_width.max(0.0),
            content_height: content_height.max(0.0),
        })
    }

    /// Re-queue LAYOUT after a host drained a frame without measuring.
    pub fn defer_layout(&mut self, ids: &[StableNodeId]) {
        for &id in ids {
            self.world.mark_layout(id);
        }
    }

    /// Rebuild the compact hit index for one document after layout or input
    /// work. The retained hierarchy remains private to this context.
    pub fn rebuild_hit_test(&mut self, document: DocumentId) {
        let started = self.stage_clock();
        self.world.rebuild_hit_test(document);
        self.record_stage(FrameStage::HitTest, started);
    }

    /// Patch only the subtrees covering `dirty`, falling back to a full document
    /// rebuild when the change is structural. See
    /// [`UiWorld::rebuild_hit_test_scoped`].
    pub fn rebuild_hit_test_for(&mut self, document: DocumentId, dirty: &[StableNodeId]) {
        let started = self.stage_clock();
        if !self.world.rebuild_hit_test_scoped(document, dirty) {
            self.world.rebuild_hit_test(document);
        }
        self.record_stage(FrameStage::HitTest, started);
    }

    /// Drain recorded scroll deltas for the in-place hit-index patch.
    pub fn take_scroll_hit_updates(&mut self) -> Vec<(StableNodeId, [f32; 2])> {
        self.world.take_scroll_hit_updates()
    }

    /// See [`UiWorld::hit_test_work_is_scroll_only`].
    pub fn hit_test_work_is_scroll_only(
        &self,
        input: &[StableNodeId],
        updates: &[(StableNodeId, [f32; 2])],
    ) -> bool {
        self.world.hit_test_work_is_scroll_only(input, updates)
    }

    /// Pre-compose a scroll translation onto the scroller subtree's hit
    /// entries instead of rebuilding the document index.
    pub fn update_hit_test_scroll(
        &mut self,
        document: DocumentId,
        scroller: StableNodeId,
        delta: [f32; 2],
    ) {
        let started = self.stage_clock();
        self.world.update_hit_test_scroll(document, scroller, delta);
        self.record_stage(FrameStage::HitTest, started);
    }

    pub fn next_animation_deadline(&self) -> Option<Duration> {
        let loading_deadline = self
            .component_lifecycle
            .loading
            .keys()
            .any(|target| self.world.is_mounted(*target))
            .then_some(self.component_lifecycle.next_loading_frame)
            .flatten();
        self.world
            .next_animation_deadline()
            .into_iter()
            .chain(loading_deadline)
            .chain(
                self.component_lifecycle
                    .tooltips
                    .iter()
                    .filter(|(target, _)| self.world.is_mounted(**target))
                    .filter_map(|(_, tooltip)| tooltip.show_at),
            )
            .min()
    }

    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        self.component_lifecycle.now = now;
        let mut frame = self.world.advance_animations(now);
        let tooltip_targets = self
            .component_lifecycle
            .tooltips
            .iter()
            .filter_map(|(&target, tooltip)| {
                if !self.world.is_mounted(target) {
                    return None;
                }
                tooltip.show_at.filter(|deadline| *deadline <= now)?;
                Some(target)
            })
            .collect::<Vec<_>>();
        for target in tooltip_targets {
            if self.open_tooltip(target).unwrap_or(false) {
                frame.component_updates.push(target);
            }
        }
        if self
            .component_lifecycle
            .next_loading_frame
            .is_some_and(|deadline| deadline <= now)
        {
            let phase = (now.as_secs_f32() / LOADING_CYCLE.as_secs_f32()).rem_euclid(1.0);
            let loading = self
                .component_lifecycle
                .loading
                .iter()
                .filter(|(target, _)| self.world.is_mounted(**target))
                .map(|(&target, &kind)| (target, kind))
                .collect::<Vec<_>>();
            for (target, kind) in loading {
                let changed = match kind {
                    LoadingComponent::Button => self
                        .update_component(Entity::<Button>::from_stable_id(target), |button, _| {
                            button.loading_phase = phase;
                        })
                        .is_ok(),
                    LoadingComponent::Switch => self
                        .update_component(Entity::<Switch>::from_stable_id(target), |switch, _| {
                            switch.loading_phase = phase;
                        })
                        .is_ok(),
                    LoadingComponent::Card => self
                        .update_component(
                            Entity::<crate::Card>::from_stable_id(target),
                            |card, _| {
                                card.loading_phase = phase;
                            },
                        )
                        .is_ok(),
                };
                if changed {
                    frame.component_updates.push(target);
                }
            }
            self.component_lifecycle.next_loading_frame = self
                .component_lifecycle
                .loading
                .keys()
                .any(|target| self.world.is_mounted(*target))
                .then(|| now.checked_add(COMPONENT_FRAME_INTERVAL))
                .flatten();
        }
        let section_targets = frame
            .samples
            .iter()
            .map(|sample| sample.target)
            .filter(|target| {
                self.views
                    .get(target)
                    .is_some_and(|view| view.is::<SidebarSection>())
            })
            .collect::<Vec<_>>();
        for target in section_targets {
            if self
                .update_component(
                    Entity::<SidebarSection>::from_stable_id(target),
                    |section, _| {
                        section.animation_progress = section.state.expansion(now);
                    },
                )
                .is_ok()
            {
                frame.component_updates.push(target);
            }
        }
        frame.next_deadline = self.next_animation_deadline();
        frame
    }

    pub fn focused_text_input(
        &self,
        document: DocumentId,
    ) -> Option<(StableNodeId, &TextInputState)> {
        self.world.focused_text_input(document)
    }

    /// Change the retained theme once and invalidate only computed paint.
    /// Re-applying the active mode defaults is a true no-op.
    ///
    /// Resets palette alphas. Hosts that apply window backdrop follow with
    /// [`Self::set_style_tokens`].
    pub fn set_theme(&mut self, mode: ThemeMode) -> Result<bool, FrameworkError> {
        if self.world.style_model() == nana_ui_core::StyleModelRef::new(mode) {
            return Ok(false);
        }
        let mut queue = MutationQueue::new();
        queue.set_theme(mode);
        self.world.commit(queue)?;
        Ok(true)
    }

    /// Install Style Model tokens, including backdrop alphas on Surface /
    /// Background / Titlebar.
    pub fn set_style_tokens(
        &mut self,
        mode: ThemeMode,
        metrics: nana_ui_core::ThemeMetrics,
        palette: nana_ui_core::SemanticPalette,
        titlebar: nana_ui_core::SemanticColor,
    ) -> Result<bool, FrameworkError> {
        let next = nana_ui_core::StyleModelRef::with_tokens(mode, metrics, palette, titlebar);
        if self.world.style_model() == next {
            return Ok(false);
        }
        let mut queue = MutationQueue::new();
        queue.set_style_tokens(mode, metrics, palette, titlebar);
        self.world.commit(queue)?;
        Ok(true)
    }

    pub fn create_view<V: View>(
        &mut self,
        document: DocumentId,
        kind: NodeKind,
        view: V,
    ) -> Result<Entity<V>, FrameworkError> {
        let id = self.allocate_id();
        let mut queue = MutationQueue::new();
        queue.create(id, document, kind);
        self.world.commit(queue)?;
        self.views.insert(id, Box::new(view));
        Ok(Entity::from_stable_id(id))
    }

    /// Create and atomically project a backend-neutral Nana component.
    pub fn create_component<C: ComponentView>(
        &mut self,
        document: DocumentId,
        component: C,
    ) -> Result<Entity<C>, FrameworkError> {
        let id = self.allocate_id();
        let mut queue = MutationQueue::new();
        queue.create(id, document, component.node_kind());
        component.project(id, &self.world, &mut queue);
        self.stamp_component_type::<C>(id, &mut queue);
        self.world.commit(queue)?;
        self.views.insert(id, Box::new(component));
        self.sync_component_lifecycle(id)?;
        Ok(Entity::from_stable_id(id))
    }

    /// Bind a typed component view onto an existing world node.
    ///
    /// The node must already exist — hosts such as Vue own identity and must
    /// not allocate IDs through [`Self::create_component`]. This replaces any
    /// previous view at `id`, projects `component` into the retained tree, and
    /// commits internally so [`Self::read`], [`Self::update_component`], and
    /// `assemble_*` can run immediately.
    pub fn bind_component<C: ComponentView>(
        &mut self,
        id: StableNodeId,
        component: C,
    ) -> Result<Entity<C>, FrameworkError> {
        if !self.world.contains(id) {
            return Err(FrameworkError::MissingView(id));
        }
        let mut queue = MutationQueue::new();
        component.project(id, &self.world, &mut queue);
        self.stamp_component_type::<C>(id, &mut queue);
        self.commit_mutations(queue)?;
        self.views.insert(id, Box::new(component));
        self.sync_component_lifecycle(id)?;
        Ok(Entity::from_stable_id(id))
    }

    /// Create a component whose view and handlers are retained without making
    /// it a document root. Inserting the entity mounts the complete subtree.
    pub fn create_detached_component<C: ComponentView>(
        &mut self,
        document: DocumentId,
        component: C,
    ) -> Result<Entity<C>, FrameworkError> {
        let id = self.allocate_id();
        let mut queue = MutationQueue::new();
        queue.create(id, document, component.node_kind());
        component.project(id, &self.world, &mut queue);
        self.stamp_component_type::<C>(id, &mut queue);
        queue.park_subtree(id);
        self.world.commit(queue)?;
        self.views.insert(id, Box::new(component));
        self.sync_component_lifecycle(id)?;
        Ok(Entity::from_stable_id(id))
    }

    pub fn append_child<P: View, C: View>(
        &mut self,
        parent: Entity<P>,
        child: Entity<C>,
    ) -> Result<(), FrameworkError> {
        self.read(parent, |_| ())?;
        self.read(child, |_| ())?;
        let mut queue = MutationQueue::new();
        queue.insert(parent.id, child.id, None);
        self.commit_mutations(queue)?;
        Ok(())
    }

    /// Reconcile a virtual List to one visible keyed window. Creation,
    /// removal, and final child order share one Runtime commit; the external
    /// materializer is published only after that commit succeeds.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize_virtual_list<K, C>(
        &mut self,
        list: Entity<List>,
        items: &mut VirtualListItems<K, C>,
        layout: &VirtualListLayout,
        scroll_offset: f32,
        viewport_extent: f32,
        overscan_extent: f32,
        key_at: impl FnMut(usize) -> K,
        mut build: impl FnMut(usize, &K) -> C,
    ) -> Result<VirtualListWindow, FrameworkError>
    where
        K: Clone + Eq + Hash,
        C: ComponentView,
    {
        self.read(list, |_| ())?;
        let plan = items
            .materializer
            .prepare(
                layout,
                scroll_offset,
                viewport_extent,
                overscan_extent,
                key_at,
            )
            .map_err(|_| FrameworkError::InvalidVirtualization)?;
        let list_node = self
            .world
            .node(list.id)
            .ok_or(FrameworkError::MissingView(list.id))?;
        let mounted = items
            .materializer
            .mounted()
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let owned = items
            .entities
            .values()
            .map(|entity| entity.id)
            .collect::<HashSet<_>>();
        if mounted.len() != items.materializer.mounted().len()
            || mounted.len() != items.entities.len()
            || items.entities.keys().any(|key| !mounted.contains(key))
            || owned.len() != items.entities.len()
            || list_node.children.len() != owned.len()
            || list_node
                .children
                .iter()
                .any(|child| !owned.contains(child))
            || items.entities.values().any(|entity| {
                self.world
                    .node(entity.id)
                    .is_none_or(|node| node.parent != Some(list.id))
                    || self
                        .views
                        .get(&entity.id)
                        .is_none_or(|view| !view.is::<C>())
            })
        {
            return Err(FrameworkError::InvalidVirtualization);
        }
        if plan.mounts.is_empty() && plan.unmounts.is_empty() {
            let desired = plan
                .order
                .iter()
                .map(|key| {
                    items
                        .entities
                        .get(key)
                        .map(|entity| entity.id)
                        .ok_or(FrameworkError::InvalidVirtualization)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if desired == list_node.children {
                let window = plan.window.clone();
                items
                    .materializer
                    .commit(plan)
                    .map_err(|_| FrameworkError::InvalidVirtualization)?;
                return Ok(window);
            }
        }

        let mut removed_nodes = HashSet::new();
        for key in &plan.unmounts {
            let entity = items
                .entities
                .get(key)
                .ok_or(FrameworkError::InvalidVirtualization)?;
            let mut stack = vec![entity.id];
            while let Some(id) = stack.pop() {
                let node = self
                    .world
                    .node(id)
                    .ok_or(FrameworkError::InvalidVirtualization)?;
                stack.extend(node.children);
                removed_nodes.insert(id);
            }
        }

        let mut mutations = MutationQueue::new();
        for key in &plan.unmounts {
            mutations.despawn_subtree(items.entities[key].id);
        }
        let mut staged = Vec::with_capacity(plan.mounts.len());
        for mount in &plan.mounts {
            let component = build(mount.index, &mount.key);
            let id = self.allocate_id();
            mutations.create(id, list_node.document, component.node_kind());
            component.project(id, &self.world, &mut mutations);
            staged.push((mount.key.clone(), Entity::from_stable_id(id), component));
        }

        let mut next_entities = items.entities.clone();
        for key in &plan.unmounts {
            next_entities.remove(key);
        }
        for (key, entity, _) in &staged {
            next_entities.insert(key.clone(), *entity);
        }
        for key in &plan.order {
            let entity = next_entities
                .get(key)
                .ok_or(FrameworkError::InvalidVirtualization)?;
            if self.world.contains(entity.id)
                && self.world.node(entity.id).and_then(|node| node.parent) != Some(list.id)
            {
                return Err(FrameworkError::InvalidVirtualization);
            }
            mutations.insert(list.id, entity.id, None);
        }

        self.world.commit(mutations)?;
        self.remove_event_handlers_for(&removed_nodes);
        for id in &removed_nodes {
            self.views.remove(id);
        }
        for (_, entity, component) in staged {
            self.views.insert(entity.id, Box::new(component));
        }
        items.entities = next_entities;
        let window = plan.window.clone();
        items
            .materializer
            .commit(plan)
            .map_err(|error| match error {
                VirtualListMaterializationError::DuplicateKey
                | VirtualListMaterializationError::StalePlan => {
                    FrameworkError::InvalidVirtualization
                }
            })?;
        Ok(window)
    }

    /// Reconcile both visible axes of a virtual Table in one Runtime commit.
    /// Rows and cells with overlapping keys retain their stable entities.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize_virtual_table<R, C>(
        &mut self,
        table: Entity<Table>,
        items: &mut VirtualTableItems<R, C>,
        layout: &VirtualTableLayout,
        scroll: (f32, f32),
        viewport: (f32, f32),
        overscan: (f32, f32),
        row_key_at: impl FnMut(usize) -> R,
        column_key_at: impl FnMut(usize) -> C,
        mut build_row: impl FnMut(usize, &R) -> TableRow,
        mut build_cell: impl FnMut(usize, &R, usize, &C) -> TableCell,
    ) -> Result<VirtualTableWindow, FrameworkError>
    where
        R: Clone + Eq + Hash,
        C: Clone + Eq + Hash,
    {
        self.read(table, |_| ())?;
        let plan = items
            .materializer
            .prepare(
                layout,
                scroll,
                viewport,
                overscan,
                row_key_at,
                column_key_at,
            )
            .map_err(|_| FrameworkError::InvalidVirtualization)?;
        let table_node = self
            .world
            .node(table.id)
            .ok_or(FrameworkError::MissingView(table.id))?;
        let mounted_rows = items
            .materializer
            .mounted_rows()
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let mounted_columns = items
            .materializer
            .mounted_columns()
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let row_ids = items
            .rows
            .values()
            .map(|entity| entity.id)
            .collect::<HashSet<_>>();
        let cell_ids = items
            .cells
            .values()
            .map(|entity| entity.id)
            .collect::<HashSet<_>>();
        let expected_cell_count = items
            .rows
            .len()
            .checked_mul(mounted_columns.len())
            .ok_or(FrameworkError::InvalidVirtualization)?;
        let invalid_rows = mounted_rows.len() != items.materializer.mounted_rows().len()
            || mounted_rows.len() != items.rows.len()
            || items.rows.keys().any(|key| !mounted_rows.contains(key))
            || row_ids.len() != items.rows.len()
            || table_node.children.len() != row_ids.len()
            || table_node
                .children
                .iter()
                .any(|child| !row_ids.contains(child));
        let invalid_columns = mounted_columns.len() != items.materializer.mounted_columns().len()
            || items.cells.len() != expected_cell_count
            || cell_ids.len() != items.cells.len()
            || items.cells.keys().any(|(row, column)| {
                !mounted_rows.contains(row) || !mounted_columns.contains(column)
            });
        if invalid_rows || invalid_columns {
            return Err(FrameworkError::InvalidVirtualization);
        }
        for (row_key, row_entity) in &items.rows {
            let Some(row_node) = self.world.node(row_entity.id) else {
                return Err(FrameworkError::InvalidVirtualization);
            };
            if row_node.parent != Some(table.id)
                || self
                    .views
                    .get(&row_entity.id)
                    .is_none_or(|view| !view.is::<TableRow>())
                || row_node.children.len() != mounted_columns.len()
            {
                return Err(FrameworkError::InvalidVirtualization);
            }
            for column_key in &mounted_columns {
                let Some(cell) = items.cells.get(&(row_key.clone(), column_key.clone())) else {
                    return Err(FrameworkError::InvalidVirtualization);
                };
                if !row_node.children.contains(&cell.id)
                    || self
                        .world
                        .node(cell.id)
                        .is_none_or(|node| node.parent != Some(row_entity.id))
                    || self
                        .views
                        .get(&cell.id)
                        .is_none_or(|view| !view.is::<TableCell>())
                {
                    return Err(FrameworkError::InvalidVirtualization);
                }
            }
        }

        let desired_rows = plan
            .rows
            .order
            .iter()
            .map(|key| {
                items
                    .rows
                    .get(key)
                    .map(|entity| entity.id)
                    .ok_or(FrameworkError::InvalidVirtualization)
            })
            .collect::<Result<Vec<_>, _>>();
        if plan.rows.mounts.is_empty()
            && plan.rows.unmounts.is_empty()
            && plan.columns.mounts.is_empty()
            && plan.columns.unmounts.is_empty()
            && desired_rows
                .as_ref()
                .is_ok_and(|rows| *rows == table_node.children)
            && plan.rows.order.iter().all(|row| {
                let Some(row_entity) = items.rows.get(row) else {
                    return false;
                };
                let desired_cells = plan
                    .columns
                    .order
                    .iter()
                    .filter_map(|column| items.cells.get(&(row.clone(), column.clone())))
                    .map(|entity| entity.id)
                    .collect::<Vec<_>>();
                self.world
                    .node(row_entity.id)
                    .is_some_and(|node| node.children == desired_cells)
            })
        {
            let window = plan.window.clone();
            items
                .materializer
                .commit(plan)
                .map_err(|_| FrameworkError::InvalidVirtualization)?;
            return Ok(window);
        }

        let removed_rows = plan.rows.unmounts.iter().cloned().collect::<HashSet<_>>();
        let mut removed_nodes = HashSet::new();
        for row in &plan.rows.unmounts {
            let mut stack = vec![items.rows[row].id];
            while let Some(id) = stack.pop() {
                let node = self
                    .world
                    .node(id)
                    .ok_or(FrameworkError::InvalidVirtualization)?;
                stack.extend(node.children);
                removed_nodes.insert(id);
            }
        }
        for row in items.rows.keys().filter(|row| !removed_rows.contains(*row)) {
            for column in &plan.columns.unmounts {
                let cell = items
                    .cells
                    .get(&(row.clone(), column.clone()))
                    .ok_or(FrameworkError::InvalidVirtualization)?;
                removed_nodes.insert(cell.id);
            }
        }

        let mut mutations = MutationQueue::new();
        for row in &plan.rows.unmounts {
            mutations.despawn_subtree(items.rows[row].id);
        }
        for row in items.rows.keys().filter(|row| !removed_rows.contains(*row)) {
            for column in &plan.columns.unmounts {
                mutations.despawn_subtree(items.cells[&(row.clone(), column.clone())].id);
            }
        }

        let row_indices = plan
            .window
            .rows
            .range
            .clone()
            .zip(plan.rows.order.iter().cloned())
            .map(|(index, key)| (key, index))
            .collect::<HashMap<_, _>>();
        let column_indices = plan
            .window
            .columns
            .range
            .clone()
            .zip(plan.columns.order.iter().cloned())
            .map(|(index, key)| (key, index))
            .collect::<HashMap<_, _>>();
        let new_rows = plan
            .rows
            .mounts
            .iter()
            .map(|mount| mount.key.clone())
            .collect::<HashSet<_>>();
        let new_columns = plan
            .columns
            .mounts
            .iter()
            .map(|mount| mount.key.clone())
            .collect::<HashSet<_>>();
        let mut next_rows = items.rows.clone();
        let mut next_cells = items.cells.clone();
        for row in &plan.rows.unmounts {
            next_rows.remove(row);
            next_cells.retain(|(cell_row, _), _| cell_row != row);
        }
        for column in &plan.columns.unmounts {
            next_cells.retain(|(_, cell_column), _| cell_column != column);
        }

        let mut staged_rows = Vec::with_capacity(plan.rows.mounts.len());
        for mount in &plan.rows.mounts {
            let component = build_row(mount.index, &mount.key);
            let id = self.allocate_id();
            mutations.create(id, table_node.document, component.node_kind());
            component.project(id, &self.world, &mut mutations);
            let entity = Entity::from_stable_id(id);
            next_rows.insert(mount.key.clone(), entity);
            staged_rows.push((entity, component));
        }

        let mut staged_cells = Vec::new();
        for row in &plan.rows.order {
            for column in &plan.columns.order {
                if !new_rows.contains(row) && !new_columns.contains(column) {
                    continue;
                }
                let row_index = row_indices[row];
                let column_index = column_indices[column];
                let component = build_cell(row_index, row, column_index, column);
                let id = self.allocate_id();
                mutations.create(id, table_node.document, component.node_kind());
                component.project(id, &self.world, &mut mutations);
                let entity = Entity::from_stable_id(id);
                next_cells.insert((row.clone(), column.clone()), entity);
                staged_cells.push((entity, component));
            }
        }
        for row in &plan.rows.order {
            let row_entity = next_rows
                .get(row)
                .ok_or(FrameworkError::InvalidVirtualization)?;
            mutations.insert(table.id, row_entity.id, None);
            for column in &plan.columns.order {
                let cell = next_cells
                    .get(&(row.clone(), column.clone()))
                    .ok_or(FrameworkError::InvalidVirtualization)?;
                mutations.insert(row_entity.id, cell.id, None);
            }
        }

        self.world.commit(mutations)?;
        self.remove_event_handlers_for(&removed_nodes);
        for id in &removed_nodes {
            self.views.remove(id);
        }
        for (entity, component) in staged_rows {
            self.views.insert(entity.id, Box::new(component));
        }
        for (entity, component) in staged_cells {
            self.views.insert(entity.id, Box::new(component));
        }
        items.rows = next_rows;
        items.cells = next_cells;
        let window = plan.window.clone();
        items
            .materializer
            .commit(plan)
            .map_err(|_| FrameworkError::InvalidVirtualization)?;
        Ok(window)
    }

    /// Reconcile a virtual Tree to one visible keyed window of flattened
    /// expanded rows. Creation, removal, and final child order share one
    /// Runtime commit; collapsed descendants are never spawned.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize_virtual_tree<K, C>(
        &mut self,
        tree: Entity<List>,
        items: &mut VirtualTreeItems<K, C>,
        layout: &VirtualTreeLayout,
        scroll_offset: f32,
        viewport_extent: f32,
        overscan_extent: f32,
        key_at: impl FnMut(usize) -> K,
        build: impl FnMut(usize, &K) -> C,
    ) -> Result<VirtualListWindow, FrameworkError>
    where
        K: Clone + Eq + Hash,
        C: ComponentView,
    {
        self.materialize_virtual_list(
            tree,
            &mut items.items,
            layout.row_layout(),
            scroll_offset,
            viewport_extent,
            overscan_extent,
            key_at,
            build,
        )
    }

    /// Dispatch a semantic activation through the component's closure-event
    /// path. Disabled buttons do not emit or mutate retained state.
    pub fn activate_button(&mut self, entity: Entity<Button>) -> Result<bool, FrameworkError> {
        self.activate_component(entity, |button| button.disabled || button.loading)
    }

    pub fn activate_icon_button(
        &mut self,
        entity: Entity<IconButton>,
    ) -> Result<bool, FrameworkError> {
        self.activate_component(entity, |button| button.disabled)
    }

    pub fn activate_list_item(&mut self, entity: Entity<ListItem>) -> Result<bool, FrameworkError> {
        self.activate_component(entity, |item| item.disabled)
    }

    pub fn activate_sidebar_row(
        &mut self,
        entity: Entity<SidebarRow>,
    ) -> Result<bool, FrameworkError> {
        self.activate_component(entity, |row| row.disabled())
    }

    pub fn activate_sidebar_footer_button(
        &mut self,
        entity: Entity<SidebarFooterButton>,
    ) -> Result<bool, FrameworkError> {
        self.activate_component(entity, |button| button.disabled)
    }

    fn enclosing_sidebar_row(&self, id: StableNodeId) -> Option<Entity<SidebarRow>> {
        let mut current = Some(id);
        while let Some(id) = current {
            if self
                .views
                .get(&id)
                .is_some_and(|view| view.is::<SidebarRow>())
            {
                return Some(Entity::from_stable_id(id));
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        None
    }

    fn sync_sidebar_row_hover(
        &mut self,
        row: Option<Entity<SidebarRow>>,
        hovered: bool,
    ) -> Result<(), FrameworkError> {
        let Some(row) = row else {
            return Ok(());
        };
        if self.read(row, |row| row.hovered == hovered)? {
            return Ok(());
        }
        self.update_component(row, |row, _| {
            row.hovered = hovered;
        })?;
        Ok(())
    }

    fn enclosing_sidebar_section(&self, id: StableNodeId) -> Option<Entity<SidebarSection>> {
        let mut current = Some(id);
        while let Some(id) = current {
            if self
                .views
                .get(&id)
                .is_some_and(|view| view.is::<SidebarSection>())
            {
                return Some(Entity::from_stable_id(id));
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        None
    }

    fn node_is_or_under(&self, root: Option<StableNodeId>, target: StableNodeId) -> bool {
        let Some(root) = root else {
            return false;
        };
        let mut current = Some(target);
        while let Some(id) = current {
            if id == root {
                return true;
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        false
    }

    fn sidebar_section_header_hovered(
        &self,
        section: Entity<SidebarSection>,
        hover: Option<StableNodeId>,
    ) -> bool {
        let Some(hover) = hover else {
            return false;
        };
        let Ok((header, body, tools, title_slot, count_slot, disclosure)) =
            self.read(section, |section| {
                (
                    section.header,
                    section.body,
                    section.tools,
                    section.title_slot,
                    section.count_slot,
                    section.disclosure,
                )
            })
        else {
            return false;
        };
        if self.node_is_or_under(body, hover) {
            return false;
        }
        hover == section.stable_id()
            || self.node_is_or_under(header, hover)
            || self.node_is_or_under(tools, hover)
            || self.node_is_or_under(title_slot, hover)
            || self.node_is_or_under(count_slot, hover)
            || self.node_is_or_under(disclosure, hover)
    }

    fn sync_sidebar_section_hover(
        &mut self,
        node: Option<StableNodeId>,
        hover: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let Some(section) = node.and_then(|id| self.enclosing_sidebar_section(id)) else {
            return Ok(());
        };
        let hovered = self.sidebar_section_header_hovered(section, hover);
        if self.read(section, |section| section.header_hovered == hovered)? {
            return Ok(());
        }
        self.update_component(section, |section, _| {
            section.header_hovered = hovered;
        })?;
        Ok(())
    }

    pub fn activate_sidebar_section(
        &mut self,
        entity: Entity<SidebarSection>,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, |section| !section.collapsible || section.disabled)? {
            return Ok(false);
        }
        let now = self.component_lifecycle.now;
        let id = entity.id;
        self.update_component(entity, |section, cx| {
            section.state.toggle(now);
            section.animation_progress = section.state.expansion(now);
            if let Some(animation) = crate::AnimationId::new(id.get()) {
                cx.mutations().start_animation(crate::AnimationSpec {
                    id: animation,
                    target: id,
                    start: now,
                    duration: crate::SidebarSectionState::animation_duration(),
                    frame_interval: COMPONENT_FRAME_INTERVAL,
                    easing: crate::Easing::EaseOutCubic,
                    iteration_count: crate::AnimationIteration::ONCE,
                    direction: crate::AnimationDirection::Normal,
                    fill_mode: crate::AnimationFillMode::None,
                    play_state: crate::AnimationPlayState::Running,
                });
            }
            cx.emit(ToggleChanged {
                checked: section.state.expanded(),
            });
        })?;
        Ok(true)
    }

    pub fn activate_settings_collapsible_card(
        &mut self,
        entity: Entity<SettingsCollapsibleCard>,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, |card| card.disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |card, cx| {
            card.expanded = !card.expanded;
            cx.emit(ToggleChanged {
                checked: card.expanded,
            });
        })?;
        Ok(true)
    }

    pub fn activate_action_menu_item(
        &mut self,
        entity: Entity<ActionMenuItem>,
    ) -> Result<bool, FrameworkError> {
        let activated = self.activate_component(entity, |item| item.disabled)?;
        if activated {
            self.close_owning_menu(entity.id)?;
        }
        Ok(activated)
    }

    /// Picking a command dismisses the menu that offered it, so the caller does
    /// not have to mirror the open state just to close it again.
    fn close_owning_menu(&mut self, item: StableNodeId) -> Result<(), FrameworkError> {
        let mut current = self.world.node(item).and_then(|node| node.parent);
        while let Some(id) = current {
            if let Some(entity) = self.view_entity::<ActionMenu>(id) {
                if self.read(entity, |menu| menu.popover.open)? {
                    self.toggle_action_menu(entity)?;
                }
                return Ok(());
            }
            if let Some(entity) = self.view_entity::<Popover>(id) {
                if self.read(entity, |popover| popover.open)? {
                    self.toggle_popover(entity)?;
                }
                return Ok(());
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        Ok(())
    }

    pub fn activate_segmented_option(
        &mut self,
        entity: Entity<SegmentedOption>,
    ) -> Result<bool, FrameworkError> {
        let Some(parent) = self.world.node(entity.id).and_then(|node| node.parent) else {
            return Ok(false);
        };
        if self
            .views
            .get(&parent)
            .is_some_and(|view| view.is::<SegmentedControl>())
        {
            return self.request_segmented_selection(Entity::from_stable_id(parent), entity);
        }
        if self
            .views
            .get(&parent)
            .is_some_and(|view| view.is::<Tabs>())
        {
            return self.activate_tabs_option(Entity::from_stable_id(parent), entity.id);
        }
        Ok(false)
    }

    /// Activate a retained component selected by hit testing without exposing
    /// its concrete Rust type to a platform adapter.
    pub fn activate_node(&mut self, id: StableNodeId) -> Result<bool, FrameworkError> {
        if let Some((root, close_action, busy, _)) = self.modal_action_context(id) {
            if busy {
                return Ok(false);
            }
            if close_action == Some(id) {
                let Some(host) = self.world.node(root).and_then(|node| node.parent) else {
                    return Ok(false);
                };
                return self.request_dialog_close(
                    Entity::from_stable_id(host),
                    nana_ui_core::DialogCloseTrigger::CloseButton,
                );
            }
        }
        let handler = self
            .views
            .get(&id)
            .and_then(|view| self.activations.get(&view.as_ref().type_id()).cloned());
        match handler {
            Some(handler) => handler(self, id),
            None => Ok(false),
        }
    }

    /// Reconcile the complete ordered option set and its controlled selection
    /// in one retained transaction. Removed options are parked, preserving
    /// their typed state and application-owned event handlers.
    pub fn set_segmented_options(
        &mut self,
        control: Entity<SegmentedControl>,
        options: Vec<Entity<SegmentedOption>>,
        selected: Option<Entity<SegmentedOption>>,
    ) -> Result<bool, FrameworkError> {
        self.set_segmented_options_inner(control, options, selected)
    }

    fn set_segmented_options_inner(
        &mut self,
        control: Entity<SegmentedControl>,
        options: Vec<Entity<SegmentedOption>>,
        selected: Option<Entity<SegmentedOption>>,
    ) -> Result<bool, FrameworkError> {
        self.read(control, |_| ())?;
        let option_ids = options.iter().map(|option| option.id).collect::<Vec<_>>();
        let unique = option_ids.iter().copied().collect::<HashSet<_>>();
        if unique.len() != option_ids.len()
            || selected.is_some_and(|selected| !unique.contains(&selected.id))
        {
            return Err(FrameworkError::InvalidComponentValue(control.id));
        }
        let control_node = self
            .world
            .node(control.id)
            .ok_or(FrameworkError::MissingView(control.id))?;
        if control_node.children.iter().any(|child| {
            !self
                .views
                .get(child)
                .is_some_and(|view| view.is::<SegmentedOption>())
        }) {
            return Err(FrameworkError::InvalidComponentHierarchy {
                parent: control.id,
                child: *control_node
                    .children
                    .iter()
                    .find(|child| {
                        !self
                            .views
                            .get(child)
                            .is_some_and(|view| view.is::<SegmentedOption>())
                    })
                    .unwrap(),
            });
        }
        for option in &options {
            self.read(*option, |_| ())?;
            let node = self
                .world
                .node(option.id)
                .ok_or(FrameworkError::MissingView(option.id))?;
            if node.document != control_node.document
                || node.parent.is_some_and(|parent| parent != control.id)
                || (node.parent.is_none()
                    && self.world.mount_state(option.id) != Some(crate::MountState::Parked))
            {
                return Err(FrameworkError::InvalidComponentHierarchy {
                    parent: control.id,
                    child: option.id,
                });
            }
        }
        let selected_id = selected.map(Entity::stable_id);
        let (size, chrome, fill) = self.read(control, |control| {
            (control.size, control.chrome, control.fill)
        })?;
        let current = self.read(control, |control| {
            (
                control.options.clone(),
                control.selected,
                control.focus_target,
            )
        })?;
        let mut enabled = Vec::new();
        for entity in &options {
            if !self.read(*entity, |option| option.disabled)? {
                enabled.push(entity.id);
            }
        }
        let focus_target = self
            .world
            .focused(control_node.document)
            .filter(|id| unique.contains(id) && enabled.contains(id))
            .or_else(|| {
                selected_id
                    .filter(|id| enabled.contains(id))
                    .or_else(|| enabled.first().copied())
            });
        let mut surface_stale = false;
        for entity in &options {
            if !self.read(*entity, |option| {
                option.size == size
                    && option.chrome == chrome
                    && option.fill == fill
                    && option.selected == (Some(entity.id) == selected_id)
            })? {
                surface_stale = true;
                break;
            }
        }
        if current == (option_ids.clone(), selected_id, focus_target)
            && control_node.children == option_ids
            && !surface_stale
        {
            return Ok(false);
        }

        let mut mutations = MutationQueue::new();
        let removed = control_node
            .children
            .iter()
            .copied()
            .filter(|id| !unique.contains(id))
            .collect::<Vec<_>>();
        for id in &removed {
            mutations.park_subtree(*id);
        }
        for id in &option_ids {
            mutations.insert(control.id, *id, None);
        }
        let mut staged_options = Vec::new();
        for option in options {
            let mut next = self.read(option, Clone::clone)?;
            next.selected = Some(option.id) == selected_id;
            next.synchronize_surface(size, chrome, fill);
            next.project(option.id, &self.world, &mut mutations);
            staged_options.push((option.id, next));
        }
        let mut next_control = self.read(control, Clone::clone)?;
        next_control.options = option_ids;
        next_control.selected = selected_id;
        next_control.focus_target = focus_target;
        next_control.project(control.id, &self.world, &mut mutations);
        self.commit_mutations(mutations)?;
        for (id, option) in staged_options {
            self.views.insert(id, Box::new(option));
        }
        self.views.insert(control.id, Box::new(next_control));
        Ok(true)
    }

    /// Publish controlled selection without replacing the option identities.
    pub fn set_segmented_selection(
        &mut self,
        control: Entity<SegmentedControl>,
        selected: Option<Entity<SegmentedOption>>,
    ) -> Result<bool, FrameworkError> {
        let ids = self.read(control, |control| control.options.clone())?;
        let options = ids.into_iter().map(Entity::from_stable_id).collect();
        self.set_segmented_options(control, options, selected)
    }

    /// Update the control density and every retained option in one commit.
    pub fn set_segmented_size(
        &mut self,
        control: Entity<SegmentedControl>,
        size: nana_ui_core::ControlSize,
    ) -> Result<bool, FrameworkError> {
        let mut next_control = self.read(control, Clone::clone)?;
        if next_control.size == size {
            return Ok(false);
        }
        next_control.apply_size(size);
        let mut mutations = MutationQueue::new();
        let mut staged_options = Vec::new();
        for id in &next_control.options {
            let entity = Entity::<SegmentedOption>::from_stable_id(*id);
            let mut option = self.read(entity, Clone::clone)?;
            option.synchronize_surface(size, next_control.chrome, next_control.fill);
            option.project(*id, &self.world, &mut mutations);
            staged_options.push((*id, option));
        }
        next_control.project(control.id, &self.world, &mut mutations);
        self.commit_mutations(mutations)?;
        for (id, option) in staged_options {
            self.views.insert(id, Box::new(option));
        }
        self.views.insert(control.id, Box::new(next_control));
        Ok(true)
    }

    /// Change one option's availability while preserving controlled checked
    /// state and atomically repairing the group's sequential tab stop.
    pub fn set_segmented_option_disabled(
        &mut self,
        control: Entity<SegmentedControl>,
        option: Entity<SegmentedOption>,
        disabled: bool,
    ) -> Result<bool, FrameworkError> {
        let control_node = self
            .world
            .node(control.id)
            .ok_or(FrameworkError::MissingView(control.id))?;
        if !control_node.children.contains(&option.id) {
            return Err(FrameworkError::InvalidComponentHierarchy {
                parent: control.id,
                child: option.id,
            });
        }
        let mut next_option = self.read(option, Clone::clone)?;
        if next_option.disabled == disabled {
            return Ok(false);
        }
        next_option.disabled = disabled;
        let mut next_control = self.read(control, Clone::clone)?;
        let mut enabled = Vec::new();
        for id in &next_control.options {
            let is_enabled = if *id == option.id {
                !disabled
            } else {
                !self.read(Entity::<SegmentedOption>::from_stable_id(*id), |option| {
                    option.disabled
                })?
            };
            if is_enabled {
                enabled.push(*id);
            }
        }
        next_control.focus_target = self
            .world
            .focused(control_node.document)
            .filter(|focused| next_control.options.contains(focused) && enabled.contains(focused))
            .or_else(|| {
                next_control
                    .selected
                    .filter(|selected| enabled.contains(selected))
                    .or_else(|| enabled.first().copied())
            });
        let document = control_node.document;
        let repair_focus = disabled && self.world.focused(document) == Some(option.id);
        let mut mutations = MutationQueue::new();
        next_option.project(option.id, &self.world, &mut mutations);
        next_control.project(control.id, &self.world, &mut mutations);
        if repair_focus {
            mutations.request_focus(document, next_control.focus_target);
        }
        self.commit_mutations(mutations)?;
        self.views.insert(option.id, Box::new(next_option));
        self.views.insert(control.id, Box::new(next_control));
        Ok(true)
    }

    pub fn request_segmented_selection(
        &mut self,
        control: Entity<SegmentedControl>,
        requested: Entity<SegmentedOption>,
    ) -> Result<bool, FrameworkError> {
        let is_child = self
            .world
            .node(control.id)
            .map(|node| node.children.contains(&requested.id))
            .ok_or(FrameworkError::MissingView(control.id))?;
        if !is_child {
            return Err(FrameworkError::InvalidComponentHierarchy {
                parent: control.id,
                child: requested.id,
            });
        }
        if self.read(requested, |option| option.disabled)? {
            return Ok(false);
        }
        let document = self.world.node(control.id).unwrap().document;
        self.update_component(control, |control, cx| {
            control.focus_target = Some(requested.id);
            cx.mutations().request_focus(document, Some(requested.id));
            cx.emit(SegmentedSelectionRequested {
                option: requested.id,
            });
            true
        })
    }

    /// Handle horizontal roving focus before range/table/text key routing.
    pub fn navigate_focused_segmented(
        &mut self,
        document: DocumentId,
        intent: RovingFocusIntent,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.world.focused(document) else {
            return Ok(false);
        };
        let Some(parent) = self.world.node(focused).and_then(|node| node.parent) else {
            return Ok(false);
        };
        if self
            .views
            .get(&parent)
            .is_some_and(|view| view.is::<Tabs>())
        {
            return self.navigate_tabs(Entity::from_stable_id(parent), intent);
        }
        if !self
            .views
            .get(&parent)
            .is_some_and(|view| view.is::<SegmentedControl>())
        {
            return Ok(false);
        }
        let control = Entity::<SegmentedControl>::from_stable_id(parent);
        let (ids, policy) = self.read(control, |control| {
            (control.options.clone(), control.roving_focus)
        })?;
        let items = ids
            .iter()
            .map(|id| {
                (
                    *id,
                    self.views
                        .get(id)
                        .and_then(|view| view.downcast_ref::<SegmentedOption>())
                        .is_some_and(|option| !option.disabled),
                )
            })
            .collect::<Vec<_>>();
        let Some(target) = policy.resolve(&items, Some(focused), intent) else {
            return Ok(false);
        };
        self.request_segmented_selection(control, Entity::from_stable_id(target))
    }

    pub(super) fn is_roving_tab_stop(&self, id: StableNodeId) -> bool {
        if !self
            .views
            .get(&id)
            .is_some_and(|view| view.is::<SegmentedOption>())
        {
            return true;
        }
        let Some(parent) = self.world.node(id).and_then(|node| node.parent) else {
            return false;
        };
        if let Some(control) = self
            .views
            .get(&parent)
            .and_then(|view| view.downcast_ref::<SegmentedControl>())
        {
            return control.focus_target == Some(id);
        }
        self.views
            .get(&parent)
            .and_then(|view| view.downcast_ref::<Tabs>())
            .is_some_and(|tabs| tabs.roving_target() == Some(id))
    }

    pub fn is_segmented_option_node(&self, id: StableNodeId) -> bool {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<SegmentedOption>())
    }

    pub(super) fn sequential_focus_candidate(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> bool {
        self.world.is_mounted(id)
            && self
                .world
                .node(id)
                .is_some_and(|node| node.document == document)
            && self
                .world
                .interaction(id)
                .is_some_and(|interaction| interaction.focusable)
            && self.is_roving_tab_stop(id)
            && !self.confirm_busy_action_subtree(id)
    }

    pub(super) fn sequential_focus_candidates(&self, document: DocumentId) -> Vec<StableNodeId> {
        self.world
            .document_order(document)
            .into_iter()
            .filter(|id| {
                self.sequential_focus_candidate(document, *id)
                    && self.world.is_overlay_reachable(*id)
            })
            .collect()
    }

    /// Move through the backend-neutral sequential focus order. Roving groups
    /// contribute exactly their current tab stop while retaining programmatic
    /// focusability for every enabled option.
    pub fn navigate_sequential_focus(
        &mut self,
        document: DocumentId,
        reverse: bool,
    ) -> Result<bool, FrameworkError> {
        let candidates = self.sequential_focus_candidates(document);
        if candidates.is_empty() {
            return Ok(false);
        }
        let next = self
            .world
            .focused(document)
            .and_then(|current| candidates.iter().position(|id| *id == current))
            .map(|index| {
                if reverse {
                    (index + candidates.len() - 1) % candidates.len()
                } else {
                    (index + 1) % candidates.len()
                }
            })
            .unwrap_or_else(|| if reverse { candidates.len() - 1 } else { 0 });
        if self.world.focused(document) != Some(candidates[next]) {
            self.focus_node(document, candidates[next])?;
        }
        Ok(true)
    }

    pub fn is_range_field(&self, id: StableNodeId) -> bool {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<RangeField>())
    }

    pub fn is_xy_pad(&self, id: StableNodeId) -> bool {
        self.views.get(&id).is_some_and(|view| view.is::<XYPad>())
    }

    pub fn pointer_target(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        self.world.hit_test(document, x, y)
    }

    pub fn set_pointer_hover(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: Option<StableNodeId>,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        self.set_pointer_hover_at(document, pointer_id, target, self.component_lifecycle.now)
    }

    pub fn set_pointer_location(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        position: Option<(f32, f32)>,
    ) {
        let key = (document, pointer_id);
        if let Some(position) = position {
            self.component_lifecycle
                .pointer_positions
                .insert(key, position);
        } else {
            self.component_lifecycle.pointer_positions.remove(&key);
        }
    }

    pub fn set_pointer_hover_at(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: Option<StableNodeId>,
        now: Duration,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        self.component_lifecycle.now = now;
        let previous = self.world.set_pointer_hover(document, pointer_id, target)?;
        if previous != target {
            if let Some(previous) = previous {
                self.leave_tooltip(previous)?;
            }
            if let Some(target) = target {
                self.enter_tooltip(target, now)?;
            }
            let previous_section = previous
                .and_then(|id| self.enclosing_sidebar_section(id))
                .map(Entity::stable_id);
            let next_section = target
                .and_then(|id| self.enclosing_sidebar_section(id))
                .map(Entity::stable_id);
            if previous_section != next_section {
                self.sync_sidebar_section_hover(previous, target)?;
            }
            self.sync_sidebar_section_hover(target, target)?;
            self.sync_scroll_view_hover(previous, target)?;
            let previous_row = previous.and_then(|id| self.enclosing_sidebar_row(id));
            let next_row = target.and_then(|id| self.enclosing_sidebar_row(id));
            if previous_row.map(Entity::stable_id) != next_row.map(Entity::stable_id) {
                self.sync_sidebar_row_hover(previous_row, false)?;
            }
            self.sync_sidebar_row_hover(next_row, true)?;
        } else if let Some(target) = target {
            self.reposition_follow_cursor_tooltip(target)?;
        }
        Ok(previous)
    }

    pub fn icon_button_tooltip(
        &self,
        entity: Entity<IconButton>,
    ) -> Result<Option<Entity<Tooltip>>, FrameworkError> {
        self.read(entity, |_| ())?;
        Ok(self
            .component_lifecycle
            .tooltips
            .get(&entity.id)
            .map(|tooltip| Entity::from_stable_id(tooltip.overlay)))
    }

    /// Atomically attach and order a ListItem's typed direct-child slots.
    /// Every existing child must be named exactly once; arbitrary nested or
    /// duplicate slot identities are rejected before retained state changes.
    pub fn set_list_item_slots(
        &mut self,
        item: Entity<ListItem>,
        slots: ListItemSlots,
    ) -> Result<bool, FrameworkError> {
        self.read(item, |_| ())?;
        let ordered = [slots.leading, slots.content, slots.trailing]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let unique = ordered.iter().copied().collect::<HashSet<_>>();
        if unique.len() != ordered.len() {
            return Err(FrameworkError::InvalidListItemSlots {
                item: item.id,
                slot: None,
            });
        }
        let item_node = self
            .world
            .node(item.id)
            .ok_or(FrameworkError::MissingView(item.id))?;
        if item_node
            .children
            .iter()
            .any(|child| !unique.contains(child))
        {
            return Err(FrameworkError::InvalidListItemSlots {
                item: item.id,
                slot: item_node
                    .children
                    .iter()
                    .find(|child| !unique.contains(child))
                    .copied(),
            });
        }
        for &slot in &ordered {
            let Some(node) = self.world.node(slot) else {
                return Err(FrameworkError::InvalidListItemSlots {
                    item: item.id,
                    slot: Some(slot),
                });
            };
            if node.document != item_node.document
                || node.parent.is_some_and(|parent| parent != item.id)
            {
                return Err(FrameworkError::InvalidListItemSlots {
                    item: item.id,
                    slot: Some(slot),
                });
            }
        }
        let changed =
            item_node.children != ordered || self.read(item, |item| item.slots != slots)?;
        if !changed {
            return Ok(false);
        }
        let item_id = item.id;
        self.update_component(item, |item, cx| {
            item.slots = slots;
            for slot in &ordered {
                cx.mutations().insert(item_id, *slot, None);
            }
        })?;
        Ok(true)
    }

    /// Atomically validate and attach an EmptyState's application-owned action.
    /// Intrinsic icon and message content remain fields of EmptyState.
    pub fn set_empty_state_action(
        &mut self,
        empty: Entity<EmptyState>,
        action: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(empty, |empty| empty.action)?;
        let owned_current = self.validate_feedback_action(empty.id, current, action)?;
        let ordered = action.into_iter().collect::<Vec<_>>();
        let changed = current != action
            || self
                .world
                .node(empty.id)
                .is_some_and(|node| node.children != ordered);
        if !changed {
            return Ok(false);
        }
        let parent = empty.id;
        self.update_component(empty, |empty, cx| {
            empty.action = action;
            if let Some(current) = owned_current
                && Some(current) != action
            {
                cx.mutations().park_subtree(current);
            }
            if let Some(action) = action {
                cx.mutations().insert(parent, action, None);
            }
        })?;
        Ok(true)
    }

    /// Atomically validate and attach a FormField's application-owned control.
    pub fn set_form_field_control(
        &mut self,
        field: Entity<FormField>,
        control: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(field, |field| field.control)?;
        let owned_current = self.validate_feedback_action(field.id, current, control)?;
        let ordered = control.into_iter().collect::<Vec<_>>();
        let changed = current != control
            || self
                .world
                .node(field.id)
                .is_some_and(|node| node.children != ordered);
        if !changed {
            return Ok(false);
        }
        let parent = field.id;
        self.update_component(field, |field, cx| {
            field.control = control;
            if let Some(current) = owned_current
                && Some(current) != control
            {
                cx.mutations().park_subtree(current);
            }
            if let Some(control) = control {
                cx.mutations().insert(parent, control, None);
            }
        })?;
        Ok(true)
    }

    /// Atomically replaces a modal's application-owned direct children.
    /// Removed children remain alive and parked so their view state and handlers
    /// can be remounted by identity.
    pub fn set_modal_slots<C: ModalSurface>(
        &mut self,
        modal: Entity<C>,
        slots: ModalSlots,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(modal, |modal| modal.slots().clone())?;
        let current_order = current.ordered();
        let ordered = slots.ordered();
        let unique = ordered.iter().copied().collect::<HashSet<_>>();
        if unique.len() != ordered.len() {
            return Err(FrameworkError::InvalidModalSlots {
                parent: modal.id,
                slot: None,
            });
        }
        let parent_node = self
            .world
            .node(modal.id)
            .ok_or(FrameworkError::MissingView(modal.id))?;
        // The view field is public API, but never trusted as ownership proof.
        // A builder-declared first mount is the only field/tree mismatch allowed.
        if parent_node.children != current_order
            && !(parent_node.children.is_empty() && current_order == ordered)
        {
            return Err(FrameworkError::InvalidModalSlots {
                parent: modal.id,
                slot: parent_node
                    .children
                    .first()
                    .copied()
                    .or(current_order.first().copied()),
            });
        }
        for slot in &ordered {
            let Some(node) = self.world.node(*slot) else {
                return Err(FrameworkError::InvalidModalSlots {
                    parent: modal.id,
                    slot: Some(*slot),
                });
            };
            if *slot == modal.id
                || node.document != parent_node.document
                || node.parent.is_some_and(|owner| owner != modal.id)
                || (node.parent.is_none()
                    && self.world.mount_state(*slot) != Some(crate::MountState::Parked))
            {
                return Err(FrameworkError::InvalidModalSlots {
                    parent: modal.id,
                    slot: Some(*slot),
                });
            }
        }
        if current == slots && parent_node.children == ordered {
            return Ok(false);
        }
        let removed = parent_node
            .children
            .iter()
            .copied()
            .filter(|child| !unique.contains(child))
            .collect::<Vec<_>>();
        let parent = modal.id;
        self.update_component(modal, |modal, cx| {
            *modal.slots_mut() = slots;
            for child in removed {
                cx.mutations().park_subtree(child);
            }
            for child in ordered {
                cx.mutations().insert(parent, child, None);
            }
        })?;
        Ok(true)
    }

    pub fn set_confirm_slots(
        &mut self,
        confirm: Entity<crate::ConfirmDialog>,
        slots: crate::ConfirmSlots,
    ) -> Result<bool, FrameworkError> {
        let modal_slots = slots.modal_slots();
        let current = self.read(confirm, |confirm| confirm.slots().clone())?;
        let ordered = modal_slots.ordered();
        let unique = ordered.iter().copied().collect::<HashSet<_>>();
        let parent_node = self
            .world
            .node(confirm.id)
            .ok_or(FrameworkError::MissingView(confirm.id))?;
        if unique.len() != ordered.len()
            || (parent_node.children != current.ordered()
                && !(parent_node.children.is_empty() && current.ordered() == ordered))
        {
            return Err(FrameworkError::InvalidModalSlots {
                parent: confirm.id,
                slot: None,
            });
        }
        for slot in &ordered {
            let node = self
                .world
                .node(*slot)
                .ok_or(FrameworkError::InvalidModalSlots {
                    parent: confirm.id,
                    slot: Some(*slot),
                })?;
            if node.document != parent_node.document
                || node.parent.is_some_and(|owner| owner != confirm.id)
                || (node.parent.is_none()
                    && self.world.mount_state(*slot) != Some(crate::MountState::Parked))
            {
                return Err(FrameworkError::InvalidModalSlots {
                    parent: confirm.id,
                    slot: Some(*slot),
                });
            }
        }
        let typed_changed =
            self.read(confirm, |confirm| confirm.confirm_slots() != Some(&slots))?;
        if current == modal_slots && parent_node.children == ordered && !typed_changed {
            return Ok(false);
        }
        let removed = parent_node
            .children
            .iter()
            .copied()
            .filter(|child| !unique.contains(child))
            .collect::<Vec<_>>();
        let parent = confirm.id;
        self.update_component(confirm, |confirm, cx| {
            *confirm.slots_mut() = modal_slots;
            confirm.set_confirm_slots_state(slots);
            for child in removed {
                cx.mutations().park_subtree(child);
            }
            for child in ordered {
                cx.mutations().insert(parent, child, None);
            }
        })?;
        Ok(true)
    }

    pub fn set_confirm_state(
        &mut self,
        confirm: Entity<crate::ConfirmDialog>,
        busy: bool,
        danger: bool,
    ) -> Result<bool, FrameworkError> {
        if self.read(confirm, |confirm| {
            confirm.busy == busy && confirm.danger == danger
        })? {
            return Ok(false);
        }
        let node = self
            .world
            .node(confirm.id)
            .ok_or(FrameworkError::MissingView(confirm.id))?;
        let document = node.document;
        let action_roots = self.read(confirm, |confirm| {
            confirm
                .slots()
                .close_action
                .into_iter()
                .chain(confirm.slots().actions.iter().copied())
                .collect::<HashSet<_>>()
        })?;
        let release_focus = busy
            && self.world.focused(document).is_some_and(|id| {
                action_roots
                    .iter()
                    .any(|root| self.overlay_descendant(*root, id))
            });
        let root = confirm.id;
        self.update_component(confirm, |confirm, cx| {
            confirm.busy = busy;
            confirm.danger = danger;
            if release_focus {
                cx.mutations().request_focus(document, Some(root));
            }
        })?;
        Ok(true)
    }

    /// Atomically validate and attach a LabeledValue's optional action child.
    /// The child retains its own activation handler; the summary never becomes
    /// an implicit action target.
    pub fn set_labeled_value_action(
        &mut self,
        summary: Entity<LabeledValue>,
        action: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        let current = self.read(summary, |summary| summary.action)?;
        let owned_current = self.validate_feedback_action(summary.id, current, action)?;
        let ordered = action.into_iter().collect::<Vec<_>>();
        let changed = current != action
            || self
                .world
                .node(summary.id)
                .is_some_and(|node| node.children != ordered);
        if !changed {
            return Ok(false);
        }
        let parent = summary.id;
        self.update_component(summary, |summary, cx| {
            summary.action = action;
            if let Some(current) = owned_current
                && Some(current) != action
            {
                cx.mutations().park_subtree(current);
            }
            if let Some(action) = action {
                cx.mutations().insert(parent, action, None);
            }
        })?;
        Ok(true)
    }

    fn validate_feedback_action(
        &self,
        parent: StableNodeId,
        current: Option<StableNodeId>,
        action: Option<StableNodeId>,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        let parent_node = self
            .world
            .node(parent)
            .ok_or(FrameworkError::MissingView(parent))?;
        if parent_node.children.len() > 1 {
            return Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: parent_node.children.get(1).copied(),
            });
        }
        let owned_current = parent_node.children.first().copied();
        match (current, owned_current) {
            (None, None) => {}
            (Some(declared), Some(owned)) if declared == owned => {}
            // A builder may declare one detached action before its first
            // explicit mount. Only the same requested identity may complete
            // that declaration; it is never treated as an owned child to park.
            (Some(declared), None) if action == Some(declared) => {}
            (declared, owned) => {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: declared.or(owned),
                });
            }
        }
        if let Some(owned) = owned_current {
            let node = self
                .world
                .node(owned)
                .ok_or(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(owned),
                })?;
            if node.document != parent_node.document || node.parent != Some(parent) {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(owned),
                });
            }
        }
        if let Some(slot) = action {
            let Some(node) = self.world.node(slot) else {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(slot),
                });
            };
            if slot == parent
                || node.document != parent_node.document
                || node.parent.is_some_and(|owner| owner != parent)
            {
                return Err(FrameworkError::InvalidFeedbackSlots {
                    parent,
                    slot: Some(slot),
                });
            }
        }
        Ok(owned_current)
    }

    fn sync_component_lifecycle(&mut self, id: StableNodeId) -> Result<(), FrameworkError> {
        if self.views.get(&id).is_some_and(|view| view.is::<Tabs>()) {
            self.sync_tabs_options(Entity::from_stable_id(id))?;
        }
        self.sync_sidebar_section_body_port(id);
        let tooltip = self
            .views
            .get(&id)
            .and_then(|view| view.downcast_ref::<IconButton>())
            .and_then(|button| (!button.disabled).then(|| button.tooltip.clone()).flatten());
        match (tooltip, self.component_lifecycle.tooltips.get(&id).copied()) {
            (Some(configured), None) => {
                let document = self
                    .world
                    .node(id)
                    .ok_or(FrameworkError::MissingView(id))?
                    .document;
                let overlay = self.allocate_id();
                let tooltip =
                    Tooltip::with_config(Arc::clone(&configured.label), configured.config);
                let mut mutations = MutationQueue::new();
                mutations.create(overlay, document, tooltip.node_kind());
                tooltip.project(overlay, &self.world, &mut mutations);
                mutations.insert(id, overlay, None);
                mutations.set_overlay_host(id, crate::OverlayHostState::default());
                self.world.commit(mutations)?;
                self.views.insert(overlay, Box::new(tooltip));
                self.component_lifecycle.tooltips.insert(
                    id,
                    TooltipLifecycle {
                        overlay,
                        show_at: None,
                        open: false,
                    },
                );
            }
            (Some(configured), Some(existing)) => {
                self.update_component(
                    Entity::<Tooltip>::from_stable_id(existing.overlay),
                    |tooltip, _| {
                        if tooltip.label != configured.label || tooltip.config != configured.config
                        {
                            *tooltip = Tooltip::with_config(
                                Arc::clone(&configured.label),
                                configured.config,
                            );
                        }
                    },
                )?;
            }
            (None, Some(existing)) => {
                let mut mutations = MutationQueue::new();
                mutations.set_overlay_host(id, crate::OverlayHostState::default());
                mutations.despawn_subtree(existing.overlay);
                self.world.commit(mutations)?;
                self.views.remove(&existing.overlay);
                self.component_lifecycle.tooltips.remove(&id);
            }
            (None, None) => {}
        }

        let desired_loading = self.views.get(&id).and_then(|view| {
            if view
                .downcast_ref::<Button>()
                .is_some_and(|button| button.loading)
            {
                Some(LoadingComponent::Button)
            } else if view
                .downcast_ref::<Switch>()
                .is_some_and(|switch| switch.loading)
            {
                Some(LoadingComponent::Switch)
            } else if view
                .downcast_ref::<crate::Card>()
                .is_some_and(|card| card.loading)
            {
                Some(LoadingComponent::Card)
            } else {
                None
            }
        });
        match desired_loading {
            Some(kind) => {
                self.component_lifecycle.loading.insert(id, kind);
                if self.world.is_mounted(id)
                    && self.component_lifecycle.next_loading_frame.is_none()
                {
                    self.component_lifecycle.next_loading_frame =
                        Some(self.component_lifecycle.now);
                }
            }
            None => {
                self.component_lifecycle.loading.remove(&id);
                if !self
                    .component_lifecycle
                    .loading
                    .keys()
                    .any(|target| self.world.is_mounted(*target))
                {
                    self.component_lifecycle.next_loading_frame = None;
                }
            }
        }
        Ok(())
    }

    fn sync_sidebar_section_body_port(&mut self, id: StableNodeId) {
        let Some(body) = self
            .views
            .get(&id)
            .and_then(|view| view.downcast_ref::<SidebarSection>())
            .and_then(|section| section.body)
        else {
            return;
        };
        let Some(style) = self.world.node_style(body).cloned() else {
            return;
        };
        let Some(list) = self
            .views
            .get_mut(&body)
            .and_then(|view| view.downcast_mut::<List>())
        else {
            return;
        };
        if list.style != style {
            list.style = style;
        }
    }

    fn retained_subtree(&self, root: StableNodeId) -> Vec<StableNodeId> {
        let mut subtree = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.world.node(id) else {
                continue;
            };
            stack.extend(node.children.iter().rev().copied());
            subtree.push(id);
        }
        subtree
    }

    fn suspend_component_lifecycle(&mut self, id: StableNodeId) {
        if let Some(button) = self
            .views
            .get_mut(&id)
            .and_then(|view| view.downcast_mut::<IconButton>())
        {
            button.tooltip_open = false;
        }
        if let Some(tooltip) = self.component_lifecycle.tooltips.get_mut(&id) {
            tooltip.show_at = None;
            tooltip.open = false;
        }
        if !self
            .component_lifecycle
            .loading
            .keys()
            .any(|target| self.world.is_mounted(*target))
        {
            self.component_lifecycle.next_loading_frame = None;
        }
    }

    fn resume_component_lifecycle(&mut self, id: StableNodeId) {
        if self.world.is_mounted(id)
            && self.component_lifecycle.loading.contains_key(&id)
            && self.component_lifecycle.next_loading_frame.is_none()
        {
            self.component_lifecycle.next_loading_frame = Some(self.component_lifecycle.now);
        }
    }

    fn enter_tooltip(&mut self, target: StableNodeId, now: Duration) -> Result<(), FrameworkError> {
        let Some(button) = self
            .views
            .get(&target)
            .and_then(|view| view.downcast_ref::<IconButton>())
        else {
            return Ok(());
        };
        let Some(configured) = button.tooltip.as_ref() else {
            return Ok(());
        };
        let Some(lifecycle) = self.component_lifecycle.tooltips.get_mut(&target) else {
            return Ok(());
        };
        lifecycle.show_at = now.checked_add(Duration::from_millis(configured.config.delay_ms));
        if lifecycle.show_at == Some(now) {
            self.open_tooltip(target)?;
        }
        Ok(())
    }

    fn leave_tooltip(&mut self, target: StableNodeId) -> Result<(), FrameworkError> {
        let Some(lifecycle) = self.component_lifecycle.tooltips.get_mut(&target) else {
            return Ok(());
        };
        lifecycle.show_at = None;
        if !lifecycle.open {
            return Ok(());
        }
        lifecycle.open = false;
        self.update_component(
            Entity::<IconButton>::from_stable_id(target),
            |button, cx| {
                button.tooltip_open = false;
                cx.mutations()
                    .set_overlay_host(target, crate::OverlayHostState::default());
            },
        )?;
        Ok(())
    }

    fn open_tooltip(&mut self, target: StableNodeId) -> Result<bool, FrameworkError> {
        let Some(lifecycle) = self.component_lifecycle.tooltips.get(&target).copied() else {
            return Ok(false);
        };
        if lifecycle.open {
            return Ok(false);
        }
        self.position_tooltip(target, lifecycle.overlay)?;
        if let Some(lifecycle) = self.component_lifecycle.tooltips.get_mut(&target) {
            lifecycle.show_at = None;
            lifecycle.open = true;
        }
        self.update_component(
            Entity::<IconButton>::from_stable_id(target),
            |button, cx| {
                button.tooltip_open = true;
                cx.mutations().set_overlay_host(
                    target,
                    crate::OverlayHostState {
                        active: Some(lifecycle.overlay),
                        restore_focus: None,
                    },
                );
            },
        )?;
        Ok(true)
    }

    fn position_open_tooltips(&mut self, document: DocumentId) -> Result<(), FrameworkError> {
        let targets = self
            .component_lifecycle
            .tooltips
            .iter()
            .filter_map(|(&target, tooltip)| {
                (tooltip.open
                    && self
                        .world
                        .node(target)
                        .is_some_and(|node| node.document == document))
                .then_some((target, tooltip.overlay))
            })
            .collect::<Vec<_>>();
        for (target, overlay) in targets {
            self.position_tooltip(target, overlay)?;
        }
        Ok(())
    }

    fn pointer_location_on(&self, target: StableNodeId) -> Option<(f32, f32)> {
        let document = self.world.node(target)?.document;
        self.component_lifecycle.pointer_positions.iter().find_map(
            |(&(owner, pointer_id), &position)| {
                (owner == document && self.world.pointer_hover(owner, pointer_id) == Some(target))
                    .then_some(position)
            },
        )
    }

    fn reposition_follow_cursor_tooltip(
        &mut self,
        target: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let Some(lifecycle) = self.component_lifecycle.tooltips.get(&target).copied() else {
            return Ok(());
        };
        if !lifecycle.open {
            return Ok(());
        }
        let follows = self
            .read(
                Entity::<Tooltip>::from_stable_id(lifecycle.overlay),
                |tooltip| tooltip.config.placement == TooltipPlacement::FollowCursor,
            )
            .unwrap_or(false);
        if follows {
            self.position_tooltip(target, lifecycle.overlay)?;
        }
        Ok(())
    }

    fn position_tooltip(
        &mut self,
        target: StableNodeId,
        overlay: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let anchor = self
            .world
            .layout_box(target)
            .ok_or(FrameworkError::MissingView(target))?;
        let document = self
            .world
            .node(target)
            .ok_or(FrameworkError::MissingView(target))?
            .document;
        let Some(viewport) = self.component_lifecycle.viewports.get(&document).copied() else {
            return Ok(());
        };
        let metrics = self.world.text_metrics(overlay).unwrap_or_default();
        let (config, mut style) = self
            .read(Entity::<Tooltip>::from_stable_id(overlay), |tooltip| {
                (tooltip.config, tooltip.style.clone())
            })?;
        let padding_x = TooltipConfig::PADDING_X;
        let padding_y = TooltipConfig::PADDING_Y;
        let desired_width = (metrics.width + padding_x * 2.0 + 2.0)
            .min(config.max_width)
            .max(0.0);
        let height = (metrics.height + padding_y * 2.0 + 2.0).max(0.0);
        let padding = config.viewport_padding.max(0.0);
        let left_available = (anchor.x - config.gap - padding).max(0.0);
        let right_available =
            (viewport.width - padding - (anchor.x + anchor.width + config.gap)).max(0.0);
        let cursor = self.pointer_location_on(target).unwrap_or((
            anchor.x + anchor.width / 2.0,
            anchor.y + anchor.height / 2.0,
        ));
        let (width, horizontal_side) = match config.placement {
            TooltipPlacement::Left => {
                let side = if left_available >= desired_width
                    || (left_available >= right_available && right_available < desired_width)
                {
                    TooltipPlacement::Left
                } else {
                    TooltipPlacement::Right
                };
                let available = if side == TooltipPlacement::Left {
                    left_available
                } else {
                    right_available
                };
                (desired_width.min(available), Some(side))
            }
            TooltipPlacement::Right => {
                let side = if right_available >= desired_width
                    || (right_available >= left_available && left_available < desired_width)
                {
                    TooltipPlacement::Right
                } else {
                    TooltipPlacement::Left
                };
                let available = if side == TooltipPlacement::Left {
                    left_available
                } else {
                    right_available
                };
                (desired_width.min(available), Some(side))
            }
            TooltipPlacement::Top | TooltipPlacement::Bottom | TooltipPlacement::FollowCursor => {
                (desired_width, None)
            }
        };
        let top = (
            anchor.x + (anchor.width - width) / 2.0,
            anchor.y - config.gap - height,
        );
        let right = (
            anchor.x + anchor.width + config.gap,
            anchor.y + (anchor.height - height) / 2.0,
        );
        let bottom = (
            anchor.x + (anchor.width - width) / 2.0,
            anchor.y + anchor.height + config.gap,
        );
        let left = (
            anchor.x - config.gap - width,
            anchor.y + (anchor.height - height) / 2.0,
        );
        let follow_above = (cursor.0, cursor.1 - height - config.gap);
        let follow_below = (cursor.0, cursor.1 + config.gap);
        let fits = |(x, y): (f32, f32)| {
            x >= padding
                && y >= padding
                && x + width <= viewport.width - padding
                && y + height <= viewport.height - padding
        };
        let preferred = match config.placement {
            TooltipPlacement::Top => top,
            TooltipPlacement::Right => right,
            TooltipPlacement::Bottom => bottom,
            TooltipPlacement::Left => left,
            TooltipPlacement::FollowCursor => follow_above,
        };
        let opposite = match config.placement {
            TooltipPlacement::Top => bottom,
            TooltipPlacement::Right => left,
            TooltipPlacement::Bottom => top,
            TooltipPlacement::Left => right,
            TooltipPlacement::FollowCursor => follow_below,
        };
        let (x, y) = if let Some(side) = horizontal_side {
            match side {
                TooltipPlacement::Left => left,
                TooltipPlacement::Right => right,
                TooltipPlacement::Top
                | TooltipPlacement::Bottom
                | TooltipPlacement::FollowCursor => unreachable!(),
            }
        } else if fits(preferred) || !fits(opposite) {
            preferred
        } else {
            opposite
        };
        let max_x = (viewport.width - padding - width).max(padding);
        let max_y = (viewport.height - padding - height).max(padding);
        let layout = Arc::make_mut(&mut style.layout);
        layout.offset_left = Some(LengthSpec::Px(x.clamp(padding, max_x)));
        layout.offset_top = Some(LengthSpec::Px(y.clamp(padding, max_y)));
        layout.width = Some(LengthSpec::Px(width));
        self.update_component(Entity::<Tooltip>::from_stable_id(overlay), |tooltip, _| {
            tooltip.style = style;
        })?;
        Ok(())
    }

    pub fn press_pointer(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        Ok(self.world.press_pointer(document, pointer_id, target)?)
    }

    pub fn release_pointer(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
    ) -> Option<StableNodeId> {
        self.world.release_pointer_press(document, pointer_id)
    }

    pub fn focus_node(
        &mut self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<bool, FrameworkError> {
        if !self
            .world
            .interaction(target)
            .is_some_and(|interaction| interaction.focusable)
            || self.confirm_busy_action_subtree(target)
        {
            return Ok(false);
        }
        // A numeric draft settles before focus leaves it, so a half-typed value
        // never survives as the visible text of an unfocused field.
        if self.world.focused(document) != Some(target) {
            self.commit_focused_number_input(document)?;
        }
        if self.is_segmented_option_node(target) {
            let Some(parent) = self.world.node(target).and_then(|node| node.parent) else {
                return Ok(false);
            };
            if self
                .views
                .get(&parent)
                .is_some_and(|view| view.is::<SegmentedControl>())
            {
                let control = Entity::<SegmentedControl>::from_stable_id(parent);
                let mut next = self.read(control, Clone::clone)?;
                let target_changed = next.focus_target != Some(target);
                let focus_changed = self.world.focused(document) != Some(target);
                if !target_changed && !focus_changed {
                    return Ok(false);
                }
                next.focus_target = Some(target);
                let mut mutations = MutationQueue::new();
                next.project(parent, &self.world, &mut mutations);
                mutations.request_focus(document, Some(target));
                self.commit_mutations(mutations)?;
                self.views.insert(parent, Box::new(next));
                return Ok(true);
            }
            if self
                .views
                .get(&parent)
                .is_some_and(|view| view.is::<Tabs>())
            {
                let tabs = Entity::<Tabs>::from_stable_id(parent);
                let Some(value) = self.tabs_option_value(tabs, target)? else {
                    return Ok(false);
                };
                let mut next = self.read(tabs, Clone::clone)?;
                let target_changed = next.focus.as_ref() != Some(&value);
                let focus_changed = self.world.focused(document) != Some(target);
                if !target_changed && !focus_changed {
                    return Ok(false);
                }
                next.focus = Some(value);
                let mut mutations = MutationQueue::new();
                next.project(parent, &self.world, &mut mutations);
                mutations.request_focus(document, Some(target));
                self.commit_mutations(mutations)?;
                self.views.insert(parent, Box::new(next));
                return Ok(true);
            }
            return Ok(false);
        }
        if self.world.focused(document) == Some(target) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.request_focus(document, Some(target));
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub fn clear_focus(&mut self, document: DocumentId) -> Result<bool, FrameworkError> {
        if self.world.focused(document).is_none() {
            return Ok(false);
        }
        self.commit_focused_number_input(document)?;
        let mut mutations = MutationQueue::new();
        mutations.request_focus(document, None);
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub fn set_ime_preedit(
        &mut self,
        document: DocumentId,
        text: String,
        selection: Option<(usize, usize)>,
    ) -> Result<bool, FrameworkError> {
        let Some((target, _)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if !self
            .world
            .accessibility(target)
            .is_some_and(|state| state.editable)
        {
            return Ok(false);
        }
        let composition = crate::ImeComposition { text, selection };
        if self.world.ime(target) == Some(&composition) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_ime(target, Some(composition));
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub fn clear_ime(&mut self, document: DocumentId) -> Result<bool, FrameworkError> {
        let Some((target, _)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if self.world.ime(target).is_none() {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_ime(target, None);
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub fn commit_ime(&mut self, document: DocumentId, text: &str) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.commit_editable_ime(entity, text);
        }
        self.commit_world_text_input_ime(document, text)
    }

    /// Delete UTF-8 bytes surrounding the focused editor's selection.
    ///
    /// Leaves IME preedit in place. Returns `Ok(false)` when no focused
    /// editable field can apply the requested span.
    pub fn delete_ime_surrounding(
        &mut self,
        document: DocumentId,
        before_bytes: usize,
        after_bytes: usize,
    ) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        self.delete_world_text_input_surrounding(document, before_bytes, after_bytes)
    }

    fn commit_world_text_input_ime(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        let Some((target, state)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        let mut next = state.clone();
        if !self
            .world
            .accessibility(target)
            .is_some_and(|state| state.editable)
        {
            return Ok(false);
        }
        if !next.replace_selection(text) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_ime(target, None);
        mutations.set_text_input(target, Some(next));
        self.world.commit(mutations)?;
        Ok(true)
    }

    fn delete_world_text_input_surrounding(
        &mut self,
        document: DocumentId,
        before_bytes: usize,
        after_bytes: usize,
    ) -> Result<bool, FrameworkError> {
        let Some((target, state)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if !self
            .world
            .accessibility(target)
            .is_some_and(|state| state.editable)
        {
            return Ok(false);
        }
        let mut next = state.clone();
        if !next.delete_surrounding(before_bytes, after_bytes) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_text_input(target, Some(next));
        self.world.commit(mutations)?;
        Ok(true)
    }

    fn delete_editable_surrounding<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        before_bytes: usize,
        after_bytes: usize,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if !editable.delete_surrounding(before_bytes, after_bytes) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    pub fn replace_focused_text(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.replace_editable_selection(entity, text);
        }
        Ok(false)
    }

    pub fn delete_focused_text_backward(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.delete_editable_backward(entity);
        }
        Ok(false)
    }

    /// Text currently selected in the focused editor, or in the focused
    /// rich-text block.
    ///
    /// An empty selection reports `None` so a host copy request never replaces
    /// the pasteboard with an empty string. The Runtime does not touch the OS
    /// pasteboard; the host writes what this returns.
    pub fn focused_selected_text(&self, document: DocumentId) -> Option<String> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.editable_selected_text(entity);
        }
        let focused = self.world.focused(document)?;
        if let Some(entity) = self.view_entity::<crate::SelectableRichText>(focused) {
            return self
                .read(entity, |text| text.copy_snapshot())
                .ok()
                .flatten()
                .map(|snapshot| snapshot.text);
        }
        if let Some(entity) = self.view_entity::<crate::NativeMarkdown>(focused) {
            return self
                .read(entity, |markdown| markdown.copy_snapshot())
                .ok()
                .flatten()
                .map(|snapshot| snapshot.text);
        }
        None
    }

    /// Remove the focused editor's selection and report what it held.
    ///
    /// Reports `None` without editing when the selection is empty or the field
    /// rejects input, so a cut on a read-only field leaves both the value and
    /// the pasteboard alone.
    pub fn cut_focused_text(
        &mut self,
        document: DocumentId,
    ) -> Result<Option<String>, FrameworkError> {
        let Some(text) = self.focused_selected_text(document) else {
            return Ok(None);
        };
        if !self.replace_focused_text(document, "")? {
            return Ok(None);
        }
        Ok(Some(text))
    }

    /// Select the whole value of the focused editor.
    ///
    /// Read-only and disabled fields still select, so their text can be copied.
    pub fn select_all_focused_text(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.select_all_editable(entity);
        }
        Ok(false)
    }

    fn editable_selected_text<C: EditableText>(&self, entity: Entity<C>) -> Option<String> {
        self.read(entity, |editable| {
            let state = editable.state();
            if !state.selection.is_valid_for(&state.value) {
                return None;
            }
            let range = state.selection.ordered();
            (!range.is_empty()).then(|| state.value[range].to_owned())
        })
        .ok()
        .flatten()
    }

    fn select_all_editable<C: EditableText>(
        &mut self,
        entity: Entity<C>,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |editable, cx| {
            let selection = TextSelection {
                anchor: 0,
                focus: editable.state().value.len(),
            };
            if editable.state().selection == selection {
                return false;
            }
            editable.state_mut().selection = selection;
            cx.emit(TextChanged {
                value: editable.state().value.clone(),
                selection,
            });
            true
        })
    }

    fn delete_editable_backward<C: EditableText>(
        &mut self,
        entity: Entity<C>,
    ) -> Result<bool, FrameworkError> {
        use unicode_segmentation::UnicodeSegmentation;

        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            {
                let state = editable.state_mut();
                if state.selection.anchor == state.selection.focus {
                    let caret = state.selection.focus;
                    let Some(previous) = state.value[..caret]
                        .grapheme_indices(true)
                        .next_back()
                        .map(|(index, _)| index)
                    else {
                        return false;
                    };
                    state.selection = TextSelection {
                        anchor: previous,
                        focus: caret,
                    };
                }
            }
            if !editable.replace_selection("") {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    fn commit_editable_ime<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            cx.mutations().set_ime(entity.stable_id(), None);
            if !editable.replace_selection(text) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    /// Apply an assistive-technology action through the same typed component
    /// mutations used by pointer and keyboard input.
    pub fn apply_accessibility_action(
        &mut self,
        document: DocumentId,
        request: AccessibilityActionRequest,
    ) -> Result<bool, FrameworkError> {
        if self.world.node(request.target).map(|node| node.document) != Some(document) {
            return Ok(false);
        }
        match request.action {
            AccessibilityAction::Click => self.activate_node(request.target),
            AccessibilityAction::Focus => self.focus_node(document, request.target),
            AccessibilityAction::SetValue(value) => {
                if let Some(entity) = self.view_entity::<TextInput>(request.target) {
                    return self.set_editable_value(entity, value);
                }
                if let Some(entity) = self.view_entity::<TextArea>(request.target) {
                    return self.set_editable_value(entity, value);
                }
                if let Some(entity) = self.view_entity::<SearchDropdown>(request.target) {
                    return self.set_editable_value(entity, value);
                }
                if let Some(entity) = self.view_entity::<CommandPalette>(request.target) {
                    return self.set_editable_value(entity, value);
                }
                if let Some(entity) = self.view_entity::<ContextMenu>(request.target) {
                    return self.set_editable_value(entity, value);
                }
                if self
                    .views
                    .get(&request.target)
                    .is_some_and(|view| view.is::<RangeField>())
                {
                    return value
                        .parse::<f64>()
                        .ok()
                        .map(|value| {
                            self.set_range_value(Entity::from_stable_id(request.target), value)
                        })
                        .transpose()
                        .map(|changed| changed.unwrap_or(false));
                }
                Ok(false)
            }
            AccessibilityAction::SetSelection(selection) => {
                if let Some(entity) = self.view_entity::<TextInput>(request.target) {
                    return self.set_editable_selection(entity, selection);
                }
                if let Some(entity) = self.view_entity::<TextArea>(request.target) {
                    return self.set_editable_selection(entity, selection);
                }
                if let Some(entity) = self.view_entity::<SearchDropdown>(request.target) {
                    return self.set_editable_selection(entity, selection);
                }
                if let Some(entity) = self.view_entity::<CommandPalette>(request.target) {
                    return self.set_editable_selection(entity, selection);
                }
                if let Some(entity) = self.view_entity::<ContextMenu>(request.target) {
                    return self.set_editable_selection(entity, selection);
                }
                Ok(false)
            }
        }
    }

    fn set_editable_value<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        value: String,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if !editable.set_value(value) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    fn set_editable_selection<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        selection: TextSelection,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if editable.state().selection == selection
                || !selection.is_valid_for(&editable.state().value)
            {
                return false;
            }
            editable.state_mut().selection = selection;
            cx.emit(TextChanged {
                value: editable.state().value.clone(),
                selection,
            });
            true
        })
    }

    fn activate_component<C: ComponentView>(
        &mut self,
        entity: Entity<C>,
        disabled: impl FnOnce(&C) -> bool,
    ) -> Result<bool, FrameworkError> {
        if let Some((root, close_action, busy, intent)) = self.modal_action_context(entity.id) {
            if busy {
                return Ok(false);
            }
            if close_action == Some(entity.id) {
                let Some(host) = self.world.node(root).and_then(|node| node.parent) else {
                    return Ok(false);
                };
                return self.request_dialog_close(
                    Entity::from_stable_id(host),
                    nana_ui_core::DialogCloseTrigger::CloseButton,
                );
            }
            if let Some(intent) = intent {
                self.update_component(
                    Entity::<crate::ConfirmDialog>::from_stable_id(root),
                    |_confirm, cx| cx.emit(intent),
                )?;
                return Ok(true);
            }
        }
        if self.read(entity, disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |_component, cx| cx.emit(Activate))?;
        Ok(true)
    }

    pub fn toggle_checkbox(&mut self, entity: Entity<Checkbox>) -> Result<bool, FrameworkError> {
        if self.read(entity, |checkbox| checkbox.disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |checkbox, cx| {
            checkbox.checked = !checkbox.checked;
            cx.emit(ToggleChanged {
                checked: checkbox.checked,
            });
        })?;
        Ok(true)
    }

    pub fn toggle_switch(&mut self, entity: Entity<Switch>) -> Result<bool, FrameworkError> {
        if self.read(entity, |switch| switch.disabled || switch.loading)? {
            return Ok(false);
        }
        self.update_component(entity, |switch, cx| {
            switch.checked = !switch.checked;
            cx.emit(ToggleChanged {
                checked: switch.checked,
            });
        })?;
        Ok(true)
    }

    /// Publish a numeric value. The field snaps and clamps it, so hosts do not
    /// have to reimplement the step grid to stay legal.
    pub fn set_number_value(
        &mut self,
        entity: Entity<NumberInput>,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.update_component(entity, |input, cx| {
            if !input.assign(value) {
                return false;
            }
            cx.emit(NumberChanged {
                value: input.value(),
            });
            true
        })
    }

    /// Move a numeric field by grid positions. Disabled and read-only fields
    /// refuse, so a stepper press cannot bypass either flag.
    pub fn step_number_input(
        &mut self,
        entity: Entity<NumberInput>,
        steps: i32,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, NumberInput::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |input, cx| {
            if !input.step_value(steps) {
                return false;
            }
            cx.emit(NumberChanged {
                value: input.value(),
            });
            true
        })
    }

    /// Parse the in-progress draft into the committed value. An unparseable
    /// draft restores the last committed value and reports no change.
    pub fn commit_number_input(
        &mut self,
        entity: Entity<NumberInput>,
    ) -> Result<bool, FrameworkError> {
        let before = self.read(entity, NumberInput::value)?;
        let touched = self.update_component(entity, |input, cx| {
            if !input.commit_draft() {
                return false;
            }
            if input.value() == before {
                return true;
            }
            cx.emit(NumberChanged {
                value: input.value(),
            });
            true
        })?;
        Ok(touched)
    }

    /// Step the focused numeric field, if any. Returns whether it moved.
    pub fn step_focused_number_input(
        &mut self,
        document: DocumentId,
        steps: i32,
    ) -> Result<bool, FrameworkError> {
        match self.focused_number_input(document) {
            Some(entity) => self.step_number_input(entity, steps),
            None => Ok(false),
        }
    }

    /// Commit the focused numeric field's draft, if any.
    pub fn commit_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        match self.focused_number_input(document) {
            Some(entity) => self.commit_number_input(entity),
            None => Ok(false),
        }
    }

    /// Discard the focused numeric field's draft and show the committed value
    /// again. Nothing is emitted: the value never moved.
    pub fn revert_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_number_input(document) else {
            return Ok(false);
        };
        self.update_component(entity, |input, _| {
            let committed = input.spec.format(input.value());
            if input.state.value == committed {
                return false;
            }
            input.state.replace_value(committed);
            true
        })
    }

    fn focused_number_input(&self, document: DocumentId) -> Option<Entity<NumberInput>> {
        let target = self.world.focused(document)?;
        self.view_entity(target)
    }

    /// Resolve a stepper press inside a numeric field to a signed step count.
    ///
    /// Coordinates are viewport-local, matching hit testing. Returns `None`
    /// when the point is on the editable text instead of the spinner, so the
    /// caller can fall through to caret placement.
    pub fn number_stepper_at(&self, id: StableNodeId, x: f32, y: f32) -> Option<i32> {
        let Some(crate::ComponentGeometry::TextInput {
            steppers: Some(steppers),
            ..
        }) = self.world.component_geometry(id)
        else {
            return None;
        };
        steppers.step_at(x, y)
    }

    /// Route a pointer press on a numeric field's spinner. Returns whether the
    /// press was consumed by a stepper.
    pub fn press_number_stepper(
        &mut self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(steps) = self.number_stepper_at(id, x, y) else {
            return Ok(false);
        };
        let Some(entity) = self.view_entity::<NumberInput>(id) else {
            return Ok(false);
        };
        self.step_number_input(entity, steps)?;
        Ok(true)
    }

    pub fn set_range_value(
        &mut self,
        entity: Entity<RangeField>,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, |range| range.disabled)? {
            return Ok(false);
        }
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.update_component(entity, |range, cx| {
            let value = range.quantize(value);
            if range.value == value {
                return false;
            }
            range.value = value;
            cx.emit(RangeChanged { value });
            true
        })
    }

    pub fn adjust_range(
        &mut self,
        entity: Entity<RangeField>,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let value = self.read(entity, |range| match adjustment {
            RangeAdjustment::Decrement => range.value - range.step,
            RangeAdjustment::Increment => range.value + range.step,
            RangeAdjustment::PageDecrement => range.value - range.page_step,
            RangeAdjustment::PageIncrement => range.value + range.page_step,
            RangeAdjustment::Minimum => range.minimum,
            RangeAdjustment::Maximum => range.maximum,
        })?;
        self.set_range_value(entity, value)
    }

    pub fn adjust_focused_range(
        &mut self,
        document: DocumentId,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.focused(document) else {
            return Ok(false);
        };
        if !self.is_range_field(target) {
            return Ok(false);
        }
        self.adjust_range(Entity::from_stable_id(target), adjustment)
    }

    pub fn begin_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
    ) -> Result<bool, FrameworkError> {
        if !self.is_range_field(target) {
            return Ok(false);
        }
        if self.read(Entity::<RangeField>::from_stable_id(target), |range| {
            range.disabled
        })? {
            return Ok(false);
        }
        let initial_value = self.read(Entity::<RangeField>::from_stable_id(target), |range| {
            range.value
        })?;
        self.update_component(Entity::<RangeField>::from_stable_id(target), |range, cx| {
            range.dragging = Some(crate::RangeDragState {
                pointer_id,
                initial_value,
            });
            cx.mutations().capture_pointer(pointer_id, target);
        })?;
        self.update_range_drag(document, pointer_id, x)
    }

    pub fn update_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_range_field(target) {
            return Ok(false);
        }
        let track = match self.world.component_geometry(target) {
            Some(crate::ComponentGeometry::Range { track, .. }) => track,
            _ => return Ok(false),
        };
        if track.width <= 0.0 {
            return Ok(false);
        }
        let value = self.read(Entity::<RangeField>::from_stable_id(target), |range| {
            range.minimum
                + f64::from(((x - track.x) / track.width).clamp(0.0, 1.0))
                    * (range.maximum - range.minimum)
        })?;
        self.set_range_value(Entity::from_stable_id(target), value)
    }

    pub fn end_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_range_field(target) {
            return Ok(false);
        }
        let initial = self.read(Entity::<RangeField>::from_stable_id(target), |range| {
            range.dragging.map(|drag| drag.initial_value)
        })?;
        let restored = if cancel {
            initial
                .map(|value| self.set_range_value(Entity::from_stable_id(target), value))
                .transpose()?
                .unwrap_or(false)
        } else {
            false
        };
        self.update_component(Entity::<RangeField>::from_stable_id(target), |range, cx| {
            range.dragging = None;
            cx.mutations().release_pointer(pointer_id, target);
        })?;
        Ok(restored || initial.is_some())
    }

    pub fn begin_xy_pad_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if !self.is_xy_pad(target) {
            return Ok(false);
        }
        if self.read(Entity::<XYPad>::from_stable_id(target), XYPad::inactive)? {
            return Ok(false);
        }
        let Some(bounds) = self.world.layout_box(target) else {
            return Ok(false);
        };
        self.update_component(Entity::<XYPad>::from_stable_id(target), |pad, cx| {
            pad.dragging = Some(XYPadDragState {
                pointer_id,
                origin_x: x - bounds.x,
                origin_y: y - bounds.y,
                axis_lock: None,
                initial: pad.value,
            });
            cx.mutations().capture_pointer(pointer_id, target);
        })?;
        self.update_xy_pad_drag(document, pointer_id, x, y, false)
    }

    pub fn update_xy_pad_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_xy_pad(target) {
            return Ok(false);
        }
        let Some(bounds) = self.world.layout_box(target) else {
            return Ok(false);
        };
        self.update_component(Entity::<XYPad>::from_stable_id(target), |pad, cx| {
            if pad.inactive() {
                return false;
            }
            if let Some(drag) = pad.dragging.as_mut() {
                if shift && drag.axis_lock.is_none() {
                    let local_x = x - bounds.x;
                    let local_y = y - bounds.y;
                    let dx = (local_x - drag.origin_x).abs() / bounds.width.max(1.0);
                    let dy = (local_y - drag.origin_y).abs() / bounds.height.max(1.0);
                    drag.axis_lock = Some(if dx >= dy {
                        crate::XYPadAxisLock::Horizontal
                    } else {
                        crate::XYPadAxisLock::Vertical
                    });
                } else if !shift {
                    drag.axis_lock = None;
                }
            } else {
                return false;
            }
            let locked = pad
                .dragging
                .and_then(|drag| drag.axis_lock.map(|axis| (axis, pad.value)));
            let value = pad.value_from_point(x, y, bounds, locked);
            pad.value = value;
            cx.emit(XYPadEvent::Input(value));
            true
        })
    }

    pub fn end_xy_pad_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_xy_pad(target) {
            return Ok(false);
        }
        let initial = self.read(Entity::<XYPad>::from_stable_id(target), |pad| {
            pad.dragging.map(|drag| drag.initial)
        })?;
        self.update_component(Entity::<XYPad>::from_stable_id(target), |pad, cx| {
            if cancel {
                if let Some(value) = initial {
                    pad.value = value;
                }
            } else if initial.is_some() {
                cx.emit(XYPadEvent::Change(pad.value));
            }
            pad.dragging = None;
            cx.mutations().release_pointer(pointer_id, target);
        })?;
        Ok(initial.is_some())
    }

    pub fn toggle_select(&mut self, entity: Entity<Select>) -> Result<bool, FrameworkError> {
        if self.read(entity, Select::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |select, _| select.toggle_open())
    }

    pub fn activate_select_at(
        &mut self,
        entity: Entity<Select>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, Select::inactive)? {
            return Ok(false);
        }
        let opened = self.read(entity, |select| select.opened)?;
        if opened {
            if let Some(crate::ComponentGeometry::Select {
                menu: Some(menu), ..
            }) = self.world.component_geometry(entity.id)
                && let Some(index) = crate::select::select_option_at(&menu, x, y)
            {
                return self.update_component(entity, |select, cx| {
                    if let Some(changed) = select.select_index(index) {
                        cx.emit(changed);
                        true
                    } else {
                        false
                    }
                });
            }
            let Some(field) = self.world.layout_box(entity.id) else {
                return Ok(false);
            };
            if field.contains(x, y) {
                return self.toggle_select(entity);
            }
            return self.update_component(entity, |select, _| {
                select.close();
                true
            });
        }
        self.toggle_select(entity)
    }

    pub fn adjust_focused_select(
        &mut self,
        document: DocumentId,
        delta: i32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Select>())
        {
            return Ok(false);
        }
        let entity = Entity::<Select>::from_stable_id(target);
        if self.read(entity, Select::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |select, _| {
            if !select.opened {
                select.toggle_open()
            } else {
                select.highlight_delta(delta)
            }
        })
    }

    pub fn adjust_focused_dropdown(
        &mut self,
        document: DocumentId,
        delta: i32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Dropdown>())
        {
            return Ok(false);
        }
        let entity = Entity::<Dropdown>::from_stable_id(target);
        if self.read(entity, Dropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if !dropdown.opened {
                if let Some(event) = dropdown.toggle_open() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            } else {
                dropdown.highlight_delta(delta)
            }
        })
    }

    pub fn commit_focused_dropdown(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Dropdown>())
        {
            return Ok(false);
        }
        self.update_component(
            Entity::<Dropdown>::from_stable_id(target),
            |dropdown, cx| {
                if let Some(event) = dropdown.commit_highlighted() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn commit_focused_select(&mut self, document: DocumentId) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Select>())
        {
            return Ok(false);
        }
        self.update_component(Entity::<Select>::from_stable_id(target), |select, cx| {
            if let Some(changed) = select.commit_highlighted() {
                cx.emit(changed);
                true
            } else {
                false
            }
        })
    }

    pub fn toggle_dropdown(&mut self, entity: Entity<Dropdown>) -> Result<bool, FrameworkError> {
        if self.read(entity, Dropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) = dropdown.toggle_open() {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn toggle_search_dropdown(
        &mut self,
        entity: Entity<SearchDropdown>,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, SearchDropdown::inactive)? {
            return Ok(false);
        }
        if self.read(entity, |dropdown| dropdown.opened)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) = dropdown.toggle_open() {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn activate_search_dropdown_at(
        &mut self,
        entity: Entity<SearchDropdown>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, SearchDropdown::inactive)? {
            return Ok(false);
        }
        let menu = match self.world.component_geometry(entity.id) {
            Some(crate::ComponentGeometry::Select { menu, .. }) => menu,
            _ => None,
        };
        let Some(field) = self.world.layout_box(entity.id) else {
            return Ok(false);
        };
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) = crate::search_dropdown::activate_search_dropdown_at(
                dropdown,
                menu.as_ref(),
                field,
                x,
                y,
            ) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn adjust_focused_search_dropdown(
        &mut self,
        document: DocumentId,
        delta: i32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<SearchDropdown>())
        {
            return Ok(false);
        }
        let entity = Entity::<SearchDropdown>::from_stable_id(target);
        if self.read(entity, SearchDropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if !dropdown.opened {
                if let Some(event) = dropdown.toggle_open() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            } else {
                dropdown.highlight_delta(delta)
            }
        })
    }

    pub fn commit_focused_search_dropdown(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<SearchDropdown>())
        {
            return Ok(false);
        }
        self.update_component(
            Entity::<SearchDropdown>::from_stable_id(target),
            |dropdown, cx| {
                if let Some(event) = dropdown.commit_highlighted() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn activate_command_palette_at(
        &mut self,
        entity: Entity<CommandPalette>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(crate::ComponentGeometry::CommandPalette {
            surface,
            input,
            rows,
            ..
        }) = self.world.component_geometry(entity.id)
        else {
            return Ok(false);
        };
        self.update_component(entity, |palette, cx| {
            if let Some(event) = crate::command_palette::activate_command_palette_at(
                palette,
                surface,
                input.bounds,
                &rows,
                x,
                y,
            ) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn navigate_focused_command_palette(
        &mut self,
        document: DocumentId,
        navigation: ActionPickerNavigation,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<CommandPalette>())
        {
            return Ok(false);
        }
        self.update_component(
            Entity::<CommandPalette>::from_stable_id(target),
            |palette, cx| {
                if let Some(event) = palette.navigate(navigation) {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn activate_dropdown_at(
        &mut self,
        entity: Entity<Dropdown>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, Dropdown::inactive)? {
            return Ok(false);
        }
        let menu = match self.world.component_geometry(entity.id) {
            Some(crate::ComponentGeometry::Select { menu, .. }) => menu,
            _ => None,
        };
        let Some(field) = self.world.layout_box(entity.id) else {
            return Ok(false);
        };
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) =
                crate::dropdown::activate_dropdown_at(dropdown, menu.as_ref(), field, x, y)
            {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn activate_tree_at(
        &mut self,
        entity: Entity<TreeView>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(crate::ComponentGeometry::TreeView { rows }) =
            self.world.component_geometry(entity.id)
        else {
            return Ok(false);
        };
        if let Some(index) = crate::tree_view::tree_disclosure_at(&rows, x, y) {
            let id = Arc::clone(&rows[index].id);
            return self.update_component(entity, |tree, cx| {
                let event = crate::TreeViewEvent::Toggle(id);
                if tree.apply_event(event.clone()) {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            });
        }
        if let Some(index) = crate::tree_view::tree_row_at(&rows, x, y) {
            if rows[index].disabled {
                return Ok(false);
            }
            let id = Arc::clone(&rows[index].id);
            return self.update_component(entity, |tree, cx| {
                let event = crate::TreeViewEvent::Select(id);
                if tree.apply_event(event.clone()) {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            });
        }
        Ok(false)
    }

    pub fn navigate_focused_tree(
        &mut self,
        document: DocumentId,
        navigation: crate::TreeNavigation,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TreeView>())
        {
            return Ok(false);
        }
        self.update_component(Entity::<TreeView>::from_stable_id(target), |tree, cx| {
            if let Some(event) = tree.navigate(navigation) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn activate_node_at(
        &mut self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.view_entity::<Select>(id) {
            return self.activate_select_at(entity, x, y);
        }
        if let Some(entity) = self.view_entity::<Dropdown>(id) {
            return self.activate_dropdown_at(entity, x, y);
        }
        if let Some(entity) = self.view_entity::<SearchDropdown>(id) {
            return self.activate_search_dropdown_at(entity, x, y);
        }
        if let Some(entity) = self.view_entity::<CommandPalette>(id) {
            return self.activate_command_palette_at(entity, x, y);
        }
        if let Some(entity) = self.view_entity::<ContextMenu>(id) {
            return self.activate_context_menu_at(entity, x, y);
        }
        if let Some(entity) = self.view_entity::<TreeView>(id) {
            return self.activate_tree_at(entity, x, y);
        }
        self.activate_node(id)
    }

    /// Route a secondary (right) press to the nearest `SecondaryPress` handler
    /// at or above the hit node.
    ///
    /// Returns the node that handled it. The framework opens no menu and picks
    /// no default items; an application with no handler gets `None`.
    pub fn secondary_press_at(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        let Some(target) = self.world.hit_test(document, x, y) else {
            return Ok(None);
        };
        let press = SecondaryPress { target, x, y };
        let mut current = Some(target);
        while let Some(id) = current {
            if self
                .event_handlers
                .contains_key(&(id, TypeId::of::<SecondaryPress>()))
                && let Some(emit) = self
                    .views
                    .get(&id)
                    .and_then(|view| self.secondary_presses.get(&view.as_ref().type_id()))
                    .cloned()
            {
                emit(self, id, press)?;
                return Ok(Some(id));
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        Ok(None)
    }

    pub fn dismiss_detached_menus(
        &mut self,
        keep: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let ids = self.views.keys().copied().collect::<Vec<_>>();
        for id in ids {
            if Some(id) == keep {
                continue;
            }
            if let Some(entity) = self.view_entity::<Select>(id) {
                self.update_component(entity, |select, _| {
                    if select.opened {
                        select.close();
                        true
                    } else {
                        false
                    }
                })?;
            } else if let Some(entity) = self.view_entity::<Dropdown>(id) {
                self.update_component(entity, |dropdown, cx| {
                    if let Some(event) = dropdown.close() {
                        cx.emit(event);
                        true
                    } else {
                        false
                    }
                })?;
            } else if let Some(entity) = self.view_entity::<SearchDropdown>(id) {
                self.update_component(entity, |dropdown, cx| {
                    if let Some(event) = dropdown.close() {
                        cx.emit(event);
                        true
                    } else {
                        false
                    }
                })?;
            }
        }
        Ok(())
    }

    /// Closes every open popover whose policy `allows` dismissal. A popover
    /// keeps its state when `inside` sits in its own subtree, so pressing one
    /// of its items still activates that item.
    fn close_open_popovers(
        &mut self,
        inside: Option<StableNodeId>,
        allows: fn(&Popover) -> bool,
    ) -> Result<bool, FrameworkError> {
        let ids = self.views.keys().copied().collect::<Vec<_>>();
        let mut dismissed = false;
        for id in ids {
            if inside.is_some_and(|node| self.world.is_descendant_or_self(node, id)) {
                continue;
            }
            if let Some(entity) = self.view_entity::<Popover>(id) {
                if self.read(entity, |popover| popover.open && allows(popover))? {
                    self.toggle_popover(entity)?;
                    dismissed = true;
                }
            } else if let Some(entity) = self.view_entity::<ActionMenu>(id)
                && self.read(entity, |menu| menu.popover.open && allows(&menu.popover))?
            {
                self.toggle_action_menu(entity)?;
                dismissed = true;
            }
        }
        Ok(dismissed)
    }

    /// Light dismiss for toggle-driven popovers, mirroring the outside-press
    /// rule the overlay host applies to dialogs and menus. `inside` is the node
    /// under the pointer. Returns whether anything closed; the caller consumes
    /// the press in that case so it cannot also drive the control underneath,
    /// nor re-open the popover through its own trigger.
    pub fn dismiss_popovers_outside(
        &mut self,
        inside: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        self.close_open_popovers(inside, |popover| popover.close_on_outside)
    }

    /// Escape closes every open popover that allows it.
    pub fn dismiss_popovers_on_escape(&mut self) -> Result<bool, FrameworkError> {
        self.close_open_popovers(None, |popover| popover.close_on_escape)
    }

    pub fn toggle_popover(&mut self, entity: Entity<Popover>) -> Result<bool, FrameworkError> {
        self.update_component(entity, |popover, cx| {
            popover.open = !popover.open;
            cx.emit(PopoverToggled { open: popover.open });
            if !popover.open {
                cx.emit(PopoverClosed);
            }
            true
        })
    }

    pub fn toggle_action_menu(
        &mut self,
        entity: Entity<ActionMenu>,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |menu, cx| {
            menu.popover.open = !menu.popover.open;
            cx.emit(PopoverToggled {
                open: menu.popover.open,
            });
            if !menu.popover.open {
                cx.emit(PopoverClosed);
            }
            true
        })
    }

    pub fn activate_context_menu_at(
        &mut self,
        entity: Entity<ContextMenu>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(geometry) = self.world.component_geometry(entity.id) else {
            return Ok(false);
        };
        let Some(index) = crate::menus::context_menu_option_at(&geometry, x, y) else {
            return Ok(false);
        };
        self.update_component(entity, |menu, cx| {
            if let Some(event) = menu.select_index(index) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn cancel_progress(&mut self, entity: Entity<Progress>) -> Result<bool, FrameworkError> {
        self.update_component(entity, |progress, cx| {
            if !progress.cancellable {
                return false;
            }
            cx.emit(ProgressCancelled);
            true
        })
    }

    pub fn dismiss_context_menu(
        &mut self,
        entity: Entity<ContextMenu>,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |menu, cx| {
            if !menu.open {
                return false;
            }
            menu.dismiss();
            cx.emit(ContextMenuEvent::Dismiss);
            true
        })
    }

    /// Select one professional tab by application-owned value.
    pub fn select_tabs_value(
        &mut self,
        entity: Entity<Tabs>,
        value: impl AsRef<str>,
    ) -> Result<bool, FrameworkError> {
        let value = value.as_ref();
        self.update_component(entity, |tabs, cx| {
            if let Some(event) = tabs.select(value) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    /// Activate the focused or selected tab on a professional strip.
    pub fn activate_tabs(&mut self, entity: Entity<Tabs>) -> Result<bool, FrameworkError> {
        let value = self.read(entity, |tabs| {
            tabs.focus.clone().or_else(|| tabs.selected.clone())
        })?;
        let Some(value) = value else {
            return Ok(false);
        };
        self.select_tabs_value(entity, value.as_ref())
    }

    fn activate_tabs_option(
        &mut self,
        entity: Entity<Tabs>,
        option: StableNodeId,
    ) -> Result<bool, FrameworkError> {
        let Some(value) = self.tabs_option_value(entity, option)? else {
            return Ok(false);
        };
        let changed = self.select_tabs_value(entity, value.as_ref())?;
        let document = self
            .world
            .node(entity.id)
            .ok_or(FrameworkError::MissingView(entity.id))?
            .document;
        let _ = self.focus_node(document, option)?;
        Ok(changed)
    }

    fn tabs_option_value(
        &self,
        entity: Entity<Tabs>,
        option: StableNodeId,
    ) -> Result<Option<Arc<str>>, FrameworkError> {
        self.read(entity, |tabs| {
            tabs.option_nodes()
                .iter()
                .find(|(_, id)| *id == option)
                .map(|(value, _)| Arc::clone(value))
        })
    }

    /// Painted option boxes after layout, when every child has a real box.
    pub fn tabs_strip_paint(
        &self,
        entity: Entity<Tabs>,
    ) -> Result<Option<nana_ui_core::TabStripPaint<Arc<str>>>, FrameworkError> {
        self.read(entity, |tabs| {
            tabs.strip_paint_from_layout(&self.world, entity.id)
        })
    }

    fn sync_tabs_options(&mut self, entity: Entity<Tabs>) -> Result<(), FrameworkError> {
        let tabs = self.read(entity, Clone::clone)?;
        let node = self
            .world
            .node(entity.id)
            .ok_or(FrameworkError::MissingView(entity.id))?;
        let document = node.document;
        let current_children = node.children.clone();

        let mut unused = HashMap::<Arc<str>, VecDeque<StableNodeId>>::new();
        for (value, id) in tabs.option_nodes() {
            unused.entry(Arc::clone(value)).or_default().push_back(*id);
        }

        let mut next_nodes = Vec::with_capacity(tabs.options.len());
        let mut created = Vec::new();
        for option in &tabs.options {
            let reusable = unused
                .get_mut(&option.value)
                .and_then(|ids| ids.pop_front())
                .filter(|id| {
                    self.world.node(*id).is_some_and(|node| {
                        node.document == document
                            && (node.parent == Some(entity.id)
                                || (node.parent.is_none()
                                    && self.world.mount_state(*id) == Some(MountState::Parked)))
                    }) && self
                        .views
                        .get(id)
                        .is_some_and(|view| view.is::<SegmentedOption>())
                });
            if let Some(id) = reusable {
                next_nodes.push((Arc::clone(&option.value), id));
            } else {
                let id = self.allocate_id();
                created.push((id, crate::tabs::tab_selection_option(option, &tabs)));
                next_nodes.push((Arc::clone(&option.value), id));
            }
        }

        let next_ids = next_nodes.iter().map(|(_, id)| *id).collect::<Vec<_>>();
        let stale = current_children
            .iter()
            .copied()
            .filter(|id| !next_ids.contains(id))
            .collect::<Vec<_>>();
        let created_ids = created.iter().map(|(id, _)| *id).collect::<HashSet<_>>();

        let mut options_dirty =
            !created.is_empty() || current_children != next_ids || !stale.is_empty();
        if !options_dirty {
            for (option, (_, id)) in tabs.options.iter().zip(next_nodes.iter()) {
                let current =
                    self.read(Entity::<SegmentedOption>::from_stable_id(*id), Clone::clone)?;
                if current != crate::tabs::tab_selection_option(option, &tabs) {
                    options_dirty = true;
                    break;
                }
            }
        }
        if !options_dirty && tabs.option_nodes() == next_nodes.as_slice() {
            return Ok(());
        }

        let stale_subtrees = stale
            .iter()
            .map(|id| self.retained_subtree(*id))
            .collect::<Vec<_>>();
        let mut mutations = MutationQueue::new();
        for id in &stale {
            mutations.despawn_subtree(*id);
        }
        for (id, option) in &created {
            mutations.create(*id, document, option.node_kind());
            option.project(*id, &self.world, &mut mutations);
        }
        for id in &next_ids {
            mutations.insert(entity.id, *id, None);
        }

        let mut staged_options = Vec::new();
        for (option, (_, id)) in tabs.options.iter().zip(next_nodes.iter()) {
            if created_ids.contains(id) {
                continue;
            }
            let mut current =
                self.read(Entity::<SegmentedOption>::from_stable_id(*id), Clone::clone)?;
            let desired = crate::tabs::tab_selection_option(option, &tabs);
            if current != desired {
                current = desired;
                current.project(*id, &self.world, &mut mutations);
                staged_options.push((*id, current));
            }
        }

        self.commit_mutations(mutations)?;
        let mut removed = HashSet::new();
        for subtree in stale_subtrees {
            for id in subtree {
                removed.insert(id);
                self.views.remove(&id);
                self.component_lifecycle.tooltips.remove(&id);
                self.component_lifecycle.loading.remove(&id);
            }
        }
        self.remove_event_handlers_for(&removed);
        for (id, option) in created {
            self.views.insert(id, Box::new(option));
        }
        for (id, option) in staged_options {
            self.views.insert(id, Box::new(option));
        }
        if let Some(tabs) = self
            .views
            .get_mut(&entity.id)
            .and_then(|view| view.downcast_mut::<Tabs>())
        {
            tabs.option_nodes = next_nodes;
        }
        Ok(())
    }

    /// Move a tab so it sits before `before`. `None` appends it to the end.
    pub fn reorder_tabs(
        &mut self,
        entity: Entity<Tabs>,
        value: impl AsRef<str>,
        before: Option<&str>,
    ) -> Result<bool, FrameworkError> {
        let value = value.as_ref();
        self.update_component(entity, |tabs, cx| {
            if let Some(event) = tabs.reorder(value, before) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    /// Request that the application close a tab. The strip is not mutated.
    pub fn close_tab(
        &mut self,
        entity: Entity<Tabs>,
        value: impl AsRef<str>,
    ) -> Result<bool, FrameworkError> {
        let value = value.as_ref();
        self.update_component(entity, |tabs, cx| {
            if let Some(event) = tabs.request_close(value) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    /// Report a cross-strip transfer. Application code applies both strips.
    pub fn transfer_tab(
        &mut self,
        source: Entity<Tabs>,
        target_strip: impl AsRef<str>,
        value: impl AsRef<str>,
        before: Option<&str>,
    ) -> Result<bool, FrameworkError> {
        let target_strip = target_strip.as_ref();
        let value = value.as_ref();
        self.update_component(source, |tabs, cx| {
            if let Some(event) = tabs.transfer_to(target_strip, value, before) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn navigate_tabs(
        &mut self,
        entity: Entity<Tabs>,
        intent: crate::RovingFocusIntent,
    ) -> Result<bool, FrameworkError> {
        let changed = self.update_component(entity, |tabs, cx| {
            if let Some(event) = tabs.navigate(intent) {
                cx.emit(event);
                true
            } else {
                false
            }
        })?;
        if let Some(target) = self.read(entity, Tabs::roving_target)? {
            let document = self
                .world
                .node(entity.id)
                .ok_or(FrameworkError::MissingView(entity.id))?
                .document;
            let _ = self.focus_node(document, target)?;
        }
        Ok(changed)
    }

    pub fn scroll_to(
        &mut self,
        entity: Entity<ScrollView>,
        offset: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !offset.x.is_finite() || !offset.y.is_finite() || offset.x < 0.0 || offset.y < 0.0 {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        let axes = self.read(entity, |scroll| scroll.axes)?;
        let offset = ScrollOffset {
            x: if matches!(axes, ScrollAxes::Horizontal | ScrollAxes::Both) {
                offset.x
            } else {
                0.0
            },
            y: if matches!(axes, ScrollAxes::Vertical | ScrollAxes::Both) {
                offset.y
            } else {
                0.0
            },
        };
        let offset = self.world.clamp_scroll_offset(entity.id, offset);
        if self.world.scroll_offset(entity.id) == Some(offset) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(entity.id, offset);
        self.world.commit(mutations)?;
        self.update(entity, |_scroll, cx| {
            cx.emit(ScrollChanged { offset });
        })?;
        Ok(true)
    }

    /// Publish measured scroll geometry and clamp an existing offset when the
    /// content or viewport shrinks. Metrics are Runtime-derived state, not a
    /// duplicate field on [`ScrollView`].
    pub fn set_scroll_metrics(
        &mut self,
        entity: Entity<ScrollView>,
        metrics: ScrollMetrics,
    ) -> Result<bool, FrameworkError> {
        self.read(entity, |_| ())?;
        if self.world.scroll_metrics(entity.id) == Some(metrics) {
            return Ok(false);
        }
        let previous = self.world.scroll_offset(entity.id).unwrap_or_default();
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_metrics(entity.id, Some(metrics));
        self.world.commit(mutations)?;
        let offset = self.world.scroll_offset(entity.id).unwrap_or_default();
        if offset != previous {
            self.update(entity, |_scroll, cx| {
                cx.emit(ScrollChanged { offset });
            })?;
        }
        Ok(true)
    }

    /// Move one scroll container by logical-pixel content offsets.
    pub fn scroll_by(
        &mut self,
        entity: Entity<ScrollView>,
        delta: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.read(entity, |_| ())?;
        let current = self.world.scroll_offset(entity.id).unwrap_or_default();
        self.scroll_to(
            entity,
            ScrollOffset {
                x: (current.x + delta.x).max(0.0),
                y: (current.y + delta.y).max(0.0),
            },
        )
    }

    /// Route a logical-pixel scroll delta to the nearest hit scroll container.
    ///
    /// L2 [`ScrollView`] and L1 `overflow: auto|scroll` share [`ScrollOffset`].
    /// At a clamped edge the event bubbles to an enclosing container.
    /// Scrollbar chrome stays on [`ScrollView`] only.
    pub fn scroll_at(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
        delta: ScrollOffset,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        if !x.is_finite() || !y.is_finite() || !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        let Some(target) = self.world.hit_test(document, x, y) else {
            return Ok(None);
        };
        let mut ancestors = Vec::new();
        let mut current = Some(target);
        while let Some(id) = current {
            ancestors.push(id);
            current = self.world.node(id).and_then(|node| node.parent);
        }
        for id in ancestors {
            if self.scroll_node_by(id, delta)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Whether a node is an L2 [`ScrollView`]. Scrollbar drag and hover chrome
    /// key off this; wheel also routes to L1 `overflow: auto|scroll` boxes.
    pub fn is_scroll_view(&self, id: StableNodeId) -> bool {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<ScrollView>())
    }

    /// Whether L1 `overflow: auto|scroll` applies on either axis.
    pub fn overflow_scrolls(&self, id: StableNodeId) -> bool {
        self.world.node_style(id).is_some_and(|style| {
            style.layout.overflow_x.scrolls() || style.layout.overflow_y.scrolls()
        })
    }

    fn overflow_axes(&self, id: StableNodeId) -> Option<(bool, bool)> {
        let style = self.world.node_style(id)?;
        let x = style.layout.overflow_x.scrolls();
        let y = style.layout.overflow_y.scrolls();
        (x || y).then_some((x, y))
    }

    fn write_scroll_metrics(
        &mut self,
        id: StableNodeId,
        metrics: ScrollMetrics,
    ) -> Result<bool, FrameworkError> {
        if self.world.scroll_metrics(id) == Some(metrics) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_metrics(id, Some(metrics));
        self.world.commit(mutations)?;
        Ok(true)
    }

    fn ensure_scroll_metrics(&mut self, id: StableNodeId) -> Result<(), FrameworkError> {
        let Some(metrics) = self.scroll_metrics_from_layout(id) else {
            return Ok(());
        };
        self.write_scroll_metrics(id, metrics)?;
        Ok(())
    }

    /// Move a [`ScrollView`] or L1 overflow scroller by `delta`. Returns
    /// `false` at a clamped edge so the caller can bubble.
    pub(crate) fn scroll_node_by(
        &mut self,
        id: StableNodeId,
        delta: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        if self.is_scroll_view(id) {
            return self.scroll_by(Entity::from_stable_id(id), delta);
        }
        let Some((scrolls_x, scrolls_y)) = self.overflow_axes(id) else {
            return Ok(false);
        };
        self.ensure_scroll_metrics(id)?;
        let current = self.world.scroll_offset(id).unwrap_or_default();
        let next = self.world.clamp_scroll_offset(
            id,
            ScrollOffset {
                x: if scrolls_x {
                    (current.x + delta.x).max(0.0)
                } else {
                    current.x
                },
                y: if scrolls_y {
                    (current.y + delta.y).max(0.0)
                } else {
                    current.y
                },
            },
        );
        if next == current {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(id, next);
        self.world.commit(mutations)?;
        Ok(true)
    }

    fn scrollbar_bar(
        &self,
        id: StableNodeId,
        axis: nana_ui_core::ScrollbarAxis,
    ) -> Option<crate::ScrollbarBar> {
        match self.world.component_geometry(id) {
            Some(crate::ComponentGeometry::Scrollbar {
                horizontal,
                vertical,
            }) => match axis {
                nana_ui_core::ScrollbarAxis::Horizontal => horizontal,
                nana_ui_core::ScrollbarAxis::Vertical => vertical,
            },
            _ => None,
        }
    }

    /// Which scrollbar axis of a scroll container a viewport point lands on.
    ///
    /// The vertical bar wins an overlap, matching its drawn order.
    pub fn scrollbar_axis_at(
        &self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<nana_ui_core::ScrollbarAxis> {
        [
            nana_ui_core::ScrollbarAxis::Vertical,
            nana_ui_core::ScrollbarAxis::Horizontal,
        ]
        .into_iter()
        .find(|axis| {
            self.scrollbar_bar(id, *axis)
                .is_some_and(|bar| bar.contains(x, y))
        })
    }

    /// Find the innermost scroll container whose scrollbar is under a point.
    ///
    /// Scrollbars overlay content, so the hit-test target is usually a child of
    /// the container that owns the bar.
    pub fn scrollbar_target_near(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Option<(StableNodeId, nana_ui_core::ScrollbarAxis)> {
        let mut current = self.world.hit_test(document, x, y);
        while let Some(id) = current {
            if let Some(axis) = self.scrollbar_axis_at(id, x, y) {
                return Some((id, axis));
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        None
    }

    /// Grab a scrollbar. A press on bare track pages toward the point first, so
    /// the thumb is under the pointer when the drag starts.
    pub fn begin_scrollbar_drag(
        &mut self,
        pointer_id: u64,
        target: StableNodeId,
        axis: nana_ui_core::ScrollbarAxis,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(bar) = self.scrollbar_bar(target, axis) else {
            return Ok(false);
        };
        let entity = Entity::<ScrollView>::from_stable_id(target);
        self.read(entity, |_| ())?;
        let position = bar.axis_position(axis, x, y);
        let track = bar.track_geometry(axis);
        // Cancel restores what the press started from, including any track jump.
        let initial_offset = self.world.scroll_offset(target).unwrap_or_default();
        let grab_offset = if track.thumb_contains(position) {
            position - track.thumb_origin
        } else {
            // Centre the thumb on the press, then keep dragging from there.
            let hold = self.axis_hold(target, axis);
            let offset = track.offset_for_position(position);
            self.scroll_to(entity, scroll_offset_on(axis, offset, hold))?;
            track.thumb_length / 2.0
        };
        self.update_component(entity, |scroll, cx| {
            scroll.dragging = Some(crate::ScrollbarDragState {
                pointer_id,
                axis,
                grab_offset,
                initial_offset,
            });
            cx.mutations().capture_pointer(pointer_id, target);
        })?;
        Ok(true)
    }

    /// The offset on the axis a drag is not touching, so it stays put.
    fn axis_hold(&self, id: StableNodeId, axis: nana_ui_core::ScrollbarAxis) -> f32 {
        let offset = self.world.scroll_offset(id).unwrap_or_default();
        match axis {
            nana_ui_core::ScrollbarAxis::Horizontal => offset.y,
            nana_ui_core::ScrollbarAxis::Vertical => offset.x,
        }
    }

    pub fn update_scrollbar_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_scroll_view(target) {
            return Ok(false);
        }
        let entity = Entity::<ScrollView>::from_stable_id(target);
        let Some(drag) = self.read(entity, |scroll| scroll.dragging)? else {
            return Ok(false);
        };
        if drag.pointer_id != pointer_id {
            return Ok(false);
        }
        let Some(bar) = self.scrollbar_bar(target, drag.axis) else {
            return Ok(false);
        };
        let track = bar.track_geometry(drag.axis);
        let offset =
            track.offset_for_thumb_origin(bar.axis_position(drag.axis, x, y) - drag.grab_offset);
        let hold = self.axis_hold(target, drag.axis);
        self.scroll_to(entity, scroll_offset_on(drag.axis, offset, hold))
    }

    pub fn end_scrollbar_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_scroll_view(target) {
            return Ok(false);
        }
        let entity = Entity::<ScrollView>::from_stable_id(target);
        let Some(drag) = self.read(entity, |scroll| scroll.dragging)? else {
            return Ok(false);
        };
        if drag.pointer_id != pointer_id {
            return Ok(false);
        }
        if cancel {
            self.scroll_to(entity, drag.initial_offset)?;
        }
        self.update_component(entity, |scroll, cx| {
            scroll.dragging = None;
            cx.mutations().release_pointer(pointer_id, target);
        })?;
        Ok(true)
    }

    /// Reveal auto-hiding scrollbars for the container under the pointer.
    fn sync_scroll_view_hover(
        &mut self,
        previous: Option<StableNodeId>,
        target: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let entered = target.and_then(|id| self.enclosing_scroll_view(id));
        let left = previous.and_then(|id| self.enclosing_scroll_view(id));
        if left == entered {
            return Ok(());
        }
        if let Some(id) = left {
            self.set_scroll_view_hover(id, false)?;
        }
        if let Some(id) = entered {
            self.set_scroll_view_hover(id, true)?;
        }
        Ok(())
    }

    fn set_scroll_view_hover(
        &mut self,
        id: StableNodeId,
        hovered: bool,
    ) -> Result<(), FrameworkError> {
        let entity = Entity::<ScrollView>::from_stable_id(id);
        if self.read(entity, |scroll| scroll.hovered)? == hovered {
            return Ok(());
        }
        self.update_component(entity, |scroll, _| {
            scroll.hovered = hovered;
        })?;
        Ok(())
    }

    fn enclosing_scroll_view(&self, id: StableNodeId) -> Option<StableNodeId> {
        let mut current = Some(id);
        while let Some(id) = current {
            if self.is_scroll_view(id) {
                return Some(id);
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        None
    }

    /// Route a backend-neutral table navigation intent from current focus.
    pub fn navigate_focused_table(
        &mut self,
        document: DocumentId,
        navigation: crate::TableNavigation,
        page_rows: usize,
    ) -> Result<bool, FrameworkError> {
        let mut current = self.world.focused(document);
        while let Some(id) = current {
            if self.views.get(&id).is_some_and(|view| view.is::<Table>()) {
                return self.navigate_table(Entity::from_stable_id(id), navigation, page_rows);
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        Ok(false)
    }

    /// Replace the active UTF-8 selection and notify typed observers without
    /// requiring an application-wide message enum.
    pub fn replace_text_input_selection(
        &mut self,
        entity: Entity<TextInput>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        self.replace_editable_selection(entity, text)
    }

    pub fn replace_text_area_selection(
        &mut self,
        entity: Entity<TextArea>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        self.replace_editable_selection(entity, text)
    }

    fn replace_editable_selection<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if !editable.replace_selection(text) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    /// Activate one direct overlay child and move focus into its retained
    /// subtree in the same Runtime transaction.
    pub fn activate_overlay<O: View>(
        &mut self,
        host: Entity<OverlayHost>,
        overlay: Entity<O>,
    ) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        self.read(overlay, |_| ())?;
        let overlay_node = self
            .world
            .node(overlay.id)
            .ok_or(FrameworkError::MissingView(overlay.id))?;
        if overlay_node.parent != Some(host.id) {
            return Err(FrameworkError::InvalidComponentHierarchy {
                parent: host.id,
                child: overlay.id,
            });
        }
        let previous =
            self.world
                .overlay_host(host.id)
                .ok_or(FrameworkError::InvalidComponentHierarchy {
                    parent: host.id,
                    child: overlay.id,
                })?;
        if previous.active == Some(overlay.id) {
            return Ok(false);
        }
        if self.runtime_overlay_kind(overlay.id).is_none() {
            return Err(FrameworkError::ViewType(overlay.id));
        }
        let activation_focus = self.modal_initial_focus(overlay_node.document, overlay.id)?;
        self.validate_modal_slots_for_activation(overlay.id)?;
        let previous_focus = self.world.focused(overlay_node.document);
        let restore_focus = previous.restore_focus.or(previous_focus);
        let next = crate::OverlayHostState {
            active: Some(overlay.id),
            restore_focus,
        };
        let activation_token = self
            .component_lifecycle
            .next_overlay_activation_token
            .checked_add(1)
            .ok_or(FrameworkError::OverlayActivationTokenExhausted(host.id))?;
        self.update_overlay_host(host, next, overlay_node.document, None, activation_focus)?;
        let Some(final_active) = self
            .world
            .overlay_host(host.id)
            .and_then(|state| state.active)
        else {
            return Ok(false);
        };
        let final_validation = self
            .world
            .is_overlay_reachable(final_active)
            .then_some(())
            .ok_or(FrameworkError::InvalidComponentValue(final_active))
            .and_then(|_| {
                self.world
                    .node(final_active)
                    .ok_or(FrameworkError::MissingView(final_active))
            })
            .and_then(|node| {
                if node.parent == Some(host.id) && node.document == overlay_node.document {
                    Ok(node)
                } else {
                    Err(FrameworkError::InvalidComponentHierarchy {
                        parent: host.id,
                        child: final_active,
                    })
                }
            })
            .and_then(|_| {
                self.runtime_overlay_kind(final_active)
                    .ok_or(FrameworkError::ViewType(final_active))
            })
            .and_then(|kind| {
                self.validate_modal_slots_for_activation(final_active)?;
                Ok(kind)
            });
        let final_kind = match final_validation {
            Ok(kind) => kind,
            Err(error) => {
                let rollback_active = previous.active.filter(|active| {
                    self.world.node(*active).is_some_and(|node| {
                        node.parent == Some(host.id)
                            && node.document == overlay_node.document
                            && self.world.is_mounted(*active)
                    })
                });
                let rollback_state = crate::OverlayHostState {
                    active: rollback_active,
                    restore_focus: rollback_active.and(previous.restore_focus),
                };
                let rollback_focus = previous_focus.filter(|focus| {
                    self.sequential_focus_candidate(overlay_node.document, *focus)
                        && rollback_active
                            .is_none_or(|root| self.overlay_reachable_within(root, *focus))
                });
                let mut rollback = MutationQueue::new();
                rollback.set_overlay_host(host.id, rollback_state);
                rollback.set_interaction(host.id, self.overlay_host_interaction(rollback_active));
                rollback.request_focus(overlay_node.document, rollback_focus);
                self.commit_mutations(rollback)?;
                return Err(error);
            }
        };
        if !final_kind.blocks_pointer() {
            return Ok(true);
        }
        self.component_lifecycle.next_overlay_activation_token = activation_token;
        self.component_lifecycle
            .overlay_activation_tokens
            .insert(host.id, activation_token);
        self.prepare_blocking_overlay_activation(overlay_node.document, final_active);
        Ok(true)
    }

    pub fn dismiss_overlay(&mut self, host: Entity<OverlayHost>) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        let previous = self
            .world
            .overlay_host(host.id)
            .ok_or(FrameworkError::MissingView(host.id))?;
        if previous.active.is_none() {
            return Ok(false);
        }
        let document = self
            .world
            .node(host.id)
            .ok_or(FrameworkError::MissingView(host.id))?
            .document;
        self.update_overlay_host(
            host,
            crate::OverlayHostState::default(),
            document,
            previous.restore_focus,
            None,
        )?;
        Ok(true)
    }

    pub fn dismiss_dialog(
        &mut self,
        host: Entity<OverlayHost>,
        trigger: nana_ui_core::DialogCloseTrigger,
    ) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        let Some(active) = self
            .world
            .overlay_host(host.id)
            .and_then(|state| state.active)
        else {
            return Ok(false);
        };
        if self.runtime_overlay_kind(active) != Some(RuntimeOverlayKind::Dialog) {
            return Err(FrameworkError::ViewType(active));
        }
        self.request_dialog_close(host, trigger)
    }

    pub fn request_dialog_close(
        &mut self,
        host: Entity<OverlayHost>,
        trigger: nana_ui_core::DialogCloseTrigger,
    ) -> Result<bool, FrameworkError> {
        self.read(host, |_| ())?;
        let Some(active) = self
            .world
            .overlay_host(host.id)
            .and_then(|state| state.active)
        else {
            return Ok(false);
        };
        if !self.dialog_allows(active, trigger) {
            return Ok(false);
        }
        self.dismiss_overlay(host)
    }

    fn update_overlay_host(
        &mut self,
        host: Entity<OverlayHost>,
        next: crate::OverlayHostState,
        document: DocumentId,
        dismiss_restore: Option<StableNodeId>,
        activation_focus: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let focus = activation_focus
            .or_else(|| {
                next.active
                    .and_then(|active| self.first_overlay_focusable(document, active))
            })
            .or_else(|| {
                next.restore_focus
                    .or(dismiss_restore)
                    .filter(|id| self.overlay_focus_candidate(document, *id))
            });
        let host_id = host.id;
        let interaction = self.overlay_host_interaction(next.active);
        self.update_component(host, |_host, cx| {
            cx.mutations().set_overlay_host(host_id, next);
            cx.mutations().set_interaction(host_id, interaction);
            cx.mutations().request_focus(document, focus);
            cx.emit(OverlayChanged {
                active: next.active,
            });
        })
    }

    /// A host stretches across its whole region, so it may only take the
    /// pointer while a modal overlay is up; a passive one such as a toast has
    /// to leave the workspace underneath usable. Projection cannot decide this
    /// because it reads the world before the new activation is committed.
    fn overlay_host_interaction(&self, active: Option<StableNodeId>) -> crate::InteractionState {
        let blocks_pointer = active
            .and_then(|id| self.world.accessibility(id))
            .and_then(|accessibility| overlay_kind_for_role(accessibility.role))
            .is_some_and(RuntimeOverlayKind::blocks_pointer);
        crate::InteractionState {
            pointer_events: blocks_pointer,
            focusable: false,
        }
    }

    /// Move table focus using backend-neutral navigation intent. The retained
    /// hierarchy is the row/cell authority; no parallel selection matrix is
    /// stored by the component or platform adapter.
    pub fn navigate_table(
        &mut self,
        table: Entity<Table>,
        navigation: crate::TableNavigation,
        page_rows: usize,
    ) -> Result<bool, FrameworkError> {
        self.read(table, |_| ())?;
        let table_node = self
            .world
            .node(table.id)
            .ok_or(FrameworkError::MissingView(table.id))?;
        let rows = table_node
            .children
            .iter()
            .filter_map(|row| {
                let row = self.world.node(*row)?;
                matches!(&row.kind, NodeKind::Element { tag } if tag == "tr").then(|| {
                    row.children
                        .into_iter()
                        .filter(|cell| {
                            self.world.node(*cell).is_some_and(|cell| {
                                matches!(&cell.kind, NodeKind::Element { tag } if tag == "td" || tag == "th")
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .filter(|cells| !cells.is_empty())
            .collect::<Vec<_>>();
        let column_count = rows.iter().map(Vec::len).max().unwrap_or_default();
        if rows.is_empty() || column_count == 0 {
            return Ok(false);
        }
        let focused = self.world.focused(table_node.document);
        let current = focused.and_then(|focused| {
            rows.iter().enumerate().find_map(|(row, cells)| {
                cells
                    .iter()
                    .position(|cell| *cell == focused)
                    .map(|column| crate::TableCursor { row, column })
            })
        });
        let mut cursor = current.unwrap_or(crate::TableCursor { row: 0, column: 0 });
        let moved =
            current.is_none() || cursor.navigate(navigation, rows.len(), column_count, page_rows);
        if !moved {
            return Ok(false);
        }
        cursor.column = cursor.column.min(rows[cursor.row].len() - 1);
        let cell = rows[cursor.row][cursor.column];
        self.update_component(table, |_table, cx| {
            cx.mutations()
                .request_focus(table_node.document, Some(cell));
            cx.emit(crate::TableCellFocused {
                row: cursor.row,
                column: cursor.column,
                cell,
            });
        })?;
        Ok(true)
    }

    pub fn read<V: View, R>(
        &self,
        entity: Entity<V>,
        read: impl FnOnce(&V) -> R,
    ) -> Result<R, FrameworkError> {
        if !self.world.contains(entity.id) {
            return Err(FrameworkError::MissingView(entity.id));
        }
        let view = self
            .views
            .get(&entity.id)
            .ok_or(FrameworkError::MissingView(entity.id))?
            .downcast_ref::<V>()
            .ok_or(FrameworkError::ViewType(entity.id))?;
        Ok(read(view))
    }

    fn inherit_segmented_option_surface<T: 'static>(&self, id: StableNodeId, staged: &mut T) {
        if TypeId::of::<T>() != TypeId::of::<SegmentedOption>() {
            return;
        }
        let Some(parent) = self.world.node(id).and_then(|node| node.parent) else {
            return;
        };
        let Some(control) = self
            .views
            .get(&parent)
            .and_then(|view| view.downcast_ref::<SegmentedControl>())
        else {
            return;
        };
        let (size, chrome, fill) = (control.size, control.chrome, control.fill);
        // SAFETY: `TypeId` matched `SegmentedOption`.
        let option = unsafe { &mut *std::ptr::from_mut(staged).cast::<SegmentedOption>() };
        option.synchronize_surface(size, chrome, fill);
    }

    /// Update typed state, deliver closure events, then atomically commit all
    /// retained-tree mutations produced by the update.
    pub fn update<V: View, R>(
        &mut self,
        entity: Entity<V>,
        update: impl FnOnce(&mut V, &mut ViewContext<'_, V>) -> R,
    ) -> Result<R, FrameworkError> {
        self.update_inner(entity, update, |_view, _world, _mutations| {})
    }

    /// Update component state and project the final state after all closure
    /// events emitted by the update have been delivered.
    pub fn update_component<C: ComponentView, R>(
        &mut self,
        entity: Entity<C>,
        update: impl FnOnce(&mut C, &mut ViewContext<'_, C>) -> R,
    ) -> Result<R, FrameworkError> {
        if !self.world.contains(entity.id) {
            return Err(FrameworkError::MissingView(entity.id));
        }
        let boxed = self
            .views
            .remove(&entity.id)
            .ok_or(FrameworkError::MissingView(entity.id))?;
        let Some(component) = boxed.downcast_ref::<C>() else {
            self.views.insert(entity.id, boxed);
            return Err(FrameworkError::ViewType(entity.id));
        };
        let mut staged = component.clone();
        let mut mutations = MutationQueue::new();
        let mut events = VecDeque::new();
        let mut program_messages = std::mem::take(&mut self.program_messages);
        let result = update(
            &mut staged,
            &mut ViewContext {
                entity,
                mutations: &mut mutations,
                events: &mut events,
                program_messages: &mut program_messages,
            },
        );
        self.inherit_segmented_option_surface(entity.id, &mut staged);
        let delivered = self.deliver_events(
            entity.id,
            &mut staged,
            &mut mutations,
            &mut events,
            &mut program_messages,
        );
        self.program_messages = program_messages;
        if delivered.is_ok() {
            staged.project(entity.id, &self.world, &mut mutations);
        }
        let commit = delivered.and_then(|()| self.commit_mutations(mutations).map(|_| ()));
        if commit.is_ok() {
            self.views.insert(entity.id, Box::new(staged));
        } else {
            self.views.insert(entity.id, boxed);
        }
        commit?;
        if !self.world.is_mounted(entity.id) {
            self.suspend_component_lifecycle(entity.id);
        }
        self.sync_component_lifecycle(entity.id)?;
        Ok(result)
    }

    fn update_inner<V: View, R>(
        &mut self,
        entity: Entity<V>,
        update: impl FnOnce(&mut V, &mut ViewContext<'_, V>) -> R,
        project: impl FnOnce(&V, &UiWorld, &mut MutationQueue),
    ) -> Result<R, FrameworkError> {
        if !self.world.contains(entity.id) {
            return Err(FrameworkError::MissingView(entity.id));
        }
        let mut boxed = self
            .views
            .remove(&entity.id)
            .ok_or(FrameworkError::MissingView(entity.id))?;
        if !boxed.is::<V>() {
            self.views.insert(entity.id, boxed);
            return Err(FrameworkError::ViewType(entity.id));
        }
        let view = boxed
            .downcast_mut::<V>()
            .expect("view type was checked before update");
        let mut mutations = MutationQueue::new();
        let mut events = VecDeque::new();
        let mut program_messages = std::mem::take(&mut self.program_messages);
        let result = update(
            view,
            &mut ViewContext {
                entity,
                mutations: &mut mutations,
                events: &mut events,
                program_messages: &mut program_messages,
            },
        );
        self.inherit_segmented_option_surface(entity.id, view);
        let delivered = self.deliver_events(
            entity.id,
            view,
            &mut mutations,
            &mut events,
            &mut program_messages,
        );
        self.program_messages = program_messages;
        if delivered.is_ok() {
            project(view, &self.world, &mut mutations);
        }
        let commit = delivered.and_then(|()| self.commit_mutations(mutations).map(|_| ()));
        self.views.insert(entity.id, boxed);
        if commit.is_ok() && !self.world.is_mounted(entity.id) {
            self.suspend_component_lifecycle(entity.id);
        }
        commit.map(|_| result)
    }

    pub fn remove_view<V: View>(&mut self, entity: Entity<V>) -> Result<V, FrameworkError> {
        self.read(entity, |_| ())?;
        let boxed = self
            .views
            .remove(&entity.id)
            .expect("validated view must remain present");
        self.despawn_node(entity.id)?;
        boxed
            .downcast::<V>()
            .map(|view| *view)
            .map_err(|_| FrameworkError::ViewType(entity.id))
    }

    pub(super) fn attach_child(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
    ) -> Result<(), FrameworkError> {
        if !self.world.contains(parent) {
            return Err(FrameworkError::MissingView(parent));
        }
        if !self.world.contains(child) {
            return Err(FrameworkError::MissingView(child));
        }
        let mut queue = MutationQueue::new();
        queue.insert(parent, child, None);
        self.commit_mutations(queue)?;
        Ok(())
    }

    pub(super) fn despawn_node(&mut self, id: StableNodeId) -> Result<(), FrameworkError> {
        if !self.world.contains(id) {
            return Err(FrameworkError::MissingView(id));
        }
        let mut subtree = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let snapshot = self
                .world
                .node(current)
                .ok_or(FrameworkError::MissingView(current))?;
            stack.extend(snapshot.children.iter().rev().copied());
            subtree.push(current);
        }
        let mut queue = MutationQueue::new();
        queue.despawn_subtree(id);
        self.world.commit(queue)?;
        let removed = subtree.iter().copied().collect::<HashSet<_>>();
        self.forget_subtree(&removed);
        Ok(())
    }

    fn forget_subtree(&mut self, removed: &HashSet<StableNodeId>) {
        self.remove_event_handlers_for(removed);
        for id in removed {
            self.component_lifecycle.tooltips.remove(id);
            self.component_lifecycle.loading.remove(id);
            self.views.remove(id);
        }
        self.component_lifecycle
            .tooltips
            .retain(|_, tooltip| !removed.contains(&tooltip.overlay));
        if self.component_lifecycle.loading.is_empty() {
            self.component_lifecycle.next_loading_frame = None;
        }
        self.assembled.retain(|parent, slots| {
            if removed.contains(parent) {
                return false;
            }
            slots.retain(|_, child| !removed.contains(&child.id));
            true
        });
    }

    fn remove_event_handlers_for(&mut self, removed: &HashSet<StableNodeId>) {
        self.event_handlers.retain(|(id, _), handlers| {
            if removed.contains(id) {
                return false;
            }
            handlers.retain(|handler| !removed.contains(&handler.observer));
            !handlers.is_empty()
        });
    }

    pub fn on<V, E>(
        &mut self,
        entity: Entity<V>,
        mut handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) -> Result<(), FrameworkError>
    where
        V: View,
        E: Send + 'static,
    {
        self.read(entity, |_| ())?;
        let erased = move |view: &mut dyn Any,
                           event: &dyn Any,
                           mutations: &mut MutationQueue,
                           events: &mut VecDeque<BoxedEvent>,
                           program_messages: &mut Vec<ProgramMessage>| {
            let view = view
                .downcast_mut::<V>()
                .expect("handler is registered for the entity view type");
            let event = event
                .downcast_ref::<E>()
                .expect("handler is indexed by event type");
            handler(
                view,
                event,
                &mut ViewContext {
                    entity,
                    mutations,
                    events,
                    program_messages,
                },
            );
        };
        self.event_handlers
            .entry((entity.id, TypeId::of::<E>()))
            .or_default()
            .push(EventHandler {
                observer: entity.id,
                callback: Box::new(erased),
            });
        Ok(())
    }

    pub fn observe<S, V, E>(
        &mut self,
        source: Entity<S>,
        observer: Entity<V>,
        mut handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) -> Result<(), FrameworkError>
    where
        S: View,
        V: View,
        E: Send + 'static,
    {
        self.read(source, |_| ())?;
        self.read(observer, |_| ())?;
        let erased = move |view: &mut dyn Any,
                           event: &dyn Any,
                           mutations: &mut MutationQueue,
                           events: &mut VecDeque<BoxedEvent>,
                           program_messages: &mut Vec<ProgramMessage>| {
            let view = view
                .downcast_mut::<V>()
                .expect("observer handler is registered for its view type");
            let event = event
                .downcast_ref::<E>()
                .expect("observer handler is indexed by event type");
            handler(
                view,
                event,
                &mut ViewContext {
                    entity: observer,
                    mutations,
                    events,
                    program_messages,
                },
            );
        };
        self.event_handlers
            .entry((source.id, TypeId::of::<E>()))
            .or_default()
            .push(EventHandler {
                observer: observer.id,
                callback: Box::new(erased),
            });
        Ok(())
    }

    pub fn register_action(
        &mut self,
        id: impl Into<ActionId>,
        when: ContextPredicate,
        handler: impl FnMut(&mut AppContext) -> Result<(), FrameworkError> + Send + 'static,
    ) -> Result<(), FrameworkError> {
        insert_action(&mut self.actions, id, when, handler)
    }

    pub fn dispatch_action(
        &mut self,
        id: &ActionId,
        context: &KeyContext,
    ) -> Result<(), FrameworkError> {
        let mut action = self
            .actions
            .remove(id)
            .ok_or_else(|| FrameworkError::MissingAction(id.clone()))?;
        if !action.when.matches(context) {
            self.actions.insert(id.clone(), action);
            return Err(FrameworkError::ActionUnavailable(id.clone()));
        }
        let result = (action.handler)(self);
        self.actions.insert(id.clone(), action);
        result
    }

    pub fn install(&mut self, extension: &impl UiExtension) -> Result<(), FrameworkError> {
        let name = extension.name().trim().to_owned();
        if name.is_empty() {
            return Err(FrameworkError::InvalidExtension);
        }
        if self.extensions.contains(&name) {
            return Err(FrameworkError::DuplicateExtension(name));
        }
        let mut registrar = ExtensionRegistrar::default();
        extension.install(&mut registrar)?;
        if let Some(id) = registrar
            .actions
            .keys()
            .find(|id| self.actions.contains_key(*id))
        {
            return Err(FrameworkError::DuplicateAction(id.clone()));
        }
        if let Some(presenter) = registrar
            .presenters
            .iter()
            .find(|presenter| self.world.has_presenter(presenter.name()))
        {
            return Err(FrameworkError::DuplicatePresenter(
                presenter.name().to_owned(),
            ));
        }
        if registrar
            .activations
            .keys()
            .any(|type_id| self.activations.contains_key(type_id))
        {
            return Err(FrameworkError::DuplicateActivation);
        }
        self.components.extend(registrar.components)?;
        self.actions.extend(registrar.actions);
        self.activations.extend(registrar.activations);
        for presenter in registrar.presenters {
            self.world.register_presenter(presenter)?;
        }
        self.extensions.insert(name);
        Ok(())
    }

    fn register_builtin_activations(&mut self) {
        self.bind_activation::<Button>(Self::activate_button);
        self.bind_activation::<IconButton>(Self::activate_icon_button);
        self.bind_activation::<ListItem>(Self::activate_list_item);
        self.bind_activation::<SidebarRow>(Self::activate_sidebar_row);
        self.bind_activation::<SidebarFooterButton>(Self::activate_sidebar_footer_button);
        self.bind_activation::<SidebarSection>(Self::activate_sidebar_section);
        self.bind_activation::<SettingsCollapsibleCard>(Self::activate_settings_collapsible_card);
        self.bind_activation::<Tabs>(Self::activate_tabs);
        self.bind_activation::<ActionMenuItem>(Self::activate_action_menu_item);
        self.bind_activation::<Select>(Self::toggle_select);
        self.bind_activation::<Dropdown>(Self::toggle_dropdown);
        self.bind_activation::<SearchDropdown>(Self::toggle_search_dropdown);
        self.bind_activation::<Popover>(Self::toggle_popover);
        self.bind_activation::<ActionMenu>(Self::toggle_action_menu);
        self.bind_activation::<ContextMenu>(Self::dismiss_context_menu);
        self.bind_activation::<Checkbox>(Self::toggle_checkbox);
        self.bind_activation::<Switch>(Self::toggle_switch);
        self.bind_activation::<Progress>(Self::cancel_progress);
        self.bind_activation::<SegmentedOption>(Self::activate_segmented_option);
    }

    fn bind_activation<C: View>(
        &mut self,
        handler: fn(&mut Self, Entity<C>) -> Result<bool, FrameworkError>,
    ) {
        self.activations.insert(
            TypeId::of::<C>(),
            Arc::new(move |context, id| handler(context, Entity::from_stable_id(id))),
        );
    }

    pub fn register_presenter(
        &mut self,
        presenter: Box<dyn TextPresenter>,
    ) -> Result<(), FrameworkError> {
        self.world
            .register_presenter(presenter)
            .map_err(FrameworkError::from)
    }

    pub(super) fn stamp_component_type<C: ComponentView>(
        &mut self,
        id: StableNodeId,
        queue: &mut MutationQueue,
    ) {
        if let Some(entry) = self.components.get_by_rust(TypeId::of::<C>()) {
            queue.set_component_type(id, Some(entry.id.clone()));
        }
        self.secondary_presses
            .entry(TypeId::of::<C>())
            .or_insert_with(|| {
                Arc::new(|context: &mut AppContext, id, press| {
                    context.update_component(Entity::<C>::from_stable_id(id), |_, cx| {
                        cx.emit(press);
                    })
                })
            });
    }

    pub(super) fn allocate_id(&mut self) -> StableNodeId {
        loop {
            let id = StableNodeId::new(self.next_id).expect("allocator never emits zero");
            self.next_id = self
                .next_id
                .checked_add(1)
                .expect("stable ID space exhausted");
            if !self.world.contains(id) && !self.world.is_retired(id) {
                return id;
            }
        }
    }

    fn deliver_events(
        &mut self,
        id: StableNodeId,
        view: &mut dyn Any,
        mutations: &mut MutationQueue,
        events: &mut VecDeque<BoxedEvent>,
        program_messages: &mut Vec<ProgramMessage>,
    ) -> Result<(), FrameworkError> {
        let mut delivered = 0;
        while let Some((emitter, event_type, event)) = events.pop_front() {
            delivered += 1;
            if delivered > MAX_EVENTS_PER_UPDATE {
                return Err(FrameworkError::EventOverflow(emitter));
            }
            let key = (emitter, event_type);
            let Some(mut handlers) = self.event_handlers.remove(&key) else {
                continue;
            };
            for handler in &mut handlers {
                if handler.observer == id {
                    (handler.callback)(view, event.as_ref(), mutations, events, program_messages);
                    continue;
                }
                let Some(mut observer) = self.views.remove(&handler.observer) else {
                    continue;
                };
                (handler.callback)(
                    observer.as_mut(),
                    event.as_ref(),
                    mutations,
                    events,
                    program_messages,
                );
                self.views.insert(handler.observer, observer);
            }
            self.event_handlers.insert(key, handlers);
        }
        Ok(())
    }
}

pub trait UiExtension {
    fn name(&self) -> &'static str;
    fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError>;
}

fn normalized_action_id(id: impl Into<ActionId>) -> Result<ActionId, FrameworkError> {
    let id = id.into();
    let id = ActionId::new(id.as_str().trim());
    if id.as_str().is_empty() {
        Err(FrameworkError::InvalidAction)
    } else {
        Ok(id)
    }
}

fn insert_action(
    actions: &mut HashMap<ActionId, RegisteredAction>,
    id: impl Into<ActionId>,
    when: ContextPredicate,
    handler: impl FnMut(&mut AppContext) -> Result<(), FrameworkError> + Send + 'static,
) -> Result<(), FrameworkError> {
    let id = normalized_action_id(id)?;
    if actions.contains_key(&id) {
        return Err(FrameworkError::DuplicateAction(id));
    }
    actions.insert(
        id,
        RegisteredAction {
            when,
            handler: Box::new(handler),
        },
    );
    Ok(())
}

#[must_use = "tasks do nothing until a host executor runs them"]
pub struct Task<T> {
    future: Pin<Box<dyn Future<Output = T> + Send + 'static>>,
}

impl<T> Task<T> {
    pub fn new(future: impl Future<Output = T> + Send + 'static) -> Self {
        Self {
            future: Box::pin(future),
        }
    }

    pub fn ready(value: T) -> Self
    where
        T: Send + 'static,
    {
        Self::new(async move { value })
    }

    pub fn into_future(self) -> Pin<Box<dyn Future<Output = T> + Send + 'static>> {
        self.future
    }

    pub fn map<U: Send + 'static>(self, map: impl FnOnce(T) -> U + Send + 'static) -> Task<U>
    where
        T: Send + 'static,
    {
        Task::new(async move { map(self.future.await) })
    }
}

/// Wake-driven event stream. Platform adapters own execution and cancellation;
/// the core runtime does not create a competing executor.
#[must_use = "subscriptions do nothing until a host consumes their stream"]
pub struct Subscription<T> {
    id: String,
    stream: Pin<Box<dyn Stream<Item = T> + Send + 'static>>,
}

impl<T> Subscription<T> {
    pub fn new(id: impl Into<String>, stream: impl Stream<Item = T> + Send + 'static) -> Self {
        Self {
            id: id.into(),
            stream: Box::pin(stream),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = T> + Send + 'static>> {
        self.stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    use crate::{
        Activate, AnimationId, AnimationSpec, Button, Card, Checkbox, Easing, IconButton, List,
        ListItem, NodeStyle, RangeChanged, RangeField, ScrollAxes, ScrollChanged, ScrollView,
        SegmentedControl, SegmentedOption, SegmentedSelectionRequested, Stack, StandardVisual,
        Switch, TabOption, Table, TableCell, TableCellFocused, TableNavigation, TableRow, Tabs,
        Text, TextArea, TextChanged, TextContent, TextInput, TextSelection, ToggleChanged,
    };

    #[derive(Debug)]
    struct Counter {
        value: usize,
    }

    struct Increment(usize);
    struct Cascade;

    #[test]
    fn bind_component_requires_an_existing_node_then_enables_read() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let id = StableNodeId::new(7).unwrap();
        let button = Button::new("Go");
        assert_eq!(
            context.bind_component(id, button.clone()),
            Err(FrameworkError::MissingView(id))
        );
        let mut queue = MutationQueue::new();
        queue.create(id, document, button.node_kind());
        context.commit_mutations(queue).unwrap();
        let entity = context.bind_component(id, button).unwrap();
        assert_eq!(entity.stable_id(), id);
        assert_eq!(
            context
                .read(entity, |button| button.label.to_string())
                .unwrap(),
            "Go"
        );
        assert_eq!(context.world().text(id), Some("Go"));
    }

    #[test]
    fn mount_reuses_keyed_entities_and_drops_unused() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let card = context.create_component(document, Card::new()).unwrap();
        let mut title = None;
        let mut save = None;
        context
            .mount(card, |ui| {
                title = Some(ui.child("title", Text::new("Nana"))?);
                save = Some(ui.child("save", Button::new("Save"))?);
                Ok(())
            })
            .unwrap();
        let title = title.unwrap();
        let save = save.unwrap();
        let title_id = title.stable_id();
        let save_id = save.stable_id();
        context
            .mount(card, |ui| {
                ui.child("title", Text::new("Nana"))?;
                ui.child("save", Button::new("Saved"))?;
                Ok(())
            })
            .unwrap();
        assert_eq!(title.stable_id(), title_id);
        assert_eq!(save.stable_id(), save_id);
        assert_eq!(
            context.read(save, |button| button.label.clone()).unwrap(),
            "Saved"
        );
        context
            .mount(card, |ui| {
                ui.child("title", Text::new("Nana"))?;
                Ok(())
            })
            .unwrap();
        assert!(context.world().contains(title_id));
        assert!(!context.world().contains(save_id));
    }

    #[test]
    fn build_commits_nested_tree_once_and_installs_handlers() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let before = context.world().generation();
        let start = context
            .build(document, |ui| {
                ui.column(12.0, |ui| {
                    ui.child("title", Text::new("你好"));
                    let start = ui.child("start", Button::new("开始"));
                    ui.on(start, |_, _: &Activate, cx| {
                        cx.dispatch_program("start");
                    });
                    ui.row(8.0, |ui| {
                        ui.child("open", Button::new("打开"));
                        ui.child("float", Button::new("浮窗"));
                    });
                    start
                })
            })
            .unwrap();
        assert_eq!(context.world().generation(), before + 1);

        let start_node = context.world().node(start.stable_id()).unwrap();
        let column = start_node.parent.expect("start lives under column");
        let column_children = context.world().node(column).unwrap().children;
        assert_eq!(column_children.len(), 3);
        assert_eq!(context.world().text(column_children[0]), Some("你好"));
        assert_eq!(column_children[1], start.stable_id());
        let row = column_children[2];
        let row_children = context.world().node(row).unwrap().children;
        assert_eq!(row_children.len(), 2);
        assert_eq!(
            context.read(start, |button| button.label.clone()).unwrap(),
            "开始"
        );
        assert!(context.activate_button(start).unwrap());
        let queued = context.take_program_messages();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].downcast_ref::<&str>().copied(), Some("start"));
    }

    #[test]
    fn create_component_append_commits_once_per_call() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let before = context.world().generation();
        let column = context
            .create_component(document, Stack::column(12.0))
            .unwrap();
        let title = context
            .create_component(document, Text::new("你好"))
            .unwrap();
        let start = context
            .create_component(document, Button::new("开始"))
            .unwrap();
        context.append_child(column, title).unwrap();
        context.append_child(column, start).unwrap();
        assert_eq!(context.world().generation(), before + 5);
    }

    #[test]
    fn build_rejects_duplicate_keys_without_committing() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let before = context.world().generation();
        let error = context
            .build(document, |ui| {
                ui.column(8.0, |ui| {
                    ui.child("save", Button::new("Save"));
                    ui.child("save", Button::new("Saved"));
                });
            })
            .unwrap_err();
        assert!(matches!(error, FrameworkError::DuplicateAssemblyKey { .. }));
        assert_eq!(context.world().generation(), before);
        assert!(
            context
                .world()
                .node(StableNodeId::new(1).unwrap())
                .is_none()
        );
    }

    #[test]
    fn build_detached_parks_roots_until_inserted() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (root, child) = context
            .build_detached(document, |ui| {
                let child = ui.leaf(Text::new("parked"));
                let root = ui.child("root", Stack::column(8.0));
                ui.nest(root, |ui| ui.adopt(child));
                (root, child)
            })
            .unwrap();
        assert_eq!(
            context.world().mount_state(root.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert_eq!(
            context.world().mount_state(child.stable_id()),
            Some(crate::MountState::Parked)
        );
        let host = context
            .create_component(document, Stack::column(0.0))
            .unwrap();
        context.append_child(host, root).unwrap();
        assert_eq!(
            context.world().mount_state(root.stable_id()),
            Some(crate::MountState::Mounted)
        );
        assert_eq!(
            context.world().mount_state(child.stable_id()),
            Some(crate::MountState::Mounted)
        );
        assert_eq!(
            context.world().node(root.stable_id()).unwrap().children,
            vec![child.stable_id()]
        );
    }

    #[test]
    fn build_child_keys_are_reused_by_mount() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let card = context.create_component(document, Card::new()).unwrap();
        let save = context
            .build_child(card, |ui| ui.child("save", Button::new("Save")))
            .unwrap();
        let save_id = save.stable_id();
        context
            .mount(card, |ui| {
                ui.child("save", Button::new("Saved"))?;
                Ok(())
            })
            .unwrap();
        assert_eq!(save.stable_id(), save_id);
        assert_eq!(
            context.read(save, |button| button.label.clone()).unwrap(),
            "Saved"
        );
    }

    #[test]
    fn sidebar_row_activate_queues_a_program_message() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let row = context
            .create_component(document, SidebarRow::new("舞台"))
            .unwrap();
        context
            .on(row, |_row, _event: &Activate, cx| {
                cx.dispatch_program("stage");
            })
            .unwrap();
        assert!(context.activate_sidebar_row(row).unwrap());
        let queued = context.take_program_messages();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].downcast_ref::<&str>().copied(), Some("stage"));
        assert!(context.take_program_messages().is_empty());
    }

    #[test]
    fn dispatch_program_keeps_the_latest_message_of_each_type() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let row = context
            .create_component(document, SidebarRow::new("舞台"))
            .unwrap();
        context
            .on(row, |_row, _event: &Activate, cx| {
                cx.dispatch_program("stage");
                cx.dispatch_program("functions");
                cx.dispatch_program(1_u8);
            })
            .unwrap();
        assert!(context.activate_sidebar_row(row).unwrap());
        assert!(context.has_program_messages());
        let queued = context.take_program_messages();
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].downcast_ref::<&str>().copied(), Some("functions"));
        assert_eq!(queued[1].downcast_ref::<u8>().copied(), Some(1));
        assert!(!context.has_program_messages());
    }

    #[test]
    fn plugin_register_activation_reaches_activate_node() {
        #[derive(Clone)]
        struct Ping;
        impl ComponentView for Ping {
            fn node_kind(&self) -> NodeKind {
                NodeKind::Element { tag: "ping".into() }
            }
            fn project(&self, _id: StableNodeId, _world: &UiWorld, _mutations: &mut MutationQueue) {
            }
        }
        impl crate::RegisterableComponent for Ping {
            const TYPE_ID: &'static str = "test.ping";
            const TAGS: &'static [&'static str] = &["ping"];
            fn from_semantic(_: &crate::SemanticSpec<'_>) -> Self {
                Ping
            }
        }
        fn activate_ping(
            context: &mut AppContext,
            entity: Entity<Ping>,
        ) -> Result<bool, FrameworkError> {
            context.update_component(entity, |_, cx| cx.emit(Activate))?;
            Ok(true)
        }
        struct PingExt;
        impl UiExtension for PingExt {
            fn name(&self) -> &'static str {
                "test.ping"
            }
            fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
                registrar.register_component::<Ping>()?;
                registrar.register_activation::<Ping>(activate_ping)
            }
        }

        let mut context = AppContext::new();
        context.install(&PingExt).unwrap();
        let ping = context
            .create_component(DocumentId::new(1).unwrap(), Ping)
            .unwrap();
        let hits = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&hits);
        context
            .on(ping, move |_, _: &Activate, _| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        assert!(context.activate_node(ping.stable_id()).unwrap());
        assert_eq!(*hits.lock().unwrap(), 1);
    }

    #[test]
    fn typed_view_update_delivers_closure_events_and_commits_one_batch() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let entity = context
            .create_view(document, NodeKind::Text, Counter { value: 0 })
            .unwrap();
        context
            .on(entity, |view, event: &Increment, cx| {
                view.value += event.0;
                let id = cx.entity().stable_id();
                cx.mutations().set_text(
                    id,
                    TextContent {
                        value: view.value.to_string(),
                    },
                );
                cx.emit(Cascade);
            })
            .unwrap();
        context
            .on(entity, |view, _event: &Cascade, _cx| view.value += 1)
            .unwrap();

        context
            .update(entity, |_view, cx| cx.emit(Increment(2)))
            .unwrap();
        assert_eq!(context.read(entity, |view| view.value).unwrap(), 3);
        assert_eq!(context.world().generation(), 2);
        assert!(
            context
                .world_mut()
                .take_system_work()
                .text
                .contains(&entity.stable_id())
        );
    }

    #[test]
    fn forged_view_type_is_an_error_and_does_not_remove_state() {
        let mut context = AppContext::new();
        let entity = context
            .create_view(
                DocumentId::new(1).unwrap(),
                NodeKind::Document,
                Counter { value: 7 },
            )
            .unwrap();
        let wrong = Entity::<String>::from_stable_id(entity.stable_id());
        assert_eq!(
            context.update(wrong, |_, _| ()),
            Err(FrameworkError::ViewType(entity.stable_id()))
        );
        assert_eq!(context.read(entity, |view| view.value).unwrap(), 7);
    }

    #[test]
    fn native_components_project_final_event_state_into_one_retained_tree() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let list = context
            .create_component(document, List::new().label("Actions"))
            .unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        let input = context
            .create_component(document, TextInput::new("你好ab").label("Name"))
            .unwrap();
        context.append_child(list, button).unwrap();
        context.append_child(list, input).unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let observed_change = Arc::new(Mutex::new(None));
        let observer = Arc::clone(&observed_change);
        context
            .on(input, move |_input, event: &TextChanged, _cx| {
                *observer.lock().unwrap() = Some(event.clone());
            })
            .unwrap();

        assert!(context.activate_button(button).unwrap());
        context
            .update_component(input, |input, _cx| {
                input.state.selection = TextSelection {
                    anchor: 0,
                    focus: "你".len(),
                };
            })
            .unwrap();
        assert!(context.replace_text_input_selection(input, "娜").unwrap());

        assert_eq!(context.world().text(button.stable_id()), Some("Running"));
        assert_eq!(
            context.world().text_input(input.stable_id()).unwrap().value,
            "娜好ab"
        );
        assert_eq!(context.world().text(input.stable_id()), Some("娜好ab"));
        assert_eq!(
            observed_change.lock().unwrap().as_ref().unwrap().selection,
            TextSelection::caret("娜".len())
        );
        assert_eq!(
            context.world().node(list.stable_id()).unwrap().children,
            vec![button.stable_id(), input.stable_id()]
        );
        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::List);
        assert_eq!(accessibility[1].role, crate::AccessibilityRole::Button);
        assert_eq!(accessibility[1].label.as_deref(), Some("Running"));
        assert_eq!(accessibility[2].role, crate::AccessibilityRole::TextInput);
        assert_eq!(accessibility[2].value.as_deref(), Some("娜好ab"));
        let extracted_button = context
            .world()
            .extract_document(document)
            .into_iter()
            .find(|node| node.id == button.stable_id())
            .unwrap();
        assert_eq!(extracted_button.text.unwrap().value, "Running");

        let generation = context.world().generation();
        context.update_component(button, |_button, _cx| {}).unwrap();
        assert_eq!(context.world().generation(), generation);
    }

    #[test]
    fn loading_button_owns_size_semantics_animation_and_activation_gate() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                Button::new("Deploy")
                    .kind(nana_ui_core::ButtonKind::Warning)
                    .size(nana_ui_core::ControlSize::Large)
                    .loading(true)
                    .invalid(true),
            )
            .unwrap();

        assert!(!context.activate_button(button).unwrap());
        assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));
        assert_eq!(
            context
                .world()
                .node_style(button.stable_id())
                .unwrap()
                .layout
                .min_height,
            Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::ControlSize::Large.height()
            ))
        );
        let accessibility = context.world().accessibility(button.stable_id()).unwrap();
        assert!(accessibility.disabled);
        assert!(accessibility.busy);
        assert!(accessibility.invalid);
        assert!(matches!(
            context.world().standard_visual(button.stable_id()),
            Some(StandardVisual::Button {
                kind: nana_ui_core::ButtonKind::Warning,
                size: nana_ui_core::ControlSize::Large,
                loading: true,
                invalid: true,
                ..
            })
        ));

        let frame = context.advance_animations(Duration::from_millis(400));
        assert_eq!(frame.component_updates, vec![button.stable_id()]);
        assert_eq!(frame.next_deadline, Some(Duration::from_millis(416)));
        assert!(matches!(
            context.world().standard_visual(button.stable_id()),
            Some(StandardVisual::Button {
                loading_phase,
                ..
            }) if (loading_phase - 0.5).abs() < f32::EPSILON
        ));

        context
            .update_component(button, |button, _cx| button.loading = false)
            .unwrap();
        assert_eq!(context.next_animation_deadline(), None);
        assert!(context.activate_button(button).unwrap());
    }

    #[test]
    fn text_input_owns_editability_privacy_size_and_busy_semantics() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                TextInput::new("secret")
                    .placeholder("Password")
                    .size(nana_ui_core::ControlSize::Large)
                    .read_only(true)
                    .secure(true)
                    .invalid(true),
            )
            .unwrap();

        assert!(context.focus_node(document, input.stable_id()).unwrap());
        assert!(!context.replace_text_input_selection(input, "x").unwrap());
        assert!(
            !context
                .set_ime_preedit(document, "输入".into(), None)
                .unwrap()
        );
        let node = context
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == input.stable_id())
            .unwrap();
        assert!(node.focused);
        assert!(!node.editable);
        assert!(node.invalid);
        assert_eq!(node.value, None);
        assert_eq!(
            context
                .world()
                .node_style(input.stable_id())
                .unwrap()
                .layout
                .min_height,
            Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::ControlSize::Large.height()
            ))
        );

        context
            .update_component(input, |input, _cx| {
                input.read_only = false;
                input.loading = true;
            })
            .unwrap();
        let state = context.world().accessibility(input.stable_id()).unwrap();
        assert!(state.disabled);
        assert!(state.busy);
        assert!(!state.editable);
        assert!(!context.replace_text_input_selection(input, "x").unwrap());

        context
            .update_component(input, |input, _cx| input.loading = false)
            .unwrap();
        assert_eq!(context.world().focused(document), Some(input.stable_id()));
        assert!(
            context
                .set_ime_preedit(document, "输入".into(), None)
                .unwrap()
        );
    }

    #[test]
    fn text_input_placeholder_uses_layout_color_and_opacity() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut field = TextInput::new("").placeholder("Hint");
        {
            let layout = Arc::make_mut(&mut field.style.layout);
            layout.placeholder_color = Some([1.0, 0.0, 0.0, 1.0]);
            layout.placeholder_opacity = Some(0.5);
        }
        let input = context.create_component(document, field).unwrap();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            input.stable_id(),
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 32.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
        context
            .world_mut()
            .resolve_styles(&[input.stable_id()])
            .unwrap();
        context
            .world_mut()
            .shape_text(&[input.stable_id()], &mut crate::MeasureTextShaper)
            .unwrap();

        match context.world().component_geometry(input.stable_id()) {
            Some(crate::ComponentGeometry::TextInput { text, .. }) => {
                assert_eq!(text.color, Some([1.0, 0.0, 0.0, 0.5]));
            }
            other => panic!("expected text input geometry, got {other:?}"),
        }
    }

    #[test]
    fn card_icon_button_and_list_item_keep_visual_and_semantic_content_distinct() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let card = context
            .create_component(document, Card::new().label("Build actions"))
            .unwrap();
        let icon = context
            .create_component(
                document,
                IconButton::new(nana_ui_core::Icon::Add, "Add source"),
            )
            .unwrap();
        let item = context
            .create_component(document, ListItem::new("Camera").selected(true))
            .unwrap();
        context.append_child(card, icon).unwrap();
        context.append_child(card, item).unwrap();
        context
            .on(icon, |button, _event: &Activate, _cx| {
                button.selected = true;
            })
            .unwrap();
        context
            .on(item, |item, _event: &Activate, _cx| {
                item.selected = false;
            })
            .unwrap();

        assert!(context.activate_icon_button(icon).unwrap());
        assert!(context.activate_list_item(item).unwrap());
        assert_eq!(context.world().text(icon.stable_id()), Some(""));
        assert_eq!(
            context.world().standard_visual(icon.stable_id()),
            Some(StandardVisual::Icon {
                icon: nana_ui_core::Icon::Add,
                size: nana_ui_core::ControlSize::Medium.icon_size(),
                tooltip: None,
            })
        );
        assert_eq!(context.world().text(item.stable_id()), Some("Camera"));

        let nodes = context.world().project_accessibility(document);
        let icon_node = nodes
            .iter()
            .find(|node| node.id == icon.stable_id())
            .unwrap();
        assert_eq!(icon_node.role, crate::AccessibilityRole::Button);
        assert_eq!(icon_node.label.as_deref(), Some("Add source"));
        assert_eq!(
            context
                .world()
                .node_style(icon.stable_id())
                .unwrap()
                .layout
                .min_width,
            Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.icon_button_size
            ))
        );
        let item_node = nodes
            .iter()
            .find(|node| node.id == item.stable_id())
            .unwrap();
        assert_eq!(item_node.role, crate::AccessibilityRole::ListItem);
        assert_eq!(item_node.selected, Some(false));
        assert_eq!(
            context
                .world()
                .node_style(card.stable_id())
                .unwrap()
                .layout
                .padding_top,
            Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_y + 24.0
            ))
        );
        assert_eq!(
            context.world().node(card.stable_id()).unwrap().children,
            vec![icon.stable_id(), item.stable_id()]
        );
    }

    #[test]
    fn text_area_reuses_utf8_editing_and_projects_multiline_semantics() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("第一行\nsecond").label("Notes"))
            .unwrap();
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection {
                    anchor: "第一".len(),
                    focus: "第一行\n".len(),
                };
            })
            .unwrap();
        let changes = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&changes);
        context
            .on(area, move |_area, event: &TextChanged, _cx| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();

        assert!(context.replace_text_area_selection(area, "段落\n").unwrap());
        assert_eq!(
            context.world().text_input(area.stable_id()).unwrap().value,
            "第一段落\nsecond"
        );
        assert_eq!(changes.lock().unwrap().len(), 1);
        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::TextInput);
        assert_eq!(accessibility[0].label.as_deref(), Some("Notes"));
        assert!(accessibility[0].multiline);
        assert_eq!(accessibility[0].value.as_deref(), Some("第一段落\nsecond"));
    }

    #[test]
    fn text_area_projects_visual_state_and_deletes_a_whole_grapheme() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let emoji = "👩‍💻";
        let area = context
            .create_component(
                document,
                TextArea::new(format!("{emoji}\n界"))
                    .placeholder("Write notes")
                    .invalid(true)
                    .height(144.0)
                    .scroll_offset(ScrollOffset { x: 4.0, y: 12.0 }),
            )
            .unwrap();

        assert!(matches!(
            context.world().standard_visual(area.stable_id()),
            Some(StandardVisual::TextInput {
                placeholder,
                secure: false,
                invalid: true,
                ..
            }) if placeholder.as_ref() == "Write notes"
        ));
        assert_eq!(
            context.world().node_style(area.stable_id()).unwrap().border,
            Some(nana_ui_core::SemanticColorRole::Danger)
        );
        assert_eq!(
            context.world().scroll_offset(area.stable_id()),
            Some(ScrollOffset { x: 4.0, y: 12.0 })
        );
        assert_eq!(
            context
                .world()
                .node_style(area.stable_id())
                .unwrap()
                .layout
                .height,
            Some(LengthSpec::Px(144.0))
        );

        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(emoji.len());
            })
            .unwrap();
        assert!(context.focus_node(document, area.stable_id()).unwrap());
        assert!(context.delete_focused_text_backward(document).unwrap());
        let state = context.world().text_input(area.stable_id()).unwrap();
        assert_eq!(state.value, "\n界");
        assert_eq!(state.selection, TextSelection::caret(0));

        assert!(
            context
                .set_ime_preedit(document, "输入".into(), None)
                .unwrap()
        );
        assert!(context.commit_ime(document, "输入").unwrap());
        let state = context.world().text_input(area.stable_id()).unwrap();
        assert_eq!(state.value, "输入\n界");
        assert_eq!(state.selection, TextSelection::caret("输入".len()));
        assert_eq!(context.world().ime(area.stable_id()), None);

        context
            .update_component(area, |area, _cx| area.disabled = true)
            .unwrap();
        assert_eq!(context.world().focused(document), None);
        assert_eq!(context.world().ime(area.stable_id()), None);
        let accessibility = context.world().accessibility(area.stable_id()).unwrap();
        assert!(accessibility.disabled);
        assert!(accessibility.invalid);
        assert!(accessibility.multiline);
    }

    #[test]
    fn native_table_projects_hierarchy_text_and_accessibility_roles() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let table = context
            .create_component(document, Table::new().label("Builds"))
            .unwrap();
        let row = context
            .create_component(document, TableRow::new().selected(true))
            .unwrap();
        let header = context
            .create_component(document, TableCell::new("Status").column_header(true))
            .unwrap();
        let cell = context
            .create_component(document, TableCell::new("Running").selected(true))
            .unwrap();
        context.append_child(table, row).unwrap();
        context.append_child(row, header).unwrap();
        context.append_child(row, cell).unwrap();
        let focused_events = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::clone(&focused_events);
        context
            .on(table, move |_table, event: &TableCellFocused, _cx| {
                events.lock().unwrap().push(event.clone());
            })
            .unwrap();

        assert!(
            context
                .navigate_table(table, TableNavigation::NextRow, 10)
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(header.stable_id()));
        assert!(
            context
                .navigate_table(table, TableNavigation::NextColumn, 10)
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(cell.stable_id()));
        assert_eq!(
            focused_events.lock().unwrap().last().unwrap(),
            &TableCellFocused {
                row: 0,
                column: 1,
                cell: cell.stable_id(),
            }
        );

        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::Table);
        assert_eq!(accessibility[0].label.as_deref(), Some("Builds"));
        assert_eq!(accessibility[1].role, crate::AccessibilityRole::Row);
        assert_eq!(accessibility[1].selected, Some(true));
        assert_eq!(
            accessibility[2].role,
            crate::AccessibilityRole::ColumnHeader
        );
        assert_eq!(accessibility[3].role, crate::AccessibilityRole::Cell);
        assert_eq!(accessibility[3].label.as_deref(), Some("Running"));
        assert_eq!(context.world().text(cell.stable_id()), Some("Running"));
    }

    #[test]
    fn native_toggle_and_slider_state_share_events_visuals_and_accessibility() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let checkbox = context
            .create_component(
                document,
                Checkbox::new("Notifications", false).invalid(true),
            )
            .unwrap();
        let switch = context
            .create_component(document, Switch::new("Auto build", true))
            .unwrap();
        let slider = context
            .create_component(
                document,
                RangeField::new(25.0, 0.0, 100.0, 1.0)
                    .unwrap()
                    .label("Volume"),
            )
            .unwrap();
        let toggles = Arc::new(Mutex::new(Vec::new()));
        let checkbox_events = Arc::clone(&toggles);
        context
            .on(checkbox, move |_checkbox, event: &ToggleChanged, _cx| {
                checkbox_events.lock().unwrap().push(event.checked);
            })
            .unwrap();
        let slider_values = Arc::new(Mutex::new(Vec::new()));
        let values = Arc::clone(&slider_values);
        context
            .on(slider, move |_slider, event: &RangeChanged, _cx| {
                values.lock().unwrap().push(event.value);
            })
            .unwrap();

        assert!(context.toggle_checkbox(checkbox).unwrap());
        assert!(context.toggle_switch(switch).unwrap());
        assert!(context.set_range_value(slider, 150.0).unwrap());
        assert!(!context.set_range_value(slider, 100.0).unwrap());
        assert_eq!(
            context.set_range_value(slider, f64::NAN),
            Err(FrameworkError::InvalidComponentValue(slider.stable_id()))
        );

        assert_eq!(*toggles.lock().unwrap(), vec![true]);
        assert_eq!(*slider_values.lock().unwrap(), vec![100.0]);
        assert_eq!(
            context.world().standard_visual(checkbox.stable_id()),
            Some(StandardVisual::Checkbox {
                checked: true,
                indeterminate: false,
                size: nana_ui_core::ControlSize::Medium,
            })
        );
        assert_eq!(
            context
                .world()
                .node_style(checkbox.stable_id())
                .unwrap()
                .layout
                .min_height,
            Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::ControlSize::Medium.height()
            ))
        );
        assert_eq!(
            context.world().standard_visual(switch.stable_id()),
            Some(StandardVisual::Switch {
                label: Arc::from("Auto build"),
                hint: None,
                checked: false,
                control_position: nana_ui_core::SwitchControlPosition::End,
                size: nana_ui_core::ControlSize::Medium,
                loading: false,
                loading_phase: 0.0,
                invalid: false,
            })
        );
        assert_eq!(
            context.world().standard_visual(slider.stable_id()),
            Some(StandardVisual::Range {
                label: Some(Arc::from("Volume")),
                value: Arc::from("100"),
                unit: None,
                size: nana_ui_core::ControlSize::Medium,
                ratio: 1.0,
                invalid: false,
            })
        );
        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::Checkbox);
        assert_eq!(accessibility[0].checked, Some(true));
        assert!(accessibility[0].invalid);
        assert_eq!(accessibility[1].role, crate::AccessibilityRole::Switch);
        assert_eq!(accessibility[1].checked, Some(false));
        assert_eq!(accessibility[2].role, crate::AccessibilityRole::Slider);
        assert_eq!(accessibility[2].value.as_deref(), Some("100"));

        let work = context.world_mut().take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let checkbox_paint = context
            .world()
            .extract_nodes(&[checkbox.stable_id()])
            .pop()
            .unwrap();
        assert_eq!(
            checkbox_paint.style.background,
            Some(nana_ui_core::SemanticPalette::dark().accent.as_rgba_array())
        );
        assert_eq!(
            checkbox_paint.style.border_color,
            Some(nana_ui_core::SemanticPalette::dark().danger.as_rgba_array())
        );
        context
            .world_mut()
            .set_pointer_hover(document, 1, Some(checkbox.stable_id()))
            .unwrap();
        let work = context.world_mut().take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let hovered_checked = context
            .world()
            .extract_nodes(&[checkbox.stable_id()])
            .pop()
            .unwrap();
        assert_ne!(
            hovered_checked.style.background, checkbox_paint.style.background,
            "a selected toggle must expose a distinct hover state"
        );
    }

    #[test]
    fn an_indeterminate_checkbox_reads_mixed_and_paints_as_engaged() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mixed = context
            .create_component(
                document,
                Checkbox::new("Notifications", false)
                    .indeterminate(true)
                    .size(nana_ui_core::ControlSize::Large),
            )
            .unwrap();
        assert_eq!(
            context.world().standard_visual(mixed.stable_id()),
            Some(StandardVisual::Checkbox {
                checked: false,
                indeterminate: true,
                size: nana_ui_core::ControlSize::Large,
            })
        );
        assert_eq!(
            context
                .world()
                .node_style(mixed.stable_id())
                .unwrap()
                .layout
                .min_height,
            Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::ControlSize::Large.height()
            ))
        );
        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].checked, Some(false));
        assert!(
            accessibility[0].mixed,
            "a mixed checkbox must not read as merely unchecked"
        );

        // Mixed shares the engaged surface with checked, so a parent checkbox
        // is not mistaken for an empty one.
        let work = context.world_mut().take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let paint = context
            .world()
            .extract_nodes(&[mixed.stable_id()])
            .pop()
            .unwrap();
        assert_eq!(
            paint.style.background,
            Some(nana_ui_core::SemanticPalette::dark().accent.as_rgba_array())
        );
    }

    #[test]
    fn a_divider_is_an_inert_hairline_with_separator_semantics() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let horizontal = context
            .create_component(document, crate::Divider::horizontal())
            .unwrap();
        let vertical = context
            .create_component(
                document,
                crate::Divider::vertical().thickness(2.0).inset(8.0),
            )
            .unwrap();

        let layout = |entity: StableNodeId| {
            context
                .world()
                .node_style(entity)
                .map(|style| Arc::clone(&style.layout))
                .unwrap()
        };
        let horizontal_layout = layout(horizontal.stable_id());
        assert_eq!(
            horizontal_layout.width,
            Some(nana_ui_core::LengthSpec::Fill)
        );
        assert_eq!(
            horizontal_layout.height,
            Some(nana_ui_core::LengthSpec::Px(1.0))
        );
        let vertical_layout = layout(vertical.stable_id());
        assert_eq!(
            vertical_layout.width,
            Some(nana_ui_core::LengthSpec::Px(2.0))
        );
        assert_eq!(vertical_layout.height, Some(nana_ui_core::LengthSpec::Fill));
        assert_eq!(
            vertical_layout.margin_top,
            Some(nana_ui_core::LengthSpec::Px(8.0))
        );

        for divider in [horizontal.stable_id(), vertical.stable_id()] {
            let interaction = context.world().interaction(divider).unwrap();
            assert!(!interaction.pointer_events);
            assert!(!interaction.focusable);
            assert_eq!(
                context.world().accessibility(divider).map(|s| s.role),
                Some(crate::AccessibilityRole::Separator)
            );
        }
        assert_eq!(
            context
                .world()
                .accessibility(vertical.stable_id())
                .and_then(|state| state.orientation),
            Some(crate::SelectionOrientation::Vertical)
        );
    }

    #[test]
    fn a_number_input_steps_snaps_and_settles_its_draft() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                crate::NumberInput::new(1.0)
                    .range(0.0, 2.0)
                    .step(0.5)
                    .precision(1)
                    .label("Scale"),
            )
            .unwrap();
        let values = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&values);
        context
            .on(input, move |_input, event: &crate::NumberChanged, _cx| {
                observed.lock().unwrap().push(event.value);
            })
            .unwrap();

        assert_eq!(
            context.world().text_input(input.stable_id()).unwrap().value,
            "1.0"
        );
        assert!(context.step_number_input(input, 1).unwrap());
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 1.5);
        assert!(context.step_number_input(input, 2).unwrap());
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 2.0);
        // Already at the maximum: no event, no phantom change.
        assert!(!context.step_number_input(input, 1).unwrap());

        // A draft is only adopted on commit, and it snaps to the step grid.
        context
            .update_component(input, |input, _| {
                input.state.replace_value("0.7".to_owned());
            })
            .unwrap();
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 2.0);
        assert!(context.commit_number_input(input).unwrap());
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 0.5);

        // Nonsense restores the committed value instead of inventing one.
        context
            .update_component(input, |input, _| {
                input.state.replace_value("banana".to_owned());
            })
            .unwrap();
        assert!(context.commit_number_input(input).unwrap());
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 0.5);
        assert_eq!(
            context.world().text_input(input.stable_id()).unwrap().value,
            "0.5"
        );

        assert_eq!(*values.lock().unwrap(), vec![1.5, 2.0, 0.5]);
        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::TextInput);
        assert_eq!(accessibility[0].numeric_value, Some(0.5));
        assert_eq!(accessibility[0].numeric_minimum, Some(0.0));
        assert_eq!(accessibility[0].numeric_maximum, Some(2.0));
        assert_eq!(accessibility[0].numeric_step, Some(0.5));
    }

    #[test]
    fn a_disabled_number_input_refuses_both_steppers() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                crate::NumberInput::new(4.0).range(0.0, 10.0).disabled(true),
            )
            .unwrap();
        assert!(!context.step_number_input(input, 1).unwrap());
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 4.0);
        let read_only = context
            .create_component(
                document,
                crate::NumberInput::new(4.0)
                    .range(0.0, 10.0)
                    .read_only(true),
            )
            .unwrap();
        assert!(!context.step_number_input(read_only, 1).unwrap());
    }

    #[test]
    fn pressing_the_spinner_steps_and_pressing_the_text_does_not() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                crate::NumberInput::new(4.0).range(0.0, 10.0).step(1.0),
            )
            .unwrap();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            input.stable_id(),
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 32.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
        let work = context.world_mut().take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        context
            .world_mut()
            .shape_text(&work.text, &mut crate::MeasureTextShaper)
            .unwrap();

        let steppers = match context.world().component_geometry(input.stable_id()) {
            Some(crate::ComponentGeometry::TextInput {
                steppers: Some(steppers),
                ..
            }) => steppers,
            other => panic!("expected spinner geometry, got {other:?}"),
        };
        let point = |bounds: crate::LayoutBox| {
            (
                bounds.x + bounds.width / 2.0,
                bounds.y + bounds.height / 2.0,
            )
        };
        let (up_x, up_y) = point(steppers.increment);
        let (down_x, down_y) = point(steppers.decrement);
        assert_eq!(
            context.number_stepper_at(input.stable_id(), up_x, up_y),
            Some(1)
        );
        assert_eq!(
            context.number_stepper_at(input.stable_id(), down_x, down_y),
            Some(-1)
        );
        // The editable text area is not a stepper, so caret placement still wins.
        assert_eq!(
            context.number_stepper_at(input.stable_id(), 8.0, 16.0),
            None
        );

        assert!(
            context
                .press_number_stepper(input.stable_id(), up_x, up_y)
                .unwrap()
        );
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 5.0);
        assert!(
            context
                .press_number_stepper(input.stable_id(), down_x, down_y)
                .unwrap()
        );
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 4.0);
        assert!(
            !context
                .press_number_stepper(input.stable_id(), 8.0, 16.0)
                .unwrap()
        );
    }

    #[test]
    fn moving_focus_away_settles_a_pending_numeric_draft() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                crate::NumberInput::new(1.0).range(0.0, 9.0).step(1.0),
            )
            .unwrap();
        let elsewhere = context
            .create_component(document, Button::new("Done"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());
        context
            .update_component(input, |input, _| {
                input.state.replace_value("7".to_owned());
            })
            .unwrap();
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 1.0);

        assert!(context.focus_node(document, elsewhere.stable_id()).unwrap());
        assert_eq!(context.read(input, crate::NumberInput::value).unwrap(), 7.0);
    }

    #[test]
    fn range_accessibility_set_value_uses_quantized_typed_action() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let range = context
            .create_component(
                document,
                RangeField::new(0.25, 0.0, 1.0, 0.25)
                    .unwrap()
                    .label("Opacity")
                    .unit("%"),
            )
            .unwrap();
        let values = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&values);
        context
            .on(range, move |_range, event: &RangeChanged, _cx| {
                observed.lock().unwrap().push(event.value);
            })
            .unwrap();

        assert!(
            context
                .apply_accessibility_action(
                    document,
                    AccessibilityActionRequest {
                        target: range.stable_id(),
                        action: AccessibilityAction::SetValue("0.62".into()),
                    },
                )
                .unwrap()
        );
        assert_eq!(*values.lock().unwrap(), vec![0.5]);
        let node = context.world().project_accessibility(document).remove(0);
        assert_eq!(node.numeric_minimum, Some(0.0));
        assert_eq!(node.numeric_maximum, Some(1.0));
        assert_eq!(node.numeric_step, Some(0.25));
        assert_eq!(node.numeric_value, Some(0.5));
    }

    #[test]
    fn failed_component_projection_keeps_typed_state_and_world_unchanged() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let slider = context
            .create_component(document, RangeField::new(25.0, 0.0, 100.0, 1.0).unwrap())
            .unwrap();
        let generation = context.world().generation();
        let visual = context.world().standard_visual(slider.stable_id());

        assert!(
            context
                .update_component(slider, |slider, _cx| slider.value = f64::NAN)
                .is_err()
        );
        assert_eq!(context.read(slider, |slider| slider.value).unwrap(), 25.0);
        assert_eq!(context.world().generation(), generation);
        assert_eq!(context.world().standard_visual(slider.stable_id()), visual);
    }

    #[test]
    fn overlay_host_switches_exclusive_visibility_and_restores_focus() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let base = context
            .create_component(document, Button::new("Open"))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(
                document,
                Dialog::new("Settings").close_policy(nana_ui_core::DialogClosePolicy {
                    close_on_outside: false,
                    ..nana_ui_core::DialogClosePolicy::default()
                }),
            )
            .unwrap();
        let menu = context
            .create_component(document, crate::ActionMenu::new().open(true))
            .unwrap();
        let menu_item = context
            .create_component(document, crate::ActionMenuItem::new("Build"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context.append_child(host, menu).unwrap();
        context.append_child(menu, menu_item).unwrap();
        let mut focus = MutationQueue::new();
        focus.request_focus(document, Some(base.stable_id()));
        context.world_mut().commit(focus).unwrap();
        let changes = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&changes);
        context
            .on(host, move |_host, event: &OverlayChanged, _cx| {
                observed.lock().unwrap().push(event.active);
            })
            .unwrap();
        let activations = Arc::new(Mutex::new(0));
        let observed_activations = Arc::clone(&activations);
        context
            .on(menu_item, move |_item, _event: &Activate, _cx| {
                *observed_activations.lock().unwrap() += 1;
            })
            .unwrap();

        let initial_work = context.world_mut().take_system_work();
        context
            .world_mut()
            .resolve_styles(&initial_work.style)
            .unwrap();
        let initial = context.world().extract_document(document);
        assert!(!initial.iter().any(|node| node.id == dialog.stable_id()));
        assert!(!initial.iter().any(|node| node.id == menu.stable_id()));

        assert!(context.activate_overlay(host, dialog).unwrap());
        let dialog_work = context.world_mut().take_system_work();
        context
            .world_mut()
            .resolve_styles(&dialog_work.style)
            .unwrap();
        assert_eq!(context.world().focused(document), Some(dialog.stable_id()));
        let generation = context.world().generation();
        assert_eq!(
            context.append_child(menu, dialog),
            Err(FrameworkError::World(
                crate::UiWorldError::InvalidOverlayHost(host.stable_id())
            ))
        );
        assert_eq!(context.world().generation(), generation);
        assert_eq!(
            context.world().node(dialog.stable_id()).unwrap().parent,
            Some(host.stable_id())
        );
        assert!(
            context
                .world()
                .project_accessibility(document)
                .iter()
                .any(|node| node.id == dialog.stable_id() && node.modal)
        );
        let mut escape_modal = MutationQueue::new();
        escape_modal.request_focus(document, Some(base.stable_id()));
        assert_eq!(
            context.world_mut().commit(escape_modal),
            Err(crate::UiWorldError::NotFocusable(base.stable_id()))
        );
        assert_eq!(context.world().focused(document), Some(dialog.stable_id()));
        assert!(
            !context
                .dismiss_dialog(host, nana_ui_core::DialogCloseTrigger::Outside)
                .unwrap()
        );
        let mut capture = MutationQueue::new();
        capture.capture_pointer(7, dialog.stable_id());
        context.world_mut().commit(capture).unwrap();
        assert!(context.activate_overlay(host, menu).unwrap());
        let menu_work = context.world_mut().take_system_work();
        assert!(menu_work.accessibility.contains(&host.stable_id()));
        context
            .world_mut()
            .resolve_styles(&menu_work.style)
            .unwrap();
        let visible = context.world().extract_document(document);
        assert!(!visible.iter().any(|node| node.id == dialog.stable_id()));
        assert!(visible.iter().any(|node| node.id == menu.stable_id()));
        assert_eq!(
            context.world().focused(document),
            Some(menu_item.stable_id())
        );
        assert!(context.activate_action_menu_item(menu_item).unwrap());
        assert_eq!(*activations.lock().unwrap(), 1);
        assert_eq!(context.world().pointer_capture(document, 7), None);
        assert!(
            context
                .world_mut()
                .take_pointer_capture_changes()
                .iter()
                .any(|change| change.pointer_id == 7 && !change.captured)
        );

        assert!(context.dismiss_overlay(host).unwrap());
        let dismissed_work = context.world_mut().take_system_work();
        assert!(dismissed_work.accessibility.contains(&host.stable_id()));
        context
            .world_mut()
            .resolve_styles(&dismissed_work.style)
            .unwrap();
        assert_eq!(context.world().focused(document), Some(base.stable_id()));
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            None
        );
        assert_eq!(
            changes.lock().unwrap().as_slice(),
            [Some(dialog.stable_id()), Some(menu.stable_id()), None]
        );
    }

    #[test]
    fn destroying_the_active_overlay_clears_authority_and_restores_focus() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let base = context
            .create_component(document, Button::new("Open"))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Temporary"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        let mut focus = MutationQueue::new();
        focus.request_focus(document, Some(base.stable_id()));
        context.world_mut().commit(focus).unwrap();
        context.activate_overlay(host, dialog).unwrap();

        context.remove_view(dialog).unwrap();

        assert_eq!(context.world().focused(document), Some(base.stable_id()));
        assert_eq!(
            context.world().overlay_host(host.stable_id()),
            Some(crate::OverlayHostState::default())
        );
        assert!(!context.dismiss_overlay(host).unwrap());
    }

    #[test]
    fn segmented_options_reconcile_atomically_and_roving_selection_skips_disabled() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new().label("Preview mode"))
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let disabled = context
            .create_detached_component(document, SegmentedOption::new("Split").disabled(true))
            .unwrap();
        let last = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        assert!(
            context
                .set_segmented_options(control, vec![first, disabled, last], Some(first))
                .unwrap()
        );

        let observed = Arc::new(Mutex::new(Vec::new()));
        let selected = Arc::clone(&observed);
        context
            .on(
                control,
                move |_control, event: &SegmentedSelectionRequested, _cx| {
                    selected.lock().unwrap().push(event.option);
                },
            )
            .unwrap();
        context.focus_node(document, first.stable_id()).unwrap();
        assert!(
            context
                .navigate_focused_segmented(document, RovingFocusIntent::Next)
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(last.stable_id()));
        assert_eq!(
            context.read(control, |control| control.selected).unwrap(),
            Some(first.stable_id())
        );
        assert!(context.read(first, |option| option.selected).unwrap());
        assert!(!context.read(last, |option| option.selected).unwrap());
        assert_eq!(&*observed.lock().unwrap(), &[last.stable_id()]);
        assert!(context.activate_node(last.stable_id()).unwrap());
        assert_eq!(
            &*observed.lock().unwrap(),
            &[last.stable_id(), last.stable_id()]
        );
        assert!(!context.activate_node(disabled.stable_id()).unwrap());
        let generation = context.world().generation();
        assert!(
            context
                .set_segmented_selection(control, Some(last))
                .unwrap()
        );
        assert_eq!(context.world().generation(), generation + 1);
        assert!(!context.read(first, |option| option.selected).unwrap());
        assert!(context.read(last, |option| option.selected).unwrap());
        assert!(
            context
                .apply_accessibility_action(
                    document,
                    AccessibilityActionRequest {
                        target: last.stable_id(),
                        action: AccessibilityAction::Click,
                    },
                )
                .unwrap()
        );
        assert!(context.read(last, |option| option.selected).unwrap());
        assert!(
            context
                .navigate_focused_segmented(document, RovingFocusIntent::Next)
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert_eq!(
            context.read(control, |control| control.selected).unwrap(),
            Some(last.stable_id())
        );

        let generation = context.world().generation();
        assert_eq!(
            context.set_segmented_options(control, vec![first, first], Some(first)),
            Err(FrameworkError::InvalidComponentValue(control.stable_id()))
        );
        assert_eq!(context.world().generation(), generation);

        assert!(
            context
                .set_segmented_options(control, vec![first, last], Some(first))
                .unwrap()
        );
        assert_eq!(
            context.world().mount_state(disabled.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert!(
            !context
                .world()
                .project_accessibility(document)
                .iter()
                .any(|node| node.id == disabled.stable_id())
        );
        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::RadioGroup);
        assert_eq!(accessibility[1].role, crate::AccessibilityRole::Radio);
        assert_eq!(accessibility[1].checked, Some(true));
        assert_eq!(accessibility[2].checked, Some(false));
        assert_eq!(context.next_animation_deadline(), None);
    }

    #[test]
    fn updating_a_filled_tab_option_keeps_the_control_surface() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(
                document,
                Tabs::new("code")
                    .size(nana_ui_core::ControlSize::Small)
                    .fill(true)
                    .options([
                        TabOption::new("code", "Code"),
                        TabOption::new("preview", "Preview"),
                    ]),
            )
            .unwrap();
        let first = Entity::<SegmentedOption>::from_stable_id(
            context
                .read(control, |tabs| tabs.option_nodes()[0].1)
                .unwrap(),
        );
        context
            .update_component(first, |option, _| {
                *option = SegmentedOption::new("Code")
                    .size(nana_ui_core::ControlSize::Small)
                    .with_selected(true);
            })
            .unwrap();
        context
            .update_component(control, |tabs, _| {
                tabs.fill = true;
            })
            .unwrap();
        assert!(context.read(first, |option| option.fill).unwrap());
        assert_eq!(
            context
                .read(first, |option| option.style.layout.width)
                .unwrap(),
            Some(LengthSpec::Fill)
        );
        assert_eq!(
            context.read(first, |option| option.node_kind()).unwrap(),
            NodeKind::Element { tag: "tab".into() }
        );
    }

    #[test]
    fn segmented_size_disabled_and_sequential_focus_share_one_authority() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let before = context
            .create_component(document, Button::new("Before"))
            .unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        let after = context
            .create_component(document, Button::new("After"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();

        let generation = context.world().generation();
        assert!(
            context
                .set_segmented_size(control, nana_ui_core::ControlSize::Large)
                .unwrap()
        );
        assert_eq!(context.world().generation(), generation + 1);
        assert_eq!(
            context.read(control, |control| control.size).unwrap(),
            nana_ui_core::ControlSize::Large
        );
        assert_eq!(
            context.read(first, |option| option.size).unwrap(),
            nana_ui_core::ControlSize::Large
        );
        assert_eq!(
            context.read(second, |option| option.size).unwrap(),
            nana_ui_core::ControlSize::Large
        );
        let radius = context.world().theme_metrics().radius_md;
        assert_eq!(
            context
                .world()
                .node_style(control.stable_id())
                .unwrap()
                .layout
                .border_radius,
            Some(radius)
        );
        assert_eq!(
            context
                .world()
                .node_style(first.stable_id())
                .unwrap()
                .layout
                .border_radius,
            Some((radius - 3.0).max(0.0))
        );

        context.focus_node(document, before.stable_id()).unwrap();
        assert!(context.navigate_sequential_focus(document, false).unwrap());
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert!(context.navigate_sequential_focus(document, false).unwrap());
        assert_eq!(context.world().focused(document), Some(after.stable_id()));
        assert!(
            context
                .apply_accessibility_action(
                    document,
                    AccessibilityActionRequest {
                        target: second.stable_id(),
                        action: AccessibilityAction::Focus,
                    },
                )
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(
            context
                .read(control, |control| control.focus_target)
                .unwrap(),
            Some(second.stable_id())
        );
        assert!(
            context
                .set_segmented_selection(control, Some(second))
                .unwrap()
        );
        assert!(
            context
                .set_segmented_selection(control, Some(first))
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(
            context
                .read(control, |control| control.focus_target)
                .unwrap(),
            Some(second.stable_id())
        );
        assert!(context.navigate_sequential_focus(document, false).unwrap());
        assert_eq!(context.world().focused(document), Some(after.stable_id()));

        context.focus_node(document, first.stable_id()).unwrap();
        assert!(
            context
                .set_segmented_option_disabled(control, first, true)
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(
            context
                .read(control, |control| control.focus_target)
                .unwrap(),
            Some(second.stable_id())
        );
        assert_eq!(
            context.read(control, |control| control.selected).unwrap(),
            Some(first.stable_id())
        );
        assert!(context.read(first, |option| option.selected).unwrap());
        assert!(context.read(first, |option| option.disabled).unwrap());
        assert_eq!(
            context
                .world()
                .project_accessibility(document)
                .into_iter()
                .find(|node| node.id == first.stable_id())
                .unwrap()
                .checked,
            Some(true)
        );
        assert!(
            context
                .set_segmented_option_disabled(control, first, false)
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(
            context
                .read(control, |control| control.focus_target)
                .unwrap(),
            Some(second.stable_id())
        );
    }

    #[test]
    fn segmented_intrinsic_width_is_stable_across_viewports_sizes_icons_and_empty_groups() {
        struct FixedShaper;
        impl crate::TextShaper for FixedShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                text: &TextContent,
                _style: &crate::ComputedStyle,
                _constraints: crate::TextShapeConstraints,
            ) -> crate::TextMetrics {
                crate::TextMetrics {
                    width: text.value.chars().count() as f32 * 7.0,
                    height: 16.0,
                    ascent: None,
                }
            }
        }

        for (index, size) in [
            nana_ui_core::ControlSize::Small,
            nana_ui_core::ControlSize::Medium,
            nana_ui_core::ControlSize::Large,
        ]
        .into_iter()
        .enumerate()
        {
            let mut context = AppContext::new();
            let document = DocumentId::new(index as u64 + 1).unwrap();
            let control = context
                .create_component(document, SegmentedControl::new().size(size))
                .unwrap();
            let icon = context
                .create_detached_component(
                    document,
                    SegmentedOption::new("Code").icon(nana_ui_core::Icon::File),
                )
                .unwrap();
            let plain = context
                .create_detached_component(document, SegmentedOption::new("Code"))
                .unwrap();
            context
                .set_segmented_options(control, vec![icon, plain], Some(icon))
                .unwrap();
            let mut shaper = FixedShaper;
            while context
                .shape_text_for_layout(document, &mut shaper)
                .unwrap()
            {}
            context
                .layout_document(document, crate::LayoutViewport::new(320.0, 100.0))
                .unwrap();
            let narrow = context.world().layout_box(control.stable_id()).unwrap();
            let icon_bounds = context.world().layout_box(icon.stable_id()).unwrap();
            let plain_bounds = context.world().layout_box(plain.stable_id()).unwrap();
            assert_eq!(narrow.height, size.height());
            assert!(icon_bounds.width > plain_bounds.width);
            assert!((narrow.width - (icon_bounds.width + plain_bounds.width + 8.0)).abs() < 0.01);

            context
                .layout_document(document, crate::LayoutViewport::new(640.0, 100.0))
                .unwrap();
            assert_eq!(
                context.world().layout_box(control.stable_id()),
                Some(narrow)
            );
        }

        let mut context = AppContext::new();
        let document = DocumentId::new(9).unwrap();
        let empty = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(640.0, 100.0))
            .unwrap();
        let empty = context.world().layout_box(empty.stable_id()).unwrap();
        assert_eq!(empty.width, 6.0);
        assert_eq!(empty.height, nana_ui_core::ControlSize::Medium.height());
    }

    #[test]
    fn segmented_request_focus_and_event_roll_back_together_when_focus_is_blocked() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Settings"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context.activate_overlay(host, dialog).unwrap();
        let generation = context.world().generation();

        assert!(matches!(
            context.request_segmented_selection(control, second),
            Err(FrameworkError::World(crate::UiWorldError::NotFocusable(id)))
                if id == second.stable_id()
        ));
        assert_eq!(context.world().generation(), generation);
        assert!(context.read(first, |option| option.selected).unwrap());
        assert!(!context.read(second, |option| option.selected).unwrap());
        assert_eq!(
            context
                .read(control, |control| control.focus_target)
                .unwrap(),
            Some(first.stable_id())
        );
        assert_eq!(
            context.read(control, |control| control.selected).unwrap(),
            Some(first.stable_id())
        );
    }

    #[test]
    fn segmented_request_rolls_back_focus_when_an_event_handler_mutation_is_invalid() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let foreign_document = DocumentId::new(2).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        let foreign = context
            .create_component(foreign_document, Button::new("Foreign"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        context.focus_node(document, first.stable_id()).unwrap();
        context
            .on(
                control,
                move |_control, _event: &SegmentedSelectionRequested, cx| {
                    cx.mutations()
                        .insert(foreign.stable_id(), second.stable_id(), None);
                },
            )
            .unwrap();
        let generation = context.world().generation();

        assert!(
            context
                .request_segmented_selection(control, second)
                .is_err()
        );
        assert_eq!(context.world().generation(), generation);
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert_eq!(
            context
                .read(control, SegmentedControl::focus_target)
                .unwrap(),
            Some(first.stable_id())
        );
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(first.stable_id())
        );
        assert!(context.read(first, SegmentedOption::selected).unwrap());
        assert!(!context.read(second, SegmentedOption::selected).unwrap());
    }

    #[test]
    fn overlay_tab_trap_reuses_segmented_sequential_focus_authority() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(
                document,
                Dialog::new("Settings").initial_focus(crate::ModalInitialFocus::Surface),
            )
            .unwrap();
        let control = context
            .create_detached_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        let action = context
            .create_detached_component(document, Button::new("Save"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context
            .set_modal_slots(
                dialog,
                ModalSlots {
                    body: Some(control.stable_id()),
                    actions: vec![action.stable_id()],
                    ..Default::default()
                },
            )
            .unwrap();
        context.activate_overlay(host, dialog).unwrap();
        assert_eq!(context.world().focused(document), Some(dialog.stable_id()));
        assert!(
            context
                .route_overlay_key(document, OverlayKey::Tab { reverse: false })
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert!(
            context
                .route_overlay_key(document, OverlayKey::Tab { reverse: false })
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(action.stable_id()));
        assert!(
            context
                .route_overlay_key(document, OverlayKey::Tab { reverse: false })
                .unwrap()
        );
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert!(context.focus_node(document, second.stable_id()).unwrap());
    }

    #[test]
    fn native_scroll_view_projects_axes_and_typed_runtime_offset() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroll = context
            .create_component(
                document,
                ScrollView::new(ScrollAxes::Vertical).label("Builds"),
            )
            .unwrap();
        let changes = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&changes);
        context
            .on(scroll, move |_scroll, event: &ScrollChanged, _cx| {
                observed.lock().unwrap().push(event.offset);
            })
            .unwrap();
        context.world_mut().take_system_work();

        assert!(
            context
                .scroll_to(scroll, ScrollOffset { x: 40.0, y: 120.0 })
                .unwrap()
        );
        assert_eq!(
            context.world().scroll_offset(scroll.stable_id()),
            Some(ScrollOffset { x: 0.0, y: 120.0 })
        );
        assert_eq!(
            *changes.lock().unwrap(),
            vec![ScrollOffset { x: 0.0, y: 120.0 }]
        );
        assert_eq!(
            context
                .world()
                .node_style(scroll.stable_id())
                .unwrap()
                .layout
                .overflow_y,
            nana_ui_core::OverflowSpec::Scroll
        );
        let work = context.world_mut().take_system_work();
        assert_eq!(work.input_hit_test, vec![scroll.stable_id()]);
        assert_eq!(work.render_extraction, vec![scroll.stable_id()]);
        assert!(work.layout.is_empty());
        assert!(
            context
                .set_scroll_metrics(
                    scroll,
                    ScrollMetrics {
                        viewport_width: 100.0,
                        viewport_height: 100.0,
                        content_width: 100.0,
                        content_height: 250.0,
                    },
                )
                .unwrap()
        );
        assert!(
            context
                .scroll_by(scroll, ScrollOffset { x: 0.0, y: 80.0 })
                .unwrap()
        );
        assert_eq!(
            context.world().scroll_offset(scroll.stable_id()).unwrap().y,
            150.0
        );
        assert!(
            context
                .set_scroll_metrics(
                    scroll,
                    ScrollMetrics {
                        viewport_width: 100.0,
                        viewport_height: 100.0,
                        content_width: 100.0,
                        content_height: 130.0,
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context.world().scroll_offset(scroll.stable_id()).unwrap().y,
            30.0
        );
        assert_eq!(
            changes.lock().unwrap().as_slice(),
            [
                ScrollOffset { x: 0.0, y: 120.0 },
                ScrollOffset { x: 0.0, y: 150.0 },
                ScrollOffset { x: 0.0, y: 30.0 },
            ]
        );
        assert!(
            !context
                .scroll_to(scroll, ScrollOffset { x: 0.0, y: 30.0 })
                .unwrap()
        );
        assert_eq!(
            context.scroll_to(
                scroll,
                ScrollOffset {
                    x: 0.0,
                    y: f32::NAN
                }
            ),
            Err(FrameworkError::InvalidComponentValue(scroll.stable_id()))
        );
    }

    #[test]
    fn layout_publishes_scroll_metrics_and_clamps_wheel_offset() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut viewport = NodeStyle::default();
        {
            let layout = std::sync::Arc::make_mut(&mut viewport.layout);
            layout.width = Some(LengthSpec::Px(200.0));
            layout.height = Some(LengthSpec::Px(120.0));
        }
        let scroll = context
            .create_component(
                document,
                ScrollView::new(ScrollAxes::Vertical).style(viewport),
            )
            .unwrap();
        for index in 0..5 {
            let mut row = NodeStyle::default();
            {
                let layout = std::sync::Arc::make_mut(&mut row.layout);
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Px(40.0));
            }
            let row = context
                .create_component(document, Text::new(format!("Row {index}")).style(row))
                .unwrap();
            context.append_child(scroll, row).unwrap();
        }
        context
            .layout_document(document, crate::LayoutViewport::new(200.0, 120.0))
            .unwrap();
        let metrics = context
            .world()
            .scroll_metrics(scroll.stable_id())
            .expect("layout publishes scroll metrics");
        assert!(
            metrics.content_height > metrics.viewport_height,
            "content {metrics:?} should overflow the viewport"
        );
        let max_y = (metrics.content_height - metrics.viewport_height).max(0.0);
        assert!(
            context
                .scroll_by(
                    scroll,
                    ScrollOffset {
                        x: 0.0,
                        y: max_y + 400.0
                    }
                )
                .unwrap()
        );
        let offset = context.world().scroll_offset(scroll.stable_id()).unwrap();
        assert!(
            (offset.y - max_y).abs() < 0.01,
            "wheel offset {} should clamp to {max_y}",
            offset.y
        );
        assert_eq!(offset.x, 0.0);
    }

    /// 200x120 scrollport holding 200px of rows, so the vertical axis overflows
    /// by 80px.
    fn overflowing_scroll_view(
        context: &mut AppContext,
        document: DocumentId,
        visibility: nana_ui_core::ScrollbarVisibility,
    ) -> Entity<ScrollView> {
        let mut viewport = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut viewport.layout);
            layout.width = Some(LengthSpec::Px(200.0));
            layout.height = Some(LengthSpec::Px(120.0));
        }
        let scroll = context
            .create_component(
                document,
                ScrollView::new(ScrollAxes::Vertical)
                    .scrollbars(visibility)
                    .style(viewport),
            )
            .unwrap();
        for index in 0..5 {
            let mut row = NodeStyle::default();
            {
                let layout = Arc::make_mut(&mut row.layout);
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Px(40.0));
            }
            let row = context
                .create_component(document, Text::new(format!("Row {index}")).style(row))
                .unwrap();
            context.append_child(scroll, row).unwrap();
        }
        context
            .layout_document(document, crate::LayoutViewport::new(200.0, 120.0))
            .unwrap();
        scroll
    }

    fn vertical_bar(
        context: &AppContext,
        scroll: Entity<ScrollView>,
    ) -> Option<crate::ScrollbarBar> {
        match context.world().component_geometry(scroll.stable_id()) {
            Some(crate::ComponentGeometry::Scrollbar { vertical, .. }) => vertical,
            _ => None,
        }
    }

    #[test]
    fn auto_hiding_scrollbars_appear_on_hover_and_follow_the_scroll_offset() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroll = overflowing_scroll_view(
            &mut context,
            document,
            nana_ui_core::ScrollbarVisibility::AutoHide,
        );
        assert!(
            vertical_bar(&context, scroll).is_none(),
            "an idle auto-hiding container draws no bar"
        );

        context
            .set_pointer_hover_at(
                document,
                1,
                Some(scroll.stable_id()),
                std::time::Duration::ZERO,
            )
            .unwrap();
        let bar = vertical_bar(&context, scroll).expect("hover reveals the bar");
        // 120 of 200 content is visible, so the thumb takes 60% of the track.
        assert!(
            (bar.thumb.height - 72.0).abs() < 0.01,
            "thumb {:?}",
            bar.thumb
        );
        assert!(
            (bar.thumb.y - bar.track.y).abs() < 0.01,
            "thumb starts at the top"
        );
        assert!((bar.max_offset - 80.0).abs() < 0.01);
        assert_eq!(bar.track_background, None, "auto-hide draws no track");

        assert!(
            context
                .scroll_to(scroll, ScrollOffset { x: 0.0, y: 80.0 })
                .unwrap()
        );
        let bar = vertical_bar(&context, scroll).expect("still hovered");
        assert!(
            (bar.thumb.y + bar.thumb.height - (bar.track.y + bar.track.height)).abs() < 0.01,
            "a maxed offset pins the thumb to the track end: {:?}",
            bar.thumb
        );

        context
            .set_pointer_hover_at(document, 1, None, std::time::Duration::ZERO)
            .unwrap();
        assert!(
            vertical_bar(&context, scroll).is_none(),
            "leaving the container hides the bar again"
        );
    }

    #[test]
    fn resident_scrollbars_draw_a_track_without_hover() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroll = overflowing_scroll_view(
            &mut context,
            document,
            nana_ui_core::ScrollbarVisibility::Always,
        );
        let bar = vertical_bar(&context, scroll).expect("resident bars need no hover");
        assert!(bar.track_background.is_some());
        assert!((bar.track.width - nana_ui_core::SCROLLBAR_METRICS.thickness).abs() < 0.01);
        assert!(
            (bar.track.x + bar.track.width - 200.0).abs() < 0.01,
            "bar hugs the right edge"
        );
    }

    #[test]
    fn hidden_scrollbars_leave_wheel_scrolling_alone() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroll = overflowing_scroll_view(
            &mut context,
            document,
            nana_ui_core::ScrollbarVisibility::Hidden,
        );
        context
            .set_pointer_hover_at(
                document,
                1,
                Some(scroll.stable_id()),
                std::time::Duration::ZERO,
            )
            .unwrap();
        assert!(vertical_bar(&context, scroll).is_none());
        assert!(
            context
                .scroll_by(scroll, ScrollOffset { x: 0.0, y: 40.0 })
                .unwrap()
        );
        assert_eq!(
            context.world().scroll_offset(scroll.stable_id()),
            Some(ScrollOffset { x: 0.0, y: 40.0 })
        );
    }

    #[test]
    fn dragging_the_thumb_moves_the_authoritative_scroll_offset() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroll = overflowing_scroll_view(
            &mut context,
            document,
            nana_ui_core::ScrollbarVisibility::Always,
        );
        let bar = vertical_bar(&context, scroll).expect("resident bar");
        let grab_x = bar.thumb.x + bar.thumb.width / 2.0;
        let grab_y = bar.thumb.y + bar.thumb.height / 2.0;
        assert_eq!(
            context.scrollbar_axis_at(scroll.stable_id(), grab_x, grab_y),
            Some(nana_ui_core::ScrollbarAxis::Vertical)
        );
        assert!(
            context
                .begin_scrollbar_drag(
                    7,
                    scroll.stable_id(),
                    nana_ui_core::ScrollbarAxis::Vertical,
                    grab_x,
                    grab_y,
                )
                .unwrap()
        );
        assert_eq!(
            context.world().pointer_capture(document, 7),
            Some(scroll.stable_id())
        );
        assert_eq!(
            context.world().scroll_offset(scroll.stable_id()),
            Some(ScrollOffset::default()),
            "grabbing the thumb must not jump the content"
        );

        // Travel is 48px for 80px of content, so half the travel is 40px.
        assert!(
            context
                .update_scrollbar_drag(document, 7, grab_x, grab_y + 24.0)
                .unwrap()
        );
        let offset = context.world().scroll_offset(scroll.stable_id()).unwrap();
        assert!((offset.y - 40.0).abs() < 0.01, "offset {offset:?}");
        assert_eq!(offset.x, 0.0, "a vertical drag holds the other axis");

        assert!(
            context
                .update_scrollbar_drag(document, 7, grab_x, grab_y + 4000.0)
                .unwrap()
        );
        assert!(
            (context.world().scroll_offset(scroll.stable_id()).unwrap().y - 80.0).abs() < 0.01,
            "the drag clamps at the maximum offset"
        );

        assert!(context.end_scrollbar_drag(document, 7, false).unwrap());
        assert_eq!(context.world().pointer_capture(document, 7), None);
        assert!(
            !context
                .update_scrollbar_drag(document, 7, grab_x, grab_y)
                .unwrap(),
            "a released pointer no longer drives the bar"
        );
    }

    #[test]
    fn pressing_bare_track_pages_toward_the_press_and_cancelling_restores_it() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroll = overflowing_scroll_view(
            &mut context,
            document,
            nana_ui_core::ScrollbarVisibility::Always,
        );
        let bar = vertical_bar(&context, scroll).expect("resident bar");
        let track_end = bar.track.y + bar.track.height - 1.0;
        assert!(
            context
                .begin_scrollbar_drag(
                    3,
                    scroll.stable_id(),
                    nana_ui_core::ScrollbarAxis::Vertical,
                    bar.thumb.x + 1.0,
                    track_end,
                )
                .unwrap()
        );
        assert!(
            (context.world().scroll_offset(scroll.stable_id()).unwrap().y - 80.0).abs() < 0.01,
            "a press below the thumb centres it on the press"
        );
        assert!(context.end_scrollbar_drag(document, 3, true).unwrap());
        assert_eq!(
            context.world().scroll_offset(scroll.stable_id()),
            Some(ScrollOffset::default()),
            "cancel restores the offset the drag started from"
        );
    }

    #[test]
    fn a_secondary_press_reaches_the_nearest_handler_above_the_hit_node() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut card = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut card.layout);
            layout.width = Some(LengthSpec::Px(200.0));
            layout.height = Some(LengthSpec::Px(100.0));
        }
        let card = context
            .create_component(document, Card::new().style(card))
            .unwrap();
        let mut row = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut row.layout);
            layout.width = Some(LengthSpec::Px(200.0));
            layout.height = Some(LengthSpec::Px(40.0));
        }
        let row = context
            .create_component(document, Button::new("Row").style(row))
            .unwrap();
        context.append_child(card, row).unwrap();
        let presses = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&presses);
        context
            .on(card, move |_card, press: &SecondaryPress, _cx| {
                observed.lock().unwrap().push(*press);
            })
            .unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(200.0, 100.0))
            .unwrap();
        context.rebuild_hit_test(document);

        assert_eq!(
            context.secondary_press_at(document, 20.0, 20.0).unwrap(),
            Some(card.stable_id()),
            "the press bubbles to the enclosing handler"
        );
        let press = *presses.lock().unwrap().first().expect("one press");
        assert_eq!(press.target, row.stable_id(), "it carries the hit node");
        assert_eq!((press.x, press.y), (20.0, 20.0));

        assert_eq!(
            context.secondary_press_at(document, 900.0, 900.0).unwrap(),
            None,
            "a press outside the tree hits nothing"
        );
        assert_eq!(presses.lock().unwrap().len(), 1);
    }

    #[test]
    fn selection_reads_back_from_editors_and_from_focused_rich_text() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());
        assert_eq!(
            context.focused_selected_text(document),
            None,
            "a caret selects nothing"
        );
        assert!(context.select_all_focused_text(document).unwrap());
        assert_eq!(
            context.focused_selected_text(document).as_deref(),
            Some("Nana")
        );
        assert!(
            !context.select_all_focused_text(document).unwrap(),
            "selecting all twice is not a change"
        );
        assert_eq!(
            context.cut_focused_text(document).unwrap().as_deref(),
            Some("Nana")
        );
        assert_eq!(context.world().text(input.stable_id()), Some(""));
        assert_eq!(context.cut_focused_text(document).unwrap(), None);

        let text = context
            .create_component(
                document,
                crate::SelectableRichText::new([crate::RichSpan::plain("Hello")]),
            )
            .unwrap();
        assert!(context.focus_node(document, text.stable_id()).unwrap());
        let area = crate::LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 20.0,
        };
        let caret = |index: usize| index as f32 * crate::rich_text::GRAPHEME_ADVANCE + 1.0;
        context
            .read(text, |text| {
                assert!(text.pointer_down(caret(0), 8.0, area));
                assert!(text.pointer_move(caret(4), 8.0, area));
                text.pointer_up(caret(4), 8.0, area)
            })
            .unwrap();
        assert_eq!(
            context.focused_selected_text(document).as_deref(),
            Some("Hell"),
            "a rich-text selection is what a host copy takes"
        );
        assert_eq!(
            context.cut_focused_text(document).unwrap(),
            None,
            "rich text is not editable, so nothing is cut"
        );
    }

    #[test]
    fn a_secondary_press_without_a_handler_opens_nothing() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Px(120.0));
            layout.height = Some(LengthSpec::Px(40.0));
        }
        let button = context
            .create_component(document, Button::new("Build").style(style))
            .unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(120.0, 40.0))
            .unwrap();
        context.rebuild_hit_test(document);
        let generation = context.world().generation();
        assert_eq!(
            context.secondary_press_at(document, 10.0, 10.0).unwrap(),
            None
        );
        assert_eq!(
            context.world().generation(),
            generation,
            "an unhandled secondary press must not touch the tree"
        );
        assert!(context.world().focused(document).is_none());
        let _ = button;
    }

    #[test]
    fn scroll_view_with_forty_rows_dirties_forty_one_hit_targets() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroll = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        for index in 0..40 {
            let row = context
                .create_component(document, Text::new(format!("Visible row {index}")))
                .unwrap();
            context.append_child(scroll, row).unwrap();
        }
        let _ = context.take_system_work();
        assert!(
            context
                .scroll_to(scroll, ScrollOffset { x: 0.0, y: 120.0 })
                .unwrap()
        );
        let work = context.take_system_work();
        // Scroller-only hit/extract; Scene recomposes descendants from offset.
        assert_eq!(work.input_hit_test.len(), 1);
        assert_eq!(work.render_extraction.len(), 1);
        assert!(work.layout.is_empty());
        let updates = context.take_scroll_hit_updates();
        assert!(
            context.hit_test_work_is_scroll_only(&work.input_hit_test, &updates),
            "pure scrolling must be recognized as patch-only"
        );
    }

    #[test]
    fn native_theme_resolves_semantic_component_paint_without_layout_work() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                Button::new("Build").kind(nana_ui_core::ButtonKind::Primary),
            )
            .unwrap();
        let work = context.world_mut().take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let dark = context
            .world()
            .extract_nodes(&[button.stable_id()])
            .pop()
            .unwrap();
        assert_eq!(
            dark.style.background,
            Some(
                nana_ui_core::SemanticPalette::dark()
                    .accent_soft
                    .as_rgba_array()
            )
        );
        context
            .world_mut()
            .set_pointer_hover(document, 1, Some(button.stable_id()))
            .unwrap();
        let work = context.world_mut().take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        assert_eq!(
            context
                .world()
                .extract_nodes(&[button.stable_id()])
                .pop()
                .unwrap()
                .style
                .background,
            Some(
                nana_ui_core::SemanticPalette::dark()
                    .accent_soft_hover
                    .as_rgba_array()
            )
        );
        context
            .world_mut()
            .press_pointer(document, 1, button.stable_id())
            .unwrap();
        let work = context.world_mut().take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        assert_eq!(
            context
                .world()
                .extract_nodes(&[button.stable_id()])
                .pop()
                .unwrap()
                .style
                .background,
            Some(
                nana_ui_core::SemanticPalette::dark()
                    .accent_soft_pressed
                    .as_rgba_array()
            )
        );
        assert_eq!(
            context.release_pointer(document, 1),
            Some(button.stable_id())
        );
        context
            .world_mut()
            .set_pointer_hover(document, 1, None)
            .unwrap();
        context.world_mut().take_system_work();

        assert!(context.set_theme(ThemeMode::Light).unwrap());
        let work = context.world_mut().take_system_work();
        assert!(work.style.is_empty());
        assert!(work.layout.is_empty());
        assert!(work.render_extraction.contains(&button.stable_id()));
        let light = context
            .world()
            .extract_nodes(&[button.stable_id()])
            .pop()
            .unwrap();
        assert_eq!(
            light.style.background,
            Some(
                nana_ui_core::SemanticPalette::light()
                    .accent_soft
                    .as_rgba_array()
            )
        );

        let mut focus = MutationQueue::new();
        focus.request_focus(document, Some(button.stable_id()));
        context.world_mut().commit(focus).unwrap();
        let work = context.world_mut().take_system_work();
        assert_eq!(work.focus_ime, vec![button.stable_id()]);
        assert_eq!(work.accessibility, vec![button.stable_id()]);
        assert!(context.world_mut().take_system_work().is_empty());
        context.world_mut().resolve_styles(&work.style).unwrap();
        assert!(context.world_mut().take_system_work().is_empty());
        let focused = context
            .world()
            .extract_nodes(&[button.stable_id()])
            .pop()
            .unwrap();
        assert_eq!(focused.style.border_color, None);
        assert_eq!(
            focused.style.background,
            Some(
                nana_ui_core::SemanticPalette::light()
                    .accent_soft
                    .as_rgba_array()
            )
        );

        context
            .update_component(button, |button, _cx| button.disabled = true)
            .unwrap();
        let work = context.world_mut().take_system_work();
        assert_eq!(work.focus_ime, vec![button.stable_id()]);
        assert_eq!(work.accessibility, vec![button.stable_id()]);
        assert!(context.world_mut().take_system_work().is_empty());
        context.world_mut().resolve_styles(&work.style).unwrap();
        let post_resolve = context.world_mut().take_system_work();
        assert_eq!(post_resolve.focus_ime, vec![button.stable_id()]);
        assert_eq!(post_resolve.accessibility, vec![button.stable_id()]);
        assert_eq!(post_resolve.render_extraction, vec![button.stable_id()]);
        assert_eq!(context.world().focused(document), None);
        let disabled = context
            .world()
            .extract_nodes(&[button.stable_id()])
            .pop()
            .unwrap();
        assert_eq!(
            disabled.style.background,
            Some(
                nana_ui_core::SemanticPalette::light()
                    .subtle
                    .as_rgba_array()
            )
        );

        let generation = context.world().generation();
        assert!(!context.set_theme(ThemeMode::Light).unwrap());
        assert_eq!(context.world().generation(), generation);
        let idle = context.world_mut().take_system_work();
        assert!(
            idle.is_empty(),
            "unexpected work after theme no-op: {idle:?}"
        );
    }

    #[test]
    fn view_mutations_schedule_host_driven_animation_frames() {
        let mut context = AppContext::new();
        let entity = context
            .create_view(
                DocumentId::new(1).unwrap(),
                NodeKind::Document,
                Counter { value: 0 },
            )
            .unwrap();
        let id = AnimationId::new(1).unwrap();
        context
            .update(entity, |_view, cx| {
                let target = cx.entity().stable_id();
                cx.mutations().start_animation(AnimationSpec {
                    id,
                    target,
                    start: Duration::from_millis(40),
                    duration: Duration::from_millis(80),
                    frame_interval: Duration::from_millis(10),
                    easing: Easing::Linear,
                    iteration_count: crate::AnimationIteration::ONCE,
                    direction: crate::AnimationDirection::Normal,
                    fill_mode: crate::AnimationFillMode::None,
                    play_state: crate::AnimationPlayState::Running,
                });
            })
            .unwrap();

        assert_eq!(
            context.next_animation_deadline(),
            Some(Duration::from_millis(40))
        );
        let frame = context.advance_animations(Duration::from_millis(80));
        assert_eq!(frame.samples.len(), 1);
        assert_eq!(frame.samples[0].target, entity.stable_id());
        assert_eq!(frame.samples[0].progress, 0.5);
        assert_eq!(frame.next_deadline, Some(Duration::from_millis(90)));
    }

    #[test]
    fn remount_resumes_loading_lifecycle_in_a_retained_descendant() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, Button::new("Host"))
            .unwrap();
        let parent = context
            .create_component(document, Button::new("Parent"))
            .unwrap();
        let loading = context
            .create_detached_component(document, Button::new("Loading").loading(true))
            .unwrap();

        assert_eq!(context.next_animation_deadline(), None);
        context.append_child(parent, loading).unwrap();
        assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));

        let mut park = MutationQueue::new();
        park.park_subtree(parent.stable_id());
        context.commit_mutations(park).unwrap();
        assert_eq!(context.next_animation_deadline(), None);

        let mut remount = MutationQueue::new();
        remount.insert(host.stable_id(), parent.stable_id(), None);
        context.commit_mutations(remount).unwrap();
        assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));

        let frame = context.advance_animations(Duration::from_millis(400));
        assert!(frame.component_updates.contains(&loading.stable_id()));
        assert_eq!(
            context
                .read(loading, |button| button.loading_phase)
                .unwrap(),
            0.5
        );
        assert!(context.next_animation_deadline().is_some());
    }

    #[test]
    fn icon_button_tooltip_uses_hover_clock_and_real_overlay_child() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                IconButton::new(nana_ui_core::Icon::About, "Details").tooltip(
                    "More details",
                    nana_ui_core::TooltipConfig {
                        placement: nana_ui_core::TooltipPlacement::Left,
                        delay_ms: 100,
                        gap: 6.0,
                        viewport_padding: 4.0,
                        max_width: 120.0,
                    },
                ),
            )
            .unwrap();
        let tooltip = context.icon_button_tooltip(button).unwrap().unwrap();
        assert_eq!(
            context.world().node(tooltip.stable_id()).unwrap().parent,
            Some(button.stable_id())
        );
        assert_eq!(
            context.world().overlay_host(button.stable_id()),
            Some(crate::OverlayHostState::default())
        );
        context
            .layout_document(document, crate::LayoutViewport::new(160.0, 80.0))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            crate::LayoutBox {
                x: 20.0,
                y: 50.0,
                width: 28.0,
                height: 28.0,
            },
        );
        context.commit_mutations(layout).unwrap();

        context
            .set_pointer_hover_at(
                document,
                1,
                Some(button.stable_id()),
                Duration::from_millis(10),
            )
            .unwrap();
        assert_eq!(
            context.next_animation_deadline(),
            Some(Duration::from_millis(110))
        );
        assert!(
            !context
                .advance_animations(Duration::from_millis(109))
                .has_updates()
        );
        assert!(
            context
                .advance_animations(Duration::from_millis(110))
                .component_updates
                .contains(&button.stable_id())
        );
        assert_eq!(
            context
                .world()
                .overlay_host(button.stable_id())
                .unwrap()
                .active,
            Some(tooltip.stable_id())
        );
        let tooltip_style = context.world().node_style(tooltip.stable_id()).unwrap();
        assert!(matches!(
            tooltip_style.layout.offset_left,
            Some(LengthSpec::Px(x)) if (4.0..=156.0).contains(&x)
        ));
        assert!(matches!(
            tooltip_style.layout.offset_top,
            Some(LengthSpec::Px(y)) if (4.0..=76.0).contains(&y)
        ));
        assert!(
            matches!(
                tooltip_style.layout.offset_left,
                Some(LengthSpec::Px(x)) if x >= 54.0
            ),
            "tooltip should flip to the anchor's right, got {:?}",
            tooltip_style.layout.offset_left
        );

        context
            .set_pointer_hover_at(document, 1, None, Duration::from_millis(111))
            .unwrap();
        assert_eq!(
            context.world().overlay_host(button.stable_id()),
            Some(crate::OverlayHostState::default())
        );
        assert_eq!(context.next_animation_deadline(), None);
    }

    #[test]
    fn parked_icon_button_closes_tooltip_projection_and_does_not_reopen_on_remount() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, Button::new("Host"))
            .unwrap();
        let button = context
            .create_component(
                document,
                IconButton::new(nana_ui_core::Icon::About, "Details").tooltip(
                    "More details",
                    nana_ui_core::TooltipConfig {
                        delay_ms: 0,
                        ..nana_ui_core::TooltipConfig::default()
                    },
                ),
            )
            .unwrap();

        context
            .set_pointer_hover_at(document, 1, Some(button.stable_id()), Duration::ZERO)
            .unwrap();
        assert!(context.read(button, |button| button.tooltip_open).unwrap());
        assert!(matches!(
            context.world().standard_visual(button.stable_id()),
            Some(StandardVisual::Icon {
                tooltip: Some(crate::TooltipVisual { open: true, .. }),
                ..
            })
        ));

        let mut park = MutationQueue::new();
        park.park_subtree(button.stable_id());
        context.commit_mutations(park).unwrap();
        assert!(!context.read(button, |button| button.tooltip_open).unwrap());
        assert!(matches!(
            context.world().standard_visual(button.stable_id()),
            Some(StandardVisual::Icon {
                tooltip: Some(crate::TooltipVisual { open: false, .. }),
                ..
            })
        ));
        assert_eq!(
            context.world().overlay_host(button.stable_id()),
            Some(crate::OverlayHostState::default())
        );
        assert_eq!(context.next_animation_deadline(), None);

        let mut remount = MutationQueue::new();
        remount.insert(host.stable_id(), button.stable_id(), None);
        context.commit_mutations(remount).unwrap();
        assert!(
            !context
                .advance_animations(Duration::from_secs(1))
                .has_updates()
        );
        assert!(!context.read(button, |button| button.tooltip_open).unwrap());
        assert_eq!(
            context.world().overlay_host(button.stable_id()),
            Some(crate::OverlayHostState::default())
        );

        context
            .set_pointer_hover_at(
                document,
                2,
                Some(button.stable_id()),
                Duration::from_secs(2),
            )
            .unwrap();
        assert!(context.read(button, |button| button.tooltip_open).unwrap());
        context
            .update_component(button, |_button, cx| {
                cx.mutations().park_subtree(button.stable_id());
            })
            .unwrap();
        assert!(!context.read(button, |button| button.tooltip_open).unwrap());
        assert!(matches!(
            context.world().standard_visual(button.stable_id()),
            Some(StandardVisual::Icon {
                tooltip: Some(crate::TooltipVisual { open: false, .. }),
                ..
            })
        ));
    }

    #[test]
    fn tooltip_default_delay_stays_closed_until_deadline_and_is_label_only() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                IconButton::new(nana_ui_core::Icon::About, "Details")
                    .tooltip("More details", nana_ui_core::TooltipConfig::default()),
            )
            .unwrap();
        let tooltip = context.icon_button_tooltip(button).unwrap().unwrap();
        assert_eq!(
            context.world().text(tooltip.stable_id()),
            Some("More details")
        );
        let accessibility = context.world().accessibility(tooltip.stable_id()).unwrap();
        assert_eq!(accessibility.role, crate::AccessibilityRole::Tooltip);
        assert_eq!(accessibility.label.as_deref(), Some("More details"));
        assert!(
            !context
                .world()
                .interaction(tooltip.stable_id())
                .unwrap()
                .focusable
        );

        context
            .set_pointer_hover_at(
                document,
                1,
                Some(button.stable_id()),
                Duration::from_millis(10),
            )
            .unwrap();
        assert_eq!(
            context.next_animation_deadline(),
            Some(Duration::from_millis(360))
        );
        assert!(
            !context
                .advance_animations(Duration::from_millis(359))
                .has_updates()
        );
        assert_eq!(
            context.world().overlay_host(button.stable_id()),
            Some(crate::OverlayHostState::default())
        );
        assert!(
            context
                .advance_animations(Duration::from_millis(360))
                .component_updates
                .contains(&button.stable_id())
        );
        assert_eq!(
            context
                .world()
                .overlay_host(button.stable_id())
                .unwrap()
                .active,
            Some(tooltip.stable_id())
        );
        assert!(context.read(button, |button| button.tooltip_open).unwrap());
    }

    #[test]
    fn tooltip_default_follows_pointer_as_a_compact_card() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                IconButton::new(nana_ui_core::Icon::About, "Details").tooltip(
                    "More details",
                    nana_ui_core::TooltipConfig {
                        delay_ms: 0,
                        ..nana_ui_core::TooltipConfig::default()
                    },
                ),
            )
            .unwrap();
        let tooltip = context.icon_button_tooltip(button).unwrap().unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(200.0, 120.0))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            crate::LayoutBox {
                x: 20.0,
                y: 50.0,
                width: 28.0,
                height: 28.0,
            },
        );
        context.commit_mutations(layout).unwrap();

        context.set_pointer_location(document, 1, Some((48.0, 72.0)));
        context
            .set_pointer_hover_at(document, 1, Some(button.stable_id()), Duration::ZERO)
            .unwrap();
        assert_eq!(
            context
                .world()
                .overlay_host(button.stable_id())
                .unwrap()
                .active,
            Some(tooltip.stable_id())
        );

        let tooltip_style = context.world().node_style(tooltip.stable_id()).unwrap();
        assert_eq!(
            tooltip_style.layout.padding_left,
            Some(LengthSpec::Px(TooltipConfig::PADDING_X))
        );
        assert_eq!(
            tooltip_style.layout.padding_top,
            Some(LengthSpec::Px(TooltipConfig::PADDING_Y))
        );
        assert_eq!(
            tooltip_style.layout.border_radius,
            Some(TooltipConfig::RADIUS)
        );
        assert_eq!(
            tooltip_style.border,
            Some(nana_ui_core::SemanticColorRole::BorderSoft)
        );
        assert!(
            matches!(
                tooltip_style.layout.offset_left,
                Some(LengthSpec::Px(x)) if (x - 48.0).abs() < 0.01
            ),
            "default tooltip should bind to the pointer x, got {:?}",
            tooltip_style.layout.offset_left
        );
        assert!(
            matches!(
                tooltip_style.layout.offset_top,
                Some(LengthSpec::Px(y)) if y < 72.0 - TooltipConfig::PADDING_Y
            ),
            "tooltip should sit above the pointer, got {:?}",
            tooltip_style.layout.offset_top
        );
    }

    #[test]
    fn loading_components_schedule_only_while_loading() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let switch = context
            .create_component(document, Switch::new("Sync", false).loading(true))
            .unwrap();
        let card = context
            .create_component(document, crate::Card::new().loading(true))
            .unwrap();
        assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));
        let frame = context.advance_animations(Duration::ZERO);
        assert_eq!(
            frame
                .component_updates
                .iter()
                .copied()
                .collect::<HashSet<_>>(),
            HashSet::from([switch.stable_id(), card.stable_id()])
        );
        assert_eq!(
            context.next_animation_deadline(),
            Some(COMPONENT_FRAME_INTERVAL)
        );
        context
            .update_component(switch, |switch, _| switch.loading = false)
            .unwrap();
        context
            .update_component(card, |card, _| card.loading = false)
            .unwrap();
        assert_eq!(context.next_animation_deadline(), None);
        assert!(
            !context
                .advance_animations(Duration::from_secs(1))
                .has_updates()
        );
    }

    #[test]
    fn list_item_slots_are_unique_direct_children_in_canonical_order() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let item = context
            .create_component(document, ListItem::new("fallback"))
            .unwrap();
        let leading = context.create_component(document, Text::new("L")).unwrap();
        let content = context.create_component(document, Text::new("C")).unwrap();
        let trailing = context.create_component(document, Text::new("T")).unwrap();
        context.append_child(item, content).unwrap();
        context.append_child(item, trailing).unwrap();
        context.append_child(item, leading).unwrap();
        let slots = ListItemSlots {
            leading: Some(leading.stable_id()),
            content: Some(content.stable_id()),
            trailing: Some(trailing.stable_id()),
        };
        assert!(context.set_list_item_slots(item, slots).unwrap());
        assert_eq!(
            context.world().node(item.stable_id()).unwrap().children,
            vec![
                leading.stable_id(),
                content.stable_id(),
                trailing.stable_id()
            ]
        );
        assert!(!context.set_list_item_slots(item, slots).unwrap());

        let duplicate = ListItemSlots {
            leading: Some(leading.stable_id()),
            content: Some(leading.stable_id()),
            trailing: Some(trailing.stable_id()),
        };
        assert!(matches!(
            context.set_list_item_slots(item, duplicate),
            Err(FrameworkError::InvalidListItemSlots {
                item: invalid,
                slot: None
            }) if invalid == item.stable_id()
        ));
    }

    #[test]
    fn composite_geometry_separates_text_controls_and_range_drag_axis() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let sized = |width, height| {
            let mut style = NodeStyle::default();
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Px(width));
            layout.height = Some(LengthSpec::Px(height));
            style
        };
        let switch = context
            .create_component(
                document,
                Switch::new("Automatic updates", false)
                    .hint("Runs in the background")
                    .style(sized(380.0, 52.0)),
            )
            .unwrap();
        let range = context
            .create_component(
                document,
                RangeField::new(50.0, 0.0, 100.0, 1.0)
                    .unwrap()
                    .label("Volume")
                    .unit("%")
                    .style(sized(300.0, 58.0)),
            )
            .unwrap();
        let card = context
            .create_component(
                document,
                Card::new().title("Overview").padding(28.0).height(120.0),
            )
            .unwrap();
        let body = context
            .create_component(document, Text::new("Body"))
            .unwrap();
        context.append_child(card, body).unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(640.0, 480.0))
            .unwrap();

        let crate::ComponentGeometry::Switch {
            label,
            hint: Some(hint),
            control,
            ..
        } = context
            .world()
            .component_geometry(switch.stable_id())
            .unwrap()
        else {
            panic!("switch geometry must include label, hint, and control");
        };
        assert!(label.bounds.x + label.bounds.width <= control.x);
        assert!(label.bounds.y + label.bounds.height <= hint.bounds.y);

        let crate::ComponentGeometry::Card {
            title: Some(title),
            content,
            ..
        } = context
            .world()
            .component_geometry(card.stable_id())
            .unwrap()
        else {
            panic!("card geometry must include title and content");
        };
        assert!(title.bounds.y + title.bounds.height <= content.y);
        assert!(title.bounds.width >= content.width - 0.01);
        assert!(context.world().layout_box(body.stable_id()).unwrap().y >= content.y);
        let card_layout = context
            .world()
            .node_style(card.stable_id())
            .unwrap()
            .layout
            .as_ref();
        assert_eq!(card_layout.padding_top, Some(LengthSpec::Px(52.0)));
        assert_eq!(card_layout.padding_bottom, Some(LengthSpec::Px(28.0)));

        let crate::ComponentGeometry::Range { track, .. } = context
            .world()
            .component_geometry(range.stable_id())
            .unwrap()
        else {
            panic!("range geometry must expose the interaction axis");
        };
        assert!(track.x > context.world().layout_box(range.stable_id()).unwrap().x);
        context
            .begin_range_drag(document, 7, range.stable_id(), track.x)
            .unwrap();
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);
        context
            .update_range_drag(document, 7, track.x + track.width)
            .unwrap();
        assert_eq!(context.read(range, |range| range.value).unwrap(), 100.0);
    }

    #[test]
    fn component_size_kind_and_fallback_geometry_preserve_design_contracts() {
        for size in [
            nana_ui_core::ControlSize::Small,
            nana_ui_core::ControlSize::Medium,
            nana_ui_core::ControlSize::Large,
        ] {
            let mut context = AppContext::new();
            let document = DocumentId::new(1).unwrap();
            let switch = context
                .create_component(
                    document,
                    Switch::new("Automatic updates", false)
                        .hint("Runs in the background")
                        .size(size),
                )
                .unwrap();
            let range = context
                .create_component(
                    document,
                    RangeField::new(0.7, 0.0, 1.0, 0.1)
                        .unwrap()
                        .label("Opacity")
                        .unit("%")
                        .size(size),
                )
                .unwrap();
            context
                .layout_document(document, crate::LayoutViewport::new(380.0, 120.0))
                .unwrap();
            assert_eq!(
                context
                    .world()
                    .layout_box(switch.stable_id())
                    .unwrap()
                    .width,
                380.0
            );
            let crate::ComponentGeometry::Switch { label, control, .. } = context
                .world()
                .component_geometry(switch.stable_id())
                .unwrap()
            else {
                panic!("switch geometry expected");
            };
            assert_eq!(label.font_size, size.text_size());
            assert!(label.bounds.x + label.bounds.width <= control.x);
            let switch_interaction = context
                .world()
                .node_style(switch.stable_id())
                .unwrap()
                .interaction;
            assert_ne!(switch_interaction.hovered, switch_interaction.pressed);
            assert_ne!(switch_interaction.pressed, switch_interaction.focused);
            let crate::ComponentGeometry::Range {
                label: Some(label),
                value,
                unit: Some(unit),
                track,
            } = context
                .world()
                .component_geometry(range.stable_id())
                .unwrap()
            else {
                panic!("range geometry expected");
            };
            assert_eq!(label.font_size, size.text_size());
            assert!(label.bounds.x + label.bounds.width <= track.x);
            assert!(track.x + track.width <= value.bounds.x);
            assert!(value.bounds.x + value.bounds.width <= unit.bounds.x + 0.01);
            assert_eq!(
                context.world().standard_visual(range.stable_id()),
                Some(StandardVisual::Range {
                    label: Some(Arc::from("Opacity")),
                    value: Arc::from("0.7"),
                    unit: Some(Arc::from("%")),
                    size,
                    ratio: 0.7,
                    invalid: false,
                })
            );
        }

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        for kind in [
            nana_ui_core::CardKind::Surface,
            nana_ui_core::CardKind::Outlined,
            nana_ui_core::CardKind::Raised,
            nana_ui_core::CardKind::Flat,
            nana_ui_core::CardKind::Selected,
        ] {
            let card = context
                .create_component(document, Card::new().kind(kind))
                .unwrap();
            let (background, border, border_width) = {
                let style = context.world().node_style(card.stable_id()).unwrap();
                (style.background, style.border, style.layout.border_width)
            };
            context
                .layout_document(document, crate::LayoutViewport::new(240.0, 120.0))
                .unwrap();
            let crate::ComponentGeometry::Card { elevation, .. } = context
                .world()
                .component_geometry(card.stable_id())
                .unwrap()
            else {
                panic!("card geometry expected");
            };
            assert_eq!(
                elevation,
                (kind == nana_ui_core::CardKind::Raised).then_some(
                    crate::ComponentElevation::surface_shadow(nana_ui_core::ThemeMode::Dark)
                )
            );
            assert_eq!(
                background,
                match kind {
                    nana_ui_core::CardKind::Surface | nana_ui_core::CardKind::Raised => {
                        Some(nana_ui_core::SemanticColorRole::Surface)
                    }
                    nana_ui_core::CardKind::Selected => {
                        Some(nana_ui_core::SemanticColorRole::Selected)
                    }
                    nana_ui_core::CardKind::Outlined | nana_ui_core::CardKind::Flat => None,
                }
            );
            assert_eq!(
                (border, border_width),
                match kind {
                    nana_ui_core::CardKind::Outlined => {
                        (Some(nana_ui_core::SemanticColorRole::Border), Some(1.0))
                    }
                    nana_ui_core::CardKind::Selected => {
                        (Some(nana_ui_core::SemanticColorRole::BorderSoft), Some(1.0))
                    }
                    _ => (None, Some(0.0)),
                }
            );
        }

        let item = context
            .create_component(document, ListItem::new("Camera"))
            .unwrap();
        let item_interaction = context
            .world()
            .node_style(item.stable_id())
            .unwrap()
            .interaction;
        assert_ne!(item_interaction.selected, item_interaction.selected_hovered);
        let leading = context.create_component(document, Text::new("L")).unwrap();
        let trailing = context.create_component(document, Text::new("T")).unwrap();
        context.append_child(item, leading).unwrap();
        context.append_child(item, trailing).unwrap();
        context
            .set_list_item_slots(
                item,
                ListItemSlots {
                    leading: Some(leading.stable_id()),
                    content: None,
                    trailing: Some(trailing.stable_id()),
                },
            )
            .unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(240.0, 120.0))
            .unwrap();
        let crate::ComponentGeometry::ListItem {
            leading: Some(leading),
            content: Some(content),
            trailing: Some(trailing),
            detail: None
        } = context
            .world()
            .component_geometry(item.stable_id())
            .unwrap()
        else {
            panic!("list item fallback geometry expected");
        };
        assert!(leading.x + leading.width <= content.x);
        assert!(content.x + content.width <= trailing.x);

        let disabled_range = context
            .create_component(
                document,
                RangeField::new(0.5, 0.0, 1.0, 0.1)
                    .unwrap()
                    .label("Opacity")
                    .disabled(true),
            )
            .unwrap();
        assert_eq!(
            context
                .world()
                .node_style(disabled_range.stable_id())
                .unwrap()
                .interaction
                .disabled
                .foreground,
            Some(nana_ui_core::SemanticColorRole::Muted)
        );
    }

    #[test]
    fn observer_view_receives_source_event_and_owns_nested_events() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let source = context
            .create_view(document, NodeKind::Text, Counter { value: 0 })
            .unwrap();
        let observer = context
            .create_view(document, NodeKind::Text, Counter { value: 0 })
            .unwrap();
        context
            .observe(source, observer, |view, event: &Increment, cx| {
                view.value += event.0;
                cx.emit(Cascade);
            })
            .unwrap();
        context
            .on(observer, |view, _event: &Cascade, _cx| view.value += 1)
            .unwrap();
        context
            .update(source, |_view, cx| cx.emit(Increment(4)))
            .unwrap();
        assert_eq!(context.read(observer, |view| view.value).unwrap(), 5);
    }

    #[test]
    fn action_context_extension_and_view_removal_have_explicit_ownership() {
        struct TestExtension {
            installed: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl UiExtension for TestExtension {
            fn name(&self) -> &'static str {
                "test.extension"
            }

            fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
                let installed = Arc::clone(&self.installed);
                registrar.register_action(
                    "counter.increment",
                    ContextPredicate::always().all_of(["editor"]),
                    move |_| {
                        installed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        Ok(())
                    },
                )
            }
        }

        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let extension = TestExtension {
            installed: Arc::clone(&count),
        };
        let mut context = AppContext::new();
        context.install(&extension).unwrap();
        assert_eq!(
            context.install(&extension),
            Err(FrameworkError::DuplicateExtension("test.extension".into()))
        );
        let action = ActionId::new("counter.increment");
        assert_eq!(
            context.dispatch_action(&action, &KeyContext::default()),
            Err(FrameworkError::ActionUnavailable(action.clone()))
        );
        context
            .dispatch_action(&action, &KeyContext::new(["editor"]))
            .unwrap();
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 1);

        let entity = context
            .create_view(
                DocumentId::new(1).unwrap(),
                NodeKind::Document,
                Counter { value: 9 },
            )
            .unwrap();
        let child = context
            .create_view(
                DocumentId::new(1).unwrap(),
                NodeKind::Text,
                Counter { value: 3 },
            )
            .unwrap();
        context
            .update(entity, |_, cx| {
                cx.mutations()
                    .insert(entity.stable_id(), child.stable_id(), None);
            })
            .unwrap();
        let removed = context.remove_view(entity).unwrap();
        assert_eq!(removed.value, 9);
        assert!(!context.world().contains(entity.stable_id()));
        assert_eq!(
            context.read(child, |view| view.value),
            Err(FrameworkError::MissingView(child.stable_id()))
        );
    }

    #[test]
    fn recursive_events_are_bounded_per_update() {
        let mut context = AppContext::new();
        let entity = context
            .create_view(
                DocumentId::new(1).unwrap(),
                NodeKind::Document,
                Counter { value: 0 },
            )
            .unwrap();
        context
            .on(entity, |_view, _event: &Cascade, cx| cx.emit(Cascade))
            .unwrap();
        assert_eq!(
            context.update(entity, |_view, cx| cx.emit(Cascade)),
            Err(FrameworkError::EventOverflow(entity.stable_id()))
        );
        assert_eq!(context.read(entity, |view| view.value).unwrap(), 0);
    }

    #[test]
    fn extension_registration_is_atomic_on_conflict() {
        struct Conflict;
        impl UiExtension for Conflict {
            fn name(&self) -> &'static str {
                "conflict.extension"
            }

            fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
                registrar
                    .register_action("unique.action", ContextPredicate::always(), |_| Ok(()))?;
                registrar.register_action("existing.action", ContextPredicate::always(), |_| Ok(()))
            }
        }

        let mut context = AppContext::new();
        context
            .register_action("existing.action", ContextPredicate::always(), |_| Ok(()))
            .unwrap();
        assert_eq!(
            context.install(&Conflict),
            Err(FrameworkError::DuplicateAction(ActionId::new(
                "existing.action"
            )))
        );
        assert_eq!(
            context.dispatch_action(&ActionId::new("unique.action"), &KeyContext::default()),
            Err(FrameworkError::MissingAction(ActionId::new(
                "unique.action"
            )))
        );
    }

    #[test]
    fn presenter_extension_installs_onto_the_world() {
        struct Keyword;
        impl crate::TextPresenter for Keyword {
            fn name(&self) -> &'static str {
                "keyword"
            }

            fn present(
                &self,
                text: &str,
                _request: &crate::HighlightRequest,
            ) -> Vec<crate::TextSpan> {
                text.match_indices("fn")
                    .map(|(start, token)| crate::TextSpan {
                        start,
                        end: start + token.len(),
                        color: nana_ui_core::SemanticColorRole::Accent,
                    })
                    .collect()
            }
        }
        struct HighlightExt;
        impl UiExtension for HighlightExt {
            fn name(&self) -> &'static str {
                "test.highlight"
            }

            fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
                registrar.register_presenter(Box::new(Keyword))
            }
        }

        let mut context = AppContext::new();
        context.install(&HighlightExt).unwrap();
        assert_eq!(
            context.install(&HighlightExt),
            Err(FrameworkError::DuplicateExtension("test.highlight".into()))
        );
        assert_eq!(
            context.register_presenter(Box::new(Keyword)),
            Err(FrameworkError::World(UiWorldError::DuplicatePresenter(
                "keyword".into()
            )))
        );
        let mut request = crate::HighlightRequest::highlight("rs");
        request.presenter = Arc::from("keyword");
        let entity = context
            .create_component(
                DocumentId::new(1).unwrap(),
                TextArea {
                    highlight: Some(request),
                    ..TextArea::new("fn main")
                },
            )
            .unwrap();
        assert_eq!(
            context
                .world()
                .highlight_request(entity.stable_id())
                .map(|request| request.language.as_ref()),
            Some("rs")
        );
        context
            .resolve_presentations(&[entity.stable_id()])
            .unwrap();
        assert_eq!(
            context
                .world()
                .text_presentation(entity.stable_id())
                .map(|presentation| presentation.spans.len()),
            Some(1)
        );
    }

    #[test]
    fn builtin_and_plugin_components_share_one_registry() {
        let mut context = AppContext::new();
        assert!(context.resolve_component_tag("button").is_some());
        assert_eq!(
            context
                .resolve_component_tag("nana-gpu")
                .map(ComponentTypeId::as_str),
            Some("nana.gpu")
        );
        assert_eq!(
            context
                .resolve_component_tag("gpu-view")
                .map(ComponentTypeId::as_str),
            Some("nana.gpu-view")
        );
        assert_eq!(
            context.resolve_component_tag("chip"),
            None,
            "chip is a Button variant, not a registry tag"
        );
        assert_eq!(
            context.resolve_component_tag("virtual-list"),
            None,
            "virtual windows use scroll-view, not a second type"
        );
        assert_eq!(
            context
                .resolve_component_tag("nana-button")
                .map(ComponentTypeId::as_str),
            Some("nana.button")
        );
        assert_eq!(
            context
                .resolve_component_tag("select")
                .map(ComponentTypeId::as_str),
            Some("nana.select")
        );
        assert_eq!(
            context
                .resolve_component_tag("nana-select")
                .map(ComponentTypeId::as_str),
            Some("nana.select")
        );
        assert_eq!(
            context
                .resolve_component_tag("tabs")
                .map(ComponentTypeId::as_str),
            Some("nana.tabs")
        );
        assert_eq!(
            context
                .resolve_component_tag("dock")
                .map(ComponentTypeId::as_str),
            Some("nana.dock")
        );
        assert_eq!(
            context
                .resolve_component_tag("form-field")
                .map(ComponentTypeId::as_str),
            Some("nana.form-field")
        );
        assert_eq!(
            context
                .resolve_component_tag("nana-form-field")
                .map(ComponentTypeId::as_str),
            Some("nana.form-field")
        );
        assert!(
            context.resolve_component_tag("form").is_none(),
            "HTML form stays a layout box; nana-form-field owns form-field"
        );
        assert!(
            context.resolve_component_tag("search").is_none(),
            "HTML search is a landmark; SearchDropdown owns search-dropdown"
        );
        assert_eq!(
            context
                .resolve_component_tag("search-dropdown")
                .map(ComponentTypeId::as_str),
            Some("nana.search-dropdown")
        );
        assert_eq!(
            context
                .resolve_component_tag("nana-search-dropdown")
                .map(ComponentTypeId::as_str),
            Some("nana.search-dropdown")
        );

        #[derive(Clone)]
        struct ProbeCard {
            title: String,
        }
        impl ComponentView for ProbeCard {
            fn node_kind(&self) -> NodeKind {
                NodeKind::Element {
                    tag: "probe-card".into(),
                }
            }
            fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
                if world.text(id) != Some(self.title.as_str()) {
                    mutations.set_text(
                        id,
                        crate::TextContent {
                            value: self.title.clone(),
                        },
                    );
                }
            }
        }
        impl crate::RegisterableComponent for ProbeCard {
            const TYPE_ID: &'static str = "test.probe-card";
            const TAGS: &'static [&'static str] = &["nana-probe-card", "probe-card"];
            fn from_semantic(spec: &crate::SemanticSpec<'_>) -> Self {
                Self {
                    title: spec
                        .attr("handle")
                        .unwrap_or_else(|| spec.display_label())
                        .to_owned(),
                }
            }
        }
        struct ProbePlugin;
        impl UiExtension for ProbePlugin {
            fn name(&self) -> &'static str {
                "test.probe"
            }
            fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
                registrar.register_component::<ProbeCard>()
            }
        }

        context.install(&ProbePlugin).unwrap();
        assert_eq!(
            context
                .resolve_component_tag("nana-probe-card")
                .map(ComponentTypeId::as_str),
            Some("test.probe-card")
        );

        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Save"))
            .unwrap();
        assert_eq!(
            context
                .world()
                .component_type(button.stable_id())
                .map(ComponentTypeId::as_str),
            Some("nana.button")
        );
        let select_type = context.resolve_component_tag("select").unwrap().clone();
        let select_layout = std::sync::Arc::new(nana_ui_core::LayoutStyle::default());
        let select_spec = crate::SemanticSpec::from_parts(&select_type, &select_layout);
        let select = context
            .create_component(document, Select::from_semantic(&select_spec))
            .unwrap();
        assert_eq!(
            context
                .world()
                .component_type(select.stable_id())
                .map(ComponentTypeId::as_str),
            Some("nana.select")
        );
        let dock = context
            .create_component(
                document,
                crate::Dock::new(crate::DockNode::item("dock", None)),
            )
            .unwrap();
        assert_eq!(
            context
                .world()
                .component_type(dock.stable_id())
                .map(ComponentTypeId::as_str),
            Some("nana.dock")
        );

        let id = StableNodeId::new(42).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(
            id,
            document,
            NodeKind::Element {
                tag: "probe-card".into(),
            },
        );
        context.commit_mutations(queue).unwrap();
        let type_id = context.resolve_component_tag("probe-card").unwrap().clone();
        let layout = std::sync::Arc::new(nana_ui_core::LayoutStyle::default());
        let spec = crate::SemanticSpec {
            label: "User",
            ..crate::SemanticSpec::from_parts(&type_id, &layout)
        };
        let mut mutations = MutationQueue::new();
        assert_eq!(
            context.bind_semantic(id, &spec, &mut mutations).unwrap(),
            crate::ComponentBindKind::Projected
        );
        context.commit_mutations(mutations).unwrap();
        assert_eq!(context.world().text(id), Some("User"));
        assert_eq!(
            context
                .world()
                .component_type(id)
                .map(ComponentTypeId::as_str),
            Some("test.probe-card")
        );
    }

    /// Documented containers and chrome must carry a type identity, or Vue tag
    /// resolution and devtools cannot name the node.
    #[test]
    fn documented_containers_and_chrome_carry_a_type_identity() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        for (tag, type_id) in [
            ("list", "nana.list"),
            ("scroll-view", "nana.scroll-view"),
            ("table", "nana.table"),
            ("tr", "nana.table-row"),
            ("td", "nana.table-cell"),
            ("reorder-list", "nana.reorder-list"),
            ("time-series-chart", "nana.time-series-chart"),
            ("desktop-shell", "nana.desktop-shell"),
            ("app-title-bar", "nana.app-title-bar"),
            ("pane-chrome", "nana.pane-chrome"),
            ("sidebar-section", "nana.sidebar-section"),
            ("sidebar-footer", "nana.sidebar-footer"),
            (
                "settings-collapsible-card",
                "nana.settings-collapsible-card",
            ),
        ] {
            assert_eq!(
                context
                    .resolve_component_tag(tag)
                    .map(ComponentTypeId::as_str),
                Some(type_id),
                "tag `{tag}` must resolve"
            );
        }
        assert!(
            context.resolve_component_tag("scroll").is_none(),
            "aliases are pruned; scroll-view keeps the single tag"
        );

        let list = context
            .create_component(document, crate::List::new())
            .unwrap();
        assert_eq!(
            context
                .world()
                .component_type(list.stable_id())
                .map(ComponentTypeId::as_str),
            Some("nana.list")
        );
        let section = context
            .create_component(document, crate::SidebarSection::new("Files"))
            .unwrap();
        assert_eq!(
            context
                .world()
                .component_type(section.stable_id())
                .map(ComponentTypeId::as_str),
            Some("nana.sidebar-section")
        );
        let chart = context
            .create_component(document, crate::TimeSeriesChart::new([1.0, 2.0]))
            .unwrap();
        assert_eq!(
            context
                .world()
                .component_type(chart.stable_id())
                .map(ComponentTypeId::as_str),
            Some("nana.time-series-chart")
        );
    }

    #[test]
    fn plugin_component_registration_is_atomic_on_conflict() {
        #[derive(Clone)]
        struct StealButton;
        impl ComponentView for StealButton {
            fn node_kind(&self) -> NodeKind {
                NodeKind::Element {
                    tag: "button".into(),
                }
            }
            fn project(&self, _id: StableNodeId, _world: &UiWorld, _mutations: &mut MutationQueue) {
            }
        }
        impl crate::RegisterableComponent for StealButton {
            const TYPE_ID: &'static str = "nana.button";
            const TAGS: &'static [&'static str] = &["stolen"];
            fn from_semantic(_spec: &crate::SemanticSpec<'_>) -> Self {
                Self
            }
        }
        struct Conflict;
        impl UiExtension for Conflict {
            fn name(&self) -> &'static str {
                "conflict.components"
            }
            fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
                registrar.register_component::<StealButton>()
            }
        }

        let mut context = AppContext::new();
        assert_eq!(
            context.install(&Conflict),
            Err(FrameworkError::DuplicateComponentType("nana.button".into()))
        );
        assert!(context.resolve_component_tag("stolen").is_none());
        assert!(context.resolve_component_tag("button").is_some());
    }

    #[cfg(feature = "syntax-highlighting")]
    #[test]
    fn new_context_installs_the_official_highlight_presenter() {
        let mut context = AppContext::new();
        assert!(context.world().has_presenter(crate::HIGHLIGHT_PRESENTER));
        let entity = context
            .create_component(
                DocumentId::new(1).unwrap(),
                crate::HostedTextarea::new("fn main() {}", "rs"),
            )
            .unwrap();
        assert_eq!(
            context
                .world()
                .highlight_request(entity.stable_id())
                .map(|request| request.presenter.as_ref()),
            Some(crate::HIGHLIGHT_PRESENTER)
        );
        context
            .resolve_presentations(&[entity.stable_id()])
            .unwrap();
        let spans = context
            .world()
            .text_presentation(entity.stable_id())
            .map(|presentation| presentation.spans.clone())
            .unwrap_or_default();
        assert!(
            spans.iter().any(|span| {
                matches!(
                    span.color,
                    nana_ui_core::SemanticColorRole::Accent
                        | nana_ui_core::SemanticColorRole::AccentStrong
                ) && &"fn main() {}"[span.start..span.end] == "fn"
            }),
            "default Syntect presenter must color rust `fn`, got {spans:?}"
        );
    }

    struct One<T>(Option<T>);

    impl<T: Unpin> Stream for One<T> {
        type Item = T;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<T>> {
            Poll::Ready(self.0.take())
        }
    }

    #[test]
    fn task_and_subscription_preserve_host_owned_async_work() {
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut future = Task::ready(2).map(|value| value + 1).into_future();
        assert_eq!(future.as_mut().poll(&mut cx), Poll::Ready(3));

        let subscription = Subscription::new("window.events", One(Some(7)));
        assert_eq!(subscription.id(), "window.events");
        let mut stream = subscription.into_stream();
        assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(7)));
        assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(None));
    }

    #[test]
    fn virtual_list_materializes_only_visible_items_and_reuses_overlap() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let list = context.create_component(document, List::new()).unwrap();
        let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 10_000));
        let mut items = VirtualListItems::<usize, Text>::default();

        let first = context
            .materialize_virtual_list(
                list,
                &mut items,
                &layout,
                0.0,
                100.0,
                20.0,
                |index| index,
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert!(first.range.len() < 10);
        assert_eq!(
            context
                .world()
                .node(list.stable_id())
                .unwrap()
                .children
                .len(),
            first.range.len()
        );
        let overlap_key = first.range.end - 1;
        let overlap_entity = items.entity(&overlap_key).unwrap();
        let removed_key = first.range.start;
        let removed_entity = items.entity(&removed_key).unwrap();

        let next = context
            .materialize_virtual_list(
                list,
                &mut items,
                &layout,
                80.0,
                100.0,
                20.0,
                |index| index,
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert!(next.range.contains(&overlap_key));
        assert_eq!(items.entity(&overlap_key), Some(overlap_entity));
        assert!(!context.world().contains(removed_entity.stable_id()));
        assert_eq!(items.mounted_keys(), next.range.clone().collect::<Vec<_>>());
        let generation = context.world().generation();

        context
            .materialize_virtual_list(
                list,
                &mut items,
                &layout,
                80.0,
                100.0,
                20.0,
                |index| index,
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert_eq!(context.world().generation(), generation);
    }

    #[test]
    fn virtual_list_rejects_foreign_item_ownership_without_mutating() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let list = context.create_component(document, List::new()).unwrap();
        let other_list = context.create_component(document, List::new()).unwrap();
        let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 100));
        let mut items = VirtualListItems::<usize, Text>::default();

        let first = context
            .materialize_virtual_list(
                list,
                &mut items,
                &layout,
                0.0,
                100.0,
                0.0,
                |index| index,
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        let moved = items.entity(&first.range.start).unwrap();
        context.append_child(other_list, moved).unwrap();
        let generation = context.world().generation();

        assert_eq!(
            context.materialize_virtual_list(
                list,
                &mut items,
                &layout,
                200.0,
                100.0,
                0.0,
                |index| index,
                |index, _| Text::new(format!("row {index}")),
            ),
            Err(FrameworkError::InvalidVirtualization)
        );
        assert_eq!(context.world().generation(), generation);
        assert_eq!(
            context.world().node(moved.stable_id()).unwrap().parent,
            Some(other_list.stable_id())
        );
        assert!(context.world().contains(moved.stable_id()));
    }

    #[test]
    fn virtual_table_materializes_a_bounded_grid_and_reuses_both_axes() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let table = context.create_component(document, Table::new()).unwrap();
        let layout = VirtualTableLayout::new(
            std::iter::repeat_n(20.0, 10_000),
            (0..100).map(|index| nana_ui_core::TableColumn::new(index.to_string(), 80.0)),
        );
        let mut items = VirtualTableItems::<usize, usize>::default();

        let first = context
            .materialize_virtual_table(
                table,
                &mut items,
                &layout,
                (0.0, 0.0),
                (160.0, 100.0),
                (0.0, 20.0),
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
            )
            .unwrap();
        assert!(first.rows.range.len() < 10);
        assert!(first.columns.range.len() < 5);
        let overlap_row = first.rows.range.end - 1;
        let overlap_column = first.columns.range.end - 1;
        let overlap_row_entity = items.row_entity(&overlap_row).unwrap();
        let overlap_cell_entity = items.cell_entity(&overlap_row, &overlap_column).unwrap();
        let removed_row = first.rows.range.start;
        let removed_cell = items
            .cell_entity(&removed_row, &first.columns.range.start)
            .unwrap();

        let next = context
            .materialize_virtual_table(
                table,
                &mut items,
                &layout,
                (80.0, 40.0),
                (160.0, 100.0),
                (0.0, 20.0),
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
            )
            .unwrap();
        assert_eq!(items.row_entity(&overlap_row), Some(overlap_row_entity));
        assert_eq!(
            items.cell_entity(&overlap_row, &overlap_column),
            Some(overlap_cell_entity)
        );
        assert!(!context.world().contains(removed_cell.stable_id()));
        assert_eq!(
            items.mounted_rows(),
            next.rows.range.clone().collect::<Vec<_>>()
        );
        assert_eq!(
            items.mounted_columns(),
            next.columns.range.clone().collect::<Vec<_>>()
        );
        let retained_cells = next.rows.range.len() * next.columns.range.len();
        assert_eq!(
            next.rows
                .range
                .clone()
                .map(|row| context
                    .world()
                    .node(items.row_entity(&row).unwrap().stable_id())
                    .unwrap()
                    .children
                    .len())
                .sum::<usize>(),
            retained_cells
        );
        let generation = context.world().generation();
        context
            .materialize_virtual_table(
                table,
                &mut items,
                &layout,
                (80.0, 40.0),
                (160.0, 100.0),
                (0.0, 20.0),
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
            )
            .unwrap();
        assert_eq!(context.world().generation(), generation);
    }

    #[test]
    fn virtual_table_rejects_foreign_row_ownership_without_mutating() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let table = context.create_component(document, Table::new()).unwrap();
        let other_table = context.create_component(document, Table::new()).unwrap();
        let layout = VirtualTableLayout::new(
            std::iter::repeat_n(20.0, 100),
            (0..10).map(|index| nana_ui_core::TableColumn::new(index.to_string(), 80.0)),
        );
        let mut items = VirtualTableItems::<usize, usize>::default();
        let window = context
            .materialize_virtual_table(
                table,
                &mut items,
                &layout,
                (0.0, 0.0),
                (160.0, 100.0),
                (0.0, 0.0),
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
            )
            .unwrap();
        let moved = items.row_entity(&window.rows.range.start).unwrap();
        context.append_child(other_table, moved).unwrap();
        let generation = context.world().generation();

        assert_eq!(
            context.materialize_virtual_table(
                table,
                &mut items,
                &layout,
                (0.0, 200.0),
                (160.0, 100.0),
                (0.0, 0.0),
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
            ),
            Err(FrameworkError::InvalidVirtualization)
        );
        assert_eq!(context.world().generation(), generation);
        assert_eq!(
            context.world().node(moved.stable_id()).unwrap().parent,
            Some(other_table.stable_id())
        );
    }

    #[test]
    fn virtual_tree_materializes_only_visible_rows_and_reuses_overlap_on_scroll_and_expand() {
        const ROW: f32 = 20.0;
        const VIEWPORT: f32 = 100.0;
        const OVERSCAN: f32 = 20.0;
        let cap = VirtualListLayout::uniform_window_item_cap(VIEWPORT, OVERSCAN, ROW);
        assert!(cap < 10_000);

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let tree = context.create_component(document, List::new()).unwrap();
        let mut keys = (0..10_000).collect::<Vec<_>>();
        let mut layout = VirtualTreeLayout::uniform(ROW, std::iter::repeat_n(0, keys.len()));
        let mut items = VirtualTreeItems::<usize, Text>::default();

        let first = context
            .materialize_virtual_tree(
                tree,
                &mut items,
                &layout,
                0.0,
                VIEWPORT,
                OVERSCAN,
                |index| keys[index],
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert!(first.range.len() <= cap);
        assert_eq!(
            context
                .world()
                .node(tree.stable_id())
                .unwrap()
                .children
                .len(),
            first.range.len()
        );
        let overlap_key = keys[first.range.end - 1];
        let overlap_entity = items.entity(&overlap_key).unwrap();
        let removed_key = keys[first.range.start];
        let removed_entity = items.entity(&removed_key).unwrap();

        let next = context
            .materialize_virtual_tree(
                tree,
                &mut items,
                &layout,
                80.0,
                VIEWPORT,
                OVERSCAN,
                |index| keys[index],
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert!(next.range.len() <= cap);
        assert!(items.mounted_keys().contains(&overlap_key));
        assert_eq!(items.entity(&overlap_key), Some(overlap_entity));
        assert!(!context.world().contains(removed_entity.stable_id()));
        assert_eq!(
            items.mounted_keys(),
            next.range
                .clone()
                .map(|index| keys[index])
                .collect::<Vec<_>>()
        );

        let parent = keys.iter().position(|key| *key == overlap_key).unwrap();
        let child_keys = [1_000_000usize, 1_000_001];
        assert!(layout.expand(
            parent,
            child_keys.map(|_| nana_ui_core::VirtualTreeRow {
                extent: ROW,
                descendant_count: 0,
            })
        ));
        keys.splice(parent + 1..parent + 1, child_keys);
        let expanded = context
            .materialize_virtual_tree(
                tree,
                &mut items,
                &layout,
                80.0,
                VIEWPORT,
                OVERSCAN,
                |index| keys[index],
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert!(expanded.range.len() <= cap);
        assert_eq!(items.entity(&overlap_key), Some(overlap_entity));
        assert!(
            context
                .world()
                .node(tree.stable_id())
                .unwrap()
                .children
                .len()
                <= cap
        );
        assert!(items.entity(&child_keys[0]).is_some());
        let generation = context.world().generation();
        context
            .materialize_virtual_tree(
                tree,
                &mut items,
                &layout,
                80.0,
                VIEWPORT,
                OVERSCAN,
                |index| keys[index],
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert_eq!(context.world().generation(), generation);
    }

    #[test]
    fn virtual_tree_expand_keeps_live_children_below_geometric_cap() {
        const ROW: f32 = 20.0;
        const VIEWPORT: f32 = 100.0;
        const OVERSCAN: f32 = 20.0;
        const DESCENDANTS: usize = 10_000;
        let cap = VirtualListLayout::uniform_window_item_cap(VIEWPORT, OVERSCAN, ROW);
        assert!(cap < DESCENDANTS);

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let tree = context.create_component(document, List::new()).unwrap();
        let mut keys = vec![0usize, 1, 2];
        let mut layout = VirtualTreeLayout::uniform(ROW, [0, 0, 0]);
        let mut items = VirtualTreeItems::<usize, Text>::default();

        context
            .materialize_virtual_tree(
                tree,
                &mut items,
                &layout,
                0.0,
                VIEWPORT,
                OVERSCAN,
                |index| keys[index],
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();

        let child_keys = (1_000_000..1_000_000 + DESCENDANTS).collect::<Vec<_>>();
        assert!(layout.expand(
            0,
            child_keys.iter().map(|_| nana_ui_core::VirtualTreeRow {
                extent: ROW,
                descendant_count: 0,
            })
        ));
        keys.splice(1..1, child_keys);
        let descendant_count = layout
            .descendant_count(0)
            .expect("expanded parent keeps a descendant count");
        assert_eq!(descendant_count, DESCENDANTS);

        context
            .materialize_virtual_tree(
                tree,
                &mut items,
                &layout,
                0.0,
                VIEWPORT,
                OVERSCAN,
                |index| keys[index],
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        let live = context
            .world()
            .node(tree.stable_id())
            .unwrap()
            .children
            .len();
        assert!(
            live <= cap,
            "live List children {live} exceed geometric cap {cap}"
        );
        assert!(
            live < descendant_count,
            "live List children {live} mounted every expanded descendant ({descendant_count})"
        );
    }

    #[test]
    fn stack_presets_express_row_and_column_layout() {
        let row = Stack::row(8.0).node_style();
        let layout = row.layout;
        assert_eq!(layout.direction, Some(nana_ui_core::FlexDirection::Row));
        assert_eq!(layout.gap, Some(nana_ui_core::LengthSpec::Px(8.0)));
        assert_eq!(layout.align_items, nana_ui_core::AlignSpec::Center);
        assert_eq!(layout.width, Some(nana_ui_core::LengthSpec::Shrink));

        let fill_column = Stack::fill_column(0.0).node_style();
        assert_eq!(
            fill_column.layout.direction,
            Some(nana_ui_core::FlexDirection::Column)
        );
        assert_eq!(
            fill_column.layout.width,
            Some(nana_ui_core::LengthSpec::Fill)
        );
        assert_eq!(
            fill_column.layout.height,
            Some(nana_ui_core::LengthSpec::Fill)
        );
        assert_eq!(fill_column.layout.flex_grow, Some(1.0));
        assert_eq!(fill_column.layout.flex_shrink, Some(1.0));

        let outlined = Stack::column(4.0)
            .outline(nana_ui_core::SemanticColorRole::Border, 1.0)
            .node_style();
        assert_eq!(
            outlined.border,
            Some(nana_ui_core::SemanticColorRole::Border)
        );
        assert_eq!(outlined.layout.border_width, Some(1.0));
    }

    #[test]
    fn card_kind_defaults_yield_to_explicit_style() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();

        let surface = context.create_component(document, Card::new()).unwrap();
        let style = context
            .world()
            .node_style(surface.stable_id())
            .cloned()
            .unwrap();
        assert_eq!(
            style.background,
            Some(nana_ui_core::SemanticColorRole::Surface)
        );
        assert_eq!(style.border, None);
        assert_eq!(style.layout.border_width, Some(0.0));

        let custom = NodeStyle::default().outline(nana_ui_core::SemanticColorRole::Border, 2.0);
        let outlined = context
            .create_component(
                document,
                Card::new()
                    .kind(nana_ui_core::CardKind::Outlined)
                    .style(custom),
            )
            .unwrap();
        let style = context
            .world()
            .node_style(outlined.stable_id())
            .cloned()
            .unwrap();
        assert_eq!(
            style.border,
            Some(nana_ui_core::SemanticColorRole::Border),
            "用户显式设置的边框不得被 kind 默认值覆盖"
        );
        assert_eq!(
            style.layout.border_width,
            Some(2.0),
            "用户显式设置的边框宽度不得被 kind 默认值覆盖"
        );
    }
}
