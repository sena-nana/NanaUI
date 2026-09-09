//! AppContext events operations.

use super::*;

impl AppContext {
    pub(super) fn remove_event_handlers_for(&mut self, removed: &HashSet<StableNodeId>) -> usize {
        // An editor's undo journal dies with the editor.
        for id in removed {
            self.text_histories.forget(*id);
        }
        let affected = removed
            .iter()
            .filter_map(|id| self.event_dependencies.remove(id))
            .flatten()
            .collect::<HashSet<_>>();
        let visited = affected.len();
        for key in affected {
            let Some(mut handlers) = self.event_handlers.remove(&key) else {
                continue;
            };
            let owners = std::iter::once(key.0)
                .chain(handlers.iter().map(|handler| handler.observer))
                .collect::<HashSet<_>>();
            for owner in owners {
                if let Some(keys) = self.event_dependencies.get_mut(&owner) {
                    keys.remove(&key);
                    if keys.is_empty() {
                        self.event_dependencies.remove(&owner);
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
                self.event_handlers.insert(key, handlers);
            }
        }
        visited
    }

    fn index_event_handler(&mut self, key: (StableNodeId, TypeId), observer: StableNodeId) {
        self.event_dependencies
            .entry(key.0)
            .or_default()
            .insert(key);
        self.event_dependencies
            .entry(observer)
            .or_default()
            .insert(key);
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
        self.index_event_handler((entity.id, TypeId::of::<E>()), entity.id);
        self.event_handlers
            .entry((entity.id, TypeId::of::<E>()))
            .or_default()
            .push(EventHandler {
                key: None,
                observer: entity.id,
                callback: Box::new(erased),
            });
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
        let key = key.into();
        if key.is_empty() {
            return Err(FrameworkError::InvalidInput);
        }
        self.on(entity, handler)?;
        let handlers = self
            .event_handlers
            .get_mut(&(entity.id, TypeId::of::<E>()))
            .expect("on inserted the handler");
        let mut replacement = handlers.pop().expect("on appended the handler");
        replacement.key = Some(key.clone());
        if let Some(existing) = handlers
            .iter_mut()
            .find(|entry| entry.key.as_ref() == Some(&key))
        {
            *existing = replacement;
        } else {
            handlers.push(replacement);
        }
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
        self.index_event_handler((source.id, TypeId::of::<E>()), observer.id);
        self.event_handlers
            .entry((source.id, TypeId::of::<E>()))
            .or_default()
            .push(EventHandler {
                key: None,
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

    pub(super) fn deliver_events(
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
        assert!(!cx.event_dependencies.contains_key(&first.id));
        let key = (source.id, TypeId::of::<Activate>());
        assert_eq!(cx.event_handlers[&key].len(), 2);
        assert!(cx.event_dependencies[&second.id].contains(&key));
        assert_eq!(cx.remove_event_handlers_for(&HashSet::from([source.id])), 1);
        assert!(!cx.event_dependencies.contains_key(&source.id));
        assert!(!cx.event_dependencies.contains_key(&second.id));
        assert_eq!(cx.event_handlers.len(), 1000);
        assert_eq!(cx.event_dependencies.len(), 1000);
        assert_eq!(cx.remove_event_handlers_for(&HashSet::new()), 0);
    }
}
