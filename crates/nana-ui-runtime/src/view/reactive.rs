//! Fine-grained signals behind the declarative view layer.
//!
//! One runtime per thread. Handles are `Copy` ids, so they cross into the
//! `Send` event handlers the retained tree requires; the values stay on the
//! thread that created them, and using a handle on another thread panics.
//!
//! Writes never touch the tree. A write marks its subscribers: effects are
//! queued for the owning [`AppContext`](crate::AppContext)'s next flush,
//! computeds only turn dirty and recompute when next read.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::StableNodeId;

static NEXT_RUNTIME: AtomicU32 = AtomicU32::new(1);
static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
}

fn with_rt<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RUNTIME.with(|rt| f(&mut rt.borrow_mut()))
}

/// Identity of an [`AppContext`](crate::AppContext) for effect routing. `0`
/// runs on any context's flush.
pub(crate) fn next_context_tag() -> u64 {
    NEXT_CONTEXT.fetch_add(1, Ordering::Relaxed)
}

macro_rules! key_type {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub(crate) struct $name {
            runtime: u32,
            index: u32,
            generation: u32,
        }
    };
}
key_type!(SignalKey);
key_type!(EffectKey);
key_type!(ScopeKey);

struct Arena<T> {
    slots: Vec<(u32, Option<T>)>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Arena<T> {
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    fn insert(&mut self, value: T) -> (u32, u32) {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.1 = Some(value);
            return (index, slot.0);
        }
        self.slots.push((0, Some(value)));
        ((self.slots.len() - 1) as u32, 0)
    }

    fn get(&self, index: u32, generation: u32) -> Option<&T> {
        self.slots
            .get(index as usize)
            .filter(|slot| slot.0 == generation)
            .and_then(|slot| slot.1.as_ref())
    }

    fn get_mut(&mut self, index: u32, generation: u32) -> Option<&mut T> {
        self.slots
            .get_mut(index as usize)
            .filter(|slot| slot.0 == generation)
            .and_then(|slot| slot.1.as_mut())
    }

    fn remove(&mut self, index: u32, generation: u32) -> Option<T> {
        let slot = self.slots.get_mut(index as usize)?;
        if slot.0 != generation {
            return None;
        }
        let value = slot.1.take()?;
        slot.0 = slot.0.wrapping_add(1);
        self.free.push(index);
        self.live -= 1;
        Some(value)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Observer {
    Effect(EffectKey),
    Computed(SignalKey),
}

type ComputeFn = Box<dyn FnMut(&dyn Any)>;

struct ComputedNode {
    dirty: bool,
    deps: Vec<SignalKey>,
    compute: Option<ComputeFn>,
}

struct SignalNode {
    /// `RefCell<T>`, for a signal and a computed alike.
    value: Rc<dyn Any>,
    subscribers: Vec<Observer>,
    computed: Option<ComputedNode>,
    created: &'static Location<'static>,
}

/// What a queued effect does when its context flushes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EffectTarget {
    /// Re-apply the bindings of one node.
    Node(StableNodeId),
    /// Re-run a keyed list or conditional block rooted at a container.
    Structural(StableNodeId),
    /// A [`watch_effect`].
    User,
}

struct EffectNode {
    deps: Vec<SignalKey>,
    queued: bool,
    context: u64,
    target: EffectTarget,
    run: Option<Box<dyn FnMut()>>,
    site: &'static Location<'static>,
}

struct ScopeNode {
    parent: Option<ScopeKey>,
    signals: Vec<SignalKey>,
    effects: Vec<EffectKey>,
    children: Vec<ScopeKey>,
    cleanups: Vec<Box<dyn FnOnce()>>,
    contexts: Vec<(TypeId, Rc<dyn Any>)>,
}

struct Frame {
    observer: Option<Observer>,
    reads: Vec<SignalKey>,
}

/// Counters for tests, benchmarks and devtools. Live counts are exact; the
/// rest accumulate for the thread's lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReactiveStats {
    pub signals: usize,
    pub effects: usize,
    pub scopes: usize,
    pub signal_writes: u64,
    pub effects_run: u64,
    pub flushes: u64,
    pub nodes_patched: u64,
    pub commits: u64,
}

struct Runtime {
    id: u32,
    signals: Arena<SignalNode>,
    effects: Arena<EffectNode>,
    scopes: Arena<ScopeNode>,
    tracking: Vec<Frame>,
    pool: Vec<Vec<SignalKey>>,
    pending: Vec<EffectKey>,
    notify_stack: Vec<Observer>,
    current_scope: Option<ScopeKey>,
    stats: ReactiveStats,
    #[cfg(feature = "reactive-trace")]
    trace: super::trace::Ring,
}

/// Whatever a disposal releases, dropped only after the runtime borrow ends:
/// a captured value's `Drop` may touch signals again.
#[derive(Default)]
struct Released {
    cleanups: Vec<Box<dyn FnOnce()>>,
    garbage: Vec<Box<dyn Any>>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            id: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            signals: Arena::new(),
            effects: Arena::new(),
            scopes: Arena::new(),
            tracking: Vec::new(),
            pool: Vec::new(),
            pending: Vec::new(),
            notify_stack: Vec::new(),
            current_scope: None,
            stats: ReactiveStats::default(),
            #[cfg(feature = "reactive-trace")]
            trace: super::trace::Ring::new(),
        }
    }

    fn check_thread(&self, runtime: u32) {
        if runtime != self.id {
            panic!("a signal handle was used on a thread other than the one that created it");
        }
    }

    fn signal(&self, key: SignalKey) -> Option<&SignalNode> {
        self.check_thread(key.runtime);
        self.signals.get(key.index, key.generation)
    }

    fn signal_mut(&mut self, key: SignalKey) -> Option<&mut SignalNode> {
        self.check_thread(key.runtime);
        self.signals.get_mut(key.index, key.generation)
    }

    fn effect_mut(&mut self, key: EffectKey) -> Option<&mut EffectNode> {
        self.check_thread(key.runtime);
        self.effects.get_mut(key.index, key.generation)
    }

    fn scope_mut(&mut self, key: ScopeKey) -> Option<&mut ScopeNode> {
        self.check_thread(key.runtime);
        self.scopes.get_mut(key.index, key.generation)
    }

    fn new_signal(
        &mut self,
        value: Rc<dyn Any>,
        computed: Option<ComputedNode>,
        created: &'static Location<'static>,
    ) -> SignalKey {
        let (index, generation) = self.signals.insert(SignalNode {
            value,
            subscribers: Vec::new(),
            computed,
            created,
        });
        let key = SignalKey {
            runtime: self.id,
            index,
            generation,
        };
        if let Some(scope) = self.current_scope
            && let Some(node) = self.scope_mut(scope)
        {
            node.signals.push(key);
        }
        key
    }

    fn track(&mut self, key: SignalKey) {
        if let Some(frame) = self.tracking.last_mut()
            && frame.observer.is_some()
            && !frame.reads.contains(&key)
        {
            frame.reads.push(key);
        }
    }

    fn begin(&mut self, observer: Option<Observer>) {
        let reads = self.pool.pop().unwrap_or_default();
        self.tracking.push(Frame { observer, reads });
    }

    fn recycle(&mut self, mut reads: Vec<SignalKey>) {
        reads.clear();
        self.pool.push(reads);
    }

    fn deps_mut(&mut self, observer: Observer) -> Option<&mut Vec<SignalKey>> {
        match observer {
            Observer::Effect(key) => self.effect_mut(key).map(|node| &mut node.deps),
            Observer::Computed(key) => self
                .signal_mut(key)
                .and_then(|node| node.computed.as_mut())
                .map(|computed| &mut computed.deps),
        }
    }

    fn unsubscribe(&mut self, dep: SignalKey, observer: Observer) {
        if let Some(node) = self.signal_mut(dep) {
            node.subscribers
                .retain(|subscriber| *subscriber != observer);
        }
    }

    /// Close the innermost frame and make its reads the observer's deps.
    /// Unchanged deps (the common case) cost no allocation and no
    /// resubscription.
    fn end(&mut self) {
        let Some(Frame { observer, reads }) = self.tracking.pop() else {
            return;
        };
        let Some(old) = observer.and_then(|observer| self.deps_mut(observer)) else {
            return self.recycle(reads);
        };
        if old.len() == reads.len() && old.iter().all(|dep| reads.contains(dep)) {
            return self.recycle(reads);
        }
        let observer = observer.expect("deps belong to an observer");
        let old = std::mem::take(old);
        for dep in &old {
            if !reads.contains(dep) {
                self.unsubscribe(*dep, observer);
            }
        }
        for dep in &reads {
            if !old.contains(dep)
                && let Some(node) = self.signal_mut(*dep)
            {
                node.subscribers.push(observer);
            }
        }
        *self.deps_mut(observer).expect("observer still exists") = reads;
        self.recycle(old);
    }

    fn notify(&mut self, key: SignalKey, _at: &'static Location<'static>) {
        self.stats.signal_writes += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::runtime::REACTIVE_SIGNAL_WRITES);
        #[cfg(feature = "reactive-trace")]
        {
            let created = self.signal(key).map(|node| node.created);
            self.trace.write(key, _at, created);
        }
        let mut stack = std::mem::take(&mut self.notify_stack);
        if let Some(node) = self.signal(key) {
            stack.extend(node.subscribers.iter().copied());
        }
        while let Some(observer) = stack.pop() {
            match observer {
                Observer::Effect(effect) => {
                    if let Some(node) = self.effect_mut(effect)
                        && !node.queued
                    {
                        node.queued = true;
                        self.pending.push(effect);
                    }
                    #[cfg(feature = "reactive-trace")]
                    self.trace.notify(key, effect);
                }
                Observer::Computed(computed) => {
                    if let Some(node) = self.signal_mut(computed)
                        && let Some(state) = node.computed.as_mut()
                        && !state.dirty
                    {
                        state.dirty = true;
                        stack.extend(node.subscribers.iter().copied());
                    }
                }
            }
        }
        self.notify_stack = stack;
    }

    fn remove_effect(&mut self, key: EffectKey, released: &mut Released) {
        let Some(node) = self.effects.remove(key.index, key.generation) else {
            return;
        };
        for dep in &node.deps {
            self.unsubscribe(*dep, Observer::Effect(key));
        }
        if let Some(run) = node.run {
            released.garbage.push(Box::new(run));
        }
        self.recycle(node.deps);
    }

    fn remove_signal(&mut self, key: SignalKey, released: &mut Released) {
        let Some(node) = self.signals.remove(key.index, key.generation) else {
            return;
        };
        if let Some(computed) = node.computed {
            for dep in &computed.deps {
                self.unsubscribe(*dep, Observer::Computed(key));
            }
            if let Some(compute) = computed.compute {
                released.garbage.push(Box::new(compute));
            }
        }
        released.garbage.push(Box::new(node.value));
    }

    fn dispose_scope(&mut self, key: ScopeKey, released: &mut Released) {
        let Some(node) = self.scopes.remove(key.index, key.generation) else {
            return;
        };
        if let Some(parent) = node.parent
            && let Some(parent) = self.scope_mut(parent)
        {
            parent.children.retain(|child| *child != key);
        }
        for child in node.children {
            self.dispose_scope(child, released);
        }
        for effect in node.effects {
            self.remove_effect(effect, released);
        }
        for signal in node.signals {
            self.remove_signal(signal, released);
        }
        released.cleanups.extend(node.cleanups.into_iter().rev());
        released.garbage.extend(
            node.contexts
                .into_iter()
                .map(|(_, value)| Box::new(value) as Box<dyn Any>),
        );
    }
}

impl Released {
    fn run(self) {
        for cleanup in self.cleanups {
            cleanup();
        }
        drop(self.garbage);
    }
}

#[cold]
#[inline(never)]
fn disposed(at: &'static Location<'static>) -> ! {
    nana_diagnostics::fault!(
        nana_diagnostics::framework::runtime::REACTIVE_DISPOSED_ACCESS;
        "signal used at {at} after its scope was disposed"
    );
    panic!("signal used at {at} after its scope was disposed");
}

enum ReadState {
    Ready(Rc<dyn Any>),
    Recompute,
}

/// The value cell of `key`, recomputing a dirty computed first. The read is
/// recorded for the innermost tracking frame unless `track` is false.
pub(crate) fn read_cell(
    key: SignalKey,
    at: &'static Location<'static>,
    track: bool,
) -> Rc<dyn Any> {
    loop {
        let state = with_rt(|rt| {
            rt.signal(key).map(|node| match &node.computed {
                Some(computed) if computed.dirty => ReadState::Recompute,
                _ => ReadState::Ready(Rc::clone(&node.value)),
            })
        });
        match state {
            None => disposed(at),
            Some(ReadState::Ready(cell)) => {
                if track {
                    with_rt(|rt| rt.track(key));
                }
                return cell;
            }
            Some(ReadState::Recompute) => recompute(key, at),
        }
    }
}

fn recompute(key: SignalKey, at: &'static Location<'static>) {
    let taken = with_rt(|rt| {
        let node = rt.signal_mut(key)?;
        let cell = Rc::clone(&node.value);
        let compute = node.computed.as_mut()?.compute.take();
        Some((cell, compute))
    });
    let Some((cell, compute)) = taken else {
        disposed(at);
    };
    let Some(mut compute) = compute else {
        panic!("computed read at {at} depends on itself");
    };
    with_rt(|rt| rt.begin(Some(Observer::Computed(key))));
    compute(&*cell);
    with_rt(|rt| {
        rt.end();
        if let Some(node) = rt.signal_mut(key)
            && let Some(computed) = node.computed.as_mut()
        {
            computed.dirty = false;
            computed.compute = Some(compute);
        }
    });
}

/// Where the signal behind `key` was created, while it exists.
pub(crate) fn created_at(key: SignalKey) -> Option<&'static Location<'static>> {
    with_rt(|rt| rt.signal(key).map(|node| node.created))
}

fn write_cell(key: SignalKey, at: &'static Location<'static>) -> Rc<dyn Any> {
    with_rt(|rt| rt.signal(key).map(|node| Rc::clone(&node.value))).unwrap_or_else(|| disposed(at))
}

/// Create a signal owned by the current scope.
#[track_caller]
pub fn signal<T: 'static>(value: T) -> Signal<T> {
    let created = Location::caller();
    let key = with_rt(|rt| rt.new_signal(Rc::new(RefCell::new(value)), None, created));
    Signal {
        key,
        _type: PhantomData,
    }
}

/// Create a derived value owned by the current scope. It is computed now,
/// turns dirty when anything it read changes, and recomputes on the next
/// read.
#[track_caller]
pub fn computed<T: 'static>(f: impl Fn() -> T + 'static) -> Computed<T> {
    let created = Location::caller();
    let key = with_rt(|rt| {
        rt.new_signal(
            Rc::new(()),
            Some(ComputedNode {
                dirty: true,
                deps: Vec::new(),
                compute: None,
            }),
            created,
        )
    });
    // The first run fills the cell; later runs replace its value in place.
    with_rt(|rt| rt.begin(Some(Observer::Computed(key))));
    let value = f();
    with_rt(|rt| {
        rt.end();
        if let Some(node) = rt.signal_mut(key) {
            node.value = Rc::new(RefCell::new(value));
            let computed = node.computed.as_mut().expect("created as a computed");
            computed.dirty = false;
            computed.compute = Some(Box::new(move |cell: &dyn Any| {
                *cell
                    .downcast_ref::<RefCell<T>>()
                    .expect("computed cell holds its own type")
                    .borrow_mut() = f();
            }));
        }
    });
    Computed {
        key,
        _type: PhantomData,
    }
}

/// Run `f` now and again whenever a signal it read changes, on the next
/// flush of any context. Owned by the current scope.
#[track_caller]
pub fn watch_effect(f: impl FnMut() + 'static) -> Effect {
    let key = create_effect(0, EffectTarget::User, Some(Box::new(f)), Location::caller());
    run_user_effect(key);
    Effect { key }
}

/// Run `f` when the current scope is disposed.
pub fn on_cleanup(f: impl FnOnce() + 'static) {
    with_rt(|rt| {
        if let Some(scope) = rt.current_scope
            && let Some(node) = rt.scope_mut(scope)
        {
            node.cleanups.push(Box::new(f));
        }
    });
}

/// Make `value` available to [`use_context`] in the current scope and every
/// scope below it (Vue `provide`). A second `provide` of the same type in the
/// same scope replaces the first. Outside any scope it does nothing.
pub fn provide<T: 'static>(value: T) {
    with_rt(|rt| {
        let Some(scope) = rt.current_scope else {
            return;
        };
        let Some(node) = rt.scope_mut(scope) else {
            return;
        };
        let value: Rc<dyn Any> = Rc::new(value);
        match node
            .contexts
            .iter_mut()
            .find(|(type_id, _)| *type_id == TypeId::of::<T>())
        {
            Some(slot) => slot.1 = value,
            None => node.contexts.push((TypeId::of::<T>(), value)),
        }
    });
}

/// The nearest value of type `T` provided by the current scope or an
/// ancestor (Vue `inject`). Read it while the view is built; event handlers
/// run outside any scope and see nothing.
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    with_rt(|rt| {
        let mut cursor = rt.current_scope;
        while let Some(key) = cursor {
            let node = rt.scopes.get(key.index, key.generation)?;
            if let Some((_, value)) = node
                .contexts
                .iter()
                .find(|(type_id, _)| *type_id == TypeId::of::<T>())
            {
                return value.downcast_ref::<T>().cloned();
            }
            cursor = node.parent;
        }
        None
    })
}

/// Run `f` without recording what it reads.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    with_rt(|rt| rt.begin(None));
    let result = f();
    with_rt(|rt| rt.end());
    result
}

/// Counters for the current thread's runtime.
pub fn reactive_stats() -> ReactiveStats {
    with_rt(|rt| ReactiveStats {
        signals: rt.signals.live,
        effects: rt.effects.live,
        scopes: rt.scopes.live,
        ..rt.stats
    })
}

pub(crate) fn record_flush(nodes_patched: u64, commits: u64) {
    with_rt(|rt| {
        rt.stats.flushes += 1;
        rt.stats.nodes_patched += nodes_patched;
        rt.stats.commits += commits;
    });
}

pub(crate) fn create_effect(
    context: u64,
    target: EffectTarget,
    run: Option<Box<dyn FnMut()>>,
    site: &'static Location<'static>,
) -> EffectKey {
    with_rt(|rt| {
        let deps = rt.pool.pop().unwrap_or_default();
        let (index, generation) = rt.effects.insert(EffectNode {
            deps,
            queued: false,
            context,
            target,
            run,
            site,
        });
        let key = EffectKey {
            runtime: rt.id,
            index,
            generation,
        };
        if let Some(scope) = rt.current_scope
            && let Some(node) = rt.scope_mut(scope)
        {
            node.effects.push(key);
        }
        key
    })
}

pub(crate) fn set_effect_target(key: EffectKey, target: EffectTarget) {
    with_rt(|rt| {
        if let Some(node) = rt.effect_mut(key) {
            node.target = target;
        }
    });
}

pub(crate) fn effect_site(key: EffectKey) -> Option<&'static Location<'static>> {
    with_rt(|rt| rt.effect_mut(key).map(|node| node.site))
}

pub(crate) fn dispose_effect(key: EffectKey) {
    let mut released = Released::default();
    with_rt(|rt| rt.remove_effect(key, &mut released));
    released.run();
}

/// Run `f` as `effect`, making what it reads the effect's dependencies.
pub(crate) fn run_tracked<R>(effect: EffectKey, f: impl FnOnce() -> R) -> R {
    with_rt(|rt| {
        rt.begin(Some(Observer::Effect(effect)));
        rt.stats.effects_run += 1;
    });
    nana_diagnostics::metric!(nana_diagnostics::framework::runtime::REACTIVE_EFFECTS_RUN);
    let result = f();
    with_rt(|rt| rt.end());
    result
}

pub(crate) fn run_user_effect(key: EffectKey) {
    let Some(mut run) = with_rt(|rt| rt.effect_mut(key).and_then(|node| node.run.take())) else {
        return;
    };
    run_tracked(key, &mut run);
    let dropped = with_rt(|rt| match rt.effect_mut(key) {
        Some(node) => {
            node.run = Some(run);
            None
        }
        None => Some(run),
    });
    drop(dropped);
}

pub(crate) fn has_pending(context: u64) -> bool {
    with_rt(|rt| {
        rt.pending.iter().any(|key| {
            rt.effects
                .get(key.index, key.generation)
                .is_some_and(|node| node.context == context || node.context == 0)
        })
    })
}

/// Move the queued effects `context` runs into `out`, in queue order.
pub(crate) fn take_pending(context: u64, out: &mut Vec<(EffectKey, EffectTarget)>) {
    with_rt(|rt| {
        let Runtime {
            pending, effects, ..
        } = rt;
        pending.retain(|key| {
            let Some(node) = effects.get_mut(key.index, key.generation) else {
                return false;
            };
            if node.context != context && node.context != 0 {
                return true;
            }
            node.queued = false;
            out.push((*key, node.target));
            false
        });
    });
}

/// Drop what `context` has queued, after a flush that did not settle.
pub(crate) fn drop_pending(context: u64) {
    let mut dropped = Vec::new();
    take_pending(context, &mut dropped);
}

pub(crate) fn create_scope(parent: Option<ScopeKey>) -> ScopeKey {
    with_rt(|rt| {
        let (index, generation) = rt.scopes.insert(ScopeNode {
            parent,
            signals: Vec::new(),
            effects: Vec::new(),
            children: Vec::new(),
            cleanups: Vec::new(),
            contexts: Vec::new(),
        });
        let key = ScopeKey {
            runtime: rt.id,
            index,
            generation,
        };
        if let Some(parent) = parent
            && let Some(node) = rt.scope_mut(parent)
        {
            node.children.push(key);
        }
        key
    })
}

pub(crate) fn current_scope() -> Option<ScopeKey> {
    with_rt(|rt| rt.current_scope)
}

pub(crate) fn with_scope<R>(scope: ScopeKey, f: impl FnOnce() -> R) -> R {
    let previous = with_rt(|rt| rt.current_scope.replace(scope));
    let result = f();
    with_rt(|rt| rt.current_scope = previous);
    result
}

pub(crate) fn dispose_scope(key: ScopeKey) {
    let mut released = Released::default();
    with_rt(|rt| rt.dispose_scope(key, &mut released));
    released.run();
}

/// Release what a dropped context owned, when it is dropped on the thread
/// that created it; elsewhere the state is unreachable and left alone
/// rather than panicking inside a destructor.
pub(crate) fn release_on_drop(effects: Vec<EffectKey>, scopes: Vec<ScopeKey>) {
    let local = RUNTIME
        .try_with(|rt| {
            let Ok(rt) = rt.try_borrow() else {
                return false;
            };
            effects.iter().all(|key| key.runtime == rt.id)
                && scopes.iter().all(|key| key.runtime == rt.id)
        })
        .unwrap_or(false);
    if !local {
        return;
    }
    for effect in effects {
        dispose_effect(effect);
    }
    for scope in scopes {
        dispose_scope(scope);
    }
}

#[cfg(feature = "reactive-trace")]
pub(crate) fn with_trace<R>(f: impl FnOnce(&mut super::trace::Ring) -> R) -> R {
    with_rt(|rt| f(&mut rt.trace))
}

/// Something a binding can read under tracking.
pub trait Readable<T: 'static>: Copy + Send + 'static {
    #[track_caller]
    fn with_value<R>(&self, f: impl FnOnce(&T) -> R) -> R;
}

/// A writable reactive value. `Copy`; the value lives in the creating
/// thread's runtime until the owning scope is disposed.
pub struct Signal<T: 'static> {
    pub(crate) key: SignalKey,
    _type: PhantomData<fn() -> T>,
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Signal<T> {}

impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<T: 'static> Signal<T> {
    /// Read and track.
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let cell = read_cell(self.key, Location::caller(), true);
        let cell = cell
            .downcast_ref::<RefCell<T>>()
            .expect("signal cell holds its own type");
        f(&cell.borrow())
    }

    #[track_caller]
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        self.with_untracked(T::clone)
    }

    #[track_caller]
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let cell = read_cell(self.key, Location::caller(), false);
        let cell = cell
            .downcast_ref::<RefCell<T>>()
            .expect("signal cell holds its own type");
        f(&cell.borrow())
    }

    /// Replace the value and notify, even when it is equal.
    #[track_caller]
    pub fn set(&self, value: T) {
        self.update(|slot| *slot = value);
    }

    /// Mutate in place and notify. `f` must not read this signal.
    #[track_caller]
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let at = Location::caller();
        let cell = write_cell(self.key, at);
        {
            let cell = cell
                .downcast_ref::<RefCell<T>>()
                .expect("signal cell holds its own type");
            f(&mut cell.borrow_mut());
        }
        with_rt(|rt| rt.notify(self.key, at));
    }

    /// Where this signal was created. `None` once its scope is disposed.
    pub fn defined_at(&self) -> Option<&'static Location<'static>> {
        created_at(self.key)
    }

    /// A closure reading this signal through `f`, for derived props:
    /// `text(count.map(|c| format!("{c} items")))`.
    pub fn map<U>(self, f: impl Fn(&T) -> U + Send + 'static) -> impl Fn() -> U + Send + 'static {
        move || self.with(&f)
    }
}

impl<T: 'static> Readable<T> for Signal<T> {
    #[track_caller]
    fn with_value<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.with(f)
    }
}

impl<T: fmt::Display + 'static> fmt::Display for Signal<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with(|value| value.fmt(formatter))
    }
}

impl<T> fmt::Debug for Signal<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Signal").field(&self.key).finish()
    }
}

/// A derived value, recomputed lazily. See [`computed`].
pub struct Computed<T: 'static> {
    pub(crate) key: SignalKey,
    _type: PhantomData<fn() -> T>,
}

impl<T> Clone for Computed<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Computed<T> {}

impl<T: 'static> Computed<T> {
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let cell = read_cell(self.key, Location::caller(), true);
        let cell = cell
            .downcast_ref::<RefCell<T>>()
            .expect("computed cell holds its own type");
        f(&cell.borrow())
    }
}

impl<T: 'static> Computed<T> {
    /// Where this computed was created. `None` once its scope is disposed.
    pub fn defined_at(&self) -> Option<&'static Location<'static>> {
        created_at(self.key)
    }
}

impl<T: 'static> Readable<T> for Computed<T> {
    #[track_caller]
    fn with_value<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.with(f)
    }
}

impl<T: fmt::Display + 'static> fmt::Display for Computed<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with(|value| value.fmt(formatter))
    }
}

impl<T> fmt::Debug for Computed<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Computed").field(&self.key).finish()
    }
}

/// Handle to a [`watch_effect`].
#[derive(Debug, Clone, Copy)]
pub struct Effect {
    key: EffectKey,
}

impl Effect {
    /// Stop re-running. Disposing its scope does the same.
    pub fn dispose(self) {
        dispose_effect(self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scoped<R>(f: impl FnOnce() -> R) -> (ScopeKey, R) {
        let scope = create_scope(None);
        let result = with_scope(scope, f);
        (scope, result)
    }

    fn flush_user_effects() {
        let mut queue = Vec::new();
        take_pending(0, &mut queue);
        for (key, target) in queue {
            assert_eq!(target, EffectTarget::User);
            run_user_effect(key);
        }
    }

    #[test]
    fn a_watch_effect_reruns_once_per_flush_after_writes() {
        let runs = Rc::new(Cell::new(0));
        let (scope, count) = scoped(|| {
            let count = signal(1);
            let seen = Rc::clone(&runs);
            watch_effect(move || {
                count.get();
                seen.set(seen.get() + 1);
            });
            count
        });
        assert_eq!(runs.get(), 1);
        count.set(2);
        count.set(3);
        flush_user_effects();
        assert_eq!(runs.get(), 2);
        flush_user_effects();
        assert_eq!(runs.get(), 2);
        dispose_scope(scope);
    }

    #[test]
    fn a_computed_recomputes_once_per_change_and_only_when_read() {
        let computes = Rc::new(Cell::new(0));
        let (scope, (count, doubled)) = scoped(|| {
            let count = signal(2);
            let seen = Rc::clone(&computes);
            let doubled = computed(move || {
                seen.set(seen.get() + 1);
                count.get() * 2
            });
            (count, doubled)
        });
        assert_eq!(computes.get(), 1, "computed when created");
        assert_eq!(doubled.get(), 4);
        assert_eq!(doubled.get(), 4);
        assert_eq!(computes.get(), 1);
        count.set(5);
        assert_eq!(computes.get(), 1);
        assert_eq!(doubled.get(), 10);
        assert_eq!(computes.get(), 2);
        dispose_scope(scope);
    }

    #[test]
    fn dependencies_follow_the_branch_actually_read() {
        let runs = Rc::new(Cell::new(0));
        let (scope, (flag, left, right)) = scoped(|| {
            let flag = signal(true);
            let left = signal(0);
            let right = signal(0);
            let seen = Rc::clone(&runs);
            watch_effect(move || {
                if flag.get() {
                    left.get();
                } else {
                    right.get();
                }
                seen.set(seen.get() + 1);
            });
            (flag, left, right)
        });
        right.set(1);
        flush_user_effects();
        assert_eq!(runs.get(), 1, "right is not read while flag is true");
        flag.set(false);
        flush_user_effects();
        assert_eq!(runs.get(), 2);
        left.set(1);
        flush_user_effects();
        assert_eq!(runs.get(), 2, "left is no longer read");
        right.set(2);
        flush_user_effects();
        assert_eq!(runs.get(), 3);
        dispose_scope(scope);
    }

    #[test]
    fn disposing_a_scope_frees_its_signals_effects_and_runs_cleanups() {
        let before = reactive_stats();
        let cleaned = Rc::new(Cell::new(false));
        let (scope, _) = scoped(|| {
            let count = signal(0);
            watch_effect(move || {
                count.get();
            });
            let child = create_scope(current_scope());
            with_scope(child, || {
                signal("nested");
            });
            let flag = Rc::clone(&cleaned);
            on_cleanup(move || flag.set(true));
        });
        let during = reactive_stats();
        assert_eq!(during.signals, before.signals + 2);
        assert_eq!(during.effects, before.effects + 1);
        assert_eq!(during.scopes, before.scopes + 2);
        dispose_scope(scope);
        let after = reactive_stats();
        assert!(cleaned.get());
        assert_eq!(
            (after.signals, after.effects, after.scopes),
            (before.signals, before.effects, before.scopes)
        );
    }

    #[test]
    #[should_panic(expected = "after its scope was disposed")]
    fn reading_a_disposed_signal_panics() {
        let (scope, count) = scoped(|| signal(0));
        dispose_scope(scope);
        count.get();
    }

    #[test]
    fn signals_format_through_display_and_track_the_read() {
        let runs = Rc::new(Cell::new(0));
        let (scope, count) = scoped(|| {
            let count = signal(7u32);
            let seen = Rc::clone(&runs);
            watch_effect(move || {
                assert!(!format!("n={count}").is_empty());
                seen.set(seen.get() + 1);
            });
            count
        });
        assert_eq!(format!("{count}"), "7");
        count.set(8);
        flush_user_effects();
        assert_eq!(runs.get(), 2);
        dispose_scope(scope);
    }
}
