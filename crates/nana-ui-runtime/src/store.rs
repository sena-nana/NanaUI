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
    ScrollMetrics, ScrollOffset, TextCodeFold, TextContent, TextInputPresentation, TextInputView,
    TextMetrics, TextSnippetSession,
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
pub(crate) static EMPTY_LIST_ITEM_VISUAL: LazyLock<crate::WidgetVisualHandle> =
    LazyLock::new(|| crate::WidgetVisualHandle::new(crate::ListItemVisual::default()));

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
}

impl Default for Hierarchy {
    fn default() -> Self {
        Self {
            parent: None,
            children: Arc::clone(&EMPTY_CHILDREN),
            index_in_parent: 0,
            children_base: 0,
        }
    }
}

/// `(own, palette_epoch, inherited_unpainted)`: `.0` holds the node's own
/// resolved style (with any local background/border/ring paint); `.2` strips
/// non-inherited local box paint so descendants inside painted containers can
/// share it directly via `Arc::clone`.
#[derive(Clone)]
pub(crate) struct ResolvedStyle(pub Arc<ComputedStyle>, pub u32, pub Arc<ComputedStyle>);

const _: () = assert!(std::mem::size_of::<Hierarchy>() == 24);

impl ResolvedStyle {
    fn interned_default() -> Self {
        Self(
            Arc::clone(&INTERNED_DEFAULT_STYLE),
            0,
            Arc::clone(&INTERNED_DEFAULT_STYLE),
        )
    }
}

/// The process-wide default a root node inherits from. Sharing this avoids
/// building a fresh `ComputedStyle` per root during style resolution.
pub(crate) fn interned_default_style() -> Arc<ComputedStyle> {
    Arc::clone(&INTERNED_DEFAULT_STYLE)
}

/// Compact inline state of a container's far-start axes (`Option<[bool; 2]>`).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum LayoutFarStart {
    #[default]
    Unset = 0,
    FalseFalse = 1,
    TrueFalse = 2,
    FalseTrue = 3,
    TrueTrue = 4,
}

impl LayoutFarStart {
    #[inline]
    fn from_pair(far: [bool; 2]) -> Self {
        match far {
            [false, false] => Self::FalseFalse,
            [true, false] => Self::TrueFalse,
            [false, true] => Self::FalseTrue,
            [true, true] => Self::TrueTrue,
        }
    }

    #[inline]
    fn to_option(self) -> Option<[bool; 2]> {
        match self {
            Self::Unset => None,
            Self::FalseFalse => Some([false, false]),
            Self::TrueFalse => Some([true, false]),
            Self::FalseTrue => Some([false, true]),
            Self::TrueTrue => Some([true, true]),
        }
    }
}

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
    /// Whether [`NodeStore::text_content`] holds non-empty text for this node.
    pub has_text: bool,
    /// 1-based index into `BUILTIN_COMPONENT_IDS`, or `0` for none/extension.
    pub builtin_component: u8,
    /// 1-based index into `NodeStore::empty_text_backends` for empty non-`Text` nodes.
    pub empty_text_backend: u8,
    /// `0` = unset, `1` = `PaddingSpec::default()`, `2..=254` = distinct table slot, `255` = sparse map.
    pub layout_padding_slot: u8,
    /// Inline accessibility preset:
    /// `0` = default, `1` = plain `Text`, `2` = decorative `Text`, `3` = sparse map,
    /// `4` = plain `Button` (label = node text),
    /// `5` = plain unselected `ListItem` (label = node text),
    /// `6` = plain selected `ListItem` (label = node text).
    pub a11y_preset: u8,
    /// Non-zero when a `Stack` component view was elided from `AppContext::views`.
    pub elided_stack_flags: u8,
    /// Inline widget visual preset (`0` = none/map, `1` = shared default [`ListItemVisual`]).
    pub visual_preset: u8,
    pub layout_far_start: LayoutFarStart,
    /// Inline formatting-context change counter (`255` spills to `layout_context_generations`).
    pub layout_context_gen: u8,
    /// Positional auto-view slot (`#v{slot}`) when declared under its world parent; `u16::MAX` = none.
    pub auto_view_slot: u16,
    /// Levels from the root down to this node, counting it: 1 for a root.
    /// Moving a subtree renumbers it (`UiWorld::set_subtree_depth`), so
    /// checking an insert against `MAX_TREE_DEPTH` reads its parent's depth
    /// instead of walking to the root.
    pub depth: u16,
    /// The box layout last wrote, measured from [`Self::layout_frame`]'s
    /// origin. Its document box is derived ([`crate::UiWorld::layout_box`]):
    /// a subtree that moves keeps every box under its root.
    pub local_layout: LayoutBox,
    layout_frame: PackedLayoutFrame,
    /// How this node stands to the coordinate spaces around it, from
    /// [`Self::resolved_layout`], [`Self::layout_frame`], its kind and its
    /// visual ([`crate::node_space::NodeSpace::of`]). Kept up to date by
    /// [`NodeStore::note_node_space`] wherever one of those is written.
    pub space: crate::node_space::NodeSpace,
    /// The formatting context the last layout ran for the children.
    pub layout_context: Option<crate::FormattingContextKind>,
    pub interaction: InteractionState,
    pub dirty: DirtyMask,
    /// Why the node is marked for render extraction, in the world's
    /// extraction epochs: the last epoch anything but a move marked it
    /// (or it was made), and the last one a layout move marked it. Only a
    /// node a move marked and nothing else did since the epoch began may
    /// reuse its last extracted record.
    pub render_full_epoch: u32,
    pub render_moved_epoch: u32,
}

/// Compact 8-byte encoding of [`crate::LayoutFrame`].
///
/// `0` encodes [`crate::LayoutFrame::Parent`], `u64::MAX` encodes
/// [`crate::LayoutFrame::Page`], and any non-zero `n` encodes
/// [`crate::LayoutFrame::Block`] for `StableNodeId(n)`.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct PackedLayoutFrame(u64);

impl PackedLayoutFrame {
    #[inline]
    const fn from_frame(frame: crate::LayoutFrame) -> Self {
        match frame {
            crate::LayoutFrame::Parent => Self(0),
            crate::LayoutFrame::Page => Self(u64::MAX),
            crate::LayoutFrame::Block(id) => {
                debug_assert!(id.get() != u64::MAX);
                Self(id.get())
            }
        }
    }

    #[inline]
    const fn get(self) -> crate::LayoutFrame {
        match self.0 {
            0 => crate::LayoutFrame::Parent,
            u64::MAX => crate::LayoutFrame::Page,
            n => match StableNodeId::new(n) {
                Some(id) => crate::LayoutFrame::Block(id),
                None => crate::LayoutFrame::Parent,
            },
        }
    }
}

const _: () = assert!(std::mem::size_of::<NodeRecord>() <= 152);

static DEFAULT_ACCESSIBILITY: std::sync::LazyLock<Arc<AccessibilityState>> =
    std::sync::LazyLock::new(|| Arc::new(AccessibilityState::default()));
static TEXT_ACCESSIBILITY: std::sync::LazyLock<Arc<AccessibilityState>> = LazyLock::new(|| {
    Arc::new(AccessibilityState {
        role: crate::AccessibilityRole::Text,
        hidden: false,
        ..AccessibilityState::default()
    })
});
static DECORATIVE_TEXT_ACCESSIBILITY: std::sync::LazyLock<Arc<AccessibilityState>> =
    LazyLock::new(|| {
        Arc::new(AccessibilityState {
            role: crate::AccessibilityRole::Text,
            hidden: true,
            ..AccessibilityState::default()
        })
    });
static DEFAULT_TEXT_CONTENT: std::sync::LazyLock<TextContent> =
    std::sync::LazyLock::new(TextContent::default);

/// `state` as a record holds it: the one shared default when it is the
/// default, its own allocation otherwise.
pub(crate) fn shared_accessibility(state: &AccessibilityState) -> Arc<AccessibilityState> {
    if *state == **DEFAULT_ACCESSIBILITY {
        Arc::clone(&DEFAULT_ACCESSIBILITY)
    } else if *state == **TEXT_ACCESSIBILITY {
        Arc::clone(&TEXT_ACCESSIBILITY)
    } else if *state == **DECORATIVE_TEXT_ACCESSIBILITY {
        Arc::clone(&DECORATIVE_TEXT_ACCESSIBILITY)
    } else {
        Arc::new(state.clone())
    }
}

fn preset_control_accessibility(preset: u8, label: Arc<str>) -> AccessibilityState {
    match preset {
        4 => AccessibilityState {
            role: crate::AccessibilityRole::Button,
            label: Some(label),
            ..AccessibilityState::default()
        },
        5 => AccessibilityState {
            role: crate::AccessibilityRole::ListItem,
            label: Some(label),
            selected: Some(false),
            ..AccessibilityState::default()
        },
        6 => AccessibilityState {
            role: crate::AccessibilityRole::ListItem,
            label: Some(label),
            selected: Some(true),
            ..AccessibilityState::default()
        },
        _ => AccessibilityState::default(),
    }
}

fn matching_control_preset(state: &AccessibilityState, node_text: &str) -> Option<u8> {
    let label = state.label.as_ref()?;
    if label.as_ref() != node_text {
        return None;
    }
    let preset = match (state.role, state.selected) {
        (crate::AccessibilityRole::Button, None) => 4,
        (crate::AccessibilityRole::ListItem, Some(false)) => 5,
        (crate::AccessibilityRole::ListItem, Some(true)) => 6,
        _ => return None,
    };
    let candidate = preset_control_accessibility(preset, Arc::clone(label));
    (*state == candidate).then_some(preset)
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
            has_text: false,
            builtin_component: 0,
            empty_text_backend: 0,
            layout_padding_slot: 0,
            a11y_preset: 0,
            elided_stack_flags: 0,
            visual_preset: 0,
            layout_far_start: LayoutFarStart::Unset,
            layout_context_gen: 0,
            auto_view_slot: u16::MAX,
            depth: 1,
            local_layout: LayoutBox::default(),
            layout_frame: PackedLayoutFrame::from_frame(crate::LayoutFrame::Parent),
            space,
            layout_context: None,
            interaction,
            dirty: DirtyMask::all(),
            render_full_epoch: 0,
            render_moved_epoch: u32::MAX,
        }
    }

    #[inline]
    pub(crate) const fn layout_frame(&self) -> crate::LayoutFrame {
        self.layout_frame.get()
    }

    #[inline]
    pub(crate) fn set_layout_frame(&mut self, frame: crate::LayoutFrame) {
        self.layout_frame = PackedLayoutFrame::from_frame(frame);
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
        if let Some(highest) = from.iter().map(|(id, _)| id).max() {
            self.reserve_through(highest, self.len() + from.len());
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
    pub(crate) fn keys(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.iter().map(|(id, _)| id)
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

    fn reserve_through(&mut self, highest: StableNodeId, entries: usize) {
        if let Some(highest) = paged_index(highest) {
            self.paged.reserve_through(highest, entries);
        }
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

    fn entries(&self) -> impl Iterator<Item = (StableNodeId, &NodeRecord)> + '_ {
        self.paged
            .iter()
            .filter_map(|(index, record)| StableNodeId::new(index as u64).map(|id| (id, record)))
            .chain(self.overflow.iter().map(|(&id, record)| (id, record)))
    }
}

#[derive(Default)]
pub(crate) struct NodeStore {
    nodes: NodeTable,
    /// One shared kind per element tag ([`Self::intern_kind`]).
    kinds: HashMap<String, Arc<NodeKind>>,
    /// The style every new node starts with, one for all of them.
    default_style: Arc<NodeStyle>,
    widget_visuals: crate::NodeMap<crate::WidgetVisualHandle>,
    /// Number of live nodes using an inline widget-visual preset (`visual_preset != 0`).
    preset_visual_count: usize,
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
    /// Cold optional text content; absent means empty unstamped text.
    texts: crate::NodeMap<TextContent>,
    /// Cold shaped text metrics; absent means `TextMetrics::default()`.
    text_metrics: crate::NodeMap<TextMetrics>,
    /// Distinct non-zero layout padding specs (`layout_padding_slot` 2..=254).
    distinct_layout_paddings: Vec<nana_ui_core::PaddingSpec>,
    /// Overflow layout paddings beyond `distinct_layout_paddings` capacity.
    layout_paddings: crate::NodeMap<nana_ui_core::PaddingSpec>,
    /// Bumped each time `layout_context` changes on a container (overflow beyond `u8::MAX - 1`).
    layout_context_generations: HashMap<StableNodeId, u64>,
    /// Distinct text backend epochs observed on empty non-`Text` nodes (indexed by `empty_text_backend`).
    empty_text_backends: Vec<crate::text_node::TextBackendEpoch>,
    /// Non-zero scroll offsets; absent means `(0.0, 0.0)`.
    scroll_offsets: HashMap<StableNodeId, ScrollOffset>,
    /// Non-preset accessibility states (`a11y_preset == 3`).
    accessibilities: crate::NodeMap<Arc<AccessibilityState>>,
    /// Number of live nodes using inline decorative-text preset (`a11y_preset == 2`).
    decorative_a11y_count: usize,
    /// Epoch a move above an open select field asked if its room stayed valid; absent means `u32::MAX`.
    render_room_epochs: HashMap<StableNodeId, u32>,
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
        self.nodes.insert(id, record);
        self.note_node_space(id);
    }

    /// Bring `id`'s space ([`NodeRecord::space`]) up to date with its
    /// record and visual, and the indexes of it ([`Self::space_pins`],
    /// [`Self::page_bound`]). Called wherever a field that decides it is
    /// written: style resolution, the layout frame, the visual, an insert.
    pub(crate) fn note_node_space(&mut self, id: StableNodeId) {
        let visual = self.widget_visual(id);
        let Some(record) = self.nodes.get(id) else {
            return;
        };
        let space = crate::node_space::NodeSpace::of(
            &record.resolved_layout,
            record.layout_frame(),
            &record.kind,
            visual,
        );
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
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
                record.layout_frame(),
                &record.kind,
                self.widget_visual(id),
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
        if record.a11y_preset == 2 {
            self.decorative_a11y_count -= 1;
        }
        if record.visual_preset != 0 {
            self.preset_visual_count -= 1;
        }
        self.widget_visuals.remove(&id);
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
        self.texts.remove(&id);
        self.text_metrics.remove(&id);
        self.layout_paddings.remove(&id);
        self.layout_context_generations.remove(&id);
        self.scroll_offsets.remove(&id);
        self.accessibilities.remove(&id);
        self.render_room_epochs.remove(&id);
        if let Some(text) = self.text_nodes.remove(&id) {
            self.text_layouts.remove(text.layout);
        }
        Some(record)
    }

    pub(crate) fn has_text_inputs(&self) -> bool {
        !self.editors.is_empty()
    }

    pub(crate) fn has_custom_render(&self) -> bool {
        !self.custom_render.is_empty()
    }

    pub(crate) fn has_widget_visuals(&self) -> bool {
        self.preset_visual_count > 0 || !self.widget_visuals.is_empty()
    }

    pub(crate) fn text_content(&self, id: StableNodeId) -> &TextContent {
        if self.texts.is_empty() {
            return &DEFAULT_TEXT_CONTENT;
        }
        self.texts.get(&id).unwrap_or(&DEFAULT_TEXT_CONTENT)
    }

    pub(crate) fn set_text_content(&mut self, id: StableNodeId, text: TextContent) {
        let materialize = if let Some(record) = self.nodes.get(id)
            && record.a11y_preset >= 4
        {
            let current = self.text_content(id);
            (current.value.as_str() != text.value.as_str())
                .then(|| (record.a11y_preset, current.value.to_shared_str()))
        } else {
            None
        };
        if let Some((preset, old_label)) = materialize {
            self.accessibilities.insert(
                id,
                Arc::new(preset_control_accessibility(preset, old_label)),
            );
            if let Some(record) = self.nodes.get_mut(id) {
                record.a11y_preset = 3;
            }
        }
        let non_empty = !text.value.is_empty();
        if let Some(record) = self.nodes.get_mut(id) {
            record.has_text = non_empty;
            if non_empty {
                record.empty_text_backend = 0;
            }
        }
        if non_empty || text.value.stamp().is_some() {
            self.texts.insert(id, text);
        } else {
            self.texts.remove(&id);
        }
    }

    pub(crate) fn text_metrics(&self, id: StableNodeId) -> TextMetrics {
        if self.text_metrics.is_empty() {
            return TextMetrics::default();
        }
        self.text_metrics.get(&id).copied().unwrap_or_default()
    }

    pub(crate) fn set_text_metrics(&mut self, id: StableNodeId, metrics: TextMetrics) {
        if metrics == TextMetrics::default() {
            self.text_metrics.remove(&id);
        } else {
            self.text_metrics.insert(id, metrics);
        }
    }

    pub(crate) fn layout_padding(&self, id: StableNodeId) -> Option<nana_ui_core::PaddingSpec> {
        let record = self.nodes.get(id)?;
        match record.layout_padding_slot {
            0 => None,
            1 => Some(nana_ui_core::PaddingSpec::default()),
            slot if slot < u8::MAX => self
                .distinct_layout_paddings
                .get(usize::from(slot - 2))
                .copied(),
            _ => self.layout_paddings.get(&id).copied(),
        }
    }

    pub(crate) fn set_layout_padding(
        &mut self,
        id: StableNodeId,
        padding: nana_ui_core::PaddingSpec,
    ) {
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        let previous_slot = record.layout_padding_slot;
        if padding == nana_ui_core::PaddingSpec::default() {
            record.layout_padding_slot = 1;
            if previous_slot == u8::MAX {
                self.layout_paddings.remove(&id);
            }
            return;
        }
        if let Some(index) = self
            .distinct_layout_paddings
            .iter()
            .position(|existing| *existing == padding)
        {
            record.layout_padding_slot = (index + 2) as u8;
            if previous_slot == u8::MAX {
                self.layout_paddings.remove(&id);
            }
            return;
        }
        if self.distinct_layout_paddings.len() < usize::from(u8::MAX) - 2 {
            self.distinct_layout_paddings.push(padding);
            record.layout_padding_slot = (self.distinct_layout_paddings.len() + 1) as u8;
            if previous_slot == u8::MAX {
                self.layout_paddings.remove(&id);
            }
        } else {
            record.layout_padding_slot = u8::MAX;
            self.layout_paddings.insert(id, padding);
        }
    }

    pub(crate) fn layout_far_start(&self, id: StableNodeId) -> Option<[bool; 2]> {
        self.nodes.get(id)?.layout_far_start.to_option()
    }

    pub(crate) fn set_layout_far_start(&mut self, id: StableNodeId, far: [bool; 2]) {
        if let Some(record) = self.nodes.get_mut(id) {
            record.layout_far_start = LayoutFarStart::from_pair(far);
        }
    }

    #[inline]
    pub(crate) fn auto_view_slot(&self, id: StableNodeId) -> Option<u16> {
        let slot = self.nodes.get(id)?.auto_view_slot;
        (slot != u16::MAX).then_some(slot)
    }

    #[inline]
    pub(crate) fn set_auto_view_slot(&mut self, id: StableNodeId, slot: Option<u16>) {
        if let Some(record) = self.nodes.get_mut(id) {
            record.auto_view_slot = slot.unwrap_or(u16::MAX);
        }
    }

    pub(crate) fn layout_context_generation(&self, id: StableNodeId) -> u64 {
        let Some(record) = self.nodes.get(id) else {
            return 0;
        };
        if record.layout_context_gen < u8::MAX {
            return u64::from(record.layout_context_gen);
        }
        if self.layout_context_generations.is_empty() {
            return u64::from(u8::MAX);
        }
        self.layout_context_generations
            .get(&id)
            .copied()
            .unwrap_or(u64::from(u8::MAX))
    }

    pub(crate) fn bump_layout_context_generation(&mut self, id: StableNodeId) {
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        if record.layout_context_gen < u8::MAX - 1 {
            record.layout_context_gen += 1;
            return;
        }
        if record.layout_context_gen == u8::MAX - 1 {
            record.layout_context_gen = u8::MAX;
            self.layout_context_generations
                .insert(id, u64::from(u8::MAX));
            return;
        }
        let entry = self
            .layout_context_generations
            .entry(id)
            .or_insert(u64::from(u8::MAX));
        *entry = entry.wrapping_add(1);
    }

    pub(crate) fn has_scroll_offsets(&self) -> bool {
        !self.scroll_offsets.is_empty()
    }

    pub(crate) fn scroll_offset(&self, id: StableNodeId) -> ScrollOffset {
        if self.scroll_offsets.is_empty() {
            return ScrollOffset::default();
        }
        self.scroll_offsets.get(&id).copied().unwrap_or_default()
    }

    pub(crate) fn set_scroll_offset(&mut self, id: StableNodeId, offset: ScrollOffset) {
        if offset == ScrollOffset::default() {
            self.scroll_offsets.remove(&id);
        } else {
            self.scroll_offsets.insert(id, offset);
        }
    }

    #[inline]
    pub(crate) fn elided_stack_flags(&self, id: StableNodeId) -> u8 {
        self.nodes
            .get(id)
            .map_or(0, |record| record.elided_stack_flags)
    }

    #[inline]
    pub(crate) fn set_elided_stack_flags(&mut self, id: StableNodeId, flags: u8) {
        if let Some(record) = self.nodes.get_mut(id) {
            record.elided_stack_flags = flags;
        }
    }

    pub(crate) fn reserve_nodes_through(&mut self, highest: StableNodeId, entries: usize) {
        self.nodes.reserve_through(highest, entries);
    }

    pub(crate) fn reserve_text_nodes_through(&mut self, highest: StableNodeId, entries: usize) {
        self.text_nodes.reserve_through(highest, entries);
    }

    pub(crate) fn text_nodes_len(&self) -> usize {
        self.text_nodes.len()
    }

    pub(crate) fn any_hidden_accessibility(&self) -> bool {
        self.decorative_a11y_count > 0 || self.accessibilities.values().any(|state| state.hidden)
    }

    pub(crate) fn accessibility(
        &self,
        id: StableNodeId,
    ) -> std::borrow::Cow<'_, AccessibilityState> {
        use std::borrow::Cow;
        if let Some(record) = self.nodes.get(id) {
            return match record.a11y_preset {
                0 => Cow::Borrowed(DEFAULT_ACCESSIBILITY.as_ref()),
                1 => Cow::Borrowed(TEXT_ACCESSIBILITY.as_ref()),
                2 => Cow::Borrowed(DECORATIVE_TEXT_ACCESSIBILITY.as_ref()),
                preset @ 4..=6 => Cow::Owned(preset_control_accessibility(
                    preset,
                    self.text_content(id).value.to_shared_str(),
                )),
                _ => Cow::Borrowed(
                    self.accessibilities
                        .get(&id)
                        .unwrap_or(&DEFAULT_ACCESSIBILITY)
                        .as_ref(),
                ),
            };
        }
        if self.accessibilities.is_empty() {
            return Cow::Borrowed(DEFAULT_ACCESSIBILITY.as_ref());
        }
        Cow::Borrowed(
            self.accessibilities
                .get(&id)
                .unwrap_or(&DEFAULT_ACCESSIBILITY)
                .as_ref(),
        )
    }

    pub(crate) fn accessibility_eq(&self, id: StableNodeId, expected: &AccessibilityState) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        match record.a11y_preset {
            0 => *expected == **DEFAULT_ACCESSIBILITY,
            1 => *expected == **TEXT_ACCESSIBILITY,
            2 => *expected == **DECORATIVE_TEXT_ACCESSIBILITY,
            preset @ 4..=6 => {
                matching_control_preset(expected, self.text_content(id).value.as_str())
                    == Some(preset)
            }
            _ => {
                *expected
                    == **self
                        .accessibilities
                        .get(&id)
                        .unwrap_or(&DEFAULT_ACCESSIBILITY)
            }
        }
    }

    pub(crate) fn accessible_label_arc(&self, id: StableNodeId) -> Option<Arc<str>> {
        let record = self.nodes.get(id)?;
        match record.a11y_preset {
            0..=2 => None,
            4..=6 => Some(self.text_content(id).value.to_shared_str()),
            _ => self
                .accessibilities
                .get(&id)
                .and_then(|state| state.label.as_ref().map(Arc::clone)),
        }
    }

    pub(crate) fn set_accessibility(&mut self, id: StableNodeId, state: &AccessibilityState) {
        let preset = if *state == **DEFAULT_ACCESSIBILITY {
            0
        } else if *state == **TEXT_ACCESSIBILITY {
            1
        } else if *state == **DECORATIVE_TEXT_ACCESSIBILITY {
            2
        } else if self.nodes.get(id).is_some_and(|record| record.has_text)
            && let Some(control_preset) =
                matching_control_preset(state, self.text_content(id).value.as_str())
        {
            control_preset
        } else {
            3
        };
        if let Some(record) = self.nodes.get_mut(id) {
            if record.a11y_preset == 2 && preset != 2 {
                self.decorative_a11y_count -= 1;
            } else if record.a11y_preset != 2 && preset == 2 {
                self.decorative_a11y_count += 1;
            }
            record.a11y_preset = preset;
        }
        if preset == 3 {
            self.accessibilities.insert(id, shared_accessibility(state));
        } else {
            self.accessibilities.remove(&id);
        }
    }

    pub(crate) fn render_room_epoch(&self, id: StableNodeId) -> u32 {
        if self.render_room_epochs.is_empty() {
            return u32::MAX;
        }
        self.render_room_epochs
            .get(&id)
            .copied()
            .unwrap_or(u32::MAX)
    }

    pub(crate) fn set_render_room_epoch(&mut self, id: StableNodeId, epoch: u32) {
        if epoch == u32::MAX {
            self.render_room_epochs.remove(&id);
        } else {
            self.render_room_epochs.insert(id, epoch);
        }
    }

    pub fn widget_visual(&self, id: StableNodeId) -> Option<&crate::WidgetVisualHandle> {
        if self
            .nodes
            .get(id)
            .is_some_and(|record| record.visual_preset == 1)
        {
            return Some(&EMPTY_LIST_ITEM_VISUAL);
        }
        self.widget_visuals.get(&id)
    }

    pub fn set_widget_visual(
        &mut self,
        id: StableNodeId,
        value: Option<crate::WidgetVisualHandle>,
    ) {
        let text_key_changed = self.widget_visual(id).and_then(|v| v.text_visual_key())
            != value.as_ref().and_then(|v| v.text_visual_key());
        let is_empty_list_item_preset = value
            .as_ref()
            .is_some_and(|handle| handle.ptr_eq(&EMPTY_LIST_ITEM_VISUAL));
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        let prev_preset = record.visual_preset;
        if is_empty_list_item_preset {
            record.visual_preset = 1;
            if prev_preset == 0 {
                self.preset_visual_count += 1;
            }
            self.widget_visuals.remove(&id);
        } else {
            if prev_preset != 0 {
                record.visual_preset = 0;
                self.preset_visual_count -= 1;
            }
            match value {
                Some(value) => {
                    self.widget_visuals.insert(id, value);
                }
                None => {
                    self.widget_visuals.remove(&id);
                }
            }
        }
        if text_key_changed {
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
    pub fn component_type(&self, id: StableNodeId) -> Option<&ComponentTypeId> {
        let record = self.nodes.get(id)?;
        if record.builtin_component != 0 {
            return crate::component_descriptors::builtin_component_type(record.builtin_component);
        }
        self.component_types.get(&id)
    }

    pub fn set_component_type(&mut self, id: StableNodeId, value: Option<ComponentTypeId>) {
        match value {
            Some(value) => {
                if let Some(index) =
                    crate::component_descriptors::builtin_component_index(value.as_str())
                    && let Some(record) = self.nodes.get_mut(id)
                {
                    record.builtin_component = index;
                    self.component_types.remove(&id);
                    return;
                }
                if let Some(record) = self.nodes.get_mut(id) {
                    record.builtin_component = 0;
                }
                self.component_types.insert(id, value);
            }
            None => {
                if let Some(record) = self.nodes.get_mut(id) {
                    record.builtin_component = 0;
                }
                self.component_types.remove(&id);
            }
        }
    }
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

    fn intern_empty_text_backend(&mut self, backend: crate::text_node::TextBackendEpoch) -> u8 {
        if let Some(index) = self
            .empty_text_backends
            .iter()
            .position(|existing| *existing == backend)
        {
            return (index + 1) as u8;
        }
        if self.empty_text_backends.len() < u8::MAX as usize {
            self.empty_text_backends.push(backend);
            return self.empty_text_backends.len() as u8;
        }
        0
    }

    pub(crate) fn is_empty_text_current(
        &self,
        id: StableNodeId,
        backend: crate::text_node::TextBackendEpoch,
    ) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        if record.has_text
            || matches!(record.kind.as_ref(), crate::world::NodeKind::Text)
            || record.empty_text_backend == 0
        {
            return false;
        }
        self.empty_text_backends
            .get(usize::from(record.empty_text_backend) - 1)
            .copied()
            == Some(backend)
    }

    pub(crate) fn note_text_scale(&mut self, id: StableNodeId) -> bool {
        self.text_nodes
            .get_mut(&id)
            .is_some_and(crate::text_node::TextNodeState::note_scale)
    }

    pub(crate) fn take_text_scale_pending(&mut self, id: StableNodeId) -> bool {
        self.text_nodes
            .get_mut(&id)
            .is_some_and(crate::text_node::TextNodeState::take_scale_pending)
    }

    /// Records a change to `id`'s text. See [`crate::text_node::TextDirty`].
    pub(crate) fn invalidate_text(&mut self, id: StableNodeId, dirty: crate::text_node::TextDirty) {
        if dirty.is_empty() {
            return;
        }
        if let Some(text) = self.text_nodes.get_mut(&id) {
            text.invalidate(dirty);
            return;
        }
        let Some(record) = self.nodes.get_mut(id) else {
            return;
        };
        let work = dirty.work();
        if dirty.intersects(crate::text_node::TextDirty::CONTENT)
            || work.intersects(crate::text_node::TextWork::SHAPE)
            || work.intersects(crate::text_node::TextWork::LAYOUT)
        {
            record.empty_text_backend = 0;
        }
        if record.has_text || matches!(record.kind.as_ref(), crate::world::NodeKind::Text) {
            self.text_nodes.entry(id).or_default().invalidate(dirty);
        }
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
        let text_node =
            record.has_text || matches!(record.kind.as_ref(), crate::world::NodeKind::Text);
        if !text_node && !self.text_nodes.contains_key(&id) {
            let backend_index = self.intern_empty_text_backend(backend);
            if let Some(record) = self.nodes.get_mut(id) {
                record.empty_text_backend = backend_index;
            }
            return false;
        }
        let constraints =
            constraints.map(|constraints| (constraints, record.style.text_horizontal_alignment));
        let language = record.resolved.0.language.clone();
        self.text_nodes.entry(id).or_default().mark_resolved(
            backend,
            constraints,
            text_node,
            language.as_ref(),
        )
    }

    /// The `nana-text` source for `id`'s current text, built once per content
    /// revision, and whether this call built it.
    pub(crate) fn text_source(
        &mut self,
        id: StableNodeId,
        style: &crate::ComputedStyle,
    ) -> Option<(&nana_text::TextSource, bool)> {
        if !self.nodes.contains(id) {
            return None;
        }
        let text = self.texts.get(&id).unwrap_or(&DEFAULT_TEXT_CONTENT);
        let rich = self.rich_texts.get(&id);
        Some(
            self.text_nodes.entry(id).or_default().source_for_objects(
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
        if !self.nodes.contains(id) {
            return false;
        }
        let text = self.text_nodes.entry(id).or_default();
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
        let widget_measuring = self
            .widget_visuals
            .iter()
            .filter(|(_, visual)| visual.measures_text())
            .map(|(id, _)| *id);
        let preset_measuring = (self.preset_visual_count > 0)
            .then(|| {
                self.nodes
                    .entries()
                    .filter(|(_, record)| record.visual_preset == 1)
                    .map(|(id, _)| id)
            })
            .into_iter()
            .flatten();
        resolved.chain(widget_measuring).chain(preset_measuring)
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
        store.set_text_content(id, TextContent { value: "hi".into() });
        store.set_text_metrics(
            id,
            TextMetrics {
                width: 12.0,
                height: 16.0,
                ascent: Some(12.0),
            },
        );
        store.set_layout_padding(id, nana_ui_core::PaddingSpec::uniform(4.0));
        store.set_layout_far_start(id, [true, false]);
        store.bump_layout_context_generation(id);
        store.set_scroll_offset(id, ScrollOffset { x: 0.0, y: 8.0 });
        store.set_accessibility(
            id,
            &AccessibilityState {
                hidden: true,
                ..AccessibilityState::default()
            },
        );
        store.set_render_room_epoch(id, 42);
        assert!(store.scroll_metrics(id).is_some());
        assert!(store.remove(id).is_some());
        assert!(!store.contains(id));
        assert!(store.scroll_metrics(id).is_none());
        assert!(store.component_type(id).is_none());
        assert!(store.ime(id).is_none());
        assert_eq!(store.text_content(id).value, "");
        assert_eq!(store.text_metrics(id), TextMetrics::default());
        assert_eq!(store.layout_padding(id), None);
        assert_eq!(store.layout_far_start(id), None);
        assert_eq!(store.layout_context_generation(id), 0);
        assert_eq!(store.scroll_offset(id), ScrollOffset::default());
        assert!(!store.accessibility(id).hidden);
        assert_eq!(store.render_room_epoch(id), u32::MAX);
    }

    #[test]
    fn node_record_fits_size_budget() {
        let size = std::mem::size_of::<NodeRecord>();
        eprintln!("size_of::<NodeRecord>() = {size}");
        assert!(
            size <= 152,
            "NodeRecord grew to {size} bytes (budget <= 152 bytes)"
        );
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
