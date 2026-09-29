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

use super::node::{AnyView, IntoView, StructuralBinding, UNBUILT, ViewBuilder, ViewState};
use super::prop::{IntoProp, PropSource};
use super::reactive::{self, EffectKey, EffectTarget, Readable, ScopeKey};
use crate::{AppContext, FrameworkError, MutationQueue, StableNodeId, Stack};

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
/// it creates so a failed commit disposes them.
pub(super) fn build_detached_into<R>(
    cx: &mut AppContext,
    container: StableNodeId,
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
        let built = build(&mut ViewBuilder { ui, st: &mut st }, &mut created);
        (built, st.parts)
    });
    match result {
        Ok((built, parts)) => {
            cx.install_view_parts(parts);
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

fn despawn(cx: &mut AppContext, removed: Vec<Built>) -> Result<(), FrameworkError> {
    let mut mutations = MutationQueue::new();
    for built in removed {
        reactive::dispose_scope(built.scope);
        for root in built.roots {
            if cx.world().contains(root) {
                mutations.despawn_subtree(root);
            }
        }
    }
    if mutations.is_empty() {
        return Ok(());
    }
    cx.commit_mutations(mutations).map(|_| ())
}

/// `v-for` with `:key`. Rows whose key stays are kept as they are; a row
/// follows data changes through its own signals, not by being rebuilt.
/// Duplicate keys after the first are skipped.
pub struct Each<T, K, S, KF, RF> {
    items: S,
    key_fn: KF,
    row_fn: RF,
    container: Stack,
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
        container: Stack::column(0.0),
        key: None,
        site: Location::caller(),
        _types: PhantomData,
    }
}

impl<T, K, S, KF, RF> Each<T, K, S, KF, RF> {
    /// Gap between rows.
    pub fn gap(mut self, gap: f32) -> Self {
        self.container = Stack::column(gap);
        self
    }

    /// Lay rows out horizontally.
    pub fn horizontal(mut self, gap: f32) -> Self {
        self.container = Stack::row(gap);
        self
    }

    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.key = Some(key.into());
        self
    }
}

struct EachBinding<T, K, S, KF, RF> {
    items: S,
    key_fn: KF,
    row_fn: RF,
    scope: Option<ScopeKey>,
    rows: HashMap<K, Built>,
    order: Vec<K>,
    _types: PhantomData<fn(T)>,
}

impl<T, K, S, KF, RF, V> EachBinding<T, K, S, KF, RF>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    /// The keys in order, and the items whose keys are not built yet.
    fn read(&self) -> (Vec<K>, Vec<(K, T)>) {
        self.items.with_value(|items| {
            let mut keys = Vec::with_capacity(items.len());
            let mut seen = HashSet::with_capacity(items.len());
            let mut fresh = Vec::new();
            for item in items {
                let key = (self.key_fn)(item);
                if !seen.insert(key.clone()) {
                    continue;
                }
                if !self.rows.contains_key(&key) {
                    fresh.push((key.clone(), item.clone()));
                }
                keys.push(key);
            }
            (keys, fresh)
        })
    }

    fn build_rows(&self, vb: &mut ViewBuilder<'_, '_, '_>, fresh: Vec<(K, T)>) -> Vec<(K, Built)> {
        fresh
            .into_iter()
            .map(|(key, item)| {
                let built = build_scoped(vb, self.scope, || (self.row_fn)(item).into_any());
                (key, built)
            })
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
        let key = vb.key_or_auto(self.key);
        let container = vb.ui.child(key, self.container);
        let id = container.stable_id();
        if id == UNBUILT {
            return;
        }
        vb.push_root(id);
        let effect =
            reactive::create_effect(vb.st.tag, EffectTarget::Structural(id), None, self.site);
        let mut binding = EachBinding {
            items: self.items,
            key_fn: self.key_fn,
            row_fn: self.row_fn,
            scope: reactive::current_scope(),
            rows: HashMap::new(),
            order: Vec::new(),
            _types: PhantomData,
        };
        let (keys, fresh) = reactive::run_tracked(effect, || binding.read());
        vb.nest(container, |vb| {
            let built = binding.build_rows(vb, fresh);
            binding.rows.extend(built);
        });
        binding.order = keys;
        vb.st.parts.structural.push((id, effect, Box::new(binding)));
    }
}

impl<T, K, S, KF, RF, V> StructuralBinding for EachBinding<T, K, S, KF, RF>
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    fn update(
        &mut self,
        cx: &mut AppContext,
        container: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let (keys, fresh) = reactive::run_tracked(effect, || self.read());
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
        despawn(cx, removed)?;
        if !fresh.is_empty() {
            let this = &*self;
            let built = build_detached_into(cx, container, |vb, created| {
                let built = this.build_rows(vb, fresh);
                created.extend(built.iter().map(|(_, built)| built.scope));
                built
            })?;
            self.rows.extend(built);
        }
        self.order = keys;
        let ordered: Vec<StableNodeId> = self
            .order
            .iter()
            .filter_map(|key| self.rows.get(key))
            .flat_map(|built| built.roots.iter().copied())
            .collect();
        cx.reconcile_children(container, &ordered)?;
        Ok(())
    }
}

/// `v-if` / `v-else`. The branch that is not shown does not exist: its
/// nodes are despawned and its scope disposed.
pub struct When {
    condition: PropSource<bool>,
    then: Box<dyn Fn() -> AnyView + Send>,
    otherwise: Option<Box<dyn Fn() -> AnyView + Send>>,
    key: Option<Cow<'static, str>>,
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
        key: None,
        site: Location::caller(),
    }
}

impl When {
    /// Show `otherwise` while the condition does not hold.
    pub fn otherwise<V: IntoView>(mut self, otherwise: impl Fn() -> V + Send + 'static) -> Self {
        self.otherwise = Some(Box::new(move || otherwise().into_any()));
        self
    }

    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.key = Some(key.into());
        self
    }
}

struct WhenBinding {
    condition: PropSource<bool>,
    then: Box<dyn Fn() -> AnyView + Send>,
    otherwise: Option<Box<dyn Fn() -> AnyView + Send>>,
    scope: Option<ScopeKey>,
    shown: bool,
    branch: Option<Built>,
}

impl WhenBinding {
    fn branch_for(&self, shown: bool) -> Option<&(dyn Fn() -> AnyView + Send)> {
        if shown {
            Some(&*self.then)
        } else {
            self.otherwise.as_deref()
        }
    }
}

impl IntoView for When {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let key = vb.key_or_auto(self.key);
        let container = vb.ui.child(key, Stack::column(0.0));
        let id = container.stable_id();
        if id == UNBUILT {
            return;
        }
        vb.push_root(id);
        let effect =
            reactive::create_effect(vb.st.tag, EffectTarget::Structural(id), None, self.site);
        let mut binding = WhenBinding {
            condition: self.condition,
            then: self.then,
            otherwise: self.otherwise,
            scope: reactive::current_scope(),
            shown: false,
            branch: None,
        };
        binding.shown = reactive::run_tracked(effect, || binding.condition.get());
        vb.nest(container, |vb| {
            if let Some(branch) = binding.branch_for(binding.shown) {
                binding.branch = Some(build_scoped(vb, binding.scope, branch));
            }
        });
        vb.st.parts.structural.push((id, effect, Box::new(binding)));
    }
}

impl StructuralBinding for WhenBinding {
    fn update(
        &mut self,
        cx: &mut AppContext,
        container: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let shown = reactive::run_tracked(effect, || self.condition.get());
        if shown == self.shown {
            return Ok(());
        }
        self.shown = shown;
        despawn(cx, self.branch.take().into_iter().collect())?;
        let roots = match self.branch_for(shown) {
            Some(_) => {
                let this = &*self;
                let built = build_detached_into(cx, container, |vb, created| {
                    let branch = this.branch_for(shown).expect("branch exists");
                    let built = build_scoped(vb, this.scope, branch);
                    created.push(built.scope);
                    built
                })?;
                let roots = built.roots.clone();
                self.branch = Some(built);
                roots
            }
            None => Vec::new(),
        };
        cx.reconcile_children(container, &roots)?;
        Ok(())
    }
}
