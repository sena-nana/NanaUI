mod accessibility;
mod animation;
mod block_reads;
mod child_numbering;
pub(crate) mod child_order;
mod clip_chain;
mod extraction;
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
mod content_parts;
pub(crate) mod inline_content;
mod scroll_bounds;
mod sticky;
mod style;
mod style_pairs;
mod text;
use hit_test::*;
pub(crate) use text::TextDisplayView;
pub(crate) use text::text_visual_key;
use text::*;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
    mem::size_of,
    sync::Arc,
    time::Duration,
};

use nana_ui_core::{
    ControlSize, InvalidationKind, InvalidationReason, LayoutDependencyFootprint, LayoutFieldMask,
    LayoutInvalidation, LayoutInvalidationSource, LayoutStyle, LengthSpec, PointerEventsSpec,
    PositionSpec, SemanticColorRole, SemanticPalette, StyleModelRef, SwitchControlPosition,
    ThemeAppearance, ThemeWorkCounters,
};

#[cfg(feature = "calendar")]
use nana_ui_core::TooltipConfig;
#[cfg(feature = "graph-canvas")]
use nana_ui_core::{GraphPoint, GraphPortKind, GraphPortSide, GraphRect, GraphSize, cubic_point};

use crate::{
    AccessibilityDelta, AccessibilityLive, AccessibilityNode, AccessibilityRole,
    AccessibilityState, AnimationFrame, AnimationId, AnimationSpec, ComponentTypeId, ComputedStyle,
    CustomRenderNode, EventListeners, EventRoute, ExtractedMove, ExtractedNode, ExtractedTextSpan,
    HighlightRequest, ImeComposition, InteractionState, LayoutBox, LayoutInput, MotionWorkCounters,
    MountState, MutationQueue, NodeStyle, OverlayHostState, PointerCaptureChange, ScrollMetrics,
    ScrollOffset, StandardVisual, TextContent, TextMetrics, TextPresentation, TextPresenter,
    TextShaper, TextVerticalAlignment, UiMutation, WorkCounters,
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

/// Lists one publish builds every result's slices in, reused node to node so
/// a result's slices are allocated once, at their final length.
/// A published layout result, kept in the node's own coordinates: its
/// border box at the origin, each child's placement as the child's local
/// box (see [`crate::LayoutFrame`]). A box that only moves keeps it.
#[derive(Debug, Clone)]
pub(crate) struct RetainedLayoutResult {
    pub(crate) local: Arc<crate::LayoutResult>,
}

/// A result placed on the page, and the local result and origin it was
/// placed from: it answers while both hold.
struct PageLayoutResult {
    local: Arc<crate::LayoutResult>,
    origin: [u32; 2],
    placed: Arc<crate::LayoutResult>,
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
fn scroll_container(layout: &nana_ui_core::LayoutStyle, visual: Option<&StandardVisual>) -> bool {
    layout.overflow_x.scrolls()
        || layout.overflow_y.scrolls()
        || matches!(visual, Some(StandardVisual::Scrollbar { .. }))
}

/// See [`UiWorld::overflow_contained_axes`].
fn overflow_contained_axes(
    layout: &nana_ui_core::LayoutStyle,
    visual: Option<&StandardVisual>,
) -> [bool; 2] {
    if scroll_container(layout, visual) {
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

fn menu_surface_open(visual: Option<&StandardVisual>) -> Option<bool> {
    match visual {
        Some(StandardVisual::MenuSurface { open, .. }) => Some(*open),
        _ => None,
    }
}

/// Menu rows are painted from one retained surface, so their virtual
/// accessibility children change without a retained child mutation. Keep the
/// semantic projection in sync when rows, filtering, or the highlighted item
/// changes even though the surface remains open.
pub(crate) fn menu_surface_accessibility_changed(
    previous: Option<&StandardVisual>,
    next: Option<&StandardVisual>,
) -> bool {
    match (previous, next) {
        (
            Some(StandardVisual::MenuSurface {
                open: previous_open,
                rows: previous_rows,
                highlighted: previous_highlighted,
                query: previous_query,
                ..
            }),
            Some(StandardVisual::MenuSurface {
                open: next_open,
                rows: next_rows,
                highlighted: next_highlighted,
                query: next_query,
                ..
            }),
        ) => {
            previous_open != next_open
                || previous_rows != next_rows
                || previous_highlighted != next_highlighted
                || previous_query != next_query
        }
        (Some(StandardVisual::MenuSurface { .. }), _)
        | (_, Some(StandardVisual::MenuSurface { .. })) => true,
        // A select field's open menu is a list box of virtual options: they
        // follow its rows, its highlight, and where it is placed and
        // scrolled. The pointer heating the scrollbar changes none of them.
        (
            Some(StandardVisual::Select {
                opened: previous_open,
                options: previous_options,
                highlighted: previous_highlighted,
                menu_scroll: previous_scroll,
                menu_room: previous_room,
                size: previous_size,
                fit_options: previous_fit,
                ..
            }),
            Some(StandardVisual::Select {
                opened,
                options,
                highlighted,
                menu_scroll,
                menu_room,
                size,
                fit_options,
                ..
            }),
        ) => {
            previous_open != opened
                || (*opened
                    && (previous_options != options
                        || previous_highlighted != highlighted
                        || previous_scroll.offset != menu_scroll.offset
                        || previous_room != menu_room
                        || previous_size != size
                        || previous_fit != fit_options))
        }
        (Some(StandardVisual::Select { .. }), _) | (_, Some(StandardVisual::Select { .. })) => true,
        _ => false,
    }
}

/// A tree's rows and their toggles are virtual accessibility children too:
/// its semantics follow the visible rows (toggle states included), the
/// active one and the toggle columns' names.
pub(crate) fn tree_view_accessibility_changed(
    previous: Option<&StandardVisual>,
    next: Option<&StandardVisual>,
) -> bool {
    match (previous, next) {
        (
            Some(StandardVisual::TreeView {
                rows: previous_rows,
                active: previous_active,
                column_labels: previous_labels,
                ..
            }),
            Some(StandardVisual::TreeView {
                rows,
                active,
                column_labels,
                ..
            }),
        ) => previous_rows != rows || previous_active != active || previous_labels != column_labels,
        (Some(StandardVisual::TreeView { .. }), _) | (_, Some(StandardVisual::TreeView { .. })) => {
            true
        }
        _ => false,
    }
}

/// Whether a menu surface with `visual` hides its child `child`: a closed
/// surface hides its items, never the content that draws its trigger.
pub(crate) fn closed_menu_hides(visual: Option<&StandardVisual>, child: StableNodeId) -> bool {
    match visual {
        Some(StandardVisual::MenuSurface {
            open: false,
            overlay,
            ..
        }) => overlay.and_then(|overlay| overlay.trigger_content) != Some(child),
        _ => false,
    }
}

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
    InvalidStandardVisual(StableNodeId),
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
            Self::InvalidStandardVisual(id) => {
                write!(
                    formatter,
                    "node {} has invalid standard visual state",
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

/// The sole authoritative retained identity and hierarchy store.
pub struct UiWorld {
    input: input::WorldInputState,
    nodes: NodeStore,
    retired: RetiredIds,
    dirty_entities: HashSet<StableNodeId, BuildIdHasher>,
    /// Typed layout causes accumulated by the mutation authority. This
    /// pending queue carries each dependency footprint through the drain so
    /// retained layout can avoid widening every seed to `ALL`.
    pending_layout_invalidations: NodeMap<LayoutInvalidation>,
    /// Monotonic, non-consuming invalidation epoch. Input routing uses this
    /// to report whether dispatch scheduled work without scanning or draining
    /// the retained work queues.
    pending_work_revision: u64,
    hit_test_index: HashMap<DocumentId, HitIndex, BuildIdHasher>,
    /// Scroll deltas awaiting the in-place hit-index patch (see
    /// `UiMutation::SetScrollOffset`). Drained by the frame driver.
    scroll_hit_updates: Vec<(StableNodeId, [f32; 2])>,
    /// Input changes that cannot be represented by scroll translation alone.
    non_scroll_hit_dirty: HashSet<StableNodeId, BuildIdHasher>,
    scroll_content_bounds: RefCell<scroll_bounds::ContentBoundsIndex>,
    /// Every node styled `overflow: auto | scroll` (what a `ScrollView`
    /// projects too). Despawned ids are dropped when a commit re-measures.
    scroll_containers: NodeSet,
    /// Scroll containers whose style changed in the commit being applied,
    /// or that stopped scrolling.
    scroll_restyled: Vec<StableNodeId>,
    /// The commit being applied wrote a box or moved a node.
    scroll_layout_touched: bool,
    /// Offsets the commit being applied set, as requested.
    scroll_requested: Vec<(StableNodeId, ScrollOffset)>,
    /// Scroll containers whose offset that re-measure clamped, for the
    /// framework to announce. Drained by `take_scroll_reclamped`.
    scroll_reclamped: HashSet<StableNodeId, BuildIdHasher>,
    /// Scroll containers whose viewport size a re-measure changed since the
    /// last [`Self::take_scroll_resized`].
    scroll_resized: HashSet<StableNodeId, BuildIdHasher>,
    /// Last recording of each custom-painted node, keyed by what the painter
    /// promised decides its output (Issue #217). Extraction reads through it,
    /// so an unchanged node is never re-recorded. Only painted nodes have an
    /// entry; despawn drops it.
    paint_recordings: RefCell<crate::NodeMap<PaintCacheEntry>>,
    /// Each box's overflow clip and containing block, kept while one batch
    /// of layout results publishes and `None` otherwise: nothing they are
    /// computed from changes during a publish, and a box's pair follows from
    /// its ancestors', so a batch finds each box's pair once instead of
    /// walking up to the root for every box. See [`Self::layout_anchors`].
    anchor_memo: RefCell<Option<crate::NodeMap<AnchorPair>>>,
    /// Markdown diagrams recorded against the theme, by laid-out scene and
    /// palette epoch. Cleared when the epoch moves; bounded.
    #[cfg(feature = "rich-text")]
    diagram_recordings: RefCell<Vec<(u64, crate::markdown_diagram::DiagramDrawing)>>,
    /// Last recording of each modal frame's card surface painter
    /// ([`crate::Dialog::surface_painter`]), apart from the node's own
    /// painter: a frame can have both. Geometry reads through it; despawn
    /// drops it.
    modal_surface_recordings: RefCell<crate::NodeMap<PaintCacheEntry>>,
    /// The `nana-text` engine the host last shaped with, for painters that
    /// measure text while recording. `None` until a host shapes through one;
    /// painters then measure with the em-based fallback.
    paint_text_engine: Option<nana_text::SharedTextEngine>,
    /// Painters set on nodes apart from their style
    /// ([`crate::UiMutation::SetPainter`]). Empty in a tree nobody paints.
    painter_overrides: crate::NodeMap<crate::NodePainter>,
    pending_render_removals: Vec<StableNodeId>,
    pending_accessibility_removals: Vec<StableNodeId>,
    animations: HashMap<AnimationId, ActiveAnimation>,
    /// Running width / height / padding / margin tracks by target, in start
    /// order: what a node's layout overlay reads instead of every animation.
    layout_length_tracks: HashMap<StableNodeId, Vec<AnimationId>, BuildIdHasher>,
    pub(crate) animation_now: Duration,
    /// Editors whose caret is in the hidden half of its blink. Paint only:
    /// extraction leaves their caret out; nothing else reads it.
    hidden_carets: Vec<StableNodeId>,
    presentation: nana_ui_core::motion::PresentationStore,
    /// Input / a11y / focus queries increment this. Idle `advance_animations`
    /// must not, including compositor-only overlays.
    presentation_query_samples: Cell<usize>,
    /// `presentation_query_samples` at the start of the last
    /// [`Self::advance_animations`]. Inspector "samples/frame" is the delta.
    presentation_samples_at_advance: Cell<usize>,
    /// Last `advance_animations` attribution (mutations / layout / style /
    /// extract). Track classification is recomputed live.
    last_motion_frame: MotionWorkCounters,
    compositor_layer_requests: HashSet<StableNodeId, BuildIdHasher>,
    motion_descriptors: nana_ui_core::motion::MotionDescriptorStore,
    /// Mutation-scoped Finished/Cancelled not yet observed. A later successful
    /// commit closes this batch so park-cancel does not leak onto an idle
    /// `advance_animations`. Deadline completions still go out on that wake.
    pending_animation_events: Vec<crate::AnimationEvent>,
    surface_motion: HashMap<StableNodeId, motion::SurfaceMotion, BuildIdHasher>,
    closing_surfaces: HashSet<StableNodeId, BuildIdHasher>,
    /// How much of a modal frame's scrim shows while it fades, 0 to 1. A
    /// frame with no entry shows its whole scrim.
    scrim_presence: HashMap<StableNodeId, f32, BuildIdHasher>,
    /// Rows of a reorder list that follow the pointer in a previewed drag,
    /// each with its list: they stack at `REORDER_PREVIEW_Z_INDEX`.
    #[cfg(feature = "controls")]
    reorder_previews: HashMap<StableNodeId, StableNodeId, BuildIdHasher>,
    /// The reduce-motion preference the application context resolved:
    /// surfaces open and close at once while it holds.
    reduced_motion: bool,
    hover_transitions: HashMap<StableNodeId, style::HoverTransition, BuildIdHasher>,
    animation_deadlines: BTreeSet<(Duration, AnimationId)>,
    /// The installed design system. This is the authority; `style_model` below
    /// is its hot slice, cached so per-node resolution does not chase an `Arc`
    /// and copy a kilobyte to read one colour.
    theme: Arc<nana_ui_core::CompiledTheme>,
    /// The design system as installed, before the system high-contrast
    /// overlay. `theme` is this one, or its high-contrast rendition while
    /// `high_contrast` is on; the installed one is never overwritten.
    installed_theme: Arc<nana_ui_core::CompiledTheme>,
    high_contrast: bool,
    style_model: StyleModelRef,
    generation: u64,
    /// Monotonic revision of published layout output. This is deliberately
    /// separate from [`Self::generation`], which also advances for paint and
    /// input state, so paint-only work cannot invalidate layout consumers.
    layout_generation: u64,
    /// Bumps when layout *inputs* change (style, tree, text, scroll, theme).
    /// `WriteLayout` does not bump it, so a full pass can reuse a previous
    /// viewport's boxes while the document content stays the same.
    layout_source_epoch: u64,
    /// What the framework's own controls say; see [`Self::framework_strings`].
    framework_strings: Arc<nana_ui_core::FrameworkStrings>,
    /// Canonical render/input-facing geometry. `NodeRecord::layout` remains a
    /// compatibility cache for old internal paths; all public geometry reads
    /// go through this table when a result has been published.
    layout_results: crate::NodeMap<RetainedLayoutResult>,
    /// Results hidden after a typed invalidation until publish confirms them.
    /// The `Arc` stays so a bit-equivalent recompute can keep the same object
    /// and generation; [`Self::layout_result`] reports these as absent.
    layout_results_suppressed: NodeSet,
    /// [`Self::layout_result`]'s answers: each result placed where its box
    /// stands on the page, while it and its box hold.
    page_layout_results: RefCell<crate::NodeMap<PageLayoutResult>>,
    /// Boxes the commit being applied wrote with a new origin, and whether
    /// that is all that changed, and every box it wrote with a new origin or
    /// size: the boxes under the first move with them
    /// ([`Self::settle_layout_moves`]).
    layout_moved_roots: Vec<(StableNodeId, bool)>,
    layout_written: NodeSet,
    /// Document boxes the commit being applied wrote, placed in their frames
    /// once its tree is in place.
    pending_document_layouts: Vec<(StableNodeId, LayoutBox)>,
    /// Nodes whose position the commit being applied changed: the boxes
    /// under them may now be measured from another frame.
    layout_reframe_roots: Vec<StableNodeId>,
    /// Counts [`Self::extract_nodes_reusing`] calls: a node's render marks
    /// (`NodeRecord::render_full_epoch`) say why it is dirty since the last.
    render_epoch: Cell<u64>,
    /// While extraction derives a component's geometry in the component's
    /// own coordinates ([`Self::local_component_geometry`]): that node. Every
    /// box read meanwhile is measured from its border-box origin, so the
    /// geometry is the same wherever the node stands.
    geometry_frame: Cell<Option<StableNodeId>>,
    /// Set when a box read under [`Self::geometry_frame`] is not inside that
    /// node, or something else on the page is read: geometry that depends on
    /// where the node stands.
    geometry_read_page: Cell<bool>,
    /// Page origins a read-only pass remembers (`world/page_origins.rs`).
    page_origins: RefCell<page_origins::PageOrigins>,
    /// While a scoped hit rebuild runs, which changes no style: the pointer
    /// events deep nodes use, remembered ([`Self::used_pointer_events`]).
    pointer_events_memo: RefCell<Option<NodeMap<PointerEventsSpec>>>,
    /// Set while a layout move marks its box: the INPUT it marks is the
    /// move's alone (no [`DirtyMask::INPUT_ELSEWHERE`]).
    marking_layout_move: bool,
    /// Set while a scroll marks its box: the INPUT it marks is the scroll's
    /// ([`DirtyMask::INPUT_SCROLLED`], no [`DirtyMask::INPUT_ELSEWHERE`]).
    marking_scroll: bool,
    /// Every accessibility node a projection handed out, as it handed it
    /// out: the nodes a delta leaves alone are checked against it.
    #[cfg(any(test, feature = "layout-verify"))]
    pub(super) accessibility_shipped: accessibility::ShippedAccessibility,
    /// Set while the frame driver commits a layout pass's box writes: it
    /// publishes the whole pass scope right after, so the commit leaves the
    /// written boxes to that publish: it would build each result only for
    /// the scope publish to check it again.
    pass_publishes_written_boxes: bool,
    /// Nodes that started or stopped being a containing block or a clip:
    /// every result under one records it as an anchor, so the next publish
    /// checks them although their boxes may not have moved.
    layout_anchor_roots: HashSet<StableNodeId>,
    /// Cursor declarations changed since the last system-work drain.
    cursor_style_dirty: bool,
    presenters: HashMap<String, Box<dyn TextPresenter>>,
    spawned_since_drain: usize,
    despawned_since_drain: usize,
    /// Boxed with the other two sets below them: each is 1.7 KiB, and inline
    /// the four were almost half of the world, which every construction of
    /// a context moves.
    counters: Box<WorldCounters>,
    frame_extracted_nodes: usize,
    frame_extracted_spans: usize,
    accumulating_frame: bool,
    /// Layout/document-order allocs recorded from `&self` hot paths.
    pending_hot_allocations: Cell<usize>,
    pending_hot_allocated_bytes: Cell<usize>,
    /// Layout seeds queued since the world was made: a running count the
    /// paths that make them read before and after, to say whose they were.
    layout_seeds_created: u64,
    /// Nodes whose style declares design intent (a radius tier, a control
    /// height or padding step, a surface inset): the only ones a metrics
    /// install can resolve differently.
    intent_nodes: HashSet<StableNodeId, BuildIdHasher>,
    /// Replaced nodes by the resource they show, and what each resource
    /// reported of its size.
    replaced: replaced::ReplacedIndex,
    /// Responsive rules by container, and the bucket each is in (Issue #265).
    responsive: responsive::ResponsiveIndex,
    /// Localized text by the scope it reads its locale from (Issue #268).
    i18n: i18n::I18nIndex,
    text_layout_cache: crate::text_layout_cache::TextLayoutCache,
    glyph_cache: crate::GlyphCache,
    /// The backend plain text last resolved against. A different one on a
    /// later pass means every resolved text node is stale.
    text_backend: Option<crate::text_node::TextBackendEpoch>,
    /// The application's language, for text no node above it names one for.
    default_language: Option<nana_text::font::LanguageTag>,
    /// The text engine's fallback language, below the application's.
    engine_fallback_language: Option<nana_text::font::LanguageTag>,
    /// The application's typography scale, for text no scope or window
    /// above it sets one for.
    default_text_scale: f32,
    /// Windows' typography scales, over the application's.
    document_text_scales: HashMap<DocumentId, f32, BuildIdHasher>,
    /// Whether `text_backend` changed — its first install included —
    /// since the framework last asked, for geometry a component spends at
    /// projection time from a measurement.
    text_backend_changed: bool,
    /// Adaptive editors shaped since their component fit was last refreshed.
    text_input_adaptive_updates: Vec<StableNodeId>,
    /// Text work of the last pass, or of the last frame that ran one.
    text_work: nana_text::TextWorkCounters,
    /// Text work of the frame being accumulated.
    text_frame_work: nana_text::TextWorkCounters,
    /// Editable work (#96) committed since the last text pass: edits,
    /// caret- and selection-only changes, composition updates. Reported with
    /// the pass that lays those changes out.
    pending_edit_work: nana_text::TextWorkCounters,
    /// Nodes style resolution turned visible since the last scheduled text
    /// pass, which re-resolves them alongside its own work.
    text_shown: Vec<StableNodeId>,
    /// Theme/style work (Issue #101) of the last style pass or theme install,
    /// or of the last frame that ran one.
    theme_work: ThemeWorkCounters,
    /// Theme/style work of the frame being accumulated.
    theme_frame_work: ThemeWorkCounters,
    /// Token-authority reads recorded from the `&self` palette paths. Folded
    /// into `theme_work` by the pass that made them. Counted in its own
    /// `Cell` rather than through a pending `ThemeWorkCounters`: this is the
    /// hottest observation on the style path — once per role a resolve reads
    /// — and a whole-struct read-modify-write per read is measurable.
    pending_theme_reads: Cell<usize>,
    /// `LayoutStyle` copies recorded from the style-write paths, folded on
    /// the same boundary. Separate from the reads because a write is rare
    /// and the two are never observed together.
    pending_layout_copies: Cell<usize>,
    /// Recently written layouts, shared by nodes whose layouts are equal.
    layouts: style::LayoutInterner,
    /// Recently written node styles, shared by records whose styles are equal.
    node_styles: style::NodeStyleInterner,
    /// Recent pairs of resolved layouts a restyle compared, and what it found.
    layout_pairs: style_pairs::LayoutPairMemo,
    /// Live Confirm modal frames. Extract, a11y, and hit-test skip ancestor
    /// confirm walks when this is zero.
    confirm_modals: usize,
    /// Live EmptyState + ModalFrame visuals that clip descendants.
    clip_visuals: usize,
    /// Nodes with an authored `z-index`, or an open triggered menu overlay
    /// whose children inject z-index. Stacking walks skip when this is zero.
    z_index_nodes: usize,
    sticky_nodes: crate::StickyDependencies,
    /// Live open triggered menus (Popover, ActionMenu, HoverCard). Their
    /// surfaces take the pointer at the content's level, above the page, so
    /// hit testing looks them up here instead of walking the tree.
    triggered_overlays: HashSet<StableNodeId, BuildIdHasher>,
    /// Live open Select, Dropdown and SearchDropdown menus. A menu paints in
    /// the root stacking context at its level, over the groups of the page
    /// around its field, so hit testing ranks by paint order where one is.
    select_menus: HashSet<StableNodeId, BuildIdHasher>,
    /// Live nodes whose box resolves against the viewport (`position: fixed`,
    /// `vw` / `vh`). A resize dirties this set together with document roots
    /// instead of discarding the retained layout cache.
    viewport_basis_nodes: usize,
    /// Ancestors visited keeping block extent reads current, for cost tests.
    #[cfg(test)]
    block_read_steps: u64,
    /// Children a run of removals from one parent already took out of its
    /// list, in one pass; their own mutations skip that edit (#389).
    unlinked_ahead: NodeSet,
    /// Parents whose children the commit being applied has still to number,
    /// each with how many children at the start and at the end of its list
    /// none of its edits reached; `None` outside its apply loop. See
    /// [`Self::renumber_children_between`].
    renumber_later: Option<NodeMap<(usize, usize)>>,
    /// Children given a number, for cost tests.
    #[cfg(test)]
    children_numbered: u64,
    /// The result chains the commit being applied has walked; `None`
    /// outside its apply loop. See [`ResultChainMemo`].
    result_chain_memo: Option<ResultChainMemo>,
    /// Nodes the result chains visited, for cost tests.
    #[cfg(test)]
    result_chain_steps: u64,
    /// Walk every result chain in full, for tests that compare against it.
    #[cfg(test)]
    result_chain_memo_off: bool,
    /// Child-list entries removals have shifted, filtered or renumbered, in
    /// validation and apply, for cost tests.
    #[cfg(test)]
    child_list_steps: u64,
    viewport_basis: HashMap<DocumentId, HashSet<StableNodeId>, BuildIdHasher>,
    /// Nodes that accept a drop, and what they accept. A sparse index rather
    /// than a field on every node: almost no tree has drop targets.
    drop_targets: HashMap<StableNodeId, nana_ui_core::DropAccepts, BuildIdHasher>,
    /// Innermost file-drop hover target, if any. Scene paints overlay chrome.
    drop_hover: Option<(StableNodeId, nana_ui_core::DropEffect)>,
    /// Document-level text selection (not a second TextInput). One range per document.
    document_text_selections: HashMap<DocumentId, crate::DocumentTextSelection, BuildIdHasher>,
    /// Viewport each document was last laid out against.
    ///
    /// Geometry projection runs on `&UiWorld` with no window context, but
    /// overlay surfaces that the framework places itself (the `Select` menu)
    /// have to fold back inside the window near its edges. Layout is the one
    /// place that already knows the viewport, so it records it here.
    document_viewports: HashMap<DocumentId, crate::LayoutViewport, BuildIdHasher>,
    /// Last applied presence flags per entity, so park/remove/despawn can
    /// decrement without double-counting.
    presence_flags: HashMap<StableNodeId, PresenceFlags, BuildIdHasher>,
    /// Subtree roots detached by Remove or Park. Mounted document/scene roots
    /// are created with no parent and are not in this set.
    detached: HashSet<StableNodeId, BuildIdHasher>,
    /// The `detached` roots that are still `Mounted` (a `Detach`, not a
    /// park). A parked root's whole subtree is `Parked`, so `is_mounted`
    /// already answers presence under it; only these need an ancestor walk.
    detached_mounted: HashSet<StableNodeId, BuildIdHasher>,
    /// Live roots per document: `parent.is_none()` and [`Self::presence_live`].
    live_document_roots: HashMap<DocumentId, BTreeSet<StableNodeId>, BuildIdHasher>,
    /// Nodes carrying an `OverlayHostState` component. Overlay bookkeeping walks
    /// this index instead of every entity, so clearing references from a removed
    /// node costs the host count rather than the world size.
    overlay_host_nodes: HashSet<StableNodeId, BuildIdHasher>,
    overlay_hosts_by_document: HashMap<DocumentId, HashSet<StableNodeId>, BuildIdHasher>,
    /// Live nodes grouped by the component that created them.
    ///
    /// Several per-pointer-event paths need "every X in this document" -- the
    /// split-handle slop probe, the calendar heatmap hover release -- and each
    /// answered by walking `document_order` and filtering, which is a
    /// full-document walk per event for a tree that usually contains none of
    /// the thing being looked for.
    ///
    /// It lives beside `component_type` rather than in `AppContext` so there is
    /// one authority: every path that gives a node a component type does it
    /// through `SetComponentType`, including the semantic-binding path that
    /// never touches `stamp_component_type`.
    nodes_by_component: HashMap<ComponentTypeId, NodeSet>,
    overlay_dependents: HashMap<StableNodeId, HashSet<StableNodeId>, BuildIdHasher>,
    /// `control -> label`: the node whose text names a control that has no
    /// label of its own ([`MutationQueue::set_labelled_by`]).
    labelled_by: HashMap<StableNodeId, StableNodeId, BuildIdHasher>,
    /// `label -> controls` it names: projected again when its text changes,
    /// and let go of with it.
    labels: HashMap<StableNodeId, HashSet<StableNodeId>, BuildIdHasher>,
    /// Live regions and how they announce, as the composites that own them
    /// set them ([`MutationQueue::set_live`]).
    live_regions: HashMap<StableNodeId, AccessibilityLive, BuildIdHasher>,
    /// Nodes visited by mutation validation since the last drain, summed over
    /// every commit the next frame will consume. Validation must scale with the
    /// batch, not the retained world; this is the sentinel for that invariant.
    validation_nodes_scanned: usize,
    /// Bumped on `SetTheme`; extract skips a second palette walk when it matches.
    palette_epoch: u64,
    /// Parents whose child list changed since the last drain (insert, detach,
    /// despawn). Consumers take the list once per commit to schedule opt-in
    /// component reprojections; see `ComponentView::wants_child_reproject`.
    structural_change_parents: Vec<StableNodeId>,
    /// Blocks whose strut the next text pass considers again: their child
    /// list or a child's display changed, or their font did. See
    /// [`Self::resolve_block_struts`].
    strut_candidates: Vec<StableNodeId>,
    /// minimap 行长单条缓存（原始值 → 每逻辑行非空白字符数）。存于
    /// `RefCell` 供 `&self` 的 presentation 构建路径读写。
    minimap_line_lengths_cache: RefCell<Option<(crate::TextValue, Vec<u32>)>>,
    /// 括号配对着色单条缓存（原始值 → 配对/未配对 span 表）。值未变
    /// （纯光标/选区同步）时复用上一次 O(n) 单趟栈扫描结果。存于
    /// `RefCell` 供 `&self` 的 presentation 构建路径读写。
    bracket_color_spans_cache: RefCell<Option<(crate::TextValue, Arc<[(usize, usize, usize)]>)>>,
    presentation_transform_documents_cache: RefCell<Option<(u64, u64, HashSet<DocumentId>)>>,
    /// Queries answered by the hit index; see [`Self::hit_test_queries`].
    hit_test_queries: Cell<u64>,
    /// Reused by `hit_test` when it has to order candidates by paint.
    hit_scratch: RefCell<hit_test::HitScratch>,
}

impl Default for UiWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl UiWorld {
    pub(crate) fn take_window_cursor_dirty(&mut self) -> bool {
        std::mem::take(&mut self.cursor_style_dirty)
    }

    pub fn new() -> Self {
        Self {
            input: input::WorldInputState::default(),
            nodes: NodeStore::new(),
            retired: RetiredIds::default(),
            dirty_entities: HashSet::default(),
            pending_layout_invalidations: NodeMap::default(),
            pending_work_revision: 0,
            hit_test_index: HashMap::default(),
            scroll_hit_updates: Vec::new(),
            non_scroll_hit_dirty: HashSet::default(),
            scroll_content_bounds: RefCell::new(scroll_bounds::ContentBoundsIndex::default()),
            scroll_containers: NodeSet::default(),
            scroll_restyled: Vec::new(),
            scroll_layout_touched: false,
            scroll_requested: Vec::new(),
            scroll_reclamped: HashSet::default(),
            scroll_resized: HashSet::default(),
            paint_recordings: RefCell::new(crate::NodeMap::default()),
            anchor_memo: RefCell::new(None),
            #[cfg(feature = "rich-text")]
            diagram_recordings: RefCell::new(Vec::new()),
            modal_surface_recordings: RefCell::new(crate::NodeMap::default()),
            paint_text_engine: None,
            painter_overrides: crate::NodeMap::default(),
            pending_render_removals: Vec::new(),
            pending_accessibility_removals: Vec::new(),
            animations: HashMap::new(),
            layout_length_tracks: HashMap::default(),
            animation_now: Duration::ZERO,
            hidden_carets: Vec::new(),
            presentation: nana_ui_core::motion::PresentationStore::new(),
            presentation_query_samples: Cell::new(0),
            presentation_samples_at_advance: Cell::new(0),
            last_motion_frame: MotionWorkCounters::default(),
            compositor_layer_requests: HashSet::default(),
            motion_descriptors: nana_ui_core::motion::MotionDescriptorStore::new(),
            pending_animation_events: Vec::new(),
            surface_motion: HashMap::default(),
            closing_surfaces: HashSet::default(),
            scrim_presence: HashMap::default(),
            #[cfg(feature = "controls")]
            reorder_previews: HashMap::default(),
            reduced_motion: false,
            hover_transitions: HashMap::default(),
            animation_deadlines: BTreeSet::new(),
            theme: nana_ui_core::builtin_theme_arc(ThemeAppearance::default()),
            installed_theme: nana_ui_core::builtin_theme_arc(ThemeAppearance::default()),
            high_contrast: false,
            style_model: StyleModelRef::default(),
            generation: 0,
            layout_generation: 0,
            layout_source_epoch: 1,
            framework_strings: Arc::default(),
            layout_results: crate::NodeMap::default(),
            layout_results_suppressed: NodeSet::default(),
            page_layout_results: RefCell::default(),
            layout_moved_roots: Vec::new(),
            layout_written: NodeSet::default(),
            pending_document_layouts: Vec::new(),
            layout_reframe_roots: Vec::new(),
            render_epoch: Cell::new(0),
            geometry_frame: Cell::new(None),
            geometry_read_page: Cell::new(false),
            page_origins: RefCell::default(),
            pointer_events_memo: RefCell::new(None),
            marking_layout_move: false,
            marking_scroll: false,
            #[cfg(any(test, feature = "layout-verify"))]
            accessibility_shipped: Default::default(),
            pass_publishes_written_boxes: false,
            layout_anchor_roots: HashSet::new(),
            cursor_style_dirty: false,
            presenters: HashMap::new(),
            spawned_since_drain: 0,
            despawned_since_drain: 0,
            counters: Box::default(),
            frame_extracted_nodes: 0,
            frame_extracted_spans: 0,
            accumulating_frame: false,
            pending_hot_allocations: Cell::new(0),
            pending_hot_allocated_bytes: Cell::new(0),
            layout_seeds_created: 0,
            intent_nodes: HashSet::default(),
            replaced: replaced::ReplacedIndex::default(),
            responsive: responsive::ResponsiveIndex::default(),
            i18n: i18n::I18nIndex::default(),
            text_layout_cache: crate::text_layout_cache::TextLayoutCache::default(),
            glyph_cache: crate::GlyphCache::default(),
            text_backend: None,
            default_language: None,
            engine_fallback_language: None,
            default_text_scale: 1.0,
            document_text_scales: HashMap::default(),
            text_backend_changed: false,
            text_input_adaptive_updates: Vec::new(),
            text_work: nana_text::TextWorkCounters::default(),
            text_frame_work: nana_text::TextWorkCounters::default(),
            pending_edit_work: nana_text::TextWorkCounters::default(),
            text_shown: Vec::new(),
            theme_work: ThemeWorkCounters::default(),
            theme_frame_work: ThemeWorkCounters::default(),
            pending_theme_reads: Cell::new(0),
            pending_layout_copies: Cell::new(0),
            layouts: style::LayoutInterner::default(),
            node_styles: style::NodeStyleInterner::default(),
            layout_pairs: style_pairs::LayoutPairMemo::default(),
            confirm_modals: 0,
            clip_visuals: 0,
            z_index_nodes: 0,
            sticky_nodes: crate::StickyDependencies::default(),
            triggered_overlays: HashSet::default(),
            select_menus: HashSet::default(),
            viewport_basis_nodes: 0,
            #[cfg(test)]
            block_read_steps: 0,
            unlinked_ahead: NodeSet::default(),
            renumber_later: None,
            #[cfg(test)]
            children_numbered: 0,
            result_chain_memo: None,
            #[cfg(test)]
            result_chain_steps: 0,
            #[cfg(test)]
            result_chain_memo_off: false,
            #[cfg(test)]
            child_list_steps: 0,
            viewport_basis: HashMap::default(),
            document_viewports: HashMap::default(),
            drop_targets: HashMap::default(),
            drop_hover: None,
            document_text_selections: HashMap::default(),
            presence_flags: HashMap::default(),
            detached: HashSet::default(),
            detached_mounted: HashSet::default(),
            live_document_roots: HashMap::default(),
            overlay_host_nodes: HashSet::default(),
            overlay_hosts_by_document: HashMap::default(),
            nodes_by_component: HashMap::new(),
            overlay_dependents: HashMap::default(),
            labelled_by: HashMap::default(),
            labels: HashMap::default(),
            live_regions: HashMap::default(),
            validation_nodes_scanned: 0,
            palette_epoch: 1,
            structural_change_parents: Vec::new(),
            strut_candidates: Vec::new(),
            minimap_line_lengths_cache: RefCell::new(None),
            bracket_color_spans_cache: RefCell::new(None),
            presentation_transform_documents_cache: RefCell::new(None),
            hit_test_queries: Cell::new(0),
            hit_scratch: RefCell::new(hit_test::HitScratch::default()),
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
        if self.hidden_carets == carets {
            return;
        }
        let previous = std::mem::replace(&mut self.hidden_carets, carets.to_vec());
        for id in previous.into_iter().chain(carets.iter().copied()) {
            if self.nodes.contains(id) {
                self.mark(id, DirtyMask::RENDER);
            }
        }
    }

    pub(crate) fn caret_hidden(&self, id: StableNodeId) -> bool {
        self.hidden_carets.contains(&id)
    }

    pub(crate) fn mark_layout(&mut self, id: StableNodeId) {
        if self.nodes.contains(id) {
            self.record_layout_invalidation(
                id,
                LayoutInvalidation::new(
                    LayoutInvalidationSource::Runtime,
                    InvalidationReason::UNKNOWN,
                    InvalidationKind::ALL,
                    LayoutFieldMask::ALL,
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
        self.pending_layout_invalidations
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
            self.counters
                .pending_layout_invalidations_counted
                .layout_invalidations_zero_delta += 1;
            return;
        }
        self.counters
            .pending_layout_invalidations_counted
            .layout_invalidations_created += 1;
        self.layout_seeds_created += 1;
        if invalidation.source == LayoutInvalidationSource::Text {
            self.counters
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
        let _ = self.mark_scroll_compatible(id, crate::schedule::DirtyMask::LAYOUT);
    }

    /// A virtual list's pass cost `rows`.
    pub(crate) fn note_virtual_rows(&mut self, rows: VirtualRowCounts) {
        let pending = &mut self.counters.pending_drain_counts;
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
        let seeds = (self.layout_seeds_created - before) as usize;
        self.counters
            .pending_drain_counts
            .component_state_layout_seeds += seeds;
    }

    /// Queue `invalidation` for `id`, merged into a cause already pending:
    /// a narrower typed seed must not hide a later, wider one. Every cause
    /// enters through here; each caller then sets the layout dirty bit
    /// through [`Self::mark_scroll_compatible`], which moves the input epoch
    /// of the full-layout snapshot.
    fn queue_layout_invalidation(&mut self, id: StableNodeId, invalidation: LayoutInvalidation) {
        self.pending_layout_invalidations
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
                LayoutFieldMask::FLOW,
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
                LayoutFieldMask::FLOW,
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
            .pending_layout_invalidations
            .keys()
            .copied()
            .filter(|id| self.document_of(*id) == Some(document))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        let mut seeds = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(invalidation) = self.pending_layout_invalidations.remove(&id) {
                seeds.push(crate::LayoutFrontierSeed::new(id, invalidation));
            }
        }
        seeds
    }

    pub(crate) fn clear_layout_frontier_seeds(&mut self, document: DocumentId) {
        let ids = self
            .pending_layout_invalidations
            .keys()
            .copied()
            .filter(|id| self.document_of(*id) == Some(document))
            .collect::<Vec<_>>();
        for id in ids {
            self.pending_layout_invalidations.remove(&id);
        }
    }

    pub(crate) fn restore_layout_frontier_seeds(&mut self, seeds: &[crate::LayoutFrontierSeed]) {
        for seed in seeds {
            if !self.nodes.contains(seed.node) || seed.invalidation.is_empty() {
                continue;
            }
            self.queue_layout_invalidation(seed.node, seed.invalidation);
            self.invalidate_layout_result(seed.node);
            let _ = self.mark_scroll_compatible(seed.node, DirtyMask::LAYOUT);
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
        self.retired.contains(id)
    }

    /// Total IDs retired over this world's lifetime.
    pub fn retired_ids(&self) -> usize {
        self.retired.len()
    }

    /// Coalesced runs backing the retired ledger. Sequential allocation keeps
    /// this near-constant while [`Self::retired_ids`] grows with churn.
    pub fn retired_id_runs(&self) -> usize {
        self.retired.runs()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// What the framework's own controls say: the title bar's buttons, a
    /// dialog's confirm and cancel, the built-in settings page. Controls
    /// read it when they are projected or assembled.
    pub fn framework_strings(&self) -> &nana_ui_core::FrameworkStrings {
        &self.framework_strings
    }

    pub(crate) fn set_framework_strings(&mut self, strings: Arc<nana_ui_core::FrameworkStrings>) {
        self.framework_strings = strings;
    }

    pub(crate) fn layout_source_epoch(&self) -> u64 {
        self.layout_source_epoch
    }

    pub(crate) fn note_layout_source_change(&mut self) {
        self.layout_source_epoch = self.layout_source_epoch.wrapping_add(1);
    }

    /// Layout-length animations change used sizes without a style write,
    /// and an open triggered menu places its items where its trigger shows
    /// through scroll offsets and transforms (z-index nodes count those
    /// menus). A viewport snapshot must not answer those frames.
    pub(crate) fn layout_source_reusable(&self) -> bool {
        self.layout_length_tracks.is_empty() && self.z_index_nodes == 0
    }

    /// Revision of the last published canonical layout snapshot. Unlike the
    /// world mutation generation this does not advance for paint, hover,
    /// transform or input-only work.
    pub fn layout_generation(&self) -> u64 {
        self.layout_generation
    }

    /// Canonical geometry for `id`, if the node has been laid out, on the
    /// page: its result placed where its box stands. Placed once per result
    /// and position; [`Self::local_layout_result`] is the result as kept.
    pub fn layout_result(&self, id: StableNodeId) -> Option<Arc<crate::LayoutResult>> {
        let local = self.local_layout_result(id)?;
        let origin = self.layout_origin(id)?;
        let key = [origin[0].to_bits(), origin[1].to_bits()];
        if let Some(page) = self.page_layout_results.borrow().get(&id)
            && page.origin == key
            && Arc::ptr_eq(&page.local, local)
        {
            return Some(Arc::clone(&page.placed));
        }
        let placed = Arc::new(self.place_layout_result_on_page(id, local, origin));
        self.page_layout_results.borrow_mut().insert(
            id,
            PageLayoutResult {
                local: Arc::clone(local),
                origin: key,
                placed: Arc::clone(&placed),
            },
        );
        Some(placed)
    }

    /// `id`'s published result in its own coordinates: its border box at the
    /// origin. A box that only moved keeps it.
    pub fn local_layout_result(&self, id: StableNodeId) -> Option<&Arc<crate::LayoutResult>> {
        if self.layout_results_suppressed.contains(&id) {
            return None;
        }
        self.layout_results.get(&id).map(|result| &result.local)
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
        self.local_layout_result(id)?;
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
        self.local_layout_result(id)?;
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
        self.local_layout_result(id)?;
        let origin = self.layout_origin(id)?;
        Some(self.child_fragments(id, Some(origin)))
    }

    /// [`Self::layout_child_fragments`] from `id`'s border-box origin.
    pub fn local_layout_child_fragments(
        &self,
        id: StableNodeId,
    ) -> Option<Vec<crate::LayoutFragment>> {
        self.local_layout_result(id)?;
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
                let bounds = match origin {
                    None => record.local_layout,
                    Some(origin) => {
                        let at = if record.layout_frame == crate::LayoutFrame::Parent {
                            origin
                        } else {
                            self.frame_origin_of(record, record.layout_frame)
                        };
                        crate::layout_result::place_box(at, record.local_layout)
                    }
                };
                Some((index, child, bounds))
            })
    }

    /// `local`, `id`'s result, placed with its border box at `origin`: its
    /// own geometry moved there. Children measured from another frame join
    /// the overflow here, on the page.
    /// Whether the result [`Self::layout_result`] last placed on the page
    /// for `local` still joins `id`'s children from other frames where they
    /// stand now. One placed for another result or another position, or
    /// none, is not what a read gives now: the next read places it afresh.
    fn placed_layout_result_holds(
        &self,
        id: StableNodeId,
        local: &Arc<crate::LayoutResult>,
    ) -> bool {
        let Some(origin) = self.layout_origin(id) else {
            return true;
        };
        let key = [origin[0].to_bits(), origin[1].to_bits()];
        let pages = self.page_layout_results.borrow();
        let Some(page) = pages
            .get(&id)
            .filter(|page| page.origin == key && Arc::ptr_eq(&page.local, local))
        else {
            return true;
        };
        let placed = self.place_layout_result_on_page(id, local, origin);
        page.placed.overflow == placed.overflow && page.placed.scroll_extent == placed.scroll_extent
    }

    fn place_layout_result_on_page(
        &self,
        id: StableNodeId,
        local: &crate::LayoutResult,
        origin: [f32; 2],
    ) -> crate::LayoutResult {
        let mut result = local.clone().placed_frame(origin);
        result.bounds.x = origin[0];
        result.bounds.y = origin[1];
        result.border_box.x = origin[0];
        result.border_box.y = origin[1];
        let mut escaping = None;
        let node = self.nodes.get(id);
        let joins_every_child = node.is_some_and(|node| !node.resolved_layout.clips_overflow())
            && local.scroll_metrics.is_none();
        for &child in node.map_or(&[][..], |node| node.hierarchy.children.as_ref()) {
            let Some(record) = self.nodes.get(child) else {
                continue;
            };
            if record.layout_frame == crate::LayoutFrame::Parent {
                continue;
            }
            if joins_every_child || self.clip_parent_of(child) == Some(id) {
                let at = self.frame_origin_of(record, record.layout_frame);
                let bounds = crate::layout_result::place_box(at, record.local_layout);
                escaping = Some(escaping.map_or(bounds, |all| union_layout_boxes(all, bounds)));
            }
        }
        result.fragments = local
            .fragments
            .iter()
            .map(|fragment| crate::LayoutResult::place_fragment(origin, fragment))
            .collect();
        result.parts = local
            .parts
            .iter()
            .map(|part| crate::LayoutResult::place_part(origin, part))
            .collect();
        if let Some(escaping) = escaping {
            result.overflow = union_layout_boxes(result.overflow, escaping);
            if local.scroll_metrics.is_none() {
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
        let from_owner = match record.layout_frame {
            crate::LayoutFrame::Parent => record.hierarchy.parent == Some(owner),
            crate::LayoutFrame::Block(block) => block == owner,
            crate::LayoutFrame::Page => false,
        };
        if from_owner {
            return Some(record.local_layout);
        }
        let [x, y] = self.record_layout_origin(record);
        let [owner_x, owner_y] = self.layout_origin(owner)?;
        Some(LayoutBox {
            x: x - owner_x,
            y: y - owner_y,
            ..record.local_layout
        })
    }

    pub(crate) fn clear_layout_results_subtree(&mut self, root: StableNodeId) {
        if self.layout_results.is_empty()
            && self.layout_results_suppressed.is_empty()
            && self.page_layout_results.get_mut().is_empty()
        {
            // Nothing published yet, so nothing to clear below `root`.
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self
                .nodes
                .get(id)
                .map(|node| node.hierarchy.children.clone())
                .unwrap_or_default();
            self.layout_results.remove(&id);
            self.layout_results_suppressed.remove(&id);
            self.page_layout_results.get_mut().remove(&id);
            stack.extend(children.iter().copied());
        }
    }

    /// Drop `start` and ancestors whose placement depends on it.
    ///
    /// A definite border box is a metric boundary. Clearing it republishes an
    /// inner child placement; its parent still describes the same border box,
    /// so the walk stops there. `start_changes_border` continues through
    /// `start` itself when that node's own border box is what changed.
    fn clear_layout_result_chain(&mut self, start: StableNodeId, start_changes_border: bool) {
        if self.layout_results.is_empty() && self.layout_results_suppressed.is_empty() {
            // Nothing published yet, so nothing to clear up the chain.
            return;
        }
        self.walk_layout_result_chain(start, start_changes_border, ResultChainWalk::Clear);
    }

    /// Walk from `start` up its ancestors, applying `walk` to each, to the
    /// first that absorbs the change: a fixed metric boundary
    /// ([`Self::is_fixed_metric_boundary`]), past `start` itself when
    /// `start_changes_border`.
    ///
    /// While a commit applies, the walk stops at an ancestor an earlier walk
    /// of that commit already took ([`ResultChainMemo`]): what lies above it
    /// was done then, and only shrinks while the commit applies.
    fn walk_layout_result_chain(
        &mut self,
        start: StableNodeId,
        start_changes_border: bool,
        walk: ResultChainWalk,
    ) {
        let memo = self.result_chain_memo.is_some();
        let mut taken = Vec::new();
        let mut end = None;
        let mut current = Some(start);
        while let Some(node) = current {
            if memo
                && node != start
                && let Some(anchor) = self.result_chain_taken(node, walk)
            {
                #[cfg(any(test, feature = "layout-verify"))]
                self.verify_result_chain_done(node, walk);
                end = anchor;
                break;
            }
            #[cfg(test)]
            {
                self.result_chain_steps += 1;
            }
            let boundary = self.is_fixed_metric_boundary(node);
            match walk {
                ResultChainWalk::Clear => {
                    self.layout_results.remove(&node);
                    self.layout_results_suppressed.remove(&node);
                    self.page_layout_results.get_mut().remove(&node);
                }
                ResultChainWalk::Suppress => self.suppress_layout_result(node),
            }
            if memo && node != start {
                taken.push(node);
            }
            let absorbs = boundary && !(node == start && start_changes_border);
            if absorbs {
                end = Some(node);
                break;
            }
            current = self.parent_id(node);
        }
        if let Some(memo) = self.result_chain_memo.as_mut() {
            let cleared = walk == ResultChainWalk::Clear;
            for node in taken {
                memo.insert(node, ResultChainStep { end, cleared });
            }
        }
    }

    /// Where the walk through `node` this commit already took ended, when
    /// it still covers what a `walk` reaching `node` from a child would do:
    /// `None` for the top of the tree, else the boundary it stopped at,
    /// which must still be one. A suppressing walk is covered by either
    /// kind (a cleared result has nothing left to suppress); a clearing one
    /// only by a clearing one.
    fn result_chain_taken(
        &self,
        node: StableNodeId,
        walk: ResultChainWalk,
    ) -> Option<Option<StableNodeId>> {
        let step = self.result_chain_memo.as_ref()?.get(&node)?;
        if walk == ResultChainWalk::Clear && !step.cleared {
            return None;
        }
        match step.end {
            Some(boundary) if !self.is_fixed_metric_boundary(boundary) => None,
            end => Some(end),
        }
    }

    /// Every test checks a walk the memo cut short against the one it
    /// skipped: from `node` on, that walk would have changed nothing.
    #[cfg(any(test, feature = "layout-verify"))]
    fn verify_result_chain_done(&self, node: StableNodeId, walk: ResultChainWalk) {
        let mut current = Some(node);
        while let Some(node) = current {
            let done = match walk {
                ResultChainWalk::Clear => {
                    !self.layout_results.contains_key(&node)
                        && !self.layout_results_suppressed.contains(&node)
                }
                ResultChainWalk::Suppress => {
                    !self.layout_results.contains_key(&node)
                        || self.layout_results_suppressed.contains(&node)
                }
            };
            assert!(
                done,
                "a result chain cut short at {node:?} left a stale result"
            );
            if self.is_fixed_metric_boundary(node) {
                break;
            }
            current = self.parent_id(node);
        }
    }

    /// Forget the chains this commit walked before `mutation` moves a node
    /// they pass through: a walk is remembered only while the parents along
    /// it stay. A node created or inserted fresh is in no remembered chain
    /// unless it is remembered itself; removals and moves forget them all.
    fn forget_result_chains_before(&mut self, mutation: &crate::UiMutation) {
        let Some(memo) = self.result_chain_memo.as_ref() else {
            return;
        };
        if memo.is_empty() {
            return;
        }
        let forget = match mutation {
            crate::UiMutation::Create { id, .. } => memo.contains_key(id),
            crate::UiMutation::Insert { child, .. } => {
                memo.contains_key(child) || self.parent_id(*child).is_some()
            }
            crate::UiMutation::Detach { .. }
            | crate::UiMutation::ParkSubtree { .. }
            | crate::UiMutation::DespawnSubtree { .. }
            | crate::UiMutation::ReorderChildren { .. } => true,
            _ => false,
        };
        if forget && let Some(memo) = self.result_chain_memo.as_mut() {
            memo.clear();
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
        if !definite_fixed_border(record.resolved_layout.as_ref())
            || self.atom_baseline_reads_content(id)
        {
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
        if self.layout_results.contains_key(&id) {
            self.layout_results_suppressed.insert(id);
        }
    }

    pub(crate) fn suppress_layout_results_subtree(&mut self, root: StableNodeId) {
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
        if self.layout_results.is_empty() {
            // Nothing published yet, so nothing to suppress up the chain.
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
        if ids.is_empty() && self.layout_anchor_roots.is_empty() {
            // Nothing to publish, nothing anchored: the walk below would
            // record no counts and mark nothing.
            return;
        }
        // A first publish builds every result, and each settles its pair in
        // the memo. A later one rebuilds few of the nodes it covers.
        let mut memo = crate::NodeMap::default();
        if self.layout_results.is_empty() {
            memo.reserve(ids.len());
        }
        *self.anchor_memo.get_mut() = Some(memo);
        self.publish_layout_results_anchored(ids, source);
        *self.anchor_memo.get_mut() = None;
    }

    fn publish_layout_results_anchored(
        &mut self,
        ids: &[StableNodeId],
        source: crate::LayoutResultSource,
    ) {
        // A child's box joins its parent's overflow. Include the ancestor
        // closure so a compatibility write cannot leave a parent's overflow
        // from an older child box. Siblings share their ancestors:
        // a walk stops at the first one already collected.
        let anchored = self.take_moved_layout_anchors();
        let ids: Vec<StableNodeId> = if anchored.is_empty() {
            ids.to_vec()
        } else {
            ids.iter().copied().chain(anchored).collect()
        };
        let ids = ids.as_slice();
        let mut seen: NodeSet = ids.iter().copied().collect();
        // The batch as it came, which a pass hands over sorted: the stable
        // sort below takes that run whole and merges the ancestors in,
        // where the set's order would be sorted from scratch.
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
        // Ancestors are republished with the batch. Downstream geometry is
        // scheduled only for a caller node whose own box, padding, fragment
        // or clip changed.
        let mut requested = ids.to_vec();
        requested.sort_unstable();
        requested.dedup();
        // The children this publish covers, by parent. Every other child
        // keeps the box its parent's held result recorded, so a parent is
        // checked against these and not against all of its children. One
        // list sorted by parent: a parent's children are one run of it.
        //
        // A first publish (no result held) has nothing to check them
        // against: every result is built, and only how many children it
        // covers is counted.
        let first_publish = self.layout_results.is_empty();
        let (covered, mut children_visited) = if first_publish {
            let count = unique
                .iter()
                .filter(|&&id| self.parent_id(id).is_some())
                .count();
            (Vec::new(), count)
        } else {
            let mut covered: Vec<(StableNodeId, StableNodeId)> = unique
                .iter()
                .filter_map(|&id| Some((self.parent_id(id)?, id)))
                .collect();
            covered.sort_unstable();
            (covered, 0usize)
        };
        let covered_ids: Vec<StableNodeId> = covered.iter().map(|&(_, id)| id).collect();
        #[cfg(feature = "benchmark")]
        let mut phase = crate::layout_engine::plan_stats::PhaseClock::start();
        let mut built = Vec::with_capacity(unique.len());
        let mut scratch = ResultScratch::default();
        let mut reused = 0usize;
        // Parents with a covered child measured from another frame, in
        // order. That child joins the parent's overflow only on the page,
        // where the placed result is kept by the local result's identity:
        // such a parent is published anew when its local result is the
        // same but the one placed on the page no longer is, and is
        // re-extracted with it.
        let mut escaped: Vec<StableNodeId> = Vec::new();
        for &id in &unique {
            if boxless.contains(&id) {
                self.layout_results.remove(&id);
                self.layout_results_suppressed.remove(&id);
                self.page_layout_results.get_mut().remove(&id);
                continue;
            }
            if first_publish {
                children_visited = children_visited.saturating_add(
                    self.nodes
                        .get(id)
                        .map_or(0, |node| node.hierarchy.children.len()),
                );
                if let Some(result) = self.build_layout_result(id, source, &mut scratch) {
                    built.push((id, result));
                }
                continue;
            }
            let start = covered.partition_point(|&(parent, _)| parent < id);
            let end = start + covered[start..].partition_point(|&(parent, _)| parent == id);
            let covered = &covered_ids[start..end];
            children_visited = children_visited.saturating_add(covered.len());
            if covered.iter().any(|&child| {
                self.nodes
                    .get(child)
                    .is_some_and(|record| record.layout_frame != crate::LayoutFrame::Parent)
            }) {
                escaped.push(id);
            }
            if self.layout_result_geometry_current(id, covered) {
                self.layout_results_suppressed.remove(&id);
                reused = reused.saturating_add(1);
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
            if self.layout_results.get(id).is_some_and(|previous| {
                previous.local.geometry_eq(&result.local)
                    && (escaped.binary_search(id).is_err()
                        || self.placed_layout_result_holds(*id, &previous.local))
            }) {
                self.nodes.note_text_fragments(*id);
                self.layout_results_suppressed.remove(id);
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
        // A first publish fills an empty table: size it once.
        if self.layout_results.is_empty() {
            self.layout_results.reserve(changed);
        }
        self.note_layout_result_publish(reused, changed, 1, children_visited);
        self.layout_generation = self.layout_generation.wrapping_add(1);
        let generation = self.layout_generation;
        // `published` and `requested` are both sorted: one cursor walks
        // the second as the first goes.
        let mut requested = requested.iter().peekable();
        for (id, mut retained) in published {
            let result = Arc::get_mut(&mut retained.local).expect("a result just built");
            while requested.next_if(|requested| **requested < id).is_some() {}
            let requested_here = requested.peek() == Some(&&id);
            // One lookup for the held result: read it, then replace it.
            let held = self.layout_results.entry(id);
            let previous = match &held {
                std::collections::hash_map::Entry::Occupied(held) => Some(&*held.get().local),
                std::collections::hash_map::Entry::Vacant(_) => None,
            };
            // The menu's growing label slot depends on chrome and its offered
            // box, not on label advance. A changed slot must reach text even
            // when the outer box stays fixed (for example a changed hint).
            let part_budget_changed = matches!(
                self.nodes.visual(id),
                Some(
                    StandardVisual::ActionMenuItem { .. }
                        | StandardVisual::ListItem { content: None, .. }
                        | StandardVisual::Card { .. }
                )
            ) && previous.is_none_or(|previous| {
                let width = |result: &crate::LayoutResult| {
                    result.parts.iter().find_map(|part| {
                        (part.kind == crate::LayoutPartKind::TextContent)
                            .then_some(part.bounds.width)
                    })
                };
                width(previous) != width(result)
            });
            let sticky_changed = self.sticky_nodes.has_dependents(id)
                && previous.is_none_or(|previous| {
                    (
                        previous.content_box,
                        previous.padding_box,
                        previous.scroll_extent,
                    ) != (result.content_box, result.padding_box, result.scroll_extent)
                });
            let project = requested_here
                && previous
                    .is_none_or(|previous| layout_result_projects_new_geometry(previous, result));
            let reshaped =
                project && previous.is_none_or(|previous| layout_result_reshapes(previous, result));
            result.generation = generation;
            result.dependency_generation = generation;
            held.insert_entry(retained);
            self.layout_results_suppressed.remove(&id);
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
                for sticky in self.sticky_nodes.affected(id) {
                    self.mark(
                        sticky,
                        DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
            }
        }
        #[cfg(feature = "benchmark")]
        phase.lap(9);
    }

    /// Mark `bits` on a box layout moved or resized, through
    /// [`Self::mark_moved`] when it only `moved`. Its INPUT mark is layout's
    /// alone (see [`SystemWork::accessibility_moved`]) unless something else
    /// marks INPUT on it too.
    fn mark_moved_by_layout(&mut self, id: StableNodeId, bits: u16, moved: bool) {
        self.marking_layout_move = true;
        if moved {
            self.mark_moved(id, bits);
        } else {
            self.mark(id, bits);
        }
        self.marking_layout_move = false;
    }

    /// [`Self::mark_scroll_compatible`] for `id`'s own scroll: its INPUT
    /// mark is the scroll's ([`SystemWork::accessibility_scrolled`]) unless
    /// something else marks INPUT on it too.
    pub(super) fn mark_scrolled(&mut self, id: StableNodeId) {
        self.marking_scroll = true;
        let _ = self.mark_scroll_compatible(id, DirtyMask::INPUT | DirtyMask::RENDER);
        self.marking_scroll = false;
    }

    /// Mark `id`, whose own 2D transform changed while what it hands its
    /// children did not ([`SystemWork::transform_kept`]). Its INPUT mark is
    /// no subtree seed for accessibility, as a layout move's is not, unless
    /// something else marks INPUT on it too.
    pub(super) fn mark_transform_kept(&mut self, id: StableNodeId) {
        self.marking_layout_move = true;
        self.mark(
            id,
            DirtyMask::TRANSFORM | DirtyMask::TRANSFORM_KEPT | DirtyMask::INPUT | DirtyMask::RENDER,
        );
        self.marking_layout_move = false;
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
    ) -> Option<RetainedLayoutResult> {
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
    ) -> Option<crate::LayoutResult> {
        scratch.clear();
        let ResultScratch {
            own_parts: parts,
            own_fragments: fragments,
            dependencies,
            ..
        } = scratch;
        let node = self.nodes.get(id)?;
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            ..node.local_layout
        };
        let padding = self.used_layout_padding(id);
        let border = node.resolved_layout.resolved_border_edges();
        let mut result = crate::LayoutResult::from_box(bounds, padding, border);
        let component_parts = self.solve_content_parts_at(id, result.content_box, bounds);
        let text_box =
            crate::layout_engine::parts::primary_text_box(&component_parts, result.content_box);
        result.source = source;
        result.scroll_offset = node.scroll_offset;
        result.formatting_context = established_context(node);
        result.participation = self.layout_node(id).and_then(|node| node.participation);
        let scroll_metrics = self.scroll_metrics(id);
        if record_has_text(node) && self.needs_native_text_fragments(id) {
            let (native, first, last) = self.native_text_fragments(id, text_box)?;
            fragments.extend(native);
            result.first_baseline = first;
            result.last_baseline = last;
            parts.push(crate::LayoutPart::new(
                crate::LayoutPartKind::TextContent,
                text_box,
            ));
        } else if matches!(node.kind.as_ref(), NodeKind::Text) || !node.text.value.is_empty() {
            let baseline = self.single_text_baseline(id, text_box);
            let source = self.text_layout(id).map(|_| 0..node.text.value.len());
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
        if node.resolved_layout.position == nana_ui_core::PositionSpec::Fixed {
            fragments.push(crate::LayoutFragment::new(
                crate::LayoutFragmentKind::Overlay,
                result.bounds,
            ));
            parts.push(crate::LayoutPart::new(
                crate::LayoutPartKind::Overlay,
                result.bounds,
            ));
        }
        if node.resolved_layout.clips_overflow() {
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
        parts.extend(
            component_parts
                .into_iter()
                .flatten()
                .filter(|part| part.kind != crate::LayoutPartKind::TextContent),
        );
        result.scroll_metrics = scroll_metrics;
        result.clip = anchors.clip;
        result.clip_parent = anchors.clip_parent;
        result.containing_block = anchors.containing_block;
        result.dependencies = shared_slice(dependencies);
        Some(result)
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
        mut result: crate::LayoutResult,
        scratch: &mut ResultScratch,
    ) -> Option<RetainedLayoutResult> {
        let ResultScratch {
            parts,
            own_fragments,
            own_parts,
            shared_parts,
            shared_fragments,
            ..
        } = scratch;
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
            if let Some(record) = self.nodes.get(child)
                && record.layout_frame == crate::LayoutFrame::Parent
                && (joins_every_child || self.clip_parent_of(child) == Some(id))
            {
                overflow = union_layout_boxes(overflow, record.local_layout);
            }
        }
        result.fragments = shared_fragments.share(own_fragments);
        let content = content_part_index(own_parts.as_slice());
        parts.extend(own_parts[..content].iter().copied());
        parts.push(crate::LayoutPart::new(
            crate::LayoutPartKind::ComponentContent,
            result.content_box,
        ));
        parts.extend(own_parts[content..].iter().copied());
        result.parts = shared_parts.share(parts);
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
        Some(RetainedLayoutResult {
            local: Arc::new(result),
        })
    }

    /// The result held for `id` still describes its geometry. `covered` are
    /// the children the current publish covers: every edit of a child list
    /// drops the parent's result, so a held result was built from this child
    /// list, and a child no publish has covered since sits where it
    /// recorded. Checking the covered children is checking them all, without
    /// a walk of a wide container's children for an edit in one of them.
    fn layout_result_geometry_current(&self, id: StableNodeId, covered: &[StableNodeId]) -> bool {
        if self.local_layout_result(id).is_some_and(|result| {
            result.participation != self.layout_node(id).and_then(|node| node.participation)
        }) {
            return false;
        }

        // Provenance is not geometry. A held result whose geometry still
        // holds is kept with its stamp, which is what a rebuild would end in
        // (`geometry_eq` ignores the source), so a different `source` alone
        // does not force one: the writeback commit publishes as a
        // compatibility write and the pass publishes again as layout, and
        // rebuilding every ancestor for each would copy a wide container's
        // placements twice a frame.
        let Some(held) = self.layout_results.get(&id) else {
            return false;
        };
        // Everything is compared where it was built, from the border box's
        // origin, and each child as its local box.
        let previous = &held.local;
        let Some(facts) = self.layout_result_facts(id) else {
            return false;
        };
        if facts.fixed || facts.clips || self.scroll_metrics(id).is_some() {
            return false;
        }
        let Some(local) = self.local_layout_box(id).map(|bounds| LayoutBox {
            x: 0.0,
            y: 0.0,
            ..bounds
        }) else {
            return false;
        };
        if previous.bounds != local
            || previous.border_box != local
            || previous.scroll_offset != facts.scroll_offset
        {
            return false;
        }
        let padding = self.used_layout_padding(id);
        let padding_box = crate::layout_result::inset(
            local,
            facts.border.top,
            facts.border.right,
            facts.border.bottom,
            facts.border.left,
        );
        let content_box = crate::layout_result::inset(
            padding_box,
            padding.top,
            padding.right,
            padding.bottom,
            padding.left,
        );
        if previous.padding_box != padding_box || previous.content_box != content_box {
            return false;
        }
        // The overflow is the box joined with the children measured from
        // it (this box neither clips nor scrolls, so it joins them all). A
        // held overflow that is the box itself had every such child inside
        // it; the children no publish covered still are, so the overflow
        // holds while each covered one is inside too. A held overflow past
        // the box is rebuilt: the child that made it may have moved back.
        if previous.overflow != local {
            return false;
        }
        for &child in covered {
            let Some(record) = self.nodes.get(child) else {
                return false;
            };
            if record.hierarchy.parent != Some(id) {
                return false;
            }
            // A child measured from another frame joins the overflow on the
            // page, where the placed result is kept by this result's
            // identity: a new one goes with every such child published.
            if record.layout_frame != crate::LayoutFrame::Parent
                || union_layout_boxes(local, record.local_layout) != local
            {
                return false;
            }
        }
        // Everything below is solved where a rebuild solves it, with the
        // border box at the origin.
        let component_parts = self.solve_content_parts_at(id, content_box, local);
        let text_box = crate::layout_engine::parts::primary_text_box(&component_parts, content_box);
        let placed_text = text_box;
        let extra = || {
            component_parts
                .iter()
                .flatten()
                .filter(|part| part.kind != crate::LayoutPartKind::TextContent)
                .copied()
        };
        if facts.text && self.needs_native_text_fragments(id) {
            let current = self
                .nodes
                .text_node(id)
                .is_some_and(|text| text.fragments_layout == text.layout);
            let content_current = previous
                .parts
                .iter()
                .find(|part| part.kind == crate::LayoutPartKind::TextContent)
                .is_some_and(|part| part.bounds == placed_text);
            let parts_current = previous
                .parts
                .iter()
                .filter(|part| {
                    matches!(
                        part.kind,
                        crate::LayoutPartKind::LeadingContent
                            | crate::LayoutPartKind::TrailingContent
                            | crate::LayoutPartKind::Hint
                            | crate::LayoutPartKind::Spinner
                            | crate::LayoutPartKind::Note
                            | crate::LayoutPartKind::Detail
                            | crate::LayoutPartKind::Label
                            | crate::LayoutPartKind::Value
                            | crate::LayoutPartKind::Header
                            | crate::LayoutPartKind::Indicator
                    )
                })
                .copied()
                .eq(extra());
            return current
                && content_current
                && parts_current
                && previous.formatting_context == facts.context
                && self.layout_anchors(id).recorded_in(previous);
        }
        let baseline = facts
            .text
            .then(|| self.single_text_baseline(id, text_box))
            .flatten();
        if facts.text {
            let source = self
                .text_layout(id)
                .and_then(|_| self.text(id))
                .map(|text| 0..text.len());
            if previous
                .fragments
                .get(..2)
                .is_none_or(|fragments| fragments.iter().any(|fragment| fragment.source != source))
            {
                return false;
            }
        }
        if previous.first_baseline != baseline || previous.last_baseline != baseline {
            return false;
        }
        if previous.formatting_context != facts.context {
            return false;
        }
        if !retained_projection_current(
            previous,
            content_box,
            placed_text,
            baseline,
            facts.text,
            extra,
        ) {
            return false;
        }
        // The dependencies are the children plus the two anchors: with the
        // box and the overflow where the result recorded them, only the
        // anchors are left to move.
        self.layout_anchors(id).recorded_in(previous)
    }

    fn layout_result_facts(&self, id: StableNodeId) -> Option<LayoutResultFacts> {
        let node = self.nodes.get(id)?;
        Some(LayoutResultFacts {
            fixed: node.resolved_layout.position == PositionSpec::Fixed,
            clips: node.resolved_layout.clips_overflow(),
            scroll_offset: node.scroll_offset,
            text: record_has_text(node),
            border: node.resolved_layout.resolved_border_edges(),
            context: established_context(node),
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
        if previous.display != next.display
            || previous.position != next.position
            || previous.float != next.float
            || previous.font_family != next.font_family
            || previous.font_weight != next.font_weight
            || previous.font_italic != next.font_italic
            || previous.font_variation_settings != next.font_variation_settings
        {
            self.strut_candidates.push(id);
            if let Some(parent) = self.parent_id(id) {
                self.strut_candidates.push(parent);
            }
        }
        // The clip chain reads which of absolute, fixed and in-flow a box
        // is, not only whether it is positioned (#201).
        if previous.position != next.position || previous.clips_overflow() != next.clips_overflow()
        {
            self.layout_anchor_roots.insert(id);
            // The scrollable range follows the same chain: the box may move
            // between its parent's area and its containing block's, and the
            // boxes it carries may land here or pass on.
            self.invalidate_scroll_content(id);
            if previous.position.establishes_containing_block()
                != next.position.establishes_containing_block()
            {
                self.scroll_content_bounds.borrow_mut().reland(self, id);
            }
            self.layout_reframe_roots.push(id);
            self.scroll_layout_touched = true;
        }
        // So does how it takes part in its parent's context: an inline-block
        // turned block-level is a flow item where its box stays put, and its
        // result, not republished for a box that did not move, would say it
        // is still atomic.
        if previous.display != next.display || previous.float != next.float {
            self.layout_anchor_roots.insert(id);
        }
    }

    /// The nodes under a noted anchor change whose published result records
    /// an anchor that is no longer the one above them.
    fn take_moved_layout_anchors(&mut self) -> Vec<StableNodeId> {
        if self.layout_anchor_roots.is_empty() {
            return Vec::new();
        }
        let mut roots: Vec<StableNodeId> = self.layout_anchor_roots.drain().collect();
        roots.sort_unstable();
        let mut moved = Vec::new();
        let mut stack = Vec::new();
        for root in roots {
            // The root itself: its own clip chain moves with its position,
            // and its own fragments say whether it is a scroll viewport or
            // an overlay.
            if self.layout_results.contains_key(&root) {
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
                if let Some(result) = self.layout_results.get(&id)
                    && !self.layout_anchors(id).recorded_in(&result.local)
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
        } = match self.anchor_memo.borrow_mut().as_mut() {
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
        !self.dirty_entities.is_empty()
            || !self.pending_accessibility_removals.is_empty()
            || !self.pending_render_removals.is_empty()
    }

    /// Monotonic revision of work scheduled since world creation. Reading it
    /// is O(1) and has no effect on the frame's pending queues.
    pub fn pending_work_revision(&self) -> u64 {
        self.pending_work_revision
    }

    /// Algorithm-level counters from the last non-empty drain, or the current
    /// frame accumulator while a product flush is running. An idle
    /// [`Self::take_system_work`] does not replace this snapshot.
    pub fn last_work_counters(&self) -> WorkCounters {
        let mut counters = self.counters.last_counters;
        counters.record_hot_path_allocation(
            self.pending_hot_allocations.get(),
            self.pending_hot_allocated_bytes.get(),
        );
        add_pending_counts(
            &mut counters,
            self.counters.pending_layout_invalidations_counted,
        );
        add_pending_counts(&mut counters, self.counters.pending_drain_counts);
        counters.scroll_content_extent_recomputes += self.pending_scroll_extent_recomputes();
        counters.i18n.accumulate(self.i18n_gauges());
        counters
    }

    pub fn record_presentation_gpu_work(&mut self, work: Option<nana_ui_core::GpuWorkObservation>) {
        let mut observed = WorkCounters::default();
        if let Some(work) = work {
            observed.record_gpu_work(work);
        }
        self.counters.last_counters.batch_rebuilds = observed.batch_rebuilds;
        self.counters.last_counters.draw_batches = observed.draw_batches;
        self.counters.last_counters.draw_calls = observed.draw_calls;
        self.counters.last_counters.gpu_upload_bytes = observed.gpu_upload_bytes;
        self.counters.last_counters.gpu_buffer_reallocations = observed.gpu_buffer_reallocations;
    }

    pub fn record_output_work(&mut self, work: nana_ui_core::OutputWorkObservation) {
        let mut observed = WorkCounters::default();
        observed.record_output_work(work);
        macro_rules! replace { ($($field:ident),* $(,)?) => { $(self.counters.last_counters.$field = observed.$field;)* }; }
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
    pub fn begin_frame_counters(&mut self) {
        self.begin_responsive_frame();
        self.commit_pending_hot_allocs();
        self.commit_pending_layout_invalidation_counts();
        self.commit_pending_theme_work();
        self.counters.frame_counters = WorkCounters::default();
        self.text_frame_work = nana_text::TextWorkCounters::default();
        self.theme_frame_work = ThemeWorkCounters::default();
        self.frame_extracted_nodes = 0;
        self.frame_extracted_spans = 0;
        self.accumulating_frame = true;
    }

    pub fn end_frame_counters(&mut self) {
        self.accumulating_frame = false;
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
        if self.accumulating_frame {
            self.frame_extracted_nodes = self.frame_extracted_nodes.saturating_add(nodes);
            self.frame_extracted_spans = self.frame_extracted_spans.saturating_add(spans);
            self.counters.last_counters.render_nodes_extracted = self.frame_extracted_nodes;
            self.counters.last_counters.extracted_text_spans = self.frame_extracted_spans;
        } else {
            self.counters.last_counters.render_nodes_extracted = nodes;
            self.counters.last_counters.extracted_text_spans = spans;
        }
    }

    /// Observe a CPU hot-path heap event from a `&self` path (layout inputs,
    /// document order). Folded into [`Self::last_work_counters`].
    pub fn record_hot_path_allocation(&self, count: usize, bytes: usize) {
        if count == 0 && bytes == 0 {
            return;
        }
        self.pending_hot_allocations
            .set(self.pending_hot_allocations.get().saturating_add(count));
        self.pending_hot_allocated_bytes
            .set(self.pending_hot_allocated_bytes.get().saturating_add(bytes));
    }

    fn commit_pending_hot_allocs(&mut self) {
        let count = self.pending_hot_allocations.replace(0);
        let bytes = self.pending_hot_allocated_bytes.replace(0);
        self.counters
            .last_counters
            .record_hot_path_allocation(count, bytes);
        if self.accumulating_frame {
            self.counters
                .frame_counters
                .record_hot_path_allocation(count, bytes);
        }
    }

    fn commit_pending_layout_invalidation_counts(&mut self) {
        let counted = std::mem::take(&mut self.counters.pending_layout_invalidations_counted);
        self.bump_last_counters(|counters| add_pending_counts(counters, counted));
    }

    /// Text work (Issue #95) of the last shaping pass, or of every pass of
    /// the last frame [`Self::begin_frame_counters`] accumulated that ran one: nodes
    /// considered, skipped on revision alone, shaped, and what the passes
    /// cloned, hashed and looked up to get there.
    pub fn last_text_work_counters(&self) -> nana_text::TextWorkCounters {
        self.text_work
    }

    /// Theme/style work (Issue #101 §4) of the last style pass or theme
    /// install, or of every pass of the last frame
    /// [`Self::begin_frame_counters`] accumulated that ran one: nodes
    /// considered, resolved and skipped, token reads, and what the pass cost
    /// Layout, Text and Paint.
    ///
    /// An idle frame leaves the previous snapshot in place, like
    /// [`Self::last_work_counters`].
    ///
    /// One theme change is more than one event — the install marks the world,
    /// the drain schedules it, the style pass resolves it — so measure it
    /// inside [`Self::begin_frame_counters`] / [`Self::end_frame_counters`].
    /// Outside that accumulator each event replaces the last, exactly as
    /// [`Self::last_text_work_counters`] does.
    pub fn last_theme_work_counters(&self) -> ThemeWorkCounters {
        let mut counters = self.theme_work;
        counters.accumulate(self.pending_theme_work());
        counters
    }

    /// The theme/style work observed since the last fold, as counters.
    fn pending_theme_work(&self) -> ThemeWorkCounters {
        let mut pending = ThemeWorkCounters::default();
        pending.record_theme_reads(self.pending_theme_reads.get());
        pending.record_layout_copy(
            self.pending_layout_copies.get(),
            self.pending_layout_copies
                .get()
                .saturating_mul(size_of::<nana_ui_core::LayoutStyle>()),
        );
        pending
    }

    /// Take that work, leaving nothing behind for the next window.
    fn take_pending_theme_work(&self) -> ThemeWorkCounters {
        let pending = self.pending_theme_work();
        self.pending_theme_reads.set(0);
        self.pending_layout_copies.set(0);
        pending
    }

    /// Observe one read of the token authority from a `&self` palette path.
    fn record_theme_read(&self) {
        self.pending_theme_reads
            .set(self.pending_theme_reads.get().saturating_add(1));
    }

    /// Observe the `LayoutStyle` a style write had to copy to hold the
    /// resolved value beside the authored intent. It is the largest heap
    /// event on the style path and would otherwise go unwatched: it happens
    /// under `Arc::make_mut`, which no allocator counter sees.
    fn record_resolved_layout_copy(&self) {
        self.pending_layout_copies
            .set(self.pending_layout_copies.get().saturating_add(1));
    }

    /// Close the pending work onto the window it was observed in, so an event
    /// from before [`Self::begin_frame_counters`] is not charged to the frame
    /// that follows it.
    fn commit_pending_theme_work(&mut self) {
        let pending = self.take_pending_theme_work();
        if pending == ThemeWorkCounters::default() {
            return;
        }
        self.theme_work.accumulate(pending);
        if self.accumulating_frame {
            self.theme_frame_work.accumulate(pending);
        }
    }

    fn record_theme_work(&mut self, mut work: ThemeWorkCounters) {
        work.accumulate(self.take_pending_theme_work());
        if self.accumulating_frame {
            self.theme_frame_work.accumulate(work);
            self.theme_work = self.theme_frame_work;
        } else {
            self.theme_work = work;
        }
    }

    fn record_text_work(&mut self, mut work: nana_text::TextWorkCounters) {
        work.accumulate(std::mem::take(&mut self.pending_edit_work));
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
        if self.accumulating_frame {
            // Published as the frame goes, so an idle frame (no text pass)
            // leaves the last frame that had one in place, as
            // `last_work_counters` does.
            self.text_frame_work.accumulate(work);
            self.text_work = self.text_frame_work;
        } else {
            self.text_work = work;
        }
    }

    fn bump_last_counters(&mut self, update: impl Fn(&mut WorkCounters)) {
        update(&mut self.counters.last_counters);
        if self.accumulating_frame {
            update(&mut self.counters.frame_counters);
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
        self.style_model.theme_appearance
    }

    pub fn theme_metrics(&self) -> nana_ui_core::ThemeMetrics {
        self.style_model.metrics
    }

    pub fn style_model(&self) -> StyleModelRef {
        self.style_model
    }

    /// The installed design system.
    ///
    /// Returned by reference: a [`CompiledTheme`](nana_ui_core::CompiledTheme)
    /// is around a kilobyte, and handing one out by value on a read path is
    /// the mistake Issue #101 §1.5 measured with `LayoutStyle`. Callers that
    /// only want a colour or a metric take [`Self::style_model`] instead.
    pub fn theme(&self) -> &nana_ui_core::CompiledTheme {
        &self.theme
    }

    /// The design system as installed: [`Self::theme`] without the system
    /// high-contrast overlay.
    pub fn installed_theme(&self) -> &nana_ui_core::CompiledTheme {
        &self.installed_theme
    }

    /// Shared installed theme for reactive consumers deriving geometry from it.
    pub fn installed_theme_arc(&self) -> Arc<nana_ui_core::CompiledTheme> {
        Arc::clone(&self.installed_theme)
    }

    /// Whether the system high-contrast overlay is on.
    pub fn high_contrast(&self) -> bool {
        self.high_contrast
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
        self.counters
            .pending_drain_counts
            .scroll_content_extent_recomputes += self.take_scroll_extent_recomputes();
        let mut ids = std::mem::take(&mut self.dirty_entities)
            .into_iter()
            .collect::<Vec<_>>();
        ids.sort_unstable();
        let dirty_len = ids.len();
        let mut work = SystemWork {
            generation: self.generation,
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
                &mut self.pending_accessibility_removals,
            )),
            render_extraction: Vec::new(),
            render_removals: sorted_unique(std::mem::take(&mut self.pending_render_removals)),
            entities_total: self.nodes.len(),
            entities_changed: 0,
            entities_spawned: std::mem::take(&mut self.spawned_since_drain),
            entities_despawned: std::mem::take(&mut self.despawned_since_drain),
            input_targets: self.live_input_target_count(),
            render_nodes_changed: 0,
            render_nodes_extracted: 0,
            extracted_text_spans: 0,
            allocations: 0,
            allocated_bytes: 0,
            text_shaped_runs: 0,
            text_layout_cache_hits: 0,
            text_layout_cache_misses: 0,
            text_wrap_layouts: 0,
            intrinsic_measure_requests: 0,
            intrinsic_measure_cache_hits: 0,
            intrinsic_measure_cache_misses: 0,
            intrinsic_measure_full_subtrees: 0,
            intrinsic_generation_bumps: 0,
            baseline_queries: 0,
            cross_context_measure_hits: 0,
            cross_context_measure_misses: 0,
            glyph_cache_hits: None,
            glyph_cache_misses: None,
            cache_eviction: None,
            // A drain always follows the commits it drains, so 0 here is an
            // observed "validated nothing", not a missing measurement.
            validation_nodes_scanned: Some(std::mem::take(&mut self.validation_nodes_scanned)),
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
                && let Some(invalidation) = self.pending_layout_invalidations.remove(&id)
            {
                work.layout_frontier_seeds
                    .push(crate::LayoutFrontierSeed::new(id, invalidation));
            }
            // An editor bears text with no value yet: its caret line and an
            // IME preedit are shaped from its presentation. Text that went
            // empty is still shaped once, to nothing: the metrics it held
            // sized its box, and a box laid out cold without text has none.
            let has_text = matches!(self.record(id).kind.as_ref(), NodeKind::Text)
                || !self.record(id).text.value.is_empty()
                || self.record(id).text_metrics != TextMetrics::default()
                || matches!(
                    self.nodes.visual(id),
                    Some(StandardVisual::TextInput { .. })
                        | Some(StandardVisual::EmptyState { .. })
                        | Some(StandardVisual::ModalFrame { .. })
                );
            let bits = if has_text {
                bits
            } else {
                // A block with no text of its own still holds a strut when
                // its lines hold atoms: its font changed, or it is new.
                if bits & DirtyMask::TEXT != 0
                    && (!self.record(id).hierarchy.children.is_empty()
                        || self.nodes.block_strut(id).is_some())
                {
                    self.strut_candidates.push(id);
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
        self.pending_layout_invalidations.extend(theirs);
        let mut remaining = pending
            .into_iter()
            .filter(|(id, _)| self.presence_live(*id))
            .map(|(id, invalidation)| crate::LayoutFrontierSeed::new(id, invalidation))
            .collect::<Vec<_>>();
        remaining.compact_sort_by_key(|seed| seed.node);
        work.layout_frontier_seeds.extend(remaining);
        work.render_nodes_changed = work.render_extraction.len();
        work.render_nodes_extracted = work.render_extraction.len();
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
            self.pending_hot_allocations.set(0);
            self.pending_hot_allocated_bytes.set(0);
            let mut counters = work.counters();
            let counted = std::mem::take(&mut self.counters.pending_layout_invalidations_counted);
            add_pending_counts(&mut counters, counted);
            add_pending_counts(
                &mut counters,
                std::mem::take(&mut self.counters.pending_drain_counts),
            );
            counters.i18n.accumulate(self.i18n_gauges());
            // Extracted node/span fields are filled by [`Self::record_extract`]
            // on the product path, not by the planned render list.
            counters.render_nodes_extracted = 0;
            counters.extracted_text_spans = 0;
            if self.accumulating_frame {
                self.counters.frame_counters.accumulate(counters);
                self.counters.last_counters = self.counters.frame_counters;
                self.counters.last_counters.render_nodes_extracted = self.frame_extracted_nodes;
                self.counters.last_counters.extracted_text_spans = self.frame_extracted_spans;
            } else {
                self.counters.last_counters = counters;
            }
        } else {
            self.commit_pending_hot_allocs();
            self.commit_pending_layout_invalidation_counts();
            let pending = std::mem::take(&mut self.counters.pending_drain_counts);
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
                let epoch = self.render_epoch.get();
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
                    self.dirty_entities.insert(id);
                    self.pending_work_revision = self.pending_work_revision.saturating_add(1);
                }
            }
        }
        self.pending_accessibility_removals
            .extend(work.accessibility_removals);
        self.pending_accessibility_removals.sort_unstable();
        self.pending_accessibility_removals.dedup();
        self.pending_render_removals.extend(work.render_removals);
        self.pending_render_removals.sort_unstable();
        self.pending_render_removals.dedup();
        self.spawned_since_drain = self
            .spawned_since_drain
            .saturating_add(work.entities_spawned);
        self.despawned_since_drain = self
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
        match modality {
            InputModality::Pointer => self.input.pointer_modality.insert(document),
            InputModality::Keyboard => self.input.pointer_modality.remove(&document),
        };
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
        self.nodes.text_input(id).is_some() && self.record(id).accessibility.editable
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
            .get(id)
            .map(|n| &n.text)
            .map(|text| text.value.as_str())
    }

    pub fn document_text_selection(
        &self,
        document: DocumentId,
    ) -> Option<&crate::DocumentTextSelection> {
        self.document_text_selections.get(&document)
    }

    pub fn set_document_text_selection(
        &mut self,
        document: DocumentId,
        selection: Option<crate::DocumentTextSelection>,
    ) {
        if self.document_text_selections.get(&document) == selection.as_ref() {
            return;
        }
        if let Some(previous) = self.document_text_selections.remove(&document)
            && self.contains(previous.node)
        {
            let _ = self.mark(previous.node, DirtyMask::RENDER);
        }
        if let Some(selection) = selection {
            if self.contains(selection.node) {
                let _ = self.mark(selection.node, DirtyMask::RENDER);
            }
            self.document_text_selections.insert(document, selection);
        }
    }

    pub(crate) fn drop_invalid_document_text_selections(&mut self) {
        let stale: Vec<DocumentId> = self
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
        let local = self.local_layout_result(id)?;
        let [x, y] = self.layout_origin(id)?;
        Some(LayoutBox {
            x,
            y,
            ..local.bounds
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
            .map(|record| self.record_layout_box(record))
    }

    /// `record`'s box on the page; see [`Self::layout_origin`].
    pub(super) fn record_layout_box(&self, record: &NodeRecord) -> LayoutBox {
        let [x, y] = self.record_layout_origin(record);
        LayoutBox {
            x,
            y,
            ..record.local_layout
        }
    }

    /// The box layout last wrote for `id`, measured from the origin of its
    /// [`Self::layout_frame`]: what stays put when a box above it moves.
    /// [`Self::layout_box`] is the frame's document origin plus this.
    pub fn local_layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        self.nodes.get(id).map(|node| node.local_layout)
    }

    /// What `id`'s [`Self::local_layout_box`] is measured from.
    pub fn layout_frame(&self, id: StableNodeId) -> Option<crate::LayoutFrame> {
        self.nodes.get(id).map(|node| node.layout_frame)
    }

    /// The node whose border-box origin `record`'s local box is measured
    /// from; `None` for the page's.
    fn frame_node(&self, record: &NodeRecord) -> Option<StableNodeId> {
        match record.layout_frame {
            crate::LayoutFrame::Parent => record.hierarchy.parent,
            crate::LayoutFrame::Block(block) => Some(block),
            crate::LayoutFrame::Page => None,
        }
    }

    /// Where `id`'s border box stands on the page: its local origin and its
    /// frames' summed from the top down, one axis at a time, so a box under
    /// one that moved is the sum a fresh layout gives. O(depth).
    pub(crate) fn layout_origin(&self, id: StableNodeId) -> Option<[f32; 2]> {
        self.nodes
            .get(id)
            .map(|record| self.record_layout_origin(record))
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

    pub(super) fn record_layout_origin(&self, record: &NodeRecord) -> [f32; 2] {
        if let Some(origin) = self.remembered_page_origin(record) {
            #[cfg(any(test, feature = "layout-verify"))]
            {
                #[cfg(test)]
                let walked = page_origins::steps::uncounted(|| self.walked_layout_origin(record));
                #[cfg(not(test))]
                let walked = self.walked_layout_origin(record);
                assert!(
                    origin[0].to_bits() == walked[0].to_bits()
                        && origin[1].to_bits() == walked[1].to_bits(),
                    "a remembered page origin {origin:?} is the walked one {walked:?}"
                );
            }
            return origin;
        }
        self.walked_layout_origin(record)
    }

    /// [`Self::record_layout_origin`], every frame walked.
    fn walked_layout_origin(&self, record: &NodeRecord) -> [f32; 2] {
        // Deriving a component's geometry in its own coordinates: a chain
        // through that node starts at its origin.
        let frame = self
            .geometry_frame
            .get()
            .and_then(|frame| self.nodes.get(frame));
        // Most chains are shallow: their locals stay on the stack.
        const INLINE: usize = 48;
        let mut inline = [[0.0f32; 2]; INLINE];
        let mut spilled: Vec<[f32; 2]> = Vec::new();
        let mut len = 0usize;
        let mut cursor = Some(record);
        let mut reached = frame.is_none();
        while let Some(node) = cursor {
            #[cfg(test)]
            page_origins::steps::step(page_origins::steps::Walk::PageOrigin);
            if frame.is_some_and(|frame| std::ptr::eq(frame, node)) {
                reached = true;
                break;
            }
            if matches!(node.kind.as_ref(), NodeKind::Document) {
                break;
            }
            let local = [node.local_layout.x, node.local_layout.y];
            if len < INLINE {
                inline[len] = local;
            } else {
                if spilled.is_empty() {
                    spilled.extend_from_slice(&inline);
                }
                spilled.push(local);
            }
            len += 1;
            cursor = self
                .frame_node(node)
                .and_then(|frame| self.nodes.get(frame));
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
        self.geometry_read_page.set(true);
        let frame = frame.expect("unreached only under a frame");
        let previous = self.geometry_frame.replace(None);
        let [fx, fy] = self.record_layout_origin(frame);
        self.geometry_frame.set(previous);
        [origin[0] - fx, origin[1] - fy]
    }

    /// `id`'s component geometry in its own coordinates: every rectangle
    /// measured from its border-box origin, so a node that only moved has
    /// the geometry it had. The page geometry ([`Self::component_geometry`])
    /// is this placed at the node's page origin, up to rounding. A visual
    /// whose geometry reads the page ([`StandardVisual::geometry_reads_page`])
    /// comes back on the page instead.
    pub(super) fn local_component_geometry(
        &self,
        id: StableNodeId,
        visual: &StandardVisual,
        style: &ComputedStyle,
    ) -> Option<crate::ComponentGeometry> {
        if visual.geometry_reads_page() {
            return self.page_component_geometry(id, visual, style);
        }
        let previous = self.geometry_frame.replace(Some(id));
        let read_before = self.geometry_read_page.replace(false);
        let geometry = self.derive_component_geometry(id, visual, style);
        let read_page = self.geometry_read_page.replace(read_before);
        self.geometry_frame.set(previous);
        debug_assert!(
            !read_page,
            "{id:?}: {visual:?} derives its geometry from the page; list it in \
             `StandardVisual::geometry_reads_page`"
        );
        geometry
    }

    /// [`Self::derive_component_geometry`] on the page, whatever frame
    /// extraction is deriving in.
    fn page_component_geometry(
        &self,
        id: StableNodeId,
        visual: &StandardVisual,
        style: &ComputedStyle,
    ) -> Option<crate::ComponentGeometry> {
        let previous = self.geometry_frame.replace(None);
        if previous.is_some() {
            self.geometry_read_page.set(true);
        }
        let geometry = self.derive_component_geometry(id, visual, style);
        self.geometry_frame.set(previous);
        geometry
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
        let node = self.nodes.get(id)?;
        if let Some(offset) = self.rich_editor_scroll_offset(id) {
            return Some(offset);
        }
        if node.accessibility.multiline
            && let Some(frame) = self.editor_frame(id)
        {
            return Some(frame.scroll_offset());
        }
        Some(node.scroll_offset)
    }

    /// The scrolling area a scroll offset is clamped to. A multiline text
    /// editor's follows its shaped value, so it is always current; every
    /// other container's is the one last published for it.
    /// The offset last set on `id`, which a multiline editor is drawn from.
    pub(crate) fn scroll_request(&self, id: StableNodeId) -> Option<ScrollOffset> {
        self.nodes.get(id).map(|node| node.scroll_offset)
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
            || (self.nodes.has_visuals()
                && matches!(
                    self.nodes.visual(id),
                    Some(StandardVisual::Scrollbar { .. })
                ))
    }

    /// The axes (`[horizontal, vertical]`) on which `id` keeps what
    /// overflows inside it out of the scrolling area of a scroll container
    /// above it, which then counts only `id`'s own box on that axis. CSS
    /// scrollable overflow stops at the nearest clipping box the same way: a
    /// scroll container keeps both axes (CSS makes it clip both), another box
    /// the axis `overflow: hidden` clips.
    pub fn overflow_contained_axes(&self, id: StableNodeId) -> [bool; 2] {
        self.nodes.get(id).map_or([false; 2], |record| {
            overflow_contained_axes(&record.style.layout, self.nodes.visual(id))
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
        Some(self.responsive.authored(id).unwrap_or(&node.style))
    }

    /// [`Self::node_style`] of a live node.
    pub(super) fn authored_style(&self, id: StableNodeId) -> &NodeStyle {
        self.responsive
            .authored(id)
            .unwrap_or(&self.record(id).style)
    }

    /// `id`'s resolved style. A palette-only theme switch re-resolves no
    /// style, so its paint colours can be the previous palette's until the
    /// node restyles; [`Self::component_geometry`] and extraction read them
    /// in the installed palette.
    pub fn computed_style(&self, id: StableNodeId) -> Option<&ComputedStyle> {
        self.nodes.get(id).map(|node| node.resolved.0.as_ref())
    }

    /// Whether the text backend changed since the last call, and clears it.
    pub(crate) fn take_text_backend_changed(&mut self) -> bool {
        std::mem::take(&mut self.text_backend_changed)
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
        self.presenters.contains_key(name)
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
        if self.presenters.contains_key(name) {
            return Err(UiWorldError::DuplicatePresenter(name.to_owned()));
        }
        let name = name.to_owned();
        self.presenters.insert(name.clone(), presenter);
        let ids = self.nodes.keys().collect::<Vec<_>>();
        for id in ids {
            if self
                .nodes
                .highlight(id)
                .is_some_and(|request| request.presenter.as_ref() == name)
            {
                self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
        }
        Ok(())
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
            .unwrap_or_else(|| self.record(id).text.value.clone())
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
        self.nodes.get(id).map(|node| node.text_metrics)
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
        self.nodes_by_component
            .get(component)
            .into_iter()
            .flatten()
            .copied()
            .filter(move |id| self.document_of(*id) == Some(document))
    }

    /// Keep [`Self::nodes_by_component`] in step with a node's component type.
    fn reindex_component(&mut self, id: StableNodeId, next: Option<&ComponentTypeId>) {
        if let Some(previous) = self.nodes.component_type(id).cloned()
            && let Some(nodes) = self.nodes_by_component.get_mut(&previous)
        {
            nodes.remove(&id);
            if nodes.is_empty() {
                self.nodes_by_component.remove(&previous);
            }
        }
        if let Some(next) = next {
            self.nodes_by_component
                .entry(next.clone())
                .or_default()
                .insert(id);
        }
    }

    fn note_structural_change(&mut self, parent: StableNodeId) {
        if self.structural_change_parents.last() != Some(&parent) {
            self.structural_change_parents.push(parent);
        }
        // Whether it has inline-level children, and so a strut, may have
        // changed with them.
        self.strut_candidates.push(parent);
    }

    /// Parents whose child list changed since the last drain, or one of
    /// whose children started or stopped generating a box (`hidden`,
    /// `display: none`), deduplicated in ascending order. Consumers must
    /// drain per commit so scheduled reprojections observe the post-mutation
    /// tree.
    pub fn take_structural_change_parents(&mut self) -> Vec<StableNodeId> {
        let mut parents = std::mem::take(&mut self.structural_change_parents);
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

    pub fn standard_visual(&self, id: StableNodeId) -> Option<StandardVisual> {
        self.nodes.visual(id).cloned()
    }

    /// [`Self::standard_visual`] without the copy, for projections that
    /// compare before they write.
    pub(crate) fn standard_visual_ref(&self, id: StableNodeId) -> Option<&StandardVisual> {
        self.nodes.visual(id)
    }

    /// The chrome `id` paints, its text colours in the installed palette as
    /// extraction paints them (#398).
    pub fn component_geometry(&self, id: StableNodeId) -> Option<crate::ComponentGeometry> {
        let visual = self.nodes.visual(id)?;
        let style = self.painted_style(id)?;
        self.page_component_geometry(id, visual, &style)
    }

    pub(crate) fn component_content_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        if let Some(result) = self.local_layout_result(id) {
            return Some(crate::layout_result::place_box(
                self.layout_origin(id)?,
                result.content_box,
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

    pub fn accessibility(&self, id: StableNodeId) -> Option<&AccessibilityState> {
        self.nodes.get(id).map(|node| node.accessibility.as_ref())
    }

    /// The node whose text names `id` when it has no label of its own; see
    /// [`MutationQueue::set_labelled_by`].
    pub fn labelled_by(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.labelled_by.get(&id).copied()
    }

    /// Relate `id` to the node that names it, or drop the relation.
    fn set_labelled_by(&mut self, id: StableNodeId, label: Option<StableNodeId>) {
        if let Some(previous) = self.labelled_by.remove(&id)
            && let Some(controls) = self.labels.get_mut(&previous)
        {
            controls.remove(&id);
            if controls.is_empty() {
                self.labels.remove(&previous);
            }
        }
        if let Some(label) = label {
            self.labelled_by.insert(id, label);
            self.labels.entry(label).or_default().insert(id);
        }
    }

    /// Drop every naming relation `id` is an end of: it is gone.
    fn forget_labelled_by(&mut self, id: StableNodeId) {
        self.set_labelled_by(id, None);
        if let Some(controls) = self.labels.remove(&id) {
            for control in controls {
                self.labelled_by.remove(&control);
                if self.nodes.contains(control) {
                    self.mark(control, DirtyMask::ACCESSIBILITY);
                }
            }
        }
    }

    /// How changes inside `id` are announced, when a composite made it a
    /// live region; see [`MutationQueue::set_live`].
    pub fn live(&self, id: StableNodeId) -> Option<AccessibilityLive> {
        self.live_regions.get(&id).copied()
    }

    /// Make `id` a live region, or stop it being one.
    fn set_live(&mut self, id: StableNodeId, live: Option<AccessibilityLive>) {
        match live {
            Some(live) => {
                self.live_regions.insert(id, live);
            }
            None => {
                self.live_regions.remove(&id);
            }
        }
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
        self.overlay_hosts_by_document
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
        self.nodes.has_visuals()
            && matches!(
                self.nodes.visual(id),
                Some(StandardVisual::ModalFrame { .. })
            )
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
        if !self.nodes.has_visuals() {
            return None;
        }
        let StandardVisual::ModalFrame { kind, slots, .. } = self.nodes.visual(id)? else {
            return None;
        };
        let presentation = self.nodes.modal_text(id).copied().unwrap_or_default();
        Some(crate::ModalLayoutInput {
            kind: *kind,
            slots: slots.clone(),
            title: presentation.title,
            description: presentation.description,
            body_text: presentation.body,
        })
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
        let plain_world = self.layout_length_tracks.is_empty()
            && self.overlay_host_nodes.is_empty()
            && self.z_index_nodes == 0
            && self.detached_mounted.is_empty()
            && !self.nodes.has_visuals();
        let record = self.nodes.get(id).ok_or(UiWorldError::MissingNode(id))?;
        let has_text =
            matches!(record.kind.as_ref(), NodeKind::Text) || !record.text.value.is_empty();
        // Header block space is already represented by Card's title band.
        // Its text still contributes preferred inline width, not body height.
        let text_metrics = has_text.then_some(record.text_metrics).map(|mut metrics| {
            if matches!(
                self.standard_visual_ref(id),
                Some(StandardVisual::Card { .. })
            ) {
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
        self.pass_publishes_written_boxes = deferred;
    }

    pub(crate) fn pass_publishes_written_boxes(&self) -> bool {
        self.pass_publishes_written_boxes
    }

    pub(crate) fn write_layout_padding(
        &mut self,
        id: StableNodeId,
        padding: nana_ui_core::PaddingSpec,
    ) -> bool {
        let record = self.record_mut(id);
        let changed = record.layout_padding != Some(padding);
        record.layout_padding = Some(padding);
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
        let record = self.record_mut(id);
        let changed = record.layout_far_start != Some(far);
        record.layout_far_start = Some(far);
        changed
    }

    pub(crate) fn layout_far_start(&self, id: StableNodeId) -> Option<[bool; 2]> {
        self.nodes.get(id)?.layout_far_start
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
        record.layout_context_generation = record.layout_context_generation.wrapping_add(1);
        if previous.is_some() && context.is_some() {
            self.bump_last_counters(|counters| counters.record_layout_context_transitions(1));
        }
        previous.is_some()
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
            context_generation: node.layout_context_generation,
            result_generation: self.local_layout_result(id).map(|result| result.generation),
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
        let record = self.record(id);
        record.layout_padding.unwrap_or_else(|| {
            // Intent (radius / control height / padding) lives on
            // `resolved_layout`. Falling back to the authored box would
            // report 0 for every control that named its inset instead of
            // spending the token.
            record
                .resolved_layout
                .resolved_padding_against(Some(record.local_layout.width))
        })
    }

    /// Layout-facing style without assembling a [`LayoutInput`].
    ///
    /// Parked, detached, and inactive-overlay nodes match [`Self::layout_inputs`]:
    /// the returned style reports [`nana_ui_core::LayoutStyle::omits_box`].
    pub(crate) fn layout_style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        self.contains(id).then(|| self.effective_layout_style(id))
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
        (self.overlay_host_nodes.is_empty() || self.overlay_host(parent).is_none())
            && !matches!(
                self.nodes.visual(parent),
                Some(StandardVisual::MenuSurface { .. })
            )
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
                Arc::make_mut(&mut style).gap = Some(LengthSpec::Px(crate::popover::MENU_ITEM_GAP));
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
        let Some(StandardVisual::ListItem {
            leading: Some(leading),
            content,
            ..
        }) = self.nodes.visual(id)
        else {
            return false;
        };
        let no_box = |slot: StableNodeId| !self.contains(slot) || self.omits_layout_box(slot);
        no_box(*leading) && content.is_none_or(no_box)
    }

    /// Whether `id`, a child of an open action menu, is a plain column that
    /// holds menu items (the container of an `each` or `when`) rather than
    /// an item of its own.
    fn groups_action_menu_items(
        &self,
        id: StableNodeId,
        style: &nana_ui_core::LayoutStyle,
    ) -> bool {
        self.nodes.visual(id).is_none()
            && matches!(style.direction, Some(nana_ui_core::FlexDirection::Column))
            && self.record(id).hierarchy.parent.is_some_and(|parent| {
                matches!(
                    self.nodes.visual(parent),
                    Some(StandardVisual::MenuSurface {
                        kind: crate::MenuSurfaceKind::ActionMenu,
                        open: true,
                        ..
                    })
                )
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
        if self.z_index_nodes == 0 {
            return;
        }
        if let Some(overlay) = self.parent_triggered_overlay(id) {
            apply_triggered_overlay(Arc::make_mut(layout), overlay);
        }
    }

    fn parent_triggered_overlay(&self, id: StableNodeId) -> Option<crate::TriggeredMenuOverlay> {
        let parent = self.record(id).hierarchy.parent?;
        match self.nodes.visual(parent)? {
            StandardVisual::MenuSurface {
                open: true,
                overlay: Some(overlay),
                ..
            } if overlay.trigger_content != Some(id) => Some(*overlay),
            _ => None,
        }
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
        if self.record(id).text.value.is_empty() {
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
            rich.is_some_and(|rich| self.record(id).text.value.as_str() != rich.text());
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
            self.record_mut(id).text =
                crate::TextContent::new(std::sync::Arc::clone(rich.shared_text()));
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
        self.is_mounted(id) && (self.detached_mounted.is_empty() || self.presence_live_walk(id))
    }

    fn presence_live_memo(&self, id: StableNodeId, memo: &mut AncestorMemo) -> bool {
        if !self.is_mounted(id) {
            return false;
        }
        // Every detached root is parked, and a mounted node cannot sit under
        // a parked one: nothing to walk.
        if self.detached_mounted.is_empty() {
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
            if self.detached.contains(&node) {
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
            if self.detached.contains(&node) {
                return false;
            }
            current = self.parent_id(node);
        }
        true
    }

    fn presence_flags_of(&self, id: StableNodeId) -> PresenceFlags {
        let visual = self.nodes.visual(id);
        let record = self.nodes.get(id);
        debug_assert!(
            record.is_none_or(|record| record.layout_depends_on_viewport
                == record.resolved_layout.depends_on_viewport()),
            "{id:?}'s cached viewport dependency went stale"
        );
        PresenceFlags {
            confirm: is_confirm_modal(visual),
            clip: is_clip_visual(visual),
            z_index: record.is_some_and(|record| record.resolved_layout.z_index.is_some())
                || is_triggered_menu_overlay(visual),
            triggered: is_triggered_menu_overlay(visual),
            select_menu: matches!(visual, Some(StandardVisual::Select { opened: true, .. })),
            viewport: record.is_some_and(|record| record.layout_depends_on_viewport),
        }
    }

    fn apply_presence_flags(&mut self, id: Option<StableNodeId>, next: PresenceFlags) {
        let previous = id
            .and_then(|id| self.presence_flags.get(&id).copied())
            .unwrap_or(PresenceFlags::NONE);
        if previous == next {
            return;
        }
        self.note_presence_counts(previous.confirm, next.confirm, previous.clip, next.clip);
        self.note_z_index_presence(previous.z_index, next.z_index);
        bump_presence(
            &mut self.viewport_basis_nodes,
            previous.viewport,
            next.viewport,
        );
        if let Some(id) = id
            && let Some(document) = self.document_of(id)
        {
            if next.viewport {
                self.viewport_basis.entry(document).or_default().insert(id);
            } else if let Some(ids) = self.viewport_basis.get_mut(&document) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.viewport_basis.remove(&document);
                }
            }
        }
        if let Some(id) = id
            && previous.triggered != next.triggered
        {
            if next.triggered {
                self.triggered_overlays.insert(id);
            } else {
                self.triggered_overlays.remove(&id);
            }
        }
        if let Some(id) = id
            && previous.select_menu != next.select_menu
        {
            if next.select_menu {
                self.select_menus.insert(id);
            } else {
                self.select_menus.remove(&id);
            }
        }
        if let Some(id) = id {
            if next == PresenceFlags::NONE {
                self.presence_flags.remove(&id);
            } else {
                self.presence_flags.insert(id, next);
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
        self.viewport_basis_nodes != 0
    }

    pub fn viewport_basis_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.viewport_basis
            .values()
            .flat_map(|ids| ids.iter().copied())
    }

    /// Mounted viewport-dependent nodes in one document, without scanning other documents.
    /// Declares that `id` accepts drops, replacing any previous declaration.
    pub(crate) fn set_drop_target(&mut self, id: StableNodeId, accepts: nana_ui_core::DropAccepts) {
        self.drop_targets.insert(id, accepts);
    }

    /// Stops `id` accepting drops. Returns whether it was a target.
    pub(crate) fn clear_drop_target(&mut self, id: StableNodeId) -> bool {
        self.drop_targets.remove(&id).is_some()
    }

    /// Every node currently registered as a drop target.
    pub(crate) fn drop_target_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.drop_targets.keys().copied()
    }

    pub(crate) fn drop_target(&self, id: StableNodeId) -> Option<&nana_ui_core::DropAccepts> {
        self.drop_targets.get(&id)
    }

    pub(crate) fn drop_hover(&self) -> Option<(StableNodeId, nana_ui_core::DropEffect)> {
        self.drop_hover
    }

    pub(crate) fn set_drop_hover(
        &mut self,
        hover: Option<(StableNodeId, nana_ui_core::DropEffect)>,
    ) -> bool {
        if self.drop_hover == hover {
            return false;
        }
        if let Some((id, _)) = self.drop_hover {
            self.mark(id, DirtyMask::RENDER);
        }
        if let Some((id, _)) = hover {
            self.mark(id, DirtyMask::RENDER);
        }
        self.drop_hover = hover;
        true
    }

    /// Records the viewport a document was laid out against.
    pub(crate) fn set_document_viewport(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) {
        self.document_viewports.insert(document, viewport);
    }

    /// Viewport `document` was last laid out against.
    pub(crate) fn document_viewport(&self, document: DocumentId) -> Option<crate::LayoutViewport> {
        self.document_viewports.get(&document).copied()
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
            LayoutFieldMask::SIZING,
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
        self.document_viewports.remove(&document);
    }

    /// Viewport of the document owning `id`, if it has been laid out.
    pub(crate) fn document_viewport_of(&self, id: StableNodeId) -> Option<crate::LayoutViewport> {
        if self.geometry_frame.get().is_some() {
            self.geometry_read_page.set(true);
        }
        let document = self.node(id)?.document;
        self.document_viewports.get(&document).copied()
    }

    pub fn viewport_basis_ids_for(
        &self,
        document: DocumentId,
    ) -> impl Iterator<Item = StableNodeId> + '_ {
        self.viewport_basis
            .get(&document)
            .into_iter()
            .flat_map(|ids| ids.iter().copied())
    }

    pub(crate) fn document_of(&self, id: StableNodeId) -> Option<DocumentId> {
        self.nodes.get(id).map(|node| node.document)
    }

    fn note_z_index_presence(&mut self, was_present: bool, now_present: bool) {
        bump_presence(&mut self.z_index_nodes, was_present, now_present);
    }

    fn note_presence_counts(
        &mut self,
        was_confirm: bool,
        now_confirm: bool,
        was_clip: bool,
        now_clip: bool,
    ) {
        bump_presence(&mut self.confirm_modals, was_confirm, now_confirm);
        bump_presence(&mut self.clip_visuals, was_clip, now_clip);
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

    fn confirm_action_effect(&self, id: StableNodeId) -> Option<(bool, bool, bool)> {
        if self.confirm_modals == 0 {
            return None;
        }
        let mut current = self.parent_id(id);
        while let Some(ancestor) = current {
            if let Some(StandardVisual::ModalFrame {
                kind: crate::ModalSurfaceKind::Confirm(_),
                busy,
                danger,
                slots,
                ..
            }) = self.nodes.visual(ancestor)
            {
                let close = slots
                    .close_action
                    .is_some_and(|root| self.is_descendant_or_self(id, root));
                let action = slots
                    .actions
                    .iter()
                    .copied()
                    .find(|root| self.is_descendant_or_self(id, *root));
                if close || action.is_some() {
                    return Some((*busy, *danger, action == slots.actions.last().copied()));
                }
            }
            current = self.parent_id(ancestor);
        }
        None
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
        if !self.is_mounted(target) {
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        if !node.interaction.pointer_events || !self.used_pointer_events(target).hittable() {
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        if !node.resolved.0.pointer_events.hittable() {
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        Ok(())
    }

    fn used_pointer_events(&self, id: StableNodeId) -> PointerEventsSpec {
        if self.record(id).hierarchy.depth > page_origins::DIRECT_DEPTH
            && let Ok(mut memo) = self.pointer_events_memo.try_borrow_mut()
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
            self.generation = self.generation.wrapping_add(1);
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
        self.pending_edit_work.composition_updates += 1;
        self.nodes
            .invalidate_text(id, crate::text_node::TextDirty::EDIT_STATE);
        self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
    }

    fn clear_overlay_references(&mut self, removed: StableNodeId) {
        self.clear_overlay_references_for(&[removed]);
    }

    /// Drop references through the reverse index; unrelated hosts are untouched.
    fn clear_overlay_references_for(&mut self, removed: &[StableNodeId]) {
        if self.overlay_host_nodes.is_empty() || removed.is_empty() {
            return;
        }
        let removed = removed.iter().copied().collect::<HashSet<_>>();
        let hosts = removed
            .iter()
            .filter_map(|id| self.overlay_dependents.get(id))
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
                && self.record(restore_focus).interaction.focusable
                && self.record(restore_focus).resolved.0.visible
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
        if self.overlay_host_nodes.is_empty() {
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
        if !self.nodes.has_visuals() {
            return true;
        }
        let Some(parent) = self.record(id).hierarchy.parent else {
            return true;
        };
        if !self.nodes.contains(parent) {
            return true;
        };
        !closed_menu_hides(self.nodes.visual(parent), id)
    }

    /// Whether a live retained root currently belongs to this document.
    /// Host service requests use this as the document lifetime authority.
    pub fn has_document(&self, document: DocumentId) -> bool {
        self.live_document_roots
            .get(&document)
            .is_some_and(|roots| !roots.is_empty())
    }

    pub(crate) fn has_document_roots(&self, document: DocumentId) -> bool {
        self.has_document(document)
    }

    pub(crate) fn document_roots(&self, document: DocumentId) -> Vec<StableNodeId> {
        let mut roots = self
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
            self.live_document_roots
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
        let Some(roots) = self.live_document_roots.get_mut(&document) else {
            return false;
        };
        let removed = roots.remove(&id);
        if roots.is_empty() {
            self.live_document_roots.remove(&document);
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
                let has_definite_size =
                    definite_fixed_border(parent_record.resolved_layout.as_ref())
                        && !self.atom_baseline_reads_content(parent);
                // A fixed inline-block is an atomic box. Inner metrics
                // stop at its border; the outer inline formatting
                // context does not reflow.
                let atomic_border = has_definite_size
                    && parent_display == Some(nana_ui_core::DisplaySpec::InlineBlock);
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
                if !upward_for_parent.is_empty() {
                    let follows_metrics = force_all || axes.intersects(upward_for_parent);
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
                        let is_icon =
                            matches!(self.nodes.visual(child), Some(StandardVisual::Icon { .. }));
                        let Some(child_record) = self.nodes.get(child) else {
                            continue;
                        };
                        let has_wrapping_text = child_record.resolved_layout.text_wraps()
                            && (matches!(child_record.kind.as_ref(), NodeKind::Text)
                                || !child_record.text.value.is_empty());
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
        if let Some(later) = &mut self.renumber_later {
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
        for (parent, (head, tail)) in self.renumber_later.take().into_iter().flatten() {
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
            self.child_list_steps += written.len() as u64;
            self.children_numbered += written.len() as u64;
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
        if !self.unlinked_ahead.is_empty() && self.unlinked_ahead.remove(&child) {
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
            self.child_list_steps += after as u64;
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
        if bits & DirtyMask::INPUT != 0 {
            self.non_scroll_hit_dirty.insert(id);
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
                    LayoutFieldMask::ALL,
                    LayoutDependencyFootprint::ALL,
                ),
            );
        }
        self.mark_scroll_compatible(id, bits)
    }

    /// Record dirtiness without claiming that hit membership or geometry changed.
    /// Layout dirtiness retires full-layout snapshots: text shaping and other
    /// post-layout writers reach layout only through a cause, not a commit.
    fn mark_scroll_compatible(&mut self, id: StableNodeId, bits: u16) -> bool {
        if bits & DirtyMask::RENDER != 0 {
            self.record_mut(id).render_full_epoch = self.render_epoch.get();
        }
        self.mark_keeping_render_reason(id, bits)
    }

    /// [`Self::mark`] for a box layout only moved: its render mark does not
    /// say the rest of its extracted record changed. What does reuse that
    /// record is still decided by `render_moved_epoch`, which only a result
    /// placed again from its own geometry sets.
    fn mark_moved(&mut self, id: StableNodeId, bits: u16) -> bool {
        debug_assert_eq!(bits & DirtyMask::LAYOUT, 0, "a move is not a layout cause");
        if bits & DirtyMask::INPUT != 0 {
            self.non_scroll_hit_dirty.insert(id);
        }
        self.mark_keeping_render_reason(id, bits)
    }

    fn mark_keeping_render_reason(&mut self, id: StableNodeId, bits: u16) -> bool {
        if bits & DirtyMask::LAYOUT != 0 {
            self.note_layout_source_change();
        }
        let bits = if bits & DirtyMask::INPUT == 0 || self.marking_layout_move {
            bits
        } else if self.marking_scroll {
            // Marked for its scroll.
            bits | DirtyMask::INPUT_SCROLLED
        } else {
            // Marked for more than a move or a scroll.
            bits | DirtyMask::INPUT_ELSEWHERE
        };
        let changed = self.record_mut(id).dirty.insert(bits);
        if changed {
            self.dirty_entities.insert(id);
            self.pending_work_revision = self.pending_work_revision.saturating_add(1);
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
    /// [`crate::store::Hierarchy::depth`]. Walks the subtree on the heap.
    pub(super) fn set_subtree_depth(&mut self, root: StableNodeId, depth: u16) {
        let hierarchy = self.hierarchy_mut(root);
        if hierarchy.children.is_empty() {
            hierarchy.depth = depth;
            return;
        }
        let mut open = vec![(root, depth)];
        while let Some((id, depth)) = open.pop() {
            let hierarchy = self.hierarchy_mut(id);
            if hierarchy.depth == depth && id != root {
                // Moved with the rest of its subtree before: already right.
                continue;
            }
            hierarchy.depth = depth;
            let below = depth.saturating_add(1);
            open.extend(hierarchy.children.iter().map(|&child| (child, below)));
        }
    }

    /// Levels from the root down to `id`, counting it.
    pub(super) fn node_depth(&self, id: StableNodeId) -> Option<usize> {
        let depth = usize::from(self.nodes.get(id)?.hierarchy.depth);
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
        self.detached.insert(root);
        if self.mount_state(root) == Some(MountState::Parked) {
            self.detached_mounted.remove(&root);
        } else {
            self.detached_mounted.insert(root);
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
            if let Some(index) = self.hit_test_index.get_mut(&document) {
                retain_hit_tree(index, id);
            }
            self.pending_render_removals.push(id);
            self.pending_accessibility_removals.push(id);
        }

        // Pointer state and animations to release: none in a page being
        // built, whose every node parks once on creation.
        if !(self.input.pointer_captures.is_empty()
            && self.input.pointer_hover.is_empty()
            && self.input.pointer_press.is_empty()
            && self.animations.is_empty())
        {
            self.release_pointers_and_animations_of(subtree);
        }

        for &id in subtree {
            self.surface_motion.remove(&id);
            self.closing_surfaces.remove(&id);
            self.scrim_presence.remove(&id);
            self.hover_transitions.remove(&id);
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

    fn positioned_out_of_flow(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|record| record.resolved_layout.position.is_out_of_flow())
    }

    pub(crate) fn layout_isolated(&self, id: StableNodeId) -> bool {
        self.node_style(id).is_some_and(|node| {
            let style=&node.layout;
            style.layout_isolation
                && matches!(style.width, Some(nana_ui_core::LengthSpec::Px(value)) if value.is_finite() && value>=0.0)
                && matches!(style.height, Some(nana_ui_core::LengthSpec::Px(value)) if value.is_finite() && value>=0.0)
                && style.position==nana_ui_core::PositionSpec::Static
                && style.float==nana_ui_core::FloatSpec::None
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
                self.non_scroll_hit_dirty.insert(id);
            }
            if !self.mark_scroll_compatible(id, bits) {
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
        let atomic = display == Some(nana_ui_core::DisplaySpec::InlineBlock);
        if fixed
            && (atomic
                || !display
                    .is_some_and(|display| display.is_inline_level() || display.is_contents()))
        {
            let (kind, fields) = if atomic {
                (InvalidationKind::PLACEMENT, LayoutFieldMask::TYPOGRAPHY)
            } else {
                (
                    InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                    LayoutFieldMask::TYPOGRAPHY.union(LayoutFieldMask::INTRINSIC),
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
            self.mark_scroll_compatible(id, DirtyMask::RENDER);
            self.bump_last_counters(|counters| counters.record_text_metrics(1, 0, 0));
            return false;
        }
        self.record_layout_invalidation(
            id,
            LayoutInvalidation::new(
                LayoutInvalidationSource::Text,
                InvalidationReason::TEXT,
                InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                LayoutFieldMask::TYPOGRAPHY.union(LayoutFieldMask::INTRINSIC),
                METRIC_EXPORTS,
            ),
        );
        // Keep the original paint invalidation for the node whose shaped
        // metrics changed; the typed layout cause above remains narrow.
        self.mark_scroll_compatible(id, DirtyMask::RENDER);
        let mut parent_reflows = 0;
        if let Some(parent) = self.parent_id(id) {
            if self.is_fixed_metric_boundary(parent) {
                self.record_layout_invalidation(
                    parent,
                    LayoutInvalidation::new(
                        LayoutInvalidationSource::Text,
                        InvalidationReason::TEXT,
                        InvalidationKind::PLACEMENT,
                        LayoutFieldMask::FLOW,
                        LayoutDependencyFootprint::NONE,
                    ),
                );
                self.mark_scroll_compatible(parent, DirtyMask::RENDER);
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
                        LayoutFieldMask::INTRINSIC.union(LayoutFieldMask::FLOW),
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

/// The ancestors the result chains of one commit have walked through, each
/// with where its walk ended (see [`UiWorld::walk_layout_result_chain`]).
///
/// Inserting a subtree into a laid-out page walks from every parent that
/// gains a child to the nearest fixed boundary: each node inserted under a
/// deep one paid the whole depth again. A later walk of the same commit can
/// stop where an earlier one passed, because
///
/// - results only leave the two tables while a commit applies (publish runs
///   after its last mutation), so what a walk did above that node stays done;
/// - the walk from there follows the same parents, which nothing moves while
///   the entry stands ([`UiWorld::forget_result_chains_before`]), and a node
///   on it that became a boundary since only stops it sooner;
/// - where the walk stopped at a boundary, the boundary is checked again: one
///   that no longer is would let the walk run further, so the entry is not
///   used.
///
/// The node a walk starts from is not entered: every node created starts
/// one, and inserting a remembered node forgets them all.
type ResultChainMemo = crate::NodeMap<ResultChainStep>;

#[derive(Clone, Copy)]
struct ResultChainStep {
    /// The boundary the walk stopped at, `None` at the top of the tree.
    end: Option<StableNodeId>,
    /// The walk cleared results; a suppressing one only hid them.
    cleared: bool,
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
    clip: bool,
    z_index: bool,
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
        clip: false,
        z_index: false,
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

fn is_confirm_modal(visual: Option<&StandardVisual>) -> bool {
    matches!(
        visual,
        Some(StandardVisual::ModalFrame {
            kind: crate::ModalSurfaceKind::Confirm(_),
            ..
        })
    )
}

fn is_clip_visual(visual: Option<&StandardVisual>) -> bool {
    matches!(
        visual,
        Some(StandardVisual::EmptyState { .. } | StandardVisual::ModalFrame { .. })
    )
}

fn apply_triggered_overlay(
    layout: &mut nana_ui_core::LayoutStyle,
    overlay: crate::TriggeredMenuOverlay,
) {
    layout.position = PositionSpec::Fixed;
    layout.width = Some(LengthSpec::Px(
        (overlay.width - overlay.padding * 2.0).max(0.0),
    ));
    layout.z_index = Some(crate::popover::MENU_OVERLAY_Z_INDEX);
}

fn is_triggered_menu_overlay(visual: Option<&StandardVisual>) -> bool {
    matches!(
        visual,
        Some(StandardVisual::MenuSurface {
            open: true,
            overlay: Some(_),
            ..
        })
    )
}

fn text_intrinsic_changed(previous: TextMetrics, next: TextMetrics) -> bool {
    // Baseline is an exported metric consumed by baseline-aligned parents.
    // Width/height-only comparison lets a baseline-only shape update leave a
    // retained placement plan stale even though the child still needs to
    // participate in the parent's formatting-context solve.
    previous.width != next.width || previous.height != next.height || previous.ascent != next.ascent
}

fn intersect_layout_boxes(left: LayoutBox, right: LayoutBox) -> Option<LayoutBox> {
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
    fn recorded_in(self, result: &crate::LayoutResult) -> bool {
        result.clip == self.clip
            && result.clip_parent == self.clip_parent
            && result.containing_block == self.containing_block
    }
}

struct LayoutResultFacts {
    fixed: bool,
    clips: bool,
    scroll_offset: ScrollOffset,
    text: bool,
    border: nana_ui_core::PaddingSpec,
    context: Option<crate::FormattingContextKind>,
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
    matches!(node.kind.as_ref(), NodeKind::Text) || !node.text.value.is_empty()
}

/// The held result's fragments and parts are still what the current content
/// box and baseline make, placed where the box stands.
fn retained_projection_current<I: Iterator<Item = crate::LayoutPart>>(
    previous: &crate::LayoutResult,
    content_box: LayoutBox,
    text_box: LayoutBox,
    baseline: Option<f32>,
    text: bool,
    extra_parts: impl Fn() -> I,
) -> bool {
    let fragments = previous.fragments.as_ref();
    let parts = previous.parts.as_ref();
    let text_fragments = usize::from(text) * 2;
    if fragments.len() != text_fragments
        || parts.len() != 1 + usize::from(text) + extra_parts().count()
    {
        return false;
    }
    let mut index = 0;
    if text {
        let line = &fragments[index];
        let run = &fragments[index + 1];
        let text_part = &parts[index];
        if line.kind != crate::LayoutFragmentKind::TextLine
            || run.kind != crate::LayoutFragmentKind::TextRun
            || line.node.is_some()
            || run.node.is_some()
            || line.index != 0
            || run.index != 0
            || line.bounds != text_box
            || run.bounds != text_box
            || line.first_baseline != baseline
            || line.last_baseline != baseline
            || run.first_baseline != baseline
            || run.last_baseline != baseline
            || text_part.kind != crate::LayoutPartKind::TextContent
            || text_part.node.is_some()
            || text_part.bounds != text_box
        {
            return false;
        }
        index += 1;
    }
    let tail = &parts[index];
    tail.kind == crate::LayoutPartKind::ComponentContent
        && tail.node.is_none()
        && tail.bounds == content_box
        && parts[index + 1..].iter().copied().eq(extra_parts())
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
}

/// Equal but for transforms and the cursor, given whether the presentations
/// are equal (`node_presentation_eq`) and what changed in layout.
fn style_excluding_transform_and_cursor_eq(
    presentation_eq: bool,
    changed: nana_ui_core::LayoutStyleChange,
) -> bool {
    use nana_ui_core::LayoutStyleChange as Change;
    presentation_eq
        && changed
            .difference(Change::TRANSFORM.union(Change::CURSOR))
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
                    .union(Change::CURSOR),
            )
            .is_empty()
}

/// Own box, padding, fragment, and clip. Child placements stay on the children,
/// so a parent republished only for those must not patch hit testing from the root.
fn layout_result_projects_new_geometry(
    previous: &crate::LayoutResult,
    next: &crate::LayoutResult,
) -> bool {
    // `geometry_eq` with the containing block and dependencies left out, field by field so a box that moved answers
    // before any list is compared.
    let own_changed = previous.bounds != next.bounds
        || previous.border_box != next.border_box
        || previous.padding_box != next.padding_box
        || previous.content_box != next.content_box
        || previous.first_baseline != next.first_baseline
        || previous.last_baseline != next.last_baseline
        || previous.overflow != next.overflow
        || previous.scroll_extent != next.scroll_extent
        || previous.scroll_metrics != next.scroll_metrics
        || previous.clip != next.clip
        || previous.clip_parent != next.clip_parent
        || previous.formatting_context != next.formatting_context
        || previous.participation != next.participation;
    // A result holds nothing of where its children sit: a child whose box
    // moved publishes and projects itself, and its parent is not marked.
    own_changed || previous.fragments != next.fragments || previous.parts != next.parts
}

/// Whether `next` changes more of [`layout_result_projects_new_geometry`]
/// than the scroll extent, scroll metrics, overflow and baselines: the box,
/// its inner boxes, its clip links, its context or its own fragments and
/// parts. A result that changes only those leaves what the box clips cut by
/// the same box.
fn layout_result_reshapes(previous: &crate::LayoutResult, next: &crate::LayoutResult) -> bool {
    previous.bounds != next.bounds
        || previous.border_box != next.border_box
        || previous.padding_box != next.padding_box
        || previous.content_box != next.content_box
        || previous.clip != next.clip
        || previous.clip_parent != next.clip_parent
        || previous.formatting_context != next.formatting_context
        || previous.participation != next.participation
        || previous.fragments != next.fragments
        || previous.parts != next.parts
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
    let border = changed.intersects(Change::BORDER_GEOMETRY.union(Change::BORDER_STYLE));

    let mut fields = LayoutFieldMask::NONE;
    if flow {
        fields = fields.union(LayoutFieldMask::FLOW);
    }
    if sizing {
        fields = fields.union(LayoutFieldMask::SIZING);
    }
    if spacing {
        fields = fields.union(LayoutFieldMask::SPACING);
    }
    if position {
        fields = fields.union(LayoutFieldMask::POSITION);
    }
    if alignment {
        fields = fields.union(LayoutFieldMask::ALIGNMENT);
    }
    if grid {
        fields = fields.union(LayoutFieldMask::GRID);
    }
    if typography || writing {
        fields = fields.union(LayoutFieldMask::TYPOGRAPHY);
    }
    if scroll {
        fields = fields.union(LayoutFieldMask::SCROLL);
    }
    if border || fields == LayoutFieldMask::NONE {
        fields = LayoutFieldMask::ALL;
    }

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
mod float_containment;
#[cfg(test)]
mod issue197;
#[cfg(test)]
mod issue199;
#[cfg(test)]
mod issue200;
#[cfg(test)]
mod issue201;
#[cfg(test)]
mod issue202;
#[cfg(test)]
mod issue204;
#[cfg(test)]
mod issue205;
#[cfg(test)]
mod issue211;
#[cfg(test)]
mod issue212;
#[cfg(test)]
mod issue213;
#[cfg(test)]
mod issue214;
mod issue255;
mod issue256;
#[cfg(test)]
mod issue257;
mod issue259;
mod issue260;
mod issue261;
mod issue262;
mod issue263;
mod issue264;
mod issue265;
mod issue266;
mod issue268;
mod issue269;
mod issue270;
mod issue333;
mod issue381;
mod issue382;
mod issue384;
mod issue385;
mod issue386;
mod issue387;
mod issue388;
mod issue389;
mod issue392;
mod issue394;
mod issue396;
mod issue397;
mod issue398;
mod line_box_memo;
mod list_item_tooltip;
mod mount_under_narrow_row;
mod pending_causes;
#[cfg(test)]
mod placement_commit;
mod reflow_oracle;
mod result_chain_memo;
#[cfg(test)]
mod tests;
