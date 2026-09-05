mod accessibility;
mod animation;
mod extraction;
mod geometry;
mod hit_test;
mod input;
mod motion;
mod mutation;
mod style;
mod text;
use hit_test::*;
use text::*;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::mem::size_of;
use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{
    ControlSize, LayoutStyle, LengthSpec, PointerEventsSpec, PositionSpec, SemanticColorRole,
    SemanticPalette, StyleModelRef, SwitchControlPosition, ThemeMode, icon_y_on_text_glyph_center,
};

#[cfg(feature = "calendar")]
use nana_ui_core::TooltipConfig;
#[cfg(feature = "graph-canvas")]
use nana_ui_core::{GraphPoint, GraphPortKind, GraphPortSide, GraphRect, GraphSize, cubic_point};

use crate::animation::ActiveAnimation;
use crate::components::{
    EmptyStateTextPresentation, ModalTextPresentation, TextColorSwatchSpan,
    TextEditorRenderOptions, TextGitGutterMark, TextGitMark, TextGitMarkKind,
    TextInputPresentation, TextMatchMark, TextMatchMarker, TextMatchSpan, TextOverlayMetrics,
    TextSwatchMark, TextWhitespaceKind, TextWhitespaceMark,
};
use crate::schedule::{DirtyMask, SystemWork, push_work};
use crate::store::{Hierarchy, NodeRecord, NodeStore, ResolvedStyle, intern_empty_children};
use crate::text_editing::clamp_boundary;
use crate::{
    AccessibilityDelta, AccessibilityNode, AccessibilityRole, AccessibilityState, AnimationFrame,
    AnimationId, AnimationSpec, ComponentTypeId, ComputedStyle, CustomRenderNode, EventListeners,
    EventRoute, ExtractedNode, ExtractedTextSpan, HighlightRequest, ImeComposition,
    InteractionState, LayoutBox, LayoutInput, MountState, MutationQueue, NodeStyle,
    OverlayHostState, PointerCaptureChange, ScrollMetrics, ScrollOffset, StandardVisual,
    TextContent, TextInputState, TextMetrics, TextPresentation, TextPresenter, TextShaper,
    TextVerticalAlignment, UiMutation, WorkCounters,
};
/// Stable external node identity. Zero is reserved so missing/default IDs
/// cannot accidentally address a live node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableNodeId(u64);

impl StableNodeId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Deepest retained tree the frame pipeline accepts.
///
/// Style resolution walks ancestors, layout and hit-test walk descendants, and
/// paint walks the scene: all recursive. Tree shape comes from application or JS
/// input and is not trustworthy, so the bound is enforced once where the tree is
/// written. Real UIs nest one to two orders of magnitude below this.
pub const MAX_TREE_DEPTH: usize = 512;

/// Retired node IDs, stored as coalesced inclusive runs.
///
/// Retirement is permanent: a stale handle must never alias a node created
/// later. Both ID allocators ([`crate::AppContext`] and the Vue tree) are
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(u64);

impl DocumentId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
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

#[derive(Debug, Clone)]
struct HitEntry {
    id: StableNodeId,
    layout: LayoutBox,
    transform: [f32; 6],
    persp: [f32; 2],
    /// Clips applied to this node's own hit (and therefore its subtree).
    self_clips: Vec<(LayoutBox, [f32; 6])>,
    /// Extra clips applied to descendants only (overflow / visual frames).
    child_clips: Vec<(LayoutBox, [f32; 6])>,
    z_index: i32,
    order: usize,
    hittable: bool,
    menu: Option<LayoutBox>,
    children: Vec<HitEntry>,
}

/// Per-pass cache of ancestor-chain answers. Extraction and hit-index share
/// most of each chain, so one walk fills every node on it.
#[derive(Default)]
struct AncestorMemo {
    live: HashMap<StableNodeId, bool>,
    stacking: HashMap<StableNodeId, i32>,
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
    InvalidStyle(StableNodeId),
    InvalidText(StableNodeId),
    InvalidLayout(StableNodeId),
    InvalidScrollOffset(StableNodeId),
    InvalidScrollMetrics(StableNodeId),
    InvalidIme(StableNodeId),
    InvalidCustomRender(StableNodeId),
    InvalidEventListener(StableNodeId),
    InvalidStandardVisual(StableNodeId),
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
            Self::InvalidStyle(id) => write!(formatter, "node {} has an invalid style", id.get()),
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

/// The sole authoritative retained identity and hierarchy store.
pub struct UiWorld {
    input: input::WorldInputState,
    nodes: NodeStore,
    retired: RetiredIds,
    dirty_entities: HashSet<StableNodeId>,
    hit_test_index: HashMap<DocumentId, Vec<HitEntry>>,
    /// Scroll deltas awaiting the in-place hit-index patch (see
    /// `UiMutation::SetScrollOffset`). Drained by the frame driver.
    scroll_hit_updates: Vec<(StableNodeId, [f32; 2])>,
    pending_render_removals: Vec<StableNodeId>,
    pending_accessibility_removals: Vec<StableNodeId>,
    animations: HashMap<AnimationId, ActiveAnimation>,
    pub(crate) animation_now: Duration,
    surface_motion: HashMap<StableNodeId, motion::SurfaceMotion>,
    closing_surfaces: HashSet<StableNodeId>,
    switch_transitions: HashMap<StableNodeId, f32>,
    hover_transitions: HashMap<StableNodeId, style::HoverTransition>,
    animation_deadlines: BTreeSet<(Duration, AnimationId)>,
    style_model: StyleModelRef,
    generation: u64,
    presenters: HashMap<String, Box<dyn TextPresenter>>,
    spawned_since_drain: usize,
    despawned_since_drain: usize,
    last_counters: WorkCounters,
    frame_counters: WorkCounters,
    frame_extracted_nodes: usize,
    frame_extracted_spans: usize,
    accumulating_frame: bool,
    /// Layout/document-order allocs recorded from `&self` hot paths.
    pending_hot_allocations: Cell<usize>,
    pending_hot_allocated_bytes: Cell<usize>,
    text_layout_cache: crate::text_layout_cache::TextLayoutCache,
    glyph_cache: crate::GlyphCache,
    /// Live Confirm modal frames. Extract, a11y, and hit-test skip ancestor
    /// confirm walks when this is zero.
    confirm_modals: usize,
    /// Live EmptyState + ModalFrame visuals that clip descendants.
    clip_visuals: usize,
    /// Nodes with an authored `z-index`. Stacking walks skip when this is zero.
    z_index_nodes: usize,
    /// Live nodes whose box resolves against the viewport (`position: fixed`,
    /// `vw` / `vh`). A resize dirties this set together with document roots
    /// instead of discarding the retained layout cache.
    viewport_basis_nodes: usize,
    viewport_basis: HashSet<StableNodeId>,
    /// Last applied presence flags per entity, so park/remove/despawn can
    /// decrement without double-counting.
    presence_flags: HashMap<StableNodeId, PresenceFlags>,
    /// Subtree roots detached by Remove or Park. Mounted document/scene roots
    /// are created with no parent and are not in this set.
    detached: HashSet<StableNodeId>,
    /// Live roots per document: `parent.is_none()` and [`Self::presence_live`].
    live_document_roots: HashMap<DocumentId, BTreeSet<StableNodeId>>,
    /// Nodes carrying an `OverlayHostState` component. Overlay bookkeeping walks
    /// this index instead of every entity, so clearing references from a removed
    /// node costs the host count rather than the world size.
    overlay_host_nodes: HashSet<StableNodeId>,
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
    /// minimap 行长单条缓存（原始值 → 每逻辑行非空白字符数）。存于
    /// `RefCell` 供 `&self` 的 presentation 构建路径读写。
    minimap_line_lengths_cache: RefCell<Option<(String, Vec<u32>)>>,
    /// 括号配对着色单条缓存（原始值 → 配对/未配对 span 表）。值未变
    /// （纯光标/选区同步）时复用上一次 O(n) 单趟栈扫描结果。存于
    /// `RefCell` 供 `&self` 的 presentation 构建路径读写。
    bracket_color_spans_cache: RefCell<Option<(String, Arc<[(usize, usize, usize)]>)>>,
}

impl Default for UiWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl UiWorld {
    pub fn new() -> Self {
        Self {
            input: input::WorldInputState::default(),
            nodes: NodeStore::new(),
            retired: RetiredIds::default(),
            dirty_entities: HashSet::new(),
            hit_test_index: HashMap::new(),
            scroll_hit_updates: Vec::new(),
            pending_render_removals: Vec::new(),
            pending_accessibility_removals: Vec::new(),
            animations: HashMap::new(),
            animation_now: Duration::ZERO,
            surface_motion: HashMap::new(),
            closing_surfaces: HashSet::new(),
            switch_transitions: HashMap::new(),
            hover_transitions: HashMap::new(),
            animation_deadlines: BTreeSet::new(),
            style_model: StyleModelRef::default(),
            generation: 0,
            presenters: HashMap::new(),
            spawned_since_drain: 0,
            despawned_since_drain: 0,
            last_counters: WorkCounters::default(),
            frame_counters: WorkCounters::default(),
            frame_extracted_nodes: 0,
            frame_extracted_spans: 0,
            accumulating_frame: false,
            pending_hot_allocations: Cell::new(0),
            pending_hot_allocated_bytes: Cell::new(0),
            text_layout_cache: crate::text_layout_cache::TextLayoutCache::default(),
            glyph_cache: crate::GlyphCache::default(),
            confirm_modals: 0,
            clip_visuals: 0,
            z_index_nodes: 0,
            viewport_basis_nodes: 0,
            viewport_basis: HashSet::new(),
            presence_flags: HashMap::new(),
            detached: HashSet::new(),
            live_document_roots: HashMap::new(),
            overlay_host_nodes: HashSet::new(),
            validation_nodes_scanned: 0,
            palette_epoch: 1,
            structural_change_parents: Vec::new(),
            minimap_line_lengths_cache: RefCell::new(None),
            bracket_color_spans_cache: RefCell::new(None),
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

    pub(crate) fn mark_layout(&mut self, id: StableNodeId) {
        if self.nodes.contains(id) {
            let _ = self.mark(id, crate::schedule::DirtyMask::LAYOUT);
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

    /// Algorithm-level counters from the last non-empty drain, or the current
    /// frame accumulator while a product flush is running. An idle
    /// [`Self::take_system_work`] does not replace this snapshot.
    pub fn last_work_counters(&self) -> WorkCounters {
        let mut counters = self.last_counters;
        counters.record_hot_path_allocation(
            self.pending_hot_allocations.get(),
            self.pending_hot_allocated_bytes.get(),
        );
        counters
    }

    /// Start a multi-pass frame accumulator. Idle drains still leave the
    /// previous snapshot in place until a non-empty pass runs.
    pub fn begin_frame_counters(&mut self) {
        self.commit_pending_hot_allocs();
        self.frame_counters = WorkCounters::default();
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
        if self.accumulating_frame {
            self.frame_extracted_nodes = self.frame_extracted_nodes.saturating_add(extracted.len());
            self.frame_extracted_spans = self.frame_extracted_spans.saturating_add(spans);
            self.last_counters.render_nodes_extracted = self.frame_extracted_nodes;
            self.last_counters.extracted_text_spans = self.frame_extracted_spans;
        } else {
            self.last_counters.render_nodes_extracted = extracted.len();
            self.last_counters.extracted_text_spans = spans;
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
        self.last_counters.record_hot_path_allocation(count, bytes);
        if self.accumulating_frame {
            self.frame_counters.record_hot_path_allocation(count, bytes);
        }
    }

    fn bump_last_counters(&mut self, update: impl Fn(&mut WorkCounters)) {
        update(&mut self.last_counters);
        if self.accumulating_frame {
            update(&mut self.frame_counters);
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

    pub fn theme_mode(&self) -> ThemeMode {
        self.style_model.theme_mode
    }

    pub fn theme_metrics(&self) -> nana_ui_core::ThemeMetrics {
        self.style_model.metrics
    }

    pub fn style_model(&self) -> StyleModelRef {
        self.style_model
    }

    /// Drain dirty components into deterministic system work. Calling this on
    /// an unchanged world returns an empty work set and performs no scheduling.
    pub fn take_system_work(&mut self) -> SystemWork {
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
            layout: Vec::new(),
            transform: Vec::new(),
            input_hit_test: Vec::new(),
            focus_ime: Vec::new(),
            accessibility: Vec::new(),
            accessibility_removals: std::mem::take(&mut self.pending_accessibility_removals),
            render_extraction: Vec::new(),
            render_removals: std::mem::take(&mut self.pending_render_removals),
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
            let has_text = matches!(self.record(id).kind.as_ref(), NodeKind::Text)
                || !self.record(id).text.value.is_empty()
                || matches!(
                    self.nodes.visual(id),
                    Some(StandardVisual::EmptyState { .. })
                        | Some(StandardVisual::ModalFrame { .. })
                );
            let bits = if has_text {
                bits
            } else {
                bits & !DirtyMask::TEXT
            };
            if bits != 0 {
                work.entities_changed += 1;
            }
            push_work(&mut work, id, bits);
        }
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
        bump_list(work.layout.len());
        bump_list(work.transform.len());
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
            // Extracted node/span fields are filled by [`Self::record_extract`]
            // on the product path, not by the planned render list.
            counters.render_nodes_extracted = 0;
            counters.extracted_text_spans = 0;
            if self.accumulating_frame {
                self.frame_counters.accumulate(counters);
                self.last_counters = self.frame_counters;
                self.last_counters.render_nodes_extracted = self.frame_extracted_nodes;
                self.last_counters.extracted_text_spans = self.frame_extracted_spans;
            } else {
                self.last_counters = counters;
            }
        } else {
            self.commit_pending_hot_allocs();
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
        for (ids, bit) in [
            (work.style, DirtyMask::STYLE),
            (work.state, DirtyMask::STATE),
            (work.text, DirtyMask::TEXT),
            (work.layout, DirtyMask::LAYOUT),
            (work.transform, DirtyMask::TRANSFORM),
            (work.input_hit_test, DirtyMask::INPUT),
            (work.focus_ime, DirtyMask::FOCUS_IME),
            (work.accessibility, DirtyMask::ACCESSIBILITY),
            (work.render_extraction, DirtyMask::RENDER),
        ] {
            for id in ids {
                if !self.nodes.contains(id) {
                    continue;
                };
                self.nodes
                    .get_mut(id)
                    .map(|n| &mut n.dirty)
                    .expect("retained node must have dirty state")
                    .insert(bit);
                self.dirty_entities.insert(id);
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

    pub fn focused_text_input(
        &self,
        document: DocumentId,
    ) -> Option<(StableNodeId, &TextInputState)> {
        let id = self.focused(document)?;
        Some((id, self.text_input(id)?))
    }

    pub fn text(&self, id: StableNodeId) -> Option<&str> {
        self.nodes
            .get(id)
            .map(|n| &n.text)
            .map(|text| text.value.as_str())
    }

    pub fn layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        self.nodes.get(id).map(|node| node.layout)
    }

    pub fn scroll_offset(&self, id: StableNodeId) -> Option<ScrollOffset> {
        self.nodes.get(id).map(|node| node.scroll_offset)
    }

    pub fn scroll_metrics(&self, id: StableNodeId) -> Option<ScrollMetrics> {
        self.nodes.scroll_metrics(id).copied()
    }

    pub fn clamp_scroll_offset(&self, id: StableNodeId, offset: ScrollOffset) -> ScrollOffset {
        self.scroll_metrics(id)
            .map_or(offset, |metrics| metrics.clamp(offset))
    }

    pub fn node_style(&self, id: StableNodeId) -> Option<&NodeStyle> {
        self.nodes.get(id).map(|node| &node.style)
    }

    pub fn computed_style(&self, id: StableNodeId) -> Option<&ComputedStyle> {
        self.nodes.get(id).map(|node| node.resolved.0.as_ref())
    }

    /// Whether a mounted node is visible through every retained overlay branch.
    /// Dirty computed styles are derived from the local hierarchy instead of
    /// treating the previous frame's visibility as current authority.
    pub fn is_overlay_reachable(&self, id: StableNodeId) -> bool {
        let mut child = id;
        let mut current = Some(id);
        while let Some(candidate) = current {
            if !self.presence_live(candidate)
                || self
                    .node_style(candidate)
                    .is_some_and(|style| style.layout.omits_box())
                || (!self.dirty_entities.contains(&candidate)
                    && self
                        .computed_style(candidate)
                        .is_some_and(|style| !style.visible))
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
        true
    }

    pub fn interaction(&self, id: StableNodeId) -> Option<InteractionState> {
        self.nodes.get(id).map(|node| node.interaction)
    }

    pub fn text_input(&self, id: StableNodeId) -> Option<&TextInputState> {
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

    fn committed_presentation_text(&self, id: StableNodeId) -> String {
        self.nodes
            .text_input(id)
            .map(|state| state.value.clone())
            .unwrap_or_else(|| self.record(id).text.value.clone())
    }

    pub fn text_metrics(&self, id: StableNodeId) -> Option<TextMetrics> {
        self.nodes.get(id).map(|node| node.text_metrics)
    }

    pub fn ime(&self, id: StableNodeId) -> Option<&ImeComposition> {
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

    fn note_structural_change(&mut self, parent: StableNodeId) {
        if self.structural_change_parents.last() != Some(&parent) {
            self.structural_change_parents.push(parent);
        }
    }

    /// Parents whose child list changed since the last drain, deduplicated in
    /// ascending order. Consumers must drain per commit so scheduled
    /// reprojections observe the post-mutation tree.
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

    pub fn component_geometry(&self, id: StableNodeId) -> Option<crate::ComponentGeometry> {
        let visual = self.nodes.visual(id)?;
        let style = self.nodes.get(id)?.resolved.0.as_ref();
        self.derive_component_geometry(id, visual, style)
    }

    pub fn accessibility(&self, id: StableNodeId) -> Option<&AccessibilityState> {
        self.nodes.get(id).map(|node| &node.accessibility)
    }

    pub fn overlay_host(&self, id: StableNodeId) -> Option<OverlayHostState> {
        self.nodes.overlay_host(id).copied()
    }

    /// Nodes that carry an `OverlayHostState`. Overlay validation iterates this
    /// instead of the entity index so cost tracks host count, not world size.
    fn overlay_host_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.overlay_host_nodes.iter().copied()
    }

    /// Drop focus and composition when dirty visual or interaction state makes
    /// the focused node ineligible.
    pub fn reconcile_focus(&mut self, ids: &[StableNodeId]) {
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
            self.mark(
                id,
                DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
            );
        }
    }

    pub fn layout_inputs(&self, ids: &[StableNodeId]) -> Result<Vec<LayoutInput>, UiWorldError> {
        if !ids.is_empty() {
            self.record_hot_path_allocation(1, ids.len().saturating_mul(size_of::<LayoutInput>()));
        }
        ids.iter()
            .copied()
            .map(|id| {
                if !self.contains(id) {
                    return Err(UiWorldError::MissingNode(id));
                }
                let hierarchy = &self.record(id).hierarchy;
                let has_text = matches!(self.record(id).kind.as_ref(), NodeKind::Text)
                    || !self.record(id).text.value.is_empty();
                Ok(LayoutInput {
                    id,
                    parent: hierarchy.parent,
                    children: Arc::clone(&hierarchy.children),
                    style: self.effective_layout_style(id),
                    text_metrics: has_text.then(|| self.record(id).text_metrics),
                    modal: self.nodes.visual(id).and_then(|visual| {
                        let StandardVisual::ModalFrame { kind, slots, .. } = visual else {
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
                    }),
                })
            })
            .collect()
    }

    pub(crate) fn write_layout_padding(
        &mut self,
        id: StableNodeId,
        padding: nana_ui_core::PaddingSpec,
    ) -> bool {
        let record = self.record_mut(id);
        let changed = record.layout_padding != Some(padding);
        record.layout_padding = Some(padding);
        changed
    }

    /// Padding resolved by the layout pass, including its containing block and font.
    pub(crate) fn used_layout_padding(&self, id: StableNodeId) -> nana_ui_core::PaddingSpec {
        let record = self.record(id);
        record.layout_padding.unwrap_or_else(|| {
            record
                .style
                .layout
                .resolved_padding_against(Some(record.layout.width))
        })
    }

    /// Layout-facing style without assembling a [`LayoutInput`].
    ///
    /// Parked, detached, and inactive-overlay nodes match [`Self::layout_inputs`]:
    /// the returned style reports [`nana_ui_core::LayoutStyle::omits_box`].
    pub(crate) fn layout_style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        self.contains(id).then(|| self.effective_layout_style(id))
    }

    fn effective_layout_style(&self, id: StableNodeId) -> Arc<nana_ui_core::LayoutStyle> {
        let mut style = Arc::clone(&self.record(id).style.layout);
        if style.omits_box()
            || !self.presence_live(id)
            || !self.overlay_branch_active(id)
            || !self.menu_branch_open(id)
        {
            Arc::make_mut(&mut style).hidden = true;
            return style;
        }
        if let Some(overlay) = self.parent_triggered_overlay(id) {
            let layout = Arc::make_mut(&mut style);
            layout.position = PositionSpec::Fixed;
            layout.width = Some(LengthSpec::Px(
                (overlay.width - overlay.padding * 2.0).max(0.0),
            ));
            layout.z_index = Some(crate::popover::MENU_OVERLAY_Z_INDEX);
        }
        style
    }

    fn parent_triggered_overlay(&self, id: StableNodeId) -> Option<crate::TriggeredMenuOverlay> {
        let parent = self.record(id).hierarchy.parent?;
        match self.nodes.visual(parent)? {
            StandardVisual::MenuSurface {
                open: true,
                overlay: Some(overlay),
                ..
            } => Some(*overlay),
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

    pub(crate) fn parent_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.nodes.get(id)?.hierarchy.parent
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
        if !self.is_mounted(id) {
            return false;
        }
        if self.detached.is_empty() {
            return true;
        }
        self.presence_live_memo(id, &mut AncestorMemo::default())
    }

    fn presence_live_memo(&self, id: StableNodeId, memo: &mut AncestorMemo) -> bool {
        if !self.is_mounted(id) {
            return false;
        }
        if self.detached.is_empty() {
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

    fn presence_flags_of(&self, id: StableNodeId) -> PresenceFlags {
        let visual = self.nodes.visual(id);
        let style = self.nodes.get(id).map(|n| &n.style);
        PresenceFlags {
            confirm: is_confirm_modal(visual),
            clip: is_clip_visual(visual),
            z_index: style.is_some_and(|style| style.layout.z_index.is_some()),
            viewport: style.is_some_and(|style| style.layout.depends_on_viewport()),
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
        if let Some(id) = id {
            if next.viewport {
                self.viewport_basis.insert(id);
            } else {
                self.viewport_basis.remove(&id);
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
        self.viewport_basis.iter().copied()
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

    pub(crate) fn is_descendant_or_self(&self, id: StableNodeId, ancestor: StableNodeId) -> bool {
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
        let mut current = Some(id);
        while let Some(node) = current {
            if let Some(specified) = self.record(node).style.layout.pointer_events {
                return specified;
            }
            current = self.parent_id(node);
        }
        PointerEventsSpec::Auto
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

    fn remove_ime(&mut self, id: StableNodeId) {
        self.nodes.set_ime(id, None);
    }

    fn clear_overlay_references(&mut self, removed: StableNodeId) {
        self.clear_overlay_references_for(&[removed]);
    }

    /// Drop `removed` from every overlay host that still points at it. Takes a
    /// slice so tearing down a subtree walks the host index once instead of
    /// once per removed node.
    fn clear_overlay_references_for(&mut self, removed: &[StableNodeId]) {
        if self.overlay_host_nodes.is_empty() || removed.is_empty() {
            return;
        }
        let updates = self
            .overlay_host_nodes
            .iter()
            .filter_map(|&host| {
                (!removed.contains(&host))
                    .then(|| self.nodes.overlay_host(host).copied())
                    .flatten()
                    .and_then(|mut state| {
                        let previous = state;
                        let restore_focus = state
                            .active
                            .is_some_and(|active| removed.contains(&active))
                            .then_some(state.restore_focus)
                            .flatten();
                        if state.active.is_some_and(|active| removed.contains(&active)) {
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
            self.nodes.set_overlay_host(host, Some(state));
            self.mark(host, DirtyMask::ACCESSIBILITY);
            let document = self.record(host).document;
            if let Some(restore_focus) = restore_focus.filter(|id| {
                self.contains(*id)
                    && self.is_mounted(*id)
                    && self.record(*id).document == document
                    && self.record(*id).interaction.focusable
                    && self.record(*id).resolved.0.visible
                    && self.active_modal_allows_focus_now(document, *id)
            }) {
                self.input.focused.insert(document, restore_focus);
                self.mark(
                    restore_focus,
                    DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
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
        let Some(parent) = self.record(id).hierarchy.parent else {
            return true;
        };
        if !self.nodes.contains(parent) {
            return true;
        };
        menu_surface_open(self.nodes.visual(parent)) != Some(false)
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
            for roots in self.live_document_roots.values_mut() {
                roots.remove(&id);
            }
            self.live_document_roots
                .retain(|_, roots| !roots.is_empty());
            return;
        }
        let node = self.record(id);
        let document = node.document;
        let parent = node.hierarchy.parent;
        let live_root = parent.is_none() && self.presence_live(id);
        if live_root {
            self.live_document_roots
                .entry(document)
                .or_default()
                .insert(id);
            return;
        }
        let empty = self
            .live_document_roots
            .get_mut(&document)
            .is_some_and(|roots| {
                roots.remove(&id);
                roots.is_empty()
            });
        if empty {
            self.live_document_roots.remove(&document);
        }
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

    fn hierarchy_mut(&mut self, id: StableNodeId) -> &mut Hierarchy {
        &mut self.record_mut(id).hierarchy
    }

    fn mark(&mut self, id: StableNodeId, bits: u16) -> bool {
        let changed = self.record_mut(id).dirty.insert(bits);
        if changed {
            self.dirty_entities.insert(id);
        }
        changed
    }

    fn mark_subtree(&mut self, root: StableNodeId, bits: u16) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id).expect("hierarchy node must exist").children;
            stack.extend(children.iter().rev().copied());
            let _ = self.mark(id, bits);
        }
    }

    fn subtree_ids(&self, root: StableNodeId) -> Vec<StableNodeId> {
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id).expect("hierarchy node must exist").children;
            stack.extend(children.iter().rev().copied());
            ids.push(id);
        }
        ids
    }

    fn set_subtree_mount_state(&mut self, root: StableNodeId, state: MountState) {
        let ids = self.subtree_ids(root);
        for id in &ids {
            self.record_mut(*id).mount = state;
        }
        if state == MountState::Mounted {
            self.mark_subtree(root, DirtyMask::ALL);
        }
    }

    fn unlink_from_parent(&mut self, id: StableNodeId) -> bool {
        let Some(parent) = self.node(id).expect("validated node must exist").parent else {
            return false;
        };
        let hierarchy = self.hierarchy_mut(parent);
        Arc::make_mut(&mut hierarchy.children).retain(|child| *child != id);
        intern_empty_children(&mut hierarchy.children);
        let _hierarchy = hierarchy;
        self.hierarchy_mut(id).parent = None;
        self.mark_ancestors(
            parent,
            DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
        );
        self.note_structural_change(parent);
        true
    }

    fn leave_live_document(&mut self, root: StableNodeId) {
        let subtree = self.subtree_ids(root);
        self.retire_subtree_from_document(&subtree);
        self.detached.insert(root);
        self.sync_subtree_presence(root);
    }

    fn retire_subtree_from_document(&mut self, subtree: &[StableNodeId]) {
        let parked = subtree.iter().copied().collect::<HashSet<_>>();
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

        let cancelled = self
            .animations
            .iter()
            .filter_map(|(&animation_id, animation)| {
                parked
                    .contains(&animation.spec.target)
                    .then_some((animation_id, animation.next_deadline))
            })
            .collect::<Vec<_>>();
        for (animation_id, deadline) in cancelled {
            self.animations.remove(&animation_id);
            self.animation_deadlines.remove(&(deadline, animation_id));
        }

        for &id in subtree {
            self.surface_motion.remove(&id);
            self.closing_surfaces.remove(&id);
            self.switch_transitions.remove(&id);
            self.hover_transitions.remove(&id);
            if self.overlay_host(id).is_some() {
                self.nodes
                    .set_overlay_host(id, Some(OverlayHostState::default()));
            }
        }
        self.clear_overlay_references_for(subtree);
        self.pending_render_removals.sort_unstable();
        self.pending_render_removals.dedup();
        self.pending_accessibility_removals.sort_unstable();
        self.pending_accessibility_removals.dedup();
    }

    fn mark_ancestors(&mut self, start: StableNodeId, bits: u16) {
        let mut current = Some(start);
        while let Some(id) = current {
            current = self
                .identity_and_parent(id)
                .expect("hierarchy node must exist")
                .1;
            if !self.mark(id, bits) {
                break;
            }
        }
    }

    fn propagate_layout_from_node(&mut self, id: StableNodeId) {
        self.mark(id, DirtyMask::LAYOUT | DirtyMask::RENDER);
        if let Some(parent) = self.parent_id(id) {
            self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
        }
    }
}

const IDENTITY_AFFINE: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PresenceFlags {
    confirm: bool,
    clip: bool,
    z_index: bool,
    /// Style resolves against the viewport (`position: fixed`, `vw` / `vh`).
    viewport: bool,
}

impl PresenceFlags {
    const NONE: Self = Self {
        confirm: false,
        clip: false,
        z_index: false,
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

fn text_intrinsic_changed(previous: TextMetrics, next: TextMetrics) -> bool {
    previous.width != next.width || previous.height != next.height
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

fn style_excluding_transform_eq(left: &NodeStyle, right: &NodeStyle) -> bool {
    left.foreground == right.foreground
        && left.background == right.background
        && left.border == right.border
        && left.interaction == right.interaction
        && left.text_horizontal_alignment == right.text_horizontal_alignment
        && left.text_vertical_alignment == right.text_vertical_alignment
        && layout_excluding_transform_eq(left.layout.as_ref(), right.layout.as_ref())
}

fn layout_excluding_transform_eq(
    left: &nana_ui_core::LayoutStyle,
    right: &nana_ui_core::LayoutStyle,
) -> bool {
    let strip = |style: &nana_ui_core::LayoutStyle| {
        let mut style = style.clone();
        style.transform = None;
        style.transform_3d = None;
        style.unsupported_transform = None;
        style.transform_origin = None;
        style.transform_box = nana_ui_core::TransformBox::ViewBox;
        style.css_perspective = None;
        style.preserve_3d = false;
        style
    };
    strip(left) == strip(right)
}

fn layout_semantics_changed(
    previous: &nana_ui_core::LayoutStyle,
    next: &nana_ui_core::LayoutStyle,
) -> bool {
    previous.direction != next.direction
        || previous.dir != next.dir
        || previous.flex_reverse != next.flex_reverse
        || previous.order != next.order
        || previous.flex_wrap != next.flex_wrap
        || previous.display != next.display
        || previous.box_sizing != next.box_sizing
        || previous.position != next.position
        || previous.gap != next.gap
        || previous.row_gap != next.row_gap
        || previous.column_gap != next.column_gap
        || previous.padding != next.padding
        || previous.padding_top != next.padding_top
        || previous.padding_right != next.padding_right
        || previous.padding_bottom != next.padding_bottom
        || previous.padding_left != next.padding_left
        || previous.margin != next.margin
        || previous.margin_top != next.margin_top
        || previous.margin_right != next.margin_right
        || previous.margin_bottom != next.margin_bottom
        || previous.margin_left != next.margin_left
        || previous.offset_top != next.offset_top
        || previous.offset_right != next.offset_right
        || previous.offset_bottom != next.offset_bottom
        || previous.offset_left != next.offset_left
        || previous.width != next.width
        || previous.height != next.height
        || previous.min_width != next.min_width
        || previous.max_width != next.max_width
        || previous.min_height != next.min_height
        || previous.max_height != next.max_height
        || previous.allow_shrink != next.allow_shrink
        || previous.align_items != next.align_items
        || previous.align_self != next.align_self
        || previous.align_content != next.align_content
        || previous.justify_content != next.justify_content
        || previous.justify_items != next.justify_items
        || previous.justify_self != next.justify_self
        || previous.flex_grow != next.flex_grow
        || previous.flex_shrink != next.flex_shrink
        || previous.flex_basis != next.flex_basis
        || previous.overflow_x != next.overflow_x
        || previous.overflow_y != next.overflow_y
        || previous.text_overflow_ellipsis != next.text_overflow_ellipsis
        || previous.line_clamp != next.line_clamp
        || previous.white_space_nowrap != next.white_space_nowrap
        || previous.white_space != next.white_space
        || previous.word_break != next.word_break
        || previous.overflow_wrap != next.overflow_wrap
        || previous.aspect_ratio != next.aspect_ratio
        || previous.font_italic != next.font_italic
        || previous.text_align != next.text_align
        || previous.word_break != next.word_break
        || previous.line_break != next.line_break
        || previous.float != next.float
        || previous.clear != next.clear
        || previous.writing_mode != next.writing_mode
        || previous.grid_template_areas != next.grid_template_areas
        || previous.grid_column_line_names != next.grid_column_line_names
        || previous.grid_row_line_names != next.grid_row_line_names
        || previous.grid_columns != next.grid_columns
        || previous.grid_rows != next.grid_rows
        || previous.grid_columns_unsupported != next.grid_columns_unsupported
        || previous.grid_rows_unsupported != next.grid_rows_unsupported
        || previous.grid_auto_columns != next.grid_auto_columns
        || previous.grid_auto_rows != next.grid_auto_rows
        || previous.grid_auto_flow != next.grid_auto_flow
        || previous.grid_columns_repeat != next.grid_columns_repeat
        || previous.grid_rows_repeat != next.grid_rows_repeat
        || previous.grid_placement != next.grid_placement
        || previous.border_width != next.border_width
        || previous.border_top_width != next.border_top_width
        || previous.border_right_width != next.border_right_width
        || previous.border_bottom_width != next.border_bottom_width
        || previous.border_left_width != next.border_left_width
        || previous.border_style != next.border_style
        || previous.border_top_style != next.border_top_style
        || previous.border_right_style != next.border_right_style
        || previous.border_bottom_style != next.border_bottom_style
        || previous.border_left_style != next.border_left_style
}

#[cfg(test)]
mod tests;
