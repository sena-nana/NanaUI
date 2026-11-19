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
    AnyView, El, FragmentBinding, FragmentDecl, IntoView, RootKeys, SlotItem, ViewBuilder,
    ViewState, widget,
};
use super::prop::{IntoProp, PropSource};
use super::reactive::{self, EffectKey, EffectTarget, FragmentId, Readable, ScopeKey};
use super::style::{ContainerStyle, container_styles};
use super::transition::Transition;
use crate::{AppContext, ComponentView, FrameworkError, MutationQueue, StableNodeId, Stack};

pub(super) struct Built {
    pub(super) scope: ScopeKey,
    pub(super) items: Vec<SlotItem>,
}

/// Build `view` in a new child scope of `parent`, recording the scope as
/// owned by the direct concrete roots it produced.
pub(super) fn build_scoped(
    vb: &mut ViewBuilder<'_, '_, '_>,
    parent: Option<ScopeKey>,
    view: impl FnOnce() -> AnyView,
) -> Built {
    let scope = reactive::create_scope(parent);
    let (_, items) = reactive::with_scope(scope, || {
        let view = reactive::untrack(view);
        vb.build_collect_slots(view)
    });
    for item in &items {
        if let SlotItem::Node(root) = *item {
            vb.st.parts.anchors.push((root, scope));
        }
    }
    Built { scope, items }
}

fn fragment_root_keys(
    cx: &AppContext,
    frag: FragmentId,
    host: StableNodeId,
    positional_when_boxed: bool,
) -> Option<RootKeys> {
    if cx.fragment_has_mount_scope(frag) {
        return None;
    }
    Some(RootKeys {
        container: host,
        positional: positional_when_boxed && cx.fragment_boxed(frag),
    })
}

/// Build detached subtrees for a host that already exists, in one commit, and
/// hand their parts to the context. `build` records every scope it creates so
/// a failed commit disposes them.
pub(super) fn build_detached_into<R>(
    cx: &mut AppContext,
    host: StableNodeId,
    parent_fragment: Option<FragmentId>,
    root_keys: Option<RootKeys>,
    build: impl FnOnce(&mut ViewBuilder<'_, '_, '_>, &mut Vec<ScopeKey>) -> R,
) -> Result<R, FrameworkError> {
    let document = cx
        .world()
        .node(host)
        .ok_or(FrameworkError::MissingView(host))?
        .document;
    let tag = cx.reactive_tag();
    let mut created = Vec::new();
    let result = cx.build_detached(document, |ui| {
        let mut st = ViewState::new_in(tag, Some(host), parent_fragment);
        st.root_keys = root_keys;
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
    let host = parent.stable_id();
    let mut created = Vec::new();
    let result = cx.build_child(parent, |ui| {
        let mut st = ViewState::new_in(tag, Some(host), None);
        let mut vb = ViewBuilder { ui, st: &mut st };
        build(&mut vb, &mut created);
        if let Some(slots) = vb.take_top_slots_if_fragments() {
            vb.st.parts.host_slots.push((host, slots));
        }
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

fn teardown_built(
    cx: &mut AppContext,
    built: Built,
    visible_nodes: &mut Vec<StableNodeId>,
    immediate_despawn: &mut Vec<StableNodeId>,
) {
    reactive::dispose_scope(built.scope);
    cx.teardown_slot_items(&built.items, visible_nodes, immediate_despawn);
}

fn forget_built(cx: &mut AppContext, built: Built) {
    reactive::dispose_scope(built.scope);
    for item in built.items {
        if let SlotItem::Fragment(frag) = item {
            cx.forget_fragment(frag);
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
    let mut visible = Vec::new();
    let mut immediate = Vec::new();
    for built in removed {
        teardown_built(cx, built, &mut visible, &mut immediate);
    }
    let mut mutations = MutationQueue::new();
    for root in immediate {
        if cx.world().contains(root) {
            mutations.despawn_subtree(root);
        }
    }
    for root in visible {
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
    container: Option<Stack>,
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
        container: None,
        style: ContainerStyle::default(),
        key: None,
        site: Location::caller(),
        _types: PhantomData,
    }
}

impl<T, K, S, KF, RF> Each<T, K, S, KF, RF> {
    /// Gap between rows.
    pub fn gap(mut self, gap: impl super::Px) -> Self {
        self.container = Some(Stack::column(gap.px()));
        self
    }

    /// Lay rows out horizontally.
    pub fn horizontal(mut self, gap: impl super::Px) -> Self {
        self.container = Some(Stack::row(gap.px()));
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

/// A row still to build: its key and the call that makes its view.
type FreshRow<'a, K> = (K, Box<dyn FnOnce() -> AnyView + 'a>);

/// The part of a keyed list that knows its items: their keys and how to
/// build a row. Everything else is [`EachBinding`], generic over the key
/// alone.
trait EachSource<K>: Send {
    /// Every item's key, in order, a repeated key repeated. Reads the items
    /// tracked.
    fn keys(&self) -> Vec<K>;

    /// The rows to build for the items at `indices`, each with its key.
    fn rows<'a>(&'a self, indices: &[usize]) -> Vec<FreshRow<'a, K>>;
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
    fn keys(&self) -> Vec<K> {
        self.items
            .with_value(|items| items.iter().map(&self.key_fn).collect())
    }

    fn rows<'a>(&'a self, indices: &[usize]) -> Vec<FreshRow<'a, K>> {
        self.items.with_value(|items| {
            indices
                .iter()
                .filter_map(|index| items.get(*index))
                .map(|item| {
                    let key = (self.key_fn)(item);
                    let item = item.clone();
                    let row_fn = &self.row_fn;
                    let row: Box<dyn FnOnce() -> AnyView + 'a> =
                        Box::new(move || row_fn(item).into_any());
                    (key, row)
                })
                .collect()
        })
    }
}

/// What a list's items ask of the rows built for the items before them.
#[derive(Debug, PartialEq)]
struct Changes<K> {
    /// The items' keys in order, the first of each.
    keys: Vec<K>,
    /// The items to build a row for, by index in the items.
    fresh: Vec<usize>,
    /// Keys with a row that no item has any more.
    removed: Vec<K>,
    /// How many keys at the start, and how many at the end, are the old
    /// ones in the old order: their rows are in place already.
    head: usize,
    tail: usize,
}

impl<K: Eq + Hash + Clone> Changes<K> {
    /// From `old`, the keys rows are built for in their order, to `items`,
    /// every item's key. `built` says whether a key has a row.
    ///
    /// A list changes in one place far more often than everywhere: a row
    /// inserted, removed or moved leaves the keys before and after it as
    /// they were. Those are found by comparing, and only the keys between
    /// are hashed. Hashing every key on every change made one inserted row
    /// cost the length of the list (5 ms in 100,000 rows).
    fn between(old: &[K], items: Vec<K>, built: impl Fn(&K) -> bool) -> Self {
        let (head, tail) = common_ends(old, &items);
        let was = &old[head..old.len() - tail];
        let now = &items[head..items.len() - tail];
        if now.is_empty() {
            return Self {
                removed: was.to_vec(),
                keys: items,
                fresh: Vec::new(),
                head,
                tail,
            };
        }
        let settled = {
            let had: HashSet<&K> = was.iter().collect();
            let mut seen: HashSet<&K> = HashSet::with_capacity(now.len());
            let mut fresh = Vec::new();
            // The keys between with the repeats left out, once one is met.
            let mut firsts: Option<Vec<K>> = None;
            let mut unsettled = false;
            for (offset, key) in now.iter().enumerate() {
                if !seen.insert(key) {
                    firsts.get_or_insert_with(|| now[..offset].to_vec());
                    continue;
                }
                if !had.contains(key) {
                    if built(key) {
                        // A key from the unchanged start or end, named
                        // again in between: which of the two comes first is
                        // a question for the whole list.
                        unsettled = true;
                        break;
                    }
                    fresh.push(head + offset);
                }
                if let Some(firsts) = &mut firsts {
                    firsts.push(key.clone());
                }
            }
            (!unsettled).then(|| {
                let removed: Vec<K> = was
                    .iter()
                    .filter(|key| !seen.contains(key))
                    .cloned()
                    .collect();
                (fresh, removed, firsts)
            })
        };
        let Some((fresh, removed, firsts)) = settled else {
            return Self::across(old, items, built);
        };
        let keys = match firsts {
            None => items,
            Some(firsts) => items[..head]
                .iter()
                .cloned()
                .chain(firsts)
                .chain(items[items.len() - tail..].iter().cloned())
                .collect(),
        };
        Self {
            keys,
            fresh,
            removed,
            head,
            tail,
        }
    }

    /// The same answer with every key hashed, for the lists
    /// [`Self::between`] cannot settle from the changed part alone.
    fn across(old: &[K], items: Vec<K>, built: impl Fn(&K) -> bool) -> Self {
        let mut fresh = Vec::new();
        let mut firsts = Vec::with_capacity(items.len());
        let removed = {
            let mut seen: HashSet<&K> = HashSet::with_capacity(items.len());
            for (index, key) in items.iter().enumerate() {
                if seen.insert(key) {
                    if !built(key) {
                        fresh.push(index);
                    }
                    firsts.push(index);
                }
            }
            old.iter()
                .filter(|key| !seen.contains(key))
                .cloned()
                .collect()
        };
        let keys = if firsts.len() == items.len() {
            items
        } else {
            firsts
                .into_iter()
                .map(|index| items[index].clone())
                .collect()
        };
        Self {
            keys,
            fresh,
            removed,
            head: 0,
            tail: 0,
        }
    }
}

/// How many elements `old` and `new` start with in common, and how many of
/// the rest they end with in common.
fn common_ends<T: PartialEq>(old: &[T], new: &[T]) -> (usize, usize) {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (head, tail)
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
    /// Every row's slot items, in `order`, and how many each key has. Kept so
    /// a change in one place takes the unchanged rows' items from here rather
    /// than look each row up again.
    items: Vec<SlotItem>,
    spans: Vec<u32>,
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
            items: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// What the items now ask for, and the rows to build for the keys that
    /// have none. Reads the items tracked; a key after the first item with
    /// it is skipped.
    fn read<'a>(
        source: &'a dyn EachSource<K>,
        order: &[K],
        rows: &HashMap<K, Built>,
    ) -> (Changes<K>, Vec<FreshRow<'a, K>>) {
        let changes = Changes::between(order, source.keys(), |key| rows.contains_key(key));
        let fresh = source.rows(&changes.fresh);
        (changes, fresh)
    }

    /// Takes `changes`' keys as the order, with the items of the rows they
    /// have. The rows of the unchanged start and end keep the items they had;
    /// only the rows between are looked up.
    fn settle(&mut self, changes: Changes<K>) {
        let Changes {
            keys, head, tail, ..
        } = changes;
        let count = |spans: &[u32]| spans.iter().map(|span| *span as usize).sum::<usize>();
        let head_items = count(&self.spans[..head]);
        let tail_spans = &self.spans[self.spans.len() - tail..];
        let tail_items = &self.items[self.items.len() - count(tail_spans)..];
        let mut items = Vec::with_capacity(self.items.len() + keys.len() - head - tail);
        let mut spans = Vec::with_capacity(keys.len());
        items.extend_from_slice(&self.items[..head_items]);
        spans.extend_from_slice(&self.spans[..head]);
        for key in &keys[head..keys.len() - tail] {
            let row = self.rows.get(key).map_or(&[][..], |built| &built.items);
            items.extend_from_slice(row);
            spans.push(row.len() as u32);
        }
        items.extend_from_slice(tail_items);
        spans.extend_from_slice(tail_spans);
        self.order = keys;
        self.items = items;
        self.spans = spans;
    }

    /// Build the rows under `host`, placed already, and keep them bound to
    /// the items.
    #[inline(never)]
    fn mount(
        mut self,
        vb: &mut ViewBuilder<'_, '_, '_>,
        host: StableNodeId,
        parent_fragment: Option<FragmentId>,
        boxed: bool,
        site: &'static Location<'static>,
    ) {
        let frag = FragmentId::next();
        let effect = reactive::create_effect(vb.st.tag, EffectTarget::Fragment(frag), None, site);
        let (changes, fresh) = reactive::run_tracked(effect, || {
            Self::read(&*self.source, &self.order, &self.rows)
        });
        let mut fresh = Some(fresh);
        let mut built = Vec::new();
        let scope = self.scope;
        if boxed {
            vb.nest_dyn(host, &mut |vb| {
                vb.record_fragment(frag);
                vb.build_fragment_slots(host, frag, &mut |vb| {
                    if let Some(fresh) = fresh.take() {
                        built = Self::build_rows(vb, scope, fresh);
                    }
                });
            });
        } else {
            vb.record_fragment(frag);
            vb.build_fragment_slots(host, frag, &mut |vb| {
                if let Some(fresh) = fresh.take() {
                    built = Self::build_rows(vb, scope, fresh);
                }
            });
        }
        // The rows it held borrow the source, which goes into the decl below.
        drop(fresh);
        self.rows.extend(built);
        self.settle(changes);
        let items = self.items.clone();
        vb.st.parts.fragments.push(FragmentDecl {
            id: frag,
            host,
            parent: parent_fragment,
            boxed,
            effect,
            items,
            binding: Box::new(self),
        });
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
        let boxed =
            stack.is_some() || key.is_some() || !style.is_empty() || vb.fragment_host().is_none();
        if boxed {
            let stack = stack.unwrap_or_else(|| Stack::column(0.0));
            if let Some(container) = container(stack.structural(), key, style, site).place(vb) {
                each_binding(items, key_fn, row_fn, transition).mount(
                    vb,
                    container.stable_id(),
                    None,
                    true,
                    site,
                );
            }
        } else if let Some(host) = vb.fragment_host() {
            let parent_fragment = vb.parent_fragment();
            each_binding(items, key_fn, row_fn, transition).mount(
                vb,
                host,
                parent_fragment,
                false,
                site,
            );
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
            each_binding(items, key_fn, row_fn, transition).mount(
                vb,
                container.stable_id(),
                None,
                true,
                site,
            );
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

impl<K: Eq + Hash + Clone + Send + 'static> FragmentBinding for EachBinding<K> {
    #[inline(never)]
    fn update(
        &mut self,
        cx: &mut AppContext,
        frag: FragmentId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let Some(host) = cx.fragment_host(frag) else {
            return Ok(());
        };
        let (changes, fresh) = reactive::run_tracked(effect, || {
            Self::read(&*self.source, &self.order, &self.rows)
        });
        let transition = self.transition;
        let moves = transition.and_then(|transition| transition.moves);
        let removed = changes
            .removed
            .iter()
            .filter_map(|key| self.rows.remove(key))
            .collect();
        // Where kept rows are before the change, to slide them from.
        let firsts: Vec<_> = match moves {
            Some(_) => {
                let mut nodes = Vec::new();
                for built in self.rows.values() {
                    cx.flatten_slot_items(&built.items, host, &mut nodes);
                }
                nodes
                    .into_iter()
                    .filter_map(|root| Some((root, cx.flip_first(root)?)))
                    .collect()
            }
            None => Vec::new(),
        };
        self.leaving.retain(|root| cx.is_leaving(*root));
        remove(cx, removed, transition.as_ref(), &mut self.leaving)?;
        let mut entered = Vec::new();
        if fresh.is_empty() {
            // Dropped here: the rows it would build borrow the source.
            drop(fresh);
        } else {
            let scope = self.scope;
            let root_keys = fragment_root_keys(cx, frag, host, false);
            let built = build_detached_into(cx, host, Some(frag), root_keys, |vb, created| {
                let built = Self::build_rows(vb, scope, fresh);
                created.extend(built.iter().map(|(_, built)| built.scope));
                built
            })?;
            if transition.is_some_and(|transition| transition.enter.is_some()) {
                for (_, built) in &built {
                    cx.flatten_slot_items(&built.items, host, &mut entered);
                }
            }
            self.rows.extend(built);
        }
        self.settle(changes);
        cx.set_fragment_items(frag, self.items.clone(), self.leaving.clone());
        let mut desired = Vec::new();
        cx.flatten_slot_items(&self.items, host, &mut desired);
        let ordered = if self.leaving.is_empty() {
            desired
        } else {
            with_leaving(cx, host, desired, &self.leaving)
        };
        cx.splice_fragment_children(frag, &ordered)?;
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

    fn teardown(
        &mut self,
        cx: &mut AppContext,
        visible_nodes: &mut Vec<StableNodeId>,
        immediate_despawn: &mut Vec<StableNodeId>,
    ) {
        for (_, built) in self.rows.drain() {
            teardown_built(cx, built, visible_nodes, immediate_despawn);
        }
        immediate_despawn.append(&mut self.leaving);
    }

    fn forget(&mut self, cx: &mut AppContext) {
        for (_, built) in self.rows.drain() {
            forget_built(cx, built);
        }
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
    let boxed = grow || node_key.is_some() || !style.is_empty() || vb.fragment_host().is_none();
    let (host, parent_fragment) = if boxed {
        let mut column = Stack::column(0.0);
        if grow {
            let layout = column.layout_mut();
            layout.flex_grow = Some(1.0);
            layout.flex_shrink = Some(1.0);
            layout.flex_basis = Some(nana_ui_core::LengthSpec::Px(0.0));
            layout.min_height = Some(nana_ui_core::LengthSpec::Px(0.0));
        }
        let Some(container) = container(column.structural(), node_key, style, site).place(vb)
        else {
            return;
        };
        (container.stable_id(), None)
    } else if let Some(host) = vb.fragment_host() {
        (host, vb.parent_fragment())
    } else {
        return;
    };
    let frag = FragmentId::next();
    let effect = reactive::create_effect(vb.st.tag, EffectTarget::Fragment(frag), None, site);
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
    if boxed {
        vb.nest_dyn(host, &mut |vb| {
            vb.record_fragment(frag);
            vb.build_fragment_slots(host, frag, &mut |vb| {
                if binding.render.exists(&binding.shown) {
                    let (render, shown) = (&binding.render, &binding.shown);
                    binding.branch = Some(build_scoped(vb, binding.scope, || render.make(shown)));
                }
            });
        });
    } else {
        vb.record_fragment(frag);
        vb.build_fragment_slots(host, frag, &mut |vb| {
            if binding.render.exists(&binding.shown) {
                let (render, shown) = (&binding.render, &binding.shown);
                binding.branch = Some(build_scoped(vb, binding.scope, || render.make(shown)));
            }
        });
    }
    let items = binding
        .branch
        .as_ref()
        .map(|branch| branch.items.clone())
        .unwrap_or_default();
    vb.st.parts.fragments.push(FragmentDecl {
        id: frag,
        host,
        parent: parent_fragment,
        boxed,
        effect,
        items,
        binding: Box::new(binding),
    });
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
    /// their roots held under the host: a branch shown after them may take
    /// the same keys, and a kept branch shown again takes them back.
    kept: Vec<(K, Built, Vec<crate::framework::AssembledKey>)>,
    /// The parked hidden stack kept branches wait in, created on first use.
    holder: Option<StableNodeId>,
    leaving: Vec<StableNodeId>,
}

impl<K: Clone + PartialEq + Send + 'static> SwitchBinding<K> {
    /// Move `branch` into the parked hidden holder, dropping the oldest kept
    /// branches beyond the limit.
    fn stash(
        &mut self,
        cx: &mut AppContext,
        frag: FragmentId,
        host: StableNodeId,
        key: K,
        branch: Built,
        limit: usize,
    ) -> Result<(), FrameworkError> {
        let holder = match self.holder.filter(|holder| cx.world().contains(*holder)) {
            Some(holder) => holder,
            None => {
                let document = cx
                    .world()
                    .node(host)
                    .ok_or(FrameworkError::MissingView(host))?
                    .document;
                let hidden = Stack::column(0.0).with_layout(|layout| layout.hidden = true);
                let holder = cx.create_detached_component(document, hidden)?.stable_id();
                self.holder = Some(holder);
                cx.mark_out_of_host_fragment(frag, true);
                holder
            }
        };
        let mut all_nodes = Vec::new();
        cx.flatten_all_live_nodes(&branch.items, &mut all_nodes);
        let mut mutations = MutationQueue::new();
        if let Some(document) = cx.world().node(host).map(|node| node.document)
            && cx.world().focused(document).is_some_and(|focused| {
                all_nodes
                    .iter()
                    .any(|root| cx.world().is_descendant_or_self(focused, *root))
            })
        {
            mutations.request_focus(document, None);
        }
        let mut host_nodes = Vec::new();
        cx.flatten_slot_items(&branch.items, host, &mut host_nodes);
        for root in host_nodes {
            if cx.world().contains(root) {
                mutations.insert(holder, root, None);
            }
        }
        cx.retarget_slot_items(&branch.items, host, holder);
        let keys = all_nodes
            .iter()
            .filter_map(|root| cx.assembled_key(host, *root))
            .collect();
        self.kept.push((key, branch, keys));
        while self.kept.len() > limit {
            let (_, evicted, _) = self.kept.remove(0);
            let mut evict_vis = Vec::new();
            let mut evict_imm = Vec::new();
            teardown_built(cx, evicted, &mut evict_vis, &mut evict_imm);
            for root in evict_vis.into_iter().chain(evict_imm) {
                if cx.world().contains(root) {
                    mutations.despawn_subtree(root);
                }
            }
        }
        cx.commit_mutations(mutations).map(|_| ())
    }
}

impl<K: Clone + PartialEq + Send + 'static> FragmentBinding for SwitchBinding<K> {
    fn update(
        &mut self,
        cx: &mut AppContext,
        frag: FragmentId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let Some(host) = cx.fragment_host(frag) else {
            return Ok(());
        };
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
                Some(limit) => self.stash(cx, frag, host, previous, branch, limit)?,
                None => remove(cx, vec![branch], transition.as_ref(), &mut self.leaving)?,
            }
        }
        let branch = match kept {
            Some((_, branch, keys)) => {
                for key in keys {
                    cx.rekey_assembled(host, key);
                }
                if let Some(holder) = self.holder {
                    cx.retarget_slot_items(&branch.items, holder, host);
                }
                Some(branch)
            }
            None if self.render.exists(&shown) => {
                let (render, scope) = (&self.render, self.scope);
                let root_keys = fragment_root_keys(cx, frag, host, true);
                Some(build_detached_into(
                    cx,
                    host,
                    Some(frag),
                    root_keys,
                    |vb, created| {
                        let built = build_scoped(vb, scope, || render.make(&shown));
                        created.push(built.scope);
                        built
                    },
                )?)
            }
            None => None,
        };
        let items = branch
            .as_ref()
            .map(|branch| branch.items.clone())
            .unwrap_or_default();
        self.branch = branch;
        cx.set_fragment_items(frag, items.clone(), self.leaving.clone());
        let mut roots = Vec::new();
        cx.flatten_slot_items(&items, host, &mut roots);
        let ordered = self
            .leaving
            .iter()
            .copied()
            .chain(roots.iter().copied())
            .collect::<Vec<_>>();
        cx.splice_fragment_children(frag, &ordered)?;
        if let Some(enter) = transition.and_then(|transition| transition.enter) {
            cx.begin_enter(&roots, &enter)?;
        }
        Ok(())
    }

    fn collect_external_nodes(&self, cx: &AppContext, out: &mut Vec<StableNodeId>) {
        if let Some(holder) = self.holder
            && cx.world().contains(holder)
        {
            out.push(holder);
        }
        for (_, branch, _) in &self.kept {
            let mut nodes = Vec::new();
            cx.flatten_all_live_nodes(&branch.items, &mut nodes);
            for node in nodes {
                if self
                    .holder
                    .is_none_or(|holder| !cx.world().is_descendant_or_self(node, holder))
                {
                    out.push(node);
                }
            }
        }
    }

    fn teardown(
        &mut self,
        cx: &mut AppContext,
        visible_nodes: &mut Vec<StableNodeId>,
        immediate_despawn: &mut Vec<StableNodeId>,
    ) {
        if let Some(branch) = self.branch.take() {
            teardown_built(cx, branch, visible_nodes, immediate_despawn);
        }
        for (_, branch, _) in self.kept.drain(..) {
            let mut kept_nodes = Vec::new();
            teardown_built(cx, branch, &mut kept_nodes, immediate_despawn);
            immediate_despawn.extend(kept_nodes);
        }
        if let Some(holder) = self.holder.take() {
            immediate_despawn.push(holder);
        }
        immediate_despawn.append(&mut self.leaving);
    }

    fn forget(&mut self, cx: &mut AppContext) {
        if let Some(branch) = self.branch.take() {
            forget_built(cx, branch);
        }
        for (_, branch, _) in self.kept.drain(..) {
            forget_built(cx, branch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator: the cases must not change between
    /// runs.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as usize) % bound.max(1)
        }
    }

    /// `old` after a few edits of every kind a list sees: rows inserted,
    /// removed and moved, and keys repeated anywhere, the unchanged start
    /// and end included.
    fn edited(old: &[u32], random: &mut Lcg) -> Vec<u32> {
        let mut items = old.to_vec();
        for _ in 0..random.below(4) {
            let at = random.below(items.len() + 1);
            match random.below(5) {
                0 => items.insert(at, 1_000 + random.below(50) as u32),
                1 if !items.is_empty() => {
                    items.remove(at.min(items.len() - 1));
                }
                2 if items.len() > 1 => {
                    let from = random.below(items.len());
                    let key = items.remove(from);
                    items.insert(random.below(items.len() + 1), key);
                }
                3 if !items.is_empty() => {
                    let again = items[random.below(items.len())];
                    items.insert(at, again);
                }
                4 if !old.is_empty() => {
                    items.insert(at, old[random.below(old.len())]);
                }
                _ => {}
            }
        }
        items
    }

    #[test]
    fn the_changed_part_of_a_list_answers_what_hashing_every_key_does() {
        let mut random = Lcg(0x5eed);
        let mut settled_between = 0;
        for _ in 0..20_000 {
            let old: Vec<u32> = (0..random.below(12) as u32).collect();
            let items = edited(&old, &mut random);
            let built = |key: &u32| old.contains(key);
            let whole = Changes::across(&old, items.clone(), built);
            let part = Changes::between(&old, items.clone(), built);
            assert_eq!(part.keys, whole.keys, "{old:?} to {items:?}");
            assert_eq!(part.fresh, whole.fresh, "{old:?} to {items:?}");
            assert_eq!(part.removed, whole.removed, "{old:?} to {items:?}");
            // The ends it reports unchanged are.
            assert_eq!(part.keys[..part.head], old[..part.head]);
            assert_eq!(
                part.keys[part.keys.len() - part.tail..],
                old[old.len() - part.tail..]
            );
            settled_between += usize::from(part.head + part.tail > 0);
        }
        assert!(settled_between > 10_000, "{settled_between} of 20,000");
    }

    #[test]
    fn one_row_more_or_less_leaves_every_other_key_unhashed() {
        let old: Vec<u32> = (0..1_000).collect();
        let asked = std::cell::Cell::new(0);
        let built = |key: &u32| {
            asked.set(asked.get() + 1);
            *key < 1_000
        };
        let mut items = old.clone();
        items.insert(500, 5_000);
        let changes = Changes::between(&old, items, built);
        assert_eq!(changes.fresh, [500]);
        assert!(changes.removed.is_empty());
        assert_eq!((changes.head, changes.tail), (500, 500));
        let mut items = old.clone();
        items.remove(250);
        let changes = Changes::between(&old, items, built);
        assert_eq!(changes.removed, [250]);
        assert_eq!((changes.head, changes.tail), (250, 749));
        assert_eq!(
            asked.get(),
            1,
            "only the inserted key is looked up among the rows"
        );
    }
}
