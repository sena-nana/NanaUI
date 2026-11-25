//! The [`AppContext`] side of the declarative view layer: mounting, the
//! binding flush, and disposal with the nodes that own reactive state.

use super::*;
use crate::view::reactive::{self as rx, EffectKey, EffectTarget, FragmentId, ScopeKey};
use crate::view::{
    CompiledMotionValue, FragmentBinding, InteractiveStyleBinding, KeyframeAnimation,
    MotionBinding, Mount, NodePatch, PSEUDO_ACTIVE, PSEUDO_FOCUS, PSEUDO_FOCUS_VISIBLE,
    PSEUDO_HOVER, PSEUDO_SELECTED, Refs, SlotItem, StructuralBinding, ViewBuilder, ViewParts,
    ViewState,
};
use crate::{
    AnimatableProperty, AnimationFillMode, AnimationId, AnimationIteration, AnimationPlayState,
    AnimationPlayback, Easing, MotionTo, MotionValue,
};

type PostFlushFn = fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>;

/// Rounds a flush runs before it gives up on effects that keep re-queueing
/// each other.
const MAX_ROUNDS: usize = 64;

struct NodeEntry {
    effect: EffectKey,
    patch: Box<dyn NodePatch>,
    source: Option<&'static crate::view::ViewSource>,
}

struct StructuralEntry {
    effect: EffectKey,
    binding: Box<dyn StructuralBinding>,
}

struct FragmentEntry {
    host: StableNodeId,
    parent: Option<FragmentId>,
    boxed: bool,
    effect: EffectKey,
    items: Vec<SlotItem>,
    leaving: Vec<StableNodeId>,
    mount_scope: Option<ScopeKey>,
    binding: Option<Box<dyn FragmentBinding>>,
}

struct Leaving {
    animation: AnimationId,
    moves: Option<(Duration, Easing)>,
}

/// A node that slides when layout moves it ([`crate::view::El::moves`]).
struct LayoutMove {
    duration: Duration,
    /// Where the last layout put it in its parent, while it stood in the
    /// flow with a size.
    placed: Option<Placed>,
}

#[derive(Clone, Copy, PartialEq)]
struct Placed {
    parent: StableNodeId,
    offset: [f32; 2],
}

struct PendingFlip {
    node: StableNodeId,
    first: nana_ui_core::FlipRect,
    duration: Duration,
    easing: Easing,
}

#[derive(Clone)]
struct RunningKeyframeTrack {
    id: AnimationId,
    spec: KeyframeAnimation,
    palette_epoch: u64,
    reduced_motion: bool,
}

pub(crate) struct ReactiveHost {
    tag: u64,
    nodes: HashMap<StableNodeId, NodeEntry, crate::BuildIdHasher>,
    structural: HashMap<StableNodeId, StructuralEntry, crate::BuildIdHasher>,
    fragments: HashMap<FragmentId, FragmentEntry>,
    host_slots: HashMap<StableNodeId, Vec<SlotItem>, crate::BuildIdHasher>,
    /// Fragments currently holding live nodes outside their declaration `host`
    /// (KeepAlive holder or active Teleport target), checked in O(1) before
    /// despawn commits.
    out_of_host_fragments: HashSet<FragmentId>,
    anchors: HashMap<StableNodeId, ScopeKey, crate::BuildIdHasher>,
    queue: Vec<(EffectKey, EffectTarget)>,
    patches: Vec<(StableNodeId, EffectKey)>,
    flushing: bool,
    /// Removed rows and branches playing their leave, despawned when it
    /// ends.
    leaving: HashMap<StableNodeId, Leaving, crate::BuildIdHasher>,
    /// Rows to slide from where they were once layout has placed them.
    flips: Vec<PendingFlip>,
    /// Properties of a node that animate when its bindings change them.
    implicit: HashMap<StableNodeId, Box<[crate::view::Implicit]>, crate::BuildIdHasher>,
    /// Nodes that slide when a layout pass moves them within their parent.
    layout_moves: HashMap<StableNodeId, LayoutMove, crate::BuildIdHasher>,
    /// Interactive pseudo-state signals (`:hover`, `:focus`, `:focus-visible`, `:active`, `:selected`).
    interactive_styles: HashMap<StableNodeId, InteractiveStyleBinding, crate::BuildIdHasher>,
    /// Nodes in `interactive_styles` whose pseudo-state mask is currently non-zero.
    active_interactive_nodes: HashSet<StableNodeId, crate::BuildIdHasher>,
    /// Nodes in `interactive_styles` that subscribe to `:selected`.
    selected_interactive_nodes: HashSet<StableNodeId, crate::BuildIdHasher>,
    /// Documents that have mounted interactive-style bindings.
    interactive_documents: Vec<DocumentId>,
    /// Nodes carrying compiled CSS dynamic transitions / `@keyframes`.
    motion_bindings: HashMap<StableNodeId, MotionBinding, crate::BuildIdHasher>,
    /// Currently registered `#87 MotionTrack` keyframe tracks per node.
    keyframe_tracks: HashMap<StableNodeId, Vec<RunningKeyframeTrack>, crate::BuildIdHasher>,
    /// Deduplicated hooks run once a reactive flush has installed every node it patched.
    post_flush_hooks: Vec<(StableNodeId, PostFlushFn)>,
}

impl Default for ReactiveHost {
    fn default() -> Self {
        Self {
            tag: rx::next_context_tag(),
            nodes: HashMap::default(),
            structural: HashMap::default(),
            fragments: HashMap::default(),
            host_slots: HashMap::default(),
            out_of_host_fragments: HashSet::default(),
            anchors: HashMap::default(),
            queue: Vec::new(),
            patches: Vec::new(),
            flushing: false,
            leaving: HashMap::default(),
            flips: Vec::new(),
            implicit: HashMap::default(),
            layout_moves: HashMap::default(),
            interactive_styles: HashMap::default(),
            active_interactive_nodes: HashSet::default(),
            selected_interactive_nodes: HashSet::default(),
            interactive_documents: Vec::new(),
            motion_bindings: HashMap::default(),
            keyframe_tracks: HashMap::default(),
            post_flush_hooks: Vec::new(),
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
            .chain(self.fragments.values().map(|entry| entry.effect))
            .collect();
        let scopes = self
            .anchors
            .values()
            .copied()
            .chain(
                self.fragments
                    .values()
                    .filter_map(|entry| entry.mount_scope),
            )
            .collect();
        rx::release_on_drop(effects, scopes);
    }
}

/// A view mounted with [`AppContext::mount_view`]. Dropping the handle keeps
/// the view; [`Self::unmount`] or despawning its roots removes it.
#[derive(Debug, Clone)]
pub struct MountedView {
    roots: Vec<StableNodeId>,
    slots: Vec<SlotItem>,
    host: Option<StableNodeId>,
    scope: ScopeKey,
}

impl MountedView {
    pub fn roots(&self) -> &[StableNodeId] {
        &self.roots
    }

    /// The first root as an entity of type `C`, for a view with one root
    /// the caller knows the type of.
    pub fn root<C: View>(&self) -> Option<Entity<C>> {
        self.roots.first().copied().map(Entity::from_stable_id)
    }

    /// Dispose every signal and effect the view created and despawn it.
    pub fn unmount(self, cx: &mut AppContext) -> Result<(), FrameworkError> {
        rx::dispose_scope(self.scope);
        let has_fragments = self
            .slots
            .iter()
            .any(|item| matches!(item, SlotItem::Fragment(_)));
        let mut visible = Vec::new();
        let mut immediate = Vec::new();
        cx.teardown_slot_items(&self.slots, &mut visible, &mut immediate);
        let mut mutations = MutationQueue::new();
        let mut seen = crate::NodeSet::default();
        for id in visible.into_iter().chain(immediate).chain(self.roots) {
            if seen.insert(id) && cx.world.contains(id) {
                mutations.despawn_subtree(id);
            }
        }
        if !mutations.is_empty() {
            cx.commit_mutations(mutations)?;
        }
        if has_fragments
            && let Some(host) = self.host
            && cx.world.contains(host)
        {
            cx.notify_host_slot_assembler(host)?;
        }
        Ok(())
    }
}

/// The retained view of a bound node, for field-level comparison.
pub(crate) fn bound_view<C: ComponentView>(
    cx: &AppContext,
    id: StableNodeId,
) -> Result<std::borrow::Cow<'_, C>, FrameworkError> {
    if let Some(view) = cx.store.views.get(&id) {
        return view
            .downcast_ref::<C>()
            .map(std::borrow::Cow::Borrowed)
            .ok_or(FrameworkError::ViewType(id));
    }
    if TypeId::of::<C>() == TypeId::of::<crate::Stack>()
        && let Some(stack) = cx.world.elided_stack(id)
    {
        // SAFETY: `TypeId` matched `Stack`.
        let cloned = unsafe { &*std::ptr::from_ref(&stack).cast::<C>() }.clone();
        return Ok(std::borrow::Cow::Owned(cloned));
    }
    if cx.world.is_elided_stack(id) {
        return Err(FrameworkError::ViewType(id));
    }
    Err(FrameworkError::MissingView(id))
}

/// Apply a node's bindings to a staged copy of its view and project it.
/// `None` when the bound fields came out equal.
pub(crate) fn stage_bound_node<C: ComponentView>(
    cx: &AppContext,
    id: StableNodeId,
    mutations: &mut MutationQueue,
    apply: impl FnOnce(&mut C),
) -> Result<Option<Box<dyn Any + Send>>, FrameworkError> {
    let current = bound_view::<C>(cx, id)?;
    let mut staged: C = (*current).clone();
    apply(&mut staged);
    cx.inherit_segmented_option_surface(id, &mut staged);
    if staged == *current {
        if !C::ALWAYS_REPROJECT {
            return Ok(None);
        }
        let mut probe = MutationQueue::new();
        cx.project_component(id, &staged, &cx.world, &mut probe);
        if cx.world.is_noop_batch(&probe) {
            return Ok(None);
        }
        mutations.append(probe);
        return Ok(Some(Box::new(staged)));
    }
    cx.project_component(id, &staged, &cx.world, mutations);
    Ok(Some(Box::new(staged)))
}

impl AppContext {
    /// Build `view` under `parent` in one commit and keep its bindings live.
    ///
    /// `view` runs inside the mount's scope, so the signals it and the
    /// components it calls create are disposed with the mount.
    pub fn mount_view<M: Mount>(
        &mut self,
        parent: StableNodeId,
        view: impl FnOnce() -> M,
    ) -> Result<M::Output, FrameworkError> {
        let document = self
            .world
            .node(parent)
            .ok_or(FrameworkError::MissingView(parent))?
            .document;
        self.mount_view_in(document, Some(parent), None, view, true)
    }

    /// Like [`Self::mount_view`], with the view's roots placed before
    /// `before`, a child of `parent`, instead of after the last child.
    pub fn mount_view_before<M: Mount>(
        &mut self,
        parent: StableNodeId,
        before: StableNodeId,
        view: impl FnOnce() -> M,
    ) -> Result<M::Output, FrameworkError> {
        let document = self
            .world
            .node(parent)
            .ok_or(FrameworkError::MissingView(parent))?
            .document;
        if self.world.node(before).and_then(|node| node.parent) != Some(parent) {
            return Err(FrameworkError::InvalidInput);
        }
        self.mount_view_in(document, Some(parent), Some(before), view, true)
    }

    /// Like [`Self::mount_view`], with the view's roots as document roots.
    pub fn mount_view_root<M: Mount>(
        &mut self,
        document: DocumentId,
        view: impl FnOnce() -> M,
    ) -> Result<M::Output, FrameworkError> {
        self.mount_view_in(document, None, None, view, false)
    }

    /// Like [`Self::mount_view_root`], with the roots parked: in no tree
    /// until something places them (a composite's slot given by id, a
    /// later `reconcile_children`).
    pub fn mount_view_detached<M: Mount>(
        &mut self,
        document: DocumentId,
        view: impl FnOnce() -> M,
    ) -> Result<M::Output, FrameworkError> {
        self.mount_view_in(document, None, None, view, true)
    }

    fn mount_view_in<M: Mount>(
        &mut self,
        document: DocumentId,
        parent: Option<StableNodeId>,
        before: Option<StableNodeId>,
        view: impl FnOnce() -> M,
        park: bool,
    ) -> Result<M::Output, FrameworkError> {
        let tag = self.reactive.tag;
        let scope = rx::create_scope(rx::current_scope());
        let mut refs = None;
        let build = |ui: &mut UiBuilder<'_>| {
            let mut st = ViewState::new_in(tag, parent, None);
            let (roots, slots) = rx::with_scope(scope, || {
                let (view, made) = view().split();
                refs = Some(made);
                ViewBuilder { ui, st: &mut st }.build_collect_slots(view)
            });
            ((roots, slots), st.parts)
        };
        // Under a parent the roots are inserted unkeyed afterwards, so the
        // parent's own keyed children are not reassembled.
        let built = if park {
            self.build_detached(document, build)
        } else {
            self.build(document, build)
        };
        let ((roots, slots), parts) = match built {
            Ok(built) => built,
            Err(error) => {
                rx::dispose_scope(scope);
                return Err(error);
            }
        };
        let has_top_fragments = slots
            .iter()
            .any(|item| matches!(item, SlotItem::Fragment(_)));
        let seed_slots = parent
            .filter(|p| has_top_fragments || self.reactive.host_slots.contains_key(p))
            .filter(|p| !self.reactive.host_slots.contains_key(p))
            .map(|p| {
                self.world
                    .node(p)
                    .map(|node| node.children.to_vec())
                    .unwrap_or_default()
            });
        if let Some(parent) = parent {
            let mut insert = MutationQueue::new();
            for root in &roots {
                insert.insert(parent, *root, before);
            }
            if !insert.is_empty()
                && let Err(error) = self.commit_mutations(insert)
            {
                rx::dispose_scope(scope);
                let mut cleanup = MutationQueue::new();
                for root in &roots {
                    cleanup.despawn_subtree(*root);
                }
                self.commit_mutations(cleanup)?;
                return Err(error);
            }
            if has_top_fragments || self.reactive.host_slots.contains_key(&parent) {
                let before_pos = before.and_then(|target| {
                    let list = self.reactive.host_slots.get(&parent)?;
                    list.iter().position(|item| match *item {
                        SlotItem::Node(id) => id == target,
                        SlotItem::Fragment(frag) => self.fragment_contains_node(frag, target),
                    })
                });
                let list = self.reactive.host_slots.entry(parent).or_insert_with(|| {
                    seed_slots
                        .unwrap_or_default()
                        .into_iter()
                        .map(SlotItem::Node)
                        .collect()
                });
                let pos = before_pos.unwrap_or_else(|| {
                    before
                        .and_then(|target| {
                            list.iter().position(|item| *item == SlotItem::Node(target))
                        })
                        .unwrap_or(list.len())
                });
                list.splice(pos..pos, slots.iter().copied());
            }
        }
        if let Err(error) = self.install_view_parts(parts) {
            // The tree stands; what failed is placing its slots.
            let mounted = MountedView {
                roots,
                slots,
                host: parent,
                scope,
            };
            mounted.unmount(self)?;
            return Err(error);
        }
        for item in &slots {
            match *item {
                SlotItem::Node(root) => {
                    self.reactive.anchors.insert(root, scope);
                }
                SlotItem::Fragment(frag) => {
                    if let Some(entry) = self.reactive.fragments.get_mut(&frag) {
                        entry.mount_scope = Some(scope);
                    }
                }
            }
        }
        rx::run_mounted(self);
        let mounted = MountedView {
            roots,
            slots,
            host: parent,
            scope,
        };
        // Every ref names an element that was built, or the caller asked
        // for a node that is not there.
        match refs.and_then(Refs::resolve) {
            Some(refs) => Ok(M::finish(mounted, refs)),
            None => {
                mounted.unmount(self)?;
                Err(FrameworkError::InvalidInput)
            }
        }
    }

    pub(crate) fn reactive_tag(&self) -> u64 {
        self.reactive.tag
    }

    /// Take over what a build left and assemble the slots of the composites
    /// it built, innermost first.
    pub(crate) fn install_view_parts(
        &mut self,
        mut parts: ViewParts,
    ) -> Result<(), FrameworkError> {
        for (id, effect, patch, source) in parts.nodes {
            if !self.world.contains(id) {
                rx::dispose_effect(effect);
                continue;
            }
            if let Some(old) = self.reactive.nodes.insert(
                id,
                NodeEntry {
                    effect,
                    patch,
                    source,
                },
            ) {
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
        for (host, slots) in parts.host_slots {
            if self.world.contains(host) {
                self.reactive
                    .host_slots
                    .entry(host)
                    .or_default()
                    .extend(slots);
            }
        }
        for mut decl in parts.fragments {
            if !self.world.contains(decl.host) {
                rx::dispose_effect(decl.effect);
                decl.binding.forget(self);
                continue;
            }
            self.reactive.fragments.insert(
                decl.id,
                FragmentEntry {
                    host: decl.host,
                    parent: decl.parent,
                    boxed: decl.boxed,
                    effect: decl.effect,
                    items: decl.items,
                    leaving: Vec::new(),
                    mount_scope: None,
                    binding: Some(decl.binding),
                },
            );
        }
        for (id, implicit) in parts.implicit {
            if self.world.contains(id) {
                self.reactive.implicit.insert(id, implicit);
            }
        }
        for (id, duration) in parts.moves {
            if self.world.contains(id) {
                self.reactive.layout_moves.insert(
                    id,
                    LayoutMove {
                        duration,
                        placed: None,
                    },
                );
            }
        }
        let mut mounted_interactive = Vec::with_capacity(parts.interactive.len());
        for (id, interactive) in parts.interactive {
            if self.world.contains(id) {
                if interactive.used_pseudos & PSEUDO_SELECTED != 0 {
                    self.reactive.selected_interactive_nodes.insert(id);
                } else {
                    self.reactive.selected_interactive_nodes.remove(&id);
                }
                self.reactive.interactive_styles.insert(id, interactive);
                if let Some(doc) = self.world.document_of(id)
                    && !self.reactive.interactive_documents.contains(&doc)
                {
                    self.reactive.interactive_documents.push(doc);
                }
                mounted_interactive.push(Some(id));
            }
        }
        for (id, motion) in parts.motion {
            if self.world.contains(id) {
                self.reactive.motion_bindings.insert(id, motion);
            }
        }
        for (id, scope) in parts.anchors {
            if let Some(old) = self.reactive.anchors.insert(id, scope)
                && old != scope
            {
                rx::dispose_scope(old);
            }
        }
        for (id, type_id) in std::mem::take(&mut parts.assemble) {
            self.run_built_assembler(id, type_id)?;
        }
        self.sync_interactive_style_nodes(&mounted_interactive);
        self.sync_keyframe_animations(false)?;
        Ok(())
    }

    /// Whether a signal write left bindings of this context to apply.
    pub fn has_pending_reactive(&self) -> bool {
        rx::has_pending(self.reactive.tag)
    }

    /// Apply every binding whose signals changed: rebuild keyed lists and
    /// conditional blocks (outermost scopes first), run watchers, then stage
    /// each changed node once and commit them together. Input routing and
    /// [`Self::take_system_work`] call this; call it yourself after writing
    /// signals outside both.
    pub fn flush_reactive(&mut self) -> Result<(), FrameworkError> {
        self.sync_interactive_style_states();
        rx::advance_epoch();
        if self.reactive.flushing || !rx::has_pending(self.reactive.tag) {
            return Ok(());
        }
        self.reactive.flushing = true;
        let started = Instant::now();
        let outcome = self.flush_reactive_rounds();
        self.reactive.flushing = false;
        let outcome = outcome.and(self.run_post_flush_hooks());
        nana_diagnostics::metric!(nana_diagnostics::framework::runtime::REACTIVE_FLUSHES);
        nana_diagnostics::metric!(
            nana_diagnostics::framework::runtime::REACTIVE_FLUSH_NS,
            started.elapsed()
        );
        outcome
    }

    fn flush_reactive_rounds(&mut self) -> Result<(), FrameworkError> {
        #[derive(Clone, Copy)]
        enum StructuralWork {
            Node(StableNodeId),
            Fragment(FragmentId),
        }

        let tag = self.reactive.tag;
        let mut queue = std::mem::take(&mut self.reactive.queue);
        let mut patches = std::mem::take(&mut self.reactive.patches);
        let mut structural: Vec<(u32, EffectKey, StructuralWork)> = Vec::new();
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
            // Rebuild structural blocks outermost-first before confirming user
            // effects or node computeds inside rows that are about to unmount.
            structural.clear();
            structural.extend(queue.iter().filter_map(|&(effect, target)| match target {
                EffectTarget::Structural(node) => Some((
                    rx::effect_scope_depth(effect),
                    effect,
                    StructuralWork::Node(node),
                )),
                EffectTarget::Fragment(frag) => Some((
                    rx::effect_scope_depth(effect),
                    effect,
                    StructuralWork::Fragment(frag),
                )),
                _ => None,
            }));
            if structural.len() > 1 {
                structural.sort_by_key(|&(depth, _, _)| depth);
            }
            for &(_, effect, work) in &structural {
                if rx::confirm(effect) {
                    let step = match work {
                        StructuralWork::Node(node) => self.run_structural(node, effect),
                        StructuralWork::Fragment(frag) => self.run_fragment(frag, effect),
                    };
                    outcome = outcome.and(step);
                }
            }
            // Rows and branches are placed now.
            rx::run_mounted(self);
            for &(effect, target) in &queue {
                if target == EffectTarget::User && rx::confirm(effect) {
                    rx::run_user_effect(effect);
                }
            }
            patches.clear();
            patches.extend(queue.iter().filter_map(|&(effect, target)| match target {
                EffectTarget::Node(node) if rx::confirm(effect) => Some((node, effect)),
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

    /// Whether a settings group must wait to mark its rows: during a flush a
    /// While a flush is patching nodes, a control's view can lag the world
    /// until the patched views are installed; queues `(node, hook)` deduplicated
    /// to run once the flush completes.
    pub(crate) fn defer_post_flush(&mut self, node: StableNodeId, hook: PostFlushFn) -> bool {
        if !self.reactive.flushing {
            return false;
        }
        if !self
            .reactive
            .post_flush_hooks
            .iter()
            .any(|&(id, f)| id == node && f as usize == hook as usize)
        {
            self.reactive.post_flush_hooks.push((node, hook));
        }
        true
    }

    /// Whether a flush is patching nodes now; see [`Self::defer_post_flush`].
    pub(crate) fn reactive_flushing(&self) -> bool {
        self.reactive.flushing
    }

    pub(crate) fn run_post_flush_hooks(&mut self) -> Result<(), FrameworkError> {
        let mut outcome = Ok(());
        for (node, hook) in std::mem::take(&mut self.reactive.post_flush_hooks) {
            if self.world.contains(node) {
                outcome = outcome.and(hook(self, node));
            }
        }
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
        if entry.effect != effect || !self.world.contains(node) || !rx::effect_alive(effect) {
            self.keep_structural(node, entry);
            return Ok(());
        }
        let outcome = entry.binding.update(self, node, effect);
        self.keep_structural(node, entry);
        outcome
    }

    /// Run the binding kept at `node` now, as its effect would: one made in
    /// a view that may name a node built after it (`El::labelled_by`).
    pub(crate) fn run_structural_now(&mut self, node: StableNodeId) -> Result<(), FrameworkError> {
        let Some(effect) = self
            .reactive
            .structural
            .get(&node)
            .map(|entry| entry.effect)
        else {
            return Ok(());
        };
        self.run_structural(node, effect)
    }

    fn keep_structural(&mut self, node: StableNodeId, entry: StructuralEntry) {
        if self.world.contains(node) {
            self.reactive.structural.insert(node, entry);
        } else {
            rx::dispose_effect(entry.effect);
        }
    }

    fn run_fragment(&mut self, frag: FragmentId, effect: EffectKey) -> Result<(), FrameworkError> {
        let Some(entry) = self.reactive.fragments.get_mut(&frag) else {
            return Ok(());
        };
        if entry.effect != effect || !self.world.contains(entry.host) || !rx::effect_alive(effect) {
            return Ok(());
        }
        let Some(mut binding) = entry.binding.take() else {
            return Ok(());
        };
        let outcome = binding.update(self, frag, effect);
        if let Some(entry) = self.reactive.fragments.get_mut(&frag)
            && self.world.contains(entry.host)
        {
            entry.binding = Some(binding);
        } else {
            rx::dispose_effect(effect);
            binding.forget(self);
            self.reactive.fragments.remove(&frag);
            self.reactive.out_of_host_fragments.remove(&frag);
        }
        outcome
    }

    pub(crate) fn run_fragment_now(&mut self, frag: FragmentId) -> Result<(), FrameworkError> {
        let Some(effect) = self.reactive.fragments.get(&frag).map(|entry| entry.effect) else {
            return Ok(());
        };
        self.run_fragment(frag, effect)
    }

    pub(crate) fn fragment_host(&self, frag: FragmentId) -> Option<StableNodeId> {
        self.reactive.fragments.get(&frag).map(|entry| entry.host)
    }

    pub(crate) fn fragment_boxed(&self, frag: FragmentId) -> bool {
        self.reactive
            .fragments
            .get(&frag)
            .is_some_and(|entry| entry.boxed)
    }

    pub(crate) fn fragment_has_mount_scope(&self, frag: FragmentId) -> bool {
        let mut cursor = Some(frag);
        while let Some(id) = cursor {
            let Some(entry) = self.reactive.fragments.get(&id) else {
                return false;
            };
            if entry.mount_scope.is_some() {
                return true;
            }
            if entry.boxed {
                return false;
            }
            cursor = entry.parent;
        }
        false
    }

    pub(crate) fn set_fragment_items(
        &mut self,
        frag: FragmentId,
        items: Vec<SlotItem>,
        leaving: Vec<StableNodeId>,
    ) {
        if let Some(entry) = self.reactive.fragments.get_mut(&frag) {
            entry.items = items;
            entry.leaving = leaving;
        }
    }

    pub(crate) fn mark_out_of_host_fragment(&mut self, frag: FragmentId, active: bool) {
        if active {
            self.reactive.out_of_host_fragments.insert(frag);
        } else {
            self.reactive.out_of_host_fragments.remove(&frag);
        }
    }

    fn fragment_contains_node(&self, frag: FragmentId, target: StableNodeId) -> bool {
        let Some(entry) = self.reactive.fragments.get(&frag) else {
            return false;
        };
        if entry.leaving.contains(&target) {
            return true;
        }
        for item in &entry.items {
            match *item {
                SlotItem::Node(id) if id == target => return true,
                SlotItem::Fragment(child) if self.fragment_contains_node(child, target) => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    pub(crate) fn flatten_slot_items(
        &self,
        items: &[SlotItem],
        target_host: StableNodeId,
        out: &mut Vec<StableNodeId>,
    ) {
        for item in items {
            match *item {
                SlotItem::Node(id) => {
                    if self.world.contains(id) {
                        out.push(id);
                    }
                }
                SlotItem::Fragment(frag) => {
                    let Some(entry) = self.reactive.fragments.get(&frag) else {
                        continue;
                    };
                    if entry.host != target_host
                        || entry
                            .binding
                            .as_ref()
                            .and_then(|binding| binding.placed_target())
                            .is_some()
                    {
                        continue;
                    }
                    self.flatten_slot_items(&entry.items, target_host, out);
                }
            }
        }
    }

    pub(crate) fn flatten_all_live_nodes(&self, items: &[SlotItem], out: &mut Vec<StableNodeId>) {
        for item in items {
            match *item {
                SlotItem::Node(id) => {
                    if self.world.contains(id) {
                        out.push(id);
                    }
                }
                SlotItem::Fragment(frag) => {
                    if let Some(entry) = self.reactive.fragments.get(&frag) {
                        self.flatten_all_live_nodes(&entry.items, out);
                    }
                }
            }
        }
    }

    fn edge_live_node_in_slot(
        &self,
        item: SlotItem,
        target_host: StableNodeId,
        forward: bool,
    ) -> Option<StableNodeId> {
        match item {
            SlotItem::Node(id) => (self.world.parent_id(id) == Some(target_host)).then_some(id),
            SlotItem::Fragment(frag) => {
                let entry = self.reactive.fragments.get(&frag)?;
                if entry.host != target_host
                    || entry
                        .binding
                        .as_ref()
                        .and_then(|binding| binding.placed_target())
                        .is_some()
                {
                    return None;
                }
                let item_edge =
                    if forward {
                        entry.items.iter().find_map(|child| {
                            self.edge_live_node_in_slot(*child, target_host, true)
                        })
                    } else {
                        entry.items.iter().rev().find_map(|child| {
                            self.edge_live_node_in_slot(*child, target_host, false)
                        })
                    };
                if entry.leaving.is_empty() {
                    return item_edge;
                }
                let live_leaving: Vec<StableNodeId> = entry
                    .leaving
                    .iter()
                    .copied()
                    .filter(|&id| self.world.parent_id(id) == Some(target_host))
                    .collect();
                if live_leaving.is_empty() {
                    return item_edge;
                }
                let children = self.world.node(target_host).map(|node| node.children)?;
                let best_leaving = live_leaving
                    .into_iter()
                    .filter_map(|id| children.iter().position(|&c| c == id).map(|pos| (pos, id)))
                    .min_by_key(|&(pos, _)| if forward { pos } else { usize::MAX - pos });
                match (item_edge, best_leaving) {
                    (Some(edge), Some((l_pos, l_id))) => {
                        let e_pos = children.iter().position(|&c| c == edge);
                        match e_pos {
                            Some(e_pos)
                                if (forward && e_pos <= l_pos) || (!forward && e_pos >= l_pos) =>
                            {
                                Some(edge)
                            }
                            _ => Some(l_id),
                        }
                    }
                    (Some(edge), None) => Some(edge),
                    (None, Some((_, l_id))) => Some(l_id),
                    (None, None) => None,
                }
            }
        }
    }

    fn anchor_for_fragment(&self, frag: FragmentId, forward: bool) -> Option<StableNodeId> {
        let mut cur = frag;
        let target_host = self.reactive.fragments.get(&cur)?.host;
        loop {
            let entry = self.reactive.fragments.get(&cur)?;
            let siblings: &[SlotItem] = match entry.parent {
                Some(parent) => {
                    let parent_entry = self.reactive.fragments.get(&parent)?;
                    if parent_entry.host != target_host {
                        return None;
                    }
                    &parent_entry.items
                }
                None => self.reactive.host_slots.get(&target_host)?.as_slice(),
            };
            let pos = siblings
                .iter()
                .position(|item| *item == SlotItem::Fragment(cur))?;
            if forward {
                for &sibling in &siblings[pos + 1..] {
                    if let Some(node) = self.edge_live_node_in_slot(sibling, target_host, true) {
                        return Some(node);
                    }
                }
            } else {
                for &sibling in siblings[..pos].iter().rev() {
                    if let Some(node) = self.edge_live_node_in_slot(sibling, target_host, false) {
                        return Some(node);
                    }
                }
            }
            match entry.parent {
                Some(parent)
                    if self
                        .reactive
                        .fragments
                        .get(&parent)
                        .is_some_and(|p| p.host == target_host) =>
                {
                    cur = parent;
                }
                _ => return None,
            }
        }
    }

    pub(crate) fn next_live_anchor_after(&self, frag: FragmentId) -> Option<StableNodeId> {
        self.anchor_for_fragment(frag, true)
    }

    pub(crate) fn prev_live_anchor_before(&self, frag: FragmentId) -> Option<StableNodeId> {
        self.anchor_for_fragment(frag, false)
    }

    pub(crate) fn splice_fragment_children(
        &mut self,
        frag: FragmentId,
        desired: &[StableNodeId],
    ) -> Result<(), FrameworkError> {
        let Some((host, boxed)) = self
            .reactive
            .fragments
            .get(&frag)
            .map(|entry| (entry.host, entry.boxed))
        else {
            return Ok(());
        };
        if !self.world.contains(host) {
            return Ok(());
        }
        if boxed {
            return self.reconcile_children(host, desired).map(|_| ());
        }
        let mut seen = crate::NodeSet::default();
        let desired: Vec<StableNodeId> = desired
            .iter()
            .copied()
            .filter(|&id| self.world.contains(id) && seen.insert(id))
            .collect();
        let current = self
            .world
            .node(host)
            .map(|node| node.children.to_vec())
            .unwrap_or_default();
        let prev = self.prev_live_anchor_before(frag);
        let next = self.next_live_anchor_after(frag);
        let mut lo = prev
            .and_then(|p| current.iter().position(|&c| c == p))
            .map_or(0, |i| i + 1);
        let hi = next
            .and_then(|n| current.iter().position(|&c| c == n))
            .unwrap_or(current.len());
        if lo > hi {
            lo = hi;
        }
        let slice = &current[lo..hi];
        if slice == desired.as_slice() {
            return self.notify_host_slot_assembler(host);
        }
        let positions = crate::world::child_order::positions_in(slice, &desired);
        let stays = crate::world::child_order::longest_increasing(&positions);
        let moves = stays.iter().filter(|stays| !**stays).count();
        if desired.len() == slice.len()
            && slice.iter().all(|id| seen.contains(id))
            && crate::framework::assemble::reorder_is_cheaper(moves, current.len(), slice.len())
        {
            let mut full = Vec::with_capacity(current.len());
            full.extend_from_slice(&current[..lo]);
            full.extend_from_slice(&desired);
            full.extend_from_slice(&current[hi..]);
            let mut mutations = MutationQueue::new();
            mutations.reorder_children(host, full);
            self.commit_mutations(mutations)?;
            return self.notify_host_slot_assembler(host);
        }
        let mut mutations = MutationQueue::new();
        let mut anchor = current[hi..].iter().copied().find(|id| !seen.contains(id));
        for (index, &child) in desired.iter().enumerate().rev() {
            if !stays[index] {
                mutations.insert(host, child, anchor);
            }
            anchor = Some(child);
        }
        if !mutations.is_empty() {
            self.commit_mutations(mutations)?;
        }
        self.notify_host_slot_assembler(host)
    }

    pub(crate) fn notify_host_slot_assembler(
        &mut self,
        host: StableNodeId,
    ) -> Result<(), FrameworkError> {
        if let Some(type_id) = self.view_type_id(host) {
            self.run_slot_assembler(host, type_id)?;
        }
        Ok(())
    }

    pub(crate) fn teleport_place_nodes(
        &mut self,
        frag: FragmentId,
        nodes: &[StableNodeId],
        target: Option<StableNodeId>,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        let Some(host) = self.fragment_host(frag) else {
            return Ok(None);
        };
        let host_doc = self.world.document_of(host);
        let valid_target = target.filter(|&target| {
            self.world.contains(target)
                && target != host
                && self.world.document_of(target) == host_doc
                && !nodes
                    .iter()
                    .any(|&node| self.world.is_descendant_or_self(target, node))
        });
        if let Some(target) = valid_target {
            let mut mutations = MutationQueue::new();
            for &node in nodes {
                if !self.world.contains(node) {
                    continue;
                }
                if self.declared_assembly_parent(node).is_some() {
                    self.promote_auto_view_slot(node);
                    self.store.placed_assembled.insert(node);
                }
                if self.world.parent_id(node) != Some(target) {
                    mutations.insert(target, node, None);
                }
            }
            if !mutations.is_empty() {
                self.commit_mutations(mutations)?;
            }
            return Ok(Some(target));
        }
        for &node in nodes {
            if self
                .declared_assembly_parent(node)
                .is_some_and(|(declared, _)| declared == host)
            {
                self.store.placed_assembled.remove(&node);
            }
        }
        self.splice_fragment_children(frag, nodes)?;
        Ok(None)
    }

    pub(crate) fn retarget_slot_items(
        &mut self,
        items: &[SlotItem],
        old_host: StableNodeId,
        new_host: StableNodeId,
    ) {
        if old_host == new_host {
            return;
        }
        for item in items {
            let SlotItem::Fragment(frag) = *item else {
                continue;
            };
            let nested = match self.reactive.fragments.get_mut(&frag) {
                Some(entry) if !entry.boxed && entry.host == old_host => {
                    entry.host = new_host;
                    entry.items.clone()
                }
                _ => continue,
            };
            self.retarget_slot_items(&nested, old_host, new_host);
        }
    }

    /// Move any boxless fragment declarations (and their sibling nodes already
    /// reparented into `new_host`) from `old_host` onto `new_host` (for
    /// composites like `SidebarSection` that adopt user children into a
    /// private body wrapper).
    pub(crate) fn retarget_fragments(&mut self, old_host: StableNodeId, new_host: StableNodeId) {
        if old_host == new_host {
            return;
        }
        let Some(old_slots) = self.reactive.host_slots.remove(&old_host) else {
            return;
        };
        let moved: Vec<SlotItem> = old_slots
            .into_iter()
            .filter(|item| match *item {
                SlotItem::Fragment(_) => true,
                SlotItem::Node(id) => self.world.parent_id(id) == Some(new_host),
            })
            .collect();
        if !moved
            .iter()
            .any(|item| matches!(item, SlotItem::Fragment(_)))
        {
            return;
        }
        self.retarget_slot_items(&moved, old_host, new_host);
        self.reactive
            .host_slots
            .entry(new_host)
            .or_default()
            .extend(moved);
    }

    pub(crate) fn teardown_slot_items(
        &mut self,
        items: &[SlotItem],
        visible_nodes: &mut Vec<StableNodeId>,
        immediate_despawn: &mut Vec<StableNodeId>,
    ) {
        for item in items {
            match *item {
                SlotItem::Node(id) => visible_nodes.push(id),
                SlotItem::Fragment(frag) => {
                    self.reactive.out_of_host_fragments.remove(&frag);
                    let Some(mut entry) = self.reactive.fragments.remove(&frag) else {
                        continue;
                    };
                    rx::dispose_effect(entry.effect);
                    if let Some(scope) = entry.mount_scope {
                        rx::dispose_scope(scope);
                    }
                    if let Some(mut binding) = entry.binding.take() {
                        binding.teardown(self, visible_nodes, immediate_despawn);
                    }
                    immediate_despawn.extend(entry.leaving);
                    if entry.parent.is_none()
                        && let Some(host_slots) = self.reactive.host_slots.get_mut(&entry.host)
                    {
                        host_slots.retain(|slot| *slot != SlotItem::Fragment(frag));
                        if host_slots.is_empty() {
                            self.reactive.host_slots.remove(&entry.host);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn forget_fragment(&mut self, frag: FragmentId) {
        self.reactive.out_of_host_fragments.remove(&frag);
        let Some(mut entry) = self.reactive.fragments.remove(&frag) else {
            return;
        };
        rx::dispose_effect(entry.effect);
        if let Some(scope) = entry.mount_scope {
            rx::dispose_scope(scope);
        }
        if let Some(mut binding) = entry.binding.take() {
            binding.forget(self);
        }
    }

    pub(crate) fn despawn_or_rescue_out_of_host_fragments(
        &mut self,
        mutations: &mut MutationQueue,
    ) -> Option<MutationQueue> {
        if self.reactive.out_of_host_fragments.is_empty() {
            return None;
        }
        let mut roots: HashSet<StableNodeId> = mutations
            .as_slice()
            .iter()
            .filter_map(|mutation| match mutation {
                crate::UiMutation::DespawnSubtree { root } => Some(*root),
                _ => None,
            })
            .collect();
        if roots.is_empty() {
            return None;
        }
        let out_of_host: Vec<FragmentId> = self
            .reactive
            .out_of_host_fragments
            .iter()
            .copied()
            .collect();
        let under = |cx: &Self, roots: &HashSet<StableNodeId>, id: StableNodeId| {
            std::iter::successors(Some(id), |id| cx.world.parent_id(*id))
                .any(|ancestor| roots.contains(&ancestor))
        };
        loop {
            let mut added = false;
            for &frag in &out_of_host {
                let Some(entry) = self.reactive.fragments.get(&frag) else {
                    continue;
                };
                if !under(self, &roots, entry.host) {
                    continue;
                }
                let mut ext = Vec::new();
                if let Some(binding) = &entry.binding {
                    binding.collect_external_nodes(self, &mut ext);
                }
                for node in ext {
                    if self.world.contains(node) && !under(self, &roots, node) {
                        roots.insert(node);
                        mutations.despawn_subtree(node);
                        added = true;
                    }
                }
            }
            if !added {
                break;
            }
        }
        let mut rescued = MutationQueue::new();
        for frag in out_of_host {
            let (host, target, items) = match self.reactive.fragments.get(&frag) {
                Some(entry) => {
                    let target = entry
                        .binding
                        .as_ref()
                        .and_then(|binding| binding.placed_target());
                    (entry.host, target, entry.items.clone())
                }
                None => continue,
            };
            let Some(target) = target else {
                continue;
            };
            if under(self, &roots, host) || !under(self, &roots, target) {
                continue;
            }
            if let Some(entry) = self.reactive.fragments.get_mut(&frag)
                && let Some(binding) = entry.binding.as_mut()
            {
                binding.clear_placed_target();
            }
            self.reactive.out_of_host_fragments.remove(&frag);
            self.retarget_slot_items(&items, target, host);
            let mut live = Vec::new();
            self.flatten_all_live_nodes(&items, &mut live);
            let before = self.next_live_anchor_after(frag);
            for node in live {
                if self
                    .world
                    .parent_id(node)
                    .is_some_and(|parent| under(self, &roots, parent))
                {
                    if self
                        .declared_assembly_parent(node)
                        .is_some_and(|(declared, _)| declared == host)
                    {
                        self.store.placed_assembled.remove(&node);
                    }
                    rescued.insert(host, node, before);
                }
            }
        }
        (!rescued.is_empty()).then_some(rescued)
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
            // Disposed earlier in this round: its scope is gone (a removed
            // branch playing its leave), and so are the signals it reads.
            if !rx::effect_alive(effect) {
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
        // What animated nodes show now, and hold logically, before the
        // commit moves their logical values.
        let now = self.component_lifecycle.now;
        let mut before = Vec::new();
        for (node, _) in &staged {
            let mut transitions: Vec<crate::view::Implicit> = Vec::new();
            if let Some(implicit) = self.reactive.implicit.get(node) {
                transitions.extend_from_slice(implicit);
            }
            if let Some(binding) = self.reactive.motion_bindings.get_mut(node) {
                let prev_mask = binding.prev_mask;
                let curr_mask = binding.current_mask();
                for item in binding.site.transitions_for(curr_mask) {
                    if let Some(existing) =
                        transitions.iter_mut().find(|t| t.property == item.property)
                    {
                        *existing = item;
                    } else {
                        transitions.push(item);
                    }
                }
                for item in binding.site.transitions_for(prev_mask) {
                    if !transitions.iter().any(|t| t.property == item.property) {
                        transitions.push(item);
                    }
                }
                binding.prev_mask = curr_mask;
            }
            for implicit in transitions {
                let Some(logical) = self
                    .world
                    .logical_motion_value(*node, implicit.property)
                    .or_else(|| {
                        (implicit.property == AnimatableProperty::Background)
                            .then_some(MotionValue::Color([0.0, 0.0, 0.0, 0.0]))
                    })
                else {
                    continue;
                };
                let shown = self
                    .world
                    .presentation_motion_value(*node, implicit.property, now)
                    .unwrap_or(logical);
                before.push((*node, implicit, logical, shown));
            }
        }
        if !mutations.is_empty() {
            self.commit_mutations(mutations)?;
            commits = 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::REACTIVE_COMMITS);
        }
        self.play_implicit(before, now)?;
        self.sync_keyframe_animations(false)?;
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
            self.store_component_view(node, view);
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
            outcome = outcome.and(self.run_slot_assembler(node, type_id));
        }
        outcome.map(|()| (patched, commits))
    }

    /// Release the reactive state owned by despawned nodes.
    pub(crate) fn forget_reactive(&mut self, removed: &HashSet<StableNodeId>) {
        let host = &mut self.reactive;
        if host.nodes.is_empty()
            && host.structural.is_empty()
            && host.anchors.is_empty()
            && host.leaving.is_empty()
            && host.implicit.is_empty()
            && host.layout_moves.is_empty()
            && host.interactive_styles.is_empty()
            && host.motion_bindings.is_empty()
            && host.keyframe_tracks.is_empty()
            && host.host_slots.is_empty()
        {
            return;
        }
        let mut frags_to_forget = Vec::new();
        for id in removed {
            let host = &mut self.reactive;
            host.leaving.remove(id);
            host.implicit.remove(id);
            host.layout_moves.remove(id);
            host.interactive_styles.remove(id);
            host.active_interactive_nodes.remove(id);
            host.selected_interactive_nodes.remove(id);
            host.motion_bindings.remove(id);
            host.keyframe_tracks.remove(id);
            if let Some(entry) = host.nodes.remove(id) {
                rx::dispose_effect(entry.effect);
            }
            if let Some(entry) = host.structural.remove(id) {
                rx::dispose_effect(entry.effect);
            }
            if let Some(scope) = host.anchors.remove(id) {
                rx::dispose_scope(scope);
            }
            if let Some(slots) = host.host_slots.remove(id) {
                for item in slots {
                    if let SlotItem::Fragment(frag) = item {
                        frags_to_forget.push(frag);
                    }
                }
            }
        }
        for frag in frags_to_forget {
            self.forget_fragment(frag);
        }
    }

    /// The bindings `node` carries: where its element was declared and each
    /// bound field with where its binding was declared. `None` for a node
    /// without bindings.
    pub fn view_bindings(&self, node: StableNodeId) -> Option<crate::view::NodeBindingInfo> {
        let entry = self.reactive.nodes.get(&node)?;
        let fields = entry.patch.fields();
        let source_fields = entry.source.map_or_else(Vec::new, |source| {
            fields
                .iter()
                .filter_map(|(field, _)| {
                    source
                        .fields
                        .iter()
                        .find(|(name, _)| {
                            *name == *field || field.rsplit('.').next() == Some(*name)
                        })
                        .map(|(_, at)| (*field, *at))
                })
                .collect()
        });
        Some(crate::view::NodeBindingInfo {
            element: rx::effect_site(entry.effect)?,
            fields,
            source: entry.source.map(|source| source.element),
            source_fields,
        })
    }

    pub(crate) fn view_is<C: 'static>(&self, node: StableNodeId) -> bool {
        (TypeId::of::<C>() == TypeId::of::<crate::Stack>() && self.world.is_elided_stack(node))
            || self
                .store
                .views
                .get(&node)
                .is_some_and(|view| view.is::<C>())
    }

    /// `node` as the view layer sees it: its control's fields and values
    /// and where its bindings were declared. `None` for a node that does not
    /// exist.
    pub fn inspect(&self, node: StableNodeId) -> Option<crate::view::Inspection> {
        if !self.world.contains(node) {
            return None;
        }
        let bindings = self.view_bindings(node);
        let bound = |name: &str| {
            bindings.as_ref().and_then(|info| {
                info.fields
                    .iter()
                    .find(|(field, _)| field.rsplit('.').next() == Some(name))
                    .map(|(_, at)| *at)
            })
        };
        let source_bound = |name: &str| {
            bindings.as_ref().and_then(|info| {
                info.source_fields
                    .iter()
                    .find(|(field, _)| field.rsplit('.').next() == Some(name))
                    .map(|(_, at)| *at)
            })
        };
        let (control, fields) = self
            .store
            .views
            .get(&node)
            .and_then(|view| crate::view::inspect_control(&**view))
            .or_else(|| {
                self.world
                    .elided_stack(node)
                    .and_then(|stack| crate::view::inspect_control(&stack))
            })
            .map_or((None, Vec::new()), |(tag, fields)| (Some(tag), fields));
        Some(crate::view::Inspection {
            node,
            control,
            fields: fields
                .into_iter()
                .map(|(name, value)| crate::view::InspectedField {
                    name,
                    value,
                    bound_at: bound(name),
                    source_bound_at: source_bound(name),
                })
                .collect(),
            element: bindings.as_ref().map(|info| info.element),
            source_element: bindings.as_ref().and_then(|info| info.source),
            layout: self.layout_cause(node),
            dynamic: self
                .world
                .document_of(node)
                .and_then(|document| self.layout.layout_cache.dynamic_inspection(document, node)),
            layout_node: self.inspect_layout_node(node),
        })
    }

    /// `node`'s layout view and the intrinsic generation of its content.
    pub fn inspect_layout_node(
        &self,
        node: StableNodeId,
    ) -> Option<crate::view::LayoutNodeInspection> {
        let view = self.world.layout_node(node)?;
        let metrics_generation = self.world.document_of(node).and_then(|document| {
            self.layout
                .layout_cache
                .intrinsic_generation(document, node)
        });
        Some(crate::view::LayoutNodeInspection {
            view,
            metrics_generation,
        })
    }

    /// Why layout last reached `node`: the cause queued for the next pass,
    /// or else the one the last pass of its document admitted it with.
    /// `None` when neither touched it.
    pub fn layout_cause(&self, node: StableNodeId) -> Option<crate::view::LayoutCause> {
        let pending = self.world.pending_layout_invalidation(node);
        if !pending.is_empty() {
            return Some(crate::view::LayoutCause {
                invalidation: pending,
                pending: true,
                seed: true,
            });
        }
        let frontier = self
            .layout
            .layout_cache
            .last_frontier(self.world.document_of(node)?)?;
        Some(crate::view::LayoutCause {
            invalidation: frontier.invalidation(node)?,
            pending: false,
            seed: frontier.is_seed(node),
        })
    }

    /// Set `field` of the built-in control at `node` from text, the way its
    /// binding would (devtools editing). Strings are taken as they are,
    /// numbers and `true` / `false` parsed; empty text clears an optional
    /// field. A bound field returns to its binding's value when that runs.
    pub fn set_field(&mut self, node: StableNodeId, field: &str, text: &str) -> Result<(), String> {
        crate::view::edit_control(self, node, field, text)
            .unwrap_or_else(|| Err(format!("node {} is not a built-in control", node.get())))
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
            source_element: info.source,
            source_bindings: info.source_fields,
            causes,
        })
    }
}

/// Enter, leave and move animations of rows and branches
/// ([`crate::view::Transition`]).
impl AppContext {
    fn flip_rect(&self, id: StableNodeId) -> Option<nana_ui_core::FlipRect> {
        let bounds = self.world.component_layout_box(id)?;
        Some(nana_ui_core::FlipRect::new(
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
        ))
    }

    /// Play `enter` on each of `roots`, just placed: from its opacity and
    /// transform to the node's own.
    pub(crate) fn begin_enter(
        &mut self,
        roots: &[StableNodeId],
        enter: &crate::view::Presence,
    ) -> Result<(), FrameworkError> {
        if !enter.plays() {
            return Ok(());
        }
        let duration = self.motion_duration(enter.duration);
        let now = self.component_lifecycle.now;
        let mut mutations = MutationQueue::new();
        for &root in roots {
            for (property, from) in presence_values(enter) {
                let Some(to) = self.world.logical_motion_value(root, property) else {
                    continue;
                };
                mutations.start_animation(crate::motion_api::presence_spec(
                    root,
                    property,
                    from,
                    to,
                    now,
                    duration,
                    self.transition_easing(presence_easing(enter, property)),
                    AnimationFillMode::Backwards,
                ));
            }
        }
        if mutations.is_empty() {
            return Ok(());
        }
        self.commit_mutations(mutations).map(|_| ())
    }

    /// Start `leave` on `root` instead of despawning it: it stays in place,
    /// out of hit testing and focus, until the leave ends. `false` when
    /// nothing plays, and the caller despawns it now.
    pub(crate) fn begin_leave(
        &mut self,
        root: StableNodeId,
        leave: &crate::view::Presence,
        moves: Option<(Duration, Option<Easing>)>,
    ) -> Result<bool, FrameworkError> {
        let moves = moves.map(|(duration, easing)| (duration, self.transition_easing(easing)));
        let Some(node) = self.world.node(root) else {
            return Ok(false);
        };
        if !leave.plays() || self.reactive.leaving.contains_key(&root) {
            return Ok(self.reactive.leaving.contains_key(&root));
        }
        let document = node.document;
        let duration = self.motion_duration(leave.duration);
        let now = self.component_lifecycle.now;
        let mut mutations = MutationQueue::new();
        let mut last = None;
        for (property, to) in presence_values(leave) {
            let Some(from) = self.world.logical_motion_value(root, property) else {
                continue;
            };
            let spec = crate::motion_api::presence_spec(
                root,
                property,
                from,
                to,
                now,
                duration,
                self.transition_easing(presence_easing(leave, property)),
                AnimationFillMode::Forwards,
            );
            last = Some(spec.id);
            mutations.start_animation(spec);
        }
        let Some(animation) = last else {
            return Ok(false);
        };
        if let Some(mut style) = self.world.node_style(root).cloned() {
            Arc::make_mut(&mut style.layout).pointer_events =
                Some(nana_ui_core::PointerEventsSpec::None);
            mutations.set_style(root, style);
        }
        if self
            .world
            .focused(document)
            .is_some_and(|focused| self.world.is_descendant_or_self(focused, root))
        {
            mutations.request_focus(document, None);
        }
        self.commit_mutations(mutations)?;
        self.reactive
            .leaving
            .insert(root, Leaving { animation, moves });
        Ok(true)
    }

    /// Whether `root` is playing its leave.
    pub(crate) fn is_leaving(&self, root: StableNodeId) -> bool {
        self.reactive.leaving.contains_key(&root)
    }

    /// Slide `node` from `first` to wherever the next layout puts it.
    pub(crate) fn flip_after_layout(
        &mut self,
        node: StableNodeId,
        first: nana_ui_core::FlipRect,
        (duration, easing): (Duration, Option<Easing>),
    ) {
        // Reduced motion: rows land where layout puts them.
        if self.reduced_motion() {
            return;
        }
        let easing = self.transition_easing(easing);
        self.reactive.flips.push(PendingFlip {
            node,
            first,
            duration,
            easing,
        });
    }

    /// The easing a transition plays with: its own, else the installed
    /// theme's standard easing.
    pub(crate) fn transition_easing(&self, easing: Option<Easing>) -> Easing {
        easing.unwrap_or_else(|| {
            self.world
                .theme()
                .motion()
                .easing(nana_ui_core::EasingRole::Standard)
        })
    }

    /// Animate each property whose logical value the commit changed, from
    /// what was shown to the new value.
    fn play_implicit(
        &mut self,
        before: Vec<(
            StableNodeId,
            crate::view::Implicit,
            MotionValue,
            MotionValue,
        )>,
        now: Duration,
    ) -> Result<(), FrameworkError> {
        if before.is_empty() {
            return Ok(());
        }
        let mut mutations = MutationQueue::new();
        for (node, implicit, logical, shown) in before {
            let Some(next) = self
                .world
                .logical_motion_value(node, implicit.property)
                .or_else(|| {
                    (implicit.property == AnimatableProperty::Background)
                        .then_some(MotionValue::Color([0.0, 0.0, 0.0, 0.0]))
                })
            else {
                continue;
            };
            if next == logical || (implicit.duration.is_zero() && implicit.delay.is_zero()) {
                continue;
            }
            // Reduced motion runs it for no time at all: the new value shows
            // at once and its Finished still arrives with the next advance.
            let duration = self.motion_duration(implicit.duration);
            let delay = self.motion_duration(implicit.delay);
            let fill_mode = if delay > Duration::ZERO {
                AnimationFillMode::Backwards
            } else {
                AnimationFillMode::None
            };
            let mut spec = crate::motion_api::presence_spec(
                node,
                implicit.property,
                shown,
                next,
                now,
                duration,
                self.transition_easing(implicit.easing),
                fill_mode,
            );
            if delay > Duration::ZERO {
                spec = spec.with_delay(delay);
            }
            mutations.start_animation(spec);
        }
        if mutations.is_empty() {
            return Ok(());
        }
        self.commit_mutations(mutations).map(|_| ())
    }

    fn sync_interactive_style_node(&mut self, node: StableNodeId) {
        let Some(binding) = self.reactive.interactive_styles.get(&node) else {
            return;
        };
        let Some(doc) = self.world.document_of(node) else {
            self.reactive.active_interactive_nodes.remove(&node);
            return;
        };
        if !self.reactive.interactive_documents.contains(&doc) {
            self.reactive.interactive_documents.push(doc);
        }
        let is_self_or_descendant = |mut cur: StableNodeId, ancestor: StableNodeId| loop {
            if cur == ancestor {
                return true;
            }
            let Some(parent) = self.world.node(cur).and_then(|node| node.parent) else {
                return false;
            };
            cur = parent;
        };
        let mut bits = 0u8;
        if binding.used_pseudos & PSEUDO_HOVER != 0
            && self
                .world
                .pointer_hover_targets(doc)
                .any(|target| is_self_or_descendant(target, node))
        {
            bits |= PSEUDO_HOVER;
        }
        if binding.used_pseudos & PSEUDO_ACTIVE != 0
            && self
                .world
                .pointer_press_targets(doc)
                .any(|target| is_self_or_descendant(target, node))
        {
            bits |= PSEUDO_ACTIVE;
        }
        if binding.used_pseudos & PSEUDO_FOCUS != 0 && self.world.focused(doc) == Some(node) {
            bits |= PSEUDO_FOCUS;
        }
        if binding.used_pseudos & PSEUDO_FOCUS_VISIBLE != 0
            && self.world.focus_visible(doc) == Some(node)
        {
            bits |= PSEUDO_FOCUS_VISIBLE;
        }
        if binding.used_pseudos & PSEUDO_SELECTED != 0
            && self.world.effective_selected(node) == Some(true)
        {
            bits |= PSEUDO_SELECTED;
        }
        if bits != 0 {
            self.reactive.active_interactive_nodes.insert(node);
        } else {
            self.reactive.active_interactive_nodes.remove(&node);
        }
        if binding.signal.get_untracked() != bits {
            binding.signal.set(bits);
        }
    }

    pub(crate) fn sync_interactive_style_ancestors(&mut self, targets: &[Option<StableNodeId>]) {
        if self.reactive.interactive_styles.is_empty() {
            return;
        }
        let mut seen = Vec::new();
        for &start in targets {
            let mut cur = start;
            while let Some(id) = cur {
                if self.reactive.interactive_styles.contains_key(&id) && !seen.contains(&id) {
                    seen.push(id);
                    self.sync_interactive_style_node(id);
                }
                cur = self.world.node(id).and_then(|node| node.parent);
            }
        }
    }

    pub(crate) fn sync_interactive_style_nodes(&mut self, targets: &[Option<StableNodeId>]) {
        if self.reactive.interactive_styles.is_empty() {
            return;
        }
        let mut seen = Vec::new();
        for &target in targets {
            if let Some(id) = target
                && self.reactive.interactive_styles.contains_key(&id)
                && !seen.contains(&id)
            {
                seen.push(id);
                self.sync_interactive_style_node(id);
            }
        }
    }

    pub(crate) fn sync_interactive_style_states(&mut self) {
        if self.reactive.interactive_styles.is_empty() {
            return;
        }
        let previous_active: Vec<StableNodeId> = self
            .reactive
            .active_interactive_nodes
            .iter()
            .copied()
            .collect();
        for id in previous_active {
            self.sync_interactive_style_node(id);
        }
        let mut docs: Vec<DocumentId> = self.reactive.interactive_documents.clone();
        for doc in self.world.focused_documents() {
            if !docs.contains(&doc) {
                docs.push(doc);
            }
        }
        let mut targets: Vec<Option<StableNodeId>> = Vec::new();
        for doc in docs {
            if !self.world.has_document(doc) {
                continue;
            }
            targets.extend(self.world.pointer_hover_targets(doc).map(Some));
            targets.extend(self.world.pointer_press_targets(doc).map(Some));
            if let Some(focused) = self.world.focused(doc) {
                targets.push(Some(focused));
            }
            if let Some(focus_visible) = self.world.focus_visible(doc) {
                targets.push(Some(focus_visible));
            }
        }
        self.sync_interactive_style_ancestors(&targets);
        if !self.reactive.selected_interactive_nodes.is_empty() {
            let selected_nodes: Vec<Option<StableNodeId>> = self
                .reactive
                .selected_interactive_nodes
                .iter()
                .copied()
                .map(Some)
                .collect();
            self.sync_interactive_style_nodes(&selected_nodes);
        }
    }

    pub(crate) fn has_selected_interactive_node(&self, id: StableNodeId) -> bool {
        self.reactive.selected_interactive_nodes.contains(&id)
    }

    pub(crate) fn sync_keyframe_animations(
        &mut self,
        force_refresh: bool,
    ) -> Result<(), FrameworkError> {
        if self.reactive.motion_bindings.is_empty() && self.reactive.keyframe_tracks.is_empty() {
            return Ok(());
        }
        let now = self.component_lifecycle.now;
        let palette_epoch = self.world.palette_epoch;
        let reduced = self.reduced_motion();
        let mut dirty_nodes: Vec<(StableNodeId, &'static crate::view::StyleSite, u64)> = Vec::new();
        for (&node, binding) in &mut self.reactive.motion_bindings {
            if !self.world.contains(node) {
                continue;
            }
            let curr_mask = binding.current_mask();
            if force_refresh || curr_mask != binding.last_anim_mask {
                binding.last_anim_mask = curr_mask;
                dirty_nodes.push((node, binding.site, curr_mask));
            }
        }
        if dirty_nodes.is_empty() {
            return Ok(());
        }
        let mut mutations = MutationQueue::new();
        for (node, site, mask) in dirty_nodes {
            let desired = site.animations_for(mask);
            let model = self.world.node_style_model(node);
            let previous = self
                .reactive
                .keyframe_tracks
                .remove(&node)
                .unwrap_or_default();
            let mut next_tracks = Vec::with_capacity(desired.len());
            for anim in desired {
                let id = css_keyframe_animation_id(node, anim.name, anim.property);
                let has_dynamic_color = anim.keyframes.iter().any(|k| k.value.is_theme_dynamic());
                let existing = previous.iter().find(|t| t.id == id);
                if let Some(prev) = existing {
                    let mut same_except_play = prev.spec;
                    same_except_play.play_state = anim.play_state;
                    if same_except_play == anim
                        && prev.reduced_motion == reduced
                        && (!has_dynamic_color || prev.palette_epoch == palette_epoch)
                    {
                        if prev.spec.play_state != anim.play_state {
                            match anim.play_state {
                                AnimationPlayState::Paused => mutations.pause_animation(id),
                                AnimationPlayState::Running => mutations.resume_animation(id),
                            }
                        }
                        next_tracks.push(RunningKeyframeTrack {
                            id,
                            spec: anim,
                            palette_epoch,
                            reduced_motion: reduced,
                        });
                        continue;
                    }
                }
                let ms_to_duration = |ms: f32| -> Duration {
                    Duration::from_nanos((f64::from(ms.max(0.0)) * 1_000_000.0).round() as u64)
                };
                let duration = if reduced {
                    Duration::ZERO
                } else {
                    ms_to_duration(anim.duration_ms)
                };
                let delay = if reduced {
                    Duration::ZERO
                } else {
                    ms_to_duration(anim.delay_ms)
                };
                if anim.iterations == AnimationIteration::Infinite && duration.is_zero() {
                    if existing.is_some() {
                        mutations.stop_animation(id);
                    }
                    continue;
                }
                let mut resolved_stops: Vec<nana_ui_core::motion::Keyframe> = anim
                    .keyframes
                    .iter()
                    .map(|stop| nana_ui_core::motion::Keyframe {
                        offset: stop.offset,
                        value: match stop.value {
                            CompiledMotionValue::Scalar(v) => MotionValue::Scalar(v),
                            CompiledMotionValue::Color(color_fn) => {
                                MotionValue::Color(color_fn().resolve_srgb(model))
                            }
                            CompiledMotionValue::Transform(t) => MotionValue::Transform(t),
                            CompiledMotionValue::Filter(filter_fn) => {
                                let mut filter = filter_fn();
                                if let Some(shadow) = filter.drop_shadow.as_mut() {
                                    shadow.color = shadow.color.resolve(model);
                                }
                                MotionValue::Filter(filter.into())
                            }
                        },
                        easing: stop.easing,
                    })
                    .collect();
                if resolved_stops.is_empty() {
                    if existing.is_some() {
                        mutations.stop_animation(id);
                    }
                    continue;
                }
                let fallback_logical = || {
                    self.world
                        .logical_motion_value(node, anim.property)
                        .or_else(|| {
                            (anim.property == AnimatableProperty::Background)
                                .then_some(MotionValue::Color([0.0, 0.0, 0.0, 0.0]))
                        })
                };
                let from_value = resolved_stops
                    .first()
                    .filter(|stop| stop.offset <= f32::EPSILON)
                    .map(|stop| stop.value)
                    .or_else(fallback_logical)
                    .unwrap_or_else(|| resolved_stops[0].value);
                if resolved_stops
                    .last()
                    .is_some_and(|stop| stop.offset < 1.0 - f32::EPSILON)
                {
                    let to_value = fallback_logical().unwrap_or(from_value);
                    resolved_stops.push(nana_ui_core::motion::Keyframe {
                        offset: 1.0,
                        value: to_value,
                        easing: None,
                    });
                }
                let mut spec = crate::AnimationSpec::new(
                    id,
                    node,
                    now,
                    duration,
                    Duration::from_nanos(16_000_000),
                    self.world
                        .node_theme(node)
                        .motion()
                        .easing(nana_ui_core::EasingRole::Linear),
                )
                .with_property(anim.property)
                .with_curve(anim.curve)
                .with_delay(delay)
                .with_range(from_value, MotionTo::Keyframes(resolved_stops))
                .with_playback(AnimationPlayback {
                    iteration_count: anim.iterations,
                    direction: anim.direction,
                    fill_mode: anim.fill_mode,
                    play_state: anim.play_state,
                    paused_at: (anim.play_state == AnimationPlayState::Paused).then_some(now),
                })
                .with_layer(crate::MotionLayer::CssAnimation);
                spec.interrupt = crate::MotionInterrupt::Replace;
                mutations.start_animation(spec);
                next_tracks.push(RunningKeyframeTrack {
                    id,
                    spec: anim,
                    palette_epoch,
                    reduced_motion: reduced,
                });
            }
            for old in previous {
                if !next_tracks.iter().any(|next| next.id == old.id) {
                    mutations.stop_animation(old.id);
                }
            }
            if !next_tracks.is_empty() {
                self.reactive.keyframe_tracks.insert(node, next_tracks);
            }
        }
        if mutations.is_empty() {
            return Ok(());
        }
        self.commit_mutations(mutations).map(|_| ())
    }

    /// `duration`, or none at all under [`Self::reduced_motion`]: for motion
    /// an application starts itself (`queue.node(id, now).transition()`,
    /// `Timeline`), which does not see the preference. A zero-length run
    /// still ends with its `Finished`.
    pub fn motion_duration(&self, duration: Duration) -> Duration {
        if self.reduced_motion() {
            Duration::ZERO
        } else {
            duration
        }
    }

    /// Where `node` is now, for [`Self::flip_after_layout`].
    pub(crate) fn flip_first(&self, node: StableNodeId) -> Option<nana_ui_core::FlipRect> {
        self.flip_rect(node)
    }

    /// Despawn the rows whose leave ended (or was cut short), sliding their
    /// siblings into the space they leave when the transition moves rows.
    pub(crate) fn finish_leaves(&mut self, events: &[crate::AnimationEvent]) {
        if self.reactive.leaving.is_empty() {
            return;
        }
        // The leave replaced whatever its track was playing (an enter still
        // in flight), which reports Cancelled under the same id while the
        // leave runs on: only an end that leaves nothing running counts.
        let done: Vec<StableNodeId> = events
            .iter()
            .filter(|event| {
                self.reactive
                    .leaving
                    .get(&event.target)
                    .is_some_and(|leaving| leaving.animation == event.id)
                    && (event.kind == crate::AnimationEventKind::Finished
                        || !self.world.animation_is_active(event.id))
            })
            .map(|event| event.target)
            .collect();
        if done.is_empty() {
            return;
        }
        let mut mutations = MutationQueue::new();
        let mut notify_hosts = Vec::new();
        for root in done {
            let Some(leaving) = self.reactive.leaving.remove(&root) else {
                continue;
            };
            if !self.world.contains(root) {
                continue;
            }
            if let Some(parent) = self.world.node(root).and_then(|node| node.parent) {
                if self.reactive.host_slots.contains_key(&parent) && !notify_hosts.contains(&parent)
                {
                    notify_hosts.push(parent);
                }
                if let Some(moves) = leaving.moves {
                    let siblings = self
                        .world
                        .node(parent)
                        .map(|node| node.children.to_vec())
                        .unwrap_or_default();
                    for sibling in siblings {
                        if sibling != root
                            && !self.reactive.leaving.contains_key(&sibling)
                            && let Some(first) = self.flip_rect(sibling)
                        {
                            self.flip_after_layout(sibling, first, (moves.0, Some(moves.1)));
                        }
                    }
                }
            }
            mutations.despawn_subtree(root);
        }
        if !mutations.is_empty() {
            if self.commit_mutations(mutations).is_err() {
                nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FLUSH_FAILED);
            } else {
                for host in notify_hosts {
                    if self.world.contains(host) {
                        let _ = self.notify_host_slot_assembler(host);
                    }
                }
            }
        }
    }

    /// After a layout pass of `document`: start the slides of rows it moved.
    pub(crate) fn play_pending_flips(&mut self, document: DocumentId) {
        if self.reactive.flips.is_empty() {
            return;
        }
        let now = self.component_lifecycle.now;
        let mut mutations = MutationQueue::new();
        let flips = std::mem::take(&mut self.reactive.flips);
        let mut kept = Vec::new();
        for flip in flips {
            let Some(node) = self.world.node(flip.node) else {
                continue;
            };
            if node.document != document {
                kept.push(flip);
                continue;
            }
            let Some(last) = self.flip_rect(flip.node) else {
                continue;
            };
            let moved = (flip.first.x - last.x).abs() > 0.5 || (flip.first.y - last.y).abs() > 0.5;
            if moved {
                mutations.start_layout_flip(
                    flip.node,
                    flip.first,
                    last,
                    now,
                    flip.duration,
                    flip.easing,
                );
            }
        }
        self.reactive.flips = kept;
        if !mutations.is_empty() && self.commit_mutations(mutations).is_err() {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FLUSH_FAILED);
        }
    }

    /// After a layout pass of `document`: slide the nodes marked with
    /// [`crate::view::El::moves`] that it moved within their parent.
    pub(crate) fn play_layout_moves(&mut self, document: DocumentId) {
        if self.reactive.layout_moves.is_empty() {
            return;
        }
        let reduced = self.reduced_motion();
        let now = self.component_lifecycle.now;
        let easing = self.transition_easing(None);
        let mut mutations = MutationQueue::new();
        let ids: Vec<StableNodeId> = self.reactive.layout_moves.keys().copied().collect();
        for id in ids {
            let Some(node) = self.world.node(id) else {
                continue;
            };
            if node.document != document {
                continue;
            }
            let placed = self.placed_in_parent(id, node.parent);
            let Some(entry) = self.reactive.layout_moves.get_mut(&id) else {
                continue;
            };
            let was = std::mem::replace(&mut entry.placed, placed);
            let duration = entry.duration;
            let (Some(was), Some(placed)) = (was, placed) else {
                continue;
            };
            if reduced || was.parent != placed.parent {
                continue;
            }
            let moved = [
                was.offset[0] - placed.offset[0],
                was.offset[1] - placed.offset[1],
            ];
            if moved[0].abs() <= 0.5 && moved[1].abs() <= 0.5 {
                continue;
            }
            let Some(last) = self.flip_rect(id) else {
                continue;
            };
            // A slide still running: start from where the node is shown.
            let [sx, sy] = self.shown_offset(id, now);
            let first = nana_ui_core::FlipRect::new(
                last.x + moved[0] + sx,
                last.y + moved[1] + sy,
                last.width,
                last.height,
            );
            mutations.start_layout_flip(id, first, last, now, duration, easing);
        }
        if !mutations.is_empty() && self.commit_mutations(mutations).is_err() {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FLUSH_FAILED);
        }
    }

    /// Where `id` stands in `parent`, while it is in the flow with a size.
    fn placed_in_parent(&self, id: StableNodeId, parent: Option<StableNodeId>) -> Option<Placed> {
        let style = self.world.node_style(id)?;
        if matches!(
            style.layout.position,
            nana_ui_core::PositionSpec::Absolute | nana_ui_core::PositionSpec::Fixed
        ) {
            return None;
        }
        let own = self.world.component_layout_box(id)?;
        if own.width <= 0.0 || own.height <= 0.0 {
            return None;
        }
        let parent = parent?;
        let base = self.world.component_layout_box(parent)?;
        Some(Placed {
            parent,
            offset: [own.x - base.x, own.y - base.y],
        })
    }

    /// How far the compositor shows `id` from its layout, by translation.
    fn shown_offset(&self, id: StableNodeId, now: Duration) -> [f32; 2] {
        match self
            .world
            .presentation_pair(id, AnimatableProperty::Transform, now)
        {
            Some(nana_ui_core::motion::PresentationPair {
                logical: MotionValue::Transform(logical),
                presentation: MotionValue::Transform(shown),
            }) => [shown.e - logical.e, shown.f - logical.f],
            _ => [0.0, 0.0],
        }
    }
}

fn css_keyframe_animation_id(
    node: StableNodeId,
    name: &str,
    property: AnimatableProperty,
) -> AnimationId {
    use std::hash::Hasher as _;
    const CSS_KEYFRAME_TAG: u64 = 0x4b46_524d_0001;
    let mut hasher = crate::IdHasher::default();
    hasher.write_u64(CSS_KEYFRAME_TAG);
    hasher.write_u64(node.get());
    hasher.write(name.as_bytes());
    let tag = match property {
        AnimatableProperty::Opacity => 1u64,
        AnimatableProperty::Transform => 2,
        AnimatableProperty::Color => 3,
        AnimatableProperty::Background => 4,
        AnimatableProperty::Width => 5,
        AnimatableProperty::Height => 6,
        AnimatableProperty::Padding => 7,
        AnimatableProperty::Margin => 8,
        AnimatableProperty::Clip => 9,
        AnimatableProperty::Blur => 10,
        AnimatableProperty::Filter => 11,
        AnimatableProperty::Shadow => 12,
        AnimatableProperty::ShaderParameter => 13,
        AnimatableProperty::FontSize => 14,
        AnimatableProperty::FontAxis(_) => 15,
        AnimatableProperty::Display => 16,
        AnimatableProperty::Progress => 17,
    };
    hasher.write_u64(tag);
    AnimationId::new(hasher.finish())
        .unwrap_or_else(|| AnimationId::new(node.get() | (1u64 << 63)).expect("nonzero"))
}

/// The curve a presence gives `property`: the transform's own when it has
/// one, else its easing. `None`: the theme's standard easing.
fn presence_easing(
    presence: &crate::view::Presence,
    property: AnimatableProperty,
) -> Option<Easing> {
    match property {
        AnimatableProperty::Transform => presence.transform_easing.or(presence.easing),
        _ => presence.easing,
    }
}

/// The properties a presence animates and its value for each.
fn presence_values(
    presence: &crate::view::Presence,
) -> impl Iterator<Item = (AnimatableProperty, MotionValue)> {
    presence
        .opacity
        .map(|opacity| (AnimatableProperty::Opacity, MotionValue::Scalar(opacity)))
        .into_iter()
        .chain(presence.transform.map(|transform| {
            (
                AnimatableProperty::Transform,
                MotionValue::Transform(transform),
            )
        }))
}

#[cfg(test)]
impl AppContext {
    pub(crate) fn event_handler_count(&self) -> usize {
        self.lifecycle.event_handlers.values().map(Vec::len).sum()
    }
}
