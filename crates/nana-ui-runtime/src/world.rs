use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::mem::size_of;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use bevy_ecs::component::{Component, Mutable};
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use nana_ui_core::{
    ControlSize, GraphPoint, GraphPortKind, GraphPortSide, LayoutStyle, PointerEventsSpec,
    SemanticColorRole, SemanticPalette, StyleModelRef, SwitchControlPosition, ThemeMode,
    TooltipConfig, cubic_point, icon_y_on_text_glyph_center,
};

use crate::animation::ActiveAnimation;
use crate::components::{EmptyStateTextPresentation, ModalTextPresentation};
use crate::schedule::{DirtyMask, SystemWork, push_work};
use crate::{
    AccessibilityDelta, AccessibilityNode, AccessibilityRole, AccessibilityState, AnimationFrame,
    AnimationId, AnimationSpec, ComponentTypeId, ComputedStyle, CustomRenderNode, EventListeners,
    EventRoute, ExtractedNode, ExtractedTextSpan, HighlightRequest, ImeComposition,
    InteractionState, LayoutBox, LayoutInput, MountState, MutationQueue, NodeStyle,
    OverlayHostState, PointerCaptureChange, ScrollMetrics, ScrollOffset, StandardVisual,
    TextContent, TextInputPresentation, TextInputState, TextMetrics, TextPresentation,
    TextPresenter, TextShaper, TextVerticalAlignment, UiMutation, WorkCounters,
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

#[derive(Component)]
struct Identity {
    stable: StableNodeId,
    document: DocumentId,
}

#[derive(Component)]
struct Kind(Arc<NodeKind>);

/// `.1` is the palette epoch (`0` = never resolved).
#[derive(Component, Clone)]
struct ResolvedStyle(Arc<ComputedStyle>, u64);

fn menu_surface_open(visual: Option<&StandardVisual>) -> Option<bool> {
    match visual {
        Some(StandardVisual::MenuSurface { open, .. }) => Some(*open),
        _ => None,
    }
}

#[derive(Component)]
struct Hierarchy {
    parent: Option<StableNodeId>,
    children: Arc<Vec<StableNodeId>>,
}

impl Default for Hierarchy {
    fn default() -> Self {
        Self {
            parent: None,
            children: Arc::clone(&EMPTY_CHILDREN),
        }
    }
}

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

fn intern_kind(kind: &NodeKind) -> Arc<NodeKind> {
    match kind {
        NodeKind::Document => Arc::clone(&INTERNED_KIND_DOCUMENT),
        NodeKind::Text => Arc::clone(&INTERNED_KIND_TEXT),
        NodeKind::Comment => Arc::clone(&INTERNED_KIND_COMMENT),
        NodeKind::Element { tag } if tag == "div" => Arc::clone(&INTERNED_KIND_DIV),
        _ => Arc::new(kind.clone()),
    }
}

fn intern_empty_children(children: &mut Arc<Vec<StableNodeId>>) {
    if children.is_empty() {
        *children = Arc::clone(&EMPTY_CHILDREN);
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
    world: World,
    entities: HashMap<StableNodeId, Entity>,
    retired: RetiredIds,
    dirty_entities: HashSet<StableNodeId>,
    focused: HashMap<DocumentId, StableNodeId>,
    hit_test_index: HashMap<DocumentId, Vec<HitEntry>>,
    /// Scroll deltas awaiting the in-place hit-index patch (see
    /// `UiMutation::SetScrollOffset`). Drained by the frame driver.
    scroll_hit_updates: Vec<(StableNodeId, [f32; 2])>,
    pointer_captures: HashMap<(DocumentId, u64), StableNodeId>,
    pointer_hover: HashMap<(DocumentId, u64), StableNodeId>,
    pointer_press: HashMap<(DocumentId, u64), StableNodeId>,
    pending_pointer_capture_changes: Vec<PointerCaptureChange>,
    pending_render_removals: Vec<StableNodeId>,
    pending_accessibility_removals: Vec<StableNodeId>,
    animations: HashMap<AnimationId, ActiveAnimation>,
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
    presence_flags: HashMap<Entity, PresenceFlags>,
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
}

impl Default for UiWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl UiWorld {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            entities: HashMap::new(),
            retired: RetiredIds::default(),
            dirty_entities: HashSet::new(),
            focused: HashMap::new(),
            hit_test_index: HashMap::new(),
            scroll_hit_updates: Vec::new(),
            pointer_captures: HashMap::new(),
            pointer_hover: HashMap::new(),
            pointer_press: HashMap::new(),
            pending_pointer_capture_changes: Vec::new(),
            pending_render_removals: Vec::new(),
            pending_accessibility_removals: Vec::new(),
            animations: HashMap::new(),
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
        }
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn contains(&self, id: StableNodeId) -> bool {
        self.entities.contains_key(&id)
    }

    pub(crate) fn mark_layout(&mut self, id: StableNodeId) {
        if self.entities.contains_key(&id) {
            let _ = self.mark(id, crate::schedule::DirtyMask::LAYOUT);
        }
    }

    /// Whether the node is `Mounted`. Parked is not; Detach stays mounted but
    /// is omitted from the live document until inserted.
    pub fn is_mounted(&self, id: StableNodeId) -> bool {
        let Some(&entity) = self.entities.get(&id) else {
            return false;
        };
        self.world
            .get::<MountState>(entity)
            .is_some_and(|state| *state == MountState::Mounted)
    }

    pub fn mount_state(&self, id: StableNodeId) -> Option<MountState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<MountState>(entity).copied()
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

    pub fn event_route(&self, target: StableNodeId) -> Option<EventRoute> {
        if !self.is_mounted(target) {
            return None;
        }
        let mut bubble = Vec::new();
        let mut current = self.parent_id(target);
        while let Some(id) = current {
            bubble.push(id);
            current = self.parent_id(id);
        }
        let mut capture = bubble.clone();
        capture.reverse();
        Some(EventRoute {
            capture,
            target,
            bubble,
        })
    }

    pub fn pointer_capture(&self, document: DocumentId, pointer_id: u64) -> Option<StableNodeId> {
        self.pointer_captures.get(&(document, pointer_id)).copied()
    }

    pub fn pointer_captures(&self, document: DocumentId) -> Vec<(u64, StableNodeId)> {
        let mut captures = self
            .pointer_captures
            .iter()
            .filter_map(|(&(owner, pointer_id), &target)| {
                (owner == document).then_some((pointer_id, target))
            })
            .collect::<Vec<_>>();
        captures.sort_unstable_by_key(|(pointer_id, _)| *pointer_id);
        captures
    }

    pub fn take_pointer_capture_changes(&mut self) -> Vec<PointerCaptureChange> {
        std::mem::take(&mut self.pending_pointer_capture_changes)
    }

    pub fn pointer_hover(&self, document: DocumentId, pointer_id: u64) -> Option<StableNodeId> {
        self.pointer_hover.get(&(document, pointer_id)).copied()
    }

    pub fn pointer_press(&self, document: DocumentId, pointer_id: u64) -> Option<StableNodeId> {
        self.pointer_press.get(&(document, pointer_id)).copied()
    }

    /// Update per-pointer hover authority and invalidate only the old/new
    /// interaction paint. DOM adapters may derive enter/leave paths from the
    /// returned previous target and the retained hierarchy.
    pub fn set_pointer_hover(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: Option<StableNodeId>,
    ) -> Result<Option<StableNodeId>, UiWorldError> {
        if let Some(target) = target {
            self.validate_pointer_target(document, target)?;
        }
        let key = (document, pointer_id);
        let previous = match target {
            Some(target) => self.pointer_hover.insert(key, target),
            None => self.pointer_hover.remove(&key),
        };
        if previous != target {
            self.generation = self.generation.wrapping_add(1);
            if let Some(previous) = previous {
                self.mark_interaction_style(previous);
            }
            if let Some(target) = target {
                self.mark_interaction_style(target);
            }
        }
        Ok(previous)
    }

    pub fn press_pointer(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
    ) -> Result<Option<StableNodeId>, UiWorldError> {
        self.validate_pointer_target(document, target)?;
        let previous = self.pointer_press.insert((document, pointer_id), target);
        if previous != Some(target) {
            self.generation = self.generation.wrapping_add(1);
            if let Some(previous) = previous {
                self.mark_interaction_style(previous);
            }
            self.mark_interaction_style(target);
        }
        Ok(previous)
    }

    pub fn release_pointer_press(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
    ) -> Option<StableNodeId> {
        let previous = self.pointer_press.remove(&(document, pointer_id));
        if let Some(previous) = previous {
            self.generation = self.generation.wrapping_add(1);
            self.mark_interaction_style(previous);
        }
        previous
    }

    pub fn clear_pointer_interactions(&mut self, document: DocumentId) {
        let affected = self
            .pointer_hover
            .iter()
            .chain(&self.pointer_press)
            .filter_map(|(&(owner, _), &target)| (owner == document).then_some(target))
            .collect::<HashSet<_>>();
        self.pointer_hover
            .retain(|(owner, _), _| *owner != document);
        self.pointer_press
            .retain(|(owner, _), _| *owner != document);
        if !affected.is_empty() {
            self.generation = self.generation.wrapping_add(1);
            for target in affected {
                self.mark_interaction_style(target);
            }
        }
    }

    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.animation_deadlines
            .first()
            .map(|(deadline, _)| *deadline)
    }

    /// Sample only active animations that are due at `now`. This method does
    /// not mark render state dirty: consumers apply sampled values through the
    /// normal atomic mutation boundary.
    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        let mut animation_deadlines_scanned = 0usize;
        let due = self
            .animation_deadlines
            .range(..=(now, AnimationId::new(u64::MAX).expect("max ID is nonzero")))
            .inspect(|_| {
                animation_deadlines_scanned = animation_deadlines_scanned.saturating_add(1)
            })
            .copied()
            .collect::<Vec<_>>();
        let mut samples = Vec::with_capacity(due.len());
        let mut animations_considered = 0usize;
        for (deadline, id) in due {
            self.animation_deadlines.remove(&(deadline, id));
            animations_considered = animations_considered.saturating_add(1);
            let (sample, next_deadline) = {
                let animation = self
                    .animations
                    .get_mut(&id)
                    .expect("due animation must remain active");
                let sample = animation
                    .sample(now)
                    .expect("due animation must produce a sample");
                let next_deadline = (!sample.finished).then_some(animation.next_deadline);
                (sample, next_deadline)
            };
            if sample.finished {
                self.animations.remove(&id);
            } else if let Some(next_deadline) = next_deadline {
                self.animation_deadlines.insert((next_deadline, id));
            }
            samples.push(sample);
        }
        AnimationFrame {
            samples,
            component_updates: Vec::new(),
            next_deadline: self.next_animation_deadline(),
            animation_deadlines_scanned,
            animations_considered,
        }
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
            entities_total: self.entities.len(),
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
            let entity = self.entities[&id];
            let bits = self
                .world
                .get_mut::<DirtyMask>(entity)
                .expect("entity must have dirty component")
                .take();
            if !self.presence_live(id) {
                continue;
            }
            let has_text = matches!(self.component::<Kind>(id).0.as_ref(), NodeKind::Text)
                || !self.component::<TextContent>(id).value.is_empty()
                || matches!(
                    self.world.get::<StandardVisual>(entity),
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
                let Some(&entity) = self.entities.get(&id) else {
                    continue;
                };
                self.world
                    .get_mut::<DirtyMask>(entity)
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
        let entity = *self.entities.get(&id)?;
        let identity = self.world.get::<Identity>(entity)?;
        let kind = self.world.get::<Kind>(entity)?;
        let hierarchy = self.world.get::<Hierarchy>(entity)?;
        Some(NodeSnapshot {
            id: identity.stable,
            document: identity.document,
            kind: kind.0.as_ref().clone(),
            parent: hierarchy.parent,
            children: hierarchy.children.as_ref().clone(),
        })
    }

    pub fn focused(&self, document: DocumentId) -> Option<StableNodeId> {
        self.focused.get(&document).copied()
    }

    pub fn focused_text_input(
        &self,
        document: DocumentId,
    ) -> Option<(StableNodeId, &TextInputState)> {
        let id = self.focused(document)?;
        Some((id, self.text_input(id)?))
    }

    pub fn text(&self, id: StableNodeId) -> Option<&str> {
        let entity = *self.entities.get(&id)?;
        self.world
            .get::<TextContent>(entity)
            .map(|text| text.value.as_str())
    }

    pub fn layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<LayoutBox>(entity).copied()
    }

    pub fn scroll_offset(&self, id: StableNodeId) -> Option<ScrollOffset> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<ScrollOffset>(entity).copied()
    }

    pub fn scroll_metrics(&self, id: StableNodeId) -> Option<ScrollMetrics> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<ScrollMetrics>(entity).copied()
    }

    pub fn clamp_scroll_offset(&self, id: StableNodeId, offset: ScrollOffset) -> ScrollOffset {
        self.scroll_metrics(id)
            .map_or(offset, |metrics| metrics.clamp(offset))
    }

    pub fn node_style(&self, id: StableNodeId) -> Option<&NodeStyle> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<NodeStyle>(entity)
    }

    pub fn computed_style(&self, id: StableNodeId) -> Option<&ComputedStyle> {
        let entity = *self.entities.get(&id)?;
        self.world
            .get::<ResolvedStyle>(entity)
            .map(|style| style.0.as_ref())
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
        let entity = *self.entities.get(&id)?;
        self.world.get::<InteractionState>(entity).copied()
    }

    pub fn text_input(&self, id: StableNodeId) -> Option<&TextInputState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<TextInputState>(entity)
    }

    pub fn text_input_presentation(&self, id: StableNodeId) -> Option<&TextInputPresentation> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<TextInputPresentation>(entity)
    }

    pub fn highlight_request(&self, id: StableNodeId) -> Option<&HighlightRequest> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<HighlightRequest>(entity)
    }

    pub fn text_presentation(&self, id: StableNodeId) -> Option<&TextPresentation> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<TextPresentation>(entity)
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
        let ids = self.entities.keys().copied().collect::<Vec<_>>();
        for id in ids {
            if self
                .world
                .get::<HighlightRequest>(self.entities[&id])
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
            let entity = self.entities[&id];
            let Some(request) = self.world.get::<HighlightRequest>(entity).cloned() else {
                if self.world.get::<TextPresentation>(entity).is_some() {
                    self.world.entity_mut(entity).remove::<TextPresentation>();
                }
                continue;
            };
            let text = self.committed_presentation_text(id);
            let source = crate::presentation::presentation_source(&text, &request);
            if self
                .world
                .get::<TextPresentation>(entity)
                .is_some_and(|presentation| presentation.source == source)
            {
                continue;
            }
            let spans = self
                .presenters
                .get(request.presenter.as_ref())
                .map(|presenter| {
                    crate::presentation::sanitize_spans(&text, presenter.present(&text, &request))
                })
                .unwrap_or_default();
            self.world
                .entity_mut(entity)
                .insert(TextPresentation { spans, source });
        }
        Ok(())
    }

    fn committed_presentation_text(&self, id: StableNodeId) -> String {
        let entity = self.entities[&id];
        self.world
            .get::<TextInputState>(entity)
            .map(|state| state.value.clone())
            .unwrap_or_else(|| self.component::<TextContent>(id).value.clone())
    }

    pub fn text_metrics(&self, id: StableNodeId) -> Option<TextMetrics> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<TextMetrics>(entity).copied()
    }

    pub fn ime(&self, id: StableNodeId) -> Option<&ImeComposition> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<ImeComposition>(entity)
    }

    pub fn custom_render(&self, id: StableNodeId) -> Option<&CustomRenderNode> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<CustomRenderNode>(entity)
    }

    pub fn has_event(&self, id: StableNodeId, event: &str) -> bool {
        self.event_listeners(id)
            .is_some_and(|listeners| listeners.contains(event))
    }

    pub fn event_listeners(&self, id: StableNodeId) -> Option<&EventListeners> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<EventListeners>(entity)
    }

    pub fn component_type(&self, id: StableNodeId) -> Option<&ComponentTypeId> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<ComponentTypeId>(entity)
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
        let entity = *self.entities.get(&id)?;
        self.world.get::<StandardVisual>(entity).cloned()
    }

    pub fn component_geometry(&self, id: StableNodeId) -> Option<crate::ComponentGeometry> {
        let entity = *self.entities.get(&id)?;
        let visual = self.world.get::<StandardVisual>(entity)?;
        let style = self.world.get::<ResolvedStyle>(entity)?.0.as_ref();
        self.derive_component_geometry(id, visual, style)
    }

    pub fn accessibility(&self, id: StableNodeId) -> Option<&AccessibilityState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<AccessibilityState>(entity)
    }

    pub fn overlay_host(&self, id: StableNodeId) -> Option<OverlayHostState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<OverlayHostState>(entity).copied()
    }

    /// Nodes that carry an `OverlayHostState`. Overlay validation iterates this
    /// instead of the entity index so cost tracks host count, not world size.
    fn overlay_host_ids(&self) -> impl Iterator<Item = StableNodeId> + '_ {
        self.overlay_host_nodes.iter().copied()
    }

    /// Project the visible accessibility tree from the same retained authority.
    pub fn project_accessibility(&self, document: DocumentId) -> Vec<AccessibilityNode> {
        self.document_order(document)
            .into_iter()
            .filter_map(|id| self.project_accessibility_node(id))
            .collect()
    }

    /// Project only accessibility nodes named by scheduled dirty work.
    pub fn project_accessibility_nodes(&self, ids: &[StableNodeId]) -> Vec<AccessibilityNode> {
        ids.iter()
            .filter_map(|&id| self.project_accessibility_node(id))
            .collect()
    }

    /// Project one complete incremental accessibility transaction, including
    /// tombstones for nodes removed from the retained world.
    pub fn project_accessibility_delta(&self, work: &SystemWork) -> AccessibilityDelta {
        let mut removed = work
            .accessibility_removals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        removed.extend(work.accessibility.iter().copied().filter(|id| {
            self.entities.contains_key(id) && self.project_accessibility_node(*id).is_none()
        }));
        AccessibilityDelta {
            generation: work.generation,
            updated: self.project_accessibility_nodes(&work.accessibility),
            removed: removed.into_iter().collect(),
        }
    }

    /// Resolve inherited visual state for dirty nodes. Parent state is always
    /// resolved before its descendants, independent of stable ID order.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        let mut resolved = HashSet::new();
        for &id in ids {
            self.resolve_style(id, &mut resolved)?;
        }
        self.reconcile_focus(ids);
        Ok(())
    }

    /// Drop focus and composition when dirty visual or interaction state makes
    /// the focused node ineligible.
    pub fn reconcile_focus(&mut self, ids: &[StableNodeId]) {
        let dirty = ids.iter().copied().collect::<HashSet<_>>();
        let invalid_focus = self
            .focused
            .iter()
            .filter_map(|(&document, &id)| {
                let invalid = dirty.contains(&id)
                    && (!self.component::<ResolvedStyle>(id).0.visible
                        || !self.component::<InteractionState>(id).focusable);
                invalid.then_some((document, id))
            })
            .collect::<Vec<_>>();
        for (document, id) in invalid_focus {
            self.focused.remove(&document);
            self.remove_ime(id);
            self.mark(
                id,
                DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
            );
        }
    }

    /// Shape only explicitly scheduled text. The runtime owns invalidation and
    /// storage while the renderer adapter supplies its real shaping backend.
    pub fn shape_text(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl TextShaper,
    ) -> Result<(), UiWorldError> {
        self.resolve_presentations(ids)?;
        // Production adapter: every host shaper (MeasureTextShaper, NanaTextShaper,
        // tests) is wrapped once so lookup/insert hit the same UiWorld caches.
        let mut cache = std::mem::take(&mut self.text_layout_cache);
        let mut glyphs = std::mem::take(&mut self.glyph_cache);
        let mut shaper = CountingShaper::new(shaper, &mut cache, &mut glyphs);
        if !ids.is_empty() {
            self.record_hot_path_allocation(
                1,
                ids.len().saturating_mul(size_of::<(
                    StableNodeId,
                    TextMetrics,
                    Option<TextInputPresentation>,
                )>()),
            );
        }
        let mut shaped = Vec::with_capacity(ids.len());
        let mut empty_shaped = Vec::new();
        let mut modal_shaped = Vec::new();
        for &id in ids {
            if !self.contains(id) {
                let _shaper = shaper;
                self.text_layout_cache = cache;
                self.glyph_cache = glyphs;
                return Err(UiWorldError::MissingNode(id));
            }
            let presentation = self.text_input_presentation_source(id);
            let text = presentation.as_ref().map_or_else(
                || self.component::<TextContent>(id).clone(),
                |source| source.text.clone(),
            );
            self.record_string_clone(text.value.len());
            let style = self.component::<ResolvedStyle>(id).0.as_ref().clone();
            if let Some(visual @ StandardVisual::EmptyState { .. }) =
                self.world.get::<StandardVisual>(self.entities[&id])
            {
                let intrinsic = shape_empty_state_text(id, visual, &style, None, &mut shaper);
                validate_text_metrics(id, intrinsic.title)?;
                if let Some(message) = intrinsic.message {
                    validate_text_metrics(id, message)?;
                }
                empty_shaped.push((id, intrinsic));
            }
            if let Some(visual @ StandardVisual::ModalFrame { .. }) =
                self.world.get::<StandardVisual>(self.entities[&id])
            {
                let intrinsic = shape_modal_text(id, visual, &style, None, &mut shaper);
                validate_text_metrics(id, intrinsic.title)?;
                if let Some(description) = intrinsic.description {
                    validate_text_metrics(id, description)?;
                }
                if let Some(body) = intrinsic.body {
                    validate_text_metrics(id, body)?;
                }
                modal_shaped.push((id, intrinsic));
            }
            let constraints = self.text_shape_constraints(id);
            let metrics = shaper.shape(id, &text, &style, constraints);
            validate_text_metrics(id, metrics)?;
            let presentation = presentation.map(|source| {
                shape_text_input_presentation(id, source, &style, constraints, &mut shaper)
            });
            shaped.push((id, metrics, presentation));
        }
        for (id, metrics, presentation) in shaped {
            let previous = *self.component::<TextMetrics>(id);
            *self.component_mut::<TextMetrics>(id) = metrics;
            if let Some(presentation) = presentation {
                self.world
                    .entity_mut(self.entities[&id])
                    .insert(presentation);
            }
            if text_intrinsic_changed(previous, metrics) {
                self.propagate_layout_from_node(id);
            }
        }
        for (id, presentation) in empty_shaped {
            self.apply_empty_state_text_presentation(id, presentation);
        }
        for (id, presentation) in modal_shaped {
            self.world
                .entity_mut(self.entities[&id])
                .insert(presentation);
        }
        let runs = shaper.runs;
        let wrap_layouts = shaper.wrap_layouts;
        let _shaper = shaper;
        let (hits, misses, evictions) = cache.take_counters();
        let glyph_stats = glyphs.take_counters();
        self.text_layout_cache = cache;
        self.glyph_cache = glyphs;
        self.bump_last_counters(|counters| {
            counters.record_text_shape(runs, hits, misses, wrap_layouts);
            counters.record_cache_eviction(evictions);
            if let Some((glyph_hits, glyph_misses)) = glyph_stats {
                counters.record_glyph_cache(glyph_hits, glyph_misses);
            }
        });
        Ok(())
    }

    /// Re-shape visible text against its resolved content box after the first
    /// layout pass. This closes wrapping/ellipsis height measurement without
    /// moving layout ownership into the renderer adapter.
    pub fn shape_text_for_layout(
        &mut self,
        document: DocumentId,
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        self.shape_text_for_layout_impl(self.document_order(document), shaper)
    }

    /// [`Self::shape_text_for_layout`] restricted to `ids` (typically the
    /// relayout scope plus nodes whose published box changed). Nodes outside
    /// the scope keep their previous shape, which already matches their
    /// unchanged constraints.
    pub fn shape_text_for_layout_scoped(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        let mut scope = ids.to_vec();
        scope.sort_unstable();
        scope.dedup();
        scope.retain(|id| self.entities.contains_key(id));
        self.shape_text_for_layout_impl(scope, shaper)
    }

    /// Nodes currently carrying a LAYOUT-dirty bit that has not been drained
    /// by [`Self::take_system_work`] — e.g. marked by a shaping pass between
    /// drains. Sorted for determinism.
    pub fn pending_layout_dirty(&self) -> Vec<StableNodeId> {
        let mut ids = self
            .dirty_entities
            .iter()
            .copied()
            .filter(|id| {
                self.entities
                    .get(id)
                    .and_then(|entity| self.world.get::<DirtyMask>(*entity))
                    .is_some_and(|mask| mask.has(DirtyMask::LAYOUT))
            })
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    fn shape_text_for_layout_impl(
        &mut self,
        ids: Vec<StableNodeId>,
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        // Same production adapter as [`Self::shape_text`].
        let mut cache = std::mem::take(&mut self.text_layout_cache);
        let mut glyphs = std::mem::take(&mut self.glyph_cache);
        let mut shaper = CountingShaper::new(shaper, &mut cache, &mut glyphs);
        let mut shaped = Vec::new();
        let mut empty_shaped = Vec::new();
        let mut modal_shaped = Vec::new();
        for id in ids {
            let presentation = self.text_input_presentation_source(id);
            let text = presentation.as_ref().map_or_else(
                || self.component::<TextContent>(id).clone(),
                |source| source.text.clone(),
            );
            self.record_string_clone(text.value.len());
            let computed = self.component::<ResolvedStyle>(id).0.as_ref();
            if let Some(visual @ StandardVisual::EmptyState { compact, .. }) =
                self.world.get::<StandardVisual>(self.entities[&id])
            {
                if computed.visible {
                    let layout = *self.component::<LayoutBox>(id);
                    let horizontal = if *compact { 6.0 } else { 16.0 };
                    let width = (layout.width - horizontal * 2.0).max(0.0);
                    let intrinsic =
                        shape_empty_state_text(id, visual, computed, Some(width), &mut shaper);
                    validate_text_metrics(id, intrinsic.title)?;
                    if let Some(message) = intrinsic.message {
                        validate_text_metrics(id, message)?;
                    }
                    // The shaped block is republished even when the metrics
                    // are unchanged: it lives in `NodeStyle`, which
                    // `EmptyState::project` rewrites from its own static
                    // style, so an unrelated re-projection can drop it.
                    empty_shaped.push((id, intrinsic));
                }
                continue;
            }
            if let Some(visual @ StandardVisual::ModalFrame { kind, slots, .. }) =
                self.world.get::<StandardVisual>(self.entities[&id])
            {
                if computed.visible {
                    let root = *self.component::<LayoutBox>(id);
                    let surface = crate::overlay_surfaces::modal_surface_bounds(root, *kind, None);
                    let chrome = crate::overlay_surfaces::ModalChrome::measure(
                        *kind,
                        crate::TextMetrics::default(),
                        None,
                        slots.close_action.is_some(),
                        slots.footer.is_some() || !slots.actions.is_empty(),
                    );
                    let wrap_width =
                        chrome.text_width(surface.width, *kind, slots.close_action.is_some());
                    let intrinsic =
                        shape_modal_text(id, visual, computed, Some(wrap_width), &mut shaper);
                    validate_text_metrics(id, intrinsic.title)?;
                    if let Some(description) = intrinsic.description {
                        validate_text_metrics(id, description)?;
                    }
                    if let Some(body) = intrinsic.body {
                        validate_text_metrics(id, body)?;
                    }
                    if self.world.get::<ModalTextPresentation>(self.entities[&id])
                        != Some(&intrinsic)
                    {
                        modal_shaped.push((id, intrinsic));
                    }
                }
                continue;
            }
            if text.value.is_empty() || !computed.visible {
                continue;
            }
            let constraints = self.text_shape_constraints(id);
            let metrics = shaper.shape(id, &text, computed, constraints);
            validate_text_metrics(id, metrics)?;
            let presentation = presentation.map(|source| {
                shape_text_input_presentation(id, source, computed, constraints, &mut shaper)
            });
            if *self.component::<TextMetrics>(id) != metrics
                || presentation.as_ref().is_some_and(|value| {
                    self.world.get::<TextInputPresentation>(self.entities[&id]) != Some(value)
                })
            {
                shaped.push((id, metrics, presentation));
            }
        }
        let mut changed = !shaped.is_empty() || !modal_shaped.is_empty();
        for (id, metrics, presentation) in shaped {
            *self.component_mut::<TextMetrics>(id) = metrics;
            if let Some(presentation) = presentation {
                self.world
                    .entity_mut(self.entities[&id])
                    .insert(presentation);
            }
        }
        for (id, presentation) in empty_shaped {
            changed |= self.apply_empty_state_text_presentation(id, presentation);
        }
        for (id, presentation) in modal_shaped {
            self.world
                .entity_mut(self.entities[&id])
                .insert(presentation);
            self.mark(id, DirtyMask::LAYOUT | DirtyMask::RENDER);
        }
        let runs = shaper.runs;
        let wrap_layouts = shaper.wrap_layouts;
        let _shaper = shaper;
        let (hits, misses, evictions) = cache.take_counters();
        let glyph_stats = glyphs.take_counters();
        self.text_layout_cache = cache;
        self.glyph_cache = glyphs;
        self.bump_last_counters(|counters| {
            counters.record_text_shape(runs, hits, misses, wrap_layouts);
            counters.record_cache_eviction(evictions);
            if let Some((glyph_hits, glyph_misses)) = glyph_stats {
                counters.record_glyph_cache(glyph_hits, glyph_misses);
            }
        });
        Ok(changed)
    }

    /// Publishes the shaped title/message block and reports whether anything
    /// changed, so a pass that only re-confirms the current block stays idle.
    fn apply_empty_state_text_presentation(
        &mut self,
        id: StableNodeId,
        presentation: EmptyStateTextPresentation,
    ) -> bool {
        let mut changed = false;
        if self
            .world
            .get::<EmptyStateTextPresentation>(self.entities[&id])
            != Some(&presentation)
        {
            self.world
                .entity_mut(self.entities[&id])
                .insert(presentation);
            changed = true;
        }
        let Some(StandardVisual::EmptyState {
            icon,
            compact,
            action,
            ..
        }) = self.world.get::<StandardVisual>(self.entities[&id])
        else {
            return changed;
        };
        let spacing = if *compact { 2.0 } else { 6.0 };
        let vertical = if *compact { 8.0 } else { 24.0 };
        let mut height = presentation.title.height;
        if icon.is_some() {
            height += 22.0 + spacing;
        }
        if let Some(message) = presentation.message {
            height += spacing + message.height;
        }
        if action.is_some() {
            height += spacing + 4.0;
        }
        let padding_top = nana_ui_core::LengthSpec::Px(vertical + height);
        let mut style = self.component::<NodeStyle>(id).clone();
        if style.layout.padding_top != Some(padding_top) {
            Arc::make_mut(&mut style.layout).padding_top = Some(padding_top);
            *self.component_mut::<NodeStyle>(id) = style;
            self.mark(id, DirtyMask::LAYOUT | DirtyMask::RENDER);
            if let Some(parent) = self.node(id).and_then(|node| node.parent) {
                self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
            }
            changed = true;
        }
        changed
    }

    fn text_input_presentation_source(
        &self,
        id: StableNodeId,
    ) -> Option<TextInputPresentationSource> {
        let StandardVisual::TextInput {
            placeholder,
            secure,
            ..
        } = self.world.get::<StandardVisual>(self.entities[&id])?
        else {
            return None;
        };
        let state = self.world.get::<TextInputState>(self.entities[&id])?;
        let ime = self.world.get::<ImeComposition>(self.entities[&id]);
        let multiline = self
            .world
            .get::<AccessibilityState>(self.entities[&id])
            .is_some_and(|state| state.multiline);
        Some(build_text_input_presentation_source(
            state,
            ime,
            placeholder,
            *secure,
            multiline,
        ))
    }

    fn text_shaping(&self, id: StableNodeId) -> crate::TextShaping {
        if self
            .world
            .get::<TextInputState>(self.entities[&id])
            .is_some()
        {
            crate::TextShaping::Advanced
        } else {
            crate::TextShaping::Auto
        }
    }

    /// Shape against the last published content box when it exists so wrap
    /// height can stop or propagate LAYOUT. Unmeasured nodes stay unconstrained.
    fn text_shape_constraints(&self, id: StableNodeId) -> crate::TextShapeConstraints {
        let source = self.component::<NodeStyle>(id);
        let layout = *self.component::<LayoutBox>(id);
        let presentation = self.text_input_presentation_source(id);
        let text_input_multiline = presentation.as_ref().is_some_and(|source| source.multiline);
        let is_text_input = presentation.is_some();
        let wrap = if is_text_input {
            text_input_multiline && source.layout.text_wraps()
        } else {
            source.layout.text_wraps()
        };
        let preserve_lines = source.layout.white_space.preserve_newlines();
        let wrap_break = source.layout.text_wrap_break();
        let ellipsis = !is_text_input && source.layout.uses_text_ellipsis();
        let max_lines = (!is_text_input)
            .then(|| source.layout.resolved_line_clamp())
            .flatten();
        let measured = layout.width > 0.0 || layout.height > 0.0;
        if !measured {
            return crate::TextShapeConstraints {
                wrap,
                ellipsis,
                max_lines,
                shaping: self.text_shaping(id),
                preserve_lines,
                wrap_break,
                ..crate::TextShapeConstraints::default()
            };
        }
        let padding = source.layout.resolved_padding_against(Some(layout.width));
        let border = source.layout.resolved_border_edges();
        let leading_visual = match self.world.get::<StandardVisual>(self.entities[&id]) {
            Some(StandardVisual::Checkbox { .. }) => 24.0,
            Some(StandardVisual::Switch { .. }) => 38.0,
            _ => 0.0,
        };
        crate::TextShapeConstraints {
            max_width: if is_text_input && !text_input_multiline {
                None
            } else {
                Some(
                    (layout.width
                        - padding.left
                        - padding.right
                        - border.left
                        - border.right
                        - leading_visual)
                        .max(0.0),
                )
            },
            max_height: (!is_text_input
                && (source
                    .layout
                    .height
                    .is_some_and(nana_ui_core::LengthSpec::is_definite_declared)
                    || source
                        .layout
                        .max_height
                        .is_some_and(nana_ui_core::LengthSpec::is_definite_declared)))
            .then(|| {
                (layout.height - padding.top - padding.bottom - border.top - border.bottom).max(0.0)
            }),
            wrap,
            ellipsis,
            max_lines,
            shaping: self.text_shaping(id),
            preserve_lines,
            wrap_break,
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
                let hierarchy = self.component::<Hierarchy>(id);
                let has_text = matches!(self.component::<Kind>(id).0.as_ref(), NodeKind::Text)
                    || !self.component::<TextContent>(id).value.is_empty();
                Ok(LayoutInput {
                    id,
                    parent: hierarchy.parent,
                    children: Arc::clone(&hierarchy.children),
                    style: self.effective_layout_style(id),
                    text_metrics: has_text.then(|| *self.component::<TextMetrics>(id)),
                    modal: self
                        .world
                        .get::<StandardVisual>(self.entities[&id])
                        .and_then(|visual| {
                            let StandardVisual::ModalFrame { kind, slots, .. } = visual else {
                                return None;
                            };
                            let presentation = self
                                .world
                                .get::<ModalTextPresentation>(self.entities[&id])
                                .copied()
                                .unwrap_or_default();
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

    /// Layout-facing style without assembling a [`LayoutInput`].
    ///
    /// Parked, detached, and inactive-overlay nodes match [`Self::layout_inputs`]:
    /// the returned style reports [`nana_ui_core::LayoutStyle::omits_box`].
    pub(crate) fn layout_style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        self.contains(id).then(|| self.effective_layout_style(id))
    }

    fn effective_layout_style(&self, id: StableNodeId) -> Arc<nana_ui_core::LayoutStyle> {
        let mut style = Arc::clone(&self.component::<NodeStyle>(id).layout);
        if style.omits_box()
            || !self.presence_live(id)
            || !self.overlay_branch_active(id)
            || !self.menu_branch_open(id)
        {
            Arc::make_mut(&mut style).hidden = true;
        }
        style
    }

    /// Rebuild one document's event-time hit-test tree after scheduled input
    /// or layout work. Pointer dispatch walks that tree in z/order with clip
    /// early-out instead of flattening and sorting every query.
    pub fn rebuild_hit_test(&mut self, document: DocumentId) {
        let roots = self.document_roots(document);
        let seeds = roots
            .iter()
            .copied()
            .map(|id| (id, IDENTITY_AFFINE))
            .collect::<Vec<_>>();
        let mut forest = self.build_hit_forest(seeds);
        for root in &mut forest {
            sort_hit_children(root);
        }
        forest.sort_by_key(|entry| (entry.z_index, entry.order));
        self.note_hit_nodes_built(&forest);
        self.hit_test_index.insert(document, forest);
    }

    /// Rebuild only the subtrees covering `dirty` instead of the whole document.
    ///
    /// Returns `false` when the change cannot be expressed as a local splice and
    /// the caller must fall back to [`Self::rebuild_hit_test`]. Structural cases
    /// that escalate: no existing index for the document, a dirty node whose
    /// parent chain is not represented in the index while still being live, and
    /// a dirty document root that changed root membership.
    pub fn rebuild_hit_test_scoped(
        &mut self,
        document: DocumentId,
        dirty: &[StableNodeId],
    ) -> bool {
        if dirty.is_empty() {
            return true;
        }
        if !self.hit_test_index.contains_key(&document) {
            return false;
        }
        let Some(roots) = self.minimal_hit_patch_roots(document, dirty) else {
            return false;
        };
        for root in roots {
            if !self.patch_hit_subtree(document, root) {
                return false;
            }
        }
        true
    }

    /// Reduce `dirty` to the shallowest nodes that cover it, dropping any node
    /// that already has a dirty ancestor. `None` means a dirty node belongs to
    /// another document and the caller should not attempt a scoped patch.
    fn minimal_hit_patch_roots(
        &self,
        document: DocumentId,
        dirty: &[StableNodeId],
    ) -> Option<Vec<StableNodeId>> {
        let mut pending = HashSet::new();
        for &id in dirty {
            if !self.contains(id) {
                // A despawned node was already spliced out by `retain_hit_tree`.
                continue;
            }
            if self.component::<Identity>(id).document != document {
                return None;
            }
            pending.insert(id);
        }
        let mut roots = Vec::new();
        for &id in &pending {
            let mut cursor = self.parent_id(id);
            let mut covered = false;
            while let Some(ancestor) = cursor {
                if pending.contains(&ancestor) {
                    covered = true;
                    break;
                }
                cursor = self.parent_id(ancestor);
            }
            if !covered {
                roots.push(id);
            }
        }
        roots.sort_unstable();
        Some(roots)
    }

    /// Replace `root`'s entry (and its descendants) in place. Reuses the parent
    /// entry's accumulated transform so no walk from the document root is
    /// needed. Returns `false` when the splice point is missing and the caller
    /// must rebuild the document.
    fn patch_hit_subtree(&mut self, document: DocumentId, root: StableNodeId) -> bool {
        let parent = self.parent_id(root);
        let Some(parent) = parent else {
            // Document roots: membership is owned by `live_document_roots`, so a
            // root entering or leaving the set is a structural change.
            let roots = self.document_roots(document);
            let present = self
                .hit_test_index
                .get(&document)
                .is_some_and(|forest| forest.iter().any(|entry| entry.id == root));
            let expected = roots.contains(&root) && self.node_hit_visible(root);
            if present != expected {
                return false;
            }
            if !expected {
                return true;
            }
            let position = roots.iter().position(|id| *id == root).unwrap_or_default();
            let mut rebuilt = self.build_hit_forest(vec![(root, IDENTITY_AFFINE)]);
            let Some(mut entry) = rebuilt.pop() else {
                return false;
            };
            entry.order = position;
            sort_hit_children(&mut entry);
            self.note_hit_nodes_built(std::slice::from_ref(&entry));
            let Some(forest) = self.hit_test_index.get_mut(&document) else {
                return false;
            };
            if let Some(slot) = forest.iter_mut().find(|slot| slot.id == root) {
                *slot = entry;
            } else {
                forest.push(entry);
            }
            forest.sort_by_key(|entry| (entry.z_index, entry.order));
            return true;
        };

        // The parent's own entry supplies the inherited transform. Its scroll is
        // read live because scroll never invalidates the parent entry itself.
        let Some(parent_transform) = self
            .hit_test_index
            .get(&document)
            .and_then(|forest| find_hit_transform(forest, parent))
        else {
            // Parent is absent from the index. That is correct only when the
            // parent is itself not hit-visible; otherwise the index is stale.
            return !self.node_hit_visible(parent);
        };
        let scroll = *self.component::<ScrollOffset>(parent);
        let child_transform =
            then_affine(parent_transform, [1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y]);
        let siblings = Arc::clone(&self.component::<Hierarchy>(parent).children);
        let position = siblings.iter().position(|id| *id == root);
        let Some(position) = position else {
            return false;
        };
        let mut rebuilt = self.build_hit_forest(vec![(root, child_transform)]);
        let entry = rebuilt.pop().map(|mut entry| {
            entry.order = position;
            sort_hit_children(&mut entry);
            entry
        });
        if let Some(entry) = entry.as_ref() {
            self.note_hit_nodes_built(std::slice::from_ref(entry));
        }
        let Some(forest) = self.hit_test_index.get_mut(&document) else {
            return false;
        };
        let Some(parent_entry) = find_hit_entry_mut(forest, parent) else {
            return false;
        };
        match entry {
            Some(entry) => {
                if let Some(slot) = parent_entry
                    .children
                    .iter_mut()
                    .find(|slot| slot.id == root)
                {
                    *slot = entry;
                } else {
                    parent_entry.children.push(entry);
                }
            }
            // The subtree turned invisible: drop it from the parent.
            None => parent_entry.children.retain(|slot| slot.id != root),
        }
        parent_entry
            .children
            .sort_by_key(|child| (child.z_index, child.order));
        true
    }

    /// Whether `id` would contribute an entry to the hit index.
    fn node_hit_visible(&self, id: StableNodeId) -> bool {
        self.contains(id) && self.component::<ResolvedStyle>(id).0.visible
    }

    /// Every build path funnels through here, so recording once keeps the
    /// sentinel honest for both a scoped patch and a full rebuild.
    fn note_hit_nodes_built(&mut self, forest: &[HitEntry]) {
        let built = forest.iter().map(count_hit_entries).sum::<usize>();
        self.bump_last_counters(|counters| counters.record_hit_test_rebuild(built));
    }

    /// Build hit entries for `seeds` and their visible descendants. Each seed
    /// carries the accumulated transform of its parent, so a scoped patch can
    /// resume from an existing entry instead of walking from the document root.
    ///
    /// `order` is assigned per sibling group from `Hierarchy` position. It is
    /// only ever compared between siblings, so a spliced subtree stays sortable
    /// against untouched siblings without renumbering the document.
    fn build_hit_forest(&self, seeds: Vec<(StableNodeId, [f32; 6])>) -> Vec<HitEntry> {
        struct Built {
            entry: HitEntry,
            parent: Option<usize>,
        }
        let mut stack = seeds
            .into_iter()
            .enumerate()
            .rev()
            .map(|(position, (id, transform))| {
                (
                    id,
                    (transform, [0.0f32, 0.0]),
                    None::<usize>,
                    position,
                    self.parent_used_pointer_events(id),
                    false,
                )
            })
            .collect::<Vec<_>>();
        let mut built: Vec<Built> = Vec::new();
        let mut memo = AncestorMemo::default();
        while let Some((id, parent_hit, parent, position, parent_used_pe, parent_blocks_3d)) =
            stack.pop()
        {
            let style = self.component::<ResolvedStyle>(id).0.as_ref();
            let layout = *self.component::<LayoutBox>(id);
            let node_style = self.component::<NodeStyle>(id).layout.as_ref();
            let local = if parent_blocks_3d && node_style.transform_3d.is_some() {
                (IDENTITY_AFFINE, [0.0, 0.0])
            } else {
                node_style
                    .world_scene_transform(layout.x, layout.y, layout.width, layout.height)
                    .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
            };
            let (transform, persp) = then_hit(parent_hit, local);
            let scroll = *self.component::<ScrollOffset>(id);
            let child_transform = then_hit(
                (transform, persp),
                ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
            );
            let child_blocks_3d = parent_blocks_3d || node_style.fails_closed_3d_context();
            let children = Arc::clone(&self.component::<Hierarchy>(id).children);
            let used_pe =
                PointerEventsSpec::inherit_from(node_style.pointer_events, parent_used_pe);
            if !style.visible {
                // `visibility:hidden` skips this entry but descendants may be
                // `visibility:visible` and still need the accumulated transform.
                stack.extend(children.iter().enumerate().rev().map(|(position, child)| {
                    (
                        *child,
                        child_transform,
                        parent,
                        position,
                        used_pe,
                        child_blocks_3d,
                    )
                }));
                continue;
            }
            let mut self_clips = Vec::new();
            let mut child_clips = Vec::new();
            if let Some((x, y, w, h)) =
                node_style.overflow_clip_box(layout.x, layout.y, layout.width, layout.height)
            {
                child_clips.push((
                    LayoutBox {
                        x,
                        y,
                        width: w,
                        height: h,
                    },
                    transform,
                ));
            }
            if self.clip_visuals != 0 {
                if matches!(
                    self.world.get::<StandardVisual>(self.entities[&id]),
                    Some(StandardVisual::EmptyState { .. })
                ) {
                    child_clips.push((layout, transform));
                }
                if let Some(crate::ComponentGeometry::ModalFrame { surface, .. }) =
                    self.component_geometry(id)
                {
                    child_clips.push((surface, transform));
                }
                if let Some(parent_id) = self.parent_id(id)
                    && let Some(StandardVisual::ModalFrame { slots, .. }) =
                        self.world.get::<StandardVisual>(self.entities[&parent_id])
                    && slots.body == Some(id)
                    && let Some(crate::ComponentGeometry::ModalFrame { body, .. }) =
                        self.component_geometry(parent_id)
                {
                    self_clips.push((body, parent_hit.0));
                }
            }
            let interaction = self.component::<InteractionState>(id);
            let confirm_busy = self
                .confirm_action_effect(id)
                .is_some_and(|effect| effect.0);
            let hittable = interaction.pointer_events && used_pe.hittable() && !confirm_busy;
            let menu = hittable
                .then(|| self.component_geometry(id))
                .flatten()
                .and_then(|geometry| match geometry {
                    crate::ComponentGeometry::Select {
                        menu: Some(menu), ..
                    } => Some(menu.surface),
                    _ => None,
                });
            let index = built.len();
            built.push(Built {
                entry: HitEntry {
                    id,
                    layout,
                    transform,
                    persp,
                    self_clips,
                    child_clips,
                    z_index: self.stacking_z_index_memo(id, &mut memo),
                    order: position,
                    hittable,
                    menu,
                    children: Vec::new(),
                },
                parent,
            });
            // Sibling position is the sort key. Invisible siblings are skipped
            // and leave gaps, which is harmless because only relative order
            // between surviving siblings is ever compared.
            stack.extend(children.iter().enumerate().rev().map(|(position, child)| {
                (
                    *child,
                    child_transform,
                    Some(index),
                    position,
                    used_pe,
                    child_blocks_3d,
                )
            }));
        }
        let n = built.len();
        let mut parent_of = Vec::with_capacity(n);
        let mut entries = Vec::with_capacity(n);
        for node in built {
            parent_of.push(node.parent);
            entries.push(Some(node.entry));
        }
        for i in (0..n).rev() {
            if let Some(parent) = parent_of[i] {
                let child = entries[i].take().expect("child hit node");
                entries[parent]
                    .as_mut()
                    .expect("parent hit node")
                    .children
                    .push(child);
            }
        }
        entries.into_iter().flatten().collect::<Vec<_>>()
    }

    /// Drain the scroll deltas recorded since the last drain.
    pub fn take_scroll_hit_updates(&mut self) -> Vec<(StableNodeId, [f32; 2])> {
        std::mem::take(&mut self.scroll_hit_updates)
    }

    /// Whether every input-dirty node is explained by recorded scroll deltas
    /// (it is a scroller or descends from one). When true, the frame driver
    /// can patch the hit index in place instead of rebuilding the document.
    pub fn hit_test_work_is_scroll_only(
        &self,
        input: &[StableNodeId],
        updates: &[(StableNodeId, [f32; 2])],
    ) -> bool {
        !updates.is_empty()
            && input.iter().all(|node| {
                updates.iter().any(|(scroller, _)| {
                    *scroller == *node || {
                        let mut cursor = self.parent_id(*node);
                        while let Some(ancestor) = cursor {
                            if ancestor == *scroller {
                                return true;
                            }
                            cursor = self.parent_id(ancestor);
                        }
                        false
                    }
                })
            })
    }

    /// Pre-compose a scroll translation onto descendant hit entries of
    /// `scroller`. The scroller chrome stays un-scrolled — rebuild applies
    /// scroll only when walking children. Equivalent to a rebuild because
    /// scroll changes nothing else about the entries (membership, order,
    /// z-index, and clips are scroll-invariant: the scroller's own clip never
    /// includes its scroll offset).
    pub fn update_hit_test_scroll(
        &mut self,
        document: DocumentId,
        scroller: StableNodeId,
        delta: [f32; 2],
    ) {
        let mut subtree = vec![scroller];
        let mut index = 0;
        while index < subtree.len() {
            let id = subtree[index];
            index += 1;
            subtree.extend(self.component::<Hierarchy>(id).children.iter().copied());
        }
        let subtree = subtree
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let Some(entries) = self.hit_test_index.get_mut(&document) else {
            return;
        };
        patch_hit_scroll(entries, scroller, &subtree, delta);
    }

    /// Topmost hit at `(x, y)`.
    ///
    /// Walks the same order as [`Self::hit_test_candidates`] but returns at the
    /// first hit, so pointer dispatch on every move does not collect and then
    /// discard the full candidate list.
    pub fn hit_test(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        let forest = self.hit_test_index.get(&document)?;
        forest
            .iter()
            .rev()
            .find_map(|node| first_hit_candidate(node, x, y))
    }

    pub fn hit_test_candidates(&self, document: DocumentId, x: f32, y: f32) -> Vec<StableNodeId> {
        let Some(forest) = self.hit_test_index.get(&document) else {
            return Vec::new();
        };
        let mut candidates = Vec::new();
        for node in forest.iter().rev() {
            collect_hit_candidates(node, x, y, &mut candidates);
        }
        candidates
    }

    /// Produce a renderer-neutral snapshot in retained document order.
    pub fn extract_document(&self, document: DocumentId) -> Vec<ExtractedNode> {
        let mut memo = AncestorMemo::default();
        self.document_order(document)
            .into_iter()
            .filter_map(|id| {
                self.extract_node_memo(id, &mut memo)
                    .filter(|node| node.style.visible)
            })
            .collect()
    }

    /// Extract only dirty nodes. Hidden nodes stay present with `visible=false`
    /// so an incremental renderer can remove their previous primitives.
    pub fn extract_nodes(&self, ids: &[StableNodeId]) -> Vec<ExtractedNode> {
        let mut memo = AncestorMemo::default();
        ids.iter()
            .filter_map(|&id| self.extract_node_memo(id, &mut memo))
            .collect()
    }

    pub fn commit(&mut self, queue: MutationQueue) -> Result<CommitReport, UiWorldError> {
        self.commit_ref(&queue)
    }

    /// Borrowing variant of [`commit`]: validate-then-apply against a queue
    /// the caller still owns. Validation runs fully before the apply loop, so
    /// a rejected batch never lands partially and the caller may replay it.
    pub fn commit_ref(&mut self, queue: &MutationQueue) -> Result<CommitReport, UiWorldError> {
        let mut report = CommitReport {
            generation: self.generation,
            mutations: queue.len(),
            created: 0,
            inserted: 0,
            detached: 0,
            reparented: 0,
            despawned: 0,
        };
        if queue.is_empty() {
            return Ok(report);
        }
        let mut scanned = 0;
        let mut validated = Ok(());
        if !self.validate_simple_mutation(queue.as_slice())? {
            let mut plan = ValidationPlan::new(self);
            validated = plan.validate(queue.as_slice());
            scanned = plan.scanned;
        }
        self.validation_nodes_scanned = self.validation_nodes_scanned.saturating_add(scanned);
        validated?;
        self.generation = self.generation.wrapping_add(1);
        report.generation = self.generation;
        for mutation in queue.as_slice() {
            self.apply(mutation, &mut report);
        }
        Ok(report)
    }

    /// Single-node creation and detached append have no staged cross-mutation
    /// state to simulate. Validate them directly so retained DOM/component
    /// construction does not scan the world or clone a growing child list.
    fn validate_simple_mutation(&self, mutations: &[UiMutation]) -> Result<bool, UiWorldError> {
        if let [UiMutation::Create { id, .. }] = mutations {
            if self.contains(*id) {
                return Err(UiWorldError::DuplicateNode(*id));
            }
            if self.is_retired(*id) {
                return Err(UiWorldError::RetiredNode(*id));
            }
            return Ok(true);
        }
        let [
            UiMutation::Insert {
                parent,
                child,
                before: None,
            },
        ] = mutations
        else {
            return Ok(false);
        };
        let (parent_document, _) = self.identity_and_parent(*parent)?;
        let (child_document, child_parent) = self.identity_and_parent(*child)?;
        if child_parent.is_some() {
            return Ok(false);
        }
        if parent_document != child_document {
            return Err(UiWorldError::CrossDocument {
                parent: *parent,
                child: *child,
            });
        }
        let mut ancestor = Some(*parent);
        let mut depth = 0usize;
        while let Some(id) = ancestor {
            if id == *child {
                return Err(UiWorldError::Cycle {
                    parent: *parent,
                    child: *child,
                });
            }
            depth += 1;
            ancestor = self.identity_and_parent(id)?.1;
        }
        // Near the depth limit the child's own height decides, and measuring it
        // is exactly the walk this fast path exists to avoid. Hand those rare
        // batches to the planner, which already bounds depth.
        Ok(depth < MAX_TREE_DEPTH)
    }

    fn identity_and_parent(
        &self,
        id: StableNodeId,
    ) -> Result<(DocumentId, Option<StableNodeId>), UiWorldError> {
        let entity = *self
            .entities
            .get(&id)
            .ok_or(UiWorldError::MissingNode(id))?;
        let identity = self
            .world
            .get::<Identity>(entity)
            .ok_or(UiWorldError::MissingNode(id))?;
        let hierarchy = self
            .world
            .get::<Hierarchy>(entity)
            .ok_or(UiWorldError::MissingNode(id))?;
        Ok((identity.document, hierarchy.parent))
    }

    fn apply(&mut self, mutation: &UiMutation, report: &mut CommitReport) {
        match mutation {
            UiMutation::Create { id, document, kind } => {
                let entity = self
                    .world
                    .spawn((
                        Identity {
                            stable: *id,
                            document: *document,
                        },
                        Kind(intern_kind(kind)),
                        Hierarchy::default(),
                        MountState::default(),
                        NodeStyle::default(),
                        ResolvedStyle(Arc::clone(&INTERNED_DEFAULT_STYLE), 0),
                        TextContent::default(),
                        TextMetrics::default(),
                        LayoutBox::default(),
                        ScrollOffset::default(),
                        initial_interaction(kind),
                        AccessibilityState::default(),
                        DirtyMask::all(),
                    ))
                    .id();
                self.entities.insert(*id, entity);
                self.dirty_entities.insert(*id);
                self.spawned_since_drain += 1;
                report.created += 1;
                self.refresh_root_membership(*id);
            }
            UiMutation::Insert {
                parent,
                child,
                before,
            } => {
                let old_parent = self
                    .identity_and_parent(*child)
                    .expect("validated child must exist")
                    .1;
                if old_parent == Some(*parent) && *before == Some(*child) {
                    return;
                }
                if let Some(old_parent) = old_parent {
                    let mut hierarchy = self.hierarchy_mut(old_parent);
                    Arc::make_mut(&mut hierarchy.children).retain(|id| id != child);
                    intern_empty_children(&mut hierarchy.children);
                }
                let mut parent_hierarchy = self.hierarchy_mut(*parent);
                let siblings = Arc::make_mut(&mut parent_hierarchy.children);
                let index = before
                    .and_then(|before| siblings.iter().position(|id| *id == before))
                    .unwrap_or(siblings.len());
                siblings.insert(index, *child);
                let _parent_hierarchy = parent_hierarchy;
                self.hierarchy_mut(*child).parent = Some(*parent);
                let parent_mount = *self.component::<MountState>(*parent);
                if *self.component::<MountState>(*child) != parent_mount {
                    self.set_subtree_mount_state(*child, parent_mount);
                }
                if old_parent == Some(*parent) {
                    // Retained-order moves carry the entire subtree; descendants
                    // keep their inherited state and local geometry until layout
                    // writeback identifies actual changes.
                    self.mark(
                        *child,
                        DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                } else {
                    self.mark_subtree(*child, DirtyMask::ALL);
                }
                self.mark_ancestors(
                    *parent,
                    DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
                if let Some(old_parent) = old_parent {
                    self.mark_ancestors(
                        old_parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
                if old_parent.is_some() {
                    report.reparented += 1;
                } else {
                    report.inserted += 1;
                }
                self.detached.remove(child);
                self.sync_subtree_presence(*child);
                self.refresh_root_membership(*child);
            }
            UiMutation::Detach { id } => {
                if self.unlink_from_parent(*id) {
                    report.detached += 1;
                }
                self.leave_live_document(*id);
                self.refresh_root_membership(*id);
            }
            UiMutation::ParkSubtree { root } => {
                self.unlink_from_parent(*root);
                self.set_subtree_mount_state(*root, MountState::Parked);
                self.leave_live_document(*root);
                self.refresh_root_membership(*root);
            }
            UiMutation::DespawnSubtree { root } => {
                let root_snapshot = self.node(*root).expect("validated root must exist");
                if let Some(parent) = root_snapshot.parent {
                    let mut hierarchy = self.hierarchy_mut(parent);
                    Arc::make_mut(&mut hierarchy.children).retain(|child| child != root);
                    intern_empty_children(&mut hierarchy.children);
                    let _hierarchy = hierarchy;
                    self.mark_ancestors(
                        parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
                let mut stack = vec![*root];
                while let Some(id) = stack.pop() {
                    let snapshot = self.node(id).expect("validated subtree must exist");
                    stack.extend(snapshot.children.iter().rev().copied());
                    let entity = self
                        .entities
                        .remove(&id)
                        .expect("entity index must contain node");
                    self.dirty_entities.remove(&id);
                    self.refresh_root_membership(id);
                    if self.focused.get(&snapshot.document) == Some(&id) {
                        self.focused.remove(&snapshot.document);
                    }
                    if let Some(index) = self.hit_test_index.get_mut(&snapshot.document) {
                        retain_hit_tree(index, id);
                    }
                    let released = self
                        .pointer_captures
                        .iter()
                        .filter_map(|(&(document, pointer_id), &target)| {
                            (target == id).then_some((document, pointer_id))
                        })
                        .collect::<Vec<_>>();
                    for key @ (_, pointer_id) in released {
                        self.pointer_captures.remove(&key);
                        self.pending_pointer_capture_changes
                            .push(PointerCaptureChange {
                                pointer_id,
                                target: id,
                                captured: false,
                            });
                    }
                    self.pointer_hover.retain(|_, target| *target != id);
                    self.pointer_press.retain(|_, target| *target != id);
                    let cancelled = self
                        .animations
                        .iter()
                        .filter_map(|(&animation_id, animation)| {
                            (animation.spec.target == id)
                                .then_some((animation_id, animation.next_deadline))
                        })
                        .collect::<Vec<_>>();
                    for (animation_id, deadline) in cancelled {
                        self.animations.remove(&animation_id);
                        self.animation_deadlines.remove(&(deadline, animation_id));
                    }
                    self.clear_overlay_references(id);
                    self.overlay_host_nodes.remove(&id);
                    self.detached.remove(&id);
                    self.forget_visual_presence(entity);
                    let _ = self.world.despawn(entity);
                    self.retired.insert(id);
                    self.pending_render_removals.push(id);
                    self.pending_accessibility_removals.push(id);
                    self.despawned_since_drain += 1;
                    report.despawned += 1;
                }
                self.refresh_root_membership(*root);
            }
            UiMutation::SetStyle { id, style } => {
                let previous = self.component::<NodeStyle>(*id).clone();
                let inherited_text_changed = previous.layout.font_size != style.layout.font_size
                    || previous.layout.font_weight != style.layout.font_weight
                    || previous.layout.font_italic != style.layout.font_italic
                    || previous.layout.font_family != style.layout.font_family
                    || previous.layout.line_height != style.layout.line_height
                    || previous.layout.letter_spacing != style.layout.letter_spacing;
                let inherited_paint_changed = previous.foreground != style.foreground
                    || previous.layout.color != style.layout.color
                    || previous.layout.opacity != style.layout.opacity;
                let paint_visibility_changed =
                    previous.layout.paint.visibility != style.layout.paint.visibility;
                let pointer_events_changed =
                    previous.layout.pointer_events != style.layout.pointer_events;
                let omits_box_changed = previous.layout.omits_box() != style.layout.omits_box();
                let transform_changed = previous.layout.transform != style.layout.transform
                    || previous.layout.transform_3d != style.layout.transform_3d
                    || previous.layout.transform_origin != style.layout.transform_origin
                    || previous.layout.transform_box != style.layout.transform_box
                    || previous.layout.css_perspective != style.layout.css_perspective
                    || previous.layout.preserve_3d != style.layout.preserve_3d
                    || previous.layout.unsupported_transform != style.layout.unsupported_transform;
                let stacking_changed = previous.layout.z_index != style.layout.z_index
                    || previous.layout.isolation != style.layout.isolation;
                let layout_changed =
                    layout_semantics_changed(previous.layout.as_ref(), style.layout.as_ref());
                *self.component_mut::<NodeStyle>(*id) = style.clone();
                self.sync_node_presence(*id);

                if !style_excluding_transform_eq(&previous, style) {
                    self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
                if inherited_paint_changed {
                    self.mark_subtree(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
                if inherited_text_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::TEXT
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::RENDER,
                    );
                }
                if omits_box_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                    if let Some(parent) = self.node(*id).and_then(|node| node.parent) {
                        self.mark(parent, DirtyMask::ACCESSIBILITY);
                    }
                } else if paint_visibility_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                    if let Some(parent) = self.node(*id).and_then(|node| node.parent) {
                        self.mark(parent, DirtyMask::ACCESSIBILITY);
                    }
                }
                if pointer_events_changed {
                    // Inherited: unspecified descendants pick up the new used
                    // value. Not a layout dirty.
                    self.mark_subtree(*id, DirtyMask::STYLE | DirtyMask::INPUT);
                    self.clear_hover_for_pointer_events_none(*id);
                }
                if transform_changed {
                    // Scene extract and hit-test read `layout.transform`; LAYOUT
                    // does not, so paint-transform is not a layout dirty.
                    self.mark_subtree(
                        *id,
                        DirtyMask::TRANSFORM | DirtyMask::INPUT | DirtyMask::RENDER,
                    );
                } else if stacking_changed {
                    self.mark_subtree(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
                if layout_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
                if (layout_changed || inherited_text_changed || omits_box_changed)
                    && let Some(parent) = self.node(*id).and_then(|node| node.parent)
                {
                    self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetTheme { mode } => {
                if self.style_model.theme_mode != *mode {
                    let previous_metrics = self.style_model.metrics;
                    self.style_model = StyleModelRef::new(*mode);
                    self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
                    // Palette roles are resolved at extract from SemanticColorRole;
                    // skip STYLE so a theme swap does not restyle the live tree.
                    let mut bits = DirtyMask::RENDER;
                    if self.style_model.metrics != previous_metrics {
                        bits |= DirtyMask::LAYOUT;
                    }
                    let mut ids = Vec::new();
                    for roots in self.live_document_roots.values() {
                        for &root in roots {
                            ids.extend(self.subtree_ids(root));
                        }
                    }
                    for id in ids {
                        self.mark(id, bits);
                    }
                }
            }
            UiMutation::SetText { id, text } => {
                *self.component_mut::<TextContent>(*id) = text.clone();
                self.mark(
                    *id,
                    DirtyMask::TEXT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::WriteLayout { id, layout } => {
                *self.component_mut::<LayoutBox>(*id) = *layout;
                // Scoped layout already emits every recomputed box, including
                // shifted descendants. Mark only this node so a bit-identical
                // child is not extracted solely because an ancestor was written.
                self.mark(
                    *id,
                    DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetScrollOffset { id, offset } => {
                let offset = self.clamp_scroll_offset(*id, *offset);
                let previous = *self.component::<ScrollOffset>(*id);
                if previous != offset {
                    *self.component_mut::<ScrollOffset>(*id) = offset;
                    // Hit-index patch + Scene extract of this scroller only.
                    // Descendants keep LayoutBox; paint uses scroll_offset.
                    self.scroll_hit_updates
                        .push((*id, [previous.x - offset.x, previous.y - offset.y]));
                    self.mark(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetScrollMetrics { id, metrics } => {
                let entity = self.entities[id];
                if let Some(metrics) = metrics {
                    self.world.entity_mut(entity).insert(*metrics);
                } else {
                    self.world.entity_mut(entity).remove::<ScrollMetrics>();
                }
                let current = *self.component::<ScrollOffset>(*id);
                let clamped = self.clamp_scroll_offset(*id, current);
                if current != clamped {
                    *self.component_mut::<ScrollOffset>(*id) = clamped;
                    self.scroll_hit_updates
                        .push((*id, [current.x - clamped.x, current.y - clamped.y]));
                    self.mark(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetInteraction { id, interaction } => {
                *self.component_mut::<InteractionState>(*id) = *interaction;
                if !interaction.pointer_events {
                    self.pointer_hover.retain(|_, target| target != id);
                    self.pointer_press.retain(|_, target| target != id);
                }
                self.mark(
                    *id,
                    DirtyMask::STATE
                        | DirtyMask::INPUT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetCustomRender { id, content } => {
                let entity = self.entities[id];
                if let Some(content) = content {
                    self.world.entity_mut(entity).insert(content.clone());
                } else {
                    self.world.entity_mut(entity).remove::<CustomRenderNode>();
                }
                self.mark(*id, DirtyMask::RENDER);
            }
            UiMutation::SetEventListener { id, event, enabled } => {
                let entity = self.entities[id];
                let mut listeners = self
                    .world
                    .get::<EventListeners>(entity)
                    .cloned()
                    .unwrap_or_default();
                listeners.set(event.clone(), *enabled);
                if listeners.is_empty() {
                    self.world.entity_mut(entity).remove::<EventListeners>();
                } else {
                    self.world.entity_mut(entity).insert(listeners);
                }
            }
            UiMutation::SetComponentType { id, type_id } => {
                let entity = self.entities[id];
                let current = self.world.get::<ComponentTypeId>(entity);
                if current != type_id.as_ref() {
                    if let Some(type_id) = type_id {
                        self.world.entity_mut(entity).insert(type_id.clone());
                    } else {
                        self.world.entity_mut(entity).remove::<ComponentTypeId>();
                    }
                }
            }
            UiMutation::SetStandardVisual { id, visual } => {
                let entity = self.entities[id];
                let (
                    text_input_presentation_changed,
                    empty_state_presentation_changed,
                    modal_presentation_changed,
                    modal_state_changed,
                    menu_state_changed,
                ) = {
                    let previous_visual = self.world.get::<StandardVisual>(entity);
                    (
                        matches!(previous_visual, Some(StandardVisual::TextInput { .. }))
                            || matches!(visual, Some(StandardVisual::TextInput { .. })),
                        matches!(previous_visual, Some(StandardVisual::EmptyState { .. }))
                            || matches!(visual, Some(StandardVisual::EmptyState { .. })),
                        matches!(previous_visual, Some(StandardVisual::ModalFrame { .. }))
                            || matches!(visual, Some(StandardVisual::ModalFrame { .. })),
                        match (previous_visual, visual) {
                            (
                                Some(StandardVisual::ModalFrame {
                                    busy: old_busy,
                                    danger: old_danger,
                                    ..
                                }),
                                Some(StandardVisual::ModalFrame { busy, danger, .. }),
                            ) => old_busy != busy || old_danger != danger,
                            _ => false,
                        },
                        // Whether a menu's items take part in layout follows the
                        // surface's own open state, so opening it has to reach
                        // them the way an overlay host reaches its branch.
                        menu_surface_open(previous_visual) != menu_surface_open(visual.as_ref()),
                    )
                };
                if let Some(visual) = visual {
                    self.world.entity_mut(entity).insert(visual.clone());
                } else {
                    self.world.entity_mut(entity).remove::<StandardVisual>();
                }
                self.sync_node_presence(*id);
                if !matches!(visual, Some(StandardVisual::TextInput { .. })) {
                    self.world
                        .entity_mut(entity)
                        .remove::<TextInputPresentation>();
                }
                if !matches!(visual, Some(StandardVisual::EmptyState { .. })) {
                    self.world
                        .entity_mut(entity)
                        .remove::<EmptyStateTextPresentation>();
                }
                if !matches!(visual, Some(StandardVisual::ModalFrame { .. })) {
                    self.world
                        .entity_mut(entity)
                        .remove::<ModalTextPresentation>();
                }
                self.mark(
                    *id,
                    DirtyMask::RENDER
                        | if text_input_presentation_changed
                            || empty_state_presentation_changed
                            || modal_presentation_changed
                        {
                            DirtyMask::TEXT | DirtyMask::LAYOUT
                        } else {
                            0
                        },
                );
                if menu_state_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                    self.mark_ancestors(*id, DirtyMask::LAYOUT | DirtyMask::RENDER);
                }
                if modal_state_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
            }
            UiMutation::SetAccessibility { id, accessibility } => {
                let previous = self.component::<AccessibilityState>(*id);
                let interaction_style_changed = previous.disabled != accessibility.disabled
                    || previous.checked != accessibility.checked
                    || previous.selected != accessibility.selected;
                *self.component_mut::<AccessibilityState>(*id) = accessibility.clone();
                self.mark(*id, DirtyMask::ACCESSIBILITY);
                if interaction_style_changed
                    && !self.component::<NodeStyle>(*id).interaction.is_empty()
                {
                    self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
            }
            UiMutation::SetOverlayHost { host, state } => {
                let entity = self.entities[host];
                let previous = self.world.get::<OverlayHostState>(entity).copied();
                if previous == Some(*state) {
                    return;
                }
                self.world.entity_mut(entity).insert(*state);
                self.overlay_host_nodes.insert(*host);
                self.mark(*host, DirtyMask::ACCESSIBILITY);
                if let Some(inactive) = previous
                    .and_then(|previous| previous.active)
                    .filter(|active| Some(*active) != state.active)
                {
                    let mut inactive_nodes = HashSet::new();
                    let mut stack = vec![inactive];
                    while let Some(id) = stack.pop() {
                        stack.extend(self.component::<Hierarchy>(id).children.iter().copied());
                        inactive_nodes.insert(id);
                    }
                    let released = self
                        .pointer_captures
                        .iter()
                        .filter_map(|(&(document, pointer_id), &target)| {
                            inactive_nodes
                                .contains(&target)
                                .then_some((document, pointer_id, target))
                        })
                        .collect::<Vec<_>>();
                    for (document, pointer_id, target) in released {
                        self.pointer_captures.remove(&(document, pointer_id));
                        self.pending_pointer_capture_changes
                            .push(PointerCaptureChange {
                                pointer_id,
                                target,
                                captured: false,
                            });
                    }
                    self.pointer_hover
                        .retain(|_, target| !inactive_nodes.contains(target));
                    self.pointer_press
                        .retain(|_, target| !inactive_nodes.contains(target));
                }
                let changed_roots = previous
                    .and_then(|previous| previous.active)
                    .into_iter()
                    .chain(state.active)
                    .collect::<HashSet<_>>();
                for root in changed_roots {
                    self.mark_subtree(
                        root,
                        DirtyMask::STYLE
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
            }
            UiMutation::CapturePointer { pointer_id, target } => {
                let document = self.component::<Identity>(*target).document;
                let previous = self
                    .pointer_captures
                    .insert((document, *pointer_id), *target);
                if previous == Some(*target) {
                    return;
                }
                if let Some(previous) = previous {
                    self.pending_pointer_capture_changes
                        .push(PointerCaptureChange {
                            pointer_id: *pointer_id,
                            target: previous,
                            captured: false,
                        });
                }
                self.pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: true,
                    });
            }
            UiMutation::ReleasePointer { pointer_id, target } => {
                let document = self.component::<Identity>(*target).document;
                self.pointer_captures.remove(&(document, *pointer_id));
                self.pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: false,
                    });
            }
            UiMutation::StartAnimation { animation } => {
                let active = ActiveAnimation::new(*animation);
                if let Some(previous) = self.animations.insert(animation.id, active) {
                    self.animation_deadlines
                        .remove(&(previous.next_deadline, animation.id));
                }
                self.animation_deadlines
                    .insert((animation.start, animation.id));
            }
            UiMutation::StopAnimation { id } => {
                if let Some(animation) = self.animations.remove(id) {
                    self.animation_deadlines
                        .remove(&(animation.next_deadline, *id));
                }
            }
            UiMutation::RequestFocus { document, target } => {
                let old = match target {
                    Some(target) => self.focused.insert(*document, *target),
                    None => self.focused.remove(document),
                };
                if let Some(old) = old.filter(|old| Some(*old) != *target) {
                    self.remove_ime(old);
                    self.mark(old, DirtyMask::STATE);
                    if !self
                        .component::<NodeStyle>(old)
                        .interaction
                        .focused
                        .is_empty()
                    {
                        self.mark(old, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                    self.mark(
                        old,
                        DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
                if let Some(target) = target {
                    self.mark(*target, DirtyMask::STATE);
                    if !self
                        .component::<NodeStyle>(*target)
                        .interaction
                        .focused
                        .is_empty()
                    {
                        self.mark(*target, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                    self.mark(
                        *target,
                        DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
            }
            UiMutation::SetIme { id, composition } => {
                let entity = self.entities[id];
                if let Some(composition) = composition {
                    self.world.entity_mut(entity).insert(composition.clone());
                } else {
                    self.world.entity_mut(entity).remove::<ImeComposition>();
                }
                self.mark(
                    *id,
                    DirtyMask::TEXT | DirtyMask::FOCUS_IME | DirtyMask::RENDER,
                );
            }
            UiMutation::SetTextInput { id, state } => {
                let entity = self.entities[id];
                if let Some(state) = state {
                    self.world.entity_mut(entity).insert(state.clone());
                    *self.component_mut::<TextContent>(*id) = TextContent {
                        value: state.value.clone(),
                    };
                } else {
                    self.world.entity_mut(entity).remove::<TextInputState>();
                    *self.component_mut::<TextContent>(*id) = TextContent::default();
                    self.remove_ime(*id);
                }
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetTextSelection { id, selection } => {
                self.component_mut::<TextInputState>(*id).selection = *selection;
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::ReplaceTextSelection { id, text } => {
                let (replaced, value) = {
                    let mut state = self.component_mut::<TextInputState>(*id);
                    let replaced = state.replace_selection(text);
                    (replaced, state.value.clone())
                };
                debug_assert!(replaced, "validated selection must remain valid");
                *self.component_mut::<TextContent>(*id) = TextContent { value };
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetHighlightRequest { id, request } => {
                let entity = self.entities[id];
                if let Some(request) = request {
                    self.world.entity_mut(entity).insert(request.clone());
                } else {
                    self.world.entity_mut(entity).remove::<HighlightRequest>();
                    self.world.entity_mut(entity).remove::<TextPresentation>();
                }
                self.mark(*id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
        }
    }

    fn extracted_text_spans(&self, entity: Entity) -> Vec<ExtractedTextSpan> {
        if self.world.get::<ImeComposition>(entity).is_some() {
            return Vec::new();
        }
        if self
            .world
            .get::<TextInputPresentation>(entity)
            .is_some_and(|presentation| presentation.placeholder)
        {
            return Vec::new();
        }
        if matches!(
            self.world.get::<StandardVisual>(entity),
            Some(StandardVisual::TextInput { secure: true, .. })
        ) {
            return Vec::new();
        }
        let Some(presentation) = self.world.get::<TextPresentation>(entity) else {
            return Vec::new();
        };
        presentation
            .spans
            .iter()
            .map(|span| ExtractedTextSpan {
                start: span.start,
                end: span.end,
                color: self.style_model.palette.get(span.color).as_rgba_array(),
            })
            .collect()
    }

    fn component<T: Component>(&self, id: StableNodeId) -> &T {
        self.world
            .get::<T>(self.entities[&id])
            .expect("entity must have runtime component")
    }

    pub(crate) fn parent_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<Hierarchy>(entity)?.parent
    }

    fn live_input_target_count(&self) -> usize {
        let mut ids = HashSet::new();
        ids.extend(self.focused.values().copied());
        ids.extend(self.pointer_hover.values().copied());
        ids.extend(self.pointer_press.values().copied());
        ids.extend(self.pointer_captures.values().copied());
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

    fn presence_flags_of(&self, entity: Entity) -> PresenceFlags {
        let visual = self.world.get::<StandardVisual>(entity);
        let style = self.world.get::<NodeStyle>(entity);
        PresenceFlags {
            confirm: is_confirm_modal(visual),
            clip: is_clip_visual(visual),
            z_index: style.is_some_and(|style| style.layout.z_index.is_some()),
            viewport: style.is_some_and(|style| style.layout.depends_on_viewport()),
        }
    }

    fn apply_presence_flags(
        &mut self,
        id: Option<StableNodeId>,
        entity: Entity,
        next: PresenceFlags,
    ) {
        let previous = self
            .presence_flags
            .get(&entity)
            .copied()
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
        if next == PresenceFlags::NONE {
            self.presence_flags.remove(&entity);
        } else {
            self.presence_flags.insert(entity, next);
        }
    }

    fn sync_node_presence(&mut self, id: StableNodeId) {
        let Some(&entity) = self.entities.get(&id) else {
            return;
        };
        let next = if self.presence_live(id) {
            self.presence_flags_of(entity)
        } else {
            PresenceFlags::NONE
        };
        self.apply_presence_flags(Some(id), entity, next);
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

    fn forget_visual_presence(&mut self, entity: Entity) {
        let id = self
            .world
            .get::<Identity>(entity)
            .map(|identity| identity.stable);
        self.apply_presence_flags(id, entity, PresenceFlags::NONE);
    }

    fn extract_node_memo(
        &self,
        id: StableNodeId,
        memo: &mut AncestorMemo,
    ) -> Option<ExtractedNode> {
        if !self.presence_live_memo(id, memo) {
            return None;
        }
        let entity = *self.entities.get(&id)?;
        let identity = self.world.get::<Identity>(entity)?;
        let (mut style, resolved_epoch) = {
            let resolved = self.world.get::<ResolvedStyle>(entity)?;
            (Arc::clone(&resolved.0), resolved.1)
        };
        if resolved_epoch != self.palette_epoch {
            let parent = self.world.get::<Hierarchy>(entity)?.parent;
            let inherited_color = parent.and_then(|parent| {
                memo.color
                    .get(&parent)
                    .copied()
                    .or_else(|| self.inherited_palette_color(Some(parent)))
            });
            let (foreground, color, background, border_color) =
                self.palette_paint_colors(id, inherited_color);
            if let Some(color) = color {
                memo.color.insert(id, color);
            }
            if style.foreground != foreground
                || style.color != color
                || style.background != background
                || style.border_color != border_color
            {
                let style = Arc::make_mut(&mut style);
                style.foreground = foreground;
                style.color = color;
                style.background = background;
                style.border_color = border_color;
            }
        }
        let kind = Arc::clone(&self.world.get::<Kind>(entity)?.0);
        let has_text = matches!(kind.as_ref(), NodeKind::Text)
            || self
                .world
                .get::<TextContent>(entity)
                .is_some_and(|text| !text.value.is_empty());
        let source_style = self.world.get::<NodeStyle>(entity)?.clone();
        let hierarchy = self.world.get::<Hierarchy>(entity)?;
        let mut standard_visual = self.world.get::<StandardVisual>(entity).cloned();
        if let Some((busy, danger, is_confirm)) = self.confirm_action_effect(id) {
            if busy && !is_confirm {
                Arc::make_mut(&mut style).color =
                    Some(self.style_model.palette.muted.as_rgba_array());
            }
            if is_confirm
                && let Some(StandardVisual::Button { kind, loading, .. }) = standard_visual.as_mut()
            {
                *kind = if danger {
                    nana_ui_core::ButtonKind::Danger
                } else {
                    nana_ui_core::ButtonKind::Primary
                };
                *loading = busy;
            }
        }
        let component_geometry = standard_visual
            .as_ref()
            .and_then(|visual| self.derive_component_geometry(id, visual, style.as_ref()));
        let standard_visual_foreground = standard_visual.as_ref().map(|visual| match visual {
            StandardVisual::ModalFrame { .. } => self.style_model.palette.text.as_rgba_array(),
            StandardVisual::Icon { .. } => style
                .color
                .unwrap_or_else(|| self.style_model.palette.muted.as_rgba_array()),
            StandardVisual::Button { .. } => style
                .color
                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array()),
            StandardVisual::TextInput { .. } => style
                .color
                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array()),
            StandardVisual::SelectionOption { .. } => style
                .color
                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array()),
            StandardVisual::Checkbox {
                checked,
                indeterminate,
                ..
            } => {
                if *checked || *indeterminate {
                    self.style_model.palette.accent_text.as_rgba_array()
                } else {
                    self.style_model.palette.muted.as_rgba_array()
                }
            }
            StandardVisual::Switch { checked: true, .. } => {
                self.style_model.palette.accent_text.as_rgba_array()
            }
            StandardVisual::Switch { checked: false, .. } => {
                self.style_model.palette.muted.as_rgba_array()
            }
            StandardVisual::Scrollbar { .. } => {
                self.style_model.palette.border_strong.as_rgba_array()
            }
            StandardVisual::Range { .. }
            | StandardVisual::Card { .. }
            | StandardVisual::ListItem { .. }
            | StandardVisual::StatusBadge { .. }
            | StandardVisual::ValidationMessage { .. }
            | StandardVisual::EmptyState { .. }
            | StandardVisual::LabeledValue { .. }
            | StandardVisual::Progress { .. }
            | StandardVisual::Spinner { .. }
            | StandardVisual::FormField { .. } => self.style_model.palette.accent.as_rgba_array(),
            StandardVisual::QrCode { .. } => [0.0, 0.0, 0.0, 1.0],
            StandardVisual::Toast { tone, .. } => self
                .style_model
                .palette
                .get(status_tone_role(tone.status()))
                .as_rgba_array(),
            StandardVisual::XYPad { .. } => self.style_model.palette.text.as_rgba_array(),
            StandardVisual::Select { .. }
            | StandardVisual::MenuSurface { .. }
            | StandardVisual::ActionMenuItem { .. }
            | StandardVisual::TreeView { .. }
            | StandardVisual::CommandPalette { .. } => {
                self.style_model.palette.text.as_rgba_array()
            }
            StandardVisual::LevelMeter { tone, .. } => self
                .style_model
                .palette
                .get(status_tone_role(*tone))
                .as_rgba_array(),
            StandardVisual::CalendarHeatmap { .. }
            | StandardVisual::TimeSeriesChart { .. }
            | StandardVisual::ReorderList { .. }
            | StandardVisual::NativeMarkdown { .. }
            | StandardVisual::SelectableRichText { .. }
            | StandardVisual::GraphCanvas { .. }
            | StandardVisual::ImageViewer { .. }
            | StandardVisual::KeyCaptureLayer { .. }
            | StandardVisual::KeymapLayer => self.style_model.palette.text.as_rgba_array(),
        });
        Some(ExtractedNode {
            id,
            kind,
            parent: hierarchy.parent,
            children: Arc::clone(&hierarchy.children),
            layout: *self.world.get::<LayoutBox>(entity)?,
            scroll_offset: *self.world.get::<ScrollOffset>(entity)?,
            z_index: self.stacking_z_index_memo(id, memo),
            source_style,
            style,
            text: if has_text {
                self.world.get::<TextContent>(entity).cloned()
            } else {
                None
            },
            text_metrics: if has_text {
                self.world.get::<TextMetrics>(entity).copied()
            } else {
                None
            },
            focused: self.focused.get(&identity.document) == Some(&id),
            ime: self.world.get::<ImeComposition>(entity).cloned(),
            text_input: self.world.get::<TextInputState>(entity).cloned(),
            text_spans: if has_text {
                self.extracted_text_spans(entity)
            } else {
                Vec::new()
            },
            standard_visual,
            component_geometry,
            standard_visual_foreground,
            custom_render: self.world.get::<CustomRenderNode>(entity).cloned(),
        })
    }

    fn stacking_z_index_memo(&self, id: StableNodeId, memo: &mut AncestorMemo) -> i32 {
        if self.z_index_nodes == 0 {
            return 0;
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut z_index = 0;
        while let Some(node) = current {
            if let Some(&known) = memo.stacking.get(&node) {
                z_index = known;
                break;
            }
            let Some(&entity) = self.entities.get(&node) else {
                break;
            };
            if let Some(authored) = self
                .world
                .get::<NodeStyle>(entity)
                .and_then(|style| style.layout.z_index)
            {
                z_index = authored;
                memo.stacking.insert(node, authored);
                break;
            }
            memo.chain.push(node);
            current = self
                .world
                .get::<Hierarchy>(entity)
                .and_then(|hierarchy| hierarchy.parent);
        }
        for node in memo.chain.drain(..) {
            memo.stacking.insert(node, z_index);
        }
        z_index
    }

    fn derive_component_geometry(
        &self,
        id: StableNodeId,
        visual: &StandardVisual,
        style: &ComputedStyle,
    ) -> Option<crate::ComponentGeometry> {
        let bounds = *self.world.get::<LayoutBox>(self.entities[&id])?;
        let source = self.world.get::<NodeStyle>(self.entities[&id])?;
        let padding = source.layout.resolved_padding_against(Some(bounds.width));
        let border = source.layout.resolved_border_width();
        let content = LayoutBox {
            x: bounds.x + border + padding.left,
            y: bounds.y + border + padding.top,
            width: (bounds.width - border * 2.0 - padding.left - padding.right).max(0.0),
            height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
        };
        let text_region = |bounds, content: Arc<str>, muted: bool, size: f32, weight| {
            crate::ComponentTextRegion {
                bounds,
                content,
                color: Some(if muted {
                    self.style_model.palette.muted.as_rgba_array()
                } else {
                    style
                        .color
                        .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array())
                }),
                font_size: size,
                font_weight: weight,
            }
        };
        match visual {
            StandardVisual::ModalFrame {
                title,
                description,
                body_text,
                kind,
                slots,
                ..
            } => {
                let presentation = self
                    .world
                    .get::<ModalTextPresentation>(self.entities[&id])
                    .copied()
                    .unwrap_or_default();
                let has_close = slots.close_action.is_some();
                let has_footer = slots.footer.is_some() || !slots.actions.is_empty();
                let chrome = crate::overlay_surfaces::ModalChrome::measure(
                    *kind,
                    presentation.title,
                    presentation.description,
                    has_close,
                    has_footer,
                );
                let body_copy = presentation.body.map_or(0.0, |metrics| metrics.height);
                let body_slot = slots
                    .body
                    .and_then(|id| self.layout_box(id))
                    .map_or(0.0, |region| region.height);
                let body_gap = if body_copy > 0.0 && body_slot > 0.0 {
                    8.0
                } else {
                    0.0
                };
                let intrinsic_height = chrome.chrome_height(body_copy + body_gap + body_slot);
                let surface = crate::overlay_surfaces::modal_surface_bounds(
                    bounds,
                    *kind,
                    Some(intrinsic_height),
                );
                let LayoutBox { x, y, width, .. } = surface;
                let text_width = chrome.text_width(width, *kind, has_close);
                let body = chrome.body_box(surface);
                let text_block = presentation.title.height
                    + presentation.description.map_or(0.0, |metrics| {
                        crate::overlay_surfaces::MODAL_TITLE_DESC_GAP + metrics.height
                    });
                let title_y = match kind {
                    crate::ModalSurfaceKind::Drawer(_) => {
                        y + (chrome.header_height - text_block) / 2.0
                    }
                    _ => y + crate::overlay_surfaces::MODAL_HEADER_PAD_TOP,
                };
                let shadow_alpha = if self.style_model.palette.background.as_rgba_array()[0] > 0.5 {
                    0.28
                } else {
                    0.45
                };
                Some(crate::ComponentGeometry::ModalFrame {
                    scrim: bounds,
                    surface,
                    body,
                    title: text_region(
                        LayoutBox {
                            x: x + chrome.pad_x,
                            y: title_y,
                            width: text_width,
                            height: presentation.title.height,
                        },
                        Arc::clone(title),
                        false,
                        14.0,
                        Some(600),
                    ),
                    description: description.as_ref().map(|description| {
                        text_region(
                            LayoutBox {
                                x: x + chrome.pad_x,
                                y: title_y
                                    + presentation.title.height
                                    + crate::overlay_surfaces::MODAL_TITLE_DESC_GAP,
                                width: text_width,
                                height: presentation.description.unwrap_or_default().height,
                            },
                            Arc::clone(description),
                            true,
                            12.0,
                            None,
                        )
                    }),
                    body_text: body_text.as_ref().map(|message| {
                        text_region(
                            LayoutBox {
                                x: body.x,
                                y: body.y,
                                width: body.width,
                                height: presentation.body.unwrap_or_default().height,
                            },
                            Arc::clone(message),
                            false,
                            crate::overlay_surfaces::MODAL_BODY_TEXT_SIZE,
                            None,
                        )
                    }),
                    background: self.style_model.palette.surface.as_rgba_array(),
                    border: [0.0; 4],
                    elevation: crate::ComponentElevation {
                        color: [0.0, 0.0, 0.0, shadow_alpha],
                        offset_x: 0.0,
                        offset_y: 14.0,
                        blur_radius: 30.0,
                        spread_radius: 0.0,
                        inset: false,
                    },
                })
            }
            StandardVisual::Button {
                label,
                size,
                loading,
                ..
            } => {
                // Loading reserves 20px through symmetric intrinsic padding in
                // the layout pass. That reservation grows the outer button; it
                // is not additional visual padding, so return it to the inline
                // content box before centering spinner + label.
                let button_content = if *loading {
                    LayoutBox {
                        x: content.x - 10.0,
                        width: content.width + 20.0,
                        ..content
                    }
                } else {
                    content
                };
                let label_width = self
                    .text_metrics(id)
                    .map_or(0.0, |metrics| metrics.width.min(button_content.width));
                let spinner_extent = size.icon_size().min(button_content.height);
                let gap = if *loading { 6.0 } else { 0.0 };
                let group_width = (label_width + if *loading { spinner_extent + gap } else { 0.0 })
                    .min(button_content.width);
                let group_x = button_content.x + (button_content.width - group_width) / 2.0;
                let spinner = (*loading).then_some(LayoutBox {
                    x: group_x,
                    y: button_content.y + (button_content.height - spinner_extent) / 2.0,
                    width: spinner_extent,
                    height: spinner_extent,
                });
                let label_x = group_x + if *loading { spinner_extent + gap } else { 0.0 };
                Some(crate::ComponentGeometry::Button {
                    label: text_region(
                        LayoutBox {
                            x: label_x,
                            y: button_content.y,
                            width: (group_x + group_width - label_x).max(0.0),
                            height: button_content.height,
                        },
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    spinner,
                    background: style.background,
                    border: style.border_color,
                    border_width: if style.border_color.is_some() {
                        source.layout.resolved_border_width()
                    } else {
                        0.0
                    },
                    focus_ring: None,
                })
            }
            StandardVisual::TextInput {
                size,
                invalid,
                steppers,
                ..
            } => {
                let presentation = self
                    .world
                    .get::<TextInputPresentation>(self.entities[&id])?;
                let accessibility = self.world.get::<AccessibilityState>(self.entities[&id]);
                let disabled = accessibility.is_some_and(|state| state.disabled);
                let steppers = steppers
                    .then(|| {
                        let band = (size.height() / 2.0).min(content.height / 2.0);
                        let width = size.indicator_size();
                        if band <= 0.0 || width <= 0.0 || content.width <= width {
                            return None;
                        }
                        let x = content.x + content.width - width;
                        let numeric = accessibility.map(|state| {
                            (
                                state.numeric_value.unwrap_or_default(),
                                state.numeric_minimum,
                                state.numeric_maximum,
                            )
                        });
                        let can_increment = !disabled
                            && numeric.is_none_or(|(value, _, maximum)| {
                                maximum.is_none_or(|maximum| value < maximum)
                            });
                        let can_decrement = !disabled
                            && numeric.is_none_or(|(value, minimum, _)| {
                                minimum.is_none_or(|minimum| value > minimum)
                            });
                        let active = self.style_model.palette.muted.as_rgba_array();
                        let inert = self.style_model.palette.faint.as_rgba_array();
                        Some(crate::NumberSteppers {
                            increment: LayoutBox {
                                x,
                                y: content.y + (content.height - band * 2.0) / 2.0,
                                width,
                                height: band,
                            },
                            decrement: LayoutBox {
                                x,
                                y: content.y + (content.height - band * 2.0) / 2.0 + band,
                                width,
                                height: band,
                            },
                            increment_color: if can_increment { active } else { inert },
                            decrement_color: if can_decrement { active } else { inert },
                            increment_enabled: can_increment,
                            decrement_enabled: can_decrement,
                            glyph_size: (width - 2.0).max(1.0),
                        })
                    })
                    .flatten();
                let content = match steppers {
                    Some(steppers) => LayoutBox {
                        width: (content.width - steppers.increment.width - 4.0).max(0.0),
                        ..content
                    },
                    None => content,
                };
                let focused =
                    self.focused.get(&self.component::<Identity>(id).document) == Some(&id);
                let metrics = self.text_metrics(id).unwrap_or_default();
                let multiline = accessibility.is_some_and(|state| state.multiline);
                let requested_scroll = *self.component::<ScrollOffset>(id);
                let mut scroll_x = if multiline {
                    requested_scroll.x
                } else {
                    (presentation.caret_x - content.width + 1.0).max(0.0)
                }
                .min((metrics.width - content.width).max(0.0));
                let line_height = if multiline {
                    presentation.line_height
                } else {
                    size.line_height()
                }
                .max(1.0)
                .min(content.height.max(1.0));
                let mut scroll_y = if multiline {
                    requested_scroll
                        .y
                        .min((metrics.height - content.height).max(0.0))
                } else {
                    0.0
                };
                if multiline && focused {
                    if presentation.caret_x < scroll_x {
                        scroll_x = presentation.caret_x;
                    } else if presentation.caret_x + 1.0 > scroll_x + content.width {
                        scroll_x = presentation.caret_x + 1.0 - content.width;
                    }
                    if presentation.caret_y < scroll_y {
                        scroll_y = presentation.caret_y;
                    } else if presentation.caret_y + line_height > scroll_y + content.height {
                        scroll_y = presentation.caret_y + line_height - content.height;
                    }
                }
                scroll_x = scroll_x.clamp(0.0, (metrics.width - content.width).max(0.0));
                scroll_y = scroll_y.clamp(0.0, (metrics.height - content.height).max(0.0));
                let line_y = if multiline {
                    content.y - scroll_y
                } else {
                    content.y + (content.height - line_height) / 2.0
                };
                let field_x = |offset: f32| content.x + offset - scroll_x;
                let (selection, preedit) = text_input_decorations(
                    presentation,
                    multiline,
                    content,
                    line_y,
                    line_height,
                    scroll_x,
                    scroll_y,
                );
                let caret = focused.then_some(LayoutBox {
                    x: field_x(presentation.caret_x),
                    y: line_y + presentation.caret_y,
                    width: 1.0,
                    height: line_height,
                });
                Some(crate::ComponentGeometry::TextInput {
                    text: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: content.x - scroll_x,
                            y: line_y,
                            width: metrics.width.max(content.width),
                            height: if multiline {
                                metrics.height.max(content.height)
                            } else {
                                line_height
                            },
                        },
                        content: Arc::from(presentation.display_value.as_str()),
                        color: Some(if presentation.placeholder {
                            text_input_placeholder_color(
                                &source.layout,
                                self.style_model.palette.faint.as_rgba_array(),
                            )
                        } else {
                            style
                                .color
                                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array())
                        }),
                        font_size: size.text_size(),
                        font_weight: style.font_weight,
                    },
                    multiline,
                    selection,
                    caret,
                    preedit,
                    background: style.background,
                    border: style.border_color,
                    border_width: {
                        let width = if style.border_color.is_some() {
                            source.layout.resolved_border_width()
                        } else {
                            0.0
                        };
                        if multiline && focused && *invalid {
                            width.max(2.0)
                        } else {
                            width
                        }
                    },
                    focus_ring: (!multiline && focused).then(|| {
                        if *invalid {
                            self.style_model.palette.danger.as_rgba_array()
                        } else {
                            self.style_model.palette.accent.as_rgba_array()
                        }
                    }),
                    selection_color: self.style_model.palette.accent_soft.as_rgba_array(),
                    caret_color: style
                        .color
                        .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array()),
                    preedit_color: self.style_model.palette.accent.as_rgba_array(),
                    steppers,
                })
            }
            StandardVisual::Switch {
                label,
                hint,
                checked,
                control_position,
                size,
                invalid,
                ..
            } => {
                let control = LayoutBox {
                    x: match control_position {
                        SwitchControlPosition::Start => content.x,
                        SwitchControlPosition::End => content.x + (content.width - 30.0).max(0.0),
                    },
                    y: content.y + (content.height - 16.0) / 2.0,
                    width: 30.0_f32.min(content.width),
                    height: 16.0_f32.min(content.height),
                };
                let text_x = if *control_position == SwitchControlPosition::Start {
                    control.x + control.width + 8.0
                } else {
                    content.x
                };
                let text_right = if *control_position == SwitchControlPosition::End {
                    control.x - 8.0
                } else {
                    content.x + content.width
                };
                let text_width = (text_right - text_x).max(0.0);
                let (label_bounds, hint_bounds) = if hint.is_some() {
                    (
                        LayoutBox {
                            x: text_x,
                            y: content.y,
                            width: text_width,
                            height: 18.0_f32.min(content.height),
                        },
                        Some(LayoutBox {
                            x: text_x,
                            y: content.y + 20.0,
                            width: text_width,
                            height: (content.height - 20.0).max(0.0),
                        }),
                    )
                } else {
                    (
                        LayoutBox {
                            x: text_x,
                            y: content.y,
                            width: text_width,
                            height: content.height,
                        },
                        None,
                    )
                };
                let palette = self.style_model.palette;
                let hovered = self.pointer_hover.values().any(|target| *target == id);
                let pressed = self.pointer_press.values().any(|target| *target == id);
                let disabled = self.component::<AccessibilityState>(id).disabled;
                let mix = |foreground: [f32; 4], background: [f32; 4], amount: f32| {
                    let amount = amount.clamp(0.0, 1.0);
                    std::array::from_fn(|channel| {
                        foreground[channel] * amount + background[channel] * (1.0 - amount)
                    })
                };
                let fade = |mut color: [f32; 4]| {
                    if disabled {
                        color[3] *= 0.55;
                    }
                    color
                };
                let track_background = if *checked {
                    if pressed {
                        palette.accent_strong.as_rgba_array()
                    } else {
                        palette.accent.as_rgba_array()
                    }
                } else {
                    mix(
                        palette.hover.as_rgba_array(),
                        palette.background.as_rgba_array(),
                        0.78,
                    )
                };
                let track_border = if *invalid {
                    palette.danger.as_rgba_array()
                } else if *checked {
                    if hovered || pressed {
                        palette.accent_strong.as_rgba_array()
                    } else {
                        palette.accent.as_rgba_array()
                    }
                } else if hovered || pressed {
                    mix(
                        palette.accent.as_rgba_array(),
                        palette.border_strong.as_rgba_array(),
                        if pressed { 0.70 } else { 0.42 },
                    )
                } else {
                    palette.border_strong.as_rgba_array()
                };
                let thumb_background = if *checked {
                    palette.accent_text.as_rgba_array()
                } else {
                    mix(
                        palette.faint.as_rgba_array(),
                        palette.background.as_rgba_array(),
                        0.70,
                    )
                };
                Some(crate::ComponentGeometry::Switch {
                    label: text_region(
                        label_bounds,
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    hint: hint.as_ref().zip(hint_bounds).map(|(hint, bounds)| {
                        text_region(
                            bounds,
                            Arc::clone(hint),
                            true,
                            (size.text_size() - 1.0).max(10.0),
                            None,
                        )
                    }),
                    control,
                    track_background: fade(track_background),
                    track_border: fade(track_border),
                    thumb_background: fade(thumb_background),
                })
            }
            StandardVisual::Scrollbar {
                axes,
                visibility,
                revealed,
                dragging,
            } => {
                if !revealed || matches!(visibility, nana_ui_core::ScrollbarVisibility::Hidden) {
                    return None;
                }
                let metrics = self.scroll_metrics(id)?;
                let offset = self.scroll_offset(id).unwrap_or_default();
                let chrome = nana_ui_core::SCROLLBAR_METRICS;
                let palette = &self.style_model.palette;
                let track_background =
                    matches!(visibility, nana_ui_core::ScrollbarVisibility::Always)
                        .then(|| palette.subtle.as_rgba_array());
                let scrolls = |axis: nana_ui_core::ScrollbarAxis| match axis {
                    nana_ui_core::ScrollbarAxis::Horizontal => {
                        axes.horizontal() && metrics.content_width > metrics.viewport_width
                    }
                    nana_ui_core::ScrollbarAxis::Vertical => {
                        axes.vertical() && metrics.content_height > metrics.viewport_height
                    }
                };
                let bar = |axis: nana_ui_core::ScrollbarAxis| {
                    if !scrolls(axis) {
                        return None;
                    }
                    let horizontal = matches!(axis, nana_ui_core::ScrollbarAxis::Horizontal);
                    // Give up the far corner only when the other axis also has
                    // a bar, so the two never overlap.
                    let corner = if scrolls(match axis {
                        nana_ui_core::ScrollbarAxis::Horizontal => {
                            nana_ui_core::ScrollbarAxis::Vertical
                        }
                        nana_ui_core::ScrollbarAxis::Vertical => {
                            nana_ui_core::ScrollbarAxis::Horizontal
                        }
                    }) {
                        chrome.thickness
                    } else {
                        0.0
                    };
                    let along = if horizontal {
                        (bounds.width - corner).max(0.0)
                    } else {
                        (bounds.height - corner).max(0.0)
                    };
                    let track = nana_ui_core::scrollbar_track(
                        if horizontal {
                            metrics.viewport_width
                        } else {
                            metrics.viewport_height
                        },
                        if horizontal {
                            metrics.content_width
                        } else {
                            metrics.content_height
                        },
                        if horizontal { offset.x } else { offset.y },
                        if horizontal { bounds.x } else { bounds.y },
                        along,
                        chrome,
                    )?;
                    let active = *dragging == Some(axis);
                    let thumb_inset = ((chrome.thickness - chrome.thumb_thickness) / 2.0).max(0.0);
                    let (track_box, thumb_box) = if horizontal {
                        let track_y = bounds.y + bounds.height - chrome.thickness;
                        (
                            LayoutBox {
                                x: track.origin,
                                y: track_y,
                                width: track.length,
                                height: chrome.thickness,
                            },
                            LayoutBox {
                                x: track.thumb_origin,
                                y: track_y + thumb_inset,
                                width: track.thumb_length,
                                height: chrome.thumb_thickness,
                            },
                        )
                    } else {
                        let track_x = bounds.x + bounds.width - chrome.thickness;
                        (
                            LayoutBox {
                                x: track_x,
                                y: track.origin,
                                width: chrome.thickness,
                                height: track.length,
                            },
                            LayoutBox {
                                x: track_x + thumb_inset,
                                y: track.thumb_origin,
                                width: chrome.thumb_thickness,
                                height: track.thumb_length,
                            },
                        )
                    };
                    Some(crate::ScrollbarBar {
                        track: track_box,
                        thumb: thumb_box,
                        track_background,
                        thumb_background: if active {
                            palette.muted.as_rgba_array()
                        } else {
                            palette.border_strong.as_rgba_array()
                        },
                        thumb_radius: chrome.thumb_radius(),
                        max_offset: track.max_offset,
                    })
                };
                let horizontal = bar(nana_ui_core::ScrollbarAxis::Horizontal);
                let vertical = bar(nana_ui_core::ScrollbarAxis::Vertical);
                (horizontal.is_some() || vertical.is_some()).then_some(
                    crate::ComponentGeometry::Scrollbar {
                        horizontal,
                        vertical,
                    },
                )
            }
            StandardVisual::Range {
                label,
                value,
                unit,
                size,
                ..
            } => {
                let gap = match size {
                    nana_ui_core::ControlSize::Small => 6.0,
                    nana_ui_core::ControlSize::Medium => 8.0,
                    nana_ui_core::ControlSize::Large => 10.0,
                };
                let label_width = label
                    .as_ref()
                    .map_or(0.0, |_| 84.0_f32.min(content.width * 0.28));
                let unit_width = unit.as_ref().map_or(0.0, |_| 32.0_f32.min(content.width));
                let value_width = 60.0_f32.min((content.width - unit_width).max(0.0));
                let trailing_width = value_width + unit_width;
                let track_x = content.x + label_width + if label.is_some() { gap } else { 0.0 };
                let track_right = content.x + content.width
                    - trailing_width
                    - if trailing_width > 0.0 { gap } else { 0.0 };
                let thumb = size.icon_size();
                let track = LayoutBox {
                    x: track_x + thumb / 2.0,
                    y: content.y + (content.height - thumb) / 2.0,
                    width: (track_right - track_x - thumb).max(0.0),
                    height: thumb.min(content.height),
                };
                Some(crate::ComponentGeometry::Range {
                    label: label.as_ref().map(|label| {
                        text_region(
                            LayoutBox {
                                x: content.x,
                                y: content.y,
                                width: label_width,
                                height: content.height,
                            },
                            Arc::clone(label),
                            false,
                            size.text_size(),
                            Some(500),
                        )
                    }),
                    value: text_region(
                        LayoutBox {
                            x: content.x + content.width - value_width - unit_width,
                            y: content.y,
                            width: value_width,
                            height: content.height,
                        },
                        Arc::clone(value),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    unit: unit.as_ref().map(|unit| {
                        text_region(
                            LayoutBox {
                                x: content.x + content.width - unit_width,
                                y: content.y,
                                width: unit_width,
                                height: content.height,
                            },
                            Arc::clone(unit),
                            true,
                            (size.text_size() - 1.0).max(10.0),
                            None,
                        )
                    }),
                    track,
                })
            }
            StandardVisual::Card {
                title,
                kind,
                loading,
                ..
            } => {
                let shaped_title_width = self
                    .text_metrics(id)
                    .map_or(0.0, |metrics| metrics.width.min(content.width));
                let title_width = (content.width - if *loading { 22.0 } else { 0.0 }).max(0.0);
                let title_y = bounds.y + border + (padding.top - 24.0).max(0.0);
                Some(crate::ComponentGeometry::Card {
                    title: title.as_ref().map(|title| {
                        text_region(
                            LayoutBox {
                                x: bounds.x + border + padding.left,
                                y: title_y,
                                width: title_width,
                                height: 18.0,
                            },
                            Arc::clone(title),
                            false,
                            13.0,
                            Some(600),
                        )
                    }),
                    content,
                    elevation: (*kind == nana_ui_core::CardKind::Raised).then_some(
                        crate::ComponentElevation::surface_shadow(self.style_model.theme_mode),
                    ),
                    spinner: (*loading).then_some(LayoutBox {
                        x: (bounds.x + border + padding.left + shaped_title_width + 8.0)
                            .min(content.x + content.width - 14.0),
                        y: title_y + 2.0,
                        width: 14.0,
                        height: 14.0,
                    }),
                })
            }
            StandardVisual::ListItem {
                leading,
                content: content_slot,
                trailing,
            } => {
                let leading = leading.and_then(|id| self.layout_box(id));
                let trailing = trailing.and_then(|id| self.layout_box(id));
                let fallback_x = leading.map_or(content.x, |leading| {
                    leading.x
                        + leading.width
                        + source.layout.main_gap_against(
                            nana_ui_core::FlexDirection::Row,
                            nana_ui_core::ParentBox::from_viewport(content.width, content.height),
                        )
                });
                let fallback_right = trailing.map_or(content.x + content.width, |trailing| {
                    trailing.x
                        - source.layout.main_gap_against(
                            nana_ui_core::FlexDirection::Row,
                            nana_ui_core::ParentBox::from_viewport(content.width, content.height),
                        )
                });
                Some(crate::ComponentGeometry::ListItem {
                    leading,
                    content: content_slot.and_then(|id| self.layout_box(id)).or_else(|| {
                        Some(LayoutBox {
                            x: fallback_x,
                            y: content.y,
                            width: (fallback_right - fallback_x).max(0.0),
                            height: content.height,
                        })
                    }),
                    trailing,
                })
            }
            StandardVisual::StatusBadge {
                label,
                tone,
                compact,
            } => {
                let (horizontal, indicator_slot, gap, text_size) = if *compact {
                    (7.0, 6.0, 5.0, 11.0)
                } else {
                    (8.0, 8.0, 6.0, 12.0)
                };
                let diameter = indicator_slot * 10.0 / 24.0;
                let foreground = self
                    .style_model
                    .palette
                    .get(status_tone_role(*tone))
                    .as_rgba_array();
                let mut background = foreground;
                background[3] *= 0.12;
                Some(crate::ComponentGeometry::StatusBadge {
                    indicator: LayoutBox {
                        x: bounds.x + horizontal + (indicator_slot - diameter) / 2.0,
                        y: bounds.y + (bounds.height - diameter) / 2.0,
                        width: diameter,
                        height: diameter,
                    },
                    label: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: bounds.x + horizontal + indicator_slot + gap,
                            y: bounds.y,
                            width: (bounds.width - horizontal * 2.0 - indicator_slot - gap)
                                .max(0.0),
                            height: bounds.height,
                        },
                        content: Arc::clone(label),
                        color: Some(foreground),
                        font_size: text_size,
                        font_weight: Some(500),
                    },
                    background,
                    foreground,
                })
            }
            StandardVisual::ValidationMessage {
                message,
                intent,
                compact,
            } => {
                let (indicator_slot, gap, text_size) = if *compact {
                    (12.0, 5.0, 11.0)
                } else {
                    (14.0, 6.0, 12.0)
                };
                let diameter = indicator_slot * 10.0 / 24.0;
                let foreground = self
                    .style_model
                    .palette
                    .get(match intent {
                        nana_ui_core::ValidationIntent::Warning => SemanticColorRole::Warning,
                        nana_ui_core::ValidationIntent::Danger => SemanticColorRole::Danger,
                    })
                    .as_rgba_array();
                Some(crate::ComponentGeometry::ValidationMessage {
                    indicator: LayoutBox {
                        x: bounds.x + (indicator_slot - diameter) / 2.0,
                        y: bounds.y + (bounds.height - diameter) / 2.0,
                        width: diameter,
                        height: diameter,
                    },
                    label: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: bounds.x + indicator_slot + gap,
                            y: bounds.y,
                            width: (bounds.width - indicator_slot - gap).max(0.0),
                            height: bounds.height,
                        },
                        content: Arc::clone(message),
                        color: Some(foreground),
                        font_size: text_size,
                        font_weight: None,
                    },
                    foreground,
                })
            }
            StandardVisual::EmptyState {
                title,
                message,
                icon,
                compact,
                action,
            } => {
                let (horizontal, vertical, title_size, message_size, spacing) = if *compact {
                    (6.0, 8.0, 12.0, 11.0, 2.0)
                } else {
                    (16.0, 24.0, 13.0, 12.0, 6.0)
                };
                let width = (bounds.width - horizontal * 2.0).max(0.0);
                let presentation = self
                    .world
                    .get::<EmptyStateTextPresentation>(self.entities[&id])
                    .copied()
                    .unwrap_or_default();
                let text_bounds = |metrics: TextMetrics, y: f32| {
                    let shaped_width = metrics.width.clamp(0.0, width);
                    crate::LayoutBox {
                        x: bounds.x
                            + horizontal
                            + if *compact {
                                0.0
                            } else {
                                (width - shaped_width) / 2.0
                            },
                        y,
                        width: shaped_width,
                        height: metrics.height,
                    }
                };
                let mut y = bounds.y + vertical;
                let icon = icon.map(|icon| {
                    let icon_width = 22.0_f32.min(width);
                    let icon_bounds = LayoutBox {
                        x: if *compact {
                            bounds.x + horizontal
                        } else {
                            bounds.x + horizontal + (width - icon_width) / 2.0
                        },
                        y,
                        width: icon_width,
                        height: 22.0,
                    };
                    y += 22.0 + spacing;
                    (
                        icon,
                        icon_bounds,
                        self.style_model.palette.faint.as_rgba_array(),
                    )
                });
                let title_region = crate::ComponentTextRegion {
                    bounds: text_bounds(presentation.title, y),
                    content: Arc::clone(title),
                    color: Some(if *compact {
                        self.style_model.palette.muted.as_rgba_array()
                    } else {
                        self.style_model.palette.text.as_rgba_array()
                    }),
                    font_size: title_size,
                    font_weight: Some(600),
                };
                y += presentation.title.height;
                let message = message.as_ref().map(|message| {
                    y += spacing;
                    crate::ComponentTextRegion {
                        bounds: text_bounds(presentation.message.unwrap_or_default(), y),
                        content: Arc::clone(message),
                        color: Some(self.style_model.palette.muted.as_rgba_array()),
                        font_size: message_size,
                        font_weight: None,
                    }
                });
                Some(crate::ComponentGeometry::EmptyState {
                    root_clip: bounds,
                    content_clip: LayoutBox {
                        x: bounds.x + horizontal,
                        y: bounds.y + vertical,
                        width,
                        height: (bounds.height - vertical * 2.0).max(0.0),
                    },
                    icon,
                    title: title_region,
                    message,
                    action: action.and_then(|action| self.layout_box(action)),
                })
            }
            StandardVisual::LabeledValue {
                label,
                value,
                value_role,
                value_weight,
                compact,
                action,
            } => {
                let gap = if *compact { 4.0 } else { 8.0 };
                let right = action
                    .and_then(|action| self.layout_box(action))
                    .map_or(bounds.x + bounds.width, |action| {
                        (action.x - gap).max(bounds.x)
                    });
                let available = (right - bounds.x).max(0.0);
                let label_size = 11.0_f32;
                let value_size = 12.0_f32;
                let label_height = (label_size * 1.2).min(bounds.height.max(label_size));
                let value_height = (value_size * 1.2).min(bounds.height.max(value_size));
                let min_label = if label.is_empty() { 0.0 } else { label_size };
                let value_width = estimated_text_width(value, value_size)
                    .min((available - gap - min_label).max(0.0));
                let value_x = (right - value_width).max(bounds.x);
                let label_width = (value_x - gap - bounds.x).max(0.0);
                let center_y = |height: f32| bounds.y + (bounds.height - height).max(0.0) / 2.0;
                Some(crate::ComponentGeometry::LabeledValue {
                    label: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: bounds.x,
                            y: center_y(label_height),
                            width: label_width,
                            height: label_height,
                        },
                        content: Arc::clone(label),
                        color: Some(self.style_model.palette.faint.as_rgba_array()),
                        font_size: label_size,
                        font_weight: None,
                    },
                    value: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: value_x,
                            y: center_y(value_height),
                            width: value_width,
                            height: value_height,
                        },
                        content: Arc::clone(value),
                        color: Some(self.style_model.palette.get(*value_role).as_rgba_array()),
                        font_size: value_size,
                        font_weight: Some(*value_weight),
                    },
                    action: action.and_then(|action| self.layout_box(action)),
                })
            }
            StandardVisual::SelectionOption {
                label,
                icon,
                size,
                show_focus_ring,
                indicator,
                selected,
                disabled,
            } => {
                let icon_extent = size.icon_size().min(content.height);
                let base_padding = if *indicator {
                    size.radio_lead()
                } else {
                    size.padding_x() + 2.0
                };
                let ring = indicator.then(|| {
                    let extent = size.indicator_size().min(bounds.height);
                    let ring = LayoutBox {
                        x: bounds.x + nana_ui_core::RADIO_ROW_INSET,
                        y: bounds.y + (bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    };
                    let (ring_color, dot_color) = if *disabled {
                        let faint = self.style_model.palette.faint.as_rgba_array();
                        (faint, faint)
                    } else if *selected {
                        let accent = self.style_model.palette.accent.as_rgba_array();
                        (accent, accent)
                    } else {
                        (
                            self.style_model.palette.border_strong.as_rgba_array(),
                            self.style_model.palette.accent.as_rgba_array(),
                        )
                    };
                    let dot_extent = extent / 2.5;
                    crate::RadioIndicator {
                        ring,
                        ring_color,
                        dot: selected.then_some((
                            LayoutBox {
                                x: ring.x + (extent - dot_extent) / 2.0,
                                y: ring.y + (extent - dot_extent) / 2.0,
                                width: dot_extent,
                                height: dot_extent,
                            },
                            dot_color,
                        )),
                    }
                });
                let icon_bounds = icon.map(|icon| {
                    (
                        icon,
                        LayoutBox {
                            x: bounds.x + base_padding,
                            y: icon_y_on_text_glyph_center(
                                content.y,
                                content.height,
                                style.font_size,
                                style.line_height,
                                matches!(
                                    source.text_vertical_alignment,
                                    TextVerticalAlignment::Center
                                ),
                                icon_extent,
                            ),
                            width: icon_extent,
                            height: icon_extent,
                        },
                        style
                            .color
                            .unwrap_or_else(|| self.style_model.palette.muted.as_rgba_array()),
                    )
                });
                let label_x = icon_bounds
                    .as_ref()
                    .map_or(content.x, |(_, icon, _)| icon.x + icon.width + 5.0);
                Some(crate::ComponentGeometry::SelectionOption {
                    icon: icon_bounds,
                    label: text_region(
                        LayoutBox {
                            x: label_x,
                            y: content.y,
                            width: (content.x + content.width - label_x).max(0.0),
                            height: content.height,
                        },
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    focus_ring: (*show_focus_ring
                        && self.focused.get(&self.component::<Identity>(id).document) == Some(&id))
                    .then(|| self.style_model.palette.accent.as_rgba_array()),
                    indicator: ring,
                })
            }
            StandardVisual::Progress {
                value_ratio,
                label,
                cancellable,
            } => progress_geometry(
                bounds,
                style,
                *value_ratio,
                6.0,
                3.0,
                label.as_ref(),
                *cancellable,
                self.style_model.palette.text.as_rgba_array(),
            ),
            StandardVisual::LevelMeter {
                value_ratio, girth, ..
            } => {
                let girth = if girth.is_finite() && *girth > 0.0 {
                    *girth
                } else {
                    4.0
                };
                progress_geometry(
                    bounds,
                    style,
                    *value_ratio,
                    girth,
                    girth / 2.0,
                    None,
                    false,
                    [0.0; 4],
                )
            }
            StandardVisual::FormField {
                label,
                hint,
                error,
                size,
                control,
            } => form_field_geometry(
                bounds,
                *size,
                label,
                hint.as_ref(),
                error.as_ref(),
                *control,
                &|id| self.layout_box(id),
                &self.style_model.palette,
            ),
            StandardVisual::Toast {
                title,
                description,
                dismissible,
                ..
            } => {
                let pad_x = 12.0;
                let pad_y = 10.0;
                let indicator = 7.0;
                let gap = 8.0;
                let dismiss = if *dismissible { 28.0 } else { 0.0 };
                let copy_x = bounds.x + pad_x + indicator + gap;
                let copy_right =
                    bounds.x + bounds.width - pad_x - if *dismissible { dismiss } else { 0.0 };
                let copy_width = (copy_right - copy_x).max(0.0);
                let title_height = 12.0;
                let desc_height = 11.0;
                let has_desc = description.is_some();
                let title_y = if has_desc {
                    bounds.y + pad_y
                } else {
                    bounds.y + (bounds.height - title_height) / 2.0
                };
                Some(crate::ComponentGeometry::Toast {
                    indicator: LayoutBox {
                        x: bounds.x + pad_x,
                        y: bounds.y + (bounds.height - indicator) / 2.0,
                        width: indicator,
                        height: indicator,
                    },
                    title: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: copy_x,
                            y: title_y,
                            width: copy_width,
                            height: title_height,
                        },
                        content: Arc::clone(title),
                        color: Some(self.style_model.palette.text.as_rgba_array()),
                        font_size: 12.0,
                        font_weight: Some(600),
                    },
                    description: description.as_ref().map(|description| {
                        crate::ComponentTextRegion {
                            bounds: LayoutBox {
                                x: copy_x,
                                y: title_y + title_height + 2.0,
                                width: copy_width,
                                height: desc_height,
                            },
                            content: Arc::clone(description),
                            color: Some(self.style_model.palette.muted.as_rgba_array()),
                            font_size: 11.0,
                            font_weight: None,
                        }
                    }),
                    dismiss: dismissible.then(|| LayoutBox {
                        x: bounds.x + bounds.width - pad_x - dismiss,
                        y: bounds.y + (bounds.height - dismiss) / 2.0,
                        width: dismiss,
                        height: dismiss,
                    }),
                })
            }
            StandardVisual::XYPad { nx, ny, .. } => {
                let pad = bounds;
                let thumb = 8.0;
                let nx = nx.clamp(0.0, 1.0);
                let ny = ny.clamp(0.0, 1.0);
                Some(crate::ComponentGeometry::XYPad {
                    pad,
                    thumb: LayoutBox {
                        x: pad.x + nx * pad.width - thumb / 2.0,
                        y: pad.y + ny * pad.height - thumb / 2.0,
                        width: thumb,
                        height: thumb,
                    },
                    h_axis: LayoutBox {
                        x: pad.x,
                        y: pad.y + pad.height / 2.0 - 0.5,
                        width: pad.width,
                        height: 1.0,
                    },
                    v_axis: LayoutBox {
                        x: pad.x + pad.width / 2.0 - 0.5,
                        y: pad.y,
                        width: 1.0,
                        height: pad.height,
                    },
                    background: style.background,
                    border: style.border_color,
                    border_width: if style.border_color.is_some() {
                        source.layout.resolved_border_width()
                    } else {
                        0.0
                    },
                    thumb_color: self.style_model.palette.accent.as_rgba_array(),
                    axis_color: self.style_model.palette.border.as_rgba_array(),
                })
            }
            StandardVisual::Select {
                label,
                placeholder,
                size,
                opened,
                options,
                highlighted,
                ..
            } => Some(crate::select::select_geometry(
                bounds,
                label,
                *placeholder,
                *size,
                *opened,
                options,
                *highlighted,
                style,
                source,
                &self.style_model.palette,
            )),
            StandardVisual::MenuSurface {
                kind: crate::MenuSurfaceKind::ContextMenu,
                query,
                rows,
                highlighted,
                ..
            } if query.is_some() || !rows.is_empty() => Some(crate::menus::context_menu_geometry(
                bounds,
                query.as_ref(),
                rows,
                *highlighted,
                &self.style_model.palette,
            )),
            StandardVisual::MenuSurface { trigger, gap, .. } => {
                Some(crate::popover::menu_surface_geometry(
                    bounds,
                    trigger.as_ref(),
                    *gap,
                    style,
                    &self.style_model.palette,
                ))
            }
            StandardVisual::ActionMenuItem {
                label,
                hint,
                icon,
                danger,
                disabled,
                size,
                ..
            } => Some(crate::menus::action_menu_item_geometry(
                bounds,
                label,
                hint.as_ref(),
                *icon,
                *danger,
                *disabled,
                *size,
                style,
                &self.style_model.palette,
            )),
            StandardVisual::TreeView { rows, size } => Some(crate::tree_view::tree_view_geometry(
                bounds,
                rows,
                *size,
                &self.style_model.palette,
            )),
            StandardVisual::CommandPalette {
                title,
                query,
                placeholder,
                empty,
                rows,
            } => Some(crate::command_palette::command_palette_geometry(
                bounds,
                title,
                query,
                placeholder,
                empty.as_ref(),
                rows,
                &self.style_model.palette,
            )),
            StandardVisual::QrCode { modules, width } => {
                let (module_size, (ox, oy)) = crate::qr_code::module_geometry(bounds, *width);
                let quiet = crate::qr_code::QUIET_ZONE_MODULES as f32;
                let dark = modules
                    .iter()
                    .enumerate()
                    .filter(|(_, dark)| **dark)
                    .map(|(index, _)| {
                        let x = index % *width;
                        let y = index / *width;
                        LayoutBox {
                            x: bounds.x + ox + (x as f32 + quiet) * module_size,
                            y: bounds.y + oy + (y as f32 + quiet) * module_size,
                            width: module_size,
                            height: module_size,
                        }
                    })
                    .collect();
                Some(crate::ComponentGeometry::QrCode {
                    field: LayoutBox {
                        x: bounds.x + ox,
                        y: bounds.y + oy,
                        width: module_size * (*width as f32 + quiet * 2.0),
                        height: module_size * (*width as f32 + quiet * 2.0),
                    },
                    module_size,
                    dark,
                })
            }
            StandardVisual::CalendarHeatmap {
                cells,
                month_labels,
                day_labels,
                cell_size,
                max_level,
                active,
                active_title,
                ..
            } => Some(calendar_heatmap_geometry(
                bounds,
                cells,
                month_labels,
                day_labels,
                *cell_size,
                *max_level,
                *active,
                active_title.as_deref(),
                self.style_model.theme_mode,
                &self.style_model.palette,
            )),
            StandardVisual::TimeSeriesChart { values } => Some(time_series_geometry(
                bounds,
                values,
                self.style_model.theme_mode,
            )),
            StandardVisual::ReorderList {
                rows,
                size,
                spacing,
                insert,
            } => Some(reorder_list_geometry(
                bounds,
                rows,
                *size,
                *spacing,
                *insert,
                &self.style_model.palette,
            )),
            StandardVisual::NativeMarkdown { text, selection } => {
                let (text, selection, selection_color) = selectable_text_regions(
                    content,
                    text,
                    *selection,
                    style,
                    &self.style_model.palette,
                );
                Some(crate::ComponentGeometry::NativeMarkdown {
                    text,
                    selection,
                    selection_color,
                })
            }
            StandardVisual::SelectableRichText { text, selection } => {
                let (text, selection, selection_color) = selectable_text_regions(
                    content,
                    text,
                    *selection,
                    style,
                    &self.style_model.palette,
                );
                Some(crate::ComponentGeometry::SelectableRichText {
                    text,
                    selection,
                    selection_color,
                })
            }
            StandardVisual::GraphCanvas {
                nodes,
                ports,
                edges,
                connecting,
                grid_spacing,
                viewport_offset_x,
                viewport_offset_y,
                viewport_zoom,
            } => Some(graph_canvas_geometry(
                bounds,
                nodes,
                ports,
                edges,
                connecting.as_ref(),
                *grid_spacing,
                *viewport_offset_x,
                *viewport_offset_y,
                *viewport_zoom,
                &self.style_model.palette,
            )),
            StandardVisual::ImageViewer {
                name,
                metadata,
                zoom,
                offset_x,
                offset_y,
            } => Some(image_viewer_geometry(
                bounds,
                name.as_ref(),
                metadata.as_ref(),
                *zoom,
                *offset_x,
                *offset_y,
                &self.style_model.palette,
            )),
            StandardVisual::KeyCaptureLayer { recording } => Some(key_capture_geometry(
                content,
                *recording,
                &self.style_model.palette,
            )),
            StandardVisual::KeymapLayer => {
                Some(keymap_geometry(content, &self.style_model.palette))
            }
            _ => None,
        }
    }

    fn project_accessibility_node(&self, id: StableNodeId) -> Option<AccessibilityNode> {
        if !self.is_mounted(id) {
            return None;
        }
        let entity = *self.entities.get(&id)?;
        let identity = self.world.get::<Identity>(entity)?;
        let style = self.world.get::<ResolvedStyle>(entity)?.0.as_ref();
        if !style.visible {
            return None;
        }
        let hierarchy = self.world.get::<Hierarchy>(entity)?;
        let state = self.world.get::<AccessibilityState>(entity)?;
        let kind = self.world.get::<Kind>(entity)?.0.as_ref();
        if matches!(kind, NodeKind::Comment) {
            return None;
        }
        let role = match (state.role, kind) {
            (AccessibilityRole::Generic, NodeKind::Document) => AccessibilityRole::Document,
            (AccessibilityRole::Generic, NodeKind::Text) => AccessibilityRole::Text,
            (role, _) => role,
        };
        let label = state.label.clone().or_else(|| {
            self.world
                .get::<TextContent>(entity)
                .filter(|text| !text.value.is_empty())
                .map(|text| Arc::<str>::from(text.value.as_str()))
        });
        let bounds = match self.component_geometry(id) {
            Some(crate::ComponentGeometry::ModalFrame { surface, .. }) => surface,
            _ => self.visible_accessibility_bounds(id)?,
        };
        Some(AccessibilityNode {
            id,
            parent: hierarchy.parent,
            children: hierarchy
                .children
                .iter()
                .copied()
                .filter(|child| {
                    let child_id = *child;
                    let child = self.entities[&child_id];
                    self.world
                        .get::<ResolvedStyle>(child)
                        .is_some_and(|style| style.0.visible)
                        && self
                            .world
                            .get::<Kind>(child)
                            .is_some_and(|kind| !matches!(kind.0.as_ref(), NodeKind::Comment))
                        && self.visible_accessibility_bounds(child_id).is_some()
                })
                .collect(),
            role,
            label,
            description: state.description.clone(),
            value: if matches!(
                self.world.get::<StandardVisual>(entity),
                Some(StandardVisual::TextInput { secure: true, .. })
            ) {
                None
            } else {
                self.world
                    .get::<TextInputState>(entity)
                    .map(|input| Arc::<str>::from(input.value.as_str()))
                    .or_else(|| state.value.clone())
            },
            disabled: state.disabled
                || self
                    .confirm_action_effect(id)
                    .is_some_and(|effect| effect.0),
            checked: state.checked,
            mixed: state.mixed,
            orientation: state.orientation,
            selected: state.selected,
            multiline: state.multiline,
            editable: state.editable,
            selection: self
                .world
                .get::<TextInputState>(entity)
                .map(|input| input.selection),
            modal: state.modal,
            busy: state.busy,
            invalid: state.invalid,
            numeric_minimum: state.numeric_minimum,
            numeric_maximum: state.numeric_maximum,
            numeric_step: state.numeric_step,
            numeric_value: state.numeric_value,
            focused: self.focused.get(&identity.document) == Some(&id),
            bounds,
        })
    }

    fn visible_accessibility_bounds(&self, id: StableNodeId) -> Option<LayoutBox> {
        let mut bounds = *self.world.get::<LayoutBox>(*self.entities.get(&id)?)?;
        if self.clip_visuals == 0 {
            return Some(bounds);
        }
        let mut parent = self
            .world
            .get::<Hierarchy>(*self.entities.get(&id)?)?
            .parent;
        while let Some(ancestor) = parent {
            let entity = *self.entities.get(&ancestor)?;
            if matches!(
                self.world.get::<StandardVisual>(entity),
                Some(StandardVisual::EmptyState { .. })
            ) {
                bounds = intersect_layout_boxes(bounds, *self.world.get::<LayoutBox>(entity)?)?;
            }
            if let Some(crate::ComponentGeometry::ModalFrame { surface, body, .. }) =
                self.component_geometry(ancestor)
            {
                bounds = intersect_layout_boxes(bounds, surface)?;
                if let Some(StandardVisual::ModalFrame { slots, .. }) =
                    self.world.get::<StandardVisual>(entity)
                    && slots
                        .body
                        .is_some_and(|body_root| self.is_descendant_or_self(id, body_root))
                {
                    bounds = intersect_layout_boxes(bounds, body)?;
                }
            }
            parent = self.world.get::<Hierarchy>(entity)?.parent;
        }
        Some(bounds)
    }

    fn component_mut<T: Component<Mutability = Mutable>>(
        &mut self,
        id: StableNodeId,
    ) -> bevy_ecs::world::Mut<'_, T> {
        self.world
            .get_mut::<T>(self.entities[&id])
            .expect("entity must have runtime component")
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
            let entity = *self.entities.get(&ancestor)?;
            if let Some(StandardVisual::ModalFrame {
                kind: crate::ModalSurfaceKind::Confirm(_),
                busy,
                danger,
                slots,
                ..
            }) = self.world.get::<StandardVisual>(entity)
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
        let entity = *self
            .entities
            .get(&target)
            .ok_or(UiWorldError::MissingNode(target))?;
        let identity = self
            .world
            .get::<Identity>(entity)
            .expect("runtime entity must have identity");
        if identity.document != document {
            return Err(UiWorldError::PointerDocument { document, target });
        }
        if !self.is_mounted(target) {
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        let interaction = self
            .world
            .get::<InteractionState>(entity)
            .expect("runtime entity must have interaction state");
        if !interaction.pointer_events || !self.used_pointer_events(target).hittable() {
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        Ok(())
    }

    fn used_pointer_events(&self, id: StableNodeId) -> PointerEventsSpec {
        let mut current = Some(id);
        while let Some(node) = current {
            if let Some(specified) = self.component::<NodeStyle>(node).layout.pointer_events {
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
                let had_hover = self.pointer_hover.values().any(|target| target == &id);
                let had_press = self.pointer_press.values().any(|target| target == &id);
                self.pointer_hover.retain(|_, target| target != &id);
                self.pointer_press.retain(|_, target| target != &id);
                if had_hover || had_press {
                    cleared.push(id);
                }
            }
            stack.extend(self.component::<Hierarchy>(id).children.iter().copied());
        }
        if !cleared.is_empty() {
            self.generation = self.generation.wrapping_add(1);
            for id in cleared {
                self.mark_interaction_style(id);
            }
        }
    }

    fn mark_interaction_style(&mut self, id: StableNodeId) {
        self.mark(id, DirtyMask::STATE);
        if !self.component::<NodeStyle>(id).interaction.is_empty() {
            self.mark(id, DirtyMask::STYLE | DirtyMask::RENDER);
        }
    }

    fn remove_ime(&mut self, id: StableNodeId) {
        let entity = self.entities[&id];
        self.world.entity_mut(entity).remove::<ImeComposition>();
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
                let entity = *self.entities.get(&host)?;
                (!removed.contains(&host))
                    .then(|| self.world.get::<OverlayHostState>(entity).copied())
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
                        (state != previous).then_some((host, entity, state, restore_focus))
                    })
            })
            .collect::<Vec<_>>();
        for (host, entity, state, restore_focus) in updates {
            self.world.entity_mut(entity).insert(state);
            self.mark(host, DirtyMask::ACCESSIBILITY);
            let document = self.component::<Identity>(host).document;
            if let Some(restore_focus) = restore_focus.filter(|id| {
                self.contains(*id)
                    && self.is_mounted(*id)
                    && self.component::<Identity>(*id).document == document
                    && self.component::<InteractionState>(*id).focusable
                    && self.component::<ResolvedStyle>(*id).0.visible
                    && self.active_modal_allows_focus_now(document, *id)
            }) {
                self.focused.insert(document, restore_focus);
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

    fn semantic_paint(&self, id: StableNodeId, local: &NodeStyle) -> crate::SemanticPaint {
        let mut paint = crate::SemanticPaint {
            foreground: local.foreground,
            background: local.background,
            border: local.border,
        };
        let accessibility = self.component::<AccessibilityState>(id);
        let selected = accessibility.checked == Some(true)
            || accessibility.mixed
            || accessibility.selected == Some(true);
        if selected {
            paint = paint.overlay(local.interaction.selected);
        }
        if self.pointer_hover.values().any(|target| *target == id) {
            paint = paint.overlay(
                if selected && !local.interaction.selected_hovered.is_empty() {
                    local.interaction.selected_hovered
                } else {
                    local.interaction.hovered
                },
            );
        }
        if self.pointer_press.values().any(|target| *target == id) {
            paint = paint.overlay(
                if selected && !local.interaction.selected_pressed.is_empty() {
                    local.interaction.selected_pressed
                } else {
                    local.interaction.pressed
                },
            );
        }
        let identity = self.component::<Identity>(id);
        if self.focused.get(&identity.document) == Some(&id) {
            paint = paint.overlay(local.interaction.focused);
        }
        if accessibility.disabled && !accessibility.busy {
            paint = paint.overlay(local.interaction.disabled);
        }
        paint
    }

    fn inherited_palette_color(&self, mut parent: Option<StableNodeId>) -> Option<[f32; 4]> {
        let palette = self.style_model.palette;
        while let Some(id) = parent {
            let local = self.component::<NodeStyle>(id);
            if let Some(color) = local.layout.color {
                return Some(color);
            }
            let paint = self.semantic_paint(id, local);
            if let Some(role) = paint.foreground {
                return Some(palette.get(role).as_rgba_array());
            }
            parent = self.component::<Hierarchy>(id).parent;
        }
        None
    }

    fn palette_paint_colors(
        &self,
        id: StableNodeId,
        inherited_color: Option<[f32; 4]>,
    ) -> (
        SemanticColorRole,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
        Option<[f32; 4]>,
    ) {
        let local = self.component::<NodeStyle>(id);
        let paint = self.semantic_paint(id, local);
        let layout = local.layout.as_ref();
        let palette = self.style_model.palette;
        let parent = self.component::<Hierarchy>(id).parent;
        let inherited_foreground = parent
            .map(|parent| self.component::<ResolvedStyle>(parent).0.foreground)
            .unwrap_or(SemanticColorRole::Text);
        let foreground = paint.foreground.unwrap_or(inherited_foreground);
        let color = layout.color.or_else(|| {
            paint
                .foreground
                .map(|role| palette.get(role).as_rgba_array())
                .or(inherited_color)
                .or_else(|| Some(palette.get(foreground).as_rgba_array()))
        });
        let background = layout.background.or_else(|| {
            paint
                .background
                .map(|role| palette.get(role).as_rgba_array())
        });
        let border_color = layout
            .resolved_border_color()
            .or_else(|| paint.border.map(|role| palette.get(role).as_rgba_array()));
        (foreground, color, background, border_color)
    }

    fn resolve_style(
        &mut self,
        id: StableNodeId,
        resolved: &mut HashSet<StableNodeId>,
    ) -> Result<(), UiWorldError> {
        if !self.contains(id) {
            return Err(UiWorldError::MissingNode(id));
        }
        if !resolved.insert(id) {
            return Ok(());
        }
        let parent = self.component::<Hierarchy>(id).parent;
        if let Some(parent) = parent {
            self.resolve_style(parent, resolved)?;
        }
        let layout = Arc::clone(&self.component::<NodeStyle>(id).layout);
        let inherited = parent
            .map(|parent| self.component::<ResolvedStyle>(parent).0.as_ref().clone())
            .unwrap_or_default();
        let inherited_color =
            parent.and_then(|parent| self.component::<ResolvedStyle>(parent).0.color);
        let (foreground, color, background, border_color) =
            self.palette_paint_colors(id, inherited_color);
        let visibility = layout.paint.visibility.unwrap_or(inherited.visibility);
        let box_visible = !layout.omits_box() && inherited.box_visible;
        let next = ComputedStyle {
            foreground,
            color,
            background,
            border_color,
            opacity: layout.opacity.unwrap_or(1.0) * inherited.opacity,
            visibility,
            box_visible,
            visible: box_visible
                && visibility != nana_ui_core::VisibilitySpec::Hidden
                && self.overlay_branch_active(id)
                && self.menu_branch_open(id),
            font_size: layout.font_size.unwrap_or(inherited.font_size),
            font_weight: layout.font_weight.or(inherited.font_weight),
            italic: layout.font_italic.unwrap_or(inherited.italic),
            font_family: layout
                .font_family
                .as_deref()
                .map(Arc::<str>::from)
                .or(inherited.font_family),
            line_height: layout.line_height.or(inherited.line_height),
            letter_spacing: layout.letter_spacing.unwrap_or(inherited.letter_spacing),
            font_features: layout
                .font_features
                .clone()
                .unwrap_or(inherited.font_features),
        };
        {
            let resolved = self.component::<ResolvedStyle>(id);
            if resolved.0.as_ref() == &next && resolved.1 == self.palette_epoch {
                return Ok(());
            }
        }
        *self.component_mut::<ResolvedStyle>(id) =
            ResolvedStyle(Arc::new(next), self.palette_epoch);
        Ok(())
    }

    fn overlay_branch_active(&self, id: StableNodeId) -> bool {
        if self.overlay_host_nodes.is_empty() {
            return true;
        }
        let Some(parent) = self.component::<Hierarchy>(id).parent else {
            return true;
        };
        self.overlay_host(parent)
            .is_none_or(|state| state.active == Some(id))
    }

    /// A closed menu keeps its items in the tree but out of the frame. They
    /// would otherwise stretch the trigger they hang under, since the trigger
    /// and the surface share one box.
    fn menu_branch_open(&self, id: StableNodeId) -> bool {
        let Some(parent) = self.component::<Hierarchy>(id).parent else {
            return true;
        };
        let Some(&entity) = self.entities.get(&parent) else {
            return true;
        };
        menu_surface_open(self.world.get::<StandardVisual>(entity)) != Some(false)
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
        let Some(&entity) = self.entities.get(&id) else {
            for roots in self.live_document_roots.values_mut() {
                roots.remove(&id);
            }
            self.live_document_roots
                .retain(|_, roots| !roots.is_empty());
            return;
        };
        let document = self
            .world
            .get::<Identity>(entity)
            .expect("entity must have identity")
            .document;
        let parent = self
            .world
            .get::<Hierarchy>(entity)
            .expect("entity must have hierarchy")
            .parent;
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
            stack.extend(
                self.component::<Hierarchy>(id)
                    .children
                    .iter()
                    .rev()
                    .copied(),
            );
        }
        self.record_id_list_alloc(order.len());
        order
    }

    fn hierarchy_mut(&mut self, id: StableNodeId) -> bevy_ecs::world::Mut<'_, Hierarchy> {
        let entity = *self.entities.get(&id).expect("validated node must exist");
        self.world
            .get_mut::<Hierarchy>(entity)
            .expect("entity must have hierarchy")
    }

    fn mark(&mut self, id: StableNodeId, bits: u16) -> bool {
        let entity = self.entities[&id];
        let changed = self
            .world
            .get_mut::<DirtyMask>(entity)
            .expect("entity must have dirty component")
            .insert(bits);
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
            *self.component_mut::<MountState>(*id) = state;
        }
        if state == MountState::Mounted {
            self.mark_subtree(root, DirtyMask::ALL);
        }
    }

    fn unlink_from_parent(&mut self, id: StableNodeId) -> bool {
        let Some(parent) = self.node(id).expect("validated node must exist").parent else {
            return false;
        };
        let mut hierarchy = self.hierarchy_mut(parent);
        Arc::make_mut(&mut hierarchy.children).retain(|child| *child != id);
        intern_empty_children(&mut hierarchy.children);
        let _hierarchy = hierarchy;
        self.hierarchy_mut(id).parent = None;
        self.mark_ancestors(
            parent,
            DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
        );
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
            let document = self.component::<Identity>(id).document;
            if self.focused.get(&document) == Some(&id) {
                self.focused.remove(&document);
            }
            self.remove_ime(id);
            if let Some(index) = self.hit_test_index.get_mut(&document) {
                retain_hit_tree(index, id);
            }
            self.pending_render_removals.push(id);
            self.pending_accessibility_removals.push(id);
        }

        let released = self
            .pointer_captures
            .iter()
            .filter_map(|(&(document, pointer_id), &target)| {
                parked
                    .contains(&target)
                    .then_some((document, pointer_id, target))
            })
            .collect::<Vec<_>>();
        for (document, pointer_id, target) in released {
            self.pointer_captures.remove(&(document, pointer_id));
            self.pending_pointer_capture_changes
                .push(PointerCaptureChange {
                    pointer_id,
                    target,
                    captured: false,
                });
        }
        self.pointer_hover
            .retain(|_, target| !parked.contains(target));
        self.pointer_press
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
            if self.overlay_host(id).is_some() {
                self.world
                    .entity_mut(self.entities[&id])
                    .insert(OverlayHostState::default());
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
struct CountingShaper<'a, S: TextShaper> {
    inner: &'a mut S,
    cache: &'a mut crate::text_layout_cache::TextLayoutCache,
    glyphs: &'a mut crate::GlyphCache,
    runs: usize,
    wrap_layouts: usize,
}

impl<'a, S: TextShaper> CountingShaper<'a, S> {
    fn new(
        inner: &'a mut S,
        cache: &'a mut crate::text_layout_cache::TextLayoutCache,
        glyphs: &'a mut crate::GlyphCache,
    ) -> Self {
        Self {
            inner,
            cache,
            glyphs,
            runs: 0,
            wrap_layouts: 0,
        }
    }
}

impl<S: TextShaper> TextShaper for CountingShaper<'_, S> {
    fn shape(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> TextMetrics {
        let key = crate::text_layout_cache::TextLayoutKey::new(text, style, constraints);
        if let Some(metrics) = self.cache.lookup(&key) {
            return metrics;
        }
        self.runs = self.runs.saturating_add(1);
        if constraints.wrap {
            self.wrap_layouts = self.wrap_layouts.saturating_add(1);
        }
        let metrics = self
            .inner
            .shape_cached(id, text, style, constraints, self.glyphs);
        self.cache.insert(key, metrics);
        metrics
    }

    fn shape_cached(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
        _glyphs: &mut crate::GlyphCache,
    ) -> TextMetrics {
        self.shape(id, text, style, constraints)
    }
}

fn shape_empty_state_text(
    id: StableNodeId,
    visual: &StandardVisual,
    inherited: &ComputedStyle,
    max_width: Option<f32>,
    shaper: &mut impl TextShaper,
) -> EmptyStateTextPresentation {
    let StandardVisual::EmptyState {
        title,
        message,
        compact,
        ..
    } = visual
    else {
        return EmptyStateTextPresentation::default();
    };
    let mut title_style = inherited.clone();
    title_style.font_size = if *compact { 12.0 } else { 13.0 };
    title_style.font_weight = Some(600);
    title_style.line_height = None;
    let mut message_style = inherited.clone();
    message_style.font_size = if *compact { 11.0 } else { 12.0 };
    message_style.font_weight = None;
    message_style.line_height = None;
    let constraints = crate::TextShapeConstraints {
        max_width,
        wrap: max_width.is_some(),
        shaping: crate::TextShaping::Auto,
        ..crate::TextShapeConstraints::default()
    };
    EmptyStateTextPresentation {
        title: shaper.shape(
            id,
            &TextContent {
                value: title.to_string(),
            },
            &title_style,
            constraints,
        ),
        message: message.as_ref().map(|message| {
            shaper.shape(
                id,
                &TextContent {
                    value: message.to_string(),
                },
                &message_style,
                constraints,
            )
        }),
    }
}

fn shape_modal_text(
    id: StableNodeId,
    visual: &StandardVisual,
    inherited: &ComputedStyle,
    max_width: Option<f32>,
    shaper: &mut impl TextShaper,
) -> ModalTextPresentation {
    let StandardVisual::ModalFrame {
        title,
        description,
        body_text,
        ..
    } = visual
    else {
        return ModalTextPresentation::default();
    };
    let constraints = crate::TextShapeConstraints {
        max_width,
        wrap: max_width.is_some(),
        shaping: crate::TextShaping::Auto,
        ..Default::default()
    };
    let mut title_style = inherited.clone();
    title_style.font_size = 14.0;
    title_style.font_weight = Some(600);
    title_style.line_height = None;
    let mut description_style = inherited.clone();
    description_style.font_size = 12.0;
    description_style.font_weight = None;
    description_style.line_height = None;
    let mut body_style = inherited.clone();
    body_style.font_size = crate::overlay_surfaces::MODAL_BODY_TEXT_SIZE;
    body_style.font_weight = None;
    body_style.line_height = None;
    ModalTextPresentation {
        title: shaper.shape(
            id,
            &TextContent {
                value: title.to_string(),
            },
            &title_style,
            constraints,
        ),
        description: description.as_ref().map(|value| {
            shaper.shape(
                id,
                &TextContent {
                    value: value.to_string(),
                },
                &description_style,
                constraints,
            )
        }),
        body: body_text.as_ref().map(|value| {
            shaper.shape(
                id,
                &TextContent {
                    value: value.to_string(),
                },
                &body_style,
                constraints,
            )
        }),
    }
}

fn progress_geometry(
    bounds: LayoutBox,
    style: &ComputedStyle,
    value_ratio: f32,
    girth: f32,
    corner_radius: f32,
    label: Option<&Arc<str>>,
    cancellable: bool,
    default_label_color: [f32; 4],
) -> Option<crate::ComponentGeometry> {
    let ratio = value_ratio.clamp(0.0, 1.0);
    let girth = if girth.is_finite() && girth > 0.0 {
        girth
    } else {
        6.0
    };
    let cancel_size = 24.0_f32.min(bounds.height).min(bounds.width);
    let heading = if label.is_some() || cancellable {
        12.0_f32.max(if cancellable { cancel_size } else { 0.0 })
    } else {
        0.0
    };
    let cancel = cancellable.then(|| LayoutBox {
        x: bounds.x + (bounds.width - cancel_size).max(0.0),
        y: bounds.y + (heading - cancel_size).max(0.0) / 2.0,
        width: cancel_size,
        height: cancel_size,
    });
    let label_width = cancel
        .map(|cancel| (cancel.x - bounds.x - 8.0).max(0.0))
        .unwrap_or(bounds.width);
    let label_region = label.map(|label| crate::ComponentTextRegion {
        bounds: LayoutBox {
            x: bounds.x,
            y: bounds.y + (heading - 12.0).max(0.0) / 2.0,
            width: label_width,
            height: 12.0_f32.min(bounds.height),
        },
        content: Arc::clone(label),
        color: Some(style.color.unwrap_or(default_label_color)),
        font_size: 12.0,
        font_weight: Some(500),
    });
    let track = if heading > 0.0 {
        let track_y = bounds.y + heading + 6.0;
        LayoutBox {
            x: bounds.x,
            y: track_y,
            width: bounds.width,
            height: girth.min((bounds.y + bounds.height - track_y).max(0.0)),
        }
    } else {
        LayoutBox {
            x: bounds.x,
            y: bounds.y + (bounds.height - girth).max(0.0) / 2.0,
            width: bounds.width,
            height: girth.min(bounds.height),
        }
    };
    Some(crate::ComponentGeometry::Progress {
        fill: LayoutBox {
            width: track.width * ratio,
            ..track
        },
        track,
        label: label_region,
        cancel,
        corner_radius: corner_radius.max(0.0),
    })
}

fn form_field_geometry(
    bounds: LayoutBox,
    size: ControlSize,
    label: &Arc<str>,
    hint: Option<&Arc<str>>,
    error: Option<&Arc<str>>,
    control: Option<crate::StableNodeId>,
    layout_box: &dyn Fn(crate::StableNodeId) -> Option<LayoutBox>,
    palette: &SemanticPalette,
) -> Option<crate::ComponentGeometry> {
    let (label_size, _gap, label_role, label_weight) =
        crate::form_surfaces::form_field_density(size);
    let label_height = label_size * 1.2;
    let support = error.or(hint);
    let support_role = if error.is_some() {
        SemanticColorRole::Danger
    } else {
        SemanticColorRole::Muted
    };
    let support_height = 12.0_f32.min(bounds.height);
    let support_y = (bounds.y + bounds.height - support_height).max(bounds.y);
    let (indicator, support_x) = if error.is_some() {
        let slot = 12.0;
        let diameter = slot * 10.0 / 24.0;
        (
            Some((
                LayoutBox {
                    x: bounds.x + (slot - diameter) / 2.0,
                    y: support_y + (support_height - diameter) / 2.0,
                    width: diameter,
                    height: diameter,
                },
                palette.get(support_role).as_rgba_array(),
            )),
            bounds.x + slot + 5.0,
        )
    } else {
        (None, bounds.x)
    };
    Some(crate::ComponentGeometry::FormField {
        label: crate::ComponentTextRegion {
            bounds: LayoutBox {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: label_height.min(bounds.height),
            },
            content: Arc::clone(label),
            color: Some(palette.get(label_role).as_rgba_array()),
            font_size: label_size,
            font_weight: Some(label_weight),
        },
        support: support.map(|message| crate::ComponentTextRegion {
            bounds: LayoutBox {
                x: support_x,
                y: support_y,
                width: (bounds.x + bounds.width - support_x).max(0.0),
                height: support_height,
            },
            content: Arc::clone(message),
            color: Some(palette.get(support_role).as_rgba_array()),
            font_size: 11.0,
            font_weight: None,
        }),
        indicator,
        control: control.and_then(layout_box),
    })
}

fn text_input_placeholder_color(layout: &LayoutStyle, faint: [f32; 4]) -> [f32; 4] {
    let mut color = layout.placeholder_color.unwrap_or(faint);
    if let Some(opacity) = layout.placeholder_opacity {
        color[3] = (color[3] * opacity).clamp(0.0, 1.0);
    }
    color
}

fn status_tone_role(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    crate::components::status_tone_role(tone)
}

#[derive(Debug, Clone)]
struct TextInputPresentationSource {
    text: TextContent,
    placeholder: bool,
    selection: Option<(usize, usize)>,
    caret: usize,
    preedit: Option<(usize, usize)>,
    multiline: bool,
}

fn build_text_input_presentation_source(
    state: &TextInputState,
    ime: Option<&ImeComposition>,
    placeholder: &str,
    secure: bool,
    multiline: bool,
) -> TextInputPresentationSource {
    use unicode_segmentation::UnicodeSegmentation;

    let mask = |value: &str| {
        if secure {
            "•".repeat(value.graphemes(true).count())
        } else {
            value.to_owned()
        }
    };
    let display_offset = |value: &str, offset: usize| {
        if secure {
            value[..offset].graphemes(true).count() * "•".len()
        } else {
            offset
        }
    };
    if state.value.is_empty() && ime.is_none() && !placeholder.is_empty() {
        return TextInputPresentationSource {
            text: TextContent {
                value: placeholder.to_owned(),
            },
            placeholder: true,
            selection: None,
            caret: 0,
            preedit: None,
            multiline,
        };
    }

    let selection = if state.selection.is_valid_for(&state.value) {
        state.selection
    } else {
        crate::TextSelection::caret(state.value.len())
    };
    if let Some(ime) = ime {
        let replaced = selection.ordered();
        let prefix = mask(&state.value[..replaced.start]);
        let suffix = mask(&state.value[replaced.end..]);
        let preedit_start = prefix.len();
        let preedit_end = preedit_start + ime.text.len();
        let ime_focus = ime
            .selection
            .map(|(_, focus)| focus)
            .filter(|focus| *focus <= ime.text.len() && ime.text.is_char_boundary(*focus))
            .unwrap_or(ime.text.len());
        return TextInputPresentationSource {
            text: TextContent {
                value: format!("{prefix}{}{suffix}", ime.text),
            },
            placeholder: false,
            selection: None,
            caret: preedit_start + ime_focus,
            preedit: Some((preedit_start, preedit_end)),
            multiline,
        };
    }

    let anchor = display_offset(&state.value, selection.anchor);
    let focus = display_offset(&state.value, selection.focus);
    TextInputPresentationSource {
        text: TextContent {
            value: mask(&state.value),
        },
        placeholder: false,
        selection: (anchor != focus).then_some((anchor.min(focus), anchor.max(focus))),
        caret: focus,
        preedit: None,
        multiline,
    }
}

fn shape_text_input_presentation(
    id: StableNodeId,
    source: TextInputPresentationSource,
    style: &ComputedStyle,
    constraints: crate::TextShapeConstraints,
    shaper: &mut impl TextShaper,
) -> TextInputPresentation {
    // Editing geometry must remain available outside a clipped viewport so the
    // Runtime can scroll the caret into view. Single-line fields retain their
    // unwrapped presentation even if their authored style omits nowrap.
    let presentation_constraints = crate::TextShapeConstraints {
        max_width: if source.multiline {
            constraints.max_width
        } else {
            None
        },
        max_height: None,
        wrap: source.multiline && constraints.wrap,
        ellipsis: false,
        max_lines: None,
        shaping: constraints.shaping,
        preserve_lines: constraints.preserve_lines,
        wrap_break: constraints.wrap_break,
    };
    let (caret_x, caret_y, line_height) = shaper.text_position(
        id,
        &source.text,
        source.caret,
        style,
        presentation_constraints,
    );
    let selection_lines = source.selection.map_or_else(Vec::new, |selection| {
        shaper.text_highlights(id, &source.text, selection, style, presentation_constraints)
    });
    let preedit_lines = source.preedit.map_or_else(Vec::new, |preedit| {
        shaper.text_highlights(id, &source.text, preedit, style, presentation_constraints)
    });
    TextInputPresentation {
        display_value: source.text.value.clone(),
        placeholder: source.placeholder,
        selection: source.selection.map(|(start, end)| {
            (
                shaper.horizontal_offset(id, &source.text, start, style),
                shaper.horizontal_offset(id, &source.text, end, style),
            )
        }),
        selection_lines: if source.multiline {
            selection_lines
        } else {
            Vec::new()
        },
        caret_x,
        caret_y: if source.multiline { caret_y } else { 0.0 },
        line_height,
        preedit: source.preedit.map(|(start, end)| {
            (
                shaper.horizontal_offset(id, &source.text, start, style),
                shaper.horizontal_offset(id, &source.text, end, style),
            )
        }),
        preedit_lines: if source.multiline {
            preedit_lines
        } else {
            Vec::new()
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn text_input_decorations(
    presentation: &TextInputPresentation,
    multiline: bool,
    content: LayoutBox,
    line_y: f32,
    line_height: f32,
    scroll_x: f32,
    scroll_y: f32,
) -> (Vec<LayoutBox>, Vec<LayoutBox>) {
    let field_x = |offset: f32| content.x + offset - scroll_x;
    if multiline {
        let selection = presentation
            .selection_lines
            .iter()
            .map(|selection| LayoutBox {
                x: field_x(selection.x),
                y: content.y + selection.y - scroll_y,
                width: selection.width,
                height: selection.height,
            })
            .collect();
        let preedit = presentation
            .preedit_lines
            .iter()
            .map(|preedit| LayoutBox {
                x: field_x(preedit.x),
                y: content.y + preedit.y + preedit.height - scroll_y - 2.0,
                width: preedit.width.max(1.0),
                height: 2.0,
            })
            .collect();
        (selection, preedit)
    } else {
        let selection = presentation
            .selection
            .map(|(start, end)| LayoutBox {
                x: field_x(start),
                y: line_y,
                width: (end - start).max(0.0),
                height: line_height,
            })
            .into_iter()
            .collect();
        let preedit = presentation
            .preedit
            .map(|(start, end)| LayoutBox {
                x: field_x(start),
                y: line_y + line_height - 2.0,
                width: (end - start).max(1.0),
                height: 2.0,
            })
            .into_iter()
            .collect();
        (selection, preedit)
    }
}

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

fn sort_hit_children(node: &mut HitEntry) {
    // Children were attached last-to-first; (z, order) restores document order
    // within a stacking level so a reverse walk is front-to-back.
    node.children
        .sort_by_key(|child| (child.z_index, child.order));
    for child in &mut node.children {
        sort_hit_children(child);
    }
}

/// Accumulated transform of `id`'s existing entry, used as the splice point for
/// a scoped rebuild so the patch never walks up to the document root.
fn find_hit_transform(forest: &[HitEntry], id: StableNodeId) -> Option<[f32; 6]> {
    for entry in forest {
        if entry.id == id {
            return Some(entry.transform);
        }
        if let Some(transform) = find_hit_transform(&entry.children, id) {
            return Some(transform);
        }
    }
    None
}

fn find_hit_entry_mut(forest: &mut [HitEntry], id: StableNodeId) -> Option<&mut HitEntry> {
    for entry in forest {
        if entry.id == id {
            return Some(entry);
        }
        if let Some(found) = find_hit_entry_mut(&mut entry.children, id) {
            return Some(found);
        }
    }
    None
}

fn count_hit_entries(entry: &HitEntry) -> usize {
    1 + entry.children.iter().map(count_hit_entries).sum::<usize>()
}

fn retain_hit_tree(nodes: &mut Vec<HitEntry>, id: StableNodeId) {
    nodes.retain_mut(|node| {
        if node.id == id {
            false
        } else {
            retain_hit_tree(&mut node.children, id);
            true
        }
    });
}

fn patch_hit_scroll(
    nodes: &mut [HitEntry],
    scroller: StableNodeId,
    subtree: &HashSet<StableNodeId>,
    delta: [f32; 2],
) {
    for node in nodes {
        if node.id != scroller && subtree.contains(&node.id) {
            let [a, b, c, d, e, f] = node.transform;
            node.transform = [
                a,
                b,
                c,
                d,
                a * delta[0] + c * delta[1] + e,
                b * delta[0] + d * delta[1] + f,
            ];
        }
        patch_hit_scroll(&mut node.children, scroller, subtree, delta);
    }
}

/// First candidate `collect_hit_candidates` would push for this subtree.
///
/// Mirrors that traversal exactly and returns at the first would-be push. Kept
/// beside it so the two orders stay in step; the pair is pinned by
/// `hit_test_matches_the_first_collected_candidate`.
fn first_hit_candidate(node: &HitEntry, x: f32, y: f32) -> Option<StableNodeId> {
    if !node
        .self_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y))
    {
        return None;
    }
    let menu_hit = node
        .menu
        .is_some_and(|menu| transformed_contains(menu, node.transform, node.persp, x, y));
    let children_ok = node
        .child_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y));
    let menu_z = node.z_index.max(1_000);
    if children_ok {
        for child in node.children.iter().rev() {
            if menu_hit && child.z_index <= menu_z {
                return Some(node.id);
            }
            if let Some(found) = first_hit_candidate(child, x, y) {
                return Some(found);
            }
        }
    }
    if menu_hit {
        return Some(node.id);
    }
    if node.hittable && transformed_contains(node.layout, node.transform, node.persp, x, y) {
        return Some(node.id);
    }
    None
}

fn collect_hit_candidates(node: &HitEntry, x: f32, y: f32, out: &mut Vec<StableNodeId>) {
    if !node
        .self_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y))
    {
        return;
    }
    let menu_hit = node
        .menu
        .is_some_and(|menu| transformed_contains(menu, node.transform, node.persp, x, y));
    let children_ok = node
        .child_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y));
    let menu_z = node.z_index.max(1_000);
    let mut emitted_menu = !menu_hit;
    if children_ok {
        for child in node.children.iter().rev() {
            if !emitted_menu && child.z_index <= menu_z {
                out.push(node.id);
                emitted_menu = true;
            }
            collect_hit_candidates(child, x, y, out);
        }
    }
    if !emitted_menu {
        out.push(node.id);
    }
    if node.hittable && transformed_contains(node.layout, node.transform, node.persp, x, y) {
        out.push(node.id);
    }
}

fn then_affine([a, b, c, d, e, f]: [f32; 6], rhs: [f32; 6]) -> [f32; 6] {
    then_hit(([a, b, c, d, e, f], [0.0, 0.0]), (rhs, [0.0, 0.0])).0
}

fn then_hit(
    (left, [lg, lh]): ([f32; 6], [f32; 2]),
    (right, [rg, rh]): ([f32; 6], [f32; 2]),
) -> ([f32; 6], [f32; 2]) {
    let [a, b, c, d, e, f] = left;
    let [ra, rb, rc, rd, re, rf] = right;
    let na = a * ra + c * rb + e * rg;
    let nb = b * ra + d * rb + f * rg;
    let nc = a * rc + c * rd + e * rh;
    let nd = b * rc + d * rd + f * rh;
    let ne = a * re + c * rf + e;
    let nf = b * re + d * rf + f;
    let ng = lg * ra + lh * rb + rg;
    let nh = lg * rc + lh * rd + rh;
    let ni = lg * re + lh * rf + 1.0;
    if !ni.is_finite() || ni.abs() < 1e-8 {
        return (IDENTITY_AFFINE, [0.0, 0.0]);
    }
    let inv = 1.0 / ni;
    (
        [na * inv, nb * inv, nc * inv, nd * inv, ne * inv, nf * inv],
        [ng * inv, nh * inv],
    )
}

fn transformed_contains(
    bounds: LayoutBox,
    [a, b, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
    x: f32,
    y: f32,
) -> bool {
    let det = a * (d - f * h) - c * (b - f * g) + e * (b * h - d * g);
    if !det.is_finite() || det.abs() <= f32::EPSILON {
        return false;
    }
    let inv = 1.0 / det;
    let ia = (d - f * h) * inv;
    let ic = (-c + e * h) * inv;
    let ie = (c * f - e * d) * inv;
    let ib = (-b + f * g) * inv;
    let id = (a - e * g) * inv;
    let if_ = (e * b - a * f) * inv;
    let ig = (b * h - d * g) * inv;
    let ih = (c * g - a * h) * inv;
    let ii = (a * d - c * b) * inv;
    if !ii.is_finite() || ii.abs() < 1e-8 {
        return false;
    }
    let w = ig * x + ih * y + ii;
    if !w.is_finite() || w.abs() < 1e-8 {
        return false;
    }
    let local_x = (ia * x + ic * y + ie) / w;
    let local_y = (ib * x + id * y + if_) / w;
    bounds.contains(local_x, local_y)
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
        || previous.float != next.float
        || previous.clear != next.clear
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

/// Validate-then-apply staging for one mutation batch.
///
/// Every field is an overlay over `source`, never a copy of it: validation cost
/// tracks the batch, not the retained world. `parked`, `pointer_captures`, and
/// `animations` fall back to `source` on a miss instead of being pre-filled, and
/// overlay-host walks use `UiWorld::overlay_host_ids`.
struct ValidationPlan<'a> {
    source: &'a UiWorld,
    nodes: HashMap<StableNodeId, PlannedNode>,
    removed: HashSet<StableNodeId>,
    newly_retired: HashSet<StableNodeId>,
    /// Mount overrides staged by this batch. Absent means "ask `source`".
    parked: HashMap<StableNodeId, bool>,
    interactions: HashMap<StableNodeId, InteractionState>,
    styles: HashMap<StableNodeId, NodeStyle>,
    focus: HashMap<DocumentId, Option<StableNodeId>>,
    /// Cloned from `source` on first pointer-capture mutation. Batches that do
    /// not touch capture never pay for it.
    pointer_captures: Option<HashMap<(DocumentId, u64), StableNodeId>>,
    /// Cloned from `source` on first animation mutation, same rationale.
    animations: Option<HashMap<AnimationId, AnimationSpec>>,
    text_inputs: HashMap<StableNodeId, Option<TextInputState>>,
    overlay_hosts: HashMap<StableNodeId, OverlayHostState>,
    accessibility: HashMap<StableNodeId, AccessibilityState>,
    /// Nodes visited by whole-set walks during this validation. Reported through
    /// `UiWorld::validation_nodes_scanned` so a reintroduced world scan fails a
    /// test instead of silently costing a frame.
    scanned: usize,
}

impl<'a> ValidationPlan<'a> {
    fn new(source: &'a UiWorld) -> Self {
        Self {
            source,
            nodes: HashMap::new(),
            removed: HashSet::new(),
            newly_retired: HashSet::new(),
            parked: HashMap::new(),
            interactions: HashMap::new(),
            styles: HashMap::new(),
            focus: HashMap::new(),
            pointer_captures: None,
            animations: None,
            text_inputs: HashMap::new(),
            overlay_hosts: HashMap::new(),
            accessibility: HashMap::new(),
            scanned: 0,
        }
    }

    /// Staged mount state. A node this batch created has no `MountState` in
    /// `source`, and `mount_state` returning `None` correctly reads as unparked.
    fn is_parked(&self, id: StableNodeId) -> bool {
        if let Some(&parked) = self.parked.get(&id) {
            return parked;
        }
        self.source.mount_state(id) == Some(MountState::Parked)
    }

    fn set_parked(&mut self, id: StableNodeId, parked: bool) {
        self.parked.insert(id, parked);
    }

    fn pointer_captures_mut(&mut self) -> &mut HashMap<(DocumentId, u64), StableNodeId> {
        if self.pointer_captures.is_none() {
            self.pointer_captures = Some(self.source.pointer_captures.clone());
        }
        self.pointer_captures
            .as_mut()
            .expect("initialized directly above")
    }

    fn pointer_capture(&self, document: DocumentId, pointer_id: u64) -> Option<StableNodeId> {
        match &self.pointer_captures {
            Some(captures) => captures.get(&(document, pointer_id)).copied(),
            None => self.source.pointer_capture(document, pointer_id),
        }
    }

    fn animations_mut(&mut self) -> &mut HashMap<AnimationId, AnimationSpec> {
        if self.animations.is_none() {
            self.animations = Some(
                self.source
                    .animations
                    .iter()
                    .map(|(&id, animation)| (id, animation.spec))
                    .collect(),
            );
        }
        self.animations
            .as_mut()
            .expect("initialized directly above")
    }

    /// Overlay hosts staged by this batch plus those already in `source`.
    fn overlay_host_candidates(&mut self) -> Vec<StableNodeId> {
        let mut hosts = self.overlay_hosts.keys().copied().collect::<HashSet<_>>();
        hosts.extend(self.source.overlay_host_ids());
        self.scanned = self.scanned.saturating_add(hosts.len());
        hosts.into_iter().collect()
    }

    fn validate(&mut self, mutations: &[UiMutation]) -> Result<(), UiWorldError> {
        for mutation in mutations {
            match mutation {
                UiMutation::Create { id, document, .. } => self.create(*id, *document)?,
                UiMutation::Insert {
                    parent,
                    child,
                    before,
                } => self.insert(*parent, *child, *before)?,
                UiMutation::Detach { id } => {
                    self.detach(*id)?;
                }
                UiMutation::ParkSubtree { root } => self.park(*root)?,
                UiMutation::DespawnSubtree { root } => self.despawn_subtree(*root)?,
                UiMutation::SetStyle { id, style } => {
                    self.node(*id)?;
                    let layout = style.layout.as_ref();
                    if layout.opacity.is_some_and(|opacity| {
                        !opacity.is_finite() || !(0.0..=1.0).contains(&opacity)
                    }) || layout
                        .font_size
                        .is_some_and(|size| !size.is_finite() || size <= 0.0)
                        || layout
                            .letter_spacing
                            .is_some_and(|spacing| !spacing.is_finite())
                        || layout
                            .font_weight
                            .is_some_and(|weight| !(1..=1000).contains(&weight))
                        || layout.color.is_some_and(|color| {
                            color.into_iter().any(|channel| {
                                !channel.is_finite() || !(0.0..=1.0).contains(&channel)
                            })
                        })
                        || layout.background.is_some_and(|color| {
                            color.into_iter().any(|channel| {
                                !channel.is_finite() || !(0.0..=1.0).contains(&channel)
                            })
                        })
                        || layout.border_color.is_some_and(|color| {
                            color.into_iter().any(|channel| {
                                !channel.is_finite() || !(0.0..=1.0).contains(&channel)
                            })
                        })
                    {
                        return Err(UiWorldError::InvalidStyle(*id));
                    }
                    self.styles.insert(*id, style.clone());
                }
                UiMutation::SetTheme { .. } => {}
                UiMutation::SetText { id, .. } => {
                    self.node(*id)?;
                }
                UiMutation::WriteLayout { id, layout } => {
                    self.node(*id)?;
                    if !layout.x.is_finite()
                        || !layout.y.is_finite()
                        || !layout.width.is_finite()
                        || !layout.height.is_finite()
                        || layout.width < 0.0
                        || layout.height < 0.0
                    {
                        return Err(UiWorldError::InvalidLayout(*id));
                    }
                }
                UiMutation::SetScrollOffset { id, offset } => {
                    self.node(*id)?;
                    if !offset.x.is_finite()
                        || !offset.y.is_finite()
                        || offset.x < 0.0
                        || offset.y < 0.0
                    {
                        return Err(UiWorldError::InvalidScrollOffset(*id));
                    }
                }
                UiMutation::SetScrollMetrics { id, metrics } => {
                    self.node(*id)?;
                    if metrics.is_some_and(|metrics| {
                        [
                            metrics.viewport_width,
                            metrics.viewport_height,
                            metrics.content_width,
                            metrics.content_height,
                        ]
                        .into_iter()
                        .any(|extent| !extent.is_finite() || extent < 0.0)
                    }) {
                        return Err(UiWorldError::InvalidScrollMetrics(*id));
                    }
                }
                UiMutation::SetInteraction { id, interaction } => {
                    self.node(*id)?;
                    self.interactions.insert(*id, *interaction);
                }
                UiMutation::SetCustomRender { id, content } => {
                    self.node(*id)?;
                    if content.as_ref().is_some_and(|content| {
                        content.renderer.trim().is_empty() || content.resource.trim().is_empty()
                    }) {
                        return Err(UiWorldError::InvalidCustomRender(*id));
                    }
                }
                UiMutation::SetEventListener { id, event, .. } => {
                    self.node(*id)?;
                    if event.trim().is_empty() {
                        return Err(UiWorldError::InvalidEventListener(*id));
                    }
                }
                UiMutation::SetComponentType { id, .. } => {
                    self.node(*id)?;
                }
                UiMutation::SetStandardVisual { id, visual } => {
                    self.node(*id)?;
                    let invalid_ratio = match visual {
                        Some(StandardVisual::Range { ratio, .. })
                        | Some(StandardVisual::Progress {
                            value_ratio: ratio, ..
                        })
                        | Some(StandardVisual::LevelMeter {
                            value_ratio: ratio, ..
                        }) => !ratio.is_finite() || !(0.0..=1.0).contains(ratio),
                        _ => false,
                    };
                    if invalid_ratio {
                        return Err(UiWorldError::InvalidStandardVisual(*id));
                    }
                }
                UiMutation::SetAccessibility { id, accessibility } => {
                    self.node(*id)?;
                    self.accessibility.insert(*id, accessibility.clone());
                }
                UiMutation::SetOverlayHost { host, state } => {
                    let host_document = self.node(*host)?.document;
                    if let Some(active) = state.active {
                        let active_node = self.node(active)?;
                        if active_node.parent != Some(*host) {
                            return Err(UiWorldError::InvalidOverlayHost(*host));
                        }
                    }
                    if let Some(restore_focus) = state.restore_focus
                        && self.node(restore_focus)?.document != host_document
                    {
                        return Err(UiWorldError::FocusDocument {
                            document: host_document,
                            target: restore_focus,
                        });
                    }
                    self.overlay_hosts.insert(*host, *state);
                }
                UiMutation::CapturePointer { pointer_id, target } => {
                    let document = self.node(*target)?.document;
                    if self.is_parked(*target) {
                        return Err(UiWorldError::NotPointerInteractive(*target));
                    }
                    self.pointer_captures_mut()
                        .insert((document, *pointer_id), *target);
                }
                UiMutation::ReleasePointer { pointer_id, target } => {
                    let document = self.node(*target)?.document;
                    if self.pointer_capture(document, *pointer_id) != Some(*target) {
                        return Err(UiWorldError::PointerCaptureMismatch {
                            pointer_id: *pointer_id,
                            target: *target,
                        });
                    }
                    self.pointer_captures_mut().remove(&(document, *pointer_id));
                }
                UiMutation::StartAnimation { animation } => {
                    self.node(animation.target)?;
                    if !animation.is_valid() || self.is_parked(animation.target) {
                        return Err(UiWorldError::InvalidAnimation(animation.id));
                    }
                    self.animations_mut().insert(animation.id, *animation);
                }
                UiMutation::StopAnimation { id } => {
                    if self.animations_mut().remove(id).is_none() {
                        return Err(UiWorldError::MissingAnimation(*id));
                    }
                }
                UiMutation::RequestFocus { document, target } => {
                    if let Some(target) = target {
                        let node = self.node(*target)?;
                        if node.document != *document {
                            return Err(UiWorldError::FocusDocument {
                                document: *document,
                                target: *target,
                            });
                        }
                        let interaction =
                            self.interactions.get(target).copied().unwrap_or_else(|| {
                                self.source
                                    .entities
                                    .get(target)
                                    .map(|_| *self.source.component::<InteractionState>(*target))
                                    .unwrap_or_default()
                            });
                        let visible = self.focus_target_visible(*target)?;
                        if !interaction.focusable
                            || !visible
                            || !self.active_modal_allows_focus(*document, *target)?
                        {
                            return Err(UiWorldError::NotFocusable(*target));
                        }
                    }
                    self.focus.insert(*document, *target);
                }
                UiMutation::SetIme { id, composition } => {
                    let document = self.node(*id)?.document;
                    if composition.is_some() && self.is_parked(*id) {
                        return Err(UiWorldError::NotFocused(*id));
                    }
                    let focused = self
                        .focus
                        .get(&document)
                        .copied()
                        .unwrap_or_else(|| self.source.focused(document));
                    if focused != Some(*id) {
                        return Err(UiWorldError::NotFocused(*id));
                    }
                    if composition.is_some() {
                        self.text_input(*id)?;
                    }
                    if let Some(ImeComposition {
                        text,
                        selection: Some((start, end)),
                    }) = composition
                        && (start > end
                            || *end > text.len()
                            || !text.is_char_boundary(*start)
                            || !text.is_char_boundary(*end)
                            || !crate::TextSelection {
                                anchor: *start,
                                focus: *end,
                            }
                            .is_valid_for(text))
                    {
                        return Err(UiWorldError::InvalidIme(*id));
                    }
                }
                UiMutation::SetTextInput { id, state } => {
                    self.node(*id)?;
                    if state
                        .as_ref()
                        .is_some_and(|state| !state.selection.is_valid_for(&state.value))
                    {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    self.text_inputs.insert(*id, state.clone());
                }
                UiMutation::SetTextSelection { id, selection } => {
                    let mut state = self.text_input(*id)?;
                    if !selection.is_valid_for(&state.value) {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    state.selection = *selection;
                    self.text_inputs.insert(*id, Some(state));
                }
                UiMutation::ReplaceTextSelection { id, text } => {
                    let mut state = self.text_input(*id)?;
                    if !state.replace_selection(text) {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    self.text_inputs.insert(*id, Some(state));
                }
                UiMutation::SetHighlightRequest { id, request } => {
                    self.node(*id)?;
                    if request
                        .as_ref()
                        .is_some_and(|request| request.presenter.trim().is_empty())
                    {
                        return Err(UiWorldError::InvalidHighlightRequest(*id));
                    }
                }
            }
        }
        self.validate_overlay_hosts()?;
        Ok(())
    }

    fn create(&mut self, id: StableNodeId, document: DocumentId) -> Result<(), UiWorldError> {
        if self.exists(id) {
            return Err(UiWorldError::DuplicateNode(id));
        }
        if self.source.is_retired(id) || self.newly_retired.contains(&id) {
            return Err(UiWorldError::RetiredNode(id));
        }
        self.removed.remove(&id);
        self.set_parked(id, false);
        self.nodes.insert(
            id,
            PlannedNode {
                document,
                parent: None,
                children: Vec::new(),
            },
        );
        Ok(())
    }

    fn insert(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
        before: Option<StableNodeId>,
    ) -> Result<(), UiWorldError> {
        let parent_document = self.node(parent)?.document;
        let child_node = self.node(child)?.clone();
        if child_node.document != parent_document {
            return Err(UiWorldError::CrossDocument { parent, child });
        }
        if parent == child || self.has_ancestor(parent, child)? {
            return Err(UiWorldError::Cycle { parent, child });
        }
        if before == Some(child) && child_node.parent == Some(parent) {
            return Ok(());
        }
        if let Some(before) = before
            && !self.node(parent)?.children.contains(&before)
        {
            return Err(UiWorldError::InvalidBefore { parent, before });
        }
        let depth = self
            .ancestor_depth(parent)?
            .saturating_add(self.subtree_height(child)?);
        if depth > MAX_TREE_DEPTH {
            return Err(UiWorldError::TreeTooDeep {
                parent,
                child,
                depth,
            });
        }
        self.detach(child)?;
        let siblings = &mut self.node_mut(parent)?.children;
        let index = before
            .and_then(|before| siblings.iter().position(|id| *id == before))
            .unwrap_or(siblings.len());
        siblings.insert(index, child);
        self.node_mut(child)?.parent = Some(parent);
        let parked = self.is_parked(parent);
        self.set_parked_subtree(child, parked)?;
        Ok(())
    }

    /// Levels from `id` up to its root, counting `id` itself.
    fn ancestor_depth(&mut self, id: StableNodeId) -> Result<usize, UiWorldError> {
        let mut depth = 1;
        let mut cursor = self.node(id)?.parent;
        while let Some(ancestor) = cursor {
            depth += 1;
            if depth > MAX_TREE_DEPTH {
                // `has_ancestor` already rejected cycles, so this is genuine
                // depth rather than a loop.
                return Ok(depth);
            }
            cursor = self.node(ancestor)?.parent;
        }
        Ok(depth)
    }

    /// Levels from `root` down to its deepest descendant, counting `root`.
    /// Stops climbing once past the limit; the caller only needs to know that.
    fn subtree_height(&mut self, root: StableNodeId) -> Result<usize, UiWorldError> {
        let mut frontier = vec![root];
        let mut height = 0;
        while !frontier.is_empty() {
            height += 1;
            if height > MAX_TREE_DEPTH {
                return Ok(height);
            }
            let mut next = Vec::new();
            for id in frontier {
                next.extend(self.node(id)?.children.iter().copied());
            }
            frontier = next;
        }
        Ok(height)
    }

    fn park(&mut self, root: StableNodeId) -> Result<(), UiWorldError> {
        self.detach(root)?;
        let subtree = self.subtree(root)?;
        self.set_parked_subtree(root, true)?;
        let parked = subtree.iter().copied().collect::<HashSet<_>>();
        let documents = subtree
            .iter()
            .map(|id| self.node(*id).map(|node| node.document))
            .collect::<Result<HashSet<_>, _>>()?;
        for document in documents {
            let focused = self
                .focus
                .get(&document)
                .copied()
                .unwrap_or_else(|| self.source.focused(document));
            if focused.is_some_and(|target| parked.contains(&target)) {
                self.focus.insert(document, None);
            }
        }
        if self.pointer_captures.is_some() || !self.source.pointer_captures.is_empty() {
            self.pointer_captures_mut()
                .retain(|_, target| !parked.contains(target));
        }
        if self.animations.is_some() || !self.source.animations.is_empty() {
            self.animations_mut()
                .retain(|_, animation| !parked.contains(&animation.target));
        }
        for id in subtree {
            self.clear_overlay_references(id);
        }
        Ok(())
    }

    fn subtree(&mut self, root: StableNodeId) -> Result<Vec<StableNodeId>, UiWorldError> {
        let mut subtree = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id)?.children.clone();
            stack.extend(children);
            subtree.push(id);
        }
        Ok(subtree)
    }

    fn set_parked_subtree(&mut self, root: StableNodeId, parked: bool) -> Result<(), UiWorldError> {
        for id in self.subtree(root)? {
            self.set_parked(id, parked);
        }
        Ok(())
    }

    fn detach(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
        let parent = self.node(id)?.parent;
        if let Some(parent) = parent {
            self.node_mut(parent)?.children.retain(|child| *child != id);
            self.node_mut(id)?.parent = None;
        }
        Ok(())
    }

    fn despawn_subtree(&mut self, root: StableNodeId) -> Result<(), UiWorldError> {
        let subtree = self.subtree(root)?;
        let removed = subtree.iter().copied().collect::<HashSet<_>>();
        let documents = subtree
            .iter()
            .map(|id| self.node(*id).map(|node| node.document))
            .collect::<Result<HashSet<_>, _>>()?;
        for document in documents {
            let focused = self
                .focus
                .get(&document)
                .copied()
                .unwrap_or_else(|| self.source.focused(document));
            if focused.is_some_and(|target| removed.contains(&target)) {
                self.focus.insert(document, None);
            }
        }
        self.detach(root)?;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id)?.children.clone();
            stack.extend(children);
            self.nodes.remove(&id);
            self.removed.insert(id);
            self.newly_retired.insert(id);
            self.parked.remove(&id);
            if self.pointer_captures.is_some() || !self.source.pointer_captures.is_empty() {
                self.pointer_captures_mut()
                    .retain(|_, target| *target != id);
            }
            if self.animations.is_some() || !self.source.animations.is_empty() {
                self.animations_mut()
                    .retain(|_, animation| animation.target != id);
            }
            self.text_inputs.remove(&id);
            self.clear_overlay_references(id);
        }
        Ok(())
    }

    fn clear_overlay_references(&mut self, removed: StableNodeId) {
        for host in self.overlay_host_candidates() {
            if host == removed || !self.exists(host) {
                continue;
            }
            let Some(mut state) = self
                .overlay_hosts
                .get(&host)
                .copied()
                .or_else(|| self.source.overlay_host(host))
            else {
                continue;
            };
            if state.active == Some(removed) {
                state.active = None;
                state.restore_focus = None;
            }
            if state.restore_focus == Some(removed) {
                state.restore_focus = None;
            }
            self.overlay_hosts.insert(host, state);
        }
    }

    fn validate_overlay_hosts(&mut self) -> Result<(), UiWorldError> {
        for host in self.overlay_host_candidates() {
            if !self.exists(host) {
                continue;
            }
            let Some(state) = self
                .overlay_hosts
                .get(&host)
                .copied()
                .or_else(|| self.source.overlay_host(host))
            else {
                continue;
            };
            let host_document = self.node(host)?.document;
            if let Some(active) = state.active
                && (!self.exists(active) || self.node(active)?.parent != Some(host))
            {
                return Err(UiWorldError::InvalidOverlayHost(host));
            }
            if let Some(active) = state.active {
                let accessibility = self
                    .accessibility
                    .get(&active)
                    .or_else(|| self.source.accessibility(active));
                if !accessibility.is_some_and(|accessibility| match accessibility.role {
                    AccessibilityRole::Dialog | AccessibilityRole::AlertDialog => {
                        accessibility.modal
                    }
                    AccessibilityRole::Menu
                    | AccessibilityRole::Tooltip
                    | AccessibilityRole::Status => true,
                    _ => false,
                }) {
                    return Err(UiWorldError::InvalidOverlayHost(host));
                }
            }
            if let Some(restore_focus) = state.restore_focus
                && (!self.exists(restore_focus)
                    || self.node(restore_focus)?.document != host_document)
            {
                return Err(UiWorldError::InvalidOverlayHost(host));
            }
        }
        Ok(())
    }

    fn has_ancestor(
        &mut self,
        mut id: StableNodeId,
        candidate: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        let mut visited = HashSet::new();
        loop {
            if id == candidate {
                return Ok(true);
            }
            if !visited.insert(id) {
                return Ok(false);
            }
            let Some(parent) = self.node(id)?.parent else {
                return Ok(false);
            };
            id = parent;
        }
    }

    fn exists(&self, id: StableNodeId) -> bool {
        !self.removed.contains(&id) && (self.nodes.contains_key(&id) || self.source.contains(id))
    }

    fn text_input(&mut self, id: StableNodeId) -> Result<TextInputState, UiWorldError> {
        self.node(id)?;
        if let Some(state) = self.text_inputs.get(&id) {
            return state.clone().ok_or(UiWorldError::MissingTextInput(id));
        }
        self.source
            .text_input(id)
            .cloned()
            .ok_or(UiWorldError::MissingTextInput(id))
    }

    fn overlay_branch_active(&mut self, id: StableNodeId) -> Result<bool, UiWorldError> {
        let Some(parent) = self.node(id)?.parent else {
            return Ok(true);
        };
        let state = self
            .overlay_hosts
            .get(&parent)
            .copied()
            .or_else(|| self.source.overlay_host(parent));
        Ok(state.is_none_or(|state| state.active == Some(id)))
    }

    fn focus_target_visible(&mut self, mut id: StableNodeId) -> Result<bool, UiWorldError> {
        loop {
            if self.is_parked(id) {
                return Ok(false);
            }
            let layout = self
                .styles
                .get(&id)
                .map(|style| style.layout.as_ref())
                .or_else(|| {
                    self.source
                        .node_style(id)
                        .map(|style| style.layout.as_ref())
                });
            if layout.is_some_and(|layout| layout.omits_box())
                || !self.overlay_branch_active(id)?
                || self
                    .node(id)?
                    .parent
                    .and_then(|parent| self.source.standard_visual(parent))
                    .is_some_and(|visual| {
                        matches!(visual, StandardVisual::MenuSurface { open: false, .. })
                    })
            {
                return Ok(false);
            }
            let Some(parent) = self.node(id)?.parent else {
                return Ok(true);
            };
            id = parent;
        }
    }

    fn active_modal_allows_focus(
        &mut self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        let hosts = self.overlay_host_candidates();
        if hosts.is_empty() {
            return Ok(true);
        }
        // Document order only breaks ties between competing modals, so it stays
        // unevaluated until a first modal is found. Hosts that exist with no
        // active surface are the common case and must not pay for the walk.
        let mut order: Option<Vec<StableNodeId>> = None;
        let mut top = None;
        for host in hosts {
            if !self.exists(host) || self.is_parked(host) {
                continue;
            }
            let Some(state) = self
                .overlay_hosts
                .get(&host)
                .copied()
                .or_else(|| self.source.overlay_host(host))
            else {
                continue;
            };
            let Some(active) = state.active else {
                continue;
            };
            if !self.exists(active)
                || self.is_parked(active)
                || self.node(host)?.document != document
                || self.node(active)?.parent != Some(host)
                || !self.focus_target_visible(active)?
            {
                continue;
            }
            let modal = self
                .accessibility
                .get(&active)
                .or_else(|| self.source.accessibility(active))
                .is_some_and(|state| state.modal);
            if modal {
                let z = self
                    .styles
                    .get(&active)
                    .or_else(|| self.source.node_style(active))
                    .and_then(|style| style.layout.z_index)
                    .unwrap_or_default();
                if order.is_none() {
                    order = Some(self.planned_document_order(document)?);
                }
                let document_order = order
                    .as_ref()
                    .expect("document order resolved directly above")
                    .iter()
                    .position(|candidate| *candidate == active)
                    .unwrap_or_default();
                if top.is_none_or(|(top_z, top_order, _)| (z, document_order) > (top_z, top_order))
                {
                    top = Some((z, document_order, active));
                }
            }
        }
        top.map(|(_, _, active)| self.has_ancestor(target, active))
            .transpose()
            .map(|allowed| allowed.unwrap_or(true))
    }

    fn planned_document_order(
        &mut self,
        document: DocumentId,
    ) -> Result<Vec<StableNodeId>, UiWorldError> {
        let ids = self
            .source
            .entities
            .keys()
            .copied()
            .chain(self.nodes.keys().copied())
            .collect::<HashSet<_>>();
        self.scanned = self.scanned.saturating_add(ids.len());
        let mut roots = Vec::new();
        for id in ids {
            if self.exists(id)
                && !self.is_parked(id)
                && self.node(id)?.document == document
                && self.node(id)?.parent.is_none()
            {
                roots.push(id);
            }
        }
        roots.sort_unstable();
        let mut order = Vec::new();
        let mut stack = roots.into_iter().rev().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            if !self.exists(id) || self.is_parked(id) {
                continue;
            }
            order.push(id);
            let children = self.node(id)?.children.clone();
            stack.extend(children.into_iter().rev());
        }
        Ok(order)
    }

    fn node(&mut self, id: StableNodeId) -> Result<&PlannedNode, UiWorldError> {
        self.ensure(id)?;
        Ok(self.nodes.get(&id).expect("ensured node must exist"))
    }

    fn node_mut(&mut self, id: StableNodeId) -> Result<&mut PlannedNode, UiWorldError> {
        self.ensure(id)?;
        Ok(self.nodes.get_mut(&id).expect("ensured node must exist"))
    }

    fn ensure(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
        if self.removed.contains(&id) {
            return Err(UiWorldError::MissingNode(id));
        }
        if self.nodes.contains_key(&id) {
            return Ok(());
        }
        let snapshot = self.source.node(id).ok_or(UiWorldError::MissingNode(id))?;
        self.nodes.insert(
            id,
            PlannedNode {
                document: snapshot.document,
                parent: snapshot.parent,
                children: snapshot.children,
            },
        );
        Ok(())
    }
}

fn calendar_heatmap_geometry(
    bounds: LayoutBox,
    cells: &[crate::CalendarHeatmapCellPaint],
    month_labels: &[crate::CalendarHeatmapLabelPaint],
    day_labels: &[crate::CalendarHeatmapLabelPaint],
    cell_size: f32,
    max_level: u8,
    active: Option<usize>,
    active_title: Option<&str>,
    mode: ThemeMode,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    let painted = cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let is_active = Some(index) == active;
            let fill = if is_active && cell.level >= max_level {
                palette.accent
            } else {
                let level = if is_active {
                    cell.level.saturating_add(1).max(1)
                } else {
                    cell.level
                };
                crate::calendar_cell_fill(mode, level, max_level)
            };
            (
                LayoutBox {
                    x: bounds.x + cell.x,
                    y: bounds.y + cell.y,
                    width: cell_size,
                    height: cell_size,
                },
                fill.as_rgba_array(),
            )
        })
        .collect::<Vec<_>>();
    let mut labels = Vec::with_capacity(month_labels.len() + day_labels.len());
    labels.extend(month_labels.iter().map(|label| {
        axis_label_region(bounds, &label.text, label.x, label.y, 10.0, true, palette)
    }));
    labels.extend(day_labels.iter().map(|label| {
        axis_label_region(bounds, &label.text, label.x, label.y, 11.0, false, palette)
    }));
    let hover = active.and_then(|index| cells.get(index)).map(|cell| {
        calendar_hover_chrome(bounds, cell, cell_size, active_title.unwrap_or(""), palette)
    });
    crate::ComponentGeometry::CalendarHeatmap {
        cells: painted,
        labels,
        hover,
    }
}

fn calendar_hover_chrome(
    bounds: LayoutBox,
    cell: &crate::CalendarHeatmapCellPaint,
    cell_size: f32,
    title: &str,
    palette: &SemanticPalette,
) -> crate::CalendarHoverGeometry {
    let pad_x = TooltipConfig::PADDING_X;
    let pad_y = TooltipConfig::PADDING_Y;
    let font_size = TooltipConfig::FONT_SIZE;
    let gap = TooltipConfig::default().gap;
    let max_width = TooltipConfig::default().max_width;
    let text_width = estimated_text_width(title, font_size);
    let tooltip_width = (text_width + pad_x * 2.0).clamp(font_size + pad_x * 2.0, max_width);
    let tooltip_height = font_size + pad_y * 2.0;
    let ring = LayoutBox {
        x: bounds.x + cell.x - 1.0,
        y: bounds.y + cell.y - 1.0,
        width: cell_size + 2.0,
        height: cell_size + 2.0,
    };
    let tooltip_x = if cell.x > bounds.width / 2.0 {
        (cell.x + cell_size - tooltip_width).max(0.0)
    } else {
        cell.x.min((bounds.width - tooltip_width).max(0.0))
    };
    let tooltip_y = if cell.y < bounds.height / 2.0 {
        (cell.y + cell_size + gap).min((bounds.height - tooltip_height).max(0.0))
    } else {
        (cell.y - tooltip_height - gap).max(0.0)
    };
    let tooltip = LayoutBox {
        x: bounds.x + tooltip_x,
        y: bounds.y + tooltip_y,
        width: tooltip_width,
        height: tooltip_height,
    };
    crate::CalendarHoverGeometry {
        ring,
        tooltip,
        title: crate::ComponentTextRegion {
            bounds: LayoutBox {
                x: tooltip.x + pad_x,
                y: tooltip.y + pad_y,
                width: (tooltip_width - pad_x * 2.0).max(0.0),
                height: font_size,
            },
            content: Arc::from(title),
            color: Some(palette.text.as_rgba_array()),
            font_size,
            font_weight: None,
        },
        ring_color: palette.text.as_rgba_array(),
        tooltip_fill: palette.surface.as_rgba_array(),
        tooltip_border: palette.border_soft.as_rgba_array(),
    }
}

fn axis_label_region(
    bounds: LayoutBox,
    text: &Arc<str>,
    x: f32,
    y: f32,
    font_size: f32,
    center: bool,
    palette: &SemanticPalette,
) -> crate::ComponentTextRegion {
    let width = estimated_text_width(text, font_size) + 2.0;
    crate::ComponentTextRegion {
        bounds: LayoutBox {
            x: bounds.x + x - if center { width * 0.5 } else { 0.0 },
            y: bounds.y + y,
            width,
            height: font_size + 2.0,
        },
        content: Arc::clone(text),
        color: Some(palette.muted.as_rgba_array()),
        font_size,
        font_weight: None,
    }
}

fn time_series_geometry(
    bounds: LayoutBox,
    values: &[f64],
    mode: ThemeMode,
) -> crate::ComponentGeometry {
    let chart = crate::TimeSeriesChart::new(values.iter().copied());
    let paint = crate::time_series_paint(mode);
    let local = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: bounds.width,
        height: bounds.height,
    };
    let inset_x = crate::TimeSeriesChart::INSET_X;
    let grid = crate::TimeSeriesChart::grid_ys(local)
        .into_iter()
        .map(|y| LayoutBox {
            x: bounds.x + inset_x,
            y: bounds.y + y,
            width: (bounds.width - inset_x * 2.0).max(0.0),
            height: 1.0,
        })
        .collect();
    let points = chart
        .points(local)
        .into_iter()
        .map(|(x, y)| [bounds.x + x, bounds.y + y])
        .collect::<Vec<_>>();
    let baseline = bounds.y
        + (bounds.height - crate::TimeSeriesChart::INSET_Y).max(crate::TimeSeriesChart::INSET_Y);
    crate::ComponentGeometry::TimeSeriesChart {
        grid,
        area: area_under_polyline(&points, baseline),
        line: points,
        grid_color: paint.grid.as_rgba_array(),
        area_color: paint.area.as_rgba_array(),
        line_color: paint.line.as_rgba_array(),
    }
}

fn reorder_list_geometry(
    bounds: LayoutBox,
    rows: &[crate::ReorderRowPaint],
    size: ControlSize,
    spacing: f32,
    insert: Option<LayoutBox>,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    let height = size.height();
    let spacing = spacing.max(0.0);
    let pad = 8.0;
    let rows = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let row_bounds = LayoutBox {
                x: bounds.x,
                y: bounds.y + index as f32 * (height + spacing),
                width: bounds.width,
                height,
            };
            let label = crate::ComponentTextRegion {
                bounds: LayoutBox {
                    x: row_bounds.x + pad,
                    y: row_bounds.y,
                    width: (row_bounds.width - pad * 2.0).max(0.0),
                    height: row_bounds.height,
                },
                content: Arc::clone(&row.label),
                color: Some(if row.disabled {
                    palette.muted.as_rgba_array()
                } else {
                    palette.text.as_rgba_array()
                }),
                font_size: size.text_size(),
                font_weight: None,
            };
            let fill = row.selected.then_some(palette.selected.as_rgba_array());
            (row_bounds, label, fill)
        })
        .collect();
    crate::ComponentGeometry::ReorderList {
        rows,
        insert: insert.map(|line| (line, palette.accent.as_rgba_array())),
    }
}

fn selectable_text_regions(
    content: LayoutBox,
    text: &Arc<str>,
    selection: Option<(usize, usize)>,
    style: &ComputedStyle,
    palette: &SemanticPalette,
) -> (crate::ComponentTextRegion, Vec<LayoutBox>, [f32; 4]) {
    let region = crate::ComponentTextRegion {
        bounds: content,
        content: Arc::clone(text),
        color: Some(style.color.unwrap_or_else(|| palette.text.as_rgba_array())),
        font_size: style.font_size,
        font_weight: style.font_weight,
    };
    let highlights = if selection.is_some() {
        vec![content]
    } else {
        Vec::new()
    };
    (region, highlights, palette.accent_soft.as_rgba_array())
}

fn graph_canvas_geometry(
    bounds: LayoutBox,
    nodes: &[crate::GraphNodePaint],
    ports: &[crate::GraphPortPaint],
    edges: &[crate::GraphEdgePaint],
    connecting: Option<&crate::GraphEdgePaint>,
    grid_spacing: f32,
    viewport_offset_x: f32,
    viewport_offset_y: f32,
    viewport_zoom: f32,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    let mut painted_edges = Vec::new();
    let mut edge_labels = Vec::new();
    for edge in edges.iter().chain(connecting) {
        let color = graph_edge_stroke_color(palette, edge);
        painted_edges.push((sample_curve(bounds, edge.curve), color));
        if !edge.connecting
            && viewport_zoom >= 0.7
            && let Some(label) = edge.label.as_ref()
        {
            let center = cubic_point(edge.curve, 0.5);
            if let Some(label_bounds) = intersect_layout_boxes(
                bounds,
                LayoutBox {
                    x: bounds.x + center.x - 40.0,
                    y: bounds.y + center.y - 16.0,
                    width: 80.0,
                    height: 12.0,
                },
            ) {
                edge_labels.push(crate::ComponentTextRegion {
                    bounds: label_bounds,
                    content: Arc::clone(label),
                    color: Some(palette.muted.as_rgba_array()),
                    font_size: 10.0,
                    font_weight: None,
                });
            }
        }
    }
    let mut separators = Vec::new();
    let nodes = nodes
        .iter()
        .filter_map(|node| {
            let raw = LayoutBox {
                x: bounds.x + node.x,
                y: bounds.y + node.y,
                width: node.width.max(0.0),
                height: node.height.max(0.0),
            };
            let node_bounds = intersect_layout_boxes(bounds, raw)?;
            let title_height = node.title_height.clamp(18.0, node_bounds.height.max(18.0));
            if node_bounds.width >= 32.0
                && node_bounds.height >= title_height
                && let Some(separator) = intersect_layout_boxes(
                    bounds,
                    LayoutBox {
                        x: node_bounds.x,
                        y: node_bounds.y + title_height,
                        width: node_bounds.width,
                        height: 1.0,
                    },
                )
            {
                separators.push(separator);
            }
            let label = crate::ComponentTextRegion {
                bounds: intersect_layout_boxes(
                    bounds,
                    LayoutBox {
                        x: raw.x + 10.0,
                        y: raw.y,
                        width: (raw.width - 20.0).max(0.0),
                        height: title_height.min(raw.height),
                    },
                )
                .unwrap_or(LayoutBox {
                    x: node_bounds.x,
                    y: node_bounds.y,
                    width: 0.0,
                    height: 0.0,
                }),
                content: Arc::clone(&node.label),
                color: Some(palette.text.as_rgba_array()),
                font_size: (12.0 * viewport_zoom).clamp(9.0, 13.0),
                font_weight: Some(500),
            };
            let fill = if node.selected {
                palette.selected.as_rgba_array()
            } else if node.hovered {
                palette.hover.as_rgba_array()
            } else {
                palette.surface.as_rgba_array()
            };
            let border = if node.selected {
                Some(palette.border_strong.as_rgba_array())
            } else {
                Some(palette.border.as_rgba_array())
            };
            Some((node_bounds, label, fill, border))
        })
        .collect();
    let mut port_labels = Vec::new();
    let ports = ports
        .iter()
        .filter_map(|port| {
            let radius = port.radius.max(0.0);
            let disc = intersect_layout_boxes(
                bounds,
                LayoutBox {
                    x: bounds.x + port.x - radius,
                    y: bounds.y + port.y - radius,
                    width: radius * 2.0,
                    height: radius * 2.0,
                },
            )?;
            let kind = match port.kind {
                GraphPortKind::Input => palette.muted.as_rgba_array(),
                GraphPortKind::Output => palette.accent.as_rgba_array(),
                GraphPortKind::Bidirectional => palette.warning.as_rgba_array(),
            };
            if viewport_zoom >= 0.72 && !port.label.is_empty() {
                let (mut label, alignment) =
                    port_label_region(bounds, port, palette.muted.as_rgba_array());
                if let Some(clipped) = intersect_layout_boxes(bounds, label.bounds) {
                    label.bounds = clipped;
                    port_labels.push((label, alignment));
                }
            }
            Some((
                disc,
                palette.background.as_rgba_array(),
                kind,
                if port.selected { 2.4 } else { 1.6 },
            ))
        })
        .collect();
    crate::ComponentGeometry::GraphCanvas {
        nodes,
        separators,
        ports,
        port_labels,
        edges: painted_edges,
        edge_labels,
        grid: graph_grid_lines(
            bounds,
            grid_spacing,
            viewport_offset_x,
            viewport_offset_y,
            viewport_zoom,
        ),
        background: palette.background.as_rgba_array(),
        grid_color: {
            let mut color = palette.border_soft.as_rgba_array();
            color[3] *= 0.72;
            color
        },
        separator_color: palette.border_soft.as_rgba_array(),
    }
}

fn graph_edge_stroke_color(palette: &SemanticPalette, edge: &crate::GraphEdgePaint) -> [f32; 4] {
    if edge.connecting {
        let mut accent = palette.accent.as_rgba_array();
        accent[3] *= 0.8;
        accent
    } else if edge.selected {
        palette.text.as_rgba_array()
    } else if edge.hovered {
        palette.muted.as_rgba_array()
    } else {
        palette.border_strong.as_rgba_array()
    }
}

const CURVE_FLATNESS: f32 = 0.75;
const CURVE_MAX_SEGMENT: f32 = 8.0;

fn sample_curve(bounds: LayoutBox, curve: [GraphPoint; 4]) -> Vec<[f32; 2]> {
    let origin = [bounds.x, bounds.y];
    let mut points = vec![[origin[0] + curve[0].x, origin[1] + curve[0].y]];
    flatten_cubic(&mut points, curve, origin, 0);
    points
}

fn flatten_cubic(
    points: &mut Vec<[f32; 2]>,
    [p0, p1, p2, p3]: [GraphPoint; 4],
    origin: [f32; 2],
    depth: u32,
) {
    let chord = p0.distance_squared(p3).sqrt();
    let deviation = line_offset(p1, p0, p3).max(line_offset(p2, p0, p3));
    if depth >= 16 || (deviation <= CURVE_FLATNESS && chord <= CURVE_MAX_SEGMENT) {
        points.push([origin[0] + p3.x, origin[1] + p3.y]);
        return;
    }
    let mid = |a: GraphPoint, b: GraphPoint| GraphPoint::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p23 = mid(p2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let split = mid(p012, p123);
    flatten_cubic(points, [p0, p01, p012, split], origin, depth + 1);
    flatten_cubic(points, [split, p123, p23, p3], origin, depth + 1);
}

fn line_offset(point: GraphPoint, start: GraphPoint, end: GraphPoint) -> f32 {
    let abx = end.x - start.x;
    let aby = end.y - start.y;
    let length_sq = abx * abx + aby * aby;
    if length_sq <= f32::EPSILON {
        return point.distance_squared(start).sqrt();
    }
    ((point.x - start.x) * aby - (point.y - start.y) * abx).abs() / length_sq.sqrt()
}

fn graph_grid_lines(
    bounds: LayoutBox,
    base_spacing: f32,
    offset_x: f32,
    offset_y: f32,
    zoom: f32,
) -> Vec<LayoutBox> {
    if !base_spacing.is_finite() || base_spacing <= 0.0 {
        return Vec::new();
    }
    let mut spacing = base_spacing
        * if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        };
    while spacing < 16.0 {
        spacing *= 2.0;
    }
    while spacing > 96.0 {
        spacing *= 0.5;
    }
    if !spacing.is_finite() || spacing < 1.0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut x = offset_x.rem_euclid(spacing);
    while x <= bounds.width {
        lines.push(LayoutBox {
            x: bounds.x + x,
            y: bounds.y,
            width: 1.0,
            height: bounds.height,
        });
        x += spacing;
    }
    let mut y = offset_y.rem_euclid(spacing);
    while y <= bounds.height {
        lines.push(LayoutBox {
            x: bounds.x,
            y: bounds.y + y,
            width: bounds.width,
            height: 1.0,
        });
        y += spacing;
    }
    lines
}

fn port_label_region(
    bounds: LayoutBox,
    port: &crate::GraphPortPaint,
    color: [f32; 4],
) -> (crate::ComponentTextRegion, crate::TextHorizontalAlignment) {
    let (x, y, width, height, align) = match port.side {
        GraphPortSide::Top => (
            port.x - 40.0,
            port.y + 8.0,
            80.0,
            12.0,
            crate::TextHorizontalAlignment::Center,
        ),
        GraphPortSide::Right => (
            port.x - 88.0,
            port.y - 7.0,
            80.0,
            14.0,
            crate::TextHorizontalAlignment::End,
        ),
        GraphPortSide::Bottom => (
            port.x - 40.0,
            port.y - 20.0,
            80.0,
            12.0,
            crate::TextHorizontalAlignment::Center,
        ),
        GraphPortSide::Left => (
            port.x + 8.0,
            port.y - 7.0,
            80.0,
            14.0,
            crate::TextHorizontalAlignment::Start,
        ),
    };
    (
        crate::ComponentTextRegion {
            bounds: LayoutBox {
                x: bounds.x + x,
                y: bounds.y + y,
                width,
                height,
            },
            content: Arc::clone(&port.label),
            color: Some(color),
            font_size: 9.5,
            font_weight: None,
        },
        align,
    )
}

fn image_viewer_geometry(
    bounds: LayoutBox,
    name: Option<&Arc<str>>,
    metadata: Option<&Arc<str>>,
    zoom: f32,
    offset_x: f32,
    offset_y: f32,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    let mut viewer = crate::ImageViewer::new(crate::ImageViewerContent::None);
    if let Some(name) = name {
        viewer = viewer.name(Arc::clone(name));
    }
    if let Some(metadata) = metadata {
        viewer = viewer.metadata(Arc::clone(metadata));
    }
    viewer.zoom = zoom;
    viewer.offset = crate::ImageViewerOffset::new(offset_x, offset_y);
    let geometry = viewer.geometry(bounds);
    let mut scrim = palette.background.as_rgba_array();
    scrim[3] = 0.94;
    let mut stage = palette.background.as_rgba_array();
    stage[3] = 0.34;
    crate::ComponentGeometry::ImageViewer {
        scrim: geometry.scrim,
        surface: geometry.surface,
        stage: geometry.stage,
        close: geometry.close,
        name: name
            .zip(geometry.name)
            .map(|(text, region)| crate::ComponentTextRegion {
                bounds: region,
                content: Arc::clone(text),
                color: Some(palette.text.as_rgba_array()),
                font_size: 12.0,
                font_weight: Some(600),
            }),
        metadata: metadata.zip(geometry.metadata).map(|(text, region)| {
            crate::ComponentTextRegion {
                bounds: region,
                content: Arc::clone(text),
                color: Some(palette.muted.as_rgba_array()),
                font_size: 11.0,
                font_weight: None,
            }
        }),
        content: geometry.content,
        scrim_color: scrim,
        surface_color: palette.surface.as_rgba_array(),
        stage_color: stage,
    }
}

fn key_capture_geometry(
    content: LayoutBox,
    recording: bool,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    let label: Arc<str> = if recording {
        Arc::from("Recording")
    } else {
        Arc::from("Idle")
    };
    crate::ComponentGeometry::KeyCaptureLayer {
        badge: key_badge_region(content, &label, !recording, palette),
        background: Some(if recording {
            palette.accent_soft.as_rgba_array()
        } else {
            palette.subtle.as_rgba_array()
        }),
    }
}

fn keymap_geometry(content: LayoutBox, palette: &SemanticPalette) -> crate::ComponentGeometry {
    crate::ComponentGeometry::KeymapLayer {
        badge: key_badge_region(content, "Keymap", false, palette),
    }
}

fn key_badge_region(
    origin: LayoutBox,
    label: &str,
    muted: bool,
    palette: &SemanticPalette,
) -> crate::ComponentTextRegion {
    const HEIGHT: f32 = 28.0;
    const PAD: f32 = 8.0;
    let font_size = 12.0;
    crate::ComponentTextRegion {
        bounds: LayoutBox {
            x: origin.x,
            y: origin.y,
            width: (estimated_text_width(label, font_size) + PAD * 2.0).max(64.0),
            height: HEIGHT.min(origin.height.max(HEIGHT)),
        },
        content: Arc::from(label),
        color: Some(if muted {
            palette.muted.as_rgba_array()
        } else {
            palette.text.as_rgba_array()
        }),
        font_size,
        font_weight: Some(600),
    }
}

fn estimated_text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|ch| {
            if ch.is_ascii() {
                font_size * 0.62
            } else {
                font_size
            }
        })
        .sum::<f32>()
        .max(font_size)
}

fn area_under_polyline(points: &[[f32; 2]], baseline: f32) -> Vec<LayoutBox> {
    const STRIP: f32 = 2.0;
    let mut strips = Vec::new();
    for pair in points.windows(2) {
        let [x0, y0] = pair[0];
        let [x1, y1] = pair[1];
        let span = x1 - x0;
        if !span.is_finite() || span.abs() < f32::EPSILON {
            continue;
        }
        let left = x0.min(x1);
        let right = x0.max(x1);
        let mut x = left;
        while x < right {
            let width = STRIP.min(right - x);
            let mid = x + width * 0.5;
            let t = (mid - x0) / span;
            let y = y0 + (y1 - y0) * t;
            let top = y.min(baseline);
            let height = (baseline - top).max(0.0);
            if height > 0.0 {
                strips.push(LayoutBox {
                    x,
                    y: top,
                    width,
                    height,
                });
            }
            x += STRIP;
        }
    }
    strips
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Easing, MeasureTextShaper};
    use nana_ui_core::{
        LayoutStyle, LengthSpec, OverflowSpec, PaintMat4, PaintTransform, PointerEventsSpec,
        SemanticColorRole,
    };

    fn node(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn document(value: u64) -> DocumentId {
        DocumentId::new(value).unwrap()
    }

    fn hit_entry_transform(world: &UiWorld, document: DocumentId, id: StableNodeId) -> [f32; 6] {
        find_hit_transform(&world.hit_test_index[&document], id).expect("hit entry")
    }

    #[test]
    fn batch_builds_reparents_and_detaches_hierarchy() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=4 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(1), node(4), Some(node(3)));
        let report = world.commit(queue).unwrap();
        assert_eq!(report.created, 4);
        assert_eq!(report.inserted, 3);
        assert_eq!(report.reparented, 0);
        assert_eq!(
            world.node(node(1)).unwrap().children,
            vec![node(2), node(4), node(3)]
        );

        let mut queue = MutationQueue::new();
        queue.insert(node(2), node(3), None);
        queue.detach(node(4));
        let report = world.commit(queue).unwrap();
        assert_eq!(report.reparented, 1);
        assert_eq!(report.detached, 1);
        assert_eq!(world.node(node(1)).unwrap().children, vec![node(2)]);
        assert_eq!(world.node(node(2)).unwrap().children, vec![node(3)]);
        assert_eq!(world.node(node(4)).unwrap().parent, None);
        assert_eq!(world.mount_state(node(4)), Some(MountState::Mounted));
        assert!(world.contains(node(4)));
        assert!(!world.document_order(document(1)).contains(&node(4)));
        assert!(world.extract_nodes(&[node(4)]).is_empty());

        let mut attach = MutationQueue::new();
        attach.insert(node(1), node(4), None);
        world.commit(attach).unwrap();
        assert_eq!(world.node(node(4)).unwrap().parent, Some(node(1)));
        assert!(world.document_order(document(1)).contains(&node(4)));
    }

    #[test]
    fn document_roots_indexes_live_parentless_nodes_without_scanning() {
        fn scanned(world: &UiWorld, document: DocumentId) -> Vec<StableNodeId> {
            let mut roots = world
                .entities
                .keys()
                .copied()
                .filter(|id| {
                    let identity = world.component::<super::Identity>(*id);
                    identity.document == document
                        && world.presence_live(*id)
                        && world.component::<super::Hierarchy>(*id).parent.is_none()
                })
                .collect::<Vec<_>>();
            roots.sort_unstable();
            roots
        }

        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "column".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        for row in 0..400u64 {
            let row_id = node(3 + row);
            queue.create(row_id, document(1), NodeKind::Element { tag: "row".into() });
            queue.insert(node(2), row_id, None);
        }
        world.commit(queue).unwrap();
        assert_eq!(world.document_roots(document(1)), vec![node(1)]);
        assert_eq!(
            world.document_roots(document(1)),
            scanned(&world, document(1))
        );

        let detached = node(13);
        let mut detach = MutationQueue::new();
        detach.detach(detached);
        world.commit(detach).unwrap();
        assert_eq!(world.node(detached).unwrap().parent, None);
        assert!(!world.presence_live(detached));
        assert_eq!(world.document_roots(document(1)), vec![node(1)]);
        assert_eq!(
            world.document_roots(document(1)),
            scanned(&world, document(1))
        );

        let mut extra = MutationQueue::new();
        extra.create(
            node(1000),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        world.commit(extra).unwrap();
        assert_eq!(world.document_roots(document(1)), vec![node(1), node(1000)]);
        assert_eq!(
            world.document_roots(document(1)),
            scanned(&world, document(1))
        );

        let mut park = MutationQueue::new();
        park.park_subtree(node(1000));
        world.commit(park).unwrap();
        assert_eq!(world.node(node(1000)).unwrap().parent, None);
        assert!(!world.presence_live(node(1000)));
        assert_eq!(world.document_roots(document(1)), vec![node(1)]);
        assert_eq!(
            world.document_roots(document(1)),
            scanned(&world, document(1))
        );

        let mut despawn = MutationQueue::new();
        despawn.despawn_subtree(node(1));
        world.commit(despawn).unwrap();
        assert!(world.document_roots(document(1)).is_empty());
        assert_eq!(
            world.document_roots(document(1)),
            scanned(&world, document(1))
        );
    }

    fn box_at(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
        LayoutBox {
            x,
            y,
            width,
            height,
        }
    }

    /// Hit entries built so far. The counter accumulates until a drain, so
    /// callers compare deltas around the build they are measuring.
    fn hit_entries_built(world: &UiWorld) -> usize {
        world
            .last_work_counters()
            .hit_test_nodes_rebuilt
            .unwrap_or_default()
    }

    /// Flatten the hit forest into a comparable projection. `HitEntry` has no
    /// `PartialEq`, and this captures everything pointer dispatch reads.
    fn hit_shape(world: &UiWorld, document: DocumentId) -> Vec<(Vec<u64>, [u32; 6], bool, i32)> {
        fn walk(
            entry: &HitEntry,
            path: &mut Vec<u64>,
            out: &mut Vec<(Vec<u64>, [u32; 6], bool, i32)>,
        ) {
            path.push(entry.id.get());
            out.push((
                path.clone(),
                entry.transform.map(f32::to_bits),
                entry.hittable,
                entry.z_index,
            ));
            for child in &entry.children {
                walk(child, path, out);
            }
            path.pop();
        }
        let mut out = Vec::new();
        for root in &world.hit_test_index[&document] {
            walk(root, &mut Vec::new(), &mut out);
        }
        out
    }

    /// Probe a coordinate grid so equivalence is asserted on what dispatch sees,
    /// not just on internal layout of the index.
    fn hit_probe_grid(world: &UiWorld, document: DocumentId) -> Vec<Vec<StableNodeId>> {
        let mut probes = Vec::new();
        for step_y in 0..12 {
            for step_x in 0..12 {
                let x = step_x as f32 * 9.0;
                let y = step_y as f32 * 9.0;
                probes.push(world.hit_test_candidates(document, x, y));
            }
        }
        probes
    }

    /// Grow a document of `rows` leaves under one column and return the world.
    fn world_with_rows(rows: u64) -> UiWorld {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "column".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        for row in 0..rows {
            let row_id = node(3 + row);
            queue.create(row_id, document(1), NodeKind::Element { tag: "row".into() });
            queue.insert(node(2), row_id, None);
        }
        world.commit(queue).unwrap();
        world
    }

    #[test]
    fn retired_ledger_stays_bounded_under_create_despawn_churn() {
        let mut world = UiWorld::new();
        let mut root = MutationQueue::new();
        root.create(node(1), document(1), NodeKind::Document);
        world.commit(root).unwrap();

        // Mirror a v-for churning its rows: allocate monotonically, despawn the
        // whole batch, repeat. Retirement stays permanent, so a stale handle
        // still cannot alias, but the ledger must not grow with the churn.
        let mut next = 2u64;
        for _ in 0..200 {
            let batch_start = next;
            let mut create = MutationQueue::new();
            for _ in 0..50 {
                let id = node(next);
                create.create(id, document(1), NodeKind::Element { tag: "row".into() });
                create.insert(node(1), id, None);
                next += 1;
            }
            world.commit(create).unwrap();
            let mut despawn = MutationQueue::new();
            for id in batch_start..next {
                despawn.despawn_subtree(node(id));
            }
            world.commit(despawn).unwrap();
            world.take_system_work();
        }

        let retired = world.retired_ids();
        assert_eq!(retired, 200 * 50);
        // Consecutive allocation coalesces into a single run regardless of how
        // many IDs churned through it.
        assert_eq!(world.retired_id_runs(), 1);

        // The contract still holds: a stale handle cannot be revived.
        let mut revive = MutationQueue::new();
        revive.create(node(2), document(1), NodeKind::Text);
        assert!(matches!(
            world.commit(revive),
            Err(UiWorldError::RetiredNode(_))
        ));
        assert!(world.is_retired(node(2)));
        assert!(world.is_retired(node(next - 1)));
        assert!(!world.is_retired(node(next)));
        assert!(!world.is_retired(node(1)));
    }

    #[test]
    fn retired_runs_coalesce_across_gaps_in_any_insert_order() {
        let mut retired = RetiredIds::default();
        for value in [10u64, 12, 11, 30, 1, 2, 29, 31] {
            retired.insert(node(value));
        }
        assert_eq!(retired.len(), 8);
        // {1,2} {10,11,12} {29,30,31}
        assert_eq!(retired.runs(), 3);
        for value in [1u64, 2, 10, 11, 12, 29, 30, 31] {
            assert!(retired.contains(node(value)), "{value} must be retired");
        }
        for value in [3u64, 9, 13, 28, 32, u64::MAX] {
            assert!(!retired.contains(node(value)), "{value} must be live");
        }
        // Re-inserting is idempotent and does not double-count.
        retired.insert(node(11));
        assert_eq!(retired.len(), 8);
        assert_eq!(retired.runs(), 3);
        // Bridging two runs merges them.
        retired.insert(node(13));
        for value in [
            14u64, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
        ] {
            retired.insert(node(value));
        }
        assert_eq!(retired.runs(), 2);
    }

    #[test]
    fn batch_validation_cost_tracks_the_batch_not_the_retained_world() {
        let small = 200u64;
        let large = 10_000u64;

        let mut small_world = world_with_rows(small);
        let mut large_world = world_with_rows(large);
        assert_eq!(small_world.len(), small as usize + 2);
        assert_eq!(large_world.len(), large as usize + 2);

        // A multi-mutation batch skips the single-mutation fast path, so this is
        // the ValidationPlan route Vue takes on every flush.
        let batch = |target: StableNodeId| {
            let mut queue = MutationQueue::new();
            queue.set_text(
                target,
                TextContent {
                    value: "updated".into(),
                },
            );
            queue.set_style(
                target,
                NodeStyle {
                    layout: std::sync::Arc::new(crate::LayoutStyle {
                        z_index: Some(3),
                        ..crate::LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
            queue.set_scroll_offset(target, ScrollOffset { x: 0.0, y: 4.0 });
            queue
        };

        small_world.commit(batch(node(3 + small - 1))).unwrap();
        large_world.commit(batch(node(3 + large - 1))).unwrap();

        // The 50x larger world must not cost 50x to validate. Before the overlay
        // rewrite both numbers tracked `len()` exactly.
        assert_eq!(validation_scanned(&mut small_world), 0);
        assert_eq!(validation_scanned(&mut large_world), 0);
    }

    /// Nodes validation visited since the previous drain, which is the window a
    /// frame consumes.
    fn validation_scanned(world: &mut UiWorld) -> usize {
        world
            .take_system_work()
            .validation_nodes_scanned
            .unwrap_or_default()
    }

    /// Build a nested grid document with clipping, transforms, z-index and a
    /// scroller so scoped patches are exercised against the tricky cases.
    fn hit_fixture(columns: u64, rows: u64) -> UiWorld {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.write_layout(node(1), box_at(0.0, 0.0, 100.0, 100.0));
        for column in 0..columns {
            let column_id = node(10 + column * 100);
            queue.create(
                column_id,
                document(1),
                NodeKind::Element {
                    tag: "column".into(),
                },
            );
            queue.insert(node(1), column_id, None);
            queue.write_layout(column_id, box_at(column as f32 * 10.0, 0.0, 10.0, 100.0));
            queue.set_style(
                column_id,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        overflow_y: OverflowSpec::Scroll,
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
            for row in 0..rows {
                let cell = node(11 + column * 100 + row);
                queue.create(cell, document(1), NodeKind::Element { tag: "cell".into() });
                queue.insert(column_id, cell, None);
                queue.write_layout(
                    cell,
                    box_at(column as f32 * 10.0, row as f32 * 8.0, 10.0, 8.0),
                );
                if row % 3 == 0 {
                    let mut raised = NodeStyle::default();
                    Arc::make_mut(&mut raised.layout).z_index = Some(2);
                    queue.set_style(cell, raised);
                }
            }
        }
        world.commit(queue).unwrap();
        world.take_system_work();
        world.rebuild_hit_test(document(1));
        world
    }

    #[test]
    fn hit_test_matches_the_first_collected_candidate() {
        let world = hit_fixture(6, 8);
        // The short-circuit walk must agree with the collecting walk everywhere,
        // including misses and clipped regions.
        for step_y in 0..30 {
            for step_x in 0..30 {
                let x = step_x as f32 * 2.5 - 5.0;
                let y = step_y as f32 * 3.5 - 5.0;
                assert_eq!(
                    world.hit_test(document(1), x, y),
                    world
                        .hit_test_candidates(document(1), x, y)
                        .into_iter()
                        .next(),
                    "hit_test disagreed at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn tree_depth_is_bounded_where_the_tree_is_written() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        for id in 2..=(MAX_TREE_DEPTH as u64 + 8) {
            create.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        world.commit(create).unwrap();

        // A chain built one level at a time is rejected at the limit, not at a
        // stack overflow inside style resolution or layout.
        let mut depth = 1u64;
        let rejected = loop {
            let parent = node(depth);
            let child = node(depth + 1);
            let mut link = MutationQueue::new();
            link.insert(parent, child, None);
            match world.commit(link) {
                Ok(_) => depth += 1,
                Err(error) => break error,
            }
            assert!(depth < MAX_TREE_DEPTH as u64 + 8, "depth was never bounded");
        };
        assert!(matches!(rejected, UiWorldError::TreeTooDeep { .. }));
        assert_eq!(depth as usize, MAX_TREE_DEPTH);

        // The rejected batch left nothing behind, and the accepted tree still
        // resolves and lays out without recursing past the bound.
        assert_eq!(
            world.node(node(MAX_TREE_DEPTH as u64 + 1)).unwrap().parent,
            None
        );
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.rebuild_hit_test(document(1));

        // Splicing an already-deep detached subtree under a deep parent is
        // rejected on the combined height, not just the parent's depth.
        let mut detached = MutationQueue::new();
        detached.insert(
            node(MAX_TREE_DEPTH as u64 + 1),
            node(MAX_TREE_DEPTH as u64 + 2),
            None,
        );
        world.commit(detached).unwrap();
        let mut splice = MutationQueue::new();
        splice.insert(
            node(MAX_TREE_DEPTH as u64),
            node(MAX_TREE_DEPTH as u64 + 1),
            None,
        );
        assert!(matches!(
            world.commit(splice),
            Err(UiWorldError::TreeTooDeep { .. })
        ));
    }

    #[test]
    fn scoped_hit_patch_matches_a_full_rebuild_for_a_leaf_layout_change() {
        let mut world = hit_fixture(6, 8);
        let target = node(11 + 3 * 100 + 5);

        let mut move_cell = MutationQueue::new();
        move_cell.write_layout(target, box_at(31.0, 41.0, 8.0, 6.0));
        world.commit(move_cell).unwrap();
        let work = world.take_system_work();
        // The frame driver patches from the INPUT set, which layout writeback
        // marks on exactly the nodes whose box moved.
        let dirty = work.input_hit_test.clone();
        assert_eq!(dirty, vec![target]);

        let before_patch = hit_entries_built(&world);
        assert!(world.rebuild_hit_test_scoped(document(1), &dirty));
        let patched_built = hit_entries_built(&world) - before_patch;
        let patched_shape = hit_shape(&world, document(1));
        let patched_probe = hit_probe_grid(&world, document(1));

        let before_full = hit_entries_built(&world);
        world.rebuild_hit_test(document(1));
        let full_built = hit_entries_built(&world) - before_full;
        assert_eq!(patched_shape, hit_shape(&world, document(1)));
        assert_eq!(patched_probe, hit_probe_grid(&world, document(1)));

        // The whole point: the patch built a subtree, not the document.
        assert_eq!(patched_built, 1);
        assert!(
            patched_built < full_built / 4,
            "scoped patch built {patched_built} entries, full rebuild built {full_built}"
        );
    }

    #[test]
    fn scoped_hit_patch_handles_visibility_insertion_and_removal() {
        let mut world = hit_fixture(4, 6);
        let column = node(10 + 100);
        let cell = node(11 + 100 + 2);

        // Hiding a cell drops it from the parent's children.
        let mut hide = MutationQueue::new();
        hide.set_style(
            cell,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    hidden: true,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(hide).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let mut dirty = work.layout.clone();
        dirty.extend(work.input_hit_test.iter().copied());
        dirty.extend(work.style.iter().copied());
        dirty.sort_unstable();
        dirty.dedup();
        assert!(world.rebuild_hit_test_scoped(document(1), &dirty));
        let patched = hit_probe_grid(&world, document(1));
        world.rebuild_hit_test(document(1));
        assert_eq!(patched, hit_probe_grid(&world, document(1)));
        assert!(
            !world
                .hit_test_candidates(document(1), 12.0, 18.0)
                .contains(&cell)
        );

        // Inserting a new child must land in Hierarchy order, not at the end.
        let inserted = node(9_001);
        let mut insert = MutationQueue::new();
        insert.create(
            inserted,
            document(1),
            NodeKind::Element { tag: "new".into() },
        );
        insert.insert(column, inserted, Some(node(11 + 100)));
        insert.write_layout(inserted, box_at(10.0, 0.0, 10.0, 8.0));
        world.commit(insert).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let mut dirty = work.layout.clone();
        dirty.extend(work.input_hit_test.iter().copied());
        dirty.sort_unstable();
        dirty.dedup();
        assert!(world.rebuild_hit_test_scoped(document(1), &dirty));
        let patched = hit_probe_grid(&world, document(1));
        world.rebuild_hit_test(document(1));
        assert_eq!(patched, hit_probe_grid(&world, document(1)));
        assert!(
            world
                .hit_test_candidates(document(1), 12.0, 3.0)
                .contains(&inserted)
        );

        // Despawning the subtree keeps the index consistent with a rebuild.
        let mut despawn = MutationQueue::new();
        despawn.despawn_subtree(inserted);
        world.commit(despawn).unwrap();
        let work = world.take_system_work();
        let mut dirty = work.layout.clone();
        dirty.extend(work.input_hit_test.iter().copied());
        dirty.sort_unstable();
        dirty.dedup();
        assert!(world.rebuild_hit_test_scoped(document(1), &dirty));
        let patched = hit_probe_grid(&world, document(1));
        world.rebuild_hit_test(document(1));
        assert_eq!(patched, hit_probe_grid(&world, document(1)));
    }

    #[test]
    fn overlay_validation_walks_hosts_not_every_entity() {
        let rows = 5_000u64;
        let mut world = world_with_rows(rows);

        // One real overlay host with a modal surface: validation may walk hosts
        // and document order, but must never enumerate the whole world per node.
        let host = node(3);
        let surface = node(3 + rows);
        let mut open = MutationQueue::new();
        open.create(
            surface,
            document(1),
            NodeKind::Element {
                tag: "dialog".into(),
            },
        );
        open.insert(host, surface, None);
        open.set_accessibility(
            surface,
            AccessibilityState {
                role: AccessibilityRole::Dialog,
                modal: true,
                ..AccessibilityState::default()
            },
        );
        open.set_overlay_host(
            host,
            OverlayHostState {
                active: Some(surface),
                restore_focus: None,
            },
        );
        world.commit(open).unwrap();
        // Close the measurement window on setup so the next reading is the
        // overlay-aware batch alone.
        validation_scanned(&mut world);

        // Host-only bookkeeping: one host, so the walk is one node wide.
        let mut touch = MutationQueue::new();
        touch.set_text(node(4), TextContent { value: "a".into() });
        touch.set_text(node(5), TextContent { value: "b".into() });
        world.commit(touch).unwrap();
        assert_eq!(validation_scanned(&mut world), 1);

        // Despawning the surface clears host references without a world scan.
        let mut close = MutationQueue::new();
        close.despawn_subtree(surface);
        close.set_text(node(4), TextContent { value: "c".into() });
        world.commit(close).unwrap();
        let scanned = validation_scanned(&mut world);
        assert!(
            scanned < rows as usize,
            "overlay teardown scanned {scanned} of {} nodes",
            world.len()
        );
        assert_eq!(world.overlay_host(host).unwrap().active, None);
    }

    #[test]
    fn parked_subtree_leaves_every_document_projection_and_remounts_intact() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        create.create(node(3), document(1), NodeKind::Text);
        create.create(node(4), document(1), NodeKind::Document);
        create.insert(node(1), node(2), None);
        create.insert(node(2), node(3), None);
        create.set_interaction(
            node(2),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        create.set_text_input(node(2), Some(TextInputState::new("value")));
        create.set_accessibility(
            node(2),
            AccessibilityState {
                role: AccessibilityRole::Dialog,
                label: Some(Arc::from("Action")),
                modal: true,
                ..AccessibilityState::default()
            },
        );
        create.set_overlay_host(
            node(1),
            OverlayHostState {
                active: Some(node(2)),
                restore_focus: Some(node(2)),
            },
        );
        create.request_focus(document(1), Some(node(2)));
        create.set_ime(
            node(2),
            Some(ImeComposition {
                text: "input".into(),
                selection: None,
            }),
        );
        create.capture_pointer(7, node(2));
        create.start_animation(AnimationSpec {
            id: crate::AnimationId::new(1).unwrap(),
            target: node(2),
            start: Duration::from_millis(10),
            duration: Duration::from_millis(100),
            frame_interval: Duration::from_millis(10),
            easing: Easing::Linear,
        });
        world.commit(create).unwrap();
        world.take_system_work();

        let mut park = MutationQueue::new();
        park.park_subtree(node(1));
        world.commit(park).unwrap();
        assert_eq!(world.mount_state(node(1)), Some(MountState::Parked));
        assert_eq!(world.mount_state(node(2)), Some(MountState::Parked));
        assert_eq!(world.document_order(document(1)), vec![node(4)]);
        assert!(world.extract_nodes(&[node(1), node(2), node(3)]).is_empty());
        assert!(
            world
                .project_accessibility_nodes(&[node(1), node(2), node(3)])
                .is_empty()
        );
        assert!(world.event_route(node(2)).is_none());
        assert_eq!(world.focused(document(1)), None);
        assert_eq!(world.ime(node(2)), None);
        assert_eq!(world.pointer_capture(document(1), 7), None);
        assert_eq!(
            world.set_pointer_hover(document(1), 8, Some(node(2))),
            Err(UiWorldError::NotPointerInteractive(node(2)))
        );
        let mut refocus = MutationQueue::new();
        refocus.request_focus(document(1), Some(node(2)));
        assert_eq!(
            world.commit(refocus),
            Err(UiWorldError::NotFocusable(node(2)))
        );
        assert_eq!(world.next_animation_deadline(), None);
        assert_eq!(
            world.overlay_host(node(1)),
            Some(OverlayHostState::default())
        );
        let work = world.take_system_work();
        assert!(work.layout.is_empty());
        assert!(work.render_extraction.is_empty());
        assert_eq!(work.render_removals, vec![node(1), node(2), node(3)]);
        assert_eq!(work.accessibility_removals, vec![node(1), node(2), node(3)]);

        let mut remount = MutationQueue::new();
        remount.insert(node(4), node(1), None);
        world.commit(remount).unwrap();
        assert!(world.is_mounted(node(1)));
        assert!(world.is_mounted(node(2)));
        assert_eq!(world.node(node(2)).unwrap().parent, Some(node(1)));
        assert!(world.document_order(document(1)).contains(&node(3)));
        let work = world.take_system_work();
        assert!(work.render_extraction.contains(&node(1)));
        assert!(work.render_extraction.contains(&node(2)));
        assert!(work.accessibility.contains(&node(2)));
    }

    #[test]
    fn only_the_top_reachable_modal_constrains_focus() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        for id in 1..=6 {
            create.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        create.insert(node(1), node(2), None);
        create.insert(node(2), node(3), None);
        create.insert(node(4), node(5), None);
        create.insert(node(5), node(6), None);
        for id in [node(3), node(6)] {
            create.set_interaction(
                id,
                InteractionState {
                    pointer_events: true,
                    focusable: true,
                },
            );
        }
        for id in [node(2), node(5)] {
            create.set_accessibility(
                id,
                AccessibilityState {
                    role: AccessibilityRole::Dialog,
                    modal: true,
                    ..AccessibilityState::default()
                },
            );
        }
        let mut lower = NodeStyle::default();
        Arc::make_mut(&mut lower.layout).z_index = Some(10);
        create.set_style(node(2), lower);
        let mut upper = NodeStyle::default();
        Arc::make_mut(&mut upper.layout).z_index = Some(20);
        create.set_style(node(5), upper);
        create.set_overlay_host(
            node(1),
            OverlayHostState {
                active: Some(node(2)),
                restore_focus: None,
            },
        );
        create.set_overlay_host(
            node(4),
            OverlayHostState {
                active: Some(node(5)),
                restore_focus: None,
            },
        );
        create.request_focus(document(1), Some(node(6)));
        world.commit(create).unwrap();
        assert_eq!(world.focused(document(1)), Some(node(6)));

        let mut lower_focus = MutationQueue::new();
        lower_focus.request_focus(document(1), Some(node(3)));
        assert_eq!(
            world.commit(lower_focus),
            Err(UiWorldError::NotFocusable(node(3)))
        );

        let mut park_upper = MutationQueue::new();
        park_upper.park_subtree(node(5));
        park_upper.request_focus(document(1), Some(node(3)));
        world.commit(park_upper).unwrap();
        assert_eq!(world.focused(document(1)), Some(node(3)));
    }

    #[test]
    fn display_none_rejects_focus_from_staged_and_committed_styles() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "input".into(),
            },
        );
        create.insert(node(1), node(2), None);
        create.set_interaction(
            node(2),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        world.commit(create).unwrap();

        let mut hidden = NodeStyle::default();
        Arc::make_mut(&mut hidden.layout).display = Some(nana_ui_core::DisplaySpec::None);
        let mut hide_and_focus = MutationQueue::new();
        hide_and_focus.set_style(node(2), hidden.clone());
        hide_and_focus.request_focus(document(1), Some(node(2)));
        assert_eq!(
            world.commit(hide_and_focus),
            Err(UiWorldError::NotFocusable(node(2)))
        );
        assert_eq!(world.focused(document(1)), None);
        assert!(!matches!(
            world.node_style(node(2)).unwrap().layout.display,
            Some(nana_ui_core::DisplaySpec::None)
        ));

        let mut hide = MutationQueue::new();
        hide.set_style(node(2), hidden);
        world.commit(hide).unwrap();
        let mut focus = MutationQueue::new();
        focus.request_focus(document(1), Some(node(2)));
        assert_eq!(
            world.commit(focus),
            Err(UiWorldError::NotFocusable(node(2)))
        );
    }

    #[test]
    fn display_none_is_omitted_from_document_extraction() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "panel".into(),
            },
        );
        create.insert(node(1), node(2), None);
        world.commit(create).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert!(
            world
                .extract_document(document(1))
                .iter()
                .any(|extracted| extracted.id == node(2))
        );

        let mut hidden = NodeStyle::default();
        Arc::make_mut(&mut hidden.layout).display = Some(nana_ui_core::DisplaySpec::None);
        let mut hide = MutationQueue::new();
        hide.set_style(node(2), hidden);
        world.commit(hide).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert!(
            world
                .extract_document(document(1))
                .iter()
                .all(|extracted| extracted.id != node(2))
        );
        let incremental = world.extract_nodes(&[node(2)]);
        assert_eq!(incremental.len(), 1);
        assert!(!incremental[0].style.visible);
    }

    #[test]
    fn visibility_hidden_skips_extract_paint_and_hit_test() {
        use nana_ui_core::VisibilitySpec;

        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "panel".into(),
            },
        );
        create.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "child".into(),
            },
        );
        create.insert(node(1), node(2), None);
        create.insert(node(2), node(3), None);
        create.write_layout(node(2), box_at(10.0, 10.0, 50.0, 50.0));
        create.write_layout(node(3), box_at(5.0, 5.0, 30.0, 30.0));
        world.commit(create).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.rebuild_hit_test(document(1));
        assert!(
            world
                .extract_document(document(1))
                .iter()
                .any(|extracted| extracted.id == node(2))
        );
        assert!(
            world
                .hit_test_candidates(document(1), 20.0, 20.0)
                .contains(&node(2))
        );

        let mut hidden = NodeStyle::default();
        Arc::make_mut(&mut hidden.layout).paint.visibility = Some(VisibilitySpec::Hidden);
        let mut hide = MutationQueue::new();
        hide.set_style(node(2), hidden);
        world.commit(hide).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let mut dirty = work.input_hit_test.clone();
        dirty.extend(work.style.iter().copied());
        dirty.sort_unstable();
        dirty.dedup();
        assert!(world.rebuild_hit_test_scoped(document(1), &dirty));

        assert!(
            world
                .extract_document(document(1))
                .iter()
                .all(|extracted| extracted.id != node(2) && extracted.id != node(3))
        );
        let extracted = world.extract_nodes(&[node(2), node(3)]);
        assert_eq!(extracted.len(), 2);
        assert!(extracted.iter().all(|node| !node.style.visible));
        assert!(
            !world
                .hit_test_candidates(document(1), 20.0, 20.0)
                .contains(&node(2))
        );
        assert!(
            !world
                .hit_test_candidates(document(1), 15.0, 15.0)
                .contains(&node(3))
        );
    }

    #[test]
    fn pointer_events_none_inherits_unless_child_is_explicit_auto() {
        use nana_ui_core::PointerEventsSpec;

        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "under".into(),
            },
        );
        create.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "overlay".into(),
            },
        );
        create.create(
            node(4),
            document(1),
            NodeKind::Element {
                tag: "inherited".into(),
            },
        );
        create.create(
            node(5),
            document(1),
            NodeKind::Element {
                tag: "explicit-auto".into(),
            },
        );
        create.insert(node(1), node(2), None);
        create.insert(node(1), node(3), None);
        create.insert(node(3), node(4), None);
        create.insert(node(3), node(5), None);
        create.write_layout(node(2), box_at(0.0, 0.0, 80.0, 80.0));
        create.write_layout(node(3), box_at(0.0, 0.0, 80.0, 80.0));
        create.write_layout(node(4), box_at(10.0, 10.0, 20.0, 20.0));
        create.write_layout(node(5), box_at(40.0, 10.0, 20.0, 20.0));
        let mut auto = NodeStyle::default();
        Arc::make_mut(&mut auto.layout).pointer_events = Some(PointerEventsSpec::Auto);
        create.set_style(node(5), auto);
        world.commit(create).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.rebuild_hit_test(document(1));
        assert_eq!(world.hit_test(document(1), 50.0, 20.0), Some(node(5)));

        let mut none = NodeStyle::default();
        Arc::make_mut(&mut none.layout).pointer_events = Some(PointerEventsSpec::None);
        let mut skip = MutationQueue::new();
        skip.set_style(node(3), none);
        world.commit(skip).unwrap();
        let work = world.take_system_work();
        assert!(
            work.layout.is_empty(),
            "pointer-events is not a layout dirty"
        );
        assert!(work.input_hit_test.contains(&node(3)));
        assert!(work.input_hit_test.contains(&node(4)));
        world.resolve_styles(&work.style).unwrap();
        assert!(world.rebuild_hit_test_scoped(document(1), &work.input_hit_test));

        assert_ne!(world.hit_test(document(1), 50.0, 50.0), Some(node(3)));
        assert_eq!(
            world.hit_test(document(1), 15.0, 15.0),
            Some(node(2)),
            "unspecified child inherits none"
        );
        assert_eq!(
            world.hit_test(document(1), 50.0, 20.0),
            Some(node(5)),
            "explicit auto child is a target again"
        );
        assert_eq!(
            world.hit_test(document(1), 20.0, 50.0),
            Some(node(2)),
            "parent padding that is not on the auto child passes through"
        );
    }

    #[test]
    fn pointer_events_none_clears_hover_without_layout() {
        use nana_ui_core::PointerEventsSpec;

        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "panel".into(),
            },
        );
        create.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "child".into(),
            },
        );
        create.insert(node(1), node(2), None);
        create.insert(node(2), node(3), None);
        create.write_layout(node(2), box_at(0.0, 0.0, 40.0, 40.0));
        create.write_layout(node(3), box_at(4.0, 4.0, 16.0, 16.0));
        world.commit(create).unwrap();
        world.take_system_work();
        world
            .set_pointer_hover(document(1), 7, Some(node(3)))
            .unwrap();
        world.take_system_work();
        assert_eq!(world.pointer_hover(document(1), 7), Some(node(3)));

        let mut none = NodeStyle::default();
        Arc::make_mut(&mut none.layout).pointer_events = Some(PointerEventsSpec::None);
        let mut skip = MutationQueue::new();
        skip.set_style(node(2), none);
        world.commit(skip).unwrap();
        assert_eq!(world.pointer_hover(document(1), 7), None);
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(3))),
            Err(UiWorldError::NotPointerInteractive(node(3)))
        );
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(2))),
            Err(UiWorldError::NotPointerInteractive(node(2)))
        );
    }

    #[test]
    fn overflow_auto_clips_descendant_hit_testing() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "port".into() },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element { tag: "item".into() },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.write_layout(node(2), box_at(0.0, 0.0, 100.0, 50.0));
        queue.write_layout(node(3), box_at(0.0, 80.0, 100.0, 20.0));
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_y: OverflowSpec::Auto,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        world.take_system_work();
        world.rebuild_hit_test(document(1));
        assert_ne!(world.hit_test(document(1), 10.0, 85.0), Some(node(3)));
        assert_eq!(world.hit_test(document(1), 10.0, 25.0), Some(node(2)));

        let mut scroll = MutationQueue::new();
        scroll.set_scroll_offset(node(2), ScrollOffset { x: 0.0, y: 80.0 });
        world.commit(scroll).unwrap();
        world.take_system_work();
        world.rebuild_hit_test(document(1));
        assert_eq!(world.hit_test(document(1), 10.0, 5.0), Some(node(3)));
        assert_eq!(world.layout_box(node(3)).unwrap().y, 80.0);
    }

    #[test]
    fn overflow_x_hidden_does_not_clip_visible_y() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "port".into() },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element { tag: "item".into() },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.write_layout(node(2), box_at(0.0, 0.0, 100.0, 50.0));
        queue.write_layout(node(3), box_at(0.0, 80.0, 40.0, 20.0));
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_x: OverflowSpec::Hidden,
                    overflow_y: OverflowSpec::Visible,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        world.take_system_work();
        world.rebuild_hit_test(document(1));
        assert_eq!(
            world.hit_test(document(1), 10.0, 85.0),
            Some(node(3)),
            "visible Y must not clip a descendant below the padding box"
        );

        let mut moved = MutationQueue::new();
        moved.write_layout(node(3), box_at(120.0, 10.0, 40.0, 20.0));
        world.commit(moved).unwrap();
        world.take_system_work();
        world.rebuild_hit_test(document(1));
        assert_ne!(
            world.hit_test(document(1), 130.0, 15.0),
            Some(node(3)),
            "hidden X must still clip a descendant to the right"
        );
    }

    #[test]
    fn visibility_visible_child_unhides_inside_hidden_parent() {
        use nana_ui_core::VisibilitySpec;

        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "panel".into(),
            },
        );
        create.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "child".into(),
            },
        );
        create.insert(node(1), node(2), None);
        create.insert(node(2), node(3), None);
        create.write_layout(node(2), box_at(10.0, 10.0, 50.0, 50.0));
        create.write_layout(node(3), box_at(5.0, 5.0, 30.0, 30.0));
        let mut parent = NodeStyle::default();
        Arc::make_mut(&mut parent.layout).paint.visibility = Some(VisibilitySpec::Hidden);
        create.set_style(node(2), parent);
        let mut child = NodeStyle::default();
        Arc::make_mut(&mut child.layout).paint.visibility = Some(VisibilitySpec::Visible);
        create.set_style(node(3), child);
        world.commit(create).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.rebuild_hit_test(document(1));

        let parent_extracted = world.extract_nodes(&[node(2)]);
        assert_eq!(parent_extracted.len(), 1);
        assert!(!parent_extracted[0].style.visible);

        let child_extracted = world.extract_nodes(&[node(3)]);
        assert_eq!(child_extracted.len(), 1);
        assert!(child_extracted[0].style.visible);
        assert!(
            world
                .extract_document(document(1))
                .iter()
                .any(|extracted| extracted.id == node(3))
        );
        assert!(
            world
                .hit_test_candidates(document(1), 15.0, 15.0)
                .contains(&node(3))
        );
        assert!(
            !world
                .hit_test_candidates(document(1), 20.0, 20.0)
                .contains(&node(2))
        );
    }

    #[test]
    fn overlay_host_rejects_an_active_child_without_overlay_semantics() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "custom".into(),
            },
        );
        create.insert(node(1), node(2), None);
        create.set_overlay_host(
            node(1),
            OverlayHostState {
                active: Some(node(2)),
                restore_focus: None,
            },
        );

        assert_eq!(
            world.commit(create),
            Err(UiWorldError::InvalidOverlayHost(node(1)))
        );
        assert!(world.is_empty());
    }

    #[test]
    fn overlay_host_rejects_a_non_modal_dialog_surface() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "dialog".into(),
            },
        );
        create.insert(node(1), node(2), None);
        create.set_accessibility(
            node(2),
            AccessibilityState {
                role: AccessibilityRole::Dialog,
                modal: false,
                ..AccessibilityState::default()
            },
        );
        create.set_overlay_host(
            node(1),
            OverlayHostState {
                active: Some(node(2)),
                restore_focus: None,
            },
        );

        assert_eq!(
            world.commit(create),
            Err(UiWorldError::InvalidOverlayHost(node(1)))
        );
        assert!(world.is_empty());
    }

    #[test]
    fn inactive_nested_and_removed_modal_hosts_do_not_block_focus() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        for id in 10..=15 {
            create.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        create.insert(node(10), node(11), None);
        create.insert(node(10), node(12), None);
        create.insert(node(11), node(15), None);
        create.insert(node(12), node(13), None);
        create.insert(node(13), node(14), None);
        create.set_interaction(
            node(15),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        create.set_accessibility(
            node(14),
            AccessibilityState {
                role: AccessibilityRole::Dialog,
                modal: true,
                ..AccessibilityState::default()
            },
        );
        create.set_accessibility(
            node(11),
            AccessibilityState {
                role: AccessibilityRole::Menu,
                ..AccessibilityState::default()
            },
        );
        create.set_overlay_host(
            node(10),
            OverlayHostState {
                active: Some(node(11)),
                restore_focus: None,
            },
        );
        create.set_overlay_host(
            node(13),
            OverlayHostState {
                active: Some(node(14)),
                restore_focus: None,
            },
        );
        create.request_focus(document(1), Some(node(15)));
        world.commit(create).unwrap();
        assert_eq!(world.focused(document(1)), Some(node(15)));

        let mut remove = MutationQueue::new();
        remove.despawn_subtree(node(13));
        world.commit(remove).unwrap();
        assert_eq!(world.overlay_host(node(10)).unwrap().active, Some(node(11)));
        assert!(world.is_overlay_reachable(node(15)));
        assert!(!world.contains(node(13)));
        assert!(!world.contains(node(14)));
        let mut refocus = MutationQueue::new();
        refocus.request_focus(document(1), Some(node(15)));
        world.commit(refocus).unwrap();
        assert_eq!(world.focused(document(1)), Some(node(15)));
    }

    #[test]
    fn planned_park_rejects_new_ime_and_remount_does_not_restore_preedit() {
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(node(1), document(1), NodeKind::Document);
        create.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "input".into(),
            },
        );
        create.create(node(3), document(1), NodeKind::Document);
        create.insert(node(1), node(2), None);
        create.set_interaction(
            node(2),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        create.set_text_input(node(2), Some(TextInputState::new("value")));
        create.request_focus(document(1), Some(node(2)));
        world.commit(create).unwrap();

        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.park_subtree(node(1));
        invalid.set_ime(
            node(2),
            Some(ImeComposition {
                text: "preedit".into(),
                selection: None,
            }),
        );
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::NotFocused(node(2)))
        );
        assert_eq!(world.generation(), generation);
        assert!(world.is_mounted(node(2)));
        assert_eq!(world.focused(document(1)), Some(node(2)));
        assert_eq!(world.ime(node(2)), None);

        let mut park = MutationQueue::new();
        park.park_subtree(node(1));
        world.commit(park).unwrap();
        assert_eq!(world.focused(document(1)), None);
        assert_eq!(world.ime(node(2)), None);

        let mut remount = MutationQueue::new();
        remount.insert(node(3), node(1), None);
        world.commit(remount).unwrap();
        assert!(world.is_mounted(node(2)));
        assert_eq!(world.focused(document(1)), None);
        assert_eq!(world.ime(node(2)), None);
    }

    #[test]
    fn invalid_batch_is_atomic() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        world.commit(queue).unwrap();

        let mut queue = MutationQueue::new();
        queue.create(node(2), document(1), NodeKind::Text);
        queue.insert(node(99), node(2), None);
        assert_eq!(
            world.commit(queue),
            Err(UiWorldError::MissingNode(node(99)))
        );
        assert!(!world.contains(node(2)));
        assert_eq!(world.len(), 1);

        let mut foreign = MutationQueue::new();
        foreign.create(node(2), document(2), NodeKind::Text);
        world.commit(foreign).unwrap();
        let generation = world.generation();
        let mut invalid_park = MutationQueue::new();
        invalid_park.park_subtree(node(1));
        invalid_park.insert(node(1), node(2), None);
        assert!(matches!(
            world.commit(invalid_park),
            Err(UiWorldError::CrossDocument { .. })
        ));
        assert_eq!(world.generation(), generation);
        assert_eq!(world.mount_state(node(1)), Some(MountState::Mounted));
        assert!(world.document_order(document(1)).contains(&node(1)));
    }

    #[test]
    fn committed_text_selection_is_unicode_safe_and_batch_atomic() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "input".into(),
            },
        );
        queue.set_text_input(node(1), Some(TextInputState::new("你好ab")));
        queue.set_text_selection(
            node(1),
            crate::TextSelection {
                anchor: 0,
                focus: "你".len(),
            },
        );
        queue.replace_text_selection(node(1), "娜");
        world.commit(queue).unwrap();
        let state = world.text_input(node(1)).unwrap();
        assert_eq!(state.value, "娜好ab");
        assert_eq!(state.selection, crate::TextSelection::caret("娜".len()));

        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.set_text_selection(
            node(1),
            crate::TextSelection {
                anchor: 1,
                focus: 1,
            },
        );
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::InvalidTextInput(node(1)))
        );
        assert_eq!(world.generation(), generation);
        assert_eq!(world.text_input(node(1)).unwrap().value, "娜好ab");
        assert_eq!(world.text(node(1)), Some("娜好ab"));

        let accessibility = world.project_accessibility(document(1));
        assert_eq!(accessibility[0].value.as_deref(), Some("娜好ab"));
        assert_eq!(
            world.extract_document(document(1))[0]
                .text_input
                .as_ref()
                .unwrap()
                .selection,
            crate::TextSelection::caret("娜".len())
        );
    }

    #[test]
    fn text_selection_and_ime_reject_partial_graphemes_atomically() {
        let value = "A👩‍💻e\u{301}";
        let emoji_interior = "A👩".len();
        let combining_interior = "A👩‍💻e".len();
        assert!(value.is_char_boundary(emoji_interior));
        assert!(value.is_char_boundary(combining_interior));
        assert!(!crate::TextSelection::caret(emoji_interior).is_valid_for(value));
        assert!(!crate::TextSelection::caret(combining_interior).is_valid_for(value));
        assert!(crate::TextSelection::caret("A👩‍💻".len()).is_valid_for(value));

        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "textarea".into(),
            },
        );
        create.set_text_input(node(1), Some(TextInputState::new(value)));
        create.set_interaction(
            node(1),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        world.commit(create).unwrap();

        let generation = world.generation();
        let mut invalid_selection = MutationQueue::new();
        invalid_selection.set_text_selection(node(1), crate::TextSelection::caret(emoji_interior));
        assert_eq!(
            world.commit(invalid_selection),
            Err(UiWorldError::InvalidTextInput(node(1)))
        );
        assert_eq!(world.generation(), generation);

        let mut focus = MutationQueue::new();
        focus.request_focus(document(1), Some(node(1)));
        world.commit(focus).unwrap();
        let generation = world.generation();
        let mut invalid_ime = MutationQueue::new();
        invalid_ime.set_ime(
            node(1),
            Some(ImeComposition {
                text: "e\u{301}".into(),
                selection: Some(("e".len(), "e".len())),
            }),
        );
        assert_eq!(
            world.commit(invalid_ime),
            Err(UiWorldError::InvalidIme(node(1)))
        );
        assert_eq!(world.generation(), generation);
        assert!(world.ime(node(1)).is_none());
    }

    #[test]
    fn text_input_presentation_masks_graphemes_and_replaces_selection_with_preedit() {
        let value = "A👩‍💻界";
        let state = TextInputState {
            value: value.into(),
            selection: crate::TextSelection {
                anchor: "A".len(),
                focus: "A👩‍💻".len(),
            },
        };
        let masked = build_text_input_presentation_source(&state, None, "", true, false);
        assert_eq!(masked.text.value, "•••");
        assert_eq!(masked.selection, Some(("•".len(), "••".len())));

        let preedit = build_text_input_presentation_source(
            &state,
            Some(&ImeComposition {
                text: "输入".into(),
                selection: Some((0, "输".len())),
            }),
            "",
            true,
            false,
        );
        assert_eq!(preedit.text.value, "•输入•");
        assert_eq!(preedit.preedit, Some(("•".len(), "•输入".len())));
        assert_eq!(preedit.caret, "•输".len());
    }

    #[test]
    fn multiline_text_presentation_tracks_utf8_lines_selection_and_preedit() {
        let value = "甲乙\nthird\n末";
        let state = TextInputState {
            value: value.into(),
            selection: crate::TextSelection {
                anchor: "甲".len(),
                focus: "甲乙\nthird\n".len(),
            },
        };
        let style = ComputedStyle {
            font_size: 10.0,
            line_height: Some(nana_ui_core::LineHeightSpec::Absolute(14.0)),
            ..ComputedStyle::default()
        };
        let source = build_text_input_presentation_source(&state, None, "", false, true);
        let mut shaper = FunctionalShaper::default();
        let presentation = shape_text_input_presentation(
            node(1),
            source,
            &style,
            crate::TextShapeConstraints::default(),
            &mut shaper,
        );

        assert_eq!(presentation.caret_x, 0.0);
        assert_eq!(presentation.caret_y, 28.0);
        assert_eq!(presentation.line_height, 14.0);
        assert_eq!(presentation.selection_lines.len(), 2);
        assert_eq!(
            presentation.selection_lines[0],
            LayoutBox {
                x: 10.0,
                y: 0.0,
                width: 10.0,
                height: 14.0,
            }
        );
        assert_eq!(
            presentation.selection_lines[1],
            LayoutBox {
                x: 0.0,
                y: 14.0,
                width: 50.0,
                height: 14.0,
            }
        );

        let composing = TextInputState {
            value: "甲\n末".into(),
            selection: crate::TextSelection::caret("甲\n".len()),
        };
        let source = build_text_input_presentation_source(
            &composing,
            Some(&ImeComposition {
                text: "输\n入".into(),
                selection: None,
            }),
            "",
            false,
            true,
        );
        let presentation = shape_text_input_presentation(
            node(1),
            source,
            &style,
            crate::TextShapeConstraints::default(),
            &mut shaper,
        );
        assert_eq!(presentation.display_value, "甲\n输\n入末");
        assert_eq!(presentation.preedit_lines.len(), 2);
        assert_eq!(presentation.caret_y, 28.0);
    }

    #[test]
    fn text_input_geometry_keeps_all_multiline_decorations_and_single_line_contract() {
        let presentation = TextInputPresentation {
            selection: Some((2.0, 9.0)),
            selection_lines: vec![
                LayoutBox {
                    x: 2.0,
                    y: 0.0,
                    width: 18.0,
                    height: 14.0,
                },
                LayoutBox {
                    x: 0.0,
                    y: 14.0,
                    width: 24.0,
                    height: 14.0,
                },
            ],
            preedit: Some((4.0, 11.0)),
            preedit_lines: vec![
                LayoutBox {
                    x: 4.0,
                    y: 14.0,
                    width: 16.0,
                    height: 14.0,
                },
                LayoutBox {
                    x: 0.0,
                    y: 28.0,
                    width: 8.0,
                    height: 14.0,
                },
            ],
            ..TextInputPresentation::default()
        };
        let content = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 42.0,
        };

        let (selection, preedit) =
            text_input_decorations(&presentation, true, content, 15.0, 14.0, 3.0, 5.0);
        assert_eq!(selection.len(), 2);
        assert_eq!(selection[0].x, 9.0);
        assert_eq!(selection[1].y, 29.0);
        assert_eq!(preedit.len(), 2);
        assert_eq!(preedit[0].y, 41.0);
        assert_eq!(preedit[1].y, 55.0);

        let (selection, preedit) =
            text_input_decorations(&presentation, false, content, 23.0, 14.0, 3.0, 5.0);
        assert_eq!(
            selection,
            vec![LayoutBox {
                x: 9.0,
                y: 23.0,
                width: 7.0,
                height: 14.0,
            }]
        );
        assert_eq!(
            preedit,
            vec![LayoutBox {
                x: 11.0,
                y: 35.0,
                width: 7.0,
                height: 2.0,
            }]
        );
    }

    #[test]
    fn presentation_shaping_uses_resolved_wrap_only_for_multiline_editors() {
        #[derive(Default)]
        struct ConstraintProbe {
            positions: Vec<crate::TextShapeConstraints>,
        }

        impl TextShaper for ConstraintProbe {
            fn shape(
                &mut self,
                _id: StableNodeId,
                _text: &TextContent,
                _style: &ComputedStyle,
                _constraints: crate::TextShapeConstraints,
            ) -> TextMetrics {
                TextMetrics::default()
            }

            fn text_position(
                &mut self,
                _id: StableNodeId,
                _text: &TextContent,
                _byte_offset: usize,
                _style: &ComputedStyle,
                constraints: crate::TextShapeConstraints,
            ) -> (f32, f32, f32) {
                self.positions.push(constraints);
                (0.0, 0.0, 14.0)
            }
        }

        let state = TextInputState {
            value: "wrapped value".into(),
            selection: crate::TextSelection::caret("wrapped value".len()),
        };
        let resolved = crate::TextShapeConstraints {
            max_width: Some(48.0),
            max_height: Some(20.0),
            wrap: true,
            ellipsis: true,
            max_lines: None,
            shaping: crate::TextShaping::Advanced,
            preserve_lines: false,
            wrap_break: nana_ui_core::TextWrapBreak::Word,
        };
        let style = ComputedStyle::default();
        let mut probe = ConstraintProbe::default();

        let multiline = build_text_input_presentation_source(&state, None, "", false, true);
        shape_text_input_presentation(node(1), multiline, &style, resolved, &mut probe);
        assert_eq!(
            probe.positions.pop(),
            Some(crate::TextShapeConstraints {
                max_width: Some(48.0),
                max_height: None,
                wrap: true,
                ellipsis: false,
                max_lines: None,
                shaping: crate::TextShaping::Advanced,
                preserve_lines: false,
                wrap_break: nana_ui_core::TextWrapBreak::Word,
            })
        );

        let single_line = build_text_input_presentation_source(&state, None, "", false, false);
        shape_text_input_presentation(node(1), single_line, &style, resolved, &mut probe);
        assert_eq!(
            probe.positions.pop(),
            Some(crate::TextShapeConstraints {
                max_width: None,
                max_height: None,
                wrap: false,
                ellipsis: false,
                max_lines: None,
                shaping: crate::TextShaping::Advanced,
                preserve_lines: false,
                wrap_break: nana_ui_core::TextWrapBreak::Word,
            })
        );
    }

    #[test]
    fn animations_are_atomic_deadline_driven_and_replaceable() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Text);
        world.commit(queue).unwrap();

        let animation_id = AnimationId::new(1).unwrap();
        let missing_id = AnimationId::new(2).unwrap();
        let animation = AnimationSpec {
            id: animation_id,
            target: node(1),
            start: Duration::from_millis(100),
            duration: Duration::from_millis(100),
            frame_interval: Duration::from_millis(16),
            easing: Easing::EaseOutCubic,
        };
        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.start_animation(animation);
        invalid.stop_animation(missing_id);
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::MissingAnimation(missing_id))
        );
        assert_eq!(world.generation(), generation);
        assert_eq!(world.next_animation_deadline(), None);

        let mut invalid_timing = MutationQueue::new();
        invalid_timing.start_animation(AnimationSpec {
            duration: Duration::ZERO,
            ..animation
        });
        assert_eq!(
            world.commit(invalid_timing),
            Err(UiWorldError::InvalidAnimation(animation_id))
        );
        assert_eq!(world.generation(), generation);

        let mut start = MutationQueue::new();
        start.start_animation(animation);
        world.commit(start).unwrap();
        assert_eq!(
            world.next_animation_deadline(),
            Some(Duration::from_millis(100))
        );
        assert!(
            world
                .advance_animations(Duration::from_millis(99))
                .samples
                .is_empty()
        );
        let first = world.advance_animations(Duration::from_millis(100));
        assert_eq!(first.samples.len(), 1);
        assert_eq!(first.samples[0].progress, 0.0);
        assert_eq!(first.next_deadline, Some(Duration::from_millis(116)));

        let replacement = AnimationSpec {
            target: node(2),
            start: Duration::from_millis(150),
            easing: Easing::Linear,
            ..animation
        };
        let mut replace = MutationQueue::new();
        replace.start_animation(replacement);
        world.commit(replace).unwrap();
        let middle = world.advance_animations(Duration::from_millis(200));
        assert_eq!(middle.samples.len(), 1);
        assert_eq!(middle.samples[0].target, node(2));
        assert_eq!(middle.samples[0].progress, 0.5);
        assert!(!middle.samples[0].finished);

        let end = world.advance_animations(Duration::from_millis(250));
        assert_eq!(end.samples.len(), 1);
        assert_eq!(end.samples[0].progress, 1.0);
        assert!(end.samples[0].finished);
        assert_eq!(end.next_deadline, None);

        let mut start_then_stop = MutationQueue::new();
        start_then_stop.start_animation(animation);
        start_then_stop.stop_animation(animation_id);
        world.commit(start_then_stop).unwrap();
        assert_eq!(world.next_animation_deadline(), None);
    }

    #[test]
    fn despawning_animation_target_cancels_its_wakeup() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.start_animation(AnimationSpec {
            id: AnimationId::new(1).unwrap(),
            target: node(2),
            start: Duration::from_millis(10),
            duration: Duration::from_secs(1),
            frame_interval: Duration::from_millis(16),
            easing: Easing::Linear,
        });
        world.commit(queue).unwrap();
        assert_eq!(
            world.next_animation_deadline(),
            Some(Duration::from_millis(10))
        );

        let mut remove = MutationQueue::new();
        remove.despawn_subtree(node(1));
        world.commit(remove).unwrap();
        assert_eq!(world.next_animation_deadline(), None);
        assert!(world.advance_animations(Duration::MAX).samples.is_empty());
    }

    #[test]
    fn advance_animations_counts_due_scheduler_lookups_not_the_idle_set() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for index in 1..=64 {
            queue.create(node(index), document(1), NodeKind::Text);
        }
        for index in 1..=64 {
            let due = index == 1;
            queue.start_animation(AnimationSpec {
                id: AnimationId::new(index).unwrap(),
                target: node(index),
                start: if due {
                    Duration::ZERO
                } else {
                    Duration::from_secs(60)
                },
                duration: Duration::from_millis(1),
                frame_interval: Duration::from_millis(16),
                easing: Easing::Linear,
            });
        }
        world.commit(queue).unwrap();

        let sparse = world.advance_animations(Duration::from_millis(1));
        assert_eq!(sparse.samples.len(), 1);
        assert_eq!(sparse.animation_deadlines_scanned, 1);
        assert_eq!(sparse.animations_considered, 1);

        let mut all_due = MutationQueue::new();
        for index in 2..=64 {
            all_due.start_animation(AnimationSpec {
                id: AnimationId::new(index).unwrap(),
                target: node(index),
                start: Duration::ZERO,
                duration: Duration::from_millis(1),
                frame_interval: Duration::from_millis(16),
                easing: Easing::Linear,
            });
        }
        world.commit(all_due).unwrap();
        let full = world.advance_animations(Duration::from_millis(1));
        assert_eq!(full.samples.len(), 63);
        assert_eq!(full.animation_deadlines_scanned, 63);
        assert_eq!(full.animations_considered, 63);
    }

    #[test]
    fn subtree_despawn_retires_ids_and_stale_handles_do_not_alias() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.despawn_subtree(node(2));
        let report = world.commit(queue).unwrap();
        assert_eq!(report.despawned, 2);
        assert_eq!(
            world.node(node(1)).unwrap().children,
            Vec::<StableNodeId>::new()
        );
        assert!(!world.contains(node(2)));
        assert!(world.is_retired(node(2)));
        let work = world.take_system_work();
        assert_eq!(work.render_removals, vec![node(2), node(3)]);
        assert_eq!(work.accessibility_removals, vec![node(2), node(3)]);
        let accessibility = world.project_accessibility_delta(&work);
        assert_eq!(accessibility.generation, work.generation);
        assert_eq!(accessibility.removed, vec![node(2), node(3)]);
        assert_eq!(
            accessibility
                .updated
                .iter()
                .map(|node| node.id)
                .collect::<Vec<_>>(),
            vec![node(1)]
        );
        assert!(accessibility.updated[0].children.is_empty());

        let mut queue = MutationQueue::new();
        queue.create(node(2), document(1), NodeKind::Text);
        assert_eq!(world.commit(queue), Err(UiWorldError::RetiredNode(node(2))));
    }

    #[test]
    fn batch_cannot_recreate_an_id_after_despawn() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        world.commit(queue).unwrap();

        let mut queue = MutationQueue::new();
        queue.despawn_subtree(node(1));
        queue.create(node(1), document(1), NodeKind::Document);
        assert_eq!(world.commit(queue), Err(UiWorldError::RetiredNode(node(1))));
        assert!(world.contains(node(1)));
        assert!(!world.is_retired(node(1)));
    }

    #[test]
    fn dirty_work_is_incremental_and_static_world_stays_idle() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=4 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(3), node(4), None);
        let report = world.commit(queue).unwrap();
        assert_eq!(report.generation, 1);

        let initial = world.take_system_work();
        assert_eq!(initial.generation, 1);
        assert_eq!(initial.style, vec![node(1), node(2), node(3), node(4)]);
        assert_eq!(initial.render_extraction, initial.style);
        assert!(world.take_system_work().is_empty());

        let empty = world.commit(MutationQueue::new()).unwrap();
        assert_eq!(empty.generation, 1);
        assert!(world.take_system_work().is_empty());

        let mut queue = MutationQueue::new();
        queue.insert(node(2), node(3), None);
        let report = world.commit(queue).unwrap();
        assert_eq!(report.generation, 2);
        let work = world.take_system_work();
        assert_eq!(work.style, vec![node(3), node(4)]);
        assert_eq!(work.text, vec![node(3), node(4)]);
        assert_eq!(work.focus_ime, vec![node(3), node(4)]);
        assert_eq!(work.layout, vec![node(1), node(2), node(3), node(4)]);
        assert_eq!(work.render_extraction, work.layout);
        assert!(world.take_system_work().is_empty());
    }

    #[test]
    fn set_component_type_is_noop_when_unchanged() {
        use bevy_ecs::change_detection::DetectChanges;

        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();

        let button = ComponentTypeId::new("nana.button").unwrap();
        let mut queue = MutationQueue::new();
        queue.set_component_type(node(1), Some(button.clone()));
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        let entity = world.entities[&node(1)];
        let tick_before = world
            .world
            .entity(entity)
            .get_ref::<ComponentTypeId>()
            .expect("stamped type")
            .last_changed();
        world.world.increment_change_tick();

        let mut queue = MutationQueue::new();
        queue.set_component_type(node(1), Some(button));
        world.commit(queue).unwrap();
        assert!(world.take_system_work().is_empty());
        let tick_after = world
            .world
            .entity(entity)
            .get_ref::<ComponentTypeId>()
            .expect("stamped type")
            .last_changed();
        assert_eq!(tick_before, tick_after);

        let mut queue = MutationQueue::new();
        queue.set_component_type(node(1), Some(ComponentTypeId::new("nana.select").unwrap()));
        world.commit(queue).unwrap();
        assert_eq!(
            world.component_type(node(1)).map(ComponentTypeId::as_str),
            Some("nana.select")
        );

        let mut queue = MutationQueue::new();
        queue.set_component_type(node(1), None);
        world.commit(queue).unwrap();
        let mut queue = MutationQueue::new();
        queue.set_component_type(node(1), None);
        world.commit(queue).unwrap();
        assert!(world.component_type(node(1)).is_none());
        assert!(world.take_system_work().is_empty());
    }

    #[test]
    fn scheduled_ui_frames_are_zero_after_static_settle_and_nonzero_when_paint_stays_dirty() {
        const TICKS: usize = 8;
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=4 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
            if id > 1 {
                queue.insert(node(id / 2), node(id), None);
            }
        }
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        assert_eq!(world.scheduled_ui_frames(TICKS), 0);

        let mut paint = MutationQueue::new();
        paint.set_style(
            node(4),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    background: Some([0.2, 0.4, 0.8, 1.0]),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(paint).unwrap();
        assert_ne!(world.scheduled_ui_frames(TICKS), 0);
        assert_eq!(world.scheduled_ui_frames(TICKS), 0);
    }

    #[test]
    fn sibling_reorder_does_not_recompute_unchanged_descendant_styles() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=4 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(2), node(4), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.insert(node(1), node(3), Some(node(2)));
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        assert!(work.style.is_empty());
        assert!(work.text.is_empty());
        assert!(work.focus_ime.is_empty());
        assert_eq!(work.input_hit_test, vec![node(3)]);
        assert_eq!(work.layout, vec![node(1)]);
        assert_eq!(work.render_extraction, vec![node(1), node(3)]);
    }

    #[test]
    fn paint_only_style_change_does_not_schedule_subtree_layout() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut paint = MutationQueue::new();
        paint.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    color: Some([0.2, 0.4, 0.8, 1.0]),
                    opacity: Some(0.8),
                    ..LayoutStyle::default()
                }),
                foreground: Some(SemanticColorRole::Accent),
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        world.commit(paint).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.style, vec![node(1), node(2), node(3)]);
        assert!(work.state.is_empty());
        assert!(work.transform.is_empty());
        assert!(work.text.is_empty());
        assert!(work.layout.is_empty());
        assert!(work.input_hit_test.is_empty());
        assert_eq!(work.render_extraction, vec![node(1), node(2), node(3)]);

        let mut layout = MutationQueue::new();
        layout.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(240.0)),
                    ..LayoutStyle::default()
                }),
                foreground: None,
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        world.commit(layout).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.layout, vec![node(1), node(2), node(3)]);
        assert_eq!(work.input_hit_test, vec![node(2), node(3)]);
    }

    #[test]
    fn text_change_with_unchanged_intrinsic_does_not_propagate_layout() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_text(
            node(3),
            TextContent {
                value: "abc".into(),
            },
        );
        world.commit(queue).unwrap();
        let mut shaper = FunctionalShaper::default();
        drain_scheduled_text(&mut world, &mut shaper);

        let mut same_size = MutationQueue::new();
        same_size.set_text(
            node(3),
            TextContent {
                value: "xyz".into(),
            },
        );
        world.commit(same_size).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.text, vec![node(3)]);
        assert_eq!(work.render_extraction, vec![node(3)]);
        assert!(work.layout.is_empty());
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, &mut shaper).unwrap();
        let after_shape = world.take_system_work();
        assert!(after_shape.layout.is_empty());
        assert!(!after_shape.text.contains(&node(1)));
        assert!(!after_shape.layout.contains(&node(1)));
        assert!(!after_shape.layout.contains(&node(2)));
    }

    #[test]
    fn text_change_that_changes_intrinsic_propagates_layout() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_text(
            node(3),
            TextContent {
                value: "abc".into(),
            },
        );
        world.commit(queue).unwrap();
        let mut shaper = FunctionalShaper::default();
        drain_scheduled_text(&mut world, &mut shaper);

        let mut longer = MutationQueue::new();
        longer.set_text(
            node(3),
            TextContent {
                value: "abcdef".into(),
            },
        );
        world.commit(longer).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.text, vec![node(3)]);
        assert!(work.layout.is_empty());
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, &mut shaper).unwrap();
        let after_shape = world.take_system_work();
        assert_eq!(after_shape.layout, vec![node(1), node(2), node(3)]);
    }

    #[test]
    fn wrapping_text_same_unconstrained_width_propagates_layout_when_wrap_height_changes() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_style(
            node(3),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    font_size: Some(10.0),
                    width: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            node(3),
            TextContent {
                value: "aaaaaaaa".into(),
            },
        );
        world.commit(queue).unwrap();
        let mut shaper = WordWrapShaper;
        drain_scheduled_text(&mut world, &mut shaper);
        let mut place = MutationQueue::new();
        place.write_layout(
            node(3),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 10.0,
            },
        );
        world.commit(place).unwrap();
        world.take_system_work();
        world.shape_text(&[node(3)], &mut shaper).unwrap();
        world.take_system_work();

        let mut wrapped = MutationQueue::new();
        wrapped.set_text(
            node(3),
            TextContent {
                value: "aa aa aa".into(),
            },
        );
        world.commit(wrapped).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.text, vec![node(3)]);
        assert!(work.layout.is_empty());
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, &mut shaper).unwrap();
        let after_shape = world.take_system_work();
        assert!(after_shape.layout.contains(&node(1)));
        assert!(after_shape.layout.contains(&node(2)));
        assert!(after_shape.layout.contains(&node(3)));
    }

    #[test]
    fn em_padding_wrap_and_card_content_use_computed_font_size() {
        #[derive(Default)]
        struct ConstraintProbe {
            constraints: Vec<crate::TextShapeConstraints>,
        }

        impl TextShaper for ConstraintProbe {
            fn shape(
                &mut self,
                _id: StableNodeId,
                _text: &TextContent,
                _style: &ComputedStyle,
                constraints: crate::TextShapeConstraints,
            ) -> TextMetrics {
                self.constraints.push(constraints);
                TextMetrics::default()
            }
        }

        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Text);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "card".into() },
        );
        let layout = Arc::new(LayoutStyle {
            font_size: Some(32.0),
            padding: Some(LengthSpec::Em(1.0)),
            ..LayoutStyle::default()
        });
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::clone(&layout),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout,
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            node(1),
            TextContent {
                value: "wrap".into(),
            },
        );
        queue.set_standard_visual(
            node(2),
            Some(StandardVisual::Card {
                title: None,
                kind: nana_ui_core::CardKind::Surface,
                loading: false,
                loading_phase: 0.0,
            }),
        );
        let box_200 = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        queue.write_layout(node(1), box_200);
        queue.write_layout(node(2), box_200);
        world.commit(queue).unwrap();
        world.resolve_styles(&[node(1), node(2)]).unwrap();

        let mut probe = ConstraintProbe::default();
        world.shape_text(&[node(1)], &mut probe).unwrap();
        assert_eq!(
            probe
                .constraints
                .last()
                .map(|constraints| constraints.max_width),
            Some(Some(136.0)),
            "1em padding at font-size 32px must wrap at 200-32-32, not 200-16-16"
        );

        let crate::ComponentGeometry::Card { content, .. } =
            world.component_geometry(node(2)).expect("card geometry")
        else {
            panic!("expected card geometry");
        };
        assert_eq!(content.x, 32.0);
        assert_eq!(content.y, 32.0);
        assert_eq!(content.width, 136.0);
        assert_eq!(content.height, 16.0);
    }

    #[test]
    fn work_counters_are_queryable_after_drain_and_extract() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();

        let mut work = world.take_system_work();
        let counters = work.counters();
        assert_eq!(counters.entities_total, 3);
        assert_eq!(counters.entities_spawned, 3);
        assert_eq!(counters.entities_despawned, 0);
        assert_eq!(counters.entities_changed, 3);
        assert_eq!(counters.style_processed, 3);
        assert_eq!(counters.layout_nodes, 3);
        assert_eq!(counters.render_nodes_changed, 3);
        assert_eq!(counters.render_nodes_extracted, 3);
        assert_eq!(counters.input_targets, 0);
        assert!(counters.allocations > 0);
        assert!(counters.allocated_bytes > 0);
        assert_eq!(counters.text_shaped_runs, 0);
        assert_eq!(world.last_work_counters().entities_changed, 3);
        assert_eq!(world.last_work_counters().render_nodes_changed, 3);
        assert_eq!(world.last_work_counters().render_nodes_extracted, 0);

        world.resolve_styles(&work.style).unwrap();
        let extracted = world.extract_nodes(&work.render_extraction);
        work.record_extract(&extracted);
        world.record_extract(&extracted);
        assert_eq!(work.counters().render_nodes_extracted, extracted.len());
        assert_eq!(work.counters().render_nodes_changed, 3);
        assert_eq!(
            world.last_work_counters().render_nodes_extracted,
            extracted.len()
        );
        assert_eq!(world.last_work_counters().render_nodes_changed, 3);

        let idle = world.take_system_work();
        assert!(idle.is_empty());
        assert_eq!(idle.counters().entities_spawned, 0);
        assert_eq!(idle.counters().entities_changed, 0);
        assert_eq!(idle.counters().allocations, 0);
        assert_eq!(idle.counters().text_shaped_runs, 0);
        assert_eq!(world.last_work_counters().entities_changed, 3);
        assert_eq!(
            world.last_work_counters().render_nodes_extracted,
            extracted.len()
        );
    }

    #[test]
    fn hot_path_allocations_and_text_shape_are_idle_zero_and_rise_on_mutation() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Text);
        queue.set_text(
            node(1),
            TextContent {
                value: "hello".into(),
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        assert!(work.counters().allocations > 0);
        assert!(work.counters().allocated_bytes > 0);
        assert_eq!(work.counters().text_shaped_runs, 0);
        world.resolve_styles(&work.style).unwrap();
        world
            .shape_text(&work.text, &mut FunctionalShaper::default())
            .unwrap();
        let after_shape = world.last_work_counters();
        assert!(after_shape.text_shaped_runs > 0);
        assert!(after_shape.text_layout_cache_misses > 0);
        assert_eq!(after_shape.text_layout_cache_hits, 0);
        assert!(after_shape.allocations > 0);

        let _ = world.layout_inputs(&work.layout).unwrap();
        let after_layout = world.last_work_counters();
        assert!(after_layout.allocations >= after_shape.allocations);

        let idle = {
            let mut idle = None;
            for _ in 0..8 {
                let work = world.take_system_work();
                if work.is_empty() {
                    idle = Some(work);
                    break;
                }
            }
            idle.expect("mutation follow-up work must settle")
        };
        assert_eq!(idle.counters().allocations, 0);
        assert_eq!(idle.counters().allocated_bytes, 0);
        assert_eq!(idle.counters().text_shaped_runs, 0);
        assert_eq!(idle.counters().text_layout_cache_misses, 0);
        assert!(world.last_work_counters().allocations > 0);

        let mut patch = MutationQueue::new();
        patch.set_text(
            node(1),
            TextContent {
                value: "world".into(),
            },
        );
        world.commit(patch).unwrap();
        let mutated = world.take_system_work();
        assert!(!mutated.text.is_empty());
        assert!(mutated.counters().allocations > 0);
        world.resolve_styles(&mutated.style).unwrap();
        world
            .shape_text(&mutated.text, &mut FunctionalShaper::default())
            .unwrap();
        let mutated_shape = world.last_work_counters();
        assert!(mutated_shape.text_shaped_runs > 0);
        assert!(mutated_shape.text_layout_cache_misses > 0);
    }

    #[test]
    fn text_layout_cache_miss_then_hit_and_shaper_without_glyph_backend_omits_glyph_cache() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Text);
        queue.set_text(
            node(1),
            TextContent {
                value: "cache-me".into(),
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(work.counters().glyph_cache_hits, None);
        assert_eq!(work.counters().glyph_cache_misses, None);
        assert_eq!(work.counters().cache_eviction, None);

        // Default TextShaper::shape_cached ignores GlyphCache.
        let mut shaper = FunctionalShaper::default();
        world.shape_text(&work.text, &mut shaper).unwrap();
        let missed = world.last_work_counters();
        assert!(missed.text_layout_cache_misses >= 1);
        assert_eq!(missed.text_layout_cache_hits, 0);
        assert!(missed.text_shaped_runs >= 1);
        assert_eq!(missed.glyph_cache_hits, None);
        assert_eq!(missed.glyph_cache_misses, None);
        assert_eq!(missed.cache_eviction, Some(0));

        world.shape_text(&work.text, &mut shaper).unwrap();
        let hit = world.last_work_counters();
        assert!(hit.text_layout_cache_hits >= 1);
        assert_eq!(
            hit.text_layout_cache_misses,
            missed.text_layout_cache_misses
        );
        assert_eq!(hit.text_shaped_runs, missed.text_shaped_runs);
        assert_eq!(hit.glyph_cache_hits, None);
        assert_eq!(hit.cache_eviction, Some(0));

        let mut place = MutationQueue::new();
        place.write_layout(
            node(1),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 24.0,
                height: 16.0,
            },
        );
        world.commit(place).unwrap();
        world.take_system_work();
        world
            .shape_text_for_layout(document(1), &mut shaper)
            .unwrap();
        let wrapped = world.last_work_counters();
        assert!(
            wrapped.text_layout_cache_misses >= 1,
            "max_width / wrap must miss the unconstrained cache entry"
        );
        let wrapped_hits = wrapped.text_layout_cache_hits;
        world
            .shape_text_for_layout(document(1), &mut shaper)
            .unwrap();
        let wrapped_hit = world.last_work_counters();
        assert!(wrapped_hit.text_layout_cache_hits > wrapped_hits);
        assert_eq!(wrapped_hit.glyph_cache_hits, None);
    }

    #[test]
    fn glyph_cache_miss_then_hit_on_measure_text_shaper() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Text);
        queue.set_text(node(1), TextContent { value: "ab".into() });
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(work.counters().glyph_cache_hits, None);
        assert_eq!(work.counters().glyph_cache_misses, None);

        // text-table / framework bench production shaper, via UiWorld::shape_text.
        let mut shaper = MeasureTextShaper;
        world.shape_text(&work.text, &mut shaper).unwrap();
        let missed = world.last_work_counters();
        assert_eq!(missed.glyph_cache_misses, Some(2));
        assert_eq!(missed.glyph_cache_hits, Some(0));
        assert!(missed.text_layout_cache_misses >= 1);

        world.shape_text(&work.text, &mut shaper).unwrap();
        let layout_hit = world.last_work_counters();
        assert!(layout_hit.text_layout_cache_hits >= 1);
        assert_eq!(layout_hit.glyph_cache_misses, Some(2));
        assert_eq!(layout_hit.glyph_cache_hits, Some(0));

        let mut patch = MutationQueue::new();
        patch.set_text(node(1), TextContent { value: "ba".into() });
        world.commit(patch).unwrap();
        let reused = world.take_system_work();
        world.resolve_styles(&reused.style).unwrap();
        world.shape_text(&reused.text, &mut shaper).unwrap();
        let hit = world.last_work_counters();
        assert_eq!(hit.glyph_cache_hits, Some(2));
        assert_eq!(hit.glyph_cache_misses, Some(0));
        assert!(hit.text_layout_cache_misses >= 1);
    }

    fn confirm_modal_visual() -> StandardVisual {
        StandardVisual::ModalFrame {
            title: Arc::from("Confirm"),
            description: None,
            body_text: None,
            kind: crate::ModalSurfaceKind::Confirm(nana_ui_core::DialogSize::Compact),
            busy: false,
            danger: false,
            slots: crate::ModalSlots::default(),
        }
    }

    fn clip_empty_state_visual() -> StandardVisual {
        StandardVisual::EmptyState {
            title: Arc::from("Empty"),
            message: None,
            icon: None,
            compact: true,
            action: None,
        }
    }

    #[test]
    fn parking_or_removing_the_last_presence_node_returns_the_skip_path() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "dialog".into(),
            },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(
            node(4),
            document(1),
            NodeKind::Element {
                tag: "section".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(1), node(4), None);
        queue.set_standard_visual(node(2), Some(confirm_modal_visual()));
        queue.set_standard_visual(node(3), Some(clip_empty_state_visual()));
        queue.set_style(
            node(4),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    z_index: Some(4),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        world.take_system_work();
        assert_eq!(world.confirm_modals, 1);
        assert_eq!(world.clip_visuals, 2);
        assert_eq!(world.z_index_nodes, 1);
        assert!(world.confirm_action_effect(node(3)).is_none());

        let mut park = MutationQueue::new();
        park.park_subtree(node(2));
        park.park_subtree(node(3));
        park.park_subtree(node(4));
        world.commit(park).unwrap();
        assert_eq!(world.confirm_modals, 0);
        assert_eq!(world.clip_visuals, 0);
        assert_eq!(world.z_index_nodes, 0);
        assert!(world.confirm_action_effect(node(1)).is_none());
        let remaining = world.extract_nodes(&[node(1)]);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].z_index, 0);

        let mut remount = MutationQueue::new();
        remount.insert(node(1), node(2), None);
        remount.insert(node(1), node(3), None);
        remount.insert(node(1), node(4), None);
        world.commit(remount).unwrap();
        assert_eq!(world.confirm_modals, 1);
        assert_eq!(world.clip_visuals, 2);
        assert_eq!(world.z_index_nodes, 1);

        let mut remove = MutationQueue::new();
        remove.detach(node(2));
        remove.detach(node(3));
        remove.detach(node(4));
        world.commit(remove).unwrap();
        assert_eq!(world.confirm_modals, 0);
        assert_eq!(world.clip_visuals, 0);
        assert_eq!(world.z_index_nodes, 0);
        assert!(world.confirm_action_effect(node(1)).is_none());
    }

    #[test]
    fn transform_and_a11y_mutations_do_not_schedule_layout() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=7 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(2), node(4), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut transform = MutationQueue::new();
        transform.set_style(
            node(4),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    transform: Some(PaintTransform {
                        e: 8.0,
                        ..PaintTransform::default()
                    }),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(transform).unwrap();
        let work = world.take_system_work();
        assert!(work.style.is_empty());
        assert!(work.state.is_empty());
        assert!(work.layout.is_empty());
        assert_eq!(work.counters().style_processed, 0);
        assert_eq!(work.counters().layout_nodes, 0);
        assert_eq!(work.transform, vec![node(4)]);
        assert_eq!(work.input_hit_test, vec![node(4)]);
        assert_eq!(work.render_extraction, vec![node(4)]);
        world.restore_system_work(work.clone());
        let restored = world.take_system_work();
        assert_eq!(restored.transform, work.transform);
        assert!(restored.style.is_empty());
        assert!(restored.layout.is_empty());

        let mut accessibility = MutationQueue::new();
        accessibility.set_accessibility(
            node(3),
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::from("beta")),
                ..AccessibilityState::default()
            },
        );
        world.commit(accessibility).unwrap();
        let work = world.take_system_work();
        assert!(work.layout.is_empty());
        assert_eq!(work.accessibility, vec![node(3)]);
    }

    #[test]
    fn frame_profiler_times_separable_runtime_stages() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Text);
        world.commit(queue).unwrap();
        let work = world.take_system_work();

        let mut profiler = crate::FrameProfiler::new();
        profiler.mark_runtime_unsupported();
        profiler.time(crate::FrameStage::Style, || {
            world.resolve_styles(&work.style).unwrap();
        });
        profiler.time(crate::FrameStage::TextShape, || {
            world
                .shape_text(&work.text, &mut FunctionalShaper::default())
                .unwrap();
        });
        if work.layout.is_empty() {
            profiler.skip(crate::FrameStage::Layout);
        } else {
            profiler.time(crate::FrameStage::Layout, || {
                world.layout_inputs(&work.layout).unwrap();
            });
        }
        profiler.time(crate::FrameStage::Extract, || {
            let extracted = world.extract_nodes(&work.render_extraction);
            world.record_extract(&extracted);
        });

        let profile = profiler.finish();
        assert_eq!(
            profile.stage(crate::FrameStage::Style).unwrap().status,
            crate::StageStatus::Ran
        );
        assert_eq!(
            profile.stage(crate::FrameStage::TextShape).unwrap().status,
            crate::StageStatus::Ran
        );
        assert_eq!(
            profile.stage(crate::FrameStage::Layout).unwrap().status,
            crate::StageStatus::Ran
        );
        assert_eq!(
            profile.stage(crate::FrameStage::GpuUpload).unwrap().status,
            crate::StageStatus::Unsupported
        );
        assert_eq!(
            profile.stage(crate::FrameStage::Batch).unwrap().status,
            crate::StageStatus::Unsupported
        );
    }

    fn drain_scheduled_text(world: &mut UiWorld, shaper: &mut impl TextShaper) {
        for _ in 0..8 {
            let work = world.take_system_work();
            if work.is_empty() {
                return;
            }
            world.resolve_styles(&work.style).unwrap();
            if !work.text.is_empty() {
                world.shape_text(&work.text, shaper).unwrap();
            }
        }
        panic!("scheduled text work did not settle");
    }

    #[test]
    fn hit_test_respects_cumulative_transform_and_ancestor_clip() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 50.0,
            },
        );
        queue.write_layout(
            node(2),
            LayoutBox {
                x: 40.0,
                y: 0.0,
                width: 30.0,
                height: 30.0,
            },
        );
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_x: OverflowSpec::Hidden,
                    transform: Some(PaintTransform {
                        e: 10.0,
                        ..PaintTransform::default()
                    }),
                    ..LayoutStyle::default()
                }),
                foreground: None,
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        queue.set_interaction(
            node(1),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
        );
        world.commit(queue).unwrap();
        world.rebuild_hit_test(document(1));

        assert_eq!(world.hit_test(document(1), 55.0, 10.0), Some(node(2)));
        assert_eq!(world.hit_test(document(1), 45.0, 10.0), None);
        assert_eq!(world.hit_test(document(1), 65.0, 10.0), None);
    }

    #[test]
    fn hit_test_follows_perspective_rotate_y_homography() {
        let mat = PaintMat4::perspective(800.0)
            .expect("d")
            .then(PaintMat4::rotate_y(30_f32.to_radians()));
        let layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        let (transform, persp) = LayoutStyle {
            transform_3d: Some(mat),
            ..LayoutStyle::default()
        }
        .world_scene_transform(layout.x, layout.y, layout.width, layout.height)
        .expect("homography");
        assert!(
            persp[0].abs() > 1e-8 || persp[1].abs() > 1e-8,
            "perspective+rotateY must keep (g,h)"
        );
        assert!(
            transformed_contains(layout, transform, persp, 100.0, 40.0),
            "projected center must hit"
        );
        let corners = mat
            .around_origin(0.0, 0.0, 100.0, 40.0)
            .projected_corners(0.0, 0.0, 200.0, 80.0)
            .expect("corners");
        let max_x = corners.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
        let min_y = corners.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        let empty_corner_x = max_x - 1.0;
        let empty_corner_y = min_y + 1.0;
        assert!(
            !transformed_contains(layout, transform, persp, empty_corner_x, empty_corner_y),
            "trapezoid AABB empty corner must miss, probe=({empty_corner_x}, {empty_corner_y}) corners={corners:?}"
        );

        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.write_layout(node(1), layout);
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    transform_3d: Some(mat),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        world.rebuild_hit_test(document(1));
        assert_eq!(world.hit_test(document(1), 100.0, 40.0), Some(node(1)));
        assert_eq!(
            world.hit_test(document(1), empty_corner_x, empty_corner_y),
            None
        );
    }

    #[test]
    fn hit_test_skips_css_pointer_events_none() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
            },
        );
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    pointer_events: Some(PointerEventsSpec::None),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        world.rebuild_hit_test(document(1));
        assert_eq!(world.hit_test(document(1), 10.0, 10.0), None);
    }

    #[test]
    fn hit_test_walks_z_order_and_skips_clipped_subtrees() {
        fn box_at(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
            LayoutBox {
                x,
                y,
                width,
                height,
            }
        }
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "root".into() },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "clip".into() },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "inside".into(),
            },
        );
        queue.create(
            node(4),
            document(1),
            NodeKind::Element {
                tag: "lower".into(),
            },
        );
        queue.create(
            node(5),
            document(1),
            NodeKind::Element {
                tag: "upper".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.insert(node(1), node(4), None);
        queue.insert(node(1), node(5), None);
        queue.write_layout(node(1), box_at(0.0, 0.0, 100.0, 100.0));
        queue.write_layout(node(2), box_at(0.0, 0.0, 40.0, 40.0));
        queue.write_layout(node(3), box_at(30.0, 0.0, 40.0, 20.0));
        queue.write_layout(node(4), box_at(50.0, 50.0, 40.0, 40.0));
        queue.write_layout(node(5), box_at(60.0, 50.0, 40.0, 40.0));
        queue.set_interaction(
            node(1),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
        );
        queue.set_interaction(
            node(2),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_x: OverflowSpec::Hidden,
                    overflow_y: OverflowSpec::Hidden,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        let mut raised = NodeStyle::default();
        Arc::make_mut(&mut raised.layout).z_index = Some(2);
        queue.set_style(node(4), raised);
        world.commit(queue).unwrap();
        world.rebuild_hit_test(document(1));

        assert_eq!(world.hit_test(document(1), 35.0, 10.0), Some(node(3)));
        assert_eq!(world.hit_test(document(1), 50.0, 10.0), None);
        assert_eq!(world.hit_test(document(1), 70.0, 60.0), Some(node(4)));
        let overlap = world.hit_test_candidates(document(1), 70.0, 60.0);
        assert_eq!(overlap.first().copied(), Some(node(4)));
        assert!(overlap.contains(&node(5)));
    }

    #[test]
    fn set_theme_marks_render_not_style_when_only_palette_roles_change() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.set_style(
            node(1),
            NodeStyle {
                background: Some(SemanticColorRole::Accent),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(
            world.extract_nodes(&[node(1)])[0].style.background,
            Some(nana_ui_core::SemanticPalette::dark().accent.as_rgba_array())
        );

        let mut theme = MutationQueue::new();
        theme.set_theme(ThemeMode::Light);
        world.commit(theme).unwrap();
        let work = world.take_system_work();
        assert!(work.style.is_empty());
        assert!(work.layout.is_empty());
        assert_eq!(work.render_extraction, vec![node(1)]);
        assert_eq!(
            world.extract_nodes(&work.render_extraction)[0]
                .style
                .background,
            Some(
                nana_ui_core::SemanticPalette::light()
                    .accent
                    .as_rgba_array()
            )
        );
    }

    #[test]
    fn grandchild_extract_inherits_ancestor_layout_color_without_restyling() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    color: Some([0.2, 0.4, 0.8, 1.0]),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(
            world.extract_nodes(&[node(3)])[0].style.color,
            Some([0.2, 0.4, 0.8, 1.0])
        );
        assert_eq!(
            world.extract_nodes(&[node(1), node(2), node(3)])[2]
                .style
                .color,
            Some([0.2, 0.4, 0.8, 1.0])
        );
    }

    #[test]
    fn theme_change_refreshes_inherited_foreground_on_extract_only() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_style(
            node(1),
            NodeStyle {
                foreground: Some(SemanticColorRole::Accent),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(
            world.extract_nodes(&[node(3)])[0].style.color,
            Some(nana_ui_core::SemanticPalette::dark().accent.as_rgba_array())
        );

        let mut theme = MutationQueue::new();
        theme.set_theme(ThemeMode::Light);
        world.commit(theme).unwrap();
        let work = world.take_system_work();
        assert!(work.style.is_empty());
        assert_eq!(
            world.extract_nodes(&[node(3)])[0].style.color,
            Some(
                nana_ui_core::SemanticPalette::light()
                    .accent
                    .as_rgba_array()
            )
        );
    }

    #[test]
    fn rejects_cycles_cross_document_parenting_and_invalid_sibling_anchor() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "main".into() },
        );
        queue.create(node(3), document(2), NodeKind::Document);
        queue.insert(node(1), node(2), None);
        world.commit(queue).unwrap();

        let mut cycle = MutationQueue::new();
        cycle.insert(node(2), node(1), None);
        assert_eq!(
            world.commit(cycle),
            Err(UiWorldError::Cycle {
                parent: node(2),
                child: node(1)
            })
        );

        let mut cross_document = MutationQueue::new();
        cross_document.insert(node(1), node(3), None);
        assert_eq!(
            world.commit(cross_document),
            Err(UiWorldError::CrossDocument {
                parent: node(1),
                child: node(3)
            })
        );

        let mut invalid_before = MutationQueue::new();
        invalid_before.insert(node(1), node(2), Some(node(3)));
        assert_eq!(
            world.commit(invalid_before),
            Err(UiWorldError::InvalidBefore {
                parent: node(1),
                before: node(3)
            })
        );
    }

    #[derive(Default)]
    struct FunctionalShaper {
        calls: Vec<StableNodeId>,
    }

    impl TextShaper for FunctionalShaper {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            self.calls.push(id);
            TextMetrics {
                width: text.value.chars().count() as f32 * style.font_size,
                height: style.font_size,
            }
        }
    }

    struct WordWrapShaper;

    impl TextShaper for WordWrapShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            text: &TextContent,
            style: &ComputedStyle,
            constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            let em = style.font_size.max(1.0);
            let intrinsic = text.value.chars().count() as f32 * em;
            let Some(max_width) = constraints.max_width.filter(|_| constraints.wrap) else {
                return TextMetrics {
                    width: intrinsic,
                    height: em,
                };
            };
            let mut lines = 1_u32;
            let mut line = 0.0;
            for word in text.value.split_whitespace() {
                let word_width = word.chars().count() as f32 * em;
                if line > 0.0 && line + em + word_width > max_width {
                    lines += 1;
                    line = word_width;
                } else {
                    if line > 0.0 {
                        line += em;
                    }
                    line += word_width;
                }
            }
            TextMetrics {
                width: intrinsic.min(max_width),
                height: em * lines as f32,
            }
        }
    }

    #[test]
    fn style_text_layout_input_focus_ime_hit_test_and_extraction_form_one_pipeline() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "input".into(),
            },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    opacity: Some(0.5),
                    z_index: Some(2),
                    ..LayoutStyle::default()
                }),
                foreground: Some(SemanticColorRole::Accent),
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            node(3),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    opacity: Some(0.5),
                    font_size: Some(20.0),
                    ..LayoutStyle::default()
                }),
                foreground: None,
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            node(3),
            TextContent {
                value: "输入".into(),
            },
        );
        queue.set_interaction(
            node(2),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        world.commit(queue).unwrap();

        let work = world.take_system_work();
        assert_eq!(work.text, vec![node(3)]);
        world.resolve_styles(&work.style).unwrap();
        let mut shaper = FunctionalShaper::default();
        world.shape_text(&work.text, &mut shaper).unwrap();
        assert_eq!(shaper.calls, vec![node(3)]);
        let layout = world.layout_inputs(&work.layout).unwrap();
        let text = layout.iter().find(|input| input.id == node(3)).unwrap();
        assert_eq!(text.parent, Some(node(2)));
        assert_eq!(text.text_metrics.unwrap().width, 40.0);

        let mut queue = MutationQueue::new();
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        queue.write_layout(
            node(2),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 40.0,
            },
        );
        queue.write_layout(
            node(3),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 40.0,
                height: 20.0,
            },
        );
        queue.request_focus(document(1), Some(node(2)));
        queue.set_text_input(node(2), Some(TextInputState::new("")));
        queue.set_ime(
            node(2),
            Some(ImeComposition {
                text: "拼音".into(),
                selection: Some((0, 6)),
            }),
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.reconcile_focus(&work.focus_ime);
        world.rebuild_hit_test(document(1));
        assert_eq!(world.hit_test(document(1), 20.0, 20.0), Some(node(2)));

        let extracted = world.extract_document(document(1));
        let input = extracted.iter().find(|entry| entry.id == node(2)).unwrap();
        let text = extracted.iter().find(|entry| entry.id == node(3)).unwrap();
        assert!(input.focused);
        assert_eq!(input.ime.as_ref().unwrap().text, "拼音");
        assert_eq!(text.style.foreground, SemanticColorRole::Accent);
        assert_eq!(text.style.opacity, 0.25);

        let mut queue = MutationQueue::new();
        queue.set_interaction(
            node(2),
            InteractionState {
                focusable: false,
                ..InteractionState::default()
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.reconcile_focus(&work.focus_ime);
        assert_eq!(world.focused(document(1)), None);
        assert!(
            world
                .extract_document(document(1))
                .iter()
                .find(|entry| entry.id == node(2))
                .unwrap()
                .ime
                .is_none()
        );
    }

    #[test]
    fn invalid_visual_input_is_rejected_atomically() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Text);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.set_text(
            node(1),
            TextContent {
                value: "valid".into(),
            },
        );
        queue.write_layout(
            node(1),
            LayoutBox {
                width: -1.0,
                ..LayoutBox::default()
            },
        );
        assert_eq!(
            world.commit(queue),
            Err(UiWorldError::InvalidLayout(node(1)))
        );
        assert_eq!(world.generation(), 1);
        assert!(world.take_system_work().is_empty());
    }

    #[test]
    fn custom_render_extension_requires_nonempty_backend_neutral_keys() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "gpu".into() },
        );
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.set_custom_render(node(1), Some(CustomRenderNode::new("", "program", 0)));
        assert_eq!(
            world.commit(queue),
            Err(UiWorldError::InvalidCustomRender(node(1)))
        );
        assert!(world.custom_render(node(1)).is_none());
        assert!(world.take_system_work().is_empty());
    }

    #[test]
    fn accessibility_projection_uses_runtime_hierarchy_focus_and_geometry() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.create(node(2), document(1), NodeKind::Text);
        queue.create(node(3), document(1), NodeKind::Comment);
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.set_text(
            node(2),
            TextContent {
                value: "Build".into(),
            },
        );
        queue.set_interaction(
            node(1),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        queue.set_accessibility(
            node(1),
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::from("Build project")),
                value: None,
                disabled: false,
                checked: None,
                selected: None,
                multiline: false,
                editable: false,
                modal: false,
                ..AccessibilityState::default()
            },
        );
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 28.0,
            },
        );
        queue.request_focus(document(1), Some(node(1)));
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();

        let projected = world.project_accessibility(document(1));
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].role, AccessibilityRole::Button);
        assert_eq!(projected[0].label.as_deref(), Some("Build project"));
        assert!(projected[0].focused);
        assert_eq!(projected[0].children, vec![node(2)]);
        assert_eq!(projected[0].bounds.width, 80.0);
        assert_eq!(projected[1].role, AccessibilityRole::Text);
        assert_eq!(projected[1].label.as_deref(), Some("Build"));

        let mut queue = MutationQueue::new();
        queue.set_accessibility(
            node(1),
            AccessibilityState {
                disabled: true,
                ..world.accessibility(node(1)).unwrap().clone()
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.accessibility, vec![node(1)]);
        assert!(work.style.is_empty());
        assert!(work.layout.is_empty());
    }

    #[test]
    fn accessibility_delta_removes_and_restores_hidden_subtrees_atomically() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(world.project_accessibility(document(1)).len(), 3);

        let mut hide = MutationQueue::new();
        hide.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    hidden: true,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(hide).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let hidden = world.project_accessibility_delta(&work);
        assert_eq!(hidden.removed, vec![node(2), node(3)]);
        assert_eq!(
            hidden
                .updated
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![node(1)]
        );
        assert!(hidden.updated[0].children.is_empty());

        let mut show = MutationQueue::new();
        show.set_style(node(2), NodeStyle::default());
        world.commit(show).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let visible = world.project_accessibility_delta(&work);
        assert!(visible.removed.is_empty());
        assert_eq!(
            visible
                .updated
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![node(1), node(2), node(3)]
        );
        assert_eq!(visible.updated[0].children, vec![node(2)]);
        assert_eq!(visible.updated[1].children, vec![node(3)]);
    }

    #[test]
    fn pointer_capture_and_event_routes_share_runtime_hierarchy_and_lifetime() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();

        assert_eq!(
            world.event_route(node(3)).unwrap(),
            EventRoute {
                capture: vec![node(1), node(2)],
                target: node(3),
                bubble: vec![node(2), node(1)],
            }
        );

        let mut queue = MutationQueue::new();
        queue.capture_pointer(7, node(2));
        queue.capture_pointer(7, node(3));
        world.commit(queue).unwrap();
        assert_eq!(world.pointer_capture(document(1), 7), Some(node(3)));
        assert_eq!(
            world.take_pointer_capture_changes(),
            vec![
                PointerCaptureChange {
                    pointer_id: 7,
                    target: node(2),
                    captured: true,
                },
                PointerCaptureChange {
                    pointer_id: 7,
                    target: node(2),
                    captured: false,
                },
                PointerCaptureChange {
                    pointer_id: 7,
                    target: node(3),
                    captured: true,
                },
            ]
        );

        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.release_pointer(7, node(2));
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::PointerCaptureMismatch {
                pointer_id: 7,
                target: node(2),
            })
        );
        assert_eq!(world.generation(), generation);
        assert_eq!(world.pointer_capture(document(1), 7), Some(node(3)));

        let mut remove = MutationQueue::new();
        remove.despawn_subtree(node(2));
        world.commit(remove).unwrap();
        assert_eq!(world.pointer_capture(document(1), 7), None);
        assert_eq!(
            world.take_pointer_capture_changes(),
            vec![PointerCaptureChange {
                pointer_id: 7,
                target: node(3),
                captured: false,
            }]
        );
        assert!(world.event_route(node(3)).is_none());
    }

    #[test]
    fn event_listeners_are_runtime_query_authority() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.set_event_listener(node(1), "click", true);
        queue.set_event_listener(node(1), "input", true);
        world.commit(queue).unwrap();

        assert!(world.has_event(node(1), "click"));
        assert!(world.has_event(node(1), "input"));
        assert!(!world.has_event(node(1), "keydown"));
        assert!(
            world
                .event_targets(document(1))
                .contains(&(1, "click".into()))
        );

        let mut queue = MutationQueue::new();
        queue.set_event_listener(node(1), "click", false);
        world.commit(queue).unwrap();
        assert!(!world.has_event(node(1), "click"));
        assert!(world.has_event(node(1), "input"));

        let mut remove = MutationQueue::new();
        remove.despawn_subtree(node(1));
        world.commit(remove).unwrap();
        assert!(!world.has_event(node(1), "input"));
        assert!(world.event_targets(document(1)).is_empty());
    }

    #[test]
    fn pointer_hover_and_press_are_runtime_owned_and_targeted() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Element { tag: "a".into() });
        queue.create(node(3), document(1), NodeKind::Element { tag: "b".into() });
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        for id in [node(2), node(3)] {
            queue.set_style(
                id,
                NodeStyle {
                    interaction: crate::InteractionStyle {
                        hovered: crate::SemanticPaint {
                            background: Some(SemanticColorRole::Hover),
                            ..crate::SemanticPaint::default()
                        },
                        pressed: crate::SemanticPaint {
                            background: Some(SemanticColorRole::Active),
                            ..crate::SemanticPaint::default()
                        },
                        ..crate::InteractionStyle::default()
                    },
                    ..NodeStyle::default()
                },
            );
        }
        world.commit(queue).unwrap();
        world.take_system_work();

        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(2))),
            Ok(None)
        );
        assert_eq!(world.pointer_hover(document(1), 7), Some(node(2)));
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2)]);
        assert_eq!(work.style, vec![node(2)]);
        assert!(work.layout.is_empty());
        assert!(work.transform.is_empty());
        assert!(work.input_hit_test.is_empty());
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(
            world.extract_nodes(&[node(2)])[0].style.background,
            Some(nana_ui_core::SemanticPalette::dark().hover.as_rgba_array())
        );
        let generation = world.generation();
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(2))),
            Ok(Some(node(2)))
        );
        assert_eq!(world.generation(), generation);
        assert!(world.take_system_work().is_empty());

        assert_eq!(world.press_pointer(document(1), 7, node(2)), Ok(None));
        assert_eq!(world.pointer_press(document(1), 7), Some(node(2)));
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2)]);
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(
            world.extract_nodes(&[node(2)])[0].style.background,
            Some(nana_ui_core::SemanticPalette::dark().active.as_rgba_array())
        );
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(3))),
            Ok(Some(node(2)))
        );
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2), node(3)]);
        assert_eq!(work.style, vec![node(2), node(3)]);
        assert_eq!(world.release_pointer_press(document(1), 7), Some(node(2)));

        let mut disable = MutationQueue::new();
        disable.set_interaction(
            node(3),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
        );
        world.commit(disable).unwrap();
        assert_eq!(world.pointer_hover(document(1), 7), None);
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(3))),
            Err(UiWorldError::NotPointerInteractive(node(3)))
        );
    }

    #[test]
    fn pointer_hover_without_interaction_style_dirties_state_not_style() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Element { tag: "a".into() });
        queue.create(node(3), document(1), NodeKind::Element { tag: "b".into() });
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(2))),
            Ok(None)
        );
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2)]);
        assert!(work.style.is_empty());
        assert!(work.layout.is_empty());
        assert!(work.transform.is_empty());
        assert!(work.render_extraction.is_empty());
        assert_eq!(work.counters().style_processed, 0);

        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(3))),
            Ok(Some(node(2)))
        );
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2), node(3)]);
        assert!(work.style.is_empty());
        assert!(work.render_extraction.is_empty());
    }

    #[test]
    fn state_only_invalidation_stays_observable_without_scheduling_a_frame() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Element { tag: "a".into() });
        queue.insert(node(1), node(2), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        world
            .set_pointer_hover(document(1), 7, Some(node(2)))
            .unwrap();
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2)]);
        // Without interaction styling nothing downstream reads the hover, so the
        // drain must not claim a frame's worth of work.
        assert!(work.is_empty());

        let mut styled = MutationQueue::new();
        styled.set_style(
            node(2),
            NodeStyle {
                interaction: crate::InteractionStyle {
                    hovered: crate::SemanticPaint {
                        background: Some(SemanticColorRole::Hover),
                        ..crate::SemanticPaint::default()
                    },
                    ..crate::InteractionStyle::default()
                },
                ..NodeStyle::default()
            },
        );
        world.commit(styled).unwrap();
        world.take_system_work();
        world.set_pointer_hover(document(1), 7, None).unwrap();
        world.take_system_work();

        world
            .set_pointer_hover(document(1), 7, Some(node(2)))
            .unwrap();
        // The same hover now has a consumer, and STYLE carries the frame.
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2)]);
        assert_eq!(work.style, vec![node(2)]);
        assert!(!work.is_empty());
    }

    #[test]
    fn request_focus_dirties_state_without_requiring_style() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.set_interaction(
            node(2),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut focus = MutationQueue::new();
        focus.request_focus(document(1), Some(node(2)));
        world.commit(focus).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.state, vec![node(2)]);
        assert!(work.style.is_empty());
        assert!(work.transform.is_empty());
        assert!(work.layout.is_empty());
        assert_eq!(work.focus_ime, vec![node(2)]);
        assert_eq!(work.render_extraction, vec![node(2)]);
    }

    #[test]
    fn scroll_offset_moves_descendant_hit_testing_without_rewriting_layout() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "scroll".into(),
            },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element { tag: "item".into() },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.write_layout(
            node(2),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
        );
        queue.write_layout(
            node(3),
            LayoutBox {
                x: 0.0,
                y: 80.0,
                width: 100.0,
                height: 20.0,
            },
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_y: OverflowSpec::Scroll,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        world.take_system_work();
        world.rebuild_hit_test(document(1));

        let mut scroll = MutationQueue::new();
        scroll.set_scroll_offset(node(2), ScrollOffset { x: 0.0, y: 60.0 });
        world.commit(scroll).unwrap();
        assert_eq!(world.layout_box(node(3)).unwrap().y, 80.0);
        assert_eq!(world.scroll_offset(node(2)).unwrap().y, 60.0);
        let work = world.take_system_work();
        // Scroller-only hit/extract; Scene recomposes descendants from offset.
        assert_eq!(work.input_hit_test, vec![node(2)]);
        assert_eq!(work.render_extraction, vec![node(2)]);
        assert!(work.layout.is_empty());
        let scroll_updates = world.take_scroll_hit_updates();
        assert!(world.hit_test_work_is_scroll_only(&work.input_hit_test, &scroll_updates));
        for (scroller, delta) in scroll_updates {
            world.update_hit_test_scroll(document(1), scroller, delta);
        }
        assert_eq!(world.hit_test(document(1), 10.0, 25.0), Some(node(3)));
        assert_ne!(world.hit_test(document(1), 10.0, 85.0), Some(node(3)));
        // Scroller chrome is un-scrolled: a point on the chrome (not the
        // shifted item) still hits the scroller after the patch.
        assert_eq!(world.hit_test(document(1), 10.0, 45.0), Some(node(2)));
        assert_eq!(world.hit_test(document(1), 10.0, 5.0), Some(node(2)));
        let patched_scroller = hit_entry_transform(&world, document(1), node(2));
        let patched_item = hit_entry_transform(&world, document(1), node(3));
        // The in-place patch must agree with a full rebuild, including the
        // scroller's own transform.
        world.rebuild_hit_test(document(1));
        assert_eq!(
            patched_scroller,
            hit_entry_transform(&world, document(1), node(2))
        );
        assert_eq!(
            patched_item,
            hit_entry_transform(&world, document(1), node(3))
        );
        assert_eq!(world.hit_test(document(1), 10.0, 25.0), Some(node(3)));
        assert_ne!(world.hit_test(document(1), 10.0, 85.0), Some(node(3)));
        assert_eq!(world.hit_test(document(1), 10.0, 45.0), Some(node(2)));
        assert_eq!(world.hit_test(document(1), 10.0, 5.0), Some(node(2)));

        let mut metrics = MutationQueue::new();
        metrics.set_scroll_metrics(
            node(2),
            Some(ScrollMetrics {
                viewport_width: 100.0,
                viewport_height: 50.0,
                content_width: 100.0,
                content_height: 100.0,
            }),
        );
        world.commit(metrics).unwrap();
        assert_eq!(world.scroll_offset(node(2)).unwrap().y, 50.0);
        let work = world.take_system_work();
        // The metrics clamp re-anchors the offset; the scroller-only input
        // mark plus the recorded delta cover the hit index update.
        assert_eq!(work.input_hit_test, vec![node(2)]);
        assert!(work.layout.is_empty());

        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.set_scroll_offset(node(2), ScrollOffset { x: 0.0, y: -1.0 });
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::InvalidScrollOffset(node(2)))
        );
        assert_eq!(world.generation(), generation);

        let mut invalid_metrics = MutationQueue::new();
        invalid_metrics.set_scroll_metrics(
            node(2),
            Some(ScrollMetrics {
                viewport_width: f32::NAN,
                viewport_height: 50.0,
                content_width: 100.0,
                content_height: 100.0,
            }),
        );
        assert_eq!(
            world.commit(invalid_metrics),
            Err(UiWorldError::InvalidScrollMetrics(node(2)))
        );
        assert_eq!(world.generation(), generation);
    }

    #[test]
    fn write_layout_does_not_extract_bit_identical_descendants() {
        fn box_at(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
            LayoutBox {
                x,
                y,
                width,
                height,
            }
        }
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "column".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        // Three rows, each with a label. Middle-row size change shifts the
        // row below; the row above and the middle label stay bit-identical.
        for row in 0..3u64 {
            let row_id = node(3 + row * 2);
            let label_id = node(4 + row * 2);
            queue.create(row_id, document(1), NodeKind::Element { tag: "row".into() });
            queue.create(label_id, document(1), NodeKind::Text);
            queue.insert(node(2), row_id, None);
            queue.insert(row_id, label_id, None);
        }
        queue.write_layout(node(1), box_at(0.0, 0.0, 100.0, 60.0));
        queue.write_layout(node(2), box_at(0.0, 0.0, 100.0, 60.0));
        queue.write_layout(node(3), box_at(0.0, 0.0, 100.0, 20.0));
        queue.write_layout(node(4), box_at(0.0, 0.0, 40.0, 20.0));
        queue.write_layout(node(5), box_at(0.0, 20.0, 100.0, 20.0));
        queue.write_layout(node(6), box_at(0.0, 20.0, 40.0, 20.0));
        queue.write_layout(node(7), box_at(0.0, 40.0, 100.0, 20.0));
        queue.write_layout(node(8), box_at(0.0, 40.0, 40.0, 20.0));
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut changed = MutationQueue::new();
        // Parent grew; WriteLayout must not mark the whole subtree.
        changed.write_layout(node(2), box_at(0.0, 0.0, 100.0, 72.0));
        changed.write_layout(node(5), box_at(0.0, 20.0, 100.0, 32.0));
        changed.write_layout(node(7), box_at(0.0, 52.0, 100.0, 20.0));
        changed.write_layout(node(8), box_at(0.0, 52.0, 40.0, 20.0));
        world.commit(changed).unwrap();
        let work = world.take_system_work();
        assert!(work.render_extraction.contains(&node(2)));
        assert!(work.render_extraction.contains(&node(5)));
        assert!(work.render_extraction.contains(&node(7)));
        assert!(work.render_extraction.contains(&node(8)));
        assert!(!work.render_extraction.contains(&node(3)));
        assert!(!work.render_extraction.contains(&node(4)));
        assert!(
            !work.render_extraction.contains(&node(6)),
            "bit-identical middle-row label must not be extracted because its ancestor was written"
        );
        assert!(work.layout.is_empty());
        assert_eq!(work.input_hit_test, work.render_extraction);
        assert_eq!(work.accessibility, work.render_extraction);
    }

    #[test]
    fn backend_neutral_text_geometry_treats_crlf_and_graphemes_atomically() {
        let text = TextContent {
            value: "A\r\n👩‍💻 e\u{301}".into(),
        };
        let style = ComputedStyle {
            font_size: 10.0,
            line_height: Some(nana_ui_core::LineHeightSpec::Absolute(14.0)),
            ..ComputedStyle::default()
        };
        let constraints = crate::TextShapeConstraints::default();
        let mut shaper = FunctionalShaper::default();
        let second_line = "A\r\n".len();

        assert_eq!(
            shaper.text_position(node(1), &text, second_line, &style, constraints),
            (0.0, 14.0, 14.0)
        );
        assert_eq!(
            shaper
                .text_highlights(node(1), &text, (0, "A".len()), &style, constraints)
                .len(),
            1
        );
        assert_eq!(
            shaper
                .text_highlights(
                    node(1),
                    &text,
                    (0, second_line + "👩‍💻".len()),
                    &style,
                    constraints,
                )
                .len(),
            2
        );
        assert_eq!(
            shaper.text_position(
                node(1),
                &text,
                second_line + "👩".len(),
                &style,
                constraints
            ),
            (0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn new_standard_visuals_derive_scene_geometry() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "calendar-heatmap".into(),
            },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "time-series-chart".into(),
            },
        );
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 200.0,
                height: 120.0,
            },
        );
        queue.write_layout(
            node(2),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 108.0,
                height: 120.0,
            },
        );
        queue.set_standard_visual(
            node(1),
            Some(StandardVisual::CalendarHeatmap {
                cells: Arc::from([
                    crate::CalendarHeatmapCellPaint {
                        x: 42.0,
                        y: 14.0,
                        level: 0,
                    },
                    crate::CalendarHeatmapCellPaint {
                        x: 42.0,
                        y: 28.0,
                        level: 4,
                    },
                ]),
                month_labels: Arc::from([crate::CalendarHeatmapLabelPaint {
                    text: Arc::from("6月"),
                    x: 47.5,
                    y: 0.0,
                }]),
                day_labels: Arc::from([crate::CalendarHeatmapLabelPaint {
                    text: Arc::from("周一"),
                    x: 0.0,
                    y: 24.0,
                }]),
                cell_size: 11.0,
                cell_radius: 2.0,
                max_level: 4,
                active: Some(1),
                active_title: Some(Arc::from("2026-06-03: 8")),
            }),
        );
        queue.set_standard_visual(
            node(2),
            Some(StandardVisual::TimeSeriesChart {
                values: Arc::from([0.0, 5.0, 10.0]),
            }),
        );
        world.commit(queue).unwrap();

        let crate::ComponentGeometry::CalendarHeatmap {
            cells,
            labels,
            hover,
        } = world
            .component_geometry(node(1))
            .expect("calendar geometry")
        else {
            panic!("expected calendar heatmap geometry");
        };
        assert_eq!(cells.len(), 2);
        assert_eq!(
            cells[0].0,
            LayoutBox {
                x: 52.0,
                y: 34.0,
                width: 11.0,
                height: 11.0,
            }
        );
        assert_eq!(
            cells[1].0,
            LayoutBox {
                x: 52.0,
                y: 48.0,
                width: 11.0,
                height: 11.0,
            }
        );
        assert_ne!(
            cells[0].1, cells[1].1,
            "active cell uses a stronger fill than the idle cell"
        );
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].content.as_ref(), "6月");
        assert_eq!(labels[0].bounds.y, 20.0);
        assert!(
            (labels[0].bounds.x + labels[0].bounds.width * 0.5 - 57.5).abs() < 0.01,
            "month labels stay centered on the first week cell"
        );
        assert!(
            labels[0].bounds.width >= 10.0 + 10.0 * 0.62,
            "month CJK must not use the Latin 0.62em estimate"
        );
        assert_eq!(labels[1].bounds.x, 10.0);
        assert_eq!(labels[1].content.as_ref(), "周一");
        assert!(
            labels[1].bounds.width >= 22.0,
            "weekday CJK must keep a full-em box so 周一 is not clipped"
        );
        let hover = hover.expect("active cell paints hover chrome");
        assert_eq!(hover.title.content.as_ref(), "2026-06-03: 8");
        assert!(hover.tooltip.width < 176.0);

        let crate::ComponentGeometry::TimeSeriesChart {
            grid, area, line, ..
        } = world.component_geometry(node(2)).expect("chart geometry")
        else {
            panic!("expected time series geometry");
        };
        assert_eq!(grid.len(), 4);
        assert_eq!(grid[0].x, 8.0);
        assert_eq!(grid[0].height, 1.0);
        assert!(!area.is_empty());
        assert!(area.iter().all(|strip| strip.width <= 2.0 + f32::EPSILON));
        let expected_line = crate::TimeSeriesChart::new([0.0, 5.0, 10.0])
            .points(LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 108.0,
                height: 120.0,
            })
            .into_iter()
            .map(|(x, y)| [x, y])
            .collect::<Vec<_>>();
        assert_eq!(line, expected_line);
    }

    #[test]
    fn diagonal_stroke_segments_overlap_so_curves_do_not_break() {
        let points = sample_curve(
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 120.0,
            },
            [
                GraphPoint::new(10.0, 40.0),
                GraphPoint::new(80.0, 40.0),
                GraphPoint::new(120.0, 80.0),
                GraphPoint::new(190.0, 80.0),
            ],
        );
        assert_curve_spacing(&points);
    }

    #[test]
    fn long_zoomed_curves_keep_segment_spacing() {
        let points = sample_curve(
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 4000.0,
                height: 800.0,
            },
            [
                GraphPoint::new(0.0, 40.0),
                GraphPoint::new(800.0, 40.0),
                GraphPoint::new(1600.0, 760.0),
                GraphPoint::new(2400.0, 760.0),
            ],
        );
        assert!(points.len() > 96);
        assert_curve_spacing(&points);
    }

    fn assert_curve_spacing(points: &[[f32; 2]]) {
        assert!(points.len() > 1);
        assert!(points.windows(2).all(|pair| {
            (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]) <= CURVE_MAX_SEGMENT
        }));
    }

    #[test]
    fn hovered_graph_edge_uses_muted_gray_instead_of_accent() {
        let paint = |hovered, selected| crate::GraphEdgePaint {
            curve: [GraphPoint::ZERO; 4],
            selected,
            hovered,
            connecting: false,
            label: None,
        };
        let dark = SemanticPalette::dark();
        assert_eq!(
            graph_edge_stroke_color(&dark, &paint(true, false)),
            dark.muted.as_rgba_array()
        );
        assert_ne!(
            graph_edge_stroke_color(&dark, &paint(true, false)),
            dark.accent.as_rgba_array()
        );
        assert_eq!(
            graph_edge_stroke_color(&dark, &paint(false, true)),
            dark.text.as_rgba_array()
        );
        let light = SemanticPalette::light();
        assert_eq!(
            graph_edge_stroke_color(&light, &paint(true, false)),
            light.muted.as_rgba_array()
        );
        assert_ne!(
            graph_edge_stroke_color(&light, &paint(true, false)),
            light.accent.as_rgba_array()
        );
    }

    #[test]
    fn idle_extract_shares_kind_style_and_children() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_text(node(3), TextContent { value: "hi".into() });
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();

        let first = world.extract_nodes(&[node(1), node(2), node(3)]);
        let second = world.extract_nodes(&[node(1), node(2), node(3)]);
        assert_eq!(first, second);
        assert_eq!(first.len(), 3);
        assert!(Arc::ptr_eq(&first[0].kind, &first[1].kind));
        for (left, right) in first.iter().zip(&second) {
            assert!(Arc::ptr_eq(&left.kind, &right.kind));
            assert!(Arc::ptr_eq(&left.style, &right.style));
            assert!(Arc::ptr_eq(&left.children, &right.children));
        }
        assert_eq!(first[0].children.as_slice(), &[node(2)]);
        assert_eq!(first[1].children.as_slice(), &[node(3)]);
        assert!(first[2].children.is_empty());
        assert_eq!(
            first[2].text.as_ref().map(|text| text.value.as_str()),
            Some("hi")
        );
        assert!(first[0].text_spans.is_empty());
        assert!(first[1].text_spans.is_empty());
    }

    #[test]
    fn dirty_extract_updates_changed_slots_and_keeps_idle_arcs() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(node(1), node(2), None);
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let before = world.extract_nodes(&[node(1), node(2)]);

        let mut paint = MutationQueue::new();
        paint.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    background: Some([1.0, 0.0, 0.0, 1.0]),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(paint).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let painted = world.extract_nodes(&[node(1), node(2)]);
        assert_eq!(painted.len(), 2);
        assert!(Arc::ptr_eq(&before[0].kind, &painted[0].kind));
        assert!(Arc::ptr_eq(&before[0].children, &painted[0].children));
        assert!(Arc::ptr_eq(&before[0].style, &painted[0].style));
        assert!(Arc::ptr_eq(&before[1].kind, &painted[1].kind));
        assert!(Arc::ptr_eq(&before[1].children, &painted[1].children));
        assert!(!Arc::ptr_eq(&before[1].style, &painted[1].style));
        assert_eq!(painted[1].style.background, Some([1.0, 0.0, 0.0, 1.0]));

        let mut insert = MutationQueue::new();
        insert.create(
            node(3),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        insert.insert(node(1), node(3), None);
        world.commit(insert).unwrap();
        world.take_system_work();
        let reparented = world.extract_nodes(&[node(1)]);
        assert_eq!(reparented.len(), 1);
        assert!(!Arc::ptr_eq(&painted[0].children, &reparented[0].children));
        assert_eq!(reparented[0].children.as_slice(), &[node(2), node(3)]);
        assert!(Arc::ptr_eq(&painted[0].kind, &reparented[0].kind));
    }
}
