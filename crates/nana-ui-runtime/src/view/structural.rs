//! Keyed lists and conditional blocks: the parts of a view whose shape
//! follows data.
//!
//! Each owns a container stack. Rows and branches are built in their own
//! scope, so removing one disposes exactly the signals and effects it
//! created. Kept rows are never rebuilt; their identity and interaction
//! state survive reordering.

use std::borrow::Cow;

// foldhash: row keys are hashed on every list change.
use hashbrown::{HashMap, HashSet};
use std::hash::Hash;
use std::marker::PhantomData;
use std::panic::Location;

use super::controls::StyledComponent;
use super::node::{
    AnyView, El, IntoView, RootKeys, StructuralBinding, ViewBuilder, ViewState, widget,
};
use super::prop::{IntoProp, PropSource};
use super::reactive::{self, EffectKey, EffectTarget, Readable, ScopeKey};
use super::style::{ContainerStyle, container_styles};
use super::transition::Transition;
use crate::{AppContext, ComponentView, FrameworkError, MutationQueue, StableNodeId, Stack};

pub(super) struct Built {
    pub(super) scope: ScopeKey,
    pub(super) roots: Vec<StableNodeId>,
}

/// Build `view` in a new child scope of `parent`, recording the scope as
/// owned by the roots it produced.
pub(super) fn build_scoped(
    vb: &mut ViewBuilder<'_, '_, '_>,
    parent: Option<ScopeKey>,
    view: impl FnOnce() -> AnyView,
) -> Built {
    let scope = reactive::create_scope(parent);
    let roots = reactive::with_scope(scope, || {
        let view = reactive::untrack(view);
        vb.build_collect(view)
    });
    for root in &roots {
        vb.st.parts.anchors.push((*root, scope));
    }
    Built { scope, roots }
}

/// Build detached subtrees for a container that already exists, in one
/// commit, and hand their parts to the context. `build` records every scope
/// it creates so a failed commit disposes them. The roots are keyed under
/// `container` as the ones built with it were (only declared keys unless
/// `positional`; see [`RootKeys`]), so a path through the container finds
/// them.
pub(super) fn build_detached_into<R>(
    cx: &mut AppContext,
    container: StableNodeId,
    positional: bool,
    build: impl FnOnce(&mut ViewBuilder<'_, '_, '_>, &mut Vec<ScopeKey>) -> R,
) -> Result<R, FrameworkError> {
    let document = cx
        .world()
        .node(container)
        .ok_or(FrameworkError::MissingView(container))?
        .document;
    let tag = cx.reactive_tag();
    let mut created = Vec::new();
    let result = cx.build_detached(document, |ui| {
        let mut st = ViewState::new(tag);
        st.root_keys = Some(RootKeys {
            container,
            positional,
        });
        let built = build(&mut ViewBuilder { ui, st: &mut st }, &mut created);
        (built, st.parts)
    });
    match result {
        Ok((built, parts)) => {
            cx.install_view_parts(parts)?;
            Ok(built)
        }
        Err(error) => {
            for scope in created {
                reactive::dispose_scope(scope);
            }
            Err(error)
        }
    }
}

/// Build a view under `parent`, an existing node, in one commit: its roots
/// become `parent`'s children, keyed under it, so `parent` is their key
/// scope. `build` records every scope it creates so a failed commit
/// disposes them.
pub(super) fn build_under<P: crate::View>(
    cx: &mut AppContext,
    parent: crate::Entity<P>,
    build: impl FnOnce(&mut ViewBuilder<'_, '_, '_>, &mut Vec<ScopeKey>),
) -> Result<(), FrameworkError> {
    let tag = cx.reactive_tag();
    let mut created = Vec::new();
    let result = cx.build_child(parent, |ui| {
        let mut st = ViewState::new(tag);
        build(&mut ViewBuilder { ui, st: &mut st }, &mut created);
        st.parts
    });
    match result {
        Ok(parts) => cx.install_view_parts(parts),
        Err(error) => {
            for scope in created {
                reactive::dispose_scope(scope);
            }
            Err(error)
        }
    }
}

/// Dispose the scopes of `removed` and despawn their roots, or, under a
/// transition with a leave, start it and add the root to `leaving`.
fn remove(
    cx: &mut AppContext,
    removed: Vec<Built>,
    transition: Option<&Transition>,
    leaving: &mut Vec<StableNodeId>,
) -> Result<(), FrameworkError> {
    let mut mutations = MutationQueue::new();
    for built in removed {
        reactive::dispose_scope(built.scope);
        for root in built.roots {
            if !cx.world().contains(root) {
                continue;
            }
            if let Some(transition) = transition
                && let Some(leave) = &transition.leave
                && cx.begin_leave(root, leave, transition.moves)?
            {
                leaving.push(root);
                continue;
            }
            mutations.despawn_subtree(root);
        }
    }
    if mutations.is_empty() {
        return Ok(());
    }
    cx.commit_mutations(mutations).map(|_| ())
}

/// `ordered` with each node of `leaving` kept after the node it followed in
/// `container`'s current children (or first, if none of those stays).
fn with_leaving(
    cx: &AppContext,
    container: StableNodeId,
    ordered: Vec<StableNodeId>,
    leaving: &[StableNodeId],
) -> Vec<StableNodeId> {
    if leaving.is_empty() {
        return ordered;
    }
    let current = cx
        .world()
        .node(container)
        .map(|node| node.children.to_vec())
        .unwrap_or_default();
    let stays: HashSet<StableNodeId> = ordered.iter().copied().collect();
    let leaves: HashSet<StableNodeId> = leaving.iter().copied().collect();
    let mut front = Vec::new();
    let mut after: HashMap<StableNodeId, Vec<StableNodeId>> = HashMap::new();
    let mut anchor = None;
    for child in current {
        if stays.contains(&child) {
            anchor = Some(child);
        } else if leaves.contains(&child) {
            match anchor {
                Some(anchor) => after.entry(anchor).or_default().push(child),
                None => front.push(child),
            }
        }
    }
    let mut out = front;
    out.reserve(ordered.len() + leaving.len());
    for node in ordered {
        out.push(node);
        if let Some(leaving) = after.remove(&node) {
            out.extend(leaving);
        }
    }
    out
}

/// `v-for` with `:key`. Rows whose key stays are kept as they are; a row
/// follows data changes through its own signals, not by being rebuilt.
/// Duplicate keys after the first are skipped.
pub struct Each<T, K, S, KF, RF> {
    items: S,
    key_fn: KF,
    row_fn: RF,
    transition: Option<Transition>,
    container: Stack,
    style: ContainerStyle,
    key: Option<Cow<'static, str>>,
    site: &'static Location<'static>,
    _types: PhantomData<fn(T) -> K>,
}

/// Keyed list over `items`: `key` names each item, `row` builds it once.
#[track_caller]
pub fn each<T, K, S, KF, RF, V>(items: S, key: KF, row: RF) -> Each<T, K, S, KF, RF>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    Each {
        items,
        key_fn: key,
        row_fn: row,
        transition: None,
        container: Stack::column(0.0),
        style: ContainerStyle::default(),
        key: None,
        site: Location::caller(),
        _types: PhantomData,
    }
}

impl<T, K, S, KF, RF> Each<T, K, S, KF, RF> {
    /// Gap between rows.
    pub fn gap(mut self, gap: impl super::Px) -> Self {
        self.container = Stack::column(gap.px());
        self
    }

    /// Lay rows out horizontally.
    pub fn horizontal(mut self, gap: impl super::Px) -> Self {
        self.container = Stack::row(gap.px());
        self
    }

    /// Animate rows entering, leaving and, with [`Transition::moves`],
    /// changing place (Vue `<TransitionGroup>`). Rows present at mount do
    /// not enter.
    pub fn transition(mut self, transition: Transition) -> Self {
        self.transition = Some(transition);
        self
    }

    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Build the rows straight into `element`, the list's container in
    /// place of a stack of its own: for a component that takes its rows as
    /// its own children, such as a `ReorderList` with live rows, which reads
    /// each row's box. The element keeps its bindings, events, refs and key;
    /// the list's own `gap`, `horizontal`, styles and key are not used. The
    /// list owns all of the element's children, so it takes none of its
    /// own, and its component places no slots and assembles nothing.
    pub fn container<C: ComponentView>(self, element: El<C>) -> EachIn<T, K, S, KF, RF, C> {
        EachIn {
            each: self,
            element,
        }
    }

    fn container_style_mut(&mut self) -> &mut ContainerStyle {
        &mut self.style
    }
}

/// A keyed list whose rows are an element's own children
/// ([`Each::container`]).
pub struct EachIn<T, K, S, KF, RF, C: ComponentView> {
    each: Each<T, K, S, KF, RF>,
    element: El<C>,
}

container_styles!(
    [T, K, S, KF, RF] Each<T, K, S, KF, RF>,
    [] When,
    [K: 'static] Dynamic<K>,
);

/// The container of a structural view: `container` as an element with the
/// view's key and styles, declared where the view is.
pub(crate) fn container<C: StyledComponent + ComponentView>(
    container: C,
    key: Option<Cow<'static, str>>,
    style: ContainerStyle,
    site: &'static Location<'static>,
) -> El<C> {
    let element = widget(container).declared_at(site);
    let element = match key {
        Some(key) => element.key(key),
        None => element,
    };
    style.apply(element)
}

/// Whether an item's key is the first of its kind, and built already.
enum Admit {
    /// A key seen earlier in the list: the item is skipped.
    Duplicate,
    /// A row with this key is built.
    Built,
    /// The first item with this key, and no row has it yet.
    Fresh,
}

/// A row still to build: its key and the call that makes its view.
type FreshRow<'a, K> = (K, Box<dyn FnOnce() -> AnyView + 'a>);

/// The part of a keyed list that knows its items: their keys and how to
/// build a row. Everything else is [`EachBinding`], generic over the key
/// alone.
trait EachSource<K>: Send {
    /// The keys `admit` lets through, in order, and the rows to build for
    /// the ones it calls fresh. Reads the items tracked.
    fn read<'a>(&'a self, admit: &mut dyn FnMut(&K) -> Admit) -> (Vec<K>, Vec<FreshRow<'a, K>>);
}

struct EachItems<T, S, KF, RF> {
    items: S,
    key_fn: KF,
    row_fn: RF,
    _types: PhantomData<fn(T)>,
}

impl<T, K, S, KF, RF, V> EachSource<K> for EachItems<T, S, KF, RF>
where
    T: Clone + 'static,
    K: Clone,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    fn read<'a>(&'a self, admit: &mut dyn FnMut(&K) -> Admit) -> (Vec<K>, Vec<FreshRow<'a, K>>) {
        self.items.with_value(|items| {
            let mut keys = Vec::with_capacity(items.len());
            let mut fresh: Vec<FreshRow<'a, K>> = Vec::new();
            for item in items {
                let key = (self.key_fn)(item);
                match admit(&key) {
                    Admit::Duplicate => continue,
                    Admit::Built => {}
                    Admit::Fresh => {
                        let item = item.clone();
                        let row_fn = &self.row_fn;
                        fresh.push((key.clone(), Box::new(move || row_fn(item).into_any())));
                    }
                }
                keys.push(key);
            }
            (keys, fresh)
        })
    }
}

/// A keyed list bound to its items: diffing, building and removing rows,
/// their order and transitions, for every item type with this key type.
struct EachBinding<K> {
    source: Box<dyn EachSource<K>>,
    transition: Option<Transition>,
    /// Removed rows still playing their leave.
    leaving: Vec<StableNodeId>,
    scope: Option<ScopeKey>,
    rows: HashMap<K, Built>,
    order: Vec<K>,
}

impl<K: Eq + Hash + Clone + Send + 'static> EachBinding<K> {
    /// A list's binding, in the scope it is built in.
    fn new(source: Box<dyn EachSource<K>>, transition: Option<Transition>) -> Self {
        Self {
            source,
            transition,
            leaving: Vec::new(),
            scope: reactive::current_scope(),
            rows: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// The keys in order, and the rows whose keys are not built yet.
    /// Duplicate keys after the first are skipped.
    fn read<'a>(
        source: &'a dyn EachSource<K>,
        rows: &HashMap<K, Built>,
    ) -> (Vec<K>, Vec<FreshRow<'a, K>>) {
        let mut seen = HashSet::new();
        source.read(&mut |key| {
            if !seen.insert(key.clone()) {
                Admit::Duplicate
            } else if rows.contains_key(key) {
                Admit::Built
            } else {
                Admit::Fresh
            }
        })
    }

    /// Build the rows under `container`, placed already, and keep them
    /// bound to the items.
    #[inline(never)]
    fn mount(
        mut self,
        vb: &mut ViewBuilder<'_, '_, '_>,
        container: StableNodeId,
        site: &'static Location<'static>,
    ) {
        let effect =
            reactive::create_effect(vb.st.tag, EffectTarget::Structural(container), None, site);
        let (keys, fresh) = reactive::run_tracked(effect, || Self::read(&*self.source, &self.rows));
        let mut fresh = Some(fresh);
        let mut built = Vec::new();
        let scope = self.scope;
        vb.nest_dyn(container, &mut |vb| {
            if let Some(fresh) = fresh.take() {
                built = Self::build_rows(vb, scope, fresh);
            }
        });
        // The rows it held borrow the source, which goes into the box below.
        drop(fresh);
        self.rows.extend(built);
        self.order = keys;
        vb.st
            .parts
            .structural
            .push((container, effect, Box::new(self)));
    }

    fn build_rows(
        vb: &mut ViewBuilder<'_, '_, '_>,
        scope: Option<ScopeKey>,
        fresh: Vec<FreshRow<'_, K>>,
    ) -> Vec<(K, Built)> {
        fresh
            .into_iter()
            .map(|(key, row)| (key, build_scoped(vb, scope, row)))
            .collect()
    }
}

impl<T, K, S, KF, RF, V> IntoView for Each<T, K, S, KF, RF>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let Each {
            items,
            key_fn,
            row_fn,
            transition,
            container: stack,
            style,
            key,
            site,
            ..
        } = self;
        if let Some(container) = container(stack.structural(), key, style, site).place(vb) {
            each_binding(items, key_fn, row_fn, transition).mount(vb, container.stable_id(), site);
        }
    }
}

impl<T, K, S, KF, RF, V, C> IntoView for EachIn<T, K, S, KF, RF, C>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
    C: ComponentView,
{
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let Each {
            items,
            key_fn,
            row_fn,
            transition,
            site,
            ..
        } = self.each;
        if let Some(container) = self.element.place(vb) {
            each_binding(items, key_fn, row_fn, transition).mount(vb, container.stable_id(), site);
        }
    }
}

/// The binding of a list over `items`.
fn each_binding<T, K, S, KF, RF, V>(
    items: S,
    key_fn: KF,
    row_fn: RF,
    transition: Option<Transition>,
) -> EachBinding<K>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    EachBinding::new(
        Box::new(EachItems {
            items,
            key_fn,
            row_fn,
            _types: PhantomData,
        }),
        transition,
    )
}

impl<K: Eq + Hash + Clone + Send + 'static> StructuralBinding for EachBinding<K> {
    #[inline(never)]
    fn update(
        &mut self,
        cx: &mut AppContext,
        container: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let (keys, fresh) = reactive::run_tracked(effect, || Self::read(&*self.source, &self.rows));
        let transition = self.transition;
        let moves = transition.and_then(|transition| transition.moves);
        let removed: Vec<K> = {
            let kept: HashSet<&K> = keys.iter().collect();
            self.order
                .iter()
                .filter(|key| !kept.contains(key))
                .cloned()
                .collect()
        };
        let removed = removed
            .into_iter()
            .filter_map(|key| self.rows.remove(&key))
            .collect();
        // Where kept rows are before the change, to slide them from.
        let firsts: Vec<_> = match moves {
            Some(_) => self
                .rows
                .values()
                .flat_map(|built| built.roots.iter().copied())
                .filter_map(|root| Some((root, cx.flip_first(root)?)))
                .collect(),
            None => Vec::new(),
        };
        self.leaving.retain(|root| cx.is_leaving(*root));
        remove(cx, removed, transition.as_ref(), &mut self.leaving)?;
        let mut entered = Vec::new();
        if !fresh.is_empty() {
            let scope = self.scope;
            let built = build_detached_into(cx, container, false, |vb, created| {
                let built = Self::build_rows(vb, scope, fresh);
                created.extend(built.iter().map(|(_, built)| built.scope));
                built
            })?;
            if transition.is_some_and(|transition| transition.enter.is_some()) {
                entered.extend(
                    built
                        .iter()
                        .flat_map(|(_, built)| built.roots.iter().copied()),
                );
            }
            self.rows.extend(built);
        }
        self.order = keys;
        let ordered: Vec<StableNodeId> = self
            .order
            .iter()
            .filter_map(|key| self.rows.get(key))
            .flat_map(|built| built.roots.iter().copied())
            .collect();
        let ordered = with_leaving(cx, container, ordered, &self.leaving);
        cx.reconcile_children(container, &ordered)?;
        if let Some(enter) = transition.and_then(|transition| transition.enter) {
            cx.begin_enter(&entered, &enter)?;
        }
        if let Some(moves) = moves {
            for (root, first) in firsts {
                cx.flip_after_layout(root, first, moves);
            }
        }
        Ok(())
    }
}

/// `v-if` / `v-else`. The branch that is not shown does not exist: its
/// nodes are despawned and its scope disposed, unless the block keeps
/// branches alive.
pub struct When {
    condition: PropSource<bool>,
    then: Box<dyn Fn() -> AnyView + Send>,
    otherwise: Option<Box<dyn Fn() -> AnyView + Send>>,
    options: SwitchOptions,
    site: &'static Location<'static>,
}

/// Show `then` while `condition` holds.
#[track_caller]
pub fn when<V: IntoView>(
    condition: impl IntoProp<bool>,
    then: impl Fn() -> V + Send + 'static,
) -> When {
    When {
        condition: condition.into_source(),
        then: Box::new(move || then().into_any()),
        otherwise: None,
        options: SwitchOptions::default(),
        site: Location::caller(),
    }
}

/// [`each`] from the data: `todos.each(|t| t.id, todo_row)`.
pub trait EachExt<T: Clone + 'static>: Readable<Vec<T>> + Sized {
    /// A keyed list over these items; see [`each`].
    #[track_caller]
    fn each<K, KF, RF, V>(self, key: KF, row: RF) -> Each<T, K, Self, KF, RF>
    where
        K: Eq + Hash + Clone + Send + 'static,
        KF: Fn(&T) -> K + Send + 'static,
        RF: Fn(T) -> V + Send + 'static,
        V: IntoView,
    {
        each(self, key, row)
    }
}

impl<T: Clone + 'static, S: Readable<Vec<T>>> EachExt<T> for S {}

/// [`when`] from the condition: `loading.then_show(spinner)`. Not `show`:
/// Vue's `v-show` keeps the node, which is [`El::visible`](super::El::visible).
pub trait WhenExt: IntoProp<bool> + Sized {
    /// Build `then` while this holds; see [`when`].
    #[track_caller]
    fn then_show<V: IntoView>(self, then: impl Fn() -> V + Send + 'static) -> When {
        when(self, then)
    }
}

impl<P: IntoProp<bool>> WhenExt for P {}

/// What a switching block does with branches it stops showing.
#[derive(Default)]
struct SwitchOptions {
    transition: Option<Transition>,
    /// Keep at most this many branches that are not shown, alive.
    keep_alive: Option<usize>,
    key: Option<Cow<'static, str>>,
    style: ContainerStyle,
    /// The container takes the space left along its parent's main axis.
    grow: bool,
}

impl When {
    /// Show `otherwise` while the condition does not hold.
    pub fn otherwise<V: IntoView>(mut self, otherwise: impl Fn() -> V + Send + 'static) -> Self {
        self.otherwise = Some(Box::new(move || otherwise().into_any()));
        self
    }

    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.options.key = Some(key.into());
        self
    }

    /// Animate the branch entering and leaving (Vue `<Transition>`). The
    /// leaving branch stays in place until its leave ends, the new one
    /// after it; with [`Transition::moves`] the new one slides up when the
    /// old one goes. The branch shown at mount does not enter.
    pub fn transition(mut self, transition: Transition) -> Self {
        self.options.transition = Some(transition);
        self
    }

    /// Keep the branch that is not shown alive, nodes and state, instead of
    /// dropping it (Vue `<KeepAlive>`): switching back shows it as it was.
    pub fn keep_alive(mut self) -> Self {
        self.options.keep_alive = Some(1);
        self
    }

    /// Let the container take the space left along its parent's main axis,
    /// so a branch that fills (a scroll view, an editor) has room to fill.
    pub fn grow(mut self) -> Self {
        self.options.grow = true;
        self
    }

    fn container_style_mut(&mut self) -> &mut ContainerStyle {
        &mut self.options.style
    }
}

struct WhenBranches {
    then: Box<dyn Fn() -> AnyView + Send>,
    otherwise: Option<Box<dyn Fn() -> AnyView + Send>>,
}

impl Branches<bool> for WhenBranches {
    fn exists(&self, shown: &bool) -> bool {
        *shown || self.otherwise.is_some()
    }

    fn make(&self, shown: &bool) -> AnyView {
        match (shown, &self.otherwise) {
            (true, _) => (self.then)(),
            (false, Some(otherwise)) => otherwise(),
            (false, None) => ().into_any(),
        }
    }
}

impl IntoView for When {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let render: Render<bool> = Box::new(WhenBranches {
            then: self.then,
            otherwise: self.otherwise,
        });
        let condition = self.condition;
        build_switch(
            vb,
            Box::new(move || condition.get()),
            render,
            self.options,
            self.site,
        );
    }
}

/// One view at a time, chosen by a key (Vue `<component :is>`). See
/// [`dynamic`].
pub struct Dynamic<K: 'static> {
    key: PropSource<K>,
    render: Render<K>,
    options: SwitchOptions,
    site: &'static Location<'static>,
}

/// The branches of a switching block. `make` runs inside the branch's own
/// scope, so what the view's functions create belongs to the branch.
trait Branches<K>: Send {
    fn exists(&self, key: &K) -> bool;
    fn make(&self, key: &K) -> AnyView;
}

type Render<K> = Box<dyn Branches<K>>;

struct DynamicRender<F>(F);

impl<K, V: IntoView, F: Fn(&K) -> V + Send> Branches<K> for DynamicRender<F> {
    fn exists(&self, _: &K) -> bool {
        true
    }

    fn make(&self, key: &K) -> AnyView {
        (self.0)(key).into_any()
    }
}

/// Show `render(key)`, built again whenever `key` changes to a different
/// value; with [`Dynamic::keep_alive`] the views of earlier keys are kept
/// and shown again as they were (tabs).
#[track_caller]
pub fn dynamic<K, V>(key: impl IntoProp<K>, render: impl Fn(&K) -> V + Send + 'static) -> Dynamic<K>
where
    K: Clone + PartialEq + 'static,
    V: IntoView,
{
    Dynamic {
        key: key.into_source(),
        render: Box::new(DynamicRender(render)),
        options: SwitchOptions::default(),
        site: Location::caller(),
    }
}

impl<K> Dynamic<K> {
    /// Keep the views of the keys shown before alive (Vue `<KeepAlive>`),
    /// at most [`Self::max`] of them.
    pub fn keep_alive(mut self) -> Self {
        self.options.keep_alive.get_or_insert(usize::MAX);
        self
    }

    /// Keep at most `max` views that are not shown; the one shown least
    /// recently goes first (Vue `<KeepAlive :max>`).
    pub fn max(mut self, max: usize) -> Self {
        self.options.keep_alive = Some(max);
        self
    }

    /// Animate the view entering and leaving.
    pub fn transition(mut self, transition: Transition) -> Self {
        self.options.transition = Some(transition);
        self
    }

    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.options.key = Some(key.into());
        self
    }

    /// Let the container take the space left along its parent's main axis,
    /// so a view that fills (a scroll view, an editor) has room to fill.
    pub fn grow(mut self) -> Self {
        self.options.grow = true;
        self
    }

    fn container_style_mut(&mut self) -> &mut ContainerStyle {
        &mut self.options.style
    }
}

impl<K: Clone + PartialEq + Send + 'static> IntoView for Dynamic<K> {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let key = self.key;
        build_switch(
            vb,
            Box::new(move || key.get()),
            self.render,
            self.options,
            self.site,
        );
    }
}

fn build_switch<K: Clone + PartialEq + Send + 'static>(
    vb: &mut ViewBuilder<'_, '_, '_>,
    key: Box<dyn Fn() -> K + Send>,
    render: Render<K>,
    options: SwitchOptions,
    site: &'static Location<'static>,
) {
    let SwitchOptions {
        transition,
        keep_alive,
        key: node_key,
        style,
        grow,
    } = options;
    let mut column = Stack::column(0.0);
    if grow {
        let layout = column.layout_mut();
        layout.flex_grow = Some(1.0);
        layout.flex_shrink = Some(1.0);
        layout.flex_basis = Some(nana_ui_core::LengthSpec::Px(0.0));
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(0.0));
    }
    let Some(container) = container(column.structural(), node_key, style, site).place(vb) else {
        return;
    };
    let id = container.stable_id();
    let effect = reactive::create_effect(vb.st.tag, EffectTarget::Structural(id), None, site);
    let shown = reactive::run_tracked(effect, &key);
    let mut binding = SwitchBinding {
        key,
        render,
        transition,
        keep_alive,
        scope: reactive::current_scope(),
        shown,
        branch: None,
        kept: Vec::new(),
        holder: None,
        leaving: Vec::new(),
    };
    vb.nest(container, |vb| {
        if binding.render.exists(&binding.shown) {
            let (render, shown) = (&binding.render, &binding.shown);
            binding.branch = Some(build_scoped(vb, binding.scope, || render.make(shown)));
        }
    });
    vb.st.parts.structural.push((id, effect, Box::new(binding)));
}

struct SwitchBinding<K> {
    key: Box<dyn Fn() -> K + Send>,
    render: Render<K>,
    transition: Option<Transition>,
    keep_alive: Option<usize>,
    scope: Option<ScopeKey>,
    shown: K,
    branch: Option<Built>,
    /// Branches kept alive, the one shown longest ago first, with the keys
    /// their roots held under the container: a branch shown after them may
    /// take the same keys, and a kept branch shown again takes them back.
    kept: Vec<(K, Built, Vec<crate::framework::AssembledKey>)>,
    /// The hidden stack kept branches wait in, created on first use.
    holder: Option<StableNodeId>,
    leaving: Vec<StableNodeId>,
}

impl<K: Clone + PartialEq + Send + 'static> SwitchBinding<K> {
    /// Move `branch` into the hidden holder, dropping the oldest kept
    /// branches beyond the limit.
    fn stash(
        &mut self,
        cx: &mut AppContext,
        container: StableNodeId,
        key: K,
        branch: Built,
        limit: usize,
    ) -> Result<(), FrameworkError> {
        let holder = match self.holder.filter(|holder| cx.world().contains(*holder)) {
            Some(holder) => holder,
            None => {
                let document = cx
                    .world()
                    .node(container)
                    .ok_or(FrameworkError::MissingView(container))?
                    .document;
                let hidden = Stack::column(0.0).with_layout(|layout| layout.hidden = true);
                let holder = cx.create_component(document, hidden)?.stable_id();
                self.holder = Some(holder);
                holder
            }
        };
        let mut mutations = MutationQueue::new();
        if let Some(document) = cx.world().node(container).map(|node| node.document)
            && cx.world().focused(document).is_some_and(|focused| {
                branch
                    .roots
                    .iter()
                    .any(|root| cx.world().is_descendant_or_self(focused, *root))
            })
        {
            mutations.request_focus(document, None);
        }
        for root in &branch.roots {
            if cx.world().contains(*root) {
                mutations.insert(holder, *root, None);
            }
        }
        let keys = branch
            .roots
            .iter()
            .filter_map(|root| cx.assembled_key(container, *root))
            .collect();
        self.kept.push((key, branch, keys));
        while self.kept.len() > limit {
            let (_, evicted, _) = self.kept.remove(0);
            reactive::dispose_scope(evicted.scope);
            for root in evicted.roots {
                if cx.world().contains(root) {
                    mutations.despawn_subtree(root);
                }
            }
        }
        cx.commit_mutations(mutations).map(|_| ())
    }
}

impl<K: Clone + PartialEq + Send + 'static> StructuralBinding for SwitchBinding<K> {
    fn update(
        &mut self,
        cx: &mut AppContext,
        container: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let shown = reactive::run_tracked(effect, &self.key);
        if shown == self.shown {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.shown, shown.clone());
        let transition = self.transition;
        self.leaving.retain(|root| cx.is_leaving(*root));
        // Taken out first, so stashing the old branch cannot evict it.
        let kept = self
            .kept
            .iter()
            .position(|(key, ..)| *key == shown)
            .map(|at| self.kept.remove(at));
        if let Some(branch) = self.branch.take() {
            match self.keep_alive {
                Some(limit) => self.stash(cx, container, previous, branch, limit)?,
                None => remove(cx, vec![branch], transition.as_ref(), &mut self.leaving)?,
            }
        }
        let branch = match kept {
            Some((_, branch, keys)) => {
                for key in keys {
                    cx.rekey_assembled(container, key);
                }
                Some(branch)
            }
            None if self.render.exists(&shown) => {
                let (render, scope) = (&self.render, self.scope);
                Some(build_detached_into(cx, container, true, |vb, created| {
                    let built = build_scoped(vb, scope, || render.make(&shown));
                    created.push(built.scope);
                    built
                })?)
            }
            None => None,
        };
        let roots = branch
            .as_ref()
            .map(|branch| branch.roots.clone())
            .unwrap_or_default();
        self.branch = branch;
        let ordered = self
            .leaving
            .iter()
            .copied()
            .chain(roots.iter().copied())
            .chain(self.holder.filter(|holder| cx.world().contains(*holder)))
            .collect::<Vec<_>>();
        cx.reconcile_children(container, &ordered)?;
        if let Some(enter) = transition.and_then(|transition| transition.enter) {
            cx.begin_enter(&roots, &enter)?;
        }
        Ok(())
    }
}
