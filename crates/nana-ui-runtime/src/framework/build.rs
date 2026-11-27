use std::any::{Any, TypeId};

use hashbrown::{HashMap, HashSet};

use crate::{
    ComponentView, DocumentId, Entity, MutationQueue, StableNodeId, Stack, View, ViewContext,
};

use super::assemble::{AssembledChild, AssemblyKey, ParentAssembly};
use super::{AppContext, FrameworkError};

const DUMMY_NODE: StableNodeId = match StableNodeId::new(u64::MAX) {
    Some(id) => id,
    None => panic!("u64::MAX is a valid stable id"),
};

struct Level {
    parent: Option<StableNodeId>,
    /// Keys used at this level, in order.
    seen: Vec<AssemblyKey>,
    /// [`Self::seen`] as a set, for membership in wide levels.
    seen_keys: HashSet<AssemblyKey>,
    autos: HashMap<&'static str, usize>,
}

/// What [`UiBuilder::child`] found under its key.
enum ChildClaim {
    /// The build already failed, or the key is invalid or taken.
    Failed,
    /// The node the key named last time, of the same type.
    Reuse(StableNodeId),
    /// No such node: create one and attach it by this key.
    Create(AssemblyKey),
}

/// Nested tree builder that commits one mutation batch, then installs handlers.
pub(crate) struct UiBuilder<'a> {
    context: &'a mut AppContext,
    document: DocumentId,
    stack: Vec<Level>,
    queue: MutationQueue,
    working: HashMap<StableNodeId, ParentAssembly, crate::BuildIdHasher>,
    pending_views: crate::NodeMap<Box<dyn Any + Send>>,
    pending_stacks: Vec<(StableNodeId, u8)>,
    pending_ons: Vec<Box<dyn FnOnce(&mut AppContext) -> Result<(), FrameworkError>>>,
    /// How many `on` calls this build has already made for a given
    /// `(node, event type)`. The count is the handler's identity, so rebuilding
    /// the same tree replaces its handlers instead of stacking new ones.
    on_slots: HashMap<(StableNodeId, TypeId), usize>,
    pending_forget: std::collections::HashSet<StableNodeId>,
    /// Children this build's queue appends under each parent, in order.
    appended: HashMap<StableNodeId, Vec<StableNodeId>, crate::BuildIdHasher>,
    /// Nodes [`UiBuilder::adopt`] moved in this build.
    adopted: HashSet<StableNodeId, crate::BuildIdHasher>,
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
    pub(crate) fn build<R>(
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
    pub(crate) fn build_detached<R>(
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
    pub(crate) fn build_child<P: View, R>(
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
                seen_keys: HashSet::new(),
                autos: HashMap::new(),
            }],
            queue: MutationQueue::new(),
            working: HashMap::default(),
            pending_views: crate::NodeMap::default(),
            pending_stacks: Vec::new(),
            pending_ons: Vec::new(),
            on_slots: HashMap::new(),
            pending_forget: std::collections::HashSet::new(),
            appended: HashMap::default(),
            adopted: HashSet::default(),
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

    pub(crate) fn failed(&self) -> bool {
        self.error.is_some()
    }

    pub(crate) fn media_env(&self) -> nana_ui_core::MediaEnv {
        self.context.world().media_env()
    }

    /// The batch this build commits, for what a view sets on a node it just
    /// placed beside the node's own projection: a locale scope, there before
    /// the children it holds are inserted.
    pub(crate) fn mutations(&mut self) -> &mut MutationQueue {
        &mut self.queue
    }

    pub(crate) fn fail<C: View>(&mut self, error: FrameworkError) -> Entity<C> {
        if self.error.is_none() {
            self.error = Some(error);
        }
        Entity::from_stable_id(DUMMY_NODE)
    }

    fn auto_key(&mut self, kind: &'static str) -> AssemblyKey {
        let n = self.current_mut().autos.entry(kind).or_insert(0);
        let index = *n;
        *n += 1;
        AssemblyKey::auto_kind(kind, index)
    }

    fn install_component<C: ComponentView>(&mut self, id: StableNodeId, component: C) {
        let view_type = super::registry::ViewType::of::<C>();
        if TypeId::of::<C>() == TypeId::of::<Stack>() {
            // SAFETY: `TypeId` matched `Stack`.
            let stack = unsafe { &*std::ptr::from_ref(&component).cast::<Stack>() };
            self.context
                .stamp_component_type_erased(id, &mut self.queue, &view_type);
            self.pending_stacks.push((id, stack.elided_flags()));
            self.pending_views.remove(&id);
        } else {
            self.install_view(id, &view_type, Box::new(component));
        }
    }

    fn builder_view_type_id(&self, id: StableNodeId) -> Option<TypeId> {
        if let Some(view) = self.pending_views.get(&id) {
            return Some(view.as_ref().type_id());
        }
        if self.pending_stacks.iter().rev().any(|&(sid, _)| sid == id) {
            return Some(TypeId::of::<Stack>());
        }
        self.context.view_type_id(id)
    }

    fn spawn<C: ComponentView>(&mut self, mut component: C) -> Entity<C> {
        self.context.share_layouts(&mut component);
        let id = self.context.allocate_id();
        self.queue.create(id, self.document, component.node_kind());
        self.context
            .project_component(id, &component, &self.context.world, &mut self.queue);
        self.install_component(id, component);
        Entity::from_stable_id(id)
    }

    /// Stamp the type of the node `id` and keep its projected view for the
    /// commit. Takes the view erased: what a component type costs a build
    /// is its own projection, not a copy of this bookkeeping.
    #[inline(never)]
    fn install_view(
        &mut self,
        id: StableNodeId,
        view: &super::registry::ViewType,
        component: Box<dyn Any + Send>,
    ) {
        self.context
            .stamp_component_type_erased(id, &mut self.queue, view);
        self.pending_views.insert(id, component);
        self.lifecycle.push(id);
    }

    fn slots(&mut self, parent: StableNodeId) -> &mut ParentAssembly {
        if !self.working.contains_key(&parent) {
            let inherited = self
                .context
                .store
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

    /// Key `root`, a top-level node of this detached build, under `parent`,
    /// the existing node it will be placed in: what [`Self::child`] records
    /// for a node built under its parent. `parent`'s other keys stay.
    pub(crate) fn at_root_level(&self) -> bool {
        self.stack.len() == 1
    }

    pub(crate) fn key_parked_root(
        &mut self,
        parent: StableNodeId,
        key: AssemblyKey,
        root: StableNodeId,
    ) {
        if self.error.is_some() || self.stack.len() != 1 || self.current().parent.is_some() {
            return;
        }
        self.slots(parent).insert(key, AssembledChild { id: root });
    }

    /// Create or reuse a keyed component under the current parent.
    pub(crate) fn child<C: ComponentView>(
        &mut self,
        key: impl Into<AssemblyKey>,
        component: C,
    ) -> Entity<C> {
        let type_id = TypeId::of::<C>();
        match self.claim_child(key.into(), type_id) {
            ChildClaim::Failed => Entity::from_stable_id(DUMMY_NODE),
            ChildClaim::Reuse(id) => {
                self.context.project_component(
                    id,
                    &component,
                    &self.context.world,
                    &mut self.queue,
                );
                self.install_component(id, component);
                Entity::from_stable_id(id)
            }
            ChildClaim::Create(key) => {
                let entity = self.spawn(component);
                self.attach_child(key, entity.id);
                entity
            }
        }
    }

    /// The keyed half of [`Self::child`] that comes before the component is
    /// projected: the key is checked and marked seen, and the node it named
    /// in the last build is either reused or queued for removal.
    #[inline(never)]
    fn claim_child(&mut self, key: AssemblyKey, type_id: TypeId) -> ChildClaim {
        if self.error.is_some() {
            return ChildClaim::Failed;
        }
        if !super::valid_assembly_key(&key) {
            self.fail::<Stack>(FrameworkError::InvalidInput);
            return ChildClaim::Failed;
        }
        if !self.current_mut().seen_keys.insert(key.clone()) {
            let parent = self.current().parent.unwrap_or(DUMMY_NODE);
            self.fail::<Stack>(FrameworkError::DuplicateAssemblyKey {
                parent,
                key: key.to_string(),
            });
            return ChildClaim::Failed;
        }
        self.current_mut().seen.push(key.clone());
        if let Some(parent) = self.current().parent
            && let Some(existing) = self.slots(parent).get(key.as_str()).copied()
        {
            if self.builder_view_type_id(existing.id) == Some(type_id) {
                return ChildClaim::Reuse(existing.id);
            }
            self.queue_despawn(existing.id);
        }
        ChildClaim::Create(key)
    }

    /// The keyed half of [`Self::child`] that follows a new node: it goes
    /// under the current parent by `key`, or is parked as a detached root.
    #[inline(never)]
    fn attach_child(&mut self, key: AssemblyKey, id: StableNodeId) {
        if let Some(parent) = self.current().parent {
            self.queue.insert(parent, id, None);
            self.appended.entry(parent).or_default().push(id);
            self.slots(parent).insert(key, AssembledChild { id });
        } else if self.park_roots {
            self.queue.park_subtree(id);
        }
    }

    /// Register an event handler after the tree batch commits.
    ///
    /// Registration is idempotent across rebuilds: handlers are keyed by the
    /// position of this call among the `on` calls this build makes for the same
    /// `(node, event type)`, so building the same tree again replaces each
    /// handler rather than appending a second copy that would fire twice.
    /// Registering several handlers for one node and event in a single build
    /// still keeps all of them — they take successive slots.
    pub(crate) fn on<V, E>(
        &mut self,
        entity: Entity<V>,
        handler: impl FnMut(&mut V, &E, &mut ViewContext<'_, V>) + Send + 'static,
    ) where
        V: View,
        E: Send + 'static,
    {
        self.on_erased(entity.id, super::events::ErasedOn::new(handler));
    }

    /// [`Self::on`] for a handler erased where it was declared.
    #[inline(never)]
    pub(crate) fn on_erased(&mut self, id: StableNodeId, on: super::events::ErasedOn) {
        if self.error.is_some() || id == DUMMY_NODE {
            return;
        }
        let slot = {
            let seen = self.on_slots.entry((id, on.event())).or_insert(0);
            let slot = *seen;
            *seen += 1;
            slot
        };
        self.pending_ons.push(Box::new(move |cx| {
            cx.on_keyed_erased(id, format!("ui::on::{slot}"), on)
        }));
    }

    /// Run `install` on the context once the tree batch commits, with the
    /// event handlers: a declaration about `entity` that lives beside the
    /// tree, such as a roving focus group.
    pub(crate) fn after_commit<V: View>(
        &mut self,
        entity: Entity<V>,
        install: impl FnOnce(&mut AppContext, Entity<V>) -> Result<(), FrameworkError> + 'static,
    ) {
        if self.error.is_some() || entity.id == DUMMY_NODE {
            return;
        }
        self.pending_ons
            .push(Box::new(move |cx| install(cx, entity)));
    }

    /// A node this build leaves for someone else to place: a slot's content,
    /// whose id goes into the component that takes it.
    #[must_use = "detached nodes are not in the tree; hand the id to whatever \
                  places it, or it never renders"]
    pub(crate) fn detached<C: ComponentView>(&mut self, component: C) -> Entity<C> {
        if self.error.is_some() {
            return Entity::from_stable_id(DUMMY_NODE);
        }
        let entity = self.spawn(component);
        self.queue.park_subtree(entity.id);
        entity
    }

    /// Name `root`, a slot's root the composite places itself, by the key
    /// its view declared, for [`AppContext::assembly_path`].
    pub(crate) fn key_slot(&mut self, root: StableNodeId, key: AssemblyKey) {
        if self.error.is_some() || root == DUMMY_NODE {
            return;
        }
        self.context.store.slot_keys.insert(root, key);
    }

    /// Place `child`, built detached, under the current parent, keyed
    /// `key` there (the key its view declared) or by position.
    pub(crate) fn adopt_as(
        &mut self,
        child: StableNodeId,
        key: Option<std::borrow::Cow<'static, str>>,
    ) {
        if self.error.is_some() || child == DUMMY_NODE {
            return;
        }
        let Some(parent) = self.current().parent else {
            self.fail::<Stack>(FrameworkError::InvalidInput);
            return;
        };
        let key = match key {
            Some(key) => AssemblyKey::from(key),
            None => self.auto_key("adopt"),
        };
        if !self.current_mut().seen_keys.insert(key.clone()) {
            self.fail::<Stack>(FrameworkError::DuplicateAssemblyKey {
                parent,
                key: key.to_string(),
            });
            return;
        }
        self.current_mut().seen.push(key.clone());
        self.queue.insert(parent, child, None);
        self.appended.entry(parent).or_default().push(child);
        self.adopted.insert(child);
        self.slots(parent).insert(key, AssembledChild { id: child });
    }

    /// Temporarily set `parent` as the current insertion parent.
    pub(crate) fn nest<P: View, R>(
        &mut self,
        parent: Entity<P>,
        children: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let mut children = Some(children);
        let mut result = None;
        self.nest_dyn(parent.id, &mut |ui| {
            if let Some(children) = children.take() {
                result = Some(children(ui));
            }
        });
        result.expect("nest runs its children once")
    }

    /// [`Self::nest`] without generics: `children` runs exactly once.
    #[inline(never)]
    pub(crate) fn nest_dyn(&mut self, parent: StableNodeId, children: &mut dyn FnMut(&mut Self)) {
        let nested = self.enter_nest(parent);
        children(self);
        self.leave_nest(nested);
    }

    /// Push a nesting scope onto `self` without running `children` in a closure.
    pub(crate) fn enter_nest(&mut self, parent: StableNodeId) -> bool {
        let nested = self.error.is_none() && parent != DUMMY_NODE;
        if nested {
            self.stack.push(Level {
                parent: Some(parent),
                seen: Vec::new(),
                seen_keys: HashSet::new(),
                autos: HashMap::new(),
            });
        }
        nested
    }

    /// Close a nesting scope started by [`Self::enter_nest`].
    pub(crate) fn leave_nest(&mut self, nested: bool) {
        if nested {
            self.finish_level();
            self.stack.pop();
        }
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
        let seen_keys = std::mem::take(&mut self.current_mut().seen_keys);
        let unused: Vec<_> = self
            .slots(parent)
            .iter()
            .filter(|(key, _)| !seen_keys.contains(key.as_str()))
            .map(|(_, child)| child.id)
            .collect();
        for id in unused {
            self.queue_despawn(id);
        }
        let forgotten = &self.pending_forget;
        self.working
            .get_mut(&parent)
            .expect("finish_level has working slots")
            .retain(|key, child| seen_keys.contains(key) && !forgotten.contains(&child.id));
        // A child placed under another parent keeps its identity here but is
        // not pulled back.
        let slots = self.slots(parent);
        let desired: Vec<_> = seen
            .iter()
            .filter_map(|key| slots.get(key.as_str()).map(|child| child.id))
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|&id| !self.context.is_placed_elsewhere(id))
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
        // The queue already appends this build's new children, so the order
        // it leaves is usually the one wanted; move only what it leaves out
        // of place. Reinserting every child instead costs a sibling scan
        // each, quadratic in a wide level.
        let appended = self.appended.remove(&parent).unwrap_or_default();
        let forgotten = &self.pending_forget;
        let queued: Vec<_> = current
            .iter()
            .chain(&appended)
            .copied()
            .filter(|id| !forgotten.contains(id))
            .collect();
        let exact =
            queued.len() == ordered.len() && !queued.iter().any(|id| self.adopted.contains(id));
        if !exact {
            for child in ordered {
                self.queue.insert(parent, child, None);
            }
            return;
        }
        if queued != ordered {
            super::assemble::move_into_order(parent, &queued, &ordered, &mut self.queue);
        }
    }

    fn commit<R>(mut self, result: R) -> Result<R, FrameworkError> {
        self.finish_level();
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let queue = self.queue;
        let pending_views = self.pending_views;
        let pending_stacks = self.pending_stacks;
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
        for (parent, mut slots) in working {
            if !self.context.world.contains(parent) {
                continue;
            }
            slots.retain(|_, child| self.context.world.contains(child.id));
            slots.shrink_positional_excess();
            self.context.store_assembled(parent, slots);
        }
        for (id, flags) in pending_stacks {
            self.context.store.views.remove(&id);
            self.context.world.set_elided_stack_flags(id, flags);
        }
        self.context.store.views.extend(pending_views);
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
