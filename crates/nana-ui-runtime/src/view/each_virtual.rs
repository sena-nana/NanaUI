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
//! [`EachVirtual::within`] lets the rows scroll with an ancestor
//! `ScrollView` (a page's) instead of one of their own, and
//! [`EachVirtual::grid`] flows items into as many columns as fit.

use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Range;
use std::panic::Location;

use hashbrown::{HashMap, HashSet};
use nana_ui_core::VirtualListLayout;

use super::node::{AnyView, IntoView, NodeRef, StructuralBinding, UNBUILT, ViewBuilder, widget};
use super::reactive::{self, EffectKey, EffectTarget, Readable, ScopeKey, Signal};
use super::structural::{build_detached_into, build_scoped};
use crate::{
    AppContext, Entity, FrameworkError, List, MutationQueue, ScrollAxes, ScrollChanged,
    ScrollLaidOut, ScrollView, ScrollViewportChanged, StableNodeId, Stack, VirtualListItems,
};
use nana_ui_core::{AlignSpec, LengthSpec};

/// Where the rows scroll.
enum Container {
    /// A `ScrollView` of the list's own.
    Own(Box<ScrollView>),
    /// An ancestor's, known once it is built.
    Within(NodeRef),
}

/// [`EachVirtual::grid`]: columns at least `min_width` wide, `gap` apart
/// both ways.
#[derive(Clone, Copy)]
struct Grid {
    min_width: f32,
    gap: f32,
}

/// What the virtual list places: one item, or one grid row of them.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Unit<K> {
    Item(K),
    /// A grid row, by its column count and the keys in it.
    Row(u64),
}

/// `v-for` over many rows: see the module docs.
pub struct EachVirtual<T, K, S, KF, RF> {
    items: S,
    key_fn: KF,
    row_fn: RF,
    row_height: f32,
    measured: bool,
    overscan: f32,
    container: Container,
    grid: Option<Grid>,
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
        container: Container::Own(Box::new(ScrollView::new(ScrollAxes::Vertical))),
        grid: None,
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

    /// The list's height in pixels (the scroll container's). Not with
    /// [`Self::within`].
    pub fn height(self, height: f32) -> Self {
        self.own_scroll(|scroll| {
            scroll.with_layout(|layout| layout.height = Some(LengthSpec::Px(height)))
        })
    }

    /// The list's width in pixels. Not with [`Self::within`].
    pub fn width(self, width: f32) -> Self {
        self.own_scroll(|scroll| {
            scroll.with_layout(|layout| layout.width = Some(LengthSpec::Px(width)))
        })
    }

    /// Take the space left along the parent's main axis. Not with
    /// [`Self::within`].
    pub fn grow(self) -> Self {
        self.own_scroll(|scroll| {
            scroll.with_layout(|layout| {
                layout.flex_grow = Some(1.0);
                layout.flex_shrink = Some(1.0);
                layout.flex_basis = Some(LengthSpec::Px(0.0));
            })
        })
    }

    /// The scroll container rows live in, to size or style it. It must scroll
    /// vertically.
    pub fn scroll_view(mut self, scroll: ScrollView) -> Self {
        self.container = Container::Own(Box::new(scroll));
        self
    }

    /// Scroll with `scroll`, an ancestor `ScrollView`, instead of a scroll
    /// container of the list's own: a list under a page's header and above
    /// its footer. The window is the part of that viewport the list covers,
    /// and follows it as content above the list grows or shrinks.
    pub fn within(mut self, scroll: NodeRef) -> Self {
        self.container = Container::Within(scroll);
        self
    }

    /// Flow the items into rows of as many columns at least `min_width` wide
    /// as the list's width fits, `gap` apart across and down; the row height
    /// is one grid row's (without the gap). Columns share the width equally,
    /// and a short last row keeps the others' column width.
    pub fn grid(mut self, min_width: f32, gap: f32) -> Self {
        self.grid = Some(Grid {
            min_width: min_width.max(1.0),
            gap: gap.max(0.0),
        });
        self
    }

    fn own_scroll(mut self, f: impl FnOnce(ScrollView) -> ScrollView) -> Self {
        if let Container::Own(scroll) = self.container {
            self.container = Container::Own(Box::new(f(*scroll)));
        }
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
    /// The ancestor to scroll with, until it is resolved into `scroll`.
    within: Option<NodeRef>,
    scroll: Option<Entity<ScrollView>>,
    list: Entity<List>,
    grid: Option<Grid>,
    /// A grid's column count at the last sync.
    columns: usize,
    state: VirtualListItems<Unit<K>, Stack>,
    layout: VirtualListLayout,
    row_height: f32,
    measured: bool,
    overscan: f32,
    /// What each placed unit is, in order, and the items it shows.
    units: Vec<Unit<K>>,
    members: Vec<Range<usize>>,
    index: HashMap<Unit<K>, usize>,
    /// Bumped when the units change, so the range gate re-materializes.
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
        if let Some(within) = self.within {
            within.get();
        }
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

    /// The scroll container, observing an ancestor's events the first time
    /// it is known. The list observes them, so they end with it.
    fn scroll(
        &mut self,
        cx: &mut AppContext,
    ) -> Result<Option<Entity<ScrollView>>, FrameworkError> {
        if self.scroll.is_some() {
            return Ok(self.scroll);
        }
        let Some(id) = self.within.and_then(|within| within.get_untracked()) else {
            return Ok(None);
        };
        let scroll = Entity::<ScrollView>::from_stable_id(id);
        let moved = self.moved;
        cx.observe(scroll, self.list, move |_, _: &ScrollChanged, _| {
            moved.update(|at| *at += 1)
        })?;
        cx.observe(scroll, self.list, move |_, _: &ScrollViewportChanged, _| {
            moved.update(|at| *at += 1)
        })?;
        cx.observe(scroll, self.list, move |_, _: &ScrollLaidOut, _| {
            moved.update(|at| *at += 1)
        })?;
        self.scroll = Some(scroll);
        Ok(self.scroll)
    }

    /// Group the items into what the list places: each item, or grid rows
    /// of as many as fit across. `None` while a grid has no width yet.
    fn units(
        &self,
        cx: &AppContext,
        items: &[(K, T)],
    ) -> (usize, Option<Vec<(Unit<K>, Range<usize>)>>) {
        let Some(grid) = self.grid else {
            return (
                1,
                Some(
                    items
                        .iter()
                        .enumerate()
                        .map(|(at, (key, _))| (Unit::Item(key.clone()), at..at + 1))
                        .collect(),
                ),
            );
        };
        let width = cx
            .world()
            .layout_box(self.list.stable_id())
            .map_or(0.0, |bounds| bounds.width);
        if width <= 0.0 {
            return (self.columns, None);
        }
        let columns = grid_columns(width, grid);
        (
            columns,
            Some(
                (0..items.len())
                    .step_by(columns)
                    .map(|start| {
                        let end = (start + columns).min(items.len());
                        let mut hasher = DefaultHasher::new();
                        columns.hash(&mut hasher);
                        for (key, _) in &items[start..end] {
                            key.hash(&mut hasher);
                        }
                        (Unit::Row(hasher.finish()), start..end)
                    })
                    .collect(),
            ),
        )
    }

    fn sync(&mut self, cx: &mut AppContext, items: Vec<(K, T)>) -> Result<(), FrameworkError> {
        let Some(scroll) = self.scroll(cx)? else {
            return Ok(());
        };
        let (columns, units) = self.units(cx, &items);
        let units = units.unwrap_or_default();
        self.columns = columns;
        if units.len() != self.units.len()
            || units
                .iter()
                .zip(&self.units)
                .any(|((unit, _), kept)| unit != kept)
        {
            // Units that stay keep the extent they were placed or measured at.
            let estimate = self.row_height + self.grid.map_or(0.0, |grid| grid.gap);
            let extents = units
                .iter()
                .map(|(unit, _)| {
                    self.index
                        .get(unit)
                        .map_or(estimate, |&at| self.layout.extent(at..at + 1))
                })
                .collect::<Vec<_>>();
            self.layout = VirtualListLayout::new(extents);
            self.index = units
                .iter()
                .enumerate()
                .map(|(at, (unit, _))| (unit.clone(), at))
                .collect();
            (self.units, self.members) = units.into_iter().unzip();
            self.version += 1;
        }
        let (units, members, index) = (&self.units, &self.members, &self.index);
        let (row_fn, scope, grid) = (&self.row_fn, self.scope, self.grid);
        let key_at = |at: usize| units[at].clone();
        let index_of = |unit: &Unit<K>| index.get(unit).copied();
        let slot = |_: usize, _: &Unit<K>| match grid {
            // The gap below a grid row is part of its extent.
            Some(grid) => Stack::column(0.0)
                .with_layout(|layout| layout.padding_bottom = Some(LengthSpec::Px(grid.gap))),
            None => Stack::column(0.0),
        };
        let mount = |cx: &mut AppContext, slot: Entity<Stack>, at: usize, _: &Unit<K>| {
            let range = members[at].clone();
            mount_row(cx, slot.stable_id(), scope, || match grid {
                None => row_fn(items[range.start].1.clone()).into_any(),
                Some(grid) => grid_row(grid, columns, &items[range], row_fn),
            })
        };
        if self.measured {
            cx.sync_virtual_list_measured_with(
                scroll,
                self.list,
                &mut self.state,
                &mut self.layout,
                self.overscan,
                self.version,
                &[],
                key_at,
                index_of,
                slot,
                mount,
            )?;
            // Rows mounted now are measured after the next layout pass.
            if self.state.pending_measure() {
                cx.notify_laid_out(scroll);
            }
        } else {
            cx.sync_virtual_list_retained_with(
                scroll,
                self.list,
                &mut self.state,
                &self.layout,
                self.overscan,
                self.version,
                &[],
                key_at,
                index_of,
                slot,
                mount,
            )?;
        }
        if self.within.is_some() || self.grid.is_some() {
            // Content above the list, or the list's own width, can change
            // with no scroll or viewport change: look again after the next
            // layout pass (one only happens when something changed).
            cx.notify_laid_out(scroll);
        }
        Ok(())
    }
}

fn grid_columns(width: f32, grid: Grid) -> usize {
    (((width + grid.gap) / (grid.min_width + grid.gap)).floor() as usize).max(1)
}

/// One grid row: each item in an equal share of the width. A short last
/// row leaves the shares after it empty, so its columns line up.
fn grid_row<T: Clone, K, V: IntoView>(
    grid: Grid,
    columns: usize,
    items: &[(K, T)],
    row_fn: &impl Fn(T) -> V,
) -> AnyView {
    let share = || {
        Stack::column(0.0).with_layout(|layout| {
            layout.width = Some(LengthSpec::Px(0.0));
            layout.min_width = Some(LengthSpec::Px(0.0));
            layout.flex_basis = Some(LengthSpec::Px(0.0));
            layout.flex_grow = Some(1.0);
            layout.flex_shrink = Some(1.0);
        })
    };
    let mut cells = items
        .iter()
        .map(|(_, item)| {
            widget(share())
                .children(row_fn(item.clone()).into_any())
                .into_any()
        })
        .collect::<Vec<_>>();
    cells.extend((items.len()..columns).map(|_| widget(share()).into_any()));
    widget(Stack::fill_row(grid.gap).align(AlignSpec::Start))
        .children(cells)
        .into_any()
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
        let moved = super::signal(0u64);
        let bump = move || moved.update(|at| *at += 1);
        // The list takes the width it is given, not its rows': rows wrap
        // their text to it and grid columns divide it.
        let mut list = List::new();
        std::sync::Arc::make_mut(&mut list.style.layout).width = Some(LengthSpec::Percent(100.0));
        let (scroll, list, within) = match self.container {
            Container::Own(scroll) => {
                let scroll = vb.place(self.key, *scroll);
                if scroll.stable_id() == UNBUILT {
                    return;
                }
                let list = vb.ui.nest(scroll, |ui| ui.child("list", list));
                vb.ui.on(scroll, move |_, _: &ScrollChanged, _| bump());
                vb.ui
                    .on(scroll, move |_, _: &ScrollViewportChanged, _| bump());
                if self.measured || self.grid.is_some() {
                    vb.ui.on(scroll, move |_, _: &ScrollLaidOut, _| bump());
                }
                (Some(scroll), list, None)
            }
            Container::Within(within) => (None, vb.place(self.key, list), Some(within)),
        };
        let id = list.stable_id();
        if id == UNBUILT {
            return;
        }
        let effect =
            reactive::create_effect(vb.st.tag, EffectTarget::Structural(id), None, self.site);
        let binding = EachVirtualBinding {
            items: self.items,
            key_fn: self.key_fn,
            row_fn: self.row_fn,
            scope: reactive::current_scope(),
            moved,
            within,
            scroll,
            list,
            grid: self.grid,
            columns: 1,
            state: if self.measured {
                VirtualListItems::measured()
            } else {
                VirtualListItems::default()
            },
            layout: VirtualListLayout::new([]),
            row_height: self.row_height,
            measured: self.measured,
            overscan: self.overscan,
            units: Vec::new(),
            members: Vec::new(),
            index: HashMap::new(),
            version: 0,
            _types: PhantomData,
        };
        // Subscribe now; the first window follows the first layout, whose
        // viewport event moves it. An ancestor's events are only heard once
        // a sync has found it, so ask for that first sync.
        let within = binding.within.is_some();
        reactive::run_tracked(effect, || binding.read());
        vb.st.parts.structural.push((id, effect, Box::new(binding)));
        if within {
            bump();
        }
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
