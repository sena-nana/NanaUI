use std::any::TypeId;
use std::borrow::{Borrow, Cow};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::ops::Deref;
use std::sync::{Arc, LazyLock, Mutex};

use crate::world::child_order::{common_ends, longest_increasing, positions_in};
use crate::{ComponentView, DocumentId, Entity, MutationQueue, StableNodeId, View};

use super::{AppContext, FrameworkError};

#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum InlineLen {
    L0 = 0,
    L1 = 1,
    L2 = 2,
    L3 = 3,
    L4 = 4,
    L5 = 5,
    L6 = 6,
    L7 = 7,
    L8 = 8,
    L9 = 9,
    L10 = 10,
    L11 = 11,
    L12 = 12,
    L13 = 13,
    L14 = 14,
}

impl InlineLen {
    #[inline]
    fn from_usize(len: usize) -> Option<Self> {
        match len {
            0 => Some(Self::L0),
            1 => Some(Self::L1),
            2 => Some(Self::L2),
            3 => Some(Self::L3),
            4 => Some(Self::L4),
            5 => Some(Self::L5),
            6 => Some(Self::L6),
            7 => Some(Self::L7),
            8 => Some(Self::L8),
            9 => Some(Self::L9),
            10 => Some(Self::L10),
            11 => Some(Self::L11),
            12 => Some(Self::L12),
            13 => Some(Self::L13),
            14 => Some(Self::L14),
            _ => None,
        }
    }
}

/// Compact assembly key: stores UTF-8 strings up to 14 bytes inline in 16 B total.
#[derive(Clone)]
pub(crate) enum AssemblyKey {
    Inline { len: InlineLen, buf: [u8; 14] },
    Shared(Arc<String>),
}

const _: () = assert!(std::mem::size_of::<AssemblyKey>() == 16);

impl AssemblyKey {
    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Inline { len, buf } => {
                let n = (*len as usize).min(14);
                std::str::from_utf8(&buf[..n]).unwrap_or("")
            }
            Self::Shared(text) => text.as_str(),
        }
    }

    pub(crate) fn auto_kind(kind: &'static str, index: usize) -> Self {
        let mut buf = [0u8; 14];
        let mut cursor = std::io::Cursor::new(&mut buf[..]);
        if write!(cursor, "#{kind}-{index}").is_ok() {
            let written = cursor.position() as usize;
            if let Some(len) = InlineLen::from_usize(written) {
                return Self::Inline { len, buf };
            }
        }
        Self::Shared(Arc::new(format!("#{kind}-{index}")))
    }

    pub(crate) fn auto_view(index: usize) -> Self {
        let mut buf = [0u8; 14];
        let mut cursor = std::io::Cursor::new(&mut buf[..]);
        if write!(cursor, "#v{index}").is_ok() {
            let written = cursor.position() as usize;
            if let Some(len) = InlineLen::from_usize(written) {
                return Self::Inline { len, buf };
            }
        }
        Self::Shared(Arc::new(format!("#v{index}")))
    }

    /// Parses canonical positional keys (`"#v0"` or `"#v"` followed by a nonzero decimal
    /// `< u16::MAX`) that round-trip byte-for-byte via [`Self::auto_view`].
    #[inline]
    pub(crate) fn parse_auto_view(key: &str) -> Option<u32> {
        let rest = key.strip_prefix("#v")?;
        let bytes = rest.as_bytes();
        if bytes.is_empty() {
            return None;
        }
        if bytes.len() > 1 && bytes[0] == b'0' {
            return None;
        }
        let idx = rest.parse::<u32>().ok()?;
        (idx < u32::from(u16::MAX)).then_some(idx)
    }
}

static AUTO_VIEW_STRS: LazyLock<Mutex<hashbrown::HashMap<u16, &'static str>>> =
    LazyLock::new(|| Mutex::new(hashbrown::HashMap::new()));

/// Cached `&'static str` for `#v{slot}` when `AppContext::assembly_key` inspects a positional slot.
pub(crate) fn auto_view_key_str(slot: u16) -> &'static str {
    let mut guard = AUTO_VIEW_STRS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    guard
        .entry(slot)
        .or_insert_with(|| Box::leak(format!("#v{slot}").into_boxed_str()))
}

impl Deref for AssemblyKey {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for AssemblyKey {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for AssemblyKey {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for AssemblyKey {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for AssemblyKey {}

impl Hash for AssemblyKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for AssemblyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for AssemblyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AssemblyKey {
    fn from(value: &str) -> Self {
        if let Some(len) = InlineLen::from_usize(value.len()) {
            let mut buf = [0u8; 14];
            buf[..value.len()].copy_from_slice(value.as_bytes());
            Self::Inline { len, buf }
        } else {
            Self::Shared(Arc::new(value.to_owned()))
        }
    }
}

impl From<String> for AssemblyKey {
    fn from(value: String) -> Self {
        if let Some(len) = InlineLen::from_usize(value.len()) {
            let mut buf = [0u8; 14];
            buf[..value.len()].copy_from_slice(value.as_bytes());
            Self::Inline { len, buf }
        } else {
            Self::Shared(Arc::new(value))
        }
    }
}

impl From<&String> for AssemblyKey {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<Cow<'_, str>> for AssemblyKey {
    fn from(value: Cow<'_, str>) -> Self {
        match value {
            Cow::Borrowed(s) => Self::from(s),
            Cow::Owned(s) => Self::from(s),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct AssembledChild {
    pub id: StableNodeId,
}

const _: () = assert!(std::mem::size_of::<Option<AssembledChild>>() == 8);

/// Keyed children under one parent: positional `#v{n}` slots avoid hash buckets.
#[derive(Clone, Default)]
pub(crate) struct ParentAssembly {
    positional: Vec<Option<AssembledChild>>,
    named: hashbrown::HashMap<AssemblyKey, AssembledChild>,
}

impl ParentAssembly {
    pub(crate) fn is_empty(&self) -> bool {
        self.named.is_empty() && self.positional.iter().all(Option::is_none)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&AssembledChild> {
        if let Some(slot) = AssemblyKey::parse_auto_view(key) {
            return self.positional.get(slot as usize).and_then(Option::as_ref);
        }
        self.named.get(key)
    }

    pub(crate) fn insert(
        &mut self,
        key: AssemblyKey,
        child: AssembledChild,
    ) -> Option<AssembledChild> {
        if let Some(slot) = AssemblyKey::parse_auto_view(key.as_str()) {
            let idx = slot as usize;
            if idx >= self.positional.len() {
                self.positional.resize(idx + 1, None);
            }
            return self.positional[idx].replace(child);
        }
        self.named.insert(key, child)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&str, &mut AssembledChild) -> bool) {
        for (idx, slot) in self.positional.iter_mut().enumerate() {
            if let Some(child) = slot {
                let key = AssemblyKey::auto_view(idx);
                if !keep(key.as_str(), child) {
                    *slot = None;
                }
            }
        }
        while matches!(self.positional.last(), Some(None)) {
            self.positional.pop();
        }
        self.named.retain(|key, child| keep(key.as_str(), child));
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (AssemblyKey, &AssembledChild)> {
        self.positional
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| {
                slot.as_ref()
                    .map(|child| (AssemblyKey::auto_view(idx), child))
            })
            .chain(self.named.iter().map(|(key, child)| (key.clone(), child)))
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &AssembledChild> {
        self.positional.iter().flatten().chain(self.named.values())
    }

    pub(crate) fn shrink_positional_excess(&mut self) {
        if self.positional.capacity() > self.positional.len().saturating_add(64) {
            self.positional.shrink_to_fit();
        }
    }
}

/// One entry of a parent's key table, taken out to be put back
/// ([`AppContext::assembled_key`], [`AppContext::rekey_assembled`]).
#[derive(Clone)]
pub(crate) struct AssembledKey {
    pub(super) key: AssemblyKey,
    pub(super) child: AssembledChild,
}

/// Identity-stable child builder for one retained parent.
pub struct AssemblyScope<'a> {
    context: &'a mut AppContext,
    parent: StableNodeId,
    document: DocumentId,
    /// The keys declared so far, in order, and as a set.
    seen: Vec<AssemblyKey>,
    seen_keys: hashbrown::HashSet<AssemblyKey>,
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
        // The children at the start and at the end that stay where they are
        // need no checking: they are this parent's children, each once. Only
        // what is asked for between them can be missing, repeated, or one of
        // those again, so a list changed in one place is checked there
        // rather than over its whole length.
        let (head, tail) = common_ends(&current, ordered);
        if head == current.len() && head == ordered.len() {
            return Ok(false);
        }
        let between = &ordered[head..ordered.len() - tail];
        let replaced: crate::NodeSet = current[head..current.len() - tail]
            .iter()
            .copied()
            .collect();
        let mut seen = crate::NodeSet::with_capacity_and_hasher(between.len(), Default::default());
        for &child in between {
            if !self.world.contains(child) {
                return Err(FrameworkError::MissingView(child));
            }
            let kept_in_place =
                self.world.parent_id(child) == Some(parent) && !replaced.contains(&child);
            if child == parent || kept_in_place || !seen.insert(child) {
                return Err(FrameworkError::InvalidComponentHierarchy { parent, child });
            }
        }
        let mut mutations = MutationQueue::new();
        reorder_between(parent, &current, ordered, (head, tail), &mut mutations);
        // commit_mutations validates the entire queue before changing either
        // the world or retained component lifecycles.
        self.commit_mutations(mutations)?;
        Ok(true)
    }

    /// Last `mount` key for `parent`; omitted keys disappear after remount.
    pub fn assembled_child(&self, parent: StableNodeId, key: &str) -> Option<StableNodeId> {
        self.assembled.get(&parent)?.get(key).map(|child| child.id)
    }

    /// Every keyed child of `parent`, with its key, in no particular order.
    pub(crate) fn assembled_children(
        &self,
        parent: StableNodeId,
    ) -> Vec<(AssemblyKey, StableNodeId)> {
        self.assembled.get(&parent).map_or_else(Vec::new, |slots| {
            slots.iter().map(|(key, child)| (key, child.id)).collect()
        })
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
            seen_keys: hashbrown::HashSet::new(),
        };
        build(&mut scope)?;
        scope.finish()
    }
}

/// What [`AssemblyScope::upsert`] found under its key.
enum Claim {
    /// The node the key named, of the same type.
    Reuse(StableNodeId),
    /// No such node: create one and record it under this key.
    Create(AssemblyKey),
}

impl AssemblyScope<'_> {
    /// What the framework's own controls say, for a composite assembling
    /// one of them.
    pub fn framework_strings(&self) -> &nana_ui_core::FrameworkStrings {
        self.context.world().framework_strings()
    }

    pub fn child<C: ComponentView>(
        &mut self,
        key: impl AsRef<str>,
        component: C,
    ) -> Result<Entity<C>, FrameworkError> {
        self.upsert(AssemblyKey::from(key.as_ref()), component)
    }

    pub fn with_child<C: ComponentView>(
        &mut self,
        key: impl AsRef<str>,
        component: C,
        children: impl FnOnce(&mut AssemblyScope<'_>) -> Result<(), FrameworkError>,
    ) -> Result<Entity<C>, FrameworkError> {
        let entity = self.upsert(AssemblyKey::from(key.as_ref()), component)?;
        let document = self.document;
        let mut nested = AssemblyScope {
            context: self.context,
            parent: entity.id,
            document,
            seen: Vec::new(),
            seen_keys: hashbrown::HashSet::new(),
        };
        children(&mut nested)?;
        nested.finish()?;
        Ok(entity)
    }

    fn upsert<C: ComponentView>(
        &mut self,
        key: AssemblyKey,
        component: C,
    ) -> Result<Entity<C>, FrameworkError> {
        let type_id = TypeId::of::<C>();
        match self.claim(key, type_id)? {
            Claim::Reuse(id) => {
                let entity = Entity::from_stable_id(id);
                self.context
                    .update_component(entity, |view: &mut C, _| view.reconcile(component))?;
                Ok(entity)
            }
            Claim::Create(key) => {
                let entity = self
                    .context
                    .create_detached_component(self.document, component)?;
                self.record(key, entity.id)?;
                Ok(entity)
            }
        }
    }

    /// The keyed half of [`Self::upsert`] before the component is used: the
    /// key is checked and marked seen, and the node it named is reused or
    /// removed. Not generic, so it is compiled once.
    #[inline(never)]
    fn claim(&mut self, key: AssemblyKey, type_id: TypeId) -> Result<Claim, FrameworkError> {
        if !super::valid_assembly_key(&key) {
            return Err(FrameworkError::InvalidInput);
        }
        if !self.seen_keys.insert(key.clone()) {
            return Err(FrameworkError::DuplicateAssemblyKey {
                parent: self.parent,
                key: key.to_string(),
            });
        }
        self.seen.push(key.clone());
        if let Some(existing) = self
            .context
            .assembled
            .get(&self.parent)
            .and_then(|slots| slots.get(key.as_str()))
            .copied()
        {
            if self.context.view_type_id(existing.id) == Some(type_id) {
                return Ok(Claim::Reuse(existing.id));
            }
            self.context.despawn_node(existing.id)?;
        }
        Ok(Claim::Create(key))
    }

    /// The keyed half of [`Self::upsert`] after a new node: it goes under
    /// the parent and into the parent's key table.
    #[inline(never)]
    fn record(&mut self, key: AssemblyKey, id: StableNodeId) -> Result<(), FrameworkError> {
        self.context.attach_child(self.parent, id)?;
        // Into the parent's table as it stands. Copying the table to add one
        // key made a parent's first assembly cost the square of its children
        // (6 s for 16,000).
        self.context.rekey_assembled(
            self.parent,
            AssembledKey {
                key,
                child: AssembledChild { id },
            },
        );
        Ok(())
    }

    fn finish(self) -> Result<(), FrameworkError> {
        let unused: Vec<_> = self
            .context
            .assembled
            .get(&self.parent)
            .into_iter()
            .flat_map(|slots| slots.iter())
            .filter(|(key, _)| !self.seen_keys.contains(key.as_str()))
            .map(|(_, child)| child.id)
            .collect();
        self.context.despawn_nodes(&unused)?;
        if let Some(mut slots) = self.context.assembled.get(&self.parent).cloned() {
            slots.retain(|key, _| self.seen_keys.contains(key));
            self.context.store_assembled(self.parent, slots);
        }
        // A child placed under another parent keeps its identity here but is
        // not pulled back.
        let desired: Vec<_> = self
            .seen
            .iter()
            .filter_map(|key| {
                self.context
                    .assembled
                    .get(&self.parent)?
                    .get(key.as_str())
                    .map(|child| child.id)
            })
            .filter(|&id| !self.context.is_placed_elsewhere(id))
            .collect();
        let current = self
            .context
            .world
            .node(self.parent)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        let assembled: hashbrown::HashSet<_> = desired.iter().copied().collect();
        let mut ordered: Vec<_> = current
            .iter()
            .copied()
            .filter(|id| !assembled.contains(id))
            .collect();
        ordered.extend(desired);
        if current.as_slice() == ordered.as_slice() {
            return Ok(());
        }
        // Every child that is not assembled is kept in front, so `ordered`
        // leaves none out and nothing is parked.
        let mut mutations = MutationQueue::new();
        let ends = common_ends(&current, &ordered);
        reorder_between(self.parent, &current, &ordered, ends, &mut mutations);
        self.context.commit_mutations(mutations)?;
        Ok(())
    }
}

/// How many children of a list an insert costs as much as looking at one of
/// them for a reorder does. An insert finds its place in the list and shifts
/// it, about 2 ns a child of the whole list and a microsecond besides; a
/// reorder finds which children moved among the ones between the first and
/// the last that did, up to 75 ns each when most of them stay.
const REORDER_LOOKS: usize = 32;
/// The microsecond an insert costs besides, in children of its list.
const INSERT_BESIDES: usize = 1024;

/// Whether `moves` children moved or brought into a list of `list` are
/// cheaper as one [`crate::UiMutation::ReorderChildren`] that has to look
/// through `looked` children than as an insert each. Two rows exchanged
/// across a long list are two inserts; a block turned round, a list sorted
/// or rows filled in between others are one reorder.
pub(crate) fn reorder_is_cheaper(moves: usize, list: usize, looked: usize) -> bool {
    moves >= 2
        && moves.saturating_mul(list.saturating_add(INSERT_BESIDES))
            >= looked.saturating_mul(REORDER_LOOKS)
}

/// Shared projection path for framework components already building one batch.
/// Input validity is checked by that batch's normal Runtime transaction.
///
/// Moves as few children as the new order allows: the longest run of kept
/// children already in increasing order stays put, and every other child is
/// inserted before its successor. Reversing costs n moves, inserting or
/// removing one child costs one. Children of `current` missing from
/// `ordered` are left alone.
pub(crate) fn move_into_order(
    parent: StableNodeId,
    current: &[StableNodeId],
    ordered: &[StableNodeId],
    mutations: &mut MutationQueue,
) {
    let reorders = move_between(
        parent,
        current,
        ordered,
        common_ends(current, ordered),
        false,
        mutations,
    );
    debug_assert!(!reorders, "only asked for when the order is complete");
}

/// [`move_into_order`] with the common head and tail found already. They
/// stay put: one row inserted or removed anywhere leaves only the rows
/// between to compare.
///
/// `complete` says that `current` is every child `parent` has and that the
/// caller takes out the ones `ordered` leaves out, so `ordered` is the whole
/// list afterwards. Then many moves are not queued one by one: the children
/// that come from elsewhere are appended, and `true` is returned for the
/// caller to queue one reorder to `ordered` once the others are out.
#[must_use]
fn move_between(
    parent: StableNodeId,
    current: &[StableNodeId],
    ordered: &[StableNodeId],
    (head, tail): (usize, usize),
    complete: bool,
    mutations: &mut MutationQueue,
) -> bool {
    let list = current.len();
    let (current, ordered_middle) = (
        &current[head..current.len() - tail],
        &ordered[head..ordered.len() - tail],
    );
    let positions = positions_in(current, ordered_middle);
    let stays = longest_increasing(&positions);
    let moves = stays.iter().filter(|stays| !**stays).count();
    // A child brought in is appended first, so the reorder looks through
    // everything after the common head to find it at the end.
    let brings = positions.iter().any(Option::is_none);
    let looked = if brings {
        ordered.len() - head
    } else {
        ordered_middle.len()
    };
    if complete
        && reorder_is_cheaper(moves, list, looked)
        && each_once(ordered_middle, &positions, current.len())
    {
        for (&child, position) in ordered_middle.iter().zip(&positions) {
            if position.is_none() {
                mutations.insert(parent, child, None);
            }
        }
        return true;
    }
    // Walking backwards, each moved child goes before the one that follows
    // it, which is already where it belongs.
    let mut next = ordered.get(ordered.len() - tail).copied();
    for (index, &child) in ordered_middle.iter().enumerate().rev() {
        if !stays[index] {
            mutations.insert(parent, child, next);
        }
        next = Some(child);
    }
    false
}

/// Whether no child is named twice: `positions` are where `ordered`'s
/// children are among the `kept` it may keep, `None` for one from elsewhere.
/// An order that repeats a child is left to the inserts, which take it as
/// they always have.
fn each_once(ordered: &[StableNodeId], positions: &[Option<usize>], kept: usize) -> bool {
    let mut placed = vec![false; kept];
    let mut brought = crate::NodeSet::default();
    ordered
        .iter()
        .zip(positions)
        .all(|(child, position)| match position {
            Some(position) => !std::mem::replace(&mut placed[*position], true),
            None => brought.insert(*child),
        })
}

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
    let ends = common_ends(&current, ordered);
    if ends.0 == current.len() && ends.0 == ordered.len() {
        return false;
    }
    reorder_between(parent, &current, ordered, ends, mutations);
    true
}

/// Moves `parent`'s children, all of them in `current`, into `ordered` and
/// parks the ones it leaves out, given the head and tail the two share.
fn reorder_between(
    parent: StableNodeId,
    current: &[StableNodeId],
    ordered: &[StableNodeId],
    (head, tail): (usize, usize),
    mutations: &mut MutationQueue,
) {
    // Extract retained descendants before parking their former ancestors. A
    // live-to-live reparent must never retire focus, IME or pointer ownership.
    let reorders = move_between(parent, current, ordered, (head, tail), true, mutations);
    // A child that is not wanted any more lies between the common head and
    // tail, like everything else that changed.
    let keep: crate::NodeSet = ordered[head..ordered.len() - tail]
        .iter()
        .copied()
        .collect();
    for child in &current[head..current.len() - tail] {
        if !keep.contains(child) {
            mutations.park_subtree(*child);
        }
    }
    if reorders {
        mutations.reorder_children(parent, ordered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, Stack};

    fn children(context: &AppContext, parent: StableNodeId) -> Vec<StableNodeId> {
        context.world().node(parent).unwrap().children.to_vec()
    }

    /// Reorder `parent` to `ordered`, returning how many children that moved
    /// or brought in, and the mutations it queued.
    fn reorder_queued(
        context: &mut AppContext,
        parent: StableNodeId,
        ordered: &[StableNodeId],
    ) -> (usize, Vec<crate::UiMutation>) {
        let mut mutations = MutationQueue::new();
        reconcile_child_order(parent, ordered, context.world(), &mut mutations);
        let queued = mutations.as_slice().to_vec();
        let report = context.commit_mutations(mutations).unwrap();
        assert_eq!(children(context, parent), ordered);
        (report.reparented + report.inserted, queued)
    }

    fn reorder(context: &mut AppContext, parent: StableNodeId, ordered: &[StableNodeId]) -> usize {
        reorder_queued(context, parent, ordered).0
    }

    #[test]
    fn a_common_head_and_tail_stay_put() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = context
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let mut all = Vec::new();
        for _ in 0..8 {
            let child = context
                .create_component(document, Stack::column(0.0))
                .unwrap()
                .stable_id();
            all.push(child);
        }
        let mut mutations = MutationQueue::new();
        for child in &all {
            mutations.insert(parent, *child, None);
        }
        context.commit_mutations(mutations).unwrap();
        // Swap the middle two: head and tail of three each stay.
        let mut swapped = all.clone();
        swapped.swap(3, 4);
        assert_eq!(reorder(&mut context, parent, &swapped), 1);
        // A new first child moves nothing else.
        let fresh = context
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let mut with_fresh = vec![fresh];
        with_fresh.extend(&swapped);
        assert_eq!(reorder(&mut context, parent, &with_fresh), 1);
    }

    #[test]
    fn reordering_moves_only_children_outside_the_longest_kept_run() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = context
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let other = context
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let mut all = Vec::new();
        for _ in 0..40 {
            let child = context
                .create_component(document, Stack::column(0.0))
                .unwrap()
                .stable_id();
            context
                .append_child(
                    crate::Entity::<Stack>::from_stable_id(parent),
                    crate::Entity::<Stack>::from_stable_id(child),
                )
                .unwrap();
            all.push(child);
        }
        let outsider = context
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        context
            .append_child(
                crate::Entity::<Stack>::from_stable_id(other),
                crate::Entity::<Stack>::from_stable_id(outsider),
            )
            .unwrap();

        let mut order = all.clone();
        // Insert at the front (taken from another parent): one move.
        order.insert(0, outsider);
        assert_eq!(reorder(&mut context, parent, &order), 1);
        // Remove from the middle: no move, one park.
        order.remove(20);
        assert_eq!(reorder(&mut context, parent, &order), 0);
        // Move one child from the end to the front: one move.
        let last = order.pop().unwrap();
        order.insert(0, last);
        assert_eq!(reorder(&mut context, parent, &order), 1);
        // Swap two: two moves at most.
        order.swap(3, 30);
        assert!(reorder(&mut context, parent, &order) <= 2);
        // Reverse: everything but one child moves.
        order.reverse();
        assert_eq!(reorder(&mut context, parent, &order), order.len() - 1);
        // Pseudo-random shuffles land exactly, with moves bounded by n - LIS.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20 {
            for index in (1..order.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                order.swap(index, (seed % (index as u64 + 1)) as usize);
            }
            let before = children(&context, parent);
            let positions: Vec<Option<usize>> = order
                .iter()
                .map(|id| before.iter().position(|b| b == id))
                .collect();
            let kept = longest_increasing(&positions)
                .iter()
                .filter(|s| **s)
                .count();
            assert_eq!(reorder(&mut context, parent, &order), order.len() - kept);
        }
    }

    /// One move is an insert, and so are two far apart in a long list. More
    /// of them close together are one reorder, after the inserts that bring
    /// children in and the parks that take them out, and the children land
    /// the same.
    #[test]
    fn many_moves_are_queued_as_one_reorder() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut stack = || {
            context
                .create_component(document, Stack::column(0.0))
                .unwrap()
                .stable_id()
        };
        let parent = stack();
        let all: Vec<_> = (0..200).map(|_| stack()).collect();
        let (fresh, other) = (stack(), stack());
        let mut mutations = MutationQueue::new();
        for child in &all {
            mutations.insert(parent, *child, None);
        }
        context.commit_mutations(mutations).unwrap();
        let kinds = |queued: &[crate::UiMutation]| {
            let count =
                |of: fn(&crate::UiMutation) -> bool| queued.iter().filter(|m| of(m)).count();
            (
                count(|m| matches!(m, crate::UiMutation::Insert { .. })),
                count(|m| matches!(m, crate::UiMutation::ParkSubtree { .. })),
                count(|m| matches!(m, crate::UiMutation::ReorderChildren { .. })),
            )
        };

        let mut order = all.clone();
        order[40..60].rotate_left(1);
        let (moves, queued) = reorder_queued(&mut context, parent, &order);
        assert_eq!((moves, kinds(&queued)), (1, (1, 0, 0)));
        // The first and the last exchanged: a reorder would look through
        // every child between them to move two.
        order.swap(0, 199);
        let (moves, queued) = reorder_queued(&mut context, parent, &order);
        assert_eq!((moves, kinds(&queued)), (2, (2, 0, 0)));
        // Ten in a row turned round: nine moves, found among those ten.
        order[90..100].reverse();
        let (moves, queued) = reorder_queued(&mut context, parent, &order);
        assert_eq!((moves, kinds(&queued)), (9, (0, 0, 1)));

        // Everything turned round, one child dropped, one brought in from
        // nowhere and one from under another parent.
        context
            .append_child(
                crate::Entity::<Stack>::from_stable_id(fresh),
                crate::Entity::<Stack>::from_stable_id(other),
            )
            .unwrap();
        order.reverse();
        let dropped = order.remove(5);
        order.insert(9, fresh);
        order.insert(20, other);
        let (moves, queued) = reorder_queued(&mut context, parent, &order);
        assert_eq!(kinds(&queued), (2, 1, 1));
        assert!(matches!(
            queued.last(),
            Some(crate::UiMutation::ReorderChildren { .. })
        ));
        // The two brought in, and every kept child but one.
        assert_eq!(moves, 2 + (order.len() - 2 - 1));
        assert_eq!(context.world().parent_id(dropped), None);
        assert_eq!(context.world().parent_id(other), Some(parent));
        assert_eq!(children(&context, fresh), []);
    }

    #[test]
    fn the_longest_increasing_run_skips_unknown_positions() {
        let positions = [Some(3), None, Some(0), Some(1), Some(4), Some(2), None];
        let marks = longest_increasing(&positions);
        let run: Vec<usize> = positions
            .iter()
            .zip(&marks)
            .filter(|(_, stays)| **stays)
            .map(|(position, _)| position.expect("marked known"))
            .collect();
        assert_eq!(run.len(), 3);
        assert!(run.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn assembly_key_is_16_bytes_and_preserves_borrow_eq_hash() {
        assert_eq!(std::mem::size_of::<AssemblyKey>(), 16);
        let short = AssemblyKey::from("0123456789abcd");
        let empty = AssemblyKey::from("");
        let long = AssemblyKey::from("0123456789abcde".to_owned());
        assert!(matches!(short, AssemblyKey::Inline { .. }));
        assert!(matches!(empty, AssemblyKey::Inline { .. }));
        assert!(matches!(long, AssemblyKey::Shared(_)));
        assert_eq!(short.as_str(), "0123456789abcd");
        assert_eq!(long.as_str(), "0123456789abcde");

        let auto_short = AssemblyKey::auto_view(42);
        let auto_kind = AssemblyKey::auto_kind("Stack", 123);
        assert_eq!(auto_short.as_str(), "#v42");
        assert_eq!(auto_kind.as_str(), "#Stack-123");
        assert!(matches!(auto_short, AssemblyKey::Inline { .. }));
        assert!(matches!(auto_kind, AssemblyKey::Inline { .. }));

        let mut set = hashbrown::HashSet::new();
        set.insert(short.clone());
        set.insert(long.clone());
        assert!(set.contains("0123456789abcd"));
        assert!(set.contains("0123456789abcde"));

        assert_eq!(AssemblyKey::parse_auto_view("#v0"), Some(0));
        assert_eq!(AssemblyKey::parse_auto_view("#v42"), Some(42));
        assert_eq!(AssemblyKey::parse_auto_view("#v01"), None);
        assert_eq!(AssemblyKey::parse_auto_view("#v65534"), Some(65534));
        assert_eq!(AssemblyKey::parse_auto_view("#v65535"), None);
    }

    #[test]
    fn parent_assembly_splits_positional_and_named_keys() {
        let mut slots = ParentAssembly::default();
        let a = AssembledChild {
            id: StableNodeId::new(1).unwrap(),
        };
        let b = AssembledChild {
            id: StableNodeId::new(2).unwrap(),
        };
        let c = AssembledChild {
            id: StableNodeId::new(3).unwrap(),
        };
        slots.insert(AssemblyKey::auto_view(2), a);
        slots.insert(AssemblyKey::from("header"), b);
        slots.insert(AssemblyKey::from("#v01"), c);
        assert_eq!(slots.get("#v2").map(|e| e.id), Some(a.id));
        assert_eq!(slots.get("header").map(|e| e.id), Some(b.id));
        assert_eq!(slots.get("#v01").map(|e| e.id), Some(c.id));
        assert!(slots.named.contains_key("#v01"));
        assert!(!slots.named.contains_key("#v2"));

        slots.retain(|key, _| key == "header");
        assert!(slots.positional.is_empty());
        assert_eq!(slots.get("#v2").map(|e| e.id), None);
        assert_eq!(slots.get("#v01").map(|e| e.id), None);
        assert_eq!(slots.get("header").map(|e| e.id), Some(b.id));
    }
}
