use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Duration;

use futures_core::Stream;
use nana_ui_core::{ActionId, ContextPredicate, KeyContext};

use crate::{
    AnimationFrame, DocumentId, MutationQueue, NodeKind, StableNodeId, UiWorld, UiWorldError,
};

const MAX_EVENTS_PER_UPDATE: usize = 16_384;

pub trait View: Send + 'static {}

impl<T: Send + 'static> View for T {}

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
    EventOverflow(StableNodeId),
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
            Self::EventOverflow(id) => {
                write!(
                    formatter,
                    "view {} emitted too many events in one update",
                    id.get()
                )
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
    next_id: u64,
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
            next_id: 1,
        }
    }

    pub fn world(&self) -> &UiWorld {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut UiWorld {
        &mut self.world
    }

    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.world.next_animation_deadline()
    }

    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        self.world.advance_animations(now)
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
        self.event_handlers.retain(|(id, _), handlers| {
            if removed.contains(id) {
                return false;
            }
            handlers.retain(|handler| !removed.contains(&handler.observer));
            !handlers.is_empty()
        });
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
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    use crate::{AnimationId, AnimationSpec, Easing, TextContent};

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
}
