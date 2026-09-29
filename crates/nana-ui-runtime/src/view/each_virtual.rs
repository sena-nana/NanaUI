//! A keyed list whose rows exist only while they are in view.
//!
//! [`each`](super::each) builds every row, which is right for tens or
//! hundreds of them and wrong for tens of thousands: every row is nodes,
//! bindings and a scope, about 2–3 KB live. [`each_virtual`] lays rows out at
//! a fixed height inside a `ScrollView` and builds only those the viewport
//! (plus overscan) covers, through the retained virtual list the Runtime
//! already has: a row scrolled away is despawned with its scope, a row that
//! holds focus or an IME composition is kept. With [`EachVirtual::measured`]
//! rows size to their content, the given height only an estimate.

use std::borrow::Cow;
use std::hash::Hash;
use std::marker::PhantomData;
use std::panic::Location;

use hashbrown::{HashMap, HashSet};
use nana_ui_core::VirtualListLayout;

use super::node::{IntoView, StructuralBinding, UNBUILT, ViewBuilder};
use super::reactive::{self, EffectKey, EffectTarget, Readable, ScopeKey, Signal};
use super::structural::{build_detached_into, build_scoped};
use crate::{
    AppContext, Entity, FrameworkError, List, MutationQueue, ScrollAxes, ScrollChanged,
    ScrollLaidOut, ScrollView, ScrollViewportChanged, StableNodeId, Stack, VirtualListItems,
};

/// `v-for` over many rows: see the module docs.
pub struct EachVirtual<T, K, S, KF, RF> {
    items: S,
    key_fn: KF,
    row_fn: RF,
    row_height: f32,
    measured: bool,
    overscan: f32,
    scroll: ScrollView,
    key: Option<Cow<'static, str>>,
    site: &'static Location<'static>,
    _types: PhantomData<fn(T) -> K>,
}

/// Keyed list over `items` whose rows, `row_height` tall, are built only
/// while they are in view. Duplicate keys after the first are skipped.
#[track_caller]
pub fn each_virtual<T, K, S, KF, RF, V>(
    items: S,
    key: KF,
    row_height: f32,
    row: RF,
) -> EachVirtual<T, K, S, KF, RF>
where
    T: Clone + Send + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    EachVirtual {
        items,
        key_fn: key,
        row_fn: row,
        row_height: row_height.max(1.0),
        measured: false,
        overscan: row_height.max(1.0) * 4.0,
        scroll: ScrollView::new(ScrollAxes::Vertical),
        key: None,
        site: Location::caller(),
        _types: PhantomData,
    }
}

impl<T, K, S, KF, RF> EachVirtual<T, K, S, KF, RF> {
    /// How far past the viewport rows are kept built, in pixels (default:
    /// four rows).
    pub fn overscan(mut self, overscan: f32) -> Self {
        self.overscan = overscan.max(0.0);
        self
    }

    /// Rows size to their content; the row height given is the estimate for
    /// rows not measured yet. Each layout pass that shows new rows is
    /// followed by one that places them at their measured heights, keeping
    /// the row at the top of the viewport where it is.
    pub fn measured(mut self) -> Self {
        self.measured = true;
        self
    }

    /// The list's height in pixels (the scroll container's).
    pub fn height(mut self, height: f32) -> Self {
        self.scroll = self
            .scroll
            .with_layout(|layout| layout.height = Some(nana_ui_core::LengthSpec::Px(height)));
        self
    }

    /// The list's width in pixels.
    pub fn width(mut self, width: f32) -> Self {
        self.scroll = self
            .scroll
            .with_layout(|layout| layout.width = Some(nana_ui_core::LengthSpec::Px(width)));
        self
    }

    /// Take the space left along the parent's main axis.
    pub fn grow(mut self) -> Self {
        self.scroll = self.scroll.with_layout(|layout| {
            layout.flex_grow = Some(1.0);
            layout.flex_shrink = Some(1.0);
            layout.flex_basis = Some(nana_ui_core::LengthSpec::Px(0.0));
        });
        self
    }

    /// The scroll container rows live in, to size or style it. It must scroll
    /// vertically.
    pub fn scroll_view(mut self, scroll: ScrollView) -> Self {
        self.scroll = scroll;
        self
    }

    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.key = Some(key.into());
        self
    }
}

struct EachVirtualBinding<T, K, S, KF, RF> {
    items: S,
    key_fn: KF,
    row_fn: RF,
    scope: Option<ScopeKey>,
    /// Written by scroll and viewport events: the window moved.
    moved: Signal<u64>,
    scroll: Entity<ScrollView>,
    list: Entity<List>,
    state: VirtualListItems<K, Stack>,
    layout: VirtualListLayout,
    row_height: f32,
    measured: bool,
    overscan: f32,
    keys: Vec<K>,
    index: HashMap<K, usize>,
    /// Bumped when the keys change, so the range gate re-materializes.
    version: u64,
    _types: PhantomData<fn(T)>,
}

impl<T, K, S, KF, RF, V> EachVirtualBinding<T, K, S, KF, RF>
where
    T: Clone + Send + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    /// Every item with its key, first of each key only; reads what the
    /// binding depends on.
    fn read(&self) -> Vec<(K, T)> {
        self.moved.get();
        self.items.with_value(|items| {
            let mut seen = HashSet::with_capacity(items.len());
            items
                .iter()
                .filter_map(|item| {
                    let key = (self.key_fn)(item);
                    seen.insert(key.clone()).then(|| (key, item.clone()))
                })
                .collect()
        })
    }

    fn sync(&mut self, cx: &mut AppContext, items: Vec<(K, T)>) -> Result<(), FrameworkError> {
        let keys_changed = items.len() != self.keys.len()
            || items
                .iter()
                .zip(&self.keys)
                .any(|((key, _), kept)| key != kept);
        if keys_changed {
            if items.len() != self.keys.len() {
                self.layout =
                    VirtualListLayout::new(std::iter::repeat_n(self.row_height, items.len()));
            }
            self.keys = items.iter().map(|(key, _)| key.clone()).collect();
            self.index = self
                .keys
                .iter()
                .enumerate()
                .map(|(index, key)| (key.clone(), index))
                .collect();
            self.version += 1;
        }
        let (keys, index, row_fn, scope) = (&self.keys, &self.index, &self.row_fn, self.scope);
        let key_at = |at: usize| keys[at].clone();
        let index_of = |key: &K| index.get(key).copied();
        let mount = |cx: &mut AppContext, slot: Entity<Stack>, at: usize, _: &K| {
            mount_row(cx, slot.stable_id(), scope, || row_fn(items[at].1.clone()))
        };
        if self.measured {
            cx.sync_virtual_list_measured_with(
                self.scroll,
                self.list,
                &mut self.state,
                &mut self.layout,
                self.overscan,
                self.version,
                &[],
                key_at,
                index_of,
                |_, _| Stack::column(0.0),
                mount,
            )?;
            // Rows mounted now are measured after the next layout pass.
            if self.state.pending_measure() {
                cx.notify_laid_out(self.scroll);
            }
        } else {
            cx.sync_virtual_list_retained_with(
                self.scroll,
                self.list,
                &mut self.state,
                &self.layout,
                self.overscan,
                self.version,
                &[],
                key_at,
                index_of,
                |_, _| Stack::column(0.0),
                mount,
            )?;
        }
        Ok(())
    }
}

/// Build one row's view into its placement slot, in a scope the row's roots
/// own: despawning the slot disposes it.
fn mount_row<V: IntoView>(
    cx: &mut AppContext,
    slot: StableNodeId,
    scope: Option<ScopeKey>,
    row: impl FnOnce() -> V,
) -> Result<(), FrameworkError> {
    let built = build_detached_into(cx, slot, |vb, created| {
        let built = build_scoped(vb, scope, || row().into_any());
        created.push(built.scope);
        built
    })?;
    let mut mutations = MutationQueue::new();
    for root in &built.roots {
        mutations.insert(slot, *root, None);
    }
    cx.commit_mutations(mutations).map(|_| ())
}

impl<T, K, S, KF, RF, V> IntoView for EachVirtual<T, K, S, KF, RF>
where
    T: Clone + Send + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let scroll = vb.place(self.key, self.scroll);
        let id = scroll.stable_id();
        if id == UNBUILT {
            return;
        }
        let list = vb.ui.nest(scroll, |ui| ui.child("list", List::new()));
        let moved = super::signal(0u64);
        vb.ui.on(scroll, move |_, _: &ScrollChanged, _| {
            moved.update(|at| *at += 1)
        });
        vb.ui.on(scroll, move |_, _: &ScrollViewportChanged, _| {
            moved.update(|at| *at += 1)
        });
        if self.measured {
            vb.ui.on(scroll, move |_, _: &ScrollLaidOut, _| {
                moved.update(|at| *at += 1)
            });
        }
        let effect =
            reactive::create_effect(vb.st.tag, EffectTarget::Structural(id), None, self.site);
        let binding = EachVirtualBinding {
            items: self.items,
            key_fn: self.key_fn,
            row_fn: self.row_fn,
            scope: reactive::current_scope(),
            moved,
            scroll,
            list,
            state: if self.measured {
                VirtualListItems::measured()
            } else {
                VirtualListItems::default()
            },
            layout: VirtualListLayout::new([]),
            row_height: self.row_height,
            measured: self.measured,
            overscan: self.overscan,
            keys: Vec::new(),
            index: HashMap::new(),
            version: 0,
            _types: PhantomData,
        };
        // Subscribe now; the first window follows the first layout, whose
        // viewport event moves it.
        reactive::run_tracked(effect, || binding.read());
        vb.st.parts.structural.push((id, effect, Box::new(binding)));
    }
}

impl<T, K, S, KF, RF, V> StructuralBinding for EachVirtualBinding<T, K, S, KF, RF>
where
    T: Clone + Send + 'static,
    K: Eq + Hash + Clone + Send + 'static,
    S: Readable<Vec<T>>,
    KF: Fn(&T) -> K + Send + 'static,
    RF: Fn(T) -> V + Send + 'static,
    V: IntoView,
{
    fn update(
        &mut self,
        cx: &mut AppContext,
        _container: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let items = reactive::run_tracked(effect, || self.read());
        self.sync(cx, items)
    }
}
