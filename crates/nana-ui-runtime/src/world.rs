mod accessibility;
mod animation;
mod block_reads;
mod child_numbering;
pub(crate) mod child_order;
mod clip_chain;
mod extraction;
mod focus_nav;
mod focus_scope;
mod geometry;
pub(crate) use geometry::{PROGRESS_GIRTH, range_value_thumb_height};
use nana_ui_core::sort::CompactSort as _;
mod hit_test;
mod i18n;
mod input;
mod motion;
mod mutation;
mod overlay_index;
pub(crate) mod page_origins;
mod presentation;
pub use i18n::{LocaleGenerations, LocaleScope};
#[cfg(feature = "controls")]
mod reorder_motion;
mod replaced;
mod responsive;
pub(crate) use block_reads::BlockExtentReads;
pub use replaced::{ReplacedMetadata, ReplacedResource};
pub(crate) mod content_parts;
pub(crate) mod inline_content;
mod scroll_bounds;
mod sticky;
mod style;
mod style_pairs;
mod subsystems;
mod text;
mod theme_inspect;
mod theme_scope;
use hit_test::*;
use subsystems::*;
pub(crate) use text::TextDisplayView;
use text::*;
pub use theme_inspect::{
    LastThemeInvalidation, ThemeDependencySummary, ThemeInteractionState, ThemeInvalidationTrigger,
    ThemeNodeInspection, ThemeNodeInvalidationStatus, ThemeResolvedToken, ThemeScopeChainEntry,
    ThemeTraceLayer, ThemeTransitionSource,
};

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
    mem::size_of,
    sync::Arc,
    time::Duration,
};

use nana_ui_core::{
    InvalidationKind, InvalidationReason, LayoutDependencyFootprint, LayoutInvalidation,
    LayoutInvalidationSource, LayoutStyle, LayoutStyleChange, LengthSpec, PointerEventsSpec,
    PositionSpec, SemanticColorRole, SemanticPalette, StyleModelRef, ThemeAppearance,
    ThemeWorkCounters,
};

#[cfg(all(test, feature = "graph-canvas"))]
use nana_ui_core::{GraphPoint, cubic_point};

use crate::{
    AccessibilityDelta, AccessibilityLive, AccessibilityNode, AccessibilityRole,
    AccessibilityState, AnimationFrame, AnimationId, AnimationSpec, ComponentTypeId, ComputedStyle,
    CustomRenderNode, EventListeners, EventRoute, ExtractedMove, ExtractedNode, ExtractedTextSpan,
    HighlightRequest, ImeComposition, InteractionState, LayoutBox, LayoutInput, MotionWorkCounters,
    MountState, MutationQueue, NodeStyle, OverlayHostState, PointerCaptureChange, ScrollMetrics,
    ScrollOffset, TextContent, TextMetrics, TextPresentation, TextPresenter, TextShaper,
    TextVerticalAlignment, UiMutation, WorkCounters,
    animation::ActiveAnimation,
    components::{
        EmptyStateTextPresentation, ModalTextPresentation, TextColorSwatchSpan,
        TextEditorRenderOptions, TextGitGutterMark, TextGitMark, TextGitMarkKind,
        TextInputPresentation, TextMatchMark, TextMatchMarker, TextMatchSpan, TextOverlayMetrics,
        TextSwatchMark, TextWhitespaceKind, TextWhitespaceMark,
    },
    schedule::{DirtyMask, SystemWork, push_work},
    store::{Hierarchy, NodeRecord, NodeStore, ResolvedStyle, intern_empty_children},
    text_editing::clamp_boundary,
};
/// `index` as a child records it: numbers wrap, and a list is far shorter
/// than they are wide.
fn child_number(index: usize) -> u32 {
    index as u32
}

/// Stable external node identity. Zero is reserved so missing/default IDs
/// cannot accidentally address a live node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableNodeId(std::num::NonZeroU64);

impl StableNodeId {
    pub const fn new(value: u64) -> Option<Self> {
        match std::num::NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Hashing for keys that are already integers: node ids, slot numbers, a hash
/// something else computed.
///
/// The default hasher is SipHash — a keyed MAC, which is the right default for
/// a map whose keys come off the wire and a poor one for a counter nobody
/// outside the process can choose. Scene paint asks such maps several times per
/// primitive per frame; a sampling profile of a transform animation put SipHash
/// alone at a tenth of the painter's time.
///
/// The mixing step is rustc's: rotate, xor the word in, multiply by an odd
/// constant. Ids are a dense counter, so the work is spreading them, not
/// hiding them.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdHasher(u64);

impl IdHasher {
    const SEED: u64 = 0x517c_c1b7_2722_0a95;

    fn add(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(Self::SEED);
    }
}

impl std::hash::Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.add(u64::from(*byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
}

/// What a map or set keyed by integers hashes with. See [`IdHasher`].
pub type BuildIdHasher = std::hash::BuildHasherDefault<IdHasher>;

/// [`HashMap`] keyed by node, hashed by [`IdHasher`].
pub type NodeMap<V> = HashMap<StableNodeId, V, BuildIdHasher>;

/// [`HashSet`] of nodes, hashed by [`IdHasher`].
pub type NodeSet = HashSet<StableNodeId, BuildIdHasher>;

pub(crate) use crate::store::{
    PublishedLayoutExt, PublishedLayoutMeta, PublishedRareGeometry, TEXT_SOURCE_MAX_LEN,
    TEXT_SOURCE_MISSING, TEXT_SOURCE_NONE_SENTINEL,
};

struct OwnLayoutBuild {
    result: crate::LayoutResult,
    deps_hash: u64,
    single_text_source: u32,
    text_box: LayoutBox,
    has_extra_parts: bool,
    fixed_or_clips: bool,
}

fn hash_dependencies(deps: &[StableNodeId]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    hash ^= deps.len() as u64;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    for &id in deps {
        hash ^= id.get();
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where the content-box part goes among a result's own `parts`: after its
/// text part, when it has one.
fn content_part_index(parts: &[crate::LayoutPart]) -> usize {
    usize::from(
        parts
            .first()
            .is_some_and(|part| part.kind == crate::LayoutPartKind::TextContent),
    )
}

#[derive(Default)]
struct ResultScratch {
    parts: Vec<crate::LayoutPart>,
    dependencies: Vec<StableNodeId>,
    /// The own fragments and parts at the origin, the content-box part left
    /// out, for the placement to place.
    own_fragments: Vec<crate::LayoutFragment>,
    own_parts: Vec<crate::LayoutPart>,
    /// The slices this publish built last, shared by the results equal to
    /// them: a result is in its own coordinates and holds only its own
    /// geometry, so boxes alike in size (the cells of a row, the rows of a
    /// list) hold equal parts and fragments.
    shared_parts: RecentSlices<crate::LayoutPart>,
    shared_fragments: RecentSlices<crate::LayoutFragment>,
}

/// The last few slices handed out, so an equal one is shared instead of
/// allocated again. A slice is never written once shared, so sharing it
/// is invisible to its readers.
struct RecentSlices<T> {
    recent: [Option<Arc<[T]>>; RECENT_SLICES],
    next: usize,
}

const RECENT_SLICES: usize = 4;

impl<T> Default for RecentSlices<T> {
    fn default() -> Self {
        Self {
            recent: std::array::from_fn(|_| None),
            next: 0,
        }
    }
}

impl<T: Clone + PartialEq> RecentSlices<T> {
    /// `items` as a shared slice: an empty one allocates nothing, and one
    /// equal to a recent slice is that slice.
    fn share(&mut self, items: &[T]) -> Arc<[T]> {
        if items.is_empty() {
            return Arc::default();
        }
        if let Some(kept) = self.recent.iter().flatten().find(|kept| ***kept == *items) {
            return Arc::clone(kept);
        }
        let made: Arc<[T]> = Arc::from(items);
        self.recent[self.next] = Some(Arc::clone(&made));
        self.next = (self.next + 1) % RECENT_SLICES;
        made
    }
}

impl ResultScratch {
    fn clear(&mut self) {
        self.parts.clear();
        self.dependencies.clear();
        self.own_fragments.clear();
        self.own_parts.clear();
    }
}

/// `items` as a shared slice; an empty one allocates nothing.
fn shared_slice<T: Clone>(items: &[T]) -> Arc<[T]> {
    if items.is_empty() {
        Arc::default()
    } else {
        Arc::from(items)
    }
}

fn sorted_unique(mut ids: Vec<StableNodeId>) -> Vec<StableNodeId> {
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// See [`UiWorld::is_scroll_container`].
fn scroll_container(
    layout: &nana_ui_core::LayoutStyle,
    widget_visual: Option<&crate::WidgetVisualHandle>,
) -> bool {
    layout.overflow_x.scrolls()
        || layout.overflow_y.scrolls()
        || widget_visual.is_some_and(crate::WidgetVisualHandle::is_scroll_container)
}

/// See [`UiWorld::overflow_contained_axes`].
fn overflow_contained_axes(
    layout: &nana_ui_core::LayoutStyle,
    widget_visual: Option<&crate::WidgetVisualHandle>,
) -> [bool; 2] {
    if scroll_container(layout, widget_visual) {
        [true, true]
    } else {
        [layout.overflow_x.clips(), layout.overflow_y.clips()]
    }
}

/// Deepest retained tree the frame pipeline accepts.
///
/// Style resolution walks ancestors, layout and hit-test walk descendants, and
/// paint walks the scene: all recursive. Tree shape comes from application
/// input and is not trustworthy, so the bound is enforced once where the tree is
/// written. Real UIs nest one to two orders of magnitude below this.
pub const MAX_TREE_DEPTH: usize = 512;

/// Most splits a panel layout nests: a [`crate::DockNode`],
/// [`crate::PaneTreeNode`] or [`crate::AreaNode`] tree. Each split halves a
/// pane, so a useful layout stays far inside this. A split the framework
/// makes (a dock drop, [`crate::AreaLayoutModel::split`]) past it is refused;
/// a layout this deep still saves and restores through `serde_json`, which
/// reads at most 128 levels, so no layout the framework reads back is deeper
/// than about twice this. The walks over a layout recurse by its depth and
/// are tested at it on a 512 KiB stack. A deeper tree an application builds
/// itself is folded to this depth as the framework takes it in (the
/// components' constructors, and every create and update of a `Dock`,
/// `PaneTree` or `AreaLayout`): a dock keeps every item, in one tab group;
/// a pane tree or area layout keeps the first pane or area of each folded
/// split.
pub const MAX_SPLIT_DEPTH: usize = 64;

/// Retired node IDs, stored as coalesced inclusive runs.
///
/// Retirement is permanent: a stale handle must never alias a node created
/// later. The ID allocator ([`crate::AppContext`]) is
/// strictly monotonic, so churning a list retires consecutive IDs. Keeping runs
/// instead of one entry per ID bounds the ledger by the number of gaps in the
/// allocation stream rather than by every node ever destroyed, with identical
/// membership semantics.
#[derive(Debug, Default)]
struct RetiredIds {
    /// Sorted, disjoint, and never adjacent: `(start, end)` inclusive.
    runs: Vec<(u64, u64)>,
    len: usize,
}

impl RetiredIds {
    /// Index of the run containing `value`, or the insertion point.
    fn locate(&self, value: u64) -> Result<usize, usize> {
        self.runs.binary_search_by(|(start, end)| {
            if value < *start {
                std::cmp::Ordering::Greater
            } else if value > *end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
    }

    fn contains(&self, id: StableNodeId) -> bool {
        self.locate(id.get()).is_ok()
    }

    fn insert(&mut self, id: StableNodeId) {
        let value = id.get();
        let Err(index) = self.locate(value) else {
            return;
        };
        let joins_previous = index > 0 && self.runs[index - 1].1.checked_add(1) == Some(value);
        let joins_next =
            index < self.runs.len() && Some(self.runs[index].0) == value.checked_add(1);
        match (joins_previous, joins_next) {
            (true, true) => {
                self.runs[index - 1].1 = self.runs[index].1;
                self.runs.remove(index);
            }
            (true, false) => self.runs[index - 1].1 = value,
            (false, true) => self.runs[index].0 = value,
            (false, false) => self.runs.insert(index, (value, value)),
        }
        self.len += 1;
    }

    fn len(&self) -> usize {
        self.len
    }

    /// Stored runs. This is the ledger's real memory cost.
    fn runs(&self) -> usize {
        self.runs.len()
    }
}

/// Stable document/window ownership boundary.
///
/// An application takes one per document from [`Self::allocate`]; a fixed
/// number through [`Self::new`] is for tests and fixtures that name their
/// documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(u64);

/// First id [`DocumentId::allocate`] hands out: above any number a test or
/// fixture writes by hand.
const FIRST_ALLOCATED_DOCUMENT: u64 = 1 << 16;
static NEXT_DOCUMENT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(FIRST_ALLOCATED_DOCUMENT);

impl DocumentId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// An id no other call in this process returns.
    pub fn allocate() -> Self {
        Self(NEXT_DOCUMENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element { tag: String },
    Text,
    Comment,
}

/// Menu rows are painted from one retained surface, so their virtual
/// accessibility children change without a retained child mutation. Keep the
/// semantic projection in sync when rows, filtering, or the highlighted item
/// changes even though the surface remains open.
pub(crate) fn menu_surface_accessibility_changed(
    previous: Option<&crate::WidgetVisualHandle>,
    next: Option<&crate::WidgetVisualHandle>,
) -> bool {
    match (
        previous.and_then(|v| v.downcast_ref::<crate::MenuSurfaceVisual>()),
        next.and_then(|v| v.downcast_ref::<crate::MenuSurfaceVisual>()),
    ) {
        (Some(prev), Some(next)) => {
            prev.open != next.open
                || prev.rows != next.rows
                || prev.highlighted != next.highlighted
                || prev.query != next.query
        }
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => false,
    }
}

/// A select field's open menu is a list box of virtual options: they
/// follow its rows, its highlight, and where it is placed and scrolled.
/// The pointer heating the scrollbar changes none of them.
pub(crate) fn select_accessibility_changed(
    previous: Option<&crate::WidgetVisualHandle>,
    next: Option<&crate::WidgetVisualHandle>,
) -> bool {
    match (
        previous.and_then(|visual| visual.downcast_ref::<crate::SelectVisual>()),
        next.and_then(|visual| visual.downcast_ref::<crate::SelectVisual>()),
    ) {
        (Some(previous), Some(next)) => {
            previous.opened != next.opened
                || (next.opened
                    && (previous.options != next.options
                        || previous.highlighted != next.highlighted
                        || previous.menu_scroll.offset != next.menu_scroll.offset
                        || previous.menu_room != next.menu_room
                        || previous.size != next.size
                        || previous.fit_options != next.fit_options))
        }
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => false,
    }
}

/// A tree's rows and their toggles are virtual accessibility children too:
/// its semantics follow the visible rows (toggle states included), the
/// active one and the toggle columns' names.
pub(crate) fn tree_view_accessibility_changed(
    previous: Option<&crate::WidgetVisualHandle>,
    next: Option<&crate::WidgetVisualHandle>,
) -> bool {
    match (
        previous.and_then(|visual| visual.downcast_ref::<crate::TreeViewVisual>()),
        next.and_then(|visual| visual.downcast_ref::<crate::TreeViewVisual>()),
    ) {
        (Some(previous), Some(next)) => {
            previous.rows != next.rows
                || previous.active != next.active
                || previous.column_labels != next.column_labels
        }
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => false,
    }
}

/// Whether a menu surface with `visual` hides its child `child`: a closed
/// surface hides its items, never the content that draws its trigger.
#[derive(Debug, Clone)]
struct HitEntry {
    id: StableNodeId,
    /// Shared authoritative child sequence at projection time. Pointer identity
    /// validates cached sibling ordinals without searching a wide parent.
    source_children: Arc<Vec<StableNodeId>>,
    /// The border box in the entry's own coordinates: at the origin, its
    /// size. Everything below is in that space too, `transform` mapping it
    /// to the space the walk reaches the entry in, [`Self::origin`] away
    /// from its parent's children.
    layout: LayoutBox,
    /// Where the box stands from its hit parent's origin, mapped through the
    /// linear part of what the parent hands its children: the one value a
    /// move of the box changes. The children of an entry that only moved
    /// keep theirs (relative geometry, stage 2). Zero under a perspective,
    /// which takes the offset into `transform` instead.
    origin: [f32; 2],
    /// Whether the entry reads geometry from beyond its own box and its hit
    /// parent's: an offset measured across frames, a component's or a
    /// sticky box's placement, an escaped bridge. A move of a box above it
    /// builds it again.
    pinned: bool,
    transform: [f32; 6],
    persp: [f32; 2],
    /// Whether the entry hands its children its own layout coordinates
    /// rather than the space `transform` maps them to: a box whose own 2D
    /// transform scales, rotates, skews or flips, with no perspective or 3D
    /// context on the way. Its children's entries then hold nothing of its
    /// transform or of anything above it, so a turn, a move or a resize of
    /// it (which moves its transform origin) builds its entry alone, and a
    /// box moved under it is the upright move of an ordinary entry. A query
    /// takes the point through the inverse of `transform` on the way down.
    local_children: bool,
    /// The scroll offset the index holds this entry's children at: the live
    /// one when it was built, moved by every scroll shift since. An entry
    /// built again in place over its children must find the live offset
    /// here, or a scroll the index has not taken yet would be lost.
    scroll: [f32; 2],
    /// Clips applied to this node's own hit (and therefore its subtree).
    self_clips: Vec<(LayoutBox, [f32; 6])>,
    /// Extra clips applied to descendants only (overflow / visual frames).
    child_clips: Vec<(LayoutBox, [f32; 6])>,
    /// Shape clips from `paint.clip_path`, kept in node-local coordinates so
    /// presentation transforms can be sampled at hit-test time.
    path_clips: Vec<crate::world::hit_test::HitPathClip>,
    z_index: i32,
    order: usize,
    hittable: bool,
    menu: Option<LayoutBox>,
    /// A painter that decides which points of the box hit (Issue #217).
    painter: Option<Arc<PainterHit>>,
    /// The entry hangs from its clip parent, not its parent: an absolute box
    /// that escapes overflow containers (#201). Its sibling order then says
    /// nothing about where it paints among its new siblings, so a point it
    /// covers is ranked by paint order.
    escaped: bool,
    children: Vec<HitEntry>,
}

/// A node's latest recording, shared with its hit index entry so a
/// re-record — on a state, focus, theme or font change, whichever path
/// asked for it — reaches hit testing without the index being rebuilt.
type SharedRecording = Arc<std::sync::RwLock<Arc<crate::PaintRecording>>>;

/// One node's last recording and what it was recorded against.
#[derive(Debug)]
struct PaintCacheEntry {
    key: crate::custom_paint::PaintCacheKey,
    recording: Arc<crate::PaintRecording>,
    /// The same recording, for the hit index.
    latest: SharedRecording,
    /// What the painter's text was measured against — the node's resolved
    /// style and the host's font backend — when it measured any. Only then
    /// does a font or backend change re-record it.
    measured_text: Option<(
        Arc<crate::ComputedStyle>,
        Option<crate::text_node::TextBackendEpoch>,
    )>,
}

/// What a painted node's hit test asks: the painter's own answer, else the
/// recording's outline when the painter asked for that.
#[derive(Debug)]
struct PainterHit {
    painter: crate::NodePainter,
    recording: Option<SharedRecording>,
}

/// Per-pass cache of ancestor-chain answers. Extraction and hit-index share
/// most of each chain, so one walk fills every node on it.
#[derive(Default)]
struct AncestorMemo {
    live: HashMap<StableNodeId, bool>,
    stacking: HashMap<StableNodeId, i32>,
    inline_visible: HashMap<StableNodeId, bool>,
    /// Paint color filled this extract pass after a palette epoch change.
    color: HashMap<StableNodeId, [f32; 4]>,
    chain: Vec<StableNodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSnapshot {
    pub id: StableNodeId,
    pub document: DocumentId,
    pub kind: NodeKind,
    pub parent: Option<StableNodeId>,
    pub children: Vec<StableNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitReport {
    pub generation: u64,
    pub mutations: usize,
    pub created: usize,
    pub inserted: usize,
    pub detached: usize,
    pub reparented: usize,
    pub despawned: usize,
}

impl CommitReport {
    /// A commit of `mutations` that changed nothing.
    pub(crate) const fn unchanged(generation: u64, mutations: usize) -> Self {
        Self {
            generation,
            mutations,
            created: 0,
            inserted: 0,
            detached: 0,
            reparented: 0,
            despawned: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiWorldError {
    DuplicateNode(StableNodeId),
    RetiredNode(StableNodeId),
    MissingNode(StableNodeId),
    CrossDocument {
        parent: StableNodeId,
        child: StableNodeId,
    },
    FocusDocument {
        document: DocumentId,
        target: StableNodeId,
    },
    PointerDocument {
        document: DocumentId,
        target: StableNodeId,
    },
    Cycle {
        parent: StableNodeId,
        child: StableNodeId,
    },
    /// Parenting `child` under `parent` would exceed [`MAX_TREE_DEPTH`]. Style
    /// resolution, layout, hit-test and paint all recurse over the retained
    /// tree, so the depth bound is enforced where the tree is written rather
    /// than re-checked in every walk.
    TreeTooDeep {
        parent: StableNodeId,
        child: StableNodeId,
        depth: usize,
    },
    InvalidBefore {
        parent: StableNodeId,
        before: StableNodeId,
    },
    /// A [`UiMutation::ReorderChildren`] whose order is not `parent`'s
    /// children, each once.
    InvalidChildOrder {
        parent: StableNodeId,
    },
    InvalidStyle(StableNodeId),
    InvalidText(StableNodeId),
    InvalidLayout(StableNodeId),
    InvalidScrollOffset(StableNodeId),
    InvalidScrollMetrics(StableNodeId),
    InvalidIme(StableNodeId),
    InvalidCustomRender(StableNodeId),
    InvalidEventListener(StableNodeId),
    InvalidWidgetVisual(StableNodeId),
    InvalidInlineContent(StableNodeId),
    InlineTextEngineRequired(StableNodeId),
    InvalidOverlayHost(StableNodeId),
    NotFocusable(StableNodeId),
    NotPointerInteractive(StableNodeId),
    NotFocused(StableNodeId),
    PointerCaptureMismatch {
        pointer_id: u64,
        target: StableNodeId,
    },
    InvalidAnimation(AnimationId),
    MissingAnimation(AnimationId),
    InvalidTextInput(StableNodeId),
    MissingTextInput(StableNodeId),
    InvalidHighlightRequest(StableNodeId),
    InvalidPresenter,
    DuplicatePresenter(String),
    /// A replaced resource reported a natural size that is not a finite,
    /// non-negative width and height.
    InvalidReplacedMetadata,
    /// A responsive rule whose breakpoints are not finite and ascending, or
    /// that has more than the bounded number of them.
    InvalidResponsiveRule(StableNodeId),
}

impl fmt::Display for UiWorldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(formatter, "node {} already exists", id.get()),
            Self::RetiredNode(id) => write!(formatter, "node {} was retired", id.get()),
            Self::MissingNode(id) => write!(formatter, "node {} does not exist", id.get()),
            Self::CrossDocument { parent, child } => write!(
                formatter,
                "cannot parent node {} under node {} from another document",
                child.get(),
                parent.get()
            ),
            Self::FocusDocument { document, target } => write!(
                formatter,
                "node {} does not belong to document {}",
                target.get(),
                document.get()
            ),
            Self::PointerDocument { document, target } => write!(
                formatter,
                "pointer target {} does not belong to document {}",
                target.get(),
                document.get()
            ),
            Self::Cycle { parent, child } => write!(
                formatter,
                "parenting node {} under node {} would create a cycle",
                child.get(),
                parent.get()
            ),
            Self::TreeTooDeep {
                parent,
                child,
                depth,
            } => write!(
                formatter,
                "parenting node {} under node {} would reach depth {depth}, past the {MAX_TREE_DEPTH} limit",
                child.get(),
                parent.get()
            ),
            Self::InvalidBefore { parent, before } => write!(
                formatter,
                "node {} is not a child of parent {}",
                before.get(),
                parent.get()
            ),
            Self::InvalidChildOrder { parent } => write!(
                formatter,
                "the order given for the children of node {} is not its children, each once",
                parent.get()
            ),
            Self::InvalidStyle(id) => write!(formatter, "node {} has an invalid style", id.get()),
            Self::InvalidInlineContent(id) => write!(
                formatter,
                "node {} has invalid inline content or atomic children",
                id.get()
            ),
            Self::InlineTextEngineRequired(id) => write!(
                formatter,
                "mixed inline node {} requires the nana-text layout engine",
                id.get()
            ),
            Self::InvalidText(id) => {
                write!(formatter, "node {} has invalid text metrics", id.get())
            }
            Self::InvalidLayout(id) => {
                write!(formatter, "node {} has an invalid layout box", id.get())
            }
            Self::InvalidScrollOffset(id) => {
                write!(formatter, "node {} has an invalid scroll offset", id.get())
            }
            Self::InvalidScrollMetrics(id) => {
                write!(formatter, "node {} has invalid scroll metrics", id.get())
            }
            Self::InvalidIme(id) => write!(formatter, "node {} has an invalid IME range", id.get()),
            Self::InvalidCustomRender(id) => {
                write!(
                    formatter,
                    "node {} has invalid custom render content",
                    id.get()
                )
            }
            Self::InvalidReplacedMetadata => {
                write!(
                    formatter,
                    "a replaced resource reported an invalid natural size"
                )
            }
            Self::InvalidResponsiveRule(id) => {
                write!(
                    formatter,
                    "node {} has a responsive rule with invalid breakpoints",
                    id.get()
                )
            }
            Self::InvalidEventListener(id) => {
                write!(
                    formatter,
                    "node {} has an invalid event listener name",
                    id.get()
                )
            }
            Self::InvalidWidgetVisual(id) => {
                write!(
                    formatter,
                    "node {} has invalid widget visual state",
                    id.get()
                )
            }
            Self::InvalidOverlayHost(id) => {
                write!(formatter, "node {} has an invalid active overlay", id.get())
            }
            Self::NotFocusable(id) => write!(formatter, "node {} cannot receive focus", id.get()),
            Self::NotPointerInteractive(id) => {
                write!(formatter, "node {} cannot receive pointer input", id.get())
            }
            Self::NotFocused(id) => write!(formatter, "node {} is not focused", id.get()),
            Self::PointerCaptureMismatch { pointer_id, target } => write!(
                formatter,
                "pointer {pointer_id} is not captured by node {}",
                target.get()
            ),
            Self::InvalidAnimation(id) => write!(formatter, "animation {} is invalid", id.get()),
            Self::MissingAnimation(id) => {
                write!(formatter, "animation {} is not active", id.get())
            }
            Self::InvalidTextInput(id) => {
                write!(formatter, "node {} has invalid text input state", id.get())
            }
            Self::MissingTextInput(id) => {
                write!(formatter, "node {} has no text input state", id.get())
            }
            Self::InvalidHighlightRequest(id) => {
                write!(
                    formatter,
                    "node {} has an invalid highlight request",
                    id.get()
                )
            }
            Self::InvalidPresenter => formatter.write_str("presenter name must not be empty"),
            Self::DuplicatePresenter(name) => {
                write!(formatter, "presenter `{name}` is already registered")
            }
        }
    }
}

impl std::error::Error for UiWorldError {}

#[derive(Debug, Clone)]
struct PlannedNode {
    document: DocumentId,
    parent: Option<StableNodeId>,
    children: Vec<StableNodeId>,
}

/// Which device the input being routed came from, for `:focus-visible`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputModality {
    Keyboard,
    Pointer,
    Gamepad,
    Programmatic,
}

/// The work counters a world keeps: the last drain's, the frame's being
/// accumulated, and two sets counted between drains. Boxed in [`UiWorld`].
#[derive(Default)]
struct WorldCounters {
    last_counters: WorkCounters,
    frame_counters: WorkCounters,
    /// What the layout mutation authority classified since the last fold.
    /// An effective-equivalent write dirties nothing, so its count cannot
    /// wait for a non-empty drain to be seen.
    pending_layout_invalidations_counted: WorkCounters,
    /// What changes made between frames cost, counted when they were made.
    /// Their cost belongs to the frame that resolves them: these wait for
    /// that frame's first drain, not for the start of the frame.
    pending_drain_counts: WorkCounters,
}

/// Single-entry caches a code editor's presentation reads through `&self`.
#[derive(Default)]
struct EditorPresentationCaches {
    /// minimap 行长单条缓存（原始值 → 每逻辑行非空白字符数）。存于
    /// `RefCell` 供 `&self` 的 presentation 构建路径读写。
    minimap_line_lengths: RefCell<Option<(crate::TextValue, Vec<u32>)>>,
    /// 括号配对着色单条缓存（原始值 → 配对/未配对 span 表）。值未变
    /// （纯光标/选区同步）时复用上一次 O(n) 单趟栈扫描结果。存于
    /// `RefCell` 供 `&self` 的 presentation 构建路径读写。
    bracket_color_spans: RefCell<Option<(crate::TextValue, Arc<[(usize, usize, usize)]>)>>,
}

/// Explicit cause passed when marking `INPUT` dirtiness, replacing global mode flags.
#[derive(Clone, Copy)]
enum MarkOrigin {
    General,
    LayoutMove,
    Scroll,
}

/// The sole authoritative retained identity and hierarchy store.
pub struct UiWorld {
    input: input::WorldInputState,
    nodes: NodeStore,
    tree: WorldTree,
    invalidation: Box<WorldInvalidation>,
    motion: Box<WorldMotion>,
    text: Box<WorldText>,
    hit_index: WorldHitIndex,
    scroll: WorldScroll,
    overlays_a11y: Box<WorldOverlaysA11y>,
    /// Bumped on `SetTheme`; extract and reactive theme checks read this directly.
    pub(crate) palette_epoch: u64,
    /// Shared text engine installed by host formatting/shaping for rich-text painters.
    pub(crate) paint_text_engine: Option<nana_text::SharedTextEngine>,
    /// Markdown diagrams recorded against the theme, keyed by palette epoch.
    #[cfg(feature = "rich-text")]
    pub(crate) diagram_recordings: RefCell<Vec<(u64, crate::markdown_diagram::DiagramDrawing)>>,
}

const _: () = assert!(std::mem::size_of::<UiWorld>() < 4096);

impl Default for UiWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl UiWorld {
    pub(crate) fn take_window_cursor_dirty(&mut self) -> bool {
        std::mem::take(&mut self.invalidation.cursor_style_dirty)
    }

    pub fn new() -> Self {
        Self {
            input: input::WorldInputState::default(),
            nodes: NodeStore::new(),
            tree: WorldTree::default(),
            invalidation: Box::default(),
            motion: Box::default(),
            text: Box::default(),
            hit_index: WorldHitIndex::default(),
            scroll: WorldScroll::default(),
            overlays_a11y: Box::default(),
            palette_epoch: 1,
            paint_text_engine: None,
            #[cfg(feature = "rich-text")]
            diagram_recordings: RefCell::new(Vec::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn contains(&self, id: StableNodeId) -> bool {
        self.nodes.contains(id)
    }

    /// Hide the carets of `carets` for the blink, showing any other one it
    /// hid. Each editor that changes is drawn again; no text or layout work.
    pub(crate) fn set_hidden_carets(&mut self, carets: &[StableNodeId]) {
        if self.text.hidden_carets == carets {
            return;
        }
        let previous = std::mem::replace(&mut self.text.hidden_carets, carets.to_vec());
        for id in previous.into_iter().chain(carets.iter().copied()) {
            if self.nodes.contains(id) {
                self.mark(id, DirtyMask::RENDER);
            }
        }
    }

    pub(crate) fn caret_hidden(&self, id: StableNodeId) -> bool {
        self.text.hidden_carets.contains(&id)
    }

    pub(crate) fn mark_layout(&mut self, id: StableNodeId) {
        if self.nodes.contains(id) {
            self.record_layout_invalidation(
                id,
                LayoutInvalidation::new(
                    LayoutInvalidationSource::Runtime,
                    InvalidationReason::UNKNOWN,
                    InvalidationKind::ALL,
                    LayoutStyleChange::ALL,
                    LayoutDependencyFootprint::ALL,
                ),
            );
        }
    }

    /// Typed layout cause queued for `id` and not yet drained.
    ///
    /// The mutation authority already stored it: `reason` and
    /// `changed_inputs` say why the node was marked, `kind` and
    /// `affected_axes` which dependencies the frontier may follow from it. A
    /// write equal in effect stores nothing, so this is
    /// [`LayoutInvalidation::none`].
    pub fn pending_layout_invalidation(&self, id: StableNodeId) -> LayoutInvalidation {
        self.invalidation
            .pending_layout_invalidations
            .get(&id)
            .copied()
            .unwrap_or_else(LayoutInvalidation::none)
    }

    /// Record a typed layout cause at a mutation boundary. The ordinary dirty
    /// bit schedules the entity for the next drain; this pending queue carries
    /// the layout payload consumed by the frame scheduler.
    ///
    /// Recording is the cause plus the node and its ancestor chain up to a
    /// fixed border. It does not walk descendants: the frontier reaches the
    /// ones that depend on this cause when layout runs, and the pass
    /// republishes their results.
    pub(crate) fn record_layout_invalidation(
        &mut self,
        id: StableNodeId,
        invalidation: LayoutInvalidation,
    ) {
        if !self.nodes.contains(id) {
            return;
        }
        if invalidation.is_empty() {
            self.invalidation
                .counters
                .pending_layout_invalidations_counted
                .layout_invalidations_zero_delta += 1;
            return;
        }
        self.invalidation
            .counters
            .pending_layout_invalidations_counted
            .layout_invalidations_created += 1;
        self.invalidation.layout_seeds_created += 1;
        if invalidation.source == LayoutInvalidationSource::Text {
            self.invalidation
                .counters
                .pending_layout_invalidations_counted
                .text_reflow_seeds += 1;
        }
        self.queue_layout_invalidation(id, invalidation);
        // A fixed border box absorbs an inner metric. Drop that box so a
        // changed child placement can be republished, and leave every
        // ancestor result in place when the border box itself did not change.
        let metric_export = LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE
            .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK)
            .union(LayoutDependencyFootprint::EXPORTS_BASELINE)
            .union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS);
        let changes_border = invalidation
            .kind
            .intersects(InvalidationKind::MEASURE.union(InvalidationKind::TOPOLOGY))
            || invalidation.affected_axes.intersects(metric_export);
        self.suppress_layout_result_chain(id, changes_border);
        let _ = self.mark_keeping_render_reason(
            id,
            crate::schedule::DirtyMask::LAYOUT,
            MarkOrigin::General,
            true,
        );
    }

    /// A virtual list's pass cost `rows`.
    pub(crate) fn note_virtual_rows(&mut self, rows: VirtualRowCounts) {
        let pending = &mut self.invalidation.counters.pending_drain_counts;
        pending.virtual_row_metric_updates += rows.metric_updates;
        pending.virtual_prefix_index_updates += rows.prefix_index_updates;
        pending.virtual_rows_remeasured += rows.rows_remeasured;
        pending.virtual_rows_repositioned += rows.rows_repositioned;
        pending.virtual_logical_rows_scanned += rows.logical_rows_scanned;
        pending.virtual_scroll_extent_updates += rows.scroll_extent_updates;
        pending.virtual_rows_materialized_from_layout += rows.rows_materialized;
    }

    /// The layout seeds queued since `before` were a component state
    /// change's.
    pub(crate) fn note_state_seeds(&mut self, before: u64) {
        let seeds = (self.invalidation.layout_seeds_created - before) as usize;
        self.invalidation
            .counters
            .pending_drain_counts
            .component_state_layout_seeds += seeds;
    }

    /// Queue `invalidation` for `id`, merged into a cause already pending:
    /// a narrower typed seed must not hide a later, wider one. Every cause
    /// enters through here; each caller then sets the layout dirty bit
    /// through [`Self::mark_scroll_compatible`], which moves the input epoch
    /// of the full-layout snapshot.
    fn queue_layout_invalidation(&mut self, id: StableNodeId, invalidation: LayoutInvalidation) {
        self.invalidation
            .pending_layout_invalidations
            .entry(id)
            .and_modify(|previous| *previous = previous.merge(invalidation))
            .or_insert(invalidation);
    }

    /// Hands back seeds a pass of another document was given: they stay
    /// pending, merged into what is queued since, for their own document's
    /// next drain. A node that left the world has nothing left to lay out.
    pub(crate) fn requeue_layout_seeds(
        &mut self,
        seeds: impl IntoIterator<Item = crate::LayoutFrontierSeed>,
    ) {
        for seed in seeds {
            if self.nodes.contains(seed.node) {
                self.queue_layout_invalidation(seed.node, seed.invalidation);
            }
        }
    }

    /// Publish the structural layout cause for a retained parent whose child
    /// list changed. The removed child may leave the live document before the
    /// next drain, so the parent is the durable frontier seed.
    pub(crate) fn record_topology_invalidation(&mut self, id: StableNodeId) {
        self.record_layout_invalidation(
            id,
            LayoutInvalidation::new(
                LayoutInvalidationSource::Structure,
                InvalidationReason::TOPOLOGY,
                InvalidationKind::TOPOLOGY
                    .union(InvalidationKind::MEASURE)
                    .union(InvalidationKind::PLACEMENT),
                LayoutStyleChange::FLOW,
                LayoutDependencyFootprint::ALL,
            ),
        );
    }

    /// Publish that `parent`'s child list changed: it re-measures, rebuilds
    /// its placement and exports its metrics. A child that arrives carries a
    /// topology cause of its own and is laid out in full; the children that
    /// stay keep their content, so this is not a forced subtree. Like an
    /// ancestor on a text's export path, `parent` couples none of its own
    /// siblings: when its box did change, its container's placement moves
    /// the ones after it. Its published result goes, and so do its
    /// ancestors' up to the first fixed border box, which may be `parent`
    /// itself: its children changed, not its border box.
    pub(crate) fn record_child_list_invalidation(&mut self, parent: StableNodeId) {
        self.record_layout_invalidation(
            parent,
            LayoutInvalidation::new(
                LayoutInvalidationSource::Structure,
                InvalidationReason::TOPOLOGY,
                InvalidationKind::TOPOLOGY
                    .union(InvalidationKind::MEASURE)
                    .union(InvalidationKind::PLACEMENT),
                LayoutStyleChange::FLOW,
                LayoutDependencyFootprint::intrinsic_container()
                    .union(LayoutDependencyFootprint::EXPORTS_BASELINE),
            ),
        );
        self.clear_layout_result_chain(parent, false);
    }

    /// Drain typed seeds for one document after the coarse system-work drain.
    /// Entries from other documents stay queued for their own layout pass.
    pub(crate) fn take_layout_frontier_seeds(
        &mut self,
        document: DocumentId,
    ) -> Vec<crate::LayoutFrontierSeed> {
        let mut ids = self
            .invalidation
            .pending_layout_invalidations
            .keys()
            .copied()
            .filter(|id| self.document_of(*id) == Some(document))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        let mut seeds = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(invalidation) = self.invalidation.pending_layout_invalidations.remove(&id) {
                seeds.push(crate::LayoutFrontierSeed::new(id, invalidation));
            }
        }
        seeds
    }

    pub(crate) fn clear_layout_frontier_seeds(&mut self, document: DocumentId) {
        let ids = self
            .invalidation
            .pending_layout_invalidations
            .keys()
            .copied()
            .filter(|id| self.document_of(*id) == Some(document))
            .collect::<Vec<_>>();
        for id in ids {
            self.invalidation.pending_layout_invalidations.remove(&id);
        }
    }

    pub(crate) fn restore_layout_frontier_seeds(&mut self, seeds: &[crate::LayoutFrontierSeed]) {
        for seed in seeds {
            if !self.nodes.contains(seed.node) || seed.invalidation.is_empty() {
                continue;
            }
            self.queue_layout_invalidation(seed.node, seed.invalidation);
            self.invalidate_layout_result(seed.node);
            let _ = self.mark_keeping_render_reason(
                seed.node,
                DirtyMask::LAYOUT,
                MarkOrigin::General,
                true,
            );
        }
    }

    /// Whether the node is `Mounted`. Parked is not; Detach stays mounted but
    /// is omitted from the live document until inserted.
    pub fn is_mounted(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| node.mount == MountState::Mounted)
    }

    pub fn mount_state(&self, id: StableNodeId) -> Option<MountState> {
        self.nodes.get(id).map(|node| node.mount)
    }

    pub fn is_retired(&self, id: StableNodeId) -> bool {
        self.tree.retired.contains(id)
    }

    /// Total IDs retired over this world's lifetime.
    pub fn retired_ids(&self) -> usize {
        self.tree.retired.len()
    }

    /// Coalesced runs backing the retired ledger. Sequential allocation keeps
    /// this near-constant while [`Self::retired_ids`] grows with churn.
    pub fn retired_id_runs(&self) -> usize {
        self.tree.retired.runs()
    }

    pub fn generation(&self) -> u64 {
        self.invalidation.generation
    }

    /// What the framework's own controls say: the title bar's buttons, a
    /// dialog's confirm and cancel, the built-in settings page. Controls
    /// read it when they are projected or assembled.
    pub fn framework_strings(&self) -> &nana_ui_core::FrameworkStrings {
        &self.invalidation.framework_strings
    }

    pub(crate) fn set_framework_strings(&mut self, strings: Arc<nana_ui_core::FrameworkStrings>) {
        self.invalidation.framework_strings = strings;
    }

    /// Whether the active host advertises native screen color sampling.
    pub const fn eyedropper_supported(&self) -> bool {
        self.motion.eyedropper_supported
    }

    /// Records host screen color sampling capability, returning `true` when it changed.
    pub fn set_eyedropper_supported(&mut self, supported: bool) -> bool {
        if self.motion.eyedropper_supported == supported {
            return false;
        }
        self.motion.eyedropper_supported = supported;
        true
    }

    pub(crate) fn layout_source_epoch(&self) -> u64 {
        self.invalidation.layout_source_epoch
    }

    pub(crate) fn note_layout_source_change(&mut self) {
        self.invalidation.layout_source_epoch =
            self.invalidation.layout_source_epoch.wrapping_add(1);
    }

    /// Layout-length animations change used sizes without a style write,
    /// and an open triggered menu places its items where its trigger shows
    /// through scroll offsets and transforms (z-index nodes count those
    /// menus). A viewport snapshot must not answer those frames.
    pub(crate) fn layout_source_reusable(&self) -> bool {
        !self.has_layout_length_overlays() && self.invalidation.z_index_nodes == 0
    }

    /// Revision of the last published canonical layout snapshot. Unlike the
    /// world mutation generation this does not advance for paint, hover,
    /// transform or input-only work.
    pub fn layout_generation(&self) -> u64 {
        self.invalidation.layout_generation
    }

    pub(crate) fn published_layout_meta(&self, id: StableNodeId) -> Option<&PublishedLayoutMeta> {
        self.nodes.published_layout(id)
    }

    pub(crate) fn has_local_layout_result(&self, id: StableNodeId) -> bool {
        self.published_layout_meta(id).is_some()
    }

    pub(crate) fn sticky_boxes(&self, id: StableNodeId) -> Option<crate::StickyBoxes> {
        self.published_layout_meta(id)
            .map(PublishedLayoutMeta::sticky_boxes)
    }

    pub(crate) fn extracted_layout(&self, id: StableNodeId) -> Option<crate::ExtractedLayout> {
        let scroll_metrics = self.nodes.scroll_metrics(id).copied();
        self.published_layout_meta(id)
            .map(|meta| meta.extracted_layout(scroll_metrics))
    }

    /// Canonical geometry for `id`, if the node has been laid out, on the
    /// page: its result placed where its box stands. [`Self::local_layout_result`]
    /// is the result in `id`'s own coordinates.
    pub fn layout_result(&self, id: StableNodeId) -> Option<Arc<crate::LayoutResult>> {
        let meta = self.published_layout_meta(id)?;
        let origin = self.layout_origin(id)?;
        let local = self.reconstruct_local_layout_result(id, meta);
        Some(Arc::new(
            self.place_layout_result_on_page(id, local, origin),
        ))
    }

    /// `id`'s published result in its own coordinates: its border box at the
    /// origin. A box that only moved keeps it.
    pub fn local_layout_result(&self, id: StableNodeId) -> Option<Arc<crate::LayoutResult>> {
        let meta = self.published_layout_meta(id)?;
        Some(Arc::new(self.reconstruct_local_layout_result(id, meta)))
    }

    fn reconstruct_local_layout_result(
        &self,
        id: StableNodeId,
        meta: &PublishedLayoutMeta,
    ) -> crate::LayoutResult {
        let bounds = meta.bounds();
        let scroll_offset = self.nodes.scroll_offset(id);
        let scroll_metrics = self.nodes.scroll_metrics(id).copied();
        let (last_baseline, scroll_extent, fragments, parts) = if let Some(ext) = meta.ext.as_ref()
        {
            (
                ext.last_baseline,
                ext.scroll_extent,
                Arc::clone(&ext.fragments),
                Arc::clone(&ext.parts),
            )
        } else if meta.single_text_source != TEXT_SOURCE_NONE_SENTINEL {
            let text_box = meta.inline_text_box();
            let source = meta.single_text_range();
            let first_baseline = meta.first_baseline();
            let mut line =
                crate::LayoutFragment::new(crate::LayoutFragmentKind::TextLine, text_box);
            line.first_baseline = first_baseline;
            line.last_baseline = first_baseline;
            line.source = source.clone();
            let mut run = crate::LayoutFragment::new(crate::LayoutFragmentKind::TextRun, text_box);
            run.first_baseline = first_baseline;
            run.last_baseline = first_baseline;
            run.source = source;
            (
                first_baseline,
                meta.overflow_box(),
                Arc::from([line, run]),
                Arc::from([
                    crate::LayoutPart::new(crate::LayoutPartKind::TextContent, text_box),
                    crate::LayoutPart::new(
                        crate::LayoutPartKind::ComponentContent,
                        meta.content_box,
                    ),
                ]),
            )
        } else {
            (
                meta.first_baseline(),
                meta.overflow_box(),
                Arc::default(),
                Arc::from([crate::LayoutPart::new(
                    crate::LayoutPartKind::ComponentContent,
                    meta.content_box,
                )]),
            )
        };
        let mut deps = Vec::new();
        if let Some(node) = self.nodes.get(id) {
            deps.extend_from_slice(&node.hierarchy.children);
        }
        let clip = meta.clip();
        let clip_parent = meta.clip_parent();
        let containing_block = meta.containing_block();
        if let Some(clip) = clip {
            deps.push(clip);
        }
        if let Some(cb) = containing_block {
            deps.push(cb);
        }
        deps.sort_unstable();
        deps.dedup();
        crate::LayoutResult {
            bounds,
            border_box: bounds,
            padding_box: meta.padding_box(),
            content_box: meta.content_box,
            first_baseline: meta.first_baseline(),
            last_baseline,
            overflow: meta.overflow_box(),
            scroll_extent,
            scroll_offset,
            scroll_metrics,
            fragments,
            parts,
            clip,
            clip_parent,
            containing_block,
            dependencies: shared_slice(&deps),
            formatting_context: meta.formatting_context,
            participation: meta.participation,
            dependency_generation: meta.generation,
            generation: meta.generation,
            source: meta.source,
        }
    }

    /// Where each of `id`'s children sits in `id`'s formatting context, in
    /// child order and on the page: each child's box where its frame
    /// stands. `None` when `id` has no published result.
    ///
    /// A result does not hold these (a child's box is the child's, and a
    /// parent holding a copy was rebuilt for every child that moved); they
    /// are read from the children when asked, so asking costs a walk of
    /// `id`'s children.
    pub fn layout_child_placements(
        &self,
        id: StableNodeId,
    ) -> Option<Vec<crate::LayoutChildPlacement>> {
        if !self.has_local_layout_result(id) {
            return None;
        }
        let origin = self.layout_origin(id)?;
        Some(
            self.child_layout_boxes(id, Some(origin))
                .map(|(index, node, bounds)| crate::LayoutChildPlacement {
                    node,
                    bounds,
                    index,
                })
                .collect(),
        )
    }

    /// [`Self::layout_child_placements`] from `id`'s border-box origin: each
    /// child's local box, measured from its frame.
    pub fn local_layout_child_placements(
        &self,
        id: StableNodeId,
    ) -> Option<Vec<crate::LayoutChildPlacement>> {
        if !self.has_local_layout_result(id) {
            return None;
        }
        Some(
            self.child_layout_boxes(id, None)
                .map(|(index, node, bounds)| crate::LayoutChildPlacement {
                    node,
                    bounds,
                    index,
                })
                .collect(),
        )
    }

    /// One fragment for each of `id`'s children, on the page: the child's
    /// box as a fragment of the kind `id`'s context gives its children
    /// (Issue #197), with the source bytes of an inline atom. `None` when
    /// `id` has no published result. Read from the children when asked, as
    /// [`Self::layout_child_placements`] is.
    pub fn layout_child_fragments(&self, id: StableNodeId) -> Option<Vec<crate::LayoutFragment>> {
        if !self.has_local_layout_result(id) {
            return None;
        }
        let origin = self.layout_origin(id)?;
        Some(self.child_fragments(id, Some(origin)))
    }

    /// [`Self::layout_child_fragments`] from `id`'s border-box origin.
    pub fn local_layout_child_fragments(
        &self,
        id: StableNodeId,
    ) -> Option<Vec<crate::LayoutFragment>> {
        if !self.has_local_layout_result(id) {
            return None;
        }
        Some(self.child_fragments(id, None))
    }

    fn child_fragments(
        &self,
        id: StableNodeId,
        origin: Option<[f32; 2]>,
    ) -> Vec<crate::LayoutFragment> {
        let Some(node) = self.nodes.get(id) else {
            return Vec::new();
        };
        let kind = child_fragment_kind(node);
        let atoms = self.inline_record(id).map(|content| &content.atoms);
        self.child_layout_boxes(id, origin)
            .map(|(index, child, bounds)| {
                let mut fragment = crate::LayoutFragment::for_node(kind, child, bounds);
                fragment.index = index;
                fragment.source = atoms.and_then(|atoms| {
                    atoms
                        .iter()
                        .find(|atom| atom.node == child)
                        .map(|atom| atom.offset..atom.offset + '\u{fffc}'.len_utf8())
                });
                fragment
            })
            .collect()
    }

    /// `id`'s children with their index among its children and their box:
    /// as measured from their frame when `origin` is `None`, and on the
    /// page with `id`'s border box at `origin` otherwise.
    fn child_layout_boxes(
        &self,
        id: StableNodeId,
        origin: Option<[f32; 2]>,
    ) -> impl Iterator<Item = (usize, StableNodeId, LayoutBox)> + '_ {
        let children = self
            .nodes
            .get(id)
            .map_or(&[][..], |node| node.hierarchy.children.as_ref());
        children
            .iter()
            .enumerate()
            .filter_map(move |(index, &child)| {
                let record = self.nodes.get(child)?;
                let local = self.nodes.local_layout(child);
                let bounds = match origin {
                    None => local,
                    Some(origin) => {
                        let frame = self.nodes.layout_frame(child);
                        let at = if frame == crate::LayoutFrame::Parent {
                            origin
                        } else {
                            self.frame_origin_of(record, frame)
                        };
                        crate::layout_result::place_box(at, local)
                    }
                };
                Some((index, child, bounds))
            })
    }

    fn place_layout_result_on_page(
        &self,
        id: StableNodeId,
        mut local: crate::LayoutResult,
        origin: [f32; 2],
    ) -> crate::LayoutResult {
        let local_has_scroll_metrics = local.scroll_metrics.is_some();
        let local_fragments = std::mem::take(&mut local.fragments);
        let local_parts = std::mem::take(&mut local.parts);
        let mut result = local.placed_frame(origin);
        result.bounds.x = origin[0];
        result.bounds.y = origin[1];
        result.border_box.x = origin[0];
        result.border_box.y = origin[1];
        let mut escaping = None;
        let node = self.nodes.get(id);
        let joins_every_child = node.is_some_and(|node| !node.resolved_layout.clips_overflow())
            && !local_has_scroll_metrics;
        for &child in node.map_or(&[][..], |node| node.hierarchy.children.as_ref()) {
            let Some(record) = self.nodes.get(child) else {
                continue;
            };
            let frame = self.nodes.layout_frame(child);
            if frame == crate::LayoutFrame::Parent {
                continue;
            }
            if joins_every_child || self.clip_parent_of(child) == Some(id) {
                let at = self.frame_origin_of(record, frame);
                let bounds = crate::layout_result::place_box(at, self.nodes.local_layout(child));
                escaping = Some(escaping.map_or(bounds, |all| union_layout_boxes(all, bounds)));
            }
        }
        result.fragments = local_fragments
            .iter()
            .map(|fragment| crate::LayoutResult::place_fragment(origin, fragment))
            .collect();
        result.parts = local_parts
            .iter()
            .map(|part| crate::LayoutResult::place_part(origin, part))
            .collect();
        if let Some(escaping) = escaping {
            result.overflow = union_layout_boxes(result.overflow, escaping);
            if !local_has_scroll_metrics {
                result.scroll_extent = result.overflow;
            }
        }
        result
    }

    /// `child`'s box measured from `owner`'s origin: its local box when it is
    /// measured from there, and otherwise the two apart on the page.
    pub(crate) fn layout_box_from(
        &self,
        owner: StableNodeId,
        child: StableNodeId,
    ) -> Option<LayoutBox> {
        let record = self.nodes.get(child)?;
        let local = self.nodes.local_layout(child);
        let from_owner = match self.nodes.layout_frame(child) {
            crate::LayoutFrame::Parent => record.hierarchy.parent == Some(owner),
            crate::LayoutFrame::Block(block) => block == owner,
            crate::LayoutFrame::Page => false,
        };
        if from_owner {
            return Some(local);
        }
        let [x, y] = self.record_layout_origin(child, record);
        let [owner_x, owner_y] = self.layout_origin(owner)?;
        Some(LayoutBox {
            x: x - owner_x,
            y: y - owner_y,
            ..local
        })
    }

    pub(crate) fn slot_box_in_frame(
        &self,
        owner: StableNodeId,
        slot: StableNodeId,
        own_frame: bool,
    ) -> Option<LayoutBox> {
        if own_frame {
            self.layout_box_from(owner, slot)
        } else {
            self.component_layout_box(slot)
        }
    }

    pub(crate) fn clear_layout_results_subtree(&mut self, root: StableNodeId) {
        if !self.nodes.has_published_layouts() {
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self
                .nodes
                .get(id)
                .map(|node| node.hierarchy.children.clone())
                .unwrap_or_default();
            self.nodes.clear_published_layout(id);
            stack.extend(children.iter().copied());
        }
    }

    /// Drop `start` and ancestors whose placement depends on it.
    fn clear_layout_result_chain(&mut self, start: StableNodeId, start_changes_border: bool) {
        if !self.nodes.has_published_layouts() {
            return;
        }
        self.walk_layout_result_chain(start, start_changes_border, ResultChainWalk::Clear);
    }

    /// Walk from `start` up its ancestors, applying `walk` to each, to the
    /// first that absorbs the change: a fixed metric boundary
    /// ([`Self::is_fixed_metric_boundary`]), past `start` itself when
    /// `start_changes_border`.
    fn walk_layout_result_chain(
        &mut self,
        start: StableNodeId,
        start_changes_border: bool,
        walk: ResultChainWalk,
    ) {
        let mut current = Some(start);
        while let Some(node) = current {
            let boundary = self.is_fixed_metric_boundary(node);
            match walk {
                ResultChainWalk::Clear => self.nodes.clear_published_layout(node),
                ResultChainWalk::Suppress => self.suppress_layout_result(node),
            }
            let absorbs = boundary && !(node == start && start_changes_border);
            if absorbs {
                break;
            }
            current = self.parent_id(node);
        }
    }

    /// A px border box with no min/max does not export a child's intrinsic
    /// size. Baseline alignment is the exception: the parent's line still
    /// moves when the child's baseline moves -- under `align-items:
    /// baseline`, and for an atom on a line of an ordinary block.
    fn is_fixed_metric_boundary(&self, id: StableNodeId) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        if !self.fixed_border(id, record) || self.atom_baseline_reads_content(id) {
            return false;
        }
        if let Some(parent) = record.hierarchy.parent
            && let Some(parent_record) = self.nodes.get(parent)
        {
            let align = record
                .resolved_layout
                .resolved_align_self(parent_record.resolved_layout.child_align_items());
            if align == nana_ui_core::AlignSpec::Baseline {
                return false;
            }
        }
        true
    }

    fn suppress_layout_result(&mut self, id: StableNodeId) {
        self.nodes.suppress_published_layout(id);
    }

    pub(crate) fn suppress_layout_results_subtree(&mut self, root: StableNodeId) {
        if !self.nodes.has_published_layouts() {
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self
                .nodes
                .get(id)
                .map(|node| node.hierarchy.children.clone())
                .unwrap_or_default();
            self.suppress_layout_result(id);
            stack.extend(children.iter().copied());
        }
    }

    pub(crate) fn suppress_layout_result_chain(
        &mut self,
        start: StableNodeId,
        start_changes_border: bool,
    ) {
        if !self.nodes.has_published_layouts() {
            return;
        }
        self.walk_layout_result_chain(start, start_changes_border, ResultChainWalk::Suppress);
    }

    /// Invalidate a node's result and every result whose placement or clip
    /// chain depends on it. The next layout commit publishes a fresh batch.
    pub(crate) fn invalidate_layout_result(&mut self, id: StableNodeId) {
        self.clear_layout_results_subtree(id);
        self.clear_layout_result_chain(id, true);
    }

    /// Publish a coherent batch of layout results after all box writes in a
    /// pass have committed. The batch gets one shared generation so callers
    /// can correlate render, input and accessibility projections.
    pub(crate) fn publish_layout_results(
        &mut self,
        ids: &[StableNodeId],
        source: crate::LayoutResultSource,
    ) {
        if ids.is_empty() && self.invalidation.layout_anchor_roots.is_empty() {
            // Nothing to publish, nothing anchored: the walk below would
            // record no counts and mark nothing.
            return;
        }
        // A first publish builds every result, and each settles its pair in
        // the memo. A later one rebuilds few of the nodes it covers.
        let mut memo = crate::NodeMap::default();
        if !self.nodes.has_published_layouts() {
            memo.reserve(ids.len());
        }
        *self.invalidation.anchor_memo.get_mut() = Some(memo);
        self.publish_layout_results_anchored(ids, source);
        *self.invalidation.anchor_memo.get_mut() = None;
    }

    fn publish_layout_results_anchored(
        &mut self,
        ids: &[StableNodeId],
        source: crate::LayoutResultSource,
    ) {
        let anchored = self.take_moved_layout_anchors();
        let ids: Vec<StableNodeId> = if anchored.is_empty() {
            ids.to_vec()
        } else {
            ids.iter().copied().chain(anchored).collect()
        };
        let ids = ids.as_slice();
        let mut seen: NodeSet = ids.iter().copied().collect();
        let mut unique: Vec<StableNodeId> = ids.to_vec();
        for &id in ids {
            let mut ancestor = self.parent_id(id);
            while let Some(parent) = ancestor {
                if !seen.insert(parent) {
                    break;
                }
                unique.push(parent);
                ancestor = self.parent_id(parent);
            }
        }
        unique.sort();
        unique.dedup();
        let boxless = self.boxless_nodes(&unique);
        let mut requested = ids.to_vec();
        requested.sort_unstable();
        requested.dedup();
        let first_publish = !self.nodes.has_published_layouts();
        let mut children_visited = if first_publish {
            unique
                .iter()
                .filter(|&&id| self.parent_id(id).is_some())
                .count()
        } else {
            0
        };
        #[cfg(feature = "benchmark")]
        let mut phase = crate::layout_engine::plan_stats::PhaseClock::start();
        let mut built = Vec::with_capacity(unique.len());
        let mut scratch = ResultScratch::default();
        let mut reused = 0usize;
        for &id in &unique {
            if boxless.contains(&id) {
                self.nodes.clear_published_layout(id);
                continue;
            }
            children_visited = children_visited.saturating_add(
                self.nodes
                    .get(id)
                    .map_or(0, |node| node.hierarchy.children.len()),
            );
            if let Some(result) = self.build_layout_result(id, source, &mut scratch) {
                built.push((id, result));
            }
        }
        #[cfg(feature = "benchmark")]
        phase.lap(8);
        let mut published = built;
        // Nothing held to compare a first publish's results with.
        published.retain(|(id, result)| {
            if first_publish {
                return true;
            }
            if self
                .nodes
                .geometry_entry(*id)
                .and_then(|g| g.published.as_ref())
                .is_some_and(|previous| previous.geometry_eq(result))
            {
                self.nodes.note_text_fragments(*id);
                self.nodes.unsuppress_published_layout(*id);
                reused = reused.saturating_add(1);
                return false;
            }
            true
        });
        if published.is_empty() {
            self.note_layout_result_publish(reused, 0, 0, children_visited);
            return;
        }
        let changed = published.len();
        if let Some(&(highest, _)) = published.last() {
            self.nodes
                .reserve_geometry_through(highest, self.nodes.len() + changed);
        }
        self.note_layout_result_publish(reused, changed, 1, children_visited);
        self.invalidation.layout_generation = self.invalidation.layout_generation.wrapping_add(1);
        let generation = self.invalidation.layout_generation;
        let mut requested = requested.iter().peekable();
        for (id, mut meta) in published {
            while requested.next_if(|requested| **requested < id).is_some() {}
            let requested_here = requested.peek() == Some(&&id);
            let (part_budget_changed, sticky_changed, project, reshaped) = {
                let previous = self
                    .nodes
                    .geometry_entry(id)
                    .and_then(|g| g.published.as_ref());
                let part_budget_changed = self
                    .nodes
                    .widget_visual(id)
                    .is_some_and(crate::WidgetVisualHandle::tracks_text_content_part_budget)
                    && previous.is_none_or(|previous| {
                        text_content_part_width(previous) != text_content_part_width(&meta)
                    });
                let sticky_changed = self.overlays_a11y.sticky_nodes.has_dependents(id)
                    && previous
                        .is_none_or(|previous| previous.sticky_boxes() != meta.sticky_boxes());
                let project = requested_here
                    && previous.is_none_or(|previous| {
                        layout_result_projects_new_geometry(previous, &meta)
                    });
                let reshaped = project
                    && previous.is_none_or(|previous| layout_result_reshapes(previous, &meta));
                (part_budget_changed, sticky_changed, project, reshaped)
            };
            meta.generation = generation;
            meta.suppressed = false;
            #[cfg(test)]
            {
                self.invalidation.layout_publish_seq =
                    self.invalidation.layout_publish_seq.wrapping_add(1);
                meta.publish_seq = self.invalidation.layout_publish_seq;
            }
            self.nodes.publish_layout(id, meta);
            self.nodes.note_text_fragments(id);
            if part_budget_changed {
                self.nodes
                    .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
                self.mark(id, DirtyMask::TEXT);
            }
            if project {
                let mut bits = DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY;
                if reshaped {
                    bits |= DirtyMask::RESHAPED;
                }
                self.mark_moved_by_layout(id, bits, false);
            } else {
                // A result rebuilt with nothing new of its own to project
                // (its dependencies or its stamp changed) still replaces the
                // one the record extraction made of it holds: extract it
                // again, or a renderer keeps the old one -- and keeps it under
                // a later move of an ancestor.
                self.mark(id, DirtyMask::RENDER);
            }
            if sticky_changed {
                self.mark(id, DirtyMask::RENDER);
                for sticky in self.overlays_a11y.sticky_nodes.affected(id) {
                    self.mark(
                        sticky,
                        DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
            }
        }
        self.input.focus_nav_epoch = self.input.focus_nav_epoch.wrapping_add(1);
        #[cfg(feature = "benchmark")]
        phase.lap(9);
    }

    fn mark_moved_by_layout(&mut self, id: StableNodeId, bits: u16, moved: bool) {
        if moved {
            self.mark_moved(id, bits);
        } else {
            self.mark_with_origin(id, bits, MarkOrigin::LayoutMove);
        }
    }

    /// Marks `id`'s own scroll: its INPUT mark is the scroll's
    /// ([`SystemWork::accessibility_scrolled`]) unless something else marks
    /// INPUT on it too.
    pub(super) fn mark_scrolled(&mut self, id: StableNodeId) {
        let _ = self.mark_keeping_render_reason(
            id,
            DirtyMask::INPUT | DirtyMask::RENDER,
            MarkOrigin::Scroll,
            true,
        );
    }

    /// Mark `id`, whose own 2D transform changed while what it hands its
    /// children did not ([`SystemWork::transform_kept`]). Its INPUT mark is
    /// no subtree seed for accessibility, as a layout move's is not, unless
    /// something else marks INPUT on it too.
    pub(super) fn mark_transform_kept(&mut self, id: StableNodeId) {
        self.mark_with_origin(
            id,
            DirtyMask::TRANSFORM | DirtyMask::TRANSFORM_KEPT | DirtyMask::INPUT | DirtyMask::RENDER,
            MarkOrigin::LayoutMove,
        );
    }

    /// The nodes of `batch` with no box: ones layout sees hidden (see
    /// [`Self::omits_layout_box`]), and what is inside them. They publish no
    /// result, as a cold layout gives them none, and whatever result one
    /// held before goes (#386). `batch` holds every ancestor of its nodes,
    /// so the walk up stays inside it, and a batch with no node omitting its
    /// own box has none.
    fn boxless_nodes(&self, batch: &[StableNodeId]) -> NodeSet {
        let omits = |id: StableNodeId| self.nodes.contains(id) && self.omits_layout_box(id);
        if !batch.iter().any(|&id| omits(id)) {
            return NodeSet::default();
        }
        let mut memo: NodeMap<bool> = NodeMap::default();
        let mut chain = Vec::new();
        for &id in batch {
            let mut current = Some(id);
            let mut boxless = false;
            while let Some(node) = current {
                if let Some(&known) = memo.get(&node) {
                    boxless = known;
                    break;
                }
                chain.push(node);
                if omits(node) {
                    boxless = true;
                    break;
                }
                current = self.parent_id(node);
            }
            for node in chain.drain(..) {
                memo.insert(node, boxless);
            }
        }
        memo.into_iter()
            .filter_map(|(id, boxless)| boxless.then_some(id))
            .collect()
    }

    fn build_layout_result(
        &self,
        id: StableNodeId,
        source: crate::LayoutResultSource,
        scratch: &mut ResultScratch,
    ) -> Option<PublishedLayoutMeta> {
        let local = self.build_own_layout_result(id, source, scratch)?;
        self.join_layout_children(id, local, scratch)
    }

    /// `id`'s own geometry with its border box at the origin: the boxes,
    /// text fragments, parts and baselines it derives from its own box,
    /// which a move of the box leaves exactly as they were. Nothing here
    /// reads where another box stands; [`Self::join_layout_children`] joins
    /// the children.
    fn build_own_layout_result(
        &self,
        id: StableNodeId,
        source: crate::LayoutResultSource,
        scratch: &mut ResultScratch,
    ) -> Option<OwnLayoutBuild> {
        scratch.clear();
        let ResultScratch {
            own_parts: parts,
            own_fragments: fragments,
            dependencies,
            ..
        } = scratch;
        let node = self.nodes.get(id)?;
        let local = self.nodes.local_layout(id);
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            ..local
        };
        let padding = self.used_layout_padding(id);
        let border = node.resolved_layout.resolved_border_edges();
        let mut result = crate::LayoutResult::from_box(bounds, padding, border);
        let component_parts = self.solve_content_parts_at(id, result.content_box, bounds);
        let text_box =
            crate::layout_engine::parts::primary_text_box(&component_parts, result.content_box);
        result.source = source;
        result.scroll_offset = self.nodes.scroll_offset(id);
        result.formatting_context = established_context(node);
        result.participation = self.layout_node_participation(id);
        let scroll_metrics = self.scroll_metrics(id);
        let fixed = node.resolved_layout.position == nana_ui_core::PositionSpec::Fixed;
        let clips = node.resolved_layout.clips_overflow();
        let mut single_text_source = TEXT_SOURCE_NONE_SENTINEL;
        if record_has_text(node) && self.needs_native_text_fragments(id) {
            let (native, first, last) = self.native_text_fragments(id, text_box)?;
            fragments.extend(native);
            result.first_baseline = first;
            result.last_baseline = last;
            parts.push(crate::LayoutPart::new(
                crate::LayoutPartKind::TextContent,
                text_box,
            ));
        } else if matches!(node.kind.as_ref(), NodeKind::Text) || node.has_text {
            let baseline = self.single_text_baseline(id, text_box);
            let source = self
                .text_layout(id)
                .map(|_| 0..self.nodes.text_content(id).value.len());
            single_text_source = match &source {
                None => TEXT_SOURCE_MISSING,
                Some(range) if range.len() <= TEXT_SOURCE_MAX_LEN => range.len() as u32,
                Some(_) => TEXT_SOURCE_NONE_SENTINEL,
            };
            let mut line =
                crate::LayoutFragment::new(crate::LayoutFragmentKind::TextLine, text_box);
            line.first_baseline = baseline;
            line.last_baseline = baseline;
            line.source = source.clone();
            fragments.push(line);
            let mut run = crate::LayoutFragment::new(crate::LayoutFragmentKind::TextRun, text_box);
            run.first_baseline = baseline;
            run.last_baseline = baseline;
            run.source = source;
            fragments.push(run);
            result.first_baseline = baseline;
            result.last_baseline = baseline;
            parts.push(crate::LayoutPart::new(
                crate::LayoutPartKind::TextContent,
                text_box,
            ));
        }
        // The content-box part follows from the box; the placement puts it
        // in (`content_part_index`).
        // Fixed-position branches are viewport overlays in the retained
        // projection. Keep that semantic part alongside the ordinary
        // content box so Scene, hit testing and accessibility can share
        // the same classification without reopening component geometry.
        if fixed {
            fragments.push(crate::LayoutFragment::new(
                crate::LayoutFragmentKind::Overlay,
                result.bounds,
            ));
            parts.push(crate::LayoutPart::new(
                crate::LayoutPartKind::Overlay,
                result.bounds,
            ));
        }
        if clips {
            fragments.push(crate::LayoutFragment::new(
                crate::LayoutFragmentKind::ScrollViewport,
                result.padding_box,
            ));
            parts.push(crate::LayoutPart::new(
                crate::LayoutPartKind::ScrollViewport,
                result.padding_box,
            ));
        }
        let anchors = self.layout_anchors(id);
        dependencies.extend_from_slice(&node.hierarchy.children);
        if let Some(clip) = anchors.clip {
            dependencies.push(clip);
        }
        if let Some(containing_block) = anchors.containing_block {
            dependencies.push(containing_block);
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        let deps_hash = hash_dependencies(dependencies);
        let extra_start = parts.len();
        parts.extend(
            component_parts
                .into_iter()
                .flatten()
                .filter(|part| part.kind != crate::LayoutPartKind::TextContent),
        );
        let has_extra_parts = parts.len() > extra_start;
        result.scroll_metrics = scroll_metrics;
        result.clip = anchors.clip;
        result.clip_parent = anchors.clip_parent;
        result.containing_block = anchors.containing_block;
        Some(OwnLayoutBuild {
            result,
            deps_hash,
            single_text_source,
            text_box,
            has_extra_parts,
            fixed_or_clips: fixed || clips,
        })
    }

    /// `local`, `id`'s own geometry, joined with its children: the overflow
    /// and scroll extent their boxes make, from `id`'s origin, and its own
    /// fragments and parts. A child measured from another frame counts in
    /// the overflow on the page only ([`Self::layout_result`]): from here it
    /// would move with every box between the two. Where each child sits is
    /// the child's ([`Self::layout_child_placements`]).
    fn join_layout_children(
        &self,
        id: StableNodeId,
        own: OwnLayoutBuild,
        scratch: &mut ResultScratch,
    ) -> Option<PublishedLayoutMeta> {
        let ResultScratch {
            parts,
            own_fragments,
            own_parts,
            shared_parts,
            shared_fragments,
            ..
        } = scratch;
        let OwnLayoutBuild {
            mut result,
            deps_hash,
            single_text_source,
            text_box,
            has_extra_parts,
            fixed_or_clips,
        } = own;
        parts.clear();
        let node = self.nodes.get(id)?;
        let mut overflow = result.bounds;
        // A box that clips or scrolls counts in its overflow only the
        // children its clip and scroll reach: not a `fixed` child, nor an
        // `absolute` one whose containing block is above it (#201). The
        // range of the boxes under them is the content index's, which
        // follows the same chain. Elsewhere overflow joins every child.
        let joins_every_child =
            !node.resolved_layout.clips_overflow() && result.scroll_metrics.is_none();
        for &child in node.hierarchy.children.iter() {
            if self.nodes.contains(child)
                && self.nodes.layout_frame(child) == crate::LayoutFrame::Parent
                && (joins_every_child || self.clip_parent_of(child) == Some(id))
            {
                overflow = union_layout_boxes(overflow, self.nodes.local_layout(child));
            }
        }
        let content = content_part_index(own_parts.as_slice());
        parts.extend(own_parts[..content].iter().copied());
        parts.push(crate::LayoutPart::new(
            crate::LayoutPartKind::ComponentContent,
            result.content_box,
        ));
        parts.extend(own_parts[content..].iter().copied());
        // Only scroll containers need the retained descendant index. Avoid
        // materializing that index for ordinary nodes while still publishing
        // the full scroll extent where a consumer can actually use it.
        if result.scroll_metrics.is_some() {
            let content_extent = self.scroll_content_extent_local(id);
            if content_extent.left.is_finite()
                && content_extent.top.is_finite()
                && content_extent.right.is_finite()
                && content_extent.bottom.is_finite()
            {
                overflow = union_layout_boxes(
                    overflow,
                    LayoutBox {
                        x: content_extent.left,
                        y: content_extent.top,
                        width: (content_extent.right - content_extent.left).max(0.0),
                        height: (content_extent.bottom - content_extent.top).max(0.0),
                    },
                );
            }
        }
        result.overflow = overflow;
        if let Some(metrics) = result.scroll_metrics {
            let scroll_area = LayoutBox {
                x: result.content_box.x + metrics.origin_x.min(0.0),
                y: result.content_box.y + metrics.origin_y.min(0.0),
                width: metrics.content_width.max(result.content_box.width),
                height: metrics.content_height.max(result.content_box.height),
            };
            result.scroll_extent = union_layout_boxes(overflow, scroll_area);
        } else {
            result.scroll_extent = overflow;
        }
        let compact_non_text = single_text_source == TEXT_SOURCE_NONE_SENTINEL
            && own_fragments.is_empty()
            && parts.len() == 1
            && parts[0].kind == crate::LayoutPartKind::ComponentContent
            && parts[0].bounds == result.content_box;
        let text_box_eq_content = text_box == result.content_box;
        let overflow_eq_bounds = result.overflow == result.bounds;
        let compact_single_text = single_text_source != TEXT_SOURCE_NONE_SENTINEL
            && (text_box_eq_content || overflow_eq_bounds)
            && own_fragments.len() == 2
            && parts.len() == 2;
        let can_inline = !has_extra_parts
            && !fixed_or_clips
            && result.last_baseline == result.first_baseline
            && result.scroll_extent == result.overflow
            && (compact_non_text || compact_single_text);
        let (single_text_source, ext, overflow, overflow_is_text_box) = if can_inline {
            let overflow_is_text_box =
                single_text_source != TEXT_SOURCE_NONE_SENTINEL && !text_box_eq_content;
            let overflow = if overflow_is_text_box {
                text_box
            } else {
                result.overflow
            };
            (single_text_source, None, overflow, overflow_is_text_box)
        } else {
            (
                TEXT_SOURCE_NONE_SENTINEL,
                Some(Box::new(PublishedLayoutExt {
                    last_baseline: result.last_baseline,
                    scroll_extent: result.scroll_extent,
                    fragments: shared_fragments.share(own_fragments),
                    parts: shared_parts.share(parts),
                })),
                result.overflow,
                false,
            )
        };
        let rare = (result.padding_box != result.bounds
            || result.clip.is_some()
            || result.clip_parent.is_some()
            || result.containing_block.is_some())
        .then(|| {
            Box::new(PublishedRareGeometry {
                padding_box: result.padding_box,
                clip: result.clip,
                clip_parent: result.clip_parent,
                containing_block: result.containing_block,
            })
        });
        let first_baseline_raw = result
            .first_baseline
            .filter(|value| !value.is_nan())
            .unwrap_or(f32::NAN);
        Some(PublishedLayoutMeta {
            generation: 0,
            deps_hash,
            width: result.bounds.width,
            height: result.bounds.height,
            content_box: result.content_box,
            overflow,
            first_baseline_raw,
            single_text_source,
            rare,
            ext,
            formatting_context: result.formatting_context,
            participation: result.participation,
            source: result.source,
            suppressed: false,
            overflow_is_text_box,
            #[cfg(test)]
            publish_seq: 0,
        })
    }

    /// Notes a node whose resolved layout went from `previous` to `next`
    /// when it started or stopped being a containing block or a clip.
    pub(crate) fn note_layout_anchor_change(
        &mut self,
        id: StableNodeId,
        previous: &nana_ui_core::LayoutStyle,
        next: &nana_ui_core::LayoutStyle,
    ) {
        self.nodes.note_node_space(id);
        // Whether a block holds a strut reads its children's display, and
        // which strut its own face (`resolve_block_struts`).
        let text_ext_diff = !previous.text_ext.ptr_eq(&next.text_ext);
        if previous.display != next.display
            || previous.position != next.position
            || previous.font_weight != next.font_weight
            || previous.font_italic != next.font_italic
            || (text_ext_diff
                && (previous.text_ext.font_family != next.text_ext.font_family
                    || previous.text_ext.font_variation_settings
                        != next.text_ext.font_variation_settings))
        {
            self.text.strut_candidates.push(id);
            if let Some(parent) = self.parent_id(id) {
                self.text.strut_candidates.push(parent);
            }
        }
        // The clip chain reads which of absolute, fixed and in-flow a box
        // is, not only whether it is positioned (#201).
        if previous.position != next.position || previous.clips_overflow() != next.clips_overflow()
        {
            self.invalidation.layout_anchor_roots.insert(id);
            // The scrollable range follows the same chain: the box may move
            // between its parent's area and its containing block's, and the
            // boxes it carries may land here or pass on.
            self.invalidate_scroll_content(id);
            if previous.position.establishes_containing_block()
                != next.position.establishes_containing_block()
            {
                self.scroll.content_bounds.borrow_mut().reland(self, id);
            }
            self.invalidation.layout_reframe_roots.push(id);
            self.scroll.layout_touched = true;
        }
        // So does how it takes part in its parent's context: an inline-block
        // turned block-level is a flow item where its box stays put, and its
        // result, not republished for a box that did not move, would say it
        // is still atomic.
        if previous.display != next.display {
            self.invalidation.layout_anchor_roots.insert(id);
        }
    }

    /// The nodes under a noted anchor change whose published result records
    /// an anchor that is no longer the one above them.
    fn take_moved_layout_anchors(&mut self) -> Vec<StableNodeId> {
        if self.invalidation.layout_anchor_roots.is_empty() {
            return Vec::new();
        }
        let mut roots: Vec<StableNodeId> = self.invalidation.layout_anchor_roots.drain().collect();
        roots.sort_unstable();
        let mut moved = Vec::new();
        let mut stack = Vec::new();
        for root in roots {
            // The root itself: its own clip chain moves with its position,
            // and its own fragments say whether it is a scroll viewport or
            // an overlay.
            if self
                .nodes
                .geometry_entry(root)
                .is_some_and(|g| g.published.is_some())
            {
                moved.push(root);
            }
            stack.extend(
                self.nodes
                    .get(root)
                    .map(|node| node.hierarchy.children.iter().copied())
                    .into_iter()
                    .flatten(),
            );
            while let Some(id) = stack.pop() {
                let Some(node) = self.nodes.get(id) else {
                    continue;
                };
                if let Some(meta) = self
                    .nodes
                    .geometry_entry(id)
                    .and_then(|g| g.published.as_ref())
                    && !self.layout_anchors(id).recorded_in_meta(meta)
                {
                    moved.push(id);
                }
                stack.extend(node.hierarchy.children.iter().copied());
            }
        }
        moved
    }

    /// What `id`'s result records about the boxes around it: the nearest
    /// positioned ancestor, and its clip chain (#201).
    fn layout_anchors(&self, id: StableNodeId) -> LayoutAnchors {
        let AnchorPair {
            clip,
            containing_block,
        } = match self.invalidation.anchor_memo.borrow_mut().as_mut() {
            Some(memo) => self.anchor_pair_memo(id, memo),
            None => AnchorPair {
                clip: self.overflow_clip_of(id),
                containing_block: self.containing_block_of(id),
            },
        };
        LayoutAnchors {
            clip,
            clip_parent: self.clip_parent_of(id),
            containing_block,
        }
    }

    /// The nearest ancestor that establishes a containing block for `id`.
    fn containing_block_of(&self, id: StableNodeId) -> Option<StableNodeId> {
        let mut ancestor = self.parent_id(id);
        while let Some(candidate) = ancestor {
            let record = self.nodes.get(candidate)?;
            if record
                .resolved_layout
                .position
                .establishes_containing_block()
            {
                return Some(candidate);
            }
            ancestor = record.hierarchy.parent;
        }
        None
    }

    /// `id`'s [`AnchorPair`], from `memo` where its ancestors' pairs are.
    ///
    /// Every box in `memo` has its ancestors there too, so the walk up stops
    /// at the first box it finds and settles the rest on the way down, each
    /// from its parent (the containing block) and its clip parent, an
    /// ancestor (the clip).
    fn anchor_pair_memo(
        &self,
        id: StableNodeId,
        memo: &mut crate::NodeMap<AnchorPair>,
    ) -> AnchorPair {
        // `id` itself is held apart from the chain above it, so a node whose
        // parent is settled (every node of a publish walked top-down)
        // collects nothing.
        let mut first = None;
        let mut unsettled = Vec::new();
        let mut cursor = Some(id);
        while let Some(node) = cursor {
            if memo.contains_key(&node) || !self.contains(node) {
                break;
            }
            match first {
                None => first = Some(node),
                Some(_) => unsettled.push(node),
            }
            cursor = self.parent_id(node);
        }
        for node in unsettled.into_iter().rev().chain(first) {
            let containing_block = self.parent_id(node).and_then(|parent| {
                let establishes = self
                    .nodes
                    .get(parent)?
                    .resolved_layout
                    .position
                    .establishes_containing_block();
                if establishes {
                    Some(parent)
                } else {
                    memo.get(&parent).and_then(|pair| pair.containing_block)
                }
            });
            let clip = self.clip_parent_of(node).and_then(|parent| {
                if self.clips_overflow(parent) {
                    Some(parent)
                } else {
                    match memo.get(&parent) {
                        Some(pair) => pair.clip,
                        None => self.overflow_clip_of(parent),
                    }
                }
            });
            memo.insert(
                node,
                AnchorPair {
                    clip,
                    containing_block,
                },
            );
        }
        // Lazily: evaluated eagerly, this fallback walked the whole chain
        // above `id` twice on every call, memo or not.
        memo.get(&id).copied().unwrap_or_else(|| AnchorPair {
            clip: self.overflow_clip_of(id),
            containing_block: self.containing_block_of(id),
        })
    }

    /// Whether any node still owes system work, i.e. whether the next flush has
    /// anything to do. [`Self::take_system_work`] drains this.
    ///
    /// A host that wrote into a document and wants to know whether that write
    /// actually changed anything asks here: an update that projects no
    /// mutations leaves nothing dirty. Answering it from the outside otherwise
    /// means re-deriving per-field diffs the commit already computed.
    pub fn has_pending_work(&self) -> bool {
        // The removal queues are drained by the same call and are refilled
        // independently of the dirty set when a frame does not settle, so
        // reporting only dirty nodes would answer "clean" while work waits.
        !self.invalidation.dirty_entities.is_empty()
            || !self.invalidation.pending_accessibility_removals.is_empty()
            || !self.invalidation.pending_render_removals.is_empty()
    }

    /// Monotonic revision of work scheduled since world creation. Reading it
    /// is O(1) and has no effect on the frame's pending queues.
    pub fn pending_work_revision(&self) -> u64 {
        self.invalidation.pending_work_revision
    }

    /// Algorithm-level counters from the last non-empty drain, or the current
    /// frame accumulator while a product flush is running. An idle
    /// [`Self::take_system_work`] does not replace this snapshot.
    pub fn last_work_counters(&self) -> WorkCounters {
        let mut counters = self.invalidation.counters.last_counters;
        counters.record_hot_path_allocation(
            self.invalidation.pending_hot_allocations.get(),
            self.invalidation.pending_hot_allocated_bytes.get(),
        );
        add_pending_counts(
            &mut counters,
            self.invalidation
                .counters
                .pending_layout_invalidations_counted,
        );
        add_pending_counts(
            &mut counters,
            self.invalidation.counters.pending_drain_counts,
        );
        counters.scroll_content_extent_recomputes += self.pending_scroll_extent_recomputes();
        counters.i18n.accumulate(self.i18n_gauges());
        counters
    }

    pub fn record_presentation_gpu_work(&mut self, work: Option<nana_ui_core::GpuWorkObservation>) {
        let mut observed = WorkCounters::default();
        if let Some(work) = work {
            observed.record_gpu_work(work);
        }
        self.invalidation.counters.last_counters.batch_rebuilds = observed.batch_rebuilds;
        self.invalidation.counters.last_counters.draw_batches = observed.draw_batches;
        self.invalidation.counters.last_counters.draw_calls = observed.draw_calls;
        self.invalidation.counters.last_counters.gpu_upload_bytes = observed.gpu_upload_bytes;
        self.invalidation
            .counters
            .last_counters
            .gpu_buffer_reallocations = observed.gpu_buffer_reallocations;
    }

    pub fn record_output_work(&mut self, work: nana_ui_core::OutputWorkObservation) {
        let mut observed = WorkCounters::default();
        observed.record_output_work(work);
        macro_rules! replace { ($($field:ident),* $(,)?) => { $(self.invalidation.counters.last_counters.$field = observed.$field;)* }; }
        replace!(
            output_extra_passes,
            output_gpu_copies,
            output_target_recreates,
            output_content_revisions,
            output_idle_reuse_frames,
            output_resolve_count,
            output_gpu_copy_bytes,
            output_gpu_convert_passes,
            output_plan_rebuilds,
            output_ui_renders,
            output_submissions,
            output_cpu_readbacks,
            output_deferred
        );
    }

    /// Start a multi-pass frame accumulator. Idle drains still leave the
    /// previous snapshot in place until a non-empty pass runs.
    pub fn begin_frame(&mut self) {
        self.begin_frame_counters();
    }

    pub fn begin_frame_counters(&mut self) {
        self.begin_responsive_frame();
        self.commit_pending_hot_allocs();
        self.commit_pending_layout_invalidation_counts();
        self.commit_pending_theme_work();
        self.invalidation.counters.frame_counters = WorkCounters::default();
        self.text.text_frame_work = nana_text::TextWorkCounters::default();
        self.invalidation.theme_state.theme_frame_work = ThemeWorkCounters::default();
        self.invalidation.frame_extracted_nodes = 0;
        self.invalidation.frame_extracted_spans = 0;
        self.invalidation.accumulating_frame = true;
    }

    pub fn end_frame(&mut self) {
        self.end_frame_counters();
    }

    pub fn end_frame_counters(&mut self) {
        self.invalidation.accumulating_frame = false;
    }

    /// Record extract output onto the last drained work counters. Draw batches
    /// and GPU upload bytes are omitted; extraction does not measure them.
    pub fn record_extract(&mut self, extracted: &[ExtractedNode]) {
        let spans = extracted.iter().map(|node| node.text_spans.len()).sum();
        self.record_extracted(extracted.len(), spans);
    }

    /// [`Self::record_extract`] for what
    /// [`Self::extract_nodes_reusing`] gave: a move counts as an extracted
    /// node, and its spans are the ones the renderer already holds.
    pub fn record_extract_reusing(&mut self, extracted: &[ExtractedNode], moved: &[ExtractedMove]) {
        let spans = extracted.iter().map(|node| node.text_spans.len()).sum();
        self.record_extracted(extracted.len() + moved.len(), spans);
    }

    fn record_extracted(&mut self, nodes: usize, spans: usize) {
        if nodes > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::SCENE_NODES_EXTRACTED,
                nodes
            );
        }
        if self.invalidation.accumulating_frame {
            self.invalidation.frame_extracted_nodes = self
                .invalidation
                .frame_extracted_nodes
                .saturating_add(nodes);
            self.invalidation.frame_extracted_spans = self
                .invalidation
                .frame_extracted_spans
                .saturating_add(spans);
            self.invalidation
                .counters
                .last_counters
                .render_nodes_extracted = self.invalidation.frame_extracted_nodes;
            self.invalidation
                .counters
                .last_counters
                .extracted_text_spans = self.invalidation.frame_extracted_spans;
        } else {
            self.invalidation
                .counters
                .last_counters
                .render_nodes_extracted = nodes;
            self.invalidation
                .counters
                .last_counters
                .extracted_text_spans = spans;
        }
    }

    /// Observe a CPU hot-path heap event from a `&self` path (layout inputs,
    /// document order). Folded into [`Self::last_work_counters`].
    pub fn record_hot_path_allocation(&self, count: usize, bytes: usize) {
        if count == 0 && bytes == 0 {
            return;
        }
        self.invalidation.pending_hot_allocations.set(
            self.invalidation
                .pending_hot_allocations
                .get()
                .saturating_add(count),
        );
        self.invalidation.pending_hot_allocated_bytes.set(
            self.invalidation
                .pending_hot_allocated_bytes
                .get()
                .saturating_add(bytes),
        );
    }

    fn commit_pending_hot_allocs(&mut self) {
        let count = self.invalidation.pending_hot_allocations.replace(0);
        let bytes = self.invalidation.pending_hot_allocated_bytes.replace(0);
        self.invalidation
            .counters
            .last_counters
            .record_hot_path_allocation(count, bytes);
        if self.invalidation.accumulating_frame {
            self.invalidation
                .counters
                .frame_counters
                .record_hot_path_allocation(count, bytes);
        }
    }

    fn commit_pending_layout_invalidation_counts(&mut self) {
        let counted = std::mem::take(
            &mut self
                .invalidation
                .counters
                .pending_layout_invalidations_counted,
        );
        self.bump_last_counters(|counters| add_pending_counts(counters, counted));
    }

    /// Text work (Issue #95) of the last shaping pass, or of every pass of
    /// the last frame [`Self::begin_frame_counters`] accumulated that ran one: nodes
    /// considered, skipped on revision alone, shaped, and what the passes
    /// cloned, hashed and looked up to get there.
    pub fn last_text_work_counters(&self) -> nana_text::TextWorkCounters {
        self.text.text_work
    }

    /// Theme/style work (Issue #101 §4) of the last style pass or theme
    /// install, or of every pass of the last frame
    /// [`Self::begin_frame_counters`] accumulated that ran one: nodes
    /// considered, resolved and skipped, token reads, and what the pass cost
    /// Layout, Text and Paint.
    ///
    /// An idle frame leaves the previous snapshot in place, like
    /// [`Self::last_work_counters`].
    /// Drain whether style resolution touched a hit-test-relevant property (#106).
    pub fn take_style_hit_changed(&mut self) -> bool {
        std::mem::take(&mut self.invalidation.style_hit_changed)
    }

    /// Current per-category theme revisions (#106).
    pub fn theme_generations(&self) -> nana_ui_core::ThemeCategoryGenerations {
        self.invalidation.theme_state.theme_generations
    }

    /// Current computed theme dependency classes for `id` (#106).
    pub fn node_theme_dependency(&self, id: StableNodeId) -> nana_ui_core::ThemeDependencyMask {
        self.nodes.theme_dependency(id)
    }

    pub fn last_theme_work_counters(&self) -> ThemeWorkCounters {
        let mut counters = self.invalidation.theme_state.theme_work;
        counters.accumulate(self.pending_theme_work());
        counters
    }

    /// The theme/style work observed since the last fold, as counters.
    fn pending_theme_work(&self) -> ThemeWorkCounters {
        let mut pending = self.invalidation.theme_state.pending_scope_theme_work;
        pending.record_theme_reads(self.invalidation.pending_theme_reads.get());
        pending.record_layout_copy(
            self.invalidation.pending_layout_copies.get(),
            self.invalidation
                .pending_layout_copies
                .get()
                .saturating_mul(size_of::<nana_ui_core::LayoutStyle>()),
        );
        pending
    }

    /// Take that work, leaving nothing behind for the next window.
    fn take_pending_theme_work(&mut self) -> ThemeWorkCounters {
        let mut pending =
            std::mem::take(&mut self.invalidation.theme_state.pending_scope_theme_work);
        let reads = self.invalidation.pending_theme_reads.replace(0);
        let copies = self.invalidation.pending_layout_copies.replace(0);
        pending.record_theme_reads(reads);
        pending.record_layout_copy(
            copies,
            copies.saturating_mul(size_of::<nana_ui_core::LayoutStyle>()),
        );
        pending
    }

    pub(crate) fn take_scope_reprojects(
        &mut self,
    ) -> Vec<(StableNodeId, nana_ui_core::ThemeDependencyMask)> {
        std::mem::take(&mut self.invalidation.theme_state.pending_scope_reprojects)
    }

    /// Observe one read of the token authority from a `&self` palette path.
    fn record_theme_read(&self) {
        self.invalidation.pending_theme_reads.set(
            self.invalidation
                .pending_theme_reads
                .get()
                .saturating_add(1),
        );
    }

    /// Observe the `LayoutStyle` a style write had to copy to hold the
    /// resolved value beside the authored intent.
    fn record_resolved_layout_copy(&self) {
        self.invalidation.pending_layout_copies.set(
            self.invalidation
                .pending_layout_copies
                .get()
                .saturating_add(1),
        );
    }

    /// Close the pending work onto the window it was observed in.
    fn commit_pending_theme_work(&mut self) {
        let pending = self.take_pending_theme_work();
        if pending == ThemeWorkCounters::default() {
            return;
        }
        self.invalidation.theme_state.theme_work.accumulate(pending);
        if self.invalidation.accumulating_frame {
            self.invalidation
                .theme_state
                .theme_frame_work
                .accumulate(pending);
        }
    }

    fn record_theme_work(&mut self, mut work: ThemeWorkCounters) {
        work.theme_scopes_active = work
            .theme_scopes_active
            .max(self.invalidation.theme_state.theme_scopes.len());
        if work.style_nodes_resolved > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::STYLE_NODES_RESOLVED,
                work.style_nodes_resolved
            );
        }
        if work.theme_dependency_skipped > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::THEME_DEPENDENCY_SKIPPED,
                work.theme_dependency_skipped
            );
        }
        if work.resolved_style_unchanged > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::RESOLVED_STYLE_UNCHANGED,
                work.resolved_style_unchanged
            );
        }
        if work.style_motion_policy_invalidations > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::THEME_MOTION_POLICY_INVALIDATIONS,
                work.style_motion_policy_invalidations
            );
        }
        if work.scope_cache_hits > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::THEME_SCOPE_CACHE_HITS,
                work.scope_cache_hits
            );
        }
        if work.scope_cache_misses > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::THEME_SCOPE_CACHE_MISSES,
                work.scope_cache_misses
            );
        }
        work.accumulate(self.take_pending_theme_work());
        if self.invalidation.accumulating_frame {
            self.invalidation
                .theme_state
                .theme_frame_work
                .accumulate(work);
            self.invalidation.theme_state.theme_work =
                self.invalidation.theme_state.theme_frame_work;
        } else {
            self.invalidation.theme_state.theme_work = work;
        }
    }

    fn record_text_work(&mut self, mut work: nana_text::TextWorkCounters) {
        work.accumulate(std::mem::take(&mut self.text.pending_edit_work));
        // The CJK line decision is Dynamic Layout's text side (Issue #211).
        if work.line_opportunities_considered > 0 || work.line_break_comparisons > 0 {
            self.bump_last_counters(|counters| {
                let dynamic = &mut counters.dynamic;
                dynamic.opportunities_considered += work.line_opportunities_considered;
                dynamic.break_comparisons += work.line_break_comparisons;
                dynamic.beam_states = dynamic.beam_states.max(work.line_beam_states);
                dynamic.budget_fallbacks += work.line_budget_fallbacks;
            });
        }
        if self.invalidation.accumulating_frame {
            // Published as the frame goes, so an idle frame (no text pass)
            // leaves the last frame that had one in place, as
            // `last_work_counters` does.
            self.text.text_frame_work.accumulate(work);
            self.text.text_work = self.text.text_frame_work;
        } else {
            self.text.text_work = work;
        }
    }

    fn bump_last_counters(&mut self, update: impl Fn(&mut WorkCounters)) {
        update(&mut self.invalidation.counters.last_counters);
        if self.invalidation.accumulating_frame {
            update(&mut self.invalidation.counters.frame_counters);
        }
    }

    pub(crate) fn record_intrinsic_measure_counters(
        &mut self,
        counters: crate::IntrinsicCacheCounters,
    ) {
        self.bump_last_counters(|work| {
            work.record_intrinsic_measure(
                counters.intrinsic_measure_requests,
                counters.intrinsic_measure_cache_hits,
                counters.intrinsic_measure_cache_misses,
                counters.intrinsic_measure_full_subtrees,
                counters.generation_bumps,
                counters.baseline_queries,
                counters.cross_context_hits,
                counters.cross_context_misses,
            );
        });
    }

    /// Fold structural layout-frontier observations from the retained engine
    /// into the same frame counters as the coarse dirty drain.
    pub(crate) fn record_layout_execution(
        &mut self,
        stats: crate::layout_engine::LayoutExecutionStats,
    ) {
        self.bump_last_counters(|counters| {
            counters.record_layout_execution(
                stats.measure_nodes,
                stats.measure_cache_hits,
                stats.measure_cache_misses,
                stats.placement_nodes,
                stats.origin_only_updates,
            );
            counters.record_layout_stack_per_level(
                stats.measure_stack_per_level,
                stats.placement_stack_per_level,
            );
            counters.record_layout_placement_boundaries(stats.placement_boundaries);
            counters.record_layout_slicing(
                stats.placement_boundaries_deferred,
                stats.placement_slices,
                stats.placement_deferrals_cancelled,
            );
            counters.record_layout_plans(
                stats.placement_plans_reused,
                stats.measure_plans_reused,
                stats.suffixes_replayed,
                stats.children_measured,
                stats.containers_uncacheable,
                stats.plan_misses,
                stats.plan_rebuilds,
                stats.local_context_fallbacks,
                stats.retain_sweeps,
            );
            counters.inline_reflows += stats.inline_reflows;
            counters.inline_atomic_measure_requests += stats.inline_atomic_measure_requests;
            counters.dynamic.accumulate(stats.dynamic);
        });
    }

    fn note_layout_result_publish(
        &mut self,
        reused: usize,
        changed: usize,
        delta_commits: usize,
        children_visited: usize,
    ) {
        self.bump_last_counters(|counters| {
            counters.record_layout_result_publish(reused, changed, delta_commits, children_visited);
        });
    }

    pub(crate) fn record_layout_frontier(&mut self, stats: crate::LayoutFrontierStats) {
        self.bump_last_counters(|counters| {
            counters.record_constraint_dependents(
                stats.constraint.seeds,
                stats.constraint.considered,
                stats.constraint.remeasured,
                stats.constraint.considered - stats.constraint.remeasured,
            );
            counters.record_layout_frontier(
                stats.seeds,
                stats.seed_merges,
                stats.nodes_measure,
                stats.nodes_placement,
                stats.contexts,
                stats.nodes_writing,
                stats.dependency_edges_visited,
                stats.propagations_stopped,
                stats.local_subtree_fallbacks,
                stats.full_document_fallbacks,
            );
            counters.record_layout_scratch(
                stats.scratch_entries,
                stats
                    .scratch_entries
                    .saturating_mul(crate::LayoutDependencyGraph::ENTRY_BYTES),
            );
        });
    }

    /// A pass a constraint change seeded solved `solves` formatting contexts
    /// from scratch.
    pub(crate) fn record_resize_context_solves(&mut self, solves: usize) {
        if solves > 0 {
            self.bump_last_counters(|counters| {
                counters.resize_context_solves =
                    counters.resize_context_solves.saturating_add(solves);
            });
        }
    }

    fn record_id_list_alloc(&self, len: usize) {
        if len == 0 {
            return;
        }
        self.record_hot_path_allocation(1, len.saturating_mul(size_of::<StableNodeId>()));
    }

    fn record_string_clone(&self, len: usize) {
        if len == 0 {
            return;
        }
        self.record_hot_path_allocation(1, len);
    }

    pub fn theme_appearance(&self) -> nana_ui_core::ThemeAppearance {
        self.invalidation.style_model.theme_appearance
    }

    pub fn theme_metrics(&self) -> nana_ui_core::ThemeMetrics {
        self.invalidation.style_model.metrics
    }

    pub fn style_model(&self) -> StyleModelRef {
        self.invalidation.style_model
    }

    /// The installed design system.
    ///
    /// Returned by reference: a [`CompiledTheme`](nana_ui_core::CompiledTheme)
    /// is around a kilobyte, and handing one out by value on a read path is
    /// the mistake Issue #101 §1.5 measured with `LayoutStyle`. Callers that
    /// only want a colour or a metric take [`Self::style_model`] instead.
    pub fn theme(&self) -> &nana_ui_core::CompiledTheme {
        &self.invalidation.theme
    }

    /// The design system as installed: [`Self::theme`] without the system
    /// high-contrast overlay.
    pub fn installed_theme(&self) -> &nana_ui_core::CompiledTheme {
        &self.invalidation.installed_theme
    }

    /// Shared installed theme for reactive consumers deriving geometry from it.
    pub fn installed_theme_arc(&self) -> Arc<nana_ui_core::CompiledTheme> {
        Arc::clone(&self.invalidation.installed_theme)
    }

    /// Whether the system high-contrast overlay is on.
    pub fn high_contrast(&self) -> bool {
        self.invalidation.high_contrast
    }

    /// Drain this world's dirty components into deterministic system work.
    ///
    /// This is the low-level world drain and intentionally has the same name
    /// as [`crate::AppContext::take_system_work`]. The `AppContext` method is
    /// the canonical application/frame entry point: it first polls local
    /// tasks and flushes the reactive host, then delegates here. Calling this
    /// method directly is reserved for world-owned tests, benchmarks and
    /// compatibility adapters that already performed those steps themselves.
    /// Calling it on an unchanged world returns an empty work set and performs
    /// no scheduling.
    pub fn take_system_work(&mut self) -> SystemWork {
        self.drain_system_work(None)
    }

    /// [`Self::take_system_work`] for one document's frame: style, text and
    /// the other world-wide work drain as usual, but only `document`'s layout
    /// seeds do. Another document's stay pending for its own frame, which a
    /// pass of this document could only drop.
    pub fn take_system_work_for(&mut self, document: DocumentId) -> SystemWork {
        self.drain_system_work(Some(document))
    }

    fn drain_system_work(&mut self, document: Option<DocumentId>) -> SystemWork {
        let ours = |world: &Self, id: StableNodeId| {
            document.is_none_or(|document| world.document_of(id) == Some(document))
        };
        self.invalidation
            .counters
            .pending_drain_counts
            .scroll_content_extent_recomputes += self.take_scroll_extent_recomputes();
        let mut ids = std::mem::take(&mut self.invalidation.dirty_entities)
            .into_iter()
            .collect::<Vec<_>>();
        ids.sort_unstable();
        let dirty_len = ids.len();
        let mut work = SystemWork {
            generation: self.invalidation.generation,
            style: Vec::new(),
            state: Vec::new(),
            text: Vec::new(),
            layout_frontier_seeds: Vec::new(),
            transform: Vec::new(),
            transform_kept: Vec::new(),
            input_hit_test: Vec::new(),
            focus_ime: Vec::new(),
            accessibility: Vec::new(),
            accessibility_moved: Vec::new(),
            accessibility_scrolled: Vec::new(),
            layout_moved: Vec::new(),
            layout_reshaped: Vec::new(),
            accessibility_removals: sorted_unique(std::mem::take(
                &mut self.invalidation.pending_accessibility_removals,
            )),
            render_extraction: Vec::new(),
            render_removals: sorted_unique(std::mem::take(
                &mut self.invalidation.pending_render_removals,
            )),
            entities_total: self.nodes.len(),
            entities_changed: 0,
            entities_spawned: std::mem::take(&mut self.tree.spawned_since_drain),
            entities_despawned: std::mem::take(&mut self.tree.despawned_since_drain),
            input_targets: self.live_input_target_count(),
            render_nodes_changed: 0,
            render_nodes_extracted: 0,
            extracted_text_spans: 0,
            allocations: 0,
            allocated_bytes: 0,
            text_shaped_runs: 0,
            text_wrap_layouts: 0,
            intrinsic_measure_requests: 0,
            intrinsic_measure_cache_hits: 0,
            intrinsic_measure_cache_misses: 0,
            intrinsic_measure_full_subtrees: 0,
            intrinsic_generation_bumps: 0,
            baseline_queries: 0,
            cross_context_measure_hits: 0,
            cross_context_measure_misses: 0,
            // A drain always follows the commits it drains, so 0 here is an
            // observed "validated nothing", not a missing measurement.
            validation_nodes_scanned: Some(std::mem::take(
                &mut self.invalidation.validation_nodes_scanned,
            )),
        };
        work.render_removals.sort_unstable();
        work.accessibility_removals.sort_unstable();
        for id in ids {
            let bits = self.record_mut(id).dirty.take();
            if !self.presence_live(id) {
                continue;
            }
            // Layout payloads are drained exclusively from the typed side
            // table. The private LAYOUT bit only causes this entity to be
            // visited by the drain.
            if bits & DirtyMask::LAYOUT != 0
                && ours(self, id)
                && let Some(invalidation) =
                    self.invalidation.pending_layout_invalidations.remove(&id)
            {
                work.layout_frontier_seeds
                    .push(crate::LayoutFrontierSeed::new(id, invalidation));
            }
            // An editor bears text with no value yet: its caret line and an
            // IME preedit are shaped from its presentation. Text that went
            // empty is still shaped once, to nothing: the metrics it held
            // sized its box, and a box laid out cold without text has none.
            let has_text = matches!(self.record(id).kind.as_ref(), NodeKind::Text)
                || self.record(id).has_text
                || self.nodes.text_metrics(id) != TextMetrics::default()
                || self
                    .widget_visual_ref(id)
                    .is_some_and(|w| w.owns_intrinsic_text() || w.is_text_input());
            let bits = if has_text {
                bits
            } else {
                // A block with no text of its own still holds a strut when
                // its lines hold atoms: its font changed, or it is new.
                if bits & DirtyMask::TEXT != 0
                    && (!self.record(id).hierarchy.children.is_empty()
                        || self.nodes.block_strut(id).is_some())
                {
                    self.text.strut_candidates.push(id);
                }
                bits & !DirtyMask::TEXT
            };
            if bits != 0 {
                work.entities_changed += 1;
            }
            push_work(&mut work, id, bits);
        }
        // Retain any typed entry queued by a post-drain authority. Sorting
        // keeps the public work batch deterministic.
        let pending = self
            .invalidation
            .pending_layout_invalidations
            .drain()
            .collect::<Vec<_>>();
        let (pending, theirs): (Vec<_>, Vec<_>) =
            pending.into_iter().partition(|(id, _)| ours(self, *id));
        // Another document's stay for its own drain; a node that has left
        // the world belongs to none and has nothing to lay out.
        let theirs = theirs
            .into_iter()
            .filter(|(id, _)| self.nodes.contains(*id))
            .collect::<Vec<_>>();
        self.invalidation
            .pending_layout_invalidations
            .extend(theirs);
        let mut remaining = pending
            .into_iter()
            .filter(|(id, _)| self.presence_live(*id))
            .map(|(id, invalidation)| crate::LayoutFrontierSeed::new(id, invalidation))
            .collect::<Vec<_>>();
        remaining.compact_sort_by_key(|seed| seed.node);
        work.layout_frontier_seeds.extend(remaining);
        work.render_nodes_changed = work.render_extraction.len();
        work.render_nodes_extracted = work.render_extraction.len();
        if work.render_nodes_changed > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::SCENE_NODES_CHANGED,
                work.render_nodes_changed
            );
        }
        let mut drain_allocs = 0usize;
        let mut drain_bytes = 0usize;
        let mut bump_list = |len: usize| {
            if len > 0 {
                drain_allocs = drain_allocs.saturating_add(1);
                drain_bytes =
                    drain_bytes.saturating_add(len.saturating_mul(size_of::<StableNodeId>()));
            }
        };
        bump_list(dirty_len);
        bump_list(work.style.len());
        bump_list(work.state.len());
        bump_list(work.text.len());
        bump_list(work.layout_frontier_seeds.len());
        bump_list(work.transform.len());
        bump_list(work.transform_kept.len());
        bump_list(work.input_hit_test.len());
        bump_list(work.focus_ime.len());
        bump_list(work.accessibility.len());
        bump_list(work.accessibility_removals.len());
        bump_list(work.render_extraction.len());
        bump_list(work.render_removals.len());
        work.record_hot_path_allocation(drain_allocs, drain_bytes);
        if !work.is_empty() {
            self.invalidation.pending_hot_allocations.set(0);
            self.invalidation.pending_hot_allocated_bytes.set(0);
            let mut counters = work.counters();
            let counted = std::mem::take(
                &mut self
                    .invalidation
                    .counters
                    .pending_layout_invalidations_counted,
            );
            add_pending_counts(&mut counters, counted);
            add_pending_counts(
                &mut counters,
                std::mem::take(&mut self.invalidation.counters.pending_drain_counts),
            );
            counters.i18n.accumulate(self.i18n_gauges());
            // Extracted node/span fields are filled by [`Self::record_extract`]
            // on the product path, not by the planned render list.
            counters.render_nodes_extracted = 0;
            counters.extracted_text_spans = 0;
            if self.invalidation.accumulating_frame {
                self.invalidation
                    .counters
                    .frame_counters
                    .accumulate(counters);
                self.invalidation.counters.last_counters =
                    self.invalidation.counters.frame_counters;
                self.invalidation
                    .counters
                    .last_counters
                    .render_nodes_extracted = self.invalidation.frame_extracted_nodes;
                self.invalidation
                    .counters
                    .last_counters
                    .extracted_text_spans = self.invalidation.frame_extracted_spans;
            } else {
                self.invalidation.counters.last_counters = counters;
            }
        } else {
            self.commit_pending_hot_allocs();
            self.commit_pending_layout_invalidation_counts();
            let pending = std::mem::take(&mut self.invalidation.counters.pending_drain_counts);
            self.bump_last_counters(|counters| add_pending_counts(counters, pending));
        }
        work
    }

    /// Count UI frames this world would emit over `ticks` host attempts with no
    /// external vsync. A frame is a non-empty dirty drain ([`Self::take_system_work`]).
    /// Empty drains do not count. Elapsed time and `idle_schedule_ms` are not frames.
    pub fn scheduled_ui_frames(&mut self, ticks: usize) -> usize {
        let mut frames = 0;
        for _ in 0..ticks {
            if self.take_system_work().is_empty() {
                continue;
            }
            frames += 1;
        }
        frames
    }

    /// Restore drained work after a frame-system failure. Derived writes are
    /// idempotent, so retrying the complete transaction is safer than losing
    /// accessibility or render invalidations from an earlier pass.
    pub fn restore_system_work(&mut self, work: SystemWork) {
        self.restore_layout_frontier_seeds(&work.layout_frontier_seeds);
        for (ids, bit) in [
            (work.style, DirtyMask::STYLE),
            (work.state, DirtyMask::STATE),
            (work.text, DirtyMask::TEXT),
            (work.transform, DirtyMask::TRANSFORM),
            (work.transform_kept, DirtyMask::TRANSFORM_KEPT),
            (
                work.input_hit_test,
                DirtyMask::INPUT | DirtyMask::INPUT_ELSEWHERE,
            ),
            (work.focus_ime, DirtyMask::FOCUS_IME),
            (work.accessibility, DirtyMask::ACCESSIBILITY),
            (work.render_extraction, DirtyMask::RENDER),
        ] {
            for id in ids {
                if !self.nodes.contains(id) {
                    continue;
                };
                let epoch = self.invalidation.render_epoch.get();
                let record = self
                    .nodes
                    .get_mut(id)
                    .expect("retained node must have dirty state");
                if bit == DirtyMask::RENDER {
                    // Why it was marked went with the drain.
                    record.render_full_epoch = epoch;
                }
                let changed = record.dirty.insert(bit);
                if changed {
                    self.invalidation.dirty_entities.insert(id);
                    self.invalidation.pending_work_revision =
                        self.invalidation.pending_work_revision.saturating_add(1);
                }
            }
        }
        self.invalidation
            .pending_accessibility_removals
            .extend(work.accessibility_removals);
        self.invalidation
            .pending_accessibility_removals
            .sort_unstable();
        self.invalidation.pending_accessibility_removals.dedup();
        self.invalidation
            .pending_render_removals
            .extend(work.render_removals);
        self.invalidation.pending_render_removals.sort_unstable();
        self.invalidation.pending_render_removals.dedup();
        self.tree.spawned_since_drain = self
            .tree
            .spawned_since_drain
            .saturating_add(work.entities_spawned);
        self.tree.despawned_since_drain = self
            .tree
            .despawned_since_drain
            .saturating_add(work.entities_despawned);
    }

    /// Whether `id` is a document root node.
    pub(crate) fn is_document_node(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|record| matches!(record.kind.as_ref(), NodeKind::Document))
    }

    pub fn node(&self, id: StableNodeId) -> Option<NodeSnapshot> {
        let node = self.nodes.get(id)?;
        Some(NodeSnapshot {
            id,
            document: node.document,
            kind: node.kind.as_ref().clone(),
            parent: node.hierarchy.parent,
            children: node.hierarchy.children.as_ref().clone(),
        })
    }

    pub fn focused(&self, document: DocumentId) -> Option<StableNodeId> {
        self.input.focused.get(&document).copied()
    }

    /// Documents that have a focused node.
    pub(crate) fn focused_documents(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.input.focused.keys().copied()
    }

    /// Record which kind of device the event being routed came from.
    ///
    /// Hosts call this from their one input entry point. The modality is read
    /// again each time focus is written, so a click makes exactly the focus it
    /// causes invisible — not every focus after it, and never a text field's
    /// (see [`Self::focus_visible`]).
    pub fn note_input_modality(&mut self, document: DocumentId, modality: InputModality) {
        self.input.last_modality.insert(document, modality);
        match modality {
            InputModality::Pointer => self.input.pointer_modality.insert(document),
            InputModality::Keyboard | InputModality::Gamepad | InputModality::Programmatic => {
                self.input.pointer_modality.remove(&document)
            }
        };
    }

    /// Most recent input modality recorded for `document`.
    pub fn input_modality(&self, document: DocumentId) -> InputModality {
        self.input
            .last_modality
            .get(&document)
            .copied()
            .unwrap_or(InputModality::Programmatic)
    }

    /// Effective focus policy for `id` (explicit override if any, else derived
    /// from `InteractionState::focusable`, and always `None` when disabled).
    pub fn focus_policy(&self, id: StableNodeId) -> crate::FocusPolicy {
        if !self.contains(id) {
            return crate::FocusPolicy::None;
        }
        let base = match self.input.focus_policies.get(&id).copied() {
            Some(policy) => policy,
            None => {
                if self.record(id).interaction.focusable {
                    crate::FocusPolicy::Strong
                } else {
                    crate::FocusPolicy::None
                }
            }
        };
        if base == crate::FocusPolicy::None || self.is_disabled_in_chain(id) {
            crate::FocusPolicy::None
        } else {
            base
        }
    }

    pub fn explicit_focus_policy(&self, id: StableNodeId) -> Option<crate::FocusPolicy> {
        self.input.focus_policies.get(&id).copied()
    }

    pub fn focus_neighbors(&self, id: StableNodeId) -> crate::FocusNeighbors {
        self.input
            .focus_neighbors
            .get(&id)
            .copied()
            .unwrap_or_default()
    }

    pub fn directional_focus_scope(
        &self,
        id: StableNodeId,
    ) -> Option<crate::DirectionalFocusScope> {
        self.input.directional_scopes.get(&id).copied()
    }

    pub(crate) fn has_directional_focus_scopes(&self) -> bool {
        !self.input.directional_scopes.is_empty()
    }

    pub fn is_disabled_in_chain(&self, mut id: StableNodeId) -> bool {
        loop {
            let Some(record) = self.nodes.get(id) else {
                return false;
            };
            if self.nodes.accessibility(id).disabled {
                return true;
            }
            let Some(parent) = record.hierarchy.parent else {
                return false;
            };
            id = parent;
        }
    }

    pub fn focus_counters(&self) -> crate::FocusCounters {
        self.input.focus_counters
    }

    pub(crate) fn record_focus_navigation_query(&mut self, candidates_considered: u64) {
        self.input.focus_counters.navigation_queries += 1;
        self.input.focus_counters.navigation_candidates_considered += candidates_considered;
        nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FOCUS_NAVIGATION_QUERIES);
        if candidates_considered > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::FOCUS_NAVIGATION_CANDIDATES,
                candidates_considered
            );
        }
    }

    pub(crate) fn is_focus_trap_root_active(&self, root: StableNodeId) -> bool {
        let Some(record) = self.nodes.get(root) else {
            return false;
        };
        if !self.is_mounted(root)
            || !self.nodes.accessibility(root).modal
            || self.surface_closed(root)
            || !self.is_overlay_reachable(root)
        {
            return false;
        }
        record.hierarchy.parent.is_some_and(|host| {
            self.overlay_host(host)
                .is_some_and(|state| state.active == Some(root))
        })
    }

    pub(crate) fn sync_focus_trap(&mut self, root: StableNodeId) {
        let active = self.is_focus_trap_root_active(root);
        let was_active = self.input.active_focus_traps.contains(&root);
        if active == was_active {
            return;
        }
        if active {
            self.input.active_focus_traps.insert(root);
            self.input.focus_counters.trap_enters += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FOCUS_TRAP_ENTERS);
        } else {
            self.input.active_focus_traps.remove(&root);
            self.input.focus_counters.trap_exits += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FOCUS_TRAP_EXITS);
        }
        let count = self.input.active_focus_traps.len() as u64;
        self.input.focus_counters.traps_active = count;
        nana_diagnostics::metric!(
            nana_diagnostics::framework::runtime::FOCUS_TRAPS_ACTIVE,
            count
        );
    }

    pub(crate) fn sync_active_focus_traps(&mut self) {
        if self.input.active_focus_traps.is_empty() {
            return;
        }
        let roots: Vec<StableNodeId> = self.input.active_focus_traps.iter().copied().collect();
        for root in roots {
            self.sync_focus_trap(root);
        }
    }

    /// The focused node, when focus should also be *shown*.
    ///
    /// Same answer as [`Self::focused`] except right after a pointer press put
    /// it on a control that takes no typing: a clicked button is focused and
    /// does not say so. A text field that takes typing always says so, as the
    /// web's `:focus-visible` holds for any element that takes keyboard input:
    /// the next key goes there, however the caret got in. Paint asks this one;
    /// hit-testing, IME and the accessibility tree keep asking
    /// [`Self::focused`], because focus is still focus — only its indicator is
    /// conditional.
    pub fn focus_visible(&self, document: DocumentId) -> Option<StableNodeId> {
        let focused = self.focused(document)?;
        if self.input.focus_from_pointer.contains(&document) && !self.takes_typing(focused) {
            return None;
        }
        Some(focused)
    }

    /// A text field the user can type into now: an editor that is neither
    /// disabled nor read-only.
    fn takes_typing(&self, id: StableNodeId) -> bool {
        self.nodes.text_input(id).is_some() && self.nodes.accessibility(id).editable
    }

    pub fn focused_text_input(
        &self,
        document: DocumentId,
    ) -> Option<(StableNodeId, crate::TextInputView<'_>)> {
        let id = self.focused(document)?;
        Some((id, self.text_input(id)?))
    }

    pub fn text(&self, id: StableNodeId) -> Option<&str> {
        self.nodes
            .contains(id)
            .then(|| self.nodes.text_content(id).value.as_str())
    }

    pub fn document_text_selection(
        &self,
        document: DocumentId,
    ) -> Option<&crate::DocumentTextSelection> {
        self.text.document_text_selections.get(&document)
    }

    pub fn set_document_text_selection(
        &mut self,
        document: DocumentId,
        selection: Option<crate::DocumentTextSelection>,
    ) {
        if self.text.document_text_selections.get(&document) == selection.as_ref() {
            return;
        }
        if let Some(previous) = self.text.document_text_selections.remove(&document)
            && self.contains(previous.node)
        {
            let _ = self.mark(previous.node, DirtyMask::RENDER);
        }
        if let Some(selection) = selection {
            if self.contains(selection.node) {
                let _ = self.mark(selection.node, DirtyMask::RENDER);
            }
            self.text
                .document_text_selections
                .insert(document, selection);
        }
    }

    pub(crate) fn drop_invalid_document_text_selections(&mut self) {
        let stale: Vec<DocumentId> = self
            .text
            .document_text_selections
            .iter()
            .filter_map(|(&document, selection)| {
                let keep = self.contains(selection.node)
                    && self
                        .computed_style(selection.node)
                        .is_some_and(|style| style.user_select.allows_document_select());
                (!keep).then_some(document)
            })
            .collect();
        for document in stale {
            self.set_document_text_selection(document, None);
        }
    }

    /// Geometry from the published canonical layout result.
    ///
    /// Projection code uses this accessor so it cannot render, hit-test, or
    /// expose a retained box that has not been committed to a result yet.
    pub fn canonical_layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        let meta = self.published_layout_meta(id)?;
        let [x, y] = self.layout_origin(id)?;
        Some(LayoutBox {
            x,
            y,
            width: meta.width,
            height: meta.height,
        })
    }

    /// The box layout last wrote for `id`: what interaction maps pointers
    /// against and components size themselves by. Unlike
    /// [`Self::canonical_layout_box`] it does not wait for the next
    /// publication after a change, so a press between a write and the next
    /// layout still lands.
    pub(crate) fn component_layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        self.nodes
            .get(id)
            .map(|record| self.record_layout_box(id, record))
    }

    /// `record`'s box on the page; see [`Self::layout_origin`].
    pub(super) fn record_layout_box(&self, id: StableNodeId, record: &NodeRecord) -> LayoutBox {
        let [x, y] = self.record_layout_origin(id, record);
        LayoutBox {
            x,
            y,
            ..self.nodes.local_layout(id)
        }
    }

    /// The box layout last wrote for `id`, measured from the origin of its
    /// [`Self::layout_frame`]: what stays put when a box above it moves.
    /// [`Self::layout_box`] is the frame's document origin plus this.
    pub fn local_layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        self.nodes.contains(id).then(|| self.nodes.local_layout(id))
    }

    /// What `id`'s [`Self::local_layout_box`] is measured from.
    pub fn layout_frame(&self, id: StableNodeId) -> Option<crate::LayoutFrame> {
        self.nodes.contains(id).then(|| self.nodes.layout_frame(id))
    }

    #[inline]
    pub(crate) fn resolved_palette_epoch(&self) -> u32 {
        (self.palette_epoch as u32).max(1)
    }

    /// The node whose border-box origin `id`'s local box is measured
    /// from; `None` for the page's.
    fn frame_node(&self, id: StableNodeId, record: &NodeRecord) -> Option<StableNodeId> {
        match self.nodes.layout_frame(id) {
            crate::LayoutFrame::Parent => record.hierarchy.parent,
            crate::LayoutFrame::Block(block) => Some(block),
            crate::LayoutFrame::Page => None,
        }
    }

    /// Where `id`'s border box stands on the page: its local origin and its
    /// frames' summed from the top down, one axis at a time, so a box under
    /// one that moved is the sum a fresh layout gives. O(depth).
    pub(crate) fn layout_origin(&self, id: StableNodeId) -> Option<[f32; 2]> {
        self.layout_origin_in(id, self.invalidation.geometry_frame.get())
    }

    pub(crate) fn layout_origin_in(
        &self,
        id: StableNodeId,
        frame_id: Option<StableNodeId>,
    ) -> Option<[f32; 2]> {
        self.nodes
            .get(id)
            .map(|record| self.record_layout_origin_in(id, record, frame_id))
    }

    /// The frame layout measures `id`'s box from, read off the styles as
    /// the clip chain reads them ([`Self::clip_position`]): the page for a
    /// `fixed` box and a triggered menu's items, an absolute box's
    /// containing block, and the parent otherwise. A document box written
    /// from outside is kept in it.
    pub(crate) fn layout_frame_from_styles(&self, id: StableNodeId) -> crate::LayoutFrame {
        let parent = self.parent_id(id);
        match self.clip_position(id) {
            PositionSpec::Fixed => return crate::LayoutFrame::Page,
            PositionSpec::Absolute => {}
            _ => return crate::LayoutFrame::Parent,
        }
        let mut ancestor = parent;
        while let Some(block) = ancestor {
            if self.is_document_node(block) {
                break;
            }
            if self.clip_position(block).establishes_containing_block() {
                return if Some(block) == parent {
                    crate::LayoutFrame::Parent
                } else {
                    crate::LayoutFrame::Block(block)
                };
            }
            ancestor = self.parent_id(block);
        }
        crate::LayoutFrame::Page
    }

    /// Where `frame`'s origin stands on the page, for `record`'s box.
    pub(super) fn frame_origin_of(
        &self,
        record: &NodeRecord,
        frame: crate::LayoutFrame,
    ) -> [f32; 2] {
        let node = match frame {
            crate::LayoutFrame::Parent => record.hierarchy.parent,
            crate::LayoutFrame::Block(block) => Some(block),
            crate::LayoutFrame::Page => None,
        };
        node.and_then(|node| self.layout_origin(node))
            .unwrap_or([0.0, 0.0])
    }

    pub(super) fn record_layout_origin(&self, id: StableNodeId, record: &NodeRecord) -> [f32; 2] {
        self.record_layout_origin_in(id, record, self.invalidation.geometry_frame.get())
    }

    pub(super) fn record_layout_origin_in(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        frame_id: Option<StableNodeId>,
    ) -> [f32; 2] {
        if let Some(origin) = self.remembered_page_origin(id, record, frame_id) {
            #[cfg(any(test, feature = "layout-verify"))]
            {
                #[cfg(test)]
                let walked = page_origins::steps::uncounted(|| {
                    self.walked_layout_origin_in(id, record, frame_id)
                });
                #[cfg(not(test))]
                let walked = self.walked_layout_origin_in(id, record, frame_id);
                assert!(
                    origin[0].to_bits() == walked[0].to_bits()
                        && origin[1].to_bits() == walked[1].to_bits(),
                    "a remembered page origin {origin:?} is the walked one {walked:?}"
                );
            }
            return origin;
        }
        self.walked_layout_origin_in(id, record, frame_id)
    }

    /// [`Self::record_layout_origin_in`], every frame walked.
    fn walked_layout_origin_in(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        frame_id: Option<StableNodeId>,
    ) -> [f32; 2] {
        // Deriving a component's geometry in its own coordinates: a chain
        // through that node starts at its origin.
        let frame = frame_id.and_then(|frame| self.nodes.get(frame));
        // Most chains are shallow: their locals stay on the stack.
        const INLINE: usize = 48;
        let mut inline = [[0.0f32; 2]; INLINE];
        let mut spilled: Vec<[f32; 2]> = Vec::new();
        let mut len = 0usize;
        let mut cursor_id = id;
        let mut cursor = Some(record);
        let mut reached = frame.is_none();
        while let Some(node) = cursor {
            #[cfg(test)]
            page_origins::steps::step(page_origins::steps::Walk::PageOrigin);
            if frame_id == Some(cursor_id) {
                reached = true;
                break;
            }
            if matches!(node.kind.as_ref(), NodeKind::Document) {
                break;
            }
            let local_box = self.nodes.local_layout(cursor_id);
            let local = [local_box.x, local_box.y];
            if len < INLINE {
                inline[len] = local;
            } else {
                if spilled.is_empty() {
                    spilled.extend_from_slice(&inline);
                }
                spilled.push(local);
            }
            len += 1;
            if let Some(next_id) = self.frame_node(cursor_id, node) {
                cursor_id = next_id;
                cursor = self.nodes.get(next_id);
            } else {
                cursor = None;
            }
        }
        let chain = if spilled.is_empty() {
            &inline[..len]
        } else {
            spilled.as_slice()
        };
        let mut rest = chain.iter().rev();
        let origin = match rest.next() {
            Some(&top) => rest.fold(top, |[x, y], [dx, dy]| [x + dx, y + dy]),
            None => [0.0, 0.0],
        };
        if reached {
            return origin;
        }
        // A box outside the node whose geometry is being derived: on the
        // page, and so measured from that node's page origin.
        self.invalidation.geometry_read_page.set(true);
        let frame_id = frame_id.expect("unreached only under a frame");
        let frame = frame.expect("unreached only under a frame");
        let [fx, fy] = self.record_layout_origin_in(frame_id, frame, None);
        [origin[0] - fx, origin[1] - fy]
    }

    pub(super) fn local_widget_paint(
        &self,
        id: StableNodeId,
        visual: &crate::WidgetVisualHandle,
        style: &ComputedStyle,
    ) -> Option<Arc<crate::WidgetPaintOutput>> {
        if visual.geometry_reads_page() {
            return self.page_widget_paint(id, visual, style);
        }
        let previous = self.invalidation.geometry_frame.replace(Some(id));
        let read_before = self.invalidation.geometry_read_page.replace(false);
        let paint = self.derive_widget_paint(id, visual, style);
        let read_page = self.invalidation.geometry_read_page.replace(read_before);
        self.invalidation.geometry_frame.set(previous);
        debug_assert!(
            !read_page,
            "{id:?}: {visual:?} derives its geometry from the page; list it in \
             `WidgetVisual::geometry_reads_page`"
        );
        paint.map(Arc::new)
    }

    fn page_widget_paint(
        &self,
        id: StableNodeId,
        visual: &crate::WidgetVisualHandle,
        style: &ComputedStyle,
    ) -> Option<Arc<crate::WidgetPaintOutput>> {
        let previous = self.invalidation.geometry_frame.replace(None);
        if previous.is_some() {
            self.invalidation.geometry_read_page.set(true);
        }
        let paint = self.derive_widget_paint(id, visual, style);
        self.invalidation.geometry_frame.set(previous);
        paint.map(Arc::new)
    }

    fn derive_widget_paint(
        &self,
        id: StableNodeId,
        visual: &crate::WidgetVisualHandle,
        style: &ComputedStyle,
    ) -> Option<crate::WidgetPaintOutput> {
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let node_theme = self.node_theme(id);
        let bounds = self.component_layout_box(id)?;
        let mut source_style = NodeStyle::clone(&self.nodes.get(id)?.style);
        source_style.layout = Arc::clone(&self.record(id).resolved_layout);
        let content = self.component_content_box(id)?;
        let chrome_radii =
            nana_ui_core::ChromeRadii::resolve(style_model.metrics, node_theme.recipes().shape());
        let ctx = crate::WidgetPaintCtx {
            id,
            world: self,
            bounds,
            content,
            style,
            source_style: &source_style,
            style_model,
            theme: node_theme,
            chrome_radii,
        };
        Some(visual.paint(&ctx))
    }

    /// Compatibility accessor for callers that need the retained box during
    /// the short writeback window before a result is published.
    pub fn layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        self.component_layout_box(id)
    }

    /// Where `id` is scrolled: the offset last set, except that a multiline
    /// editor reports where it is drawn — that request clamped to its value
    /// and, while focused, moved to show the caret.
    pub fn scroll_offset(&self, id: StableNodeId) -> Option<ScrollOffset> {
        let _ = self.nodes.get(id)?;
        if let Some(offset) = self.rich_editor_scroll_offset(id) {
            return Some(offset);
        }
        if self.nodes.accessibility(id).multiline
            && let Some(frame) = self.editor_frame(id)
        {
            return Some(frame.scroll_offset());
        }
        Some(self.nodes.scroll_offset(id))
    }

    /// The scrolling area a scroll offset is clamped to. A multiline text
    /// editor's follows its shaped value, so it is always current; every
    /// other container's is the one last published for it.
    /// The offset last set on `id`, which a multiline editor is drawn from.
    pub(crate) fn scroll_request(&self, id: StableNodeId) -> Option<ScrollOffset> {
        self.nodes
            .contains(id)
            .then(|| self.nodes.scroll_offset(id))
    }

    pub fn scroll_metrics(&self, id: StableNodeId) -> Option<ScrollMetrics> {
        self.text_scroll_metrics(id)
            .or_else(|| self.nodes.scroll_metrics(id).copied())
    }

    /// A box styled `overflow: auto | scroll`, or a `ScrollView` (its
    /// scrollbar visual) whatever chrome restyled its overflow — a workspace
    /// region borrowing it as its surface, say.
    pub(crate) fn is_scroll_container(&self, id: StableNodeId) -> bool {
        let scrolls = self.nodes.get(id).is_some_and(|record| {
            record.style.layout.overflow_x.scrolls() || record.style.layout.overflow_y.scrolls()
        });
        scrolls
            || (self.nodes.has_widget_visuals()
                && self
                    .nodes
                    .widget_visual(id)
                    .is_some_and(crate::WidgetVisualHandle::is_scroll_container))
    }

    /// The axes (`[horizontal, vertical]`) on which `id` keeps what
    /// overflows inside it out of the scrolling area of a scroll container
    /// above it, which then counts only `id`'s own box on that axis. CSS
    /// scrollable overflow stops at the nearest clipping box the same way: a
    /// scroll container keeps both axes (CSS makes it clip both), another box
    /// the axis `overflow: hidden` clips.
    pub fn overflow_contained_axes(&self, id: StableNodeId) -> [bool; 2] {
        self.nodes.get(id).map_or([false; 2], |record| {
            overflow_contained_axes(&record.style.layout, self.nodes.widget_visual(id))
        })
    }

    /// The scrolling area of `id`'s laid-out box over the boxes under it
    /// that reach it (see [`Self::overflow_contained_axes`]).
    pub(crate) fn layout_scroll_metrics(&self, id: StableNodeId) -> Option<ScrollMetrics> {
        // Both from the box's own origin: the area does not move with it.
        let viewport = self.local_layout_box(id).map(|bounds| LayoutBox {
            x: 0.0,
            y: 0.0,
            ..bounds
        })?;
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return None;
        }
        let extent = self.scroll_content_extent_local(id);
        Some(ScrollMetrics::scrolling_area(
            viewport,
            [extent.left, extent.top, extent.right, extent.bottom],
            self.scroll_far_start_axes(id),
        ))
    }

    /// Which page axes of scroll container `id` start at the right / bottom
    /// (`[horizontal, vertical]`), where its scroll origin then sits: an RTL
    /// inline axis, `vertical-rl`'s block axis, a `*-reverse` flex main axis.
    pub fn scroll_far_start_axes(&self, id: StableNodeId) -> [bool; 2] {
        crate::layout_engine::far_start_axes(self, id)
    }

    /// Clamp to the scrolling area. A container nothing has published
    /// metrics for yet is measured here, so its range is never guessed; one
    /// with no laid-out box has nothing to scroll past its origin.
    pub fn clamp_scroll_offset(&self, id: StableNodeId, offset: ScrollOffset) -> ScrollOffset {
        let measured = || {
            self.is_scroll_container(id)
                .then(|| self.layout_scroll_metrics(id))
                .flatten()
        };
        match self.scroll_metrics(id).or_else(measured) {
            Some(metrics) => metrics.clamp(offset),
            None => ScrollOffset {
                x: offset.x.max(0.0),
                y: offset.y.max(0.0),
            },
        }
    }

    /// The style `id` was authored with. What the pipeline reads is its
    /// effective style, with a responsive variant written over it while its
    /// rule's bucket has one (Issue #265).
    pub fn node_style(&self, id: StableNodeId) -> Option<&NodeStyle> {
        let node = self.nodes.get(id)?;
        Some(
            self.overlays_a11y
                .responsive
                .authored(id)
                .unwrap_or(&node.style),
        )
    }

    /// [`Self::node_style`] of a live node.
    pub(super) fn authored_style(&self, id: StableNodeId) -> &NodeStyle {
        self.overlays_a11y
            .responsive
            .authored(id)
            .unwrap_or(&self.record(id).style)
    }

    /// `id`'s resolved style. A palette-only theme switch re-resolves no
    /// style, so its paint colours can be the previous palette's until the
    /// node restyles; widget paint derivation and extraction read them
    /// in the installed palette.
    pub fn computed_style(&self, id: StableNodeId) -> Option<&ComputedStyle> {
        self.nodes.get(id).map(|node| node.resolved.0.as_ref())
    }

    /// Whether the text backend changed since the last call, and clears it.
    pub(crate) fn take_text_backend_changed(&mut self) -> bool {
        std::mem::take(&mut self.text.text_backend_changed)
    }

    /// Measures chrome text painted on `id` through the engine that shaped
    /// this world, or by estimate before any engine has.
    pub(crate) fn chrome_text_measure(
        &self,
        id: StableNodeId,
    ) -> crate::text_width::ChromeTextMeasure<'_> {
        crate::text_width::ChromeTextMeasure::new(
            self.paint_text_engine.as_ref(),
            self.computed_style(id),
        )
    }

    /// Whether a mounted node is visible through every retained overlay branch.
    /// Dirty computed styles are derived from the local hierarchy instead of
    /// treating the previous frame's visibility as current authority.
    pub fn is_overlay_reachable(&self, id: StableNodeId) -> bool {
        let mut visibility = None;
        let mut child = id;
        let mut current = Some(id);
        while let Some(candidate) = current {
            // Visibility inherits from the nearest declaration; a visible child
            // may override a hidden parent. Read authored styles while dirty.
            visibility = visibility.or_else(|| {
                self.node_style(candidate)
                    .and_then(|style| style.layout.paint.visibility)
            });
            if !self.presence_live(candidate)
                || !self.menu_branch_open(candidate)
                || self
                    .node_style(candidate)
                    .is_some_and(|style| style.layout.omits_box())
            {
                return false;
            }
            let parent = self.parent_id(candidate);
            if let Some(parent) = parent {
                if self
                    .overlay_host(parent)
                    .is_some_and(|state| state.active != Some(child))
                {
                    return false;
                }
                child = parent;
            }
            current = parent;
        }
        visibility != Some(nana_ui_core::VisibilitySpec::Hidden)
    }

    pub fn interaction(&self, id: StableNodeId) -> Option<InteractionState> {
        self.nodes.get(id).map(|node| node.interaction)
    }

    /// The editor's committed text and selections, read from its session
    /// without a copy.
    pub fn text_input(&self, id: StableNodeId) -> Option<crate::TextInputView<'_>> {
        self.nodes.text_input(id)
    }

    pub fn text_input_presentation(&self, id: StableNodeId) -> Option<&TextInputPresentation> {
        self.nodes.text_input_presentation(id)
    }

    pub fn highlight_request(&self, id: StableNodeId) -> Option<&HighlightRequest> {
        self.nodes.highlight(id)
    }

    pub fn text_presentation(&self, id: StableNodeId) -> Option<&TextPresentation> {
        self.nodes.text_presentation(id)
    }

    pub fn has_presenter(&self, name: &str) -> bool {
        self.text.presenters.contains_key(name)
    }

    fn invalidate_presenter(&mut self, name: &str) {
        let ids = self.nodes.keys().collect::<Vec<_>>();
        for id in ids {
            if self
                .nodes
                .highlight(id)
                .is_some_and(|request| request.presenter.as_ref() == name)
            {
                self.nodes.set_text_presentation(id, None);
                self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
            // Markdown paints fenced code through the presenters directly.
            #[cfg(feature = "rich-text")]
            if self
                .widget_visual_as::<crate::NativeMarkdownVisual>(id)
                .is_some_and(|visual| visual.code_presenter.as_ref() == name)
            {
                self.mark(id, DirtyMask::RENDER);
            }
        }
        #[cfg(feature = "rich-text")]
        self.text.code_presentations.borrow_mut().clear();
    }

    /// Install a named text presenter. Matching [`HighlightRequest`] nodes are
    /// marked dirty so the next TEXT system can derive spans.
    pub fn register_presenter(
        &mut self,
        presenter: Box<dyn TextPresenter>,
    ) -> Result<(), UiWorldError> {
        let name = presenter.name().trim();
        if name.is_empty() {
            return Err(UiWorldError::InvalidPresenter);
        }
        if self.text.presenters.contains_key(name) {
            return Err(UiWorldError::DuplicatePresenter(name.to_owned()));
        }
        let name = name.to_owned();
        self.text.presenters.insert(name.clone(), presenter);
        self.invalidate_presenter(&name);
        Ok(())
    }

    /// Install `presenter`, replacing any existing presenter registered under
    /// the same name and invalidating cached highlights on matching nodes.
    pub fn replace_presenter(
        &mut self,
        presenter: Box<dyn TextPresenter>,
    ) -> Result<Option<Box<dyn TextPresenter>>, UiWorldError> {
        let name = presenter.name().trim();
        if name.is_empty() {
            return Err(UiWorldError::InvalidPresenter);
        }
        let name = name.to_owned();
        let previous = self.text.presenters.insert(name.clone(), presenter);
        self.invalidate_presenter(&name);
        Ok(previous)
    }

    /// Spans the presenter `presenter` gives `source` as `language`, or
    /// `None` when no such presenter is registered or it colors nothing.
    /// Fenced code in [`crate::NativeMarkdown`] paints through this.
    #[cfg(feature = "rich-text")]
    pub(crate) fn code_presentation(
        &self,
        presenter: &str,
        language: &str,
        source: &str,
    ) -> Option<Arc<[crate::TextSpan]>> {
        let presenter_impl = self.text.presenters.get(presenter)?;
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (presenter, language, source).hash(&mut hasher);
            hasher.finish()
        };
        if let Some(entry) = self.text.code_presentations.borrow().iter().find(|entry| {
            entry.hash == key
                && entry.presenter.as_ref() == presenter
                && entry.language.as_ref() == language
                && entry.source.as_ref() == source
        }) {
            return (!entry.spans.is_empty()).then(|| Arc::clone(&entry.spans));
        }
        let request = HighlightRequest::new(presenter, language);
        let spans: Arc<[crate::TextSpan]> =
            crate::presentation::sanitize_spans(source, presenter_impl.present(source, &request))
                .into();
        let mut held = self.text.code_presentations.borrow_mut();
        if held.len() >= 128 {
            held.remove(0);
        }
        held.push(crate::world::subsystems::CodePresentationEntry {
            hash: key,
            presenter: Arc::from(presenter),
            language: Arc::from(language),
            source: Arc::from(source),
            spans: Arc::clone(&spans),
        });
        (!spans.is_empty()).then_some(spans)
    }

    /// Derive [`TextPresentation`] for scheduled text nodes. Committed text
    /// only; IME preedit is ignored here and omitted from extraction.
    pub fn resolve_presentations(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        for &id in ids {
            if !self.contains(id) {
                return Err(UiWorldError::MissingNode(id));
            }
            let Some(request) = self.nodes.highlight(id).cloned() else {
                if self.nodes.text_presentation(id).is_some() {
                    self.nodes.set_text_presentation(id, None);
                }
                continue;
            };
            let text = self.committed_presentation_text(id);
            let source = crate::presentation::presentation_source(&text, &request);
            if self
                .nodes
                .text_presentation(id)
                .is_some_and(|presentation| presentation.source == source)
            {
                continue;
            }
            // 基础层先出，语义 overlay 在 presenter 结果之后、sanitize 之前
            // 合并（overlay 段优先，重叠处丢基础层）；presenter 未注册时
            // 基础层为空，overlay 仍单独生效（宿主喂数据、框架只渲染）。
            let base = self
                .text
                .presenters
                .get(request.presenter.as_ref())
                .map(|presenter| presenter.present(&text, &request))
                .unwrap_or_default();
            let spans = match request.overlay.as_ref() {
                Some(overlay) => crate::presentation::sanitize_spans(
                    &text,
                    crate::presentation::merge_overlay_spans(base, overlay),
                ),
                None => crate::presentation::sanitize_spans(&text, base),
            };
            self.nodes
                .set_text_presentation(id, Some(TextPresentation { spans, source }));
        }
        Ok(())
    }

    fn committed_presentation_text(&self, id: StableNodeId) -> crate::TextValue {
        self.nodes
            .text_input(id)
            .map(|state| state.value_shared())
            .unwrap_or_else(|| self.nodes.text_content(id).value.clone())
    }

    /// The per-glyph presentation of a rich text node.
    pub fn glyph_presentation(&self, id: StableNodeId) -> Option<&nana_ui_core::GlyphPresentation> {
        self.nodes.glyph_presentation(id)
    }

    /// The caret and selection marks of a rich text editor node.
    pub fn rich_editor_marks(&self, id: StableNodeId) -> Option<&crate::RichEditorMarks> {
        self.nodes.rich_editor_marks(id)
    }

    /// The rich text `id` was given with `SetRichText`, if any.
    pub fn rich_text(&self, id: StableNodeId) -> Option<&nana_ui_core::RichText> {
        self.nodes.rich_text(id)
    }

    pub fn text_metrics(&self, id: StableNodeId) -> Option<TextMetrics> {
        self.nodes.contains(id).then(|| self.nodes.text_metrics(id))
    }

    /// The width a plain text that wrapped to its box takes unwrapped, when
    /// that is wider than the lines it wrapped to: what it asks of layout
    /// once its box may grow. For a select that sizes to its options
    /// ([`crate::Select::fit_options`]) it is also its widest option, when
    /// that is wider than the label it shows. `None` otherwise.
    pub(crate) fn text_natural_width(&self, id: StableNodeId) -> Option<f32> {
        self.nodes.text_natural_width(id).copied()
    }

    /// The width a plain text that wraps was last shaped against: its box
    /// then, or infinity before it had one. Negative infinity when its
    /// widest line cannot break: that line is as wide in a box of any width,
    /// however narrow. `None` for text that does not wrap.
    pub(crate) fn text_wrap_limit(&self, id: StableNodeId) -> Option<f32> {
        self.nodes.text_wrap_limit(id).copied()
    }

    /// The editor's IME state: its preedit, or an empty one while an IME is
    /// attached with nothing composed.
    pub fn ime(&self, id: StableNodeId) -> Option<crate::ImeView<'_>> {
        self.nodes.ime(id)
    }

    pub fn custom_render(&self, id: StableNodeId) -> Option<&CustomRenderNode> {
        self.nodes.custom_render(id)
    }

    pub fn has_event(&self, id: StableNodeId, event: &str) -> bool {
        self.event_listeners(id)
            .is_some_and(|listeners| listeners.contains(event))
    }

    pub fn event_listeners(&self, id: StableNodeId) -> Option<&EventListeners> {
        self.nodes.event_listeners(id)
    }

    pub fn component_type(&self, id: StableNodeId) -> Option<&ComponentTypeId> {
        self.nodes.component_type(id)
    }

    /// Live nodes of one component type in `document`, in no particular order.
    ///
    /// Callers that used to walk `document_order` and filter cost the number of
    /// that component instead of the world size.
    pub fn nodes_of_component(
        &self,
        document: DocumentId,
        component: &str,
    ) -> impl Iterator<Item = StableNodeId> + '_ {
        self.tree
            .nodes_by_component
            .get(component)
            .into_iter()
            .flatten()
            .copied()
            .filter(move |id| self.document_of(*id) == Some(document))
    }

    /// Keep [`Self::nodes_by_component`] in step with a node's component type.
    fn reindex_component(&mut self, id: StableNodeId, next: Option<&ComponentTypeId>) {
        if let Some(previous) = self.nodes.component_type(id).cloned()
            && let Some(nodes) = self.tree.nodes_by_component.get_mut(&previous)
        {
            nodes.remove(&id);
            if nodes.is_empty() {
                self.tree.nodes_by_component.remove(&previous);
            }
        }
        if let Some(next) = next {
            self.tree
                .nodes_by_component
                .entry(next.clone())
                .or_default()
                .insert(id);
        }
    }

    fn note_structural_change(&mut self, parent: StableNodeId) {
        if self.tree.structural_change_parents.last() != Some(&parent) {
            self.tree.structural_change_parents.push(parent);
        }
        // Whether it has inline-level children, and so a strut, may have
        // changed with them.
        self.text.strut_candidates.push(parent);
    }

    /// Parents whose child list changed since the last drain, or one of
    /// whose children started or stopped generating a box (`hidden`,
    /// `display: none`), deduplicated in ascending order. Consumers must
    /// drain per commit so scheduled reprojections observe the post-mutation
    /// tree.
    pub fn take_structural_change_parents(&mut self) -> Vec<StableNodeId> {
        let mut parents = std::mem::take(&mut self.tree.structural_change_parents);
        parents.sort_unstable();
        parents.dedup();
        parents
    }

    pub fn event_targets(&self, document: DocumentId) -> HashSet<(u64, String)> {
        self.document_order(document)
            .into_iter()
            .flat_map(|id| {
                self.event_listeners(id)
                    .into_iter()
                    .flat_map(move |listeners| {
                        listeners
                            .iter()
                            .map(move |event| (id.get(), event.to_string()))
                    })
            })
            .collect()
    }

    /// Returns the open widget visual attached to `id` without cloning the handle.
    pub(crate) fn widget_visual_ref(&self, id: StableNodeId) -> Option<&crate::WidgetVisualHandle> {
        self.nodes.widget_visual(id)
    }

    /// Returns the open widget visual attached to `id`, if any.
    pub fn widget_visual(&self, id: StableNodeId) -> Option<crate::WidgetVisualHandle> {
        self.nodes.widget_visual(id).cloned()
    }

    /// Returns a typed reference to `id`'s open widget visual when it matches `V`.
    pub fn widget_visual_as<V: crate::WidgetVisual>(&self, id: StableNodeId) -> Option<&V> {
        self.nodes.widget_visual(id)?.downcast_ref::<V>()
    }

    /// Returns a cloned copy of `id`'s [`crate::TextInputVisual`] when attached.
    pub fn text_input_visual(&self, id: StableNodeId) -> Option<crate::TextInputVisual> {
        self.widget_visual_as::<crate::TextInputVisual>(id).cloned()
    }

    /// Returns a typed reference to `id`'s [`crate::TextInputVisual`] without cloning.
    pub(crate) fn text_input_visual_ref(
        &self,
        id: StableNodeId,
    ) -> Option<&crate::TextInputVisual> {
        self.widget_visual_as::<crate::TextInputVisual>(id)
    }

    /// Active horizontal and vertical scrollbar bars derived for `id` in page coordinates.
    pub fn scrollbar_bars(&self, id: StableNodeId) -> Option<crate::ScrollbarBars> {
        let visual = self.widget_visual_as::<crate::ScrollbarVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.invalidation.geometry_frame.replace(None);
        let bounds = self.component_layout_box(id);
        let resolved = &self.record(id).resolved_layout;
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let node_theme = self.node_theme(id);
        let bars = bounds
            .and_then(|bounds| visual.bars_in(id, self, bounds, resolved, style_model, node_theme));
        self.invalidation.geometry_frame.set(previous);
        bars
    }

    /// Projects `id`'s open widget visual in page coordinates, matching extraction.
    pub fn widget_paint(&self, id: StableNodeId) -> Option<Arc<crate::WidgetPaintOutput>> {
        let visual = self.nodes.widget_visual(id)?;
        let style = self.painted_style(id)?;
        self.page_widget_paint(id, visual, &style)
    }

    /// Page-space geometry of a [`crate::EmptyStateVisual`] node, matching extraction.
    pub fn empty_state_geometry(&self, id: StableNodeId) -> Option<crate::EmptyStateGeometry> {
        let visual = self.widget_visual_as::<crate::EmptyStateVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.invalidation.geometry_frame.replace(None);
        let bounds = self.component_layout_box(id);
        let content = self.component_content_box(id);
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let secondary = self.secondary_text_color(id);
        let geometry = bounds.zip(content).map(|(bounds, content)| {
            visual.geometry_in(id, self, bounds, content, style_model, secondary)
        });
        self.invalidation.geometry_frame.set(previous);
        geometry
    }

    pub(crate) fn empty_state_text(
        &self,
        id: StableNodeId,
    ) -> Option<&crate::components::EmptyStateTextPresentation> {
        self.nodes.empty_state_text(id)
    }

    /// Page-space geometry of a [`crate::ActionMenuItemVisual`] node, matching extraction.
    pub fn action_menu_item_geometry(
        &self,
        id: StableNodeId,
    ) -> Option<crate::ActionMenuItemGeometry> {
        let visual = self.widget_visual_as::<crate::ActionMenuItemVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.invalidation.geometry_frame.replace(None);
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let parts = self.component_content_parts(id);
        let geometry =
            visual.geometry_from_parts(&style, &style_model.palette, self.node_theme(id), &parts);
        self.invalidation.geometry_frame.set(previous);
        Some(geometry)
    }

    pub(crate) fn select_menu_paint_room(
        &self,
        id: StableNodeId,
        opened: bool,
        menu_room: Option<crate::SelectMenuRoom>,
    ) -> (Option<crate::SelectMenuRoom>, Option<crate::LayoutViewport>) {
        if self.invalidation.geometry_frame.get() == Some(id) {
            (self.local_select_menu_room(id, opened, menu_room), None)
        } else {
            (
                menu_room,
                opened.then(|| self.document_viewport_of(id)).flatten(),
            )
        }
    }

    pub(crate) fn select_fitted_options_width(
        &self,
        id: StableNodeId,
        fit_options: bool,
    ) -> Option<f32> {
        fit_options
            .then(|| {
                self.text_natural_width(id)
                    .or_else(|| self.text_metrics(id).map(|metrics| metrics.width))
            })
            .flatten()
    }

    /// Page-space geometry of a [`crate::SelectVisual`] node, matching extraction.
    pub fn select_geometry(&self, id: StableNodeId) -> Option<crate::SelectGeometry> {
        let visual = self.widget_visual_as::<crate::SelectVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.invalidation.geometry_frame.replace(None);
        let bounds = self.component_layout_box(id);
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let node_theme = self.node_theme(id);
        let parts = self.component_content_parts(id);
        let (menu_room, viewport) =
            self.select_menu_paint_room(id, visual.opened, visual.menu_room);
        let fitted = self.select_fitted_options_width(id, visual.fit_options);
        let geometry = bounds.map(|bounds| {
            visual.geometry_from_parts(
                bounds,
                menu_room,
                viewport,
                &style,
                style_model,
                node_theme,
                &parts,
                self.chrome_text_measure(id),
                fitted,
            )
        });
        self.invalidation.geometry_frame.set(previous);
        geometry
    }

    /// Page-space geometry of a [`crate::MenuSurfaceVisual`] node, matching extraction.
    pub fn menu_surface_geometry(&self, id: StableNodeId) -> Option<crate::MenuSurfaceGeometry> {
        let visual = self.widget_visual_as::<crate::MenuSurfaceVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.invalidation.geometry_frame.replace(None);
        let bounds = self.component_layout_box(id);
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let geometry = bounds.map(|bounds| {
            visual.geometry_for(id, self, bounds, &style, style_model, self.node_theme(id))
        });
        self.invalidation.geometry_frame.set(previous);
        geometry
    }

    /// Page-space geometry of a [`crate::ModalFrameVisual`] node, matching extraction.
    pub fn modal_frame_geometry(&self, id: StableNodeId) -> Option<crate::ModalFrameGeometry> {
        let visual = self.widget_visual_as::<crate::ModalFrameVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.invalidation.geometry_frame.replace(None);
        let bounds = self.component_layout_box(id);
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let geometry = bounds.and_then(|bounds| {
            visual.geometry_for(id, self, bounds, &style, style_model, self.node_theme(id))
        });
        self.invalidation.geometry_frame.set(previous);
        geometry
    }

    /// Page-space geometry of a [`crate::RangeVisual`] node, matching extraction.
    pub fn range_geometry(&self, id: StableNodeId) -> Option<crate::RangeGeometry> {
        let visual = self.widget_visual_as::<crate::RangeVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.invalidation.geometry_frame.replace(None);
        let content = self.component_content_box(id);
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let secondary = self.secondary_text_color(id);
        let geometry = content
            .map(|content| visual.geometry_in(id, self, content, &style, style_model, secondary));
        self.invalidation.geometry_frame.set(previous);
        geometry
    }

    /// Computes a chart's current layout under its node box and theme without running extraction.
    #[cfg(feature = "charts")]
    pub fn chart_layout(&self, id: StableNodeId) -> Option<Arc<nana_ui_charts::ChartLayout>> {
        let visual = self.widget_visual_as::<crate::ChartVisual>(id)?;
        let bounds = self.component_layout_box(id)?;
        Some(visual.spec.layout_for(
            [bounds.width, bounds.height],
            self.node_theme(id),
            self.node_style_model(id),
            self.chrome_text_measure(id),
            self.animation_now(),
        ))
    }

    /// The node's `min-height` / `max-height` in px, resolved the way layout
    /// resolves them: percentages against the parent's content box (the
    /// viewport for a root), `vh` against the document viewport and `em`
    /// against the nearest declared font size. The values are in the box
    /// the node's `box-sizing` names.
    pub(crate) fn resolved_height_limits(&self, id: StableNodeId) -> Option<(f32, Option<f32>)> {
        let style = self.layout_style(id)?;
        let viewport = self.document_viewport_of(id);
        let percent_base = match self.parent_id(id) {
            Some(parent) => self
                .component_content_box(parent)
                .map(|bounds| bounds.height),
            None => viewport.map(|viewport| viewport.height),
        };
        let mut element_px = style.font_size;
        let mut ancestor = self.parent_id(id);
        while element_px.is_none()
            && let Some(node) = ancestor
        {
            element_px = self.layout_style(node).and_then(|style| style.font_size);
            ancestor = self.parent_id(node);
        }
        let fonts = nana_ui_core::FontSizeContext::new(
            nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX,
            element_px.unwrap_or(nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX),
        );
        let vp = viewport.map(|viewport| (viewport.width, viewport.height));
        Some((
            style.resolved_min_height_fonts(percent_base, vp, fonts),
            style.resolved_max_height_fonts(percent_base, vp, fonts),
        ))
    }

    pub(crate) fn component_content_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        if let Some(meta) = self.published_layout_meta(id) {
            return Some(crate::layout_result::place_box(
                self.layout_origin(id)?,
                meta.content_box,
            ));
        }
        let node = self.nodes.get(id)?;
        let bounds = self.component_layout_box(id)?;
        let padding = self.used_layout_padding(id);
        let border = node.resolved_layout.resolved_border_edges();
        Some(LayoutBox {
            x: bounds.x + border.left + padding.left,
            y: bounds.y + border.top + padding.top,
            width: (bounds.width - border.left - border.right - padding.left - padding.right)
                .max(0.0),
            height: (bounds.height - border.top - border.bottom - padding.top - padding.bottom)
                .max(0.0),
        })
    }

    pub fn accessibility(
        &self,
        id: StableNodeId,
    ) -> Option<std::borrow::Cow<'_, AccessibilityState>> {
        self.nodes
            .contains(id)
            .then(|| self.nodes.accessibility(id))
    }

    pub(crate) fn accessibility_eq(&self, id: StableNodeId, expected: &AccessibilityState) -> bool {
        self.nodes.accessibility_eq(id, expected)
    }

    pub(crate) fn accessible_label_arc(&self, id: StableNodeId) -> Option<Arc<str>> {
        self.nodes.accessible_label_arc(id)
    }

    /// The node whose text names `id` when it has no label of its own; see
    /// [`MutationQueue::set_labelled_by`].
    pub fn labelled_by(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.overlays_a11y.labelled_by.get(&id).copied()
    }

    /// Relate `id` to the node that names it, or drop the relation.
    fn set_labelled_by(&mut self, id: StableNodeId, label: Option<StableNodeId>) {
        if let Some(previous) = self.overlays_a11y.labelled_by.remove(&id)
            && let Some(controls) = self.overlays_a11y.labels.get_mut(&previous)
        {
            controls.remove(&id);
            if controls.is_empty() {
                self.overlays_a11y.labels.remove(&previous);
            }
        }
        if let Some(label) = label {
            self.overlays_a11y.labelled_by.insert(id, label);
            self.overlays_a11y
                .labels
                .entry(label)
                .or_default()
                .insert(id);
        }
    }

    /// Drop every naming relation `id` is an end of: it is gone.
    fn forget_labelled_by(&mut self, id: StableNodeId) {
        self.set_labelled_by(id, None);
        if let Some(controls) = self.overlays_a11y.labels.remove(&id) {
            for control in controls {
                self.overlays_a11y.labelled_by.remove(&control);
                if self.nodes.contains(control) {
                    self.mark(control, DirtyMask::ACCESSIBILITY);
                }
            }
        }
    }

    /// How changes inside `id` are announced, when a composite made it a
    /// live region; see [`MutationQueue::set_live`].
    pub fn live(&self, id: StableNodeId) -> Option<AccessibilityLive> {
        self.overlays_a11y.live_regions.get(&id).copied()
    }

    /// Make `id` a live region, or stop it being one.
    fn set_live(&mut self, id: StableNodeId, live: Option<AccessibilityLive>) {
        match live {
            Some(live) => {
                self.overlays_a11y.live_regions.insert(id, live);
            }
            None => {
                self.overlays_a11y.live_regions.remove(&id);
            }
        }
    }

    /// Explicit `aria-selected` override on `id`, if any (`El::aria_selected`).
    pub fn selected_override(&self, id: StableNodeId) -> Option<bool> {
        self.overlays_a11y.selected.get(&id).copied()
    }

    /// Effective `aria-selected` state of `id`, reading the explicit override
    /// first and falling back to its component's [`AccessibilityState::selected`].
    pub fn effective_selected(&self, id: StableNodeId) -> Option<bool> {
        self.overlays_a11y
            .selected
            .get(&id)
            .copied()
            .or_else(|| self.nodes.accessibility(id).selected)
    }

    fn set_selected_override(&mut self, id: StableNodeId, selected: Option<bool>) {
        match selected {
            Some(selected) => {
                self.overlays_a11y.selected.insert(id, selected);
            }
            None => {
                self.overlays_a11y.selected.remove(&id);
            }
        }
    }

    /// Visual mounting scope of `id`, if one was assigned via
    /// [`MutationQueue::set_overlay_mount`].
    pub fn overlay_mount(&self, id: StableNodeId) -> Option<nana_ui_core::OverlayMount> {
        if !self.nodes.has_overlay_mounts() {
            return None;
        }
        self.nodes.overlay_mount(id).copied()
    }

    pub fn overlay_host(&self, id: StableNodeId) -> Option<OverlayHostState> {
        self.nodes.overlay_host(id).copied()
    }

    /// Nodes that carry an `OverlayHostState`. Overlay validation iterates this
    /// instead of the entity index so cost tracks host count, not world size.
    pub(crate) fn overlay_host_ids(
        &self,
        document: DocumentId,
    ) -> impl Iterator<Item = StableNodeId> + '_ {
        self.overlays_a11y
            .overlay_hosts_by_document
            .get(&document)
            .into_iter()
            .flatten()
            .copied()
    }

    /// Drop focus and composition when dirty visual or interaction state makes
    /// the focused node ineligible.
    pub fn reconcile_focus(&mut self, ids: &[StableNodeId]) {
        // A settled document has no focused node. Building a membership set
        // of every style-dirty id just to discover that is the whole cost.
        if self.input.focused.is_empty() {
            return;
        }
        let dirty = ids.iter().copied().collect::<HashSet<_>>();
        let invalid_focus = self
            .input
            .focused
            .iter()
            .filter_map(|(&document, &id)| {
                let invalid = dirty.contains(&id)
                    && (!self.record(id).resolved.0.visible
                        || self.record(id).resolved.0.inert
                        || !self.record(id).interaction.focusable);
                invalid.then_some((document, id))
            })
            .collect::<Vec<_>>();
        for (document, id) in invalid_focus {
            self.input.focused.remove(&document);
            self.remove_ime(id);
            self.mark_focus_changed(id);
        }
    }

    pub fn layout_inputs(&self, ids: &[StableNodeId]) -> Result<Vec<LayoutInput>, UiWorldError> {
        if !ids.is_empty() {
            self.record_hot_path_allocation(1, ids.len().saturating_mul(size_of::<LayoutInput>()));
        }
        ids.iter().map(|&id| self.layout_input(id)).collect()
    }

    /// Whether `id` is a modal frame, whose children are placed by the modal
    /// slot contract (the Overlay formatting context).
    pub(crate) fn places_modal_slots(&self, id: StableNodeId) -> bool {
        self.invalidation.modal_frames != 0
            && self
                .widget_visual_ref(id)
                .is_some_and(crate::WidgetVisualHandle::is_modal_frame)
    }

    /// Project a single layout input without allocating a batch container.
    pub(crate) fn layout_input(&self, id: StableNodeId) -> Result<LayoutInput, UiWorldError> {
        let parent = self
            .nodes
            .get(id)
            .ok_or(UiWorldError::MissingNode(id))?
            .hierarchy
            .parent;
        let view =
            self.layout_input_view(id, |style| crate::layout_engine::StyleFacts::of(style))?;
        Ok(LayoutInput {
            id,
            parent,
            children: Arc::clone(view.children),
            style: view.style.to_arc(),
            writing: view.writing,
            containing_writing: view.containing_writing,
            text_metrics: view.text_metrics,
            modal: self.modal_layout_input(id),
        })
    }

    /// What a modal frame's slots lay out from: its kind, slots and the
    /// metrics of its texts. `None` for every other node.
    pub(crate) fn modal_layout_input(&self, id: StableNodeId) -> Option<crate::ModalLayoutInput> {
        if self.invalidation.modal_frames == 0 {
            return None;
        }
        self.widget_visual_ref(id)?.modal_layout_input(id, self)
    }

    /// [`Self::layout_input`] as a view of the record: the child list and,
    /// for an ordinary mounted box in a world without overlays, motion or
    /// length tracks, the layout style are borrowed from the record rather
    /// than cloned, so a pass that reads every node does not take and drop
    /// two references per node.
    pub(crate) fn layout_input_view(
        &self,
        id: StableNodeId,
        facts_of: impl FnOnce(&Arc<nana_ui_core::LayoutStyle>) -> crate::layout_engine::StyleFacts,
    ) -> Result<LayoutInputView<'_>, UiWorldError> {
        let plain_world = !self.has_layout_length_overlays()
            && self.overlays_a11y.overlay_host_nodes.is_empty()
            && self.invalidation.z_index_nodes == 0
            && self.tree.detached_mounted.is_empty()
            && !self.nodes.has_widget_visuals()
            && self.invalidation.modal_frames == 0
            && self.invalidation.menu_surfaces == 0
            && self.invalidation.list_leading_visuals == 0;
        let record = self.nodes.get(id).ok_or(UiWorldError::MissingNode(id))?;
        let has_text = matches!(record.kind.as_ref(), NodeKind::Text) || record.has_text;
        // Header block space is already represented by Card's title band.
        // Its text still contributes preferred inline width, not body height.
        let text_metrics = has_text
            .then(|| self.nodes.text_metrics(id))
            .map(|mut metrics| {
                if self
                    .widget_visual_ref(id)
                    .is_some_and(crate::WidgetVisualHandle::uses_card_title_typography)
                {
                    metrics.height = 0.0;
                }
                metrics
            });
        let resolved_layout = &record.resolved_layout;
        let style = if plain_world
            && record.mount == MountState::Mounted
            && !resolved_layout.omits_box()
            && !resolved_layout.has_logical_box_edges()
        {
            LayoutStyleRef::Borrowed(resolved_layout)
        } else {
            LayoutStyleRef::Owned(self.hit_motion_layout(id))
        };
        Ok(LayoutInputView {
            children: &record.hierarchy.children,
            // An owned style (motion, overlays, a hidden box) is decided
            // when a measurement asks: most such nodes are only placed.
            facts: match &style {
                LayoutStyleRef::Borrowed(style) => facts_of(style),
                LayoutStyleRef::Owned(_) => crate::layout_engine::StyleFacts::UNKNOWN,
            },
            style,
            writing: record_writing(record),
            containing_writing: record_containing_writing(record),
            text_metrics,
        })
    }

    /// While set, a commit leaves the results of the boxes it writes to the
    /// publish of the layout pass scope that follows it.
    pub(crate) fn set_pass_publishes_written_boxes(&mut self, deferred: bool) {
        self.invalidation.pass_publishes_written_boxes = deferred;
    }

    pub(crate) fn pass_publishes_written_boxes(&self) -> bool {
        self.invalidation.pass_publishes_written_boxes
    }

    pub(crate) fn write_layout_padding(
        &mut self,
        id: StableNodeId,
        padding: nana_ui_core::PaddingSpec,
    ) -> bool {
        let changed = self.nodes.layout_padding(id) != Some(padding);
        self.nodes.set_layout_padding(id, padding);
        if changed {
            self.nodes
                .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
            // Rules read the content box, which padding moves.
            self.note_responsive_resize(id);
        }
        changed
    }

    /// Record which page axes layout placed `id`'s children from the far
    /// end. Returns whether that changed, which moves the scroll origin.
    pub(crate) fn write_layout_far_start(&mut self, id: StableNodeId, far: [bool; 2]) -> bool {
        let changed = self.nodes.layout_far_start(id) != Some(far);
        self.nodes.set_layout_far_start(id, far);
        changed
    }

    pub(crate) fn layout_far_start(&self, id: StableNodeId) -> Option<[bool; 2]> {
        self.nodes
            .contains(id)
            .then(|| self.nodes.layout_far_start(id))
            .flatten()
    }

    #[inline]
    pub(crate) fn auto_view_slot(&self, id: StableNodeId) -> Option<u16> {
        self.nodes.auto_view_slot(id)
    }

    #[inline]
    pub(crate) fn set_auto_view_slot(&mut self, id: StableNodeId, slot: Option<u16>) {
        self.nodes.set_auto_view_slot(id, slot);
    }

    #[inline]
    pub(crate) fn is_elided_stack(&self, id: StableNodeId) -> bool {
        self.nodes.elided_stack_flags(id) != 0
    }

    #[inline]
    pub(crate) fn set_elided_stack_flags(&mut self, id: StableNodeId, flags: u8) {
        self.nodes.set_elided_stack_flags(id, flags);
    }

    pub(crate) fn elided_stack(&self, id: StableNodeId) -> Option<crate::view_components::Stack> {
        let flags = self.nodes.elided_stack_flags(id);
        if flags == 0 {
            return None;
        }
        let record = self.nodes.get(id)?;
        let style = self
            .overlays_a11y
            .responsive
            .authored(id)
            .map(|authored| Arc::new(authored.clone()))
            .unwrap_or_else(|| Arc::clone(&record.style));
        Some(crate::view_components::Stack::from_elided(style, flags))
    }

    /// Record the formatting context layout ran for `id`'s children, or
    /// none for a node layout now gives no box (#392). A different context
    /// than the recorded one is a transition: the record and its generation
    /// move, and the children keep their identity. The next result publish
    /// rebuilds the node's fragments under the new kind. Returns whether a
    /// recorded context changed or went.
    pub(crate) fn write_formatting_context(
        &mut self,
        id: StableNodeId,
        context: Option<crate::FormattingContextKind>,
    ) -> bool {
        let Some(record) = self.nodes.get_mut(id) else {
            return false;
        };
        let previous = std::mem::replace(&mut record.layout_context, context);
        if previous == context {
            return false;
        }
        self.nodes.bump_layout_context_generation(id);
        if previous.is_some() && context.is_some() {
            self.bump_last_counters(|counters| counters.record_layout_context_transitions(1));
        }
        previous.is_some()
    }

    fn layout_node_participation(&self, id: StableNodeId) -> Option<crate::ParticipationKind> {
        let node = self.nodes.get(id)?;
        let content = self.layout_content_kind(id)?;
        let parent = node
            .hierarchy
            .parent
            .and_then(|parent| self.nodes.get(parent));
        let parent_context = parent.and_then(|parent| parent.layout_context);
        match parent {
            None => Some(crate::ParticipationKind::Root),
            Some(_) => parent_context.map(|context| {
                if self.inline_parent(id).is_some() && !node.resolved_layout.omits_box() {
                    crate::ParticipationKind::AtomicInline
                } else {
                    crate::ParticipationKind::classify(context, &node.resolved_layout, content)
                }
            }),
        }
    }

    /// The explicit rect `id` was given ([`crate::MutationQueue::set_explicit_rect`]):
    /// where a `display: free` parent places it, in logical pixels of the
    /// parent's padding box. It is kept whatever the parent's display, and
    /// read only while the parent is a free container.
    pub fn explicit_rect(&self, id: StableNodeId) -> Option<crate::ExplicitRect> {
        self.nodes.explicit_rect(id).copied()
    }

    /// A border box nothing inside can change: a fixed width and height, or
    /// a free item's explicit rect.
    fn fixed_border(&self, id: StableNodeId, record: &NodeRecord) -> bool {
        definite_fixed_border(record.resolved_layout.as_ref())
            || (self.nodes.explicit_rect(id).is_some() && self.is_free_item(id))
    }

    /// Whether a `display: free` parent places `id` by its explicit rect.
    pub(crate) fn is_free_item(&self, id: StableNodeId) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let parent = record
            .hierarchy
            .parent
            .and_then(|parent| self.nodes.get(parent));
        crate::layout_node::is_free_item(
            parent.map(|parent| parent.resolved_layout.as_ref()),
            record.resolved_layout.as_ref(),
        )
    }

    /// Store `rect` for `id` and seed only what it moves. Origin only: the
    /// node is placed again where its parent's padding box puts it, nothing
    /// measures. A new size also lays out what is inside it. A free item
    /// sizes nothing of its parent and couples no sibling, so nothing beside
    /// it or outside the free container is measured.
    pub(crate) fn apply_explicit_rect(
        &mut self,
        id: StableNodeId,
        rect: Option<crate::ExplicitRect>,
    ) {
        let previous = self.nodes.explicit_rect(id).copied();
        if previous == rect {
            return;
        }
        self.nodes.set_explicit_rect(id, rect);
        if !self.is_free_item(id) || !self.presence_live(id) {
            return;
        }
        use nana_ui_core::LayoutStyleChange as Change;
        let mut changed = Change::POSITION;
        match (previous, rect) {
            (Some(previous), Some(rect)) => {
                if previous.width.to_bits() != rect.width.to_bits() {
                    changed = changed.union(Change::WIDTH);
                }
                if previous.height.to_bits() != rect.height.to_bits() {
                    changed = changed.union(Change::HEIGHT);
                }
            }
            // To or from the shrink-to-fit size.
            _ => changed = changed.union(Change::WIDTH).union(Change::HEIGHT),
        }
        // Classified as the same write to `left` / `top` / `width` /
        // `height` would be; the frontier keeps it off the siblings and the
        // parent's measure because the node is a free item.
        let invalidation = self.classify_layout_change(id, changed);
        self.record_layout_invalidation(id, invalidation);
    }

    /// The layout view of `id` (Issue #197): its resolved intent, content,
    /// the context it establishes, and how it takes part in its parent's.
    /// Built from the records on each call; nothing is retained for it.
    pub fn layout_node(&self, id: StableNodeId) -> Option<crate::LayoutNode> {
        let node = self.nodes.get(id)?;
        let content = self.layout_content_kind(id)?;
        let parent = node
            .hierarchy
            .parent
            .and_then(|parent| self.nodes.get(parent));
        let parent_context = parent.and_then(|parent| parent.layout_context);
        let participation = match parent {
            None => Some(crate::ParticipationKind::Root),
            Some(_) => parent_context.map(|context| {
                if self.inline_parent(id).is_some() && !node.resolved_layout.omits_box() {
                    crate::ParticipationKind::AtomicInline
                } else {
                    crate::ParticipationKind::classify(context, &node.resolved_layout, content)
                }
            }),
        };
        Some(crate::LayoutNode {
            id,
            intent: Arc::clone(&node.resolved_layout),
            content,
            established: established_context(node),
            parent_context,
            participation,
            context_generation: self.nodes.layout_context_generation(id),
            result_generation: self.published_layout_meta(id).map(|meta| meta.generation),
        })
    }

    pub(crate) fn layout_content_kind(&self, id: StableNodeId) -> Option<crate::LayoutContentKind> {
        let node = self.nodes.get(id)?;
        Some(if !node.hierarchy.children.is_empty() {
            crate::LayoutContentKind::Children
        } else if self.shows_replaced_content(id) {
            crate::LayoutContentKind::Replaced
        } else if record_has_text(node) {
            crate::LayoutContentKind::Text
        } else {
            crate::LayoutContentKind::Empty
        })
    }

    pub(crate) fn established_layout_context(
        &self,
        id: StableNodeId,
    ) -> Option<crate::FormattingContextKind> {
        self.nodes.get(id).and_then(established_context)
    }

    /// Padding resolved by the layout pass, including its containing block and font.
    pub(crate) fn used_layout_padding(&self, id: StableNodeId) -> nana_ui_core::PaddingSpec {
        if let Some(px) = self
            .presentation_length_overlay(id)
            .and_then(|overlay| overlay.padding())
        {
            return nana_ui_core::PaddingSpec::uniform(px);
        }
        let record = self.record(id);
        self.nodes.layout_padding(id).unwrap_or_else(|| {
            // Intent (radius / control height / padding) lives on
            // `resolved_layout`. Falling back to the authored box would
            // report 0 for every control that named its inset instead of
            // spending the token.
            record
                .resolved_layout
                .resolved_padding_against(Some(self.nodes.local_layout(id).width))
        })
    }

    /// Layout-facing style without assembling a [`LayoutInput`].
    ///
    /// Parked, detached, and inactive-overlay nodes match [`Self::layout_inputs`]:
    /// the returned style reports [`nana_ui_core::LayoutStyle::omits_box`].
    pub(crate) fn layout_style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        self.contains(id)
            .then(|| self.motion_layout(id, &self.effective_layout_style(id)))
    }

    /// True when `effective_layout_style` for `parent`'s children reduces to
    /// each child's own `style.layout`, with no adjustment derived from an
    /// ancestor.
    ///
    /// The scoped layout engine reuses a container's cached child placement
    /// when nothing in the change closure altered it. That is only sound if a
    /// child's layout style cannot change without the child itself being
    /// marked LAYOUT-dirty -- and `set_style` guarantees exactly that (it
    /// marks the subtree when layout semantics change). The two escapes are
    /// the adjustments below, each derived from an ancestor and so able to
    /// move without touching the child:
    ///
    /// - `overlay_branch_active`, when the parent is an overlay host;
    /// - `menu_branch_open` / `parent_triggered_overlay`, when the parent is a
    ///   menu surface.
    ///
    /// `presence_live` is ancestor-derived as well, but it never differs
    /// between a parent and its children: only an unlinked root is ever
    /// `detached`, a child in `parent`'s list shares its mount state, and a
    /// subtree that comes back is inserted with all of it marked dirty. A
    /// parked node elsewhere in the world does not disable the plans.
    ///
    /// Logical edges landed in an inherited writing context are derived from
    /// an ancestor too, but they do not escape: changing an ancestor's
    /// `writing-mode` or `direction` marks its whole subtree LAYOUT-dirty.
    ///
    /// Reporting false is always safe: it only costs the caller its fast path.
    pub(crate) fn children_layout_style_is_local(&self, parent: StableNodeId) -> bool {
        (self.overlays_a11y.overlay_host_nodes.is_empty() || self.overlay_host(parent).is_none())
            && !self
                .widget_visual_ref(parent)
                .is_some_and(crate::WidgetVisualHandle::is_menu_surface)
    }

    /// Whether `id` lays out with no box of its own: its style omits one, or
    /// it is parked, in an overlay branch that is not active, or an item of a
    /// closed menu. [`Self::effective_layout_style`] reports it hidden.
    fn omits_layout_box(&self, id: StableNodeId) -> bool {
        self.record(id).resolved_layout.omits_box()
            || !self.presence_live(id)
            || !self.overlay_branch_active(id)
            || !self.menu_branch_open(id)
    }

    fn effective_layout_style(&self, id: StableNodeId) -> Arc<nana_ui_core::LayoutStyle> {
        let record = self.record(id);
        let mut style = Arc::clone(&record.resolved_layout);
        if self.omits_layout_box(id) {
            Arc::make_mut(&mut style).hidden = true;
            return style;
        }
        // A box that inherits its writing mode or direction resolved its
        // logical edges against its own declarations; land them against the
        // context it actually lays out in. The context comparison goes first:
        // it reads the record and two style fields, where the logical edges
        // sit on three other cache lines of a 4.8 KB style.
        let writing = record_writing(record);
        if writing != style.writing_context() && style.has_logical_box_edges() {
            Arc::make_mut(&mut style).resolve_logical_box_edges_in(writing);
        }
        if self.lone_list_trailing(id, &style) {
            Arc::make_mut(&mut style).justify_content = nana_ui_core::JustifySpec::End;
        }
        if let Some(overlay) = self.parent_triggered_overlay(id) {
            apply_triggered_overlay(Arc::make_mut(&mut style), overlay);
            if self.groups_action_menu_items(id, &style) {
                // A keyed list or a conditional block of commands: its items
                // are spaced as the menu spaces its own.
                let menu_theme = self.node_theme(id);
                let item_gap = menu_theme
                    .spacing()
                    .resolve(menu_theme.recipes().menu().item_gap);
                Arc::make_mut(&mut style).gap = Some(LengthSpec::Px(item_gap));
            }
        }
        style
    }

    /// Whether `id` is a list row that spreads its leading and trailing
    /// slots to its two ends while the leading slot (and a content slot)
    /// lays out no box. The row then lays out as one with no leading slot:
    /// its trailing slot stays at the end rather than becoming the only item
    /// `SpaceBetween` puts at the start (#333). A slot's visibility is its
    /// own style, so the row's projection cannot see it change.
    fn lone_list_trailing(&self, id: StableNodeId, style: &nana_ui_core::LayoutStyle) -> bool {
        if style.justify_content != nana_ui_core::JustifySpec::SpaceBetween {
            return false;
        }
        let Some((leading, content)) = self
            .widget_visual_ref(id)
            .and_then(crate::WidgetVisualHandle::lone_list_trailing_slots)
        else {
            return false;
        };
        let no_box = |slot: StableNodeId| !self.contains(slot) || self.omits_layout_box(slot);
        no_box(leading) && content.is_none_or(no_box)
    }

    /// Whether `id`, a child of an open action menu, is a plain column that
    /// holds menu items (the container of an `each` or `when`) rather than
    /// an item of its own.
    fn groups_action_menu_items(
        &self,
        id: StableNodeId,
        style: &nana_ui_core::LayoutStyle,
    ) -> bool {
        self.widget_visual_ref(id).is_none()
            && matches!(style.direction, Some(nana_ui_core::FlexDirection::Column))
            && self.record(id).hierarchy.parent.is_some_and(|parent| {
                self.widget_visual_ref(parent)
                    .is_some_and(crate::WidgetVisualHandle::is_action_menu_open)
            })
    }

    /// The content of an open triggered menu (Popover, ActionMenu,
    /// HoverCard) as layout, hit testing and paint all see it: a
    /// viewport-fixed surface above the page. Extraction reads the resolved
    /// style, so it folds this in the same way `effective_layout_style` does.
    pub(super) fn triggered_overlay_layout(
        &self,
        id: StableNodeId,
        layout: &mut Arc<nana_ui_core::LayoutStyle>,
    ) {
        // Open triggered menus count among the z-index nodes; with none of
        // those there is no overlay content to find.
        if self.invalidation.z_index_nodes == 0 {
            return;
        }
        if let Some(overlay) = self.parent_triggered_overlay(id) {
            apply_triggered_overlay(Arc::make_mut(layout), overlay);
        }
    }

    fn parent_triggered_overlay(&self, id: StableNodeId) -> Option<crate::TriggeredMenuOverlay> {
        let parent = self.record(id).hierarchy.parent?;
        let overlay = self
            .widget_visual_ref(parent)
            .and_then(crate::WidgetVisualHandle::triggered_menu_overlay)?;
        (overlay.trigger_content != Some(id)).then_some(overlay)
    }

    pub(crate) fn parent_triggered_overlay_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        if self.overlays_a11y.triggered_overlays.is_empty() {
            return None;
        }
        let parent = self.record(id).hierarchy.parent?;
        let overlay = self
            .widget_visual_ref(parent)
            .and_then(crate::WidgetVisualHandle::triggered_menu_overlay)?;
        (overlay.trigger_content != Some(id)).then_some(parent)
    }

    pub(crate) fn resolve_overlay_mount(
        &self,
        id: StableNodeId,
        mount: nana_ui_core::OverlayMount,
    ) -> crate::VisualParent {
        match mount {
            nana_ui_core::OverlayMount::TopLayer => crate::VisualParent::TopLayer,
            nana_ui_core::OverlayMount::NearestModal => {
                let mut cursor = self.record(id).hierarchy.parent;
                while let Some(ancestor) = cursor {
                    if self
                        .widget_visual_ref(ancestor)
                        .is_some_and(crate::WidgetVisualHandle::is_modal_frame)
                    {
                        return crate::VisualParent::Modal(ancestor);
                    }
                    cursor = self
                        .nodes
                        .get(ancestor)
                        .and_then(|record| record.hierarchy.parent);
                }
                crate::VisualParent::Root
            }
            nana_ui_core::OverlayMount::Inline => {
                if let Some(menu_id) = self.parent_triggered_overlay_id(id) {
                    crate::VisualParent::Overlay(menu_id)
                } else if let Some(parent) = self.record(id).hierarchy.parent {
                    crate::VisualParent::Node(parent)
                } else {
                    crate::VisualParent::Root
                }
            }
        }
    }

    pub(crate) fn flex_row_main_gap(&self, id: StableNodeId, width: f32, height: f32) -> f32 {
        self.nodes.get(id).map_or(0.0, |node| {
            node.resolved_layout.main_gap_against(
                nana_ui_core::FlexDirection::Row,
                nana_ui_core::ParentBox::from_viewport(width, height),
            )
        })
    }

    fn record(&self, id: StableNodeId) -> &NodeRecord {
        self.nodes
            .get(id)
            .expect("entity must have runtime component")
    }

    fn record_mut(&mut self, id: StableNodeId) -> &mut NodeRecord {
        self.nodes
            .get_mut(id)
            .expect("entity must have runtime component")
    }

    /// Records a change to a node's authored text.
    fn invalidate_text_content(&mut self, id: StableNodeId) {
        self.nodes
            .invalidate_text(id, crate::text_node::TextDirty::CONTENT);
        if !self.record(id).has_text {
            // Emptied text has nothing to draw, and a non-Text element without
            // text never reaches a text pass that could release it later.
            self.nodes.release_text_layout(id);
        }
    }

    /// Gives `id` rich text, or drops its spans (`None`), pricing the change
    /// by the tier it touches ([`crate::text_node::classify_rich_change`]).
    pub(super) fn apply_rich_text(
        &mut self,
        id: StableNodeId,
        rich: Option<&nana_ui_core::RichText>,
    ) {
        use crate::text_node::TextDirty;
        let previous = self.nodes.rich_text(id).cloned();
        let text_changed =
            rich.is_some_and(|rich| self.nodes.text_content(id).value.as_str() != rich.text());
        if text_changed && self.nodes.rich_editor_marks(id).is_some() {
            self.nodes.set_text_viewport_pin(id, None);
        }
        let mut dirty = if text_changed {
            // New text: whatever spans it has are styled afresh, and the
            // scene's paint runs follow them.
            let mut dirty = TextDirty::PAINT;
            if rich.is_some_and(|rich| !rich.objects().is_empty()) {
                dirty |= TextDirty::CONSTRAINT;
            }
            if rich.is_some_and(|rich| {
                rich.spans()
                    .iter()
                    .any(|(_, style)| !style.shape.is_empty())
            }) {
                dirty |= TextDirty::SHAPE_STYLE;
            }
            dirty
        } else {
            crate::text_node::classify_rich_change(previous.as_ref(), rich)
        };
        // The content class was settled above; what is left are the tiers.
        dirty = dirty.intersection(
            TextDirty::SHAPE_STYLE
                .union(TextDirty::CONSTRAINT)
                .union(TextDirty::PAINT)
                .union(TextDirty::GLYPH_PRESENTATION),
        );
        let canonical = rich.cloned().map(crate::InlineContent::from);
        self.nodes.set_rich_text(
            id,
            canonical
                .as_ref()
                .and_then(crate::InlineContent::rich_text)
                .cloned(),
        );
        let mut marks = 0u16;
        if text_changed {
            self.mark_inline_text_owners(id);
            let rich = rich.expect("text only changes when a value was given");
            self.nodes.set_text_content(
                id,
                crate::TextContent::new(std::sync::Arc::clone(rich.shared_text())),
            );
            self.invalidate_text_content(id);
            marks |= DirtyMask::TEXT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY;
        }
        self.nodes.invalidate_text(id, dirty);
        if dirty.intersects(TextDirty::SHAPE_STYLE.union(TextDirty::CONSTRAINT)) {
            marks |= DirtyMask::TEXT | DirtyMask::RENDER;
        }
        if dirty.contains(TextDirty::PAINT) {
            marks |= DirtyMask::RENDER;
        }
        // An effect index alone is read by the presentation layer only: the
        // scene carries the new per-glyph table and no text pass runs.
        if dirty.contains(TextDirty::GLYPH_PRESENTATION) {
            marks |= DirtyMask::RENDER;
        }
        if marks != 0 {
            self.mark(id, marks);
        }
        self.recompute_node_theme_deps(id);
    }

    /// The writing mode and direction `id` lays out in. See
    /// [`record_writing`].
    pub(crate) fn layout_writing(&self, id: StableNodeId) -> nana_ui_core::WritingContext {
        self.nodes
            .get(id)
            .map_or_else(Default::default, record_writing)
    }

    /// The writing context of `id`'s containing block: its parent's (its own
    /// for a root). `id`'s percentage margins and paddings resolve against
    /// that block's inline size (CSS Box Model §5), so a node that sets a
    /// writing mode orthogonal to its parent's still resolves on the parent's
    /// inline axis.
    pub(crate) fn containing_writing(&self, id: StableNodeId) -> nana_ui_core::WritingContext {
        self.nodes
            .get(id)
            .map_or_else(Default::default, record_containing_writing)
    }

    /// The corner radius `id`'s resolved layout carries: its design intent
    /// resolved against the installed theme, or the number it wrote.
    pub(crate) fn resolved_border_radius(&self, id: StableNodeId) -> Option<f32> {
        if !self.contains(id) {
            return None;
        }
        self.record(id).resolved_layout.border_radius
    }

    /// Border width carried by `id`'s resolved layout, or `0.0` when absent.
    pub(crate) fn resolved_border_width(&self, id: StableNodeId) -> f32 {
        self.nodes
            .get(id)
            .map_or(0.0, |record| record.resolved_layout.resolved_border_width())
    }

    /// Resolved border edges carried by `id`'s resolved layout.
    pub(crate) fn resolved_border_edges(&self, id: StableNodeId) -> nana_ui_core::PaddingSpec {
        self.nodes.get(id).map_or_else(Default::default, |record| {
            record.resolved_layout.resolved_border_edges()
        })
    }

    /// The node's parent, without copying the node as [`Self::node`] does.
    pub fn parent_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.nodes.get(id)?.hierarchy.parent
    }

    /// The node's element tag, without copying the node as [`Self::node`]
    /// does; `None` for a node that is not an element or not in the world.
    pub(crate) fn element_tag(&self, id: StableNodeId) -> Option<&str> {
        match self.nodes.get(id)?.kind.as_ref() {
            NodeKind::Element { tag } => Some(tag),
            _ => None,
        }
    }

    /// The node's children, without copying the node as [`Self::node`] does;
    /// empty for a node that is not in the world.
    pub(crate) fn child_ids(&self, id: StableNodeId) -> &[StableNodeId] {
        self.nodes
            .get(id)
            .map_or(&[], |node| node.hierarchy.children.as_slice())
    }

    fn live_input_target_count(&self) -> usize {
        let mut ids = HashSet::new();
        ids.extend(self.input.focused.values().copied());
        ids.extend(self.input.pointer_hover.values().copied());
        ids.extend(self.input.pointer_press.values().copied());
        ids.extend(self.input.pointer_captures.values().copied());
        ids.len()
    }

    pub(crate) fn presence_live(&self, id: StableNodeId) -> bool {
        // One query: a memo made for it would remember nothing past it, so
        // walk without one rather than fill and drop a table per call.
        self.is_mounted(id)
            && (self.tree.detached_mounted.is_empty() || self.presence_live_walk(id))
    }

    fn presence_live_memo(&self, id: StableNodeId, memo: &mut AncestorMemo) -> bool {
        if !self.is_mounted(id) {
            return false;
        }
        // Every detached root is parked, and a mounted node cannot sit under
        // a parked one: nothing to walk.
        if self.tree.detached_mounted.is_empty() {
            debug_assert!(
                self.presence_live_walk(id),
                "{id:?} sits under a detached root"
            );
            return true;
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut live = true;
        while let Some(node) = current {
            if let Some(&known) = memo.live.get(&node) {
                live = known;
                break;
            }
            memo.chain.push(node);
            if self.tree.detached.contains(&node) {
                live = false;
                break;
            }
            current = self.parent_id(node);
        }
        for node in memo.chain.drain(..) {
            memo.live.insert(node, live);
        }
        live
    }

    /// `presence_live` by the ancestor walk alone.
    fn presence_live_walk(&self, id: StableNodeId) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            if self.tree.detached.contains(&node) {
                return false;
            }
            current = self.parent_id(node);
        }
        true
    }

    fn presence_flags_of(&self, id: StableNodeId) -> PresenceFlags {
        let widget_visual = self.widget_visual_ref(id);
        let record = self.nodes.get(id);
        debug_assert!(
            record.is_none_or(|record| record.layout_depends_on_viewport
                == record.resolved_layout.depends_on_viewport()),
            "{id:?}'s cached viewport dependency went stale"
        );
        let triggered = widget_visual.is_some_and(|w| w.triggered_menu_overlay().is_some());
        PresenceFlags {
            confirm: widget_visual.is_some_and(crate::WidgetVisualHandle::is_confirm_modal),
            modal_frame: widget_visual.is_some_and(crate::WidgetVisualHandle::is_modal_frame),
            menu_surface: widget_visual.is_some_and(crate::WidgetVisualHandle::is_menu_surface),
            clip: widget_visual.is_some_and(|w| w.clips_descendants() || w.is_modal_frame()),
            list_leading: widget_visual.is_some_and(|w| w.lone_list_trailing_slots().is_some()),
            z_index: record.is_some_and(|record| record.resolved_layout.z_index.is_some())
                || triggered,
            inert: record.is_some_and(|record| record.style.layout.inert),
            triggered,
            select_menu: widget_visual.is_some_and(crate::WidgetVisualHandle::is_open_select_menu),
            viewport: record.is_some_and(|record| record.layout_depends_on_viewport),
        }
    }

    fn apply_presence_flags(&mut self, id: Option<StableNodeId>, next: PresenceFlags) {
        let previous = id
            .and_then(|id| self.tree.presence_flags.get(&id).copied())
            .unwrap_or(PresenceFlags::NONE);
        if previous == next {
            return;
        }
        self.note_presence_counts(previous.confirm, next.confirm, previous.clip, next.clip);
        bump_presence(
            &mut self.invalidation.modal_frames,
            previous.modal_frame,
            next.modal_frame,
        );
        bump_presence(
            &mut self.invalidation.menu_surfaces,
            previous.menu_surface,
            next.menu_surface,
        );
        self.note_z_index_presence(previous.z_index, next.z_index);
        bump_presence(
            &mut self.invalidation.inert_nodes,
            previous.inert,
            next.inert,
        );
        bump_presence(
            &mut self.invalidation.list_leading_visuals,
            previous.list_leading,
            next.list_leading,
        );
        bump_presence(
            &mut self.invalidation.viewport_basis_nodes,
            previous.viewport,
            next.viewport,
        );
        if let Some(id) = id
            && let Some(document) = self.document_of(id)
        {
            if next.viewport {
                self.invalidation
                    .viewport_basis
                    .entry(document)
                    .or_default()
                    .insert(id);
            } else if let Some(ids) = self.invalidation.viewport_basis.get_mut(&document) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.invalidation.viewport_basis.remove(&document);
                }
            }
        }
        if let Some(id) = id
            && previous.triggered != next.triggered
        {
            if next.triggered {
                self.overlays_a11y.triggered_overlays.insert(id);
            } else {
                self.overlays_a11y.triggered_overlays.remove(&id);
            }
        }
        if let Some(id) = id
            && previous.select_menu != next.select_menu
        {
            if next.select_menu {
                self.overlays_a11y.select_menus.insert(id);
            } else {
                self.overlays_a11y.select_menus.remove(&id);
            }
        }
        if let Some(id) = id {
            if next == PresenceFlags::NONE {
                self.tree.presence_flags.remove(&id);
            } else {
                self.tree.presence_flags.insert(id, next);
            }
        }
    }

    fn sync_node_presence(&mut self, id: StableNodeId) {
        if !self.nodes.contains(id) {
            return;
        }
        let next = if self.presence_live(id) {
            self.presence_flags_of(id)
        } else {
            PresenceFlags::NONE
        };
        self.apply_presence_flags(Some(id), next);
    }

    fn sync_subtree_presence(&mut self, root: StableNodeId) {
        for id in self.subtree_ids(root) {
            self.sync_node_presence(id);
        }
    }

    pub fn uses_viewport_basis(&self) -> bool {
        self.invalidation.viewport_basis_nodes != 0
    }

    pub fn viewport_basis_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.invalidation
            .viewport_basis
            .values()
            .flat_map(|ids| ids.iter().copied())
    }

    /// Mounted viewport-dependent nodes in one document, without scanning other documents.
    /// Declares that `id` accepts drops, replacing any previous declaration.
    pub(crate) fn set_drop_target(&mut self, id: StableNodeId, accepts: nana_ui_core::DropAccepts) {
        self.overlays_a11y.drop_targets.insert(id, accepts);
    }

    /// Stops `id` accepting drops. Returns whether it was a target.
    pub(crate) fn clear_drop_target(&mut self, id: StableNodeId) -> bool {
        self.overlays_a11y.drop_targets.remove(&id).is_some()
    }

    /// Every node currently registered as a drop target.
    pub(crate) fn drop_target_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.overlays_a11y.drop_targets.keys().copied()
    }

    pub(crate) fn drop_target(&self, id: StableNodeId) -> Option<&nana_ui_core::DropAccepts> {
        self.overlays_a11y.drop_targets.get(&id)
    }

    pub(crate) fn drop_hover(&self) -> Option<(StableNodeId, nana_ui_core::DropEffect)> {
        self.overlays_a11y.drop_hover
    }

    pub(crate) fn set_drop_hover(
        &mut self,
        hover: Option<(StableNodeId, nana_ui_core::DropEffect)>,
    ) -> bool {
        if self.overlays_a11y.drop_hover == hover {
            return false;
        }
        if let Some((id, _)) = self.overlays_a11y.drop_hover {
            self.mark(id, DirtyMask::RENDER);
        }
        if let Some((id, _)) = hover {
            self.mark(id, DirtyMask::RENDER);
        }
        self.overlays_a11y.drop_hover = hover;
        true
    }

    /// Records the viewport a document was laid out against.
    pub(crate) fn set_document_viewport(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) {
        self.invalidation
            .document_viewports
            .insert(document, viewport);
    }

    /// Viewport `document` was last laid out against.
    pub(crate) fn document_viewport(&self, document: DocumentId) -> Option<crate::LayoutViewport> {
        self.invalidation.document_viewports.get(&document).copied()
    }

    /// What a viewport resize of `document` from `previous` to `viewport`
    /// seeds. Each root: the constraint it hands its children moved on the
    /// axes the viewport did, so only the children that consume those axes
    /// lay out again. Each box that resolves against a side of the viewport
    /// that moved: its own size may move on either axis, as a sizing write
    /// would. A `vw` box is not seeded by a height-only resize. With no
    /// previous viewport, the whole document.
    pub(crate) fn viewport_resize_seeds(
        &self,
        document: DocumentId,
        previous: Option<crate::LayoutViewport>,
        viewport: crate::LayoutViewport,
    ) -> Vec<crate::LayoutFrontierSeed> {
        let roots = self.document_roots(document);
        let Some(previous) = previous else {
            return roots
                .into_iter()
                .chain(self.viewport_basis_ids_for(document))
                .map(crate::LayoutFrontierSeed::layout)
                .collect();
        };
        let inline = previous.width.to_bits() != viewport.width.to_bits();
        let block = previous.height.to_bits() != viewport.height.to_bits();
        if !inline && !block {
            return Vec::new();
        }
        let mut axes = LayoutDependencyFootprint::NONE;
        if inline {
            axes = axes.union(LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT);
        }
        if block {
            axes = axes.union(LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT);
        }
        let root = LayoutInvalidation::new(
            LayoutInvalidationSource::Viewport,
            InvalidationReason::VIEWPORT,
            InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
            LayoutStyleChange::SIZING,
            axes,
        );
        let basis = LayoutInvalidation {
            source: LayoutInvalidationSource::Viewport,
            reason: InvalidationReason::VIEWPORT,
            ..layout_style_invalidation(
                nana_ui_core::LayoutStyleChange::SIZING_BOTH
                    .union(nana_ui_core::LayoutStyleChange::POSITION),
                false,
                None,
            )
        };
        let moved = nana_ui_core::ViewportAxes {
            width: inline,
            height: block,
        };
        roots
            .into_iter()
            .map(|id| crate::LayoutFrontierSeed::new(id, root))
            .chain(
                self.viewport_basis_ids_for(document)
                    .filter(|id| {
                        self.nodes.get(*id).is_some_and(|record| {
                            record.resolved_layout.viewport_axes().intersects(moved)
                        })
                    })
                    .map(|id| crate::LayoutFrontierSeed::new(id, basis)),
            )
            .collect()
    }

    /// Forgets a document's viewport once it holds no roots.
    pub(crate) fn clear_document_viewport(&mut self, document: DocumentId) {
        self.invalidation.document_viewports.remove(&document);
    }

    /// Viewport of the document owning `id`, if it has been laid out.
    pub(crate) fn document_viewport_of(&self, id: StableNodeId) -> Option<crate::LayoutViewport> {
        if self.invalidation.geometry_frame.get().is_some() {
            self.invalidation.geometry_read_page.set(true);
        }
        let document = self.node(id)?.document;
        self.invalidation.document_viewports.get(&document).copied()
    }

    pub fn viewport_basis_ids_for(
        &self,
        document: DocumentId,
    ) -> impl Iterator<Item = StableNodeId> + '_ {
        self.invalidation
            .viewport_basis
            .get(&document)
            .into_iter()
            .flat_map(|ids| ids.iter().copied())
    }

    /// The document `id` belongs to; `None` for a node that does not exist.
    pub fn document_of(&self, id: StableNodeId) -> Option<DocumentId> {
        self.nodes.get(id).map(|node| node.document)
    }

    pub(crate) fn is_focus_visible_node(&self, id: StableNodeId) -> bool {
        self.document_of(id)
            .and_then(|document| self.focus_visible(document))
            == Some(id)
    }

    pub(crate) fn is_focused_node(&self, id: StableNodeId) -> bool {
        self.document_of(id)
            .and_then(|document| self.focused(document))
            == Some(id)
    }

    pub(crate) fn widget_interaction_state(&self, id: StableNodeId) -> (bool, bool, bool) {
        let hovered = self
            .input
            .pointer_hover
            .values()
            .any(|target| *target == id);
        let pressed = self
            .input
            .pointer_press
            .values()
            .any(|target| *target == id);
        let disabled = self.nodes.accessibility(id).disabled;
        (hovered, pressed, disabled)
    }

    fn note_z_index_presence(&mut self, was_present: bool, now_present: bool) {
        bump_presence(
            &mut self.invalidation.z_index_nodes,
            was_present,
            now_present,
        );
    }

    fn note_presence_counts(
        &mut self,
        was_confirm: bool,
        now_confirm: bool,
        was_clip: bool,
        now_clip: bool,
    ) {
        bump_presence(
            &mut self.invalidation.confirm_modals,
            was_confirm,
            now_confirm,
        );
        bump_presence(&mut self.invalidation.clip_visuals, was_clip, now_clip);
    }

    fn forget_visual_presence(&mut self, id: StableNodeId) {
        self.apply_presence_flags(Some(id), PresenceFlags::NONE);
    }

    pub fn is_descendant_or_self(&self, id: StableNodeId, ancestor: StableNodeId) -> bool {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.parent_id(candidate);
        }
        false
    }

    pub(crate) fn confirm_action_effect(&self, id: StableNodeId) -> Option<(bool, bool, bool)> {
        if self.invalidation.confirm_modals == 0 {
            return None;
        }
        let mut current = self.parent_id(id);
        while let Some(ancestor) = current {
            if let Some(modal) = self.widget_visual_as::<crate::ModalFrameVisual>(ancestor)
                && matches!(modal.kind, crate::ModalSurfaceKind::Confirm(_))
            {
                let slots = &modal.slots;
                let close = slots
                    .close_action
                    .is_some_and(|root| self.is_descendant_or_self(id, root));
                let action = slots
                    .actions
                    .iter()
                    .copied()
                    .find(|root| self.is_descendant_or_self(id, *root));
                if close || action.is_some() {
                    return Some((
                        modal.busy,
                        modal.danger,
                        action == slots.actions.last().copied(),
                    ));
                }
            }
            current = self.parent_id(ancestor);
        }
        None
    }

    pub(crate) fn has_active_scrollbar_chrome(&self, id: StableNodeId) -> bool {
        self.widget_visual_as::<crate::ScrollbarVisual>(id)
            .is_some_and(crate::ScrollbarVisual::is_shown)
            && self.scrollbar_bars(id).is_some()
    }

    fn scrollbar_subtree_visible(&self, id: StableNodeId) -> bool {
        let mut current = Some(id);
        while let Some(node_id) = current {
            let Some(node) = self.nodes.get(node_id) else {
                return false;
            };
            if !node.resolved.0.box_visible
                || !node.resolved.0.visible
                || node.resolved.0.opacity <= 0.0
                || self.motion_blocks_input(node_id)
            {
                return false;
            }
            current = self.parent_id(node_id);
        }
        true
    }

    /// Scroll containers of `document` that currently appear in the hit index
    /// and have shown scrollbar chrome, ordered topmost-first.
    pub(crate) fn active_scrollbar_containers(&self, document: DocumentId) -> Vec<StableNodeId> {
        let Some(index) = self.hit_index.hit_test_index.get(&document) else {
            return Vec::new();
        };
        let mut containers: Vec<_> = self
            .scroll
            .containers
            .iter()
            .copied()
            .filter_map(|id| {
                if self.document_of(id) != Some(document)
                    || !self.is_mounted(id)
                    || !self.has_active_scrollbar_chrome(id)
                {
                    return None;
                }
                let node = self.nodes.get(id)?;
                if !node.interaction.pointer_events
                    || !self.scrollbar_subtree_visible(id)
                    || self.is_inert(id)
                {
                    return None;
                }
                let indexed = index.entries.get(&id)?;
                Some((id, (indexed.entry.z_index, node.depth, indexed.entry.order)))
            })
            .collect();
        containers.sort_by(|(_, a), (_, b)| b.cmp(a));
        containers.into_iter().map(|(id, _)| id).collect()
    }

    /// Whether `target` can be what a pointer of `document` is over: the
    /// hover and capture owner routing writes each sample.
    pub(crate) fn accepts_pointer(&self, document: DocumentId, target: StableNodeId) -> bool {
        self.validate_pointer_target(document, target).is_ok()
    }

    fn validate_pointer_target(
        &self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<(), UiWorldError> {
        let node = self
            .nodes
            .get(target)
            .ok_or(UiWorldError::MissingNode(target))?;
        if node.document != document {
            return Err(UiWorldError::PointerDocument { document, target });
        }
        if !self.is_mounted(target) || !node.interaction.pointer_events || self.is_inert(target) {
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        if !self.used_pointer_events(target).hittable()
            || !node.resolved.0.pointer_events.hittable()
        {
            if self.has_active_scrollbar_chrome(target) && self.scrollbar_subtree_visible(target) {
                return Ok(());
            }
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        Ok(())
    }

    fn used_pointer_events(&self, id: StableNodeId) -> PointerEventsSpec {
        if self.record(id).depth > page_origins::DIRECT_DEPTH
            && let Ok(mut memo) = self.hit_index.pointer_events_memo.try_borrow_mut()
            && let Some(memo) = memo.as_mut()
        {
            return self.used_pointer_events_remembered(id, memo);
        }
        self.walked_pointer_events(id)
    }

    /// [`Self::used_pointer_events`], every ancestor walked.
    fn walked_pointer_events(&self, id: StableNodeId) -> PointerEventsSpec {
        let mut current = Some(id);
        while let Some(node) = current {
            #[cfg(test)]
            page_origins::steps::step(page_origins::steps::Walk::PointerEvents);
            if let Some(specified) = self.record(node).style.layout.pointer_events {
                return specified;
            }
            current = self.parent_id(node);
        }
        PointerEventsSpec::Auto
    }

    /// [`Self::used_pointer_events`] through `memo`: the nodes a walk
    /// passes take the answer it finds, so the nodes of a deep chain each
    /// walk a step.
    fn used_pointer_events_remembered(
        &self,
        id: StableNodeId,
        memo: &mut NodeMap<PointerEventsSpec>,
    ) -> PointerEventsSpec {
        let mut passed = Vec::new();
        let mut current = Some(id);
        let used = loop {
            let Some(node) = current else {
                break PointerEventsSpec::Auto;
            };
            #[cfg(test)]
            page_origins::steps::step(page_origins::steps::Walk::PointerEvents);
            if let Some(&known) = memo.get(&node) {
                break known;
            }
            if let Some(specified) = self.record(node).style.layout.pointer_events {
                break specified;
            }
            passed.push(node);
            current = self.parent_id(node);
        };
        for node in passed {
            memo.insert(node, used);
        }
        #[cfg(test)]
        let walked = page_origins::steps::uncounted(|| self.walked_pointer_events(id));
        #[cfg(all(not(test), feature = "layout-verify"))]
        let walked = self.walked_pointer_events(id);
        #[cfg(any(test, feature = "layout-verify"))]
        assert_eq!(used, walked, "{id:?}: remembered pointer events");
        used
    }

    fn parent_used_pointer_events(&self, id: StableNodeId) -> PointerEventsSpec {
        self.parent_id(id)
            .map(|parent| self.used_pointer_events(parent))
            .unwrap_or(PointerEventsSpec::Auto)
    }

    /// Whether `id` or one of its ancestors is
    /// [`nana_ui_core::LayoutStyle::inert`], read from the styles as they
    /// are now rather than the last resolved ones, so a commit that made a
    /// subtree inert keeps the pointer and focus out of it before the next
    /// frame resolves it.
    pub fn is_inert(&self, id: StableNodeId) -> bool {
        if self.invalidation.inert_nodes == 0 {
            return false;
        }
        let mut current = Some(id);
        while let Some(node) = current {
            let Some(record) = self.nodes.get(node) else {
                return false;
            };
            if record.style.layout.inert {
                return true;
            }
            current = record.hierarchy.parent;
        }
        false
    }

    /// A commit made `root`'s subtree inert. What it held of the pointer
    /// and the focus is let go now, not at the next frame: the focus and
    /// its composition, pointer captures, hovers and presses, and the hit
    /// entries the index keeps until it is built again. Animations keep
    /// running: an inert subtree still paints.
    fn release_inert_subtree(&mut self, root: StableNodeId) {
        let subtree = self.subtree_ids(root);
        let document = self.record(root).document;
        if let Some(focused) = self.input.focused.get(&document).copied()
            && subtree.contains(&focused)
        {
            self.input.focused.remove(&document);
            self.release_focus(focused);
            self.input.focus_counters.focus_changes += 1;
        }
        let inside = subtree.iter().copied().collect::<HashSet<_>>();
        let released = self
            .input
            .pointer_captures
            .iter()
            .filter_map(|(&(document, pointer_id), &target)| {
                inside
                    .contains(&target)
                    .then_some((document, pointer_id, target))
            })
            .collect::<Vec<_>>();
        for (document, pointer_id, target) in released {
            self.input.pointer_captures.remove(&(document, pointer_id));
            self.input
                .pending_pointer_capture_changes
                .push(PointerCaptureChange {
                    pointer_id,
                    target,
                    captured: false,
                });
        }
        let mut cleared = Vec::new();
        for (_, target) in self
            .input
            .pointer_hover
            .iter()
            .chain(self.input.pointer_press.iter())
        {
            if inside.contains(target) && !cleared.contains(target) {
                cleared.push(*target);
            }
        }
        self.input
            .pointer_hover
            .retain(|_, target| !inside.contains(target));
        self.input
            .pointer_press
            .retain(|_, target| !inside.contains(target));
        if !cleared.is_empty() {
            self.invalidation.generation = self.invalidation.generation.wrapping_add(1);
            for id in cleared {
                self.mark_interaction_style(id);
            }
        }
        self.withdraw_inert_hits(root);
    }

    fn clear_hover_for_pointer_events_none(&mut self, root: StableNodeId) {
        let mut stack = vec![root];
        let mut cleared = Vec::new();
        while let Some(id) = stack.pop() {
            if !self.used_pointer_events(id).hittable() {
                let had_hover = self
                    .input
                    .pointer_hover
                    .values()
                    .any(|target| target == &id);
                let had_press = self
                    .input
                    .pointer_press
                    .values()
                    .any(|target| target == &id);
                self.input.pointer_hover.retain(|_, target| target != &id);
                self.input.pointer_press.retain(|_, target| target != &id);
                if had_hover || had_press {
                    cleared.push(id);
                }
            }
            stack.extend(self.record(id).hierarchy.children.iter().copied());
        }
        if !cleared.is_empty() {
            self.invalidation.generation = self.invalidation.generation.wrapping_add(1);
            for id in cleared {
                self.mark_interaction_style(id);
            }
        }
    }

    /// What a node that just lost focus needs: its composition cancelled and
    /// its focused state and style redrawn.
    pub(super) fn release_focus(&mut self, old: StableNodeId) {
        self.remove_ime(old);
        self.mark(old, DirtyMask::STATE);
        if !self.record(old).style.interaction.focused.is_empty() {
            if self.record(old).hierarchy.children.is_empty() {
                self.mark(old, DirtyMask::STYLE | DirtyMask::RENDER);
            } else {
                self.mark_subtree(old, DirtyMask::STYLE | DirtyMask::RENDER);
            }
        }
        self.mark_focus_changed(old);
    }

    /// Focus left the editor, or it is going away: an unfinished composition
    /// is cancelled and an attached IME detached. The editor presentation was
    /// built with the preedit spliced in, so it is re-derived rather than left
    /// drawing text that is no longer there.
    fn remove_ime(&mut self, id: StableNodeId) {
        let Some(editor) = self.nodes.editor_mut(id) else {
            return;
        };
        if editor.ime().is_none() {
            return;
        }
        editor.session.blur();
        editor.empty_preedit = None;
        let _ = editor.session.take_work();
        self.text.pending_edit_work.composition_updates += 1;
        self.nodes
            .invalidate_text(id, crate::text_node::TextDirty::EDIT_STATE);
        self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
    }

    fn clear_overlay_references(&mut self, removed: StableNodeId) {
        self.clear_overlay_references_for(&[removed]);
    }

    /// Drop references through the reverse index; unrelated hosts are untouched.
    fn clear_overlay_references_for(&mut self, removed: &[StableNodeId]) {
        if self.overlays_a11y.overlay_host_nodes.is_empty() || removed.is_empty() {
            return;
        }
        let removed = removed.iter().copied().collect::<HashSet<_>>();
        let hosts = removed
            .iter()
            .filter_map(|id| self.overlays_a11y.overlay_dependents.get(id))
            .flatten()
            .copied()
            .collect::<HashSet<_>>();
        // The overlays closing here: usually the removed root alone.
        let closing = hosts
            .iter()
            .filter(|host| !removed.contains(host))
            .filter_map(|host| self.nodes.overlay_host(*host)?.active)
            .filter(|active| removed.contains(active))
            .collect::<Vec<_>>();
        // Overlays opened from inside it, hosted elsewhere (a dropdown's
        // listbox hosted outside the popover it opened from), and those
        // opened from inside them in turn: focus in any of them belongs to
        // the closing overlay too. The openers are still in the tree here: a
        // despawn takes the root first, and parking keeps the nodes.
        let mut opened_from_closing = Vec::new();
        if !closing.is_empty() {
            let open = self
                .overlays_a11y
                .overlay_host_nodes
                .iter()
                .filter(|host| !removed.contains(host))
                .filter_map(|host| self.nodes.overlay_host(*host))
                .filter_map(|state| {
                    let active = state.active.filter(|active| !removed.contains(active))?;
                    Some((active, state.restore_focus?))
                })
                .collect::<Vec<_>>();
            let mut frontier = closing;
            while let Some(parent) = frontier.pop() {
                for &(active, opener) in &open {
                    if opened_from_closing.contains(&active) {
                        continue;
                    }
                    // Inside the overlay it was opened from; one elsewhere in
                    // the removal (a parked subtree) is unrelated.
                    if self.contains(opener) && self.is_descendant_or_self(opener, parent) {
                        opened_from_closing.push(active);
                        frontier.push(active);
                    }
                }
            }
        }
        // A host inside `removed` goes with it and is skipped.
        let updates = hosts
            .into_iter()
            .filter_map(|host| {
                (!removed.contains(&host))
                    .then(|| self.nodes.overlay_host(host).copied())
                    .flatten()
                    .and_then(|mut state| {
                        let previous = state;
                        let closed = state.active.filter(|active| removed.contains(active));
                        let restore_focus = closed.zip(state.restore_focus);
                        if closed.is_some() {
                            state.active = None;
                            state.restore_focus = None;
                        }
                        if state
                            .restore_focus
                            .is_some_and(|target| removed.contains(&target))
                        {
                            state.restore_focus = None;
                        }
                        (state != previous).then_some((host, state, restore_focus))
                    })
            })
            .collect::<Vec<_>>();
        for (host, state, restore_focus) in updates {
            self.write_overlay_host(host, Some(state));
            self.mark(host, DirtyMask::ACCESSIBILITY);
            let document = self.record(host).document;
            let Some((overlay, restore_focus)) = restore_focus else {
                continue;
            };
            // Focus goes back unless the user moved it to another node, which
            // keeps it, with whatever composition it has going. It is still
            // on the overlay or inside it when a despawn removes the root
            // first, or in an overlay opened from it. An active modal above
            // still keeps it where it is (checked below). A document with no focus gets the opener back too: the
            // removal may have dropped it, or the framework may have (a
            // disabled or hidden editor), and a keyboard or screen-reader user
            // left with nothing focused is worse off than one whose cleared
            // focus comes back to the control that opened the overlay.
            let focused = self.input.focused.get(&document).copied();
            let focus_left = focused.is_none_or(|focused| {
                removed.contains(&focused)
                    || !self.contains(focused)
                    || self.is_descendant_or_self(focused, overlay)
                    || opened_from_closing
                        .iter()
                        .any(|nested| self.is_descendant_or_self(focused, *nested))
            });
            if focus_left
                && self.contains(restore_focus)
                && self.is_mounted(restore_focus)
                && self.record(restore_focus).document == document
                && self.focus_policy(restore_focus).allows_programmatic()
                && self.record(restore_focus).resolved.0.visible
                && !self.is_inert(restore_focus)
                && self.active_modal_allows_focus_now(document, restore_focus)
            {
                // The node focus leaves, if it is still here, loses it as a
                // RequestFocus would take it: composition ended, style redrawn.
                if let Some(old) =
                    focused.filter(|old| *old != restore_focus && self.contains(*old))
                {
                    self.release_focus(old);
                }
                self.input.focused.insert(document, restore_focus);
                self.input.focus_counters.focus_changes += 1;
                self.mark_focus_changed(restore_focus);
            }
        }
    }

    fn active_modal_allows_focus_now(&self, document: DocumentId, target: StableNodeId) -> bool {
        let order = self.document_order(document);
        let top = order
            .iter()
            .enumerate()
            .filter_map(|(host_order, host)| {
                let state = self.overlay_host(*host)?;
                let active = state.active?;
                let node = self.node(active)?;
                if node.parent != Some(*host)
                    || self.surface_closed(active)
                    || !self.is_overlay_reachable(active)
                    || !self
                        .accessibility(active)
                        .is_some_and(|accessibility| accessibility.modal)
                {
                    return None;
                }
                let z = self
                    .node_style(active)
                    .and_then(|style| style.layout.z_index)
                    .unwrap_or_default();
                let active_order = order
                    .iter()
                    .position(|candidate| *candidate == active)
                    .unwrap_or(host_order);
                Some((z, active_order, active))
            })
            .max_by_key(|(z, active_order, _)| (*z, *active_order));
        top.is_none_or(|(_, _, active)| self.has_ancestor_now(target, active))
    }

    fn has_ancestor_now(&self, mut id: StableNodeId, candidate: StableNodeId) -> bool {
        let mut visited = HashSet::new();
        loop {
            if id == candidate {
                return true;
            }
            if !visited.insert(id) {
                return false;
            }
            let Some(parent) = self.parent_id(id) else {
                return false;
            };
            id = parent;
        }
    }

    fn overlay_branch_active(&self, id: StableNodeId) -> bool {
        if self.overlays_a11y.overlay_host_nodes.is_empty() {
            return true;
        }
        let Some(parent) = self.record(id).hierarchy.parent else {
            return true;
        };
        self.overlay_host(parent)
            .is_none_or(|state| state.active == Some(id))
    }

    /// A closed menu keeps its items in the tree but out of the frame. They
    /// would otherwise stretch the in-flow trigger they hang under.
    fn menu_branch_open(&self, id: StableNodeId) -> bool {
        if self.invalidation.menu_surfaces == 0 {
            return true;
        }
        let Some(parent) = self.record(id).hierarchy.parent else {
            return true;
        };
        if !self.nodes.contains(parent) {
            return true;
        };
        !self
            .widget_visual_ref(parent)
            .is_some_and(|w| w.closed_menu_hides(id))
    }

    /// Whether a live retained root currently belongs to this document.
    /// Host service requests use this as the document lifetime authority.
    pub fn has_document(&self, document: DocumentId) -> bool {
        self.tree
            .live_document_roots
            .get(&document)
            .is_some_and(|roots| !roots.is_empty())
    }

    pub(crate) fn has_document_roots(&self, document: DocumentId) -> bool {
        self.has_document(document)
    }

    pub(crate) fn document_roots(&self, document: DocumentId) -> Vec<StableNodeId> {
        let mut roots = self
            .tree
            .live_document_roots
            .get(&document)
            .map(|set| set.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        roots.sort_unstable();
        roots
    }

    fn refresh_root_membership(&mut self, id: StableNodeId) {
        if !self.nodes.contains(id) {
            return;
        }
        let node = self.record(id);
        let document = node.document;
        let parent = node.hierarchy.parent;
        let live_root = parent.is_none() && self.presence_live(id);
        let moved = if live_root {
            self.tree
                .live_document_roots
                .entry(document)
                .or_default()
                .insert(id)
        } else {
            self.remove_document_root(document, id)
        };
        // A window's right-to-left locale sits on its roots.
        if moved && self.locale_sets_direction() {
            self.restyle(id, None);
        }
    }

    /// Returns whether `id` was one of `document`'s roots.
    fn remove_document_root(&mut self, document: DocumentId, id: StableNodeId) -> bool {
        let Some(roots) = self.tree.live_document_roots.get_mut(&document) else {
            return false;
        };
        let removed = roots.remove(&id);
        if roots.is_empty() {
            self.tree.live_document_roots.remove(&document);
        }
        removed
    }

    pub fn document_order(&self, document: DocumentId) -> Vec<StableNodeId> {
        let roots = self.document_roots(document);
        let mut order = Vec::new();
        let mut stack = roots.into_iter().rev().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(self.record(id).hierarchy.children.iter().rev().copied());
        }
        self.record_id_list_alloc(order.len());
        order
    }

    /// The parent constraint that runs along `parent`'s line, the axis its
    /// children share out: the inline one for a row, the block one for a
    /// column. A container lays its children on a line along its direction
    /// whatever its `display`, unless it is a grid or an inline formatting
    /// context. Horizontal writing only: elsewhere the physical axes the
    /// constraints name do not follow the direction this simply.
    fn line_constraint(
        &self,
        parent: StableNodeId,
        record: &NodeRecord,
    ) -> Option<LayoutDependencyFootprint> {
        let style = record.resolved_layout.as_ref();
        if style
            .display
            .is_some_and(|display| display.is_grid_container() || display.is_inline_level())
            || self.layout_writing(parent).is_vertical()
        {
            return None;
        }
        Some(
            match style
                .direction
                .unwrap_or(nana_ui_core::FlexDirection::Column)
            {
                nana_ui_core::FlexDirection::Row => {
                    LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT
                }
                nana_ui_core::FlexDirection::Column => {
                    LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT
                }
            },
        )
    }

    /// Downward footprint of one child under a parent-constraint seed.
    ///
    /// `edge` is what this child consumes, so the frontier measures it.
    /// `pass` is the subset of `changed` that still reaches its descendants.
    /// A definite content box that cannot pass the constraint returns an
    /// empty `pass`. An empty `edge` means the child is not part of this
    /// measure descent.
    fn child_constraint_descent(
        child: &nana_ui_core::LayoutStyle,
        parent: &nana_ui_core::LayoutStyle,
        changed: LayoutDependencyFootprint,
        has_wrapping_text: bool,
        is_icon: bool,
        reads_block_extent: bool,
    ) -> (LayoutDependencyFootprint, LayoutDependencyFootprint) {
        let inline_flag = LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT;
        let block_flag = LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT;
        let writing_flag = LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT;
        let inline_changed = changed.intersects(inline_flag);
        let block_changed = changed.intersects(block_flag);
        let writing_changed = changed.intersects(writing_flag);
        if !inline_changed && !block_changed && !writing_changed {
            return (
                LayoutDependencyFootprint::NONE,
                LayoutDependencyFootprint::NONE,
            );
        }

        let tracks = crate::layout_engine::spec_tracks_containing_block;
        let width_auto = matches!(child.width, None | Some(LengthSpec::Auto));
        let inline_stretch = !is_icon && width_auto && child.stretch_fit_inline();
        let padding_tracks = tracks(child.padding)
            || tracks(child.padding_top)
            || tracks(child.padding_right)
            || tracks(child.padding_bottom)
            || tracks(child.padding_left);
        let inline_definite = child.width.is_some()
            && !crate::layout_engine::depends_on_used_basis(child.width)
            && !tracks(child.min_width)
            && !tracks(child.max_width)
            && !inline_stretch;
        let block_definite = child.height.is_some()
            && !crate::layout_engine::depends_on_used_basis(child.height)
            && !tracks(child.min_height)
            && !tracks(child.max_height);
        let content_inline_definite = inline_definite && !padding_tracks;
        let content_block_definite = block_definite && !padding_tracks;

        let parent_flex = parent
            .display
            .is_some_and(nana_ui_core::DisplaySpec::is_flex_container);
        let parent_grid = parent
            .display
            .is_some_and(nana_ui_core::DisplaySpec::is_grid_container);
        let row = parent.direction == Some(nana_ui_core::FlexDirection::Row);
        let mut inline_consumes = inline_changed
            && (tracks(child.width)
                || tracks(child.min_width)
                || tracks(child.max_width)
                || inline_stretch
                || padding_tracks);
        let mut block_consumes = block_changed
            && (tracks(child.height)
                || tracks(child.min_height)
                || tracks(child.max_height)
                || padding_tracks);
        if parent_flex && tracks(child.flex_basis) {
            if row && inline_changed {
                inline_consumes = true;
            } else if !row && block_changed {
                block_consumes = true;
            }
        }

        // Whether the child may read the changed constraint on each axis
        // without a style that says so: per axis, so a width-only change
        // never measures a child for its height.
        let (mut uncertain_inline, mut uncertain_block) = if parent_grid {
            (
                inline_changed && !inline_definite,
                block_changed && !block_definite,
            )
        } else if parent_flex {
            if row {
                (false, block_changed && !block_definite)
            } else {
                (inline_changed && !inline_definite, false)
            }
        } else {
            (false, false)
        };
        if has_wrapping_text && inline_changed && !inline_definite {
            uncertain_inline = true;
        }
        // Something under a child whose height follows its content reads the
        // block extent the child is offered: a percentage or fill height, a
        // column that wraps, a grid (`world/block_reads.rs`). Layout resolves
        // it against that extent, so the child passes it down.
        if reads_block_extent && block_changed && !block_definite {
            uncertain_block = true;
        }
        let aspect = child
            .aspect_ratio
            .is_some_and(|ratio| ratio.is_finite() && ratio > 0.0);
        // An aspect ratio carries a size from one axis to the other.
        if aspect && ((inline_changed && !inline_definite) || (block_changed && !block_definite)) {
            uncertain_inline = true;
            uncertain_block = true;
        }
        if writing_changed && child.has_logical_box_edges() {
            uncertain_inline = true;
            uncertain_block = true;
        }
        let uncertain_inline = uncertain_inline && inline_changed;
        let uncertain_block = uncertain_block && block_changed;

        let mut edge = LayoutDependencyFootprint::NONE;
        let mut pass = LayoutDependencyFootprint::NONE;
        if inline_consumes || uncertain_inline {
            edge = edge.union(inline_flag);
        }
        if block_consumes || uncertain_block {
            edge = edge.union(block_flag);
        }
        if writing_changed {
            edge = edge.union(writing_flag);
            pass = pass.union(writing_flag);
        }
        if (inline_consumes || uncertain_inline) && !content_inline_definite {
            pass = pass.union(inline_flag);
        }
        if (block_consumes || uncertain_block) && !content_block_definite {
            pass = pass.union(block_flag);
        }
        (edge, pass)
    }

    /// Build only the dependency closure needed by typed seeds. Parent links
    /// are walked upward for exported metrics; descendants and formatting
    /// contexts are expanded only when the seed footprint consumes those
    /// dependencies. This keeps frontier scratch independent of unrelated
    /// document branches.
    pub(crate) fn layout_dependency_graph_for_seeds(
        &self,
        document: DocumentId,
        seeds: &[crate::LayoutFrontierSeed],
    ) -> crate::LayoutDependencyGraph {
        use std::collections::VecDeque;

        let mut graph = crate::LayoutDependencyGraph::default();
        let upward = LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS
            .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE)
            .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK)
            .union(LayoutDependencyFootprint::EXPORTS_BASELINE);
        let downward = LayoutDependencyFootprint::parent_constraints()
            .union(LayoutDependencyFootprint::DEPENDS_ON_CONTAINING_BLOCK)
            .union(LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT);
        let lateral = LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX
            .union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING);
        let mut up_seen = NodeSet::default();
        let mut constraint_seen = NodeSet::default();
        let mut context_seen = NodeSet::default();
        let mut context_parents: NodeMap<LayoutDependencyFootprint> = NodeMap::default();
        let mut pending = VecDeque::new();

        for seed in seeds {
            if self.document_of(seed.node) != Some(document) {
                continue;
            }
            let axes = seed.invalidation.affected_axes;
            // An unknown cause or a subtree that just arrived: lay out all
            // of it. A parent whose child list changed is not one; it carries
            // a narrower footprint of its own.
            let force_all = axes == LayoutDependencyFootprint::ALL;
            if !force_all && axes.intersects(LayoutDependencyFootprint::parent_constraints()) {
                graph.note_constraint_seed();
            }
            pending.push_back((
                seed.node,
                axes,
                force_all || axes.intersects(downward),
                force_all || axes.intersects(lateral),
                force_all,
            ));
        }

        while let Some((node, axes, expand_down, expand_context, force_all)) = pending.pop_front() {
            if self.document_of(node) != Some(document) {
                continue;
            }
            if self.layout_isolated(node) {
                graph.isolate(node);
            }
            // Every typed seed needs its exported metric path. An isolated
            // node owns its subtree metrics, so its parent edge is a boundary.
            if up_seen.insert(node)
                && !self.layout_isolated(node)
                && let Some(parent) = self.parent_id(node)
                && self.document_of(parent) == Some(document)
            {
                let Some(parent_record) = self.nodes.get(parent) else {
                    continue;
                };
                let parent_display = parent_record.resolved_layout.display;
                // A box whose lines an atom's baseline reads is not fixed
                // to the line it sits on, whatever its size: what it holds
                // moves that baseline.
                let has_definite_size = self.fixed_border(parent, parent_record)
                    && !self.atom_baseline_reads_content(parent);
                // A fixed inline-block is an atomic box. Inner metrics
                // stop at its border; the outer inline formatting
                // context does not reflow.
                let atomic_border = has_definite_size
                    && matches!(
                        parent_display,
                        Some(
                            nana_ui_core::DisplaySpec::InlineFlex
                                | nana_ui_core::DisplaySpec::InlineGrid
                        )
                    );
                let is_formatting_context = !atomic_border
                    && parent_display.is_some_and(|display| {
                        display.is_flex_container()
                            || display.is_grid_container()
                            || display.is_inline_level()
                    });
                // Fixed ordinary boxes stop intrinsic export and keep a
                // lateral edge. Formatting contexts still consume metrics.
                // A forced subtree reaches its ancestors as any metric change
                // does: they re-measure along the path, and only the subtree
                // itself is laid out in full.
                let upward_for_parent = if has_definite_size && !is_formatting_context {
                    lateral
                } else if is_formatting_context {
                    upward.union(lateral)
                } else {
                    upward
                };
                // A free item sizes nothing of its parent: whatever moved
                // inside it, the parent only places it again.
                let free_item = self.is_free_item(node);
                if !upward_for_parent.is_empty() {
                    let follows_metrics =
                        force_all || (!free_item && axes.intersects(upward_for_parent));
                    let edge_structural = !follows_metrics;
                    let edge_upward = if edge_structural {
                        LayoutDependencyFootprint::NONE
                    } else {
                        upward_for_parent
                    };
                    let edge_downward = if edge_structural {
                        LayoutDependencyFootprint::NONE
                    } else {
                        downward
                    };
                    let parent_is_document =
                        matches!(parent_record.kind.as_ref(), NodeKind::Document);
                    let exports_size = force_all || axes.intersects(upward);
                    // Unchanged fixed border: do not replay the parent.
                    let inner_fixed =
                        edge_structural && !exports_size && self.is_fixed_metric_boundary(node);
                    if !inner_fixed
                        && (!edge_structural || (!parent_is_document && !has_definite_size))
                    {
                        graph.add_parent_dependency_split(parent, node, edge_upward, edge_downward);
                    }
                    if !inner_fixed && !edge_structural && !parent_is_document {
                        pending.push_back((
                            parent,
                            upward_for_parent,
                            axes.intersects(lateral) && parent_record.hierarchy.children.len() <= 1,
                            false,
                            false,
                        ));
                    }
                }
            }

            if expand_down {
                let constraint_axes = if force_all {
                    LayoutDependencyFootprint::ALL
                } else {
                    axes.intersection(
                        LayoutDependencyFootprint::parent_constraints()
                            .union(LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT),
                    )
                };
                // Reached only to be placed again, a node moves as a whole:
                // placing it at a new origin places its subtree, and at the old
                // one its subtree keeps its boxes. Neither needs a descendant in
                // the frontier, where it would only keep the pass from pruning
                // a subtree that did not move -- a splitter drag would place
                // every box of every pane after it.
                if !constraint_axes.is_empty() && constraint_seen.insert(node) {
                    let children = self
                        .nodes
                        .get(node)
                        .map(|record| record.hierarchy.children.clone())
                        .unwrap_or_default();
                    let parent_style = self
                        .nodes
                        .get(node)
                        .map(|record| Arc::clone(&record.resolved_layout));
                    for child in children.iter().copied() {
                        if self.document_of(child) != Some(document) {
                            continue;
                        }
                        if force_all {
                            graph.add_parent_dependency_split(
                                node,
                                child,
                                LayoutDependencyFootprint::ALL,
                                LayoutDependencyFootprint::ALL,
                            );
                            pending.push_back((
                                child,
                                LayoutDependencyFootprint::ALL,
                                true,
                                expand_context,
                                true,
                            ));
                            continue;
                        }
                        let Some(parent_style) = parent_style.as_deref() else {
                            continue;
                        };
                        let is_icon = self
                            .widget_visual_ref(child)
                            .is_some_and(crate::WidgetVisualHandle::is_icon_visual);
                        let Some(child_record) = self.nodes.get(child) else {
                            continue;
                        };
                        let has_wrapping_text = child_record.resolved_layout.text_wraps()
                            && (matches!(child_record.kind.as_ref(), NodeKind::Text)
                                || child_record.has_text);
                        let (edge, pass) = Self::child_constraint_descent(
                            child_record.resolved_layout.as_ref(),
                            parent_style,
                            constraint_axes,
                            has_wrapping_text,
                            is_icon,
                            child_record.block_reads.reads(),
                        );
                        graph.note_constraint_dependent(!edge.is_empty());
                        if edge.is_empty() {
                            continue;
                        }
                        graph.add_parent_dependency_split(
                            node,
                            child,
                            LayoutDependencyFootprint::NONE,
                            edge,
                        );
                        if !pass.is_empty() {
                            pending.push_back((child, pass, true, false, false));
                        }
                    }
                }
            }

            if expand_context && context_seen.insert(node) {
                // Absolute and fixed boxes do not participate in sibling flow.
                // A content change stays on the positioned child; the containing
                // block reaches it through the parent edge, not through siblings.
                if !force_all && self.positioned_out_of_flow(node) {
                    continue;
                }
                let Some(parent) = self.parent_id(node) else {
                    continue;
                };
                let Some(parent_record) = self.nodes.get(parent) else {
                    continue;
                };
                // A node whose size moved along its parent's line moves what
                // the line leaves its flexible siblings: a splitter's first
                // pane widens and the fill pane beside it narrows. Each such
                // sibling descends that axis as a constraint seed would, so
                // only its children that read the axis measure.
                if !force_all
                    && let Some(line) = self.line_constraint(parent, parent_record)
                    && axes.intersects(line)
                {
                    for &sibling in parent_record.hierarchy.children.iter() {
                        if sibling != node
                            && self.document_of(sibling) == Some(document)
                            && !self.positioned_out_of_flow(sibling)
                            && self.nodes.get(sibling).is_some_and(|record| {
                                flexes_along(record.resolved_layout.as_ref(), line)
                            })
                        {
                            graph.note_constraint_seed();
                            graph.add_context_dependency_forward(node, sibling, line);
                            pending.push_back((sibling, line, true, false, false));
                        }
                    }
                }
                let explicit_context =
                    parent_record
                        .resolved_layout
                        .display
                        .is_some_and(|display| {
                            display.is_flex_container()
                                || display.is_grid_container()
                                || display.is_inline_level()
                        });
                if !explicit_context {
                    // A forced subtree exports its metrics: its parent lays
                    // out again and moves the siblings after it. Only an
                    // isolated one, which exports none, links them.
                    if force_all && !self.layout_isolated(node) {
                        continue;
                    }
                    // Block flow: only later siblings move, each because the
                    // one before it did. Link this node to its next in-flow
                    // sibling and expand that one the same way, so the chain
                    // covers the suffix that moves and nothing before it. A
                    // tail node links nothing, and finding the next sibling
                    // reads the retained index instead of scanning the list.
                    let Some(index) = self.child_index(parent, node) else {
                        continue;
                    };
                    let next = parent_record.hierarchy.children[index + 1..]
                        .iter()
                        .copied()
                        .find(|sibling| !self.positioned_out_of_flow(*sibling));
                    if let Some(next) = next
                        && self.document_of(next) == Some(document)
                    {
                        graph.add_context_dependency_forward(node, next, lateral);
                        pending.push_back((next, lateral, true, true, false));
                    }
                    continue;
                }
                let siblings = parent_record.hierarchy.children.as_ref();
                let filtered;
                let flow_siblings: &[StableNodeId] =
                    if siblings.iter().any(|id| self.positioned_out_of_flow(*id)) {
                        filtered = siblings
                            .iter()
                            .copied()
                            .filter(|id| !self.positioned_out_of_flow(*id))
                            .collect::<Vec<_>>();
                        filtered.as_slice()
                    } else {
                        siblings
                    };
                // A flex, grid or inline context couples its items both ways:
                // its domain is built once, as a chain, since a change reaches
                // the other items through the ones between, and linking every
                // pair is quadratic in a wide container for nothing.
                // A later node of the same domain with footprints the chain
                // lacks widens it; that happens at most once per footprint bit.
                let linked = context_parents.get(&parent).copied();
                if linked.is_some_and(|linked| linked.contains(lateral)) {
                    continue;
                }
                let lateral = linked.map_or(lateral, |linked| linked.union(lateral));
                context_parents.insert(parent, lateral);
                for pair in flow_siblings.windows(2) {
                    graph.add_context_dependency(pair[0], pair[1], lateral);
                }
                // Include the formatting-context siblings as seeds in the
                // local graph; their own parent links let a lateral change
                // reach the shared container without scanning descendants.
                for &sibling in flow_siblings {
                    if sibling != node && self.document_of(sibling) == Some(document) {
                        pending.push_back((sibling, lateral, true, false, false));
                    }
                }
            }
        }
        graph
    }

    fn hierarchy_mut(&mut self, id: StableNodeId) -> &mut Hierarchy {
        &mut self.record_mut(id).hierarchy
    }

    /// Number `parent`'s children again after an edit of its list that left
    /// the first `head` and the last `tail` of them alone.
    ///
    /// A child records its place counted from a number its parent keeps
    /// ([`Hierarchy::children_base`]), so the children an edit shifted as one
    /// block need not be written: moving the base renumbers all of them. The
    /// longer of the two sides the edit left alone keeps its numbers and the
    /// rest is written, so a row put first among 100,000 writes one number
    /// where it wrote 100,000, and an edit in the middle at most half.
    ///
    /// While a commit is being applied the numbering waits for its last
    /// mutation: a commit that moves many children of one parent (a list
    /// reversed, rows filled in between others) numbered the rest of the
    /// list after every one of them, and reversing 16,000 rows took 0.6 s.
    /// An index that is behind in the meantime is found out and scanned
    /// for ([`Self::child_index`]), so nothing reads a wrong one.
    pub(crate) fn renumber_children_between(
        &mut self,
        parent: StableNodeId,
        head: usize,
        tail: usize,
    ) {
        if let Some(later) = &mut self.tree.renumber_later {
            later
                .entry(parent)
                .and_modify(|kept| *kept = (kept.0.min(head), kept.1.min(tail)))
                .or_insert((head, tail));
            return;
        }
        self.renumber_children_now(parent, head, tail);
    }

    /// Numbers the children the commit just applied left behind.
    pub(crate) fn renumber_children_left(&mut self) {
        for (parent, (head, tail)) in self.tree.renumber_later.take().into_iter().flatten() {
            self.renumber_children_now(parent, head, tail);
        }
    }

    fn renumber_children_now(&mut self, parent: StableNodeId, head: usize, tail: usize) {
        let Some((children, base)) = self.nodes.get(parent).map(|record| {
            (
                Arc::clone(&record.hierarchy.children),
                record.hierarchy.children_base,
            )
        }) else {
            return;
        };
        let head = head.min(children.len());
        let tail = tail.min(children.len() - head);
        let edited_end = children.len() - tail;
        // What the base has to be for the children after the edit to be
        // numbered as they already are.
        let tail_base = children[edited_end..]
            .first()
            .and_then(|first| self.nodes.get(*first))
            .map(|record| {
                record
                    .hierarchy
                    .index_in_parent
                    .wrapping_sub(child_number(edited_end))
            });
        let (base, written) = match tail_base {
            // The list is as long as it was: both sides keep their numbers.
            Some(tail_base) if head > 0 && tail_base == base => (base, head..edited_end),
            Some(tail_base) if tail > head => (tail_base, 0..edited_end),
            _ if head > 0 => (base, head..children.len()),
            _ => (0, 0..children.len()),
        };
        #[cfg(test)]
        {
            self.tree.child_list_steps += written.len() as u64;
            self.tree.children_numbered += written.len() as u64;
        }
        for index in written {
            if let Some(record) = self.nodes.get_mut(children[index]) {
                record.hierarchy.index_in_parent = base.wrapping_add(child_number(index));
            }
        }
        if let Some(record) = self.nodes.get_mut(parent) {
            record.hierarchy.children_base = base;
        }
    }

    /// Take `child` out of `parent`'s children and renumber the ones after
    /// it, unless a run of removals already did.
    fn remove_child(&mut self, parent: StableNodeId, child: StableNodeId) {
        if !self.tree.unlinked_ahead.is_empty() && self.tree.unlinked_ahead.remove(&child) {
            return;
        }
        let Some(index) = self.child_index(parent, child) else {
            return;
        };
        self.link_block_extent_reads(parent, child, false);
        let hierarchy = self.hierarchy_mut(parent);
        Arc::make_mut(&mut hierarchy.children).remove(index);
        let after = hierarchy.children.len() - index;
        intern_empty_children(&mut hierarchy.children);
        #[cfg(test)]
        {
            self.tree.child_list_steps += after as u64;
        }
        self.renumber_children_between(parent, index, after);
    }

    /// `child`'s index among `parent`'s children, without scanning them
    /// while the retained index is current.
    pub(crate) fn child_index(&self, parent: StableNodeId, child: StableNodeId) -> Option<usize> {
        let children = &self.nodes.get(parent)?.hierarchy.children;
        self.recorded_child_index(parent, child)
            .or_else(|| children.iter().position(|id| *id == child))
    }

    /// `child`'s index among `parent`'s children by the number it records,
    /// if that number is current.
    pub(crate) fn recorded_child_index(
        &self,
        parent: StableNodeId,
        child: StableNodeId,
    ) -> Option<usize> {
        let hierarchy = &self.nodes.get(parent)?.hierarchy;
        let number = self.nodes.get(child)?.hierarchy.index_in_parent;
        let index = number.wrapping_sub(hierarchy.children_base) as usize;
        (hierarchy.children.get(index) == Some(&child)).then_some(index)
    }

    fn mark(&mut self, id: StableNodeId, bits: u16) -> bool {
        self.mark_with_origin(id, bits, MarkOrigin::General)
    }

    fn mark_with_origin(&mut self, id: StableNodeId, bits: u16, origin: MarkOrigin) -> bool {
        if bits & DirtyMask::INPUT != 0 {
            self.hit_index.non_scroll_hit_dirty.insert(id);
        }
        // Internal callers that only have a dirty bit still publish an
        // explicit runtime-wide invalidation, merged into any narrower entry
        // a typed mutation authority already installed.
        if bits & DirtyMask::LAYOUT != 0 {
            self.queue_layout_invalidation(
                id,
                LayoutInvalidation::new(
                    LayoutInvalidationSource::Runtime,
                    InvalidationReason::UNKNOWN,
                    InvalidationKind::ALL,
                    LayoutStyleChange::ALL,
                    LayoutDependencyFootprint::ALL,
                ),
            );
        }
        self.mark_keeping_render_reason(id, bits, origin, true)
    }

    /// [`Self::mark`] for a box layout only moved: its render mark does not
    /// say the rest of its extracted record changed. What does reuse that
    /// record is still decided by `render_moved_epoch`, which only a result
    /// placed again from its own geometry sets.
    fn mark_moved(&mut self, id: StableNodeId, bits: u16) -> bool {
        debug_assert_eq!(bits & DirtyMask::LAYOUT, 0, "a move is not a layout cause");
        if bits & DirtyMask::INPUT != 0 {
            self.hit_index.non_scroll_hit_dirty.insert(id);
        }
        self.mark_keeping_render_reason(id, bits, MarkOrigin::LayoutMove, false)
    }

    fn mark_keeping_render_reason(
        &mut self,
        id: StableNodeId,
        bits: u16,
        origin: MarkOrigin,
        set_render_full: bool,
    ) -> bool {
        if set_render_full && (bits & DirtyMask::RENDER != 0) {
            self.record_mut(id).render_full_epoch = self.invalidation.render_epoch.get();
        }
        if bits & DirtyMask::LAYOUT != 0 {
            self.note_layout_source_change();
        }
        let bits = if bits & DirtyMask::INPUT == 0 {
            bits
        } else {
            match origin {
                MarkOrigin::LayoutMove => bits,
                MarkOrigin::Scroll => bits | DirtyMask::INPUT_SCROLLED,
                MarkOrigin::General => bits | DirtyMask::INPUT_ELSEWHERE,
            }
        };
        let changed = self.record_mut(id).dirty.insert(bits);
        if changed {
            self.invalidation.dirty_entities.insert(id);
            self.invalidation.pending_work_revision =
                self.invalidation.pending_work_revision.saturating_add(1);
        }
        changed
    }

    fn mark_subtree(&mut self, root: StableNodeId, bits: u16) {
        // Layout invalidation is a typed frontier seed. Keep the ordinary
        // dirty bit on the root only: the frontier expands its dependency
        // closure to descendants, while marking every descendant here would
        // synthesize broad Runtime/UNKNOWN seeds and defeat retained plans.
        let subtree_bits = bits & !DirtyMask::LAYOUT;
        let root_layout = bits & DirtyMask::LAYOUT;
        if self.record(root).hierarchy.children.is_empty() {
            // A leaf, as most nodes a build mounts are: no walk.
            let _ = self.mark(root, subtree_bits | root_layout);
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = Arc::clone(&self.record(id).hierarchy.children);
            stack.extend(children.iter().rev().copied());
            let _ = self.mark(id, subtree_bits | if id == root { root_layout } else { 0 });
        }
    }

    fn subtree_ids(&self, root: StableNodeId) -> Vec<StableNodeId> {
        // Most subtrees a build parks or mounts are one node: no walk.
        let children = &self.record(root).hierarchy.children;
        if children.is_empty() {
            return vec![root];
        }
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            stack.extend(self.record(id).hierarchy.children.iter().rev().copied());
            ids.push(id);
        }
        ids
    }

    fn set_subtree_mount_state(&mut self, root: StableNodeId, state: MountState) {
        if self.record(root).hierarchy.children.is_empty() {
            self.record_mut(root).mount = state;
        } else {
            for id in self.subtree_ids(root) {
                self.record_mut(id).mount = state;
            }
        }
        if state == MountState::Mounted {
            self.mark_subtree(root, DirtyMask::ALL);
        }
    }

    /// Gives `root` depth `depth` and its descendants theirs below it; see
    /// [`crate::store::NodeRecord::depth`]. Walks the subtree on the heap.
    pub(super) fn set_subtree_depth(&mut self, root: StableNodeId, depth: u16) {
        let record = self.record_mut(root);
        if record.hierarchy.children.is_empty() {
            record.depth = depth;
            return;
        }
        let mut open = vec![(root, depth)];
        while let Some((id, depth)) = open.pop() {
            let record = self.record_mut(id);
            if record.depth == depth && id != root {
                // Moved with the rest of its subtree before: already right.
                continue;
            }
            record.depth = depth;
            let below = depth.saturating_add(1);
            open.extend(
                record
                    .hierarchy
                    .children
                    .iter()
                    .map(|&child| (child, below)),
            );
        }
    }

    /// Levels from the root down to `id`, counting it.
    pub(super) fn node_depth(&self, id: StableNodeId) -> Option<usize> {
        let depth = usize::from(self.nodes.get(id)?.depth);
        #[cfg(test)]
        {
            // Every test checks the kept depth against the walk it replaces.
            let mut walked = 1;
            let mut cursor = self.nodes.get(id)?.hierarchy.parent;
            while let Some(ancestor) = cursor {
                walked += 1;
                cursor = self.nodes.get(ancestor)?.hierarchy.parent;
            }
            assert_eq!(depth, walked, "kept depth of {id:?} is stale");
        }
        Some(depth)
    }

    fn unlink_from_parent(&mut self, id: StableNodeId) -> bool {
        let Some(parent) = self.record(id).hierarchy.parent else {
            return false;
        };
        // Removing a child changes the parent's flow and exported metrics even
        // when the removed subtree itself is no longer live. Keep the live
        // parent in the typed frontier so the retained layout cannot preserve
        // the old child placement.
        self.record_child_list_invalidation(parent);
        self.remove_child(parent, id);
        self.hierarchy_mut(id).parent = None;
        self.set_subtree_depth(id, 1);
        self.mark_ancestors(
            parent,
            DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
        );
        self.note_structural_change(parent);
        // A rule that read the parent, and localized text that read a scope
        // above it, read what is above them now.
        self.responsive_reparented(id);
        self.i18n_reparented(id, Some(parent));
        true
    }

    fn leave_live_document(&mut self, root: StableNodeId) {
        if self.record(root).hierarchy.children.is_empty() {
            self.retire_subtree_from_document(std::slice::from_ref(&root));
        } else {
            let subtree = self.subtree_ids(root);
            self.retire_subtree_from_document(&subtree);
        }
        self.tree.detached.insert(root);
        if self.mount_state(root) == Some(MountState::Parked) {
            self.tree.detached_mounted.remove(&root);
        } else {
            self.tree.detached_mounted.insert(root);
        }
        self.sync_subtree_presence(root);
    }

    fn retire_subtree_from_document(&mut self, subtree: &[StableNodeId]) {
        for &id in subtree {
            let document = self.record(id).document;
            if self.input.focused.get(&document) == Some(&id) {
                self.input.focused.remove(&document);
            }
            self.remove_ime(id);
            if let Some(index) = self.hit_index.hit_test_index.get_mut(&document) {
                retain_hit_tree(index, id);
            }
            self.invalidation.pending_render_removals.push(id);
            self.invalidation.pending_accessibility_removals.push(id);
        }

        // Pointer state and animations to release: none in a page being
        // built, whose every node parks once on creation.
        if !(self.input.pointer_captures.is_empty()
            && self.input.pointer_hover.is_empty()
            && self.input.pointer_press.is_empty()
            && self.motion.animations.is_empty())
        {
            self.release_pointers_and_animations_of(subtree);
        }

        for &id in subtree {
            self.remove_surface_presence(id);
            self.motion.hover_transitions.remove(&id);
            if self.overlay_host(id).is_some() {
                self.write_overlay_host(id, Some(OverlayHostState::default()));
            }
        }
        self.clear_overlay_references_for(subtree);
        // Sorted and deduplicated once, when system work takes them: doing
        // it here, once per removed subtree, made dropping n rows O(n² log n).
    }

    /// Release the pointer captures, hovers and presses on `subtree`'s
    /// nodes and cancel their non-overlay animations.
    fn release_pointers_and_animations_of(&mut self, subtree: &[StableNodeId]) {
        let parked = subtree.iter().copied().collect::<HashSet<_>>();
        let released = self
            .input
            .pointer_captures
            .iter()
            .filter_map(|(&(document, pointer_id), &target)| {
                parked
                    .contains(&target)
                    .then_some((document, pointer_id, target))
            })
            .collect::<Vec<_>>();
        for (document, pointer_id, target) in released {
            self.input.pointer_captures.remove(&(document, pointer_id));
            self.input
                .pending_pointer_capture_changes
                .push(PointerCaptureChange {
                    pointer_id,
                    target,
                    captured: false,
                });
        }
        self.input
            .pointer_hover
            .retain(|_, target| !parked.contains(target));
        self.input
            .pointer_press
            .retain(|_, target| !parked.contains(target));
        self.drop_non_overlay_animations_for_parked(&parked);
    }

    /// Out of its siblings' flow: positioned, or a free item its explicit
    /// rect places.
    fn positioned_out_of_flow(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|record| record.resolved_layout.position.is_out_of_flow())
            || self.is_free_item(id)
    }

    pub(crate) fn layout_isolated(&self, id: StableNodeId) -> bool {
        self.node_style(id).is_some_and(|node| {
            let style=&node.layout;
            style.layout_isolation
                && matches!(style.width, Some(nana_ui_core::LengthSpec::Px(value)) if value.is_finite() && value>=0.0)
                && matches!(style.height, Some(nana_ui_core::LengthSpec::Px(value)) if value.is_finite() && value>=0.0)
                && style.position==nana_ui_core::PositionSpec::Static
                && !style.display.is_some_and(nana_ui_core::DisplaySpec::is_grid_container)
                && style.min_width.is_none() && style.max_width.is_none()
                && style.min_height.is_none() && style.max_height.is_none()
        })
    }

    fn mark_ancestors(&mut self, start: StableNodeId, bits: u16) {
        // Layout propagation is represented by typed frontier seeds. Ancestor
        // dirty masks still carry render/input/accessibility work, but they do
        // not synthesize a second coarse layout channel.
        let mut bits = bits & !DirtyMask::LAYOUT;
        let mut current = Some(start);
        while let Some(id) = current {
            current = self
                .identity_and_parent(id)
                .expect("hierarchy node must exist")
                .1;
            if bits & DirtyMask::INPUT != 0 {
                self.hit_index.non_scroll_hit_dirty.insert(id);
            }
            if !self.mark_keeping_render_reason(id, bits, MarkOrigin::General, true) {
                break;
            }
            if self.layout_isolated(id) {
                bits &= !(DirtyMask::LAYOUT | DirtyMask::RENDER);
                if bits == 0 {
                    break;
                }
            }
        }
    }

    /// Classify `changed`, a write to `id`'s layout, as `id` now stands:
    /// whether its aspect ratio ties its axes, and the line its parent lays
    /// it along, a column unless the parent says otherwise.
    fn classify_layout_change(
        &self,
        id: StableNodeId,
        changed: nana_ui_core::LayoutStyleChange,
    ) -> LayoutInvalidation {
        let aspect_ratio = self
            .nodes
            .get(id)
            .is_some_and(|record| record.resolved_layout.aspect_ratio.is_some());
        let parent_line = self
            .parent_id(id)
            .and_then(|parent| self.nodes.get(parent))
            .map(|record| {
                record
                    .resolved_layout
                    .direction
                    .unwrap_or(nana_ui_core::FlexDirection::Column)
            });
        layout_style_invalidation(changed, aspect_ratio, parent_line)
    }

    /// Seeds layout from text whose metrics moved. Returns whether the change
    /// went on to the parent's measurement: a fixed box keeps it.
    fn propagate_layout_from_node(&mut self, id: StableNodeId) -> bool {
        let fixed = self.is_fixed_metric_boundary(id);
        let display = self
            .nodes
            .get(id)
            .and_then(|record| record.resolved_layout.display);
        // A fixed box with text of its own -- a fixed-size button's label --
        // exports the same border box whatever the text says, unless it is
        // inline-level or its parent aligns it by baseline: its content lays
        // out again, and nothing above it measures. A fixed inline-block's
        // shaped text does not change the border box the outer line packed
        // either: the seed stays on the atomic, which is only placed again.
        // One whose baseline its text puts is not fixed (see
        // `is_fixed_metric_boundary`): its line aligns again.
        let atomic = matches!(
            display,
            Some(nana_ui_core::DisplaySpec::InlineFlex | nana_ui_core::DisplaySpec::InlineGrid)
        );
        if fixed && (atomic || !display.is_some_and(nana_ui_core::DisplaySpec::is_inline_level)) {
            let (kind, fields) = if atomic {
                (InvalidationKind::PLACEMENT, LayoutStyleChange::TYPOGRAPHY)
            } else {
                (
                    InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                    LayoutStyleChange::TYPOGRAPHY.union(LayoutStyleChange::INTRINSIC),
                )
            };
            self.record_layout_invalidation(
                id,
                LayoutInvalidation::new(
                    LayoutInvalidationSource::Text,
                    InvalidationReason::TEXT,
                    kind,
                    fields,
                    LayoutDependencyFootprint::NONE,
                ),
            );
            self.mark_keeping_render_reason(id, DirtyMask::RENDER, MarkOrigin::General, true);
            self.bump_last_counters(|counters| counters.record_text_metrics(1, 0, 0));
            return false;
        }
        self.record_layout_invalidation(
            id,
            LayoutInvalidation::new(
                LayoutInvalidationSource::Text,
                InvalidationReason::TEXT,
                InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                LayoutStyleChange::TYPOGRAPHY.union(LayoutStyleChange::INTRINSIC),
                METRIC_EXPORTS,
            ),
        );
        // Keep the original paint invalidation for the node whose shaped
        // metrics changed; the typed layout cause above remains narrow.
        self.mark_keeping_render_reason(id, DirtyMask::RENDER, MarkOrigin::General, true);
        let mut parent_reflows = 0;
        if let Some(parent) = self.parent_id(id) {
            if self.is_fixed_metric_boundary(parent) {
                self.record_layout_invalidation(
                    parent,
                    LayoutInvalidation::new(
                        LayoutInvalidationSource::Text,
                        InvalidationReason::TEXT,
                        InvalidationKind::PLACEMENT,
                        LayoutStyleChange::FLOW,
                        LayoutDependencyFootprint::NONE,
                    ),
                );
                self.mark_keeping_render_reason(
                    parent,
                    DirtyMask::RENDER,
                    MarkOrigin::General,
                    true,
                );
            } else {
                // The parent measures again for the metrics it reads; whether
                // its own box moved is what that finds out. Like every other
                // ancestor on the text's export path it couples no siblings:
                // when it did grow, its container's placement moves the ones
                // after it. Coupled, it linked every later sibling of a long
                // page into the frontier.
                self.record_layout_invalidation(
                    parent,
                    LayoutInvalidation::new(
                        LayoutInvalidationSource::Text,
                        InvalidationReason::TEXT,
                        InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                        LayoutStyleChange::INTRINSIC.union(LayoutStyleChange::FLOW),
                        LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS,
                    ),
                );
                self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
                parent_reflows = 1;
            }
        }
        self.bump_last_counters(|counters| counters.record_text_metrics(1, 0, parent_reflows));
        parent_reflows > 0
    }
}

/// Adds counts observed between drains onto `counters`. They carry no live
/// entity count, so `counters` keeps its own.
fn add_pending_counts(counters: &mut WorkCounters, mut pending: WorkCounters) {
    pending.entities_total = counters.entities_total;
    counters.accumulate(pending);
}

/// What a virtual list's pass cost (Issue #262). Counted into the next
/// drain: the frame that lays the pass's commits out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct VirtualRowCounts {
    /// Row extents that moved in the row index.
    pub(crate) metric_updates: usize,
    /// Row index entries written since the list last placed its rows.
    pub(crate) prefix_index_updates: usize,
    /// Mounted rows the list took a new height from.
    pub(crate) rows_remeasured: usize,
    /// Mounted rows whose placement the list patched.
    pub(crate) rows_repositioned: usize,
    /// Logical rows looked up, by index or by key.
    pub(crate) logical_rows_scanned: usize,
    /// Moves of the list's total height.
    pub(crate) scroll_extent_updates: usize,
    /// Rows created for the window.
    pub(crate) rows_materialized: usize,
}

/// A typography scale text can be set at: a positive finite number.
fn valid_text_scale(scale: f32) -> bool {
    scale.is_finite() && scale > 0.0
}

/// What a box whose own metrics moved can move: its intrinsic size and
/// baseline, read by its parent, and the siblings and context around it.
const METRIC_EXPORTS: LayoutDependencyFootprint = LayoutDependencyFootprint::intrinsic_container()
    .union(LayoutDependencyFootprint::EXPORTS_BASELINE)
    .union(LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX)
    .union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING);

/// Which result chain a walk is: one that drops results, or one that hides
/// them until publish confirms them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ResultChainWalk {
    Clear,
    Suppress,
}

fn definite_fixed_border(style: &LayoutStyle) -> bool {
    matches!(style.width, Some(LengthSpec::Px(value)) if value.is_finite() && value >= 0.0)
        && matches!(style.height, Some(LengthSpec::Px(value)) if value.is_finite() && value >= 0.0)
        && style.min_width.is_none()
        && style.max_width.is_none()
        && style.min_height.is_none()
        && style.max_height.is_none()
}

/// The writing mode and direction a node lays out in: its own
/// `writing-mode` / `direction`, or else its parent's — inherited, as CSS
/// inherits them.
///
/// The layout style carries only what a node declares; the record carries
/// what it inherits ([`NodeRecord::inherited_writing`], written when
/// styles resolve). Reading the declared value first keeps a node that
/// sets its own writing mode right even before its style has been
/// resolved.
fn record_writing(record: &NodeRecord) -> nana_ui_core::WritingContext {
    let declared = &record.resolved_layout;
    let inherited = record.inherited_writing;
    nana_ui_core::WritingContext::used(
        declared.writing_mode.unwrap_or(inherited.mode),
        declared.dir.unwrap_or(inherited.direction),
        declared
            .text_ext
            .text_orientation
            .unwrap_or(record.inherited_orientation),
    )
}

/// [`UiWorld::containing_writing`] for a record already in hand: what it
/// inherits is its parent's writing context. A root is its own containing
/// block's frame.
fn record_containing_writing(record: &NodeRecord) -> nana_ui_core::WritingContext {
    if record.hierarchy.parent.is_some() {
        let parent = record.inherited_writing;
        nana_ui_core::WritingContext::used(
            parent.mode,
            parent.direction,
            record.inherited_orientation,
        )
    } else {
        record_writing(record)
    }
}

/// What a layout pass reads of a node ([`UiWorld::layout_input_view`]):
/// [`LayoutInput`] borrowing the record's child list and, where it can, its
/// layout style. The rare modal part is asked for where it is used
/// ([`UiWorld::modal_layout_input`]): a pass's table holds one of these per
/// node it reads, and 48 bytes keep a slot at 56.
#[derive(Debug)]
pub(crate) struct LayoutInputView<'a> {
    pub children: &'a Arc<Vec<StableNodeId>>,
    pub style: LayoutStyleRef<'a>,
    /// What measurement asks of [`Self::style`], decided once per style.
    pub facts: crate::layout_engine::StyleFacts,
    pub writing: nana_ui_core::WritingContext,
    pub containing_writing: nana_ui_core::WritingContext,
    pub text_metrics: Option<crate::TextMetrics>,
}

/// A node's layout style as a pass holds it: borrowed from the world, or an
/// owned handle where the pass derived it (motion, overlays, a hidden box).
#[derive(Debug)]
pub(crate) enum LayoutStyleRef<'a> {
    Borrowed(&'a Arc<nana_ui_core::LayoutStyle>),
    Owned(Arc<nana_ui_core::LayoutStyle>),
}

impl LayoutStyleRef<'_> {
    /// An owned handle, for a caller that keeps the style past the borrow.
    pub(crate) fn to_arc(&self) -> Arc<nana_ui_core::LayoutStyle> {
        Arc::clone(self.arc())
    }

    /// The handle, wherever it lives.
    pub(crate) fn arc(&self) -> &Arc<nana_ui_core::LayoutStyle> {
        match self {
            Self::Borrowed(style) => style,
            Self::Owned(style) => style,
        }
    }

    /// The same style for as long as the world is borrowed: a copy of the
    /// borrow, or another handle to the owned one.
    pub(crate) fn shared<'b>(&self) -> LayoutStyleRef<'b>
    where
        Self: 'b,
    {
        match self {
            Self::Borrowed(style) => LayoutStyleRef::Borrowed(style),
            Self::Owned(style) => LayoutStyleRef::Owned(Arc::clone(style)),
        }
    }

    /// A borrow of this style for as long as `self` is borrowed.
    pub(crate) fn reborrow(&self) -> LayoutStyleRef<'_> {
        LayoutStyleRef::Borrowed(self.arc())
    }
}

impl std::ops::Deref for LayoutStyleRef<'_> {
    type Target = nana_ui_core::LayoutStyle;

    fn deref(&self) -> &Self::Target {
        self.arc()
    }
}

impl AsRef<nana_ui_core::LayoutStyle> for LayoutStyleRef<'_> {
    fn as_ref(&self) -> &nana_ui_core::LayoutStyle {
        self
    }
}

const IDENTITY_AFFINE: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PresenceFlags {
    confirm: bool,
    modal_frame: bool,
    menu_surface: bool,
    clip: bool,
    list_leading: bool,
    z_index: bool,
    /// The node's own style is inert.
    inert: bool,
    /// An open triggered menu, whose surface hangs above the page.
    triggered: bool,
    /// An open select menu, which paints above the page.
    select_menu: bool,
    /// Style resolves against the viewport (`position: fixed`, `vw` / `vh`).
    viewport: bool,
}

impl PresenceFlags {
    const NONE: Self = Self {
        confirm: false,
        modal_frame: false,
        menu_surface: false,
        clip: false,
        list_leading: false,
        z_index: false,
        inert: false,
        triggered: false,
        select_menu: false,
        viewport: false,
    };
}

fn bump_presence(count: &mut usize, was_present: bool, now_present: bool) {
    if was_present == now_present {
        return;
    }
    if now_present {
        *count = count.saturating_add(1);
    } else {
        *count = count.saturating_sub(1);
    }
}

fn apply_triggered_overlay(
    layout: &mut nana_ui_core::LayoutStyle,
    overlay: crate::TriggeredMenuOverlay,
) {
    layout.position = PositionSpec::Fixed;
    layout.width = Some(LengthSpec::Px(
        (overlay.width - overlay.padding * 2.0).max(0.0),
    ));
    layout
        .z_index
        .get_or_insert(crate::popover::MENU_OVERLAY_Z_INDEX);
}

fn text_intrinsic_changed(previous: TextMetrics, next: TextMetrics) -> bool {
    // Baseline is an exported metric consumed by baseline-aligned parents.
    // Width/height-only comparison lets a baseline-only shape update leave a
    // retained placement plan stale even though the child still needs to
    // participate in the parent's formatting-context solve.
    previous.width != next.width || previous.height != next.height || previous.ascent != next.ascent
}

pub(crate) fn intersect_layout_boxes(left: LayoutBox, right: LayoutBox) -> Option<LayoutBox> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    (right_edge > x && bottom_edge > y).then_some(LayoutBox {
        x,
        y,
        width: right_edge - x,
        height: bottom_edge - y,
    })
}

/// A box's overflow clip and containing block: the two anchors that follow
/// from its ancestors' (see [`UiWorld::anchor_pair_memo`]).
#[derive(Clone, Copy)]
struct AnchorPair {
    clip: Option<StableNodeId>,
    containing_block: Option<StableNodeId>,
}

/// The boxes a result names around its node; see [`UiWorld::layout_anchors`].
#[derive(Clone, Copy, PartialEq)]
struct LayoutAnchors {
    clip: Option<StableNodeId>,
    clip_parent: Option<StableNodeId>,
    containing_block: Option<StableNodeId>,
}

impl LayoutAnchors {
    fn recorded_in_meta(self, meta: &PublishedLayoutMeta) -> bool {
        meta.clip() == self.clip
            && meta.clip_parent() == self.clip_parent
            && meta.containing_block() == self.containing_block
    }
}

fn text_content_part_width(meta: &PublishedLayoutMeta) -> Option<f32> {
    if let Some(ext) = meta.ext.as_ref() {
        ext.parts.iter().find_map(|part| {
            (part.kind == crate::LayoutPartKind::TextContent).then_some(part.bounds.width)
        })
    } else if meta.single_text_source != TEXT_SOURCE_NONE_SENTINEL {
        Some(meta.inline_text_box().width)
    } else {
        None
    }
}

/// The kind of every child fragment of `node`: what its children take part
/// in. Layout records the context it ran; before it has, the style alone
/// answers provisionally.
fn child_fragment_kind(node: &NodeRecord) -> crate::LayoutFragmentKind {
    node.layout_context
        .unwrap_or_else(|| crate::FormattingContextKind::from_display(&node.resolved_layout))
        .child_fragment_kind()
}

/// The context `node` lays out its own content in. A container reports what
/// layout ran; a leaf with text runs its own lines, which belong to the text
/// engine, not to layout nodes; an empty leaf establishes none.
fn established_context(node: &NodeRecord) -> Option<crate::FormattingContextKind> {
    if !node.hierarchy.children.is_empty() {
        return node.layout_context;
    }
    record_has_text(node).then_some(crate::FormattingContextKind::Inline)
}

fn record_has_text(node: &NodeRecord) -> bool {
    matches!(node.kind.as_ref(), NodeKind::Text) || node.has_text
}

fn union_layout_boxes(left: LayoutBox, right: LayoutBox) -> LayoutBox {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let bottom_edge = (left.y + left.height).max(right.y + right.height);
    LayoutBox {
        x,
        y,
        width: (right_edge - x).max(0.0),
        height: (bottom_edge - y).max(0.0),
    }
}

/// Drain/layout shaper adapter. `UiWorld::shape_text` and
/// `shape_text_for_layout` construct this around the host `TextShaper`
/// (`MeasureTextShaper`, `NanaTextShaper`, …). It is not a test-only wrapper:
/// empty-state / modal / presentation helpers also call `self.shape` on it.
fn initial_interaction(kind: &NodeKind) -> InteractionState {
    match kind {
        NodeKind::Text | NodeKind::Comment => InteractionState {
            pointer_events: false,
            focusable: false,
        },
        NodeKind::Document | NodeKind::Element { .. } => InteractionState::default(),
    }
}

fn validate_text_metrics(id: StableNodeId, metrics: TextMetrics) -> Result<(), UiWorldError> {
    if !metrics.width.is_finite()
        || !metrics.height.is_finite()
        || metrics.width < 0.0
        || metrics.height < 0.0
    {
        return Err(UiWorldError::InvalidText(id));
    }
    Ok(())
}

fn node_presentation_eq(left: &NodeStyle, right: &NodeStyle) -> bool {
    left.foreground == right.foreground
        && left.background == right.background
        && left.border == right.border
        && left.interaction == right.interaction
        && left.text_horizontal_alignment == right.text_horizontal_alignment
        && left.text_vertical_alignment == right.text_vertical_alignment
        && left.painter == right.painter
        && left.color_overrides == right.color_overrides
        && left.theme_patch == right.theme_patch
}

/// Equal but for transforms, the cursor and inertness (each marked on its
/// own), given whether the presentations are equal (`node_presentation_eq`)
/// and what changed in layout.
fn style_excluding_transform_and_cursor_eq(
    presentation_eq: bool,
    changed: nana_ui_core::LayoutStyleChange,
) -> bool {
    use nana_ui_core::LayoutStyleChange as Change;
    presentation_eq
        && changed
            .difference(Change::TRANSFORM.union(Change::CURSOR).union(Change::INERT))
            .is_empty()
}

/// True when the write changes only fields layout resolves into a box: a
/// field paint also reads off the style (border widths and styles, how text
/// breaks, aligns and ends) is not one, since changing it alone may leave
/// every box in place and still change the pixels. Transforms and the
/// cursor ride along without counting. `presentation_eq` is
/// `node_presentation_eq` of the two styles.
fn style_change_is_layout_geometry_only(
    presentation_eq: bool,
    changed: nana_ui_core::LayoutStyleChange,
) -> bool {
    use nana_ui_core::LayoutStyleChange as Change;
    if !presentation_eq {
        return false;
    }
    changed.intersects(Change::LAYOUT)
        && changed
            .difference(
                Change::GEOMETRY
                    .union(Change::TRANSFORM)
                    .union(Change::CURSOR)
                    .union(Change::INERT),
            )
            .is_empty()
}

/// Own box, padding, fragment, and clip. Child placements stay on the children,
/// so a parent republished only for those must not patch hit testing from the root.
fn layout_result_projects_new_geometry(
    previous: &PublishedLayoutMeta,
    next: &PublishedLayoutMeta,
) -> bool {
    // `geometry_eq` with the containing block and dependencies left out, field by field so a box that moved answers
    // before any list is compared.
    let own_changed = previous.width != next.width
        || previous.height != next.height
        || previous.padding_box() != next.padding_box()
        || previous.content_box != next.content_box
        || previous.first_baseline() != next.first_baseline()
        || previous.last_baseline() != next.last_baseline()
        || previous.overflow_box() != next.overflow_box()
        || previous.scroll_extent() != next.scroll_extent()
        || previous.clip() != next.clip()
        || previous.clip_parent() != next.clip_parent()
        || previous.formatting_context != next.formatting_context
        || previous.participation != next.participation;
    // A result holds nothing of where its children sit: a child whose box
    // moved publishes and projects itself, and its parent is not marked.
    own_changed || !previous.fragments_and_parts_eq(next)
}

/// Whether `next` changes more of [`layout_result_projects_new_geometry`]
/// than the scroll extent, scroll metrics, overflow and baselines: the box,
/// its inner boxes, its clip links, its context or its own fragments and
/// parts. A result that changes only those leaves what the box clips cut by
/// the same box.
fn layout_result_reshapes(previous: &PublishedLayoutMeta, next: &PublishedLayoutMeta) -> bool {
    previous.width != next.width
        || previous.height != next.height
        || previous.padding_box() != next.padding_box()
        || previous.content_box != next.content_box
        || previous.clip() != next.clip()
        || previous.clip_parent() != next.clip_parent()
        || previous.formatting_context != next.formatting_context
        || previous.participation != next.participation
        || !previous.fragments_and_parts_eq(next)
}

/// Classify a style mutation at the layout authority boundary: the
/// dependency classes the retained frontier consumes, from what changed.
/// `aspect_ratio` says the node ties its two axes, so a size on one moves
/// the other. `parent_line` is the direction of the line the node sits in,
/// along which a flex factor sizes it; `None` when the node has no parent.
/// Whether an item's size along its parent's line, `line` the constraint
/// that runs along it, is what the line leaves it: a grow factor, or a fill.
/// A sibling's size moving along the line moves such an item's too. An item
/// sized by its content or a length of its own stays (a shrink on overflow
/// is left to its memo, keyed by the extent it is offered).
fn flexes_along(style: &nana_ui_core::LayoutStyle, line: LayoutDependencyFootprint) -> bool {
    let size = if line == LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT {
        style.width
    } else {
        style.height
    };
    style.flex_grow.is_some_and(|grow| grow > 0.0) || size == Some(LengthSpec::Fill)
}

fn layout_style_invalidation(
    changed: nana_ui_core::LayoutStyleChange,
    aspect_ratio: bool,
    parent_line: Option<nana_ui_core::FlexDirection>,
) -> LayoutInvalidation {
    use nana_ui_core::LayoutStyleChange as Change;
    if !changed.intersects(Change::LAYOUT) {
        return LayoutInvalidation::none();
    }
    let writing = changed.intersects(Change::WRITING);
    let spacing = changed.intersects(Change::SPACING);
    let sizing = changed.intersects(Change::SIZING);
    let position = changed.intersects(Change::POSITION);
    let grid = changed.intersects(Change::GRID);
    let flow = changed.intersects(Change::FLOW) || grid;
    let alignment = changed.intersects(Change::ALIGNMENT);
    let typography = changed.intersects(Change::TEXT_LAYOUT);
    let scroll = changed.intersects(Change::SCROLL);
    // Border widths and styles take or give room without a group of their
    // own: classified as everything, as before.
    let fields = changed.intersection(Change::LAYOUT);

    let mut kind = if position
        && !sizing
        && !spacing
        && !flow
        && !alignment
        && !typography
        && !writing
        && !scroll
    {
        InvalidationKind::PLACEMENT
    } else {
        InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT)
    };
    if writing {
        kind = kind.union(InvalidationKind::WRITING_CONTEXT);
    }
    if scroll {
        kind = kind.union(InvalidationKind::SCROLL_OVERFLOW);
    }
    let mut footprint = LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS
        .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE)
        .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK);
    if typography {
        footprint = footprint
            .union(LayoutDependencyFootprint::EXPORTS_BASELINE)
            .union(LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX)
            .union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING);
    }
    if sizing || spacing || flow || alignment {
        footprint = footprint
            .union(LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX)
            .union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING);
        // The constraint the node hands its children moves on the axis the
        // write sized: a width the inline one, a height the block one, a flex
        // factor the one its parent's line runs along. Spacing, alignment and
        // the shape of the flow move both, and so does a size on a box whose
        // aspect ratio ties its axes.
        let factors_only = flow && !changed.intersects(Change::FLOW_SHAPE);
        let both = spacing
            || alignment
            || aspect_ratio
            || (flow && !factors_only)
            || (factors_only && parent_line.is_none());
        let along = |line: nana_ui_core::FlexDirection| factors_only && parent_line == Some(line);
        if both
            || changed.intersects(Change::SIZING_WIDTH)
            || along(nana_ui_core::FlexDirection::Row)
        {
            footprint =
                footprint.union(LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT);
        }
        if both
            || changed.intersects(Change::SIZING_HEIGHT)
            || along(nana_ui_core::FlexDirection::Column)
        {
            footprint =
                footprint.union(LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT);
        }
    }
    if position {
        footprint = footprint.union(LayoutDependencyFootprint::DEPENDS_ON_CONTAINING_BLOCK);
    }
    if writing {
        footprint = footprint.union(LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT);
    }
    let mut reason = InvalidationReason::STYLE;
    if writing {
        reason = reason.union(InvalidationReason::WRITING);
    }
    if scroll {
        reason = reason.union(InvalidationReason::SCROLL);
    }
    LayoutInvalidation::new(
        LayoutInvalidationSource::Author,
        reason,
        kind,
        fields,
        footprint,
    )
}

mod block_stretch;
mod deep_walk_tests;
mod line_box_memo;
mod list_item_tooltip;
mod mount_under_narrow_row;
mod pending_causes;
#[cfg(test)]
mod placement_commit;
mod reflow_oracle;
#[cfg(test)]
mod tests;
