//! AppContext events operations.

use super::*;

/// A node listening for [`SizeChanged`]: the size it was last told, and how
/// to tell it, since the watch outlives the handler's typed call site.
#[derive(Clone, Copy)]
pub(crate) struct SizeWatch {
    sent: Option<(f32, f32)>,
    emit: fn(&mut AppContext, StableNodeId, SizeChanged) -> Result<(), FrameworkError>,
}

fn emit_size<V: View>(
    cx: &mut AppContext,
    id: StableNodeId,
    event: SizeChanged,
) -> Result<(), FrameworkError> {
    cx.update(Entity::<V>::from_stable_id(id), |_, cx| cx.emit(event))
}

type SizeEmitFn = fn(&mut AppContext, StableNodeId, SizeChanged) -> Result<(), FrameworkError>;

/// An event handler erased where it is declared: everything installing it
/// needs, as data, so the installation code is not copied per handler.
pub(crate) struct ErasedOn {
    /// The observer's view type, checked where it is installed.
    view: TypeId,
    event: TypeId,
    /// How to tell the source its size, when the event is [`SizeChanged`].
    size_emit: Option<SizeEmitFn>,
    callback: ErasedEventHandler,
}

impl ErasedOn {
    pub(crate) fn new<V, E>(
        mut handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) -> Self
    where
        V: View,
        E: Send + 'static,
    {
        Self {
            view: TypeId::of::<V>(),
            event: TypeId::of::<E>(),
            size_emit: (TypeId::of::<E>() == TypeId::of::<SizeChanged>())
                .then_some(emit_size::<V> as SizeEmitFn),
            callback: Box::new(
                move |document: DocumentId,
                      observer: StableNodeId,
                      view: &mut dyn Any,
                      event: &dyn Any,
                      mutations: &mut MutationQueue,
                      events: &mut VecDeque<BoxedEvent>,
                      program_messages: &mut Vec<ProgramMessage>,
                      now: Duration| {
                    let view = view
                        .downcast_mut::<V>()
                        .expect("handler is registered for its view type");
                    let event = event
                        .downcast_ref::<E>()
                        .expect("handler is indexed by event type");
                    let mut cx = ViewContext {
                        document,
                        entity: Entity::from_stable_id(observer),
                        mutations,
                        events,
                        program_messages,
                        now,
                        reassemble: false,
                    };
                    handler(view, event, &mut cx);
                    cx.reassemble
                },
            ),
        }
    }

    /// The event type this handler listens for.
    pub(crate) fn event(&self) -> TypeId {
        self.event
    }
}

impl AppContext {
    pub(super) fn remove_event_handlers_for(&mut self, removed: &HashSet<StableNodeId>) -> usize {
        // An editor's undo journal dies with the editor.
        for id in removed {
            self.text_histories.forget(*id);
            self.lifecycle.key_handlers.remove(id);
            self.lifecycle.roving_focus_groups.remove(id);
            self.lifecycle.clamp_watchers.remove(id);
            self.lifecycle.size_watchers.remove(id);
        }
        let affected = removed
            .iter()
            .filter_map(|id| self.lifecycle.event_dependencies.remove(id))
            .flatten()
            .collect::<HashSet<_>>();
        let visited = affected.len();
        for key in affected {
            let Some(mut handlers) = self.lifecycle.event_handlers.remove(&key) else {
                continue;
            };
            let owners = std::iter::once(key.0)
                .chain(handlers.iter().map(|handler| handler.observer))
                .collect::<HashSet<_>>();
            for owner in owners {
                if let Some(keys) = self.lifecycle.event_dependencies.get_mut(&owner) {
                    keys.remove(&key);
                    if keys.is_empty() {
                        self.lifecycle.event_dependencies.remove(&owner);
                    }
                }
            }
            if removed.contains(&key.0) {
                continue;
            }
            handlers.retain(|handler| !removed.contains(&handler.observer));
            if !handlers.is_empty() {
                for handler in &handlers {
                    self.index_event_handler(key, handler.observer);
                }
                self.lifecycle.event_handlers.insert(key, handlers);
            }
        }
        visited
    }

    /// Drop every handler `observer` registered on `source` with
    /// [`Self::observe`]; the handlers of anyone else stay.
    pub(crate) fn unobserve(&mut self, source: StableNodeId, observer: StableNodeId) {
        let Some(keys) = self.lifecycle.event_dependencies.get(&source) else {
            return;
        };
        let keys = keys
            .iter()
            .copied()
            .filter(|key| key.0 == source)
            .collect::<Vec<_>>();
        for key in keys {
            let Some(handlers) = self.lifecycle.event_handlers.get_mut(&key) else {
                continue;
            };
            handlers.retain(|handler| handler.observer != observer);
            let empty = handlers.is_empty();
            if empty {
                self.lifecycle.event_handlers.remove(&key);
            }
            let owners = if empty {
                vec![source, observer]
            } else {
                vec![observer]
            };
            for owner in owners {
                if let Some(dependencies) = self.lifecycle.event_dependencies.get_mut(&owner) {
                    dependencies.remove(&key);
                    if dependencies.is_empty() {
                        self.lifecycle.event_dependencies.remove(&owner);
                    }
                }
            }
        }
    }

    fn index_event_handler(&mut self, key: (StableNodeId, TypeId), observer: StableNodeId) {
        self.lifecycle
            .event_dependencies
            .entry(key.0)
            .or_default()
            .insert(key);
        self.lifecycle
            .event_dependencies
            .entry(observer)
            .or_default()
            .insert(key);
    }

    pub(super) fn has_event_listeners<E: 'static>(&self, source: StableNodeId) -> bool {
        self.lifecycle
            .event_handlers
            .get(&(source, TypeId::of::<E>()))
            .is_some_and(|handlers| !handlers.is_empty())
    }

    pub fn on<V, E>(
        &mut self,
        entity: Entity<V>,
        handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) -> Result<(), FrameworkError>
    where
        V: View,
        E: Send + 'static,
    {
        self.read(entity, |_| ())?;
        self.install_handler(entity.id, entity.id, ErasedOn::new(handler), None);
        Ok(())
    }

    /// Whether `id` holds a view of type `view`: [`Self::read`]'s check for
    /// an erased handler.
    fn check_view(&self, id: StableNodeId, view: TypeId) -> Result<(), FrameworkError> {
        if !self.world.contains(id) {
            return Err(FrameworkError::MissingView(id));
        }
        let Some(actual) = self.view_type_id(id) else {
            return Err(FrameworkError::MissingView(id));
        };
        if actual != view {
            return Err(FrameworkError::ViewType(id));
        }
        Ok(())
    }

    /// Index `on` under `(source, its event)` for `observer`, start the
    /// clamp or size watch its event asks for, and append it, or with `key`
    /// replace the handler already holding that key.
    #[inline(never)]
    fn install_handler(
        &mut self,
        source: StableNodeId,
        observer: StableNodeId,
        on: ErasedOn,
        key: Option<String>,
    ) {
        let slot = (source, on.event);
        self.index_event_handler(slot, observer);
        self.note_gesture_listener(on.event);
        if on.event == TypeId::of::<TextClamped>() {
            self.lifecycle.clamp_watchers.entry(source).or_insert(None);
        }
        if let Some(emit) = on.size_emit {
            self.lifecycle
                .size_watchers
                .entry(source)
                .or_insert(SizeWatch { sent: None, emit });
        }
        let handlers = self.lifecycle.event_handlers.entry(slot).or_default();
        let existing = key.as_ref().and_then(|key| {
            handlers
                .iter_mut()
                .find(|entry| entry.key.as_ref() == Some(key))
        });
        let handler = EventHandler {
            key,
            observer,
            callback: on.callback,
        };
        match existing {
            Some(existing) => *existing = handler,
            None => handlers.push(handler),
        }
    }

    /// [`Self::on_keyed`] for a handler erased where it was declared.
    #[inline(never)]
    pub(crate) fn on_keyed_erased(
        &mut self,
        id: StableNodeId,
        key: String,
        on: ErasedOn,
    ) -> Result<(), FrameworkError> {
        if key.is_empty() {
            return Err(FrameworkError::InvalidInput);
        }
        self.check_view(id, on.view)?;
        self.install_handler(id, id, on, Some(key));
        Ok(())
    }

    /// Send [`SizeChanged`] to each listening node of `document` whose box
    /// the layout pass that just committed gave a new size.
    pub(super) fn announce_size_changes(
        &mut self,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        if self.lifecycle.size_watchers.is_empty() {
            return Ok(());
        }
        let changed = self
            .lifecycle
            .size_watchers
            .iter()
            .filter_map(|(&id, watch)| {
                if self.world.document_of(id) != Some(document) || !self.world.is_mounted(id) {
                    return None;
                }
                let bounds = self.world.canonical_layout_box(id)?;
                let size = (bounds.width, bounds.height);
                (watch.sent != Some(size)).then_some((id, size, watch.emit))
            })
            .collect::<Vec<_>>();
        for (id, (width, height), emit) in changed {
            if let Some(watch) = self.lifecycle.size_watchers.get_mut(&id) {
                watch.sent = Some((width, height));
            }
            emit(self, id, SizeChanged { width, height })?;
        }
        Ok(())
    }

    /// Send [`TextClamped`] to each listening text whose clamp changed with
    /// the shaping that just ran.
    pub(super) fn announce_text_clamps(&mut self) -> Result<(), FrameworkError> {
        if self.lifecycle.clamp_watchers.is_empty() {
            return Ok(());
        }
        let changed = self
            .lifecycle
            .clamp_watchers
            .iter()
            .filter_map(|(id, sent)| {
                let clamped = self.world.text_layout(*id).map(|(_, layout)| {
                    layout
                        .overflow
                        .contains(nana_text::OverflowFlags::TRUNCATED_LINES)
                })?;
                (*sent != Some(clamped)).then_some((*id, clamped))
            })
            .collect::<Vec<_>>();
        for (id, clamped) in changed {
            self.lifecycle.clamp_watchers.insert(id, Some(clamped));
            self.update(Entity::<crate::Text>::from_stable_id(id), |_, cx| {
                cx.emit(TextClamped { clamped });
            })?;
        }
        Ok(())
    }

    /// Replace a named binding while leaving ordinary `on` subscriptions intact.
    /// Bindings are removed with their entity, just like ordinary handlers.
    pub fn on_keyed<V, E>(
        &mut self,
        entity: Entity<V>,
        key: impl Into<String>,
        handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) -> Result<(), FrameworkError>
    where
        V: View,
        E: Send + 'static,
    {
        self.on_keyed_erased(entity.id, key.into(), ErasedOn::new(handler))
    }

    pub fn observe<S, V, E>(
        &mut self,
        source: Entity<S>,
        observer: Entity<V>,
        handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) -> Result<(), FrameworkError>
    where
        S: View,
        V: View,
        E: Send + 'static,
    {
        self.read(source, |_| ())?;
        self.read(observer, |_| ())?;
        let mut on = ErasedOn::new(handler);
        // The size watch tells the source, not the observer.
        if on.size_emit.is_some() {
            on.size_emit = Some(emit_size::<S>);
        }
        self.install_handler(source.id, observer.id, on, None);
        Ok(())
    }

    pub fn register_action(
        &mut self,
        id: impl Into<ActionId>,
        when: ContextPredicate,
        handler: impl FnMut(&mut AppContext) -> Result<(), FrameworkError> + Send + 'static,
    ) -> Result<(), FrameworkError> {
        insert_action(&mut self.lifecycle.actions, id, when, handler)
    }

    /// Binds an execution handler to `id` with an always-true predicate.
    ///
    /// Availability (`enabled` / `when` / dynamic checks) lives in [`ActionRegistry`];
    /// use [`Self::dispatch_registered_action`] (or the keymap / menu / palette
    /// adapters) to evaluate descriptor availability before running the handler.
    pub fn bind_action_handler(
        &mut self,
        id: impl Into<ActionId>,
        handler: impl FnMut(&mut AppContext) -> Result<(), FrameworkError> + Send + 'static,
    ) -> Result<(), FrameworkError> {
        insert_action(
            &mut self.lifecycle.actions,
            id,
            ContextPredicate::always(),
            handler,
        )
    }

    /// Removes the execution handler registered for `id`.
    pub fn unbind_action_handler(&mut self, id: &ActionId) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        self.lifecycle.actions.remove(&id).is_some()
    }

    /// Returns whether an execution handler is currently bound for `id`.
    pub fn has_action_handler(&self, id: &ActionId) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        self.lifecycle.actions.contains_key(&id)
    }

    /// Augments `base` with Runtime focus, modal, editable text-input, and IME
    /// composition tags for `document`.
    pub fn document_key_context(&self, document: DocumentId, mut base: KeyContext) -> KeyContext {
        if self.has_blocking_runtime_overlay(document) {
            base.insert(crate::CONTEXT_MODAL);
        }
        if self.editable_focused_text_input(document).is_some() {
            base.insert(crate::CONTEXT_TEXT_INPUT);
        }
        if self
            .world
            .focused(document)
            .is_some_and(|focused| self.world.ime(focused).is_some())
        {
            base.insert(crate::CONTEXT_IME_COMPOSING);
        }
        base
    }

    /// Returns whether `id` is available in `registry` AND has a matching handler here.
    pub fn can_dispatch_registered_action(
        &self,
        registry: &crate::ActionRegistry,
        id: &ActionId,
        context: &KeyContext,
    ) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        registry.is_available(&id, context)
            && self
                .lifecycle
                .actions
                .get(&id)
                .is_some_and(|action| action.when.matches(context))
    }

    pub fn dispatch_action(
        &mut self,
        id: &ActionId,
        context: &KeyContext,
    ) -> Result<(), FrameworkError> {
        let id = normalized_action_id(id.clone())?;
        let mut action = self
            .lifecycle
            .actions
            .remove(&id)
            .ok_or_else(|| FrameworkError::MissingAction(id.clone()))?;
        if !action.when.matches(context) {
            self.lifecycle.actions.insert(id.clone(), action);
            return Err(FrameworkError::ActionUnavailable(id));
        }
        let result = (action.handler)(self);
        self.lifecycle.actions.insert(id, action);
        result
    }

    /// Single canonical adapter between [`ActionRegistry`] availability and
    /// `AppContext` handler execution.
    pub fn dispatch_registered_action(
        &mut self,
        registry: &crate::ActionRegistry,
        id: &ActionId,
        context: &KeyContext,
    ) -> Result<(), FrameworkError> {
        let id = normalized_action_id(id.clone())?;
        if registry.get(&id).is_none() {
            return Err(FrameworkError::MissingAction(id));
        }
        if !registry.is_available(&id, context) {
            return Err(FrameworkError::ActionUnavailable(id));
        }
        self.dispatch_action(&id, context)
    }

    /// Resolves `key` through `keymap` against `registry` and, on dispatch,
    /// invokes the matching handler through [`Self::dispatch_registered_action`].
    pub fn dispatch_keymap_key(
        &mut self,
        keymap: &crate::Keymap,
        state: &mut crate::KeymapState,
        registry: &crate::ActionRegistry,
        key: &nana_ui_input::KeyInput,
        context: &KeyContext,
    ) -> Result<crate::KeymapMatch, FrameworkError> {
        if !key.is_pressed() {
            return Ok(crate::KeymapMatch::NoMatch);
        }
        let Some(stroke) = crate::CapturedStroke::from_canonical(key) else {
            return Ok(crate::KeymapMatch::NoMatch);
        };
        let matched = keymap.resolve(state, stroke, context, registry);
        if let crate::KeymapMatch::Dispatch(ref action) = matched {
            self.dispatch_registered_action(registry, action, context)?;
        }
        Ok(matched)
    }

    /// Routes an already-routed canonical input event through `keymap` only when
    /// no modal barrier, IME composition, menu bar, `on_key`, or focused editor
    /// consumed its default action (`disposition.prevent_default`).
    pub fn dispatch_routed_key(
        &mut self,
        keymap: &crate::Keymap,
        state: &mut crate::KeymapState,
        registry: &crate::ActionRegistry,
        event: &nana_ui_input::CanonicalInputEvent,
        disposition: nana_ui_input::InputDisposition,
        context: &KeyContext,
    ) -> Result<crate::KeymapMatch, FrameworkError> {
        if disposition.prevent_default {
            return Ok(crate::KeymapMatch::NoMatch);
        }
        match &event.payload {
            nana_ui_input::InputPayload::Key(key) => {
                self.dispatch_keymap_key(keymap, state, registry, key, context)
            }
            _ => Ok(crate::KeymapMatch::NoMatch),
        }
    }

    /// Resolves `menu_id` via `menu_map` and executes the action through
    /// [`Self::dispatch_registered_action`].
    pub fn dispatch_menu_action(
        &mut self,
        registry: &crate::ActionRegistry,
        menu_map: &crate::MenuActionMap,
        menu_id: u32,
        context: &KeyContext,
    ) -> Result<ActionId, FrameworkError> {
        let action = menu_map
            .action_for(menu_id)
            .cloned()
            .ok_or(FrameworkError::InvalidAction)?;
        self.dispatch_registered_action(registry, &action, context)?;
        Ok(action)
    }

    /// Handles a [`CommandPaletteEvent`] through [`Self::dispatch_registered_action`]
    /// when an item is selected.
    pub fn dispatch_palette_event(
        &mut self,
        registry: &crate::ActionRegistry,
        event: &CommandPaletteEvent,
        context: &KeyContext,
    ) -> Result<Option<ActionId>, FrameworkError> {
        match event {
            CommandPaletteEvent::Select(id) => {
                let normalized = normalized_action_id(id.clone())?;
                self.dispatch_registered_action(registry, &normalized, context)?;
                Ok(Some(normalized))
            }
            CommandPaletteEvent::Search(_)
            | CommandPaletteEvent::Navigate(_)
            | CommandPaletteEvent::Dismiss => Ok(None),
        }
    }

    /// Runs the handlers for queued events. Returns every other view a
    /// handler ran on, and whether it asked, through
    /// [`ViewContext::reassemble`], for its assembler to run once the caller
    /// commits.
    ///
    /// Observer handlers change those views in place, so nothing has
    /// projected their new state yet; the caller does that after it commits.
    pub(super) fn deliver_events(
        &mut self,
        id: StableNodeId,
        view: &mut dyn Any,
        mutations: &mut MutationQueue,
        events: &mut VecDeque<BoxedEvent>,
        program_messages: &mut Vec<ProgramMessage>,
    ) -> Result<Vec<super::TouchedObserver>, FrameworkError> {
        let mut touched: Vec<super::TouchedObserver> = Vec::new();
        let mut delivered = 0;
        while let Some((emitter, event_type, event)) = events.pop_front() {
            if event_type == TypeId::of::<crate::ColorPickRequested>() {
                self.request_color_pick(emitter);
            }
            delivered += 1;
            if delivered > MAX_EVENTS_PER_UPDATE {
                return Err(FrameworkError::EventOverflow(emitter));
            }
            let key = (emitter, event_type);
            let Some(mut handlers) = self.lifecycle.event_handlers.remove(&key) else {
                continue;
            };
            for handler in &mut handlers {
                if handler.observer == id {
                    let Some(document) = self.world.document_of(id) else {
                        continue;
                    };
                    (handler.callback)(
                        document,
                        id,
                        view,
                        event.as_ref(),
                        mutations,
                        events,
                        program_messages,
                        self.component_lifecycle.now,
                    );
                    continue;
                }
                let Some(observer_doc) = self.world.document_of(handler.observer) else {
                    continue;
                };
                let (mut observer, before_stack) = match self.store.views.remove(&handler.observer)
                {
                    Some(observer) => (observer, None),
                    None => match self.world.elided_stack(handler.observer) {
                        Some(stack) => {
                            let before = stack.clone();
                            (Box::new(stack) as Box<dyn Any + Send>, Some(before))
                        }
                        None => continue,
                    },
                };
                let asked = (handler.callback)(
                    observer_doc,
                    handler.observer,
                    observer.as_mut(),
                    event.as_ref(),
                    mutations,
                    events,
                    program_messages,
                    self.component_lifecycle.now,
                );
                if before_stack.as_ref().is_some_and(|before| {
                    !asked && observer.downcast_ref::<crate::Stack>() == Some(before)
                }) {
                    continue;
                }
                let observer_type = (*observer).type_id();
                match touched.iter_mut().find(|seen| seen.id == handler.observer) {
                    Some(seen) => seen.reassemble |= asked,
                    None => touched.push(super::TouchedObserver {
                        id: handler.observer,
                        type_id: observer_type,
                        reassemble: asked,
                    }),
                }
                self.store.views.insert(handler.observer, observer);
            }
            self.lifecycle.event_handlers.insert(key, handlers);
        }
        Ok(touched)
    }
}

#[cfg(test)]
mod dependency_tests {
    use super::*;
    use crate::Text;

    #[test]
    fn removing_observer_visits_only_subscribed_buckets_and_keeps_other_listeners() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let source = cx.create_component(document, Text::new("source")).unwrap();
        let first = cx.create_component(document, Text::new("first")).unwrap();
        let second = cx.create_component(document, Text::new("second")).unwrap();
        for _ in 0..1000 {
            let unrelated = cx
                .create_component(document, Text::new("unrelated"))
                .unwrap();
            cx.on::<_, Activate>(unrelated, |_, _, _| {}).unwrap();
        }
        cx.observe::<_, _, Activate>(source, first, |_, _, _| {})
            .unwrap();
        cx.observe::<_, _, Activate>(source, second, |_, _, _| {})
            .unwrap();
        cx.on_keyed::<_, Activate>(source, "binding", |_, _, _| {})
            .unwrap();
        cx.on_keyed::<_, Activate>(source, "binding", |_, _, _| {})
            .unwrap();
        assert_eq!(cx.remove_event_handlers_for(&HashSet::from([first.id])), 1);
        assert!(!cx.lifecycle.event_dependencies.contains_key(&first.id));
        let key = (source.id, TypeId::of::<Activate>());
        assert_eq!(cx.lifecycle.event_handlers[&key].len(), 2);
        assert!(cx.lifecycle.event_dependencies[&second.id].contains(&key));
        assert_eq!(cx.remove_event_handlers_for(&HashSet::from([source.id])), 1);
        assert!(!cx.lifecycle.event_dependencies.contains_key(&source.id));
        assert!(!cx.lifecycle.event_dependencies.contains_key(&second.id));
        assert_eq!(cx.lifecycle.event_handlers.len(), 1000);
        assert_eq!(cx.lifecycle.event_dependencies.len(), 1000);
        assert_eq!(cx.remove_event_handlers_for(&HashSet::new()), 0);
    }
}
