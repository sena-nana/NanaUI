//! Dense node table plus sparse side maps. Replaces `bevy_ecs::World` inside
//! [`crate::UiWorld`]. Public identity stays [`crate::StableNodeId`].

mod paged;

use hashbrown::HashMap;
use std::sync::{Arc, LazyLock};

use crate::ComputedStyle;
use crate::component_registry::ComponentTypeId;
use crate::components::{
    AccessibilityState, CustomRenderNode, EmptyStateTextPresentation, EventListeners, ImeView,
    InteractionState, LayoutBox, ModalTextPresentation, MountState, NodeStyle, OverlayHostState,
    ScrollMetrics, ScrollOffset, StandardVisual, TextCodeFold, TextContent, TextInputPresentation,
    TextInputView, TextMetrics, TextSnippetSession,
};
use crate::presentation::{HighlightRequest, TextPresentation};
use crate::schedule::DirtyMask;
use crate::world::{DocumentId, NodeKind, StableNodeId};

static EMPTY_CHILDREN: LazyLock<Arc<Vec<StableNodeId>>> = LazyLock::new(|| Arc::new(Vec::new()));
static INTERNED_KIND_DOCUMENT: LazyLock<Arc<NodeKind>> =
    LazyLock::new(|| Arc::new(NodeKind::Document));
static INTERNED_KIND_TEXT: LazyLock<Arc<NodeKind>> = LazyLock::new(|| Arc::new(NodeKind::Text));
static INTERNED_KIND_COMMENT: LazyLock<Arc<NodeKind>> =
    LazyLock::new(|| Arc::new(NodeKind::Comment));
static INTERNED_KIND_DIV: LazyLock<Arc<NodeKind>> =
    LazyLock::new(|| Arc::new(NodeKind::Element { tag: "div".into() }));
static INTERNED_DEFAULT_STYLE: LazyLock<Arc<ComputedStyle>> =
    LazyLock::new(|| Arc::new(ComputedStyle::default()));

/// How many element tags a store shares kinds for ([`NodeStore::intern_kind`]).
const KINDS: usize = 256;

fn intern_kind(kind: &NodeKind) -> Arc<NodeKind> {
    match kind {
        NodeKind::Document => Arc::clone(&INTERNED_KIND_DOCUMENT),
        NodeKind::Text => Arc::clone(&INTERNED_KIND_TEXT),
        NodeKind::Comment => Arc::clone(&INTERNED_KIND_COMMENT),
        NodeKind::Element { tag } if tag == "div" => Arc::clone(&INTERNED_KIND_DIV),
        _ => Arc::new(kind.clone()),
    }
}

pub(crate) fn intern_empty_children(children: &mut Arc<Vec<StableNodeId>>) {
    if children.is_empty() {
        *children = Arc::clone(&EMPTY_CHILDREN);
    }
}

#[derive(Clone)]
pub(crate) struct Hierarchy {
    pub parent: Option<StableNodeId>,
    pub children: Arc<Vec<StableNodeId>>,
    /// This node's index in its parent's `children`, counted from the
    /// parent's [`Self::children_base`] and wrapping: finding a sibling
    /// reads it instead of scanning the list
    /// (`UiWorld::renumber_children_between` keeps it).
    pub index_in_parent: u32,
    /// What this node's first child records as its `index_in_parent`. An
    /// edit that shifts a block of the children moves this instead of
    /// writing each of them a new number.
    pub children_base: u32,
    /// Levels from the root down to this node, counting it: 1 for a root.
    /// Moving a subtree renumbers it (`UiWorld::set_subtree_depth`), so
    /// checking an insert against `MAX_TREE_DEPTH` reads its parent's depth
    /// instead of walking to the root.
    pub depth: u16,
}

impl Default for Hierarchy {
    fn default() -> Self {
        Self {
            parent: None,
            children: Arc::clone(&EMPTY_CHILDREN),
            index_in_parent: 0,
            children_base: 0,
            depth: 1,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ResolvedStyle(pub Arc<ComputedStyle>, pub u64);

impl ResolvedStyle {
    fn interned_default() -> Self {
        Self(Arc::clone(&INTERNED_DEFAULT_STYLE), 0)
    }
}

/// The process-wide default a root node inherits from. Sharing this avoids
/// building a fresh `ComputedStyle` per root during style resolution.
pub(crate) fn interned_default_style() -> Arc<ComputedStyle> {
    Arc::clone(&INTERNED_DEFAULT_STYLE)
}

#[repr(align(64))]
pub(crate) struct NodeRecord {
    pub document: DocumentId,
    pub kind: Arc<NodeKind>,
    pub hierarchy: Hierarchy,
    pub mount: MountState,
    pub style: Arc<NodeStyle>,
    /// [`Self::style`]'s layout with the node's design intent already applied
    /// (radius tier → `border_radius`), resolved against the installed
    /// `ThemeMetrics`.
    ///
    /// Resolved on write — when the style is set and when a theme install
    /// changes the metrics — rather than on every read. `LayoutStyle` is 4.8 KB,
    /// so resolving it inside extraction meant an `Arc::make_mut` copy per
    /// control per frame; a palette switch over 1k controls cost three times
    /// what it should. Resolving on write pays that once per change and hands
    /// every reader an `Arc` it can clone.
    ///
    /// [`Self::style`] stays exactly as authored, because that is what
    /// projection diffs against.
    pub resolved_layout: Arc<nana_ui_core::LayoutStyle>,
    pub resolved: ResolvedStyle,
    /// Whether [`Self::style`]'s layout reads the viewport, kept with it so
    /// presence bookkeeping after a visual or text write does not rescan
    /// the layout's lengths.
    pub layout_depends_on_viewport: bool,
    /// Whether this node's subtree reads the block extent it is offered,
    /// kept up to date as styles and child lists change. See
    /// `world/block_reads.rs`.
    pub block_reads: crate::world::BlockExtentReads,
    /// The writing mode, direction and text orientation this node inherits:
    /// its parent's computed values, as last resolved. Kept beside the
    /// record, not only inside the parent's `ComputedStyle`, so layout reads a
    /// node's writing context and its containing block's without a parent
    /// lookup or an `Arc` deref.
    pub inherited_writing: nana_ui_core::WritingContext,
    pub inherited_orientation: nana_ui_core::TextOrientationSpec,
    pub text: TextContent,
    pub text_metrics: TextMetrics,
    /// The box layout last wrote, measured from [`Self::layout_frame`]'s
    /// origin. Its document box is derived ([`crate::UiWorld::layout_box`]):
    /// a subtree that moves keeps every box under its root.
    pub local_layout: LayoutBox,
    pub layout_frame: crate::LayoutFrame,
    /// How this node stands to the coordinate spaces around it, from
    /// [`Self::resolved_layout`], [`Self::layout_frame`], its kind and its
    /// visual ([`crate::node_space::NodeSpace::of`]). Kept up to date by
    /// [`NodeStore::note_node_space`] wherever one of those is written.
    pub space: crate::node_space::NodeSpace,
    pub layout_padding: Option<nana_ui_core::PaddingSpec>,
    /// Page axes the last layout placed the children from the far end.
    pub layout_far_start: Option<[bool; 2]>,
    /// The formatting context the last layout ran for the children.
    pub layout_context: Option<crate::FormattingContextKind>,
    /// Bumped each time [`Self::layout_context`] changes.
    pub layout_context_generation: u64,
    pub scroll_offset: ScrollOffset,
    pub interaction: InteractionState,
    /// Shared: most nodes keep the default, one allocation for all of them
    /// ([`shared_accessibility`]), which keeps the record in a smaller
    /// allocation size class.
    pub accessibility: Arc<AccessibilityState>,
    pub dirty: DirtyMask,
    /// Why the node is marked for render extraction, in the world's
    /// extraction epochs: the last epoch anything but a move marked it
    /// (or it was made), and the last one a layout move marked it. Only a
    /// node a move marked and nothing else did since the epoch began may
    /// reuse its last extracted record.
    pub render_full_epoch: u64,
    pub render_moved_epoch: u64,
    /// The last epoch a move above an open select field marked it only to
    /// ask whether its menu's surface still lands where its record has it
    /// (`UiWorld::local_select_menu_surface`): if so, and nothing else
    /// marked it since the epoch began, its record is kept.
    pub render_room_epoch: u64,
}

static DEFAULT_ACCESSIBILITY: std::sync::LazyLock<Arc<AccessibilityState>> =
    std::sync::LazyLock::new(|| Arc::new(AccessibilityState::default()));

/// `state` as a record holds it: the one shared default when it is the
/// default, its own allocation otherwise.
pub(crate) fn shared_accessibility(state: &AccessibilityState) -> Arc<AccessibilityState> {
    if *state == **DEFAULT_ACCESSIBILITY {
        Arc::clone(&DEFAULT_ACCESSIBILITY)
    } else {
        Arc::new(state.clone())
    }
}

impl NodeRecord {
    #[cfg(test)]
    pub fn new(document: DocumentId, kind: &NodeKind, interaction: InteractionState) -> Self {
        Self::with_kind(
            document,
            intern_kind(kind),
            Arc::new(NodeStyle::default()),
            interaction,
        )
    }

    /// [`Self::new`] with the kind already shared ([`NodeStore::intern_kind`])
    /// and the default style a store shares ([`NodeStore::default_style`]).
    pub(crate) fn with_kind(
        document: DocumentId,
        kind: Arc<NodeKind>,
        style: Arc<NodeStyle>,
        interaction: InteractionState,
    ) -> Self {
        let space = crate::node_space::NodeSpace::of(
            &style.layout,
            crate::LayoutFrame::Parent,
            &kind,
            None,
        );
        Self {
            document,
            kind,
            hierarchy: Hierarchy::default(),
            mount: MountState::default(),
            resolved_layout: Arc::clone(&style.layout),
            style,
            resolved: ResolvedStyle::interned_default(),
            layout_depends_on_viewport: false,
            block_reads: crate::world::BlockExtentReads::default(),
            inherited_writing: nana_ui_core::WritingContext::default(),
            inherited_orientation: nana_ui_core::TextOrientationSpec::Mixed,
            text: TextContent::default(),
            text_metrics: TextMetrics::default(),
            local_layout: LayoutBox::default(),
            layout_frame: crate::LayoutFrame::Parent,
            space,
            layout_padding: None,
            layout_far_start: None,
            layout_context: None,
            layout_context_generation: 0,
            scroll_offset: ScrollOffset::default(),
            interaction,
            accessibility: Arc::clone(&DEFAULT_ACCESSIBILITY),
            dirty: DirtyMask::all(),
            render_full_epoch: 0,
            render_moved_epoch: u64::MAX,
            render_room_epoch: u64::MAX,
        }
    }
}

macro_rules! sparse {
    ($field:ident, $ty:ty, $get:ident, $set:ident) => {
        pub fn $get(&self, id: StableNodeId) -> Option<&$ty> {
            self.$field.get(&id)
        }

        pub fn $set(&mut self, id: StableNodeId, value: Option<$ty>) {
            if let Some(value) = value {
                self.$field.insert(id, value);
            } else {
                self.$field.remove(&id);
            }
        }
    };
}

/// 代码折叠视图状态（仅喂过折叠区间的编辑器节点持有条目）。
#[derive(Clone, Debug, Default)]
pub(crate) struct TextFoldViewState {
    /// 宿主上一次喂入的折叠区间（用于重喂时的漂移匹配）。
    pub offered: Arc<[TextCodeFold]>,
    /// 当前折叠态的区间（`offered` 的子集，按 `start` 排序）。
    pub collapsed: Vec<TextCodeFold>,
}

/// 补全弹层会话状态（仅候选会话活跃的编辑器节点持有条目）。
///
/// 候选列表由宿主喂入（过滤是宿主的职责），组件只维护键盘选中与滚动：
/// 喂入相同列表不重置选中态（包括 Esc 关闭态）；喂入不同列表时视为新
/// 会话（选中归零、重新打开）；喂入空列表整个条目移除（零分配待机）。
#[derive(Clone, Debug)]
pub(crate) struct TextCompletionViewState {
    pub items: Arc<[crate::TextCompletion]>,
    pub selected: usize,
    /// 第一条可见候选的绝对下标。
    pub scroll: usize,
    /// Esc 关闭标记：会话数据保留（宿主重喂相同列表不复活弹层），
    /// 换新列表或宿主撤空重喂后清除。
    pub dismissed: bool,
}

/// hover 文档浮窗状态（仅宿主喂入 hover 的编辑器节点持有条目）。
#[derive(Clone, Debug)]
pub(crate) struct TextHoverViewState {
    pub doc: crate::TextHover,
    /// 正文滚过的逻辑行数。
    pub scroll: usize,
    /// 由诊断 span 指针命中写入。离开 span 时框架撤掉；宿主喂入的
    /// 符号 hover 把此位置为 false。
    pub diagnostic: bool,
}

/// One text editor, stored once: the session is the only copy of its
/// committed text, its selections and its IME composition (Issue #182).
#[derive(Debug)]
pub(crate) struct EditorRecord {
    pub session: nana_text::EditSession,
    /// An IME attached with nothing composed: the preedit a platform reports
    /// as empty between keystrokes, or when composition just ended and the
    /// commit is still on its way. Not a composition — the session has none,
    /// and caret motion and display do not treat it as one — but it is still
    /// the IME's turn: a submit waits for its commit or its end. Carries the
    /// IME's (empty) selection so the state reads back as it was written.
    pub empty_preedit: Option<Option<(usize, usize)>>,
    /// What the editor draws, built from the session and the overlays on it
    /// once per change of those, not once per consumer and pass.
    pub display: std::cell::RefCell<EditorDisplayMemo>,
}

/// Memoized display pieces of one editor, each with the inputs it was built
/// from. The keys are stamps and revisions, not dirty bits: a stale entry
/// cannot survive a change it did not see.
#[derive(Debug, Default)]
pub(crate) struct EditorDisplayMemo {
    /// The fold / inlay display view.
    pub view: Option<(DisplayViewKey, Option<crate::world::TextDisplayView>)>,
    /// The committed text with the preedit spliced in, stamped.
    pub composed: Option<((nana_text::TextStamp, u64), crate::TextValue)>,
}

/// What a fold / inlay view is a function of.
#[derive(Debug, Clone)]
pub(crate) struct DisplayViewKey {
    pub text: nana_text::TextStamp,
    pub collapsed: Vec<crate::TextCodeFold>,
    /// The inlay set, by identity: a new feed is a new `Arc`. Held, not
    /// just its address: a freed feed's address can come back for the next
    /// one, and a key that did not keep it alive would then match it.
    pub inlays: Option<Arc<[crate::TextInlay]>>,
    pub composing: bool,
}

impl PartialEq for DisplayViewKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.composing == other.composing
            && self.collapsed == other.collapsed
            && match (&self.inlays, &other.inlays) {
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl EditorRecord {
    pub fn new(session: nana_text::EditSession) -> Self {
        Self {
            session,
            empty_preedit: None,
            display: Default::default(),
        }
    }

    pub fn ime(&self) -> Option<ImeView<'_>> {
        if let Some(composition) = self.session.composition() {
            return Some(ImeView {
                text: &composition.text,
                selection: composition
                    .selection
                    .as_ref()
                    .map(|range| (range.start, range.end)),
            });
        }
        self.empty_preedit.map(|selection| ImeView {
            text: "",
            selection,
        })
    }
}

/// Ids below this index slots. A larger one, which the
/// allocator reaches only after sixteen million nodes and a caller can
/// otherwise only make up, stays in the overflow map: the directory of pages
/// is as long as the span of ids in use, and this bounds it at 2 MB.
const PAGED_IDS: u64 = 1 << 24;

/// The paged slot `id` has, if it has one.
#[inline]
fn paged_index(id: StableNodeId) -> Option<usize> {
    let raw = id.get();
    (raw < PAGED_IDS).then_some(raw as usize)
}

/// The id whose slot `index` is: [`paged_index`] backwards. A slot holds a
/// value only at an id's index, and ids are never zero.
#[inline]
fn slot_id(index: usize) -> StableNodeId {
    StableNodeId::new(index as u64).expect("a held slot is an id's")
}

/// A map keyed by node, laid out as [`NodeTable`] lays out the records:
/// ids index slots ([`paged::Slots`]: one run while they are close together,
/// pages once they are not), the few past [`PAGED_IDS`] fall back to a hash
/// map. A pass looks the same nodes up many times over,
/// and indexing costs neither a hash nor a probe; a walk in id order (the
/// order nodes are created in) reads the table front to back instead of at
/// random.
///
/// The API follows `HashMap<StableNodeId, V>`, so a table holding about one
/// entry per node can change type without its users changing, except that
/// iteration hands out ids by value. Iteration is in id order, then the
/// overflow's; nothing may depend on either. What it holds follows the
/// entries in it, not the highest id it has seen.
///
/// A slot holds the value alone: its index is the id. Holding the id beside
/// it took 8 bytes a slot in every table, and more where the value is small
/// (a context kind of one byte took a slot of 16).
#[derive(Clone)]
pub(crate) struct DenseNodeMap<V> {
    paged: paged::Slots<V>,
    overflow: crate::NodeMap<V>,
}

impl<V> Default for DenseNodeMap<V> {
    fn default() -> Self {
        Self {
            paged: paged::Slots::default(),
            overflow: crate::NodeMap::default(),
        }
    }
}

impl<V: std::fmt::Debug> std::fmt::Debug for DenseNodeMap<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V: PartialEq> PartialEq for DenseNodeMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(id, value)| other.get(&id) == Some(value))
    }
}

impl<V> DenseNodeMap<V> {
    /// A map about to hold `ids`, with room for them when they are close
    /// enough together to be held as one run.
    pub(crate) fn with_highest_of(ids: &[StableNodeId]) -> Self {
        let mut map = Self::default();
        if let Some(&highest) = ids.iter().max() {
            map.reserve_through(highest, ids.len());
        }
        map
    }

    pub(crate) fn len(&self) -> usize {
        self.paged.len() + self.overflow.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub(crate) fn get(&self, id: &StableNodeId) -> Option<&V> {
        match self.paged.run_slot(id.get() as usize) {
            Some(slot) => slot.as_ref(),
            None => self.past_run(id),
        }
    }

    /// [`Self::get`] for an id the run does not reach.
    #[inline(never)]
    fn past_run(&self, id: &StableNodeId) -> Option<&V> {
        match paged_index(*id) {
            Some(index) => self.paged.past_run(index),
            None => self.overflow.get(id),
        }
    }

    #[inline(always)]
    pub(crate) fn get_mut(&mut self, id: &StableNodeId) -> Option<&mut V> {
        let raw = id.get() as usize;
        if self.paged.run_slot(raw).is_some() {
            return self.paged.run_slot_mut(raw).and_then(|slot| slot.as_mut());
        }
        self.past_run_mut(id)
    }

    #[inline(never)]
    fn past_run_mut(&mut self, id: &StableNodeId) -> Option<&mut V> {
        match paged_index(*id) {
            Some(index) => self.paged.past_run_mut(index),
            None => self.overflow.get_mut(id),
        }
    }

    pub(crate) fn contains_key(&self, id: &StableNodeId) -> bool {
        self.get(id).is_some()
    }

    pub(crate) fn insert(&mut self, id: StableNodeId, value: V) -> Option<V> {
        match paged_index(id) {
            Some(index) => self.paged.insert(index, value),
            None => self.overflow.insert(id, value),
        }
    }

    pub(crate) fn remove(&mut self, id: &StableNodeId) -> Option<V> {
        match paged_index(*id) {
            Some(index) => self.paged.remove(index),
            None => self.overflow.remove(id),
        }
    }

    pub(crate) fn entry(&mut self, id: StableNodeId) -> DenseEntry<'_, V> {
        DenseEntry { map: self, id }
    }

    /// Room for every id up to `highest` without growing, for a table
    /// about to hold `entries` nodes: a table of every node of a document
    /// indexes ids from its first to the highest, which is more slots than
    /// nodes. Nothing is set aside when the ids are too far apart for one
    /// run of slots, or the table is on pages.
    pub(crate) fn reserve_through(&mut self, highest: StableNodeId, entries: usize) {
        if let Some(highest) = paged_index(highest) {
            self.paged.reserve_through(highest, entries);
        }
    }

    /// Move every entry of `from` into this map: the whole table when this
    /// one is empty (a first layout's retained cache), entry by entry
    /// otherwise.
    pub(crate) fn absorb(&mut self, from: &mut Self) {
        if self.is_empty() {
            std::mem::swap(self, from);
            from.clear();
            return;
        }
        self.extend(from.drain());
    }

    /// Every entry, leaving the map empty.
    pub(crate) fn drain(&mut self) -> <Self as IntoIterator>::IntoIter {
        std::mem::take(self).into_iter()
    }

    pub(crate) fn clear(&mut self) {
        self.paged.clear();
        self.overflow.clear();
    }

    pub(crate) fn iter(&self) -> DenseIter<'_, V> {
        DenseIter {
            paged: self.paged.iter(),
            overflow: self.overflow.iter(),
        }
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.iter().map(|(_, value)| value)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&StableNodeId, &mut V) -> bool) {
        self.paged
            .retain(|index, value| keep(&slot_id(index), value));
        self.overflow.retain(|id, value| keep(id, value));
    }

    /// The value at `id`, inserting what `make` gives when there is none;
    /// `None` when `make` gives nothing.
    pub(crate) fn get_or_try_insert<E>(
        &mut self,
        id: StableNodeId,
        make: impl FnOnce() -> Result<Option<V>, E>,
    ) -> Result<Option<&V>, E> {
        if self.get(&id).is_none() {
            let Some(value) = make()? else {
                return Ok(None);
            };
            self.insert(id, value);
        }
        Ok(self.get(&id))
    }
}

/// [`DenseNodeMap::entry`]: the subset of `hash_map::Entry` its users need.
pub(crate) struct DenseEntry<'a, V> {
    map: &'a mut DenseNodeMap<V>,
    id: StableNodeId,
}

impl<'a, V> DenseEntry<'a, V> {
    pub(crate) fn or_insert_with(self, make: impl FnOnce() -> V) -> &'a mut V {
        if !self.map.contains_key(&self.id) {
            self.map.insert(self.id, make());
        }
        self.map.get_mut(&self.id).expect("inserted above")
    }

    pub(crate) fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

impl<V> std::ops::Index<&StableNodeId> for DenseNodeMap<V> {
    type Output = V;

    fn index(&self, id: &StableNodeId) -> &V {
        self.get(id).expect("no entry for node")
    }
}

impl<V> Extend<(StableNodeId, V)> for DenseNodeMap<V> {
    fn extend<I: IntoIterator<Item = (StableNodeId, V)>>(&mut self, items: I) {
        for (id, value) in items {
            self.insert(id, value);
        }
    }
}

impl<V> FromIterator<(StableNodeId, V)> for DenseNodeMap<V> {
    fn from_iter<I: IntoIterator<Item = (StableNodeId, V)>>(items: I) -> Self {
        let mut map = Self::default();
        map.extend(items);
        map
    }
}

impl<V> IntoIterator for DenseNodeMap<V> {
    type Item = (StableNodeId, V);
    type IntoIter = std::iter::Chain<
        std::iter::Map<paged::SlotsIntoIter<V>, fn((usize, V)) -> (StableNodeId, V)>,
        <crate::NodeMap<V> as IntoIterator>::IntoIter,
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn entry<V>((index, value): (usize, V)) -> (StableNodeId, V) {
            (slot_id(index), value)
        }
        let entry: fn((usize, V)) -> (StableNodeId, V) = entry::<V>;
        self.paged.into_iter().map(entry).chain(self.overflow)
    }
}

/// [`DenseNodeMap::iter`]: each entry's id by value, as no slot holds it.
pub(crate) struct DenseIter<'a, V> {
    paged: paged::SlotsIter<'a, V>,
    overflow: <&'a crate::NodeMap<V> as IntoIterator>::IntoIter,
}

impl<'a, V> Iterator for DenseIter<'a, V> {
    type Item = (StableNodeId, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self.paged.next() {
            Some((index, value)) => Some((slot_id(index), value)),
            None => self.overflow.next().map(|(id, value)| (*id, value)),
        }
    }
}

impl<'a, V> IntoIterator for &'a DenseNodeMap<V> {
    type Item = (StableNodeId, &'a V);
    type IntoIter = DenseIter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The node records, by id: slots for ids below [`PAGED_IDS`]
/// ([`paged::Slots`]), a hash map for the rest. What it holds follows the nodes alive, not the
/// highest id the session has reached.
#[derive(Default)]
struct NodeTable {
    paged: paged::Slots<NodeRecord>,
    overflow: crate::NodeMap<NodeRecord>,
}

impl NodeTable {
    #[inline(always)]
    fn get(&self, id: StableNodeId) -> Option<&NodeRecord> {
        match self.paged.run_slot(id.get() as usize) {
            Some(slot) => slot.as_ref(),
            None => self.past_run(id),
        }
    }

    /// [`Self::get`] for an id the run does not reach.
    #[inline(never)]
    fn past_run(&self, id: StableNodeId) -> Option<&NodeRecord> {
        match paged_index(id) {
            Some(index) => self.paged.past_run(index),
            None => self.overflow.get(&id),
        }
    }

    #[inline(always)]
    fn get_mut(&mut self, id: StableNodeId) -> Option<&mut NodeRecord> {
        let raw = id.get() as usize;
        if self.paged.run_slot(raw).is_some() {
            return self.paged.run_slot_mut(raw).and_then(|slot| slot.as_mut());
        }
        self.past_run_mut(id)
    }

    #[inline(never)]
    fn past_run_mut(&mut self, id: StableNodeId) -> Option<&mut NodeRecord> {
        match paged_index(id) {
            Some(index) => self.paged.past_run_mut(index),
            None => self.overflow.get_mut(&id),
        }
    }

    fn contains(&self, id: StableNodeId) -> bool {
        self.get(id).is_some()
    }

    fn len(&self) -> usize {
        self.paged.len() + self.overflow.len()
    }

    fn insert(&mut self, id: StableNodeId, record: NodeRecord) {
        match paged_index(id) {
            Some(index) => self.paged.insert(index, record),
            None => self.overflow.insert(id, record),
        };
    }

    fn remove(&mut self, id: StableNodeId) -> Option<NodeRecord> {
        match paged_index(id) {
            Some(index) => self.paged.remove(index),
            None => self.overflow.remove(&id),
        }
    }

    fn records(&self) -> impl Iterator<Item = &NodeRecord> + '_ {
        self.paged
            .iter()
            .map(|(_, record)| record)
            .chain(self.overflow.values())
    }

    fn keys(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.paged
            .iter()
            .filter_map(|(index, _)| StableNodeId::new(index as u64))
            .chain(self.overflow.keys().copied())
    }
}

#[derive(Default)]
pub(crate) struct NodeStore {
    nodes: NodeTable,
    /// One shared kind per element tag ([`Self::intern_kind`]).
    kinds: HashMap<String, Arc<NodeKind>>,
    /// The style every new node starts with, one for all of them.
    default_style: Arc<NodeStyle>,
    visuals: HashMap<StableNodeId, StandardVisual>,
    /// Nodes whose space ([`NodeRecord::space`]) pins accessibility
    /// ([`crate::node_space::NodeSpace::pins_accessibility`]): their
    /// accessible bounds, or those of the boxes under them, may not keep
    /// their offset from their parent's when a box above them moves.
    space_pins: hashbrown::HashSet<StableNodeId>,
    /// Nodes whose space is page-bound
    /// ([`crate::node_space::NodeSpace::page_bound`]): an open select,
    /// whose menu a layout pass gives a new room ([`crate::SelectMenuRoom`]),
    /// or a visual whose geometry reads the page. The few a move above them
    /// looks at again for extraction.
    page_bound: hashbrown::HashSet<StableNodeId>,
    custom_render: HashMap<StableNodeId, CustomRenderNode>,
    event_listeners: HashMap<StableNodeId, EventListeners>,
    component_types: HashMap<StableNodeId, ComponentTypeId>,
    overlay_hosts: HashMap<StableNodeId, OverlayHostState>,
    overlay_mounts: HashMap<StableNodeId, nana_ui_core::OverlayMount>,
    scroll_metrics: HashMap<StableNodeId, ScrollMetrics>,
    /// Every text editor's session: its committed text, selections and IME
    /// composition, stored once (Issue #182).
    editors: HashMap<StableNodeId, EditorRecord>,
    highlights: HashMap<StableNodeId, HighlightRequest>,
    text_presentations: HashMap<StableNodeId, TextPresentation>,
    text_input_presentations: HashMap<StableNodeId, TextInputPresentation>,
    empty_state_text: HashMap<StableNodeId, EmptyStateTextPresentation>,
    modal_text: HashMap<StableNodeId, ModalTextPresentation>,
    /// 代码折叠视图状态（仅喂过折叠区间的编辑器节点持有条目）。
    text_fold_views: HashMap<StableNodeId, TextFoldViewState>,
    /// 行内提示（inlay）集（仅喂过非空 inlay 的编辑器节点持有条目；
    /// 世界校验后按 `(offset, label)` 排序去重）。
    text_inlays: HashMap<StableNodeId, Arc<[crate::TextInlay]>>,
    /// 活跃 snippet 会话（仅会话进行中的编辑器持有条目）。
    text_snippets: HashMap<StableNodeId, TextSnippetSession>,
    /// 补全弹层会话（仅候选会话活跃的编辑器持有条目）。
    text_completions: HashMap<StableNodeId, TextCompletionViewState>,
    /// hover 文档浮窗（仅宿主喂入 hover 的编辑器持有条目）。
    text_hovers: HashMap<StableNodeId, TextHoverViewState>,
    /// 签名帮助浮窗（仅宿主喂入签名的编辑器持有条目）。
    text_signatures: HashMap<StableNodeId, crate::TextSignatureHelp>,
    /// minimap 视口钉住（仅显式导航过的多行编辑器持有条目）。
    text_viewport_pins: HashMap<StableNodeId, ScrollOffset>,
    /// 拖拽移动选中文本的落点指示线（仅拖拽态编辑器持有条目；文本空间
    /// 矩形）。框架侧拖拽状态机写入，提取层翻译为节点空间图元。
    text_drop_indicators: HashMap<StableNodeId, LayoutBox>,
    /// The width a plain text that wrapped to its box would take on its
    /// own lines, unwrapped (its max-content width). Only text that wrapped
    /// holds an entry: its metrics are the lines it wrapped to, and layout
    /// needs this to give it more room once its box may grow.
    text_natural_widths: HashMap<StableNodeId, f32>,
    /// The strut an ordinary block's line boxes start from, per px of its
    /// font size: its own face's ascent and descent. Only a block with
    /// inline-level children and no line box of its own holds an entry, and
    /// only under a shaper that reports struts; see
    /// `UiWorld::resolve_block_struts`.
    block_struts: HashMap<StableNodeId, crate::FontStrut>,
    /// The width a plain text that wraps was last shaped against: its box
    /// width then, or infinity when it had no box yet. Text that does not
    /// wrap holds no entry. Layout trusts the lines' own width only within a
    /// box at least this narrow; see `text_inline_size`.
    text_wrap_limits: HashMap<StableNodeId, f32>,
    /// Languages nodes name for themselves (`lang`). Only those nodes hold an
    /// entry; their subtrees inherit through `ComputedStyle::language`.
    languages: HashMap<StableNodeId, nana_text::font::LanguageTag>,
    /// Typography scales nodes set for themselves. Only those nodes hold an
    /// entry; their subtrees inherit through `ComputedStyle::text_scale`.
    text_scales: HashMap<StableNodeId, f32>,
    /// Application-owned rich text (spans over the node's text), only for
    /// nodes given one with `SetRichText`.
    rich_texts: HashMap<StableNodeId, nana_ui_core::RichText>,
    /// Only mixed inline paragraphs pay for atomic participation metadata.
    inline_contents: HashMap<StableNodeId, crate::inline_content::InlineRecord>,
    inline_parents: HashMap<StableNodeId, StableNodeId>,
    /// Caret and selection of rich text editors.
    rich_editor_marks: HashMap<StableNodeId, crate::RichEditorMarks>,
    /// Per-glyph presentation of rich text nodes.
    glyph_presentations: HashMap<StableNodeId, nana_ui_core::GlyphPresentation>,
    /// Revisions, resolution stamp and retained layout handle of every node's
    /// text (Issue #95), one entry per node. Kept apart from the dense records
    /// on purpose: a large relayout scope decides "no text work" for every
    /// candidate, and a table of small entries keeps that scan in cache where
    /// the full records would not.
    ///
    /// Indexed by id like the records, not hashed: an entry per node in a
    /// hash table is every bucket touched (a quarter to a half of them empty)
    /// and, each time the table grows, a whole new table beside the old one,
    /// which the allocator then keeps.
    text_nodes: DenseNodeMap<crate::text_node::TextNodeState>,
    /// Layouts retained by plain text nodes, addressed by
    /// [`crate::text_node::TextNodeState::layout`]. Released with the node.
    text_layouts: nana_text::TextLayoutStore,
}

/// minimap 视口钉住：显式视口导航（minimap 点击/拖动）写入的滚动偏移。
/// 几何层在「钉住值 == 请求滚动」期间跳过光标 reveal（视口停在用户导航
/// 到的位置）；光标移动后由 shape 趟清除，reveal 恢复权威。
pub(crate) type TextViewportPin = ScrollOffset;

impl NodeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// `kind` as a record holds it: every element of one tag shares an
    /// `Arc`, so a page of a few tags holds a few kinds, not one per node.
    /// Past [`KINDS`] distinct tags, a new tag gets its own.
    /// The style a new node starts with, shared ([`NodeRecord::style`]).
    pub(crate) fn default_style(&self) -> Arc<NodeStyle> {
        Arc::clone(&self.default_style)
    }

    pub(crate) fn intern_kind(&mut self, kind: &NodeKind) -> Arc<NodeKind> {
        let NodeKind::Element { tag } = kind else {
            return intern_kind(kind);
        };
        if let Some(kept) = self.kinds.get(tag.as_str()) {
            return Arc::clone(kept);
        }
        let kept = intern_kind(kind);
        if self.kinds.len() < KINDS {
            self.kinds.insert(tag.clone(), Arc::clone(&kept));
        }
        kept
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 0
    }

    pub fn contains(&self, id: StableNodeId) -> bool {
        self.nodes.contains(id)
    }

    pub fn get(&self, id: StableNodeId) -> Option<&NodeRecord> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: StableNodeId) -> Option<&mut NodeRecord> {
        self.nodes.get_mut(id)
    }

    pub fn keys(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.nodes.keys()
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &NodeRecord> + '_ {
        self.nodes.records()
    }

    /// 持有 hover 浮窗状态的节点（稀疏表直接迭代，命中测试路由用）。
    pub fn text_hover_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.text_hovers.keys().copied()
    }

    pub fn insert(&mut self, id: StableNodeId, record: NodeRecord) {
        self.text_nodes
            .insert(id, crate::text_node::TextNodeState::default());
        self.nodes.insert(id, record);
        self.note_node_space(id);
    }

    /// Bring `id`'s space ([`NodeRecord::space`]) up to date with its
    /// record and visual, and the indexes of it ([`Self::space_pins`],
    /// [`Self::page_bound`]). Called wherever a field that decides it is
    /// written: style resolution, the layout frame, the visual, an insert.
    pub(crate) fn note_node_space(&mut self, id: StableNodeId) {
        let visual = self.visuals.get(&id);
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        let space = crate::node_space::NodeSpace::of(
            &record.resolved_layout,
            record.layout_frame,
            &record.kind,
            visual,
        );
        record.space = space;
        if space.pins_accessibility() {
            self.space_pins.insert(id);
        } else {
            self.space_pins.remove(&id);
        }
        if space.page_bound() {
            self.page_bound.insert(id);
        } else {
            self.page_bound.remove(&id);
        }
    }

    /// Every node whose space pins accessibility
    /// ([`crate::node_space::NodeSpace::pins_accessibility`]), in no
    /// particular order.
    pub(crate) fn space_pins(&self) -> impl ExactSizeIterator<Item = StableNodeId> + '_ {
        self.space_pins.iter().copied()
    }

    /// Every node whose space is page-bound
    /// ([`crate::node_space::NodeSpace::page_bound`]), in no particular
    /// order.
    pub(crate) fn page_bound(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.page_bound.iter().copied()
    }

    /// Whether any node is page-bound ([`Self::page_bound`]).
    pub(crate) fn has_page_bound(&self) -> bool {
        !self.page_bound.is_empty()
    }

    /// Nodes whose visual is an open Select, Dropdown or SearchDropdown
    /// menu ([`crate::node_space::NodeSpace::MENU_ROOM`]), in no particular
    /// order.
    pub(crate) fn open_select_menus(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.page_bound().filter(|&id| {
            self.nodes.get(id).is_some_and(|record| {
                record
                    .space
                    .contains(crate::node_space::NodeSpace::MENU_ROOM)
            })
        })
    }

    /// The space of every node is what its record and visual give, and the
    /// indexes hold exactly the nodes they should. Small worlds only.
    #[cfg(any(test, feature = "layout-verify"))]
    pub(crate) fn check_node_spaces(&self) {
        if self.len() > 4096 {
            return;
        }
        let (mut pins, mut bound) = (0, 0);
        for id in self.keys() {
            let record = self.nodes.get(id).expect("a key has a record");
            let space = crate::node_space::NodeSpace::of(
                &record.resolved_layout,
                record.layout_frame,
                &record.kind,
                self.visuals.get(&id),
            );
            assert_eq!(record.space, space, "{id:?}: its space is stale");
            assert_eq!(
                space.pins_accessibility(),
                self.space_pins.contains(&id),
                "{id:?}: the pins disagree with its space {space:?}"
            );
            assert_eq!(
                space.page_bound(),
                self.page_bound.contains(&id),
                "{id:?}: the page-bound nodes disagree with its space {space:?}"
            );
            pins += usize::from(space.pins_accessibility());
            bound += usize::from(space.page_bound());
        }
        assert_eq!(pins, self.space_pins.len(), "pins of nodes that are gone");
        assert_eq!(
            bound,
            self.page_bound.len(),
            "page-bound nodes that are gone"
        );
    }

    /// Drop the dense record and every sparse entry. The only despawn path.
    pub fn remove(&mut self, id: StableNodeId) -> Option<NodeRecord> {
        let record = self.nodes.remove(id)?;
        self.visuals.remove(&id);
        self.space_pins.remove(&id);
        self.page_bound.remove(&id);
        self.custom_render.remove(&id);
        self.event_listeners.remove(&id);
        self.component_types.remove(&id);
        self.overlay_hosts.remove(&id);
        self.overlay_mounts.remove(&id);
        self.scroll_metrics.remove(&id);
        self.editors.remove(&id);
        self.highlights.remove(&id);
        self.text_presentations.remove(&id);
        self.text_input_presentations.remove(&id);
        self.empty_state_text.remove(&id);
        self.modal_text.remove(&id);
        self.text_fold_views.remove(&id);
        self.text_inlays.remove(&id);
        self.text_snippets.remove(&id);
        self.text_completions.remove(&id);
        self.text_hovers.remove(&id);
        self.text_signatures.remove(&id);
        self.text_viewport_pins.remove(&id);
        self.text_drop_indicators.remove(&id);
        self.text_natural_widths.remove(&id);
        self.block_struts.remove(&id);
        self.text_wrap_limits.remove(&id);
        self.languages.remove(&id);
        self.text_scales.remove(&id);
        self.rich_texts.remove(&id);
        self.set_inline_atoms(id, Arc::from([]));
        self.inline_parents.remove(&id);
        self.rich_editor_marks.remove(&id);
        self.glyph_presentations.remove(&id);
        if let Some(text) = self.text_nodes.remove(&id) {
            self.text_layouts.remove(text.layout);
        }
        Some(record)
    }

    pub fn visual(&self, id: StableNodeId) -> Option<&StandardVisual> {
        self.visuals.get(&id)
    }

    pub(crate) fn has_visuals(&self) -> bool {
        !self.visuals.is_empty()
    }

    pub(crate) fn has_text_inputs(&self) -> bool {
        !self.editors.is_empty()
    }

    pub(crate) fn has_custom_render(&self) -> bool {
        !self.custom_render.is_empty()
    }

    /// Sets a node's visual. A visual change that moves what plain text
    /// resolution reads — which path the text takes, or the inset a leading
    /// indicator takes from its box — is a constraint change for its text;
    /// any other visual change (a sampled progress value) is not.
    pub fn set_visual(&mut self, id: StableNodeId, value: Option<StandardVisual>) {
        let before = crate::world::text_visual_key(self.visuals.get(&id));
        let after = crate::world::text_visual_key(value.as_ref());
        if let Some(value) = value {
            self.visuals.insert(id, value);
        } else {
            self.visuals.remove(&id);
        }
        if before != after {
            self.invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
        }
        self.note_node_space(id);
    }
    sparse!(
        custom_render,
        CustomRenderNode,
        custom_render,
        set_custom_render
    );
    sparse!(
        event_listeners,
        EventListeners,
        event_listeners,
        set_event_listeners
    );
    sparse!(
        component_types,
        ComponentTypeId,
        component_type,
        set_component_type
    );
    sparse!(
        overlay_hosts,
        OverlayHostState,
        overlay_host,
        set_overlay_host
    );
    pub(crate) fn has_overlay_mounts(&self) -> bool {
        !self.overlay_mounts.is_empty()
    }

    sparse!(
        overlay_mounts,
        nana_ui_core::OverlayMount,
        overlay_mount,
        set_overlay_mount
    );
    sparse!(
        scroll_metrics,
        ScrollMetrics,
        scroll_metrics,
        set_scroll_metrics
    );
    pub fn text_input(&self, id: StableNodeId) -> Option<TextInputView<'_>> {
        self.editors
            .get(&id)
            .map(|editor| TextInputView::of(&editor.session))
    }

    pub fn ime(&self, id: StableNodeId) -> Option<ImeView<'_>> {
        self.editors.get(&id)?.ime()
    }

    pub(crate) fn editor(&self, id: StableNodeId) -> Option<&EditorRecord> {
        self.editors.get(&id)
    }

    pub(crate) fn editor_mut(&mut self, id: StableNodeId) -> Option<&mut EditorRecord> {
        self.editors.get_mut(&id)
    }

    pub(crate) fn set_editor(&mut self, id: StableNodeId, editor: Option<EditorRecord>) {
        match editor {
            Some(editor) => {
                self.editors.insert(id, editor);
            }
            None => {
                self.editors.remove(&id);
            }
        }
    }
    sparse!(highlights, HighlightRequest, highlight, set_highlight);
    sparse!(
        text_presentations,
        TextPresentation,
        text_presentation,
        set_text_presentation
    );
    sparse!(
        text_input_presentations,
        TextInputPresentation,
        text_input_presentation,
        set_text_input_presentation
    );
    sparse!(
        empty_state_text,
        EmptyStateTextPresentation,
        empty_state_text,
        set_empty_state_text
    );
    sparse!(
        modal_text,
        ModalTextPresentation,
        modal_text,
        set_modal_text
    );
    sparse!(
        text_fold_views,
        TextFoldViewState,
        text_fold_view,
        set_text_fold_view
    );
    sparse!(
        text_inlays,
        Arc<[crate::TextInlay]>,
        text_inlays,
        set_text_inlays
    );
    sparse!(
        text_snippets,
        TextSnippetSession,
        text_snippet_session,
        set_text_snippet_session
    );
    sparse!(
        text_completions,
        TextCompletionViewState,
        text_completion_view,
        set_text_completion_view
    );
    sparse!(
        text_hovers,
        TextHoverViewState,
        text_hover_view,
        set_text_hover_view
    );
    sparse!(
        text_signatures,
        crate::TextSignatureHelp,
        text_signature,
        set_text_signature
    );
    sparse!(
        text_viewport_pins,
        ScrollOffset,
        text_viewport_pin,
        set_text_viewport_pin
    );
    sparse!(
        text_drop_indicators,
        LayoutBox,
        text_drop_indicator,
        set_text_drop_indicator
    );
    sparse!(
        text_natural_widths,
        f32,
        text_natural_width,
        set_text_natural_width
    );
    sparse!(text_wrap_limits, f32, text_wrap_limit, set_text_wrap_limit);
    sparse!(block_struts, crate::FontStrut, block_strut, set_block_strut);

    /// Every block holding a strut: what a new font set can change.
    pub(crate) fn strut_holders(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.block_struts.keys().copied()
    }
    sparse!(
        languages,
        nana_text::font::LanguageTag,
        language,
        set_language
    );
    sparse!(text_scales, f32, text_scale, set_text_scale);
    sparse!(rich_texts, nana_ui_core::RichText, rich_text, set_rich_text);
    sparse!(
        glyph_presentations,
        nana_ui_core::GlyphPresentation,
        glyph_presentation,
        set_glyph_presentation
    );
    sparse!(
        rich_editor_marks,
        crate::RichEditorMarks,
        rich_editor_marks,
        set_rich_editor_marks
    );

    pub(crate) fn text_layouts(&self) -> &nana_text::TextLayoutStore {
        &self.text_layouts
    }

    pub(crate) fn text_node(&self, id: StableNodeId) -> Option<&crate::text_node::TextNodeState> {
        self.text_nodes.get(&id)
    }
    pub(crate) fn note_text_fragments(&mut self, id: StableNodeId) {
        if let Some(text) = self.text_nodes.get_mut(&id) {
            text.fragments_layout = text.layout;
        }
    }

    /// Records a change to `id`'s text. See [`crate::text_node::TextDirty`].
    pub(crate) fn invalidate_text(&mut self, id: StableNodeId, dirty: crate::text_node::TextDirty) {
        if let Some(text) = self.text_nodes.get_mut(&id) {
            text.invalidate(dirty);
        }
    }

    /// Notes that a typography scale change reached `id`'s text; see
    /// [`crate::text_node::TextNodeState::note_scale`].
    pub(crate) fn note_text_scale(&mut self, id: StableNodeId) -> bool {
        self.text_nodes
            .get_mut(&id)
            .is_some_and(crate::text_node::TextNodeState::note_scale)
    }

    /// Whether a typography scale change reached `id`'s text since it last
    /// resolved, clearing it.
    pub(crate) fn take_text_scale_pending(&mut self, id: StableNodeId) -> bool {
        self.text_nodes
            .get_mut(&id)
            .is_some_and(crate::text_node::TextNodeState::take_scale_pending)
    }

    /// Stamps `id`'s text as resolved at its current revisions. Returns
    /// whether a language change had invalidated it in vain; see
    /// [`crate::text_node::TextNodeState::mark_resolved`].
    pub(crate) fn mark_text_resolved(
        &mut self,
        id: StableNodeId,
        backend: crate::text_node::TextBackendEpoch,
        constraints: Option<crate::TextShapeConstraints>,
    ) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let text_node = !record.text.value.is_empty()
            || matches!(record.kind.as_ref(), crate::world::NodeKind::Text);
        let constraints =
            constraints.map(|constraints| (constraints, record.style.text_horizontal_alignment));
        let language = record.resolved.0.language.clone();
        self.text_nodes.get_mut(&id).is_some_and(|text| {
            text.mark_resolved(backend, constraints, text_node, language.as_ref())
        })
    }

    /// The `nana-text` source for `id`'s current text, built once per content
    /// revision, and whether this call built it.
    pub(crate) fn text_source(
        &mut self,
        id: StableNodeId,
        style: &crate::ComputedStyle,
    ) -> Option<(&nana_text::TextSource, bool)> {
        let text = &self.nodes.get(id)?.text;
        let rich = self.rich_texts.get(&id);
        Some(
            self.text_nodes.get_mut(&id)?.source_for_objects(
                &text.value,
                rich,
                style,
                self.inline_contents
                    .get(&id)
                    .map_or(&[], |inline| inline.objects.as_ref()),
            ),
        )
    }

    pub(crate) fn inline_record(
        &self,
        id: StableNodeId,
    ) -> Option<&crate::inline_content::InlineRecord> {
        self.inline_contents.get(&id)
    }
    pub(crate) fn has_inline_content(&self) -> bool {
        !self.inline_contents.is_empty()
    }
    pub(crate) fn inline_parent(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.inline_parents.get(&id).copied()
    }
    pub(crate) fn set_inline_atoms(
        &mut self,
        id: StableNodeId,
        atoms: Arc<[crate::InlineAtomic]>,
    ) -> bool {
        if self
            .inline_contents
            .get(&id)
            .is_some_and(|previous| previous.atoms == atoms)
            || (atoms.is_empty() && !self.inline_contents.contains_key(&id))
        {
            return false;
        }
        if let Some(previous) = self.inline_contents.get_mut(&id)
            && previous.atoms.len() == atoms.len()
            && previous
                .atoms
                .iter()
                .zip(atoms.iter())
                .all(|(a, b)| (a.offset, a.node) == (b.offset, b.node))
        {
            previous.atoms = atoms;
            return false;
        }
        let previous = self.inline_contents.remove(&id).unwrap_or_default();
        let old: HashMap<_, _> = previous
            .atoms
            .iter()
            .zip(previous.objects.iter())
            .map(|(atom, object)| (atom.node, object.metrics))
            .collect();
        for atom in previous.atoms.iter() {
            if self.inline_parents.get(&atom.node) == Some(&id) {
                self.inline_parents.remove(&atom.node);
            }
        }
        if atoms.is_empty() {
            if self.inline_contents.is_empty() {
                self.inline_contents = HashMap::new();
                // A destination already installed earlier in this batch keeps
                // its reverse binding when an old owner is cleared.
                if self.inline_parents.is_empty() {
                    self.inline_parents = HashMap::new();
                }
            }
            return true;
        }
        let objects = atoms
            .iter()
            .map(|atom| {
                self.inline_parents.insert(atom.node, id);
                nana_text::InlineObject {
                    offset: atom.offset,
                    id: atom.node.get(),
                    metrics: old.get(&atom.node).copied().unwrap_or(
                        nana_text::InlineObjectMetrics {
                            width_px: 0.0,
                            ascent_px: 0.0,
                            descent_px: 0.0,
                            envelope: None,
                        },
                    ),
                }
            })
            .collect::<Vec<_>>()
            .into();
        self.inline_contents
            .insert(id, crate::inline_content::InlineRecord { atoms, objects });
        true
    }
    pub(crate) fn set_inline_objects(
        &mut self,
        id: StableNodeId,
        objects: Arc<[nana_text::InlineObject]>,
    ) -> bool {
        let Some(inline) = self.inline_contents.get_mut(&id) else {
            return false;
        };
        if inline.objects == objects {
            return false;
        }
        inline.objects = objects;
        true
    }

    /// Retains `layout` for `id`, replacing what the node held. A node handed
    /// the very layout it already holds keeps its handle. Returns whether the
    /// handle was kept.
    pub(crate) fn retain_text_layout(
        &mut self,
        id: StableNodeId,
        layout: Arc<nana_text::TextLayout>,
    ) -> bool {
        let Some(text) = self.text_nodes.get_mut(&id) else {
            return false;
        };
        if self
            .text_layouts
            .get(text.layout)
            .is_some_and(|current| Arc::ptr_eq(current, &layout))
        {
            return true;
        }
        self.text_layouts.remove(text.layout);
        text.layout = self.text_layouts.insert(layout);
        false
    }

    /// Drops the layout `id` retains, if any. O(1) when it holds none.
    pub(crate) fn release_text_layout(&mut self, id: StableNodeId) {
        if let Some(text) = self.text_nodes.get_mut(&id)
            && !text.layout.is_null()
        {
            let held = std::mem::take(&mut text.layout);
            self.text_layouts.remove(held);
            // The source copy served only the released layout.
            text.release_source();
        }
    }

    /// Every node whose text a new font set can change: text that was measured
    /// (an empty Text node's line box included), plus the visuals that measure text of their own every pass
    /// and so are never stamped.
    pub(crate) fn text_bearing_nodes(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        let resolved = self
            .text_nodes
            .iter()
            .filter(|(_, text)| text.stamp().is_some_and(|stamp| stamp.measured))
            .map(|(id, _)| id);
        let measuring = self
            .visuals
            .iter()
            .filter(|(_, visual)| measures_text_each_pass(visual))
            .map(|(id, _)| *id);
        resolved.chain(measuring)
    }

    pub(crate) fn text_completion_view_mut(
        &mut self,
        id: StableNodeId,
    ) -> Option<&mut TextCompletionViewState> {
        self.text_completions.get_mut(&id)
    }

    pub(crate) fn text_hover_view_mut(
        &mut self,
        id: StableNodeId,
    ) -> Option<&mut TextHoverViewState> {
        self.text_hovers.get_mut(&id)
    }

    pub(crate) fn diagnostic_hover_ids(&self) -> Vec<StableNodeId> {
        self.text_hovers
            .iter()
            .filter(|(_, state)| state.diagnostic)
            .map(|(id, _)| *id)
            .collect()
    }
}

/// Visuals that measure text of their own every pass — intrinsic text, or
/// chrome sized to the text painted into it — and so are never stamped.
fn measures_text_each_pass(visual: &StandardVisual) -> bool {
    match visual {
        StandardVisual::EmptyState { .. }
        | StandardVisual::ModalFrame { .. }
        | StandardVisual::TextInput { .. }
        | StandardVisual::KeyCaptureLayer { .. }
        | StandardVisual::KeymapLayer
        | StandardVisual::ListItem { .. }
        | StandardVisual::PathText { .. }
        | StandardVisual::LabeledValue { .. }
        | StandardVisual::ActionMenuItem { .. }
        | StandardVisual::CommandPalette { .. } => true,
        #[cfg(feature = "calendar")]
        StandardVisual::CalendarHeatmap { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InteractionState;
    use crate::world::{DocumentId, NodeKind, StableNodeId};

    fn node(id: u64) -> StableNodeId {
        StableNodeId::new(id).unwrap()
    }

    fn document(id: u64) -> DocumentId {
        DocumentId::new(id).unwrap()
    }

    #[test]
    fn remove_clears_sparse_side_maps() {
        let mut store = NodeStore::new();
        let id = node(2);
        store.insert(
            id,
            NodeRecord::new(
                document(1),
                &NodeKind::Text,
                InteractionState {
                    pointer_events: false,
                    focusable: false,
                },
            ),
        );
        store.set_scroll_metrics(
            id,
            Some(ScrollMetrics {
                viewport_width: 10.0,
                viewport_height: 10.0,
                content_width: 20.0,
                content_height: 20.0,
                origin_x: 0.0,
                origin_y: 0.0,
            }),
        );
        store.set_component_type(id, Some(ComponentTypeId::new("nana.button").unwrap()));
        store.set_editor(
            id,
            Some(EditorRecord {
                empty_preedit: Some(None),
                ..EditorRecord::new(nana_text::EditSession::new("a"))
            }),
        );
        assert!(store.scroll_metrics(id).is_some());
        assert!(store.remove(id).is_some());
        assert!(!store.contains(id));
        assert!(store.scroll_metrics(id).is_none());
        assert!(store.component_type(id).is_none());
        assert!(store.ime(id).is_none());
    }

    #[test]
    fn sparse_set_none_removes_entry() {
        let mut store = NodeStore::new();
        let id = node(3);
        store.insert(
            id,
            NodeRecord::new(
                document(1),
                &NodeKind::Element { tag: "div".into() },
                InteractionState::default(),
            ),
        );
        store.set_overlay_host(id, Some(OverlayHostState::default()));
        assert!(store.overlay_host(id).is_some());
        store.set_overlay_host(id, None);
        assert!(store.overlay_host(id).is_none());
        assert!(store.contains(id));
    }

    #[test]
    fn dense_ids_and_overflow_ids_round_trip() {
        let mut store = NodeStore::new();
        let record = || {
            NodeRecord::new(
                document(1),
                &NodeKind::Text,
                InteractionState {
                    pointer_events: true,
                    focusable: false,
                },
            )
        };
        store.insert(node(1), record());
        store.insert(node(2), record());
        // A hole large enough to stay out of the contiguous table.
        store.insert(node(20_000), record());
        store.insert(node(80_000), record());
        assert_eq!(store.len(), 4);
        assert!(store.contains(node(1)));
        assert!(store.contains(node(20_000)));
        assert!(store.contains(node(80_000)));
        assert!(!store.contains(node(3)));
        let mut keys = store.keys().map(|id| id.get()).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, vec![1, 2, 20_000, 80_000]);
        assert!(store.remove(node(20_000)).is_some());
        assert!(store.remove(node(1)).is_some());
        assert_eq!(store.len(), 2);
        store.insert(node(1), record());
        assert!(store.get(node(1)).is_some());
        assert!(store.get(node(80_000)).is_some());
        assert_eq!(store.len(), 3);
    }
}
