//! Cohesive state subsystems partitioned out of [`super::UiWorld`].

use super::*;

/// Retained tree topology, detachment, retirement, and per-component indexes.
#[derive(Default)]
pub(in crate::world) struct WorldTree {
    pub(in crate::world) retired: RetiredIds,
    /// Last applied presence flags per entity, so park/remove/despawn can
    /// decrement without double-counting.
    pub(in crate::world) presence_flags: HashMap<StableNodeId, PresenceFlags, BuildIdHasher>,
    /// Subtree roots detached by Remove or Park.
    pub(in crate::world) detached: HashSet<StableNodeId, BuildIdHasher>,
    /// The `detached` roots that are still `Mounted`.
    pub(in crate::world) detached_mounted: HashSet<StableNodeId, BuildIdHasher>,
    /// Live roots per document: `parent.is_none()` and `presence_live`.
    pub(in crate::world) live_document_roots:
        HashMap<DocumentId, BTreeSet<StableNodeId>, BuildIdHasher>,
    /// Live nodes grouped by the component that created them.
    pub(in crate::world) nodes_by_component: HashMap<ComponentTypeId, NodeSet>,
    pub(in crate::world) spawned_since_drain: usize,
    pub(in crate::world) despawned_since_drain: usize,
    /// Children already taken out of a parent's list in one removal run.
    pub(in crate::world) unlinked_ahead: NodeSet,
    /// Parents whose children still need numbering once the apply loop ends.
    pub(in crate::world) renumber_later: Option<NodeMap<(usize, usize)>>,
    /// Parents whose child list changed since the last drain.
    pub(in crate::world) structural_change_parents: Vec<StableNodeId>,
    #[cfg(test)]
    pub(in crate::world) block_read_steps: u64,
    #[cfg(test)]
    pub(in crate::world) children_numbered: u64,
    #[cfg(test)]
    pub(in crate::world) child_list_steps: u64,
}

/// Dirty flags, layout invalidation causes, style resolution caches, paint
/// recordings, theme runtime, and work counters.
pub(in crate::world) struct WorldInvalidation {
    pub(in crate::world) dirty_entities: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) pending_layout_invalidations: NodeMap<LayoutInvalidation>,
    pub(in crate::world) pending_work_revision: u64,
    pub(in crate::world) pending_render_removals: Vec<StableNodeId>,
    pub(in crate::world) pending_accessibility_removals: Vec<StableNodeId>,
    pub(in crate::world) paint_recordings: RefCell<crate::NodeMap<PaintCacheEntry>>,
    pub(in crate::world) anchor_memo: RefCell<Option<crate::NodeMap<AnchorPair>>>,
    pub(in crate::world) modal_surface_recordings: RefCell<crate::NodeMap<PaintCacheEntry>>,
    pub(in crate::world) painter_overrides: crate::NodeMap<crate::NodePainter>,
    pub(in crate::world) media_time_mappings: crate::NodeMap<crate::MediaTimeMapping>,
    pub(in crate::world) theme: Arc<nana_ui_core::CompiledTheme>,
    pub(in crate::world) installed_theme: Arc<nana_ui_core::CompiledTheme>,
    pub(in crate::world) high_contrast: bool,
    pub(in crate::world) style_model: StyleModelRef,
    pub(in crate::world) theme_state: theme_scope::ThemeRuntimeState,
    pub(in crate::world) generation: u64,
    pub(in crate::world) layout_generation: u64,
    pub(in crate::world) layout_source_epoch: u64,
    pub(in crate::world) framework_strings: Arc<nana_ui_core::FrameworkStrings>,
    #[cfg(test)]
    pub(in crate::world) layout_publish_seq: u64,
    pub(in crate::world) layout_moved_roots: Vec<(StableNodeId, bool)>,
    pub(in crate::world) layout_written: NodeSet,
    pub(in crate::world) pending_document_layouts: Vec<(StableNodeId, LayoutBox)>,
    pub(in crate::world) layout_reframe_roots: Vec<StableNodeId>,
    pub(in crate::world) render_epoch: Cell<u32>,
    pub(in crate::world) geometry_frame: Cell<Option<StableNodeId>>,
    pub(in crate::world) geometry_read_page: Cell<bool>,
    pub(in crate::world) page_origins: RefCell<page_origins::PageOrigins>,
    pub(in crate::world) pass_publishes_written_boxes: bool,
    pub(in crate::world) layout_anchor_roots: HashSet<StableNodeId>,
    pub(in crate::world) cursor_style_dirty: bool,
    pub(in crate::world) style_hit_changed: bool,
    pub(in crate::world) counters: WorldCounters,
    pub(in crate::world) frame_extracted_nodes: usize,
    pub(in crate::world) frame_extracted_spans: usize,
    pub(in crate::world) accumulating_frame: bool,
    pub(in crate::world) pending_hot_allocations: Cell<usize>,
    pub(in crate::world) pending_hot_allocated_bytes: Cell<usize>,
    pub(in crate::world) layout_seeds_created: u64,
    pub(in crate::world) validation_nodes_scanned: usize,
    pub(in crate::world) intent_nodes: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) pending_theme_reads: Cell<usize>,
    pub(in crate::world) pending_layout_copies: Cell<usize>,
    pub(in crate::world) layouts: style::LayoutInterner,
    pub(in crate::world) node_styles: style::NodeStyleInterner,
    pub(in crate::world) inherited_styles: style::InheritedStyleInterner,
    pub(in crate::world) computed_styles: style::ComputedStyleInterner,
    pub(in crate::world) layout_pairs: style_pairs::LayoutPairMemo,
    pub(in crate::world) confirm_modals: usize,
    pub(in crate::world) modal_frames: usize,
    pub(in crate::world) menu_surfaces: usize,
    pub(in crate::world) clip_visuals: usize,
    pub(in crate::world) list_leading_visuals: usize,
    pub(in crate::world) z_index_nodes: usize,
    pub(in crate::world) inert_nodes: usize,
    pub(in crate::world) viewport_basis_nodes: usize,
    pub(in crate::world) viewport_basis: HashMap<DocumentId, HashSet<StableNodeId>, BuildIdHasher>,
    pub(in crate::world) document_viewports:
        HashMap<DocumentId, crate::LayoutViewport, BuildIdHasher>,
}

impl Default for WorldInvalidation {
    fn default() -> Self {
        Self {
            dirty_entities: HashSet::default(),
            pending_layout_invalidations: NodeMap::default(),
            pending_work_revision: 0,
            pending_render_removals: Vec::new(),
            pending_accessibility_removals: Vec::new(),
            paint_recordings: RefCell::new(crate::NodeMap::default()),
            anchor_memo: RefCell::new(None),
            modal_surface_recordings: RefCell::new(crate::NodeMap::default()),
            painter_overrides: crate::NodeMap::default(),
            media_time_mappings: crate::NodeMap::default(),
            theme: nana_ui_core::builtin_theme_arc(ThemeAppearance::default()),
            installed_theme: nana_ui_core::builtin_theme_arc(ThemeAppearance::default()),
            high_contrast: false,
            style_model: StyleModelRef::default(),
            theme_state: theme_scope::ThemeRuntimeState::default(),
            generation: 0,
            layout_generation: 0,
            layout_source_epoch: 1,
            framework_strings: Arc::default(),
            #[cfg(test)]
            layout_publish_seq: 0,
            layout_moved_roots: Vec::new(),
            layout_written: NodeSet::default(),
            pending_document_layouts: Vec::new(),
            layout_reframe_roots: Vec::new(),
            render_epoch: Cell::new(0),
            geometry_frame: Cell::new(None),
            geometry_read_page: Cell::new(false),
            page_origins: RefCell::default(),
            pass_publishes_written_boxes: false,
            layout_anchor_roots: HashSet::new(),
            cursor_style_dirty: false,
            style_hit_changed: false,
            counters: WorldCounters::default(),
            frame_extracted_nodes: 0,
            frame_extracted_spans: 0,
            accumulating_frame: false,
            pending_hot_allocations: Cell::new(0),
            pending_hot_allocated_bytes: Cell::new(0),
            layout_seeds_created: 0,
            validation_nodes_scanned: 0,
            intent_nodes: HashSet::default(),
            pending_theme_reads: Cell::new(0),
            pending_layout_copies: Cell::new(0),
            layouts: style::LayoutInterner::default(),
            node_styles: style::NodeStyleInterner::default(),
            inherited_styles: style::InheritedStyleInterner::default(),
            computed_styles: style::ComputedStyleInterner::default(),
            layout_pairs: style_pairs::LayoutPairMemo::default(),
            confirm_modals: 0,
            modal_frames: 0,
            menu_surfaces: 0,
            clip_visuals: 0,
            list_leading_visuals: 0,
            z_index_nodes: 0,
            inert_nodes: 0,
            viewport_basis_nodes: 0,
            viewport_basis: HashMap::default(),
            document_viewports: HashMap::default(),
        }
    }
}

/// Active timelines, presentation overlays, surface presence, and motion state.
pub(in crate::world) struct WorldMotion {
    pub(in crate::world) animations: HashMap<AnimationId, ActiveAnimation>,
    pub(in crate::world) settled_length_overlays: NodeMap<motion::PresentationLengthOverlay>,
    pub(in crate::world) animation_now: Duration,
    pub(in crate::world) presentation_clock: crate::presentation_clock::PresentationClock,
    pub(in crate::world) presentation: nana_ui_core::motion::PresentationStore,
    pub(in crate::world) presentation_query_samples: Cell<usize>,
    pub(in crate::world) presentation_samples_at_advance: Cell<usize>,
    pub(in crate::world) last_motion_frame: MotionWorkCounters,
    pub(in crate::world) compositor_layer_requests: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) motion_descriptors: nana_ui_core::motion::MotionDescriptorStore,
    pub(in crate::world) pending_animation_events: Vec<crate::AnimationEvent>,
    pub(in crate::world) surface_presence: NodeMap<motion::SurfacePresence>,
    pub(in crate::world) closing_surfaces_count: usize,
    #[cfg(feature = "controls")]
    pub(in crate::world) reorder_previews: HashMap<StableNodeId, StableNodeId, BuildIdHasher>,
    pub(in crate::world) reduced_motion: bool,
    pub(in crate::world) eyedropper_supported: bool,
    pub(in crate::world) hover_transitions:
        HashMap<StableNodeId, style::HoverTransition, BuildIdHasher>,
    pub(in crate::world) animation_deadlines: BTreeSet<(Duration, AnimationId)>,
    pub(in crate::world) presentation_transform_documents_cache:
        RefCell<Option<(u64, u64, HashSet<DocumentId>)>>,
}

impl Default for WorldMotion {
    fn default() -> Self {
        Self {
            animations: HashMap::new(),
            settled_length_overlays: NodeMap::default(),
            animation_now: Duration::ZERO,
            presentation_clock: Default::default(),
            presentation: nana_ui_core::motion::PresentationStore::new(),
            presentation_query_samples: Cell::new(0),
            presentation_samples_at_advance: Cell::new(0),
            last_motion_frame: MotionWorkCounters::default(),
            compositor_layer_requests: HashSet::default(),
            motion_descriptors: nana_ui_core::motion::MotionDescriptorStore::new(),
            pending_animation_events: Vec::new(),
            surface_presence: NodeMap::default(),
            closing_surfaces_count: 0,
            #[cfg(feature = "controls")]
            reorder_previews: HashMap::default(),
            reduced_motion: false,
            eyedropper_supported: false,
            hover_transitions: HashMap::default(),
            animation_deadlines: BTreeSet::new(),
            presentation_transform_documents_cache: RefCell::new(None),
        }
    }
}

/// Cached spans produced by a [`TextPresenter`] for one fenced code block.
#[cfg(feature = "rich-text")]
pub(in crate::world) struct CodePresentationEntry {
    pub(in crate::world) hash: u64,
    pub(in crate::world) presenter: Arc<str>,
    pub(in crate::world) language: Arc<str>,
    pub(in crate::world) source: Arc<str>,
    pub(in crate::world) spans: Arc<[crate::TextSpan]>,
}

/// Text presenters, language/scale defaults, and editor caches.
pub(in crate::world) struct WorldText {
    pub(in crate::world) hidden_carets: Vec<StableNodeId>,
    pub(in crate::world) presenters: HashMap<String, Box<dyn TextPresenter>>,
    #[cfg(feature = "rich-text")]
    pub(in crate::world) code_presentations: RefCell<Vec<CodePresentationEntry>>,
    pub(in crate::world) text_backend: Option<crate::text_node::TextBackendEpoch>,
    pub(in crate::world) default_language: Option<nana_text::font::LanguageTag>,
    pub(in crate::world) engine_fallback_language: Option<nana_text::font::LanguageTag>,
    pub(in crate::world) default_text_scale: f32,
    pub(in crate::world) document_text_scales: HashMap<DocumentId, f32, BuildIdHasher>,
    pub(in crate::world) text_backend_changed: bool,
    pub(in crate::world) text_input_adaptive_updates: Vec<StableNodeId>,
    pub(in crate::world) text_work: nana_text::TextWorkCounters,
    pub(in crate::world) text_frame_work: nana_text::TextWorkCounters,
    pub(in crate::world) pending_edit_work: nana_text::TextWorkCounters,
    pub(in crate::world) text_shown: Vec<StableNodeId>,
    pub(in crate::world) strut_candidates: Vec<StableNodeId>,
    pub(in crate::world) document_text_selections:
        HashMap<DocumentId, crate::DocumentTextSelection, BuildIdHasher>,
    pub(in crate::world) editor_caches: EditorPresentationCaches,
}

impl Default for WorldText {
    fn default() -> Self {
        Self {
            hidden_carets: Vec::new(),
            presenters: HashMap::new(),
            #[cfg(feature = "rich-text")]
            code_presentations: RefCell::new(Vec::new()),
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
            strut_candidates: Vec::new(),
            document_text_selections: HashMap::default(),
            editor_caches: EditorPresentationCaches::default(),
        }
    }
}

/// Per-document hit index and query scratch buffers.
#[derive(Default)]
pub(in crate::world) struct WorldHitIndex {
    pub(in crate::world) hit_test_index: HashMap<DocumentId, HitIndex, BuildIdHasher>,
    pub(in crate::world) non_scroll_hit_dirty: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) pointer_events_memo: RefCell<Option<NodeMap<PointerEventsSpec>>>,
    pub(in crate::world) hit_test_queries: Cell<u64>,
    pub(in crate::world) hit_scratch: RefCell<hit_test::HitScratch>,
}

/// Scroll containers, content-bounds index, deferred clamps, and frame deltas.
#[derive(Default)]
pub(in crate::world) struct WorldScroll {
    pub(in crate::world) hit_updates: Vec<(StableNodeId, [f32; 2])>,
    pub(in crate::world) content_bounds: RefCell<scroll_bounds::ContentBoundsIndex>,
    pub(in crate::world) containers: NodeSet,
    pub(in crate::world) restyled: Vec<StableNodeId>,
    pub(in crate::world) layout_touched: bool,
    pub(in crate::world) requested: Vec<(StableNodeId, ScrollOffset)>,
    pub(in crate::world) reclamped: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) resized: HashSet<StableNodeId, BuildIdHasher>,
}

/// Overlays, sticky/responsive/i18n/replaced indexes, drop targets, and a11y.
#[derive(Default)]
pub(in crate::world) struct WorldOverlaysA11y {
    pub(in crate::world) sticky_nodes: crate::StickyDependencies,
    pub(in crate::world) triggered_overlays: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) select_menus: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) overlay_host_nodes: HashSet<StableNodeId, BuildIdHasher>,
    pub(in crate::world) overlay_hosts_by_document:
        HashMap<DocumentId, HashSet<StableNodeId>, BuildIdHasher>,
    pub(in crate::world) overlay_dependents:
        HashMap<StableNodeId, HashSet<StableNodeId>, BuildIdHasher>,
    pub(in crate::world) labelled_by: HashMap<StableNodeId, StableNodeId, BuildIdHasher>,
    pub(in crate::world) labels: HashMap<StableNodeId, HashSet<StableNodeId>, BuildIdHasher>,
    pub(in crate::world) live_regions: HashMap<StableNodeId, AccessibilityLive, BuildIdHasher>,
    pub(in crate::world) replaced: replaced::ReplacedIndex,
    pub(in crate::world) responsive: responsive::ResponsiveIndex,
    pub(in crate::world) i18n: i18n::I18nIndex,
    pub(in crate::world) drop_targets:
        HashMap<StableNodeId, nana_ui_core::DropAccepts, BuildIdHasher>,
    pub(in crate::world) drop_hover: Option<(StableNodeId, nana_ui_core::DropEffect)>,
    #[cfg(any(test, feature = "layout-verify"))]
    pub(in crate::world) accessibility_shipped: accessibility::ShippedAccessibility,
}
