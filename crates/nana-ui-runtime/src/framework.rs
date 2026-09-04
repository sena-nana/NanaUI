mod choice;
mod events;
mod frame;
mod lifecycle;
mod modal;
mod registry;
mod scroll;
mod selection;
mod text_input;
mod value_input;
mod virtualize;
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
    WorkspaceMutation,
};

#[cfg(test)]
use crate::Dialog;
use crate::component_registry::{
    ComponentBindKind, ComponentBindRequest, ComponentRegistry, ComponentTypeId,
    RegisterableComponent, SemanticSpec, alias_entry, registerable_entry, tag_entry,
};
use crate::{
    AccessibilityAction, AccessibilityActionRequest, ActionMenu, ActionMenuItem, Activate,
    AnimationFrame, BreadcrumbSegment, Button, Checkbox, CodeEditing, CommandPalette,
    ComponentView, ContextMenu, ContextMenuEvent, DocumentId, Dropdown, EmptyState, FileTab,
    FormField, FrameProfile, FrameProfiler, FrameStage, IconButton, LabeledValue, List, ListItem,
    ListItemSlots, ModalSlots, ModalSurface, MountState, MutationQueue, NodeKind, NumberChanged,
    NumberInput, OverlayChanged, OverlayHost, Popover, PopoverClosed, PopoverToggled, Progress,
    ProgressCancelled, RangeAdjustment, RangeChanged, RangeField, RovingFocusIntent, ScrollAxes,
    ScrollChanged, ScrollMetrics, ScrollOffset, ScrollView, SearchDropdown, SearchDropdownEvent,
    SecondaryPress, SegmentedControl, SegmentedOption, SegmentedSelectionRequested, Select,
    SettingsCollapsibleCard, SidebarFooterButton, SidebarRow, SidebarSection, StableNodeId,
    StandardVisual, Switch, Table, TableCell, TableRow, Tabs, TextArea, TextChanged, TextInput,
    TextInputState, TextPresenter, TextSelection, ToggleChanged, Tooltip, TreeView, UiWorld,
    UiWorldError, Workspace, XYPad, XYPadDragState, XYPadEvent,
};

mod assemble;
mod build;
mod overlay;
pub(crate) mod text_edit;
pub use assemble::AssemblyScope;
pub use build::UiBuilder;
pub(crate) use overlay::overlay_kind_for_role;
pub use overlay::{
    ActiveRuntimeOverlay, OverlayKey, OverlayPointerDecision, OverlayPointerPhase,
    RuntimeOverlayKind,
};
pub use text_edit::{TextDeleteKind, TextFindScope};

const MAX_EVENTS_PER_UPDATE: usize = 16_384;
pub(crate) const COMPONENT_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const LOADING_CYCLE: Duration = Duration::from_millis(800);

pub trait View: Send + 'static {}

impl<T: Send + 'static> View for T {}

trait EditableText: ComponentView {
    type Change: Send + 'static;
    fn accepts_input(&self) -> bool;
    /// Replace the text of every active selection (single cursor replaces its
    /// own selection; multiple cursors each receive an insertion).
    fn replace_selection(&mut self, text: &str) -> bool;
    fn text_atoms(&self) -> &[crate::TextAtomSpan] {
        &[]
    }
    /// IME commit path: replace only the primary selection's text. This is
    /// the documented multi-cursor IME restriction — composition commits to
    /// the primary cursor alone and other cursors survive via offset
    /// remapping.
    fn commit_ime_text(&mut self, text: &str) -> bool {
        self.state_mut().replace_primary_selection(text)
    }
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

    fn text_atoms(&self) -> &[crate::TextAtomSpan] {
        &self.atom_spans
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

    fn commit_ime_text(&mut self, text: &str) -> bool {
        // Composite search surfaces are single-selection and keep `query`
        // synchronized through their own replace path.
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

    fn commit_ime_text(&mut self, text: &str) -> bool {
        // Searchable menus keep `query` synchronized with the committed
        // state through their own replace path.
        self.replace_selection(text)
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

    fn commit_ime_text(&mut self, text: &str) -> bool {
        // Composite palettes are single-selection and keep `query`
        // synchronized through their own replace path.
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
    workspace_transitions: HashMap<StableNodeId, ()>,
    next_workspace_frame: Option<Duration>,
    overlay_pointer_sequences: HashSet<(DocumentId, u64)>,
    overlay_outside_presses: HashMap<(DocumentId, u64), (StableNodeId, u64)>,
    overlay_activation_tokens: HashMap<StableNodeId, u64>,
    next_overlay_activation_token: u64,
    split_hover_probe_last: HashMap<DocumentId, Duration>,
}

impl ComponentLifecycle {
    fn begin_split_hover_probe(&mut self, document: DocumentId, now: Duration) -> bool {
        const HOVER_PROBE_INTERVAL: Duration = Duration::from_millis(8);
        if let Some(last) = self.split_hover_probe_last.get(&document)
            && now.saturating_sub(*last) < HOVER_PROBE_INTERVAL
        {
            return false;
        }
        self.split_hover_probe_last.insert(document, now);
        true
    }
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

/// Reproject a component view whose concrete type the scheduler no longer
/// knows, via the typed `update_component` pipeline captured at stamp time.
type ChildReprojectFn = fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>;

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
    /// Opt-in reproject entry points keyed by node, registered from
    /// [`ComponentView::wants_child_reproject`] when a component view is
    /// stamped. The stored function reprojects through the typed
    /// `update_component` pipeline.
    child_reproject_views: HashMap<StableNodeId, ChildReprojectFn>,
    /// Nodes queued for one child-structure reproject; deduplicated per drain.
    pending_child_reprojects: Vec<StableNodeId>,
    /// Guards reentrant drains while a reproject commits its own mutations.
    draining_child_reprojects: bool,
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
    text_edit: text_edit::TextEditSession,
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
            child_reproject_views: HashMap::new(),
            pending_child_reprojects: Vec::new(),
            draining_child_reprojects: false,
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
            text_edit: text_edit::TextEditSession::default(),
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
        self.prepare_surface_closing(&mut mutations);
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
                self.resume_component_lifecycle(id)?;
            }
        }
        self.collect_child_reprojects();
        self.drain_child_reprojects()?;
        Ok(report)
    }

    /// Queue opt-in parents whose child structure just changed for one
    /// reproject through the `update_component` pipeline.
    fn collect_child_reprojects(&mut self) {
        for parent in self.world.take_structural_change_parents() {
            if !self.child_reproject_views.contains_key(&parent)
                || self.pending_child_reprojects.contains(&parent)
            {
                continue;
            }
            self.pending_child_reprojects.push(parent);
        }
    }

    /// Run queued reprojects. Reentrant commits made by a reproject append to
    /// the queue and are consumed by the same drain; nodes whose view is
    /// temporarily absent (mid-update or not yet installed by a build) wait
    /// for the next commit instead.
    fn drain_child_reprojects(&mut self) -> Result<(), FrameworkError> {
        if self.draining_child_reprojects {
            return Ok(());
        }
        self.draining_child_reprojects = true;
        let result = self.drain_child_reprojects_inner();
        self.draining_child_reprojects = false;
        result
    }

    fn drain_child_reprojects_inner(&mut self) -> Result<(), FrameworkError> {
        loop {
            let pending = std::mem::take(&mut self.pending_child_reprojects);
            if pending.is_empty() {
                return Ok(());
            }
            let mut deferred = Vec::new();
            let mut progressed = false;
            for id in pending {
                let Some(reproject) = self.child_reproject_views.get(&id).copied() else {
                    continue;
                };
                if !self.views.contains_key(&id) {
                    deferred.push(id);
                    continue;
                }
                progressed = true;
                reproject(self, id)?;
            }
            self.pending_child_reprojects = deferred;
            if !progressed {
                // Every entry is waiting on a view no reproject created; leave
                // them pending for a later drain instead of spinning forever.
                return Ok(());
            }
        }
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
                    easing: crate::Easing::EaseInOutCubic,
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
        self.world.animation_now = now;
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
        self.child_reproject_views
            .retain(|id, _| !removed.contains(id));
        self.pending_child_reprojects
            .retain(|id| !removed.contains(id));
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
mod tests;
