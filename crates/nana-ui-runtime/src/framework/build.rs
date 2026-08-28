use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};

use crate::{
    ComponentView, DocumentId, Entity, MutationQueue, StableNodeId, Stack, View, ViewContext,
};

use super::assemble::AssembledChild;
use super::{AppContext, FrameworkError};

const DUMMY_NODE: StableNodeId = match StableNodeId::new(u64::MAX) {
    Some(id) => id,
    None => panic!("u64::MAX is a valid stable id"),
};

struct Level {
    parent: Option<StableNodeId>,
    seen: Vec<String>,
    autos: HashMap<&'static str, usize>,
}

/// Nested tree builder that commits one mutation batch, then installs handlers.
pub struct UiBuilder<'a> {
    context: &'a mut AppContext,
    document: DocumentId,
    stack: Vec<Level>,
    queue: MutationQueue,
    working: HashMap<StableNodeId, HashMap<String, AssembledChild>>,
    pending_views: HashMap<StableNodeId, Box<dyn Any + Send>>,
    pending_ons: Vec<Box<dyn FnOnce(&mut AppContext) -> Result<(), FrameworkError>>>,
    pending_forget: HashSet<StableNodeId>,
    lifecycle: Vec<StableNodeId>,
    park_roots: bool,
    error: Option<FrameworkError>,
}

impl AppContext {
    /// Build a keyed subtree and commit it in one retained-tree batch.
    ///
    /// Nested closures do not return [`Result`]. Events queued with
    /// [`UiBuilder::on`] are installed after that commit. This is initial
    /// construction (and first attach); later add/remove of children still uses
    /// [`Self::mount`]. Do not call this from a click handler to rebuild a page.
    pub fn build<R>(
        &mut self,
        document: DocumentId,
        build: impl FnOnce(&mut UiBuilder<'_>) -> R,
    ) -> Result<R, FrameworkError> {
        UiBuilder::run(self, document, None, false, build)
    }

    /// Like [`Self::build`], but top-level nodes stay parked until inserted.
    ///
    /// Use this for subtrees that a later assemble step (shell, dock, overlay)
    /// will attach. Nested children still insert in the same commit.
    pub fn build_detached<R>(
        &mut self,
        document: DocumentId,
        build: impl FnOnce(&mut UiBuilder<'_>) -> R,
    ) -> Result<R, FrameworkError> {
        UiBuilder::run(self, document, None, true, build)
    }

    /// Build keyed children of an existing parent in one commit.
    ///
    /// Keys share the table used by [`Self::mount`], so a later `mount` on the
    /// same parent reuses identities.
    pub fn build_child<P: View, R>(
        &mut self,
        parent: Entity<P>,
        build: impl FnOnce(&mut UiBuilder<'_>) -> R,
    ) -> Result<R, FrameworkError> {
        self.read(parent, |_| ())?;
        let document = self
            .world
            .node(parent.id)
            .ok_or(FrameworkError::MissingView(parent.id))?
            .document;
        UiBuilder::run(self, document, Some(parent.id), false, build)
    }
}

impl<'a> UiBuilder<'a> {
    fn run<R>(
        context: &'a mut AppContext,
        document: DocumentId,
        parent: Option<StableNodeId>,
        park_roots: bool,
        build: impl FnOnce(&mut Self) -> R,
    ) -> Result<R, FrameworkError> {
        let mut builder = Self {
            context,
            document,
            stack: vec![Level {
                parent,
                seen: Vec::new(),
                autos: HashMap::new(),
            }],
            queue: MutationQueue::new(),
            working: HashMap::new(),
            pending_views: HashMap::new(),
            pending_ons: Vec::new(),
            pending_forget: HashSet::new(),
            lifecycle: Vec::new(),
            park_roots,
            error: None,
        };
        let result = build(&mut builder);
        builder.commit(result)
    }

    fn current(&self) -> &Level {
        self.stack.last().expect("builder always has a level")
    }

    fn current_mut(&mut self) -> &mut Level {
        self.stack.last_mut().expect("builder always has a level")
    }

    fn fail<C: View>(&mut self, error: FrameworkError) -> Entity<C> {
        if self.error.is_none() {
            self.error = Some(error);
        }
        Entity::from_stable_id(DUMMY_NODE)
    }

    fn auto_key(&mut self, kind: &'static str) -> String {
        let n = self.current_mut().autos.entry(kind).or_insert(0);
        let index = *n;
        *n += 1;
        format!("#{kind}-{index}")
    }

    fn spawn<C: ComponentView>(&mut self, component: C) -> Entity<C> {
        let id = self.context.allocate_id();
        self.queue.create(id, self.document, component.node_kind());
        component.project(id, &self.context.world, &mut self.queue);
        self.context.stamp_component_type::<C>(id, &mut self.queue);
        self.pending_views.insert(id, Box::new(component));
        self.lifecycle.push(id);
        Entity::from_stable_id(id)
    }

    fn nest_child<C: ComponentView, R>(
        &mut self,
        key: impl Into<String>,
        component: C,
        children: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let entity = self.child(key, component);
        self.nest(entity, children)
    }

    fn slots(&mut self, parent: StableNodeId) -> &mut HashMap<String, AssembledChild> {
        if !self.working.contains_key(&parent) {
            let inherited = self
                .context
                .assembled
                .get(&parent)
                .cloned()
                .unwrap_or_default();
            self.working.insert(parent, inherited);
        }
        self.working
            .get_mut(&parent)
            .expect("working slots inserted")
    }

    /// Create or reuse a keyed component under the current parent.
    pub fn child<C: ComponentView>(&mut self, key: impl Into<String>, component: C) -> Entity<C> {
        if self.error.is_some() {
            return Entity::from_stable_id(DUMMY_NODE);
        }
        let key = key.into();
        if key.is_empty() {
            return self.fail(FrameworkError::InvalidInput);
        }
        if self.current().seen.iter().any(|seen| seen == &key) {
            let parent = self.current().parent.unwrap_or(DUMMY_NODE);
            return self.fail(FrameworkError::DuplicateAssemblyKey { parent, key });
        }
        self.current_mut().seen.push(key.clone());
        let type_id = TypeId::of::<C>();
        if let Some(parent) = self.current().parent
            && let Some(existing) = self.slots(parent).get(&key).copied()
        {
            if existing.type_id == type_id {
                component.project(existing.id, &self.context.world, &mut self.queue);
                self.context
                    .stamp_component_type::<C>(existing.id, &mut self.queue);
                self.pending_views.insert(existing.id, Box::new(component));
                self.lifecycle.push(existing.id);
                return Entity::from_stable_id(existing.id);
            }
            self.queue_despawn(existing.id);
        }
        let entity = self.spawn(component);
        if let Some(parent) = self.current().parent {
            self.queue.insert(parent, entity.id, None);
            self.slots(parent).insert(
                key,
                AssembledChild {
                    id: entity.id,
                    type_id,
                },
            );
        } else if self.park_roots {
            self.queue.park_subtree(entity.id);
        }
        entity
    }

    /// Register an event handler after the tree batch commits.
    pub fn on<V, E>(
        &mut self,
        entity: Entity<V>,
        handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) where
        V: View,
        E: Send + 'static,
    {
        if self.error.is_some() || entity.id == DUMMY_NODE {
            return;
        }
        self.pending_ons
            .push(Box::new(move |cx| cx.on(entity, handler)));
    }

    /// Create a parked node that is not inserted under the current parent.
    ///
    /// Use with [`Self::adopt`] when a parent constructor needs the child's id
    /// (slots, shell regions) before the child can live under that parent.
    pub fn leaf<C: ComponentView>(&mut self, component: C) -> Entity<C> {
        if self.error.is_some() {
            return Entity::from_stable_id(DUMMY_NODE);
        }
        let entity = self.spawn(component);
        self.queue.park_subtree(entity.id);
        entity
    }

    /// Insert an existing node under the current parent.
    pub fn adopt<C: View>(&mut self, child: Entity<C>) {
        if self.error.is_some() || child.id == DUMMY_NODE {
            return;
        }
        let Some(parent) = self.current().parent else {
            self.fail::<C>(FrameworkError::InvalidInput);
            return;
        };
        let key = self.auto_key("adopt");
        self.current_mut().seen.push(key.clone());
        self.queue.insert(parent, child.id, None);
        self.slots(parent).insert(
            key,
            AssembledChild {
                id: child.id,
                type_id: TypeId::of::<C>(),
            },
        );
    }

    /// Temporarily set `parent` as the current insertion parent.
    pub fn nest<P: View, R>(
        &mut self,
        parent: Entity<P>,
        children: impl FnOnce(&mut Self) -> R,
    ) -> R {
        if self.error.is_some() || parent.id == DUMMY_NODE {
            return children(self);
        }
        self.stack.push(Level {
            parent: Some(parent.id),
            seen: Vec::new(),
            autos: HashMap::new(),
        });
        let result = children(self);
        self.finish_level();
        self.stack.pop();
        result
    }

    /// Keyed container: create/reuse `component`, then nest children. Returns
    /// the nested closure's value.
    pub fn with<C: ComponentView, R>(
        &mut self,
        key: impl Into<String>,
        component: C,
        children: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.nest_child(key, component, children)
    }

    pub fn column<R>(&mut self, gap: f32, children: impl FnOnce(&mut Self) -> R) -> R {
        let key = self.auto_key("column");
        self.nest_child(key, Stack::column(gap), children)
    }

    pub fn row<R>(&mut self, gap: f32, children: impl FnOnce(&mut Self) -> R) -> R {
        let key = self.auto_key("row");
        self.nest_child(key, Stack::row(gap), children)
    }

    fn queue_despawn(&mut self, root: StableNodeId) {
        if !self.context.world.contains(root) {
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.context.world.node(id) {
                stack.extend(node.children.iter().copied());
            }
            self.pending_forget.insert(id);
        }
        self.queue.despawn_subtree(root);
    }

    fn finish_level(&mut self) {
        if self.error.is_some() {
            return;
        }
        let Some(parent) = self.current().parent else {
            return;
        };
        let seen = self.current().seen.clone();
        let unused: Vec<_> = self
            .slots(parent)
            .iter()
            .filter(|(key, _)| !seen.iter().any(|seen| seen == *key))
            .map(|(_, child)| child.id)
            .collect();
        for id in unused {
            self.queue_despawn(id);
        }
        let forgotten = &self.pending_forget;
        self.working
            .get_mut(&parent)
            .expect("finish_level has working slots")
            .retain(|key, child| {
                seen.iter().any(|seen| seen == key) && !forgotten.contains(&child.id)
            });
        let desired: Vec<_> = seen
            .iter()
            .filter_map(|key| self.slots(parent).get(key).map(|child| child.id))
            .collect();
        let current = self
            .context
            .world
            .node(parent)
            .map(|node| node.children)
            .unwrap_or_default();
        let assembled: HashSet<_> = desired.iter().copied().collect();
        let mut ordered: Vec<_> = current
            .iter()
            .copied()
            .filter(|id| !assembled.contains(id) && !self.pending_forget.contains(id))
            .collect();
        ordered.extend(desired);
        if current.as_slice() == ordered.as_slice() {
            return;
        }
        for child in ordered {
            self.queue.insert(parent, child, None);
        }
    }

    fn commit<R>(mut self, result: R) -> Result<R, FrameworkError> {
        self.finish_level();
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let queue = self.queue;
        let pending_views = self.pending_views;
        let pending_ons = self.pending_ons;
        let pending_forget = self.pending_forget;
        let lifecycle = self.lifecycle;
        let working = self.working;
        if !queue.is_empty() {
            self.context.commit_mutations(queue)?;
        }
        if !pending_forget.is_empty() {
            self.context.forget_subtree(&pending_forget);
        }
        for (parent, slots) in working {
            if slots.is_empty() {
                self.context.assembled.remove(&parent);
            } else {
                self.context.assembled.insert(parent, slots);
            }
        }
        self.context.views.extend(pending_views);
        for id in lifecycle {
            if self.context.world.contains(id) {
                self.context.sync_component_lifecycle(id)?;
            }
        }
        for install in pending_ons {
            install(self.context)?;
        }
        Ok(result)
    }
}
