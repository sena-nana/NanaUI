use std::any::TypeId;
use std::collections::HashSet;

use crate::{ComponentView, DocumentId, Entity, MutationQueue, StableNodeId, View};

use super::{AppContext, FrameworkError};

#[derive(Clone, Copy)]
pub(crate) struct AssembledChild {
    pub id: StableNodeId,
    pub type_id: TypeId,
}

/// Identity-stable child builder for one retained parent.
pub struct AssemblyScope<'a> {
    context: &'a mut AppContext,
    parent: StableNodeId,
    document: DocumentId,
    seen: Vec<String>,
}

impl AppContext {
    /// Publish the complete order of existing children without destroying omitted
    /// subtrees. Omitted children are parked and may be inserted again later.
    ///
    /// Unlike [`Self::mount`], this does not own keyed component construction or
    /// replace component values. Legal reparenting and parked parents are allowed.
    /// Invalid nodes, duplicate children, cycles and cross-document moves fail
    /// before any tree or component lifecycle change. An unchanged order is a no-op.
    pub fn reconcile_children(
        &mut self,
        parent: StableNodeId,
        ordered: &[StableNodeId],
    ) -> Result<bool, FrameworkError> {
        let current = self
            .world
            .node(parent)
            .ok_or(FrameworkError::MissingView(parent))?
            .children;
        if current.as_slice() == ordered {
            return Ok(false);
        }
        let mut seen = HashSet::with_capacity(ordered.len());
        for &child in ordered {
            if !self.world.contains(child) {
                return Err(FrameworkError::MissingView(child));
            }
            if child == parent || !seen.insert(child) {
                return Err(FrameworkError::InvalidComponentHierarchy { parent, child });
            }
        }
        let mut mutations = MutationQueue::new();
        if !reconcile_child_order(parent, ordered, &self.world, &mut mutations) {
            return Ok(false);
        }
        // commit_mutations validates the entire queue before changing either
        // the world or retained component lifecycles.
        self.commit_mutations(mutations)?;
        Ok(true)
    }

    /// Reconcile keyed children of `parent` without rebuilding the tree.
    pub fn mount<P: View>(
        &mut self,
        parent: Entity<P>,
        build: impl FnOnce(&mut AssemblyScope<'_>) -> Result<(), FrameworkError>,
    ) -> Result<(), FrameworkError> {
        self.read(parent, |_| ())?;
        let document = self
            .world
            .node(parent.id)
            .ok_or(FrameworkError::MissingView(parent.id))?
            .document;
        let mut scope = AssemblyScope {
            context: self,
            parent: parent.id,
            document,
            seen: Vec::new(),
        };
        build(&mut scope)?;
        scope.finish()
    }
}

impl AssemblyScope<'_> {
    pub fn child<C: ComponentView>(
        &mut self,
        key: impl Into<String>,
        component: C,
    ) -> Result<Entity<C>, FrameworkError> {
        self.upsert(key.into(), component)
    }

    pub fn with_child<C: ComponentView>(
        &mut self,
        key: impl Into<String>,
        component: C,
        children: impl FnOnce(&mut AssemblyScope<'_>) -> Result<(), FrameworkError>,
    ) -> Result<Entity<C>, FrameworkError> {
        let entity = self.upsert(key.into(), component)?;
        let document = self.document;
        let mut nested = AssemblyScope {
            context: self.context,
            parent: entity.id,
            document,
            seen: Vec::new(),
        };
        children(&mut nested)?;
        nested.finish()?;
        Ok(entity)
    }

    fn upsert<C: ComponentView>(
        &mut self,
        key: String,
        component: C,
    ) -> Result<Entity<C>, FrameworkError> {
        if key.is_empty() {
            return Err(FrameworkError::InvalidInput);
        }
        if self.seen.iter().any(|seen| seen == &key) {
            return Err(FrameworkError::DuplicateAssemblyKey {
                parent: self.parent,
                key,
            });
        }
        self.seen.push(key.clone());
        let type_id = TypeId::of::<C>();
        if let Some(existing) = self
            .context
            .assembled
            .get(&self.parent)
            .and_then(|slots| slots.get(&key))
            .copied()
        {
            if existing.type_id == type_id {
                let entity = Entity::from_stable_id(existing.id);
                self.context
                    .update_component(entity, |view: &mut C, _| view.reconcile(component))?;
                return Ok(entity);
            }
            self.context.despawn_node(existing.id)?;
        }
        let entity = self
            .context
            .create_detached_component(self.document, component)?;
        self.context.attach_child(self.parent, entity.id)?;
        self.context
            .assembled
            .entry(self.parent)
            .or_default()
            .insert(
                key,
                AssembledChild {
                    id: entity.id,
                    type_id,
                },
            );
        Ok(entity)
    }

    fn finish(self) -> Result<(), FrameworkError> {
        let unused: Vec<_> = self
            .context
            .assembled
            .get(&self.parent)
            .into_iter()
            .flatten()
            .filter(|(key, _)| !self.seen.iter().any(|seen| seen == *key))
            .map(|(_, child)| child.id)
            .collect();
        for id in unused {
            self.context.despawn_node(id)?;
        }
        if let Some(slots) = self.context.assembled.get_mut(&self.parent) {
            slots.retain(|key, _| self.seen.iter().any(|seen| seen == key));
        }
        let desired: Vec<_> = self
            .seen
            .iter()
            .filter_map(|key| {
                self.context
                    .assembled
                    .get(&self.parent)?
                    .get(key)
                    .map(|child| child.id)
            })
            .collect();
        let current = self
            .context
            .world
            .node(self.parent)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        let assembled: HashSet<_> = desired.iter().copied().collect();
        let mut ordered: Vec<_> = current
            .iter()
            .copied()
            .filter(|id| !assembled.contains(id))
            .collect();
        ordered.extend(desired);
        if current.as_slice() == ordered.as_slice() {
            return Ok(());
        }
        let mut mutations = MutationQueue::new();
        for child in ordered {
            mutations.insert(self.parent, child, None);
        }
        self.context.commit_mutations(mutations)?;
        Ok(())
    }
}

/// Shared projection path for framework components already building one batch.
/// Input validity is checked by that batch's normal Runtime transaction.
pub(crate) fn reconcile_child_order(
    parent: StableNodeId,
    ordered: &[StableNodeId],
    world: &crate::UiWorld,
    mutations: &mut MutationQueue,
) -> bool {
    let current = world
        .node(parent)
        .map(|node| node.children)
        .unwrap_or_default();
    if current.as_slice() == ordered {
        return false;
    }
    let keep = ordered.iter().copied().collect::<HashSet<_>>();
    // Extract retained descendants before parking their former ancestors. A
    // live-to-live reparent must never retire focus, IME or pointer ownership.
    for &child in ordered {
        mutations.insert(parent, child, None);
    }
    for child in &current {
        if !keep.contains(child) {
            mutations.park_subtree(*child);
        }
    }
    true
}
