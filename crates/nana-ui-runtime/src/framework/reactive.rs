//! The [`AppContext`] side of the declarative view layer: mounting, the
//! binding flush, and disposal with the nodes that own reactive state.

use super::*;
use crate::view::reactive::{self as rx, EffectKey, EffectTarget, ScopeKey};
use crate::view::{IntoView, NodePatch, StructuralBinding, ViewBuilder, ViewParts, ViewState};

/// Rounds a flush runs before it gives up on effects that keep re-queueing
/// each other.
const MAX_ROUNDS: usize = 64;

struct NodeEntry {
    effect: EffectKey,
    patch: Box<dyn NodePatch>,
}

struct StructuralEntry {
    effect: EffectKey,
    binding: Box<dyn StructuralBinding>,
}

pub(crate) struct ReactiveHost {
    tag: u64,
    nodes: HashMap<StableNodeId, NodeEntry>,
    structural: HashMap<StableNodeId, StructuralEntry>,
    anchors: HashMap<StableNodeId, ScopeKey>,
    queue: Vec<(EffectKey, EffectTarget)>,
    patches: Vec<(StableNodeId, EffectKey)>,
    flushing: bool,
}

impl Default for ReactiveHost {
    fn default() -> Self {
        Self {
            tag: rx::next_context_tag(),
            nodes: HashMap::new(),
            structural: HashMap::new(),
            anchors: HashMap::new(),
            queue: Vec::new(),
            patches: Vec::new(),
            flushing: false,
        }
    }
}

impl Drop for ReactiveHost {
    fn drop(&mut self) {
        let effects = self
            .nodes
            .values()
            .map(|entry| entry.effect)
            .chain(self.structural.values().map(|entry| entry.effect))
            .collect();
        let scopes = self.anchors.values().copied().collect();
        rx::release_on_drop(effects, scopes);
    }
}

/// A view mounted with [`AppContext::mount_view`]. Dropping the handle keeps
/// the view; [`Self::unmount`] or despawning its roots removes it.
#[derive(Debug, Clone)]
pub struct MountedView {
    roots: Vec<StableNodeId>,
    scope: ScopeKey,
}

impl MountedView {
    pub fn roots(&self) -> &[StableNodeId] {
        &self.roots
    }

    /// Dispose every signal and effect the view created and despawn it.
    pub fn unmount(self, cx: &mut AppContext) -> Result<(), FrameworkError> {
        rx::dispose_scope(self.scope);
        let mut mutations = MutationQueue::new();
        for root in self.roots {
            if cx.world.contains(root) {
                mutations.despawn_subtree(root);
            }
        }
        if mutations.is_empty() {
            return Ok(());
        }
        cx.commit_mutations(mutations).map(|_| ())
    }
}

/// The retained view of a bound node, for field-level comparison.
pub(crate) fn bound_view<C: ComponentView>(
    cx: &AppContext,
    id: StableNodeId,
) -> Result<&C, FrameworkError> {
    cx.views
        .get(&id)
        .ok_or(FrameworkError::MissingView(id))?
        .downcast_ref::<C>()
        .ok_or(FrameworkError::ViewType(id))
}

/// Apply a node's bindings to a staged copy of its view and project it.
/// `None` when the bound fields came out equal.
pub(crate) fn stage_bound_node<C: ComponentView>(
    cx: &AppContext,
    id: StableNodeId,
    mutations: &mut MutationQueue,
    apply: impl FnOnce(&mut C),
) -> Result<Option<Box<dyn Any + Send>>, FrameworkError> {
    let current = cx
        .views
        .get(&id)
        .ok_or(FrameworkError::MissingView(id))?
        .downcast_ref::<C>()
        .ok_or(FrameworkError::ViewType(id))?;
    let mut staged = current.clone();
    apply(&mut staged);
    cx.inherit_segmented_option_surface(id, &mut staged);
    if staged == *current {
        if !C::ALWAYS_REPROJECT {
            return Ok(None);
        }
        let mut probe = MutationQueue::new();
        staged.project(id, &cx.world, &mut probe);
        if cx.world.is_noop_batch(&probe) {
            return Ok(None);
        }
        mutations.append(probe);
        return Ok(Some(Box::new(staged)));
    }
    staged.project(id, &cx.world, mutations);
    Ok(Some(Box::new(staged)))
}

impl AppContext {
    /// Build `view` under `parent` in one commit and keep its bindings live.
    ///
    /// `view` runs inside the mount's scope, so the signals it and the
    /// components it calls create are disposed with the mount.
    pub fn mount_view<V: IntoView>(
        &mut self,
        parent: StableNodeId,
        view: impl FnOnce() -> V,
    ) -> Result<MountedView, FrameworkError> {
        let document = self
            .world
            .node(parent)
            .ok_or(FrameworkError::MissingView(parent))?
            .document;
        self.mount_view_in(document, Some(parent), view)
    }

    /// Like [`Self::mount_view`], with the view's roots as document roots.
    pub fn mount_view_root<V: IntoView>(
        &mut self,
        document: DocumentId,
        view: impl FnOnce() -> V,
    ) -> Result<MountedView, FrameworkError> {
        self.mount_view_in(document, None, view)
    }

    fn mount_view_in<V: IntoView>(
        &mut self,
        document: DocumentId,
        parent: Option<StableNodeId>,
        view: impl FnOnce() -> V,
    ) -> Result<MountedView, FrameworkError> {
        let tag = self.reactive.tag;
        let scope = rx::create_scope(rx::current_scope());
        let build = |ui: &mut UiBuilder<'_>| {
            let mut st = ViewState::new(tag);
            let roots = rx::with_scope(scope, || {
                let view = view();
                ViewBuilder { ui, st: &mut st }.build_collect(view)
            });
            (roots, st.parts)
        };
        // Under a parent the roots are inserted unkeyed afterwards, so the
        // parent's own keyed children are not reassembled.
        let built = match parent {
            Some(_) => self.build_detached(document, build),
            None => self.build(document, build),
        };
        let (roots, parts) = match built {
            Ok(built) => built,
            Err(error) => {
                rx::dispose_scope(scope);
                return Err(error);
            }
        };
        if let Some(parent) = parent {
            let mut insert = MutationQueue::new();
            for root in &roots {
                insert.insert(parent, *root, None);
            }
            if let Err(error) = self.commit_mutations(insert) {
                rx::dispose_scope(scope);
                let mut cleanup = MutationQueue::new();
                for root in &roots {
                    cleanup.despawn_subtree(*root);
                }
                self.commit_mutations(cleanup)?;
                return Err(error);
            }
        }
        self.install_view_parts(parts);
        for root in &roots {
            self.reactive.anchors.insert(*root, scope);
        }
        Ok(MountedView { roots, scope })
    }

    pub(crate) fn reactive_tag(&self) -> u64 {
        self.reactive.tag
    }

    pub(crate) fn install_view_parts(&mut self, parts: ViewParts) {
        for (id, effect, patch) in parts.nodes {
            if !self.world.contains(id) {
                rx::dispose_effect(effect);
                continue;
            }
            if let Some(old) = self.reactive.nodes.insert(id, NodeEntry { effect, patch }) {
                rx::dispose_effect(old.effect);
            }
        }
        for (id, effect, binding) in parts.structural {
            if !self.world.contains(id) {
                rx::dispose_effect(effect);
                continue;
            }
            if let Some(old) = self
                .reactive
                .structural
                .insert(id, StructuralEntry { effect, binding })
            {
                rx::dispose_effect(old.effect);
            }
        }
        for (id, scope) in parts.anchors {
            if let Some(old) = self.reactive.anchors.insert(id, scope)
                && old != scope
            {
                rx::dispose_scope(old);
            }
        }
    }

    /// Whether a signal write left bindings of this context to apply.
    pub fn has_pending_reactive(&self) -> bool {
        rx::has_pending(self.reactive.tag)
    }

    /// Apply every binding whose signals changed: run watchers, rebuild
    /// keyed lists and conditional blocks, then stage each changed node once
    /// and commit them together. Input routing and
    /// [`Self::take_system_work`] call this; call it yourself after writing
    /// signals outside both.
    pub fn flush_reactive(&mut self) -> Result<(), FrameworkError> {
        if self.reactive.flushing || !rx::has_pending(self.reactive.tag) {
            return Ok(());
        }
        self.reactive.flushing = true;
        let started = Instant::now();
        let outcome = self.flush_reactive_rounds();
        self.reactive.flushing = false;
        nana_diagnostics::metric!(nana_diagnostics::framework::runtime::REACTIVE_FLUSHES);
        nana_diagnostics::metric!(
            nana_diagnostics::framework::runtime::REACTIVE_FLUSH_NS,
            started.elapsed()
        );
        outcome
    }

    fn flush_reactive_rounds(&mut self) -> Result<(), FrameworkError> {
        let tag = self.reactive.tag;
        let mut queue = std::mem::take(&mut self.reactive.queue);
        let mut patches = std::mem::take(&mut self.reactive.patches);
        let mut outcome = Ok(());
        let (mut patched, mut commits) = (0, 0);
        for _ in 0..MAX_ROUNDS {
            queue.clear();
            rx::take_pending(tag, &mut queue);
            if queue.is_empty() {
                break;
            }
            #[cfg(feature = "reactive-trace")]
            let round = rx::with_trace(|trace| trace.begin_round());
            #[cfg(not(feature = "reactive-trace"))]
            let round = 0;
            for &(effect, target) in &queue {
                if target == EffectTarget::User {
                    rx::run_user_effect(effect);
                }
            }
            for &(effect, target) in &queue {
                if let EffectTarget::Structural(node) = target {
                    outcome = outcome.and(self.run_structural(node, effect));
                }
            }
            patches.clear();
            patches.extend(queue.iter().filter_map(|&(effect, target)| match target {
                EffectTarget::Node(node) => Some((node, effect)),
                _ => None,
            }));
            if !patches.is_empty() {
                match self.patch_bound_nodes(&patches, round) {
                    Ok((nodes, commit)) => {
                        patched += nodes;
                        commits += commit;
                    }
                    Err(error) => outcome = outcome.and(Err(error)),
                }
            }
        }
        if rx::has_pending(tag) {
            nana_diagnostics::fault!(
                nana_diagnostics::framework::runtime::REACTIVE_DID_NOT_SETTLE,
                rounds = MAX_ROUNDS as u64
            );
            rx::drop_pending(tag);
        }
        rx::record_flush(patched, commits);
        self.reactive.queue = queue;
        self.reactive.patches = patches;
        outcome
    }

    fn run_structural(
        &mut self,
        node: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let Some(mut entry) = self.reactive.structural.remove(&node) else {
            return Ok(());
        };
        if entry.effect != effect || !self.world.contains(node) {
            self.keep_structural(node, entry);
            return Ok(());
        }
        let outcome = entry.binding.update(self, node, effect);
        self.keep_structural(node, entry);
        outcome
    }

    fn keep_structural(&mut self, node: StableNodeId, entry: StructuralEntry) {
        if self.world.contains(node) {
            self.reactive.structural.insert(node, entry);
        } else {
            rx::dispose_effect(entry.effect);
        }
    }

    /// Stage every queued node against the current world, commit the lot
    /// once, then install the staged views and follow their lifecycles.
    fn patch_bound_nodes(
        &mut self,
        patches: &[(StableNodeId, EffectKey)],
        #[allow(unused_variables)] round: u64,
    ) -> Result<(u64, u64), FrameworkError> {
        let mut mutations = MutationQueue::new();
        let mut staged: Vec<(StableNodeId, Box<dyn Any + Send>)> = Vec::new();
        let mut outcome = Ok(());
        for &(node, effect) in patches {
            let Some(mut entry) = self.reactive.nodes.remove(&node) else {
                continue;
            };
            if entry.effect != effect || !self.world.contains(node) {
                if self.world.contains(node) {
                    self.reactive.nodes.insert(node, entry);
                }
                continue;
            }
            let result = rx::run_tracked(effect, || entry.patch.stage(self, node, &mut mutations));
            self.reactive.nodes.insert(node, entry);
            match result {
                Ok(Some(view)) => {
                    #[cfg(feature = "reactive-trace")]
                    rx::with_trace(|trace| trace.patch(round, node, effect));
                    staged.push((node, view));
                }
                Ok(None) => {}
                Err(error) => outcome = outcome.and(Err(error)),
            }
        }
        let mut commits = 0;
        if !mutations.is_empty() {
            self.commit_mutations(mutations)?;
            commits = 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::REACTIVE_COMMITS);
        }
        let patched = staged.len() as u64;
        if patched > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::REACTIVE_NODES_PATCHED,
                patched
            );
        }
        let mut changed = Vec::with_capacity(staged.len());
        for (node, view) in staged {
            changed.push((node, Any::type_id(&*view)));
            self.views.insert(node, view);
        }
        for (node, type_id) in changed {
            if !self.world.contains(node) {
                continue;
            }
            if !self.world.is_mounted(node) {
                outcome = outcome.and(self.suspend(node));
            }
            outcome = outcome.and(self.sync_component_lifecycle(node));
            outcome = outcome.and(self.run_component_assembler(node, type_id));
        }
        outcome.map(|()| (patched, commits))
    }

    /// Release the reactive state owned by despawned nodes.
    pub(crate) fn forget_reactive(&mut self, removed: &HashSet<StableNodeId>) {
        let host = &mut self.reactive;
        if host.nodes.is_empty() && host.structural.is_empty() && host.anchors.is_empty() {
            return;
        }
        for id in removed {
            if let Some(entry) = host.nodes.remove(id) {
                rx::dispose_effect(entry.effect);
            }
            if let Some(entry) = host.structural.remove(id) {
                rx::dispose_effect(entry.effect);
            }
            if let Some(scope) = host.anchors.remove(id) {
                rx::dispose_scope(scope);
            }
        }
    }

    /// The bindings `node` carries: where its element was declared and each
    /// bound field with where its binding was declared. `None` for a node
    /// without bindings.
    pub fn view_bindings(&self, node: StableNodeId) -> Option<crate::view::NodeBindingInfo> {
        let entry = self.reactive.nodes.get(&node)?;
        Some(crate::view::NodeBindingInfo {
            element: rx::effect_site(entry.effect)?,
            fields: entry.patch.fields(),
        })
    }

    /// Why `node` was last patched: its element, its bound fields, and the
    /// signal writes that queued it. `None` when the trace holds no patch of
    /// it (it has no bindings, never changed, or the ring moved on).
    #[cfg(feature = "reactive-trace")]
    pub fn why_updated(&self, node: StableNodeId) -> Option<crate::view::WhyUpdated> {
        let causes = rx::with_trace(|trace| trace.why(node))?;
        let info = self.view_bindings(node)?;
        Some(crate::view::WhyUpdated {
            node,
            element: info.element,
            bindings: info.fields,
            causes,
        })
    }
}

#[cfg(test)]
impl AppContext {
    pub(crate) fn event_handler_count(&self) -> usize {
        self.event_handlers.values().map(Vec::len).sum()
    }
}
