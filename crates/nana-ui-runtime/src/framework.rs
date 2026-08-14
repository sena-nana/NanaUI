use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::hash::Hash;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_core::Stream;
use nana_ui_core::{
    ActionId, ContextPredicate, KeyContext, LengthSpec, ThemeMode, TooltipPlacement,
    VirtualListLayout, VirtualListMaterializationError, VirtualListMaterializer, VirtualListWindow,
    VirtualTableLayout, VirtualTableMaterializer, VirtualTableWindow,
};

use crate::{
    AccessibilityAction, AccessibilityActionRequest, Activate, AnimationFrame, Button, Checkbox,
    ComponentView, Dialog, DocumentId, IconButton, List, ListItem, ListItemSlots, MenuItem,
    MutationQueue, NodeKind, OverlayChanged, OverlayHost, RangeAdjustment, RangeChanged,
    RangeField, ScrollAxes, ScrollChanged, ScrollMetrics, ScrollOffset, ScrollView, Slider,
    SliderChanged, StableNodeId, Switch, Tab, TabList, TabSelected, Table, TableCell, TableRow,
    TextArea, TextChanged, TextInput, TextInputState, TextSelection, ToggleChanged, Tooltip,
    UiWorld, UiWorldError,
};

const MAX_EVENTS_PER_UPDATE: usize = 16_384;
const COMPONENT_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const LOADING_CYCLE: Duration = Duration::from_millis(800);

pub trait View: Send + 'static {}

impl<T: Send + 'static> View for T {}

trait EditableText: ComponentView {
    fn disabled(&self) -> bool;
    fn replace_selection(&mut self, text: &str) -> bool;
    fn state(&self) -> &TextInputState;
    fn state_mut(&mut self) -> &mut TextInputState;
}

impl EditableText for TextInput {
    fn disabled(&self) -> bool {
        self.disabled
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
}

impl EditableText for TextArea {
    fn disabled(&self) -> bool {
        self.disabled
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
type ErasedEventHandler =
    Box<dyn FnMut(&mut dyn Any, &dyn Any, &mut MutationQueue, &mut VecDeque<BoxedEvent>) + Send>;
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
    tooltips: HashMap<StableNodeId, TooltipLifecycle>,
    loading: HashMap<StableNodeId, LoadingComponent>,
    next_loading_frame: Option<Duration>,
}

#[derive(Default)]
pub struct ExtensionRegistrar {
    actions: HashMap<ActionId, RegisteredAction>,
}

impl ExtensionRegistrar {
    pub fn register_action(
        &mut self,
        id: impl Into<ActionId>,
        when: ContextPredicate,
        handler: impl FnMut(&mut AppContext) -> Result<(), FrameworkError> + Send + 'static,
    ) -> Result<(), FrameworkError> {
        let id = normalized_action_id(id)?;
        if self.actions.contains_key(&id) {
            return Err(FrameworkError::DuplicateAction(id));
        }
        self.actions.insert(
            id,
            RegisteredAction {
                when,
                handler: Box::new(handler),
            },
        );
        Ok(())
    }
}

pub struct ViewContext<'a, V: View> {
    entity: Entity<V>,
    mutations: &'a mut MutationQueue,
    events: &'a mut VecDeque<BoxedEvent>,
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
    InvalidExtension,
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
    FrameDidNotSettle,
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
            Self::InvalidExtension => formatter.write_str("extension name must not be empty"),
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
            Self::FrameDidNotSettle => {
                formatter.write_str("runtime frame did not settle within the bounded pass limit")
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
    component_lifecycle: ComponentLifecycle,
    next_id: u64,
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
        Self {
            world: UiWorld::new(),
            views: HashMap::new(),
            event_handlers: HashMap::new(),
            actions: HashMap::new(),
            extensions: HashSet::new(),
            component_lifecycle: ComponentLifecycle::default(),
            next_id: 1,
        }
    }

    pub fn world(&self) -> &UiWorld {
        &self.world
    }

    #[cfg(test)]
    pub(crate) fn world_mut(&mut self) -> &mut UiWorld {
        &mut self.world
    }

    /// Commit one validated batch without exposing mutable access to the
    /// retained authority. Compatibility adapters and frame drivers use this
    /// for layout writeback and platform state projection.
    pub fn commit_mutations(
        &mut self,
        mutations: MutationQueue,
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.world.commit(mutations).map_err(FrameworkError::from)
    }

    /// Drain deterministic work scheduled since the previous frame.
    pub fn take_system_work(&mut self) -> crate::SystemWork {
        self.world.take_system_work()
    }

    /// Return a drained system batch to the scheduler after a canonical frame
    /// fails. Frame drivers should restore every consumed batch before retry.
    pub fn restore_system_work(&mut self, work: crate::SystemWork) {
        self.world.restore_system_work(work);
    }

    /// Resolve inherited style for the supplied dirty nodes.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), FrameworkError> {
        self.world.resolve_styles(ids).map_err(FrameworkError::from)
    }

    /// Shape only scheduled text through the host's real text backend.
    pub fn shape_text(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl crate::TextShaper,
    ) -> Result<(), FrameworkError> {
        self.world
            .shape_text(ids, shaper)
            .map_err(FrameworkError::from)
    }

    pub fn shape_text_for_layout(
        &mut self,
        document: DocumentId,
        shaper: &mut impl crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        self.world
            .shape_text_for_layout(document, shaper)
            .map_err(FrameworkError::from)
    }

    /// Compute and atomically publish canonical Runtime layout for one window.
    pub fn layout_document(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.component_lifecycle
            .viewports
            .insert(document, viewport);
        self.position_open_tooltips(document)?;
        let layouts =
            crate::RuntimeLayoutEngine.layout_document(&self.world, document, viewport)?;
        let mut mutations = MutationQueue::new();
        for (id, layout) in layouts {
            if self.world.layout_box(id) != Some(layout) {
                mutations.write_layout(id, layout);
            }
        }
        self.commit_mutations(mutations)
    }

    /// Rebuild the compact hit index for one document after layout or input
    /// work. The retained hierarchy remains private to this context.
    pub fn rebuild_hit_test(&mut self, document: DocumentId) {
        self.world.rebuild_hit_test(document);
    }

    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.world
            .next_animation_deadline()
            .into_iter()
            .chain(self.component_lifecycle.next_loading_frame)
            .chain(
                self.component_lifecycle
                    .tooltips
                    .values()
                    .filter_map(|tooltip| tooltip.show_at),
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
                .map(|(&target, &kind)| (target, kind))
                .collect::<Vec<_>>();
            for (target, kind) in loading {
                let changed = match kind {
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
            self.component_lifecycle.next_loading_frame = now.checked_add(COMPONENT_FRAME_INTERVAL);
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
    /// Re-applying the active mode is a true no-op.
    pub fn set_theme(&mut self, mode: ThemeMode) -> Result<bool, FrameworkError> {
        if self.world.theme_mode() == mode {
            return Ok(false);
        }
        let mut queue = MutationQueue::new();
        queue.set_theme(mode);
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
        self.world.commit(queue)?;
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

    /// Dispatch a semantic activation through the component's closure-event
    /// path. Disabled buttons do not emit or mutate retained state.
    pub fn activate_button(&mut self, entity: Entity<Button>) -> Result<bool, FrameworkError> {
        self.activate_component(entity, |button| button.disabled)
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

    pub fn activate_menu_item(&mut self, entity: Entity<MenuItem>) -> Result<bool, FrameworkError> {
        self.activate_component(entity, |item| item.disabled)
    }

    /// Activate a retained component selected by hit testing without exposing
    /// its concrete Rust type to a platform adapter.
    pub fn activate_node(&mut self, id: StableNodeId) -> Result<bool, FrameworkError> {
        let Some(view) = self.views.get(&id) else {
            return Ok(false);
        };
        if view.is::<Button>() {
            return self.activate_button(Entity::from_stable_id(id));
        }
        if view.is::<IconButton>() {
            return self.activate_icon_button(Entity::from_stable_id(id));
        }
        if view.is::<ListItem>() {
            return self.activate_list_item(Entity::from_stable_id(id));
        }
        if view.is::<MenuItem>() {
            return self.activate_menu_item(Entity::from_stable_id(id));
        }
        if view.is::<Checkbox>() {
            return self.toggle_checkbox(Entity::from_stable_id(id));
        }
        if view.is::<Switch>() {
            return self.toggle_switch(Entity::from_stable_id(id));
        }
        if view.is::<Tab>() {
            let Some(parent) = self.world.node(id).and_then(|node| node.parent) else {
                return Ok(false);
            };
            if self
                .views
                .get(&parent)
                .is_some_and(|view| view.is::<TabList>())
            {
                return self.select_tab(Entity::from_stable_id(parent), Entity::from_stable_id(id));
            }
        }
        Ok(false)
    }

    pub fn is_range_field(&self, id: StableNodeId) -> bool {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<RangeField>())
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

    fn sync_component_lifecycle(&mut self, id: StableNodeId) -> Result<(), FrameworkError> {
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
                let was_idle = self.component_lifecycle.loading.is_empty();
                self.component_lifecycle.loading.insert(id, kind);
                if was_idle {
                    self.component_lifecycle.next_loading_frame =
                        Some(self.component_lifecycle.now);
                }
            }
            None => {
                self.component_lifecycle.loading.remove(&id);
                if self.component_lifecycle.loading.is_empty() {
                    self.component_lifecycle.next_loading_frame = None;
                }
            }
        }
        Ok(())
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
        let padding_x = nana_ui_core::UI_METRICS.panel_padding_x;
        let padding_y = nana_ui_core::UI_METRICS.panel_padding_y;
        let desired_width = (metrics.width + padding_x * 2.0 + 2.0)
            .min(config.max_width)
            .max(0.0);
        let height = (metrics.height + padding_y * 2.0 + 2.0).max(0.0);
        let padding = config.viewport_padding.max(0.0);
        let left_available = (anchor.x - config.gap - padding).max(0.0);
        let right_available =
            (viewport.width - padding - (anchor.x + anchor.width + config.gap)).max(0.0);
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
            TooltipPlacement::Top | TooltipPlacement::Bottom => (desired_width, None),
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
        };
        let opposite = match config.placement {
            TooltipPlacement::Top => bottom,
            TooltipPlacement::Right => left,
            TooltipPlacement::Bottom => top,
            TooltipPlacement::Left => right,
        };
        let (x, y) = if let Some(side) = horizontal_side {
            match side {
                TooltipPlacement::Left => left,
                TooltipPlacement::Right => right,
                TooltipPlacement::Top | TooltipPlacement::Bottom => unreachable!(),
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
            || self.world.focused(document) == Some(target)
        {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.request_focus(document, Some(target));
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
        let Some((target, _)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TextInput>())
        {
            return self.commit_editable_ime(Entity::<TextInput>::from_stable_id(target), text);
        }
        if self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TextArea>())
        {
            return self.commit_editable_ime(Entity::<TextArea>::from_stable_id(target), text);
        }
        Ok(false)
    }

    pub fn replace_focused_text(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        let Some((target, _)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TextInput>())
        {
            return self.replace_text_input_selection(Entity::from_stable_id(target), text);
        }
        if self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TextArea>())
        {
            return self.replace_text_area_selection(Entity::from_stable_id(target), text);
        }
        Ok(false)
    }

    pub fn delete_focused_text_backward(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some((target, _)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TextInput>())
        {
            return self.delete_editable_backward(Entity::<TextInput>::from_stable_id(target));
        }
        if self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TextArea>())
        {
            return self.delete_editable_backward(Entity::<TextArea>::from_stable_id(target));
        }
        Ok(false)
    }

    fn delete_editable_backward<C: EditableText>(
        &mut self,
        entity: Entity<C>,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, EditableText::disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            let state = editable.state_mut();
            if state.selection.anchor == state.selection.focus {
                let caret = state.selection.focus;
                let Some(previous) = state.value[..caret]
                    .char_indices()
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
            if !state.replace_selection("") {
                return false;
            }
            cx.emit(TextChanged {
                value: state.value.clone(),
                selection: state.selection,
            });
            true
        })
    }

    fn commit_editable_ime<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, EditableText::disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            cx.mutations().set_ime(entity.stable_id(), None);
            if !editable.replace_selection(text) {
                return false;
            }
            cx.emit(TextChanged {
                value: editable.state().value.clone(),
                selection: editable.state().selection,
            });
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
                if self
                    .views
                    .get(&request.target)
                    .is_some_and(|view| view.is::<TextInput>())
                {
                    return self.set_editable_value(
                        Entity::<TextInput>::from_stable_id(request.target),
                        value,
                    );
                }
                if self
                    .views
                    .get(&request.target)
                    .is_some_and(|view| view.is::<TextArea>())
                {
                    return self.set_editable_value(
                        Entity::<TextArea>::from_stable_id(request.target),
                        value,
                    );
                }
                if self
                    .views
                    .get(&request.target)
                    .is_some_and(|view| view.is::<Slider>())
                {
                    return value
                        .parse::<f32>()
                        .ok()
                        .map(|value| {
                            self.set_slider_value(Entity::from_stable_id(request.target), value)
                        })
                        .transpose()
                        .map(|changed| changed.unwrap_or(false));
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
                if self
                    .views
                    .get(&request.target)
                    .is_some_and(|view| view.is::<TextInput>())
                {
                    return self.set_editable_selection(
                        Entity::<TextInput>::from_stable_id(request.target),
                        selection,
                    );
                }
                if self
                    .views
                    .get(&request.target)
                    .is_some_and(|view| view.is::<TextArea>())
                {
                    return self.set_editable_selection(
                        Entity::<TextArea>::from_stable_id(request.target),
                        selection,
                    );
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
        if self.read(entity, EditableText::disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if editable.state().value == value {
                return false;
            }
            editable.state_mut().replace_value(value);
            cx.emit(TextChanged {
                value: editable.state().value.clone(),
                selection: editable.state().selection,
            });
            true
        })
    }

    fn set_editable_selection<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        selection: TextSelection,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, EditableText::disabled)? {
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

    pub fn set_slider_value(
        &mut self,
        entity: Entity<Slider>,
        value: f32,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, |slider| slider.disabled)? {
            return Ok(false);
        }
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.update_component(entity, |slider, cx| {
            let value = value.clamp(slider.minimum, slider.maximum);
            if slider.value == value {
                return false;
            }
            slider.value = value;
            cx.emit(SliderChanged { value });
            true
        })
    }

    /// Select one direct Tab child with one retained-state commit. Typed Tab
    /// state is published only after that commit succeeds; observers run next.
    pub fn select_tab(
        &mut self,
        tab_list: Entity<TabList>,
        selected: Entity<Tab>,
    ) -> Result<bool, FrameworkError> {
        self.read(tab_list, |_| ())?;
        self.read(selected, |_| ())?;
        let list_node = self
            .world
            .node(tab_list.id)
            .ok_or(FrameworkError::MissingView(tab_list.id))?;
        if !list_node.children.contains(&selected.id) {
            return Err(FrameworkError::InvalidComponentHierarchy {
                parent: tab_list.id,
                child: selected.id,
            });
        }
        if self.read(selected, |tab| tab.disabled || tab.selected)? {
            return Ok(false);
        }

        let mut mutations = MutationQueue::new();
        let mut staged = Vec::new();
        for id in list_node.children {
            let Some(tab) = self
                .views
                .get(&id)
                .and_then(|view| view.downcast_ref::<Tab>())
            else {
                continue;
            };
            let next_selected = id == selected.id;
            if tab.selected == next_selected {
                continue;
            }
            let mut next = tab.clone();
            next.selected = next_selected;
            next.project(id, &self.world, &mut mutations);
            staged.push((id, next));
        }
        mutations.request_focus(list_node.document, Some(selected.id));
        self.world.commit(mutations)?;
        for (id, tab) in staged {
            self.views.insert(id, Box::new(tab));
        }
        self.update_component(tab_list, |_tab_list, cx| {
            cx.emit(TabSelected { tab: selected.id });
        })?;
        Ok(true)
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

    /// Route a logical-pixel scroll delta to the nearest hit ScrollView. At a
    /// clamped edge the event bubbles to an enclosing ScrollView.
    pub fn scroll_at(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
        delta: ScrollOffset,
    ) -> Result<Option<Entity<ScrollView>>, FrameworkError> {
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
            if !self
                .views
                .get(&id)
                .is_some_and(|view| view.is::<ScrollView>())
            {
                continue;
            }
            let entity = Entity::from_stable_id(id);
            if self.scroll_by(entity, delta)? {
                return Ok(Some(entity));
            }
        }
        Ok(None)
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
        if self.read(entity, EditableText::disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if !editable.replace_selection(text) {
                return false;
            }
            cx.emit(TextChanged {
                value: editable.state().value.clone(),
                selection: editable.state().selection,
            });
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
        let restore_focus = previous
            .restore_focus
            .or_else(|| self.world.focused(overlay_node.document));
        let next = crate::OverlayHostState {
            active: Some(overlay.id),
            restore_focus,
        };
        self.update_overlay_host(host, next, overlay_node.document, None)?;
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
        let dialog = self
            .views
            .get(&active)
            .and_then(|view| view.downcast_ref::<Dialog>())
            .ok_or(FrameworkError::ViewType(active))?;
        if !dialog.close_policy.allows(trigger) {
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
    ) -> Result<(), FrameworkError> {
        let focus = next
            .active
            .and_then(|active| Self::first_focusable_in_subtree(&self.world, active))
            .or_else(|| {
                next.restore_focus.or(dismiss_restore).filter(|id| {
                    self.world.contains(*id)
                        && self
                            .world
                            .interaction(*id)
                            .is_some_and(|interaction| interaction.focusable)
                })
            });
        let host_id = host.id;
        self.update_component(host, |_host, cx| {
            cx.mutations().set_overlay_host(host_id, next);
            cx.mutations().request_focus(document, focus);
            cx.emit(OverlayChanged {
                active: next.active,
            });
        })
    }

    fn first_focusable_in_subtree(world: &UiWorld, root: StableNodeId) -> Option<StableNodeId> {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if world
                .interaction(id)
                .is_some_and(|interaction| interaction.focusable)
            {
                return Some(id);
            }
            stack.extend(world.node(id)?.children.into_iter().rev());
        }
        None
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
        let result = update(
            &mut staged,
            &mut ViewContext {
                entity,
                mutations: &mut mutations,
                events: &mut events,
            },
        );
        let delivered = self.deliver_events(entity.id, &mut staged, &mut mutations, &mut events);
        if delivered.is_ok() {
            staged.project(entity.id, &self.world, &mut mutations);
        }
        let commit = delivered.and_then(|()| {
            self.world
                .commit(mutations)
                .map(|_| ())
                .map_err(FrameworkError::from)
        });
        if commit.is_ok() {
            self.views.insert(entity.id, Box::new(staged));
        } else {
            self.views.insert(entity.id, boxed);
        }
        commit?;
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
        let result = update(
            view,
            &mut ViewContext {
                entity,
                mutations: &mut mutations,
                events: &mut events,
            },
        );
        let delivered = self.deliver_events(entity.id, view, &mut mutations, &mut events);
        if delivered.is_ok() {
            project(view, &self.world, &mut mutations);
        }
        let commit = delivered.and_then(|()| {
            self.world
                .commit(mutations)
                .map(|_| ())
                .map_err(FrameworkError::from)
        });
        self.views.insert(entity.id, boxed);
        commit.map(|_| result)
    }

    pub fn remove_view<V: View>(&mut self, entity: Entity<V>) -> Result<V, FrameworkError> {
        self.read(entity, |_| ())?;
        let mut subtree = Vec::new();
        let mut stack = vec![entity.id];
        while let Some(id) = stack.pop() {
            let snapshot = self.world.node(id).ok_or(FrameworkError::MissingView(id))?;
            stack.extend(snapshot.children.iter().rev().copied());
            subtree.push(id);
        }
        let mut queue = MutationQueue::new();
        queue.despawn_subtree(entity.id);
        self.world.commit(queue)?;
        let removed = subtree.iter().copied().collect::<HashSet<_>>();
        self.remove_event_handlers_for(&removed);
        for id in &removed {
            self.component_lifecycle.tooltips.remove(id);
            self.component_lifecycle.loading.remove(id);
        }
        self.component_lifecycle
            .tooltips
            .retain(|_, tooltip| !removed.contains(&tooltip.overlay));
        if self.component_lifecycle.loading.is_empty() {
            self.component_lifecycle.next_loading_frame = None;
        }
        let boxed = self
            .views
            .remove(&entity.id)
            .expect("validated view must remain present");
        for id in subtree.into_iter().filter(|id| *id != entity.id) {
            self.views.remove(&id);
        }
        boxed
            .downcast::<V>()
            .map(|view| *view)
            .map_err(|_| FrameworkError::ViewType(entity.id))
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
                           events: &mut VecDeque<BoxedEvent>| {
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
                           events: &mut VecDeque<BoxedEvent>| {
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
        let id = normalized_action_id(id)?;
        if self.actions.contains_key(&id) {
            return Err(FrameworkError::DuplicateAction(id));
        }
        self.actions.insert(
            id,
            RegisteredAction {
                when,
                handler: Box::new(handler),
            },
        );
        Ok(())
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
        self.actions.extend(registrar.actions);
        self.extensions.insert(name);
        Ok(())
    }

    fn allocate_id(&mut self) -> StableNodeId {
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
                    (handler.callback)(view, event.as_ref(), mutations, events);
                    continue;
                }
                let Some(mut observer) = self.views.remove(&handler.observer) else {
                    continue;
                };
                (handler.callback)(observer.as_mut(), event.as_ref(), mutations, events);
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
        Slider, SliderChanged, StandardVisual, Switch, Tab, TabList, TabSelected, Table, TableCell,
        TableCellFocused, TableNavigation, TableRow, Text, TextChanged, TextContent, TextInput,
        TextSelection, ToggleChanged,
    };

    #[derive(Debug)]
    struct Counter {
        value: usize,
    }

    struct Increment(usize);
    struct Cascade;

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
            .create_component(document, Checkbox::new("Notifications", false))
            .unwrap();
        let switch = context
            .create_component(document, Switch::new("Auto build", true))
            .unwrap();
        let slider = context
            .create_component(
                document,
                Slider::new(25.0, 0.0, 100.0).unwrap().label("Volume"),
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
            .on(slider, move |_slider, event: &SliderChanged, _cx| {
                values.lock().unwrap().push(event.value);
            })
            .unwrap();

        assert!(context.toggle_checkbox(checkbox).unwrap());
        assert!(context.toggle_switch(switch).unwrap());
        assert!(context.set_slider_value(slider, 150.0).unwrap());
        assert!(!context.set_slider_value(slider, 100.0).unwrap());
        assert_eq!(
            context.set_slider_value(slider, f32::NAN),
            Err(FrameworkError::InvalidComponentValue(slider.stable_id()))
        );

        assert_eq!(*toggles.lock().unwrap(), vec![true]);
        assert_eq!(*slider_values.lock().unwrap(), vec![100.0]);
        assert_eq!(
            context.world().standard_visual(checkbox.stable_id()),
            Some(StandardVisual::Checkbox { checked: true })
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
            Some(StandardVisual::Slider { ratio: 1.0 })
        );
        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::Checkbox);
        assert_eq!(accessibility[0].checked, Some(true));
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
            .create_component(document, Slider::new(25.0, 0.0, 100.0).unwrap())
            .unwrap();
        let generation = context.world().generation();
        let visual = context.world().standard_visual(slider.stable_id());

        assert!(
            context
                .update_component(slider, |slider, _cx| slider.value = f32::NAN)
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
            .create_component(document, crate::Menu::new().label("Actions"))
            .unwrap();
        let menu_item = context
            .create_component(document, MenuItem::new("Build"))
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
        assert!(context.activate_menu_item(menu_item).unwrap());
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
    fn tab_selection_commits_one_group_and_emits_after_publication() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let tabs = context
            .create_component(document, TabList::new().label("Workspace"))
            .unwrap();
        let first = context
            .create_component(document, Tab::new("Preview").selected(true))
            .unwrap();
        let second = context
            .create_component(document, Tab::new("Program"))
            .unwrap();
        context.append_child(tabs, first).unwrap();
        context.append_child(tabs, second).unwrap();
        let observed = Arc::new(Mutex::new(None));
        let selected = Arc::clone(&observed);
        context
            .on(tabs, move |_tabs, event: &TabSelected, _cx| {
                *selected.lock().unwrap() = Some(event.tab);
            })
            .unwrap();
        context.world_mut().take_system_work();
        let generation = context.world().generation();

        assert!(context.select_tab(tabs, second).unwrap());
        assert!(!context.read(first, |tab| tab.selected).unwrap());
        assert!(context.read(second, |tab| tab.selected).unwrap());
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(*observed.lock().unwrap(), Some(second.stable_id()));
        assert_eq!(context.world().generation(), generation + 1);
        assert!(!context.select_tab(tabs, second).unwrap());

        let accessibility = context.world().project_accessibility(document);
        assert_eq!(accessibility[0].role, crate::AccessibilityRole::TabList);
        assert_eq!(accessibility[1].selected, Some(false));
        assert_eq!(accessibility[2].selected, Some(true));
        let work = context.world_mut().take_system_work();
        assert_eq!(work.style, vec![first.stable_id(), second.stable_id()]);
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
    fn native_theme_resolves_semantic_component_paint_without_layout_work() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
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
        assert_eq!(work.style, vec![button.stable_id()]);
        assert!(work.layout.is_empty());
        context.world_mut().resolve_styles(&work.style).unwrap();
        let light = context
            .world()
            .extract_nodes(&work.render_extraction)
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
        assert_eq!(
            focused.style.border_color,
            Some(
                nana_ui_core::SemanticPalette::light()
                    .accent
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
            let background = context
                .world()
                .node_style(card.stable_id())
                .unwrap()
                .background;
            let expected_elevation = kind == nana_ui_core::CardKind::Raised;
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
            assert_eq!(elevation.is_some(), expected_elevation);
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
}
